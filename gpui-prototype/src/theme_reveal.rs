use gpui_kit::{
    AnyElement, App, Bounds, IntoElement, ParentElement, Pixels, Point, RenderImage, Styled,
    Window, canvas, div, px,
};
use image::RgbaImage;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

const EXPAND: Duration = Duration::from_millis(420);
const FEATHER: f32 = 26.;

#[derive(Default)]
pub struct ThemeReveal {
    target: Option<bool>,
    transition: Option<Circle>,
}

struct Circle {
    origin: Point<Pixels>,
    started: Instant,
    snapshot: Rc<Snapshot>,
}

struct Snapshot {
    pixels: RgbaImage,
    logical_size: gpui_kit::Size<Pixels>,
    texture: RefCell<Option<Arc<RenderImage>>>,
}

impl ThemeReveal {
    pub fn is_active(&self) -> bool {
        self.transition.is_some()
    }

    fn clear(&mut self, window: &mut Window) {
        if let Some(circle) = self.transition.take() {
            if let Some(texture) = circle.snapshot.texture.borrow_mut().take() {
                let _ = window.drop_image(texture);
            }
        }
    }

    /// Keep the previous visible frame above the new theme, opening a feathered
    /// hole in it. Reversing captures the current composite, preserving continuity.
    pub fn advance(&mut self, dark: bool, window: &mut Window, cx: &App) -> Option<bool> {
        let previous = self.target.replace(dark);
        if previous.is_none() || cx.reduce_motion() {
            let active = self.is_active();
            self.clear(window);
            return (previous != Some(dark) || active).then_some(dark);
        }
        if previous != Some(dark) {
            // Capture before changing the theme or releasing the previous texture.
            let snapshot = capture(window);
            self.clear(window);
            match snapshot {
                Ok(snapshot) => {
                    self.transition = Some(Circle {
                        origin: window.mouse_position(),
                        started: Instant::now(),
                        snapshot: Rc::new(snapshot),
                    })
                }
                Err(error) => eprintln!("Could not capture theme transition: {error:#}"),
            }
            window.request_animation_frame();
            return Some(dark);
        }
        if let Some(circle) = &self.transition {
            if circle.started.elapsed() >= EXPAND {
                self.clear(window);
            } else {
                window.request_animation_frame();
            }
        }
        None
    }

    pub fn overlay(&self) -> Option<AnyElement> {
        let circle = self.transition.as_ref()?;
        let snapshot = circle.snapshot.clone();
        let scale = snapshot.pixels.width() as f32 / f32::from(snapshot.logical_size.width);
        let origin = (
            f32::from(circle.origin.x).clamp(0., f32::from(snapshot.logical_size.width)) * scale,
            f32::from(circle.origin.y).clamp(0., f32::from(snapshot.logical_size.height)) * scale,
        );
        let far_x = origin.0.max(snapshot.pixels.width() as f32 - origin.0);
        let far_y = origin.1.max(snapshot.pixels.height() as f32 - origin.1);
        let progress = (circle.started.elapsed().as_secs_f32() / EXPAND.as_secs_f32()).min(1.);
        let feather = FEATHER * scale;
        let radius = (far_x.hypot(far_y) + feather) * (1. - (1. - progress).powi(3));
        Some(
            div()
                .absolute()
                .inset_0()
                .overflow_hidden()
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds: Bounds<Pixels>, _, window, _| {
                            if let Some(texture) = snapshot.texture.borrow_mut().take() {
                                let _ = window.drop_image(texture);
                            }
                            // RenderImage frames are immutable. Only the feather needs distance
                            // calculations; the transparent interior is cleared a row at a time.
                            let frame = image::Frame::new(reveal_mask(
                                &snapshot.pixels,
                                origin,
                                radius,
                                feather,
                            ));
                            let texture = Arc::new(RenderImage::new(vec![frame]));
                            if let Err(error) = window.paint_image(
                                bounds,
                                bounds,
                                px(0.).into(),
                                texture.clone(),
                                0,
                                false,
                            ) {
                                eprintln!("Could not paint theme transition: {error:#}");
                            }
                            *snapshot.texture.borrow_mut() = Some(texture);
                        },
                    )
                    .size_full(),
                )
                .into_any_element(),
        )
    }
}

fn reveal_mask(source: &RgbaImage, origin: (f32, f32), radius: f32, feather: f32) -> RgbaImage {
    let mut image = source.clone();
    let width = source.width() as usize;
    let outer_squared = radius * radius;
    let inner = (radius - feather).max(0.);
    let inner_squared = inner * inner;
    let bound = |x: f32| x.ceil().clamp(0., width as f32) as usize;
    for (y, row) in image.chunks_exact_mut(width * 4).enumerate() {
        let dy = y as f32 + 0.5 - origin.1;
        let dy_squared = dy * dy;
        if dy_squared >= outer_squared {
            continue;
        }
        let outer_x = (outer_squared - dy_squared).sqrt();
        let start = bound(origin.0 - outer_x - 0.5);
        let end = bound(origin.0 + outer_x - 0.5);
        let (left, right) = if radius >= feather && dy_squared < inner_squared {
            let inner_x = (inner_squared - dy_squared).sqrt();
            let left = bound(origin.0 - inner_x - 0.5);
            let right = bound(origin.0 + inner_x - 0.5);
            row[left * 4..right * 4].fill(0);
            (left, right)
        } else {
            (end, end)
        };
        for x in (start..left).chain(right..end) {
            let dx = x as f32 + 0.5 - origin.0;
            let t = (((dx * dx + dy_squared).sqrt() - (radius - feather)) / feather).clamp(0., 1.);
            row[x * 4 + 3] = (row[x * 4 + 3] as f32 * t * t * (3. - 2. * t)).round() as u8;
        }
    }
    image
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn capture(window: &Window) -> anyhow::Result<Snapshot> {
    use anyhow::{Context, ensure};
    let id = native_window_id(window)?;
    let native = xcap::Window::all()?
        .into_iter()
        .find(|candidate| candidate.id().ok() == Some(id))
        .context("current window is not available for capture")?;
    let mut image = native.capture_image()?;
    let logical_size = window.viewport_size();
    let scale = window.scale_factor();
    let width = (f32::from(logical_size.width) * scale).round() as u32;
    let height = (f32::from(logical_size.height) * scale).round() as u32;
    ensure!(
        width > 0 && height > 0 && image.width() >= width && image.height() >= height,
        "captured window does not contain its client area"
    );
    let x = (image.width() - width) / 2;
    #[cfg(target_os = "macos")]
    let bottom = 0;
    #[cfg(target_os = "windows")]
    let bottom = x.min(image.height() - height);
    let y = image.height() - height - bottom;
    if image.dimensions() != (width, height) {
        image = image::imageops::crop_imm(&image, x, y, width, height).to_image();
    }
    // GPUI stores RenderImage frames in BGRA order.
    for pixel in image.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok(Snapshot {
        pixels: image,
        logical_size,
        texture: RefCell::new(None),
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn capture(_: &Window) -> anyhow::Result<Snapshot> {
    anyhow::bail!("window capture is unavailable on this platform")
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn native_window_id(window: &Window) -> anyhow::Result<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("no native window handle: {error}"))?
        .as_raw()
    {
        #[cfg(target_os = "windows")]
        RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get() as u32),
        #[cfg(target_os = "macos")]
        RawWindowHandle::AppKit(handle) => {
            use std::ffi::c_void;
            #[link(name = "objc")]
            unsafe extern "C" {
                fn sel_registerName(name: *const std::ffi::c_char) -> *const c_void;
                fn objc_msgSend();
            }
            // SAFETY: GPUI's handle is a live NSView on the UI thread. These
            // selectors return its NSWindow and that window's NSInteger number.
            unsafe {
                let get_window: unsafe extern "C" fn(*mut c_void, *const c_void) -> *mut c_void =
                    std::mem::transmute(objc_msgSend as *const c_void);
                let number: unsafe extern "C" fn(*mut c_void, *const c_void) -> isize =
                    std::mem::transmute(objc_msgSend as *const c_void);
                let parent = get_window(
                    handle.ns_view.as_ptr(),
                    sel_registerName(c"window".as_ptr()),
                );
                anyhow::ensure!(!parent.is_null(), "GPUI view has no native window");
                Ok(number(parent, sel_registerName(c"windowNumber".as_ptr())) as u32)
            }
        }
        _ => anyhow::bail!("unsupported native window handle"),
    }
}

#[cfg(test)]
mod tests {
    use super::reveal_mask;
    use image::{Rgba, RgbaImage};

    #[test]
    fn feather_reveals_center_preserves_outside_and_blends_edge() {
        let source = RgbaImage::from_pixel(81, 81, Rgba([30, 60, 90, 255]));
        let masked = reveal_mask(&source, (40.5, 40.5), 30., 10.);
        assert_eq!(masked.get_pixel(40, 40).0, [0, 0, 0, 0]);
        assert_eq!(masked.get_pixel(80, 80).0, [30, 60, 90, 255]);
        assert_eq!(masked.get_pixel(65, 40).0, [30, 60, 90, 128]);
        assert_eq!(masked.get_pixel(15, 40), masked.get_pixel(65, 40));
    }

    #[test]
    fn initial_and_complete_masks_cover_opposite_boundaries() {
        let source = RgbaImage::from_pixel(31, 19, Rgba([30, 60, 90, 255]));
        assert_eq!(reveal_mask(&source, (0., 0.), 0., 10.), source);
        assert!(
            reveal_mask(&source, (0., 0.), 50., 10.)
                .pixels()
                .all(|pixel| pixel[3] == 0)
        );
    }
}
