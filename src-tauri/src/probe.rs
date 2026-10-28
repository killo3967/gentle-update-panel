//! Collects the update picture: ecosystem tools, pi itself, and every pi package.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::Deserialize;
use tokio::sync::Semaphore;

use crate::cli;
use crate::domain::{ItemKind, Snapshot, Status, UpdateItem, Upgrade};
use crate::parsers;

const GENTLE_TIMEOUT: Duration = Duration::from_secs(120);
const PI_TIMEOUT: Duration = Duration::from_secs(90);
const REGISTRY: &str = "https://registry.npmjs.org";
const MAX_PARALLEL_LOOKUPS: usize = 6;

pub struct Prober {
    client: reqwest::Client,
}

impl Prober {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("gentle-update-panel/0.1 (+windows)")
            .timeout(Duration::from_secs(15))
            .build()
            .expect("http client");
        Self { client }
    }

    pub async fn collect(
        &self,
        ignored: &[String],
        progress: impl Fn(&str, &str) + Send + Sync,
    ) -> Snapshot {
        let started = Instant::now();
        let mut items: Vec<UpdateItem> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let ignored: std::collections::HashSet<String> = ignored.iter().cloned().collect();
        let skip = |id: &str| ignored.contains(id);

        progress("ecosystem", "Consultando gentle-ai update");
        match gentle_ai_tools().await {
            Ok(rows) => items.extend(
                rows.into_iter().map(|row| {
                    let row_ignored = skip(&row.name);
                    ecosystem_item(row, row_ignored)
                }),
            ),
            Err(error) => warnings.push(error),
        }

        progress("pi", "Leyendo la versión de pi");
        let pi_installed = pi_version(&mut warnings).await;

        progress("packages", "Listando paquetes de pi");
        let entries = pi_packages(&mut warnings).await;

        let mut latest_by_package: HashMap<String, Option<String>> = HashMap::new();
        let semaphore = std::sync::Arc::new(Semaphore::new(MAX_PARALLEL_LOOKUPS));
        let mut lookups = Vec::new();

        let pi_ignored = skip("pi");
        let pi_latest = if pi_ignored {
            None
        } else {
            self.npm_latest("@earendil-works/pi-coding-agent").await
        };
        if pi_latest.is_none() && !pi_ignored {
            warnings.push("no se pudo consultar el registro npm para pi".to_string());
        }

        let mut packages_meta = Vec::new();
        for entry in &entries {
            let meta = read_meta(&entry.path, &entry.source);
            let item_id = format!("pkg:{}", entry.source);
            if let (Some(npm_name), false) = (npm_name(&entry.source), skip(&item_id)) {
                let client = self.client.clone();
                let semaphore = semaphore.clone();
                let npm_name_owned = npm_name.to_string();
                lookups.push(tokio::spawn(async move {
                    let _permit = semaphore.acquire_owned().await.ok();
                    let result = npm_latest_with(&client, &npm_name_owned).await;
                    (npm_name_owned, result)
                }));
            }
            packages_meta.push((entry.clone(), meta));
        }

        for lookup in lookups {
            if let Ok((name, result)) = lookup.await {
                match result {
                    Ok(version) => {
                        latest_by_package.insert(name, version);
                    }
                    Err(_) => {
                        latest_by_package.insert(name, None);
                    }
                }
            }
        }

        let unresolved: Vec<String> = latest_by_package
            .iter()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.clone())
            .collect();
        if !unresolved.is_empty() {
            let sample = unresolved
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            warnings.push(format!(
                "sin datos de npm para {} paquete(s): {sample}{}",
                unresolved.len(),
                if unresolved.len() > 3 { "…" } else { "" }
            ));
        }

        items.push(pi_item(pi_installed, pi_latest, pi_ignored));

        for (entry, meta) in packages_meta {
            let latest = npm_name(&entry.source).and_then(|name| {
                latest_by_package
                    .get(name)
                    .cloned()
                    .unwrap_or_default()
            });
            let item_ignored = skip(&format!("pkg:{}", entry.source));
            items.push(package_item(&entry.source, meta, latest, item_ignored));
        }

        progress("done", "Comprobación terminada");
        Snapshot {
            checked_at: Utc::now().to_rfc3339(),
            duration_ms: started.elapsed().as_millis() as u64,
            items,
            warnings,
        }
    }

    async fn npm_latest(&self, package: &str) -> Option<String> {
        npm_latest_with(&self.client, package).await.ok().flatten()
    }
}

impl Default for Prober {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Ecosystem
// ---------------------------------------------------------------------------

async fn gentle_ai_tools() -> Result<Vec<parsers::ToolRow>, String> {
    let args = vec!["update".to_string()];
    let output = cli::run("gentle-ai", &args, GENTLE_TIMEOUT).await?;
    if output.timed_out {
        return Err("gentle-ai update no respondió a tiempo".to_string());
    }
    let rows = parsers::parse_gentle_ai_update(&output.stdout);
    if rows.is_empty() {
        let detail = output.combined();
        let detail: String = detail.chars().take(400).collect();
        return Err(format!(
            "gentle-ai update no devolvió ninguna herramienta{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ));
    }
    Ok(rows)
}

fn ecosystem_item(row: parsers::ToolRow, ignored: bool) -> UpdateItem {
    let status = status_for(row.installed.as_deref(), row.latest.as_deref());
    let detail = match (status, row.installed.as_ref(), row.latest.as_ref()) {
        (Status::Missing, _, Some(latest)) => {
            Some(format!("no está instalado; gentle-ai puede instalarlo en {latest}"))
        }
        (Status::UpdateAvailable, Some(installed), Some(latest)) => {
            Some(format!("{installed} → {latest}"))
        }
        _ => None,
    };

    UpdateItem {
        id: row.name.clone(),
        kind: ItemKind::Ecosystem,
        name: row.name.clone(),
        source: None,
        installed: row.installed,
        latest: row.latest,
        status,
        detail,
        upgrade: Upgrade::GentleAiTool { tool: row.name },
        notes_key: Some("gh:Gentleman-Programming/gentle-ai".to_string()),
        ignored,
    }
}

// ---------------------------------------------------------------------------
// pi and its packages
// ---------------------------------------------------------------------------

async fn pi_version(warnings: &mut Vec<String>) -> Option<String> {
    let args = vec!["--version".to_string()];
    match cli::run("pi", &args, PI_TIMEOUT).await {
        Ok(output) if !output.timed_out => {
            let version = output.stdout.trim().to_string();
            if version.is_empty() {
                warnings.push("`pi --version` no devolvió ninguna versión".to_string());
                None
            } else {
                Some(version.trim_start_matches('v').to_string())
            }
        }
        Ok(_) => {
            warnings.push("`pi --version` no respondió a tiempo".to_string());
            None
        }
        Err(error) => {
            warnings.push(error);
            None
        }
    }
}

async fn pi_packages(warnings: &mut Vec<String>) -> Vec<parsers::PackageEntry> {
    let args = vec!["list".to_string()];
    match cli::run("pi", &args, PI_TIMEOUT).await {
        Ok(output) if !output.timed_out => {
            let entries = parsers::parse_pi_list(&output.stdout);
            if entries.is_empty() {
                warnings.push("`pi list` no devolvió paquetes instalados".to_string());
            }
            entries
        }
        Ok(_) => {
            warnings.push("`pi list` no respondió a tiempo".to_string());
            Vec::new()
        }
        Err(error) => {
            warnings.push(error);
            Vec::new()
        }
    }
}

fn pi_item(installed: Option<String>, latest: Option<String>, ignored: bool) -> UpdateItem {
    let status = status_for(installed.as_deref(), latest.as_deref());
    UpdateItem {
        id: "pi".to_string(),
        kind: ItemKind::Pi,
        name: "pi".to_string(),
        source: Some("npm:@earendil-works/pi-coding-agent".to_string()),
        detail: if ignored {
            None
        } else {
            versions_detail(status, installed.as_deref(), latest.as_deref())
        },
        installed,
        latest,
        status,
        upgrade: Upgrade::PiSelf,
        notes_key: Some("pi:changelog".to_string()),
        ignored,
    }
}

#[derive(Debug, Clone, Default)]
pub struct PackageMeta {
    pub version: Option<String>,
    pub repository: Option<String>,
}

fn package_item(
    source: &str,
    meta: PackageMeta,
    latest: Option<String>,
    ignored: bool,
) -> UpdateItem {
    let name = display_name(source);
    let is_git = source.starts_with("git:");
    let status = if is_git {
        match meta.version.clone() {
            Some(_) => Status::Unknown,
            None => Status::Unknown,
        }
    } else {
        status_for(meta.version.as_deref(), latest.as_deref())
    };

    let detail = if ignored {
        None
    } else if is_git {
        Some("paquete git: pi lo actualiza desde el remoto, sin comparar versiones".to_string())
    } else {
        versions_detail(status, meta.version.as_deref(), latest.as_deref())
    };

    UpdateItem {
        id: format!("pkg:{source}"),
        kind: ItemKind::PiPackage,
        name,
        source: Some(source.to_string()),
        installed: meta.version,
        latest: if is_git { None } else { latest },
        status,
        detail,
        upgrade: Upgrade::PiPackage {
            source: source.to_string(),
        },
        notes_key: notes_key_for(source, meta.repository.as_deref()),
        ignored,
    }
}

/// Where the release notes come from. Never guess: a wrong repository shows
/// someone else's notes, which is worse than showing none.
fn notes_key_for(source: &str, repository: Option<&str>) -> Option<String> {
    // `pi` pulls a git package from its own remote, so that repo owns the notes.
    if source.starts_with("git:") {
        if let Some(slug) = github_slug(source) {
            return Some(format!("gh:{slug}"));
        }
    }

    if let Some(slug) = repository.and_then(github_slug) {
        return Some(format!("gh:{slug}"));
    }

    npm_name(source).map(|npm| format!("npm:{npm}"))
}

/// `npm:@scope/pkg` → `@scope/pkg`, `git:github.com/o/r` → `o/r`.
pub fn display_name(source: &str) -> String {
    source
        .strip_prefix("npm:")
        .or_else(|| source.strip_prefix("git:"))
        .unwrap_or(source)
        .to_string()
}

pub fn npm_name(source: &str) -> Option<&str> {
    source.strip_prefix("npm:")
}

fn versions_detail(
    status: Status,
    installed: Option<&str>,
    latest: Option<&str>,
) -> Option<String> {
    match (status, installed, latest) {
        (Status::UpdateAvailable, Some(installed), Some(latest)) => {
            Some(format!("{installed} → {latest}"))
        }
        (Status::Missing, _, Some(latest)) => Some(format!("sin instalar; última {latest}")),
        (Status::Unknown, _, None) => Some("versión publicada desconocida".to_string()),
        _ => None,
    }
}

fn status_for(installed: Option<&str>, latest: Option<&str>) -> Status {
    match (installed, latest) {
        (Some(installed), Some(latest)) => {
            if installed == latest {
                Status::UpToDate
            } else if parsers::is_newer(latest, installed) {
                Status::UpdateAvailable
            } else if parsers::parse_version(latest).is_some() {
                Status::UpToDate
            } else {
                Status::Unknown
            }
        }
        (None, Some(_)) => Status::Missing,
        _ => Status::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Local package metadata
// ---------------------------------------------------------------------------

fn read_meta(path: &str, source: &str) -> PackageMeta {
    let candidate = if path.is_empty() {
        fallback_package_dir(source)
    } else {
        Some(PathBuf::from(path))
    };

    let Some(directory) = candidate else {
        return PackageMeta::default();
    };

    read_package_json(&directory.join("package.json"))
}

fn fallback_package_dir(source: &str) -> Option<PathBuf> {
    let npm = npm_name(source)?;
    let home = cli::home_dir()?;
    let directory = home
        .join(".pi")
        .join("agent")
        .join("npm")
        .join("node_modules")
        .join(npm.replace('/', std::path::MAIN_SEPARATOR_STR));
    directory.is_dir().then_some(directory)
}

fn read_package_json(path: &Path) -> PackageMeta {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return PackageMeta::default();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return PackageMeta::default();
    };

    let version = json
        .get("version")
        .and_then(|value| value.as_str())
        .map(|value| value.trim_start_matches('v').to_string());

    let repository = match json.get("repository") {
        Some(serde_json::Value::String(url)) => Some(url.clone()),
        Some(serde_json::Value::Object(map)) => map
            .get("url")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        _ => None,
    };

    PackageMeta {
        version,
        repository,
    }
}

// ---------------------------------------------------------------------------
// npm registry
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LatestPayload {
    version: Option<String>,
}

pub async fn npm_latest_with(
    client: &reqwest::Client,
    package: &str,
) -> Result<Option<String>, String> {
    // The registry occasionally drops a request (seen as 3 failures in one pass);
    // one retry keeps the panel from showing false "sin datos" rows.
    let mut last_error = String::new();
    for attempt in 0..2 {
        match npm_latest_once(client, package).await {
            Ok(version) => return Ok(version),
            Err(error) => {
                last_error = error;
                if attempt == 0 {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }
    Err(last_error)
}

async fn npm_latest_once(
    client: &reqwest::Client,
    package: &str,
) -> Result<Option<String>, String> {
    let encoded = package.replace('/', "%2F");
    let url = format!("{REGISTRY}/{encoded}/latest");
    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|error| format!("npm {package}: {error}"))?;

    if !response.status().is_success() {
        return Err(format!("npm {package}: HTTP {}", response.status()));
    }

    let payload: LatestPayload = response
        .json()
        .await
        .map_err(|error| format!("npm {package}: respuesta ilegible ({error})"))?;

    Ok(payload
        .version
        .map(|version| version.trim_start_matches('v').to_string()))
}

// ---------------------------------------------------------------------------
// Repository URLs
// ---------------------------------------------------------------------------

/// Extracts `owner/repo` from the forms npm packages use.
pub fn github_slug(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let normalized = trimmed
        .strip_prefix("git+")
        .unwrap_or(trimmed)
        .trim_end_matches(".git");

    if let Some(rest) = normalized.strip_prefix("github:") {
        return Some(rest.trim_matches('/').to_string());
    }

    for marker in ["github.com/", "github.com:"] {
        if let Some(index) = normalized.find(marker) {
            let rest = &normalized[index + marker.len()..];
            let slug: Vec<&str> = rest.split('/').filter(|part| !part.is_empty()).collect();
            if slug.len() >= 2 {
                return Some(format!("{}/{}", slug[0], slug[1]));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_scoped_and_plain_sources() {
        assert_eq!(display_name("npm:@amaster.ai/pi-attachments"), "@amaster.ai/pi-attachments");
        assert_eq!(display_name("npm:gentle-pi"), "gentle-pi");
        assert_eq!(display_name("git:github.com/penniey/pi-paste-image"), "github.com/penniey/pi-paste-image");
        assert_eq!(npm_name("npm:gentle-pi"), Some("gentle-pi"));
        assert_eq!(npm_name("git:github.com/penniey/pi-paste-image"), None);
    }

    #[test]
    fn extracts_github_slugs_from_repository_urls() {
        assert_eq!(
            github_slug("git+https://github.com/Gentleman-Programming/gentle-shell.git"),
            Some("Gentleman-Programming/gentle-shell".to_string())
        );
        assert_eq!(
            github_slug("https://github.com/penniey/pi-paste-image"),
            Some("penniey/pi-paste-image".to_string())
        );
        assert_eq!(
            github_slug("git@github.com:penniey/pi-paste-image.git"),
            Some("penniey/pi-paste-image".to_string())
        );
        assert_eq!(github_slug("github:owner/repo"), Some("owner/repo".to_string()));
        assert_eq!(github_slug("https://gitlab.com/owner/repo"), None);
    }

    #[test]
    fn classifies_versions_into_rows() {
        assert_eq!(status_for(Some("4.3.1"), Some("4.3.1")), Status::UpToDate);
        assert_eq!(status_for(Some("4.3.0"), Some("4.3.1")), Status::UpdateAvailable);
        assert_eq!(status_for(Some("4.3.2"), Some("4.3.1")), Status::UpToDate);
        assert_eq!(status_for(None, Some("2.10.1")), Status::Missing);
        assert_eq!(status_for(Some("1.0.0"), None), Status::Unknown);
        assert_eq!(status_for(None, None), Status::Unknown);
    }

    #[test]
    fn git_packages_take_notes_from_their_own_remote() {
        assert_eq!(
            notes_key_for("git:github.com/penniey/pi-paste-image", None),
            Some("gh:penniey/pi-paste-image".to_string()),
            "a git source names its own repo; it must never borrow another one's notes"
        );
    }

    #[test]
    fn npm_packages_prefer_the_declared_repository() {
        assert_eq!(
            notes_key_for("npm:gentle-pi", Some("git+https://github.com/Gentleman-Programming/gentle-pi.git")),
            Some("gh:Gentleman-Programming/gentle-pi".to_string())
        );
        assert_eq!(
            notes_key_for("npm:@scope/pkg", None),
            Some("npm:@scope/pkg".to_string()),
            "without metadata the registry page is the honest source"
        );
    }

    #[test]
    fn a_git_package_without_a_known_host_gets_no_notes() {
        assert_eq!(
            notes_key_for("git:gitlab.com/owner/repo", None),
            None,
            "guessing would show somebody else's releases"
        );
    }

    #[test]
    fn reads_installed_version_and_repository_from_package_json() {
        let directory = std::env::temp_dir().join(format!("gup-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("package.json");
        std::fs::write(
            &file,
            r#"{"version":"2.15.0","repository":{"type":"git","url":"git+https://github.com/o/r.git"}}"#,
        )
        .unwrap();

        let meta = read_package_json(&file);
        assert_eq!(meta.version.as_deref(), Some("2.15.0"));
        assert_eq!(meta.repository.as_deref(), Some("git+https://github.com/o/r.git"));

        std::fs::remove_dir_all(&directory).ok();
    }
}
