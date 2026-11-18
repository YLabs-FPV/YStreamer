use std::io;
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::thread;
use std::time::Duration;

/// A cable going in bounces, and TVs drop hot-plug while they wake up:
/// act once things have been quiet this long
const SETTLE: Duration = Duration::from_millis(500);
/// The kernel's multicast group, not udev's rebroadcast of it
const KERNEL_GROUP: u32 = 1;

/// Call `on_change` after a burst of DRM hot-plug events has settled
pub fn watch(on_change: impl Fn() + Send + 'static) -> io::Result<()> {
    let socket = open()?;
    thread::Builder::new()
        .name("hdmi-hotplug".into())
        .spawn(move || {
            let mut buf = vec![0u8; 8192];
            loop {
                match recv(&socket, &mut buf) {
                    Ok(n) if is_drm_hotplug(&buf[..n]) => {}
                    Ok(_) => continue,
                    // Events were lost; one of them may have been ours
                    Err(e) if e.raw_os_error() == Some(libc::ENOBUFS) => {}
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        eprintln!("[hdmi] hot-plug events: {e}");
                        thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                }
                // Whatever else arrives meanwhile is covered by this one
                while readable(&socket, SETTLE) {
                    let _ = recv(&socket, &mut buf);
                }
                on_change();
            }
        })?;
    Ok(())
}

fn open() -> io::Result<OwnedFd> {
    // SAFETY: plain syscalls; the fd is owned as soon as it exists
    unsafe {
        let fd = libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            libc::NETLINK_KOBJECT_UEVENT,
        );
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let socket = OwnedFd::from_raw_fd(fd);
        let mut addr: libc::sockaddr_nl = mem::zeroed();
        addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        addr.nl_groups = KERNEL_GROUP;
        if libc::bind(
            socket.as_raw_fd(),
            &addr as *const libc::sockaddr_nl as *const libc::sockaddr,
            mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        ) < 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(socket)
    }
}

fn recv(socket: &OwnedFd, buf: &mut [u8]) -> io::Result<usize> {
    // SAFETY: writes at most buf.len() bytes into buf
    let n = unsafe { libc::recv(socket.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

fn readable(socket: &OwnedFd, within: Duration) -> bool {
    let mut pfd = libc::pollfd {
        fd: socket.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one pollfd, owned here
    unsafe { libc::poll(&mut pfd, 1, within.as_millis() as libc::c_int) > 0 }
}

/// `change@/devices/...\0ACTION=change\0SUBSYSTEM=drm\0HOTPLUG=1\0...`
fn is_drm_hotplug(msg: &[u8]) -> bool {
    let mut drm = false;
    let mut hotplug = false;
    for field in msg.split(|&b| b == 0) {
        match field {
            b"SUBSYSTEM=drm" => drm = true,
            b"HOTPLUG=1" => hotplug = true,
            _ => {}
        }
    }
    drm && hotplug
}

#[cfg(test)]
#[path = "hotplug.test.rs"]
mod tests;
