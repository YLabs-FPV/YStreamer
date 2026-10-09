use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use super::Frame;

const MAX_PAYLOAD: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    src: u8,
    dst: u8,
    response: bool,
    cmd_set: u8,
    cmd_id: u8,
}

struct Entry {
    count: u64,
    changes: u64,
    len: usize,
    payload: Vec<u8>,
    last_seen: Instant,
    last_change: Instant,
}

#[derive(serde::Serialize)]
pub struct FrameStats {
    src: u8,
    dst: u8,
    response: bool,
    cmd_set: u8,
    cmd_id: u8,
    count: u64,
    changes: u64,
    /// Full payload length; `payload` may be truncated
    len: usize,
    payload: String,
    seen_ms_ago: u64,
    changed_ms_ago: u64,
}

pub struct Monitor {
    entries: Mutex<HashMap<Key, Entry>>,
}

impl Monitor {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn record(&self, f: &Frame) {
        let key = Key {
            src: f.src,
            dst: f.dst,
            response: f.is_response(),
            cmd_set: f.cmd_set,
            cmd_id: f.cmd_id,
        };
        let head = &f.payload[..f.payload.len().min(MAX_PAYLOAD)];
        let now = Instant::now();
        let mut entries = self.entries.lock().unwrap();
        let e = entries.entry(key).or_insert_with(|| Entry {
            count: 0,
            changes: 0,
            len: f.payload.len(),
            payload: head.to_vec(),
            last_seen: now,
            last_change: now,
        });
        e.count += 1;
        e.last_seen = now;
        if e.payload != head || e.len != f.payload.len() {
            e.changes += 1;
            e.last_change = now;
            e.len = f.payload.len();
            e.payload.clear();
            e.payload.extend_from_slice(head);
        }
    }

    pub fn snapshot(&self) -> Vec<FrameStats> {
        let now = Instant::now();
        let entries = self.entries.lock().unwrap();
        let mut out: Vec<FrameStats> = entries
            .iter()
            .map(|(k, e)| FrameStats {
                src: k.src,
                dst: k.dst,
                response: k.response,
                cmd_set: k.cmd_set,
                cmd_id: k.cmd_id,
                count: e.count,
                changes: e.changes,
                len: e.len,
                payload: hex::encode(&e.payload),
                seen_ms_ago: (now - e.last_seen).as_millis() as u64,
                changed_ms_ago: (now - e.last_change).as_millis() as u64,
            })
            .collect();
        out.sort_by_key(|s| (s.cmd_set, s.cmd_id, s.src, s.dst, s.response));
        out
    }

    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }
}
