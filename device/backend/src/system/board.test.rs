use super::*;

#[test]
fn raspberry_pi_by_model() {
    assert_eq!(
        Board::from_device_tree(
            "Raspberry Pi 4 Model B Rev 1.4\0",
            "raspberrypi,4-model-b\0brcm,bcm2711\0"
        ),
        Board::RaspberryPi
    );
    assert_eq!(
        Board::from_device_tree("Raspberry Pi Compute Module 4 Rev 1.1\0", ""),
        Board::RaspberryPi
    );
}

#[test]
fn rock_3a_by_compatible_or_model() {
    // The Radxa 6.1 kernel
    assert_eq!(
        Board::from_device_tree("Radxa ROCK 3 Model A\0", "radxa,rock3a\0rockchip,rk3568\0"),
        Board::Rockchip
    );
    // The older 5.10 one spells the model differently and lists the chip only
    assert_eq!(
        Board::from_device_tree("Radxa ROCK3 Model A\0", "rockchip,rk3568\0"),
        Board::Rockchip
    );
    assert_eq!(
        Board::from_device_tree("Radxa ROCK 5B", ""),
        Board::Rockchip
    );
}

#[test]
fn anything_else_is_other() {
    assert_eq!(Board::from_device_tree("", ""), Board::Other);
    assert_eq!(
        Board::from_device_tree("Generic x86 PC", "pc\0"),
        Board::Other
    );
    // Not a word boundary trap: "rock" inside another name isn't Rockchip
    assert_eq!(
        Board::from_device_tree("Bedrock Mini", "bedrock,mini\0"),
        Board::Other
    );
}
