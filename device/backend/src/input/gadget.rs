use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

use crate::system::board::Board;

const CONFIGFS: &str = "/sys/kernel/config/usb_gadget";
const CONTROLLERS: &str = "/sys/class/udc";
// Not the name older versions use: they'd add their serial function next to
// this one's, and the controller won't take a gadget with an unserved
// accessory function, so going back to one would leave no link
const NAME: &str = "ystreamer-aoa";
// What older versions build, with the generic serial function. Only unbound,
// so a rollback finds it as it left it
const OLD_NAME: &str = "ystreamer";
// A FunctionFS function, so the interface can say what it is: vendor class
// and subclass 0xFF, like any Android accessory. The RC-N2/N3 resets the bus
// every couple of seconds on anything else (the generic serial function's
// subclass is 0); the goggles take either
const FUNCTION: &str = "ffs.ystreamer";
const MOUNT: &str = "/run/ystreamer-usb";
/// Bulk OUT: what the goggles or the remote send us
pub const EP_OUT: &str = "/run/ystreamer-usb/ep1";
/// Bulk IN: what we send them
pub const EP_IN: &str = "/run/ystreamer-usb/ep2";

#[derive(Debug, PartialEq)]
enum Outcome {
    /// Built and waiting for its descriptors, then the controller named here
    Ready(String),
    AlreadyThere(String),
}

pub fn ensure() {
    // libcomposite brings configfs along, usb_f_fs is the function. Both
    // are built into the Pi's and the ROCK 3A's kernels, where this is a
    // no-op; the controller's own driver (dwc2, dwc3) is as well
    for module in ["libcomposite", "usb_f_fs"] {
        let _ = Command::new("modprobe").arg(module).status();
    }
    let started = match setup(Path::new(CONFIGFS), Path::new(CONTROLLERS), Board::detect()) {
        Ok(Outcome::Ready(controller)) => start(&Path::new(CONFIGFS).join(NAME), &controller),
        Ok(Outcome::AlreadyThere(_)) => return,
        Err(e) => Err(e),
    };
    match started {
        Ok(()) => println!("[usb] gadget set up"),
        Err(e) => eprintln!("[usb] no gadget, so no link to the goggles: {e}"),
    }
}

fn setup(configfs: &Path, controllers: &Path, board: Board) -> Result<Outcome, String> {
    let gadgets = fs::read_dir(configfs)
        .map_err(|_| "USB gadget support isn't loaded (libcomposite)".to_string())?;
    // The controller takes one gadget at a time
    for gadget in gadgets.flatten() {
        let udc = gadget.path().join("UDC");
        let bound = fs::read_to_string(&udc).unwrap_or_default();
        if bound.trim().is_empty() {
            continue;
        }
        let name = gadget.file_name().to_string_lossy().into_owned();
        // This one's unbinds when its process exits, so bound means another
        // process is serving it
        if name == NAME {
            return Ok(Outcome::AlreadyThere(name));
        }
        // An older version's stays bound after it exits. Anything else is
        // another program's: Radxa's images bind one of their own (ADB over
        // USB) on the only port the goggles can use
        if name != OLD_NAME {
            eprintln!("[usb] taking the port over from the {name} gadget");
        }
        fs::write(&udc, "\n").map_err(|e| format!("{}: {e}", udc.display()))?;
    }
    let controller = fs::read_dir(controllers)
        .ok()
        .and_then(|mut dir| dir.next()?.ok())
        .map(|c| c.file_name().to_string_lossy().into_owned())
        .ok_or_else(|| no_controller(board).to_string())?;

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
    // The descriptors below are full and high speed only. The Pi's dwc2 goes
    // no faster, but the ROCK 3A's dwc3 does, and a super-speed host would
    // find no configuration at all unless the gadget is capped here. A
    // kernel too old to have the attribute is a dwc2 one, so it's not fatal
    if let Err(e) = put("max_speed", "high-speed") {
        eprintln!("[usb] couldn't cap the gadget at high speed: {e}");
    }
    put("strings/0x409/serialnumber", "0123456789ABCDEF")?;
    put("strings/0x409/manufacturer", "YStreamer")?;
    put("strings/0x409/product", "YStreamer Goggles Adapter")?;
    put("configs/c.1/strings/0x409/configuration", "Config 1: AOA")?;
    put("configs/c.1/MaxPower", "250")?;

    let function = gadget.join("functions").join(FUNCTION);
    fs::create_dir_all(&function).map_err(|e| format!("{}: {e}", function.display()))?;
    let link = gadget.join("configs/c.1").join(FUNCTION);
    if !link.exists() {
        std::os::unix::fs::symlink(&function, &link)
            .map_err(|e| format!("{}: {e}", link.display()))?;
    }
    Ok(Outcome::Ready(controller))
}

/// Nothing under /sys/class/udc: the port is a host, and each board has its
/// own way of making it a device
fn no_controller(board: Board) -> &'static str {
    match board {
        Board::RaspberryPi => {
            "the USB-C port isn't in device mode; \
             config.txt needs dtoverlay=dwc2,dr_mode=peripheral"
        }
        Board::Rockchip => {
            "no USB device controller; set the board's OTG switch to DEVICE \
             (on a ROCK 3A: the switch beside the upper USB 3 port), make sure \
             rsetup's overlay doesn't force the port to host mode, and reboot"
        }
        Board::Other => "no USB device controller: the board's USB port isn't in device mode",
    }
}

/// The kernel binds the gadget only once ep0 has the function's descriptors,
/// and unbinds it when ep0 closes, so ep0 stays open for the life of the
/// process
fn start(gadget: &Path, controller: &str) -> Result<(), String> {
    mount()?;
    let path = format!("{MOUNT}/ep0");
    let mut ep0 = File::options()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("{path}: {e}"))?;
    ep0.write_all(&descriptors())
        .and_then(|()| ep0.write_all(&strings()))
        .map_err(|e| format!("{path}: {e}"))?;
    thread::Builder::new()
        .name("usb-ep0".into())
        .spawn(move || serve_ep0(ep0))
        .map_err(|e| e.to_string())?;

    // Binding is what makes it appear on the bus
    let udc = gadget.join("UDC");
    fs::write(&udc, controller).map_err(|e| format!("{}: {e}", udc.display()))
}

fn mount() -> Result<(), String> {
    let instance = FUNCTION.trim_start_matches("ffs.");
    let mounts = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let mounted = mounts.lines().find_map(|l| {
        let mut fields = l.split(' ');
        let source = fields.next()?;
        (fields.next()? == MOUNT).then_some(source)
    });
    match mounted {
        Some(source) if source == instance => return Ok(()),
        // Another function's, from a build that named it differently
        Some(_) => {
            let _ = Command::new("umount").arg(MOUNT).status();
        }
        None => {}
    }
    fs::create_dir_all(MOUNT).map_err(|e| format!("{MOUNT}: {e}"))?;
    match Command::new("mount")
        .args(["-t", "functionfs", instance, MOUNT])
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(format!("couldn't mount the accessory function on {MOUNT}")),
        Err(e) => Err(format!("mount: {e}")),
    }
}

/// FunctionFS descriptors: one interface with a bulk pair, at full and high
/// speed. ep1 is the OUT endpoint, ep2 the IN one
fn descriptors() -> Vec<u8> {
    const MAGIC_V2: u32 = 3;
    const HAS_FS_DESC: u32 = 1;
    const HAS_HS_DESC: u32 = 2;
    let interface = [9, 4, 0, 0, 2, 0xFF, 0xFF, 0x00, 1];
    let bulk = |address: u8, packet: u16| {
        let [lo, hi] = packet.to_le_bytes();
        [7, 5, address, 0x02, lo, hi, 0]
    };
    let mut body = Vec::new();
    for packet in [64, 512] {
        body.extend_from_slice(&interface);
        body.extend_from_slice(&bulk(0x01, packet));
        body.extend_from_slice(&bulk(0x82, packet));
    }
    let mut out = Vec::new();
    let length = 20 + body.len() as u32;
    for word in [MAGIC_V2, length, HAS_FS_DESC | HAS_HS_DESC, 3, 3] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.extend_from_slice(&body);
    out
}

/// The interface's name, in US English
fn strings() -> Vec<u8> {
    const MAGIC: u32 = 2;
    let name = b"AOA\0";
    let length = 16 + 2 + name.len() as u32;
    let mut out = Vec::new();
    for word in [MAGIC, length, 1, 1] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.extend_from_slice(&0x0409u16.to_le_bytes());
    out.extend_from_slice(name);
    out
}

/// Events from the function. Nothing on this interface uses a control
/// request, so any that arrive get an empty answer
fn serve_ep0(mut ep0: File) {
    const SETUP: u8 = 4;
    let mut event = [0u8; 12];
    let mut logged = false;
    loop {
        match ep0.read(&mut event) {
            Ok(12) if event[8] == SETUP => {
                let length = u16::from_le_bytes([event[6], event[7]]) as usize;
                let _ = if event[0] & 0x80 != 0 {
                    ep0.write(&[])
                } else {
                    ep0.read(&mut vec![0; length])
                };
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => {
                if !logged {
                    eprintln!("[usb] accessory function: {e}");
                    logged = true;
                }
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

#[cfg(test)]
#[path = "gadget.test.rs"]
mod tests;
