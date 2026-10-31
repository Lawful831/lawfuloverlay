//! Client settings, persisted as JSON.
//!
//! The file location and keys match the Python client, so existing settings
//! carry over.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const BG_STYLES: [(&str, &str); 3] = [
    ("none", "Ninguno (transparente)"),
    ("badge", "Badge sólido"),
    ("auto_badge", "Badge adaptativo (semitransparente)"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // Conexión
    pub ws_url: String,

    // Pantalla
    pub overlay_w: u32,
    pub overlay_h: u32,
    pub always_on_top: bool,

    // Fuente
    pub font_family: String,
    pub font_size: u32,
    pub font_bold: bool,

    // Estilo: "none" | "badge" | "auto_badge"
    pub bg_style: String,
    pub badge_color: String,
    pub badge_padding: u32,
    pub badge_radius: u32,
    pub text_color: String,
    pub outline_color: String,
    pub outline_width: u32,

    // Detección
    pub auto_detect: bool,
    /// ms between pixel samples.
    pub detect_interval: u32,
    /// Side of the sample square in px.
    pub detect_sample_px: u32,
    /// Minimum per-channel delta before the colour is updated.
    pub detect_threshold: u32,

    // Voz
    pub tts_enabled: bool,
    pub tts_voice: String,

    // Actualizaciones
    pub auto_update: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // A private build can bake in its server: LAWFUL_OVERLAY_DEFAULT_WS_URL=... cargo build
            ws_url: option_env!("LAWFUL_OVERLAY_DEFAULT_WS_URL").unwrap_or("ws://127.0.0.1:8765").into(),
            overlay_w: 440,
            overlay_h: 110,
            always_on_top: true,
            font_family: "Arial".into(),
            font_size: 14,
            font_bold: true,
            bg_style: "none".into(),
            badge_color: "#111827".into(),
            badge_padding: 10,
            badge_radius: 8,
            text_color: "#FFFFFF".into(),
            outline_color: "#000000".into(),
            outline_width: 2,
            auto_detect: true,
            detect_interval: 2500,
            detect_sample_px: 60,
            detect_threshold: 25,
            tts_enabled: false,
            tts_voice: "en-US-AriaNeural".into(),
            auto_update: true,
        }
    }
}

/// `LAWFUL_OVERLAY_CONFIG` overrides the location (handy for testing).
pub fn config_path() -> PathBuf {
    if let Some(custom) = std::env::var_os("LAWFUL_OVERLAY_CONFIG") {
        return PathBuf::from(custom);
    }
    let dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".discord_chat_overlay");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("client_settings.json")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str(&raw) {
            Ok(cfg) => {
                eprintln!("[cfg] Cargado desde {}", path.display());
                cfg
            }
            Err(e) => {
                eprintln!("[cfg] Error al cargar: {e}");
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let path = config_path();
        let result = serde_json::to_string_pretty(self)
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(&path, json).map_err(|e| e.to_string()));
        match result {
            Ok(()) => eprintln!("[cfg] Guardado en {}", path.display()),
            Err(e) => eprintln!("[cfg] Error al guardar: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_files_fall_back_to_defaults() {
        let cfg: Config = serde_json::from_str(r##"{"ws_url": "ws://x:1/?token=t", "font_size": 20}"##).unwrap();
        assert_eq!(cfg.ws_url, "ws://x:1/?token=t");
        assert_eq!(cfg.font_size, 20);
        assert_eq!(cfg.overlay_w, Config::default().overlay_w);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let cfg: Config = serde_json::from_str(r#"{"legacy": 1}"#).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn roundtrip() {
        let cfg = Config::default();
        let back: Config = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(cfg, back);
    }
}
