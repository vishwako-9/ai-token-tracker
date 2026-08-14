use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;

use super::Collector;
use crate::models::UsageRecord;

/// Antigravity CLI stores each conversation as a SQLite DB whose rows are
/// protobuf-encoded BLOBs (`steps.step_payload`, `trajectory_metadata_blob.data`,
/// etc.). The protobuf schema is not bundled and token counts are not exposed
/// as plaintext anywhere in the DBs (verified). v1 therefore detects Antigravity
/// but does not collect usage records.
pub struct AntigravityCollector {
    _conv_dir: PathBuf,
}

impl AntigravityCollector {
    pub fn new(conv_dir: PathBuf) -> Self {
        Self { _conv_dir: conv_dir }
    }
}

#[async_trait]
impl Collector for AntigravityCollector {
    fn name(&self) -> &'static str {
        "antigravity"
    }

    async fn collect(&self) -> Result<Vec<UsageRecord>> {
        Ok(vec![])
    }
}