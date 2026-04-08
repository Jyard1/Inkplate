//! Serpentine Floyd-Steinberg error diffusion for grayscale → binary.
//!
//! Serpentine (alternating left-to-right / right-to-left rows) avoids the
//! diagonal worm artifacts that plain FS produces on smooth gradients.
//! Output is still in the density convention — 0 = full ink, 255 = no ink.

use image::{GrayImage, ImageBuffer};

/// Floyd-Steinberg dither a density map down to pure binary 0/255.
pub fn floyd_steinberg_grayscale(src: &GrayImage) -> GrayImage {
    let (w, h) = src.dimensions();
    if w == 0 || h == 0 {
        return src.clone();
    }
    let mut buf: Vec<f32> = src.iter().map(|&p| p as f32).collect();
    let stride = w as usize;

    for y in 0..h as usize {
        let left_to_right = y % 2 == 0;
        let xs: Box<dyn Iterator<Item = usize>> = if left_to_right {
            Box::new(0..stride)
        } else {
            Box::new((0..stride).rev())
        };

        for x in xs {
            let i = y * stride + x;
            let old = buf[i];
            let new = if old < 128.0 { 0.0 } else { 255.0 };
            buf[i] = new;
            let err = old - new;

            // Serpentine: flip the offsets when walking right-to-left.
            let (dx1, dx2) = if left_to_right {
                (1i32, -1i32)
            } else {
                (-1i32, 1i32)
            };

            let set = |buf: &mut [f32], xi: i32, yi: usize, w: f32| {
                if xi < 0 || xi >= stride as i32 {
                    return;
                }
                let j = yi * stride + xi as usize;
                buf[j] += err * w;
            };

            set(&mut buf, x as i32 + dx1, y, 7.0 / 16.0);
            if y + 1 < h as usize {
                set(&mut buf, x as i32 + dx2, y + 1, 3.0 / 16.0);
                set(&mut buf, x as i32, y + 1, 5.0 / 16.0);
                set(&mut buf, x as i32 + dx1, y + 1, 1.0 / 16.0);
            }
        }
    }

    let out: Vec<u8> = buf
        .iter()
        .map(|&v| if v < 128.0 { 0u8 } else { 255u8 })
        .collect();
    ImageBuffer::from_raw(w, h, out).expect("size matches")
}

#[cfg(test)]
mod tests {
    use image::Luma;

    use super::*;

    #[test]
    fn binary_output() {
        let src: GrayImage = ImageBuffer::from_fn(8, 8, |x, _| Luma([(x * 32) as u8]));
        let out = floyd_steinberg_grayscale(&src);
        for p in out.iter() {
            assert!(*p == 0 || *p == 255);
        }
    }

    #[test]
    fn flat_is_preserved() {
        let src: GrayImage = ImageBuffer::from_pixel(16, 16, Luma([255]));
        let out = floyd_steinberg_grayscale(&src);
        for p in out.iter() {
            assert_eq!(*p, 255);
        }

        let src: GrayImage = ImageBuffer::from_pixel(16, 16, Luma([0]));
        let out = floyd_steinberg_grayscale(&src);
        for p in out.iter() {
            assert_eq!(*p, 0);
        }
    }
}
