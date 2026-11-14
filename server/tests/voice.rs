//! Unit/integration tests for `voice::speech_for_tts` and `Voice` state.

use lawful_overlay_server::text::strip_voice_templates;
use lawful_overlay_server::voice::{speech_for_tts, DefaultVoice, Voice};
use serenity::all::ChannelId;
use songbird::error::{ConnectionError, JoinError};

#[test]
fn speech_for_tts_trims_and_rejects_blank() {
    assert_eq!(speech_for_tts(""), None);
    assert_eq!(speech_for_tts("   \n\t  "), None);
    assert_eq!(
        speech_for_tts("  hola mundo  "),
        Some("hola mundo".to_string())
    );
}

#[test]
fn speech_for_tts_truncates_at_500_chars() {
    let text = format!("  {}  ", "a".repeat(600));
    let out = speech_for_tts(&text).expect("non-empty");
    assert_eq!(out.chars().count(), 500);
    assert_eq!(out, "a".repeat(500));

    let multibyte = "á".repeat(550);
    let out_mb = speech_for_tts(&multibyte).expect("non-empty");
    assert_eq!(out_mb.chars().count(), 500);
}

#[test]
fn voice_channel_tracking() {
    let voice = Voice::new();
    assert_eq!(voice.channel(), None);
    let ch = ChannelId::new(42);
    voice.set_channel(Some(ch));
    assert_eq!(voice.channel(), Some(ch));
    voice.set_channel(None);
    assert_eq!(voice.channel(), None);
}

#[test]
fn join_error_debug_format_includes_inner_details() {
    let dropped = JoinError::Dropped;
    let dropped_dbg = format!("{dropped:?}");
    assert!(!dropped_dbg.is_empty());
    assert!(dropped_dbg.contains("Dropped"), "{dropped_dbg}");

    let driver_err = JoinError::Driver(ConnectionError::TimedOut);
    let msg = format!("no se pudo unir al canal de voz: {driver_err:?}");
    assert!(msg.contains("Driver"), "{msg}");
    assert!(msg.contains("TimedOut"), "{msg}");
}

#[test]
fn one_shot_takes_precedence_over_default_voice() {
    let env_id = "11111111111111111111111111111111";
    let cmd_id = "22222222222222222222222222222222";
    let msg_id = "ca3007f96ae7499ab87d27ea3599956a";

    let path = std::env::temp_dir()
        .join(format!("lawful-voice-integ-{}", std::process::id()))
        .join("voice.json");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());

    let dv = DefaultVoice::new(env_id.to_string(), path.clone());
    assert_eq!(dv.get(), env_id);
    assert_eq!(dv.set(cmd_id).unwrap(), cmd_id);
    assert_eq!(dv.get(), cmd_id);

    let (texto, one_shot) = strip_voice_templates(&format!("//{msg_id}// hola"));
    assert_eq!(texto, "hola");
    let voice = one_shot.unwrap_or_else(|| dv.get());
    assert_eq!(voice, msg_id);

    let (texto2, one_shot2) = strip_voice_templates("hola sin plantilla");
    assert_eq!(texto2, "hola sin plantilla");
    let voice2 = one_shot2.unwrap_or_else(|| dv.get());
    assert_eq!(voice2, cmd_id);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
