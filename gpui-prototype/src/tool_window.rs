use anyhow::{Result, bail};
use gpui_kit::Window;

/// Keep a tool above its owning workspace without making either window modal.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn attach(child: &Window, parent: &Window) -> Result<()> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let native = |window: &Window| {
        HasWindowHandle::window_handle(window)
            .map(|handle| handle.as_raw())
            .map_err(|error| anyhow::anyhow!("no native window handle: {error}"))
    };
    match (native(child)?, native(parent)?) {
        #[cfg(target_os = "macos")]
        (RawWindowHandle::AppKit(child), RawWindowHandle::AppKit(parent)) => {
            use std::ffi::{c_char, c_void};

            #[link(name = "objc")]
            unsafe extern "C" {
                fn sel_registerName(name: *const c_char) -> *const c_void;
                fn objc_msgSend();
            }
            // SAFETY: Both GPUI windows and their NSViews are live, and this runs
            // on the UI thread. The typed sends match NSView.window and
            // NSWindow.addChildWindow:ordered: (NSWindowAbove = 1).
            unsafe {
                let get_window: unsafe extern "C" fn(*mut c_void, *const c_void) -> *mut c_void =
                    std::mem::transmute(objc_msgSend as *const c_void);
                let add_child: unsafe extern "C" fn(
                    *mut c_void,
                    *const c_void,
                    *mut c_void,
                    isize,
                ) = std::mem::transmute(objc_msgSend as *const c_void);
                let selector = sel_registerName(c"window".as_ptr());
                let child = get_window(child.ns_view.as_ptr(), selector);
                let parent = get_window(parent.ns_view.as_ptr(), selector);
                anyhow::ensure!(
                    !child.is_null() && !parent.is_null(),
                    "GPUI view has no native window"
                );
                add_child(
                    parent,
                    sel_registerName(c"addChildWindow:ordered:".as_ptr()),
                    child,
                    1,
                );
            }
            Ok(())
        }
        #[cfg(target_os = "windows")]
        (RawWindowHandle::Win32(child), RawWindowHandle::Win32(parent)) => {
            #[link(name = "user32")]
            unsafe extern "system" {
                fn SetWindowLongPtrW(window: isize, index: i32, value: isize) -> isize;
            }
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn SetLastError(error: u32);
                fn GetLastError() -> u32;
            }
            // SAFETY: These are live top-level HWNDs on their owning UI thread.
            // GWLP_HWNDPARENT sets the owner of a top-level window, preserving
            // independent input and keeping it above the owner without TOPMOST.
            unsafe {
                SetLastError(0);
                let previous = SetWindowLongPtrW(child.hwnd.get(), -8, parent.hwnd.get());
                let error = GetLastError();
                if previous == 0 && error != 0 {
                    return Err(std::io::Error::from_raw_os_error(error as i32).into());
                }
            }
            Ok(())
        }
        _ => bail!("unsupported native tool window handles"),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn attach(_: &Window, _: &Window) -> Result<()> {
    bail!("native tool window ownership is unavailable on this platform")
}
