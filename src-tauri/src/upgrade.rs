//! Applying the selected updates, streaming each command's output.

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::domain::{ApplyFailure, ApplyOutcome, ApplyStep, LogEvent, Snapshot, Status, Upgrade};
use crate::state::AppState;
use crate::{cli, doctor};

const UPGRADE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// One executable step: the UI-facing description plus the argv to run.
pub struct PlannedStep {
    pub step: ApplyStep,
    pub program: String,
    pub args: Vec<String>,
}

pub fn plan(snapshot: &Snapshot, ids: &[String]) -> Vec<PlannedStep> {
    let mut steps = Vec::new();

    for item in &snapshot.items {
        if !ids.iter().any(|id| id == &item.id) {
            continue;
        }
        // Only actionable rows make sense in a batch.
        if !matches!(item.status, Status::UpdateAvailable | Status::Missing) {
            continue;
        }
        // Ignoring wins over an explicit request: the list is a promise.
        if item.ignored {
            continue;
        }

        let versions = match (&item.installed, &item.latest) {
            (Some(installed), Some(latest)) => format!("{installed} → {latest}"),
            (None, Some(latest)) => format!("instalar {latest}"),
            _ => item.name.clone(),
        };
        let label = format!("{} · {versions}", item.name);

        let planned = match &item.upgrade {
            Upgrade::GentleAiTool { tool } => PlannedStep {
                step: ApplyStep {
                    id: item.id.clone(),
                    label,
                    command: format!("gentle-ai upgrade {tool}"),
                },
                program: "gentle-ai".to_string(),
                args: vec!["upgrade".to_string(), tool.clone()],
            },
            Upgrade::PiSelf => PlannedStep {
                step: ApplyStep {
                    id: item.id.clone(),
                    label,
                    command: "pi update --self".to_string(),
                },
                program: "pi".to_string(),
                args: vec!["update".to_string(), "--self".to_string()],
            },
            Upgrade::PiPackage { source } => PlannedStep {
                step: ApplyStep {
                    id: item.id.clone(),
                    label,
                    command: format!("pi update --extension {source}"),
                },
                program: "pi".to_string(),
                args: vec![
                    "update".to_string(),
                    "--extension".to_string(),
                    source.clone(),
                ],
            },
            Upgrade::Manual { hint } => PlannedStep {
                step: ApplyStep {
                    id: item.id.clone(),
                    label: format!("{} · {hint}", item.name),
                    command: String::new(),
                },
                program: String::new(),
                args: Vec::new(),
            },
        };
        steps.push(planned);
    }

    steps
}

/// Runs every step in plan order and then `gentle-ai doctor` when configured.
/// The caller is responsible for re-checking afterwards.
pub async fn apply(app: &AppHandle, ids: Vec<String>) -> ApplyOutcome {
    let Some(snapshot) = app.state::<AppState>().snapshot() else {
        return ApplyOutcome {
            applied: Vec::new(),
            failed: vec![ApplyFailure {
                id: "state".to_string(),
                label: "Estado".to_string(),
                reason: "no hay una comprobación guardada; comprueba de nuevo".to_string(),
                output: String::new(),
            }],
            doctor: None,
            snapshot: None,
        };
    };

    let steps = plan(&snapshot, &ids);
    let mut applied = Vec::new();
    let mut failed = Vec::new();

    for (index, planned) in steps.iter().enumerate() {
        let _ = app.emit(
            "apply:step",
            serde_json::json!({
                "id": planned.step.id,
                "label": planned.step.label,
                "command": planned.step.command,
                "index": index,
                "total": steps.len(),
            }),
        );

        if planned.program.is_empty() {
            failed.push(ApplyFailure {
                id: planned.step.id.clone(),
                label: planned.step.label.clone(),
                reason: "esta pieza no se actualiza automáticamente".to_string(),
                output: String::new(),
            });
            continue;
        }

        let _ = app.emit(
            "apply:log",
            LogEvent {
                id: planned.step.id.clone(),
                stream: "command".to_string(),
                line: format!("$ {}", planned.step.command),
            },
        );

        let app_for_lines = app.clone();
        let id_for_lines = planned.step.id.clone();
        let result = cli::run_streaming(
            &planned.program,
            &planned.args,
            UPGRADE_TIMEOUT,
            move |line| {
                let _ = app_for_lines.emit(
                    "apply:log",
                    LogEvent {
                        id: id_for_lines.clone(),
                        stream: "stdout".to_string(),
                        line: line.to_string(),
                    },
                );
            },
        )
        .await;

        match result {
            Ok(output) if output.ok() => {
                applied.push(planned.step.id.clone());
                let _ = app.emit(
                    "apply:log",
                    LogEvent {
                        id: planned.step.id.clone(),
                        stream: "done".to_string(),
                        line: format!("{} actualizado", planned.step.label),
                    },
                );
            }
            Ok(output) => {
                let reason = if output.timed_out {
                    "la actualización excedió el tiempo máximo".to_string()
                } else {
                    let stderr = output.stderr.trim();
                    let stdout = output.stdout.trim();
                    let detail = if !stderr.is_empty() { stderr } else { stdout };
                    format!(
                        "salió con código {}{}",
                        output
                            .code
                            .map(|code| code.to_string())
                            .unwrap_or_else(|| "desconocido".to_string()),
                        if detail.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", first_lines(detail, 6))
                        }
                    )
                };
                let combined = output.combined();
                let tool = planned
                    .step
                    .label
                    .split(" · ")
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let hint = crate::blockers::lock_hint(&tool, &combined, &crate::blockers::detect());
                let reason = match hint {
                    Some(hint) => format!("{reason} · {hint}"),
                    None => reason,
                };

                failed.push(ApplyFailure {
                    id: planned.step.id.clone(),
                    label: planned.step.label.clone(),
                    reason,
                    output: combined,
                });
            }
            Err(error) => failed.push(ApplyFailure {
                id: planned.step.id.clone(),
                label: planned.step.label.clone(),
                reason: error,
                output: String::new(),
            }),
        }
    }

    let config = app.state::<AppState>().config();
    let doctor_report = if config.doctor_after_update && !steps.is_empty() {
        let _ = app.emit(
            "apply:log",
            LogEvent {
                id: "doctor".to_string(),
                stream: "command".to_string(),
                line: "$ gentle-ai doctor".to_string(),
            },
        );
        Some(doctor::run_view(app).await)
    } else {
        None
    };

    ApplyOutcome {
        applied,
        failed,
        doctor: doctor_report,
        snapshot: None,
    }
}

fn first_lines(text: &str, count: usize) -> String {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .take(count)
        .collect::<Vec<_>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ItemKind;

    fn item(id: &str, upgrade: Upgrade, status: Status) -> crate::domain::UpdateItem {
        crate::domain::UpdateItem {
            id: id.to_string(),
            kind: ItemKind::PiPackage,
            name: id.to_string(),
            source: None,
            installed: Some("1.0.0".to_string()),
            latest: Some("1.1.0".to_string()),
            status,
            detail: None,
            upgrade,
            notes_key: None,
            ignored: false,
        }
    }

    #[test]
    fn plans_only_the_requested_items_in_snapshot_order() {
        let snapshot = Snapshot {
            checked_at: "now".to_string(),
            duration_ms: 0,
            items: vec![
                item("a", Upgrade::GentleAiTool { tool: "a".into() }, Status::UpdateAvailable),
                item("b", Upgrade::PiSelf, Status::UpdateAvailable),
                item("c", Upgrade::PiPackage { source: "npm:c".into() }, Status::UpdateAvailable),
            ],
            warnings: Vec::new(),
        };

        let steps = plan(&snapshot, &["c".to_string(), "a".to_string()]);
        assert_eq!(
            steps.iter().map(|step| step.step.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "c"],
            "selection order must not reorder the install plan"
        );
        assert_eq!(steps[0].step.command, "gentle-ai upgrade a");
        assert_eq!(steps[0].program, "gentle-ai");
        assert_eq!(steps[0].args, vec!["upgrade".to_string(), "a".to_string()]);
        assert_eq!(steps[1].step.command, "pi update --extension npm:c");
        assert_eq!(
            steps[1].args,
            vec![
                "update".to_string(),
                "--extension".to_string(),
                "npm:c".to_string()
            ],
            "package sources must reach pi verbatim, never re-split"
        );
    }

    #[test]
    fn skips_ignored_items() {
        let mut snapshot = Snapshot {
            checked_at: "now".to_string(),
            duration_ms: 0,
            items: vec![
                item("a", Upgrade::PiSelf, Status::UpdateAvailable),
                item("b", Upgrade::PiSelf, Status::UpdateAvailable),
            ],
            warnings: Vec::new(),
        };
        snapshot.items[0].ignored = true;

        let steps = plan(&snapshot, &["a".to_string(), "b".to_string()]);
        assert_eq!(
            steps.iter().map(|step| step.step.id.as_str()).collect::<Vec<_>>(),
            vec!["b"],
            "an ignored item must never be updated, even if it is requested"
        );
    }

    #[test]
    fn ignores_items_that_are_already_current() {
        let snapshot = Snapshot {
            checked_at: "now".to_string(),
            duration_ms: 0,
            items: vec![item("a", Upgrade::PiSelf, Status::UpToDate)],
            warnings: Vec::new(),
        };
        assert!(plan(&snapshot, &["a".to_string()]).is_empty());
    }
}
