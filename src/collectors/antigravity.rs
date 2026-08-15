use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::Collector;
use crate::models::{AntigravityRequest, UsageRecord};

/// Antigravity CLI stores each conversation as a SQLite DB whose rows are
/// protobuf-encoded BLOBs (`steps.step_payload`, `trajectory_metadata_blob.data`,
/// etc.). The protobuf schema is not bundled and token counts are not exposed
/// as plaintext anywhere in the DBs (verified). v1 therefore detects Antigravity
/// but does not collect usage records.
///
/// Request *volume* is recoverable though: every generation row in the
/// `gen_metadata` table carries the model name (nested field 19) and a Unix
/// timestamp (nested field 1). `collect_requests` aggregates those into
/// per-day, per-model counts so the tool tracks Antigravity usage volume even
/// though token cost is unavailable.
pub struct AntigravityCollector {
    conv_dir: PathBuf,
}

impl AntigravityCollector {
    pub fn new(conv_dir: PathBuf) -> Self {
        Self { conv_dir }
    }

    /// Count generation requests per (date, model) across all conversation DBs.
    pub fn collect_requests(&self) -> Result<Vec<AntigravityRequest>> {
        let mut counts: BTreeMap<(String, String), i64> = BTreeMap::new();

        for entry in std::fs::read_dir(&self.conv_dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "db") {
                for (model, ts) in read_request_metadata(&path)? {
                    let date = DateTime::<Utc>::from_timestamp(ts, 0)
                        .map(|d| d.format("%Y-%m-%d").to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    *counts.entry((date, model)).or_insert(0) += 1;
                }
            }
        }

        Ok(counts
            .into_iter()
            .map(|((date, model), request_count)| AntigravityRequest {
                date,
                model,
                request_count,
            })
            .collect())
    }
}

/// Read `gen_metadata` rows from one conversation DB, returning (model, epoch
/// seconds) pairs. Model-less rows (failed requests) are bucketed as
/// "unknown". DBs that are not SQLite (e.g. stale `.db` files) are skipped.
fn read_request_metadata(path: &std::path::Path) -> Result<Vec<(String, i64)>> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;

    let has_table = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='gen_metadata')",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if has_table == 0 {
        return Ok(vec![]);
    }

    let mut out = Vec::new();
    let mut stmt = conn.prepare("SELECT data FROM gen_metadata")?;
    let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
    for row in rows {
        let blob = row?;
        let (model, ts) = scan_request_metadata(&blob);
        if let Some(ts) = ts {
            out.push((model, ts));
        }
    }
    Ok(out)
}

/// Walk a `gen_metadata` protobuf blob to find the model name (nested field
/// 19, a short printable string) and a Unix timestamp (nested field 1 varint
/// in the plausible epoch range). Returns the last model found and the first
/// timestamp found.
fn scan_request_metadata(blob: &[u8]) -> (String, Option<i64>) {
    let mut model: Option<String> = None;
    let mut ts: Option<i64> = None;
    walk(blob, 0, &mut model, &mut ts);
    (model.unwrap_or_else(|| "unknown".to_string()), ts)
}

/// Recursive protobuf walker. Bounded by depth to survive malformed blobs.
fn walk(blob: &[u8], depth: usize, model: &mut Option<String>, ts: &mut Option<i64>) {
    if depth > 8 || blob.len() > 4 * 1024 * 1024 {
        return;
    }
    let mut i = 0usize;
    while i < blob.len() {
        let Some((field, wire_type)) = read_varint(blob, &mut i).map(|tag| (tag >> 3, tag & 7)) else {
            return;
        };
        match wire_type {
            0 => {
                let Some(value) = read_varint(blob, &mut i) else {
                    return;
                };
                // Nested field 1 in the epoch range: request timestamp.
                if field == 1 && (1_700_000_000..2_100_000_000).contains(&value) && ts.is_none() {
                    *ts = Some(value as i64);
                }
            }
            1 => {
                i += 8;
                if i > blob.len() {
                    return;
                }
            }
            2 => {
                let Some(len) = read_varint(blob, &mut i).map(|l| l as usize) else {
                    return;
                };
                if i + len > blob.len() {
                    return;
                }
                let sub = &blob[i..i + len];
                i += len;
                // Nested field 19 holding a short printable string: model name.
                if field == 19 && is_printable(sub) && sub.len() < 64 {
                    *model = Some(String::from_utf8_lossy(sub).into_owned());
                }
                walk(sub, depth + 1, model, ts);
            }
            5 => {
                i += 4;
                if i > blob.len() {
                    return;
                }
            }
            _ => return,
        }
    }
}

fn read_varint(blob: &[u8], i: &mut usize) -> Option<u64> {
    let mut value: u64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = *blob.get(*i)?;
        *i += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

fn is_printable(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(|b| (0x20..0x7f).contains(b))
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