mod cli;
mod input;
mod net;
mod output;
mod settings;
mod system;
mod web;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use input::duml::monitor::Monitor;
use input::duml::session::Session;
use input::duml::{self, DumlReassemblyBuffer};
use input::goggles::Goggles;
use input::uvc::UvcSource;
use input::{Fanout, InputState};
use net::network;
use output::hdmi::GstPlayer;
use output::overlay;
use output::recording::Recorder;
use output::rtmp::RtmpOutput;
use output::rtsp::RtspServer;
use output::splash::Splash;
use output::srt::SrtOutput;
use output::udp::UdpOutput;
use settings::InputMode;
use system::{button, metrics};
use web::{api, auth};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::Command::parse(&args) {
        Ok(cli::Command::Run) => run(),
        Ok(command) => std::process::exit(cli::execute(command)),
        Err(message) => std::process::exit(cli::usage_error(&message)),
    }
}

fn run() -> anyhow::Result<()> {
    let store = Arc::new(settings::SettingsStore::load());
    let cfg = store.get();

    // Outputs first, so the web server can reconfigure them from settings
    let overlay = overlay::Overlay::load(store.path().with_file_name("overlay.png"), &cfg.overlay);
    let player = GstPlayer::new(&cfg.display, Arc::clone(&overlay))?;
    let rtsp_server = Arc::new(RtspServer::new(&cfg.rtsp)?);
    let srt = SrtOutput::new(&cfg.srt);
    let rtmp = RtmpOutput::new(&cfg.rtmp);
    let udp = UdpOutput::new(&cfg.udp);
    let recorder = Arc::new(Recorder::new(cfg.recording.clone()));
    // What a transfer onto the card left behind if the power went mid-copy
    output::transfer::clear_unfinished(&recorder.dir());
    let button = button::RecordButton::start(&cfg.recording.button, Arc::clone(&recorder));

    input::gadget::ensure();
    let goggles = Goggles::new();
    let session = Session::start(goggles.clone());
    let monitor = Arc::new(Monitor::new());

    let network = network::Network::new();

    let input = InputState::new(cfg.input.mode);

    let listener = web::bind()?;
    let web_port = listener.local_addr()?.port();
    let _ = system::update::PORT.set(web_port);
    let splash = Splash::new(
        web_port,
        Arc::clone(&store),
        Arc::clone(&network),
        goggles.clone(),
        Arc::clone(&session),
        Arc::clone(&input),
    );

    let auth = auth::Auth::load(store.path().with_file_name("auth.json"));

    let (web_tx, web_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let (state_tx, state_rx) = tokio::sync::mpsc::unbounded_channel();

    let metrics = metrics::Metrics::new();
    metrics.start(metrics::Sources {
        player: Arc::clone(&player),
        rtsp: Arc::clone(&rtsp_server),
        srt: Arc::clone(&srt),
        rtmp: Arc::clone(&rtmp),
        udp: Arc::clone(&udp),
        recorder: Arc::clone(&recorder),
    });

    let fanout = Arc::new(Fanout {
        state: Arc::clone(&input),
        splash: Arc::clone(&splash),
        player: Arc::clone(&player),
        rtsp: Arc::clone(&rtsp_server),
        srt: Arc::clone(&srt),
        rtmp: Arc::clone(&rtmp),
        udp: Arc::clone(&udp),
        recorder: Arc::clone(&recorder),
        web_tx,
        metrics: Arc::clone(&metrics),
    });
    fanout.start_splash();
    let uvc = UvcSource::start(
        Arc::clone(&input),
        &cfg.input.uvc,
        Arc::clone(&fanout),
        Arc::clone(&overlay),
    );

    let legacy = input::legacy::LegacySource::start(Arc::clone(&input), Arc::clone(&fanout));

    let services = api::Services {
        store: Arc::clone(&store),
        auth,
        network: Arc::clone(&network),
        rtsp: Arc::clone(&rtsp_server),
        srt: Arc::clone(&srt),
        rtmp: Arc::clone(&rtmp),
        udp: Arc::clone(&udp),
        player: Arc::clone(&player),
        recorder: Arc::clone(&recorder),
        transfers: output::transfer::Transfers::new(),
        button,
        goggles: goggles.clone(),
        session: Arc::clone(&session),
        monitor: Arc::clone(&monitor),
        splash: Arc::clone(&splash),
        input: Arc::clone(&input),
        metrics,
        uvc,
        legacy,
        overlay: Arc::clone(&overlay),
    };

    web::start(listener, services, web_rx, state_rx);

    let camera_poller = duml::camera::Poller::start(goggles.clone(), Arc::clone(&session));
    let mut aircraft_recorder = duml::aircraft_recorder::Tracker::new();
    let mut link = duml::link::Tracker::new();
    let mut duml = DumlReassemblyBuffer::new();

    let chunk_count = Arc::new(AtomicU64::new(0));
    {
        let chunk_count = Arc::clone(&chunk_count);
        thread::spawn(move || {
            let mut last = 0u64;
            loop {
                thread::sleep(Duration::from_secs(1));
                let now = chunk_count.load(Ordering::Relaxed);
                if now != last && settings::verbose() {
                    println!("[usb] {} video chunks/s", now - last);
                }
                last = now;
            }
        });
    }

    let mut last_mode = None;
    let alive = goggles.clone();
    goggles.run(|data| {
        duml.process(
            data,
            |chunk| {
                alive.mark_alive();
                if fanout.video(InputMode::DjiFpv, chunk) {
                    chunk_count.fetch_add(1, Ordering::Relaxed);
                }
            },
            |frame| {
                alive.mark_alive();
                monitor.record(frame);
                session.on_frame(frame);
                if let Some(quality) = link.on_frame(frame) {
                    let _ = state_tx.send(duml::camera::CameraState {
                        link_quality: Some(quality),
                        ..Default::default()
                    });
                }
                let state = if !frame.is_response() {
                    if frame.cmd_set == duml::camera::CMD_SET && frame.cmd_id == 129 {
                        camera_poller.saw_push();
                    }
                    if let Some(aircraft_recorder) =
                        aircraft_recorder.on_frame(frame.cmd_set, frame.cmd_id, frame.payload)
                    {
                        let _ = state_tx.send(duml::camera::CameraState {
                            aircraft_recorder: Some(aircraft_recorder),
                            ..Default::default()
                        });
                    }
                    duml::camera::parse_duml_response(frame.cmd_set, frame.cmd_id, frame.payload)
                } else if frame.cmd_set == duml::camera::CMD_SET {
                    duml::camera::parse_get_reply(frame.cmd_id, frame.payload)
                } else {
                    None
                };
                if let Some(state) = state {
                    if state.res.is_some() || state.fps.is_some() {
                        let mode =
                            format!("res={:?} ar={:?} fps={:?}", state.res, state.ar, state.fps);
                        if last_mode.as_ref() != Some(&mode) {
                            println!("[cam] {mode}");
                            last_mode = Some(mode);
                        }
                    }
                    let _ = state_tx.send(state);
                }
            },
        );
    })
}
