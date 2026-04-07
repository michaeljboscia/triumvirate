# Frontend Guidelines — Triumvirate v2.2 Dashboard

---

## Framework

- **Svelte 5** with runes (`$state`, `$derived`, `$effect`)
- **Tailwind CSS 4** (CSS-first config, `@theme` directive)
- **Vite 6** for build + dev server
- **@sveltejs/adapter-static** for static site generation
- **TypeScript** throughout

## File Structure

```
dashboard/
    src/
        lib/
            api.ts          # Daemon REST client
            ws.ts           # WebSocket connection + store
            types.ts        # TypeScript types matching shared-types DTOs
            stores/
                agents.ts   # Agent state store
                fleet.ts    # Fleet state store
                ledger.ts   # Ledger health + search store
                lessons.ts  # Lessons store
                reviews.ts  # Reviews store
        routes/
            +layout.svelte  # Nav bar, health indicator
            sessions/
                +page.svelte
            fleet/
                +page.svelte
            ledger/
                +page.svelte
            lessons/
                +page.svelte
            reviews/
                +page.svelte
            metrics/
                +page.svelte
    static/             # Favicon, static assets
    DESIGN_SYSTEM.md    # Design tokens (must exist before dev starts)
    vite.config.ts
    svelte.config.js
    tailwind.config.ts
    package.json
```

## Component Naming

- Files: `PascalCase.svelte`
- Props: `camelCase`
- Events: `on:eventname` (lowercase)
- Stores: `camelCase` in `lib/stores/`

## State Management

- **WebSocket store** (`ws.ts`): single connection to `GET /ws`. Distributes events to feature stores.
- **Feature stores**: each view has a Svelte 5 store that derives state from WebSocket events + REST queries.
- **No global state object.** Each store is independent. Cross-store data flows through the WebSocket event stream.

## API Client

- `api.ts` wraps `fetch()` calls to daemon REST endpoints
- Base URL: relative in production (same origin), proxied in dev
- Bearer token read from `~/.triumvirate/daemon.token` (injected at build via env var or read at runtime)
- All API calls return typed responses matching `shared-types` DTOs

## WebSocket Protocol

- Connect to `/ws` on page load
- Reconnect with exponential backoff (1s, 2s, 4s, max 30s)
- Events are JSON with `type` discriminator
- Store updates are reactive via Svelte 5 runes

## Styling Rules

- All styling via Tailwind utility classes — no `<style>` blocks
- Design tokens from `DESIGN_SYSTEM.md` mapped to Tailwind `@theme`
- Component variants via `class-variance-authority` (CVA)
- Dark mode: `prefers-color-scheme` media query
- Responsive: mobile-first, breakpoints at `sm` (640), `md` (768), `lg` (1024)

## Build

- `npm run build` → `dist/` with relative asset paths (`--base=./`)
- `dist/` embedded by rust-embed in the `triumvirate` binary
- CI: `npm ci && npm run build` before `cargo build --release`
- Dev: `npm run dev` (Vite :5173) proxies to daemon :8080
