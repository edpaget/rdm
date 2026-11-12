//! The seed bootstrap behind every confidence interval in the SCBench report.
//!
//! The resample unit is the seed: each statistic's samples are one value per
//! seed, sorted by seed. An arm's interval resamples its seeds with
//! replacement [`RESAMPLES`] times and takes the [`LEVEL`] percentile interval
//! of the resample means ([`super::super::jsnum::percentile`], NumPy
//! `'linear'`). A delta between two groups resamples each group's seeds
//! independently (an unpaired two-sample bootstrap): seed `k` of two arms are
//! independent agent sessions, since SCBench's `--seed` does not fix Claude's
//! sampling.
//!
//! Every statistic draws from its own [`SplitMix64`] stream, seeded from its
//! key ([`seed_for`]), so adding a metric or an arm never moves another
//! interval, and the output is identical across runs and platforms.

use sha2::{Digest, Sha256};

use super::super::jsnum::percentile;

/// Bootstrap resamples per statistic.
pub const RESAMPLES: usize = 10_000;
/// The confidence level of every interval.
pub const LEVEL: f64 = 0.95;
/// The domain prefix of every statistic's RNG seed ([`seed_for`]).
pub const SEED_PREFIX: &str = "rdm-scbench-bootstrap\0";

/// SplitMix64 (Steele, Lea and Flood 2014): a 64-bit generator whose output
/// depends only on its seed.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// A generator starting from `seed`.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The next output.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A uniform index below `n` (`next_u64() % n`; the modulo bias is at
    /// most `n / 2^64`).
    fn index(&mut self, n: usize) -> usize {
        // `n` is a seed count, far below 2^32.
        #[allow(clippy::cast_possible_truncation)]
        let i = (self.next_u64() % n as u64) as usize;
        i
    }
}

/// The RNG seed of statistic `key`: the first 8 bytes, little-endian, of
/// `sha256(SEED_PREFIX + key)`.
pub fn seed_for(key: &str) -> u64 {
    let digest = Sha256::digest(format!("{SEED_PREFIX}{key}").as_bytes());
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(first)
}

/// The arithmetic mean of `xs` (in order); `None` when empty.
pub fn mean(xs: &[f64]) -> Option<f64> {
    #[allow(clippy::cast_precision_loss)]
    (!xs.is_empty()).then(|| xs.iter().sum::<f64>() / xs.len() as f64)
}

fn resample_mean(rng: &mut SplitMix64, xs: &[f64]) -> f64 {
    let mut sum = 0.0;
    for _ in 0..xs.len() {
        sum += xs[rng.index(xs.len())];
    }
    #[allow(clippy::cast_precision_loss)]
    let m = sum / xs.len() as f64;
    m
}

fn interval(draws: &[f64], lo_bound: f64, hi_bound: f64) -> (f64, f64) {
    let tail = (1.0 - LEVEL) / 2.0;
    let lo = percentile(draws, tail).clamp(lo_bound, hi_bound);
    let hi = percentile(draws, 1.0 - tail).clamp(lo_bound, hi_bound);
    (lo, hi)
}

fn bounds(xs: &[f64]) -> (f64, f64) {
    xs.iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &x| {
            (lo.min(x), hi.max(x))
        })
}

/// The bootstrap percentile interval of the mean of `samples` (one per seed,
/// in seed order), drawn from `key`'s stream; `None` with fewer than two
/// samples, which have no spread to resample.
pub fn mean_ci(key: &str, samples: &[f64]) -> Option<(f64, f64)> {
    if samples.len() < 2 {
        return None;
    }
    let mut rng = SplitMix64::new(seed_for(key));
    let draws: Vec<f64> = (0..RESAMPLES)
        .map(|_| resample_mean(&mut rng, samples))
        .collect();
    let (lo, hi) = bounds(samples);
    Some(interval(&draws, lo, hi))
}

/// The unpaired bootstrap percentile interval of `mean(a) - mean(b)`, each
/// group resampled independently from `key`'s stream; `None` when either
/// group has fewer than two samples.
pub fn delta_ci(key: &str, a: &[f64], b: &[f64]) -> Option<(f64, f64)> {
    if a.len() < 2 || b.len() < 2 {
        return None;
    }
    let mut rng = SplitMix64::new(seed_for(key));
    let draws: Vec<f64> = (0..RESAMPLES)
        .map(|_| resample_mean(&mut rng, a) - resample_mean(&mut rng, b))
        .collect();
    let ((a_lo, a_hi), (b_lo, b_hi)) = (bounds(a), bounds(b));
    Some(interval(&draws, a_lo - b_hi, a_hi - b_lo))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_matches_the_reference_outputs() {
        let mut rng = SplitMix64::new(0);
        assert_eq!(rng.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(rng.next_u64(), 0x6e78_9e6a_a1b9_65f4);
        assert_eq!(rng.next_u64(), 0x06c4_5d18_8009_454f);
    }

    #[test]
    fn constant_samples_have_a_point_interval() {
        assert_eq!(mean_ci("k", &[2.5, 2.5, 2.5]), Some((2.5, 2.5)));
    }

    #[test]
    fn one_sample_has_no_interval() {
        assert_eq!(mean_ci("k", &[4.0]), None);
        assert_eq!(mean_ci("k", &[]), None);
        assert_eq!(delta_ci("k", &[1.0], &[1.0, 2.0]), None);
    }

    #[test]
    fn interval_lies_within_the_samples_around_the_mean() {
        let (lo, hi) = mean_ci("k", &[1.0, 2.0, 3.0]).expect("ci");
        assert!((1.0..2.0).contains(&lo), "{lo}");
        assert!(hi > 2.0 && hi <= 3.0, "{hi}");
    }

    #[test]
    fn a_key_always_draws_the_same_interval_and_keys_draw_apart() {
        let xs = [1.0, 4.0, 2.0, 8.0, 5.0];
        assert_eq!(mean_ci("arm/a/m", &xs), mean_ci("arm/a/m", &xs));
        assert_ne!(seed_for("arm/a/m"), seed_for("arm/b/m"));
        let mut a = SplitMix64::new(seed_for("arm/a/m"));
        let mut b = SplitMix64::new(seed_for("arm/b/m"));
        let a: Vec<u64> = (0..4).map(|_| a.next_u64()).collect();
        let b: Vec<u64> = (0..4).map(|_| b.next_u64()).collect();
        assert!(a.iter().zip(&b).all(|(x, y)| x != y), "{a:?} {b:?}");
    }

    #[test]
    fn delta_of_identical_groups_contains_zero_and_a_shift_excludes_it() {
        let same = [1.0, 2.0, 3.0];
        let (lo, hi) = delta_ci("d", &same, &same).expect("ci");
        assert!(lo <= 0.0 && 0.0 <= hi, "{lo} {hi}");
        let (lo, hi) = delta_ci("d", &[10.0, 11.0, 12.0], &[0.0, 1.0, 2.0]).expect("ci");
        assert!(8.0 <= lo && hi <= 12.0 && lo > 0.0, "{lo} {hi}");
    }

    #[test]
    fn mean_is_the_plain_sample_mean() {
        assert_eq!(mean(&[1.0, 2.0, 6.0]), Some(3.0));
        assert_eq!(mean(&[]), None);
    }
}
