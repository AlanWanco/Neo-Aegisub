//! Dialogs a script asks this front end to show while it waits for an answer.
//!
//! The host keeps a request in every snapshot until it gets a reply, so the
//! snapshot decides what is on screen: a request opens the modal, stays while
//! its id is unchanged, and goes away once the host stops reporting it. Form
//! requests draw one typed control per entry. Picker requests use the native
//! file dialog, except for the two flags it cannot honour (`must_exist=false`
//! on open and `no_overwrite_prompt` on save); those use a path field instead.

use std::path::Path;

use gpui_kit::base::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    StyledExt,
    button::{Button, ButtonVariants},
    color_picker::ColorPicker,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenuItem},
    scroll::ScrollableElement,
};
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, Window, div, prelude::*, px,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::Workspace;
use crate::colour_picker::{LinkedPicker, format_hex_colour, format_hex_rgb, parse_hex_colour};
use crate::page;
use crate::workspace_theme::{self, Palette};

/// One request from the host, exactly as the snapshot carries it.
#[derive(Clone, Debug, Deserialize)]
pub struct ScriptDialogRequest {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub controls: Vec<ScriptControl>,
    #[serde(default)]
    pub buttons: Vec<ScriptButton>,
    #[serde(default)]
    pub directory: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub wildcard: String,
    #[serde(default)]
    pub multiple: bool,
    #[serde(default = "must_exist_default")]
    pub must_exist: bool,
    #[serde(default)]
    pub no_overwrite_prompt: bool,
}

fn must_exist_default() -> bool {
    true
}

fn one() -> i64 {
    1
}

#[derive(Clone, Debug, Deserialize)]
pub struct ScriptControl {
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub x: i64,
    #[serde(default)]
    pub y: i64,
    #[serde(default = "one")]
    pub width: i64,
    #[serde(default = "one")]
    pub height: i64,
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub items: Vec<String>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ScriptButton {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub id: String,
}

/// The live widget behind one control. Text-like values stay in gpui-component
/// states; the rest is plain Rust state the handlers update.
pub enum ControlWidget {
    Label,
    Edit(Entity<InputState>),
    Textbox(Entity<TextareaState>),
    Number {
        input: Entity<InputState>,
        integer: bool,
    },
    Dropdown(String),
    Checkbox(bool),
    Colour {
        input: Entity<InputState>,
        picker: LinkedPicker,
        alpha: bool,
    },
    Unsupported,
}

pub struct ControlState {
    pub spec: ScriptControl,
    pub widget: ControlWidget,
    /// Wraps multi-line and colour controls so Enter can tell they own it.
    pub focus: FocusHandle,
    _input_events: Option<gpui_kit::Subscription>,
}

pub struct ScriptDialogState {
    pub request: ScriptDialogRequest,
    pub(crate) controls: Vec<ControlState>,
    pub(crate) path: Option<Entity<TextareaState>>,
    _path_events: Option<gpui_kit::Subscription>,
    built: bool,
    /// The modal card itself; key events inside it belong to the dialog.
    pub(crate) focus: FocusHandle,
    focus_pending: bool,
    pub error: Option<String>,
    /// A reply is in flight.
    pub sending: bool,
    /// The native picker window is up; nothing else may answer meanwhile.
    pub picking: bool,
    started: bool,
    /// Last reply payload, kept so a failed picker reply can be retried.
    pending: Option<Value>,
}

impl ScriptDialogState {
    pub(crate) fn new(mut request: ScriptDialogRequest, cx: &App) -> Self {
        if request.kind == "form" && request.buttons.is_empty() {
            request.buttons = vec![
                ScriptButton {
                    label: "确定".into(),
                    id: "ok".into(),
                },
                ScriptButton {
                    label: "取消".into(),
                    id: "cancel".into(),
                },
            ];
        }
        Self {
            request,
            controls: Vec::new(),
            path: None,
            _path_events: None,
            built: false,
            focus: cx.focus_handle(),
            focus_pending: true,
            error: None,
            sending: false,
            picking: false,
            started: false,
            pending: None,
        }
    }

    fn is_form(&self) -> bool {
        self.request.kind == "form"
    }

    fn is_picker(&self) -> bool {
        matches!(self.request.kind.as_str(), "open" | "save")
    }

    /// Flags the native dialog cannot express: it always requires an existing
    /// file on open and always asks before overwriting on save.
    fn needs_path_form(&self) -> bool {
        match self.request.kind.as_str() {
            "open" => !self.request.must_exist,
            "save" => self.request.no_overwrite_prompt,
            _ => false,
        }
    }

    fn card_visible(&self) -> bool {
        if self.is_picker() {
            self.path.is_some() || self.error.is_some()
        } else {
            true
        }
    }

    fn buttons(&self) -> &[ScriptButton] {
        &self.request.buttons
    }

    fn build(&mut self, window: &mut Window, cx: &mut Context<Workspace>) {
        if self.built {
            return;
        }
        self.built = true;
        if self.is_form() {
            let specs = std::mem::take(&mut self.request.controls);
            self.controls = specs
                .into_iter()
                .map(|spec| make_control(spec, window, cx))
                .collect();
        } else if self.needs_path_form() {
            let dir = self.request.directory.as_str();
            let file = self.request.filename.as_str();
            let initial = match (dir.is_empty(), file.is_empty()) {
                (false, false) => Path::new(dir).join(file).to_string_lossy().into_owned(),
                (false, true) => dir.to_string(),
                (true, false) => file.to_string(),
                (true, true) => String::new(),
            };
            let path = new_textarea(&initial, window, cx);
            path.update(cx, |state, cx| state.set_submit_on_enter(true, cx));
            self._path_events =
                Some(
                    cx.subscribe(&path, |view, _, event: &InputEvent, cx| match event {
                        InputEvent::PressEnter { shift: false, .. } => view.submit_script_paths(cx),
                        InputEvent::Change => view.script_control_edited(cx),
                        _ => {}
                    }),
                );
            self.path = Some(path);
        }
    }

    /// Moves focus into the modal's first editable field, or onto the card.
    fn focus_first(&self, window: &mut Window, cx: &mut App) {
        for control in &self.controls {
            let handle = match &control.widget {
                ControlWidget::Edit(input)
                | ControlWidget::Number { input, .. }
                | ControlWidget::Colour { input, .. } => input.read(cx).focus_handle(cx),
                ControlWidget::Textbox(area) => area.read(cx).focus_handle(cx),
                _ => continue,
            };
            handle.focus(window, cx);
            return;
        }
        match &self.path {
            Some(path) => path.read(cx).focus_handle(cx).focus(window, cx),
            None => self.focus.focus(window, cx),
        }
    }

    fn control_error(&self, index: usize, cx: &App) -> Option<String> {
        read_control(&self.controls[index], cx).err()
    }

    /// Typed values for every named control. Strict reads report the first
    /// invalid control; lenient reads fall back to the host's own value so a
    /// cancel can still return what the user had.
    pub(crate) fn read_values(&self, strict: bool, cx: &App) -> Result<Map<String, Value>, String> {
        let mut values = Map::new();
        for control in &self.controls {
            if control.spec.name.is_empty() {
                continue;
            }
            let value = match read_control(control, cx) {
                Ok(Some(value)) => value,
                Ok(None) => continue,
                Err(error) if strict => return Err(error),
                Err(_) => control.spec.value.clone(),
            };
            values.insert(control.spec.name.clone(), value);
        }
        Ok(values)
    }
}

fn make_control(
    spec: ScriptControl,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> ControlState {
    let text = value_text(&spec.value);
    let widget = match spec.class.as_str() {
        "label" => ControlWidget::Label,
        "edit" | "alpha" => ControlWidget::Edit(new_input(&text, window, cx)),
        "textbox" => ControlWidget::Textbox(new_textarea(&text, window, cx)),
        "intedit" | "floatedit" => ControlWidget::Number {
            input: new_input(&text, window, cx),
            integer: spec.class != "floatedit",
        },
        "dropdown" => ControlWidget::Dropdown(dropdown_initial(&spec)),
        "checkbox" => ControlWidget::Checkbox(spec.value == Value::Bool(true)),
        "color" | "coloralpha" => {
            let alpha = spec.class == "coloralpha";
            let input = new_input(&text, window, cx);
            let picker = LinkedPicker::new(
                &input,
                alpha,
                window,
                cx,
                |view: &mut Workspace, cx: &mut Context<Workspace>| view.script_control_edited(cx),
            );
            picker.pull(&input, window, cx);
            ControlWidget::Colour {
                input,
                picker,
                alpha,
            }
        }
        _ => ControlWidget::Unsupported,
    };
    let input_events = match &widget {
        ControlWidget::Edit(input)
        | ControlWidget::Number { input, .. }
        | ControlWidget::Colour { input, .. } => {
            Some(cx.subscribe(input, |view, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    view.script_control_edited(cx);
                }
            }))
        }
        ControlWidget::Textbox(input) => {
            Some(cx.subscribe(input, |view, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    view.script_control_edited(cx);
                }
            }))
        }
        _ => None,
    };
    ControlState {
        spec,
        widget,
        focus: cx.focus_handle(),
        _input_events: input_events,
    }
}

fn new_input(text: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Entity<InputState> {
    let input = cx.new(|cx| InputState::new(window, cx));
    input.update(cx, |state, cx| {
        state.set_value(text.to_string(), window, cx)
    });
    input
}

fn new_textarea(
    text: &str,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Entity<TextareaState> {
    let area = cx.new(|cx| TextareaState::new(window, cx));
    area.update(cx, |state, cx| {
        state.set_value(text.to_string(), window, cx)
    });
    area
}

fn value_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn dropdown_initial(spec: &ScriptControl) -> String {
    match &spec.value {
        Value::String(text) => text.clone(),
        _ => spec.items.first().cloned().unwrap_or_default(),
    }
}

fn caption(spec: &ScriptControl) -> String {
    if spec.label.is_empty() {
        spec.name.clone()
    } else {
        spec.label.clone()
    }
}

/// Reads one control into its typed value. `Ok(None)` for labels, which carry
/// no value. Numbers are checked against min/max; colours must parse.
fn read_control(control: &ControlState, cx: &App) -> Result<Option<Value>, String> {
    let spec = &control.spec;
    let label = caption(spec);
    match &control.widget {
        ControlWidget::Label => Ok(None),
        ControlWidget::Edit(input) => Ok(Some(Value::String(input.read(cx).value().to_string()))),
        ControlWidget::Textbox(area) => Ok(Some(Value::String(area.read(cx).value().to_string()))),
        ControlWidget::Number { input, integer } => {
            let text = input.read(cx).value().trim().to_string();
            let number = if *integer {
                let whole: i64 = text.parse().map_err(|_| format!("{label}：请输入整数"))?;
                whole as f64
            } else {
                text.parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| format!("{label}：请输入数字"))?
            };
            if let Some(min) = spec.min {
                if number < min {
                    return Err(format!("{label}：不能小于 {min}"));
                }
            }
            if let Some(max) = spec.max {
                if number > max {
                    return Err(format!("{label}：不能大于 {max}"));
                }
            }
            Ok(Some(if *integer {
                json!(number as i64)
            } else {
                json!(number)
            }))
        }
        ControlWidget::Dropdown(selected) => Ok(Some(Value::String(selected.clone()))),
        ControlWidget::Checkbox(checked) => Ok(Some(Value::Bool(*checked))),
        ControlWidget::Colour { input, alpha, .. } => {
            let text = input.read(cx).value().to_string();
            let colour = parse_hex_colour(&text).ok_or_else(|| {
                if *alpha {
                    format!("{label}：颜色应为 #RRGGBBAA")
                } else {
                    format!("{label}：颜色应为 #RRGGBB")
                }
            })?;
            let hex = if *alpha {
                format_hex_colour(colour)
            } else {
                format_hex_rgb(colour)
            };
            Ok(Some(Value::String(hex.to_lowercase())))
        }
        ControlWidget::Unsupported => Err(format!("{label}：不支持的控件类型 {}", spec.class)),
    }
}

/// Splits the path field into absolute paths, one per line.
fn resolve_paths(text: &str, directory: &str, multiple: bool) -> Result<Vec<String>, String> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return Err("请输入文件路径".into());
    }
    if lines.len() > 1 && !multiple {
        return Err("只能选择一个文件".into());
    }
    let base = Path::new(directory);
    lines
        .into_iter()
        .map(|line| {
            let path = Path::new(line);
            if path.is_absolute() {
                return Ok(line.to_string());
            }
            if base.is_absolute() {
                return Ok(base.join(path).to_string_lossy().into_owned());
            }
            Err(format!("请输入绝对路径：{line}"))
        })
        .collect()
}

/// Converts wx wildcard groups to extension filters. Native macOS dialogs union
/// the groups, so an unrestricted group must leave the whole chooser unrestricted.
fn wildcard_filters(wildcard: &str) -> Vec<(String, Vec<String>)> {
    let parts: Vec<&str> = wildcard.split('|').collect();
    let mut filters = Vec::new();
    for pair in parts.chunks(2) {
        let [name, patterns] = pair else {
            continue;
        };
        let mut extensions = Vec::new();
        let mut everything = false;
        for glob in patterns
            .split(';')
            .map(str::trim)
            .filter(|glob| !glob.is_empty())
        {
            match glob.strip_prefix("*.") {
                Some(ext) if !ext.is_empty() && ext != "*" => extensions.push(ext.to_string()),
                _ => everything = true,
            }
        }
        if everything {
            return Vec::new();
        }
        if !extensions.is_empty() {
            filters.push((name.trim().to_string(), extensions));
        }
    }
    filters
}

fn path_string(file: &rfd::FileHandle) -> String {
    file.path().to_string_lossy().into_owned()
}

async fn native_paths(request: &ScriptDialogRequest) -> Vec<String> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title(request.title.clone());
    if !request.directory.is_empty() {
        dialog = dialog.set_directory(&request.directory);
    }
    if !request.filename.is_empty() {
        dialog = dialog.set_file_name(request.filename.clone());
    }
    for (name, extensions) in wildcard_filters(&request.wildcard) {
        dialog = dialog.add_filter(name, extensions.as_slice());
    }
    if request.kind == "save" {
        return dialog
            .save_file()
            .await
            .map(|file| vec![path_string(&file)])
            .unwrap_or_default();
    }
    if request.multiple {
        return dialog
            .pick_files()
            .await
            .map(|files| files.iter().map(path_string).collect::<Vec<String>>())
            .unwrap_or_default();
    }
    dialog
        .pick_file()
        .await
        .map(|file| vec![path_string(&file)])
        .unwrap_or_default()
}

async fn send_reply(bridge: &crate::Bridge, id: i64, response: Value) -> anyhow::Result<()> {
    bridge
        .call(json!({"op": "script_dialog_reply", "id": id, "response": response}))
        .await
        .map(|_| ())
}

fn is_cancel_id(id: &str) -> bool {
    matches!(id, "cancel" | "no" | "close")
}

fn default_button_index(buttons: &[ScriptButton]) -> Option<usize> {
    buttons
        .iter()
        .position(|button| matches!(button.id.as_str(), "ok" | "yes" | "save" | "apply"))
        .or((!buttons.is_empty()).then_some(0))
}

/// Next value for a number control after a step button press, clamped to its range.
fn step_value(
    text: &str,
    integer: bool,
    direction: f64,
    spec: &ScriptControl,
) -> Result<String, String> {
    let current: f64 = text
        .parse()
        .map_err(|_| format!("{}：先输入有效数字再使用步进", caption(spec)))?;
    let mut next = current + direction * spec.step.unwrap_or(1.);
    if let Some(min) = spec.min {
        next = next.max(min);
    }
    if let Some(max) = spec.max {
        next = next.min(max);
    }
    Ok(if integer {
        format!("{}", next.round() as i64)
    } else {
        format!("{}", (next * 1e9).round() / 1e9)
    })
}

fn grid_columns(controls: &[ControlState]) -> u16 {
    controls
        .iter()
        .map(|control| control.spec.x.max(0) + control.spec.width.max(1))
        .max()
        .unwrap_or(1)
        .clamp(1, 64) as u16
}

fn grid_cell(spec: &ScriptControl) -> gpui_kit::Div {
    div()
        .min_w_0()
        .col_start(spec.x.clamp(0, 256) as i16 + 1)
        .col_span(spec.width.clamp(1, 256) as u16)
        .row_start(spec.y.clamp(0, 256) as i16 + 1)
        .row_span(spec.height.clamp(1, 256) as u16)
}

fn tooltipped(index: usize, hint: String, element: impl IntoElement) -> AnyElement {
    if hint.is_empty() {
        return element.into_any_element();
    }
    page::with_tooltip(div().id(("script-tip", index)).child(element), hint).into_any_element()
}

fn labelled(
    index: usize,
    caption: String,
    hint: String,
    body: AnyElement,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let label = tooltipped(index, hint, page::field_label(cx, caption));
    div()
        .v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .child(label)
        .child(body)
        .into_any_element()
}

fn step_button(
    index: usize,
    label: &'static str,
    direction: f64,
    busy: bool,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let id = if direction < 0. {
        "script-step-down"
    } else {
        "script-step-up"
    };
    Button::new((id, index))
        .label(label)
        .ghost()
        .small()
        .rounded(px(999.))
        .px(px(10.))
        .py(px(3.))
        .disabled(busy)
        .on_click(
            cx.listener(move |view, _, window, cx| view.script_step(index, direction, window, cx)),
        )
        .into_any_element()
}

fn render_control(
    index: usize,
    control: &ControlState,
    error: Option<String>,
    busy: bool,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let p = Palette::current(cx);
    let spec = &control.spec;
    let caption = caption(spec);
    let hint = spec.hint.clone();
    let element: AnyElement = match &control.widget {
        ControlWidget::Label => tooltipped(
            index,
            hint,
            div().text_sm().text_color(p.text).child(caption),
        ),
        ControlWidget::Checkbox(checked) => {
            let row = page::check_row(cx, ("script-check", index), caption, *checked, busy)
                .on_click(cx.listener(move |view, _, _, cx| view.script_toggle(index, cx)));
            tooltipped(index, hint, row)
        }
        ControlWidget::Edit(input) => labelled(
            index,
            caption,
            hint,
            Input::new(input)
                .id(format!("script-edit-{index}"))
                .w_full()
                .into_any_element(),
            cx,
        ),
        ControlWidget::Textbox(area) => labelled(
            index,
            caption,
            hint,
            div()
                .id(("script-textbox", index))
                .track_focus(&control.focus)
                .w_full()
                .child(Textarea::new(area).h(px(132.)).w_full())
                .into_any_element(),
            cx,
        ),
        ControlWidget::Number { input, .. } => {
            let row = div()
                .h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(input)
                            .id(format!("script-number-{index}"))
                            .w_full(),
                    ),
                )
                .child(step_button(index, "−", -1., busy, cx))
                .child(step_button(index, "+", 1., busy, cx));
            labelled(index, caption, hint, row.into_any_element(), cx)
        }
        ControlWidget::Dropdown(selected) => {
            let items = spec.items.clone();
            let current = selected.clone();
            let weak = cx.weak_entity();
            let trigger = page::select_trigger(cx, ("script-dropdown", index), current, busy)
                .dropdown_menu(move |mut menu, _, _| {
                    for item in items.iter() {
                        let choice = item.clone();
                        let weak = weak.clone();
                        menu = menu.item(PopupMenuItem::new(choice.clone()).on_click(
                            move |_, _, cx| {
                                let _ = weak.update(cx, |view, cx| {
                                    view.script_set_dropdown(index, choice.clone(), cx)
                                });
                            },
                        ));
                    }
                    menu.scrollable(true)
                });
            labelled(
                index,
                caption,
                hint,
                div().w_full().child(trigger).into_any_element(),
                cx,
            )
        }
        ControlWidget::Colour { input, picker, .. } => {
            let row = div()
                .id(("script-colour", index))
                .track_focus(&control.focus)
                .h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(input)
                            .id(format!("script-colour-{index}"))
                            .w_full(),
                    ),
                )
                .child(
                    ColorPicker::new(&picker.picker)
                        .anchor(gpui_kit::Anchor::TopLeft)
                        .accessibility_label(caption.clone()),
                );
            labelled(index, caption, hint, row.into_any_element(), cx)
        }
        ControlWidget::Unsupported => tooltipped(
            index,
            hint,
            div()
                .text_sm()
                .text_color(p.muted)
                .child(format!("不支持的控件类型：{}", spec.class)),
        ),
    };
    let mut column = div().v_flex().w_full().min_w_0().gap_1().child(element);
    if let Some(error) = error {
        column = column.child(page::error(cx, error));
    }
    column.into_any_element()
}

fn footer_button(
    cx: &mut Context<Workspace>,
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<gpui_kit::SharedString>,
    primary: bool,
    disabled: bool,
    action: impl Fn(&mut Workspace, &mut Context<Workspace>) + 'static,
) -> AnyElement {
    let button = Button::new(id)
        .label(label)
        .rounded(px(999.))
        .small()
        .px(px(13.))
        .py(px(3.));
    let button = if primary {
        button.primary()
    } else {
        button.ghost()
    };
    button
        .disabled(disabled)
        .on_click(cx.listener(move |view, _, _, cx| action(view, cx)))
        .into_any_element()
}

impl Workspace {
    /// Keeps the modal in step with the snapshot. Only the main window owns
    /// script dialogs, so a detached tool window never shows or answers one.
    pub(crate) fn sync_script_dialog(&mut self, cx: &mut Context<Self>) {
        if self.detached.is_some() {
            return;
        }
        match self.snapshot.script_dialog.as_ref() {
            Some(request) => {
                let id = request.id;
                let current = self.script_dialog.as_ref().map(|dialog| dialog.request.id);
                let busy = self
                    .script_dialog
                    .as_ref()
                    .is_some_and(|dialog| dialog.sending || dialog.picking);
                if current != Some(id) && !busy && id > self.script_dialog_replied {
                    self.script_dialog = Some(ScriptDialogState::new(request.clone(), cx));
                }
            }
            None => {
                let idle = self
                    .script_dialog
                    .as_ref()
                    .is_some_and(|dialog| !dialog.sending && !dialog.picking);
                if idle {
                    self.script_dialog = None;
                }
            }
        }
        self.start_script_picker(cx);
    }

    /// Opens the native picker once per request. Path-form requests are
    /// answered from the modal instead.
    fn start_script_picker(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.script_dialog.as_mut() else {
            return;
        };
        if !dialog.is_picker() || dialog.started || dialog.needs_path_form() {
            return;
        }
        dialog.started = true;
        dialog.picking = true;
        let request = dialog.request.clone();
        let id = request.id;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let paths = native_paths(&request).await;
            let response = json!({ "paths": paths });
            // Replies go out from the task itself, so a closed window cannot
            // leave the host waiting.
            let result = send_reply(&bridge, id, response.clone()).await;
            let _ = weak.update(cx, |view, cx| {
                view.script_reply_finished(id, response, result, cx)
            });
        })
        .detach();
    }

    fn spawn_script_reply(&mut self, response: Value, cx: &mut Context<Self>) {
        let Some(dialog) = self.script_dialog.as_mut() else {
            return;
        };
        if dialog.sending {
            return;
        }
        dialog.sending = true;
        dialog.error = None;
        dialog.pending = Some(response.clone());
        let id = dialog.request.id;
        let bridge = self.bridge.clone();
        let weak = cx.weak_entity();
        cx.notify();
        cx.spawn(async move |_, cx| {
            let result = send_reply(&bridge, id, response.clone()).await;
            let _ = weak.update(cx, |view, cx| {
                view.script_reply_finished(id, response, result, cx)
            });
        })
        .detach();
    }

    /// A reply succeeded: the dialog is done. A failure keeps it on screen with
    /// the error, and the same payload can be sent again.
    fn script_reply_finished(
        &mut self,
        id: i64,
        response: Value,
        result: anyhow::Result<()>,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.script_dialog.as_mut() else {
            return;
        };
        if dialog.request.id != id {
            return;
        }
        dialog.sending = false;
        dialog.picking = false;
        match result {
            Ok(()) => {
                self.script_dialog_replied = id;
                self.script_dialog = None;
            }
            Err(error) => {
                dialog.pending = Some(response);
                dialog.error = Some(format!("回复宿主失败：{error}。可重试。"));
            }
        }
        cx.notify();
    }

    pub(crate) fn cancel_script_dialog(&mut self, cx: &mut Context<Self>) {
        let response = {
            let Some(dialog) = self.script_dialog.as_ref() else {
                return;
            };
            if dialog.sending || dialog.picking {
                return;
            }
            if dialog.is_picker() {
                json!({ "paths": [] })
            } else if dialog.is_form() {
                let values = dialog.read_values(false, cx).unwrap_or_default();
                json!({ "button": -1, "values": values })
            } else {
                json!({ "button": -1, "values": {} })
            }
        };
        self.spawn_script_reply(response, cx);
    }

    pub(crate) fn submit_script_button(&mut self, index: usize, cx: &mut Context<Self>) {
        let result = {
            let Some(dialog) = self.script_dialog.as_ref() else {
                return;
            };
            if dialog.sending || dialog.picking || !dialog.is_form() {
                return;
            }
            let buttons = dialog.buttons();
            let Some(button) = buttons.get(index) else {
                return;
            };
            // Cancel-like buttons answer even with invalid input; the rest must validate.
            let strict = !is_cancel_id(&button.id);
            dialog
                .read_values(strict, cx)
                .map(|values| json!({ "button": index, "values": values }))
        };
        match result {
            Ok(response) => self.spawn_script_reply(response, cx),
            Err(error) => self.script_error(error, cx),
        }
    }

    pub(crate) fn submit_script_paths(&mut self, cx: &mut Context<Self>) {
        let result = {
            let Some(dialog) = self.script_dialog.as_ref() else {
                return;
            };
            if dialog.sending || dialog.picking {
                return;
            }
            let Some(path) = dialog.path.as_ref() else {
                return;
            };
            let text = path.read(cx).value().to_string();
            resolve_paths(&text, &dialog.request.directory, dialog.request.multiple)
        };
        match result {
            Ok(paths) => self.spawn_script_reply(json!({ "paths": paths }), cx),
            Err(error) => self.script_error(error, cx),
        }
    }

    pub(crate) fn retry_script_reply(&mut self, cx: &mut Context<Self>) {
        let Some(response) = self
            .script_dialog
            .as_ref()
            .and_then(|dialog| dialog.pending.clone())
        else {
            return;
        };
        self.spawn_script_reply(response, cx);
    }

    fn script_error(&mut self, error: String, cx: &mut Context<Self>) {
        if let Some(dialog) = self.script_dialog.as_mut() {
            dialog.error = Some(error);
        }
        cx.notify();
    }

    pub(crate) fn script_control_edited(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.script_dialog.as_mut() {
            dialog.error = None;
        }
        cx.notify();
    }

    pub(crate) fn script_set_dropdown(
        &mut self,
        index: usize,
        choice: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(dialog) = self.script_dialog.as_mut() {
            if let Some(ControlState {
                widget: ControlWidget::Dropdown(selected),
                ..
            }) = dialog.controls.get_mut(index)
            {
                *selected = choice;
            }
            dialog.error = None;
        }
        cx.notify();
    }

    pub(crate) fn script_toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(dialog) = self.script_dialog.as_mut() {
            if !dialog.sending {
                if let Some(ControlState {
                    widget: ControlWidget::Checkbox(checked),
                    ..
                }) = dialog.controls.get_mut(index)
                {
                    *checked = !*checked;
                }
                dialog.error = None;
            }
        }
        cx.notify();
    }

    pub(crate) fn script_step(
        &mut self,
        index: usize,
        direction: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.script_dialog.as_ref() else {
            return;
        };
        if dialog.sending {
            return;
        }
        let Some(control) = dialog.controls.get(index) else {
            return;
        };
        let ControlWidget::Number { input, integer } = &control.widget else {
            return;
        };
        let input = input.clone();
        let integer = *integer;
        let spec = control.spec.clone();
        let text = input.read(cx).value().trim().to_string();
        match step_value(&text, integer, direction, &spec) {
            Ok(next) => {
                input.update(cx, |state, cx| state.set_value(next, window, cx));
                self.script_control_edited(cx);
            }
            Err(error) => self.script_error(error, cx),
        }
    }

    /// Runs before any document shortcut. Returns true when the key belongs to
    /// the modal and must not reach the rest of the window.
    pub(crate) fn handle_script_dialog_key(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let modifiers = &event.keystroke.modifiers;
        let primary = modifiers.platform || modifiers.control;
        let Some(dialog) = self.script_dialog.as_ref() else {
            return false;
        };
        if dialog.picking || dialog.sending || !dialog.card_visible() {
            return true;
        }
        let in_card = dialog.focus.contains_focused(window, cx);
        let multiline_focus = dialog.controls.iter().any(|control| {
            matches!(
                control.widget,
                ControlWidget::Textbox(_) | ControlWidget::Colour { .. }
            ) && control.focus.contains_focused(window, cx)
        });
        let text_focus = dialog.controls.iter().any(|control| {
            matches!(
                control.widget,
                ControlWidget::Edit(_)
                    | ControlWidget::Number { .. }
                    | ControlWidget::Textbox(_)
                    | ControlWidget::Colour { .. }
            ) && control.focus.contains_focused(window, cx)
        }) || dialog
            .path
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle(cx).contains_focused(window, cx));
        if key == "escape" {
            let escape_button = dialog
                .buttons()
                .iter()
                .rposition(|button| matches!(button.id.as_str(), "no" | "close"));
            if let Some(index) = escape_button {
                self.submit_script_button(index, cx);
            } else {
                self.cancel_script_dialog(cx);
            }
            return true;
        }
        if !in_card {
            if let Some(dialog) = self.script_dialog.as_ref() {
                dialog.focus_first(window, cx);
            }
            return true;
        }
        // Cmd/Ctrl shortcuts stay with the dialog except text editing keys,
        // so the window's own commands cannot fire behind it.
        if primary && (!text_focus || !matches!(key, "a" | "c" | "v" | "x" | "z" | "y")) {
            return true;
        }
        match key {
            "enter" | "return" => self.script_dialog_enter(modifiers.shift, multiline_focus, cx),
            _ => false,
        }
    }

    fn script_dialog_enter(
        &mut self,
        shift: bool,
        multiline_focus: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(dialog) = self.script_dialog.as_ref() else {
            return false;
        };
        if dialog.is_form() {
            // Multi-line fields and colour pickers keep Enter for themselves.
            if multiline_focus {
                return false;
            }
            let buttons = dialog.buttons();
            let Some(index) = default_button_index(&buttons) else {
                return false;
            };
            self.submit_script_button(index, cx);
            true
        } else if dialog.is_picker() {
            if dialog.path.is_some() {
                if shift {
                    return false;
                }
                self.submit_script_paths(cx);
            } else {
                self.retry_script_reply(cx);
            }
            true
        } else {
            self.cancel_script_dialog(cx);
            true
        }
    }

    /// Draws the modal above everything else. Returns None when no card is due,
    /// including while the native picker is up.
    pub(crate) fn render_script_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.detached.is_some() {
            return None;
        }
        let viewport_height = window.viewport_size().height;
        let dialog = self.script_dialog.as_mut()?;
        dialog.build(window, cx);
        if !dialog.card_visible() {
            return None;
        }
        if dialog.focus_pending {
            dialog.focus_pending = false;
            dialog.focus_first(window, cx);
        }
        let dialog = &*dialog;
        let p = Palette::current(cx);
        let busy = dialog.sending || dialog.picking;
        let picker = dialog.is_picker();
        let path_form = dialog.path.clone();
        let errors: Vec<Option<String>> = (0..dialog.controls.len())
            .map(|index| dialog.control_error(index, cx))
            .collect();
        let has_error = errors.iter().any(Option::is_some);
        let title = if dialog.request.title.is_empty() {
            if picker {
                "选择文件".to_string()
            } else {
                "脚本对话框".to_string()
            }
        } else {
            dialog.request.title.clone()
        };
        let subtitle: &'static str = if dialog.is_form() {
            "Enter 确认，Esc 取消；数值与颜色在确认时校验"
        } else if path_form.is_some() {
            "每行一个绝对路径 · Enter 确认，Esc 取消"
        } else if picker {
            "回复宿主时出错，可重试或取消"
        } else {
            "不支持的弹窗类型"
        };
        let body: AnyElement = if dialog.is_form() {
            let mut grid = div()
                .grid()
                .grid_cols(grid_columns(&dialog.controls))
                .gap_3();
            for (index, control) in dialog.controls.iter().enumerate() {
                grid = grid.child(grid_cell(&control.spec).child(render_control(
                    index,
                    control,
                    errors[index].clone(),
                    busy,
                    cx,
                )));
            }
            grid.into_any_element()
        } else if let Some(path) = path_form.as_ref() {
            let multiple = dialog.request.multiple;
            let label = if multiple {
                "文件路径（每行一个）"
            } else {
                "文件路径"
            };
            div()
                .v_flex()
                .w_full()
                .gap_2()
                .child(page::field_label(cx, label))
                .child(
                    Textarea::new(path)
                        .h(px(if multiple { 132. } else { 40. }))
                        .w_full(),
                )
                .into_any_element()
        } else if picker {
            div().into_any_element()
        } else {
            div()
                .text_sm()
                .text_color(p.muted)
                .child(format!("不支持的弹窗类型：{}", dialog.request.kind))
                .into_any_element()
        };
        let body_el = div()
            .id("script-dialog-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .p(px(20.))
            .child(body);

        let mut actions: Vec<AnyElement> = Vec::new();
        let footer_hint: &'static str;
        if dialog.is_form() {
            footer_hint = "Enter 确认 · Esc 取消";
            let buttons = dialog.buttons();
            let default_index = default_button_index(&buttons);
            for (index, button) in buttons.iter().enumerate() {
                let cancel = is_cancel_id(&button.id);
                actions.push(footer_button(
                    cx,
                    ("script-button", index),
                    button.label.clone(),
                    Some(index) == default_index,
                    busy || (has_error && !cancel),
                    move |view, cx| view.submit_script_button(index, cx),
                ));
            }
        } else if picker && path_form.is_some() {
            footer_hint = "Enter 确认 · Esc 取消";
            actions.push(footer_button(
                cx,
                "script-cancel",
                "取消",
                false,
                busy,
                |view, cx| view.cancel_script_dialog(cx),
            ));
            actions.push(footer_button(
                cx,
                "script-confirm",
                "确定",
                true,
                busy,
                |view, cx| view.submit_script_paths(cx),
            ));
        } else if picker {
            footer_hint = "Enter 重试 · Esc 取消";
            actions.push(footer_button(
                cx,
                "script-cancel",
                "取消",
                false,
                busy,
                |view, cx| view.cancel_script_dialog(cx),
            ));
            actions.push(footer_button(
                cx,
                "script-retry",
                "重试",
                true,
                busy,
                |view, cx| view.retry_script_reply(cx),
            ));
        } else {
            footer_hint = "Esc 关闭";
            actions.push(footer_button(
                cx,
                "script-close",
                "关闭",
                true,
                busy,
                |view, cx| view.cancel_script_dialog(cx),
            ));
        }

        let error_el = dialog
            .error
            .clone()
            .map(|text| div().px(px(20.)).pt(px(12.)).child(page::error(cx, text)));
        let header = div()
            .v_flex()
            .flex_shrink_0()
            .gap_1()
            .px(px(20.))
            .py(px(16.))
            .border_b_1()
            .border_color(p.divider)
            .child(div().text_size(px(15.)).font_semibold().child(title))
            .child(div().text_xs().text_color(p.muted).child(subtitle));
        let footer = div()
            .h_flex()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .gap_3()
            .px(px(20.))
            .py(px(14.))
            .border_t_1()
            .border_color(p.divider)
            .child(div().text_xs().text_color(p.muted).child(footer_hint))
            .child(div().h_flex().gap_2().children(actions));
        let card = div()
            .id("script-dialog")
            .track_focus(&dialog.focus)
            .w(px(560.))
            .max_w_full()
            .h((viewport_height - px(96.)).min(px(720.)))
            .max_h(viewport_height - px(96.))
            .v_flex()
            .rounded(px(workspace_theme::RADIUS_LG))
            .border_1()
            .border_color(p.border)
            .bg(p.surface)
            .overflow_hidden()
            .shadow_lg()
            .child(header)
            .child(body_el)
            .children(error_el)
            .child(footer);
        Some(
            div()
                .absolute()
                .inset_0()
                .h_flex()
                .items_center()
                .justify_center()
                .bg(gpui_kit::hsla(0., 0., 0., 0.42))
                // A backdrop click must not reach the document; the dialog
                // only closes through its own buttons or Escape.
                .on_mouse_down(
                    gpui_kit::MouseButton::Left,
                    cx.listener(|_, _, _, cx| cx.stop_propagation()),
                )
                .child(card)
                .into_any_element(),
        )
    }
}
