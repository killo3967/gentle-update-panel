//! Commands exposed to the webview.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;

use crate::domain::{ApplyOutcome, AppStatus, Config, NotesPayload, Snapshot};
use crate::journal::{self, kind, Draft};
use crate::state::AppState;
use crate::blockers::{self, Blocker, Impact, PreflightReport, PreflightStep};
use crate::doctor::DoctorView;
use crate::health::{self, PiHealth};
use crate::notes::NoteHit;
use crate::translate::TranslatedText;
use crate::{doctor, service, upgrade};

/// Releases the "an update is running" flag even if the task panics.
struct ApplyingGuard(AppHandle);

impl Drop for ApplyingGuard {
    fn drop(&mut self) {
        if let Some(state) = self.0.try_state::<AppState>() {
            state.applying.store(false, Ordering::SeqCst);
        }
    }
}

#[tauri::command]
pub fn app_status(app: AppHandle) -> AppStatus {
    let state = app.state::<AppState>();
    AppStatus {
        config: state.config(),
        snapshot: state.snapshot(),
        doctor: state.doctor().map(|report| {
            doctor::view(&report, &state.config(), state.snapshot().as_ref())
        }),
        pi_health: state.pi_health(),
        panel_update: state.panel_update(),
        journal: journal::list(&app),
        journal_path: journal::path_string(&app),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        version: app.package_info().version.to_string(),
        app_dir: app
            .path()
            .app_config_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
    }
}

#[tauri::command]
pub fn save_config(app: AppHandle, mut config: Config) -> Config {
    if config.interval_minutes > 0 {
        config.interval_minutes = config.interval_minutes.clamp(5, 10_080);
    }
    if config.last_view.is_empty() {
        config.last_view = "overview".to_string();
    }
    // Ids come from the webview: trim, drop empties, dedupe and cap the list.
    let mut ignored: Vec<String> = config
        .ignored
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty() && id.len() <= 200)
        .collect();
    ignored.sort();
    ignored.dedup();
    ignored.truncate(200);
    config.ignored = ignored;

    let state = app.state::<AppState>();
    state.save_config(&config);
    if let Ok(mut current) = state.config.lock() {
        *current = config.clone();
    }

    // Keep the stored snapshot consistent with the new ignore list so counters,
    // the tray badge and the window update without waiting for a re-check.
    if let Some(mut snapshot) = state.snapshot() {
        snapshot.apply_ignored(&config.ignored, config.ignore_missing_tools);
        state.store_snapshot(&snapshot);
        crate::tray::reflect(&app);
        let _ = app.emit("snapshot", &snapshot);
    }

    config
}

#[tauri::command]
pub async fn check_now(app: AppHandle) -> Snapshot {
    service::run_check(&app).await
}

#[tauri::command]
pub async fn run_doctor(app: AppHandle) -> DoctorView {
    let view = doctor::run_view(&app).await;
    journal::record(
        &app,
        Draft::new(
            kind::DOCTOR,
            format!("Doctor manual: {}", doctor::status_label(&view.status)),
        )
        .detail(doctor::summary_line(&view)),
    );
    view
}

/// Empties the journal; the file stays so the next entry can be appended.
#[tauri::command]
pub fn clear_journal(app: AppHandle) {
    journal::clear(&app);
}

/// Recomputes the filtered view from the stored report, without running the CLI.
#[tauri::command]
pub fn doctor_view(app: AppHandle) -> Option<DoctorView> {
    let state = app.state::<AppState>();
    let report = state.doctor()?;
    Some(doctor::view(
        &report,
        &state.config(),
        state.snapshot().as_ref(),
    ))
}

#[tauri::command]
pub async fn run_pi_health(app: AppHandle) -> PiHealth {
    let report = health::run().await;
    if let Some(state) = app.try_state::<AppState>() {
        state.store_pi_health(&report);
    }
    report
}

#[tauri::command]
pub fn blockers() -> Vec<Blocker> {
    blockers::detect()
}

#[tauri::command]
pub fn apply_preflight(app: AppHandle, ids: Vec<String>) -> PreflightReport {
    let state = app.state::<AppState>();
    let running = blockers::detect();
    if !running.is_empty() {
        let detail = running
            .iter()
            .map(|blocker| format!("{} ({})", blocker.name, blocker.kind))
            .collect::<Vec<_>>()
            .join(", ");
        journal::record(
            &app,
            Draft::new(
                kind::PREFLIGHT,
                format!(
                    "{} {} en marcha antes de actualizar",
                    running.len(),
                    if running.len() == 1 {
                        "proceso"
                    } else {
                        "procesos"
                    }
                ),
            )
            .detail(detail),
        );
    }

    let Some(snapshot) = state.snapshot() else {
        return PreflightReport {
            blockers: running,
            steps: Vec::new(),
        };
    };

    let steps = upgrade::plan(&snapshot, &ids)
        .into_iter()
        .map(|planned| {
            let (impact, reason) = snapshot
                .items
                .iter()
                .find(|item| item.id == planned.step.id)
                .map(|item| blockers::impact(&item.id, item.kind, &running))
                .unwrap_or((Impact::None, None));

            PreflightStep {
                id: planned.step.id,
                label: planned.step.label,
                impact,
                reason,
            }
        })
        .collect();

    PreflightReport {
        blockers: running,
        steps,
    }
}

#[tauri::command]
pub async fn apply_updates(app: AppHandle, ids: Vec<String>) -> Result<ApplyOutcome, String> {
    {
        let state = app.state::<AppState>();
        if state.applying.swap(true, Ordering::SeqCst) {
            return Err("ya hay una actualización en curso".to_string());
        }
    }
    let _guard = ApplyingGuard(app.clone());

    // Wait for a running check to finish before touching binaries.
    let lock = app.state::<AppState>().work_lock.clone();
    let _work = lock.lock().await;

    let previous = app.state::<AppState>().snapshot();
    let outcome = upgrade::apply(&app, ids, upgrade::Origin::Manual).await;
    let snapshot = service::collect(&app).await;
    service::record_summary(
        &app,
        &snapshot,
        previous.as_ref(),
        "Comprobación posterior a la actualización",
    );

    Ok(ApplyOutcome {
        snapshot: Some(snapshot),
        ..outcome
    })
}

/// Hands the panel's own update to the helper, which waits for this process to
/// exit before installing. The app comes back already updated.
#[tauri::command]
pub async fn apply_panel_update(app: AppHandle) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        if state.applying.swap(true, Ordering::SeqCst) {
            return Err("ya hay una actualización en curso".to_string());
        }
    }
    let _guard = ApplyingGuard(app.clone());

    // Nothing else may touch binaries while the installer is being prepared.
    let lock = app.state::<AppState>().work_lock.clone();
    let _work = lock.lock().await;

    crate::panel::apply(&app).await
}

#[tauri::command]
pub async fn get_notes(
    app: AppHandle,
    key: String,
    installed: Option<String>,
    latest: Option<String>,
    force: bool,
) -> NotesPayload {
    let notes: Arc<_> = {
        let state = app.state::<AppState>();
        state.notes.clone()
    };
    notes
        .get(&key, installed.as_deref(), latest.as_deref(), force)
        .await
}

#[tauri::command]
pub async fn translate_release(
    app: AppHandle,
    cache_key: String,
    text: String,
    force: bool,
) -> Result<TranslatedText, String> {
    let (service, config) = {
        let state = app.state::<AppState>();
        (state.translations.clone(), state.config())
    };

    if !config.translation_enabled {
        return Err("la traducción está desactivada".to_string());
    }

    service
        .translate(
            &cache_key,
            &text,
            // Google first, Microsoft as the fallback: there is no engine choice.
            "auto",
            &crate::locale::target_language(),
            force,
        )
        .await
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|error| error.to_string())?;
    Ok(manager.is_enabled().unwrap_or(enabled))
}

#[tauri::command]
pub fn search_notes(app: AppHandle, query: String) -> Vec<NoteHit> {
    app.state::<AppState>().notes.search(&query)
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    let trimmed = url.trim();
    if !trimmed.starts_with("https://") || trimmed.len() > 2048 {
        return Err("solo se abren enlaces https".to_string());
    }
    if trimmed.chars().any(|c| c.is_whitespace() || c == '"') {
        return Err("enlace inválido".to_string());
    }

    std::process::Command::new("cmd")
        .args(["/C", "start", "", &trimmed])
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}
