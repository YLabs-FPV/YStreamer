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
fn builds_an_android_accessory_with_a_serial_function() {
    let sys = Sysfs::new(Some("fe980000.usb"));
    assert_eq!(setup(&sys.configfs, &sys.controllers), Ok(Outcome::Created));

    assert_eq!(sys.read("idVendor"), "0x18d1");
    assert_eq!(sys.read("idProduct"), "0x2d00");
    assert_eq!(sys.read("bDeviceClass"), "0xFF");
    assert_eq!(sys.read("strings/0x409/manufacturer"), "YStreamer");
    assert_eq!(sys.read("configs/c.1/MaxPower"), "250");
    assert_eq!(sys.read("UDC"), "fe980000.usb");

    let gadget = sys.configfs.join(NAME);
    assert!(gadget.join("functions/gser.usb0").is_dir());
    assert_eq!(
        fs::read_link(gadget.join("configs/c.1/gser.usb0")).unwrap(),
        gadget.join("functions/gser.usb0")
    );
}

#[test]
fn leaves_a_bound_gadget_alone() {
    let sys = Sysfs::new(Some("fe980000.usb"));
    setup(&sys.configfs, &sys.controllers).unwrap();
    assert_eq!(
        setup(&sys.configfs, &sys.controllers),
        Ok(Outcome::AlreadyThere(NAME.into()))
    );

    // one made by hand, under another name
    let other = Sysfs::new(Some("fe980000.usb"));
    fs::create_dir_all(other.configfs.join("ycast")).unwrap();
    fs::write(other.configfs.join("ycast/UDC"), "fe980000.usb\n").unwrap();
    assert_eq!(
        setup(&other.configfs, &other.controllers),
        Ok(Outcome::AlreadyThere("ycast".into()))
    );
    assert!(!other.configfs.join(NAME).exists());
}

#[test]
fn finishes_a_gadget_that_was_left_unbound() {
    let sys = Sysfs::new(Some("fe980000.usb"));
    setup(&sys.configfs, &sys.controllers).unwrap();
    fs::write(sys.configfs.join(NAME).join("UDC"), "\n").unwrap();
    assert_eq!(setup(&sys.configfs, &sys.controllers), Ok(Outcome::Created));
    assert_eq!(sys.read("UDC"), "fe980000.usb");
}

#[test]
fn says_why_it_cannot() {
    let no_port = Sysfs::new(None);
    let e = setup(&no_port.configfs, &no_port.controllers).unwrap_err();
    assert!(e.contains("dr_mode=peripheral"), "{e}");
    assert!(!no_port.configfs.join(NAME).exists());

    let e = setup(Path::new("/nonexistent/usb_gadget"), &no_port.controllers).unwrap_err();
    assert!(e.contains("libcomposite"), "{e}");
}
