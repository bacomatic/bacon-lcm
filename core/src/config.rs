// core/src/config.rs
use crate::error::{ConfigError, ConfigResult};
use crate::types::CompactionConfig;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Main LCM configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcmConfig {
    pub summarizer: SummarizerConfig,
    pub embedder: Option<EmbedderConfig>,
    pub tokenizer: Option<TokenizerConfig>,
    pub compaction: CompactionConfig,
    pub database_url: Option<String>,
    pub dashboard: Option<DashboardConfig>,
    pub rust: Option<RustSpecificConfig>,
}

/// Summarizer configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummarizerConfig {
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub max_tokens: Option<usize>,
    pub temperature: Option<f32>,
}

/// Embedder configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedderConfig {
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub dimensions: Option<usize>,
}

/// Tokenizer configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenizerConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
}

/// Dashboard configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardConfig {
    pub enabled: bool,
    pub port: Option<u16>,
}

/// Rust-specific configuration extensions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustSpecificConfig {
    pub max_concurrent_requests: Option<usize>,
    pub request_timeout_ms: Option<u64>,
    pub retry_policy: Option<RetryPolicy>,
    pub parallel_compaction: Option<bool>,
    pub compaction_workers: Option<usize>,
    pub memory_limit_mb: Option<usize>,
}

/// Retry policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryPolicy {
    pub max_retries: usize,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub exponential_base: f64,
}

impl Default for RustSpecificConfig {
    fn default() -> Self {
        Self {
            max_concurrent_requests: Some(10),
            request_timeout_ms: Some(30000),
            retry_policy: Some(RetryPolicy {
                max_retries: 3,
                base_delay_ms: 1000,
                max_delay_ms: 30000,
                exponential_base: 2.0,
            }),
            parallel_compaction: Some(true),
            compaction_workers: Some(4),
            memory_limit_mb: Some(512),
        }
    }
}

impl LcmConfig {
    /// Load configuration from file and environment variables
    pub fn load() -> ConfigResult<Self> {
        // Start with defaults
        let mut config = Self::defaults();

        // Load from config file if it exists
        if let Some(file_config) = Self::load_from_default_paths()? {
            config.merge(file_config);
        }

        // Override with environment variables
        config.merge_env();

        // Validate configuration
        config.validate()?;

        Ok(config)
    }

    /// Load configuration from a specific file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> ConfigResult<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = serde_json::from_str(&content)?;
        Ok(config)
    }

    /// Try to load from default config file locations
    fn load_from_default_paths() -> ConfigResult<Option<Self>> {
        let paths = [
            "bacon-lcm.config.json",
            ".bacon-lcm/config.json",
            "~/.config/bacon-lcm/config.json",
        ];

        for path in paths {
            if let Ok(expanded) = shellexpand::full(path) {
                if std::path::Path::new(expanded.as_ref()).exists() {
                    return Ok(Some(Self::load_from_file(expanded.as_ref())?));
                }
            }
        }

        Ok(None)
    }

    /// Merge another configuration into this one
    fn merge(&mut self, other: Self) {
        // Simple merge strategy - other takes precedence
        if other.summarizer.provider != "echo" || self.summarizer.provider == "echo" {
            self.summarizer = other.summarizer;
        }
        if other.embedder.is_some() {
            self.embedder = other.embedder;
        }
        if other.tokenizer.is_some() {
            self.tokenizer = other.tokenizer;
        }
        if other.database_url.is_some() {
            self.database_url = other.database_url;
        }
        if other.dashboard.is_some() {
            self.dashboard = other.dashboard;
        }
        if other.rust.is_some() {
            self.rust = other.rust;
        }
    }

    /// Override configuration with environment variables
    fn merge_env(&mut self) {
        // Summarizer settings
        if let Ok(provider) = std::env::var("LCM_SUMMARIZER_PROVIDER") {
            self.summarizer.provider = provider;
        }
        if let Ok(model) = std::env::var("LCM_SUMMARIZER_MODEL") {
            self.summarizer.model = model;
        }
        if let Ok(base_url) = std::env::var("LCM_SUMMARIZER_BASE_URL") {
            self.summarizer.base_url = Some(base_url);
        }

        // API keys (check multiple sources)
        if let Ok(api_key) = std::env::var("LCM_API_KEY") {
            self.summarizer.api_key = Some(api_key);
        } else if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
            self.summarizer.api_key = Some(api_key);
        } else if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
            self.summarizer.api_key = Some(api_key);
        }

        // Database
        if let Ok(database_url) = std::env::var("DATABASE_URL") {
            self.database_url = Some(database_url);
        }

        // Dashboard
        if std::env::var("DASHBOARD").is_ok() {
            self.dashboard
                .get_or_insert_with(DashboardConfig::default)
                .enabled = true;
        }
        if let Ok(port) = std::env::var("DASHBOARD_PORT") {
            self.dashboard
                .get_or_insert_with(DashboardConfig::default)
                .port = Some(port.parse().unwrap_or(3333));
        }

        // Compaction thresholds
        if let Ok(max_tokens) = std::env::var("LCM_MODEL_MAX_TOKENS") {
            self.compaction.thresholds.model_max_tokens =
                max_tokens.parse().unwrap_or(128000);
        }
        if let Ok(soft_limit) = std::env::var("LCM_SOFT_LIMIT") {
            self.compaction.thresholds.soft_limit = soft_limit.parse().unwrap_or(80000);
        }
        if let Ok(hard_limit) = std::env::var("LCM_HARD_LIMIT") {
            self.compaction.thresholds.hard_limit =
                hard_limit.parse().unwrap_or(110000);
        }
        if let Ok(fresh_tail) = std::env::var("LCM_FRESH_TAIL_COUNT") {
            self.compaction.fresh_tail_count = fresh_tail.parse().unwrap_or(10);
        }

        // Rust-specific settings
        if let Ok(max_requests) = std::env::var("LCM_RUST_MAX_CONCURRENT_REQUESTS") {
            self.rust
                .get_or_insert_with(RustSpecificConfig::default)
                .max_concurrent_requests = Some(max_requests.parse().unwrap_or(10));
        }
        if let Ok(workers) = std::env::var("LCM_RUST_COMPACTION_WORKERS") {
            self.rust
                .get_or_insert_with(RustSpecificConfig::default)
                .compaction_workers = Some(workers.parse().unwrap_or(4));
        }
    }

    /// Validate configuration
    fn validate(&self) -> ConfigResult<()> {
        // Validate summarizer
        if self.summarizer.provider.is_empty() {
            return Err(ConfigError::MissingRequired(
                "summarizer.provider".to_string(),
            ));
        }
        if self.summarizer.model.is_empty() {
            return Err(ConfigError::MissingRequired(
                "summarizer.model".to_string(),
            ));
        }

        // Validate compaction thresholds
        if self.compaction.thresholds.soft_limit >= self.compaction.thresholds.hard_limit {
            return Err(ConfigError::InvalidValue(
                "soft_limit must be less than hard_limit".to_string(),
            ));
        }
        if self.compaction.thresholds.hard_limit
            > self.compaction.thresholds.model_max_tokens
        {
            return Err(ConfigError::InvalidValue(
                "hard_limit must be less than model_max_tokens".to_string(),
            ));
        }

        Ok(())
    }

    /// Get default configuration
    fn defaults() -> Self {
        Self {
            summarizer: SummarizerConfig {
                provider: "echo".to_string(),
                model: "echo".to_string(),
                base_url: None,
                api_key: None,
                max_tokens: Some(1024),
                temperature: Some(0.3),
            },
            embedder: None,
            tokenizer: None,
            compaction: CompactionConfig::default(),
            database_url: None,
            dashboard: None,
            rust: Some(RustSpecificConfig::default()),
        }
    }
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: Some(3333),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_example_json_parses() {
        // Locate the example config file relative to the workspace root
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let example_path =
            std::path::Path::new(manifest_dir).join("../bacon-lcm.config.example.json");

        let config = LcmConfig::load_from_file(&example_path).expect(
            "bacon-lcm.config.example.json should parse successfully with camelCase serde rename",
        );

        // Validate key fields were deserialized correctly
        assert_eq!(config.summarizer.provider, "openai");
        assert_eq!(config.summarizer.model, "gpt-4o-mini");
        assert_eq!(
            config.summarizer.base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(config.summarizer.max_tokens, Some(1024));

        let embedder = config.embedder.expect("embedder should be present");
        assert_eq!(embedder.provider, "openai");
        assert_eq!(embedder.model, "text-embedding-3-small");
        assert_eq!(embedder.dimensions, Some(1536));

        let tokenizer = config.tokenizer.expect("tokenizer should be present");
        assert_eq!(tokenizer.provider.as_deref(), Some("tiktoken"));
        assert_eq!(tokenizer.model.as_deref(), Some("gpt-4o"));

        assert_eq!(config.compaction.thresholds.model_max_tokens, 128000);
        assert_eq!(config.compaction.thresholds.soft_limit, 80000);
        assert_eq!(config.compaction.thresholds.hard_limit, 110000);
        assert_eq!(config.compaction.fresh_tail_count, 10);
        assert_eq!(config.compaction.leaf_group_size, 20);
        assert_eq!(config.compaction.condensed_group_size, 10);
        assert!(config.compaction.parallel_compaction);
        assert_eq!(config.compaction.max_concurrent_compactions, 4);

        assert_eq!(
            config.database_url.as_deref(),
            Some("postgres://localhost:5432/bacon_lcm")
        );

        let dashboard = config.dashboard.expect("dashboard should be present");
        assert!(dashboard.enabled);
        assert_eq!(dashboard.port, Some(3333));

        let rust = config.rust.expect("rust should be present");
        assert_eq!(rust.max_concurrent_requests, Some(10));
        assert_eq!(rust.request_timeout_ms, Some(30000));
        assert_eq!(rust.parallel_compaction, Some(true));
        assert_eq!(rust.compaction_workers, Some(4));
        assert_eq!(rust.memory_limit_mb, Some(512));

        let retry = rust.retry_policy.expect("retryPolicy should be present");
        assert_eq!(retry.max_retries, 3);
        assert_eq!(retry.base_delay_ms, 1000);
        assert_eq!(retry.max_delay_ms, 30000);
        assert_eq!(retry.exponential_base, 2.0);
    }

    #[test]
    fn test_default_config() {
        let config = LcmConfig::defaults();
        assert_eq!(config.summarizer.provider, "echo");
        assert_eq!(config.compaction.thresholds.model_max_tokens, 128000);
        assert!(config.rust.is_some());
    }

    #[test]
    fn test_config_validation() {
        let mut config = LcmConfig::defaults();

        // Valid config should pass
        assert!(config.validate().is_ok());

        // Invalid thresholds should fail
        config.compaction.thresholds.soft_limit = 90000;
        config.compaction.thresholds.hard_limit = 80000;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_env_override() {
        env::set_var("LCM_SUMMARIZER_PROVIDER", "openai");
        env::set_var("LCM_SUMMARIZER_MODEL", "gpt-4");

        let mut config = LcmConfig::defaults();
        config.merge_env();

        assert_eq!(config.summarizer.provider, "openai");
        assert_eq!(config.summarizer.model, "gpt-4");

        env::remove_var("LCM_SUMMARIZER_PROVIDER");
        env::remove_var("LCM_SUMMARIZER_MODEL");
    }
}
