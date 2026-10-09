use super::*;

fn eth(address: &str, gateway: &str, dns: &str) -> EthernetSettings {
    EthernetSettings {
        mode: EthernetMode::Static,
        address: address.into(),
        gateway: gateway.into(),
        dns: dns.into(),
    }
}

#[test]
fn accepts_a_normal_static_setup() {
    assert!(
        eth("192.168.1.50/24", "192.168.1.1", "1.1.1.1, 8.8.8.8")
            .validate()
            .is_ok()
    );
    assert!(eth("10.0.0.2/8", "", "").validate().is_ok());
}

#[test]
fn rejects_what_would_lock_you_out() {
    assert!(
        eth("192.168.1.50", "", "").validate().is_err(),
        "missing prefix"
    );
    assert!(
        eth("192.168.1.300/24", "", "").validate().is_err(),
        "bad octet"
    );
    assert!(
        eth("192.168.1.50/33", "", "").validate().is_err(),
        "bad prefix"
    );
    assert!(eth("0.0.0.0/24", "", "").validate().is_err(), "unspecified");
    assert!(
        eth("192.168.1.50/24", "192.168.2.1", "")
            .validate()
            .is_err(),
        "gateway off-net"
    );
    assert!(
        eth("192.168.1.50/24", "", "8.8.8").validate().is_err(),
        "bad dns"
    );
}

#[test]
fn dhcp_ignores_the_static_fields() {
    let mut e = eth("garbage", "junk", "nope");
    e.mode = EthernetMode::Auto;
    assert!(e.validate().is_ok());
}
