use std::time::{Duration, Instant};

use gpui::{App, Pixels, Window, px};

use super::is_reduced;

/// How long a glide takes to cover about two thirds of what is left. Short enough that the
/// page keeps up with a wheel being turned, long enough that a step is seen to travel.
const SETTLE_TIME: Duration = Duration::from_millis(60);
/// The least time a frame counts for, so a frame drawn straight after a push still moves.
const SHORTEST_FRAME: Duration = Duration::from_millis(4);
/// Less than this is covered at once: nobody sees it, and the glide can end.
const REST_DISTANCE: Pixels = px(0.5);

/// A distance still to cover, moved a share each frame so that a jump arrives smoothly: a
/// mouse wheel's steps, scrolled rather than skipped. More can be pushed while it moves.
#[derive(Debug, Default)]
pub struct Glide {
    remaining: Pixels,
    last_frame: Option<Instant>,
}

impl Glide {
    /// Adds `distance` to cover. A push the other way replaces what was left, so turning back
    /// takes effect at once.
    pub fn push(&mut self, distance: Pixels) {
        let is_turning_back = f32::from(self.remaining) * f32::from(distance) < 0.0;
        if is_turning_back {
            self.remaining = Pixels::ZERO;
        }
        if self.remaining == Pixels::ZERO {
            self.last_frame = Some(Instant::now());
        }
        self.remaining += distance;
    }

    pub fn is_moving(&self) -> bool {
        self.remaining != Pixels::ZERO
    }

    /// How far to move in this frame. Asks for another frame while some distance is left; under
    /// reduced motion everything is covered at once.
    pub fn advance(&mut self, window: &mut Window, cx: &App) -> Pixels {
        let now = Instant::now();
        let elapsed = self
            .last_frame
            .map_or(SHORTEST_FRAME, |last| now.saturating_duration_since(last));
        let step = if is_reduced(cx) {
            self.remaining
        } else {
            step_after(self.remaining, elapsed)
        };
        self.remaining -= step;
        self.last_frame = Some(now);
        if self.is_moving() {
            window.request_animation_frame();
        }
        step
    }
}

/// The part of `remaining` covered in a frame `elapsed` long: a fixed share per unit of time,
/// so the glide slows as it arrives and looks the same at any frame rate.
fn step_after(remaining: Pixels, elapsed: Duration) -> Pixels {
    let elapsed = elapsed.max(SHORTEST_FRAME);
    let share = 1.0 - (-elapsed.as_secs_f32() / SETTLE_TIME.as_secs_f32()).exp();
    let step = remaining * share;
    if (remaining - step).abs() < REST_DISTANCE {
        return remaining;
    }
    step
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_millis(8);

    #[test]
    fn a_glide_covers_exactly_its_distance_and_then_rests() {
        for distance in [px(120.0), px(-120.0)] {
            let mut remaining = distance;
            let mut frames = 0;
            while remaining != Pixels::ZERO {
                let step = step_after(remaining, FRAME);
                assert!(
                    f32::from(step) * f32::from(distance) > 0.0,
                    "every step heads the same way"
                );
                assert!(step.abs() <= remaining.abs(), "and never overshoots");
                remaining -= step;
                frames += 1;
            }
            assert!((10..80).contains(&frames), "{frames} frames at 120 Hz");
        }
    }

    #[test]
    fn a_longer_frame_covers_more_of_what_is_left() {
        let short = step_after(px(100.0), FRAME);
        let long = step_after(px(100.0), FRAME * 2);
        assert!(px(0.0) < short && short < long && long < px(100.0));
    }

    #[test]
    fn pushes_add_up_and_a_push_back_starts_over() {
        let mut glide = Glide::default();
        assert!(!glide.is_moving());
        glide.push(px(40.0));
        glide.push(px(20.0));
        assert_eq!(glide.remaining, px(60.0));
        glide.push(px(-10.0));
        assert_eq!(glide.remaining, px(-10.0));
    }
}
