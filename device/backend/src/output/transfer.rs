use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Serialize;

use super::recording;

const CHUNK_BYTES: usize = 1 << 20;
/// Written out to the drive this often, so the progress is what's really
/// on it: otherwise the whole file lands in memory at once and the last
/// sync takes as long as the copy, with the bar already full
const SYNC_BYTES: u64 = 16 << 20;
/// The largest file FAT32 can hold
const FAT_MAX_FILE_BYTES: u64 = u32::MAX as u64;
/// A copy carries this until it's whole, so a pulled drive or a power cut
/// never leaves half a recording under a recording's name
const UNFINISHED: &str = "part";

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Serialize, Clone, Debug)]
pub struct Progress {
    pub from: String,
    pub to: String,
    pub moving: bool,
    pub state: State,
    pub files_total: u32,
    pub files_done: u32,
    /// Already there, so not copied again
    pub skipped: u32,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub current: Option<String>,
    pub error: Option<String>,
}

/// What a transfer will do, worked out and checked before anything is written
#[derive(Debug)]
pub struct Plan {
    from: PathBuf,
    to: PathBuf,
    moving: bool,
    items: Vec<Item>,
    skipped: u32,
}

#[derive(Debug, PartialEq)]
struct Item {
    name: String,
    size: u64,
    /// None when the destination has this recording already
    copy_as: Option<String>,
}

/// Where the copies are going, as far as planning needs to know
pub struct Destination<'a> {
    pub dir: &'a Path,
    pub fstype: &'a str,
    pub free_bytes: u64,
}

/// `names` of None means every recording the destination doesn't have yet.
/// `busy` says whether a file is still being recorded
pub fn plan(
    from: &Path,
    to: &Destination,
    names: Option<&[String]>,
    moving: bool,
    busy: &dyn Fn(&str) -> bool,
) -> Result<Plan, String> {
    if from == to.dir {
        return Err("Those are the same place".into());
    }
    let everything = names.is_none();
    let names: Vec<String> = match names {
        Some(names) => names.to_vec(),
        None => recording::list(from).into_iter().map(|f| f.name).collect(),
    };

    let mut items = Vec::new();
    let mut skipped = 0;
    let mut taken: Vec<String> = Vec::new();
    for name in names {
        let source = recording::resolve(from, &name)?;
        if busy(&name) {
            // Asked for by name it's a mistake worth saying; swept up with
            // the rest it's just not ready yet
            if everything {
                continue;
            }
            return Err(format!("{name} is still being recorded"));
        }
        let size = fs::metadata(&source)
            .map_err(|e| format!("Couldn't read {name}: {e}"))?
            .len();

        let there = |candidate: &str| fs::metadata(to.dir.join(candidate)).ok().map(|m| m.len());
        let copy_as = if there(&name) == Some(size) {
            skipped += 1;
            // Nothing to copy, but a move still has the original to remove
            if !moving {
                continue;
            }
            None
        } else {
            let fat = matches!(to.fstype, "vfat" | "msdos");
            if fat && size > FAT_MAX_FILE_BYTES {
                return Err(format!(
                    "{name} is larger than 4 GB, more than a FAT32 drive can hold in one file. \
                     A drive formatted as exFAT can take it"
                ));
            }
            // Same name, different recording: neither is overwritten
            let free = unused_name(&name, |c| {
                there(c).is_some() || taken.iter().any(|t| t == c)
            });
            taken.push(free.clone());
            Some(free)
        };
        items.push(Item {
            name,
            size,
            copy_as,
        });
    }

    let needed: u64 = items
        .iter()
        .filter(|i| i.copy_as.is_some())
        .map(|i| i.size)
        .sum();
    if needed > to.free_bytes {
        return Err(format!(
            "Not enough room: {} to copy, {} free",
            human(needed),
            human(to.free_bytes)
        ));
    }
    Ok(Plan {
        from: from.to_path_buf(),
        to: to.dir.to_path_buf(),
        moving,
        items,
        skipped,
    })
}

/// `name`, or `name (2)`, `name (3)`... with the extension kept
fn unused_name(name: &str, used: impl Fn(&str) -> bool) -> String {
    if !used(name) {
        return name.to_string();
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (2..)
        .map(|n| format!("{stem} ({n}).{ext}"))
        .find(|candidate| !used(candidate))
        .expect("an unused name exists")
}

fn human(bytes: u64) -> String {
    const GB: f64 = (1u64 << 30) as f64;
    const MB: f64 = (1u64 << 20) as f64;
    if bytes as f64 >= GB {
        format!("{:.1} GB", bytes as f64 / GB)
    } else {
        format!("{:.0} MB", (bytes as f64 / MB).max(1.0))
    }
}

/// One transfer at a time, running on its own thread whether or not a
/// browser is watching
#[derive(Default)]
pub struct Transfers {
    progress: Mutex<Option<Progress>>,
    cancel: AtomicBool,
}

impl Transfers {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn progress(&self) -> Option<Progress> {
        self.progress.lock().unwrap().clone()
    }

    pub fn running(&self) -> bool {
        self.progress().is_some_and(|p| p.state == State::Running)
    }

    pub fn start(self: &Arc<Self>, plan: Plan) -> Result<(), String> {
        let mut progress = self.progress.lock().unwrap();
        if progress.as_ref().is_some_and(|p| p.state == State::Running) {
            return Err("A transfer is already running".into());
        }
        let copies = plan.items.iter().filter(|i| i.copy_as.is_some());
        *progress = Some(Progress {
            from: plan.from.display().to_string(),
            to: plan.to.display().to_string(),
            moving: plan.moving,
            state: State::Running,
            files_total: plan.items.len() as u32,
            files_done: 0,
            skipped: plan.skipped,
            bytes_total: copies.map(|i| i.size).sum(),
            bytes_done: 0,
            current: None,
            error: None,
        });
        drop(progress);

        self.cancel.store(false, Ordering::SeqCst);
        let this = Arc::clone(self);
        thread::spawn(move || {
            yield_to_everything_else();
            let outcome = this.run(&plan);
            let mut progress = this.progress.lock().unwrap();
            if let Some(p) = progress.as_mut() {
                p.current = None;
                match outcome {
                    Ok(()) => p.state = State::Done,
                    Err(Stopped::Cancelled) => p.state = State::Cancelled,
                    Err(Stopped::Failed(why)) => {
                        eprintln!("[transfer] {why}");
                        p.state = State::Failed;
                        p.error = Some(why);
                    }
                }
            }
        });
        Ok(())
    }

    /// Stops a running transfer after the chunk in hand; clears a finished one
    pub fn cancel(&self) {
        if self.running() {
            self.cancel.store(true, Ordering::SeqCst);
        } else {
            *self.progress.lock().unwrap() = None;
        }
    }

    fn update(&self, change: impl FnOnce(&mut Progress)) {
        if let Some(p) = self.progress.lock().unwrap().as_mut() {
            change(p);
        }
    }

    fn run(&self, plan: &Plan) -> Result<(), Stopped> {
        fs::create_dir_all(&plan.to).map_err(|e| failed("Couldn't open the destination", e))?;
        clear_unfinished(&plan.to);

        for item in &plan.items {
            self.update(|p| p.current = Some(item.name.clone()));
            let source = plan.from.join(&item.name);
            if let Some(copy_as) = &item.copy_as {
                self.copy(&source, &plan.to.join(copy_as), item.size)?;
            }
            if plan.moving {
                fs::remove_file(&source).map_err(|e| {
                    failed(
                        &format!("Copied {}, but couldn't remove the original", item.name),
                        e,
                    )
                })?;
            }
            self.update(|p| p.files_done += 1);
        }
        // Past this the drive can be pulled
        flush(&plan.to);
        Ok(())
    }

    fn copy(&self, source: &Path, dest: &Path, size: u64) -> Result<(), Stopped> {
        let name = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let unfinished = unfinished_path(dest);
        let result = (|| {
            let mut input =
                File::open(source).map_err(|e| failed(&format!("Couldn't read {name}"), e))?;
            let mut output = File::create(&unfinished)
                .map_err(|e| failed(&format!("Couldn't write {name}"), e))?;
            let mut chunk = vec![0u8; CHUNK_BYTES];
            let mut copied = 0u64;
            let mut unsynced = 0u64;
            loop {
                if self.cancel.load(Ordering::SeqCst) {
                    return Err(Stopped::Cancelled);
                }
                let n = input
                    .read(&mut chunk)
                    .map_err(|e| failed(&format!("Couldn't read {name}"), e))?;
                if n == 0 {
                    break;
                }
                output
                    .write_all(&chunk[..n])
                    .map_err(|e| failed(&format!("Couldn't write {name}"), e))?;
                copied += n as u64;
                unsynced += n as u64;
                if unsynced >= SYNC_BYTES {
                    output
                        .sync_data()
                        .map_err(|e| failed(&format!("Couldn't write {name}"), e))?;
                    self.update(|p| p.bytes_done += unsynced);
                    unsynced = 0;
                }
            }
            output
                .sync_all()
                .map_err(|e| failed(&format!("Couldn't finish writing {name}"), e))?;
            self.update(|p| p.bytes_done += unsynced);
            if copied != size {
                return Err(Stopped::Failed(format!(
                    "{name} changed while it was being copied"
                )));
            }
            fs::rename(&unfinished, dest).map_err(|e| failed(&format!("Couldn't finish {name}"), e))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&unfinished);
        }
        result
    }
}

#[derive(Debug)]
enum Stopped {
    Cancelled,
    Failed(String),
}

fn failed(what: &str, e: std::io::Error) -> Stopped {
    Stopped::Failed(format!("{what}: {e}"))
}

fn unfinished_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(UNFINISHED);
    dest.with_file_name(name)
}

/// What an earlier transfer left when it was cut short
pub fn clear_unfinished(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == UNFINISHED) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Everything written to the filesystem `dir` is on, onto the drive itself
fn flush(dir: &Path) {
    use std::os::fd::AsRawFd;
    if let Ok(dir) = File::open(dir) {
        unsafe { libc::syncfs(dir.as_raw_fd()) };
    }
}

/// Copying competes with a recording for the same card, and the recording
/// can't wait: this thread only gets the disk when nothing else wants it
fn yield_to_everything_else() {
    const WHO_THREAD: libc::c_int = 1;
    const IDLE_CLASS: libc::c_int = 3 << 13;
    unsafe { libc::syscall(libc::SYS_ioprio_set, WHO_THREAD, 0, IDLE_CLASS) };
}

#[cfg(test)]
#[path = "transfer.test.rs"]
mod tests;
