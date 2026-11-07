use super::*;

#[test]
fn a_screen_plugged_in_is_a_hotplug() {
    let msg = b"change@/devices/platform/gpu/drm/card1\0ACTION=change\0\
        DEVPATH=/devices/platform/gpu/drm/card1\0SUBSYSTEM=drm\0HOTPLUG=1\0\
        DEVNAME=dri/card1\0DEVTYPE=drm_minor\0SEQNUM=4242\0MAJOR=226\0MINOR=1\0";
    assert!(is_drm_hotplug(msg));
}

#[test]
fn other_drm_events_are_not() {
    // A connector property changing, e.g. content protection
    let msg = b"change@/devices/platform/gpu/drm/card1\0ACTION=change\0\
        SUBSYSTEM=drm\0CONNECTOR=33\0PROPERTY=21\0";
    assert!(!is_drm_hotplug(msg));
}

#[test]
fn hotplug_from_another_subsystem_is_not() {
    let msg = b"change@/devices/foo\0ACTION=change\0SUBSYSTEM=usb\0HOTPLUG=1\0";
    assert!(!is_drm_hotplug(msg));
}

#[test]
fn only_whole_fields_match() {
    let msg = b"change@/x\0SUBSYSTEM=drm\0HOTPLUG=10\0";
    assert!(!is_drm_hotplug(msg));
    assert!(!is_drm_hotplug(b""));
}
