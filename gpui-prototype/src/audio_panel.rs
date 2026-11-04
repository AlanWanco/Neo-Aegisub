use std::{cell::RefCell, rc::Rc, sync::Arc};

use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    Icon, StyledExt, Theme,
    button::{Button, ButtonVariants},
};
use gpui_kit::{
    Bounds, Context, Image, IntoElement, MouseButton, ObjectFit, Pixels, Styled, StyledImage,
    canvas, div, img, prelude::*, px, relative, rgb,
};
use serde::Deserialize;
use serde_json::json;

use crate::{Workspace, format_time};

#[derive(Clone, Debug, Deserialize)]
pub struct Syllable {
    pub text: String,
    pub start: i64,
    pub duration: i64,
    pub tag: String,
    pub characters: usize,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct AudioState {
    #[serde(default)]
    pub loaded: bool,
    #[serde(default)]
    pub epoch: u64,
    #[serde(default)]
    pub render_revision: u64,
    #[serde(default)]
    pub snap_distance: i64,
    #[serde(default)]
    pub drag_sensitivity: i64,
    #[serde(default)]
    pub duration: i64,
    #[serde(default)]
    pub decoded: i64,
    #[serde(default)]
    pub playing: bool,
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub start: i64,
    #[serde(default)]
    pub end: i64,
    #[serde(default)]
    pub rate: f64,
    #[serde(default)]
    pub volume: i64,
    #[serde(default)]
    pub autocommit: bool,
    #[serde(default)]
    pub autonext: bool,
    #[serde(default)]
    pub autoscroll: bool,
    #[serde(default)]
    pub snap: bool,
    #[serde(default)]
    pub spectrum: bool,
    #[serde(default)]
    pub medusa: bool,
    #[serde(default)]
    pub karaoke: bool,
    #[serde(default)]
    pub pending_splits: bool,
    #[serde(default)]
    pub syllables: Vec<Syllable>,
    #[serde(default)]
    pub markers: Vec<AudioMarker>,
}

/// One vertical line on the audio display. The host sends the pen each marker
/// provider uses, so seconds ticks (dotted, dim) stay distinguishable from
/// keyframes, the play cursor and inactive line boundaries.
#[derive(Clone, Debug, Deserialize)]
pub struct AudioMarker {
    pub time: i64,
    #[serde(default)]
    pub colour: String,
    #[serde(default = "default_marker_width")]
    pub width: i32,
    #[serde(default)]
    pub dotted: bool,
}

fn default_marker_width() -> i32 {
    2
}

impl AudioMarker {
    fn hsla(&self, fallback: gpui_kit::Hsla) -> gpui_kit::Hsla {
        let hex = self.colour.strip_prefix('#').unwrap_or(&self.colour);
        let digits = if hex.len() == 6 || hex.len() == 8 {
            u32::from_str_radix(hex, 16).ok()
        } else {
            None
        };
        match digits {
            Some(digits) if hex.len() == 6 => gpui_kit::rgba((digits << 8) | 0xff).into(),
            Some(digits) => gpui_kit::rgba(digits).into(),
            None => fallback,
        }
    }
}

#[derive(Deserialize)]
pub struct AudioImage {
    #[serde(flatten)]
    pub state: AudioState,
    pub image: Option<String>,
    #[serde(default)]
    pub view_start: f64,
}
/// A horizontal pan in progress. The ruler above the waveform and the
/// scrollbar below it move the same view window: `shaft` is zero for the
/// ruler, whose pixels map straight onto time, and is the thumb travel in
/// pixels for the scrollbar, whose pixels map onto the scrollable remainder.
#[derive(Clone, Copy)]
pub struct PanDrag {
    origin_x: f32,
    origin_start: i64,
    shaft: f32,
}

/// Identity of one cached waveform bitmap: every input the host's render
/// depends on, plus the slice of absolute time it covers. Slices sit on a fixed
/// time grid, so panning reuses them instead of invalidating the whole view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TileKey {
    pub epoch: u64,
    pub revision: u64,
    pub mode: &'static str,
    pub gain: i64,
    /// Length of the slice in milliseconds: the grid step.
    pub span: i64,
    /// Slice number, so `index * span` is where it starts.
    pub index: i64,
    /// Pixel height the host rendered the slice at. A bitmap taller or shorter
    /// than the track would be scaled by its own aspect ratio, so the track
    /// height is part of the identity instead of a fixed constant.
    pub height: i64,
}

impl TileKey {
    pub fn start(&self) -> i64 {
        self.index.saturating_mul(self.span)
    }
}

/// One decoded slice, placed where the host actually started it. The render
/// rounds its start to its own pixel grid, which is a fraction of a pixel at
/// view scale but would leave a seam between neighbours if ignored.
pub struct AudioTile {
    pub key: TileKey,
    pub image: Arc<Image>,
    pub start: i64,
}

/// Slices kept decoded at once. A tile is about a quarter of the viewport, so
/// this holds several screens of scrolling in both directions while staying in
/// the tens of megabytes of RGBA at worst.
pub const TILE_CACHE_LIMIT: usize = 48;
/// Concurrent render requests. Each costs a few milliseconds on the host, so
/// piling them up only delays the slice the user is actually looking at.
pub const TILE_INFLIGHT_LIMIT: usize = 3;

pub struct AudioPanel {
    pub mode: &'static str,
    pub view_start: i64,
    pub span: i64,
    pub gain: i64,
    /// Waveform bitmaps, each covering a fixed slice of absolute time. Panning
    /// reuses the ones already decoded instead of asking the host to redraw the
    /// whole view: one tile is a fraction of the viewport and a fraction of the
    /// cost, so a long recording scrolls without a full render per step.
    pub tiles: Vec<AudioTile>,
    /// Tiles whose render is already in flight, so a burst of mouse moves does
    /// not queue the same slice twice.
    pub tile_inflight: Vec<TileKey>,
    /// Bumped on every gesture step so only the last one schedules a render.
    pub render_generation: u64,
    pub bounds: Rc<RefCell<Bounds<Pixels>>>,
    pub gesture: Option<u64>,
    pub dragging: bool,
    pub pending_ms: Option<i64>,
    pub drag_loading: bool,
    pub finish_drag: bool,
    pub drag_button: Option<MouseButton>,
    pub drag_moved: bool,
    pub snap_inverted: bool,
    pub pan_drag: Option<PanDrag>,
}

impl AudioPanel {
    pub fn new() -> Self {
        Self {
            mode: "waveform",
            view_start: 0,
            span: 12000,
            gain: 800,
            tiles: Vec::new(),
            tile_inflight: Vec::new(),
            render_generation: 0,
            bounds: Rc::new(RefCell::new(Bounds::default())),
            gesture: None,
            dragging: false,
            pending_ms: None,
            drag_loading: false,
            finish_drag: false,
            drag_button: None,
            drag_moved: false,
            snap_inverted: false,
            pan_drag: None,
        }
    }
    /// Length of one slice: a quarter of the viewport, snapped to a 1-2-5 step
    /// so the grid stays put across small zoom changes instead of sliding with
    /// every wheel tick and throwing the cache away.
    pub fn tile_span(&self) -> i64 {
        let quarter = (self.span / 4).max(50);
        let mut scale = 1;
        while scale * 10 <= quarter {
            scale *= 10;
        }
        for step in [1, 2, 5] {
            if step * scale >= quarter {
                return step * scale;
            }
        }
        scale * 10
    }

    pub fn tile_key(&self, index: i64, epoch: u64, revision: u64, height: i64) -> TileKey {
        TileKey {
            epoch,
            revision,
            mode: self.mode,
            gain: self.gain,
            span: self.tile_span(),
            index,
            height,
        }
    }

    /// Slices the view shows, plus one either side so a small pan is already
    /// decoded by the time the gesture starts. Clamped to the media: a slice
    /// before zero or past the end would only render silence.
    pub fn wanted_tiles(
        &self,
        epoch: u64,
        revision: u64,
        duration: i64,
        height: i64,
    ) -> Vec<TileKey> {
        let span = self.tile_span();
        // The last slice starts before the final sample, not at the end.
        let media_last = (duration.max(1) - 1).div_euclid(span).max(0);
        let first = (self.view_start.div_euclid(span) - 1).max(0);
        let last = ((self.view_start + self.span).div_euclid(span) + 1)
            .min(media_last)
            .max(first);
        (first..=last)
            .map(|index| self.tile_key(index, epoch, revision, height))
            .collect()
    }

    pub fn cached_tile(&self, key: &TileKey) -> Option<&Arc<Image>> {
        self.tiles
            .iter()
            .find(|tile| tile.key == *key)
            .map(|tile| &tile.image)
    }

    /// Keeps a freshly rendered slice and, once the cache is over its limit,
    /// drops the least useful bitmaps first: stale generations, then the slices
    /// furthest from what is on screen.
    pub fn store_tile(
        &mut self,
        key: TileKey,
        start: i64,
        image: Arc<Image>,
        cx: &mut Context<Workspace>,
    ) {
        if let Some(slot) = self.tiles.iter_mut().find(|tile| tile.key == key) {
            slot.image = image;
            slot.start = start;
            return;
        }
        self.tiles.push(AudioTile { key, image, start });
        let center = self.view_start + self.span / 2;
        while self.tiles.len() > TILE_CACHE_LIMIT {
            let worst = self
                .tiles
                .iter()
                .enumerate()
                .max_by_key(|(_, tile)| {
                    let cached = &tile.key;
                    let stale = cached.epoch != key.epoch
                        || cached.revision != key.revision
                        || cached.mode != key.mode
                        || cached.gain != key.gain
                        || cached.span != key.span
                        || cached.height != key.height;
                    let distance = (tile.start + cached.span / 2 - center).abs();
                    (!stale, distance)
                })
                .map(|(index, _)| index);
            let Some(index) = worst else { break };
            let dropped = self.tiles.remove(index);
            dropped.image.remove_asset(cx);
        }
    }

    /// Drops every cached slice, for when the audio itself is replaced.
    pub fn clear_tiles(&mut self, cx: &mut Context<Workspace>) {
        for tile in self.tiles.drain(..) {
            tile.image.remove_asset(cx);
        }
        self.tile_inflight.clear();
    }

    pub fn time_at(&self, x: Pixels, state: &AudioState) -> i64 {
        let bounds = *self.bounds.borrow();
        let fraction =
            (f32::from(x - bounds.left()) / f32::from(bounds.size.width).max(1.)).clamp(0., 1.);
        (self.view_start + (self.span as f32 * fraction).round() as i64)
            .clamp(0, state.duration.max(0))
    }

    fn zoom_at(&mut self, span: i64, fraction: f64, duration: i64) {
        let span = span.clamp(250, 600000);
        if span == self.span {
            return;
        }
        let anchor = self.view_start as f64 + self.span as f64 * fraction;
        self.view_start =
            ((anchor - span as f64 * fraction).round() as i64).clamp(0, (duration - span).max(0));
        self.span = span;
    }

    /// Moves the view window from a cursor position while a pan is active.
    /// The ruler and the scrollbar differ only in how many milliseconds one
    /// pixel of travel is worth.
    fn pan_to(&mut self, x: f32, width: f32, duration: i64) {
        let Some(drag) = self.pan_drag else {
            return;
        };
        let delta = if drag.shaft > 1. {
            (x - drag.origin_x) / drag.shaft * (duration - self.span).max(0) as f32
        } else {
            -(x - drag.origin_x) / width.max(1.) * self.span as f32
        };
        self.view_start =
            (drag.origin_start + delta.round() as i64).clamp(0, (duration - self.span).max(0));
    }

    pub fn render_compact(
        &self,
        state: &AudioState,
        video: bool,
        appearance: &crate::Appearance,
        height: f32,
        focused: bool,
        line_text: &str,
        cx: &mut Context<Workspace>,
    ) -> impl IntoElement {
        let mut controls = div()
            .h_flex()
            .flex_wrap()
            .gap_2()
            .items_center()
            .flex_shrink_0();
        let mode_seg = crate::page::segmented(
            cx,
            [
                ("waveform", "波形", self.mode == "waveform", false),
                ("spectrum", "频谱", self.mode == "spectrum", false),
            ]
            .into_iter()
            .map(|(id, label, selected, disabled)| {
                (format!("audio-mode-{id}"), label, selected, disabled)
            })
            .collect(),
            |view, index, _, cx| {
                let mode = if index == 0 { "waveform" } else { "spectrum" };
                view.audio.mode = mode;
                if view.snapshot.audio.spectrum != (mode == "spectrum") {
                    view.invoke(json!({"op":"command", "name":"audio/opt/spectrum"}), cx);
                }
                view.refresh_audio(cx);
                cx.notify();
            },
        );
        for (id, label, command, active, tip) in [
            (
                "audio-auto-commit",
                "自动",
                "audio/opt/autocommit",
                state.autocommit,
                "自动提交：改动时间后立即写入字幕",
            ),
            (
                "audio-auto-next",
                "前进",
                "audio/opt/autonext",
                state.autonext,
                "提交后前进：提交一行后自动选中下一行",
            ),
            (
                "audio-auto-scroll",
                "跟随",
                "audio/opt/autoscroll",
                state.autoscroll,
                "跟随当前句：波形随播放自动滚动",
            ),
        ] {
            let chip =
                crate::page::toggle_chip(cx, id, label, active, false).on_click(cx.listener(
                    move |view, _, _, cx| view.invoke(json!({"op":"command", "name":command}), cx),
                ));
            controls = controls.child(crate::page::with_tooltip(chip, tip));
        }
        controls = controls.child(crate::page::with_tooltip(
            crate::page::toggle_chip(cx, "audio-snap", "吸附", state.snap, false).on_click(
                cx.listener(|view, _, _, cx| {
                    view.invoke(
                        json!({"op":"audio_action", "action":"snap", "enabled":!view.snapshot.audio.snap}),
                        cx,
                    )
                }),
            ),
            "吸附：拖动边界时贴到关键帧和音频峰值",
        ));
        // "OK" read as an acknowledgement button; it is the karaoke timing mode.
        controls = controls.child(crate::page::with_tooltip(
            crate::page::toggle_chip(
                cx,
                "audio-karaoke-mode",
                "卡拉OK",
                state.karaoke,
                !state.loaded,
            )
            .on_click(cx.listener(|view, _, _, cx| {
                view.invoke(
                    json!({"op":"audio_action", "action":"karaoke", "enabled":!view.snapshot.audio.karaoke}),
                    cx,
                )
            })),
            "卡拉 OK 模式：按音节切分时间",
        ));
        controls = controls.child(crate::page::with_tooltip(
            crate::page::toggle_chip(cx, "audio-medusa", "美杜莎", state.medusa, false).on_click(
                cx.listener(|view, _, _, cx| {
                    // Medusa mode swaps the global timing hotkeys for the
                    // audio-editing ones; the app command owns that option.
                    view.invoke(
                        json!({"op":"command", "name":"app/toggle/global_hotkeys"}),
                        cx,
                    )
                }),
            ),
            "美杜莎模式：把全局时间热键换成音频编辑热键",
        ));
        let bounds = self.bounds.clone();
        // The rendered waveform image is drawn for a dark display, so keep the
        // dark track once audio exists and follow the theme while it is empty.
        let graph_bg: gpui_kit::Hsla = if state.loaded {
            rgb(0x101519).into()
        } else {
            Theme::global(cx).muted
        };
        let mut graph = div()
            .id("audio-graph")
            .relative()
            .w_full()
            .h(px(height))
            .min_h(px(28.))
            .rounded(gpui_kit::px(crate::workspace_theme::RADIUS_MD))
            .bg(graph_bg)
            .overflow_hidden()
            .child(
                canvas(
                    move |region, _, _| {
                        *bounds.borrow_mut() = region;
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        let width = f32::from(self.bounds.borrow().size.width).max(1.);
        // The container's measured height is what the host was asked to render
        // for, so slices are matched against it rather than against the height
        // the layout requested: flex compression can make the two differ, and
        // using the requested one would leave the track permanently blank.
        let measured_height = f32::from(self.bounds.borrow().size.height);
        // Every cached slice is drawn over the time range it was rendered for,
        // so panning and zooming stay in step with the gesture instead of
        // freezing on the previous render until a new whole view lands. Slices
        // are independent, so a missing one costs a quarter of the view for a
        // few milliseconds rather than a blank waveform.
        let (epoch, revision) = (state.epoch, state.render_revision);
        let tile_span = self.tile_span();
        let track_height = track_pixels(if measured_height > 0.5 {
            measured_height
        } else {
            height
        });
        let container_height = if measured_height > 0.5 {
            measured_height
        } else {
            height
        };
        let (tiles, mode, gain, view_span) = (&self.tiles, self.mode, self.gain, self.span);
        let mut drawn = false;
        // Exact slices first; a slice rendered for another height is only a
        // stand-in while a resize settles, so it never hides a matching one.
        for exact in [true, false] {
            if drawn {
                break;
            }
            for tile in tiles {
                let key = tile.key;
                if key.epoch != epoch
                    || key.revision != revision
                    || key.mode != mode
                    || key.gain != gain
                    || key.span != tile_span
                    || (key.height == track_height) != exact
                {
                    continue;
                }
                if tile.start + key.span <= self.view_start
                    || tile.start >= self.view_start + view_span
                {
                    continue;
                }
                drawn = true;
                // The slice lands on the container's own rect: 1:1 when the host
                // could render at its height, and scaled when the track is
                // squeezed below the host's smallest renderable height or while a
                // resize has not landed yet, so the whole frequency range stays
                // visible instead of being clipped or blanked.
                let layer_height = if exact && key.height as f32 <= container_height {
                    key.height as f32
                } else {
                    container_height
                };
                graph = graph.child(audio_image_layer(
                    tile.image.clone(),
                    tile.start,
                    key.span,
                    self.view_start,
                    view_span,
                    width,
                    layer_height,
                ));
            }
        }
        if !drawn {
            graph = graph.child(
                div()
                    .p_4()
                    .text_color(Theme::global(cx).muted_foreground)
                    .child(if state.loaded {
                        "正在生成音频画面…"
                    } else {
                        "载入音频后显示波形；视频有音轨时可从视频载入"
                    }),
            );
        }
        // Markers first: the selection borders and grab handles have to stay on
        // top of them, otherwise the pale lines cross the red and blue handles
        // and nothing reads as draggable.
        let inactive_line = appearance.color(
            "Colour/Audio Display/Line Boundary Inactive Line",
            gpui_kit::rgba(0xe5c78bff).into(),
        );
        for marker in &state.markers {
            if marker.time < self.view_start || marker.time > self.view_start + self.span {
                continue;
            }
            let left = (marker.time - self.view_start) as f32 / self.span as f32 * width;
            let mut line = div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(left))
                .w(px(marker.width.max(1) as f32))
                .bg(marker.hsla(inactive_line));
            if marker.dotted {
                line = line.opacity(0.55);
            }
            graph = graph.child(line);
        }
        let left = ((state.start - self.view_start) as f32 / self.span as f32).clamp(0., 1.);
        let right = ((state.end - self.view_start) as f32 / self.span as f32).clamp(0., 1.);
        if state.loaded && right >= left {
            let start_color = appearance.color(
                "Colour/Audio Display/Line boundary Start",
                rgb(0x7ec2ff).into(),
            );
            let end_color = appearance.color(
                "Colour/Audio Display/Line boundary End",
                rgb(0x7ec2ff).into(),
            );
            // Only the active line's two boundaries can be dragged, so the grab
            // handles sit on them; every other marker stays a plain line.
            graph = graph.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(width * left))
                    .w(px(width * (right - left)))
                    .bg(gpui_kit::rgba(0x65b8ff26))
                    .border_l_2()
                    .border_color(start_color)
                    .child(
                        div()
                            .absolute()
                            .right_0()
                            .top_0()
                            .bottom_0()
                            .w(px(2.))
                            .bg(end_color),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(2.))
                            .left(px(-3.))
                            .w(px(9.))
                            .h(px(11.))
                            .rounded(px(2.))
                            .border_1()
                            .border_color(gpui_kit::rgba(0x101418cc))
                            .bg(start_color),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(2.))
                            .right(px(-3.))
                            .w(px(9.))
                            .h(px(11.))
                            .rounded(px(2.))
                            .border_1()
                            .border_color(gpui_kit::rgba(0x101418cc))
                            .bg(end_color),
                    ),
            );
        }
        if state.playing {
            let position =
                ((state.position - self.view_start) as f32 / self.span as f32).clamp(0., 1.);
            graph = graph.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(width * position))
                    .w(px(2.))
                    .bg(appearance.color("Colour/Audio Display/Play Cursor", rgb(0xffffff).into())),
            );
        }
        let graph = graph
            .on_scroll_wheel(
                cx.listener(|view, event: &gpui_kit::ScrollWheelEvent, _, cx| {
                    if !view.snapshot.audio.loaded || view.audio.dragging {
                        return;
                    }
                    let delta = match event.delta {
                        gpui_kit::ScrollDelta::Pixels(delta) => f32::from(delta.y) / 40.,
                        gpui_kit::ScrollDelta::Lines(delta) => delta.y,
                    };
                    if delta == 0. {
                        return;
                    }
                    let bounds = *view.audio.bounds.borrow();
                    let fraction = (f64::from(f32::from(event.position.x - bounds.left()))
                        / f64::from(f32::from(bounds.size.width).max(1.)))
                    .clamp(0., 1.);
                    let span =
                        (view.audio.span as f64 * 1.25_f64.powf(-f64::from(delta))).round() as i64;
                    view.audio
                        .zoom_at(span, fraction, view.snapshot.audio.duration);
                    // Wheel zoom fires many times per gesture; request one
                    // render once it settles instead of one per tick.
                    view.schedule_audio_render(cx);
                    cx.notify();
                }),
            )
            .on_any_mouse_down(
                cx.listener(|view, event: &gpui_kit::MouseDownEvent, window, cx| {
                    if !view.snapshot.audio.loaded
                        || view.audio.drag_loading
                        || !matches!(event.button, MouseButton::Left | MouseButton::Right)
                    {
                        return;
                    }
                    view.key_focus = crate::KeyFocus::Audio;
                    view.focus_handle.focus(window, cx);
                    let ms = view.audio.time_at(event.position.x, &view.snapshot.audio);
                    view.audio.snap_inverted = event.modifiers.shift;
                    view.audio.drag_button = Some(event.button);
                    view.begin_audio_drag(
                        ms,
                        event.button == MouseButton::Right,
                        event.modifiers.control || event.modifiers.platform,
                        event.modifiers.alt,
                        cx,
                    );
                }),
            )
            .on_mouse_move(
                cx.listener(|view, event: &gpui_kit::MouseMoveEvent, _, cx| {
                    if !view.audio.dragging || event.pressed_button != view.audio.drag_button {
                        return;
                    }
                    view.audio.drag_moved = true;
                    view.audio.snap_inverted = event.modifiers.shift;
                    view.audio.pending_ms =
                        Some(view.audio.time_at(event.position.x, &view.snapshot.audio));
                    view.flush_audio_drag(cx);
                }),
            )
            .capture_any_mouse_up(cx.listener(|view, event: &gpui_kit::MouseUpEvent, _, cx| {
                if !view.audio.dragging || Some(event.button) != view.audio.drag_button {
                    return;
                }
                view.audio.snap_inverted = event.modifiers.shift;
                view.audio.pending_ms =
                    Some(view.audio.time_at(event.position.x, &view.snapshot.audio));
                view.audio.finish_drag = true;
                view.flush_audio_drag(cx);
            }))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, event: &gpui_kit::MouseUpEvent, _, cx| {
                    if !view.audio.dragging || view.audio.drag_button != Some(MouseButton::Left) {
                        return;
                    }
                    view.audio.snap_inverted = event.modifiers.shift;
                    view.audio.pending_ms =
                        Some(view.audio.time_at(event.position.x, &view.snapshot.audio));
                    view.audio.finish_drag = true;
                    view.flush_audio_drag(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Right,
                cx.listener(|view, event: &gpui_kit::MouseUpEvent, _, cx| {
                    if !view.audio.dragging || view.audio.drag_button != Some(MouseButton::Right) {
                        return;
                    }
                    view.audio.snap_inverted = event.modifiers.shift;
                    view.audio.pending_ms =
                        Some(view.audio.time_at(event.position.x, &view.snapshot.audio));
                    view.audio.finish_drag = true;
                    view.flush_audio_drag(cx);
                }),
            );
        let p = Theme::global(cx);
        // The host protocol takes percent, but `state.rate` is a multiplier;
        // comparing 1.5 against 150 never matched, so the label stayed on 1×
        // and every click resent the same 1.5× value.
        let rate_steps = [("0.5×", 0.5), ("1×", 1.0), ("1.5×", 1.5), ("2×", 2.0)];
        let current = rate_steps
            .iter()
            .position(|(_, rate)| (state.rate - rate).abs() < 0.05)
            .unwrap_or(1);
        let next = rate_steps[(current + 1) % rate_steps.len()];
        let rates = div()
            .h_flex()
            .items_center()
            .p_0p5()
            .rounded_full()
            .bg(p.muted)
            .child(
                Button::new("audio-rate-cycle")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(9.))
                    .py(gpui_kit::px(3.))
                    .label(rate_steps[current].0)
                    .tooltip("点击切换播放倍速")
                    .ghost()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.invoke(
                            json!({"op":"audio_action", "action":"rate", "value":(next.1 * 100.).round() as i64}),
                            cx,
                        )
                    })),
            );
        let sound = div()
            .h_flex()
            .flex_wrap()
            .gap_1()
            .items_center()
            .child(rates)
            .child(div().w(px(1.)).h(px(18.)).mx_1().bg(p.border))
            .child(
                Button::new("audio-volume-down")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(9.))
                    .py(gpui_kit::px(3.))
                    .child(Icon::new(IconName::Minus).size(px(15.)))
                    .accessibility_label("音量 −")
                    .tooltip("音量 −10%")
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| {
                        let volume = (view.snapshot.audio.volume - 10).clamp(0, 200);
                        view.invoke(
                            json!({"op":"audio_action", "action":"volume", "value":volume}),
                            cx,
                        );
                    })),
            )
            .child(crate::page::with_tooltip(
                div()
                    .id("audio-volume-value")
                    .h_flex()
                    .items_center()
                    .gap_1()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(Icon::new(IconName::Volume2).size(px(15.)))
                    .child(format!("{}%", state.volume)),
                "播放音量",
            ))
            .child(
                Button::new("audio-volume-up")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(9.))
                    .py(gpui_kit::px(3.))
                    .child(Icon::new(IconName::Plus).size(px(15.)))
                    .accessibility_label("音量 +")
                    .tooltip("音量 +10%")
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| {
                        let volume = (view.snapshot.audio.volume + 10).clamp(0, 200);
                        view.invoke(
                            json!({"op":"audio_action", "action":"volume", "value":volume}),
                            cx,
                        );
                    })),
            );
        let mut karaoke = div().v_flex().gap_2().flex_shrink_0();
        if state.karaoke {
            let mut syllables = div().flex().flex_wrap().gap_2();
            for (index, syllable) in state.syllables.iter().enumerate() {
                let mut cell = div()
                    .v_flex()
                    .p_2()
                    .border_1()
                    .border_color(Theme::global(cx).border)
                    .gap_1()
                    .child(div().text_sm().child(syllable.text.clone()))
                    .child(div().text_xs().child(format!(
                        "{} · {} ms · {}",
                        format_time(syllable.start),
                        syllable.duration,
                        syllable.tag
                    )));
                let mut splits = div().h_flex().gap_1();
                for character in 1..syllable.characters {
                    splits = splits.child(Button::new(format!("split-{index}-{character}")).icon(IconName::Scissors).rounded(gpui_kit::px(999.)).small().px(gpui_kit::px(13.)).py(gpui_kit::px(3.)).label(format!("{character}｜"))
                        .on_click(cx.listener(move |view, _, _, cx| view.invoke(json!({"op":"audio_action", "action":"karaoke_split", "syllable":index, "character":character}), cx))));
                }
                if index > 0 {
                    splits = splits.child(Button::new(format!("merge-{index}")).icon(IconName::Plus).rounded(gpui_kit::px(999.)).small().px(gpui_kit::px(13.)).py(gpui_kit::px(3.)).label("合并前一节")
                        .on_click(cx.listener(move |view, _, _, cx| view.invoke(json!({"op":"audio_action", "action":"karaoke_merge", "syllable":index}), cx))));
                }
                cell = cell.child(splits);
                syllables = syllables.child(cell);
            }
            let mut actions = div()
                .h_flex()
                .gap_2()
                .child(
                    Button::new("accept-karaoke-splits")
                        .icon(IconName::Scissors)
                        .rounded(gpui_kit::px(999.))
                        .small()
                        .px(gpui_kit::px(13.))
                        .py(gpui_kit::px(3.))
                        .label("应用音节拆分")
                        .primary()
                        .disabled(!state.pending_splits)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.invoke(json!({"op":"audio_action", "action":"karaoke_accept"}), cx)
                        })),
                )
                .child(
                    Button::new("cancel-karaoke-splits")
                        .icon(IconName::Undo2)
                        .rounded(gpui_kit::px(999.))
                        .small()
                        .px(gpui_kit::px(13.))
                        .py(gpui_kit::px(3.))
                        .label("还原音节")
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.invoke(json!({"op":"audio_action", "action":"karaoke_cancel"}), cx)
                        })),
                );
            for (id, tag) in [
                ("karaoke-k", "\\k"),
                ("karaoke-kf", "\\kf"),
                ("karaoke-ko", "\\ko"),
            ] {
                actions = actions.child(
                    Button::new(id)
                        .icon(IconName::Asterisk)
                        .rounded(gpui_kit::px(999.))
                        .small()
                        .px(gpui_kit::px(13.))
                        .py(gpui_kit::px(3.))
                        .label(tag)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.invoke(
                                json!({"op":"audio_action", "action":"karaoke_tag", "tag":tag}),
                                cx,
                            )
                        })),
                );
            }
            karaoke = karaoke.child(actions).child(syllables);
        }
        let load_audio = if video {
            Button::new("audio-load-preview")
                .icon(IconName::Video)
                .rounded(gpui_kit::px(999.))
                .small()
                .px(gpui_kit::px(13.))
                .py(gpui_kit::px(3.))
                .label("从视频载入")
                .on_click(cx.listener(|view, _, _, cx| {
                    view.invoke(json!({"op":"command", "name":"audio/open/video"}), cx)
                }))
        } else {
            Button::new("open-audio")
                .icon(IconName::FolderOpen)
                .rounded(gpui_kit::px(999.))
                .small()
                .px(gpui_kit::px(13.))
                .py(gpui_kit::px(3.))
                .label("打开音频")
                .on_click(cx.listener(|view, _, _, cx| {
                    view.invoke(json!({"op":"command", "name":"audio/open"}), cx)
                }))
        };
        let hint = if state.loaded {
            format!(
                "{} → {} · {}",
                format_time(state.start),
                format_time(state.end),
                format_time(state.duration)
            )
        } else {
            // The strip is only 40px tall in the workspace; a permanent sentence
            // here costs a whole line of waveform.
            String::new()
        };
        let title_row = div()
            .h_flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .child(Icon::new(IconName::AudioLines).size(px(14.)))
            .child(
                div()
                    .flex_shrink_0()
                    .text_sm()
                    .font_semibold()
                    .child("音频波形"),
            )
            // The line being timed, in the same plain form as the editor's
            // "show original" line: raw text with its tags, one ellipsised
            // line, so it reads as the script rather than as a rendered cue.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(crate::workspace_theme::Palette::current(cx).muted)
                    .child(line_text.to_string()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_size(px(11.))
                    .text_color(Theme::global(cx).muted_foreground)
                    .child(hint),
            );
        // The wx audio display lets you drag its ruler to scroll sideways and
        // gives you a scrollbar whose thumb is the visible window. Both are
        // pure view panning here: they move `view_start` and never the cursor.
        let (ruler, scrollbar) = if state.loaded {
            let p = Theme::global(cx);
            let duration = state.duration.max(1);
            let ink = gpui_kit::rgba(0xc6d0dcff);
            let step = ruler_step(self.span, width);
            let mut ruler = div()
                .id("audio-ruler")
                .relative()
                .w_full()
                .h(px(20.))
                .flex_shrink_0()
                .overflow_hidden()
                .bg(graph_bg)
                .rounded_t(gpui_kit::px(crate::workspace_theme::RADIUS_MD))
                .cursor_pointer()
                .on_any_mouse_down(cx.listener(
                    |view, event: &gpui_kit::MouseDownEvent, window, cx| {
                        if !view.snapshot.audio.loaded || event.button != MouseButton::Left {
                            return;
                        }
                        view.claim_key_focus(crate::KeyFocus::Audio, window, cx);
                        view.audio.pan_drag = Some(PanDrag {
                            origin_x: f32::from(event.position.x),
                            origin_start: view.audio.view_start,
                            shaft: 0.,
                        });
                    },
                ))
                .on_mouse_move(
                    cx.listener(|view, event: &gpui_kit::MouseMoveEvent, _, cx| {
                        if event.pressed_button != Some(MouseButton::Left)
                            || view.audio.pan_drag.is_none()
                        {
                            return;
                        }
                        let width = f32::from(view.audio.bounds.borrow().size.width).max(1.);
                        view.audio.pan_to(
                            f32::from(event.position.x),
                            width,
                            view.snapshot.audio.duration,
                        );
                        // Slices are fetched while the drag runs: each one is
                        // cheap and lands in its own stretch of time, so the
                        // waveform fills in behind the pointer instead of
                        // waiting for the gesture to end.
                        view.refresh_audio(cx);
                        cx.notify();
                    }),
                )
                .capture_any_mouse_up(cx.listener(|view, _: &gpui_kit::MouseUpEvent, _, cx| {
                    if view.audio.pan_drag.take().is_some() {
                        view.schedule_audio_render(cx);
                    }
                }))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(1.))
                        .bg(p.border),
                );
            let mut mark = (self.view_start / step) * step;
            while mark <= self.view_start + self.span {
                if mark >= self.view_start {
                    let fraction = (mark - self.view_start) as f32 / self.span.max(1) as f32;
                    ruler = ruler
                        .child(
                            div()
                                .absolute()
                                .top(px(14.))
                                .left(relative(fraction))
                                .w(px(1.5))
                                .h(px(5.))
                                .bg(ink.opacity(0.55)),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(px(1.))
                                .left(relative((fraction + 3. / width).min(0.98)))
                                .whitespace_nowrap()
                                .text_size(px(9.5))
                                .text_color(ink.opacity(0.85))
                                .child(format_time(mark)),
                        );
                }
                mark += step;
            }
            let (thumb_left, thumb_width) =
                thumb_fractions(width, self.span, duration, self.view_start);
            let mut scrollbar = div()
                .id("audio-scrollbar")
                .relative()
                .w_full()
                .h(px(16.))
                .flex_shrink_0()
                .cursor_pointer()
                .on_any_mouse_down(
                    cx.listener(|view, event: &gpui_kit::MouseDownEvent, _, cx| {
                        if !view.snapshot.audio.loaded || event.button != MouseButton::Left {
                            return;
                        }
                        let duration = view.snapshot.audio.duration.max(1);
                        let span = view.audio.span;
                        if duration <= span {
                            return;
                        }
                        let bounds = *view.audio.bounds.borrow();
                        let width = f32::from(bounds.size.width).max(1.);
                        let thumb = (width * span as f32 / duration as f32).max(28.).min(width);
                        let shaft = (width - thumb).max(1.);
                        let target = (f32::from(event.position.x - bounds.left()) - thumb / 2.)
                            .clamp(0., shaft);
                        let start = (target / shaft * (duration - span) as f32).round() as i64;
                        view.audio.view_start = start;
                        view.audio.pan_drag = Some(PanDrag {
                            origin_x: f32::from(event.position.x),
                            origin_start: start,
                            shaft,
                        });
                        view.schedule_audio_render(cx);
                        cx.notify();
                    }),
                )
                .on_mouse_move(
                    cx.listener(|view, event: &gpui_kit::MouseMoveEvent, _, cx| {
                        if event.pressed_button != Some(MouseButton::Left)
                            || view.audio.pan_drag.is_none()
                        {
                            return;
                        }
                        let width = f32::from(view.audio.bounds.borrow().size.width).max(1.);
                        view.audio.pan_to(
                            f32::from(event.position.x),
                            width,
                            view.snapshot.audio.duration,
                        );
                        // Slices are fetched while the drag runs: each one is
                        // cheap and lands in its own stretch of time, so the
                        // waveform fills in behind the thumb instead of waiting
                        // for the gesture to end.
                        view.refresh_audio(cx);
                        cx.notify();
                    }),
                )
                .capture_any_mouse_up(cx.listener(|view, _: &gpui_kit::MouseUpEvent, _, cx| {
                    if view.audio.pan_drag.take().is_some() {
                        view.schedule_audio_render(cx);
                    }
                }))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px(5.))
                        .h(px(6.))
                        .rounded(px(3.))
                        .bg(ink.opacity(0.16)),
                );
            if state.end > state.start {
                let from = state.start.max(0) as f32 / duration as f32;
                let to = state.end.max(0) as f32 / duration as f32;
                scrollbar = scrollbar.child(
                    div()
                        .absolute()
                        .top(px(5.))
                        .h(px(6.))
                        .left(relative(from.clamp(0., 1.)))
                        .w(relative((to - from).clamp(0., 1.).max(0.002)))
                        .rounded(px(3.))
                        .bg(appearance
                            .color(
                                "Colour/Audio Display/Line boundary Start",
                                rgb(0x7ec2ff).into(),
                            )
                            .opacity(0.45)),
                );
            }
            scrollbar = scrollbar.child(
                div()
                    .absolute()
                    .top(px(4.))
                    .h(px(8.))
                    .left(relative(thumb_left))
                    .w(relative(thumb_width))
                    .rounded(px(4.))
                    .bg(p.muted_foreground),
            );
            (ruler.into_any_element(), scrollbar.into_any_element())
        } else {
            (div().into_any_element(), div().into_any_element())
        };
        let palette = crate::workspace_theme::Palette::new(Theme::global(cx).is_dark());
        let mut panel = div()
            .v_flex()
            .w_full()
            .min_w_0()
            .min_h_0()
            .gap_3()
            // The waveform strip is a card like every other workspace pane, so the
            // workspace reads as one set of surfaces instead of a bare canvas.
            .rounded(gpui_kit::px(crate::workspace_theme::RADIUS_LG))
            .border_1()
            .border_color(if focused {
                palette.accent
            } else {
                palette.border
            })
            .bg(palette.surface)
            .shadow(crate::page::card_shadow(Theme::global(cx).is_dark()))
            // Clicking anywhere in the strip claims the audio shortcut context.
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    view.claim_key_focus(crate::KeyFocus::Audio, window, cx)
                }),
            )
            .p_3()
            .child(title_row.when(!state.loaded, |row| row.child(load_audio)))
            .child(ruler)
            // The scrollbar hugs the waveform the way the wx timeline does, so the
            // pair reads as one control instead of two separate rows.
            .child(
                div()
                    .v_flex()
                    .w_full()
                    .min_w_0()
                    .min_h_0()
                    .child(graph)
                    .child(scrollbar),
            );
        let compact_viewport = div()
            .h_flex()
            .flex_wrap()
            .gap_1()
            .items_center()
            .child(
                Button::new("audio-gain-down")
                    .child(Icon::new(IconName::Minus).size(px(15.)))
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(9.))
                    .py(gpui_kit::px(3.))
                    .tooltip("降低波形增益（÷2）")
                    .disabled(!state.loaded)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.audio.gain = (view.audio.gain / 2).max(10);
                        view.refresh_audio(cx);
                        cx.notify();
                    })),
            )
            .child(crate::page::with_tooltip(
                div()
                    .id("audio-gain-value")
                    .h_flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(Theme::global(cx).muted_foreground)
                    .child(Icon::new(IconName::AudioLines).size(px(15.)))
                    .child(format!("{}%", self.gain)),
                "波形显示增益：只改变画面幅度，不改变播放音量",
            ))
            .child(
                Button::new("audio-gain-up")
                    .child(Icon::new(IconName::Plus).size(px(15.)))
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(9.))
                    .py(gpui_kit::px(3.))
                    .tooltip("提高波形增益（×2）")
                    .disabled(!state.loaded)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.audio.gain = (view.audio.gain * 2).min(1000);
                        view.refresh_audio(cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("audio-fit-line")
                    .child(Icon::new(IconName::Locate).size(px(15.)))
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(9.))
                    .py(gpui_kit::px(3.))
                    .accessibility_label("定位当前句")
                    .tooltip("缩放到当前句的时间范围；在波形上滚轮缩放，以鼠标位置为中心")
                    .disabled(!state.loaded)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.audio.view_start = (view.snapshot.audio.start - 1000).max(0);
                        view.audio.span = (view.snapshot.audio.end - view.snapshot.audio.start
                            + 2000)
                            .clamp(1000, 600000);
                        view.refresh_audio(cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("audio-zoom-out")
                    .child(Icon::new(IconName::ZoomOut).size(px(15.)))
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(9.))
                    .py(gpui_kit::px(3.))
                    .accessibility_label("缩小波形")
                    .tooltip("缩小波形时间轴")
                    .disabled(!state.loaded)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.audio
                            .zoom_at(view.audio.span * 2, 0.5, view.snapshot.audio.duration);
                        view.refresh_audio(cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("audio-zoom-in")
                    .child(Icon::new(IconName::ZoomIn).size(px(15.)))
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(9.))
                    .py(gpui_kit::px(3.))
                    .accessibility_label("放大波形")
                    .tooltip("放大波形时间轴")
                    .disabled(!state.loaded)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.audio
                            .zoom_at(view.audio.span / 2, 0.5, view.snapshot.audio.duration);
                        view.refresh_audio(cx);
                        cx.notify();
                    })),
            );
        // View/playback above, behaviour and modes below.
        panel = panel
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .flex_shrink_0()
                    .child(mode_seg)
                    .child(compact_viewport)
                    .child(sound),
            )
            .child(controls)
            .child(karaoke.when(state.karaoke, |row| row));
        panel.into_any_element()
    }
}

/// Pixel height the host renders a slice at: the track's own height, inside the
/// width the render request accepts. A bitmap rendered at a different height
/// than the track is scaled by its aspect ratio, which made the spectrum look
/// taller as the track got wider.
pub fn track_pixels(height: f32) -> i64 {
    (height.round() as i64).clamp(64, 512)
}

/// Places one waveform bitmap over the time range it was rendered for.
fn audio_image_layer(
    image: Arc<Image>,
    start: i64,
    span: i64,
    view_start: i64,
    view_span: i64,
    width: f32,
    height: f32,
) -> impl IntoElement {
    let left = (start - view_start) as f32 / view_span.max(1) as f32;
    let image_width = span.max(1) as f32 / view_span.max(1) as f32;
    // Both axes are pinned to the track instead of leaving either one to the
    // bitmap's intrinsic size: an auto height follows the rendered width, so
    // the spectrum grew taller whenever the track got wider.
    img(image)
        .absolute()
        .top_0()
        .h(px(height))
        .left(px(width * left))
        .w(px(width * image_width))
        .object_fit(ObjectFit::Fill)
}

/// Spacing between ruler labels: the smallest step that still leaves room for
/// a `0:00:01.00` label at the current zoom.
fn ruler_step(span: i64, width: f32) -> i64 {
    const STEPS: [i64; 14] = [
        50, 100, 200, 500, 1_000, 2_000, 5_000, 10_000, 15_000, 30_000, 60_000, 120_000, 300_000,
        600_000,
    ];
    let span = span.max(1);
    STEPS
        .iter()
        .copied()
        .find(|step| *step as f32 / span as f32 * width.max(1.) >= 76.)
        .unwrap_or_else(|| *STEPS.last().unwrap())
}

/// Scrollbar thumb as `(left, width)` fractions of the track for a view window
/// of `span` inside `duration`. The thumb keeps a minimum grab width, so the
/// shaft it can travel along is what maps onto the unscrolled remainder.
fn thumb_fractions(width: f32, span: i64, duration: i64, view_start: i64) -> (f32, f32) {
    let track = width.max(1.);
    let span = span.max(1);
    let duration = duration.max(1);
    if duration <= span {
        return (0., 1.);
    }
    let thumb = (track * span as f32 / duration as f32).max(28.).min(track);
    let shaft = (track - thumb).max(0.);
    let left = view_start.clamp(0, duration - span) as f32 / (duration - span) as f32 * shaft;
    (left / track, thumb / track)
}

#[cfg(test)]
mod tests {
    use super::{AudioPanel, PanDrag, thumb_fractions};

    #[test]
    fn zoom_preserves_time_under_pointer() {
        let mut panel = AudioPanel::new();
        panel.view_start = 10000;
        panel.zoom_at(6000, 0.75, 60000);
        assert_eq!((panel.view_start, panel.span), (14500, 6000));
        panel.zoom_at(12000, 0.75, 60000);
        assert_eq!((panel.view_start, panel.span), (10000, 12000));
    }

    #[test]
    fn zoom_stays_within_media_bounds() {
        let mut panel = AudioPanel::new();
        panel.zoom_at(24000, 0.75, 60000);
        assert_eq!(panel.view_start, 0);
        panel.view_start = 48000;
        panel.span = 12000;
        panel.zoom_at(24000, 0.25, 60000);
        assert_eq!(panel.view_start, 36000);
        panel.zoom_at(60000, 0.5, 18000);
        assert_eq!(panel.view_start, 0);
    }

    #[test]
    fn zoom_limits_do_not_shift_time_at_same_scale() {
        let mut panel = AudioPanel::new();
        panel.view_start = 10000;
        panel.zoom_at(1, 0.75, 900000);
        assert_eq!(panel.span, 250);
        let start = panel.view_start;
        panel.zoom_at(1, 0.25, 900000);
        assert_eq!(panel.view_start, start);
        panel.zoom_at(i64::MAX, 0.5, 900000);
        assert_eq!(panel.span, 600000);
    }

    #[test]
    fn ruler_drag_scrolls_opposite_the_pointer_and_clamps() {
        let mut panel = AudioPanel::new();
        panel.view_start = 5000;
        panel.span = 10_000;
        panel.pan_drag = Some(PanDrag {
            origin_x: 100.,
            origin_start: 5000,
            shaft: 0.,
        });
        // Dragging 200 px left over a 1000 px view moves the window 2 s later.
        panel.pan_to(-100., 1000., 60_000);
        assert_eq!(panel.view_start, 7000);
        panel.pan_to(-5000., 1000., 60_000);
        assert_eq!(
            panel.view_start, 50_000,
            "the window stops at duration - span"
        );
        panel.pan_to(9000., 1000., 60_000);
        assert_eq!(panel.view_start, 0, "it never scrolls before zero");
    }

    #[test]
    fn thumb_wraps_the_visible_window_and_keeps_a_grab_width() {
        // 12 s of 120 s in a 1200 px track: a 120 px thumb on a 1080 px shaft.
        assert_eq!(thumb_fractions(1200., 12_000, 120_000, 0), (0., 0.1));
        let (left, width) = thumb_fractions(1200., 12_000, 120_000, 54_000);
        assert!((left - 0.45).abs() < 1e-4, "{left}");
        assert!((width - 0.1).abs() < 1e-4, "{width}");
        // Fully zoomed out: the thumb covers the whole track.
        assert_eq!(thumb_fractions(1200., 120_000, 120_000, 0), (0., 1.));
        // A very wide view keeps the 28 px minimum grab width.
        let (_, width) = thumb_fractions(800., 6_000, 600_000, 0);
        assert!((width - 28. / 800.).abs() < 1e-5, "{width}");
    }

    #[test]
    fn tile_span_snaps_to_a_stable_grid() {
        // A quarter of the viewport, rounded up to a 1-2-5 step, so a few zoom
        // ticks in a row keep the same grid instead of invalidating the cache.
        let mut panel = AudioPanel::new();
        panel.span = 12_000;
        assert_eq!(panel.tile_span(), 5_000);
        panel.span = 10_000;
        assert_eq!(panel.tile_span(), 5_000, "same grid one zoom step away");
        panel.span = 600_000;
        assert_eq!(panel.tile_span(), 200_000);
        panel.span = 250;
        assert_eq!(panel.tile_span(), 100);
    }

    #[test]
    fn wanted_tiles_cover_the_view_with_one_spare_each_side() {
        let mut panel = AudioPanel::new();
        panel.span = 12_000;
        let keys = panel.wanted_tiles(3, 7, 60_000, 200);
        let indices: Vec<i64> = keys.iter().map(|key| key.index).collect();
        assert_eq!(indices, vec![0, 1, 2, 3], "0..10 s, 10..15 s, 15..20 s");
        assert!(keys.iter().all(|key| key.span == 5_000 && key.epoch == 3));

        // In the middle of a recording the grid stays absolute, so the slices
        // already decoded are the ones next to the view, not a new grid.
        panel.view_start = 31_000;
        let indices: Vec<i64> = panel
            .wanted_tiles(3, 7, 600_000, 200)
            .iter()
            .map(|key| key.index)
            .collect();
        assert_eq!(indices, vec![5, 6, 7, 8, 9]);

        // At the end of the media the request stops at the last slice that
        // holds samples instead of rendering silence past it: a 2 s view is a
        // 500 ms grid, so 18..20 s is slices 36..39 plus one either side.
        panel.view_start = 18_000;
        panel.span = 2_000;
        let indices: Vec<i64> = panel
            .wanted_tiles(3, 7, 20_000, 200)
            .iter()
            .map(|key| key.index)
            .collect();
        assert_eq!(indices, vec![35, 36, 37, 38, 39]);
    }

    #[test]
    fn a_taller_track_is_a_different_slice() {
        // The bitmap is rendered at the track's height. Reusing a slice from a
        // shorter track would scale it by its own aspect ratio, which is how
        // the spectrum came to look taller as the track got wider.
        let panel = AudioPanel::new();
        let short = panel.wanted_tiles(3, 7, 60_000, 64);
        let tall = panel.wanted_tiles(3, 7, 60_000, 240);
        assert_eq!(short.len(), tall.len());
        assert!(short.iter().zip(&tall).all(|(a, b)| a != b));
        assert!(tall.iter().all(|key| key.height == 240));
    }
}
