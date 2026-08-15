mod collectors;
mod config;
mod costs;
mod db;
mod display;
mod models;
mod server;
mod theme;
mod tui;

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
        #[arg(long, default_value = "6")]
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
    /// Launch the interactive terminal UI
    Tui {
        /// Number of days to look back (default: 30)
        #[arg(short, long, default_value = "30")]
        days: u32,
    },
    /// Recompute cost for all records from current pricing
    Reprice,
    /// Set a manual price override for a model that has appeared in usage
    PriceSet {
        /// Exact model name as recorded in usage data
        model: String,
        /// Input rate in $/million tokens
        #[arg(long)]
        input: f64,
        /// Output rate in $/million tokens
        #[arg(long)]
        output: f64,
        /// Cache read rate in $/million tokens
        #[arg(long)]
        cache_read: Option<f64>,
        /// Cache write rate in $/million tokens
        #[arg(long)]
        cache_write: Option<f64>,
    },
    /// List manual price overrides, or show which models have no price at all
    PriceList {
        /// Show only models in usage data that currently have no price
        #[arg(long)]
        unpriced: bool,
    },
    /// Count Antigravity requests per model/day (token usage unavailable)
    Antigravity {
        /// Number of days to look back (default: 30)
        #[arg(short, long, default_value = "30")]
        days: u32,
        #[arg(long)]
        json: bool,
    },
    /// Generate shell tab-completion scripts
    Completions {
        /// Target shell: bash, elvish, fish, powershell, zsh
        #[arg(value_enum)]
        shell: clap_complete::Shell,
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

    if cfg.fetch_pricing && costs::pricing_cache_stale() {
        print!("Refreshing model pricing... ");
        match costs::update_pricing_cache().await {
            Ok(_) => println!("{}", "ok".green()),
            Err(e) => println!("{}: {} (using cached/fallback)", "warn".yellow(), e),
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

    // Every token has a market price even on subscription/"free" plans; price
    // any record still missing a cost so reports show an estimate, not $0.
    let priced = db.recompute_missing_costs()?;
    if priced > 0 {
        println!(
            "{}",
            format!("Priced {priced} previously-unpriced records at API list rates (estimated).")
                .yellow()
        );
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
            Commands::Tui { days } => {
                tui::run(db, days)?;
            }
            Commands::Reprice => {
                use colored::Colorize;
                if cfg.fetch_pricing && costs::pricing_cache_stale() {
                    print!("Refreshing model pricing... ");
                    match costs::update_pricing_cache().await {
                        Ok(_) => println!("{}", "ok".green()),
                        Err(e) => println!("{}: {} (using cached/fallback)", "warn".yellow(), e),
                    }
                }
                let priced = db.recompute_all_costs()?;
                println!(
                    "{}",
                    format!("Repriced {priced} records at current API list rates.").bold()
                );
            }
            Commands::PriceSet { model, input, output, cache_read, cache_write } => {
                use colored::Colorize;
                let known = db.distinct_model_names()?;
                if !known.contains(&model) {
                    eprintln!("{}", format!("Unknown model: '{}'", model).red().bold());
                    eprintln!(
                        "{}",
                        "price-set only accepts a model name that has actually appeared in usage data."
                            .yellow()
                    );
                    eprintln!();
                    eprintln!("{}", "Models seen in usage records:".bold());
                    if known.is_empty() {
                        eprintln!(
                            "  (none yet — run `tokentracker sync` first so real model names exist)"
                        );
                    } else {
                        for m in &known {
                            eprintln!("  {}", m.cyan());
                        }
                    }
                    let close: Vec<&String> = known
                        .iter()
                        .filter(|m| {
                            m.contains(&model)
                                || model.to_ascii_lowercase().contains(&m.to_ascii_lowercase())
                        })
                        .collect();
                    if !close.is_empty() {
                        eprintln!();
                        eprintln!("{}", "Closest matches:".bold());
                        for m in close {
                            eprintln!("  {}", m.cyan());
                        }
                    }
                    std::process::exit(1);
                }
                let ov = crate::models::PricingOverride {
                    model: model.clone(),
                    input_per_mtok: input,
                    output_per_mtok: output,
                    cache_read_per_mtok: cache_read,
                    cache_write_per_mtok: cache_write,
                    set_at: chrono::Utc::now().to_rfc3339(),
                };
                db.upsert_pricing_override(&ov)?;
                println!(
                    "{}",
                    format!(
                        "Set override for {}: ${:.4}/M in, ${:.4}/M out{}",
                        model.cyan(),
                        input,
                        output,
                        if cache_read.is_some() || cache_write.is_some() {
                            format!(
                                " (cache read ${:.4}/M, cache write ${:.4}/M)",
                                cache_read.unwrap_or(0.0),
                                cache_write.unwrap_or(0.0)
                            )
                        } else {
                            String::new()
                        }
                    )
                    .bold()
                );
            }
            Commands::PriceList { unpriced } => {
                use colored::Colorize;
                if unpriced {
                    let known = db.distinct_model_names()?;
                    let mut unpriced: Vec<String> = Vec::new();
                    for m in &known {
                        if costs::calculate_cost(
                            m,
                            "unknown",
                            0,
                            0,
                            0,
                            0,
                        )
                        .is_none()
                        {
                            unpriced.push(m.clone());
                        }
                    }
                    if unpriced.is_empty() {
                        println!(
                            "{}",
                            "No unpriced models in usage data. Everything has a price."
                                .green()
                                .bold()
                        );
                    } else {
                        println!("{}", "Models in usage data with no price:".bold());
                        for m in &unpriced {
                            println!("  {}", m.yellow());
                        }
                        println!();
                        println!(
                            "{}",
                            "Set a price with `tokentracker price-set <model> --input <rate> --output <rate>`."
                                .dimmed()
                        );
                    }
                } else {
                    let overrides = db.pricing_overrides()?;
                    if overrides.is_empty() {
                        println!("{}", "No manual price overrides set.".dimmed());
                    } else {
                        println!("{}", "Manual price overrides:".bold());
                        for ov in overrides {
                            println!(
                                "  {:<30} ${:.4}/M in  ${:.4}/M out  {}",
                                ov.model.cyan(),
                                ov.input_per_mtok,
                                ov.output_per_mtok,
                                if ov.cache_read_per_mtok.is_some()
                                    || ov.cache_write_per_mtok.is_some()
                                {
                                    format!(
                                        "(cache read ${:.4}, write ${:.4})",
                                        ov.cache_read_per_mtok.unwrap_or(0.0),
                                        ov.cache_write_per_mtok.unwrap_or(0.0)
                                    )
                                } else {
                                    String::new()
                                }
                            );
                        }
                    }
                }
            }
            Commands::Antigravity { days, json } => {
                let collector = collectors::antigravity::AntigravityCollector::new(
                    collectors::antigravity_conv_dir(),
                );
                let requests = collector.collect_requests()?;
                db.upsert_antigravity_requests(&requests)?;
                if json {
                    let since = chrono::Utc::now() - chrono::Duration::days(days as i64);
                    let since_str = since.format("%Y-%m-%d").to_string();
                    let rows = db.query_antigravity_requests(Some(&since_str))?;
                    println!("{}", serde_json::to_string_pretty(&rows)?);
                } else {
                    let since = chrono::Utc::now() - chrono::Duration::days(days as i64);
                    let since_str = since.format("%Y-%m-%d").to_string();
                    let rows = db.query_antigravity_requests(Some(&since_str))?;
                    display::print_antigravity_requests(&rows);
                }
            }
            Commands::Completions { shell } => {
                use clap::CommandFactory;
                use clap_complete::generate;
                let mut cmd = Cli::command();
                let bin_name = cmd.get_name().to_string();
                generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
            }
        }
        Ok(())
    }

    run(cli, &cfg, &db)
}