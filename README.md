# LawfulOverlay

A transparent, always-on-top Discord chat overlay for streamers and gamers.

## Architecture

```
┌─────────────────────────────────┐          ┌──────────────────────────┐
│  SERVER  (docker-compose.yml)   │          │  CLIENT  (lawful-overlay)│
│                                 │          │                          │
│  server/  (Rust)                │ WebSocket│  client/  (Rust, egui)   │
│  ─ Discord bot (serenity)       │◄────────►│  ─ connects to server    │
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
# Edit .env — fill in DISCORD_BOT_TOKEN, TARGET_USER_IDS, TARGET_SERVER_IDS
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

The whole project is Rust. Install the toolchain from <https://rustup.rs> once, then:

```bash
# Start the overlay
cargo run --release -p lawful-overlay

# Or build standalone executables into dist\ (Windows)
scriptsuild.bat
```

The client connects to `ws://127.0.0.1:8765` by default.
You can change the URL via the ⚙ settings button in the overlay.
The client updates itself: on start (and every 6 h) it checks the latest GitHub release and, if newer, downloads `LawfulOverlay.exe`, swaps it in and restarts. Turn it off in ⚙ → *Actualizar automáticamente*, or set `LAWFUL_OVERLAY_NO_UPDATE=1`. To ship an update, bump `version` in `client/Cargo.toml` and push a matching tag (`git tag v0.3.0 && git push origin v0.3.0`); `.github/workflows/release.yml` builds and attaches the exe.
Settings are stored in `~/.discord_chat_overlay/client_settings.json`.

The socket requires a shared token. Put it in the URL itself:

```
ws://<host>:8765/?token=<WS_TOKEN>
```

A client that connects without it is closed immediately with code `1008`
(`token required`). A browser page from an origin outside `WS_ORIGINS` is closed
with `1008` (`origin not allowed`), which is what prevents cross-site WebSocket
hijacking. Native clients send no `Origin` header and are unaffected.

---

## Running the server without Docker

```bash
cd server
cp .env.example .env      # fill it in
cargo run --release -p lawful-overlay-server
```

Flags: `--ws-only` runs the socket without the Discord bot; `--healthcheck`
exits 0 when the local socket accepts connections (used by the container).

## Testing without Discord

```bash
scripts	est_local.bat     # mock server + overlay client, on a throwaway settings file
cargo test                 # unit tests + socket policy tests
```

---

## Server Configuration (`server/.env`)

| Variable            | Required | Description                                    |
|---------------------|----------|------------------------------------------------|
| `DISCORD_BOT_TOKEN` | ✅       | Bot token from the Discord Developer Portal    |
| `TARGET_USER_IDS`   | ✅       | Comma-separated Discord user IDs to monitor    |
| `TARGET_SERVER_IDS` | ✅       | Comma-separated Discord guild (server) IDs to monitor. The legacy `TARGET_SERVER_ID` is still accepted and merged in |
| `WS_HOST`           | ❌       | WebSocket bind address (default `0.0.0.0`)     |
| `WS_PORT`           | ❌       | WebSocket port (default `8765`)                |
| `WS_TOKEN`          | ✅       | Shared client token; the server will not start without it |
| `WS_ORIGINS`        | ❌       | Comma-separated browser origins allowed to connect |
| `WS_MAX_CLIENTS`    | ❌       | Global connection cap (default `25`)           |
| `WS_MAX_PER_IP`     | ❌       | Connections per source address (default `3`)   |
| `WS_MAX_FRAME_BYTES`| ❌       | Largest accepted frame (default `65536`)       |
| `WS_MAX_FRAMES_PER_S` | ❌     | Frames per second per connection before it is closed (default `20`) |
| `WS_TLS_CERT`       | ❌       | Certificate chain; enables `wss://` in the app |
| `WS_TLS_KEY`        | ❌       | Private key matching `WS_TLS_CERT`             |

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
├── Cargo.toml              # Workspace: server + client
├── docker-compose.yml      # Orchestrates the server container
├── client/                 # Desktop overlay (no bot token)
│   └── src/                #   app, settings, net, tts, color, sample, config
├── server/
│   ├── src/                # Discord bot, WebSocket hub, text rules, config
│   │   └── bin/mock.rs     # Mock server for testing the client
│   ├── tests/ws.rs         # Socket policy tests (token, origin, caps, rate limit)
│   ├── Dockerfile          # Multi-stage, non-root image
│   └── .env.example        # Template — copy to .env and fill in values
└── scripts/                # build.bat, test_local.bat
```

## License

MIT