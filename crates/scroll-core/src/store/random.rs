//! A small seeded random generator (SplitMix64) for generated test and benchmark data: the same
//! seed always gives the same data.

pub(crate) struct Random(pub(crate) u64);

impl Random {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub(crate) fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    pub(crate) fn pick<'items, T>(&mut self, items: &'items [T]) -> &'items T {
        &items[self.below(items.len())]
    }

    /// True once in `count` times on average.
    #[cfg(test)]
    pub(crate) fn one_in(&mut self, count: usize) -> bool {
        self.below(count) == 0
    }

    #[cfg(feature = "fixture")]
    pub(crate) fn with_probability(&mut self, probability: f64) -> bool {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64) < probability
    }
}
