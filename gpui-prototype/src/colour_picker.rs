//! One colour picker for the whole workspace.
//!
//! Every place that edits a colour (preferences, the style manager, the dummy
//! video, the override-tag dialog) shows gpui-component's `ColorPicker`, so its
//! popover, palette and sliders inherit the workspace theme instead of each
//! panel growing its own swatch and hex field.

use gpui_kit::component::{
    color_picker::{ColorPickerEvent, ColorPickerState},
    input::{InputEvent, InputState},
};
use gpui_kit::{App, AppContext, Context, Entity, Hsla, Subscription, Window};

/// Colours travel to and from the host as `#rrggbb` or `#rrggbbaa`.
///
/// The host's alpha byte is Aegisub's transparency byte, not CSS opacity:
/// `agi::Color::a` counts transparency, so `00` is opaque (`src/compat.cpp`
/// builds `wxColour(r, g, b, 255 - color.a)`). Invert it on the way in and out
/// so the picker previews the colour the grid and audio panel actually draw.
pub fn parse_hex_colour(value: &str) -> Option<Hsla> {
    let hex = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    let digits = u32::from_str_radix(hex, 16).ok()?;
    let rgba = if hex.len() == 6 {
        (digits << 8) | 0xff
    } else {
        (digits & 0xffff_ff00) | (0xff - (digits & 0xff))
    };
    Some(gpui_kit::rgba(rgba).into())
}

fn bytes(colour: Hsla) -> (u32, u32, u32, u32) {
    let rgba: gpui_kit::Rgba = colour.into();
    let byte = |channel: f32| (channel.clamp(0., 1.) * 255.).round() as u32;
    (byte(rgba.r), byte(rgba.g), byte(rgba.b), byte(rgba.a))
}

/// `#rrggbbaa` with the host's transparency byte.
pub fn format_hex_colour(colour: Hsla) -> String {
    let (r, g, b, a) = bytes(colour);
    format!("#{r:02x}{g:02x}{b:02x}{:02x}", 0xff - a)
}

/// `#rrggbb`, for fields that have no alpha.
pub fn format_hex_rgb(colour: Hsla) -> String {
    let (r, g, b, _) = bytes(colour);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Two colours that print the same bytes are the same colour: the Hsla round
/// trip can drift in the last float bit, which must not read as an edit.
pub fn same_colour(a: Hsla, b: Hsla) -> bool {
    bytes(a) == bytes(b)
}

/// A picker bound to a text field that stays the stored value.
///
/// Picking writes the formatted colour into the field; typing a valid colour
/// moves the picker. `ColorPickerState::set_value` and `InputState::set_value`
/// both stay silent, so the two directions cannot ping-pong.
pub struct LinkedPicker {
    pub picker: Entity<ColorPickerState>,
    _events: [Subscription; 2],
}

impl LinkedPicker {
    /// `alpha` selects `#rrggbbaa` over `#rrggbb` when the picker writes.
    /// `on_edit` runs after the picker changed the field, so the owner can mark
    /// it dirty the way a keystroke would have.
    pub fn new<T: 'static>(
        input: &Entity<InputState>,
        alpha: bool,
        window: &mut Window,
        cx: &mut Context<T>,
        on_edit: impl Fn(&mut T, &mut Context<T>) + 'static,
    ) -> Self {
        let start = parse_hex_colour(input.read(cx).value().as_ref())
            .unwrap_or_else(|| gpui_kit::rgba(0xffff_ffff).into());
        let picker = cx.new(|cx| ColorPickerState::new(window, cx).default_value(start));
        let field = input.clone();
        let from_picker = cx.subscribe_in(
            &picker,
            window,
            move |this, _, event: &ColorPickerEvent, window, cx| {
                let ColorPickerEvent::Change(Some(colour)) = event else {
                    return;
                };
                let current = parse_hex_colour(field.read(cx).value().as_ref());
                if current.is_some_and(|current| same_colour(current, *colour)) {
                    return;
                }
                let text = if alpha {
                    format_hex_colour(*colour)
                } else {
                    format_hex_rgb(*colour)
                };
                field.update(cx, |state, cx| state.set_value(text, window, cx));
                on_edit(this, cx);
            },
        );
        let target = picker.clone();
        let from_field = cx.subscribe_in(
            input,
            window,
            move |_, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    pull(&target, input, window, cx);
                }
            },
        );
        Self {
            picker,
            _events: [from_picker, from_field],
        }
    }

    /// Re-reads the field after something other than typing rewrote it.
    pub fn pull(&self, input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
        pull(&self.picker, input, window, cx);
    }
}

fn pull(
    picker: &Entity<ColorPickerState>,
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(colour) = parse_hex_colour(input.read(cx).value().as_ref()) else {
        return;
    };
    if picker
        .read(cx)
        .value()
        .is_some_and(|value| same_colour(value, colour))
    {
        return;
    }
    picker.update(cx, |state, cx| state.set_value(colour, window, cx));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_byte_is_ass_transparency() {
        let opaque = parse_hex_colour("#ff000000").unwrap();
        let clear = parse_hex_colour("#ff0000ff").unwrap();
        assert_eq!(bytes(opaque), (255, 0, 0, 255));
        assert_eq!(bytes(clear), (255, 0, 0, 0));
        assert_eq!(format_hex_colour(opaque), "#ff000000");
        assert_eq!(format_hex_colour(clear), "#ff0000ff");
    }

    #[test]
    fn six_digits_are_opaque_and_round_trip_without_drift() {
        for value in ["#000000", "#ffffff", "#1e90ff", "#7f7f7f", "#0a84ff"] {
            let colour = parse_hex_colour(value).unwrap();
            assert_eq!(format_hex_rgb(colour), value);
            assert_eq!(format_hex_colour(colour), format!("{value}00"));
        }
        assert!(parse_hex_colour("#12345").is_none());
        assert!(parse_hex_colour("zzzzzz").is_none());
    }
}
