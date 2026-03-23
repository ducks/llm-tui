//! Provider registry for dynamic provider management

use super::{
    BedrockProvider, ClaudeProvider, GeminiProvider, LlmProvider, OllamaProvider, OpenAIProvider,
};
use crate::config::ProviderConfig;
use std::collections::HashMap;

/// Provider registry that holds all available providers
pub struct ProviderRegistry {
    providers: HashMap<String, Box<dyn LlmProvider>>,
}

impl ProviderRegistry {
    /// Create a new empty registry
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Register a provider
    pub fn register(&mut self, name: String, provider: Box<dyn LlmProvider>) {
        self.providers.insert(name, provider);
    }

    /// Get a provider by name
    pub fn get(&self, name: &str) -> Option<&dyn LlmProvider> {
        self.providers.get(name).map(|p| &**p)
    }

    /// Check if a provider is available
    pub fn is_available(&self, name: &str) -> bool {
        self.providers.contains_key(name)
    }

    /// List all available provider names
    #[allow(dead_code)]
    pub fn available_providers(&self) -> Vec<String> {
        self.providers
            .iter()
            .filter(|(_, provider)| provider.is_available())
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Build registry from config
    pub fn from_config(config: &crate::config::Config) -> Self {
        let mut registry = Self::new();

        for (name, provider_config) in &config.providers {
            if let Some(provider) = Self::build_provider(name, provider_config) {
                registry.register(name.clone(), provider);
            }
        }

        registry
    }

    fn build_provider(name: &str, config: &ProviderConfig) -> Option<Box<dyn LlmProvider>> {
        match config {
            ProviderConfig::Anthropic { .. } => {
                let api_key = config.resolve_api_key()?;
                Some(Box::new(ClaudeProvider::new(api_key)))
            }
            ProviderConfig::Openai { base_url, .. } => {
                let api_key = config.resolve_api_key()?;
                if let Some(url) = base_url {
                    Some(Box::new(OpenAIProvider::with_base_url(
                        api_key,
                        url.clone(),
                        name.to_string(),
                    )))
                } else {
                    Some(Box::new(OpenAIProvider::with_base_url(
                        api_key,
                        "https://api.openai.com/v1".to_string(),
                        name.to_string(),
                    )))
                }
            }
            ProviderConfig::OpenaiCompatible { base_url, .. } => {
                let api_key = config.resolve_api_key()?;
                Some(Box::new(OpenAIProvider::with_base_url(
                    api_key,
                    base_url.clone(),
                    name.to_string(),
                )))
            }
            ProviderConfig::Gemini { .. } => {
                let api_key = config.resolve_api_key()?;
                Some(Box::new(GeminiProvider::new(api_key)))
            }
            ProviderConfig::Bedrock { .. } => Some(Box::new(BedrockProvider::new())),
            ProviderConfig::Ollama { base_url, .. } => {
                Some(Box::new(OllamaProvider::new(base_url)))
            }
        }
    }
}
