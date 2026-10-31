mod vmx;
mod wire;

use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::ops::RangeInclusive;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{
    Receiver as Queue, RecvTimeoutError, SyncSender, TrySendError, sync_channel,
};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::decoded::{Decoder, Limit, Picture, SharedDecoder};
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
/// Receivers subscribe as soon as they connect; anything that hasn't this
/// long after connecting isn't one, however much else it sent
const SUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Every connection costs two threads, so a busy LAN can't pile them up.
/// A mixer takes one each for video; eight leaves room for preview monitors
const MAX_RECEIVERS: usize = 8;
/// A receiver also opens a connection for audio, and counts itself
/// connected only while both are open. Those get their own places so they
/// don't take receivers'. Nothing is sent on them
const MAX_COMPANIONS: usize = 8;
/// Connections yet to subscribe have their own, smaller allowance, so they
/// can never keep receivers out
const MAX_PENDING: usize = 4;
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
    /// Subscribed to anything, so a receiver's
    client: AtomicBool,
    video: AtomicBool,
    alive: AtomicBool,
}

struct Connection {
    flags: Arc<Flags>,
    queue: SyncSender<Arc<Vec<u8>>>,
    /// Kept to close it from the accept thread
    socket: TcpStream,
    since: Instant,
}

impl Connection {
    fn subscribed(&self) -> bool {
        self.flags.client.load(Ordering::Relaxed)
    }

    fn watching(&self) -> bool {
        self.flags.video.load(Ordering::Relaxed)
    }

    /// Ends both of its threads: the reader sees the socket close, the
    /// writer sees `alive` drop
    fn close(&self) {
        self.flags.alive.store(false, Ordering::Relaxed);
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}

/// Forgets connections that ended, closes those past the deadline to
/// subscribe, and those that subscribed once their kind was full. Which
/// kind a connection is shows only once it subscribes, so that's where its
/// cap applies; the newest go, as they would have been turned away
fn sweep(connections: &mut Vec<Connection>) {
    let (mut receivers, mut companions) = (0, 0);
    // Oldest first, as they connected
    for c in connections.iter() {
        let (count, cap) = if c.watching() {
            (&mut receivers, MAX_RECEIVERS)
        } else if c.subscribed() {
            (&mut companions, MAX_COMPANIONS)
        } else {
            if c.since.elapsed() >= SUBSCRIBE_TIMEOUT {
                c.close();
            }
            continue;
        };
        if c.flags.alive.load(Ordering::Relaxed) {
            *count += 1;
            if *count > cap {
                c.close();
            }
        }
    }
    connections.retain(|c| c.flags.alive.load(Ordering::Relaxed));
}

/// Makes room for a new connection, or says there's none
fn admit(connections: &mut Vec<Connection>) -> bool {
    sweep(connections);
    if connections.iter().filter(|c| c.subscribed()).count() >= MAX_RECEIVERS + MAX_COMPANIONS {
        return false;
    }
    let pending: Vec<_> = connections.iter().filter(|c| !c.subscribed()).collect();
    // The oldest is the one least likely to be a receiver about to subscribe
    if pending.len() >= MAX_PENDING
        && let Some(oldest) = pending.iter().min_by_key(|c| c.since)
    {
        oldest.close();
    }
    connections.retain(|c| c.flags.alive.load(Ordering::Relaxed));
    true
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
        let connections: Arc<Mutex<Vec<Connection>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let accept = {
            let connections = Arc::clone(&connections);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let mut connections = connections.lock().unwrap();
                            if !admit(&mut connections) {
                                let _ = stream.shutdown(Shutdown::Both);
                            } else if let Some(c) = connect(stream) {
                                connections.push(c);
                            }
                        }
                        Err(_) => {
                            sweep(&mut connections.lock().unwrap());
                            thread::sleep(ACCEPT_POLL);
                        }
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
        sweep(&mut connections);
        connections.iter().filter(|c| c.watching()).count() as u32
    }

    pub fn send(&self, frame: Arc<Vec<u8>>) {
        let mut connections = self.connections.lock().unwrap();
        // So one over the cap doesn't get a picture before it's closed
        sweep(&mut connections);
        for c in connections.iter() {
            if c.watching() {
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
/// listens for the receiver subscribing
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
    let socket = stream.try_clone().ok()?;
    let flags = Arc::new(Flags {
        client: AtomicBool::new(false),
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
    Some(Connection {
        flags,
        queue,
        socket,
        since: Instant::now(),
    })
}

fn write_frames(mut stream: TcpStream, frames: Queue<Arc<Vec<u8>>>, flags: &Flags) {
    let hello = wire::metadata(&wire::sender_info(env!("CARGO_PKG_VERSION")));
    if stream.write_all(&hello).is_ok() {
        // Woken now and then to notice the connection being closed
        while flags.alive.load(Ordering::Relaxed) {
            match frames.recv_timeout(ACCEPT_POLL) {
                Ok(frame) if stream.write_all(&frame).is_ok() => {}
                Err(RecvTimeoutError::Timeout) => {}
                _ => break,
            }
        }
    }
    flags.alive.store(false, Ordering::Relaxed);
    let _ = stream.shutdown(Shutdown::Both);
}

fn read_requests(mut stream: TcpStream, flags: &Flags) {
    let mut header = [0u8; wire::HEADER_LEN];
    let mut data = Vec::new();
    while stream.read_exact(&mut header).is_ok() {
        let Some(h) = wire::parse_header(&header) else {
            break;
        };
        if h.data_len > wire::MAX_INCOMING {
            break;
        }
        // Grows with what actually arrives, not with what the header claims
        data.clear();
        let len = h.data_len as u64;
        if !matches!((&mut stream).take(len).read_to_end(&mut data), Ok(n) if n as u64 == len) {
            break;
        }
        if h.kind != wire::METADATA {
            continue;
        }
        match wire::subscription(&data) {
            Some(wire::Subscription::Video) => {
                flags.video.store(true, Ordering::Relaxed);
                flags.client.store(true, Ordering::Relaxed);
            }
            // Kept open so the receiver stays connected, but never sent
            // anything: there's no audio, and no metadata beyond the hello
            Some(wire::Subscription::Audio | wire::Subscription::Metadata) => {
                flags.client.store(true, Ordering::Relaxed);
            }
            None => {}
        }
    }
    flags.alive.store(false, Ordering::Relaxed);
    let _ = stream.shutdown(Shutdown::Both);
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
    shared: Arc<SharedDecoder>,
    decoder: Mutex<Option<Decoder>>,
    status: Mutex<OmtStatus>,
}

impl OmtOutput {
    pub fn new(cfg: &NetworkSourceSettings, shared: Arc<SharedDecoder>) -> Arc<Self> {
        let this = Arc::new(Self {
            running: Mutex::new(None),
            shared,
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
                match Decoder::start(&self.shared, r.limit, encoding(r)) {
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
