//! The language the panel speaks and the one release notes are translated into:
//! the one picked in Settings, or the one the system is set to when nothing has
//! been picked.

use std::sync::RwLock;

/// Languages the panel has texts for. Anything else is not a target: it falls
/// back to English, which is not a promise about the user but about the panel —
/// it always has something to say.
pub const SUPPORTED: [&str; 6] = ["es", "en", "pt", "fr", "it", "de"];

/// The language picked in Settings. `None` follows Windows.
static CHOSEN: RwLock<Option<String>> = RwLock::new(None);

/// Records the language the user picked. `None` follows Windows, and anything
/// outside [`SUPPORTED`] is ignored: a stale or hand-edited config file cannot
/// leave the panel in a language it has no texts for.
pub fn set_language(language: Option<&str>) {
    let chosen = language
        .and_then(primary_language)
        .filter(|code| SUPPORTED.contains(&code.as_str()));
    if let Ok(mut slot) = CHOSEN.write() {
        *slot = chosen;
    }
}

/// ISO 639-1 code of the language the user reads, e.g. `es`.
pub fn target_language() -> String {
    if let Ok(slot) = CHOSEN.read() {
        if let Some(chosen) = slot.as_ref() {
            return chosen.clone();
        }
    }
    os_locale()
        .as_deref()
        .and_then(primary_language)
        .unwrap_or_else(|| "en".to_string())
}

/// Reduces a locale tag to its language: `es-ES` → `es`, `zh-Hans-CN` → `zh`,
/// `es_ES.UTF-8` → `es`. Anything that does not look like a language is `None`,
/// so the caller can fall back instead of sending nonsense to the engine.
pub fn primary_language(locale: &str) -> Option<String> {
    let head = locale
        .trim()
        .split(|c: char| matches!(c, '-' | '_' | '.' | '@'))
        .next()?
        .trim();
    if head.len() < 2 || head.len() > 3 || !head.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(head.to_ascii_lowercase())
}

#[cfg(windows)]
fn os_locale() -> Option<String> {
    // LOCALE_NAME_MAX_LENGTH from winnls.h.
    const LOCALE_NAME_MAX_LENGTH: usize = 85;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultLocaleName(locale_name: *mut u16, cch_locale_name: i32) -> i32;
    }

    let mut buffer = [0u16; LOCALE_NAME_MAX_LENGTH];
    // SAFETY: the buffer is allocated with exactly the length we declare, so the
    // call can only write inside it. It returns 0 on failure, and the character
    // count (including the terminator) on success.
    let written = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), LOCALE_NAME_MAX_LENGTH as i32) };
    if written <= 1 {
        return None;
    }
    let name = String::from_utf16_lossy(&buffer[..written as usize - 1]);
    Some(name).filter(|name| !name.is_empty())
}

#[cfg(not(windows))]
fn os_locale() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok())
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_plain_language_as_is() {
        assert_eq!(primary_language("es").as_deref(), Some("es"));
        assert_eq!(primary_language("EN").as_deref(), Some("en"));
    }

    #[test]
    fn drops_region_script_and_encoding() {
        assert_eq!(primary_language("es-ES").as_deref(), Some("es"));
        assert_eq!(primary_language("en-US").as_deref(), Some("en"));
        assert_eq!(primary_language("zh-Hans-CN").as_deref(), Some("zh"));
        assert_eq!(primary_language("es_ES.UTF-8").as_deref(), Some("es"));
        assert_eq!(primary_language("ca_ES@valencia").as_deref(), Some("ca"));
        assert_eq!(primary_language("  pt-BR  ").as_deref(), Some("pt"));
    }

    #[test]
    fn rejects_what_is_not_a_language() {
        assert_eq!(primary_language(""), None);
        assert_eq!(primary_language("   "), None);
        assert_eq!(primary_language("e"), None);
        assert_eq!(primary_language("español"), None);
        assert_eq!(primary_language("123"), None);
    }

    #[test]
    fn a_chosen_language_wins_and_unsupported_ones_are_ignored() {
        set_language(Some("de-DE"));
        assert_eq!(target_language(), "de", "the region is dropped, the choice is not");

        // Catalan is a language Windows knows and the panel has no texts for.
        set_language(Some("ca"));
        assert_ne!(target_language(), "ca");

        set_language(None);
        assert_eq!(primary_language(&target_language()).as_deref(), Some(target_language().as_str()));
    }

    #[test]
    fn target_is_a_language_code() {
        let target = target_language();
        assert_eq!(primary_language(&target).as_deref(), Some(target.as_str()));
    }
}
