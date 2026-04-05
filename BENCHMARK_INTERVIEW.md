# Benchmark Domination — Interview Transcript

Raw questions and answers from the specification interview. Used to produce BENCHMARK_SPEC.md.

---

## Q1: Parser Scope — Permanent Feature or Benchmark Shim?

**Q:** Is the hardware NL parser a permanent LogicAffeine feature or a benchmark-specific shim?

**A:** Permanent feature. But the user questioned why we need a new parser — "Don't we have a natural language parser?"

**Clarification:** The LOGOS parser handles linguistic English ("Every dog runs" → FOL). FVEval uses hardware specification language ("Both sig_F and sig_H are high, or all bits of sig_I are high"). The gap:
1. Hardware vocabulary: "bitwise NOR", "reduction XOR", "all bits of"
2. Signal names: `sig_X` not recognized as nouns
3. Numeric temporal bounds: "5 to 9 clock cycles later"
4. Verilog constants: `1'b1`
5. Nested expression trees described in English

**Resolution:** Extend the existing LOGOS parser (Path A), not build a separate one.

---

## Q2: Proof Strength — Bounded vs Unbounded Z3

**Q:** Our Z3 equivalence is bounded (checks up to N timesteps). Jasper does unbounded. How do we frame this?

**A:** Both — tiered. Bounded Z3 first, then attempt unbounded via k-induction/IC3 as a bonus column.

---

## Q3: Endgame

**Q:** What's the endgame for these results?

**A:** NVIDIA pitch/demo. Needs impressive numbers and demo capability.

---

## Q4: Jasper Access

**Q:** Do you have access to Cadence JasperGold?

**A:** No. Use Z3 equivalence checker as verification backend.

---

## Q5: Benchmark Scope

**Q:** Phase 1 only or all benchmarks?

**A:** All benchmarks. Full sweep.

---

## Q6: Block Marker for Hardware Mode

**Q:** How should we switch the parser into hardware mode?

**A:** `## Specification` block marker. Like `## Main` for programs.

---

## Q7: FOL Extensions

**Q:** Should extended FOL support full bitvector theory or just SVA-scoped subset?

**A:** Full bitvector theory. BV(N) types, extract/concat, arithmetic. Maps to Z3 QF_BV.

---

## Q8: Temporal Delay Design

**Q:** Should delays support ranges from day one?

**A:** Yes. Delay(n) + DelayRange(min,max) as two separate constructs.

---

## Q9: Lexicon Strategy

**Q:** Should hardware entries go into the existing lexicon.json or a separate file?

**A:** Extend existing lexicon. But the user suggested exploring a block-based approach (like `## Main`) for context switching. Discussion led to `## Specification` as the context marker that activates hardware vocabulary.

The user emphasized: "We should support all the versions of the language and allow mix and matching, just like English."

---

## Q10: NL Grammar Style

**Q:** Should we match FVEval phrasing or define our own clean grammar?

**A:** Both — flexible parser. Support FVEval's exact phrasing AND our own grammar. "Think we must support all the versions of the language and allow mix and matching, just like English." The existing LOGOS parser already handles a lot of English; leverage that.

---

## Q11: Recursive Expression Parsing

**Q:** How to handle deeply nested expression trees in NL?

**A:** Recursive descent on NL. The user also suggested: "Look at our existing Futamura projections — see if we can use those. What if we wrote an interpreter for this?"

**Noted for future:** Write a LOGOS interpreter for hardware NL, then P1-specialize it into a compiled SVA generator. Stunning demo of Futamura projections but probably not for v1.

---

## Q12: LLM Verifier Feature

**Q:** Should we ALSO verify LLM-generated SVA (take GPT-4's output and prove/disprove it)?

**A:** No. Compete directly. Position as replacement, not complement.

---

## Q13: Error Handling

**Q:** How should we handle NL we can't parse?

**A:** Flag as ambiguous. Shows we KNOW we don't know (vs LLMs that hallucinate confidently).

---

## Q14: Ambiguity Output Format

**Q:** What should the ambiguous spec output look like?

**A:** Both — structured diagnostic (machine-readable) AND Socratic explanation (human-readable). Same analysis, two renderings.

---

## Q15: RTL Pipeline Architecture

**Q:** For VERT/AssertionBench code-to-assertion, should it go through FOL?

**A:** RTL → SVA directly for output, RTL → FOL for Z3 verification. Best of both worlds.

---

## Q16: Unbounded Proof Investment

**Q:** How much effort on hardening k-induction/IC3?

**A:** Bounded first, but also invest in unbounded. Harden k-induction and IC3.

---

## Q17: Testbench Parsing

**Q:** Should we parse FVEval's testbench modules for signal metadata?

**A:** "Don't force us onto the tests." We want to win with natural language. If parsing testbenches for signal widths is a universal improvement, do it. But the system shouldn't depend on the benchmark's testbench format.

---

## Q18: AssertionBench Scope

**Q:** All 101 designs or curated subset?

**A:** All 101 designs.

---

## Q19: Feature Gating

**Q:** Feature-gate hardware extensions behind a Cargo flag?

**A:** Always on. No feature gate. Must ensure zero regression on existing tests.

---

## Q20: VERT Strategy

**Q:** Separate RTL parser or skip VERT?

**A:** Separate RTL behavioral parser. Two front-ends (NL + RTL), one formal core.

---

## Q21: Relationship to CRUSH_ASSERTIONFORGE.md

**Q:** How does this relate to the existing CRUSH_ASSERTIONFORGE.md?

**A:** That one is already complete. This is a new spec. Name it appropriately.

---

## Q22: Target Numbers

**Q:** What would make you feel like we crushed the benchmarks?

**A:** 100% or flag as ambiguous. Binary: either produce correct SVA (verified by Z3) or explicitly flag as ambiguous/unparseable. No wrong answers, ever.

---

## Q23: Test Specification Detail

**Q:** Include RED tests in spec?

**A:** Yes — describe what we are testing for and WHY a test proving that is sufficient. Don't write the entire test.

---

## Q24: Scope Concern

**Q:** Full send or phase it?

**A:** Full send, no compromise.

---

## Q25: Latency Reporting

**Q:** Report latency/throughput numbers?

**A:** Report our own latencies but don't benchmark LLMs (PhD students will do that later). We aren't paying for tokens to test them.

---

## Q26: Benchmark Harness

**Q:** Rust binary, Python, or both?

**A:** Unobtrusive. Separate crate (sva-bench or similar). Main goal is showing we win.

---

## Q27: AssertionBench Approach

**Q:** How ambitious for AssertionBench?

**A:** Full behavioral analysis. Parse always blocks, detect FSMs, infer invariants, generate comprehensive assertions.

---

## Q28: Spec Format

**Q:** How should the spec be structured?

**A:** Single markdown file.

---

## Summary of Key Decisions

| Decision | Choice |
|----------|--------|
| Parser architecture | Extend existing LOGOS parser (Path A) |
| Block marker | `## Specification` |
| FOL extensions | Full bitvector theory |
| Temporal delays | Delay(n) + DelayRange(min,max) |
| Lexicon | Extend existing lexicon.json |
| NL grammar | Flexible — FVEval phrasing + our own |
| Expression parsing | Recursive descent on NL |
| Feature gating | Always on, no feature flag |
| Error handling | Flag as ambiguous (both formats) |
| RTL pipeline | SVA directly, FOL for verification |
| Proof strength | Bounded first, then unbounded k-ind/IC3 |
| Jasper | None — Z3 as verification backend |
| LLM verifier | No — compete directly |
| Benchmarks | All: FVEval + VERT + AssertionBench |
| AssertionBench | All 101 designs, full behavioral analysis |
| Target | 100% correct or flagged as ambiguous |
| Harness | Separate benchmark crate |
| Scope | Full send, no compromise |
| Latency | Report ours, PhD students benchmark LLMs later |
