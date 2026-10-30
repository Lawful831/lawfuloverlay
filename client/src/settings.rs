//! The settings window: a second native window with tabs.
//!
//! Edits go to a draft; "Aplicar y cerrar" hands the draft back to the app and
//! "Cancelar" (or closing the window) throws it away.

use eframe::egui::{self, Color32, ComboBox, DragValue, Frame, RichText};

use crate::color::{hex_or, parse_hex, to_hex};
use crate::config::{config_path, Config, BG_STYLES};

const DARK: Color32 = Color32::from_rgb(0x1E, 0x1F, 0x22);
const MID: Color32 = Color32::from_rgb(0x2B, 0x2D, 0x31);
const ENTRY: Color32 = Color32::from_rgb(0x38, 0x3A, 0x40);
const FG: Color32 = Color32::from_rgb(0xDC, 0xDD, 0xDE);
const ACC: Color32 = Color32::from_rgb(0x58, 0x65, 0xF2);
const HINT: Color32 = Color32::from_rgb(0x88, 0x88, 0x88);

const FONTS: [&str; 9] = [
    "Arial", "Calibri", "Consolas", "Georgia", "Helvetica", "Segoe UI", "Tahoma", "Trebuchet MS", "Verdana",
];
const TTS_VOICES: [&str; 8] = [
    "en-US-AriaNeural",
    "en-US-GuyNeural",
    "en-GB-SoniaNeural",
    "en-GB-RyanNeural",
    "es-ES-ElviraNeural",
    "es-ES-AlvaroNeural",
    "es-MX-DaliaNeural",
    "es-MX-JorgeNeural",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Conexion,
    Pantalla,
    Estilo,
    Deteccion,
    Voz,
}

const TABS: [(Tab, &str); 5] = [
    (Tab::Conexion, "Conexión"),
    (Tab::Pantalla, "Pantalla"),
    (Tab::Estilo, "Estilo"),
    (Tab::Deteccion, "Detección"),
    (Tab::Voz, "Voz"),
];

pub enum Outcome {
    Open,
    Apply(Box<Config>),
    Cancel,
}

pub struct Settings {
    draft: Config,
    tab: Tab,
}

impl Settings {
    pub fn new(current: &Config) -> Self {
        Self { draft: current.clone(), tab: Tab::Conexion }
    }

    /// Show the window for one frame.
    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let builder = egui::ViewportBuilder::default()
            .with_title("LawfulOverlay — Configuración")
            .with_inner_size([520.0, 480.0])
            .with_resizable(false)
            .with_always_on_top();

        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("settings"), builder, |ui, _class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                return Outcome::Cancel;
            }
            let mut outcome = Outcome::Open;
            style(ui);
            egui::CentralPanel::default()
                .frame(Frame::new().fill(DARK).inner_margin(10.0))
                .show(ui, |ui| {
                    egui::Panel::bottom("buttons").frame(Frame::new().fill(DARK)).show(ui, |ui| {
                        ui.add_space(6.0);
                        // Buttons first, so a long file path can't push them out of view.
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let apply = egui::Button::new(RichText::new("Aplicar y cerrar").strong().color(Color32::WHITE))
                                .fill(ACC);
                            if ui.add(apply).clicked() {
                                outcome = Outcome::Apply(Box::new(self.draft.clone()));
                            }
                            if ui.add(egui::Button::new("Cancelar").fill(MID)).clicked() {
                                outcome = Outcome::Cancel;
                            }
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("Archivo: {}", config_path().display()))
                                        .monospace()
                                        .size(10.0)
                                        .color(Color32::from_gray(0x66)),
                                )
                                .truncate(),
                            );
                        });
                    });
                    ui.horizontal(|ui| {
                        for (tab, label) in TABS {
                            ui.selectable_value(&mut self.tab, tab, RichText::new(label).strong());
                        }
                    });
                    ui.separator();
                    match self.tab {
                        Tab::Conexion => self.tab_conexion(ui),
                        Tab::Pantalla => self.tab_pantalla(ui),
                        Tab::Estilo => self.tab_estilo(ui),
                        Tab::Deteccion => self.tab_deteccion(ui),
                        Tab::Voz => self.tab_voz(ui),
                    }
                });
            outcome
        })
    }

    fn tab_conexion(&mut self, ui: &mut egui::Ui) {
        row(ui, "URL del servidor WebSocket", |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.draft.ws_url).desired_width(260.0).font(egui::TextStyle::Monospace));
        });
        ui.separator();
        hint(
            ui,
            "El overlay se conecta a este servidor WebSocket.\n\
             Si el servidor exige token, ponlo en la URL: ws://host:8765/?token=…\n\
             Los cambios se aplican al reiniciar la aplicación.",
        );
    }

    fn tab_pantalla(&mut self, ui: &mut egui::Ui) {
        row(ui, "Ancho máximo (px)", |ui| {
            ui.add(DragValue::new(&mut self.draft.overlay_w).range(200..=1920));
        });
        hint(ui, "La ventana se ajusta sola al texto; el alto no se configura.");
        ui.checkbox(&mut self.draft.always_on_top, "Siempre encima");
        ui.checkbox(&mut self.draft.auto_update, "Actualizar automáticamente al iniciar");
        ui.separator();
        header(ui, "Fuente");
        row(ui, "Familia", |ui| {
            let current = self.draft.font_family.clone();
            ComboBox::from_id_salt("font_family").selected_text(&current).show_ui(ui, |ui| {
                let mut families: Vec<&str> = FONTS.to_vec();
                if !families.contains(&current.as_str()) {
                    families.insert(0, &current);
                }
                for f in families {
                    ui.selectable_value(&mut self.draft.font_family, f.to_string(), f);
                }
            });
        });
        row(ui, "Tamaño (pt)", |ui| {
            ui.add(DragValue::new(&mut self.draft.font_size).range(8..=72));
        });
        ui.checkbox(&mut self.draft.font_bold, "Negrita");
    }

    fn tab_estilo(&mut self, ui: &mut egui::Ui) {
        header(ui, "Fondo del texto");
        for (value, label) in BG_STYLES {
            ui.radio_value(&mut self.draft.bg_style, value.to_string(), label);
        }
        ui.separator();
        color_row(ui, "Color del badge", &mut self.draft.badge_color);
        row(ui, "Relleno del badge (px)", |ui| {
            ui.add(DragValue::new(&mut self.draft.badge_padding).range(0..=40));
        });
        row(ui, "Radio de esquinas (px)", |ui| {
            ui.add(DragValue::new(&mut self.draft.badge_radius).range(0..=30));
        });
        ui.separator();
        color_row(ui, "Color del texto (manual)", &mut self.draft.text_color);
        color_row(ui, "Color del contorno", &mut self.draft.outline_color);
        row(ui, "Ancho del contorno (px)", |ui| {
            ui.add(DragValue::new(&mut self.draft.outline_width).range(0..=8));
        });
    }

    fn tab_deteccion(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.draft.auto_detect, "Activar detección automática de color");
        ui.separator();
        row(ui, "Intervalo (ms)", |ui| {
            ui.add(DragValue::new(&mut self.draft.detect_interval).range(500..=30_000).speed(50));
        });
        row(ui, "Tamaño de muestra (px)", |ui| {
            ui.add(DragValue::new(&mut self.draft.detect_sample_px).range(20..=200));
        });
        row(ui, "Umbral de cambio", |ui| {
            ui.add(DragValue::new(&mut self.draft.detect_threshold).range(5..=100));
        });
        ui.separator();
        hint(
            ui,
            "La detección lee un pequeño cuadrado de píxeles (tamaño de muestra\n\
             × tamaño de muestra) situado en el centro del área de contenido,\n\
             solo al intervalo indicado.  Ningún dato se almacena ni se envía.\n\n\
             Consejo: el estilo 'Badge adaptativo' queda genial con la detección activa.",
        );
    }

    fn tab_voz(&mut self, ui: &mut egui::Ui) {
        row(ui, "Voz Edge TTS", |ui| {
            let current = self.draft.tts_voice.clone();
            ComboBox::from_id_salt("tts_voice").selected_text(&current).width(200.0).show_ui(ui, |ui| {
                let mut voices: Vec<&str> = TTS_VOICES.to_vec();
                if !voices.contains(&current.as_str()) {
                    voices.insert(0, &current);
                }
                for v in voices {
                    ui.selectable_value(&mut self.draft.tts_voice, v.to_string(), v);
                }
            });
        });
        ui.separator();
        hint(
            ui,
            "Usa Edge TTS; si falla, usa la voz local de Windows.\n\
             La cola de voz se limpia al cambiar de voz.",
        );
    }
}

fn style(ui: &mut egui::Ui) {
    let v = ui.visuals_mut();
    v.override_text_color = Some(FG);
    v.widgets.inactive.bg_fill = ENTRY;
    v.widgets.inactive.weak_bg_fill = MID;
    v.widgets.hovered.weak_bg_fill = ENTRY;
    v.selection.bg_fill = ACC;
    v.extreme_bg_color = ENTRY;
}

fn header(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).strong().color(ACC));
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(HINT));
}

fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([190.0, 20.0], egui::Label::new(label).halign(egui::Align::LEFT));
        add(ui);
    });
}

fn color_row(ui: &mut egui::Ui, label: &str, hex: &mut String) {
    row(ui, label, |ui| {
        let current = hex_or(hex, Color32::BLACK);
        let mut rgb = [current.r(), current.g(), current.b()];
        if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
            *hex = to_hex(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
        }
        let edit = ui.add(egui::TextEdit::singleline(hex).desired_width(70.0).font(egui::TextStyle::Monospace));
        if edit.lost_focus() && parse_hex(hex).is_none() {
            *hex = to_hex(current);
        }
    });
}
