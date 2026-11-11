//! WebSocket listener (background thread). Reconnects with exponential backoff.

use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use base64::Engine;
use eframe::egui;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::text::{is_web_url, redact_url};

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
        /// The URLs behind the link tags in `content`, in order.
        links: Vec<String>,
        /// Images to show under the text (web URLs only).
        images: Vec<String>,
    },
    /// Neither Edge TTS nor the local Windows voice worked.
    TtsFailed,
}

/// The server pings every 20 s; silence for much longer means a dead link.
const IDLE_LIMIT: Duration = Duration::from_secs(70);
const CONNECT_LIMIT: Duration = Duration::from_secs(15);

/// How long the speech thread waits for the server to answer a `tts` request
/// (the server gives Fish Audio 60 s).
const TTS_WAIT: Duration = Duration::from_secs(90);

type TtsResult = Result<Vec<u8>, String>;

/// A request for Fish Audio speech, answered on `reply` by the socket thread.
struct TtsRequest {
    text: String,
    voice: String,
    reply: Sender<TtsResult>,
}

/// How the speech thread asks the server for Fish Audio speech. The API key
/// lives on the server only; this is just a request over the open socket.
#[derive(Clone)]
pub struct TtsLink {
    tx: UnboundedSender<TtsRequest>,
}

impl TtsLink {
    /// mp3 audio, or why there is none (no key on the server, a limit, a
    /// refusal, no connection). Blocks until the server answers.
    pub fn synthesize(&self, text: &str, voice: &str) -> TtsResult {
        let (reply, answer) = mpsc::channel();
        let request = TtsRequest { text: text.into(), voice: voice.into(), reply };
        self.tx.send(request).map_err(|_| "sin conexión con el servidor".to_string())?;
        answer.recv_timeout(TTS_WAIT).unwrap_or_else(|_| Err("el servidor no respondió".into()))
    }

    /// A link to nowhere: every request fails.
    #[cfg(test)]
    pub fn offline() -> Self {
        Self { tx: unbounded_channel().0 }
    }
}

/// Start the socket thread. The returned link carries speech requests to it.
pub fn spawn(url: String, tx: Sender<Event>, ctx: egui::Context) -> TtsLink {
    let (requests, tts_rx) = unbounded_channel();
    std::thread::Builder::new()
        .name("ws-listener".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(listen(url, tx, ctx, tts_rx));
        })
        .expect("spawn ws thread");
    TtsLink { tx: requests }
}

/// A `tts_audio` / `tts_error` reply: the request id and its outcome.
fn parse_tts_reply(raw: &str) -> Option<(u64, TtsResult)> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let id = v.get("id")?.as_u64()?;
    match v.get("type")?.as_str()? {
        "tts_audio" => {
            let audio = v.get("audio")?.as_str()?;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(audio)
                .map_err(|_| "audio no válido".to_string());
            Some((id, decoded))
        }
        "tts_error" => {
            let error = v.get("error").and_then(|e| e.as_str()).unwrap_or("error");
            Some((id, Err(error.to_string())))
        }
        _ => None,
    }
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
    let urls = |key: &str| -> Vec<String> {
        v.get(key)
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|u| u.as_str()).map(str::to_string).collect())
            .unwrap_or_default()
    };
    let links = urls("links");
    let images = urls("images").into_iter().filter(|u| is_web_url(u)).collect();
    Some(Event::Chat { user, content, speech, links, images })
}

async fn listen(url: String, tx: Sender<Event>, ctx: egui::Context, mut tts_rx: UnboundedReceiver<TtsRequest>) {
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
                let mut pending: HashMap<u64, Sender<TtsResult>> = HashMap::new();
                let mut next_id = 0u64;
                let mut last_seen = tokio::time::Instant::now();
                loop {
                    tokio::select! {
                        frame = ws.next() => {
                            last_seen = tokio::time::Instant::now();
                            match frame {
                                Some(Ok(Message::Text(raw))) => {
                                    if let Some(event) = parse_frame(&raw) {
                                        if !send(event) {
                                            return;
                                        }
                                    } else if let Some((id, result)) = parse_tts_reply(&raw) {
                                        if let Some(reply) = pending.remove(&id) {
                                            let _ = reply.send(result);
                                        }
                                    }
                                }
                                Some(Ok(Message::Close(frame))) => {
                                    if let Some(f) = frame {
                                        eprintln!("[ws] cerrado por el servidor: {} {}", f.code, f.reason);
                                        if !f.reason.is_empty() && !send(Event::Status(format!("Servidor: {}", f.reason))) {
                                            return;
                                        }
                                    }
                                    break;
                                }
                                Some(Ok(_)) => {}
                                Some(Err(e)) => {
                                    eprintln!("[ws] {e}");
                                    break;
                                }
                                None => break,
                            }
                        }
                        Some(req) = tts_rx.recv() => {
                            next_id += 1;
                            let body = serde_json::json!({ "type": "tts", "id": next_id, "text": req.text, "voice": req.voice });
                            if ws.send(Message::text(body.to_string())).await.is_err() {
                                let _ = req.reply.send(Err("sin conexión con el servidor".into()));
                                break;
                            }
                            pending.insert(next_id, req.reply);
                        }
                        _ = tokio::time::sleep_until(last_seen + IDLE_LIMIT) => {
                            eprintln!("[ws] sin actividad; reconectando");
                            break;
                        }
                    }
                }
                for (_, reply) in pending.drain() {
                    let _ = reply.send(Err("conexión perdida".into()));
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
        // Speech requests made while offline fail at once, so the Edge voice takes over.
        let wake = tokio::time::sleep(Duration::from_secs(delay));
        tokio::pin!(wake);
        loop {
            tokio::select! {
                _ = &mut wake => break,
                Some(req) = tts_rx.recv() => {
                    let _ = req.reply.send(Err("sin conexión con el servidor".into()));
                }
            }
        }
        delay = (delay * 2).min(60);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_server_messages() {
        let raw = r#"{"type":"message","username":"ana","content":"hola [GIF]","content_speech":"hola gif","user_id":"1","channel":"c"}"#;
        let Some(Event::Chat { user, content, speech, links, images }) = parse_frame(raw) else {
            panic!("expected a chat event");
        };
        assert_eq!((user.as_str(), content.as_str(), speech.as_str()), ("ana", "hola [GIF]", "hola gif"));
        assert!(links.is_empty() && images.is_empty());
    }

    #[test]
    fn parses_links_and_images() {
        let raw = r#"{"type":"message","username":"a","content":"[Link de x]","links":["https://x.com/1"],"images":["https://m/a.png","file:///C:/a.png",3]}"#;
        let Some(Event::Chat { links, images, .. }) = parse_frame(raw) else {
            panic!();
        };
        assert_eq!(links, ["https://x.com/1"]);
        assert_eq!(images, ["https://m/a.png"]);
    }

    #[test]
    fn speech_falls_back_to_content() {
        let Some(Event::Chat { speech, .. }) = parse_frame(r#"{"type":"message","username":"a","content":"x"}"#) else {
            panic!();
        };
        assert_eq!(speech, "x");
    }

    #[test]
    fn parses_tts_replies() {
        let audio = base64::engine::general_purpose::STANDARD.encode(b"mp3");
        let ok = format!(r#"{{"type":"tts_audio","id":4,"audio":"{audio}"}}"#);
        assert_eq!(parse_tts_reply(&ok).unwrap(), (4, Ok(b"mp3".to_vec())));
        let err = parse_tts_reply(r#"{"type":"tts_error","id":5,"error":"rate limit"}"#).unwrap();
        assert_eq!(err, (5, Err("rate limit".to_string())));
        assert!(parse_tts_reply(r#"{"type":"message","id":1}"#).is_none());
        assert!(parse_tts_reply("not json").is_none());
    }

    #[test]
    fn offline_link_fails_at_once() {
        assert!(TtsLink::offline().synthesize("hola", "v").is_err());
    }

    #[test]
    fn ignores_other_frames() {
        assert!(parse_frame("not json").is_none());
        assert!(parse_frame(r#"{"type":"ping"}"#).is_none());
        assert!(parse_frame(r#"{"username":"a"}"#).is_none());
    }
}
