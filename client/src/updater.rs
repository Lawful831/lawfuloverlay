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
    // No digest, no install: an update that cannot be checked is not trusted.
    let expected = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .ok_or("la release no publica el hash SHA-256")?;
    let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if !actual.eq_ignore_ascii_case(expected) {
        return Err("el hash SHA-256 no coincide".into());
    }
    Ok(())
}

/// Once this build is code-signed, only an update signed by the same
/// publisher may replace it. Unsigned builds skip the check, so the first
/// signed release still reaches clients installed before signing existed.
fn check_publisher(exe: &Path, new: &Path) -> Result<(), String> {
    let Some(current) = authenticode::publisher(exe) else {
        return Ok(());
    };
    match authenticode::publisher(new) {
        Some(publisher) if publisher == current => Ok(()),
        Some(publisher) => Err(format!("la actualización está firmada por otro editor ({publisher})")),
        None => Err("la actualización no tiene una firma digital válida".into()),
    }
}

#[cfg(windows)]
mod authenticode {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Security::Cryptography::{CertNameToStrW, CERT_X500_NAME_STR, X509_ASN_ENCODING};
    use windows_sys::Win32::Security::WinTrust::{
        WTHelperGetProvCertFromChain, WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData,
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_FILE_INFO,
        WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };

    /// The certificate subject of the file's embedded Authenticode signature,
    /// when Windows accepts that signature as valid and trusted. `None` for
    /// unsigned files and for signatures Windows rejects.
    pub fn publisher(path: &Path) -> Option<String> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: both structs are plain data that outlive the two calls;
        // `wide` is NUL-terminated; the state handle opened by VERIFY is
        // always released by the CLOSE call, and the pointers read from it
        // are only used before that.
        unsafe {
            let mut file: WINTRUST_FILE_INFO = std::mem::zeroed();
            file.cbStruct = std::mem::size_of::<WINTRUST_FILE_INFO>() as u32;
            file.pcwszFilePath = wide.as_ptr();

            let mut data: WINTRUST_DATA = std::mem::zeroed();
            data.cbStruct = std::mem::size_of::<WINTRUST_DATA>() as u32;
            data.dwUIChoice = WTD_UI_NONE;
            data.fdwRevocationChecks = WTD_REVOKE_NONE;
            data.dwUnionChoice = WTD_CHOICE_FILE;
            data.Anonymous.pFile = &mut file;
            data.dwStateAction = WTD_STATEACTION_VERIFY;

            let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
            let data_ptr = std::ptr::addr_of_mut!(data);
            let status = WinVerifyTrust(std::ptr::null_mut(), &mut action, data_ptr.cast::<c_void>());
            let subject = if status == 0 { signer_subject((*data_ptr).hWVTStateData) } else { None };

            (*data_ptr).dwStateAction = WTD_STATEACTION_CLOSE;
            WinVerifyTrust(std::ptr::null_mut(), &mut action, data_ptr.cast::<c_void>());
            subject
        }
    }

    /// SAFETY: `state` must be the open state handle of a successful verification.
    unsafe fn signer_subject(state: windows_sys::Win32::Foundation::HANDLE) -> Option<String> {
        let prov = WTHelperProvDataFromStateData(state);
        if prov.is_null() {
            return None;
        }
        let signer = WTHelperGetProvSignerFromChain(prov, 0, 0, 0);
        if signer.is_null() {
            return None;
        }
        let cert = WTHelperGetProvCertFromChain(signer, 0);
        if cert.is_null() || (*cert).pCert.is_null() || (*(*cert).pCert).pCertInfo.is_null() {
            return None;
        }
        let name = &(*(*(*cert).pCert).pCertInfo).Subject;
        let mut buf = [0u16; 1024];
        let len = CertNameToStrW(X509_ASN_ENCODING, name, CERT_X500_NAME_STR, buf.as_mut_ptr(), buf.len() as u32);
        // `len` counts the terminating NUL.
        (len > 1).then(|| String::from_utf16_lossy(&buf[..len as usize - 1]))
    }
}

#[cfg(not(windows))]
mod authenticode {
    pub fn publisher(_path: &std::path::Path) -> Option<String> {
        None
    }
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
    if let Err(e) = check_publisher(exe, &new) {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    if let Err(e) = install(exe, &new) {
        // Usually a folder the user can't write to; retrying soon would only
        // download the same file again, so wait for the regular check.
        let _ = std::fs::remove_file(&new);
        eprintln!("[update] {e}");
        crate::telemetry::report("update_install_failed", &e);
        say(format!("No se pudo instalar {tag}: mueve LawfulOverlay a una carpeta con permisos de escritura."));
        return Ok(());
    }

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
                crate::telemetry::report("update_failed", &e);
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

    /// The test binary is unsigned; a copy of a Microsoft-signed exe is not.
    /// The copy is named `*.new`, like the file the updater checks.
    #[cfg(windows)]
    #[test]
    fn reads_authenticode_publisher() {
        let me = std::env::current_exe().unwrap();
        assert_eq!(authenticode::publisher(&me), None);
        assert!(check_publisher(&me, &me).is_ok(), "unsigned builds skip the check");

        let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap();
        let candidates = [
            PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
            windir.join(r"System32\OneDriveSetup.exe"),
            windir.join(r"SysWOW64\OneDriveSetup.exe"),
        ];
        let Some(signed) = candidates.iter().find(|p| authenticode::publisher(p).is_some()) else {
            eprintln!("no embedded-signed exe found on this machine; skipping");
            return;
        };
        let copy = std::env::temp_dir().join(format!("lawful_sig_test_{}.new", std::process::id()));
        std::fs::copy(signed, &copy).unwrap();
        let publisher = authenticode::publisher(&copy);
        let same = check_publisher(signed, &copy);
        let unsigned_update = check_publisher(signed, &me);
        let _ = std::fs::remove_file(&copy);

        let publisher = publisher.expect("signature survives the .new name");
        assert!(publisher.contains("Microsoft"), "{publisher}");
        assert!(same.is_ok(), "{same:?}");
        assert!(unsigned_update.is_err(), "a signed build must refuse an unsigned update");
    }

    #[test]
    fn dev_builds_are_detected() {
        assert!(is_dev_build(Path::new(r"C:\x\target\release\lawful-overlay.exe")));
        assert!(!is_dev_build(Path::new(r"C:\Apps\LawfulOverlay.exe")));
    }
}
