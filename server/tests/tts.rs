//! `tts` requests over the socket, against a local stand-in for Fish Audio.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use lawful_overlay_server::fish::{self, Fish};
use lawful_overlay_server::ws::{self, Hub, Policy};
use serde_json::{json, Value};
use tokio::net::TcpListener as TokioListener;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

const KEY: &str = "fish-test-key-123";
const MP3: &[u8] = b"ID3 fake mp3 bytes";

type Answer = fn(&str) -> (u16, Vec<u8>);

/// What the fake Fish endpoint was asked, in order.
#[derive(Default, Clone)]
struct Seen {
    models: Arc<Mutex<Vec<String>>>,
    auths: Arc<Mutex<Vec<String>>>,
}

/// A plain-HTTP stand-in for `https://api.fish.audio/v1/tts`. `answer` gives
/// the status and body for each model. Returns the endpoint URL.
fn fake_fish(answer: Answer) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1/tts", listener.local_addr().unwrap());
    let seen = Seen::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut model, mut auth, mut len) = (String::new(), String::new(), 0usize);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    let value = value.trim();
                    match name.to_ascii_lowercase().as_str() {
                        "model" => model = value.to_string(),
                        "authorization" => auth = value.to_string(),
                        "content-length" => len = value.parse().unwrap_or(0),
                        _ => {}
                    }
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            log.models.lock().unwrap().push(model.clone());
            log.auths.lock().unwrap().push(auth);
            let (status, payload) = answer(&model);
            let head = format!(
                "HTTP/1.1 {status} status\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&payload).unwrap();
        }
    });
    (url, seen)
}

/// The paid model is out of credit; the free one answers.
fn paid_refused(model: &str) -> (u16, Vec<u8>) {
    if model == "s2.1-pro" {
        (402, br#"{"status":402,"message":"no credit"}"#.to_vec())
    } else {
        (200, MP3.to_vec())
    }
}

fn always_ok(_: &str) -> (u16, Vec<u8>) {
    (200, MP3.to_vec())
}

/// A bad key whose error message echoes the key back.
fn echoes_key(_: &str) -> (u16, Vec<u8>) {
    (
        401,
        format!(r#"{{"message":"invalid key {KEY}"}}"#).into_bytes(),
    )
}

fn fish_at(url: &str, daily_limit: u32) -> Option<Arc<Fish>> {
    Fish::with_endpoint(KEY, url, daily_limit).map(Arc::new)
}

fn policy(tts: Option<Arc<Fish>>) -> Policy {
    Policy {
        token: Some("s3cret".into()),
        origins: HashSet::new(),
        max_clients: 4,
        max_per_ip: 4,
        max_frame_bytes: 64 * 1024,
        max_frames_per_s: 50,
        tts,
    }
}

async fn start(tts: Option<Arc<Fish>>) -> u16 {
    let listener = TokioListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hub = Arc::new(Hub::new());
    tokio::spawn(ws::serve(listener, hub, Arc::new(policy(tts)), None));
    port
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(port: u16) -> Socket {
    connect_async(format!("ws://127.0.0.1:{port}/?token=s3cret"))
        .await
        .unwrap()
        .0
}

/// Sends one `tts` request and returns the next JSON reply.
async fn ask(ws: &mut Socket, id: u64, text: &str) -> Value {
    let req = json!({ "type": "tts", "id": id, "text": text, "voice": "voice-abc" });
    ws.send(Message::text(req.to_string())).await.unwrap();
    loop {
        match timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("no reply")
        {
            Some(Ok(Message::Text(raw))) => return serde_json::from_str(&raw).unwrap(),
            Some(Ok(_)) => continue,
            other => panic!("expected a reply, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn audio_comes_back_base64_and_falls_back_to_the_free_model() {
    let (url, seen) = fake_fish(paid_refused);
    let mut ws = connect(start(fish_at(&url, 300)).await).await;

    let reply = ask(&mut ws, 7, "[happy] hola").await;
    assert_eq!(reply["type"], "tts_audio");
    assert_eq!(reply["id"], 7);
    let audio = base64::engine::general_purpose::STANDARD
        .decode(reply["audio"].as_str().unwrap())
        .unwrap();
    assert_eq!(audio, MP3);

    assert_eq!(*seen.models.lock().unwrap(), ["s2.1-pro", "s2.1-pro-free"]);
    assert!(seen
        .auths
        .lock()
        .unwrap()
        .iter()
        .all(|a| a == &format!("Bearer {KEY}")));
}

#[tokio::test]
async fn without_a_key_the_socket_refuses_tts() {
    let mut ws = connect(start(None).await).await;
    let reply = ask(&mut ws, 1, "hola").await;
    assert_eq!(
        (reply["type"].clone(), reply["error"].clone()),
        (json!("tts_error"), json!("tts unavailable"))
    );
}

#[tokio::test]
async fn overlong_or_empty_text_never_reaches_fish() {
    let (url, seen) = fake_fish(always_ok);
    let mut ws = connect(start(fish_at(&url, 300)).await).await;

    let long = "a".repeat(fish::MAX_CHARS + 1);
    assert_eq!(ask(&mut ws, 1, &long).await["error"], "text too long");
    assert_eq!(ask(&mut ws, 2, "").await["error"], "empty text");
    assert!(seen.models.lock().unwrap().is_empty());
}

#[tokio::test]
async fn errors_never_contain_the_key() {
    let (url, _seen) = fake_fish(echoes_key);
    let mut ws = connect(start(fish_at(&url, 300)).await).await;

    let reply = ask(&mut ws, 3, "hola").await;
    assert_eq!(reply["type"], "tts_error");
    let text = reply.to_string();
    assert!(text.contains("HTTP 401"), "{text}");
    assert!(!text.contains(KEY), "{text}");
}

#[tokio::test]
async fn each_connection_is_rate_limited() {
    let (url, _seen) = fake_fish(always_ok);
    let mut ws = connect(start(fish_at(&url, 300)).await).await;

    for id in 1..=10 {
        assert_eq!(
            ask(&mut ws, id, "hola").await["type"],
            "tts_audio",
            "request {id}"
        );
    }
    assert_eq!(ask(&mut ws, 11, "hola").await["error"], "rate limit");
}

#[tokio::test]
async fn the_daily_cap_is_shared_by_all_connections() {
    let (url, seen) = fake_fish(always_ok);
    let port = start(fish_at(&url, 1)).await;

    let mut first = connect(port).await;
    assert_eq!(ask(&mut first, 1, "hola").await["type"], "tts_audio");
    let mut second = connect(port).await;
    assert_eq!(
        ask(&mut second, 1, "hola").await["error"],
        "daily limit reached"
    );
    assert_eq!(seen.models.lock().unwrap().len(), 1);
}
