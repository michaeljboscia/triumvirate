# Research 024: BubbleTea TUI for 4-Way Agent Collaboration

**Confirmed:** BubbleTea can handle multi-pane concurrent streaming.

## Key Facts
- Elm-inspired Model/Update/View architecture
- Composable child models for split panes
- Community `panes` component for selectable pane layouts
- Commands execute in goroutines (non-blocking I/O)
- Cell-based renderer — only updates changed portions (minimal flicker)
- `split-editors` example shows multi-pane with focus switching

## Architecture for Triumvirate TUI
```
┌─────────────────┬─────────────────┐
│ Claude (Opus)   │ Gemini (Pro)    │
│ streaming...    │ challenging...  │
│                 │                 │
├─────────────────┼─────────────────┤
│ Codex (GPT-5.2) │ NATS Event Log │
│ implementing... │ debate.arch... │
│                 │ tools.exec...  │
├─────────────────┴─────────────────┤
│ > You: migrate auth to supabase   │
│                                   │
└───────────────────────────────────┘
```

## Also Found
- **gotmux:** Go library for programmatic tmux control
- **Sunder:** Go terminal multiplexer (tmux alternative in Go)
- **tview:** Alternative TUI library with grid/flexbox layouts

## Sources
github.com (charmbracelet/bubbletea), charm.land, lobehub.com, shi.foo, reddit.com
