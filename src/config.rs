use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AutosaveMode {
    Disabled,
    OnSend,
    Timer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_autosave_mode")]
    pub autosave_mode: AutosaveMode,

    #[serde(default = "default_autosave_interval_seconds")]
    pub autosave_interval_seconds: u64,

    #[serde(default = "default_llm_provider")]
    pub default_llm_provider: String,

    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,

    #[serde(default = "default_ollama_auto_start")]
    pub ollama_auto_start: bool,

    #[serde(default = "default_ollama_model")]
    pub ollama_model: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_api_key: Option<String>,

    #[serde(default = "default_claude_model")]
    pub claude_model: String,
}

fn default_autosave_mode() -> AutosaveMode {
    AutosaveMode::OnSend
}

fn default_autosave_interval_seconds() -> u64 {
    30
}

fn default_llm_provider() -> String {
    "ollama".to_string()
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_ollama_auto_start() -> bool {
    true
}

fn default_ollama_model() -> String {
    "llama2".to_string()
}

fn default_claude_model() -> String {
    "claude-3-5-sonnet-20241022".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            autosave_mode: default_autosave_mode(),
            autosave_interval_seconds: default_autosave_interval_seconds(),
            default_llm_provider: default_llm_provider(),
            ollama_url: default_ollama_url(),
            ollama_auto_start: default_ollama_auto_start(),
            ollama_model: default_ollama_model(),
            claude_api_key: None,
            claude_model: default_claude_model(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::get_config_path()?;

        if config_path.exists() {
            let contents = fs::read_to_string(&config_path)?;
            let config: Config = toml::from_str(&contents)?;
            Ok(config)
        } else {
            // Create default config file
            let config = Config::default();
            config.save()?;
            Ok(config)
        }
    }

    pub fn save(&self) -> Result<()> {
        let config_path = Self::get_config_path()?;
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)?;
        fs::write(config_path, contents)?;
        Ok(())
    }

    fn get_config_path() -> Result<PathBuf> {
        let mut path = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;
        path.push("llm-tui");
        path.push("config.toml");
        Ok(path)
    }
}
