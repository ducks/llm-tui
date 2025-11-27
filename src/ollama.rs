use anyhow::Result;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModel {
    pub name: String,
    pub modified_at: String,
    pub size: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    message: Option<ChatMessage>,
    done: bool,
}

#[derive(Debug, Serialize)]
struct PullRequest {
    name: String,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct PullResponse {
    status: String,
    #[serde(default)]
    completed: Option<i64>,
    #[serde(default)]
    total: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    models: Vec<OllamaModel>,
}

pub enum LlmEvent {
    Token(String),
    Done,
    Error(String),
}

pub struct OllamaClient {
    base_url: String,
    client: Client,
    process: Option<Child>,
}

impl OllamaClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: Client::new(),
            process: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.client
            .get(&format!("{}/api/tags", self.base_url))
            .timeout(Duration::from_secs(2))
            .send()
            .is_ok()
    }

    pub fn start_server(&mut self) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }

        let child = Command::new("ollama")
            .arg("serve")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        self.process = Some(child);

        // Wait for server to be ready
        for _ in 0..30 {
            thread::sleep(Duration::from_millis(100));
            if self.is_running() {
                return Ok(());
            }
        }

        Err(anyhow::anyhow!("Ollama server failed to start"))
    }

    pub fn list_models(&self) -> Result<Vec<OllamaModel>> {
        let response: ModelsResponse = self
            .client
            .get(&format!("{}/api/tags", self.base_url))
            .send()?
            .json()?;
        Ok(response.models)
    }

    pub fn pull_model(&self, name: &str) -> Result<Receiver<String>> {
        let (tx, rx) = channel();
        let client = self.client.clone();
        let url = format!("{}/api/pull", self.base_url);
        let name = name.to_string();

        thread::spawn(move || {
            let request = PullRequest {
                name,
                stream: true,
            };

            let response = match client.post(&url).json(&request).send() {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(format!("Error: {}", e));
                    return;
                }
            };

            let reader = BufReader::new(response);
            for line in reader.lines() {
                if let Ok(line) = line {
                    if let Ok(response) = serde_json::from_str::<PullResponse>(&line) {
                        let status = if let (Some(completed), Some(total)) =
                            (response.completed, response.total)
                        {
                            format!(
                                "{}: {:.1}%",
                                response.status,
                                (completed as f64 / total as f64) * 100.0
                            )
                        } else {
                            response.status
                        };
                        if tx.send(status).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }

    pub fn delete_model(&self, name: &str) -> Result<()> {
        #[derive(Serialize)]
        struct DeleteRequest {
            name: String,
        }

        self.client
            .delete(&format!("{}/api/delete", self.base_url))
            .json(&DeleteRequest {
                name: name.to_string(),
            })
            .send()?;
        Ok(())
    }

    pub fn chat(&self, model: &str, messages: Vec<ChatMessage>) -> Result<Receiver<LlmEvent>> {
        let (tx, rx) = channel();
        let client = self.client.clone();
        let url = format!("{}/api/chat", self.base_url);
        let request = ChatRequest {
            model: model.to_string(),
            messages,
            stream: true,
        };

        thread::spawn(move || {
            let response = match client.post(&url).json(&request).send() {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(LlmEvent::Error(format!("Request failed: {}", e)));
                    return;
                }
            };

            let reader = BufReader::new(response);
            for line in reader.lines() {
                if let Ok(line) = line {
                    match serde_json::from_str::<ChatResponse>(&line) {
                        Ok(response) => {
                            if response.done {
                                let _ = tx.send(LlmEvent::Done);
                                break;
                            } else if let Some(message) = response.message {
                                if !message.content.is_empty() {
                                    if tx.send(LlmEvent::Token(message.content)).is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(LlmEvent::Error(format!("Parse error: {}", e)));
                            break;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

impl Drop for OllamaClient {
    fn drop(&mut self) {
        // Don't kill the Ollama server on drop - let it keep running
        // for other applications
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
        }
    }
}
