use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use std::path::PathBuf;

use super::Collector;
use crate::models::UsageRecord;

pub struct OpenCodeCollector {
    db_path: PathBuf,
}

impl OpenCodeCollector {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }
}

#[derive(Debug, Deserialize)]
struct SessionRow {
    id: Option<String>,
    model: Option<String>,
    tokens_input: Option<i64>,
    tokens_output: Option<i64>,
    tokens_reasoning: Option<i64>,
    tokens_cache_read: Option<i64>,
    tokens_cache_write: Option<i64>,
    cost: Option<f64>,
    time_updated: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ModelJson {
    #[serde(default)]
    id: Option<String>,
}

/// OpenCode stores the model as a JSON object like
/// `{"id":"deepseek-v4-flash-free","providerID":"opencode","variant":"high"}`.
/// Extract the `id` field when possible, otherwise fall back to the raw value.
fn extract_model(raw: &str) -> String {
    serde_json::from_str::<ModelJson>(raw)
        .ok()
        .and_then(|m| m.id)
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| raw.to_string())
}

#[async_trait]
impl Collector for OpenCodeCollector {
    fn name(&self) -> &'static str {
        "opencode"
    }

    async fn collect(&self) -> Result<Vec<UsageRecord>> {
        let collected_at = Utc::now().to_rfc3339();
        let mut records = Vec::new();

        let conn = match rusqlite::Connection::open_with_flags(
            &self.db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(c) => c,
            Err(e) => {
                anyhow::bail!("Cannot open opencode.db ({}): {}", self.db_path.display(), e);
            }
        };

        let mut stmt = conn.prepare(
            "SELECT id, model, tokens_input, tokens_output, tokens_reasoning,
                    tokens_cache_read, tokens_cache_write, cost, time_updated
             FROM session
             WHERE tokens_input > 0 OR tokens_output > 0
             ORDER BY time_updated",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                model: row.get(1)?,
                tokens_input: row.get(2)?,
                tokens_output: row.get(3)?,
                tokens_reasoning: row.get(4)?,
                tokens_cache_read: row.get(5)?,
                tokens_cache_write: row.get(6)?,
                cost: row.get(7)?,
                time_updated: row.get(8)?,
            })
        })?;

        for row in rows {
            let s = row?;
            let input = s.tokens_input.unwrap_or(0).max(0);
            let output = s.tokens_output.unwrap_or(0).max(0);
            if input == 0 && output == 0 {
                continue;
            }

            let recorded_at = s
                .time_updated
                .and_then(epoch_millis_to_rfc3339)
                .unwrap_or_else(|| Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string());

            records.push(UsageRecord {
                id: None,
                provider: "opencode".to_string(),
                model: s
                    .model
                    .map(|m| extract_model(&m))
                    .unwrap_or_else(|| "opencode-unknown".to_string()),
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: s.tokens_cache_read.unwrap_or(0).max(0),
                cache_write_tokens: s.tokens_cache_write.unwrap_or(0).max(0),
                reasoning_tokens: s.tokens_reasoning.unwrap_or(0).max(0),
                cost_usd: s.cost,
                session_id: s.id,
                recorded_at,
                collected_at: collected_at.clone(),
                metadata: None,
            });
        }

        Ok(records)
    }
}

fn epoch_millis_to_rfc3339(ms: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_epoch_millis() {
        let s = epoch_millis_to_rfc3339(1_700_000_000_000).unwrap();
        assert!(s.starts_with("2023-11-14T"));
    }

    #[test]
    fn extracts_model_id_from_json_object() {
        assert_eq!(
            extract_model(r#"{"id":"deepseek-v4-flash-free","providerID":"opencode","variant":"high"}"#),
            "deepseek-v4-flash-free"
        );
        assert_eq!(extract_model("gpt-5"), "gpt-5");
    }
}