use super::{Segment, SegmentData};
use crate::config::{InputData, SegmentId};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Default)]
pub struct QuotaSegment;

#[derive(Debug, Deserialize)]
struct QuotaResponse {
    code: bool,
    data: TokenUsageData,
    #[serde(rename = "message")]
    _message: String,
}

#[derive(Debug, Deserialize)]
struct TokenUsageData {
    object: String,
    name: String,
    total_granted: u64,
    total_used: u64,
    total_available: u64,
    unlimited_quota: bool,
    model_limits: HashMap<String, Value>,
    model_limits_enabled: bool,
    expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct ClaudeSettings {
    env: Option<ClaudeEnv>,
}

#[derive(Debug, Deserialize)]
struct ClaudeEnv {
    #[serde(rename = "ANTHROPIC_BASE_URL")]
    anthropic_base_url: Option<String>,
    #[serde(rename = "ANTHROPIC_AUTH_TOKEN")]
    anthropic_auth_token: Option<String>,
}

impl QuotaSegment {
    pub fn new() -> Self {
        Self
    }

    fn debug_enabled() -> bool {
        matches!(
            std::env::var("CCLINE_DEBUG_QUOTA")
                .ok()
                .as_deref()
                .map(|v| v.to_ascii_lowercase()),
            Some(v) if v == "1" || v == "true" || v == "yes" || v == "on"
        )
    }

    fn debug_log(message: impl AsRef<str>) {
        if Self::debug_enabled() {
            eprintln!("[quota] {}", message.as_ref());
        }
    }

    /// Get the Claude settings file path
    fn get_claude_settings_path() -> Option<PathBuf> {
        if let Ok(config_dir) = std::env::var("CLAUDE_CONFIG_DIR") {
            let settings_path = PathBuf::from(config_dir).join("settings.json");
            if settings_path.exists() {
                Self::debug_log(format!(
                    "using CLAUDE_CONFIG_DIR settings: {}",
                    settings_path.display()
                ));
                return Some(settings_path);
            }
            Self::debug_log(format!(
                "CLAUDE_CONFIG_DIR set but settings.json not found: {}",
                settings_path.display()
            ));
        }

        if let Some(home_dir) = dirs::home_dir() {
            let settings_path = home_dir.join(".claude").join("settings.json");
            if settings_path.exists() {
                Self::debug_log(format!(
                    "using default settings: {}",
                    settings_path.display()
                ));
                return Some(settings_path);
            }
            Self::debug_log(format!(
                "default settings.json not found: {}",
                settings_path.display()
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

    /// Read ANTHROPIC_BASE_URL from env or Claude settings
    fn get_anthropic_base_url() -> Option<String> {
        if let Ok(base_url) = std::env::var("ANTHROPIC_BASE_URL") {
            let trimmed = base_url.trim();
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
            Self::debug_log("using ANTHROPIC_BASE_URL from settings");
            Some(trimmed.to_string())
        }
    }

    /// Read ANTHROPIC_AUTH_TOKEN from env or Claude settings
    fn get_anthropic_auth_token() -> Option<String> {
        if let Ok(auth_token) = std::env::var("ANTHROPIC_AUTH_TOKEN") {
            let trimmed = auth_token.trim();
            if !trimmed.is_empty() {
                Self::debug_log(format!(
                    "using ANTHROPIC_AUTH_TOKEN from process env (len={})",
                    trimmed.len()
                ));
                return Some(trimmed.to_string());
            }
            Self::debug_log("ANTHROPIC_AUTH_TOKEN env var is empty");
        }

        let settings = Self::load_claude_settings()?;
        let auth_token = settings.env?.anthropic_auth_token?;
        let trimmed = auth_token.trim();
        if trimmed.is_empty() {
            Self::debug_log("ANTHROPIC_AUTH_TOKEN in settings is empty");
            None
        } else {
            Self::debug_log(format!(
                "using ANTHROPIC_AUTH_TOKEN from settings (len={})",
                trimmed.len()
            ));
            Some(trimmed.to_string())
        }
    }

    fn fetch_quota_data() -> Option<QuotaResponse> {
        use ureq;

        // Get base URL from Claude settings
        let base_url = Self::get_anthropic_base_url()?;
        let auth_token = Self::get_anthropic_auth_token()?;

        // Construct the usage endpoint URL
        let usage_url = if base_url.ends_with('/') {
            format!("{}api/usage/token/", base_url)
        } else {
            format!("{}/api/usage/token/", base_url)
        };
        Self::debug_log(format!("requesting {}", usage_url));

        let response = match ureq::get(&usage_url)
            .header("Authorization", &format!("Bearer {}", auth_token))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .config()
            .timeout_global(Some(std::time::Duration::from_secs(5)))
            .build()
            .call()
        {
            Ok(response) => response,
            Err(err) => {
                Self::debug_log(format!("request failed: {}", err));
                return None;
            }
        };

        if response.status() == 200 {
            let parsed = response.into_body().read_json::<QuotaResponse>().ok();
            if parsed.is_none() {
                Self::debug_log("failed to parse quota response body as JSON");
            }
            parsed
        } else {
            let status = response.status();
            let body = response
                .into_body()
                .read_to_string()
                .unwrap_or_else(|_| "<failed to read body>".to_string());
            Self::debug_log(format!("unexpected status {} body: {}", status, body));
            None
        }
    }

    fn format_count(value: u64) -> String {
        match value {
            1_000_000_000.. => {
                let scaled = value as f64 / 1_000_000_000.0;
                if scaled.fract() == 0.0 {
                    format!("{}B", scaled as u64)
                } else {
                    format!("{scaled:.1}B")
                }
            }
            1_000_000.. => {
                let scaled = value as f64 / 1_000_000.0;
                if scaled.fract() == 0.0 {
                    format!("{}M", scaled as u64)
                } else {
                    format!("{scaled:.1}M")
                }
            }
            1_000.. => {
                let scaled = value as f64 / 1_000.0;
                if scaled.fract() == 0.0 {
                    format!("{}k", scaled as u64)
                } else {
                    format!("{scaled:.1}k")
                }
            }
            _ => value.to_string(),
        }
    }

    fn format_quota_display(quota: &TokenUsageData) -> String {
        if quota.unlimited_quota {
            "∞".to_string()
        } else {
            let percent = if quota.total_granted == 0 {
                0.0
            } else {
                quota.total_available as f64 * 100.0 / quota.total_granted as f64
            };

            let remaining = Self::format_count(quota.total_available);
            if percent.fract() == 0.0 {
                format!("{remaining} ({percent:.0}%)")
            } else {
                format!("{remaining} ({percent:.1}%)")
            }
        }
    }

    fn format_display_name(name: &str) -> String {
        let trimmed = name.trim();
        let short = trimmed
            .split('(')
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(trimmed);
        short.to_string()
    }
}

impl Segment for QuotaSegment {
    fn collect(&self, _input: &InputData) -> Option<SegmentData> {
        let base_url = Self::get_anthropic_base_url()?;
        let response = Self::fetch_quota_data()?;
        if !response.code || response.data.object != "token_usage" {
            Self::debug_log(format!(
                "response rejected: code={} object={}",
                response.code, response.data.object
            ));
            return None;
        }

        let quota = response.data;
        let primary_display = Self::format_display_name(&quota.name);
        let secondary_display = format!("· {}", Self::format_quota_display(&quota));

        let mut metadata = HashMap::new();
        metadata.insert("name".to_string(), quota.name.clone());
        metadata.insert("total_granted".to_string(), quota.total_granted.to_string());
        metadata.insert("total_used".to_string(), quota.total_used.to_string());
        metadata.insert(
            "total_available".to_string(),
            quota.total_available.to_string(),
        );
        metadata.insert(
            "unlimited_quota".to_string(),
            quota.unlimited_quota.to_string(),
        );
        metadata.insert(
            "model_limits_enabled".to_string(),
            quota.model_limits_enabled.to_string(),
        );
        metadata.insert(
            "model_limit_count".to_string(),
            quota.model_limits.len().to_string(),
        );
        metadata.insert("expires_at".to_string(), quota.expires_at.to_string());
        metadata.insert(
            "expires_at_display".to_string(),
            if quota.expires_at == 0 {
                "never".to_string()
            } else {
                quota.expires_at.to_string()
            },
        );
        metadata.insert("base_url".to_string(), base_url);
        if !quota.model_limits.is_empty() {
            let mut model_names = quota.model_limits.keys().cloned().collect::<Vec<_>>();
            model_names.sort();
            metadata.insert("model_limits".to_string(), model_names.join(","));
        }

        Some(SegmentData {
            primary: primary_display,
            secondary: secondary_display,
            metadata,
        })
    }

    fn id(&self) -> SegmentId {
        SegmentId::Quota
    }
}
