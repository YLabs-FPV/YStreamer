mod vmx;
mod wire;

use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::ops::RangeInclusive;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver as Queue, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::decoded::{Decoder, Limit, Picture};
use crate::settings::NetworkSourceSettings;

const POLL: Duration = Duration::from_secs(1);
/// Keep encoding through a receiver reconnecting, e.g. vMix switching inputs
const LINGER: Duration = Duration::from_secs(5);
/// OMT senders take the first free port here; receivers find it by mDNS
const PORTS: RangeInclusive<u16> = 6400..=6600;
const ACCEPT_POLL: Duration = Duration::from_millis(100);
/// Frames waiting for a receiver. A slower one misses pictures instead of
/// falling behind
const QUEUE: usize = 2;
/// Kept small for the same reason: Linux would otherwise buffer seconds of
/// video for a receiver that can't keep up
const SEND_BUFFER: libc::c_int = 256 * 1024;
/// A receiver that takes nothing for this long is gone
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// avahi-daemon announces what's in its services folder
const SERVICE_FILE: &str = "/etc/avahi/services/ystreamer-omt.service";

#[derive(Serialize, Clone, Default, Debug, PartialEq)]
pub struct OmtStatus {
    pub active: bool,
    pub port: Option<u16>,
    /// Announced over mDNS, so receivers list it by themselves
    pub announced: bool,
    /// Receivers taking video
    pub receivers: u32,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub error: Option<String>,
}

struct Flags {
    video: AtomicBool,
    alive: AtomicBool,
}

struct Connection {
    flags: Arc<Flags>,
    queue: SyncSender<Arc<Vec<u8>>>,
}

/// Takes receivers' connections and sends frames to those that asked for
/// video
pub struct Server {
    port: u16,
    connections: Arc<Mutex<Vec<Connection>>>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl Server {
    pub fn start(ports: RangeInclusive<u16>) -> Result<Self, String> {
        let (listener, port) = ports
            .clone()
            .find_map(|p| TcpListener::bind(("0.0.0.0", p)).ok().map(|l| (l, p)))
            .ok_or_else(|| format!("No free port between {} and {}", ports.start(), ports.end()))?;
        // Polled, so stopping doesn't need a connection to wake it up
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("OMT port {port}: {e}"))?;
        let connections = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let accept = {
            let connections = Arc::clone(&connections);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if let Some(c) = connect(stream) {
                                connections.lock().unwrap().push(c);
                            }
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL),
                        Err(_) => thread::sleep(ACCEPT_POLL),
                    }
                }
            })
        };
        Ok(Self {
            port,
            connections,
            stop,
            accept: Some(accept),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Receivers that asked for video; forgets the ones that left
    pub fn watching(&self) -> u32 {
        let mut connections = self.connections.lock().unwrap();
        connections.retain(|c| c.flags.alive.load(Ordering::Relaxed));
        connections
            .iter()
            .filter(|c| c.flags.video.load(Ordering::Relaxed))
            .count() as u32
    }

    pub fn send(&self, frame: Arc<Vec<u8>>) {
        for c in self.connections.lock().unwrap().iter() {
            if c.flags.video.load(Ordering::Relaxed) {
                match c.queue.try_send(Arc::clone(&frame)) {
                    Ok(()) | Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Disconnected(_)) => {
                        c.flags.alive.store(false, Ordering::Relaxed)
                    }
                }
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        // Closing the queues ends each connection's threads
        self.connections.lock().unwrap().clear();
    }
}

/// A writer thread drains the queue onto the socket; a reader thread
/// listens for the receiver asking for video
fn connect(stream: TcpStream) -> Option<Connection> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    // Safety: a valid socket and an int option
    unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            (&SEND_BUFFER as *const libc::c_int).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
    let reader = stream.try_clone().ok()?;
    let flags = Arc::new(Flags {
        video: AtomicBool::new(false),
        alive: AtomicBool::new(true),
    });
    let (queue, frames) = sync_channel(QUEUE);
    {
        let flags = Arc::clone(&flags);
        thread::spawn(move || write_frames(stream, frames, &flags));
    }
    {
        let flags = Arc::clone(&flags);
        thread::spawn(move || read_requests(reader, &flags));
    }
    Some(Connection { flags, queue })
}

fn write_frames(mut stream: TcpStream, frames: Queue<Arc<Vec<u8>>>, flags: &Flags) {
    let hello = wire::metadata(&wire::sender_info(env!("CARGO_PKG_VERSION")));
    if stream.write_all(&hello).is_ok() {
        while let Ok(frame) = frames.recv() {
            if stream.write_all(&frame).is_err() {
                break;
            }
        }
    }
    flags.alive.store(false, Ordering::Relaxed);
    let _ = stream.shutdown(Shutdown::Both);
}

fn read_requests(mut stream: TcpStream, flags: &Flags) {
    let mut header = [0u8; wire::HEADER_LEN];
    while stream.read_exact(&mut header).is_ok() {
        let Some(h) = wire::parse_header(&header) else {
            break;
        };
        if h.data_len > wire::MAX_INCOMING {
            break;
        }
        let mut data = vec![0; h.data_len];
        if stream.read_exact(&mut data).is_err() {
            break;
        }
        if h.kind == wire::METADATA && wire::wants_video(&data) {
            flags.video.store(true, Ordering::Relaxed);
        }
    }
    flags.alive.store(false, Ordering::Relaxed);
}

struct Running {
    server: Arc<Server>,
    limit: Limit,
    /// Width and height packed into one, 0 before the first picture
    size: Arc<AtomicU64>,
    idle_since: Option<Instant>,
}

/// Open Media Transport: an open, royalty-free alternative to NDI that vMix
/// and OBS (with a plugin) receive. The video is decoded and compressed again
/// as VMX1, in software, only while a receiver asks for it
pub struct OmtOutput {
    running: Mutex<Option<Running>>,
    decoder: Mutex<Option<Decoder>>,
    status: Mutex<OmtStatus>,
}

impl OmtOutput {
    pub fn new(cfg: &NetworkSourceSettings) -> Arc<Self> {
        let this = Arc::new(Self {
            running: Mutex::new(None),
            decoder: Mutex::new(None),
            status: Mutex::new(OmtStatus::default()),
        });
        if let Err(e) = this.apply(cfg) {
            eprintln!("[omt] not started: {e}");
        }
        let watcher = Arc::clone(&this);
        thread::spawn(move || {
            loop {
                thread::sleep(POLL);
                watcher.supervise();
            }
        });
        this
    }

    pub fn status(&self) -> OmtStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn apply(&self, cfg: &NetworkSourceSettings) -> Result<(), String> {
        let mut running = self.running.lock().unwrap();
        // Taken out first: stopping it waits for its thread, and the video
        // shouldn't wait behind the lock meanwhile
        let old = self.decoder.lock().unwrap().take();
        drop(old);
        if running.take().is_some() {
            eprintln!("[omt] stopped");
        }
        *self.status.lock().unwrap() = OmtStatus::default();
        if !cfg.enabled {
            withdraw();
            return Ok(());
        }

        let result = vmx::Encoder::new(16, 16).and_then(|_| Server::start(PORTS));
        let mut status = self.status.lock().unwrap();
        match result {
            Ok(server) => {
                let port = server.port();
                let name = cfg.name.trim();
                eprintln!("[omt] source \"{name}\" on port {port}");
                status.announced = match announce(name, port) {
                    Ok(()) => true,
                    Err(e) => {
                        eprintln!("[omt] not announced over mDNS: {e}");
                        false
                    }
                };
                status.active = true;
                status.port = Some(port);
                *running = Some(Running {
                    server: Arc::new(server),
                    limit: Limit {
                        max_height: cfg.max_height,
                        max_fps: cfg.max_fps,
                    },
                    size: Arc::new(AtomicU64::new(0)),
                    idle_since: None,
                });
                Ok(())
            }
            Err(e) => {
                withdraw();
                status.error = Some(e.clone());
                Err(e)
            }
        }
    }

    pub fn push(&self, chunk: &[u8]) {
        if let Some(decoder) = self.decoder.lock().unwrap().as_mut() {
            decoder.push(chunk);
        }
    }

    pub fn push_au(&self, au: &[u8]) {
        if let Some(decoder) = self.decoder.lock().unwrap().as_ref() {
            decoder.push_au(au);
        }
    }

    /// Encodes while someone watches, and keeps the status current
    fn supervise(&self) {
        // Dropped last, once the video can get at the decoder slot again
        let mut retired = None;
        let mut running = self.running.lock().unwrap();
        let Some(r) = running.as_mut() else {
            return;
        };
        let receivers = r.server.watching();
        let mut decoder = self.decoder.lock().unwrap();
        let mut error = decoder.as_ref().and_then(Decoder::error);
        if error.is_some() {
            // Started again on the next round
            retired = decoder.take();
        }

        if receivers > 0 {
            r.idle_since = None;
            if decoder.is_none() && error.is_none() {
                match Decoder::start(r.limit, encoding(r)) {
                    Ok(d) => {
                        eprintln!("[omt] {receivers} receiver(s), encoding");
                        *decoder = Some(d);
                    }
                    Err(e) => error = Some(e),
                }
            }
        } else if decoder.is_some() {
            let since = *r.idle_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= LINGER {
                retired = decoder.take();
                r.size.store(0, Ordering::Relaxed);
                eprintln!("[omt] no receivers, encoding stopped");
            }
        }

        let size = r.size.load(Ordering::Relaxed);
        let mut status = self.status.lock().unwrap();
        status.receivers = receivers;
        status.width = (size != 0).then_some((size >> 32) as u32);
        status.height = (size != 0).then_some(size as u32);
        if let Some(e) = error {
            if status.error.as_deref() != Some(&e) {
                eprintln!("[omt] {e}");
            }
            status.error = Some(e);
        } else if decoder.is_some() {
            status.error = None;
        }
        drop(status);
        drop(decoder);
        drop(running);
        drop(retired);
    }
}

/// What the decoder does with each picture: compress it and send it out
fn encoding(r: &Running) -> impl FnMut(&Picture) + Send + 'static {
    let server = Arc::clone(&r.server);
    let size = Arc::clone(&r.size);
    let max_fps = r.limit.max_fps;
    let started = Instant::now();
    let mut encoder: Option<vmx::Encoder> = None;
    move |picture| {
        let wanted = (picture.width, picture.height);
        if encoder.as_ref().map(vmx::Encoder::size) != Some(wanted) {
            encoder = vmx::Encoder::new(wanted.0, wanted.1)
                .inspect_err(|e| eprintln!("[omt] {e}"))
                .ok();
        }
        let Some(data) = encoder.as_mut().and_then(|e| e.encode(picture)) else {
            return;
        };
        let timestamp = (started.elapsed().as_nanos() / 100) as i64;
        let frame = wire::video(data, wanted.0, wanted.1, picture.rate(max_fps), timestamp);
        server.send(Arc::new(frame));
        size.store((wanted.0 as u64) << 32 | wanted.1 as u64, Ordering::Relaxed);
    }
}

/// Receivers parse the name back as "machine (source)"; avahi fills in the
/// machine
fn service(name: &str, port: u16) -> String {
    format!(
        "<?xml version=\"1.0\" standalone='no'?>\n\
         <!DOCTYPE service-group SYSTEM \"avahi-service.dtd\">\n\
         <service-group>\n  \
           <name replace-wildcards=\"yes\">%h ({name})</name>\n  \
           <service>\n    \
             <type>_omt._tcp</type>\n    \
             <port>{port}</port>\n  \
           </service>\n\
         </service-group>\n"
    )
}

fn announce(name: &str, port: u16) -> Result<(), String> {
    fs::write(SERVICE_FILE, service(name, port)).map_err(|e| format!("{SERVICE_FILE}: {e}"))
}

fn withdraw() {
    let _ = fs::remove_file(SERVICE_FILE);
}

#[cfg(test)]
#[path = "omt.test.rs"]
mod tests;
