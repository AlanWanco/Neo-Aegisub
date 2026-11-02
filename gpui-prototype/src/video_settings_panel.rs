use std::collections::BTreeMap;

use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    StyledExt, Theme,
    button::{Button, ButtonVariants},
    color_picker::ColorPicker,
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{
    Anchor, App, AppContext, Context, Entity, IntoElement, Subscription, Window, div, prelude::*,
    px,
};
use serde::Deserialize;

use crate::colour_picker::LinkedPicker;
use crate::{Workspace, format_time};

#[derive(Clone, Copy, PartialEq)]
pub enum VideoPage {
    Dummy,
    Jump,
    Aspect,
    Details,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct VideoSettings {
    pub loaded: bool,
    pub path: String,
    pub width: i64,
    pub height: i64,
    pub frames: i64,
    pub fps: f64,
    pub duration: i64,
    pub decoder: String,
    pub actual_matrix: String,
    pub forced_matrix: String,
    pub actual_range: String,
    pub forced_range: String,
    pub frame: i64,
    pub time: i64,
    pub aspect: f64,
    pub dummy_fps: String,
    pub dummy_width: i64,
    pub dummy_height: i64,
    pub dummy_length: i64,
    pub dummy_color: String,
    pub dummy_pattern: bool,
}

pub struct VideoSettingsPanel {
    pub state: VideoSettings,
    pub page: VideoPage,
    pub inputs: BTreeMap<&'static str, Entity<InputState>>,
    pub pattern: bool,
    pub initialized: bool,
    pub loading: bool,
    pub error: Option<String>,
    pending_values: BTreeMap<&'static str, String>,
    /// The dummy video colour is `#RRGGBB`; the picker edits that same text.
    color_picker: LinkedPicker,
    _events: Vec<Subscription>,
}

impl VideoSettingsPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let mut inputs = BTreeMap::new();
        let mut events = Vec::new();
        for key in [
            "fps", "width", "height", "length", "color", "frame", "time", "aspect",
        ] {
            let input = cx.new(|cx| InputState::new(window, cx));
            events.push(
                cx.subscribe(&input, move |view, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        view.video_settings.error = None;
                        cx.notify();
                    }
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        let action = match key {
                            "frame" => "jump_frame",
                            "time" => "jump_time",
                            "aspect" => "aspect",
                            _ => "dummy",
                        };
                        view.video_settings_request(action, cx);
                    }
                }),
            );
            inputs.insert(key, input);
        }
        let color_picker = LinkedPicker::new(&inputs["color"], false, window, cx, |view, cx| {
            view.video_settings.error = None;
            cx.notify();
        });
        Self {
            state: VideoSettings::default(),
            page: VideoPage::Details,
            inputs,
            pattern: false,
            initialized: false,
            loading: false,
            error: None,
            pending_values: BTreeMap::new(),
            color_picker,
            _events: events,
        }
    }

    pub fn receive(&mut self, state: VideoSettings) {
        if !self.initialized {
            self.pending_values = BTreeMap::from([
                ("fps", state.dummy_fps.clone()),
                ("width", state.dummy_width.to_string()),
                ("height", state.dummy_height.to_string()),
                ("length", state.dummy_length.to_string()),
                ("color", state.dummy_color.clone()),
                ("frame", state.frame.to_string()),
                ("time", format_time(state.time)),
                ("aspect", format!("{:.6}", state.aspect)),
            ]);
            self.pattern = state.dummy_pattern;
            self.initialized = true;
        }
        self.state = state;
    }

    pub fn sync(&mut self, window: &mut Window, cx: &mut App) {
        for (key, value) in std::mem::take(&mut self.pending_values) {
            self.inputs[key].update(cx, |state, cx| state.set_value(value, window, cx));
            if key == "color" {
                self.color_picker.pull(&self.inputs[key], window, cx);
            }
        }
    }

    pub fn render(&self, cx: &mut Context<Workspace>) -> impl IntoElement {
        let p = crate::workspace_theme::Palette::current(cx);
        let disabled = self.loading || !self.initialized;
        let tabs = crate::page::tabs(
            cx,
            [
                (VideoPage::Details, "视频详情"),
                (VideoPage::Jump, "精确跳转"),
                (VideoPage::Aspect, "显示比例"),
                (VideoPage::Dummy, "空白视频"),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (page, label))| {
                (
                    format!("video-page-{index}"),
                    label.to_owned(),
                    self.page == page,
                )
            })
            .collect(),
            |view, index, _, cx| {
                view.video_settings.page = match index {
                    0 => VideoPage::Details,
                    1 => VideoPage::Jump,
                    2 => VideoPage::Aspect,
                    _ => VideoPage::Dummy,
                };
                cx.notify();
            },
        );
        let mut panel = crate::page::page(cx, "video-settings-panel").child(tabs);
        let fields: Vec<_> = match self.page {
            VideoPage::Dummy => vec![
                ("width", "宽度 · 像素"),
                ("height", "高度 · 像素"),
                ("fps", "帧率 · 支持 24000/1001"),
                ("length", "持续帧数"),
                ("color", "颜色 · #RRGGBB"),
            ],
            VideoPage::Jump => vec![
                ("frame", "跳转帧号 · 从 0 开始"),
                ("time", "跳转时间 · 时:分:秒.百分秒"),
            ],
            VideoPage::Aspect => vec![("aspect", "比例 · 小数、16:9 或 853x480")],
            _ => vec![],
        };
        if !fields.is_empty() {
            let mut inputs = div().h_flex().flex_wrap().items_start().gap_3();
            for (key, label) in fields {
                inputs = inputs.child(
                    div()
                        .v_flex()
                        .gap_1()
                        .w(px(280.))
                        .child(crate::page::field_label(cx, label))
                        .child({
                            let input = Input::new(&self.inputs[key]).disabled(
                                disabled || (self.page != VideoPage::Dummy && !self.state.loaded),
                            );
                            if key == "color" {
                                div()
                                    .h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        ColorPicker::new(&self.color_picker.picker)
                                            .anchor(Anchor::TopLeft)
                                            .accessibility_label(label),
                                    )
                                    .child(input)
                                    .into_any_element()
                            } else {
                                input.into_any_element()
                            }
                        }),
                );
            }
            let action = match self.page {
                VideoPage::Dummy => Some(
                    Button::new("dummy-video-create")
                        .icon(IconName::Video)
                        .rounded(gpui_kit::px(999.))
                        .small()
                        .px(gpui_kit::px(13.))
                        .py(gpui_kit::px(3.))
                        .label("载入空白视频")
                        .primary()
                        .disabled(disabled)
                        .on_click(
                            cx.listener(|view, _, _, cx| view.video_settings_request("dummy", cx)),
                        )
                        .into_any_element(),
                ),
                VideoPage::Jump => Some(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("jump-frame-apply")
                                .icon(IconName::Frame)
                                .rounded(gpui_kit::px(999.))
                                .small()
                                .px(gpui_kit::px(13.))
                                .py(gpui_kit::px(3.))
                                .label("按帧跳转")
                                .primary()
                                .disabled(disabled || !self.state.loaded)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.video_settings_request("jump_frame", cx)
                                })),
                        )
                        .child(
                            Button::new("jump-time-apply")
                                .icon(IconName::Clock3)
                                .rounded(gpui_kit::px(999.))
                                .small()
                                .px(gpui_kit::px(13.))
                                .py(gpui_kit::px(3.))
                                .label("按时间跳转")
                                .disabled(disabled || !self.state.loaded)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.video_settings_request("jump_time", cx)
                                })),
                        )
                        .into_any_element(),
                ),
                VideoPage::Aspect => Some(
                    Button::new("custom-aspect-apply")
                        .icon(IconName::Check)
                        .rounded(gpui_kit::px(999.))
                        .small()
                        .px(gpui_kit::px(13.))
                        .py(gpui_kit::px(3.))
                        .label("应用显示比例")
                        .primary()
                        .disabled(disabled || !self.state.loaded)
                        .on_click(
                            cx.listener(|view, _, _, cx| view.video_settings_request("aspect", cx)),
                        )
                        .into_any_element(),
                ),
                _ => None,
            };
            let (title, icon) = match self.page {
                VideoPage::Dummy => ("创建空白视频", IconName::Video),
                VideoPage::Jump => ("跳转到时间点", IconName::Locate),
                _ => ("设置显示比例", IconName::ZoomIn),
            };
            let mut body = crate::page::section_body().child(inputs);
            if self.page == VideoPage::Dummy {
                let mut resolutions = div().h_flex().flex_wrap().gap_2();
                for (width, height) in [
                    (640, 480),
                    (1280, 720),
                    (1920, 1080),
                    (2560, 1440),
                    (3840, 2160),
                    (1080, 1920),
                ] {
                    resolutions = resolutions.child(
                        Button::new(format!("dummy-resolution-{width}-{height}"))
                            .icon(IconName::Frame)
                            .rounded(gpui_kit::px(999.))
                            .small()
                            .px(gpui_kit::px(13.))
                            .py(gpui_kit::px(3.))
                            .label(format!("{width} × {height}"))
                            .disabled(disabled)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                for (key, value) in [("width", width), ("height", height)] {
                                    view.video_settings.inputs[key].update(cx, |state, cx| {
                                        state.set_value(value.to_string(), window, cx)
                                    });
                                }
                            })),
                    );
                }
                body = body
                    .child(crate::page::field_label(cx, "常用分辨率"))
                    .child(resolutions)
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                gpui_kit::component::switch::Switch::new("dummy-pattern")
                                    .checked(self.pattern)
                                    .disabled(disabled)
                                    .on_change(cx.listener(|view, checked: &bool, _, cx| {
                                        view.video_settings.pattern = *checked;
                                        cx.notify();
                                    })),
                            )
                            .child(crate::page::field_label(cx, "棋盘格图案")),
                    );
            }
            if self.page == VideoPage::Aspect {
                body = body.child(crate::page::hint(
                    cx,
                    "有效范围 0.5–5.0，调整视频预览的显示比例。",
                ));
            }
            if let Some(action) = action {
                body = body.child(div().h_flex().items_center().gap_3().child(action));
            }
            panel = panel.child(crate::page::section_with_icon(
                cx,
                Some(icon),
                title,
                None,
                body,
            ));
        }
        if self.page == VideoPage::Details {
            let state = &self.state;
            let mut body = crate::page::section_body();
            if state.loaded {
                for (label, value) in [
                    ("文件", state.path.clone()),
                    ("分辨率", format!("{} × {}", state.width, state.height)),
                    ("帧率", format!("{:.6}", state.fps)),
                    (
                        "长度",
                        format!("{} 帧 · {}", state.frames, format_time(state.duration)),
                    ),
                    (
                        "当前帧",
                        format!("{} · {}", state.frame, format_time(state.time)),
                    ),
                    ("显示比例", format!("{:.6}", state.aspect)),
                    ("实际色彩矩阵", state.actual_matrix.clone()),
                    ("强制色彩矩阵", state.forced_matrix.clone()),
                    ("实际范围", state.actual_range.clone()),
                    ("强制范围", state.forced_range.clone()),
                    ("解码器", state.decoder.clone()),
                ] {
                    body = body.child(
                        div()
                            .h_flex()
                            .items_start()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(110.))
                                    .flex_shrink_0()
                                    .text_sm()
                                    .text_color(p.muted)
                                    .child(label),
                            )
                            .child(div().flex_1().min_w_0().text_sm().child(value)),
                    );
                }
            } else {
                body = body.child(crate::page::hint(
                    cx,
                    "尚未载入视频。打开视频文件即可查看详情，或切到“空白视频”生成占位画面。",
                ));
            }
            panel = panel.child(crate::page::section_with_icon(
                cx,
                Some(IconName::Video),
                if state.loaded {
                    "视频详情"
                } else {
                    "尚未载入视频"
                },
                Some(
                    Button::new("refresh-video-details")
                        .icon(IconName::RefreshCw)
                        .rounded(gpui_kit::px(999.))
                        .small()
                        .px(gpui_kit::px(13.))
                        .py(gpui_kit::px(3.))
                        .label("刷新详情")
                        .disabled(self.loading)
                        .on_click(
                            cx.listener(|view, _, _, cx| view.video_settings_request("state", cx)),
                        )
                        .into_any_element(),
                ),
                body,
            ));
        }
        if let Some(error) = &self.error {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(Theme::global(cx).danger)
                    .child(error.clone()),
            );
        }
        panel
    }
}
