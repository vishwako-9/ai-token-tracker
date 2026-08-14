use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;

use super::Collector;
use crate::models::UsageRecord;

/// Ollama exposes loaded models via `/api/ps` but has no historical token
/// usage API. v1 does not fabricate usage: the collector returns no records.
/// Run `tokentracker status` to see currently loaded models.
pub struct OllamaCollector {
    host: String,
}

impl OllamaCollector {
    pub fn new(host: String) -> Self {
        Self { host }
    }
}

#[derive(Debug, Deserialize)]
struct PsResponse {
    #[serde(default)]
    models: Vec<PsModel>,
}

#[derive(Debug, Deserialize)]
struct PsModel {
    #[serde(default)]
    name: String,
}

#[async_trait]
impl Collector for OllamaCollector {
    fn name(&self) -> &'static str {
        "ollama"
    }

    async fn collect(&self) -> Result<Vec<UsageRecord>> {
        // Loaded-models probe only; nothing to record.
        let _ = self.loaded_models().await;
        Ok(vec![])
    }
}

impl OllamaCollector {
    pub async fn loaded_models(&self) -> Vec<String> {
        let url = format!("{}/api/ps", self.host.trim_end_matches('/'));
        let Ok(resp) = reqwest::get(&url).await else {
            return Vec::new();
        };
        let Ok(ps) = resp.json::<PsResponse>().await else {
            return Vec::new();
        };
        ps.models.into_iter().map(|m| m.name).collect()
    }
}