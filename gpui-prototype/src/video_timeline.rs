use gpui_kit::base::{Disableable, Slider as BaseSlider, SliderIndicator, SliderTrack};
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    StyledExt, Theme,
    button::{Button, ButtonVariants},
    slider::{SliderEvent, SliderState},
};
use gpui_kit::{AnimationExt, Context, Entity, IntoElement, Subscription, div, prelude::*, px};
use serde_json::json;

use crate::{Snapshot, Workspace, format_time, workspace_theme::Palette};
use gpui_kit::assets::IconName;

/// Stiff and critically damped: the bar reaches its hovered thickness in about
/// a tenth of a second with no overshoot, so the bar reads as one motion with
/// the pointer arriving rather than as a flicker.
const SCRUB_SPRING: gpui_kit::SpringConfig = gpui_kit::SpringConfig::new(650., 40., 1.);

pub struct VideoTimeline {
    pub slider: Entity<SliderState>,
    pub dragging: bool,
    pub seeking: bool,
    pub pending_frame: Option<i64>,
    pub resume_after_drag: bool,
    pub hovering_scrubber: bool,
    _events: Subscription,
}

impl VideoTimeline {
    pub fn new(cx: &mut Context<Workspace>) -> Self {
        let slider = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.000001));
        let events = cx.subscribe(&slider, |view, _, event: &SliderEvent, cx| {
            let value = match event {
                SliderEvent::Change(value) => {
                    // Scrubbing over a running decoder would trade the preview
                    // frame for the playback frame it keeps producing, so the
                    // preview only follows the pointer while playback is paused.
                    if !view.timeline.dragging && view.snapshot.playing {
                        view.timeline.resume_after_drag = true;
                        view.invoke(json!({"op":"command", "name":"video/stop"}), cx);
                    }
                    view.timeline.dragging = true;
                    value.start()
                }
                SliderEvent::Release(value) => {
                    view.timeline.dragging = false;
                    value.start()
                }
            };
            view.timeline.pending_frame =
                Some((value * (view.snapshot.frame_count - 1).max(0) as f32).round() as i64);
            view.flush_video_seek(cx);
        });
        Self {
            slider,
            dragging: false,
            seeking: false,
            pending_frame: None,
            resume_after_drag: false,
            hovering_scrubber: false,
            _events: events,
        }
    }

    /// The scrub bar straddles the border between the frame readout and the
    /// transport row: the band takes no height of its own at the top of the
    /// controls block, and its 16 px hit strip overflows equally above and
    /// below, so the track sits on the seam instead of reading as a row.
    ///
    /// Thickening is a spring, so the bar grows out of the seam under the
    /// pointer and settles back instead of snapping between two sizes. The
    /// slider only handles the drag; the bar itself is painted here so its
    /// thickness can be animated without the indicator's own layout.
    pub fn scrubber(
        &self,
        snapshot: &Snapshot,
        cx: &mut Context<Workspace>,
    ) -> impl IntoElement + use<> {
        let p = Palette::current(cx);
        let slider = self.slider.clone();
        let accent = p.accent;
        let groove = p.border;
        let end = self.slider.read(cx).percentage().end;
        let enabled = snapshot.video;
        let thick = self.hovering_scrubber || self.dragging;
        let target = if thick { 1. } else { 0. };
        div()
            .id("video-scrubber")
            .w_full()
            .h(px(0.))
            .flex()
            .items_center()
            .child(
                div()
                    .id("video-scrubber-hit")
                    .relative()
                    .w_full()
                    .h(px(16.))
                    .flex()
                    .items_center()
                    .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                        if view.timeline.hovering_scrubber != *hovered {
                            view.timeline.hovering_scrubber = *hovered;
                            cx.notify();
                        }
                    }))
                    // `BaseSlider` converts a pointer position to a value
                    // through `SliderState::bounds`, and only `SliderIndicator`
                    // records those bounds at prepaint. Without it the bounds
                    // stay zero, every click divides by zero and the seek snaps
                    // to frame 0 with an invisible (NaN) fill, so the indicator
                    // stays here as the invisible value-mapping surface even
                    // though the track and knob are painted below by hand.
                    .child(
                        SliderIndicator::new(&slider)
                            .absolute()
                            .left(px(5.))
                            .right(px(5.))
                            .top_0()
                            .bottom_0(),
                    )
                    .child(
                        BaseSlider::new(&slider)
                            .disabled(!enabled)
                            .w_full()
                            .h(px(16.))
                            .px(px(5.))
                            .flex()
                            .items_center()
                            .child(
                                SliderTrack::new(&slider)
                                    .disabled(!enabled)
                                    .w_full()
                                    .h(px(16.)),
                            ),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(5.))
                            .right(px(5.))
                            .top_0()
                            .bottom_0()
                            .with_spring(
                                gpui_kit::ElementId::from("video-scrubber-thickness"),
                                gpui_kit::SpringAnimation::new(SCRUB_SPRING).to(target),
                                move |layer, at| {
                                    let track = 3. + 3. * at;
                                    let knob = 8. + 3. * at;
                                    layer
                                        .child(
                                            div()
                                                .absolute()
                                                .left_0()
                                                .right_0()
                                                .top(px(8. - track / 2.))
                                                .h(px(track))
                                                .rounded_full()
                                                .bg(groove)
                                                .child(
                                                    div()
                                                        .absolute()
                                                        .left_0()
                                                        .top_0()
                                                        .h_full()
                                                        .w(gpui_kit::relative(end))
                                                        .rounded_full()
                                                        .bg(accent),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .absolute()
                                                .left(gpui_kit::relative(end))
                                                .top(px(8. - knob / 2.))
                                                .ml(px(-knob / 2.))
                                                .size(px(knob))
                                                .rounded_full()
                                                .bg(accent),
                                        )
                                },
                            ),
                    ),
            )
    }

    pub fn render(&self, snapshot: &Snapshot, cx: &mut Context<Workspace>) -> impl IntoElement {
        let p = Palette::current(cx);
        let mut controls = div().h_flex().flex_wrap().gap_1().items_center()
            .child(
                Button::new("play-video").rounded(gpui_kit::px(999.)).small().px(gpui_kit::px(13.)).py(gpui_kit::px(3.))
                    .icon(if snapshot.playing { IconName::Pause } else { IconName::Play })
                    .label(if snapshot.playing { "暂停" } else { "播放" })
                    .primary()

                    .disabled(!snapshot.video)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.invoke(json!({"op":"command", "name":if view.snapshot.playing { "video/stop" } else { "video/play" }}), cx);
                    })),
            )
            .child(div().w(px(1.)).h(px(16.)).mx_2().bg(p.divider));
        for (id, label, icon, command) in [
            (
                "previous-keyframe",
                "上一关键帧",
                IconName::SkipBack,
                "video/frame/prev/keyframe",
            ),
            (
                "previous-frame",
                "上一帧",
                IconName::ChevronLeft,
                "video/frame/prev",
            ),
            (
                "next-frame",
                "下一帧",
                IconName::ChevronRight,
                "video/frame/next",
            ),
            (
                "next-keyframe",
                "下一关键帧",
                IconName::SkipForward,
                "video/frame/next/keyframe",
            ),
            (
                "line-start-frame",
                "句首",
                IconName::ArrowLeft,
                "video/jump/start",
            ),
            (
                "line-end-frame",
                "句尾",
                IconName::ArrowRight,
                "video/jump/end",
            ),
        ] {
            let enabled = snapshot.video;
            controls = controls.child(
                Button::new(id)
                    .rounded(gpui_kit::px(999.))
                    .small()
                    .px(gpui_kit::px(13.))
                    .py(gpui_kit::px(3.))
                    .when(!id.starts_with("line-"), |button| {
                        button.child(gpui_kit::component::Icon::new(icon).size(gpui_kit::px(15.)))
                    })
                    .when(id.starts_with("line-"), |button| {
                        button.icon(icon).label(label)
                    })
                    .accessibility_label(label)
                    .tooltip(label)
                    .ghost()
                    .disabled(!enabled)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.invoke(json!({"op":"command", "name":command}), cx)
                    })),
            );
        }
        let active = snapshot
            .lines
            .iter()
            .find(|line| line.id == snapshot.active);
        div()
            .id("video-playback-controls")
            .v_flex()
            .flex_shrink_0()
            .min_h(px(74.))
            .rounded_b(px(crate::workspace_theme::RADIUS_LG - 1.))
            .bg(p.surface)
            // The divider belongs to the readout above: the transport block
            // must not paint over the seek bar that rides on this edge.
            .when(snapshot.video, |block| {
                block.child(self.scrubber(snapshot, cx))
            })
            .child(
                div()
                    .v_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(
                        div().h_flex().flex_wrap().items_center().gap_3().child(
                            div()
                                .font_family(Theme::global(cx).mono_font_family.clone())
                                .text_size(px(11.))
                                .text_color(p.muted)
                                .child(format!(
                                    "{} / {} · 帧 {} / {} · {:.2}×",
                                    format_time(snapshot.video_time),
                                    format_time(snapshot.video_duration),
                                    snapshot.frame,
                                    (snapshot.frame_count - 1).max(0),
                                    snapshot.playback_rate
                                )),
                        ),
                    )
                    .child(
                        controls.child(
                            div().ml_2().text_size(px(11.)).text_color(p.muted).child(
                                active
                                    .map(|line| {
                                        format!("当前句帧 {}–{}", line.start_frame, line.end_frame)
                                    })
                                    .unwrap_or_default(),
                            ),
                        ),
                    ),
            )
    }
}
