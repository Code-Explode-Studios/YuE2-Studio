//! How far the original's tuning is from the cover's, and moving the cover there.
//!
//! An old record played back at the wrong speed is sharp or flat by any amount:
//! the fine part (within half a semitone) is the circular mean of where the
//! spectral peaks sit between semitones; the whole semitones come from the
//! pitch-class profiles of the two, each read against its own fine tuning.

use crate::stft::Stft;

use super::features::{bin_classes, centred, RATE};

const SIZE: usize = 8192;
const HOP: usize = 2048;
const LOW_HZ: f32 = 80.0;
const HIGH_HZ: f32 = 4_000.0;
/// Whole semitones searched either way: a cover is made from a transcription of the original, so it is in nearly
/// the same key.
const SEMITONES: i32 = 3;

/// Cents in (-50, 50] the tuning of `mono` (at `RATE`) sits from A = 440 Hz.
pub fn deviation(mono: &[f32]) -> f32 {
    let stft = Stft::new(SIZE, HOP);
    let (low, high) = ((LOW_HZ * SIZE as f32 / RATE as f32) as usize, (HIGH_HZ * SIZE as f32 / RATE as f32) as usize);
    let (mut c, mut s) = (0.0f64, 0.0f64);
    let mut magnitudes = vec![0.0f32; SIZE / 2 + 1];
    stft.each_frame(mono, |_, spectrum| {
        for (slot, value) in magnitudes.iter_mut().zip(spectrum) {
            *slot = value.norm();
        }
        let top = magnitudes[low..high].iter().copied().fold(0.0f32, f32::max);
        if top <= 1e-6 {
            return;
        }
        for k in low.max(1)..high {
            let (a, b, g) = (magnitudes[k - 1], magnitudes[k], magnitudes[k + 1]);
            if b < 0.05 * top || b <= a || b < g {
                continue;
            }
            // the peak between bins, from the parabola through the log magnitudes
            let (la, lb, lg) = (a.max(1e-9).ln(), b.ln(), g.max(1e-9).ln());
            let curve = la - 2.0 * lb + lg;
            let shift = if curve.abs() > 1e-9 { 0.5 * (la - lg) / curve } else { 0.0 };
            let hz = (k as f32 + shift) * RATE as f32 / SIZE as f32;
            let cents = 1200.0 * (hz / 440.0).log2();
            let angle = 2.0 * std::f64::consts::PI * (cents as f64 / 100.0);
            c += b as f64 * angle.cos();
            s += b as f64 * angle.sin();
        }
    });
    if c == 0.0 && s == 0.0 {
        return 0.0;
    }
    (s.atan2(c) / (2.0 * std::f64::consts::PI) * 100.0) as f32
}

/// The pitch-class profile of `mono`, read against a reference A `cents` from 440 Hz.
pub fn profile(mono: &[f32], cents: f32) -> [f32; 12] {
    let stft = Stft::new(SIZE, HOP);
    let classes = bin_classes(SIZE, RATE, cents, LOW_HZ, HIGH_HZ);
    let mut total = [0.0f32; 12];
    stft.each_frame(mono, |_, spectrum| {
        for (value, class) in spectrum.iter().zip(&classes) {
            if let Some((class, upper)) = class {
                let level = value.norm();
                total[*class] += level * (1.0 - upper);
                total[(*class + 1) % 12] += level * upper;
            }
        }
    });
    centred(total)
}

/// Cents the original (`original`, at `RATE`) sits above the cover: shifting the cover up by this matches them.
pub fn offset(original: &[f32], cover: &[f32]) -> f32 {
    let (fine_original, fine_cover) = (deviation(original), deviation(cover));
    let (profile_original, profile_cover) = (profile(original, fine_original), profile(cover, fine_cover));
    let semitones = (-SEMITONES..=SEMITONES)
        .map(|k| {
            let fit: f32 = (0..12).map(|i| profile_original[i] * profile_cover[(i as i32 - k).rem_euclid(12) as usize]).sum();
            // a tie keeps the nearer key
            (k, fit - k.abs() as f32 * 1e-4)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(0, |(k, _)| k);
    100.0 * semitones as f32 + fine_original - fine_cover
}

/// Windowed-sinc kernel: `PHASES` fractional positions of `2 * TAPS` taps each.
const TAPS: usize = 16;
const PHASES: usize = 1024;

/// `x` played `cents` higher: read faster by `2^(cents/1200)`, so it is that much shorter too. The read is
/// band-limited, so nothing folds back from above the Nyquist when it is played faster.
pub fn shift(x: &[f32], cents: f32) -> Vec<f32> {
    let ratio = 2f64.powf(cents as f64 / 1200.0);
    if (ratio - 1.0).abs() < 1e-6 {
        return x.to_vec();
    }
    let cutoff = (1.0 / ratio).min(1.0) * 0.97;
    let kernel: Vec<f32> = (0..PHASES)
        .flat_map(|phase| {
            let fraction = phase as f64 / PHASES as f64;
            (0..2 * TAPS).map(move |tap| {
                let u = tap as f64 - (TAPS as f64 - 1.0) - fraction;
                let sinc = if u.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * cutoff * u).sin() / (std::f64::consts::PI * cutoff * u) };
                let window = 0.5 + 0.5 * (std::f64::consts::PI * u / TAPS as f64).cos();
                (cutoff * sinc * if u.abs() < TAPS as f64 { window } else { 0.0 }) as f32
            })
        })
        .collect();
    let frames = (x.len() as f64 / ratio).floor() as usize;
    (0..frames)
        .map(|m| {
            let position = m as f64 * ratio;
            let whole = position.floor() as isize;
            let phase = (((position - whole as f64) * PHASES as f64) as usize).min(PHASES - 1);
            let taps = &kernel[phase * 2 * TAPS..(phase + 1) * 2 * TAPS];
            let first = whole - (TAPS as isize - 1);
            taps.iter()
                .enumerate()
                .map(|(t, h)| {
                    let at = first + t as isize;
                    if at >= 0 && (at as usize) < x.len() { x[at as usize] * h } else { 0.0 }
                })
                .sum()
        })
        .collect()
}
