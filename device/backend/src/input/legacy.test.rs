use super::*;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn asks_at_once_then_backs_off_to_the_cap() {
    let t0 = Instant::now();
    let mut a = Asking::new(t0);
    assert!(a.should_ask(t0));
    // Empty reads every 100 ms: no storm of requests
    assert!(!a.should_ask(t0 + ms(100)));
    assert!(!a.should_ask(t0 + ms(400)));
    assert!(a.should_ask(t0 + ms(500)));
    assert!(!a.should_ask(t0 + ms(1400)));
    assert!(a.should_ask(t0 + ms(1500)));
    assert!(a.should_ask(t0 + ms(3500)));
    assert!(!a.should_ask(t0 + ms(5000)));
    assert!(a.should_ask(t0 + ms(5500)));
}

#[test]
fn hints_once_after_a_silent_spell() {
    let t0 = Instant::now();
    let mut a = Asking::new(t0);
    assert!(!a.should_hint(t0 + ms(2900)));
    assert!(a.should_hint(t0 + ms(3000)));
    assert!(!a.should_hint(t0 + ms(9000)));
}

#[test]
fn video_resets_the_backoff() {
    let t0 = Instant::now();
    let mut a = Asking::new(t0);
    assert!(a.should_ask(t0));
    assert!(a.should_ask(t0 + ms(500)));
    a.got_video();
    let t1 = Instant::now();
    assert!(a.should_ask(t1));
    assert!(a.should_ask(t1 + ms(500)));
}
