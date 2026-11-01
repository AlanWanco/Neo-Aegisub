use std::collections::BTreeMap;

use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{
    App, AppContext, Context, Entity, IntoElement, Subscription, Window, div, prelude::*, px,
};
use serde::Deserialize;
use serde_json::Value;

use crate::{Snapshot, Workspace};

#[derive(Default, Deserialize)]
pub struct SearchReply {
    #[serde(default)]
    pub items: Vec<SearchItem>,
    #[serde(default)]
    pub list_count: i64,
    #[serde(default)]
    pub search_defaults: Value,
    #[serde(default)]
    pub selection_defaults: Value,
    #[serde(default)]
    pub history: Vec<String>,
    pub count: Option<usize>,
    #[serde(default)]
    pub rows: Vec<i64>,
    pub found: Option<bool>,
    pub replaced: Option<bool>,
    pub row: Option<i64>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct SearchItem {
    #[serde(default)]
    pub row: i64,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub after: String,
}

pub struct SearchPanel {
    pub find: Entity<InputState>,
    pub replace: Entity<InputState>,
    pub field: i64,
    pub mode: i64,
    pub selection_action: i64,
    pub options: BTreeMap<&'static str, bool>,
    pub selection_mode: bool,
    pub initialized: bool,
    pub loading: bool,
    pub focus_pending: bool,
    pub error: Option<String>,
    pub message: String,
    pub history: Vec<String>,
    pub items: Vec<SearchItem>,
    pub items_count: i64,
    pending_find: Option<String>,
    pending_replace: Option<String>,
    _events: Vec<Subscription>,
}

impl SearchPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let find = cx.new(|cx| InputState::new(window, cx));
        let replace = cx.new(|cx| InputState::new(window, cx));
        let mut events = Vec::new();
        for (input, replacing) in [(&find, false), (&replace, true)] {
            events.push(cx.subscribe(input, move |view, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    view.find_panel.error = None;
                    cx.notify();
                }
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.search_request(
                        if view.find_panel.selection_mode {
                            "select"
                        } else if replacing {
                            "replace"
                        } else {
                            "find"
                        },
                        cx,
                    );
                }
            }));
        }
        Self {
            find,
            replace,
            field: 0,
            mode: 1,
            selection_action: 0,
            options: BTreeMap::from([
                ("match_case", false),
                ("use_regex", false),
                ("ignore_comments", false),
                ("skip_tags", false),
                ("only_selection", false),
                ("invert", false),
                ("comments", false),
                ("dialogue", true),
            ]),
            selection_mode: false,
            items: Vec::new(),
            items_count: 0,
            initialized: false,
            loading: false,
            focus_pending: false,
            error: None,
            message: String::new(),
            history: vec![],
            pending_find: None,
            pending_replace: None,
            _events: events,
        }
    }

    pub fn receive(&mut self, reply: SearchReply) {
        if !self.initialized {
            let defaults = if self.selection_mode {
                &reply.selection_defaults
            } else {
                &reply.search_defaults
            };
            self.field = defaults["field"].as_i64().unwrap_or(0).clamp(0, 3);
            self.mode = defaults["mode"].as_i64().unwrap_or(1).clamp(0, 2);
            self.selection_action = defaults["selection_action"]
                .as_i64()
                .unwrap_or(0)
                .clamp(0, 3);
            for (key, value) in &mut self.options {
                if let Some(option) = defaults[*key].as_bool() {
                    *value = option;
                }
            }
            self.pending_find = Some(defaults["find"].as_str().unwrap_or("").into());
            self.pending_replace = Some(defaults["replace_with"].as_str().unwrap_or("").into());
            self.initialized = true;
        }
        self.history = reply.history;
    }

    pub fn sync(&mut self, window: &mut Window, cx: &mut App) {
        if let Some(value) = self.pending_find.take() {
            self.find
                .update(cx, |state, cx| state.set_value(value, window, cx));
        }
        if let Some(value) = self.pending_replace.take() {
            self.replace
                .update(cx, |state, cx| state.set_value(value, window, cx));
        }
        if self.focus_pending {
            self.focus_pending = false;
            self.find.update(cx, |state, cx| state.focus(window, cx));
        }
    }

    pub fn render(&self, snapshot: &Snapshot, cx: &mut Context<Workspace>) -> impl IntoElement {
        let disabled = self.loading || !self.initialized;
        let modes = crate::page::segmented(
            cx,
            vec![
                (
                    "search-mode-find".into(),
                    "查找替换",
                    !self.selection_mode,
                    self.loading,
                ),
                (
                    "search-mode-select".into(),
                    "条件选行",
                    self.selection_mode,
                    self.loading,
                ),
            ],
            |view, index, _, cx| {
                view.open_search(index == 1, cx);
            },
        );
        let mut fields = vec![crate::page::field_cell(crate::page::field(
            cx,
            "查找内容",
            Input::new(&self.find).h(px(32.)).disabled(disabled),
        ))];
        if !self.selection_mode {
            fields.push(crate::page::field_cell(crate::page::field(
                cx,
                "替换为",
                Input::new(&self.replace).h(px(32.)).disabled(disabled),
            )));
        }
        let field_kind = crate::page::segmented(
            cx,
            [(0, "字幕文本"), (1, "样式"), (2, "角色"), (3, "效果")]
                .into_iter()
                .map(|(value, label)| {
                    (
                        format!("search-field-{value}"),
                        label,
                        self.field == value,
                        disabled,
                    )
                })
                .collect(),
            |view, index, _, cx| {
                view.find_panel.field = index as i64;
                cx.notify();
            },
        );
        let mut options = div().h_flex().flex_wrap().gap_2();
        let keys = if self.selection_mode {
            vec![
                ("match_case", "区分大小写"),
                ("invert", "选择不匹配项"),
                ("dialogue", "对白"),
                ("comments", "注释"),
            ]
        } else {
            vec![
                ("match_case", "区分大小写"),
                ("use_regex", "正则表达式"),
                ("ignore_comments", "跳过注释"),
                ("skip_tags", "跳过 ASS 标签"),
                ("only_selection", "仅选中行"),
            ]
        };
        for (key, label) in keys {
            options = options.child(
                crate::page::toggle_chip(
                    cx,
                    format!("search-option-{key}"),
                    label,
                    self.options[key],
                    disabled,
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    let value = view.find_panel.options[key];
                    view.find_panel.options.insert(key, !value);
                    if matches!(key, "comments" | "dialogue") && value {
                        let other = if key == "comments" {
                            "dialogue"
                        } else {
                            "comments"
                        };
                        view.find_panel.options.insert(other, true);
                    }
                    cx.notify();
                })),
            );
        }
        let mut panel = crate::page::page(cx, "search-panel")
            .child(modes)
            .child(crate::page::section_with_icon(
                cx,
                Some(IconName::Search),
                if self.selection_mode {
                    "匹配条件"
                } else {
                    "查找与替换"
                },
                None,
                crate::page::section_body().child(crate::page::field_row(fields)),
            ))
            .child(crate::page::section_with_icon(
                cx,
                Some(IconName::Settings2),
                "检索范围",
                None,
                crate::page::section_body()
                    .child(crate::page::field_label(cx, "检索字段"))
                    .child(field_kind)
                    .child(crate::page::field_label(cx, "选项"))
                    .child(options),
            ));
        if self.selection_mode {
            let match_mode = crate::page::segmented(
                cx,
                [(0, "完全匹配"), (1, "包含"), (2, "正则表达式")]
                    .into_iter()
                    .map(|(mode, label)| {
                        (
                            format!("selection-match-mode-{mode}"),
                            label,
                            self.mode == mode,
                            disabled,
                        )
                    })
                    .collect(),
                |view, index, _, cx| {
                    view.find_panel.mode = index as i64;
                    cx.notify();
                },
            );
            let selection_action = crate::page::segmented(
                cx,
                [
                    (0, "设为选区"),
                    (1, "加入选区"),
                    (2, "从选区移除"),
                    (3, "与选区相交"),
                ]
                .into_iter()
                .map(|(action, label)| {
                    (
                        format!("selection-action-{action}"),
                        label,
                        self.selection_action == action,
                        disabled,
                    )
                })
                .collect(),
                |view, index, _, cx| {
                    view.find_panel.selection_action = index as i64;
                    cx.notify();
                },
            );
            panel = panel.child(crate::page::section_with_icon(
                cx,
                Some(IconName::Check),
                "选择方式",
                None,
                crate::page::section_body()
                    .child(crate::page::field_label(cx, "匹配方式"))
                    .child(match_mode)
                    .child(crate::page::field_label(cx, "选择操作"))
                    .child(selection_action),
            ));
        }
        let buttons: Vec<(&'static str, &'static str)> = if self.selection_mode {
            vec![("preview", "预览匹配"), ("select", "应用选择")]
        } else {
            vec![
                ("find", "查找下一个"),
                ("replace", "替换当前并查找"),
                ("replace_all", "全部替换"),
            ]
        };
        let mut toolbar = div()
            .h_flex()
            .items_center()
            .gap_2()
            .w_full()
            .pt_3()
            .mt_1()
            .border_t_1()
            .border_color(crate::workspace_theme::Palette::current(cx).border);
        for (index, (action, label)) in buttons.into_iter().enumerate() {
            let primary = index == 0;
            toolbar = toolbar.child(
                Button::new(format!("search-{action}"))
                    .icon(IconName::Search)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .label(label)
                    .when(primary, |button| button.primary())
                    .when(action == "replace_all", |button| button.ghost())
                    .disabled(
                        disabled || (!self.selection_mode && self.find.read(cx).value().is_empty()),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| view.search_request(action, cx))),
            );
        }
        toolbar = toolbar
            .child(
                div()
                    .w(px(1.))
                    .h(px(18.))
                    .mx_1()
                    .bg(crate::workspace_theme::Palette::current(cx).divider),
            )
            .child(
                Button::new("search-back-to-grid")
                    .icon(IconName::ArrowLeft)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .label("回到字幕")
                    .ghost()
                    .disabled(self.loading)
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.active_panel = crate::Panel::Subtitles;
                        view.focus_handle.focus(window, cx);
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(crate::page::badge(
                cx,
                format!("当前选中 {} 行", snapshot.selected.len()),
            ));
        panel = panel.child(toolbar);
        if !self.message.is_empty() {
            panel = panel.child(crate::page::hint(cx, self.message.clone()));
        }
        if let Some(error) = &self.error {
            panel = panel.child(crate::page::error(cx, error.clone()));
        }
        if self.loading {
            panel = panel.child(crate::page::hint(cx, "正在检索…"));
        }
        if self.items_count > 0 || !self.items.is_empty() {
            let p = crate::workspace_theme::Palette::current(cx);
            let mut rows = div().v_flex().w_full().gap_1();
            for item in self.items.iter().take(200) {
                let changed = item.after != item.text;
                rows = rows.child(
                    div()
                        .v_flex()
                        .w_full()
                        .gap_0p5()
                        .px_2()
                        .py_1()
                        .rounded(px(6.))
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(p.muted)
                                        .child(format!("{:03}", item.row)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(p.muted)
                                        .child(item.style.clone()),
                                ),
                        )
                        .child(div().text_sm().child(item.text.clone()))
                        .when(changed, |card| {
                            card.child(
                                div()
                                    .text_sm()
                                    .text_color(p.accent)
                                    .child(item.after.clone()),
                            )
                        }),
                );
            }
            panel = panel.child(crate::page::section(
                cx,
                "查找结果",
                Some(crate::page::badge(cx, format!("{} 行", self.items_count)).into_any_element()),
                crate::page::section_body().child(rows),
            ));
        }
        if !self.history.is_empty() {
            let mut history = div().h_flex().flex_wrap().gap_2();
            for (index, text) in self.history.iter().enumerate() {
                let value = text.clone();
                history = history.child(
                    Button::new(format!("search-history-{index}"))
                        .icon(IconName::Replace)
                        .rounded(gpui_kit::px(999.))
                        .small()
                        .px(gpui_kit::px(13.))
                        .py(gpui_kit::px(3.))
                        .label(text.clone())
                        .ghost()
                        .disabled(disabled)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.find_panel.pending_find = Some(value.clone());
                            cx.notify();
                        })),
                );
            }
            panel = panel.child(crate::page::section(
                cx,
                "最近查找",
                None,
                crate::page::section_body().child(history),
            ));
        }
        panel
    }
}
