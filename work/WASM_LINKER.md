# WASM AOT Linker (P2) — architecture + slice roadmap

Goal: features the self-contained AOT emitter can't honestly reimplement in emitted wasm — arbitrary
`BigInt`, the real `FxHashMap` order, the scheduler/relay transports — work Logos→native-wasm by
STATICALLY LINKING the existing Rust runtime (`logicaffeine_base`/`data`/`system`), compiled once to
`wasm32-unknown-unknown`, into the program. One module, one linear memory, one allocator, no second
implementation to keep in sync with the VM.

Code lives in `crates/logicaffeine_compile/src/vm/wasm/link.rs` (no new crate).

## Toolchain (verified present on this host)

- `rustc 1.96` + the `wasm32-unknown-unknown` target.
- `rust-lld` at `$(rustc --print sysroot)/lib/rustlib/<host>/bin/rust-lld`, driven `-flavor wasm`.
- Proven: `rustc --emit=obj` produces a relocatable wasm object; `rust-lld` links cross-object calls
  into one module with a single shared `memory` export.

## The relocatable object format (reverse-engineered from `rustc --emit=obj`)

A relocatable object = a normal module whose function-index references are placeholders, plus:
- **`linking` custom section**: metadata version 2 + a `WASM_SYMBOL_TABLE` (id 8). Each function is a
  symbol — DEFINED (`main`, flags 0, module index, name) or UNDEFINED (a `logos_rt_*`/host import,
  flags `0x10`, import index, name from the import).
- **`reloc.CODE` custom section**: target section index, then one `R_WASM_FUNCTION_INDEX_LEB` (type 0)
  per code function-index reference — `(offset within the code section content, symbol index)`. Each
  `call` target is emitted as a FIXED 5-byte padded LEB the linker overwrites in place.

Our encoder computes reloc offsets from the actual emitted bytes (not hardcoded), so it stays correct
as bodies grow past 1-byte LEBs.

## Slices (each independently green by construction)

- **S1 — the link pipeline (DONE, green).** `emit_probe_object()` hand-emits a relocatable object
  whose `main() -> i64` calls the undefined `logos_rt_probe`; `build_runtime_object()` compiles a
  `logos_rt_probe(x)=x+1` runtime to a wasm32 object; `link_objects()` runs `rust-lld`. Test
  `link::tests::probe_object_links_against_runtime_and_runs` links them and asserts `main() == 42` in
  wasmi. Retires the biggest risk: our own relocatable encoder is wasm-ld-consumable. Skips cleanly if
  the toolchain is absent.
- **S2 — shared memory (DONE, green).** `emit_load_object()` imports `env.__linear_memory` and
  `i64.load`s a pointer the runtime returns after writing `42` into a static that `rust-lld` places in
  that shared memory. Test `emitted_code_reads_runtime_written_shared_memory` asserts the load yields
  42 — one linear memory across both objects. REMAINING (S3): the ALLOCATOR half — the emitted side
  must stop using its bump allocator and call the runtime's `malloc`/`free` when the runtime is linked.
- **S3 — the allocator + the Text-handle ABI (DONE, green).** Two parts:
  - **S3a**: a `no_std` + bump-`#[global_allocator]` runtime `Box`-allocates; our emitted code reads it
    — the allocator links + runs on `wasm32-unknown-unknown` `--emit=obj`.
  - **S3b**: the runtime multiplies 10¹²×10¹² = 10²⁴ (`__multi3`), `to_string()`s it (real `core::fmt`
    + `liballoc`), and builds a `Text` in the emitter's `[len][cap][data_ptr]` layout in shared memory;
    our side reads the exact decimal. Test `runtime_returns_a_text_handle_our_side_reads`.
  - **The link line now mirrors rustc's**: objects + real `liballoc`/`libcore`/`libcompiler_builtins`
    rlibs + `--export=__heap_base --export=__data_end -z stack-size=1048576 --stack-first --gc-sections`,
    so the runtime may use `std`/`String`/`i128`. The two allocator-shim symbols a lib `--emit=obj`
    leaves undefined are supplied by synthesized objects: `__rust_no_alloc_shim_is_unstable_v2` (no-op,
    scanned from our runtime's imports) and `__rust_alloc_error_handler` (trap = abort-on-OOM, its
    mangled name DERIVED from the marker's `___rustc`-crate prefix since liballoc — not our object —
    imports it). `link_objects` does this automatically.
- **S3-flagship — REAL arbitrary-precision BigInt (DONE, green).** The runtime is a `std` crate using
  the SAME `logicaffeine_base::BigInt` the VM uses (compiled to wasm32 via `cargo build -p
  logicaffeine-base --target wasm32-unknown-unknown`, then `rustc --emit=obj --extern
  logicaffeine_base=<rlib>`), with `std::alloc::System` as the global allocator (→ linked `dlmalloc`,
  real `free`). Test `runtime_computes_real_base_bigint_beyond_i128`: `emit_bigint_object()` drives the
  `logos_rt_bigint_from_i64`/`mul`/`to_text` ABI to compute `(10¹²)^8 = 10⁹⁶` (97 digits, far past
  i128) and reads back the exact decimal. NO reimplementation, NO divergence from the VM. Two link
  modes now: `link_objects` (minimal rlibs + both shims, for `no_std`+bump runtimes) and
  `link_objects_with_rlibs(objs, rlibs, emit_handler_shim, dir)` — the BigInt test passes ALL sysroot
  rlibs + base/bumpalo with `emit_handler_shim=false` (libstd DEFINES the OOM handler → a shim would
  duplicate it). `wasm_all_sysroot_rlibs()` returns the whole sysroot rlib set.
- **S4 — wire into `compile_to_wasm`.**
  - **S4-foundation — the relocatable transform (DONE, green).** `reloc.rs::module_to_relocatable`
    converts a finished self-contained module (from `module.rs`) into an LLVM-compatible relocatable
    object: a total-or-refuse instruction decoder rewrites every `call` target to a 5-byte padded LEB
    and appends the `linking` symbol table (functions in index order: host imports UNDEFINED, defined
    functions `main`/`fnN`) + `reloc.CODE`. A POST-PASS (zero changes to the shared `emit_body`, so no
    risk to the crypto/guide/benchmark corpus). Test `relocatable_links_and_matches_standalone_over_a_corpus`:
    calls/recursion(fib)/collatz/mutual-arithmetic each convert → link → run BYTE-IDENTICAL to
    standalone. SOUND: refuses (Err → caller falls back) any module with globals/memory/data/element
    sections (a promoted `Let mutable`, the heap model, closures) — those need global/data/type
    relocations + global & data symbols, the next sub-slices. Strips the pre-link export section (its
    index would be stale post-shift); `main` re-exported via its symbol's EXPORTED flag + `--export=main`.
  - **S4-next — the dynamic-Int model (DONE, green).** `assemble_program_linked` lowers an integer
    `Op::Pow` to the real `logicaffeine_base::BigInt` runtime — `logos_rt_bigint_from_i64`→`_pow`→
    `_to_text` yields a `Text` handle (kind inference resolves `Pow(Int,Int)→Text` when `linked`, so
    `Show` prints it through the ordinary `print_text` path — no new `Kind`). The module imports
    `env.__linear_memory` (the linker's one shared memory, runtime-owned allocator; emitter-side heap is
    refused — two allocators would corrupt it) and the three bigint functions by undefined symbol.
    `linked` threads through `kind::infer`/`Env` + `plan_main`/`plan_function` + `Ctx`; `linked == false`
    is byte-identical (the whole standalone corpus is untouched). Test
    `link::tests::linked_program_computes_overflowing_power_as_bigint_matching_the_vm`: `Show 2 to the
    power of 200` → `assemble_program_linked` → `module_to_relocatable` → `rust-lld` vs the base runtime
    → runs, prints the exact 61-digit 2²⁰⁰ == `vm_outcome` (the WASM==VM lock, now through the BigInt
    linker). 127/127 green (vm::wasm lib + wasm_aot_args/lock/unit).
  - **S4-next-2 — the full BigInt NUMERIC TOWER (DONE, green).** A distinct `Kind::BigInt` (an i32 handle
    to the runtime's `Box<BigInt>`, rendered to a decimal `Text` only at `Show`) lets a chain keep
    computing on real BigInts: `Pow(Int,Int)` linked → `Kind::BigInt`; `* + - / %` with a BigInt operand →
    `logos_rt_bigint_{mul,add,sub,div,mod}` (`div`/`mod` = the VM's own `div_rem().0`/`.1`; an `Int`
    operand promoted via `from_i64`); `Show(BigInt)` is the two-step `to_text`→`print_text`. **General
    Int-overflow→BigInt** via a sound demand analysis (`kind::bigint_demanded_regs`, reader footprint from
    the exhaustive `regsplit::op_def_uses`): an integer op whose result is only `Show`n or fed into more
    BigInt arithmetic is promoted, so `Show 99999999999 * 99999999999` and `Show i64::MAX + 1` compute the
    exact value instead of trapping (matching the VM's promote-on-overflow), while a loop counter read by a
    compare stays fast i64 (a reg any i64 op reads is never promoted → never a miscompile). Test covers
    `(2^100)*(3^50)`, mixed `(2^100)*3`, add/sub incl. the NEGATIVE `(2^50)-(3^50)`, div/mod `(2^200) div/mod
    7`, and pure-Int overflow (simple + nested), each `== BigInt` and `== VM`. All gated on `linked`, so the
    standalone corpus is byte-identical (127/127, five regression runs).
  - **S4-next-3 — the public `compile_to_wasm_linked` entry (DONE, green).**
    `crate::compile::compile_to_wasm_linked(source) -> Result<Vec<u8>, ParseError>` (gated
    `wasm-jit` + `not(wasm32)`) takes Logos SOURCE straight to a linked `.wasm` in ONE call:
    parse → `assemble_program_linked` → `module_to_relocatable` → `link_relocatable_bigint`. The base
    `BigInt` runtime object + rlibs are built once and cached in a thread-local, so the amortized
    per-program cost is just the `rust-lld` link. `BIGINT_RUNTIME_SRC` + `build_bigint_runtime` were
    promoted out of `#[cfg(test)]` (the rest of the link infra was already non-test). Test
    `public_compile_to_wasm_linked_entry_runs_a_bigint_program` runs `Show (2^128)*3` +
    `Show 99999999999*99999999999` through the public fn == the VM. Web wasm32 build re-verified 0
    errors; standalone regression 128/128. TOOLCHAIN-DEPENDENT (needs cargo + a wasm32 `base` build) —
    a dev/test-time API, not a shipped-binary path.
  - **S4-next-4 — CLI + FUZZ lock + MIXED heap+BigInt (DONE, green).** `largo build/run --emit wasm`
    (self-contained) and `--emit wasm-linked` (BigInt) are wired + TDD-locked over the real binary + node
    (`apps/logicaffeine_cli/tests/emit_wasm.rs`, 6 tests). A deterministic FUZZ lock
    (`fuzz_linked_bigint_arithmetic_is_byte_identical_to_the_vm`) runs 40 random nested arithmetic
    programs through the linker, each == VM — the pit of success. And MIXED heap+BigInt programs now link
    + run == VM (`Show "x = " + (2^200)`): the emitter's bump allocator is seeded from a runtime-owned
    SLAB (`logos_rt_alloc` at a `main` prologue) so it coexists with `dlmalloc`, and concat stringifies a
    BigInt operand via `to_text` (op_kind_effect gives a Text operand a veto over the BigInt-add path).
    This closes what was mis-named the "data-relocation gap" — the emitter has NO data segments (Text is
    built at runtime) and BigInt literals aren't expressible, so the real gap was allocator coexistence.
    130/130 standalone regression, web 0 errors, CLI 6/6.
  - **S4-next-5 — linked `Repeat` loops + CLOSURES (DONE, green).** `Repeat` works: the iterator stack
    `__iter_sp` gets its OWN runtime slab at the prologue (grows down from the slab top). A demand guard
    keeps an Int-initialized loop accumulator a sound i64 (a candidate must have every writer be arith).
    CLOSURES work via a DIRECT CALL — `lower_call_value` already statically resolves the callee, so linker
    mode emits `call fn_base+func` instead of `call_indirect` and skips the table/element sections
    entirely (no reloc reverse-engineering needed; the reloc transform accepts the module as-is). Tests
    `linked_repeat_loops_match_the_vm` (plain loop, per-iteration BigInt, accumulator) +
    `linked_closures_match_the_vm` (non-capturing + capturing, both returning BigInt). 132/132 standalone
    regression byte-identical, web 0 errors.
  - **S4-next-6 — the three "niche" cases (DONE, green).** (1) **Move-chains**: `Let x be a*b. Show x.`
    (compiled `Mul T; Move x=T; Show x`) was trapping because the demand analysis didn't cross `Move`.
    Reformulated `bigint_demanded_regs` as a "bigint-safe" greatest-fixpoint over every register — safe iff
    every reader is a `Show`, an arith op with a safe result, or a `Move` with a safe destination — so the
    product is promoted and `Move` propagates the kind. (2) **Heap > 4 MiB → UNBOUNDED**: `emit_alloc` now
    takes `ctx` and, in linker mode, draws each block straight from `logos_rt_alloc` (dlmalloc growing
    memory on demand) instead of a fixed bump slab — linked heap is now unbounded (better than the
    self-contained 4 MiB). (3) **Dynamic closures** are a SHARED AOT limitation, not a linked gap:
    `lower_call_value` refuses an unknown-origin callee *before* the `linked` branch (the `call_indirect`
    type index needs a static callee), so the self-contained path refuses them too. Tests
    `linked_let_bindings_match_the_vm` + `linked_large_heap_beyond_the_slab_matches_the_vm`; 9 linked tests,
    134/134 standalone byte-identical, web 0 errors. The linked BigInt tier is now exact-to-VM on everything
    the self-contained AOT supports, with an unbounded heap; every remaining refusal is shared with the
    self-contained tier (never a linked-vs-VM miscompile).
- **S5+ — real collections / transports.** `logicaffeine_data` collections + `logicaffeine_runtime`
  scheduler + relay over the ABI; `retain`/`release` at the liveness/affine points the Rust-emit path
  already computes (THE soundness obligation, plan risk #1).

## Invariants

- Sound or refuse: a linked module must be byte-faithful to the VM/tree-walker, or the emitter
  declines (never miscompiles). The corpus ratchet enforces WASM==VM==tw.
- Determinism: pin lld flags; the tier cache keys on source, so the linked bytes must be reproducible.
