# LLM TUI

A terminal user interface for managing LLM chat sessions with support for multiple providers (Claude, OpenAI, local models) and project-based organization.

## Features

- **Session Management**: Create, browse, and resume chat sessions
- **Project Support**: Organize sessions by project
- **Vim Keybindings**: Familiar modal editing (Normal/Insert/Command modes)
- **Multi-LLM Support**: (Planned) Claude, OpenAI, Ollama, llama.cpp
- **Context Automation**: (Planned) Auto-save and import context

## Installation

```bash
cd ~/dev/llm-tui
nix-shell
cargo build --release
cargo run
```

## Usage

### Keybindings

**Session List Screen:**
- `j/k` or `↓/↑`: Navigate sessions
- `g`: Go to top
- `G`: Go to bottom
- `Enter`: Open selected session
- `:new [name]`: Create new session (datetime if no name provided)
- `:project <name>`: Set current project context
- `q`: Quit

**Chat Screen:**
- `i`: Enter insert mode to type message
- `Esc`: Return to normal mode
- `Enter` (normal mode): Send message
- `Enter` (insert mode): Add newline
- `Ctrl+Space` (insert mode): Send message
- `1`: Return to session list
- `2`: Return to chat (if in a session)
- `:w` or `:save`: Save session
- `:q` or `:quit`: Quit

### Commands

- `:new` - Create new session with datetime ID
- `:new my-session-name` - Create new session with custom name
- `:project discourse-yaks` - Set current project to "discourse-yaks"
- `:w` - Save current session
- `:q` - Quit

## Configuration

Config file location: `~/.config/llm-tui/config.toml`

Default configuration:
```toml
autosave_mode = "onsend"
autosave_interval_seconds = 30
default_llm_provider = "none"
```

Settings:
- `autosave_mode`: How to save sessions (default: "onsend")
  - `"disabled"`: Manual save only (use `:w`)
  - `"onsend"`: Save immediately when sending messages
  - `"timer"`: Save every N seconds (see `autosave_interval_seconds`)
- `autosave_interval_seconds`: Timer interval in seconds (default: 30)
- `default_llm_provider`: Default LLM provider for new sessions (default: "none")

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
- [ ] Model management commands (:models, :pull, :delete)
- [ ] File editing capabilities
- [ ] Claude API integration
- [ ] OpenAI API integration
- [ ] Setup wizard for API keys
- [ ] Context import from files/directories
- [ ] Daily notes integration
- [ ] Search functionality
- [ ] Session export
- [ ] Custom keybindings configuration
