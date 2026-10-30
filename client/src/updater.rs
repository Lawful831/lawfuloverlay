//! Self-update from the project's GitHub releases.
//!
//! A background thread asks GitHub for the latest release; when its tag is
//! newer than this build, it downloads `LawfulOverlay.exe`, verifies it, swaps
//! it in for the running executable and relaunches. A running exe can be
//! renamed on Windows but not overwritten, so the old file is moved aside to
//! `*.old` and removed on the next start.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use eframe::egui;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::net::Event;

const LATEST_URL: &str = "https://api.github.com/repos/Lawful831/lawfuloverlay/releases/latest";
const DOWNLOAD_PREFIX: &str = "https://github.com/Lawful831/lawfuloverlay/releases/download/";
const ASSET_NAME: &str = "LawfulOverlay.exe";
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(15 * 60);

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    /// `sha256:<hex>`; GitHub fills it in for uploaded assets.
    digest: Option<String>,
}

/// `v1.2.3` / `1.2.3` → `(1, 2, 3)`. Anything else (pre-releases included) is ignored.
fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.trim().trim_start_matches('v').split('.');
    let v = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(v)
}

fn exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Dev runs live under `target/`; replacing those would break `cargo run`.
fn is_dev_build(exe: &Path) -> bool {
    exe.components().any(|c| c.as_os_str() == "target")
}

/// Remove the leftovers of a previous update. Call once at startup.
pub fn cleanup() {
    if let Some(exe) = exe_path() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
        let _ = std::fs::remove_file(exe.with_extension("new"));
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .user_agent(concat!("LawfulOverlay/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// The newest release's asset, if it is newer than this build.
fn find_update(agent: &ureq::Agent) -> Result<Option<(String, Asset)>, String> {
    let mut resp = agent
        .get(LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let release: Release = serde_json::from_reader(resp.body_mut().with_config().limit(1 << 20).reader())
        .map_err(|e| e.to_string())?;

    let current = parse_version(env!("CARGO_PKG_VERSION")).ok_or("versión local ilegible")?;
    let Some(latest) = parse_version(&release.tag_name) else {
        return Ok(None);
    };
    if latest <= current {
        return Ok(None);
    }
    let asset = release
        .assets
        .into_iter()
        .find(|a| a.name == ASSET_NAME)
        .ok_or("la release no incluye LawfulOverlay.exe")?;
    if !asset.browser_download_url.starts_with(DOWNLOAD_PREFIX) {
        return Err("URL de descarga inesperada".into());
    }
    Ok(Some((release.tag_name, asset)))
}

/// Download the asset next to the running exe, checking size and digest.
fn download(agent: &ureq::Agent, asset: &Asset, dest: &Path) -> Result<(), String> {
    if asset.size == 0 || asset.size > MAX_DOWNLOAD {
        return Err("tamaño de descarga inválido".into());
    }
    let mut resp = agent.get(&asset.browser_download_url).call().map_err(|e| e.to_string())?;
    let mut reader = resp.body_mut().with_config().limit(MAX_DOWNLOAD).reader();

    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        total += n as u64;
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    if total != asset.size {
        return Err(format!("descarga incompleta ({total} de {} bytes)", asset.size));
    }
    if let Some(expected) = asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")) {
        let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if !actual.eq_ignore_ascii_case(expected) {
            return Err("el hash SHA-256 no coincide".into());
        }
    }
    Ok(())
}

/// Swap the downloaded file in for the running exe.
fn install(exe: &Path, new: &Path) -> Result<(), String> {
    let old = exe.with_extension("old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| format!("no se pudo apartar el exe actual: {e}"))?;
    if let Err(e) = std::fs::rename(new, exe) {
        let _ = std::fs::rename(&old, exe); // put the working copy back
        return Err(format!("no se pudo instalar la nueva versión: {e}"));
    }
    Ok(())
}

fn update_once(tx: &Sender<Event>, ctx: &egui::Context, exe: &Path) -> Result<(), String> {
    let say = |text: String| {
        let _ = tx.send(Event::Status(text));
        ctx.request_repaint();
    };
    let agent = agent();
    let Some((tag, asset)) = find_update(&agent)? else {
        return Ok(());
    };

    let new = exe.with_extension("new");
    say(format!("Descargando actualización {tag}…"));
    if let Err(e) = download(&agent, &asset, &new) {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    install(exe, &new).inspect_err(|_| {
        let _ = std::fs::remove_file(&new);
    })?;

    say(format!("Actualizado a {tag}. Reiniciando…"));
    std::thread::sleep(Duration::from_millis(800));
    std::process::Command::new(exe)
        .spawn()
        .map_err(|e| format!("no se pudo reiniciar: {e}"))?;
    std::process::exit(0);
}

/// Start checking for updates now and every few hours after.
pub fn spawn(tx: Sender<Event>, ctx: egui::Context) {
    if std::env::var_os("LAWFUL_OVERLAY_NO_UPDATE").is_some() {
        return;
    }
    let Some(exe) = exe_path() else { return };
    if is_dev_build(&exe) {
        eprintln!("[update] build de desarrollo; sin actualizaciones automáticas");
        return;
    }
    std::thread::spawn(move || loop {
        let wait = match update_once(&tx, &ctx, &exe) {
            Ok(()) => CHECK_EVERY,
            Err(e) => {
                eprintln!("[update] {e}");
                RETRY_AFTER
            }
        };
        std::thread::sleep(wait);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.10.0"), Some((0, 10, 0)));
        assert_eq!(parse_version("v1.2"), None);
        assert_eq!(parse_version("v1.2.3-beta"), None);
        assert_eq!(parse_version("v1.2.3.4"), None);
    }

    #[test]
    fn orders_numerically() {
        assert!(parse_version("v0.10.0") > parse_version("v0.9.9"));
    }

    #[test]
    fn dev_builds_are_detected() {
        assert!(is_dev_build(Path::new(r"C:\x\target\release\lawful-overlay.exe")));
        assert!(!is_dev_build(Path::new(r"C:\Apps\LawfulOverlay.exe")));
    }
}
