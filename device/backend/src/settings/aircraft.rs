use serde::{Deserialize, Serialize};

/// Controls for the aircraft beyond the camera settings every air unit takes
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct AircraftSettings {
    /// Let the browser and a gamepad move the camera. Off unless asked for:
    /// while the browser holds an Avata 2's gimbal, the remote can't move it
    pub gimbal_control: bool,
    /// The goggles don't report the Avata 360 by a code YStreamer knows, and
    /// it takes its own commands for the view and the lens
    pub avata360: bool,
}

#[cfg(test)]
#[path = "aircraft.test.rs"]
mod tests;
