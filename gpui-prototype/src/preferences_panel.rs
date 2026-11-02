use std::collections::BTreeMap;

use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    Icon, StyledExt, Theme,
    button::{Button, ButtonVariants},
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    scroll::ScrollableElement,
    switch::Switch,
};
use gpui_kit::{
    Anchor, AppContext, Context, Entity, IntoElement, Subscription, Window, div, prelude::*, px,
};
use serde::Deserialize;
use serde_json::json;

use crate::Workspace;
use crate::colour_picker::{format_hex_colour, parse_hex_colour};

#[derive(Clone, Deserialize)]
pub struct PreferenceOption {
    pub name: String,
    pub label: String,
    pub group: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
    pub minimum: f64,
    pub maximum: f64,
    pub directory: bool,
    pub is_default: bool,
    pub choices: Vec<String>,
    pub note: String,
}

#[derive(Deserialize)]
pub struct PreferenceState {
    pub options: Vec<PreferenceOption>,
}

pub struct PreferencesPanel {
    pub options: Vec<PreferenceOption>,
    pub inputs: BTreeMap<String, Entity<InputState>>,
    pub drafts: BTreeMap<String, String>,
    pub search: Entity<InputState>,
    pub pickers: BTreeMap<String, Entity<ColorPickerState>>,
    pub group: String,
    pub loading: bool,
    pub error: Option<String>,
    pub confirm_reset: bool,
    subscriptions: Vec<Subscription>,
}

impl PreferencesPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索偏好名称或路径"));
        let subscription = cx.subscribe(&search, |view, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                view.preferences.confirm_reset = false;
                cx.notify();
            }
        });
        Self {
            options: vec![],
            inputs: BTreeMap::new(),
            drafts: BTreeMap::new(),
            search,
            pickers: BTreeMap::new(),
            loading: false,
            error: None,
            confirm_reset: false,
            subscriptions: vec![subscription],
            group: String::new(),
        }
    }

    pub fn receive(&mut self, state: PreferenceState, submitted: &serde_json::Value) {
        if let Some(fields) = submitted.as_object() {
            self.drafts.retain(|key, value| {
                fields.get(key).and_then(|field| field.as_str()) != Some(value)
            });
        }
        self.options = state.options;
    }

    pub fn sync(view: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
        for option in view.preferences.options.clone() {
            if option.kind == "bool" || !option.choices.is_empty() {
                continue;
            }
            let value = view
                .preferences
                .drafts
                .get(&option.name)
                .unwrap_or(&option.value)
                .clone();
            if !view.preferences.inputs.contains_key(&option.name) {
                let input = cx.new(|cx| InputState::new(window, cx).default_value(value.clone()));
                let key = option.name.clone();
                let subscription =
                    cx.subscribe(&input, move |view, state, event: &InputEvent, cx| {
                        if matches!(event, InputEvent::Change) {
                            let value = state.read(cx).value().to_string();
                            if view
                                .preferences
                                .options
                                .iter()
                                .any(|option| option.name == key && option.value == value)
                            {
                                view.preferences.drafts.remove(&key);
                            } else {
                                view.preferences.drafts.insert(key.clone(), value);
                            }
                            view.preferences.error = None;
                            cx.notify();
                        }
                    });
                view.preferences.inputs.insert(option.name.clone(), input);
                view.preferences.subscriptions.push(subscription);
            }
            let input = &view.preferences.inputs[&option.name];
            if input.read(cx).value().as_ref() != value {
                input.update(cx, |state, cx| state.set_value(value.clone(), window, cx));
            }
            if option.kind == "color" {
                let colour =
                    parse_hex_colour(&value).unwrap_or_else(|| gpui_kit::rgba(0xffff_ffff).into());
                if !view.preferences.pickers.contains_key(&option.name) {
                    let picker =
                        cx.new(|cx| ColorPickerState::new(window, cx).default_value(colour));
                    let key = option.name.clone();
                    let subscription =
                        cx.subscribe(&picker, move |view, _, event: &ColorPickerEvent, cx| {
                            let ColorPickerEvent::Change(colour) = event;
                            let Some(colour) = colour else {
                                return;
                            };
                            let text = format_hex_colour(*colour);
                            let original = view
                                .preferences
                                .options
                                .iter()
                                .find(|option| option.name == key)
                                .map(|option| option.value.clone());
                            if original.as_deref() == Some(text.as_str()) {
                                view.preferences.drafts.remove(&key);
                            } else {
                                view.preferences.drafts.insert(key.clone(), text);
                            }
                            view.preferences.error = None;
                            cx.notify();
                        });
                    view.preferences.pickers.insert(option.name.clone(), picker);
                    view.preferences.subscriptions.push(subscription);
                }
                let picker = &view.preferences.pickers[&option.name];
                if picker.read(cx).value().map(format_hex_colour).as_deref() != Some(value.as_str())
                {
                    picker.update(cx, |state, cx| state.set_value(colour, window, cx));
                }
            }
        }
    }

    pub fn render(&self, cx: &mut Context<Workspace>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_lowercase();
        let mut names = vec!["全部".to_owned()];
        for option in &self.options {
            if !names.contains(&option.group) {
                names.push(option.group.clone());
            }
        }
        let groups = crate::page::tabs(
            cx,
            names
                .iter()
                .enumerate()
                .map(|(index, group)| {
                    (
                        format!("preference-group-{index}"),
                        group.clone(),
                        *group == self.group,
                    )
                })
                .collect(),
            |view, index, _, cx| {
                let names = {
                    let mut names = vec!["全部".to_owned()];
                    for option in &view.preferences.options {
                        if !names.contains(&option.group) {
                            names.push(option.group.clone());
                        }
                    }
                    names
                };
                if let Some(group) = names.get(index) {
                    view.preferences.group = group.clone();
                }
                view.preferences.confirm_reset = false;
                cx.notify();
            },
        );
        let mut options = div()
            .id("preference-option-list")
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_0()
            .overflow_y_scrollbar();
        let visible: Vec<_> = self
            .options
            .iter()
            .filter(|option| {
                (self.group == "全部" || self.group == option.group)
                    && (query.is_empty()
                        || format!("{} {}", option.label, option.name)
                            .to_lowercase()
                            .contains(&query))
            })
            .collect();
        for option in &visible {
            let key = option.name.clone();
            let value = self.drafts.get(&key).unwrap_or(&option.value).clone();
            let changed = self.drafts.contains_key(&key);
            let is_default = option.is_default && (!changed || value == option.value);
            let mut description = div()
                .v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().text_sm().font_semibold().child(option.label.clone()))
                        .when(changed, |label| {
                            label.child(
                                div()
                                    .text_xs()
                                    .text_color(Theme::global(cx).primary)
                                    .child("未应用"),
                            )
                        }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(Theme::global(cx).muted_foreground)
                        .child(format!(
                            "{}{}",
                            key,
                            if is_default { " · 默认" } else { "" }
                        )),
                );
            if !option.note.is_empty() {
                description = description.child(
                    div()
                        .text_xs()
                        .text_color(Theme::global(cx).muted_foreground)
                        .child(option.note.clone()),
                );
            }
            let mut control = div()
                .h_flex()
                .flex_shrink_0()
                .items_center()
                .justify_end()
                .gap_2()
                .w(px(330.));
            if option.kind == "bool" {
                control = control.child(
                    Switch::new(format!("preference-bool-{key}"))
                        .checked(value == "1")
                        .accessibility_label(option.label.clone())
                        .disabled(self.loading)
                        .on_change(cx.listener(move |view, checked: &bool, _, cx| {
                            let value = if *checked { "1" } else { "0" };
                            if view
                                .preferences
                                .options
                                .iter()
                                .any(|option| option.name == key && option.value == value)
                            {
                                view.preferences.drafts.remove(&key);
                            } else {
                                view.preferences.drafts.insert(key.clone(), value.into());
                            }
                            view.preferences.confirm_reset = false;
                            cx.notify();
                        })),
                );
            } else if !option.choices.is_empty() {
                let choices = option.choices.clone();
                let integer = option.kind == "int";
                let loading = self.loading;
                let display = if integer {
                    value
                        .parse::<usize>()
                        .ok()
                        .and_then(|index| choices.get(index))
                        .cloned()
                        .unwrap_or(value.clone())
                } else {
                    value.clone()
                };
                let weak = cx.weak_entity();
                control = control.child(
                    crate::page::select_trigger(
                        cx,
                        format!("preference-choice-{key}"),
                        if display.is_empty() {
                            "自动".to_owned()
                        } else {
                            display
                        },
                        loading,
                    )
                    .w(px(280.))
                    .dropdown_menu(move |mut menu, _, _| {
                        for (index, label) in choices.iter().enumerate() {
                            let proposed = if integer {
                                index.to_string()
                            } else {
                                label.clone()
                            };
                            let weak = weak.clone();
                            let key = key.clone();
                            menu = menu.item(
                                PopupMenuItem::new(if label.is_empty() {
                                    "自动".into()
                                } else {
                                    label.clone()
                                })
                                .checked(value == proposed)
                                .disabled(loading)
                                .on_click(move |_, _, cx| {
                                    let _ = weak.update(cx, |view, cx| {
                                        if view.preferences.options.iter().any(|option| {
                                            option.name == key && option.value == proposed
                                        }) {
                                            view.preferences.drafts.remove(&key);
                                        } else {
                                            view.preferences
                                                .drafts
                                                .insert(key.clone(), proposed.clone());
                                        }
                                        view.preferences.confirm_reset = false;
                                        cx.notify();
                                    });
                                }),
                            );
                        }
                        menu
                    }),
                );
            } else if let Some(input) = self.inputs.get(&key) {
                let hint = if option.kind == "int" || option.kind == "double" {
                    format!("{}–{}", option.minimum, option.maximum)
                } else if option.kind == "color" {
                    "#RRGGBB / #RRGGBBAA".into()
                } else {
                    String::new()
                };
                if option.kind == "color" {
                    if let Some(picker) = self.pickers.get(&key) {
                        // gpui-component's picker: its popover, palette and
                        // sliders inherit the workspace theme instead of the
                        // native macOS colour panel.
                        control = control.child(
                            ColorPicker::new(picker)
                                .anchor(Anchor::TopRight)
                                .accessibility_label(option.label.clone()),
                        );
                    }
                }
                control = control.child(
                    Input::new(input)
                        .disabled(self.loading)
                        .w(px(if option.kind == "color" {
                            230.
                        } else if option.directory {
                            205.
                        } else {
                            280.
                        }))
                        .aria_label(option.label.clone()),
                );
                if option.directory {
                    control = control.child(
                        Button::new(format!("preference-browse-{key}"))
                            .icon(IconName::FolderOpen)
                            .rounded(gpui_kit::px(999.))
                            .small()
                            .px(gpui_kit::px(13.))
                            .py(gpui_kit::px(3.))
                            .label("选择…")
                            .disabled(self.loading)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.pick_preference_directory(key.clone(), cx)
                            })),
                    );
                }
                if !hint.is_empty() {
                    description = description.child(
                        div()
                            .text_xs()
                            .text_color(Theme::global(cx).muted_foreground)
                            .child(hint),
                    );
                }
            }
            options = options.child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_4()
                    .px_3()
                    .py_3()
                    .border_b_1()
                    .border_color(Theme::global(cx).border)
                    .child(description)
                    .child(control),
            );
        }

        let reset_names: Vec<_> = visible.iter().map(|option| option.name.clone()).collect();
        let mut toolbar = div()
            .h_flex()
            .flex_wrap()
            .gap_2()
            .items_center()
            .child(
                Button::new("preferences-apply")
                    .icon(IconName::Check)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .label(if self.drafts.is_empty() {
                        "应用修改".to_owned()
                    } else {
                        format!("应用 {} 项修改", self.drafts.len())
                    })
                    .primary()
                    .disabled(self.loading || self.drafts.is_empty())
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.preferences_request(
                            json!({"action":"apply","fields":view.preferences.drafts}),
                            cx,
                        )
                    })),
            )
            .child(
                // The legacy config is just config.json; there is no export
                // command upstream, so importing means reading that file.
                Button::new("preferences-import")
                    .child(Icon::new(IconName::FileInput).size(px(15.)))
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .label("导入旧版配置…")
                    .tooltip("读取旧版 Aegisub 的 config.json，覆盖同名设置")
                    .disabled(self.loading)
                    .on_click(cx.listener(|view, _, _, cx| view.import_config_file(cx))),
            )
            .child(
                Button::new("preferences-discard")
                    .icon(IconName::Undo2)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .label("放弃未应用修改")
                    .disabled(self.loading || self.drafts.is_empty())
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.preferences.drafts.clear();
                        view.preferences.error = None;
                        view.preferences.confirm_reset = false;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("preferences-reset-visible")
                    .icon(IconName::RotateCw)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .label(if self.confirm_reset {
                        "确认恢复当前列表默认值"
                    } else {
                        "恢复当前列表默认值…"
                    })
                    .disabled(self.loading || reset_names.is_empty())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if view.preferences.confirm_reset {
                            view.preferences_request(
                                json!({"action":"reset","names":reset_names}),
                                cx,
                            );
                        } else {
                            view.preferences.confirm_reset = true;
                            cx.notify();
                        }
                    })),
            )
            .child(
                Button::new("preferences-hotkeys")
                    .icon(IconName::Settings2)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .label("快捷键…")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.active_panel = crate::Panel::Hotkeys;
                        view.hotkey_request(json!({"action":"state"}), cx);
                    })),
            )
            .child(div().ml_auto().child(Input::new(&self.search).w(px(280.))));
        if self.confirm_reset {
            toolbar = toolbar.child(
                Button::new("preferences-reset-cancel")
                    .icon(IconName::Close)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .label("取消恢复")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.preferences.confirm_reset = false;
                        cx.notify();
                    })),
            );
        }
        let mut panel = crate::page::page(cx, "preferences-panel")
            .child(
                div()
                    .h_flex()
                    .items_end()
                    .gap_3()
                    .child(div().flex_1().min_w_0().child(groups))
                    .child(crate::page::badge(cx, format!("{} 项", visible.len()))),
            )
            .child(toolbar);
        if self.loading {
            panel = panel.child("正在同步设置…");
        }
        if let Some(error) = &self.error {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(Theme::global(cx).danger)
                    .child(error.clone()),
            );
        }
        panel.child(options)
    }
}
