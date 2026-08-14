use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use std::path::PathBuf;

use super::Collector;
use crate::models::UsageRecord;

/// Gemini CLI stores per-session JSONL under
/// `~/.gemini/tmp/<user>/chats/session-<id>.json` (one JSON object per line).
/// Each line has `message.metadata.usageMetadata` plus `message.metadata.model`.
pub struct GeminiCliCollector {
    tmp_dir: PathBuf,
}

impl GeminiCliCollector {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            tmp_dir: home.join(".gemini").join("tmp"),
        }
    }
}

impl Default for GeminiCliCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct Line {
    #[serde(default)]
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    metadata: Option<Metadata>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    #[serde(default)]
    usage_metadata: Option<UsageMetadata>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageMetadata {
    #[serde(default)]
    prompt_token_count: Option<i64>,
    #[serde(default)]
    candidates_token_count: Option<i64>,
    #[serde(default)]
    cached_content_token_count: Option<i64>,
    #[serde(default)]
    thoughts_token_count: Option<i64>,
}

fn parse_session_file(path: &std::path::Path, collected_at: &str) -> Vec<UsageRecord> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };

    let mut records = Vec::new();
    let mut model_hint: Option<String> = None;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Line>(line) else {
            continue;
        };
        let Some(msg) = entry.message.as_ref() else {
            continue;
        };
        let Some(meta) = msg.metadata.as_ref() else {
            continue;
        };
        if model_hint.is_none() {
            model_hint = meta.model.clone();
        }
        let Some(usage) = meta.usage_metadata.as_ref() else {
            continue;
        };

        let input = usage.prompt_token_count.unwrap_or(0).max(0);
        let output = usage.candidates_token_count.unwrap_or(0).max(0);
        if input == 0 && output == 0 {
            continue;
        }

        let model = model_hint.clone().unwrap_or_else(|| "gemini-unknown".to_string());

        records.push(UsageRecord {
            id: None,
            provider: "gemini_cli".to_string(),
            model,
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: usage.cached_content_token_count.unwrap_or(0).max(0),
            cache_write_tokens: 0,
            reasoning_tokens: usage.thoughts_token_count.unwrap_or(0).max(0),
            cost_usd: None,
            session_id: None,
            recorded_at: Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            collected_at: collected_at.to_string(),
            metadata: None,
        });
    }
    records
}

fn collect_chats(dir: &std::path::Path, collected_at: &str, records: &mut Vec<UsageRecord>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_chats(&path, collected_at, records);
        } else {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string());
            if name.is_some_and(|n| n.starts_with("session-") && n.ends_with(".json")) {
                records.extend(parse_session_file(&path, collected_at));
            }
        }
    }
}

#[async_trait]
impl Collector for GeminiCliCollector {
    fn name(&self) -> &'static str {
        "gemini_cli"
    }

    async fn collect(&self) -> Result<Vec<UsageRecord>> {
        let collected_at = Utc::now().to_rfc3339();
        let mut records = Vec::new();
        collect_chats(&self.tmp_dir, &collected_at, &mut records);
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tmp(content: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("tt-gemini-{}-{nanos}.json", std::process::id()));
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn parses_usage_metadata() {
        let path = write_tmp(
            r#"{"message":{"role":"model","metadata":{"model":"gemini-3-flash","usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":50,"cachedContentTokenCount":20,"thoughtsTokenCount":5}}}}"#,
        );
        let recs = parse_session_file(&path, "t");
        let _ = std::fs::remove_file(&path);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].input_tokens, 100);
        assert_eq!(recs[0].output_tokens, 50);
        assert_eq!(recs[0].cache_read_tokens, 20);
        assert_eq!(recs[0].reasoning_tokens, 5);
        assert_eq!(recs[0].model, "gemini-3-flash");
    }

    #[test]
    fn skips_lines_without_usage() {
        let path = write_tmp(r#"{"message":{"role":"user","content":"hi"}}"#);
        let recs = parse_session_file(&path, "t");
        let _ = std::fs::remove_file(&path);
        assert!(recs.is_empty());
    }
}