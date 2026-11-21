//! What the alignment compares, frame by frame: the harmony (chroma, twelve
//! pitch classes) and the attacks (onset strength, the rise of the spectrum).
//! Both hold up across arrangements: a new band plays the same chords on the
//! same beats with other instruments.

use crate::stft::Stft;

/// Features are taken at this rate: harmony and attacks lie well below its Nyquist.
pub const RATE: u32 = 22_050;
const SIZE: usize = 4096;
const LOW_HZ: f32 = 55.0;
const HIGH_HZ: f32 = 5_000.0;

pub struct Features {
    /// Per frame, centred (the flat part removed) and of unit length, or all zero in silence.
    pub chroma: Vec<[f32; 12]>,
    /// Per frame, about 0 to 1.
    pub onset: Vec<f32>,
    /// Seconds between frames.
    pub hop_seconds: f64,
    /// Where frame 0 is centred, in seconds of the signal (before its start: the analysis pads).
    pub first_seconds: f64,
}

impl Features {
    pub fn len(&self) -> usize {
        self.chroma.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chroma.is_empty()
    }

    /// The time of a frame, fractional frames included.
    pub fn time_of(&self, frame: f64) -> f64 {
        self.first_seconds + frame * self.hop_seconds
    }
}

/// Each spectrum bin's two neighbouring pitch classes and its share of the upper one, for a reference A `cents`
/// away from 440 Hz; none outside the band.
pub(crate) fn bin_classes(size: usize, rate: u32, cents: f32, low: f32, high: f32) -> Vec<Option<(usize, f32)>> {
    (0..size / 2 + 1)
        .map(|k| {
            let hz = k as f32 * rate as f32 / size as f32;
            (low..=high).contains(&hz).then(|| {
                let class = (69.0 + 12.0 * (hz / 440.0).log2() - cents / 100.0).rem_euclid(12.0);
                ((class.floor() as usize) % 12, class.fract())
            })
        })
        .collect()
}

/// A chroma vector with its mean taken out and scaled to unit length; zero when there is nothing in it.
pub(crate) fn centred(mut chroma: [f32; 12]) -> [f32; 12] {
    let mean = chroma.iter().sum::<f32>() / 12.0;
    chroma.iter_mut().for_each(|value| *value -= mean);
    let length = chroma.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length > 1e-6 {
        chroma.iter_mut().for_each(|value| *value /= length);
    } else {
        chroma = [0.0; 12];
    }
    chroma
}

/// The features of `mono`, sampled at `RATE`, every `hop` samples.
pub fn features(mono: &[f32], hop: usize) -> Features {
    let stft = Stft::new(SIZE, hop);
    let classes = bin_classes(SIZE, RATE, 0.0, LOW_HZ, HIGH_HZ);
    // a full-scale sine reads as 1 after this
    let scale = 4.0 / SIZE as f32;
    let mut chroma = Vec::with_capacity(stft.frame_count(mono.len()));
    let mut flux = Vec::with_capacity(chroma.capacity());
    let mut last = vec![0.0f32; SIZE / 2 + 1];
    stft.each_frame(mono, |_, spectrum| {
        let mut frame = [0.0f32; 12];
        let mut rise = 0.0f32;
        for (k, value) in spectrum.iter().enumerate() {
            let level = (1.0 + 100.0 * value.norm() * scale).ln();
            rise += (level - last[k]).max(0.0);
            last[k] = level;
            if let Some((class, upper)) = classes[k] {
                frame[class] += level * (1.0 - upper);
                frame[(class + 1) % 12] += level * upper;
            }
        }
        chroma.push(centred(frame));
        flux.push(rise);
    });
    // scaled by a high percentile, so one crash does not flatten every other attack
    let mut sorted: Vec<f32> = flux.iter().copied().filter(|value| *value > 0.0).collect();
    sorted.sort_by(f32::total_cmp);
    let top = sorted.get(sorted.len().saturating_sub(1) * 98 / 100).copied().unwrap_or(0.0);
    let onset = flux.iter().map(|value| if top > 0.0 { (value / top).min(1.5) } else { 0.0 }).collect();
    Features { chroma, onset, hop_seconds: hop as f64 / RATE as f64, first_seconds: stft.frame_center(0) / RATE as f64 }
}
