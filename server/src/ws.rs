//! The public WebSocket broadcaster.
//!
//! Clients are listeners: they authenticate with `?token=`, receive every
//! relayed message and may send nothing meaningful. Everything they do send is
//! discarded, but counted and rate limited so a frame flood cannot buy CPU.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::time::{interval, timeout, Instant, MissedTickBehavior};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::config::Config;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const PING_EVERY: Duration = Duration::from_secs(20);
const PONG_WITHIN: Duration = Duration::from_secs(20);
/// Messages a slow client may fall behind before it starts skipping.
const BACKLOG: usize = 64;

/// What goes over the wire to the overlay.
#[derive(Debug, Clone, Serialize)]
pub struct OverlayMessage {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub username: String,
    /// Overlay text — links and media tagged.
    pub content: String,
    /// What the voice reads out. Absent from the mock server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_speech: Option<String>,
    pub user_id: String,
    pub channel: String,
}

impl OverlayMessage {
    pub fn new(
        username: impl Into<String>,
        content: impl Into<String>,
        content_speech: Option<String>,
        user_id: impl Into<String>,
        channel: impl Into<String>,
    ) -> Self {
        Self {
            kind: "message",
            username: username.into(),
            content: content.into(),
            content_speech,
            user_id: user_id.into(),
            channel: channel.into(),
        }
    }
}

/// Connection policy.
#[derive(Debug, Clone)]
pub struct Policy {
    /// `None` accepts everyone (the mock server only).
    pub token: Option<String>,
    pub origins: HashSet<String>,
    pub max_clients: usize,
    pub max_per_ip: usize,
    pub max_frame_bytes: usize,
    pub max_frames_per_s: u32,
}

impl Policy {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            token: Some(cfg.ws_token.clone()),
            origins: cfg.ws_origins.clone(),
            max_clients: cfg.max_clients,
            max_per_ip: cfg.max_per_ip,
            max_frame_bytes: cfg.max_frame_bytes,
            max_frames_per_s: cfg.max_frames_per_s,
        }
    }

    /// No token, any origin, generous limits. For the local mock server.
    pub fn open() -> Self {
        Self {
            token: None,
            origins: HashSet::new(),
            max_clients: 1000,
            max_per_ip: 1000,
            max_frame_bytes: 64 * 1024,
            max_frames_per_s: u32::MAX,
        }
    }

    /// Native clients send no Origin. Browsers always do, so a foreign Origin is
    /// either a hostile page or a misconfigured client; both are refused.
    fn origin_allowed(&self, origin: Option<&str>) -> bool {
        match (&self.token, origin) {
            (None, _) | (_, None) => true,
            (_, Some(o)) => self.origins.contains(o.trim_end_matches('/')),
        }
    }

    fn token_allowed(&self, path: &str) -> bool {
        let Some(expected) = &self.token else {
            return true;
        };
        if expected.is_empty() {
            return false;
        }
        let given = url::Url::parse(&format!("ws://localhost{path}"))
            .ok()
            .and_then(|u| {
                u.query_pairs()
                    .find(|(k, _)| k == "token")
                    .map(|(_, v)| v.into_owned())
            })
            .unwrap_or_default();
        given.as_bytes().ct_eq(expected.as_bytes()).into()
    }
}

#[derive(Default)]
struct Registry {
    total: usize,
    per_ip: HashMap<IpAddr, usize>,
}

/// Holds a slot in the registry; released on drop.
struct Admission {
    registry: Arc<Mutex<Registry>>,
    ip: IpAddr,
}

impl Drop for Admission {
    fn drop(&mut self) {
        let mut reg = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        reg.total = reg.total.saturating_sub(1);
        if let Some(n) = reg.per_ip.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                reg.per_ip.remove(&self.ip);
            }
        }
        info!("Client disconnected. Total: {}", reg.total);
    }
}

/// Fan-out point between the Discord bot and the connected overlays.
pub struct Hub {
    tx: broadcast::Sender<Arc<str>>,
    registry: Arc<Mutex<Registry>>,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

impl Hub {
    pub fn new() -> Self {
        Self {
            tx: broadcast::channel(BACKLOG).0,
            registry: Arc::default(),
        }
    }

    pub fn client_count(&self) -> usize {
        self.registry.lock().unwrap_or_else(|e| e.into_inner()).total
    }

    /// Send a payload to every connected overlay client.
    pub fn broadcast(&self, msg: &OverlayMessage) {
        if self.tx.receiver_count() == 0 {
            return;
        }
        match serde_json::to_string(msg) {
            Ok(json) => {
                let _ = self.tx.send(json.into());
            }
            Err(e) => warn!("could not serialise message: {e}"),
        }
    }

    fn admit(&self, ip: IpAddr, policy: &Policy) -> Result<Admission, &'static str> {
        let mut reg = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if reg.total >= policy.max_clients {
            return Err("server full");
        }
        if reg.per_ip.get(&ip).copied().unwrap_or(0) >= policy.max_per_ip {
            return Err("too many connections from this address");
        }
        reg.total += 1;
        *reg.per_ip.entry(ip).or_insert(0) += 1;
        info!("Client connected ({ip}). Total: {}", reg.total);
        Ok(Admission {
            registry: Arc::clone(&self.registry),
            ip,
        })
    }
}

pub fn tls_acceptor(cert: &str, key: &str) -> Result<TlsAcceptor, String> {
    let certs = rustls_pemfile::certs(&mut BufReader::new(
        File::open(cert).map_err(|e| format!("cannot open WS_TLS_CERT {cert}: {e}"))?,
    ))
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| format!("bad certificate in {cert}: {e}"))?;
    let key = rustls_pemfile::private_key(&mut BufReader::new(
        File::open(key).map_err(|e| format!("cannot open WS_TLS_KEY {key}: {e}"))?,
    ))
    .map_err(|e| format!("bad private key in {key}: {e}"))?
    .ok_or_else(|| format!("no private key found in {key}"))?;
    let config = tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("invalid TLS certificate/key: {e}"))?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Accept connections until the future is dropped.
pub async fn serve(
    listener: TcpListener,
    hub: Arc<Hub>,
    policy: Arc<Policy>,
    tls: Option<TlsAcceptor>,
) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                warn!("accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let (hub, policy, tls) = (Arc::clone(&hub), Arc::clone(&policy), tls.clone());
        tokio::spawn(async move {
            match tls {
                Some(acceptor) => match timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
                    Ok(Ok(stream)) => handle(stream, peer, hub, policy).await,
                    Ok(Err(e)) => debug!("TLS handshake with {peer} failed: {e}"),
                    Err(_) => debug!("TLS handshake with {peer} timed out"),
                },
                None => handle(stream, peer, hub, policy).await,
            }
        });
    }
}

fn close_frame(code: CloseCode, reason: &'static str) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: reason.into(),
    }))
}

async fn handle<S>(stream: S, peer: SocketAddr, hub: Arc<Hub>, policy: Arc<Policy>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut path = String::new();
    let mut origin: Option<String> = None;
    let config = WebSocketConfig::default()
        .max_message_size(Some(policy.max_frame_bytes))
        .max_frame_size(Some(policy.max_frame_bytes));
    let callback = |req: &Request, resp: Response| {
        path = req.uri().to_string();
        origin = req
            .headers()
            .get("origin")
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
        Ok(resp)
    };
    let ws = match timeout(
        HANDSHAKE_TIMEOUT,
        tokio_tungstenite::accept_hdr_async_with_config(stream, callback, Some(config)),
    )
    .await
    {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => return debug!("WebSocket handshake with {peer} failed: {e}"),
        Err(_) => return debug!("WebSocket handshake with {peer} timed out"),
    };
    let (mut sink, mut stream) = ws.split();

    let refusal = if !policy.origin_allowed(origin.as_deref()) {
        warn!("Rejected origin {origin:?} from {peer}");
        Some(close_frame(CloseCode::Policy, "origin not allowed"))
    } else if !policy.token_allowed(&path) {
        warn!("Rejected unauthenticated client from {peer}");
        Some(close_frame(CloseCode::Policy, "token required"))
    } else {
        None
    };
    if let Some(frame) = refusal {
        let _ = sink.send(frame).await;
        let _ = sink.close().await;
        return;
    }

    let _admission = match hub.admit(peer.ip(), &policy) {
        Ok(a) => a,
        Err(reason) => {
            warn!("Rejected client from {peer}: {reason}");
            let _ = sink.send(close_frame(CloseCode::Again, reason)).await;
            let _ = sink.close().await;
            return;
        }
    };

    let mut rx = hub.tx.subscribe();
    let mut ping = interval(PING_EVERY);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ping.tick().await; // the first tick is immediate
    let mut ping_sent: Option<Instant> = None;
    let mut frames: u32 = 0;
    let mut window = Instant::now();

    loop {
        tokio::select! {
            frame = stream.next() => match frame {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(Message::Pong(_))) => ping_sent = None,
                Some(Ok(_)) => {
                    if window.elapsed() >= Duration::from_secs(1) {
                        frames = 0;
                        window = Instant::now();
                    }
                    frames += 1;
                    if frames > policy.max_frames_per_s {
                        warn!("Closing {peer}: {frames} frames in {:.2}s", window.elapsed().as_secs_f32());
                        let _ = sink.send(close_frame(CloseCode::Policy, "rate limit")).await;
                        break;
                    }
                }
            },
            msg = rx.recv() => match msg {
                Ok(json) => {
                    if sink.send(Message::text(json.to_string())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("{peer} is {n} messages behind; skipped them");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = ping.tick() => {
                match ping_sent {
                    Some(sent) if sent.elapsed() >= PONG_WITHIN => {
                        debug!("{peer} did not answer ping");
                        break;
                    }
                    Some(_) => {}
                    None => {
                        if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                            break;
                        }
                        ping_sent = Some(Instant::now());
                    }
                }
            }
        }
    }
    let _ = sink.close().await;
}

/// Plain TCP connect to the local socket: the container health check.
pub async fn healthcheck(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    matches!(
        timeout(Duration::from_secs(2), TcpStream::connect(addr)).await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(token: &str, origins: &[&str]) -> Policy {
        Policy {
            token: Some(token.to_string()),
            origins: origins.iter().map(|s| s.to_string()).collect(),
            ..Policy::open()
        }
    }

    #[test]
    fn token_must_match() {
        let p = policy("s3cret", &[]);
        assert!(p.token_allowed("/?token=s3cret"));
        assert!(p.token_allowed("/?a=1&token=s3cret"));
        assert!(!p.token_allowed("/?token=wrong"));
        assert!(!p.token_allowed("/"));
        assert!(!p.token_allowed("/?token="));
        assert!(!policy("", &[]).token_allowed("/?token="));
    }

    #[test]
    fn origins() {
        let p = policy("t", &["https://ok.example"]);
        assert!(p.origin_allowed(None));
        assert!(p.origin_allowed(Some("https://ok.example")));
        assert!(p.origin_allowed(Some("https://ok.example/")));
        assert!(!p.origin_allowed(Some("https://evil.example")));
    }

    #[test]
    fn admission_caps_and_release() {
        let hub = Hub::new();
        let p = Policy {
            max_clients: 3,
            max_per_ip: 2,
            ..policy("t", &[])
        };
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        let first = hub.admit(a, &p).unwrap();
        let _second = hub.admit(a, &p).unwrap();
        assert!(hub.admit(a, &p).is_err(), "per-IP cap");
        let _third = hub.admit(b, &p).unwrap();
        assert!(hub.admit(b, &p).is_err(), "global cap");
        drop(first);
        assert_eq!(hub.client_count(), 2);
        assert!(hub.admit(b, &p).is_ok());
    }
}
