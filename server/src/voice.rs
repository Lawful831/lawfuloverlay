//! Discord voice channel management and MP3 playback via Songbird.

use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

use serde::{Deserialize, Serialize};
use serenity::all::{ChannelId, Context, GuildId};
use songbird::input::Input;
use tracing::warn;

use crate::fish::MAX_CHARS;
use crate::text::VOICE_ID_LEN;

#[derive(Serialize, Deserialize)]
struct VoiceState {
    voice: String,
}

fn is_valid_voice_id(id: &str) -> bool {
    id.len() == VOICE_ID_LEN && id.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Debug)]
pub struct DefaultVoice {
    current: RwLock<String>,
    fallback: String,
    path: PathBuf,
}

impl DefaultVoice {
    /// `fallback` es FISH_VOICE del entorno; `path` viene de VOICE_STATE_PATH
    /// (default "/app/state/voice.json"). Si el archivo existe y trae un id válido,
    /// ese gana sobre el entorno.
    pub fn new(fallback: String, path: PathBuf) -> Self {
        let initial = match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<VoiceState>(&raw) {
                Ok(st) if st.voice.is_empty() || st.voice == "default" => fallback.clone(),
                Ok(st) if is_valid_voice_id(&st.voice) => st.voice.to_ascii_lowercase(),
                Ok(st) => {
                    warn!(
                        "Ignoring invalid voice id {:?} in {}",
                        st.voice,
                        path.display()
                    );
                    fallback.clone()
                }
                Err(e) => {
                    warn!("Ignoring corrupt voice state file {}: {e}", path.display());
                    fallback.clone()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => fallback.clone(),
            Err(e) => {
                warn!("Could not read voice state file {}: {e}", path.display());
                fallback.clone()
            }
        };
        Self {
            current: RwLock::new(initial),
            fallback,
            path,
        }
    }

    pub fn get(&self) -> String {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Valida (32 hex o "default"), escribe el archivo (creando el directorio) y
    /// actualiza memoria. "default" borra el id guardado y vuelve al del entorno.
    pub fn set(&self, id: &str) -> Result<String, String> {
        let (stored, next) = if id == "default" {
            (String::new(), self.fallback.clone())
        } else if is_valid_voice_id(id) {
            let low = id.to_ascii_lowercase();
            (low.clone(), low)
        } else {
            return Err(
                "Id inválido: se esperan 32 caracteres hexadecimales (o `default`).".to_string(),
            );
        };

        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!("No se pudo crear el directorio {}: {e}", parent.display())
                })?;
            }
        }
        let json = serde_json::to_string(&VoiceState { voice: stored })
            .map_err(|e| format!("No se pudo serializar el estado de voz: {e}"))?;
        std::fs::write(&self.path, json)
            .map_err(|e| format!("No se pudo guardar {}: {e}", self.path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = next.clone();
        Ok(next)
    }
}

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

    const ID: &str = "ca3007f96ae7499ab87d27ea3599956a";
    const ID2: &str = "9a9cf47702da476aa4629e2506d4a857";

    fn temp_state_path(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lawful-overlay-voice-test-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("sub").join("voice.json")
    }

    #[test]
    fn default_voice_persists_validates_and_resets() {
        let path = temp_state_path("persist");
        let dv = DefaultVoice::new(ID.to_string(), path.clone());
        assert_eq!(dv.get(), ID);

        // Set a new voice id (uppercase normalized to lowercase).
        let set_res = dv.set(&ID2.to_uppercase()).unwrap();
        assert_eq!(set_res, ID2);
        assert_eq!(dv.get(), ID2);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!(r#"{{"voice":"{ID2}"}}"#)
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        // Reloading from disk picks the saved id over the fallback.
        let reloaded = DefaultVoice::new(ID.to_string(), path.clone());
        assert_eq!(reloaded.get(), ID2);

        // Invalid ids are rejected without changing memory or disk.
        for bad in ["", "xyz", &ID[..31], &format!("{ID}0")] {
            let err = dv.set(bad).unwrap_err();
            assert_eq!(
                err,
                "Id inválido: se esperan 32 caracteres hexadecimales (o `default`)."
            );
            assert_eq!(dv.get(), ID2);
        }

        // "default" clears the saved id and restores fallback.
        assert_eq!(dv.set("default").unwrap(), ID);
        assert_eq!(dv.get(), ID);
        let after_reset = DefaultVoice::new(ID.to_string(), path.clone());
        assert_eq!(after_reset.get(), ID);

        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn corrupt_or_invalid_file_falls_back_cleanly() {
        let path = temp_state_path("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        std::fs::write(&path, "{not json").unwrap();
        let dv = DefaultVoice::new(ID.to_string(), path.clone());
        assert_eq!(dv.get(), ID);

        std::fs::write(&path, r#"{"voice":"not-a-hex-id"}"#).unwrap();
        let dv2 = DefaultVoice::new(ID.to_string(), path.clone());
        assert_eq!(dv2.get(), ID);

        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }
}
