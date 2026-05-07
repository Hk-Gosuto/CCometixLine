use super::{Segment, SegmentData};
use crate::config::{InputData, ModelConfig, SegmentId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct ClaudeSettings {
    env: Option<ClaudeEnv>,
}

#[derive(Debug, Deserialize)]
struct ClaudeEnv {
    #[serde(rename = "ANTHROPIC_BASE_URL")]
    anthropic_base_url: Option<String>,
}

#[derive(Default)]
pub struct ModelSegment;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelPricing {
    model_name: String,
    model_ratio: f64,
    completion_ratio: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct PricingCache {
    data: Vec<ModelPricing>,
    cached_at: String,
}

#[derive(Debug, Deserialize)]
struct PricingResponse {
    data: Vec<PricingEntry>,
}

#[derive(Debug, Deserialize)]
struct PricingEntry {
    model_name: String,
    model_ratio: Option<f64>,
    completion_ratio: Option<f64>,
}

impl ModelSegment {
    pub fn new() -> Self {
        Self
    }

    fn debug_enabled() -> bool {
        matches!(
            std::env::var("CCLINE_DEBUG_MODEL")
                .ok()
                .as_deref()
                .map(|v| v.to_ascii_lowercase()),
            Some(v) if v == "1" || v == "true" || v == "yes" || v == "on"
        )
    }

    fn debug_log(message: impl AsRef<str>) {
        if Self::debug_enabled() {
            eprintln!("[model] {}", message.as_ref());
        }
    }

    fn get_cache_path() -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        Some(
            home.join(".claude")
                .join("ccline")
                .join(".pricing_cache.json"),
        )
    }

    fn load_cache() -> Option<PricingCache> {
        let path = Self::get_cache_path()?;
        if !path.exists() {
            return None;
        }
        let content = fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    fn save_cache(cache: &PricingCache) {
        if let Some(path) = Self::get_cache_path() {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(cache) {
                let _ = fs::write(path, json);
            }
        }
    }

    fn is_cache_valid(cache: &PricingCache, cache_duration: u64) -> bool {
        if let Ok(cached_at) = DateTime::parse_from_rfc3339(&cache.cached_at) {
            let elapsed = Utc::now().signed_duration_since(cached_at.with_timezone(&Utc));
            elapsed.num_seconds() < cache_duration as i64
        } else {
            false
        }
    }

    fn get_claude_settings_path() -> Option<PathBuf> {
        if let Ok(config_dir) = std::env::var("CLAUDE_CONFIG_DIR") {
            let path = PathBuf::from(config_dir).join("settings.json");
            if path.exists() {
                Self::debug_log(format!(
                    "using CLAUDE_CONFIG_DIR settings: {}",
                    path.display()
                ));
                return Some(path);
            }
            Self::debug_log(format!(
                "CLAUDE_CONFIG_DIR set but settings.json not found: {}",
                path.display()
            ));
        }
        if let Some(home_dir) = dirs::home_dir() {
            let path = home_dir.join(".claude").join("settings.json");
            if path.exists() {
                Self::debug_log(format!("using default settings: {}", path.display()));
                return Some(path);
            }
            Self::debug_log(format!(
                "default settings.json not found: {}",
                path.display()
            ));
        }
        Self::debug_log("no settings.json found");
        None
    }

    fn load_claude_settings() -> Option<ClaudeSettings> {
        let settings_path = Self::get_claude_settings_path()?;
        let content = fs::read_to_string(&settings_path).ok()?;
        let settings = serde_json::from_str(&content).ok();
        if settings.is_none() {
            Self::debug_log(format!(
                "failed to parse settings.json: {}",
                settings_path.display()
            ));
        }
        settings
    }

    fn get_base_url() -> Option<String> {
        if let Ok(url) = std::env::var("ANTHROPIC_BASE_URL") {
            let trimmed = url.trim();
            if !trimmed.is_empty() {
                Self::debug_log("using ANTHROPIC_BASE_URL from process env");
                return Some(trimmed.to_string());
            }
            Self::debug_log("ANTHROPIC_BASE_URL env var is empty");
        }
        let settings = Self::load_claude_settings()?;
        let base_url = settings.env?.anthropic_base_url?;
        let trimmed = base_url.trim();
        if trimmed.is_empty() {
            Self::debug_log("ANTHROPIC_BASE_URL in settings is empty");
            None
        } else {
            Self::debug_log(format!(
                "using ANTHROPIC_BASE_URL from settings: {}",
                trimmed
            ));
            Some(trimmed.to_string())
        }
    }

    fn fetch_pricing(base_url: &str, timeout_secs: u64) -> Option<Vec<ModelPricing>> {
        let url = if base_url.ends_with('/') {
            format!("{}api/pricing", base_url)
        } else {
            format!("{}/api/pricing", base_url)
        };
        Self::debug_log(format!("fetching pricing from: {}", url));

        let resp = ureq::get(&url)
            .config()
            .timeout_global(Some(std::time::Duration::from_secs(timeout_secs)))
            .build()
            .call()
            .ok()?;

        if resp.status() != 200 {
            Self::debug_log(format!(
                "pricing fetch failed with status: {}",
                resp.status()
            ));
            return None;
        }

        let parsed: PricingResponse = resp.into_body().read_json().ok()?;
        Self::debug_log(format!("pricing fetched: {} models", parsed.data.len()));
        Some(
            parsed
                .data
                .into_iter()
                .filter_map(|e| {
                    Some(ModelPricing {
                        model_name: e.model_name,
                        model_ratio: e.model_ratio?,
                        completion_ratio: e.completion_ratio?,
                    })
                })
                .collect(),
        )
    }

    fn get_pricing(model_id: &str, cache_duration: u64, timeout_secs: u64) -> Option<ModelPricing> {
        let cached = Self::load_cache();
        let list = if cached
            .as_ref()
            .map(|c| Self::is_cache_valid(c, cache_duration))
            .unwrap_or(false)
        {
            Self::debug_log("using cached pricing data");
            cached.unwrap().data
        } else {
            let base_url = match Self::get_base_url() {
                Some(u) => u,
                None => {
                    Self::debug_log("no base_url found, falling back to cache");
                    return cached.and_then(|c| {
                        c.data.into_iter().find(|m| {
                            m.model_name == model_id
                                || model_id.starts_with(&m.model_name)
                                || m.model_name.starts_with(model_id)
                        })
                    });
                }
            };
            match Self::fetch_pricing(&base_url, timeout_secs) {
                Some(data) => {
                    Self::save_cache(&PricingCache {
                        data: data.clone(),
                        cached_at: Utc::now().to_rfc3339(),
                    });
                    data
                }
                None => {
                    Self::debug_log("fetch failed, falling back to cache");
                    cached.map(|c| c.data).unwrap_or_default()
                }
            }
        };

        Self::debug_log(format!(
            "looking up model_id='{}' in {} entries",
            model_id,
            list.len()
        ));
        // Exact match first, then prefix match in both directions
        let found = list
            .iter()
            .find(|m| m.model_name == model_id)
            .or_else(|| list.iter().find(|m| model_id.starts_with(&m.model_name)))
            .or_else(|| list.iter().find(|m| m.model_name.starts_with(model_id)))
            .cloned();
        if found.is_none() {
            Self::debug_log(format!(
                "no match for '{}', available: {}",
                model_id,
                list.iter()
                    .map(|m| m.model_name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        found
    }

    fn format_model_name(&self, id: &str, display_name: &str) -> String {
        let model_config = ModelConfig::load();

        if let Some(config_name) = model_config.get_display_name(id) {
            config_name
        } else {
            let base = if display_name.is_empty() {
                id.to_string()
            } else {
                display_name.to_string()
            };
            match model_config.get_display_suffix(id) {
                Some(suffix) => format!("{}{}", base, suffix),
                None => base,
            }
        }
    }
}

impl Segment for ModelSegment {
    fn collect(&self, input: &InputData) -> Option<SegmentData> {
        let config = crate::config::Config::load().ok()?;
        let segment_config = config.segments.iter().find(|s| s.id == SegmentId::Model);

        let cache_duration = segment_config
            .and_then(|sc| sc.options.get("pricing_cache_duration"))
            .and_then(|v| v.as_u64())
            .unwrap_or(3600);

        let timeout = segment_config
            .and_then(|sc| sc.options.get("pricing_timeout"))
            .and_then(|v| v.as_u64())
            .unwrap_or(3);

        let mut metadata = HashMap::new();
        metadata.insert("model_id".to_string(), input.model.id.clone());
        metadata.insert("display_name".to_string(), input.model.display_name.clone());

        let secondary =
            if let Some(pricing) = Self::get_pricing(&input.model.id, cache_duration, timeout) {
                metadata.insert("model_ratio".to_string(), pricing.model_ratio.to_string());
                metadata.insert(
                    "completion_ratio".to_string(),
                    pricing.completion_ratio.to_string(),
                );
                let out_ratio = pricing.model_ratio * pricing.completion_ratio;
                format_ratio_display(pricing.model_ratio, out_ratio)
            } else {
                String::new()
            };

        Some(SegmentData {
            primary: self.format_model_name(&input.model.id, &input.model.display_name),
            secondary,
            metadata,
        })
    }

    fn id(&self) -> SegmentId {
        SegmentId::Model
    }
}

fn format_ratio_display(input: f64, output: f64) -> String {
    format!("· ↑{} ↓{}", fmt_ratio(input), fmt_ratio(output))
}

fn fmt_ratio(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as u64)
    } else {
        // up to 4 decimal places, strip trailing zeros
        let s = format!("{:.4}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}
