//! The NDI runtime, opened when it's installed. It's NDI's own library
//! under NDI's own licence, so YStreamer neither ships it nor links to it:
//! the owner installs it, and it's looked up by name at run time

use std::ffi::{CString, c_char, c_int, c_void};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use libloading::Library;

/// The runtime's names, newest first. Both versions take the same calls
pub(super) const LIBRARIES: [&str; 2] = ["libndi.so.6", "libndi.so.5"];
/// Where NDI's installers point to the runtime, overriding the usual search
const DIR_VARIABLES: [&str; 2] = ["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5"];
/// The NDI SDK's install script puts the libraries here, which isn't always
/// on the loader's path
const EXTRA_DIRS: [&str; 1] = ["/usr/local/lib"];

/// `NDI_LIB_FOURCC('N', 'V', '1', '2')`
pub const FOURCC_NV12: u32 = u32::from_le_bytes(*b"NV12");
pub const FRAME_PROGRESSIVE: c_int = 1;

/// `NDIlib_send_create_t`
#[repr(C)]
struct SendCreate {
    p_ndi_name: *const c_char,
    p_groups: *const c_char,
    clock_video: bool,
    clock_audio: bool,
}

/// `NDIlib_video_frame_v2_t`
#[repr(C)]
pub struct VideoFrame {
    pub xres: c_int,
    pub yres: c_int,
    pub four_cc: u32,
    pub frame_rate_n: c_int,
    pub frame_rate_d: c_int,
    pub picture_aspect_ratio: f32,
    pub frame_format_type: c_int,
    /// 100 ns units
    pub timecode: i64,
    pub p_data: *const u8,
    pub line_stride_in_bytes: c_int,
    pub p_metadata: *const c_char,
    pub timestamp: i64,
}

type Instance = *mut c_void;

pub struct Runtime {
    /// Never closed: the functions below point into it
    _library: Library,
    pub path: String,
    initialize: unsafe extern "C" fn() -> bool,
    /// Whether NDI started, once a source is first made
    started: OnceLock<bool>,
    send_create: unsafe extern "C" fn(*const SendCreate) -> Instance,
    send_destroy: unsafe extern "C" fn(Instance),
    send_video: unsafe extern "C" fn(Instance, *const VideoFrame),
    connections: unsafe extern "C" fn(Instance, u32) -> c_int,
}

static LOADED: Mutex<Option<&'static Runtime>> = Mutex::new(None);

/// The runtime, opened on first use. Not finding it isn't remembered, so
/// installing it takes effect without a restart
pub fn load() -> Result<&'static Runtime, String> {
    let mut loaded = LOADED.lock().unwrap();
    if let Some(runtime) = *loaded {
        return Ok(runtime);
    }
    let runtime: &'static Runtime = Box::leak(Box::new(open()?));
    eprintln!("[ndi] using the NDI runtime at {}", runtime.path);
    *loaded = Some(runtime);
    Ok(runtime)
}

fn candidates() -> Vec<String> {
    let mut list = Vec::new();
    for var in DIR_VARIABLES {
        if let Some(dir) = std::env::var_os(var) {
            for name in LIBRARIES {
                list.push(PathBuf::from(&dir).join(name).display().to_string());
            }
        }
    }
    // Bare names go through the loader's own search
    list.extend(LIBRARIES.iter().map(|n| n.to_string()));
    for dir in EXTRA_DIRS {
        list.extend(LIBRARIES.iter().map(|n| format!("{dir}/{n}")));
    }
    list
}

fn open() -> Result<Runtime, String> {
    let mut problem = None;
    for path in candidates() {
        // Safety: the NDI runtime's initialisers have no requirements
        let library = match unsafe { Library::new(&path) } {
            Ok(library) => library,
            Err(_) => continue,
        };
        match bind(library, &path) {
            Ok(runtime) => return Ok(runtime),
            Err(e) => problem = Some(e),
        }
    }
    Err(problem
        .unwrap_or_else(|| "The NDI runtime isn't installed: libndi.so.6 wasn't found".to_string()))
}

fn bind(library: Library, path: &str) -> Result<Runtime, String> {
    // Safety: the types match the declarations in NDI's headers
    unsafe {
        macro_rules! get {
            ($name:literal) => {
                *library
                    .get($name)
                    .map_err(|e| format!("{path} isn't a usable NDI runtime: {e}"))?
            };
        }
        let supported: unsafe extern "C" fn() -> bool = get!(b"NDIlib_is_supported_CPU\0");
        let runtime = Runtime {
            initialize: get!(b"NDIlib_initialize\0"),
            started: OnceLock::new(),
            send_create: get!(b"NDIlib_send_create\0"),
            send_destroy: get!(b"NDIlib_send_destroy\0"),
            send_video: get!(b"NDIlib_send_send_video_v2\0"),
            connections: get!(b"NDIlib_send_get_no_connections\0"),
            path: path.to_string(),
            _library: library,
        };
        if !supported() {
            return Err("The NDI runtime doesn't support this processor".into());
        }
        Ok(runtime)
    }
}

/// One NDI source on the network
pub struct Sender {
    runtime: &'static Runtime,
    instance: Instance,
}

// NDI's senders may be queried from one thread while another sends
unsafe impl Send for Sender {}
unsafe impl Sync for Sender {}

impl Sender {
    pub fn new(runtime: &'static Runtime, name: &str) -> Result<Self, String> {
        // Started on first use rather than when found, so a device with NDI
        // off only ever looks at the library
        // Safety: NDI allows initialising before anything else
        if !*runtime
            .started
            .get_or_init(|| unsafe { (runtime.initialize)() })
        {
            return Err("The NDI runtime didn't start".into());
        }
        let name = CString::new(name).map_err(|_| "NDI source name can't contain NUL")?;
        let create = SendCreate {
            p_ndi_name: name.as_ptr(),
            p_groups: std::ptr::null(),
            // Pictures go out as they're decoded, not paced by NDI
            clock_video: false,
            clock_audio: false,
        };
        // Safety: `create` and the name it points to outlive the call
        let instance = unsafe { (runtime.send_create)(&create) };
        if instance.is_null() {
            return Err("The NDI runtime couldn't create the source".into());
        }
        Ok(Self { runtime, instance })
    }

    /// Receivers connected right now
    pub fn connections(&self) -> u32 {
        // Safety: the instance is alive until drop
        unsafe { (self.runtime.connections)(self.instance, 0) }.max(0) as u32
    }

    /// Returns once NDI is done with the frame's data
    pub fn send(&self, frame: &VideoFrame) {
        // Safety: the synchronous send doesn't keep the pointers
        unsafe { (self.runtime.send_video)(self.instance, frame) }
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        // Safety: created by send_create and not used after this
        unsafe { (self.runtime.send_destroy)(self.instance) }
    }
}
