//! Text clean-up shared by the overlay and the voice.

/// What the overlay may draw: anything but control characters, invisible
/// format characters and astral characters other than emoji (fonts have no
/// glyphs for those). The zero-width joiner stays: it glues emoji sequences
/// like 👨‍👩‍👧 together. Characters that fail are replaced by spaces, then
/// whitespace is collapsed.
pub fn display_safe(text: &str) -> String {
    squash(text).trim_matches(' ').to_string()
}

/// [`display_safe`] without the trim, so pieces of one line can be joined.
pub fn squash(text: &str) -> String {
    collapse(text.chars().map(|c| if drawable(c) { c } else { ' ' }))
}

fn drawable(c: char) -> bool {
    !c.is_control() && ((c as u32) <= 0xFFFF || is_astral_emoji(c)) && (c == '\u{200D}' || !is_invisible(c))
}

/// Pictographs, regional indicators (flags) and the tag characters of
/// subdivision flags.
pub fn is_astral_emoji(c: char) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF | 0xE0020..=0xE007F)
}

/// Whitespace runs → one space; ends are kept.
fn collapse(chars: impl Iterator<Item = char>) -> String {
    let mut out = String::new();
    for c in chars {
        if !c.is_whitespace() {
            out.push(c);
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out
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

/// A Discord custom emoji at the start of `text` (`<:name:id>`, or
/// `<a:name:id>` when animated): its length in bytes and its image URL.
pub fn custom_emoji(text: &str) -> Option<(usize, String)> {
    let end = text.find('>')?;
    let inner = text.get(1..end).filter(|_| text.starts_with('<'))?;
    let mut parts = inner.split(':');
    let animated = match parts.next()? {
        "" => false,
        "a" => true,
        _ => return None,
    };
    let name = parts.next()?;
    let id = parts.next()?;
    let valid = !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !id.is_empty()
        && id.bytes().all(|b| b.is_ascii_digit());
    if !valid || parts.next().is_some() {
        return None;
    }
    let ext = if animated { "gif" } else { "png" };
    Some((end + 1, format!("https://cdn.discordapp.com/emojis/{id}.{ext}")))
}

/// The voice skips emoji (astral and custom) and the speaker markers (`*name*`).
pub fn clean_tts_text(text: &str) -> String {
    let mut kept = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if let Some((len, _)) = custom_emoji(rest) {
            kept.push(' ');
            rest = &rest[len..];
            continue;
        }
        let speakable = (c as u32) <= 0xFFFF && !c.is_control() && !is_invisible(c);
        if c != '*' {
            kept.push(if speakable { c } else { ' ' });
        }
        rest = &rest[c.len_utf8()..];
    }
    collapse(kept.chars()).trim_matches(' ').to_string()
}

/// A chat message with its voice markup taken apart.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Markup {
    /// For the overlay: no `//voice//` template and no `[...]` spans, except
    /// the tags the server writes itself (`[Link de …]`, `[GIF]`, …).
    pub shown: String,
    /// For Fish Audio: the template is gone, `[...]` cues stay (Fish reads
    /// them as emotion and tone directions). Other voices get
    /// [`strip_brackets`] on top.
    pub spoken: String,
    /// The Fish Audio voice id of the first `//id//` template, lowercased.
    pub voice: Option<String>,
}

/// Length of a Fish Audio voice id: 32 hex characters.
const VOICE_ID_LEN: usize = 32;

/// Tags the server puts in the text; they are not voice cues.
fn is_server_tag(inner: &str) -> bool {
    inner.starts_with("Link de ") || matches!(inner, "GIF" | "Imagen" | "Archivo")
}

/// The voice id of a `//id//` template at the start of `text`.
fn voice_template(text: &str) -> Option<&str> {
    let id = text.strip_prefix("//")?.get(..VOICE_ID_LEN)?;
    let after = &text[2 + VOICE_ID_LEN..];
    let closed = after.starts_with("//") && !after[2..].starts_with('/');
    (closed && id.bytes().all(|b| b.is_ascii_hexdigit())).then_some(id)
}

/// Split a message into what is drawn, what is spoken and the voice it asks for.
///
/// A template is `//` + 32 hex characters + `//`. It is ignored inside a word
/// that holds `://`, so a URL never selects a voice. Every template is
/// removed; the first one names the voice. A `[` with no `]` is plain text.
pub fn parse_markup(text: &str) -> Markup {
    let mut out = Markup::default();
    let mut word_start = 0;
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
        let tail = &text[i..];
        if c.is_whitespace() {
            word_start = i + c.len_utf8();
        }
        if c == '/' && !text[..i].ends_with('/') {
            let word_end = tail.find(char::is_whitespace).map_or(text.len(), |n| i + n);
            let in_url = text[word_start..word_end].contains("://");
            if let Some(id) = voice_template(tail).filter(|_| !in_url) {
                out.voice.get_or_insert_with(|| id.to_ascii_lowercase());
                out.shown.push(' ');
                out.spoken.push(' ');
                i += VOICE_ID_LEN + 4;
                continue;
            }
        }
        if c == '[' {
            if let Some(end) = tail.find(']') {
                let span = &tail[..=end];
                out.spoken.push_str(span);
                if is_server_tag(&span[1..end]) {
                    out.shown.push_str(span);
                } else {
                    out.shown.push(' ');
                }
                i += end + 1;
                continue;
            }
        }
        out.shown.push(c);
        out.spoken.push(c);
        i += c.len_utf8();
    }
    out.shown = collapse(out.shown.chars()).trim_matches(' ').to_string();
    out.spoken = collapse(out.spoken.chars()).trim_matches(' ').to_string();
    out
}

/// Remove every `[...]` span: voices other than Fish Audio would read the cues aloud.
pub fn strip_brackets(text: &str) -> String {
    let mut kept = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let Some(len) = rest[start..].find(']') else { break };
        kept.push_str(&rest[..start]);
        kept.push(' ');
        rest = &rest[start + len + 1..];
    }
    kept.push_str(rest);
    collapse(kept.chars()).trim_matches(' ').to_string()
}

/// Only web links are opened or fetched.
pub fn is_web_url(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
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
    fn keeps_emoji_strips_control() {
        assert_eq!(display_safe("hola \u{1F602} mundo"), "hola \u{1F602} mundo");
        assert_eq!(display_safe("a\n\tb\0c"), "a b c");
        assert_eq!(display_safe("  espacios   raros  "), "espacios raros");
        // Family: man ZWJ woman ZWJ girl.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(display_safe(family), family);
        // Astral but not emoji (math letters): no glyphs.
        assert_eq!(display_safe("x\u{1D400}y"), "x y");
    }

    #[test]
    fn squash_keeps_ends() {
        assert_eq!(squash("  a  b "), " a b ");
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
    fn custom_emoji_urls() {
        assert_eq!(
            custom_emoji("<:pepe_hi:123>!"),
            Some((14, "https://cdn.discordapp.com/emojis/123.png".into()))
        );
        assert_eq!(
            custom_emoji("<a:dance:9>"),
            Some((11, "https://cdn.discordapp.com/emojis/9.gif".into()))
        );
        assert_eq!(custom_emoji("<b:x:1>"), None);
        assert_eq!(custom_emoji("<:x:12a>"), None);
        assert_eq!(custom_emoji("< hola >"), None);
        assert_eq!(custom_emoji("hola"), None);
    }

    #[test]
    fn tts_text() {
        assert_eq!(clean_tts_text("*Lauful*: mira esto"), "Lauful: mira esto");
        assert_eq!(clean_tts_text("jaja \u{1F602} <:pepe:1> ok"), "jaja ok");
    }

    const ID: &str = "ca3007f96ae7499ab87d27ea3599956a";
    const ID2: &str = "9a9cf47702da476aa4629e2506d4a857";

    #[test]
    fn voice_template_is_taken_out() {
        let m = parse_markup(&format!("//{ID}// hola mundo"));
        assert_eq!((m.shown.as_str(), m.spoken.as_str()), ("hola mundo", "hola mundo"));
        assert_eq!(m.voice.as_deref(), Some(ID));

        // Anywhere in the message, glued to words, upper case.
        let m = parse_markup(&format!("hola//{}//mundo", ID.to_uppercase()));
        assert_eq!(m.shown, "hola mundo");
        assert_eq!(m.voice.as_deref(), Some(ID));

        let m = parse_markup("sin plantilla");
        assert_eq!((m.shown.as_str(), m.voice), ("sin plantilla", None));
    }

    #[test]
    fn first_of_several_templates_wins_and_all_go() {
        let m = parse_markup(&format!("a //{ID}// b //{ID2}// c"));
        assert_eq!((m.shown.as_str(), m.spoken.as_str()), ("a b c", "a b c"));
        assert_eq!(m.voice.as_deref(), Some(ID));
    }

    #[test]
    fn only_real_voice_ids_match() {
        for text in [
            "https://example.com//x//".to_string(),
            format!("https://example.com//{ID}//"),
            format!("https://{ID}//"),
            format!("//{}//", &ID[..31]),      // too short
            format!("//{ID}0//"),              // too long
            format!("//{}g//", &ID[..31]),     // not hex
            format!("///{ID}//"),              // extra slash before
            format!("//{ID}///"),              // extra slash after
            format!("//{ID}/"),                // not closed
            "// comentario // normal".to_string(),
        ] {
            let m = parse_markup(&text);
            assert_eq!(m.voice, None, "{text}");
            assert_eq!(m.shown, text, "{text}");
        }
    }

    #[test]
    fn brackets_are_hidden_but_kept_for_fish() {
        let m = parse_markup(&format!("//{ID}// [happy] hola [laughing][whispering] adios"));
        assert_eq!(m.shown, "hola adios");
        assert_eq!(m.spoken, "[happy] hola [laughing][whispering] adios");
        assert_eq!(strip_brackets(&m.spoken), "hola adios");

        assert_eq!(parse_markup("[] vacio").shown, "vacio");
        assert_eq!(parse_markup("[solo]").shown, "");
    }

    #[test]
    fn unterminated_brackets_are_plain_text() {
        let m = parse_markup("hola [sin cerrar");
        assert_eq!((m.shown.as_str(), m.spoken.as_str()), ("hola [sin cerrar", "hola [sin cerrar"));
        assert_eq!(parse_markup("a ] b [c] d [e").shown, "a ] b d [e");
        assert_eq!(strip_brackets("a ] b [c] d [e"), "a ] b d [e");
    }

    #[test]
    fn server_tags_stay_on_the_overlay() {
        let m = parse_markup("mira [Link de youtube] [sad] [GIF]");
        assert_eq!(m.shown, "mira [Link de youtube] [GIF]");
        assert_eq!(parse_markup("[Imagen]").shown, "[Imagen]");
        assert_eq!(parse_markup("[Archivo]").shown, "[Archivo]");
    }

    #[test]
    fn web_urls() {
        assert!(is_web_url("https://youtu.be/x"));
        assert!(is_web_url("HTTP://a"));
        assert!(!is_web_url("file:///C:/x"));
        assert!(!is_web_url("javascript:alert(1)"));
    }

    #[test]
    fn token_is_redacted() {
        assert_eq!(redact_url("ws://h:1/?token=abc&x=1"), "ws://h:1/?token=***&x=1");
        assert_eq!(redact_url("ws://h:1"), "ws://h:1");
    }
}
