use super::*;
use std::path::PathBuf;

struct Sysfs {
    root: PathBuf,
    configfs: PathBuf,
    controllers: PathBuf,
}

impl Sysfs {
    fn new(controller: Option<&str>) -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("ystreamer-gadget-{}-{n}", std::process::id()));
        let configfs = root.join("usb_gadget");
        let controllers = root.join("udc");
        fs::create_dir_all(&configfs).unwrap();
        fs::create_dir_all(&controllers).unwrap();
        if let Some(name) = controller {
            fs::create_dir_all(controllers.join(name)).unwrap();
        }
        Self {
            root,
            configfs,
            controllers,
        }
    }

    fn read(&self, file: &str) -> String {
        fs::read_to_string(self.configfs.join(NAME).join(file)).unwrap()
    }
}

impl Drop for Sysfs {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn builds_an_android_accessory_with_an_accessory_function() {
    let sys = Sysfs::new(Some("fe980000.usb"));
    assert_eq!(
        setup(&sys.configfs, &sys.controllers, Board::RaspberryPi),
        Ok(Outcome::Ready("fe980000.usb".into()))
    );

    assert_eq!(sys.read("idVendor"), "0x18d1");
    assert_eq!(sys.read("idProduct"), "0x2d00");
    assert_eq!(sys.read("bDeviceClass"), "0xFF");
    assert_eq!(sys.read("strings/0x409/manufacturer"), "YStreamer");
    assert_eq!(sys.read("configs/c.1/MaxPower"), "250");
    // Full and high speed descriptors only, so no faster than that
    assert_eq!(sys.read("max_speed"), "high-speed");
    // Bound only once the function has its descriptors
    assert!(!sys.configfs.join(NAME).join("UDC").exists());

    let gadget = sys.configfs.join(NAME);
    assert!(gadget.join("functions").join(FUNCTION).is_dir());
    assert_eq!(
        fs::read_link(gadget.join("configs/c.1").join(FUNCTION)).unwrap(),
        gadget.join("functions").join(FUNCTION)
    );
}

#[test]
fn takes_over_the_serial_gadget_an_older_version_left_bound() {
    let sys = Sysfs::new(Some("fe980000.usb"));
    let old = sys.configfs.join(OLD_NAME);
    fs::create_dir_all(old.join("functions/gser.usb0")).unwrap();
    fs::create_dir_all(old.join("configs/c.1")).unwrap();
    std::os::unix::fs::symlink(
        old.join("functions/gser.usb0"),
        old.join("configs/c.1/gser.usb0"),
    )
    .unwrap();
    fs::write(old.join("UDC"), "fe980000.usb\n").unwrap();

    assert_eq!(
        setup(&sys.configfs, &sys.controllers, Board::RaspberryPi),
        Ok(Outcome::Ready("fe980000.usb".into()))
    );
    assert_eq!(fs::read_to_string(old.join("UDC")).unwrap(), "\n");
    // Left as it was, for a rollback to bind again
    assert!(old.join("configs/c.1/gser.usb0").is_symlink());
    assert!(!old.join("configs/c.1").join(FUNCTION).exists());
    let gadget = sys.configfs.join(NAME);
    assert!(gadget.join("configs/c.1").join(FUNCTION).is_symlink());
    assert!(!gadget.join("configs/c.1/gser.usb0").exists());
}

#[test]
fn leaves_its_own_gadget_alone_while_another_process_serves_it() {
    let sys = Sysfs::new(Some("fe980000.usb"));
    setup(&sys.configfs, &sys.controllers, Board::RaspberryPi).unwrap();
    fs::write(sys.configfs.join(NAME).join("UDC"), "fe980000.usb\n").unwrap();
    assert_eq!(
        setup(&sys.configfs, &sys.controllers, Board::RaspberryPi),
        Ok(Outcome::AlreadyThere(NAME.into()))
    );
    assert_eq!(sys.read("UDC"), "fe980000.usb\n");
}

/// Radxa's images can bind their own gadget (radxa-otgutils: ADB and USB
/// networking, in a gadget named after the controller) to the one
/// controller, which the goggles need
#[test]
fn takes_the_controller_over_from_another_programs_gadget() {
    let sys = Sysfs::new(Some("fcc00000.usb"));
    let theirs = sys.configfs.join("fcc00000.usb");
    fs::create_dir_all(theirs.join("functions/ffs.adb")).unwrap();
    fs::create_dir_all(theirs.join("functions/ecm.usb0")).unwrap();
    fs::write(theirs.join("UDC"), "fcc00000.usb\n").unwrap();
    assert_eq!(
        setup(&sys.configfs, &sys.controllers, Board::Rockchip),
        Ok(Outcome::Ready("fcc00000.usb".into()))
    );
    // Unbound, not removed: whatever made it can have it back
    assert_eq!(fs::read_to_string(theirs.join("UDC")).unwrap(), "\n");
    assert!(theirs.join("functions/ffs.adb").is_dir());
    assert!(
        sys.configfs
            .join(NAME)
            .join("functions")
            .join(FUNCTION)
            .is_dir()
    );
}

#[test]
fn finishes_a_gadget_that_was_left_unbound() {
    let sys = Sysfs::new(Some("fe980000.usb"));
    setup(&sys.configfs, &sys.controllers, Board::RaspberryPi).unwrap();
    fs::write(sys.configfs.join(NAME).join("UDC"), "\n").unwrap();
    assert_eq!(
        setup(&sys.configfs, &sys.controllers, Board::RaspberryPi),
        Ok(Outcome::Ready("fe980000.usb".into()))
    );
}

#[test]
fn describes_a_vendor_interface_with_a_bulk_pair() {
    let d = descriptors();
    let word = |i: usize| u32::from_le_bytes(d[i * 4..i * 4 + 4].try_into().unwrap());
    // magic v2, total length, full + high speed, 3 descriptors at each
    assert_eq!(
        (word(0), word(1) as usize, word(2), word(3), word(4)),
        (3, d.len(), 3, 3, 3)
    );
    for (at, packet) in [(20, 64u16), (43, 512)] {
        // interface: 2 endpoints, class and subclass 0xFF, protocol 0
        assert_eq!(&d[at..at + 9], &[9, 4, 0, 0, 2, 0xFF, 0xFF, 0, 1]);
        let out = &d[at + 9..at + 16];
        let inp = &d[at + 16..at + 23];
        assert_eq!((out[2], out[3]), (0x01, 0x02));
        assert_eq!((inp[2], inp[3]), (0x82, 0x02));
        assert_eq!(u16::from_le_bytes([out[4], out[5]]), packet);
    }
    assert_eq!(d.len(), 20 + 2 * 23);

    let s = strings();
    assert_eq!(
        u32::from_le_bytes(s[4..8].try_into().unwrap()) as usize,
        s.len()
    );
    assert!(s.ends_with(b"\x09\x04AOA\0"));
}

#[test]
fn says_why_it_cannot() {
    let no_port = Sysfs::new(None);
    let e = setup(&no_port.configfs, &no_port.controllers, Board::RaspberryPi).unwrap_err();
    assert!(e.contains("dr_mode=peripheral"), "{e}");
    assert!(!no_port.configfs.join(NAME).exists());

    // The ROCK 3A has a switch for it, not a boot setting
    let e = setup(&no_port.configfs, &no_port.controllers, Board::Rockchip).unwrap_err();
    assert!(e.contains("OTG switch"), "{e}");
    assert!(!e.contains("config.txt"), "{e}");

    let e = setup(
        Path::new("/nonexistent/usb_gadget"),
        &no_port.controllers,
        Board::RaspberryPi,
    )
    .unwrap_err();
    assert!(e.contains("libcomposite"), "{e}");
}
