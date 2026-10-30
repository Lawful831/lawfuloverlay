//! End-to-end checks of the socket policy against a real listener.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use lawful_overlay_server::ws::{self, Hub, OverlayMessage, Policy};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Message;

async fn start(policy: Policy) -> (u16, Arc<Hub>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hub = Arc::new(Hub::new());
    tokio::spawn(ws::serve(listener, Arc::clone(&hub), Arc::new(policy), None));
    (port, hub)
}

fn policy() -> Policy {
    Policy {
        token: Some("s3cret".into()),
        origins: HashSet::from(["https://ok.example".to_string()]),
        max_clients: 2,
        max_per_ip: 2,
        max_frame_bytes: 1024,
        max_frames_per_s: 5,
    }
}

/// The close frame the server sends, as (code, reason).
async fn expect_close(
    ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
) -> (CloseCode, String) {
    loop {
        match timeout(Duration::from_secs(5), ws.next()).await.expect("timed out") {
            Some(Ok(Message::Close(Some(f)))) => return (f.code, f.reason.to_string()),
            Some(Ok(_)) => continue,
            other => panic!("expected a close frame, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn missing_or_wrong_token_is_refused() {
    let (port, _hub) = start(policy()).await;
    for url in [format!("ws://127.0.0.1:{port}/"), format!("ws://127.0.0.1:{port}/?token=nope")] {
        let (mut ws, _) = connect_async(url).await.unwrap();
        assert_eq!(expect_close(&mut ws).await, (CloseCode::Policy, "token required".into()));
    }
}

#[tokio::test]
async fn foreign_origin_is_refused() {
    let (port, _hub) = start(policy()).await;
    let mut req = format!("ws://127.0.0.1:{port}/?token=s3cret").into_client_request().unwrap();
    req.headers_mut().insert("origin", "https://evil.example".parse().unwrap());
    let (mut ws, _) = connect_async(req).await.unwrap();
    assert_eq!(expect_close(&mut ws).await, (CloseCode::Policy, "origin not allowed".into()));
}

#[tokio::test]
async fn authenticated_client_receives_broadcasts() {
    let (port, hub) = start(policy()).await;
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/?token=s3cret")).await.unwrap();
    while hub.client_count() == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    hub.broadcast(&OverlayMessage::new("ana", "hola [GIF]", Some("hola gif".into()), "42", "general"));
    let Message::Text(text) = timeout(Duration::from_secs(5), ws.next()).await.unwrap().unwrap().unwrap() else {
        panic!("expected a text frame");
    };
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["type"], "message");
    assert_eq!(v["username"], "ana");
    assert_eq!(v["content"], "hola [GIF]");
    assert_eq!(v["content_speech"], "hola gif");
    assert_eq!(v["user_id"], "42");
    assert_eq!(v["channel"], "general");
}

#[tokio::test]
async fn connection_cap_refuses_with_1013() {
    let (port, hub) = start(policy()).await;
    let url = format!("ws://127.0.0.1:{port}/?token=s3cret");
    let _a = connect_async(url.clone()).await.unwrap().0;
    let _b = connect_async(url.clone()).await.unwrap().0;
    while hub.client_count() < 2 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (mut c, _) = connect_async(url).await.unwrap();
    assert_eq!(expect_close(&mut c).await.0, CloseCode::Again);
}

#[tokio::test]
async fn frame_flood_is_closed_with_1008() {
    use futures_util::SinkExt;
    let (port, _hub) = start(policy()).await;
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}/?token=s3cret")).await.unwrap();
    for _ in 0..50 {
        if ws.send(Message::text("x")).await.is_err() {
            break;
        }
    }
    assert_eq!(expect_close(&mut ws).await, (CloseCode::Policy, "rate limit".into()));
}
