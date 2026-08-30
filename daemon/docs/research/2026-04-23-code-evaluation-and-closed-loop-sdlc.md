# Evaluating Code, AI-Generated Code, Multi-Agent Systems, Performance & Memory, and Closed-Loop SDLC Feedback

**Captured:** 2026-04-23
**Source:** External Q&A discussion, captured as reference artifact during Round 4 of Pantheon Gates spec authoring.
**Relevance to Pantheon:** Directly informs REQ-092 (OTel integration), REQ-093 (drift detection), REQ-094 (planted-defect peer-review effectiveness), REQ-095 (N≥3 framework), and the long-term closed-loop training-data feedback vision. The Q6 description of a feedback loop "from production observed this code is slow/memory-hungry/buggy back to the next time someone writes similar code, don't do that" is the articulated form of the user's Round 4 architectural commitment ("predict drift and fine-tune and retrain models to minimize non-compliant output").

A conversation on how code quality is judged, how those frameworks apply to AI-generated code, how multi-agent writing/evaluation systems actually perform, what can and can't be predicted about performance and memory from source, how refactoring actually works, and what a full production-feedback-to-code-generation pipeline would look like.

---

## Q1: Is there a way to judge or evaluate code empirically? Or is it all subjective in some way?

Mostly empirical, with a subjective residue that shrinks as you get more specific about what "good" means.

### The empirical side is bigger than people think

Correctness is testable (unit tests, property tests, fuzzing, formal verification for the ambitious). Performance is measurable (latency, throughput, memory, allocations, cache misses — all with real numbers). Reliability shows up in crash rates, error budgets, MTTR. Security has concrete signals: static analysis findings, CVE counts, results from fuzzers and pen tests. Even "maintainability," which feels squishy, has proxies that correlate with real outcomes: cyclomatic complexity, coupling metrics, churn-vs-bug-density, test coverage, time-to-first-commit for new contributors, PR review time. Code review comments per line is a surprisingly informative signal across a team.

### The genuinely subjective part

It's smaller and mostly about *taste within constraints that are already satisfied*. Given two implementations that are equally correct, fast, and secure, preferences about naming, abstraction level, functional vs imperative style, how much to comment, where to draw module boundaries — those are partly aesthetic and partly contextual (what does the rest of the codebase look like, what does this team know, what will this need to become in 18 months). Reasonable senior engineers disagree here and both can be right.

### The useful move

Convert as much of a code judgment as possible into "good relative to *what goal*." "Is this code good?" is underspecified. "Is this code fast enough to hit our p99 SLO, correct on the test suite, and understandable by someone who joins next quarter?" is answerable — the first two empirically, the third with proxies and a bit of judgment. The more you pin down the goal, the more the subjective surface area shrinks.

---

## Q2: How are these concepts applied to machine/AI created code?

Same metrics, mostly — but the weights shift and a few new failure modes show up.

### What transfers directly

Correctness tests, benchmarks, static analysis, security scanners, complexity metrics. None of that cares who wrote the code. In fact the whole LLM-coding evaluation ecosystem (HumanEval, SWE-bench, LiveCodeBench, Aider's benchmark, METR's task suite) is built on exactly this — run the model's output against hidden tests, measure pass rate. That's pure empiricism, and it's why we can say things like "model X solves 65% of SWE-bench Verified" with a straight face.

### What shifts in weight

**Correctness matters more, and subtly.** Human-written bugs tend to cluster around edge cases the author didn't think of. AI-written bugs cluster differently — plausible-looking code that references a function that doesn't exist, an API shape from two versions ago, a subtly wrong off-by-one in code that otherwise reads beautifully. "Looks reasonable" is a much weaker signal than with humans, so tests and type checks carry more weight and eyeballing carries less.

**Provenance and licensing.** Did the model regurgitate GPL code? Is there a training-data contamination issue where the model has "seen" your benchmark? These are empirical questions (similarity search, n-gram overlap against known corpora) but they're ones you rarely ask about human code.

**Security gets a new category.** Prompt injection, supply-chain attacks via hallucinated package names ("slopsquatting" — models invent a plausible package, attackers register it), and over-permissioned agent actions. Measurable, but you have to know to measure them.

**Maintainability is the big one.** This is where AI code often scores badly on the proxies: higher duplication, inconsistent abstractions across files (each generation starts fresh), dead code from abandoned approaches, comments that describe what rather than why. You can measure all of this — churn, duplication ratio, comment-to-code semantic alignment — and the numbers tend to be worse than hand-written equivalents unless someone's actively curating.

**A new empirical axis: edit distance from acceptance.** How much did the human have to change before merging? Copilot, Cursor, and friends track this internally; it's one of the better real-world quality signals because it captures correctness, style fit, and contextual appropriateness in one number.

### The subjective residue

Roughly the same size as before — taste questions about structure and naming — but there's an additional judgment call that's genuinely new: *how much to trust the code you didn't write and don't fully understand.* That's not quite subjective, more like a calibration problem, and the answer is mostly "trust the tests, not the prose."

---

## Q3: If you have 2 or 3 or 4 agents write code, and 2 or 3 or 4 different agents evaluate it, and different agents test it, will that produce meaningfully different and better output?

Sometimes yes, and the "when" is fairly well-characterized now.

### What consistently helps

**Generate-and-verify with a real oracle.** Multiple generators + a checker that actually runs the code (tests, type checker, compiler, fuzzer) beats single-shot generation pretty reliably. The gain comes from the oracle, not the multiplicity — you're basically doing rejection sampling against ground truth. AlphaCode, AlphaCodium, and most agentic coding setups work this way, and the lift is real (often 10-30 percentage points on benchmarks).

**Diverse generators on genuinely hard problems.** If the problem has multiple valid approaches and your generators actually explore different ones (different models, different temperatures, different prompts/personas), ensembling helps. The key word is *diverse* — four calls to the same model at temp 0.2 give you four nearly-identical answers and no benefit.

**Adversarial test generation.** An agent whose job is to break the code — write edge-case tests, fuzz inputs, find counterexamples — is one of the higher-leverage roles. This is closer to empirical evaluation than to "another opinion," which is why it works.

### What helps less than people hope

**LLM-as-judge for code quality.** Having agents evaluate each other's code without executing it is weaker than it sounds. Judges have correlated blind spots with generators (especially if they're the same base model), they rate confident-sounding wrong code highly, and they miss the same subtle bugs the generator made. There's decent research showing judge-model agreement with ground truth on code is mediocre — often 60-75% — and that agreement drops on exactly the hard cases where you needed help. Execution-based evaluation dominates.

**Debate and multi-agent discussion.** Mixed evidence. Sometimes produces better answers, often produces longer answers that sound better and aren't. On coding specifically, the gains are smaller than on reasoning tasks, probably because code has a ground truth that debate doesn't access.

**Adding more agents past ~3.** Diminishing returns set in fast. The second generator helps a lot, the third helps some, the fourth is usually noise. Cost scales linearly, quality scales logarithmically at best.

### The practical shape that tends to work

One or two diverse generators, an adversarial test-writer, and an executor/verifier that runs everything and reports objective results back. The executor is doing most of the real work — it's the thing connecting the system to reality. Adding more "reviewer" agents that just read code and opine rarely moves the needle much.

So: meaningfully better, yes, but mostly because you've built a feedback loop against reality, not because you've assembled a committee. The committee framing is the trap — it produces confident consensus on wrong answers. The verifier framing is the win.

---

## Q4: So 2-3 writers, never from the same family or base training, actual execution, adversarial review, but diversity and follow through enforced through instruction/prose & hooks

Solid synthesis. A few refinements worth flagging:

### Cross-family diversity is right but partial

Different base models (say, Claude + GPT + Gemini, or adding DeepSeek/Qwen) do have meaningfully different failure modes and training data — that's the real source of the diversity benefit. But within-family diversity via temperature, prompting, and role assignment still adds something on top, because same-family models at different temperatures or with different system prompts explore different regions of the solution space. So: cross-family as the primary axis, intra-family variation as a secondary multiplier.

### "Enforced through instruction/prose" is the weak link

Prompts asking agents to "be adversarial" or "disagree with the other agent" produce surface-level diversity — different wording, different framing — without much underlying divergence in approach. Stronger diversity levers:

- Different tools available to each agent
- Different context windows (one sees the full codebase, one sees only the diff)
- Different success criteria in their prompts (one optimizes for correctness, one for simplicity, one for performance)
- Different role constraints (one must write tests first, one must write implementation first)

Structural divergence beats instructed divergence.

### Hooks are doing more work than they get credit for

This is the part people underweight. Pre-commit hooks, type checkers, linters, fuzzers, coverage gates, mutation testing, dependency audits — these are your actual quality floor. They run deterministically, they don't get tired, they don't hallucinate agreement. An agent pipeline with weak hooks and strong prompts performs worse than one with strong hooks and mediocre prompts. Put your complexity budget there first.

### Adversarial review works best when it has teeth

An agent that writes failing test cases and runs them is doing adversarial review. An agent that writes a prose critique of the code is doing code review theater. The former finds real bugs; the latter finds style preferences. Worth being specific about which you're building.

### One missing piece: a referee or reconciliation step

When your 2-3 writers disagree, something has to pick. Options:

- Pick the one that passes the most tests (cheap, usually right)
- Pick the one with lowest complexity that passes all tests (better)
- Have a separate model diff them and pick (expensive, sometimes right)
- Merge the best parts (hard to get right, often produces Frankencode)

The "most tests passing, tiebreak on simplicity" heuristic is boring and effective.

### The shape you're describing

Diverse writers, real execution, adversarial tests, hook-enforced invariants — is roughly what the better agentic coding systems converge on. The main thing to push on: let the environment do the discrimination, not the agents. Agents propose; hooks, tests, and executors dispose.

---

## Q5: Do you know how fast it will be as a unit of code? How much memory it will take? How does memory refactoring work? How does performance refactoring work?

Four related but distinct questions.

### Can I predict speed from looking at code?

Roughly, not precisely. I can give you asymptotic complexity (O(n log n) vs O(n²)) reliably — that's a structural property of the algorithm. I can spot obvious perf killers: nested loops over large collections, repeated work that could be memoized, synchronous I/O in hot paths, allocations in inner loops, unbounded recursion.

What I can't do is predict wall-clock time. Modern performance is dominated by cache behavior, branch prediction, SIMD, memory bandwidth, JIT warmup, GC pauses, and hardware specifics — none of which are visible in the source. Two algorithmically-identical implementations can differ 10x in practice because one is cache-friendly and one isn't. Anyone who tells you they can eyeball microsecond-level performance from code is either working in a very narrow domain or bluffing. The honest workflow is: reason about complexity class statically, measure everything else.

### Can I predict memory usage?

Better than speed, but with caveats. Static footprint (struct sizes, array allocations with known bounds) is calculable — you can add up the bytes. Dynamic footprint (how much memory a running system will hold) depends on workload, allocator behavior, fragmentation, GC strategy, and caching decisions that aren't in the code. For a known input size and a language without hidden allocations (C, Rust, Zig), estimates are pretty good. For Python, JS, Java? The language runtime itself is hundreds of MB before your code runs, and every object has overhead you don't see (a Python int is ~28 bytes, a JS object has hidden class metadata, a Java object has a 12-16 byte header). Estimates are order-of-magnitude at best.

### Memory refactoring — how it actually works

The loop is: measure, find the biggest offender, fix it, re-measure. Tools do the finding — heap profilers (pprof, heaptrack, Valgrind massif, Python's tracemalloc, Chrome DevTools heap snapshots, Java's JFR). They tell you what's allocated, how much, and who allocated it.

Common fixes, roughly in order of impact:

**Switch data structures.** Replacing a list of objects with a struct-of-arrays layout, or a hash map with a sorted array for small sizes, can cut memory by 2-10x. Interning repeated strings. Using smaller integer types when the range allows. Bitsets instead of boolean arrays.

**Stop copying.** Pass by reference, use views/slices instead of copies, stream instead of materializing, use iterators/generators instead of building full lists. A lot of memory bloat is the same data existing in three places.

**Bound the unbounded.** Caches without eviction policies, queues without backpressure, logs without rotation. Add limits.

**Release eagerly.** In GC'd languages, drop references when done. In manual-memory languages, match allocations with frees and prefer arena/pool allocation for short-lived bursts.

**Fix leaks.** Separate category — these are bugs, not tradeoffs. Profilers find them by showing memory that grows without bound across a steady-state workload.

### Performance refactoring — how it actually works

Same shape: profile, find hotspot, fix, re-measure. The iron law is *don't optimize what you haven't measured.* Intuition about where programs spend time is wrong most of the time, even for experienced engineers.

Tools: CPU profilers (perf, pprof, py-spy, async-profiler, Instruments, VTune). Flame graphs are the standard visualization — they show you which functions eat the most time, stacked by call hierarchy. 90% of the work is finding the right 10 lines to change.

Fixes, roughly by leverage:

**Algorithmic.** O(n²) → O(n log n) dwarfs every other optimization. If you're doing quadratic work on large data, nothing else matters until that's fixed. This is where reading the code actually helps — you can often spot it.

**I/O and concurrency.** Batch database queries (the N+1 problem is probably the single most common real-world perf bug), add caching at the right layer, parallelize independent work, make blocking calls async. For most web/backend code, this tier is where the real wins live — CPU is usually not the bottleneck.

**Memory access patterns.** Sequential access beats random access by 10-100x due to caches. Struct-of-arrays beats array-of-structs for bulk operations. Reducing allocations in hot loops. This tier matters enormously for compute-bound code and barely at all for I/O-bound code.

**Reduce work.** Memoize pure functions, cache expensive computations, lazy-evaluate, short-circuit. Often the fastest code is the code that doesn't run.

**Micro-optimizations.** SIMD, loop unrolling, branch hint annotations, replacing virtual calls with direct calls. High effort, small wins, only justified when you've exhausted the above and you're in a genuine hot path.

### The meta-point for AI-generated code

Models (me included) are decent at the algorithmic tier — we can spot quadratic loops and suggest better data structures. We're mediocre at the memory-access and micro-optimization tiers because those require knowing hardware details and measuring. And we're bad at predicting which tier actually matters for your workload, because that requires a profiler.

So the honest answer to "can you tell me how fast this code will be": I can tell you its complexity class, flag obvious inefficiencies, and estimate memory within an order of magnitude. For anything more precise, the code has to run on real hardware with real inputs and be measured. The refactoring process is the same for human and AI code — it's profile-driven, not intuition-driven, and the profiler is doing the real work.

---

## Q6: But we could create testing and optimization systems that collect all these data points and feed them back into various optimization and refactoring workflows as code gets written, committed, and moves further and further down the SDLC pipeline from commit to test to dev to QA…..

Yes, and the pieces mostly exist — what's missing is the integration and the feedback loops going back to the code-generation step.

### What already exists and works

**Continuous profiling in production is a solved problem.** Pyroscope, Parca, Datadog Continuous Profiler, Google Cloud Profiler — they sample running processes constantly, aggregate flame graphs, attribute CPU and memory to specific functions and git commits. Netflix, Google, and Meta have run variants of this for a decade. You can already answer "which function got slower between v1.4.2 and v1.4.3" with a dashboard.

**Benchmark-gated CI exists.** Criterion (Rust), JMH (Java), pytest-benchmark, Google Benchmark all integrate with CI to flag regressions. Touca, Bencher, Codspeed are newer services doing this as a product — they'll fail a PR if p99 regresses by more than X%. Rust compiler and V8 teams gate merges on this.

**Coverage, mutation testing, fuzzing-in-CI are mature.** Codecov, Stryker/PITest, OSS-Fuzz's ClusterFuzzLite. You can require 80% coverage, require mutation score above a threshold, require new code to survive N hours of fuzzing before merge.

**Memory regression detection is thinner but real.** Valgrind massif in CI, heaptrack diffs, Java's JFR with baseline comparisons. Bloaty for binary size. Android and iOS both gate releases on app-size budgets.

**Observability-to-commit correlation exists.** Sentry, Honeycomb, and others tie production errors and latency spikes back to specific deploys and commits. "This commit caused the p99 spike" is a query you can run today.

### What's partial

**Cross-stage data aggregation.** Each stage has its own tools with its own storage. Getting "this function, across unit tests, integration tests, staging load tests, and prod" into one view requires custom plumbing. Some APM vendors are moving toward this but nobody has nailed it.

**Automated refactoring based on profile data.** Profile-guided optimization (PGO) exists for compilers — you run a workload, the compiler uses the profile to inline and lay out code better. LLVM, GCC, Go, and .NET all support it. But PGO operates below the source level. Source-level refactoring driven by profiles ("this hot function should be rewritten to avoid allocation") is mostly manual.

**Flaky test detection and quarantine.** Datadog, Trunk, BuildKite all do some version. Works okay, not great.

### Where the gap is

The feedback loop from "production observed this code is slow/memory-hungry/buggy" back to "the next time someone (human or AI) writes similar code, don't do that" is almost entirely absent as automation. It happens in human heads and postmortems. Codifying it is hard because it requires:

**A knowledge base of "what went wrong and why"** — linking production incidents to code patterns, keyed by something more abstract than file+line. This is what SRE postmortems try to produce; very little is machine-readable.

**Pattern-matching from new code to past incidents** — "this new PR has the shape of the N+1 query that took us down last year." Static analyzers do a crude version (Semgrep rules, CodeQL queries) but those rules are written by humans after the fact. Auto-generating them from incidents is an open problem.

**Injecting that knowledge into the code-generation step** — so when an AI agent writes a database query, it knows your codebase's history with N+1s and preemptively batches. Current agents have no memory of your prod incidents. RAG over postmortems is the obvious move and almost nobody does it well.

**Closing the loop on AI-generated code specifically** — tracking which agent, which prompt, which model produced which code, and correlating that with downstream quality signals (bugs, perf, rework). You'd learn "generator A writes code that has 3x the memory regressions of generator B on our workload" and route accordingly. This requires provenance tracking that barely exists.

### The shape of the system you're describing

**Commit-time:** static analysis, type checks, unit tests with coverage and mutation gates, micro-benchmarks with regression thresholds, memory-allocation diffs. All feed a data store keyed by commit SHA and the specific functions touched.

**Pre-merge:** integration tests with profiling enabled, fuzzing budget, dependency audit, binary-size check. Flame graphs stored and diffed against main.

**Staging:** load tests with production-shaped traffic, continuous profiling, chaos injection. Results tagged to commit.

**Production:** continuous profiling, APM, error tracking, real-user metrics. Tagged to deploy and commit.

**Feedback layer** (the part that doesn't really exist yet): all of the above aggregated per-function and per-pattern, with an API that code-generation agents query before writing. "Before I write a cache here, what's the eviction policy our codebase prefers? What cache-related incidents have we had? What's our current memory headroom?" Plus a learning loop that turns incidents into new lint rules or new RAG entries automatically.

### Why it isn't built yet, mostly

The instrumentation is expensive to set up per-codebase, the payoff is diffuse, it requires cross-team coordination (SRE + platform + dev tools), and until recently there was no obvious consumer of the aggregated data beyond humans-reading-dashboards. AI coding agents are the first consumer that could actually use a structured, queryable history of "what went well and badly in this codebase" at code-writing time. That's a genuinely new architectural opportunity, and it's where the next wave of dev-tools startups would likely focus — not on generating code faster but on giving generators memory of their own codebase's production reality.

### Bottom line

Yes, you could build this. Most of the raw data pipelines exist. The unbuilt part is the feedback into generation. That's a real engineering project, not a research problem — but it's a substantial one, probably 12-24 months of work for a serious team to do well, and the hard part is organizational (getting the data sources to cooperate) more than technical.

---

## How this informs Pantheon — integration notes

**Directly validates Round 4 REQ decisions:**
- **Q3/Q4 content → REQ-094 + REQ-095.** The "cross-family diversity as primary axis," "execution-based evaluation dominates," "structural divergence beats instructed divergence," and "let the environment discriminate" findings align one-for-one with: protocol-level convergence via `submit_review` MCP tool (structural, not prose), N≥3 cross-lineage panel commitment (cross-family primary), planted-defect methodology (execution/ground-truth-based rather than judge-based), hook-enforced YAML and runtime validation (REQ-074 Pass 1 credential detection, REQ-069 subprocess reaping, REQ-080 MAD outlier detection).
- **Q6 closed-loop feedback → REQ-093 training-data forward-anchor.** The "feedback loop from production back into generation" description IS what REQ-093's training-data extraction pipeline aspires to — capture per-defect hit/miss records with full turn context, extract as fine-tuning examples. Pantheon is architecturally positioned as that feedback mechanism, not just a validation harness.
- **Q5 profiling-driven refactoring → REQ-092 OTel integration.** The "measure, find biggest offender, fix, re-measure" loop requires the OTel span/metric/log infrastructure REQ-092 anchors for Gate 3+.

**Commercial positioning implications:**
- The Q6 "12-24 months of work for a serious team; the hard part is organizational" framing is exactly Pantheon's competitive window. Closing the loop is a substantial engineering project most orgs won't tackle. A ready-made substrate that delivers it is commercially differentiated.
- The "AI coding agents are the first consumer that could actually use a structured, queryable history" framing is the Old Iron commercial thesis verbatim — customer codebases are heterogeneous, compliance-bound, and have rich production signal that current agentic tooling ignores.

**Future REQ candidates surfaced by this artifact:**
- Production-incident-to-defect-pattern extraction pipeline (Q6 "pattern-matching from new code to past incidents") — belongs to a future Gate 5+ scope that validates the feedback-into-generation loop directly.
- Provenance tracking for AI-generated code (Q2 "agent/prompt/model produced which code" correlation) — extends REQ-068 `provider` tracking to capture generator identity alongside billing identity.
- Flake-quarantine UX (Q6 "flaky test detection works okay, not great") — extends REQ-047/048 flake corpus with operator-facing quarantine tooling.

**Pattern observation for Pantheon's design vocabulary:**
- "Committee framing is the trap; verifier framing is the win." (Q3) This phrasing is worth lifting into commercial positioning and internal design discussions — it crisply captures why Pantheon's execution-based gate validation (runs commands, measures outcomes, treats flakes as first-class data) is architecturally distinct from "N models vote on quality" alternatives.
