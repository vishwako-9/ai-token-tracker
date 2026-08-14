use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_db_path")]
    pub db_path: String,
    #[serde(default = "default_true")]
    pub claude_code_enabled: bool,
    #[serde(default = "default_true")]
    pub codex_enabled: bool,
    #[serde(default = "default_true")]
    pub opencode_enabled: bool,
    #[serde(default = "default_true")]
    pub gemini_cli_enabled: bool,
    #[serde(default = "default_true")]
    pub antigravity_enabled: bool,
    #[serde(default)]
    pub ollama_enabled: bool,
    #[serde(default)]
    pub ollama_host: Option<String>,
    #[serde(default = "default_true")]
    pub fetch_pricing: bool,
    #[serde(skip)]
    pub config_path: PathBuf,
}

fn default_db_path() -> String {
    config_dir()
        .join("tokentracker.db")
        .to_string_lossy()
        .to_string()
}

fn default_true() -> bool {
    true
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tokentracker")
}

fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn load_config() -> Result<Config> {
    let path = config_file();
    let mut cfg = if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        let mut cfg: Config = toml::from_str(&content)?;
        cfg.config_path = path.clone();
        cfg
    } else {
        Config {
            db_path: default_db_path(),
            claude_code_enabled: true,
            codex_enabled: true,
            opencode_enabled: true,
            gemini_cli_enabled: true,
            antigravity_enabled: true,
            ollama_enabled: false,
            ollama_host: None,
            fetch_pricing: true,
            config_path: path.clone(),
        }
    };

    apply_env_overrides(&mut cfg);

    if path.exists() {
        tighten_config_permissions(&path)?;
    }

    Ok(cfg)
}

pub fn save_config(cfg: &Config) -> Result<()> {
    let dir = cfg
        .config_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("config_path has no parent directory"))?;
    std::fs::create_dir_all(dir)?;
    let content = toml::to_string_pretty(cfg)?;
    std::fs::write(&cfg.config_path, content)?;
    tighten_config_permissions(&cfg.config_path)?;
    Ok(())
}

pub fn set_config_value(cfg: &Config, key: &str, value: &str) -> Result<()> {
    let mut cfg = cfg.clone();
    match key {
        "db_path" => cfg.db_path = value.to_string(),
        "claude_code_enabled" => cfg.claude_code_enabled = value.parse()?,
        "codex_enabled" => cfg.codex_enabled = value.parse()?,
        "opencode_enabled" => cfg.opencode_enabled = value.parse()?,
        "gemini_cli_enabled" => cfg.gemini_cli_enabled = value.parse()?,
        "antigravity_enabled" => cfg.antigravity_enabled = value.parse()?,
        "ollama_enabled" => cfg.ollama_enabled = value.parse()?,
        "ollama_host" => cfg.ollama_host = Some(value.to_string()),
        "fetch_pricing" => cfg.fetch_pricing = value.parse()?,
        _ => anyhow::bail!("Unknown config key: {}", key),
    }
    save_config(&cfg)?;
    Ok(())
}

fn apply_env_overrides(cfg: &mut Config) {
    if let Some(value) = env_var_value("TOKENTRACKER_DB_PATH") {
        cfg.db_path = value;
    }
    if let Some(value) = env_var_value("TOKENTRACKER_OLLAMA_HOST") {
        cfg.ollama_host = Some(value);
    }
    for (name, var) in [
        ("claude_code_enabled", "TOKENTRACKER_CLAUDE_CODE"),
        ("codex_enabled", "TOKENTRACKER_CODEX"),
        ("opencode_enabled", "TOKENTRACKER_OPENCODE"),
        ("gemini_cli_enabled", "TOKENTRACKER_GEMINI_CLI"),
        ("antigravity_enabled", "TOKENTRACKER_ANTIGRAVITY"),
        ("ollama_enabled", "TOKENTRACKER_OLLAMA"),
    ] {
        if let Some(v) = env_var_value(var) {
            if let Ok(b) = v.parse() {
                match name {
                    "claude_code_enabled" => cfg.claude_code_enabled = b,
                    "codex_enabled" => cfg.codex_enabled = b,
                    "opencode_enabled" => cfg.opencode_enabled = b,
                    "gemini_cli_enabled" => cfg.gemini_cli_enabled = b,
                    "antigravity_enabled" => cfg.antigravity_enabled = b,
                    _ => cfg.ollama_enabled = b,
                }
            }
        }
    }
}

fn env_var_value(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

#[cfg(unix)]
fn tighten_config_permissions(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn tighten_config_permissions(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

pub fn print_config(cfg: &Config) {
    use colored::Colorize;
    println!("{}", "tokentracker configuration".bold());
    println!("  config: {}", cfg.config_path.display());
    println!("  db:     {}", cfg.db_path);
    println!();
    println!("{}", "Collectors:".bold());
    for (name, enabled) in [
        ("claude_code", cfg.claude_code_enabled),
        ("codex", cfg.codex_enabled),
        ("opencode", cfg.opencode_enabled),
        ("gemini_cli", cfg.gemini_cli_enabled),
        ("antigravity", cfg.antigravity_enabled),
        ("ollama", cfg.ollama_enabled),
    ] {
        let label = if enabled { "enabled".green() } else { "disabled".dimmed() };
        println!("  {:<14} {}", name, label);
    }
    let host = cfg
        .ollama_host
        .clone()
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    println!("  {:<14} {}", "ollama_host", host);
    println!(
        "  {:<14} {}",
        "fetch_pricing",
        if cfg.fetch_pricing {
            "on".green()
        } else {
            "off".dimmed()
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_loads_when_no_file() {
        let cfg = load_config().unwrap();
        assert!(cfg.claude_code_enabled);
        assert!(!cfg.ollama_enabled);
    }

    #[test]
    fn set_and_reload_roundtrips() {
        let dir = std::env::temp_dir().join(format!("ttcfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let mut cfg = Config {
            db_path: default_db_path(),
            claude_code_enabled: true,
            codex_enabled: true,
            opencode_enabled: true,
            gemini_cli_enabled: true,
            antigravity_enabled: true,
            ollama_enabled: false,
            ollama_host: None,
            fetch_pricing: true,
            config_path: path.clone(),
        };
        save_config(&cfg).unwrap();

        set_config_value(&cfg, "ollama_enabled", "true").unwrap();
        cfg = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(cfg.ollama_enabled);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn unknown_key_is_rejected() {
        let cfg = Config {
            db_path: "x".into(),
            claude_code_enabled: true,
            codex_enabled: true,
            opencode_enabled: true,
            gemini_cli_enabled: true,
            antigravity_enabled: true,
            ollama_enabled: false,
            ollama_host: None,
            fetch_pricing: true,
            config_path: PathBuf::from("x.toml"),
        };
        assert!(set_config_value(&cfg, "nope", "1").is_err());
    }
}