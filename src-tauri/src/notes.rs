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
/// Bump when the shape of a cached payload changes (e.g. number of releases) or
/// when a key would resolve to different releases than before.
const CACHE_VERSION: &str = "v4";
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

/// Some packages are published from a monorepo, so npm only names the umbrella
/// repository and the button would open a releases page that usually holds
/// nothing. Entries are explicit on purpose — guessing a package's own page from
/// its name would eventually send the user to a stranger's repository — and are
/// keyed by the package, because every sibling in that monorepo shares one slug.
const PAGE_OVERRIDES: &[(&str, &str)] = &[(
    "@juicesharp/rpiv-btw",
    "https://github.com/juicesharp/rpiv-btw/releases",
)];

/// The page a package offers for itself, or nothing.
fn page_override(package: Option<&str>) -> Option<&'static str> {
    let package = package?;
    PAGE_OVERRIDES
        .iter()
        .find(|(name, _)| *name == package)
        .map(|(_, url)| *url)
}

/// The override never enters the cache: an entry keeps the URL its key implies,
/// so each caller gets the page that belongs to the package it is showing.
fn with_page_override(mut payload: NotesPayload, package: Option<&str>) -> NotesPayload {
    if let Some(url) = page_override(package) {
        payload.url = Some(url.to_string());
    }
    payload
}

#[cfg(test)]
mod page_override_tests {
    use super::*;

    fn monorepo_payload() -> NotesPayload {
        NotesPayload {
            key: "gh:juicesharp/rpiv-mono".to_string(),
            title: "juicesharp/rpiv-mono".to_string(),
            releases: Vec::new(),
            url: Some("https://github.com/juicesharp/rpiv-mono/releases".to_string()),
            fetched_at: "2026-01-01T00:00:00Z".to_string(),
            error: None,
        }
    }

    #[test]
    fn a_monorepo_package_opens_the_page_it_names() {
        let overridden = with_page_override(monorepo_payload(), Some("@juicesharp/rpiv-btw"));
        assert_eq!(
            overridden.url.as_deref(),
            Some("https://github.com/juicesharp/rpiv-btw/releases")
        );
    }

    #[test]
    fn a_sibling_or_an_anonymous_caller_keeps_the_declared_repository() {
        let declared = monorepo_payload();
        let sibling = with_page_override(declared.clone(), Some("@juicesharp/rpiv-other"));
        let anonymous = with_page_override(declared.clone(), None);
        assert_eq!(sibling.url, declared.url);
        assert_eq!(anonymous.url, declared.url);
    }
}

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

/// `v3|gh:owner/repo[#family]|1.0.0|1.1.0`; keys under another version are never
/// read again.
fn is_current_cache_key(key: &str) -> bool {
    key.strip_prefix(CACHE_VERSION)
        .is_some_and(|rest| rest.starts_with('|'))
}

/// `owner/repo#pi-v` → (`owner/repo`, `Some("pi-v")`). A repository can publish
/// several products in one release feed, and then the bare slug would hand out
/// another product's notes.
fn split_tag_family(slug: &str) -> (&str, Option<&str>) {
    let Some((slug, family)) = slug.split_once('#') else {
        return (slug, None);
    };
    match family.trim() {
        "" => (slug, None),
        family => (slug, Some(family)),
    }
}

/// The version a tag publishes for this family, or nothing when the tag belongs
/// to another product. A family keeps only its own tags, and one `v` after the
/// family is optional (`pi-v0.6.0`).
fn release_version_from_tag(tag: &str, family: Option<&str>) -> Option<String> {
    let tag = tag.trim();
    let rest = match family {
        Some(family) => tag.strip_prefix(family)?,
        None => tag,
    };
    let version = rest.trim().trim_start_matches('v');
    (!version.is_empty()).then(|| version.to_string())
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
        package: Option<&str>,
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
                    return with_page_override(entry.payload, package);
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
        with_page_override(payload, package)
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

    async fn fetch(&self, key: &str, installed: Option<&str>, latest: Option<&str>) -> NotesPayload {
        let base = NotesPayload {
            key: key.to_string(),
            title: key.to_string(),
            releases: Vec::new(),
            url: None,
            fetched_at: Utc::now().to_rfc3339(),
            error: None,
        };

        if key == "pi:changelog" {
            return self.fetch_pi_changelog(base, installed, latest).await;
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

    async fn fetch_pi_changelog(
        &self,
        mut payload: NotesPayload,
        installed: Option<&str>,
        latest: Option<&str>,
    ) -> NotesPayload {
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

        let mut all = parsers::parse_changelog(&markdown);

        // The changelog ships inside the installed build, so it cannot know
        // about the version that is still only upstream. Without this the panel
        // offers 1.5.1 and then shows the notes of 1.5.0.
        if latest.is_some_and(|latest| changelog_is_behind(&all, latest)) {
            if let Some(latest) = latest {
                if let Some(published) = self.fetch_published_changelog(latest).await {
                    all = merge_releases(published, all);
                }
            }
        }

        payload.releases = window(&all, installed).into_iter().map(trim_body).collect();
        payload
    }

    /// The changelog published with the pending version, straight from npm.
    async fn fetch_published_changelog(&self, version: &str) -> Option<Vec<Release>> {
        let url = format!(
            "https://unpkg.com/@earendil-works/pi-coding-agent@{version}/CHANGELOG.md"
        );
        let response = self.client.get(&url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let markdown = response.text().await.ok()?;
        let releases = parsers::parse_changelog(&markdown);
        (!releases.is_empty()).then_some(releases)
    }

    async fn fetch_github(
        &self,
        mut payload: NotesPayload,
        slug: &str,
        installed: Option<&str>,
    ) -> NotesPayload {
        let (slug, family) = split_tag_family(slug);
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
            .filter_map(|release| {
                Some(Release {
                    version: release_version_from_tag(&release.tag_name, family)?,
                    date: release.published_at.map(|stamp| stamp.chars().take(10).collect()),
                    body: release.body.unwrap_or_default(),
                })
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

/// The local copy stops at the installed build, so it trails a version that is
/// only published upstream.
fn changelog_is_behind(releases: &[Release], latest: &str) -> bool {
    let Some(latest) = parsers::parse_version(latest) else {
        return false;
    };
    releases
        .iter()
        .filter_map(|release| parsers::parse_version(&release.version))
        .max()
        .map_or(true, |newest| newest < latest)
}

/// The published changelog is the superset: its bodies win and any release the
/// local copy still has is kept.
fn merge_releases(published: Vec<Release>, local: Vec<Release>) -> Vec<Release> {
    let mut merged = published;
    let known: std::collections::HashSet<String> =
        merged.iter().map(|release| release.version.clone()).collect();
    merged.extend(
        local
            .into_iter()
            .filter(|release| !known.contains(&release.version)),
    );
    merged
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
        assert!(is_current_cache_key("v4|gh:o/r|1.0.0|1.1.0"));
        assert!(
            !is_current_cache_key("v3|gh:o/r|1.0.0|1.1.0"),
            "notes cached before the published changelog was read can never be served again"
        );
        assert!(!is_current_cache_key("v2|gh:o/r|1.0.0|-"));
        assert!(
            !is_current_cache_key("v30|gh:o/r|1.0.0|-"),
            "the separator makes the prefix unambiguous"
        );
    }

    #[test]
    fn a_notes_key_can_name_the_family_it_wants() {
        assert_eq!(
            split_tag_family("Gentleman-Programming/engram#pi-v"),
            ("Gentleman-Programming/engram", Some("pi-v"))
        );
        assert_eq!(
            split_tag_family("Gentleman-Programming/gentle-pi"),
            ("Gentleman-Programming/gentle-pi", None)
        );
        assert_eq!(
            split_tag_family("owner/repo#"),
            ("owner/repo", None),
            "an empty family must not filter every release away"
        );
    }

    #[test]
    fn a_family_keeps_only_its_own_tags() {
        // The real feed of Gentleman-Programming/engram mixes the Go core's `v3.x`
        // tags with the Pi plugin's `pi-v0.x` ones.
        let tags = [
            "v3.6.2",
            "pi-v0.6.0",
            "v3.6.1",
            "pi-v0.5.0",
            "v3.4.0",
            "pi-v0.4.1",
            "pi-v0.4.0",
        ];

        let plugin: Vec<String> = tags
            .iter()
            .filter_map(|tag| release_version_from_tag(tag, Some("pi-v")))
            .collect();
        assert_eq!(
            plugin,
            vec!["0.6.0", "0.5.0", "0.4.1", "0.4.0"],
            "the core's tags must never reach the plugin's notes"
        );

        let core: Vec<String> = tags
            .iter()
            .filter_map(|tag| release_version_from_tag(tag, None))
            .collect();
        assert_eq!(
            core,
            vec![
                "3.6.2",
                "pi-v0.6.0",
                "3.6.1",
                "pi-v0.5.0",
                "3.4.0",
                "pi-v0.4.1",
                "pi-v0.4.0"
            ],
            "without a family nothing is filtered: only the leading `v` is dropped"
        );

        // The same repo, the other product: `#v` keeps the core's own tags and
        // drops every `pi-v` one, because the family is stripped first.
        let core_family: Vec<String> = tags
            .iter()
            .filter_map(|tag| release_version_from_tag(tag, Some("v")))
            .collect();
        assert_eq!(core_family, vec!["3.6.2", "3.6.1", "3.4.0"]);
    }

    #[test]
    fn a_scoped_family_keeps_the_server_and_drops_the_bundle_stub() {
        // The real feed of upstash/context7, newest first: the mcpb bundle, the
        // CLI, the MCP server and the SDK share one release list.
        let tags = [
            "mcpb-v4.5.0",
            "ctx7@0.9.0",
            "@upstash/context7-mcp@4.5.0",
            "ctx7@0.8.2",
            "@upstash/context7-sdk@0.6.0",
            "mcpb-v4.4.4",
        ];

        let server: Vec<String> = tags
            .iter()
            .filter_map(|tag| release_version_from_tag(tag, Some("@upstash/context7-mcp@")))
            .collect();
        assert_eq!(
            server,
            vec!["4.5.0"],
            "the bundle stub and the CLI must never reach the server's notes"
        );
    }

    #[test]
    fn a_family_tag_without_a_version_is_dropped() {
        assert_eq!(release_version_from_tag("pi-v", Some("pi-v")), None);
        assert_eq!(release_version_from_tag("   ", None), None);
    }

    #[test]
    fn the_plugin_row_opens_the_plugin_notes_and_not_the_core_ones() {
        let tags = [
            "v3.6.2",
            "pi-v0.6.0",
            "v3.6.1",
            "v3.6.0",
            "v3.5.4",
            "pi-v0.5.0",
            "v3.4.0",
            "pi-v0.4.1",
        ];
        let parsed: Vec<Release> = tags
            .iter()
            .filter_map(|tag| {
                Some(Release {
                    version: release_version_from_tag(tag, Some("pi-v"))?,
                    date: None,
                    body: format!("notes for {tag}"),
                })
            })
            .collect();

        let shown = window(&parsed, Some("0.4.2"));
        let shown: Vec<&str> = shown
            .iter()
            .map(|release| release.version.as_str())
            .collect();
        assert_eq!(
            shown,
            vec!["0.6.0", "0.5.0"],
            "an installed 0.4.2 has exactly these plugin upgrades pending"
        );
    }

    fn release(version: &str, body: &str) -> Release {
        Release {
            version: version.to_string(),
            date: None,
            body: body.to_string(),
        }
    }

    #[test]
    fn a_local_changelog_behind_the_pending_version_asks_for_the_published_one() {
        let local = vec![release("1.5.0", "local"), release("1.4.0", "older")];

        assert!(
            changelog_is_behind(&local, "1.5.1"),
            "the installed build cannot ship the notes of a version it does not have"
        );
        assert!(!changelog_is_behind(&local, "v1.5.0"));
        assert!(!changelog_is_behind(&local, "1.4.9"));
        assert!(!changelog_is_behind(&local, "sin versión"));
    }

    #[test]
    fn the_published_changelog_wins_and_the_local_releases_stay() {
        let published = vec![release("1.5.1", "published"), release("1.5.0", "published 1.5.0")];
        let local = vec![release("1.5.0", "local 1.5.0"), release("1.4.0", "local 1.4.0")];

        let merged = merge_releases(published, local);
        let bodies: Vec<(&str, &str)> = merged
            .iter()
            .map(|release| (release.version.as_str(), release.body.as_str()))
            .collect();

        assert_eq!(
            bodies,
            vec![
                ("1.5.1", "published"),
                ("1.5.0", "published 1.5.0"),
                ("1.4.0", "local 1.4.0"),
            ]
        );
    }
}
