use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::gadget::{EP_IN, EP_OUT};

const UDC_DIR: &str = "/sys/class/udc";
const SUSPENDED_PATH: &str = "/sys/bus/gadget/devices/gadget.0/suspended";

const SILENCE_TIMEOUT: Duration = Duration::from_secs(8);
const RETRY_BACKOFF: Duration = Duration::from_secs(2);
const POLL_SLICE: Duration = Duration::from_millis(250);
// A short blip isn't always enough for the goggles to drop the old session
const REPLUG_OFF: Duration = Duration::from_secs(3);
const FIRST_BYTES_LOGGED: usize = 32;
// Frames waiting for the goggles to collect them
const SEND_QUEUE: usize = 64;

/// What the reader thread hands the session
enum Rx {
    Data(Vec<u8>),
    Closed(io::Error),
}

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
    /// Every write to the goggles (registration, replies, commands) goes
    /// through this one queue, so frames can never interleave mid-packet
    outbox: SyncSender<Vec<u8>>,
    /// The other end of `outbox`, until `run` starts the thread that writes
    unsent: Arc<Mutex<Option<Receiver<Vec<u8>>>>>,
    open: Arc<AtomicBool>,
    reset: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
    generation: Arc<AtomicU64>,
    frames: Arc<AtomicU64>,
}

impl Goggles {
    pub fn new() -> Self {
        let (outbox, unsent) = mpsc::sync_channel(SEND_QUEUE);
        Self {
            outbox,
            unsent: Arc::new(Mutex::new(Some(unsent))),
            open: Arc::new(AtomicBool::new(false)),
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
        let not_connected = || io::Error::new(io::ErrorKind::NotConnected, "goggles not connected");
        if !self.open.load(Ordering::Relaxed) {
            return Err(not_connected());
        }
        self.outbox.try_send(bytes.to_vec()).map_err(|e| match e {
            TrySendError::Full(_) => {
                io::Error::new(io::ErrorKind::WouldBlock, "goggles aren't taking data")
            }
            TrySendError::Disconnected(_) => not_connected(),
        })
    }

    pub fn reset(&self) {
        self.reset.store(true, Ordering::Relaxed);
    }

    /// Own the device forever: wait for the goggles and hand every read to
    /// `on_data`, reconnecting whenever the link drops
    pub fn run(&self, mut on_data: impl FnMut(&[u8])) -> ! {
        if let Some(unsent) = self.unsent.lock().unwrap().take() {
            thread::spawn(move || write_loop(unsent));
        }
        let (inbox_tx, inbox) = mpsc::channel();
        thread::spawn(move || read_loop(inbox_tx));
        let mut quiet = false;
        loop {
            if !link_up() {
                self.set_state(LinkState::Unplugged);
                wait_for_link();
            }
            self.set_state(LinkState::Connecting);

            match self.run_session(&mut on_data, &inbox, quiet) {
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

    fn run_session(
        &self,
        on_data: &mut impl FnMut(&[u8]),
        inbox: &Receiver<Rx>,
        quiet: bool,
    ) -> SessionEnd {
        self.reset.store(false, Ordering::Relaxed);

        // A close left over from the last session isn't this one's. Data
        // is kept: the remote sends its only parameter sets right away
        let mut next = loop {
            match inbox.try_recv() {
                Ok(Rx::Closed(_)) => continue,
                Ok(data) => break Some(data),
                Err(_) => break None,
            }
        };

        self.open.store(true, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Relaxed);
        if !quiet {
            eprintln!("[usb] Device open");
        }

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

            let rx = match next.take() {
                Some(rx) => Ok(rx),
                None => inbox.recv_timeout(POLL_SLICE),
            };
            match rx {
                Ok(Rx::Data(data)) => {
                    let n = data.len();
                    bytes += n as u64;
                    if first.len() < FIRST_BYTES_LOGGED {
                        let take = n.min(FIRST_BYTES_LOGGED - first.len());
                        first.extend_from_slice(&data[..take]);
                    }
                    on_data(&data);
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
                Ok(Rx::Closed(e)) => {
                    eprintln!("[usb] Link closed: {e}");
                    break SessionEnd::Retry;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break SessionEnd::Retry,
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

        self.open.store(false, Ordering::Relaxed);
        end
    }
}

/// Reads the link for as long as the process runs. A read blocks until the
/// other end sends something (the endpoint can't be polled), so reading is
/// done here, which leaves the session free to notice silence and resets
fn read_loop(inbox: Sender<Rx>) {
    let mut buf = vec![0u8; 128 * 1024];
    let mut logged = false;
    loop {
        if !link_up() {
            thread::sleep(POLL_SLICE);
            continue;
        }
        let mut ep = match File::open(EP_OUT) {
            Ok(ep) => ep,
            Err(e) => {
                if !logged {
                    eprintln!("[usb] Waiting for {EP_OUT}: {e}");
                    logged = true;
                }
                thread::sleep(POLL_SLICE);
                continue;
            }
        };
        logged = false;
        let closed = loop {
            match ep.read(&mut buf) {
                // A zero-length packet, which the remote sends; a link that
                // goes down fails the read instead
                Ok(0) => {}
                Ok(n) => {
                    if inbox.send(Rx::Data(buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => break e,
            }
        };
        if inbox.send(Rx::Closed(closed)).is_err() {
            return;
        }
        thread::sleep(POLL_SLICE);
    }
}

/// Writes queued frames. A write returns once the other end has collected
/// it, or fails when the link goes down; the frame is dropped then
fn write_loop(unsent: Receiver<Vec<u8>>) {
    let mut ep: Option<File> = None;
    for frame in unsent {
        if ep.is_none() {
            ep = File::options().write(true).open(EP_IN).ok();
        }
        let sent = ep
            .as_mut()
            .is_some_and(|file| file.write_all(&frame).is_ok());
        if !sent {
            ep = None;
        }
    }
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
