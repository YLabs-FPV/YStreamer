use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use super::transport::HEADER_LEN;
use super::*;

fn header(len: usize, session: [u8; 2], seq: u16, kind: u8) -> Vec<u8> {
    let mut d = vec![0u8; len];
    d[0..2].copy_from_slice(&(0x8000 | len as u16).to_le_bytes());
    d[2..4].copy_from_slice(&session);
    d[4..6].copy_from_slice(&seq.to_le_bytes());
    d[6] = kind;
    d
}

fn video(session: [u8; 2], seq: u16, unit: u8, count: u8, index: u8, payload: &[u8]) -> Vec<u8> {
    let mut d = header(HEADER_LEN, session, seq, KIND_VIDEO);
    d[16] = unit;
    d[17] = count | ((index & 1) << 7);
    d[18] = index >> 1;
    d.extend_from_slice(payload);
    let len = d.len() as u16;
    d[0..2].copy_from_slice(&(0x8000 | len).to_le_bytes());
    d
}

/// What the goggles saw of the session
#[derive(Default, Debug)]
struct Seen {
    controls: usize,
    acked: Option<u16>,
}

/// Plays the goggles: takes the opener, accepts it, sends two units, the
/// first in two fragments the wrong way round, and waits for the ack of
/// the last fragment
fn goggles(socket: UdpSocket, done: Arc<AtomicBool>, seen: mpsc::Sender<Seen>) {
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut buf = [0u8; 2048];
    let (len, client) = socket.recv_from(&mut buf).unwrap();
    assert_eq!(len, 48, "opener first");
    let session = [buf[2], buf[3]];

    socket
        .send_to(
            &[0x09, 0x80, session[0], session[1], 0, 0, 0, 0, 0x01],
            client,
        )
        .unwrap();
    socket
        .send_to(&header(34, session, 0x0100, KIND_STATE), client)
        .unwrap();
    for d in [
        video(session, 0x0210, 5, 2, 1, b"b"),
        video(session, 0x0208, 5, 2, 0, b"a"),
        video(session, 0x0218, 6, 1, 0, b"c"),
    ] {
        socket.send_to(&d, client).unwrap();
    }

    let mut s = Seen::default();
    let until = Instant::now() + Duration::from_secs(2);
    while s.acked != Some(0x0218) && Instant::now() < until {
        let Ok(len) = socket.recv(&mut buf) else {
            break;
        };
        match buf[6] {
            0x05 if buf[HEADER_LEN] == 0x55 => s.controls += 1,
            0x04 if len == 34 => s.acked = Some(u16::from_le_bytes([buf[8], buf[9]])),
            _ => {}
        }
    }
    done.store(true, Ordering::Relaxed);
    seen.send(s).unwrap();
}

#[test]
fn session_opens_reassembles_and_acks() {
    let goggles_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let ours = UdpSocket::bind("127.0.0.1:0").unwrap();
    ours.connect(goggles_socket.local_addr().unwrap()).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let finished = Arc::clone(&done);
    let peer = thread::spawn(move || goggles(goggles_socket, finished, tx));

    let mut events = Vec::new();
    let ended = run(
        &ours,
        || !done.load(Ordering::Relaxed),
        |event| {
            events.push(match event {
                Event::Open => "open".to_string(),
                Event::Video(unit) => String::from_utf8_lossy(unit).into_owned(),
            })
        },
    );
    peer.join().unwrap();
    let seen = rx.recv().unwrap();

    assert!(matches!(ended, Ended::Deselected));
    assert_eq!(events, ["open", "ab", "c"]);
    assert_eq!(seen.acked, Some(0x0218), "acks the last fragment");
    assert!(seen.controls >= 1, "keeps the session alive");
}

#[test]
fn silence_means_sharing_is_off() {
    let goggles_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let ours = UdpSocket::bind("127.0.0.1:0").unwrap();
    ours.connect(goggles_socket.local_addr().unwrap()).unwrap();
    let ended = run(&ours, || true, |_| panic!("nothing to report"));
    assert!(matches!(ended, Ended::NoAnswer));
}
