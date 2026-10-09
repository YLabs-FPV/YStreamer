//! Nothing mounts a USB drive on a headless system, so a plugged-in one
//! wouldn't be there to record to. This mounts them, through udisks, which
//! picks the mount point (under /media) and options, and cleans up after
//! the drive is pulled

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;

use crate::net::network::run;

const FILESYSTEMS: [&str; 7] = ["vfat", "exfat", "ext4", "ext3", "ext2", "ntfs", "f2fs"];

#[derive(Deserialize)]
struct Listing {
    blockdevices: Vec<Block>,
}

#[derive(Deserialize)]
struct Block {
    path: String,
    fstype: Option<String>,
    rm: bool,
    hotplug: bool,
    mountpoint: Option<String>,
    #[serde(default)]
    children: Vec<Block>,
}

/// From `lsblk -J`: the partitions of plugged-in drives that hold a
/// filesystem and aren't mounted
fn unmounted(listing: &str) -> Vec<String> {
    fn walk(blocks: &[Block], out: &mut Vec<String>) {
        for b in blocks {
            let usable = b
                .fstype
                .as_deref()
                .is_some_and(|fs| FILESYSTEMS.contains(&fs));
            // The card the system runs from also counts as hot-pluggable
            let external = (b.rm || b.hotplug) && !b.path.starts_with("/dev/mmcblk");
            if usable && external && b.mountpoint.is_none() {
                out.push(b.path.clone());
            }
            walk(&b.children, out);
        }
    }
    let mut out = Vec::new();
    if let Ok(listing) = serde_json::from_str::<Listing>(listing) {
        walk(&listing.blockdevices, &mut out);
    }
    out
}

// A drive that won't mount is tried once, not every few seconds, and one
// that was ejected stays so until it's pulled
static LEFT_ALONE: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Unmounts a drive so it can be pulled without losing what was written
pub async fn eject(device: &str) -> Result<(), String> {
    run(
        "udisksctl",
        &["unmount", "--no-user-interaction", "-b", device],
    )
    .await?;
    LEFT_ALONE
        .lock()
        .unwrap()
        .get_or_insert_default()
        .insert(device.to_string());
    println!("[drives] ejected {device}");
    Ok(())
}

pub async fn mount_plugged_in() {
    let Ok(listing) = run("lsblk", &["-J", "-o", "PATH,FSTYPE,RM,HOTPLUG,MOUNTPOINT"]).await else {
        return;
    };
    let waiting = unmounted(&listing);
    let todo: Vec<String> = {
        let mut left_alone = LEFT_ALONE.lock().unwrap();
        let left_alone = left_alone.get_or_insert_default();
        left_alone.retain(|path| waiting.contains(path));
        waiting
            .into_iter()
            .filter(|p| !left_alone.contains(p))
            .collect()
    };
    for path in todo {
        match run(
            "udisksctl",
            &["mount", "--no-user-interaction", "-b", &path],
        )
        .await
        {
            Ok(_) => println!("[drives] mounted {path}"),
            Err(e) => {
                eprintln!("[drives] {path}: {e}");
                LEFT_ALONE
                    .lock()
                    .unwrap()
                    .get_or_insert_default()
                    .insert(path);
            }
        }
    }
}

/// Keeps looking, so a drive is ready to record to without anyone opening
/// the settings page
pub async fn watch() {
    loop {
        mount_plugged_in().await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
#[path = "drives.test.rs"]
mod tests;
