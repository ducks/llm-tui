use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: Option<String>,
    pub project: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub llm_provider: String,
    pub messages: Vec<Message>,
}

impl Session {
    pub fn new(name: Option<String>, project: Option<String>) -> Self {
        let now = Utc::now();
        let id = now.format("%Y%m%d-%H%M%S").to_string();

        Self {
            id: id.clone(),
            name,
            project,
            created_at: now,
            updated_at: now,
            llm_provider: "none".to_string(),
            messages: Vec::new(),
        }
    }

    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.id.clone())
    }

    pub fn save(&self) -> Result<()> {
        let path = self.get_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load(project: Option<&str>, session_id: &str) -> Result<Self> {
        let mut path = get_sessions_dir()?;
        if let Some(proj) = project {
            path.push(proj);
        }
        path.push(format!("{}.json", session_id));

        let json = fs::read_to_string(path)?;
        let session: Session = serde_json::from_str(&json)?;
        Ok(session)
    }

    fn get_path(&self) -> Result<PathBuf> {
        let mut path = get_sessions_dir()?;
        if let Some(ref proj) = self.project {
            path.push(proj);
        }
        path.push(format!("{}.json", self.id));
        Ok(path)
    }
}

pub fn get_sessions_dir() -> Result<PathBuf> {
    let mut path = dirs::data_local_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find local data directory"))?;
    path.push("llm-tui");
    path.push("sessions");
    Ok(path)
}

pub fn list_sessions() -> Result<Vec<Session>> {
    let base_dir = get_sessions_dir()?;
    if !base_dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();

    // Walk through all subdirectories (projects) and root
    for entry in walkdir::WalkDir::new(&base_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
    {
        if let Ok(json) = fs::read_to_string(entry.path()) {
            if let Ok(session) = serde_json::from_str::<Session>(&json) {
                sessions.push(session);
            }
        }
    }

    // Sort by updated_at descending (most recent first)
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(sessions)
}
