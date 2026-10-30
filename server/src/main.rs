use std::process::ExitCode;
use std::sync::Arc;

use lawful_overlay_server::config::Config;
use lawful_overlay_server::discord::{run_bot, Handler};
use lawful_overlay_server::ws::{self, Hub, Policy};
use tokio::net::TcpListener;
use tracing::{error, info};

#[tokio::main]
async fn main() -> ExitCode {
    let _ = dotenvy::dotenv();
    lawful_overlay_server::init_logging();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            error!("{msg}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let cfg = Config::from_env()?;

    // Container health check: is the socket accepting connections?
    if flag("--healthcheck") {
        return if ws::healthcheck(cfg.ws_port).await {
            Ok(())
        } else {
            Err(format!("nothing is listening on port {}", cfg.ws_port))
        };
    }

    let ws_only = flag("--ws-only");
    if cfg.ws_token.is_empty() {
        return Err("WS_TOKEN is not set: refusing to expose an unauthenticated socket".into());
    }
    if cfg.bot_token.is_empty() && !ws_only {
        return Err("DISCORD_BOT_TOKEN is not set (check server/.env)".into());
    }

    let tls = if !cfg.tls_cert.is_empty() && !cfg.tls_key.is_empty() {
        Some(ws::tls_acceptor(&cfg.tls_cert, &cfg.tls_key)?)
    } else {
        None
    };
    let scheme = if tls.is_some() { "wss" } else { "ws" };

    let listener = TcpListener::bind((cfg.ws_host.as_str(), cfg.ws_port))
        .await
        .map_err(|e| format!("cannot bind {}:{}: {e}", cfg.ws_host, cfg.ws_port))?;
    info!(
        "Starting WebSocket server on {scheme}://{}:{} (max {} clients)",
        cfg.ws_host, cfg.ws_port, cfg.max_clients
    );

    let hub = Arc::new(Hub::new());
    let policy = Arc::new(Policy::from_config(&cfg));
    let server = ws::serve(listener, Arc::clone(&hub), policy, tls);

    if ws_only {
        info!("--ws-only: WebSocket server running without the Discord bot");
        tokio::select! {
            _ = server => {}
            _ = shutdown_signal() => info!("Shutting down."),
        }
        return Ok(());
    }

    let handler = Handler {
        hub,
        user_ids: cfg.target_user_ids.clone(),
        server_ids: cfg.target_server_ids.clone(),
    };
    tokio::select! {
        _ = server => Ok(()),
        res = run_bot(&cfg.bot_token, handler) => res.map_err(|e| format!("Discord bot stopped: {e}")),
        _ = shutdown_signal() => { info!("Shutting down."); Ok(()) }
    }
}

/// Ctrl+C everywhere, plus SIGTERM (what `docker stop` sends) on Unix.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
