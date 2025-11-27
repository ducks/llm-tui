# Claude Code Tool Analysis

Analysis of how Claude Code implements tools for file editing, web browsing,
and other operations. This document examines the tools available in the Claude
Code system for potential porting to llm-tui.

## Tool Categories

### 1. File Operations

#### Read Tool
- **Purpose**: Read files from filesystem
- **Parameters**:
  - `file_path` (required): Absolute path to file
  - `offset` (optional): Line number to start reading from
  - `limit` (optional): Number of lines to read
- **Features**:
  - Returns files with line numbers (cat -n format)
  - Can read images (multimodal support)
  - Can read Jupyter notebooks (.ipynb files)
  - Handles long files with offset/limit for pagination
  - Max 2000 lines or 2000 chars per line
- **Notes**: Can be called multiple times in parallel

#### Write Tool
- **Purpose**: Write new files or overwrite existing files
- **Parameters**:
  - `file_path` (required): Absolute path
  - `content` (required): File content to write
- **Safety**: Requires Read before Write on existing files
- **Notes**: ALWAYS prefer editing over writing new files

#### Edit Tool
- **Purpose**: Make exact string replacements in files
- **Parameters**:
  - `file_path` (required): Absolute path
  - `old_string` (required): Text to replace (must be unique)
  - `new_string` (required): Replacement text
  - `replace_all` (optional): Replace all occurrences (default false)
- **Safety**: Requires Read before Edit
- **Notes**:
  - Must preserve exact indentation from Read output
  - Line numbers in Read output are not part of file content
  - Fails if old_string is not unique (must provide more context)

#### Glob Tool
- **Purpose**: Fast file pattern matching
- **Parameters**:
  - `pattern` (required): Glob pattern (e.g., "**/*.rs", "src/**/*.ts")
  - `path` (optional): Directory to search in
- **Features**: Returns matching paths sorted by modification time
- **Notes**: Works with any codebase size, very fast

#### Grep Tool
- **Purpose**: Content search using ripgrep
- **Parameters**:
  - `pattern` (required): Regex pattern
  - `glob` (optional): Filter files (e.g., "*.js")
  - `type` (optional): File type (e.g., "js", "py", "rust")
  - `output_mode`: "content", "files_with_matches", "count"
  - `-i`: Case insensitive
  - `-n`: Show line numbers
  - `-A`/`-B`/`-C`: Context lines
  - `multiline`: Enable multiline matching
  - `head_limit`: Limit output lines
- **Notes**: Full regex support, faster than grep

### 2. Shell Operations

#### Bash Tool
- **Purpose**: Execute shell commands
- **Parameters**:
  - `command` (required): Command to execute
  - `description`: Clear description of what command does
  - `timeout` (optional): Max 600000ms (10 minutes), default 120000ms
  - `run_in_background` (optional): Run in background
- **Safety Rules**:
  - NEVER update git config
  - NEVER use destructive git commands unless explicitly requested
  - NEVER skip hooks (--no-verify, --no-gpg-sign)
  - NEVER force push to main/master
  - Avoid git commit --amend (only use when explicitly requested)
  - Only commit when explicitly asked by user
- **Notes**:
  - Use specialized tools (Read/Edit/Write) instead of cat/sed/awk/echo
  - Quote paths with spaces
  - Use && to chain dependent commands
  - Use ; for sequential commands where failure is ok
  - Can run multiple commands in parallel if independent

#### BashOutput Tool
- **Purpose**: Retrieve output from background bash shell
- **Parameters**:
  - `bash_id` (required): Shell ID
  - `filter` (optional): Regex to filter output lines

#### KillShell Tool
- **Purpose**: Terminate background bash shell
- **Parameters**: `shell_id` (required)

### 3. Web Operations

#### WebFetch Tool
- **Purpose**: Fetch and analyze web content
- **Parameters**:
  - `url` (required): Fully-formed valid URL
  - `prompt` (required): What information to extract
- **Features**:
  - Converts HTML to markdown
  - Processes content with AI model
  - HTTP auto-upgraded to HTTPS
  - 15-minute cache for repeated access
  - Handles redirects (informs user, requires new request)
- **Notes**: Read-only, does not modify files

### 4. Advanced Tools

#### Task Tool (Agent Launcher)
- **Purpose**: Launch specialized sub-agents for complex tasks
- **Parameters**:
  - `description` (required): Short 3-5 word description
  - `prompt` (required): Detailed task instructions
  - `subagent_type` (required): Type of agent to use
- **Available Agents**:
  - `general-purpose`: Complex multi-step tasks, searching, research
  - `statusline-setup`: Configure status line settings
  - `output-style-setup`: Create output styles
  - `Explore`: Fast codebase exploration (glob, grep, read, bash)
    - Thoroughness: "quick", "medium", "very thorough"
- **Notes**:
  - Can launch multiple agents in parallel
  - Each agent is stateless (single response)
  - Agent output not visible to user (you must summarize)
  - Use for complex searches instead of manual glob/grep

#### TodoWrite Tool
- **Purpose**: Track and plan tasks during session
- **Parameters**: `todos` array with content, status, activeForm
- **Task States**: pending, in_progress, completed
- **Rules**:
  - Use for complex multi-step tasks (3+ steps)
  - Update in real-time
  - Exactly ONE task in_progress at a time
  - Mark completed immediately after finishing
  - Never mark completed if tests fail or errors occur
- **Notes**: Do NOT use for trivial single-step tasks

#### AskUserQuestion Tool
- **Purpose**: Ask user questions during execution
- **Parameters**: `questions` array with question, header, options, multiSelect
- **Features**:
  - 1-4 questions per call
  - 2-4 options per question
  - Short header (max 12 chars)
  - User can always select "Other" for custom input
  - Supports multi-select for non-exclusive choices

### 5. GitHub Operations (MCP)

Claude Code includes Model Context Protocol (MCP) tools for GitHub:

- `create_or_update_file`: Create/update single file
- `push_files`: Push multiple files in single commit
- `create_repository`: Create new repo
- `get_file_contents`: Read file from repo
- `search_repositories`: Search GitHub repos
- `create_issue`: Create issue
- `create_pull_request`: Create PR
- `fork_repository`: Fork repo
- `create_branch`: Create branch
- `list_commits`: List commits
- `list_issues`: List/filter issues
- `update_issue`: Update issue
- `add_issue_comment`: Comment on issue
- `search_code`: Search code across GitHub
- `search_issues`: Search issues/PRs
- `search_users`: Search users
- `get_issue`: Get issue details
- `get_pull_request`: Get PR details
- `list_pull_requests`: List/filter PRs
- `create_pull_request_review`: Review PR
- `merge_pull_request`: Merge PR
- `get_pull_request_files`: List changed files
- `get_pull_request_status`: Get status checks
- `update_pull_request_branch`: Update PR branch
- `get_pull_request_comments`: Get review comments
- `get_pull_request_reviews`: Get reviews

## Implementation Strategy for llm-tui

### Phase 1: Core File Operations (Highest Priority)
These are essential for code assistance:

1. **File Reading** (Read tool equivalent)
   - Use std::fs for basic reading
   - Add line number display
   - Support offset/limit for large files
   - Consider image support later (not essential)

2. **File Writing** (Write tool equivalent)
   - Use std::fs::write
   - Add safety check (confirm before overwriting)
   - Track which files have been read

3. **File Editing** (Edit tool equivalent)
   - String search and replace
   - Safety: ensure old_string is unique
   - Preserve formatting/indentation
   - Support replace_all flag

4. **File Search** (Glob tool equivalent)
   - Use walkdir or globwalk crate
   - Support glob patterns
   - Sort by modification time

5. **Content Search** (Grep tool equivalent)
   - Use ripgrep library (grep crate) or call rg binary
   - Support regex patterns
   - Output modes: content, files, count
   - Context lines support

### Phase 2: Shell Execution (High Priority)
Essential for running tests, builds, etc:

1. **Command Execution** (Bash tool equivalent)
   - Use std::process::Command
   - Capture stdout/stderr
   - Support timeout
   - Background execution with channels
   - Safety rules for git operations

### Phase 3: Web Operations (Medium Priority)
Useful but not essential:

1. **Web Fetching** (WebFetch tool equivalent)
   - Use reqwest for HTTP
   - Convert HTML to markdown (html2text crate)
   - Add caching layer
   - Handle redirects

### Phase 4: Advanced Features (Lower Priority)
Nice to have, but complex:

1. **Task Management** (TodoWrite tool equivalent)
   - In-memory task tracking
   - Display in UI
   - Not essential for MVP

2. **Sub-agents** (Task tool equivalent)
   - Very complex, requires full agent system
   - Defer to much later phase

3. **GitHub Integration** (MCP tools)
   - Use octocrab crate for GitHub API
   - Add as optional feature
   - Not essential for core functionality

## Tool Request/Response Cycle

From examining Claude Code's behavior:

1. **User sends message** to LLM
2. **LLM decides** whether to use tools or respond with text
3. **LLM returns** either:
   - Text response (shown to user)
   - Tool use request(s) (JSON with tool name + parameters)
4. **Application executes** requested tool(s)
5. **Application sends** tool results back to LLM
6. **LLM processes** results and decides:
   - Use more tools (repeat cycle)
   - Respond to user with findings

This is handled by Claude API's native tool use support. The application:
- Defines available tools in API request
- Receives tool use requests in response
- Executes tools locally
- Sends results back in next API call

## Key Rust Crates Needed

### File Operations
- `std::fs` - Standard file operations
- `walkdir` or `globwalk` - Directory walking and glob patterns
- `grep` or call `rg` binary - Content search

### Shell Execution
- `std::process::Command` - Run commands
- `std::sync::mpsc` - Channels for streaming output

### Web Operations
- `reqwest` - HTTP client (already have for Ollama)
- `html2text` - Convert HTML to markdown
- `url` - URL parsing and validation

### API Integration
- `serde` and `serde_json` - JSON serialization (already have)
- `tokio` - Async runtime (already have)

### GitHub (Optional)
- `octocrab` - GitHub API client

## Next Steps

1. **Research Claude API tool use** - Understand the exact JSON format for:
   - Tool definitions sent to API
   - Tool requests received from API
   - Tool results sent back to API

2. **Implement file operation tools** first (highest value):
   - File reading
   - File editing (string replacement)
   - File writing
   - File search (glob patterns)
   - Content search (ripgrep)

3. **Implement shell execution** second:
   - Command execution
   - Streaming output
   - Background execution

4. **Test with simple prompts** like:
   - "Read the README.md file"
   - "Search for 'Session' in all .rs files"
   - "Add a comment to main.rs explaining what it does"
   - "Run cargo test"

5. **Add web fetching** third:
   - HTTP requests
   - HTML to markdown conversion
   - Caching

6. **Document for users** how to:
   - Enable tool use
   - Grant permissions
   - Understand what tools are doing
