use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::{AppSink, AppSrc};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::input::InputState;
use crate::input::duml::devices::Kind;
use crate::input::duml::session::Session;
use crate::input::goggles::{Goggles, LinkState};
use crate::input::h264;
use crate::input::uvc::UvcState;
use crate::net::network::Network;
use crate::settings::InputMode;
use crate::settings::Scaling;
use crate::settings::SettingsStore;

mod screen;

/// Silence longer than this means the goggles stopped sending
const VIDEO_TIMEOUT: Duration = Duration::from_secs(1);
/// Every frame is a keyframe, so a new viewer gets a picture within one
/// interval; faster just spends WiFi on an unchanging image
const FRAME_INTERVAL: Duration = Duration::from_millis(500);
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
/// A GOP past this means the goggles stopped repeating SPS, so there's
/// nothing sensible left to freeze on
const MAX_GOP_BYTES: usize = 16 << 20;
const PIPELINE_TIMEOUT: Duration = Duration::from_secs(15);

/// Access units ready to push, alternated so decoders never see a repeat
type Frames = Vec<Vec<u8>>;

pub struct Splash {
    web_port: u16,
    store: Arc<SettingsStore>,
    network: Arc<Network>,
    goggles: Goggles,
    session: Arc<Session>,
    input: Arc<InputState>,
    feed: Mutex<Feed>,
    custom: Mutex<Option<Custom>>,
    status: Mutex<Option<screen::Screen>>,
}

struct Custom {
    frames: Frames,
    scaling: Scaling,
    /// The image as a data URI, for drawing the status over it
    uri: Arc<String>,
}

struct Feed {
    /// When the last goggles chunk arrived, or None while the splash is up
    live: Option<Instant>,
    /// The stream since its latest SPS
    gop: Vec<u8>,
}

impl Splash {
    pub fn new(
        web_port: u16,
        store: Arc<SettingsStore>,
        network: Arc<Network>,
        goggles: Goggles,
        session: Arc<Session>,
        input: Arc<InputState>,
    ) -> Arc<Self> {
        Arc::new(Self {
            web_port,
            store,
            network,
            goggles,
            session,
            input,
            feed: Mutex::new(Feed {
                live: None,
                gop: Vec::new(),
            }),
            custom: Mutex::new(None),
            status: Mutex::new(None),
        })
    }

    /// `push` gets whole access units, never interleaved with `video` chunks
    pub fn start(self: &Arc<Self>, push: impl Fn(&[u8]) + Send + 'static) {
        let this = Arc::clone(self);
        thread::spawn(move || this.run(push));
    }

    /// Route a goggles chunk to `push`. After the splash, the stream only
    /// resumes at an SPS, so decoders never see frames missing references
    pub fn video(&self, chunk: &[u8], push: impl FnOnce()) {
        let mut feed = self.feed.lock().unwrap();
        let sps = h264::nals(chunk).find(|nal| nal.kind == h264::SPS);
        if feed.live.is_none() {
            if sps.is_none() {
                return;
            }
            println!("[video] goggles stream started");
        }
        feed.live = Some(Instant::now());

        match sps {
            Some(sps) => {
                feed.gop.clear();
                feed.gop.extend_from_slice(&chunk[sps.start..]);
            }
            None if !feed.gop.is_empty() => feed.gop.extend_from_slice(chunk),
            None => {}
        }
        if feed.gop.len() > MAX_GOP_BYTES {
            feed.gop = Vec::new();
        }
        push();
    }

    pub fn has_custom_image(&self) -> bool {
        self.image_path().exists()
    }

    /// Validated by encoding it before anything is saved, so a bad upload
    /// can't leave the splash blank
    pub fn set_custom_image(&self, image: &[u8]) -> anyhow::Result<()> {
        let custom = Custom::new(image, self.store.get().splash.image_scaling)?;
        let path = self.image_path();
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, image)?;
        fs::rename(&tmp, &path)?;
        *self.custom.lock().unwrap() = Some(custom);
        println!("[splash] custom image set");
        Ok(())
    }

    pub fn remove_custom_image(&self) -> std::io::Result<()> {
        match fs::remove_file(self.image_path()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        *self.custom.lock().unwrap() = None;
        println!("[splash] custom image removed");
        Ok(())
    }

    /// What shows while there's no video, as (bytes, mime type).
    /// The arguments preview those settings before they're saved
    pub fn preview(
        &self,
        status_over_image: Option<bool>,
        image_scaling: Option<Scaling>,
    ) -> anyhow::Result<(Vec<u8>, &'static str)> {
        let cfg = self.store.get().splash;
        let over_image = status_over_image.unwrap_or(cfg.status_over_image);
        let scaling = image_scaling.unwrap_or(cfg.image_scaling);
        let custom = self.custom.lock().unwrap().as_ref().map(|c| c.uri.clone());
        let svg = {
            let status = self.status.lock().unwrap();
            match (&custom, &*status) {
                (Some(uri), Some(status)) if over_image => status.over_image(uri, scaling),
                (Some(_), _) => {
                    let image = fs::read(self.image_path())?;
                    let mime =
                        ImageKind::sniff(&image).map_or("application/octet-stream", |k| k.mime());
                    return Ok((image, mime));
                }
                (None, Some(status)) => status.svg(),
                (None, None) => anyhow::bail!("splash screen not rendered yet"),
            }
        };
        Ok((svg_to_png(&svg)?, "image/png"))
    }

    /// From the saved file, scaled the way the settings say now
    fn load_custom_image(&self) {
        let Ok(image) = fs::read(self.image_path()) else {
            return;
        };
        match Custom::new(&image, self.store.get().splash.image_scaling) {
            Ok(custom) => *self.custom.lock().unwrap() = Some(custom),
            Err(e) => eprintln!("[splash] custom image unusable: {e}"),
        }
    }

    fn image_path(&self) -> PathBuf {
        self.store.path().with_file_name("splash-image")
    }

    /// The selected input, how far along it is, and what it's called
    fn source_status(&self) -> (InputMode, LinkState, Option<String>) {
        match self.input.mode() {
            InputMode::DjiFpv => {
                // The model arrives a moment after the link comes up
                let name = self
                    .session
                    .devices()
                    .into_iter()
                    .find(|d| d.kind == Kind::Goggles)
                    .map(|d| d.name);
                (InputMode::DjiFpv, self.goggles.state(), name)
            }
            InputMode::Uvc => {
                let uvc = self.input.uvc.lock().unwrap().clone();
                let state = match uvc.state {
                    UvcState::Live => LinkState::Live,
                    UvcState::Starting => LinkState::Connecting,
                    _ => LinkState::Unplugged,
                };
                (InputMode::Uvc, state, uvc.device)
            }
            InputMode::DjiFpvLegacy => {
                use crate::input::legacy::LegacyState;
                let state = match self.input.legacy.lock().unwrap().state {
                    LegacyState::Waiting | LegacyState::Live => LinkState::Live,
                    _ => LinkState::Unplugged,
                };
                (InputMode::DjiFpvLegacy, state, None)
            }
        }
    }

    fn run(&self, push: impl Fn(&[u8])) {
        // Network status shells out to nmcli through tokio::process
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build splash runtime");

        self.load_custom_image();

        let mut screen = Frames::new();
        let mut screen_svg = String::new();
        let mut over_image = Frames::new();
        let mut over_image_svg = String::new();
        let mut over_image_of: Option<Arc<String>> = None;
        let mut frozen: Option<Frames> = None;
        let mut refreshed: Option<Instant> = None;
        let mut shown_source: Option<(InputMode, LinkState, Option<String>)> = None;
        let mut next = 0;

        loop {
            thread::sleep(FRAME_INTERVAL);

            let cfg = self.store.get().splash;
            let rescale = self
                .custom
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|c| c.scaling != cfg.image_scaling);
            if rescale {
                self.load_custom_image();
            }
            let background = if cfg.status_over_image {
                self.custom.lock().unwrap().as_ref().map(|c| c.uri.clone())
            } else {
                None
            };
            let background_changed = match (&background, &over_image_of) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                (None, None) => false,
                _ => true,
            };

            let shown = self.source_status();
            if shown_source.as_ref() != Some(&shown)
                || background_changed
                || refreshed.is_none_or(|t| t.elapsed() >= REFRESH_INTERVAL)
            {
                refreshed = Some(Instant::now());
                let status = rt.block_on(screen::gather(
                    self.web_port,
                    &self.store,
                    &self.network,
                    shown.0,
                    shown.1,
                    shown.2.clone(),
                ));
                shown_source = Some(shown);

                let svg = status.svg();
                if svg != screen_svg {
                    match encode_svg(&svg) {
                        Ok(frames) => {
                            screen = frames;
                            screen_svg = svg;
                        }
                        Err(e) => eprintln!("[splash] render failed: {e}"),
                    }
                }

                match &background {
                    Some(uri) => {
                        let svg = status.over_image(uri, cfg.image_scaling);
                        if svg != over_image_svg {
                            // Without it the image still shows, just bare
                            over_image = encode_svg(&svg).unwrap_or_else(|e| {
                                eprintln!("[splash] status over image failed: {e}");
                                Frames::new()
                            });
                            over_image_svg = svg;
                        }
                    }
                    None => {
                        over_image = Frames::new();
                        over_image_svg = String::new();
                    }
                }
                over_image_of = background;
                *self.status.lock().unwrap() = Some(status);
            }

            let lost = {
                let mut feed = self.feed.lock().unwrap();
                if feed.live.is_some_and(|t| t.elapsed() >= VIDEO_TIMEOUT) {
                    feed.live = None;
                    Some(std::mem::take(&mut feed.gop))
                } else {
                    None
                }
            };
            // Decoding takes a second or two; meanwhile HDMI holds its last
            // picture and video() can resume the stream
            if let Some(gop) = lost {
                println!("[video] goggles stream stopped, showing splash");
                frozen = None;
                if cfg.freeze_last_frame && !gop.is_empty() {
                    match freeze(&gop, cfg.grayscale_freeze) {
                        Ok(frames) => frozen = Some(frames),
                        Err(e) => eprintln!("[splash] couldn't freeze the last frame: {e}"),
                    }
                }
            }

            let feed = self.feed.lock().unwrap();
            if feed.live.is_some() {
                frozen = None;
                continue;
            }
            let custom = self.custom.lock().unwrap();
            let frames = match (&frozen, &*custom) {
                (Some(frozen), _) if cfg.freeze_last_frame => frozen,
                (_, Some(_)) if !over_image.is_empty() => &over_image,
                (_, Some(custom)) => &custom.frames,
                _ => &screen,
            };
            if !frames.is_empty() {
                next = (next + 1) % frames.len();
                push(&frames[next]);
            }
        }
    }
}

impl Custom {
    fn new(image: &[u8], scaling: Scaling) -> anyhow::Result<Self> {
        use base64::Engine;
        let frames = encode_image(image, scaling)?;
        let mime = ImageKind::sniff(image).map_or("application/octet-stream", |k| k.mime());
        let data = base64::engine::general_purpose::STANDARD.encode(image);
        Ok(Self {
            frames,
            scaling,
            uri: Arc::new(format!("data:{mime};base64,{data}")),
        })
    }
}

fn encode_svg(svg: &str) -> anyhow::Result<Frames> {
    let caps = gst::Caps::new_empty_simple("image/svg+xml");
    encode(
        "rsvgdec",
        gst::Buffer::from_slice(svg.as_bytes().to_vec()),
        Some(&caps),
    )
}

/// Decoders are picked by hand because decodebin prefers v4l2jpegdec,
/// which never finishes a still image
fn encode_image(image: &[u8], scaling: Scaling) -> anyhow::Result<Frames> {
    let decode = match ImageKind::sniff(image) {
        Some(ImageKind::Jpeg) => "jpegparse ! jpegdec",
        Some(ImageKind::Png) => "pngdec",
        None => anyhow::bail!("only JPEG and PNG images are supported"),
    };
    // Scaling to the screen adds bars by itself; filling it takes cropping
    // to the screen's shape first
    let decode = match scaling {
        Scaling::Fit => decode.to_string(),
        Scaling::Fill => format!(
            "{decode} ! videoconvert ! aspectratiocrop aspect-ratio={}/{}",
            screen::WIDTH,
            screen::HEIGHT
        ),
    };
    encode(&decode, gst::Buffer::from_slice(image.to_vec()), None)
        .map_err(|e| anyhow::anyhow!("not a usable image ({e})"))
}

enum ImageKind {
    Jpeg,
    Png,
}

impl ImageKind {
    fn sniff(image: &[u8]) -> Option<Self> {
        if image.starts_with(b"\x89PNG") {
            Some(Self::Png)
        } else if image.starts_with(&[0xff, 0xd8]) {
            Some(Self::Jpeg)
        } else {
            None
        }
    }

    fn mime(&self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
        }
    }
}

fn freeze(gop: &[u8], grayscale: bool) -> anyhow::Result<Frames> {
    let caps = gst::Caps::builder("video/x-h264")
        .field("stream-format", "byte-stream")
        .build();
    let mut recent: Vec<gst::Sample> = Vec::new();
    process(
        "appsrc name=src ! h264parse ! avdec_h264 ! appsink name=sink sync=false",
        gst::Buffer::from_slice(gop.to_vec()),
        Some(&caps),
        |sample| {
            // Only the last two are wanted; holding all of a 4K GOP would take gigabytes
            if recent.len() == 2 {
                recent.remove(0);
            }
            recent.push(sample);
        },
    )?;
    let picture = recent
        .first()
        .ok_or_else(|| anyhow::anyhow!("nothing decoded"))?;
    let mut buffer = picture
        .buffer_owned()
        .ok_or_else(|| anyhow::anyhow!("decoded sample has no buffer"))?;
    // Its timestamp would fall outside the fresh pipeline's segment
    let buffer_ref = buffer.make_mut();
    buffer_ref.set_pts(gst::ClockTime::NONE);
    buffer_ref.set_dts(gst::ClockTime::NONE);
    let filter = if grayscale {
        "videoconvert ! videobalance saturation=0"
    } else {
        "identity"
    };
    encode(filter, buffer, picture.caps())
}

/// Colorimetry is pinned because rsvgdec's sRGB transfer ends up in the
/// SPS, and v4l2h264dec refuses to negotiate it
///
/// Encode the picture as two IDR frames to alternate between: repeating one
/// IDR verbatim would reuse its idr_pic_id, which decoders may take for a
/// duplicate of the same picture
fn encode(decode: &str, input: gst::Buffer, caps: Option<&gst::CapsRef>) -> anyhow::Result<Frames> {
    let mut frames = Frames::new();
    process(
        &format!(
            "appsrc name=src ! {decode} ! videoconvert ! videoscale ! \
             imagefreeze num-buffers=2 ! \
             video/x-raw,format=I420,colorimetry=bt709,width={},height={},pixel-aspect-ratio=1/1,framerate=2/1 ! \
             x264enc speed-preset=medium tune=stillimage key-int-max=1 aud=true ! \
             video/x-h264,stream-format=byte-stream,alignment=au,profile=constrained-baseline ! \
             appsink name=sink sync=false",
            screen::WIDTH,
            screen::HEIGHT,
        ),
        input,
        caps,
        |sample| {
            if let Some(buffer) = sample.buffer()
                && let Ok(map) = buffer.map_readable()
            {
                frames.push(map.to_vec());
            }
        },
    )?;
    if frames.is_empty() {
        anyhow::bail!("encoder produced no frames");
    }
    Ok(frames)
}

fn svg_to_png(svg: &str) -> anyhow::Result<Vec<u8>> {
    let caps = gst::Caps::new_empty_simple("image/svg+xml");
    let mut png = None;
    process(
        "appsrc name=src ! rsvgdec ! videoconvert ! pngenc ! appsink name=sink sync=false",
        gst::Buffer::from_slice(svg.as_bytes().to_vec()),
        Some(&caps),
        |sample| {
            if let Some(buffer) = sample.buffer()
                && let Ok(map) = buffer.map_readable()
            {
                png = Some(map.to_vec());
            }
        },
    )?;
    png.ok_or_else(|| anyhow::anyhow!("no PNG produced"))
}

/// Push one buffer through a pipeline with an appsrc `src` and appsink
/// `sink`, and hand every sample to `on_sample` until EOS
fn process(
    desc: &str,
    input: gst::Buffer,
    caps: Option<&gst::CapsRef>,
    mut on_sample: impl FnMut(gst::Sample),
) -> anyhow::Result<()> {
    gst::init()?;
    let pipeline = gst::parse::launch(desc)?
        .dynamic_cast::<gst::Pipeline>()
        .map_err(|_| anyhow::anyhow!("not a pipeline"))?;

    let bin = pipeline.upcast_ref::<gst::Bin>();
    let element = |name: &str| {
        bin.by_name(name)
            .ok_or_else(|| anyhow::anyhow!("no element {name}"))
    };
    let src = element("src")?
        .dynamic_cast::<AppSrc>()
        .map_err(|_| anyhow::anyhow!("src is not an appsrc"))?;
    let sink = element("sink")?
        .dynamic_cast::<AppSink>()
        .map_err(|_| anyhow::anyhow!("sink is not an appsink"))?;
    src.set_format(gst::Format::Time);
    if let Some(caps) = caps {
        src.set_caps(Some(&caps.to_owned()));
    }
    let bus = pipeline
        .bus()
        .ok_or_else(|| anyhow::anyhow!("pipeline has no bus"))?;

    let result = (|| {
        pipeline.set_state(gst::State::Playing)?;
        let _ = src.push_buffer(input);
        let _ = src.end_of_stream();

        let started = Instant::now();
        loop {
            // A failing source also sends EOS, so errors are checked first
            if let Some(msg) = bus.pop_filtered(&[gst::MessageType::Error])
                && let gst::MessageView::Error(err) = msg.view()
            {
                anyhow::bail!("{}", err.error());
            }
            if let Some(sample) = sink.try_pull_sample(gst::ClockTime::from_mseconds(100)) {
                on_sample(sample);
            } else if sink.is_eos() {
                return Ok(());
            } else if started.elapsed() > PIPELINE_TIMEOUT {
                anyhow::bail!("timed out");
            }
        }
    })();
    pipeline.set_state(gst::State::Null)?;
    result
}
