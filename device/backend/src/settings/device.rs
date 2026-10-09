use std::fs;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct DeviceSettings {
    pub hostname: String,
    /// An IANA name such as `Europe/Madrid`. Recordings are named in it
    pub timezone: String,
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            hostname: current_hostname(),
            timezone: current_timezone(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct AdvancedSettings {
    pub verbose_logs: bool,
    pub terminal: bool,
    /// Ask the update server now and then whether there's a newer version
    pub check_for_updates: bool,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        Self {
            verbose_logs: false,
            terminal: false,
            check_for_updates: true,
        }
    }
}

pub fn validate_hostname(h: &str) -> Result<(), String> {
    let ok = !h.is_empty()
        && h.len() <= 63
        && !h.starts_with('-')
        && !h.ends_with('-')
        && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(
            "Device name must be 1-63 letters, digits or '-', not starting or ending with '-'"
                .into(),
        )
    }
}

/// The form of a zone name; whether the system has it is for timedatectl
pub fn validate_timezone(tz: &str) -> Result<(), String> {
    let ok = !tz.is_empty()
        && tz.len() <= 64
        && !tz
            .split('/')
            .any(|part| part.is_empty() || part.starts_with(['.', '-']))
        && tz
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "'{tz}' isn't a timezone name, such as Europe/Madrid"
        ))
    }
}

/// What /etc/localtime points at; glibc reads it as UTC when it's missing
pub fn current_timezone() -> String {
    fs::read_link("/etc/localtime")
        .ok()
        .and_then(|target| zone_from_path(&target.to_string_lossy()))
        .unwrap_or_else(|| "Etc/UTC".into())
}

fn zone_from_path(path: &str) -> Option<String> {
    let (_, zone) = path.split_once("zoneinfo/")?;
    validate_timezone(zone).ok()?;
    Some(zone.to_string())
}

pub fn current_hostname() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "device.test.rs"]
mod tests;
