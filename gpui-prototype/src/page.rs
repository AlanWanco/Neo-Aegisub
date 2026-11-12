//! Shared page scaffolding.
//!
//! Every tool page uses the same rhythm so the workspace reads as one
//! application instead of a pile of hand-placed buttons:
//!
//! * a page header (title + one-line description),
//! * a toolbar row holding the page's actions, with a single primary action,
//! * sections rendered as bordered cards with their own header,
//! * consistent 8/12/16/24 spacing.

use std::cell::RefCell;
use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable, ElementExt};
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    Icon, StyledExt, Theme,
    button::{Button, ButtonVariants},
    scroll::{Scrollable, ScrollableElement},
};
use gpui_kit::{
    AnimationExt, Bounds, IntoElement, ParentElement, Pixels, SharedString, SpringConfig, Styled,
    div, prelude::*, px,
};

use crate::workspace_theme::Palette;

/// Outer scroll container for a tool page.
pub fn page(cx: &gpui_kit::App, id: &'static str) -> Scrollable<gpui_kit::Stateful<gpui_kit::Div>> {
    div()
        .id(id)
        .v_flex()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .gap_3()
        .p_4()
        .overflow_y_scrollbar()
        .bg(Palette::current(cx).canvas)
}

/// A row of page actions. Pass one primary action first; the rest should be
/// secondary or ghost so the page has a single obvious next step.
pub fn toolbar(children: impl IntoIterator<Item = gpui_kit::AnyElement>) -> gpui_kit::Div {
    div()
        .h_flex()
        .flex_wrap()
        .items_center()
        .gap(px(10.5))
        .children(children)
}

/// Bordered card with a header row (title, optional trailing content).
pub fn section(
    cx: &gpui_kit::App,
    title: impl Into<gpui_kit::SharedString>,
    trailing: Option<gpui_kit::AnyElement>,
    body: gpui_kit::Div,
) -> gpui_kit::Div {
    section_with_icon(cx, None, title, trailing, body)
}

/// Section card whose header carries a small icon as a visual anchor.
pub fn section_with_icon(
    cx: &gpui_kit::App,
    icon: Option<IconName>,
    title: impl Into<gpui_kit::SharedString>,
    trailing: Option<gpui_kit::AnyElement>,
    body: gpui_kit::Div,
) -> gpui_kit::Div {
    let p = Palette::current(cx);
    let mut head = div()
        .h_flex()
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .gap_3()
        .px_4()
        .py_3()
        // overflow_hidden clips to a rectangle, so the strip needs the card's
        // top corners rounded too or its square corners poke through them.
        .rounded_t(px(crate::workspace_theme::RADIUS_LG - 1.))
        .border_b_1()
        .border_color(p.divider)
        .bg(p.raised)
        .child({
            let mut row = div().h_flex().items_center().gap_2();
            if let Some(icon) = icon {
                row = row.child(Icon::new(icon).size(px(14.)).text_color(p.accent));
            }
            row.child(div().text_sm().font_semibold().child(title.into()))
        });
    if let Some(trailing) = trailing {
        head = head.child(trailing);
    }
    div()
        .v_flex()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .rounded(px(crate::workspace_theme::RADIUS_LG))
        .border_1()
        .border_color(p.border)
        .bg(p.surface)
        .shadow(card_shadow(Theme::global(cx).is_dark()))
        .child(head)
        .child(body)
}

/// Soft elevation for cards: an offset plus a blur, so the card sits above the
/// canvas instead of relying on a border alone.
pub fn card_shadow(dark: bool) -> Vec<gpui_kit::BoxShadow> {
    let alpha = if dark { 0.36 } else { 0.10 };
    vec![
        gpui_kit::base::box_shadow(0., 1., 2., 0., gpui_kit::hsla(0., 0., 0., alpha)),
        gpui_kit::base::box_shadow(0., 8., 20., -6., gpui_kit::hsla(0., 0., 0., alpha * 0.8)),
    ]
}

/// Body of a section: padded column with a comfortable gap.
pub fn section_body() -> gpui_kit::Div {
    div().v_flex().w_full().min_w_0().gap_3().p_4()
}

/// Muted helper text.
pub fn hint(cx: &gpui_kit::App, text: impl Into<gpui_kit::SharedString>) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(Palette::current(cx).muted)
        .child(text.into())
}

/// Label above a control.
pub fn field_label(cx: &gpui_kit::App, text: impl Into<gpui_kit::SharedString>) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(Palette::current(cx).muted)
        .child(text.into())
}

/// One labelled control.
pub fn field(
    cx: &gpui_kit::App,
    label: impl Into<gpui_kit::SharedString>,
    control: impl IntoElement,
) -> gpui_kit::Div {
    // Full width, but never vertically greedy: a `flex_1` here would stretch
    // fields down the whole page inside a column container.
    div()
        .v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .child(field_label(cx, label))
        .child(control)
}

/// Row of fields sharing the width equally.
pub fn field_row(children: impl IntoIterator<Item = gpui_kit::AnyElement>) -> gpui_kit::Div {
    div()
        .h_flex()
        .items_start()
        .w_full()
        .min_w_0()
        .gap_3()
        .children(children)
}

/// Cell inside [`field_row`]; gives the field an equal share of the row.
pub fn field_cell(field: gpui_kit::Div) -> gpui_kit::AnyElement {
    div().flex_1().min_w_0().child(field).into_any_element()
}

/// Error text placed directly under the control it belongs to.
pub fn error(cx: &gpui_kit::App, text: impl Into<gpui_kit::SharedString>) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(Theme::global(cx).danger)
        .child(text.into())
}

/// Small pill used for counts and states in section headers.
pub fn badge(cx: &gpui_kit::App, text: impl Into<gpui_kit::SharedString>) -> gpui_kit::Div {
    let p = Palette::current(cx);
    div()
        .rounded_full()
        .px_2()
        .py_0p5()
        .text_xs()
        .text_color(p.muted)
        .bg(p.inset)
        .child(text.into())
}

/// Where each segment of a segmented control landed, recorded during prepaint
/// and read back by the next render.
///
/// A render pass cannot ask the layout engine where its own children are: their
/// bounds only exist once layout has run. So the sliding pill reads the
/// geometry the previous frame recorded. Keyed by workspace entity as well as
/// control, because a panel detached into its own window has the same control
/// ids as the main window and must keep its own geometry.
#[derive(Default)]
pub struct SegmentBounds(RefCell<HashMap<(gpui_kit::EntityId, SharedString), Vec<Bounds<Pixels>>>>);

impl gpui_kit::Global for SegmentBounds {}

/// The motion tokens' `spring_move` (280 ms response, 0.85 damping) expressed
/// as the physical parameters the spring integrator takes.
const SEGMENT_SPRING: SpringConfig = SpringConfig::new(503.6, 38.15, 1.0);

/// Interpolates the pill between segments as `at` travels from one index to the
/// next, so its width morphs with it instead of snapping at the midpoint.
fn segment_at(geometry: &[Bounds<Pixels>], origin: Pixels, at: f32) -> (Pixels, Pixels) {
    let last = geometry.len().saturating_sub(1);
    let at = at.clamp(0., last as f32);
    let lower = (at.floor() as usize).min(last);
    let upper = (lower + 1).min(last);
    let frac = at - lower as f32;
    let (from, to) = (geometry[lower], geometry[upper]);
    (
        from.origin.x - origin + (to.origin.x - from.origin.x) * frac,
        from.size.width + (to.size.width - from.size.width) * frac,
    )
}

/// Mutually exclusive options rendered as one pill group, so the current
/// choice is obvious and the row reads as a single control. The active segment
/// is one pill that slides between positions instead of a chip that blinks from
/// one segment to the next.
pub fn segmented(
    cx: &mut gpui_kit::Context<crate::Workspace>,
    segments: Vec<(String, &'static str, bool, bool)>,
    on_select: impl Fn(
        &mut crate::Workspace,
        usize,
        &mut gpui_kit::Window,
        &mut gpui_kit::Context<crate::Workspace>,
    ) + 'static,
) -> gpui_kit::Div {
    let handler = std::rc::Rc::new(on_select);
    let p = Palette::current(cx);
    let selected = segments
        .iter()
        .position(|(_, _, selected, _)| *selected)
        .unwrap_or(0);
    let key: SharedString = segments
        .first()
        .map(|(id, ..)| id.clone())
        .unwrap_or_else(|| "segmented".to_owned())
        .into();
    let owner = (cx.entity_id(), key.clone());
    let mut geometry = cx
        .global::<SegmentBounds>()
        .0
        .borrow()
        .get(&owner)
        .cloned()
        .unwrap_or_default();
    // A control that lost a segment must not slide toward the stale slot.
    geometry.truncate(segments.len());
    // Until the segments have been measured there is no geometry to place a
    // pill on; the active segment wears its own chip for that first frame so
    // nothing flashes.
    let pill = geometry
        .first()
        .map(|first| first.origin.x)
        .filter(|_| {
            geometry
                .get(selected)
                .is_some_and(|bounds| bounds.size.width > px(0.))
        })
        .map(|origin| {
            let geometry = geometry.clone();
            div()
                .absolute()
                .top_0()
                .h(px(26.))
                .rounded_full()
                .bg(p.surface)
                .shadow(card_shadow(Theme::global(cx).is_dark()))
                .with_spring(
                    gpui_kit::ElementId::from(format!("{key}-pill")),
                    gpui_kit::SpringAnimation::new(SEGMENT_SPRING).to(selected as f32),
                    move |pill, at| {
                        let (left, width) = segment_at(&geometry, origin, at);
                        pill.left(left).w(width)
                    },
                )
        });
    // The track hugs its items: a full-width inset bar reads as an empty field.
    let mut row = div()
        .h_flex()
        .items_center()
        .gap(px(2.5))
        .p(px(2.5))
        .rounded_full()
        .border_1()
        .border_color(p.border)
        .bg(p.inset);
    let mut pill = pill;
    for (index, (id, label, selected, disabled)) in segments.into_iter().enumerate() {
        let handler = handler.clone();
        let owner = owner.clone();
        let item = div()
            .id(id)
            .h(px(26.))
            .px_3()
            .h_flex()
            .items_center()
            .rounded_full()
            .text_sm()
            .whitespace_nowrap()
            .role(gpui_kit::Role::Button)
            .aria_label(label)
            // The pill carries the chip; the label only carries the accent.
            .when(selected && pill.is_none(), |item| {
                item.bg(p.surface)
                    .shadow(card_shadow(Theme::global(cx).is_dark()))
            })
            .when(selected, |item| item.text_color(p.accent).font_medium())
            .when(!selected && !disabled, |item| {
                item.text_color(p.text).hover(|item| item.bg(p.raised))
            })
            .when(disabled, |item| item.text_color(p.muted).opacity(0.5))
            .when(!disabled, |item| item.cursor_pointer())
            .on_click(cx.listener(move |view, _, window, cx| handler(view, index, window, cx)))
            .child(label);
        // The wrapper is the pill's positioning context: its origin is the
        // first segment's origin, so the pill travels in row-local pixels.
        let mut wrapper = div()
            .relative()
            .flex_shrink_0()
            .on_prepaint(move |bounds, _, cx| {
                let global = cx.global_mut::<SegmentBounds>();
                let mut geometry = global.0.borrow_mut();
                let geometry = geometry.entry(owner.clone()).or_default();
                if geometry.len() <= index {
                    geometry.resize(index + 1, Bounds::default());
                }
                geometry[index] = bounds;
            });
        if index == 0 {
            if let Some(pill) = pill.take() {
                wrapper = wrapper.child(pill);
            }
        }
        row = row.child(wrapper.child(item));
    }
    // Not full width: a wrapping row of controls must be able to sit beside it.
    div().h_flex().flex_shrink_0().child(row)
}

/// Horizontal tab bar with an underline for the active tab. Use it when the
/// options switch the page's content instead of setting a value.
pub fn tabs(
    cx: &mut gpui_kit::Context<crate::Workspace>,
    items: Vec<(String, String, bool)>,
    on_select: impl Fn(
        &mut crate::Workspace,
        usize,
        &mut gpui_kit::Window,
        &mut gpui_kit::Context<crate::Workspace>,
    ) + 'static,
) -> gpui_kit::Div {
    let handler = std::rc::Rc::new(on_select);
    let p = Palette::current(cx);
    let mut bar = div()
        .h_flex()
        .flex_wrap()
        .items_end()
        .w_full()
        .min_w_0()
        .gap_1()
        .border_b_1()
        .border_color(p.divider);
    for (index, (id, label, active)) in items.into_iter().enumerate() {
        let handler = handler.clone();
        let mut tab = div().v_flex().items_center().gap_1();
        tab = tab.child(
            Button::new(id)
                .rounded(gpui_kit::px(999.))
                .small()
                .px(gpui_kit::px(13.))
                .py(gpui_kit::px(3.))
                .label(label)
                .ghost()
                .text_color(if active { p.text } else { p.muted })
                .when(active, |button| button.font_semibold())
                .on_click(cx.listener(move |view, _, window, cx| handler(view, index, window, cx))),
        );
        if active {
            tab = tab.child(div().h(px(2.)).w_full().rounded(px(1.)).bg(p.accent));
        }
        bar = bar.child(tab);
    }
    bar
}

/// Selectable row for lists. Selection is a tint plus a left accent bar, not a
/// filled button: a list of filled pills makes every entry look like an action.
pub fn list_row(
    cx: &gpui_kit::App,
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<gpui_kit::SharedString>,
    selected: bool,
    disabled: bool,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let p = Palette::current(cx);
    div()
        .id(id)
        .h_flex()
        .w_full()
        .min_w_0()
        .h(px(34.))
        .gap_2()
        .items_center()
        .px_3()
        .rounded(px(crate::workspace_theme::RADIUS_SM))
        .border_l_2()
        .border_color(if selected {
            p.accent
        } else {
            gpui_kit::rgba(0x00000000).into()
        })
        .when(selected, |row| row.bg(p.selected))
        .when(!selected && !disabled, |row| {
            row.hover(|row| row.bg(p.raised))
        })
        .text_sm()
        .text_color(if selected { p.selected_text } else { p.text })
        .when(disabled, |row| row.opacity(0.5))
        .cursor_pointer()
        .child(div().truncate().child(label.into()))
}

/// Multi-select chip. Chips are for picking things (styles, fields, flags);
/// buttons are for acting. Keeping the two visually distinct stops a row of
/// identical pills from hiding which one is the page's real action.
pub fn toggle_chip(
    cx: &gpui_kit::App,
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<gpui_kit::SharedString>,
    on: bool,
    disabled: bool,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let p = Palette::current(cx);
    let label: gpui_kit::SharedString = label.into();
    div()
        .id(id)
        .role(gpui_kit::Role::Button)
        .aria_label(label.clone())
        .h(px(28.))
        .px_3()
        .h_flex()
        .items_center()
        .rounded_full()
        .border_1()
        .text_sm()
        .whitespace_nowrap()
        // A selected toggle is a tint, not a second primary button: solid accent
        // stays reserved for the one action that moves the task forward.
        .when(on, |chip| {
            chip.bg(p.selected)
                .border_color(p.accent)
                .text_color(p.selected_text)
        })
        .when(!on, |chip| {
            chip.bg(p.inset).border_color(p.border).text_color(p.text)
        })
        .when(!on && !disabled, |chip| {
            chip.hover(|chip| chip.bg(p.raised))
        })
        .when(disabled, |chip| chip.opacity(0.45))
        .when(!disabled, |chip| chip.cursor_pointer())
        // Only a selected chip gets a leading mark; an unselected one reserves no
        // room for it, so the label sits centred instead of shifted right.
        .children(on.then(|| {
            Icon::new(IconName::Check)
                .size(px(13.))
                .text_color(p.accent)
        }))
        .child(label)
}

/// Tooltip for elements that are not `Button`s.
///
/// `toggle_chip` returns a `Div`, and only `Button`/`Switch`/`Checkbox` carry a
/// one-line `tooltip` helper. This attaches the same tooltip widget through the
/// element's low-level builder.
pub fn with_tooltip(
    el: gpui_kit::Stateful<gpui_kit::Div>,
    text: impl Into<gpui_kit::SharedString> + 'static,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let text = text.into();
    el.tooltip(move |window, cx| {
        gpui_kit::component::tooltip::Tooltip::new(text.clone()).build(window, cx)
    })
}

/// Dropdown trigger that reads like a select: value on the left, caret on the
/// right. A plain `Button` centers its label, which makes a wide select look
/// like a call to action instead of a field.
pub fn select_trigger(
    cx: &gpui_kit::App,
    id: impl Into<gpui_kit::ElementId>,
    value: impl Into<gpui_kit::SharedString>,
    disabled: bool,
) -> Button {
    let p = Palette::current(cx);
    Button::new(id)
        .rounded(gpui_kit::px(999.))
        .small()
        .px(gpui_kit::px(13.))
        .py(gpui_kit::px(3.))
        .w_full()
        .disabled(disabled)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_left()
                .child(value.into()),
        )
        .child(
            Icon::new(IconName::ChevronDown)
                .size(px(14.))
                .text_color(p.muted),
        )
}

/// Checkbox row for a boolean setting inside a card.
pub fn check_row(
    cx: &gpui_kit::App,
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<gpui_kit::SharedString>,
    checked: bool,
    disabled: bool,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let p = Palette::current(cx);
    div()
        .id(id)
        .h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .py_1()
        .rounded(px(4.))
        .text_sm()
        .text_color(p.text)
        .when(!disabled, |row| {
            row.cursor_pointer().hover(|row| row.bg(p.raised))
        })
        .when(disabled, |row| row.opacity(0.5))
        .child(
            div()
                .size(px(15.))
                .flex_shrink_0()
                .h_flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .border_color(if checked { p.accent } else { p.border })
                .when(checked, |box_| box_.bg(p.accent))
                .when(checked, |box_| {
                    box_.child(
                        Icon::new(IconName::Check)
                            .size(px(11.))
                            .text_color(p.canvas),
                    )
                }),
        )
        .child(div().flex_1().min_w_0().truncate().child(label.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(x: f32, width: f32) -> Bounds<Pixels> {
        Bounds {
            origin: gpui_kit::point(px(x), px(0.)),
            size: gpui_kit::size(px(width), px(26.)),
        }
    }

    #[test]
    fn the_pill_travels_and_morphs_between_segments() {
        let geometry = [bounds(100., 60.), bounds(200., 90.)];
        // The track's own origin is the first segment's, so the pill starts at 0.
        assert_eq!(segment_at(&geometry, px(100.), 0.), (px(0.), px(60.)));
        assert_eq!(segment_at(&geometry, px(100.), 1.), (px(100.), px(90.)));
        // Halfway through the spring it is halfway between both segments, not
        // parked on either: this is what makes the pill slide instead of jump.
        assert_eq!(segment_at(&geometry, px(100.), 0.5), (px(50.), px(75.)));
        // A spring can overshoot its target; the pill must not leave the track.
        assert_eq!(segment_at(&geometry, px(100.), 4.), (px(100.), px(90.)));
        assert_eq!(segment_at(&geometry, px(100.), -2.), (px(0.), px(60.)));
    }
}
