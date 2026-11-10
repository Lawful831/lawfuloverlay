//! Text to speech: Fish Audio when this build has a key and a voice id is
//! known, then Edge TTS, then the local Windows voice.
//!
//! Speech runs on its own thread so a slow synthesis never stalls the UI.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::net::Event;
use crate::text::{clean_tts_text, parse_markup, strip_brackets};

const FISH_TTS_URL: &str = "https://api.fish.audio/v1/tts";
/// Tried in order: the paid model, then the free tier when the paid one is
/// refused (no credit, rate limit, overload). Both are in the S2 family,
/// which reads `[cues]` in the text as emotion and tone directions.
const FISH_MODELS: [&str; 2] = ["s2.1-pro", "s2.1-pro-free"];
/// How long the paid model is skipped after it refuses a request.
const FISH_PAID_RETRY: Duration = Duration::from_secs(600);
const FISH_TIMEOUT: Duration = Duration::from_secs(60);
const FISH_MAX_AUDIO: u64 = 32 << 20;

struct Job {
    /// Generation when queued; a job from an older generation is dropped.
    generation: u64,
    speech: Speech,
}

/// One thing to say, prepared for each kind of voice.
#[derive(Debug, Clone, Default, PartialEq)]
struct Speech {
    /// For Fish Audio: `[cues]` kept.
    fish: String,
    /// For the Edge and Windows voices: no `[cues]`, they would be read aloud.
    plain: String,
    /// Fish Audio voice asked for by the message itself (`//id//`).
    voice: Option<String>,
}

impl Speech {
    fn plain(text: &str) -> Self {
        let plain = clean_tts_text(text);
        Self { fish: plain.clone(), plain, voice: None }
    }

    /// A chat message. Only the message is read: the overlay shows who wrote it.
    fn chat(speech: &str) -> Self {
        let markup = parse_markup(speech);
        Self {
            fish: clean_tts_text(&markup.spoken),
            plain: clean_tts_text(&strip_brackets(&markup.spoken)),
            voice: markup.voice,
        }
    }
}

/// The voices chosen in the settings.
#[derive(Debug, Clone, Default)]
struct Voices {
    edge: String,
    /// Fish Audio voice id; empty = Fish only speaks messages that name a voice.
    fish: String,
}

pub struct Tts {
    jobs: Sender<Job>,
    generation: Arc<AtomicU64>,
    failed: Arc<AtomicBool>,
    voices: Arc<Mutex<Voices>>,
}

impl Tts {
    pub fn new(edge_voice: &str, fish_voice: &str, events: Sender<Event>, ctx: egui::Context) -> Self {
        let (jobs, queue) = mpsc::channel::<Job>();
        let generation = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(AtomicBool::new(false));
        let voices = Arc::new(Mutex::new(Voices { edge: edge_voice.into(), fish: fish_voice.into() }));

        let worker = (Arc::clone(&generation), Arc::clone(&voices));
        std::thread::Builder::new()
            .name("tts".into())
            .spawn(move || {
                let (generation, voices) = worker;
                while let Ok(job) = queue.recv() {
                    if job.generation != generation.load(Ordering::SeqCst) {
                        continue;
                    }
                    let voices = voices.lock().map(|v| v.clone()).unwrap_or_default();
                    if !platform::speak(&job.speech, &voices) {
                        let _ = events.send(Event::TtsFailed);
                        ctx.request_repaint();
                    }
                }
            })
            .expect("spawn tts thread");

        Self { jobs, generation, failed, voices }
    }

    /// Queue a notice (cleaned for speech). Ignored after a failure until [`Tts::clear_failure`].
    pub fn speak(&self, text: &str) {
        self.queue(Speech::plain(text));
    }

    /// Queue a chat message (the user name is not read): `//voice id//` picks
    /// its Fish Audio voice and `[cues]` reach Fish Audio only.
    pub fn speak_chat(&self, speech: &str) {
        self.queue(Speech::chat(speech));
    }

    fn queue(&self, speech: Speech) {
        if self.failed.load(Ordering::SeqCst) || speech.fish.is_empty() {
            return;
        }
        let generation = self.generation.load(Ordering::SeqCst);
        let _ = self.jobs.send(Job { generation, speech });
    }

    /// Drop everything still waiting (the sentence being spoken finishes).
    pub fn reset_queue(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    pub fn set_voices(&self, edge_voice: &str, fish_voice: &str) {
        if let Ok(mut v) = self.voices.lock() {
            *v = Voices { edge: edge_voice.into(), fish: fish_voice.into() };
        }
    }

    pub fn mark_failed(&self) {
        self.failed.store(true, Ordering::SeqCst);
    }

    pub fn clear_failure(&self) {
        self.failed.store(false, Ordering::SeqCst);
    }
}

/// `en-US-AriaNeural` → `Microsoft Server Speech Text to Speech Voice (en-US, AriaNeural)`.
/// Names already in long form pass through.
fn full_voice_name(short: &str) -> String {
    match short.rsplit_once('-') {
        Some((locale, name)) if !short.contains(' ') => {
            format!("Microsoft Server Speech Text to Speech Voice ({locale}, {name})")
        }
        _ => short.to_string(),
    }
}

fn synthesize_edge(text: &str, voice: &str) -> Result<Vec<u8>, String> {
    use msedge_tts::tts::client::connect;
    use msedge_tts::tts::SpeechConfig;

    let config = SpeechConfig {
        voice_name: full_voice_name(voice),
        audio_format: "audio-24khz-48kbitrate-mono-mp3".into(),
        pitch: 0,
        rate: 0,
        volume: 0,
    };
    crate::install_tls_provider();
    // A panic inside the service client must not kill the speech thread:
    // report it as a failure so the Windows voice takes over.
    let audio = std::panic::catch_unwind(move || {
        let mut client = connect().map_err(|e| e.to_string())?;
        client.synthesize(text, &config).map_err(|e| e.to_string())
    })
    .map_err(|_| "the Edge voice client panicked".to_string())??;
    if audio.audio_bytes.is_empty() {
        return Err("empty audio".into());
    }
    Ok(audio.audio_bytes)
}

/// Why a Fish Audio request gave no audio. The text never contains the key.
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

impl std::fmt::Display for FishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            0 => write!(f, "{}", self.message),
            status => write!(f, "HTTP {status} {}", self.message),
        }
    }
}

/// Fish Audio speech as mp3: the paid model, then the free one when the paid
/// one is refused. `voice` is a model id (`reference_id`); empty uses the
/// service's default voice.
fn synthesize_fish(text: &str, voice: &str, key: &str) -> Result<Vec<u8>, String> {
    // After a refusal the paid model is left alone for a while, so each
    // message does not wait on a request that is known to fail.
    static PAID_REFUSED: Mutex<Option<Instant>> = Mutex::new(None);
    let refused_at = PAID_REFUSED.lock().ok().and_then(|at| *at);
    let skip_paid = refused_at.is_some_and(|at| at.elapsed() < FISH_PAID_RETRY);

    let mut last = String::new();
    for (i, model) in FISH_MODELS.into_iter().enumerate().skip(usize::from(skip_paid)) {
        match fish_request(text, voice, key, model) {
            Ok(audio) => return Ok(audio),
            Err(e) => {
                last = format!("{model}: {e}");
                if !e.free_model_may_work() {
                    break;
                }
                if i == 0 {
                    eprintln!("[tts] fish audio {last}");
                    if let Ok(mut at) = PAID_REFUSED.lock() {
                        *at = Some(Instant::now());
                    }
                }
            }
        }
    }
    Err(last)
}

fn fish_request(text: &str, voice: &str, key: &str, model: &str) -> Result<Vec<u8>, FishError> {
    let mut body = serde_json::json!({ "text": text, "format": "mp3", "mp3_bitrate": 128 });
    if !voice.is_empty() {
        body["reference_id"] = voice.into();
    }
    crate::install_tls_provider();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(FISH_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let fail = |status: u16, message: String| FishError { status, message: message.replace(key, "***") };
    let mut resp = agent
        .post(FISH_TTS_URL)
        .header("Authorization", &format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .header("model", model)
        .send(body.to_string())
        .map_err(|e| fail(0, e.to_string()))?;
    let status = resp.status().as_u16();
    if status != 200 {
        // `{"status": 402, "message": "..."}`
        let detail = resp.body_mut().with_config().limit(4096).read_to_string().unwrap_or_default();
        let message = serde_json::from_str::<serde_json::Value>(&detail)
            .ok()
            .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string))
            .unwrap_or_default();
        return Err(fail(status, message));
    }
    let audio = resp
        .body_mut()
        .with_config()
        .limit(FISH_MAX_AUDIO)
        .read_to_vec()
        .map_err(|e| fail(0, e.to_string()))?;
    if audio.is_empty() {
        return Err(fail(0, "empty audio".into()));
    }
    Ok(audio)
}

#[cfg(windows)]
mod platform {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::core::PCWSTR;
    use windows::Win32::Media::Speech::{ISpVoice, SpVoice, SPF_IS_NOT_XML};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows_sys::Win32::Media::Multimedia::mciSendStringW;

    use super::{synthesize_edge, synthesize_fish, Speech, Voices};
    use crate::config::fish_api_key;

    /// Fish Audio (the message's own voice, else the one in the settings),
    /// then the Edge voice, then the Windows voice.
    pub fn speak(speech: &Speech, voices: &Voices) -> bool {
        let fish_voice = speech.voice.as_deref().unwrap_or(&voices.fish);
        if !fish_voice.is_empty() {
            if let Some(key) = fish_api_key() {
                if fish(&speech.fish, fish_voice, &key) {
                    return true;
                }
            }
        }
        // Nothing left once the cues are gone: not a failure.
        speech.plain.is_empty() || edge(&speech.plain, &voices.edge) || windows_voice(&speech.plain)
    }

    fn mci(cmd: &str) -> Result<(), String> {
        let wide: Vec<u16> = std::ffi::OsStr::new(cmd).encode_wide().chain(Some(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call; no return buffer is requested.
        let err = unsafe { mciSendStringW(wide.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut()) };
        if err == 0 {
            Ok(())
        } else {
            Err(format!("mci error {err}"))
        }
    }

    fn play_mp3(path: &Path) -> Result<(), String> {
        let alias = format!("tts_{:?}", std::thread::current().id()).replace(|c: char| !c.is_alphanumeric(), "_");
        mci(&format!("open \"{}\" type mpegvideo alias {alias}", path.display()))?;
        let played = mci(&format!("play {alias} wait"));
        let _ = mci(&format!("close {alias}"));
        played
    }

    fn fish(text: &str, voice: &str, key: &str) -> bool {
        match synthesize_fish(text, voice, key) {
            Ok(audio) => play(audio),
            Err(e) => {
                eprintln!("[tts] fish audio {e}");
                false
            }
        }
    }

    fn edge(text: &str, voice: &str) -> bool {
        match synthesize_edge(text, voice) {
            Ok(audio) => play(audio),
            Err(e) => {
                eprintln!("[tts] edge: {e}");
                false
            }
        }
    }

    fn play(audio: Vec<u8>) -> bool {
        let path = std::env::temp_dir().join(format!("lawful_tts_{}.mp3", std::process::id()));
        let played = std::fs::write(&path, audio)
            .map_err(|e| e.to_string())
            .and_then(|()| play_mp3(&path));
        let _ = std::fs::remove_file(&path);
        match played {
            Ok(()) => true,
            Err(e) => {
                eprintln!("[tts] playback: {e}");
                false
            }
        }
    }

    /// The voice built into Windows (SAPI), called in-process through COM.
    /// No child process is started: a hidden PowerShell is exactly what
    /// antivirus heuristics look for.
    pub(super) fn windows_voice(text: &str) -> bool {
        // Chat text is untrusted, so it is spoken as plain text, never as SAPI XML.
        let wide: Vec<u16> = text.encode_utf16().filter(|&c| c != 0).chain(Some(0)).collect();
        // SAFETY: COM is initialised on this thread before use and balanced
        // with CoUninitialize only when this call initialised it; the voice is
        // released before that; `wide` is NUL-terminated and outlives `Speak`.
        unsafe {
            // Fails with RPC_E_CHANGED_MODE if the thread already uses another
            // apartment model; COM is usable either way.
            let initialised = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let spoken = CoCreateInstance::<_, ISpVoice>(&SpVoice, None, CLSCTX_ALL)
                .and_then(|voice| voice.Speak(PCWSTR(wide.as_ptr()), SPF_IS_NOT_XML.0 as u32, None));
            if initialised {
                CoUninitialize();
            }
            match spoken {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("[tts] windows voice: {e}");
                    false
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn speak(_speech: &super::Speech, _voices: &super::Voices) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_names() {
        assert_eq!(
            full_voice_name("en-US-AriaNeural"),
            "Microsoft Server Speech Text to Speech Voice (en-US, AriaNeural)"
        );
        assert_eq!(
            full_voice_name("es-MX-JorgeNeural"),
            "Microsoft Server Speech Text to Speech Voice (es-MX, JorgeNeural)"
        );
        let long = "Microsoft Server Speech Text to Speech Voice (en-US, AriaNeural)";
        assert_eq!(full_voice_name(long), long);
    }

    /// Speaks out loud through the built-in Windows voice.
    #[cfg(windows)]
    #[test]
    #[ignore = "plays audio"]
    fn windows_voice_speaks() {
        assert!(platform::windows_voice("<prueba> uno & dos"));
    }

    /// Speaks out loud through the whole chain the app uses (Edge voice first).
    #[cfg(windows)]
    #[test]
    #[ignore = "plays audio, needs network"]
    fn speaks_like_the_app() {
        let voices = Voices { edge: "es-MX-DaliaNeural".into(), fish: String::new() };
        assert!(platform::speak(&Speech::plain("Texto a voz activado"), &voices));
    }

    #[test]
    fn chat_speech_keeps_cues_for_fish_only() {
        let id = "ca3007f96ae7499ab87d27ea3599956a";
        let s = Speech::chat(&format!("//{id}// [happy] hola \u{1F602} [laughing]"));
        assert_eq!(s.fish, "[happy] hola [laughing]");
        assert_eq!(s.plain, "hola");
        assert_eq!(s.voice.as_deref(), Some(id));

        // No user name is read, and a message that is only a cue says nothing on plain voices.
        let s = Speech::chat("hola");
        assert_eq!((s.fish.as_str(), s.plain.as_str(), s.voice), ("hola", "hola", None));
        assert_eq!(Speech::chat("[laughing]").plain, "");
    }

    #[test]
    fn free_model_is_tried_when_the_paid_one_is_refused() {
        let err = |status| FishError { status, message: String::new() };
        for status in [402, 403, 429, 500, 503] {
            assert!(err(status).free_model_may_work(), "{status}");
        }
        for status in [0, 400, 401, 404, 422] {
            assert!(!err(status).free_model_may_work(), "{status}");
        }
        assert_eq!(FISH_MODELS, ["s2.1-pro", "s2.1-pro-free"]);
    }

    /// The key baked into this build, or `LAWFUL_FISH_API_KEY` at run time.
    fn fish_test_key() -> String {
        crate::config::fish_api_key()
            .or_else(|| std::env::var("LAWFUL_FISH_API_KEY").ok().filter(|k| !k.trim().is_empty()))
            .expect("set LAWFUL_FISH_API_KEY to run this test")
            .trim()
            .to_string()
    }

    /// Hits the live Fish Audio service (may spend credit). `LAWFUL_FISH_TEST_VOICE`
    /// picks the voice id; without it the service's default voice answers.
    #[test]
    #[ignore = "needs network and a Fish Audio key"]
    fn fish_service_returns_mp3() {
        let voice = std::env::var("LAWFUL_FISH_TEST_VOICE").unwrap_or_default();
        let audio = synthesize_fish("[happy] Hola, esto es una prueba", &voice, &fish_test_key()).expect("synthesis");
        assert!(audio.len() > 1000, "only {} bytes", audio.len());
    }

    /// The free model on its own, as used when the paid one is refused.
    #[test]
    #[ignore = "needs network and a Fish Audio key"]
    fn fish_free_model_returns_mp3() {
        let audio = fish_request("Hola, esto es una prueba", "", &fish_test_key(), FISH_MODELS[1]).expect("synthesis");
        assert!(audio.len() > 1000, "only {} bytes", audio.len());
    }

    #[test]
    #[ignore = "needs network"]
    fn fish_errors_do_not_leak_the_key() {
        let key = "not-a-real-key-0123456789";
        let err = synthesize_fish("hola", "", key).expect_err("a made-up key must be refused");
        assert!(err.contains("HTTP 40"), "{err}");
        assert!(!err.contains(key), "{err}");
    }

    /// Hits the live Edge service: `cargo test -p lawful-overlay -- --ignored`.
    #[test]
    #[ignore = "needs network"]
    fn edge_service_returns_mp3() {
        let audio = synthesize_edge("Hola, esto es una prueba", "es-MX-DaliaNeural").expect("synthesis");
        assert!(audio.len() > 1000, "only {} bytes", audio.len());
    }
}
