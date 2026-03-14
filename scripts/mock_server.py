"""
LawfulOverlay — Mock WebSocket Server
======================================
Simulates the production server locally so you can test the overlay client
WITHOUT needing Docker, a Discord bot token, or an internet connection.

Usage:
    python scripts/mock_server.py [--port 8765] [--interval 4]

The server sends fake Discord messages in a loop at a configurable interval,
plus immediately sends a "connected" banner so you can verify the client works.
"""

import asyncio
import json
import random
import argparse
import logging
from typing import Set
import websockets
from websockets.asyncio.server import ServerConnection, serve as ws_serve

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [MOCK] %(message)s",
)
log = logging.getLogger("mock-server")

# ── Fake message pool ──────────────────────────────────────────────────────
FAKE_USERS = ["StreamerDave", "xXGamerXx", "ProPlayer99", "CoolKid2000", "MysticWizard"]
FAKE_MESSAGES = [
    "GG that was insane!",
    "let's go!!!",
    "wait how did you do that move",
    "lmaooo rekt",
    "gg wp no re",
    "that clutch though 👀",
    "POGGERS",
    "bro what was that aim",
    "okay that was actually clean",
    "EZ Clap",
    "are you using hacks lol",
    "teach me sensei 🙏",
    "chat is cooked",
    "I can't believe that worked",
    "speedrun world record incoming?",
]

connected_clients: Set[ServerConnection] = set()
clients_lock = asyncio.Lock()


async def register(ws: ServerConnection) -> None:
    async with clients_lock:
        connected_clients.add(ws)
    log.info(f"Client connected from {ws.remote_address}. Total clients: {len(connected_clients)}")


async def unregister(ws: ServerConnection) -> None:
    async with clients_lock:
        connected_clients.discard(ws)
    log.info(f"Client disconnected. Remaining: {len(connected_clients)}")


async def broadcast(payload: dict) -> None:  # noqa: ANN001
    if not connected_clients:
        return
    msg = json.dumps(payload)
    async with clients_lock:
        targets = set(connected_clients)
    dead: Set[ServerConnection] = set()
    for ws in targets:
        try:
            await ws.send(msg)
        except websockets.ConnectionClosed:
            dead.add(ws)
    async with clients_lock:
        connected_clients.difference_update(dead)


async def ws_handler(ws: ServerConnection) -> None:
    await register(ws)
    # Send a welcome message immediately so the client shows something straight away
    await ws.send(json.dumps({
        "type": "message",
        "username": "ServidorMock",
        "content": "🟢 Servidor mock conectado — mensajes de prueba en camino…",
        "user_id": "0",
        "channel": "prueba",
    }))
    try:
        async for _ in ws:
            pass  # client sends nothing; just keep alive
    except websockets.ConnectionClosed:
        pass
    finally:
        await unregister(ws)


async def message_broadcaster(interval: float) -> None:
    """Sends a random fake message to all clients every `interval` seconds."""
    await asyncio.sleep(2)  # brief warm-up
    while True:
        if connected_clients:
            user = random.choice(FAKE_USERS)
            text = random.choice(FAKE_MESSAGES)
            payload = {
                "type": "message",
                "username": user,
                "content": text,
                "user_id": str(random.randint(100_000_000, 999_999_999)),
                "channel": "general",
            }
            log.info(f"Broadcasting → {user}: {text}")
            await broadcast(payload)
        await asyncio.sleep(interval)


async def main(host: str, port: int, interval: float) -> None:
    log.info(f"Starting mock server on ws://{host}:{port}")
    log.info(f"Fake messages every {interval}s — press Ctrl+C to stop")

    server = await ws_serve(ws_handler, host, port)
    broadcaster = asyncio.create_task(message_broadcaster(interval))

    try:
        await asyncio.Future()  # run forever
    except asyncio.CancelledError:
        pass
    finally:
        broadcaster.cancel()
        server.close()
        await server.wait_closed()
        log.info("Mock server stopped.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="LawfulOverlay mock WebSocket server")
    parser.add_argument("--host", default="127.0.0.1", help="Bind address (default: 127.0.0.1)")
    parser.add_argument("--port", type=int, default=8765, help="WebSocket port (default: 8765)")
    parser.add_argument("--interval", type=float, default=4.0, help="Seconds between fake messages (default: 4)")
    args = parser.parse_args()

    try:
        asyncio.run(main(args.host, args.port, args.interval))
    except KeyboardInterrupt:
        print("\nStopped.")
