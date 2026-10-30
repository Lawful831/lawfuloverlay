//! LawfulOverlay server: a Discord bot plus the WebSocket broadcaster the
//! overlay clients connect to. The bot token never leaves this process.

pub mod config;
pub mod discord;
pub mod text;
pub mod ws;

/// Console logging in the same shape the Python server used.
pub fn init_logging() {
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_target(true)
        .init();
}
