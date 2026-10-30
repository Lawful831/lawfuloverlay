//! Server configuration, read from the environment (and `.env`).

use std::collections::HashSet;
use std::env;
use std::fmt::Display;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Config {
    pub bot_token: String,
    pub target_user_ids: HashSet<u64>,
    pub target_server_ids: HashSet<u64>,

    pub ws_host: String,
    pub ws_port: u16,

    /// The socket is public on purpose. These limits are what keep "public"
    /// from meaning "free denial of service and free read of the stream".
    pub ws_token: String,
    pub ws_origins: HashSet<String>,
    pub max_clients: usize,
    pub max_per_ip: usize,
    pub max_frame_bytes: usize,
    pub max_frames_per_s: u32,
    pub tls_cert: String,
    pub tls_key: String,
}

fn var(name: &str) -> String {
    env::var(name).unwrap_or_default()
}

/// Comma-separated numeric IDs; anything that isn't all digits is skipped.
pub fn parse_id_list(raw: &str) -> HashSet<u64> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|s| s.parse().ok())
        .collect()
}

fn parse_origins(raw: &str) -> HashSet<String> {
    raw.split(',')
        .map(|o| o.trim().trim_end_matches('/'))
        .filter(|o| !o.is_empty())
        .map(str::to_string)
        .collect()
}

fn number<T>(name: &str, default: T) -> Result<T, String>
where
    T: FromStr + Display,
    T::Err: Display,
{
    match env::var(name) {
        Ok(v) if !v.trim().is_empty() => v
            .trim()
            .parse()
            .map_err(|e| format!("{name}={v:?} is not valid (default {default}): {e}")),
        _ => Ok(default),
    }
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        // Accepts a single ID or a comma-separated list, under either name.
        let server_ids = [var("TARGET_SERVER_IDS"), var("TARGET_SERVER_ID")]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(",");

        let ws_host = match var("WS_HOST") {
            h if h.is_empty() => "0.0.0.0".to_string(),
            h => h,
        };

        Ok(Self {
            bot_token: var("DISCORD_BOT_TOKEN"),
            target_user_ids: parse_id_list(&var("TARGET_USER_IDS")),
            target_server_ids: parse_id_list(&server_ids),
            ws_host,
            ws_port: number("WS_PORT", 8765)?,
            ws_token: var("WS_TOKEN"),
            ws_origins: parse_origins(&var("WS_ORIGINS")),
            max_clients: number("WS_MAX_CLIENTS", 25)?,
            max_per_ip: number("WS_MAX_PER_IP", 3)?,
            max_frame_bytes: number("WS_MAX_FRAME_BYTES", 64 * 1024)?,
            max_frames_per_s: number("WS_MAX_FRAMES_PER_S", 20)?,
            tls_cert: var("WS_TLS_CERT"),
            tls_key: var("WS_TLS_KEY"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_lists() {
        let ids = parse_id_list(" 12, 34 ,x,,5a, 678 ");
        assert_eq!(ids, HashSet::from([12, 34, 678]));
        assert!(parse_id_list("").is_empty());
    }

    #[test]
    fn origins_are_normalised() {
        let o = parse_origins("https://a.example/, https://b.example ,");
        assert!(o.contains("https://a.example") && o.contains("https://b.example"));
        assert_eq!(o.len(), 2);
    }
}
