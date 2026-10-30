//! Durable journal of what the panel detected, applied and verified.
//!
//! It is an append-only NDJSON file plus a bounded in-memory ring: appending one
//! line costs one line, and the file is only rewritten when it drifts past the
//! capacity, so a chatty update cannot turn 200 stdout lines into 200 rewrites of
//! the whole history. The webview reads the list once on boot and then appends
//! the `journal:entry` events, so a check that runs while the panel is closed is
//! not lost.

use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;
use crate::state::read_json;

/// Newest entries kept on disk. This is a log, not an archive.
pub const CAPACITY: usize = 800;

/// Rewrite the file once it holds this many lines: appends stay cheap and the
/// file never drifts far from [`CAPACITY`].
const COMPACT_AT: usize = CAPACITY + 200;

/// Longest text stored per field: one runaway stdout line must not own the file.
const MAX_TEXT: usize = 600;

/// Streamed output lines journaled per step. The activity drawer keeps the rest.
pub const LINES_PER_STEP: usize = 200;

/// Event emitted for every new entry.
pub const EVENT: &str = "journal:entry";

/// File name inside the app data directory.
pub const FILE_NAME: &str = "journal.ndjson";

/// Name used before the switch to one JSON document; imported once and removed.
const LEGACY_FILE_NAME: &str = "journal.json";

/// Entry kinds, shared with the webview.
pub mod kind {
    pub const APP: &str = "app";
    pub const CHECK: &str = "check";
    pub const DETECTED: &str = "detected";
    pub const PREFLIGHT: &str = "preflight";
    pub const APPLY: &str = "apply";
    pub const STEP: &str = "step";
    pub const OUTPUT: &str = "output";
    pub const APPLIED: &str = "applied";
    pub const FAILED: &str = "failed";
    pub const DOCTOR: &str = "doctor";
    /// The panel's own release: found, then updated.
    pub const PANEL: &str = "panel";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    /// Monotonic id; the webview dedupes by it.
    pub seq: u64,
    pub at: String,
    pub kind: String,
    pub title: String,
    pub detail: Option<String>,
    pub item_id: Option<String>,
    /// `stdout` for streamed lines, `command` for the invoked command.
    pub stream: Option<String>,
    /// Translation key for the title; `title` stays as the frozen fallback.
    #[serde(default)]
    pub title_key: Option<String>,
    /// Values for the `{name}` placeholders in `title_key`.
    #[serde(default)]
    pub title_args: Option<BTreeMap<String, String>>,
    /// Same, for the detail line: only the ones the panel composes itself.
    #[serde(default)]
    pub detail_key: Option<String>,
    #[serde(default)]
    pub detail_args: Option<BTreeMap<String, String>>,
}

/// Shape of the old `journal.json`, kept only to import it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LegacyFile {
    seq: u64,
    entries: Vec<JournalEntry>,
}

/// What a caller knows before the journal stamps the time and the sequence.
pub struct Draft {
    kind: String,
    title: String,
    key: Option<String>,
    args: BTreeMap<String, String>,
    detail_key: Option<String>,
    detail_args: BTreeMap<String, String>,
    detail: Option<String>,
    item_id: Option<String>,
    stream: Option<String>,
}

impl Draft {
    pub fn new(kind: &'static str, title: impl Into<String>) -> Self {
        Self {
            kind: kind.to_string(),
            title: title.into(),
            key: None,
            args: BTreeMap::new(),
            detail_key: None,
            detail_args: BTreeMap::new(),
            detail: None,
            item_id: None,
            stream: None,
        }
    }

    /// Marks the title as translatable in the webview.
    pub fn key(mut self, key: &str) -> Self {
        self.key = Some(key.to_string());
        self
    }

    /// A value for a `{name}` placeholder of the key.
    pub fn arg(mut self, name: &str, value: impl Into<String>) -> Self {
        self.args.insert(name.to_string(), value.into());
        self
    }

    /// Marks the detail line as translatable in the webview.
    pub fn detail_key(mut self, key: &str) -> Self {
        self.detail_key = Some(key.to_string());
        self
    }

    /// A value for a `{name}` placeholder of the detail key.
    pub fn detail_arg(mut self, name: &str, value: impl Into<String>) -> Self {
        self.detail_args.insert(name.to_string(), value.into());
        self
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        if !detail.trim().is_empty() {
            self.detail = Some(truncate(&detail));
        }
        self
    }

    pub fn item(mut self, id: &str) -> Self {
        self.item_id = Some(id.to_string());
        self
    }

    pub fn stream(mut self, stream: &'static str) -> Self {
        self.stream = Some(stream.to_string());
        self
    }
}

pub struct Journal {
    path: PathBuf,
    seq: u64,
    entries: VecDeque<JournalEntry>,
    /// Lines currently in the file, used to decide when to compact.
    lines: usize,
}

impl Journal {
    pub fn new(path: PathBuf) -> Self {
        let mut journal = Self {
            path,
            seq: 0,
            entries: VecDeque::new(),
            lines: 0,
        };

        if journal.path.exists() {
            journal.read_ndjson();
        } else {
            journal.import_legacy();
        }

        let loaded = journal.lines;
        trim(&mut journal.entries);
        if journal.lines != journal.entries.len() || loaded > CAPACITY {
            journal.rewrite();
        }

        // The counter restarts from the file. After a wipe the file is empty, and
        // the webview has no entries to collide with, so starting at zero is safe.
        journal.seq = journal
            .entries
            .iter()
            .map(|entry| entry.seq)
            .max()
            .unwrap_or(0)
            .max(journal.seq);

        journal
    }

    /// Oldest first: the webview renders the list reversed.
    pub fn all(&self) -> Vec<JournalEntry> {
        self.entries.iter().cloned().collect()
    }

    pub fn path(&self) -> String {
        self.path.display().to_string()
    }

    pub fn push(&mut self, draft: Draft) -> JournalEntry {
        self.seq += 1;
        let entry = JournalEntry {
            seq: self.seq,
            at: Utc::now().to_rfc3339(),
            kind: draft.kind,
            title: truncate(&draft.title),
            detail: draft.detail,
            item_id: draft.item_id,
            stream: draft.stream,
            title_key: draft.key,
            title_args: if draft.args.is_empty() {
                None
            } else {
                Some(draft.args)
            },
            detail_key: draft.detail_key,
            detail_args: if draft.detail_args.is_empty() {
                None
            } else {
                Some(draft.detail_args)
            },
        };

        if let Ok(line) = serde_json::to_string(&entry) {
            if self.append(&line).is_ok() {
                self.lines += 1;
            } else {
                // A failing disk must not lose the in-memory history.
                self.rewrite();
            }
        }

        self.entries.push_back(entry.clone());
        trim(&mut self.entries);
        if self.lines > COMPACT_AT {
            self.rewrite();
        }
        entry
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.rewrite();
    }

    fn append(&self, line: &str) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{line}")
    }

    /// Writes the whole in-memory history beside the target, then renames it.
    fn rewrite(&mut self) {
        let mut body = String::new();
        for entry in &self.entries {
            if let Ok(line) = serde_json::to_string(entry) {
                body.push_str(&line);
                body.push('\n');
            }
        }

        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let temporary = self.path.with_extension("ndjson.tmp");
        if std::fs::write(&temporary, &body).is_ok()
            && std::fs::rename(&temporary, &self.path).is_ok()
        {
            self.lines = self.entries.len();
        }
    }

    fn read_ndjson(&mut self) {
        let Ok(raw) = std::fs::read_to_string(&self.path) else {
            return;
        };

        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // A crash mid-append can leave one truncated line: drop that line,
            // never the file.
            if let Ok(entry) = serde_json::from_str::<JournalEntry>(line) {
                self.lines += 1;
                self.entries.push_back(entry);
            }
        }
    }

    fn import_legacy(&mut self) {
        let legacy = self.path.with_file_name(LEGACY_FILE_NAME);
        let Some(stored) = read_json::<LegacyFile>(&legacy) else {
            return;
        };

        self.seq = stored.seq;
        self.entries = stored.entries.into();
        trim(&mut self.entries);
        self.rewrite();
        if self.lines == self.entries.len() {
            let _ = std::fs::remove_file(&legacy);
        }
    }
}

fn trim(entries: &mut VecDeque<JournalEntry>) {
    while entries.len() > CAPACITY {
        entries.pop_front();
    }
}

/// Keeps a single field readable even when a CLI prints a whole JSON blob.
fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_TEXT {
        return text.to_string();
    }
    let clipped: String = text.chars().take(MAX_TEXT).collect();
    format!("{clipped}…")
}

/// Appends an entry, persists it and tells the webview.
pub fn record(app: &AppHandle, draft: Draft) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };

    let entry = {
        let Ok(mut journal) = state.journal.lock() else {
            return;
        };
        journal.push(draft)
    };

    let _ = app.emit(EVENT, &entry);
}

/// Entries stored so far, oldest first. A failed read is an empty journal.
pub fn list(app: &AppHandle) -> Vec<JournalEntry> {
    app.try_state::<AppState>()
        .and_then(|state| state.journal.lock().ok().map(|journal| journal.all()))
        .unwrap_or_default()
}

pub fn path_string(app: &AppHandle) -> String {
    app.try_state::<AppState>()
        .and_then(|state| state.journal.lock().ok().map(|journal| journal.path()))
        .unwrap_or_default()
}

pub fn clear(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut journal) = state.journal.lock() {
            journal.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or_default();
        let directory = std::env::temp_dir().join(format!(
            "gentle-journal-{name}-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn journal_in(directory: &Path) -> Journal {
        Journal::new(directory.join(FILE_NAME))
    }

    fn lines_on_disk(directory: &Path) -> usize {
        std::fs::read_to_string(directory.join(FILE_NAME))
            .map(|raw| raw.lines().filter(|line| !line.trim().is_empty()).count())
            .unwrap_or(0)
    }

    #[test]
    fn keeps_only_the_newest_entries() {
        let directory = temp_dir("cap");
        let mut journal = journal_in(&directory);

        for index in 0..(CAPACITY + 12) {
            journal.push(Draft::new(kind::CHECK, format!("línea {index}")));
        }

        let entries = journal.all();
        assert_eq!(entries.len(), CAPACITY);
        assert_eq!(entries.last().unwrap().title, format!("línea {}", CAPACITY + 11));
        assert!(
            lines_on_disk(&directory) <= COMPACT_AT,
            "the file compacts instead of growing without bound"
        );
        std::fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn survives_a_reopen_with_monotonic_ids() {
        let directory = temp_dir("reopen");
        let mut journal = journal_in(&directory);
        journal.push(Draft::new(kind::DETECTED, "uno"));
        let last = journal.push(Draft::new(kind::APPLIED, "dos")).seq;

        let reopened = journal_in(&directory);
        assert_eq!(reopened.all().len(), 2, "entries come back from the file");
        assert_eq!(reopened.all().last().unwrap().seq, last);

        let mut journal = journal_in(&directory);
        assert_eq!(
            journal.push(Draft::new(kind::CHECK, "tres")).seq,
            last + 1,
            "ids keep growing after a restart"
        );
        std::fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn clear_forgets_the_entries_but_not_the_sequence() {
        let directory = temp_dir("clear");
        let mut journal = journal_in(&directory);
        let last = journal.push(Draft::new(kind::CHECK, "uno")).seq;
        journal.clear();

        assert!(journal.all().is_empty());
        assert_eq!(
            journal.push(Draft::new(kind::CHECK, "dos")).seq,
            last + 1,
            "clearing does not recycle ids inside the session"
        );

        let reopened = journal_in(&directory);
        assert_eq!(reopened.all().len(), 1, "clearing is persisted");
        std::fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn a_truncated_last_line_is_dropped_but_the_rest_survives() {
        let directory = temp_dir("truncated");
        let mut journal = journal_in(&directory);
        journal.push(Draft::new(kind::CHECK, "buena"));
        journal.push(Draft::new(kind::CHECK, "también buena"));

        let path = directory.join(FILE_NAME);
        let mut raw = std::fs::read_to_string(&path).unwrap();
        raw.push_str("{\"seq\": 99, \"at\": \"2026-");
        std::fs::write(&path, raw).unwrap();

        let reopened = journal_in(&directory);
        assert_eq!(reopened.all().len(), 2);
        assert_eq!(reopened.all()[1].seq, 2, "the broken id never appears");
        std::fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn imports_the_legacy_document_once() {
        let directory = temp_dir("legacy");
        let legacy = directory.join(LEGACY_FILE_NAME);
        std::fs::write(
            &legacy,
            r#"{"seq":7,"entries":[
                {"seq":6,"at":"2026-10-28T20:00:00+00:00","kind":"app","title":"viejo","detail":null,"itemId":null,"stream":null},
                {"seq":7,"at":"2026-10-28T20:01:00+00:00","kind":"check","title":"otro","detail":null,"itemId":null,"stream":null}
            ]}"#,
        )
        .unwrap();

        let mut journal = journal_in(&directory);

        assert_eq!(journal.all().len(), 2);
        assert!(directory.join(FILE_NAME).exists(), "the new file is written");
        assert!(!legacy.exists(), "the old document is gone");
        assert_eq!(
            journal.push(Draft::new(kind::CHECK, "nuevo")).seq,
            8,
            "the sequence continues after the import"
        );
        std::fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn long_lines_are_clipped() {
        let directory = temp_dir("clip");
        let mut journal = journal_in(&directory);
        let entry = journal.push(Draft::new(kind::OUTPUT, "x".repeat(MAX_TEXT * 2)));

        assert!(entry.title.chars().count() <= MAX_TEXT + 1);
        assert!(entry.title.ends_with('…'));
        std::fs::remove_dir_all(directory).ok();
    }
}
