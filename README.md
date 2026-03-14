# LawfulOverlay

A transparent, always-on-top Discord chat overlay for streamers and gamers.

## Architecture

```
┌─────────────────────────────────┐          ┌──────────────────────────┐
│  SERVER  (docker-compose.yml)   │          │  CLIENT  (app.py)        │
│                                 │          │                          │
│  server/bot.py                  │ WebSocket│  Tkinter overlay         │
│  ─ Discord bot (discord.py)     │◄────────►│  ─ connects to server    │
│  ─ WebSocket broadcaster        │  ws://   │  ─ displays messages     │
│                                 │          │  ─ NO bot token          │
└─────────────────────────────────┘          └──────────────────────────┘
```

**The bot token and Discord credentials live exclusively on the server.**
The client only opens a WebSocket connection and renders incoming text.

---

## Quick Start

### 1 · Set up the server

```bash
cd server
cp .env.example .env
# Edit .env — fill in DISCORD_BOT_TOKEN, TARGET_USER_IDS, TARGET_SERVER_ID
```

> **Discord Developer Portal requirements**
> - Under your bot's settings → *Privileged Gateway Intents* → enable **Message Content Intent**.
> - The bot only needs the `messages` and `guilds` intents; it does **not** request Presence or Server Members intents.
> - If your bot is in fewer than 100 servers, no verification is required.

### 2 · Start the server with Docker Compose

```bash
docker compose up -d
# View logs
docker compose logs -f lawful-overlay-bot
```

### 3 · Run the client

```bash
# Install client dependencies once
pip install -r requirements.txt

# Start the overlay
python app.py
```

The client connects to `ws://127.0.0.1:8765` by default.
You can change the URL via the ⚙ settings button in the overlay.

---

## Server Configuration (`server/.env`)

| Variable            | Required | Description                                    |
|---------------------|----------|------------------------------------------------|
| `DISCORD_BOT_TOKEN` | ✅       | Bot token from the Discord Developer Portal    |
| `TARGET_USER_IDS`   | ✅       | Comma-separated Discord user IDs to monitor    |
| `TARGET_SERVER_ID`  | ✅       | Numeric ID of the Discord guild (server)        |
| `WS_HOST`           | ❌       | WebSocket bind address (default `0.0.0.0`)     |
| `WS_PORT`           | ❌       | WebSocket port (default `8765`)                |

---

## Security & Policy Compliance

| Area | What we do |
|---|---|
| **Token safety** | Bot token is read from `.env` on the server only; it is never transmitted to clients |
| **Minimal intents** | Only `message_content` privileged intent is requested — no presence, no member list |
| **Non-root container** | Docker image runs under a dedicated `overlay` system user |
| **No capabilities** | All Linux capabilities dropped (`cap_drop: ALL`) |
| **Read-only filesystem** | Container filesystem is read-only; only `/tmp` is writable |
| **Localhost-only port** | Docker Compose binds the port to `127.0.0.1` only by default |
| **Resource limits** | CPU cap 0.5 core, memory cap 256 MB |
| **No privilege escalation** | `no-new-privileges:true` security option |
| **Secrets not in VCS** | `.env` / `server/.env` are in `.gitignore` |
| **Rate limiting** | The bot does not call any Discord API endpoints beyond reading messages — no spam risk |

---

## Project Structure

```
LawfulOverlay/
├── app.py                  # Desktop overlay client (no bot token)
├── requirements.txt        # Client dependencies
├── docker-compose.yml      # Orchestrates the server container
└── server/
    ├── bot.py              # Discord bot + WebSocket broadcaster
    ├── requirements.txt    # Server dependencies
    ├── Dockerfile          # Secure, non-root image definition
    └── .env.example        # Template — copy to .env and fill in values
```

## License

MIT