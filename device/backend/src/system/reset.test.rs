use super::*;

/// `held` over a pin that reads grounded for the first `grounded_polls` reads
fn hold(grounded_polls: u32) -> (bool, Duration) {
    let mut reads = 0;
    let mut waited = Duration::ZERO;
    let result = held(
        || {
            reads += 1;
            reads <= grounded_polls
        },
        Duration::from_secs(5),
        Duration::from_millis(100),
        |d| waited += d,
    );
    (result, waited)
}

#[test]
fn pin_left_alone_costs_no_time() {
    assert_eq!(hold(0), (false, Duration::ZERO));
}

#[test]
fn brief_touch_does_nothing() {
    let (reset, waited) = hold(20);
    assert!(!reset);
    assert_eq!(waited, Duration::from_secs(2));
}

#[test]
fn letting_go_just_before_the_end_does_nothing() {
    assert!(!hold(50).0);
}

#[test]
fn held_all_the_way_resets() {
    let (reset, waited) = hold(u32::MAX);
    assert!(reset);
    assert_eq!(waited, Duration::from_secs(5));
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ystreamer-reset-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn clears_settings_and_login_and_nothing_else() {
    let dir = scratch("clear");
    let settings = dir.join("settings.json");
    for file in [
        "settings.json",
        "auth.json",
        "overlay.png",
        "splash-image",
        "image",
    ] {
        fs::write(dir.join(file), file).unwrap();
    }

    clear(&settings);

    assert!(!settings.exists());
    assert!(!dir.join("auth.json").exists());
    assert_eq!(
        fs::read_to_string(dir.join("settings.before-reset.json")).unwrap(),
        "settings.json"
    );
    for kept in ["overlay.png", "splash-image", "image"] {
        assert!(dir.join(kept).exists(), "{kept}");
    }
}

#[test]
fn clearing_a_fresh_device_is_fine() {
    let dir = scratch("fresh");
    clear(&dir.join("settings.json"));
    assert!(fs::read_dir(&dir).unwrap().next().is_none());
}
