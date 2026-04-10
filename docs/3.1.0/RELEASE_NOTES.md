# Triumvirate 3.1.0 — MCP Consolidation

**Release date:** 2026-04-10

One daemon. All tools. No more TypeScript.

---

## What's New

The Rust `triumvirate` daemon is now the **only** MCP server you need. The legacy TypeScript inter-agent server has been retired. All 40+ tools — session management, ABE dispatch, fleet operations, knowledge (ledger/lessons/memory/scratchpad), peer review, and Gemini queries — run through a single binary.

Old tool names like `spawn_daemon` and `ask_daemon` still work via 10 backwards-compatible aliases. No existing skills or workflows break.

## Install / Upgrade

**New install:**
```bash
# Clone + build
git clone https://github.com/michaeljboscia/triumvirate.git
cd triumvirate/daemon
cargo build --release

# Binary is at target/release/triumvirate
./target/release/triumvirate --version
# → triumvirate 3.1.0
```

**Existing users — upgrade steps:**

1. Pull latest and rebuild:
   ```bash
   cd ~/projects/triumvirate && git pull && cd daemon && cargo build --release
   ```

2. Your `~/.claude.json` should already have a `"triumvirate"` MCP entry. If it also has an `"inter-agent"` entry, remove it:
   ```bash
   # Backup first
   cp ~/.claude.json ~/.claude.json.bak
   # Remove inter-agent (requires jq)
   jq 'del(.mcpServers["inter-agent"])' ~/.claude.json > /tmp/c.json && mv /tmp/c.json ~/.claude.json
   ```

3. Restart Claude Code. The daemon starts automatically via the `triumvirate mcp` bridge.

## Verify

```bash
triumvirate --version      # → triumvirate 3.1.0
triumvirate doctor         # → Triumvirate daemon v3.1.0 + health checks
```

From inside Claude Code:
```
mcp__triumvirate__ping     # → pong
mcp__triumvirate__daemon_health  # → {"status":"ok","version":"3.1.0"}
```

## Rollback

If something breaks:
```bash
cp ~/.claude.json.bak ~/.claude.json
```
This restores the inter-agent TS server entry. Both can coexist — having both entries is safe (they use different tool namespaces).

## Known Issues

- ABE worker completion detection is experimental. The daemon's per-task sentinel watcher doesn't auto-activate yet. Orchestrators should poll `.triumvirate/TASK_COMPLETE.json` directly.
- `cargo test --workspace` can hang on integration tests that spawn real agent subprocesses. Use `cargo test -p <crate>` for reliable test runs.

## Full Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for the complete list of changes, migration notes, and breaking changes.
