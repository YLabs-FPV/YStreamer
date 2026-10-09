use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::display::{Display, ModeInfo};
use crate::input::h264::{self, AccessUnitAssembler};
use crate::output::overlay::{self, Overlay, Rect};
use crate::settings::{DisplaySettings, Scaling};

// kmssink takes the decoder's YUV dmabufs straight onto a display plane. A
// v4l2convert here turned every frame into 32-bit RGB first, which capped
// HDMI at ~20 fps once the goggles went to 60 fps. vc4 is an atomic driver
// whose commit already waits for vblank; without skip-vsync kmssink waited a
// second time, 30 fps at most
const PIPELINE: &str = "appsrc name=src is-live=true block=false max-bytes=2000000 format=time ! \
    h264parse config-interval=-1 ! \
    v4l2h264dec ! \
    queue max-size-buffers=1 leaky=downstream ! \
    {crop}kmssink name=sink sync=false skip-vsync=true";

// DRM plane rotation bits
const ROTATE_0: i32 = 0x1;
const ROTATE_180: i32 = 0x4;
const REFLECT_X: i32 = 0x10;

struct Running {
    pipeline: gst::Pipeline,
    appsrc: AppSrc,
    sink: gst::Element,
}

struct State {
    cfg: DisplaySettings,
    display: Option<Display>,
    running: Option<Running>,
}

pub struct GstPlayer {
    overlay: Arc<Overlay>,
    state: Mutex<State>,
    enabled: AtomicBool,
    assembler: Mutex<AccessUnitAssembler>,
}

impl GstPlayer {
    pub fn new(cfg: &DisplaySettings, overlay: Arc<Overlay>) -> anyhow::Result<Arc<Self>> {
        gst::init()?;
        let this = Arc::new(Self {
            overlay,
            state: Mutex::new(State {
                cfg: DisplaySettings {
                    hdmi_output: false,
                    ..cfg.clone()
                },
                display: None,
                running: None,
            }),
            enabled: AtomicBool::new(false),
            assembler: Mutex::new(AccessUnitAssembler::new()),
        });
        // No screen at boot isn't fatal; saving the settings retries
        if let Err(e) = this.apply(cfg) {
            eprintln!("[hdmi] not started: {e}");
        }

        let stats = Arc::clone(&this);
        thread::spawn(move || {
            let mut last_rendered: u64 = 0;
            loop {
                thread::sleep(Duration::from_secs(1));
                // Also catches the video changing size, e.g. 16:9 to 4:3
                stats.refresh_logo();
                let Some(sink) = stats.sink() else {
                    continue;
                };
                let s = sink.property::<gst::Structure>("stats");
                let rendered = s.get::<u64>("rendered").unwrap_or(0);
                let dropped = s.get::<u64>("dropped").unwrap_or(0);
                if rendered != last_rendered && crate::settings::verbose() {
                    println!(
                        "[hdmi] {} fps (dropped={})",
                        rendered.saturating_sub(last_rendered),
                        dropped
                    );
                }
                last_rendered = rendered;
            }
        });
        Ok(this)
    }

    /// Everything but a mode change is instant; a mode change also rebuilds
    /// the pipeline, so the picture returns with the next keyframe
    pub fn apply(&self, cfg: &DisplaySettings) -> Result<(), String> {
        let mut st = self.state.lock().unwrap();

        if !cfg.hdmi_output {
            self.enabled.store(false, Ordering::SeqCst);
            if let Some(r) = st.running.take() {
                let _ = r.pipeline.set_state(gst::State::Null);
                eprintln!("[hdmi] output off");
            }
            // Hands the screen back to the console
            st.display = None;
            st.cfg = cfg.clone();
            return Ok(());
        }

        let unchanged = st.running.is_some()
            && st.cfg.mode == cfg.mode
            && st.cfg.scaling == cfg.scaling
            && st.cfg.rotate_180 == cfg.rotate_180
            && st.cfg.mirror == cfg.mirror;
        if unchanged {
            st.cfg = cfg.clone();
            return Ok(());
        }

        self.enabled.store(false, Ordering::SeqCst);
        if let Some(r) = st.running.take() {
            let _ = r.pipeline.set_state(gst::State::Null);
        }

        let fresh = st.display.is_none();
        if fresh {
            st.display = Some(Display::open()?);
        }
        let mode_changed = fresh || st.cfg.mode != cfg.mode;
        let display = st.display.as_mut().expect("opened above");
        if mode_changed {
            display.set_mode(&cfg.mode)?;
        }
        let mode = display
            .current()
            .cloned()
            .ok_or("The screen has no active mode")?;

        let running = build(cfg, &mode, display.fd())?;
        running
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("Couldn't start HDMI output: {e}"))?;
        if st.cfg.hdmi_output != cfg.hdmi_output || fresh {
            eprintln!("[hdmi] output on");
        }
        st.running = Some(running);
        st.cfg = cfg.clone();
        self.enabled.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// The connected screen's modes, and the one in use
    pub fn modes(&self) -> Result<(Vec<ModeInfo>, Option<ModeInfo>), String> {
        let st = self.state.lock().unwrap();
        if let Some(d) = &st.display {
            return Ok((d.modes(), d.current().cloned()));
        }
        drop(st);
        // Output is off: look without taking the screen from the console
        let d = Display::open()?;
        Ok((d.modes(), None))
    }

    pub fn refresh_logo(&self) {
        let mut st = self.state.lock().unwrap();
        let State {
            cfg,
            display,
            running,
        } = &mut *st;
        let Some(display) = display.as_mut() else {
            return;
        };
        let target = (|| {
            // Already in the picture: drawing it again would double it
            if self.overlay.burned() {
                return None;
            }
            let (logo, overlay_cfg) = self.overlay.active()?;
            let screen = display.current()?;
            let caps = running.as_ref()?.sink.static_pad("sink")?.current_caps()?;
            let s = caps.structure(0)?;
            let video = (s.get::<i32>("width").ok()?, s.get::<i32>("height").ok()?);
            let screen = (screen.width as u32, screen.height as u32);
            let area = video_area(screen, (video.0 as u32, video.1 as u32), cfg.scaling);
            let mut at = overlay::place(area, (logo.width, logo.height), &overlay_cfg);
            // The video plane is turned in hardware; the logo goes with it
            if cfg.rotate_180 {
                at.x = screen.0 as i32 - at.x - at.w as i32;
                at.y = screen.1 as i32 - at.y - at.h as i32;
            }
            if cfg.mirror {
                at.x = screen.0 as i32 - at.x - at.w as i32;
            }
            Some((logo, overlay_cfg.opacity_percent, at))
        })();
        match target {
            Some((logo, opacity, at)) => {
                let rotation = plane_rotation(cfg) as u64;
                if let Err(e) =
                    display.show_logo(&logo, self.overlay.version(), at, opacity, rotation)
                {
                    eprintln!("[hdmi] {e}");
                }
            }
            None => display.hide_logo(),
        }
    }

    /// Pictures shown and dropped since the output started
    pub fn frames(&self) -> Option<(u64, u64)> {
        let stats = self.sink()?.property::<gst::Structure>("stats");
        Some((
            stats.get::<u64>("rendered").unwrap_or(0),
            stats.get::<u64>("dropped").unwrap_or(0),
        ))
    }

    fn sink(&self) -> Option<gst::Element> {
        let st = self.state.lock().unwrap();
        st.running.as_ref().map(|r| r.sink.clone())
    }

    /// Raw chunks from the goggles, cut into pictures here
    pub fn push(&self, data: &[u8]) -> anyhow::Result<()> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Ok(());
        }
        let mut assembler = self.assembler.lock().unwrap();
        assembler.push(data);
        while let Some(au) = assembler.next_au() {
            self.push_au(au)?;
        }
        Ok(())
    }

    /// One complete picture, e.g. from the splash screen
    pub fn push_au(&self, mut owned: Vec<u8>) -> anyhow::Result<()> {
        if owned.len() < 10 || !self.enabled.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Some(appsrc) = self
            .state
            .lock()
            .unwrap()
            .running
            .as_ref()
            .map(|r| r.appsrc.clone())
        else {
            return Ok(());
        };

        patch_sps_level(&mut owned);
        appsrc
            .push_buffer(gst::Buffer::from_mut_slice(owned))
            .map_err(|e| anyhow::anyhow!("push_buffer failed: {:?}", e))?;
        Ok(())
    }
}

fn plane_rotation(cfg: &DisplaySettings) -> i32 {
    let mut rotation = if cfg.rotate_180 { ROTATE_180 } else { ROTATE_0 };
    if cfg.mirror {
        rotation |= REFLECT_X;
    }
    rotation
}

/// Where kmssink puts the picture: centred and fitted, or across the whole
/// screen when filling (cropped edges fall outside it)
fn video_area(screen: (u32, u32), video: (u32, u32), scaling: Scaling) -> Rect {
    let full = Rect {
        x: 0,
        y: 0,
        w: screen.0,
        h: screen.1,
    };
    if scaling == Scaling::Fill || video.0 == 0 || video.1 == 0 {
        return full;
    }
    // Compare shapes without floats: video wider than the screen?
    if video.0 as u64 * screen.1 as u64 >= video.1 as u64 * screen.0 as u64 {
        let h = (screen.0 as u64 * video.1 as u64 / video.0 as u64) as u32;
        Rect {
            x: 0,
            y: ((screen.1 - h.min(screen.1)) / 2) as i32,
            w: screen.0,
            h,
        }
    } else {
        let w = (screen.1 as u64 * video.0 as u64 / video.1 as u64) as u32;
        Rect {
            x: ((screen.0 - w.min(screen.0)) / 2) as i32,
            y: 0,
            w,
            h: screen.1,
        }
    }
}

fn build(cfg: &DisplaySettings, mode: &ModeInfo, fd: i32) -> Result<Running, String> {
    // Fill crops in the display hardware: aspectratiocrop only attaches crop
    // metadata, which kmssink turns into the plane's source rectangle
    let crop = match cfg.scaling {
        Scaling::Fit => String::new(),
        Scaling::Fill => format!(
            "aspectratiocrop aspect-ratio={}/{} ! ",
            mode.width, mode.height
        ),
    };
    let pipeline = gst::parse::launch(&PIPELINE.replace("{crop}", &crop))
        .map_err(|e| format!("HDMI pipeline: {e}"))?
        .dynamic_cast::<gst::Pipeline>()
        .expect("Failed to cast to Pipeline");

    let appsrc = pipeline
        .by_name("src")
        .expect("Failed to find appsrc")
        .dynamic_cast::<AppSrc>()
        .expect("Failed to cast to AppSrc");
    // Whole pictures: with a bare byte stream h264parse only knows a
    // picture is complete once the next one starts, a frame of latency
    let caps = gst::Caps::builder("video/x-h264")
        .field("stream-format", "byte-stream")
        .field("alignment", "au")
        .build();
    appsrc.set_caps(Some(&caps));
    appsrc.set_stream_type(gstreamer_app::AppStreamType::Stream);
    appsrc.set_max_bytes(4_000_000);
    appsrc.set_block(false);
    appsrc.set_latency(gst::ClockTime::ZERO, gst::ClockTime::NONE);

    let sink = pipeline.by_name("sink").expect("Failed to find kmssink");
    sink.set_property("fd", fd);
    // Always set, or the plane keeps the last pipeline's rotation
    sink.set_property(
        "plane-properties",
        gst::Structure::builder("s")
            .field("rotation", plane_rotation(cfg))
            .build(),
    );

    if let Some(bus) = pipeline.bus() {
        bus.set_sync_handler(|_, msg| {
            use gst::MessageView;
            match msg.view() {
                MessageView::Error(err) => {
                    eprintln!("[hdmi] error: {} ({:?})", err.error(), err.debug());
                }
                MessageView::Warning(w) => {
                    eprintln!("[hdmi] warning: {} ({:?})", w.error(), w.debug());
                }
                _ => (),
            }
            gst::BusSyncReply::Drop
        });
    }

    Ok(Running {
        pipeline,
        appsrc,
        sink,
    })
}

/// Patch SPS level for the Raspberry Pi 4's H.264 decoder, which is limited to level 4.2
fn patch_sps_level(data: &mut [u8]) {
    let Some(sps) = h264::nals(data).find(|nal| nal.kind == h264::SPS) else {
        return;
    };
    // Raising this to e.g. 5.2 to lift 4.2's ~16fps/4K macroblock-rate cap was
    // tried and made the decoder fail outright - no picture at all
    if let Some(level) = data.get_mut(sps.header + 3) {
        *level = 42; // H.264 level 4.2
    }
}

impl Drop for GstPlayer {
    fn drop(&mut self) {
        if let Some(r) = self.state.lock().unwrap().running.take() {
            let _ = r.pipeline.set_state(gst::State::Null);
        }
    }
}
