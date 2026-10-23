//! The RTMP input: OBS, a phone app or DJI Fly's live streaming pushes to
//! rtmp://<device>/live/<key>, and that becomes the video source. The server
//! runs in-process, so there's no second program to ship and no extra hop
//! between the app's H.264 and the outputs.

use std::collections::{HashMap, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rml_rtmp::handshake::{Handshake, HandshakeProcessResult, PeerType};
use rml_rtmp::sessions::{
    ServerSession, ServerSessionConfig, ServerSessionEvent, ServerSessionResult,
};
use serde::Serialize;
use subtle::ConstantTimeEq;

use super::flv::{AvcTags, TagError};
use super::{Fanout, InputState};
use crate::settings::{InputMode, RTMP_MIN_KEY_LEN, RtmpInputSettings};

const POLL: Duration = Duration::from_millis(100);
const RETRY: Duration = Duration::from_secs(2);
/// A connection that sends nothing for this long is dropped
const IDLE: Duration = Duration::from_secs(10);
/// No picture for this long and the app counts as connected but not sending
const STALL: Duration = Duration::from_secs(3);
/// From connecting to streaming with the right key. Without it, clients
/// sending a byte now and then would hold their slots for good
const LOGIN: Duration = Duration::from_secs(10);
/// Connections not streaming yet, handled at once; more are turned away, so
/// a port scan or a stuck app can't pile up threads. The app streaming
/// isn't counted, so these can't lock it out
const MAX_PENDING: usize = 4;
/// Wrong keys from one address before it's turned away for a while
const MAX_STRIKES: u32 = 3;
const FIRST_BAN: Duration = Duration::from_secs(60);
const MAX_BAN: Duration = Duration::from_secs(15 * 60);
/// Wrong keys are forgotten after this long without another
const STRIKE_MEMORY: Duration = Duration::from_secs(15 * 60);
/// Addresses remembered at once, so guessing from many can't use up memory
const MAX_TRACKED: usize = 64;
/// Wrong keys from all addresses together, refilled one at a time. Once
/// spent, an address that has sent a wrong key is turned away without its
/// key being checked, and one wrong key from any other gets it banned
const GLOBAL_BUDGET: u32 = 6;
const GLOBAL_REFILL: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Where the access units go: the fan-out, or a test
pub type Pictures = Arc<dyn Fn(&[u8]) + Send + Sync>;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RtmpInputState {
    /// Another input is selected
    #[default]
    Idle,
    /// Listening, no app streaming
    Waiting,
    /// An app is streaming, no picture yet
    Connected,
    Live,
    /// The port couldn't be opened
    Error,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct RtmpInputStatus {
    pub state: RtmpInputState,
    /// The port listened on
    pub port: Option<u16>,
    /// Address of the app streaming
    pub publisher: Option<String>,
    /// The stream has B-frames, which add delay
    pub reorders: bool,
    /// Why the port couldn't be opened, or why the last app was turned away
    pub error: Option<String>,
}

pub struct RtmpSource {
    state: Arc<InputState>,
    cfg: Mutex<RtmpInputSettings>,
    /// Bumped on every change of settings, so connections made under the
    /// old key or allowed addresses close
    generation: AtomicU64,
    /// One app streams at a time
    publishing: AtomicBool,
    pending: AtomicUsize,
    strikes: Mutex<Strikes>,
    login: Duration,
}

impl RtmpSource {
    pub fn start(
        state: Arc<InputState>,
        cfg: &RtmpInputSettings,
        fanout: Arc<Fanout>,
    ) -> Arc<Self> {
        Self::start_with(
            state,
            cfg,
            Arc::new(move |au| {
                fanout.video(InputMode::Rtmp, au);
            }),
        )
    }

    pub fn start_with(
        state: Arc<InputState>,
        cfg: &RtmpInputSettings,
        pictures: Pictures,
    ) -> Arc<Self> {
        Self::launch(state, cfg, pictures, LOGIN)
    }

    fn launch(
        state: Arc<InputState>,
        cfg: &RtmpInputSettings,
        pictures: Pictures,
        login: Duration,
    ) -> Arc<Self> {
        let this = Arc::new(Self {
            state,
            cfg: Mutex::new(cfg.clone()),
            generation: AtomicU64::new(0),
            publishing: AtomicBool::new(false),
            pending: AtomicUsize::new(0),
            strikes: Mutex::new(Strikes::new(Instant::now())),
            login,
        });
        let listener = Arc::clone(&this);
        thread::spawn(move || listener.run(pictures));
        this
    }

    /// Drops the app streaming now; it's let back in if it still qualifies
    pub fn apply(&self, cfg: &RtmpInputSettings) {
        *self.cfg.lock().unwrap() = cfg.clone();
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    pub fn status(&self) -> RtmpInputStatus {
        self.state.rtmp.lock().unwrap().clone()
    }

    fn update(&self, f: impl FnOnce(&mut RtmpInputStatus)) {
        f(&mut self.state.rtmp.lock().unwrap());
    }

    fn selected(&self) -> bool {
        self.state.mode() == InputMode::Rtmp
    }

    /// The port is only open while this input is selected
    fn run(self: Arc<Self>, pictures: Pictures) {
        // With the port it was opened for, which may be 0 in tests
        let mut listening: Option<(TcpListener, u16)> = None;
        let mut last_error = None;
        loop {
            if !self.selected() {
                if listening.take().is_some() {
                    eprintln!("[rtmp-in] stopped listening");
                }
                last_error = None;
                self.update(|s| *s = RtmpInputStatus::default());
                thread::sleep(POLL);
                continue;
            }

            let port = self.cfg.lock().unwrap().port;
            if listening.as_ref().is_some_and(|(_, p)| *p != port) {
                listening = None;
            }
            if listening.is_none() {
                match listen(port) {
                    Ok(l) => {
                        let actual = l.local_addr().map(|a| a.port()).unwrap_or(port);
                        eprintln!("[rtmp-in] listening on port {actual}");
                        last_error = None;
                        self.update(|s| {
                            *s = RtmpInputStatus {
                                state: RtmpInputState::Waiting,
                                port: Some(actual),
                                ..Default::default()
                            }
                        });
                        listening = Some((l, port));
                    }
                    Err(e) => {
                        let error = format!("Couldn't listen on port {port}: {e}");
                        if last_error.as_ref() != Some(&error) {
                            eprintln!("[rtmp-in] {error}");
                        }
                        self.update(|s| {
                            *s = RtmpInputStatus {
                                state: RtmpInputState::Error,
                                error: Some(error.clone()),
                                ..Default::default()
                            }
                        });
                        last_error = Some(error);
                        thread::sleep(RETRY);
                        continue;
                    }
                }
            }

            let Some((listener, _)) = &listening else {
                continue;
            };
            match listener.accept() {
                Ok((stream, peer)) => self.admit(stream, peer, &pictures),
                Err(e) if e.kind() == ErrorKind::WouldBlock => thread::sleep(POLL),
                Err(e) => {
                    eprintln!("[rtmp-in] accept: {e}");
                    thread::sleep(POLL);
                }
            }
        }
    }

    fn admit(self: &Arc<Self>, stream: TcpStream, peer: SocketAddr, pictures: &Pictures) {
        // Generation first: settings changed in between close it right away
        let generation = self.generation.load(Ordering::SeqCst);
        let cfg = self.cfg.lock().unwrap().clone();
        if !cfg.allows(peer.ip()) {
            self.refused(format!(
                "Turned away {}: not one of the allowed addresses",
                peer.ip()
            ));
            return;
        }
        // Turned away quietly: logging each try would let a guesser fill the log
        if let Some(left) = self
            .strikes
            .lock()
            .unwrap()
            .banned(peer.ip(), Instant::now())
        {
            self.update(|s| {
                s.error = Some(format!(
                    "Turned away {}: too many wrong keys, let in again in {} s",
                    peer.ip(),
                    left.as_secs().max(1)
                ))
            });
            return;
        }
        if self.pending.fetch_add(1, Ordering::SeqCst) >= MAX_PENDING {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        let this = Arc::clone(self);
        let pictures = Arc::clone(pictures);
        thread::spawn(move || {
            let mut conn = Connection {
                source: &this,
                stream,
                peer: peer.ip(),
                key: cfg.key,
                generation,
                pictures,
                tags: AvcTags::new(),
                accepted: Instant::now(),
                pending: true,
                publishing: false,
                last_picture: None,
                bad_tags: 0,
            };
            if let Err(e) = conn.serve() {
                eprintln!("[rtmp-in] {}: {e}", conn.peer);
            }
        });
    }

    fn refused(&self, why: String) {
        eprintln!("[rtmp-in] {why}");
        self.update(|s| s.error = Some(why));
    }

    fn current(&self, generation: u64) -> bool {
        self.selected() && self.generation.load(Ordering::SeqCst) == generation
    }
}

fn listen(port: u16) -> std::io::Result<TcpListener> {
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

/// One app's connection, from the handshake to the end of its stream
struct Connection<'a> {
    source: &'a RtmpSource,
    stream: TcpStream,
    peer: IpAddr,
    key: String,
    generation: u64,
    pictures: Pictures,
    tags: AvcTags,
    accepted: Instant,
    /// Holds one of the source's slots for connections not streaming yet
    pending: bool,
    /// Holds the source's one publishing slot
    publishing: bool,
    last_picture: Option<Instant>,
    bad_tags: u32,
}

impl Connection<'_> {
    fn serve(&mut self) -> Result<(), String> {
        // Accepted sockets may inherit the listener's non-blocking mode
        self.stream
            .set_nonblocking(false)
            .and_then(|_| self.stream.set_read_timeout(Some(POLL)))
            .and_then(|_| self.stream.set_write_timeout(Some(WRITE_TIMEOUT)))
            .map_err(|e| e.to_string())?;
        let _ = self.stream.set_nodelay(true);
        let mut buf = vec![0u8; 64 * 1024];

        let mut handshake = Handshake::new(PeerType::Server);
        let rest = loop {
            let Some(n) = self.read(&mut buf)? else {
                return Ok(());
            };
            match handshake
                .process_bytes(&buf[..n])
                .map_err(|e| format!("handshake: {e}"))?
            {
                HandshakeProcessResult::InProgress { response_bytes } => {
                    self.send(&response_bytes)?
                }
                HandshakeProcessResult::Completed {
                    response_bytes,
                    remaining_bytes,
                } => {
                    self.send(&response_bytes)?;
                    break remaining_bytes;
                }
            }
        };

        let (mut session, results) =
            ServerSession::new(ServerSessionConfig::new()).map_err(|e| e.to_string())?;
        if !self.handle(&mut session, results)? {
            return Ok(());
        }
        let results = session.handle_input(&rest).map_err(|e| e.to_string())?;
        if !self.handle(&mut session, results)? {
            return Ok(());
        }
        loop {
            let Some(n) = self.read(&mut buf)? else {
                return Ok(());
            };
            let results = session.handle_input(&buf[..n]).map_err(|e| e.to_string())?;
            if !self.handle(&mut session, results)? {
                return Ok(());
            }
        }
    }

    /// Some bytes, or None once the connection should end quietly
    fn read(&mut self, buf: &mut [u8]) -> Result<Option<usize>, String> {
        let since = Instant::now();
        loop {
            if !self.source.current(self.generation) {
                return Ok(None);
            }
            if !self.publishing && self.accepted.elapsed() > self.source.login {
                return Err(format!(
                    "didn't start streaming within {} s",
                    self.source.login.as_secs_f32()
                ));
            }
            match self.stream.read(buf) {
                Ok(0) => return Ok(None),
                Ok(n) => {
                    self.check_stall();
                    return Ok(Some(n));
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    if since.elapsed() > IDLE {
                        return Err(format!("sent nothing for {} s", IDLE.as_secs()));
                    }
                    self.check_stall();
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.stream.write_all(bytes).map_err(|e| e.to_string())
    }

    /// Sends what the session wants sent and acts on what it raised.
    /// False once the connection should end
    fn handle(
        &mut self,
        session: &mut ServerSession,
        results: Vec<ServerSessionResult>,
    ) -> Result<bool, String> {
        let mut queue = VecDeque::from(results);
        while let Some(result) = queue.pop_front() {
            match result {
                ServerSessionResult::OutboundResponse(packet) => self.send(&packet.bytes)?,
                ServerSessionResult::RaisedEvent(event) => match self.event(session, event)? {
                    Some(more) => queue.extend(more),
                    None => {
                        // Whatever answer is queued still goes out first
                        for r in queue {
                            if let ServerSessionResult::OutboundResponse(p) = r {
                                let _ = self.send(&p.bytes);
                            }
                        }
                        return Ok(false);
                    }
                },
                ServerSessionResult::UnhandleableMessageReceived(_) => {}
            }
        }
        Ok(true)
    }

    /// More results to handle, or None to end the connection
    fn event(
        &mut self,
        session: &mut ServerSession,
        event: ServerSessionEvent,
    ) -> Result<Option<Vec<ServerSessionResult>>, String> {
        let err = |e: rml_rtmp::sessions::ServerSessionError| e.to_string();
        match event {
            // Any application name: the key is what keeps others out
            ServerSessionEvent::ConnectionRequested { request_id, .. } => {
                session.accept_request(request_id).map(Some).map_err(err)
            }
            ServerSessionEvent::PublishStreamRequested {
                request_id,
                stream_key,
                ..
            } => {
                // Some apps add query parameters to the key
                let key = stream_key.split('?').next().unwrap_or_default();
                let right: bool = key.as_bytes().ct_eq(self.key.as_bytes()).into();
                let refusal = if self.key.len() < RTMP_MIN_KEY_LEN {
                    Some("no usable stream key is set on the device")
                } else if !right {
                    let ban = self
                        .source
                        .strikes
                        .lock()
                        .unwrap()
                        .strike(self.peer, Instant::now());
                    if let Some(ban) = ban {
                        eprintln!(
                            "[rtmp-in] {}: too many wrong keys, turned away for {} s",
                            self.peer,
                            ban.as_secs()
                        );
                    }
                    Some("wrong stream key")
                } else if self.source.publishing.swap(true, Ordering::SeqCst) {
                    Some("another app is already streaming")
                } else {
                    None
                };
                if let Some(why) = refusal {
                    self.source
                        .refused(format!("Turned away {}: {why}", self.peer));
                    let reply = session
                        .reject_request(request_id, "NetStream.Publish.BadName", why)
                        .map_err(err)?;
                    self.handle(session, reply)?;
                    return Ok(None);
                }
                self.publishing = true;
                self.pending = false;
                self.source.pending.fetch_sub(1, Ordering::SeqCst);
                self.source.strikes.lock().unwrap().forget(self.peer);
                eprintln!("[rtmp-in] {} is streaming", self.peer);
                let peer = self.peer.to_string();
                self.source.update(|s| {
                    s.state = RtmpInputState::Connected;
                    s.publisher = Some(peer);
                    s.reorders = false;
                    s.error = None;
                });
                session.accept_request(request_id).map(Some).map_err(err)
            }
            ServerSessionEvent::VideoDataReceived { data, .. } if self.publishing => {
                Ok(self.video(&data).then(Vec::new))
            }
            ServerSessionEvent::PublishStreamFinished { .. } => Ok(None),
            // Audio is dropped: no output carries the app's sound
            _ => Ok(Some(Vec::new())),
        }
    }

    /// False when the stream can't be used
    fn video(&mut self, tag: &[u8]) -> bool {
        match self.tags.push(tag) {
            Ok(Some(au)) => {
                (self.pictures)(&au);
                let first = self.last_picture.is_none();
                self.last_picture = Some(Instant::now());
                let reorders = self.tags.reorders();
                self.source.update(|s| {
                    if s.state == RtmpInputState::Connected || s.reorders != reorders {
                        s.state = RtmpInputState::Live;
                        s.reorders = reorders;
                    }
                });
                if first && reorders {
                    eprintln!("[rtmp-in] the stream has B-frames, which add delay");
                }
                true
            }
            Ok(None) => true,
            Err(e @ TagError::Unsupported(_)) => {
                self.source
                    .refused(format!("Turned away {}: {e}", self.peer));
                false
            }
            // A damaged tag costs one picture; a stream of them, the connection
            Err(e) => {
                self.bad_tags += 1;
                if self.bad_tags == 1 || self.bad_tags.is_multiple_of(100) {
                    eprintln!("[rtmp-in] {e} ({} so far)", self.bad_tags);
                }
                true
            }
        }
    }

    fn check_stall(&self) {
        if self.last_picture.is_some_and(|t| t.elapsed() > STALL) {
            self.source.update(|s| {
                if s.state == RtmpInputState::Live {
                    s.state = RtmpInputState::Connected;
                }
            });
        }
    }
}

impl Drop for Connection<'_> {
    fn drop(&mut self) {
        if self.pending {
            self.source.pending.fetch_sub(1, Ordering::SeqCst);
        }
        if !self.publishing {
            return;
        }
        eprintln!("[rtmp-in] {} stopped streaming", self.peer);
        self.source.publishing.store(false, Ordering::SeqCst);
        self.source.update(|s| {
            if matches!(s.state, RtmpInputState::Connected | RtmpInputState::Live) {
                s.state = RtmpInputState::Waiting;
                s.publisher = None;
                s.reorders = false;
            }
        });
    }
}

/// Wrong keys per address, for turning away one that keeps guessing, and
/// across all of them, so moving to another address buys little
struct Strikes {
    by_ip: HashMap<IpAddr, Record>,
    /// Wrong keys anyone may still send before the stricter rules apply
    budget: u32,
    refilled: Instant,
}

#[derive(Clone, Copy)]
struct Record {
    wrong: u32,
    last: Instant,
    banned_until: Option<Instant>,
}

impl Record {
    fn banned_at(&self, now: Instant) -> bool {
        self.banned_until.is_some_and(|until| until > now)
    }
}

impl Strikes {
    fn new(now: Instant) -> Self {
        Self {
            by_ip: HashMap::new(),
            budget: GLOBAL_BUDGET,
            refilled: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let earned =
            (now.duration_since(self.refilled).as_millis() / GLOBAL_REFILL.as_millis()) as u32;
        if earned == 0 {
            return;
        }
        self.budget = (self.budget + earned).min(GLOBAL_BUDGET);
        self.refilled = if self.budget == GLOBAL_BUDGET {
            now
        } else {
            self.refilled + GLOBAL_REFILL * earned
        };
    }

    /// How long `ip` is still turned away for, if it is. With the shared
    /// budget spent, that's every address that has sent a wrong key; one
    /// that hasn't still gets its key checked, so the real app gets in
    fn banned(&mut self, ip: IpAddr, now: Instant) -> Option<Duration> {
        self.refill(now);
        let next_refill = self.refilled + GLOBAL_REFILL;
        match self.by_ip.get(&ip.to_canonical()) {
            Some(r) if r.banned_at(now) => r.banned_until.map(|until| until - now),
            Some(_) if self.budget == 0 => Some(next_refill - now),
            Some(_) => None,
            // No room left to note a wrong key from it: turned away rather
            // than forgetting someone's ban
            None => self.full_of_bans(now),
        }
    }

    /// When every remembered address is banned, how long until one isn't
    fn full_of_bans(&self, now: Instant) -> Option<Duration> {
        if self.by_ip.len() < MAX_TRACKED || self.by_ip.values().any(|r| !r.banned_at(now)) {
            return None;
        }
        let soonest = self.by_ip.values().filter_map(|r| r.banned_until).min()?;
        Some(soonest - now)
    }

    /// Counts a wrong key; the length of the ban it earns, if it earns one.
    /// Each wrong key past the limit doubles the ban, and with the shared
    /// budget spent the first one already earns it
    fn strike(&mut self, ip: IpAddr, now: Instant) -> Option<Duration> {
        let ip = ip.to_canonical();
        self.refill(now);
        let spent = self.budget == 0;
        self.budget = self.budget.saturating_sub(1);
        self.by_ip
            .retain(|_, r| now.duration_since(r.last) < STRIKE_MEMORY || r.banned_at(now));
        if !self.by_ip.contains_key(&ip) && self.by_ip.len() >= MAX_TRACKED {
            // Only an address that isn't banned makes room
            let oldest = self
                .by_ip
                .iter()
                .filter(|(_, r)| !r.banned_at(now))
                .min_by_key(|(_, r)| r.last)
                .map(|(ip, _)| *ip);
            match oldest {
                Some(oldest) => {
                    self.by_ip.remove(&oldest);
                }
                None => return self.full_of_bans(now),
            }
        }
        let r = self.by_ip.entry(ip).or_insert(Record {
            wrong: 0,
            last: now,
            banned_until: None,
        });
        r.wrong += 1;
        r.last = now;
        if r.wrong < MAX_STRIKES && !spent {
            return None;
        }
        let ban = FIRST_BAN
            .saturating_mul(1 << r.wrong.saturating_sub(MAX_STRIKES).min(10))
            .min(MAX_BAN);
        r.banned_until = Some(now + ban);
        Some(ban)
    }

    /// The right key: what came before is forgiven
    fn forget(&mut self, ip: IpAddr) {
        self.by_ip.remove(&ip.to_canonical());
    }
}

#[cfg(test)]
#[path = "rtmp.test.rs"]
mod tests;
