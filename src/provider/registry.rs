//! Provider registry for dynamic provider management

use super::{
    BedrockProvider, ClaudeProvider, GeminiProvider, LlmProvider, OllamaProvider, OpenAIProvider,
};
use std::collections::HashMap;

/// Provider registry that holds all available providers
pub struct ProviderRegistry {
    providers: HashMap<String, Box<dyn LlmProvider>>,
}

impl ProviderRegistry {
    /// Create a new empty registry
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

        // Always register Ollama (works locally without API key)
        registry.register(
            "ollama".to_string(),
            Box::new(OllamaProvider::new(&config.ollama_url)),
        );

        // Register Claude if API key is present
        if let Some(ref api_key) = config.claude_api_key {
            if !api_key.is_empty() {
                registry.register(
                    "claude".to_string(),
                    Box::new(ClaudeProvider::new(api_key.clone())),
                );
            }
        }

        // Always register Bedrock (uses AWS credentials from environment)
        registry.register("bedrock".to_string(), Box::new(BedrockProvider::new()));

        // Register OpenAI if API key is present
        if let Some(ref api_key) = config.openai_api_key {
            if !api_key.is_empty() {
                registry.register(
                    "openai".to_string(),
                    Box::new(OpenAIProvider::new(api_key.clone())),
                );
            }
        }

        // Register Gemini if API key is present
        if let Some(ref api_key) = config.gemini_api_key {
            if !api_key.is_empty() {
                registry.register(
                    "gemini".to_string(),
                    Box::new(GeminiProvider::new(api_key.clone())),
                );
            }
        }

        registry
    }
}
