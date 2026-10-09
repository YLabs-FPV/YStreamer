use super::*;

fn button(pin: u8, led_pin: Option<u8>) -> ButtonSettings {
    ButtonSettings {
        pin,
        led_pin,
        ..Default::default()
    }
}

#[test]
fn the_defaults_are_valid() {
    assert!(ButtonSettings::default().validate().is_ok());
}

#[test]
fn accepts_free_header_pins() {
    assert!(button(17, None).validate().is_ok());
    assert!(button(4, Some(27)).validate().is_ok());
}

#[test]
fn rejects_pins_that_are_not_free() {
    // 2 and 3 are I2C, 14 and 15 the serial console
    for pin in [0, 2, 3, 14, 15, 28, 40] {
        assert!(button(pin, None).validate().is_err(), "button on {pin}");
        assert!(button(17, Some(pin)).validate().is_err(), "light on {pin}");
    }
}

#[test]
fn rejects_the_button_and_light_on_one_pin() {
    assert!(button(17, Some(17)).validate().is_err());
}
