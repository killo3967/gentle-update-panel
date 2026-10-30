//! The check itself: collect, persist, journal, reflect in the tray, notify.

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::domain::{Automation, ProgressEvent, Snapshot, Status, UpdateItem};
use crate::journal::{self, kind, Draft};
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
    tray::reflect(app);
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
    // The panel's own release is checked in the same cycle, so it lands in the
    // tray, the notification and the journal like any other piece.
    crate::panel::refresh(app).await;
    record_summary(
        app,
        &snapshot,
        previous.as_ref(),
        "Comprobación completada",
        "journal.check.done",
    );
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
        let before = snapshot.clone();
        tauri::async_runtime::spawn(async move {
            let outcome = upgrade::apply(&handle, ids, upgrade::Origin::Auto).await;
            let _ = handle.emit("apply:done", &outcome);
            let after = collect(&handle).await;
            record_summary(
                &handle,
                &after,
                Some(&before),
                "Comprobación posterior a la actualización",
                "journal.check.afterUpdate",
            );
        });
    }

    snapshot
}

/// Journals the outcome of a check: one summary line plus one line per new find.
pub fn record_summary(
    app: &AppHandle,
    snapshot: &Snapshot,
    previous: Option<&Snapshot>,
    title: &str,
    key: &str,
) {
    let pending: Vec<&UpdateItem> = snapshot
        .items
        .iter()
        .filter(|item| is_pending(item))
        .collect();

    let seconds = (snapshot.duration_ms as f64 / 1000.0).round().max(1.0) as u64;
    let mut detail = format!(
        "{} piezas · {} pendientes · {seconds} s",
        snapshot.items.len(),
        pending.len()
    );
    if !snapshot.warnings.is_empty() {
        detail.push_str(&format!(" · {} avisos", snapshot.warnings.len()));
    }

    let mut draft = Draft::new(kind::CHECK, title)
        .key(key)
        .detail(detail)
        .detail_key("journal.check.detail")
        .detail_arg("items", snapshot.items.len().to_string())
        .detail_arg("pending", pending.len().to_string())
        .detail_arg("seconds", seconds.to_string());
    if !snapshot.warnings.is_empty() {
        draft = draft
            .detail_key("journal.check.detailWarnings")
            .detail_arg("warnings", snapshot.warnings.len().to_string());
    }
    journal::record(app, draft);

    // A pending row the stored snapshot did not have is news worth telling.
    for item in pending {
        let known = previous
            .map(|previous| {
                previous
                    .items
                    .iter()
                    .any(|known| known.id == item.id && known.status == item.status)
            })
            .unwrap_or(false);
        if known {
            continue;
        }

        let mut draft = Draft::new(kind::DETECTED, detection_title(item))
            .key(detection_key(item))
            .arg("name", item.name.clone())
            .item(&item.id);
        if let Some(versions) = detection_detail(item) {
            draft = draft.detail(versions);
        }
        journal::record(app, draft);
    }
}

fn is_pending(item: &UpdateItem) -> bool {
    !item.ignored && matches!(item.status, Status::UpdateAvailable | Status::Missing)
}

fn detection_key(item: &UpdateItem) -> &'static str {
    match item.status {
        Status::Missing => "journal.detected.missing",
        _ => "journal.detected.update",
    }
}

fn detection_title(item: &UpdateItem) -> String {
    match item.status {
        Status::Missing => format!("Pendiente de instalar: {}", item.name),
        _ => format!("Actualización detectada: {}", item.name),
    }
}

fn detection_detail(item: &UpdateItem) -> Option<String> {
    match (&item.installed, &item.latest) {
        (Some(installed), Some(latest)) => Some(format!("{installed} → {latest}")),
        (None, Some(latest)) => Some(format!("instalar {latest}")),
        (Some(installed), None) => Some(format!("instalada {installed}")),
        _ => None,
    }
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
        crate::i18n::t("notify.updates.one").to_string()
    } else {
        crate::i18n::t_args(
            "notify.updates.many",
            &[("count", &fresh.len().to_string())],
        )
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
