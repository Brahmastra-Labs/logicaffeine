# The "Break My Code with AI" Challenge — Rules, Intake, and Launch Readiness

**Status: not open yet.** The challenge launches when every gate in section 5 holds, and two are
still open: gate 6 (the correctness core measured at zero non-equivalent mutation survivors) and
gate 7 (a recorded fuzz soak with no crashes). Until both close, the rules below describe the
challenge as designed rather than a standing offer, and a break is not a win. It is still a bug
worth reporting: open a GitHub issue, or report a remotely triggerable break privately as
`.github/SECURITY.md` describes.

The rules of the public challenge, what it claims and where it stops, the process that turns every
win into a permanent test, and the gates that must hold before the challenge launches.

---

## 1. The claim

> Make LOGOS produce a **wrong answer**, or **crash it**, *while the entire test suite still
> passes.* If you can, you win. If you can't — that is the proof.

Three properties back it:

1. **The correctness core is mutation-tested.** `.config/mutants.toml` scopes mutation testing
   (`examine_globs`) to the trusted kernel, the SAT/DRAT certifier, the wire codec, and the crypto
   primitives, and the target is that every non-equivalent mutation of those files is caught by a
   test. `.config/mutants/known-survivors.txt` records each file's measurement and every surviving
   mutant with its verdict; `mutation_lock` fails when an enrolled file disappears, when a kernel
   source file is neither enrolled nor exempted with a reason, and when a recorded survivor has no
   verdict. Gate 6 below states which files are measured.
2. **No input crashes the front door.** `never_panic_lock` feeds a curated adversarial corpus to
   every public entry point: FOL transpilation, the lexer, the interpreter, the VM, compilation to
   Rust and to WebAssembly, and the three wire decoders. It adds random bytes to the lexer, the
   parser and the wire decoders, and randomized source text to the lexer and both parsers, and
   requires each to return a value or a structured error. Its `Surface` catalog has no wildcard arm,
   so a new entry point fails the build until it is swept.
3. **Every execution tier that accepts a program agrees on it.** `run_all_tiers` runs a behavioral
   test on the tree-walker and the VM and requires the same output and the same error text
   (`tier_differential_default`), and `tier_coverage_ratchet` lets the count of single-tier tests
   only fall. `wasm_aot_lock` holds the direct WebAssembly backend to the tree-walker over its
   corpus, and each ahead-of-time backend has its own differential (`e2e_differential` for Rust,
   `c_bytecode_lock` for C, `riscv_aot_lock`, `cuda_aot_lock`).

---

## 2. Rules of engagement

- **Precondition:** the full suite (`./scripts/run-all-tests-fast.sh`) is green on the commit
  under challenge. A "break" that requires *editing a test* or *removing a lock* does not
  count — the claim is conditioned on the suite passing **unchanged**.
- **A win is precisely one of:**
  1. A program whose output or error text differs between two execution tiers that both accept it, or
  2. Any input that panics, aborts, or overflows the stack of a public entry point, or
  3. A wrong answer from the correctness core (a false theorem, an accepted unsound refutation, a
     corrupted wire value, a wrong hash) with the suite passing, or
  4. A semantics-preserving transform (optimization on vs off, an unused binding, a re-run) that
     changes output (`metamorphic_lock`).
- **Out of scope:** the web application (`apps/logicaffeine_web`) and the exploratory SAT/coNP
  research suites, which `./scripts/run-research-tests.sh` runs outside the full suite. Performance
  or DoS that *terminates* (no crash) is a hardening note, not a win. An *unbounded* allocation or
  recursion that aborts is a win: `oom_injection.rs` runs every wire decoder under a capped
  allocator because a 476 GB `T_SET_STRINGS` preallocation and a 4.26 GB decompression bomb were
  found exactly that way.
- **Equivalent mutants are not wins.** "Change `|` to `^` in SHA `ch`" passes the suite because
  it is mathematically identical: that is documented in `.config/mutants/known-survivors.txt`, not a hole.
- **Submitting:** open a GitHub issue with the input or program and the commit it breaks. Report a
  remotely triggerable break (a wire-decoder crash or an allocation bomb) privately, as
  `.github/SECURITY.md` describes.

---

## 3. What the challenge claims, and where it stops

*"Mutation testing is scoped to the trusted kernel, the SAT certifier, the wire codec and the crypto
primitives, and every surviving mutant is enumerated with its verdict; equivalent mutants are
listed, not hidden. No input crashes the front door. Every execution tier that accepts a program
agrees with the tree-walker on it. Running these tools already found and fixed a stack overflow on
deeply nested input, two lexer integer underflows, a 476 GB wire-decode preallocation and a 4.26 GB
decompression bomb. Try to break it — and when you do, your input becomes a permanent test."*

The limits, stated with the claim:

- **Mutation is scoped to the correctness core**, never the whole workspace, and the claim for a
  file holds only once `.config/mutants/known-survivors.txt` records it at zero non-equivalent
  survivors (gate 6).
- **Equivalent mutants exist.** A mutation that leaves the code semantically identical cannot be
  killed by any test; each is listed with its justification, so the claim is "zero killable
  survivors", not a naive 100%.
- **The never-panic lock proves the front door**, not every internal `unwrap`: no byte string fed to
  a public entry point unwinds. `catch_unwind` cannot see an abort or a stack overflow; those are the
  fuzz targets' job (gate 7).
- **Out of memory means a clean abort or a clean error**, not recovery: Rust aborts on allocation
  failure, so `oom_injection.rs` proves no decoder requests an allocation above its cap.

---

## 4. Intake — every win becomes a permanent test

When a submission wins, it makes the suite permanently stronger. The process, every time:

1. **Reproduce** the exact input/program on the challenged commit. Confirm it breaks one of the
   properties in section 1 with the suite green.
2. **Minimize** it to the smallest reproducer:
   - crash input → `cargo +nightly fuzz tmin <target> <artifact>` in the target's fuzz crate
     (`fuzz_lex` and `fuzz_parse` in `crates/logicaffeine_language/fuzz`, `fuzz_wire_decode` in
     `crates/logicaffeine_compile/fuzz`), or by hand.
   - execution-tier divergence / wrong answer → shrink the program to the minimal failing case.
3. **Seed it permanently** into the lock that owns the property, so it can never regress:
   - front-door crash → add the bytes to `curated_corpus()` in `never_panic_lock.rs` **and** to the
     fuzz target's `regressions/` directory.
   - execution-tier divergence → add the program to `tier_differential_default.rs`, or to the corpus of the
     backend's lock (`e2e_differential`, `c_bytecode_lock`, `wasm_aot_lock`, `riscv_aot_lock`,
     `cuda_aot_lock`).
   - wire/decoder crash → `robustness_properties.rs` + a `fuzz_wire_decode` regression; an allocation
     bomb also joins `oom_injection.rs`.
   - correctness-core wrong answer → a targeted test in `crates/logicaffeine_tests` (a unit body
     under `unit/<crate dir>/` for white-box access) that kills the corresponding mutant.
4. **Fix the implementation**, never the test: the test defines the spec. Confirm the new seed
   goes red → green.
5. **Verify** the full suite is green again, then commit. Credit the finder.

The suite only ever grows teeth: a fixed win is a build-checked regression test forever.

---

## 5. Launch gates

Launch only when every gate holds.

| # | Gate | Held by | State |
|---|---|---|---|
| 1 | Full fast suite green | `./scripts/run-all-tests-fast.sh` | Per commit: run on the launch commit |
| 2 | No input crashes the front door | `never_panic_lock` | In the suite |
| 3 | Execution tiers agree by default | `run_all_tiers`, `tier_differential_default`, `tier_coverage_ratchet`, `wasm_aot_lock` | In the suite |
| 4 | Wire round-trip identity and decode totality | `robustness_properties` | In the suite |
| 5 | Coverage measured, per-crate floors held | `scripts/test/run-coverage.sh` (cargo-llvm-cov) writes `.config/coverage/crate-summary.json`; `scripts/test/check-coverage-floors.sh` holds it to `.config/coverage/thresholds.toml` | Manual run; no workflow runs it |
| 6 | Correctness core at zero non-equivalent survivors | `scripts/test/run-mutation-core.sh`, `.config/mutants/known-survivors.txt`, `mutation_lock` | Open: `reduction.rs`, `recheck.rs`, `certifier.rs` and `xor_drat.rs` have no recorded measurement at zero, and the kernel files recorded at zero have been substantially rewritten since their measurement |
| 7 | cargo-fuzz targets run clean over a soak | `scripts/test/run-fuzz.sh --time <seconds>` over `fuzz_lex`, `fuzz_parse`, `fuzz_wire_decode` | Open: no soak is recorded, and no workflow runs the targets |
| 8 | Allocation failure and solver starvation degrade cleanly | `oom_injection.rs`; `solver_fallback_lock` (a starved solve returns the correct verdict or an honest give-up, checked against a truth-table oracle) | In the suite |
| 9 | Limits stated with the challenge | Section 3 | Stated |
| 10 | Intake process and finder credit | Section 4 | Documented |

**The two hard gates before launch:** #6 (the correctness core at zero non-equivalent survivors)
and #7 (a real fuzz soak with no crashes).

---
[Docs index](../README.md) · [Root README](../../README.md) · [Changelog](../../CHANGELOG.md)
