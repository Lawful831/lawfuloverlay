"""
LawfulOverlay - Discord Bot Backend
Runs the Discord bot and broadcasts messages to WebSocket clients.
Bot token and sensitive config are kept server-side only — never sent to clients.
"""

import asyncio
import json
import os
import re
import sys
import logging
from typing import Set
from urllib.parse import urlsplit

import discord
import websockets
from websockets.asyncio.server import ServerConnection, serve as ws_serve
from dotenv import load_dotenv

load_dotenv()

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s - %(message)s",
)
logger = logging.getLogger("lawful-overlay")

# ── Config from environment ──────────────────────────────────────────────────
# Empty means "check at startup" so the module can be imported for the self-check
BOT_TOKEN: str = os.environ.get("DISCORD_BOT_TOKEN", "")

_raw_user_ids = os.environ.get("TARGET_USER_IDS", "")
TARGET_USER_IDS: Set[int] = {
    int(uid.strip())
    for uid in _raw_user_ids.split(",")
    if uid.strip().isdigit()
}

# Accepts a single ID or a comma-separated list, under either variable name
_raw_server_ids = ",".join(
    filter(None, (os.environ.get("TARGET_SERVER_IDS", ""), os.environ.get("TARGET_SERVER_ID", "")))
)
TARGET_SERVER_IDS: Set[int] = {
    int(gid.strip())
    for gid in _raw_server_ids.split(",")
    if gid.strip().isdigit()
}

WS_HOST: str = os.environ.get("WS_HOST", "0.0.0.0")
WS_PORT: int = int(os.environ.get("WS_PORT", "8765"))

# ── Links / attachments → readable text ──────────────────────────────────────
_URL_RE = re.compile(r"https?://[^\s<>]*[^\s<>.,;:!?)\]]|www\.[^\s<>]*[^\s<>.,;:!?)\]]", re.IGNORECASE)

# Recognisable site names; anything else falls back to the domain label
_SITE_NAMES: dict[str, str] = {
    "youtu": "youtube", "youtube": "youtube", "youtube-nocookie": "youtube",
    "x": "twitter", "twitter": "twitter", "t": "twitter",
    "twitch": "twitch", "tiktok": "tiktok", "instagram": "instagram",
    "discord": "discord", "reddit": "reddit", "spotify": "spotify",
    "github": "github", "imgur": "imgur", "tenor": "tenor",
    "facebook": "facebook", "fb": "facebook", "soundcloud": "soundcloud",
    "steamcommunity": "steam", "steampowered": "steam", "pixiv": "pixiv",
}
# Second-level labels that are not the site name (bbc.co.uk → bbc)
_SECOND_LEVEL = {"co", "com", "org", "net", "gov", "ac", "edu"}


def _site_name(url: str) -> str:
    host = urlsplit(url if "//" in url else "//" + url).hostname or url
    labels = host.lower().removeprefix("www.").split(".")
    if len(labels) > 1 and labels[-2] in _SECOND_LEVEL:
        labels = labels[:-1]
    name = labels[-2] if len(labels) > 1 else labels[0]
    return _SITE_NAMES.get(name, name)


def humanize_links(text: str, brackets: bool = False) -> str:
    """"https://youtube.com/watch?v=x" → "Link de youtube" (o "[Link de youtube]")."""
    def repl(m: re.Match) -> str:
        name = f"Link de {_site_name(m.group())}"
        return f"[{name}]" if brackets else name

    return _URL_RE.sub(repl, text)


def _is_gif(att: discord.Attachment) -> bool:
    return (att.content_type or "").lower() == "image/gif" \
        or (att.filename or "").lower().endswith(".gif")


def _media_kind(message: discord.Message, content: str) -> str:
    """'gif' | 'image' | 'file' | '' — what the message carries."""
    embeds = [] if content else message.embeds  # link previews: the text already names them
    if any(_is_gif(a) for a in message.attachments) \
            or any((e.type or "").lower() == "gifv" for e in embeds):
        return "gif"
    if any((a.content_type or "").startswith("image/") for a in message.attachments) \
            or any(e.image or e.thumbnail for e in embeds):
        return "image"
    return "file" if message.attachments else ""


_MEDIA_TEXT = {
    "gif": "Compartido un gif",
    "image": "Compartido una imagen",
    "file": "Compartido un archivo",
}

# Etiquetas para el overlay (texto visible)
_MEDIA_TAG = {"gif": "[GIF]", "image": "[Imagen]", "file": "[Archivo]"}


def _join_parts(parts: list[str]) -> str:
    return " ".join(p for p in parts if p)


def display_text(message: discord.Message, content: str) -> str:
    """Texto del overlay: enlaces y adjuntos etiquetados, con el texto original."""
    parts = [humanize_links(content, brackets=True)] if content else []
    kind = _media_kind(message, content)
    if kind:
        parts.append(_MEDIA_TAG[kind])
    return _join_parts(parts)


def speech_text(message: discord.Message, content: str) -> str:
    """
    Text to be spoken: readable links plus an announcement for media.
    Embed media is only announced when the message has no text of its own,
    so a YouTube link doesn't turn into "Link de youtube Compartido una imagen".
    """
    parts = [humanize_links(content)] if content else []
    kind = _media_kind(message, content)
    if kind:
        parts.append(_MEDIA_TEXT[kind])
    return _join_parts(parts)

# ── WebSocket client registry ────────────────────────────────────────────────
connected_clients: Set[ServerConnection] = set()
clients_lock = asyncio.Lock()


async def register(ws: ServerConnection) -> None:
    async with clients_lock:
        connected_clients.add(ws)
    logger.info(f"Client connected ({ws.remote_address}). Total: {len(connected_clients)}")


async def unregister(ws: ServerConnection) -> None:
    async with clients_lock:
        connected_clients.discard(ws)
    logger.info(f"Client disconnected. Total: {len(connected_clients)}")


async def broadcast(payload: dict) -> None:
    """Send a JSON payload to every connected overlay client."""
    if not connected_clients:
        return
    message = json.dumps(payload)
    async with clients_lock:
        targets = set(connected_clients)  # snapshot to avoid mutation during iteration
    dead: Set[ServerConnection] = set()
    for ws in targets:
        try:
            await ws.send(message)
        except websockets.ConnectionClosed:
            dead.add(ws)
    async with clients_lock:
        connected_clients.difference_update(dead)


# ── WebSocket handler ────────────────────────────────────────────────────────
async def ws_handler(ws: ServerConnection) -> None:
    await register(ws)
    try:
        # Keep the connection alive; we don't expect messages from the client
        async for _ in ws:
            pass
    except websockets.ConnectionClosed:
        pass
    finally:
        await unregister(ws)


# ── Discord bot ───────────────────────────────────────────────────────────────
class OverlayBotClient(discord.Client):
    """
    Minimal Discord client.
    Only listens to messages from specific users in specific guilds.
    Privileged intents used:
      - message_content  → required to read message body (must be enabled in Dev Portal)
    We deliberately do NOT request presences or members intents beyond default.
    """

    async def on_ready(self) -> None:
        logger.info(f"Logged in as {self.user} (ID: {self.user.id})")
        if TARGET_SERVER_IDS:
            for server_id in sorted(TARGET_SERVER_IDS):
                guild = self.get_guild(server_id)
                if guild:
                    logger.info(f"Monitoring guild: {guild.name}")
                else:
                    logger.warning(
                        f"Guild {server_id} not found. "
                        "Ensure the bot is a member of the server."
                    )
        else:
            logger.warning("TARGET_SERVER_IDS is not set — will match all guilds.")

        if not TARGET_USER_IDS:
            logger.warning("TARGET_USER_IDS is empty — no messages will be relayed.")

    async def on_message(self, message: discord.Message) -> None:
        # Ignore the bot's own messages
        if message.author == self.user:
            return

        # Guild filter
        if TARGET_SERVER_IDS and (
            not message.guild or message.guild.id not in TARGET_SERVER_IDS
        ):
            return

        # User filter
        if TARGET_USER_IDS and message.author.id not in TARGET_USER_IDS:
            return

        username = message.author.display_name
        content = message.clean_content.strip()  # resolves mentions and custom emoji
        speech = speech_text(message, content)
        display = display_text(message, content)

        logger.info(f"Relaying message from {username}: {(display or speech)[:80]}")

        await broadcast(
            {
                "type": "message",
                "username": username,
                "content": display or speech,   # overlay text — tagged links/media
                "content_speech": speech,       # what the voice reads out
                "user_id": str(message.author.id),
                "channel": getattr(message.channel, "name", "DM"),
            }
        )


# ── Entry point ───────────────────────────────────────────────────────────────
async def main() -> None:
    if not BOT_TOKEN:
        raise SystemExit("DISCORD_BOT_TOKEN is not set (check server/.env)")

    intents = discord.Intents.default()
    intents.message_content = True  # Privileged — must be toggled ON in Dev Portal

    bot = OverlayBotClient(intents=intents)

    logger.info(f"Starting WebSocket server on ws://{WS_HOST}:{WS_PORT}")
    ws_server = await ws_serve(ws_handler, WS_HOST, WS_PORT)

    logger.info("Starting Discord bot…")
    try:
        await bot.start(BOT_TOKEN)
    finally:
        ws_server.close()
        await ws_server.wait_closed()


def _selfcheck() -> None:
    """python bot.py --selfcheck"""
    assert _site_name("https://www.youtube.com/watch?v=x") == "youtube"
    assert _site_name("https://youtu.be/x") == "youtube"
    assert _site_name("https://music.youtube.com/x") == "youtube"
    assert _site_name("https://x.com/a") == "twitter"
    assert _site_name("https://t.co/z") == "twitter"
    assert _site_name("https://open.spotify.com/track/1") == "spotify"
    assert _site_name("https://www.bbc.co.uk/news") == "bbc"
    assert _site_name("https://ejemplo.ar/x") == "ejemplo"

    assert humanize_links("mira https://youtube.com/watch?v=x") == "mira Link de youtube"
    assert humanize_links("mira https://youtube.com/x.") == "mira Link de youtube."
    assert humanize_links("Link (https://youtu.be/x), ok") == "Link (Link de youtube), ok"
    assert humanize_links("https://.") == "https://."
    assert humanize_links("a www.twitch.tv/b y https://t.co/z") == "a Link de twitch y Link de twitter"
    assert humanize_links("sin enlaces") == "sin enlaces"
    assert humanize_links("mira https://youtube.com/x", brackets=True) == "mira [Link de youtube]"

    class _Att:
        def __init__(self, content_type: str | None, filename: str = "") -> None:
            self.content_type = content_type
            self.filename = filename

    class _Msg:
        def __init__(self, attachments=(), embeds=()) -> None:
            self.attachments = list(attachments)
            self.embeds = list(embeds)

    class _Embed:
        def __init__(self, type: str = "rich") -> None:
            self.type = type
            self.image = self.thumbnail = object()

    assert speech_text(_Msg([_Att("image/png")]), "") == "Compartido una imagen"
    assert speech_text(_Msg([_Att("image/gif", "a.gif")]), "") == "Compartido un gif"
    assert speech_text(_Msg([_Att(None, "a.GIF")]), "") == "Compartido un gif"
    assert speech_text(_Msg([_Att(None, "a.pdf")]), "") == "Compartido un archivo"
    assert speech_text(_Msg([_Att("image/png")]), "https://youtu.be/x") == \
        "Link de youtube Compartido una imagen"
    assert speech_text(_Msg([], [_Embed("gifv")]), "") == "Compartido un gif"
    assert speech_text(_Msg([], [_Embed()]), "") == "Compartido una imagen"
    assert speech_text(_Msg([], [_Embed()]), "https://youtu.be/x") == "Link de youtube"
    assert speech_text(_Msg(), "") == ""

    assert display_text(_Msg(), "mira https://youtube.com/x") == "mira [Link de youtube]"
    assert display_text(_Msg([_Att("image/png")]), "mira esto") == "mira esto [Imagen]"
    assert display_text(_Msg([_Att("image/gif", "a.gif")]), "") == "[GIF]"
    assert display_text(_Msg([_Att(None, "a.pdf")]), "") == "[Archivo]"
    assert display_text(_Msg([], [_Embed("gifv")]), "") == "[GIF]"
    assert display_text(_Msg(), "") == ""
    print("selfcheck OK")


if __name__ == "__main__":
    if "--selfcheck" in sys.argv:
        _selfcheck()
    else:
        asyncio.run(main())
