use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nusb::descriptors::TransferType;
use nusb::transfer::{
    Bulk, ControlIn, ControlOut, ControlType, Direction, In, Out, Recipient, TransferError,
};
use nusb::{Device, DeviceId, DeviceInfo, MaybeFuture};
use serde::Serialize;

use super::InputState;
use super::duml::Frame;
use super::duml::monitor::Monitor;
use super::goggles::Goggles;
use crate::settings::InputMode;

/// What DJI goggles announce when a phone is plugged into them, as recorded
/// from our own Goggles 3 on an Android phone's USB log. DJI Fly opens any
/// accessory that matches the first two
const IDENTITY: [&str; 6] = [
    "DJI",
    "com.dji.logiclink",
    "DJI glass",
    "v0.0.0.0",
    "www.dji.com",
    "000000000000000",
];

// Android Open Accessory requests
const AOA_GET_PROTOCOL: u8 = 51;
const AOA_SEND_STRING: u8 = 52;
const AOA_START: u8 = 53;
// What a phone becomes once it's an accessory, with or without adb
const ACCESSORY_VENDOR: u16 = 0x18D1;
const ACCESSORY_PRODUCTS: [u16; 2] = [0x2D00, 0x2D01];

// The V1/V2 goggles, and iPhones, which can't be accessories this way
const DJI_VENDOR: u16 = 0x2CA3;
const APPLE_VENDOR: u16 = 0x05AC;
const HUB: u8 = 0x09;
// Audio, HID, mass storage, video, wireless (USB tethering, WiFi dongles).
// A phone offers MTP or its vendor's own interfaces, so these are never one
const NOT_A_PHONE: [u8; 5] = [0x01, 0x03, 0x08, 0x0E, 0xE0];

const LINK_HEADER: usize = 8;
// The LogicLink length is 16 bits, so nothing waiting can be longer
const MAX_PENDING: usize = LINK_HEADER + u16::MAX as usize;

const CONTROL_TIMEOUT: Duration = Duration::from_millis(200);
const SWITCH_WAIT: Duration = Duration::from_secs(10);
const READ_LEN: usize = 16 * 1024;
const READ_TIMEOUT: Duration = Duration::from_millis(250);
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
const WRITE_BATCH: usize = 256 * 1024;
// Around a second of the goggles' stream. A phone further behind than that
// loses the oldest data, so the video path never waits for it
const QUEUE_BYTES: usize = 2 * 1024 * 1024;
const WRITE_FAILURES: u32 = 15;
// DJI Fly talks to the goggles several times a second while it's open
const DJI_FLY_GONE: Duration = Duration::from_secs(5);
// A phone switched while locked, or before DJI Fly was installed, never
// opens the accessory. A USB reset makes Android offer it again, but only a
// few times, so a phone left plugged in doesn't keep popping up a dialog
const SILENT_PHONE: Duration = Duration::from_secs(25);
const RESETS: u32 = 3;
const RETRY: Duration = Duration::from_secs(1);

#[derive(Serialize, Clone, Copy, Default, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum PassthroughState {
    /// Turned off, or another source is selected
    #[default]
    Off,
    NoPhone,
    /// The phone is in accessory mode; DJI Fly hasn't opened it yet
    Phone,
    DjiFly,
}

#[derive(Serialize, Clone, Default, PartialEq, Debug)]
pub struct PassthroughStatus {
    pub state: PassthroughState,
    pub error: Option<String>,
    /// Bytes since the service started, for developer mode
    pub to_phone: u64,
    pub from_phone: u64,
    pub dropped: u64,
}

/// The goggles' stream on its way to the phone. Bounded by bytes and
/// dropping the oldest, so a slow phone loses data instead of holding up
/// the goggles' read loop
struct Queue {
    pending: Mutex<Pending>,
    ready: Condvar,
    cap: usize,
}

#[derive(Default)]
struct Pending {
    chunks: VecDeque<Vec<u8>>,
    bytes: usize,
}

impl Queue {
    fn new(cap: usize) -> Self {
        Self {
            pending: Mutex::new(Pending::default()),
            ready: Condvar::new(),
            cap,
        }
    }

    /// Returns how many bytes were dropped to make room
    fn push(&self, data: &[u8]) -> usize {
        let mut q = self.pending.lock().unwrap();
        let mut dropped = 0;
        while q.bytes + data.len() > self.cap
            && let Some(old) = q.chunks.pop_front()
        {
            q.bytes -= old.len();
            dropped += old.len();
        }
        q.bytes += data.len();
        q.chunks.push_back(data.to_vec());
        self.ready.notify_one();
        dropped
    }

    /// Everything waiting, up to `max` bytes in one piece so the phone gets
    /// a few large writes instead of many small ones
    fn take(&self, max: usize, wait: Duration) -> Option<Vec<u8>> {
        let mut q = self.pending.lock().unwrap();
        if q.chunks.is_empty() {
            q = self
                .ready
                .wait_timeout_while(q, wait, |q| q.chunks.is_empty())
                .unwrap()
                .0;
        }
        let mut out = q.chunks.pop_front()?;
        while let Some(next) = q.chunks.front()
            && out.len() + next.len() <= max
        {
            out.extend_from_slice(next);
            q.chunks.pop_front();
        }
        q.bytes -= out.len();
        Some(out)
    }

    fn clear(&self) {
        *self.pending.lock().unwrap() = Pending::default();
    }
}

/// Splits what DJI Fly writes into whole LogicLink frames, read the same way
/// as the goggles' side, so our own writes to the goggles can only land
/// between two of its frames, never inside one
#[derive(Default)]
struct Framer {
    buf: Vec<u8>,
}

impl Framer {
    fn push(&mut self, data: &[u8], mut on_frame: impl FnMut(&[u8])) {
        self.buf.extend_from_slice(data);
        let mut i = 0;
        while i + LINK_HEADER <= self.buf.len() {
            if self.buf[i] != 0x55 || self.buf[i + 1] != 0xCC {
                i += 1;
                continue;
            }
            let len = u16::from_le_bytes([self.buf[i + 4], self.buf[i + 5]]) as usize;
            let total = LINK_HEADER + len;
            if i + total > self.buf.len() {
                break;
            }
            on_frame(&self.buf[i..i + total]);
            i += total;
        }
        self.buf.drain(..i);
        if self.buf.len() > MAX_PENDING {
            self.buf.clear();
        }
    }
}

/// What the passthrough remembers between looks at the bus
#[derive(Default)]
struct Bus {
    /// Devices that aren't Android phones, so each is asked only once
    not_phones: HashSet<DeviceId>,
    /// A phone that wouldn't switch, and why. Not asked again until replugged
    failed: Option<(DeviceId, String)>,
    /// USB resets of a phone DJI Fly hasn't opened
    resets: u32,
}

impl Bus {
    fn forget_gone(&mut self, devices: &[DeviceInfo]) {
        let present = |id: &DeviceId| devices.iter().any(|d| d.id() == *id);
        self.not_phones.retain(present);
        if self.failed.as_ref().is_some_and(|(id, _)| !present(id)) {
            self.failed = None;
        }
    }
}

/// Passes the goggles' link through to DJI Fly on an Android phone in a
/// USB-A port, and DJI Fly's commands back to the goggles. We are the USB
/// host to the phone and switch it into accessory mode the way the goggles
/// would, so DJI Fly works next to everything else YStreamer does
pub struct Passthrough {
    enabled: AtomicBool,
    input: Arc<InputState>,
    goggles: Goggles,
    monitor: Arc<Monitor>,
    queue: Queue,
    /// A phone is claimed and takes the goggles' stream
    attached: AtomicBool,
    status: Mutex<(PassthroughState, Option<String>)>,
    to_phone: AtomicU64,
    from_phone: AtomicU64,
    dropped: AtomicU64,
}

impl Passthrough {
    pub fn start(
        enabled: bool,
        input: Arc<InputState>,
        goggles: Goggles,
        monitor: Arc<Monitor>,
    ) -> Arc<Self> {
        let this = Arc::new(Self {
            enabled: AtomicBool::new(enabled),
            input,
            goggles,
            monitor,
            queue: Queue::new(QUEUE_BYTES),
            attached: AtomicBool::new(false),
            status: Mutex::new((PassthroughState::Off, None)),
            to_phone: AtomicU64::new(0),
            from_phone: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        });
        let p = Arc::clone(&this);
        thread::spawn(move || p.run());
        this
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn status(&self) -> PassthroughStatus {
        let (state, error) = self.status.lock().unwrap().clone();
        PassthroughStatus {
            state,
            error,
            to_phone: self.to_phone.load(Ordering::Relaxed),
            from_phone: self.from_phone.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
        }
    }

    /// Raw bytes as the goggles sent them. Never blocks: with no phone it's
    /// a single load, and a phone that can't keep up only loses its own data
    pub fn forward(&self, data: &[u8]) {
        if !self.attached.load(Ordering::Relaxed) {
            return;
        }
        let dropped = self.queue.push(data);
        if dropped > 0 {
            self.dropped.fetch_add(dropped as u64, Ordering::Relaxed);
        }
    }

    /// Only the USB-C link has anything to pass through
    fn active(&self) -> bool {
        self.enabled.load(Ordering::Relaxed) && self.input.mode() == InputMode::DjiFpv
    }

    fn set(&self, state: PassthroughState, error: Option<String>) {
        *self.status.lock().unwrap() = (state, error);
    }

    fn state(&self) -> PassthroughState {
        self.status.lock().unwrap().0
    }

    fn run(self: Arc<Self>) {
        let mut bus = Bus::default();
        let mut last_error = None;
        loop {
            if self.active() {
                match self.look(&mut bus) {
                    Ok(()) => last_error = None,
                    Err(e) => {
                        // Once, not every second while it lasts
                        if last_error.as_ref() != Some(&e) {
                            eprintln!("[passthrough] {e}");
                        }
                        self.set(PassthroughState::NoPhone, Some(e.clone()));
                        last_error = Some(e);
                    }
                }
            } else {
                bus = Bus::default();
                last_error = None;
                self.set(PassthroughState::Off, None);
            }
            thread::sleep(RETRY);
        }
    }

    /// One look at the bus: relay to a phone that's already an accessory, or
    /// switch one that isn't yet
    fn look(self: &Arc<Self>, bus: &mut Bus) -> Result<(), String> {
        let devices = list_devices()?;
        bus.forget_gone(&devices);

        let accessory = match devices.iter().find(|d| is_accessory(d)) {
            Some(d) => d.clone(),
            None => {
                let Some(phone) = find_phone(&devices, bus) else {
                    let error = bus.failed.as_ref().map(|(_, e)| e.clone());
                    self.set(PassthroughState::NoPhone, error);
                    return Ok(());
                };
                println!("[passthrough] Android phone found, switching it to accessory mode");
                match switch(&phone).and_then(|()| wait_for_accessory()) {
                    Ok(d) => d,
                    Err(e) => {
                        bus.failed = Some((phone.id(), e.clone()));
                        return Err(e);
                    }
                }
            }
        };
        self.relay(&accessory, bus)
    }

    /// Runs until the phone goes away or passthrough is turned off
    fn relay(self: &Arc<Self>, info: &DeviceInfo, bus: &mut Bus) -> Result<(), String> {
        let device = info
            .open()
            .wait()
            .map_err(|e| format!("Couldn't open the phone: {e}"))?;
        let (number, ep_in, ep_out) = accessory_endpoints(&device)
            .ok_or("The phone's accessory interface has no bulk endpoints")?;
        // Not reset: DJI Fly may already be holding it
        let interface = device
            .detach_and_claim_interface(number)
            .wait()
            .map_err(|e| format!("Couldn't claim the phone's accessory interface: {e}"))?;
        let mut inp = interface
            .endpoint::<Bulk, In>(ep_in)
            .map_err(|e| format!("Phone bulk in: {e}"))?;
        let out = interface
            .endpoint::<Bulk, Out>(ep_out)
            .map_err(|e| format!("Phone bulk out: {e}"))?;

        println!("[passthrough] phone connected, waiting for DJI Fly");
        bus.failed = None;
        self.queue.clear();
        self.attached.store(true, Ordering::Relaxed);
        self.set(PassthroughState::Phone, None);

        // Set by whichever side ends first, the writer when the phone stops
        // taking data, this loop for everything else
        let ended = Arc::new(AtomicBool::new(false));
        let writer = {
            let this = Arc::clone(self);
            let ended = Arc::clone(&ended);
            thread::spawn(move || this.write_to_phone(out, &ended))
        };

        let claimed = Instant::now();
        let mut last_rx: Option<Instant> = None;
        let mut framer = Framer::default();
        let mut reset = false;
        inp.submit(inp.allocate(READ_LEN));
        let end = loop {
            if !self.active() {
                break Ok(());
            }
            if ended.load(Ordering::Relaxed) {
                break Err("Lost the phone: it stopped taking data".into());
            }
            let Some(done) = inp.wait_next_complete(READ_TIMEOUT) else {
                if last_rx.is_some_and(|at| at.elapsed() > DJI_FLY_GONE)
                    && self.state() == PassthroughState::DjiFly
                {
                    println!("[passthrough] DJI Fly went quiet");
                    self.set(PassthroughState::Phone, None);
                }
                if last_rx.is_none() && claimed.elapsed() > SILENT_PHONE && bus.resets < RESETS {
                    bus.resets += 1;
                    println!(
                        "[passthrough] DJI Fly hasn't opened the phone's accessory, resetting it ({}/{RESETS})",
                        bus.resets
                    );
                    reset = true;
                    break Ok(());
                }
                continue;
            };
            match done.status {
                Ok(()) | Err(TransferError::Cancelled) if done.actual_len > 0 => {
                    let data = &done.buffer[..done.actual_len];
                    self.from_phone
                        .fetch_add(data.len() as u64, Ordering::Relaxed);
                    framer.push(data, |frame| self.to_goggles(frame));
                    last_rx = Some(Instant::now());
                    bus.resets = 0;
                    if self.state() != PassthroughState::DjiFly {
                        println!("[passthrough] DJI Fly attached");
                        self.set(PassthroughState::DjiFly, None);
                    }
                }
                Ok(()) | Err(TransferError::Cancelled) => {}
                Err(TransferError::Stall) => {
                    let _ = inp.clear_halt().wait();
                }
                Err(e) => break Err(format!("Lost the phone: {e}")),
            }
            inp.submit(inp.allocate(READ_LEN));
        };

        self.attached.store(false, Ordering::Relaxed);
        ended.store(true, Ordering::Relaxed);
        let _ = writer.join();
        self.queue.clear();
        drop(inp);
        drop(interface);
        if reset {
            let _ = device.reset().wait();
        }
        println!("[passthrough] phone released");
        end
    }

    fn to_goggles(&self, frame: &[u8]) {
        if let Some(f) = Frame::decode(&frame[LINK_HEADER..]) {
            self.monitor.record(&f);
        }
        // Nothing to do with it while the goggles are unplugged
        let _ = self.goggles.send_from_phone(frame);
    }

    /// Drains the queue into the phone until it's gone or `stop` is set.
    /// A phone that stalls now and then (DJI Fly buffering) is fine; only a
    /// long run of failures gives it up
    fn write_to_phone(&self, mut out: nusb::Endpoint<Bulk, Out>, stop: &AtomicBool) {
        let mut failures = 0;
        while !stop.load(Ordering::Relaxed) {
            let Some(batch) = self.queue.take(WRITE_BATCH, READ_TIMEOUT) else {
                continue;
            };
            let done = out.transfer_blocking(batch.into(), WRITE_TIMEOUT);
            match done.status {
                Ok(()) => failures = 0,
                // DJI Fly isn't reading yet
                Err(TransferError::Cancelled) => {}
                Err(TransferError::Disconnected) => failures = WRITE_FAILURES,
                Err(TransferError::Stall) => {
                    let _ = out.clear_halt().wait();
                    failures += 1;
                }
                Err(_) => failures += 1,
            }
            self.to_phone
                .fetch_add(done.actual_len as u64, Ordering::Relaxed);
            if failures >= WRITE_FAILURES {
                stop.store(true, Ordering::Relaxed);
            }
        }
    }
}

fn list_devices() -> Result<Vec<DeviceInfo>, String> {
    Ok(nusb::list_devices()
        .wait()
        .map_err(|e| format!("Couldn't list USB devices: {e}"))?
        .collect())
}

fn is_accessory(d: &DeviceInfo) -> bool {
    d.vendor_id() == ACCESSORY_VENDOR && ACCESSORY_PRODUCTS.contains(&d.product_id())
}

/// Whether a device is worth asking if it can be an accessory. Asking is a
/// vendor request most devices just refuse, but some take it badly
fn might_be_phone(vendor: u16, class: u8, interfaces: &[u8]) -> bool {
    vendor != DJI_VENDOR
        && vendor != APPLE_VENDOR
        && class != HUB
        && !interfaces.iter().any(|c| NOT_A_PHONE.contains(c))
}

fn find_phone(devices: &[DeviceInfo], bus: &mut Bus) -> Option<DeviceInfo> {
    let failed = bus.failed.as_ref().map(|(id, _)| *id);
    for d in devices {
        if bus.not_phones.contains(&d.id()) || failed == Some(d.id()) {
            continue;
        }
        let classes: Vec<u8> = d.interfaces().map(|i| i.class()).collect();
        if !might_be_phone(d.vendor_id(), d.class(), &classes)
            || has_network_interface(d)
            || aoa_protocol(d).is_none()
        {
            bus.not_phones.insert(d.id());
            continue;
        }
        return Some(d.clone());
    }
    None
}

/// Some USB network adapters offer vendor interfaces only
#[cfg(target_os = "linux")]
fn has_network_interface(d: &DeviceInfo) -> bool {
    std::fs::read_dir(d.sysfs_path())
        .map(|dir| dir.flatten().any(|e| e.path().join("net").is_dir()))
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn has_network_interface(_: &DeviceInfo) -> bool {
    false
}

/// The accessory protocol version an Android phone speaks, 1 or 2
fn aoa_protocol(d: &DeviceInfo) -> Option<u16> {
    let device = d.open().wait().ok()?;
    protocol_of(&device)
}

fn protocol_of(device: &Device) -> Option<u16> {
    let reply = device
        .control_in(
            ControlIn {
                control_type: ControlType::Vendor,
                recipient: Recipient::Device,
                request: AOA_GET_PROTOCOL,
                value: 0,
                index: 0,
                length: 2,
            },
            CONTROL_TIMEOUT,
        )
        .wait()
        .ok()?;
    let version = u16::from_le_bytes(reply.get(..2)?.try_into().ok()?);
    matches!(version, 1 | 2).then_some(version)
}

/// Hand the phone the goggles' identity and ask it to become an accessory
fn switch(info: &DeviceInfo) -> Result<(), String> {
    let device = info
        .open()
        .wait()
        .map_err(|e| format!("Couldn't open the phone: {e}"))?;
    protocol_of(&device).ok_or("The phone stopped answering")?;
    for (index, s) in IDENTITY.iter().enumerate() {
        device
            .control_out(
                ControlOut {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Device,
                    request: AOA_SEND_STRING,
                    value: 0,
                    index: index as u16,
                    data: &nul_terminated(s),
                },
                CONTROL_TIMEOUT,
            )
            .wait()
            .map_err(|e| format!("The phone refused the accessory identity: {e}"))?;
    }
    device
        .control_out(
            ControlOut {
                control_type: ControlType::Vendor,
                recipient: Recipient::Device,
                request: AOA_START,
                value: 0,
                index: 0,
                data: &[],
            },
            CONTROL_TIMEOUT,
        )
        .wait()
        .map_err(|e| format!("The phone refused to switch to accessory mode: {e}"))
}

fn nul_terminated(s: &str) -> Vec<u8> {
    let mut out = s.as_bytes().to_vec();
    out.push(0);
    out
}

/// The phone drops off the bus and comes back as an accessory
fn wait_for_accessory() -> Result<DeviceInfo, String> {
    let deadline = Instant::now() + SWITCH_WAIT;
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(200));
        if let Some(d) = list_devices()?.into_iter().find(is_accessory) {
            return Ok(d);
        }
    }
    Err("The phone didn't come back as an accessory. Unplug it and plug it in again".into())
}

/// The accessory's interface, with its bulk in and out endpoints
fn accessory_endpoints(device: &Device) -> Option<(u8, u8, u8)> {
    let config = device.active_configuration().ok()?;
    config.interface_alt_settings().find_map(|alt| {
        let bulk = |direction| {
            alt.endpoints()
                .find(|e| e.transfer_type() == TransferType::Bulk && e.direction() == direction)
                .map(|e| e.address())
        };
        Some((
            alt.interface_number(),
            bulk(Direction::In)?,
            bulk(Direction::Out)?,
        ))
    })
}

#[cfg(test)]
#[path = "passthrough.test.rs"]
mod tests;
