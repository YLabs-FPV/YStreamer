use super::*;

const HOSTS_FROM_CLOUD_INIT: &str = "\
# Your system has configured 'manage_etc_hosts' as True.
127.0.1.1 rpi rpi
127.0.0.1 localhost

::1 localhost ip6-localhost ip6-loopback
";

#[test]
fn renames_the_local_line_and_nothing_else() {
    let hosts = with_local_name(HOSTS_FROM_CLOUD_INIT, "ystreamer");
    assert_eq!(
        hosts,
        "\
# Your system has configured 'manage_etc_hosts' as True.
127.0.1.1 ystreamer
127.0.0.1 localhost

::1 localhost ip6-localhost ip6-loopback
"
    );
    // and again changes nothing
    assert_eq!(with_local_name(&hosts, "ystreamer"), hosts);
}

#[test]
fn adds_the_local_line_when_there_is_none() {
    assert_eq!(
        with_local_name("127.0.0.1 localhost\n", "ystreamer"),
        "127.0.0.1 localhost\n127.0.1.1 ystreamer\n"
    );
    assert_eq!(with_local_name("", "pi"), "127.0.1.1 pi\n");
}
