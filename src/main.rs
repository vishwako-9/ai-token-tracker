mod collectors;
mod config;
mod costs;
mod db;
mod display;
mod models;
mod server;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::collectors::Provider;

#[derive(Parser)]
#[command(name = "tokentracker")]
#[command(about = "Local-first AI coding token usage and cost tracker")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Sync usage data from local collectors
    Sync {
        /// Specific provider to sync (default: all enabled)
        #[arg(short, long)]
        provider: Option<Provider>,
    },
    /// Show usage summary
    Summary {
        /// Number of days to look back (default: 30)
        #[arg(short, long, default_value = "30")]
        days: u32,
        /// Filter by provider
        #[arg(short = 'P', long)]
        provider: Option<Provider>,
        /// Filter by model (substring)
        #[arg(short, long)]
        model: Option<String>,
    },
    /// Show daily usage breakdown
    Daily {
        #[arg(short, long, default_value = "90")]
        days: u32,
        #[arg(short = 'P', long)]
        provider: Option<Provider>,
        #[arg(short, long)]
        model: Option<String>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        all: bool,
    },
    /// Show weekly usage breakdown (ISO weeks)
    Weekly {
        #[arg(short, long, default_value = "12")]
        weeks: u32,
        #[arg(short = 'P', long)]
        provider: Option<Provider>,
        #[arg(short, long)]
        model: Option<String>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        all: bool,
    },
    /// Show monthly usage breakdown
    Monthly {
        #[arg(short, long, default_value = "6")]
        months: u32,
        #[arg(short = 'P', long)]
        provider: Option<Provider>,
        #[arg(short, long)]
        model: Option<String>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        all: bool,
    },
    /// Show recent records
    Detail {
        #[arg(short, long)]
        model: Option<String>,
        #[arg(short = 'P', long)]
        provider: Option<Provider>,
        #[arg(short, long)]
        since: Option<String>,
        #[arg(short, long)]
        until: Option<String>,
        #[arg(short, long, default_value = "50")]
        limit: usize,
    },
    /// List known models and their pricing
    Models {
        #[arg(short = 'P', long)]
        provider: Option<String>,
    },
    /// Show collector detection status
    Status,
    /// Manage configuration
    Config {
        /// Set a config value (KEY=VALUE)
        #[arg(short, long)]
        set: Option<String>,
        /// Show current config
        #[arg(short, long)]
        list: bool,
    },
    /// Update model pricing from LiteLLM
    UpdatePricing,
    /// Export usage data
    Export {
        #[arg(short, long, default_value = "csv")]
        format: String,
        #[arg(short, long)]
        output: Option<String>,
        #[arg(short, long, default_value = "30")]
        days: u32,
    },
    /// Serve the local web dashboard
    Serve {
        /// Port to bind (default: 7680)
        #[arg(short, long)]
        port: Option<u16>,
    },
}

fn canonical_provider(p: Option<Provider>) -> Option<&'static str> {
    p.map(Provider::canonical_name)
}

async fn cmd_sync(
    cfg: &config::Config,
    db: &db::Database,
    provider_filter: Option<&str>,
) -> Result<()> {
    use colored::Colorize;

    if cfg.fetch_pricing {
        let cache = dirs::cache_dir()
            .unwrap_or_default()
            .join("tokentracker")
            .join("litellm_pricing.json");
        if !cache.exists() {
            print!("First run: fetching model pricing... ");
            match costs::update_pricing_cache().await {
                Ok(_) => println!("{}", "ok".green()),
                Err(e) => println!("{}: {} (using fallback)", "warn".yellow(), e),
            }
        }
    }

    let providers = collectors::get_collectors(cfg, provider_filter)?;

    if providers.is_empty() {
        if let Some(filter) = provider_filter {
            println!("{}", collectors::explain_provider_filter(cfg, filter).yellow());
        } else {
            println!(
                "{}",
                "No providers detected. Run `tokentracker status` to see why.".yellow()
            );
        }
        return Ok(());
    }

    let mut total_records = 0usize;
    for collector in &providers {
        let name = collector.name();
        print!("Syncing {}... ", name.cyan());
        match collector.collect().await {
            Ok(records) => {
                let count = records.len();
                let mut inserted = 0usize;
                for record in records {
                    inserted += db.insert_record(&record)?;
                }
                total_records += count;
                println!(
                    "{} ({} records, {} new)",
                    "ok".green(),
                    count,
                    inserted
                );
            }
            Err(e) => {
                println!("{}: {}", "error".red(), e);
            }
        }
    }
    println!(
        "{}",
        format!("Synced {total_records} usage records.").bold()
    );
    Ok(())
}

fn cmd_status(cfg: &config::Config) -> Result<()> {
    use colored::Colorize;
    println!("{}", "Collector status".bold());
    for status in collectors::local_collector_statuses() {
        let state = match status.state {
            collectors::LocalCollectorState::Detected => "detected".green(),
            collectors::LocalCollectorState::NotFound => "not found".dimmed(),
            collectors::LocalCollectorState::Unsupported => "unsupported".yellow(),
        };
        if let Some(note) = status.note {
            println!(
                "  {:<14} {} ({}) - {}",
                status.name,
                state,
                status.path.display(),
                note
            );
        } else {
            println!(
                "  {:<14} {} ({})",
                status.name,
                state,
                status.path.display()
            );
        }
    }

    if cfg.ollama_enabled {
        let host = cfg
            .ollama_host
            .clone()
            .unwrap_or_else(|| "http://localhost:11434".to_string());
        let ollama = collectors::ollama::OllamaCollector::new(host);
        let models = tokio::runtime::Runtime::new()?.block_on(ollama.loaded_models());
        if models.is_empty() {
            println!(
                "  {:<14} {}",
                "ollama models",
                "none currently loaded".dimmed()
            );
        } else {
            println!(
                "  {:<14} {}",
                "ollama models",
                models.join(", ").cyan()
            );
        }
    }
    Ok(())
}

fn cmd_config(cfg: &config::Config, set: Option<&str>, list: bool) -> Result<()> {
    use colored::Colorize;
    if let Some(kv) = set {
        let (key, value) = kv
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("Expected KEY=VALUE format"))?;
        config::set_config_value(cfg, key.trim(), value.trim())?;
        println!("Set {} = {}", key.trim().cyan(), value.trim());
    } else if list || true {
        config::print_config(cfg);
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = config::load_config()?;
    let db = db::Database::open(&cfg.db_path)?;

    #[tokio::main]
    async fn run(
        cli: Cli,
        cfg: &config::Config,
        db: &db::Database,
    ) -> Result<()> {
        match cli.command {
            Commands::Sync { provider } => {
                cmd_sync(cfg, db, canonical_provider(provider)).await?;
            }
            Commands::Summary { days, provider, model } => {
                let rows = db.query_summary(days, canonical_provider(provider), model.as_deref())?;
                display::print_summary(&rows);
            }
            Commands::Daily { days, provider, model, since, until, json, all } => {
                let rows = db.query_daily(days, canonical_provider(provider), model.as_deref(), since.as_deref(), until.as_deref())?;
                if json {
                    let filtered = display::filter_daily_rows(&rows, all);
                    println!("{}", serde_json::to_string_pretty(&filtered)?);
                } else {
                    display::print_daily(&rows, "Token Usage Report — Daily", all);
                }
            }
            Commands::Weekly { weeks, provider, model, since, until, json, all } => {
                let rows = db.query_weekly(weeks, canonical_provider(provider), model.as_deref(), since.as_deref(), until.as_deref())?;
                if json {
                    let filtered = display::filter_daily_rows(&rows, all);
                    println!("{}", serde_json::to_string_pretty(&filtered)?);
                } else {
                    display::print_daily(&rows, "Token Usage Report — Weekly", all);
                }
            }
            Commands::Monthly { months, provider, model, since, until, json, all } => {
                let rows = db.query_monthly(months, canonical_provider(provider), model.as_deref(), since.as_deref(), until.as_deref())?;
                if json {
                    let filtered = display::filter_daily_rows(&rows, all);
                    println!("{}", serde_json::to_string_pretty(&filtered)?);
                } else {
                    display::print_daily(&rows, "Token Usage Report — Monthly", all);
                }
            }
            Commands::Detail { model, provider, since, until, limit } => {
                let rows = db.query_detail(model.as_deref(), canonical_provider(provider), since.as_deref(), until.as_deref(), Some(limit))?;
                display::print_detail(&rows);
            }
            Commands::Models { provider } => {
                let models = costs::get_model_pricing(provider.as_deref());
                display::print_models(&models);
            }
            Commands::Status => {
                cmd_status(cfg)?;
            }
            Commands::Config { set, list } => {
                cmd_config(cfg, set.as_deref(), list)?;
            }
            Commands::UpdatePricing => {
                use colored::Colorize;
                print!("Fetching pricing from LiteLLM... ");
                costs::update_pricing_cache().await?;
                println!("{}", "ok".green());
                let models = costs::get_model_pricing(None);
                println!("Cached pricing for {} models", models.len());
            }
            Commands::Export { format, output, days } => {
                let since = chrono::Utc::now() - chrono::Duration::days(days as i64);
                let since_str = since.format("%Y-%m-%d").to_string();
                let rows = db.query_detail(None, None, Some(&since_str), None, None)?;
                let content = match format.as_str() {
                    "json" => display::to_json(&rows)?,
                    "csv" => display::to_csv(&rows),
                    other => anyhow::bail!("Unknown export format: '{}'. Supported: csv, json", other),
                };
                match output {
                    Some(path) => {
                        std::fs::write(&path, &content)?;
                        println!("Exported {} records to {}", rows.len(), path);
                    }
                    None => print!("{}", content),
                }
            }
            Commands::Serve { port } => {
                server::serve(&cfg.db_path, port).await?;
            }
        }
        Ok(())
    }

    run(cli, &cfg, &db)
}