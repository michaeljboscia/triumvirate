# Research 014: Tree-sitter — AST-Native Code Operations

**Stop treating code as text. Agents should edit via AST nodes, not line numbers.**

## What Tree-sitter Does
- Generates ASTs (Abstract Syntax Trees) from source code
- Incremental parsing — updates only changed parts, microseconds even for large files
- Error-tolerant — handles incomplete/broken code (critical for AI mid-edit)
- Go bindings available (`smacker/go-tree-sitter`)
- Supports 100+ languages

## Why This Changes Everything for AI Code Generation
1. **Reliability** — AI describes "shape of logic," system compiles to valid code
2. **Security** — generated code adheres to predefined rules
3. **Simpler reasoning** — AI selects AST nodes instead of fighting syntax
4. **Language agnostic** — same AST can target different languages
5. **No formatting errors** — editing AST nodes preserves indentation, structure

## How to Use in Triumvirate
Instead of:
```
Edit file.go, replace lines 10-15 with new code
```

Agent says:
```json
{"action": "UPDATE_NODE", "target": "function:calculateAuth", "new_body": "..."}
```

Go daemon:
1. Parses file into AST via Tree-sitter
2. Finds the target node
3. Replaces the subtree
4. Regenerates source from modified AST
5. Validates syntax before writing to disk

## Phase 2+ Feature
This is powerful but complex. Start with structured text editing (Go's `go/ast` for Go files). Graduate to full Tree-sitter for multi-language support.

## Sources
dev.to, dropstone.io, reddit.com, github.io, hackernoon.com, medium.com, kiro.dev, go.dev
