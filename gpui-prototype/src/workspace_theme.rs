use std::borrow::Cow;

use gpui_kit::component::Theme;
use gpui_kit::{App, AssetSource, Hsla, Result, SharedString, px, rgb};

// Bundle the same Lucide family used by GPUI's controls, with the few
// additional media/editing symbols needed by this workspace.
gpui_kit::assets::icon_assets!(
    StudioIcons,
    [
        Captions,
        Pencil,
        AudioLines,
        Languages,
        Clock3,
        Paperclip,
        Save,
        Scissors,
        SpellCheck,
        Move,
        ZoomIn,
        ZoomOut,
        Fullscreen,
        Bold,
        Italic,
        Underline,
        Strikethrough,
        SkipBack,
        SkipForward,
        Repeat,
        Crosshair,
        Video,
        FileInput,
        RefreshCw,
        Volume1,
        Volume2,
        Locate,
        Target,
        Eraser,
        Delete,
        TextCursorInput,
        // Used by the audio toolbar's hotkey button; gpui-kit bundles only a
        // 101-icon default set, so every other icon has to be listed here.
        Keyboard,
    ]
);

pub struct StudioAssets;

impl AssetSource for StudioAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match StudioIcons.load(path)? {
            Some(asset) => Ok(Some(asset)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(StudioIcons.list(path)?);
        Ok(paths)
    }
}

/// Corner radius scale. Every container and control picks from these three so
/// the surfaces read as one family instead of a pile of one-off values.
pub const RADIUS_SM: f32 = 8.0;
pub const RADIUS_MD: f32 = 12.0;
pub const RADIUS_LG: f32 = 16.0;

#[derive(Clone, Copy)]
pub struct Palette {
    pub canvas: Hsla,
    pub surface: Hsla,
    pub raised: Hsla,
    pub inset: Hsla,
    pub border: Hsla,
    pub divider: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub selected: Hsla,
    pub selected_text: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        let color =
            |dark_value, light_value| rgb(if dark { dark_value } else { light_value }).into();
        Self {
            canvas: color(0x14161b, 0xe9edf4),
            surface: color(0x20232b, 0xffffff),
            raised: color(0x282c36, 0xf5f7fb),
            inset: color(0x191c23, 0xeaeef5),
            border: color(0x3a4150, 0xd2d9e5),
            divider: color(0x2b303c, 0xe6eaf1),
            text: color(0xe9ecf4, 0x252c3c),
            muted: color(0xa1aabd, 0x626e83),
            accent: color(0xa7b4ff, 0x425cb3),
            selected: color(0x303c5b, 0xe5eaff),
            selected_text: color(0xe2e8ff, 0x293e83),
            success: color(0x8acbb8, 0x277861),
            warning: color(0xe4bc85, 0x8b5c24),
        }
    }

    pub fn current(cx: &App) -> Self {
        Self::new(Theme::global(cx).is_dark())
    }
}

pub fn apply(cx: &mut App) {
    let dark = Theme::global(cx).is_dark();
    let p = Palette::new(dark);
    let theme = Theme::global_mut(cx);
    theme.font_size = px(15.);
    theme.radius = px(RADIUS_MD);
    theme.radius_lg = px(RADIUS_LG);
    theme.background = p.surface;
    theme.foreground = p.text;
    theme.border = p.border;
    theme.muted = p.raised;
    theme.muted_foreground = p.muted;
    theme.accent = p.selected;
    theme.accent_foreground = p.selected_text;
    theme.primary = rgb(if dark { 0x9cafff } else { 0x4c62b5 }).into();
    theme.primary_foreground = rgb(if dark { 0x18213e } else { 0xffffff }).into();
    theme.primary_hover = rgb(if dark { 0xb5c3ff } else { 0x3e52a0 }).into();
    theme.primary_active = rgb(if dark { 0x879cf3 } else { 0x354789 }).into();
    theme.button = p.raised;
    theme.button_hover = rgb(if dark { 0x343b4c } else { 0xe8ecf5 }).into();
    theme.button_active = p.selected;
    theme.button_foreground = p.text;
    theme.button_primary = theme.primary;
    theme.button_primary_foreground = theme.primary_foreground;
    theme.button_primary_hover = theme.primary_hover;
    theme.button_primary_active = theme.primary_active;
    theme.secondary = p.raised;
    theme.secondary_foreground = p.text;
    theme.secondary_hover = theme.button_hover;
    theme.secondary_active = p.selected;
    theme.button_secondary = theme.secondary;
    theme.button_secondary_foreground = p.text;
    theme.button_secondary_hover = theme.secondary_hover;
    theme.button_secondary_active = p.selected;
    // `input` is the field's outline colour, not its fill: keeping it on the
    // border tone is what makes a field readable on a card of the same hue.
    theme.input = p.border;
    theme.ring = p.accent;
    theme.caret = p.accent;
    theme.selection = p.selected;
    theme.popover = p.surface;
    theme.popover_foreground = p.text;
    theme.colors.list = p.surface;
    theme.list_head = p.raised;
    theme.list_even = p.inset;
    theme.list_hover = p.raised;
    theme.list_active = p.selected;
    theme.list_active_border = p.accent;
    theme.sidebar = p.canvas;
    theme.sidebar_foreground = p.muted;
    theme.sidebar_accent = p.selected;
    theme.sidebar_accent_foreground = p.selected_text;
    theme.sidebar_border = p.divider;
    theme.slider_bar = p.accent;
    theme.slider_thumb = p.surface;
    theme.progress_bar = p.accent;
    theme.scrollbar_thumb = p.border;
    theme.scrollbar_thumb_hover = p.muted;
    theme.title_bar = p.canvas;
    theme.title_bar_border = p.divider;
    theme.tokens = (&theme.colors).into();
    Theme::sync_base(cx);
}
