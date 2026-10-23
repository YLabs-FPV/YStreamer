use super::*;

use gstreamer as gst;
use gstreamer::prelude::*;
use std::sync::atomic::AtomicU16;

/// Each test its own ports, away from a real OMT sender's
fn ports() -> RangeInclusive<u16> {
    static NEXT: AtomicU16 = AtomicU16::new(46400);
    let start = NEXT.fetch_add(20, Ordering::Relaxed);
    start..=start + 19
}

fn read_frame(stream: &mut TcpStream) -> (wire::Header, Vec<u8>) {
    let mut header = [0u8; wire::HEADER_LEN];
    stream.read_exact(&mut header).unwrap();
    let header = wire::parse_header(&header).unwrap();
    let mut data = vec![0; header.data_len];
    stream.read_exact(&mut data).unwrap();
    (header, data)
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn subscribe(port: u16) -> TcpStream {
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let (hello, xml) = read_frame(&mut client);
    assert_eq!(hello.kind, wire::METADATA);
    assert!(String::from_utf8(xml).unwrap().starts_with("<OMTInfo "));
    client
        .write_all(&wire::metadata(r#"<OMTSubscribe Video="true" />"#))
        .unwrap();
    client
}

#[test]
fn service_file_names_the_source() {
    let xml = service("Pilot 1", 6401);
    assert!(xml.contains("<name replace-wildcards=\"yes\">%h (Pilot 1)</name>"));
    assert!(xml.contains("<type>_omt._tcp</type>"));
    assert!(xml.contains("<port>6401</port>"));
}

#[test]
fn takes_the_next_free_port() {
    let range = ports();
    let first = Server::start(range.clone()).unwrap();
    let second = Server::start(range.clone()).unwrap();
    assert_eq!(first.port(), *range.start());
    assert_eq!(second.port(), range.start() + 1);
}

#[test]
fn sends_only_to_receivers_that_asked() {
    let server = Server::start(ports()).unwrap();
    let mut idle = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
    idle.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let _hello = read_frame(&mut idle);
    let mut watcher = subscribe(server.port());
    wait_for("the subscription", || server.watching() == 1);

    let frame = wire::video(&[1, 2, 3], 64, 32, (30, 1), 7);
    server.send(Arc::new(frame.clone()));
    let (header, data) = read_frame(&mut watcher);
    assert_eq!(header.kind, wire::VIDEO);
    assert_eq!(header.timestamp, 7);
    assert_eq!(data, frame[wire::HEADER_LEN..]);

    // Nothing reached the one that didn't ask
    idle.set_nonblocking(true).unwrap();
    assert_eq!(
        idle.read(&mut [0u8; 1]).unwrap_err().kind(),
        ErrorKind::WouldBlock
    );

    drop(watcher);
    // The departure shows once a write fails or the read side sees it
    wait_for("the receiver to leave", || {
        server.send(Arc::new(vec![0; 64]));
        server.watching() == 0
    });
}

#[test]
fn stopping_closes_the_connections() {
    let server = Server::start(ports()).unwrap();
    let mut client = subscribe(server.port());
    wait_for("the subscription", || server.watching() == 1);
    drop(server);
    let mut rest = Vec::new();
    // Closed rather than left hanging
    assert!(client.read_to_end(&mut rest).is_ok());
}

#[cfg(target_arch = "aarch64")]
#[test]
fn encodes_a_picture() {
    let (y, uv) = (vec![80u8; 64 * 32], vec![128u8; 64 * 16]);
    let picture = Picture {
        width: 64,
        height: 32,
        y: &y,
        y_stride: 64,
        uv: &uv,
        uv_stride: 64,
        frame_rate: None,
    };
    let mut encoder = vmx::Encoder::new(64, 32).unwrap();
    let first = encoder.encode(&picture).unwrap().to_vec();
    assert!(!first.is_empty());
    // Every picture stands alone, so the next is complete too
    assert!(!encoder.encode(&picture).unwrap().is_empty());
    // A picture of another size is refused, not misread
    let mut other = vmx::Encoder::new(32, 32).unwrap();
    assert_eq!(other.encode(&picture), None);
}

#[cfg(target_arch = "aarch64")]
#[test]
fn h264_in_vmx1_out() {
    gst::init().unwrap();
    for element in ["x264enc", "avdec_h264", "videotestsrc"] {
        if gst::ElementFactory::find(element).is_none() {
            eprintln!("skipping: no {element}");
            return;
        }
    }
    let pipeline = gst::parse::launch(
        "videotestsrc num-buffers=20 ! video/x-raw,width=1920,height=1080,framerate=30/1 ! \
         x264enc tune=zerolatency threads=1 key-int-max=10 ! \
         video/x-h264,stream-format=byte-stream,alignment=au ! appsink name=sink sync=false",
    )
    .unwrap()
    .dynamic_cast::<gst::Pipeline>()
    .unwrap();
    let sink = pipeline
        .by_name("sink")
        .unwrap()
        .dynamic_cast::<gstreamer_app::AppSink>()
        .unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();
    let mut aus = Vec::new();
    while let Ok(sample) = sink.pull_sample() {
        aus.push(sample.buffer().unwrap().map_readable().unwrap().to_vec());
    }
    let _ = pipeline.set_state(gst::State::Null);

    let r = Running {
        server: Arc::new(Server::start(ports()).unwrap()),
        limit: Limit {
            max_height: 720,
            max_fps: 30,
        },
        size: Arc::new(AtomicU64::new(0)),
        idle_since: None,
    };
    let mut client = subscribe(r.server.port());
    wait_for("the subscription", || r.server.watching() == 1);
    let mut decoder = Decoder::start(r.limit, encoding(&r)).unwrap();
    for au in &aus {
        decoder.push(au);
        thread::sleep(Duration::from_millis(33));
    }
    let (header, data) = read_frame(&mut client);
    assert_eq!(header.kind, wire::VIDEO);
    assert_eq!(&data[0..4], b"VMX1");
    assert_eq!(&data[4..8], &1280i32.to_le_bytes());
    assert_eq!(&data[8..12], &720i32.to_le_bytes());
    assert!(data.len() > 32);
    assert_eq!(r.size.load(Ordering::Relaxed), 1280 << 32 | 720);
}
