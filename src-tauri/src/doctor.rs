//! `gentle-ai doctor` as a first-class action, with per-check ignore rules.
//!
//! The policy lives here (not in the webview) so it is testable: a check is
//! hidden when its id is in `config.doctor_ignored`, or when the ecosystem
//! reports that tool as not installed and the user enabled that rule.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::cli;
use crate::domain::{Config, DoctorReport, Snapshot, Status};
use crate::parsers;

const DOCTOR_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorCheckView {
    pub id: String,
    pub marker: String,
    pub message: String,
    pub remedy: Option<String>,
    pub ignored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorView {
    pub checks: Vec<DoctorCheckView>,
    pub ignored_count: usize,
    /// Effective status once ignored checks are removed.
    pub status: String,
    /// What the CLI actually said, kept for honesty.
    pub original_status: String,
    pub summary: Option<String>,
    pub raw: String,
}

/// Applies the ignore rules to a parsed report.
pub fn view(report: &DoctorReport, config: &Config, snapshot: Option<&Snapshot>) -> DoctorView {
    let missing: Vec<&str> = snapshot
        .map(|snapshot| {
            snapshot
                .items
                .iter()
                .filter(|item| item.status == Status::Missing)
                .map(|item| item.id.as_str())
                .collect()
        })
        .unwrap_or_default();

    let checks: Vec<DoctorCheckView> = report
        .checks
        .iter()
        .map(|check| {
            let tool_missing = check
                .id
                .strip_prefix("tool:")
                .map(|tool| missing.iter().any(|missing| *missing == tool))
                .unwrap_or(false);

            let ignored = config.doctor_ignored.iter().any(|id| id == &check.id)
                || (config.ignore_missing_tools && tool_missing);

            DoctorCheckView {
                id: check.id.clone(),
                marker: check.marker.clone(),
                message: check.message.clone(),
                remedy: check.remedy.clone(),
                ignored,
            }
        })
        .collect();

    let active: Vec<&DoctorCheckView> = checks.iter().filter(|check| !check.ignored).collect();
    let has_issue = active.iter().any(|check| check.marker != "ok");

    let status = if report.checks.is_empty() {
        report.status.clone()
    } else if has_issue {
        if report.status == "ok" {
            "degraded".to_string()
        } else {
            report.status.clone()
        }
    } else {
        "ok".to_string()
    };

    DoctorView {
        ignored_count: checks.iter().filter(|check| check.ignored).count(),
        checks,
        status,
        original_status: report.status.clone(),
        summary: report.summary.clone(),
        raw: report.raw.clone(),
    }
}

/// Runs the CLI, stores the raw report, and returns the filtered view.
pub async fn run_view(app: &AppHandle) -> DoctorView {
    let report = run().await;
    let state = app.state::<crate::state::AppState>();
    state.store_doctor(&report);
    view(&report, &state.config(), state.snapshot().as_ref())
}

/// Spanish label for a doctor status, shared by the journal and the UI copy.
pub fn status_label(status: &str) -> &'static str {
    match status.to_lowercase().as_str() {
        "ok" | "healthy" => "correcto",
        "degraded" | "warn" => "degradado",
        "failed" | "fail" => "con fallos",
        "timeout" => "sin respuesta",
        _ => "sin datos",
    }
}

/// One line worth keeping in the journal: how many checks ran and what failed.
pub fn summary_line(view: &DoctorView) -> String {
    let active = view.checks.iter().filter(|check| !check.ignored).count();
    let problems: Vec<&DoctorCheckView> = view
        .checks
        .iter()
        .filter(|check| !check.ignored && check.marker != "ok")
        .collect();

    let mut parts = vec![format!(
        "{active} {}",
        if active == 1 {
            "comprobación"
        } else {
            "comprobaciones"
        }
    )];
    if !problems.is_empty() {
        parts.push(format!("{} con problemas", problems.len()));
        if let Some(first) = problems.first() {
            parts.push(format!("{}: {}", first.id, first.message));
        }
    }
    if view.ignored_count > 0 {
        parts.push(format!("{} silenciadas", view.ignored_count));
    }

    parts.join(" · ")
}

pub async fn run() -> DoctorReport {
    let args = vec!["doctor".to_string()];
    match cli::run("gentle-ai", &args, DOCTOR_TIMEOUT).await {
        Ok(output) => {
            let mut report = parsers::parse_doctor(&output.stdout);
            if report.checks.is_empty() {
                // Nothing parsed: keep the raw text so the UI can show why.
                report.raw = output.combined();
                report.status = if output.ok() {
                    "unknown".to_string()
                } else {
                    "failed".to_string()
                };
            } else if output.timed_out {
                report.status = "timeout".to_string();
            }
            report
        }
        Err(error) => DoctorReport {
            checks: Vec::new(),
            summary: None,
            status: "failed".to_string(),
            raw: error,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ItemKind, UpdateItem, Upgrade};
    use crate::parsers::DoctorCheck;

    fn check(id: &str, marker: &str) -> DoctorCheck {
        DoctorCheck {
            id: id.to_string(),
            marker: marker.to_string(),
            message: format!("{id} message"),
            remedy: None,
        }
    }

    fn report(checks: Vec<DoctorCheck>, status: &str) -> DoctorReport {
        DoctorReport {
            checks,
            summary: Some("5 passed, 0 failed, 3 warnings".to_string()),
            status: status.to_string(),
            raw: "raw".to_string(),
        }
    }

    fn item(id: &str, status: Status) -> UpdateItem {
        UpdateItem {
            id: id.to_string(),
            kind: ItemKind::Ecosystem,
            name: id.to_string(),
            source: None,
            installed: None,
            latest: Some("1.0.0".to_string()),
            status,
            detail: None,
            upgrade: Upgrade::GentleAiTool {
                tool: id.to_string(),
            },
            notes_key: None,
            ignored: false,
        }
    }

    #[test]
    fn always_ignored_check_disappears_and_status_heals() {
        let report = report(
            vec![check("tool:gentle-ai", "ok"), check("tool:hermes", "!!")],
            "degraded",
        );
        let mut config = Config::default();
        config.doctor_ignored = vec!["tool:hermes".to_string()];

        let view = view(&report, &config, None);

        assert_eq!(view.ignored_count, 1);
        assert_eq!(view.status, "ok", "the only warning is ignored");
        assert_eq!(view.original_status, "degraded");
        assert!(view.checks.iter().find(|c| c.id == "tool:hermes").unwrap().ignored);
    }

    #[test]
    fn missing_tool_is_ignored_only_when_the_rule_is_on() {
        let report = report(
            vec![
                check("tool:gentle-ai", "ok"),
                check("tool:gga", "!!"),
                check("disk:space", "ok"),
            ],
            "degraded",
        );
        let snapshot = Snapshot {
            checked_at: "now".to_string(),
            duration_ms: 0,
            items: vec![
                item("gga", Status::Missing),
                item("engram", Status::UpToDate),
            ],
            warnings: Vec::new(),
        };

        let mut config = Config::default();
        assert_eq!(
            view(&report, &config, Some(&snapshot)).status,
            "degraded",
            "rule off: gga still warns"
        );

        config.ignore_missing_tools = true;
        let filtered = view(&report, &config, Some(&snapshot));

        assert_eq!(filtered.status, "ok");
        assert_eq!(filtered.ignored_count, 1);
        assert!(filtered.checks.iter().find(|c| c.id == "tool:gga").unwrap().ignored);
        assert!(
            !filtered.checks.iter().find(|c| c.id == "disk:space").unwrap().ignored,
            "only tool checks of missing tools are hidden"
        );
    }

    #[test]
    fn installed_tool_is_never_auto_ignored() {
        let report = report(vec![check("tool:gga", "!!")], "degraded");
        let snapshot = Snapshot {
            checked_at: "now".to_string(),
            duration_ms: 0,
            items: vec![item("gga", Status::UpdateAvailable)],
            warnings: Vec::new(),
        };
        let mut config = Config::default();
        config.ignore_missing_tools = true;

        let view = view(&report, &config, Some(&snapshot));
        assert_eq!(view.ignored_count, 0);
        assert_eq!(view.status, "degraded");
    }

    #[test]
    fn report_without_checks_keeps_its_own_status() {
        let report = report(Vec::new(), "failed");
        let view = view(&report, &Config::default(), None);
        assert_eq!(view.status, "failed");
        assert_eq!(view.ignored_count, 0);
    }
}
