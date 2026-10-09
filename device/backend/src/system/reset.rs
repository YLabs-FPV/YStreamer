use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use gpiocdev::Request;
use gpiocdev::line::{Bias, Value};

use super::button::CHIP;

/// Physical pin 40, the last one on the header, straight across from a
/// ground pin
pub const PIN: u8 = 21;
const HOLD: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(100);
const SHOWN_FOR: Duration = Duration::from_secs(10 * 60);

/// On the card's boot partition, which any computer can write to
const FILES: [&str; 2] = [
    "/boot/firmware/ystreamer-reset",
    "/boot/firmware/ystreamer-reset.txt",
];

struct Done {
    at: Instant,
    still_held: bool,
}

static DONE: OnceLock<Done> = OnceLock::new();

pub fn at_startup(settings: &Path) {
    let requested = request_file(settings);
    let by_file = requested.exists() || FILES.iter().any(|f| Path::new(f).exists());
    let pin = Pin::open();
    let by_pin = pin
        .as_ref()
        .is_some_and(|pin| held(|| pin.grounded(), HOLD, POLL, thread::sleep));
    if !by_file && !by_pin {
        return;
    }

    let how = if by_pin {
        "the reset pin"
    } else {
        "the reset file"
    };
    println!("[reset] {how} asks for the defaults");
    clear(settings);
    let _ = fs::remove_file(requested);
    for file in FILES {
        let _ = fs::remove_file(file);
    }
    let _ = DONE.set(Done {
        at: Instant::now(),
        still_held: pin.is_some_and(|pin| pin.grounded()),
    });
}

/// Ask for a reset at the next start, the way `ystreamer reset` does
pub fn request(settings: &Path) -> Result<(), String> {
    let file = request_file(settings);
    let failed = |e: std::io::Error| format!("Couldn't write {}: {e}", file.display());
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(failed)?;
    }
    fs::write(&file, "").map_err(failed)
}

fn request_file(settings: &Path) -> PathBuf {
    settings.with_file_name("reset-requested")
}

/// Saved network profiles outlive the settings, so they need applying again
pub fn happened() -> bool {
    DONE.get().is_some()
}

pub fn notice() -> Option<&'static str> {
    let done = DONE.get().filter(|done| done.at.elapsed() < SHOWN_FOR)?;
    Some(if done.still_held {
        "Settings were reset. Remove the reset jumper, or the next start resets them again"
    } else {
        "Settings were reset to the defaults"
    })
}

/// True once `grounded` has stayed true for all of `hold`. Letting go at
/// any point calls it off, so a brief touch does nothing
fn held(
    mut grounded: impl FnMut() -> bool,
    hold: Duration,
    poll: Duration,
    mut wait: impl FnMut(Duration),
) -> bool {
    let mut waited = Duration::ZERO;
    while grounded() {
        if waited >= hold {
            return true;
        }
        wait(poll);
        waited += poll;
    }
    false
}

/// The settings are kept aside rather than deleted, in case it was a mistake
fn clear(settings: &Path) {
    let kept = kept_copy(settings);
    match fs::rename(settings, &kept) {
        Ok(()) => println!("[reset] previous settings kept as {}", kept.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("[reset] couldn't set the settings aside: {e}"),
    }
    match fs::remove_file(settings.with_file_name("auth.json")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            eprintln!("[reset] couldn't remove the login: {e}")
        }
        _ => {}
    }
}

fn kept_copy(settings: &Path) -> PathBuf {
    settings.with_extension("before-reset.json")
}

struct Pin(Request);

impl Pin {
    fn open() -> Option<Self> {
        let request = Request::builder()
            .on_chip(CHIP)
            .with_consumer("ystreamer-reset")
            .with_line(PIN as u32)
            .as_input()
            .with_bias(Bias::PullUp)
            .request()
            .ok()?;
        // The pull-up needs a moment to lift an unconnected pin
        thread::sleep(Duration::from_millis(20));
        Some(Self(request))
    }

    fn grounded(&self) -> bool {
        matches!(self.0.value(PIN as u32), Ok(Value::Inactive))
    }
}

#[cfg(test)]
#[path = "reset.test.rs"]
mod tests;
