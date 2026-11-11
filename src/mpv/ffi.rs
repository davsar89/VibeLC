//! The subset of libmpv's C API that VibeLC uses, loaded at run time, or with the `static-mpv`
//! feature linked into the program.
//!
//! Loading at run time means no C headers or import libraries are needed to build on any OS,
//! and a missing library becomes a clear message instead of a loader error. The declarations
//! follow `mpv/client.h`, `mpv/render.h` and `mpv/render_gl.h`; every type and function used
//! here exists with the same layout in client API 1.109 (mpv 0.34) and 2.x (mpv 0.35+), except
//! `mpv_opengl_init_params`, see below.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_double, c_int, c_ulong, c_void};
use std::path::PathBuf;

use libloading::Library;

/// Opaque `mpv_handle`.
#[repr(C)]
pub struct mpv_handle {
    _private: [u8; 0],
}

/// Opaque `mpv_render_context`.
#[repr(C)]
pub struct mpv_render_context {
    _private: [u8; 0],
}

pub type mpv_format = c_int;
pub const MPV_FORMAT_NONE: mpv_format = 0;
pub const MPV_FORMAT_STRING: mpv_format = 1;
pub const MPV_FORMAT_FLAG: mpv_format = 3;
pub const MPV_FORMAT_INT64: mpv_format = 4;
pub const MPV_FORMAT_DOUBLE: mpv_format = 5;
pub const MPV_FORMAT_NODE: mpv_format = 6;
pub const MPV_FORMAT_NODE_ARRAY: mpv_format = 7;
pub const MPV_FORMAT_NODE_MAP: mpv_format = 8;

pub type mpv_event_id = c_int;
pub const MPV_EVENT_NONE: mpv_event_id = 0;
pub const MPV_EVENT_SHUTDOWN: mpv_event_id = 1;
pub const MPV_EVENT_LOG_MESSAGE: mpv_event_id = 2;
pub const MPV_EVENT_GET_PROPERTY_REPLY: mpv_event_id = 3;
pub const MPV_EVENT_SET_PROPERTY_REPLY: mpv_event_id = 4;
pub const MPV_EVENT_COMMAND_REPLY: mpv_event_id = 5;
pub const MPV_EVENT_START_FILE: mpv_event_id = 6;
pub const MPV_EVENT_END_FILE: mpv_event_id = 7;
pub const MPV_EVENT_FILE_LOADED: mpv_event_id = 8;
pub const MPV_EVENT_PROPERTY_CHANGE: mpv_event_id = 22;

pub const MPV_END_FILE_REASON_EOF: c_int = 0;
pub const MPV_END_FILE_REASON_STOP: c_int = 2;
pub const MPV_END_FILE_REASON_QUIT: c_int = 3;
pub const MPV_END_FILE_REASON_ERROR: c_int = 4;
pub const MPV_END_FILE_REASON_REDIRECT: c_int = 5;

#[repr(C)]
#[derive(Clone, Copy)]
pub union mpv_node_u {
    pub string: *mut c_char,
    pub flag: c_int,
    pub int64: i64,
    pub double_: c_double,
    pub list: *mut mpv_node_list,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct mpv_node {
    pub u: mpv_node_u,
    pub format: mpv_format,
}

#[repr(C)]
pub struct mpv_node_list {
    pub num: c_int,
    pub values: *mut mpv_node,
    pub keys: *mut *mut c_char,
}

#[repr(C)]
pub struct mpv_event {
    pub event_id: mpv_event_id,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_event_property {
    pub name: *const c_char,
    pub format: mpv_format,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_event_log_message {
    pub prefix: *const c_char,
    pub level: *const c_char,
    pub text: *const c_char,
    pub log_level: c_int,
}

/// Only the two leading fields, which every supported API version has. Later versions append
/// playlist ids, which older libraries do not allocate, so they must never be read.
#[repr(C)]
pub struct mpv_event_end_file {
    pub reason: c_int,
    pub error: c_int,
}

pub type mpv_render_param_type = c_int;
pub const MPV_RENDER_PARAM_INVALID: mpv_render_param_type = 0;
pub const MPV_RENDER_PARAM_API_TYPE: mpv_render_param_type = 1;
pub const MPV_RENDER_PARAM_OPENGL_INIT_PARAMS: mpv_render_param_type = 2;
pub const MPV_RENDER_PARAM_OPENGL_FBO: mpv_render_param_type = 3;
pub const MPV_RENDER_PARAM_FLIP_Y: mpv_render_param_type = 4;
pub const MPV_RENDER_PARAM_X11_DISPLAY: mpv_render_param_type = 8;
pub const MPV_RENDER_PARAM_WL_DISPLAY: mpv_render_param_type = 9;
pub const MPV_RENDER_PARAM_ADVANCED_CONTROL: mpv_render_param_type = 10;
pub const MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME: mpv_render_param_type = 12;
pub const MPV_RENDER_PARAM_SKIP_RENDERING: mpv_render_param_type = 13;
/// `render_d3d11.h`, client API 2.6 (mpv 0.42).
#[cfg(windows)]
pub const MPV_RENDER_PARAM_D3D11_INIT_PARAMS: mpv_render_param_type = 21;
pub const MPV_RENDER_PARAM_D3D11_FBO: mpv_render_param_type = 22;

/// `ID3D11Device*`; mpv takes its own reference.
#[cfg(windows)]
#[repr(C)]
pub struct mpv_d3d11_init_params {
    pub device: *mut c_void,
}

/// `ID3D11Texture2D*` with render-target binding, on that device.
#[repr(C)]
pub struct mpv_d3d11_fbo {
    pub tex: *mut c_void,
}

pub const MPV_RENDER_UPDATE_FRAME: u64 = 1;

#[repr(C)]
pub struct mpv_render_param {
    pub type_: mpv_render_param_type,
    pub data: *mut c_void,
}

pub type GetProcAddressFn = unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char) -> *mut c_void;

/// Client API 1.x has a third, deprecated `extra_exts` field that 2.x removed. Keeping it (always
/// null) is correct for both: 1.x reads it as "none", 2.x never reads past the second field.
#[repr(C)]
pub struct mpv_opengl_init_params {
    pub get_proc_address: Option<GetProcAddressFn>,
    pub get_proc_address_ctx: *mut c_void,
    pub extra_exts: *const c_char,
}

#[repr(C)]
pub struct mpv_opengl_fbo {
    pub fbo: c_int,
    pub w: c_int,
    pub h: c_int,
    pub internal_format: c_int,
}

pub type WakeupFn = unsafe extern "C" fn(d: *mut c_void);

/// Oldest supported client API: 1.109 is mpv 0.34 (Ubuntu 22.04), the first version with
/// everything VibeLC relies on (`video-margin-ratio-*`, `stop keep-playlist`,
/// `playlist-play-index`, `auto-copy-safe`).
pub const MIN_API_VERSION: c_ulong = (1 << 16) | 109;

/// Function table. Field names are the C names without the `mpv_` prefix.
pub struct Api {
    pub client_api_version: unsafe extern "C" fn() -> c_ulong,
    pub error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    pub create: unsafe extern "C" fn() -> *mut mpv_handle,
    pub initialize: unsafe extern "C" fn(*mut mpv_handle) -> c_int,
    pub terminate_destroy: unsafe extern "C" fn(*mut mpv_handle),
    pub set_option_string: unsafe extern "C" fn(*mut mpv_handle, *const c_char, *const c_char) -> c_int,
    pub command: unsafe extern "C" fn(*mut mpv_handle, *mut *const c_char) -> c_int,
    pub command_async: unsafe extern "C" fn(*mut mpv_handle, u64, *mut *const c_char) -> c_int,
    pub set_property_async: unsafe extern "C" fn(*mut mpv_handle, u64, *const c_char, mpv_format, *mut c_void) -> c_int,
    pub get_property: unsafe extern "C" fn(*mut mpv_handle, *const c_char, mpv_format, *mut c_void) -> c_int,
    pub get_property_async: unsafe extern "C" fn(*mut mpv_handle, u64, *const c_char, mpv_format) -> c_int,
    pub observe_property: unsafe extern "C" fn(*mut mpv_handle, u64, *const c_char, mpv_format) -> c_int,
    pub request_log_messages: unsafe extern "C" fn(*mut mpv_handle, *const c_char) -> c_int,
    pub wait_event: unsafe extern "C" fn(*mut mpv_handle, c_double) -> *mut mpv_event,
    pub set_wakeup_callback: unsafe extern "C" fn(*mut mpv_handle, Option<WakeupFn>, *mut c_void),
    pub free_node_contents: unsafe extern "C" fn(*mut mpv_node),
    pub render_context_create:
        unsafe extern "C" fn(*mut *mut mpv_render_context, *mut mpv_handle, *mut mpv_render_param) -> c_int,
    pub render_context_set_update_callback:
        unsafe extern "C" fn(*mut mpv_render_context, Option<WakeupFn>, *mut c_void),
    pub render_context_update: unsafe extern "C" fn(*mut mpv_render_context) -> u64,
    pub render_context_render: unsafe extern "C" fn(*mut mpv_render_context, *mut mpv_render_param) -> c_int,
    pub render_context_report_swap: unsafe extern "C" fn(*mut mpv_render_context),
    pub render_context_free: unsafe extern "C" fn(*mut mpv_render_context),
    /// Keeps the function pointers above valid (none when libmpv is linked in).
    _lib: Option<Library>,
}

/// Calls `$m!` with every function of [`Api`] as `field: "c_name"`.
macro_rules! functions {
    ($m:ident) => {
        $m! {
            client_api_version: "mpv_client_api_version",
            error_string: "mpv_error_string",
            create: "mpv_create",
            initialize: "mpv_initialize",
            terminate_destroy: "mpv_terminate_destroy",
            set_option_string: "mpv_set_option_string",
            command: "mpv_command",
            command_async: "mpv_command_async",
            set_property_async: "mpv_set_property_async",
            get_property: "mpv_get_property",
            get_property_async: "mpv_get_property_async",
            observe_property: "mpv_observe_property",
            request_log_messages: "mpv_request_log_messages",
            wait_event: "mpv_wait_event",
            set_wakeup_callback: "mpv_set_wakeup_callback",
            free_node_contents: "mpv_free_node_contents",
            render_context_create: "mpv_render_context_create",
            render_context_set_update_callback: "mpv_render_context_set_update_callback",
            render_context_update: "mpv_render_context_update",
            render_context_render: "mpv_render_context_render",
            render_context_report_swap: "mpv_render_context_report_swap",
            render_context_free: "mpv_render_context_free",
        }
    };
}

/// libmpv linked into the program (`static-mpv`). Declared without their parameters: [`Api`]'s
/// fields carry the prototypes, and [`linked`] gives each address the field's type.
#[cfg(feature = "static-mpv")]
mod linked {
    macro_rules! declare {
        ($($field:ident: $c:literal,)*) => {
            unsafe extern "C" {
                $(#[link_name = $c] pub fn $field();)*
            }
        };
    }
    functions!(declare);
}

#[cfg(feature = "static-mpv")]
fn linked() -> Api {
    macro_rules! table {
        ($($field:ident: $c:literal,)*) => {
            Api {
                // SAFETY: the field's type matches the C prototype in libmpv's headers.
                $($field: unsafe { std::mem::transmute::<unsafe extern "C" fn(), _>(linked::$field) },)*
                _lib: None,
            }
        };
    }
    functions!(table)
}

/// Why libmpv could not be used.
#[derive(Debug)]
pub enum LoadError {
    /// No candidate file could be opened; carries what was tried and the last loader message.
    NotFound { tried: Vec<String>, last_error: String },
    /// The library lacks a function VibeLC needs.
    MissingSymbol { library: String, symbol: String },
    /// The library is older than [`MIN_API_VERSION`].
    TooOld { library: String, version: c_ulong },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::NotFound { tried, last_error } => {
                write!(f, "libmpv was not found (tried {}): {last_error}.\n\n{}", tried.join(", "), install_hint())
            }
            LoadError::MissingSymbol { library, symbol } => {
                write!(f, "{library} does not provide {symbol}; it is not a usable libmpv.\n\n{}", install_hint())
            }
            LoadError::TooOld { library, version } => write!(
                f,
                "{library} has client API {}.{}, but VibeLC needs {}.{} (mpv 0.34) or newer.\n\n{}",
                version >> 16,
                version & 0xffff,
                MIN_API_VERSION >> 16,
                MIN_API_VERSION & 0xffff,
                install_hint()
            ),
        }
    }
}

impl std::error::Error for LoadError {}

fn install_hint() -> &'static str {
    if cfg!(windows) {
        "Put libmpv-2.dll (from an mpv \"dev\" build for Windows) next to vibelc.exe."
    } else {
        "Install the VibeLC package, which includes it, or your distribution's libmpv \
         (sudo apt install libmpv2, or libmpv1 on Ubuntu 22.04)."
    }
}

/// Files to try, most specific first. `VIBELC_LIBMPV` overrides everything; then comes the copy
/// shipped with VibeLC (next to vibelc.exe, or in `/usr/lib/vibelc` for the .deb), by absolute
/// path so that the current directory can never supply one; then the system's.
fn candidates() -> Vec<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["libmpv-2.dll", "mpv-2.dll", "mpv-1.dll"]
    } else {
        &["libmpv.so.2", "libmpv.so.1", "libmpv.so"]
    };
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from));
    let bundled = if cfg!(windows) { exe_dir } else { exe_dir.map(|d| d.join("../lib/vibelc")) };
    let mut out: Vec<PathBuf> = std::env::var_os("VIBELC_LIBMPV").map(PathBuf::from).into_iter().collect();
    out.extend(bundled.iter().flat_map(|dir| names.iter().map(|n| dir.join(n))));
    out.extend(names.iter().map(PathBuf::from));
    out
}

impl Api {
    /// Opens the first libmpv that loads and checks that it is recent enough.
    pub fn load() -> Result<Api, LoadError> {
        #[cfg(feature = "static-mpv")]
        if std::env::var_os("VIBELC_LIBMPV").is_none() {
            log::info!("using the libmpv linked into vibelc");
            return Ok(linked());
        }
        let mut tried = Vec::new();
        let mut last_error = String::from("no candidates");
        for path in candidates() {
            let shown = path.display().to_string();
            // SAFETY: loading libmpv runs its initializers, which have no preconditions.
            match unsafe { Library::new(&path) } {
                Ok(lib) => {
                    log::info!("loaded {shown}");
                    return Api::from_library(lib, shown);
                }
                Err(e) => {
                    tried.push(shown);
                    last_error = e.to_string();
                }
            }
        }
        Err(LoadError::NotFound { tried, last_error })
    }

    fn from_library(lib: Library, name: String) -> Result<Api, LoadError> {
        macro_rules! sym {
            ($c:literal) => {{
                let symbol = $c;
                // SAFETY: the declared type matches the C prototype in libmpv's headers.
                match unsafe { lib.get(symbol) } {
                    Ok(s) => *s,
                    Err(_) => {
                        return Err(LoadError::MissingSymbol { library: name, symbol: symbol.to_string() });
                    }
                }
            }};
        }
        let client_api_version: unsafe extern "C" fn() -> c_ulong = sym!("mpv_client_api_version");
        // SAFETY: takes no arguments and only returns a constant.
        let version = unsafe { client_api_version() };
        if version < MIN_API_VERSION {
            return Err(LoadError::TooOld { library: name, version });
        }
        macro_rules! table {
            ($($field:ident: $c:literal,)*) => {
                Api { $($field: sym!($c),)* _lib: Some(lib) }
            };
        }
        Ok(functions!(table))
    }

    /// "major.minor" of the loaded client API.
    pub fn version_string(&self) -> String {
        // SAFETY: takes no arguments and only returns a constant.
        let v = unsafe { (self.client_api_version)() };
        format!("{}.{}", v >> 16, v & 0xffff)
    }
}
