# Contributing to Triumvirate

Issues and PRs welcome. Read this first.

---

## Setup

```bash
# Prerequisites
rustup update stable    # Rust 1.93+
node --version          # Node.js 20+ (for Svelte dashboard)

# Clone and build
git clone https://github.com/michaeljboscia/triumvirate-agentd
cd triumvirate-agentd/daemon
cargo build
cargo test
```

## Quality Gates

Every PR must pass:

```bash
cargo fmt --check              # Formatting
cargo clippy -- -D warnings    # Linting (zero warnings)
cargo test                     # Unit tests
cargo test --features mock     # Integration tests with mock CLIs
cargo build --release          # Release build compiles
```

## Code Standards

- **Rust edition 2024**, Tokio async runtime
- `///` doc comments on every public struct, trait, function, enum
- `//` inline comments on non-obvious logic
- `// FEAT-XXX` at the top of each module referencing the PRD feature ID
- `// Adapted from <source> (<repo>, <license>)` on borrowed patterns
- No `.unwrap()` in production code — use `?` or explicit error handling
- No `println!` — use `tracing::info!`, `tracing::warn!`, `tracing::error!`
- No `unsafe` without explicit justification

## Project Structure

```
daemon/
├── crates/
│   ├── agentd/         # Main binary (connectors, fabric, web, fleet)
│   ├── proto/          # Shared types (events, agent protocol parsers)
│   ├── workflow/        # Workflow engine (state machine, persistence)
│   ├── mock-claude/    # Mock CLI for testing
│   ├── mock-gemini/    # Mock CLI for testing
│   └── mock-codex/     # Mock CLI for testing
├── frontend/           # Svelte + Tailwind dashboard
├── static/             # POC HTML (legacy, replaced by frontend/)
├── config/             # Default config templates
└── policies/           # Cedar policy files
```

## Documentation

Canonical docs are in `docs/v2/`. Read the PRD (`docs/v2/PRD.md`) before making feature changes. Read the SPEC (`SPEC.md`) before making architectural changes.

## Attribution

If you borrow a pattern from an external project, add:
1. Inline comment: `// Adapted from <project> (<repo>, <license>)`
2. Entry in `daemon/NOTICE.md`

## License

MIT. By contributing, you agree your contributions are licensed under MIT.
