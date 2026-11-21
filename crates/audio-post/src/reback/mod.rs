//! Re-back: a new backing track under an old recording's own vocals.
//!
//! A generated cover never keeps the original's timing, and a band of the
//! 1940s played without a click, so its tempo wanders. Instead of asking the
//! model to copy the timing, the cover is bent to it: tuned to the original
//! (old records run sharp or flat), aligned by dynamic time warping on harmony
//! and attacks, stretched along that map without changing pitch, and the
//! original vocal stem laid over it untouched.

pub mod features;
mod dtw;
pub mod map;
pub mod stretch;
pub mod tuning;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub use map::TimeMap;

use crate::Stereo;

/// The most cells the alignment's path matrix may have: one byte each.
const MAX_CELLS: usize = 80_000_000;
/// The finest feature hop at `features::RATE`, about 23 ms.
const FINEST_HOP: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Gain of the original vocal stem, in dB.
    pub vocal_gain_db: f32,
    /// Gain of the stretched cover, in dB.
    pub backing_gain_db: f32,
    /// Measure the tuning offset and move the cover to it.
    pub tune: bool,
    /// A tuning offset to use instead of measuring one, in cents.
    pub tuning_cents: Option<f32>,
    /// How much attacks count beside harmony in the alignment.
    pub onset_weight: f32,
    /// Seconds the time map is smoothed over: tempo changes slower than this are followed.
    pub smoothing_seconds: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self { vocal_gain_db: 0.0, backing_gain_db: 0.0, tune: true, tuning_cents: None, onset_weight: 0.5, smoothing_seconds: 1.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Tuning,
    Aligning,
    Stretching,
    Mixing,
}

/// What a run found, for the person to check.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Cents the cover was moved (the original sits this far above the cover).
    pub tuning_cents: f32,
    /// Whether the offset was measured, not given.
    pub tuning_measured: bool,
    pub original_seconds: f64,
    pub cover_seconds: f64,
    /// The slowest and fastest the cover is played to follow the original, over 4 s windows.
    pub slowest: f64,
    pub fastest: f64,
    /// The time map, thinned: (seconds of the original, seconds of the cover).
    pub map: Vec<(f64, f64)>,
}

/// The mid signal.
pub fn mono(audio: &Stereo) -> Vec<f32> {
    audio.left.iter().zip(&audio.right).map(|(l, r)| (l + r) * 0.5).collect()
}

/// `mono` at the feature rate.
fn at_feature_rate(mono: &[f32], rate: u32) -> Result<Vec<f32>> {
    if rate == features::RATE { Ok(mono.to_vec()) } else { crate::resample::mono(mono, rate, features::RATE) }
}

/// The part of `mono` between its first and last sound: (start, end) in samples. Sound is within 50 dB of the
/// loudest 20 ms.
fn sounding(mono: &[f32], rate: u32) -> (usize, usize) {
    let block = (rate as usize / 50).max(1);
    let levels: Vec<f32> = mono.chunks(block).map(|chunk| (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt()).collect();
    let loudest = levels.iter().copied().fold(0.0f32, f32::max);
    if loudest <= 0.0 {
        return (0, mono.len());
    }
    let floor = loudest * 10f32.powf(-50.0 / 20.0);
    let first = levels.iter().position(|level| *level > floor).unwrap_or(0);
    let last = levels.iter().rposition(|level| *level > floor).unwrap_or(levels.len() - 1);
    (first * block, ((last + 1) * block).min(mono.len()))
}

/// Cents the original sits above the cover. Both are mono at `rate`.
pub fn tuning_offset(original: &[f32], cover: &[f32], rate: u32) -> Result<f32> {
    Ok(tuning::offset(&at_feature_rate(original, rate)?, &at_feature_rate(cover, rate)?))
}

/// Where each moment of `original` falls in `cover`, both mono at `rate`. Silence before and after either is left
/// out of the alignment, so a fade or a count-in of a different length does not pull the ends.
pub fn time_map(original: &[f32], cover: &[f32], rate: u32, settings: &Settings) -> Result<TimeMap> {
    let (original_start, original_end) = sounding(original, rate);
    let (cover_start, cover_end) = sounding(cover, rate);
    let a = at_feature_rate(&original[original_start..original_end], rate)?;
    let b = at_feature_rate(&cover[cover_start..cover_end], rate)?;
    if a.len() < features::RATE as usize || b.len() < features::RATE as usize {
        bail!("the original and the cover must each have at least a second of sound");
    }
    let mut hop = FINEST_HOP;
    while (a.len() / hop + 2) * (b.len() / hop + 2) > MAX_CELLS {
        hop *= 2;
    }
    let (fa, fb) = (features::features(&a, hop), features::features(&b, hop));
    let path = dtw::path(&fa, &fb, settings.onset_weight);
    Ok(TimeMap::from_path(&path, &fa, &fb, settings.smoothing_seconds).offset(original_start as f64 / rate as f64, cover_start as f64 / rate as f64))
}

fn gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// The new backing track under the original vocals: `vocals` is the original's vocal stem, `guide` what the cover
/// is aligned to (the original without its vocals, or the whole mix), `cover` the generated instrumental. The
/// result is as long as `vocals`; the vocals are never stretched. None when `stop` asked to stop.
pub fn reback(
    vocals: &Stereo,
    guide: &Stereo,
    cover: &Stereo,
    settings: &Settings,
    progress: &mut dyn FnMut(Stage),
    stop: &dyn Fn() -> bool,
) -> Result<Option<(Stereo, Report)>> {
    let rate = vocals.rate;
    let guide = guide.resampled(rate)?;
    let mut cover = cover.resampled(rate)?;
    let guide_mono = mono(&guide);

    progress(Stage::Tuning);
    let (tuning_cents, tuning_measured) = match (settings.tuning_cents, settings.tune) {
        (Some(cents), _) => (cents, false),
        (None, true) => (tuning_offset(&guide_mono, &mono(&cover), rate)?, true),
        (None, false) => (0.0, false),
    };
    if tuning_cents.abs() >= 1.0 {
        cover = Stereo::new(tuning::shift(&cover.left, tuning_cents), tuning::shift(&cover.right, tuning_cents), rate);
    }
    if stop() {
        return Ok(None);
    }

    progress(Stage::Aligning);
    let map = time_map(&guide_mono, &mono(&cover), rate, settings)?;
    if stop() {
        return Ok(None);
    }

    progress(Stage::Stretching);
    let Some(backing) = stretch::along(&cover, vocals.frames(), |t| map.at(t), stop) else {
        return Ok(None);
    };

    progress(Stage::Mixing);
    let (vocal_gain, backing_gain) = (gain(settings.vocal_gain_db), gain(settings.backing_gain_db));
    let mix = |voice: &[f32], band: &[f32]| -> Vec<f32> { voice.iter().zip(band).map(|(v, b)| v * vocal_gain + b * backing_gain).collect() };
    let mut out = Stereo::new(mix(&vocals.left, &backing.left), mix(&vocals.right, &backing.right), rate);
    out.keep_below(0.99);
    let (slowest, fastest) = map.speeds(4.0);
    let report = Report {
        tuning_cents,
        tuning_measured,
        original_seconds: vocals.frames() as f64 / rate as f64,
        cover_seconds: cover.frames() as f64 / rate as f64,
        slowest,
        fastest,
        map: map.thinned(200),
    };
    Ok(Some((out, report)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use realfft::RealFftPlanner;

    const RATE: u32 = 44_100;

    /// A sine sweep whose instantaneous frequency follows score time: 110 Hz rising three octaves over 24 s of
    /// score, louder on every half second of score, like a beat. `score_time` takes a time of the signal to the
    /// score's.
    fn sweep(seconds: f64, score_time: impl Fn(f64) -> f64) -> Vec<f32> {
        let mut phase = 0.0f64;
        (0..(seconds * RATE as f64) as usize)
            .map(|i| {
                let tau = score_time(i as f64 / RATE as f64);
                let hz = 110.0 * 2f64.powf(3.0 * tau / 24.0);
                phase += 2.0 * std::f64::consts::PI * hz / RATE as f64;
                let beat = (-(tau.rem_euclid(0.5)) / 0.06).exp();
                (0.4 * (0.35 + 0.65 * beat) * phase.sin()) as f32
            })
            .collect()
    }

    /// A tempo that drifts like a band without a click: the original at `t` plays the score's `drift(t)`.
    fn drift(t: f64) -> f64 {
        t + 1.2 * (2.0 * std::f64::consts::PI * t / 16.0).sin()
    }

    /// The strongest frequency of `x` around `at` seconds.
    fn frequency(x: &[f32], at: f64) -> f64 {
        const N: usize = 8192;
        let start = ((at * RATE as f64) as usize).saturating_sub(N / 2);
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(N);
        let mut input: Vec<f32> = (0..N).map(|i| x.get(start + i).copied().unwrap_or(0.0) * (0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos())).collect();
        let mut output = fft.make_output_vec();
        fft.process(&mut input, &mut output).unwrap();
        let magnitudes: Vec<f32> = output.iter().map(|c| c.norm().max(1e-9)).collect();
        let k = (1..magnitudes.len() - 1).max_by(|a, b| magnitudes[*a].total_cmp(&magnitudes[*b])).unwrap();
        let (a, b, g) = (magnitudes[k - 1].ln(), magnitudes[k].ln(), magnitudes[k + 1].ln());
        let shift = 0.5 * (a - g) / (a - 2.0 * b + g);
        (k as f64 + shift as f64) * RATE as f64 / N as f64
    }

    fn chord(seconds: f64, cents: f64) -> Vec<f32> {
        // A, C sharp, E with two partials each
        let roots = [220.0, 277.18, 329.63].map(|hz: f64| hz * 2f64.powf(cents / 1200.0));
        (0..(seconds * RATE as f64) as usize)
            .map(|i| {
                let t = i as f64 / RATE as f64;
                roots.iter().map(|hz| (2.0 * std::f64::consts::PI * hz * t).sin() * 0.2 + (4.0 * std::f64::consts::PI * hz * t).sin() * 0.08).sum::<f64>() as f32
            })
            .collect()
    }

    #[test]
    fn the_alignment_recovers_a_drifting_tempo() {
        let original = sweep(24.0, drift);
        let cover = sweep(24.0, |t| t);
        let map = time_map(&original, &cover, RATE, &Settings::default()).unwrap();
        let mut errors = Vec::new();
        let mut t = 1.5;
        while t < 22.5 {
            errors.push((map.at(t) - drift(t)).abs());
            t += 0.25;
        }
        let mean = errors.iter().sum::<f64>() / errors.len() as f64;
        let worst = errors.iter().copied().fold(0.0, f64::max);
        assert!(mean < 0.03, "mean error {mean:.3} s");
        assert!(worst < 0.08, "worst error {worst:.3} s");
        let (slowest, fastest) = map.speeds(4.0);
        assert!(slowest < 0.8 && fastest > 1.2, "the drift is followed, not averaged away: {slowest:.2} to {fastest:.2}");
    }

    #[test]
    fn a_constant_stretch_keeps_the_pitch_and_takes_the_length_asked() {
        let tone: Vec<f32> = (0..RATE as usize * 4).map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin() * 0.5).collect();
        let input = Stereo::new(tone.clone(), tone, RATE);
        let frames = RATE as usize * 5;
        let out = stretch::along(&input, frames, |t| t * 0.8, &|| false).unwrap();
        assert_eq!(out.frames(), frames);
        for at in [0.5, 2.0, 3.5, 4.5] {
            let hz = frequency(&out.left, at);
            assert!((hz - 440.0).abs() < 1.0, "{hz:.2} Hz at {at} s");
        }
        // no gaps or cancellation where the frames join
        let middle = &out.left[RATE as usize..RATE as usize * 4];
        let level = (middle.iter().map(|s| s * s).sum::<f32>() / middle.len() as f32).sqrt();
        assert!((level - 0.5 / 2f32.sqrt()).abs() < 0.02, "level {level:.3}");
    }

    #[test]
    fn stretching_the_cover_along_the_drift_gives_the_original_back() {
        let original = sweep(24.0, drift);
        let cover = sweep(24.0, |t| t);
        let cover = Stereo::new(cover.clone(), cover, RATE);
        let out = stretch::along(&cover, original.len(), drift, &|| false).unwrap();
        let mut misses = 0;
        let mut checked = 0;
        let mut t = 1.0;
        while t < 23.0 {
            let (want, got) = (frequency(&original, t), frequency(&out.left, t));
            if ((got - want) / want).abs() > 0.015 {
                misses += 1;
            }
            checked += 1;
            t += 0.5;
        }
        assert!(misses * 20 <= checked, "{misses} of {checked} frames off pitch");
    }

    #[test]
    fn the_tuning_offset_is_measured_whole_semitones_included() {
        let cover = chord(6.0, 0.0);
        for cents in [30.0, -35.0, 130.0, -160.0] {
            let original = chord(6.0, cents);
            let found = tuning_offset(&original, &cover, RATE).unwrap();
            assert!((found as f64 - cents).abs() < 3.0, "{cents} cents read as {found:.1}");
        }
    }

    #[test]
    fn shifting_moves_the_pitch_by_the_cents_asked() {
        let tone: Vec<f32> = (0..RATE as usize * 3).map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin() * 0.5).collect();
        let up = tuning::shift(&tone, 100.0);
        let hz = frequency(&up, 1.0);
        assert!((hz - 466.16).abs() < 0.5, "{hz:.2} Hz");
        assert!((up.len() as f64 - tone.len() as f64 / 2f64.powf(100.0 / 1200.0)).abs() < 2.0);
    }

    #[test]
    fn reback_keeps_the_vocals_untouched_and_lays_the_cover_in_time() {
        let original = sweep(24.0, drift);
        let guide = Stereo::new(original.clone(), original.clone(), RATE);
        // a "voice" the cover never has: a quiet 1 kHz tone
        let voice: Vec<f32> = (0..original.len()).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / RATE as f32).sin() * 0.05).collect();
        let vocals = Stereo::new(voice.clone(), voice.clone(), RATE);
        let cover = sweep(24.0, |t| t);
        let cover = Stereo::new(cover.clone(), cover, RATE);
        let settings = Settings { backing_gain_db: -120.0, tune: false, ..Settings::default() };
        let mut stages = Vec::new();
        let (out, report) = reback(&vocals, &guide, &cover, &settings, &mut |stage| stages.push(stage), &|| false).unwrap().unwrap();
        assert_eq!(stages, [Stage::Tuning, Stage::Aligning, Stage::Stretching, Stage::Mixing]);
        assert_eq!(out.frames(), vocals.frames());
        let worst = out.left.iter().zip(&voice).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "the vocals come out as they went in: {worst}");
        assert_eq!(report.tuning_cents, 0.0);
        let stopped = reback(&vocals, &guide, &cover, &Settings::default(), &mut |_| {}, &|| true).unwrap();
        assert!(stopped.is_none());
    }
}
