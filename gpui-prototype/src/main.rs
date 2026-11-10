// Release builds are GUI-only: without this Windows opens a console window for
// the front-end that the host launches in the background.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod attachment_panel;
mod file_panel;
mod spelling_panel;
use file_panel::{FilePage, FilePanel, FileState};
use spelling_panel::{SpellState, SpellingPanel};
mod audio_panel;
use attachment_panel::{AttachmentPanel, AttachmentState};
mod automation_panel;
use automation_panel::{AutomationPanel, AutomationState};
mod colour_picker;
mod command_catalog;

use audio_panel::{AudioImage, AudioPanel, AudioState, TILE_INFLIGHT_LIMIT};
mod document_panels;
mod hotkey_panel;
mod paste_panel;
mod preferences_panel;
use hotkey_panel::{HotkeyPanel, HotkeyState};
use preferences_panel::{PreferenceState, PreferencesPanel};
mod resample_panel;
use paste_panel::{PastePanel, PasteReply};
mod search_panel;
use search_panel::{SearchPanel, SearchReply};
mod styling_panel;
use resample_panel::{ResamplePanel, ResampleState};
mod page;
mod timing_panel;
mod timing_processor_panel;
use timing_panel::{ShiftState, TimingPanel};
use timing_processor_panel::{ProcessorState, TimingProcessorPanel};
mod translation_panel;
use styling_panel::StylingPanel;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod mpv_surface;
mod video_settings_panel;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use mpv_surface::MpvSurface;
mod video_timeline;
use translation_panel::{TranslationPanel, TranslationState};
use video_settings_panel::{VideoPage, VideoSettings, VideoSettingsPanel};
mod kanji_panel;
mod script_dialog;
mod visual_panel;
mod workspace_theme;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::{ObjectFit, StyledImage};
use image::Frame as VideoImageFrame;
use script_dialog::{ScriptDialogRequest, ScriptDialogState};
use workspace_theme::Palette;

use visual_panel::{VisualPanel, VisualReply};

use video_timeline::VideoTimeline;

use document_panels::{DocumentDetails, DocumentPanels, LibraryDetails};

use command_catalog::{Catalog, populate_menu, subtitle_context_menu};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow};
use async_net::TcpStream;
use futures_lite::io::{AsyncReadExt, AsyncWriteExt};
use gpui_kit::{
    App, AppContext, Context, Entity, ExternalPaths, Focusable, IntoElement, Render, Subscription,
    Window, WindowBounds, WindowOptions, canvas, div, prelude::*, px, size, uniform_list,
};
use gpui_kit::{
    base::{Disableable, Selectable},
    component::{
        Icon, Root, StyledExt, Theme, ThemeMode, WindowExt,
        button::{Button, ButtonVariants},
        color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        menu::{ContextMenuExt, DropdownMenu, PopupMenuItem},
        scroll::ScrollableElement,
    },
    platform::application,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, PartialEq)]
struct RunCommand {
    name: String,
}

// Native menu bars cannot carry a closure, so the menu item identifies the
// catalog command and the app-level handler forwards it to the workspace.
impl gpui_kit::Action for RunCommand {
    fn boxed_clone(&self) -> Box<dyn gpui_kit::Action> {
        Box::new(self.clone())
    }

    fn partial_eq(&self, other: &dyn gpui_kit::Action) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn name(&self) -> &'static str {
        "aegisub::RunCommand"
    }

    fn name_for_type() -> &'static str
    where
        Self: Sized,
    {
        "aegisub::RunCommand"
    }

    fn build(value: Value) -> Result<Box<dyn gpui_kit::Action>>
    where
        Self: Sized,
    {
        Ok(Box::new(Self {
            name: value
                .get("name")
                .and_then(|name| name.as_str())
                .unwrap_or_default()
                .to_owned(),
        }))
    }
}

fn dropped_kind(path: &std::path::Path) -> Option<&'static str> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "ass" | "ssa" | "srt" | "vtt" | "sub" => Some("subtitle"),
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" | "mpg" | "mpeg" | "ts" => Some("video"),
        "wav" | "flac" | "mp3" | "ogg" | "m4a" | "aac" | "opus" | "wma" => Some("audio"),
        "txt" | "log" | "pass" | "stats" => Some("timecodes"),
        _ => None,
    }
}

fn menu_label(text: &str) -> String {
    let label = text.split('\t').next().unwrap_or(text).replace('&', "");
    let label = label.trim();
    match label {
        "File" => "文件",
        "Edit" => "编辑",
        "View" => "视图",
        "Subtitle" => "字幕",
        "Timing" => "时间",
        "Video" => "视频",
        "Audio" => "音频",
        "Automation" => "自动化",
        "Help" => "帮助",
        _ => label,
    }
    .to_owned()
}

fn native_menu_items(catalog: &Catalog, submenu: &str) -> Vec<gpui_kit::MenuItem> {
    let mut items = Vec::new();
    for entry in catalog.menu(submenu) {
        if !entry.submenu.is_empty() {
            let children = native_menu_items(catalog, &entry.submenu);
            if !children.is_empty() {
                items.push(gpui_kit::MenuItem::submenu(
                    gpui_kit::Menu::new(menu_label(&entry.text)).items(children),
                ));
            }
            continue;
        }
        if entry.command.is_empty() {
            if !matches!(items.last(), Some(gpui_kit::MenuItem::Separator)) {
                items.push(gpui_kit::MenuItem::separator());
            }
            continue;
        }
        let Some(command) = catalog.command(&entry.command) else {
            continue;
        };
        if !command.can_invoke() {
            continue;
        }
        let label = if entry.text.is_empty() {
            command.label.clone()
        } else {
            menu_label(&entry.text)
        };
        items.push(
            gpui_kit::MenuItem::action(
                label,
                RunCommand {
                    name: command.name.clone(),
                },
            )
            .checked(command.active),
        );
    }
    while matches!(items.last(), Some(gpui_kit::MenuItem::Separator)) {
        items.pop();
    }
    items
}

/// macOS puts the menu bar in the system bar rather than inside the window.
fn install_native_menus(catalog: &Catalog, cx: &mut App) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let mut menus = vec![gpui_kit::Menu::new("Neo Aegisub").items(vec![
        gpui_kit::MenuItem::action(
            "关于 Neo Aegisub",
            RunCommand {
                name: "app/about".into(),
            },
        ),
        gpui_kit::MenuItem::separator(),
        gpui_kit::MenuItem::action(
            "新建窗口",
            RunCommand {
                name: "app/new_window".into(),
            },
        ),
        gpui_kit::MenuItem::separator(),
        gpui_kit::MenuItem::action(
            "退出 Neo Aegisub",
            RunCommand {
                name: "app/exit".into(),
            },
        ),
    ])];
    for entry in catalog.menu("main") {
        if entry.submenu.is_empty()
            || entry.submenu == "main/window"
            || entry.special == "automation"
        {
            continue;
        }
        let items = native_menu_items(catalog, &entry.submenu);
        if items.is_empty() {
            continue;
        }
        menus.push(gpui_kit::Menu::new(menu_label(&entry.text)).items(items));
    }
    cx.set_menus(menus);
}

#[derive(Clone, Debug, Deserialize)]
struct Line {
    id: i64,
    row: i64,
    start: i64,
    end: i64,
    #[serde(default)]
    start_frame: i64,
    #[serde(default)]
    end_frame: i64,
    layer: i64,
    comment: bool,
    style: String,
    actor: String,
    #[serde(default)]
    effect: String,
    #[serde(default)]
    margin: [i64; 3],
    text: String,
    #[serde(default)]
    display_text: String,
}

/// Space between subtitle list columns. Rows and the header both use it, so a
/// column boundary lands on the same x in both.
const COLUMN_GAP: f32 = 12.;

/// The subtitle text column never shrinks below this, so adding optional
/// columns squeezes the text last instead of making it disappear.
const MIN_TEXT_COLUMN: f32 = 140.;

/// One column of the subtitle list. The text column is implicit and always
/// last, so it keeps absorbing whatever width the fixed columns leave over.
/// Which part of the workspace owns the keyboard, so shortcuts stay
/// context-sensitive the way Aegisub's hotkey contexts are.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyFocus {
    Grid,
    Audio,
    Video,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, Deserialize)]
enum SubtitleColumn {
    IdLayer,
    Time,
    StyleActor,
    Layer,
    Effect,
    Actor,
    Comment,
}

impl SubtitleColumn {
    fn label(self) -> &'static str {
        match self {
            Self::IdLayer => "行 / 层",
            Self::Time => "开始 → 结束",
            Self::StyleActor => "样式 / 角色",
            Self::Layer => "图层",
            Self::Effect => "效果",
            Self::Actor => "演员",
            Self::Comment => "类型",
        }
    }

    fn default_width(self, secondary: f32) -> f32 {
        match self {
            Self::IdLayer => (secondary * 4.2).clamp(46., 70.),
            Self::Time => (secondary * 15.5).clamp(170., 245.),
            Self::StyleActor => (secondary * 12.6).clamp(150., 235.),
            Self::Layer => 52.,
            Self::Effect => (secondary * 9.).clamp(90., 160.),
            Self::Actor => (secondary * 7.5).clamp(80., 140.),
            Self::Comment => 56.,
        }
    }

    /// Floor used when the columns have to be squeezed to fit the panel.
    fn min_width(self) -> f32 {
        match self {
            Self::IdLayer => 44.,
            Self::Time => 172.,
            Self::StyleActor => 104.,
            Self::Layer => 32.,
            Self::Effect => 52.,
            Self::Actor => 52.,
            Self::Comment => 40.,
        }
    }

    /// Columns the user can add from the header menu.
    fn optional() -> [Self; 4] {
        [Self::Layer, Self::Effect, Self::Actor, Self::Comment]
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Appearance {
    #[serde(default)]
    colors: BTreeMap<String, u32>,
    #[serde(default)]
    default_colors: Vec<String>,
    #[serde(default)]
    grid_font: String,
    #[serde(default)]
    grid_size: i64,
    #[serde(default)]
    edit_font: String,
    #[serde(default)]
    edit_size: i64,
    #[serde(default)]
    soft_line_break: bool,
    #[serde(default)]
    highlight_visible: bool,
    #[serde(default)]
    focus_grid: bool,
    #[serde(default)]
    grid_widths: String,
    #[serde(default)]
    seek_on_double_click: bool,
    #[serde(default)]
    loop_playback: bool,
}

impl Appearance {
    fn color(&self, name: &str, fallback: gpui_kit::Hsla) -> gpui_kit::Hsla {
        if self.default_colors.iter().any(|key| key == name) {
            return fallback;
        }
        self.colors
            .get(name)
            .map(|color| gpui_kit::rgba(*color).into())
            .unwrap_or(fallback)
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct PlaybackSnapshot {
    #[serde(default)]
    video: bool,
    #[serde(default)]
    playing: bool,
    #[serde(default)]
    frame: i64,
    #[serde(default)]
    video_epoch: u64,
    #[serde(default)]
    video_time: i64,
    #[serde(default)]
    audio_playing: bool,
    #[serde(default)]
    audio_position: i64,
}

/// Progress of a long host task (indexing, exporting, ...). The host used to
/// draw this with a wx dialog; while the GPUI front end is attached it reports
/// the state here instead and we draw the card.
#[derive(Clone, Debug, Default, Deserialize)]
struct Progress {
    #[serde(default)]
    title: String,
    #[serde(default)]
    message: String,
    /// 0..1, or negative while the task cannot estimate its own progress.
    #[serde(default)]
    fraction: f64,
    #[serde(default)]
    cancellable: bool,
}

/// A warning the host could not show itself because it has no wx window.
#[derive(Clone, Debug, Deserialize)]
struct Notice {
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
    /// Preference that silences this warning; empty for one-off notices.
    #[serde(default)]
    suppress: String,
    /// Host-selected button row; "unsaved" asks the user to resolve edits.
    #[serde(default)]
    kind: String,
    /// Answers for a question notice; the host gets the index back.
    #[serde(default)]
    choices: Vec<String>,
}

/// Shared host data presented by the GPUI dialogs.
struct LogEntry {
    section: String,
    message: String,
    severity: String,
}

impl LogEntry {
    fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || self.message.to_lowercase().contains(query)
            || self.section.to_lowercase().contains(query)
    }
}

struct LanguageItem {
    code: String,
    name: String,
}

struct FontState {
    face: String,
    size: i64,
    bold: bool,
    italic: bool,
    underline: bool,
}

const FORK_RELEASES_URL: &str = "https://github.com/AlanWanco/Neo-Aegisub/releases";

#[derive(Default, Deserialize)]
struct UpdateState {
    enabled: bool,
    checking: bool,
    current_version: String,
    source: String,
    message: String,
    releases: Vec<UpdateRelease>,
}

#[derive(Deserialize)]
struct UpdateRelease {
    name: String,
    url: String,
    description: String,
}

enum HostDialog {
    Updates(UpdateState),
    About {
        version: String,
        long_version: String,
        build_time: String,
        credit: String,
        official: bool,
    },
    Log {
        entries: Vec<LogEntry>,
    },
    Language {
        items: Vec<LanguageItem>,
        active: String,
        restart: bool,
        note: Option<String>,
    },
    Font {
        faces: Vec<String>,
        state: FontState,
        error: Option<String>,
    },
    Export {
        formats: Vec<ExportFormat>,
        encodings: Vec<String>,
        encoding: String,
        busy: bool,
    },
    Fonts {
        fonts: Vec<FontEntry>,
        busy: bool,
    },
    Kanji(kanji_panel::KanjiState),
}

#[derive(Clone, Debug)]
struct ExportFormat {
    name: String,
    description: String,
    checked: bool,
}

#[derive(Clone, Debug)]
struct FontEntry {
    name: String,
    size: i64,
}

/// The override colour dialog. The host owns the per-line override tags; this
/// only picks the colour and hands it back.
struct ColorDialog {
    title: &'static str,
    hint: &'static str,
    /// The colour the picker still has to receive; it can only be set while a
    /// window is at hand, which is render time.
    pending: Option<gpui_kit::Hsla>,
    error: Option<String>,
    loading: bool,
    /// Recently applied colours, newest first. The host keeps the list in the
    /// same option the wx picker uses, so both dialogs share one history.
    recent: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct Snapshot {
    #[serde(default)]
    file: String,
    #[serde(default)]
    modified: bool,
    #[serde(default)]
    can_save: bool,
    #[serde(default = "default_true")]
    dark: bool,
    #[serde(default = "default_true")]
    show_toolbar: bool,
    #[serde(default)]
    appearance: Appearance,
    #[serde(default)]
    revision: u64,
    #[serde(default)]
    details_revision: u64,
    #[serde(default)]
    video: bool,
    #[serde(default)]
    video_path: String,
    #[serde(default)]
    timecodes: bool,
    #[serde(default)]
    playing: bool,
    #[serde(default)]
    audio: AudioState,
    #[serde(default)]
    frame: i64,
    #[serde(default)]
    frame_count: i64,
    #[serde(default)]
    video_keyframes: Arc<[i64]>,
    #[serde(default)]
    video_time: i64,
    #[serde(default)]
    video_duration: i64,
    #[serde(default)]
    playback_rate: f64,
    #[serde(default)]
    aspect: f64,
    #[serde(default)]
    video_epoch: u64,
    #[serde(default)]
    video_overscan: bool,
    #[serde(default)]
    visual_autohide: bool,
    fps: Option<f64>,
    #[serde(default = "no_active_line")]
    active: i64,
    #[serde(default)]
    selected: Vec<i64>,
    /// Style names in the script, for the line property dropdown.
    #[serde(default)]
    styles: Vec<String>,
    #[serde(default)]
    text_selection: [usize; 2],
    #[serde(default)]
    lines: Vec<Line>,
    #[serde(default)]
    notices: Vec<Notice>,
    #[serde(default)]
    progress: Option<Progress>,
    /// Commands the host wants this front end to run, because it owns their UI.
    #[serde(default)]
    commands: Vec<String>,
    /// The dialog the host is waiting on; repeated in every snapshot until answered.
    #[serde(default)]
    script_dialog: Option<ScriptDialogRequest>,
}

/// Status bar hints per hotkey context: (context, label, [(command, text)]).
const SHORTCUT_HINTS: &[(&str, &str, &[(&str, &str)])] = &[
    (
        "Subtitle Grid",
        "字幕列表",
        &[
            ("video/frame/prev", "上一帧"),
            ("video/frame/next", "下一帧"),
            ("subtitle/select/all", "全选"),
        ],
    ),
    (
        "Subtitle Edit Box",
        "文本编辑",
        &[
            ("grid/line/next/create", "提交"),
            ("edit/color/primary", "主色"),
            ("edit/color/secondary", "次色"),
            ("edit/color/outline", "描边色"),
        ],
    ),
    (
        "Audio",
        "音频",
        &[
            ("audio/commit", "提交"),
            ("audio/play/selection", "播放选区"),
            ("audio/play/selection/before", "听前文"),
            ("audio/play/selection/after", "听后文"),
            ("time/prev", "上一句"),
            ("time/next", "下一句"),
        ],
    ),
    (
        "Video",
        "视频",
        &[
            ("video/frame/prev", "上一帧"),
            ("video/frame/next", "下一帧"),
            ("video/tool/drag", "拖动"),
            ("video/tool/clip", "裁剪"),
        ],
    ),
];

fn pretty_key(key: &str) -> String {
    let mut out = key.to_owned();
    if cfg!(target_os = "macos") {
        out = out.replace("Ctrl-", "⌘").replace("Alt-", "⌥");
    }
    out = out
        .replace("KP_Enter", "小键盘 ⏎")
        .replace("Enter", "⏎")
        .replace("Space", "空格")
        .replace("Left", "←")
        .replace("Right", "→")
        .replace("Up", "↑")
        .replace("Down", "↓");
    out
}

/// Style, actor, layer and effect sit under the edit box, where Aegisub puts
/// them: they describe the line being typed, not its timing.
fn line_field_row(
    cx: &mut Context<Workspace>,
    view: &Workspace,
    p: &crate::workspace_theme::Palette,
) -> gpui_kit::Div {
    let mut row = div().h_flex().items_center().gap_2().w_full().min_w_0();
    // The time fields only ever hold "0:00:00.00", so they keep a fixed width
    // (label + box) and the free space goes to the actor field instead.
    for (key, label, grow, basis) in [
        ("style", "样式", 1.4, 200.),
        ("actor", "角色", 1.0, 90.),
        ("start", "始", 0., 118.),
        ("end", "终", 0., 118.),
    ] {
        row = row.child(
            div()
                .h_flex()
                .items_center()
                .gap_1p5()
                .flex_basis(gpui_kit::px(basis))
                .flex_grow(grow)
                .flex_shrink(if grow > 0. { 1. } else { 0. })
                .min_w_0()
                // The label rides beside the field, not above it: a stacked label
                // cost a whole text line per row and doubled the panel height.
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(label),
                )
                .child(div().flex_1().min_w_0().child(if key == "style" {
                    // Style names come from the script, so a text box only ever
                    // produced typos: pick one of the styles that exist.
                    let options = view.snapshot.styles.clone();
                    let current = view.property_inputs[key].read(cx).value().to_string();
                    let weak = cx.weak_entity();
                    let disabled = view.selected_line < 0;
                    let label = current.clone();
                    page::select_trigger(cx, "editor-style-select", label, disabled)
                        .h(px(32.))
                        .accessibility_label("样式")
                        .dropdown_menu(move |mut menu, _, _| {
                            for name in options.iter() {
                                let choice = name.clone();
                                let target = weak.clone();
                                menu = menu.item(PopupMenuItem::new(choice.clone()).on_click(
                                    move |_, window, cx| {
                                        let choice = choice.clone();
                                        let _ = target.update(cx, |view, cx| {
                                            if let Some(input) = view.property_inputs.get("style") {
                                                input.update(cx, |state, cx| {
                                                    state.set_value(choice.clone(), window, cx);
                                                });
                                            }
                                            view.properties_dirty = true;
                                            view.commit_properties(cx);
                                            cx.notify();
                                        });
                                    },
                                ));
                            }
                            menu.scrollable(true)
                        })
                        .into_any_element()
                } else {
                    Input::new(&view.property_inputs[key])
                        .id(format!("editor-{key}"))
                        .aria_label(label)
                        .h(px(32.))
                        .w_full()
                        .into_any_element()
                })),
        );
    }
    // No apply button here: the panel's own button commits the same fields, and
    // two checkmarks for one action is just a second way to be wrong.
    row
}

/// Karaoke timing tags: `\k`, `\kf`, `\ko` in either case.
fn has_karaoke_tag(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes
        .windows(2)
        .any(|pair| pair[0] == b'\\' && (pair[1] == b'k' || pair[1] == b'K'))
}

/// Tags that make a line move, animate, fade or draw: `\t`, `\move`, `\pos`,
/// `\clip`, `\fad`, `\org` and the `\p` drawing mode.
fn has_effect_tag(text: &str) -> bool {
    const TAGS: [&str; 9] = [
        "\\t(", "\\t (", "\\move(", "\\pos(", "\\clip(", "\\iclip(", "\\fad(", "\\fade(", "\\org(",
    ];
    TAGS.iter().any(|tag| text.contains(tag)) || text.contains("\\p1") || text.contains("\\p2")
}

/// A 16px rounded chip holding one 11px icon.
fn tag_text_badge(text: &'static str, color: gpui_kit::Hsla) -> gpui_kit::Div {
    div()
        .h(px(16.))
        .px(px(3.))
        .rounded(px(3.))
        .h_flex()
        .items_center()
        .bg(color.opacity(0.16))
        .child(
            div()
                .text_size(px(9.))
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(color)
                .child(text),
        )
}

fn tag_badge(icon: gpui_kit::assets::IconName, color: gpui_kit::Hsla) -> gpui_kit::Div {
    div()
        .h_flex()
        .items_center()
        .justify_center()
        .w(px(16.))
        .h(px(16.))
        .flex_shrink_0()
        .rounded(px(3.))
        .bg(color.opacity(0.16))
        .child(Icon::new(icon).size(px(11.)).text_color(color))
}
/// A packed RGB triple for mpv's `background-color`, which carries no alpha.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn rgb_hex(color: gpui_kit::Hsla) -> u32 {
    let rgb = color.to_rgb();
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u32;
    channel(rgb.r) << 16 | channel(rgb.g) << 8 | channel(rgb.b)
}

/// The rect a fitted picture covers inside `area`: the video's aspect at the
/// largest scale that fits, centred. Both renderers paint into this one rect
/// (the host frame draws the image there, mpv's window is placed there), so
/// handing the preview from one renderer to the other cannot move it.
fn fitted_picture(
    area: gpui_kit::Bounds<gpui_kit::Pixels>,
    aspect: f32,
) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    let width = f32::from(area.size.width).max(1.);
    let height = f32::from(area.size.height).max(1.);
    let aspect = if aspect.is_finite() && aspect > 0. {
        aspect
    } else {
        16. / 9.
    };
    let (w, h) = if width / height > aspect {
        (height * aspect, height)
    } else {
        (width, width / aspect)
    };
    gpui_kit::Bounds {
        origin: gpui_kit::point(
            area.origin.x + px((width - w) / 2.),
            area.origin.y + px((height - h) / 2.),
        ),
        size: size(px(w), px(h)),
    }
}

fn escape_line_breaks(text: String, soft: bool) -> String {
    if !text.contains('\n') {
        return text;
    }
    text.replace('\n', if soft { "\\n" } else { "\\N" })
}

/// Whether a row survives the search box. Shared by the list and by the
/// selection follower, which has to translate the active line's id into the
/// same index the filtered list is showing.
fn matches_subtitle_filter(line: &Line, lower_filter: &str) -> bool {
    lower_filter.is_empty()
        || line.text.to_lowercase().contains(lower_filter)
        || line.actor.to_lowercase().contains(lower_filter)
        || line.style.to_lowercase().contains(lower_filter)
}

fn hotkey_key(key: &str) -> String {
    match key {
        "backspace" => "Backspace".into(),
        "delete" => "Delete".into(),
        "enter" => "Enter".into(),
        "escape" => "Escape".into(),
        "space" => "Space".into(),
        "tab" => "Tab".into(),
        "left" => "Left".into(),
        "right" => "Right".into(),
        "up" => "Up".into(),
        "down" => "Down".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "PageUp".into(),
        "pagedown" => "PageDown".into(),
        "insert" => "Insert".into(),
        "minus" => "-".into(),
        "equal" => "=".into(),
        "comma" => ",".into(),
        "period" => ".".into(),
        "slash" => "/".into(),
        "backslash" => "\\".into(),
        "semicolon" => ";".into(),
        "quote" => "'".into(),
        "leftbracket" => "[".into(),
        "rightbracket" => "]".into(),
        "grave" => "`".into(),
        _ => key.to_uppercase(),
    }
}

/// Aegisub spells keypad keys `KP_*`, but gpui reports the character the key
/// types, so a keypad 1 and a main-row 1 are the same keystroke. Binding a
/// keypad key therefore records the plain name; looking one up still has to try
/// the keypad spelling, or every existing `KP_*` binding stays dead.
fn keypad_name(key: &str) -> Option<&'static str> {
    Some(match key {
        "0" => "KP_0",
        "1" => "KP_1",
        "2" => "KP_2",
        "3" => "KP_3",
        "4" => "KP_4",
        "5" => "KP_5",
        "6" => "KP_6",
        "7" => "KP_7",
        "8" => "KP_8",
        "9" => "KP_9",
        "." => "KP_Decimal",
        "+" => "KP_Add",
        "-" => "KP_Subtract",
        "*" => "KP_Multiply",
        "/" => "KP_Divide",
        "enter" => "KP_Enter",
        _ => return None,
    })
}

fn default_true() -> bool {
    true
}

fn no_active_line() -> i64 {
    -1
}

#[derive(serde::Deserialize)]
struct StylePreviewReply {
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// The newest video frame, held as a GPU texture, plus the picture's own zoom
/// and pan. The host opens no video window: it sends frames as raw BGRA through
/// `video_frame` and the workspace paints them.
struct VideoView {
    image: Option<std::sync::Arc<gpui_kit::RenderImage>>,
    /// Host serial of `image`; sent back so an unchanged frame carries no pixels.
    serial: i64,
    /// Frame number `image` shows, -1 when there is none.
    frame: i64,
    inflight: bool,
    /// Retries left while the host has not yet produced the frame being shown.
    retries: u8,
    /// Content zoom: 1 fits the whole frame in the picture area.
    zoom: f32,
    /// Native pixel size of the current frame, used to report source-relative zoom.
    source_size: Option<(u32, u32)>,
    /// Pan as a fraction of the picture area, so it survives resizing.
    pan: (f32, f32),
    /// Pointer position, pan, and pressed button at the start of a drag.
    pan_drag: Option<(gpui_kit::MouseButton, f32, f32, (f32, f32))>,
    /// Last pointer position inside the picture area, in window pixels.
    pointer: Option<(f32, f32)>,
    /// Painted size of the picture area, read by the pan gesture.
    bounds: std::rc::Rc<std::cell::RefCell<gpui_kit::Bounds<gpui_kit::Pixels>>>,
}

impl Default for VideoView {
    fn default() -> Self {
        Self {
            image: None,
            serial: -1,
            frame: -1,
            inflight: false,
            retries: 0,
            zoom: 1.,
            source_size: None,
            pan: (0., 0.),
            pan_drag: None,
            pointer: None,
            bounds: Default::default(),
        }
    }
}

/// Header of a raw preview `video_frame` reply. The pixels follow as `width * height * 4`
/// raw BGRA8 bytes, top row first; an empty body means no new frame.
#[derive(Deserialize)]
struct VideoFrameHead {
    serial: i64,
    frame: i64,
    width: u32,
    height: u32,
    pixel_format: String,
    #[serde(default)]
    unchanged: bool,
}

#[derive(Clone)]
struct Bridge {
    endpoint: String,
    token: String,
}

const MAX_AUDIO_HEADER_BYTES: usize = 64 * 1024;
/// Largest frame body accepted from the host: 8192 x 8192 BGRA8.
const MAX_VIDEO_FRAME_BYTES: usize = 8192 * 8192 * 4;

impl Bridge {
    /// Opens a connection and sends one authenticated request line.
    async fn send(&self, body: Value) -> Result<TcpStream> {
        let address: SocketAddr = self
            .endpoint
            .parse()
            .context("invalid local bridge endpoint")?;
        let mut stream = TcpStream::connect(address)
            .await
            .context("Neo Aegisub bridge is unavailable")?;
        let mut request = body;
        request["token"] = Value::String(self.token.clone());
        let mut bytes = serde_json::to_vec(&request)?;
        bytes.push(b'\n');
        stream
            .write_all(&bytes)
            .await
            .context("could not write to Neo Aegisub bridge")?;
        Ok(stream)
    }

    async fn call(&self, body: Value) -> Result<Value> {
        let mut stream = self.send(body).await?;
        // The host answers with one newline-terminated JSON line and closes the
        // socket afterwards. Reading to EOF makes every request wait for the
        // close handshake as well, which costs milliseconds per playback frame.
        let mut response = Vec::new();
        let mut chunk = [0u8; 65536];
        loop {
            let read = stream
                .read(&mut chunk)
                .await
                .context("could not read Neo Aegisub bridge response")?;
            if read == 0 {
                break;
            }
            response.extend_from_slice(&chunk[..read]);
            if response.last() == Some(&b'\n') {
                break;
            }
        }
        let value: Value =
            serde_json::from_slice(&response).context("invalid Neo Aegisub bridge response")?;
        if value.get("ok").and_then(Value::as_bool) == Some(false) {
            return Err(anyhow!(
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Neo Aegisub command failed")
                    .to_owned()
            ));
        }
        Ok(value)
    }

    /// Sends a request answered by one JSON header line followed by exactly
    /// `byte_length` raw bytes. The header is read up to its newline and the
    /// body by its declared length, since the pixels may contain newline bytes.
    /// An `ok:false` reply carries no body.
    async fn raw_call(&self, body: Value, max_body: usize) -> Result<(Value, Vec<u8>)> {
        let mut stream = self.send(body).await?;
        let mut received = Vec::new();
        let mut chunk = [0u8; 65536];
        let header_end = loop {
            if let Some(position) = received.iter().position(|byte| *byte == b'\n') {
                break position;
            }
            if received.len() > MAX_AUDIO_HEADER_BYTES {
                return Err(anyhow!(
                    "bridge reply header exceeds {MAX_AUDIO_HEADER_BYTES} bytes"
                ));
            }
            let read = stream
                .read(&mut chunk)
                .await
                .context("could not read Neo Aegisub bridge response")?;
            if read == 0 {
                return Err(anyhow!(
                    "Neo Aegisub bridge closed before the reply header ended"
                ));
            }
            received.extend_from_slice(&chunk[..read]);
        };
        let header: Value = serde_json::from_slice(&received[..header_end])
            .context("invalid Neo Aegisub bridge response")?;
        if header.get("ok").and_then(Value::as_bool) == Some(false) {
            return Err(anyhow!(
                header
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Neo Aegisub command failed")
                    .to_owned()
            ));
        }
        let length = header
            .get("byte_length")
            .and_then(Value::as_u64)
            .context("bridge reply has no byte_length")? as usize;
        if length > max_body {
            return Err(anyhow!(
                "bridge reply body of {length} bytes exceeds {max_body}"
            ));
        }
        // Bytes that arrived with the header are the start of the body.
        let mut body = received.split_off(header_end + 1);
        if body.len() > length {
            return Err(anyhow!(
                "bridge sent {} bytes after its header, expected {length}",
                body.len()
            ));
        }
        let have = body.len();
        body.resize(length, 0);
        stream
            .read_exact(&mut body[have..])
            .await
            .context("Neo Aegisub bridge closed before the body ended")?;
        Ok((header, body))
    }

    /// Requests one audio waveform or spectrum bitmap.
    async fn audio_render(&self, body: Value) -> Result<(AudioImage, Vec<u8>)> {
        let most = audio_panel::TILE_MAX_WIDTH as usize * audio_panel::TILE_MAX_HEIGHT as usize * 4;
        let (header, pixels) = self.raw_call(body, most).await?;
        let reply: AudioImage = serde_json::from_value(header).context("invalid audio display")?;
        let length = reply.body_length()?;
        if pixels.len() != length {
            return Err(anyhow!(
                "audio render sent {} bytes, expected {length}",
                pixels.len()
            ));
        }
        Ok((reply, pixels))
    }

    /// Requests the newest video frame. `since` is the serial of the frame the
    /// caller already holds; the host then answers with an empty body.
    async fn video_frame(&self, since: i64) -> Result<(VideoFrameHead, Vec<u8>)> {
        let (header, pixels) = self
            .raw_call(
                json!({"op": "video_frame", "since": since}),
                MAX_VIDEO_FRAME_BYTES,
            )
            .await?;
        let head: VideoFrameHead =
            serde_json::from_value(header).context("invalid video frame header")?;
        if head.pixel_format != "bgra8" {
            return Err(anyhow!(
                "unsupported video pixel format {}",
                head.pixel_format
            ));
        }
        if pixels.len() != head.width as usize * head.height as usize * 4 {
            return Err(anyhow!(
                "video frame body of {} bytes does not match {}x{}",
                pixels.len(),
                head.width,
                head.height
            ));
        }
        Ok((head, pixels))
    }

    async fn snapshot(&self) -> Result<Snapshot> {
        let value = self.call(json!({"op": "snapshot"})).await?;
        serde_json::from_value(value).context("invalid document snapshot")
    }

    async fn playback(&self) -> Result<PlaybackSnapshot> {
        let value = self.call(json!({"op": "playback"})).await?;
        serde_json::from_value(value).context("invalid playback snapshot")
    }

    /// Writes the current subtitles to a host-owned temporary ASS file for mpv.
    /// Returns its path and the document revision it holds.
    async fn native_subtitles(&self) -> Result<(String, u64)> {
        let value = self.call(json!({"op": "native_subtitles"})).await?;
        let path = value
            .get("path")
            .and_then(Value::as_str)
            .context("bridge reply has no subtitle path")?
            .to_owned();
        let revision = value
            .get("revision")
            .and_then(Value::as_u64)
            .context("bridge reply has no subtitle revision")?;
        Ok((path, revision))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Panel {
    Subtitles,
    Styles,
    Project,
    Visual,
    Video,
    Translation,
    Styling,
    Timing,
    Automation,
    Resample,
    Processor,
    Search,
    Paste,
    VideoSettings,
    Attachments,
    Preferences,
    Hotkeys,
    Files,
    Spelling,
    Commands,
}

impl Panel {
    fn label(self) -> &'static str {
        match self {
            Self::Subtitles => "字幕编辑",
            Self::Visual => "视觉排版",
            Self::Video => "视频",
            Self::Translation => "翻译助手",
            Self::Styles => "样式管理",
            Self::Styling => "匹配样式",
            Self::Timing => "时间平移",
            Self::Processor => "时间后处理",
            Self::Resample => "重采样",
            Self::Search => "查找替换",
            Self::Spelling => "拼写检查",
            Self::Paste => "覆盖粘贴",
            Self::Project => "脚本属性",
            Self::VideoSettings => "视频设置",
            Self::Automation => "自动化脚本",
            Self::Attachments => "附件管理",
            Self::Files => "文件与恢复",
            Self::Preferences => "偏好设置",
            Self::Hotkeys => "快捷键",
            Self::Commands => "全部命令",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Subtitles => IconName::Captions,
            Self::Visual => IconName::Frame,
            Self::Video => IconName::Maximize,
            Self::Translation => IconName::Languages,
            Self::Styles => IconName::Palette,
            Self::Styling => IconName::CaseSensitive,
            Self::Timing => IconName::Clock3,
            Self::Processor => IconName::Settings2,
            Self::Resample => IconName::Maximize,
            Self::Search => IconName::Search,
            Self::Spelling => IconName::SpellCheck,
            Self::Paste => IconName::Copy,
            Self::Project => IconName::FileText,
            Self::VideoSettings => IconName::Play,
            Self::Automation => IconName::SquareTerminal,
            Self::Attachments => IconName::Paperclip,
            Self::Files => IconName::FolderOpen,
            Self::Preferences => IconName::Settings,
            Self::Hotkeys => IconName::Keyboard,
            Self::Commands => IconName::Menu,
        }
    }
}

#[derive(Clone, Copy)]
enum WorkspaceResizeDrag {
    SubtitleEditor {
        start_y: gpui_kit::Pixels,
        start_height: f32,
    },
    /// Height of the compact waveform strip.
    AudioPanel {
        start_y: gpui_kit::Pixels,
        start_height: f32,
    },
    /// Width of one fixed column of the subtitle list.
    SubtitleColumn {
        column: SubtitleColumn,
        start_x: gpui_kit::Pixels,
        start_width: f32,
    },
    /// Width of the line inspector beside the text editor.
    PropertiesRail {
        start_x: gpui_kit::Pixels,
        start_width: f32,
    },
    /// Width of the left column (audio and video) beside the editor and list.
    ColumnSplit {
        start_x: gpui_kit::Pixels,
        start_width: f32,
    },
    /// Height of the font preview at the top of the style editor.
    StylePreview {
        start_y: gpui_kit::Pixels,
        start_height: f32,
    },
}

struct Workspace {
    bridge: Bridge,
    focus_handle: gpui_kit::FocusHandle,
    snapshot: Snapshot,
    video_epoch: u64,
    list_scroll_handle: gpui_kit::UniformListScrollHandle,
    /// Row where a press-and-drag selection started, while the button is held.
    grid_drag_anchor: Option<i64>,
    /// Row the drag painted last, so a move inside one row selects nothing new.
    grid_drag_row: i64,
    /// Set when a pointer gesture picked rows, so the resulting snapshot leaves
    /// the list where the user put it instead of pinning the row to the top.
    follow_suppressed: bool,
    subtitle_filter_cache: Option<(u64, String, String, Arc<Vec<Line>>)>,
    subtitle_editor_height: f32,
    /// Fixed columns shown before the subtitle text column.
    subtitle_columns: Vec<SubtitleColumn>,
    /// User-adjusted widths; anything missing falls back to the default.
    subtitle_column_widths: Vec<(SubtitleColumn, f32)>,
    subtitle_widths_loaded: bool,
    /// Measured width of the subtitle list, used to squeeze the columns.
    subtitle_grid_width: f32,
    layout_drag: Option<WorkspaceResizeDrag>,
    audio_panel_height: f32,
    /// Width of the left column; `None` splits the body evenly.
    column_split: Option<f32>,
    /// Height of the font preview at the top of the style editor.
    style_preview_height: f32,
    /// Width the user dragged the line inspector to; `None` keeps the default.
    properties_rail_width: Option<f32>,
    time_in_frames: bool,
    show_original: bool,
    editor_original: String,
    editor: Entity<TextareaState>,
    search: Entity<InputState>,
    command_search: Entity<InputState>,
    catalog: Arc<Catalog>,
    catalog_loading: bool,
    document: DocumentPanels,
    audio: AudioPanel,
    timeline: VideoTimeline,
    visual: VisualPanel,
    video_view: VideoView,
    translation: TranslationPanel,
    styling: StylingPanel,
    timing: TimingPanel,
    automation: AutomationPanel,
    resample: ResamplePanel,
    processor: TimingProcessorPanel,
    find_panel: SearchPanel,
    paste: PastePanel,
    video_settings: VideoSettingsPanel,
    attachments: AttachmentPanel,
    preferences: PreferencesPanel,
    hotkeys: HotkeyPanel,
    files: FilePanel,
    spelling: SpellingPanel,
    property_inputs: BTreeMap<&'static str, Entity<InputState>>,
    properties_dirty: bool,
    /// 边距/图层/效果 这一排低频字段是否展开。
    properties_more: bool,
    /// Set when the user answered "save" on the unsaved-changes card: quit as
    /// soon as the document comes back clean.
    quit_after_save: bool,
    /// Command to run once the document is clean again, e.g. closing it after
    /// the user chose "save" on the unsaved-changes card.
    after_save: Option<String>,
    /// 视图菜单里的容器开关：字幕区始终在，音频条和视频预览各自可选。
    show_audio: bool,
    show_video: bool,
    /// The picture lives in its own window; the embedded card is hidden until
    /// that window closes again.
    video_popped: bool,
    /// The file whose native libmpv surface could not be created. mpv is the
    /// renderer on macOS and Windows; this only stops it retrying one bad file.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native_video_failed: Option<String>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    mpv_surface: Option<MpvSurface>,
    /// Whether mpv draws the picture: fitted, with no visual or overscan overlay.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native_visible: bool,
    /// A subtitle file request to the host is in flight.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native_subs_inflight: bool,
    properties_line_id: i64,
    _property_events: Vec<Subscription>,
    _command_search_events: Subscription,
    pending_edits: Arc<AtomicU64>,
    editor_line_id: i64,
    editor_dirty_line: i64,
    pending_text_selection: Option<[usize; 2]>,
    active_panel: Panel,
    /// Set when this window exists only to show one tool, opened from the sidebar.
    detached: Option<Panel>,
    key_focus: KeyFocus,
    sidebar_collapsed: Option<bool>,
    selected_line: i64,
    status: String,
    notices: Vec<Notice>,
    /// Ticked "do not show again" on the notice card currently on screen.
    notice_suppress: bool,
    color_dialog: Option<ColorDialog>,
    color_picker: Entity<ColorPickerState>,
    _color_events: Vec<Subscription>,
    host_dialog: Option<HostDialog>,
    script_dialog: Option<ScriptDialogState>,
    script_dialog_replied: i64,
    font_filter: Entity<InputState>,
    log_filter: Entity<InputState>,
    _host_dialog_events: Vec<Subscription>,
    _editor_events: Subscription,
    _search_events: Subscription,
}

impl Workspace {
    fn clear_audio_image(&mut self, cx: &mut Context<Self>) {
        self.audio.clear_tiles(cx);
    }

    fn new(bridge: Bridge, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let color_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let font_filter = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("")
                .placeholder("筛选字体")
        });
        let log_filter = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("")
                .placeholder("筛选日志")
        });
        let mut host_dialog_events = Vec::new();
        for input in [&font_filter, &log_filter] {
            host_dialog_events.push(cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        let color_events =
            vec![
                cx.subscribe(&color_picker, |view, _, _: &ColorPickerEvent, cx| {
                    if let Some(dialog) = view.color_dialog.as_mut() {
                        dialog.error = None;
                    }
                    cx.notify();
                }),
            ];
        let editor = cx.new(|cx| TextareaState::new(window, cx));
        editor.update(cx, |state, cx| state.set_submit_on_enter(true, cx));
        let editor_for_events = editor.clone();
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("")
                .placeholder("筛选文本、角色或样式")
        });
        let search_events = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let command_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索命令名称、说明或 ID"));
        let command_search_events =
            cx.subscribe(&command_search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let mut property_inputs = BTreeMap::new();
        let mut property_events = Vec::new();
        for key in [
            "start",
            "end",
            "layer",
            "style",
            "actor",
            "effect",
            "margin_left",
            "margin_right",
            "margin_vertical",
        ] {
            let input = cx.new(|cx| InputState::new(window, cx));
            property_events.push(cx.subscribe(&input, |view, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    view.properties_dirty = true;
                    cx.notify();
                }
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.commit_properties(cx);
                }
            }));
            property_inputs.insert(key, input);
        }
        let document = DocumentPanels::new(window, cx);
        let timeline = VideoTimeline::new(cx);
        let translation = TranslationPanel::new(window, cx);
        let styling = StylingPanel::new(window, cx);
        let timing = TimingPanel::new(window, cx);
        let resample = ResamplePanel::new(window, cx);
        let processor = TimingProcessorPanel::new(window, cx);
        let find_panel = SearchPanel::new(window, cx);
        let video_settings = VideoSettingsPanel::new(window, cx);
        let preferences = PreferencesPanel::new(window, cx);
        let hotkeys = HotkeyPanel::new(window, cx);
        let files = FilePanel::new(window, cx);
        let spelling = SpellingPanel::new(window, cx);
        let pending_edits = Arc::new(AtomicU64::new(0));
        let editor_events = cx.subscribe(&editor, {
            let weak = cx.weak_entity();
            let bridge = bridge.clone();
            let pending_edits = pending_edits.clone();
            move |this, _, event: &InputEvent, cx| {
                if let InputEvent::PressEnter { shift, .. } = event {
                    // Enter commits the edit box and steps to the next line,
                    // creating one at the end of the script (Aegisub's default
                    // binding for the Subtitle Edit Box context).
                    if *shift {
                        return;
                    }
                    let text = this.editor_text(cx);
                    let id = this.selected_line;
                    let ids = this.snapshot.selected.clone();
                    this.editor_dirty_line = id;
                    this.invoke(json!({"op":"edit", "id":id, "ids":ids, "text":text}), cx);
                    this.invoke(json!({"op":"command", "name":"grid/line/next/create"}), cx);
                    return;
                }
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let line_id = this.selected_line;
                let selected_ids = this.snapshot.selected.clone();
                this.editor_dirty_line = line_id;
                let epoch = pending_edits.fetch_add(1, Ordering::Relaxed) + 1;
                let editor = editor_for_events.clone();
                let pending_edits = pending_edits.clone();
                let bridge = bridge.clone();
                let weak = weak.clone();
                cx.spawn(async move |_, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(450))
                        .await;
                    if pending_edits.load(Ordering::Relaxed) != epoch || line_id < 0 {
                        return;
                    }
                    let Some(view) = weak.upgrade() else {
                        return;
                    };
                    let current_line = view.read_with(cx, |view, _| view.selected_line);
                    if current_line != line_id {
                        return;
                    }
                    let soft =
                        view.read_with(cx, |view, _| view.snapshot.appearance.soft_line_break);
                    let text = escape_line_breaks(
                        editor.read_with(cx, |state, _| state.value().to_string()),
                        soft,
                    );
                    match bridge
                        .call(json!({"op":"edit", "id":line_id, "ids":selected_ids, "text":text}))
                        .await
                    {
                        Ok(_) => {
                            if let Ok(snapshot) = bridge.snapshot().await {
                                if let Some(view) = weak.upgrade() {
                                    let _ = view
                                        .update(cx, |view, cx| view.apply_snapshot(snapshot, cx));
                                }
                            }
                        }
                        Err(error) => {
                            if let Some(view) = weak.upgrade() {
                                let _ = view.update(cx, |view, cx| {
                                    view.status = format!("提交失败：{error}");
                                    cx.notify();
                                });
                            }
                        }
                    }
                })
                .detach();
            }
        });

        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            bridge,
            focus_handle,
            snapshot: Snapshot {
                file: "正在连接 Neo Aegisub…".into(),
                notices: Vec::new(),
                progress: None,
                script_dialog: None,
                commands: Vec::new(),
                modified: false,
                can_save: false,
                dark: true,
                show_toolbar: true,
                appearance: Appearance::default(),
                revision: 0,
                details_revision: 0,
                video: false,
                video_path: String::new(),
                timecodes: false,
                playing: false,
                audio: AudioState::default(),
                styles: Vec::new(),
                frame: 0,
                frame_count: 0,
                video_keyframes: Arc::default(),
                video_time: 0,
                video_duration: 0,
                playback_rate: 1.,
                aspect: 1.,
                video_epoch: 0,
                video_overscan: false,
                visual_autohide: false,
                fps: None,
                active: -1,
                selected: vec![],
                text_selection: [0, 0],
                lines: vec![],
            },
            video_epoch: 0,
            list_scroll_handle: gpui_kit::UniformListScrollHandle::new(),
            grid_drag_anchor: None,
            grid_drag_row: -1,
            follow_suppressed: false,
            subtitle_filter_cache: None,
            subtitle_editor_height: 252.,
            subtitle_columns: vec![
                SubtitleColumn::IdLayer,
                SubtitleColumn::Time,
                SubtitleColumn::StyleActor,
            ],
            subtitle_column_widths: Vec::new(),
            subtitle_widths_loaded: false,
            subtitle_grid_width: 0.,
            layout_drag: None,
            audio_panel_height: 80.,
            column_split: None,
            style_preview_height: 132.,
            properties_rail_width: None,
            time_in_frames: false,
            show_original: false,
            editor_original: String::new(),
            editor,
            search,
            command_search,
            catalog: Arc::new(Catalog::default()),
            catalog_loading: false,
            document,
            audio: AudioPanel::new(),
            timeline,
            visual: VisualPanel::new(),
            video_view: VideoView::default(),
            translation,
            styling,
            timing,
            automation: AutomationPanel::default(),
            resample,
            processor,
            find_panel,
            paste: PastePanel::default(),
            video_settings,
            attachments: AttachmentPanel::default(),
            preferences,
            hotkeys,
            files,
            spelling,
            property_inputs,
            properties_dirty: false,
            properties_more: false,
            quit_after_save: false,
            after_save: None,
            show_audio: true,
            show_video: true,
            video_popped: false,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            native_video_failed: None,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            mpv_surface: None,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            native_visible: false,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            native_subs_inflight: false,
            properties_line_id: -1,
            _property_events: property_events,
            _command_search_events: command_search_events,
            pending_edits,
            editor_line_id: -1,
            editor_dirty_line: -1,
            pending_text_selection: None,
            active_panel: Panel::Subtitles,
            detached: None,
            key_focus: KeyFocus::Grid,
            sidebar_collapsed: None,
            selected_line: -1,
            status: "已连接当前 Neo Aegisub 文档".into(),
            notices: Vec::new(),
            notice_suppress: false,
            color_dialog: None,
            color_picker,
            _color_events: color_events,
            host_dialog: None,
            script_dialog: None,
            script_dialog_replied: 0,
            font_filter,
            log_filter,
            _host_dialog_events: host_dialog_events,
            _editor_events: editor_events,
            _search_events: search_events,
        }
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        if !self.subtitle_widths_loaded {
            self.subtitle_column_widths =
                serde_json::from_str(&snapshot.appearance.grid_widths).unwrap_or_default();
            self.subtitle_widths_loaded = true;
        }
        // "保存" on the unsaved-changes card only deferred the quit until the
        // host confirms the document is clean again.
        if self.quit_after_save && !snapshot.modified {
            self.quit_after_save = false;
            cx.quit();
            return;
        }
        if !snapshot.notices.is_empty() {
            self.notices.extend(snapshot.notices.iter().cloned());
        }
        let forwarded = snapshot.commands.clone();
        if snapshot.dark != Theme::global(cx).is_dark() {
            Theme::change(
                if snapshot.dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                },
                None,
                cx,
            );
            workspace_theme::apply(cx);
        }
        let command_state_changed = snapshot.revision != self.snapshot.revision
            || snapshot.selected != self.snapshot.selected
            || snapshot.video != self.snapshot.video
            || snapshot.timecodes != self.snapshot.timecodes
            || snapshot.show_toolbar != self.snapshot.show_toolbar;
        let document_changed = snapshot.file != self.snapshot.file;
        let details_changed = snapshot.details_revision != self.snapshot.details_revision;
        if document_changed {
            self.document.style_form.dirty.clear();
            self.document.script_form.dirty.clear();
            self.properties_dirty = false;
        }
        if snapshot.audio.render_revision != self.snapshot.audio.render_revision {
            self.audio.tile_inflight.clear();
        }
        self.audio.gain = snapshot.audio.gain;
        if snapshot.audio.epoch != self.snapshot.audio.epoch
            || snapshot.audio.loaded != self.snapshot.audio.loaded
        {
            self.clear_audio_image(cx);
            self.audio.tile_inflight.clear();
            self.audio.gesture = None;
            self.audio.dragging = false;
        }
        let active_exists = snapshot.lines.iter().any(|line| line.id == snapshot.active);
        let selected = if active_exists { snapshot.active } else { -1 };
        let active_changed = selected != self.selected_line;
        if snapshot.audio.autoscroll
            && (active_changed || snapshot.audio.epoch != self.snapshot.audio.epoch)
        {
            self.audio.view_start = (snapshot.audio.start - 1000).max(0);
        }
        if snapshot.audio.spectrum != self.snapshot.audio.spectrum {
            self.audio.mode = if snapshot.audio.spectrum {
                "spectrum"
            } else {
                "waveform"
            };
        }
        self.selected_line = selected;
        // Timing moves the active row one line at a time, so the list keeps it
        // pinned to the top: the rows coming up next stay under it instead of
        // scrolling off the bottom edge. A pointer gesture in the grid is the
        // one exception, since the row it picked has to stay where it was hit.
        if active_changed && !self.follow_suppressed && selected >= 0 {
            let filter = self.search.read(cx).value().to_lowercase();
            if let Some(index) = snapshot
                .lines
                .iter()
                .filter(|line| matches_subtitle_filter(line, &filter))
                .position(|line| line.id == selected)
            {
                self.list_scroll_handle
                    .scroll_to_item_strict(index, gpui_kit::ScrollStrategy::Top);
            }
        }
        self.follow_suppressed = false;
        let media_changed = snapshot.video_epoch != self.video_epoch
            || snapshot.video != self.snapshot.video
            || snapshot.aspect != self.snapshot.aspect;
        if snapshot.revision != self.snapshot.revision {
            self.subtitle_filter_cache = None;
        }
        if snapshot.video_epoch != self.video_epoch {
            // A new or unloaded video starts with its own picture, zoom and pan.
            self.reset_video_view(cx);
        }
        self.video_epoch = snapshot.video_epoch;
        if active_changed {
            self.editor_line_id = -1;
            self.editor_dirty_line = -1;
        } else if self.editor_dirty_line == selected {
            if let Some(line) = snapshot.lines.iter().find(|line| line.id == selected) {
                if self.editor.read(cx).value().as_ref() == line.text {
                    self.editor_dirty_line = -1;
                }
            }
        }
        self.snapshot = snapshot;
        self.audio.follow_playback(&self.snapshot.audio);
        self.sync_script_dialog(cx);
        if !self.snapshot.modified {
            if let Some(action) = self.after_save.take() {
                self.invoke(json!({"op": "command", "name": action, "direct": true}), cx);
            }
        }
        if command_state_changed {
            self.refresh_catalog(cx);
        }
        if details_changed || document_changed || media_changed {
            self.refresh_document(cx);
        }
        if self.active_panel == Panel::Subtitles {
            self.refresh_audio(cx);
        }
        self.refresh_video_frame(cx);
        if self.active_panel == Panel::Attachments
            && self.snapshot.revision != self.attachments.state.revision
        {
            self.attachment_request(json!({"action":"state"}), cx);
        }
        let visual_state = (
            self.snapshot.revision,
            self.snapshot.frame,
            self.snapshot.active,
            self.video_epoch as i64,
        );
        if self.active_panel == Panel::Visual
            && !self.visual.dragging
            && self.visual.drawn != Some(visual_state)
        {
            self.visual.drawn = Some(visual_state);
            self.queue_visual(json!({"action":"draw"}), cx);
        }
        for name in forwarded {
            self.invoke(json!({"op": "command", "name": name}), cx);
        }
        cx.notify();
    }

    fn apply_playback(&mut self, playback: PlaybackSnapshot, cx: &mut Context<Self>) {
        if self.snapshot.video != playback.video || self.video_epoch != playback.video_epoch {
            return;
        }
        let frame = self.snapshot.frame;
        let playing = self.snapshot.playing;
        let audio_playing = self.snapshot.audio.playing;
        let audio_position = self.snapshot.audio.position;
        self.snapshot.frame = playback.frame;
        self.snapshot.video_time = playback.video_time;
        self.snapshot.playing = playback.playing;
        self.snapshot.audio.playing = playback.audio_playing;
        self.snapshot.audio.position = playback.audio_position;
        if self.audio.follow_playback(&self.snapshot.audio) && self.active_panel == Panel::Subtitles {
            self.refresh_audio(cx);
        }
        // Repaint only when a visible playback value changes.
        if self.snapshot.frame != frame
            || self.snapshot.playing != playing
            || self.snapshot.audio.playing != audio_playing
            || self.snapshot.audio.position != audio_position
        {
            cx.notify();
        }
        if self.snapshot.frame != frame {
            self.refresh_video_frame(cx);
        }
    }

    fn select_lines(&mut self, id: i64, extend: bool, toggle: bool, cx: &mut Context<Self>) {
        if extend {
            self.select_range(self.selected_line, id, cx);
            return;
        }
        let mut ids = if toggle {
            self.snapshot.selected.clone()
        } else {
            vec![]
        };
        if toggle {
            if ids.contains(&id) {
                ids.retain(|selected| *selected != id);
            } else {
                ids.push(id);
            }
        } else {
            ids.push(id);
        }
        // Keep an active row when removing one row from a multi-selection.
        let active = if ids.contains(&id) {
            id
        } else if ids.is_empty() {
            ids.push(id);
            id
        } else if ids.contains(&self.selected_line) {
            self.selected_line
        } else {
            ids[0]
        };
        self.invoke(json!({"op":"select", "id":active, "ids":ids}), cx);
    }

    /// Select every row between two ids in script order, with `target` active.
    /// Press-and-drag in the grid passes the row the press landed on, which is
    /// what makes a drag paint one contiguous block instead of a fresh range
    /// from wherever the cursor happens to be.
    fn select_range(&mut self, anchor: i64, target: i64, cx: &mut Context<Self>) {
        let (Some(anchor_index), Some(target_index)) = (
            self.snapshot
                .lines
                .iter()
                .position(|line| line.id == anchor),
            self.snapshot
                .lines
                .iter()
                .position(|line| line.id == target),
        ) else {
            return;
        };
        let ids: Vec<i64> = self.snapshot.lines
            [anchor_index.min(target_index)..=anchor_index.max(target_index)]
            .iter()
            .map(|line| line.id)
            .collect();
        self.invoke(json!({"op":"select", "id":target, "ids":ids}), cx);
    }

    /// Classify dropped files the same way the open dialogs do, so dragging a
    /// subtitle, video, audio or timecode file onto the window opens it.
    fn open_dropped(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        let mut opened = Vec::new();
        for path in paths.paths() {
            let Some(kind) = dropped_kind(path) else {
                continue;
            };
            // Only one subtitle can be the active document; media files add to it.
            if kind == "subtitle" && opened.iter().any(|(kind, _)| *kind == "subtitle") {
                continue;
            }
            opened.push((kind, path.to_string_lossy().to_string()));
        }
        if opened.is_empty() {
            self.status = "拖入的文件类型不受支持：可用字幕、视频、音频或时间码".into();
            cx.notify();
            return;
        }
        for (kind, path) in opened {
            let label = match kind {
                "subtitle" => "字幕",
                "video" => "视频",
                "audio" => "音频",
                _ => "时间码",
            };
            self.status = format!("已拖入{label}：{path}");
            self.invoke(json!({"op":"open", "kind":kind, "path":path}), cx);
        }
        cx.notify();
    }

    /// Read an old `config.json` and apply every option this build knows about.
    fn import_config_file(&mut self, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("导入旧版配置")
                .add_filter("Neo Aegisub 配置文件", &["json"])
                .pick_file()
                .await;
            let Some(file) = picked else {
                return;
            };
            let path = file.path().to_string_lossy().to_string();
            let result = bridge
                .call(json!({"op":"preferences","action":"import","path":path}))
                .await;
            let _ = weak.update(cx, |view, cx| match result {
                Ok(value) => {
                    let imported = value["imported"].as_i64().unwrap_or(0);
                    let skipped = value["skipped"].as_i64().unwrap_or(0);
                    view.status = if skipped > 0 {
                        format!(
                            "已导入 {imported} 项设置，{skipped} 项当前版本不适用；重启后全部生效"
                        )
                    } else {
                        format!("已导入 {imported} 项设置；重启后全部生效")
                    };
                    view.preferences_request(json!({"action":"state"}), cx);
                }
                Err(error) => {
                    view.status = format!("导入配置失败：{error}");
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn pick_file(&mut self, kind: &'static str, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let dialog = rfd::AsyncFileDialog::new();
            let dialog = match kind {
                "subtitle" => dialog.add_filter("字幕文件", &["ass", "ssa", "srt", "vtt"]),
                "timecodes" => dialog.add_filter("时间码文件", &["txt"]),
                "keyframes" => dialog.add_filter("关键帧文件", &["txt", "log", "stats", "pass"]),
                "audio" => dialog.add_filter(
                    "音频文件",
                    &["wav", "flac", "mp3", "ogg", "m4a", "mp4", "mkv"],
                ),
                _ => dialog.add_filter("视频文件", &["mp4", "mkv", "mov", "avi", "webm"]),
            };
            if let Some(file) = dialog.pick_file().await {
                let _ = weak.update(cx, |view, cx| {
                    view.invoke(
                        json!({"op":"open", "kind":kind, "path":file.path().to_string_lossy()}),
                        cx,
                    )
                });
            }
        })
        .detach();
    }

    fn save_file_as(&mut self, cx: &mut Context<Self>) {
        let filename = std::path::Path::new(&self.snapshot.file)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("untitled");
        let filename = format!("{filename}.ass");
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            if let Some(file) = rfd::AsyncFileDialog::new()
                .set_file_name(filename)
                .add_filter("ASS 字幕", &["ass"])
                .save_file()
                .await
            {
                let _ = weak.update(cx, |view, cx| {
                    view.invoke(
                        json!({"op":"save_as", "path":file.path().to_string_lossy(), "overwrite":true}),
                        cx,
                    )
                });
            }
        })
        .detach();
    }

    fn save_media_file(&mut self, kind: &'static str, cx: &mut Context<Self>) {
        let stem = std::path::Path::new(&self.snapshot.file)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("untitled");
        let (extension, label, title) = match kind {
            "audio_clip" => ("wav", "WAV 音频", "导出所选字幕范围的音频"),
            "keyframes" => ("key.txt", "关键帧文本", "保存关键帧"),
            _ => ("timecodes.txt", "VFR 时间码", "保存时间码"),
        };
        let filename = format!("{stem}.{extension}");
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            if let Some(file) = rfd::AsyncFileDialog::new().set_title(title).set_file_name(filename).add_filter(label, &[if kind == "audio_clip" {"wav"} else {"txt"}]).save_file().await {
                let _ = weak.update(cx, |view, cx| {
                    view.invoke(json!({"op":"save_media","kind":kind,"path":file.path().to_string_lossy(),"overwrite":true}), cx);
                });
            }
        }).detach();
    }

    /// 新建窗口：同一份文档的第二个窗口，各自持有自己的桥接连接。
    fn open_new_window(&mut self, cx: &mut Context<Self>) {
        // Cascade the new window: offset it and shrink it a little, otherwise every
        // window ends up centered on exactly the same spot.
        let cascade = cx.windows().len() as f32;
        let bounds = cx.primary_display().map(|display| {
            let screen = display.bounds();
            let step = px(32.) * cascade;
            let width = (screen.size.width - step * 2. - px(64.))
                .min(px(1420.))
                .max(px(880.));
            let height = (screen.size.height - step * 2. - px(64.))
                .min(px(920.))
                .max(px(560.));
            gpui_kit::Bounds {
                origin: gpui_kit::point(
                    screen.origin.x + (screen.size.width - width) / 2. + step,
                    screen.origin.y + (screen.size.height - height) / 2. + step,
                ),
                size: size(width, height),
            }
        });
        if let Err(error) = open_workspace(cx, self.bridge.clone(), bounds, None) {
            eprintln!("Could not open another GPUI window: {error:#}");
            self.status = format!("无法打开新窗口：{error}");
            cx.notify();
        }
    }

    /// The host can answer a colour pick with its own wx dialog; the front end
    /// asks for the starting colour first so the picker can be ours.
    fn dialog_call(
        &mut self,
        action: &'static str,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&mut Self, &Value, &mut Context<Self>) + 'static,
    ) {
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge.call(json!({"op": "dialog", "action": action})).await;
            let _ = weak.update(cx, |view, cx| match result {
                Ok(value) => {
                    apply(view, &value, cx);
                    cx.notify();
                }
                Err(error) => {
                    view.status = format!("对话框请求失败：{error}");
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn open_about(&mut self, cx: &mut Context<Self>) {
        self.dialog_call("about", cx, |view, value, _| {
            let text = |key: &str| value[key].as_str().unwrap_or("").to_owned();
            view.host_dialog = Some(HostDialog::About {
                version: text("version"),
                long_version: text("long_version"),
                build_time: text("build_time"),
                credit: text("credit"),
                official: value["official"].as_bool().unwrap_or(false),
            });
        });
    }

    fn open_updates(&mut self, cx: &mut Context<Self>) {
        self.host_dialog = Some(HostDialog::Updates(UpdateState {
            enabled: true,
            checking: true,
            source: FORK_RELEASES_URL.into(),
            message: "正在检查更新…".into(),
            ..Default::default()
        }));
        cx.notify();
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let mut action = "updates";
            loop {
                let result = bridge
                    .call(json!({"op": "dialog", "action": action}))
                    .await
                    .and_then(|value| {
                        serde_json::from_value::<UpdateState>(value).map_err(Into::into)
                    });
                let checking = weak
                    .update(cx, |view, cx| {
                        let Some(HostDialog::Updates(state)) = &mut view.host_dialog else {
                            return false;
                        };
                        match result {
                            Ok(value) => *state = value,
                            Err(error) => {
                                state.checking = false;
                                state.message = format!("检查更新失败：{error}");
                            }
                        }
                        cx.notify();
                        state.checking
                    })
                    .unwrap_or(false);
                if !checking {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(350))
                    .await;
                if !weak
                    .update(cx, |view, _| {
                        matches!(view.host_dialog, Some(HostDialog::Updates(_)))
                    })
                    .unwrap_or(false)
                {
                    break;
                }
                action = "update_state";
            }
        })
        .detach();
    }

    fn open_log(&mut self, cx: &mut Context<Self>) {
        self.dialog_call("log", cx, |view, value, _| {
            let entries = Self::read_log_entries(value);
            view.host_dialog = Some(HostDialog::Log { entries });
        });
    }

    fn refresh_log(&mut self, cx: &mut Context<Self>) {
        self.dialog_call("log", cx, |view, value, _| {
            if let Some(HostDialog::Log { entries }) = &mut view.host_dialog {
                *entries = Self::read_log_entries(value);
            }
        });
    }

    fn read_log_entries(value: &Value) -> Vec<LogEntry> {
        value["entries"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|entry| LogEntry {
                        section: entry["section"].as_str().unwrap_or("").to_owned(),
                        message: entry["message"].as_str().unwrap_or("").to_owned(),
                        severity: entry["severity"].as_str().unwrap_or("").to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn copy_log(&mut self, cx: &mut Context<Self>) {
        let Some(HostDialog::Log { entries }) = &self.host_dialog else {
            return;
        };
        let query = self.log_filter.read(cx).value().to_lowercase();
        let text = entries
            .iter()
            .filter(|entry| entry.matches(&query))
            .map(|entry| format!("[{}] {}: {}", entry.severity, entry.section, entry.message))
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
    }

    /// Copies the displayed frame, or the host's raw/subtitles-only render, as
    /// a PNG on the clipboard. The host owns frame decoding and PNG encoding.
    fn copy_video_frame(&mut self, variant: &'static str, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge
                .raw_call(
                    json!({"op": "video_frame", "since": -1, "variant": variant, "format": "png"}),
                    MAX_VIDEO_FRAME_BYTES,
                )
                .await;
            match result {
                Ok((header, bytes)) if !bytes.is_empty() && header["pixel_format"] == "png" => {
                    let image = gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, bytes);
                    let _ = weak.update(cx, |view, cx| {
                        cx.write_to_clipboard(gpui_kit::ClipboardItem {
                            entries: vec![gpui_kit::ClipboardEntry::Image(image)],
                        });
                        view.status = "画面已复制到剪贴板".to_owned();
                        cx.notify();
                    });
                }
                Ok(_) => set_status(&weak, cx, "复制画面失败：主机没有返回图像".to_owned()),
                Err(error) => set_status(&weak, cx, format!("复制画面失败：{error}")),
            }
        })
        .detach();
    }

    /// Copies the pointer's position in the source video, the same `(x, y)`
    /// text the old display produced.
    fn copy_video_coordinates(&mut self, cx: &mut Context<Self>) {
        let (Some((px, py)), Some(image)) = (self.video_view.pointer, &self.video_view.image)
        else {
            self.status = "指针不在画面上".to_owned();
            cx.notify();
            return;
        };
        let area = *self.video_view.bounds.borrow();
        let aw = f32::from(area.size.width).max(1.);
        let ah = f32::from(area.size.height).max(1.);
        let size = image.size(0);
        let iw = size.width.0.max(1) as f32;
        let ih = size.height.0.max(1) as f32;
        let zoom = self.video_view.zoom;
        let (pan_x, pan_y) = self.video_view.pan;
        let vw = aw * zoom;
        let vh = ah * zoom;
        let scale = (vw / iw).min(vh / ih);
        let (dw, dh) = (iw * scale, ih * scale);
        let left = aw * ((1. - zoom) / 2. + pan_x) + (vw - dw) / 2.;
        let top = ah * ((1. - zoom) / 2. + pan_y) + (vh - dh) / 2.;
        let x = ((px - left) / dw * iw).floor();
        let y = ((py - top) / dh * ih).floor();
        if x < 0. || y < 0. || x >= iw || y >= ih {
            self.status = "指针不在画面上".to_owned();
            cx.notify();
            return;
        }
        let text = format!("({}, {})", x as i64, y as i64);
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text.clone()));
        self.status = format!("已复制坐标 {text}");
        cx.notify();
    }

    fn open_language(&mut self, cx: &mut Context<Self>) {
        self.dialog_call("languages", cx, |view, value, _| {
            let items = value["languages"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .map(|item| LanguageItem {
                            code: item["code"].as_str().unwrap_or("").to_owned(),
                            name: item["name"].as_str().unwrap_or("").to_owned(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            view.host_dialog = Some(HostDialog::Language {
                items,
                active: value["active"].as_str().unwrap_or("").to_owned(),
                restart: false,
                note: value["note"].as_str().map(str::to_owned),
            });
        });
    }

    fn open_export(&mut self, cx: &mut Context<Self>) {
        self.dialog_call("export_formats", cx, |view, value, _| {
            let formats = value["formats"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .map(|format| ExportFormat {
                            name: format["name"].as_str().unwrap_or("").to_owned(),
                            description: format["description"].as_str().unwrap_or("").to_owned(),
                            checked: format["checked"].as_bool().unwrap_or(false),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let encodings = value["encodings"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|name| name.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let encoding = value["encoding"].as_str().unwrap_or("").to_owned();
            view.host_dialog = Some(HostDialog::Export {
                formats,
                encodings,
                encoding,
                busy: false,
            });
        });
    }

    fn export_script(&mut self, cx: &mut Context<Self>) {
        let Some(HostDialog::Export {
            formats,
            encoding,
            busy,
            ..
        }) = &mut self.host_dialog
        else {
            return;
        };
        if *busy {
            return;
        }
        let filters: Vec<String> = formats
            .iter()
            .filter(|format| format.checked)
            .map(|format| format.name.clone())
            .collect();
        if filters.is_empty() {
            self.status = "至少选择一个导出格式".into();
            cx.notify();
            return;
        }
        let encoding = encoding.clone();
        *busy = true;
        let stem = std::path::Path::new(&self.snapshot.file)
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_else(|| "subtitles".to_owned());
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let Some(file) = rfd::AsyncFileDialog::new()
                .set_title("导出字幕")
                .set_file_name(format!("{stem}.ass"))
                .save_file()
                .await
            else {
                let _ = weak.update(cx, |view, cx| {
                    if let Some(HostDialog::Export { busy, .. }) = &mut view.host_dialog {
                        *busy = false;
                    }
                    cx.notify();
                });
                return;
            };
            let path = file.path().to_string_lossy().to_string();
            let result = bridge
                .call(json!({
                    "op": "dialog",
                    "action": "export",
                    "path": path,
                    "filters": filters,
                    "encoding": encoding,
                }))
                .await;
            let _ = weak.update(cx, |view, cx| {
                match result {
                    Ok(_) => {
                        view.host_dialog = None;
                        view.status = format!("已导出到 {path}");
                    }
                    Err(error) => {
                        if let Some(HostDialog::Export { busy, .. }) = &mut view.host_dialog {
                            *busy = false;
                        }
                        view.status = format!("导出失败：{error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_font_collector(&mut self, cx: &mut Context<Self>) {
        self.dialog_call("font_collect", cx, |view, value, _| {
            let fonts = value["fonts"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .map(|font| FontEntry {
                            name: font["name"].as_str().unwrap_or("").to_owned(),
                            size: font["size"].as_i64().unwrap_or(0),
                        })
                        .collect()
                })
                .unwrap_or_default();
            view.host_dialog = Some(HostDialog::Fonts { fonts, busy: false });
        });
    }

    fn copy_fonts(&mut self, cx: &mut Context<Self>) {
        let Some(HostDialog::Fonts { busy, .. }) = &mut self.host_dialog else {
            return;
        };
        if *busy {
            return;
        }
        *busy = true;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let Some(directory) = rfd::AsyncFileDialog::new()
                .set_title("选择字体保存目录")
                .pick_folder()
                .await
            else {
                let _ = weak.update(cx, |view, cx| {
                    if let Some(HostDialog::Fonts { busy, .. }) = &mut view.host_dialog {
                        *busy = false;
                    }
                    cx.notify();
                });
                return;
            };
            let destination = directory.path().to_string_lossy().to_string();
            let result = bridge
                .call(json!({
                    "op": "dialog",
                    "action": "font_copy",
                    "destination": destination,
                }))
                .await;
            let _ = weak.update(cx, |view, cx| {
                if let Some(HostDialog::Fonts { busy, .. }) = &mut view.host_dialog {
                    *busy = false;
                }
                match result {
                    Ok(value) => {
                        let copied = value["copied"].as_i64().unwrap_or(0);
                        view.status = format!("已复制 {copied} 个字体文件到 {destination}");
                    }
                    Err(error) => view.status = format!("复制字体失败：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_font(&mut self, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let faces = bridge
                .call(json!({"op": "dialog", "action": "fonts"}))
                .await;
            let state = bridge
                .call(json!({"op": "dialog", "action": "font_state"}))
                .await;
            let _ = weak.update(cx, |view, cx| {
                let (Ok(faces), Ok(state)) = (faces, state) else {
                    view.status = "无法读取字体列表".into();
                    cx.notify();
                    return;
                };
                if !state["available"].as_bool().unwrap_or(false) {
                    view.status = "先选中一行字幕再改字体".into();
                    cx.notify();
                    return;
                }
                let faces = faces["fonts"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(|face| face.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                view.host_dialog = Some(HostDialog::Font {
                    faces,
                    state: FontState {
                        face: state["face"].as_str().unwrap_or("").to_owned(),
                        size: state["size"].as_i64().unwrap_or(20),
                        bold: state["bold"].as_bool().unwrap_or(false),
                        italic: state["italic"].as_bool().unwrap_or(false),
                        underline: state["underline"].as_bool().unwrap_or(false),
                    },
                    error: None,
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_language(&mut self, code: String, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge
                .call(json!({"op": "dialog", "action": "set_language", "code": code}))
                .await;
            let _ = weak.update(cx, |view, cx| match result {
                Ok(value) => {
                    if let Some(HostDialog::Language { restart, .. }) = view.host_dialog.as_mut() {
                        *restart = value["restart"].as_bool().unwrap_or(false);
                    }
                    view.status = if value["restart"].as_bool().unwrap_or(false) {
                        "界面语言已保存，重启后生效".into()
                    } else {
                        "界面语言已保存".into()
                    };
                    cx.notify();
                }
                Err(error) => {
                    view.status = format!("切换语言失败：{error}");
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn apply_font(&mut self, cx: &mut Context<Self>) {
        let Some(HostDialog::Font { state, error, .. }) = self.host_dialog.as_mut() else {
            return;
        };
        if state.size < 1 || state.size > 999 {
            *error = Some("字号需要在 1 到 999 之间".into());
            cx.notify();
            return;
        }
        let payload = json!({
            "op": "dialog",
            "action": "font_apply",
            "face": state.face,
            "size": state.size,
            "bold": state.bold,
            "italic": state.italic,
            "underline": state.underline,
        });
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge.call(payload).await;
            let _ = weak.update(cx, |view, cx| match result {
                Ok(_) => {
                    view.host_dialog = None;
                    view.status = "已应用字体".into();
                    view.refresh_document(cx);
                }
                Err(error) => {
                    if let Some(HostDialog::Font { error: slot, .. }) = view.host_dialog.as_mut() {
                        *slot = Some(format!("应用失败：{error}"));
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn close_host_dialog(&mut self, cx: &mut Context<Self>) {
        if matches!(&self.host_dialog, Some(HostDialog::Kanji(state)) if state.busy()) {
            return;
        }
        self.host_dialog = None;
        cx.notify();
    }

    fn render_host_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::AnyElement> {
        let p = Palette::current(cx);
        let Some(mut dialog) = self.host_dialog.take() else {
            return None;
        };
        let font_query = self.font_filter.read(cx).value().to_lowercase();
        let log_query = self.log_filter.read(cx).value().to_lowercase();

        let (title, hint, body, _footer): (
            gpui_kit::SharedString,
            gpui_kit::SharedString,
            gpui_kit::AnyElement,
            gpui_kit::AnyElement,
        ) = match &mut dialog {
            HostDialog::Updates(state) => {
                let mut body = div()
                    .v_flex()
                    .gap_3()
                    .child(div().text_sm().child(state.message.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted)
                            .child(format!("当前版本：{}", state.current_version)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted)
                            .child(state.source.clone()),
                    );
                for (index, release) in state.releases.iter().enumerate() {
                    let url = release.url.clone();
                    body = body.child(
                        div()
                            .v_flex()
                            .gap_2()
                            .child(div().text_sm().font_semibold().child(release.name.clone()))
                            .when(!release.description.is_empty(), |v| {
                                v.child(
                                    div()
                                        .id(("update-description", index))
                                        .max_h(px(220.))
                                        .overflow_y_scrollbar()
                                        .text_xs()
                                        .child(release.description.clone()),
                                )
                            })
                            .child(
                                Button::new(("open-release", index))
                                    .label("下载此版本")
                                    .primary()
                                    .small()
                                    .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
                            ),
                    );
                }
                body = body.child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("updates-source")
                                .label("查看发布页")
                                .ghost()
                                .small()
                                .on_click(
                                    cx.listener(|_, _, _, cx| cx.open_url(FORK_RELEASES_URL)),
                                ),
                        )
                        .child(
                            Button::new("updates-check")
                                .label("重新检查")
                                .small()
                                .disabled(state.checking || !state.enabled)
                                .on_click(cx.listener(|view, _, _, cx| view.open_updates(cx))),
                        ),
                );
                (
                    "检查更新".into(),
                    "AlanWanco/Neo-Aegisub · 正式版与预览版".into(),
                    body.into_any_element(),
                    div().into_any_element(),
                )
            }
            HostDialog::About {
                version,
                long_version,
                build_time,
                credit,
                official,
            } => {
                let mut rows: Vec<gpui_kit::AnyElement> = Vec::new();
                let mut add = |label: &'static str, value: String| {
                    rows.push(
                        div()
                            .h_flex()
                            .gap_3()
                            .items_baseline()
                            .child(
                                div()
                                    .w(px(96.))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(p.muted)
                                    .child(label),
                            )
                            .child(div().text_sm().child(value))
                            .into_any_element(),
                    );
                };
                add("版本", version.clone());
                if long_version != version {
                    add("构建", long_version.clone());
                }
                if !build_time.is_empty() {
                    add("构建时间", build_time.clone());
                }
                if !credit.is_empty() {
                    add("构建者", credit.clone());
                }
                add(
                    "发布版本",
                    if *official { "正式版" } else { "开发版" }.to_owned(),
                );
                add("界面", "GPUI 工作台".to_owned());
                add("图标", "Papirus Development Team · GPL-3.0".to_owned());
                drop(add);
                let rows = div().v_flex().gap_2().children(rows).child(
                    Button::new("about-project")
                        .label("项目与发布版本")
                        .ghost()
                        .small()
                        .on_click(cx.listener(|_, _, _, cx| cx.open_url(FORK_RELEASES_URL))),
                );
                (
                    "关于 Neo Aegisub".into(),
                    "字幕编辑与卡拉 OK 打轴工具".into(),
                    rows.into_any_element(),
                    div().into_any_element(),
                )
            }
            HostDialog::Log { entries } => {
                let shown: Vec<&LogEntry> = entries
                    .iter()
                    .filter(|entry| entry.matches(&log_query))
                    .collect();
                let mut list = div()
                    .id("host-dialog-log")
                    .v_flex()
                    .gap_1()
                    .h(px(340.))
                    .overflow_y_scrollbar()
                    .rounded(px(workspace_theme::RADIUS_MD))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.inset)
                    .p(px(8.));
                for (index, entry) in shown.iter().enumerate() {
                    let tone: gpui_kit::Hsla = match entry.severity.as_str() {
                        "error" | "exception" | "assert" => gpui_kit::rgb(0xFF6B6B).into(),
                        "warning" => gpui_kit::rgb(0xE0A030).into(),
                        _ => p.muted,
                    };
                    list = list.child(
                        div()
                            .id(("log-entry", index))
                            .h_flex()
                            .gap_2()
                            .items_start()
                            .child(
                                div()
                                    .w(px(120.))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(tone)
                                    .child(entry.section.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .text_color(p.text)
                                    .child(entry.message.clone()),
                            ),
                    );
                }
                (
                    "日志".into(),
                    format!("最近 {} 条记录，仅显示末尾 400 条", shown.len()).into(),
                    div()
                        .v_flex()
                        .gap_2()
                        .child(self.log_filter.clone())
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .child(
                                    Button::new("log-refresh")
                                        .label("刷新日志")
                                        .ghost()
                                        .small()
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.refresh_log(cx)),
                                        ),
                                )
                                .child(
                                    Button::new("log-copy")
                                        .label("复制筛选结果")
                                        .ghost()
                                        .small()
                                        .on_click(cx.listener(|view, _, _, cx| view.copy_log(cx))),
                                ),
                        )
                        .child(list)
                        .into_any_element(),
                    div().into_any_element(),
                )
            }
            HostDialog::Language {
                items,
                active,
                restart,
                note,
            } => {
                let mut list = div()
                    .id("host-dialog-language")
                    .v_flex()
                    .gap_1()
                    .h(px(340.))
                    .overflow_y_scrollbar()
                    .rounded(px(workspace_theme::RADIUS_MD))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.inset)
                    .p(px(8.));
                for item in items.iter() {
                    let code = item.code.clone();
                    list =
                        list.child(
                            crate::page::list_row(
                                cx,
                                format!("language-{}", item.code),
                                item.name.clone(),
                                &item.code == active,
                                false,
                            )
                            .on_click(cx.listener(
                                move |view, _, _, cx| view.apply_language(code.clone(), cx),
                            )),
                        );
                }
                (
                    "界面语言".into(),
                    if let Some(note) = note {
                        note.clone().into()
                    } else if *restart {
                        "已保存，重启后生效".into()
                    } else {
                        "选择界面显示语言".into()
                    },
                    div().v_flex().gap_2().child(list).into_any_element(),
                    div().into_any_element(),
                )
            }
            HostDialog::Font {
                faces,
                state,
                error,
            } => {
                let matching: Vec<&String> = faces
                    .iter()
                    .filter(|face| {
                        font_query.is_empty() || face.to_lowercase().contains(&font_query)
                    })
                    .collect();
                let mut list = div()
                    .id("host-dialog-fonts")
                    .v_flex()
                    .gap_1()
                    .h(px(260.))
                    .overflow_y_scrollbar()
                    .rounded(px(workspace_theme::RADIUS_MD))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.inset)
                    .p(px(8.));
                for face in matching.iter().take(300) {
                    let picked = face.as_str().to_owned();
                    list = list.child(
                        crate::page::list_row(
                            cx,
                            format!("font-{picked}"),
                            picked.clone(),
                            picked == state.face,
                            false,
                        )
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if let Some(HostDialog::Font { state, .. }) = view.host_dialog.as_mut()
                            {
                                state.face = picked.clone();
                            }
                            cx.notify();
                        })),
                    );
                }
                let size_row = div()
                    .h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_xs().text_color(p.muted).child("字号"))
                    .child(
                        Button::new("font-size-down")
                            .label("−")
                            .ghost()
                            .rounded(px(999.))
                            .small()
                            .px(px(13.))
                            .py(px(3.))
                            .on_click(cx.listener(|view, _, _, cx| {
                                if let Some(HostDialog::Font { state, .. }) =
                                    view.host_dialog.as_mut()
                                {
                                    state.size = (state.size - 1).max(1);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .w(px(56.))
                            .text_center()
                            .text_sm()
                            .child(state.size.to_string()),
                    )
                    .child(
                        Button::new("font-size-up")
                            .label("+")
                            .ghost()
                            .rounded(px(999.))
                            .small()
                            .px(px(13.))
                            .py(px(3.))
                            .on_click(cx.listener(|view, _, _, cx| {
                                if let Some(HostDialog::Font { state, .. }) =
                                    view.host_dialog.as_mut()
                                {
                                    state.size = (state.size + 1).min(999);
                                }
                                cx.notify();
                            })),
                    )
                    .child(div().w(px(16.)).into_any_element())
                    .child(font_flag_button(
                        cx,
                        "font-bold",
                        "粗体",
                        state.bold,
                        |state| state.bold = !state.bold,
                    ))
                    .child(font_flag_button(
                        cx,
                        "font-italic",
                        "斜体",
                        state.italic,
                        |state| state.italic = !state.italic,
                    ))
                    .child(font_flag_button(
                        cx,
                        "font-underline",
                        "下划线",
                        state.underline,
                        |state| state.underline = !state.underline,
                    ));
                let body = div()
                    .v_flex()
                    .gap_2()
                    .child(self.font_filter.clone())
                    .child(list)
                    .child(size_row)
                    .when_some(error.clone(), |v, message| {
                        v.child(
                            div()
                                .text_xs()
                                .text_color(gpui_kit::rgb(0xFF6B6B))
                                .child(message),
                        )
                    });
                (
                    "字体".into(),
                    format!("当前 {} · {} pt", state.face, state.size).into(),
                    body.into_any_element(),
                    div().into_any_element(),
                )
            }
            HostDialog::Export {
                formats,
                encodings,
                encoding,
                busy,
            } => {
                let busy = *busy;
                let mut list = div()
                    .id("host-dialog-formats")
                    .v_flex()
                    .gap_1()
                    .h(px(230.))
                    .overflow_y_scrollbar()
                    .rounded(px(workspace_theme::RADIUS_MD))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.inset)
                    .p(px(8.));
                for (index, format) in formats.iter().enumerate() {
                    let checked = format.checked;
                    let name = format.name.clone();
                    let description = format.description.clone();
                    list = list.child(
                        div()
                            .id(("export-format", index))
                            .h_flex()
                            .gap_3()
                            .items_start()
                            .px(px(8.))
                            .py(px(6.))
                            .rounded(px(workspace_theme::RADIUS_SM))
                            .cursor_pointer()
                            .when(checked, |v| v.bg(p.raised))
                            .child(
                                div()
                                    .mt(px(2.))
                                    .size(px(15.))
                                    .rounded(px(4.))
                                    .border_1()
                                    .border_color(if checked { p.accent } else { p.border })
                                    .when(checked, |v| v.bg(p.accent))
                                    .child(div().size_full().when(checked, |v| {
                                        v.child(
                                            Icon::new(IconName::Check)
                                                .size(px(11.))
                                                .text_color(p.surface),
                                        )
                                    })),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .v_flex()
                                    .gap_1()
                                    .child(div().text_sm().child(name))
                                    .when(!description.is_empty(), |v| {
                                        v.child(
                                            div().text_xs().text_color(p.muted).child(description),
                                        )
                                    }),
                            )
                            .on_click(cx.listener(move |view, _, _, cx| {
                                if let Some(HostDialog::Export { formats, .. }) =
                                    view.host_dialog.as_mut()
                                {
                                    if let Some(format) = formats.get_mut(index) {
                                        format.checked = !format.checked;
                                    }
                                }
                                cx.notify();
                            })),
                    );
                }
                let options = encodings.clone();
                let encoding_weak = cx.weak_entity();
                let encoding_row = div()
                    .h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().w(px(64.)).text_xs().text_color(p.muted).child("编码"))
                    .child(
                        crate::page::select_trigger(cx, "export-encoding", encoding.clone(), false)
                            .w(px(240.))
                            .dropdown_menu(move |mut menu, _, _| {
                                for name in options.iter() {
                                    let choice = name.clone();
                                    let target = encoding_weak.clone();
                                    menu = menu.item(PopupMenuItem::new(choice.clone()).on_click(
                                        move |_, _, cx| {
                                            let choice = choice.clone();
                                            let _ = target.update(cx, |view, cx| {
                                                if let Some(HostDialog::Export {
                                                    encoding, ..
                                                }) = view.host_dialog.as_mut()
                                                {
                                                    *encoding = choice;
                                                }
                                                cx.notify();
                                            });
                                        },
                                    ));
                                }
                                menu
                            }),
                    );
                let body = div()
                    .v_flex()
                    .gap_3()
                    .child(list)
                    .child(encoding_row)
                    .when(busy, |v| {
                        v.child(div().text_xs().text_color(p.muted).child("正在导出…"))
                    });
                (
                    "导出字幕".into(),
                    "选择要写入的格式与编码".into(),
                    body.into_any_element(),
                    div().into_any_element(),
                )
            }
            HostDialog::Fonts { fonts, busy } => {
                let busy = *busy;
                let mut list = div()
                    .id("host-dialog-font-files")
                    .v_flex()
                    .gap_1()
                    .h(px(230.))
                    .overflow_y_scrollbar()
                    .rounded(px(workspace_theme::RADIUS_MD))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.inset)
                    .p(px(8.));
                for font in fonts.iter() {
                    let size = if font.size > 0 {
                        format!("{} KB", (font.size as f64 / 1024.).round() as i64)
                    } else {
                        String::new()
                    };
                    list = list.child(
                        div()
                            .h_flex()
                            .gap_3()
                            .items_center()
                            .px(px(8.))
                            .py(px(6.))
                            .rounded(px(workspace_theme::RADIUS_SM))
                            .child(
                                Icon::new(IconName::ALargeSmall)
                                    .size(px(14.))
                                    .text_color(p.muted),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_sm()
                                    .child(font.name.clone()),
                            )
                            .child(div().text_xs().text_color(p.muted).child(size)),
                    );
                }
                let body = div()
                    .v_flex()
                    .gap_3()
                    .child(list)
                    .when(fonts.is_empty(), |v| {
                        v.child(
                            div()
                                .text_xs()
                                .text_color(p.muted)
                                .child("脚本没有用到需要打包的字体"),
                        )
                    })
                    .when(busy, |v| {
                        v.child(div().text_xs().text_color(p.muted).child("正在复制…"))
                    });
                (
                    "字体收集".into(),
                    format!("找到 {} 个字体文件", fonts.len()).into(),
                    body.into_any_element(),
                    div().into_any_element(),
                )
            }
            HostDialog::Kanji(state) => (
                "汉字打轴".into(),
                "把源音节配给目标文字，生成卡拉 OK 标签".into(),
                kanji_panel::render(state, window, cx),
                div().into_any_element(),
            ),
        };

        self.host_dialog = Some(dialog);

        let footer = div()
            .h_flex()
            .items_center()
            .justify_between()
            .px(px(20.))
            .py(px(14.))
            .border_t_1()
            .border_color(p.divider)
            .rounded_b(px(workspace_theme::RADIUS_LG - 1.))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted)
                    .child(match &self.host_dialog {
                        Some(HostDialog::Log { entries }) => {
                            format!("{} 条记录", entries.len())
                        }
                        Some(HostDialog::Language { active, .. }) => {
                            format!("当前 {active}")
                        }
                        Some(HostDialog::Font { state, .. }) => {
                            format!("{} · {} pt", state.face, state.size)
                        }
                        Some(HostDialog::Export { formats, .. }) => {
                            let checked = formats.iter().filter(|format| format.checked).count();
                            format!("{checked} / {} 个格式", formats.len())
                        }
                        Some(HostDialog::Fonts { fonts, .. }) => {
                            format!("{} 个字体文件", fonts.len())
                        }
                        Some(HostDialog::Kanji(state)) => state.summary(),
                        _ => String::new(),
                    }),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("host-dialog-close")
                            .label(if matches!(self.host_dialog, Some(HostDialog::Kanji(_))) { "取消" } else { "关闭" })
                            .disabled(matches!(&self.host_dialog, Some(HostDialog::Kanji(state)) if state.busy()))
                            .ghost()
                            .rounded(px(999.))
                            .small()
                            .px(px(13.))
                            .py(px(3.))
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.close_host_dialog(cx);
                                if view.host_dialog.is_none() { view.focus_handle.focus(window, cx); }
                            })),
                    )
                    .when(
                        matches!(self.host_dialog, Some(HostDialog::Font { .. })),
                        |v| {
                            v.child(
                                Button::new("host-dialog-apply-font")
                                    .label("应用字体")
                                    .primary()
                                    .rounded(px(999.))
                                    .small()
                                    .px(px(13.))
                                    .py(px(3.))
                                    .on_click(cx.listener(|view, _, _, cx| view.apply_font(cx))),
                            )
                        },
                    )
                    .when(
                        matches!(self.host_dialog, Some(HostDialog::Export { .. })),
                        |v| {
                            v.child(
                                Button::new("host-dialog-export")
                                    .label("导出…")
                                    .primary()
                                    .rounded(px(999.))
                                    .small()
                                    .px(px(13.))
                                    .py(px(3.))
                                    .on_click(cx.listener(|view, _, _, cx| view.export_script(cx))),
                            )
                        },
                    )
                    .when(
                        matches!(self.host_dialog, Some(HostDialog::Fonts { .. })),
                        |v| {
                            v.child(
                                Button::new("host-dialog-copy-fonts")
                                    .label("复制到文件夹…")
                                    .primary()
                                    .rounded(px(999.))
                                    .small()
                                    .px(px(13.))
                                    .py(px(3.))
                                    .on_click(cx.listener(|view, _, _, cx| view.copy_fonts(cx))),
                            )
                        },
                    )
                    .when(
                        matches!(self.host_dialog, Some(HostDialog::Kanji(_))),
                        |v| {
                            v.child(
                                Button::new("host-dialog-kanji-apply")
                                    .label("应用已接受的行")
                                    .primary()
                                    .disabled(!matches!(&self.host_dialog, Some(HostDialog::Kanji(state)) if state.can_apply()))
                                    .rounded(px(999.))
                                    .small()
                                    .px(px(13.))
                                    .py(px(3.))
                                    .on_click(cx.listener(|view, _, _, cx| view.apply_kanji(cx))),
                            )
                        },
                    ),
            );

        let card = div()
            .id("host-dialog")
            .w(px(
                if matches!(self.host_dialog, Some(HostDialog::Kanji(_))) {
                    760.
                } else {
                    560.
                },
            ))
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .max_w_full()
            .v_flex()
            .rounded(px(workspace_theme::RADIUS_LG))
            .border_1()
            .border_color(p.border)
            .bg(p.surface)
            .overflow_hidden()
            .shadow_lg()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .px(px(20.))
                    .py(px(16.))
                    .border_b_1()
                    .border_color(p.divider)
                    .rounded_t(px(workspace_theme::RADIUS_LG - 1.))
                    .child(div().text_size(px(15.)).font_semibold().child(title))
                    .child(div().text_xs().text_color(p.muted).child(hint)),
            )
            .child(
                div()
                    .id("host-dialog-body")
                    .max_h(window.viewport_size().height - px(220.))
                    .overflow_y_scrollbar()
                    .v_flex()
                    .gap_3()
                    .p(px(20.))
                    .child(body),
            )
            .child(footer);
        let _ = window;
        Some(card.into_any_element())
    }

    fn request_color(&mut self, kind: &'static str, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge
                .call(json!({"op": "color", "action": "query", "kind": kind}))
                .await;
            let _ = weak.update(cx, |view, cx| match result {
                Ok(value) => {
                    let hex = value["color"].as_str().unwrap_or("#FFFFFF").to_owned();
                    let recent = value["recent"]
                        .as_array()
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|item| item.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default();
                    view.open_color_dialog(kind, &hex, recent, cx);
                }
                Err(error) => {
                    view.status = format!("无法读取颜色：{error}");
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn open_color_dialog(
        &mut self,
        kind: &'static str,
        hex: &str,
        recent: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let (title, hint) = match kind {
            "secondary" => ("次色", "设置当前位置的 \\2c 卡拉 OK 颜色"),
            "outline" => ("描边色", "设置当前位置的 \\3c 描边颜色"),
            "shadow" => ("阴影色", "设置当前位置的 \\4c 阴影颜色"),
            _ => ("主色", "设置当前位置的 \\c 主填充颜色"),
        };
        // ASS stores alpha inverted: &H00& is opaque and &HFF& is clear.
        let colour = crate::colour_picker::parse_hex_colour(hex)
            .unwrap_or_else(|| gpui_kit::rgba(0xffff_ffff).into());
        self.color_dialog = Some(ColorDialog {
            title,
            hint,
            pending: Some(colour),
            error: None,
            loading: false,
            recent,
        });
        cx.notify();
    }

    fn apply_color_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.color_dialog.as_mut() else {
            return;
        };
        let Some(picked) = self.color_picker.read(cx).value() else {
            dialog.error = Some("请先选择颜色".into());
            cx.notify();
            return;
        };
        let color = crate::colour_picker::format_hex_colour(picked).to_uppercase();
        dialog.loading = true;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge
                .call(json!({"op": "color", "action": "apply", "color": color}))
                .await;
            let _ = weak.update(cx, |view, cx| {
                match result {
                    Ok(_) => {
                        view.color_dialog = None;
                        view.status = "已应用颜色".into();
                        view.refresh_document(cx);
                    }
                    Err(error) => {
                        if let Some(dialog) = view.color_dialog.as_mut() {
                            dialog.loading = false;
                            dialog.error = Some(error.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn cancel_color_dialog(&mut self, cx: &mut Context<Self>) {
        if self.color_dialog.is_none() {
            return;
        }
        self.color_dialog = None;
        cx.notify();
        let bridge = self.bridge.clone();
        cx.spawn(async move |_, _| {
            let _ = bridge
                .call(json!({"op": "color", "action": "cancel"}))
                .await;
        })
        .detach();
    }

    /// Long host tasks (indexing a video, collecting fonts) report their state
    /// through the snapshot; this draws the card the wx progress dialog used to.
    fn render_progress_card(&mut self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let progress = self.snapshot.progress.clone()?;
        let p = Palette::current(cx);
        let border = p.border;
        let fraction = progress.fraction.clamp(-1., 1.);
        let percent = if fraction < 0. {
            None
        } else {
            Some((fraction * 100.).round() as i32)
        };
        let fill: f32 = if fraction < 0. {
            0.28
        } else {
            fraction.max(0.02) as f32
        };

        Some(
            div()
                .id("host-progress")
                .w(px(460.))
                .max_w_full()
                .v_flex()
                .rounded(px(workspace_theme::RADIUS_LG))
                .border_1()
                .border_color(border)
                .bg(p.surface)
                .overflow_hidden()
                .shadow_lg()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .px(px(20.))
                        .py(px(14.))
                        .bg(p.raised)
                        .rounded_t(px(workspace_theme::RADIUS_LG - 1.))
                        .border_b_1()
                        .border_color(border)
                        .child(
                            Icon::new(IconName::Clock3)
                                .size(px(16.))
                                .text_color(p.accent),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child(if progress.title.is_empty() {
                                    "处理中".to_string()
                                } else {
                                    progress.title.clone()
                                }),
                        ),
                )
                .child(
                    div()
                        .v_flex()
                        .gap_3()
                        .px(px(20.))
                        .py(px(16.))
                        .text_sm()
                        .text_color(p.text)
                        .child(if progress.message.is_empty() {
                            "正在处理…".to_string()
                        } else {
                            progress.message.clone()
                        })
                        .child(
                            div()
                                .h(px(6.))
                                .w_full()
                                .rounded(px(999.))
                                .bg(p.inset)
                                .overflow_hidden()
                                .child(
                                    div()
                                        .h_full()
                                        .w(gpui_kit::relative(fill))
                                        .rounded(px(999.))
                                        .bg(p.accent),
                                ),
                        )
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .justify_between()
                                .child(div().text_xs().text_color(p.muted).child(match percent {
                                    Some(percent) => format!("{percent}%"),
                                    None => "正在计算进度…".to_string(),
                                }))
                                .when(progress.cancellable, |row| {
                                    row.child(
                                        Button::new("progress-cancel")
                                            .label("取消")
                                            .ghost()
                                            .rounded(px(999.))
                                            .small()
                                            .px(px(13.))
                                            .py(px(3.))
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.cancel_progress(cx)
                                            })),
                                    )
                                }),
                        ),
                )
                .into_any_element(),
        )
    }

    fn cancel_progress(&mut self, cx: &mut Context<Self>) {
        self.invoke(json!({"op": "progress_cancel"}), cx);
    }

    fn render_color_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::AnyElement> {
        let p = Palette::current(cx);
        let Some(dialog) = self.color_dialog.as_mut() else {
            return None;
        };
        if let Some(colour) = dialog.pending.take() {
            self.color_picker
                .update(cx, |state, cx| state.set_value(colour, window, cx));
        }
        let current = self
            .color_picker
            .read(cx)
            .value()
            .unwrap_or_else(|| gpui_kit::rgba(0xffff_ffff).into());
        let preview_label = {
            let hex = crate::colour_picker::format_hex_rgb(current).to_uppercase();
            let opacity = (current.a * 100.).round() as i32;
            format!("{hex} · {opacity}%")
        };
        let title = dialog.title;
        let hint = dialog.hint;
        let error = dialog.error.clone();
        let loading = dialog.loading;
        let recent = dialog.recent.clone();
        let featured: Vec<gpui_kit::Hsla> = recent
            .iter()
            .filter_map(|hex| crate::colour_picker::parse_hex_colour(hex))
            .collect();
        let card = div()
            .id("color-dialog")
            .w(px(480.))
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .max_w_full()
            .v_flex()
            .rounded(px(workspace_theme::RADIUS_LG))
            .border_1()
            .border_color(p.border)
            .bg(p.surface)
            .overflow_hidden()
            .shadow_lg()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .px(px(20.))
                    .py(px(16.))
                    .border_b_1()
                    .border_color(p.divider)
                    .rounded_t(px(workspace_theme::RADIUS_LG - 1.))
                    .child(div().text_size(px(15.)).font_semibold().child(title))
                    .child(div().text_xs().text_color(p.muted).child(hint)),
            )
            .child(
                div()
                    .v_flex()
                    .gap_3()
                    .p(px(20.))
                    .child(
                        div()
                            .relative()
                            .h(px(56.))
                            .w_full()
                            .rounded(px(workspace_theme::RADIUS_SM))
                            .border_1()
                            .border_color(p.border)
                            .bg(p.inset)
                            .overflow_hidden()
                            // overflow_hidden clips to a rectangle, so the fill
                            // has to carry the radius itself or it paints over
                            // the four corners of the rounded frame.
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .rounded(px(workspace_theme::RADIUS_SM))
                                    .bg(current),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .h_flex()
                                    .items_end()
                                    .justify_end()
                                    .p(px(8.))
                                    .child(
                                        div()
                                            .rounded(px(6.))
                                            .bg(p.surface)
                                            .px(px(6.))
                                            .py(px(2.))
                                            .text_xs()
                                            .text_color(p.muted)
                                            .child(preview_label),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_3()
                            .items_center()
                            .child(crate::page::field_label(cx, "颜色"))
                            .child({
                                // The same picker as the settings page; the
                                // host's recent colours ride along as its
                                // featured row.
                                let mut picker = ColorPicker::new(&self.color_picker)
                                    .anchor(gpui_kit::Anchor::TopLeft)
                                    .accessibility_label(title);
                                if !featured.is_empty() {
                                    picker = picker.featured_colors(featured);
                                }
                                picker
                            }),
                    )
                    .when_some(error, |v, error| {
                        v.child(
                            div()
                                .text_xs()
                                .text_color(gpui_kit::rgb(0xFF6B6B))
                                .child(error),
                        )
                    }),
            )
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .px(px(20.))
                    .py(px(14.))
                    .border_t_1()
                    .border_color(p.divider)
                    .rounded_b(px(workspace_theme::RADIUS_LG - 1.))
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted)
                            .child("只影响当前行与所选行的 override 标签"),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Button::new("color-cancel")
                                    .label("取消")
                                    .ghost()
                                    .rounded(px(999.))
                                    .small()
                                    .px(px(13.))
                                    .py(px(3.))
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.cancel_color_dialog(cx)),
                                    ),
                            )
                            .child(
                                Button::new("color-apply")
                                    .label("应用")
                                    .primary()
                                    .rounded(px(999.))
                                    .small()
                                    .px(px(13.))
                                    .py(px(3.))
                                    .disabled(loading)
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.apply_color_dialog(cx)),
                                    ),
                            ),
                    ),
            );
        Some(card.into_any_element())
    }

    fn invoke(&mut self, mut payload: Value, cx: &mut Context<Self>) {
        let direct = payload["direct"].as_bool().unwrap_or(false);
        if payload["op"] == "command" && !direct {
            if let Some((tool, subtool)) = payload["name"]
                .as_str()
                .and_then(visual_panel::command_tool)
            {
                self.follow_visual_command(tool, subtool, cx);
            }
            match payload["name"].as_str() {
                // These two belong to the front end: the host would answer them
                // by opening its own wx window or frame.
                Some("app/exit") => {
                    cx.quit();
                    return;
                }
                Some(name @ ("subtitle/new" | "subtitle/close")) => {
                    if self.snapshot.modified {
                        self.after_save = Some(name.to_owned());
                        self.invoke(
                            json!({"op": "command", "name": "subtitle/save", "direct": true}),
                            cx,
                        );
                        self.status = "正在保存修改，保存完成后继续".to_owned();
                        cx.notify();
                    } else {
                        self.invoke(json!({"op": "command", "name": name, "direct": true}), cx);
                    }
                    return;
                }
                Some("app/new_window") => {
                    self.open_new_window(cx);
                    return;
                }
                Some("app/about") => {
                    self.open_about(cx);
                    return;
                }
                Some("app/log") => {
                    self.open_log(cx);
                    return;
                }
                Some("app/language") => {
                    self.open_language(cx);
                    return;
                }
                Some("app/updates") => {
                    self.open_updates(cx);
                    return;
                }
                Some("edit/font") => {
                    self.open_font(cx);
                    return;
                }
                Some("tool/export") => {
                    self.open_export(cx);
                    return;
                }
                Some("tool/font_collector") => {
                    self.open_font_collector(cx);
                    return;
                }
                Some("tool/time/kanji") => {
                    self.open_kanji(cx);
                    return;
                }
                Some("subtitle/open/video") => {
                    self.pick_file("video", cx);
                    return;
                }
                Some(
                    name @ ("edit/color/primary"
                    | "edit/color/secondary"
                    | "edit/color/outline"
                    | "edit/color/shadow"),
                ) => {
                    let kind = match name {
                        "edit/color/secondary" => "secondary",
                        "edit/color/outline" => "outline",
                        "edit/color/shadow" => "shadow",
                        _ => "primary",
                    };
                    self.request_color(kind, cx);
                    return;
                }
                Some(name @ ("audio/save/clip" | "keyframe/save" | "timecode/save")) => {
                    self.save_media_file(
                        match name {
                            "audio/save/clip" => "audio_clip",
                            "keyframe/save" => "keyframes",
                            _ => "timecodes",
                        },
                        cx,
                    );
                    return;
                }
                Some("subtitle/spellcheck") => {
                    self.open_tool(Panel::Spelling, cx, |view, cx| {
                        view.spelling_request(json!({"action":"state"}), cx)
                    });
                    return;
                }
                Some("app/toggle/toolbar") => {
                    self.preferences_request(json!({"action":"apply","fields":{"App/Show Toolbar":if self.snapshot.show_toolbar {"0"} else {"1"}}}), cx);
                    return;
                }
                Some("app/options") => {
                    self.open_tool(Panel::Preferences, cx, |view, cx| {
                        view.preferences_request(json!({"action":"state"}), cx)
                    });
                    return;
                }
                Some("subtitle/attachment") => {
                    self.open_tool(Panel::Attachments, cx, |view, cx| {
                        view.attachment_request(json!({"action":"state"}), cx)
                    });
                    return;
                }
                Some("subtitle/open/charset") => {
                    self.open_tool(Panel::Files, cx, |view, cx| {
                        view.files.page = FilePage::Encoding;
                        view.file_tools_request(json!({"action":"state"}), cx);
                        view.pick_encoded_subtitle(cx);
                    });
                    return;
                }
                Some("subtitle/open/autosave") => {
                    self.open_tool(Panel::Files, cx, |view, cx| {
                        view.files.page = FilePage::Recovery;
                        view.file_tools_request(json!({"action":"state"}), cx);
                    });
                    return;
                }
                Some("subtitle/open") => {
                    self.pick_file("subtitle", cx);
                    return;
                }
                Some(
                    name @ ("video/open/dummy"
                    | "video/jump"
                    | "video/aspect/custom"
                    | "video/details"),
                ) => {
                    let page = match name {
                        "video/open/dummy" => VideoPage::Dummy,
                        "video/jump" => VideoPage::Jump,
                        "video/aspect/custom" => VideoPage::Aspect,
                        _ => VideoPage::Details,
                    };
                    self.open_tool(Panel::VideoSettings, cx, move |view, cx| {
                        view.video_settings.page = page;
                        view.video_settings.initialized = false;
                        view.video_settings_request("state", cx);
                    });
                    return;
                }
                Some("video/open") => {
                    self.pick_file("video", cx);
                    return;
                }
                Some("video/copy_coordinates") => {
                    self.copy_video_coordinates(cx);
                    return;
                }
                Some(
                    name @ ("video/frame/copy" | "video/frame/copy/raw" | "video/frame/copy/subs"),
                ) => {
                    self.copy_video_frame(
                        match name {
                            "video/frame/copy/raw" => "raw",
                            "video/frame/copy/subs" => "subs",
                            _ => "display",
                        },
                        cx,
                    );
                    return;
                }
                Some("video/zoom/in") => {
                    self.video_zoom_by(1., cx);
                    return;
                }
                Some("video/zoom/out") => {
                    self.video_zoom_by(-1., cx);
                    return;
                }
                Some("video/zoom/100") => {
                    self.video_zoom_actual(cx);
                    return;
                }
                Some("video/reset_pan") => {
                    self.video_view.pan = (0., 0.);
                    cx.notify();
                    return;
                }
                Some("video/detach") => {
                    if self.video_popped {
                        self.status = "视频画面已在独立窗口".to_owned();
                    } else {
                        self.open_tool(Panel::Video, cx, |_, _| {});
                        self.video_popped = true;
                        self.status = "视频画面已移到独立窗口".to_owned();
                    }
                    cx.notify();
                    return;
                }
                Some("audio/open") => {
                    self.pick_file("audio", cx);
                    return;
                }
                Some("keyframe/open") => {
                    self.pick_file("keyframes", cx);
                    return;
                }
                Some("timecode/open") => {
                    self.pick_file("timecodes", cx);
                    return;
                }
                Some("subtitle/save/as") => {
                    self.save_file_as(cx);
                    return;
                }
                Some("subtitle/save") if !self.snapshot.can_save => {
                    self.save_file_as(cx);
                    return;
                }
                Some(
                    name @ ("app/display/subs"
                    | "app/display/video_subs"
                    | "app/display/audio_subs"
                    | "app/display/full"),
                ) => {
                    // Container visibility and menu checks belong to this workspace.
                    self.show_audio = matches!(name, "app/display/audio_subs" | "app/display/full");
                    self.show_video = matches!(name, "app/display/video_subs" | "app/display/full");
                    self.sync_display_mode(cx);
                    cx.notify();
                    return;
                }
                Some(name)
                    if name.starts_with("audio/go_to") || name.starts_with("audio/scroll/") =>
                {
                    // These commands only move the waveform within the workspace.
                    let state = &self.snapshot.audio;
                    let start = match name {
                        "audio/go_to/start" => state.start - self.audio.span / 2,
                        "audio/go_to/end" => state.end - self.audio.span / 2,
                        "audio/scroll/left" => {
                            self.audio.view_start
                                - (self.audio.span * 128
                                    / f32::from(self.audio.bounds.borrow().size.width).max(1.)
                                        as i64)
                        }
                        "audio/scroll/right" => {
                            self.audio.view_start
                                + (self.audio.span * 128
                                    / f32::from(self.audio.bounds.borrow().size.width).max(1.)
                                        as i64)
                        }
                        _ => (state.start + state.end - self.audio.span) / 2,
                    };
                    self.audio.view_start =
                        start.clamp(0, (state.duration - self.audio.span).max(0));
                    self.refresh_audio(cx);
                    cx.notify();
                    return;
                }
                Some("am/manager" | "am/meta") => {
                    self.open_tool(Panel::Automation, cx, |view, cx| {
                        view.automation_request(json!({"action":"state"}), cx)
                    });
                    return;
                }
                Some("am/reload" | "am/reload/autoload") => {
                    let action = if payload["name"] == "am/reload" {
                        "reload_all"
                    } else {
                        "rescan"
                    };
                    self.automation_request(json!({"action":action}), cx);
                    return;
                }
                Some("edit/line/paste/over") => {
                    self.open_tool(Panel::Paste, cx, |view, cx| view.paste_request(false, cx));
                    return;
                }
                Some("tool/line/select") => {
                    self.open_search(true, cx);
                    return;
                }
                Some("subtitle/find" | "edit/find_replace") => {
                    self.open_search(false, cx);
                    return;
                }
                Some("subtitle/find/next") => {
                    if self.find_panel.initialized
                        && !self.find_panel.find.read(cx).value().is_empty()
                    {
                        self.search_request("find", cx);
                    } else {
                        self.open_search(false, cx);
                    }
                    return;
                }
                Some("tool/time/postprocess") => {
                    self.open_tool(Panel::Processor, cx, |view, cx| {
                        view.processor_request("state", cx)
                    });
                    return;
                }
                Some("tool/resampleres") => {
                    self.open_tool(Panel::Resample, cx, |view, cx| {
                        view.resample_request("state", cx)
                    });
                    return;
                }
                Some("time/shift") => {
                    self.open_tool(Panel::Timing, cx, |view, cx| {
                        view.shift_times_request("state", cx)
                    });
                    return;
                }
                Some("tool/style/assistant") => {
                    self.open_tool(Panel::Styling, cx, |view, cx| {
                        view.styling.line_id = -1;
                        view.styling.focus_pending = true;
                        view.refresh_document(cx);
                        view.styling_action("start", None, cx);
                    });
                    return;
                }
                Some("tool/styling_assistant/commit") => {
                    self.styling_action("commit", None, cx);
                    return;
                }
                Some("tool/styling_assistant/preview") => {
                    self.styling_action("preview", None, cx);
                    return;
                }
                Some("tool/translation_assistant") => {
                    self.open_tool(Panel::Translation, cx, |view, cx| {
                        view.translation_action("start", cx)
                    });
                    return;
                }
                Some(name) if name.starts_with("tool/translation_assistant/") => {
                    let action = match name.rsplit('/').next().unwrap_or_default() {
                        "commit" => "commit",
                        "preview" => "preview",
                        "next" => "next",
                        "prev" => "prev",
                        _ => "insert_original",
                    };
                    self.translation_action(action, cx);
                    return;
                }
                Some("tool/style/manager") => {
                    self.open_tool(Panel::Styles, cx, |view, cx| {
                        view.refresh_document(cx);
                        view.refresh_library(cx);
                    });
                    return;
                }
                Some("subtitle/properties") => {
                    self.open_tool(Panel::Project, cx, |view, cx| view.refresh_document(cx));
                    return;
                }
                _ => {}
            }
        }
        if payload["op"] == "command" && self.selected_line >= 0 {
            let range = self.editor.read(cx).selected_range();
            payload["text_selection"] = json!([range.start, range.end]);
        }
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        let draft = self.pending_draft(cx);
        cx.spawn(async move |_, cx| {
            if let Some(draft) = draft {
                if let Err(error) = bridge.call(draft).await {
                    set_status(&weak, cx, format!("字幕提交失败：{error}"));
                    return;
                }
            }
            let succeeded = invoke_weak(&bridge, &weak, cx, payload.clone()).await;
            if let Some(view) = weak.upgrade() {
                let _ = view.update(cx, |view, cx| {
                    if succeeded {
                        match payload["op"].as_str() {
                            Some("script_info") => view
                                .document
                                .script_form
                                .acknowledge(&payload["fields"], cx),
                            Some("style") => {
                                view.document.style_form.acknowledge(&payload["fields"], cx);
                                if let Some(name) = payload["fields"]["name"].as_str() {
                                    view.document.style = name.into();
                                }
                            }
                            Some("styling") => {
                                view.styling.pending_value = None;
                                view.styling.line_id = -1;
                                view.styling.focus_pending = true;
                            }
                            Some("open")
                                if payload["kind"] == "subtitle"
                                    && view.active_panel == Panel::Files =>
                            {
                                view.active_panel = Panel::Subtitles;
                            }
                            Some("save_media") => {
                                view.status = format!(
                                    "已保存：{}",
                                    payload["path"].as_str().unwrap_or_default()
                                )
                            }
                            Some("line_fields") => view.properties_dirty = false,
                            Some("command") => {
                                view.pending_text_selection = Some(view.snapshot.text_selection);
                                cx.notify();
                            }
                            _ => {}
                        }
                    }
                    view.refresh_catalog(cx);
                    view.refresh_document(cx);
                    if view.active_panel == Panel::Attachments {
                        view.attachment_request(json!({"action":"state"}), cx);
                    }
                    if view.active_panel == Panel::Processor {
                        view.processor_request("state", cx);
                    }
                    if view.active_panel == Panel::Spelling {
                        view.spelling_request(json!({"action":"state"}), cx);
                    }
                });
            }
        })
        .detach();
    }

    fn pending_draft(&mut self, cx: &mut Context<Self>) -> Option<Value> {
        if self.editor_dirty_line != self.selected_line || self.selected_line < 0 {
            return None;
        }
        self.pending_edits.fetch_add(1, Ordering::Relaxed);
        Some(
            json!({"op":"edit", "id":self.selected_line, "ids":self.snapshot.selected, "text":self.editor.read(cx).value().to_string()}),
        )
    }

    fn flush_video_seek(&mut self, cx: &mut Context<Self>) {
        // One seek at a time. A seek restarts the decoder at the previous
        // keyframe, so issuing the next one while the last is still running
        // throws that work away. The newest position waits in `pending_frame`
        // and is flushed when the current seek returns.
        if self.timeline.seeking || !self.snapshot.video {
            return;
        }
        let Some(frame) = self.timeline.pending_frame.take() else {
            if !self.timeline.dragging && std::mem::take(&mut self.timeline.resume_after_drag) {
                self.invoke(json!({"op":"command", "name":"video/play"}), cx);
            }
            return;
        };
        self.timeline.seeking = true;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            invoke_weak(&bridge, &weak, cx, json!({"op":"seek", "frame":frame})).await;
            let _ = weak.update(cx, |view, cx| {
                view.timeline.seeking = false;
                view.flush_video_seek(cx);
            });
        })
        .detach();
    }

    fn select_visual_tool(
        &mut self,
        mode: &'static str,
        subtool: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.active_panel = Panel::Visual;
        self.visual.tool = mode;
        if let Some(subtool) = subtool {
            self.visual.subtool = subtool;
        }
        self.visual.bounds.replace(gpui_kit::Bounds::default());
        self.visual
            .canvas_bounds
            .replace(gpui_kit::Bounds::default());
        self.visual.queue.clear();
        self.visual.commands.clear();
        self.visual.drawn = None;
        self.visual.dragging = false;
        cx.notify();
    }

    /// A `video/tool/*` command switches the host tool without this panel
    /// hearing about it, and the next request would switch it straight back.
    /// On the visual page the tool is reselected as a toolbar click would be;
    /// elsewhere it is only remembered for the next visit.
    fn follow_visual_command(
        &mut self,
        tool: &'static str,
        subtool: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        if self.active_panel == Panel::Visual {
            self.select_visual_tool(tool, subtool, cx);
            return;
        }
        self.visual.tool = tool;
        if let Some(subtool) = subtool {
            self.visual.subtool = subtool;
        }
        self.visual.commands.clear();
        self.visual.drawn = None;
    }

    /// Sends one interaction to the visual tool. Requests are serialised: the
    /// host answers with the drawing commands for the resulting state, and a
    /// second request in flight would draw over the first one's answer.
    fn queue_visual(&mut self, mut request: Value, cx: &mut Context<Self>) {
        if !self.snapshot.video {
            return;
        }
        let bounds = *self.visual.canvas_bounds.borrow();
        if bounds.size.width < px(16.) || bounds.size.height < px(16.) {
            return;
        }
        request["op"] = json!("visual");
        request["tool"] = json!(self.visual.tool);
        request["width"] = json!(f32::from(bounds.size.width).round() as i64);
        request["height"] = json!(f32::from(bounds.size.height).round() as i64);
        if self.visual.tool == "vector_clip" {
            request["subtool"] = json!(self.visual.subtool);
        }
        // Only the newest pointer move matters; a drag step that was never
        // seen still has to be applied, so nothing but moves is coalesced.
        let replace = self.visual.queue.back().is_some_and(|previous| {
            previous["action"] == request["action"]
                && matches!(request["action"].as_str(), Some("move" | "draw"))
        });
        if replace {
            self.visual.queue.pop_back();
        }
        self.visual.queue.push_back(request);
        self.flush_visual(cx);
    }

    fn flush_visual(&mut self, cx: &mut Context<Self>) {
        if self.visual.loading {
            return;
        }
        let Some(request) = self.visual.queue.pop_front() else {
            return;
        };
        self.visual.loading = true;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        let changes = matches!(
            request["action"].as_str(),
            Some("down" | "double" | "toggle_move" | "up")
        ) || self.visual.dragging;
        let request_epoch = self.video_epoch;
        let refresh_catalog = matches!(request["action"].as_str(), Some("draw" | "up"));
        cx.spawn(async move |_, cx| {
            let result = bridge.call(request).await.and_then(|value| {
                serde_json::from_value::<VisualReply>(value).context("invalid visual drawing")
            });
            let snapshot = if changes && result.is_ok() {
                bridge.snapshot().await.ok()
            } else {
                None
            };
            let _ = weak.update(cx, |view, cx| {
                view.visual.loading = false;
                match result {
                    Ok(reply) => {
                        let bounds = *view.visual.canvas_bounds.borrow();
                        if view.video_epoch == request_epoch
                            && reply.tool == view.visual.tool
                            && reply.width == f32::from(bounds.size.width).round() as i64
                            && reply.height == f32::from(bounds.size.height).round() as i64
                            && (reply.tool != "vector_clip" || reply.subtool == view.visual.subtool)
                        {
                            view.visual.commands = reply.commands;
                        }
                    }
                    Err(error) => view.status = format!("视觉工具失败：{error}"),
                }
                if let Some(snapshot) = snapshot {
                    view.apply_snapshot(snapshot, cx);
                    view.refresh_video_frame(cx);
                }
                if refresh_catalog {
                    view.refresh_catalog(cx);
                }
                view.flush_visual(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Forgets the picture of the previous video and resets zoom and pan.
    fn reset_video_view(&mut self, cx: &mut Context<Self>) {
        if let Some(old) = self.video_view.image.take() {
            cx.drop_image(old, None);
        }
        self.video_view.serial = -1;
        self.video_view.frame = -1;
        self.video_view.retries = 0;
        self.video_view.zoom = 1.;
        self.video_view.source_size = None;
        self.video_view.pan = (0., 0.);
        self.video_view.pan_drag = None;
    }

    /// Ratio between the fitted picture and the frame's native pixels.
    fn video_fit_scale(&self) -> Option<f32> {
        let (source_width, source_height) = self.video_view.source_size?;
        let area = *self.video_view.bounds.borrow();
        let width = f32::from(area.size.width);
        let height = f32::from(area.size.height);
        if width <= 0. || height <= 0. {
            return None;
        }
        let scale = (width / source_width as f32).min(height / source_height as f32);
        (scale > 0. && scale.is_finite()).then_some(scale)
    }

    /// Zoom shown to the user, relative to the frame's native pixels; the
    /// internal `zoom` value remains relative to the fitted picture.
    fn video_zoom_percent(&self) -> f32 {
        self.video_fit_scale()
            .map_or(self.video_view.zoom * 100., |scale| {
                self.video_view.zoom * scale * 100.
            })
    }

    /// Zooms the picture by `steps` wheel notches; going back to the fit
    /// recentres it. The lower bound leaves room for small sources at 100%.
    fn video_zoom_by(&mut self, steps: f32, cx: &mut Context<Self>) {
        let view = &mut self.video_view;
        view.zoom = (view.zoom * 1.25_f32.powf(steps)).clamp(0.125, 10.);
        if view.zoom <= 1. {
            view.pan = (0., 0.);
        }
        cx.notify();
    }

    fn video_fit_window(&mut self, cx: &mut Context<Self>) {
        self.video_view.zoom = 1.;
        self.video_view.pan = (0., 0.);
        cx.notify();
    }

    /// Shows a percentage of the frame's native pixels.
    fn video_zoom_to_percent(&mut self, percent: f32, cx: &mut Context<Self>) {
        let Some(scale) = self.video_fit_scale() else {
            return;
        };
        self.video_view.zoom = (percent / 100. / scale).clamp(0.125, 10.);
        self.video_view.pan = (0., 0.);
        cx.notify();
    }

    /// Shows one native source pixel per logical screen pixel.
    fn video_zoom_actual(&mut self, cx: &mut Context<Self>) {
        self.video_zoom_to_percent(100., cx);
    }

    fn video_reset_view(&mut self, cx: &mut Context<Self>) {
        self.video_fit_window(cx);
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn sync_native_video(&mut self, window: &Window, cx: &mut Context<Self>) {
        let was_visible = self.native_visible;
        self.update_native_video(window, cx);
        // The host paints the picture again once mpv stops drawing it.
        if was_visible && !self.native_visible {
            self.refresh_video_frame(cx);
        }
    }

    /// Whether a GPUI layer is drawn over the picture. mpv draws into a window
    /// of its own, above every layer the host paints in this one, so any
    /// overlay that lands on the picture has to be drawn while mpv steps
    /// aside: popovers (dropdowns, selects, colour pickers) register
    /// themselves with the Base global while open, and a notice covers the
    /// whole window with a scrim.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn overlay_over_picture(&self, cx: &App) -> bool {
        gpui_kit::base::GlobalState::is_in_deferred_context(cx) || !self.notices.is_empty()
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn update_native_video(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.snapshot.video
            || self.snapshot.video_path.is_empty()
            || self.video_popped
        {
            self.mpv_surface = None;
            self.native_visible = false;
            return;
        }
        let bounds = *self.video_view.bounds.borrow();
        if f32::from(bounds.size.width) <= 0. || f32::from(bounds.size.height) <= 0. {
            return;
        }
        let path = self.snapshot.video_path.clone();
        if self.native_video_failed.as_deref() == Some(path.as_str()) {
            self.mpv_surface = None;
            self.native_visible = false;
            return;
        }
        // mpv's window covers the picture, but rounding inside it can still
        // leave a row or column of its own, painted in the card's colour.
        let background = rgb_hex(Palette::current(cx).inset);
        let needs_surface = self
            .mpv_surface
            .as_ref()
            .map_or(true, |surface| surface.path() != path);
        if needs_surface {
            self.mpv_surface = None;
            match MpvSurface::new(window, &path, background) {
                Ok(surface) => self.mpv_surface = Some(surface),
                Err(error) => {
                    self.native_video_failed = Some(path.clone());
                    self.native_visible = false;
                    let status = format!("原生视频不可用：{error}");
                    if self.status != status {
                        self.status = status;
                    }
                    return;
                }
            }
        }
        // mpv draws only while the picture is fitted and nothing sits on it.
        let fit = self.video_view.zoom == 1. && self.video_view.pan == (0., 0.);
        let overlay = self.active_panel == Panel::Visual
            || self.snapshot.video_overscan
            || self.host_dialog.is_some()
            || self.color_dialog.is_some()
            || self.script_dialog.is_some()
            // Both mpv's window and the host's dialogs are windows of their
            // own; the host's layers never reach over either.
            || self.overlay_over_picture(cx);
        let shown = self.show_video && self.detached.is_none_or(|panel| panel == Panel::Video);
        // Paused or not, mpv holds the picture: switching renderers on a pause
        // is what moved and flashed the picture, so playback state stays out.
        let wants = fit && !overlay && shown;
        let frame_secs = self
            .snapshot
            .fps
            .filter(|fps| *fps > 0.)
            .map_or(0., |fps| 1. / fps);
        let Some(surface) = self.mpv_surface.as_mut() else {
            return;
        };
        surface.set_background(background);
        // mpv holds the picture whenever the host does not paint it, so a
        // pause never changes who draws: the picture cannot move or flash on
        // the transition. The host picture is left for overlays, zoomed views,
        // the popped-out window and platforms without a native surface.
        self.native_visible = wants;
        surface.sync(
            self.snapshot.playing,
            self.snapshot.video_time,
            self.snapshot.playback_rate,
            frame_secs,
        );
        // mpv covers the picture rect rather than the card: the host paints
        // that same rect when an overlay takes the picture, so taking turns
        // cannot resize or move it.
        surface.place(
            fitted_picture(bounds, self.snapshot.aspect as f32),
            self.native_visible,
        );
        let stale = self.native_visible
            && !self.native_subs_inflight
            && surface.subtitle_revision() != Some(self.snapshot.revision);
        if stale {
            self.request_native_subtitles(cx);
        }
    }

    /// Asks the host for the current subtitles as a temporary ASS file and loads
    /// them into mpv. One request is in flight at a time.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn request_native_subtitles(&mut self, cx: &mut Context<Self>) {
        self.native_subs_inflight = true;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge.native_subtitles().await;
            let _ = weak.update(cx, |view, cx| {
                view.native_subs_inflight = false;
                match result {
                    Ok((path, revision)) => {
                        if let Some(surface) = view.mpv_surface.as_mut() {
                            if let Err(error) = surface.load_subtitles(&path, revision) {
                                view.status = format!("原生字幕不可用：{error}");
                            }
                        }
                    }
                    Err(error) => view.status = format!("读取字幕失败：{error:#}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn sync_native_video(&mut self, _window: &Window, _cx: &mut Context<Self>) {}

    /// Reads the newest frame from the host, starting a fresh retry budget.
    fn refresh_video_frame(&mut self, cx: &mut Context<Self>) {
        self.video_view.retries = 25;
        self.fetch_video_frame(cx);
    }

    /// One `video_frame` request at a time. With no video the picture is dropped.
    fn fetch_video_frame(&mut self, cx: &mut Context<Self>) {
        // While mpv holds the picture, it also decodes the frame the host clock
        // points at; fetching pixels here as well would send every frame twice
        // over the bridge for nothing.
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if self.native_visible && (self.snapshot.playing || self.snapshot.audio.playing) {
            return;
        }
        if !self.snapshot.video {
            if self.video_view.image.is_some() {
                self.reset_video_view(cx);
                cx.notify();
            }
            return;
        }
        // The popped-out window paints the picture; fetching it here as well
        // would send every frame twice over the bridge for nothing.
        if self.video_popped && self.detached.is_none() {
            return;
        }
        if self.video_view.inflight {
            return;
        }
        self.video_view.inflight = true;
        let since = self.video_view.serial;
        let epoch = self.video_epoch;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge.video_frame(since).await;
            let _ = weak.update(cx, |view, cx| {
                view.video_view.inflight = false;
                match result {
                    Ok((head, pixels)) => view.apply_video_frame(head, pixels, epoch, cx),
                    Err(error) => {
                        view.status = format!("读取视频画面失败：{error:#}");
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn apply_video_frame(
        &mut self,
        head: VideoFrameHead,
        pixels: Vec<u8>,
        epoch: u64,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.video_epoch {
            // The video changed while the frame was in flight.
            self.fetch_video_frame(cx);
            return;
        }
        if !pixels.is_empty() {
            let Some(image) = image::RgbaImage::from_raw(head.width, head.height, pixels) else {
                self.status = "视频画面数据与尺寸不符".into();
                cx.notify();
                return;
            };
            let image =
                std::sync::Arc::new(gpui_kit::RenderImage::new([VideoImageFrame::new(image)]));
            if let Some(old) = self.video_view.image.replace(image) {
                cx.drop_image(old, None);
            }
            self.video_view.serial = head.serial;
            self.video_view.frame = head.frame;
            self.video_view.source_size = Some((head.width, head.height));
            cx.notify();
        } else if !head.unchanged {
            // The host dropped its frame, e.g. while a new video loads.
            if let Some(old) = self.video_view.image.take() {
                cx.drop_image(old, None);
            }
            self.video_view.serial = head.serial;
            self.video_view.frame = -1;
            cx.notify();
        }
        // While playing, keep asking for the newest frame instead of waiting
        // for the next playback poll: the poll also carries the document and is
        // deliberately slower than the video, so letting it gate the picture
        // turns a fast video into a low-frame-rate one.
        let playing = self.snapshot.playing || self.snapshot.audio.playing;
        if playing {
            // The retry budget is for seeks, not for continuous playback.
            self.video_view.retries = 25;
        }
        if (playing || self.video_view.frame != self.snapshot.frame) && self.video_view.retries > 0
        {
            if !playing {
                self.video_view.retries -= 1;
            }
            let weak = cx.weak_entity();
            cx.spawn(async move |_, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(if playing { 10 } else { 40 }))
                    .await;
                let _ = weak.update(cx, |view, cx| view.fetch_video_frame(cx));
            })
            .detach();
        }
    }

    /// The preview picture at the display aspect ratio, capped in width. The
    /// frame is a GPU texture painted here; the wheel zooms it and the left or
    /// middle button pans it.
    fn video_region(&self, fill: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(cx);
        let aspect = if self.snapshot.aspect > 0. {
            self.snapshot.aspect as f32
        } else {
            16. / 9.
        };
        let bounds = self.video_view.bounds.clone();
        let zoom = self.video_view.zoom;
        let (pan_x, pan_y) = self.video_view.pan;
        let mut area = div()
            .relative()
            .overflow_hidden()
            .w_full()
            .bg(p.inset)
            .when(fill, |area| area.flex_1().min_h_0())
            .when(!fill, |area| {
                // A definite width keeps the aspect ratio from measuring
                // against the card's full width, which would leave the card
                // hundreds of pixels taller than the picture it holds.
                area.w(px(560.)).max_w(gpui_kit::relative(1.)).mx_auto().aspect_ratio(aspect)
            })
            .on_scroll_wheel(
                cx.listener(|view, event: &gpui_kit::ScrollWheelEvent, _, cx| {
                    let delta = match event.delta {
                        gpui_kit::ScrollDelta::Pixels(delta) => f32::from(delta.y) / 40.,
                        gpui_kit::ScrollDelta::Lines(delta) => delta.y,
                    };
                    if delta != 0. {
                        view.video_zoom_by(delta, cx);
                    }
                }),
            )
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, event: &gpui_kit::MouseDownEvent, _, _| {
                    if view.video_view.zoom <= 1. {
                        return;
                    }
                    view.video_view.pan_drag = Some((
                        gpui_kit::MouseButton::Left,
                        f32::from(event.position.x),
                        f32::from(event.position.y),
                        view.video_view.pan,
                    ));
                }),
            )
            .on_mouse_down(
                gpui_kit::MouseButton::Middle,
                cx.listener(|view, event: &gpui_kit::MouseDownEvent, _, _| {
                    view.video_view.pan_drag = Some((
                        gpui_kit::MouseButton::Middle,
                        f32::from(event.position.x),
                        f32::from(event.position.y),
                        view.video_view.pan,
                    ));
                }),
            )
            .on_mouse_move(
                cx.listener(|view, event: &gpui_kit::MouseMoveEvent, _, cx| {
                    view.video_view.pointer =
                        Some((f32::from(event.position.x), f32::from(event.position.y)));
                    let Some((button, x, y, origin)) = view.video_view.pan_drag else {
                        return;
                    };
                    if event.pressed_button != Some(button) {
                        view.video_view.pan_drag = None;
                        return;
                    }
                    let area = *view.video_view.bounds.borrow();
                    let width = f32::from(area.size.width).max(1.);
                    let height = f32::from(area.size.height).max(1.);
                    view.video_view.pan = (
                        origin.0 + (f32::from(event.position.x) - x) / width,
                        origin.1 + (f32::from(event.position.y) - y) / height,
                    );
                    cx.notify();
                }),
            )
            .on_mouse_up(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, _, _, _| view.video_view.pan_drag = None),
            )
            .on_mouse_up(
                gpui_kit::MouseButton::Middle,
                cx.listener(|view, _, _, _| view.video_view.pan_drag = None),
            )
            .on_mouse_up_out(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, _, _, _| view.video_view.pan_drag = None),
            )
            .on_mouse_up_out(
                gpui_kit::MouseButton::Middle,
                cx.listener(|view, _, _, _| view.video_view.pan_drag = None),
            )
            .child(
                canvas(
                    move |area, _, _| *bounds.borrow_mut() = area,
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        if !self.snapshot.video {
            // No picture yet: the placeholder takes the whole area instead of
            // the card collapsing to its title and transport rows.
            area = area.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .v_flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(IconName::Play).size(px(26.)).text_color(p.muted))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(p.muted)
                                    .child("打开视频后在预览区检查字幕位置"),
                            )
                            .child(
                                Button::new("empty-open-video")
                                    .rounded(gpui_kit::px(999.))
                                    .small()
                                    .px(gpui_kit::px(13.))
                                    .py(gpui_kit::px(3.))
                                    .label("打开视频")
                                    .child(Icon::new(IconName::FolderOpen).size(px(15.)))
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.pick_file("video", cx)),
                                    ),
                            ),
                    ),
            );
        }
        if let Some(image) = &self.video_view.image {
            // Fitted, the picture covers the rect mpv's window is placed on
            // rather than whatever `Contain` rounds to, so the two renderers
            // swap without moving a pixel.
            let fitted = fill && zoom == 1. && pan_x == 0. && pan_y == 0.;
            let picture = fitted_picture(
                gpui_kit::Bounds {
                    origin: gpui_kit::point(px(0.), px(0.)),
                    size: self.video_view.bounds.borrow().size,
                },
                aspect,
            );
            let element = gpui_kit::img(image.clone()).absolute();
            area = area.child(if fitted {
                element
                    .left(picture.origin.x)
                    .top(picture.origin.y)
                    .w(picture.size.width)
                    .h(picture.size.height)
                    .object_fit(ObjectFit::Fill)
            } else {
                element
                    .left(gpui_kit::relative((1. - zoom) / 2. + pan_x))
                    .top(gpui_kit::relative((1. - zoom) / 2. + pan_y))
                    .w(gpui_kit::relative(zoom))
                    .h(gpui_kit::relative(zoom))
                    .object_fit(if fill {
                        ObjectFit::Contain
                    } else {
                        ObjectFit::Fill
                    })
            });
        }
        if self.snapshot.video && self.snapshot.video_overscan {
            // Broadcast safe areas, as the wx display drew them: the outer box
            // is action-safe (90%), the inner one title-safe (80%).
            for inset in [0.05_f32, 0.10] {
                area = area.child(
                    div()
                        .absolute()
                        .left(gpui_kit::relative(inset))
                        .top(gpui_kit::relative(inset))
                        .w(gpui_kit::relative(1. - inset * 2.))
                        .h(gpui_kit::relative(1. - inset * 2.))
                        .border_1()
                        .border_color(p.muted.opacity(0.55)),
                );
            }
        }
        if self.snapshot.video && self.active_panel == Panel::Visual {
            area = area.child(self.visual.overlay(
                aspect,
                zoom,
                (pan_x, pan_y),
                self.snapshot.visual_autohide,
                cx,
            ));
        }
        area
    }

    /// Asks the host for the waveform slices the view is missing, at most a few
    /// at a time. Each slice is a fraction of the viewport and is kept once
    /// decoded, so scrolling a long recording reuses everything already seen
    /// instead of re-rendering the whole width on every step.
    fn refresh_audio(&mut self, cx: &mut Context<Self>) {
        if !self.snapshot.audio.loaded {
            return;
        }
        let marker_view = (
            self.snapshot.audio.epoch,
            self.audio.view_start,
            self.audio.span,
        );
        if !self.audio.marker_loading && self.audio.marker_view != Some(marker_view) {
            self.audio.marker_loading = true;
            let bridge = self.bridge.clone();
            let weak = cx.weak_entity();
            cx.spawn(async move |_, cx| {
                let result = bridge
                    .call(json!({"op":"audio_action", "action":"view", "start":marker_view.1, "span":marker_view.2}))
                    .await
                    .and_then(|value| serde_json::from_value::<AudioState>(value).context("invalid audio markers"));
                let _ = weak.update(cx, |view, cx| {
                    view.audio.marker_loading = false;
                    match result {
                        Ok(state) => {
                            if state.epoch == marker_view.0 && state.epoch == view.snapshot.audio.epoch {
                                view.audio.marker_view = Some(marker_view);
                                if (view.audio.view_start, view.audio.span) == (marker_view.1, marker_view.2) {
                                    view.snapshot.audio.markers = state.markers;
                                }
                            }
                            // A scroll during the request sends only its newest viewport.
                            view.refresh_audio(cx);
                        }
                        Err(error) => view.status = format!("音频标记更新失败：{error:#}"),
                    }
                    cx.notify();
                });
            }).detach();
        }
        let cached = &self.audio.tiles;
        self.audio
            .tile_inflight
            .retain(|key| !cached.iter().any(|tile| tile.key == *key));
        let sent = self.audio.tile_inflight.len();
        if sent >= TILE_INFLIGHT_LIMIT {
            return;
        }
        let (epoch, revision) = (
            self.snapshot.audio.epoch,
            self.snapshot.audio.render_revision,
        );
        // The slice bitmap is rendered at the track's own measured height, so the
        // spectrum fills the container exactly instead of taking on the bitmap's
        // own aspect ratio, and a taller track is re-rendered rather than scaled.
        let bounds = *self.audio.bounds.borrow();
        let track = f32::from(bounds.size.height);
        if track <= 0.5 {
            // The track has not been laid out yet; the first paint records its
            // height and the next refresh asks for slices at that size.
            return;
        }
        let height = audio_panel::track_pixels(track);
        let width = f32::from(bounds.size.width).max(1.);
        let view_start = self.audio.view_start;
        let view_end = view_start + self.audio.span;
        let center = view_start + self.audio.span / 2;
        let mut missing: Vec<_> = self
            .audio
            .wanted_tiles(epoch, revision, self.snapshot.audio.duration, height, width)
            .into_iter()
            .filter(|key| self.audio.cached_tile(key).is_none())
            .filter(|key| !self.audio.tile_inflight.contains(key))
            .collect();
        // Visible slices first, nearest the centre of the view first; the spare
        // slices on either side only prefetch.
        missing.sort_by_key(|key| {
            let visible = key.start() < view_end && key.start() + key.span > view_start;
            (!visible, (key.start() + key.span / 2 - center).abs())
        });
        missing.truncate(TILE_INFLIGHT_LIMIT - sent);
        for key in missing {
            self.audio.tile_inflight.push(key);
            let bridge = self.bridge.clone();
            let weak = cx.weak_entity();
            cx.spawn(async move |_, cx| {
                let result = bridge
                    .audio_render(json!({"op":"audio_render", "start":key.start(), "span":key.span, "mode":key.mode, "gain":key.gain, "width":key.width, "height":key.height}))
                    .await
                    .and_then(|(reply, pixels)| {
                        if i64::from(reply.width) != key.width || i64::from(reply.height) != key.height {
                            return Err(anyhow!(
                                "audio render came back {}x{} for a {}x{} request",
                                reply.width,
                                reply.height,
                                key.width,
                                key.height
                            ));
                        }
                        let (epoch, revision) = (reply.state.epoch, reply.state.render_revision);
                        let start = reply.view_start.round() as i64;
                        let image = reply.into_render_image(pixels)?;
                        Ok((epoch, revision, start, image))
                    });
                let _ = weak.update(cx, |view, cx| {
                    view.audio.tile_inflight.retain(|pending| *pending != key);
                    match result {
                        // A slice is drawn where the host actually started it,
                        // so one that lands after the view moved on is still
                        // useful the next time that stretch of time shows up.
                        Ok((epoch, revision, start, image))
                            if epoch == view.snapshot.audio.epoch
                                && revision == view.snapshot.audio.render_revision =>
                        {
                            if let Some(image) = image {
                                view.audio.store_tile(key, start, image, cx);
                            }
                            view.refresh_audio(cx);
                        }
                        // Stale generation: ignored, but the queue keeps moving.
                        Ok(_) => view.refresh_audio(cx),
                        // A failed render is not chained again here, so a broken
                        // slice cannot spin; the next snapshot or gesture retries.
                        Err(error) => {
                            view.status = format!("音频画面生成失败：{error:#}");
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn begin_audio_drag(
        &mut self,
        ms: i64,
        right: bool,
        control: bool,
        alt: bool,
        cx: &mut Context<Self>,
    ) {
        if self.audio.drag_loading {
            return;
        }
        self.audio.dragging = true;
        self.audio.finish_drag = false;
        self.audio.drag_moved = false;
        self.audio.pending_ms = None;
        self.audio.drag_loading = true;
        let sensitivity = (self.audio.span * self.snapshot.audio.drag_sensitivity
            / f32::from(self.audio.bounds.borrow().size.width).max(1.) as i64)
            .clamp(1, 10000);
        let snap = if self.snapshot.audio.snap != self.audio.snap_inverted {
            (self.audio.span * self.snapshot.audio.snap_distance
                / f32::from(self.audio.bounds.borrow().size.width).max(1.) as i64)
                .clamp(0, 10000)
        } else {
            0
        };
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge.call(json!({"op":"audio_action", "action":"down", "ms":ms, "right":right, "control":control, "alt":alt, "sensitivity":sensitivity, "snap":snap})).await;
            let _ = weak.update(cx, |view, cx| {
                view.audio.drag_loading = false;
                match result {
                    Ok(reply) => {
                        view.audio.gesture = reply["gesture"].as_u64();
                        if let Ok(state) = serde_json::from_value::<AudioState>(reply) { view.snapshot.audio = state; }
                        view.flush_audio_drag(cx);
                    }
                    Err(error) => { view.audio.dragging = false; view.status = format!("音频选区失败：{error}"); }
                }
                cx.notify();
            });
        }).detach();
    }

    fn flush_audio_drag(&mut self, cx: &mut Context<Self>) {
        if self.audio.drag_loading {
            return;
        }
        let (Some(gesture), Some(ms)) = (self.audio.gesture, self.audio.pending_ms.take()) else {
            return;
        };
        self.audio.drag_loading = true;
        let finished = self.audio.finish_drag;
        let dragged = self.audio.drag_moved;
        if finished {
            self.audio.dragging = false;
        }
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        let snap = if self.snapshot.audio.snap != self.audio.snap_inverted {
            (self.audio.span * self.snapshot.audio.snap_distance
                / f32::from(self.audio.bounds.borrow().size.width).max(1.) as i64)
                .clamp(0, 10000)
        } else {
            0
        };
        cx.spawn(async move |_, cx| {
            let result = bridge.call(json!({"op":"audio_action", "action":if finished {"up"} else {"drag"}, "ms":ms, "gesture":gesture, "snap":snap, "dragged":dragged})).await;
            let _ = weak.update(cx, |view, cx| {
                view.audio.drag_loading = false;
                match result {
                    Ok(reply) => { if let Ok(state) = serde_json::from_value::<AudioState>(reply) { view.snapshot.audio = state; } }
                    Err(error) => { view.audio.dragging = false; view.status = format!("音频拖动失败：{error}"); }
                }
                if finished { view.audio.gesture = None; }
                else { view.flush_audio_drag(cx); }
                cx.notify();
            });
        }).detach();
    }

    fn spelling_request(&mut self, mut request: Value, cx: &mut Context<Self>) {
        if self.spelling.loading {
            return;
        }
        let action = request["action"].as_str().unwrap_or("state").to_owned();
        request["op"] = json!("spelling");
        if action == "configure" {
            for (key, value) in [
                ("language", json!(self.spelling.state.language)),
                ("skip_comments", json!(self.spelling.state.skip_comments)),
                ("skip_uppercase", json!(self.spelling.state.skip_uppercase)),
            ] {
                if request.get(key).is_none() {
                    request[key] = value;
                }
            }
        }
        if matches!(
            action.as_str(),
            "next" | "ignore_all" | "replace" | "replace_all" | "add"
        ) {
            request["id"] = json!(self.spelling.state.id);
            request["start"] = json!(self.spelling.state.start);
            request["expected"] = json!(self.spelling.state.word);
            request["replacement"] = json!(self.spelling.replacement.read(cx).value().to_string());
        }
        let draft = if action == "state" {
            None
        } else {
            self.pending_draft(cx)
        };
        self.spelling.loading = true;
        self.spelling.error = None;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = async {
                if let Some(draft) = draft {
                    bridge.call(draft).await?;
                }
                let state = serde_json::from_value::<SpellState>(bridge.call(request).await?)?;
                let snapshot = bridge.snapshot().await?;
                Ok::<_, anyhow::Error>((state, snapshot))
            }
            .await;
            let _ = weak.update(cx, |view, cx| {
                view.spelling.loading = false;
                match result {
                    Ok((state, snapshot)) => {
                        view.status = if state.changed > 0 {
                            format!("已替换 {} 处，可撤销", state.changed)
                        } else {
                            "拼写检查已同步".into()
                        };
                        view.spelling.dictionary_checked = action == "state"
                            && state.dictionary_word == view.spelling.dictionary_word;
                        view.spelling.receive(state, action != "state");
                        view.spelling.confirm_remove = false;
                        if action == "remove" {
                            view.spelling.dictionary_checked = false;
                            view.spelling.dictionary_word.clear();
                            view.spelling.state.can_remove = false;
                        }
                        view.apply_snapshot(snapshot, cx);
                        view.pending_text_selection = Some(view.snapshot.text_selection);
                    }
                    Err(error) => {
                        view.spelling.error = Some(error.to_string());
                        view.status = format!("拼写检查失败：{error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn pick_encoded_subtitle(&mut self, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            if let Some(file) = rfd::AsyncFileDialog::new()
                .set_title("选择要指定编码的字幕")
                .add_filter("字幕文件", &["ass", "ssa", "srt", "txt", "vtt"])
                .pick_file()
                .await
            {
                let _ = weak.update(cx, |view, cx| {
                    view.file_tools_request(
                        json!({"action":"inspect", "path":file.path().to_string_lossy()}),
                        cx,
                    )
                });
            }
        })
        .detach();
    }

    fn file_tools_request(&mut self, mut request: Value, cx: &mut Context<Self>) {
        if self.files.loading {
            return;
        }
        request["op"] = json!("file_tools");
        self.files.loading = true;
        self.files.error = None;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge.call(request).await.and_then(|value| {
                serde_json::from_value::<FileState>(value).context("invalid file tool state")
            });
            let _ = weak.update(cx, |view, cx| {
                view.files.loading = false;
                match result {
                    Ok(state) => view.files.receive(state),
                    Err(error) => view.files.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_file_tools_selection(&mut self, cx: &mut Context<Self>) {
        let (path, encoding, recovery) = if self.files.page == FilePage::Encoding {
            (self.files.path.clone(), self.files.encoding.clone(), false)
        } else {
            (self.files.version.clone(), String::new(), true)
        };
        if path.is_empty() {
            return;
        }
        self.invoke(
            json!({"op":"open", "kind":"subtitle", "path":path, "encoding":encoding, "recovery":recovery}),
            cx,
        );
    }

    fn pick_preference_directory(&mut self, key: String, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            if let Some(directory) = rfd::AsyncFileDialog::new().pick_folder().await {
                let _ = weak.update(cx, |view, cx| {
                    view.preferences
                        .drafts
                        .insert(key.clone(), directory.path().to_string_lossy().into_owned());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn preferences_request(&mut self, mut request: Value, cx: &mut Context<Self>) {
        if self.preferences.loading {
            return;
        }
        request["op"] = json!("preferences");
        self.preferences.loading = true;
        self.preferences.error = None;
        let sent = request["fields"].clone();
        let reset = request["names"].clone();
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = async {
                let state = serde_json::from_value::<PreferenceState>(bridge.call(request).await?)?;
                let snapshot = bridge.snapshot().await?;
                Ok::<_, anyhow::Error>((state, snapshot))
            }
            .await;
            let _ = weak.update(cx, |view, cx| {
                view.preferences.loading = false;
                match result {
                    Ok((state, snapshot)) => {
                        if let Some(names) = reset.as_array() {
                            for name in names {
                                if let Some(name) = name.as_str() {
                                    view.preferences.drafts.remove(name);
                                }
                            }
                        }
                        view.preferences.receive(state, &sent);
                        view.preferences.confirm_reset = false;
                        view.audio.gesture = None;
                        view.apply_snapshot(snapshot, cx);
                        view.refresh_catalog(cx);
                        view.status = "偏好设置已同步".into();
                    }
                    Err(error) => {
                        view.preferences.error = Some(error.to_string());
                        view.status = format!("设置未应用：{error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn hotkey_request(&mut self, mut request: Value, cx: &mut Context<Self>) {
        if self.hotkeys.loading {
            return;
        }
        request["op"] = json!("hotkeys");
        self.hotkeys.loading = true;
        self.hotkeys.error = None;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge.call(request).await.and_then(|state| {
                serde_json::from_value::<HotkeyState>(state).context("invalid hotkey response")
            });
            let _ = weak.update(cx, |view, cx| {
                view.hotkeys.loading = false;
                match result {
                    Ok(state) => {
                        view.hotkeys.bindings = state.bindings;
                        view.hotkeys.confirm_reset = false;
                        view.hotkeys.replace = false;
                        view.refresh_catalog(cx);
                        view.status = match (state.imported, state.skipped) {
                            (Some(imported), Some(skipped)) if skipped > 0 => {
                                format!("已导入 {imported} 条快捷键，跳过 {skipped} 条")
                            }
                            (Some(imported), _) => format!("已导入 {imported} 条快捷键"),
                            _ => "快捷键已同步".into(),
                        };
                    }
                    Err(error) => {
                        view.hotkeys.error = Some(error.to_string());
                        view.status = format!("快捷键未应用：{error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn hotkey_import(&mut self, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("导入旧版快捷键")
                .add_filter("Neo Aegisub 快捷键文件", &["json"])
                .pick_file()
                .await;
            let Some(file) = picked else {
                return;
            };
            let path = file.path().to_string_lossy().to_string();
            let _ = weak.update(cx, |view, cx| {
                view.hotkey_request(json!({"action":"import","path":path}), cx);
            });
        })
        .detach();
    }

    fn pick_attachments(&mut self, kind: &'static str, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let dialog = rfd::AsyncFileDialog::new();
            let dialog = if kind == "font" {
                dialog.add_filter("字体文件", &["ttf", "ttc", "otf", "pfb"])
            } else {
                dialog.add_filter(
                    "图像文件",
                    &["png", "jpg", "jpeg", "bmp", "gif", "ico", "wmf"],
                )
            };
            if let Some(files) = dialog.pick_files().await {
                let paths: Vec<_> = files
                    .iter()
                    .map(|file| file.path().to_string_lossy().into_owned())
                    .collect();
                let _ = weak.update(cx, |view, cx| {
                    view.attachment_request(json!({"action":"add","group":kind,"paths":paths}), cx)
                });
            }
        })
        .detach();
    }

    fn extract_attachments(&mut self, cx: &mut Context<Self>) {
        let items = self.attachments.selected_items();
        if items.is_empty() {
            return;
        }
        let revision = self.attachments.state.revision;
        let file = self
            .attachments
            .state
            .items
            .iter()
            .find(|item| self.attachments.selected.contains(&item.index))
            .unwrap();
        let name = if file.safe_name {
            file.filename.clone()
        } else {
            "attachment".into()
        };
        let directory = items.len() > 1;
        let names: Vec<_> = self
            .attachments
            .state
            .items
            .iter()
            .filter(|item| self.attachments.selected.contains(&item.index))
            .map(|item| item.filename.clone())
            .collect();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let path = if directory { rfd::AsyncFileDialog::new().pick_folder().await.map(|file| file.path().to_owned()) }
            else { rfd::AsyncFileDialog::new().set_file_name(name).save_file().await.map(|file| file.path().to_owned()) };
            if let Some(path) = path {
                let exists = if directory { names.iter().any(|name| path.join(name).exists()) } else { path.exists() };
                let overwrite = if exists {
                    rfd::AsyncMessageDialog::new().set_title("替换已有附件文件").set_description("目标文件已存在，是否替换所选附件对应的文件？").set_buttons(rfd::MessageButtons::YesNo).show().await == rfd::MessageDialogResult::Yes
                } else { false };
                if exists && !overwrite { return; }
                let _ = weak.update(cx, |view, cx| view.attachment_request(json!({"action":"extract","items":items,"revision":revision,"path":path.to_string_lossy(),"directory":directory,"overwrite":overwrite}), cx));
            }
        }).detach();
    }

    fn attachment_request(&mut self, mut request: Value, cx: &mut Context<Self>) {
        if self.attachments.loading {
            return;
        }
        request["op"] = json!("attachments");
        self.attachments.loading = true;
        self.attachments.error = None;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        let draft = if request["action"] == "state" {
            self.pending_draft(cx)
        } else {
            None
        };
        cx.spawn(async move |_, cx| {
            let result = async {
                if let Some(draft) = draft {
                    bridge.call(draft).await?;
                }
                let state = serde_json::from_value::<AttachmentState>(bridge.call(request).await?)?;
                let snapshot = bridge.snapshot().await?;
                Ok::<_, anyhow::Error>((state, snapshot))
            }
            .await;
            let _ = weak.update(cx, |view, cx| {
                view.attachments.loading = false;
                match result {
                    Ok((state, snapshot)) => {
                        view.attachments.receive(state);
                        view.apply_snapshot(snapshot, cx);
                        view.status = "附件已同步".into();
                    }
                    Err(error) => {
                        view.attachments.error = Some(error.to_string());
                        view.status = format!("附件操作失败：{error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn video_settings_request(&mut self, action: &'static str, cx: &mut Context<Self>) {
        if self.video_settings.loading {
            return;
        }
        let mut request = json!({"op":"video_settings", "action":action});
        let fields: &[&str] = match action {
            "dummy" => &["width", "height", "length"],
            "jump_frame" => &["frame"],
            _ => &[],
        };
        for &field in fields {
            match self.video_settings.inputs[field]
                .read(cx)
                .value()
                .trim()
                .parse::<i64>()
            {
                Ok(value) => request[field] = json!(value),
                Err(_) => {
                    self.video_settings.error = Some(format!("{field}：请输入有效整数"));
                    cx.notify();
                    return;
                }
            }
        }
        if action == "dummy" {
            request["fps"] = json!(
                self.video_settings.inputs["fps"]
                    .read(cx)
                    .value()
                    .to_string()
            );
            request["color"] = json!(
                self.video_settings.inputs["color"]
                    .read(cx)
                    .value()
                    .to_string()
            );
            request["pattern"] = json!(self.video_settings.pattern);
        }
        if action == "aspect" {
            request["value"] = json!(
                self.video_settings.inputs["aspect"]
                    .read(cx)
                    .value()
                    .to_string()
            );
        }
        if action == "jump_time" {
            match parse_time(&self.video_settings.inputs["time"].read(cx).value()) {
                Ok(value) => request["time"] = json!(value),
                Err(error) => {
                    self.video_settings.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        }
        self.video_settings.loading = true;
        self.video_settings.error = None;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        let draft = self.pending_draft(cx);
        cx.spawn(async move |_, cx| {
            let result = async {
                if let Some(draft) = draft {
                    bridge.call(draft).await?;
                }
                let state = serde_json::from_value::<VideoSettings>(bridge.call(request).await?)?;
                let snapshot = bridge.snapshot().await?;
                Ok::<_, anyhow::Error>((state, snapshot))
            }
            .await;
            let _ = weak.update(cx, |view, cx| {
                view.video_settings.loading = false;
                match result {
                    Ok((state, snapshot)) => {
                        view.video_settings.receive(state);
                        view.apply_snapshot(snapshot, cx);
                        view.status = "视频参数已同步".into();
                    }
                    Err(error) => {
                        view.video_settings.error = Some(error.to_string());
                        view.status = format!("视频操作失败：{error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn paste_request(&mut self, apply: bool, cx: &mut Context<Self>) {
        if self.paste.loading {
            return;
        }
        let mut request = json!({"op":"paste_over"});
        if apply {
            request["fields"] = json!(self.paste.fields);
        }
        self.paste.loading = true;
        self.paste.error = None;
        let draft = self.pending_draft(cx);
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = async {
                if let Some(draft) = draft {
                    bridge.call(draft).await?;
                }
                let state = serde_json::from_value::<PasteReply>(bridge.call(request).await?)?;
                let snapshot = bridge.snapshot().await?;
                Ok::<_, anyhow::Error>((state, snapshot))
            }
            .await;
            let _ = weak.update(cx, |view, cx| {
                view.paste.loading = false;
                match result {
                    Ok((state, snapshot)) => {
                        if !view.paste.initialized && state.fields.len() == 11 {
                            view.paste.fields.copy_from_slice(&state.fields);
                            view.paste.initialized = true;
                        }
                        if let Some(changed) = state.changed {
                            view.paste.message = format!("已覆盖 {changed} 行，可撤销");
                            view.status = view.paste.message.clone();
                        }
                        view.apply_snapshot(snapshot, cx);
                    }
                    Err(error) => {
                        view.paste.error = Some(error.to_string());
                        view.status = format!("覆盖粘贴失败：{error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_search(&mut self, selection: bool, cx: &mut Context<Self>) {
        self.open_tool(Panel::Search, cx, move |view, cx| {
            let changed = view.find_panel.selection_mode != selection;
            view.find_panel.selection_mode = selection;
            if changed {
                view.find_panel.initialized = false;
            }
            view.find_panel.focus_pending = true;
            view.search_request("state", cx);
        });
    }

    /// Shows a tool page the way the sidebar does: in its own window, so the
    /// editor stays where it is. A window that already is that tool (or the
    /// subtitle page itself) configures in place.
    fn open_tool(
        &mut self,
        panel: Panel,
        cx: &mut Context<Self>,
        configure: impl FnOnce(&mut Workspace, &mut Context<Workspace>) + 'static,
    ) {
        if panel == Panel::Subtitles || self.detached == Some(panel) {
            self.active_panel = panel;
            configure(self, cx);
            cx.notify();
            return;
        }
        if let Err(error) = focus_or_open_panel_window(cx, self.bridge.clone(), panel) {
            eprintln!("Could not open tool window: {error:#}");
            return;
        }
        let Some(view) = PANEL_VIEWS.with(|views| views.borrow().get(&panel).cloned()) else {
            eprintln!("Tool window opened but no view is registered for it");
            return;
        };
        let _ = view.update(cx, |view, cx| {
            view.active_panel = panel;
            configure(view, cx);
            cx.notify();
        });
    }

    fn search_request(&mut self, action: &'static str, cx: &mut Context<Self>) {
        if self.find_panel.loading {
            return;
        }
        let mut request = json!({"op":"search", "action":action});
        if action != "state" {
            request["find"] = json!(self.find_panel.find.read(cx).value().to_string());
            request["replace_with"] = json!(self.find_panel.replace.read(cx).value().to_string());
            request["field"] = json!(self.find_panel.field);
            request["mode"] = json!(self.find_panel.mode);
            request["selection_action"] = json!(self.find_panel.selection_action);
            for (&key, value) in &self.find_panel.options {
                request[key] = json!(value);
            }
        }
        self.find_panel.loading = true;
        self.find_panel.error = None;
        let draft = self.pending_draft(cx);
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = async {
                if let Some(draft) = draft {
                    bridge.call(draft).await?;
                }
                let mut listing = request.clone();
                let wants_list = !matches!(action, "state" | "select" | "preview");
                if wants_list {
                    listing["action"] = json!("list");
                }
                let mut reply = serde_json::from_value::<SearchReply>(bridge.call(request).await?)?;
                if wants_list {
                    if let Ok(list) = bridge.call(listing).await {
                        if let Ok(list) = serde_json::from_value::<SearchReply>(list) {
                            reply.items = list.items;
                            reply.list_count = list.count.unwrap_or(0) as i64;
                        }
                    }
                }
                let snapshot = bridge.snapshot().await?;
                Ok::<_, anyhow::Error>((reply, snapshot))
            }
            .await;
            let _ = weak.update(cx, |view, cx| {
                view.find_panel.loading = false;
                match result {
                    Ok((reply, snapshot)) => {
                        view.find_panel.items = reply.items.clone();
                        view.find_panel.items_count = reply.list_count;
                        if action != "state" {
                            view.find_panel.message = match action {
                                "select" | "preview" => format!(
                                    "匹配 {} 行：{}",
                                    reply.count.unwrap_or(0),
                                    reply
                                        .rows
                                        .iter()
                                        .map(ToString::to_string)
                                        .collect::<Vec<_>>()
                                        .join("、")
                                ),
                                "replace_all" => {
                                    format!("已替换 {} 处，可撤销", reply.count.unwrap_or(0))
                                }
                                _ if reply.found == Some(true) => format!(
                                    "{}第 {} 行",
                                    if reply.replaced == Some(true) {
                                        "已替换，并找到"
                                    } else {
                                        "找到"
                                    },
                                    reply.row.unwrap_or(0)
                                ),
                                _ if reply.replaced == Some(true) => {
                                    "已替换，未找到其他匹配".into()
                                }
                                _ => "没有找到匹配项".into(),
                            };
                        }
                        view.find_panel.receive(reply);
                        view.apply_snapshot(snapshot, cx);
                        if matches!(action, "find" | "replace") {
                            view.pending_text_selection = Some(view.snapshot.text_selection);
                        }
                        view.status = if action == "state" {
                            "检索参数已同步".into()
                        } else {
                            view.find_panel.message.clone()
                        };
                    }
                    Err(error) => {
                        view.find_panel.error = Some(error.to_string());
                        view.status = format!("检索失败：{error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn processor_request(&mut self, action: &'static str, cx: &mut Context<Self>) {
        if self.processor.loading {
            return;
        }
        let mut request = json!({"op":"timing_processor", "action":action});
        if action == "apply" {
            for &(key, label) in timing_processor_panel::NUMBERS {
                let maximum = if key == "adjacent_bias" {
                    100
                } else {
                    35999990
                };
                let parsed = self.processor.inputs[key]
                    .read(cx)
                    .value()
                    .trim()
                    .parse::<i64>();
                match parsed {
                    Ok(value) if (0..=maximum).contains(&value) => request[key] = json!(value),
                    _ => {
                        self.processor.error =
                            Some(format!("{label}：请输入 0 到 {maximum} 的整数"));
                        cx.notify();
                        return;
                    }
                }
            }
            for (&key, value) in &self.processor.options {
                request[key] = json!(value);
            }
            request["styles"] = json!(self.processor.selected_styles);
        }
        self.processor.loading = true;
        self.processor.error = None;
        let draft = if action == "apply" {
            self.pending_draft(cx)
        } else {
            None
        };
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = async {
                if let Some(draft) = draft {
                    bridge.call(draft).await?;
                }
                let state = serde_json::from_value::<ProcessorState>(bridge.call(request).await?)?;
                let snapshot = bridge.snapshot().await?;
                Ok::<_, anyhow::Error>((state, snapshot))
            }
            .await;
            let _ = weak.update(cx, |view, cx| {
                view.processor.loading = false;
                match result {
                    Ok((state, snapshot)) => {
                        view.status = state.changed.map_or_else(
                            || "后处理参数已同步".into(),
                            |changed| format!("时间后处理修改 {changed} 行，可撤销"),
                        );
                        view.processor.receive(state);
                        view.apply_snapshot(snapshot, cx);
                    }
                    Err(error) => {
                        view.processor.error = Some(error.to_string());
                        view.status = format!("时间后处理失败：{error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn resample_request(&mut self, action: &'static str, cx: &mut Context<Self>) {
        if self.resample.loading {
            return;
        }
        let mut request = json!({"op":"resample", "action":action});
        if action == "apply" {
            for &(key, _) in resample_panel::FIELDS {
                let value = self.resample.inputs[key]
                    .read(cx)
                    .value()
                    .trim()
                    .parse::<i64>();
                let range = if matches!(key, "left" | "right" | "top" | "bottom") {
                    -9999..=9999
                } else {
                    1..=999999
                };
                if matches!(key, "left" | "right" | "top" | "bottom") && self.resample.ar_mode != 3
                {
                    request[key] = json!(0);
                    continue;
                }
                match value {
                    Ok(value) if range.contains(&value) => request[key] = json!(value),
                    _ => {
                        self.resample.error = Some(format!(
                            "{key}：请输入 {} 到 {} 的整数",
                            range.start(),
                            range.end()
                        ));
                        cx.notify();
                        return;
                    }
                }
            }
            request["ar_mode"] = json!(self.resample.ar_mode);
            request["source_matrix"] = json!(self.resample.source_matrix);
            request["dest_matrix"] = json!(self.resample.dest_matrix);
            request["expected_source"] =
                json!([self.resample.state.source_x, self.resample.state.source_y]);
        }
        self.resample.loading = true;
        self.resample.error = None;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        let draft = self.pending_draft(cx);
        cx.spawn(async move |_, cx| {
            let result = async {
                if let Some(draft) = draft {
                    bridge.call(draft).await?;
                }
                let state = serde_json::from_value::<ResampleState>(bridge.call(request).await?)?;
                let snapshot = bridge.snapshot().await?;
                Ok::<_, anyhow::Error>((state, snapshot))
            }
            .await;
            let _ = weak.update(cx, |view, cx| {
                view.resample.loading = false;
                match result {
                    Ok((state, snapshot)) => {
                        if action == "apply" {
                            view.resample.initialized = false;
                        }
                        view.resample.receive(state);
                        view.status = if action == "apply" {
                            "重采样已应用，可撤销".into()
                        } else {
                            "重采样参数已同步".into()
                        };
                        view.apply_snapshot(snapshot, cx);
                        view.refresh_document(cx);
                    }
                    Err(error) => {
                        view.resample.error = Some(format!("重采样失败：{error}"));
                        view.status = "重采样未完成，请检查参数后重试".into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn pick_automation_script(&mut self, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            if let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter("Automation 脚本", &["lua", "moon"])
                .pick_file()
                .await
            {
                let _ = weak.update(cx, |view, cx| {
                    view.automation_request(
                        json!({"action":"add", "path":file.path().to_string_lossy()}),
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    fn automation_request(&mut self, mut request: Value, cx: &mut Context<Self>) {
        if self.automation.loading {
            return;
        }
        request["op"] = json!("automation_manager");
        self.automation.loading = true;
        self.automation.error = None;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        let draft = self.pending_draft(cx);
        cx.spawn(async move |_, cx| {
            let result = async {
                if let Some(draft) = draft {
                    bridge.call(draft).await?;
                }
                let state = serde_json::from_value::<AutomationState>(bridge.call(request).await?)?;
                let snapshot = bridge.snapshot().await?;
                Ok::<_, anyhow::Error>((state, snapshot))
            }
            .await;
            let _ = weak.update(cx, |view, cx| {
                view.automation.loading = false;
                match result {
                    Ok((state, snapshot)) => {
                        view.automation.state = state;
                        view.automation.confirm_remove = None;
                        view.status = "Automation 脚本已同步".into();
                        view.apply_snapshot(snapshot, cx);
                    }
                    Err(error) => {
                        view.automation.error = Some(error.to_string());
                        view.status = format!("Automation 操作失败：{error}");
                    }
                }
                view.refresh_catalog(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn shift_times_request(&mut self, action: &'static str, cx: &mut Context<Self>) {
        if self.timing.loading {
            return;
        }
        let mut request = json!({"op":"shift_times", "action":action});
        if action == "apply" {
            let parsed = if self.timing.by_time {
                parse_time(&self.timing.time.read(cx).value())
                    .map_err(|_| anyhow!("请输入有效的时:分:秒.百分秒"))
            } else {
                self.timing
                    .frames
                    .read(cx)
                    .value()
                    .trim()
                    .parse::<i64>()
                    .context("请输入非负整数帧数")
            };
            let amount = match parsed {
                Ok(amount) if (0..=35999990).contains(&amount) => amount,
                Ok(_) => {
                    self.timing.error = Some("平移量必须在 0 到 35999990 之间".into());
                    cx.notify();
                    return;
                }
                Err(error) => {
                    self.timing.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            };
            request["by_time"] = json!(self.timing.by_time);
            request["backward"] = json!(self.timing.backward);
            request["amount"] = json!(amount);
            request["mode"] = json!(self.timing.mode);
            request["fields"] = json!(self.timing.fields);
        }
        self.timing.loading = true;
        self.timing.error = None;
        let draft = if action == "apply" {
            self.pending_draft(cx)
        } else {
            None
        };
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = async {
                if let Some(draft) = draft {
                    bridge.call(draft).await?;
                }
                let state = serde_json::from_value::<ShiftState>(bridge.call(request).await?)?;
                let snapshot = bridge.snapshot().await?;
                Ok::<_, anyhow::Error>((state, snapshot))
            }
            .await;
            let _ = weak.update(cx, |view, cx| {
                view.timing.loading = false;
                match result {
                    Ok((state, snapshot)) => {
                        view.status = if let Some(changed) = state.changed {
                            format!("已平移 {changed} 行")
                        } else {
                            "平移历史已同步".into()
                        };
                        view.timing.receive(state);
                        view.apply_snapshot(snapshot, cx);
                    }
                    Err(error) => {
                        view.timing.error = Some(format!("平移失败：{error}"));
                        view.status = "平移未完成，请检查参数后重试".into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn styling_action(
        &mut self,
        action: &'static str,
        style: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let style = style.unwrap_or_else(|| self.styling.input.read(cx).value().to_string());
        if let Some(value) = self
            .document
            .details
            .styles
            .iter()
            .filter_map(|style| style.get("name"))
            .find(|name| name.eq_ignore_ascii_case(&style))
        {
            self.styling.pending_value = Some(value.clone());
        }
        self.invoke(json!({"op":"styling", "action":action, "id":self.snapshot.active, "style":style, "seek":self.styling.seek}), cx);
    }

    fn translation_action(&mut self, action: &'static str, cx: &mut Context<Self>) {
        if self.translation.loading {
            return;
        }
        if action == "insert_original" {
            let mut value = self.translation.editor.read(cx).value().to_string();
            let selected = self.translation.editor.read(cx).selected_range();
            if selected.end <= value.len()
                && value.is_char_boundary(selected.start)
                && value.is_char_boundary(selected.end)
            {
                value.replace_range(selected, &self.translation.state.text);
                self.translation.pending_text = Some(value);
                self.translation.focus_pending = true;
                cx.notify();
            }
            return;
        }
        self.translation.loading = true;
        let state = &self.translation.state;
        let mut request = json!({"op":"translation", "action":action, "block":state.block, "seek":self.translation.seek, "skip_whitespace":self.translation.skip_whitespace});
        if action != "start" && state.available {
            request["id"] = json!(state.id);
        }
        if matches!(action, "preview" | "commit") {
            request["text"] = json!(self.translation.editor.read(cx).value().to_string());
            request["expected"] = json!(state.text);
        }
        let draft = self.pending_draft(cx);
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = async {
                if let Some(draft) = draft {
                    bridge.call(draft).await?;
                }
                let value = bridge.call(request).await?;
                let state = serde_json::from_value::<TranslationState>(value)
                    .context("invalid translation state")?;
                let snapshot = bridge.snapshot().await?;
                Ok::<_, anyhow::Error>((state, snapshot))
            }
            .await;
            let _ = weak.update(cx, |view, cx| {
                view.translation.loading = false;
                match result {
                    Ok((state, snapshot)) => {
                        let keep = state.boundary && matches!(action, "next" | "prev");
                        if !keep {
                            view.translation.pending_text = Some(String::new());
                        }
                        view.translation.focus_pending = true;
                        view.status = if state.exhausted {
                            "已到最后一个可翻译文本块".into()
                        } else {
                            "翻译助手已同步".into()
                        };
                        view.translation.state = state;
                        view.apply_snapshot(snapshot, cx);
                    }
                    Err(error) => view.status = format!("翻译助手：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_library(&mut self, cx: &mut Context<Self>) {
        self.library_request(
            json!({"action":"list", "catalog":self.document.library.catalog}),
            cx,
        );
    }

    fn library_request(&mut self, mut payload: Value, cx: &mut Context<Self>) {
        if self.document.library_loading {
            return;
        }
        if !self.document.style_form.dirty.is_empty() && payload["action"] != "edit" {
            self.status = "请先应用或放弃当前样式的修改".into();
            cx.notify();
            return;
        }
        payload["op"] = json!("style_library");
        self.document.library_loading = true;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge.call(payload.clone()).await.and_then(|value| {
                serde_json::from_value::<LibraryDetails>(value).context("invalid style library")
            });
            let _ = weak.update(cx, |view, cx| {
                view.document.library_loading = false;
                match result {
                    Ok(mut details) => {
                        if matches!(
                            payload["action"].as_str(),
                            Some("import_preview" | "import")
                        ) {
                            view.document.import_path =
                                payload["path"].as_str().unwrap_or_default().into();
                            view.document.import_styles =
                                std::mem::take(&mut details.import_styles);
                            view.document.selected_import.clear();
                        }
                        if payload["action"] == "edit" {
                            view.document.style_form.acknowledge(&payload["fields"], cx);
                            if let Some(name) = payload["fields"]["name"].as_str() {
                                view.document.library_style = name.into();
                            }
                        }
                        if payload["action"] == "duplicate" {
                            if let Some(name) = payload["fields"]["name"].as_str() {
                                view.document.library_style = name.into();
                            }
                        }
                        if details.catalog != view.document.library.catalog {
                            view.document.selected_library.clear();
                            view.document.editing_library = false;
                        }
                        view.document.selected_library.retain(|name| {
                            details
                                .styles
                                .iter()
                                .any(|style| style.get("name") == Some(name))
                        });
                        if view.document.editing_library
                            && !details.styles.iter().any(|style| {
                                style.get("name") == Some(&view.document.library_style)
                            })
                        {
                            view.document.editing_library = false;
                        }
                        view.document.library = details;
                        view.status = "样式库已同步".into();
                        view.refresh_document(cx);
                    }
                    Err(error) => view.status = format!("样式库操作失败：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn pick_style_import(&mut self, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            if let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter("字幕样式", &["ass", "ssa", "srt"])
                .pick_file()
                .await
            {
                let _ = weak.update(cx, |view, cx| {
                    view.library_request(
                        json!({"action":"import_preview", "path":file.path().to_string_lossy()}),
                        cx,
                    )
                });
            }
        })
        .detach();
    }

    fn new_style(&mut self, cx: &mut Context<Self>) {
        if !self.document.style_form.dirty.is_empty() {
            self.status = "请先应用或放弃当前样式的修改".into();
            cx.notify();
            return;
        }
        let mut name = "新样式".to_string();
        let mut suffix = 2;
        while self
            .document
            .details
            .styles
            .iter()
            .any(|style| style.get("name") == Some(&name))
        {
            name = format!("新样式 {suffix}");
            suffix += 1;
        }
        self.document.editing_library = false;
        self.invoke(
            json!({"op":"style", "action":"create", "source":"", "fields":{"name":name}}),
            cx,
        );
    }

    fn refresh_document(&mut self, cx: &mut Context<Self>) {
        if self.document.loading {
            return;
        }
        self.document.loading = true;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge
                .call(json!({"op":"document"}))
                .await
                .and_then(|value| {
                    serde_json::from_value::<DocumentDetails>(value)
                        .context("invalid document details")
                });
            let _ = weak.update(cx, |view, cx| {
                view.document.loading = false;
                match result {
                    Ok(details) => view.document.details = details,
                    Err(error) => view.status = format!("项目属性读取失败：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn commit_style(&mut self, cx: &mut Context<Self>) {
        if self.document.editing_library {
            self.library_request(json!({"action":"edit", "catalog":self.document.library.catalog, "source":self.document.library_style, "fields":self.document.style_form.patch(cx)}), cx);
            return;
        }
        self.invoke(json!({"op":"style", "action":"edit", "source":self.document.style, "fields":self.document.style_form.patch(cx)}), cx);
    }

    fn duplicate_style(&mut self, cx: &mut Context<Self>) {
        if !self.document.style_form.dirty.is_empty() {
            self.status = "请先应用或放弃当前样式的修改".into();
            cx.notify();
            return;
        }
        if self.document.editing_library {
            let base = format!("{} 副本", self.document.library_style);
            let mut name = base.clone();
            let mut suffix = 2;
            while self
                .document
                .library
                .styles
                .iter()
                .any(|style| style.get("name") == Some(&name))
            {
                name = format!("{base} {suffix}");
                suffix += 1;
            }
            self.library_request(json!({"action":"duplicate", "catalog":self.document.library.catalog, "source":self.document.library_style, "fields":{"name":name}}), cx);
            return;
        }
        let base = format!("{} 副本", self.document.style);
        let mut name = base.clone();
        let mut suffix = 2;
        while self.document.details.styles.iter().any(|style| {
            style
                .get("name")
                .is_some_and(|existing| existing.eq_ignore_ascii_case(&name))
        }) {
            name = format!("{base} {suffix}");
            suffix += 1;
        }
        self.invoke(json!({"op":"style", "action":"create", "source":self.document.style, "fields":{"name":name}}), cx);
    }

    fn sync_display_mode(&mut self, cx: &mut Context<Self>) {
        if self.detached.is_some() {
            return;
        }
        for command in &mut Arc::make_mut(&mut self.catalog).commands {
            command.active = match command.name.as_str() {
                "app/display/subs" => !self.show_audio && !self.show_video,
                "app/display/audio_subs" => self.show_audio && !self.show_video,
                "app/display/video_subs" => !self.show_audio && self.show_video,
                "app/display/full" => self.show_audio && self.show_video,
                _ => continue,
            };
        }
        install_native_menus(&self.catalog, cx);
    }

    fn refresh_catalog(&mut self, cx: &mut Context<Self>) {
        if self.catalog_loading {
            return;
        }
        self.catalog_loading = true;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = bridge
                .call(json!({"op":"catalog"}))
                .await
                .and_then(|value| {
                    serde_json::from_value::<Catalog>(value).context("invalid command catalog")
                });
            let _ = weak.update(cx, |view, cx| {
                view.catalog_loading = false;
                match result {
                    Ok(catalog) => {
                        view.catalog = Arc::new(catalog);
                        view.sync_display_mode(cx);
                    }
                    Err(error) => view.status = format!("命令读取失败：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn commit_properties(&mut self, cx: &mut Context<Self>) {
        let current = self
            .snapshot
            .lines
            .iter()
            .find(|line| line.id == self.selected_line);
        let Some(line) = current else {
            return;
        };
        let mut fields = serde_json::Map::new();
        for (key, input) in &self.property_inputs {
            let value = input.read(cx).value().to_string();
            let parsed: Result<Value> = match *key {
                "start" | "end" => {
                    if self.time_in_frames {
                        let fps = self.snapshot.fps.unwrap_or(0.);
                        if fps <= 0. {
                            Err(anyhow::anyhow!("没有视频帧率，无法按帧输入"))
                        } else {
                            value
                                .trim()
                                .parse::<i64>()
                                .map(|frame| {
                                    Value::from((frame as f64 * 1000. / fps).round() as i64)
                                })
                                .context("请输入帧号")
                        }
                    } else {
                        parse_time(&value).map(Value::from)
                    }
                }
                "layer" | "margin_left" | "margin_right" | "margin_vertical" => {
                    value.parse::<i64>().map(Value::from).context("请输入整数")
                }
                _ => Ok(Value::from(value)),
            };
            let value = match parsed {
                Ok(value) => value,
                Err(error) => {
                    self.status = format!("{key}：{error}");
                    cx.notify();
                    return;
                }
            };
            let previous = match *key {
                "start" => json!(line.start),
                "end" => json!(line.end),
                "layer" => json!(line.layer),
                "style" => json!(line.style),
                "actor" => json!(line.actor),
                "effect" => json!(line.effect),
                "margin_left" => json!(line.margin[0]),
                "margin_right" => json!(line.margin[1]),
                _ => json!(line.margin[2]),
            };
            if value != previous {
                fields.insert((*key).into(), value);
            }
        }
        self.invoke(
            json!({"op":"line_fields", "ids":self.snapshot.selected, "fields":fields}),
            cx,
        );
    }

    /// Asks the host to render the style being edited with its subtitles
    /// provider. The request is debounced because every keystroke in the font
    /// or size field changes the signature.
    fn refresh_style_preview(&mut self, cx: &mut Context<Self>) {
        const PREVIEW_WIDTH: u32 = 900;
        const PREVIEW_HEIGHT: u32 = 150;
        if self.document.preview_pending {
            return;
        }
        let fields = self.document.style_form.values(cx);
        let key = format!("{PREVIEW_WIDTH}x{PREVIEW_HEIGHT}{}", json!(fields));
        if key == self.document.preview_key {
            return;
        }
        self.document.preview_pending = true;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(140))
                .await;
            let Some(view) = weak.upgrade() else {
                return;
            };
            let current = view.read_with(cx, |view, cx| {
                format!(
                    "{PREVIEW_WIDTH}x{PREVIEW_HEIGHT}{}",
                    json!(view.document.style_form.values(cx))
                )
            });
            if current != key {
                // Still typing: let the next render queue the newest values.
                let _ = view.update(cx, |view, cx| {
                    view.document.preview_pending = false;
                    cx.notify();
                });
                return;
            }
            let reply = bridge
                .call(json!({
                    "op": "style_preview",
                    "fields": fields,
                    "width": PREVIEW_WIDTH.to_string(),
                    "height": PREVIEW_HEIGHT.to_string(),
                }))
                .await;
            let encoded = match reply.and_then(|value| {
                serde_json::from_value::<StylePreviewReply>(value)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
            }) {
                Ok(reply) => {
                    if let Some(error) = reply.error {
                        let _ = view.update(cx, |view, cx| {
                            view.document.preview_pending = false;
                            view.status = format!("样式预览失败：{error}");
                            cx.notify();
                        });
                        return;
                    }
                    reply.image
                }
                Err(error) => {
                    let _ = view.update(cx, |view, cx| {
                        view.document.preview_pending = false;
                        view.status = format!("样式预览失败：{error}");
                        cx.notify();
                    });
                    return;
                }
            };
            let Some(encoded) = encoded else {
                let _ = view.update(cx, |view, cx| {
                    view.document.preview_pending = false;
                    cx.notify();
                });
                return;
            };
            let decoded = cx
                .background_executor()
                .spawn(async move {
                    let bytes = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        encoded,
                    )?;
                    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
                        .with_guessed_format()?;
                    let rgba =
                        image::DynamicImage::from_decoder(reader.into_decoder()?)?.into_rgba8();
                    let (width, height) = rgba.dimensions();
                    let mut bgra = rgba.into_raw();
                    for pixel in bgra.chunks_exact_mut(4) {
                        pixel.swap(0, 2);
                    }
                    let image = image::RgbaImage::from_raw(width, height, bgra)
                        .context("style preview dimensions do not match its pixel data")?;
                    Ok::<_, anyhow::Error>(std::sync::Arc::new(gpui_kit::RenderImage::new(vec![
                        VideoImageFrame::new(image),
                    ])))
                })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.document.preview_pending = false;
                match decoded {
                    Ok(image) => {
                        view.document.preview_image = Some(image);
                        view.document.preview_key = key;
                    }
                    Err(error) => view.status = format!("样式预览解码失败：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Aegisub stores line breaks as `\N` (or `\n` for soft breaks); a literal
    /// newline inside a Dialogue field would corrupt the file.
    fn editor_text(&self, cx: &mut Context<Self>) -> String {
        let raw = self.editor.read(cx).value().to_string();
        escape_line_breaks(raw, self.snapshot.appearance.soft_line_break)
    }

    fn handle_key(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.host_dialog, Some(HostDialog::Kanji(_))) {
            self.handle_kanji_key(event, window, cx);
            return;
        }
        let key = &event.keystroke;
        // The script dialog owns the keyboard while it is up: its keys never
        // reach the document, and the window's own shortcuts stay quiet.
        if self.script_dialog.is_some() {
            if self.handle_script_dialog_key(event, window, cx) {
                cx.stop_propagation();
            }
            cx.notify();
            return;
        }
        // A host notice behaves like a modal: Enter or Escape acknowledges it.
        if !self.notices.is_empty()
            && matches!(key.key.as_str(), "escape" | "enter" | "return" | "space")
        {
            self.notices.remove(0);
            cx.notify();
            return;
        }
        if self.active_panel == Panel::Hotkeys && self.hotkeys.recording {
            if key.key == "escape" {
                self.hotkeys.recording = false;
            } else if !matches!(
                key.key.as_str(),
                "shift" | "control" | "alt" | "cmd" | "platform"
            ) {
                let mut combo = String::new();
                if if cfg!(target_os = "macos") {
                    key.modifiers.platform
                } else {
                    key.modifiers.control
                } {
                    combo.push_str("Ctrl-");
                }
                if key.modifiers.alt {
                    combo.push_str("Alt-");
                }
                if key.modifiers.shift {
                    combo.push_str("Shift-");
                }
                combo.push_str(&hotkey_key(&key.key));
                self.hotkeys
                    .key
                    .update(cx, |state, cx| state.set_value(combo, window, cx));
                self.hotkeys.recording = false;
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let primary = if cfg!(target_os = "macos") {
            key.modifiers.platform
        } else {
            key.modifiers.control
        };
        if primary && key.modifiers.shift && key.key == "p" {
            self.active_panel = Panel::Commands;
            self.refresh_catalog(cx);
            self.command_search
                .update(cx, |state, cx| state.focus(window, cx));
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let translation_focused = self
            .translation
            .editor
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let styling_focused = self
            .styling
            .input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let assistant_context = if self.active_panel == Panel::Translation && translation_focused {
            Some("Translation Assistant")
        } else if self.active_panel == Panel::Styling && styling_focused {
            Some("Styling Assistant")
        } else {
            None
        };
        if let Some(context) = assistant_context {
            let shortcut = match key.key.as_str() {
                "f8" => "F8",
                "pageup" => "PageUp",
                "pagedown" => "PageDown",
                "home" => "Home",
                "end" => "End",
                "insert" => "Insert",
                _ => "",
            };
            if !shortcut.is_empty() && !primary && !key.modifiers.alt && !key.modifiers.shift {
                if let Some(command) = self.catalog.shortcut(context, shortcut) {
                    let name = command.name.clone();
                    if context == "Styling Assistant"
                        && matches!(name.as_str(), "grid/line/next" | "grid/line/prev")
                    {
                        self.styling_action(
                            if name.ends_with("next") {
                                "next"
                            } else {
                                "prev"
                            },
                            None,
                            cx,
                        );
                    } else {
                        self.invoke(json!({"op":"command", "name":name}), cx);
                    }
                    cx.stop_propagation();
                    return;
                }
            }
        }
        let editor_focused = self.editor.read(cx).focus_handle(cx).is_focused(window);
        let input_focused = window.has_focused_input(cx);
        let command_search_focused = self
            .command_search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if editor_focused {
            let line_break =
                key.key == "enter" && key.modifiers.shift && !primary && !key.modifiers.alt;
            let hard_space = key.key == "space"
                && if cfg!(target_os = "macos") {
                    key.modifiers.alt && !primary && !key.modifiers.shift
                } else {
                    primary && key.modifiers.shift && !key.modifiers.alt
                };
            if line_break || hard_space {
                let text = if hard_space {
                    "\\h"
                } else if self.snapshot.appearance.soft_line_break {
                    "\\n"
                } else {
                    "\\N"
                };
                self.editor
                    .update(cx, |state, cx| state.replace(text, window, cx));
                self.editor_dirty_line = self.selected_line;
                self.invoke(json!({"op":"edit", "id":self.selected_line, "ids":self.snapshot.selected, "text":self.editor.read(cx).value().to_string()}), cx);
                cx.stop_propagation();
                return;
            }
        }
        // A character key belongs to whichever text field has focus. With no text
        // focus there is nothing to type into, so the key is only a shortcut:
        // macOS reports Option+letter as a character, and dropping those here is
        // what used to make every Alt+letter binding dead in the grid.
        if event.prefer_character_input
            && (input_focused || editor_focused || command_search_focused)
        {
            return;
        }
        // A form field keeps the keys that would type into it, but command
        // shortcuts still have to reach the catalog: pressing Cmd+S with the
        // filter box focused must save, not be swallowed.
        if input_focused && !editor_focused && !command_search_focused && !primary {
            return;
        }
        if command_search_focused {
            if !primary
                || matches!(
                    key.key.as_str(),
                    "a" | "c" | "v" | "x" | "left" | "right" | "up" | "down"
                )
            {
                return;
            }
            let mut binding = Vec::new();
            if primary {
                binding.push("Ctrl");
            }
            if key.modifiers.alt {
                binding.push("Alt");
            }
            if key.modifiers.shift {
                binding.push("Shift");
            }
            let key_name = hotkey_key(&key.key);
            binding.push(&key_name);
            if let Some(command) = self.catalog.shortcut("Subtitle Grid", &binding.join("-")) {
                let name = command.name.clone();
                self.invoke(json!({"op":"command","name":name}), cx);
                cx.stop_propagation();
            }
            return;
        }
        if editor_focused
            && (!primary
                || matches!(
                    key.key.as_str(),
                    "a" | "c" | "v" | "x" | "left" | "right" | "up" | "down"
                ))
        {
            return;
        }
        let mut combo = String::new();
        if primary {
            combo.push_str("Ctrl-");
        }
        if key.modifiers.alt {
            combo.push_str("Alt-");
        }
        if key.modifiers.shift {
            combo.push_str("Shift-");
        }
        let name = hotkey_key(&key.key);
        combo.push_str(&name);
        let context = if command_search_focused {
            "Subtitle Grid"
        } else {
            self.key_context(window, cx)
        };
        let command = self.catalog.shortcut(context, &combo).or_else(|| {
            let keypad = keypad_name(&key.key)?;
            let alternate = combo.strip_suffix(&name)?;
            self.catalog
                .shortcut(context, &format!("{alternate}{keypad}"))
        });
        if let Some(command) = command {
            let name = command.name.clone();
            self.invoke(json!({"op":"command", "name":name}), cx);
            cx.stop_propagation();
        } else if context == "Video" && combo == "Space" {
            // Space plays and pauses, as in the transport row: video/play only
            // starts playback, so the toggle has to pick the stop side itself.
            let name = if self.snapshot.playing {
                "video/stop"
            } else {
                "video/play"
            };
            self.invoke(json!({"op":"command", "name":name}), cx);
            cx.stop_propagation();
        }
    }

    /// Resolve the Aegisub hotkey context from the widget that owns the keyboard.
    fn key_context(&self, window: &mut Window, cx: &mut Context<Self>) -> &'static str {
        if self.editor.read(cx).focus_handle(cx).is_focused(window) {
            return "Subtitle Edit Box";
        }
        match self.key_focus {
            KeyFocus::Audio => "Audio",
            KeyFocus::Video => "Video",
            KeyFocus::Grid => match self.active_panel {
                Panel::Visual => "Video",
                _ => "Subtitle Grid",
            },
        }
    }

    pub(crate) fn set_key_focus(&mut self, focus: KeyFocus, cx: &mut Context<Self>) {
        if self.key_focus != focus {
            self.key_focus = focus;
            cx.notify();
        }
    }

    /// Claim the shortcut context for a panel the user just clicked. The window
    /// also has to focus the workspace itself: clicking a plain div leaves no
    /// focused element behind, and then no key ever reaches `handle_key`.
    pub(crate) fn claim_key_focus(
        &mut self,
        focus: KeyFocus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_key_focus(focus, cx);
        self.focus_handle.focus(window, cx);
    }

    /// Which panel owns the keyboard right now. The focused one gets an accent
    /// outline, so the shortcut context is visible instead of implicit.
    fn focus_area(&self, window: &mut Window, cx: &mut Context<Self>) -> KeyFocus {
        if self.editor.read(cx).focus_handle(cx).is_focused(window) {
            return KeyFocus::Grid;
        }
        match self.key_focus {
            KeyFocus::Grid => match self.active_panel {
                Panel::Visual => KeyFocus::Video,
                _ => KeyFocus::Grid,
            },
            other => other,
        }
    }

    /// Shortcut hints for the status bar. Keys come from the host hotkey table so
    /// remapped bindings show up here too; the labels are the short UI names.
    fn shortcut_hints(&self, context: &str) -> Option<(&'static str, Vec<(String, &'static str)>)> {
        let (_, label, items) = SHORTCUT_HINTS
            .iter()
            .find(|(name, _, _)| *name == context)?;
        let hints: Vec<_> = items
            .iter()
            .filter_map(|(command, text)| {
                self.catalog
                    .keys(context, command)
                    .map(|key| (pretty_key(key), *text))
            })
            .take(4)
            .collect();
        (!hints.is_empty()).then_some((*label, hints))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(cx);
        let filename = std::path::Path::new(&self.snapshot.file)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("未命名字幕");
        let enabled = |name: &str| {
            self.catalog
                .command(name)
                .is_some_and(|command| command.can_invoke())
        };
        div()
            .h_flex()
            .h(px(42.))
            .flex_shrink_0()
            .gap_2()
            .px_4()
            .items_center()
            .bg(p.surface)
            .border_b_1()
            .border_color(p.divider)
            .child(
                div()
                    .h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(16.))
                            .font_semibold()
                            .child(self.active_panel.label()),
                    )
                    .child(
                        div()
                            .h_flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .text_size(px(11.))
                            .text_color(p.muted)
                            .child(Icon::new(IconName::FileText).size(px(12.)).flex_shrink_0())
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(filename.to_owned()),
                            ),
                    ),
            )
            .child(
                Button::new("open-subtitles")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .icon(IconName::FolderOpen)
                    .label("打开")
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| view.pick_file("subtitle", cx))),
            )
            .child(
                Button::new("save-subtitles")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .icon(IconName::Save)
                    .label("保存")
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.invoke(json!({"op":"command", "name":"subtitle/save"}), cx)
                    })),
            )
            .child(div().w(px(1.)).h(px(20.)).mx_1().bg(p.divider))
            .child(
                Button::new("undo")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .child(Icon::new(IconName::Undo2).size(px(15.)))
                    .accessibility_label("撤销")
                    .tooltip("撤销")
                    .ghost()
                    .disabled(!enabled("edit/undo"))
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.invoke(json!({"op":"command", "name":"edit/undo"}), cx)
                    })),
            )
            .child(
                Button::new("redo")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .child(Icon::new(IconName::Redo2).size(px(15.)))
                    .accessibility_label("重做")
                    .tooltip("重做")
                    .ghost()
                    .disabled(!enabled("edit/redo"))
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.invoke(json!({"op":"command", "name":"edit/redo"}), cx)
                    })),
            )
            .child(div().w(px(1.)).h(px(20.)).mx_1().bg(p.divider))
            .child(
                Button::new("open-video")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .icon(IconName::Play)
                    .label("视频")
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| view.pick_file("video", cx))),
            )
            .child(
                div()
                    .ml_2()
                    .h_flex()
                    .gap_1()
                    .items_center()
                    .text_size(px(11.))
                    .text_color(if self.snapshot.modified {
                        p.warning
                    } else {
                        p.success
                    })
                    .child(
                        Icon::new(if self.snapshot.modified {
                            IconName::Asterisk
                        } else {
                            IconName::CircleCheck
                        })
                        .size(px(12.)),
                    )
                    .child(if self.snapshot.modified {
                        "未保存"
                    } else {
                        "已保存"
                    }),
            )
    }

    fn render_legacy_menus(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(cx);
        let mut bar = div()
            .h_flex()
            .items_center()
            .gap_2()
            .px_4()
            .h(px(38.))
            .bg(p.surface)
            .flex_shrink_0();
        for entry in self.catalog.menu("main") {
            if entry.special == "window" || entry.submenu == "main/window" {
                continue;
            }
            let catalog = self.catalog.clone();
            let weak = cx.weak_entity();
            let refresh_weak = weak.clone();
            let original_label = entry.text.replace('&', "");
            let label = match original_label.as_str() {
                "File" => "文件",
                "Edit" => "编辑",
                "View" => "视图",
                "Subtitle" => "字幕",
                "Timing" => "时间",
                "Video" => "视频",
                "Audio" => "音频",
                "Automation" => "自动化",
                "Help" => "帮助",
                _ => &original_label,
            }
            .to_owned();
            let submenu = entry.submenu.clone();
            let entries = if entry.special == "automation" {
                vec![entry]
            } else {
                catalog.menu(&submenu)
            };
            bar = bar.child(
                Button::new(format!("legacy-menu-{label}"))
                    .icon(IconName::Menu)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .label(label)
                    .ghost()
                    .dropdown_menu(move |menu, window, cx| {
                        populate_menu(
                            menu,
                            entries.clone(),
                            catalog.clone(),
                            weak.clone(),
                            window,
                            cx,
                        )
                    })
                    .on_open_change(move |open, _, cx| {
                        if *open {
                            let _ = refresh_weak.update(cx, |view, cx| view.refresh_catalog(cx));
                        }
                    }),
            );
        }
        bar.child(div().flex_1()).child(
            Button::new("command-browser")
                .rounded(gpui_kit::px(999.))
                .small()
                .px(gpui_kit::px(13.))
                .py(gpui_kit::px(3.))
                .icon(IconName::Search)
                .label("搜索命令")
                .ghost()
                .on_click(cx.listener(|view, _, _, cx| {
                    view.active_panel = Panel::Commands;
                    view.refresh_catalog(cx);
                    cx.notify();
                })),
        )
    }

    fn render_commands(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.command_search.read(cx).value().to_lowercase();
        let commands: Vec<_> = self
            .catalog
            .commands
            .iter()
            .filter(|command| {
                command.surface != "internal"
                    && (query.is_empty()
                        || format!("{} {} {}", command.name, command.label, command.help)
                            .to_lowercase()
                            .contains(&query))
            })
            .cloned()
            .collect();
        let count = commands.len();
        let weak = cx.weak_entity();
        page::page(cx, "commands-panel")
            .child(page::toolbar([
                Input::new(&self.command_search)
                    .id("command-search")
                    .w(px(320.))
                    .into_any_element(),
                Button::new("refresh-command-catalog").icon(IconName::RefreshCw).rounded(gpui_kit::px(999.)).small().px(gpui_kit::px(13.)).py(gpui_kit::px(3.))
                    .label("刷新状态")
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| view.refresh_catalog(cx)))
                    .into_any_element(),
                page::badge(cx, format!("{count} 项命令")).into_any_element(),
            ]))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(crate::workspace_theme::RADIUS_LG))
                    .border_1()
                    .border_color(crate::workspace_theme::Palette::current(cx).border)
                    .bg(crate::workspace_theme::Palette::current(cx).surface)
                    .shadow(page::card_shadow(Theme::global(cx).is_dark()))
                    .child(uniform_list("legacy-command-list", count, cx.processor(move |_, range: Range<usize>, _, cx| {
                range.map(|index| {
                    let command = commands[index].clone();
                    let name = command.name.clone();
                    let weak = weak.clone();
                    div().w_full().h(px(68.)).px_2().border_b_1().border_color(Theme::global(cx).border).h_flex().gap_3().items_center()
                        .child(div().flex_1().min_w_0()
                            .child(div().text_sm().child(format!("{}{}", if command.active { "✓ " } else { "" }, command.label)))
                            .child(div().text_xs().text_color(Theme::global(cx).muted_foreground).child(format!("{} · {}", command.name, command.help))))
                        .child(div().w(px(120.)).flex_shrink_0().text_xs().child(command.surface_label()))
                        .child(div().w(px(100.)).flex_shrink_0().text_xs().child(command.shortcut.clone()))
                        .child(Button::new(format!("invoke-{name}")).icon(IconName::Play).rounded(gpui_kit::px(999.)).small().px(gpui_kit::px(13.)).py(gpui_kit::px(3.)).label("执行").disabled(!command.can_invoke()).on_click(move |_, _, cx| {
                            let _ = weak.update(cx, |view, cx| view.invoke(json!({"op":"command", "name":name}), cx));
                        }))
                }).collect()
            })).size_full()),
            )
    }

    /// Line inspector. `rail` is the copy that lives beside the text editor:
    /// narrower, no card chrome, and its own scroll.
    /// Height the stacked properties card reserves inside the editor section.
    fn properties_height(&self) -> f32 {
        if self.properties_more { 95. } else { 56. }
    }

    fn render_properties(&self, rail: Option<f32>, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(cx);
        // Inline label + field. The label above each box cost a full text line
        // per row, which is what made this panel four rows tall.
        let inline = |key: &'static str, label: &'static str, grow: f32| {
            div()
                .h_flex()
                .items_center()
                .gap_1p5()
                .flex_basis(gpui_kit::px(0.))
                .flex_grow(grow)
                .flex_shrink(1.)
                .min_w_0()
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(label),
                )
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(&self.property_inputs[key])
                            .id(format!("property-{key}"))
                            .aria_label(label)
                            .h(px(32.))
                            .w_full(),
                    ),
                )
        };
        let commented = self
            .snapshot
            .lines
            .iter()
            .find(|line| line.id == self.selected_line)
            .is_some_and(|line| line.comment);
        let more = self.properties_more;
        // Margins, layer and effect are used on maybe one line in twenty, so they
        // hide behind one toggle instead of holding a permanent row.
        let more_button = Button::new("properties-more")
            .rounded(gpui_kit::px(999.))
            .px(gpui_kit::px(9.))
            .py(gpui_kit::px(5.))
            .child(Icon::new(IconName::Settings2).size(px(15.)))
            .accessibility_label(if more {
                "收起边距与图层"
            } else {
                "展开边距与图层"
            })
            .tooltip(if more {
                "收起边距与图层"
            } else {
                "展开边距与图层"
            })
            .when(more, |button| button.bg(p.selected).text_color(p.accent))
            .ghost()
            .on_click(cx.listener(|view, _, _, cx| {
                view.properties_more = !view.properties_more;
                cx.notify();
            }));
        let toggle_comment = Button::new("toggle-comment")
            .rounded(gpui_kit::px(999.))
            .px(gpui_kit::px(9.))
            .py(gpui_kit::px(5.))
            .child(Icon::new(IconName::Captions).size(px(15.)))
            .accessibility_label(if commented { "改为对白" } else { "改为注释" })
            .tooltip(if commented { "改为对白" } else { "改为注释" })
            .when(commented, |button| button.bg(p.selected).text_color(p.accent))
            .ghost()
            .disabled(self.selected_line < 0)
            .on_click(cx.listener(move |view, _, _, cx| {
                view.invoke(json!({"op": "line_fields", "ids": view.snapshot.selected, "fields": {"comment": !commented}}), cx);
            }));
        // Enter commits while typing, so this stays an icon rather than a pill.
        let apply = Button::new("apply-line-fields")
            .rounded(gpui_kit::px(999.))
            .px(gpui_kit::px(9.))
            .py(gpui_kit::px(5.))
            .child(Icon::new(IconName::Check).size(px(15.)))
            .accessibility_label("应用修改")
            .tooltip("应用修改")
            .primary()
            .disabled(self.selected_line < 0 || !self.properties_dirty)
            .on_click(cx.listener(|view, _, _, cx| view.commit_properties(cx)));
        let core = div()
            .h_flex()
            .items_center()
            .gap_1p5()
            .w_full()
            .min_w_0()
            .child(div().flex_1().min_w_0().child(line_field_row(cx, self, &p)))
            .child(more_button)
            .child(toggle_comment)
            .child(apply);
        let more_row = more.then(|| {
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .w_full()
                .min_w_0()
                .child(inline("margin_left", "左", 1.))
                .child(inline("margin_right", "右", 1.))
                .child(inline("margin_vertical", "垂直", 1.))
                .child(inline("layer", "图层", 1.))
                .child(inline("effect", "效果", 2.))
        });
        div()
            .id("line-properties")
            .v_flex()
            .min_w_0()
            .min_h_0()
            .when(rail.is_some(), |panel| {
                panel
                    .w(px(rail.unwrap_or(272.)))
                    .flex_shrink_0()
                    .h_full()
                    .p_3()
                    .gap_3()
                    .border_l_1()
                    .border_color(p.divider)
            })
            .when(rail.is_none(), |panel| {
                panel
                    // The panel sizes to its fields; letting it flex made it lose
                    // the fight with the editor and the list.
                    .flex_none()
                    .h(px(self.properties_height()))
                    // The properties strip reads as a band across the editor, not
                    // as a card of its own, so it keeps square corners.
                    .rounded(px(0.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.surface)
                    .shadow(crate::page::card_shadow(Theme::global(cx).is_dark()))
                    .p_2()
                    .gap(px(7.))
            })
            .child(core)
            .children(more_row)
    }

    fn activate_panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        if self.active_panel == Panel::Visual && panel != Panel::Visual {
            // Leaving the page mid-drag must not leave the tool holding a
            // button it will never see released.
            self.visual.dragging = false;
            self.visual.queue.clear();
            self.queue_visual(json!({"action":"cancel"}), cx);
        }
        self.active_panel = panel;
        if panel == Panel::Visual {
            // The overlay still holds the picture from the last visit, and
            // nothing else asks for a new one unless the layout moved.
            self.queue_visual(json!({"action":"draw"}), cx);
        }
        if panel == Panel::Commands {
            self.refresh_catalog(cx);
        }
        if panel == Panel::Styles || panel == Panel::Project {
            self.refresh_document(cx);
        }
        if panel == Panel::Styles {
            self.refresh_library(cx);
        }
        if panel == Panel::VideoSettings {
            self.video_settings_request("state", cx);
        }
        if panel == Panel::Preferences {
            self.preferences_request(json!({"action":"state"}), cx);
        }
        if panel == Panel::Hotkeys {
            self.hotkey_request(json!({"action":"state"}), cx);
        }
        if panel == Panel::Paste {
            self.paste_request(false, cx);
        }
        if panel == Panel::Files {
            self.file_tools_request(json!({"action":"state"}), cx);
        }
        if panel == Panel::Attachments {
            self.attachment_request(json!({"action":"state"}), cx);
        }
        if panel == Panel::Spelling {
            self.spelling_request(json!({"action":"state"}), cx);
        }
        if panel == Panel::Search {
            self.find_panel.focus_pending = true;
            self.search_request("state", cx);
        }
        if panel == Panel::Processor {
            self.processor_request("state", cx);
        }
        if panel == Panel::Resample {
            self.resample_request("state", cx);
        }
        if panel == Panel::Automation {
            self.automation_request(json!({"action":"state"}), cx);
        }
        if panel == Panel::Timing {
            self.shift_times_request("state", cx);
        }
        if panel == Panel::Translation {
            self.translation_action("start", cx);
        }
        if panel == Panel::Styling {
            self.styling.line_id = -1;
            self.styling.focus_pending = true;
            self.refresh_document(cx);
            self.styling_action("start", None, cx);
        }
        cx.notify();
    }

    fn render_menu(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(cx);
        let compact = self
            .sidebar_collapsed
            .unwrap_or(f32::from(window.viewport_size().width) < 1150.);
        let toggle_label = if compact {
            "展开侧栏"
        } else {
            "折叠侧栏"
        };
        let mut navigation = div()
            .id("workspace-navigation")
            .v_flex()
            .min_h_0()
            .flex_1()
            .gap_1()
            .px_3()
            .py_4()
            .overflow_y_scrollbar();
        for (group, panels) in [
            (
                "工作区",
                vec![Panel::Subtitles, Panel::Visual, Panel::Translation],
            ),
            (
                "字幕工具",
                vec![
                    Panel::Styles,
                    Panel::Styling,
                    Panel::Timing,
                    Panel::Processor,
                    Panel::Resample,
                    Panel::Search,
                    Panel::Spelling,
                    Panel::Paste,
                ],
            ),
            (
                "项目与设置",
                vec![
                    Panel::Project,
                    Panel::VideoSettings,
                    Panel::Automation,
                    Panel::Attachments,
                    Panel::Files,
                    Panel::Preferences,
                    Panel::Hotkeys,
                    Panel::Commands,
                ],
            ),
        ] {
            navigation = navigation.child(
                div()
                    .mt_4()
                    .mb_2()
                    .px_2()
                    .text_size(px(11.))
                    .text_color(p.muted)
                    .child(if compact { "" } else { group }),
            );
            for panel in panels {
                let selected = self.active_panel == panel;
                let content = div()
                    .h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .when(compact, |v| v.justify_center())
                    .child(Icon::new(panel.icon()).size(px(15.)))
                    .when(!compact, |v| {
                        v.child(div().flex_1().text_size(px(13.)).child(panel.label()))
                    });
                let button = Button::new(format!("panel-{}", panel.label()))
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .ghost()
                    .selected(selected)
                    .accessibility_label(panel.label())
                    .tooltip(panel.label())
                    .w_full()
                    .h(px(36.))
                    .px_2()
                    .child(content)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        // The workspace window keeps the subtitle editor; every
                        // tool is its own window, so a tool never replaces the
                        // editing context the user is working in.
                        if panel == Panel::Subtitles {
                            view.activate_panel(panel, window, cx);
                        } else if let Err(error) =
                            focus_or_open_panel_window(cx, view.bridge.clone(), panel)
                        {
                            eprintln!("Could not open tool window: {error:#}");
                        }
                    }));
                let row = div().relative().w_full().child(button);
                navigation = navigation.child(row);
            }
        }
        div()
            .v_flex()
            .h_full()
            .min_h_0()
            .overflow_hidden()
            .w(px(if compact { 60. } else { 164. }))
            .flex_shrink_0()
            .bg(p.canvas)
            .border_r_1()
            .border_color(p.divider)
            .child(
                div()
                    .h_flex()
                    .h(px(64.))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .when(compact, |v| v.justify_center())
                    .flex_shrink_0()
                    .when(!compact, |v| {
                        v.child(
                            div()
                                .v_flex()
                                .child(
                                    div()
                                        .font_semibold()
                                        .text_size(px(15.))
                                        .child("Neo Aegisub"),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(p.muted)
                                        .child("字幕工作台"),
                                ),
                        )
                    })
                    .child(
                        // Button::icon() renders nothing inside this header row,
                        // so the icon goes in as an explicit child like the nav
                        // rows do.
                        Button::new("toggle-sidebar")
                            .rounded(gpui_kit::px(999.))
                            .small()
                            .px(gpui_kit::px(11.))
                            .py(gpui_kit::px(3.))
                            .ghost()
                            .accessibility_label(toggle_label)
                            .tooltip(toggle_label)
                            .child(
                                Icon::new(if compact {
                                    IconName::PanelLeftOpen
                                } else {
                                    IconName::PanelLeftClose
                                })
                                .size(px(16.))
                                .text_color(p.muted),
                            )
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.sidebar_collapsed = Some(!compact);
                                cx.notify();
                            })),
                    ),
            )
            .child(navigation)
    }

    /// Raw text of the line being timed, tags and all: the audio strip shows it
    /// the way the editor's "show original" line does, so the strip reads as
    /// part of the script rather than as a rendered cue.
    fn timed_line_text(&self) -> String {
        self.snapshot
            .lines
            .iter()
            .find(|line| line.id == self.selected_line)
            .map(|line| line.text.clone())
            .unwrap_or_default()
    }

    fn render_player(
        &self,
        window: &Window,
        fill: bool,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = Palette::current(cx);
        let readout = if self.snapshot.video {
            format!(
                "帧 {:05} · {:.2}:1 · {} fps",
                self.snapshot.frame,
                self.snapshot.aspect,
                self.snapshot
                    .fps
                    .map(|fps| format!("{fps:.3}"))
                    .unwrap_or_else(|| "VFR".into())
            )
        } else {
            "未加载视频".into()
        };
        let zoom_percent = self.video_zoom_percent();
        // Zoom and pan are local to the view; mpv and the workspace both place
        // the picture from them, so these buttons only touch local state.
        let controls = div()
            .h_flex()
            .gap_1()
            .items_center()
            .flex_shrink_0()
            .child(
                Button::new("video-content-out")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .child(Icon::new(IconName::ZoomOut).size(px(15.)))
                    .accessibility_label("缩小画面")
                    .tooltip("缩小画面")
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| view.video_zoom_by(-1., cx))),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(p.muted)
                    .w(px(48.))
                    .text_center()
                    .child(format!("{zoom_percent:.0}%")),
            )
            .child(
                Button::new("video-content-in")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .child(Icon::new(IconName::ZoomIn).size(px(15.)))
                    .accessibility_label("放大画面")
                    .tooltip("放大画面")
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| view.video_zoom_by(1., cx))),
            )
            .child(
                Button::new("video-zoom-reset")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .child(Icon::new(IconName::RotateCw).size(px(15.)))
                    .accessibility_label("复位画面")
                    .tooltip("复位缩放和平移")
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| view.video_reset_view(cx))),
            )
            .child(
                Button::new("video-fit-menu")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(10.))
                    .py(gpui_kit::px(3.))
                    .label("适应")
                    .tooltip("缩放预设")
                    .outline()
                    .dropdown_menu({
                        let weak = cx.weak_entity();
                        move |mut menu, _, _| {
                            for (label, percent) in [
                                ("适应窗口", 0.),
                                ("50%", 50.),
                                ("100%", 100.),
                                ("200%", 200.),
                                ("400%", 400.),
                            ] {
                                let weak = weak.clone();
                                menu = menu.item(PopupMenuItem::new(label).on_click(
                                    move |_, _, cx| {
                                        let _ = weak.update(cx, |view, cx| {
                                            if percent <= 0. {
                                                view.video_fit_window(cx);
                                            } else {
                                                view.video_zoom_to_percent(percent, cx);
                                            }
                                        });
                                    },
                                ));
                            }
                            menu
                        }
                    }),
            )
            .child(
                Button::new("video-fit-window")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(10.))
                    .py(gpui_kit::px(3.))
                    .child(Icon::new(IconName::Fullscreen).size(px(15.)))
                    .accessibility_label("适应窗口")
                    .tooltip("适应窗口：让整幅画面适应预览区")
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| view.video_fit_window(cx))),
            )
            .child(
                Button::new("video-actual-size")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(10.))
                    .py(gpui_kit::px(3.))
                    .label("100%")
                    .tooltip("按视频原始像素显示")
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| view.video_zoom_actual(cx))),
            )
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(p.divider))
            .child(
                Button::new("video-pan-menu")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .child(Icon::new(IconName::Move).size(px(15.)))
                    .accessibility_label("平移画面")
                    .tooltip("平移画面")
                    .ghost()
                    .dropdown_menu({
                        let weak = cx.weak_entity();
                        move |mut menu, _, _| {
                            for (label, dx, dy) in [
                                ("向左平移", -20., 0.),
                                ("向右平移", 20., 0.),
                                ("向上平移", 0., -20.),
                                ("向下平移", 0., 20.),
                            ] {
                                let weak = weak.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(label).on_click(
                                        move |_, _, cx| {
                                            let _ = weak.update(cx, |view, cx| {
                                                let area = *view.video_view.bounds.borrow();
                                                let width = f32::from(area.size.width).max(1.);
                                                let height = f32::from(area.size.height).max(1.);
                                                view.video_view.pan.0 += dx / width;
                                                view.video_view.pan.1 += dy / height;
                                                cx.notify();
                                            });
                                        },
                                    ),
                                );
                            }
                            menu
                        }
                    }),
            )
            .child(
                Button::new("video-overscan")
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .child(Icon::new(IconName::Frame).size(px(15.)))
                    .label("安全区")
                    .toggled(self.snapshot.video_overscan)
                    .tooltip("电视过扫描安全区")
                    .ghost()
                    .selected(self.snapshot.video_overscan)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.invoke(json!({"op":"command", "name":"video/show_overscan"}), cx)
                    })),
            );
        // Title and controls share one row: the title names the card, the
        // controls act on the picture and disappear with it, so an empty card
        // is a plain title over a stretched placeholder instead of a stack of
        // buttons that do nothing.
        let header = div()
            .h_flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_2()
            .flex_shrink_0()
            .rounded_t(px(crate::workspace_theme::RADIUS_LG - 1.))
            .bg(p.surface)
            .text_color(p.text)
            .child(Icon::new(IconName::Play).size(px(14.)).text_color(p.muted))
            .child({
                let popped = self.detached == Some(Panel::Video);
                div()
                    .id("video-surface-title")
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .font_semibold()
                    .when(!popped, |title| {
                        title
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.open_tool(Panel::Video, cx, |_, _| {});
                                view.video_popped = true;
                                cx.notify();
                            }))
                    })
                    .child(if popped { "视频" } else { "视频预览" })
            })
            .when(self.snapshot.video, |row| {
                row.child(div().flex_1()).child(controls)
            });
        // The seek bar is not built here: `Timeline::render` owns it so it can
        // sit on the seam between this readout and the transport row.
        div()
            .v_flex()
            .w_full()
            .min_w_0()
            .min_h_0()
            .when(fill, |card| card.flex_1())
            .when(!fill, |card| card.flex_shrink_0())
            .overflow_hidden()
            .rounded(px(crate::workspace_theme::RADIUS_LG))
            .border_1()
            .border_color(if focused { p.accent } else { p.border })
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    view.claim_key_focus(KeyFocus::Video, window, cx)
                }),
            )
            .child(header)
            .child(self.video_region(fill, cx))
            .child(
                div()
                    .h_flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .bg(p.inset)
                    .border_t_1()
                    .border_color(p.divider)
                    .text_color(p.muted)
                    .child(div().child(readout))
                    .when(f32::from(window.viewport_size().width) > 1100., |v| {
                        v.child("滚轮缩放与拖动在视频区域中进行")
                    }),
            )
            .child(self.timeline.render(&self.snapshot, cx))
    }

    fn subtitle_column_width(&self, column: SubtitleColumn, secondary: f32) -> f32 {
        self.subtitle_column_widths
            .iter()
            .find(|(key, _)| *key == column)
            .map(|(_, width)| *width)
            .unwrap_or_else(|| column.default_width(secondary))
    }

    fn set_subtitle_column_width(&mut self, column: SubtitleColumn, width: f32) {
        match self
            .subtitle_column_widths
            .iter_mut()
            .find(|(key, _)| *key == column)
        {
            Some(entry) => entry.1 = width,
            None => self.subtitle_column_widths.push((column, width)),
        }
    }

    fn save_subtitle_widths(&self, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let widths = serde_json::to_string(&self.subtitle_column_widths).unwrap();
        cx.spawn(async move |view, cx| {
            if let Err(error) = bridge
                .call(json!({"op":"preferences", "action":"grid_widths", "widths":widths}))
                .await
            {
                let _ = view.update(cx, |view, cx| {
                    view.status = format!("保存列宽失败：{error}");
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Adds or removes an optional column, keeping the canonical left-to-right
    /// order so a re-added column returns to where it was.
    fn toggle_subtitle_column(&mut self, column: SubtitleColumn) {
        const ORDER: [SubtitleColumn; 7] = [
            SubtitleColumn::IdLayer,
            SubtitleColumn::Time,
            SubtitleColumn::StyleActor,
            SubtitleColumn::Layer,
            SubtitleColumn::Effect,
            SubtitleColumn::Actor,
            SubtitleColumn::Comment,
        ];
        if let Some(index) = self.subtitle_columns.iter().position(|key| *key == column) {
            self.subtitle_columns.remove(index);
            return;
        }
        let rank = |column: SubtitleColumn| {
            ORDER
                .iter()
                .position(|key| *key == column)
                .unwrap_or(usize::MAX)
        };
        let position = self
            .subtitle_columns
            .iter()
            .position(|key| rank(*key) > rank(column))
            .unwrap_or(self.subtitle_columns.len());
        self.subtitle_columns.insert(position, column);
    }

    fn render_subtitle_panel(
        &mut self,
        rail: Option<f32>,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let dark = Theme::global(cx).is_dark();
        let p = Palette::new(dark);
        let appearance = &self.snapshot.appearance;
        let line_bg = appearance.color("Colour/Subtitle Grid/Background/Background", p.surface);
        let foreground = appearance.color("Colour/Subtitle Grid/Standard", p.text);
        let selected_foreground =
            appearance.color("Colour/Subtitle Grid/Selection", p.selected_text);
        let comment_bg = appearance.color("Colour/Subtitle Grid/Background/Comment", p.inset);
        let selected_comment_bg = appearance.color(
            "Colour/Subtitle Grid/Background/Selected Comment",
            p.selected,
        );
        let inframe_bg = appearance.color("Colour/Subtitle Grid/Background/Inframe", p.raised);
        let highlight_visible = appearance.highlight_visible && self.snapshot.video;
        let video_time = self.snapshot.video_time;
        let focus_grid = appearance.focus_grid;
        // The option is a point size. GPUI's logical pixel is a point on macOS
        // but 1/96 inch elsewhere, so only Windows and Linux need the 4/3
        // points-to-pixels conversion the wxWidgets grid got from wxDC.
        let points_to_px = if cfg!(target_os = "macos") {
            1.
        } else {
            4. / 3.
        };
        let grid_size = if appearance.grid_size > 0 {
            (appearance.grid_size as f32 * points_to_px).clamp(4., 56.)
        } else {
            12.
        };
        let grid_font = appearance.grid_font.clone();
        // Secondary columns scale with the configured grid font so the list
        // reads as one type scale instead of 16px text beside 11px labels.
        let secondary_size = (grid_size * 0.9).clamp(12., 22.);
        // Column widths follow the type size, otherwise a larger grid font
        // pushes the time range into the style column. Anything the user has
        // dragged keeps its pixel width instead.
        let columns: std::rc::Rc<Vec<(SubtitleColumn, f32)>> = std::rc::Rc::new(
            self.subtitle_columns
                .iter()
                .map(|column| (*column, self.subtitle_column_width(*column, secondary_size)))
                .collect(),
        );
        let active_columns = self.subtitle_columns.clone();
        // Adding columns must not push the text column off the panel, so once
        // the fixed columns no longer fit they shrink together (down to a per
        // column floor) instead of overflowing.
        let grid_width = self.subtitle_grid_width;
        let columns: std::rc::Rc<Vec<(SubtitleColumn, f32)>> = if grid_width > 0. {
            let room = grid_width - 26. - COLUMN_GAP * columns.len() as f32 - MIN_TEXT_COLUMN;
            let total: f32 = columns.iter().map(|(_, width)| *width).sum();
            if total > room && room > 0. {
                // Widths the user dragged are honoured; only the untouched
                // columns give way, so resizing one column actually resizes it.
                let dragged: Vec<bool> = columns
                    .iter()
                    .map(|(column, _)| {
                        self.subtitle_column_widths
                            .iter()
                            .any(|(other, _)| other == column)
                    })
                    .collect();
                let fixed: f32 = columns
                    .iter()
                    .zip(&dragged)
                    .filter(|(_, dragged)| **dragged)
                    .map(|((_, width), _)| *width)
                    .sum();
                let floors: f32 = columns
                    .iter()
                    .zip(&dragged)
                    .filter(|(_, dragged)| !**dragged)
                    .map(|((column, _), _)| column.min_width())
                    .sum();
                let auto: f32 = total - fixed;
                if fixed + floors <= room {
                    let factor = (room - fixed).max(0.) / auto.max(1.);
                    let factor = factor.min(1.);
                    std::rc::Rc::new(
                        columns
                            .iter()
                            .zip(&dragged)
                            .map(|((column, width), dragged)| {
                                if *dragged {
                                    (*column, *width)
                                } else {
                                    (*column, (*width * factor).max(column.min_width()))
                                }
                            })
                            .collect(),
                    )
                } else {
                    // Even the floors do not fit: everything shrinks so the
                    // text column keeps its minimum instead of running off.
                    let factor = room / total;
                    std::rc::Rc::new(
                        columns
                            .iter()
                            .map(|(column, width)| {
                                (*column, (*width * factor).max(column.min_width()))
                            })
                            .collect(),
                    )
                }
            } else {
                columns
            }
        } else {
            columns
        };
        let menu_weak = cx.weak_entity();
        let size_weak = cx.weak_entity();
        let row_height = (grid_size * 1.6 + 12.).max(36.);
        let stripe = p.surface;
        let border = appearance.color("Colour/Subtitle Grid/Lines", p.divider);
        let header_bg = appearance.color("Colour/Subtitle Grid/Header", p.inset);
        let muted = p.muted;
        // The list follows the same toggle as the start/end fields; otherwise the
        // grid keeps timecodes while the editor shows frame numbers.
        let frame_rate = self
            .time_in_frames
            .then_some(())
            .and(self.snapshot.fps)
            .filter(|fps| *fps > 0.);
        let selected_bg = appearance.color("Colour/Subtitle Grid/Background/Selection", p.selected);
        let accent = p.accent;
        let selected = self.snapshot.selected.clone();
        let weak = cx.weak_entity();
        let time_font = Theme::global(cx).mono_font_family.clone();
        let filtered = self.search.read(cx).value().to_string();
        let lower_filter = filtered.to_lowercase();
        let cache_matches =
            self.subtitle_filter_cache
                .as_ref()
                .is_some_and(|(revision, file, query, _)| {
                    *revision == self.snapshot.revision
                        && file == &self.snapshot.file
                        && query == &lower_filter
                });
        if !cache_matches {
            let visible = self
                .snapshot
                .lines
                .iter()
                .filter(|line| matches_subtitle_filter(line, &lower_filter))
                .cloned()
                .collect();
            self.subtitle_filter_cache = Some((
                self.snapshot.revision,
                self.snapshot.file.clone(),
                lower_filter.clone(),
                Arc::new(visible),
            ));
        }
        let visible = self
            .subtitle_filter_cache
            .as_ref()
            .expect("subtitle filter cache was just populated")
            .3
            .clone();
        let editor = Textarea::new(&self.editor)
            .aria_label("所选字幕文本")
            .bg(appearance.color("Colour/Subtitle/Background", p.inset))
            .text_color(appearance.color("Colour/Subtitle/Syntax/Normal", p.text))
            .when(appearance.edit_size > 0, |editor| {
                editor.text_size(px((appearance.edit_size as f32 * 4. / 3.).clamp(4., 56.)))
            })
            .when(!appearance.edit_font.is_empty(), |editor| {
                editor.font_family(appearance.edit_font.clone())
            });
        let mut format_toolbar = div().h_flex().flex_wrap().gap_1().items_center();
        // One palette button with the four colour slots behind a menu: four
        // identical AB buttons said nothing about which slot they edited.
        let color_weak = cx.weak_entity();
        let color_menu = Button::new("format-color")
            .rounded(gpui_kit::px(999.))
            .px(px(9.))
            .py(px(4.))
            .ghost()
            .tooltip("字幕颜色：主要色 / 次要色 / 描边颜色 / 阴影颜色")
            .child(Icon::new(IconName::Palette).size(px(15.)))
            .child(Icon::new(IconName::ChevronDown).size(px(11.)))
            .dropdown_menu(move |mut menu, _, _| {
                for (label, kind) in [
                    ("主要色", "primary"),
                    ("次要色", "secondary"),
                    ("描边颜色", "outline"),
                    ("阴影颜色", "shadow"),
                ] {
                    let target = color_weak.clone();
                    let name = format!("edit/color/{kind}");
                    menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                        let name = name.clone();
                        let _ = target.update(cx, |view, cx| {
                            view.invoke(json!({"op": "command", "name": name}), cx);
                        });
                    }));
                }
                menu
            });
        for (id, label, icon, name) in [
            ("format-bold", "粗体", IconName::Bold, "edit/style/bold"),
            (
                "format-italic",
                "斜体",
                IconName::Italic,
                "edit/style/italic",
            ),
            (
                "format-underline",
                "下划线",
                IconName::Underline,
                "edit/style/underline",
            ),
            (
                "format-strikeout",
                "删除线",
                IconName::Strikethrough,
                "edit/style/strikeout",
            ),
            ("format-font", "字体", IconName::ALargeSmall, "edit/font"),
            (
                "split-preserve",
                "光标处分行",
                IconName::Scissors,
                "edit/line/split/preserve",
            ),
            ("copy-lines", "复制行", IconName::Copy, "edit/line/copy"),
            ("cut-lines", "剪切行", IconName::Scissors, "edit/line/cut"),
            ("paste-lines", "粘贴行", IconName::Inbox, "edit/line/paste"),
        ] {
            let enabled = self
                .catalog
                .command(name)
                .is_some_and(|command| command.can_invoke());
            format_toolbar = format_toolbar.child(
                Button::new(id)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .child(Icon::new(icon).size(px(15.)))
                    .accessibility_label(label)
                    .tooltip(label)
                    .ghost()
                    .disabled(!enabled)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.invoke(json!({"op":"command", "name":name}), cx)
                    })),
            );
        }
        format_toolbar = format_toolbar.child(color_menu);
        // These two toggles close the formatting row: they change what the
        // timing fields and the editor show, and a row of their own would hold
        // two chips on an otherwise empty line.
        format_toolbar = format_toolbar
            .child(div().mx_1().h(px(16.)).w(px(1.)).bg(p.divider))
            .child(crate::page::with_tooltip(
                crate::page::toggle_chip(
                    cx,
                    "editor-frame-mode",
                    // The row is shared with the formatting controls, so the
                    // disabled state gets a short label; the tooltip explains it.
                    if self.snapshot.fps.is_some() {
                        "按帧显示"
                    } else {
                        "按帧"
                    },
                    self.time_in_frames,
                    self.snapshot.fps.is_none(),
                )
                .on_click(cx.listener(|view, _, window, cx| {
                    view.time_in_frames = !view.time_in_frames;
                    let fps = view.snapshot.fps.unwrap_or(0.);
                    if fps > 0. {
                        for key in ["start", "end"] {
                            let text = view.property_inputs[key].read(cx).value().to_string();
                            let ms = if view.time_in_frames {
                                parse_time(&text).ok()
                            } else {
                                text.trim()
                                    .parse::<f64>()
                                    .ok()
                                    .map(|frame| (frame * 1000. / fps).round() as i64)
                            };
                            if let Some(ms) = ms {
                                let shown = if view.time_in_frames {
                                    format!("{}", (ms as f64 * fps / 1000.).round() as i64)
                                } else {
                                    let centis = (ms as f64 / 10.).round() as i64;
                                    format!(
                                        "{}:{:02}.{:02}",
                                        centis / 6000,
                                        (centis / 100) % 60,
                                        centis % 100
                                    )
                                };
                                view.property_inputs[key]
                                    .update(cx, |state, cx| state.set_value(shown, window, cx));
                            }
                        }
                    }
                    cx.notify();
                })),
                if self.snapshot.fps.is_some() {
                    "时间按帧号显示，而不是时间码"
                } else {
                    "没有视频帧率，无法按帧输入"
                },
            ))
            .child(
                crate::page::toggle_chip(
                    cx,
                    "editor-show-original",
                    "显示原始字幕",
                    self.show_original,
                    false,
                )
                .on_click(cx.listener(|view, _, _, cx| {
                    view.show_original = !view.show_original;
                    cx.notify();
                })),
            );
        // Playback lives next to the text it affects: jumping frames or replaying
        // the current line used to require moving the pointer to the video panel.
        let mut playback_toolbar = div()
            .h_flex()
            .flex_wrap()
            .gap_1()
            .items_center()
            // A pill outline keeps the transport controls readable as one unit
            // now that the formatting row sits below them.
            .rounded(gpui_kit::px(999.))
            .border_1()
            .border_color(p.divider)
            .px_1()
            .py_1();
        for (id, label, icon, name) in [
            (
                "editor-play",
                "播放/暂停",
                if self.snapshot.playing {
                    IconName::Pause
                } else {
                    IconName::Play
                },
                "video/play",
            ),
            (
                "editor-prev-frame",
                "上一帧",
                IconName::ChevronLeft,
                "video/frame/prev",
            ),
            (
                "editor-next-frame",
                "下一帧",
                IconName::ChevronRight,
                "video/frame/next",
            ),
            (
                "editor-play-line-video",
                "播放当前行视频",
                IconName::Video,
                "video/play/line",
            ),
            (
                "editor-play-line-audio",
                "播放当前行音频",
                IconName::AudioLines,
                "audio/play/selection",
            ),
            ("editor-undo", "撤销", IconName::Undo2, "edit/undo"),
            ("editor-redo", "重做", IconName::Redo2, "edit/redo"),
        ] {
            let enabled = self
                .catalog
                .command(name)
                .is_some_and(|command| command.can_invoke());
            playback_toolbar = playback_toolbar.child(
                Button::new(id)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .child(Icon::new(icon).size(px(15.)))
                    .accessibility_label(label)
                    .tooltip(label)
                    .ghost()
                    .disabled(!enabled)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.invoke(json!({"op":"command", "name":name}), cx)
                    })),
            );
        }
        let row_count = visible.len();
        let properties_rail =
            rail.map(|width| self.render_properties(Some(width), cx).into_any_element());
        // Without the side rail the inspector stacks under the editor, and the
        // style/role/layer/effect row moves back into it.
        let stacked_properties = rail
            .is_none()
            .then(|| self.render_properties(None, cx).into_any_element());
        let rail_handle = rail.map(|rail_width| {
            div()
                .id("properties-rail-resize-handle")
                .role(gpui_kit::Role::Splitter)
                .aria_label("调整行属性宽度")
                .w(px(6.))
                .h_full()
                .flex_shrink_0()
                .cursor_col_resize()
                .h_flex()
                .items_center()
                .justify_center()
                .child(div().w(px(1.)).h_full().bg(p.divider))
                .hover(|v| v.bg(p.accent.opacity(0.18)))
                .on_mouse_down(
                    gpui_kit::MouseButton::Left,
                    cx.listener(move |view, event: &gpui_kit::MouseDownEvent, _, cx| {
                        view.layout_drag = Some(WorkspaceResizeDrag::PropertiesRail {
                            start_x: event.position.x,
                            start_width: rail_width,
                        });
                        cx.notify();
                    }),
                )
        });
        // The editor header navigates lines instead of resizing the panel: the
        // rail handle already resizes it, and stepping through dialogue is what
        // timing work actually needs.
        let (previous_line, next_line) = {
            let index = self
                .snapshot
                .lines
                .iter()
                .position(|line| line.id == self.selected_line);
            match index {
                Some(index) => (
                    index.checked_sub(1).map(|i| self.snapshot.lines[i].id),
                    self.snapshot.lines.get(index + 1).map(|line| line.id),
                ),
                None => (
                    self.snapshot.lines.last().map(|line| line.id),
                    self.snapshot.lines.first().map(|line| line.id),
                ),
            }
        };
        let editor_height = self.subtitle_editor_height;
        // The four line-text actions of the classic edit box, directly under the
        // text they rewrite; they used to be reachable only from the palette.
        let mut text_actions = div().h_flex().gap_1().items_center();
        for (id, label, tip, name, icon) in [
            (
                "text-revert",
                "还原",
                "恢复到载入本行时的原文",
                "edit/revert",
                IconName::Undo2,
            ),
            (
                "text-clear",
                "清除",
                "清空整行文本",
                "edit/clear",
                IconName::Eraser,
            ),
            (
                "text-clear-text",
                "清除文本",
                "清除文本，保留特效标签",
                "edit/clear/text",
                IconName::Delete,
            ),
            (
                "text-insert-original",
                "插入原文",
                "在光标处插入原文",
                "edit/insert_original",
                IconName::TextCursorInput,
            ),
        ] {
            let enabled = self
                .catalog
                .command(name)
                .is_some_and(|command| command.can_invoke());
            text_actions = text_actions.child(
                Button::new(id)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .icon(icon)
                    .label(label)
                    .tooltip(tip)
                    .ghost()
                    .disabled(!enabled)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.invoke(json!({"op": "command", "name": name}), cx)
                    })),
            );
        }
        // Header, formatting bar and the text box stay in one column so the
        // inspector can sit beside them as a second column.
        let editor_content = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            // Without a floor the editor loses every share negotiation to the
            // properties card and the list, and collapses to nothing.
            .min_h(px(150.))
            .gap_2()
            .when(rail.is_some(), |content| content.py_3().pl_3().pr_2())
            .when(rail.is_none(), |content| content.px_3().py_3())
            .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Icon::new(IconName::Pencil)
                                    .size(px(15.))
                                    .text_color(p.muted),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_sm()
                                    .font_semibold()
                                    .child("文本编辑"),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("previous-subtitle-line")
                                    .rounded(gpui_kit::px(999.))
                                    .small()
                                    .px(gpui_kit::px(9.))
                                    .py(gpui_kit::px(3.))
                                    .child(Icon::new(IconName::ChevronUp).size(px(15.)))
                                    .accessibility_label("上一条字幕")
                                    .tooltip("上一条字幕")
                                    .ghost()
                                    .disabled(previous_line.is_none())
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        if let Some(id) = previous_line {
                                            view.select_lines(id, false, false, cx);
                                        }
                                    })),
                            )
                            .child(
                                Button::new("next-subtitle-line")
                                    .rounded(gpui_kit::px(999.))
                                    .small()
                                    .px(gpui_kit::px(9.))
                                    .py(gpui_kit::px(3.))
                                    .child(Icon::new(IconName::ChevronDown).size(px(15.)))
                                    .accessibility_label("下一条字幕")
                                    .tooltip("下一条字幕")
                                    .ghost()
                                    .disabled(next_line.is_none())
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        if let Some(id) = next_line {
                                            view.select_lines(id, false, false, cx);
                                        }
                                    })),
                            )
                            .child(
                                Button::new("apply-edit").rounded(gpui_kit::px(999.)).small().px(gpui_kit::px(13.)).py(gpui_kit::px(3.))
                                    .icon(IconName::Check)
                                    .label("应用文本")
                                    .ghost()

                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let id = this.selected_line;
                                        let text = this.editor_text(cx);
                                        this.invoke(json!({"op":"edit", "id":id, "ids":this.snapshot.selected, "text":text}), cx);
                                    })),
                            ),
                    )
                    // A column child stretches to the card width; the pill has
                    // to hug its buttons, so a row wrapper sizes it.
                    .child(div().h_flex().child(playback_toolbar))
                    .child(format_toolbar)
                    .when(self.show_original, |col| {
                        col.child(
                            div()
                                .flex_shrink_0()
                                .truncate()
                                .text_sm()
                                .text_color(p.muted)
                                .child(self.editor_original.clone()),
                        )
                    })
                    .child(editor.flex_1().min_h(px(40.)))
                    .child(text_actions);
        let editor_section = div()
            .id("subtitle-editor-section")
            .flex_shrink_0()
            .min_w_0()
            .when(rail.is_some(), |section| section.h_flex().items_stretch())
            .when(rail.is_none(), |section| section.v_flex())
            .rounded_t(px(crate::workspace_theme::RADIUS_LG - 1.))
            .bg(p.surface)
            .border_b_1()
            .border_color(border)
            .child(editor_content)
            .children(rail_handle)
            .children(properties_rail)
            .children(stacked_properties);
        let subtitle_list = div()
            .v_flex()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(
                div()
                    .h_flex()
                    .h(px(44.))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .flex_shrink_0()
                    // The card clips to a rectangle, so a tinted strip needs its
                    // own rounded corners to stay inside the card outline.
                    .rounded_t(px(crate::workspace_theme::RADIUS_LG - 1.))
                    .bg(p.raised)
                    .border_b_1()
                    .border_color(p.divider)
                    .child(
                        Icon::new(IconName::Captions)
                            .size(px(15.))
                            .text_color(p.muted),
                    )
                    .child(div().text_sm().font_semibold().child("字幕列表"))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(p.muted)
                            .child(format!("{} 行", row_count)),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("subtitle-columns")
                            .rounded(gpui_kit::px(999.))
                            .small()
                            .px(gpui_kit::px(13.))
                            .py(gpui_kit::px(3.))
                            .icon(IconName::Columns3)
                            .label("列")
                            .tooltip("添加或移除字幕列表的列")
                            .ghost()
                            .dropdown_menu(move |mut menu, _, _| {
                                let weak = menu_weak.clone();
                                for column in SubtitleColumn::optional() {
                                    let weak = weak.clone();
                                    let active = active_columns.contains(&column);
                                    menu = menu.item(
                                        PopupMenuItem::new(column.label())
                                            .checked(active)
                                            .on_click(move |_, _, cx| {
                                                let _ = weak.update(cx, |view, cx| {
                                                    view.toggle_subtitle_column(column);
                                                    cx.notify();
                                                });
                                            }),
                                    );
                                }
                                menu
                            }),
                    )
                    .child(
                        Input::new(&self.search)
                            .id("subtitle-search")
                            .prefix(Icon::new(IconName::Search).size(px(13.)))
                            .w(px(190.))
                            .min_w_0()
                            .flex_shrink(1.),
                    ),
            )
            .child(
                div()
                    .h_flex()
                    .h(px(32.))
                    .flex_shrink_0()
                    .px_3()
                    // Rows inset their text by the selection bar's 2px; the
                    // header needs the same inset or its labels sit 2px off.
                    .pl(px(14.))
                    .items_center()
                    .bg(header_bg)
                    .border_b_1()
                    .border_color(border)
                    .children(columns.iter().map(|(column, width)| {
                        let column = *column;
                        let width = *width;
                        div()
                            .h_flex()
                            .h_full()
                            .items_center()
                            .relative()
                            .w(px(width))
                            .flex_shrink_0()
                            .mr(px(COLUMN_GAP))
                            .text_size(px(secondary_size))
                            .text_color(muted)
                            .child(column.label())
                            // Only the divider resizes the column: the label
                            // keeps the normal cursor and its full hit area.
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .right(px(-3.))
                                    .w(px(7.))
                                    .cursor_col_resize()
                                    .hover(|grip| grip.bg(muted.opacity(0.12)))
                                    .on_mouse_down(
                                        gpui_kit::MouseButton::Left,
                                        cx.listener(
                                            move |view, event: &gpui_kit::MouseDownEvent, _, cx| {
                                                view.layout_drag =
                                                    Some(WorkspaceResizeDrag::SubtitleColumn {
                                                        column,
                                                        start_x: event.position.x,
                                                        start_width: width,
                                                    });
                                                cx.stop_propagation();
                                                cx.notify();
                                            },
                                        ),
                                    ),
                            )
                            .child(column_rule(muted.opacity(0.28)))
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(MIN_TEXT_COLUMN))
                            .text_size(px(secondary_size))
                            .text_color(muted)
                            .child("字幕文本"),
                    )
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(90.))
                    // A drag ends wherever the button is released, including on
                    // the padding between rows, so the viewport clears it and
                    // not just the row that started it.
                    .capture_any_mouse_up(cx.listener(
                        |view, _: &gpui_kit::MouseUpEvent, _, cx| {
                            if view.grid_drag_anchor.take().is_some() {
                                cx.notify();
                            }
                        },
                    ))
                    .child(
                        // The grid needs its own width to decide whether the
                        // columns still fit, and layout only knows it here.
                        canvas(
                            move |bounds, _, cx| {
                                let width = f32::from(bounds.size.width);
                                let _ = size_weak.update(cx, |view, cx| {
                                    if (view.subtitle_grid_width - width).abs() > 1. {
                                        view.subtitle_grid_width = width;
                                        cx.notify();
                                    }
                                });
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(
                        uniform_list(
                            "subtitle-rows",
                            row_count,
                            cx.processor(move |_, range: Range<usize>, _, _| {
                                range
                                    .map(|index| {
                                        let line = visible[index].clone();
                                        let is_active = selected.contains(&line.id);
                                        let row_bg = if is_active && line.comment {
                                            selected_comment_bg
                                        } else if is_active {
                                            selected_bg
                                        } else if line.comment {
                                            comment_bg
                                        } else if highlight_visible
                                            && line.start <= video_time
                                            && video_time < line.end
                                        {
                                            inframe_bg
                                        } else if line.row % 2 == 1 {
                                            stripe
                                        } else {
                                            line_bg
                                        };
                                        let weak = weak.clone();
                                        div()
                                        .id(format!("subtitle-row-{}", line.id))
                                        .h(px(row_height))
                                        .text_size(px(grid_size))
                                        .whitespace_nowrap()
                                        .when(!grid_font.is_empty(), |row| {
                                            row.font_family(grid_font.clone())
                                        })
                                        .relative()
                                        .w_full()
                                        .h_flex()
                                        .items_center()
                                        .px_3()
                                        .pl(px(14.))
                                        .bg(row_bg)
                                        .border_b_1()
                                        .border_color(border)
                                        .when(!is_active && line.comment, |row| {
                                            // Comments get their own stripe so the
                                            // tinted background is not the only cue.
                                            row.child(
                                                div()
                                                    .ml(px(-14.))
                                                    .mr(px(12.))
                                                    .h_full()
                                                    .w(px(2.))
                                                    .flex_shrink_0()
                                                    .bg(p.warning.opacity(0.75)),
                                            )
                                        })
                                        .when(is_active, |row| {
                                            // The selection accent is its own bar:
                                            // a left border shares one colour with
                                            // the divider, which also turned the
                                            // row's bottom line blue.
                                            row.child(
                                                div()
                                                    .ml(px(-14.))
                                                    .mr(px(12.))
                                                    .h_full()
                                                    .w(px(2.))
                                                    .flex_shrink_0()
                                                    .bg(accent),
                                            )
                                        })
                                        .cursor_pointer()
                                        .on_mouse_down(gpui_kit::MouseButton::Left, {
                                            let weak = weak.clone();
                                            let id = line.id;
                                            move |event: &gpui_kit::MouseDownEvent, window, cx| {
                                                let _ = weak.update(cx, |view, cx| {
                                                    view.key_focus = KeyFocus::Grid;
                                                    if focus_grid {
                                                        view.focus_handle.focus(window, cx);
                                                    }
                                                    // The press both selects and arms a
                                                    // drag, so sliding from here paints
                                                    // a block starting on this row.
                                                    view.grid_drag_anchor = Some(id);
                                                    view.grid_drag_row = id;
                                                    view.follow_suppressed = true;
                                                    if event.click_count == 2 {
                                                        view.invoke(json!({"op":"select", "id":id, "ids":[id], "double_click":true}), cx);
                                                        return;
                                                    }
                                                    view.select_lines(
                                                        id,
                                                        event.modifiers.shift,
                                                        event.modifiers.control
                                                            || event.modifiers.platform,
                                                        cx,
                                                    );
                                                });
                                            }
                                        })
                                        .on_mouse_down(gpui_kit::MouseButton::Right, {
                                            let weak = weak.clone();
                                            move |_, window, cx| {
                                                let _ = weak.update(cx, |view, cx| {
                                                    view.key_focus = KeyFocus::Grid;
                                                    view.focus_handle.focus(window, cx);
                                                });
                                            }
                                        })
                                        .on_mouse_move({
                                            let weak = weak.clone();
                                            let id = line.id;
                                            move |event: &gpui_kit::MouseMoveEvent, _, cx| {
                                                let _ = weak.update(cx, |view, cx| {
                                                    if event.pressed_button
                                                        != Some(gpui_kit::MouseButton::Left)
                                                    {
                                                        view.grid_drag_anchor = None;
                                                        return;
                                                    }
                                                    let Some(anchor) = view.grid_drag_anchor
                                                    else {
                                                        return;
                                                    };
                                                    if view.grid_drag_row == id {
                                                        return;
                                                    }
                                                    view.grid_drag_row = id;
                                                    view.follow_suppressed = true;
                                                    view.select_range(anchor, id, cx);
                                                });
                                            }
                                        })
                                        .children(columns.iter().map(|(column, width)| {
                                            let column = *column;
                                            subtitle_cell(
                                                column,
                                                *width,
                                                &line,
                                                secondary_size,
                                                time_font.clone(),
                                                muted,
                                                accent,
                                                accent.opacity(0.28),
                                                frame_rate,
                                            )
                                        }))
                                        .children({
                                            // A glyph per special tag, parked at the row's
                                            // right edge so it never squeezes the columns.
                                            let mut badges =
                                                div().h_flex().items_center().gap_1().flex_shrink_0();
                                            let mut any = false;
                                            if line.comment {
                                                badges = badges
                                                    .child(tag_badge(IconName::Captions, p.warning));
                                                any = true;
                                            }
                                            if has_karaoke_tag(&line.text) {
                                                badges =
                                                    badges.child(tag_badge(IconName::AudioLines, accent));
                                                any = true;
                                            }
                                            if has_effect_tag(&line.text) {
                                                badges =
                                                    badges.child(tag_text_badge("FX", accent));
                                                any = true;
                                            }
                                            any.then_some(div().pl(px(8.)).flex_shrink_0().child(badges))
                                        })
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w(px(MIN_TEXT_COLUMN))
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .text_size(px(grid_size))
                                                .text_color(if is_active {
                                                    selected_foreground
                                                } else {
                                                    foreground
                                                })
                                                .child(line.display_text),
                                        )
                                        .context_menu({
                                            let weak = weak.clone();
                                            let id = line.id;
                                            move |menu, window, cx| {
                                                subtitle_context_menu(menu, id, weak.clone(), window, cx)
                                            }
                                        })
                                    })
                                    .collect()
                            }),
                        )
                        .track_scroll(&self.list_scroll_handle)
                        .size_full(),
                    ),
            );
        div()
            .relative()
            .v_flex()
            .flex_1()
            .min_w_0()
            .min_h(px(300.))
            .overflow_hidden()
            .rounded(px(crate::workspace_theme::RADIUS_LG))
            .border_1()
            .border_color(if focused { p.accent } else { p.border })
            .bg(line_bg)
            // The list viewport paints a square background that gpui clips to
            // a rectangle, so it covers the card's rounded bottom corners.
            // Stop the list a radius short of the edge instead.
            .pb(px(crate::workspace_theme::RADIUS_LG))
            .shadow(crate::page::card_shadow(dark))
            // The stacked properties card is a child of this section, so the
            // section has to budget for it; otherwise the card is laid out over
            // the editor's toolbars and clips them.
            .child(
                editor_section
                    .h(px(editor_height
                        + if rail.is_none() {
                            self.properties_height()
                        } else {
                            0.
                        }))
                    .flex_shrink_0(),
            )
            .child(
                div()
                    .id("subtitle-editor-list-resize-handle")
                    .aria_label("调整字幕编辑区高度")
                    .h(px(6.))
                    .w_full()
                    .flex_shrink_0()
                    .h_flex()
                    .items_center()
                    .justify_center()
                    .cursor_row_resize()
                    .bg(p.surface)
                    .child(
                        div()
                            .w(px(36.))
                            .h(px(2.))
                            .rounded(px(1.))
                            .bg(p.muted.opacity(0.7)),
                    )
                    .on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(|view, event: &gpui_kit::MouseDownEvent, _, cx| {
                            view.layout_drag = Some(WorkspaceResizeDrag::SubtitleEditor {
                                start_y: event.position.y,
                                start_height: view.subtitle_editor_height,
                            });
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .child(subtitle_list.flex_1().min_h_0())
    }
}

// Header and body ticks share geometry so each column boundary stays aligned.
fn column_rule(colour: gpui_kit::Hsla) -> gpui_kit::Div {
    div()
        .absolute()
        .top(px(8.))
        .bottom(px(8.))
        .right_0()
        .w(px(1.))
        .bg(colour)
}

fn subtitle_cell(
    column: SubtitleColumn,
    width: f32,
    line: &Line,
    secondary_size: f32,
    time_font: gpui_kit::SharedString,
    muted: gpui_kit::Hsla,
    accent: gpui_kit::Hsla,
    divider: gpui_kit::Hsla,
    frame_rate: Option<f64>,
) -> gpui_kit::AnyElement {
    let cell = || {
        div()
            .w(px(width))
            .h_full()
            .flex()
            .items_center()
            .relative()
            .flex_shrink_0()
            // Fixed-width columns overflowed into the next one when the grid
            // font grew; clip instead of letting the time run under the style.
            .overflow_hidden()
            .mr(px(COLUMN_GAP))
            .text_size(px(secondary_size))
            .child(column_rule(divider))
    };
    match column {
        SubtitleColumn::IdLayer => cell()
            .text_color(muted)
            .child(format!("{:03} / {}", line.row + 1, line.layer))
            .into_any_element(),
        SubtitleColumn::Time => {
            // Same numbers the start/end fields show once 按帧显示 is on.
            let range = match frame_rate {
                Some(fps) => format!(
                    "{} → {}",
                    (line.start as f64 * fps / 1000.).round() as i64,
                    (line.end as f64 * fps / 1000.).round() as i64
                ),
                None => format!("{} → {}", format_time(line.start), format_time(line.end)),
            };
            cell()
                .font_family(time_font)
                .text_color(muted)
                .child(range)
                .into_any_element()
        }
        SubtitleColumn::StyleActor => {
            // Style and actor share one line now: the stacked pair doubled the
            // row height for two short labels. Style keeps the accent colour and
            // the actor truncates first.
            let mut row = cell()
                .h_flex()
                .items_center()
                .gap_1()
                .min_w_0()
                .overflow_hidden()
                .child(
                    div()
                        .flex_shrink_0()
                        .max_w(px(width * 0.6))
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(accent)
                        .child(line.style.clone()),
                );
            if !line.actor.is_empty() {
                row = row.child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(muted)
                        .child(line.actor.clone()),
                );
            }
            row.into_any_element()
        }
        SubtitleColumn::Layer => cell()
            .text_color(muted)
            .child(line.layer.to_string())
            .into_any_element(),
        SubtitleColumn::Effect => cell()
            .min_w_0()
            .overflow_hidden()
            .text_ellipsis()
            .text_color(muted)
            .child(line.effect.clone())
            .into_any_element(),
        SubtitleColumn::Actor => cell()
            .min_w_0()
            .overflow_hidden()
            .text_ellipsis()
            .text_color(muted)
            .child(line.actor.clone())
            .into_any_element(),
        SubtitleColumn::Comment => cell()
            .text_color(if line.comment { accent } else { muted })
            .child(if line.comment { "注释" } else { "对白" })
            .into_any_element(),
    }
}

/// Parses `#RRGGBB`, `#RRGGBBAA` or the same without the hash.
fn font_flag_button(
    cx: &mut Context<Workspace>,
    id: &'static str,
    label: &'static str,
    checked: bool,
    toggle: fn(&mut FontState),
) -> gpui_kit::AnyElement {
    Button::new(id)
        .label(label)
        .rounded(px(999.))
        .small()
        .px(px(13.))
        .py(px(3.))
        .when(checked, |button| button.primary())
        .when(!checked, |button| button.ghost())
        .on_click(cx.listener(move |view, _, _, cx| {
            if let Some(HostDialog::Font { state, .. }) = view.host_dialog.as_mut() {
                toggle(state);
            }
            cx.notify();
        }))
        .into_any_element()
}

fn format_time(ms: i64) -> String {
    let total = ms.max(0);
    format!(
        "{}:{:02}:{:02}.{:02}",
        total / 3600000,
        (total / 60000) % 60,
        (total / 1000) % 60,
        (total % 1000) / 10
    )
}

fn parse_time(value: &str) -> Result<i64> {
    let parts: Vec<_> = value.trim().split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return Err(anyhow!("请输入 时:分:秒.百分秒"));
    }
    let seconds: Vec<_> = parts[parts.len() - 1].split('.').collect();
    if seconds.len() > 2 {
        return Err(anyhow!("无效的时间码"));
    }
    let sec = seconds[0].parse::<i64>()?;
    let min = parts[parts.len() - 2].parse::<i64>()?;
    let hour = if parts.len() == 3 {
        parts[0].parse::<i64>()?
    } else {
        0
    };
    let fraction = seconds.get(1).copied().unwrap_or("");
    if hour < 0
        || min < 0
        || sec < 0
        || min >= 60
        || sec >= 60
        || fraction.len() > 3
        || !fraction.chars().all(|ch| ch.is_ascii_digit())
    {
        return Err(anyhow!("无效的时间码"));
    }
    let ms = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<i64>()? * 10_i64.pow(3 - fraction.len() as u32)
    };
    hour.checked_mul(3600000)
        .and_then(|total| total.checked_add(min * 60000 + sec * 1000 + ms))
        .filter(|total| *total <= i32::MAX as i64 - 10)
        .ok_or_else(|| anyhow!("时间码超出范围"))
}

fn set_status(
    weak: &gpui_kit::WeakEntity<Workspace>,
    cx: &mut gpui_kit::AsyncApp,
    message: String,
) {
    if let Some(view) = weak.upgrade() {
        let _ = view.update(cx, |view, cx| {
            view.status = message;
            cx.notify();
        });
    }
}

async fn invoke_weak(
    bridge: &Bridge,
    weak: &gpui_kit::WeakEntity<Workspace>,
    cx: &mut gpui_kit::AsyncApp,
    payload: Value,
) -> bool {
    match bridge.call(payload).await {
        Ok(_) => match bridge.snapshot().await {
            Ok(snapshot) => {
                if let Some(view) = weak.upgrade() {
                    let _ = view.update(cx, |view, cx| {
                        view.status = "已同步当前项目".into();
                        view.apply_snapshot(snapshot, cx);
                    });
                }
                true
            }
            Err(error) => {
                set_status(weak, cx, format!("同步失败：{error}"));
                false
            }
        },
        Err(error) => {
            set_status(weak, cx, format!("操作失败：{error}"));
            false
        }
    }
}
impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus_area = self.focus_area(window, cx);
        self.sync_native_video(window, cx);
        // A track whose measured size changed is requested again at its new
        // raster size. The bounds read here were measured by the previous paint.
        let track = *self.audio.bounds.borrow();
        let track_size = (f32::from(track.size.width), f32::from(track.size.height));
        if track_size != self.audio.last_track {
            self.audio.last_track = track_size;
            self.refresh_audio(cx);
        }
        if let Some(line) = self
            .snapshot
            .lines
            .iter()
            .find(|line| line.id == self.selected_line)
        {
            if self.editor_line_id != line.id {
                // "Show original" means the text this line had when it was picked,
                // so it is captured on selection rather than every frame.
                self.editor_original = line.text.clone();
                let text = line.text.clone();
                self.editor
                    .update(cx, |state, cx| state.set_value(text, window, cx));
                self.editor_line_id = line.id;
                self.editor_dirty_line = -1;
            } else if self.editor_dirty_line != line.id
                && !self.editor.read(cx).focus_handle(cx).is_focused(window)
                && self.editor.read(cx).value().as_ref() != line.text
            {
                // A change made in Aegisub (undo, Lua, or the original editor) replaces the local draft.
                // Never while the editor has focus: `set_value` resets the selection to
                // 0..0, and doing that in the middle of an IME composition throws the
                // caret back to the start of the line.
                let text = line.text.clone();
                self.editor
                    .update(cx, |state, cx| state.set_value(text, window, cx));
            }
        }
        if let Some([start, end]) = self.pending_text_selection.take() {
            self.editor
                .update(cx, |state, cx| state.set_selected_range(start..end, cx));
        }
        if let Some(line) = self
            .snapshot
            .lines
            .iter()
            .find(|line| line.id == self.selected_line)
        {
            if self.properties_line_id != line.id || !self.properties_dirty {
                for (key, input) in &self.property_inputs {
                    let value = match *key {
                        "start" | "end" => {
                            let ms = if self.snapshot.audio.loaded {
                                if *key == "start" {
                                    self.snapshot.audio.start
                                } else {
                                    self.snapshot.audio.end
                                }
                            } else if *key == "start" {
                                line.start
                            } else {
                                line.end
                            };
                            if self.time_in_frames {
                                self.snapshot
                                    .fps
                                    .filter(|fps| *fps > 0.)
                                    .map(|fps| (ms as f64 * fps / 1000.).round() as i64)
                                    .map(|frame| frame.to_string())
                                    .unwrap_or_else(|| format_time(ms))
                            } else {
                                format_time(ms)
                            }
                        }
                        "layer" => line.layer.to_string(),
                        "style" => line.style.clone(),
                        "actor" => line.actor.clone(),
                        "effect" => line.effect.clone(),
                        "margin_left" => line.margin[0].to_string(),
                        "margin_right" => line.margin[1].to_string(),
                        _ => line.margin[2].to_string(),
                    };
                    if input.read(cx).value().as_ref() != value {
                        input.update(cx, |state, cx| state.set_value(value, window, cx));
                    }
                }
                self.properties_line_id = line.id;
                self.properties_dirty = false;
            }
        }
        if !self.timeline.dragging
            && !self.timeline.seeking
            && self.timeline.pending_frame.is_none()
        {
            let position =
                self.snapshot.frame as f32 / (self.snapshot.frame_count - 1).max(1) as f32;
            if (self.timeline.slider.read(cx).value().start() - position).abs() > 0.000001 {
                self.timeline
                    .slider
                    .update(cx, |state, cx| state.set_value(position, window, cx));
            }
        }
        if let Some(text) = self.translation.pending_text.take() {
            self.translation
                .editor
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        if self.translation.focus_pending && self.active_panel == Panel::Translation {
            self.translation.focus_pending = false;
            self.translation
                .editor
                .update(cx, |state, cx| state.focus(window, cx));
        }
        if self.active_panel == Panel::Styling {
            self.styling.sync(&self.snapshot, window, cx);
        }
        self.document.sync(window, cx);
        self.timing.sync(window, cx);
        self.resample.sync(window, cx);
        self.processor.sync(window, cx);
        self.video_settings.sync(window, cx);
        if self.active_panel == Panel::Spelling {
            self.spelling.sync(window, cx);
        }
        if self.active_panel == Panel::Preferences {
            PreferencesPanel::sync(self, window, cx);
        }
        if self.active_panel == Panel::Hotkeys {
            self.hotkeys.sync(window, cx);
        }
        if self.active_panel == Panel::Search {
            self.find_panel.sync(window, cx);
        }
        let dark = Theme::global(cx).is_dark();
        let p = Palette::new(dark);
        let canvas = p.canvas;
        let border = p.divider;
        let muted = p.muted;
        let root = div()
            .id("aegisub-gpui-workspace")
            .track_focus(&self.focus_handle)
            .on_drop(cx.listener(|view, paths: &ExternalPaths, _, cx| view.open_dropped(paths, cx)))
            .on_mouse_move(
                cx.listener(|view, event: &gpui_kit::MouseMoveEvent, _, cx| {
                    if let Some(drag) = view.layout_drag {
                        if event.pressed_button == Some(gpui_kit::MouseButton::Left) {
                            match drag {
                                WorkspaceResizeDrag::SubtitleEditor {
                                    start_y,
                                    start_height,
                                } => {
                                    view.subtitle_editor_height = (start_height
                                        + f32::from(event.position.y - start_y))
                                    .clamp(210., 420.);
                                }
                                WorkspaceResizeDrag::AudioPanel {
                                    start_y,
                                    start_height,
                                } => {
                                    view.audio_panel_height = (start_height
                                        + f32::from(event.position.y - start_y))
                                    .clamp(64., 320.);
                                }
                                WorkspaceResizeDrag::PropertiesRail {
                                    start_x,
                                    start_width,
                                } => {
                                    // The inspector sits on the right, so dragging
                                    // the handle left makes it wider.
                                    view.properties_rail_width = Some(
                                        (start_width - f32::from(event.position.x - start_x))
                                            .clamp(200., 520.),
                                    );
                                }
                                WorkspaceResizeDrag::ColumnSplit {
                                    start_x,
                                    start_width,
                                } => {
                                    view.column_split = Some(
                                        (start_width + f32::from(event.position.x - start_x))
                                            .max(280.),
                                    );
                                }
                                WorkspaceResizeDrag::SubtitleColumn {
                                    column,
                                    start_x,
                                    start_width,
                                } => {
                                    let width = (start_width
                                        + f32::from(event.position.x - start_x))
                                    .clamp(40., 600.);
                                    view.set_subtitle_column_width(column, width);
                                }
                                WorkspaceResizeDrag::StylePreview {
                                    start_y,
                                    start_height,
                                } => {
                                    view.style_preview_height = (start_height
                                        + f32::from(event.position.y - start_y))
                                    .clamp(64., 320.);
                                }
                            }
                            cx.notify();
                        } else {
                            // The button came up somewhere the drag root never
                            // saw: end the gesture here and persist it, or the
                            // new width would only live until the next launch.
                            if matches!(drag, WorkspaceResizeDrag::SubtitleColumn { .. }) {
                                view.save_subtitle_widths(cx);
                            }
                            view.layout_drag = None;
                        }
                    }
                }),
            )
            .on_mouse_up(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    if let Some(drag) = view.layout_drag.take() {
                        if matches!(drag, WorkspaceResizeDrag::SubtitleColumn { .. }) {
                            view.save_subtitle_widths(cx);
                        }
                        cx.notify();
                    }
                }),
            )
            .capture_any_mouse_up(cx.listener(|view, _, _, cx| {
                if let Some(drag) = view.layout_drag.take() {
                    if matches!(drag, WorkspaceResizeDrag::SubtitleColumn { .. }) {
                        view.save_subtitle_widths(cx);
                    }
                    cx.notify();
                }
            }))
            .size_full()
            .overflow_hidden()
            .h_flex()
            .items_stretch()
            .bg(canvas)
            .text_color(p.text);
        let mut body = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden();
        if self.active_panel == Panel::Subtitles {
            let show_video = self.show_video && !self.video_popped;
            // Two columns need room for the preview beside the editor: below
            // 1080 logical pixels the preview stacks under the waveform.
            let side_by_side = show_video && f32::from(window.viewport_size().width) >= 1080.;
            let sidebar = if self
                .sidebar_collapsed
                .unwrap_or(f32::from(window.viewport_size().width) < 1150.)
            {
                60.
            } else {
                164.
            };
            let body_width = (f32::from(window.viewport_size().width) - sidebar - 48.).max(320.);
            let column_width = self
                .column_split
                .unwrap_or((body_width - 12.) / 2.)
                .clamp(280., (body_width - 280.).max(280.));
            // The inspector stacks under the editor, so there is no side rail
            // to size or drag any more.
            let properties_rail: Option<f32> = None;
            let grip = p.grip.opacity(0.7);
            let audio_handle = div()
                .id("audio-panel-resize-handle")
                .aria_label("调整音频波形高度")
                .role(gpui_kit::Role::Splitter)
                .h(px(4.))
                .w_full()
                .flex_shrink_0()
                .h_flex()
                .items_center()
                .justify_center()
                .cursor_row_resize()
                .child(div().w(px(48.)).h(px(3.)).rounded(px(2.)).bg(grip))
                .on_mouse_down(
                    gpui_kit::MouseButton::Left,
                    cx.listener(|view, event: &gpui_kit::MouseDownEvent, _, cx| {
                        view.layout_drag = Some(WorkspaceResizeDrag::AudioPanel {
                            start_y: event.position.y,
                            start_height: view.audio_panel_height,
                        });
                        cx.stop_propagation();
                        cx.notify();
                    }),
                );
            let column_handle = div()
                .id("workspace-column-resize-handle")
                .aria_label("调整视频与字幕列表宽度")
                .role(gpui_kit::Role::Splitter)
                .w(px(4.))
                .h_full()
                .flex_shrink_0()
                .v_flex()
                .items_center()
                .justify_center()
                .cursor_col_resize()
                .child(div().h(px(48.)).w(px(3.)).rounded(px(2.)).bg(grip))
                .on_mouse_down(
                    gpui_kit::MouseButton::Left,
                    cx.listener(move |view, event: &gpui_kit::MouseDownEvent, _, cx| {
                        view.layout_drag = Some(WorkspaceResizeDrag::ColumnSplit {
                            start_x: event.position.x,
                            start_width: column_width,
                        });
                        cx.stop_propagation();
                        cx.notify();
                    }),
                );
            let line_text = self.timed_line_text();
            let audio_strip = self
                .audio
                .render_compact(
                    &self.snapshot.audio,
                    self.snapshot.video,
                    &self.snapshot.appearance,
                    self.audio_panel_height,
                    focus_area == KeyFocus::Audio,
                    &line_text,
                    cx,
                )
                .into_any_element();
            let player = self
                .render_player(window, side_by_side, focus_area == KeyFocus::Video, cx)
                .into_any_element();
            let mut columns = div()
                .h_flex()
                .items_stretch()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .gap_1();
            if self.show_audio || show_video {
                let mut left = div()
                    .v_flex()
                    .when(side_by_side, |column| {
                        column.w(px(column_width)).h_full().flex_shrink_0()
                    })
                    .when(!side_by_side, |column| column.w_full().flex_shrink_0())
                    .min_w_0()
                    .min_h_0()
                    .gap_0p5();
                if self.show_audio {
                    left = left.child(audio_strip).child(audio_handle);
                }
                if show_video {
                    // No handle under the preview: it fills whatever the audio
                    // strip leaves, so there is no height to trade.
                    left = left.child(player);
                }
                columns = columns.child(left);
                if side_by_side {
                    columns = columns.child(column_handle);
                }
            }
            columns = columns.child({
                // The editor and the subtitle list are one region: they share
                // a single outline so the highlight says which area owns the
                // keyboard, not which card was clicked last.
                let p = crate::workspace_theme::Palette::current(cx);
                let focused = focus_area == KeyFocus::Grid;
                div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .rounded(px(crate::workspace_theme::RADIUS_LG))
                    .border_1()
                    .border_color(if focused { p.accent } else { p.border })
                    .child(self.render_subtitle_panel(properties_rail, focused, cx))
            });
            if !side_by_side {
                columns = columns.flex_col();
            }
            body = body.child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .gap_1()
                    .p_2()
                    .child(columns),
            );
        } else if self.active_panel == Panel::Video {
            // The popped-out preview owns the whole window: picture, readout and
            // transport, with the same view state the embedded card uses.
            body = body.child(self.render_player(window, true, true, cx));
        } else if self.active_panel == Panel::Visual {
            body = body.child(
                page::page(cx, "visual-panel")
                    .child(self.visual.toolbar(cx))
                    .child(self.render_player(window, true, true, cx))
                    .child(page::hint(
                        cx,
                        "双击十字定位；拖动控制点调整，Shift 约束，Ctrl/⌘ 多选。",
                    )),
            );
        } else if self.active_panel == Panel::Commands {
            body = body.child(self.render_commands(cx));
        } else if self.active_panel == Panel::Styling {
            body = body.child(
                self.styling
                    .render(&self.snapshot, &self.document.details, cx),
            );
        } else if self.active_panel == Panel::Translation {
            body = body.child(self.translation.render(
                self.snapshot.video,
                self.snapshot.audio.loaded,
                cx,
            ));
        } else if self.active_panel == Panel::Automation {
            body = body.child(self.automation.render(&self.catalog, cx));
        } else if self.active_panel == Panel::Spelling {
            body = body.child(self.spelling.render(cx));
        } else if self.active_panel == Panel::Files {
            body = body.child(self.files.render(cx));
        } else if self.active_panel == Panel::Preferences {
            body = body.child(self.preferences.render(cx));
        } else if self.active_panel == Panel::Hotkeys {
            body = body.child(self.hotkeys.render(&self.catalog, cx));
        } else if self.active_panel == Panel::Attachments {
            body = body.child(self.attachments.render(cx));
        } else if self.active_panel == Panel::VideoSettings {
            body = body.child(self.video_settings.render(cx));
        } else if self.active_panel == Panel::Paste {
            body = body.child(self.paste.render(&self.snapshot, cx));
        } else if self.active_panel == Panel::Search {
            body = body.child(self.find_panel.render(&self.snapshot, cx));
        } else if self.active_panel == Panel::Processor {
            body = body.child(self.processor.render(cx));
        } else if self.active_panel == Panel::Resample {
            body = body.child(self.resample.render(cx));
        } else if self.active_panel == Panel::Timing {
            body = body.child(self.timing.render(&self.snapshot, cx));
        } else if self.active_panel == Panel::Styles {
            self.refresh_style_preview(cx);
            body = body.child(self.document.render_styles(self.style_preview_height, cx));
        } else {
            body = body.child(self.document.render_project(cx));
        }
        // Pages with their own toolbars do not own the keyboard shortcuts.
        let hints = match self.active_panel {
            Panel::Subtitles | Panel::Visual => self.shortcut_hints(self.key_context(window, cx)),
            _ => None,
        };
        let body = body.child(
            div()
                .h_flex()
                .h(px(26.))
                .flex_shrink_0()
                .px_3()
                .gap_4()
                .items_center()
                .justify_between()
                .bg(p.surface)
                .border_t_1()
                .border_color(border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(self.status.clone()),
                )
                .when_some(hints, |bar, (label, items)| {
                    bar.child(
                        div()
                            .h_flex()
                            .flex_shrink_0()
                            .gap_3()
                            .items_center()
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(div().flex_shrink_0().child(label))
                            .children(items.into_iter().map(|(key, text)| {
                                div()
                                    .h_flex()
                                    .flex_shrink_0()
                                    .gap_1()
                                    .items_center()
                                    .child(
                                        div()
                                            .px(px(5.))
                                            .py(px(1.))
                                            .rounded(px(crate::workspace_theme::RADIUS_SM - 3.))
                                            .bg(p.inset)
                                            .border_1()
                                            .border_color(border)
                                            .text_size(px(10.))
                                            .child(key),
                                    )
                                    .child(div().child(text))
                            })),
                    )
                })
                .child(
                    div()
                        .h_flex()
                        .gap_3()
                        .text_xs()
                        .text_color(muted)
                        .child(if self.snapshot.video {
                            let frame = if self.video_view.frame >= 0 {
                                self.video_view.frame
                            } else {
                                self.snapshot.frame
                            };
                            format!("视频帧 {} / {}", frame + 1, self.snapshot.frame_count)
                        } else {
                            "无视频".into()
                        })
                        .child(if self.snapshot.file.is_empty() {
                            "未命名字幕".to_owned()
                        } else {
                            std::path::Path::new(&self.snapshot.file)
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("未命名字幕")
                                .to_owned()
                        }),
                ),
        );
        // A detached window shows one tool and nothing else: no sidebar, no
        // legacy menu bar, no document toolbar.
        let detached = self.detached.is_some();
        let workspace = root
            .when(!detached, |v| v.child(self.render_menu(window, cx)))
            .child(
                div()
                    .v_flex()
                    .h_full()
                    .overflow_hidden()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .when(!cfg!(target_os = "macos") && !detached, |bar| {
                        bar.child(self.render_legacy_menus(cx))
                    })
                    .when(self.snapshot.show_toolbar && !detached, |v| {
                        v.child(self.render_toolbar(cx))
                    })
                    // A detached window has no chrome around the page, so the
                    // page's own padding is all that keeps content off the edge.
                    .when(detached, |v| v.p_3())
                    .child(body),
            );
        let mut layered = div()
            .relative()
            .size_full()
            .capture_key_down(cx.listener(Self::handle_key))
            .child(workspace);
        if let Some(card) = self.render_color_dialog(window, cx) {
            layered = layered.child(
                div()
                    .absolute()
                    .inset_0()
                    .h_flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui_kit::hsla(0., 0., 0., 0.42))
                    .on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(|view, _, _, cx| view.cancel_color_dialog(cx)),
                    )
                    .child(card),
            );
        }
        if let Some(card) = self.render_host_dialog(window, cx) {
            layered = layered.child(
                div()
                    .absolute()
                    .inset_0()
                    .h_flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui_kit::hsla(0., 0., 0., 0.42))
                    .on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(|view, _, _, cx| view.close_host_dialog(cx)),
                    )
                    .child(card),
            );
        }
        if let Some(card) = self.render_progress_card(cx) {
            layered = layered.child(
                div()
                    .absolute()
                    .inset_0()
                    .h_flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui_kit::hsla(0., 0., 0., 0.42))
                    .child(card),
            );
        }
        if let Some(card) = self.render_script_dialog(window, cx) {
            layered = layered.child(card);
        }
        if self.notices.is_empty() {
            return layered.into_any_element();
        }
        let notice = self.notices[0].clone();
        let remaining = self.notices.len() - 1;
        let suppress_checked = self.notice_suppress && !notice.suppress.is_empty();
        let card = div()
            .id("host-notice")
            .w(px(520.))
            .max_w_full()
            .v_flex()
            .rounded(px(workspace_theme::RADIUS_LG))
            .border_1()
            .border_color(border)
            .bg(p.surface)
            .overflow_hidden()
            .shadow_lg()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .px(px(20.))
                    .py(px(14.))
                    .bg(p.raised)
                    .rounded_t(px(workspace_theme::RADIUS_LG - 1.))
                    .border_b_1()
                    .border_color(border)
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .size(px(16.))
                            .text_color(p.warning),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(notice.title.clone()),
                    ),
            )
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .px(px(20.))
                    .py(px(16.))
                    .text_sm()
                    .text_color(p.text)
                    .children(
                        notice
                            .body
                            .split('\n')
                            .map(str::trim)
                            .filter(|line| !line.is_empty())
                            .map(|line| div().child(line.to_owned())),
                    ),
            )
            // Long option labels (the YCbCr matrix ones run past 40 characters)
            // cannot share one footer row with the actions, so they stack.
            .when(!notice.choices.is_empty(), |card| {
                let labels = notice.choices.clone();
                let mut list = div().v_flex().w_full().gap_2().px(px(20.)).pb(px(4.));
                for (index, label) in labels.iter().enumerate() {
                    list = list.child(
                        Button::new(format!("notice-choice-{index}"))
                            .label(label.clone())
                            .w_full()
                            .whitespace_normal()
                            .rounded(px(10.))
                            .px(px(13.))
                            .py(px(8.))
                            .when(index == 0, |button| button.ghost())
                            .when(index > 0, |button| button.primary())
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.invoke(json!({"op":"notice_choice","index":index}), cx);
                                if !view.notices.is_empty() {
                                    view.notices.remove(0);
                                }
                                cx.notify();
                            })),
                    );
                }
                card.child(list)
            })
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .px(px(20.))
                    .pb(px(16.))
                    .child(div().text_xs().text_color(muted).child(if remaining > 0 {
                        format!("还有 {remaining} 条提示")
                    } else {
                        String::new()
                    }))
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_2()
                            .when(notice.kind == "unsaved", |row| {
                                row.child(
                                    Button::new("notice-save")
                                        .label("保存")
                                        .primary()
                                        .rounded(px(999.))
                                        .px(px(13.))
                                        .py(px(3.))
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.invoke(
                                                json!({"op":"command","name":"subtitle/save"}),
                                                cx,
                                            );
                                            view.quit_after_save = true;
                                            view.notice_suppress = false;
                                            if !view.notices.is_empty() {
                                                view.notices.remove(0);
                                            }
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("notice-save-as")
                                        .label("另存为")
                                        .rounded(px(999.))
                                        .px(px(13.))
                                        .py(px(3.))
                                        .ghost()
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.invoke(
                                                json!({"op":"command","name":"subtitle/save/as"}),
                                                cx,
                                            );
                                            view.notice_suppress = false;
                                            if !view.notices.is_empty() {
                                                view.notices.remove(0);
                                            }
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("notice-discard")
                                        .label("不保存")
                                        .rounded(px(999.))
                                        .px(px(13.))
                                        .py(px(3.))
                                        .ghost()
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.invoke(json!({"op":"notice_discard"}), cx);
                                            view.notice_suppress = false;
                                            // Quitting here would race the bridge
                                            // call: the host would still consider
                                            // the file dirty and prompt a hidden
                                            // wx dialog. Quit once it reports clean.
                                            view.quit_after_save = true;
                                            if !view.notices.is_empty() {
                                                view.notices.remove(0);
                                            }
                                            cx.notify();
                                        })),
                                )
                            })
                            .when(notice.kind == "linked", |row| {
                                row.child(div().flex_1())
                                    .child(
                                        Button::new("notice-linked-skip")
                                            .label("不打开")
                                            .rounded(px(999.))
                                            .px(px(13.))
                                            .py(px(3.))
                                            .ghost()
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.invoke(
                                                    json!({"op":"linked_files","apply":false}),
                                                    cx,
                                                );
                                                if !view.notices.is_empty() {
                                                    view.notices.remove(0);
                                                }
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("notice-linked-apply")
                                            .label("确定")
                                            .primary()
                                            .rounded(px(999.))
                                            .px(px(13.))
                                            .py(px(3.))
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.invoke(
                                                    json!({"op":"linked_files","apply":true}),
                                                    cx,
                                                );
                                                if !view.notices.is_empty() {
                                                    view.notices.remove(0);
                                                }
                                                cx.notify();
                                            })),
                                    )
                            })
                            .when(!notice.choices.is_empty(), |row| {
                                row.child(
                                    Button::new("notice-choice-cancel")
                                        .label("取消")
                                        .rounded(px(999.))
                                        .px(px(13.))
                                        .py(px(3.))
                                        .ghost()
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.invoke(
                                                json!({"op":"notice_choice","index":-1}),
                                                cx,
                                            );
                                            if !view.notices.is_empty() {
                                                view.notices.remove(0);
                                            }
                                            cx.notify();
                                        })),
                                )
                            })
                            .when(
                                notice.kind != "unsaved"
                                    && notice.kind != "linked"
                                    && notice.choices.is_empty()
                                    && !notice.suppress.is_empty(),
                                |row| {
                                    let key = notice.suppress.clone();
                                    row.child(
                                        Button::new("host-notice-suppress")
                                            .label("下次不再提示")
                                            .rounded(px(999.))
                                            .px(px(13.))
                                            .py(px(3.))
                                            .ghost()
                                            .selected(suppress_checked)
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.notice_suppress = !view.notice_suppress;
                                                cx.notify();
                                            })),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("dismiss-host-notice")
                                            .label("知道了")
                                            .primary()
                                            .rounded(px(999.))
                                            .small()
                                            .px(px(13.))
                                            .py(px(3.))
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                if view.notice_suppress && !key.is_empty() {
                                                    view.invoke(
                                                        json!({"op":"notice_suppress","key":key}),
                                                        cx,
                                                    );
                                                }
                                                view.notice_suppress = false;
                                                if !view.notices.is_empty() {
                                                    view.notices.remove(0);
                                                }
                                                cx.notify();
                                            })),
                                    )
                                },
                            )
                            .when(
                                notice.kind != "unsaved"
                                    && notice.kind != "linked"
                                    && notice.choices.is_empty()
                                    && notice.suppress.is_empty(),
                                |row| {
                                    row.child(
                                        Button::new("dismiss-host-notice")
                                            .label("知道了")
                                            .primary()
                                            .rounded(px(999.))
                                            .small()
                                            .px(px(13.))
                                            .py(px(3.))
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.notice_suppress = false;
                                                if !view.notices.is_empty() {
                                                    view.notices.remove(0);
                                                }
                                                cx.notify();
                                            })),
                                    )
                                },
                            ),
                    ),
            );
        layered
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .h_flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui_kit::hsla(0., 0., 0., 0.42))
                    .child(card),
            )
            .into_any_element()
    }
}

/// Opens a workspace window wired to the bridge. Every window keeps its own
/// connection, so File > New Window gives an independent view of the document.
/// Tool windows open at the size their page actually needs: a page with two
/// fields has no business being as tall as the preferences list.
fn panel_window_size(panel: Panel) -> gpui_kit::Size<gpui_kit::Pixels> {
    let (width, height) = match panel {
        Panel::Subtitles => (1420., 920.),
        Panel::Styles => (1100., 720.),
        Panel::Styling => (640., 520.),
        Panel::Project => (720., 620.),
        Panel::Visual => (1100., 780.),
        // 16:9 plus the header and transport rows, so the picture opens at its
        // natural shape instead of a cramped letterbox.
        Panel::Video => (980., 640.),
        Panel::Translation => (700., 600.),
        Panel::Timing => (700., 660.),
        Panel::Processor => (900., 600.),
        Panel::Resample => (680., 620.),
        Panel::Search => (560., 640.),
        Panel::Paste => (620., 620.),
        Panel::VideoSettings => (680., 620.),
        Panel::Attachments => (900., 600.),
        Panel::Preferences => (1000., 780.),
        Panel::Hotkeys => (980., 780.),
        Panel::Files => (980., 700.),
        Panel::Spelling => (600., 440.),
        Panel::Automation => (900., 600.),
        Panel::Commands => (900., 720.),
    };
    size(px(width), px(height))
}

fn open_workspace(
    cx: &mut App,
    bridge: Bridge,
    bounds: Option<gpui_kit::Bounds<gpui_kit::Pixels>>,
    detached: Option<Panel>,
) -> Result<gpui_kit::WindowHandle<gpui_kit::component::Root>> {
    let window_bounds = match bounds {
        Some(bounds) => WindowBounds::Windowed(bounds),
        None => match detached {
            Some(panel) => WindowBounds::centered(panel_window_size(panel), cx),
            None => WindowBounds::centered(size(px(1420.), px(920.)), cx),
        },
    };
    let title: gpui_kit::SharedString = match detached {
        Some(panel) => format!("Neo Aegisub · {}", panel.label()).into(),
        None => "Neo Aegisub".into(),
    };
    cx.open_window(
        WindowOptions {
            window_bounds: Some(window_bounds),
            titlebar: Some(gpui_kit::TitlebarOptions {
                title: Some(title),
                ..Default::default()
            }),
            ..Default::default()
        },
        move |window, cx| {
            cx.activate(true);
            window.activate_window();
            let workspace = cx.new(|cx| {
                let mut view = Workspace::new(bridge.clone(), window, cx);
                if let Some(panel) = detached {
                    view.detached = Some(panel);
                    // Go through the normal activation path so the panel asks the
                    // host for its state; assigning active_panel directly left the
                    // detached window stuck on disabled, uninitialised controls.
                    view.activate_panel(panel, window, cx);
                }
                view
            });
            if let Some(panel) = detached {
                PANEL_VIEWS.with(|views| {
                    views.borrow_mut().insert(panel, workspace.downgrade());
                });
            }
            if detached.is_none() {
                MAIN_VIEW.with(|main| main.borrow_mut().replace(workspace.downgrade()));
            }
            // The wx frame used to ask this itself, but by the time the GPUI
            // window is gone the host has no client to talk to and falls back to
            // a native modal. Ask here instead, while the card can still render.
            let close_target = workspace.downgrade();
            window.on_window_should_close(cx, move |_, cx| {
                let Some(view) = close_target.upgrade() else {
                    return true;
                };
                // A closing tool window hands back whatever it borrowed: the
                // popped-out preview belongs to the subtitle window again.
                let popped = view.update(cx, |view, _| view.detached == Some(Panel::Video));
                if popped {
                    if let Some(main) = MAIN_VIEW.with(|main| main.borrow().clone()) {
                        let _ = main.update(cx, |main, cx| {
                            if main.video_popped {
                                main.video_popped = false;
                                // Frames stopped while the picture lived in the
                                // other window; ask for the current one again.
                                main.video_view.serial = -1;
                                main.refresh_video_frame(cx);
                                cx.notify();
                            }
                        });
                    }
                }
                view.update(cx, |view, cx| {
                    if view.detached.is_some() || !view.snapshot.modified {
                        view.cancel_script_dialog(cx);
                        return true;
                    }
                    if view.notices.iter().any(|notice| notice.kind == "unsaved") {
                        return false;
                    }
                    let name = std::path::Path::new(&view.snapshot.file)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "无标题".to_owned());
                    view.notices.push(Notice {
                        title: "未保存的更改".to_owned(),
                        body: format!("{name} 有未保存的修改。"),
                        suppress: String::new(),
                        kind: "unsaved".to_owned(),
                        choices: Vec::new(),
                    });
                    cx.notify();
                    false
                })
            });
            if detached.is_none() {
                let menu_target = workspace.downgrade();
                cx.on_action(move |action: &RunCommand, cx| {
                    let name = action.name.clone();
                    if let Some(view) = menu_target.upgrade() {
                        let _ = view.update(cx, |view, cx| {
                            view.invoke(json!({"op":"command", "name":name}), cx)
                        });
                    }
                });
            }
            workspace.update(cx, |view, cx| {
                view.refresh_catalog(cx);
                view.refresh_document(cx);
            });
            let weak = workspace.downgrade();
            let poll_bridge = bridge.clone();
            cx.spawn(async move |cx| {
                let mut failing_since: Option<Instant> = None;
                let mut playing = false;
                let mut last_document_snapshot = Instant::now();
                loop {
                    if weak.upgrade().is_none() {
                        break;
                    }
                    // A full document snapshot is a large reply; during
                    // playback it competes with the frame requests, so only
                    // refresh it when the document can actually have changed.
                    let read_document =
                        !playing || last_document_snapshot.elapsed() >= Duration::from_millis(800);
                    let result = if read_document {
                        poll_bridge.snapshot().await.map(Ok)
                    } else {
                        poll_bridge.playback().await.map(Err)
                    };
                    match result {
                        Ok(Ok(snapshot)) => {
                            failing_since = None;
                            last_document_snapshot = Instant::now();
                            playing = snapshot.playing || snapshot.audio.playing;
                            if let Some(view) = weak.upgrade() {
                                let _ = view.update(cx, |view, cx| {
                                    if view.status.starts_with("Neo Aegisub 连接中断") {
                                        view.status = "已重新连接 Neo Aegisub".into();
                                    }
                                    view.apply_snapshot(snapshot, cx);
                                });
                            }
                        }
                        Ok(Err(playback)) => {
                            failing_since = None;
                            playing = playback.playing || playback.audio_playing;
                            if let Some(view) = weak.upgrade() {
                                let _ = view.update(cx, |view, cx| {
                                    view.apply_playback(playback, cx);
                                });
                            }
                        }
                        Err(error) => {
                            let since = *failing_since.get_or_insert_with(Instant::now);
                            set_status(&weak, cx, format!("Neo Aegisub 连接中断：{error}"));
                            // A host that stalls while loading audio must not take
                            // the window down with it; only a bridge that stays
                            // unreachable this long ends the session.
                            if since.elapsed() >= Duration::from_secs(15) {
                                cx.update(|app| app.quit());
                                break;
                            }
                        }
                    }
                    cx.background_executor()
                        .timer(Duration::from_millis(if playing { 16 } else { 350 }))
                        .await;
                }
            })
            .detach();
            cx.new(|cx| Root::new(workspace, window, cx))
        },
    )
}

/// Opens one tool in its own window. The window keeps its own bridge
/// connection, so it stays in sync with the document in every other window.
fn open_panel_window(
    cx: &mut App,
    bridge: Bridge,
    panel: Panel,
) -> Result<gpui_kit::WindowHandle<gpui_kit::component::Root>> {
    open_workspace(cx, bridge, None, Some(panel))
}

thread_local! {
    /// One window per tool: clicking the same sidebar row twice should raise the
    /// window that is already open instead of stacking duplicates.
    static PANEL_WINDOWS: std::cell::RefCell<
        std::collections::HashMap<Panel, gpui_kit::WindowHandle<gpui_kit::component::Root>>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
    /// The workspace inside each tool window, so a command can configure the
    /// page (find vs. select mode, video sub-page) after raising the window.
    static PANEL_VIEWS: std::cell::RefCell<
        std::collections::HashMap<Panel, gpui_kit::WeakEntity<Workspace>>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
    /// The subtitle workspace, so a tool window closing can hand back state it
    /// borrowed from it (a popped-out preview docks back into this window).
    static MAIN_VIEW: std::cell::RefCell<Option<gpui_kit::WeakEntity<Workspace>>> =
        const { std::cell::RefCell::new(None) };
}

fn focus_or_open_panel_window(
    cx: &mut App,
    bridge: Bridge,
    panel: Panel,
) -> Result<gpui_kit::WindowHandle<gpui_kit::component::Root>> {
    if let Some(handle) = PANEL_WINDOWS.with(|windows| windows.borrow().get(&panel).copied()) {
        // A closed window leaves a dead handle behind; drop it and reopen.
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return Ok(handle);
        }
        PANEL_WINDOWS.with(|windows| windows.borrow_mut().remove(&panel));
    }
    let handle = open_panel_window(cx, bridge, panel)?;
    PANEL_WINDOWS.with(|windows| windows.borrow_mut().insert(panel, handle));
    Ok(handle)
}

fn run(endpoint: String, token: String) -> Result<()> {
    application()
        .with_assets(workspace_theme::StudioAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            cx.set_global(crate::page::SegmentBounds::default());
            Theme::change(ThemeMode::Dark, None, cx);
            workspace_theme::apply(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bridge = Bridge { endpoint, token };
            if let Err(error) = open_workspace(cx, bridge, None, None) {
                eprintln!("Could not create GPUI window: {error:#}");
                cx.quit();
            }
            cx.activate(true);
        });
    Ok(())
}

fn main() -> Result<()> {
    let mut endpoint = None;
    let mut token = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bridge" => endpoint = args.next(),
            "--token" => token = args.next(),
            _ => {}
        }
    }
    let endpoint =
        endpoint.context("missing --bridge endpoint; launch this prototype from Neo Aegisub")?;
    let token = token.context("missing --token; launch this prototype from Neo Aegisub")?;
    run(endpoint, token)
}

#[cfg(test)]
mod drop_tests {
    use super::dropped_kind;
    use std::path::Path;

    #[test]
    fn classifies_dropped_files_like_the_open_dialogs() {
        assert_eq!(dropped_kind(Path::new("/tmp/a.ASS")), Some("subtitle"));
        assert_eq!(dropped_kind(Path::new("/tmp/a.srt")), Some("subtitle"));
        assert_eq!(dropped_kind(Path::new("/tmp/a.mkv")), Some("video"));
        assert_eq!(dropped_kind(Path::new("/tmp/a.flac")), Some("audio"));
        assert_eq!(dropped_kind(Path::new("/tmp/a.txt")), Some("timecodes"));
        assert_eq!(dropped_kind(Path::new("/tmp/a.png")), None);
        assert_eq!(dropped_kind(Path::new("/tmp/no-extension")), None);
    }
}

#[cfg(test)]
mod picture_tests {
    use super::fitted_picture;
    use gpui_kit::{point, px, size, Bounds, Pixels};

    fn area(width: f32, height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(width), px(height)),
        }
    }

    fn measure(bounds: Bounds<Pixels>) -> (f32, f32, f32, f32) {
        (
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        )
    }

    #[test]
    fn fits_a_wide_area_by_height() {
        let (x, y, w, h) = measure(fitted_picture(area(1000., 600.), 4. / 3.));
        assert_eq!((x, y, w, h), (100., 0., 800., 600.));
    }

    #[test]
    fn fits_a_tall_area_by_width() {
        let (x, y, w, h) = measure(fitted_picture(area(400., 600.), 16. / 9.));
        assert_eq!((x, y, w), (0., 187.5, 400.));
        assert!((h - 225.).abs() < 0.01);
    }

    #[test]
    fn keeps_the_whole_area_when_the_aspect_matches() {
        let (x, y, w, h) = measure(fitted_picture(area(640., 360.), 16. / 9.));
        assert_eq!((x, y), (0., 0.));
        assert!((w - 640.).abs() < 0.01 && (h - 360.).abs() < 0.01);
    }

    #[test]
    fn falls_back_to_sixteen_by_nine_for_an_unusable_aspect() {
        for aspect in [0., -1.7, f32::NAN, f32::INFINITY] {
            let (_, _, w, h) = measure(fitted_picture(area(640., 360.), aspect));
            assert!((w - 640.).abs() < 0.01 && (h - 360.).abs() < 0.01, "{aspect}");
        }
    }

    #[test]
    fn stays_finite_in_an_empty_area() {
        let (x, y, w, h) = measure(fitted_picture(area(0., 0.), 16. / 9.));
        assert!(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite());
        assert!(w >= 1. && h > 0.);
    }

}
