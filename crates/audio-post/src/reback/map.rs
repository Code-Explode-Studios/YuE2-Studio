//! The time map: where each moment of the original falls in the cover.

use serde::Serialize;

use super::features::Features;

/// The slowest and fastest the cover may be played to follow the original: a
/// path outside this is a mistake of the alignment, not a band speeding up.
pub const SLOWEST: f64 = 0.5;
pub const FASTEST: f64 = 2.0;

/// Piecewise linear through `points` (seconds of the original, seconds of the cover), increasing in both; beyond
/// the ends it goes on at the speed of the original.
#[derive(Debug, Clone, Serialize)]
pub struct TimeMap {
    pub points: Vec<(f64, f64)>,
}

impl TimeMap {
    /// From an alignment path of `a` (the original) against `b` (the cover): each original frame takes the mean of
    /// the cover frames it was matched with, a local linear regression over `smoothing` seconds takes out the
    /// staircase of the frame grid, and the speed is held between `SLOWEST` and `FASTEST`.
    pub fn from_path(path: &[(usize, usize)], a: &Features, b: &Features, smoothing: f64) -> TimeMap {
        let n = a.len();
        if n == 0 || path.is_empty() {
            return TimeMap { points: vec![(0.0, 0.0)] };
        }
        let mut sum = vec![0.0f64; n];
        let mut count = vec![0u32; n];
        for &(i, j) in path {
            sum[i] += j as f64;
            count[i] += 1;
        }
        let x: Vec<f64> = (0..n).map(|i| a.time_of(i as f64)).collect();
        let y: Vec<f64> = (0..n).map(|i| b.time_of(sum[i] / count[i].max(1) as f64)).collect();
        let frac = (smoothing / (n as f64 * a.hop_seconds)).clamp(0.0, 1.0);
        let mut fitted = if n > 3 && frac > 0.0 { crate::lowess::smooth(&x, &y, frac, a.hop_seconds) } else { y };
        for i in 1..n {
            let dx = x[i] - x[i - 1];
            fitted[i] = fitted[i].clamp(fitted[i - 1] + SLOWEST * dx, fitted[i - 1] + FASTEST * dx);
        }
        TimeMap { points: x.into_iter().zip(fitted).collect() }
    }

    /// The cover's time for `t` seconds of the original.
    pub fn at(&self, t: f64) -> f64 {
        let points = &self.points;
        let (first, last) = (points[0], points[points.len() - 1]);
        if t <= first.0 {
            return first.1 + (t - first.0);
        }
        if t >= last.0 {
            return last.1 + (t - last.0);
        }
        let after = points.partition_point(|point| point.0 <= t);
        let (p, q) = (points[after - 1], points[after]);
        let span = q.0 - p.0;
        if span <= 0.0 { p.1 } else { p.1 + (q.1 - p.1) * (t - p.0) / span }
    }

    /// The map moved by where the two sequences were cut: `original` seconds into the original, `cover` into the cover.
    pub fn offset(mut self, original: f64, cover: f64) -> TimeMap {
        self.points.iter_mut().for_each(|point| {
            point.0 += original;
            point.1 += cover;
        });
        self
    }

    /// The slowest and fastest the cover is played, over windows of `seconds`.
    pub fn speeds(&self, seconds: f64) -> (f64, f64) {
        let (start, end) = (self.points[0].0, self.points[self.points.len() - 1].0);
        let (mut slowest, mut fastest) = (f64::INFINITY, 0.0f64);
        let mut t = start;
        while t + seconds <= end {
            let speed = (self.at(t + seconds) - self.at(t)) / seconds;
            slowest = slowest.min(speed);
            fastest = fastest.max(speed);
            t += seconds / 2.0;
        }
        if fastest == 0.0 { (1.0, 1.0) } else { (slowest, fastest) }
    }

    /// At most `count` of the points, evenly spread, for a report.
    pub fn thinned(&self, count: usize) -> Vec<(f64, f64)> {
        let step = self.points.len().div_ceil(count.max(1)).max(1);
        let mut kept: Vec<(f64, f64)> = self.points.iter().step_by(step).copied().collect();
        if let Some(last) = self.points.last()
            && kept.last() != Some(last)
        {
            kept.push(*last);
        }
        kept
    }
}
