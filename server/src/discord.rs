//! Discord side: listen for messages from the target users in the target
//! guilds and hand them to the hub.

use std::collections::HashSet;
use std::sync::Arc;

use serenity::all::{
    Cache, ChannelId, Client, Context, EventHandler, GatewayIntents, GuildId, Message, Ready,
};
use serenity::async_trait;
use tracing::{info, warn};

use crate::text::{display_text, speech_text, Attachment, Embed};
use crate::ws::{Hub, OverlayMessage};

pub struct Handler {
    pub hub: Arc<Hub>,
    pub user_ids: HashSet<u64>,
    pub server_ids: HashSet<u64>,
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
                .or_else(|| guild.threads.iter().find(|t| t.id == channel).map(|t| t.name.clone()))
        })
        .unwrap_or_else(|| "DM".to_string())
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!("Logged in as {} (ID: {})", ready.user.name, ready.user.id);
        if self.server_ids.is_empty() {
            warn!("TARGET_SERVER_IDS is not set — will match all guilds.");
        }
        if self.user_ids.is_empty() {
            warn!("TARGET_USER_IDS is empty — no messages will be relayed.");
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
            })
            .collect();
        let embeds: Vec<Embed> = msg
            .embeds
            .iter()
            .map(|e| Embed {
                kind: e.kind.clone(),
                has_image: e.image.is_some() || e.thumbnail.is_some(),
            })
            .collect();

        let speech = speech_text(&attachments, &embeds, content);
        let display = display_text(&attachments, &embeds, content);
        let shown = if display.is_empty() { speech.clone() } else { display };

        info!(
            "Relaying message from {username}: {}",
            shown.chars().take(80).collect::<String>()
        );

        self.hub.broadcast(&OverlayMessage::new(
            username,
            shown,
            Some(speech),
            msg.author.id.to_string(),
            channel_name(&ctx.cache, msg.guild_id, msg.channel_id),
        ));
    }
}

pub async fn run_bot(token: &str, handler: Handler) -> Result<(), serenity::Error> {
    // MESSAGE_CONTENT is privileged: it must be toggled ON in the Dev Portal.
    // No presences or members intents are requested.
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;
    info!("Starting Discord bot…");
    let mut client = Client::builder(token, intents)
        .event_handler(handler)
        .await?;
    client.start().await
}
