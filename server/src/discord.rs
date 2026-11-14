//! Discord side: listen for messages from the target users in the target
//! guilds and hand them to the hub.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serenity::all::{
    Cache, ChannelId, Client, CommandOptionType, Context, CreateCommand, CreateCommandOption,
    EditInteractionResponse, EventHandler, GatewayIntents, GuildId, Interaction, Message, Ready,
};
use serenity::async_trait;
use songbird::SerenityInit;
use tracing::{info, warn};

use crate::fish::Fish;
use crate::text::{
    display_text, image_urls, link_urls, speech_text, strip_voice_templates, Attachment, Embed,
};
use crate::voice::{self, speech_for_tts, DefaultVoice, Voice};
use crate::ws::{Hub, OverlayMessage};

static COMMANDS_REGISTERED: AtomicBool = AtomicBool::new(false);

pub struct Handler {
    pub hub: Arc<Hub>,
    pub user_ids: HashSet<u64>,
    pub server_ids: HashSet<u64>,
    pub fish: Option<Arc<Fish>>,
    pub default_voice: DefaultVoice,
    pub voice: Arc<Voice>,
}

impl Handler {
    /// Guild filter, then user filter. Empty sets mean "match all".
    fn wanted(&self, msg: &Message) -> bool {
        if !self.server_ids.is_empty()
            && !msg
                .guild_id
                .is_some_and(|g| self.server_ids.contains(&g.get()))
        {
            return false;
        }
        self.user_ids.is_empty() || self.user_ids.contains(&msg.author.id.get())
    }
}

fn channel_name(cache: &Cache, guild: Option<GuildId>, channel: ChannelId) -> String {
    guild
        .and_then(|gid| {
            let guild = cache.guild(gid)?;
            guild
                .channels
                .get(&channel)
                .map(|c| c.name.clone())
                .or_else(|| {
                    guild
                        .threads
                        .iter()
                        .find(|t| t.id == channel)
                        .map(|t| t.name.clone())
                })
        })
        .unwrap_or_else(|| "DM".to_string())
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("Logged in as {} (ID: {})", ready.user.name, ready.user.id);
        if self.server_ids.is_empty() {
            warn!("TARGET_SERVER_IDS is not set — will match all guilds.");
        }
        if self.user_ids.is_empty() {
            warn!("TARGET_USER_IDS is empty — no messages will be relayed.");
        }
        if !COMMANDS_REGISTERED.swap(true, Ordering::Relaxed) {
            // Comandos por guild, no globales: los globales tardan hasta una hora
            // en propagarse y el usuario probaría /join con el comando invisible.
            let commands = [
                (
                    "join",
                    CreateCommand::new("join")
                        .description("Entra a tu canal de voz y lee los mensajes en voz alta"),
                ),
                (
                    "leave",
                    CreateCommand::new("leave").description("Sale del canal de voz"),
                ),
                (
                    "voice",
                    CreateCommand::new("voice")
                        .description("Cambia la voz por defecto de Fish Audio")
                        .add_option(
                            CreateCommandOption::new(
                                CommandOptionType::String,
                                "id",
                                "Id de 32 caracteres hexadecimales o default",
                            )
                            .required(true),
                        ),
                ),
            ];
            for guild_id in ctx.cache.guilds() {
                for (name, builder) in &commands {
                    if let Err(e) = guild_id.create_command(&ctx.http, builder.clone()).await {
                        warn!("Could not register /{name} in guild {guild_id}: {e}");
                    }
                }
            }
        }
    }

    async fn cache_ready(&self, ctx: Context, _guilds: Vec<GuildId>) {
        let mut ids: Vec<u64> = self.server_ids.iter().copied().collect();
        ids.sort_unstable();
        for id in ids {
            match ctx.cache.guild(GuildId::new(id)) {
                Some(guild) => info!("Monitoring guild: {}", guild.name),
                None => warn!("Guild {id} not found. Ensure the bot is a member of the server."),
            }
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Command(cmd) = interaction else {
            return;
        };
        if !matches!(cmd.data.name.as_str(), "join" | "leave" | "voice") {
            return;
        }
        // Unirse a voz puede tardar más de los 3 s que Discord da para responder;
        // sin diferir, el usuario ve "Unknown interaction" y el resultado se pierde.
        if let Err(e) = cmd.defer_ephemeral(&ctx.http).await {
            warn!("Could not defer interaction for /{}: {e}", cmd.data.name);
            return;
        }
        let reply = match cmd.data.name.as_str() {
            "join" => {
                let user_channel = cmd.guild_id.and_then(|gid| {
                    let guild = ctx.cache.guild(gid)?;
                    guild.voice_states.get(&cmd.user.id)?.channel_id
                });
                match user_channel {
                    None => "Entra a un canal de voz primero.".to_string(),
                    Some(ch_id) => {
                        let ch_name = channel_name(&ctx.cache, cmd.guild_id, ch_id);
                        match voice::join(&ctx, &self.voice, ch_id).await {
                            Ok(()) if self.fish.is_none() => format!(
                                "Entré a **{ch_name}**, pero la voz está apagada (falta FISH_API_KEY)."
                            ),
                            Ok(()) => format!("Entré a **{ch_name}**, sordo y listo para leer."),
                            Err(e) => {
                                warn!("Failed to join voice channel {ch_id}: {e}");
                                format!("No pude entrar a **{ch_name}**: {e}")
                            }
                        }
                    }
                }
            }
            "leave" => {
                if self.voice.channel().is_none() {
                    "No estoy en ningún canal.".to_string()
                } else {
                    if let Err(e) = voice::leave(&ctx, &self.voice).await {
                        warn!("Failed to leave voice channel: {e}");
                    }
                    "Salí del canal de voz.".to_string()
                }
            }
            "voice" => {
                let raw_id = cmd
                    .data
                    .options
                    .iter()
                    .find(|o| o.name == "id")
                    .and_then(|o| o.value.as_str())
                    .unwrap_or("");
                match self.default_voice.set(raw_id) {
                    Ok(v) if raw_id == "default" => {
                        format!("Voz por defecto: la del servidor (`{v}`).")
                    }
                    Ok(v) => format!("Voz por defecto: `{v}`."),
                    Err(e) => e,
                }
            }
            _ => return,
        };

        if let Err(e) = cmd
            .edit_response(&ctx.http, EditInteractionResponse::new().content(reply))
            .await
        {
            warn!(
                "Could not send interaction response for /{}: {e}",
                cmd.data.name
            );
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.id == ctx.cache.current_user().id || !self.wanted(&msg) {
            return;
        }

        let username = msg
            .member
            .as_ref()
            .and_then(|m| m.nick.clone())
            .or_else(|| msg.author.global_name.clone())
            .unwrap_or_else(|| msg.author.name.clone());
        // Resolves mentions to names.
        let content = msg.content_safe(&ctx.cache);
        let content = content.trim();

        let attachments: Vec<Attachment> = msg
            .attachments
            .iter()
            .map(|a| Attachment {
                content_type: a.content_type.clone(),
                filename: a.filename.clone(),
                url: a.proxy_url.clone(),
            })
            .collect();
        let embeds: Vec<Embed> = msg
            .embeds
            .iter()
            .map(|e| Embed {
                kind: e.kind.clone(),
                has_image: e.image.is_some() || e.thumbnail.is_some(),
                image_url: e
                    .image
                    .as_ref()
                    .and_then(|i| i.proxy_url.clone())
                    .or_else(|| e.thumbnail.as_ref().and_then(|t| t.proxy_url.clone())),
            })
            .collect();

        let speech = speech_text(&attachments, &embeds, content);
        let display = display_text(&attachments, &embeds, content);
        let shown = if display.is_empty() {
            speech.clone()
        } else {
            display
        };
        let images = image_urls(&attachments, &embeds, content);

        info!(
            "Relaying message from {username}: {}",
            shown.chars().take(80).collect::<String>()
        );

        self.hub.broadcast(
            &OverlayMessage::new(
                username,
                shown,
                Some(speech.clone()),
                msg.author.id.to_string(),
                channel_name(&ctx.cache, msg.guild_id, msg.channel_id),
            )
            .with_media(link_urls(content), images),
        );

        if self.voice.channel().is_some() && !speech.is_empty() {
            let (texto, one_shot) = strip_voice_templates(&speech);
            let one_shot = one_shot.or_else(|| strip_voice_templates(content).1);
            let Some(texto) = speech_for_tts(&texto) else {
                return;
            };
            let Some(fish) = self.fish.clone() else {
                return;
            };
            let voice = one_shot.unwrap_or_else(|| self.default_voice.get());
            let playback = Arc::clone(&self.voice);
            tokio::task::spawn(async move {
                let res =
                    tokio::task::spawn_blocking(move || fish.synthesize(&texto, &voice)).await;
                let mp3 = match res {
                    Ok(Ok(mp3)) => mp3,
                    Ok(Err(e)) => {
                        warn!("Fish Audio synthesis failed: {e}");
                        return;
                    }
                    Err(e) => {
                        warn!("Fish Audio synthesis task failed: {e}");
                        return;
                    }
                };
                if let Err(e) = voice::play_mp3(&ctx, &playback, mp3).await {
                    warn!("Voice playback failed: {e}");
                }
            });
        }
    }
}

pub async fn run_bot(token: &str, handler: Handler) -> Result<(), Box<serenity::Error>> {
    // MESSAGE_CONTENT is privileged: it must be toggled ON in the Dev Portal.
    // No presences or members intents are requested.
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_VOICE_STATES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;
    info!("Starting Discord bot…");
    let mut client = Client::builder(token, intents)
        .event_handler(handler)
        .register_songbird()
        .await
        .map_err(Box::new)?;
    client.start().await.map_err(Box::new)
}
