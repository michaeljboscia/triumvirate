# Design System — Triumvirate Dashboard

**Status:** Spec deliverable. Must be approved before dashboard development (REQ-048).

---

## Colors

### Base Palette

| Token | Hex | Usage |
|-------|-----|-------|
| `--color-bg` | `#0f1117` | Page background |
| `--color-surface` | `#1a1d27` | Card/panel background |
| `--color-surface-hover` | `#232736` | Interactive surface hover |
| `--color-border` | `#2d3142` | Borders, dividers |
| `--color-text` | `#e4e4e7` | Primary text |
| `--color-text-muted` | `#71717a` | Secondary text, labels |
| `--color-text-dim` | `#3f3f46` | Disabled text |

### Status Colors

| Token | Hex | Usage |
|-------|-----|-------|
| `--color-healthy` | `#22c55e` | Green — system healthy, approve |
| `--color-degraded` | `#eab308` | Yellow — degraded, warning |
| `--color-dead` | `#ef4444` | Red — dead, failed, reject |
| `--color-working` | `#3b82f6` | Blue — in progress, active |
| `--color-idle` | `#71717a` | Gray — idle, pending |
| `--color-stuck` | `#f97316` | Orange — stuck agent |

### Agent Colors

| Token | Hex | Usage |
|-------|-----|-------|
| `--color-claude` | `#d97706` | Claude accent (amber) |
| `--color-gemini` | `#2563eb` | Gemini accent (blue) |
| `--color-codex` | `#16a34a` | Codex accent (green) |

---

## Typography

| Token | Value | Usage |
|-------|-------|-------|
| `--font-sans` | `'Inter', system-ui, sans-serif` | Body text, UI |
| `--font-mono` | `'JetBrains Mono', 'Fira Code', monospace` | Code, metrics, IDs |
| `--text-xs` | `0.75rem / 1rem` | Timestamps, badges |
| `--text-sm` | `0.875rem / 1.25rem` | Labels, secondary |
| `--text-base` | `1rem / 1.5rem` | Body text |
| `--text-lg` | `1.125rem / 1.75rem` | Section headers |
| `--text-xl` | `1.25rem / 1.75rem` | View titles |
| `--text-2xl` | `1.5rem / 2rem` | Page titles |
| `--font-weight-normal` | `400` | Body |
| `--font-weight-medium` | `500` | Labels, emphasis |
| `--font-weight-semibold` | `600` | Headers |
| `--font-weight-bold` | `700` | Metrics, counters |

---

## Spacing

4px base unit. Scale: 0, 1, 2, 3, 4, 5, 6, 8, 10, 12, 16, 20, 24.

| Token | Value | Common Usage |
|-------|-------|------|
| `--space-1` | `4px` | Tight inline spacing |
| `--space-2` | `8px` | Icon gaps, badge padding |
| `--space-3` | `12px` | Card padding (compact) |
| `--space-4` | `16px` | Card padding (standard) |
| `--space-6` | `24px` | Section gaps |
| `--space-8` | `32px` | View padding |

---

## Border Radius

| Token | Value | Usage |
|-------|-------|-------|
| `--radius-sm` | `4px` | Badges, small chips |
| `--radius-md` | `8px` | Cards, inputs |
| `--radius-lg` | `12px` | Modals, large panels |
| `--radius-full` | `9999px` | Avatars, pills |

---

## Shadows

| Token | Value | Usage |
|-------|-------|-------|
| `--shadow-sm` | `0 1px 2px rgba(0,0,0,0.3)` | Subtle elevation |
| `--shadow-md` | `0 4px 6px rgba(0,0,0,0.4)` | Cards |
| `--shadow-lg` | `0 10px 15px rgba(0,0,0,0.5)` | Modals, dropdowns |

---

## Breakpoints

| Token | Value | Usage |
|-------|-------|-------|
| `sm` | `640px` | Mobile landscape |
| `md` | `768px` | Tablet |
| `lg` | `1024px` | Desktop |
| `xl` | `1280px` | Wide desktop |

---

## Component Variants

### Health Indicator
- Green dot (12px) + "Healthy" label
- Yellow dot + "Degraded" label
- Red dot + "Dead" label
- Pulsing animation on red (attention-grab)

### Agent Badge
- Rounded pill with agent color background at 15% opacity
- Agent name in agent color text
- State suffix in muted text: "Claude (working)" / "Gemini (idle)"

### Kanban Column (Fleet)
- Column header with state name + count badge
- Cards with task title, assigned agent badge, time elapsed
- Drag not supported (status updates via daemon, not UI)

### Confidence Bar (Lessons)
- Horizontal bar, 0-100% width
- Color gradient: green (>0.7) → yellow (0.3-0.7) → red (<0.3)
- Numeric label right-aligned
