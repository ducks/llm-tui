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

/// Per-provider configuration, tagged by type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderConfig {
    Anthropic {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key_env: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key_cmd: Option<String>,
        #[serde(default = "default_claude_model")]
        model: String,
        #[serde(default = "default_claude_context_window")]
        context_window: i64,
    },
    Openai {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key_env: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key_cmd: Option<String>,
        #[serde(default = "default_openai_model")]
        model: String,
        #[serde(default = "default_openai_context_window")]
        context_window: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
    },
    OpenaiCompatible {
        base_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key_env: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key_cmd: Option<String>,
        model: String,
        #[serde(default = "default_openai_context_window")]
        context_window: i64,
    },
    Gemini {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key_env: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key_cmd: Option<String>,
        #[serde(default = "default_gemini_model")]
        model: String,
        #[serde(default = "default_gemini_context_window")]
        context_window: i64,
    },
    Bedrock {
        #[serde(default = "default_bedrock_model")]
        model: String,
        #[serde(default = "default_bedrock_context_window")]
        context_window: i64,
    },
    Ollama {
        #[serde(default = "default_ollama_url")]
        base_url: String,
        #[serde(default = "default_ollama_model")]
        model: String,
        #[serde(default = "default_ollama_context_window")]
        context_window: i64,
        #[serde(default)]
        auto_start: bool,
    },
}

impl ProviderConfig {
    pub fn model(&self) -> &str {
        match self {
            Self::Anthropic { model, .. }
            | Self::Openai { model, .. }
            | Self::OpenaiCompatible { model, .. }
            | Self::Gemini { model, .. }
            | Self::Bedrock { model, .. }
            | Self::Ollama { model, .. } => model,
        }
    }

    pub fn set_model(&mut self, new_model: String) {
        match self {
            Self::Anthropic { model, .. }
            | Self::Openai { model, .. }
            | Self::OpenaiCompatible { model, .. }
            | Self::Gemini { model, .. }
            | Self::Bedrock { model, .. }
            | Self::Ollama { model, .. } => *model = new_model,
        }
    }

    pub fn context_window(&self) -> i64 {
        match self {
            Self::Anthropic { context_window, .. }
            | Self::Openai { context_window, .. }
            | Self::OpenaiCompatible { context_window, .. }
            | Self::Gemini { context_window, .. }
            | Self::Bedrock { context_window, .. }
            | Self::Ollama { context_window, .. } => *context_window,
        }
    }

    /// Resolve the API key: direct value wins, then command, then env var.
    pub fn resolve_api_key(&self) -> Option<String> {
        let (api_key, api_key_cmd, api_key_env) = match self {
            Self::Anthropic {
                api_key,
                api_key_cmd,
                api_key_env,
                ..
            } => (api_key, api_key_cmd, api_key_env),
            Self::Openai {
                api_key,
                api_key_cmd,
                api_key_env,
                ..
            } => (api_key, api_key_cmd, api_key_env),
            Self::OpenaiCompatible {
                api_key,
                api_key_cmd,
                api_key_env,
                ..
            } => (api_key, api_key_cmd, api_key_env),
            Self::Gemini {
                api_key,
                api_key_cmd,
                api_key_env,
                ..
            } => (api_key, api_key_cmd, api_key_env),
            Self::Bedrock { .. } => return None,
            Self::Ollama { .. } => return None,
        };

        if let Some(key) = api_key {
            if !key.is_empty() {
                return Some(key.clone());
            }
        }

        if let Some(cmd) = api_key_cmd {
            if let Ok(output) = std::process::Command::new("sh").arg("-c").arg(cmd).output() {
                if output.status.success() {
                    let key = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !key.is_empty() {
                        return Some(key);
                    }
                }
            }
        }

        if let Some(env_name) = api_key_env {
            if let Ok(key) = std::env::var(env_name) {
                if !key.is_empty() {
                    return Some(key);
                }
            }
        }

        None
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

fn default_ollama_model() -> String {
    "llama2".to_string()
}

fn default_claude_model() -> String {
    "claude-3-5-sonnet-20241022".to_string()
}

fn default_bedrock_model() -> String {
    "us.anthropic.claude-sonnet-4-20250514-v1:0".to_string()
}

fn default_ollama_context_window() -> i64 {
    4096
}

fn default_claude_context_window() -> i64 {
    200000
}

fn default_bedrock_context_window() -> i64 {
    200000
}

fn default_openai_model() -> String {
    "gpt-4o".to_string()
}

fn default_openai_context_window() -> i64 {
    128000
}

fn default_gemini_model() -> String {
    "gemini-2.5-flash".to_string()
}

fn default_gemini_context_window() -> i64 {
    1000000
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
            base_url: default_ollama_url(),
            model: default_ollama_model(),
            context_window: default_ollama_context_window(),
            auto_start: true,
        },
    );
    providers
}

impl Default for Config {
    fn default() -> Self {
        let mut providers = default_providers();

        // Add Claude if env var is set
        if std::env::var("ANTHROPIC_API_KEY").is_ok() {
            providers.insert(
                "claude".to_string(),
                ProviderConfig::Anthropic {
                    api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
                    api_key: None,
                    api_key_cmd: None,
                    model: default_claude_model(),
                    context_window: default_claude_context_window(),
                },
            );
        }

        // Add Bedrock (uses AWS env creds, always available)
        providers.insert(
            "bedrock".to_string(),
            ProviderConfig::Bedrock {
                model: default_bedrock_model(),
                context_window: default_bedrock_context_window(),
            },
        );

        // Add OpenAI if env var is set
        if std::env::var("OPENAI_API_KEY").is_ok() {
            providers.insert(
                "openai".to_string(),
                ProviderConfig::Openai {
                    api_key_env: Some("OPENAI_API_KEY".to_string()),
                    api_key: None,
                    api_key_cmd: None,
                    model: default_openai_model(),
                    context_window: default_openai_context_window(),
                    base_url: std::env::var("OPENAI_BASE_URL").ok(),
                },
            );
        }

        // Add Gemini if env var is set
        if std::env::var("GEMINI_API_KEY").is_ok() {
            providers.insert(
                "gemini".to_string(),
                ProviderConfig::Gemini {
                    api_key_env: Some("GEMINI_API_KEY".to_string()),
                    api_key: None,
                    api_key_cmd: None,
                    model: default_gemini_model(),
                    context_window: default_gemini_context_window(),
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

        // Ollama (always present)
        providers.insert(
            "ollama".to_string(),
            ProviderConfig::Ollama {
                base_url: legacy.ollama_url,
                model: legacy.ollama_model,
                context_window: legacy.ollama_context_window,
                auto_start: legacy.ollama_auto_start,
            },
        );

        // Claude
        if legacy.claude_api_key.is_some() {
            providers.insert(
                "claude".to_string(),
                ProviderConfig::Anthropic {
                    api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
                    api_key: None,
                    api_key_cmd: None,
                    model: legacy.claude_model,
                    context_window: legacy.claude_context_window,
                },
            );
        }

        // Bedrock (always present)
        providers.insert(
            "bedrock".to_string(),
            ProviderConfig::Bedrock {
                model: legacy.bedrock_model,
                context_window: legacy.bedrock_context_window,
            },
        );

        // OpenAI
        if legacy.openai_api_key.is_some() {
            providers.insert(
                "openai".to_string(),
                ProviderConfig::Openai {
                    api_key_env: Some("OPENAI_API_KEY".to_string()),
                    api_key: None,
                    api_key_cmd: None,
                    model: legacy.openai_model,
                    context_window: legacy.openai_context_window,
                    base_url: legacy.openai_base_url,
                },
            );
        }

        // Gemini
        if legacy.gemini_api_key.is_some() {
            providers.insert(
                "gemini".to_string(),
                ProviderConfig::Gemini {
                    api_key_env: Some("GEMINI_API_KEY".to_string()),
                    api_key: None,
                    api_key_cmd: None,
                    model: legacy.gemini_model,
                    context_window: legacy.gemini_context_window,
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
    #[serde(default = "default_ollama_model")]
    ollama_model: String,
    #[serde(default)]
    claude_api_key: Option<String>,
    #[serde(default = "default_claude_model")]
    claude_model: String,
    #[serde(default = "default_bedrock_model")]
    bedrock_model: String,
    #[serde(default = "default_ollama_context_window")]
    ollama_context_window: i64,
    #[serde(default = "default_claude_context_window")]
    claude_context_window: i64,
    #[serde(default = "default_bedrock_context_window")]
    bedrock_context_window: i64,
    #[serde(default)]
    openai_api_key: Option<String>,
    #[serde(default = "default_openai_model")]
    openai_model: String,
    #[serde(default = "default_openai_context_window")]
    openai_context_window: i64,
    #[serde(default)]
    openai_base_url: Option<String>,
    #[serde(default)]
    gemini_api_key: Option<String>,
    #[serde(default = "default_gemini_model")]
    gemini_model: String,
    #[serde(default = "default_gemini_context_window")]
    gemini_context_window: i64,
    #[serde(default = "default_autocompact_threshold")]
    autocompact_threshold: f64,
    #[serde(default = "default_autocompact_keep_recent")]
    autocompact_keep_recent: usize,
}
