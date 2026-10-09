use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;
use serde::Serialize;

use crate::input::h264::has_keyframe;
use crate::settings::{RecordingFormat, RecordingSettings};

const EXTENSIONS: [&str; 2] = ["mp4", "ts"];
const MP4_FRAGMENT_MS: u32 = 1000;
// Long enough to ride out a breakup in the air, short enough that landing
// ends the clip
const VIDEO_GONE: Duration = Duration::from_secs(10);
// FAT32 can't hold a file of 4 GiB. A split lands on the next keyframe and
// an MP4 still gets its index written afterwards, hence the margin
const FAT_MAX_FILE_BYTES: u64 = 3_500 * 1024 * 1024;

struct Session {
    pipeline: gst::Pipeline,
    appsrc: AppSrc,
    started: Instant,
    /// Stem shared by every file of this session (no extension, no index)
    stem: String,
    /// Recording must begin at a keyframe, or the first file starts with
    /// frames that reference pictures we never wrote
    waiting_for_keyframe: bool,
    /// Started by itself when video arrived, so it also ends by itself
    /// when the video goes
    automatic: bool,
    last_video: Instant,
}

pub struct Recorder {
    session: Mutex<Option<Session>>,
    cfg: Mutex<RecordingSettings>,
    /// Why recording stopped on its own (disk full, pipeline error)
    error: Mutex<Option<String>>,
    /// Cleared once auto-start has fired, so stopping by hand sticks
    auto_start_pending: AtomicBool,
}

#[derive(Serialize, Clone, PartialEq)]
pub struct RecordingStatus {
    pub active: bool,
    pub elapsed_secs: u64,
    pub bytes: u64,
    /// The file, or the first of them when the session is split
    pub file: Option<String>,
    pub error: Option<String>,
    pub storage: Storage,
}

#[derive(Serialize, Clone, PartialEq)]
pub struct Storage {
    pub path: String,
    /// False when the folder doesn't exist yet and couldn't be created
    pub writable: bool,
    pub total_bytes: u64,
    pub free_bytes: u64,
    /// Space taken by recordings in this folder
    pub used_bytes: u64,
    pub count: u32,
}

#[derive(Serialize)]
pub struct RecordingFile {
    pub name: String,
    pub size_bytes: u64,
    /// Unix seconds
    pub modified: u64,
}

/// A place recordings could be written, offered in the UI
#[derive(Serialize)]
pub struct StorageOption {
    pub path: String,
    pub label: String,
    pub free_bytes: u64,
    pub removable: bool,
    /// The configured drive, when it isn't plugged in
    pub missing: bool,
}

pub struct Mount {
    pub device: String,
    pub point: PathBuf,
    pub fstype: String,
}

fn parse_mounts(table: &str) -> Vec<Mount> {
    table
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let (device, point, fstype) = (f.next()?, f.next()?, f.next()?);
            Some(Mount {
                device: device.to_string(),
                // /proc/mounts escapes spaces and friends as octal
                point: PathBuf::from(point.replace("\\040", " ")),
                fstype: fstype.to_string(),
            })
        })
        .collect()
}

pub fn mounts() -> Vec<Mount> {
    parse_mounts(&fs::read_to_string("/proc/mounts").unwrap_or_default())
}

/// The filesystem a path is on
pub fn mount_of<'a>(path: &Path, mounts: &'a [Mount]) -> Option<&'a Mount> {
    mounts
        .iter()
        .filter(|m| path.starts_with(&m.point))
        .max_by_key(|m| m.point.components().count())
}

/// Where plugged-in drives get mounted
fn external(path: &Path) -> bool {
    path.starts_with("/media") || path.starts_with("/mnt")
}

/// A folder meant to be on a plugged-in drive, with no drive under it
fn drive_missing(dir: &Path, mounts: &[Mount]) -> bool {
    external(dir) && !mount_of(dir, mounts).is_some_and(|m| external(&m.point))
}

/// The drive a folder is on, if it's a plugged-in one
pub fn drive_of(dir: &Path) -> Option<Mount> {
    let mounts = mounts();
    let i = mounts
        .iter()
        .position(|m| mount_of(dir, &mounts).is_some_and(|found| std::ptr::eq(found, m)))?;
    let mount = mounts.into_iter().nth(i)?;
    (external(&mount.point) && mount.device.starts_with("/dev/")).then_some(mount)
}

impl Recorder {
    pub fn new(cfg: RecordingSettings) -> Self {
        let auto = cfg.auto_start;
        Self {
            session: Mutex::new(None),
            cfg: Mutex::new(cfg),
            error: Mutex::new(None),
            auto_start_pending: AtomicBool::new(auto),
        }
    }

    pub fn configure(&self, cfg: &RecordingSettings) {
        let mut guard = self.cfg.lock().unwrap();
        if guard.auto_start != cfg.auto_start {
            self.auto_start_pending
                .store(cfg.auto_start, Ordering::Relaxed);
        }
        *guard = cfg.clone();
        // Path, format and split length apply to the next recording; the
        // running one keeps the settings it started with
    }

    pub fn is_active(&self) -> bool {
        self.session.lock().unwrap().is_some()
    }

    /// Cheap and safe to call when not recording
    pub fn push(&self, data: &[u8]) {
        let mut guard = self.session.lock().unwrap();
        let Some(session) = guard.as_mut() else {
            // Nothing recording: pick up auto-start on the first keyframe
            drop(guard);
            if self.auto_start_pending.load(Ordering::Relaxed) && has_keyframe(data) {
                self.auto_start_pending.store(false, Ordering::Relaxed);
                if let Err(e) = self.start_session(true) {
                    eprintln!("[rec] auto-start failed: {e}");
                    *self.error.lock().unwrap() = Some(e);
                }
            }
            return;
        };

        session.last_video = Instant::now();
        if session.waiting_for_keyframe {
            if !has_keyframe(data) {
                return;
            }
            session.waiting_for_keyframe = false;
        }

        let Ok(mut buffer) = gst::Buffer::with_size(data.len()) else {
            return;
        };
        {
            let buffer_ref = buffer.get_mut().unwrap();
            if buffer_ref.copy_from_slice(0, data).is_err() {
                return;
            }
        }
        let _ = session.appsrc.push_buffer(buffer);
    }

    pub fn start(&self) -> Result<(), String> {
        self.start_session(false)
    }

    fn start_session(&self, automatic: bool) -> Result<(), String> {
        let mut guard = self.session.lock().unwrap();
        if guard.is_some() {
            return Err("Already recording".into());
        }
        let cfg = self.cfg.lock().unwrap().clone();

        let dir = PathBuf::from(&cfg.path);
        let mounts = mounts();
        if drive_missing(&dir, &mounts) {
            return Err(format!("The drive for {} isn't plugged in", dir.display()));
        }
        fs::create_dir_all(&dir).map_err(|e| format!("Can't use {}: {e}", dir.display()))?;
        let max_file_bytes = match mount_of(&dir, &mounts) {
            Some(m) if m.fstype == "vfat" || m.fstype == "msdos" => FAT_MAX_FILE_BYTES,
            // 0 = never split
            _ => 0,
        };
        if let Some((_, free)) = disk_usage(&dir)
            && free < cfg.reserve_mb as u64 * 1024 * 1024
        {
            return Err(format!(
                "Not enough free space ({} left, {} MB reserved)",
                human_bytes(free),
                cfg.reserve_mb
            ));
        }

        let stem = format!("{}-{}", sanitize(&cfg.prefix), local_timestamp());
        let ext = cfg.format.extension();
        // With splitting on, splitmuxsink fills in the %03d for each part
        let location = if cfg.split_minutes > 0 || max_file_bytes > 0 {
            dir.join(format!("{stem}-%03d.{ext}"))
        } else {
            dir.join(format!("{stem}.{ext}"))
        };

        let pipeline = gst::Pipeline::new();
        let appsrc = gst::ElementFactory::make("appsrc")
            .property("is-live", true)
            .property("format", gst::Format::Time)
            .property("do-timestamp", true)
            .property(
                "caps",
                gst::Caps::builder("video/x-h264")
                    .field("stream-format", "byte-stream")
                    .field("alignment", "stream")
                    .build(),
            )
            .build()
            .map_err(|e| format!("appsrc: {e}"))?;
        let parse = gst::ElementFactory::make("h264parse")
            .build()
            .map_err(|e| format!("h264parse: {e}"))?;
        let muxer = gst::ElementFactory::make(cfg.format.muxer())
            .build()
            .map_err(|e| format!("{}: {e}", cfg.format.muxer()))?;
        if cfg.format == RecordingFormat::Mp4 {
            muxer.set_property("fragment-duration", MP4_FRAGMENT_MS);
            muxer.set_property_from_str("fragment-mode", "first-moov-then-finalise");
        }
        let sink = gst::ElementFactory::make("splitmuxsink")
            .property("location", location.to_string_lossy().as_ref())
            .property("muxer", &muxer)
            // 0 = never split
            .property(
                "max-size-time",
                cfg.split_minutes as u64 * 60 * 1_000_000_000,
            )
            .property("max-size-bytes", max_file_bytes)
            // The goggles' own keyframes decide where splits land
            .property("send-keyframe-requests", false)
            .build()
            .map_err(|e| format!("splitmuxsink: {e}"))?;

        pipeline
            .add_many([&appsrc, &parse, &sink])
            .map_err(|e| format!("pipeline: {e}"))?;
        gst::Element::link_many([&appsrc, &parse, &sink]).map_err(|e| format!("link: {e}"))?;

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("Couldn't start recording: {e}"))?;

        let appsrc = appsrc
            .dynamic_cast::<AppSrc>()
            .map_err(|_| "appsrc cast".to_string())?;

        eprintln!("[rec] recording to {}", location.display());
        *self.error.lock().unwrap() = None;
        *guard = Some(Session {
            pipeline,
            appsrc,
            started: Instant::now(),
            stem,
            waiting_for_keyframe: true,
            automatic,
            last_video: Instant::now(),
        });
        Ok(())
    }

    pub fn stop(&self) -> Result<(), String> {
        let Some(session) = self.session.lock().unwrap().take() else {
            return Err("Not recording".into());
        };
        self.auto_start_pending.store(false, Ordering::Relaxed);
        finish(&session);
        eprintln!(
            "[rec] stopped after {}s",
            session.started.elapsed().as_secs()
        );
        Ok(())
    }

    fn stop_with_error(&self, reason: String) {
        if let Some(session) = self.session.lock().unwrap().take() {
            finish(&session);
        }
        self.auto_start_pending.store(false, Ordering::Relaxed);
        eprintln!("[rec] {reason}");
        *self.error.lock().unwrap() = Some(reason);
    }

    /// Called about once a second: watches free space and the pipeline bus
    pub fn tick(&self) {
        let cfg = self.cfg.lock().unwrap().clone();
        let bus_error = {
            let mut guard = self.session.lock().unwrap();
            let Some(session) = guard.as_ref() else {
                return;
            };
            if session.automatic && session.last_video.elapsed() > VIDEO_GONE {
                let ended = guard.take();
                // Finishing a file can take seconds; don't hold up the status
                drop(guard);
                if let Some(session) = ended {
                    finish(&session);
                }
                eprintln!("[rec] stopped: no video for {}s", VIDEO_GONE.as_secs());
                // The next video gets a file of its own
                self.auto_start_pending
                    .store(cfg.auto_start, Ordering::Relaxed);
                return;
            }
            let Some(session) = guard.as_ref() else {
                return;
            };
            session
                .pipeline
                .bus()
                .and_then(|bus| bus.pop_filtered(&[gst::MessageType::Error]))
                .map(|msg| match msg.view() {
                    gst::MessageView::Error(e) => e.error().to_string(),
                    _ => "pipeline error".to_string(),
                })
        };
        if let Some(e) = bus_error {
            self.stop_with_error(format!("Recording stopped: {e}"));
            return;
        }
        if let Some((_, free)) = disk_usage(Path::new(&cfg.path))
            && free < cfg.reserve_mb as u64 * 1024 * 1024
        {
            self.stop_with_error(format!(
                "Recording stopped: only {} left on {}",
                human_bytes(free),
                cfg.path
            ));
        }
    }

    pub fn status(&self) -> RecordingStatus {
        let cfg = self.cfg.lock().unwrap().clone();
        let dir = PathBuf::from(&cfg.path);
        let guard = self.session.lock().unwrap();
        let (active, elapsed, bytes, file) = match guard.as_ref() {
            Some(s) => {
                let parts = session_files(&dir, &s.stem);
                let bytes = parts.iter().map(|(_, size, _)| size).sum();
                let first = parts.first().map(|(name, _, _)| name.clone());
                (true, s.started.elapsed().as_secs(), bytes, first)
            }
            None => (false, 0, 0, None),
        };
        drop(guard);

        RecordingStatus {
            active,
            elapsed_secs: elapsed,
            bytes,
            file,
            error: self.error.lock().unwrap().clone(),
            storage: storage(&dir),
        }
    }

    pub fn dir(&self) -> PathBuf {
        PathBuf::from(&self.cfg.lock().unwrap().path)
    }

    /// Whether `name` in `dir` is a file of the recording in progress, which
    /// must be neither deleted nor copied yet
    pub fn is_writing(&self, dir: &Path, name: &str) -> bool {
        dir == self.dir()
            && self
                .session
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|s| name.starts_with(&s.stem))
    }
}

/// Drain the pipeline: EOS lets splitmuxsink write the index of the file it
/// has open, which is what makes an MP4 seekable
fn finish(session: &Session) {
    let _ = session.appsrc.end_of_stream();
    if let Some(bus) = session.pipeline.bus() {
        let _ = bus.timed_pop_filtered(
            gst::ClockTime::from_seconds(5),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        );
    }
    let _ = session.pipeline.set_state(gst::State::Null);
}

fn session_files(dir: &Path, stem: &str) -> Vec<(String, u64, u64)> {
    list(dir)
        .into_iter()
        .filter(|f| f.name.starts_with(stem))
        .map(|f| (f.name, f.size_bytes, f.modified))
        .collect()
}

pub fn list(dir: &Path) -> Vec<RecordingFile> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<RecordingFile> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_str()?.to_ascii_lowercase();
            if !EXTENSIONS.contains(&ext.as_str()) {
                return None;
            }
            let meta = e.metadata().ok()?;
            Some(RecordingFile {
                name: path.file_name()?.to_str()?.to_string(),
                size_bytes: meta.len(),
                modified: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            })
        })
        .collect();
    // Newest first
    files.sort_by(|a, b| b.modified.cmp(&a.modified).then(b.name.cmp(&a.name)));
    files
}

/// Resolve a user-supplied file name inside `dir`, refusing anything that
/// isn't a plain recording file sitting directly in it
pub fn resolve(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let bad = name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.starts_with('.')
        || Path::new(name).components().count() != 1;
    let ext_ok = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false);
    if bad || !ext_ok {
        return Err("Not a recording file".into());
    }
    let path = dir.join(name);
    if !path.is_file() {
        return Err("No such recording".into());
    }
    Ok(path)
}

pub fn storage(dir: &Path) -> Storage {
    // Creating the folder of a drive that isn't there would put it, and the
    // recordings, on the system card
    let writable = !drive_missing(dir, &mounts())
        && fs::create_dir_all(dir).is_ok()
        && fs::metadata(dir)
            .map(|m| !m.permissions().readonly())
            .unwrap_or(false);
    let files = list(dir);
    let (total, free) = disk_usage(dir).unwrap_or((0, 0));
    Storage {
        path: dir.display().to_string(),
        writable,
        total_bytes: total,
        free_bytes: free,
        used_bytes: files.iter().map(|f| f.size_bytes).sum(),
        count: files.len() as u32,
    }
}

/// Mounted filesystems that could hold recordings: the SD card plus anything
/// mounted under /media or /mnt (USB sticks, typically)
pub fn storage_options(current: &Path) -> Vec<StorageOption> {
    let mut options = vec![StorageOption {
        path: "/var/lib/ystreamer/recordings".into(),
        label: "SD card".into(),
        // On the image the recordings have a partition of their own
        free_bytes: disk_usage(Path::new("/var/lib/ystreamer"))
            .or_else(|| disk_usage(Path::new("/")))
            .map(|(_, f)| f)
            .unwrap_or(0),
        removable: false,
        missing: false,
    }];

    let mounts = mounts();
    for m in &mounts {
        if !external(&m.point) || !m.device.starts_with("/dev/") {
            continue;
        }
        options.push(StorageOption {
            label: drive_label(&m.point),
            free_bytes: disk_usage(&m.point).map(|(_, f)| f).unwrap_or(0),
            path: m.point.join("ystreamer").display().to_string(),
            removable: true,
            missing: false,
        });
    }

    // Keep whatever is configured selectable, even somewhere custom
    let shown = current.display().to_string();
    if !options.iter().any(|o| o.path == shown) {
        let missing = drive_missing(current, &mounts);
        options.insert(
            0,
            StorageOption {
                label: match current.parent() {
                    Some(drive) if missing => drive_label(drive),
                    _ => "Current folder".into(),
                },
                free_bytes: if missing {
                    0
                } else {
                    disk_usage(current).map(|(_, f)| f).unwrap_or(0)
                },
                path: shown,
                removable: missing,
                missing,
            },
        );
    }
    options
}

fn drive_label(point: &Path) -> String {
    point
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| point.display().to_string())
}

pub fn disk_usage(path: &Path) -> Option<(u64, u64)> {
    // statvfs needs a path that exists; walk up until one does
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let c = std::ffi::CString::new(probe.as_os_str().as_encoded_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let frsize = st.f_frsize as u64;
    Some((st.f_blocks as u64 * frsize, st.f_bavail as u64 * frsize))
}

fn human_bytes(n: u64) -> String {
    let gb = n as f64 / 1024.0_f64.powi(3);
    if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{} MB", n / 1024 / 1024)
    }
}

/// `YYYYmmdd-HHMMSS` in local time
fn local_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    crate::system::reload_timezone();
    unsafe { libc::localtime_r(&now, &mut tm) };
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

fn sanitize(prefix: &str) -> String {
    let cleaned: String = prefix
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if cleaned.is_empty() {
        "ystreamer".into()
    } else {
        cleaned
    }
}

impl RecordingFormat {
    fn extension(self) -> &'static str {
        match self {
            RecordingFormat::Mp4 => "mp4",
            RecordingFormat::Ts => "ts",
        }
    }

    fn muxer(self) -> &'static str {
        match self {
            RecordingFormat::Mp4 => "mp4mux",
            RecordingFormat::Ts => "mpegtsmux",
        }
    }
}

#[cfg(test)]
#[path = "recording.test.rs"]
mod tests;
