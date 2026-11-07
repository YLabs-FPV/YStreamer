use super::*;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[test]
fn waits_while_the_parameter_sets_may_still_come() {
    let t0 = Instant::now();
    let mut m = MissedStart::default();
    assert_eq!(m.check(t0, false), Recovery::Wait);
    assert_eq!(m.check(t0 + secs(2), false), Recovery::Wait);
    assert_eq!(m.check(t0 + secs(3), true), Recovery::Wait);
    assert_eq!(m.check(t0 + secs(30), true), Recovery::Wait);
}

#[test]
fn replugs_once_then_asks_the_user() {
    let t0 = Instant::now();
    let mut m = MissedStart::default();
    assert_eq!(m.check(t0, false), Recovery::Wait);
    assert_eq!(m.check(t0 + secs(3), false), Recovery::Replug);
    // The replug starts a new link, with no parameter sets either
    m.new_link();
    let t1 = t0 + secs(7);
    assert_eq!(m.check(t1, false), Recovery::Wait);
    assert_eq!(m.check(t1 + secs(3), false), Recovery::AskUser);
    assert_eq!(m.check(t1 + secs(9), false), Recovery::AskUser);
}

#[test]
fn a_good_start_forgets_earlier_trouble() {
    let t0 = Instant::now();
    let mut m = MissedStart::default();
    m.check(t0, false);
    assert_eq!(m.check(t0 + secs(3), false), Recovery::Replug);
    m.new_link();
    assert_eq!(m.check(t0 + secs(7), true), Recovery::Wait);
    // A later restart gets its own replug again
    m.new_link();
    let t1 = t0 + secs(60);
    m.check(t1, false);
    assert_eq!(m.check(t1 + secs(3), false), Recovery::Replug);
}
