use super::*;
use std::time::{Duration, Instant};

#[test]
fn a_shell_runs_commands_and_knows_its_size() {
    let me = info().users.pop().expect("an account to run as");
    let Shell { master, mut child } = open_shell(&me, 132, 43).expect("a shell");
    let mut writer = File::from(master.try_clone().unwrap());
    let mut reader = File::from(master);
    writer
        .write_all(b"echo sum-$((40+2)); stty size; exit\n")
        .unwrap();

    let (tx, rx) = std_mpsc::channel();
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut seen = String::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && !(seen.contains("sum-42") && seen.contains("43 132")) {
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(200)) {
            seen.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(seen.contains("sum-42"), "{seen}");
    assert!(seen.contains("43 132"), "{seen}");
}
