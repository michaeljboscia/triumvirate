# D-028 (draft, file after the panel closes)

**Sight gate rejects a COMPLETE read when it arrives as limit/offset windows.**

Found 2026-09-22, twice, on the D-027 panel's grok seat.

`read_args_are_partial()` at daemon/crates/triumvirate/src/agent_exec.rs:2376:
- shell reads -> `command_reads_whole_file()`, and a coverage UNION runs over sed windows
  (see agent-adapter/src/codex.rs:230, "the coverage union never ran").
- every other tool -> partial if `limit`/`offset`/`start_line`/`end_line`/`line_offset`/
  `max_lines` is present at all. No union. Coverage is never computed.

Evidence, from the gate's OWN rejection text:
- agy_resilience.rs, 750 lines, read as [offset 1, limit 750] -> the whole file, rejected.
- orchestrator.rs, 1844 lines, read as [1..1000] + [1001..1844] -> no gap, rejected.

Impact: a reviewer that reads a large file correctly, in tiled windows, through its native
read tool can NEVER satisfy the gate. The reviewer is blamed ("only read PART of it") for a
complete read. Cost so far: two full grok deep reviews discarded. Worse shape: it teaches the
operator to weaken `require_sight`, which is the one guard standing between a real review and
a review written from memory.

Fix direction: apply the same union to structured reads. Build ranges from offset/limit
(offset..offset+limit-1), union them per source, compare against the line count, exactly as the
shell path does. Note the two paths disagreeing is the same two-surface shape as D-027.

CHECK to close: a review whose only reads are two adjacent limit/offset windows covering every
line of a named source is ACCEPTED; one that leaves a gap is still rejected.
