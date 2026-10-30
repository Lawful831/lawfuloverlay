//! LawfulOverlay mock WebSocket server, for testing the client without Discord.
//!
//! ```text
//! cargo run -p lawful-overlay-server --bin mock-server -- [--host 127.0.0.1] [--port 8765] [--interval 4]
//! ```
//!
//! Sends a fake Discord message to every connected client at a fixed interval.
//! Accepts any token and origin.

use std::sync::Arc;
use std::time::Duration;

use lawful_overlay_server::ws::{self, Hub, OverlayMessage, Policy};
use rand::seq::IndexedRandom;
use rand::RngExt;
use tokio::net::TcpListener;
use tracing::info;

const FAKE_USERS: &[&str] = &["StreamerDave", "xXGamerXx", "ProPlayer99", "CoolKid2000", "MysticWizard"];
const FAKE_MESSAGES: &[&str] = &[
    "GG that was insane!",
    "let's go!!!",
    "wait how did you do that move",
    "lmaooo rekt",
    "gg wp no re",
    "that clutch though 👀",
    "POGGERS",
    "bro what was that aim",
    "okay that was actually clean",
    "EZ Clap",
    "are you using hacks lol",
    "teach me sensei 🙏",
    "chat is cooked",
    "I can't believe that worked",
    "speedrun world record incoming?",
];

fn arg<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    lawful_overlay_server::init_logging();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let host: String = arg(&args, "--host", "127.0.0.1".to_string());
    let port: u16 = arg(&args, "--port", 8765);
    let interval: f64 = arg(&args, "--interval", 4.0);

    let listener = TcpListener::bind((host.as_str(), port)).await?;
    info!("Starting mock server on ws://{host}:{port}");
    info!("Fake messages every {interval}s — press Ctrl+C to stop");

    let hub = Arc::new(Hub::new());
    let server = ws::serve(listener, Arc::clone(&hub), Arc::new(Policy::open()), None);

    let broadcaster = {
        let hub = Arc::clone(&hub);
        async move {
            // Brief warm-up before the first message.
            tokio::time::sleep(Duration::from_secs(2)).await;
            loop {
                if hub.client_count() > 0 {
                    let (user, text, id) = {
                        let mut rng = rand::rng();
                        (
                            *FAKE_USERS.choose(&mut rng).expect("non-empty"),
                            *FAKE_MESSAGES.choose(&mut rng).expect("non-empty"),
                            rng.random_range(100_000_000u64..999_999_999),
                        )
                    };
                    info!("Broadcasting → {user}: {text}");
                    hub.broadcast(&OverlayMessage::new(user, text, None, id.to_string(), "general"));
                }
                tokio::time::sleep(Duration::from_secs_f64(interval.max(0.1))).await;
            }
        }
    };

    tokio::select! {
        _ = server => {}
        _ = broadcaster => {}
        _ = tokio::signal::ctrl_c() => info!("Mock server stopped."),
    }
    Ok(())
}
