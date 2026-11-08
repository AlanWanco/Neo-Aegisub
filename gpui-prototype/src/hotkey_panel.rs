use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    StyledExt, Theme,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{
    App, AppContext, Context, Entity, IntoElement, Subscription, Window, div, prelude::*, px,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    Workspace,
    command_catalog::{Catalog, Hotkey},
};

#[derive(Deserialize)]
pub struct HotkeyState {
    pub bindings: Vec<Hotkey>,
    pub imported: Option<i64>,
    pub skipped: Option<i64>,
}

pub struct HotkeyPanel {
    pub context: Entity<InputState>,
    pub key: Entity<InputState>,
    pub command: Entity<InputState>,
    pub search: Entity<InputState>,
    pub bindings: Vec<Hotkey>,
    pub loading: bool,
    pub recording: bool,
    pub replace: bool,
    pub confirm_reset: bool,
    pub error: Option<String>,
    pub pending: Option<(String, String, String)>,
    pub pending_search: Option<String>,
    _events: Vec<Subscription>,
}

impl HotkeyPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let context = cx.new(|cx| InputState::new(window, cx).default_value("Default"));
        let key = cx.new(|cx| InputState::new(window, cx).placeholder("例如 Ctrl-Shift-K"));
        let command = cx.new(|cx| InputState::new(window, cx).placeholder("例如 video/frame/next"));
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("筛选快捷键、上下文或命令"));
        let events = [&context, &key, &command, &search]
            .into_iter()
            .map(|input| {
                cx.subscribe(input, |view, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        view.hotkeys.error = None;
                        view.hotkeys.replace = false;
                        view.hotkeys.confirm_reset = false;
                        cx.notify();
                    }
                })
            })
            .collect();
        Self {
            context,
            key,
            command,
            search,
            bindings: vec![],
            loading: false,
            recording: false,
            replace: false,
            confirm_reset: false,
            error: None,
            pending: None,
            pending_search: None,
            _events: events,
        }
    }

    pub fn sync(&mut self, window: &mut Window, cx: &mut App) {
        if let Some(query) = self.pending_search.take() {
            self.search
                .update(cx, |state, cx| state.set_value(query, window, cx));
        }
        if let Some((context, key, command)) = self.pending.take() {
            self.context
                .update(cx, |state, cx| state.set_value(context, window, cx));
            self.key
                .update(cx, |state, cx| state.set_value(key, window, cx));
            self.command
                .update(cx, |state, cx| state.set_value(command, window, cx));
        }
    }

    pub fn render(&self, catalog: &Catalog, cx: &mut Context<Workspace>) -> impl IntoElement {
        let mut form = div().h_flex().flex_wrap().gap_3();
        for (label, input) in [
            ("上下文", &self.context),
            ("快捷键 · macOS 的 Ctrl 对应 ⌘", &self.key),
            ("命令名称 · 可空白以屏蔽默认绑定", &self.command),
        ] {
            form = form.child(
                div()
                    .v_flex()
                    .gap_1()
                    .w(px(340.))
                    .child(div().text_sm().child(label))
                    .child(Input::new(input).disabled(self.loading)),
            );
        }
        let selected_context = self.context.read(cx).value().to_string();
        let selected_key = self.key.read(cx).value().to_string();
        let selected_command = self.command.read(cx).value().to_string();
        let conflicts: Vec<_> = self
            .bindings
            .iter()
            .filter(|binding| {
                binding.context == selected_context
                    && binding.key == selected_key
                    && binding.command != selected_command
            })
            .map(|binding| binding.command.clone())
            .collect();
        let mut actions = div().h_flex().flex_wrap().gap_2()
            .child(Button::new("hotkey-record").icon(IconName::SquareTerminal).rounded(gpui_kit::px(999.)).small().px(gpui_kit::px(13.)).py(gpui_kit::px(3.)).label(if self.recording { "按下要绑定的组合键，Esc 取消" } else { "录入组合键" }).disabled(self.loading).on_click(cx.listener(|view, _, window, cx| { view.hotkeys.recording = !view.hotkeys.recording; view.focus_handle.focus(window, cx); cx.notify(); })))
            .child(Button::new("hotkey-save").icon(IconName::Save).rounded(gpui_kit::px(999.)).small().px(gpui_kit::px(13.)).py(gpui_kit::px(3.)).label("保存绑定").primary().disabled(self.loading || selected_key.is_empty() || selected_context.is_empty()).on_click(cx.listener(|view, _, _, cx| {
                view.hotkey_request(json!({"action":"set","context":view.hotkeys.context.read(cx).value().to_string(),"key":view.hotkeys.key.read(cx).value().to_string(),"command":view.hotkeys.command.read(cx).value().to_string(),"replace":view.hotkeys.replace}), cx);
            })))
            .child(Button::new("hotkey-remove").icon(IconName::Delete).rounded(gpui_kit::px(999.)).small().px(gpui_kit::px(13.)).py(gpui_kit::px(3.)).label("移除此绑定").disabled(self.loading || !self.bindings.iter().any(|binding| binding.context == selected_context && binding.key == selected_key && binding.command == selected_command)).on_click(cx.listener(|view, _, _, cx| {
                view.hotkey_request(json!({"action":"remove","context":view.hotkeys.context.read(cx).value().to_string(),"key":view.hotkeys.key.read(cx).value().to_string(),"command":view.hotkeys.command.read(cx).value().to_string()}), cx);
            })))
            .child(Button::new("hotkey-replace").icon(IconName::Replace).rounded(gpui_kit::px(999.)).small().px(gpui_kit::px(13.)).py(gpui_kit::px(3.)).label("替换同上下文的冲突绑定").when(self.replace, |button| button.primary()).disabled(self.loading).on_click(cx.listener(|view, _, _, cx| { view.hotkeys.replace = !view.hotkeys.replace; cx.notify(); })))
            .child(Button::new("hotkey-reset").icon(IconName::RotateCw).rounded(gpui_kit::px(999.)).small().px(gpui_kit::px(13.)).py(gpui_kit::px(3.)).label(if self.confirm_reset { "确认恢复全部默认快捷键" } else { "恢复默认快捷键…" }).disabled(self.loading).on_click(cx.listener(|view, _, _, cx| {
                if view.hotkeys.confirm_reset { view.hotkey_request(json!({"action":"reset"}), cx); }
                else { view.hotkeys.confirm_reset = true; cx.notify(); }
            })))
            .child(Button::new("hotkey-import").icon(IconName::FileInput).rounded(gpui_kit::px(999.)).small().px(gpui_kit::px(13.)).py(gpui_kit::px(3.)).label("导入旧版快捷键…").disabled(self.loading).on_click(cx.listener(|view, _, _, cx| view.hotkey_import(cx))));
        if self.confirm_reset {
            actions = actions.child(
                Button::new("hotkey-cancel-reset")
                    .icon(IconName::Close)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .label("取消恢复")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.hotkeys.confirm_reset = false;
                        cx.notify();
                    })),
            );
        }
        let query = self.search.read(cx).value().to_lowercase();
        let column_header = div()
            .h_flex()
            .w_full()
            .gap_3()
            .px_2()
            .pb_2()
            .border_b_1()
            .border_color(crate::workspace_theme::Palette::current(cx).border)
            .child(
                div()
                    .w(px(150.))
                    .flex_shrink_0()
                    .child(crate::page::field_label(cx, "上下文")),
            )
            .child(
                div()
                    .w(px(150.))
                    .flex_shrink_0()
                    .child(crate::page::field_label(cx, "快捷键")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(crate::page::field_label(cx, "命令")),
            );
        // No inner scroll box: a nested scrollbar inside the page's own scroll
        // container never receives the wheel, so the tail of the list was
        // unreachable. Let the page scroll through all bindings instead.
        let mut list = div()
            .id("hotkey-binding-list")
            .v_flex()
            .w_full()
            .min_w_0()
            .gap_0();
        for (index, binding) in self.bindings.iter().enumerate() {
            if !query.is_empty()
                && !format!("{} {} {}", binding.context, binding.key, binding.command)
                    .to_lowercase()
                    .contains(&query)
            {
                continue;
            }
            let (context, key, command) = (
                binding.context.clone(),
                binding.key.clone(),
                binding.command.clone(),
            );
            let label = catalog
                .command(&command)
                .map(|command| command.label.clone())
                .unwrap_or_else(|| {
                    if command.is_empty() {
                        "屏蔽绑定".into()
                    } else {
                        "当前未注册的命令".into()
                    }
                });
            let p = crate::workspace_theme::Palette::current(cx);
            // A left-aligned row reads like a table; the previous centred
            // single-line label made the list look like a wall of text.
            list = list.child(
                Button::new(format!("hotkey-binding-{index}"))
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .ghost()
                    .w_full()
                    .h(px(34.))
                    .border_b_1()
                    .border_color(p.border)
                    .accessibility_label(format!(
                        "{} · {} · {}",
                        binding.context, binding.key, binding.command
                    ))
                    .disabled(self.loading)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.hotkeys.pending =
                            Some((context.clone(), key.clone(), command.clone()));
                        view.hotkeys.replace = false;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_3()
                            .items_center()
                            .child(
                                div()
                                    .w(px(150.))
                                    .flex_shrink_0()
                                    .text_left()
                                    .text_size(px(12.))
                                    .text_color(p.muted)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(binding.context.clone()),
                            )
                            .child(
                                div()
                                    .w(px(150.))
                                    .flex_shrink_0()
                                    .text_left()
                                    .text_size(px(12.))
                                    .text_color(p.accent)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(if binding.key.is_empty() {
                                        "—".to_owned()
                                    } else {
                                        binding.key.clone()
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_left()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(format!("{label} · {}", binding.command)),
                            ),
                    ),
            );
        }
        let mut panel = crate::page::page(cx, "hotkeys-panel")
            .child(crate::page::toolbar([
                Button::new("hotkeys-back").rounded(gpui_kit::px(999.)).small().px(gpui_kit::px(13.)).py(gpui_kit::px(3.))
                    .icon(IconName::ArrowLeft)
                    .label("返回设置")
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.active_panel = crate::Panel::Preferences;
                        view.hotkeys.recording = false;
                        cx.notify();
                    }))
                    .into_any_element(),
            ]))
            .child(crate::page::section(
                cx,
                "绑定",
                None,
                crate::page::section_body()
                    .child(form)
                    .child(actions)
                    .child(crate::page::hint(
                        cx,
                        "小键盘按键和主键区在窗口系统里是同一个键：录小键盘 1 会得到 1。要单独绑定小键盘，直接在组合键框里输入 KP_1、KP_Decimal、KP_Add、KP_Enter 这类名字。",
                    )),
            ))
            .child(div().text_sm().text_color(Theme::global(cx).muted_foreground).child("常用上下文：Default、Subtitle Grid、Subtitle Edit Box、Video、Audio、Translation Assistant、Styling Assistant、Always。"));
        if !conflicts.is_empty() {
            panel = panel.child(
                div()
                    .text_sm()
                    .child(format!("冲突命令：{}", conflicts.join("、"))),
            );
        }
        if let Some(error) = &self.error {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(Theme::global(cx).danger)
                    .child(error.clone()),
            );
        }
        let list_body = crate::page::section_body()
            .child(Input::new(&self.search).w(px(450.)))
            .child(column_header)
            .child(list);
        panel.child(crate::page::section(
            cx,
            "全部绑定",
            Some(crate::page::badge(cx, format!("{} 条", self.bindings.len())).into_any_element()),
            list_body,
        ))
    }
}
