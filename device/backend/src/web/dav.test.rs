use super::*;
use std::fs;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::settings::RecordingSettings;

const CLIP: &[u8] = b"0123456789";

/// A recordings folder, with a file beside it that must stay out of reach
struct Place {
    root: PathBuf,
    share: PathBuf,
}

impl Place {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("ystreamer-dav-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let share = root.join("share");
        fs::create_dir_all(share.join("inner")).unwrap();
        fs::write(share.join("a.mp4"), CLIP).unwrap();
        fs::write(share.join("b & c.ts"), b"ts").unwrap();
        fs::write(share.join(".hidden.mp4"), b"no").unwrap();
        fs::write(share.join("notes.txt"), b"no").unwrap();
        fs::write(share.join("inner/deep.mp4"), b"no").unwrap();
        fs::write(root.join("outside.mp4"), b"secret").unwrap();
        std::os::unix::fs::symlink(root.join("outside.mp4"), share.join("link.mp4")).unwrap();
        Self { root, share }
    }

    fn drive(&self, on: bool) -> Drive {
        let cfg = RecordingSettings {
            path: self.share.display().to_string(),
            webdav: on,
            ..Default::default()
        };
        Drive {
            recorder: Arc::new(Recorder::new(cfg)),
            auth: Auth::load(self.root.join("auth.json")),
        }
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

async fn listen(drive: Drive) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app: Router = router(drive);
    tokio::spawn(async move { axum::serve(listener, app).await });
    address
}

struct Reply {
    status: u16,
    /// Lowercase names
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// A request as a client sends it, path and all, without anything tidying
/// the path on the way
async fn send(address: SocketAddr, method: &str, path: &str, extra: &[(&str, &str)]) -> Reply {
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: device\r\nConnection: close\r\n");
    for (name, value) in extra {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("Content-Length: 0\r\n\r\n");
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    socket.read_to_end(&mut raw).await.unwrap();

    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Reply {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    }
}

#[tokio::test]
async fn lists_the_recordings_for_a_propfind() {
    let place = Place::new();
    let address = listen(place.drive(true)).await;
    let reply = send(address, "PROPFIND", "/dav/", &[("Depth", "1")]).await;
    assert_eq!(reply.status, 207);
    assert_eq!(
        reply.header("content-type"),
        Some("application/xml; charset=\"utf-8\"")
    );

    let xml = reply.text();
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"));
    assert!(xml.contains("<D:multistatus xmlns:D=\"DAV:\">"));
    assert!(xml.contains("<D:href>/dav/</D:href>"));
    assert!(xml.contains("<D:resourcetype><D:collection/></D:resourcetype>"));
    assert!(xml.contains("<D:href>/dav/a.mp4</D:href>"));
    assert!(xml.contains("<D:getcontentlength>10</D:getcontentlength>"));
    assert!(xml.contains("<D:getcontenttype>video/mp4</D:getcontenttype>"));
    // Escaped in the name, encoded in the address
    assert!(xml.contains("<D:href>/dav/b%20%26%20c.ts</D:href>"));
    assert!(xml.contains("<D:displayname>b &amp; c.ts</D:displayname>"));
    // Finder won't show a folder whose dates it can't read
    assert_eq!(xml.matches("<D:getlastmodified>").count(), 3);
    assert!(xml.contains(" GMT</D:getlastmodified>"));

    for kept_out in [
        "link.mp4",
        "outside",
        ".hidden",
        "notes.txt",
        "inner",
        "deep",
    ] {
        assert!(!xml.contains(kept_out), "{kept_out} is listed");
    }
}

#[tokio::test]
async fn depth_zero_is_the_folder_alone() {
    let place = Place::new();
    let address = listen(place.drive(true)).await;
    for path in ["/dav", "/dav/"] {
        let reply = send(address, "PROPFIND", path, &[("Depth", "0")]).await;
        assert_eq!(reply.status, 207, "{path}");
        let xml = reply.text();
        assert_eq!(xml.matches("<D:response>").count(), 1, "{path}");
        assert!(xml.contains("<D:collection/>"));
        assert!(xml.contains("<D:quota-available-bytes>"), "{path}");
        assert!(xml.contains("<D:quota-used-bytes>"), "{path}");
    }
}

#[test]
fn reports_the_drive_size() {
    assert_eq!(
        quota(1000, 300),
        "<D:quota-available-bytes>300</D:quota-available-bytes>\
         <D:quota-used-bytes>700</D:quota-used-bytes></D:prop>"
    );
}

#[tokio::test]
async fn propfind_on_a_file_describes_it() {
    let place = Place::new();
    let address = listen(place.drive(true)).await;
    let reply = send(address, "PROPFIND", "/dav/a.mp4", &[("Depth", "0")]).await;
    assert_eq!(reply.status, 207);
    let xml = reply.text();
    assert_eq!(xml.matches("<D:response>").count(), 1);
    assert!(xml.contains("<D:href>/dav/a.mp4</D:href>"));
    assert!(xml.contains("<D:resourcetype/>"));

    let missing = send(address, "PROPFIND", "/dav/gone.mp4", &[]).await;
    assert_eq!(missing.status, 404);
}

#[tokio::test]
async fn options_says_it_speaks_webdav_before_any_login() {
    let place = Place::new();
    let drive = place.drive(true);
    drive
        .auth
        .set_password(Some("hunter22".into()))
        .await
        .unwrap();
    let address = listen(drive).await;
    for path in ["/dav", "/dav/", "/dav/a.mp4"] {
        let reply = send(address, "OPTIONS", path, &[]).await;
        assert_eq!(reply.status, 200, "{path}");
        assert_eq!(reply.header("dav"), Some("1"));
        assert_eq!(reply.header("ms-author-via"), Some("DAV"));
        assert!(reply.header("allow").unwrap().contains("PROPFIND"));
    }
}

#[tokio::test]
async fn serves_whole_files_and_ranges() {
    let place = Place::new();
    let address = listen(place.drive(true)).await;

    let whole = send(address, "GET", "/dav/a.mp4", &[]).await;
    assert_eq!(whole.status, 200);
    assert_eq!(whole.body, CLIP);
    assert_eq!(whole.header("content-type"), Some("video/mp4"));
    assert_eq!(whole.header("accept-ranges"), Some("bytes"));
    assert!(whole.header("last-modified").unwrap().ends_with(" GMT"));
    assert!(whole.header("etag").is_some());

    let part = send(address, "GET", "/dav/a.mp4", &[("Range", "bytes=2-5")]).await;
    assert_eq!(part.status, 206);
    assert_eq!(part.body, b"2345");
    assert_eq!(part.header("content-range"), Some("bytes 2-5/10"));

    let tail = send(address, "GET", "/dav/a.mp4", &[("Range", "bytes=-3")]).await;
    assert_eq!(tail.status, 206);
    assert_eq!(tail.body, b"789");

    let head = send(address, "HEAD", "/dav/a.mp4", &[]).await;
    assert_eq!(head.status, 200);
    assert_eq!(head.header("content-length"), Some("10"));
    assert!(head.body.is_empty());

    let encoded = send(address, "GET", "/dav/b%20%26%20c.ts", &[]).await;
    assert_eq!(encoded.status, 200);
    assert_eq!(encoded.body, b"ts");
    assert_eq!(encoded.header("content-type"), Some("video/mp2t"));
}

#[tokio::test]
async fn nothing_outside_the_folder_is_reachable() {
    let place = Place::new();
    let address = listen(place.drive(true)).await;
    for path in [
        "/dav/../outside.mp4",
        "/dav/..",
        "/dav/%2e%2e",
        "/dav/%2e%2e/outside.mp4",
        "/dav/%2e%2e%2foutside.mp4",
        "/dav/%2E%2E%2Foutside.mp4",
        "/dav/..%2foutside.mp4",
        "/dav/..%5coutside.mp4",
        "/dav/inner/deep.mp4",
        "/dav/inner%2fdeep.mp4",
        "/dav/inner%2Fdeep.mp4",
        "/dav/%2fetc%2fpasswd",
        "/dav/link.mp4",
        "/dav/.hidden.mp4",
        "/dav/notes.txt",
        "/dav/a.mp4%00",
        "/dav/a.mp4/",
        "/dav/%zz.mp4",
        "/dav/%ff.mp4",
    ] {
        for method in ["GET", "HEAD", "PROPFIND"] {
            let reply = send(address, method, path, &[]).await;
            assert_eq!(reply.status, 404, "{method} {path}");
            assert!(!reply.text().contains("secret"), "{method} {path}");
        }
    }
}

#[tokio::test]
async fn it_is_read_only() {
    let place = Place::new();
    let address = listen(place.drive(true)).await;
    for method in [
        "PUT",
        "DELETE",
        "MKCOL",
        "COPY",
        "MOVE",
        "PROPPATCH",
        "LOCK",
        "UNLOCK",
        "POST",
    ] {
        let reply = send(address, method, "/dav/a.mp4", &[]).await;
        assert_eq!(reply.status, 405, "{method}");
        assert_eq!(reply.header("allow"), Some(ALLOW));
    }
    assert_eq!(fs::read(place.share.join("a.mp4")).unwrap(), CLIP);
}

#[tokio::test]
async fn is_not_there_while_turned_off() {
    let place = Place::new();
    let address = listen(place.drive(false)).await;
    for method in ["OPTIONS", "PROPFIND", "GET"] {
        for path in ["/dav/", "/dav/a.mp4"] {
            let reply = send(address, method, path, &[]).await;
            assert_eq!(reply.status, 404, "{method} {path}");
        }
    }
}

#[tokio::test]
async fn asks_for_the_admin_password_once_one_is_set() {
    use base64::Engine;
    let basic = |user_and_password: &str| {
        let encoded = base64::engine::general_purpose::STANDARD.encode(user_and_password);
        format!("Basic {encoded}")
    };

    let place = Place::new();
    let drive = place.drive(true);
    // Without a password, as everywhere else, nothing is asked
    let open = listen(drive.clone()).await;
    assert_eq!(send(open, "PROPFIND", "/dav/", &[]).await.status, 207);

    drive
        .auth
        .set_password(Some("hunter22".into()))
        .await
        .unwrap();
    let address = listen(drive).await;
    let refused = send(address, "PROPFIND", "/dav/", &[]).await;
    assert_eq!(refused.status, 401);
    assert!(
        refused
            .header("www-authenticate")
            .unwrap()
            .starts_with("Basic realm=")
    );

    let wrong = basic("admin:hunter23");
    let reply = send(address, "GET", "/dav/a.mp4", &[("Authorization", &wrong)]).await;
    assert_eq!(reply.status, 401);
    assert!(reply.body.is_empty());

    // Any user name, as clients make one up or ask for it
    for user in ["admin", "", "DOMAIN\\someone"] {
        let right = basic(&format!("{user}:hunter22"));
        let reply = send(address, "GET", "/dav/a.mp4", &[("Authorization", &right)]).await;
        assert_eq!(reply.status, 200, "{user}");
        assert_eq!(reply.body, CLIP);
    }
}

#[test]
fn reads_targets() {
    assert_eq!(target("/dav"), Some(Target::Folder));
    assert_eq!(target("/dav/"), Some(Target::Folder));
    assert_eq!(target("/dav/a.mp4"), Some(Target::File("a.mp4".into())));
    assert_eq!(
        target("/dav/b%20%26%20c.ts"),
        Some(Target::File("b & c.ts".into()))
    );
    for refused in [
        "/davx",
        "/dav//",
        "/dav/a/b.mp4",
        "/dav/a%2fb.mp4",
        "/dav/%2e%2e",
        "/dav/..",
        "/dav/.",
        "/dav/._a.mp4",
        "/dav/.DS_Store",
        "/dav/a%5cb.mp4",
        "/dav/a%00.mp4",
        "/dav/a%0a.mp4",
        "/dav/%",
        "/dav/%4",
        "/dav/%c3%28.mp4",
        "/other/a.mp4",
    ] {
        assert_eq!(target(refused), None, "{refused}");
    }
}

#[test]
fn encodes_and_decodes_names() {
    let name = "ystreamer 20250101 #1 ü.mp4";
    let encoded = percent_encode(name);
    assert!(encoded.bytes().all(|b| b.is_ascii_graphic()));
    assert!(!encoded.contains(['/', ' ', '#']));
    assert_eq!(percent_decode(&encoded).as_deref(), Some(name));
    assert_eq!(percent_decode("a%2Fb").as_deref(), Some("a/b"));
    assert_eq!(percent_decode("%e2%82"), None);
    assert_eq!(percent_decode("%+1"), None);
}

#[test]
fn writes_dates_as_clients_read_them() {
    assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
    assert_eq!(http_date(1_700_000_000), "Tue, 14 Nov 2023 22:13:20 GMT");
    assert_eq!(iso_date(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(iso_date(4_102_444_799), "2099-12-31T23:59:59Z");
}

#[test]
fn keeps_the_mdns_record_in_step() {
    let place = Place::new();
    let dir = place.root.join("services");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join(AVAHI_FILE);

    assert!(write_service(&dir, true, 80).unwrap());
    let xml = fs::read_to_string(&file).unwrap();
    assert!(xml.contains("<type>_webdav._tcp</type>"));
    assert!(xml.contains("<port>80</port>"));
    assert!(xml.contains("<txt-record>path=/dav/</txt-record>"));
    assert!(xml.contains("<name replace-wildcards=\"yes\">%h recordings</name>"));

    // Left alone when right, so avahi doesn't announce it again
    assert!(!write_service(&dir, true, 80).unwrap());
    assert!(write_service(&dir, true, 8080).unwrap());
    assert!(
        fs::read_to_string(&file)
            .unwrap()
            .contains("<port>8080</port>")
    );

    assert!(write_service(&dir, false, 8080).unwrap());
    assert!(!file.exists());
    assert!(!write_service(&dir, false, 8080).unwrap());
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
}
