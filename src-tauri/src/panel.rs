//! The panel updating itself: find a newer release of this repo, download its
//! Windows installer and leave the install to a helper that waits for this
//! process to exit. Nothing here verifies a signature: the installer comes from
//! the release the repo itself publishes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::journal::{self, kind, Draft};
use crate::state::AppState;

const REPO: &str = "killo3967/gentle-update-panel";
/// Only the NSIS installer for this architecture is worth downloading.
const ASSET_SUFFIX: &str = "x64-setup.exe";
const USER_AGENT: &str = "gentle-update-panel (+windows)";
/// A release installer is never this small; anything smaller is an error page.
const MIN_INSTALLER_BYTES: u64 = 500_000;

/// Event carrying download progress, in bytes.
pub const EVENT: &str = "panel:progress";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PanelUpdate {
    pub current: String,
    pub latest: String,
    pub asset: String,
    pub url: String,
    pub size: u64,
    pub release_url: String,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PanelProgress {
    /// `downloading`, `installing` or `failed`.
    pub phase: String,
    pub received: u64,
    pub total: u64,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

/// True when `latest` is a higher version than `current`. Unparseable versions
/// never offer an update: guessing wrong here restarts the app for nothing.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (
        semver::Version::parse(latest.trim_start_matches('v')),
        semver::Version::parse(current.trim_start_matches('v')),
    ) {
        (Ok(latest), Ok(current)) => latest > current,
        _ => false,
    }
}

/// Reads the GitHub release payload and keeps it only if it beats `current`.
pub fn parse_release(body: &str, current: &str, checked_at: &str) -> Option<PanelUpdate> {
    let release: Release = serde_json::from_str(body).ok()?;
    let latest = release.tag_name.trim_start_matches('v').to_string();
    if !is_newer(&latest, current) {
        return None;
    }

    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with(ASSET_SUFFIX))?;

    Some(PanelUpdate {
        current: current.to_string(),
        latest,
        asset: asset.name.clone(),
        url: asset.browser_download_url.clone(),
        size: asset.size,
        release_url: release.html_url,
        checked_at: checked_at.to_string(),
    })
}

/// Asks GitHub for the newest release. `Ok(None)` means "nothing newer".
pub async fn check(current: &str) -> Result<Option<PanelUpdate>, String> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?;

    let response = client
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.status().is_success() {
        return Err(format!("GitHub respondió {}", response.status()));
    }

    let body = response.text().await.map_err(|error| error.to_string())?;
    Ok(parse_release(&body, current, &now()))
}

/// Checks, stores the result and tells the user when it is news.
pub async fn refresh(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let current = app.package_info().version.to_string();
    let previous = state.panel_update();

    let found = match check(&current).await {
        Ok(found) => found,
        Err(_) => return,
    };

    let is_news = found
        .as_ref()
        .map(|update| {
            previous
                .as_ref()
                .map(|previous| previous.latest != update.latest)
                .unwrap_or(true)
        })
        .unwrap_or(false);

    let unchanged = match (&found, &previous) {
        (Some(found), Some(previous)) => found == previous,
        (None, None) => true,
        _ => false,
    };
    if unchanged {
        return;
    }

    state.store_panel_update(found.as_ref());
    let _ = app.emit("panel:update", found.as_ref());
    crate::tray::reflect(app);

    if !is_news {
        return;
    }
    let Some(update) = found else {
        return;
    };

    journal::record(
        app,
        Draft::new(
            kind::PANEL,
            format!("Nueva versión del panel: {}", update.latest),
        )
        .key("journal.panel.found")
        .arg("version", update.latest.clone())
        .detail(format!("{} → {}", update.current, update.latest)),
    );

    let config = state.config();
    if config.notify_on_update {
        let _ = app
            .notification()
            .builder()
            .title(crate::i18n::t_args(
                "notify.panel.title",
                &[("version", &update.latest)],
            ))
            .body(crate::i18n::t("notify.panel.body").to_string())
            .show();
    }
}

/// Downloads the installer and hands the install to a helper, then exits.
/// The helper waits for this process to die before touching the installed files.
pub async fn apply(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let update = state
        .panel_update()
        .ok_or_else(|| "error.panel.none".to_string())?;

    let directory = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?
        .join("updates");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let installer = directory.join(sanitize(&update.asset));

    if !already_downloaded(&installer, update.size) {
        download(app, &update, &installer).await?;
    }

    verify_installer(&installer, update.size)?;

    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let script = write_helper(&directory, &installer, &exe)?;

    journal::record(
        app,
        Draft::new(
            kind::PANEL,
            format!("Actualizando el panel a {}", update.latest),
        )
        .key("journal.panel.applying")
        .arg("version", update.latest.clone())
        .detail(format!("instalador {}", installer.display())),
    );

    spawn_helper(&script)?;

    // Give the helper a moment to start waiting on us before the process goes.
    tokio::time::sleep(Duration::from_millis(600)).await;
    app.exit(0);
    Ok(())
}

fn already_downloaded(path: &Path, expected: u64) -> bool {
    match std::fs::metadata(path) {
        Ok(metadata) => expected > 0 && metadata.len() == expected,
        Err(_) => false,
    }
}

async fn download(app: &AppHandle, update: &PanelUpdate, target: &Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|error| error.to_string())?;

    let mut response = client
        .get(&update.url)
        .header("Accept", "application/octet-stream")
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.status().is_success() {
        return fail(app, &format!("error.panel.download|status={}", response.status()));
    }

    let total = if update.size > 0 {
        update.size
    } else {
        response.content_length().unwrap_or(0)
    };

    let partial = target.with_extension("download");
    let mut file = std::fs::File::create(&partial).map_err(|error| error.to_string())?;
    let mut received: u64 = 0;
    let mut last_emit = std::time::Instant::now();

    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => return fail(app, &format!("error.panel.interrupted|detail={error}")),
        };
        file.write_all(&chunk).map_err(|error| error.to_string())?;
        received += chunk.len() as u64;

        if last_emit.elapsed() >= Duration::from_millis(200) {
            last_emit = std::time::Instant::now();
            emit(app, PanelProgress {
                phase: "downloading".to_string(),
                received,
                total,
            });
        }
    }

    file.flush().map_err(|error| error.to_string())?;
    drop(file);
    std::fs::rename(&partial, target).map_err(|error| error.to_string())?;
    emit(app, PanelProgress {
        phase: "downloading".to_string(),
        received,
        total,
    });
    Ok(())
}

/// The file has to look like the installer we asked for: big enough and a PE.
fn verify_installer(path: &Path, expected: u64) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    if bytes.len() < MIN_INSTALLER_BYTES as usize {
        return Err(format!("error.panel.tooSmall|bytes={}", bytes.len()));
    }
    if expected > 0 && bytes.len() as u64 != expected {
        return Err(format!(
            "error.panel.sizeMismatch|bytes={}|expected={expected}",
            bytes.len()
        ));
    }
    if &bytes[..2] != b"MZ" {
        return Err("error.panel.notExecutable".to_string());
    }
    Ok(())
}

/// Writes the script that outlives this process: wait, install silently,
/// relaunch the app, remove itself.
fn write_helper(directory: &Path, installer: &Path, exe: &Path) -> Result<PathBuf, String> {
    let pid = std::process::id();
    let image = exe
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| "gentle-update-panel".to_string());
    let script = directory.join("panel-update.ps1");

    // `/R` lets the installer restart the new build itself, which is the path that
    // gives it a normal window: starting it by hand the instant the installer
    // returns left the panel hidden with an empty webview. The fallback waits
    // longer and marks the launch so the app insists on showing itself.
    let body = format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         Wait-Process -Id {pid} -Timeout 120\n\
         Start-Process -FilePath '{installer}' -ArgumentList '/S','/R' -Wait\n\
         Start-Sleep -Seconds 4\n\
         if (-not (Get-Process -Name '{image}' -ErrorAction SilentlyContinue)) {{\n\
         \x20 Start-Process -FilePath '{exe}' -ArgumentList '--updated' -WindowStyle Normal\n\
         }}\n\
         Remove-Item -LiteralPath '{script}' -Force\n",
        pid = pid,
        installer = ps_path(installer),
        image = image,
        exe = ps_path(exe),
        script = ps_path(&script),
    );

    std::fs::write(&script, body).map_err(|error| error.to_string())?;
    Ok(script)
}

/// PowerShell single-quoted strings need their quotes doubled.
fn ps_path(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "panel-setup.exe".to_string()
    } else {
        cleaned
    }
}

fn spawn_helper(script: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Stdio;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

        // DETACHED_PROCESS is not an option here: a console-less PowerShell never
        // runs the script (measured). A hidden console plus its own process group
        // keeps the helper alive after this process exits, which is all we need.
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(script)
            .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = script;
        Err("error.panel.windowsOnly".to_string())
    }
}

fn emit(app: &AppHandle, progress: PanelProgress) {
    let _ = app.emit(EVENT, progress);
}

fn fail(app: &AppHandle, message: &str) -> Result<(), String> {
    emit(
        app,
        PanelProgress {
            phase: "failed".to_string(),
            received: 0,
            total: 0,
        },
    );
    Err(message.to_string())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHECKED_AT: &str = "2026-10-30T10:00:00+00:00";

    fn release(tag: &str, assets: &[(&str, &str, u64)]) -> String {
        let assets: Vec<String> = assets
            .iter()
            .map(|(name, url, size)| {
                format!(
                    r#"{{"name":"{name}","browser_download_url":"{url}","size":{size}}}"#
                )
            })
            .collect();
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://github.com/killo3967/gentle-update-panel/releases/tag/{tag}","assets":[{}]}}"#,
            assets.join(",")
        )
    }

    #[test]
    fn compares_versions_without_trusting_the_v_prefix() {
        assert!(is_newer("0.6.0", "0.5.0"));
        assert!(is_newer("v0.6.0", "0.5.0"));
        assert!(is_newer("0.5.1", "0.5.0"));
        assert!(!is_newer("0.5.0", "0.5.0"));
        assert!(!is_newer("0.4.9", "0.5.0"));
        assert!(!is_newer("latest", "0.5.0"));
        assert!(!is_newer("0.6.0", "no-version"));
    }

    #[test]
    fn reads_a_newer_release_with_its_installer() {
        let body = release(
            "v0.6.0",
            &[
                ("Gentle.Update.Panel_0.6.0_x64-setup.exe", "https://example/installer.exe", 1_961_472),
                ("source.tar.gz", "https://example/source.tar.gz", 100),
            ],
        );
        let update = parse_release(&body, "0.5.0", CHECKED_AT).expect("debería haber actualización");
        assert_eq!(update.latest, "0.6.0");
        assert_eq!(update.current, "0.5.0");
        assert_eq!(update.asset, "Gentle.Update.Panel_0.6.0_x64-setup.exe");
        assert_eq!(update.url, "https://example/installer.exe");
        assert_eq!(update.size, 1_961_472);
        assert_eq!(update.checked_at, CHECKED_AT);
        assert!(update.release_url.ends_with("/v0.6.0"));
    }

    #[test]
    fn stays_quiet_when_the_release_is_not_newer() {
        let body = release(
            "v0.5.0",
            &[("Gentle.Update.Panel_0.5.0_x64-setup.exe", "https://example/installer.exe", 1)],
        );
        assert_eq!(parse_release(&body, "0.5.0", CHECKED_AT), None);
    }

    #[test]
    fn ignores_a_release_without_a_windows_installer() {
        let body = release("v0.6.0", &[("source.tar.gz", "https://example/source.tar.gz", 100)]);
        assert_eq!(parse_release(&body, "0.5.0", CHECKED_AT), None);
    }

    #[test]
    fn survives_a_payload_that_is_not_a_release() {
        assert_eq!(parse_release("{\"message\":\"Not Found\"}", "0.5.0", CHECKED_AT), None);
        assert_eq!(parse_release("", "0.5.0", CHECKED_AT), None);
    }

    #[test]
    fn keeps_a_downloaded_name_safe() {
        assert_eq!(
            sanitize("Gentle.Update.Panel_0.6.0_x64-setup.exe"),
            "Gentle.Update.Panel_0.6.0_x64-setup.exe"
        );
        assert_eq!(sanitize("../../evil.exe"), ".._.._evil.exe");
        assert_eq!(sanitize(""), "panel-setup.exe");
    }

    #[test]
    fn the_helper_waits_and_relaunches_through_the_installer() {
        let directory = std::env::temp_dir().join("gentle-panel-helper-test");
        std::fs::create_dir_all(&directory).expect("temp dir");
        let script = write_helper(
            &directory,
            Path::new("C:/tmp/Gentle_0.7.1_x64-setup.exe"),
            Path::new("C:/Apps/gentle-update-panel.exe"),
        )
        .expect("script");
        let body = std::fs::read_to_string(&script).expect("contenido");

        // The installer restarts the app; we only start it if that did not happen,
        // and then the app is told to insist on showing its window.
        assert!(body.contains("'/S','/R'"), "{body}");
        assert!(body.contains("Start-Sleep -Seconds 4"), "{body}");
        assert!(body.contains("--updated"), "{body}");
        assert!(body.contains("-WindowStyle Normal"), "{body}");
        assert!(body.contains("Wait-Process -Id"), "{body}");

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn refuses_a_file_that_is_not_an_installer() {
        let directory = std::env::temp_dir().join("gentle-panel-verify-test");
        std::fs::create_dir_all(&directory).expect("temp dir");
        let small = directory.join("small.exe");
        std::fs::write(&small, b"MZ").expect("write");
        assert!(verify_installer(&small, 0).is_err());

        let fake = directory.join("fake.exe");
        let mut bytes = vec![0u8; MIN_INSTALLER_BYTES as usize + 1];
        bytes[0] = b'M';
        bytes[1] = b'Z';
        std::fs::write(&fake, &bytes).expect("write");
        assert!(verify_installer(&fake, bytes.len() as u64).is_ok());
        assert!(verify_installer(&fake, bytes.len() as u64 + 1).is_err());

        bytes[0] = b'<';
        std::fs::write(&fake, &bytes).expect("write");
        assert!(verify_installer(&fake, bytes.len() as u64).is_err());

        std::fs::remove_dir_all(&directory).ok();
    }
}
