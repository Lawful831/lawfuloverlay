# SPEC: subir songbird a 0.6 para que la voz funcione (DAVE)

## Causa raíz confirmada

Discord exige **DAVE** (el cifrado de extremo a extremo de voz) en la negociación del canal.
`songbird 0.4.6`, la versión que usa el repo, **no implementa DAVE**: su `Cargo.toml` no tiene
`davey` y su driver no menciona el protocolo. Por eso `manager.join()` falla con
`JoinError::Driver(...)` y el mensaje queda en `establishing connection failed`.

Prueba de que el resto de la red está bien, ya verificada por el operador en la red del contenedor:
UDP saliente OK, DNS OK, TLS 1.3 contra `discord.gg` OK, reloj correcto, gateway del bot conectado.

`songbird 0.6.1` sí incluye `davey` y depende de **serenity 0.12**, la misma que ya usa el repo.
Ese es el salto: songbird, no serenity.

## Alcance

Solo `server/`. No tocar `client/`, ni el protocolo WebSocket, ni el formato de los frames.

## 1. Dependencias

En `server/Cargo.toml`, cambiar `songbird` a `0.6.1`.

Features a usar (`default-features = false`): `["builtin-queue", "driver", "gateway", "serenity", "rustls"]`.
Si el compilador pide más, añadirlas y anotar cuáles en el informe. **No** activar `openssl`.

`symphonia` sigue siendo necesario para decodificar mp3 si 0.6 ya no lo trae por defecto; si la API
de `Input` cambió, ajustar `play_mp3` para que siga aceptando `Vec<u8>` de mp3.

## 2. Adaptar `server/src/voice.rs`

La API de songbird 0.6 puede haber cambiado respecto a 0.4. Mantener el comportamiento exacto:

- `join`: unirse al canal, **dejar al bot sordo** (`self_deaf`), guardar el canal en `Voice`.
- `leave`: salir y limpiar.
- `play_mp3`: encolar el mp3 en la llamada actual, sin cortar lo que ya suena.
- `speech_for_tts`: sin cambios (500 chars, `None` si vacío).

`join` debe registrar el error del driver con `{:?}` para que un fallo futuro diga la causa real,
no solo `establishing connection failed`.

## 3. Mantener `/join` y `/leave`

`discord.rs` no cambia de contrato: mismas respuestas efímeras, mismo `defer_ephemeral` antes de
intentar la conexión, mismos comandos por guild.

## 4. Mantener el binario `voice-selftest`

Debe seguir compilando y funcionando con la nueva versión, imprimiendo `JOIN_OK`, `SORDO_OK`,
`LEAVE_OK` en ese orden cuando la conexión funciona.

## Aceptación (ejecutar tú y pegar la salida literal)

    cargo fmt -p lawful-overlay-server
    cargo check --locked -p lawful-overlay-server -j 2
    cargo test --locked -p lawful-overlay-server -j 2
    cargo clippy --locked -p lawful-overlay-server -j 2 -- -D warnings
    cargo build --locked -p lawful-overlay-server --bin voice-selftest -j 2

Los cinco en 0. **No** ejecutes el binario contra Discord: el token no está aquí y la prueba en
vivo la hace el operador.

## Informe final (máximo 12 líneas, texto plano)

- versión final de songbird/serenity y features usadas
- archivos tocados y qué cambió en la API de voz
- salida literal de los cinco comandos
- si `Input`/`enqueue_input`/`deafen` cambiaron de firma, cómo quedaron
- cualquier cosa del spec que no se pudo cumplir
