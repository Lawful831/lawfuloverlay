//! A line as the overlay draws it: the text, which parts of it are links,
//! where Discord's custom emoji go, and the images under it.

use std::ops::Range;

use crate::text::{custom_emoji, display_safe, is_astral_emoji, squash};

/// Holds a custom emoji's place in the text: about one em wide, draws nothing.
pub const EMOJI_SLOT: char = '\u{2003}';

/// How the server tags a link in the text (`[Link de youtube]`).
const LINK_TAG: &str = "[Link de ";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rich {
    pub text: String,
    /// Char ranges of the link tags, and where each leads.
    pub links: Vec<(Range<usize>, String)>,
    /// Char index of each custom emoji's slot, and its image.
    pub emoji: Vec<(usize, String)>,
    pub images: Vec<String>,
}

impl Rich {
    pub fn plain(text: &str) -> Self {
        Self { text: display_safe(text), ..Default::default() }
    }

    /// `*user*: content`. The n-th link tag in `content` opens `links[n]`.
    pub fn chat(user: &str, content: &str, links: &[String], images: Vec<String>) -> Self {
        let mut line = Line::default();
        line.push_text(&format!("*{user}*: "));
        let mut links = links.iter();
        let mut rest = content;
        let mut plain_start = 0;
        let mut i = 0;
        while i < rest.len() {
            let tail = &rest[i..];
            let tag = tail.starts_with(LINK_TAG).then(|| tail.find(']')).flatten();
            if let Some((len, url)) = custom_emoji(tail) {
                line.push_text(&rest[plain_start..i]);
                line.push_emoji(url);
                rest = &rest[i + len..];
                (plain_start, i) = (0, 0);
            } else if let Some((end, url)) = tag.and_then(|end| links.next().map(|url| (end, url))) {
                line.push_text(&rest[plain_start..i]);
                line.push_link(&rest[i..=i + end], url);
                rest = &rest[i + end + 1..];
                (plain_start, i) = (0, 0);
            } else {
                i += tail.chars().next().map_or(1, char::len_utf8);
            }
        }
        line.push_text(&rest[plain_start..]);
        line.finish(images)
    }

    /// Per char: is it part of an emoji, which the emoji font paints in its own colours?
    pub fn emoji_mask(&self) -> Vec<bool> {
        let chars: Vec<char> = self.text.chars().collect();
        let joins = |c: char| matches!(c, '\u{FE0F}' | '\u{20E3}' | '\u{200D}');
        chars
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let next = chars.get(i + 1).copied();
                is_astral_emoji(c)
                    || joins(c)
                    || matches!(c as u32, 0x2300..=0x23FF | 0x2600..=0x27BF | 0x2B00..=0x2BFF)
                    // Keycaps (1️⃣) and text symbols asked to show as emoji (❤️).
                    || next.is_some_and(|n| n == '\u{FE0F}' || n == '\u{20E3}')
            })
            .collect()
    }
}

#[derive(Default)]
struct Line {
    rich: Rich,
    chars: usize,
}

impl Line {
    fn push_str(&mut self, s: &str) {
        self.rich.text.push_str(s);
        self.chars += s.chars().count();
    }

    fn push_text(&mut self, text: &str) {
        let text = squash(text);
        let text = if self.rich.text.is_empty() || self.rich.text.ends_with(' ') {
            text.trim_start_matches(' ')
        } else {
            &text
        };
        self.push_str(text);
    }

    fn push_emoji(&mut self, url: String) {
        self.rich.emoji.push((self.chars, url));
        self.rich.text.push(EMOJI_SLOT);
        self.chars += 1;
    }

    fn push_link(&mut self, tag: &str, url: &str) {
        let start = self.chars;
        self.push_str(&display_safe(tag));
        self.rich.links.push((start..self.chars, url.to_string()));
    }

    fn finish(mut self, images: Vec<String>) -> Rich {
        let kept = self.rich.text.trim_end_matches(' ').len();
        self.rich.text.truncate(kept);
        self.rich.images = images;
        self.rich
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(text: &str, range: Range<usize>) -> String {
        text.chars().skip(range.start).take(range.len()).collect()
    }

    #[test]
    fn links_pair_with_tags_in_order() {
        let urls = ["https://youtu.be/x".to_string(), "https://t.co/z".to_string()];
        let rich = Rich::chat("ana", "mira [Link de youtube] y [Link de twitter]  ya", &urls, vec![]);
        assert_eq!(rich.text, "*ana*: mira [Link de youtube] y [Link de twitter] ya");
        assert_eq!(rich.links.len(), 2);
        assert_eq!(slice(&rich.text, rich.links[0].0.clone()), "[Link de youtube]");
        assert_eq!(rich.links[0].1, urls[0]);
        assert_eq!(slice(&rich.text, rich.links[1].0.clone()), "[Link de twitter]");
    }

    #[test]
    fn tags_without_urls_stay_text() {
        let rich = Rich::chat("ana", "[Link de x] [Link de", &[], vec![]);
        assert_eq!(rich.text, "*ana*: [Link de x] [Link de");
        assert!(rich.links.is_empty());
    }

    #[test]
    fn custom_emoji_get_slots() {
        let rich = Rich::chat("ana", "hola <:pepe:12> \u{f1}<a:b:3>", &[], vec![]);
        assert_eq!(rich.text, format!("*ana*: hola {EMOJI_SLOT} \u{f1}{EMOJI_SLOT}"));
        let at = |i: usize| rich.text.chars().nth(i);
        assert_eq!(rich.emoji.len(), 2);
        assert_eq!(at(rich.emoji[0].0), Some(EMOJI_SLOT));
        assert_eq!(at(rich.emoji[1].0), Some(EMOJI_SLOT));
        assert!(rich.emoji[1].1.ends_with("/3.gif"));
    }

    #[test]
    fn emoji_mask_covers_sequences() {
        let rich = Rich::plain("a \u{1F468}\u{200D}\u{1F469} 1\u{FE0F}\u{20E3} \u{2764}\u{FE0F}");
        let mask: String = rich.emoji_mask().iter().map(|&e| if e { 'e' } else { '.' }).collect();
        assert_eq!(mask, "..eee.eee.ee");
    }
}
