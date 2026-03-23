use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AutosaveMode {
    Disabled,
    OnSend,
    Timer,
}

/// Common fields shared by all provider types.
/// Flattened into each variant so TOML stays flat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCommon {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_context_window")]
    pub context_window: i64,
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u32,
}

impl Default for ProviderCommon {
    fn default() -> Self {
        Self {
            model: default_model(),
            context_window: default_context_window(),
            max_output_tokens: default_max_output_tokens(),
        }
    }
}

/// API key configuration, shared by providers that need authentication.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiKeyConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_cmd: Option<String>,
}

impl ApiKeyConfig {
    pub fn from_env(env_var: &str) -> Self {
        Self {
            api_key_env: Some(env_var.to_string()),
            api_key: None,
            api_key_cmd: None,
        }
    }

    /// Resolve the API key: direct value wins, then command, then env var.
    pub fn resolve(&self) -> Option<String> {
        if let Some(key) = &self.api_key {
            if !key.is_empty() {
                return Some(key.clone());
            }
        }

        if let Some(cmd) = &self.api_key_cmd {
            if let Ok(output) = std::process::Command::new("sh").arg("-c").arg(cmd).output() {
                if output.status.success() {
                    let key = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !key.is_empty() {
                        return Some(key);
                    }
                }
            }
        }

        if let Some(env_name) = &self.api_key_env {
            if let Ok(key) = std::env::var(env_name) {
                if !key.is_empty() {
                    return Some(key);
                }
            }
        }

        None
    }

    pub fn source_description(&self) -> String {
        if self.api_key.as_ref().is_some_and(|k| !k.is_empty()) {
            "direct".to_string()
        } else if let Some(cmd) = &self.api_key_cmd {
            format!("cmd: {}", cmd)
        } else if let Some(env) = &self.api_key_env {
            format!("env: {}", env)
        } else {
            "none".to_string()
        }
    }
}

/// Per-provider configuration, tagged by type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderConfig {
    Anthropic {
        #[serde(flatten)]
        common: ProviderCommon,
        #[serde(flatten)]
        auth: ApiKeyConfig,
    },
    Openai {
        #[serde(flatten)]
        common: ProviderCommon,
        #[serde(flatten)]
        auth: ApiKeyConfig,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
    },
    OpenaiCompatible {
        #[serde(flatten)]
        common: ProviderCommon,
        #[serde(flatten)]
        auth: ApiKeyConfig,
        base_url: String,
    },
    Gemini {
        #[serde(flatten)]
        common: ProviderCommon,
        #[serde(flatten)]
        auth: ApiKeyConfig,
    },
    Bedrock {
        #[serde(flatten)]
        common: ProviderCommon,
    },
    Ollama {
        #[serde(flatten)]
        common: ProviderCommon,
        #[serde(default = "default_ollama_url")]
        base_url: String,
        #[serde(default)]
        auto_start: bool,
    },
}

impl ProviderConfig {
    fn common(&self) -> &ProviderCommon {
        match self {
            Self::Anthropic { common, .. }
            | Self::Openai { common, .. }
            | Self::OpenaiCompatible { common, .. }
            | Self::Gemini { common, .. }
            | Self::Bedrock { common, .. }
            | Self::Ollama { common, .. } => common,
        }
    }

    fn common_mut(&mut self) -> &mut ProviderCommon {
        match self {
            Self::Anthropic { common, .. }
            | Self::Openai { common, .. }
            | Self::OpenaiCompatible { common, .. }
            | Self::Gemini { common, .. }
            | Self::Bedrock { common, .. }
            | Self::Ollama { common, .. } => common,
        }
    }

    pub fn provider_type_name(&self) -> &str {
        match self {
            Self::Anthropic { .. } => "anthropic",
            Self::Openai { .. } => "openai",
            Self::OpenaiCompatible { .. } => "openai_compatible",
            Self::Gemini { .. } => "gemini",
            Self::Bedrock { .. } => "bedrock",
            Self::Ollama { .. } => "ollama",
        }
    }

    pub fn model(&self) -> &str {
        &self.common().model
    }

    pub fn set_model(&mut self, new_model: String) {
        self.common_mut().model = new_model;
    }

    pub fn context_window(&self) -> i64 {
        self.common().context_window
    }

    pub fn max_output_tokens(&self) -> u32 {
        self.common().max_output_tokens
    }

    pub fn base_url(&self) -> Option<&str> {
        match self {
            Self::Openai { base_url, .. } => base_url.as_deref(),
            Self::OpenaiCompatible { base_url, .. } => Some(base_url.as_str()),
            Self::Ollama { base_url, .. } => Some(base_url.as_str()),
            _ => None,
        }
    }

    pub fn key_source_description(&self) -> String {
        match self {
            Self::Anthropic { auth, .. }
            | Self::Openai { auth, .. }
            | Self::OpenaiCompatible { auth, .. }
            | Self::Gemini { auth, .. } => auth.source_description(),
            Self::Bedrock { .. } => "AWS credentials".to_string(),
            Self::Ollama { .. } => "none (local)".to_string(),
        }
    }

    pub fn resolve_api_key(&self) -> Option<String> {
        match self {
            Self::Anthropic { auth, .. }
            | Self::Openai { auth, .. }
            | Self::OpenaiCompatible { auth, .. }
            | Self::Gemini { auth, .. } => auth.resolve(),
            Self::Bedrock { .. } | Self::Ollama { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_autosave_mode")]
    pub autosave_mode: AutosaveMode,

    #[serde(default = "default_autosave_interval_seconds")]
    pub autosave_interval_seconds: u64,

    #[serde(default = "default_provider_name")]
    pub default_provider: String,

    #[serde(default = "default_providers")]
    pub providers: HashMap<String, ProviderConfig>,

    #[serde(default = "default_autocompact_threshold")]
    pub autocompact_threshold: f64,

    #[serde(default = "default_autocompact_keep_recent")]
    pub autocompact_keep_recent: usize,
}

// Default functions

fn default_autosave_mode() -> AutosaveMode {
    AutosaveMode::OnSend
}

fn default_autosave_interval_seconds() -> u64 {
    30
}

fn default_provider_name() -> String {
    "ollama".to_string()
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_model() -> String {
    "unknown".to_string()
}

fn default_context_window() -> i64 {
    4096
}

fn default_max_output_tokens() -> u32 {
    4096
}

fn default_autocompact_threshold() -> f64 {
    0.75
}

fn default_autocompact_keep_recent() -> usize {
    10
}

fn default_providers() -> HashMap<String, ProviderConfig> {
    let mut providers = HashMap::new();
    providers.insert(
        "ollama".to_string(),
        ProviderConfig::Ollama {
            common: ProviderCommon {
                model: "llama2".to_string(),
                context_window: 4096,
                max_output_tokens: 4096,
            },
            base_url: default_ollama_url(),
            auto_start: true,
        },
    );
    providers
}

impl Default for Config {
    fn default() -> Self {
        let mut providers = default_providers();

        if std::env::var("ANTHROPIC_API_KEY").is_ok() {
            providers.insert(
                "claude".to_string(),
                ProviderConfig::Anthropic {
                    common: ProviderCommon {
                        model: "claude-3-5-sonnet-20241022".to_string(),
                        context_window: 200000,
                        max_output_tokens: 8192,
                    },
                    auth: ApiKeyConfig::from_env("ANTHROPIC_API_KEY"),
                },
            );
        }

        providers.insert(
            "bedrock".to_string(),
            ProviderConfig::Bedrock {
                common: ProviderCommon {
                    model: "us.anthropic.claude-sonnet-4-20250514-v1:0".to_string(),
                    context_window: 200000,
                    max_output_tokens: 8192,
                },
            },
        );

        if std::env::var("OPENAI_API_KEY").is_ok() {
            providers.insert(
                "openai".to_string(),
                ProviderConfig::Openai {
                    common: ProviderCommon {
                        model: "gpt-4o".to_string(),
                        context_window: 128000,
                        max_output_tokens: 16384,
                    },
                    auth: ApiKeyConfig::from_env("OPENAI_API_KEY"),
                    base_url: std::env::var("OPENAI_BASE_URL").ok(),
                },
            );
        }

        if std::env::var("GEMINI_API_KEY").is_ok() {
            providers.insert(
                "gemini".to_string(),
                ProviderConfig::Gemini {
                    common: ProviderCommon {
                        model: "gemini-2.5-flash".to_string(),
                        context_window: 1000000,
                        max_output_tokens: 8192,
                    },
                    auth: ApiKeyConfig::from_env("GEMINI_API_KEY"),
                },
            );
        }

        Self {
            autosave_mode: default_autosave_mode(),
            autosave_interval_seconds: default_autosave_interval_seconds(),
            default_provider: default_provider_name(),
            providers,
            autocompact_threshold: default_autocompact_threshold(),
            autocompact_keep_recent: default_autocompact_keep_recent(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::get_config_path()?;

        if !config_path.exists() {
            let config = Config::default();
            config.save()?;
            return Ok(config);
        }

        let contents = fs::read_to_string(&config_path)?;

        // Check if the TOML actually has a [providers] table.
        // We can't just try deserializing as Config because serde defaults
        // will populate the providers map even for legacy configs.
        let raw: toml::Value = toml::from_str(&contents)?;
        let has_providers_table = raw
            .get("providers")
            .and_then(|v| v.as_table())
            .is_some_and(|t| !t.is_empty());

        if has_providers_table {
            if let Ok(config) = toml::from_str::<Config>(&contents) {
                return Ok(config);
            }
        }

        // Fall back to legacy format
        if let Ok(legacy) = toml::from_str::<LegacyConfig>(&contents) {
            let config = Config::from_legacy(legacy);
            config.save()?;
            return Ok(config);
        }

        // If neither works, return default
        let config = Config::default();
        config.save()?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let config_path = Self::get_config_path()?;
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)?;
        fs::write(&config_path, contents)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&config_path, perms)?;
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub fn provider(&self, name: &str) -> Option<&ProviderConfig> {
        self.providers.get(name)
    }

    pub fn model_for_provider(&self, name: &str) -> String {
        self.providers
            .get(name)
            .map(|p| p.model().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    }

    pub fn context_window_for_provider(&self, name: &str) -> i64 {
        self.providers
            .get(name)
            .map(|p| p.context_window())
            .unwrap_or(4096)
    }

    pub fn max_output_tokens_for_provider(&self, name: &str) -> u32 {
        self.providers
            .get(name)
            .map(|p| p.max_output_tokens())
            .unwrap_or(4096)
    }

    pub fn set_model_for_provider(&mut self, name: &str, model: String) {
        if let Some(p) = self.providers.get_mut(name) {
            p.set_model(model);
        }
    }

    /// Find the first ollama-type provider config (for management ops).
    pub fn ollama_config(&self) -> Option<(&str, &ProviderConfig)> {
        self.providers
            .iter()
            .find(|(_, p)| matches!(p, ProviderConfig::Ollama { .. }))
            .map(|(n, p)| (n.as_str(), p))
    }

    fn get_config_path() -> Result<PathBuf> {
        let mut path =
            dirs::config_dir().ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;
        path.push("llm-tui");
        path.push("config.toml");
        Ok(path)
    }

    fn from_legacy(legacy: LegacyConfig) -> Self {
        let mut providers = HashMap::new();

        providers.insert(
            "ollama".to_string(),
            ProviderConfig::Ollama {
                common: ProviderCommon {
                    model: legacy.ollama_model,
                    context_window: legacy.ollama_context_window,
                    ..Default::default()
                },
                base_url: legacy.ollama_url,
                auto_start: legacy.ollama_auto_start,
            },
        );

        if legacy.claude_api_key.is_some() {
            providers.insert(
                "claude".to_string(),
                ProviderConfig::Anthropic {
                    common: ProviderCommon {
                        model: legacy.claude_model,
                        context_window: legacy.claude_context_window,
                        ..Default::default()
                    },
                    auth: ApiKeyConfig::from_env("ANTHROPIC_API_KEY"),
                },
            );
        }

        providers.insert(
            "bedrock".to_string(),
            ProviderConfig::Bedrock {
                common: ProviderCommon {
                    model: legacy.bedrock_model,
                    context_window: legacy.bedrock_context_window,
                    ..Default::default()
                },
            },
        );

        if legacy.openai_api_key.is_some() {
            providers.insert(
                "openai".to_string(),
                ProviderConfig::Openai {
                    common: ProviderCommon {
                        model: legacy.openai_model,
                        context_window: legacy.openai_context_window,
                        ..Default::default()
                    },
                    auth: ApiKeyConfig::from_env("OPENAI_API_KEY"),
                    base_url: legacy.openai_base_url,
                },
            );
        }

        if legacy.gemini_api_key.is_some() {
            providers.insert(
                "gemini".to_string(),
                ProviderConfig::Gemini {
                    common: ProviderCommon {
                        model: legacy.gemini_model,
                        context_window: legacy.gemini_context_window,
                        ..Default::default()
                    },
                    auth: ApiKeyConfig::from_env("GEMINI_API_KEY"),
                },
            );
        }

        Config {
            autosave_mode: legacy.autosave_mode,
            autosave_interval_seconds: legacy.autosave_interval_seconds,
            default_provider: legacy.default_llm_provider,
            providers,
            autocompact_threshold: legacy.autocompact_threshold,
            autocompact_keep_recent: legacy.autocompact_keep_recent,
        }
    }
}

// Legacy config format for migration
#[derive(Debug, Deserialize)]
struct LegacyConfig {
    #[serde(default = "default_autosave_mode")]
    autosave_mode: AutosaveMode,
    #[serde(default = "default_autosave_interval_seconds")]
    autosave_interval_seconds: u64,
    #[serde(default = "default_provider_name")]
    default_llm_provider: String,
    #[serde(default = "default_ollama_url")]
    ollama_url: String,
    #[serde(default)]
    ollama_auto_start: bool,
    #[serde(default)]
    ollama_model: String,
    #[serde(default)]
    claude_api_key: Option<String>,
    #[serde(default)]
    claude_model: String,
    #[serde(default)]
    bedrock_model: String,
    #[serde(default = "default_context_window")]
    ollama_context_window: i64,
    #[serde(default = "default_context_window")]
    claude_context_window: i64,
    #[serde(default = "default_context_window")]
    bedrock_context_window: i64,
    #[serde(default)]
    openai_api_key: Option<String>,
    #[serde(default)]
    openai_model: String,
    #[serde(default = "default_context_window")]
    openai_context_window: i64,
    #[serde(default)]
    openai_base_url: Option<String>,
    #[serde(default)]
    gemini_api_key: Option<String>,
    #[serde(default)]
    gemini_model: String,
    #[serde(default = "default_context_window")]
    gemini_context_window: i64,
    #[serde(default = "default_autocompact_threshold")]
    autocompact_threshold: f64,
    #[serde(default = "default_autocompact_keep_recent")]
    autocompact_keep_recent: usize,
}
