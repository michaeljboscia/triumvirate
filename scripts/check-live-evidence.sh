#!/usr/bin/env bash
# Residual limitation: this proves tree identity, seat coverage and run metadata, but cannot
# prove the JSON came from a real run. PR review is the compensating control.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FILE="${1:-$ROOT/docs/seven-primes/live-evidence.json}"
[[ -f "$FILE" ]] || { echo "RESULT: FAIL missing live evidence: $FILE"; exit 1; }
# A git failure must say so, not surface as a "code_tree mismatch" (finding by the Mistral seat's T-303 review).
TREE="$(git -C "$ROOT" ls-tree -r HEAD -- daemon scripts .github | git hash-object --stdin)" \
    || { echo "RESULT: FAIL cannot compute code_tree (git failed in $ROOT)"; exit 1; }
python3 -I - "$FILE" "$TREE" <<'PY'
import json,re,sys
try: v=json.load(open(sys.argv[1],encoding="utf-8"))
except Exception as e: print(f"RESULT: FAIL invalid evidence: {e}"); raise SystemExit(1)
if v.get("code_tree") != sys.argv[2]: print("RESULT: FAIL code_tree mismatch"); raise SystemExit(1)
rows=v.get("seats"); passed=0
if not isinstance(rows,dict): print("RESULT: FAIL seats object missing"); raise SystemExit(1)
for seat in ("kimi","mistral","qwen"):
 row=rows.get(seat)
 if not isinstance(row,dict): print(f"RESULT: FAIL missing seat: {seat}"); raise SystemExit(1)
 if row.get("status")=="PASS":
  n=row.get("tests_run"); m=re.search(r"test result: ok\. ([1-9][0-9]*) passed",row.get("cargo_summary",""))
  if not isinstance(n,int) or n<1 or not m or int(m.group(1))!=n or not all(row.get(k) for k in ("cli_version","timestamp")) or "served_model" not in row: print(f"RESULT: FAIL incomplete PASS: {seat}"); raise SystemExit(1)
  passed+=1
 elif row.get("status")=="NOT LAUNCHED":
  if not row.get("reason"): print(f"RESULT: FAIL missing reason: {seat}"); raise SystemExit(1)
 else: print(f"RESULT: FAIL invalid status: {seat}"); raise SystemExit(1)
if not passed: print("RESULT: FAIL zero PASS seats"); raise SystemExit(1)
# served_model may be null: REQ-043, a seat whose stream names no model (kimi, mistral) records it as unknown.
print(f"RESULT: PASS code_tree matches; {passed} of 3 seats PASS")
PY
