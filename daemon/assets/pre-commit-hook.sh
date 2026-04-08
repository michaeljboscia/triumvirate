#!/usr/bin/env bash
set -euo pipefail

CONTRACT_PATH="${TRIUMVIRATE_CONTRACT_PATH:-.triumvirate/contract.json}"

if [[ ! -f "$CONTRACT_PATH" ]]; then
  echo "BLOCKED: missing contract.json at $CONTRACT_PATH"
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "BLOCKED: jq is required for contract validation"
  exit 1
fi

allowed_file() {
  local file="$1"
  jq -e --arg file "$file" '.allowed_files | index($file) != null' "$CONTRACT_PATH" >/dev/null
}

contains_stub_markers() {
  local file="$1"
  rg -n "TODO|FIXME|unimplemented!|placeholder" "$file" >/dev/null 2>&1
}

# Commit message check (best effort in hook context)
commit_format="$(jq -r '.commit_format // empty' "$CONTRACT_PATH")"
if [[ -n "$commit_format" ]]; then
  msg="$(git log -1 --pretty=%B 2>/dev/null || true)"
  if [[ -n "$msg" ]] && ! [[ "$msg" =~ $commit_format ]]; then
    echo "BLOCKED: Commit message does not match contract format ($commit_format)."
    exit 1
  fi
fi

staged_files="$(git diff --cached --name-only)"
if [[ -z "${staged_files//[[:space:]]/}" ]]; then
  exit 0
fi

while IFS= read -r file; do
  [[ -z "$file" ]] && continue
  if [[ "$file" == .triumvirate/* ]]; then
    continue
  fi

  if ! allowed_file "$file"; then
    echo "BLOCKED: Write to $file denied by contract $(jq -r '.task_id' "$CONTRACT_PATH")."
    echo "Allowed files: $(jq -r '.allowed_files | join(", ")' "$CONTRACT_PATH")"
    exit 1
  fi

  if [[ -f "$file" ]] && contains_stub_markers "$file"; then
    echo "BLOCKED: stub marker detected in $file"
    exit 1
  fi
done <<< "$staged_files"

exit 0
