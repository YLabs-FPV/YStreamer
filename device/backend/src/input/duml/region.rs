//! The radio region: CE or FCC transmit limits, the choice DJI's own apps
//! make by country. The region goes to the goggles' transceiver and the
//! aircraft's, and never to the flight controller: writing its parameters
//! can hold up the GPS lock or leave the aircraft in ATTI.
//!
//! The frames follow tw-streamer's notes on the transceivers, worked out from
//! the goggles' and aircraft's own firmware. They haven't been flight-tested.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::devices::Kind;
use super::session::Session;
use super::{calc_crc16, encode, next_seq};
use crate::input::goggles::{Goggles, LinkState};
use crate::settings::{RadioRegion, RadioRegionSettings, SettingsStore};

const SERVICE: u8 = 0x12;
const REMOTE_RADIO: u8 = 0x06;
const GROUND_SDR: u8 = 0x09;
const SKY_SDR: u8 = 0x0E;

const CMD_SET_SERVICE: u8 = 0x10;
const CMD_SERVICE_MODE: u8 = 0x58;
const CMD_SET_RADIO: u8 = 0x06;
const CMD_RADIO_REGION: u8 = 0x72;
const CMD_SET_SDR: u8 = 0x07;
const CMD_SDR_REGION: u8 = 0x30;

/// Acknowledged before it runs, unlike our other requests
const FLAGS_ACK_FIRST: u8 = 0x20;

const BAND_2G4: u8 = 0x01;
const BAND_5G8: u8 = 0x02;

/// Sent the same to enter service mode and to leave it
const SERVICE_MODE: [u8; 3] = [0x03, 0x01, 0x00];
const RADIO_COMMIT: [u8; 7] = [0, 0, 0, 0, 0, 0x01, 0xFF];
const RADIO_FACTORY: [u8; 7] = [0; 7];

/// The transceivers drop back to CE when the region isn't repeated
const KEEPALIVE: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_millis(250);

/// Beside the settings. It says this unit changed the region, so turning the
/// setting off knows there's something to put back, even after a restart
const MARKER: &str = "radio-region-applied";

/// The country twice, the band, a channel map, and a CRC over all of it
fn region_payload(country: [u8; 2], band: u8) -> [u8; 12] {
    let mut p = [0u8; 12];
    p[0..2].copy_from_slice(&country);
    p[4..6].copy_from_slice(&country);
    p[8] = band;
    sealed(p)
}

/// No country at all is the transceivers' factory setting, CE
fn factory_payload() -> [u8; 12] {
    sealed([0u8; 12])
}

fn sealed(mut p: [u8; 12]) -> [u8; 12] {
    let crc = calc_crc16(&p[..10]);
    p[10..].copy_from_slice(&crc.to_le_bytes());
    p
}

fn frame(dst: u8, seq: u16, cmd_set: u8, cmd_id: u8, payload: &[u8]) -> Vec<u8> {
    encode(dst, seq, FLAGS_ACK_FIRST, cmd_set, cmd_id, payload)
}

/// Both bands on both transceivers, inside service mode, then committed
pub fn set_frames(country: [u8; 2], mut seq: impl FnMut() -> u16) -> Vec<Vec<u8>> {
    let low = region_payload(country, BAND_2G4);
    let high = region_payload(country, BAND_5G8);
    vec![
        frame(
            SERVICE,
            seq(),
            CMD_SET_SERVICE,
            CMD_SERVICE_MODE,
            &SERVICE_MODE,
        ),
        frame(GROUND_SDR, seq(), CMD_SET_SDR, CMD_SDR_REGION, &low),
        frame(GROUND_SDR, seq(), CMD_SET_SDR, CMD_SDR_REGION, &high),
        frame(SKY_SDR, seq(), CMD_SET_SDR, CMD_SDR_REGION, &low),
        frame(SKY_SDR, seq(), CMD_SET_SDR, CMD_SDR_REGION, &high),
        frame(
            REMOTE_RADIO,
            seq(),
            CMD_SET_RADIO,
            CMD_RADIO_REGION,
            &RADIO_COMMIT,
        ),
        frame(
            SERVICE,
            seq(),
            CMD_SET_SERVICE,
            CMD_SERVICE_MODE,
            &SERVICE_MODE,
        ),
    ]
}

/// The short repeat that keeps the region from dropping back to CE
pub fn keepalive_frames(country: [u8; 2], mut seq: impl FnMut() -> u16) -> Vec<Vec<u8>> {
    let low = region_payload(country, BAND_2G4);
    vec![
        frame(GROUND_SDR, seq(), CMD_SET_SDR, CMD_SDR_REGION, &low),
        frame(SKY_SDR, seq(), CMD_SET_SDR, CMD_SDR_REGION, &low),
        frame(
            REMOTE_RADIO,
            seq(),
            CMD_SET_RADIO,
            CMD_RADIO_REGION,
            &RADIO_COMMIT,
        ),
    ]
}

/// Both transceivers back to their factory CE setting
pub fn restore_frames(mut seq: impl FnMut() -> u16) -> Vec<Vec<u8>> {
    let factory = factory_payload();
    vec![
        frame(GROUND_SDR, seq(), CMD_SET_SDR, CMD_SDR_REGION, &factory),
        frame(SKY_SDR, seq(), CMD_SET_SDR, CMD_SDR_REGION, &factory),
        frame(
            REMOTE_RADIO,
            seq(),
            CMD_SET_RADIO,
            CMD_RADIO_REGION,
            &RADIO_FACTORY,
        ),
    ]
}

/// What a link gets in full when the region is applied to it
fn apply_frames(region: RadioRegion, seq: impl FnMut() -> u16) -> Vec<Vec<u8>> {
    match region.country() {
        Some(country) => set_frames(country, seq),
        None => restore_frames(seq),
    }
}

#[derive(Debug, PartialEq)]
enum Action {
    Wait,
    Apply(RadioRegion),
    Keepalive([u8; 2]),
    Restore,
}

#[derive(Clone, Copy)]
struct Link {
    /// Live and registered: the goggles take our requests
    up: bool,
    generation: u64,
    aircraft: bool,
}

#[derive(Default)]
struct State {
    generation: u64,
    aircraft: bool,
    /// What this link was last given in full
    applied: Option<RadioRegion>,
    /// This link got the CE restore
    restored: bool,
    sent_at: Option<Instant>,
    reapply: bool,
}

/// One tick's choice. Turning the setting off sends the CE restore only when
/// this unit changed the region: one set some other way isn't ours to undo
fn decide(st: &mut State, link: Link, cfg: RadioRegionSettings, marked: bool) -> Action {
    if link.generation != st.generation {
        *st = State {
            generation: link.generation,
            ..State::default()
        };
    }
    if !link.up {
        return Action::Wait;
    }
    // The aircraft's transceiver only hears what's sent while it's linked
    let aircraft_joined = link.aircraft && !st.aircraft;
    st.aircraft = link.aircraft;
    let reapply = std::mem::take(&mut st.reapply);

    if !cfg.enabled {
        st.applied = None;
        return if marked && (!st.restored || aircraft_joined) {
            Action::Restore
        } else {
            Action::Wait
        };
    }
    if st.applied != Some(cfg.region) || aircraft_joined || reapply {
        return Action::Apply(cfg.region);
    }
    match cfg.region.country() {
        Some(country) if st.sent_at.is_none_or(|at| at.elapsed() >= KEEPALIVE) => {
            Action::Keepalive(country)
        }
        _ => Action::Wait,
    }
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct RegionStatus {
    /// As saved in the settings
    pub enabled: bool,
    /// This unit changed the region and hasn't put CE back yet
    pub applied: bool,
    /// What the goggles connected now were last given in full
    pub active: Option<RadioRegion>,
    pub sent_ms_ago: Option<u64>,
}

/// Keeps the radios on the region from the settings while the goggles are
/// connected
pub struct Applier {
    goggles: Goggles,
    session: Arc<Session>,
    store: Arc<SettingsStore>,
    marker: PathBuf,
    state: Mutex<State>,
}

impl Applier {
    pub fn start(goggles: Goggles, session: Arc<Session>, store: Arc<SettingsStore>) -> Arc<Self> {
        let applier = Arc::new(Self {
            goggles,
            session,
            marker: store.path().with_file_name(MARKER),
            store,
            state: Mutex::new(State::default()),
        });
        let this = Arc::clone(&applier);
        thread::spawn(move || {
            loop {
                this.tick();
                thread::sleep(TICK);
            }
        });
        applier
    }

    pub fn status(&self) -> RegionStatus {
        let st = self.state.lock().unwrap();
        let current =
            st.generation == self.goggles.generation() && self.goggles.state() == LinkState::Live;
        RegionStatus {
            enabled: self.store.get().radio_region.enabled,
            applied: self.marker.exists(),
            active: st.applied.filter(|_| current),
            sent_ms_ago: st
                .sent_at
                .filter(|_| current)
                .map(|at| at.elapsed().as_millis() as u64),
        }
    }

    /// Send the whole set again on the next tick
    pub fn reapply(&self) {
        self.state.lock().unwrap().reapply = true;
    }

    fn tick(&self) {
        let cfg = self.store.get().radio_region;
        let session = self.session.status();
        let link = Link {
            up: self.goggles.state() == LinkState::Live && session.registered,
            generation: self.goggles.generation(),
            aircraft: session.devices.iter().any(|d| d.kind == Kind::Aircraft),
        };
        let marked = self.marker.exists();

        let mut st = self.state.lock().unwrap();
        let action = decide(&mut st, link, cfg, marked);
        let frames = match action {
            Action::Wait => return,
            Action::Apply(region) => {
                // Before sending, so a restart halfway still knows
                if let Err(e) = fs::write(&self.marker, format!("{region:?}\n")) {
                    eprintln!("[radio] Can't write {}: {e}", self.marker.display());
                }
                apply_frames(region, next_seq)
            }
            Action::Keepalive(country) => keepalive_frames(country, next_seq),
            Action::Restore => restore_frames(next_seq),
        };
        if self.send(&frames).is_err() {
            return;
        }
        st.sent_at = Some(Instant::now());
        match action {
            Action::Apply(region) => {
                st.applied = Some(region);
                st.restored = false;
                eprintln!("[radio] Region set to {region:?}");
            }
            Action::Restore => {
                st.restored = true;
                if link.aircraft {
                    let _ = fs::remove_file(&self.marker);
                    eprintln!("[radio] Region back to CE");
                } else {
                    eprintln!("[radio] Goggles back to CE; the aircraft follows once it's linked");
                }
            }
            _ => {}
        }
    }

    fn send(&self, frames: &[Vec<u8>]) -> io::Result<()> {
        for f in frames {
            self.goggles.send(f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "region.test.rs"]
mod tests;
