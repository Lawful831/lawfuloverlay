//! The overlay window: title bar, outlined text on an optional badge, and the
//! loops that feed it (messages, voice, colour detection).

use std::ops::Range;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, Galley,
    Image, OpenUrl, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
};
use eframe::egui::load::{SizedTexture, TexturePoll};
use eframe::egui::text::{CCursor, LayoutJob, TextFormat};

use crate::color::{self, hex_or};
use crate::config::Config;
use crate::net::{self, Event};
use crate::rich::Rich;
use crate::sample;
use crate::settings::{Outcome, Settings};
use crate::text::{is_web_url, parse_markup};
use crate::tts::Tts;
use crate::updater;

pub const WINDOW_TITLE: &str = "LawfulOverlay";
const FRAME_H: f32 = 28.0;
const CHROME_BG: Color32 = Color32::from_rgb(0x1E, 0x1F, 0x22);
const CHROME_FG: Color32 = Color32::from_rgb(0xBA, 0xBB, 0xBE);
const OFFSETS: [(f32, f32); 8] = [
    (-1.0, -1.0),
    (0.0, -1.0),
    (1.0, -1.0),
    (-1.0, 0.0),
    (1.0, 0.0),
    (-1.0, 1.0),
    (0.0, 1.0),
    (1.0, 1.0),
];
/// Tk sized fonts in points at 96 dpi; egui sizes are pixels.
const PT_TO_PX: f32 = 96.0 / 72.0;
/// Narrowest window: wide enough for the title-bar buttons.
const MIN_W: f32 = 230.0;
const MIN_CONTENT_H: f32 = 24.0;
const COLOR_FADE: Duration = Duration::from_millis(450);
/// Space between the text and the images, and between images.
const IMAGE_GAP: f32 = 6.0;
/// A custom emoji's size relative to the line height.
const EMOJI_SCALE: f32 = 0.85;

fn overlay_family() -> FontFamily {
    FontFamily::Name("overlay".into())
}

/// Font files for the families offered in the settings. `(regular, bold)`.
fn font_files(family: &str) -> Option<(&'static str, &'static str)> {
    Some(match family {
        "Arial" | "Helvetica" => ("arial.ttf", "arialbd.ttf"),
        "Calibri" => ("calibri.ttf", "calibrib.ttf"),
        "Consolas" => ("consola.ttf", "consolab.ttf"),
        "Georgia" => ("georgia.ttf", "georgiab.ttf"),
        "Segoe UI" => ("segoeui.ttf", "segoeuib.ttf"),
        "Tahoma" => ("tahoma.ttf", "tahomabd.ttf"),
        "Trebuchet MS" => ("trebuc.ttf", "trebucbd.ttf"),
        "Verdana" => ("verdana.ttf", "verdanab.ttf"),
        _ => return None,
    })
}

fn read_system_font(file: &str) -> Option<Vec<u8>> {
    let windir = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot"))?;
    std::fs::read(std::path::Path::new(&windir).join("Fonts").join(file)).ok()
}

fn load_system_font(family: &str, bold: bool) -> Option<Vec<u8>> {
    let (regular, heavy) = font_files(family)?;
    read_system_font(if bold { heavy } else { regular })
}

/// Register the "overlay" font family: the chosen system font first, egui's
/// built-ins behind it for symbols the font lacks.
fn install_fonts(ctx: &egui::Context, family: &str, bold: bool) {
    let mut fonts = FontDefinitions::default();
    // Segoe UI Symbol covers the title-bar icons and symbols like ⚙ ❤ that egui's built-in fonts lack.
    if let Some(bytes) = read_system_font("seguisym.ttf") {
        fonts.font_data.insert("symbols".into(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push("symbols".into());
        }
    }
    let mut stack = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    // Segoe UI Emoji draws emoji in colour; it goes right behind the chosen font.
    if let Some(bytes) = read_system_font("seguiemj.ttf") {
        fonts.font_data.insert("emoji".into(), Arc::new(FontData::from_owned(bytes)));
        stack.insert(0, "emoji".into());
    }
    if let Some(bytes) = load_system_font(family, bold) {
        fonts.font_data.insert("overlay".into(), Arc::new(FontData::from_owned(bytes)));
        stack.insert(0, "overlay".into());
    } else {
        eprintln!("[fuente] no se encontró {family}; usando la fuente por defecto");
    }
    fonts.families.insert(overlay_family(), stack);
    ctx.set_fonts(fonts);
}

/// egui spots animated images by the extension at the end of the URI;
/// Discord's media URLs carry a signed query after it.
fn loadable_uri(url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    let ext = [".gif", ".webp"].into_iter().find(|ext| path.ends_with(ext));
    match ext {
        Some(ext) if url.contains('?') => format!("{}&_=a{ext}", url.trim_end_matches('&')),
        _ => url.to_string(),
    }
}

/// The image at `url` once it has downloaded (the current frame, if animated).
fn texture(ctx: &egui::Context, url: &str) -> Option<SizedTexture> {
    match Image::new(loadable_uri(url)).load_for_size(ctx, Vec2::splat(1024.0)) {
        Ok(TexturePoll::Ready { texture }) if texture.size.min_elem() > 0.0 => Some(texture),
        _ => None,
    }
}

/// Where chars `range` of `galley` sit: one rect per row they span.
fn char_rects(galley: &Galley, range: Range<usize>) -> Vec<Rect> {
    let mut rects: Vec<Rect> = Vec::new();
    for i in range {
        let a = galley.pos_from_cursor(CCursor::new(i));
        let b = galley.pos_from_cursor(CCursor::new(i + 1));
        if (a.min.y - b.min.y).abs() > 0.5 {
            continue;
        }
        let r = Rect::from_min_max(a.min, Pos2::new(b.min.x, a.max.y));
        match rects.last_mut() {
            Some(last) if (last.min.y - r.min.y).abs() <= 0.5 => *last = last.union(r),
            _ => rects.push(r),
        }
    }
    rects
}

struct Fade {
    from: (Color32, Color32),
    to: (Color32, Color32),
    started: Instant,
}

pub struct OverlayApp {
    cfg: Config,
    message: Rich,
    text_color: Color32,
    outline_color: Color32,
    fade: Option<Fade>,
    last_bg: Option<[u8; 3]>,
    next_detect: Instant,
    capture_excluded: bool,
    transparent_set: bool,
    /// Whether the title bar is currently shown.
    hovered: bool,
    /// The pointer has been over the window at least once.
    pointer_seen: bool,
    /// The pointer has entered and left; from then on the bar shows on hover only.
    intro_done: bool,
    last_size: Vec2,
    events: Receiver<Event>,
    tts: Tts,
    settings: Option<Settings>,
    font_key: (String, bool),
}

impl OverlayApp {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config) -> Self {
        let ctx = cc.egui_ctx.clone();
        install_fonts(&ctx, &cfg.font_family, cfg.font_bold);
        egui_extras::install_image_loaders(&ctx);

        let (tx, events) = mpsc::channel();
        let tts_link = net::spawn(cfg.ws_url.clone(), tx.clone(), ctx.clone());
        if cfg.auto_update {
            updater::spawn(tx.clone(), ctx.clone());
        }
        let tts = Tts::new(&cfg.tts_voice, &cfg.fish_voice, tx, ctx, tts_link);

        Self {
            text_color: hex_or(&cfg.text_color, Color32::WHITE),
            outline_color: hex_or(&cfg.outline_color, Color32::BLACK),
            font_key: (cfg.font_family.clone(), cfg.font_bold),
            next_detect: Instant::now() + Duration::from_millis(u64::from(cfg.detect_interval)),
            message: Rich::plain("Esperando mensajes…"),
            fade: None,
            last_bg: None,
            capture_excluded: false,
            transparent_set: false,
            // Shown at start so it is discoverable; hover rules apply after the first visit.
            hovered: true,
            pointer_seen: false,
            intro_done: false,
            last_size: Vec2::ZERO,
            events,
            tts,
            settings: None,
            cfg,
        }
    }

    // ── Messages ─────────────────────────────────────────────────────────

    fn drain_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                // Status notices are shown but never spoken.
                Event::Status(text) => self.message = Rich::plain(&text),
                Event::Chat { user, content, speech, links, images } => {
                    // Voice markup (`//voice id//`, `[cues]`) is never drawn.
                    self.message = Rich::chat(&user, &parse_markup(&content).shown, &links, images);
                    if self.cfg.tts_enabled {
                        self.tts.speak_chat(&speech);
                    }
                }
                Event::TtsFailed => {
                    self.tts.mark_failed();
                    self.cfg.tts_enabled = false;
                    self.cfg.save();
                    self.message = Rich::plain("TTS no disponible: sin Edge TTS ni voz local de Windows.");
                }
            }
        }
    }

    fn toggle_tts(&mut self) {
        self.tts.clear_failure();
        self.tts.reset_queue();
        self.cfg.tts_enabled = !self.cfg.tts_enabled;
        self.cfg.save();
        if self.cfg.tts_enabled {
            self.tts.speak("Texto a voz activado");
        }
    }

    // ── Colours ──────────────────────────────────────────────────────────

    fn start_fade(&mut self, text: Color32, outline: Color32) {
        self.fade = Some(Fade {
            from: (self.text_color, self.outline_color),
            to: (text, outline),
            started: Instant::now(),
        });
    }

    fn step_fade(&mut self, ctx: &egui::Context) {
        let Some(fade) = &self.fade else { return };
        let t = (fade.started.elapsed().as_secs_f32() / COLOR_FADE.as_secs_f32()).min(1.0);
        let eased = 1.0 - (1.0 - t).powi(3); // cubic ease-out
        self.text_color = color::lerp(fade.from.0, fade.to.0, eased);
        self.outline_color = color::lerp(fade.from.1, fade.to.1, eased);
        if t < 1.0 {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            self.fade = None;
        }
    }

    /// Sample the pixels behind the overlay and pick readable colours.
    fn detect(&mut self, ctx: &egui::Context) {
        self.next_detect = Instant::now() + Duration::from_millis(u64::from(self.cfg.detect_interval));
        if !self.capture_excluded {
            self.capture_excluded = sample::exclude_from_capture(WINDOW_TITLE);
        }
        let Some(outer) = ctx.input(|i| i.viewport().outer_rect) else { return };
        let ppp = ctx.pixels_per_point();
        let side = self.cfg.detect_sample_px as f32;
        let cx = (outer.min.x + outer.width() / 2.0) * ppp;
        let bar_h = if self.hovered { FRAME_H } else { 0.0 };
        let cy = (outer.min.y + bar_h + (outer.height() - bar_h) / 2.0) * ppp;
        let Some(bg) = sample::sample_bg((cx - side / 2.0) as i32, (cy - side / 2.0) as i32, side as i32) else {
            return;
        };

        let moved = self.last_bg.is_none_or(|last| {
            bg.iter()
                .zip(last)
                .any(|(a, b)| u32::from(a.abs_diff(b)) > self.cfg.detect_threshold)
        });
        if !moved {
            return;
        }
        self.last_bg = Some(bg);
        eprintln!("[detección] fondo {bg:?}");
        let lum = color::relative_luminance(bg[0], bg[1], bg[2]);
        let (text, outline) = color::wcag_best_pair(lum);
        if self.cfg.bg_style == "auto_badge" {
            self.cfg.badge_color = color::auto_badge_color(lum).to_string();
        }
        self.start_fade(hex_or(text, Color32::WHITE), hex_or(outline, Color32::BLACK));
    }

    // ── Settings ─────────────────────────────────────────────────────────

    fn apply_settings(&mut self, ctx: &egui::Context, mut new: Config) {
        // The voice toggle lives on the title bar, not in the dialog.
        new.tts_enabled = self.cfg.tts_enabled;
        new.fish_voice = new.fish_voice.trim().to_string();
        if new.tts_voice != self.cfg.tts_voice || new.fish_voice != self.cfg.fish_voice {
            self.tts.reset_queue();
            self.tts.set_voices(&new.tts_voice, &new.fish_voice);
        }
        self.cfg = new;
        self.cfg.save();

        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if self.cfg.always_on_top {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        }));
        let key = (self.cfg.font_family.clone(), self.cfg.font_bold);
        if key != self.font_key {
            install_fonts(ctx, &key.0, key.1);
            self.font_key = key;
        }
        if !self.cfg.auto_detect {
            self.fade = None;
            self.text_color = hex_or(&self.cfg.text_color, Color32::WHITE);
            self.outline_color = hex_or(&self.cfg.outline_color, Color32::BLACK);
        }
        self.next_detect = Instant::now() + Duration::from_millis(u64::from(self.cfg.detect_interval));
    }

    // ── Painting ─────────────────────────────────────────────────────────

    /// The message laid out for painting. Links are underlined. For the
    /// outline pass emoji are left out: they paint in their own colours, so
    /// the offset copies would smear them.
    fn layout(&self, painter: &egui::Painter, wrap: f32, color: Color32, outline: bool) -> Arc<Galley> {
        let font = FontId::new(self.cfg.font_size as f32 * PT_TO_PX, overlay_family());
        let chars: Vec<char> = self.message.text.chars().collect();
        let mask = self.message.emoji_mask();
        let kind = |i: usize| (self.message.links.iter().any(|(r, _)| r.contains(&i)), mask[i]);
        let mut job = LayoutJob::default();
        let mut start = 0;
        while start < chars.len() {
            let (link, emoji) = kind(start);
            let mut end = start + 1;
            while end < chars.len() && kind(end) == (link, emoji) {
                end += 1;
            }
            let color = if outline && emoji { Color32::TRANSPARENT } else { color };
            let underline = if link { Stroke::new((font.size / 14.0).max(1.0), color) } else { Stroke::NONE };
            let run: String = chars[start..end].iter().collect();
            job.append(&run, 0.0, TextFormat { font_id: font.clone(), color, underline, ..Default::default() });
            start = end;
        }
        job.wrap.max_width = wrap;
        job.halign = Align::Center;
        painter.layout_job(job)
    }

    /// The downloaded images in one row, each at most `image_max_h` tall (never
    /// enlarged), shrunk together to fit `max_w`. Rects start at the origin.
    fn image_row(&self, ctx: &egui::Context, max_w: f32) -> Vec<(Rect, SizedTexture, &str)> {
        if !self.cfg.show_images {
            return Vec::new();
        }
        let max_h = self.cfg.image_max_h as f32;
        let loaded: Vec<(SizedTexture, &str)> = self
            .message
            .images
            .iter()
            .filter_map(|url| texture(ctx, url).map(|t| (t, url.as_str())))
            .collect();
        let natural = |t: &SizedTexture| {
            let h = t.size.y.min(max_h);
            Vec2::new(h * t.size.x / t.size.y, h)
        };
        let gaps = IMAGE_GAP * loaded.len().saturating_sub(1) as f32;
        let total_w: f32 = loaded.iter().map(|(t, _)| natural(t).x).sum();
        let scale = ((max_w - gaps) / total_w).min(1.0);
        let mut x = 0.0;
        loaded
            .into_iter()
            .map(|(t, url)| {
                let size = natural(&t) * scale;
                let rect = Rect::from_min_size(Pos2::new(x, 0.0), size);
                x += size.x + IMAGE_GAP;
                (rect, t, url)
            })
            .collect()
    }

    /// Size the window to the text and images: wrapped at the configured
    /// width, nothing more. The bar shows at start until the pointer has
    /// visited and left once; afterwards hovering adds it above the text, so
    /// the text never moves.
    fn fit_window(&mut self, ui: &egui::Ui, ctx: &egui::Context, hovered: bool) {
        let cfg = &self.cfg;
        let pad = cfg.badge_padding as f32;
        let ow = cfg.outline_width as f32;
        let max_w = cfg.overlay_w as f32;
        let wrap = (max_w - pad * 2.0 - ow * 2.0).max(10.0);
        let text = self.layout(ui.painter(), wrap, Color32::WHITE, false).size();
        let row = row_size(&self.image_row(ctx, wrap));
        let body = if row.y > 0.0 { Vec2::new(text.x.max(row.x), text.y + IMAGE_GAP + row.y) } else { text };

        let content_w = (body.x + (pad + ow) * 2.0).clamp(MIN_W, max_w.max(MIN_W));
        let content_h = (body.y + (pad + ow) * 2.0).max(MIN_CONTENT_H);
        let bar_h = if hovered { FRAME_H } else { 0.0 };
        let want = Vec2::new(content_w, content_h + bar_h);

        let size_changed = (want - self.last_size).abs().max_elem() > 0.5;
        if size_changed || hovered != self.hovered {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(want));
            if hovered != self.hovered {
                // Grow/shrink upward so the text stays put.
                if let Some(outer) = ctx.input(|i| i.viewport().outer_rect) {
                    let dy = if hovered { -FRAME_H } else { FRAME_H };
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Pos2::new(outer.min.x, outer.min.y + dy)));
                }
            }
            self.last_size = want;
        }
        self.hovered = hovered;
    }

    fn paint_title_bar(&mut self, ui: &mut egui::Ui, bar: Rect) {
        let ctx = ui.ctx().clone();
        ui.painter().rect_filled(bar, 0, CHROME_BG);
        ui.painter().text(
            Pos2::new(bar.min.x + 10.0, bar.center().y),
            Align2::LEFT_CENTER,
            "Discord Overlay",
            FontId::proportional(12.0),
            CHROME_FG,
        );

        // Dragging comes first so the buttons, added after, sit on top of it.
        let drag = ui.interact(bar, egui::Id::unique("title_drag"), Sense::click_and_drag());
        if drag.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        let button = |ui: &mut egui::Ui, slot: usize, label: &str, fill: Color32| {
            let size = Vec2::new(30.0, 22.0);
            let min = Pos2::new(bar.max.x - 4.0 - (slot as f32 + 1.0) * size.x - slot as f32 * 2.0, bar.center().y - size.y / 2.0);
            ui.put(
                Rect::from_min_size(min, size),
                egui::Button::new(RichText::new(label).strong().color(Color32::WHITE)).fill(fill).corner_radius(0),
            )
            .clicked()
        };

        if button(ui, 0, "×", Color32::from_rgb(0xED, 0x42, 0x45)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if button(ui, 1, "⚙", Color32::from_rgb(0x58, 0x65, 0xF2)) && self.settings.is_none() {
            self.settings = Some(Settings::new(&self.cfg));
        }
        let (label, fill) = if self.cfg.tts_enabled {
            ("🔊", Color32::from_rgb(0x57, 0xF2, 0x87))
        } else {
            ("🔇", Color32::from_rgb(0x4E, 0x50, 0x58))
        };
        if button(ui, 2, label, fill) {
            self.toggle_tts();
        }
    }

    /// Paints the message; returns the clickable rects (links, images) and
    /// where each leads.
    fn paint_message(&self, ui: &egui::Ui, area: Rect) -> Vec<(Rect, String)> {
        let painter = ui.painter_at(area);
        let cfg = &self.cfg;
        let pad = cfg.badge_padding as f32;

        if matches!(cfg.bg_style.as_str(), "badge" | "auto_badge") {
            let mut fill = hex_or(&cfg.badge_color, Color32::from_rgb(0x11, 0x18, 0x27));
            if cfg.bg_style == "auto_badge" {
                fill = Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), 128); // semi-transparent
            }
            painter.rect(
                area.shrink(pad / 2.0),
                CornerRadius::same(cfg.badge_radius.min(255) as u8),
                fill,
                Stroke::NONE,
                StrokeKind::Inside,
            );
        }

        let wrap = (area.width() - pad * 2.0).max(10.0);
        let outline = self.layout(&painter, wrap, self.outline_color, true);
        let fill = self.layout(&painter, wrap, self.text_color, false);
        let images = self.image_row(ui.ctx(), wrap);
        let row = row_size(&images);

        // Text on top, images under it; the whole block centred.
        let body_h = fill.rect.height() + if row.y > 0.0 { IMAGE_GAP + row.y } else { 0.0 };
        let top = area.center().y - body_h / 2.0;
        let origin = Pos2::new(area.center().x - fill.rect.center().x, top - fill.rect.min.y);

        let ow = cfg.outline_width as f32;
        if ow > 0.0 {
            for (dx, dy) in OFFSETS {
                painter.galley(origin + Vec2::new(dx * ow, dy * ow), outline.clone(), self.outline_color);
            }
        }
        painter.galley(origin, fill.clone(), self.text_color);

        for (index, url) in &self.message.emoji {
            let Some(slot) = char_rects(&fill, *index..*index + 1).first().map(|r| r.translate(origin.to_vec2())) else {
                continue;
            };
            if let Some(texture) = texture(ui.ctx(), url) {
                let side = slot.height() * EMOJI_SCALE;
                let fit = texture.size * (side / texture.size.max_elem());
                Image::from_texture(texture).paint_at(ui, Rect::from_center_size(slot.center(), fit));
            }
        }

        let mut targets = Vec::new();
        for (range, url) in &self.message.links {
            if is_web_url(url) {
                for rect in char_rects(&fill, range.clone()) {
                    targets.push((rect.translate(origin.to_vec2()), url.clone()));
                }
            }
        }
        let row_origin = Vec2::new(area.center().x - row.x / 2.0, top + fill.rect.height() + IMAGE_GAP);
        for (rect, texture, url) in images {
            let rect = rect.translate(row_origin);
            Image::from_texture(texture).corner_radius(4).paint_at(ui, rect);
            targets.push((rect, url.to_string()));
        }
        targets
    }
}

impl eframe::App for OverlayApp {
    /// Fully transparent: only the bar, badge and text are visible.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.transparent_set {
            self.transparent_set = sample::make_transparent(WINDOW_TITLE);
            ctx.request_repaint();
        }
        self.drain_events();
        self.step_fade(&ctx);

        if self.cfg.auto_detect {
            let now = Instant::now();
            if now >= self.next_detect {
                self.detect(&ctx);
            }
            ctx.request_repaint_after(self.next_detect.saturating_duration_since(Instant::now()));
        }

        let full = ui.max_rect();
        // The bar only appears while the pointer is over the window, so at rest
        // nothing but the text is visible.
        let over = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| full.contains(p));
        if over {
            self.pointer_seen = true;
        } else if self.pointer_seen {
            self.intro_done = true;
        }
        let hovered = over || !self.intro_done;
        self.fit_window(ui, &ctx, hovered);

        let bar_h = if hovered { FRAME_H } else { 0.0 };
        let bar = Rect::from_min_size(full.min, Vec2::new(full.width(), bar_h));
        let content = Rect::from_min_max(Pos2::new(full.min.x, bar.max.y), full.max);
        if hovered {
            self.paint_title_bar(ui, bar);
        }
        let targets = self.paint_message(ui, content);
        let pointer = ctx.input(|i| i.pointer.hover_pos());
        if let Some((_, url)) = pointer.and_then(|p| targets.iter().find(|(rect, _)| rect.contains(p))) {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
            if ctx.input(|i| i.pointer.primary_clicked()) {
                ctx.open_url(OpenUrl::new_tab(url));
            }
        }

        if let Some(mut dialog) = self.settings.take() {
            match dialog.show(&ctx) {
                Outcome::Open => self.settings = Some(dialog),
                Outcome::Apply(new) => self.apply_settings(&ctx, *new),
                Outcome::Cancel => {}
            }
        }
    }
}

/// Width and height of an image row.
fn row_size<T>(row: &[(Rect, SizedTexture, T)]) -> Vec2 {
    row.iter().fold(Vec2::ZERO, |size, (rect, _, _)| size.max(rect.max.to_vec2()))
}

/// Initial window options for the saved settings.
pub fn native_options(cfg: &Config) -> eframe::NativeOptions {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(WINDOW_TITLE)
        .with_inner_size([cfg.overlay_w as f32, MIN_CONTENT_H + FRAME_H])
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_taskbar(false);
    if cfg.always_on_top {
        viewport = viewport.with_always_on_top();
    }
    eframe::NativeOptions { viewport, wgpu_options: wgpu_options(), ..Default::default() }
}

/// Choose the GPU backend so the window reliably shows through. Left to itself
/// wgpu takes whichever adapter it finds first: on Vulkan the window is
/// transparent, on DX12's default HWND swapchain it is opaque, so it changed
/// from launch to launch.
///
/// Vulkan comes first. It presents into the window's own redirection surface,
/// which DWM blends with per-pixel alpha, even though drivers report only
/// `Opaque` for it (so its surface capabilities can't be used to choose).
/// DX12 over a DirectComposition visual is the fallback: it supports
/// transparency, but its visual sits on top of the redirection surface, and
/// whatever Windows leaves in there (a white fill after a resize) shows through.
#[cfg(windows)]
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{wgpu, WgpuConfiguration, WgpuSetup};
    use std::sync::Arc;

    let mut config = WgpuConfiguration::default();
    if let WgpuSetup::CreateNew(setup) = &mut config.wgpu_setup {
        setup.instance_descriptor.backends = wgpu::Backends::VULKAN | wgpu::Backends::DX12;
        setup.instance_descriptor.backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
        setup.native_adapter_selector = Some(Arc::new(|adapters, _surface| {
            let rank = |adapter: &wgpu::Adapter| {
                let info = adapter.get_info();
                let real_gpu = matches!(info.device_type, wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu);
                (real_gpu, info.backend == wgpu::Backend::Vulkan, info.device_type == wgpu::DeviceType::DiscreteGpu)
            };
            let best = adapters.iter().max_by_key(|a| rank(a)).ok_or("no graphics adapter")?;
            eprintln!("[gpu] {:?} {}", best.get_info().backend, best.get_info().name);
            Ok(best.clone())
        }));
    }
    config
}

#[cfg(not(windows))]
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    eframe::egui_wgpu::WgpuConfiguration::default()
}

