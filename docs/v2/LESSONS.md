# LESSONS — Triumvirate v2

Mistakes made, patterns discovered, things that broke and why.

---

## L-001: Run Goat Rodeo Before Scaffolding (2026-04-05)

**What went wrong:** Scaffolded the Rust daemon before running the Goat Rodeo on the spec. The scaffold compiled and ran, but was built against a spec full of Go assumptions that didn't hold in Rust (PTY for all agents, embedded NATS, embedded Temporal).

**Why:** Eagerness to see code compile. Skipped the spec review step from the user's own rules ("Run /goatrodeo on plans before coding").

**Rule:** ALWAYS run /goatrodeo before writing implementation code. Plans are draft specs, not green lights.

---

## L-002: Don't Auto-Resolve Architectural Changes (2026-04-05)

**What went wrong:** During the Goat Rodeo, auto-resolved "drop NATS KV" and "drop Temporal DevTools" as if they were minor. User pushed back — these are architecture changes that affect scope.

**Why:** Applied the auto-resolve test ("would the user be surprised?") incorrectly. Removing named technologies from the architecture is always a surprise.

**Rule:** Dropping a named technology is NEVER auto-resolvable. Surface it to the user.

---

## L-003: Don't Re-Decide What's Already Decided (2026-04-05)

**What went wrong:** Listed "GR1-D1 Web-only UI" and "GR1-D3 Adaptive lead" as Round 2 auto-resolved items. These were already decided in Round 1.

**Why:** Treated confirmation as a new decision. Noise, not signal.

**Rule:** If something was decided in a prior round and nothing challenges it, don't list it again.

---

## L-004: Follow the User's Conversation Thread (2026-04-05)

**What went wrong:** User was deep in the Temporal discussion (asking about decompile, port, replatform, drawbacks). I "reset" the Goat Rodeo and started from scratch with a numbered decision list, yanking them out of the conversation they were engaged in.

**Why:** Tried to impose process structure over a productive organic discussion.

**Rule:** If the user is productively exploring a decision, follow their thread. Don't interrupt with process.

---

## L-005: Listen For The Real Requirement (2026-04-05)

**What went wrong:** User asked about "replicating Claude Teams functionality." I assumed they meant 1-1-1 (one of each agent). They meant N-of-each — a dynamic fleet. Had to be corrected twice before understanding REQ-7.

**Why:** Projected my understanding of the existing spec onto what the user was saying, instead of asking what they actually meant.

**Rule:** When the user describes something new, ask what they mean before assuming you know.

---

## L-006: Codex Reads Files Mid-Session (2026-04-05)

**What went wrong:** Told the user Codex wouldn't pick up doc changes until its session restarted. Codex had already read the updated files and implemented the new features.

**Why:** Assumed Codex only reads instruction files at boot. In reality, Codex re-reads files during its session — proven on YellingToad Go rewrite and again here.

**Rule:** Codex picks up file changes mid-session. Updating progress.txt and canonical docs IS the communication mechanism. Don't waste time worrying about session restarts.
