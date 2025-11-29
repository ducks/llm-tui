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

impl Session {
    pub fn new(name: Option<String>, project: Option<String>, model: Option<String>) -> Self {
        let now = Utc::now();
        let id = now.format("%Y%m%d-%H%M%S").to_string();

        Self {
            id: id.clone(),
            name,
            project,
            created_at: now,
            updated_at: now,
            llm_provider: "ollama".to_string(),
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

    pub fn add_message_with_flag(&mut self, role: String, content: String, model: Option<String>, tools_executed: bool) {
        self.messages.push(Message {
            role,
            content,
            timestamp: Utc::now(),
            model,
            tools_executed,
        });
        self.updated_at = Utc::now();
    }
}
