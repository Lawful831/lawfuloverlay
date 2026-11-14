# SPEC: comando /join con voz real en el bot de LawfulOverlay

## Contexto

`server/` es un binario Rust (serenity 0.12.5, `default-features = false`) que ya
relaya mensajes de Discord por WebSocket y sintetiza voz con Fish Audio
(`server/src/fish.rs`, `Fish::synthesize(text, voice) -> Result<Vec<u8>, String>`,
mp3, bloqueante).

Hoy el bot **no tiene voz**: no existe feature `voice`, ni songbird, ni comandos
slash. Eso es lo que se implementa aquí.

## Alcance

Solo `server/`. No tocar `client/`, ni `docker-compose.yml`, ni la lógica de
WebSocket salvo lo indicado.

## 1. Dependencias (`server/Cargo.toml`)

- `serenity`: agregar feature `voice` a la lista existente.
- Agregar `songbird` con `default-features = false` y features:
  `["builtin-queue", "driver", "serenity", "rustls", "symphonia"]`.
  **No** usar OpenSSL ni ffmpeg: la imagen final es `debian:bookworm-slim` sin
  libssl ni ffmpeg, y el audio llega como mp3 en memoria.
- Actualizar `Cargo.lock` (build normal, no `--locked` la primera vez) y dejarlo
  commiteado.

## 2. Módulo nuevo `server/src/voice.rs`

API pública exacta:

```rust
pub struct Voice { /* canal actual, Mutex<Option<ChannelId>> */ }

impl Voice {
    pub fn new() -> Self;
    pub fn channel(&self) -> Option<ChannelId>;
    pub fn set_channel(&self, ch: Option<ChannelId>);
}

/// Une el bot al canal, lo deja sordo (self_deaf) y lo registra en Voice.
pub async fn join(ctx: &Context, voice: &Voice, channel_id: ChannelId) -> Result<(), String>;

/// Sale del canal y limpia el estado.
pub async fn leave(ctx: &Context, voice: &Voice) -> Result<(), String>;

/// Encola un mp3 en el canal actual. No-op silencioso si no hay canal.
pub async fn play_mp3(ctx: &Context, voice: &Voice, mp3: Vec<u8>) -> Result<(), String>;

/// Texto listo para TTS: recorta a 500 caracteres y devuelve None si queda vacío.
pub fn speech_for_tts(text: &str) -> Option<String>;
```

Reglas:
- `join`: `songbird::get(ctx).await` → `join(guild_id, channel_id)`; después
  `call.lock().await.deafen(true)` y además `ctx.http.edit_voice_state(...)`
  con `self_deaf(true)` y `self_mute(true)` para que el gateway lo vea sordo.
  El `guild_id` se obtiene del canal (`channel_id.to_channel(&ctx.http).await`).
- `play_mp3`: `songbird::input::Input::try_from(std::io::Cursor::new(mp3))`
  (symphonia decodifica mp3). Encolar con `call.play_source(input)`.
- `speech_for_tts` es pura y se prueba con tests unitarios.

## 3. Comandos slash (`server/src/discord.rs`)

- En `ready()`: registrar dos comandos globales con
  `serenity::model::application::Command::create_global_command`:
  - `join` — descripción: "Entra a tu canal de voz y lee los mensajes en voz alta"
  - `leave` — descripción: "Sale del canal de voz"
  Registrar una sola vez; si falla, `warn!` y seguir (no tumbar el bot).
- En `interaction_create` manejar `Interaction::ApplicationCommand`:
  - `join`: leer `guild.voice_states[user_id].channel_id` del cache.
    Sin canal → respuesta efímera "Entra a un canal de voz primero."
    Con `FISH_API_KEY` ausente → unir igual y responder
    "Entré a **<canal>**, pero la voz está apagada (falta FISH_API_KEY)."
    Éxito → "Entré a **<canal>**, sordo y listo para leer."
  - `leave` → "Salí del canal de voz." o "No estoy en ningún canal."
  - Todas las respuestas efímeras, en español, sin emoji.

## 4. Reproducción de mensajes

En `Handler::message`, después del `broadcast`, si hay canal de voz activo y
`speech` no está vacío:

1. `speech_for_tts(&speech)` → si `None`, no hacer nada.
2. `tokio::task::spawn` con `tokio::task::spawn_blocking` para
   `fish.synthesize(&texto, &voice_id)` (bloqueante, nunca en el runtime).
3. Con el mp3, `voice::play_mp3(&ctx, &voice, mp3).await`.
4. Cualquier error: `warn!` con el motivo y continuar. Nunca panic, nunca
   bloquear el relay.

Serialización: el audio se encola con songbird; no lanzar dos `play_source`
simultáneos sobre la misma llamada.

## 5. Configuración (`server/src/config.rs`)

Nueva variable opcional `FISH_VOICE` (id de voz de Fish Audio). Vacía = voz por
defecto del servicio, que es como ya funciona `synthesize`.

## 6. `main.rs`

- Construir `Fish` una vez (`Fish::new(&cfg.fish_api_key)`), compartirlo con
  `Policy` y con `Handler` (`Option<Arc<Fish>>`).
- `Client::builder(...).event_handler(handler).register_songbird().await?`
  (`use songbird::SerenityInit;`).
- `Handler` recibe además `voice: Arc<Voice>` y `fish_voice: String`.

## 7. Tests (`server/tests/`)

- `speech_for_tts`: recorta a 500, `trim`, devuelve `None` con solo espacios.
- Los tests existentes (`tests/ws.rs`, `tests/tts.rs`) deben seguir pasando.

## Aceptación (ejecutar tú y pegar salida literal)

En el contenedor de build, desde la raíz del repo:

    cargo fmt --all -- --check
    cargo check --locked -p lawful-overlay-server
    cargo test --locked -p lawful-overlay-server
    cargo clippy --locked -p lawful-overlay-server -- -D warnings

`fmt` puede reformatear antes de comprobar; `check`, `test` y `clippy` deben salir
en 0. Si clippy se queja de código preexistente que no tocaste, arréglalo solo si
es trivial; si no, dilo en el informe y no lo silencies con `allow` global.

## Informe final (texto plano, máximo 15 líneas)

- archivos tocados y líneas añadidas
- salida literal de los cuatro comandos
- decisión de versiones de songbird/serenity y por qué
- cualquier cosa del spec que no se pudo cumplir
