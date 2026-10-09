use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::{AppSink, AppSrc};

use crate::settings::OverlaySettings;

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
/// Plenty for a logo, and keeps the display plane and the blend cheap
const MAX_SIDE: u32 = 2048;
const DECODE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Logo {
    pub width: u32,
    pub height: u32,
    /// Straight (not premultiplied) alpha, rows top to bottom
    pub bgra: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

pub struct Overlay {
    path: PathBuf,
    logo: RwLock<Option<Arc<Logo>>>,
    cfg: RwLock<OverlaySettings>,
    /// Bumped on every change, so browsers refetch the image
    version: AtomicU64,
    /// The picture reaching HDMI and the browsers already has the logo in
    /// it, so they mustn't draw it a second time
    burned: AtomicBool,
}

impl Overlay {
    pub fn load(path: PathBuf, cfg: &OverlaySettings) -> Arc<Self> {
        let logo = fs::read(&path).ok().and_then(|png| match decode(&png) {
            Ok(logo) => Some(Arc::new(logo)),
            Err(e) => {
                eprintln!("[overlay] {} is unusable: {e}", path.display());
                None
            }
        });
        Arc::new(Self {
            path,
            logo: RwLock::new(logo),
            cfg: RwLock::new(cfg.clone()),
            version: AtomicU64::new(1),
            burned: AtomicBool::new(false),
        })
    }

    pub fn cfg(&self) -> OverlaySettings {
        self.cfg.read().unwrap().clone()
    }

    pub fn set_cfg(&self, cfg: &OverlaySettings) {
        *self.cfg.write().unwrap() = cfg.clone();
    }

    /// The logo, if it's switched on and there is one
    pub fn active(&self) -> Option<(Arc<Logo>, OverlaySettings)> {
        let cfg = self.cfg();
        if !cfg.enabled {
            return None;
        }
        Some((self.logo()?, cfg))
    }

    pub fn burned(&self) -> bool {
        self.burned.load(Ordering::Relaxed)
    }

    pub fn set_burned(&self, burned: bool) {
        self.burned.store(burned, Ordering::Relaxed);
    }

    pub fn logo(&self) -> Option<Arc<Logo>> {
        self.logo.read().unwrap().clone()
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    /// Decoded before anything is saved, so a bad upload changes nothing
    pub fn set_image(&self, png: &[u8]) -> Result<(), String> {
        let logo = decode(png)?;
        let tmp = self.path.with_extension("tmp");
        let err = |e: std::io::Error| format!("Couldn't save the logo: {e}");
        fs::write(&tmp, png).map_err(err)?;
        fs::rename(&tmp, &self.path).map_err(err)?;
        *self.logo.write().unwrap() = Some(Arc::new(logo));
        self.version.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub fn remove(&self) -> Result<(), String> {
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Couldn't remove the logo: {e}")),
        }
        *self.logo.write().unwrap() = None;
        self.version.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

/// Where the logo goes within `video`, which may itself sit inside a larger
/// screen. The size follows the picture's width and the position is a share
/// of the room left around the logo, so it looks the same at every
/// resolution and never leaves the picture
pub fn place(video: Rect, logo: (u32, u32), cfg: &OverlaySettings) -> Rect {
    let (lw, lh) = (logo.0.max(1) as f32, logo.1.max(1) as f32);
    let (vw, vh) = (video.w as f32, video.h as f32);
    let mut w = vw * cfg.size_percent / 100.0;
    let mut h = w * lh / lw;
    if h > vh {
        h = vh;
        w = h * lw / lh;
    }
    let (w, h) = (w.round().max(1.0), h.round().max(1.0));
    Rect {
        x: video.x + ((vw - w) * cfg.x_percent / 100.0).round() as i32,
        y: video.y + ((vh - h) * cfg.y_percent / 100.0).round() as i32,
        w: w as u32,
        h: h as u32,
    }
}

fn decode(png: &[u8]) -> Result<Logo, String> {
    if !png.starts_with(PNG_MAGIC) {
        return Err("not a PNG image".into());
    }
    gst::init().map_err(|e| e.to_string())?;
    let pipeline = gst::parse::launch(
        "appsrc name=src ! pngdec ! videoconvert ! video/x-raw,format=BGRA ! \
         appsink name=sink sync=false",
    )
    .map_err(|e| e.to_string())?
    .dynamic_cast::<gst::Pipeline>()
    .expect("Failed to cast to Pipeline");
    let src = pipeline
        .by_name("src")
        .expect("Failed to find appsrc")
        .dynamic_cast::<AppSrc>()
        .expect("Failed to cast to AppSrc");
    let sink = pipeline
        .by_name("sink")
        .expect("Failed to find appsink")
        .dynamic_cast::<AppSink>()
        .expect("Failed to cast to AppSink");
    src.set_caps(Some(&gst::Caps::new_empty_simple("image/png")));

    let result = (|| {
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| e.to_string())?;
        src.push_buffer(gst::Buffer::from_slice(png.to_vec()))
            .map_err(|e| format!("{e:?}"))?;
        src.end_of_stream().map_err(|e| format!("{e:?}"))?;
        let sample = sink
            .try_pull_sample(gst::ClockTime::from_mseconds(
                DECODE_TIMEOUT.as_millis() as u64
            ))
            .ok_or("the image couldn't be read")?;
        let info = sample
            .caps()
            .and_then(|c| gstreamer_video::VideoInfo::from_caps(c).ok())
            .ok_or("the image has no size")?;
        let (width, height) = (info.width(), info.height());
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(format!(
                "the image is {width} x {height}; at most {MAX_SIDE} pixels a side"
            ));
        }
        let buffer = sample.buffer().ok_or("the image is empty")?;
        let map = buffer.map_readable().map_err(|e| e.to_string())?;
        // Rows may be padded; keep exactly width * 4 bytes of each
        let stride = info.stride()[0] as usize;
        let row = width as usize * 4;
        let mut bgra = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            let start = y * stride;
            bgra.extend_from_slice(
                map.get(start..start + row)
                    .ok_or("the image is truncated")?,
            );
        }
        Ok(Logo {
            width,
            height,
            bgra,
        })
    })();
    let _ = pipeline.set_state(gst::State::Null);
    result
}

#[cfg(test)]
#[path = "overlay.test.rs"]
mod tests;
