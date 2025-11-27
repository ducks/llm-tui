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

## Session Storage

Sessions and messages are stored in a SQLite database at:
`~/.local/share/llm-tui/sessions.db`

Benefits:
- Efficient storage for long conversations
- Fast queries and filtering
- No system SQLite required (bundled with the binary)
- Single file to backup or sync

## Roadmap

- [ ] LLM provider integration (Claude API, OpenAI, Ollama)
- [ ] Setup wizard for API keys
- [ ] Context import from files/directories
- [ ] Daily notes integration
- [ ] Search functionality
- [ ] Session export
- [ ] Custom keybindings configuration
