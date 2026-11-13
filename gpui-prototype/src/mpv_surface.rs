//! libmpv native surface (experimental) for macOS and Windows.
//!
//! The shared part here owns the libmpv API, the command queue, the reader
//! thread and teardown. Each platform module (`macos`, `windows`) provides a
//! `Video` that owns the native window mpv draws into, plus `retire`.
//!
//! macOS: libmpv draws into its own NSWindow, which is attached as a child
//! window of the GPUI window. The window is borderless, ignores mouse events and
//! is placed over the preview area in screen coordinates. It is shown while the
//! video plays and hidden (`orderOut`) when paused or editing, when the GPUI
//! picture (host frame with libass subtitles) is shown instead. The mpv content
//! is never reparented into an NSView, so its Vulkan/Metal surface stays intact.
//! mpv is configured with a unique `title` (it contains the handle address) so
//! the window it creates can be found among `NSApp windows`. Its creation is
//! asynchronous, so `place` retries the lookup on each render until it appears.
//!
//! Windows: a container HWND (class `AegisubMpvVideo`) is created as an owned
//! popup of GPUI's HWND before mpv starts, and mpv is given it with the `wid`
//! option, so mpv renders into the container. A child window cannot be used:
//! DWM stops compositing mpv's flip-model presents inside GPUI's
//! DirectComposition window and the picture freezes. The container is layered
//! with an explicit alpha and `WS_EX_TRANSPARENT`, and it answers
//! `HTTRANSPARENT` to `WM_NCHITTEST`, so hit testing and clicks stay on GPUI.
//! The container is placed in screen pixels (client origin plus logical bounds
//! times the window DPI over 96, clipped to the client area) and hidden with
//! `ShowWindow` when paused or editing.
//!
//! Threads: after initialization the GPUI main thread never waits on mpv.
//! Commands go through `mpv_command_async`, and `time-pos` is read by a reader
//! thread that also drains mpv's event queue (FILE_LOADED tells it when
//! subtitles can be added). Teardown runs on a helper thread: it stops and
//! joins the reader, then `mpv_terminate_destroy` (which waits for mpv's video
//! thread, which may need the main thread). Native windows are released after
//! that, on their owning thread.

use anyhow::{Result, anyhow};
use gpui_kit::{Bounds, Pixels, Window};
use libloading::Library;
use std::ffi::{CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[cfg(any(target_os = "macos", target_os = "windows"))]
use platform::{Video, retire};

const MPV_FORMAT_DOUBLE: c_int = 5;
const MPV_ERROR_OPTION_NOT_FOUND: c_int = -5;
const MPV_EVENT_SHUTDOWN: c_int = 1;
const MPV_EVENT_FILE_LOADED: c_int = 8;
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
    error: c_int,
    reply_userdata: u64,
    data: *const c_void,
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

/// Library paths tried in order. The executable's own package location comes
/// first (macOS `Contents/Frameworks`, Windows the executable directory), then
/// the executable directory, then development loader fallbacks.
fn library_candidates() -> Vec<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let mut paths = Vec::new();
    #[cfg(target_os = "macos")]
    {
        if let Some(contents) = exe_dir.as_deref().and_then(Path::parent) {
            paths.push(contents.join("Frameworks/libmpv.2.dylib"));
        }
        if let Some(dir) = &exe_dir {
            paths.push(dir.join("libmpv.2.dylib"));
        }
        for name in [
            "/opt/homebrew/lib/libmpv.2.dylib",
            "libmpv.2.dylib",
            "libmpv.dylib",
        ] {
            paths.push(PathBuf::from(name));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(dir) = &exe_dir {
            paths.push(dir.join("libmpv-2.dll"));
            paths.push(dir.join("mpv-2.dll"));
        }
        for name in ["libmpv-2.dll", "mpv-2.dll"] {
            paths.push(PathBuf::from(name));
        }
    }
    paths
}

fn load_library() -> Result<Library> {
    let mut errors = Vec::new();
    for path in library_candidates() {
        match unsafe { Library::new(&path) } {
            Ok(library) => return Ok(library),
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    Err(anyhow!("libmpv not found: {}", errors.join("; ")))
}

impl MpvApi {
    fn load() -> Result<Self> {
        let library = load_library()?;
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

/// The loaded API, or the load error. Loaded once per process: a failed load is
/// not retried on every render.
fn mpv_api() -> Result<Arc<MpvApi>> {
    static API: LazyLock<std::result::Result<Arc<MpvApi>, String>> = LazyLock::new(|| {
        MpvApi::load()
            .map(Arc::new)
            .map_err(|error| format!("{error:#}"))
    });
    API.clone().map_err(|message| anyhow!("{message}"))
}

/// State shared between the main thread and the reader thread.
struct Shared {
    /// `f64` bits of the last `time-pos`; NaN while no file is playing.
    position: AtomicU64,
    /// Bumped by `load_file` before `loadfile` is queued.
    load_generation: AtomicU64,
    /// `load_generation` at the last FILE_LOADED event seen by the reader.
    loaded_generation: AtomicU64,
    /// `f64` bits of the seconds the host wants on screen; NaN until the first
    /// `sync`. mpv drops a seek queued while a file is still loading, so the
    /// reader replays this once FILE_LOADED arrives.
    desired_position: AtomicU64,
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
            desired_position: AtomicU64::new(f64::NAN.to_bits()),
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
            let event_id = if event.is_null() {
                -1
            } else {
                // The event pointer is only valid until the next `mpv_wait_event`.
                unsafe { (*event).event_id }
            };
            if event_id == MPV_EVENT_SHUTDOWN {
                break;
            }
            if event_id == MPV_EVENT_FILE_LOADED {
                let mut pending = shared
                    .pending_subs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let generation = shared.load_generation.load(Ordering::Acquire);
                shared
                    .loaded_generation
                    .store(generation, Ordering::Release);
                // A seek queued before this event is dropped by mpv, which
                // left the picture on the first frame while the timeline sat
                // at the restored cursor. Replay the host's position now.
                let desired = f64::from_bits(shared.desired_position.load(Ordering::Acquire));
                if desired.is_finite() {
                    let text = format!("{desired:.6}");
                    let _ = queue_command(&api, handle, &["seek", text.as_str(), "absolute+exact"]);
                }
                if let Some(path) = pending.take() {
                    let _ = queue_subtitles(&api, handle, &path);
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
    /// Native window mpv draws into; platform specific.
    video: Video,
    api: Arc<MpvApi>,
    mpv: *mut MpvHandle,
    shared: Arc<Shared>,
    /// Taken by `Drop`, which joins it before terminating mpv.
    reader: Option<JoinHandle<()>>,
    path: String,
    /// Host playback state last applied to mpv.
    playing: bool,
    rate: f64,
    seek_at: Option<Instant>,
    /// Host time (ms) of the paused position already sent to mpv, so a paused
    /// seek is queued once instead of on every render.
    paused_at: Option<i64>,
    subtitle_revision: Option<u64>,
    /// Letterbox colour, as `0xRRGGBB`, matching the card the window covers.
    background: u32,
    /// Aspect, logarithmic zoom, and pan already queued to mpv.
    view: Option<[f64; 4]>,
}

impl MpvSurface {
    /// Starts mpv with its native window and loads `path`. The window is
    /// created before mpv initializes (Windows hands it to mpv as `wid`); on
    /// macOS it is attached by `place` once mpv has created it.
    pub fn new(window: &Window, path: &str, background: u32) -> Result<Self> {
        let api = mpv_api()?;
        let mpv = unsafe { (api.create)() };
        if mpv.is_null() {
            return Err(anyhow!("mpv_create failed"));
        }
        let video = match Video::create(window, mpv as usize) {
            Ok(video) => video,
            Err(error) => {
                unsafe { (api.terminate)(mpv) };
                return Err(error);
            }
        };
        // From here on `Drop` owns cleanup, so any later `?` is safe.
        let mut surface = Self {
            video,
            api,
            mpv,
            shared: Arc::new(Shared::new()),
            reader: None,
            path: String::new(),
            playing: false,
            rate: 1.,
            seek_at: None,
            paused_at: None,
            subtitle_revision: None,
            background,
            view: None,
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
        let mut options: Vec<(String, String)> = [
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
            // Native pan and zoom require aspect handling. The workspace
            // supplies its display aspect and the same fitted view transform.
            ("keepaspect", "yes"),
            // Create the window during asynchronous load; init must not wait on the UI thread.
            ("force-window", "yes"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
        options.push((
            "background-color".to_owned(),
            format!("#{:06x}", self.background),
        ));
        options.extend(
            self.video
                .options()
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value)),
        );
        for (name, value) in &options {
            let c_name = CString::new(name.as_str())?;
            let c_value = CString::new(value.as_str())?;
            let status = unsafe {
                (self.api.set_option_string)(self.mpv, c_name.as_ptr(), c_value.as_ptr())
            };
            // Without script engines these features are absent, as are their options.
            if status == MPV_ERROR_OPTION_NOT_FOUND {
                continue;
            }
            check(status, &format!("mpv option {name}={value}"))?;
        }
        check(unsafe { (self.api.initialize)(self.mpv) }, "mpv_initialize")?;
        Ok(())
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

    /// Queues a seek and reports whether mpv took the command. Before the file
    /// is loaded mpv rejects it; `sync` then retries after `SEEK_RETRY`.
    fn seek(&mut self, target: f64, now: Instant) -> bool {
        let text = format!("{target:.6}");
        let queued = self
            .command(&["seek", text.as_str(), "absolute+exact"])
            .is_ok();
        if queued {
            self.seek_at = Some(now);
        }
        queued
    }

    /// Slack when comparing `time-pos` with a time on the host clock.
    fn settle_tolerance(frame_secs: f64) -> f64 {
        frame_secs.max(0.05)
    }

    /// Recolours the letterbox under the picture, which mpv paints itself: the
    /// window covers the card, so a black border would not match a light theme.
    pub fn set_background(&mut self, background: u32) {
        if background == self.background {
            return;
        }
        let value = format!("#{background:06x}");
        if self
            .command(&["set", "background-color", value.as_str()])
            .is_ok()
        {
            self.background = background;
        }
    }

    pub fn set_view(&mut self, aspect: f64, zoom: f32, pan: (f32, f32)) {
        let values = [aspect, f64::from(zoom.log2()), f64::from(pan.0), f64::from(pan.1)];
        if self.view == Some(values) {
            return;
        }
        for (index, property) in ["video-aspect-override", "video-zoom", "video-pan-x", "video-pan-y"]
            .into_iter().enumerate()
        {
            if self.view.is_some_and(|previous| previous[index] == values[index]) {
                continue;
            }
            if self.command(&["set", property, &values[index].to_string()]).is_err() {
                return;
            }
        }
        self.view = Some(values);
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
        self.paused_at = None;
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

    /// Drops mpv's external subtitle track: a CSRI renderer's subtitles reach
    /// the screen through the host's layer instead, and mpv's own libass would
    /// draw a second, differently rendered copy on top.
    pub fn clear_subtitles(&mut self) {
        let mut pending = self
            .shared
            .pending_subs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let had_pending = pending.take().is_some();
        let had_track = self.subtitle_revision.take().is_some();
        if self.shared.file_loaded() && (had_pending || had_track) {
            let _ = queue_command(&self.api, self.mpv, &["sub-remove"]);
        }
    }

    /// Places mpv's native window over `bounds` (GPUI window coordinates) and
    /// shows or hides it. Only changed values reach the OS.
    pub fn place(&mut self, bounds: Bounds<Pixels>, visible: bool) {
        self.video.place(bounds, visible);
    }

    /// Brings mpv to the host's playback state. Called every render, but it only
    /// queues mpv commands on a change: a pause/play transition, a rate change,
    /// or drift while playing. Drift is judged from the reader thread's cached
    /// position, so this never waits on mpv. While paused mpv keeps the picture
    /// up, so seeks made then are queued here as well.
    pub fn sync(&mut self, playing: bool, time_ms: i64, rate: f64, frame_secs: f64) {
        let rate = rate.max(0.01);
        if rate != self.rate && self.set_speed(rate).is_ok() {
            self.rate = rate;
        }
        // Match the host's frame timestamp. Adding half a frame can make an
        // exact seek select the next frame when switching preview renderers.
        let target = time_ms as f64 / 1000.;
        self.shared
            .desired_position
            .store(target.to_bits(), Ordering::Release);
        let now = Instant::now();
        if !playing {
            if self.playing && self.set_pause(true).is_ok() {
                self.playing = false;
            }
            // mpv keeps the picture up while paused, so it has to follow the
            // host here too: a paused seek or a frame step would otherwise
            // leave the picture where playback stopped. A seek queued before
            // FILE_LOADED is dropped by mpv while the async API still reports
            // it as sent, so recording it as applied would leave a restored
            // cursor showing the first frame; the reader repeats
            // `desired_position` once the file is in.
            if !self.shared.file_loaded() {
                return;
            }
            let retry = self
                .seek_at
                .map_or(true, |at| now.duration_since(at) >= SEEK_RETRY);
            if self.paused_at != Some(time_ms) && retry && self.seek(target, now) {
                self.paused_at = Some(time_ms);
            }
            return;
        }
        self.paused_at = None;
        if !self.playing {
            // A resume from the frame mpv already holds needs no seek: seeking
            // there would repaint from the preceding keyframe and show an
            // older picture for a moment. Only a position a paused seek left
            // behind needs one, and that seek was queued above.
            let moved = self.position().map_or(true, |position| {
                (position - target).abs() > Self::settle_tolerance(frame_secs)
            });
            if moved {
                self.seek(target, now);
            }
            if self.set_pause(false).is_ok() {
                self.playing = true;
            }
            return;
        }
        let position = self.position();
        let drifted = position.map_or(true, |position| (position - target).abs() > PLAY_TOLERANCE);
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
        // Main thread: take the native window out of GPUI's window hierarchy
        // and hide it. The returned token is released after mpv has closed.
        let token = self.video.detach();
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
            retire(token);
        });
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use anyhow::{Result, anyhow};
    use gpui_kit::{Bounds, Pixels, Window};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::{CStr, c_char, c_void};
    use std::ptr;

    /// `NSWindowAbove` (NSWindowOrderingMode).
    const NS_WINDOW_ABOVE: isize = 1;

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
    type SelRectCall =
        unsafe extern "C" fn(*mut c_void, *const c_void, CGRect, *mut c_void) -> CGRect;

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

    /// mpv's NSWindow, attached to GPUI's window once it exists.
    pub struct Video {
        /// GPUI's NSView; its window is the parent of `child`.
        parent: *mut c_void,
        /// Unique mpv window title (contains the handle address) used to find `child`.
        title: String,
        /// mpv's NSWindow, attached as a child of `parent_window`. Null until found.
        child: *mut c_void,
        /// Retained while `child` is attached; released by `detach`.
        parent_window: *mut c_void,
        /// Whether the child window is currently shown.
        visible: bool,
        /// Last screen-space frame applied to `child`.
        frame: Option<CGRect>,
    }

    impl Video {
        pub fn create(window: &Window, id: usize) -> Result<Self> {
            Ok(Self {
                parent: parent_ns_view(window)?,
                title: format!("aegisub-mpv-{id:x}"),
                child: ptr::null_mut(),
                parent_window: ptr::null_mut(),
                visible: false,
                frame: None,
            })
        }

        /// mpv options that route its picture to this window.
        pub fn options(&self) -> Vec<(&'static str, String)> {
            vec![("title", self.title.clone())]
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

        /// Places mpv's window over `bounds` and shows or hides it. Attaches the
        /// window the first time it is found; until then this is retried on every
        /// call. Only changed values reach AppKit.
        pub fn place(&mut self, bounds: Bounds<Pixels>, visible: bool) {
            if self.child.is_null() {
                let found = find_window(&self.title);
                if found.is_null() || !self.attach(found) {
                    return;
                }
            }
            let parent_bounds = unsafe { view_bounds(self.parent) };
            let flipped =
                unsafe { msg_send::<SelBoolGet>()(self.parent, selector(c"isFlipped")) } != 0;
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

        /// Detaches the window from GPUI on the main thread. Returns a token for
        /// `retire`; macOS needs no later step.
        pub fn detach(&mut self) -> usize {
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
            0
        }
    }

    /// Releases what `Video::detach` left behind; nothing on macOS.
    pub fn retire(_token: usize) {}
}

#[cfg(target_os = "windows")]
mod platform {
    //! An owned popup HWND that mpv renders into through `wid`, kept over the
    //! video area of GPUI's window.
    //!
    //! A `WS_CHILD` container cannot be used: mpv presents through a DXGI
    //! flip-model swapchain, and DWM stops compositing those presents once the
    //! window sits inside GPUI's `WS_EX_NOREDIRECTIONBITMAP` (DirectComposition)
    //! window, so the picture freezes on the first frames while mpv keeps
    //! decoding at full rate. An owned popup presents on its own surface, which
    //! is also how the AppKit backend shows video (a separate NSWindow). Being
    //! owned keeps it above the workspace, in front of it after activation and
    //! hidden with it. The popup is layered and transparent with an explicit
    //! alpha, and it answers `HTTRANSPARENT` to `WM_NCHITTEST`, so every click
    //! on the video area keeps going to GPUI.

    use anyhow::{Result, anyhow};
    use gpui_kit::{Bounds, Pixels, Window};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::LazyLock;

    type Hwnd = *mut c_void;
    type WndProc = unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize;

    const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
    const WS_CLIPCHILDREN: u32 = 0x0200_0000;
    const WS_POPUP: u32 = 0x8000_0000;
    const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
    const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
    const WS_EX_LAYERED: u32 = 0x0008_0000;
    const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
    const LWA_ALPHA: u32 = 0x0000_0002;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SW_HIDE: i32 = 0;
    const SW_SHOWNOACTIVATE: i32 = 4;
    const WM_CLOSE: u32 = 0x0010;
    const WM_NCHITTEST: u32 = 0x0084;
    const HTTRANSPARENT: isize = -1;
    const DEFAULT_DPI: u32 = 96;
    const INITIAL_SIZE: i32 = 640;

    #[repr(C)]
    struct WndClassW {
        style: u32,
        wnd_proc: Option<WndProc>,
        cls_extra: i32,
        wnd_extra: i32,
        instance: *mut c_void,
        icon: *mut c_void,
        cursor: *mut c_void,
        background: *mut c_void,
        menu_name: *const u16,
        class_name: *const u16,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterClassW(class: *const WndClassW) -> u16;
        fn CreateWindowExW(
            ex_style: u32,
            class_name: *const u16,
            window_name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: Hwnd,
            menu: *mut c_void,
            instance: *mut c_void,
            param: *const c_void,
        ) -> Hwnd;
        fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
        fn SetWindowPos(
            hwnd: Hwnd,
            insert_after: Hwnd,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
        fn GetDpiForWindow(hwnd: Hwnd) -> u32;
        fn ClientToScreen(hwnd: Hwnd, point: *mut Point) -> i32;
        fn GetClientRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
        fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
        fn SetLayeredWindowAttributes(hwnd: Hwnd, colour: u32, alpha: u8, flags: u32) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn class_name() -> &'static [u16] {
        static NAME: LazyLock<Vec<u16>> = LazyLock::new(|| wide("AegisubMpvVideo"));
        &NAME
    }

    fn module_instance() -> *mut c_void {
        unsafe { GetModuleHandleW(ptr::null()) }
    }

    /// The container never takes the mouse: hit testing has to go to GPUI
    /// underneath it, which also receives the clicks.
    unsafe extern "system" fn container_proc(
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        if message == WM_NCHITTEST {
            return HTTRANSPARENT;
        }
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    fn register_class() -> std::result::Result<(), String> {
        let class = WndClassW {
            style: 0,
            wnd_proc: Some(container_proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance: module_instance(),
            icon: ptr::null_mut(),
            cursor: ptr::null_mut(),
            background: ptr::null_mut(),
            menu_name: ptr::null(),
            class_name: class_name().as_ptr(),
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err("RegisterClassW failed for AegisubMpvVideo".to_owned());
        }
        Ok(())
    }

    fn parent_hwnd(window: &Window) -> Result<Hwnd> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| anyhow!("no native window handle: {error}"))?;
        match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get() as Hwnd),
            _ => Err(anyhow!("window is not Win32-backed")),
        }
    }

    /// Converts logical bounds to physical client pixels of the parent window.
    /// Both edges are rounded, not the origin and the size: GPUI paints the
    /// picture at fractional device pixels, so rounding each of those would
    /// move the right and bottom edges by up to a pixel and the window would
    /// visibly change size when it takes over from the GPUI picture.
    fn physical_frame(bounds: Bounds<Pixels>, scale: f64) -> (i32, i32, i32, i32) {
        let scaled = |value: Pixels| f64::from(f32::from(value)) * scale;
        let left = scaled(bounds.origin.x).round() as i32;
        let top = scaled(bounds.origin.y).round() as i32;
        let right = scaled(bounds.origin.x + bounds.size.width).round() as i32;
        let bottom = scaled(bounds.origin.y + bounds.size.height).round() as i32;
        (left, top, (right - left).max(1), (bottom - top).max(1))
    }

    /// The popup HWND that mpv renders into, owned by GPUI's window.
    pub struct Video {
        parent: Hwnd,
        hwnd: Hwnd,
        visible: bool,
        /// Screen rectangle of the last placement and the parent's client
        /// origin, which moves without changing `bounds`.
        placed: Option<((i32, i32), (i32, i32, i32, i32))>,
    }

    impl Video {
        pub fn create(window: &Window, _id: usize) -> Result<Self> {
            let parent = parent_hwnd(window)?;
            static REGISTERED: LazyLock<std::result::Result<(), String>> =
                LazyLock::new(register_class);
            REGISTERED.clone().map_err(|error| anyhow!("{error}"))?;
            let ex_style = WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT;
            let style = WS_POPUP | WS_CLIPSIBLINGS | WS_CLIPCHILDREN;
            // Created hidden; `place` shows it once the video is playing.
            let hwnd = unsafe {
                CreateWindowExW(
                    ex_style,
                    class_name().as_ptr(),
                    ptr::null(),
                    style,
                    0,
                    0,
                    INITIAL_SIZE,
                    INITIAL_SIZE,
                    parent,
                    ptr::null_mut(),
                    module_instance(),
                    ptr::null(),
                )
            };
            if hwnd.is_null() {
                return Err(anyhow!("CreateWindowExW failed for the mpv video window"));
            }
            // A layered window is only click-through once it has an explicit
            // alpha; without one hit-testing treats it as opaque.
            unsafe { SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA) };
            Ok(Self {
                parent,
                hwnd,
                visible: false,
                placed: None,
            })
        }

        /// `wid` makes mpv render into the container.
        pub fn options(&self) -> Vec<(&'static str, String)> {
            vec![("wid", (self.hwnd as usize).to_string())]
        }

        /// Places the popup over `bounds` and shows or hides it.
        ///
        /// `bounds` are logical pixels of the keyboard focus window and cover
        /// the picture itself, not the card around it: the host paints that
        /// same rectangle, so playback handing the picture between the two
        /// renderers cannot resize or move it. A popup is positioned in screen
        /// pixels, so the bounds are scaled by the parent's DPI and offset by
        /// its client origin. The result is clipped to the client area, which a
        /// child window would have done by itself.
        pub fn place(&mut self, bounds: Bounds<Pixels>, visible: bool) {
            let dpi = unsafe { GetDpiForWindow(self.parent) };
            let dpi = if dpi == 0 { DEFAULT_DPI } else { dpi };
            let scale = f64::from(dpi) / f64::from(DEFAULT_DPI);
            let (x, y, width, height) = physical_frame(bounds, scale);
            let mut origin = Point { x: 0, y: 0 };
            unsafe { ClientToScreen(self.parent, &mut origin) };
            let mut client = Rect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            unsafe { GetClientRect(self.parent, &mut client) };
            let left = (origin.x + x).max(origin.x);
            let top = (origin.y + y).max(origin.y);
            let right = (origin.x + x + width).min(origin.x + client.right);
            let bottom = (origin.y + y + height).min(origin.y + client.bottom);
            let frame = (left, top, (right - left).max(0), (bottom - top).max(0));
            let placed = ((origin.x, origin.y), frame);
            let fits = frame.2 > 0 && frame.3 > 0;
            if fits && self.placed != Some(placed) {
                unsafe {
                    SetWindowPos(
                        self.hwnd,
                        ptr::null_mut(),
                        frame.0,
                        frame.1,
                        frame.2,
                        frame.3,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
                self.placed = Some(placed);
            }
            let visible = visible && fits;
            if self.visible != visible {
                let cmd = if visible { SW_SHOWNOACTIVATE } else { SW_HIDE };
                unsafe { ShowWindow(self.hwnd, cmd) };
                self.visible = visible;
            }
        }

        /// Hides the container on the main thread. Returns the token that
        /// `retire` needs once mpv has closed its window.
        pub fn detach(&mut self) -> usize {
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            self.visible = false;
            self.hwnd as usize
        }
    }

    /// Destroys the container. Posted rather than called because only the
    /// thread that created a window may destroy it; `WM_CLOSE` reaches the
    /// container's procedure on the GPUI thread, where DefWindowProc destroys it.
    pub fn retire(token: usize) {
        if token != 0 {
            unsafe { PostMessageW(token as Hwnd, WM_CLOSE, 0, 0) };
        }
    }
}
