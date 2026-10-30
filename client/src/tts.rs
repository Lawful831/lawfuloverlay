//! Text to speech: Edge TTS first, the local Windows voice as a fallback.
//!
//! Speech runs on its own thread so a slow synthesis never stalls the UI.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::net::Event;
use crate::text::clean_tts_text;

struct Job {
    /// Generation when queued; a job from an older generation is dropped.
    generation: u64,
    text: String,
}

pub struct Tts {
    jobs: Sender<Job>,
    generation: Arc<AtomicU64>,
    failed: Arc<AtomicBool>,
    voice: Arc<Mutex<String>>,
}

impl Tts {
    pub fn new(voice: &str, events: Sender<Event>, ctx: egui::Context) -> Self {
        let (jobs, queue) = mpsc::channel::<Job>();
        let generation = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(AtomicBool::new(false));
        let voice = Arc::new(Mutex::new(voice.to_string()));

        let worker = (Arc::clone(&generation), Arc::clone(&voice));
        std::thread::Builder::new()
            .name("tts".into())
            .spawn(move || {
                let (generation, voice) = worker;
                while let Ok(job) = queue.recv() {
                    if job.generation != generation.load(Ordering::SeqCst) || job.text.is_empty() {
                        continue;
                    }
                    let voice = voice.lock().map(|v| v.clone()).unwrap_or_default();
                    if !platform::speak(&job.text, &voice) {
                        let _ = events.send(Event::TtsFailed);
                        ctx.request_repaint();
                    }
                }
            })
            .expect("spawn tts thread");

        Self { jobs, generation, failed, voice }
    }

    /// Queue `text` (cleaned for speech). Ignored after a failure until [`Tts::clear_failure`].
    pub fn speak(&self, text: &str) {
        if self.failed.load(Ordering::SeqCst) {
            return;
        }
        let text = clean_tts_text(text);
        if !text.is_empty() {
            let generation = self.generation.load(Ordering::SeqCst);
            let _ = self.jobs.send(Job { generation, text });
        }
    }

    /// Drop everything still waiting (the sentence being spoken finishes).
    pub fn reset_queue(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    pub fn set_voice(&self, voice: &str) {
        if let Ok(mut v) = self.voice.lock() {
            *v = voice.to_string();
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
    let mut client = connect().map_err(|e| e.to_string())?;
    let audio = client.synthesize(text, &config).map_err(|e| e.to_string())?;
    if audio.audio_bytes.is_empty() {
        return Err("empty audio".into());
    }
    Ok(audio.audio_bytes)
}

#[cfg(windows)]
mod platform {
    use std::io::Write;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Media::Multimedia::mciSendStringW;

    use super::synthesize_edge;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn speak(text: &str, voice: &str) -> bool {
        edge(text, voice) || windows_voice(text)
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

    fn edge(text: &str, voice: &str) -> bool {
        let audio = match synthesize_edge(text, voice) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("[tts] edge: {e}");
                return false;
            }
        };
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

    fn windows_voice(text: &str) -> bool {
        const SCRIPTS: [&str; 2] = [
            "[Console]::InputEncoding=[System.Text.Encoding]::UTF8;$t=[Console]::In.ReadToEnd();Add-Type -AssemblyName System.Speech;\
             $s=New-Object System.Speech.Synthesis.SpeechSynthesizer;$s.Speak($t)",
            "[Console]::InputEncoding=[System.Text.Encoding]::UTF8;$t=[Console]::In.ReadToEnd();\
             $s=New-Object -ComObject SAPI.SpVoice;[void]$s.Speak($t)",
        ];
        for exe in ["powershell.exe", "pwsh.exe"] {
            for script in SCRIPTS {
                if run_powershell(exe, script, text) {
                    return true;
                }
            }
        }
        false
    }

    fn run_powershell(exe: &str, script: &str, input: &str) -> bool {
        let Ok(mut child) = Command::new(exe)
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
        else {
            return false;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input.as_bytes());
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn speak(_text: &str, _voice: &str) -> bool {
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

    /// Hits the live Edge service: `cargo test -p lawful-overlay -- --ignored`.
    #[test]
    #[ignore = "needs network"]
    fn edge_service_returns_mp3() {
        let audio = synthesize_edge("Hola, esto es una prueba", "es-MX-DaliaNeural").expect("synthesis");
        assert!(audio.len() > 1000, "only {} bytes", audio.len());
    }
}
