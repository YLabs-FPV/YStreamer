use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordingFormat {
    #[default]
    Mp4,
    Ts,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct RecordingSettings {
    pub path: String,
    pub format: RecordingFormat,
    pub prefix: String,
    /// 0 = one continuous file
    pub split_minutes: u16,
    pub auto_start: bool,
    pub reserve_mb: u32,
    pub button: ButtonSettings,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ButtonKind {
    /// Each press starts or stops recording
    #[default]
    Push,
    /// Records for as long as it's closed
    Switch,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ButtonSettings {
    pub enabled: bool,
    pub kind: ButtonKind,
    pub pin: u8,
    /// Driven high while recording
    pub led_pin: Option<u8>,
}

impl Default for ButtonSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: ButtonKind::Push,
            pin: 17,
            led_pin: None,
        }
    }
}

impl ButtonSettings {
    /// GPIO21 is missing: holding it to ground while starting resets the
    /// settings
    pub const PINS: [u8; 21] = [
        4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 16, 17, 18, 19, 20, 22, 23, 24, 25, 26, 27,
    ];

    pub fn validate(&self) -> Result<(), String> {
        let known = |pin: &u8| Self::PINS.contains(pin);
        if !known(&self.pin) || !self.led_pin.as_ref().is_none_or(known) {
            return Err("That GPIO pin can't be used".into());
        }
        if self.led_pin == Some(self.pin) {
            return Err("The button and its light need different pins".into());
        }
        Ok(())
    }
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            path: "/var/lib/ystreamer/recordings".into(),
            format: RecordingFormat::Mp4,
            prefix: "ystreamer".into(),
            split_minutes: 10,
            auto_start: false,
            reserve_mb: 500,
            button: ButtonSettings::default(),
        }
    }
}

impl RecordingSettings {
    pub fn validate(&self) -> Result<(), String> {
        self.button.validate()?;
        if !self.path.starts_with('/') {
            return Err("Recording folder must be an absolute path".into());
        }
        if self.path.contains('\0') {
            return Err("Recording folder is not a valid path".into());
        }
        if self.split_minutes > 240 {
            return Err("Split length must be 240 minutes or less".into());
        }
        if !(50..=100_000).contains(&self.reserve_mb) {
            return Err("Reserved space must be between 50 MB and 100 GB".into());
        }
        if !self
            .prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || self.prefix.is_empty()
        {
            return Err("File name prefix may only use letters, digits, '-' and '_'".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "recording.test.rs"]
mod tests;
