use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub model: Option<String>,
    #[serde(default)]
    pub tools_executed: bool,
    #[serde(default)]
    pub is_summary: bool,
    #[serde(default)]
    pub token_count: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: Option<String>,
    pub project: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub llm_provider: String,
    pub model: Option<String>,
    pub messages: Vec<Message>,
}

/// Estimate token count for text (rough approximation: 1 token ≈ 4 characters)
pub fn estimate_tokens(text: &str) -> i64 {
    (text.len() as f64 / 4.0).ceil() as i64
}

impl Session {
    pub fn new(
        name: Option<String>,
        project: Option<String>,
        provider: String,
        model: Option<String>,
    ) -> Self {
        let now = Utc::now();
        let id = now.format("%Y%m%d-%H%M%S").to_string();

        Self {
            id: id.clone(),
            name,
            project,
            created_at: now,
            updated_at: now,
            llm_provider: provider,
            model,
            messages: Vec::new(),
        }
    }

    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.id.clone())
    }

    pub fn add_message(&mut self, role: String, content: String, model: Option<String>) {
        self.add_message_with_flag(role, content, model, false);
    }

    pub fn add_message_with_flag(
        &mut self,
        role: String,
        content: String,
        model: Option<String>,
        tools_executed: bool,
    ) {
        self.add_message_full(role, content, model, tools_executed, false, None);
    }

    pub fn add_message_full(
        &mut self,
        role: String,
        content: String,
        model: Option<String>,
        tools_executed: bool,
        is_summary: bool,
        token_count: Option<i64>,
    ) {
        // Auto-calculate token count if not provided
        let final_token_count = token_count.or_else(|| Some(estimate_tokens(&content)));

        self.messages.push(Message {
            role,
            content,
            timestamp: Utc::now(),
            model,
            tools_executed,
            is_summary,
            token_count: final_token_count,
        });
        self.updated_at = Utc::now();
    }

    /// Calculate total tokens in non-summary messages
    pub fn total_tokens(&self) -> i64 {
        self.messages
            .iter()
            .filter(|m| !m.is_summary)
            .map(|m| m.token_count.unwrap_or(0))
            .sum()
    }

    /// Check if autocompact should be triggered
    pub fn should_autocompact(&self, context_window: i64, threshold: f64) -> bool {
        let total = self.total_tokens();
        let limit = (context_window as f64 * threshold) as i64;
        total > limit
    }

    /// Get indices of messages to compact (all non-summary messages except last N)
    pub fn get_compactable_range(&self, keep_recent: usize) -> Option<(usize, usize)> {
        // Find all non-summary, non-tools_executed message indices
        let compactable_indices: Vec<usize> = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.is_summary && !m.tools_executed)
            .map(|(i, _)| i)
            .collect();

        if compactable_indices.len() <= keep_recent {
            return None; // Not enough messages to compact
        }

        // Compact all but the last keep_recent
        let end_idx = compactable_indices.len() - keep_recent;
        Some((compactable_indices[0], compactable_indices[end_idx - 1]))
    }
}
