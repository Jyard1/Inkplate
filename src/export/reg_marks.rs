//! Registration marks — crosshairs in the four corners of the film so
//! the press operator can align every screen to the same reference
//! points before printing.
//!
//! Each mark is a `+` crosshair centered at a configurable offset
//! from the nearest corner. The lines are drawn with anti-aliased
//! segments via `imageproc::drawing::draw_antialiased_line_segment_mut`
//! so they stay readable even at moderate DPI.
//!
//! All films in one export job should be drawn with identical opts so
//! the marks register across screens.

use image::GrayImage;
use imageproc::drawing::{draw_antialiased_line_segment_mut, draw_filled_circle_mut};
use imageproc::pixelops::interpolate;

#[derive(Debug, Clone, Copy)]
pub struct RegMarkOpts {
    /// Distance from the edge of the film to the mark center, in pixels.
    pub offset_px: i32,
    /// Length of each crosshair arm, in pixels.
    pub arm_length: i32,
    /// Line thickness, in pixels (1 = single-pixel anti-aliased line).
    pub thickness: u32,
    /// Optional filled center dot — useful when the crosshair lines
    /// are very thin.
    pub center_dot: bool,
}

impl Default for RegMarkOpts {
    fn default() -> Self {
        Self {
            offset_px: 48,
            arm_length: 32,
            thickness: 2,
            center_dot: true,
        }
    }
}

pub fn draw(img: &mut GrayImage, opts: &RegMarkOpts) {
    let (w, h) = img.dimensions();
    let w = w as i32;
    let h = h as i32;

    let centers = [
        (opts.offset_px, opts.offset_px),
        (w - opts.offset_px - 1, opts.offset_px),
        (opts.offset_px, h - opts.offset_px - 1),
        (w - opts.offset_px - 1, h - opts.offset_px - 1),
    ];

    for &(cx, cy) in &centers {
        draw_crosshair(img, cx, cy, opts);
        if opts.center_dot {
            let r = (opts.thickness as i32 + 1).max(2);
            draw_filled_circle_mut(img, (cx, cy), r, image::Luma([0]));
        }
    }
}

fn draw_crosshair(img: &mut GrayImage, cx: i32, cy: i32, opts: &RegMarkOpts) {
    let arm = opts.arm_length;
    let thickness = opts.thickness.max(1) as i32;
    // Horizontal arm(s)
    for dy in 0..thickness {
        let yy = cy - thickness / 2 + dy;
        draw_antialiased_line_segment_mut(
            img,
            (cx - arm, yy),
            (cx + arm, yy),
            image::Luma([0]),
            interpolate,
        );
    }
    // Vertical arm(s)
    for dx in 0..thickness {
        let xx = cx - thickness / 2 + dx;
        draw_antialiased_line_segment_mut(
            img,
            (xx, cy - arm),
            (xx, cy + arm),
            image::Luma([0]),
            interpolate,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Luma};

    #[test]
    fn marks_darken_corners() {
        let mut img: GrayImage = ImageBuffer::from_pixel(200, 200, Luma([255]));
        draw(&mut img, &RegMarkOpts::default());
        // Top-left center should be dark now.
        let opts = RegMarkOpts::default();
        let (cx, cy) = (opts.offset_px as u32, opts.offset_px as u32);
        assert!(img.get_pixel(cx, cy)[0] < 128);
    }
}
