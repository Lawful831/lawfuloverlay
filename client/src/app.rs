//! The overlay window: title bar, outlined text on an optional badge, and the
//! loops that feed it (messages, voice, colour detection).

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect, RichText,
    Sense, Stroke, StrokeKind, Vec2,
};
use eframe::egui::text::{LayoutJob, TextFormat};

use crate::color::{self, hex_or};
use crate::config::Config;
use crate::net::{self, Event};
use crate::sample;
use crate::settings::{Outcome, Settings};
use crate::text::display_safe;
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
    if let Some(bytes) = load_system_font(family, bold) {
        fonts.font_data.insert("overlay".into(), Arc::new(FontData::from_owned(bytes)));
        stack.insert(0, "overlay".into());
    } else {
        eprintln!("[fuente] no se encontró {family}; usando la fuente por defecto");
    }
    fonts.families.insert(overlay_family(), stack);
    ctx.set_fonts(fonts);
}

struct Fade {
    from: (Color32, Color32),
    to: (Color32, Color32),
    started: Instant,
}

pub struct OverlayApp {
    cfg: Config,
    message: String,
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

        let (tx, events) = mpsc::channel();
        net::spawn(cfg.ws_url.clone(), tx.clone(), ctx.clone());
        if cfg.auto_update {
            updater::spawn(tx.clone(), ctx.clone());
        }
        let tts = Tts::new(&cfg.tts_voice, tx, ctx);

        Self {
            text_color: hex_or(&cfg.text_color, Color32::WHITE),
            outline_color: hex_or(&cfg.outline_color, Color32::BLACK),
            font_key: (cfg.font_family.clone(), cfg.font_bold),
            next_detect: Instant::now() + Duration::from_millis(u64::from(cfg.detect_interval)),
            message: display_safe("Esperando mensajes…"),
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
                Event::Status(text) => self.message = display_safe(&text),
                Event::Chat { user, content, speech } => {
                    self.message = display_safe(&format!("*{user}*: {content}"));
                    if self.cfg.tts_enabled {
                        self.tts.speak(&format!("*{user}*: {speech}"));
                    }
                }
                Event::TtsFailed => {
                    self.tts.mark_failed();
                    self.cfg.tts_enabled = false;
                    self.cfg.save();
                    self.message = display_safe("TTS no disponible: sin Edge TTS ni voz local de Windows.");
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
        if new.tts_voice != self.cfg.tts_voice {
            self.tts.reset_queue();
            self.tts.set_voice(&new.tts_voice);
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

    /// Size the window to the text: wrapped at the configured width, nothing
    /// more. The bar shows at start until the pointer has visited and left once;
    /// afterwards hovering adds it above the text, so the text never moves.
    fn fit_window(&mut self, ui: &egui::Ui, ctx: &egui::Context, hovered: bool) {
        let cfg = &self.cfg;
        let pad = cfg.badge_padding as f32;
        let ow = cfg.outline_width as f32;
        let max_w = cfg.overlay_w as f32;
        let mut job = LayoutJob::single_section(
            self.message.clone(),
            TextFormat {
                font_id: FontId::new(cfg.font_size as f32 * PT_TO_PX, overlay_family()),
                ..Default::default()
            },
        );
        job.wrap.max_width = (max_w - pad * 2.0 - ow * 2.0).max(10.0);
        let text = ui.painter().layout_job(job).size();

        let content_w = (text.x + (pad + ow) * 2.0).clamp(MIN_W, max_w.max(MIN_W));
        let content_h = (text.y + (pad + ow) * 2.0).max(MIN_CONTENT_H);
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

    fn paint_message(&self, ui: &egui::Ui, area: Rect) {
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

        let font = FontId::new(cfg.font_size as f32 * PT_TO_PX, overlay_family());
        let wrap = (area.width() - pad * 2.0).max(10.0);
        let layout = |color: Color32| {
            let mut job = LayoutJob::single_section(
                self.message.clone(),
                TextFormat { font_id: font.clone(), color, ..Default::default() },
            );
            job.wrap.max_width = wrap;
            job.halign = Align::Center;
            painter.layout_job(job)
        };
        let outline = layout(self.outline_color);
        let fill = layout(self.text_color);
        let center = area.center();
        let origin = center - fill.rect.center().to_vec2();

        let ow = cfg.outline_width as f32;
        if ow > 0.0 {
            for (dx, dy) in OFFSETS {
                painter.galley(origin + Vec2::new(dx * ow, dy * ow), outline.clone(), self.outline_color);
            }
        }
        painter.galley(origin, fill, self.text_color);
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
        self.paint_message(ui, content);

        if let Some(mut dialog) = self.settings.take() {
            match dialog.show(&ctx) {
                Outcome::Open => self.settings = Some(dialog),
                Outcome::Apply(new) => self.apply_settings(&ctx, *new),
                Outcome::Cancel => {}
            }
        }
    }
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
    eframe::NativeOptions { viewport, ..Default::default() }
}

