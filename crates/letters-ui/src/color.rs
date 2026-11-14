//! Colour arithmetic for the looks yewi derives in CSS with `color-mix()`.
//!
//! Every function takes and returns an [`Ink`], so a derived colour stays right in both schemes.

use crate::tokens::{Color as Ink, Rgba, Scheme};

/// `color-mix(in oklch, a, b share)`: `share` of `b` mixed into `a`, as yewi mixes hover and active fills.
pub fn mix(a: Ink, b: Ink, share: f32) -> Ink {
    Ink { light: mix_oklch(a.light, b.light, share), dark: mix_oklch(a.dark, b.dark, share) }
}

/// `color-mix(in srgb, a, b share)`.
pub fn blend(a: Ink, b: Ink, share: f32) -> Ink {
    Ink { light: mix_srgb(a.light, b.light, share), dark: mix_srgb(a.dark, b.dark, share) }
}

/// `color-mix(in srgb, ink alpha, transparent)`: the colour at a share of its opacity.
pub fn fade(ink: Ink, alpha: f32) -> Ink {
    let fade = |c: Rgba| Rgba { a: (f32::from(c.a) * alpha).round().clamp(0.0, 255.0) as u8, ..c };
    Ink { light: fade(ink.light), dark: fade(ink.dark) }
}

/// One colour in both schemes, for colours a document or a person chose.
pub const fn same(r: u8, g: u8, b: u8) -> Ink {
    Ink { light: Rgba::new(r, g, b, 255), dark: Rgba::new(r, g, b, 255) }
}

pub(crate) fn resolve(ink: Ink, scheme: Scheme, opacity: f32) -> letters_render::Color {
    let c = ink.resolve(scheme);
    let a = if opacity >= 1.0 { c.a } else { (f32::from(c.a) * opacity).round() as u8 };
    letters_render::Color::rgba(c.r, c.g, c.b, a)
}

fn mix_srgb(a: Rgba, b: Rgba, share: f32) -> Rgba {
    // Premultiplied, so mixing with a transparent colour fades instead of darkening.
    let (wa, wb) = ((1.0 - share) * f32::from(a.a), share * f32::from(b.a));
    let alpha = wa + wb;
    if alpha <= 0.0 {
        return Rgba::new(0, 0, 0, 0);
    }
    let channel = |x: u8, y: u8| ((f32::from(x) * wa + f32::from(y) * wb) / alpha).round() as u8;
    Rgba::new(channel(a.r, b.r), channel(a.g, b.g), channel(a.b, b.b), alpha.round() as u8)
}

fn mix_oklch(a: Rgba, b: Rgba, share: f32) -> Rgba {
    let ([la, ca, ha], [lb, cb, hb]) = (to_oklch(a), to_oklch(b));
    // A grey has no hue of its own, so it takes the other colour's.
    const GREY: f32 = 1e-4;
    let (ha, hb) = match (ca < GREY, cb < GREY) {
        (true, false) => (hb, hb),
        (false, true) => (ha, ha),
        _ => (ha, hb),
    };
    let mut turn = hb - ha;
    if turn > 180.0 {
        turn -= 360.0;
    } else if turn < -180.0 {
        turn += 360.0;
    }
    let lerp = |x: f32, y: f32| x + (y - x) * share;
    let alpha = lerp(f32::from(a.a), f32::from(b.a)).round() as u8;
    from_oklch([lerp(la, lb), lerp(ca, cb), ha + turn * share], alpha)
}

fn to_linear(channel: u8) -> f32 {
    let s = f32::from(channel) / 255.0;
    if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}

fn from_linear(value: f32) -> u8 {
    let v = value.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

fn to_oklch(c: Rgba) -> [f32; 3] {
    let (r, g, b) = (to_linear(c.r), to_linear(c.g), to_linear(c.b));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let b = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    [lightness, a.hypot(b), b.atan2(a).to_degrees()]
}

fn from_oklch([lightness, chroma, hue]: [f32; 3], alpha: u8) -> Rgba {
    let (a, b) = (chroma * hue.to_radians().cos(), chroma * hue.to_radians().sin());
    let l = (lightness + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (lightness - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (lightness - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    Rgba::new(
        from_linear(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
        from_linear(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s),
        from_linear(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
        alpha,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens;

    fn close(a: Rgba, b: Rgba) -> bool {
        let near = |x: u8, y: u8| x.abs_diff(y) <= 1;
        near(a.r, b.r) && near(a.g, b.g) && near(a.b, b.b) && near(a.a, b.a)
    }

    #[test]
    fn mixing_matches_the_values_yewi_generates() {
        // `--yi-accent-solid-hover` is `color-mix(in oklch, accent-solid, black or white 12%)`.
        let toward = Ink { light: Rgba::new(0, 0, 0, 255), dark: Rgba::new(255, 255, 255, 255) };
        let hover = mix(tokens::ACCENT_SOLID, toward, 0.12);
        assert!(close(hover.light, tokens::ACCENT_SOLID_HOVER.light), "{hover:?}");
        assert!(close(hover.dark, tokens::ACCENT_SOLID_HOVER.dark), "{hover:?}");
    }

    #[test]
    fn fading_keeps_the_colour_and_scales_its_alpha() {
        assert_eq!(fade(tokens::ACCENT, 0.12), tokens::ACCENT_SOFT);
        assert_eq!(fade(tokens::LINE, 0.7), tokens::CARD_LINE);
    }

    #[test]
    fn blending_with_an_opaque_colour_stays_opaque() {
        let edge = blend(tokens::SURFACE, tokens::ACCENT, 0.45);
        assert_eq!(edge.light.a, 255);
        assert!(edge.light.b > edge.light.r, "leans towards the accent: {edge:?}");
    }
}
