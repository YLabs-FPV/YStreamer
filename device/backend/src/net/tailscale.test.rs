use super::*;

fn parse(json: &str) -> Status {
    interpret(serde_json::from_str(json).unwrap())
}

#[test]
fn a_fresh_install_needs_a_login() {
    let s = parse(r#"{"BackendState":"NeedsLogin","AuthURL":"","Self":{"HostName":"rpi"}}"#);
    assert_eq!(s.state, State::NeedsLogin);
    assert_eq!(s.login_url, None);
    assert_eq!(s.name, None);

    let s = parse(r#"{"BackendState":"NeedsLogin","AuthURL":"https://login.tailscale.com/a/1"}"#);
    assert_eq!(
        s.login_url.as_deref(),
        Some("https://login.tailscale.com/a/1")
    );
}

#[test]
fn connected_reports_name_addresses_and_active_peers() {
    let s = parse(
        r#"{"BackendState":"Running","AuthURL":"",
        "Self":{"HostName":"rpi","DNSName":"rpi.tail1234.ts.net.","TailscaleIPs":["100.64.0.5","fd7a::5"]},
        "CurrentTailnet":{"Name":"me@example.com"},
        "Peer":{
          "a":{"DNSName":"laptop.tail1234.ts.net.","Active":true,"CurAddr":"192.168.1.9:41641","Relay":"ams"},
          "b":{"DNSName":"phone.tail1234.ts.net.","Active":true,"CurAddr":"","Relay":"fra"},
          "c":{"DNSName":"idle.tail1234.ts.net.","Active":false}
        }}"#,
    );
    assert_eq!(s.state, State::Connected);
    assert_eq!(s.name.as_deref(), Some("rpi.tail1234.ts.net"));
    assert_eq!(s.addresses, ["100.64.0.5", "fd7a::5"]);
    assert_eq!(s.network.as_deref(), Some("me@example.com"));
    assert_eq!(
        s.peers,
        [
            Peer {
                name: "laptop".into(),
                direct: true,
                relay: Some("ams".into())
            },
            Peer {
                name: "phone".into(),
                direct: false,
                relay: Some("fra".into())
            },
        ]
    );
}

#[test]
fn stopped_keeps_the_name_but_has_no_address() {
    let s = parse(
        r#"{"BackendState":"Stopped","Self":{"DNSName":"rpi.tail1234.ts.net.","TailscaleIPs":["100.64.0.5"]}}"#,
    );
    assert_eq!(s.state, State::Disconnected);
    assert_eq!(s.name.as_deref(), Some("rpi.tail1234.ts.net"));
    assert!(s.addresses.is_empty());
}
