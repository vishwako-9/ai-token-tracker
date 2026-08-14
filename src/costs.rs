use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::Result;
use serde::Deserialize;

use crate::models::ModelPricing;

const LITELLM_PRICING_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

#[derive(Debug, Deserialize)]
struct LiteLLMEntry {
    #[serde(default)]
    litellm_provider: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    input_cost_per_token: Option<f64>,
    #[serde(default)]
    output_cost_per_token: Option<f64>,
    #[serde(default)]
    cache_read_input_token_cost: Option<f64>,
    #[serde(default)]
    cache_creation_input_token_cost: Option<f64>,
}

fn cache_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tokentracker")
        .join("litellm_pricing.json")
}

pub async fn update_pricing_cache() -> Result<()> {
    let resp = reqwest::get(LITELLM_PRICING_URL).await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to fetch LiteLLM pricing: {}", resp.status());
    }
    let body = resp.text().await?;

    let _: HashMap<String, serde_json::Value> = serde_json::from_str(&body)?;

    let path = cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &body)?;
    Ok(())
}

static PRICING_CACHE: OnceLock<Option<HashMap<String, LiteLLMEntry>>> = OnceLock::new();

fn load_cached_pricing() -> Option<&'static HashMap<String, LiteLLMEntry>> {
    PRICING_CACHE
        .get_or_init(|| {
            let path = cache_path();
            let content = std::fs::read_to_string(path).ok()?;
            serde_json::from_str(&content).ok()
        })
        .as_ref()
}

pub fn get_model_pricing(provider_filter: Option<&str>) -> Vec<ModelPricing> {
    let entries = match load_cached_pricing() {
        Some(e) => e,
        None => return get_fallback_pricing(provider_filter),
    };

    let mut models: Vec<ModelPricing> = entries
        .iter()
        .filter_map(|(model_key, entry)| {
            let mode = entry.mode.as_deref().unwrap_or("");
            if mode != "chat" && mode != "completion" {
                return None;
            }
            let litellm_provider = entry.litellm_provider.as_deref()?;
            let provider = normalize_provider(litellm_provider)?;

            if let Some(filter) = provider_filter {
                if provider != filter {
                    return None;
                }
            }

            let input_per_token = entry.input_cost_per_token?;
            let output_per_token = entry.output_cost_per_token?;

            Some(ModelPricing {
                provider: provider.to_string(),
                model: model_key.clone(),
                input_per_mtok: input_per_token * 1_000_000.0,
                output_per_mtok: output_per_token * 1_000_000.0,
                cache_read_per_mtok: entry.cache_read_input_token_cost.map(|c| c * 1_000_000.0),
                cache_write_per_mtok: entry
                    .cache_creation_input_token_cost
                    .map(|c| c * 1_000_000.0),
            })
        })
        .collect();

    models.sort_by(|a, b| a.provider.cmp(&b.provider).then(a.model.cmp(&b.model)));
    models
}

fn normalize_provider(litellm_provider: &str) -> Option<&'static str> {
    match litellm_provider {
        "anthropic" => Some("anthropic"),
        "openai" => Some("openai"),
        "gemini" | "vertex_ai" | "vertex_ai_beta" => Some("gemini"),
        "ollama" | "ollama_chat" => Some("ollama"),
        "deepseek" => Some("deepseek"),
        "openrouter" => Some("openrouter"),
        _ => None,
    }
}

pub fn calculate_cost(
    model: &str,
    provider: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
) -> Option<f64> {
    if let Some(entries) = load_cached_pricing() {
        let prefixed = format!("{}/{}", provider, model);
        let entry = entries
            .get(model)
            .or_else(|| entries.get(&prefixed))
            .or_else(|| {
                model
                    .rsplit_once('/')
                    .and_then(|(_, bare)| entries.get(bare))
            });

        if let Some(entry) = entry {
            if let (Some(input_cpt), Some(output_cpt)) =
                (entry.input_cost_per_token, entry.output_cost_per_token)
            {
                let input_cost = input_tokens as f64 * input_cpt;
                let output_cost = output_tokens as f64 * output_cpt;
                let cache_read_cost = entry
                    .cache_read_input_token_cost
                    .map(|c| cache_read_tokens as f64 * c)
                    .unwrap_or(0.0);
                let cache_write_cost = entry
                    .cache_creation_input_token_cost
                    .map(|c| cache_write_tokens as f64 * c)
                    .unwrap_or(0.0);
                return Some(input_cost + output_cost + cache_read_cost + cache_write_cost);
            }
        }
    }

    calculate_cost_fallback(
        model,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
    )
}

struct FallbackRates {
    input: f64,
    output: f64,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
}

fn rates_no_cache(input: f64, output: f64) -> FallbackRates {
    FallbackRates {
        input,
        output,
        cache_read: None,
        cache_write: None,
    }
}

fn fallback_rates(model: &str) -> Option<FallbackRates> {
    if model.contains("opus") {
        return Some(FallbackRates {
            input: 15.0,
            output: 75.0,
            cache_read: Some(1.5),
            cache_write: Some(18.75),
        });
    }
    if model.contains("sonnet") {
        return Some(FallbackRates {
            input: 3.0,
            output: 15.0,
            cache_read: Some(0.3),
            cache_write: Some(3.75),
        });
    }
    if model.contains("haiku") {
        return Some(FallbackRates {
            input: 0.80,
            output: 4.0,
            cache_read: Some(0.08),
            cache_write: Some(1.0),
        });
    }
    if model.contains("gpt-4o-mini") {
        return Some(rates_no_cache(0.15, 0.60));
    }
    if model.contains("gpt-4o") {
        return Some(rates_no_cache(2.50, 10.0));
    }
    if model.contains("gpt-4.1-nano") {
        return Some(rates_no_cache(0.10, 0.40));
    }
    if model.contains("gpt-4.1-mini") {
        return Some(rates_no_cache(0.40, 1.60));
    }
    if model.contains("gpt-4.1") {
        return Some(rates_no_cache(2.0, 8.0));
    }
    if model.contains("gpt-5-mini") {
        return Some(rates_no_cache(0.25, 2.0));
    }
    if model.contains("gpt-5") {
        return Some(rates_no_cache(1.25, 10.0));
    }
    if has_word(model, "o4-mini") {
        return Some(rates_no_cache(1.10, 4.40));
    }
    if has_word(model, "o3-mini") {
        return Some(rates_no_cache(1.10, 4.40));
    }
    if has_word(model, "o3") {
        return Some(rates_no_cache(2.0, 8.0));
    }
    if model.contains("deepseek-reasoner") {
        return Some(rates_no_cache(0.55, 2.19));
    }
    if model.contains("deepseek-chat") {
        return Some(rates_no_cache(0.27, 1.10));
    }
    if model.contains("gemini-2.5-flash") || model.contains("gemini-3-flash") {
        return Some(rates_no_cache(0.30, 2.50));
    }
    if model.contains("gemini-3") || model.contains("gemini-2.5-pro") {
        return Some(rates_no_cache(1.25, 10.0));
    }
    if model.contains("gemini-2.0-flash") {
        return Some(rates_no_cache(0.10, 0.40));
    }
    None
}

fn has_word(model: &str, token: &str) -> bool {
    let bytes = model.as_bytes();
    let tb = token.as_bytes();
    if tb.is_empty() || bytes.len() < tb.len() {
        return false;
    }
    for i in 0..=bytes.len() - tb.len() {
        if &bytes[i..i + tb.len()] != tb {
            continue;
        }
        let left_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let right_idx = i + tb.len();
        let right_ok = right_idx == bytes.len() || !bytes[right_idx].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
    }
    false
}

fn calculate_cost_fallback(
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
) -> Option<f64> {
    let rates = fallback_rates(model)?;

    let per_mtok = |tokens: i64, rate: f64| (tokens as f64 / 1_000_000.0) * rate;
    let cost = per_mtok(input_tokens, rates.input)
        + per_mtok(output_tokens, rates.output)
        + rates
            .cache_read
            .map(|r| per_mtok(cache_read_tokens, r))
            .unwrap_or(0.0)
        + rates
            .cache_write
            .map(|r| per_mtok(cache_write_tokens, r))
            .unwrap_or(0.0);
    Some(cost)
}

fn get_fallback_pricing(provider_filter: Option<&str>) -> Vec<ModelPricing> {
    let all = vec![
        mp("anthropic", "claude-opus-4", 15.0, 75.0, Some(1.5), Some(18.75)),
        mp("anthropic", "claude-sonnet-4", 3.0, 15.0, Some(0.3), Some(3.75)),
        mp("anthropic", "claude-haiku-3-5", 0.80, 4.0, Some(0.08), Some(1.0)),
        mp("openai", "gpt-5", 1.25, 10.0, None, None),
        mp("openai", "gpt-5-mini", 0.25, 2.0, None, None),
        mp("openai", "gpt-4o", 2.50, 10.0, None, None),
        mp("openai", "gpt-4o-mini", 0.15, 0.60, None, None),
        mp("openai", "o3-mini", 1.10, 4.40, None, None),
        mp("gemini", "gemini-3-pro", 1.25, 10.0, None, None),
        mp("gemini", "gemini-3-flash", 0.30, 2.50, None, None),
        mp("gemini", "gemini-2.5-flash", 0.30, 2.50, None, None),
        mp("deepseek", "deepseek-chat", 0.27, 1.10, None, None),
        mp("deepseek", "deepseek-reasoner", 0.55, 2.19, None, None),
        mp("ollama", "local-models", 0.0, 0.0, None, None),
    ];

    match provider_filter {
        Some(p) => all.into_iter().filter(|m| m.provider == p).collect(),
        None => all,
    }
}

fn mp(
    provider: &str,
    model: &str,
    input: f64,
    output: f64,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
) -> ModelPricing {
    ModelPricing {
        provider: provider.to_string(),
        model: model.to_string(),
        input_per_mtok: input,
        output_per_mtok: output,
        cache_read_per_mtok: cache_read,
        cache_write_per_mtok: cache_write,
    }
}

#[cfg(test)]
mod fallback_tests {
    use super::*;

    #[test]
    fn opus_includes_cache_costs() {
        let cost = calculate_cost_fallback(
            "claude-opus-4-20250514",
            1_000_000,
            1_000_000,
            1_000_000,
            1_000_000,
        )
        .unwrap();
        assert!((cost - 110.25).abs() < 1e-9);
    }

    #[test]
    fn gpt_4o1_preview_is_not_priced_as_o1() {
        let priced = calculate_cost_fallback("gpt-4o1-preview", 1_000_000, 0, 0, 0);
        if let Some(c) = priced {
            assert!((c - 15.0).abs() > 1e-6, "mispriced");
        }
    }

    #[test]
    fn unknown_model_returns_none() {
        assert!(calculate_cost_fallback("totally-unknown", 1, 1, 0, 0).is_none());
    }

    #[test]
    fn gpt_5_family_has_pricing() {
        let c = calculate_cost_fallback("gpt-5", 1_000_000, 0, 0, 0).unwrap();
        assert!((c - 1.25).abs() < 1e-9);
        let m = calculate_cost_fallback("gpt-5-mini", 1_000_000, 0, 0, 0).unwrap();
        assert!((m - 0.25).abs() < 1e-9);
    }

    #[test]
    fn gemini_3_family_has_pricing() {
        let c = calculate_cost_fallback("gemini-3-flash", 1_000_000, 0, 0, 0).unwrap();
        assert!((c - 0.30).abs() < 1e-9);
    }
}