---
name: persist-or-fail
description: Use when building, modifying, or running any sensor, scanner, or data pipeline that produces results — enforces mandatory persistence so compute never evaporates. Also use when writing inline test scripts, batch runners, or any code that calls a sensor's run/execute/scan method.
---

# Persist or Fail

## Overview

If compute was spent to produce data, that data MUST be persisted before the function returns. A scan that doesn't save is a scan that didn't happen. There is no valid reason to run a sensor and not capture the results.

## Rules

### Rule 1: Persistence lives inside run(), not the caller
The sensor's core execution method (`run()`, `execute()`, `scan()`) MUST call its own persistence function internally. The caller should never be responsible for saving results.

**You will be tempted to:** "I'll add save_result() in the CLI wrapper — that's the natural place."
**Why that fails:** On 2026-03-21, Wanderer's save_result() was only in CLI main(). Every inline test script, every batch runner, every asyncio.run(w.run()) call bypassed it. 11 domain scans evaporated. 3 days of development testing — gone. The caller doesn't know and doesn't care about persistence. The sensor does.

**The right way:**
```python
FALLBACK_DIR = os.path.expanduser("~/sensor_fallback")  # Durable path, not /tmp

async def run(self, dry_run: bool = False) -> Result:
    result = Result(...)
    try:
        # ... all compute in try block ...
        pass
    finally:
        # Persistence in finally — saves partial results even on crash
        if not dry_run:
            self._persist(result)

def _persist(self, result: Result):
    """Persist or fail. Both DB and fallback fail = exception, run marked failed."""
    try:
        save_result(result)
        # Verify the write landed
        count = verify_row_count(result.scan_id)
        assert count > 0, f"Persistence verification failed: 0 rows for {result.scan_id}"
    except Exception as db_err:
        # Fallback: durable disk path (NOT /tmp on ephemeral hosts)
        os.makedirs(FALLBACK_DIR, exist_ok=True)
        fallback_path = f"{FALLBACK_DIR}/{result.scan_id}.json"
        try:
            with open(fallback_path, 'w') as f:
                json.dump(result.to_dict(), f)
            print(f"DB failed: {db_err} — saved to {fallback_path}")
        except Exception as disk_err:
            # Both failed — the run FAILS
            raise RuntimeError(
                f"PERSISTENCE TOTAL FAILURE: DB={db_err}, Disk={disk_err}"
            ) from disk_err
```

### Rule 2: If tables exist, use them
If the Supabase schema has tables for this sensor's output, results go in those tables. Not CSV. Not JSON. Not stdout. The tables.

**You will be tempted to:** "I'll just print the results for now and persist later."
**Why that fails:** "Later" never comes. The script exits, the data is in a terminal buffer, and the terminal closes. stdout is not persistence.

**The right way:** Check if the persistence function exists for this sensor. If it does, call it. If the DB is unreachable, fall back to a local JSON file in a known directory — never to /dev/null.

### Rule 3: Fallback to disk, never to nothing
If the database is unreachable (no SUPABASE_DB_URL, connection timeout, auth failure), write results to a local file. The file path must be predictable and logged.

**You will be tempted to:** "The DB is down, I'll just skip the save and return the result."
**Why that fails:** The result object gets garbage collected when the script exits. The compute that produced it is unrecoverable. A JSON file on disk can be loaded and re-inserted later. Nothing cannot.

**The right way:**
```python
# NEVER /tmp on ephemeral VMs — use a durable, configurable path
FALLBACK_DIR = os.environ.get("SENSOR_FALLBACK_DIR", os.path.expanduser("~/sensor_fallback"))
os.makedirs(FALLBACK_DIR, exist_ok=True)
fallback_path = f"{FALLBACK_DIR}/{sensor}_{domain}_{scan_id}.json"
```

### Rule 4: dry_run is the only escape hatch
The ONLY way to skip persistence is an explicit `dry_run=True` parameter. If it's not set, persistence happens. There is no implicit skip.

**You will be tempted to:** "I'm just testing the detection logic, I don't need to save."
**Why that fails:** You said the same thing 11 times on 2026-03-21. Each "just testing" burned 5 minutes of real compute, real network requests, and real browser automation. Test data is still data. If you truly don't want it saved, pass dry_run=True and accept that you're making a conscious choice.

**The right way:**
```python
async def run(self, dry_run: bool = False) -> Result:
    # ... compute ...
    if not dry_run:
        self._persist(result)
    else:
        print(f"DRY RUN — results NOT persisted (scan_id={result.scan_id})")
    return result
```

### Rule 5: Inline scripts are safe ONLY if run() persists internally
If Rule 1 is implemented (persistence inside run()), inline scripts are fine. If Rule 1 is NOT yet implemented for a sensor, use the CLI.

**You will be tempted to:** "I'll just call run() directly — it probably saves."
**Why that fails:** "Probably" is how 11 scans evaporated. Before using an inline script, VERIFY that the sensor's run() method calls persistence internally. Check the code, don't assume.

**The right way:**
```python
# BEFORE writing an inline script, verify persistence is in run():
grep -n 'save_result\|_persist\|write_result' src/wanderer.py | grep -v '#'

# If persistence IS in run() — inline scripts are safe:
result = asyncio.run(w.run())  # Persists internally

# If persistence is NOT in run() — use CLI or add save_result():
result = asyncio.run(w.run())
save_result(result)  # Explicit until run() is fixed
```

### Rule 6: Verify persistence happened
After a batch run, query the database to confirm rows were written. "The script finished" is not proof. "The rows exist" is proof.

**You will be tempted to:** "save_result() didn't throw an exception, so it worked."
**Why that fails:** Silent failures exist. Connection pools that drop writes. Transactions that roll back. DuckDB COPY APPEND that silently overwrites (2026-03-19, 5 hours lost).

**The right way:**
```sql
SELECT scan_id, domain, created_at
FROM wanderer_scans
WHERE created_at > now() - interval '1 hour'
ORDER BY created_at DESC;
```

### Rule 7: Never say "done" without querying the database
"Committed to git" is not "done." "Run completed" is not "done." "Done" means rows exist in the target table. Query them. Show the count. If you can't show the count, it's not done.

**You will be tempted to:** "The code is committed and tested, persistence is handled."
**Why that fails:** On 2026-03-21, said "done" after 11 commits. Zero rows in Supabase. Said "done" again after re-running 11 domains. Zero rows again. SUPABASE_DB_URL wasn't set. The code "handled" persistence by catching the exception and continuing silently. "Done" was a lie told twice in the same session.

**The right way:**
```sql
-- This is what "done" looks like:
SELECT count(*), max(created_at) FROM wanderer_tech_detections WHERE scan_id = '<last_scan_id>';
-- If count = 0, you are NOT done.
```

### Rule 8: Verified persistence — row count or checksum
Writing is not enough. Verify the write landed. Query the count after insert. Compare expected vs actual. If they don't match, the run failed.

**You will be tempted to:** "save_result() didn't throw, so it worked."
**Why that fails:** DuckDB COPY APPEND silently overwrote 5 hours of data on 2026-03-19. The write "succeeded" — it just destroyed everything already there. Transactions can roll back. Connection pools can drop writes. The only proof is a post-write verification.

**The right way:**
```python
# After persistence, verify
count = cur.execute("SELECT count(*) FROM wanderer_tech_detections WHERE scan_id = %s", (scan_id,)).fetchone()[0]
expected = len(result.tech_profiles)
if count == 0:
    raise RuntimeError(f"Persistence verification FAILED: 0 rows for scan_id={scan_id}")
print(f"   Verified: {count} tech detection rows persisted")
```

## Validation Checklist

Run when building or modifying ANY sensor:
- [ ] `run()` calls persistence internally, in a `finally` block (not the caller)
- [ ] Fallback to durable disk path if DB unreachable (NOT /tmp on ephemeral hosts)
- [ ] `dry_run` parameter exists as explicit opt-out
- [ ] If both DB and fallback fail, run raises an exception (not silent continue)
- [ ] Inline scripts verified that run() persists before use (grep check)
- [ ] Post-write verification: row count or file size check after every persist
- [ ] Write mode explicit: INSERT/upsert, never silent overwrite

## Reference

### 2026-03-21: The Evaporation Incident
- 11 Wanderer domain scans run via inline `asyncio.run(w.run())`
- `save_result()` was only in CLI `main()`, not in `run()`
- Zero results persisted to Supabase
- ~50 minutes of compute wasted re-running
- 3 days of prior development testing — unknown data loss
- Fix: moved `save_result()` into `run()` (commit `48c91d4`)

### 2026-03-21: The Double Evaporation
- Fixed persistence by moving save_result() into run()
- Launched 11 re-runs to "fix" the first evaporation
- SUPABASE_DB_URL was not set in the shell
- try/except caught the error silently, tail -5 hid the output
- All 11 re-runs evaporated AGAIN
- Wrote the persist-or-fail skill during this same session
- The skill described the correct pattern (finally + fallback + fail-closed)
- The code committed used the wrong pattern (try/except + silent continue)
- Said "done" without verifying a single row existed in Supabase
- Total: 22 scans across 2 attempts, zero persisted, ~100 min wasted

### 2026-03-19: DuckDB Silent Overwrite
- DuckDB COPY APPEND silently overwrote instead of appending
- 5 hours of road KNN computation lost
- Same root cause: trusted execution = trusted persistence

### The Math
- One Wanderer scan: ~5 minutes, 13 pages, 28+ tech detections
- 11 scans lost: ~55 minutes compute, ~308 tech detections evaporated
- 3 days of testing: unknown — possibly hundreds of scans
- Cost of Rule 1 (persistence in run()): 6 lines of code, 0 performance impact
