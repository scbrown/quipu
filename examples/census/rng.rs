//! Deterministic randomness for the Census run.
//!
//! `SplitMix64` — self-contained so the run has no dependence on system
//! randomness or crate-version drift; the seed is the only entropy input,
//! per the determinism commitment in `docs/design/paper.md` §8.

pub struct SplitMix64(#[allow(dead_code)] u64);

// The skeleton threads the rng through Ctx but plants nothing random yet;
// the phase implementations (quipu-y41) are the consumers.
#[allow(dead_code)]
impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..bound` (bound must be non-zero).
    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }
}
