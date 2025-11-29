# LLM TUI

A terminal user interface for LLM chat sessions supporting Ollama, Claude (Anthropic), and AWS Bedrock. Features vim-style keybindings, session management, tool execution with confirmation, and persistent file context.

## Features

- **Multi-Provider Support**: Ollama (local models), Claude API (Anthropic), and AWS Bedrock
- **Tool System**: 6 built-in tools (Read, Write, Edit, Glob, Grep, Bash) with user confirmation
- **File Context Persistence**: Files read during sessions are cached and restored across restarts
- **Session Management**: Create, browse, rename, and delete chat sessions
- **Project Support**: Organize sessions by project
- **Vim Keybindings**: Modal editing (Normal/Insert/Command modes)
- **Ollama Integration**: Streaming responses from local models with automatic memory management
- **Model Management**: Browse, download, and switch between models
- **Context Loading**: Import context from files or other sessions
- **SQLite Storage**: Efficient persistent storage with full conversation history
- **Autosave**: Configurable save modes (disabled, on-send, timer)

![Screenshot showing llm-tui session and project manage](screenshots/session-project-management.png)

![Screenshot showing llm-tui mistral chat](screenshots/chat.png)

![Screenshot showing LLM browsing screen](screenshots/browse.png)

## Installation

### Prerequisites
- [Ollama](https://ollama.ai) installed and running (for local models)
- Anthropic API key (optional, for Claude API)
- AWS credentials with Bedrock access (optional, for AWS Bedrock)
- Rust toolchain (or use Nix)

### With Nix
```bash
git clone https://github.com/yourusername/llm-tui
cd llm-tui
nix-shell
cargo build --release
./target/release/llm-tui
```

### Without Nix
```bash
git clone https://github.com/yourusername/llm-tui
cd llm-tui
cargo build --release
./target/release/llm-tui
```

The app will auto-start Ollama if configured (see Configuration section).

## Usage

### Keybindings

**Session List Screen (Press 1):**
- `j/k` or `↓/↑`: Navigate sessions
- `g`: Go to top
- `G`: Go to bottom
- `Enter`: Open selected session
- `d`: Delete selected session
- `1`: Sessions screen
- `q`: Quit

**Chat Screen (Press 2):**
- `i`: Enter insert mode to type message
- `Esc`: Return to normal mode
- `Enter` (normal mode): Send message
- `Enter` (insert mode): Add newline
- `Ctrl+Space` (insert mode): Send message
- `1`: Return to session list
- `2`: Return to chat (if in a session)
- `3`: Model management
- `4`: Browse model library

**Models Screen (Press 3):**
- `j/k` or `↓/↑`: Navigate models
- `Enter`: Select model (set as active)
- `3`: Models screen
- `4`: Browse model library

**Browser Screen (Press 4):**
- `j/k` or `↓/↑`: Navigate available models
- `Enter`: Download selected model
- `3`: Installed models
- `4`: Browse library

### Commands

**Session Management:**
- `:new` - Create new session with datetime ID
- `:new my-session-name` - Create new session with custom name
- `:rename my-new-name` - Rename current session
- `:delete-session` or `:ds` - Delete current session
- `:project discourse-yaks` - Set current project
- `:w` or `:save` - Save current session manually
- `:q` or `:quit` - Quit application

**Provider Management:**
- `:provider ollama` - Switch to Ollama (local models)
- `:provider claude` - Switch to Claude API (requires ANTHROPIC_API_KEY)
- `:provider bedrock` - Switch to AWS Bedrock (requires AWS credentials)

**Context Loading:**
- `:load filename.md` - Load context from a local file
- `:load session-name` - Load context from another session
  - Matches by exact ID, exact name, or partial name
  - Cannot load from current session

**Model Management:**
- `:models` - Open model management screen
- `:pull modelname` - Download a model from Ollama library
- `:delete modelname` - Remove an installed model

## Tool System

When using Claude or Bedrock providers, the AI can use these tools to interact with your system:

- **Read**: Read file contents (sandboxed to home directory)
- **Write**: Create or overwrite files
- **Edit**: Make targeted edits to existing files
- **Glob**: Find files by pattern (e.g., `*.rs`, `src/**/*.toml`)
- **Grep**: Search file contents with regex
- **Bash**: Execute shell commands (sandboxed to home directory, 2min timeout)

All tool executions require user confirmation (y/n/q). Tool results are cached per session, and files read during a session are automatically restored when reopening the session.

## Configuration

Config file location: `~/.config/llm-tui/config.toml`

Default configuration:
```toml
autosave_mode = "onsend"
autosave_interval_seconds = 30
ollama_url = "http://localhost:11434"
ollama_auto_start = true
ollama_model = "llama2"
anthropic_api_key = ""  # Set via ANTHROPIC_API_KEY env var
bedrock_model = "us.anthropic.claude-sonnet-4-20250514-v1:0"
```

Settings:
- `autosave_mode`: How to save sessions (default: "onsend")
  - `"disabled"`: Manual save only (use `:w`)
  - `"onsend"`: Save immediately when sending messages
  - `"timer"`: Save every N seconds (see `autosave_interval_seconds`)
- `autosave_interval_seconds`: Timer interval in seconds (default: 30)
- `ollama_url`: Ollama server URL (default: "http://localhost:11434")
- `ollama_auto_start`: Auto-start Ollama server if not running (default: true)
- `ollama_model`: Default model to use (default: "llama2")

The config file is automatically created with defaults on first run.

Examples:
```toml
# Save every 5 minutes
autosave_mode = "timer"
autosave_interval_seconds = 300

# Disable autosave entirely
autosave_mode = "disabled"
```

## Session Storage

Sessions and messages are stored in a SQLite database at:
`~/.local/share/llm-tui/sessions.db`

Benefits:
- Efficient storage for long conversations
- Fast queries and filtering
- No system SQLite required (bundled with the binary)
- Single file to backup or sync

## Model Recommendations

### For Chat
- **mistral** - Fast, efficient, great for conversation
- **llama3.2** - Latest generation, excellent instruction following
- **phi3** - Microsoft's model, good balance of size and quality
- **qwen2.5** - Strong at reasoning and chat

### For Code
- **codellama** - Meta's code-specialized model
- **deepseek-coder** - Excellent at code generation and understanding
- **starcoder2** - Multi-language code model

### Note on Base Models
Models without `:chat` suffix (like `llama2`) are base models trained for text
completion, not conversation. They will try to continue your text rather than
respond as an assistant. Always use the `:chat` variant or dedicated chat
models for interactive use.

Examples:
- `llama2` - Base model (text completion)
- `llama2:chat` - Chat-tuned variant (conversation)

## Roadmap

- [x] Ollama integration with streaming responses
- [x] SQLite-based session storage
- [x] Configurable autosave modes
- [x] Model management (browse, download, select models)
- [x] Model management commands (:models, :pull, :delete)
- [x] Context loading from files and sessions (:load)
- [x] Session rename and delete
- [x] Claude API integration
- [x] AWS Bedrock integration
- [x] Tool system (Read, Write, Edit, Glob, Grep, Bash)
- [x] Tool confirmation workflow
- [x] File context persistence across sessions
- [ ] OpenAI API integration
- [ ] Setup wizard for API keys
- [ ] Daily notes integration
- [ ] Search functionality
- [ ] Session export
- [ ] Custom keybindings configuration
