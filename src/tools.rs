use anyhow::{anyhow, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::fs;
use std::path::Path;

// Helper to deserialize string booleans
fn deserialize_bool_flexible<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;

    let value: serde_json::Value = Deserialize::deserialize(deserializer)?;
    match value {
        serde_json::Value::Bool(b) => Ok(b),
        serde_json::Value::String(s) => match s.to_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(true),
            "false" | "0" | "no" => Ok(false),
            _ => Err(Error::custom(format!("invalid boolean string: {}", s))),
        },
        serde_json::Value::Number(n) => Ok(n.as_i64().unwrap_or(0) != 0),
        _ => Err(Error::custom("expected boolean or string")),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadParams {
    pub file_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteParams {
    pub file_path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditParams {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    #[serde(default, deserialize_with = "deserialize_bool_flexible")]
    pub replace_all: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobParams {
    pub pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrepParams {
    pub pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_mode: Option<String>, // "content", "files_with_matches", "count"
}

pub struct Tools {
    read_files: Vec<String>, // Track which files have been read (for safety)
}

impl Tools {
    pub fn new() -> Self {
        Self {
            read_files: Vec::new(),
        }
    }

    /// Read a file with line numbers (cat -n format)
    pub fn read(&mut self, params: ReadParams) -> Result<String> {
        let path = Path::new(&params.file_path);

        // Safety check: ensure path is within home directory
        let path_abs = path.canonicalize()
            .or_else(|_| std::env::current_dir().map(|cwd| cwd.join(path)))?;
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .map_err(|_| anyhow!("HOME environment variable not set"))?;

        if !path_abs.starts_with(&home) {
            return Err(anyhow!("Access denied: can only read files within home directory ({})", home.display()));
        }

        if !path.exists() {
            return Err(anyhow!("File does not exist: {}", params.file_path));
        }

        if !path.is_file() {
            return Err(anyhow!("Path is not a file: {}", params.file_path));
        }

        let content = fs::read_to_string(path)?;

        // Track that this file was read (for Edit/Write safety)
        if !self.read_files.contains(&params.file_path) {
            self.read_files.push(params.file_path.clone());
        }

        let lines: Vec<&str> = content.lines().collect();

        let start = params.offset.unwrap_or(1).saturating_sub(1);
        let end = if let Some(limit) = params.limit {
            (start + limit).min(lines.len())
        } else {
            lines.len()
        };

        let mut result = String::new();
        for (i, line) in lines[start..end].iter().enumerate() {
            let line_num = start + i + 1;
            result.push_str(&format!("{:6}→{}\n", line_num, line));
        }

        Ok(result)
    }

    /// Write content to a file
    pub fn write(&self, params: WriteParams) -> Result<String> {
        let path = Path::new(&params.file_path);

        // Safety check: ensure path is within home directory
        let path_abs = std::env::current_dir()?.join(path);
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .map_err(|_| anyhow!("HOME environment variable not set"))?;

        if !path_abs.starts_with(&home) {
            return Err(anyhow!("Access denied: can only write files within home directory ({})", home.display()));
        }

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(path, &params.content)?;

        Ok(format!("File created successfully at: {}", params.file_path))
    }

    /// Edit a file by replacing old_string with new_string
    pub fn edit(&self, params: EditParams) -> Result<String> {
        let path = Path::new(&params.file_path);

        // Safety check: ensure path is within home directory
        let path_abs = path.canonicalize()
            .or_else(|_| std::env::current_dir().map(|cwd| cwd.join(path)))?;
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .map_err(|_| anyhow!("HOME environment variable not set"))?;

        if !path_abs.starts_with(&home) {
            return Err(anyhow!("Access denied: can only edit files within home directory ({})", home.display()));
        }

        if !path.exists() {
            return Err(anyhow!("File does not exist: {}", params.file_path));
        }

        let content = fs::read_to_string(path)?;

        // Count occurrences of old_string
        let count = content.matches(&params.old_string).count();

        if count == 0 {
            return Err(anyhow!(
                "old_string not found in file: '{}'",
                params.old_string
            ));
        }

        if count > 1 && !params.replace_all {
            return Err(anyhow!(
                "old_string appears {} times in file. Use replace_all=true to replace all occurrences, or provide a more specific old_string.",
                count
            ));
        }

        // Perform replacement
        let new_content = if params.replace_all {
            content.replace(&params.old_string, &params.new_string)
        } else {
            content.replacen(&params.old_string, &params.new_string, 1)
        };

        // Write back
        fs::write(path, &new_content)?;

        // Find the line where the change occurred and return context
        let new_lines: Vec<&str> = new_content.lines().collect();
        for (i, line) in new_lines.iter().enumerate() {
            if line.contains(&params.new_string) {
                // Show 3 lines before and after for context
                let start = i.saturating_sub(3);
                let end = (i + 4).min(new_lines.len());

                let mut result = format!("The file {} has been updated. Here's the result of running `cat -n` on a snippet of the edited file:\n", params.file_path);
                for (j, line) in new_lines[start..end].iter().enumerate() {
                    let line_num = start + j + 1;
                    result.push_str(&format!("{:6}→{}\n", line_num, line));
                }
                return Ok(result);
            }
        }

        Ok(format!("File {} has been updated", params.file_path))
    }

    /// Find files matching a glob pattern
    pub fn glob(&self, params: GlobParams) -> Result<String> {
        let base_path = params.path.as_deref().unwrap_or(".");

        // Safety check: ensure path is within home directory
        let base_path_abs = std::path::Path::new(base_path).canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from(base_path));
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .map_err(|_| anyhow!("HOME environment variable not set"))?;

        if !base_path_abs.starts_with(&home) {
            return Err(anyhow!("Access denied: path must be within home directory ({})", home.display()));
        }

        let pattern = format!("{}/{}", base_path, params.pattern);

        let mut paths = Vec::new();
        for entry in glob::glob(&pattern)? {
            match entry {
                Ok(path) => {
                    // Skip hidden files, build directories, and system paths
                    let path_str = path.to_string_lossy();
                    if !path_str.contains("/.")
                        && !path_str.contains("/target/")
                        && !path_str.starts_with("/boot")
                        && !path_str.starts_with("/dev")
                        && !path_str.starts_with("/sys")
                        && !path_str.starts_with("/proc")
                        && !path_str.starts_with("/etc")
                        && !path_str.starts_with("/lost+found") {
                        paths.push(path.display().to_string());
                    }
                }
                Err(_) => {}, // Silently skip permission errors
            }
        }

        // Sort by modification time (most recent first)
        paths.sort_by_cached_key(|p| {
            fs::metadata(p)
                .and_then(|m| m.modified())
                .map(|t| std::time::SystemTime::now().duration_since(t).unwrap_or_default())
                .unwrap_or_default()
        });

        Ok(paths.join("\n"))
    }

    /// Search file contents using grep (simple implementation)
    pub fn grep(&self, params: GrepParams) -> Result<String> {
        // For now, we'll implement a simple grep
        // In a full implementation, we'd use the grep crate or call rg binary

        let base_path = params.path.as_deref().unwrap_or(".");
        let pattern = &params.pattern;
        let output_mode = params.output_mode.as_deref().unwrap_or("files_with_matches");

        let mut results = Vec::new();
        let glob_pattern = if let Some(g) = params.glob {
            format!("{}/{}", base_path, g)
        } else {
            format!("{}/**/*", base_path)
        };

        for entry in glob::glob(&glob_pattern)? {
            if let Ok(path) = entry {
                if !path.is_file() {
                    continue;
                }

                let path_str = path.to_string_lossy();
                if path_str.contains("/.") || path_str.contains("/target/") {
                    continue;
                }

                if let Ok(content) = fs::read_to_string(&path) {
                    let matches: Vec<_> = content
                        .lines()
                        .enumerate()
                        .filter(|(_, line)| line.contains(pattern))
                        .collect();

                    if !matches.is_empty() {
                        match output_mode {
                            "files_with_matches" => {
                                results.push(path.display().to_string());
                            }
                            "content" => {
                                for (line_num, line) in matches {
                                    results.push(format!("{}:{}:{}", path.display(), line_num + 1, line));
                                }
                            }
                            "count" => {
                                results.push(format!("{}:{}", path.display(), matches.len()));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if results.is_empty() {
            return Ok("No matches found".to_string());
        }

        if output_mode == "files_with_matches" {
            Ok(format!("Found {} files\n{}", results.len(), results.join("\n")))
        } else {
            Ok(results.join("\n"))
        }
    }
}
