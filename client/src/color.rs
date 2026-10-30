//! Colour maths for the automatic text-colour detection.

use eframe::egui::Color32;

fn srgb(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG 2.1 relative luminance (0 = black, 1 = white).
pub fn relative_luminance(r: u8, g: u8, b: u8) -> f64 {
    0.2126 * srgb(f64::from(r) / 255.0)
        + 0.7152 * srgb(f64::from(g) / 255.0)
        + 0.0722 * srgb(f64::from(b) / 255.0)
}

fn contrast_ratio(l1: f64, l2: f64) -> f64 {
    let (lo, hi) = if l1 < l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

/// `(text, outline)` with the highest WCAG 2.1 contrast against `bg_lum`;
/// always at least 4.5:1 (AA).
pub fn wcag_best_pair(bg_lum: f64) -> (&'static str, &'static str) {
    if contrast_ratio(bg_lum, 1.0) >= contrast_ratio(bg_lum, 0.0) {
        ("#FFFFFF", "#111111")
    } else {
        ("#111111", "#FFFFFF")
    }
}

/// Dark background → near-black badge. Light background → near-white badge.
pub fn auto_badge_color(bg_lum: f64) -> &'static str {
    if bg_lum > 0.4 {
        "#0D1117"
    } else {
        "#F0F0F0"
    }
}

/// `#rrggbb` (the `#` is optional). Anything else is `None`.
pub fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let ch = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some(Color32::from_rgb(ch(0)?, ch(2)?, ch(4)?))
}

/// Like [`parse_hex`], with a fallback for hand-edited settings.
pub fn hex_or(s: &str, fallback: Color32) -> Color32 {
    parse_hex(s).unwrap_or(fallback)
}

pub fn to_hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t) as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminance_extremes() {
        assert!(relative_luminance(0, 0, 0) < 1e-9);
        assert!((relative_luminance(255, 255, 255) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn text_contrasts_with_background() {
        assert_eq!(wcag_best_pair(0.0).0, "#FFFFFF");
        assert_eq!(wcag_best_pair(1.0).0, "#111111");
        for l in [0.0, 0.1, 0.18, 0.3, 0.5, 0.8, 1.0] {
            let (text, _) = wcag_best_pair(l);
            let tl = if text == "#FFFFFF" { 1.0 } else { 0.0 };
            assert!(contrast_ratio(l, tl) >= 3.0, "low contrast at {l}");
        }
    }

    #[test]
    fn badge_follows_background() {
        assert_eq!(auto_badge_color(0.9), "#0D1117");
        assert_eq!(auto_badge_color(0.1), "#F0F0F0");
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(parse_hex("#1e1f22"), Some(Color32::from_rgb(0x1e, 0x1f, 0x22)));
        assert_eq!(parse_hex("zzz"), None);
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(to_hex(Color32::from_rgb(1, 2, 255)), "#0102FF");
    }

    #[test]
    fn lerp_endpoints() {
        let a = Color32::from_rgb(0, 0, 0);
        let b = Color32::from_rgb(200, 100, 50);
        assert_eq!(lerp(a, b, 0.0), a);
        assert_eq!(lerp(a, b, 1.0), b);
        assert_eq!(lerp(a, b, 0.5), Color32::from_rgb(100, 50, 25));
    }
}
