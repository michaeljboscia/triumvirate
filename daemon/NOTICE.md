# NOTICE

Triumvirate v2 daemon includes implementation patterns informed by open-source prior art.

## Referenced Projects

- Ruflo (`ruvnet/ruflo`, MIT)
  - Routing and multi-model coordination ideas informed early architecture.
- Clash
  - Worktree conflict-management patterns informed fleet merge/worktree flow.
- swarms-rs
  - Agent lifecycle supervision patterns informed connector orchestration.
- Temporal (`temporalio/temporal`)
  - Event-sourced workflow persistence/recovery patterns informed workflow crate design.

## Attribution Notes

This project does not vendor source code from the projects above.
Where patterns were adapted, implementation was re-authored in Rust for this codebase.
