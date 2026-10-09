use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::input::h264;
use crate::output::hdmi::GstPlayer;
use crate::output::recording::Recorder;
use crate::output::rtmp::{RtmpOutput, RtmpState};
use crate::output::rtsp::RtspServer;
use crate::output::srt::SrtOutput;
use crate::output::udp::UdpOutput;

const INTERVAL: Duration = Duration::from_secs(1);
/// Half an hour
const HISTORY: usize = 1800;
/// The 3D core. Video decoding and encoding run elsewhere on the chip and
/// report no load at all
const GPU_STATS: &str = "/sys/devices/platform/v3dbus/fec00000.v3d/gpu_stats";

pub struct Metrics {
    input_bytes: AtomicU64,
    input_pictures: AtomicU64,
    /// Browsers watching, kept current by the web server
    pub viewers: AtomicUsize,
    history: Mutex<VecDeque<Sample>>,
}

/// What the sampler asks about the outputs
pub struct Sources {
    pub player: Arc<GstPlayer>,
    pub rtsp: Arc<RtspServer>,
    pub srt: Arc<SrtOutput>,
    pub rtmp: Arc<RtmpOutput>,
    pub udp: Arc<UdpOutput>,
    pub recorder: Arc<Recorder>,
}

struct Sample {
    seq: u64,
    unix_ms: u64,
    values: Vec<(String, f32)>,
}

#[derive(Serialize)]
pub struct Snapshot {
    /// Number of the newest sample; pass it back as `after` for only the
    /// ones since
    pub seq: u64,
    /// Number of the first sample returned
    pub first: u64,
    pub interval_ms: u64,
    pub history: usize,
    /// When each sample was taken, Unix milliseconds
    pub t: Vec<u64>,
    /// One value per entry of `t`; null where a reading didn't exist then
    pub series: BTreeMap<String, Vec<Option<f32>>>,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            input_bytes: AtomicU64::new(0),
            input_pictures: AtomicU64::new(0),
            viewers: AtomicUsize::new(0),
            history: Mutex::new(VecDeque::with_capacity(HISTORY)),
        })
    }

    /// Video on its way to the outputs. Every picture ends in a delimiter,
    /// which makes them countable without decoding
    pub fn video(&self, chunk: &[u8]) {
        self.input_bytes
            .fetch_add(chunk.len() as u64, Ordering::Relaxed);
        let pictures = h264::nals(chunk)
            .filter(|nal| nal.kind == h264::AUD)
            .count();
        self.input_pictures
            .fetch_add(pictures as u64, Ordering::Relaxed);
    }

    pub fn start(self: &Arc<Self>, sources: Sources) {
        let this = Arc::clone(self);
        thread::spawn(move || {
            let mut sampler = Sampler::new(this, sources);
            let mut next = Instant::now() + INTERVAL;
            loop {
                thread::sleep(next.saturating_duration_since(Instant::now()));
                next += INTERVAL;
                sampler.sample();
            }
        });
    }

    pub fn snapshot(&self, after: u64) -> Snapshot {
        let history = self.history.lock().unwrap();
        let latest = history.back().map_or(0, |s| s.seq);
        // A number from before a restart: start over with everything
        let after = if after > latest { 0 } else { after };
        let fresh: Vec<&Sample> = history.iter().filter(|s| s.seq > after).collect();
        let mut series: BTreeMap<String, Vec<Option<f32>>> = BTreeMap::new();
        for (i, sample) in fresh.iter().enumerate() {
            for (name, value) in &sample.values {
                let column = match series.get_mut(name) {
                    Some(column) => column,
                    None => series
                        .entry(name.clone())
                        .or_insert_with(|| vec![None; fresh.len()]),
                };
                column[i] = Some(*value);
            }
        }
        Snapshot {
            seq: latest,
            first: fresh.first().map_or(latest + 1, |s| s.seq),
            interval_ms: INTERVAL.as_millis() as u64,
            history: HISTORY,
            t: fresh.iter().map(|s| s.unix_ms).collect(),
            series,
        }
    }
}

struct Sampler {
    metrics: Arc<Metrics>,
    sources: Sources,
    seq: u64,
    at: Instant,
    /// Per processor ("cpu", "cpu0", ...): busy and total ticks
    cpu: HashMap<String, (u64, u64)>,
    app_ticks: u64,
    gpu_ns: u64,
    /// Per interface: bytes received and sent
    net: HashMap<String, (u64, u64)>,
    input_bytes: u64,
    input_pictures: u64,
    hdmi: Option<(u64, u64)>,
}

impl Sampler {
    fn new(metrics: Arc<Metrics>, sources: Sources) -> Self {
        let mut this = Self {
            metrics,
            sources,
            seq: 0,
            at: Instant::now(),
            cpu: HashMap::new(),
            app_ticks: 0,
            gpu_ns: 0,
            net: HashMap::new(),
            input_bytes: 0,
            input_pictures: 0,
            hdmi: None,
        };
        // Rates need a reading to compare with; this one isn't kept
        this.collect();
        this
    }

    fn sample(&mut self) {
        let values = self.collect();
        self.seq += 1;
        let sample = Sample {
            seq: self.seq,
            unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
            values,
        };
        let mut history = self.metrics.history.lock().unwrap();
        if history.len() == HISTORY {
            history.pop_front();
        }
        history.push_back(sample);
    }

    fn collect(&mut self) -> Vec<(String, f32)> {
        let now = Instant::now();
        let secs = now.duration_since(self.at).as_secs_f32().max(0.001);
        self.at = now;
        let mut out = Vec::with_capacity(40);
        let mut put = |name: &str, value: f32| {
            out.push((name.to_string(), (value * 10.0).round() / 10.0));
        };
        let read = |path: &str| fs::read_to_string(path).ok();

        // Processors
        let stat = read("/proc/stat").unwrap_or_default();
        let mut whole_machine_ticks = 0;
        for (name, busy, total) in cpu_times(&stat) {
            if let Some((was_busy, was_total)) = self.cpu.insert(name.clone(), (busy, total)) {
                let span = total.saturating_sub(was_total);
                if name == "cpu" {
                    whole_machine_ticks = span;
                }
                if span > 0 {
                    put(
                        &name,
                        busy.saturating_sub(was_busy) as f32 * 100.0 / span as f32,
                    );
                }
            }
        }
        // This program's share of the whole machine, like the total above
        if let Some(ticks) = read("/proc/self/stat").and_then(|s| own_ticks(&s)) {
            if whole_machine_ticks > 0 {
                let used = ticks.saturating_sub(self.app_ticks) as f32;
                put("cpu_app", used * 100.0 / whole_machine_ticks as f32);
            }
            self.app_ticks = ticks;
        }
        if let Some(ns) = read(GPU_STATS).map(|s| gpu_runtime_ns(&s)) {
            let busy = ns.saturating_sub(self.gpu_ns) as f32 / 1e9;
            put("gpu", (busy * 100.0 / secs).min(100.0));
            self.gpu_ns = ns;
        }
        let number = |path: &str| read(path).and_then(|s| s.trim().parse::<f32>().ok());
        if let Some(millidegrees) = number("/sys/class/thermal/thermal_zone0/temp") {
            put("temp", millidegrees / 1000.0);
        }
        if let Some(khz) = number("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq") {
            put("freq", khz / 1000.0);
        }
        if let Some(alarm) = number("/sys/class/hwmon/hwmon1/in0_lcrit_alarm") {
            put("undervoltage", alarm);
        }
        if let Some(load) =
            read("/proc/loadavg").and_then(|s| s.split_whitespace().next()?.parse::<f32>().ok())
        {
            put("load", load);
        }

        // Memory, in MB
        let meminfo = read("/proc/meminfo").unwrap_or_default();
        let kb = |text: &str, key: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(key))
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|v| v.parse::<f32>().ok())
        };
        if let (Some(total), Some(available)) =
            (kb(&meminfo, "MemTotal:"), kb(&meminfo, "MemAvailable:"))
        {
            put("mem_total", total / 1024.0);
            put("mem_used", (total - available) / 1024.0);
        }
        if let Some(rss) = read("/proc/self/status").and_then(|s| kb(&s, "VmRSS:")) {
            put("mem_app", rss / 1024.0);
        }

        // Network, in kbit/s
        let dev = read("/proc/net/dev").unwrap_or_default();
        let mut seen = HashMap::new();
        for (name, rx, tx) in interfaces(&dev) {
            if let Some((was_rx, was_tx)) = self.net.get(&name) {
                let rate =
                    |now: u64, was: u64| now.saturating_sub(was) as f32 * 8.0 / 1000.0 / secs;
                put(&format!("net.{name}.rx"), rate(rx, *was_rx));
                put(&format!("net.{name}.tx"), rate(tx, *was_tx));
            }
            seen.insert(name, (rx, tx));
        }
        self.net = seen;
        if let Some(dbm) = read("/proc/net/wireless").and_then(|s| wifi_signal(&s)) {
            put("wifi_dbm", dbm);
        }

        // Video
        let bytes = self.metrics.input_bytes.load(Ordering::Relaxed);
        let pictures = self.metrics.input_pictures.load(Ordering::Relaxed);
        put("fps_in", (pictures - self.input_pictures) as f32 / secs);
        put(
            "kbps_in",
            (bytes - self.input_bytes) as f32 * 8.0 / 1000.0 / secs,
        );
        self.input_bytes = bytes;
        self.input_pictures = pictures;

        let hdmi = self.sources.player.frames();
        if let (Some((shown, dropped)), Some((was_shown, was_dropped))) = (hdmi, self.hdmi) {
            put("fps_hdmi", shown.saturating_sub(was_shown) as f32 / secs);
            put(
                "hdmi_dropped",
                dropped.saturating_sub(was_dropped) as f32 / secs,
            );
        }
        self.hdmi = hdmi;

        // Who's taking it
        put(
            "viewers_web",
            self.metrics.viewers.load(Ordering::Relaxed) as f32,
        );
        if let Some(clients) = self.sources.rtsp.clients() {
            put("viewers_rtsp", clients as f32);
        }
        let srt = self.sources.srt.status();
        if srt.active {
            put("viewers_srt", srt.clients as f32);
        }
        let udp = self.sources.udp.status();
        if udp.active {
            put("udp_destinations", udp.destinations as f32);
        }
        let rtmp = self.sources.rtmp.status();
        if rtmp.state != RtmpState::Off {
            let live = rtmp.state == RtmpState::Live;
            put(
                "kbps_rtmp",
                if live { rtmp.bitrate_kbps as f32 } else { 0.0 },
            );
        }
        put("recording", self.sources.recorder.is_active() as u8 as f32);
        out
    }
}

/// `/proc/stat`: per processor, the ticks spent busy and in total
fn cpu_times(stat: &str) -> Vec<(String, u64, u64)> {
    stat.lines()
        .take_while(|l| l.starts_with("cpu"))
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?.to_string();
            let ticks: Vec<u64> = fields.filter_map(|f| f.parse().ok()).collect();
            if ticks.len() < 5 {
                return None;
            }
            // user nice system idle iowait irq softirq steal; the rest are
            // guest times, already counted in user
            let total: u64 = ticks.iter().take(8).sum();
            let idle = ticks[3] + ticks[4];
            Some((name, total - idle, total))
        })
        .collect()
}

/// `/proc/self/stat`: user plus system ticks. The name in brackets may hold
/// spaces, so fields are counted from after it
fn own_ticks(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace().skip(11);
    let user: u64 = fields.next()?.parse().ok()?;
    let system: u64 = fields.next()?.parse().ok()?;
    Some(user + system)
}

/// The busiest of the 3D core's queues: they overlap, so adding them up
/// would count time twice
fn gpu_runtime_ns(stats: &str) -> u64 {
    stats
        .lines()
        .skip(1)
        .filter_map(|l| l.split_whitespace().nth(3)?.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
}

/// `/proc/net/dev`, without the interfaces that aren't a way in or out
fn interfaces(dev: &str) -> Vec<(String, u64, u64)> {
    const INTERNAL: [&str; 5] = ["lo", "docker", "br-", "veth", "dummy"];
    dev.lines()
        .filter_map(|line| {
            let (name, counters) = line.split_once(':')?;
            let name = name.trim();
            if INTERNAL.iter().any(|p| name.starts_with(p)) {
                return None;
            }
            let fields: Vec<u64> = counters
                .split_whitespace()
                .filter_map(|f| f.parse().ok())
                .collect();
            (fields.len() >= 9).then(|| (name.to_string(), fields[0], fields[8]))
        })
        .collect()
}

/// `/proc/net/wireless`: signal level of the first interface that has a link
fn wifi_signal(wireless: &str) -> Option<f32> {
    wireless.lines().skip(2).find_map(|line| {
        let mut fields = line.split_whitespace().skip(2);
        let quality: f32 = fields.next()?.trim_end_matches('.').parse().ok()?;
        let level: f32 = fields.next()?.trim_end_matches('.').parse().ok()?;
        (quality > 0.0).then_some(level)
    })
}

#[cfg(test)]
#[path = "metrics.test.rs"]
mod tests;
