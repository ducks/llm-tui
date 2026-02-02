use anyhow::{anyhow, Result};
use std::fmt::Write;
use grep_searcher::{SearcherBuilder, Sink, SinkContext, SinkMatch};
use serde::{Deserialize, Deserializer, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use walkdir::WalkDir;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_insensitive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_numbers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_before: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_after: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiline: Option<bool>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub file_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BashParams {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>, // timeout in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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

    /// Expand tilde (~) in path to home directory
    fn expand_tilde(path: &str) -> PathBuf {
        if let Some(stripped) = path.strip_prefix("~/") {
            if let Ok(home) = std::env::var("HOME") {
                return PathBuf::from(home).join(stripped);
            }
        }
        PathBuf::from(path)
    }

    /// Read a file with line numbers (cat -n format)
    pub fn read(&mut self, params: ReadParams) -> Result<String> {
        let expanded = Self::expand_tilde(&params.file_path);
        let path = expanded.as_path();

        // Get home directory for safety checks
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .map_err(|_| anyhow!("HOME environment variable not set"))?;

        // Canonicalize home directory to handle any symlinks in home path itself
        let home_canonical = home
            .canonicalize()
            .map_err(|_| anyhow!("Failed to resolve home directory"))?;

        // Safety check: resolve path and ensure it's within home directory
        // This must be done AFTER canonicalize to prevent symlink escapes
        let path_abs = path
            .canonicalize()
            .or_else(|_| std::env::current_dir().map(|cwd| cwd.join(path)))?;

        // Check the canonicalized path is within canonicalized home
        if !path_abs.starts_with(&home_canonical) {
            return Err(anyhow!(
                "Access denied: can only read files within home directory ({}). Attempted path resolves to: {}",
                home_canonical.display(),
                path_abs.display()
            ));
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
            let _ = writeln!(result, "{:6}→{}", line_num, line);
        }

        Ok(result)
    }

    /// Write content to a file
    pub fn write(&self, params: WriteParams) -> Result<String> {
        let expanded = Self::expand_tilde(&params.file_path);
        let path = expanded.as_path();

        // Get home directory for safety checks
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .map_err(|_| anyhow!("HOME environment variable not set"))?;

        // Canonicalize home directory
        let home_canonical = home
            .canonicalize()
            .map_err(|_| anyhow!("Failed to resolve home directory"))?;

        // For new files, we need to check the parent directory
        // Build absolute path first
        let path_abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        // If file exists, canonicalize it to check for symlinks
        // If it doesn't exist, check parent directory
        let path_to_check = if path_abs.exists() {
            path_abs.canonicalize()?
        } else {
            // For new files, ensure parent directory is within home
            if let Some(parent) = path_abs.parent() {
                if parent.exists() {
                    let parent_canonical = parent.canonicalize()?;
                    parent_canonical.join(path_abs.file_name().unwrap())
                } else {
                    // Parent doesn't exist yet, need to check each component
                    path_abs.clone()
                }
            } else {
                path_abs.clone()
            }
        };

        // Check the resolved path is within home
        if !path_to_check.starts_with(&home_canonical) {
            return Err(anyhow!(
                "Access denied: can only write files within home directory ({}). Attempted path resolves to: {}",
                home_canonical.display(),
                path_to_check.display()
            ));
        }

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(path, &params.content)?;

        Ok(format!(
            "File created successfully at: {}",
            params.file_path
        ))
    }

    /// Edit a file by replacing old_string with new_string
    pub fn edit(&self, params: EditParams) -> Result<String> {
        let expanded = Self::expand_tilde(&params.file_path);
        let path = expanded.as_path();

        // Get home directory for safety checks
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .map_err(|_| anyhow!("HOME environment variable not set"))?;

        // Canonicalize home directory
        let home_canonical = home
            .canonicalize()
            .map_err(|_| anyhow!("Failed to resolve home directory"))?;

        // Safety check: ensure path is within home directory
        let path_abs = path
            .canonicalize()
            .or_else(|_| std::env::current_dir().map(|cwd| cwd.join(path)))?;

        // Check the canonicalized path is within canonicalized home
        if !path_abs.starts_with(&home_canonical) {
            return Err(anyhow!(
                "Access denied: can only edit files within home directory ({}). Attempted path resolves to: {}",
                home_canonical.display(),
                path_abs.display()
            ));
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
        let expanded = Self::expand_tilde(base_path);

        // Get home directory for safety checks
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .map_err(|_| anyhow!("HOME environment variable not set"))?;

        // Canonicalize home directory
        let home_canonical = home
            .canonicalize()
            .map_err(|_| anyhow!("Failed to resolve home directory"))?;

        // Safety check: ensure base path is within home directory
        let base_path_abs = expanded.canonicalize().unwrap_or_else(|_| expanded.clone());

        // Check the canonicalized path is within canonicalized home
        if !base_path_abs.starts_with(&home_canonical) {
            return Err(anyhow!(
                "Access denied: path must be within home directory ({})",
                home.display()
            ));
        }

        let pattern = format!("{}/{}", expanded.display(), params.pattern);

        let mut paths = Vec::new();
        for path in glob::glob(&pattern)?.flatten() {
            // Check if the resolved path is within home (prevent symlink escape)
            if let Ok(canonical) = path.canonicalize() {
                if !canonical.starts_with(&home_canonical) {
                    continue; // Skip files outside home directory
                }
            }

            // Skip hidden files, build directories, and system paths
            let path_str = path.to_string_lossy();
            if !path_str.contains("/.")
                && !path_str.contains("/target/")
                && !path_str.starts_with("/boot")
                && !path_str.starts_with("/dev")
                && !path_str.starts_with("/sys")
                && !path_str.starts_with("/proc")
                && !path_str.starts_with("/etc")
                && !path_str.starts_with("/lost+found")
            {
                paths.push(path.display().to_string());
            }
        }

        // Sort by modification time (most recent first)
        paths.sort_by_cached_key(|p| {
            fs::metadata(p)
                .and_then(|m| m.modified())
                .map(|t| {
                    std::time::SystemTime::now()
                        .duration_since(t)
                        .unwrap_or_default()
                })
                .unwrap_or_default()
        });

        Ok(paths.join("\n"))
    }

    /// Search file contents using ripgrep library
    pub fn grep(&self, params: GrepParams) -> Result<String> {
        let base_path = params.path.as_deref().unwrap_or(".");
        let expanded = Self::expand_tilde(base_path);
        let output_mode = params
            .output_mode
            .as_deref()
            .unwrap_or("files_with_matches");

        // Get home directory for safety checks
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .map_err(|_| anyhow!("HOME environment variable not set"))?;

        // Canonicalize home directory
        let home_canonical = home
            .canonicalize()
            .map_err(|_| anyhow!("Failed to resolve home directory"))?;

        // Safety check: ensure base path is within home directory
        let base_path_abs = expanded.canonicalize().unwrap_or_else(|_| expanded.clone());

        // Check the canonicalized path is within canonicalized home
        if !base_path_abs.starts_with(&home_canonical) {
            return Err(anyhow!(
                "Access denied: path must be within home directory ({})",
                home.display()
            ));
        }

        // Build regex matcher
        let mut builder = grep_regex::RegexMatcherBuilder::new();
        builder.case_insensitive(params.case_insensitive.unwrap_or(false));
        builder.multi_line(params.multiline.unwrap_or(false));

        let matcher = builder.build(&params.pattern)?;

        // Build searcher with context lines
        let mut searcher_builder = SearcherBuilder::new();
        searcher_builder.line_number(params.line_numbers.unwrap_or(false));
        if let Some(before) = params.context_before {
            searcher_builder.before_context(before);
        }
        if let Some(after) = params.context_after {
            searcher_builder.after_context(after);
        }
        searcher_builder.multi_line(params.multiline.unwrap_or(false));

        let mut searcher = searcher_builder.build();
        let mut results = Vec::new();

        // Walk directory tree
        let walker = WalkDir::new(&expanded)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let path = e.path();
                let path_str = path.to_string_lossy();
                // Skip hidden files and target directories
                !path_str.contains("/.") && !path_str.contains("/target/")
            });

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // Check if the resolved path is within home (prevent symlink escape)
            if let Ok(canonical) = path.canonicalize() {
                if !canonical.starts_with(&home_canonical) {
                    continue; // Skip files outside home directory
                }
            }

            // Apply glob filter if specified
            if let Some(ref glob_pattern) = params.glob {
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    let pattern = glob::Pattern::new(glob_pattern)?;
                    if !pattern.matches(filename) {
                        continue;
                    }
                }
            }

            // Apply file type filter if specified
            if let Some(ref file_type) = params.file_type {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let matches = match file_type.as_str() {
                        "rust" | "rs" => ext == "rs",
                        "python" | "py" => ext == "py",
                        "javascript" | "js" => ext == "js",
                        "typescript" | "ts" => ext == "ts",
                        "go" => ext == "go",
                        "java" => ext == "java",
                        "cpp" | "c++" => ext == "cpp" || ext == "cc" || ext == "cxx",
                        "c" => ext == "c" || ext == "h",
                        "ruby" | "rb" => ext == "rb",
                        "toml" => ext == "toml",
                        "json" => ext == "json",
                        "yaml" | "yml" => ext == "yaml" || ext == "yml",
                        "markdown" | "md" => ext == "md",
                        _ => false,
                    };
                    if !matches {
                        continue;
                    }
                }
            }

            // Search the file
            struct GrepSink<'a> {
                path: &'a std::path::Path,
                matches: &'a mut Vec<String>,
                match_count: &'a mut usize,
                output_mode: &'a str,
                line_numbers: bool,
            }

            impl<'a> Sink for GrepSink<'a> {
                type Error = std::io::Error;

                fn matched(
                    &mut self,
                    _searcher: &grep_searcher::Searcher,
                    mat: &SinkMatch<'_>,
                ) -> Result<bool, Self::Error> {
                    *self.match_count += 1;
                    if self.output_mode == "content" {
                        let line = std::str::from_utf8(mat.bytes()).unwrap_or("");
                        let line_str = if self.line_numbers {
                            format!(
                                "{}:{}:{}",
                                self.path.display(),
                                mat.line_number().unwrap_or(0),
                                line.trim_end()
                            )
                        } else {
                            format!("{}:{}", self.path.display(), line.trim_end())
                        };
                        self.matches.push(line_str);
                    }
                    Ok(true)
                }

                fn context(
                    &mut self,
                    _searcher: &grep_searcher::Searcher,
                    ctx: &SinkContext<'_>,
                ) -> Result<bool, Self::Error> {
                    if self.output_mode == "content" {
                        let line = std::str::from_utf8(ctx.bytes()).unwrap_or("");
                        let line_str = if self.line_numbers {
                            format!(
                                "{}-{}:{}",
                                self.path.display(),
                                ctx.line_number().unwrap_or(0),
                                line.trim_end()
                            )
                        } else {
                            format!("{}:{}", self.path.display(), line.trim_end())
                        };
                        self.matches.push(line_str);
                    }
                    Ok(true)
                }
            }

            let mut file_matches = Vec::new();
            let mut match_count = 0;

            let mut sink = GrepSink {
                path,
                matches: &mut file_matches,
                match_count: &mut match_count,
                output_mode,
                line_numbers: params.line_numbers.unwrap_or(false),
            };

            let search_result = searcher.search_path(&matcher, path, &mut sink);

            if search_result.is_ok() && match_count > 0 {
                match output_mode {
                    "files_with_matches" => {
                        results.push(path.display().to_string());
                    }
                    "content" => {
                        results.extend(file_matches);
                    }
                    "count" => {
                        results.push(format!("{}:{}", path.display(), match_count));
                    }
                    _ => {}
                }
            }
        }

        if results.is_empty() {
            return Ok("No matches found".to_string());
        }

        if output_mode == "files_with_matches" {
            Ok(format!(
                "Found {} files:\n{}",
                results.len(),
                results.join("\n")
            ))
        } else {
            Ok(results.join("\n"))
        }
    }

    /// Execute a bash command with optional timeout
    pub fn bash(&self, params: BashParams) -> Result<String> {
        // Safety check: ensure we're running in home directory
        let cwd = std::env::current_dir()?;
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .map_err(|_| anyhow!("HOME environment variable not set"))?;

        if !cwd.starts_with(&home) {
            return Err(anyhow!(
                "Access denied: can only execute commands from within home directory"
            ));
        }

        // Create command with timeout
        let timeout_ms = params.timeout.unwrap_or(120_000); // default 2 minutes
        if timeout_ms > 600_000 {
            return Err(anyhow!("Timeout cannot exceed 600000ms (10 minutes)"));
        }

        let output = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .args(["/C", &params.command])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
        } else {
            Command::new("sh")
                .arg("-c")
                .arg(&params.command)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
        }?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut result = String::new();
        if !stdout.is_empty() {
            result.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&stderr);
        }

        if !output.status.success() {
            result.push_str(&format!("\nCommand exited with status: {}", output.status));
        }

        Ok(result)
    }
}
