use std::ffi::CStr;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc as std_mpsc;
use std::thread;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

const REGULAR_UID: u32 = 1000;

#[derive(Serialize)]
pub struct Info {
    /// Accounts a shell can be opened as, the default first
    pub users: Vec<String>,
}

#[derive(Deserialize)]
struct Resize {
    cols: u16,
    rows: u16,
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

fn user_name(uid: u32) -> Option<String> {
    let entry = unsafe { libc::getpwuid(uid) };
    if entry.is_null() {
        return None;
    }
    let name = unsafe { CStr::from_ptr((*entry).pw_name) };
    Some(name.to_string_lossy().into_owned())
}

/// Root can become anyone; anyone else is only themselves
pub fn info() -> Info {
    let users = if is_root() {
        let mut users = vec!["root".to_string()];
        users.extend(user_name(REGULAR_UID));
        users
    } else {
        user_name(unsafe { libc::geteuid() }).into_iter().collect()
    };
    Info { users }
}

struct Shell {
    master: OwnedFd,
    child: Child,
}

fn set_size(master: &OwnedFd, cols: u16, rows: u16) {
    let size = libc::winsize {
        ws_row: rows.clamp(2, 500),
        ws_col: cols.clamp(2, 1000),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &size) };
}

fn open_shell(user: &str, cols: u16, rows: u16) -> std::io::Result<Shell> {
    let (mut master, mut slave) = (0, 0);
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if opened != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    set_size(&master, cols, rows);

    // A login shell as that account, with its own environment and home
    let mut command = if is_root() && user != "root" {
        let mut su = Command::new("su");
        su.args(["-", user]);
        su
    } else {
        let mut shell = Command::new("/bin/bash");
        shell.arg("-l");
        if let Some(home) = std::env::var_os("HOME") {
            shell.current_dir(home);
        }
        shell
    };
    command
        .env("TERM", "xterm-256color")
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave));
    unsafe {
        command.pre_exec(|| {
            // Its own session with the pty as controlling terminal, so
            // Ctrl+C and job control reach the shell and not this service
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    Ok(Shell { master, child })
}

/// Runs until the browser leaves or the shell exits
pub async fn serve(mut socket: WebSocket, user: String, cols: u16, rows: u16) {
    let shell = match open_shell(&user, cols, rows) {
        Ok(shell) => shell,
        Err(e) => {
            let note = format!("Couldn't start a shell: {e}\r\n");
            let _ = socket.send(Message::Binary(note.into_bytes().into())).await;
            return;
        }
    };
    let Shell { master, mut child } = shell;
    let pid = child.id() as i32;
    println!("[terminal] shell opened as {user}");

    // The pty is read and written with blocking calls, on threads of their own
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
    let (in_tx, in_rx) = std_mpsc::channel::<Vec<u8>>();
    let files = master
        .try_clone()
        .and_then(|a| Ok((File::from(a), File::from(master.try_clone()?))));
    let Ok((mut reader, mut writer)) = files else {
        let _ = child.kill();
        return;
    };
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        // Ends with an error once the shell is gone
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || out_tx.blocking_send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    thread::spawn(move || {
        while let Ok(bytes) = in_rx.recv() {
            if writer.write_all(&bytes).is_err() {
                break;
            }
        }
    });

    let (mut sink, mut stream) = socket.split();
    loop {
        tokio::select! {
            output = out_rx.recv() => match output {
                Some(bytes) => {
                    if sink.send(Message::Binary(bytes.into())).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
            message = stream.next() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    if in_tx.send(bytes.to_vec()).is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Text(text))) => {
                    if let Ok(size) = serde_json::from_str::<Resize>(&text) {
                        set_size(&master, size.cols, size.rows);
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => break,
            },
        }
    }

    // Hang up on everything started from the shell, as closing an SSH
    // window does
    unsafe { libc::kill(-pid, libc::SIGHUP) };
    drop(in_tx);
    let _ = sink.close().await;
    thread::spawn(move || {
        let _ = child.wait();
    });
    println!("[terminal] shell closed");
}

#[cfg(test)]
#[path = "terminal.test.rs"]
mod tests;
