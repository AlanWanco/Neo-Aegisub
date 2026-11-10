use gpui_kit::base::Disableable;
use gpui_kit::component::{
    Sizable, StyledExt,
    button::{Button, ButtonVariants},
    menu::{DropdownMenu, PopupMenuItem},
};
use gpui_kit::{Context, FocusHandle, IntoElement, Window, div, prelude::*, px};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{HostDialog, Workspace, workspace_theme::Palette};

#[derive(Clone, Deserialize)]
pub struct KanjiLine {
    id: i64,
    row: i64,
    plain: String,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
struct Selection {
    source: usize,
    dest: usize,
}

#[derive(Deserialize)]
struct Syllable {
    text: String,
    duration: i64,
}

#[derive(Deserialize)]
struct Group {
    source: String,
    dest: String,
    duration: i64,
}

#[derive(Deserialize)]
struct Match {
    selected_source: Vec<Syllable>,
    remaining_source: Vec<Syllable>,
    selected_dest: String,
    remaining_dest: String,
    selection: Selection,
    groups: Vec<Group>,
    output: String,
    complete: bool,
}

#[derive(Serialize)]
struct Entry {
    source_id: i64,
    dest_id: i64,
    groups: Vec<Selection>,
}

struct Step {
    source: usize,
    dest: usize,
    drafts: usize,
}

pub struct KanjiState {
    styles: Vec<String>,
    source_style: String,
    dest_style: String,
    source_lines: Vec<KanjiLine>,
    dest_lines: Vec<KanjiLine>,
    revision: i64,
    source: usize,
    dest: usize,
    groups: Vec<Selection>,
    matched: Option<Match>,
    drafts: Vec<Entry>,
    history: Vec<Step>,
    interpolate: bool,
    started: bool,
    busy: bool,
    error: Option<String>,
    focus: FocusHandle,
    focus_pending: bool,
}

impl KanjiState {
    fn new(cx: &mut Context<Workspace>) -> Self {
        Self {
            styles: vec![],
            source_style: String::new(),
            dest_style: String::new(),
            source_lines: vec![],
            dest_lines: vec![],
            revision: 0,
            source: 0,
            dest: 0,
            groups: vec![],
            matched: None,
            drafts: vec![],
            history: vec![],
            interpolate: false,
            started: false,
            busy: false,
            error: None,
            focus: cx.focus_handle(),
            focus_pending: true,
        }
    }

    fn pair(&self) -> Option<(i64, i64)> {
        Some((
            self.source_lines.get(self.source)?.id,
            self.dest_lines.get(self.dest)?.id,
        ))
    }

    pub fn can_apply(&self) -> bool {
        !self.busy && !self.drafts.is_empty()
    }
    pub fn busy(&self) -> bool {
        self.busy
    }
    pub fn summary(&self) -> String {
        format!("{} 行待应用", self.drafts.len())
    }

    fn step(&mut self, source: bool, dest: bool) {
        self.history.push(Step {
            source: self.source,
            dest: self.dest,
            drafts: self.drafts.len(),
        });
        self.source += usize::from(source);
        self.dest += usize::from(dest);
        self.groups.clear();
        self.matched = None;
        self.focus_pending = true;
    }
}

impl Workspace {
    pub(crate) fn open_kanji(&mut self, cx: &mut Context<Self>) {
        self.host_dialog = Some(HostDialog::Kanji(KanjiState::new(cx)));
        self.load_kanji_styles(cx);
    }

    fn load_kanji_styles(&mut self, cx: &mut Context<Self>) {
        let Some(HostDialog::Kanji(state)) = self.host_dialog.as_mut() else {
            return;
        };
        state.busy = true;
        let payload = json!({"op":"dialog", "action":"kanji_preview", "source":state.source_style, "dest":state.dest_style});
        self.kanji_request(
            payload,
            cx,
            |state, value| {
                state.styles = serde_json::from_value(value["styles"].clone())?;
                state.revision = value["revision"].as_i64().unwrap_or(0);
                if state.source_style.is_empty() {
                    state.source_style = state.styles.first().cloned().unwrap_or_default();
                    state.dest_style = state.styles.get(1).cloned().unwrap_or_default();
                    state.interpolate = value["interpolate"].as_bool().unwrap_or(false);
                } else {
                    state.source_lines = serde_json::from_value(value["source_lines"].clone())?;
                    state.dest_lines = serde_json::from_value(value["dest_lines"].clone())?;
                    if state.source_lines.is_empty() || state.dest_lines.is_empty() {
                        state.error = Some("所选样式没有可配对的非空字幕行".into());
                    } else {
                        state.started = true;
                    }
                }
                Ok(())
            },
            true,
        );
    }

    fn kanji_request(
        &mut self,
        payload: Value,
        cx: &mut Context<Self>,
        receive: impl FnOnce(&mut KanjiState, Value) -> anyhow::Result<()> + 'static,
        match_after: bool,
    ) {
        let bridge = self.bridge.clone();
        cx.spawn(async move |weak, cx| {
            let result = bridge.call(payload).await;
            let _ = weak.update(cx, |view, cx| {
                let Some(HostDialog::Kanji(state)) = view.host_dialog.as_mut() else {
                    return;
                };
                state.busy = false;
                match result.and_then(|value| receive(state, value)) {
                    Ok(()) => {
                        if match_after && state.started {
                            view.request_kanji_match(None, cx);
                        }
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn request_kanji_match(&mut self, selection: Option<Selection>, cx: &mut Context<Self>) {
        let Some(HostDialog::Kanji(state)) = self.host_dialog.as_mut() else {
            return;
        };
        let Some((source_id, dest_id)) = state.pair() else {
            state.matched = None;
            cx.notify();
            return;
        };
        if state.busy {
            return;
        }
        state.busy = true;
        state.error = None;
        let mut payload = json!({"op":"dialog", "action":"kanji_match", "revision":state.revision,
            "source_id":source_id, "dest_id":dest_id, "groups":state.groups,
            "auto":selection.is_none() && state.interpolate});
        if let Some(selection) = selection {
            payload["selection"] = json!(selection);
        }
        self.kanji_request(
            payload,
            cx,
            |state, value| {
                state.matched = Some(serde_json::from_value(value)?);
                Ok(())
            },
            false,
        );
    }

    fn kanji_action(&mut self, action: &'static str, cx: &mut Context<Self>) {
        let Some(HostDialog::Kanji(state)) = self.host_dialog.as_mut() else {
            return;
        };
        if state.busy {
            return;
        }
        state.error = None;
        state.focus_pending = true;
        let mut selection = None;
        match action {
            "start" => {
                if state.source_style.is_empty()
                    || state.dest_style.is_empty()
                    || state.source_style == state.dest_style
                {
                    state.error = Some("请选择不同的源样式和目标样式".into());
                    cx.notify();
                    return;
                }
                self.load_kanji_styles(cx);
                return;
            }
            "source-more" | "source-less" | "dest-more" | "dest-less" => {
                let Some(matched) = &state.matched else {
                    return;
                };
                let mut next = matched.selection;
                match action {
                    "source-more" if !matched.remaining_source.is_empty() => next.source += 1,
                    "source-less" => next.source = next.source.saturating_sub(1),
                    "dest-more" if !matched.remaining_dest.is_empty() => next.dest += 1,
                    "dest-less" => next.dest = next.dest.saturating_sub(1),
                    _ => return,
                }
                selection = Some(next);
            }
            "auto" => {
                state.interpolate = !state.interpolate;
                if !state.interpolate {
                    cx.notify();
                    return;
                }
            }
            "link" => {
                let Some(matched) = &state.matched else {
                    return;
                };
                if matched.complete {
                    self.kanji_action("accept", cx);
                    return;
                }
                if matched.selection.source == 0 && matched.selection.dest == 0 {
                    return;
                }
                state.groups.push(matched.selection);
            }
            "unlink" => {
                selection = state.groups.pop();
                if selection.is_none() {
                    return;
                }
            }
            "accept" => {
                let Some(matched) = &state.matched else {
                    return;
                };
                if !matched.complete {
                    state.error = Some("请先配完源音节与目标文字，再接受此行".into());
                    cx.notify();
                    return;
                }
                let Some((source_id, dest_id)) = state.pair() else {
                    return;
                };
                // Record navigation before adding the draft so Back removes exactly this accepted line.
                let groups = std::mem::take(&mut state.groups);
                state.step(true, true);
                state.drafts.push(Entry {
                    source_id,
                    dest_id,
                    groups,
                });
            }
            "skip-source" | "skip-dest" => {
                if state.pair().is_none() {
                    return;
                }
                state.step(action == "skip-source", action == "skip-dest");
            }
            "back" => {
                let Some(step) = state.history.pop() else {
                    return;
                };
                state.source = step.source;
                state.dest = step.dest;
                state.drafts.truncate(step.drafts);
                state.groups.clear();
                state.matched = None;
                state.focus_pending = true;
            }
            _ => return,
        }
        self.request_kanji_match(selection, cx);
    }

    pub(crate) fn apply_kanji(&mut self, cx: &mut Context<Self>) {
        let Some(HostDialog::Kanji(state)) = self.host_dialog.as_mut() else {
            return;
        };
        if !state.can_apply() {
            return;
        }
        state.busy = true;
        state.error = None;
        let payload = json!({"op":"dialog", "action":"kanji_apply", "revision":state.revision,
            "interpolate":state.interpolate, "entries":state.drafts});
        let bridge = self.bridge.clone();
        cx.spawn(async move |weak, cx| {
            let result = bridge.call(payload).await;
            let _ = weak.update(cx, |view, cx| {
                match result {
                    Ok(value) => {
                        view.host_dialog = None;
                        view.status = format!(
                            "已转移 {} 行卡拉 OK 音节，可一次撤销",
                            value["changed"].as_i64().unwrap_or(0)
                        );
                        view.refresh_document(cx);
                    }
                    Err(error) => {
                        if let Some(HostDialog::Kanji(state)) = view.host_dialog.as_mut() {
                            state.busy = false;
                            state.error = Some(error.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn handle_kanji_key(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(HostDialog::Kanji(state)) = self.host_dialog.as_ref() else {
            return;
        };
        if event.keystroke.key == "escape" {
            if !state.busy {
                self.close_host_dialog(cx);
                self.focus_handle.focus(window, cx);
            }
            cx.stop_propagation();
            return;
        }
        if !state.focus.is_focused(window) {
            return;
        }
        let action = match event.keystroke.key.as_str() {
            "up" => "source-more",
            "down" => "source-less",
            "right" => "dest-more",
            "left" => "dest-less",
            "enter" | "return" => {
                if state.started {
                    "link"
                } else {
                    "start"
                }
            }
            "backspace" => "unlink",
            _ => return,
        };
        self.kanji_action(action, cx);
        cx.stop_propagation();
    }
}

pub fn render(
    state: &mut KanjiState,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> gpui_kit::AnyElement {
    let p = Palette::current(cx);
    if state.focus_pending {
        state.focus.focus(window, cx);
        state.focus_pending = false;
    }
    let style_select =
        |id: &'static str, current: String, source: bool, cx: &mut Context<Workspace>| {
            let styles = state.styles.clone();
            let weak = cx.weak_entity();
            crate::page::select_trigger(
                cx,
                id,
                if current.is_empty() {
                    "选择样式".into()
                } else {
                    current
                },
                state.busy || state.started,
            )
            .flex_1()
            .min_w_0()
            .dropdown_menu(move |mut menu, _, _| {
                for style in &styles {
                    let style = style.clone();
                    let weak = weak.clone();
                    menu =
                        menu.item(PopupMenuItem::new(style.clone()).on_click(move |_, _, cx| {
                            let _ = weak.update(cx, |view, cx| {
                                if let Some(HostDialog::Kanji(state)) = view.host_dialog.as_mut() {
                                    if source {
                                        state.source_style = style.clone();
                                    } else {
                                        state.dest_style = style.clone();
                                    }
                                }
                                cx.notify();
                            });
                        }));
                }
                menu
            })
        };
    let mut body = div()
        .id("kanji-body")
        .track_focus(&state.focus)
        .v_flex()
        .gap_3()
        .child(
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(div().text_xs().child("源样式"))
                .child(style_select(
                    "kanji-source",
                    state.source_style.clone(),
                    true,
                    cx,
                ))
                .child(div().text_xs().child("目标样式"))
                .child(style_select(
                    "kanji-dest",
                    state.dest_style.clone(),
                    false,
                    cx,
                )),
        )
        .child(
            crate::page::toggle_chip(
                cx,
                "kanji-interpolate",
                "日语自动匹配",
                state.interpolate,
                state.busy,
            )
            .on_click(cx.listener(|view, _, _, cx| view.kanji_action("auto", cx))),
        );
    if !state.started {
        body = body.child(div().text_sm().text_color(p.muted).child("把罗马音音节配给汉字或假名。目标文本的原有标签将被替换为新的 \\k 标签，整行起止时间保持原值。"))
            .child(Button::new("kanji-start").label("开始配对").primary().small().disabled(state.busy)
                .on_click(cx.listener(|view, _, _, cx| view.kanji_action("start", cx))));
    } else {
        let progress = format!(
            "源 {}/{} 行 · 目标 {}/{} 行 · {} 行待应用",
            (state.source + 1).min(state.source_lines.len()),
            state.source_lines.len(),
            (state.dest + 1).min(state.dest_lines.len()),
            state.dest_lines.len(),
            state.drafts.len()
        );
        body = body.child(div().text_xs().text_color(p.muted).child(progress));
        let mut display = div()
            .id("kanji-matcher")
            .v_flex()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(if state.focus.is_focused(window) {
                p.accent
            } else {
                p.border
            })
            .bg(p.inset)
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    if let Some(HostDialog::Kanji(state)) = &view.host_dialog {
                        state.focus.focus(window, cx);
                    }
                }),
            );
        if let Some((source, dest)) = state
            .source_lines
            .get(state.source)
            .zip(state.dest_lines.get(state.dest))
        {
            display = display
                .child(div().text_xs().text_color(p.muted).child(format!(
                    "源第 {} 行：{}",
                    source.row + 1,
                    source.plain
                )))
                .child(div().text_xs().text_color(p.muted).child(format!(
                    "目标第 {} 行：{}",
                    dest.row + 1,
                    dest.plain
                )));
        }
        if let Some(matched) = &state.matched {
            for group in &matched.groups {
                display =
                    display.child(div().h_flex().gap_2().text_xs().text_color(p.muted).child(
                        format!(
                            "{} → {} · {} ms",
                            visible(&group.source),
                            visible(&group.dest),
                            group.duration
                        ),
                    ));
            }
            let source = matched
                .selected_source
                .iter()
                .map(|s| visible(&s.text))
                .collect::<Vec<_>>()
                .join(" | ");
            let remaining = matched
                .remaining_source
                .iter()
                .map(|s| visible(&s.text))
                .collect::<Vec<_>>()
                .join(" | ");
            let duration: i64 = matched.selected_source.iter().map(|s| s.duration).sum();
            display = display
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .flex_wrap()
                        .items_center()
                        .child(div().text_xs().child("音节"))
                        .child(
                            div()
                                .rounded(px(4.))
                                .px_2()
                                .py_1()
                                .bg(p.selected)
                                .text_color(p.selected_text)
                                .child(visible(&source)),
                        )
                        .child(div().text_sm().text_color(p.muted).child(remaining)),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .flex_wrap()
                        .items_center()
                        .child(div().text_xs().child("文字"))
                        .child(
                            div()
                                .rounded(px(4.))
                                .px_2()
                                .py_1()
                                .bg(p.selected)
                                .text_color(p.selected_text)
                                .child(visible(&matched.selected_dest)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(p.muted)
                                .child(matched.remaining_dest.clone()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted)
                        .child(format!("当前组 {} ms · ↑↓ 调整音节，←→ 调整文字", duration)),
                );
            if !matched.output.is_empty() {
                display = display.child(div().text_xs().child(format!("结果：{}", matched.output)));
            }
        } else {
            display = display.child(div().text_sm().child(if state.pair().is_none() {
                "已到所选样式末尾。可返回上一对，或应用已接受的行。"
            } else {
                "正在读取音节…"
            }));
        }
        body = body.child(display);
        let complete = state.matched.as_ref().is_some_and(|m| m.complete);
        let controls = [
            (
                "source-less",
                "音节 −",
                !state
                    .matched
                    .as_ref()
                    .is_some_and(|m| m.selection.source > 0),
            ),
            (
                "source-more",
                "音节 +",
                !state
                    .matched
                    .as_ref()
                    .is_some_and(|m| !m.remaining_source.is_empty()),
            ),
            (
                "dest-less",
                "文字 −",
                !state.matched.as_ref().is_some_and(|m| m.selection.dest > 0),
            ),
            (
                "dest-more",
                "文字 +",
                !state
                    .matched
                    .as_ref()
                    .is_some_and(|m| !m.remaining_dest.is_empty()),
            ),
            (
                "link",
                "配成一组 · Enter",
                state.matched.is_none() || complete,
            ),
            ("unlink", "撤回分组 · Backspace", state.groups.is_empty()),
            ("accept", "接受此行", !complete),
            ("skip-source", "跳过源行", state.pair().is_none()),
            ("skip-dest", "跳过目标行", state.pair().is_none()),
            ("back", "返回上一对", state.history.is_empty()),
        ];
        let mut buttons = div().h_flex().flex_wrap().gap_2();
        for (action, label, disabled) in controls {
            buttons = buttons.child(
                Button::new(action)
                    .label(label)
                    .small()
                    .disabled(state.busy || disabled)
                    .on_click(cx.listener(move |view, _, _, cx| view.kanji_action(action, cx))),
            );
        }
        body = body.child(buttons).child(
            div()
                .text_xs()
                .text_color(p.muted)
                .child("接受此行只保存草稿；应用时统一写入，取消丢弃本次配对。"),
        );
    }
    if state.busy {
        body = body.child(div().text_xs().text_color(p.muted).child("正在处理…"));
    }
    if let Some(error) = &state.error {
        body = body.child(div().text_sm().text_color(p.warning).child(error.clone()));
    }
    body.into_any_element()
}

fn visible(text: &str) -> String {
    if text.is_empty() {
        "∅".into()
    } else {
        text.to_owned()
    }
}
