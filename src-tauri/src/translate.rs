//! Free, keyless translation of release notes.
//!
//! Two unofficial endpoints, both measured on this machine:
//! - Google `translate_a/single` (fast, better with markdown, but requires
//!   SChannel/native-tls: rustls gets a 429) → primary.
//! - Microsoft `translate/translatetext` (no token, works with any TLS stack,
//!   but destroys inline code) → fallback.
//!
//! The translator never sees code, URLs or upload helpers: those become
//! `[[Tn]]` placeholders and are restored afterwards. If a placeholder is lost,
//! the translation is discarded rather than shown half-broken.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};

const GOOGLE_URL: &str = "https://translate.googleapis.com/translate_a/single";
const MICROSOFT_URL: &str = "https://edge.microsoft.com/translate/translatetext";
const GOOGLE_BLOCK: usize = 1_500;
const MICROSOFT_BLOCK: usize = 4_000;
const THROTTLE: Duration = Duration::from_millis(250);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BODY_CHARS: usize = 40_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslatedText {
    pub text: String,
    pub engine: String,
    pub cached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Protected {
    pub text: String,
    pub tokens: Vec<String>,
}

/// Replaces fenced blocks, inline code, link destinations and bare URLs with
/// `[[Tn]]` placeholders so a translator cannot rewrite them.
pub fn protect(markdown: &str) -> Protected {
    let mut tokens: Vec<String> = Vec::new();

    let fenced = Regex::new(r"(?s)```.*?```").expect("static regex");
    let mut text = replace_matches(markdown, &fenced, &mut tokens);

    let inline = Regex::new(r"`[^`\n]+`").expect("static regex");
    text = replace_matches(&text, &inline, &mut tokens);

    let link = Regex::new(r"\[([^\]]*)\]\((https?://[^)\s]+)\)").expect("static regex");
    text = replace_link_targets(&text, &link, &mut tokens);

    let bare = Regex::new(r"https?://[^\s)\]>]+").expect("static regex");
    text = replace_matches(&text, &bare, &mut tokens);

    Protected { text, tokens }
}

/// Puts the placeholders back. Returns `None` if any was lost or duplicated,
/// because that means the translator mangled the structure.
pub fn restore(translated: &str, tokens: &[String]) -> Option<String> {
    let mut text = translated.to_string();
    for (index, token) in tokens.iter().enumerate() {
        let placeholder = format!("[[T{index}]]");
        if text.matches(&placeholder).count() != 1 {
            return None;
        }
        text = text.replace(&placeholder, token);
    }
    Some(text)
}

/// Splits markdown into paragraph blocks, splitting long ones by sentence so a
/// GET request stays inside the URL limits.
pub fn blocks(markdown: &str, max_chars: usize) -> Vec<String> {
    let mut result = Vec::new();

    for block in markdown.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        if block.chars().count() <= max_chars {
            result.push(block.to_string());
            continue;
        }

        let mut buffer = String::new();
        for sentence in split_sentences(block) {
            if buffer.chars().count() + sentence.chars().count() + 1 > max_chars && !buffer.is_empty()
            {
                result.push(std::mem::take(&mut buffer));
            }
            if !buffer.is_empty() {
                buffer.push(' ');
            }
            buffer.push_str(&sentence);
        }
        if !buffer.is_empty() {
            result.push(buffer);
        }
    }
    result
}

/// Sentence splitter without look-around (the regex crate does not support it).
/// A boundary is `.`, `!` or `?` followed by whitespace; `v4.3.1` stays whole.
fn split_sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut index = 0;

    while index < chars.len() {
        let character = chars[index];
        current.push(character);

        if matches!(character, '.' | '!' | '?') {
            let mut next = index + 1;
            while next < chars.len() && chars[next].is_whitespace() {
                next += 1;
            }
            if next > index + 1 && next < chars.len() {
                parts.push(current.trim().to_string());
                current.clear();
                index = next;
                continue;
            }
        }
        index += 1;
    }

    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

/// Joins consecutive blocks while they fit, so a long note needs two or three
/// requests instead of one per paragraph.
pub fn pack(blocks: Vec<String>, max_chars: usize) -> Vec<String> {
    let mut packed: Vec<String> = Vec::new();
    let mut current = String::new();

    for block in blocks {
        if current.is_empty() {
            current = block;
            continue;
        }
        if current.chars().count() + block.chars().count() + 2 <= max_chars {
            current.push_str("

");
            current.push_str(&block);
        } else {
            packed.push(std::mem::take(&mut current));
            current = block;
        }
    }
    if !current.is_empty() {
        packed.push(current);
    }
    packed
}

fn replace_matches(input: &str, pattern: &Regex, tokens: &mut Vec<String>) -> String {
    let mut out = String::new();
    let mut last = 0;
    for found in pattern.find_iter(input) {
        out.push_str(&input[last..found.start()]);
        out.push_str(&format!("[[T{}]]", tokens.len()));
        tokens.push(found.as_str().to_string());
        last = found.end();
    }
    out.push_str(&input[last..]);
    out
}

fn replace_link_targets(input: &str, pattern: &Regex, tokens: &mut Vec<String>) -> String {
    let mut out = String::new();
    let mut last = 0;
    for captures in pattern.captures_iter(input) {
        let whole = captures.get(0).expect("group 0");
        out.push_str(&input[last..whole.start()]);
        out.push('[');
        out.push_str(captures.get(1).map(|m| m.as_str()).unwrap_or(""));
        out.push_str("]([[T");
        out.push_str(&tokens.len().to_string());
        out.push_str("]])");
        tokens.push(captures.get(2).expect("group 2").as_str().to_string());
        last = whole.end();
    }
    out.push_str(&input[last..]);
    out
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    text: String,
    engine: String,
    fetched_at: String,
}

pub struct TranslationService {
    client: reqwest::Client,
    cache_path: PathBuf,
    cache: Mutex<HashMap<String, CacheEntry>>,
}

impl TranslationService {
    pub fn new(cache_path: PathBuf) -> Self {
        let cache = std::fs::read_to_string(&cache_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<HashMap<String, CacheEntry>>(&raw).ok())
            .unwrap_or_default();

        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("http client");

        Self {
            client,
            cache_path,
            cache: Mutex::new(cache),
        }
    }

    pub async fn translate(
        &self,
        cache_key: &str,
        markdown: &str,
        engine: &str,
        target: &str,
        force: bool,
    ) -> Result<TranslatedText, String> {
        if markdown.trim().is_empty() {
            return Ok(TranslatedText {
                text: String::new(),
                engine: "none".to_string(),
                cached: true,
            });
        }
        if markdown.chars().count() > MAX_BODY_CHARS {
            return Err(format!(
                "el texto es demasiado largo para traducir ({} caracteres)",
                markdown.chars().count()
            ));
        }

        let hash = fingerprint(markdown);
        let engine_used = if engine == "auto" { "auto" } else { engine };
        let key = format!("{cache_key}|{engine_used}|{target}|{hash}");

        if !force {
            if let Some(entry) = self.cache.lock().ok().and_then(|cache| cache.get(&key).cloned()) {
                return Ok(TranslatedText {
                    text: entry.text,
                    engine: entry.engine,
                    cached: true,
                });
            }
        }

        let (translated, used) = match engine {
            "microsoft" => {
                let text = self
                    .microsoft(markdown, target)
                    .await
                    .map_err(|error| format!("Microsoft: {error}"))?;
                (text, "microsoft".to_string())
            }
            "google" => {
                let text = self
                    .google(markdown, target)
                    .await
                    .map_err(|error| format!("Google: {error}"))?;
                (text, "google".to_string())
            }
            _ => match self.google(markdown, target).await {
                Ok(text) => (text, "google".to_string()),
                Err(google_error) => {
                    let text = self
                        .microsoft(markdown, target)
                        .await
                        .map_err(|ms_error| format!("{google_error} · Microsoft: {ms_error}"))?;
                    (text, "microsoft".to_string())
                }
            },
        };

        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(
                key,
                CacheEntry {
                    text: translated.clone(),
                    engine: used.clone(),
                    fetched_at: Utc::now().to_rfc3339(),
                },
            );
            let snapshot = cache.clone();
            drop(cache);
            self.persist(&snapshot);
        }

        Ok(TranslatedText {
            text: translated,
            engine: used,
            cached: false,
        })
    }

    async fn google(&self, markdown: &str, target: &str) -> Result<String, String> {
        let protected = protect(markdown);
        let chunks = pack(blocks(&protected.text, GOOGLE_BLOCK), GOOGLE_BLOCK);
        let mut parts: Vec<String> = Vec::new();

        for (index, chunk) in chunks.iter().enumerate() {
            if index > 0 {
                tokio::time::sleep(THROTTLE).await;
            }
            parts.push(self.google_chunk(chunk, target).await?);
        }

        let joined = parts.join("\n\n");
        restore(&joined, &protected.tokens)
            .ok_or_else(|| "el traductor alteró el código protegido".to_string())
    }

    async fn google_chunk(&self, text: &str, target: &str) -> Result<String, String> {
        let response = self
            .client
            .get(GOOGLE_URL)
            .query(&[
                ("client", "gtx"),
                ("sl", "auto"),
                ("tl", target),
                ("dt", "t"),
                ("q", text),
            ])
            .send()
            .await
            .map_err(|error| format!("sin respuesta ({error})"))?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status().as_u16()));
        }

        let payload: serde_json::Value = response
            .json()
            .await
            .map_err(|error| format!("respuesta ilegible ({error})"))?;

        let segments = payload
            .get(0)
            .and_then(|value| value.as_array())
            .ok_or("respuesta inesperada")?;

        let mut out = String::new();
        for segment in segments {
            if let Some(text) = segment.get(0).and_then(|value| value.as_str()) {
                out.push_str(text);
            }
        }
        Ok(out)
    }

    async fn microsoft(&self, markdown: &str, target: &str) -> Result<String, String> {
        let protected = protect(markdown);
        let chunks = pack(blocks(&protected.text, MICROSOFT_BLOCK), MICROSOFT_BLOCK);
        if chunks.is_empty() {
            return Ok(String::new());
        }

        let response = self
            .client
            .post(MICROSOFT_URL)
            .query(&[("isEnterpriseClient", "false"), ("to", target)])
            .json(&chunks)
            .send()
            .await
            .map_err(|error| format!("sin respuesta ({error})"))?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status().as_u16()));
        }

        let payload: serde_json::Value = response
            .json()
            .await
            .map_err(|error| format!("respuesta ilegible ({error})"))?;

        let items = payload.as_array().ok_or("respuesta inesperada")?;
        let mut parts = Vec::new();
        for item in items {
            let translated = item
                .get("translations")
                .and_then(|value| value.as_array())
                .and_then(|list| list.first())
                .and_then(|first| first.get("text"))
                .and_then(|value| value.as_str())
                .ok_or("falta el texto traducido")?;
            parts.push(translated.to_string());
        }

        let joined = parts.join("\n\n");
        restore(&joined, &protected.tokens)
            .ok_or_else(|| "el traductor alteró el código protegido".to_string())
    }

    fn persist(&self, cache: &HashMap<String, CacheEntry>) {
        if let Ok(raw) = serde_json::to_string(cache) {
            if let Some(parent) = self.cache_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&self.cache_path, raw);
        }
    }
}

fn fingerprint(text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "Gentle AI v4.3.1 supersedes `v4.3.0`, whose run stopped on an untidy `go.mod`.\n\n\
See [the workflow](https://github.com/o/r/actions/runs/42) for details and read https://example.com/docs.\n\n\
```bash\npi update --extensions\n```";

    #[test]
    fn protects_code_links_and_urls_but_not_prose() {
        let protected = protect(NOTE);

        assert!(!protected.text.contains("`go.mod`"));
        assert!(!protected.text.contains("https://github.com/o/r"));
        assert!(!protected.text.contains("pi update --extensions"));
        assert!(protected.text.contains("supersedes [[T"));
        assert!(protected.text.contains("See [the workflow]([[T"), "link text stays translatable");
        assert_eq!(
            protected.tokens.len(),
            5,
            "2 inline spans + 1 link URL + 1 bare URL + 1 fenced block"
        );
    }

    #[test]
    fn restores_every_token_exactly() {
        let protected = protect(NOTE);
        let restored = restore(&protected.text, &protected.tokens).expect("roundtrip");

        assert_eq!(restored, NOTE);
    }

    #[test]
    fn rejects_a_translation_that_lost_a_placeholder() {
        let protected = protect(NOTE);
        let mangled = protected.text.replace("[[T0]]", "«codigo»");

        assert_eq!(restore(&mangled, &protected.tokens), None);
    }

    #[test]
    fn plain_prose_needs_no_tokens() {
        let protected = protect("Just a sentence with no code.");
        assert!(protected.tokens.is_empty());
        assert_eq!(protected.text, "Just a sentence with no code.");
    }

    #[test]
    fn blocks_split_paragraphs_and_long_ones() {
        let short = blocks("one\n\ntwo", 100);
        assert_eq!(short, vec!["one".to_string(), "two".to_string()]);

        let long = format!("{}. {}. {}", "a".repeat(80), "b".repeat(80), "c".repeat(10));
        let split = blocks(&long, 100);
        assert!(split.len() >= 2, "long block must be split: {split:?}");
        assert!(split.iter().all(|block| block.chars().count() <= 100));
    }

    /// Live check against both unofficial endpoints from the app's own client.
    /// Run with: cargo test --lib live_translates -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_translates_with_both_engines_and_keeps_code() {
        let cache = std::env::temp_dir().join(format!("gup-translations-{}.json", std::process::id()));
        let service = TranslationService::new(cache.clone());
        let text = "The `go.mod` file is tidy and [run 42](https://example.com/x) is green.";

        for engine in ["google", "microsoft"] {
            match service.translate("live", text, engine, "es", true).await {
                Ok(translated) => {
                    println!("{engine} -> {}", translated.text);
                    assert!(
                        translated.text.contains("`go.mod`"),
                        "{engine} lost the protected code span"
                    );
                    assert!(
                        translated.text.contains("https://example.com/x"),
                        "{engine} lost the protected link"
                    );
                }
                Err(error) => panic!("{engine} falló: {error}"),
            }
        }
        std::fs::remove_file(cache).ok();
    }

    #[test]
    fn packing_reduces_requests_without_exceeding_the_limit() {
        let blocks = vec![
            "a".repeat(400),
            "b".repeat(400),
            "c".repeat(400),
            "d".repeat(400),
        ];
        let packed = pack(blocks, 900);

        assert_eq!(packed.len(), 2, "400+400 fits, the third forces a new chunk");
        assert!(packed.iter().all(|chunk| chunk.chars().count() <= 900));
        assert!(packed[0].starts_with('a') && packed[0].contains('b'));
        assert!(packed[1].starts_with('c') && packed[1].contains('d'));
    }

    #[test]
    fn fingerprint_changes_with_the_body() {
        assert_eq!(fingerprint("same"), fingerprint("same"));
        assert_ne!(fingerprint("same"), fingerprint("same "));
    }
}
