//! LawfulOverlay — Discord chat overlay client.
//!
//! Connects to the LawfulOverlay server over WebSocket and shows the messages
//! on a transparent, always-on-top window.
//!
//! Privacy: "Detección automática" reads a small square of pixels (60×60 by
//! default) directly under the overlay to pick a legible text colour. That data
//! is never stored or sent anywhere. The bot token is not handled here; all
//! credentials live on the server.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod color;
mod config;
mod net;
mod sample;
mod settings;
mod text;
mod tts;
mod updater;

fn main() -> eframe::Result {
    eprintln!("LawfulOverlay — Cliente");
    updater::cleanup();
    let cfg = config::Config::load();
    eprintln!("Servidor: {}", text::redact_url(&cfg.ws_url));

    let options = app::native_options(&cfg);
    eframe::run_native(
        app::WINDOW_TITLE,
        options,
        Box::new(|cc| Ok(Box::new(app::OverlayApp::new(cc, cfg)))),
    )
}
