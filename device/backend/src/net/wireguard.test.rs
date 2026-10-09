use super::*;

#[test]
fn reads_peers_from_the_dump() {
    let dump = "privkey\tpubkey\t51820\toff\n\
                peerkey\t(none)\t203.0.113.7:51820\t10.8.0.0/24,fd00::/64\t1000\t2048\t4096\t25\n\
                other\t(none)\t(none)\t(none)\t0\t0\t0\toff\n";
    let peers = parse_dump(dump, 1030);
    assert_eq!(
        peers,
        [
            Peer {
                endpoint: Some("203.0.113.7:51820".into()),
                allowed_ips: vec!["10.8.0.0/24".into(), "fd00::/64".into()],
                handshake_age_secs: Some(30),
                rx_bytes: 2048,
                tx_bytes: 4096,
                keepalive: true,
            },
            Peer {
                endpoint: None,
                allowed_ips: vec![],
                handshake_age_secs: None,
                rx_bytes: 0,
                tx_bytes: 0,
                keepalive: false,
            },
        ]
    );
}

#[test]
fn refuses_files_that_are_not_configurations() {
    assert!(check("hello").is_err());
    assert!(check("[Interface]\nPrivateKey = x\n").is_err());
    assert!(check("[Interface]\nPrivateKey = x\n[Peer]\nPublicKey = y\n").is_ok());
    assert!(check("[interface]\n privatekey=x\n[peer]\npublickey=y\n").is_ok());
}
