use super::*;

use std::process::Command as Shell;

/// Where this machine's runtime sits in the SDK
fn this_target() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "aarch64-rpi4-linux-gnueabi",
        _ => "x86_64-linux-gnu",
    }
}

struct Fixture {
    root: PathBuf,
    paths: Paths,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("ystreamer-ndi-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let paths = Paths {
            work: root.join("work"),
            lib: root.join("lib"),
            doc: root.join("doc"),
        };
        fs::create_dir_all(&paths.work).unwrap();
        Self { root, paths }
    }

    /// A small stand-in for the SDK's archive, with the files given
    fn payload(&self, files: &[(&str, &[u8])]) -> Vec<u8> {
        let src = self.root.join("src");
        let _ = fs::remove_dir_all(&src);
        for (name, data) in files {
            let path = src.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        }
        let out = self.root.join("payload.tar.gz");
        let ok = Shell::new("tar")
            .arg("-czf")
            .arg(&out)
            .arg("-C")
            .arg(&src)
            .arg("NDI SDK for Linux")
            .status()
            .unwrap();
        assert!(ok.success());
        fs::read(out).unwrap()
    }

    /// NDI's install script: its text, the marker, then the archive
    fn script(&self, payload: &[u8]) -> PathBuf {
        let mut data = b"#!/bin/bash\n\
            ARCHIVE=`awk '/^__NDI_ARCHIVE_BEGIN__/ { print NR+1; exit 0; }' \"$0\"`\n\
            tail -n+$ARCHIVE \"$0\" | tar xvz\n\
            exit 0\n\
            __NDI_ARCHIVE_BEGIN__\n"
            .to_vec();
        data.extend_from_slice(payload);
        let path = self.root.join("Install_NDI_SDK_v6_Linux.sh");
        fs::write(&path, data).unwrap();
        path
    }

    /// NDI's download: the script alone in a .tar.gz
    fn download(&self, payload: &[u8]) -> PathBuf {
        self.script(payload);
        let out = self.root.join("download.tar.gz");
        let ok = Shell::new("tar")
            .arg("-czf")
            .arg(&out)
            .arg("-C")
            .arg(&self.root)
            .arg("Install_NDI_SDK_v6_Linux.sh")
            .status()
            .unwrap();
        assert!(ok.success());
        out
    }

    /// Into the working directory, where uploads and downloads land
    fn incoming(&self, from: &Path) -> PathBuf {
        let to = self.paths.work(UPLOAD);
        fs::copy(from, &to).unwrap();
        to
    }

    fn sdk(&self) -> Vec<u8> {
        self.payload(&[
            (LICENSE, b"NDI's licence, for the test"),
            (NOTICES, b"Third-party notices"),
            (
                &format!("{LIB_PREFIX}{}/libndi.so.6.0.1", this_target()),
                b"\x7fELF not really",
            ),
            (
                &format!("{LIB_PREFIX}i686-linux-gnu/libndi.so.6.0.1"),
                b"other",
            ),
        ])
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn finds_the_archive_after_the_marker_line() {
    let script = b"#!/bin/sh\nawk '/^__NDI_ARCHIVE_BEGIN__/'\n__NDI_ARCHIVE_BEGIN__\n\x1f\x8bdata";
    let offset = payload_offset(&script[..]).unwrap();
    assert_eq!(&script[offset as usize..], b"\x1f\x8bdata");
}

#[test]
fn marker_with_windows_line_ending_counts() {
    let script = b"#!/bin/sh\r\n__NDI_ARCHIVE_BEGIN__\r\nrest";
    assert_eq!(payload_offset(&script[..]), Some(script.len() as u64 - 4));
}

#[test]
fn no_marker_no_archive() {
    assert_eq!(payload_offset(&b"#!/bin/sh\necho hi\n"[..]), None);
    // Mentioned but not on a line of its own
    assert_eq!(
        payload_offset(&b"#!/bin/sh\n__NDI_ARCHIVE_BEGIN__ x\n"[..]),
        None
    );
    // Without the line ending there's no archive after it
    assert_eq!(
        payload_offset(&b"#!/bin/sh\n__NDI_ARCHIVE_BEGIN__"[..]),
        None
    );
}

#[test]
fn marker_only_looked_for_near_the_start() {
    let mut script = b"#!/bin/sh\n".to_vec();
    script.extend(std::iter::repeat_n(b'#', MAX_HEADER as usize));
    script.extend_from_slice(b"\n__NDI_ARCHIVE_BEGIN__\n");
    assert_eq!(payload_offset(&script[..]), None);
}

const LISTING: &str = "NDI SDK for Linux/
NDI SDK for Linux/NDI SDK License Agreement.txt
NDI SDK for Linux/lib/
NDI SDK for Linux/lib/i686-linux-gnu/libndi.so.6.3.2
NDI SDK for Linux/lib/arm-rpi4-linux-gnueabihf/libndi.so.6.3.2
NDI SDK for Linux/lib/x86_64-linux-gnu/libndi.so.6.3.2
NDI SDK for Linux/lib/aarch64-rpi4-linux-gnueabi/
NDI SDK for Linux/lib/aarch64-rpi4-linux-gnueabi/libndi.so.6.3.2
NDI SDK for Linux/licenses/libndi_licenses.txt
";

#[test]
fn picks_the_runtime_for_the_processor() {
    assert_eq!(
        pick_library(LISTING, "aarch64"),
        Ok((
            "NDI SDK for Linux/lib/aarch64-rpi4-linux-gnueabi/libndi.so.6.3.2".into(),
            "6.3.2".into()
        ))
    );
    assert_eq!(
        pick_library(LISTING, "x86_64"),
        Ok((
            "NDI SDK for Linux/lib/x86_64-linux-gnu/libndi.so.6.3.2".into(),
            "6.3.2".into()
        ))
    );
}

#[test]
fn no_runtime_for_other_processors() {
    let err = pick_library(LISTING, "riscv64").unwrap_err();
    assert!(err.contains("riscv64"), "{err}");
    let only_32_bit = "NDI SDK for Linux/lib/arm-rpi4-linux-gnueabihf/libndi.so.6.3.2\n";
    assert!(pick_library(only_32_bit, "aarch64").is_err());
}

#[test]
fn prefers_the_full_version_over_links() {
    let listing = "NDI SDK for Linux/lib/x86_64-linux-gnu/libndi.so.6
NDI SDK for Linux/lib/x86_64-linux-gnu/libndi.so.6.3.2
NDI SDK for Linux/lib/x86_64-linux-gnu/libndi.so
";
    assert_eq!(pick_library(listing, "x86_64").unwrap().1, "6.3.2");
}

#[test]
fn refuses_runtimes_ystreamer_cant_open() {
    let listing = "NDI SDK for Linux/lib/x86_64-linux-gnu/libndi.so.7.0.0\n";
    let err = pick_library(listing, "x86_64").unwrap_err();
    assert!(err.contains("7.0.0") && err.contains("6 and 5"), "{err}");
    let old = "NDI SDK for Linux/lib/x86_64-linux-gnu/libndi.so.5.6.1\n";
    assert_eq!(pick_library(old, "x86_64").unwrap().1, "5.6.1");
}

#[test]
fn ignores_paths_that_leave_the_sdk() {
    let listing = "NDI SDK for Linux/lib/x86_64-linux-gnu/../../../libndi.so.6.0.0
NDI SDK for Linux/lib/x86_64-linux-gnu/sub/libndi.so.6.0.0
";
    assert!(pick_library(listing, "x86_64").is_err());
}

#[test]
fn reads_the_version_from_the_file_name() {
    assert_eq!(parse_version("libndi.so.6.3.2"), Some(("6.3.2".into(), 6)));
    assert_eq!(parse_version("libndi.so.5"), Some(("5".into(), 5)));
    assert_eq!(parse_version("libndi.so"), None);
    assert_eq!(parse_version("libndi.so."), None);
    assert_eq!(parse_version("libndi.so.6..2"), None);
    assert_eq!(parse_version("libndi.so.6.3.2.bak"), None);
    assert_eq!(parse_version("libother.so.6"), None);
}

#[test]
fn size_comes_from_the_last_response() {
    let headers = "HTTP/1.1 301 Moved Permanently\r
Location: https://example.com/sdk.tar.gz\r
Content-Length: 162\r
\r
HTTP/2 200 \r
content-type: application/x-gzip\r
content-length: 61792217\r
\r
";
    assert_eq!(content_length(headers), Some(61792217));
    // A redirect's own size isn't the download's
    let no_size = "HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\nHTTP/2 200\r\n\r\n";
    assert_eq!(content_length(no_size), None);
    assert_eq!(content_length(""), None);
}

#[test]
fn knows_the_install_script_by_name() {
    assert!(is_installer("Install_NDI_SDK_v6_Linux.sh"));
    assert!(is_installer("Install_NDI_SDK_v6.1_Linux.sh"));
    assert!(!is_installer("Install_NDI_SDK__Linux.sh"));
    assert!(!is_installer("dir/Install_NDI_SDK_v6_Linux.sh"));
    assert!(!is_installer("./Install_NDI_SDK_v6_Linux.sh"));
    assert!(!is_installer("Install_NDI_SDK_v../x_Linux.sh"));
    assert!(!is_installer("Install_NDI_SDK_v6_Linux.sh.txt"));
    assert!(!is_installer("Install_NDI_Advanced_SDK_v6_Linux.sh"));
}

#[tokio::test]
async fn stages_ndis_download() {
    let f = Fixture::new("download");
    let download = f.download(&f.sdk());
    let incoming = f.incoming(&download);
    stage(&f.paths, &incoming).await.unwrap();

    let staged = status_in(&f.paths).staged.unwrap();
    assert_eq!(staged.version, "6.0.1");
    assert_eq!(staged.license, "NDI's licence, for the test");
    // Only the archive and what's needed to install it are kept
    let mut kept: Vec<_> = fs::read_dir(&f.paths.work)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    kept.sort();
    assert_eq!(kept, [PAYLOAD, STAGED]);
}

#[tokio::test]
async fn stages_the_bare_script() {
    let f = Fixture::new("script");
    let script = f.script(&f.sdk());
    let incoming = f.incoming(&script);
    stage(&f.paths, &incoming).await.unwrap();
    assert_eq!(status_in(&f.paths).staged.unwrap().version, "6.0.1");
    assert!(!incoming.exists());
}

#[tokio::test]
async fn refuses_an_sdk_without_its_licence() {
    let f = Fixture::new("unlicensed");
    let payload = f.payload(&[(
        &format!("{LIB_PREFIX}{}/libndi.so.6.0.1", this_target()),
        b"lib",
    )]);
    let incoming = f.incoming(&f.download(&payload));
    let err = stage(&f.paths, &incoming).await.unwrap_err();
    assert!(err.contains("licence"), "{err}");
    assert!(status_in(&f.paths).staged.is_none());
}

#[tokio::test]
async fn refuses_an_sdk_without_this_processors_runtime() {
    let f = Fixture::new("other-arch");
    let payload = f.payload(&[
        (LICENSE, b"licence"),
        (
            &format!("{LIB_PREFIX}i686-linux-gnu/libndi.so.6.0.1"),
            b"lib",
        ),
    ]);
    let incoming = f.incoming(&f.download(&payload));
    let err = stage(&f.paths, &incoming).await.unwrap_err();
    assert!(err.contains("no runtime"), "{err}");
}

#[tokio::test]
async fn refuses_other_files() {
    let f = Fixture::new("junk");
    for junk in [
        &b"PNG and so on"[..],
        b"",
        // A script, but not NDI's
        b"#!/bin/sh\necho hello\n",
        // A gzip, but not a tar
        b"\x1f\x8b\x08\x00garbage",
    ] {
        let file = f.paths.work(UPLOAD);
        fs::write(&file, junk).unwrap();
        assert_eq!(stage(&f.paths, &file).await, Err(NOT_THE_SDK.into()));
    }
    // A real .tar.gz without the install script in it
    let other = f.root.join("other.tar.gz");
    fs::write(f.root.join("readme"), "hi").unwrap();
    assert!(
        Shell::new("tar")
            .arg("-czf")
            .arg(&other)
            .arg("-C")
            .arg(&f.root)
            .arg("readme")
            .status()
            .unwrap()
            .success()
    );
    let incoming = f.incoming(&other);
    assert_eq!(stage(&f.paths, &incoming).await, Err(NOT_THE_SDK.into()));
}

#[tokio::test]
async fn installs_the_accepted_runtime() {
    let f = Fixture::new("install");
    let incoming = f.incoming(&f.download(&f.sdk()));
    stage(&f.paths, &incoming).await.unwrap();
    // An older version already there gets replaced
    fs::create_dir_all(&f.paths.lib).unwrap();
    fs::write(f.paths.lib.join("libndi.so.6.0.0"), "old").unwrap();
    std::os::unix::fs::symlink("libndi.so.6.0.0", f.paths.lib.join("libndi.so.6")).unwrap();

    assert_eq!(install_files(&f.paths).await, Ok("6.0.1".into()));

    let library = f.paths.lib.join("libndi.so.6.0.1");
    assert_eq!(fs::read(&library).unwrap(), b"\x7fELF not really");
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&library).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
    assert_eq!(
        fs::read_link(f.paths.lib.join("libndi.so.6")).unwrap(),
        Path::new("libndi.so.6.0.1")
    );
    assert_eq!(
        fs::read_to_string(f.paths.doc.join("NDI SDK License Agreement.txt")).unwrap(),
        "NDI's licence, for the test"
    );
    assert_eq!(
        fs::read_to_string(f.paths.doc.join("libndi_licenses.txt")).unwrap(),
        "Third-party notices"
    );
    // No temporary names left behind
    let mut names: Vec<_> = fs::read_dir(&f.paths.lib)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["libndi.so.6", "libndi.so.6.0.0", "libndi.so.6.0.1"]);
}

#[tokio::test]
async fn nothing_to_install_until_staged() {
    let f = Fixture::new("unstaged");
    assert!(install_files(&f.paths).await.is_err());
    assert!(!f.paths.lib.exists());
}

#[test]
fn declining_leaves_nothing() {
    let f = Fixture::new("decline");
    fs::write(f.paths.work(PAYLOAD), "x").unwrap();
    clear(&f.paths).unwrap();
    assert!(!f.paths.work.exists());
    // Already gone is fine
    clear(&f.paths).unwrap();
}

/// Stages the real SDK when a test points at one, e.g.
/// `NDI_SDK=~/Install_NDI_SDK_v6_Linux.tar.gz cargo test ndi::install`;
/// skips everywhere else, so tests never download it
#[tokio::test]
async fn stages_the_real_sdk() {
    let Some(sdk) = std::env::var_os("NDI_SDK") else {
        eprintln!("skipping: NDI_SDK isn't set");
        return;
    };
    let f = Fixture::new("real");
    let incoming = f.incoming(Path::new(&sdk));
    stage(&f.paths, &incoming).await.unwrap();
    let staged = status_in(&f.paths).staged.unwrap();
    assert!(staged.version.starts_with('6'), "{}", staged.version);
    assert!(staged.license.contains("NDI"));
}
