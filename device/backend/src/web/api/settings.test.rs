use super::*;

/// Both buttons on, the input button on `pin`
fn with_buttons(pin: u8) -> Settings {
    let mut s = Settings::default();
    s.recording.button.enabled = true;
    s.recording.button.pin = 17;
    s.recording.button.led_pin = Some(18);
    s.input.button.enabled = true;
    s.input.button.pin = pin;
    s
}

#[test]
fn both_buttons_fit_on_their_own_pins() {
    assert!(validate(&with_buttons(27)).is_ok());
}

#[test]
fn the_input_button_cant_share_the_record_button_pin() {
    assert!(validate(&with_buttons(17)).is_err());
}

#[test]
fn the_input_button_cant_share_the_record_light_pin() {
    assert!(validate(&with_buttons(18)).is_err());
}

#[test]
fn a_pin_is_only_taken_by_a_button_that_is_on() {
    let mut s = with_buttons(17);
    s.recording.button.enabled = false;
    assert!(validate(&s).is_ok());
}
