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

### Voice (text to speech)

The 🔊 button reads messages aloud: only the message, not the name of who wrote it (the overlay still shows the name). Voices are tried in this order:

1. **Fish Audio**, only when the server has a `FISH_API_KEY` and a voice id is known: the one in ⚙ → *Voz* → *Voz Fish Audio (ID)*, or the one the message names. The client asks the server over the WebSocket; the server calls Fish Audio, so the key never reaches a client. The paid model (`s2.1-pro`) is tried first; if it is refused (no credit, rate limit, overload) the free one (`s2.1-pro-free`) is tried. If the server has no key or refuses (a limit, an error), the next voice takes over.
2. **Edge TTS** (the voice picked in ⚙ → *Voz*).
3. The voice built into Windows.

Markup in a chat message:

| Written in Discord | Effect |
|---|---|
| `//ca3007f96ae7499ab87d27ea3599956a// hola` | That message is spoken with that Fish Audio voice (the 32 hex characters at the end of a `fish.audio/m/…` URL). The template is not shown. |
| `[happy] hola [laughing]` | Text in square brackets is never shown. Fish Audio gets it as an emotion or tone cue; the other voices skip it. The server's own tags (`[Link de …]`, `[GIF]`, `[Imagen]`, `[Archivo]`) still show. |

**Fish Audio key.** Set `FISH_API_KEY` in the server's `.env` (see the table below) and redeploy the server. The client holds no key and needs no build variable. The server limits each request to 500 characters, each connection to 10 requests per minute, and the whole server to 300 Fish Audio calls per UTC day (constants in `server/src/fish.rs` and `server/src/ws.rs`). Never commit the key.

### Windows trust

What the client does so Windows and antivirus tools can tell what it is:

- **Identity.** The exe carries version info (product, publisher, version, copyright) and an application manifest that declares it never asks for elevation (`asInvoker`). See `client/build.rs`.
- **No hidden child processes.** The fallback Windows voice runs in-process through SAPI. The client starts no PowerShell or `cmd`. The only process it ever starts is its own new version after an update.
- **Checked updates.** An update is installed only if it comes from this repository's releases over HTTPS, has the size GitHub reports, and matches the SHA-256 digest GitHub publishes. Once a build is code-signed, it also refuses any update that is not validly signed by the same publisher.
- **Verifiable releases.** Each release has `SHA256SUMS.txt` and a build provenance attestation. To prove an exe was built by this repository's workflow: `gh attestation verify LawfulOverlay.exe --repo Lawful831/lawfuloverlay`.

**Code signing** is the one step that removes the SmartScreen "unknown publisher" warning, and it needs a certificate that only the maintainer can obtain. The release workflow already signs when these are set in the GitHub repository (Settings → Secrets and variables → Actions):

| Kind | Name | Value |
|---|---|---|
| Variable | `AZURE_SIGNING_ACCOUNT` | Artifact Signing account name |
| Variable | `AZURE_SIGNING_PROFILE` | Certificate profile name |
| Variable | `AZURE_SIGNING_ENDPOINT` | Region endpoint, e.g. `https://eus.codesigning.azure.net/` |
| Secret | `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET` | App registration with the *Artifact Signing Certificate Profile Signer* role |

This uses [Azure Artifact Signing](https://learn.microsoft.com/azure/artifact-signing/) (formerly Trusted Signing), a paid Microsoft service with identity validation; check that your country is eligible. A free alternative for open-source projects is [SignPath Foundation](https://signpath.org/); it needs a different signing step in the workflow. Until one is set up, releases are unsigned and new versions can still show a SmartScreen warning.

If Defender flags a release by mistake, report it at <https://www.microsoft.com/wdsi/filesubmission> (choose *Software developer*). Microsoft then corrects the detection for every user, which an exclusion on one machine does not do.

**Windows Defender still flags or removes the exe?** As a last resort on your own machine, `scripts/defender_exclusion.ps1` (kept in the repo only, not shipped with the exe) excludes the exe, the updater's `LawfulOverlay.new` / `LawfulOverlay.old` files and the settings folder. Every path is worked out on the machine that runs it, so it works for any user and install folder. It asks for administrator rights, skips exclusions that already exist, and `-Remove` undoes them. Put it next to `LawfulOverlay.exe` (or pass `-ExePath`) and run:

```powershell
powershell -ExecutionPolicy Bypass -File .\defender_exclusion.ps1
powershell -ExecutionPolicy Bypass -File .\defender_exclusion.ps1 -ExePath "D:\Apps\LawfulOverlay.exe"
powershell -ExecutionPolicy Bypass -File .\defender_exclusion.ps1 -Remove
```

Run it again if you move the exe.

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
| `FISH_API_KEY`      | ❌       | Fish Audio API key. Unset = no Fish Audio voices; clients fall back to Edge and Windows. Never logged |

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
| **Media through Discord** | Images are fetched through Discord's media proxy, never from the linked sites; the overlay only opens `http(s)` links, and only when clicked |
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