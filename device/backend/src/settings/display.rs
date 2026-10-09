use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct DisplaySettings {
    pub hdmi_output: bool,
    /// `WxH@Hz` as listed by the screen, or "auto" for its preferred mode.
    /// 1080p60 by default: 4K screens prefer 4K, which the Pi can't scan out
    /// at 60 fps
    pub mode: String,
    pub scaling: Scaling,
    pub rotate_180: bool,
    pub mirror: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            hdmi_output: true,
            mode: "1920x1080@60.00".into(),
            scaling: Scaling::Fit,
            rotate_180: false,
            mirror: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Scaling {
    #[default]
    Fit,
    Fill,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct OverlaySettings {
    pub enabled: bool,
    pub x_percent: f32,
    pub y_percent: f32,
    pub size_percent: f32,
    pub opacity_percent: u8,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            x_percent: 97.0,
            y_percent: 5.0,
            size_percent: 15.0,
            opacity_percent: 100,
        }
    }
}

impl OverlaySettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(2.0..=100.0).contains(&self.size_percent) {
            return Err("Logo size must be 2-100% of the picture width".into());
        }
        if !(0.0..=100.0).contains(&self.x_percent) || !(0.0..=100.0).contains(&self.y_percent) {
            return Err("Logo position must be within the picture".into());
        }
        if !(10..=100).contains(&self.opacity_percent) {
            return Err("Logo opacity must be 10-100%".into());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct SplashSettings {
    pub freeze_last_frame: bool,
    pub grayscale_freeze: bool,
    pub status_over_image: bool,
    pub image_scaling: Scaling,
    pub show_ap_password: bool,
}

impl Default for SplashSettings {
    fn default() -> Self {
        Self {
            freeze_last_frame: false,
            grayscale_freeze: true,
            status_over_image: true,
            image_scaling: Scaling::Fit,
            show_ap_password: true,
        }
    }
}
