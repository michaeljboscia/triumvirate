#!/bin/bash
# ============================================================================
# Triumvirate — Interactive Installer
#
# Installs the multi-agent development system in tiers:
#
#   [1] Full Stack    — daemon + skills + operating environment + stenographer
#   [2] Daemon+Skills — daemon binary + goatrodeo/postrodeo methodology
#   [3] Daemon Only   — just the coordination daemon, register as MCP server
#   [4] Skills Only   — just the methodology skills (no build required)
#
# Safe to re-run. Backs up existing files before overwriting.
# ============================================================================

set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")" && pwd)"
DAEMON_DIR="$REPO_DIR/daemon"
SKILLS_DIR="$REPO_DIR/skills/claude"
STARTER_KIT_DIR="$REPO_DIR/starter-kit"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

info()    { printf "${BLUE}  [info]${NC}  %s\n" "$1"; }
ok()      { printf "${GREEN}  [  ok]${NC}  %s\n" "$1"; }
warn()    { printf "${YELLOW}  [warn]${NC}  %s\n" "$1"; }
err()     { printf "${RED}  [ err]${NC}  %s\n" "$1"; }
step()    { printf "\n${BOLD}  %s${NC}\n\n" "$1"; }

# ── Banner ───────────────────────────────────────────────────────
echo ""
echo "  ╔═══════════════════════════════════════════════════════════╗"
echo "  ║                                                           ║"
echo "  ║   Triumvirate — Multi-Agent Development System            ║"
echo "  ║                                                           ║"
echo "  ║   Claude + Gemini + Codex, coordinated by one daemon.     ║"
echo "  ║                                                           ║"
echo "  ╚═══════════════════════════════════════════════════════════╝"
echo ""

# ── Detect Environment ───────────────────────────────────────────
step "Detecting your environment..."

HAS_RUST=false
HAS_CARGO=false
HAS_CLAUDE=false
HAS_GEMINI=false
HAS_CODEX=false
HAS_NODE=false
HAS_OLLAMA=false
HAS_JQ=false

command -v rustc  &>/dev/null && HAS_RUST=true
command -v cargo  &>/dev/null && HAS_CARGO=true
command -v claude &>/dev/null && HAS_CLAUDE=true
command -v gemini &>/dev/null && HAS_GEMINI=true
command -v codex  &>/dev/null && HAS_CODEX=true
command -v node   &>/dev/null && HAS_NODE=true
command -v ollama &>/dev/null && HAS_OLLAMA=true
command -v jq     &>/dev/null && HAS_JQ=true

RUST_VERSION=""
if $HAS_RUST; then
  RUST_VERSION=$(rustc --version 2>/dev/null | awk '{print $2}')
fi

# Display results
$HAS_RUST    && ok "Rust $RUST_VERSION"          || warn "Rust — not found"
$HAS_CARGO   && ok "Cargo"                        || warn "Cargo — not found"
$HAS_CLAUDE  && ok "Claude Code"                  || warn "Claude Code — not found"
$HAS_GEMINI  && ok "Gemini CLI"                   || warn "Gemini CLI — not found"
$HAS_CODEX   && ok "Codex CLI"                    || warn "Codex CLI — not found"
$HAS_NODE    && ok "Node.js"                      || warn "Node.js — not found"
$HAS_OLLAMA  && ok "Ollama"                       || warn "Ollama — not found (optional)"
$HAS_JQ      && ok "jq"                           || warn "jq — not found"

AGENT_COUNT=0
$HAS_CLAUDE && AGENT_COUNT=$((AGENT_COUNT + 1))
$HAS_GEMINI && AGENT_COUNT=$((AGENT_COUNT + 1))
$HAS_CODEX  && AGENT_COUNT=$((AGENT_COUNT + 1))

echo ""
if [[ $AGENT_COUNT -eq 0 ]]; then
  warn "No agent CLIs detected. You need at least one:"
  echo "    Claude:  https://docs.anthropic.com/en/docs/claude-code"
  echo "    Gemini:  https://github.com/google-gemini/gemini-cli"
  echo "    Codex:   https://github.com/openai/codex"
  echo ""
  read -p "  Continue anyway? [y/N]: " continue_anyway
  [[ "$continue_anyway" =~ ^[Yy] ]] || exit 0
else
  ok "$AGENT_COUNT agent CLI(s) detected"
fi

# ── Choose Installation Tier ─────────────────────────────────────
step "What would you like to install?"

echo "    ${BOLD}1. Full Stack${NC} ${DIM}(recommended)${NC}"
echo "       Daemon + methodology skills + operating environment + stenographer"
echo "       Everything you need for multi-agent development."
echo ""
echo "    ${BOLD}2. Daemon + Skills${NC}"
echo "       The coordination daemon + goatrodeo/postrodeo methodology."
echo "       No hooks, no stenographer, no agent configs."
echo ""
echo "    ${BOLD}3. Daemon Only${NC}"
echo "       Just the Rust binary registered as an MCP server."
echo "       You handle everything else."
echo ""
echo "    ${BOLD}4. Skills Only${NC} ${DIM}(no build required)${NC}"
echo "       Just the goatrodeo/postrodeo skills for Claude."
echo "       No daemon, no build step."
echo ""

read -p "  Choose [1]: " tier_choice
TIER="${tier_choice:-1}"

echo ""

# ── Validate prerequisites for chosen tier ───────────────────────
case "$TIER" in
  1|2|3)
    if ! $HAS_CARGO; then
      err "Rust/Cargo is required to build the daemon."
      echo "    Install: https://rustup.rs"
      echo "    Then re-run this installer."
      exit 1
    fi
    ;;
  4)
    # Skills only — no build needed
    ;;
  *)
    err "Invalid choice. Run the installer again."
    exit 1
    ;;
esac

if [[ "$TIER" == "1" ]] && ! $HAS_JQ; then
  err "jq is required for the full stack install (MCP wiring)."
  echo "    macOS:  brew install jq"
  echo "    Linux:  apt-get install jq"
  exit 1
fi

# ════════════════════════════════════════════════════════════════
# TIER EXECUTION
# ════════════════════════════════════════════════════════════════

DAEMON_BIN=""

# ── Build Daemon (tiers 1, 2, 3) ────────────────────────────────
if [[ "$TIER" =~ ^[123]$ ]]; then
  step "Building the daemon..."

  if [[ ! -f "$DAEMON_DIR/Cargo.toml" ]]; then
    err "Cannot find daemon/Cargo.toml — is this the triumvirate repo?"
    exit 1
  fi

  (cd "$DAEMON_DIR" && cargo build --release 2>&1) || {
    err "Build failed. Check Rust version (requires 1.82+)."
    exit 1
  }

  DAEMON_BIN="$DAEMON_DIR/target/release/triumvirate"
  if [[ ! -f "$DAEMON_BIN" ]]; then
    err "Build succeeded but binary not found at $DAEMON_BIN"
    exit 1
  fi

  ok "Daemon built: $DAEMON_BIN"

  # Register as MCP server in Claude
  step "Registering daemon as MCP server..."

  CLAUDE_JSON="$HOME/.claude.json"

  if $HAS_JQ; then
    if [[ -f "$CLAUDE_JSON" ]]; then
      # Check if triumvirate is already registered
      if jq -e '.mcpServers.triumvirate' "$CLAUDE_JSON" &>/dev/null; then
        info "Triumvirate already registered in ~/.claude.json — updating path"
      fi
      # Merge into existing config
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
    ok "Registered: triumvirate → ~/.claude.json"
  else
    warn "jq not available — add this to ~/.claude.json manually:"
    echo ""
    echo "    {\"mcpServers\": {\"triumvirate\": {\"command\": \"$DAEMON_BIN\"}}}"
    echo ""
  fi
fi

# ── Install Skills (tiers 1, 2, 4) ──────────────────────────────
if [[ "$TIER" =~ ^[124]$ ]]; then
  step "Installing methodology skills..."

  SKILLS_DST="$HOME/.claude/skills"
  CMDS_DST="$HOME/.claude/commands"
  mkdir -p "$SKILLS_DST" "$CMDS_DST"

  # Skills
  for skill in "$SKILLS_DIR"/goatrodeo.md "$SKILLS_DIR"/postrodeo.md "$SKILLS_DIR"/design-goatrodeo.md; do
    [[ -f "$skill" ]] || continue
    name="$(basename "$skill")"
    cp "$skill" "$SKILLS_DST/$name"
    ok "Skill: $name → $SKILLS_DST/"
  done

  # Command metadata
  for cmd in "$SKILLS_DIR"/goatrodeo-command.md "$SKILLS_DIR"/postrodeo-command.md "$SKILLS_DIR"/design-goatrodeo-command.md; do
    [[ -f "$cmd" ]] || continue
    # Strip "-command" from filename for the commands dir
    name="$(basename "$cmd" | sed 's/-command//')"
    cp "$cmd" "$CMDS_DST/$name"
    ok "Command: $name → $CMDS_DST/"
  done

  ok "Methodology skills installed"
fi

# ── Full Stack: Run Starter Kit (tier 1 only) ────────────────────
if [[ "$TIER" == "1" ]]; then
  step "Setting up the operating environment..."

  echo "  The starter kit will now configure hooks, agent configs,"
  echo "  stenographer, and MCP wiring for all your agents."
  echo ""
  read -p "  Continue? [Y/n]: " continue_starter
  if [[ ! "$continue_starter" =~ ^[Nn] ]]; then
    if [[ -x "$STARTER_KIT_DIR/install.sh" ]]; then
      (cd "$STARTER_KIT_DIR" && bash install.sh)
    else
      err "Starter kit not found at $STARTER_KIT_DIR/install.sh"
      warn "You can run it manually later: cd starter-kit && ./install.sh"
    fi
  else
    info "Skipped — run later with: cd starter-kit && ./install.sh"
  fi
fi

# ════════════════════════════════════════════════════════════════
# SUMMARY
# ════════════════════════════════════════════════════════════════

echo ""
echo "  ╔═══════════════════════════════════════════════════════════╗"
echo "  ║                  Installation Complete                    ║"
echo "  ╚═══════════════════════════════════════════════════════════╝"
echo ""

case "$TIER" in
  1)
    echo "  ${BOLD}Installed: Full Stack${NC}"
    echo ""
    echo "    Daemon:        $DAEMON_BIN"
    echo "    MCP server:    registered in ~/.claude.json"
    echo "    Skills:        ~/.claude/skills/{goatrodeo,postrodeo,design-goatrodeo}.md"
    echo "    Environment:   hooks, configs, stenographer (see starter-kit output above)"
    echo ""
    echo "  ${BOLD}Get started:${NC}"
    echo ""
    echo "    Open Claude and try:"
    echo "      \"spawn a Gemini session called research\""
    echo "      \"ask research to summarize this repo\""
    echo ""
    echo "    Run a goatrodeo:"
    echo "      /goatrodeo path/to/your/spec.md"
    ;;
  2)
    echo "  ${BOLD}Installed: Daemon + Skills${NC}"
    echo ""
    echo "    Daemon:        $DAEMON_BIN"
    echo "    MCP server:    registered in ~/.claude.json"
    echo "    Skills:        ~/.claude/skills/{goatrodeo,postrodeo,design-goatrodeo}.md"
    echo ""
    echo "  ${BOLD}Get started:${NC}"
    echo ""
    echo "    Open Claude and try:"
    echo "      \"spawn a Gemini session called research\""
    echo ""
    echo "    Want hooks + stenographer later?"
    echo "      cd starter-kit && ./install.sh"
    ;;
  3)
    echo "  ${BOLD}Installed: Daemon Only${NC}"
    echo ""
    echo "    Daemon:        $DAEMON_BIN"
    echo "    MCP server:    registered in ~/.claude.json"
    echo ""
    echo "  ${BOLD}Get started:${NC}"
    echo ""
    echo "    Open Claude and try:"
    echo "      \"spawn a Gemini session called research\""
    echo ""
    echo "    Want skills?"
    echo "      cp skills/claude/*.md ~/.claude/skills/"
    echo ""
    echo "    Want the full operating environment?"
    echo "      cd starter-kit && ./install.sh"
    ;;
  4)
    echo "  ${BOLD}Installed: Skills Only${NC}"
    echo ""
    echo "    Skills:        ~/.claude/skills/{goatrodeo,postrodeo,design-goatrodeo}.md"
    echo ""
    echo "  ${BOLD}Get started:${NC}"
    echo ""
    echo "    Open Claude and try:"
    echo "      /goatrodeo path/to/your/spec.md"
    echo ""
    echo "    Note: Without the daemon, goatrodeo will use Claude's"
    echo "    built-in Agent tool instead of persistent daemon sessions."
    echo ""
    echo "    Want the daemon?"
    echo "      ./install.sh   (choose option 2 or 3)"
    ;;
esac

echo ""
echo "  ${DIM}Docs:       docs/plain-english-guide.md${NC}"
echo "  ${DIM}Roadmap:    ROADMAP.md${NC}"
echo "  ${DIM}Issues:     https://github.com/michaeljboscia/triumvirate/issues${NC}"
echo ""
