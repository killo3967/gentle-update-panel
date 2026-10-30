//! The language release notes are translated into: the one the system is set to.

/// ISO 639-1 code of the language the user reads, e.g. `es`.
pub fn target_language() -> String {
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
    fn target_is_a_language_code() {
        let target = target_language();
        assert_eq!(primary_language(&target).as_deref(), Some(target.as_str()));
    }
}
