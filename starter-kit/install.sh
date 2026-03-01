#!/bin/bash
# ============================================================================
# Triumvirate Starter Kit — Installer
#
# Builds and wires the complete Triumvirate operating environment:
#   mcp-server/          — Builds the inter-agent MCP server (npm install + tsc)
#   ~/.claude/hooks/     — Claude Code hooks (session lifecycle + safety gates)
#   ~/.claude/           — Claude Code settings and instructions
#   ~/.claude.json       — MCP server registration (inter-agent-gemini + inter-agent-codex)
#   ~/.codex/            — Codex CLI config, hooks, and skills
#   ~/.codex/config.toml — MCP server registration (inter-agent-gemini)
#   ~/.gemini/           — Gemini CLI instructions
#   ~/.gemini/settings.json — MCP server registration (inter-agent-codex)
#
# Usage:
#   cd triumvirate/starter-kit
#   chmod +x install.sh
#   ./install.sh
#
# Safe to re-run — backs up existing files before overwriting.
# ============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BACKUP_SUFFIX=".backup-$(date +%Y%m%d_%H%M%S)"

# Colors (if terminal supports them)
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No color

info()  { printf "${BLUE}[INFO]${NC}  %s\n" "$1"; }
ok()    { printf "${GREEN}[OK]${NC}    %s\n" "$1"; }
warn()  { printf "${YELLOW}[WARN]${NC}  %s\n" "$1"; }
err()   { printf "${RED}[ERROR]${NC} %s\n" "$1"; }

# Backup a file if it exists
backup_if_exists() {
  local target="$1"
  if [[ -f "$target" ]]; then
    cp "$target" "${target}${BACKUP_SUFFIX}"
    warn "Backed up existing: ${target} → ${target}${BACKUP_SUFFIX}"
  fi
}

# Copy a file, creating parent directories as needed
install_file() {
  local src="$1" dest="$2"
  local dest_dir
  dest_dir="$(dirname "$dest")"
  mkdir -p "$dest_dir"
  backup_if_exists "$dest"
  cp "$src" "$dest"
  ok "Installed: $dest"
}

# Copy a directory recursively
install_dir() {
  local src="$1" dest="$2"
  mkdir -p "$dest"
  # Copy files preserving structure
  (cd "$src" && find . -type f | while IFS= read -r f; do
    f="${f#./}"
    install_file "$src/$f" "$dest/$f"
  done)
}

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║          Triumvirate Starter Kit — Installer                ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

# ── Prerequisites ──────────────────────────────────────────────
if ! command -v jq &>/dev/null; then
  err "jq is required but not installed."
  err "  macOS:  brew install jq"
  err "  Ubuntu: apt-get install jq"
  exit 1
fi

# ── Verify we're in the right directory ────────────────────────
if [[ ! -d "$SCRIPT_DIR/claude/hooks" ]]; then
  err "Cannot find claude/hooks/ — run this script from the starter-kit directory."
  exit 1
fi

# ── 1. Claude Code Hooks ──────────────────────────────────────
info "Installing Claude Code hooks..."
CLAUDE_HOOKS_DIR="$HOME/.claude/hooks"
mkdir -p "$CLAUDE_HOOKS_DIR"

for hook in "$SCRIPT_DIR"/claude/hooks/*.sh; do
  [[ -f "$hook" ]] || continue
  dest="$CLAUDE_HOOKS_DIR/$(basename "$hook")"
  install_file "$hook" "$dest"
  chmod +x "$dest"
done
ok "Claude hooks installed ($(ls "$SCRIPT_DIR"/claude/hooks/*.sh | wc -l | tr -d ' ') files)"

# ── 2. Claude Code Settings ──────────────────────────────────
info "Installing Claude Code settings..."

# settings.json — MERGE hooks into existing if present
SETTINGS_DEST="$HOME/.claude/settings.json"
if [[ -f "$SETTINGS_DEST" ]]; then
  # Check if hooks are already configured
  if jq -e '.hooks' "$SETTINGS_DEST" >/dev/null 2>&1; then
    warn "settings.json already has hooks configured — skipping (won't overwrite)."
    warn "To use starter-kit hooks, manually merge from: $SCRIPT_DIR/claude/settings.json"
  else
    # Merge: add hooks key to existing settings
    backup_if_exists "$SETTINGS_DEST"
    HOOKS_JSON=$(jq '.hooks' "$SCRIPT_DIR/claude/settings.json")
    jq --argjson hooks "$HOOKS_JSON" '. + {hooks: $hooks}' "$SETTINGS_DEST" > "${SETTINGS_DEST}.tmp" \
      && mv "${SETTINGS_DEST}.tmp" "$SETTINGS_DEST"
    ok "Merged hooks into existing settings.json"
  fi
else
  install_file "$SCRIPT_DIR/claude/settings.json" "$SETTINGS_DEST"
fi

# CLAUDE.md — only install if not present (don't overwrite custom instructions)
CLAUDE_MD_DEST="$HOME/.claude/CLAUDE.md"
if [[ -f "$CLAUDE_MD_DEST" ]]; then
  warn "CLAUDE.md already exists — skipping (won't overwrite your instructions)."
  warn "Starter template available at: $SCRIPT_DIR/claude/CLAUDE.md"
else
  install_file "$SCRIPT_DIR/claude/CLAUDE.md" "$CLAUDE_MD_DEST"
fi

# ── 3. Codex CLI ──────────────────────────────────────────────
info "Installing Codex CLI configuration..."
CODEX_DIR="$HOME/.codex"
mkdir -p "$CODEX_DIR/hooks" "$CODEX_DIR/skills"

# Hooks
for hook in "$SCRIPT_DIR"/codex/hooks/*.sh; do
  [[ -f "$hook" ]] || continue
  dest="$CODEX_DIR/hooks/$(basename "$hook")"
  install_file "$hook" "$dest"
  chmod +x "$dest"
done

# Skills (recursive copy)
if [[ -d "$SCRIPT_DIR/codex/skills" ]]; then
  install_dir "$SCRIPT_DIR/codex/skills" "$CODEX_DIR/skills"
fi

# config.toml — only if not present
if [[ -f "$CODEX_DIR/config.toml" ]]; then
  warn "config.toml already exists — skipping (won't overwrite)."
  warn "Starter template available at: $SCRIPT_DIR/codex/config.toml"
else
  install_file "$SCRIPT_DIR/codex/config.toml" "$CODEX_DIR/config.toml"
fi

# AGENTS.md — only if not present
if [[ -f "$CODEX_DIR/AGENTS.md" ]]; then
  warn "AGENTS.md already exists — skipping."
else
  install_file "$SCRIPT_DIR/codex/AGENTS.md" "$CODEX_DIR/AGENTS.md"
fi

ok "Codex configuration installed"

# ── 4. Gemini CLI ──────────────────────────────────────────────
info "Installing Gemini CLI configuration..."
GEMINI_DIR="$HOME/.gemini"
mkdir -p "$GEMINI_DIR/hooks"

if [[ -f "$GEMINI_DIR/GEMINI.md" ]]; then
  warn "GEMINI.md already exists — skipping."
else
  install_file "$SCRIPT_DIR/gemini/GEMINI.md" "$GEMINI_DIR/GEMINI.md"
fi

# Gemini hooks (session persistence, auto-stage, pre-compact summarization)
for hook in "$SCRIPT_DIR"/gemini/hooks/*.sh; do
  [[ -f "$hook" ]] || continue
  dest="$GEMINI_DIR/hooks/$(basename "$hook")"
  install_file "$hook" "$dest"
  chmod +x "$dest"
done
ok "Gemini hooks installed ($(ls "$SCRIPT_DIR"/gemini/hooks/*.sh 2>/dev/null | wc -l | tr -d ' ') files)"

ok "Gemini configuration installed"

# ── 5. Inter-Agent MCP Server ─────────────────────────────────
# This is the core of the Triumvirate — without it, the agents can't
# spawn daemons or do multi-turn inter-agent conversations.
# Each agent registers the OTHER agents' MCP servers so they are
# first-party participants with spawn_daemon / ask_daemon support.
info "Building and wiring inter-agent MCP server..."

MCP_SERVER_DIR="$(cd "$SCRIPT_DIR/../mcp-server" 2>/dev/null && pwd)" || {
  err "Cannot find mcp-server/ directory. Make sure you cloned the full triumvirate repo."
  err "Expected: $(dirname "$SCRIPT_DIR")/mcp-server"
  exit 1
}

# Check node is available
if ! command -v node &>/dev/null; then
  err "Node.js is required to build the MCP server."
  err "  macOS:  brew install node"
  err "  Ubuntu: apt-get install nodejs npm"
  exit 1
fi
if ! command -v npm &>/dev/null; then
  err "npm is required to build the MCP server."
  exit 1
fi

# Build the MCP server
info "Installing MCP server dependencies and building..."
(cd "$MCP_SERVER_DIR" && npm install --silent && npm run build --silent) || {
  err "MCP server build failed. Check Node.js version (requires >=20)."
  exit 1
}
ok "MCP server built: $MCP_SERVER_DIR/dist/"

# Make start scripts executable
chmod +x "$MCP_SERVER_DIR/start-gemini.sh" "$MCP_SERVER_DIR/start-codex.sh"
ok "Start scripts ready"

# ── Wire Claude: add both servers to ~/.claude.json ──────────
CLAUDE_JSON="$HOME/.claude.json"
GEMINI_START="$MCP_SERVER_DIR/start-gemini.sh"
CODEX_START="$MCP_SERVER_DIR/start-codex.sh"

if [[ -f "$CLAUDE_JSON" ]]; then
  backup_if_exists "$CLAUDE_JSON"
  # Merge mcpServers into existing config (preserves all other keys)
  jq --arg gs "$GEMINI_START" --arg cs "$CODEX_START" '
    .mcpServers["inter-agent-gemini"] = {"command": $gs} |
    .mcpServers["inter-agent-codex"]  = {"command": $cs}
  ' "$CLAUDE_JSON" > "${CLAUDE_JSON}.tmp" && mv "${CLAUDE_JSON}.tmp" "$CLAUDE_JSON"
else
  jq -n --arg gs "$GEMINI_START" --arg cs "$CODEX_START" '{
    mcpServers: {
      "inter-agent-gemini": {command: $gs},
      "inter-agent-codex":  {command: $cs}
    }
  }' > "$CLAUDE_JSON"
fi
ok "Claude wired: inter-agent-gemini + inter-agent-codex → $CLAUDE_JSON"

# ── Wire Gemini: add inter-agent-codex to ~/.gemini/settings.json ──
GEMINI_SETTINGS="$HOME/.gemini/settings.json"
if [[ -f "$GEMINI_SETTINGS" ]]; then
  backup_if_exists "$GEMINI_SETTINGS"
  jq --arg cs "$CODEX_START" '
    .mcpServers["inter-agent-codex"] = {"command": $cs}
  ' "$GEMINI_SETTINGS" > "${GEMINI_SETTINGS}.tmp" && mv "${GEMINI_SETTINGS}.tmp" "$GEMINI_SETTINGS"
  ok "Gemini wired: inter-agent-codex → $GEMINI_SETTINGS"
else
  warn "~/.gemini/settings.json not found — Gemini MCP not configured."
  warn "After installing Gemini CLI, add manually:"
  warn "  {mcpServers: {\"inter-agent-codex\": {command: \"$CODEX_START\"}}}"
fi

# ── Wire Codex: uncomment and set inter-agent-gemini in config.toml ──
CODEX_CONFIG="$HOME/.codex/config.toml"
if [[ -f "$CODEX_CONFIG" ]]; then
  # Check if MCP server is already configured
  if grep -q "inter-agent-gemini" "$CODEX_CONFIG" && ! grep -q "^#.*inter-agent-gemini" "$CODEX_CONFIG"; then
    info "Codex config.toml already has inter-agent-gemini — skipping."
  else
    backup_if_exists "$CODEX_CONFIG"
    # Append the MCP server config block
    cat >> "$CODEX_CONFIG" <<EOF

# ── Inter-agent MCP server (added by Triumvirate installer) ──────────
[mcp_servers.inter-agent-gemini]
command = "$GEMINI_START"
EOF
    ok "Codex wired: inter-agent-gemini → $CODEX_CONFIG"
  fi
else
  warn "~/.codex/config.toml not found — Codex MCP not configured."
fi

ok "Inter-agent MCP server wired into all 3 agents"

# ── 6. Shared Templates ──────────────────────────────────────
info "Installing shared templates..."

# .env.example — copy to ~/.claude/ as reference (not as .env)
if [[ ! -f "$HOME/.claude/.env" ]]; then
  cp "$SCRIPT_DIR/shared/.env.example" "$HOME/.claude/.env.example"
  ok "Copied .env.example to ~/.claude/.env.example (rename to .env and fill in your keys)"
else
  info ".env already exists — skipping .env.example copy"
fi

# taxonomy.json.example — copy to ~/.claude/ as reference
cp "$SCRIPT_DIR/shared/taxonomy.json.example" "$HOME/.claude/taxonomy.json.example"
ok "Copied taxonomy.json.example to ~/.claude/"

# ── 7. AI Memory Directory ──────────────────────────────────
# Central session log store — all agents write here when it exists.
# This is a git repo so session logs are versioned and shareable.
AI_MEM_DIR="$HOME/.ai-memory"
if [[ -d "$AI_MEM_DIR" ]]; then
  info "AI memory directory already exists: $AI_MEM_DIR"
else
  info "Creating AI memory directory..."
  mkdir -p "$AI_MEM_DIR"
  (cd "$AI_MEM_DIR" && git init --quiet)
  ok "Created $AI_MEM_DIR (git-initialized)"
  info "Session logs will be stored here when projects have a taxonomy.json"
fi

# ── 8. Verify ─────────────────────────────────────────────────
echo ""
info "Verifying installation..."
ISSUES=0

# Check hooks are executable
for hook in "$CLAUDE_HOOKS_DIR"/*.sh; do
  [[ -f "$hook" ]] || continue
  if [[ ! -x "$hook" ]]; then
    warn "Hook not executable: $hook"
    chmod +x "$hook"
    ISSUES=$((ISSUES + 1))
  fi
done

# Check gemini CLI (optional, for pre-compact summarization)
if ! command -v gemini &>/dev/null; then
  info "Gemini CLI not found — pre-compact will use jq fallback for summarization"
fi

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                    Installation Complete!                    ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""
echo "  Claude hooks:     $CLAUDE_HOOKS_DIR/"
echo "  Claude MCP:       $HOME/.claude.json  (inter-agent-gemini + inter-agent-codex)"
echo "  Codex config:     $CODEX_DIR/"
echo "  Gemini config:    $GEMINI_DIR/"
echo "  MCP server:       $MCP_SERVER_DIR/dist/"
echo ""
echo "  Next steps:"
echo "    1. Copy ~/.claude/.env.example to ~/.claude/.env"
echo "       Fill in your API keys (at minimum: GEMINI_API_KEY)"
echo "    2. Uncomment the .env sourcing block in ~/.claude/hooks/session-start.sh"
echo "    3. Create your first project (must be a git repo):"
echo "       mkdir -p ~/projects/my-project/.claude"
echo "       cd ~/projects/my-project && git init"
echo "       cp ~/.claude/taxonomy.json.example .claude/taxonomy.json"
echo "       # Edit taxonomy.json with your project details"
echo "       git add .claude/taxonomy.json && git commit -m 'init: add taxonomy'"
echo "    4. Start all three agents:"
echo "       claude        # Primary — hooks auto-load, MCP connects to Gemini + Codex"
echo "       gemini        # Research + analysis — can daemon-call Codex"
echo "       codex         # Code generation — can daemon-call Gemini"
echo ""

if [[ "$ISSUES" -gt 0 ]]; then
  warn "$ISSUES issue(s) found — see warnings above."
fi
