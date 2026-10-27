//! Summaries and confidence intervals. The bootstrap uses a fixed seed, so a
//! report built twice from the same runs gives the same numbers.

use serde::{Deserialize, Serialize};

const BOOTSTRAP_ITERS: usize = 10_000;
const SEED: u64 = 0x5EED_0FB3_7C00;

/// `p` in 0..=100, linearly interpolated between the closest ranks.
/// `None` for no samples.
pub fn percentile(samples: &[f64], p: f64) -> Option<f64> {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    percentile_sorted(&sorted, p)
}

fn percentile_sorted(sorted: &[f64], p: f64) -> Option<f64> {
    let last = sorted.len().checked_sub(1)?;
    let rank = p.clamp(0.0, 100.0) / 100.0 * last as f64;
    let (lo, hi) = (rank.floor() as usize, rank.ceil() as usize);
    Some(sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo as f64))
}

pub fn median(samples: &[f64]) -> Option<f64> {
    percentile(samples, 50.0)
}

pub fn mean(samples: &[f64]) -> Option<f64> {
    (!samples.is_empty()).then(|| samples.iter().sum::<f64>() / samples.len() as f64)
}

/// The shape of a distribution, e.g. one run's frame times.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub n: usize,
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub p999: f64,
    pub max: f64,
}

impl Summary {
    pub fn of(samples: &[f64]) -> Option<Self> {
        let mut sorted = samples.to_vec();
        sorted.sort_by(f64::total_cmp);
        let at = |p| percentile_sorted(&sorted, p);
        Some(Self {
            n: sorted.len(),
            mean: mean(&sorted)?,
            p50: at(50.0)?,
            p95: at(95.0)?,
            p99: at(99.0)?,
            p999: at(99.9)?,
            max: *sorted.last()?,
        })
    }
}

/// A median with its 95% confidence interval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    pub median: f64,
    pub lo: f64,
    pub hi: f64,
}

impl Estimate {
    /// Whether the whole interval is below zero (for a difference: the first
    /// group is lower, beyond noise).
    pub fn below_zero(&self) -> bool {
        self.hi < 0.0
    }
}

/// Median of `samples` with a percentile-bootstrap 95% interval.
pub fn median_ci(samples: &[f64]) -> Option<Estimate> {
    let median = median(samples)?;
    let (lo, hi) = bootstrap(|rng| median_of(&resample(samples, rng)))?;
    Some(Estimate { median, lo, hi })
}

/// `median(a) - median(b)` with a 95% interval. When `hi < 0`, `a` is lower
/// than `b` beyond what resampling noise explains.
pub fn diff_ci(a: &[f64], b: &[f64]) -> Option<Estimate> {
    let median = median(a)? - median(b)?;
    let (lo, hi) = bootstrap(|rng| median_of(&resample(a, rng)) - median_of(&resample(b, rng)))?;
    Some(Estimate { median, lo, hi })
}

fn median_of(samples: &[f64]) -> f64 {
    median(samples).unwrap_or(f64::NAN)
}

fn bootstrap(mut stat: impl FnMut(&mut Rng) -> f64) -> Option<(f64, f64)> {
    let mut rng = Rng(SEED);
    let mut stats: Vec<f64> = Vec::with_capacity(BOOTSTRAP_ITERS);
    for _ in 0..BOOTSTRAP_ITERS {
        stats.push(stat(&mut rng));
    }
    if stats.iter().any(|s| s.is_nan()) {
        return None;
    }
    stats.sort_by(f64::total_cmp);
    Some((percentile_sorted(&stats, 2.5)?, percentile_sorted(&stats, 97.5)?))
}

fn resample(samples: &[f64], rng: &mut Rng) -> Vec<f64> {
    (0..samples.len()).map(|_| samples[rng.below(samples.len())]).collect()
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_interpolate() {
        let s = [4.0, 1.0, 3.0, 2.0];
        assert_eq!(median(&s), Some(2.5));
        assert_eq!(percentile(&s, 0.0), Some(1.0));
        assert_eq!(percentile(&s, 100.0), Some(4.0));
        assert_eq!(median(&[]), None);
        let sum = Summary::of(&(1..=1000).map(f64::from).collect::<Vec<_>>()).unwrap();
        assert_eq!((sum.n, sum.p50, sum.max), (1000, 500.5, 1000.0));
        assert!((sum.p999 - 999.001).abs() < 1e-9);
    }

    #[test]
    fn clear_difference_is_below_zero() {
        let a = [1.0, 1.1, 0.9, 1.05, 0.95];
        let b = [2.0, 2.1, 1.9, 2.05, 1.95];
        let d = diff_ci(&a, &b).unwrap();
        assert!((d.median + 1.0).abs() < 1e-9);
        assert!(d.below_zero(), "{d:?}");
    }

    #[test]
    fn overlapping_groups_are_not_called_different() {
        let a = [1.0, 3.0, 2.0, 2.5, 1.5];
        let b = [1.2, 2.8, 2.2, 2.6, 1.4];
        assert!(!diff_ci(&a, &b).unwrap().below_zero());
    }

    #[test]
    fn interval_contains_the_median_and_is_deterministic() {
        let s = [5.0, 7.0, 6.0, 8.0, 5.5, 6.5];
        let e = median_ci(&s).unwrap();
        assert!(e.lo <= e.median && e.median <= e.hi);
        assert_eq!(median_ci(&s), Some(e));
    }
}
