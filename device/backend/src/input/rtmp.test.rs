use super::*;

use gstreamer as gst;
use gstreamer::prelude::*;
use rml_rtmp::sessions::{
    ClientSession, ClientSessionConfig, ClientSessionEvent, ClientSessionResult, PublishRequestType,
};
use rml_rtmp::time::RtmpTimestamp;

const KEY: &str = "testkey1";

type Received = Arc<Mutex<Vec<Vec<u8>>>>;

fn settings(allowed_from: &str) -> RtmpInputSettings {
    RtmpInputSettings {
        // Any free port, so tests can run side by side
        port: 0,
        key: KEY.into(),
        allowed_from: allowed_from.into(),
    }
}

/// A listening source and the pictures it hands on
fn source(cfg: &RtmpInputSettings) -> (Arc<RtmpSource>, Arc<InputState>, Received, u16) {
    let state = InputState::new(InputMode::Rtmp);
    let received: Received = Arc::default();
    let sink = Arc::clone(&received);
    let source = RtmpSource::start_with(
        Arc::clone(&state),
        cfg,
        Arc::new(move |au: &[u8]| sink.lock().unwrap().push(au.to_vec())),
    );
    let port = wait_until(|| source.status().port).expect("the source never listened");
    (source, state, received, port)
}

fn wait_until<T>(mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(v) = f() {
            return Some(v);
        }
        thread::sleep(Duration::from_millis(20));
    }
    None
}

fn have(elements: &[&str]) -> bool {
    gst::init().is_ok()
        && elements
            .iter()
            .all(|e| gst::ElementFactory::find(e).is_some())
}

/// Streams a few seconds of test picture the way an encoder app would,
/// noting every state the source went through. Whether it ended cleanly
fn push_with_gstreamer(source: &RtmpSource, port: u16, key: &str) -> (bool, Vec<RtmpInputState>) {
    let pipeline = gst::parse::launch(&format!(
        "videotestsrc is-live=true num-buffers=60 ! video/x-raw,width=320,height=240,framerate=30/1 ! \
         x264enc tune=zerolatency speed-preset=ultrafast key-int-max=15 ! \
         flvmux streamable=true ! rtmp2sink location=rtmp://127.0.0.1:{port}/live/{key}"
    ))
    .unwrap()
    .dynamic_cast::<gst::Pipeline>()
    .unwrap();
    let bus = pipeline.bus().unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    let ok = loop {
        let state = source.status().state;
        if seen.last() != Some(&state) {
            seen.push(state);
        }
        if let Some(msg) = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(20),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        ) {
            break matches!(msg.view(), gst::MessageView::Eos(_));
        }
        if Instant::now() > deadline {
            break false;
        }
    };
    pipeline.set_state(gst::State::Null).unwrap();
    (ok, seen)
}

#[test]
fn takes_a_stream_from_an_encoder() {
    if !have(&["videotestsrc", "x264enc", "flvmux", "rtmp2sink"]) {
        eprintln!("skipped: GStreamer's x264enc, flvmux or rtmp2sink isn't installed");
        return;
    }
    let (source, _state, received, port) = source(&settings(""));
    let (ok, seen) = push_with_gstreamer(&source, port, KEY);
    assert!(ok, "the push failed");
    assert!(seen.contains(&RtmpInputState::Live), "never live: {seen:?}");

    let pictures = received.lock().unwrap().clone();
    assert!(pictures.len() > 40, "only {} pictures", pictures.len());
    let kinds = |au: &[u8]| {
        crate::input::h264::nals(au)
            .map(|n| n.kind)
            .collect::<Vec<_>>()
    };
    // The first picture is a keyframe that a decoder can start at
    let first = kinds(&pictures[0]);
    assert_eq!(&first[..2], &[crate::input::h264::SPS, 8], "{first:?}");
    assert!(first.contains(&crate::input::h264::IDR));
    for au in &pictures {
        assert!(au.ends_with(&[0, 0, 0, 1, 0x09, 0xf0]));
        let k = kinds(au);
        assert_eq!(
            k.iter().filter(|&&k| k == crate::input::h264::AUD).count(),
            1
        );
        if k.contains(&crate::input::h264::IDR) {
            assert!(k.contains(&crate::input::h264::SPS));
        }
    }

    // Back to waiting for the next one once the app stops
    assert!(
        wait_until(|| (source.status().state == RtmpInputState::Waiting).then_some(())).is_some()
    );
    assert_eq!(source.status().publisher, None);
}

#[test]
fn turns_away_a_wrong_key() {
    if !have(&["videotestsrc", "x264enc", "flvmux", "rtmp2sink"]) {
        eprintln!("skipped: GStreamer's x264enc, flvmux or rtmp2sink isn't installed");
        return;
    }
    let (source, _state, received, port) = source(&settings(""));
    let (ok, seen) = push_with_gstreamer(&source, port, "guessed");
    assert!(!ok);
    assert!(!seen.contains(&RtmpInputState::Live));
    assert!(received.lock().unwrap().is_empty());
    let error = source.status().error.unwrap_or_default();
    assert!(error.contains("wrong stream key"), "{error}");
}

/// A publisher made from the library's own client side, for sending tags
/// no encoder here can make
struct Client {
    stream: TcpStream,
    session: ClientSession,
    buf: Vec<u8>,
}

impl Client {
    fn connect(port: u16) -> Self {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut handshake = Handshake::new(PeerType::Client);
        stream
            .write_all(&handshake.generate_outbound_p0_and_p1().unwrap())
            .unwrap();
        let mut buf = vec![0u8; 16 * 1024];
        let rest = loop {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0, "closed during the handshake");
            match handshake.process_bytes(&buf[..n]).unwrap() {
                HandshakeProcessResult::InProgress { response_bytes } => {
                    stream.write_all(&response_bytes).unwrap()
                }
                HandshakeProcessResult::Completed {
                    response_bytes,
                    remaining_bytes,
                } => {
                    stream.write_all(&response_bytes).unwrap();
                    break remaining_bytes;
                }
            }
        };
        let (session, _) = ClientSession::new(ClientSessionConfig::new()).unwrap();
        let mut client = Self {
            stream,
            session,
            buf,
        };
        let results = client.session.handle_input(&rest).unwrap();
        client.send_all(results);
        client
    }

    fn send_all(&mut self, results: Vec<ClientSessionResult>) -> Vec<ClientSessionEvent> {
        let mut events = Vec::new();
        for r in results {
            match r {
                ClientSessionResult::OutboundResponse(p) => {
                    let _ = self.stream.write_all(&p.bytes);
                }
                ClientSessionResult::RaisedEvent(e) => events.push(e),
                _ => {}
            }
        }
        events
    }

    /// Reads until the session raises `want`; false if the server hangs up
    fn wait_for(&mut self, want: fn(&ClientSessionEvent) -> bool) -> bool {
        loop {
            let n = match self.stream.read(&mut self.buf) {
                Ok(0) | Err(_) => return false,
                Ok(n) => n,
            };
            let Ok(results) = self.session.handle_input(&self.buf[..n]) else {
                return false;
            };
            if self.send_all(results).iter().any(want) {
                return true;
            }
        }
    }

    /// Connected and publishing under `key`, or None if turned away
    fn publish(port: u16, key: &str) -> Option<Self> {
        let mut c = Self::connect(port);
        let r = c.session.request_connection("live".into()).unwrap();
        c.send_all(vec![r]);
        if !c.wait_for(|e| matches!(e, ClientSessionEvent::ConnectionRequestAccepted)) {
            return None;
        }
        let r = c
            .session
            .request_publishing(key.into(), PublishRequestType::Live)
            .unwrap();
        c.send_all(vec![r]);
        c.wait_for(|e| matches!(e, ClientSessionEvent::PublishRequestAccepted))
            .then_some(c)
    }

    fn video(&mut self, tag: &[u8]) {
        let r = self
            .session
            .publish_video_data(tag.to_vec().into(), RtmpTimestamp::new(0), false)
            .unwrap();
        self.send_all(vec![r]);
    }

    /// Whether the server closes the connection within the read timeout
    fn hung_up(&mut self) -> bool {
        matches!(self.stream.read(&mut self.buf), Ok(0))
            || matches!(self.stream.read(&mut self.buf), Ok(0))
    }
}

#[test]
fn turns_away_h265_by_name() {
    let (source, _state, received, port) = source(&settings(""));
    let mut client = Client::publish(port, KEY).expect("publishing was refused");
    assert_eq!(source.status().state, RtmpInputState::Connected);
    assert_eq!(source.status().publisher.as_deref(), Some("127.0.0.1"));

    // An Enhanced RTMP H.265 sequence header, as OBS sends it
    let mut tag = vec![0x90];
    tag.extend(b"hvc1");
    tag.extend([1, 2, 3]);
    client.video(&tag);
    assert!(client.hung_up());
    assert!(received.lock().unwrap().is_empty());

    let status = wait_until(|| {
        let s = source.status();
        (s.state == RtmpInputState::Waiting).then_some(s)
    })
    .unwrap();
    assert!(status.error.unwrap().contains("H.265"));
}

#[test]
fn one_app_streams_at_a_time() {
    let (source, _state, _received, port) = source(&settings(""));
    let first = Client::publish(port, KEY).expect("publishing was refused");
    assert!(Client::publish(port, KEY).is_none());
    assert!(source.status().error.unwrap().contains("already streaming"));

    // Free again once the first hangs up
    drop(first);
    assert!(
        wait_until(|| (source.status().state == RtmpInputState::Waiting).then_some(())).is_some()
    );
    assert!(Client::publish(port, KEY).is_some());
}

#[test]
fn turns_away_addresses_not_allowed() {
    let (source, _state, _received, port) = source(&settings("192.0.2.7"));
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut buf = [0u8; 16];
    assert!(matches!(stream.read(&mut buf), Ok(0) | Err(_)));
    let error = source.status().error.unwrap_or_default();
    assert!(error.contains("not one of the allowed"), "{error}");
}

#[test]
fn a_new_key_drops_the_app_streaming() {
    let (source, _state, _received, port) = source(&settings(""));
    let mut client = Client::publish(port, KEY).expect("publishing was refused");
    source.apply(&RtmpInputSettings {
        key: "otherkey".into(),
        ..settings("")
    });
    assert!(client.hung_up());
    assert!(Client::publish(port, KEY).is_none());
}

#[test]
fn closes_the_port_when_another_input_is_picked() {
    let (source, state, _received, port) = source(&settings(""));
    let client = Client::publish(port, KEY).expect("publishing was refused");
    state.set_mode(InputMode::Uvc);
    assert!(wait_until(|| (source.status().state == RtmpInputState::Idle).then_some(())).is_some());
    drop(client);
    thread::sleep(POLL * 3);
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
}
