# Raw peer output: local-inference-buy-vs-rent.md unit 2 (lines 131-284)

Section 3 throughput reality, section 4 the NVIDIA alternative, section 5 the cloud pivot and retainer product.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**1.** Section 3 quotes "174.4 tok/s prompt processing," "14.5 tok/s decode," "404.77GB peak memory," at "32K context plus 128 generated tokens" (135-140), on GLM-5.2 oQ4. **It states quantization and context length. It does NOT state batch size or concurrency. So the 14.5 tok/s figure is not comparable to the 15 tok/s/stream production floor under 4-way batched load.**

**2.** The second throughput claim is "about 12 tok/s" at "mixed-four-bit" and "around 74.5K context" (142). No exact model/build, batch size, generated-token count, or concurrency. Reported second-hand, not reproduced.

**3.** Line 154 concludes "throughput per dollar per watt may pencil" and "at $25,000, it does not." **There is no cost model behind that statement anywhere in 131-284: no cost per token, no amortization period, no utilization assumption, no break-even calculation.**

**4.** NVIDIA pricing is stale and internally inconsistent (170-173). Current sources show NVIDIA Marketplace at $13,250 (Thunder Compute), PNY ~$11,360, Newegg ~$12,099, B&H $13,349+, while Tom's Hardware and Igor's Lab report NVIDIA at $16,000. **The document mixes these into one table and cites $16,000 to one source while its Thunder Compute citation currently says $13,250.**

**5.** The build cost (178) is a parts budget, not an economic model. **It omits power cost, cooling cost, install cost for the 240V circuit, depreciation, resale, utilization, and any cloud comparison.** It mentions the electrical constraint (193-198) without costing it.

**6.** "NVIDIA wins prompt-processing throughput by an order of magnitude" (208) is **unsourced in this range.** No NVIDIA throughput benchmark is quoted, and **no hardware anywhere is tested against the 15 tok/s/stream floor.**

**7.** Section 5's retainer economics are qualitative only (232): warm pool means idle GPU cost, a handful of conversations will not reach crossover, meter or cap token spend. **No cost per client per month, no token budget per client, no model API cost, no gross margin, no break-even client count.**

**8.** "Realistic cold start: 15 to 45 minutes" (224) is precise but unsourced.

**Bottom line: lines 131-284 do NOT contain a cost model.** They contain throughput anecdotes, current-ish parts pricing, electrical feasibility notes, and qualitative retainer logic.

---

## GEMINI (strategic angle)

**1. Section 3 selects framing to guarantee failure.** It **deliberately tests a 418.1GB model on a 512GB machine** (135-139), choking on memory overhead, to justify 14.5 tok/s (138) and a "frustrating" verdict (154). *What it was for:* evaluating Mac Studio viability. *Problem persists:* yes. *Replacement:* **throughput data for models properly sized for the architecture (70B-class), not edge-case capacity stress tests.**

**2. Section 4 is a strawman constructed to be rejected.** It anchors entirely on enterprise RTX PRO 6000 cards **at peak shortage pricing** ($16,000, 173), leverages the 240V constraint (198) for a 4x build, then points out the bloated 418GB model **still would not fit** (208). *Replacement:* **a realistic evaluation of consumer-grade multi-GPU rigs (4090/5090) that practitioners actually use.**

**3. Section 5 is a rationalization for abandoning infrastructure.** It claims the RAG layer is the real asset (238), **but admits under "Context Rot" (263) that keeping it updated is bespoke manual labor.** Rebranding that as "we maintain a living model of your operations" (266) is **unscalable consulting disguised as a recurring SaaS product.** *Replacement:* a scalable product definition that does not rely on infinite manual context maintenance.

**4. The chilling-effect argument belongs in section 5**, attacking line 244 ("token volume high enough that spend exceeds infrastructure cost"). The document assumes API tokens are a neutral commodity (238). **They are not. Metered inference prevents zero-marginal-cost, high-iteration agentic loops because the cost of failure is literal money. The true crossover threshold is not bulk token volume; it is the operational chilling effect that prevents the work from being attempted.** Adding this invalidates the premise that you can simply "rent the intelligence" (248) for autonomous workflows.

**5. The one thing to keep: lines 148-151, "Benchmarks Do Not Predict Task Outcomes."** Recognizing that models fail at **"integration and specification-reading, not code generation"** (151) is a sharp, accurate insight that must survive any rewrite.

---

## DEEPSEEK (adversarial logic angle)

Asked whether benchmarking the largest model that fits is legitimate characterization or a rigged test.

> The reviewer is mostly right: the test is **not illegitimate as a stress test, but it is being over-read as a
> representative benchmark.**
>
> **Rule:** a maximum-capacity test answers *"What is the boundary performance when running the largest model that
> fits?"* It does **not** answer *"How well does this machine handle normal local inference?"*
>
> **Legitimate when:** you are explicitly asking a capacity question, comparing machines at their respective limits,
> or looking for the upper bound of model size.
>
> **Misleading when:** you pick the largest model that technically fits, get a poor number caused by memory pressure,
> and generalize to the machine's overall adequacy; or imply smaller properly-sized models would perform similarly.
> **They will not: leaving headroom for KV cache, batching, and OS overhead often improves throughput dramatically.**
>
> So 14.5 tok/s is a valid **capacity-limit datapoint**, but not valid evidence the machine is inadequate for
> interactive use, **unless the intended workload really is "the largest model that can fit."**

**Two-peer convergence, with DeepSeek supplying the rule that makes the objection precise.** The number is not wrong;
**the inference drawn from it is.** And the fix is cheap: keep the capacity datapoint, label it as one, and add a
properly-sized measurement to answer the question the document actually asks.
