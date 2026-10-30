//! Types shared by the Rust core and the webview.

use serde::{Deserialize, Serialize};

pub use crate::parsers::{DoctorReport, ItemKind, Release, Status, Upgrade};

/// One row in the panel: a tool, pi itself, or an installed pi package.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateItem {
    pub id: String,
    pub kind: ItemKind,
    pub name: String,
    /// Raw declaration such as `npm:gentle-pi` or `git:github.com/...`.
    pub source: Option<String>,
    pub installed: Option<String>,
    pub latest: Option<String>,
    pub status: Status,
    /// Human hint shown under the row (why it is unknown, what to do).
    pub detail: Option<String>,
    pub upgrade: Upgrade,
    /// Where release notes come from: `pi:changelog`, `gh:owner/repo`, `npm:name`.
    pub notes_key: Option<String>,
    /// Excluded from counters, notifications and batches; still listed so it can
    /// be un-ignored later.
    pub ignored: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub checked_at: String,
    pub duration_ms: u64,
    pub items: Vec<UpdateItem>,
    /// Non-fatal problems found while probing (tool missing, registry unreachable…).
    pub warnings: Vec<String>,
}

impl Snapshot {
    /// Re-applies the configuration's ignore rules to a stored snapshot, so the
    /// UI never shows a state that contradicts `config.json`.
    pub fn apply_ignored(&mut self, ignored: &[String], ignore_missing_tools: bool) {
        for item in &mut self.items {
            let manual = ignored.iter().any(|id| id == &item.id);
            // A tool the ecosystem reports as not installed was disabled on
            // purpose; its "missing" row is noise, not an error. pi itself and
            // its packages are never hidden: there, missing is a real breakage.
            let disabled_tool = ignore_missing_tools
                && item.kind == ItemKind::Ecosystem
                && item.status == Status::Missing;
            item.ignored = manual || disabled_tool;
        }
    }

    pub fn needs_attention(&self) -> usize {
        self.items
            .iter()
            .filter(|item| {
                !item.ignored && matches!(item.status, Status::Missing | Status::UpdateAvailable)
            })
            .count()
    }

    pub fn doctor_hint(&self, previous: Option<&Snapshot>) -> Vec<String> {
        let current: Vec<String> = self
            .items
            .iter()
            .filter(|item| !item.ignored && item.status == Status::UpdateAvailable)
            .map(|item| item.id.clone())
            .collect();

        match previous {
            None => current,
            Some(previous) => {
                let known: Vec<&String> = previous
                    .items
                    .iter()
                    .filter(|item| !item.ignored && item.status == Status::UpdateAvailable)
                    .map(|item| &item.id)
                    .collect();
                current
                    .into_iter()
                    .filter(|id| !known.contains(&id))
                    .collect()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Automation {
    /// Tray badge and toast only.
    Notify,
    /// Badge plus a toast that opens the panel with the batch prepared.
    Confirm,
    /// Apply every available update as soon as it is detected.
    Auto,
}

impl Default for Automation {
    fn default() -> Self {
        Self::Notify
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// Minutes between background checks. 0 pauses them.
    pub interval_minutes: u64,
    pub automation: Automation,
    pub notify_on_update: bool,
    pub doctor_after_update: bool,
    /// Start hidden when Windows launches the app.
    pub start_minimized: bool,
    /// Last window the user was looking at, restored on open.
    pub last_view: String,
    /// Last piece the user opened, restored on open.
    pub last_selected_id: Option<String>,
    /// Item ids the user does not want to be bothered about.
    pub ignored: Vec<String>,
    /// Doctor check ids hidden from the health view, e.g. `tool:hermes`.
    pub doctor_ignored: Vec<String>,
    /// Hide `tool:<name>` checks *and* ecosystem rows for tools the ecosystem
    /// reports as not installed (a deliberately disabled tool is not an error).
    #[serde(alias = "doctorIgnoreMissingTools")]
    pub ignore_missing_tools: bool,
    /// Translate release notes on demand.
    pub translation_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_minutes: 360,
            automation: Automation::Notify,
            notify_on_update: true,
            doctor_after_update: true,
            start_minimized: true,
            last_view: "overview".to_string(),
            last_selected_id: None,
            ignored: Vec::new(),
            doctor_ignored: Vec::new(),
            ignore_missing_tools: false,
            translation_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub config: Config,
    pub snapshot: Option<Snapshot>,
    pub doctor: Option<crate::doctor::DoctorView>,
    pub pi_health: Option<crate::health::PiHealth>,
    /// What the panel detected and did, oldest first.
    pub journal: Vec<crate::journal::JournalEntry>,
    /// Where the journal lives, so the UI can point at the file.
    pub journal_path: String,
    pub autostart: bool,
    pub version: String,
    pub app_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyStep {
    pub id: String,
    pub label: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyFailure {
    pub id: String,
    pub label: String,
    pub reason: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyOutcome {
    pub applied: Vec<String>,
    pub failed: Vec<ApplyFailure>,
    pub doctor: Option<crate::doctor::DoctorView>,
    pub snapshot: Option<Snapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotesPayload {
    pub key: String,
    pub title: String,
    pub releases: Vec<Release>,
    /// Link the user can open when the panel has nothing better to show.
    pub url: Option<String>,
    pub fetched_at: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub phase: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEvent {
    pub id: String,
    pub stream: String,
    pub line: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, status: Status, ignored: bool) -> UpdateItem {
        UpdateItem {
            id: id.to_string(),
            kind: ItemKind::PiPackage,
            name: id.to_string(),
            source: None,
            installed: Some("1.0.0".to_string()),
            latest: Some("1.1.0".to_string()),
            status,
            detail: None,
            upgrade: Upgrade::PiSelf,
            notes_key: None,
            ignored,
        }
    }

    fn snapshot(items: Vec<UpdateItem>) -> Snapshot {
        Snapshot {
            checked_at: "now".to_string(),
            duration_ms: 0,
            items,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn apply_ignored_marks_only_listed_items_both_ways() {
        let mut report = snapshot(vec![
            item("a", Status::UpdateAvailable, false),
            item("b", Status::UpdateAvailable, false),
        ]);

        report.apply_ignored(&["a".to_string()], false);
        assert!(report.items[0].ignored);
        assert!(!report.items[1].ignored);
        assert_eq!(report.needs_attention(), 1);

        report.apply_ignored(&[], false);
        assert!(report.items.iter().all(|item| !item.ignored));
        assert_eq!(report.needs_attention(), 2);
    }

    #[test]
    fn missing_ecosystem_tool_is_hidden_when_the_rule_is_on() {
        let mut report = snapshot(vec![
            item("gga", Status::Missing, false),
            item("engram", Status::UpToDate, false),
            item("pi", Status::Missing, false),
        ]);
        report.items[0].kind = ItemKind::Ecosystem;
        report.items[1].kind = ItemKind::Ecosystem;
        report.items[2].kind = ItemKind::Pi;
        let mut package = item("pkg:npm:ghost", Status::Missing, false);
        package.kind = ItemKind::PiPackage;
        report.items.push(package);

        report.apply_ignored(&[], true);

        assert!(report.items[0].ignored, "gga: ecosystem tool reported as not installed");
        assert!(!report.items[1].ignored, "installed tool stays visible");
        assert!(!report.items[2].ignored, "a missing pi core is never hidden");
        assert!(!report.items[3].ignored, "a missing package is never hidden");
        assert_eq!(report.needs_attention(), 2, "pi + package, not gga");
    }

    #[test]
    fn missing_ecosystem_tool_stays_visible_when_the_rule_is_off() {
        let mut report = snapshot(vec![item("gga", Status::Missing, false)]);
        report.items[0].kind = ItemKind::Ecosystem;
        report.apply_ignored(&[], false);
        assert!(!report.items[0].ignored);
        assert_eq!(report.needs_attention(), 1);
    }

    #[test]
    fn manual_ignore_still_wins_over_the_rule() {
        let mut report = snapshot(vec![item("engram", Status::UpToDate, false)]);
        report.apply_ignored(&["engram".to_string()], false);
        assert!(report.items[0].ignored);
    }

    #[test]
    fn counters_skip_ignored_items() {
        let report = snapshot(vec![
            item("a", Status::UpdateAvailable, true),
            item("b", Status::UpdateAvailable, false),
            item("gga", Status::Missing, true),
            item("pi", Status::UpToDate, false),
        ]);

        assert_eq!(report.needs_attention(), 1, "only the non-ignored pending item counts");
    }

    #[test]
    fn fresh_hints_skip_ignored_items() {
        let report = snapshot(vec![
            item("a", Status::UpdateAvailable, true),
            item("b", Status::UpdateAvailable, false),
        ]);

        assert_eq!(report.doctor_hint(None), vec!["b".to_string()]);
    }
}
