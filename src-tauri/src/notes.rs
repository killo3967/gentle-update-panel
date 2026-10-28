//! Release notes, fetched lazily for the item the user opens and cached so a
//! six-hourly poll never burns the GitHub rate limit.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::cache::{self, Cached, Limits};
use crate::domain::{NotesPayload, Release};
use crate::parsers;
use crate::{cli, probe};

const CACHE_TTL_SECONDS: i64 = 6 * 60 * 60;
/// Bump when the shape of a cached payload changes (e.g. number of releases).
const CACHE_VERSION: &str = "v2";
const MAX_RELEASES: usize = 8;
const MAX_BODY_CHARS: usize = 20_000;
const MAX_HITS: usize = 40;
const GITHUB: &str = "https://api.github.com";

/// One entry can reach ~92 KB (eight releases of 20 000 chars), so the count and
/// byte budgets matter more than the age one.
const CACHE_LIMITS: Limits = Limits {
    max_entries: 150,
    max_bytes: 3 * 1024 * 1024,
    max_age_days: 45,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteHit {
    pub notes_key: String,
    pub version: String,
    pub title: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    fetched_at: String,
    payload: NotesPayload,
}

impl Cached for CacheEntry {
    fn fetched_at(&self) -> &str {
        &self.fetched_at
    }
}

/// `v2|gh:owner/repo|1.0.0|1.1.0`; keys under another version are never read again.
fn is_current_cache_key(key: &str) -> bool {
    key.strip_prefix(CACHE_VERSION)
        .is_some_and(|rest| rest.starts_with('|'))
}

pub struct NotesService {
    client: reqwest::Client,
    cache_path: PathBuf,
    cache: Mutex<HashMap<String, CacheEntry>>,
}

impl NotesService {
    pub fn new(cache_path: PathBuf) -> Self {
        let mut cache: HashMap<String, CacheEntry> = cache::load(&cache_path);

        // Clean once at startup: drop keys from an older cache version and put
        // the file back under its budgets, so it stops carrying dead weight.
        let before = cache.len();
        cache.retain(|key, _| is_current_cache_key(key));
        cache::prune(&mut cache, CACHE_LIMITS, Utc::now());
        if cache.len() != before {
            let _ = cache::store(&cache_path, &cache);
        }

        let client = reqwest::Client::builder()
            .user_agent("gentle-update-panel/0.1 (+windows)")
            .timeout(Duration::from_secs(20))
            .build()
            .expect("http client");

        Self {
            client,
            cache_path,
            cache: Mutex::new(cache),
        }
    }

    pub async fn get(
        &self,
        key: &str,
        installed: Option<&str>,
        latest: Option<&str>,
        force: bool,
    ) -> NotesPayload {
        let cache_key = format!(
            "{CACHE_VERSION}|{key}|{}|{}",
            installed.unwrap_or("-"),
            latest.unwrap_or("-")
        );

        if !force {
            if let Some(entry) = self.cache.lock().ok().and_then(|cache| cache.get(&cache_key).cloned())
            {
                if is_fresh(&entry.fetched_at) {
                    return entry.payload;
                }
            }
        }

        let payload = self.fetch(key, installed, latest).await;
        if payload.error.is_none() {
            if let Ok(mut cache) = self.cache.lock() {
                cache.insert(
                    cache_key,
                    CacheEntry {
                        fetched_at: payload.fetched_at.clone(),
                        payload: payload.clone(),
                    },
                );
                // Bound the live map, not just the copy on disk.
                cache::prune(&mut cache, CACHE_LIMITS, Utc::now());
                let snapshot = cache.clone();
                drop(cache);
                self.persist(&snapshot);
            }
        }
        payload
    }

    /// Case-insensitive search over every cached release body.
    /// Only covers notes that were already fetched at least once.
    pub fn search(&self, query: &str) -> Vec<NoteHit> {
        let needle = query.trim().to_lowercase();
        if needle.chars().count() < 2 {
            return Vec::new();
        }

        let Ok(cache) = self.cache.lock() else {
            return Vec::new();
        };

        let mut hits: Vec<NoteHit> = Vec::new();
        let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

        for entry in cache.values() {
            if entry.payload.error.is_some() {
                continue;
            }
            for release in &entry.payload.releases {
                let haystack = release.body.to_lowercase();
                let Some(position) = haystack.find(&needle) else {
                    continue;
                };
                let key = (entry.payload.key.clone(), release.version.clone());
                if !seen.insert(key) {
                    continue;
                }

                let start = position.saturating_sub(70);
                let snippet: String = release.body.chars().skip(start).take(220).collect();
                hits.push(NoteHit {
                    notes_key: entry.payload.key.clone(),
                    version: release.version.clone(),
                    title: entry.payload.title.clone(),
                    snippet: snippet
                        .replace(['\n', '\r'], " ")
                        .trim()
                        .to_string(),
                });
            }
        }

        hits.sort_by(|a, b| a.notes_key.cmp(&b.notes_key).then(b.version.cmp(&a.version)));
        hits.truncate(MAX_HITS);
        hits
    }

    fn persist(&self, cache: &HashMap<String, CacheEntry>) {
        let _ = cache::store(&self.cache_path, cache);
    }

    async fn fetch(&self, key: &str, installed: Option<&str>, _latest: Option<&str>) -> NotesPayload {
        let base = NotesPayload {
            key: key.to_string(),
            title: key.to_string(),
            releases: Vec::new(),
            url: None,
            fetched_at: Utc::now().to_rfc3339(),
            error: None,
        };

        if key == "pi:changelog" {
            return self.fetch_pi_changelog(base, installed);
        }

        if let Some(slug) = key.strip_prefix("gh:") {
            return self.fetch_github(base, slug, installed).await;
        }

        if let Some(package) = key.strip_prefix("npm:") {
            return self.fetch_npm(base, package).await;
        }

        NotesPayload {
            error: Some(format!("origen de notas no reconocido: {key}")),
            ..base
        }
    }

    fn fetch_pi_changelog(&self, mut payload: NotesPayload, installed: Option<&str>) -> NotesPayload {
        payload.title = "pi".to_string();
        payload.url = Some("https://www.npmjs.com/package/@earendil-works/pi-coding-agent".to_string());

        let Some(path) = find_pi_changelog() else {
            payload.error = Some("no se encontró el CHANGELOG de pi en la instalación local".to_string());
            return payload;
        };

        let Ok(markdown) = std::fs::read_to_string(&path) else {
            payload.error = Some(format!("no se pudo leer {}", path.display()));
            return payload;
        };

        let all = parsers::parse_changelog(&markdown);
        payload.releases = window(&all, installed).into_iter().map(trim_body).collect();
        payload
    }

    async fn fetch_github(
        &self,
        mut payload: NotesPayload,
        slug: &str,
        installed: Option<&str>,
    ) -> NotesPayload {
        payload.title = slug.to_string();
        payload.url = Some(format!("https://github.com/{slug}/releases"));

        let url = format!("{GITHUB}/repos/{slug}/releases?per_page=30");
        let token = std::env::var("GITHUB_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty());

        // A single retry absorbs the transient DNS/TLS blips seen on Windows.
        let mut response = None;
        let mut last_error = String::new();
        for attempt in 0..2 {
            let mut request = self
                .client
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28");
            if let Some(token) = token.as_deref() {
                request = request.bearer_auth(token);
            }

            match request.send().await {
                Ok(ok) => {
                    response = Some(ok);
                    break;
                }
                Err(error) => {
                    last_error = describe_error(&error);
                    if attempt == 0 {
                        tokio::time::sleep(Duration::from_millis(600)).await;
                    }
                }
            }
        }

        let Some(response) = response else {
            payload.error = Some(format!("GitHub no respondió ({slug}): {last_error}"));
            return payload;
        };

        if !response.status().is_success() {
            payload.error = Some(format!(
                "GitHub devolvió HTTP {} para {slug}",
                response.status().as_u16()
            ));
            return payload;
        }

        #[derive(Deserialize)]
        struct GhRelease {
            tag_name: String,
            #[serde(default)]
            body: Option<String>,
            #[serde(default)]
            published_at: Option<String>,
            #[serde(default)]
            draft: bool,
        }

        let releases: Vec<GhRelease> = match response.json().await {
            Ok(releases) => releases,
            Err(error) => {
                payload.error = Some(format!("respuesta de GitHub ilegible: {error}"));
                return payload;
            }
        };

        let mut parsed: Vec<Release> = releases
            .into_iter()
            .filter(|release| !release.draft)
            .map(|release| Release {
                version: release
                    .tag_name
                    .trim()
                    .trim_start_matches('v')
                    .to_string(),
                date: release.published_at.map(|stamp| stamp.chars().take(10).collect()),
                body: release.body.unwrap_or_default(),
            })
            .collect();

        // Newest first, whatever order the API used.
        parsed.sort_by(|a, b| {
            let left = parsers::parse_version(&a.version);
            let right = parsers::parse_version(&b.version);
            match (left, right) {
                (Some(left), Some(right)) => right.cmp(&left),
                _ => std::cmp::Ordering::Equal,
            }
        });

        payload.releases = window(&parsed, installed).into_iter().map(trim_body).collect();
        payload
    }

    async fn fetch_npm(&self, mut payload: NotesPayload, package: &str) -> NotesPayload {
        payload.title = package.to_string();
        payload.url = Some(format!("https://www.npmjs.com/package/{package}"));

        match probe::npm_latest_with(&self.client, package).await {
            Ok(Some(version)) => {
                payload.releases.push(Release {
                    version,
                    date: None,
                    body: "El paquete no publica notas de versión en un repositorio accesible; consulta la ficha de npm.".to_string(),
                });
            }
            Ok(None) => payload.error = Some("npm no devolvió versión".to_string()),
            Err(error) => payload.error = Some(error),
        }
        payload
    }
}

/// Picks the releases to show, newest first. When nothing is pending it keeps
/// the newest `MAX_RELEASES` so history stays browsable with the prev/next nav.
fn window(releases: &[Release], installed: Option<&str>) -> Vec<Release> {
    let mut pending = parsers::releases_between(releases, installed);
    if pending.is_empty() {
        pending = releases.iter().take(MAX_RELEASES).cloned().collect();
        pending.reverse();
    }
    pending.reverse();
    pending.truncate(MAX_RELEASES);
    pending
}

fn trim_body(release: Release) -> Release {
    let body: String = release.body.chars().take(MAX_BODY_CHARS).collect();
    Release { body, ..release }
}

/// reqwest's `Display` hides the useful part; walk the cause chain.
fn describe_error(error: &reqwest::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    parts.join(" · ")
}

fn is_fresh(fetched_at: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(fetched_at)
        .map(|stamp| {
            let age = Utc::now().signed_duration_since(stamp.with_timezone(&Utc));
            age.num_seconds() < CACHE_TTL_SECONDS
        })
        .unwrap_or(false)
}

/// Locates the changelog that ships with the installed pi build.
pub fn find_pi_changelog() -> Option<PathBuf> {
    let home = cli::home_dir()?;

    let releases_dir = home.join(".pi").join("agent").join("install").join("releases");
    let mut best: Option<(semver::Version, PathBuf)> = None;
    if let Ok(entries) = std::fs::read_dir(&releases_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(version) = semver::Version::parse(name.trim_start_matches('v')) else {
                continue;
            };
            let changelog = entry
                .path()
                .join("node_modules")
                .join("@earendil-works")
                .join("pi-coding-agent")
                .join("CHANGELOG.md");
            if changelog.is_file() && best.as_ref().map(|(current, _)| version > *current).unwrap_or(true)
            {
                best = Some((version, changelog));
            }
        }
    }

    if let Some((_, path)) = best {
        return Some(path);
    }

    let npm_copy = home
        .join(".pi")
        .join("agent")
        .join("npm")
        .join("node_modules")
        .join("@earendil-works")
        .join("pi-coding-agent")
        .join("CHANGELOG.md");
    npm_copy.is_file().then_some(npm_copy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_under_an_older_cache_version_are_stale() {
        assert!(is_current_cache_key("v2|gh:o/r|1.0.0|1.1.0"));
        assert!(
            !is_current_cache_key("gh:o/r|1.0.0|1.1.0"),
            "pre-v2 entries can never be read again"
        );
        assert!(!is_current_cache_key("v1|gh:o/r|1.0.0|-"));
        assert!(
            !is_current_cache_key("v20|gh:o/r|1.0.0|-"),
            "the separator makes the prefix unambiguous"
        );
    }
}
