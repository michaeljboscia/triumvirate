# Pantheon v4.0 — Frontend Guidelines

**Spec:** specs/PANTHEON_V4.md  
**Design System:** docs/4.0.0/DESIGN_SYSTEM.md  
**Tech Stack:** docs/4.0.0/TECH_STACK.md  

---

## Framework

Svelte 5 with runes ($state, $effect, $props, $derived). NOT Svelte 4. No legacy stores API. No `onMount` for reactive logic — use `$effect` instead. `onMount` only for one-time imperative setup (xterm.js initialization).

---

## Component Architecture

```
App.svelte
├── Sidebar.svelte
│   ├── SessionTree.svelte (hierarchical session/worker tree)
│   │   ├── SessionNode.svelte (top-level: terminal panel)
│   │   └── WorkerNode.svelte (indented: daemon worker)
│   └── UnmanagedSection.svelte
├── TerminalArea.svelte (PaneForge container)
│   ├── TabBar.svelte
│   └── TerminalPanel.svelte (xterm.js + PTY bridge)
├── StatusArea.svelte
│   ├── TokenEconomics.svelte
│   ├── FleetStatus.svelte
│   └── SystemHealth.svelte
├── WorkerDrawer.svelte (bottom/right detail panel)
└── Dialogs/
    ├── DirectoryPicker.svelte
    ├── QuitConfirmation.svelte
    └── KillConfirmation.svelte
```

---

## File Structure

```
pantheon/src/
├── App.svelte                  # Root: three-region flexbox layout
├── app.css                     # Tailwind imports + global resets
├── lib/
│   ├── components/             # UI components (above tree)
│   ├── stores/
│   │   ├── daemon.ts           # WebSocket connection, health state machine
│   │   ├── sessions.ts         # Terminal panel state (tabs, splits)
│   │   ├── workers.ts          # Worker hierarchy (from WebSocket events)
│   │   ├── preferences.ts      # Theme, recent projects, settings
│   │   └── processes.ts        # Unmanaged process scan results
│   ├── types/
│   │   ├── events.ts           # AgentStreamEvent, WorkerLifecycle (mirror shared-types)
│   │   ├── daemon.ts           # DaemonStatus, WorkersResponse, etc.
│   │   └── terminal.ts         # TerminalPanel, PtyData, PtyExit
│   └── utils/
│       ├── theme.ts            # Dark/light mode logic
│       ├── format.ts           # Token formatting ("248K in / 31K out")
│       └── ansi.ts             # ANSI stripping for notification detection
```

---

## Naming Conventions

| Type | Convention | Example |
|---|---|---|
| Component files | PascalCase.svelte | `TerminalPanel.svelte` |
| Store files | camelCase.ts | `daemon.ts` |
| Type files | camelCase.ts | `events.ts` |
| Utility files | camelCase.ts | `format.ts` |
| CSS classes | Tailwind utilities | `class="flex items-center gap-2"` |
| Svelte runes | $ prefix | `$state`, `$effect`, `$props` |
| Tauri IPC functions | snake_case (Rust convention) | `create_terminal`, `scan_processes` |
| Tauri events | kebab-case | `pty-data`, `daemon-state` |

---

## State Management

**No global store library (Zustand, Redux, etc.).** Svelte 5 runes + Tauri event listeners are sufficient.

### Pattern: Reactive Store Module

```typescript
// stores/daemon.ts
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

// Reactive state using module-level $state (Svelte 5 rune)
let status = $state<'starting' | 'ready' | 'degraded' | 'disconnected'>('disconnected');
let workers = $state<Worker[]>([]);
let lastSeq = $state(0);

// Derived
let isConnected = $derived(status === 'ready' || status === 'degraded');

// Initialize in App.svelte onMount
export async function init() {
  await listen('daemon-state', (event) => {
    status = event.payload.state;
  });
  // ... WebSocket event forwarding from Tauri backend
}

export { status, workers, lastSeq, isConnected };
```

### Pattern: xterm.js Integration

```svelte
<!-- TerminalPanel.svelte -->
<script lang="ts">
  import { Terminal } from '@xterm/xterm';
  import { WebglAddon } from '@xterm/addon-webgl';
  import { FitAddon } from '@xterm/addon-fit';
  import { SearchAddon } from '@xterm/addon-search';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';

  let { terminalId, theme } = $props();
  let container = $state<HTMLDivElement>();

  $effect(() => {
    if (!container) return;

    const term = new Terminal({
      scrollback: 5000,
      cursorBlink: true,
      allowProposedApi: true,
      theme: theme,
    });

    const fit = new FitAddon();
    const search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);

    // WebGL with context loss recovery
    let webgl: WebglAddon;
    function loadWebGL() {
      webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        webgl.dispose();
        loadWebGL();
      });
      term.loadAddon(webgl);
    }

    term.open(container);
    fit.fit();
    loadWebGL();

    // PTY data from Tauri backend
    const unlisten = listen(`pty-data-${terminalId}`, (event) => {
      term.write(new Uint8Array(event.payload.bytes));
    });

    // User input to PTY
    term.onData((data) => {
      invoke('write_to_terminal', { terminalId, data: Array.from(new TextEncoder().encode(data)) });
    });

    // Resize
    const observer = new ResizeObserver(() => {
      fit.fit();
      invoke('resize_terminal', { terminalId, rows: term.rows, cols: term.cols });
    });
    observer.observe(container);

    return () => {
      observer.disconnect();
      webgl?.dispose();
      term.dispose();
      unlisten.then(fn => fn());
    };
  });
</script>

<div bind:this={container} class="w-full h-full" />
```

---

## Layout Rules

1. **Outer layout is CSS flexbox, NOT PaneForge.** Three regions: sidebar (fixed width, collapsible), terminal area (flex: 1), status area (fixed width, collapsible).

2. **PaneForge manages ONLY the terminal area** — tabs and splits within the center region.

3. **Worker detail drawer** is a separate panel that overlays or sits below the terminal area. Not managed by PaneForge.

4. **Responsive behavior** is window-width based, not viewport-based (this is a desktop app, not a web page):
   - ≥ 1200px: all three regions visible
   - < 1200px: status area auto-collapses
   - 900px minimum: sidebar + terminal area

---

## Tailwind Configuration

```typescript
// tailwind.config.ts
import type { Config } from 'tailwindcss';

export default {
  content: ['./src/**/*.{svelte,ts}'],
  darkMode: 'class', // Toggle via document.documentElement.classList
  theme: {
    extend: {
      colors: {
        // Reference DESIGN_SYSTEM.md tokens
      },
      fontFamily: {
        sans: ['system-ui', '-apple-system', 'BlinkMacSystemFont', 'sans-serif'],
        mono: ['"SF Mono"', 'Menlo', 'Monaco', 'monospace'],
      },
    },
  },
} satisfies Config;
```

---

## Tauri IPC Bridge

### Invoking Rust Commands
```typescript
import { invoke } from '@tauri-apps/api/core';

// Type-safe invoke wrapper
const terminalId = await invoke<string>('create_terminal', { cwd: '/path/to/project' });
```

### Listening to Events
```typescript
import { listen } from '@tauri-apps/api/event';

const unlisten = await listen<PtyData>('pty-data', (event) => {
  // event.payload has the typed data
});

// Cleanup
unlisten();
```

### Rules
- All Tauri commands use snake_case (Rust convention)
- All event names use kebab-case
- Type definitions in `lib/types/` mirror Rust structs
- Never use `fetch()` to call daemon directly from frontend — always go through Tauri backend (the backend handles auth tokens)

---

## Component Rules

1. **Props via $props(), not export let.** Svelte 5 only.
2. **Cleanup in $effect return.** Every $effect that creates listeners, observers, or timers must return a cleanup function.
3. **No component-level CSS.** Use Tailwind utility classes. Exception: xterm.js container needs explicit height.
4. **DESIGN_SYSTEM.md is law.** No colors, spacing, or typography outside the design system.
5. **One component per file.** No multi-component files.
6. **Feature IDs in comments.** Every component that implements a FEAT must reference it: `// FEAT-001: Embedded Terminal Panels`
