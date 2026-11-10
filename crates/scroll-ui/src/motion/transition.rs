use std::time::Instant;

use gpui::{App, ElementId, Window};

use super::{Easing, Speed, Timing, is_reduced};

/// A change between two states of an element, played when the value it shows changes and
/// never on the frame it is first drawn (so nothing animates as a window opens or a row
/// scrolls into view).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    /// A row's highlight following the selection.
    Select,
    /// A switch's knob, a checkbox's fill or a disclosure chevron moving to its other state.
    Toggle,
    /// A tree's newly shown rows fading in as their parent opens.
    Reveal,
    /// The page's content fading in as another page opens.
    Page,
    /// The sidebar sliding open or closed.
    Sidebar,
}

impl Transition {
    fn timing(self) -> Timing {
        let (speed, easing) = match self {
            Transition::Select => (Speed::Fast, Easing::Out),
            Transition::Toggle => (Speed::Fast, Easing::InOut),
            Transition::Reveal | Transition::Page => (Speed::Normal, Easing::Out),
            Transition::Sidebar => (Speed::Slow, Easing::InOut),
        };
        Timing { speed, easing }
    }

    /// How far the element named `id` (unique among its siblings) has come since `value`
    /// last changed, eased from 0 to 1. It is 1 the first time `id` is drawn, while `value`
    /// stays the same, and always under reduced motion. The caller draws the in-between state.
    pub fn progress<T: PartialEq + 'static>(
        self,
        id: impl Into<ElementId>,
        value: T,
        window: &mut Window,
        cx: &App,
    ) -> f32 {
        if is_reduced(cx) {
            return 1.0;
        }
        let started = window.with_global_id(id.into(), |global_id, window| {
            window.with_element_state(global_id, |state: Option<Change<T>>, _| {
                let change = Change::after(state, value, Instant::now());
                (change.started, change)
            })
        });
        started.map_or(1.0, |started| self.timing().animate(started, window))
    }

    /// For a two-state element, such as a switch or a selected row: how far it is toward on,
    /// from 0 (off) to 1 (on), moving there after `is_on` changes. See `progress`.
    pub fn fraction_on(
        self,
        id: impl Into<ElementId>,
        is_on: bool,
        window: &mut Window,
        cx: &App,
    ) -> f32 {
        let progress = self.progress(id, is_on, window, cx);
        if is_on { progress } else { 1.0 - progress }
    }
}

/// The value an element showed last frame, and when it last changed.
struct Change<T> {
    value: T,
    started: Option<Instant>,
}

impl<T: PartialEq> Change<T> {
    /// The state once `value` is drawn at `now`: a change starts a transition, first sight
    /// does not.
    fn after(previous: Option<Self>, value: T, now: Instant) -> Self {
        let started = match previous {
            None => None,
            Some(previous) if previous.value != value => Some(now),
            Some(previous) => previous.started,
        };
        Change { value, started }
    }
}
