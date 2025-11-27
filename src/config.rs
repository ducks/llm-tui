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
}

fn default_autosave_mode() -> AutosaveMode {
    AutosaveMode::OnSend
}

fn default_autosave_interval_seconds() -> u64 {
    30
}

fn default_llm_provider() -> String {
    "none".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            autosave_mode: default_autosave_mode(),
            autosave_interval_seconds: default_autosave_interval_seconds(),
            default_llm_provider: default_llm_provider(),
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
