//! WebSocket listener (background thread). Reconnects with exponential backoff.

use std::sync::mpsc::Sender;
use std::time::Duration;

use eframe::egui;
use futures_util::StreamExt;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::text::redact_url;

/// What the background threads tell the UI.
#[derive(Debug)]
pub enum Event {
    /// Connection status — shown, never spoken.
    Status(String),
    Chat {
        user: String,
        /// What the overlay shows.
        content: String,
        /// What the voice reads (falls back to `content`).
        speech: String,
    },
    /// Neither Edge TTS nor the local Windows voice worked.
    TtsFailed,
}

/// The server pings every 20 s; silence for much longer means a dead link.
const IDLE_LIMIT: Duration = Duration::from_secs(70);
const CONNECT_LIMIT: Duration = Duration::from_secs(15);

pub fn spawn(url: String, tx: Sender<Event>, ctx: egui::Context) {
    std::thread::Builder::new()
        .name("ws-listener".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(listen(url, tx, ctx));
        })
        .expect("spawn ws thread");
}

/// Parse a server frame. Anything that isn't a `message` is ignored.
pub fn parse_frame(raw: &str) -> Option<Event> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    if v.get("type")?.as_str()? != "message" {
        return None;
    }
    let text = |key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let user = match text("username") {
        u if u.is_empty() => "?".to_string(),
        u => u,
    };
    let content = text("content");
    let speech = match text("content_speech") {
        s if s.is_empty() => content.clone(),
        s => s,
    };
    Some(Event::Chat { user, content, speech })
}

async fn listen(url: String, tx: Sender<Event>, ctx: egui::Context) {
    let send = |event: Event| {
        let ok = tx.send(event).is_ok();
        ctx.request_repaint();
        ok
    };
    let shown = redact_url(&url);
    let mut delay = 2u64;
    loop {
        if !send(Event::Status(format!("Conectando a {shown}…"))) {
            return; // the UI is gone
        }
        let outcome = tokio::time::timeout(CONNECT_LIMIT, connect_async(url.as_str())).await;
        match outcome {
            Ok(Ok((mut ws, _))) => {
                delay = 2;
                if !send(Event::Status("Conectado ✓".into())) {
                    return;
                }
                loop {
                    match tokio::time::timeout(IDLE_LIMIT, ws.next()).await {
                        Ok(Some(Ok(Message::Text(raw)))) => {
                            if let Some(event) = parse_frame(&raw) {
                                if !send(event) {
                                    return;
                                }
                            }
                        }
                        Ok(Some(Ok(Message::Close(frame)))) => {
                            if let Some(f) = frame {
                                eprintln!("[ws] cerrado por el servidor: {} {}", f.code, f.reason);
                                if !f.reason.is_empty() && !send(Event::Status(format!("Servidor: {}", f.reason))) {
                                    return;
                                }
                            }
                            break;
                        }
                        Ok(Some(Ok(_))) => {}
                        Ok(Some(Err(e))) => {
                            eprintln!("[ws] {e}");
                            break;
                        }
                        Ok(None) => break,
                        Err(_) => {
                            eprintln!("[ws] sin actividad; reconectando");
                            break;
                        }
                    }
                }
                if !send(Event::Status(format!("Desconectado. Reintentando en {delay}s…"))) {
                    return;
                }
            }
            Ok(Err(e)) => {
                eprintln!("[ws] {e} — reintento en {delay}s");
                if !send(Event::Status(format!("Desconectado. Reintentando en {delay}s…"))) {
                    return;
                }
            }
            Err(_) => {
                if !send(Event::Status(format!("Sin respuesta. Reintentando en {delay}s…"))) {
                    return;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(delay)).await;
        delay = (delay * 2).min(60);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_server_messages() {
        let raw = r#"{"type":"message","username":"ana","content":"hola [GIF]","content_speech":"hola gif","user_id":"1","channel":"c"}"#;
        let Some(Event::Chat { user, content, speech }) = parse_frame(raw) else {
            panic!("expected a chat event");
        };
        assert_eq!((user.as_str(), content.as_str(), speech.as_str()), ("ana", "hola [GIF]", "hola gif"));
    }

    #[test]
    fn speech_falls_back_to_content() {
        let Some(Event::Chat { speech, .. }) = parse_frame(r#"{"type":"message","username":"a","content":"x"}"#) else {
            panic!();
        };
        assert_eq!(speech, "x");
    }

    #[test]
    fn ignores_other_frames() {
        assert!(parse_frame("not json").is_none());
        assert!(parse_frame(r#"{"type":"ping"}"#).is_none());
        assert!(parse_frame(r#"{"username":"a"}"#).is_none());
    }
}
