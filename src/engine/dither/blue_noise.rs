//! Blue-noise dither via a cached void-and-cluster threshold tile.
//!
//! Void-and-cluster (Ulichney 1993) produces a tileable threshold map
//! whose frequency spectrum is dominated by high frequencies — visually,
//! dots are evenly spaced without the directional artifacts of FS or the
//! repetitive grid of Bayer. The tile is expensive to generate (O(N² log N))
//! so we memoize it per tile size.
//!
//! The current port ships with a pre-built 64×64 tile to avoid a heavy
//! dependency on an FFT crate for the Gaussian-weighted blur pass. The
//! tile is generated deterministically from a xorshift seed the first time
//! it's needed and then reused for the lifetime of the process.

use std::sync::OnceLock;

use image::{GrayImage, ImageBuffer, Luma};

const TILE_SIZE: usize = 64;

static BLUE_NOISE_TILE: OnceLock<[[u8; TILE_SIZE]; TILE_SIZE]> = OnceLock::new();

fn tile() -> &'static [[u8; TILE_SIZE]; TILE_SIZE] {
    BLUE_NOISE_TILE.get_or_init(generate_tile)
}

/// Threshold the input against the blue-noise tile. Tiles across the
/// image, so arbitrary sizes work without generating a bespoke tile.
pub fn blue_noise_grayscale(src: &GrayImage) -> GrayImage {
    let (w, h) = src.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    let t = tile();
    for (x, y, p) in src.enumerate_pixels() {
        let threshold = t[(y as usize) % TILE_SIZE][(x as usize) % TILE_SIZE];
        let bit = if p[0] > threshold { 255 } else { 0 };
        out.put_pixel(x, y, Luma([bit]));
    }
    out
}

/// Generate a void-and-cluster tile. This is a simplified, dependency-free
/// variant of Ulichney's algorithm:
///
/// 1. Start with a random binary pattern at ~10% density.
/// 2. Repeatedly: find the largest "cluster" (densest neighborhood),
///    remove a dot; find the largest "void" (emptiest neighborhood),
///    add a dot. Continue until the pattern is "most homogeneous".
/// 3. Number each dot by removal / placement order, building a threshold
///    map whose isosurface at any level is itself a blue-noise pattern.
///
/// "Largest cluster / void" is measured with a Gaussian-weighted
/// neighborhood sum computed on a torus (wraps at tile edges) so the
/// result is seamless when tiled.
// x and y are used to index `score` AND for spatial offset math via dx/dy,
// so the iter().enumerate() rewrite that clippy suggests doesn't apply.
#[allow(clippy::needless_range_loop)]
fn generate_tile() -> [[u8; TILE_SIZE]; TILE_SIZE] {
    const N: usize = TILE_SIZE;
    const SIGMA: f32 = 1.5;
    const RADIUS: isize = 5;

    // --- precompute Gaussian kernel offsets and weights
    let mut kernel: Vec<(isize, isize, f32)> = Vec::new();
    for dy in -RADIUS..=RADIUS {
        for dx in -RADIUS..=RADIUS {
            let d2 = (dx * dx + dy * dy) as f32;
            let w = (-d2 / (2.0 * SIGMA * SIGMA)).exp();
            if w > 0.005 {
                kernel.push((dx, dy, w));
            }
        }
    }

    // --- initial random binary pattern, ~10% density
    let mut pattern = [[false; N]; N];
    let target = (N * N) / 10;
    let mut rng: u32 = 0xC0FFEE;
    let mut placed = 0;
    while placed < target {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        let x = (rng as usize) % N;
        let y = ((rng as usize) / N) % N;
        if !pattern[y][x] {
            pattern[y][x] = true;
            placed += 1;
        }
    }

    // --- score field: Gaussian sum of surrounding true pixels.
    let mut score = [[0.0f32; N]; N];
    let recompute_score = |pattern: &[[bool; N]; N], score: &mut [[f32; N]; N]| {
        for y in 0..N {
            for x in 0..N {
                let mut s = 0.0f32;
                for &(dx, dy, w) in &kernel {
                    let nx = ((x as isize + dx).rem_euclid(N as isize)) as usize;
                    let ny = ((y as isize + dy).rem_euclid(N as isize)) as usize;
                    if pattern[ny][nx] {
                        s += w;
                    }
                }
                score[y][x] = s;
            }
        }
    };
    recompute_score(&pattern, &mut score);

    // --- iterative redistribution
    let iterations = N * N / 4;
    for _ in 0..iterations {
        // Find largest cluster (max score among ON pixels).
        let (mut max_v, mut mx, mut my) = (-1.0f32, 0usize, 0usize);
        for y in 0..N {
            for x in 0..N {
                if pattern[y][x] && score[y][x] > max_v {
                    max_v = score[y][x];
                    mx = x;
                    my = y;
                }
            }
        }
        pattern[my][mx] = false;
        // Recompute locally by subtracting kernel at (mx, my).
        for &(dx, dy, w) in &kernel {
            let nx = ((mx as isize + dx).rem_euclid(N as isize)) as usize;
            let ny = ((my as isize + dy).rem_euclid(N as isize)) as usize;
            score[ny][nx] -= w;
        }

        // Find largest void (min score among OFF pixels).
        let (mut min_v, mut vx, mut vy) = (f32::INFINITY, 0usize, 0usize);
        for y in 0..N {
            for x in 0..N {
                if !pattern[y][x] && score[y][x] < min_v {
                    min_v = score[y][x];
                    vx = x;
                    vy = y;
                }
            }
        }
        pattern[vy][vx] = true;
        for &(dx, dy, w) in &kernel {
            let nx = ((vx as isize + dx).rem_euclid(N as isize)) as usize;
            let ny = ((vy as isize + dy).rem_euclid(N as isize)) as usize;
            score[ny][nx] += w;
        }

        // If nothing changed this pass the pattern is stable.
        if mx == vx && my == vy {
            break;
        }
    }

    // --- number pixels in order of isolation to build the threshold map.
    // Phase 1: number existing ON pixels from most-clustered to least.
    let initial_on = count_on(&pattern);
    let mut thresholds = [[0u16; N]; N];
    let mut working = pattern;
    recompute_score(&working, &mut score);
    for rank in (0..initial_on).rev() {
        let (mut max_v, mut mx, mut my) = (-1.0f32, 0usize, 0usize);
        for y in 0..N {
            for x in 0..N {
                if working[y][x] && score[y][x] > max_v {
                    max_v = score[y][x];
                    mx = x;
                    my = y;
                }
            }
        }
        thresholds[my][mx] = rank as u16;
        working[my][mx] = false;
        for &(dx, dy, w) in &kernel {
            let nx = ((mx as isize + dx).rem_euclid(N as isize)) as usize;
            let ny = ((my as isize + dy).rem_euclid(N as isize)) as usize;
            score[ny][nx] -= w;
        }
    }

    // Phase 2: number remaining OFF pixels from least-void to most.
    let mut working = pattern;
    recompute_score(&working, &mut score);
    for rank in initial_on..(N * N) {
        let (mut min_v, mut vx, mut vy) = (f32::INFINITY, 0usize, 0usize);
        for y in 0..N {
            for x in 0..N {
                if !working[y][x] && score[y][x] < min_v {
                    min_v = score[y][x];
                    vx = x;
                    vy = y;
                }
            }
        }
        thresholds[vy][vx] = rank as u16;
        working[vy][vx] = true;
        for &(dx, dy, w) in &kernel {
            let nx = ((vx as isize + dx).rem_euclid(N as isize)) as usize;
            let ny = ((vy as isize + dy).rem_euclid(N as isize)) as usize;
            score[ny][nx] += w;
        }
    }

    // Rescale 0..(N*N-1) to 0..255.
    let mut out = [[0u8; N]; N];
    let max_rank = (N * N - 1) as f32;
    for y in 0..N {
        for x in 0..N {
            out[y][x] = ((thresholds[y][x] as f32 / max_rank) * 255.0).round() as u8;
        }
    }
    out
}

fn count_on<const N: usize>(pattern: &[[bool; N]; N]) -> usize {
    let mut n = 0;
    for row in pattern {
        for &p in row {
            if p {
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_output() {
        let src: GrayImage = ImageBuffer::from_fn(32, 32, |x, _| Luma([(x * 8) as u8]));
        let out = blue_noise_grayscale(&src);
        for p in out.iter() {
            assert!(*p == 0 || *p == 255);
        }
    }

    #[test]
    fn tile_is_cached() {
        // Second call should reuse the generated tile.
        let a = tile() as *const _;
        let b = tile() as *const _;
        assert_eq!(a, b);
    }
}
