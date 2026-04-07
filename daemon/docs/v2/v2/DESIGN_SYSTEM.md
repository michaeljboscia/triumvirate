# DESIGN_SYSTEM — Triumvirate v2

**Version:** 1.0
**Date:** 2026-04-05
**Cross-refs:** FRONTEND_GUIDELINES.md, APP_FLOW.md

---

## Color Palette

### Background

| Token | Hex | Usage |
|-------|-----|-------|
| `bg-base` | `#0a0a0f` | Body background |
| `bg-surface` | `#0d0d14` | Panes, cards, input areas |
| `bg-elevated` | `#12121c` | Input fields, code blocks |
| `bg-border` | `#1a1a2e` | Borders, dividers, grid gaps |
| `bg-hover` | `#2a2a40` | Button hover, row hover |

### Agent Colors

| Token | Hex | Agent | Usage |
|-------|-----|-------|-------|
| `agent-claude` | `#7c6ef0` | Claude | Accent border, status dot, primary action |
| `agent-gemini` | `#4ecdc4` | Gemini | Accent border, status dot |
| `agent-codex` | `#50fa7b` | Codex | Accent border, status dot |
| `agent-system` | `#f0a040` | System/Events | Event log accent |
| `agent-human` | `#e0e0e8` | Human | Input text |

### Status Colors

| Token | Hex | Usage |
|-------|-----|-------|
| `status-ready` | `#4ade80` | Agent ready, system ok |
| `status-busy` | `#f0c040` | Agent thinking, degraded |
| `status-dead` | `#ef4444` | Agent dead, error |
| `status-idle` | `#606080` | Agent idle, no events |

### Text

| Token | Hex | Usage |
|-------|-----|-------|
| `text-primary` | `#e0e0e8` | Primary text, human input |
| `text-secondary` | `#a0a0b8` | Headers, labels |
| `text-muted` | `#808098` | Agent output body |
| `text-dim` | `#606080` | Hints, timestamps, model tags |
| `text-disabled` | `#404060` | Placeholder text, empty states |

### Semantic

| Token | Hex | Usage |
|-------|-----|-------|
| `accent-primary` | `#7c6ef0` | Primary buttons, focus rings, links |
| `accent-primary-hover` | `#6b5ce0` | Primary button hover |
| `success-bg` | `#1a2e1a` | Success badge background |
| `success-border` | `#2a4a2a` | Success badge border |
| `success-text` | `#4ade80` | Success badge text |
| `warning-bg` | `#2e2a1a` | Warning/degraded background |
| `warning-border` | `#4a4020` | Warning badge border |
| `warning-text` | `#f0c040` | Warning badge text |
| `error-bg` | `#2e1a1a` | Error background |
| `error-border` | `#4a2020` | Error badge border |
| `error-text` | `#ef4444` | Error badge text |

---

## Typography

| Token | Family | Size | Weight | Line Height | Usage |
|-------|--------|------|--------|-------------|-------|
| `font-mono` | `'SF Mono', 'Fira Code', 'JetBrains Mono', monospace` | — | — | — | All text (monospace app) |
| `text-xs` | mono | 11px | 400 | 1.4 | Model tags, hints |
| `text-sm` | mono | 12px | 400 | 1.5 | Buttons, labels, badges |
| `text-base` | mono | 13px | 400 | 1.6 | Agent output, event log |
| `text-md` | mono | 14px | 400 | 1.6 | Human input, primary content |
| `text-lg` | mono | 16px | 600 | 1.4 | Section headers |
| `text-xl` | mono | 20px | 700 | 1.3 | Page titles (rare) |

Letter spacing: `0.05em` on headers only.

---

## Spacing

Base unit: **4px**

| Token | Value | Usage |
|-------|-------|-------|
| `space-1` | 4px | Inline gaps |
| `space-2` | 8px | Icon gaps, tight padding |
| `space-3` | 12px | Pane padding, input padding |
| `space-4` | 16px | Section padding, card padding |
| `space-6` | 24px | Page-level padding |
| `space-8` | 32px | Large section gaps |

---

## Border Radius

| Token | Value | Usage |
|-------|-------|-------|
| `radius-sm` | 4px | Status dots (full circle: 50%) |
| `radius-md` | 6px | Buttons |
| `radius-lg` | 8px | Input fields, cards |
| `radius-xl` | 12px | Badges |

---

## Shadows

None. This is a dark terminal-style UI. No drop shadows. Depth is conveyed through background color layering (`bg-base` → `bg-surface` → `bg-elevated`).

---

## Borders

| Token | Value | Usage |
|-------|-------|-------|
| `border-default` | `1px solid #1a1a2e` | Pane borders, dividers |
| `border-focus` | `1px solid #7c6ef0` | Input focus state |
| `border-agent` | `3px solid {agent-color}` | Left border on agent pane headers |
| `border-button` | `1px solid #2a2a40` | Button borders |

---

## Breakpoints

| Token | Value | Behavior |
|-------|-------|----------|
| `bp-mobile` | 640px | Single column, stacked panes |
| `bp-tablet` | 1024px | 2-column grid |
| `bp-desktop` | 1280px | Full grid (2x2 or dynamic) |
| `bp-wide` | 1920px | Expanded panes with more content |

Mobile-first: default styles target `< bp-mobile`. Enhancements via `@media (min-width: ...)`.

---

## Animation

| Token | Value | Usage |
|-------|-------|-------|
| `duration-fast` | 100ms | Status dot transitions |
| `duration-normal` | 200ms | Button hover, pane transitions |
| `duration-slow` | 400ms | View toggle (tasks ↔ agents) |
| `easing-default` | `ease-in-out` | All transitions |

No animations on agent output streaming — text appears instantly. Animations only on UI chrome (buttons, status changes, view transitions).

---

## Component Tokens

### Status Dot

- Size: 8px × 8px
- Border radius: 50%
- Colors: `status-ready`, `status-idle`, `status-dead`

### Badge

- Font: `text-sm`
- Padding: `space-1` vertical, `space-3` horizontal
- Border radius: `radius-xl`
- Variants: success, warning, error (colors from semantic palette)

### Button

- Font: `text-sm`, `font-mono`
- Padding: `space-1.5` vertical (6px), `space-4` horizontal
- Border radius: `radius-md`
- Default: `bg-border` background, `text-secondary` color, `border-button`
- Primary: `accent-primary` background, white text, `accent-primary` border
- Hover: respective `-hover` variants

### Pane Header

- Padding: `space-3` vertical, `space-4` horizontal
- Bottom border: `border-default`
- Left border: `border-agent` (3px, agent color)
- Content: status dot + agent name (bold) + model tag (right-aligned, `text-dim`)

### Input Area

- Height: 200px
- Background: `bg-surface`
- Textarea: `bg-elevated`, `border-default`, `radius-lg`
- Focus: `border-focus`
- Font: `text-md`

---

## Themes

v1 ships with dark theme only. No light theme. No theme toggle. The dark terminal aesthetic is the product identity.
