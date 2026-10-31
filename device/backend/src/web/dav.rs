//! The recordings folder as a read-only network drive (WebDAV class 1), for
//! Finder, Explorer, Linux file managers and file apps on phones

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::SystemTime;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;

use crate::output::recording::{self, Recorder};
use crate::web::auth::{Access, Auth};

pub const PREFIX: &str = "/dav";
/// Class 1 only: without locking, Finder mounts the drive read-only
const DAV_CLASS: &str = "1";
const ALLOW: &str = "OPTIONS, GET, HEAD, PROPFIND";
const REALM: &str = "Basic realm=\"YStreamer recordings\", charset=\"UTF-8\"";
const AVAHI_SERVICES: &str = "/etc/avahi/services";
const AVAHI_FILE: &str = "ystreamer-webdav.service";

/// The web server's port, for the mDNS record
static PORT: AtomicU16 = AtomicU16::new(80);

#[derive(Clone)]
pub struct Drive {
    pub recorder: Arc<Recorder>,
    pub auth: Arc<Auth>,
}

pub fn router<S>(drive: Drive) -> Router<S> {
    Router::new()
        .route(PREFIX, any(handle))
        .route("/dav/", any(handle))
        .route("/dav/{*rest}", any(handle))
        .with_state(drive)
}

/// What a path under `/dav` names
#[derive(Debug, PartialEq)]
pub(crate) enum Target {
    Folder,
    File(String),
}

/// Only the folder itself and plain names directly in it. Decoding comes
/// after splitting, so an encoded `/` can't reach another folder, and names
/// starting with a dot are hidden, which also covers `..` and the `._` files
/// Finder asks for
pub(crate) fn target(path: &str) -> Option<Target> {
    let rest = path.strip_prefix(PREFIX)?;
    let rest = match rest {
        "" | "/" => return Some(Target::Folder),
        rest => rest.strip_prefix('/')?,
    };
    if rest.contains('/') {
        return None;
    }
    let name = percent_decode(rest)?;
    let bad = name.is_empty()
        || name.starts_with('.')
        || name.contains(['/', '\\', '\0'])
        || name.chars().any(char::is_control);
    (!bad).then_some(Target::File(name))
}

async fn handle(State(drive): State<Drive>, req: Request) -> Response {
    let Some(dir) = drive.recorder.shared_dir() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // Clients ask before they log in, and the answer gives nothing away
    if req.method() == Method::OPTIONS {
        return options();
    }
    let headers = req.headers();
    if drive.auth.access(headers) < Access::Admin && !drive.auth.check_basic(headers).await {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, REALM)],
        )
            .into_response();
    }
    let Some(target) = target(req.uri().path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let method = req.method().as_str();
    match (method, target) {
        ("PROPFIND", Target::Folder) => {
            let shallow = headers
                .get("depth")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|d| d.trim() == "0");
            multistatus(folder_listing(&dir, !shallow))
        }
        ("PROPFIND", Target::File(name)) => match entry(&dir, &name) {
            Some(file) => multistatus(listing(&[file], None)),
            None => StatusCode::NOT_FOUND.into_response(),
        },
        ("GET" | "HEAD", Target::Folder) => index_page(&dir),
        ("GET" | "HEAD", Target::File(name)) => serve(&dir, &name, headers, method == "HEAD").await,
        _ => (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, ALLOW)]).into_response(),
    }
}

/// Also answered at `/`, where Windows asks first
pub fn options() -> Response {
    (
        [
            (header::ALLOW, ALLOW),
            (header::HeaderName::from_static("dav"), DAV_CLASS),
            (header::HeaderName::from_static("ms-author-via"), "DAV"),
            (header::CONTENT_LENGTH, "0"),
        ],
        Body::empty(),
    )
        .into_response()
}

/// A recording as listed
pub(crate) struct Entry {
    name: String,
    size: u64,
    modified: u64,
}

/// Regular files only: a link in the folder could point anywhere
fn regular(path: &Path) -> Option<std::fs::Metadata> {
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|m| m.file_type().is_file())
}

fn entry(dir: &Path, name: &str) -> Option<Entry> {
    let path = recording::resolve(dir, name).ok()?;
    let meta = regular(&path)?;
    Some(Entry {
        name: name.to_string(),
        size: meta.len(),
        modified: unix_secs(meta.modified().ok()),
    })
}

fn entries(dir: &Path) -> Vec<Entry> {
    recording::list(dir)
        .into_iter()
        // The same names `target` lets through
        .filter(|f| !f.name.starts_with('.') && regular(&dir.join(&f.name)).is_some())
        .map(|f| Entry {
            name: f.name,
            size: f.size_bytes,
            modified: f.modified,
        })
        .collect()
}

fn folder_listing(dir: &Path, with_files: bool) -> String {
    let modified = unix_secs(std::fs::metadata(dir).and_then(|m| m.modified()).ok());
    let files = if with_files { entries(dir) } else { Vec::new() };
    listing(&files, Some(modified))
}

/// A `multistatus` body: the folder first when `folder` carries its time,
/// then the files. Every property is sent whatever was asked for, which
/// all the clients accept
pub(crate) fn listing(files: &[Entry], folder: Option<u64>) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\">\n",
    );
    if let Some(modified) = folder {
        let _ = writeln!(
            xml,
            "<D:response><D:href>{PREFIX}/</D:href><D:propstat><D:prop>\
             <D:displayname>Recordings</D:displayname>\
             <D:resourcetype><D:collection/></D:resourcetype>\
             <D:creationdate>{}</D:creationdate>\
             <D:getlastmodified>{}</D:getlastmodified>\
             </D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
            iso_date(modified),
            http_date(modified),
        );
    }
    for f in files {
        let _ = writeln!(
            xml,
            "<D:response><D:href>{PREFIX}/{}</D:href><D:propstat><D:prop>\
             <D:displayname>{}</D:displayname>\
             <D:resourcetype/>\
             <D:getcontentlength>{}</D:getcontentlength>\
             <D:getcontenttype>{}</D:getcontenttype>\
             <D:getetag>{}</D:getetag>\
             <D:creationdate>{}</D:creationdate>\
             <D:getlastmodified>{}</D:getlastmodified>\
             </D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
            percent_encode(&f.name),
            xml_escape(&f.name),
            f.size,
            mime(&f.name),
            xml_escape(&etag(f.size, f.modified)),
            iso_date(f.modified),
            http_date(f.modified),
        );
    }
    xml.push_str("</D:multistatus>\n");
    xml
}

fn multistatus(xml: String) -> Response {
    (
        StatusCode::MULTI_STATUS,
        [(header::CONTENT_TYPE, "application/xml; charset=\"utf-8\"")],
        xml,
    )
        .into_response()
}

/// For a browser opening the address
fn index_page(dir: &Path) -> Response {
    let mut html = String::from(
        "<!doctype html><meta charset=\"utf-8\"><title>Recordings</title><h1>Recordings</h1><ul>",
    );
    for f in entries(dir) {
        let _ = write!(
            html,
            "<li><a href=\"{PREFIX}/{}\">{}</a></li>",
            percent_encode(&f.name),
            xml_escape(&f.name)
        );
    }
    html.push_str("</ul>");
    axum::response::Html(html).into_response()
}

async fn serve(dir: &Path, name: &str, headers: &HeaderMap, head_only: bool) -> Response {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let Some(path) = recording::resolve(dir, name).ok() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // O_NOFOLLOW, so a link swapped in after the check above isn't followed
    let Ok(mut file) = tokio::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .await
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(meta) = file.metadata().await.ok().filter(|m| m.is_file()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let total = meta.len();
    let modified = unix_secs(meta.modified().ok());

    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| crate::web::api::parse_range(v, total));
    let (status, start, len) = match range {
        Some((start, end)) => (StatusCode::PARTIAL_CONTENT, start, end - start + 1),
        None => (StatusCode::OK, 0, total),
    };

    let mut out = HeaderMap::new();
    let mut set = |key: header::HeaderName, value: String| {
        if let Ok(v) = HeaderValue::from_str(&value) {
            out.insert(key, v);
        }
    };
    set(header::CONTENT_TYPE, mime(name).to_string());
    set(header::ACCEPT_RANGES, "bytes".into());
    set(header::CONTENT_LENGTH, len.to_string());
    set(header::LAST_MODIFIED, http_date(modified));
    set(header::ETAG, etag(total, modified));
    if let Some((start, end)) = range {
        set(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{total}"),
        );
    }

    if head_only {
        return (status, out, Body::empty()).into_response();
    }
    if start > 0 && file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file.take(len)));
    (status, out, body).into_response()
}

fn mime(name: &str) -> &'static str {
    if name.to_ascii_lowercase().ends_with(".mp4") {
        "video/mp4"
    } else {
        "video/mp2t"
    }
}

/// Changes whenever the file grows, as the one being recorded does
fn etag(size: u64, modified: u64) -> String {
    format!("\"{size:x}-{modified:x}\"")
}

fn unix_secs(t: Option<SystemTime>) -> u64 {
    t.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// Year, month, day, hour, minute, second and weekday (0 = Sunday) in UTC
fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32, usize) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's days-to-civil
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    let weekday = (days + 4).rem_euclid(7) as usize;
    (
        year,
        month,
        day,
        (rem / 3600) as u32,
        (rem / 60 % 60) as u32,
        (rem % 60) as u32,
        weekday,
    )
}

/// RFC 1123, which `getlastmodified` must be in for Finder
pub(crate) fn http_date(secs: u64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let (y, mo, d, h, mi, s, wd) = civil(secs);
    format!(
        "{}, {d:02} {} {y} {h:02}:{mi:02}:{s:02} GMT",
        DAYS[wd],
        MONTHS[mo as usize - 1]
    )
}

/// RFC 3339, for `creationdate`
pub(crate) fn iso_date(secs: u64) -> String {
    let (y, mo, d, h, mi, s, _) = civil(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

pub(crate) fn percent_encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// `None` for a broken escape or bytes that aren't UTF-8
pub(crate) fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Remember the web server's port and put the mDNS record in step with the
/// setting
pub fn start(port: u16, on: bool) {
    PORT.store(port, Ordering::Relaxed);
    advertise(on);
}

/// Announce the drive over mDNS as `_webdav._tcp`, through a service file
/// avahi-daemon picks up by itself, or take it back
pub fn advertise(on: bool) {
    let dir = Path::new(AVAHI_SERVICES);
    // No avahi, no announcement; the address still works
    if !dir.is_dir() {
        return;
    }
    match write_service(dir, on, PORT.load(Ordering::Relaxed)) {
        Ok(true) if on => eprintln!("[dav] announcing the network drive over mDNS"),
        Ok(_) => {}
        Err(e) => eprintln!(
            "[dav] couldn't update {}: {e}",
            dir.join(AVAHI_FILE).display()
        ),
    }
}

/// Whether the file changed. It's left alone when it's already right, as
/// avahi re-announces on every change
pub(crate) fn write_service(dir: &Path, on: bool, port: u16) -> std::io::Result<bool> {
    let path: PathBuf = dir.join(AVAHI_FILE);
    if !on {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        };
    }
    let xml = service_file(port);
    if std::fs::read_to_string(&path).is_ok_and(|old| old == xml) {
        return Ok(false);
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, xml)?;
    std::fs::rename(&tmp, &path)?;
    Ok(true)
}

/// `%h` is the device name, kept up to date by avahi. `path` is what Linux
/// file managers open
pub(crate) fn service_file(port: u16) -> String {
    format!(
        "<?xml version=\"1.0\" standalone='no'?>\n\
         <!DOCTYPE service-group SYSTEM \"avahi-service.dtd\">\n\
         <service-group>\n  \
           <name replace-wildcards=\"yes\">%h recordings</name>\n  \
           <service>\n    \
             <type>_webdav._tcp</type>\n    \
             <port>{port}</port>\n    \
             <txt-record>path={PREFIX}/</txt-record>\n  \
           </service>\n\
         </service-group>\n"
    )
}

#[cfg(test)]
#[path = "dav.test.rs"]
mod tests;
