//! Text clean-up shared by the overlay and the voice.

/// Characters the overlay font can't be trusted with: anything outside the
/// BMP (emoji are drawn as boxes) and control characters. They are replaced by
/// spaces, then whitespace is collapsed.
pub fn display_safe(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if (c as u32) <= 0xFFFF && !c.is_control() && !is_invisible(c) { c } else { ' ' })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Unicode "Cf" (format) and private-use characters that survive `is_control`:
/// zero-width joiners, bidi marks, BOM, soft hyphen… None draw anything useful.
fn is_invisible(c: char) -> bool {
    matches!(
        c as u32,
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0xE000..=0xF8FF // private use: no glyphs
    )
}

/// The speaker markers (`*name*`) are for the eyes, not the voice.
pub fn clean_tts_text(text: &str) -> String {
    display_safe(&text.replace('*', ""))
}

/// Credentials in the URL must never reach the screen (this overlay is often streamed).
pub fn redact_url(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let query = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((k, _)) if k.eq_ignore_ascii_case("token") => format!("{k}=***"),
            _ => pair.to_string(),
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{query}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_astral_and_control() {
        assert_eq!(display_safe("hola \u{1F602} mundo"), "hola mundo");
        assert_eq!(display_safe("a\n\tb\0c"), "a b c");
        assert_eq!(display_safe("  espacios   raros  "), "espacios raros");
    }

    #[test]
    fn keeps_bmp_symbols() {
        assert_eq!(display_safe("coraz\u{f3}n \u{2764} ok"), "coraz\u{f3}n \u{2764} ok");
    }

    #[test]
    fn drops_invisible_format_chars() {
        assert_eq!(display_safe("a\u{200B}b"), "a b");
    }

    #[test]
    fn tts_text() {
        assert_eq!(clean_tts_text("*Lauful*: mira esto"), "Lauful: mira esto");
    }

    #[test]
    fn token_is_redacted() {
        assert_eq!(redact_url("ws://h:1/?token=abc&x=1"), "ws://h:1/?token=***&x=1");
        assert_eq!(redact_url("ws://h:1"), "ws://h:1");
    }
}
