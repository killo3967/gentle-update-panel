//! Bounds for the two on-disk caches (release notes and translations).
//!
//! Both are `HashMap<String, Entry>` rewritten whole, so the only thing that
//! keeps them affordable is never letting them grow: an entry is dropped when it
//! is older than the age budget, and the oldest ones go until the entry and byte
//! budgets fit. Without that, the notes cache reached hundreds of KB of dead
//! entries and every single write paid for them.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Serialize};

/// A cache value that knows when it was stored.
pub trait Cached {
    fn fetched_at(&self) -> &str;
}

/// What a cache may keep. Entries are evicted oldest first.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_entries: usize,
    pub max_bytes: usize,
    pub max_age_days: i64,
}

/// Reads a cache, or an empty map when the file is missing or unreadable.
/// A corrupt cache is never fatal: it is simply rebuilt.
pub fn load<T: DeserializeOwned>(path: &Path) -> HashMap<String, T> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Drops expired entries, then the oldest ones until both budgets fit.
/// Returns how many entries were removed.
pub fn prune<T: Cached + Serialize>(
    entries: &mut HashMap<String, T>,
    limits: Limits,
    now: DateTime<Utc>,
) -> usize {
    let before = entries.len();

    // Age first: an expired entry is worthless whatever its size.
    entries.retain(|_, entry| match stamp(entry.fetched_at()) {
        Some(fetched) => (now - fetched).num_days() <= limits.max_age_days,
        None => false,
    });

    let mut oldest = keys_oldest_first(entries);

    while entries.len() > limits.max_entries {
        match oldest.pop() {
            Some(key) => {
                entries.remove(&key);
            }
            None => break,
        }
    }

    // Then the byte budget, measured as the entry is stored.
    let mut bytes: usize = entries.values().map(size).sum::<usize>() + entries.len();
    while bytes > limits.max_bytes && entries.len() > 1 {
        match oldest.pop() {
            Some(key) => {
                if let Some(entry) = entries.remove(&key) {
                    bytes = bytes.saturating_sub(size(&entry) + 1);
                }
            }
            None => break,
        }
    }

    before - entries.len()
}

/// Writes beside the target and renames, so a crash never leaves half a cache.
pub fn store<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let raw = serde_json::to_string(value)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, raw)?;
    std::fs::rename(temporary, path)
}

/// Keys ordered newest first, so `pop()` always yields the oldest one.
fn keys_oldest_first<T: Cached>(entries: &HashMap<String, T>) -> Vec<String> {
    let mut keys: Vec<(DateTime<Utc>, String)> = entries
        .iter()
        .map(|(key, entry)| (stamp(entry.fetched_at()).unwrap_or(DateTime::<Utc>::MIN_UTC), key.clone()))
        .collect();
    keys.sort_by(|left, right| right.0.cmp(&left.0));
    keys.into_iter().map(|(_, key)| key).collect()
}

fn stamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

fn size<T: Serialize>(entry: &T) -> usize {
    serde_json::to_string(entry).map(|raw| raw.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Serialize, serde::Deserialize)]
    struct Entry {
        fetched_at: String,
        body: String,
    }

    impl Cached for Entry {
        fn fetched_at(&self) -> &str {
            &self.fetched_at
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-28T12:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn entry(days_ago: i64, size: usize) -> Entry {
        Entry {
            fetched_at: (now() - chrono::Duration::days(days_ago)).to_rfc3339(),
            body: "x".repeat(size),
        }
    }

    fn map(entries: Vec<Entry>) -> HashMap<String, Entry> {
        entries
            .into_iter()
            .enumerate()
            .map(|(index, entry)| (format!("k{index}"), entry))
            .collect()
    }

    const ROOMY: Limits = Limits {
        max_entries: 10,
        max_bytes: 1_000_000,
        max_age_days: 30,
    };

    #[test]
    fn expired_entries_go_even_when_there_is_room() {
        let mut entries = map(vec![entry(1, 10), entry(90, 10)]);

        let removed = prune(&mut entries, ROOMY, now());

        assert_eq!(removed, 1);
        assert_eq!(entries.len(), 1);
        assert!(entries.contains_key("k0"), "the fresh one stays");
    }

    #[test]
    fn the_oldest_go_first_when_the_count_budget_is_exceeded() {
        let mut entries = map(vec![entry(5, 10), entry(3, 10), entry(1, 10)]);
        let limits = Limits {
            max_entries: 2,
            ..ROOMY
        };

        prune(&mut entries, limits, now());

        assert_eq!(entries.len(), 2);
        assert!(!entries.contains_key("k0"), "five days old is the first to go");
        assert!(entries.contains_key("k2"));
    }

    #[test]
    fn the_byte_budget_trims_the_cache_but_never_empties_it() {
        let mut entries = map(vec![entry(5, 400), entry(3, 400), entry(1, 400)]);
        let limits = Limits {
            max_entries: 10,
            max_bytes: 900,
            max_age_days: 30,
        };

        prune(&mut entries, limits, now());

        assert_eq!(entries.len(), 1, "only the newest survives");
        assert!(entries.contains_key("k2"));

        // A single entry over budget is kept: better one big note than none.
        let mut single = map(vec![entry(1, 5_000)]);
        prune(&mut single, limits, now());
        assert_eq!(single.len(), 1);
    }

    #[test]
    fn unparseable_timestamps_are_dropped() {
        let mut entries = map(vec![entry(1, 10)]);
        entries.insert(
            "broken".to_string(),
            Entry {
                fetched_at: "ayer".to_string(),
                body: String::new(),
            },
        );

        prune(&mut entries, ROOMY, now());

        assert!(!entries.contains_key("broken"));
    }

    #[test]
    fn store_and_load_round_trip_atomically() {
        let path = std::env::temp_dir().join(format!("gup-cache-{}.json", std::process::id()));
        let entries = map(vec![entry(1, 10)]);

        store(&path, &entries).unwrap();
        let loaded: HashMap<String, Entry> = load(&path);

        assert_eq!(loaded.len(), 1);
        assert!(loaded["k0"].body.starts_with('x'));
        assert!(!path.with_extension("json.tmp").exists(), "the temp file is renamed away");

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_corrupt_cache_loads_as_empty_instead_of_failing() {
        let path = std::env::temp_dir().join(format!("gup-cache-bad-{}.json", std::process::id()));
        std::fs::write(&path, "{\"k0\": {\"fetched_at\": ").unwrap();

        let loaded: HashMap<String, Entry> = load(&path);

        assert!(loaded.is_empty());
        std::fs::remove_file(path).ok();
    }
}
