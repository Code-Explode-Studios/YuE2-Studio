//! Dynamic time warping between two feature sequences: the cheapest monotonic
//! path from both starts to both ends.

use super::features::Features;

/// Added to a step that holds one sequence still, so a path does not stall where a diagonal costs the same.
const STALL: f32 = 0.02;

/// The distance of frame `i` of `a` from frame `j` of `b`: harmony (cosine, 0 to 1) and attacks.
fn cost(a: &Features, i: usize, b: &Features, j: usize, onset_weight: f32) -> f32 {
    let dot: f32 = a.chroma[i].iter().zip(&b.chroma[j]).map(|(x, y)| x * y).sum();
    (1.0 - dot) * 0.5 + onset_weight * (a.onset[i] - b.onset[j]).abs()
}

/// The aligned frame pairs `(i of a, j of b)`, from `(0, 0)` to the two last frames, in order.
pub fn path(a: &Features, b: &Features, onset_weight: f32) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 {
        return Vec::new();
    }
    // 0: diagonal, 1: from the frame before in a, 2: from the frame before in b
    let mut steps = vec![0u8; n * m];
    let mut previous = vec![f32::INFINITY; m];
    let mut current = vec![f32::INFINITY; m];
    for i in 0..n {
        for j in 0..m {
            let d = cost(a, i, b, j, onset_weight);
            if i == 0 && j == 0 {
                current[0] = d;
                continue;
            }
            let diagonal = if i > 0 && j > 0 { previous[j - 1] + 2.0 * d } else { f32::INFINITY };
            let from_a = if i > 0 { previous[j] + d + STALL } else { f32::INFINITY };
            let from_b = if j > 0 { current[j - 1] + d + STALL } else { f32::INFINITY };
            let (best, step) = if diagonal <= from_a && diagonal <= from_b {
                (diagonal, 0)
            } else if from_a <= from_b {
                (from_a, 1)
            } else {
                (from_b, 2)
            };
            current[j] = best;
            steps[i * m + j] = step;
        }
        std::mem::swap(&mut previous, &mut current);
        current.fill(f32::INFINITY);
    }
    let (mut i, mut j) = (n - 1, m - 1);
    let mut path = vec![(i, j)];
    while i > 0 || j > 0 {
        match steps[i * m + j] {
            0 => {
                i -= 1;
                j -= 1;
            }
            1 => i -= 1,
            _ => j -= 1,
        }
        path.push((i, j));
    }
    path.reverse();
    path
}
