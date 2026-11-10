//! Motion: every animation in the app, timed and eased in one place.
//!
//! There are two kinds, and nothing else:
//! - a `Motion`, played from the frame an element is first drawn under an id (an entrance, or
//!   a fade out under a new id): `panel.motion(id, Motion::DropIn, window, cx)`;
//! - a `Transition`, played when a value changes but not when it is first drawn:
//!   `Transition::Select.fraction_on(id, is_selected, window, cx)` gives 0 to 1 as a row
//!   becomes selected, and the component draws its own in-between state from it.
//!
//! Both are visual only: the element is interactive from its first frame and nothing waits
//! for an animation. Frames are requested only while one runs. Under
//! `MotionPreference::Reduced` every element is drawn in its final state, with no state kept
//! and no frames requested. Durations stay short (at most `Speed::Slow`) on purpose.

mod entrance;
mod transition;

use std::time::{Duration, Instant};

use gpui::{
    AnyView, App, AppContext, Context, ElementId, Global, Hsla, IntoElement, ParentElement, Render,
    Rgba, Styled, Window, div, prelude::FluentBuilder,
};

use crate::controls::button::Button;
use crate::controls::switch::Switch;
use crate::theme::Theme;

pub use entrance::{Motion, MotionExt};
pub use transition::Transition;

/// Whether the app animates. `Reduced` draws every animated element in its final state at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MotionPreference {
    #[default]
    Full,
    Reduced,
}

impl Global for MotionPreference {}

pub fn set_preference(preference: MotionPreference, cx: &mut App) {
    cx.set_global(preference);
}

pub fn preference(cx: &App) -> MotionPreference {
    cx.try_global::<MotionPreference>()
        .copied()
        .unwrap_or_default()
}

fn is_reduced(cx: &App) -> bool {
    preference(cx) == MotionPreference::Reduced
}

/// The colour `amount` (0 to 1) of the way from `from` to `to`, for drawing a transition's
/// in-between state. Alpha is mixed too, so a translucent colour can become an opaque one.
pub(crate) fn mix(from: Hsla, to: Hsla, amount: f32) -> Hsla {
    if amount <= 0.0 {
        return from;
    }
    if amount >= 1.0 {
        return to;
    }
    let (from, to) = (Rgba::from(from), Rgba::from(to));
    let between = |start: f32, end: f32| start + (end - start) * amount;
    Hsla::from(Rgba {
        r: between(from.r, to.r),
        g: between(from.g, to.g),
        b: between(from.b, to.b),
        a: between(from.a, to.a),
    })
}

/// How long a motion lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Speed {
    /// Selection and small toggles.
    Fast,
    /// Popovers, menus, modals, notices and page content.
    Normal,
    /// Large surfaces, such as the sidebar.
    Slow,
}

impl Speed {
    fn duration(self) -> Duration {
        match self {
            Speed::Fast => Duration::from_millis(120),
            Speed::Normal => Duration::from_millis(170),
            Speed::Slow => Duration::from_millis(230),
        }
    }
}

/// The shape of a motion over its duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Easing {
    /// Starts quickly and settles: things arriving.
    Out,
    /// Starts and ends gently: things moving from one place to another.
    InOut,
}

impl Easing {
    fn apply(self, linear: f32) -> f32 {
        match self {
            Easing::Out => 1.0 - (1.0 - linear).powi(3),
            Easing::InOut => gpui::ease_in_out(linear),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Timing {
    speed: Speed,
    easing: Easing,
}

impl Timing {
    /// The eased progress at `now` of a motion that started at `started`, from 0 to 1.
    fn progress(self, started: Instant, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(started);
        let linear = elapsed.as_secs_f32() / self.speed.duration().as_secs_f32();
        self.easing.apply(linear.min(1.0))
    }

    /// The progress now, asking for another frame until it reaches 1.
    fn animate(self, started: Instant, window: &mut Window) -> f32 {
        let progress = self.progress(started, Instant::now());
        if progress < 1.0 {
            window.request_animation_frame();
        }
        progress
    }
}

/// The entrances, replayed on demand, and a transition driven by a switch.
pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|_| MotionStory {
        plays: 0,
        is_on: false,
    })
    .into()
}

struct MotionStory {
    /// Bumped by "Play again", so every sample is new and enters afresh.
    plays: usize,
    is_on: bool,
}

impl Render for MotionStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let samples = [
            ("FadeIn", Motion::FadeIn),
            ("DropIn", Motion::DropIn),
            ("RiseIn", Motion::RiseIn),
            ("FadeOut", Motion::FadeOut),
        ];
        let plays = self.plays;
        let sample = |label: &'static str, motion: Motion, window: &mut Window| {
            div()
                .px(theme.spacing.md)
                .py(theme.spacing.xs)
                .rounded(theme.radius.md)
                .border(theme.size.border_width)
                .border_color(theme.colors.border)
                .bg(theme.colors.surface_elevated)
                .shadow(theme.shadow.popover.clone())
                .child(label)
                .motion(
                    ElementId::NamedInteger(label.into(), plays as u64),
                    motion,
                    window,
                    cx,
                )
        };
        let samples: Vec<_> = samples
            .into_iter()
            .map(|(label, motion)| sample(label, motion, window))
            .collect();
        let travel = theme.size.popover_min_width / 2.0;
        let along = Transition::Toggle.fraction_on("motion-story-bar", self.is_on, window, cx);
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.lg)
            .text_size(theme.text.ui.size)
            .text_color(theme.colors.text)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme.spacing.md)
                    .child(
                        Button::new("motion-play", "Play again").on_click(cx.listener(
                            |story, _, _, cx| {
                                story.plays += 1;
                                cx.notify();
                            },
                        )),
                    )
                    .children(samples),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme.spacing.md)
                    .child(
                        Switch::new("motion-toggle")
                            .when(self.is_on, Switch::checked)
                            .on_toggle(cx.listener(|story, is_on: &bool, _, cx| {
                                story.is_on = *is_on;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .w(travel + theme.size.checkbox)
                            .h(theme.size.checkbox)
                            .child(
                                div()
                                    .ml(travel * along)
                                    .size(theme.size.checkbox)
                                    .rounded_full()
                                    .bg(theme.colors.accent),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use gpui::{Entity, TestAppContext, VisualTestContext};

    use super::*;

    const EASINGS: [Easing; 2] = [Easing::Out, Easing::InOut];
    const SPEEDS: [Speed; 3] = [Speed::Fast, Speed::Normal, Speed::Slow];

    #[test]
    fn easings_run_from_zero_to_one_without_going_back() {
        for easing in EASINGS {
            assert_eq!(easing.apply(0.0), 0.0, "{easing:?} starts at rest");
            assert_eq!(easing.apply(1.0), 1.0, "{easing:?} ends exactly in place");
            let mut previous = 0.0;
            for step in 1..=100 {
                let value = easing.apply(step as f32 / 100.0);
                assert!((0.0..=1.0).contains(&value), "{easing:?} stays in bounds");
                assert!(value >= previous, "{easing:?} never moves backwards");
                previous = value;
            }
        }
    }

    #[test]
    fn every_speed_is_short_and_they_are_ordered() {
        for speed in SPEEDS {
            assert!(speed.duration() <= Duration::from_millis(250), "{speed:?}");
        }
        assert!(Speed::Fast.duration() < Speed::Normal.duration());
        assert!(Speed::Normal.duration() < Speed::Slow.duration());
    }

    #[test]
    fn progress_runs_over_the_duration_and_stops_at_one() {
        let timing = Timing {
            speed: Speed::Normal,
            easing: Easing::Out,
        };
        let started = Instant::now();
        let duration = Speed::Normal.duration();
        assert_eq!(timing.progress(started, started), 0.0);
        let halfway = timing.progress(started, started + duration / 2);
        assert!(0.0 < halfway && halfway < 1.0);
        assert_eq!(timing.progress(started, started + duration), 1.0);
        assert_eq!(timing.progress(started, started + duration * 10), 1.0);
    }

    #[test]
    fn mixing_keeps_both_ends_exact_and_mixes_alpha_between() {
        let from = gpui::hsla(0.0, 0.0, 0.0, 0.2);
        let to = gpui::hsla(0.6, 0.8, 0.5, 1.0);
        assert_eq!(mix(from, to, 0.0), from);
        assert_eq!(mix(from, to, 1.0), to);
        assert!((mix(from, to, 0.5).a - 0.6).abs() < 1e-4);
    }

    #[gpui::test]
    fn the_preference_is_full_until_set(cx: &mut TestAppContext) {
        cx.update(|cx| {
            assert_eq!(preference(cx), MotionPreference::Full);
            set_preference(MotionPreference::Reduced, cx);
            assert_eq!(preference(cx), MotionPreference::Reduced);
        });
    }

    /// Records what its entrance and its transition give each time it is drawn.
    struct Probe {
        is_on: bool,
        progress: f32,
        opacity: Option<f32>,
    }

    impl Render for Probe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.progress = Transition::Toggle.progress("transition", self.is_on, window, cx);
            let mut element = div().motion("entrance", Motion::FadeIn, window, cx);
            self.opacity = element.style().opacity;
            element
        }
    }

    fn open_probe(cx: &mut TestAppContext) -> (Entity<Probe>, &mut VisualTestContext) {
        cx.add_window_view(|_, _| Probe {
            is_on: false,
            progress: 0.0,
            opacity: None,
        })
    }

    /// Turns the probe on or off and returns its transition's progress once redrawn.
    fn turn(probe: &Entity<Probe>, is_on: bool, cx: &mut VisualTestContext) -> f32 {
        probe.update(cx, |probe, cx| {
            probe.is_on = is_on;
            cx.notify();
        });
        cx.run_until_parked();
        probe.read_with(cx, |probe, _| probe.progress)
    }

    #[gpui::test]
    fn a_transition_plays_on_a_change_but_not_on_first_sight(cx: &mut TestAppContext) {
        let (probe, cx) = open_probe(cx);
        assert_eq!(
            probe.read_with(cx, |probe, _| probe.progress),
            1.0,
            "first sight"
        );
        assert_eq!(turn(&probe, false, cx), 1.0, "no change, nothing plays");
        assert!(turn(&probe, true, cx) < 1.0, "a change starts a transition");
    }

    #[gpui::test]
    fn an_entrance_starts_from_its_first_state(cx: &mut TestAppContext) {
        let (probe, cx) = open_probe(cx);
        let opacity = probe.read_with(cx, |probe, _| probe.opacity);
        assert!(opacity.is_some_and(|opacity| opacity < 1.0));
    }

    #[gpui::test]
    fn reduced_motion_draws_everything_as_it_ends(cx: &mut TestAppContext) {
        cx.update(|cx| set_preference(MotionPreference::Reduced, cx));
        let (probe, cx) = open_probe(cx);
        assert_eq!(probe.read_with(cx, |probe, _| probe.opacity), Some(1.0));
        assert_eq!(turn(&probe, true, cx), 1.0);
    }

    #[gpui::test]
    fn the_story_renders(cx: &mut TestAppContext) {
        crate::foundation::story::testing::open_story(story, cx);
    }
}
