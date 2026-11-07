//! Reads from a FunctionFS endpoint with several reads queued in the kernel
//! at once, through Linux AIO. A plain `read` has one at a time: between
//! two, nothing on our side takes data, and a remote that streams without
//! waiting (an RC-N does) drops what it couldn't send. Android's accessory
//! driver, which DJI Fly reads from, keeps reads queued the same way

use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;

/// `struct iocb` from `linux/aio_abi.h`, little-endian layout
#[repr(C)]
#[derive(Default)]
struct Iocb {
    aio_data: u64,
    aio_key: u32,
    aio_rw_flags: i32,
    aio_lio_opcode: u16,
    aio_reqprio: i16,
    aio_fildes: u32,
    aio_buf: u64,
    aio_nbytes: u64,
    aio_offset: i64,
    aio_reserved2: u64,
    aio_flags: u32,
    aio_resfd: u32,
}

/// `struct io_event`
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct IoEvent {
    data: u64,
    obj: u64,
    res: i64,
    res2: i64,
}

const IOCB_CMD_PREAD: u16 = 0;

struct Slot {
    buf: Box<[u8]>,
    iocb: Box<Iocb>,
    /// Order the read was queued in, so data comes out in order
    seq: u64,
}

pub struct QueuedReader {
    // Declared first, so the context goes (and with it every read still
    // queued) before the buffers the kernel writes into
    ctx: Context,
    slots: Vec<Slot>,
    file: File,
    queued: u64,
    delivered: u64,
    /// Finished reads waiting for the ones queued before them: seq to
    /// (slot, result)
    done: BTreeMap<u64, (usize, i64)>,
}

struct Context(libc::c_ulong);

impl Drop for Context {
    fn drop(&mut self) {
        // Cancels what's queued and waits for it, so no read writes into
        // freed buffers
        unsafe { libc::syscall(libc::SYS_io_destroy, self.0) };
    }
}

impl QueuedReader {
    /// `depth` reads of `size` bytes each, all queued at once
    pub fn new(file: File, depth: usize, size: usize) -> io::Result<Self> {
        let mut ctx: libc::c_ulong = 0;
        if unsafe { libc::syscall(libc::SYS_io_setup, depth as libc::c_long, &mut ctx) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let ctx = Context(ctx);
        let slots = (0..depth)
            .map(|_| Slot {
                buf: vec![0u8; size].into_boxed_slice(),
                iocb: Box::default(),
                seq: 0,
            })
            .collect();
        let mut reader = Self {
            ctx,
            slots,
            file,
            queued: 0,
            delivered: 0,
            done: BTreeMap::new(),
        };
        for i in 0..depth {
            reader.submit(i)?;
        }
        Ok(reader)
    }

    fn submit(&mut self, i: usize) -> io::Result<()> {
        let fd = self.file.as_raw_fd() as u32;
        let slot = &mut self.slots[i];
        slot.seq = self.queued;
        *slot.iocb = Iocb {
            aio_data: i as u64,
            aio_lio_opcode: IOCB_CMD_PREAD,
            aio_fildes: fd,
            aio_buf: slot.buf.as_mut_ptr() as u64,
            aio_nbytes: slot.buf.len() as u64,
            ..Iocb::default()
        };
        let mut list = [&mut *slot.iocb as *mut Iocb];
        let n = unsafe { libc::syscall(libc::SYS_io_submit, self.ctx.0, 1, list.as_mut_ptr()) };
        if n != 1 {
            return Err(io::Error::last_os_error());
        }
        self.queued += 1;
        Ok(())
    }

    /// The next read to finish, in the order they were queued: its bytes
    /// (none for a zero-length packet), or why the link stopped
    pub fn next(&mut self) -> io::Result<Vec<u8>> {
        loop {
            if let Some((i, res)) = self.done.remove(&self.delivered) {
                self.delivered += 1;
                if res < 0 {
                    let e = io::Error::from_raw_os_error(-res as i32);
                    // A read that couldn't happen just then isn't the link
                    // closing; anything else is, and isn't queued again
                    if matches!(
                        e.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) {
                        self.submit(i)?;
                        continue;
                    }
                    return Err(e);
                }
                let data = self.slots[i].buf[..res as usize].to_vec();
                self.submit(i)?;
                return Ok(data);
            }
            let mut events = [IoEvent::default(); 16];
            let n = unsafe {
                libc::syscall(
                    libc::SYS_io_getevents,
                    self.ctx.0,
                    1 as libc::c_long,
                    events.len() as libc::c_long,
                    events.as_mut_ptr(),
                    std::ptr::null_mut::<libc::timespec>(),
                )
            };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            for event in &events[..n as usize] {
                let i = event.data as usize;
                self.done.insert(self.slots[i].seq, (i, event.res));
            }
        }
    }
}

#[cfg(test)]
#[path = "aio.test.rs"]
mod tests;
