//! Discord voice channel management and MP3 playback via Songbird.

use std::sync::Mutex;

use serenity::all::{ChannelId, Context, GuildId};
use songbird::input::Input;

use crate::fish::MAX_CHARS;

pub struct Voice {
    channel: Mutex<Option<ChannelId>>,
}

impl Default for Voice {
    fn default() -> Self {
        Self::new()
    }
}

impl Voice {
    pub fn new() -> Self {
        Self {
            channel: Mutex::new(None),
        }
    }

    pub fn channel(&self) -> Option<ChannelId> {
        *self.channel.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_channel(&self, ch: Option<ChannelId>) {
        *self.channel.lock().unwrap_or_else(|e| e.into_inner()) = ch;
    }
}

async fn guild_of(ctx: &Context, channel_id: ChannelId) -> Result<GuildId, String> {
    let channel = channel_id
        .to_channel(&ctx.http)
        .await
        .map_err(|e| format!("no se pudo obtener el canal {channel_id}: {e}"))?;
    let guild_channel = channel
        .guild()
        .ok_or_else(|| "el canal no pertenece a un servidor".to_string())?;
    Ok(guild_channel.guild_id)
}

/// Une el bot al canal, lo deja sordo (self_deaf) y lo registra en Voice.
pub async fn join(ctx: &Context, voice: &Voice, channel_id: ChannelId) -> Result<(), String> {
    let guild_id = guild_of(ctx, channel_id).await?;
    let manager = songbird::get(ctx)
        .await
        .ok_or_else(|| "songbird no inicializado".to_string())?;
    let call = manager
        .join(guild_id, channel_id)
        .await
        .map_err(|e| format!("no se pudo unir al canal de voz: {e:?}"))?;
    call.lock()
        .await
        .deafen(true)
        .await
        .map_err(|e| format!("no se pudo ensordecer en el canal: {e}"))?;
    // Songbird ya manda el Voice State Update con self_deaf: un PATCH extra por
    // REST competiría con su propio socket de voz y puede mover al bot de canal.
    voice.set_channel(Some(channel_id));
    Ok(())
}

/// Sale del canal y limpia el estado.
pub async fn leave(ctx: &Context, voice: &Voice) -> Result<(), String> {
    let Some(channel_id) = voice.channel() else {
        return Ok(());
    };
    voice.set_channel(None);
    let guild_id = guild_of(ctx, channel_id).await?;
    let manager = songbird::get(ctx)
        .await
        .ok_or_else(|| "songbird no inicializado".to_string())?;
    manager
        .remove(guild_id)
        .await
        .map_err(|e| format!("no se pudo salir del canal de voz: {e}"))
}

/// Encola un mp3 en el canal actual. No-op silencioso si no hay canal.
pub async fn play_mp3(ctx: &Context, voice: &Voice, mp3: Vec<u8>) -> Result<(), String> {
    let Some(channel_id) = voice.channel() else {
        return Ok(());
    };
    let guild_id = guild_of(ctx, channel_id).await?;
    let manager = songbird::get(ctx)
        .await
        .ok_or_else(|| "songbird no inicializado".to_string())?;
    let call = manager
        .get(guild_id)
        .ok_or_else(|| "no hay llamada de voz activa".to_string())?;
    let input = Input::from(mp3);
    call.lock().await.enqueue_input(input).await;
    Ok(())
}

/// Texto listo para TTS: recorta a 500 caracteres y devuelve None si queda vacío.
pub fn speech_for_tts(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_whitespace_returns_none() {
        assert_eq!(speech_for_tts(""), None);
        assert_eq!(speech_for_tts("   \n\t  "), None);
    }

    #[test]
    fn trims_and_truncates_to_500_chars() {
        assert_eq!(
            speech_for_tts("  hola mundo  "),
            Some("hola mundo".to_string())
        );
        let long = "ñ".repeat(600);
        let out = speech_for_tts(&long).unwrap();
        assert_eq!(out.chars().count(), 500);
    }

    #[test]
    fn voice_channel_state() {
        let v = Voice::new();
        assert_eq!(v.channel(), None);
        let ch = ChannelId::new(123);
        v.set_channel(Some(ch));
        assert_eq!(v.channel(), Some(ch));
        v.set_channel(None);
        assert_eq!(v.channel(), None);
    }
}
