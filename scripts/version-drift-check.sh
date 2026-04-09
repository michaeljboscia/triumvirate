#!/bin/bash
# Verify staged spec/doc files declare a version matching Cargo.toml
set -e

CARGO_VERSION=$(grep -m1 '^version = ' daemon/Cargo.toml | cut -d'"' -f2)

if [ -z "$CARGO_VERSION" ]; then
    echo "version-drift-check: could not read Cargo workspace version"
    exit 1
fi

STAGED=$(git diff --cached --name-only --diff-filter=AM | grep -E '^(specs|docs)/.*\.md$' || true)

FAILED=0
for file in $STAGED; do
    if [ ! -f "$file" ]; then continue; fi
    # Look for a version declaration in the first 20 lines
    SPEC_VERSION=$(head -n 20 "$file" | grep -oE '(Version:|Target version:|version:)\s*`?[0-9]+\.[0-9]+\.[0-9]+`?' | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)
    if [ -n "$SPEC_VERSION" ] && [ "$SPEC_VERSION" != "$CARGO_VERSION" ]; then
        echo "version-drift-check: $file declares $SPEC_VERSION but Cargo.toml is at $CARGO_VERSION"
        FAILED=1
    fi
done

if [ $FAILED -eq 1 ]; then
    echo ""
    echo "Fix: bump daemon/Cargo.toml, or update the spec version header, or stage both together."
    exit 1
fi
