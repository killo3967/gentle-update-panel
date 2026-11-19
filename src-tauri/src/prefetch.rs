//! Warms the notes of a version that has just appeared, so opening the detail
//! finds them already translated instead of waiting for an engine.
//!
//! Nothing here is on the critical path: it runs aside from the check, it never
//! touches the snapshot, and a failure only costs a retry on the next check.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager};

use crate::domain::{Release, Status, UpdateItem};
use crate::journal::{self, kind, Draft};
use crate::locale;
use crate::state::AppState;

/// One warm-up at a time. The keyless engines answer 429 as soon as several
/// notes are asked for at once, and two checks in a row would otherwise leave
/// two of them running against the same engines.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Frees the slot even when the warm-up returns early.
struct Slot;

impl Drop for Slot {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

/// The key the detail view asks the translation with, so the warm-up and the
/// panel land on the same cache entry. Mirrors what `loadTranslation` in
/// `store.ts` puts in front of `translationKey`.
pub fn translation_cache_key(notes_key: &str, version: &str) -> String {
    format!("{notes_key}|{version}")
}

/// The release the panel opens first: the pending one, not the newest of the
/// feed. `None` when the feed does not carry it, and then there is nothing
/// worth warming.
pub fn pending_release<'a>(releases: &'a [Release], latest: Option<&str>) -> Option<&'a Release> {
    let latest = latest?;
    releases.iter().find(|release| release.version == latest)
}

/// Only an update that is actually waiting has notes the user will open: an
/// ignored row is out of every count, and a missing one has no version to look
/// for yet.
pub fn worth_warming(item: &UpdateItem) -> bool {
    !item.ignored
        && item.status == Status::UpdateAvailable
        && item.notes_key.is_some()
        && item.latest.is_some()
}

/// Starts the warm-up without waiting for it. The check returns to the panel
/// while this keeps working in the background.
pub fn spawn(app: &AppHandle, items: Vec<UpdateItem>) {
    let targets: Vec<UpdateItem> = items.into_iter().filter(worth_warming).collect();
    if targets.is_empty() {
        return;
    }
    if !app.state::<AppState>().config().translation_enabled {
        return;
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let _slot = Slot;
        warm(&handle, targets).await;
    });
}

async fn warm(app: &AppHandle, items: Vec<UpdateItem>) {
    let (notes, translations) = {
        let state = app.state::<AppState>();
        (state.notes.clone(), state.translations.clone())
    };
    let target = locale::target_language();
    for item in &items {
        let Some(notes_key) = item.notes_key.as_deref() else {
            continue;
        };
        // The same call the detail makes, so the notes land in the cache it
        // reads and the download is not paid twice.
        let payload = notes
            .get(
                notes_key,
                item.installed.as_deref(),
                item.latest.as_deref(),
                false,
                Some(item.name.as_str()),
            )
            .await;
        if payload.error.is_some() {
            continue;
        }
        let Some(release) = pending_release(&payload.releases, item.latest.as_deref()) else {
            continue;
        };
        if release.body.trim().is_empty() {
            continue;
        }
        let cache_key = translation_cache_key(notes_key, &release.version);
        if translations
            .translate(&cache_key, &release.body, "auto", &target, false)
            .await
            .is_err()
        {
            continue;
        }
        journal::record(
            app,
            Draft::new(kind::NOTES, format!("Notas traducidas: {}", item.name))
                .key("journal.notesWarmed")
                .arg("name", item.name.clone())
                .item(&item.id),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str) -> Release {
        Release {
            version: version.to_string(),
            date: None,
            body: format!("cambios de {version}"),
        }
    }

    #[test]
    fn warms_the_key_the_detail_asks_for() {
        assert_eq!(
            translation_cache_key("gh:owner/repo", "1.2.3"),
            "gh:owner/repo|1.2.3"
        );
    }

    #[test]
    fn picks_the_pending_release_and_not_the_newest() {
        let releases = vec![release("2.0.0"), release("1.5.0"), release("1.0.0")];
        let pending = pending_release(&releases, Some("1.5.0")).expect("release");
        assert_eq!(pending.version, "1.5.0");
    }

    #[test]
    fn a_feed_without_the_pending_version_warms_nothing() {
        let releases = vec![release("1.0.0")];
        assert!(pending_release(&releases, Some("1.5.0")).is_none());
        assert!(pending_release(&releases, None).is_none());
    }
}
