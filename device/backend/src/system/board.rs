use std::fs;
use std::sync::OnceLock;

/// The board this runs on, from the device tree. Only a hint: what has a
/// runtime probe (the decoder, the display plane, the USB controller) goes
/// by the probe. The board decides what hasn't got one: whether the reset pin
/// means anything, and what to tell someone whose USB port isn't in device
/// mode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Board {
    RaspberryPi,
    /// Radxa ROCK 3A, and other boards with a Rockchip chip
    Rockchip,
    Other,
}

static BOARD: OnceLock<Board> = OnceLock::new();

impl Board {
    pub fn detect() -> Self {
        *BOARD.get_or_init(|| {
            let read = |p: &str| fs::read_to_string(p).unwrap_or_default();
            Self::from_device_tree(
                &read("/proc/device-tree/model"),
                &read("/proc/device-tree/compatible"),
            )
        })
    }

    /// `compatible` is a list of NUL-separated strings, most specific first
    pub fn from_device_tree(model: &str, compatible: &str) -> Self {
        let model = model.trim_end_matches('\0').trim();
        let compatible: Vec<&str> = compatible
            .split('\0')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let compatible_with = |prefix: &str| compatible.iter().any(|c| c.starts_with(prefix));
        if model.starts_with("Raspberry Pi") || compatible_with("raspberrypi,") {
            Self::RaspberryPi
        } else if compatible_with("rockchip,")
            || compatible_with("radxa,")
            || model.contains("ROCK")
        {
            Self::Rockchip
        } else {
            Self::Other
        }
    }
}

#[cfg(test)]
#[path = "board.test.rs"]
mod tests;
