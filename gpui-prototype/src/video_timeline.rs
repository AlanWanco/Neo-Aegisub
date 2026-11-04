use gpui_kit::base::{Disableable, Slider as BaseSlider, SliderIndicator, SliderTrack};
use gpui_kit::component::Sizable;
use gpui_kit::component::{
    StyledExt, Theme,
    button::{Button, ButtonVariants},
    slider::{SliderEvent, SliderState},
};
use gpui_kit::{Context, Entity, IntoElement, Subscription, div, prelude::*, px};
use serde_json::json;

use crate::{Snapshot, Workspace, format_time, workspace_theme::Palette};
use gpui_kit::assets::IconName;

pub struct VideoTimeline {
    pub slider: Entity<SliderState>,
    pub dragging: bool,
    pub seeking: bool,
    pub pending_frame: Option<i64>,
    pub resume_after_drag: bool,
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
            _events: events,
        }
    }

    /// The scrub bar sits on the bottom edge of the picture, the way a media
    /// player overlays it, instead of taking a row in the control strip.
    pub fn scrubber(&self, snapshot: &Snapshot, cx: &mut Context<Workspace>) -> impl IntoElement {
        let p = Palette::current(cx);
        let progress = self.slider.read(cx).percentage();
        let enabled = snapshot.video;
        div()
            .id("video-scrubber")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(16.))
            .flex()
            .items_end()
            .child(
                BaseSlider::new(&self.slider)
                    .disabled(!enabled)
                    .w_full()
                    .h(px(16.))
                    .px(px(5.))
                    // The thumb overhangs the 3 px track, so the bar floats a
                    // few pixels up instead of being clipped by the surface.
                    .pb(px(4.))
                    .flex()
                    .items_end()
                    .child(
                        SliderTrack::new(&self.slider)
                            .disabled(!enabled)
                            .w_full()
                            .h(px(16.))
                            .flex()
                            .items_end()
                            .child(
                                SliderIndicator::new(&self.slider)
                                    .relative()
                                    .w_full()
                                    .h(px(3.))
                                    .rounded_full()
                                    .bg(gpui_kit::rgba(0xffffff2e))
                                    .child(
                                        div()
                                            .absolute()
                                            .left_0()
                                            .top_0()
                                            .h_full()
                                            .w(gpui_kit::relative(progress.end))
                                            .rounded_full()
                                            .bg(p.accent),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .left(gpui_kit::relative(progress.end))
                                            .top(px(-2.5))
                                            .ml(px(-4.))
                                            .size(px(8.))
                                            .rounded_full()
                                            .bg(p.accent),
                                    ),
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
            .px_3()
            .py_2()
            .gap_2()
            .border_t_1()
            .border_color(p.divider)
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
                            .map(|line| format!("当前句帧 {}–{}", line.start_frame, line.end_frame))
                            .unwrap_or_default(),
                    ),
                ),
            )
    }
}
