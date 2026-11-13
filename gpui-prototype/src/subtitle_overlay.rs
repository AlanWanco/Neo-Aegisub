//! The renderer's own subtitles drawn above mpv's picture.
//!
//! mpv paints video and its bundled libass subtitles on the GPU, which is the
//! only way to keep a high frame rate. With a CSRI renderer (VSFilter,
//! VSFilterMod) the host's subtitles are a different rendition, so playback
//! would show the wrong ones. This module owns a second native window, stacked
//! above mpv's, that draws the host's subtitle bitmap at the video's position:
//!
//! * macOS: a borderless, transparent `NSWindow` attached as a child of the
//!   GPUI window, above mpv's child window. Its content view keeps one
//!   `CALayer` whose `contents` is the bitmap as a `CGImage`.
//! * Windows: an owned popup `HWND` with `WS_EX_LAYERED`, painted with
//!   `UpdateLayeredWindow` from a top-down DIB section.
//!
//! The bitmap arrives premultiplied and already cropped to the drawn pixels
//! (`gpui_subtitles.cpp`), in video pixels; the layer scales it into the
//! picture rect it was placed over, so the subtitles land where the host
//! composited them.

#![cfg(any(target_os = "macos", target_os = "windows"))]

use anyhow::Result;
use gpui_kit::{Bounds, Pixels, Window};

/// Subtitles cut out of one video frame, in video pixels.
#[derive(Debug, Clone, Default)]
pub struct Image {
    /// Size of the frame the bitmap was cut from; the scale for the picture.
    pub video_width: u32,
    /// Only the width sets the scale: the picture keeps the video's aspect.
    #[allow(dead_code)]
    pub video_height: u32,
    /// Where the bitmap sits in that frame.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA8, `width * height * 4` bytes, top row first.
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn is_empty(&self) -> bool {
        self.width == 0
            || self.height == 0
            || self.rgba.len() < self.width as usize * self.height as usize * 4
    }
}

pub struct SubtitleLayer {
    layer: platform::Layer,
}

impl SubtitleLayer {
    pub fn new(window: &Window) -> Result<Self> {
        Ok(Self {
            layer: platform::Layer::new(window)?,
        })
    }

    /// Keeps the layer over `bounds` (the picture) and shows or hides it.
    pub fn place(&mut self, bounds: Bounds<Pixels>, visible: bool) {
        self.layer.place(bounds, visible);
    }

    /// Draws `image`, or nothing when it is `None` or empty.
    pub fn set_image(&mut self, image: Option<Image>) {
        self.layer.set_image(image);
    }

    /// Releases the native window. Main thread.
    pub fn detach(&mut self) {
        self.layer.detach();
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Image;
    use anyhow::{Result, anyhow};
    use gpui_kit::{Bounds, Pixels, Window};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::{CStr, c_char, c_void};
    use std::ptr;

    /// `NSWindowAbove` (NSWindowOrderingMode).
    const NS_WINDOW_ABOVE: isize = 1;
    /// `NSBackingStoreBuffered`.
    const NS_BACKING_BUFFERED: usize = 2;
    /// `kCGImageAlphaPremultipliedLast`: bytes are R, G, B, A.
    const ALPHA_PREMULTIPLIED_LAST: u32 = 1;

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

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
        fn CGColorSpaceRelease(space: *mut c_void);
        fn CGDataProviderCreateWithData(
            info: *mut c_void,
            data: *const c_void,
            size: usize,
            release: *const c_void,
        ) -> *mut c_void;
        fn CGDataProviderRelease(provider: *mut c_void);
        fn CGImageCreate(
            width: usize,
            height: usize,
            bits_per_component: usize,
            bits_per_pixel: usize,
            bytes_per_row: usize,
            space: *mut c_void,
            bitmap_info: u32,
            provider: *mut c_void,
            decode: *const f64,
            should_interpolate: bool,
            intent: u32,
        ) -> *mut c_void;
        fn CGImageRelease(image: *mut c_void);
    }

    type SelObj = unsafe extern "C" fn(*mut c_void, *const c_void) -> *mut c_void;
    type SelVoid = unsafe extern "C" fn(*mut c_void, *const c_void);
    type SelBool = unsafe extern "C" fn(*mut c_void, *const c_void, i8);
    type SelBoolGet = unsafe extern "C" fn(*mut c_void, *const c_void) -> i8;
    type SelObjArg = unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void);
    type SelCStrArg = unsafe extern "C" fn(*mut c_void, *const c_void, *const c_char) -> *mut c_void;
    type SelChildWindow = unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void, isize);
    type SelRectVoid = unsafe extern "C" fn(*mut c_void, *const c_void, CGRect);
    type SelRectBool = unsafe extern "C" fn(*mut c_void, *const c_void, CGRect, i8);
    type SelRectGet = unsafe extern "C" fn(*mut c_void, *const c_void) -> CGRect;
    /// `initWithContentRect:styleMask:backing:defer:`
    type SelWindowInit =
        unsafe extern "C" fn(*mut c_void, *const c_void, CGRect, usize, usize, i8) -> *mut c_void;
    /// A rect-in, rect-out message (`convertRect:toView:`, `convertRectToScreen:`).
    type SelRectCall =
        unsafe extern "C" fn(*mut c_void, *const c_void, CGRect, *mut c_void) -> CGRect;

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

    unsafe fn cls(name: &CStr) -> *mut c_void {
        unsafe { objc_getClass(name.as_ptr()) }
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

    unsafe fn convert_rect(receiver: *mut c_void, sel: &CStr, rect: CGRect) -> CGRect {
        #[cfg(target_arch = "x86_64")]
        {
            let mut out = CGRect::default();
            let send: unsafe extern "C" fn(
                *mut c_void,
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

    fn parent_ns_view(window: &Window) -> Result<*mut c_void> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| anyhow!("no native window handle: {error}"))?;
        match handle.as_raw() {
            RawWindowHandle::AppKit(handle) => Ok(handle.ns_view.as_ptr()),
            _ => Err(anyhow!("window is not AppKit-backed")),
        }
    }

    /// The bitmap currently uploaded. `CGDataProviderCreateWithData` does not
    /// take ownership, so the pixels stay here until the image is released.
    struct Content {
        image: *mut c_void,
        /// Backs the CGImage: the data provider borrows it without owning it.
        #[allow(dead_code)]
        pixels: Vec<u8>,
        /// Where the bitmap sits inside the placed rect, in points.
        frame: CGRect,
    }

    pub struct Layer {
        parent: *mut c_void,
        parent_window: *mut c_void,
        window: *mut c_void,
        /// The one sublayer that carries the bitmap.
        layer: *mut c_void,
        content: Option<Content>,
        /// Placed rect size in points, for the pixel-to-point scale.
        size: CGSize,
        visible: bool,
        frame: Option<CGRect>,
    }

    impl Layer {
        pub fn new(window: &Window) -> Result<Self> {
            let parent = parent_ns_view(window)?;
            unsafe {
                let parent_window = msg_send::<SelObj>()(parent, selector(c"window"));
                if parent_window.is_null() {
                    return Err(anyhow!("window is not attached yet"));
                }
                let alloc = msg_send::<SelObj>()(cls(c"NSWindow"), selector(c"alloc"));
                let create = msg_send::<SelWindowInit>();
                let native = create(
                    alloc,
                    selector(c"initWithContentRect:styleMask:backing:defer:"),
                    CGRect {
                        origin: CGPoint { x: 0., y: 0. },
                        size: CGSize {
                            width: 1.,
                            height: 1.,
                        },
                    },
                    0,
                    NS_BACKING_BUFFERED,
                    0,
                );
                if native.is_null() {
                    return Err(anyhow!("could not create the subtitle window"));
                }
                msg_send::<SelBool>()(native, selector(c"setOpaque:"), 0);
                msg_send::<SelBool>()(native, selector(c"setIgnoresMouseEvents:"), 1);
                msg_send::<SelBool>()(native, selector(c"setHasShadow:"), 0);
                let clear = msg_send::<SelObj>()(cls(c"NSColor"), selector(c"clearColor"));
                msg_send::<SelObjArg>()(native, selector(c"setBackgroundColor:"), clear);

                let view = msg_send::<SelObj>()(native, selector(c"contentView"));
                msg_send::<SelBool>()(view, selector(c"setWantsLayer:"), 1);
                let root = msg_send::<SelObj>()(view, selector(c"layer"));
                let layer = msg_send::<SelObj>()(cls(c"CALayer"), selector(c"layer"));
                let gravity = msg_send::<SelCStrArg>()(
                    cls(c"NSString"),
                    selector(c"stringWithUTF8String:"),
                    c"resize".as_ptr(),
                );
                msg_send::<SelObjArg>()(layer, selector(c"setContentsGravity:"), gravity);
                msg_send::<SelObjArg>()(root, selector(c"addSublayer:"), layer);

                msg_send::<SelVoid>()(parent_window, selector(c"retain"));
                msg_send::<SelChildWindow>()(
                    parent_window,
                    selector(c"addChildWindow:ordered:"),
                    native,
                    NS_WINDOW_ABOVE,
                );
                msg_send::<SelObjArg>()(native, selector(c"orderOut:"), ptr::null_mut());
                msg_send::<SelVoid>()(parent_window, selector(c"makeKeyWindow"));
                Ok(Self {
                    parent,
                    parent_window,
                    window: native,
                    layer,
                    content: None,
                    size: CGSize::default(),
                    visible: false,
                    frame: None,
                })
            }
        }

        pub fn place(&mut self, bounds: Bounds<Pixels>, visible: bool) {
            unsafe {
                let parent_bounds = view_bounds(self.parent);
                let flipped = msg_send::<SelBoolGet>()(self.parent, selector(c"isFlipped")) != 0;
                let x = f64::from(f32::from(bounds.origin.x));
                let y = f64::from(f32::from(bounds.origin.y));
                let width = f64::from(f32::from(bounds.size.width));
                let height = f64::from(f32::from(bounds.size.height));
                let y = if flipped {
                    y
                } else {
                    parent_bounds.size.height - y - height
                };
                let local = CGRect {
                    origin: CGPoint { x, y },
                    size: CGSize { width, height },
                };
                let in_window = convert_rect(self.parent, c"convertRect:toView:", local);
                let screen = convert_rect(self.parent_window, c"convertRectToScreen:", in_window);
                if self.frame != Some(screen) {
                    msg_send::<SelRectBool>()(
                        self.window,
                        selector(c"setFrame:display:"),
                        screen,
                        1,
                    );
                    self.frame = Some(screen);
                }
                if self.size != screen.size {
                    self.size = screen.size;
                    self.layout();
                }
                if self.visible != visible {
                    let sel = if visible { c"orderFront:" } else { c"orderOut:" };
                    msg_send::<SelObjArg>()(self.window, selector(sel), ptr::null_mut());
                    self.visible = visible;
                } else if visible {
                    // mpv raises its own window whenever it is shown again; the
                    // subtitles have to stay the front-most child.
                    msg_send::<SelObjArg>()(self.window, selector(c"orderFront:"), ptr::null_mut());
                }
            }
        }

        pub fn set_image(&mut self, image: Option<Image>) {
            let Some(image) = image.filter(|image| !image.is_empty()) else {
                self.drop_image();
                return;
            };
            let frame = self.frame_of(&image);
            let pixels = image.rgba;
            unsafe {
                let provider = CGDataProviderCreateWithData(
                    ptr::null_mut(),
                    pixels.as_ptr().cast(),
                    pixels.len(),
                    ptr::null(),
                );
                if provider.is_null() {
                    return;
                }
                let space = CGColorSpaceCreateDeviceRGB();
                let cg = CGImageCreate(
                    image.width as usize,
                    image.height as usize,
                    8,
                    32,
                    image.width as usize * 4,
                    space,
                    ALPHA_PREMULTIPLIED_LAST,
                    provider,
                    ptr::null(),
                    true,
                    0,
                );
                CGColorSpaceRelease(space);
                CGDataProviderRelease(provider);
                if cg.is_null() {
                    return;
                }
                self.drop_image();
                // No implicit animation: a new bitmap has to appear at once,
                // and Core Animation would otherwise cross-fade it. The layer
                // keeps its own reference to the image; this one stays with
                // `Content` so `drop_image` releases it exactly once.
                msg_send::<SelVoid>()(cls(c"CATransaction"), selector(c"begin"));
                msg_send::<SelBool>()(cls(c"CATransaction"), selector(c"setDisableActions:"), 1);
                msg_send::<SelRectVoid>()(self.layer, selector(c"setFrame:"), frame);
                msg_send::<SelObjArg>()(self.layer, selector(c"setContents:"), cg);
                msg_send::<SelVoid>()(cls(c"CATransaction"), selector(c"commit"));
                self.content = Some(Content {
                    image: cg,
                    pixels,
                    frame,
                });
            }
        }

        /// Where the bitmap goes inside the placed rect, in the content view's
        /// (bottom-left) coordinates.
        fn frame_of(&self, image: &Image) -> CGRect {
            let scale = if self.size.width > 0. && image.video_width > 0 {
                self.size.width / f64::from(image.video_width)
            } else {
                1.
            };
            CGRect {
                origin: CGPoint {
                    x: f64::from(image.x) * scale,
                    y: self.size.height - f64::from(image.y + image.height as i32) * scale,
                },
                size: CGSize {
                    width: f64::from(image.width) * scale,
                    height: f64::from(image.height) * scale,
                },
            }
        }

        fn layout(&mut self) {
            let Some(content) = &self.content else { return };
            let frame = content.frame;
            unsafe {
                msg_send::<SelVoid>()(cls(c"CATransaction"), selector(c"begin"));
                msg_send::<SelBool>()(cls(c"CATransaction"), selector(c"setDisableActions:"), 1);
                msg_send::<SelRectVoid>()(self.layer, selector(c"setFrame:"), frame);
                msg_send::<SelVoid>()(cls(c"CATransaction"), selector(c"commit"));
            }
        }

        fn drop_image(&mut self) {
            if let Some(content) = self.content.take() {
                unsafe {
                    // The layer's reference goes first; `Content` owns the other.
                    msg_send::<SelObjArg>()(self.layer, selector(c"setContents:"), ptr::null_mut());
                    CGImageRelease(content.image);
                }
            }
        }

        pub fn detach(&mut self) {
            self.drop_image();
            unsafe {
                msg_send::<SelObjArg>()(
                    self.parent_window,
                    selector(c"removeChildWindow:"),
                    self.window,
                );
                msg_send::<SelObjArg>()(self.window, selector(c"orderOut:"), ptr::null_mut());
                msg_send::<SelVoid>()(self.window, selector(c"close"));
                msg_send::<SelVoid>()(self.parent_window, selector(c"release"));
            }
            self.window = ptr::null_mut();
            self.layer = ptr::null_mut();
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::Image;
    use anyhow::{Result, anyhow};
    use gpui_kit::{Bounds, Pixels, Window};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::LazyLock;

    type Hwnd = *mut c_void;
    type WndProc = unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize;

    const WS_POPUP: u32 = 0x8000_0000;
    const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
    const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
    const WS_EX_LAYERED: u32 = 0x0008_0000;
    const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SW_HIDE: i32 = 0;
    const SW_SHOWNOACTIVATE: i32 = 4;
    const WM_NCHITTEST: u32 = 0x0084;
    const HTTRANSPARENT: isize = -1;
    const DEFAULT_DPI: u32 = 96;
    const ULW_ALPHA: u32 = 0x0000_0002;
    const AC_SRC_OVER: u8 = 0x00;
    const AC_SRC_ALPHA: u8 = 0x01;
    const DIB_RGB_COLORS: u32 = 0;
    const BI_RGB: u32 = 0;

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

    #[repr(C)]
    #[derive(Default)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct Size {
        width: i32,
        height: i32,
    }

    #[repr(C)]
    struct BitmapInfoHeader {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bit_count: u16,
        compression: u32,
        size_image: u32,
        x_pels_per_meter: i32,
        y_pels_per_meter: i32,
        clr_used: u32,
        clr_important: u32,
    }

    #[repr(C)]
    struct BitmapInfo {
        header: BitmapInfoHeader,
        colours: [u32; 1],
    }

    #[repr(C)]
    struct BlendFunction {
        blend_op: u8,
        blend_flags: u8,
        source_constant_alpha: u8,
        alpha_format: u8,
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
        fn DestroyWindow(hwnd: Hwnd) -> i32;
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
        fn GetDC(hwnd: Hwnd) -> *mut c_void;
        fn ReleaseDC(hwnd: Hwnd, dc: *mut c_void) -> i32;
        fn UpdateLayeredWindow(
            hwnd: Hwnd,
            dst: *mut c_void,
            dst_point: *const Point,
            size: *const Size,
            src: *mut c_void,
            src_point: *const Point,
            colour: u32,
            blend: *const BlendFunction,
            flags: u32,
        ) -> i32;
    }

    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn CreateCompatibleDC(dc: *mut c_void) -> *mut c_void;
        fn DeleteDC(dc: *mut c_void) -> i32;
        fn CreateDIBSection(
            dc: *mut c_void,
            info: *const BitmapInfo,
            usage: u32,
            bits: *mut *mut c_void,
            section: *mut c_void,
            offset: u32,
        ) -> *mut c_void;
        fn SelectObject(dc: *mut c_void, object: *mut c_void) -> *mut c_void;
        fn DeleteObject(object: *mut c_void) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn class_name() -> &'static [u16] {
        static NAME: LazyLock<Vec<u16>> = LazyLock::new(|| wide("AegisubSubtitleLayer"));
        &NAME
    }

    unsafe extern "system" fn layer_proc(
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
            wnd_proc: Some(layer_proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance: unsafe { GetModuleHandleW(ptr::null()) },
            icon: ptr::null_mut(),
            cursor: ptr::null_mut(),
            background: ptr::null_mut(),
            menu_name: ptr::null(),
            class_name: class_name().as_ptr(),
        };
        if unsafe { RegisterClassW(&class) } != 0 {
            return Ok(());
        }
        // The class outlives every layer this process makes: a second one is
        // registered already, and that is fine.
        match std::io::Error::last_os_error().raw_os_error() {
            Some(1410) => Ok(()),
            _ => Err(format!(
                "RegisterClassW failed: {}",
                std::io::Error::last_os_error()
            )),
        }
    }

    pub struct Layer {
        parent: Hwnd,
        window: Hwnd,
        mem_dc: *mut c_void,
        dib: *mut c_void,
        old_bitmap: *mut c_void,
        /// Placed rect origin in physical pixels.
        origin: Point,
        /// Placed rect size in physical pixels; the video scale.
        size: Size,
        /// Whether the caller wants the layer shown.
        visible: bool,
        /// Whether a bitmap is currently uploaded.
        painted: bool,
    }

    impl Layer {
        pub fn new(window: &Window) -> Result<Self> {
            let handle = HasWindowHandle::window_handle(window)
                .map_err(|error| anyhow!("no native window handle: {error}"))?;
            let parent = match handle.as_raw() {
                RawWindowHandle::Win32(handle) => handle.hwnd.get() as Hwnd,
                _ => return Err(anyhow!("window is not Win32-backed")),
            };
            register_class().map_err(|error| anyhow!(error))?;
            let native = unsafe {
                CreateWindowExW(
                    WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT,
                    class_name().as_ptr(),
                    ptr::null(),
                    WS_POPUP,
                    0,
                    0,
                    1,
                    1,
                    parent,
                    ptr::null_mut(),
                    GetModuleHandleW(ptr::null()),
                    ptr::null(),
                )
            };
            if native.is_null() {
                return Err(anyhow!(
                    "CreateWindowExW failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let mem_dc = unsafe { CreateCompatibleDC(ptr::null_mut()) };
            if mem_dc.is_null() {
                unsafe { DestroyWindow(native) };
                return Err(anyhow!("CreateCompatibleDC failed"));
            }
            Ok(Self {
                parent,
                window: native,
                mem_dc,
                dib: ptr::null_mut(),
                old_bitmap: ptr::null_mut(),
                origin: Point::default(),
                size: Size {
                    width: 1,
                    height: 1,
                },
                visible: false,
                painted: false,
            })
        }

        pub fn place(&mut self, bounds: Bounds<Pixels>, visible: bool) {
            let dpi = unsafe { GetDpiForWindow(self.parent) }.max(DEFAULT_DPI);
            let scale = f64::from(dpi) / f64::from(DEFAULT_DPI);
            // GPUI works in logical pixels; the window manager and
            // `ClientToScreen` take physical ones.
            let mut point = Point {
                x: (f64::from(f32::from(bounds.origin.x)) * scale).round() as i32,
                y: (f64::from(f32::from(bounds.origin.y)) * scale).round() as i32,
            };
            let width = f32::from(bounds.size.width).max(1.);
            let height = f32::from(bounds.size.height).max(1.);
            unsafe {
                ClientToScreen(self.parent, &mut point);
                self.origin = point;
                self.size = Size {
                    width: (f64::from(width) * scale).round() as i32,
                    height: (f64::from(height) * scale).round() as i32,
                };
                SetWindowPos(
                    self.window,
                    ptr::null_mut(),
                    self.origin.x,
                    self.origin.y,
                    self.size.width,
                    self.size.height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            self.visible = visible;
            self.apply();
        }

        pub fn set_image(&mut self, image: Option<Image>) {
            let Some(image) = image.filter(|image| !image.is_empty()) else {
                self.painted = false;
                self.apply();
                return;
            };
            // Premultiplied RGBA from the host; `UpdateLayeredWindow` wants
            // premultiplied BGRA.
            let mut pixels = image.rgba;
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            let mut bits: *mut c_void = ptr::null_mut();
            unsafe {
                self.replace_dib(image.width as i32, image.height as i32, &mut bits);
                if bits.is_null() {
                    return;
                }
                ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
                // The picture rect is the placed window; place the crop in it
                // with the same scale mpv uses for the video.
                let scale = f64::from(self.size.width) / f64::from(image.video_width.max(1));
                let dst = Point {
                    x: self.origin.x + (f64::from(image.x) * scale).round() as i32,
                    y: self.origin.y + (f64::from(image.y) * scale).round() as i32,
                };
                let size = Size {
                    width: ((f64::from(image.width) * scale).round() as i32).max(1),
                    height: ((f64::from(image.height) * scale).round() as i32).max(1),
                };
                let src = Point::default();
                let blend = BlendFunction {
                    blend_op: AC_SRC_OVER,
                    blend_flags: 0,
                    source_constant_alpha: 0,
                    alpha_format: AC_SRC_ALPHA,
                };
                let screen = GetDC(ptr::null_mut());
                let updated = UpdateLayeredWindow(
                    self.window,
                    screen,
                    &dst,
                    &size,
                    self.mem_dc,
                    &src,
                    0,
                    &blend,
                    ULW_ALPHA,
                );
                ReleaseDC(ptr::null_mut(), screen);
                self.painted = updated != 0;
            }
            self.apply();
        }

        /// Shows the layer only while the caller wants it and it has something
        /// to show: a layered window keeps its last pixels while hidden.
        fn apply(&mut self) {
            unsafe {
                ShowWindow(
                    self.window,
                    if self.visible && self.painted {
                        SW_SHOWNOACTIVATE
                    } else {
                        SW_HIDE
                    },
                );
            }
        }

        unsafe fn replace_dib(&mut self, width: i32, height: i32, bits: &mut *mut c_void) {
            if !self.dib.is_null() {
                unsafe {
                    SelectObject(self.mem_dc, self.old_bitmap);
                    DeleteObject(self.dib);
                }
                self.dib = ptr::null_mut();
            }
            let info = BitmapInfo {
                // Negative height: top-down rows, the order the host sends.
                header: BitmapInfoHeader {
                    size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                    width,
                    height: -height,
                    planes: 1,
                    bit_count: 32,
                    compression: BI_RGB,
                    size_image: 0,
                    x_pels_per_meter: 0,
                    y_pels_per_meter: 0,
                    clr_used: 0,
                    clr_important: 0,
                },
                colours: [0],
            };
            let dib =
                unsafe { CreateDIBSection(self.mem_dc, &info, DIB_RGB_COLORS, bits, ptr::null_mut(), 0) };
            if dib.is_null() {
                return;
            }
            self.dib = dib;
            self.old_bitmap = unsafe { SelectObject(self.mem_dc, dib) };
        }

        pub fn detach(&mut self) {
            unsafe {
                ShowWindow(self.window, SW_HIDE);
                if !self.dib.is_null() {
                    SelectObject(self.mem_dc, self.old_bitmap);
                    DeleteObject(self.dib);
                    self.dib = ptr::null_mut();
                }
                if !self.mem_dc.is_null() {
                    DeleteDC(self.mem_dc);
                    self.mem_dc = ptr::null_mut();
                }
                if !self.window.is_null() {
                    DestroyWindow(self.window);
                    self.window = ptr::null_mut();
                }
            }
        }
    }
}
