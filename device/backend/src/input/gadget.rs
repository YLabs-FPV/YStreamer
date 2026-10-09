use std::fs;
use std::path::Path;
use std::process::Command;

const CONFIGFS: &str = "/sys/kernel/config/usb_gadget";
const CONTROLLERS: &str = "/sys/class/udc";
const NAME: &str = "ystreamer";

#[derive(Debug, PartialEq)]
enum Outcome {
    Created,
    AlreadyThere(String),
}

pub fn ensure() {
    // Brings configfs along. The controller's own driver, dwc2, is built
    // into the kernel and needs only the overlay in config.txt
    if !Path::new(CONFIGFS).is_dir() {
        let _ = Command::new("modprobe").arg("libcomposite").status();
    }
    match setup(Path::new(CONFIGFS), Path::new(CONTROLLERS)) {
        Ok(Outcome::Created) => println!("[usb] gadget set up"),
        Ok(Outcome::AlreadyThere(_)) => {}
        Err(e) => eprintln!("[usb] no gadget, so no link to the goggles: {e}"),
    }
}

fn setup(configfs: &Path, controllers: &Path) -> Result<Outcome, String> {
    let gadgets = fs::read_dir(configfs)
        .map_err(|_| "USB gadget support isn't loaded (libcomposite)".to_string())?;
    // The controller takes one gadget at a time
    for gadget in gadgets.flatten() {
        let bound = fs::read_to_string(gadget.path().join("UDC")).unwrap_or_default();
        if !bound.trim().is_empty() {
            return Ok(Outcome::AlreadyThere(
                gadget.file_name().to_string_lossy().into_owned(),
            ));
        }
    }
    let controller = fs::read_dir(controllers)
        .ok()
        .and_then(|mut dir| dir.next()?.ok())
        .map(|c| c.file_name().to_string_lossy().into_owned())
        .ok_or(
            "the USB-C port isn't in device mode; \
             config.txt needs dtoverlay=dwc2,dr_mode=peripheral",
        )?;

    let gadget = configfs.join(NAME);
    let put = |file: &str, value: &str| {
        let path = gadget.join(file);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        fs::write(&path, value).map_err(|e| format!("{}: {e}", path.display()))
    };

    // An Android accessory, which is how the goggles know a phone
    put("idVendor", "0x18d1")?;
    put("idProduct", "0x2d00")?;
    put("bcdDevice", "0x0100")?;
    put("bcdUSB", "0x0200")?;
    put("bDeviceClass", "0xFF")?;
    put("bDeviceSubClass", "0xFF")?;
    put("bDeviceProtocol", "0x00")?;
    put("strings/0x409/serialnumber", "0123456789ABCDEF")?;
    put("strings/0x409/manufacturer", "YStreamer")?;
    put("strings/0x409/product", "YStreamer Goggles Adapter")?;
    put("configs/c.1/strings/0x409/configuration", "Config 1: AOA")?;
    put("configs/c.1/MaxPower", "250")?;

    // Generic serial, not the modem class, which ModemManager would grab
    let function = gadget.join("functions/gser.usb0");
    fs::create_dir_all(&function).map_err(|e| format!("{}: {e}", function.display()))?;
    let link = gadget.join("configs/c.1/gser.usb0");
    if !link.exists() {
        std::os::unix::fs::symlink(&function, &link)
            .map_err(|e| format!("{}: {e}", link.display()))?;
    }

    // Binding is what makes it appear on the bus
    put("UDC", &controller)?;
    Ok(Outcome::Created)
}

#[cfg(test)]
#[path = "gadget.test.rs"]
mod tests;
