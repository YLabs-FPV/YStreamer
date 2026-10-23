use serde::{Deserialize, Serialize};

/// The transmit limits the goggles and aircraft radios follow, the same
/// choice DJI's own apps make by country
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RadioRegion {
    /// FCC, as in the United States
    #[default]
    Us,
    /// CE, the radios' factory setting
    Eu,
    Id,
}

impl RadioRegion {
    /// The country code the radios take; None for their factory CE setting
    pub fn country(self) -> Option<[u8; 2]> {
        match self {
            Self::Us => Some(*b"US"),
            Self::Eu => None,
            Self::Id => Some(*b"ID"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct RadioRegionSettings {
    /// Off leaves the radios as DJI's apps last set them
    pub enabled: bool,
    pub region: RadioRegion,
}

#[cfg(test)]
#[path = "radio.test.rs"]
mod tests;
