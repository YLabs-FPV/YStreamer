//! VMX1, the codec OMT carries: every picture compressed on its own, in
//! software. libvmx is vendored in `vmx/` (MIT) and built by build.rs for
//! 64-bit ARM, the Pi's processor; elsewhere OMT reports it can't run

use super::super::decoded::Picture;

#[cfg(target_arch = "aarch64")]
mod ffi {
    use std::ffi::{c_int, c_uchar, c_void};

    #[repr(C)]
    pub struct Size {
        pub width: c_int,
        pub height: c_int,
    }

    /// The standard-quality profile OMT senders use by default
    pub const PROFILE_OMT_SQ: c_int = 166;
    pub const COLORSPACE_BT709: c_int = 709;
    pub const OK: c_int = 0;

    unsafe extern "C" {
        pub fn VMX_Create(size: Size, profile: c_int, color_space: c_int) -> *mut c_void;
        pub fn VMX_Destroy(instance: *mut c_void);
        pub fn VMX_SetThreads(instance: *mut c_void, threads: c_int);
        pub fn VMX_EncodeNV12(
            instance: *mut c_void,
            y: *mut c_uchar,
            y_stride: c_int,
            uv: *mut c_uchar,
            uv_stride: c_int,
            interlaced: c_int,
        ) -> c_int;
        pub fn VMX_SaveTo(instance: *mut c_void, out: *mut c_uchar, max_len: c_int) -> c_int;
    }
}

/// An encoder for one picture size
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub struct Encoder {
    #[cfg(target_arch = "aarch64")]
    instance: *mut std::ffi::c_void,
    width: u32,
    height: u32,
    out: Vec<u8>,
}

// Used by one thread at a time
unsafe impl Send for Encoder {}

impl Encoder {
    #[cfg(target_arch = "aarch64")]
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        if !(16..=7680).contains(&width) || !(16..=4320).contains(&height) {
            return Err(format!("VMX can't encode {width}x{height}"));
        }
        let size = ffi::Size {
            width: width as i32,
            height: height as i32,
        };
        // Safety: plain values in, an owned instance or null out
        let instance = unsafe { ffi::VMX_Create(size, ffi::PROFILE_OMT_SQ, ffi::COLORSPACE_BT709) };
        if instance.is_null() {
            return Err(format!("VMX couldn't start at {width}x{height}"));
        }
        // libvmx picks few threads for small pictures. One core stays with
        // the rest of YStreamer: the web interface and the other outputs
        let threads = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .saturating_sub(1)
            .max(1);
        // Safety: the instance was just created
        unsafe { ffi::VMX_SetThreads(instance, threads as i32) };
        Ok(Self {
            instance,
            width,
            height,
            // What libvmx asks for as the most a picture can take
            out: vec![0; width as usize * height as usize * 4],
        })
    }

    #[cfg(not(target_arch = "aarch64"))]
    pub fn new(_width: u32, _height: u32) -> Result<Self, String> {
        Err("OMT's encoder is only built for 64-bit ARM".into())
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The compressed picture, valid until the next call
    #[cfg(target_arch = "aarch64")]
    pub fn encode(&mut self, picture: &Picture) -> Option<&[u8]> {
        if (picture.width, picture.height) != self.size() {
            return None;
        }
        // Safety: the planes cover the picture at their strides, which the
        // decoder guarantees; libvmx only reads them despite the signature
        let n = unsafe {
            let err = ffi::VMX_EncodeNV12(
                self.instance,
                picture.y.as_ptr().cast_mut(),
                picture.y_stride as i32,
                picture.uv.as_ptr().cast_mut(),
                picture.uv_stride as i32,
                0,
            );
            if err != ffi::OK {
                return None;
            }
            ffi::VMX_SaveTo(self.instance, self.out.as_mut_ptr(), self.out.len() as i32)
        };
        (n > 0).then(|| &self.out[..n as usize])
    }

    #[cfg(not(target_arch = "aarch64"))]
    pub fn encode(&mut self, _picture: &Picture) -> Option<&[u8]> {
        None
    }
}

#[cfg(target_arch = "aarch64")]
impl Drop for Encoder {
    fn drop(&mut self) {
        // Safety: created in new and not used after this
        unsafe { ffi::VMX_Destroy(self.instance) }
    }
}
