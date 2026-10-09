use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const DEVICE_PATH: &str = "/dev/ttyGS0";
const UDC_DIR: &str = "/sys/class/udc";
const SUSPENDED_PATH: &str = "/sys/bus/gadget/devices/gadget.0/suspended";

const SILENCE_TIMEOUT: Duration = Duration::from_secs(8);
const RETRY_BACKOFF: Duration = Duration::from_secs(2);
const POLL_SLICE: Duration = Duration::from_millis(250);
// A short blip isn't always enough for the goggles to drop the old session
const REPLUG_OFF: Duration = Duration::from_secs(3);
const FIRST_BYTES_LOGGED: usize = 32;

/// Holds a writable clone of the current device fd, or None while disconnected.
/// All writes to the goggles (registration, replies, commands) go through this
/// single Mutex, so they can never interleave mid-packet
type WriteHandle = Arc<Mutex<Option<File>>>;

enum SessionEnd {
    LinkDown,
    Retry,
    NoAnswer,
    Replug,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkState {
    Unplugged,
    Connecting,
    Live,
}

#[derive(Clone)]
pub struct Goggles {
    write: WriteHandle,
    reset: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
    generation: Arc<AtomicU64>,
    frames: Arc<AtomicU64>,
}

impl Goggles {
    pub fn new() -> Self {
        Self {
            write: Arc::new(Mutex::new(None)),
            reset: Arc::new(AtomicBool::new(false)),
            state: Arc::new(AtomicU8::new(LinkState::Unplugged as u8)),
            generation: Arc::new(AtomicU64::new(0)),
            frames: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The parser found a valid frame. Bytes alone don't count: after a
    /// restart the goggles can send data that isn't the protocol at all
    pub fn mark_alive(&self) {
        self.frames.fetch_add(1, Ordering::Relaxed);
    }

    pub fn state(&self) -> LinkState {
        match self.state.load(Ordering::Relaxed) {
            s if s == LinkState::Live as u8 => LinkState::Live,
            s if s == LinkState::Connecting as u8 => LinkState::Connecting,
            _ => LinkState::Unplugged,
        }
    }

    /// Changes whenever a new session opens the device, so protocol state
    /// tied to the old one can be dropped
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    fn set_state(&self, state: LinkState) {
        self.state.store(state as u8, Ordering::Relaxed);
    }

    pub fn send(&self, bytes: &[u8]) -> io::Result<()> {
        match self.write.lock().unwrap().as_mut() {
            Some(file) => {
                file.write_all(bytes)?;
                file.flush()
            }
            None => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "goggles not connected",
            )),
        }
    }

    pub fn reset(&self) {
        self.reset.store(true, Ordering::Relaxed);
    }

    /// Own the device forever: wait for the goggles and hand every read to
    /// `on_data`, reconnecting whenever the link drops
    pub fn run(&self, mut on_data: impl FnMut(&[u8])) -> ! {
        let mut quiet = false;
        loop {
            if !link_up() {
                self.set_state(LinkState::Unplugged);
                wait_for_link();
            }
            self.set_state(LinkState::Connecting);

            match self.run_session(&mut on_data, quiet) {
                SessionEnd::LinkDown => {
                    eprintln!("[usb] Goggles disconnected");
                    quiet = false;
                    // Let the gadget settle before trusting the state again
                    thread::sleep(Duration::from_millis(500));
                }
                SessionEnd::Retry => {
                    eprintln!("[usb] Reconnecting in {:?}", RETRY_BACKOFF);
                    quiet = false;
                    thread::sleep(RETRY_BACKOFF);
                }
                SessionEnd::NoAnswer => {
                    if !quiet {
                        eprintln!("[usb] Goggles not talking, reconnecting the USB link");
                    }
                    quiet = true;
                    if !replug() {
                        thread::sleep(RETRY_BACKOFF);
                    }
                }
                SessionEnd::Replug => {
                    quiet = false;
                    if !replug() {
                        thread::sleep(RETRY_BACKOFF);
                    }
                }
            }
        }
    }

    fn run_session(&self, on_data: &mut impl FnMut(&[u8]), quiet: bool) -> SessionEnd {
        self.reset.store(false, Ordering::Relaxed);

        let Some(file) = open_device() else {
            return if link_up() {
                eprintln!("[usb] Could not open {}", DEVICE_PATH);
                SessionEnd::Retry
            } else {
                SessionEnd::LinkDown
            };
        };

        // Publish a writable clone so registration + commands can reach the fd
        match file.try_clone() {
            Ok(w) => *self.write.lock().unwrap() = Some(w),
            Err(e) => {
                eprintln!("[usb] write-clone failed: {}", e);
                return SessionEnd::Retry;
            }
        }
        self.generation.fetch_add(1, Ordering::Relaxed);
        if !quiet {
            eprintln!("[usb] Device open");
        }

        let mut reader = file; // read side (writes go via the handle clone)
        let mut buf = vec![0u8; 128 * 1024];
        let mut last_rx = Instant::now();
        let mut saw_telemetry = false;
        let mut frames = self.frames.load(Ordering::Relaxed);
        let mut bytes: u64 = 0;
        let mut first = Vec::new();

        let end = loop {
            if !link_up() {
                break SessionEnd::LinkDown;
            }

            if self.reset.load(Ordering::Relaxed) {
                eprintln!("[usb] Link reset requested");
                break SessionEnd::Replug;
            }

            if wait_readable(&reader, POLL_SLICE) {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        eprintln!("[usb] EOF on {}", DEVICE_PATH);
                        break SessionEnd::Retry;
                    }
                    Ok(n) => {
                        bytes += n as u64;
                        if first.len() < FIRST_BYTES_LOGGED {
                            let take = n.min(FIRST_BYTES_LOGGED - first.len());
                            first.extend_from_slice(&buf[..take]);
                        }
                        on_data(&buf[..n]);
                        let now = self.frames.load(Ordering::Relaxed);
                        if now != frames {
                            frames = now;
                            last_rx = Instant::now();
                            if !saw_telemetry {
                                saw_telemetry = true;
                                self.set_state(LinkState::Live);
                                eprintln!("[usb] Link is live");
                            }
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => {
                        eprintln!("[usb] Read error: {}", e);
                        break SessionEnd::Retry;
                    }
                }
            }

            if last_rx.elapsed() > SILENCE_TIMEOUT {
                if !saw_telemetry {
                    if bytes > 0 && !quiet {
                        eprintln!(
                            "[usb] Got {bytes} bytes but no valid frames (starting {})",
                            hex::encode(&first)
                        );
                    }
                    break SessionEnd::NoAnswer;
                }
                eprintln!(
                    "[usb] No telemetry for {:?}, link went dead",
                    SILENCE_TIMEOUT
                );
                break SessionEnd::Replug;
            }
        };

        // Clear the write handle BEFORE `reader` drops, so every device fd
        // clone is gone before the next open
        *self.write.lock().unwrap() = None;
        end
    }
}

fn set_raw(file: &File) -> anyhow::Result<()> {
    let fd = file.as_raw_fd();
    unsafe {
        let mut tios: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut tios) != 0 {
            return Err(anyhow::anyhow!("tcgetattr failed"));
        }
        libc::cfmakeraw(&mut tios);
        if libc::tcsetattr(fd, libc::TCSANOW, &tios) != 0 {
            return Err(anyhow::anyhow!("tcsetattr failed"));
        }
        libc::tcflush(fd, libc::TCIFLUSH);
    }
    Ok(())
}

fn udc() -> Option<PathBuf> {
    fs::read_dir(UDC_DIR)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.join("state").exists())
}

fn udc_state() -> Option<String> {
    let s = fs::read_to_string(udc()?.join("state")).ok()?;
    Some(s.trim().to_string())
}

/// Drop off the bus and come back. The goggles take it as a replug and
/// restart the accessory link, which is the only way out once they've given
/// up on it (e.g. after we were restarted while plugged in)
fn replug() -> bool {
    let Some(path) = udc().map(|u| u.join("soft_connect")) else {
        return false;
    };
    if let Err(e) = fs::write(&path, "disconnect") {
        eprintln!("[usb] Can't disconnect from the bus: {e}");
        return false;
    }
    thread::sleep(REPLUG_OFF);
    if let Err(e) = fs::write(&path, "connect") {
        eprintln!("[usb] Can't reconnect to the bus: {e}");
    }
    true
}

/// The host stopped polling (cable still in, other end asleep)
fn is_suspended() -> bool {
    fs::read_to_string(SUSPENDED_PATH)
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

/// True only when a host has actually enumerated and configured the gadget.
///
/// `suspended` alone is not a connection test: it reads 0 whenever the gadget
/// merely isn't suspended, which includes "nothing is plugged in at all". The
/// UDC state is the real signal - anything short of `configured` ("not
/// attached", or "powered" for a charge-only cable) means no usable link
fn link_up() -> bool {
    match udc_state() {
        Some(state) => state == "configured" && !is_suspended(),
        None => !is_suspended(),
    }
}

fn wait_for_link() {
    if link_up() {
        return;
    }
    eprintln!("[usb] Waiting for goggles");
    while !link_up() {
        thread::sleep(Duration::from_millis(100));
    }
    eprintln!("[usb] Goggles connected");
}

fn open_device() -> Option<File> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut logged = false;
    while link_up() && Instant::now() < deadline {
        match File::options().read(true).write(true).open(DEVICE_PATH) {
            Ok(f) if set_raw(&f).is_ok() => return Some(f),
            Ok(_) => eprintln!("[usb] {} is not a usable tty", DEVICE_PATH),
            Err(e) => {
                if !logged {
                    eprintln!("[usb] Waiting for {}: {}", DEVICE_PATH, e);
                    logged = true;
                }
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
    None
}

fn wait_readable(file: &File, timeout: Duration) -> bool {
    let mut pfd = libc::pollfd {
        fd: file.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let n = unsafe { libc::poll(&mut pfd, 1, timeout.as_millis() as libc::c_int) };
    n > 0 && pfd.revents != 0
}
