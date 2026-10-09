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
