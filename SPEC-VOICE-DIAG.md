# SPEC: diagnosticar por qué falla la conexión de voz

## Síntoma real (ya reproducido en producción)

`/join` responde y el bot aparece en el canal, pero:

```
WARN lawful_overlay_server::discord: Failed to join voice channel 1203653364777484332:
no se pudo unir al canal de voz: failed to join voice channel: establishing connection failed
```

Ese texto es `JoinError::Driver(_)` formateado con `Display`, que **se come el error
interno**. La causa está en `songbird::error::JoinError::Driver(DriverError)`, y el
`Display` de `JoinError` no lo imprime. Sin ese dato no se puede saber si falló el
WebSocket de voz, el TLS, el socket UDP o la IP discovery.

Contexto de red ya verificado en el host:
- El contenedor vive en la red docker `coolify` (10.0.1.0/24, IP 10.0.1.2).
- El firewall del host ya permite UDP saliente desde esa subred; un contenedor de
  prueba en la misma red resuelve UDP a 8.8.8.8:53 sin problema.
- Reloj del contenedor correcto (UTC).
- El gateway de Discord funciona: el bot se loguea y relaya mensajes.

## Alcance

Solo `server/`. No tocar `client/`, ni el protocolo WebSocket.

## 1. Exponer el error interno

En `server/src/voice.rs`, `join()` debe reportar el error del driver con su `Debug`
completo, no solo `Display`. Cambiar el `map_err` de `manager.join(...)` para que
produzca algo como:

    format!("no se pudo unir al canal de voz: {e:?}")

`JoinError` implementa `Debug` y ese `Debug` sí incluye el `DriverError` interno.

## 2. Binario de autodiagnóstico: `server/src/bin/voice-selftest.rs`

Nuevo binario, registrado en `server/Cargo.toml` como
`[[bin]] name = "voice-selftest" path = "src/bin/voice-selftest.rs"`.

Comportamiento:

1. Lee `DISCORD_BOT_TOKEN` del entorno (obligatorio; si falta, error claro y exit 2).
2. Acepta un argumento posicional: el **id del canal de voz** (`u64`). Si falta,
   exit 2 con uso.
3. Construye un `Client` de serenity con los mismos intents que el bot real
   (`GUILDS | GUILD_MESSAGES | GUILD_VOICE_STATES | DIRECT_MESSAGES | MESSAGE_CONTENT`),
   `register_songbird()`, y un handler mínimo cuyo `ready`:
   - obtiene el guild del canal con `channel_id.to_channel(&ctx.http)`,
   - llama a `voice::join(&ctx, &voice, channel_id)`,
   - si falla: imprime `JOIN_FALLÓ: {e:?}` y sale con código 1,
   - si funciona: imprime `JOIN_OK canal=<id>`, espera 2 s, comprueba que el estado
     del bot está sordo con `songbird::get(&ctx).await` + `call.lock().await.deafen(true)`
     ya aplicado (imprime `SORDO_OK`), luego `voice::leave(&ctx, &voice)`, imprime
     `LEAVE_OK` y sale con 0.
4. Timeout total de 45 s: si no termina, imprime `TIMEOUT` y exit 3.
5. Nada de `unwrap()` en rutas de error; todo con mensajes explícitos.

El binario es de diagnóstico: puede imprimir por stdout sin miedo al protocolo MCP.

## 3. Tests

- Los tests existentes deben seguir pasando.
- Añadir a `server/tests/voice.rs` un test que verifique el formateo del error:
  construir un `JoinError::Dropped` (o el variante que exista) y comprobar que
  `format!("{e:?}")` no está vacío. Si `JoinError` no es construible desde fuera,
  omitir ese test y decirlo en el informe.

## Aceptación (ejecutar tú y pegar salida)

    cargo fmt -p lawful-overlay-server
    cargo check --locked -p lawful-overlay-server -j 2
    cargo test --locked -p lawful-overlay-server -j 2
    cargo clippy --locked -p lawful-overlay-server -j 2 -- -D warnings
    cargo build --locked -p lawful-overlay-server --bin voice-selftest -j 2

Todo en 0. **No** ejecutes el binario contra Discord: el token no está aquí y la
prueba en vivo la hace el operador.

## Informe final (máximo 10 líneas, texto plano)

- archivos tocados
- salida literal de los cinco comandos
- qué variante de `JoinError`/`DriverError` imprime `Debug` y qué campos añade
- cualquier limitación del binario de autodiagnóstico
