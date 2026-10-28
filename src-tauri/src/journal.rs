//! Durable journal of what the panel detected, applied and verified.
//!
//! It is a bounded ring buffer persisted to `journal.json`. The webview reads it
//! once on boot and then appends the `journal:entry` events, so a check that runs
//! while the panel is closed is not lost.

use std::collections::VecDeque;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;
use crate::state::{read_json, write_json};

/// Newest entries kept on disk. This is a log, not an archive.
pub const CAPACITY: usize = 800;

/// Longest text stored per field: one runaway stdout line must not own the file.
const MAX_TEXT: usize = 600;

/// Streamed output lines journaled per step. The activity drawer keeps the rest.
pub const LINES_PER_STEP: usize = 200;

/// Event emitted for every new entry.
pub const EVENT: &str = "journal:entry";

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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct JournalFile {
    seq: u64,
    entries: Vec<JournalEntry>,
}

/// What a caller knows before the journal stamps the time and the sequence.
pub struct Draft {
    kind: String,
    title: String,
    detail: Option<String>,
    item_id: Option<String>,
    stream: Option<String>,
}

impl Draft {
    pub fn new(kind: &'static str, title: impl Into<String>) -> Self {
        Self {
            kind: kind.to_string(),
            title: title.into(),
            detail: None,
            item_id: None,
            stream: None,
        }
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
}

impl Journal {
    pub fn new(path: PathBuf) -> Self {
        let stored = read_json::<JournalFile>(&path).unwrap_or_default();
        let mut entries: VecDeque<JournalEntry> = stored.entries.into();
        let mut seq = stored.seq;
        for entry in &entries {
            seq = seq.max(entry.seq);
        }
        trim(&mut entries);

        Self {
            path,
            seq,
            entries,
        }
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
        };

        self.entries.push_back(entry.clone());
        trim(&mut self.entries);
        self.persist();
        entry
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.persist();
    }

    fn persist(&self) {
        let file = JournalFile {
            seq: self.seq,
            entries: self.entries.iter().cloned().collect(),
        };
        let _ = write_json(&self.path, &file);
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

    fn temp_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or_default();
        path.push(format!("gentle-journal-{name}-{}-{stamp}.json", std::process::id()));
        path
    }

    #[test]
    fn keeps_only_the_newest_entries() {
        let path = temp_path("cap");
        let mut journal = Journal::new(path.clone());

        for index in 0..(CAPACITY + 12) {
            journal.push(Draft::new(kind::CHECK, format!("línea {index}")));
        }

        let entries = journal.all();
        assert_eq!(entries.len(), CAPACITY);
        assert_eq!(entries.last().unwrap().title, format!("línea {}", CAPACITY + 11));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn survives_a_reopen_with_monotonic_ids() {
        let path = temp_path("reopen");
        let mut journal = Journal::new(path.clone());
        journal.push(Draft::new(kind::DETECTED, "uno"));
        let last = journal.push(Draft::new(kind::APPLIED, "dos")).seq;

        let reopened = Journal::new(path.clone());
        assert_eq!(reopened.all().len(), 2, "entries come back from disk");
        assert_eq!(reopened.all().last().unwrap().seq, last);

        let mut journal = Journal::new(path.clone());
        assert_eq!(
            journal.push(Draft::new(kind::CHECK, "tres")).seq,
            last + 1,
            "ids keep growing after a restart"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn clear_forgets_the_entries_but_not_the_sequence() {
        let path = temp_path("clear");
        let mut journal = Journal::new(path.clone());
        let last = journal.push(Draft::new(kind::CHECK, "uno")).seq;
        journal.clear();

        assert!(journal.all().is_empty());
        assert_eq!(journal.push(Draft::new(kind::CHECK, "dos")).seq, last + 1);

        let reopened = Journal::new(path.clone());
        assert_eq!(reopened.all().len(), 1, "clearing is persisted");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn long_lines_are_clipped() {
        let path = temp_path("clip");
        let mut journal = Journal::new(path.clone());
        let entry = journal.push(Draft::new(kind::OUTPUT, "x".repeat(MAX_TEXT * 2)));

        assert!(entry.title.chars().count() <= MAX_TEXT + 1);
        assert!(entry.title.ends_with('…'));
        let _ = std::fs::remove_file(path);
    }
}
