# FRONTEND_GUIDELINES — Triumvirate v2

**Version:** 1.0
**Date:** 2026-04-05
**Cross-refs:** DESIGN_SYSTEM.md, APP_FLOW.md, TECH_STACK.md

---

## Framework

- **Svelte 5** with runes (`$state`, `$derived`, `$effect`)
- **Tailwind CSS 4** for utility-first styling
- **Vite 6** for build tooling
- Build output embedded in Rust binary via `rust-embed`

---

## File Structure

```
frontend/
├── src/
│   ├── app.html              # HTML shell
│   ├── App.svelte             # Root component, route handling
│   ├── lib/
│   │   ├── stores/
│   │   │   ├── agents.svelte.ts    # Agent pool state
│   │   │   ├── tasks.svelte.ts     # Task list state
│   │   │   ├── fabric.svelte.ts    # WebSocket connection + events
│   │   │   ├── quota.svelte.ts     # Quota meters
│   │   │   └── memory.svelte.ts    # Memory viewer state
│   │   ├── api.ts             # REST API client
│   │   └── ws.ts              # WebSocket client
│   ├── components/
│   │   ├── layout/
│   │   │   ├── Header.svelte
│   │   │   ├── InputArea.svelte
│   │   │   └── ViewToggle.svelte
│   │   ├── agents/
│   │   │   ├── AgentPane.svelte
│   │   │   ├── AgentGrid.svelte
│   │   │   ├── StatusDot.svelte
│   │   │   └── QuotaMeter.svelte
│   │   ├── tasks/
│   │   │   ├── TaskCard.svelte
│   │   │   ├── TaskList.svelte
│   │   │   └── FleetProgress.svelte
│   │   ├── memory/
│   │   │   ├── MemoryViewer.svelte
│   │   │   └── DecisionConfirm.svelte
│   │   ├── workflow/
│   │   │   ├── WorkflowPanel.svelte
│   │   │   └── MergeResolver.svelte
│   │   └── common/
│   │       ├── Badge.svelte
│   │       ├── Button.svelte
│   │       └── RoutingLog.svelte
│   └── views/
│       ├── TasksView.svelte
│       ├── AgentsView.svelte
│       ├── MemoryView.svelte
│       ├── SessionsView.svelte
│       ├── WorkflowView.svelte
│       ├── QuotaView.svelte
│       └── SettingsView.svelte
├── static/                    # Static assets (if any)
├── tailwind.config.ts         # Design system tokens
├── vite.config.ts
├── package.json
└── tsconfig.json
```

---

## Naming Conventions

| Type | Convention | Example |
|------|-----------|---------|
| Components | PascalCase | `AgentPane.svelte` |
| Stores | camelCase with `.svelte.ts` | `agents.svelte.ts` |
| Utilities | camelCase with `.ts` | `api.ts` |
| CSS classes | Tailwind utilities only | No custom CSS classes |
| Props | camelCase | `agentId`, `isStreaming` |
| Events | `on` + PascalCase | `onMessage`, `onInterrupt` |

---

## Component Architecture

### Hierarchy

```
App
├── Header (system status badge, view toggle)
├── ViewToggle (tasks ↔ agents)
├── [Active View]
│   ├── TasksView
│   │   ├── TaskList
│   │   │   └── TaskCard (per task)
│   │   │       └── AgentPane (inline, per assigned agent)
│   │   └── FleetProgress (if fleet active)
│   ├── AgentsView
│   │   └── AgentGrid
│   │       └── AgentPane (per running agent)
│   │           └── StatusDot
│   ├── MemoryView → MemoryViewer
│   ├── SessionsView (stenographer logs)
│   ├── WorkflowView → WorkflowPanel
│   ├── QuotaView → QuotaMeter (per agent type)
│   └── SettingsView
├── InputArea (always visible at bottom)
│   ├── Textarea
│   └── Button (Send, /debate, Interrupt)
└── [Overlays]
    ├── DecisionConfirm (modal)
    └── MergeResolver (modal)
```

### Rules

1. **Components are dumb.** They receive data via props and emit events. No direct API calls.
2. **Stores are smart.** All state lives in Svelte 5 rune-based stores. Stores call the API and process WebSocket events.
3. **One source of truth.** The WebSocket connection in `fabric.svelte.ts` is the primary data source. REST API is for initial load and mutations only.
4. **No prop drilling beyond 2 levels.** If data needs to go deeper, use a store.

---

## State Management

Svelte 5 runes. No external state library.

```typescript
// agents.svelte.ts
let agents = $state<Agent[]>([]);
let agentsByType = $derived(groupBy(agents, 'type'));

// Updated from WebSocket events
export function handleAgentOutput(event: AgentOutputEvent) {
  // ...
}
```

### Store Responsibilities

| Store | Data | Updates From |
|-------|------|-------------|
| `agents` | Agent instances, health, streaming output | WebSocket `agent_output`, `health_change` |
| `tasks` | Task list, status, assignments | WebSocket `task_update` |
| `fabric` | WebSocket connection, raw event buffer | WebSocket connection |
| `quota` | Per-agent quota percentages | WebSocket `quota_update` |
| `memory` | Memory entries, proposed decisions | REST `/api/memory`, WebSocket `decision_proposed` |

---

## Responsive Behavior

Mobile-first per DESIGN_SYSTEM.md breakpoints.

| Breakpoint | Layout |
|-----------|--------|
| `< 640px` | Single column. Panes stack vertically. Input area fixed at bottom. |
| `640-1024px` | 2-column grid. Tasks/agents in left column, detail in right. |
| `1024-1280px` | Full grid. 2×2 agent panes or task cards. |
| `> 1280px` | Wide layout. Side panel for routing log / quota. |

---

## WebSocket Integration

```typescript
// ws.ts
const ws = new WebSocket('ws://127.0.0.1:8080/ws');

ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  switch (msg.type) {
    case 'agent_output': agentStore.handleOutput(msg); break;
    case 'health_change': agentStore.handleHealth(msg); break;
    case 'task_update': taskStore.handleUpdate(msg); break;
    case 'quota_update': quotaStore.handleUpdate(msg); break;
    case 'decision_proposed': memoryStore.handleProposal(msg); break;
  }
};
```

---

## Performance Rules

1. **No re-renders on every streaming token.** Buffer agent output and flush to DOM on requestAnimationFrame.
2. **Virtual scroll** for routing log (can be thousands of entries).
3. **Lazy load** views that aren't visible (sessions, workflows, settings).
4. **No external fonts loaded at runtime.** System monospace fonts only.
