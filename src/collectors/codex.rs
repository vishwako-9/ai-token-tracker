use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use std::path::{Path, PathBuf};

use super::Collector;
use crate::models::UsageRecord;

pub struct CodexCollector {
    codex_dir: PathBuf,
}

impl CodexCollector {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            codex_dir: home.join(".codex"),
        }
    }
}

impl Default for CodexCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct Line {
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    payload: Option<Payload>,
}

#[derive(Debug, Deserialize)]
struct Payload {
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    info: Option<TokenInfo>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenInfo {
    #[serde(default)]
    last_token_usage: Option<TokenUsage>,
}

#[derive(Debug, Deserialize)]
struct TokenUsage {
    #[serde(default)]
    input_tokens: Option<i64>,
    #[serde(default)]
    cached_input_tokens: Option<i64>,
    #[serde(default)]
    output_tokens: Option<i64>,
    #[serde(default)]
    reasoning_output_tokens: Option<i64>,
}

struct FileResult {
    records: Vec<UsageRecord>,
}

fn parse_rollout_file(path: &Path, collected_at: &str) -> FileResult {
    let Ok(content) = std::fs::read_to_string(path) else {
        return FileResult { records: Vec::new() };
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

        let Some(payload) = entry.payload.as_ref() else {
            continue;
        };

        if model_hint.is_none() {
            model_hint = payload.model.clone();
        }

        // Only token_count events carry billable usage. We use the per-turn
        // delta (`last_token_usage`), not the cumulative `total_token_usage`.
        if entry.r#type.as_deref() != Some("event_msg") {
            continue;
        }
        if payload.r#type.as_deref() != Some("token_count") {
            continue;
        }
        let Some(last) = payload
            .info
            .as_ref()
            .and_then(|i| i.last_token_usage.as_ref())
        else {
            continue;
        };

        let input = last.input_tokens.unwrap_or(0).max(0);
        let output = last.output_tokens.unwrap_or(0).max(0);
        if input == 0 && output == 0 {
            continue;
        }

        let model = model_hint.clone().unwrap_or_else(|| "codex".to_string());

        records.push(UsageRecord {
            id: None,
            provider: "codex".to_string(),
            model,
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: last.cached_input_tokens.unwrap_or(0).max(0),
            cache_write_tokens: 0,
            reasoning_tokens: last.reasoning_output_tokens.unwrap_or(0).max(0),
            // Codex on subscription plans has no per-token billing.
            cost_usd: None,
            session_id: None,
            recorded_at: entry
                .timestamp
                .clone()
                .unwrap_or_else(|| Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string()),
            collected_at: collected_at.to_string(),
            metadata: None,
        });
    }

    FileResult { records }
}

/// Recurse a directory collecting every `*.jsonl` whose filename starts with
/// `rollout-`. Codex writes archived rollouts into
/// `~/.codex/archived_sessions` and active ones under `~/.codex/sessions/<y>/<m>/<d>`.
fn collect_rollouts(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rollouts(&path, out);
        } else if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("rollout-") && n.to_string_lossy().ends_with(".jsonl"))
        {
            out.push(path);
        }
    }
}

#[async_trait]
impl Collector for CodexCollector {
    fn name(&self) -> &'static str {
        "codex"
    }

    async fn collect(&self) -> Result<Vec<UsageRecord>> {
        let collected_at = Utc::now().to_rfc3339();
        let mut records = Vec::new();

        let archived = self.codex_dir.join("archived_sessions");
        let active = self.codex_dir.join("sessions");

        let mut files = Vec::new();
        if archived.exists() {
            collect_rollouts(&archived, &mut files);
        }
        if active.exists() {
            collect_rollouts(&active, &mut files);
        }

        for f in files {
            records.extend(parse_rollout_file(&f, &collected_at).records);
        }

        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tmp(content: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("tt-codex-{}-{nanos}.jsonl", std::process::id()));
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn extracts_last_token_usage_deltas() {
        let path = write_tmp(
            r#"{"timestamp":"2026-08-01T00:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":30000},"last_token_usage":{"input_tokens":15000,"cached_input_tokens":12000,"output_tokens":300,"reasoning_output_tokens":20}}}}"#,
        );
        let res = parse_rollout_file(&path, "t");
        let _ = std::fs::remove_file(&path);
        assert_eq!(res.records.len(), 1);
        let r = &res.records[0];
        assert_eq!(r.provider, "codex");
        assert_eq!(r.input_tokens, 15000);
        assert_eq!(r.cache_read_tokens, 12000);
        assert_eq!(r.output_tokens, 300);
        assert_eq!(r.reasoning_tokens, 20);
    }

    #[test]
    fn ignores_non_token_count_events() {
        let path = write_tmp(
            r#"{"timestamp":"t","type":"event_msg","payload":{"type":"task_started"}}
{"timestamp":"t","type":"response_item","payload":{"type":"message"}}"#,
        );
        let res = parse_rollout_file(&path, "t");
        let _ = std::fs::remove_file(&path);
        assert!(res.records.is_empty());
    }

    #[test]
    fn captures_model_hint_from_payload() {
        let path = write_tmp(
            r#"{"timestamp":"t","type":"session_meta","payload":{"model":"gpt-5"}}
{"timestamp":"t","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":5}}}}"#,
        );
        let res = parse_rollout_file(&path, "t");
        let _ = std::fs::remove_file(&path);
        assert_eq!(res.records.len(), 1);
        assert_eq!(res.records[0].model, "gpt-5");
    }
}