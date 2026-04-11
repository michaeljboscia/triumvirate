# Pantheon v4.0 — Design System

**Spec:** specs/PANTHEON_V4.md  

---

## Platform

Native macOS app (Tauri v2 + WKWebView). NOT a web app. NOT mobile. Desktop-only, Apple Silicon only for v4.0.

**Note:** The "mobile-first mandate" from the uncompromising-executor template does NOT apply to this project. Pantheon is a native macOS desktop application. All design targets macOS window sizes (minimum 900x600). No mobile breakpoints.

---

## Colors

### Light Mode
| Token | Hex | Usage |
|---|---|---|
| bg-primary | #FFFFFF | Main background |
| bg-secondary | #F5F5F5 | Sidebar, status area |
| bg-tertiary | #EBEBEB | Hover states, selected items |
| text-primary | #1A1B26 | Body text |
| text-secondary | #6B7280 | Muted labels, timestamps |
| border | #E5E7EB | Panel borders, dividers |
| accent | #3B82F6 | Selected tab, active session indicator |
| success | #22C55E | Committed worker, ready state |
| warning | #F59E0B | Idle warning badge, degraded state |
| error | #EF4444 | Failed worker, disconnected state |
| info | #6366F1 | Working status |

### Dark Mode
| Token | Hex | Usage |
|---|---|---|
| bg-primary | #1A1B26 | Main background |
| bg-secondary | #24283B | Sidebar, status area |
| bg-tertiary | #2F334D | Hover states, selected items |
| text-primary | #C0CAF5 | Body text |
| text-secondary | #565F89 | Muted labels, timestamps |
| border | #3B4261 | Panel borders, dividers |
| accent | #7AA2F7 | Selected tab, active session indicator |
| success | #9ECE6A | Committed worker, ready state |
| warning | #E0AF68 | Idle warning badge, degraded state |
| error | #F7768E | Failed worker, disconnected state |
| info | #7DCFFF | Working status |

### xterm.js Terminal Theme
| Token | Light | Dark |
|---|---|---|
| background | #FFFFFF | #1A1B26 |
| foreground | #1A1B26 | #C0CAF5 |
| cursor | #3B82F6 | #7AA2F7 |
| selectionBackground | rgba(59,130,246,0.3) | rgba(122,162,247,0.3) |

---

## Typography

| Token | Value | Usage |
|---|---|---|
| font-sans | system-ui, -apple-system, BlinkMacSystemFont | UI text |
| font-mono | "SF Mono", "Menlo", "Monaco", monospace | Terminal panels, code, event logs |
| text-xs | 11px / 1.4 | Timestamps, badges |
| text-sm | 13px / 1.5 | Sidebar items, status values |
| text-base | 15px / 1.6 | Body text, labels |
| text-lg | 17px / 1.4 | Section headers |
| text-xl | 20px / 1.3 | Panel titles |
| font-normal | 400 | Body text |
| font-medium | 500 | Labels, sidebar items |
| font-semibold | 600 | Headers, active tab |

---

## Spacing

Base unit: 4px

| Token | Value | Usage |
|---|---|---|
| space-1 | 4px | Tight padding, icon margins |
| space-2 | 8px | List item padding, inline gaps |
| space-3 | 12px | Section padding |
| space-4 | 16px | Panel padding, card padding |
| space-6 | 24px | Section margins |
| space-8 | 32px | Major section gaps |

---

## Layout Dimensions

| Token | Value | Usage |
|---|---|---|
| sidebar-width | 250px | Default sidebar width |
| sidebar-min | 200px | Minimum sidebar width |
| sidebar-max | 400px | Maximum sidebar width |
| status-width | 280px | Default status area width |
| status-min | 220px | Minimum status area width |
| tab-height | 36px | Tab bar height |
| drawer-height | 250px | Worker detail drawer height |
| tray-icon-size | 22x22 (@2x: 44x44) | Menubar icon |
| min-window-width | 900px | Minimum window width |
| min-window-height | 600px | Minimum window height |
| status-collapse-breakpoint | 1200px | Auto-collapse status area |

---

## Border Radius

| Token | Value | Usage |
|---|---|---|
| radius-sm | 4px | Badges, small buttons |
| radius-md | 6px | Inputs, cards |
| radius-lg | 8px | Panels, drawers |
| radius-full | 9999px | Circular indicators |

---

## Shadows

| Token | Value | Usage |
|---|---|---|
| shadow-sm | 0 1px 2px rgba(0,0,0,0.05) | Subtle elevation |
| shadow-md | 0 4px 6px rgba(0,0,0,0.1) | Drawers, dropdowns |
| shadow-lg | 0 10px 15px rgba(0,0,0,0.15) | Dialogs, modals |

---

## Status Indicators

| State | Icon Shape | Color Token |
|---|---|---|
| Ready | Filled circle | success |
| Working | Pulsing dot | info |
| Queued | Empty circle | text-secondary |
| Committed | Checkmark | success |
| Failed | X mark | error |
| Idle | Hollow circle | text-secondary |
| Warning | Exclamation in circle | warning |
| Disconnected | Slash through circle | error |
| Starting | Pulsing animation | text-secondary |

---

## Animation

| Token | Value | Usage |
|---|---|---|
| duration-fast | 100ms | Hover states, focus |
| duration-normal | 200ms | Sidebar collapse, panel transitions |
| duration-slow | 300ms | Drawer open/close |
| easing | cubic-bezier(0.4, 0, 0.2, 1) | All transitions |
| pulse-duration | 1.5s | Starting state menubar icon |

---

## Menubar Icons (Template Images)

All icons are macOS template images: black (#000000) on transparent background, 22x22pt @2x (44x44px). System applies tinting for dark/light mode and Liquid Glass automatically.

| State | Shape Description |
|---|---|
| Ready | Solid filled circle |
| Degraded | Circle with exclamation mark inside |
| Disconnected | Circle with diagonal slash |
| Starting | Circle with animated pulse (alternating opacity) |
