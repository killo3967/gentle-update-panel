//! The check itself: collect, persist, reflect in the tray, notify.

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::domain::{Automation, ProgressEvent, Snapshot};
use crate::probe::Prober;
use crate::state::AppState;
use crate::{tray, upgrade};

/// Runs the probes and publishes the result. Never notifies by itself.
pub async fn collect(app: &AppHandle) -> Snapshot {
    let prober = Prober::new();
    let config = app.state::<AppState>().config();
    let handle = app.clone();
    let mut snapshot = prober
        .collect(&config.ignored, move |phase, message| {
            let _ = handle.emit(
                "check:progress",
                ProgressEvent {
                    phase: phase.to_string(),
                    message: message.to_string(),
                },
            );
        })
        .await;

    // One place decides what is hidden: the manual list plus the disabled-tool rule.
    snapshot.apply_ignored(&config.ignored, config.ignore_missing_tools);

    if let Some(state) = app.try_state::<AppState>() {
        state.store_snapshot(&snapshot);
    }
    tray::reflect(app, &snapshot);
    let _ = app.emit("snapshot", &snapshot);
    snapshot
}

/// Full cycle used by the scheduler and the "check now" button.
pub async fn run_check(app: &AppHandle) -> Snapshot {
    // Wait for any update in progress instead of reading half-written binaries.
    let lock = app.state::<AppState>().work_lock.clone();
    let _work = lock.lock().await;

    let previous = app.state::<AppState>().snapshot();
    let config = app.state::<AppState>().config();

    let snapshot = collect(app).await;
    let fresh = snapshot.doctor_hint(previous.as_ref());

    if fresh.is_empty() {
        return snapshot;
    }

    if config.notify_on_update {
        notify(app, &snapshot, &fresh, config.automation);
    }

    if config.automation == Automation::Auto {
        let handle = app.clone();
        let ids = fresh.clone();
        tauri::async_runtime::spawn(async move {
            let outcome = upgrade::apply(&handle, ids).await;
            let _ = handle.emit("apply:done", &outcome);
            let _ = collect(&handle).await;
        });
    }

    snapshot
}

fn notify(app: &AppHandle, snapshot: &Snapshot, fresh: &[String], automation: Automation) {
    let names: Vec<String> = fresh
        .iter()
        .filter_map(|id| {
            snapshot
                .items
                .iter()
                .find(|item| &item.id == id)
                .map(|item| item.name.clone())
        })
        .collect();

    let title = if fresh.len() == 1 {
        "1 actualización disponible".to_string()
    } else {
        format!("{} actualizaciones disponibles", fresh.len())
    };

    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(names.join(", "))
        .show();

    if automation != Automation::Notify {
        tray::show_main(app);
        let _ = app.emit("preselect", fresh);
    }
}
