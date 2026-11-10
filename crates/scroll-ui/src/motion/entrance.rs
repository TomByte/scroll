use std::time::{Duration, Instant};

use gpui::{App, ElementId, Pixels, Styled, Window, px};

use super::{Easing, Speed, Timing, is_reduced};

/// How far a sliding entrance travels into place.
const SLIDE_DISTANCE: Pixels = px(4.0);

/// A motion played from the frame an element is first drawn under its id (and again if it is
/// gone for a frame and comes back): an entrance, or a fade out under a new id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// Fades in: a modal's backdrop, a tooltip.
    FadeIn,
    /// Fades in while dropping into place: a popover hanging below its anchor, an error
    /// bubble under its field.
    DropIn,
    /// Fades in while rising into place: a popover above its anchor, a modal, a notice.
    RiseIn,
    /// Fades out: a notice leaving. Its host keeps it drawn for `duration` and then drops it.
    FadeOut,
}

impl Motion {
    pub fn duration(self) -> Duration {
        self.timing().speed.duration()
    }

    fn timing(self) -> Timing {
        let easing = match self {
            Motion::FadeOut => Easing::InOut,
            Motion::FadeIn | Motion::DropIn | Motion::RiseIn => Easing::Out,
        };
        Timing {
            speed: Speed::Normal,
            easing,
        }
    }

    fn style<E: Styled>(self, element: E, progress: f32) -> E {
        let still_to_travel = SLIDE_DISTANCE * (1.0 - progress);
        match self {
            Motion::FadeIn => element.opacity(progress),
            Motion::DropIn => element.opacity(progress).top(-still_to_travel),
            Motion::RiseIn => element.opacity(progress).top(still_to_travel),
            Motion::FadeOut => element.opacity(1.0 - progress),
        }
    }
}

/// When an element with an entrance was first drawn.
struct Entrance {
    started: Instant,
}

/// Attaches a `Motion` to any element.
pub trait MotionExt: Styled + Sized {
    /// Plays `motion` from the frame this element is first drawn under `id`, which must be
    /// unique among its siblings; a new `id` plays it again. The sliding entrances move the
    /// element with `top`, so they are not for absolutely positioned elements.
    fn motion(
        self,
        id: impl Into<ElementId>,
        motion: Motion,
        window: &mut Window,
        cx: &App,
    ) -> Self {
        if is_reduced(cx) {
            return motion.style(self, 1.0);
        }
        let started = window.with_global_id(id.into(), |global_id, window| {
            window.with_element_state(global_id, |state: Option<Entrance>, _| {
                let entrance = state.unwrap_or_else(|| Entrance {
                    started: Instant::now(),
                });
                (entrance.started, entrance)
            })
        });
        let progress = motion.timing().animate(started, window);
        motion.style(self, progress)
    }
}

impl<E: Styled> MotionExt for E {}
