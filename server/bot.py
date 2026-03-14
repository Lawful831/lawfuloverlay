"""
LawfulOverlay - Discord Bot Backend
Runs the Discord bot and broadcasts messages to WebSocket clients.
Bot token and sensitive config are kept server-side only — never sent to clients.
"""

import asyncio
import json
import os
import logging
from typing import Set

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
BOT_TOKEN: str = os.environ["DISCORD_BOT_TOKEN"]

_raw_user_ids = os.environ.get("TARGET_USER_IDS", "")
TARGET_USER_IDS: Set[int] = {
    int(uid.strip())
    for uid in _raw_user_ids.split(",")
    if uid.strip().isdigit()
}

_raw_server_id = os.environ.get("TARGET_SERVER_ID", "")
TARGET_SERVER_ID: int | None = (
    int(_raw_server_id.strip()) if _raw_server_id.strip().isdigit() else None
)

WS_HOST: str = os.environ.get("WS_HOST", "0.0.0.0")
WS_PORT: int = int(os.environ.get("WS_PORT", "8765"))

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
    Only listens to messages from specific users in a specific guild.
    Privileged intents used:
      - message_content  → required to read message body (must be enabled in Dev Portal)
    We deliberately do NOT request presences or members intents beyond default.
    """

    async def on_ready(self) -> None:
        logger.info(f"Logged in as {self.user} (ID: {self.user.id})")
        if TARGET_SERVER_ID:
            guild = self.get_guild(TARGET_SERVER_ID)
            if guild:
                logger.info(f"Monitoring guild: {guild.name}")
            else:
                logger.warning(
                    f"Guild {TARGET_SERVER_ID} not found. "
                    "Ensure the bot is a member of the server."
                )
        else:
            logger.warning("TARGET_SERVER_ID is not set — will match all guilds.")

        if not TARGET_USER_IDS:
            logger.warning("TARGET_USER_IDS is empty — no messages will be relayed.")

    async def on_message(self, message: discord.Message) -> None:
        # Ignore the bot's own messages
        if message.author == self.user:
            return

        # Guild filter
        if TARGET_SERVER_ID and (
            not message.guild or message.guild.id != TARGET_SERVER_ID
        ):
            return

        # User filter
        if TARGET_USER_IDS and message.author.id not in TARGET_USER_IDS:
            return

        username = message.author.display_name
        content = message.content

        logger.info(f"Relaying message from {username}: {content[:80]}")

        await broadcast(
            {
                "type": "message",
                "username": username,
                "content": content,
                "user_id": str(message.author.id),
                "channel": getattr(message.channel, "name", "DM"),
            }
        )


# ── Entry point ───────────────────────────────────────────────────────────────
async def main() -> None:
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


if __name__ == "__main__":
    asyncio.run(main())
