use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    StyledExt, Theme,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
};
use gpui_kit::{
    App, AppContext, Context, Entity, IntoElement, Subscription, Window, div, prelude::*, px,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Snapshot, Workspace, format_time};

#[derive(Clone, Default, Deserialize)]
pub struct ShiftState {
    pub by_time: bool,
    pub backward: bool,
    pub time: i64,
    pub frames: i64,
    pub mode: i64,
    pub fields: i64,
    pub history: Vec<Value>,
    pub changed: Option<usize>,
}

pub struct TimingPanel {
    pub time: Entity<InputState>,
    pub frames: Entity<InputState>,
    pub by_time: bool,
    pub backward: bool,
    pub mode: i64,
    pub fields: i64,
    pub initialized: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub history: Vec<Value>,
    pub confirm_clear: bool,
    show_history: bool,
    pending_inputs: Option<(String, String)>,
    _events: Vec<Subscription>,
}

impl TimingPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let time = cx.new(|cx| InputState::new(window, cx).default_value("0:00:00.00"));
        let frames = cx.new(|cx| InputState::new(window, cx).default_value("0"));
        let events = [&time, &frames]
            .into_iter()
            .map(|input| {
                cx.subscribe(input, |view, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        view.shift_times_request("apply", cx);
                    }
                    if matches!(event, InputEvent::Change) {
                        view.timing.error = None;
                        cx.notify();
                    }
                })
            })
            .collect();
        Self {
            time,
            frames,
            by_time: true,
            backward: false,
            mode: 0,
            fields: 0,
            initialized: false,
            loading: false,
            error: None,
            history: vec![],
            confirm_clear: false,
            show_history: false,
            pending_inputs: None,
            _events: events,
        }
    }

    pub fn receive(&mut self, state: ShiftState) {
        if !self.initialized {
            self.by_time = state.by_time;
            self.backward = state.backward;
            self.mode = state.mode;
            self.fields = state.fields;
            self.pending_inputs = Some((format_time(state.time), state.frames.to_string()));
            self.initialized = true;
        }
        self.history = state.history;
    }

    pub fn sync(&mut self, window: &mut Window, cx: &mut App) {
        if let Some((time, frames)) = self.pending_inputs.take() {
            self.time
                .update(cx, |state, cx| state.set_value(time, window, cx));
            self.frames
                .update(cx, |state, cx| state.set_value(frames, window, cx));
        }
    }

    fn restore(&mut self, entry: &Value, cx: &App) {
        self.by_time = entry["is by time"].as_bool().unwrap_or(true);
        self.backward = entry["is backward"].as_bool().unwrap_or(false);
        self.mode = entry["mode"].as_i64().unwrap_or(0).clamp(0, 2);
        self.fields = entry["fields"].as_i64().unwrap_or(0).clamp(0, 2);
        let amount = entry["amount"].as_str().unwrap_or("0").to_owned();
        self.pending_inputs = Some(if self.by_time {
            (amount, self.frames.read(cx).value().to_string())
        } else {
            (self.time.read(cx).value().to_string(), amount)
        });
        self.error = None;
    }

    pub fn render(&self, snapshot: &Snapshot, cx: &mut Context<Workspace>) -> impl IntoElement {
        let disabled = self.loading || !self.initialized;
        let p = crate::workspace_theme::Palette::current(cx);
        let units = crate::page::segmented(
            cx,
            vec![
                ("shift-unit-time".into(), "按时间", self.by_time, disabled),
                (
                    "shift-unit-frames".into(),
                    "按帧",
                    !self.by_time,
                    disabled || !snapshot.timecodes,
                ),
            ],
            |view, index, _, cx| {
                view.timing.by_time = index == 0;
                view.timing.error = None;
                cx.notify();
            },
        );
        let direction = crate::page::segmented(
            cx,
            vec![
                (
                    "shift-direction-delay".into(),
                    "向后延迟",
                    !self.backward,
                    disabled,
                ),
                (
                    "shift-direction-advance".into(),
                    "向前提前",
                    self.backward,
                    disabled,
                ),
            ],
            |view, index, _, cx| {
                view.timing.backward = index == 1;
                cx.notify();
            },
        );
        let no_selection = snapshot.selected.is_empty();
        let scope = crate::page::segmented(
            cx,
            vec![
                ("shift-mode-all".into(), "全部行", self.mode == 0, disabled),
                (
                    "shift-mode-selected".into(),
                    "选中行",
                    self.mode == 1,
                    disabled || no_selection,
                ),
                (
                    "shift-mode-onward".into(),
                    "首个选中行及以后",
                    self.mode == 2,
                    disabled || no_selection,
                ),
            ],
            |view, index, _, cx| {
                view.timing.mode = index as i64;
                cx.notify();
            },
        );
        let fields = crate::page::segmented(
            cx,
            vec![
                (
                    "shift-fields-both".into(),
                    "开始和结束",
                    self.fields == 0,
                    disabled,
                ),
                (
                    "shift-fields-start".into(),
                    "只改开始",
                    self.fields == 1,
                    disabled,
                ),
                (
                    "shift-fields-end".into(),
                    "只改结束",
                    self.fields == 2,
                    disabled,
                ),
            ],
            |view, index, _, cx| {
                view.timing.fields = index as i64;
                cx.notify();
            },
        );
        let count = match self.mode {
            1 => snapshot.selected.len(),
            2 => snapshot
                .lines
                .iter()
                .position(|line| snapshot.selected.contains(&line.id))
                .map_or(0, |index| snapshot.lines.len() - index),
            _ => snapshot.lines.len(),
        };
        let mut form = div()
            .id("shift-parameters-scroll")
            .v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scrollbar()
            .p_4()
            .gap_3()
            .child(units)
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(if self.by_time {
                        "平移时长"
                    } else {
                        "平移帧数"
                    })
                    .child(
                        Input::new(if self.by_time {
                            &self.time
                        } else {
                            &self.frames
                        })
                        .id("shift-amount")
                        .aria_label(if self.by_time {
                            "平移时长"
                        } else {
                            "平移帧数"
                        })
                        .disabled(disabled)
                        .w_full(),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(Theme::global(cx).muted_foreground)
                            .child(if self.by_time {
                                "格式：时:分:秒.百分秒，例如 0:00:01.50。"
                            } else {
                                "输入非负整数，使用项目当前的帧率或变帧率时间码。"
                            }),
                    ),
            )
            .child(div().v_flex().gap_1().child("方向").child(direction))
            .child(div().v_flex().gap_1().child("影响范围").child(scope))
            .child(div().v_flex().gap_1().child("修改时间字段").child(fields));
        if !snapshot.timecodes {
            form = form.child(
                div()
                    .text_sm()
                    .text_color(Theme::global(cx).muted_foreground)
                    .child("载入视频或时间码后可按帧平移。"),
            );
        }
        if let Some(error) = &self.error {
            form = form.child(
                div()
                    .text_color(Theme::global(cx).danger)
                    .child(error.clone()),
            );
        }
        let actions = div()
            .h_flex()
            .flex_shrink_0()
            .flex_wrap()
            .items_center()
            .gap_2()
            .p_3()
            .border_t_1()
            .border_color(p.divider)
            .child(
                Button::new("shift-apply")
                    .icon(IconName::Check)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .label(if self.loading {
                        "处理中…"
                    } else {
                        "应用平移"
                    })
                    .primary()
                    .disabled(disabled || count == 0 || (!self.by_time && !snapshot.timecodes))
                    .on_click(cx.listener(|view, _, _, cx| view.shift_times_request("apply", cx))),
            )
            .child(
                Button::new("shift-undo")
                    .icon(IconName::Undo2)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .label("撤销")
                    .disabled(self.loading)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.invoke(json!({"op":"command", "name":"edit/undo"}), cx)
                    })),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(p.muted)
                    .child(format!("影响 {count} 行 · 可撤销")),
            );
        let parameters = div()
            .id("shift-parameters")
            .v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .rounded(px(crate::workspace_theme::RADIUS_LG))
            .border_1()
            .border_color(p.border)
            .bg(p.surface)
            .shadow(crate::page::card_shadow(Theme::global(cx).is_dark()))
            .child(
                div()
                    .flex_shrink_0()
                    .px_4()
                    .py_3()
                    .font_semibold()
                    .border_b_1()
                    .border_color(p.divider)
                    .child("平移参数"),
            )
            .child(form)
            .child(actions);
        let mut history = div()
            .id("shift-history-list")
            .v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .p_2()
            .gap_1()
            .overflow_y_scrollbar();
        for (index, entry) in self.history.iter().enumerate() {
            let entry = entry.clone();
            let filename = entry["filename"]
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or("未命名字幕");
            let amount = entry["amount"].as_str().unwrap_or("?");
            let unit = if entry["is by time"] == true {
                ""
            } else {
                " 帧"
            };
            let direction = if entry["is backward"] == true {
                "提前"
            } else {
                "延迟"
            };
            let scope = match entry["mode"].as_i64() {
                Some(1) => "选中行",
                Some(2) => "选择及以后",
                _ => "全部行",
            };
            let fields = match entry["fields"].as_i64() {
                Some(1) => "开始",
                Some(2) => "结束",
                _ => "开始+结束",
            };
            let details = format!("{direction} {amount}{unit} · {scope} · {fields}");
            let description = format!("{filename} · {details}");
            history = history.child(
                Button::new(format!("shift-history-{index}"))
                    .child(gpui_kit::component::Icon::new(IconName::Clock3).size(gpui_kit::px(15.)))
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .ghost()
                    .w_full()
                    .h(px(60.))
                    .accessibility_label(description.clone())
                    .tooltip(description)
                    .child(
                        div()
                            .v_flex()
                            .w_full()
                            .min_w_0()
                            .text_left()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(filename.to_owned()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(details),
                            ),
                    )
                    .disabled(disabled)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.timing.restore(&entry, cx);
                        view.timing.show_history = false;
                        view.status = "已恢复平移参数，点击“应用平移”后生效".into();
                        cx.notify();
                    })),
            );
        }
        if self.history.is_empty() {
            history = history.child(div().text_sm().child("尚无平移记录。"));
        }
        let history = div()
            .id("shift-history")
            .v_flex()
            .min_h_0()
            .min_w_0()
            .flex_1()
            .overflow_hidden()
            .rounded(px(crate::workspace_theme::RADIUS_LG))
            .border_1()
            .border_color(p.border)
            .bg(p.surface)
            .shadow(crate::page::card_shadow(Theme::global(cx).is_dark()))
            .child(
                div()
                    .h_flex()
                    .flex_shrink_0()
                    .justify_between()
                    .items_center()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(p.divider)
                    .child(
                        div()
                            .font_semibold()
                            .child(format!("历史记录 · {}", self.history.len())),
                    )
                    .child(
                        Button::new("shift-clear-history")
                            .icon(IconName::Delete)
                            .rounded(gpui_kit::px(999.))
                            .small()
                            .px(gpui_kit::px(13.))
                            .py(gpui_kit::px(3.))
                            .label(if self.confirm_clear {
                                "确认清空"
                            } else {
                                "清空…"
                            })
                            .disabled(disabled || self.history.is_empty())
                            .on_click(cx.listener(|view, _, _, cx| {
                                if view.timing.confirm_clear {
                                    view.timing.confirm_clear = false;
                                    view.shift_times_request("clear_history", cx);
                                } else {
                                    view.timing.confirm_clear = true;
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(p.muted)
                    .child("点击记录填入参数，再点击应用；使用当前字幕选择。"),
            )
            .child(history);
        // Parameters beside the history, not above it: stacked they forced the
        // tool window to 500 px wide and 1200 px tall, and the history mostly
        // sat below the fold. Both cards share the height and scroll inside,
        // so the wrapping row has to stretch them instead of sizing them to
        // their content.
        div()
            .id("timing-panel")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .p_3()
            .h_flex()
            .items_stretch()
            .flex_wrap()
            .gap_3()
            .child(parameters.flex_1().min_w(px(320.)))
            .child(history.flex_1().min_w(px(300.)))
    }
}
