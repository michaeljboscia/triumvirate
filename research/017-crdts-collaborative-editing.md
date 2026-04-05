# Research 017: CRDTs — Lock-Free Concurrent Agent Editing

**If Claude and Codex edit the same file at the same time, CRDTs prevent corruption.**

## Key Findings
- CRDTs guarantee eventual consistency without coordination
- **Yjs:** Fast, optimized for real-time text editing, Rust port (Yrs) exists
- **Automerge:** Full history, Byzantine fault tolerance, Rust core + WASM
- arxiv paper confirmed: multi-agent LLM code generation using Yjs CRDTs — lock-free, conflict-free

## Critical Caveat
CRDTs resolve CHARACTER-LEVEL conflicts, not SEMANTIC conflicts. Two agents refactoring the same function differently → valid CRDT merge but broken code. Higher-level coordination (debate/planning) still needed before parallel execution.

## How to Use in Triumvirate
1. Agents get CRDT-backed virtual documents for parallel work
2. Go daemon manages CRDT state, merges operations, writes to disk
3. Combined with Tree-sitter: CRDT merge → AST validation → disk write (only valid code lands)
4. Phase 2+ feature — start with file-level locking, graduate to CRDTs

## Sources
redis.io, wikipedia.org, crdt.tech, vlcn.io, github.com (Yjs), arxiv.org, medium.com, hackernoon.com
