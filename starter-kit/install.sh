#!/bin/bash
# ============================================================================
# Triumvirate Starter Kit — Installer
#
# Builds and wires the complete Triumvirate operating environment:
#   daemon/              — Builds the Rust daemon (cargo build --release)
#   ~/.claude/hooks/     — Claude Code hooks (session lifecycle + safety gates)
#   ~/.claude/           — Claude Code settings and instructions
#   ~/.claude.json       — MCP server registration (triumvirate daemon)
#   ~/.codex/            — Codex CLI config, hooks, and skills
#   ~/.codex/config.toml — MCP server registration (triumvirate daemon)
#   ~/.gemini/           — Gemini CLI instructions
#   ~/.gemini/settings.json — MCP server registration (triumvirate daemon)
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

# ── 1b. Claude Skills ──────────────────────────────────────
info "Installing Claude skills..."
SKILLS_SRC="$SCRIPT_DIR/claude/skills"
SKILLS_DST="$HOME/.claude/skills"

if [[ -d "$SKILLS_SRC" ]]; then
  mkdir -p "$SKILLS_DST"
  for skill_dir in "$SKILLS_SRC"/*/; do
    [[ -d "$skill_dir" ]] || continue
    skill_name="$(basename "$skill_dir")"
    if [[ -d "$SKILLS_DST/$skill_name" ]]; then
      warn "Skill '$skill_name' already exists — skipping (won't overwrite)"
    else
      cp -r "$skill_dir" "$SKILLS_DST/$skill_name"
      ok "Installed skill: $skill_name"
    fi
  done
  # Also install standalone skill files (not in subdirectories)
  for skill_file in "$SKILLS_SRC"/*.md; do
    [[ -f "$skill_file" ]] || continue
    skill_name="$(basename "$skill_file")"
    install_file "$skill_file" "$SKILLS_DST/$skill_name"
  done
  ok "Claude skills installed"
fi

# ── 1b2. Claude Commands ──────────────────────────────────────
info "Installing Claude commands..."
CMDS_SRC="$SCRIPT_DIR/claude/commands"
CMDS_DST="$HOME/.claude/commands"

if [[ -d "$CMDS_SRC" ]]; then
  mkdir -p "$CMDS_DST"
  for cmd_file in "$CMDS_SRC"/*.md; do
    [[ -f "$cmd_file" ]] || continue
    cmd_name="$(basename "$cmd_file")"
    install_file "$cmd_file" "$CMDS_DST/$cmd_name"
  done
  ok "Claude commands installed ($(ls "$CMDS_SRC"/*.md 2>/dev/null | wc -l | tr -d ' ') files)"
fi

# ── 1c. Claude Rules + Lessons ──────────────────────────────
info "Installing Claude rules and lessons templates..."
RULES_SRC="$SCRIPT_DIR/claude/rules"
RULES_DST="$HOME/.claude/rules"
if [[ -d "$RULES_SRC" ]]; then
  mkdir -p "$RULES_DST"
  for rule in "$RULES_SRC"/*.md; do
    [[ -f "$rule" ]] || continue
    rule_name="$(basename "$rule")"
    if [[ -f "$RULES_DST/$rule_name" ]]; then
      warn "Rule '$rule_name' already exists — skipping"
    else
      cp "$rule" "$RULES_DST/$rule_name"
      ok "Installed rule: $rule_name"
    fi
  done
fi

LESSONS_DST="$HOME/.claude/lessons"
if [[ ! -d "$LESSONS_DST" ]]; then
  mkdir -p "$LESSONS_DST"
  cp "$SCRIPT_DIR/claude/lessons/TEMPLATE.md" "$LESSONS_DST/TEMPLATE.md"
  ok "Created lessons directory with template"
fi

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

# ── 5. Triumvirate Daemon ────────────────────────────────────────
# The Rust daemon is the coordination layer. It provides MCP tools
# (spawn_session, ask_session, dismiss_session, etc.) that let any
# agent talk to any other agent with persistent sessions.
info "Setting up Triumvirate daemon..."

DAEMON_DIR="$(cd "$SCRIPT_DIR/../daemon" 2>/dev/null && pwd)" || {
  err "Cannot find daemon/ directory. Make sure you cloned the full triumvirate repo."
  err "Expected: $(dirname "$SCRIPT_DIR")/daemon"
  exit 1
}

DAEMON_BIN="$DAEMON_DIR/target/release/triumvirate"

# Build if not already built
if [[ -f "$DAEMON_BIN" ]]; then
  ok "Daemon already built: $DAEMON_BIN"
else
  if ! command -v cargo &>/dev/null; then
    err "Rust/Cargo is required to build the daemon."
    err "  Install: https://rustup.rs"
    err "  Then re-run this installer."
    exit 1
  fi
  info "Building daemon (this may take a minute)..."
  (cd "$DAEMON_DIR" && cargo build --release 2>&1) || {
    err "Daemon build failed. Check Rust version (requires 1.82+)."
    exit 1
  }
  ok "Daemon built: $DAEMON_BIN"
fi

# ── Wire Claude: register triumvirate as MCP server ──────────
CLAUDE_JSON="$HOME/.claude.json"

if [[ -f "$CLAUDE_JSON" ]]; then
  if jq -e '.mcpServers.triumvirate' "$CLAUDE_JSON" &>/dev/null; then
    info "Triumvirate already registered in ~/.claude.json — updating path"
  fi
  backup_if_exists "$CLAUDE_JSON"
  jq --arg bin "$DAEMON_BIN" '
    .mcpServers //= {} |
    .mcpServers.triumvirate = {command: $bin}
  ' "$CLAUDE_JSON" > "${CLAUDE_JSON}.tmp" && mv "${CLAUDE_JSON}.tmp" "$CLAUDE_JSON"
else
  jq -n --arg bin "$DAEMON_BIN" '{
    mcpServers: {
      triumvirate: {command: $bin}
    }
  }' > "$CLAUDE_JSON"
fi
ok "Claude wired: triumvirate → $CLAUDE_JSON"

# ── Wire Gemini: add triumvirate to ~/.gemini/settings.json ──
GEMINI_SETTINGS="$HOME/.gemini/settings.json"
if [[ -f "$GEMINI_SETTINGS" ]]; then
  backup_if_exists "$GEMINI_SETTINGS"
  jq --arg bin "$DAEMON_BIN" '
    .mcpServers //= {} |
    .mcpServers.triumvirate = {command: $bin}
  ' "$GEMINI_SETTINGS" > "${GEMINI_SETTINGS}.tmp" && mv "${GEMINI_SETTINGS}.tmp" "$GEMINI_SETTINGS"
  ok "Gemini wired: triumvirate → $GEMINI_SETTINGS"
else
  warn "~/.gemini/settings.json not found — Gemini MCP not configured."
  warn "After installing Gemini CLI, add manually:"
  warn "  {mcpServers: {\"triumvirate\": {command: \"$DAEMON_BIN\"}}}"
fi

# ── Wire Codex: add triumvirate to config.toml ──
CODEX_CONFIG="$HOME/.codex/config.toml"
if [[ -f "$CODEX_CONFIG" ]]; then
  if grep -q "triumvirate" "$CODEX_CONFIG" && ! grep -q "^#.*triumvirate" "$CODEX_CONFIG"; then
    info "Codex config.toml already has triumvirate — skipping."
  else
    backup_if_exists "$CODEX_CONFIG"
    cat >> "$CODEX_CONFIG" <<EOF

# ── Triumvirate daemon (added by Triumvirate installer) ──────────
[mcp_servers.triumvirate]
command = "$DAEMON_BIN"
EOF
    ok "Codex wired: triumvirate → $CODEX_CONFIG"
  fi
else
  warn "~/.codex/config.toml not found — Codex MCP not configured."
fi

ok "Triumvirate daemon wired into all detected agents"

# ── 6. Stenographer (Local Session Notes) ─────────────────────
info "Installing Stenographer (local Ollama session notes)..."
STENO_DIR="$HOME/.triumvirate/stenographer"
mkdir -p "$STENO_DIR/parsers" "$STENO_DIR/prompts"

# Core files
install_file "$SCRIPT_DIR/stenographer/stenographer.py" "$STENO_DIR/stenographer.py"
for parser in "$SCRIPT_DIR"/stenographer/parsers/*.py; do
  [[ -f "$parser" ]] || continue
  install_file "$parser" "$STENO_DIR/parsers/$(basename "$parser")"
done
for prompt in "$SCRIPT_DIR"/stenographer/prompts/*.txt; do
  [[ -f "$prompt" ]] || continue
  install_file "$prompt" "$STENO_DIR/prompts/$(basename "$prompt")"
done

# Utility scripts (session_log_path, gap_fill, health_check, workers)
for util in "$SCRIPT_DIR"/stenographer/*.py; do
  [[ -f "$util" ]] || continue
  util_name="$(basename "$util")"
  # Skip parsers (handled above) and __pycache__
  [[ "$util_name" == "__"* ]] && continue
  install_file "$util" "$STENO_DIR/$util_name"
done

# Create state and log directories
mkdir -p "$HOME/.triumvirate/locks"
chmod +x "$STENO_DIR/stenographer.py"
[[ -f "$STENO_DIR/session-save-ctl.py" ]] && chmod +x "$STENO_DIR/session-save-ctl.py"
[[ -f "$STENO_DIR/session-save-worker.py" ]] && chmod +x "$STENO_DIR/session-save-worker.py"
ok "Stenographer installed: $STENO_DIR/"

# Check Ollama — powers the free background note-taker (Stenographer)
if command -v ollama &>/dev/null; then
  if ollama list 2>/dev/null | grep -q "qwen2.5"; then
    ok "Ollama found with qwen2.5 model — background note-taker ready"
  else
    echo ""
    echo "  Ollama is installed but needs an AI model to take session notes."
    echo "  This is a one-time download that runs entirely on your computer (free)."
    echo ""
    echo "  Model sizes:"
    echo "    1. qwen2.5:3b   — 1.9 GB  (lightest, works on any machine)"
    echo "    2. qwen2.5:7b   — 4.4 GB  (recommended)"
    echo "    3. qwen2.5:14b  — 8.7 GB  (better quality, needs 16GB+ RAM)"
    echo "    4. Skip for now"
    echo ""
    read -p "  Which model? [2]: " model_choice
    case "${model_choice:-2}" in
      1) PULL_MODEL="qwen2.5:3b" ;;
      2) PULL_MODEL="qwen2.5:7b" ;;
      3) PULL_MODEL="qwen2.5:14b" ;;
      *) PULL_MODEL="" ;;
    esac
    if [[ -n "$PULL_MODEL" ]]; then
      info "Downloading $PULL_MODEL (this may take a few minutes)..."
      ollama pull "$PULL_MODEL" && \
        ok "Model ready: $PULL_MODEL — background note-taker is active" || \
        warn "Download failed — you can try later with: ollama pull $PULL_MODEL"
    else
      info "Skipped — background note-taker will be inactive until you pull a model"
      info "  Run later: ollama pull qwen2.5:7b"
    fi
  fi
else
  echo ""
  info "Ollama not found."
  echo "  Ollama is a free, local AI that powers the background note-taker."
  echo "  Without it, your AI assistants still save notes at key moments,"
  echo "  but you miss the automatic mid-session saves."
  echo ""
  echo "  To install later:"
  echo "    macOS:  brew install ollama"
  echo "    Linux:  curl -fsSL https://ollama.ai/install.sh | sh"
  echo "    Then:   ollama pull qwen2.5:7b"
fi

# ── 7. Shared Templates ──────────────────────────────────────
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

# ── 8. AI Memory (session notes storage) ─────────────────────
# This is where your AI assistants save their notes between sessions.
# Think of it as their shared notebook — all three agents read and write here.
AI_MEM_DIR="$HOME/.ai-memory"
if [[ -d "$AI_MEM_DIR" ]]; then
  info "AI memory directory already exists: $AI_MEM_DIR"
else
  echo ""
  echo "  Triumvirate saves session notes so your AI assistants remember"
  echo "  what you worked on yesterday (or last week, or last month)."
  echo ""
  read -p "  Create AI memory folder at ~/.ai-memory? [Y/n]: " create_memory
  if [[ ! "$create_memory" =~ ^[Nn] ]]; then
    mkdir -p "$AI_MEM_DIR"
    (cd "$AI_MEM_DIR" && git init --quiet)
    ok "Created AI memory folder: $AI_MEM_DIR"

    # Offer to back it up to GitHub (private)
    if command -v gh &>/dev/null; then
      echo ""
      echo "  Optional: back up your AI memory to a private GitHub repo."
      echo "  This lets you access your session notes from other computers"
      echo "  and keeps them safe if your hard drive dies."
      echo ""
      read -p "  Create private GitHub repo for AI memory? [y/N]: " create_remote
      if [[ "$create_remote" =~ ^[Yy] ]]; then
        (cd "$AI_MEM_DIR" && \
          gh repo create ai-memory --private -y 2>/dev/null && \
          git remote add origin "$(gh repo view ai-memory --json sshUrl -q .sshUrl)" 2>/dev/null && \
          git add -A && git commit -m "init: AI memory" --quiet 2>/dev/null && \
          git push -u origin main --quiet 2>/dev/null) && \
          ok "AI memory backed up to private GitHub repo" || \
          warn "Couldn't create GitHub repo — memory will be local only (still works fine)"
      fi
    fi
  else
    warn "Skipped AI memory — session notes will be saved in each project's session-logs/ folder"
  fi
fi

# ── 8b. Project directory ────────────────────────────────────
echo ""
echo "  Where do you keep your projects?"
echo "  (The AI uses this to find your work and organize session notes)"
echo ""
echo "  Common choices:"
echo "    1. ~/projects/      (default)"
echo "    2. ~/Documents/"
echo "    3. ~/Desktop/"
echo "    4. Custom path"
echo ""
read -p "  Enter 1-4 or a custom path [1]: " project_choice
case "${project_choice:-1}" in
  1) PROJECTS_DIR="$HOME/projects" ;;
  2) PROJECTS_DIR="$HOME/Documents" ;;
  3) PROJECTS_DIR="$HOME/Desktop" ;;
  4|*)
    if [[ "$project_choice" =~ ^/ || "$project_choice" =~ ^~ ]]; then
      PROJECTS_DIR="${project_choice/#\~/$HOME}"
    else
      PROJECTS_DIR="$HOME/projects"
    fi
    ;;
esac
mkdir -p "$PROJECTS_DIR"
ok "Projects directory: $PROJECTS_DIR"

# Save to .env so session-start.sh can find it
ENV_FILE="$HOME/.claude/.env"
[[ ! -f "$ENV_FILE" ]] && touch "$ENV_FILE"
if ! grep -q "TRIUMVIRATE_PROJECTS_DIR" "$ENV_FILE"; then
  echo "" >> "$ENV_FILE"
  echo "# Where your projects live — used by session-start to find your work" >> "$ENV_FILE"
  echo "TRIUMVIRATE_PROJECTS_DIR=\"$PROJECTS_DIR\"" >> "$ENV_FILE"
fi
info "When you start Claude, it will look here for your work"

# ── 8c. Beginner mode (auto-save) ───────────────────────────
echo ""
echo "  How experienced are you with git (version control)?"
echo ""
echo "    1. I don't know what git is  → BEGINNER MODE (auto-saves everything)"
echo "    2. I know the basics         → STANDARD MODE (you control when to save)"
echo "    3. I'm experienced           → STANDARD MODE (full manual control)"
echo ""
read -p "  Enter 1-3 [1]: " git_experience
case "${git_experience:-1}" in
  1)
    # Add auto-commit/push to .env
    ENV_FILE="$HOME/.claude/.env"
    if [[ -f "$ENV_FILE" ]]; then
      if ! grep -q "TRIUMVIRATE_AUTO_COMMIT" "$ENV_FILE"; then
        echo "" >> "$ENV_FILE"
        echo "# Beginner mode — automatically saves your work after every change" >> "$ENV_FILE"
        echo "TRIUMVIRATE_AUTO_COMMIT=1" >> "$ENV_FILE"
        echo "TRIUMVIRATE_AUTO_PUSH=1" >> "$ENV_FILE"
      fi
    else
      cat > "$ENV_FILE" <<'ENVEOF'
# Beginner mode — automatically saves your work after every change
TRIUMVIRATE_AUTO_COMMIT=1
TRIUMVIRATE_AUTO_PUSH=1
ENVEOF
    fi
    ok "Beginner mode ON — your work is auto-saved after every change"
    info "You'll also get reminders to back up to GitHub"
    ;;
  *)
    ok "Standard mode — you control when to save (commit) and back up (push)"
    ;;
esac

# ── 8d. Claude subscription tier ──────────────────────────────
echo ""
echo "  What Claude subscription do you have?"
echo "  (This adjusts how often session notes are saved)"
echo ""
echo "    1. Claude Pro ($20/mo)          — saves every ~50K tokens"
echo "    2. Claude Max 5x ($100/mo)       — saves every ~100K tokens"
echo "    3. Claude Max 20x ($200/mo)     — saves every ~200K tokens"
echo "    4. API key (pay-per-use)        — saves every ~50K tokens"
echo "    5. I'm not sure                 — uses safe defaults"
echo ""
read -p "  Enter 1-5 [5]: " sub_choice
ENV_FILE="$HOME/.claude/.env"
[[ ! -f "$ENV_FILE" ]] && touch "$ENV_FILE"
case "${sub_choice:-5}" in
  1|4|5)
    # Pro / API / unsure: conservative threshold, check often
    if ! grep -q "TOKEN_GATE_THRESHOLD_KB" "$ENV_FILE"; then
      echo "" >> "$ENV_FILE"
      echo "# Claude Pro / API: moderate pace, check every 15 tool calls, save at ~50K tokens" >> "$ENV_FILE"
      echo "TOKEN_GATE_THRESHOLD_KB=200" >> "$ENV_FILE"
      echo "TOKEN_GATE_CHECK_EVERY_N=15" >> "$ENV_FILE"
      echo "TOKEN_GATE_COOLDOWN_SECS=300" >> "$ENV_FILE"
    fi
    ok "Tuned for Pro tier: notes every ~50K tokens, hooks check every 15 calls"
    ;;
  2)
    # Max 5x: faster output, check less often to reduce hook overhead
    if ! grep -q "TOKEN_GATE_THRESHOLD_KB" "$ENV_FILE"; then
      echo "" >> "$ENV_FILE"
      echo "# Claude Max 5x: faster pace, check less often to reduce hook overhead" >> "$ENV_FILE"
      echo "TOKEN_GATE_THRESHOLD_KB=400" >> "$ENV_FILE"
      echo "TOKEN_GATE_CHECK_EVERY_N=25" >> "$ENV_FILE"
      echo "TOKEN_GATE_COOLDOWN_SECS=600" >> "$ENV_FILE"
    fi
    ok "Tuned for Max 5x: notes every ~100K tokens, hooks check every 25 calls"
    ;;
  3)
    # Max 20x: very fast, wider intervals to avoid Ollama/hook bottleneck
    if ! grep -q "TOKEN_GATE_THRESHOLD_KB" "$ENV_FILE"; then
      echo "" >> "$ENV_FILE"
      echo "# Claude Max 20x: very fast pace, wide intervals to avoid hook bottleneck" >> "$ENV_FILE"
      echo "TOKEN_GATE_THRESHOLD_KB=800" >> "$ENV_FILE"
      echo "TOKEN_GATE_CHECK_EVERY_N=40" >> "$ENV_FILE"
      echo "TOKEN_GATE_COOLDOWN_SECS=900" >> "$ENV_FILE"
    fi
    ok "Tuned for Max 20x: notes every ~200K tokens, hooks check every 40 calls"
    ;;
esac

# ── 9. Verify ─────────────────────────────────────────────────
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
echo "  Stenographer:     $STENO_DIR/"
echo ""
echo "  Next steps:"
echo ""
echo "    1. SET UP YOUR API KEYS"
echo "       cp ~/.claude/.env.example ~/.claude/.env"
echo "       Open ~/.claude/.env in a text editor and add your keys."
echo "       At minimum you need: GEMINI_API_KEY (free from Google AI Studio)"
echo "       See docs/getting-api-access.md for step-by-step instructions."
echo ""
echo "    2. INSTALL SUPERPOWERS (recommended)"
echo "       Superpowers is a plugin that gives Claude advanced skills"
echo "       like planning, debugging, and code review workflows."
echo "       Install it by running:"
echo "         claude /install-plugin https://github.com/anthropics/claude-code-plugins/tree/main/superpowers"
echo ""
echo "    3. START WORKING"
echo "       Just open your terminal and type:"
echo "         claude"
echo "       Claude will show you a project picker. Pick a project or"
echo "       tell it what you want to work on. Everything else is automatic."
echo ""
echo "    4. OPTIONAL: Enable long-term memory (Oracle Engine)"
echo "       cp $SCRIPT_DIR/claude/settings.local.json.example ~/.claude/settings.local.json"
echo "       This lets your AI remember things across weeks and months."
echo "       See docs/oracle-engine.md for details."
echo ""
echo "  Read docs/plain-english-guide.md if you want to understand"
echo "  what all this does — no jargon, just plain English."
echo ""

if [[ "$ISSUES" -gt 0 ]]; then
  warn "$ISSUES issue(s) found — see warnings above."
fi
