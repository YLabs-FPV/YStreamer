use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::devices::{self, Device};
use super::{FLAGS_REQUEST, FLAGS_RESPONSE, Frame, encode, next_seq};
use crate::input::goggles::{Goggles, LinkState};

const REGISTRAR: u8 = 0x3C;
const TOPIC_HOST: u8 = 0x28;

const CMD_GENERAL: u8 = 0x00;
const CMD_IDENTITY: u8 = 0x81;
const CMD_IDENTITY_ACK: u8 = 0x82;
const CMD_LINK: u8 = 0x88;
const CMD_TOPIC: u8 = 0x99;

const OP_REGISTER: u8 = 0x17;
const OP_REGISTERED: u8 = 0x18;
const OP_HEARTBEAT: u8 = 0x19;
const OP_HEARTBEAT_ACK: u8 = 0x1a;
const OP_VERSION: u8 = 0x1d;

const APP_VERSION: &str = "1.21.1";

const RETRY_INTERVAL: Duration = Duration::from_secs(1);
const WARN_AFTER_ATTEMPTS: u32 = 5;

const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(250);
// Linked devices repeat their identity about once a second
const DEVICE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(serde::Serialize, Clone, Default)]
pub struct SessionStatus {
    pub registered: bool,
    pub attempts: u32,
    pub heartbeats: u64,
    pub devices: Vec<Device>,
}

#[derive(Default)]
struct State {
    generation: u64,
    status: SessionStatus,
    last_attempt: Option<Instant>,
    last_heartbeat: Option<Instant>,
    warned_no_heartbeat: bool,
    warned_link_response: bool,
    devices: HashMap<u8, (Device, Instant)>,
}

/// The app side of the DUML conversation
pub struct Session {
    goggles: Goggles,
    state: Mutex<State>,
}

impl Session {
    pub fn start(goggles: Goggles) -> Arc<Self> {
        let session = Arc::new(Self {
            goggles,
            state: Mutex::new(State::default()),
        });
        let s = Arc::clone(&session);
        thread::spawn(move || {
            loop {
                s.tick();
                thread::sleep(TICK);
            }
        });
        session
    }

    pub fn status(&self) -> SessionStatus {
        let mut status = self.state.lock().unwrap().status.clone();
        status.devices = self.devices();
        status
    }

    pub fn devices(&self) -> Vec<Device> {
        if self.goggles.state() != LinkState::Live {
            return Vec::new();
        }
        let st = self.state.lock().unwrap();
        let mut out: Vec<Device> = st
            .devices
            .values()
            .filter(|(_, seen)| seen.elapsed() < DEVICE_TIMEOUT)
            .map(|(d, _)| d.clone())
            .collect();
        out.sort_by_key(|d| (d.kind, d.address));
        out
    }

    fn tick(&self) {
        {
            let mut st = self.state.lock().unwrap();

            let generation = self.goggles.generation();
            if generation != st.generation {
                *st = State {
                    generation,
                    ..State::default()
                };
            }

            if st.status.registered {
                let alive = st
                    .last_heartbeat
                    .is_some_and(|t| t.elapsed() < HEARTBEAT_TIMEOUT);
                if alive {
                    return;
                }
                if !st.warned_no_heartbeat {
                    eprintln!(
                        "[duml] No heartbeat from the goggles, re-registering to keep the link"
                    );
                    st.warned_no_heartbeat = true;
                }
                st.status.registered = false;
            }

            if st
                .last_attempt
                .is_some_and(|t| t.elapsed() < RETRY_INTERVAL)
            {
                return;
            }
        }

        // Not connected yet; try again next tick
        if self.register().is_err() {
            return;
        }

        let mut st = self.state.lock().unwrap();
        st.last_attempt = Some(Instant::now());
        st.status.attempts += 1;
        if st.status.attempts == WARN_AFTER_ATTEMPTS && !st.warned_no_heartbeat {
            eprintln!(
                "[duml] Goggles haven't accepted {} registrations yet",
                WARN_AFTER_ATTEMPTS
            );
        }
    }

    fn register(&self) -> std::io::Result<()> {
        self.request(TOPIC_HOST, CMD_TOPIC, &topic_payload())?;
        self.request(REGISTRAR, CMD_LINK, &register_payload())?;
        self.request(REGISTRAR, CMD_LINK, &version_payload())
    }

    fn request(&self, dst: u8, cmd_id: u8, payload: &[u8]) -> std::io::Result<()> {
        let frame = encode(dst, next_seq(), FLAGS_REQUEST, CMD_GENERAL, cmd_id, payload);
        self.goggles.send(&frame)
    }

    /// Called for every DUML frame the goggles send
    pub fn on_frame(&self, f: &Frame) {
        if f.cmd_set != CMD_GENERAL {
            if f.wants_reply() {
                self.reply(f, &[0x00]);
            }
            return;
        }

        if f.is_response() {
            if f.src == REGISTRAR && f.cmd_id == CMD_LINK {
                self.on_link_response(f.payload);
            }
            return;
        }
        if !f.wants_reply() {
            return;
        }

        match f.cmd_id {
            CMD_IDENTITY => {
                self.on_identity(f.src, f.payload);
                self.reply(f, &identity_reply(f.payload));
            }
            CMD_IDENTITY_ACK => {
                self.on_identity(f.src, f.payload);
                self.reply(f, &[0x00]);
            }
            CMD_LINK if f.payload.first() == Some(&OP_HEARTBEAT) => {
                self.on_heartbeat();
                self.reply(f, &[OP_HEARTBEAT_ACK, 0x00, 0x00, 0x00, 0x00]);
            }
            _ => self.reply(f, &[0x00]),
        }
    }

    fn on_link_response(&self, payload: &[u8]) {
        match payload.first() {
            Some(&OP_REGISTERED) => {
                let mut st = self.state.lock().unwrap();
                if !st.status.registered {
                    st.status.registered = true;
                    // Give the first heartbeat a full timeout to arrive
                    st.last_heartbeat = Some(Instant::now());
                    if !st.warned_no_heartbeat {
                        eprintln!(
                            "[duml] Registered with the goggles (attempt {})",
                            st.status.attempts
                        );
                    }
                }
            }
            Some(&OP_VERSION) => {}
            _ => {
                let mut st = self.state.lock().unwrap();
                if !st.warned_link_response {
                    st.warned_link_response = true;
                    eprintln!("[duml] Unexpected link response: {}", hex::encode(payload));
                }
            }
        }
    }

    fn on_heartbeat(&self) {
        let mut st = self.state.lock().unwrap();
        st.status.heartbeats += 1;
        st.last_heartbeat = Some(Instant::now());
        // Only a registered app gets heartbeats, so the accept got lost
        if !st.status.registered {
            st.status.registered = true;
            eprintln!("[duml] Registered with the goggles (heartbeat seen)");
        }
    }

    fn on_identity(&self, src: u8, payload: &[u8]) {
        let Some(device) = devices::from_identity(src, payload) else {
            return;
        };
        let mut st = self.state.lock().unwrap();
        let known = st
            .devices
            .get(&src)
            .is_some_and(|(d, seen)| *d == device && seen.elapsed() < DEVICE_TIMEOUT);
        if !known {
            eprintln!(
                "[duml] Linked: {} ({}) at {:#04x}",
                device.name, device.code, src
            );
        }
        st.devices.insert(src, (device, Instant::now()));
    }

    fn reply(&self, f: &Frame, payload: &[u8]) {
        let frame = encode(f.src, f.seq, FLAGS_RESPONSE, f.cmd_set, f.cmd_id, payload);
        let _ = self.goggles.send(&frame);
    }
}

fn topic_payload() -> Vec<u8> {
    let mut topic = vec![
        0x02, 0x02, 0x00, 0x00, 0xd5, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x13, 0x00,
    ];
    let name = b"camcap_common";
    topic.extend_from_slice(&(name.len() as u16).to_le_bytes());
    topic.extend_from_slice(name);
    topic.extend_from_slice(&[0; 4]);
    topic
}

fn register_payload() -> Vec<u8> {
    let mut register = vec![OP_REGISTER, 0x00, 0x00, 0x23, 0x00];
    register.extend_from_slice(&padded(b"APP", 8));
    register.push(0x02);
    register
}

fn version_payload() -> Vec<u8> {
    let mut version = vec![OP_VERSION, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01];
    // The length counts a NUL that isn't sent
    version.extend_from_slice(&(APP_VERSION.len() as u16 + 1).to_le_bytes());
    version.extend_from_slice(APP_VERSION.as_bytes());
    version
}

fn identity_reply(request: &[u8]) -> Vec<u8> {
    let mut out = vec![0x00];
    out.extend_from_slice(&padded(b"APP", 32));
    out.extend_from_slice(&[0x00, 0x02, 0, 0, 0, 0, 0, 0]);
    match request.get(40..48) {
        Some(field) => out.extend_from_slice(field),
        None => out.extend_from_slice(&[0x05, 0x1c, 0, 0, 0, 0, 0, 0]),
    }
    out.resize(64, 0);
    out
}

fn padded(s: &[u8], len: usize) -> Vec<u8> {
    let mut out = s.to_vec();
    out.resize(len, 0);
    out
}
