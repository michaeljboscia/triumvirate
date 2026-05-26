# DESIGN_SYSTEM — N/A for v1

This integration is backend-only (a Rust daemon agent + an MCP tool surface). There is no
UI, no rendered output, no colors/typography/spacing/shadows.

The operator's UI is whatever surface they already use to talk to Claude (Claude Code TUI,
Claude desktop, etc.) — Triumvirate does not own that surface, it returns text through
`ask_agent`.

If a future expansion adds a UI (e.g. a dedicated DeepSeek-CoT viewer for the captured
`reasoning_content` logs), this doc gains real content.
