use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::{Context, IntoElement, prelude::*};
use serde_json::json;

use crate::Workspace;

const VISUAL_TOOLS: [(&str, &str); 7] = [
    ("cross", "十字"),
    ("drag", "拖动"),
    ("rotate/z", "Z 旋转"),
    ("rotate/xy", "XY 旋转"),
    ("scale", "缩放"),
    ("clip", "矩形裁剪"),
    ("vector_clip", "矢量裁剪"),
];

/// Toolbar selection for the native video window. The window draws the
/// picture and handles every pointer event; this only records which tool and
/// vector subtool the toolbar shows. The host receives them through the
/// `video_surface` configure action.
pub struct VisualPanel {
    pub tool: &'static str,
    pub subtool: usize,
}

impl VisualPanel {
    pub fn new() -> Self {
        Self {
            tool: "cross",
            subtool: 1,
        }
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
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.video_surface(vec![json!({"action":"toggle_move"})], cx);
                    })),
            );
        }
        bar
    }
}
