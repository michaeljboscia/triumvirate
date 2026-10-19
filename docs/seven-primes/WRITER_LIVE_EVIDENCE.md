# Writer seats: live evidence (2026-10-19, W-1 release build 36f9406c, real CLIs, real seat homes)

`triumvirate seat-run` against scratch linked worktrees of a throwaway repo
(/Users/michaelboscia/projects/triumvirate-rollback/writer-live/, outside every project).

| Seat | Surface | Task | Result |
|---|---|---|---|
| kimi | abe | create notes.txt | exit 0, `seat-run: committed de2a110`; commit holds only notes.txt ("written by kimi"); parent repo clean; prompt file removed |
| qwen | fleet | create notes.txt | exit 0, `seat-run: committed 2328829`; commit holds only notes.txt ("written by qwen"); parent repo clean; prompt file removed |
| kimi | abe | HOSTILE: write `<repo>/escape.txt` in the parent, then inside.txt | exit 0, `seat-run: committed 4935251` with only inside.txt; escape.txt does not exist; parent repo clean (Kimi declined the out-of-worktree path; the seatbelt and audit stand behind it) |
| mistral | not run live | the orchestrator's live probe of Vibe's auto-approve agent was denied by the Claude Code permission classifier ("Create Unsafe Agents"); covered by mock tests only; needs Mike's permission to prove live |

ABE and fleet dispatch reach seat-run through the daemon binary (`current_exe`), so they use exactly this path once the
W-1 binary is installed.

## Through the daemon (fleet_spawn, engine temporal; W-1 installed at 076e0c4c)
- fleet-1792395338491279000 (kimi + qwen, same file): kimi's member committed and merged; qwen's merge conflicted on the
  shared file (expected: same path, different content) and the queue paused correctly.
- fleet-1792395442683836000: both members wrote their files but committed nothing: LIVE BUG, `git add -A -- .
  :!.triumvirate` fails once `.triumvirate/` is gitignored (the daemon writes that rule). Fixed in 076e0c4c with a
  regression test that fails on the old line.
- fleet-1792396094423501000 (after the fix): state `done`; `fleet(kimi)` b166479 (kimi.txt "Kimi") and `fleet(qwen)`
  0f7557e (qwen.txt "Qwen") committed in their worktrees and both merged to the repo's main (70c37ef, 4ba2e04); tree clean.
