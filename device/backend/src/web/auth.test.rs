use super::*;

#[test]
fn hashes_verify() {
    let h = hash_password("hunter22");
    assert!(verify_hash(&h, "hunter22"));
    assert!(!verify_hash(&h, "hunter23"));
    assert!(!verify_hash("", ""));
}

#[test]
fn tokens_are_bound_to_the_secret() {
    let exp = now_secs() + 60;
    let token = format!("{exp}.{}", B64.encode(sign("a", exp)));
    assert!(token_valid("a", &token));
    assert!(!token_valid("b", &token));
    let old = now_secs() - 1;
    let expired = format!("{old}.{}", B64.encode(sign("a", old)));
    assert!(!token_valid("a", &expired));
}

#[test]
fn reads_the_password_of_basic_auth() {
    use base64::engine::general_purpose::STANDARD;
    let with = |value: String| {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, value.parse().unwrap());
        basic_password(&headers)
    };
    let basic = |s: &str| format!("Basic {}", STANDARD.encode(s));
    assert_eq!(
        with(basic("admin:pw:with:colons")).as_deref(),
        Some("pw:with:colons")
    );
    assert_eq!(with(basic(":pw")).as_deref(), Some("pw"));
    assert_eq!(
        with(format!("basic {}", STANDARD.encode("u:pw"))).as_deref(),
        Some("pw")
    );
    assert_eq!(with(basic("no colon")), None);
    assert_eq!(with("Bearer abc".into()), None);
    assert_eq!(with("Basic !!!".into()), None);
    assert_eq!(basic_password(&HeaderMap::new()), None);
}

#[tokio::test]
async fn a_remembered_drive_password_goes_with_a_new_password() {
    use base64::engine::general_purpose::STANDARD;
    let path = std::env::temp_dir().join(format!("ystreamer-auth-{}.json", std::process::id()));
    let auth = Auth::load(path.clone());
    let sending = |password: &str| {
        let mut headers = HeaderMap::new();
        let value = format!("Basic {}", STANDARD.encode(format!("me:{password}")));
        headers.insert(header::AUTHORIZATION, value.parse().unwrap());
        headers
    };

    auth.set_password(Some("first-one".into())).await.unwrap();
    assert!(auth.check_basic(&sending("first-one")).await);
    assert!(auth.drive_login.read().unwrap().is_some());
    // Again, from what was remembered
    assert!(auth.check_basic(&sending("first-one")).await);

    auth.set_password(Some("second-one".into())).await.unwrap();
    assert!(!auth.check_basic(&sending("first-one")).await);
    assert!(auth.check_basic(&sending("second-one")).await);
    let _ = fs::remove_file(path);
}
