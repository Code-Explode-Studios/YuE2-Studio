//! Time stretching along a time map without changing the pitch: WSOLA
//! (waveform similarity overlap-add, Verhelst and Roelands 1993). Each output
//! frame is read from where the map puts it in the input, moved by up to a
//! tolerance to where it best continues the frame before, so the waveform
//! joins without phase cancellation. Transients stay sharp, which a phase
//! vocoder smears: what a drum kit needs.

use crate::Stereo;

/// Frame length at 44.1 kHz, about 46 ms; the hop is half of it.
const FRAME: usize = 2048;
const HOP: usize = FRAME / 2;
/// How far a frame may move from where the map puts it, about 12 ms.
const TOLERANCE: isize = 512;
/// The coarse search runs on the signal decimated by this, then refines at full rate.
const COARSE: isize = 4;

fn sample(signal: &[f32], at: isize) -> f32 {
    if at >= 0 && (at as usize) < signal.len() { signal[at as usize] } else { 0.0 }
}

fn correlation(signal: &[f32], a: isize, b: isize, len: usize) -> f32 {
    (0..len as isize).map(|i| sample(signal, a + i) * sample(signal, b + i)).sum()
}

/// Where the frame starting near `nominal` best continues the input from `natural` on.
fn best_start(mid: &[f32], coarse: &[f32], natural: isize, nominal: isize) -> isize {
    let coarse_len = FRAME / COARSE as usize;
    let (natural_c, nominal_c) = (natural.div_euclid(COARSE), nominal.div_euclid(COARSE));
    let reach = TOLERANCE / COARSE;
    let (mut best_c, mut score) = (nominal_c, correlation(coarse, natural_c, nominal_c, coarse_len));
    for delta in -reach..=reach {
        let candidate = correlation(coarse, natural_c, nominal_c + delta, coarse_len);
        // a tie keeps the start nearer the map
        if candidate > score || candidate == score && delta.abs() < (best_c - nominal_c).abs() {
            best_c = nominal_c + delta;
            score = candidate;
        }
    }
    let around = best_c * COARSE + (nominal - nominal_c * COARSE);
    let (mut best, mut score) = (around, correlation(mid, natural, around, FRAME));
    for delta in -COARSE..=COARSE {
        let start = (around + delta).clamp(nominal - TOLERANCE, nominal + TOLERANCE);
        let candidate = correlation(mid, natural, start, FRAME);
        if candidate > score {
            best = start;
            score = candidate;
        }
    }
    best
}

/// `frames` frames of `input` played along `map`, which takes a time of the output in seconds to the time of the
/// input to play there. Both channels move together, by the mid signal.
pub fn along(input: &Stereo, frames: usize, map: impl Fn(f64) -> f64, stop: &dyn Fn() -> bool) -> Option<Stereo> {
    let rate = input.rate as f64;
    let mid: Vec<f32> = input.left.iter().zip(&input.right).map(|(l, r)| (l + r) * 0.5).collect();
    let coarse: Vec<f32> = mid.chunks(COARSE as usize).map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32).collect();
    // periodic Hann: at half overlap the windows add to exactly one
    let window: Vec<f32> = (0..FRAME).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / FRAME as f32).cos()).collect();
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    let mut weight = vec![0.0f32; frames];
    let half = (FRAME / 2) as isize;
    let mut previous: Option<isize> = None;
    for k in 0..frames / HOP + 2 {
        if k % 256 == 0 && stop() {
            return None;
        }
        let centre = (k * HOP) as isize;
        let nominal = (map(centre as f64 / rate) * rate).round() as isize - half;
        let start = match previous {
            None => nominal,
            Some(previous) => best_start(&mid, &coarse, previous + HOP as isize, nominal),
        };
        for (i, w) in window.iter().enumerate() {
            let out = centre - half + i as isize;
            if out < 0 || out as usize >= frames {
                continue;
            }
            let at = start + i as isize;
            left[out as usize] += sample(&input.left, at) * w;
            right[out as usize] += sample(&input.right, at) * w;
            weight[out as usize] += w;
        }
        previous = Some(start);
    }
    for ((l, r), w) in left.iter_mut().zip(right.iter_mut()).zip(&weight) {
        if *w > 1e-3 {
            *l /= w;
            *r /= w;
        }
    }
    Some(Stereo::new(left, right, input.rate))
}
