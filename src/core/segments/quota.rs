use super::{Segment, SegmentData};
use crate::config::{InputData, SegmentId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Default)]
pub struct QuotaSegment;

#[derive(Debug, Deserialize, Serialize)]
struct QuotaSnapshot {
    entitlement: u32,
    overage_count: u32,
    overage_permitted: bool,
    percent_remaining: f64,
    quota_id: String,
    quota_remaining: u32,
    remaining: u32,
    unlimited: bool,
    timestamp_utc: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct QuotaSnapshots {
    chat: QuotaSnapshot,
    completions: QuotaSnapshot,
    premium_interactions: QuotaSnapshot,
}

#[derive(Debug, Deserialize, Serialize)]
struct QuotaResponse {
    access_type_sku: String,
    analytics_tracking_id: String,
    assigned_date: String,
    can_signup_for_limited: bool,
    chat_enabled: bool,
    copilot_plan: String,
    organization_login_list: Vec<String>,
    quota_reset_date: String,
    quota_snapshots: QuotaSnapshots,
    quota_reset_date_utc: String,
}

#[derive(Debug, Deserialize)]
struct ClaudeSettings {
    env: Option<ClaudeEnv>,
}

#[derive(Debug, Deserialize)]
struct ClaudeEnv {
    #[serde(rename = "ANTHROPIC_BASE_URL")]
    anthropic_base_url: Option<String>,
}

impl QuotaSegment {
    pub fn new() -> Self {
        Self
    }

    /// Get the Claude settings file path
    fn get_claude_settings_path() -> Option<PathBuf> {
        if let Some(home_dir) = dirs::home_dir() {
            let settings_path = home_dir.join(".claude").join("settings.json");
            if settings_path.exists() {
                return Some(settings_path);
            }
        }
        None
    }

    /// Read ANTHROPIC_BASE_URL from Claude settings
    fn get_anthropic_base_url() -> Option<String> {
        let settings_path = Self::get_claude_settings_path()?;
        let content = fs::read_to_string(settings_path).ok()?;
        let settings: ClaudeSettings = serde_json::from_str(&content).ok()?;
        settings.env?.anthropic_base_url
    }

    #[cfg(feature = "quota")]
    fn fetch_quota_data() -> Option<QuotaResponse> {
        use ureq;

        // Get base URL from Claude settings
        let base_url = Self::get_anthropic_base_url()?;

        // Construct the usage endpoint URL
        let usage_url = if base_url.ends_with('/') {
            format!("{}usage", base_url)
        } else {
            format!("{}/usage", base_url)
        };

        let response = ureq::get(&usage_url)
            .timeout(std::time::Duration::from_secs(5))
            .call()
            .ok()?;

        if response.status() == 200 {
            response.into_json::<QuotaResponse>().ok()
        } else {
            None
        }
    }

    #[cfg(not(feature = "quota"))]
    fn fetch_quota_data() -> Option<QuotaResponse> {
        None
    }

    fn format_quota_display(quota: &QuotaSnapshot) -> String {
        if quota.unlimited {
            "∞".to_string()
        } else {
            let remaining = quota.remaining;
            let percent = quota.percent_remaining;

            if remaining >= 1000 {
                let k_value = remaining as f64 / 1000.0;
                if k_value.fract() == 0.0 {
                    format!("{}k ({:.0}%)", k_value as u32, percent)
                } else {
                    format!("{:.1}k ({:.1}%)", k_value, percent)
                }
            } else if percent.fract() == 0.0 {
                format!("{} ({:.0}%)", remaining, percent)
            } else {
                format!("{} ({:.1}%)", remaining, percent)
            }
        }
    }
}

impl Segment for QuotaSegment {
    fn collect(&self, _input: &InputData) -> Option<SegmentData> {
        #[cfg(feature = "quota")]
        {
            // Check if we can get the base URL from settings
            Self::get_anthropic_base_url()?;

            let quota_data = Self::fetch_quota_data()?;

            let premium = &quota_data.quota_snapshots.premium_interactions;
            let chat = &quota_data.quota_snapshots.chat;
            let completions = &quota_data.quota_snapshots.completions;

            // Focus on premium_interactions as it's the most limited resource
            let primary_display = if !premium.unlimited {
                format!("Premium: {}", Self::format_quota_display(premium))
            } else if !chat.unlimited {
                format!("Chat: {}", Self::format_quota_display(chat))
            } else if !completions.unlimited {
                format!("Code: {}", Self::format_quota_display(completions))
            } else {
                "Unlimited".to_string()
            };

            let mut metadata = HashMap::new();
            metadata.insert(
                "premium_remaining".to_string(),
                premium.remaining.to_string(),
            );
            metadata.insert(
                "premium_percent".to_string(),
                premium.percent_remaining.to_string(),
            );
            metadata.insert(
                "premium_unlimited".to_string(),
                premium.unlimited.to_string(),
            );
            metadata.insert("chat_unlimited".to_string(), chat.unlimited.to_string());
            metadata.insert(
                "completions_unlimited".to_string(),
                completions.unlimited.to_string(),
            );
            metadata.insert(
                "reset_date".to_string(),
                quota_data.quota_reset_date.clone(),
            );
            metadata.insert("plan".to_string(), quota_data.copilot_plan.clone());

            // Add the base URL to metadata for debugging
            if let Some(base_url) = Self::get_anthropic_base_url() {
                metadata.insert("base_url".to_string(), base_url);
            }

            Some(SegmentData {
                primary: primary_display,
                secondary: String::new(),
                metadata,
            })
        }
        #[cfg(not(feature = "quota"))]
        {
            // Feature not enabled, don't show quota segment
            None
        }
    }

    fn id(&self) -> SegmentId {
        SegmentId::Quota
    }
}
