//! Links and attachments → readable text.
//!
//! A Discord message is turned into two strings: the one shown on the overlay
//! (links and media tagged in brackets) and the one the voice reads out.

use std::sync::LazyLock;

use regex::{Captures, Regex};

static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://[^\s<>]*[^\s<>.,;:!?)\]]|www\.[^\s<>]*[^\s<>.,;:!?)\]]")
        .expect("URL regex is valid")
});

/// Recognisable site names; anything else falls back to the domain label.
const SITE_NAMES: &[(&str, &str)] = &[
    ("youtu", "youtube"),
    ("youtube", "youtube"),
    ("youtube-nocookie", "youtube"),
    ("x", "twitter"),
    ("twitter", "twitter"),
    ("t", "twitter"),
    ("twitch", "twitch"),
    ("tiktok", "tiktok"),
    ("instagram", "instagram"),
    ("discord", "discord"),
    ("reddit", "reddit"),
    ("spotify", "spotify"),
    ("github", "github"),
    ("imgur", "imgur"),
    ("tenor", "tenor"),
    ("facebook", "facebook"),
    ("fb", "facebook"),
    ("soundcloud", "soundcloud"),
    ("steamcommunity", "steam"),
    ("steampowered", "steam"),
    ("pixiv", "pixiv"),
];

/// Second-level labels that are not the site name (bbc.co.uk → bbc).
const SECOND_LEVEL: &[&str] = &["co", "com", "org", "net", "gov", "ac", "edu"];

/// Host part of a URL or scheme-less `www.` link, lowercased, without
/// credentials or port.
fn hostname(url: &str) -> Option<String> {
    let rest = url.split_once("//").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(inner) = host_port.strip_prefix('[') {
        inner.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    if host.is_empty() {
        None
    } else {
        Some(host.to_lowercase())
    }
}

pub fn site_name(url: &str) -> String {
    let host = hostname(url).unwrap_or_else(|| url.to_string());
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let mut labels: Vec<&str> = host.split('.').collect();
    if labels.len() > 1 && SECOND_LEVEL.contains(&labels[labels.len() - 2]) {
        labels.pop();
    }
    let name = if labels.len() > 1 {
        labels[labels.len() - 2]
    } else {
        labels[0]
    };
    SITE_NAMES
        .iter()
        .find(|(k, _)| *k == name)
        .map_or(name, |(_, v)| v)
        .to_string()
}

/// `"https://youtube.com/watch?v=x"` → `"Link de youtube"` (or `"[Link de youtube]"`).
pub fn humanize_links(text: &str, brackets: bool) -> String {
    URL_RE
        .replace_all(text, |m: &Captures| {
            let name = format!("Link de {}", site_name(&m[0]));
            if brackets {
                format!("[{name}]")
            } else {
                name
            }
        })
        .into_owned()
}

#[derive(Debug, Clone, Default)]
pub struct Attachment {
    pub content_type: Option<String>,
    pub filename: String,
}

#[derive(Debug, Clone, Default)]
pub struct Embed {
    /// Discord embed type ("rich", "gifv", "image", …).
    pub kind: Option<String>,
    /// The embed carries an image or a thumbnail.
    pub has_image: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Gif,
    Image,
    File,
}

impl MediaKind {
    /// What the voice announces.
    fn spoken(self) -> &'static str {
        match self {
            MediaKind::Gif => "Compartido un gif",
            MediaKind::Image => "Compartido una imagen",
            MediaKind::File => "Compartido un archivo",
        }
    }

    /// Tag shown on the overlay.
    fn tag(self) -> &'static str {
        match self {
            MediaKind::Gif => "[GIF]",
            MediaKind::Image => "[Imagen]",
            MediaKind::File => "[Archivo]",
        }
    }
}

fn is_gif(att: &Attachment) -> bool {
    att.content_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("image/gif"))
        || att.filename.to_lowercase().ends_with(".gif")
}

/// What the message carries. Link previews (embeds) are ignored when the
/// message has text of its own: the text already names them.
pub fn media_kind(attachments: &[Attachment], embeds: &[Embed], content: &str) -> Option<MediaKind> {
    let embeds: &[Embed] = if content.is_empty() { embeds } else { &[] };
    if attachments.iter().any(is_gif)
        || embeds
            .iter()
            .any(|e| e.kind.as_deref().is_some_and(|k| k.eq_ignore_ascii_case("gifv")))
    {
        return Some(MediaKind::Gif);
    }
    if attachments
        .iter()
        .any(|a| a.content_type.as_deref().is_some_and(|t| t.starts_with("image/")))
        || embeds.iter().any(|e| e.has_image)
    {
        return Some(MediaKind::Image);
    }
    (!attachments.is_empty()).then_some(MediaKind::File)
}

fn join_parts(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Overlay text: links and attachments tagged, original text kept.
pub fn display_text(attachments: &[Attachment], embeds: &[Embed], content: &str) -> String {
    let mut parts = Vec::new();
    if !content.is_empty() {
        parts.push(humanize_links(content, true));
    }
    if let Some(kind) = media_kind(attachments, embeds, content) {
        parts.push(kind.tag().to_string());
    }
    join_parts(&parts)
}

/// Text to be spoken: readable links plus an announcement for media.
pub fn speech_text(attachments: &[Attachment], embeds: &[Embed], content: &str) -> String {
    let mut parts = Vec::new();
    if !content.is_empty() {
        parts.push(humanize_links(content, false));
    }
    if let Some(kind) = media_kind(attachments, embeds, content) {
        parts.push(kind.spoken().to_string());
    }
    join_parts(&parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn att(content_type: Option<&str>, filename: &str) -> Attachment {
        Attachment {
            content_type: content_type.map(str::to_string),
            filename: filename.to_string(),
        }
    }

    fn embed(kind: &str) -> Embed {
        Embed {
            kind: Some(kind.to_string()),
            has_image: true,
        }
    }

    #[test]
    fn site_names() {
        assert_eq!(site_name("https://www.youtube.com/watch?v=x"), "youtube");
        assert_eq!(site_name("https://youtu.be/x"), "youtube");
        assert_eq!(site_name("https://music.youtube.com/x"), "youtube");
        assert_eq!(site_name("https://x.com/a"), "twitter");
        assert_eq!(site_name("https://t.co/z"), "twitter");
        assert_eq!(site_name("https://open.spotify.com/track/1"), "spotify");
        assert_eq!(site_name("https://www.bbc.co.uk/news"), "bbc");
        assert_eq!(site_name("https://ejemplo.ar/x"), "ejemplo");
        assert_eq!(site_name("https://user:pw@Example.com:8080/x"), "example");
    }

    #[test]
    fn links() {
        assert_eq!(humanize_links("mira https://youtube.com/watch?v=x", false), "mira Link de youtube");
        assert_eq!(humanize_links("mira https://youtube.com/x.", false), "mira Link de youtube.");
        assert_eq!(humanize_links("Link (https://youtu.be/x), ok", false), "Link (Link de youtube), ok");
        assert_eq!(humanize_links("https://.", false), "https://.");
        assert_eq!(
            humanize_links("a www.twitch.tv/b y https://t.co/z", false),
            "a Link de twitch y Link de twitter"
        );
        assert_eq!(humanize_links("sin enlaces", false), "sin enlaces");
        assert_eq!(
            humanize_links("mira https://youtube.com/x", true),
            "mira [Link de youtube]"
        );
    }

    #[test]
    fn speech() {
        assert_eq!(speech_text(&[att(Some("image/png"), "")], &[], ""), "Compartido una imagen");
        assert_eq!(speech_text(&[att(Some("image/gif"), "a.gif")], &[], ""), "Compartido un gif");
        assert_eq!(speech_text(&[att(None, "a.GIF")], &[], ""), "Compartido un gif");
        assert_eq!(speech_text(&[att(None, "a.pdf")], &[], ""), "Compartido un archivo");
        assert_eq!(
            speech_text(&[att(Some("image/png"), "")], &[], "https://youtu.be/x"),
            "Link de youtube Compartido una imagen"
        );
        assert_eq!(speech_text(&[], &[embed("gifv")], ""), "Compartido un gif");
        assert_eq!(speech_text(&[], &[embed("rich")], ""), "Compartido una imagen");
        // A link preview isn't announced when the message has text of its own.
        assert_eq!(speech_text(&[], &[embed("rich")], "https://youtu.be/x"), "Link de youtube");
        assert_eq!(speech_text(&[], &[], ""), "");
    }

    #[test]
    fn display() {
        assert_eq!(display_text(&[], &[], "mira https://youtube.com/x"), "mira [Link de youtube]");
        assert_eq!(display_text(&[att(Some("image/png"), "")], &[], "mira esto"), "mira esto [Imagen]");
        assert_eq!(display_text(&[att(Some("image/gif"), "a.gif")], &[], ""), "[GIF]");
        assert_eq!(display_text(&[att(None, "a.pdf")], &[], ""), "[Archivo]");
        assert_eq!(display_text(&[], &[embed("gifv")], ""), "[GIF]");
        assert_eq!(display_text(&[], &[], ""), "");
    }
}
