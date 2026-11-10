//! How long to wait before reconnecting: doubling from half a second up to thirty, with jitter so many
//! clients cut off at once do not all come back at the same moment.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

const FIRST_DELAY: Duration = Duration::from_millis(500);
const MAX_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub(crate) struct Backoff {
    failures: u32,
}

impl Backoff {
    /// The delay before the next attempt: a random point in the upper half of the current ceiling.
    pub(crate) fn next_delay(&mut self) -> Duration {
        let ceiling = FIRST_DELAY
            .saturating_mul(2u32.saturating_pow(self.failures))
            .min(MAX_DELAY);
        self.failures = self.failures.saturating_add(1);
        let half = ceiling / 2;
        half + half.mul_f64(random_fraction())
    }

    pub(crate) fn reset(&mut self) {
        self.failures = 0;
    }
}

/// Each `RandomState` is keyed differently, so hashing nothing with a fresh one is random enough for jitter.
fn random_fraction() -> f64 {
    let bits = RandomState::new().build_hasher().finish();
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_double_up_to_the_cap_and_stay_in_the_upper_half() {
        let mut backoff = Backoff::default();
        for ceiling in [500, 1000, 2000, 4000, 8000, 16000, 30000, 30000] {
            let delay = backoff.next_delay();
            let ceiling = Duration::from_millis(ceiling);
            assert!(
                delay >= ceiling / 2 && delay <= ceiling,
                "{delay:?} for {ceiling:?}"
            );
        }
        backoff.reset();
        assert!(backoff.next_delay() <= FIRST_DELAY);
    }
}
