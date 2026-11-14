//! Self-diagnostic binary for Discord voice connection via Songbird.

use std::num::NonZeroU64;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lawful_overlay_server::fish::install_tls_provider;
use lawful_overlay_server::voice::{self, Voice};
use serenity::all::{ChannelId, Client, Context, EventHandler, GatewayIntents, Ready};
use serenity::async_trait;
use songbird::SerenityInit;
use tokio::sync::mpsc::{self, UnboundedSender};

const USAGE: &str = "uso: voice-selftest <channel_id>";

struct DiagHandler {
    channel_id: ChannelId,
    voice: Voice,
    started: AtomicBool,
    tx: UnboundedSender<u8>,
}

impl DiagHandler {
    async fn diagnose(&self, ctx: &Context) -> Result<(), String> {
        let channel = self
            .channel_id
            .to_channel(&ctx.http)
            .await
            .map_err(|e| format!("no se pudo obtener el canal {}: {e:?}", self.channel_id))?;
        let guild_id = channel
            .guild()
            .ok_or_else(|| "el canal no pertenece a un servidor".to_string())?
            .guild_id;

        voice::join(ctx, &self.voice, self.channel_id).await?;
        println!("JOIN_OK canal={}", self.channel_id.get());

        tokio::time::sleep(Duration::from_secs(2)).await;

        let manager = songbird::get(ctx)
            .await
            .ok_or_else(|| "songbird no inicializado".to_string())?;
        let call = manager
            .get(guild_id)
            .ok_or_else(|| "no hay llamada de voz activa".to_string())?;
        if !call.lock().await.is_deaf() {
            return Err("el bot no está en estado sordo (self_deaf)".to_string());
        }
        println!("SORDO_OK");

        voice::leave(ctx, &self.voice).await?;
        println!("LEAVE_OK");
        Ok(())
    }
}

#[async_trait]
impl EventHandler for DiagHandler {
    async fn ready(&self, ctx: Context, _ready: Ready) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let code = match self.diagnose(&ctx).await {
            Ok(()) => 0,
            Err(e) => {
                println!("JOIN_FALLÓ: {e:?}");
                1
            }
        };
        let _ = self.tx.send(code);
    }
}

async fn run_diag(token: &str, channel_id: ChannelId) -> u8 {
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_VOICE_STATES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let handler = DiagHandler {
        channel_id,
        voice: Voice::new(),
        started: AtomicBool::new(false),
        tx,
    };

    let mut client = match Client::builder(token, intents)
        .event_handler(handler)
        .register_songbird()
        .await
    {
        Ok(c) => c,
        Err(e) => {
            println!("JOIN_FALLÓ: {e:?}");
            return 1;
        }
    };

    tokio::select! {
        Some(code) = rx.recv() => code,
        res = client.start() => match res {
            Ok(()) => 1,
            Err(e) => {
                println!("JOIN_FALLÓ: {e:?}");
                1
            }
        },
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let token = match std::env::var("DISCORD_BOT_TOKEN") {
        Ok(val) if !val.trim().is_empty() => val.trim().to_string(),
        _ => {
            eprintln!("error: falta la variable de entorno DISCORD_BOT_TOKEN\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let Some(raw_channel) = std::env::args().nth(1) else {
        eprintln!("error: falta el id del canal de voz\n{USAGE}");
        return ExitCode::from(2);
    };
    let Ok(channel_num) = raw_channel.parse::<NonZeroU64>() else {
        eprintln!("error: channel_id inválido '{raw_channel}' (debe ser u64 > 0)\n{USAGE}");
        return ExitCode::from(2);
    };
    let channel_id = ChannelId::from(channel_num);

    install_tls_provider();

    match tokio::time::timeout(Duration::from_secs(45), run_diag(&token, channel_id)).await {
        Ok(code) => ExitCode::from(code),
        Err(_) => {
            println!("TIMEOUT");
            ExitCode::from(3)
        }
    }
}
