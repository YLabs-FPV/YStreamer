//! Fetching and installing NDI's runtime for the owner. YStreamer may not
//! hand out NDI's files, so they come straight from NDI or from the owner,
//! are kept only until the owner accepts or declines NDI's licence, and the
//! licence shown is the one inside the package

use std::ffi::OsStr;
use std::fmt::Display;
use std::fs;
use std::io::{self, BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::runtime::LIBRARIES;

const SDK_URL: &str = "https://downloads.ndi.tv/SDK/NDI_SDK_Linux/Install_NDI_SDK_v6_Linux.tar.gz";
/// The SDK is about 62 MB; this leaves room for it to grow
const MAX_BYTES: u64 = 256 << 20;
const WORK_DIR: &str = "/var/lib/ystreamer/ndi";
/// Where runtime.rs looks besides the loader's own search
const LIB_DIR: &str = "/usr/local/lib";
/// The accepted licence stays next to the library it covers
const DOC_DIR: &str = "/usr/local/share/doc/ndi";

/// The line in NDI's install script after which its archive starts
const MARKER: &[u8] = b"__NDI_ARCHIVE_BEGIN__";
/// The script's text before the archive is about 115 lines
const MAX_HEADER: u64 = 1 << 20;
const LIB_PREFIX: &str = "NDI SDK for Linux/lib/";
const LICENSE: &str = "NDI SDK for Linux/NDI SDK License Agreement.txt";
const NOTICES: &str = "NDI SDK for Linux/licenses/libndi_licenses.txt";
const MAX_LICENSE_BYTES: usize = 1 << 20;
/// Going through 60 MB of gzip takes a few seconds on a Pi 4
const TAR_TIMEOUT: Duration = Duration::from_secs(180);

const DOWNLOAD: &str = "download";
const UPLOAD: &str = "upload";
const PAYLOAD: &str = "payload.tar.gz";
const STAGED: &str = "staged.json";
const SCRATCH: &str = "extract";

const NOT_THE_SDK: &str = "That isn't NDI's SDK for Linux";

/// One download, upload or install at a time
static BUSY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// The download underway, for showing its progress
static FETCHING: std::sync::Mutex<Option<Fetching>> = std::sync::Mutex::new(None);

#[derive(Serialize, Default)]
pub struct Status {
    /// Fetched and waiting for the owner to accept NDI's licence
    staged: Option<Staged>,
    /// Being downloaded from NDI right now
    download: Option<Progress>,
}

#[derive(Serialize)]
pub struct Progress {
    received: u64,
    /// What NDI's server said the download's size is, if it did
    total: Option<u64>,
    /// Downloaded, and being checked
    checking: bool,
}

#[derive(Clone, Copy)]
struct Fetching {
    total: Option<u64>,
    checking: bool,
}

/// Shows the download as underway until dropped, however it ends
struct Shown;

impl Shown {
    fn start() -> Self {
        *FETCHING.lock().unwrap() = Some(Fetching {
            total: None,
            checking: false,
        });
        Self
    }

    fn update(&self, change: impl FnOnce(&mut Fetching)) {
        if let Some(fetching) = FETCHING.lock().unwrap().as_mut() {
            change(fetching);
        }
    }
}

impl Drop for Shown {
    fn drop(&mut self) {
        *FETCHING.lock().unwrap() = None;
    }
}

#[derive(Serialize)]
pub struct Staged {
    version: String,
    license: String,
}

/// What's kept between fetching the SDK and the owner's answer, so it
/// survives a restart
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Saved {
    version: String,
    /// The library inside the payload
    member: String,
    license: String,
    /// The payload has NDI's third-party notices
    notices: bool,
}

/// Separate so tests never touch the system's directories
struct Paths {
    work: PathBuf,
    lib: PathBuf,
    doc: PathBuf,
}

impl Paths {
    fn system() -> Self {
        Self {
            work: WORK_DIR.into(),
            lib: LIB_DIR.into(),
            doc: DOC_DIR.into(),
        }
    }

    fn work(&self, name: &str) -> PathBuf {
        self.work.join(name)
    }
}

fn busy() -> Result<tokio::sync::MutexGuard<'static, ()>, String> {
    BUSY.try_lock()
        .map_err(|_| "NDI's runtime is already being downloaded or installed".into())
}

pub fn status() -> Status {
    status_in(&Paths::system())
}

fn status_in(paths: &Paths) -> Status {
    Status {
        staged: load(paths).map(|saved| Staged {
            version: saved.version,
            license: saved.license,
        }),
        download: FETCHING.lock().unwrap().map(|fetching| Progress {
            // curl writes straight into the file, so its size is how far
            // along it is
            received: fs::metadata(paths.work(DOWNLOAD)).map_or(0, |m| m.len()),
            total: fetching.total,
            checking: fetching.checking,
        }),
    }
}

/// Fetch the SDK from NDI and get it ready for the owner to accept
pub async fn download() -> Result<Status, String> {
    let _busy = busy()?;
    let shown = Shown::start();
    let paths = Paths::system();
    fresh(&paths)?;
    let total = size().await;
    shown.update(|f| f.total = total);
    let file = paths.work(DOWNLOAD);
    let result = match fetch(&file).await {
        Ok(()) => {
            shown.update(|f| f.checking = true);
            stage(&paths, &file).await
        }
        Err(e) => Err(e),
    };
    finish(&paths, result)
}

/// Take the SDK the owner downloaded themselves, for a device without
/// internet. Written as it arrives: some Pis have 1 GB of memory
pub async fn upload<S, E>(mut body: S) -> Result<Status, String>
where
    S: Stream<Item = Result<bytes::Bytes, E>> + Unpin,
    E: Display,
{
    let _busy = busy()?;
    let paths = Paths::system();
    fresh(&paths)?;
    let file = paths.work(UPLOAD);
    let result = async {
        let mut out = tokio::fs::File::create(&file)
            .await
            .map_err(|e| format!("Couldn't store the file: {e}"))?;
        let mut size = 0u64;
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|e| format!("The upload didn't finish: {e}"))?;
            size += chunk.len() as u64;
            if size > MAX_BYTES {
                return Err("That file is too big to be NDI's SDK".to_string());
            }
            out.write_all(&chunk)
                .await
                .map_err(|e| format!("Couldn't store the file: {e}"))?;
        }
        out.flush()
            .await
            .map_err(|e| format!("Couldn't store the file: {e}"))?;
        drop(out);
        stage(&paths, &file).await
    }
    .await;
    finish(&paths, result)
}

/// The owner declined NDI's licence: nothing of NDI's stays behind
pub async fn discard() -> Result<Status, String> {
    let _busy = busy()?;
    let paths = Paths::system();
    clear(&paths)?;
    Ok(status_in(&paths))
}

/// Only ever called once the owner has accepted NDI's licence
pub async fn install() -> Result<(), String> {
    let _busy = busy()?;
    let paths = Paths::system();
    let version = install_files(&paths).await?;
    // runtime.rs also looks in /usr/local/lib directly, so the runtime
    // works even if the loader's cache couldn't be updated
    if let Err(e) = crate::net::network::run("ldconfig", &[]).await {
        eprintln!("[ndi] ldconfig failed: {e}");
    }
    clear(&paths)?;
    println!("[ndi] runtime {version} installed after the owner accepted NDI's licence");
    Ok(())
}

/// An empty working directory, so nothing from an earlier try gets mixed in
fn fresh(paths: &Paths) -> Result<(), String> {
    clear(paths)?;
    fs::create_dir_all(&paths.work)
        .map_err(|e| format!("Couldn't create {}: {e}", paths.work.display()))
}

fn clear(paths: &Paths) -> Result<(), String> {
    match fs::remove_dir_all(&paths.work) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            Err(format!("Couldn't delete {}: {e}", paths.work.display()))
        }
        _ => Ok(()),
    }
}

/// Nothing half-done is kept when fetching or checking fails
fn finish(paths: &Paths, result: Result<(), String>) -> Result<Status, String> {
    if let Err(e) = result {
        let _ = clear(paths);
        return Err(e);
    }
    Ok(status_in(paths))
}

/// The download's size, asked for first so its progress can be shown.
/// Without it the download still goes ahead
async fn size() -> Option<u64> {
    let out = Command::new("curl")
        .args(["--fail", "--silent", "--head", "--location"])
        .args(["--proto", "=https", "--proto-redir", "=https"])
        .args(["--connect-timeout", "15", "--max-time", "30"])
        .arg(SDK_URL)
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    out.status
        .success()
        .then(|| content_length(&String::from_utf8_lossy(&out.stdout)))
        .flatten()
}

/// From the headers of the last response, after any redirects
fn content_length(headers: &str) -> Option<u64> {
    let last = headers
        .lines()
        .rev()
        .take_while(|line| !line.starts_with("HTTP/"));
    last.filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
}

async fn fetch(into: &Path) -> Result<(), String> {
    let out = Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--proto", "=https", "--proto-redir", "=https"])
        .args(["--connect-timeout", "15", "--max-time", "900"])
        .args(["--max-filesize", &MAX_BYTES.to_string()])
        .arg("--output")
        .arg(into)
        .arg(SDK_URL)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("Couldn't run curl: {e}"))?;
    if !out.status.success() {
        let reason = String::from_utf8_lossy(&out.stderr);
        let reason = reason.trim().trim_start_matches("curl: ");
        return Err(format!("Couldn't download NDI's SDK: {reason}"));
    }
    Ok(())
}

/// Check what was fetched is NDI's SDK with a runtime for this device, and
/// keep only the archive inside it and its licence. Takes either NDI's
/// download as it comes or the install script inside it, and deletes it
async fn stage(paths: &Paths, file: &Path) -> Result<(), String> {
    let mut start = [0u8; 2];
    let read = fs::File::open(file).and_then(|mut f| f.read_exact(&mut start));
    let script = if read.is_err() {
        return Err(NOT_THE_SDK.into());
    } else if start == [0x1f, 0x8b] {
        let script = unwrap_download(paths, file).await;
        let _ = fs::remove_file(file);
        script?
    } else if &start == b"#!" {
        file.to_path_buf()
    } else {
        return Err(NOT_THE_SDK.into());
    };

    let payload = paths.work(PAYLOAD);
    let split = {
        let (script, payload) = (script.clone(), payload.clone());
        tokio::task::spawn_blocking(move || split(&script, &payload))
            .await
            .map_err(|e| e.to_string())
    };
    let _ = fs::remove_file(&script);
    split??;

    let listing = tar(["-tzf".as_ref(), payload.as_os_str()])
        .await
        .map_err(|_| NOT_THE_SDK.to_string())?;
    let listing = String::from_utf8_lossy(&listing);
    let (member, version) = pick_library(&listing, std::env::consts::ARCH)?;
    if !listing.lines().any(|l| l == LICENSE) {
        return Err("NDI's SDK came without its licence, so it can't be installed".into());
    }
    let license = tar([
        "-xzOf".as_ref(),
        payload.as_os_str(),
        "--".as_ref(),
        LICENSE.as_ref(),
    ])
    .await
    .map_err(|_| "Couldn't read NDI's licence from the SDK".to_string())?;
    if license.len() > MAX_LICENSE_BYTES || license.iter().all(u8::is_ascii_whitespace) {
        return Err("Couldn't read NDI's licence from the SDK".into());
    }
    save(
        paths,
        &Saved {
            version,
            member,
            license: String::from_utf8_lossy(&license).into_owned(),
            notices: listing.lines().any(|l| l == NOTICES),
        },
    )
}

/// The install script from NDI's download, which holds nothing else
async fn unwrap_download(paths: &Paths, file: &Path) -> Result<PathBuf, String> {
    let listing = tar(["-tzf".as_ref(), file.as_os_str()])
        .await
        .map_err(|_| NOT_THE_SDK.to_string())?;
    let listing = String::from_utf8_lossy(&listing);
    let mut scripts = listing.lines().filter(|name| is_installer(name));
    let (Some(name), None) = (scripts.next(), scripts.next()) else {
        return Err(NOT_THE_SDK.into());
    };
    tar([
        "-xzf".as_ref(),
        file.as_os_str(),
        "-C".as_ref(),
        paths.work.as_os_str(),
        "--no-same-owner".as_ref(),
        "--no-wildcards".as_ref(),
        "--".as_ref(),
        name.as_ref(),
    ])
    .await
    .map_err(|_| NOT_THE_SDK.to_string())?;
    let script = paths.work(name);
    if !fs::symlink_metadata(&script).is_ok_and(|m| m.is_file()) {
        return Err(NOT_THE_SDK.into());
    }
    Ok(script)
}

/// `Install_NDI_SDK_v6_Linux.sh`, at the top of the download
fn is_installer(name: &str) -> bool {
    let Some(version) = name
        .strip_prefix("Install_NDI_SDK_v")
        .and_then(|rest| rest.strip_suffix("_Linux.sh"))
    else {
        return false;
    };
    !version.is_empty()
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.')
        && !name.contains('/')
        && !name.contains("..")
}

/// Copy out the archive the script would unpack, rather than running it
fn split(script: &Path, payload: &Path) -> Result<(), String> {
    let mut file = fs::File::open(script).map_err(|e| format!("Couldn't read the SDK: {e}"))?;
    let offset = payload_offset(io::BufReader::new(&mut file)).ok_or(NOT_THE_SDK)?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("Couldn't read the SDK: {e}"))?;
    let mut out = fs::File::create(payload).map_err(|e| format!("Couldn't store the SDK: {e}"))?;
    let copied =
        io::copy(&mut file, &mut out).map_err(|e| format!("Couldn't store the SDK: {e}"))?;
    if copied == 0 {
        return Err(NOT_THE_SDK.into());
    }
    Ok(())
}

/// Where the archive starts: just after the marker's line, which comes
/// within the script's first megabyte
fn payload_offset(reader: impl BufRead) -> Option<u64> {
    let mut reader = reader.take(MAX_HEADER);
    let mut line = Vec::new();
    let mut offset = 0u64;
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line).ok()?;
        if n == 0 || line.last() != Some(&b'\n') {
            return None;
        }
        offset += n as u64;
        let text = line.strip_suffix(b"\n").unwrap_or(&line);
        let text = text.strip_suffix(b"\r").unwrap_or(text);
        if text == MARKER {
            return Some(offset);
        }
    }
}

/// The runtime built for `arch`, and its version. The SDK has one per
/// kind of processor, under `lib/<target>/libndi.so.<version>`
fn pick_library(listing: &str, arch: &str) -> Result<(String, String), String> {
    let wanted = |target: &str| match arch {
        "aarch64" => target.starts_with("aarch64-"),
        "x86_64" => target == "x86_64-linux-gnu",
        _ => false,
    };
    let mut found: Option<(String, String, u32)> = None;
    for member in listing.lines() {
        let Some((target, name)) = member
            .strip_prefix(LIB_PREFIX)
            .and_then(|rest| rest.split_once('/'))
        else {
            continue;
        };
        if !wanted(target) || name.contains('/') || member.contains("..") {
            continue;
        }
        let Some((version, major)) = parse_version(name) else {
            continue;
        };
        // The fullest version, over any shorter names linking to it
        if found
            .as_ref()
            .is_none_or(|(_, v, _)| version.len() > v.len())
        {
            found = Some((member.to_string(), version, major));
        }
    }
    let (member, version, major) =
        found.ok_or(format!("NDI's SDK has no runtime for {arch} devices"))?;
    if !LIBRARIES.contains(&format!("libndi.so.{major}").as_str()) {
        let usable = LIBRARIES
            .map(|l| l.trim_start_matches("libndi.so."))
            .join(" and ");
        return Err(format!(
            "This is NDI's runtime {version}, and YStreamer works with versions {usable}"
        ));
    }
    Ok((member, version))
}

/// `libndi.so.6.3.2` is version 6.3.2, major 6
fn parse_version(file_name: &str) -> Option<(String, u32)> {
    let version = file_name.strip_prefix("libndi.so.")?;
    if version
        .split('.')
        .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let major = version.split('.').next()?.parse().ok()?;
    Some((version.to_string(), major))
}

/// Put the accepted runtime where runtime.rs finds it, with NDI's licence
/// beside it. Returns its version
async fn install_files(paths: &Paths) -> Result<String, String> {
    let saved = load(paths).ok_or("Download NDI's runtime first")?;
    let name = Path::new(&saved.member)
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or("Couldn't install NDI's runtime: the package is damaged")?;
    let (_, major) =
        parse_version(name).ok_or("Couldn't install NDI's runtime: the package is damaged")?;

    let scratch = paths.work(SCRATCH);
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).map_err(|e| format!("Couldn't unpack NDI's runtime: {e}"))?;
    let payload = paths.work(PAYLOAD);
    let mut args: Vec<&OsStr> = vec![
        "-xzf".as_ref(),
        payload.as_os_str(),
        "-C".as_ref(),
        scratch.as_os_str(),
        "--no-same-owner".as_ref(),
        "--no-wildcards".as_ref(),
        "--".as_ref(),
        saved.member.as_ref(),
    ];
    if saved.notices {
        args.push(NOTICES.as_ref());
    }
    tar(args)
        .await
        .map_err(|e| format!("Couldn't unpack NDI's runtime: {e}"))?;
    let library = scratch.join(&saved.member);
    // A link here could point anywhere on the system
    if !fs::symlink_metadata(&library).is_ok_and(|m| m.is_file()) {
        return Err("Couldn't install NDI's runtime: the package is damaged".into());
    }

    let failed = |e: io::Error| format!("Couldn't install NDI's runtime: {e}");
    fs::create_dir_all(&paths.lib).map_err(failed)?;
    // Swapped in whole, so nothing ever opens a half-copied library
    let partial = paths.lib.join(format!(".{name}.ystreamer"));
    fs::copy(&library, &partial).map_err(failed)?;
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o755)).map_err(failed)?;
    }
    fs::rename(&partial, paths.lib.join(name)).map_err(failed)?;
    // What runtime.rs asks for, as NDI's own install script would link it
    let link = format!("libndi.so.{major}");
    let partial = paths.lib.join(format!(".{link}.ystreamer"));
    let _ = fs::remove_file(&partial);
    std::os::unix::fs::symlink(name, &partial).map_err(failed)?;
    fs::rename(&partial, paths.lib.join(&link)).map_err(failed)?;

    let failed = |e: io::Error| format!("Couldn't keep NDI's licence: {e}");
    fs::create_dir_all(&paths.doc).map_err(failed)?;
    fs::write(
        paths.doc.join("NDI SDK License Agreement.txt"),
        &saved.license,
    )
    .map_err(failed)?;
    let notices = scratch.join(NOTICES);
    if saved.notices && fs::symlink_metadata(&notices).is_ok_and(|m| m.is_file()) {
        fs::copy(&notices, paths.doc.join("libndi_licenses.txt")).map_err(failed)?;
    }
    Ok(saved.version)
}

fn save(paths: &Paths, saved: &Saved) -> Result<(), String> {
    let json = serde_json::to_vec(saved).map_err(|e| e.to_string())?;
    let partial = paths.work(&format!("{STAGED}.partial"));
    fs::write(&partial, json)
        .and_then(|()| fs::rename(&partial, paths.work(STAGED)))
        .map_err(|e| format!("Couldn't store the SDK: {e}"))
}

fn load(paths: &Paths) -> Option<Saved> {
    if !paths.work(PAYLOAD).is_file() {
        return None;
    }
    serde_json::from_slice(&fs::read(paths.work(STAGED)).ok()?).ok()
}

/// The system's tar with a longer time limit than `run`'s
async fn tar<I, S>(args: I) -> Result<Vec<u8>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let fut = Command::new("tar").args(args).kill_on_drop(true).output();
    let out = match tokio::time::timeout(TAR_TIMEOUT, fut).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("tar: {e}")),
        Err(_) => return Err("tar timed out".into()),
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(stderr
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .unwrap_or("tar failed")
            .to_string());
    }
    Ok(out.stdout)
}

#[cfg(test)]
#[path = "install.test.rs"]
mod tests;
