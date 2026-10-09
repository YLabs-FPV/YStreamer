use super::*;

const PASSWD: &str = "\
root:x:0:0:root:/root:/bin/bash
daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin
sshd:x:100:65534::/run/sshd:/usr/sbin/nologin
ystreamer:x:1000:1000:,,,:/home/ystreamer:/bin/bash
guest:x:1001:1001::/home/guest:/bin/bash
";

#[test]
fn finds_the_account_people_log_in_with() {
    assert_eq!(login_user(PASSWD).as_deref(), Some("ystreamer"));
    assert_eq!(login_user("root:x:0:0:root:/root:/bin/bash\n"), None);
    assert_eq!(login_user(""), None);
    // a uid that only starts with 1000
    assert_eq!(login_user("svc:x:10001:10001::/:/bin/sh\n"), None);
}

#[test]
fn asks_for_a_usable_password() {
    assert!(check_password("correct horse").is_ok());
    assert!(check_password("with:colon and spaces").is_ok());
    assert!(check_password("pässwörd").is_ok());
    assert!(check_password("short").is_err());
    assert!(check_password("").is_err());
    assert!(check_password("two\nlines here").is_err());
    assert!(check_password(&"a".repeat(300)).is_err());
}
