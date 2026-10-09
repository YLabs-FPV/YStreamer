use super::*;

// lsblk on a Pi 4 with a USB stick just plugged in
const STICK_PLUGGED_IN: &str = r#"{"blockdevices": [
  {"path":"/dev/loop0","fstype":"swap","rm":false,"hotplug":false,"mountpoint":null},
  {"path":"/dev/sda","fstype":null,"rm":true,"hotplug":false,"mountpoint":null},
  {"path":"/dev/sda1","fstype":"vfat","rm":true,"hotplug":false,"mountpoint":null},
  {"path":"/dev/mmcblk0","fstype":null,"rm":false,"hotplug":true,"mountpoint":null},
  {"path":"/dev/mmcblk0p1","fstype":"vfat","rm":false,"hotplug":true,"mountpoint":"/boot/firmware"},
  {"path":"/dev/mmcblk0p2","fstype":"ext4","rm":false,"hotplug":true,"mountpoint":"/"},
  {"path":"/dev/zram0","fstype":"swap","rm":false,"hotplug":false,"mountpoint":"[SWAP]"}
]}"#;

#[test]
fn finds_the_stick_and_nothing_else() {
    assert_eq!(unmounted(STICK_PLUGGED_IN), ["/dev/sda1"]);
}

#[test]
fn leaves_mounted_drives_alone() {
    let mounted = STICK_PLUGGED_IN.replace(
        r#""/dev/sda1","fstype":"vfat","rm":true,"hotplug":false,"mountpoint":null"#,
        r#""/dev/sda1","fstype":"vfat","rm":true,"hotplug":false,"mountpoint":"/media/root/USB DISK""#,
    );
    assert!(unmounted(&mounted).is_empty());
}

#[test]
fn takes_a_usb_disk_that_does_not_call_itself_removable() {
    let ssd = r#"{"blockdevices": [
      {"path":"/dev/sdb","fstype":null,"rm":false,"hotplug":true,"mountpoint":null,
       "children":[
         {"path":"/dev/sdb1","fstype":"exfat","rm":false,"hotplug":true,"mountpoint":null},
         {"path":"/dev/sdb2","fstype":"swap","rm":false,"hotplug":true,"mountpoint":null}
       ]}
    ]}"#;
    assert_eq!(unmounted(ssd), ["/dev/sdb1"]);
}

#[test]
fn never_touches_the_system_card() {
    let spare = r#"{"blockdevices": [
      {"path":"/dev/mmcblk0p3","fstype":"ext4","rm":false,"hotplug":true,"mountpoint":null}
    ]}"#;
    assert!(unmounted(spare).is_empty());
    assert!(unmounted("not json").is_empty());
}
