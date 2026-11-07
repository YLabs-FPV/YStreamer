pub mod transport;

use std::fs;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::h264::RecoveryPoints;
use super::{Fanout, InputState};
use crate::settings::InputMode;
use transport::{Assembler, KIND_STATE, KIND_VIDEO, Packet};

/// Goggles 3, whose network adapter appears with liveview sharing on
const VENDOR: &str = "2ca3";
const PRODUCT: &str = "0020";
/// Ours on the goggles' USB link. They serve the session DJI Fly gets over
/// their WiFi here too
pub const HOST: Ipv4Addr = Ipv4Addr::new(192, 168, 60, 1);
const PEER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 60, 2), 9003);

/// How often DJI Fly sends its keep-alive and acks. The goggles stop
/// streaming to a client that goes quiet
const CONTROL_EVERY: Duration = Duration::from_millis(16);
const ACK_EVERY: Duration = Duration::from_millis(20);
const POLL: Duration = Duration::from_millis(2);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);
const LINK_TIMEOUT: Duration = Duration::from_secs(3);
/// With sharing switched off the session can stay up without video
const VIDEO_TIMEOUT: Duration = Duration::from_secs(3);
/// The goggles turn away a session opened right after the last one ended;
/// after a pause a fresh one works
const RETRY: Duration = Duration::from_secs(1);

#[derive(Serialize, Clone, Copy, Default, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LiveviewState {
    /// Another source is selected
    #[default]
    Idle,
    NoDevice,
    /// Plugged in, but the goggles don't answer: liveview sharing is off
    Waiting,
    /// Session open, no video yet
    Connected,
    Live,
    Error,
}

#[derive(Serialize, Clone, Default, PartialEq, Debug)]
pub struct LiveviewStatus {
    pub state: LiveviewState,
    /// The goggles' network interface
    pub interface: Option<String>,
    pub error: Option<String>,
}

enum Ended {
    /// Another source was selected
    Deselected,
    NoAnswer,
    Quiet(&'static str),
    Failed(String),
}

pub struct LiveviewSource {
    state: Arc<InputState>,
}

impl LiveviewSource {
    pub fn start(state: Arc<InputState>, fanout: Arc<Fanout>) -> Arc<Self> {
        let source = Arc::new(Self { state });
        let this = Arc::clone(&source);
        thread::spawn(move || {
            // NetworkManager is driven through tokio::process
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to build liveview runtime");
            loop {
                if this.selected() {
                    this.attempt(&fanout, &rt);
                } else {
                    this.set(LiveviewState::Idle, None, None);
                }
                thread::sleep(RETRY);
            }
        });
        source
    }

    pub fn status(&self) -> LiveviewStatus {
        self.state.liveview.lock().unwrap().clone()
    }

    fn selected(&self) -> bool {
        self.state.mode() == InputMode::DjiGogglesUsb
    }

    /// Returns whether the state changed, so only changes get logged
    fn set(&self, state: LiveviewState, interface: Option<&str>, error: Option<String>) -> bool {
        let next = LiveviewStatus {
            state,
            interface: interface.map(str::to_string),
            error,
        };
        let mut status = self.state.liveview.lock().unwrap();
        let changed = status.state != next.state || status.error != next.error;
        *status = next;
        changed
    }

    fn attempt(&self, fanout: &Fanout, rt: &tokio::runtime::Runtime) {
        let Some(interface) = interface() else {
            if self.set(LiveviewState::NoDevice, None, None) {
                println!("[liveview] no goggles with liveview sharing plugged in");
            }
            return;
        };
        if let Err(e) = rt.block_on(crate::net::network::goggles_link(&interface)) {
            let e = format!("Couldn't set up the goggles' network on {interface}: {e}");
            if self.set(LiveviewState::Error, Some(&interface), Some(e.clone())) {
                eprintln!("[liveview] {e}");
            }
            return;
        }
        keep_awake(&interface);
        if self.status().interface.as_deref() != Some(interface.as_str()) {
            println!("[liveview] goggles on {interface}");
        }

        match self.session(&interface, fanout) {
            Ended::Deselected => {}
            Ended::NoAnswer => {
                if self.set(LiveviewState::Waiting, Some(&interface), None) {
                    println!("[liveview] goggles don't answer; liveview sharing off?");
                }
            }
            // Still plugged in and answering; the next session finds out more
            Ended::Quiet(why) => {
                if self.status().state == LiveviewState::Live {
                    println!("[liveview] {why}, reconnecting");
                }
                self.set(LiveviewState::Connected, Some(&interface), None);
            }
            Ended::Failed(e) => {
                if self.set(LiveviewState::Error, Some(&interface), Some(e.clone())) {
                    eprintln!("[liveview] {e}");
                }
            }
        }
    }

    fn session(&self, interface: &str, fanout: &Fanout) -> Ended {
        let socket = match UdpSocket::bind((HOST, 0)).and_then(|socket| {
            socket.connect(PEER)?;
            Ok(socket)
        }) {
            Ok(socket) => socket,
            Err(e) => return Ended::Failed(format!("Couldn't open the goggles' session: {e}")),
        };
        let mut recovery = RecoveryPoints::default();
        let mut live = false;
        run(
            &socket,
            || self.selected(),
            |event| match event {
                Event::Open => {
                    if self.set(LiveviewState::Connected, Some(interface), None) {
                        println!("[liveview] session open, waiting for video");
                    }
                }
                Event::Video(unit) => {
                    fanout.video(InputMode::DjiGogglesUsb, &recovery.mark(unit));
                    if !live {
                        live = true;
                        println!("[liveview] video started");
                        self.set(LiveviewState::Live, Some(interface), None);
                    }
                }
            },
        )
    }
}

enum Event<'a> {
    /// The goggles took the session
    Open,
    /// A piece of the H.264 stream, in order
    Video(&'a [u8]),
}

/// One session on a socket connected to the goggles, until `keep_going`
/// says stop or the goggles do. Each needs a fresh socket, id and sequence:
/// the goggles won't take a reused one
fn run(socket: &UdpSocket, keep_going: impl Fn() -> bool, mut on: impl FnMut(Event)) -> Ended {
    let failed = |what: &str, e: std::io::Error| Ended::Failed(format!("{what}: {e}"));
    if let Err(e) = socket.set_read_timeout(Some(POLL)) {
        return failed("Couldn't open the goggles' session", e);
    }
    let session = loop {
        let id = rand::random::<u16>();
        if id != 0 {
            break id;
        }
    };
    let mut seq = rand::random::<u16>() & 0xfff8;
    let mut peer_seq = seq;
    let mut duml_seq = rand::random::<u16>();
    let mut counter = 0u8;
    // Nobody listening yet, e.g. sharing just switched on: wait it out
    let send = |datagram: &[u8]| match socket.send(datagram) {
        Err(e) if e.kind() != ErrorKind::ConnectionRefused => Err(e),
        _ => Ok(()),
    };
    if let Err(e) = send(&transport::opener(session, seq)) {
        return failed("Couldn't reach the goggles", e);
    }

    let started = Instant::now();
    let mut last_control: Option<Instant> = None;
    let mut last_ack = started;
    let mut last_received = started;
    let mut last_video = started;
    let mut answered = false;
    let mut state = None;
    let mut ack_seq = None;
    let mut assembler = Assembler::default();
    let mut buf = vec![0u8; 65_535];

    while keep_going() {
        let now = Instant::now();
        if last_control.is_none_or(|t| now - t >= CONTROL_EVERY) {
            seq = seq.wrapping_add(8);
            duml_seq = duml_seq.wrapping_add(1);
            counter = counter.wrapping_add(1);
            let control = transport::control(session, seq, peer_seq, counter, duml_seq);
            if let Err(e) = send(&control) {
                return failed("Lost the goggles", e);
            }
            last_control = Some(now);
        }
        if let (Some(video_seq), Some(state)) = (ack_seq, &state)
            && now - last_ack >= ACK_EVERY
        {
            if let Err(e) = send(&transport::ack(session, video_seq, state)) {
                return failed("Lost the goggles", e);
            }
            last_ack = now;
        }

        match socket.recv(&mut buf) {
            Ok(len) => match transport::parse(&buf[..len], session) {
                Some(Packet::OpenerAck) => {
                    last_received = Instant::now();
                    if !answered {
                        answered = true;
                        on(Event::Open);
                    }
                }
                Some(Packet::Transport {
                    seq: their_seq,
                    kind,
                    datagram,
                }) => {
                    last_received = Instant::now();
                    if their_seq != 0 {
                        peer_seq = their_seq;
                    }
                    if kind == KIND_STATE {
                        state = transport::state(datagram).or(state);
                    }
                    if kind == KIND_VIDEO {
                        last_video = last_received;
                        for unit in assembler.push(datagram) {
                            ack_seq = Some(unit.last_seq);
                            on(Event::Video(&unit.data));
                        }
                    }
                }
                None => {}
            },
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::ConnectionRefused
                ) => {}
            // The goggles went away and took the interface with them
            Err(e) => return failed("Lost the goggles", e),
        }

        let now = Instant::now();
        if !answered && now - started >= ANSWER_TIMEOUT {
            return Ended::NoAnswer;
        }
        if answered && now - last_received >= LINK_TIMEOUT {
            return Ended::Quiet("goggles went quiet");
        }
        if answered && now - last_video >= VIDEO_TIMEOUT {
            if crate::settings::verbose() {
                println!(
                    "[liveview] duplicate={} invalid={} skipped={}",
                    assembler.duplicates, assembler.invalid, assembler.skipped
                );
            }
            return Ended::Quiet("video stopped");
        }
    }
    Ended::Deselected
}

/// The goggles' network interface, found by the USB device behind it:
/// its name comes from the goggles' MAC address
fn interface() -> Option<String> {
    fs::read_dir("/sys/class/net")
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|dev| is_goggles(dev))
}

/// The interface's sysfs device is the USB interface; the IDs live on its
/// parent, the USB device
pub fn is_goggles(dev: &str) -> bool {
    let id = |file: &str| {
        fs::read_to_string(format!("/sys/class/net/{dev}/device/../{file}"))
            .map(|v| v.trim().to_ascii_lowercase())
    };
    id("idVendor").is_ok_and(|v| v == VENDOR) && id("idProduct").is_ok_and(|p| p == PRODUCT)
}

/// A suspended USB link goes as quiet as an unplugged one
fn keep_awake(dev: &str) {
    let control = Path::new("/sys/class/net")
        .join(dev)
        .join("device/../power/control");
    if fs::read_to_string(&control).is_ok_and(|v| v.trim() == "on") {
        return;
    }
    if let Err(e) = fs::write(&control, "on") {
        eprintln!("[liveview] couldn't keep the goggles' USB link awake: {e}");
    }
}

#[cfg(test)]
#[path = "liveview.test.rs"]
mod tests;
