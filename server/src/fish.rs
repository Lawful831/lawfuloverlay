//! Fish Audio text to speech, called from the server so the API key never
//! reaches an overlay client.
//!
//! Models are tried in order: the paid one, then the free one when the paid
//! one is refused (no credit, rate limit, overload). After a refusal the paid
//! model is skipped for a while. Errors never contain the key.

use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;

pub const TTS_URL: &str = "https://api.fish.audio/v1/tts";
/// Longest text the server will send to Fish Audio, in characters.
pub const MAX_CHARS: usize = 500;
/// Fish Audio calls the server makes per UTC day, across all clients.
pub const DAILY_CALLS: u32 = 300;

const MODELS: [&str; 2] = ["s2.1-pro", "s2.1-pro-free"];
const PAID_RETRY: Duration = Duration::from_secs(600);
const TIMEOUT: Duration = Duration::from_secs(60);
const MAX_AUDIO: u64 = 32 << 20;

pub struct Fish {
    key: String,
    endpoint: String,
    daily_limit: u32,
    agent: ureq::Agent,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    paid_refused: Option<Instant>,
    day: u64,
    calls: u32,
}

/// Why one model gave no audio. The message never contains the key.
#[derive(Debug)]
struct FishError {
    /// HTTP status, or 0 when the service was not reached.
    status: u16,
    message: String,
}

impl FishError {
    /// A bad key (401) or a bad voice id or request (400, 404, 422) fails the
    /// same way on the free model; anything else is worth one more try there.
    fn free_model_may_work(&self) -> bool {
        self.status != 0 && !matches!(self.status, 400 | 401 | 404 | 422)
    }
}

impl fmt::Display for FishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            0 => write!(f, "{}", self.message),
            status => write!(f, "HTTP {status} {}", self.message),
        }
    }
}

impl fmt::Debug for Fish {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fish")
            .field("endpoint", &self.endpoint)
            .field("daily_limit", &self.daily_limit)
            .finish_non_exhaustive()
    }
}

/// Pick the TLS crypto backend for the whole process: several are compiled
/// in, and rustls will not choose one by itself. Call once at start-up.
pub fn install_tls_provider() {
    let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();
}

impl Fish {
    /// `None` when no key is set: the server then offers no Fish Audio voice.
    pub fn new(key: &str) -> Option<Self> {
        Self::with_endpoint(key, TTS_URL, DAILY_CALLS)
    }

    /// Like [`Fish::new`], with another endpoint and daily cap (tests point it at a local fake).
    pub fn with_endpoint(key: &str, endpoint: &str, daily_limit: u32) -> Option<Self> {
        let key = key.trim();
        if key.is_empty() {
            return None;
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        Some(Self {
            key: key.to_string(),
            endpoint: endpoint.to_string(),
            daily_limit,
            agent,
            state: Mutex::default(),
        })
    }

    /// Speech as mp3. `voice` is a Fish Audio model id (`reference_id`); empty
    /// uses the service's default voice. Blocking: call it off the async runtime.
    pub fn synthesize(&self, text: &str, voice: &str) -> Result<Vec<u8>, String> {
        self.take_daily_call()?;
        let skip_paid = {
            let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.paid_refused.is_some_and(|at| at.elapsed() < PAID_RETRY)
        };
        let mut last = String::new();
        for (i, model) in MODELS.into_iter().enumerate().skip(usize::from(skip_paid)) {
            match self.request(text, voice, model) {
                Ok(audio) => return Ok(audio),
                Err(e) => {
                    last = format!("{model}: {e}");
                    if !e.free_model_may_work() {
                        break;
                    }
                    if i == 0 {
                        tracing::warn!("Fish Audio refused the paid model: {last}");
                        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
                        st.paid_refused = Some(Instant::now());
                    }
                }
            }
        }
        Err(last)
    }

    /// Counts a call against today's cap (UTC day), or refuses it.
    fn take_daily_call(&self) -> Result<(), String> {
        let day = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() / 86_400)
            .unwrap_or(0);
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.day != day {
            st.day = day;
            st.calls = 0;
        }
        if st.calls >= self.daily_limit {
            return Err("daily limit reached".into());
        }
        st.calls += 1;
        Ok(())
    }

    fn request(&self, text: &str, voice: &str, model: &str) -> Result<Vec<u8>, FishError> {
        let mut body = json!({ "text": text, "format": "mp3", "mp3_bitrate": 128 });
        if !voice.is_empty() {
            body["reference_id"] = voice.into();
        }
        let fail = |status: u16, message: String| FishError {
            status,
            message: message.replace(&self.key, "***"),
        };
        let mut resp = self
            .agent
            .post(&self.endpoint)
            .header("Authorization", &format!("Bearer {}", self.key))
            .header("Content-Type", "application/json")
            .header("model", model)
            .send(body.to_string())
            .map_err(|e| fail(0, e.to_string()))?;
        let status = resp.status().as_u16();
        if status != 200 {
            // `{"status": 402, "message": "..."}`
            let detail = resp
                .body_mut()
                .with_config()
                .limit(4096)
                .read_to_string()
                .unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&detail)
                .ok()
                .and_then(|v| {
                    v.get("message")
                        .and_then(|m| m.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_default();
            return Err(fail(status, message));
        }
        let audio = resp
            .body_mut()
            .with_config()
            .limit(MAX_AUDIO)
            .read_to_vec()
            .map_err(|e| fail(0, e.to_string()))?;
        if audio.is_empty() {
            return Err(fail(0, "empty audio".into()));
        }
        Ok(audio)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_means_no_fish() {
        assert!(Fish::new("").is_none());
        assert!(Fish::new("  \n").is_none());
        assert!(Fish::new("sk-test").is_some());
    }

    /// Reaches the real service over TLS with a made-up key: expects a refusal, spends nothing.
    #[test]
    #[ignore = "needs network"]
    fn live_service_refuses_a_made_up_key_without_leaking_it() {
        install_tls_provider();
        let key = "not-a-real-key-0123456789";
        let err = Fish::new(key)
            .unwrap()
            .synthesize("hola", "")
            .expect_err("must be refused");
        assert!(err.contains("HTTP 40"), "{err}");
        assert!(!err.contains(key), "{err}");
    }

    #[test]
    fn debug_output_hides_the_key() {
        let fish = Fish::new("sk-very-secret-value").unwrap();
        assert!(!format!("{fish:?}").contains("sk-very-secret-value"));
    }

    #[test]
    fn free_model_is_tried_when_the_paid_one_is_refused() {
        let err = |status| FishError {
            status,
            message: String::new(),
        };
        for status in [402, 403, 429, 500, 503] {
            assert!(err(status).free_model_may_work(), "{status}");
        }
        for status in [0, 400, 401, 404, 422] {
            assert!(!err(status).free_model_may_work(), "{status}");
        }
        assert_eq!(MODELS, ["s2.1-pro", "s2.1-pro-free"]);
    }
}
