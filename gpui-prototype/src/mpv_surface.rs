//! macOS libmpv surface (experimental).
//!
//! libmpv draws into its own NSWindow, which this module attaches as a child
//! window of the GPUI window. The window is borderless, ignores mouse events and
//! is placed over the preview area in screen coordinates. It is shown while the
//! video plays and hidden (`orderOut`) when paused or editing, when the GPUI
//! picture (host frame with libass subtitles) is shown instead. The mpv content
//! is never reparented into an NSView, so its Vulkan/Metal surface stays intact.
//!
//! mpv is configured with a unique `title` (it contains the handle address) so
//! the window it creates can be found among `NSApp windows`. Its creation is
//! asynchronous, so `place` retries the lookup on each render until it appears.
//!
//! Threads: after initialization the GPUI main thread never waits on mpv.
//! Commands go through `mpv_command_async`, and `time-pos` is read by a reader
//! thread that also drains mpv's event queue (FILE_LOADED tells it when
//! subtitles can be added). Teardown runs on a helper thread: it stops and
//! joins the reader, then `mpv_terminate_destroy` (which waits for mpv's video
//! thread, which may need the main thread).

use anyhow::{Result, anyhow};
use gpui_kit::{Bounds, Pixels, Window};
use libloading::Library;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const MPV_FORMAT_DOUBLE: c_int = 5;
const MPV_EVENT_SHUTDOWN: c_int = 1;
const MPV_EVENT_FILE_LOADED: c_int = 8;
/// `NSWindowAbove` (NSWindowOrderingMode).
const NS_WINDOW_ABOVE: isize = 1;
/// How often the reader thread refreshes the cached `time-pos`.
const POSITION_POLL: Duration = Duration::from_millis(10);
/// Drift from the host clock that triggers a seek while playing, in seconds.
const PLAY_TOLERANCE: f64 = 0.3;
/// Minimum gap between seeks, so a seek mpv has not finished yet is not
/// repeated on every render.
const SEEK_RETRY: Duration = Duration::from_millis(200);

type MpvHandle = c_void;

/// Leading field of `mpv_event`; only `event_id` is read.
#[repr(C)]
struct MpvEvent {
    event_id: c_int,
}

type FnCreate = unsafe extern "C" fn() -> *mut MpvHandle;
type FnSetOptionString =
    unsafe extern "C" fn(*mut MpvHandle, *const c_char, *const c_char) -> c_int;
type FnInitialize = unsafe extern "C" fn(*mut MpvHandle) -> c_int;
/// `mpv_command_async(ctx, reply_userdata, args)`; the reply arrives as an event.
type FnCommandAsync = unsafe extern "C" fn(*mut MpvHandle, u64, *const *const c_char) -> c_int;
type FnWaitEvent = unsafe extern "C" fn(*mut MpvHandle, f64) -> *const MpvEvent;
/// `mpv_get_property(ctx, name, format, data)`.
type FnGetProperty =
    unsafe extern "C" fn(*mut MpvHandle, *const c_char, c_int, *mut c_void) -> c_int;
type FnTerminate = unsafe extern "C" fn(*mut MpvHandle);

struct MpvApi {
    _library: Library,
    create: FnCreate,
    set_option_string: FnSetOptionString,
    initialize: FnInitialize,
    command_async: FnCommandAsync,
    wait_event: FnWaitEvent,
    get_property: FnGetProperty,
    terminate: FnTerminate,
}

// SAFETY: the fields never change after `load`. The library stays mapped until
// the last `Arc` is dropped, which happens on the teardown thread after
// `mpv_terminate_destroy` has returned.
unsafe impl Send for MpvApi {}
unsafe impl Sync for MpvApi {}

impl MpvApi {
    fn load() -> Result<Self> {
        let mut last = None;
        let library = [
            "/opt/homebrew/lib/libmpv.2.dylib",
            "libmpv.2.dylib",
            "libmpv.dylib",
        ]
        .iter()
        .find_map(|path| match unsafe { Library::new(path) } {
            Ok(library) => Some(library),
            Err(error) => {
                last = Some(error);
                None
            }
        })
        .ok_or_else(|| anyhow!("libmpv not found: {:?}", last))?;
        unsafe {
            Ok(Self {
                create: *library.get::<FnCreate>(b"mpv_create\0")?,
                set_option_string: *library.get::<FnSetOptionString>(b"mpv_set_option_string\0")?,
                initialize: *library.get::<FnInitialize>(b"mpv_initialize\0")?,
                command_async: *library.get::<FnCommandAsync>(b"mpv_command_async\0")?,
                wait_event: *library.get::<FnWaitEvent>(b"mpv_wait_event\0")?,
                get_property: *library.get::<FnGetProperty>(b"mpv_get_property\0")?,
                terminate: *library.get::<FnTerminate>(b"mpv_terminate_destroy\0")?,
                _library: library,
            })
        }
    }
}

/// State shared between the main thread and the reader thread.
struct Shared {
    /// `f64` bits of the last `time-pos`; NaN while no file is playing.
    position: AtomicU64,
    /// Bumped by `load_file` before `loadfile` is queued.
    load_generation: AtomicU64,
    /// `load_generation` at the last FILE_LOADED event seen by the reader.
    loaded_generation: AtomicU64,
    /// Subtitle file waiting for the current file's FILE_LOADED.
    pending_subs: Mutex<Option<String>>,
    stop: AtomicBool,
}

impl Shared {
    fn new() -> Self {
        Self {
            position: AtomicU64::new(f64::NAN.to_bits()),
            load_generation: AtomicU64::new(0),
            loaded_generation: AtomicU64::new(0),
            pending_subs: Mutex::new(None),
            stop: AtomicBool::new(false),
        }
    }

    /// True once the current generation's file has reached FILE_LOADED.
    fn file_loaded(&self) -> bool {
        let generation = self.load_generation.load(Ordering::Acquire);
        generation != 0 && self.loaded_generation.load(Ordering::Acquire) == generation
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *const c_void;
    fn objc_msgSend();
    #[cfg(target_arch = "x86_64")]
    fn objc_msgSend_stret();
}

// Each type below is one Objective-C method shape. Call sites pick the shape
// that matches the method they send; the receiver and selector are live
// objects created or stored by this module.
type SelObj = unsafe extern "C" fn(*mut c_void, *const c_void) -> *mut c_void;
type SelIdx = unsafe extern "C" fn(*mut c_void, *const c_void, usize) -> *mut c_void;
type SelUsize = unsafe extern "C" fn(*mut c_void, *const c_void) -> usize;
type SelUsizeArg = unsafe extern "C" fn(*mut c_void, *const c_void, usize);
type SelCStrGet = unsafe extern "C" fn(*mut c_void, *const c_void) -> *const c_char;
type SelVoid = unsafe extern "C" fn(*mut c_void, *const c_void);
type SelBool = unsafe extern "C" fn(*mut c_void, *const c_void, i8);
type SelBoolGet = unsafe extern "C" fn(*mut c_void, *const c_void) -> i8;
type SelObjArg = unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void);
type SelChildWindow = unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void, isize);
type SelRectBool = unsafe extern "C" fn(*mut c_void, *const c_void, CGRect, i8);
type SelRectGet = unsafe extern "C" fn(*mut c_void, *const c_void) -> CGRect;
/// `convertRect:toView:` (second argument nil) or `convertRectToScreen:`
/// (which ignores the extra argument).
type SelRectCall = unsafe extern "C" fn(*mut c_void, *const c_void, CGRect, *mut c_void) -> CGRect;

/// Typed pointer to `objc_msgSend`. `F` must be a function pointer whose shape
/// matches the Objective-C method being sent.
unsafe fn msg_send<F: Copy>() -> F {
    debug_assert_eq!(
        std::mem::size_of::<F>(),
        std::mem::size_of::<*const c_void>()
    );
    unsafe { std::mem::transmute_copy::<*const c_void, F>(&(objc_msgSend as *const c_void)) }
}

fn selector(name: &CStr) -> *const c_void {
    unsafe { sel_registerName(name.as_ptr()) }
}

unsafe fn view_bounds(view: *mut c_void) -> CGRect {
    #[cfg(target_arch = "x86_64")]
    {
        let mut rect = CGRect::default();
        let send: unsafe extern "C" fn(*mut CGRect, *mut c_void, *const c_void) =
            unsafe { std::mem::transmute(objc_msgSend_stret as *const c_void) };
        unsafe { send(&mut rect, view, selector(c"bounds")) };
        rect
    }
    #[cfg(not(target_arch = "x86_64"))]
    unsafe {
        msg_send::<SelRectGet>()(view, selector(c"bounds"))
    }
}

/// Sends a rect-in, rect-out method. On x86_64 the CGRect result is returned
/// through `objc_msgSend_stret`.
unsafe fn convert_rect(receiver: *mut c_void, sel: &CStr, rect: CGRect) -> CGRect {
    #[cfg(target_arch = "x86_64")]
    {
        let mut out = CGRect::default();
        let send: unsafe extern "C" fn(
            *mut CGRect,
            *mut c_void,
            *const c_void,
            CGRect,
            *mut c_void,
        ) = unsafe { std::mem::transmute(objc_msgSend_stret as *const c_void) };
        unsafe { send(&mut out, receiver, selector(sel), rect, ptr::null_mut()) };
        out
    }
    #[cfg(not(target_arch = "x86_64"))]
    unsafe {
        msg_send::<SelRectCall>()(receiver, selector(sel), rect, ptr::null_mut())
    }
}

/// Finds the NSWindow whose title equals `title` among the app's windows.
/// Main thread only.
fn find_window(title: &str) -> *mut c_void {
    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return ptr::null_mut();
        }
        let app = msg_send::<SelObj>()(class, selector(c"sharedApplication"));
        if app.is_null() {
            return ptr::null_mut();
        }
        let windows = msg_send::<SelObj>()(app, selector(c"windows"));
        if windows.is_null() {
            return ptr::null_mut();
        }
        let count = msg_send::<SelUsize>()(windows, selector(c"count"));
        for index in 0..count {
            let window = msg_send::<SelIdx>()(windows, selector(c"objectAtIndex:"), index);
            if window.is_null() {
                continue;
            }
            let ns_title = msg_send::<SelObj>()(window, selector(c"title"));
            if ns_title.is_null() {
                continue;
            }
            let utf8 = msg_send::<SelCStrGet>()(ns_title, selector(c"UTF8String"));
            if !utf8.is_null() && CStr::from_ptr(utf8).to_bytes() == title.as_bytes() {
                return window;
            }
        }
        ptr::null_mut()
    }
}

fn parent_ns_view(window: &Window) -> Result<*mut c_void> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow!("no native window handle: {error}"))?;
    match handle.as_raw() {
        RawWindowHandle::AppKit(handle) => Ok(handle.ns_view.as_ptr()),
        _ => Err(anyhow!("window is not AppKit-backed")),
    }
}

/// Converts a GPUI rect (top-left origin unless the view is non-flipped) into
/// the view's own coordinate system.
fn rect_from_bounds(bounds: Bounds<Pixels>, parent_height: f64, flipped: bool) -> CGRect {
    let x = f64::from(f32::from(bounds.origin.x));
    let y = f64::from(f32::from(bounds.origin.y));
    let width = f64::from(f32::from(bounds.size.width));
    let height = f64::from(f32::from(bounds.size.height));
    let y = if flipped {
        y
    } else {
        parent_height - y - height
    };
    CGRect {
        origin: CGPoint { x, y },
        size: CGSize { width, height },
    }
}

fn check(status: c_int, what: &str) -> Result<()> {
    if status < 0 {
        Err(anyhow!("{what} failed ({status})"))
    } else {
        Ok(())
    }
}

/// Queues `args` as an mpv command without waiting for it. mpv's reply arrives
/// as an event that the reader thread drains.
fn queue_command(api: &MpvApi, mpv: *mut MpvHandle, args: &[&str]) -> Result<()> {
    let owned = args
        .iter()
        .map(|arg| CString::new(*arg))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut pointers: Vec<*const c_char> = owned.iter().map(|arg| arg.as_ptr()).collect();
    pointers.push(ptr::null());
    check(
        unsafe { (api.command_async)(mpv, 0, pointers.as_ptr()) },
        args[0],
    )
}

/// Replaces mpv's external subtitle track. `sub-remove` without an id drops the
/// track added last time; its error is ignored when there is none.
fn queue_subtitles(api: &MpvApi, mpv: *mut MpvHandle, path: &str) -> Result<()> {
    let _ = queue_command(api, mpv, &["sub-remove"]);
    queue_command(api, mpv, &["sub-add", path, "select"])
}

/// Reads `time-pos` and drains mpv's events until shutdown or `stop`. After
/// startup only this thread calls `mpv_wait_event` and `mpv_get_property`, so
/// the main thread never waits on mpv.
fn spawn_reader(api: Arc<MpvApi>, mpv: usize, shared: Arc<Shared>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let handle = mpv as *mut MpvHandle;
        while !shared.stop.load(Ordering::Acquire) {
            let event = unsafe { (api.wait_event)(handle, POSITION_POLL.as_secs_f64()) };
            // The event pointer is only valid until the next `mpv_wait_event`.
            if !event.is_null() {
                match unsafe { (*event).event_id } {
                    MPV_EVENT_SHUTDOWN => break,
                    MPV_EVENT_FILE_LOADED => {
                        let mut pending = shared
                            .pending_subs
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner);
                        let generation = shared.load_generation.load(Ordering::Acquire);
                        shared
                            .loaded_generation
                            .store(generation, Ordering::Release);
                        if let Some(path) = pending.take() {
                            let _ = queue_subtitles(&api, handle, &path);
                        }
                    }
                    _ => {}
                }
            }
            let mut value = 0f64;
            let status = unsafe {
                (api.get_property)(
                    handle,
                    c"time-pos".as_ptr(),
                    MPV_FORMAT_DOUBLE,
                    &mut value as *mut f64 as *mut c_void,
                )
            };
            let bits = if status >= 0 {
                value.to_bits()
            } else {
                f64::NAN.to_bits()
            };
            shared.position.store(bits, Ordering::Release);
        }
    })
}

pub struct MpvSurface {
    /// GPUI's NSView; its window is the parent of `child`.
    parent: *mut c_void,
    /// Unique mpv window title (contains the handle address) used to find `child`.
    title: String,
    /// mpv's NSWindow, attached as a child of `parent_window`. Null until found.
    child: *mut c_void,
    /// Retained while `child` is attached; released by `Drop`.
    parent_window: *mut c_void,
    api: Arc<MpvApi>,
    mpv: *mut MpvHandle,
    shared: Arc<Shared>,
    /// Taken by `Drop`, which joins it before terminating mpv.
    reader: Option<JoinHandle<()>>,
    path: String,
    /// Whether the child window is currently shown.
    visible: bool,
    /// Last screen-space frame applied to `child`.
    frame: Option<CGRect>,
    /// Host playback state last applied to mpv.
    playing: bool,
    rate: f64,
    seek_at: Option<Instant>,
    subtitle_revision: Option<u64>,
}

impl MpvSurface {
    /// Starts mpv with its own unique-titled window and loads `path`. The window
    /// is attached to GPUI's window by `place` once it exists.
    pub fn new(window: &Window, path: &str) -> Result<Self> {
        let parent = parent_ns_view(window)?;
        let api = Arc::new(MpvApi::load()?);
        let mpv = unsafe { (api.create)() };
        if mpv.is_null() {
            return Err(anyhow!("mpv_create failed"));
        }
        // From here on `Drop` owns cleanup, so any later `?` is safe.
        let mut surface = Self {
            parent,
            title: format!("aegisub-mpv-{:x}", mpv as usize),
            child: ptr::null_mut(),
            parent_window: ptr::null_mut(),
            api,
            mpv,
            shared: Arc::new(Shared::new()),
            reader: None,
            path: String::new(),
            visible: false,
            frame: None,
            playing: false,
            rate: 1.,
            seek_at: None,
            subtitle_revision: None,
        };
        surface.configure()?;
        surface.reader = Some(spawn_reader(
            Arc::clone(&surface.api),
            mpv as usize,
            Arc::clone(&surface.shared),
        ));
        surface.load_file(path)?;
        Ok(surface)
    }

    fn configure(&self) -> Result<()> {
        let options = [
            ("config", "no"),
            ("load-scripts", "no"),
            ("terminal", "no"),
            ("audio", "no"),
            ("pause", "yes"),
            ("keep-open", "yes"),
            ("osc", "no"),
            ("osd-level", "0"),
            ("sub-auto", "no"),
            ("input-default-bindings", "no"),
            ("vo", "gpu"),
            // Create the window during asynchronous load; init must not wait on AppKit.
            ("force-window", "yes"),
            ("title", self.title.as_str()),
        ];
        for (name, value) in options {
            let c_name = CString::new(name)?;
            let c_value = CString::new(value)?;
            let status = unsafe {
                (self.api.set_option_string)(self.mpv, c_name.as_ptr(), c_value.as_ptr())
            };
            check(status, &format!("mpv option {name}={value}"))?;
        }
        check(unsafe { (self.api.initialize)(self.mpv) }, "mpv_initialize")
    }

    /// Queues a command without waiting for mpv.
    fn command(&self, args: &[&str]) -> Result<()> {
        queue_command(&self.api, self.mpv, args)
    }

    fn set_pause(&self, paused: bool) -> Result<()> {
        self.command(&["set", "pause", if paused { "yes" } else { "no" }])
    }

    fn set_speed(&self, rate: f64) -> Result<()> {
        let value = rate.to_string();
        self.command(&["set", "speed", value.as_str()])
    }

    /// Last `time-pos` the reader thread saw, or `None` while no file is playing.
    fn position(&self) -> Option<f64> {
        let value = f64::from_bits(self.shared.position.load(Ordering::Acquire));
        (!value.is_nan()).then_some(value)
    }

    fn seek(&mut self, target: f64, now: Instant) {
        let text = format!("{target:.6}");
        // Queued without waiting. If the file is not loaded yet mpv rejects it;
        // the position stays drifted, so `sync` retries after SEEK_RETRY.
        let _ = self.command(&["seek", text.as_str(), "absolute+exact"]);
        self.seek_at = Some(now);
    }

    /// Attaches mpv's window as a borderless, mouse-transparent child of the
    /// GPUI window. Returns false if GPUI's window is not available yet.
    fn attach(&mut self, child: *mut c_void) -> bool {
        let parent_window = unsafe { msg_send::<SelObj>()(self.parent, selector(c"window")) };
        if parent_window.is_null() {
            return false;
        }
        unsafe {
            msg_send::<SelVoid>()(parent_window, selector(c"retain"));
            msg_send::<SelUsizeArg>()(child, selector(c"setStyleMask:"), 0);
            msg_send::<SelBool>()(child, selector(c"setIgnoresMouseEvents:"), 1);
            msg_send::<SelChildWindow>()(
                parent_window,
                selector(c"addChildWindow:ordered:"),
                child,
                NS_WINDOW_ABOVE,
            );
            msg_send::<SelObjArg>()(child, selector(c"orderOut:"), ptr::null_mut());
            // mpv may have taken key status when it created its window; give it back to GPUI.
            msg_send::<SelVoid>()(parent_window, selector(c"makeKeyWindow"));
        }
        self.child = child;
        self.parent_window = parent_window;
        self.visible = false;
        self.frame = None;
        true
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn subtitle_revision(&self) -> Option<u64> {
        self.subtitle_revision
    }

    /// Replaces the current file. Pauses first so the new file never starts
    /// playing unannounced; `sync` resumes it. Subtitles requested for the old
    /// file are dropped.
    pub fn load_file(&mut self, path: &str) -> Result<()> {
        self.set_pause(true)?;
        self.playing = false;
        {
            let mut pending = self
                .shared
                .pending_subs
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *pending = None;
            self.shared.load_generation.fetch_add(1, Ordering::AcqRel);
        }
        self.command(&["loadfile", path, "replace"])?;
        self.path = path.to_owned();
        self.seek_at = None;
        self.subtitle_revision = None;
        Ok(())
    }

    /// Makes the ASS file `path`, written by the host for `revision`, mpv's
    /// subtitle track. It is added now if the current file has loaded,
    /// otherwise on that file's FILE_LOADED event.
    pub fn load_subtitles(&mut self, path: &str, revision: u64) -> Result<()> {
        let mut pending = self
            .shared
            .pending_subs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.shared.file_loaded() {
            queue_subtitles(&self.api, self.mpv, path)?;
        } else {
            *pending = Some(path.to_owned());
        }
        self.subtitle_revision = Some(revision);
        Ok(())
    }

    /// Places mpv's window over `bounds` (GPUI window coordinates, converted to
    /// screen coordinates) and shows or hides it. Attaches the window the first
    /// time it is found; until then this is retried on every call. Only changed
    /// values reach AppKit.
    pub fn place(&mut self, bounds: Bounds<Pixels>, visible: bool) {
        if self.child.is_null() {
            let found = find_window(&self.title);
            if found.is_null() || !self.attach(found) {
                return;
            }
        }
        let parent_bounds = unsafe { view_bounds(self.parent) };
        let flipped = unsafe { msg_send::<SelBoolGet>()(self.parent, selector(c"isFlipped")) } != 0;
        let local = rect_from_bounds(bounds, parent_bounds.size.height, flipped);
        let in_window = unsafe { convert_rect(self.parent, c"convertRect:toView:", local) };
        let screen =
            unsafe { convert_rect(self.parent_window, c"convertRectToScreen:", in_window) };
        if self.frame != Some(screen) {
            unsafe {
                msg_send::<SelRectBool>()(self.child, selector(c"setFrame:display:"), screen, 1)
            };
            self.frame = Some(screen);
        }
        if self.visible != visible {
            let sel = if visible {
                c"orderFront:"
            } else {
                c"orderOut:"
            };
            unsafe { msg_send::<SelObjArg>()(self.child, selector(sel), ptr::null_mut()) };
            self.visible = visible;
        }
    }

    /// Brings mpv to the host's playback state. Called every render, but it only
    /// queues mpv commands on a change: a pause/play transition, a rate change,
    /// or drift while playing. Drift is judged from the reader thread's cached
    /// position, so this never waits on mpv. Paused frames are not driven here;
    /// the GPUI picture shows them.
    pub fn sync(&mut self, playing: bool, time_ms: i64, rate: f64, frame_secs: f64) {
        let rate = rate.max(0.01);
        if rate != self.rate && self.set_speed(rate).is_ok() {
            self.rate = rate;
        }
        if !playing {
            if self.playing && self.set_pause(true).is_ok() {
                self.playing = false;
            }
            return;
        }
        let target = time_ms as f64 / 1000. + frame_secs * 0.5;
        let now = Instant::now();
        if !self.playing {
            // Seek before unpausing so playback never starts from a stale position.
            self.seek(target, now);
            if self.set_pause(false).is_ok() {
                self.playing = true;
            }
            return;
        }
        let drifted = self
            .position()
            .map_or(true, |position| (position - target).abs() > PLAY_TOLERANCE);
        let retry = self
            .seek_at
            .map_or(true, |at| now.duration_since(at) >= SEEK_RETRY);
        if drifted && retry {
            self.seek(target, now);
        }
    }
}

impl Drop for MpvSurface {
    fn drop(&mut self) {
        if !self.child.is_null() {
            // Detach on the main thread now, so the child window is no longer
            // part of GPUI's window before mpv (on the helper thread) closes it.
            unsafe {
                msg_send::<SelObjArg>()(
                    self.parent_window,
                    selector(c"removeChildWindow:"),
                    self.child,
                );
                msg_send::<SelObjArg>()(self.child, selector(c"orderOut:"), ptr::null_mut());
                msg_send::<SelVoid>()(self.parent_window, selector(c"release"));
            }
        }
        let api = Arc::clone(&self.api);
        let shared = Arc::clone(&self.shared);
        let reader = self.reader.take();
        let mpv = self.mpv as usize;
        // Teardown may wait on mpv's core and on the video thread, which may
        // need the main thread. Stop and join the reader first so nothing uses
        // the handle after `mpv_terminate_destroy`, all on a helper thread.
        std::thread::spawn(move || {
            shared.stop.store(true, Ordering::Release);
            if let Some(reader) = reader {
                let _ = reader.join();
            }
            unsafe { (api.terminate)(mpv as *mut MpvHandle) };
        });
    }
}
