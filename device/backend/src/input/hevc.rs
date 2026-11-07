use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::h264;
use super::h265::{self, ParamSets};

/// After a picture a decoder can start from went in, this long without a
/// picture out means the converter doesn't work here
const FIRST_PICTURE: Duration = Duration::from_secs(4);
/// Pictures stopped coming out this long after they had been: hung
const STALLED: Duration = Duration::from_secs(3);
/// What comes out goes to every output, the browser included, so it's
/// sized like a typical goggles stream
const BITRATE_KBPS: u32 = 8000;
const KEYFRAME_FRAMES: u32 = 30;
/// The Pi 4's H.264 encoder can't keep up with 1080p60 (it tops out near
/// 50 with GStreamer, and won't start at 60 in ffmpeg), so a 60 stream
/// comes out at 30: steady, rather than dropping whichever pictures it's
/// late for
const MAX_FPS: u32 = 30;
/// After the converter failed to start, before trying again
const RETRY: Duration = Duration::from_secs(5);
/// Joined a stream after its keyframe; the drone behind an RC-N remote
/// sends one when it links to the remote, or when asked
pub const WAITING_FOR_KEYFRAME: &str = "Waiting for a keyframe from the drone";
/// Appended to every picture, as the goggles do; see `uvc.rs`
const AUD: [u8; 6] = [0, 0, 0, 1, 0x09, 0xf0];
/// Where ffmpeg starts each H.264 picture (`h264_metadata=aud=insert`)
const PICTURE_START: [u8; 5] = [0, 0, 0, 1, 0x09];
/// Chunks of H.265 waiting for ffmpeg, ~4 KB each: about two seconds,
/// enough for it to start up. More than that and ffmpeg has stopped taking
/// them; they're dropped rather than hold up the link, and a gap in the
/// stream is a damaged picture until it heals, so it's logged
const INPUT_QUEUE: usize = 1024;
const DROPS_LOGGED_EVERY: Duration = Duration::from_secs(5);
/// ffmpeg writes each picture whole, at once: this long after a read with
/// nothing more to read, what's there is complete. Waiting for the next
/// picture's start instead would add a picture's time to the delay
const PICTURE_SETTLED_MS: i32 = 3;
/// ffmpeg's complaints about a damaged stream, at most this many a window
const LOG_LINES: u32 = 5;
const LOG_WINDOW: Duration = Duration::from_secs(10);

/// How H.265 becomes the H.264 every output takes. The Pi 4 decodes
/// H.265 in hardware but only encodes H.264, so this is the one
/// conversion the device makes; outputs stay as they are
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Converter {
    /// Raspberry Pi's ffmpeg, in a process of its own, with the hardware
    /// H.265 decoder and H.264 encoder. Not GStreamer's `v4l2slh265dec`:
    /// in 1.26 it crashes the process (SIGSEGV in `output_picture`) on any
    /// picture whose reference it never got, which a live link brings now
    /// and then; ffmpeg shows a damaged picture for a moment and goes on.
    /// Its own process keeps anything else that goes wrong away from
    /// YStreamer: it's started again, and the drone asked for a keyframe
    Hardware,
}

/// ffmpeg's arguments: H.265 in on stdin, H.264 out on stdout.
/// `in_decoder`: pictures stay in the hardware decoder's memory until
/// they're picked; without it, every one is copied out first, which a
/// stream the hardware turns down (decoded by ffmpeg itself) needs
fn ffmpeg_args(in_decoder: bool) -> Vec<String> {
    let (output_format, filters) = if in_decoder {
        (
            &["-hwaccel_output_format", "drm_prime"][..],
            format!("fps={MAX_FPS},hwdownload,format=yuv420p"),
        )
    } else {
        (&[][..], format!("fps={MAX_FPS},format=yuv420p"))
    };
    let before_input = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostats",
        // Pictures go through as they come, not after a look ahead. Not
        // `-fflags nobuffer`: it throws away what was read while probing,
        // which is the keyframe everything after it refers to
        "-flags",
        "low_delay",
        "-probesize",
        "32",
        "-analyzeduration",
        "0",
        // Frame threads would hold pictures back to decode them in parallel
        "-threads",
        "1",
        "-hwaccel",
        "drm",
    ];
    // Picked while still in the decoder's memory, so only the pictures
    // kept are copied out of it: the copying is what costs (it competes
    // with HDMI for memory), and half the pictures are dropped. Copying
    // all of them, a busy Pi fell behind
    let after_input = [
        "-vf",
        &filters,
        "-fpsmax",
        &MAX_FPS.to_string(),
        "-c:v",
        "h264_v4l2m2m",
        "-b:v",
        &format!("{}k", BITRATE_KBPS),
        "-g",
        &KEYFRAME_FRAMES.to_string(),
        "-bf",
        "0",
        // The parameter sets with every keyframe, so outputs can join at
        // any of them, and a delimiter starting every picture
        "-bsf:v",
        "dump_extra=freq=keyframe,h264_metadata=aud=insert",
        "-flush_packets",
        "1",
        "-f",
        "h264",
        "pipe:1",
    ];
    before_input
        .iter()
        .chain(output_format)
        .chain(&["-f", "hevc", "-i", "pipe:0"])
        .chain(&after_input)
        .map(|a| a.to_string())
        .collect()
}

struct Running {
    child: Child,
    /// To the thread writing into ffmpeg
    input: SyncSender<Vec<u8>>,
    converter: Converter,
    /// When a picture a decoder can start from went in
    irap_at: Option<Instant>,
    /// Chunks dropped because ffmpeg wasn't taking them, since last logged
    dropped: u64,
    dropped_logged: Option<Instant>,
}

/// Gets every converted picture, an H.264 access unit
pub type OnPicture = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// Turns the H.265 from an RC-N remote into H.264 for the outputs
pub struct Transcoder {
    on_picture: OnPicture,
    running: Mutex<Option<Running>>,
    /// Unix millis of the last picture out; shared with the reading thread
    last_out: Arc<AtomicU64>,
    retry_at: Mutex<Option<Instant>>,
    /// See `ffmpeg_args`; off once that way brought no pictures
    in_decoder: Mutex<bool>,
}

impl Transcoder {
    pub fn new(on_picture: OnPicture) -> Self {
        Self {
            on_picture,
            running: Mutex::new(None),
            last_out: Arc::new(AtomicU64::new(0)),
            retry_at: Mutex::new(None),
            in_decoder: Mutex::new(true),
        }
    }

    /// The converter in use, if one is running
    pub fn converter(&self) -> Option<Converter> {
        self.running.lock().unwrap().as_ref().map(|r| r.converter)
    }

    /// Feed the link's bytes; starts the converter when there's none, and
    /// starts it again when it stops working.
    /// Errors are for the status, not fatal
    pub fn push(&self, data: &[u8], params: &ParamSets) -> Result<(), String> {
        let mut running = self.running.lock().unwrap();
        if let Some(r) = running.as_mut()
            && let Some(reason) = self.failed(r)
        {
            stop(running.take());
            eprintln!("[hevc] conversion stopped working ({reason}), starting it again");
            let mut in_decoder = self.in_decoder.lock().unwrap();
            if *in_decoder && self.last_out.load(Ordering::Relaxed) == 0 {
                // Never a picture: the hardware may have turned the stream
                // down, and ffmpeg's own pictures can't be picked that way
                eprintln!("[hevc] copying every picture out of the decoder from now on");
                *in_decoder = false;
            }
        }
        if running.is_none() {
            let mut retry_at = self.retry_at.lock().unwrap();
            if retry_at.is_some_and(|at| Instant::now() < at) {
                return Err("Couldn't convert the H.265 video".into());
            }
            match self.start(params) {
                Ok(r) => {
                    *retry_at = None;
                    *running = Some(r);
                }
                Err(e) => {
                    eprintln!("[hevc] can't convert: {e}");
                    *retry_at = Some(Instant::now() + RETRY);
                    return Err(e);
                }
            }
        }
        let r = running.as_mut().expect("started above");
        // Nothing goes in before a picture the decoder can start from: the
        // ones before it refer to pictures it never had
        let data = match r.irap_at {
            Some(_) => data,
            None => match h265::irap_start(data) {
                Some(start) => {
                    r.irap_at = Some(Instant::now());
                    &data[start..]
                }
                None => return Err(WAITING_FOR_KEYFRAME.into()),
            },
        };
        match r.input.try_send(data.to_vec()) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            // ffmpeg isn't taking any; `failed` sees to it if it lasts
            Err(TrySendError::Full(_)) => r.dropped += 1,
        }
        if r.dropped > 0
            && r.dropped_logged
                .is_none_or(|at| at.elapsed() > DROPS_LOGGED_EVERY)
        {
            eprintln!(
                "[hevc] ffmpeg fell behind: dropped {} chunks of video",
                r.dropped
            );
            r.dropped = 0;
            r.dropped_logged = Some(Instant::now());
        }
        Ok(())
    }

    /// Stops the converter, e.g. for a new link
    pub fn stop(&self) {
        stop(self.running.lock().unwrap().take());
        *self.retry_at.lock().unwrap() = None;
        *self.in_decoder.lock().unwrap() = true;
    }

    fn failed(&self, r: &mut Running) -> Option<String> {
        if let Ok(Some(status)) = r.child.try_wait() {
            return Some(format!("ffmpeg exited, {status}"));
        }
        let last_out = self.last_out.load(Ordering::Relaxed);
        if last_out == 0 {
            let waited = r.irap_at.is_some_and(|at| at.elapsed() > FIRST_PICTURE);
            return waited.then(|| "no pictures came out".into());
        }
        let since = now_millis().saturating_sub(last_out);
        (since > STALLED.as_millis() as u64).then(|| "pictures stopped coming out".into())
    }

    fn start(&self, params: &ParamSets) -> Result<Running, String> {
        self.last_out.store(0, Ordering::Relaxed);
        let mut child = Command::new("ffmpeg")
            .args(ffmpeg_args(*self.in_decoder.lock().unwrap()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => "ffmpeg isn't installed".to_string(),
                _ => format!("ffmpeg: {e}"),
            })?;
        let (Some(mut stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            return Err("ffmpeg's pipes".into());
        };
        let (input, chunks) = mpsc::sync_channel::<Vec<u8>>(INPUT_QUEUE);
        let spawned = thread::Builder::new()
            .name("hevc-in".into())
            .spawn(move || {
                for chunk in chunks {
                    if stdin.write_all(&chunk).is_err() {
                        break;
                    }
                }
            })
            .and_then(|_| {
                let on_picture = Arc::clone(&self.on_picture);
                let last_out = Arc::clone(&self.last_out);
                thread::Builder::new()
                    .name("hevc-out".into())
                    .spawn(move || read_pictures(stdout, &on_picture, &last_out))
            })
            .and_then(|_| {
                thread::Builder::new()
                    .name("hevc-log".into())
                    .spawn(move || log_errors(stderr))
            });
        if let Err(e) = spawned {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e.to_string());
        }
        println!("[hevc] converting H.265 to H.264 (ffmpeg)");
        // A remote sends its parameter sets only with a keyframe; a decoder
        // started after that needs them handed over first. It then waits
        // for the next keyframe
        if let Some(params) = params.annex_b() {
            let _ = input.try_send(params);
        }
        Ok(Running {
            child,
            input,
            converter: Converter::Hardware,
            irap_at: None,
            dropped: 0,
            dropped_logged: None,
        })
    }
}

impl Drop for Transcoder {
    fn drop(&mut self) {
        stop(self.running.lock().unwrap().take());
    }
}

/// Ends ffmpeg; its threads end with its pipes
fn stop(running: Option<Running>) {
    if let Some(mut r) = running {
        let _ = r.child.kill();
        let _ = r.child.wait();
    }
}

/// The pictures ffmpeg writes, each handed over with the goggles' framing
fn read_pictures(mut stdout: ChildStdout, on_picture: &OnPicture, last_out: &AtomicU64) {
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 256 * 1024];
    loop {
        match stdout.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        if more_coming(&stdout) {
            continue;
        }
        for picture in pictures(&buf) {
            let picture = h264::strip_leading_aud(picture);
            let mut au = Vec::with_capacity(picture.len() + AUD.len());
            au.extend_from_slice(picture);
            au.extend_from_slice(&AUD);
            last_out.store(now_millis(), Ordering::Relaxed);
            on_picture(&au);
        }
        buf.clear();
    }
}

/// Whether more of what's been read is about to arrive
fn more_coming(stdout: &ChildStdout) -> bool {
    let mut fd = libc::pollfd {
        fd: stdout.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut fd, 1, PICTURE_SETTLED_MS) > 0 }
}

/// Whole pictures, each from its delimiter to the next one's
fn pictures(buf: &[u8]) -> Vec<&[u8]> {
    let mut starts: Vec<usize> = buf
        .windows(PICTURE_START.len())
        .enumerate()
        .filter(|(_, w)| *w == PICTURE_START)
        .map(|(i, _)| i)
        .collect();
    // Anything before the first delimiter belongs with the first picture
    if starts.first() != Some(&0) {
        starts.insert(0, 0);
    }
    starts.push(buf.len());
    starts
        .windows(2)
        .map(|w| &buf[w[0]..w[1]])
        .filter(|p| !p.is_empty())
        .collect()
}

/// ffmpeg's messages, e.g. a picture whose reference never came. A damaged
/// stream can bring many a second; only a few go to the log
fn log_errors(stderr: ChildStderr) {
    let mut window = Instant::now();
    let (mut logged, mut skipped) = (0u32, 0u32);
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        if window.elapsed() > LOG_WINDOW {
            if skipped > 0 {
                eprintln!("[ffmpeg] ({skipped} more)");
            }
            window = Instant::now();
            (logged, skipped) = (0, 0);
        }
        if logged < LOG_LINES {
            eprintln!("[ffmpeg] {line}");
            logged += 1;
        } else {
            skipped += 1;
        }
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1)
}

#[cfg(test)]
#[path = "hevc.test.rs"]
mod tests;
