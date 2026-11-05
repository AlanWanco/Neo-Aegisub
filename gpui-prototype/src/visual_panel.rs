use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::{
    Bounds, ContentMask, Context, FillOptions, FillRule, IntoElement, Path, PathBuilder, PathStyle,
    Pixels, Point, TextRun, Window, canvas, div, point, prelude::*, px, size,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Workspace;

#[derive(Clone, Debug, Deserialize)]
pub struct DrawCommand {
    pub kind: String,
    #[serde(default)]
    pub primitive: u32,
    #[serde(default)]
    pub fill: bool,
    #[serde(default)]
    pub width: f32,
    #[serde(default)]
    pub points: Vec<[f32; 2]>,
    #[serde(default)]
    pub polygons: Vec<Vec<[f32; 2]>>,
    #[serde(default)]
    pub bounds: Vec<[f32; 2]>,
    #[serde(default)]
    pub outside: bool,
    #[serde(default)]
    pub color: [f32; 4],
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub size: f32,
}

#[derive(Deserialize)]
pub struct VisualReply {
    pub commands: Vec<DrawCommand>,
    pub tool: String,
    pub subtool: usize,
    pub width: i64,
    pub height: i64,
}

const VISUAL_TOOLS: [(&str, &str); 7] = [
    ("cross", "十字"),
    ("drag", "拖动"),
    ("rotate/z", "Z 旋转"),
    ("rotate/xy", "XY 旋转"),
    ("scale", "缩放"),
    ("clip", "矩形裁剪"),
    ("vector_clip", "矢量裁剪"),
];

/// Overlay for the visual typesetting tools. The native tools draw through
/// `src/visual_render.cpp`, which records primitives instead of rasterising
/// them, so the frontend paints the recorded triangles and paths itself.
/// The host tool coordinates are the canvas the picture occupies at zoom 1;
/// the workspace owns zoom and pan, and this panel maps pointer positions
/// back through them.
pub struct VisualPanel {
    pub tool: &'static str,
    pub subtool: usize,
    /// The picture rectangle on screen, zoom and pan included.
    pub bounds: Rc<RefCell<Bounds<Pixels>>>,
    /// The picture fitted to the viewport before zoom and pan. The host sizes
    /// its canvas to these pixels, so tool coordinates are relative to it.
    pub canvas_bounds: Rc<RefCell<Bounds<Pixels>>>,
    pub commands: Vec<DrawCommand>,
    pub queue: VecDeque<Value>,
    pub loading: bool,
    pub dragging: bool,
    pub drawn: Option<(u64, i64, i64, i64)>,
}

impl VisualPanel {
    pub fn new() -> Self {
        Self {
            tool: "cross",
            subtool: 1,
            bounds: Rc::new(RefCell::new(Bounds::default())),
            canvas_bounds: Rc::new(RefCell::new(Bounds::default())),
            commands: vec![],
            queue: VecDeque::new(),
            loading: false,
            dragging: false,
            drawn: None,
        }
    }

    /// Pointer position in the host canvas' own pixels.
    pub fn local(&self, position: Point<Pixels>) -> (i64, i64) {
        let bounds = *self.bounds.borrow();
        let zoom = picture_scale(bounds, *self.canvas_bounds.borrow());
        (
            (f32::from(position.x - bounds.left()) / zoom).round() as i64,
            (f32::from(position.y - bounds.top()) / zoom).round() as i64,
        )
    }

    pub fn toolbar(&self, cx: &mut Context<Workspace>) -> impl IntoElement {
        let active = VISUAL_TOOLS
            .iter()
            .position(|(mode, _)| *mode == self.tool)
            .unwrap_or(0);
        let mut bar = crate::page::toolbar([crate::page::segmented(
            cx,
            VISUAL_TOOLS
                .iter()
                .enumerate()
                .map(|(index, (_, label))| {
                    (
                        format!("visual-mode-{index}"),
                        *label,
                        index == active,
                        false,
                    )
                })
                .collect(),
            |view, index, _, cx| {
                view.select_visual_tool(VISUAL_TOOLS[index].0, None, cx);
            },
        )
        .into_any_element()]);
        if self.tool == "vector_clip" {
            for (mode, label) in [
                (0, "控制点"),
                (1, "直线"),
                (2, "贝塞尔"),
                (3, "转换"),
                (4, "插点"),
                (5, "删点"),
                (6, "手绘"),
                (7, "平滑手绘"),
            ] {
                bar = bar.child(
                    Button::new(format!("visual-vector-{mode}"))
                        .icon(IconName::Asterisk)
                        .rounded(gpui_kit::px(999.))
                        .small()
                        .px(gpui_kit::px(13.))
                        .py(gpui_kit::px(3.))
                        .label(label)
                        .ghost()
                        .when(mode == self.subtool, |button| button.primary())
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.select_visual_tool("vector_clip", Some(mode), cx);
                        })),
                );
            }
        }
        if self.tool == "drag" {
            bar = bar.child(
                Button::new("visual-toggle-move")
                    .icon(IconName::Move)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .label("pos ↔ move")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.queue_visual(json!({"action":"toggle_move"}), cx)
                    })),
            );
        }
        bar
    }

    /// Draws the recorded commands over the picture and forwards pointer
    /// events to the tool. `zoom` and `pan` are the workspace video view's
    /// own transform, so handles stay on the pixels they were drawn over.
    pub fn overlay(
        &self,
        aspect: f32,
        zoom: f32,
        pan: (f32, f32),
        cx: &mut Context<Workspace>,
    ) -> impl IntoElement {
        let region = self.bounds.clone();
        let canvas_region = self.canvas_bounds.clone();
        let weak = cx.weak_entity();
        let commands = self.commands.clone();
        div()
            .id("visual-canvas")
            .absolute()
            .inset_0()
            .size_full()
            .overflow_hidden()
            .cursor(gpui_kit::CursorStyle::Crosshair)
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let base = video_base_bounds(bounds, aspect, None);
                        let picture = zoomed_video_bounds(
                            base,
                            zoom,
                            point(px(pan.0 * f32::from(bounds.size.width)), px(pan.1 * f32::from(bounds.size.height))),
                        );
                        let changed =
                            *canvas_region.borrow() != base || *region.borrow() != picture;
                        *canvas_region.borrow_mut() = base;
                        *region.borrow_mut() = picture;
                        if changed {
                            // The host canvas changed size or position: the
                            // recorded primitives are in the old pixels.
                            let weak = weak.clone();
                            cx.defer(move |cx| {
                                let _ = weak.update(cx, |view, cx| {
                                    view.visual.drawn = None;
                                    view.queue_visual(json!({"action":"draw"}), cx);
                                });
                            });
                        }
                        commands
                            .into_iter()
                            .map(|command| {
                                let text = if command.kind == "text" {
                                    let run = TextRun {
                                        len: command.text.len(),
                                        font: window.text_style().font(),
                                        color: rgba(command.color).into(),
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    };
                                    Some(window.text_system().shape_line(
                                        command.text.clone().into(),
                                        px(command.size * zoom),
                                        &[run],
                                        None,
                                    ))
                                } else {
                                    None
                                };
                                (command, text)
                            })
                            .collect::<Vec<_>>()
                    },
                    move |bounds, commands, window, cx| {
                        let base = video_base_bounds(bounds, aspect, None);
                        let picture = zoomed_video_bounds(
                            base,
                            zoom,
                            point(
                                px(pan.0 * f32::from(bounds.size.width)),
                                px(pan.1 * f32::from(bounds.size.height)),
                            ),
                        );
                        let scale = picture_scale(picture, base);
                        window.with_content_mask(
                            Some(ContentMask { bounds: picture }),
                            |window| {
                                for (command, text) in commands {
                                    if let Some(text) = text {
                                        let _ = text.paint(
                                            picture.origin
                                                + point(px(command.x * scale), px(command.y * scale)),
                                            px(command.size * scale * 1.3),
                                            gpui_kit::TextAlign::Left,
                                            None,
                                            window,
                                            cx,
                                        );
                                    } else {
                                        paint(&command, picture.origin, scale, window);
                                    }
                                }
                            },
                        );
                    },
                )
                .absolute()
                .size_full(),
            )
            .on_mouse_move(cx.listener(
                |view, event: &gpui_kit::MouseMoveEvent, _, cx| {
                    if event.pressed_button == Some(gpui_kit::MouseButton::Middle) {
                        return;
                    }
                    let (x, y) = view.visual.local(event.position);
                    view.queue_visual(
                        json!({"action":"move", "x":x,"y":y,"shift":event.modifiers.shift,"control":event.modifiers.control || event.modifiers.platform,"alt":event.modifiers.alt}),
                        cx,
                    );
                },
            ))
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, event: &gpui_kit::MouseDownEvent, window, cx| {
                    view.key_focus = crate::KeyFocus::Video;
                    view.focus_handle.focus(window, cx);
                    if event.click_count >= 2 {
                        let (x, y) = view.visual.local(event.position);
                        view.visual.dragging = true;
                        view.queue_visual(
                            json!({"action":"double", "x":x,"y":y,"shift":event.modifiers.shift,"control":event.modifiers.control || event.modifiers.platform,"alt":event.modifiers.alt}),
                            cx,
                        );
                        return;
                    }
                    let (x, y) = view.visual.local(event.position);
                    view.visual.dragging = true;
                    view.queue_visual(
                        json!({"action":"down", "x":x,"y":y,"shift":event.modifiers.shift,"control":event.modifiers.control || event.modifiers.platform,"alt":event.modifiers.alt}),
                        cx,
                    );
                }),
            )
            .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                if !hovered && !view.visual.dragging {
                    view.queue_visual(json!({"action":"leave","x":0,"y":0}), cx);
                }
            }))
            .on_mouse_up(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, event: &gpui_kit::MouseUpEvent, _, cx| {
                    if !view.visual.dragging {
                        return;
                    }
                    let (x, y) = view.visual.local(event.position);
                    view.visual.dragging = false;
                    view.queue_visual(
                        json!({"action":"up", "x":x,"y":y,"shift":event.modifiers.shift,"control":event.modifiers.control || event.modifiers.platform,"alt":event.modifiers.alt}),
                        cx,
                    );
                }),
            )
            .on_mouse_up_out(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, event: &gpui_kit::MouseUpEvent, _, cx| {
                    if !view.visual.dragging {
                        return;
                    }
                    let (x, y) = view.visual.local(event.position);
                    view.visual.dragging = false;
                    view.queue_visual(
                        json!({"action":"up", "x":x,"y":y,"shift":event.modifiers.shift,"control":event.modifiers.control || event.modifiers.platform,"alt":event.modifiers.alt}),
                        cx,
                    );
                }),
            )
    }
}

/// Screen pixels per host canvas pixel.
fn picture_scale(picture: Bounds<Pixels>, base: Bounds<Pixels>) -> f32 {
    let base = f32::from(base.size.width);
    if base < 1. {
        return 1.;
    }
    (f32::from(picture.size.width) / base).clamp(0.01, 100.)
}

pub fn fit_video_bounds(bounds: Bounds<Pixels>, aspect: f32) -> Bounds<Pixels> {
    let width = f32::from(bounds.size.width).min(f32::from(bounds.size.height) * aspect.max(0.01));
    let height = width / aspect.max(0.01);
    Bounds::new(
        bounds.origin
            + point(
                (bounds.size.width - px(width)) / 2.,
                (bounds.size.height - px(height)) / 2.,
            ),
        size(px(width), px(height)),
    )
}

pub fn video_base_bounds(
    bounds: Bounds<Pixels>,
    aspect: f32,
    display_size: Option<gpui_kit::Size<Pixels>>,
) -> Bounds<Pixels> {
    let mut image = display_size.unwrap_or_else(|| fit_video_bounds(bounds, aspect).size);
    // C++ visual tools receive integer sizes; paint and hit testing use exactly those sizes.
    image.width = px(f32::from(image.width).round().clamp(16., 8192.));
    image.height = px(f32::from(image.height).round().clamp(16., 8192.));
    Bounds::new(
        bounds.center() - point(image.width / 2., image.height / 2.),
        image,
    )
}

pub fn zoomed_video_bounds(base: Bounds<Pixels>, zoom: f32, pan: Point<Pixels>) -> Bounds<Pixels> {
    let zoom = zoom.clamp(0.125, 10.);
    let size = size(base.size.width * zoom, base.size.height * zoom);
    Bounds::new(
        base.center() - gpui_kit::point(size.width / 2., size.height / 2.) + pan,
        size,
    )
}

fn rgba(color: [f32; 4]) -> gpui_kit::Rgba {
    gpui_kit::Rgba {
        r: color[0],
        g: color[1],
        b: color[2],
        a: color[3],
    }
}

fn paint(command: &DrawCommand, origin: Point<Pixels>, zoom: f32, window: &mut Window) {
    if command.kind == "text" || command.color[3] <= 0. {
        return;
    }
    let color = rgba(command.color);
    if command.kind == "mask" {
        if let Some(path) = mask_path(command, origin, zoom) {
            window.paint_path(path, color);
        }
        return;
    }
    let mut builder = if command.fill {
        PathBuilder::fill()
    } else {
        PathBuilder::stroke(px(command.width.max(1.) * zoom))
    };
    if command.primitive == 8 {
        // GL_QUAD_STRIP
        for i in (0..command.points.len().saturating_sub(3)).step_by(2) {
            append(
                &mut builder,
                &[
                    command.points[i],
                    command.points[i + 1],
                    command.points[i + 3],
                    command.points[i + 2],
                ],
                origin,
                zoom,
                true,
            );
        }
    } else if command.primitive == 1 {
        for line in command.points.chunks_exact(2) {
            append(&mut builder, line, origin, zoom, false);
        }
    } else {
        append(
            &mut builder,
            &command.points,
            origin,
            zoom,
            command.fill || command.primitive == 2,
        );
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn mask_path(command: &DrawCommand, origin: Point<Pixels>, zoom: f32) -> Option<Path<Pixels>> {
    let mut builder = PathBuilder::fill().with_style(PathStyle::Fill(
        FillOptions::default().with_fill_rule(FillRule::NonZero),
    ));
    for polygon in &command.polygons {
        append(&mut builder, polygon, origin, zoom, true);
    }
    let inside = builder.build().ok()?;
    if !command.outside {
        return Some(inside);
    }
    // Tessellate the ASS nonzero-winding region first. Its disjoint triangles
    // can then be subtracted using even/odd fill, including overlaps and holes.
    let mut outside = PathBuilder::fill();
    append(&mut outside, &command.bounds, origin, zoom, true);
    for triangle in inside.vertices.chunks_exact(3) {
        outside.move_to(triangle[0].xy_position);
        outside.line_to(triangle[1].xy_position);
        outside.line_to(triangle[2].xy_position);
        outside.close();
    }
    outside.build().ok()
}

fn append(
    builder: &mut PathBuilder,
    points: &[[f32; 2]],
    origin: Point<Pixels>,
    zoom: f32,
    closed: bool,
) {
    for (index, value) in points.iter().enumerate() {
        let position = origin + point(px(value[0] * zoom), px(value[1] * zoom));
        if index == 0 {
            builder.move_to(position);
        } else {
            builder.line_to(position);
        }
    }
    if closed && !points.is_empty() {
        builder.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(polygons: Vec<Vec<[f32; 2]>>, outside: bool) -> DrawCommand {
        DrawCommand {
            kind: "mask".into(),
            primitive: 0,
            fill: true,
            width: 0.,
            points: vec![],
            polygons,
            bounds: vec![[0., 0.], [20., 0.], [20., 20.], [0., 20.]],
            outside,
            color: [0., 0., 0., 0.5],
            text: String::new(),
            x: 0.,
            y: 0.,
            size: 0.,
        }
    }

    fn area(command: &DrawCommand, zoom: f32) -> f32 {
        let path = mask_path(command, point(px(0.), px(0.)), zoom).expect("mask must tessellate");
        path.vertices
            .chunks_exact(3)
            .map(|v| {
                let a = v[1].xy_position - v[0].xy_position;
                let b = v[2].xy_position - v[0].xy_position;
                (f32::from(a.x) * f32::from(b.y) - f32::from(a.y) * f32::from(b.x)).abs() / 2.
            })
            .sum()
    }

    #[test]
    fn ass_mask_nonzero_union_and_inverse() {
        let a = vec![[2., 2.], [12., 2.], [12., 12.], [2., 12.]];
        let b = vec![[7., 2.], [17., 2.], [17., 12.], [7., 12.]];
        assert!((area(&mask(vec![a.clone(), b.clone()], false), 1.) - 150.).abs() < 0.01);
        assert!((area(&mask(vec![a.clone(), b], true), 1.) - 250.).abs() < 0.01);
        let mut reversed = a.clone();
        reversed.reverse();
        assert!((area(&mask(vec![reversed], true), 1.) - 300.).abs() < 0.01);
        assert!((area(&mask(vec![a.clone(), a], true), 1.) - 300.).abs() < 0.01);
    }

    #[test]
    fn ass_mask_preserves_holes() {
        let outer = vec![[2., 2.], [18., 2.], [18., 18.], [2., 18.]];
        let hole = vec![[5., 5.], [5., 15.], [15., 15.], [15., 5.]];
        assert!((area(&mask(vec![outer.clone(), hole.clone()], false), 1.) - 156.).abs() < 0.01);
        assert!((area(&mask(vec![outer, hole], true), 1.) - 244.).abs() < 0.01);
    }

    #[test]
    fn canvas_coordinates_follow_letterboxed_video() {
        let container = Bounds::new(point(px(10.), px(20.)), size(px(1200.), px(400.)));
        let video = fit_video_bounds(container, 16. / 9.);
        assert!((f32::from(video.size.width) - 711.1111).abs() < 0.01);
        assert!((f32::from(video.size.height) - 400.).abs() < 0.01);
        assert!((f32::from(video.center().x) - 610.).abs() < 0.01);
        let portrait = fit_video_bounds(container, 0.5);
        assert_eq!(portrait.size, size(px(200.), px(400.)));
    }

    #[test]
    fn zoomed_picture_keeps_pointer_on_the_same_canvas_pixel() {
        let viewport = Bounds::new(point(px(30.), px(40.)), size(px(800.), px(450.)));
        let base = video_base_bounds(viewport, 16. / 9., None);
        let panel = VisualPanel::new();
        let anchor = base.origin + point(px(200.), px(130.));
        *panel.canvas_bounds.borrow_mut() = base;
        *panel.bounds.borrow_mut() = base;
        assert_eq!(panel.local(anchor), (200, 130));
        // The picture is scaled by exactly two, so a canvas pixel keeps its
        // position relative to the picture's origin.
        let zoomed = zoomed_video_bounds(base, 2., point(px(40.), px(-10.)));
        *panel.bounds.borrow_mut() = zoomed;
        assert_eq!(
            panel.local(zoomed.origin + point(px(400.), px(260.))),
            (200, 130)
        );
        assert_eq!(
            f32::from(zoomed.size.width),
            2. * f32::from(base.size.width)
        );
    }

    #[test]
    fn scaled_masks_keep_area_and_inverse_holes() {
        let outer = vec![[2., 2.], [18., 2.], [18., 18.], [2., 18.]];
        let hole = vec![[5., 5.], [5., 15.], [15., 15.], [15., 5.]];
        for zoom in [0.5, 2., 4.] {
            assert!(
                (area(&mask(vec![outer.clone(), hole.clone()], false), zoom) - 156. * zoom * zoom)
                    .abs()
                    < 0.01
            );
            assert!(
                (area(&mask(vec![outer.clone(), hole.clone()], true), zoom) - 244. * zoom * zoom)
                    .abs()
                    < 0.01
            );
        }
    }
}
