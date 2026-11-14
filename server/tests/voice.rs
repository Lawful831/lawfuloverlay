//! Unit/integration tests for `voice::speech_for_tts` and `Voice` state.

use lawful_overlay_server::voice::{speech_for_tts, Voice};
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
