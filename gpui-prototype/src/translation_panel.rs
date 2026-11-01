use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    Icon, StyledExt,
    button::{Button, ButtonVariants},
    input::{InputEvent, Textarea, TextareaState},
};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, Subscription, Window, div, prelude::*, px,
};
use serde::Deserialize;
use serde_json::json;

use crate::Workspace;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct TranslationState {
    pub available: bool,
    #[serde(default)]
    pub exhausted: bool,
    #[serde(default)]
    pub boundary: bool,
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub row: usize,
    #[serde(default)]
    pub block: usize,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub line_text: String,
    #[serde(default)]
    pub position: usize,
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub line_count: usize,
}

pub struct TranslationPanel {
    pub state: TranslationState,
    pub editor: Entity<TextareaState>,
    pub loading: bool,
    pub seek: bool,
    pub skip_whitespace: bool,
    pub pending_text: Option<String>,
    pub focus_pending: bool,
    _events: Subscription,
}

impl TranslationPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let editor = cx.new(|cx| TextareaState::new(window, cx).submit_on_enter(true));
        let events = cx.subscribe(&editor, |view, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                view.translation_action("commit", cx);
            }
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            state: TranslationState::default(),
            editor,
            loading: false,
            seek: true,
            skip_whitespace: true,
            pending_text: None,
            focus_pending: false,
            _events: events,
        }
    }

    pub fn render(
        &self,
        video: bool,
        audio: bool,
        cx: &mut Context<Workspace>,
    ) -> impl IntoElement {
        // Two rows: every labelled action on the first, every icon-only action
        // below. Mixed into one wrapped row they read as an undifferentiated
        // strip and the icons looked like stray glyphs between the pills.
        let p = crate::workspace_theme::Palette::current(cx);
        let divider = p.divider;
        let separator = || div().w(px(1.)).h(px(18.)).bg(divider).into_any_element();
        let pill = |id: &'static str| {
            Button::new(id)
                .rounded(gpui_kit::px(999.))
                .small()
                .px(gpui_kit::px(13.))
                .py(gpui_kit::px(3.))
        };
        let icon_pill = |id: &'static str| {
            Button::new(id)
                .rounded(gpui_kit::px(999.))
                .small()
                .px(gpui_kit::px(11.))
                .py(gpui_kit::px(3.))
        };

        let status = if self.state.available {
            format!(
                "行 {} / {} · 文本块 {} / {}{}",
                self.state.row + 1,
                self.state.line_count,
                self.state.position + 1,
                self.state.count,
                if self.state.exhausted {
                    " · 已到最后一块"
                } else {
                    ""
                }
            )
        } else {
            "当前脚本没有可翻译的文本块".to_owned()
        };

        let mut labelled = vec![
            pill("translation-commit")
                .icon(IconName::Check)
                .label("提交并前进")
                .primary()
                .disabled(!self.state.available || self.loading)
                .on_click(cx.listener(|view, _, _, cx| view.translation_action("commit", cx)))
                .into_any_element(),
            pill("translation-preview")
                .icon(IconName::Eye)
                .label("提交并预览")
                .disabled(!self.state.available || self.loading)
                .on_click(cx.listener(|view, _, _, cx| view.translation_action("preview", cx)))
                .into_any_element(),
            separator(),
            pill("translation-original")
                .icon(IconName::FileInput)
                .label("插入原文")
                .tooltip("把整行原文插入到译文")
                .ghost()
                .disabled(!self.state.available || self.loading)
                .on_click(
                    cx.listener(|view, _, _, cx| view.translation_action("insert_original", cx)),
                )
                .into_any_element(),
            pill("translation-seek")
                .icon(IconName::Crosshair)
                .label("定位视频预览")
                .tooltip("切块时把视频定位到该块开始时间")
                .ghost()
                .when(self.seek, |button| button.primary())
                .on_click(cx.listener(|view, _, _, cx| {
                    view.translation.seek = !view.translation.seek;
                    cx.notify();
                }))
                .into_any_element(),
            pill("translation-skip-whitespace")
                .icon(IconName::SkipForward)
                .label("跳过空白块")
                .ghost()
                .when(self.skip_whitespace, |button| button.primary())
                .on_click(cx.listener(|view, _, _, cx| {
                    view.translation.skip_whitespace = !view.translation.skip_whitespace;
                    view.translation_action("start", cx);
                }))
                .into_any_element(),
            div().flex_1().into_any_element(),
            crate::page::badge(cx, status).into_any_element(),
        ];
        labelled.shrink_to_fit();

        let mut icons = vec![
            icon_pill("translation-prev")
                .child(Icon::new(IconName::ChevronLeft).size(px(15.)))
                .accessibility_label("上一块")
                .tooltip("上一块")
                .ghost()
                .disabled(!self.state.available || self.loading)
                .on_click(cx.listener(|view, _, _, cx| view.translation_action("prev", cx)))
                .into_any_element(),
            icon_pill("translation-next")
                .child(Icon::new(IconName::ChevronRight).size(px(15.)))
                .accessibility_label("下一块")
                .tooltip("下一块")
                .ghost()
                .disabled(!self.state.available || self.loading)
                .on_click(cx.listener(|view, _, _, cx| view.translation_action("next", cx)))
                .into_any_element(),
            separator(),
            icon_pill("translation-refresh")
                .child(Icon::new(IconName::RefreshCw).size(px(15.)))
                .accessibility_label("从当前行刷新")
                .tooltip("重新读取当前行与文本块")
                .ghost()
                .disabled(self.loading)
                .on_click(cx.listener(|view, _, _, cx| view.translation_action("start", cx)))
                .into_any_element(),
            separator(),
        ];
        for (id, icon, label, command, enabled) in [
            (
                "translation-video",
                IconName::Video,
                "播放当前行视频",
                "video/play/line",
                video,
            ),
            (
                "translation-audio",
                IconName::AudioLines,
                "播放当前行音频",
                "audio/play/selection",
                audio,
            ),
            (
                "translation-undo",
                IconName::Undo2,
                "撤销",
                "edit/undo",
                true,
            ),
        ] {
            icons.push(
                icon_pill(id)
                    .child(Icon::new(icon).size(px(15.)))
                    .accessibility_label(label)
                    .tooltip(label)
                    .ghost()
                    .disabled(!enabled)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.invoke(json!({"op":"command", "name":command}), cx)
                    }))
                    .into_any_element(),
            );
        }

        crate::page::page(cx, "translation-assistant")
            .child(
                div()
                    .v_flex()
                    .w_full()
                    .gap_2()
                    .child(crate::page::toolbar(labelled))
                    .child(crate::page::toolbar(icons)),
            )
            .child(crate::page::section(
                cx,
                "整行原文",
                None,
                crate::page::section_body()
                    .child(crate::page::hint(cx, "覆盖标签、注释块和绘图保持原样"))
                    .child(
                        div()
                            .p_3()
                            .rounded(px(crate::workspace_theme::RADIUS_MD))
                            .border_1()
                            .border_color(p.border)
                            .text_sm()
                            .child(self.state.line_text.clone()),
                    ),
            ))
            .child(crate::page::section(
                cx,
                "当前文本块",
                None,
                crate::page::section_body().child(
                    div()
                        .p_3()
                        .rounded(px(crate::workspace_theme::RADIUS_MD))
                        .bg(p.selected)
                        .text_color(p.selected_text)
                        .child(self.state.text.clone()),
                ),
            ))
            .child(crate::page::section(
                cx,
                "译文",
                None,
                crate::page::section_body()
                    .child(Textarea::new(&self.editor).h(px(150.)).w_full())
                    .child(crate::page::hint(
                        cx,
                        "Enter 提交并前进，Shift+Enter 换行；F8 预览、PageUp/PageDown 切块。切块会清空当前译文；刷新可重新读取外部修改。",
                    )),
            ))
    }
}
