pub mod antigravity;
pub mod claude_code;
pub mod codex;
pub mod gemini_cli;
pub mod ollama;
pub mod opencode;

use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;

use crate::config::Config;
use crate::models::UsageRecord;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum Provider {
    ClaudeCode,
    Codex,
    Opencode,
    GeminiCli,
    Antigravity,
    Ollama,
}

impl Provider {
    pub fn canonical_name(self) -> &'static str {
        match self {
            Provider::ClaudeCode => "claude_code",
            Provider::Codex => "codex",
            Provider::Opencode => "opencode",
            Provider::GeminiCli => "gemini_cli",
            Provider::Antigravity => "antigravity",
            Provider::Ollama => "ollama",
        }
    }
}

#[async_trait]
pub trait Collector: Send + Sync {
    fn name(&self) -> &'static str;
    /// Collect usage records. Return records already normalized; cost is
    /// computed by the caller if not set.
    async fn collect(&self) -> Result<Vec<UsageRecord>>;
}

pub struct LocalCollectorStatus {
    pub name: &'static str,
    pub state: LocalCollectorState,
    pub path: PathBuf,
    pub note: Option<&'static str>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LocalCollectorState {
    Detected,
    NotFound,
}

pub fn get_collectors(
    cfg: &Config,
    provider_filter: Option<&str>,
) -> Result<Vec<Box<dyn Collector>>> {
    let provider_filter = provider_filter.map(canonical_provider_name);
    let mut collectors: Vec<Box<dyn Collector>> = Vec::new();

    let should_include = |name: &str| provider_filter.is_none() || provider_filter == Some(name);

    if should_include("claude_code") && cfg.claude_code_enabled {
        collectors.push(Box::new(claude_code::ClaudeCodeCollector::new()));
    }

    if should_include("codex") && cfg.codex_enabled {
        let dir = codex_sessions_dir();
        if dir.exists() {
            collectors.push(Box::new(codex::CodexCollector::new()));
        }
    }

    if should_include("opencode") && cfg.opencode_enabled {
        let db_path = opencode_db_path();
        if db_path.exists() {
            collectors.push(Box::new(opencode::OpenCodeCollector::new(db_path)));
        }
    }

    if should_include("gemini_cli") && cfg.gemini_cli_enabled {
        let dir = gemini_cli_tmp_dir();
        if dir.exists() {
            collectors.push(Box::new(gemini_cli::GeminiCliCollector::new()));
        }
    }

    if should_include("antigravity") && cfg.antigravity_enabled {
        let dir = antigravity_conv_dir();
        if dir.exists() {
            collectors.push(Box::new(antigravity::AntigravityCollector::new(dir)));
        }
    }

    if should_include("ollama") && cfg.ollama_enabled {
        let host = cfg
            .ollama_host
            .clone()
            .unwrap_or_else(|| "http://localhost:11434".to_string());
        collectors.push(Box::new(ollama::OllamaCollector::new(host)));
    }

    Ok(collectors)
}

pub fn local_collector_statuses() -> Vec<LocalCollectorStatus> {
    vec![
        LocalCollectorStatus {
            name: "claude_code",
            state: if claude_projects_dir().exists() {
                LocalCollectorState::Detected
            } else {
                LocalCollectorState::NotFound
            },
            path: claude_projects_dir(),
            note: None,
        },
        LocalCollectorStatus {
            name: "codex",
            state: if codex_sessions_dir().exists() {
                LocalCollectorState::Detected
            } else {
                LocalCollectorState::NotFound
            },
            path: codex_sessions_dir(),
            note: None,
        },
        LocalCollectorStatus {
            name: "opencode",
            state: if opencode_db_path().exists() {
                LocalCollectorState::Detected
            } else {
                LocalCollectorState::NotFound
            },
            path: opencode_db_path(),
            note: None,
        },
        LocalCollectorStatus {
            name: "gemini_cli",
            state: if gemini_cli_tmp_dir().exists() {
                LocalCollectorState::Detected
            } else {
                LocalCollectorState::NotFound
            },
            path: gemini_cli_tmp_dir(),
            note: None,
        },
        LocalCollectorStatus {
            name: "antigravity",
            state: if antigravity_conv_dir().exists() {
                LocalCollectorState::Detected
            } else {
                LocalCollectorState::NotFound
            },
            path: antigravity_conv_dir(),
            note: Some("token usage is not parseable (protobuf blobs); request volume via `tokentracker antigravity`"),
        },
        LocalCollectorStatus {
            name: "ollama",
            state: LocalCollectorState::Detected,
            path: PathBuf::from("http://localhost:11434"),
            note: Some("no historical token usage API; reports loaded models only"),
        },
    ]
}

pub fn canonical_provider_name(name: &str) -> &str {
    match name {
        "antigravity" => "antigravity",
        "gemini-cli" => "gemini_cli",
        _ => name,
    }
}

pub fn explain_provider_filter(cfg: &Config, provider_filter: &str) -> String {
    match canonical_provider_name(provider_filter) {
        "claude_code" => {
            if cfg.claude_code_enabled {
                "Provider 'claude_code' is enabled, but no ~/.claude/projects logs were found."
                    .to_string()
            } else {
                "Provider 'claude_code' is disabled in config.".to_string()
            }
        }
        "codex" => "Provider 'codex' is supported but no ~/.codex session logs were found."
            .to_string(),
        "opencode" => "Provider 'opencode' is supported but no local OpenCode database was found."
            .to_string(),
        "gemini_cli" => {
            "Provider 'gemini_cli' is supported but no ~/.gemini/tmp session logs were found."
                .to_string()
        }
        "antigravity" => "Provider 'antigravity' does not expose token usage locally "
            .to_string()
            + "(conversations are protobuf blobs); run `tokentracker antigravity` for request volume.",
        "ollama" => {
            if cfg.ollama_enabled {
                "Provider 'ollama' is enabled, but Ollama has no historical token usage API; "
                    .to_string()
            } else {
                "Provider 'ollama' is disabled. Run `tokentracker config --set ollama_enabled=true`."
                    .to_string()
            }
        }
        other => format!(
            "Unknown provider '{}'. Known: claude_code, codex, opencode, gemini_cli, antigravity, ollama",
            other
        ),
    }
}

fn claude_projects_dir() -> PathBuf {
    home().join(".claude").join("projects")
}

fn codex_sessions_dir() -> PathBuf {
    home().join(".codex")
}

fn opencode_db_path() -> PathBuf {
    home().join(".local/share/opencode/opencode.db")
}

fn gemini_cli_tmp_dir() -> PathBuf {
    home().join(".gemini").join("tmp")
}

pub fn antigravity_conv_dir() -> PathBuf {
    home().join(".gemini/antigravity-cli/conversations")
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}