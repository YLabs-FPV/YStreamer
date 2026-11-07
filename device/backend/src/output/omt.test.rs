use super::*;

use gstreamer as gst;
use gstreamer::prelude::*;
use std::io::ErrorKind;
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

/// Connects and sends each of `requests`, as libomtnet's receiver does
fn connect_with(port: u16, requests: &[&str]) -> TcpStream {
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let (hello, xml) = read_frame(&mut client);
    assert_eq!(hello.kind, wire::METADATA);
    assert!(String::from_utf8(xml).unwrap().starts_with("<OMTInfo "));
    for xml in requests {
        client.write_all(&wire::metadata(xml)).unwrap();
    }
    client
}

fn subscribe(port: u16) -> TcpStream {
    connect_with(port, &[r#"<OMTSubscribe Video="true" />"#])
}

/// A receiver's second connection
fn subscribe_audio(port: u16) -> TcpStream {
    connect_with(port, &[r#"<OMTSubscribe Audio="true" />"#])
}

/// Both of a receiver's connections, as vMix opens them
fn mixer(port: u16) -> (TcpStream, TcpStream) {
    let video = connect_with(
        port,
        &[
            r#"<OMTSubscribe Metadata="true" />"#,
            r#"<OMTSubscribe Video="true" />"#,
            r#"<OMTSettings Preview="false" />"#,
        ],
    );
    (video, subscribe_audio(port))
}

/// Nothing has arrived, and the connection is still open
fn quiet(stream: &mut TcpStream) {
    stream.set_nonblocking(true).unwrap();
    assert_eq!(
        stream.read(&mut [0u8; 1]).unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    stream.set_nonblocking(false).unwrap();
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
    quiet(&mut idle);

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

fn open_connections(server: &Server) -> usize {
    let mut connections = server.connections.lock().unwrap();
    connections.retain(|c| c.flags.alive.load(Ordering::Relaxed));
    connections.len()
}

/// Read until the server closes, failing if it doesn't within the timeout
fn closed(stream: &mut TcpStream) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut rest = Vec::new();
    match stream.read_to_end(&mut rest) {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::ConnectionReset => {}
        Err(e) => panic!("still open: {e}"),
    }
    rest
}

#[test]
fn turns_away_connections_over_the_cap() {
    let server = Server::start(ports()).unwrap();
    let mut held: Vec<_> = (0..MAX_RECEIVERS).map(|_| mixer(server.port())).collect();
    let full = MAX_RECEIVERS + MAX_COMPANIONS;
    wait_for("the receivers", || {
        server.watching() == MAX_RECEIVERS as u32 && subscribed(&server) == full
    });

    // Closed at once, before the sender says hello
    let mut extra = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
    assert!(closed(&mut extra).is_empty());
    assert_eq!(open_connections(&server), full);

    // A place frees up when a receiver leaves
    drop(held.pop());
    wait_for("the receiver to leave", || open_connections(&server) < full);
    let _next = mixer(server.port());
    wait_for("the next receiver", || {
        server.watching() == MAX_RECEIVERS as u32
    });
}

fn subscribed(server: &Server) -> usize {
    let mut connections = server.connections.lock().unwrap();
    sweep(&mut connections);
    connections.iter().filter(|c| c.subscribed()).count()
}

#[test]
fn a_receiver_over_the_cap_is_closed_once_it_asks_for_video() {
    let server = Server::start(ports()).unwrap();
    let _held: Vec<_> = (0..MAX_RECEIVERS)
        .map(|_| subscribe(server.port()))
        .collect();
    wait_for("the receivers", || {
        server.watching() == MAX_RECEIVERS as u32
    });

    // Let in, as it might have been for audio
    let started = Instant::now();
    let mut extra = subscribe(server.port());
    closed(&mut extra);
    assert!(started.elapsed() < SUBSCRIBE_TIMEOUT);
    assert_eq!(server.watching(), MAX_RECEIVERS as u32);

    // Audio has places of its own
    let mut audio = subscribe_audio(server.port());
    wait_for("the audio subscription", || {
        subscribed(&server) == MAX_RECEIVERS + 1
    });
    quiet(&mut audio);
}

#[test]
fn an_audio_subscriber_stays_without_getting_video() {
    let server = Server::start(ports()).unwrap();
    let mut audio = subscribe_audio(server.port());
    wait_for("the audio subscription", || subscribed(&server) == 1);
    // Not a receiver, so nothing gets encoded for it
    assert_eq!(server.watching(), 0);

    // Past the deadline, still open
    thread::sleep(SUBSCRIBE_TIMEOUT + Duration::from_secs(1));
    assert_eq!(open_connections(&server), 1);
    assert_eq!(server.watching(), 0);

    // A receiver's two connections: only the video one gets pictures
    let (mut video, mut companion) = mixer(server.port());
    wait_for("the receiver", || subscribed(&server) == 3);
    assert_eq!(server.watching(), 1);
    let frame = wire::video(&[1, 2, 3], 64, 32, (30, 1), 7);
    server.send(Arc::new(frame.clone()));
    let (header, data) = read_frame(&mut video);
    assert_eq!(header.kind, wire::VIDEO);
    assert_eq!(data, frame[wire::HEADER_LEN..]);
    quiet(&mut audio);
    quiet(&mut companion);
}

#[test]
fn junk_doesnt_count_as_subscribing() {
    let server = Server::start(ports()).unwrap();
    let mut client = connect_with(
        server.port(),
        &[
            r#"<OMTSettings Preview="true" />"#,
            r#"<OMTSubscribe Video="false" />"#,
            "<OMTSubscribe Video=\"true\"/>",
            "not xml",
        ],
    );
    let started = Instant::now();
    closed(&mut client);
    let took = started.elapsed();
    assert!(took >= SUBSCRIBE_TIMEOUT - Duration::from_millis(500));
    assert!(
        took < SUBSCRIBE_TIMEOUT + Duration::from_secs(2),
        "took {took:?}"
    );
    wait_for("the connection to go", || open_connections(&server) == 0);
}

/// Connected and past the sender's hello, without subscribing
fn pending(port: u16) -> TcpStream {
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let _hello = read_frame(&mut client);
    client
}

#[test]
fn unsubscribed_connections_cant_keep_receivers_out() {
    let server = Server::start(ports()).unwrap();
    let mut first = pending(server.port());
    let mut rest: Vec<_> = (1..MAX_PENDING).map(|_| pending(server.port())).collect();
    wait_for("the connections", || {
        open_connections(&server) == MAX_PENDING
    });

    // One more pushes out the oldest rather than being turned away
    let started = Instant::now();
    rest.push(pending(server.port()));
    closed(&mut first);
    assert!(started.elapsed() < SUBSCRIBE_TIMEOUT);

    // Receivers still get in alongside them
    let _receivers: Vec<_> = (0..MAX_RECEIVERS)
        .map(|_| subscribe(server.port()))
        .collect();
    wait_for("the receivers", || {
        server.watching() == MAX_RECEIVERS as u32
    });
}

#[test]
fn a_slow_drip_doesnt_buy_more_time() {
    let server = Server::start(ports()).unwrap();
    let mut client = pending(server.port());
    let started = Instant::now();
    let mut drip = client.try_clone().unwrap();
    // Half a header, a byte at a time, so every read sees fresh data
    let dripping = thread::spawn(move || {
        while drip.write_all(&[1]).is_ok() {
            thread::sleep(Duration::from_millis(500));
        }
    });
    closed(&mut client);
    let took = started.elapsed();
    assert!(took >= SUBSCRIBE_TIMEOUT - Duration::from_millis(500));
    assert!(
        took < SUBSCRIBE_TIMEOUT + Duration::from_secs(2),
        "took {took:?}"
    );
    wait_for("the connection to go", || open_connections(&server) == 0);
    let _ = dripping.join();
}

#[test]
fn drops_a_header_that_claims_too_much() {
    let server = Server::start(ports()).unwrap();
    let mut client = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let _hello = read_frame(&mut client);
    let mut frame = wire::metadata("");
    frame[12..16].copy_from_slice(&(wire::MAX_INCOMING as i32 + 1).to_le_bytes());
    client.write_all(&frame).unwrap();
    closed(&mut client);
    wait_for("the connection to go", || open_connections(&server) == 0);
}

#[test]
fn drops_a_client_that_never_subscribes() {
    let server = Server::start(ports()).unwrap();
    let mut client = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let _hello = read_frame(&mut client);
    let started = Instant::now();
    closed(&mut client);
    assert!(started.elapsed() >= SUBSCRIBE_TIMEOUT - Duration::from_millis(500));
    wait_for("the connection to go", || open_connections(&server) == 0);
}

#[test]
fn a_subscribed_receiver_may_stay_quiet() {
    let server = Server::start(ports()).unwrap();
    let mut client = subscribe(server.port());
    wait_for("the subscription", || server.watching() == 1);
    thread::sleep(SUBSCRIBE_TIMEOUT + Duration::from_secs(1));
    assert_eq!(server.watching(), 1);
    server.send(Arc::new(wire::metadata("<x />")));
    assert_eq!(read_frame(&mut client).0.kind, wire::METADATA);
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
        "videotestsrc num-buffers=20 ! video/x-raw,format=I420,width=1920,height=1080,framerate=30/1 ! \
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
    let shared = SharedDecoder::new();
    let _decoder = Decoder::start(&shared, r.limit, encoding(&r)).unwrap();
    for au in &aus {
        shared.push(au);
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
