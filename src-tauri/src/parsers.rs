//! Pure parsers for the CLI output this panel depends on.
//!
//! Every function here is deterministic and side-effect free, which is why the
//! whole contract lives in tests: the CLIs print human text, not JSON.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    Ecosystem,
    Pi,
    PiPackage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    UpToDate,
    UpdateAvailable,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Upgrade {
    /// `gentle-ai upgrade <tool>`
    GentleAiTool { tool: String },
    /// `pi update --self`
    PiSelf,
    /// `pi update --extension <source>`
    PiPackage { source: String },
    /// Nothing this panel knows how to update.
    Manual { hint: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRow {
    pub name: String,
    pub installed: Option<String>,
    pub latest: Option<String>,
    pub marker: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageEntry {
    pub source: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorCheck {
    pub id: String,
    pub marker: String,
    pub message: String,
    pub remedy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
    pub summary: Option<String>,
    pub status: String,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    pub date: Option<String>,
    pub body: String,
}

// ---------------------------------------------------------------------------
// gentle-ai update
// ---------------------------------------------------------------------------

/// Parses the table printed by `gentle-ai update`.
///
/// Real shape:
/// ```text
///   [ok] gentle-ai     installed: 4.3.1       latest: 4.3.1
///   [??] gga           installed: -           latest: 2.10.1
/// ```
pub fn parse_gentle_ai_update(output: &str) -> Vec<ToolRow> {
    static LINE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = LINE.get_or_init(|| {
        regex::Regex::new(
            r"^\s*\[(?P<marker>[^\]]+)\]\s+(?P<name>[A-Za-z0-9_.-]+)\s+installed:\s*(?P<installed>\S+)\s+latest:\s*(?P<latest>\S+)\s*$",
        )
        .expect("static pattern")
    });

    output
        .lines()
        .filter_map(|line| {
            let caps = re.captures(line)?;
            Some(ToolRow {
                name: caps["name"].to_string(),
                installed: normalize_installed(&caps["installed"]),
                latest: normalize_installed(&caps["latest"]),
                marker: caps["marker"].to_string(),
            })
        })
        .collect()
}

/// `-`, empty or whitespace mean "not installed".
pub fn normalize_installed(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "-" || trimmed == "unknown" {
        None
    } else {
        Some(trimmed.trim_start_matches('v').to_string())
    }
}

// ---------------------------------------------------------------------------
// pi list
// ---------------------------------------------------------------------------

/// Parses `pi list`, keeping the declared source and its install path.
///
/// Annotations such as `(filtered)` belong to the CLI, not to the source.
pub fn parse_pi_list(output: &str) -> Vec<PackageEntry> {
    static ANNOTATION: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let annotation = ANNOTATION.get_or_init(|| {
        regex::Regex::new(r"\s+\([A-Za-z0-9 _-]+\)$").expect("static pattern")
    });

    let mut entries: Vec<PackageEntry> = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            continue;
        }
        let indent = trimmed.len() - trimmed.trim_start().len();
        let content = trimmed.trim_start();

        // Sources keep their declaration (`npm:…`, `git:…`); install paths are indented under them.
        if indent == 2 && !content.ends_with(':') {
            let source = annotation.replace(content, "").trim().to_string();
            entries.push(PackageEntry {
                source,
                path: String::new(),
            });
        } else if indent >= 4 {
            if let Some(last) = entries.last_mut() {
                if last.path.is_empty() {
                    last.path = content.to_string();
                }
            }
        }
    }

    entries
}

// ---------------------------------------------------------------------------
// gentle-ai doctor
// ---------------------------------------------------------------------------

pub fn parse_doctor(output: &str) -> DoctorReport {
    static CHECK: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let check_re = CHECK.get_or_init(|| {
        regex::Regex::new(r"^\s{2}\[(?P<marker>[^\]]+)\]\s{2,}(?P<id>\S+)\s{2,}(?P<msg>.+)$")
            .expect("static pattern")
    });

    let mut checks: Vec<DoctorCheck> = Vec::new();
    let mut summary = None;
    let mut status = "unknown".to_string();

    for line in output.lines() {
        if let Some(caps) = check_re.captures(line) {
            checks.push(DoctorCheck {
                id: caps["id"].to_string(),
                marker: caps["marker"].to_string(),
                message: caps["msg"].trim().to_string(),
                remedy: None,
            });
            continue;
        }

        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Remedy:") {
            if let Some(last) = checks.last_mut() {
                last.remedy = Some(rest.trim().to_string());
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Summary:") {
            summary = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Status:") {
            let value = rest.trim();
            if !value.is_empty() {
                status = value.to_string();
            }
        }
    }

    DoctorReport {
        checks,
        summary,
        status,
        raw: output.to_string(),
    }
}

// ---------------------------------------------------------------------------
// pi CHANGELOG.md
// ---------------------------------------------------------------------------

/// Splits the markdown changelog into releases, newest first, as written.
pub fn parse_changelog(markdown: &str) -> Vec<Release> {
    static HEADING: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let heading = HEADING.get_or_init(|| {
        regex::Regex::new(r"^##\s+\[(?P<version>[^\]]+)\](?:\s+-\s+(?P<date>\S+))?\s*$")
            .expect("static pattern")
    });

    let mut releases: Vec<Release> = Vec::new();
    let mut current: Option<Release> = None;

    for line in markdown.lines() {
        if let Some(caps) = heading.captures(line) {
            if let Some(done) = current.take() {
                releases.push(done);
            }
            current = Some(Release {
                version: caps["version"].trim().to_string(),
                date: caps.name("date").map(|m| m.as_str().to_string()),
                body: String::new(),
            });
            continue;
        }

        // A level-one heading ends the release section.
        if line.starts_with("# ") {
            if let Some(done) = current.take() {
                releases.push(done);
            }
            continue;
        }

        if let Some(entry) = current.as_mut() {
            entry.body.push_str(line);
            entry.body.push('\n');
        }
    }

    if let Some(done) = current.take() {
        releases.push(done);
    }

    for release in &mut releases {
        release.body = release.body.trim().to_string();
    }
    releases.retain(|release| !release.body.is_empty());
    releases
}

/// Releases strictly newer than `installed`, oldest first, so the reader
/// follows the upgrade path instead of jumping to the newest entry.
pub fn releases_between(releases: &[Release], installed: Option<&str>) -> Vec<Release> {
    let installed_version = installed.and_then(parse_version);

    let mut pending: Vec<(Option<semver::Version>, Release)> = releases
        .iter()
        .filter(|release| {
            let Some(version) = parse_version(&release.version) else {
                return false;
            };
            match installed_version.as_ref() {
                Some(current) => version > *current,
                None => true,
            }
        })
        .map(|release| (parse_version(&release.version), release.clone()))
        .collect();

    pending.sort_by(|a, b| match (&a.0, &b.0) {
        (Some(left), Some(right)) => left.cmp(right),
        _ => std::cmp::Ordering::Equal,
    });
    pending.into_iter().map(|(_, release)| release).collect()
}

// ---------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------

/// Compares two version strings, tolerating a leading `v` and missing parts.
pub fn is_newer(latest: &str, installed: &str) -> bool {
    match (parse_version(latest), parse_version(installed)) {
        (Some(latest), Some(installed)) => latest > installed,
        _ => false,
    }
}

pub fn parse_version(raw: &str) -> Option<semver::Version> {
    let cleaned = raw.trim().trim_start_matches('v');
    semver::Version::parse(cleaned).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENTLE_AI_UPDATE: &str = "Update Check
============

  [ok] gentle-ai     installed: 4.3.1       latest: 4.3.1     
  [ok] engram        installed: 3.6.0       latest: 3.7.0     
  [??] gga           installed: -           latest: 2.10.1    

Updates available: 1
";

    #[test]
    fn parses_the_gentle_ai_update_table() {
        let rows = parse_gentle_ai_update(GENTLE_AI_UPDATE);
        assert_eq!(rows.len(), 3, "header lines must not become rows");

        assert_eq!(rows[0].name, "gentle-ai");
        assert_eq!(rows[0].marker, "ok");
        assert_eq!(rows[0].installed.as_deref(), Some("4.3.1"));
        assert_eq!(rows[0].latest.as_deref(), Some("4.3.1"));

        assert_eq!(rows[1].name, "engram");
        assert_eq!(rows[1].latest.as_deref(), Some("3.7.0"));

        assert_eq!(rows[2].name, "gga");
        assert_eq!(rows[2].marker, "??");
        assert_eq!(rows[2].installed, None, "'-' means not installed");
        assert_eq!(rows[2].latest.as_deref(), Some("2.10.1"));
    }

    #[test]
    fn ignores_unrelated_gentle_ai_output() {
        assert!(parse_gentle_ai_update("Error: no network\n").is_empty());
        assert!(parse_gentle_ai_update("All tools are up to date!").is_empty());
    }

    #[test]
    fn normalizes_missing_installed_versions() {
        assert_eq!(normalize_installed("-"), None);
        assert_eq!(normalize_installed("  "), None);
        assert_eq!(normalize_installed("unknown"), None);
        assert_eq!(normalize_installed(" v4.3.1 "), Some("4.3.1".to_string()));
    }

    const PI_LIST: &str = "User packages:
  npm:gentle-engram
    C:\\Users\\tomas\\.pi\\agent\\npm\\node_modules\\gentle-engram
  npm:@amaster.ai/pi-attachments (filtered)
    C:\\Users\\tomas\\.pi\\agent\\npm\\node_modules\\@amaster.ai\\pi-attachments
  git:github.com/penniey/pi-paste-image
    C:\\Users\\tomas\\.pi\\agent\\git\\github.com\\penniey\\pi-paste-image
";

    #[test]
    fn parses_pi_list_sources_and_paths() {
        let entries = parse_pi_list(PI_LIST);
        assert_eq!(entries.len(), 3);

        assert_eq!(entries[0].source, "npm:gentle-engram");
        assert_eq!(
            entries[0].path,
            r"C:\Users\tomas\.pi\agent\npm\node_modules\gentle-engram"
        );

        assert_eq!(
            entries[1].source, "npm:@amaster.ai/pi-attachments",
            "the (filtered) annotation is not part of the source"
        );

        assert_eq!(entries[2].source, "git:github.com/penniey/pi-paste-image");
    }

    #[test]
    fn parses_pi_list_without_installed_paths() {
        let entries = parse_pi_list("User packages:\n  npm:pi-btw\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "");
    }

    const DOCTOR: &str = "gentle-ai doctor — system health check
=======================================

  [ok]  tool:gentle-ai                 gentle-ai found at C:\\Users\\tomas\\go\\bin\\gentle-ai.exe (version 4.3.1)
  [!!]  tool:gga                       gga found at C:\\Users\\tomas\\bin\\gga.cmd; version probe failed: exit status 1
       Remedy: Repair or reinstall gga at C:\\Users\\tomas\\bin\\gga.cmd, then run 'gentle-ai doctor' again
  [ok]  state:json                     state file OK — 1 agent(s) installed: hermes
  [!!]  installed:asset_version        installed assets were configured by gentle-ai 4.2.0
       Remedy: run gentle-ai sync

Summary: 5 passed, 0 failed, 3 warnings
Status:  degraded
";

    #[test]
    fn parses_doctor_checks_remedies_and_status() {
        let report = parse_doctor(DOCTOR);
        assert_eq!(report.checks.len(), 4);
        assert_eq!(report.checks[0].id, "tool:gentle-ai");
        assert_eq!(report.checks[0].marker, "ok");
        assert!(report.checks[0].message.contains("version 4.3.1"));

        assert_eq!(report.checks[1].marker, "!!");
        assert_eq!(
            report.checks[1].remedy.as_deref(),
            Some(r"Repair or reinstall gga at C:\Users\tomas\bin\gga.cmd, then run 'gentle-ai doctor' again"),
            "continuation lines are remedies, not checks"
        );

        assert_eq!(report.status, "degraded");
        assert_eq!(
            report.summary.as_deref(),
            Some("5 passed, 0 failed, 3 warnings")
        );
        assert!(report.raw.contains("Status:"));
    }

    #[test]
    fn doctor_status_falls_back_to_unknown() {
        let report = parse_doctor("gentle-ai doctor — system health check\n");
        assert_eq!(report.status, "unknown");
        assert_eq!(report.summary, None);
        assert!(report.checks.is_empty());
    }

    const CHANGELOG: &str = "# Changelog

## [1.4.0] - 2026-10-21

### New Features

- Image input for classifiers.

## [1.3.2] - 2026-10-18

### Fixed

- Compaction no longer fails.

## [1.2.0] - 2026-10-01

### Added

- Something older.
";

    #[test]
    fn parses_changelog_releases_newest_first() {
        let releases = parse_changelog(CHANGELOG);
        assert_eq!(
            releases.iter().map(|r| r.version.as_str()).collect::<Vec<_>>(),
            vec!["1.4.0", "1.3.2", "1.2.0"]
        );
        assert_eq!(releases[0].date.as_deref(), Some("2026-10-21"));
        assert!(releases[0].body.contains("Image input for classifiers"));
        assert!(!releases[0].body.contains("1.3.2"), "body stops at next heading");
        assert!(releases[1].body.contains("Compaction"));
    }

    #[test]
    fn selects_only_releases_above_the_installed_version() {
        let releases = parse_changelog(CHANGELOG);
        let pending = releases_between(&releases, Some("1.2.0"));
        assert_eq!(
            pending.iter().map(|r| r.version.as_str()).collect::<Vec<_>>(),
            vec!["1.3.2", "1.4.0"],
            "oldest first so the upgrade path reads naturally"
        );

        let all = releases_between(&releases, None);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn compares_versions_with_and_without_prefix() {
        assert!(is_newer("4.3.2", "4.3.1"));
        assert!(is_newer("v4.3.2", "4.3.1"));
        assert!(is_newer("5.0.0", "4.99.99"));
        assert!(!is_newer("4.3.1", "4.3.1"));
        assert!(!is_newer("4.3.1", "v4.3.2"));
        assert!(!is_newer("not-a-version", "4.3.1"), "unparseable is never newer");
    }
}
