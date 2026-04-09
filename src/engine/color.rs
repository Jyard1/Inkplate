//! Color science: sRGB ↔ CIE LAB conversion, ΔE, hex parsing, color naming.
//!
//! All LAB conversion uses the D65 illuminant to match the Python reference.
//! Functions come in both scalar (`rgb_to_lab`) and bulk (`rgb_slice_to_lab`)
//! forms. The bulk form writes into a caller-provided buffer so tight
//! pipeline loops don't pay per-pixel allocation cost.

use std::fmt;

/// An sRGB triple, 8-bit per channel, gamma-encoded. This is the format
/// stored in [`image::RgbImage`] and in layer ink swatches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const BLACK: Rgb = Rgb(0, 0, 0);
    pub const WHITE: Rgb = Rgb(255, 255, 255);

    pub fn to_array(self) -> [u8; 3] {
        [self.0, self.1, self.2]
    }

    pub fn from_array(a: [u8; 3]) -> Self {
        Rgb(a[0], a[1], a[2])
    }

    /// Linear BT.601 luminance in `[0, 255]`. Cheap; use [`to_lab`](Self::to_lab)
    /// if you need perceptually uniform distance.
    pub fn luma_601(self) -> u8 {
        let l = 0.299 * self.0 as f32 + 0.587 * self.1 as f32 + 0.114 * self.2 as f32;
        l.round().clamp(0.0, 255.0) as u8
    }

    pub fn to_lab(self) -> Lab {
        rgb_to_lab(self)
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }
}

/// CIE LAB color, D65 illuminant. `l` in `[0, 100]`, `a`/`b` roughly in
/// `[-128, 127]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
}

impl Lab {
    /// Euclidean ΔE*ab (CIE76). Fast, rough — fine for quick nearest-
    /// neighbour lookups where perceptual accuracy doesn't matter.
    /// For palette clustering and spot-plate assignment use
    /// [`Self::delta_e94`] instead, which weights lightness less than
    /// chroma/hue and matches how a screen printer eyeballs colors.
    pub fn delta_e(self, other: Lab) -> f32 {
        let dl = self.l - other.l;
        let da = self.a - other.a;
        let db = self.b - other.b;
        (dl * dl + da * da + db * db).sqrt()
    }

    /// CIE94 ΔE — graphic-arts variant with `k_L = 2`.
    ///
    /// This is the distance we use for every "is color A visually the
    /// same as color B?" decision in the engine: palette clustering,
    /// spot plate assignment, hue-family merging. The graphic-arts
    /// variant down-weights lightness relative to chroma and hue,
    /// which is the right trade-off for screen printing because:
    ///
    /// - A dark shadow of a red object is still "red" to the press
    ///   operator — they want it on the red screen, not on black.
    /// - Two shades of the same hue (bright red + blood red) should
    ///   land on the same plate.
    /// - Distinct hues (red vs orange, cyan vs teal) should stay on
    ///   separate plates even when their lightness is similar.
    ///
    /// Reference: `other` is treated as the reference color (its
    /// chroma drives the normalisation). For palette-assignment calls
    /// pass the palette target as `other`.
    pub fn delta_e94(self, other: Lab) -> f32 {
        // Graphic-arts weighting factors from the CIE94 paper.
        const K_L: f32 = 2.0;
        const K1: f32 = 0.048;
        const K2: f32 = 0.014;

        let dl = self.l - other.l;
        let da = self.a - other.a;
        let db = self.b - other.b;

        let c1 = other.chroma();
        let c2 = self.chroma();
        let dc = c2 - c1;
        // dH² = dA² + dB² − dC². Floating-point slop can push this
        // slightly negative for near-identical colors, so clamp.
        let dh_sq = (da * da + db * db - dc * dc).max(0.0);

        let s_l = 1.0;
        let s_c = 1.0 + K1 * c1;
        let s_h = 1.0 + K2 * c1;

        let tl = dl / (K_L * s_l);
        let tc = dc / s_c;
        let th_sq = dh_sq / (s_h * s_h);

        (tl * tl + tc * tc + th_sq).sqrt()
    }

    /// Hue angle in degrees, `[0, 360)`.
    pub fn hue_deg(self) -> f32 {
        let h = self.b.atan2(self.a).to_degrees();
        if h < 0.0 {
            h + 360.0
        } else {
            h
        }
    }

    /// Chroma = √(a² + b²). Low chroma ≈ near-grayscale.
    pub fn chroma(self) -> f32 {
        (self.a * self.a + self.b * self.b).sqrt()
    }
}

// ---------------------------------------------------------------------------
// sRGB ↔ LAB
// ---------------------------------------------------------------------------

/// Gamma-decode an 8-bit sRGB channel to linear `[0, 1]`.
#[inline]
fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.040_448 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear `[0, 1]` back to 8-bit gamma-encoded sRGB.
#[inline]
fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let out = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (out * 255.0).round().clamp(0.0, 255.0) as u8
}

/// D65 white point in XYZ (CIE 1931 2°).
const XN: f32 = 0.950_47;
const YN: f32 = 1.000_00;
const ZN: f32 = 1.088_83;

#[inline]
fn xyz_f(t: f32) -> f32 {
    const DELTA: f32 = 6.0 / 29.0;
    if t > DELTA * DELTA * DELTA {
        t.cbrt()
    } else {
        t / (3.0 * DELTA * DELTA) + 4.0 / 29.0
    }
}

pub fn rgb_to_lab(rgb: Rgb) -> Lab {
    let r = srgb_to_linear(rgb.0);
    let g = srgb_to_linear(rgb.1);
    let b = srgb_to_linear(rgb.2);

    // sRGB (linear) → XYZ, D65
    let x = 0.412_456 * r + 0.357_576 * g + 0.180_437 * b;
    let y = 0.212_673 * r + 0.715_152 * g + 0.072_175 * b;
    let z = 0.019_334 * r + 0.119_192 * g + 0.950_304 * b;

    let fx = xyz_f(x / XN);
    let fy = xyz_f(y / YN);
    let fz = xyz_f(z / ZN);

    Lab {
        l: 116.0 * fy - 16.0,
        a: 500.0 * (fx - fy),
        b: 200.0 * (fy - fz),
    }
}

pub fn lab_to_rgb(lab: Lab) -> Rgb {
    const DELTA: f32 = 6.0 / 29.0;
    let fy = (lab.l + 16.0) / 116.0;
    let fx = fy + lab.a / 500.0;
    let fz = fy - lab.b / 200.0;

    let finv = |f: f32| -> f32 {
        if f > DELTA {
            f * f * f
        } else {
            3.0 * DELTA * DELTA * (f - 4.0 / 29.0)
        }
    };

    let x = XN * finv(fx);
    let y = YN * finv(fy);
    let z = ZN * finv(fz);

    // XYZ → linear sRGB (D65)
    let r = 3.240_454 * x - 1.537_138 * y - 0.498_531 * z;
    let g = -0.969_266 * x + 1.876_01 * y + 0.041_556 * z;
    let b = 0.055_643 * x - 0.204_025 * y + 1.057_225 * z;

    Rgb(linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b))
}

/// Convert a flat RGB pixel buffer (`[r, g, b, r, g, b, ...]`) into a
/// parallel Lab buffer of the same length in triples. The output buffer
/// must already be sized to `pixels.len() / 3`.
///
/// This is the hot path for palette extraction and color-range fuzziness
/// — callers should reuse the output buffer across frames.
pub fn rgb_slice_to_lab(pixels: &[u8], out: &mut [Lab]) {
    assert_eq!(pixels.len() / 3, out.len(), "RGB/LAB length mismatch");
    for (i, chunk) in pixels.chunks_exact(3).enumerate() {
        out[i] = rgb_to_lab(Rgb(chunk[0], chunk[1], chunk[2]));
    }
}

// ---------------------------------------------------------------------------
// Hex parsing
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum HexError {
    #[error("hex color must be 6 digits, got {0}")]
    Length(usize),
    #[error("invalid hex digit in {0:?}")]
    Parse(String),
}

/// Parse `#RRGGBB` or `RRGGBB` into an [`Rgb`].
pub fn hex_to_rgb(hex: &str) -> Result<Rgb, HexError> {
    let s = hex.strip_prefix('#').unwrap_or(hex);
    if s.len() != 6 {
        return Err(HexError::Length(s.len()));
    }
    let parse = |h: &str| u8::from_str_radix(h, 16).map_err(|_| HexError::Parse(s.to_string()));
    Ok(Rgb(parse(&s[0..2])?, parse(&s[2..4])?, parse(&s[4..6])?))
}

// ---------------------------------------------------------------------------
// Color naming
// ---------------------------------------------------------------------------

/// Short, filename-safe name for an arbitrary RGB value: `red`, `darkblue`,
/// `gray`, `black`, etc. Used for auto-generated layer names and export
/// filename templates.
pub fn color_name(rgb: Rgb) -> &'static str {
    let Rgb(r, g, b) = rgb;
    let r = r as i32;
    let g = g as i32;
    let b = b as i32;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let chroma = max - min;

    if max < 32 {
        return "black";
    }
    if min > 224 {
        return "white";
    }
    if chroma < 20 {
        return match max {
            m if m < 96 => "darkgray",
            m if m > 180 => "lightgray",
            _ => "gray",
        };
    }

    // Hue bucketing: pick the dominant channel and label by neighbour.
    //
    // "light" here means desaturated / pastel — pure red (255, 0, 0) should
    // be called "red", not "pink". That requires both a high max *and* a
    // high min, otherwise saturated primaries get pastel names.
    let light = max > 200 && min > 80;
    let dark = max < 110;
    let base = if r == max && g >= b {
        if g > r * 2 / 3 {
            "yellow"
        } else if b > r / 3 {
            "magenta"
        } else {
            "red"
        }
    } else if r == max {
        "red"
    } else if g == max && r >= b {
        if r > g * 2 / 3 {
            "yellow"
        } else {
            "green"
        }
    } else if g == max {
        "green"
    } else if b >= r && b >= g {
        if r > b * 2 / 3 {
            "purple"
        } else if g > b * 2 / 3 {
            "cyan"
        } else {
            "blue"
        }
    } else {
        "gray"
    };

    match (base, light, dark) {
        ("red", true, _) => "pink",
        (c, _, true) => match c {
            "red" => "darkred",
            "green" => "darkgreen",
            "blue" => "darkblue",
            "yellow" => "olive",
            "cyan" => "teal",
            "magenta" => "darkmagenta",
            "purple" => "darkpurple",
            other => other,
        },
        (c, _, _) => c,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn hex_roundtrip() {
        assert_eq!(hex_to_rgb("#FF8040").unwrap(), Rgb(0xFF, 0x80, 0x40));
        assert_eq!(hex_to_rgb("000000").unwrap(), Rgb::BLACK);
        assert_eq!(Rgb(255, 128, 64).to_string(), "#FF8040");
        assert!(hex_to_rgb("#ZZZZZZ").is_err());
        assert!(hex_to_rgb("#12345").is_err());
    }

    #[test]
    fn lab_known_values() {
        // Pure white → L≈100, a≈0, b≈0
        let white = rgb_to_lab(Rgb::WHITE);
        assert_relative_eq!(white.l, 100.0, epsilon = 0.1);
        assert_relative_eq!(white.a, 0.0, epsilon = 0.1);
        assert_relative_eq!(white.b, 0.0, epsilon = 0.1);

        // Pure black → L=0
        let black = rgb_to_lab(Rgb::BLACK);
        assert_relative_eq!(black.l, 0.0, epsilon = 0.1);

        // Mid gray → L≈53.4
        let gray = rgb_to_lab(Rgb(128, 128, 128));
        assert_relative_eq!(gray.l, 53.39, epsilon = 0.2);
        assert_relative_eq!(gray.a, 0.0, epsilon = 0.1);
    }

    #[test]
    fn lab_roundtrip_is_close() {
        for &c in &[
            Rgb(200, 40, 40),
            Rgb(20, 180, 90),
            Rgb(70, 110, 220),
            Rgb(180, 180, 40),
        ] {
            let rt = lab_to_rgb(rgb_to_lab(c));
            // ±2 on each channel is fine; sRGB quantization eats the rest.
            assert!((rt.0 as i32 - c.0 as i32).abs() <= 2);
            assert!((rt.1 as i32 - c.1 as i32).abs() <= 2);
            assert!((rt.2 as i32 - c.2 as i32).abs() <= 2);
        }
    }

    #[test]
    fn hue_angle_sanity() {
        assert!(rgb_to_lab(Rgb(200, 20, 20)).hue_deg() < 45.0); // red-ish
        let green = rgb_to_lab(Rgb(20, 200, 20)).hue_deg();
        assert!((90.0..180.0).contains(&green));
    }

    #[test]
    fn delta_e94_sanity() {
        // Identical colors → 0.
        let red = rgb_to_lab(Rgb(220, 30, 30));
        assert!(red.delta_e94(red) < 0.01);

        // A dark shade of red should be "closer" (smaller ΔE94) to
        // the bright red reference than pure black is. Under plain
        // CIE76 black actually wins — that's the bug CIE94 fixes.
        let dark_red = rgb_to_lab(Rgb(90, 15, 15));
        let black = rgb_to_lab(Rgb::BLACK);
        let d_to_red = dark_red.delta_e94(red);
        let d_to_black = black.delta_e94(red);
        assert!(
            d_to_red < d_to_black,
            "dark red ({d_to_red}) must be closer to red than black ({d_to_black}) is to red"
        );

        // A saturated yellow must land closer to yellow than to red
        // under CIE94.
        let yellow = rgb_to_lab(Rgb(240, 220, 40));
        let yellow_target = rgb_to_lab(Rgb(230, 210, 30));
        let d_yy = yellow.delta_e94(yellow_target);
        let d_yr = yellow.delta_e94(red);
        assert!(
            d_yy < d_yr,
            "yellow→yellow ({d_yy}) must be < yellow→red ({d_yr})"
        );
    }

    #[test]
    fn names_are_sensible() {
        assert_eq!(color_name(Rgb(255, 0, 0)), "red");
        assert_eq!(color_name(Rgb(0, 0, 0)), "black");
        assert_eq!(color_name(Rgb(255, 255, 255)), "white");
        assert_eq!(color_name(Rgb(128, 128, 128)), "gray");
        assert_eq!(color_name(Rgb(0, 0, 180)), "blue");
    }
}
