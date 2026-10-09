use std::fs::{self, File, OpenOptions};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};

use drm::buffer::{Buffer, DrmFourcc};
use drm::control::{
    Device as ControlDevice, Mode, ModeFlags, ModeTypeFlags, connector, crtc,
    dumbbuffer::DumbBuffer, framebuffer, plane,
};
use drm::{ClientCapability, Device as _};

use crate::output::overlay::{Logo, Rect};

pub const AUTO: &str = "auto";
// Panels report whole millimetres, so 16:9 comes out as e.g. 630x360 (1.75)
const SHAPE_TOLERANCE: f64 = 0.05;

// DRM plane property values
const PLANE_TYPE_OVERLAY: u64 = 0;
const BLEND_COVERAGE: u64 = 1;
const ALPHA_OPAQUE: u64 = 0xffff;
/// Above the video plane, which sits at the bottom of the overlay range
const LOGO_ZPOS: u64 = 10;

/// The logo's own hardware plane, layered over the video for free
struct LogoPlane {
    plane: plane::Handle,
    fb: framebuffer::Handle,
    buffer: DumbBuffer,
    /// Which upload the buffer holds
    version: u64,
    size: (u32, u32),
}

struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl drm::Device for Card {}
impl ControlDevice for Card {}

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct ModeInfo {
    /// `WxH@Hz`, what settings store
    pub id: String,
    pub width: u16,
    pub height: u16,
    pub refresh: f64,
    /// The screen's own choice
    pub preferred: bool,
    /// Same shape as the panel. Otherwise the screen stretches it, and
    /// kmssink's non-square pixel handling crops the picture
    pub fits_screen: bool,
}

pub struct Display {
    card: Card,
    connector: connector::Handle,
    crtc: crtc::Handle,
    modes: Vec<(ModeInfo, Mode)>,
    current: Option<ModeInfo>,
    background: Option<(framebuffer::Handle, DumbBuffer)>,
    logo: Option<LogoPlane>,
}

impl Display {
    /// The first connected HDMI port on any card that has one
    pub fn open() -> Result<Self, String> {
        let mut cards: Vec<_> = fs::read_dir("/dev/dri")
            .map_err(|e| format!("/dev/dri: {e}"))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("card"))
            })
            .collect();
        cards.sort();

        for path in cards {
            let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
                continue;
            };
            let card = Card(file);
            // Without this only a few legacy planes are listed
            let _ = card.set_client_capability(ClientCapability::UniversalPlanes, true);
            let Ok(res) = card.resource_handles() else {
                continue;
            };
            for &handle in res.connectors() {
                let Ok(info) = card.get_connector(handle, true) else {
                    continue;
                };
                if info.state() != connector::State::Connected
                    || info.interface() != connector::Interface::HDMIA
                {
                    continue;
                }
                let crtc = info
                    .current_encoder()
                    .and_then(|e| card.get_encoder(e).ok())
                    .and_then(|e| e.crtc())
                    .or_else(|| {
                        info.encoders()
                            .iter()
                            .filter_map(|&e| card.get_encoder(e).ok())
                            .flat_map(|e| res.filter_crtcs(e.possible_crtcs()))
                            .next()
                    })
                    .ok_or("HDMI port has no display controller")?;

                let panel = info.size().filter(|&(w, h)| w > 0 && h > 0);
                let mut modes: Vec<(ModeInfo, Mode)> = Vec::new();
                for &m in info.modes() {
                    if m.flags().contains(ModeFlags::INTERLACE) {
                        continue;
                    }
                    let mut mi = describe(&m);
                    if let Some((pw, ph)) = panel {
                        let shape = mi.width as f64 / mi.height as f64;
                        let panel = pw as f64 / ph as f64;
                        mi.fits_screen = (shape / panel - 1.0).abs() < SHAPE_TOLERANCE;
                    }
                    if !modes.iter().any(|(o, _)| o.id == mi.id) {
                        modes.push((mi, m));
                    }
                }
                let current = card
                    .get_crtc(crtc)
                    .ok()
                    .and_then(|c| c.mode())
                    .map(|m| describe(&m));

                return Ok(Self {
                    card,
                    connector: handle,
                    crtc,
                    modes,
                    current,
                    background: None,
                    logo: None,
                });
            }
        }
        Err("No screen connected to HDMI".into())
    }

    /// Up to 1080p, as close to 60 Hz as the screen goes, then the biggest.
    /// Not the screen's own preference: 4K screens prefer 4K, which the Pi
    /// can only drive at 30 Hz
    fn best_for_video(&self) -> Option<&(ModeInfo, Mode)> {
        self.modes
            .iter()
            .filter(|(m, _)| m.fits_screen && m.width <= 1920 && m.height <= 1080)
            .min_by_key(|(m, _)| {
                let off_60 = ((m.refresh - 60.0).abs() * 100.0) as u32;
                (
                    off_60 > 100,
                    std::cmp::Reverse(m.width as u32 * m.height as u32),
                    off_60,
                )
            })
    }

    pub fn modes(&self) -> Vec<ModeInfo> {
        self.modes.iter().map(|(m, _)| m.clone()).collect()
    }

    pub fn current(&self) -> Option<&ModeInfo> {
        self.current.as_ref()
    }

    /// kmssink borrows this and never closes it, so a copy would leak and
    /// keep control of the screen after we let go. Stop the pipeline before
    /// dropping the Display
    pub fn fd(&self) -> RawFd {
        self.card.0.as_raw_fd()
    }

    /// Switch to `wanted`, or "auto" for the best mode for video. A mode the
    /// screen doesn't list falls back the same way, so a settings file moved
    /// to another screen still shows a picture
    pub fn set_mode(&mut self, wanted: &str) -> Result<ModeInfo, String> {
        let with_type = |t: ModeTypeFlags| {
            self.modes
                .iter()
                .find(move |(_, m)| m.mode_type().contains(t))
        };
        let (info, mode) = self
            .modes
            .iter()
            .find(|(m, _)| wanted != AUTO && m.id == wanted)
            .or_else(|| self.best_for_video())
            .or_else(|| with_type(ModeTypeFlags::PREFERRED))
            .or_else(|| with_type(ModeTypeFlags::USERDEF))
            .or_else(|| self.modes.first())
            .cloned()
            .ok_or("The screen lists no usable modes")?;
        if wanted != AUTO && info.id != wanted {
            eprintln!(
                "[hdmi] {wanted} isn't offered by this screen, using {}",
                info.id
            );
        }

        let (w, h) = mode.size();
        let mut buffer = self
            .card
            .create_dumb_buffer((w as u32, h as u32), DrmFourcc::Xrgb8888, 32)
            .map_err(|e| format!("background buffer: {e}"))?;
        if let Ok(mut map) = self.card.map_dumb_buffer(&mut buffer) {
            map.as_mut().fill(0);
        }
        let fb = self
            .card
            .add_framebuffer(&buffer, 24, 32)
            .map_err(|e| format!("background framebuffer: {e}"))?;

        if let Err(e) =
            self.card
                .set_crtc(self.crtc, Some(fb), (0, 0), &[self.connector], Some(mode))
        {
            let _ = self.card.destroy_framebuffer(fb);
            let _ = self.card.destroy_dumb_buffer(buffer);
            return Err(format!("Couldn't switch to {}: {e}", info.id));
        }

        if let Some((old_fb, old_buf)) = self.background.replace((fb, buffer)) {
            let _ = self.card.destroy_framebuffer(old_fb);
            let _ = self.card.destroy_dumb_buffer(old_buf);
        }
        eprintln!("[hdmi] mode {}", info.id);
        self.current = Some(info.clone());
        Ok(info)
    }
}

impl Display {
    /// Draw `logo` at `at` (screen pixels), scaled by the display hardware.
    /// `rotation` is the plane rotation bitmask the video uses, so the logo
    /// turns with it
    pub fn show_logo(
        &mut self,
        logo: &Logo,
        version: u64,
        at: Rect,
        opacity_percent: u8,
        rotation: u64,
    ) -> Result<(), String> {
        if self.logo.as_ref().is_some_and(|l| l.version != version) {
            self.hide_logo();
        }
        if self.logo.is_none() {
            let plane = self.free_overlay_plane()?;
            let (fb, buffer) = self.upload(logo)?;
            self.logo = Some(LogoPlane {
                plane,
                fb,
                buffer,
                version,
                size: (logo.width, logo.height),
            });
        }
        let l = self.logo.as_ref().expect("set above");
        let alpha = ALPHA_OPAQUE * opacity_percent.min(100) as u64 / 100;
        for (name, value) in [
            ("zpos", LOGO_ZPOS),
            ("pixel blend mode", BLEND_COVERAGE),
            ("alpha", alpha),
            ("rotation", rotation),
        ] {
            // Best effort: a driver without one of these still shows the logo
            let _ = self.set_plane_property(l.plane, name, value);
        }
        self.card
            .set_plane(
                l.plane,
                self.crtc,
                Some(l.fb),
                0,
                (at.x, at.y, at.w, at.h),
                // Source rectangle in 16.16 fixed point: the whole image
                (0, 0, l.size.0 << 16, l.size.1 << 16),
            )
            .map_err(|e| format!("Couldn't place the logo: {e}"))
    }

    pub fn hide_logo(&mut self) {
        if let Some(l) = self.logo.take() {
            let _ = self
                .card
                .set_plane(l.plane, self.crtc, None, 0, (0, 0, 0, 0), (0, 0, 0, 0));
            let _ = self.card.destroy_framebuffer(l.fb);
            let _ = self.card.destroy_dumb_buffer(l.buffer);
        }
    }

    fn upload(&self, logo: &Logo) -> Result<(framebuffer::Handle, DumbBuffer), String> {
        let mut buffer = self
            .card
            .create_dumb_buffer((logo.width, logo.height), DrmFourcc::Argb8888, 32)
            .map_err(|e| format!("logo buffer: {e}"))?;
        let pitch = buffer.pitch() as usize;
        let row = logo.width as usize * 4;
        {
            let mut map = self
                .card
                .map_dumb_buffer(&mut buffer)
                .map_err(|e| format!("logo buffer: {e}"))?;
            let pixels = map.as_mut();
            // ARGB8888 is B, G, R, A in memory, which is how the logo is kept
            for (y, src) in logo.bgra.chunks_exact(row).enumerate() {
                if let Some(dst) = pixels.get_mut(y * pitch..y * pitch + row) {
                    dst.copy_from_slice(src);
                }
            }
        }
        match self.card.add_framebuffer(&buffer, 32, 32) {
            Ok(fb) => Ok((fb, buffer)),
            Err(e) => {
                let _ = self.card.destroy_dumb_buffer(buffer);
                Err(format!("logo framebuffer: {e}"))
            }
        }
    }

    /// From the end of the list: kmssink takes the first free one for the
    /// video, and the two must not land on the same plane
    fn free_overlay_plane(&self) -> Result<plane::Handle, String> {
        let res = self.card.resource_handles().map_err(|e| e.to_string())?;
        let planes = self.card.plane_handles().map_err(|e| e.to_string())?;
        planes
            .into_iter()
            .rev()
            .find(|&p| {
                let Ok(info) = self.card.get_plane(p) else {
                    return false;
                };
                info.crtc().is_none()
                    && res.filter_crtcs(info.possible_crtcs()).contains(&self.crtc)
                    && info.formats().contains(&(DrmFourcc::Argb8888 as u32))
                    && self.plane_property(p, "type") == Some(PLANE_TYPE_OVERLAY)
            })
            .ok_or_else(|| "No free display plane for the logo".to_string())
    }

    fn find_property(
        &self,
        plane: plane::Handle,
        name: &str,
    ) -> Option<(drm::control::property::Handle, u64)> {
        let props = self.card.get_properties(plane).ok()?;
        props.iter().find_map(|(&handle, &value)| {
            let info = self.card.get_property(handle).ok()?;
            (info.name().to_str().ok()? == name).then_some((handle, value))
        })
    }

    fn plane_property(&self, plane: plane::Handle, name: &str) -> Option<u64> {
        self.find_property(plane, name).map(|(_, value)| value)
    }

    fn set_plane_property(&self, plane: plane::Handle, name: &str, value: u64) -> Option<()> {
        let (handle, current) = self.find_property(plane, name)?;
        if current != value {
            self.card.set_property(plane, handle, value).ok()?;
        }
        Some(())
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        self.hide_logo();
        if let Some((fb, buf)) = self.background.take() {
            let _ = self.card.destroy_framebuffer(fb);
            let _ = self.card.destroy_dumb_buffer(buf);
        }
    }
}

fn describe(m: &Mode) -> ModeInfo {
    let (w, h) = m.size();
    let (_, _, htotal) = m.hsync();
    let (_, _, vtotal) = m.vsync();
    let refresh = if htotal > 0 && vtotal > 0 {
        m.clock() as f64 * 1000.0 / (htotal as f64 * vtotal as f64)
    } else {
        m.vrefresh() as f64
    };
    ModeInfo {
        id: format!("{w}x{h}@{refresh:.2}"),
        width: w,
        height: h,
        refresh,
        preferred: m.mode_type().contains(ModeTypeFlags::PREFERRED),
        fits_screen: true,
    }
}
