//! Safe wrapper over the libmpv subset in [`ffi`].
//!
//! [`Mpv`] is one player core: options, commands, properties and events. [`RenderContext`] draws
//! its video with OpenGL into a framebuffer we own. libmpv is thread-safe; callbacks registered
//! here run on mpv's threads and must only signal (wake the UI), never call back into mpv.

pub mod ffi;

use std::ffi::{CStr, CString, OsStr, c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, Once, PoisonError};

pub use ffi::Api;

/// A libmpv error code with its description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: i32,
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for Error {}

/// Owned copy of an `mpv_node` (a property value or command result).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Node {
    #[default]
    None,
    String(String),
    Flag(bool),
    Int(i64),
    Double(f64),
    Array(Vec<Node>),
    Map(Vec<(String, Node)>),
}

impl Node {
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Node::Double(d) => Some(d),
            Node::Int(i) => Some(i as f64),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Node::Int(i) => Some(i),
            Node::Double(d) if d.is_finite() => Some(d as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Node::Flag(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Node::String(s) => Some(s),
            _ => None,
        }
    }

    /// Value of `key` in a map node.
    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Elements of an array node (empty for anything else).
    pub fn items(&self) -> &[Node] {
        match self {
            Node::Array(a) => a,
            _ => &[],
        }
    }

    pub fn str_of(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Node::as_str)
    }

    pub fn f64_of(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Node::as_f64)
    }

    pub fn i64_of(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Node::as_i64)
    }

    pub fn bool_of(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(Node::as_bool)
    }
}

/// A value to set a property to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value<'a> {
    Flag(bool),
    Int(i64),
    Double(f64),
    Str(&'a str),
}

impl From<bool> for Value<'_> {
    fn from(v: bool) -> Self {
        Value::Flag(v)
    }
}
impl From<i64> for Value<'_> {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}
impl From<f64> for Value<'_> {
    fn from(v: f64) -> Self {
        Value::Double(v)
    }
}
impl<'a> From<&'a str> for Value<'a> {
    fn from(v: &'a str) -> Self {
        Value::Str(v)
    }
}

/// Why a file stopped playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Eof,
    Stop,
    Quit,
    Error,
    Redirect,
    Other(i32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Shutdown,
    Log {
        prefix: String,
        level: String,
        text: String,
    },
    PropertyChange {
        name: String,
        value: Node,
    },
    GetPropertyReply {
        error: i32,
        name: String,
        value: Node,
    },
    /// Completion of an async command or property change.
    Reply {
        id: u64,
        error: i32,
    },
    StartFile,
    FileLoaded,
    EndFile {
        reason: EndReason,
        error: i32,
    },
    Other,
}

type Callback = Box<dyn Fn() + Send + Sync>;

struct Core {
    api: Arc<Api>,
    handle: *mut ffi::mpv_handle,
    wakeup: Mutex<Option<Box<Callback>>>,
}

// SAFETY: libmpv's client API is thread-safe ("all functions are thread-safe" in client.h), and
// the callback box is only replaced under the mutex.
unsafe impl Send for Core {}
// SAFETY: as above.
unsafe impl Sync for Core {}

impl Drop for Core {
    fn drop(&mut self) {
        // SAFETY: handle is valid until this call; afterwards mpv invokes no more callbacks, so the
        // callback box may be dropped after it.
        unsafe { (self.api.terminate_destroy)(self.handle) };
        self.wakeup.lock().unwrap_or_else(PoisonError::into_inner).take();
    }
}

/// One mpv player core.
#[derive(Clone)]
pub struct Mpv {
    core: Arc<Core>,
}

unsafe extern "C" fn callback_trampoline(data: *mut c_void) {
    // SAFETY: `data` is the address of a `Callback` box kept alive by its owner until mpv has
    // been told to stop calling it.
    let f = unsafe { &*(data as *const Callback) };
    // A panic must not unwind into C.
    let _ = catch_unwind(AssertUnwindSafe(f));
}

fn cstring(s: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|_| invalid_param(s))
}

/// mpv takes file names as raw bytes on Unix and as UTF-8 on Windows.
fn os_cstring(s: &OsStr) -> Result<CString, Error> {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        s.as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = s.to_string_lossy().into_owned().into_bytes();
    CString::new(bytes).map_err(|_| invalid_param(&s.to_string_lossy()))
}

fn invalid_param(what: &str) -> Error {
    Error { code: -4, message: format!("argument contains a NUL byte: {what:?}") }
}

/// Copies a C string; mpv strings are UTF-8 except Unix file names, shown lossily.
///
/// # Safety
/// `p` is null or points to a NUL-terminated string.
unsafe fn string_from(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        // SAFETY: guaranteed by the caller.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

/// Deep-copies an `mpv_node`.
///
/// # Safety
/// `node` is a valid node as produced by libmpv (lists hold `num` values and, for maps, keys).
unsafe fn node_from(node: &ffi::mpv_node, depth: u32) -> Node {
    if depth > 64 {
        return Node::None;
    }
    // SAFETY (whole block): the union member read is the one `format` says is valid.
    unsafe {
        match node.format {
            ffi::MPV_FORMAT_STRING => Node::String(string_from(node.u.string)),
            ffi::MPV_FORMAT_FLAG => Node::Flag(node.u.flag != 0),
            ffi::MPV_FORMAT_INT64 => Node::Int(node.u.int64),
            ffi::MPV_FORMAT_DOUBLE => Node::Double(node.u.double_),
            ffi::MPV_FORMAT_NODE_ARRAY | ffi::MPV_FORMAT_NODE_MAP => {
                let list = node.u.list;
                if list.is_null() {
                    return Node::None;
                }
                let n = usize::try_from((*list).num).unwrap_or(0);
                let values = if n == 0 || (*list).values.is_null() {
                    &[][..]
                } else {
                    std::slice::from_raw_parts((*list).values, n)
                };
                if node.format == ffi::MPV_FORMAT_NODE_ARRAY {
                    Node::Array(values.iter().map(|v| node_from(v, depth + 1)).collect())
                } else {
                    let keys = (*list).keys;
                    Node::Map(
                        values
                            .iter()
                            .enumerate()
                            .map(|(i, v)| {
                                let key = if keys.is_null() { String::new() } else { string_from(*keys.add(i)) };
                                (key, node_from(v, depth + 1))
                            })
                            .collect(),
                    )
                }
            }
            _ => Node::None,
        }
    }
}

/// Reads the value of a property event (`mpv_event_property`).
///
/// # Safety
/// `prop` is a valid `mpv_event_property` from the current event.
unsafe fn property_value(prop: &ffi::mpv_event_property) -> Node {
    if prop.data.is_null() {
        return Node::None;
    }
    // SAFETY: `data` points to a value of type `format`, per client.h.
    unsafe {
        match prop.format {
            ffi::MPV_FORMAT_NODE => node_from(&*(prop.data as *const ffi::mpv_node), 0),
            ffi::MPV_FORMAT_STRING => Node::String(string_from(*(prop.data as *const *const c_char))),
            ffi::MPV_FORMAT_FLAG => Node::Flag(*(prop.data as *const c_int) != 0),
            ffi::MPV_FORMAT_INT64 => Node::Int(*(prop.data as *const i64)),
            ffi::MPV_FORMAT_DOUBLE => Node::Double(*(prop.data as *const f64)),
            _ => Node::None,
        }
    }
}

static LOCALE: Once = Once::new();

impl Mpv {
    /// Creates and initializes a core. `options` are set before initialization (some, such as
    /// `config`, only work then); one this mpv version rejects is logged and skipped.
    pub fn new(api: Arc<Api>, options: &[(&str, &str)]) -> Result<Mpv, Error> {
        #[cfg(unix)]
        LOCALE.call_once(|| {
            // mpv refuses to start unless LC_NUMERIC is "C" (client.h, mpv_create). Nothing in
            // VibeLC changes the locale, so setting it once, before the first core, is enough.
            // SAFETY: called once, before other threads that use the locale exist.
            unsafe { libc::setlocale(libc::LC_NUMERIC, c"C".as_ptr()) };
        });
        #[cfg(not(unix))]
        LOCALE.call_once(|| {});

        // SAFETY: no preconditions.
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err(Error { code: -2, message: "mpv_create failed".into() });
        }
        let mpv = Mpv { core: Arc::new(Core { api, handle, wakeup: Mutex::new(None) }) };
        for (name, value) in options {
            let (n, v) = (cstring(name)?, cstring(value)?);
            // SAFETY: valid handle and NUL-terminated strings.
            if let Err(e) = mpv.check(unsafe { (mpv.core.api.set_option_string)(handle, n.as_ptr(), v.as_ptr()) }) {
                log::warn!("mpv option {name}={value}: {e}");
            }
        }
        // SAFETY: valid, uninitialized handle.
        mpv.check(unsafe { (mpv.core.api.initialize)(handle) })?;
        Ok(mpv)
    }

    fn check(&self, code: c_int) -> Result<(), Error> {
        if code >= 0 { Ok(()) } else { Err(self.error(code)) }
    }

    fn error(&self, code: c_int) -> Error {
        // SAFETY: returns a static string for any code.
        let message = unsafe { string_from((self.core.api.error_string)(code)) };
        Error { code, message }
    }

    /// `f` runs on an mpv thread whenever new events are queued.
    pub fn set_wakeup_callback(&self, f: impl Fn() + Send + Sync + 'static) {
        let mut slot = self.core.wakeup.lock().unwrap_or_else(PoisonError::into_inner);
        let boxed: Box<Callback> = Box::new(Box::new(f));
        let data = &*boxed as *const Callback as *mut c_void;
        // SAFETY: the box outlives its registration: it is kept in `slot` until replaced here or
        // until the core is destroyed. A replaced box could still be running (mpv may call the
        // old callback concurrently), so replaced callbacks are leaked rather than freed.
        unsafe { (self.core.api.set_wakeup_callback)(self.core.handle, Some(callback_trampoline), data) };
        if let Some(old) = slot.replace(boxed) {
            std::mem::forget(old);
        }
    }

    fn with_args<R>(args: &[impl AsRef<OsStr>], f: impl FnOnce(*mut *const c_char) -> R) -> Result<R, Error> {
        let owned: Vec<CString> = args.iter().map(|a| os_cstring(a.as_ref())).collect::<Result<_, _>>()?;
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        Ok(f(ptrs.as_mut_ptr()))
    }

    /// Runs a command and waits for it (for worker threads and tests; the UI uses
    /// [`Mpv::command_async`]).
    pub fn command(&self, args: &[impl AsRef<OsStr>]) -> Result<(), Error> {
        // SAFETY: NULL-terminated array of NUL-terminated strings, valid during the call.
        let code = Mpv::with_args(args, |p| unsafe { (self.core.api.command)(self.core.handle, p) })?;
        self.check(code)
    }

    /// Queues a command; failures arrive as [`Event::CommandReply`] with this `id`.
    pub fn command_async(&self, id: u64, args: &[impl AsRef<OsStr>]) -> Result<(), Error> {
        // SAFETY: as in `command`; mpv copies the arguments before returning.
        let code = Mpv::with_args(args, |p| unsafe { (self.core.api.command_async)(self.core.handle, id, p) })?;
        self.check(code)
    }

    /// Queues a property change; failures arrive as [`Event::SetPropertyReply`] with this `id`.
    pub fn set_property_async<'a>(&self, id: u64, name: &str, value: impl Into<Value<'a>>) -> Result<(), Error> {
        let n = cstring(name)?;
        let api = &self.core.api;
        let h = self.core.handle;
        // SAFETY: each branch passes a pointer to a value of the declared format, valid during
        // the call; mpv copies it.
        let code = unsafe {
            match value.into() {
                Value::Flag(b) => {
                    let mut v: c_int = b.into();
                    (api.set_property_async)(h, id, n.as_ptr(), ffi::MPV_FORMAT_FLAG, (&raw mut v).cast())
                }
                Value::Int(i) => {
                    let mut v = i;
                    (api.set_property_async)(h, id, n.as_ptr(), ffi::MPV_FORMAT_INT64, (&raw mut v).cast())
                }
                Value::Double(d) => {
                    let mut v = d;
                    (api.set_property_async)(h, id, n.as_ptr(), ffi::MPV_FORMAT_DOUBLE, (&raw mut v).cast())
                }
                Value::Str(s) => {
                    let s = cstring(s)?;
                    let mut p = s.as_ptr();
                    (api.set_property_async)(h, id, n.as_ptr(), ffi::MPV_FORMAT_STRING, (&raw mut p).cast())
                }
            }
        };
        self.check(code)
    }

    /// Reads a property and waits for it (for worker threads and tests).
    pub fn get_property(&self, name: &str) -> Result<Node, Error> {
        let n = cstring(name)?;
        let mut node = ffi::mpv_node { u: ffi::mpv_node_u { int64: 0 }, format: ffi::MPV_FORMAT_NONE };
        // SAFETY: `node` receives an mpv-allocated tree, copied and then freed with
        // mpv_free_node_contents as client.h requires.
        unsafe {
            self.check((self.core.api.get_property)(
                self.core.handle,
                n.as_ptr(),
                ffi::MPV_FORMAT_NODE,
                (&raw mut node).cast(),
            ))?;
            let out = node_from(&node, 0);
            (self.core.api.free_node_contents)(&raw mut node);
            Ok(out)
        }
    }

    /// Requests a property; the value arrives as [`Event::GetPropertyReply`].
    pub fn get_property_async(&self, name: &str) -> Result<(), Error> {
        let n = cstring(name)?;
        // SAFETY: valid handle and string.
        self.check(unsafe { (self.core.api.get_property_async)(self.core.handle, 0, n.as_ptr(), ffi::MPV_FORMAT_NODE) })
    }

    /// Delivers [`Event::PropertyChange`] for `name` now and on every change.
    pub fn observe(&self, name: &str) -> Result<(), Error> {
        let n = cstring(name)?;
        // SAFETY: valid handle and string.
        self.check(unsafe { (self.core.api.observe_property)(self.core.handle, 0, n.as_ptr(), ffi::MPV_FORMAT_NODE) })
    }

    /// Delivers mpv's own log lines at `level` ("warn", "info", "v", ...) as [`Event::Log`].
    pub fn request_log_messages(&self, level: &str) -> Result<(), Error> {
        let l = cstring(level)?;
        // SAFETY: valid handle and string.
        self.check(unsafe { (self.core.api.request_log_messages)(self.core.handle, l.as_ptr()) })
    }

    /// Next queued event, waiting up to `timeout` seconds (0 = don't wait).
    pub fn next_event(&self, timeout: f64) -> Option<Event> {
        // SAFETY: the returned event and its data stay valid until the next mpv_wait_event on
        // this handle; everything is copied before returning. Only the UI thread (or, for a
        // worker's own core, that worker) waits on a given core.
        unsafe {
            let ev = (self.core.api.wait_event)(self.core.handle, timeout);
            if ev.is_null() {
                return None;
            }
            let ev = &*ev;
            let data = ev.data;
            Some(match ev.event_id {
                ffi::MPV_EVENT_NONE => return None,
                ffi::MPV_EVENT_SHUTDOWN => Event::Shutdown,
                ffi::MPV_EVENT_LOG_MESSAGE if !data.is_null() => {
                    let m = &*(data as *const ffi::mpv_event_log_message);
                    Event::Log { prefix: string_from(m.prefix), level: string_from(m.level), text: string_from(m.text) }
                }
                ffi::MPV_EVENT_PROPERTY_CHANGE | ffi::MPV_EVENT_GET_PROPERTY_REPLY if !data.is_null() => {
                    let p = &*(data as *const ffi::mpv_event_property);
                    let name = string_from(p.name);
                    let value = property_value(p);
                    if ev.event_id == ffi::MPV_EVENT_PROPERTY_CHANGE {
                        Event::PropertyChange { name, value }
                    } else {
                        Event::GetPropertyReply { error: ev.error, name, value }
                    }
                }
                ffi::MPV_EVENT_SET_PROPERTY_REPLY | ffi::MPV_EVENT_COMMAND_REPLY => {
                    Event::Reply { id: ev.reply_userdata, error: ev.error }
                }
                ffi::MPV_EVENT_START_FILE => Event::StartFile,
                ffi::MPV_EVENT_FILE_LOADED => Event::FileLoaded,
                ffi::MPV_EVENT_END_FILE if !data.is_null() => {
                    let e = &*(data as *const ffi::mpv_event_end_file);
                    let reason = match e.reason {
                        ffi::MPV_END_FILE_REASON_EOF => EndReason::Eof,
                        ffi::MPV_END_FILE_REASON_STOP => EndReason::Stop,
                        ffi::MPV_END_FILE_REASON_QUIT => EndReason::Quit,
                        ffi::MPV_END_FILE_REASON_ERROR => EndReason::Error,
                        ffi::MPV_END_FILE_REASON_REDIRECT => EndReason::Redirect,
                        other => EndReason::Other(other),
                    };
                    Event::EndFile { reason, error: e.error }
                }
                _ => Event::Other,
            })
        }
    }

    /// Description of an error code, for event replies.
    pub fn error_message(&self, code: i32) -> String {
        self.error(code).message
    }
}

/// Draws an [`Mpv`] core's video with OpenGL or Direct3D 11. Must be created, used and dropped
/// on the thread that owns the GL context (current) or the D3D11 immediate context.
pub struct RenderContext {
    core: Arc<Core>,
    ctx: *mut ffi::mpv_render_context,
    update: Option<Box<Callback>>,
    /// Kept alive for mpv, which may resolve GL functions after creation (OpenGL only).
    _get_proc_address: Option<Box<ProcAddressFn>>,
}

type ProcAddressFn = Box<dyn Fn(&CStr) -> *const c_void>;

/// Where [`RenderContext::render`] draws.
#[derive(Clone, Copy)]
pub enum Target {
    /// OpenGL framebuffer `fbo` (0 = the window) of `w` x `h` pixels.
    Gl { fbo: i32, w: i32, h: i32 },
    /// An `ID3D11Texture2D` with render-target binding, on the context's device.
    #[cfg_attr(not(windows), allow(dead_code))]
    D3d11(*mut c_void),
}

/// The window system connection, which mpv needs for VA-API and VDPAU hardware decoding.
#[derive(Clone, Copy)]
pub enum NativeDisplay {
    X11(*mut c_void),
    Wayland(*mut c_void),
}

unsafe extern "C" fn get_proc_address_trampoline(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    if ctx.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `ctx` is the `ProcAddressFn` box owned by the RenderContext; `name` is a C string.
    let (f, name) = unsafe { (&*(ctx as *const ProcAddressFn), CStr::from_ptr(name)) };
    catch_unwind(AssertUnwindSafe(|| f(name) as *mut c_void)).unwrap_or(std::ptr::null_mut())
}

impl RenderContext {
    /// # Safety
    /// The OpenGL context that `get_proc_address` resolves functions for must be current on this
    /// thread, here and for every later call on the returned value (including drop).
    /// `display`, if given, is the connection the GL context's window belongs to and must
    /// outlive the returned value.
    pub unsafe fn new_gl(
        mpv: &Mpv,
        get_proc_address: ProcAddressFn,
        display: Option<NativeDisplay>,
    ) -> Result<RenderContext, Error> {
        let get_proc_address: Box<ProcAddressFn> = Box::new(get_proc_address);
        let mut init = ffi::mpv_opengl_init_params {
            get_proc_address: Some(get_proc_address_trampoline),
            get_proc_address_ctx: &*get_proc_address as *const ProcAddressFn as *mut c_void,
            extra_exts: std::ptr::null(),
        };
        // Advanced control lets software decoders write frames straight into mapped GPU buffers
        // (direct rendering), saving a full-frame copy each. render.h's price: the render
        // thread never waits for the core (VibeLC's makes only the calls marked safe for it),
        // and calls `mpv_render_context_update` after every update callback (`frame_due`).
        let mut advanced: c_int = 1;
        let mut params = vec![
            ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_API_TYPE, data: c"opengl".as_ptr() as *mut c_void },
            ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_OPENGL_INIT_PARAMS, data: (&raw mut init).cast() },
            ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_ADVANCED_CONTROL, data: (&raw mut advanced).cast() },
        ];
        match display {
            Some(NativeDisplay::X11(d)) => {
                params.push(ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_X11_DISPLAY, data: d })
            }
            Some(NativeDisplay::Wayland(d)) => {
                params.push(ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_WL_DISPLAY, data: d })
            }
            None => {}
        }
        // SAFETY: GL context current per this function's contract.
        unsafe { RenderContext::create(mpv, params, Some(get_proc_address)) }
    }

    /// mpv draws with Direct3D 11 on `device` (an `ID3D11Device`, created with video support so
    /// that D3D11VA frames are shown in place, without a copy). Needs client API 2.6 (mpv 0.42).
    ///
    /// # Safety
    /// `device` is a valid `ID3D11Device`; every later call on the returned value is made on the
    /// thread that uses its immediate context.
    #[cfg(windows)]
    pub unsafe fn new_d3d11(mpv: &Mpv, device: *mut c_void) -> Result<RenderContext, Error> {
        let mut init = ffi::mpv_d3d11_init_params { device };
        let mut advanced: c_int = 1; // as in `new_gl`
        let params = vec![
            ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_API_TYPE, data: c"d3d11".as_ptr() as *mut c_void },
            ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_D3D11_INIT_PARAMS, data: (&raw mut init).cast() },
            ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_ADVANCED_CONTROL, data: (&raw mut advanced).cast() },
        ];
        // SAFETY: per this function's contract.
        unsafe { RenderContext::create(mpv, params, None) }
    }

    /// # Safety
    /// The parameters' data outlives the call and the graphics API's context is usable here.
    unsafe fn create(
        mpv: &Mpv,
        mut params: Vec<ffi::mpv_render_param>,
        get_proc_address: Option<Box<ProcAddressFn>>,
    ) -> Result<RenderContext, Error> {
        params.push(ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_INVALID, data: std::ptr::null_mut() });
        let mut ctx = std::ptr::null_mut();
        // SAFETY: valid handle; the parameter list is terminated and its data outlives the call.
        let code = unsafe { (mpv.core.api.render_context_create)(&raw mut ctx, mpv.core.handle, params.as_mut_ptr()) };
        mpv.check(code)?;
        Ok(RenderContext { core: Arc::clone(&mpv.core), ctx, update: None, _get_proc_address: get_proc_address })
    }

    /// `f` runs on an mpv thread whenever a new frame should be drawn.
    pub fn set_update_callback(&mut self, f: impl Fn() + Send + Sync + 'static) {
        let boxed: Box<Callback> = Box::new(Box::new(f));
        let data = &*boxed as *const Callback as *mut c_void;
        // SAFETY: the box is kept until drop (after the context is freed); a replaced one is
        // leaked because mpv may still be running it.
        unsafe { (self.core.api.render_context_set_update_callback)(self.ctx, Some(callback_trampoline), data) };
        if let Some(old) = self.update.replace(boxed) {
            std::mem::forget(old);
        }
    }

    /// Whether a new frame should be rendered. Call on the render thread after each update
    /// callback.
    pub fn frame_due(&self) -> bool {
        // SAFETY: valid context, called on the render thread as render.h requires.
        unsafe { (self.core.api.render_context_update)(self.ctx) & ffi::MPV_RENDER_UPDATE_FRAME != 0 }
    }

    /// Renders the current frame into `target`. With `skip`, mpv only advances its timing (for a
    /// hidden window).
    ///
    /// # Safety
    /// The graphics context is usable here (see the constructors) and `target` belongs to it.
    pub unsafe fn render(&self, target: Target, skip: bool) {
        let mut fbo = ffi::mpv_opengl_fbo { fbo: 0, w: 0, h: 0, internal_format: 0 };
        let mut tex = ffi::mpv_d3d11_fbo { tex: std::ptr::null_mut() };
        let mut flip: c_int = 1; // a GL window's framebuffer has its origin at the bottom
        let mut block: c_int = 0; // never wait for the display time on the UI thread
        let mut skip_flag: c_int = skip.into();
        let first = match target {
            Target::Gl { fbo: id, w, h } => {
                (fbo.fbo, fbo.w, fbo.h) = (id, w, h);
                ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_OPENGL_FBO, data: (&raw mut fbo).cast() }
            }
            Target::D3d11(t) => {
                tex.tex = t;
                flip = 0;
                ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_D3D11_FBO, data: (&raw mut tex).cast() }
            }
        };
        let mut params = [
            first,
            ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_FLIP_Y, data: (&raw mut flip).cast() },
            ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME, data: (&raw mut block).cast() },
            ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_SKIP_RENDERING, data: (&raw mut skip_flag).cast() },
            ffi::mpv_render_param { type_: ffi::MPV_RENDER_PARAM_INVALID, data: std::ptr::null_mut() },
        ];
        // SAFETY: valid context, terminated parameter list; GL current per contract.
        let code = unsafe { (self.core.api.render_context_render)(self.ctx, params.as_mut_ptr()) };
        if code < 0 {
            log::warn!("mpv render failed: {}", Mpv { core: Arc::clone(&self.core) }.error_message(code));
        }
    }

    /// Tells mpv a frame was presented (after the buffer swap), for its frame timing.
    pub fn report_swap(&self) {
        // SAFETY: valid context.
        unsafe { (self.core.api.render_context_report_swap)(self.ctx) };
    }
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        // SAFETY: valid context, GL current per the contract of `new_gl`. After free, mpv calls
        // no more callbacks, so the boxes can be dropped.
        unsafe {
            (self.core.api.render_context_set_update_callback)(self.ctx, None, std::ptr::null_mut());
            (self.core.api.render_context_free)(self.ctx);
        }
        self.update.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds C nodes in Rust memory to check the deep copy without libmpv.
    #[test]
    fn node_tree_is_copied() {
        let key_a = CString::new("title").unwrap();
        let key_b = CString::new("ids").unwrap();
        let s = CString::new("Ünïcode").unwrap();
        let mut ints = [
            ffi::mpv_node { u: ffi::mpv_node_u { int64: 7 }, format: ffi::MPV_FORMAT_INT64 },
            ffi::mpv_node { u: ffi::mpv_node_u { double_: 2.5 }, format: ffi::MPV_FORMAT_DOUBLE },
            ffi::mpv_node { u: ffi::mpv_node_u { flag: 1 }, format: ffi::MPV_FORMAT_FLAG },
        ];
        let mut arr = ffi::mpv_node_list { num: 3, values: ints.as_mut_ptr(), keys: std::ptr::null_mut() };
        let mut values = [
            ffi::mpv_node { u: ffi::mpv_node_u { string: s.as_ptr() as *mut c_char }, format: ffi::MPV_FORMAT_STRING },
            ffi::mpv_node { u: ffi::mpv_node_u { list: &raw mut arr }, format: ffi::MPV_FORMAT_NODE_ARRAY },
        ];
        let mut keys = [key_a.as_ptr() as *mut c_char, key_b.as_ptr() as *mut c_char];
        let mut map = ffi::mpv_node_list { num: 2, values: values.as_mut_ptr(), keys: keys.as_mut_ptr() };
        let root = ffi::mpv_node { u: ffi::mpv_node_u { list: &raw mut map }, format: ffi::MPV_FORMAT_NODE_MAP };

        // SAFETY: the tree above is well formed and outlives the call.
        let node = unsafe { node_from(&root, 0) };
        assert_eq!(node.str_of("title"), Some("Ünïcode"));
        assert_eq!(node.get("ids").map(Node::items), Some(&[Node::Int(7), Node::Double(2.5), Node::Flag(true)][..]));
        assert_eq!(node.get("ids").unwrap().items()[0].as_f64(), Some(7.0));
        assert_eq!(node.get("missing"), None);
    }

    #[test]
    fn empty_and_null_lists_are_safe() {
        let mut empty = ffi::mpv_node_list { num: 0, values: std::ptr::null_mut(), keys: std::ptr::null_mut() };
        let arr = ffi::mpv_node { u: ffi::mpv_node_u { list: &raw mut empty }, format: ffi::MPV_FORMAT_NODE_ARRAY };
        let null =
            ffi::mpv_node { u: ffi::mpv_node_u { list: std::ptr::null_mut() }, format: ffi::MPV_FORMAT_NODE_MAP };
        // SAFETY: well-formed nodes.
        unsafe {
            assert_eq!(node_from(&arr, 0), Node::Array(vec![]));
            assert_eq!(node_from(&null, 0), Node::None);
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_file_names_pass_as_raw_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let raw = OsStr::from_bytes(b"/tmp/caf\xe9.mkv"); // Latin-1, not UTF-8
        assert_eq!(os_cstring(raw).unwrap().as_bytes(), b"/tmp/caf\xe9.mkv");
        assert!(os_cstring(OsStr::new("a\0b")).is_err());
    }
}
