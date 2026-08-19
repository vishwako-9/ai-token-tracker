use std::collections::HashMap;
use std::hash::Hasher;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A provider pricing page to watch. The checker only detects whether the
/// page's content changed since the last run; it never reads or interprets
/// the numbers on it — a human does that.
pub struct WatchEntry {
    pub name: &'static str,
    pub url: &'static str,
}

/// Pages to watch. Add new entries here; nothing else needs to change.
pub const WATCHED_PAGES: &[WatchEntry] = &[WatchEntry {
    name: "DeepSeek Pricing",
    url: "https://api-docs.deepseek.com/quick_start/pricing/",
}];

const STATE_FILENAME: &str = "price_watch.json";

/// Last-seen content hash and check time for a watched URL, persisted in a
/// small JSON file next to `litellm_pricing.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCheck {
    hash: u64,
    checked_at: String,
}

fn state_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tokentracker")
        .join(STATE_FILENAME)
}

/// Simple change-detection checksum of the page body. Deterministic across
/// runs for the same input; no crypto-strength guarantee needed here.
fn hash_body(body: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(body);
    hasher.finish()
}

fn load_state(path: &Path) -> HashMap<String, StoredCheck> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_state(path: &Path, state: &HashMap<String, StoredCheck>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(state)?;
    std::fs::write(path, content)?;
    Ok(())
}

async fn check_entry(entry: &WatchEntry, state: &mut HashMap<String, StoredCheck>) -> Result<()> {
    use colored::Colorize;

    let resp = reqwest::get(entry.url).await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} for {}", resp.status(), entry.url);
    }
    let body = resp.text().await?;
    let hash = hash_body(body.as_bytes());

    match state.get(entry.url) {
        Some(prev) if prev.hash == hash => {
            println!(
                "  {} — {}",
                entry.name.cyan(),
                format!("unchanged since {}", prev.checked_at).green()
            );
        }
        Some(prev) => {
            println!(
                "  {} — {}",
                entry.name.cyan(),
                format!(
                    "CHANGED since {} — go check {}",
                    prev.checked_at, entry.url
                )
                .red()
                .bold()
            );
        }
        None => {
            println!(
                "  {} — {}",
                entry.name.cyan(),
                "first check, baseline saved".cyan()
            );
        }
    }

    state.insert(
        entry.url.to_string(),
        StoredCheck {
            hash,
            checked_at: chrono::Utc::now().to_rfc3339(),
        },
    );
    Ok(())
}

/// Check each watched pricing page for content changes since the last run and
/// persist the fresh hashes. Deliberately change-detection only: nothing is
/// parsed, and no rates are auto-updated.
pub async fn price_check() -> Result<()> {
    use colored::Colorize;

    println!("{}", "Checking watched pricing pages".bold());
    let path = state_path();
    let mut state = load_state(&path);
    for entry in WATCHED_PAGES {
        if let Err(e) = check_entry(entry, &mut state).await {
            println!("  {} — {}", entry.name.cyan(), format!("{e}").red());
        }
    }
    save_state(&path, &state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic_and_sensitive() {
        let a = hash_body(b"deepseek input 0.22 output 0.66");
        let b = hash_body(b"deepseek input 0.22 output 0.66");
        assert_eq!(a, b);
        let c = hash_body(b"deepseek input 0.23 output 0.66");
        assert_ne!(a, c);
    }

    #[test]
    fn state_roundtrips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STATE_FILENAME);
        let mut state = HashMap::new();
        state.insert(
            "https://api-docs.deepseek.com/quick_start/pricing/".to_string(),
            StoredCheck {
                hash: 12345,
                checked_at: "2026-08-19T00:00:00Z".to_string(),
            },
        );
        save_state(&path, &state).unwrap();
        let loaded = load_state(&path);
        assert_eq!(loaded.len(), 1);
        let stored = &loaded["https://api-docs.deepseek.com/quick_start/pricing/"];
        assert_eq!(stored.hash, 12345);
        assert_eq!(stored.checked_at, "2026-08-19T00:00:00Z");
    }

    #[test]
    fn missing_state_file_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STATE_FILENAME);
        assert!(load_state(&path).is_empty());
    }
}