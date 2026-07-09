# ASM_BACKEND.md — Native x86-64 Backend

The campaign source of truth for compiling Logos straight to x86-64 machine code.
This is the design spec, the ABI contract, the op-lowering catalog, the
optimization roadmap, and the append-only ledger. Implementation is driven from
here; when this disagrees with the code, one of them is a bug.

---

## 1. Thesis & reframe

**We already JIT to x86.** `logicaffeine_forge` is a real x86-64 machine-code
JIT: copy-and-patch stencils (`int_stencils.rs`, extracted at build time) glued
at runtime by `buffer.rs`, **plus** a register-allocating whole-function emitter
(`regalloc.rs` over the `x64asm.rs` byte encoder). It is tiered: cold code runs
on the bytecode VM, hot regions/functions tier up to native via
`logicaffeine_jit::ForgeTier`, with precise deopt back to the VM on a failed
speculation or a checked-op side-exit. The "JIT to ASM" question is **answered
and shipping**.

**We do not AOT to x86.** The ahead-of-time paths today go through *source text*:
`compile_to_rust` (→ rustc) and `compile_to_c` (→ gcc). The only backend that
emits machine-level code *directly* — with no external compiler — is
`compile_to_wasm` (`vm/wasm/`), which hand-encodes a `.wasm` module from
bytecode under an exhaustive op-coverage lock (`tests/wasm_aot_lock.rs`). There
is **no `compile_to_native`**.

**This campaign builds it.** Take the bytecode the VM already runs and emit
**optimized x86-64 machine code directly** — first executed in-process via an
mmap'd page, then persisted as a standalone ELF — with no rustc and no gcc in the
path. The objective is not to serve the JIT. It is an experiment in producing the
**fastest code on earth**: becoming a genuine optimizing native *backend* (real
instruction selection, register allocation, scheduling, machine peepholes) over a
program the AST optimizer has already heavily transformed.

**Why this is safe to push hard.** The **triple lock** — tree-walker (the
reference oracle) == bytecode VM == native — held byte-identical over a growing
corpus plus seeded fuzzing. The tree-walker and VM already prove byte-identical to
each other through the shared semantics kernel (`crate::semantics`); the native
backend is held to the same bar. Tests are the IP and the scaffold: we can make
the codegen absurdly aggressive because any divergence is a hard RED.

### What exists vs. what's new

| Capability | Status | Where |
|---|---|---|
| JIT Logos → x86 (hot regions, tiered, deopt) | **DONE** | `logicaffeine_forge`, `logicaffeine_jit` |
| AOT Logos → Rust source → rustc | DONE | `compile.rs::compile_to_rust`, `codegen/` |
| AOT Logos → C source → gcc | DONE | `compile.rs::compile_to_c`, `codegen_c/` |
| AOT Logos → `.wasm` (direct, no toolchain) | DONE | `compile.rs::compile_to_wasm`, `vm/wasm/` |
| **AOT Logos → x86 machine code (direct)** | **NEW** | `forge/src/native/`, `compile_to_native[_elf]` |

The native backend is the **x86 analog of the WASM AOT backend**: same input
(`CompiledProgram` bytecode, already midend-optimized), same coverage discipline
(exhaustive `op_support` catalog + behavioral biconditional + monotone ratchet),
emitting x86 instead of wasm, tuned for maximum perf rather than portability.

### Stance decisions (locked)

- **Deliverable:** in-process `compile_to_native` **first**, standalone
  `compile_to_native_elf` **second**.
- **Runtime model:** *hybrid*. Reuse the existing Rust runtime through the JIT's
  `logos_rt_*` SysV seam for the **boxed/cold** ops (Show formatting, Map, Text,
  BigInt, struct/enum, CRDT, concurrency, networking) — the whole language works
  without porting it. Build a genuine optimizing native path for the **hot
  scalar/float/array kernels** — that is where "fastest on earth" is won.
- **First milestone scope:** scalar + array kernels (WASM AOT P0–P2) to a green
  differential lock fast, then chase `gcc -O3` on the benchmark corpus.
- **Home:** new modules under `crates/logicaffeine_forge/src/native/` (x86-only,
  `#[cfg(target_arch = "x86_64")]`, like the rest of forge) + entry points in
  `compile.rs` + the lock test in `logicaffeine_compile/tests/`. **No new crate**
  (repo cleanliness is a deliberate value).

---

## 2. Backend pipeline

The AST-level **midend is already done.** `optimize/optimize_program` runs the
full pass pipeline (PE/CTFE fixpoint, GVN, LICM, DCE, unroll, scalarize,
affine-scalarize, deforest, e-graph saturation + Oracle facts, loop-split,
inline-recursive, …) *before* bytecode is emitted. So by the time the native
backend sees a `CompiledProgram`, the hard machine-independent work is done. The
native backend is a **classic optimizing backend**: its job is world-class
instruction selection + register allocation + scheduling + encoding.

```
CompiledProgram (bytecode)               ← already midend-optimized (optimize/)
   │   vm/instruction.rs : Op, CompiledProgram, CompiledFunction
   │
   │  (1) static kind inference  →  per-register Int/Float/Bool/Text/Seq*/Struct…
   │      SHARE vm/wasm/kind.rs (lift to a backend-agnostic module)
   │  (2) build CFG + lift to SSA
   ▼
Native IR (NIR)  —  SSA, basic blocks, typed virtual regs, runtime-call nodes
   │  forge/src/native/nir.rs
   │  backend passes: copy-prop / GVN cleanup, addressing-mode forming,
   │  bounds-check elision from OracleFacts, boxed-op → logos_rt_* call nodes
   ▼
Instruction selection  —  tile NIR → x86 MachineInstr (still virtual regs)
   │  forge/src/native/isel.rs
   │  LEA addr arithmetic, fold load→ALU, fused cmp+jcc, cmov, test, magic-div
   ▼
Register allocation  —  linear-scan + live-range splitting + coalescing
   │  forge/src/native/ra.rs   (Phase 1 reuses forge/src/regalloc.rs verbatim)
   │  GP + XMM classes, SysV save/restore, spill slots, move elimination
   ▼
Scheduling + machine peephole  —  latency/port-aware reorder, strength reduce
   ▼
Encoding  —  forge/src/x64asm.rs (grown to full shapes)  →  bytes + relocations
   │
   ├─ in-process:  JitPage (mmap RX) → call            [compile_to_native]
   └─ standalone:  ELF object/exe + relocate logos_rt_* and Logos→Logos calls
                   forge/src/native/elf.rs               [compile_to_native_elf]
```

### Reuse map (file : symbol — lift-and-shift, reinvent nothing)

| Need | Reuse | Notes |
|---|---|---|
| x86 byte encoder | `forge/src/x64asm.rs` : `Asm`, `Reg`, `Xmm`, `Cond`, `LabelId` | Int + SSE2-scalar + packed-double already encoded; grow it (§6.1). |
| Whole-function codegen (Phase 1) | `forge/src/regalloc.rs` : `compile_function_regalloc`, `compile_region_regalloc[_precise]` | Bit-identical to bytecode already; reuse verbatim for first green. |
| Executable memory | `forge/src/lib.rs` : `JitPage::new`, `::with_layout`, `::as_ptr`, `::patch_word` | mmap RW→RX, W^X sealed; `with_layout` resolves addresses pre-seal. |
| Relocation / patching | `forge/src/{buffer,patch,stencil_model}.rs` : `HoleValue` (`Cont`/`Const`), `RelocKind` (`Rel32`/`Abs64`/`GotRel32`), patchers | Feeds the ELF relocation table. |
| Runtime seam (breadth) | `logicaffeine_jit/src/lib.rs` : `logos_rt_*` | SysV bridge native→Rust runtime (§4.4). The whole-language escape hatch. |
| Bytecode → low IR | `logicaffeine_jit` : `adapt_function`, `MicroOp` | Existing kind-resolved lowering; front of NIR. |
| Static kind inference | `vm/wasm/kind.rs` | LIFT to shared module; both AOT backends consume it. |
| Coverage lock template | `tests/wasm_aot_lock.rs` : `op_support`, `op_name`, `all_op_variants`, ratchet | Template for `native_aot_lock.rs` (§5). |
| Benchmark methodology | `benchmarks/run.sh`, fair-vs-C harness | "Fastest on earth" measured here (§5.4). |

---

## 3. Input IR — `CompiledProgram`

Defined in `vm/instruction.rs`. The native backend consumes exactly what the VM
dispatches and `compile_to_wasm` consumes — no new front-end IR layer.

```
CompiledProgram {
    constants: Vec<Constant>,          // Int / Float / Text / … pool
    code: Vec<Op>,                     // Main's linear bytecode (entry pc 0) + Halt
    register_count: usize,             // Main frame width
    functions: Vec<CompiledFunction>,  // each: code, register_count, entry, params
    fn_index: HashMap<Symbol, FuncIdx>,
    globals: Vec<String>,              // promoted top-level Main bindings
    named_regs, loop_locals, …
}
```

`Op` has ~100 variants (the exhaustive catalog lives in `op_name`/`all_op_variants`
in `wasm_aot_lock.rs`). The native backend reaches the *same* program text that
the VM and tree-walker run, so the differential is a true two-engines-one-program
comparison.

---

## 4. ABI

### 4.1 Native function entry ABI (the forge contract — reused)

A compiled Logos function is `unsafe extern "C"` and follows SysV AMD64:

```
fn(base: *mut i64,   // rdi — frame base (i64 slot array)
   sp:   *mut i64,   // rsi — operand stack pointer (grow-up)
   r0..r3: i64,      // rdx, rcx, r8, r9 — 4 GP register-pinned hot scalars
   f0..f5: f64)      // xmm0..xmm5 — 6 XMM register-pinned hot floats
   -> i64             // rax
```

Register discipline (`x64asm.rs` / `regalloc.rs`):
- `r15` = frame base pointer inside the body.
- Callee-saved GP for resident slots: `rbx`, `r12`, `r13`, `r14` (saved in
  prologue, restored in epilogue). Caller-saved scratch: `rax`, `rdx`, `rcx`
  (the div/shift/imul scratch), plus `r8`–`r11`, `rdi`, `rsi` for short-lived
  temporaries.
- XMM: `xmm0`–`xmm13` allocatable for f64 slots (all caller-saved under SysV, no
  save needed); `xmm14`/`xmm15` reserved scratch for float arithmetic.
- `4 GP pins is the SysV arg limit`; `xmm0–7` are free, so XMM float threading is
  the keystone lever for the float cluster (nbody etc.).

### 4.2 Value representation at runtime

Two switchable VM reps exist (`vm/value.rs`): the default 16-byte fat
`Value(RuntimeValue)` and the 8-byte NaN-boxed `Value(Narrow)` (feature
`narrow-value`, proven lossless by `narrow_corpus_differential.rs`). The native
backend does **not** carry a tagged cell on the hot path; it specializes by the
inferred kind:

| Kind | Native representation |
|---|---|
| Int (proven scalar) | raw `i64` in a GP reg / frame slot. Overflow → `jo` side-exit / BigInt promotion (deopt in JIT mode; runtime-call promote in AOT). |
| Float | raw `f64` bits in an XMM reg / frame slot. |
| Bool | `i64` 0/1. |
| Seq (Int/Float/Bool/i32) | handle **triple** `[vec_handle, data_ptr, len]` in 3 consecutive slots; element load/store is inline native, growth via `logos_rt_push_*`. |
| Text / Map / Set / Struct / Enum / CRDT / Date / Moment / closure | boxed handle (`i64`); all ops via `logos_rt_*` runtime calls. |

The boundary between "inline native" and "runtime call" is the boundary between
the optimizing backend and the breadth seam. It is exactly the line `op_support`
draws (§5.1).

### 4.3 Internal Logos→Logos fastcall (perf lever, Phase 3)

We own both sides of a Logos call, so beyond the SysV entry ABI we will define an
internal fastcall: pinned-register arguments (extend the `r0..r3`/`f0..f5` pin
scheme), no frame round-trip for leaf calls, and AOT tail-call (already a language
semantic via `crate::tail_call`, three-tier-consistent on tree-walker/VM/AOT).

### 4.4 Runtime-call seam (`logos_rt_*`) — the breadth escape hatch

`logicaffeine_jit/src/lib.rs` already exports the SysV bridge native code uses to
re-enter the Rust runtime. The AOT backend reuses it for every boxed/cold op.
Current surface (grows as coverage grows):

```
logos_rt_alloc_list_i64(...)           logos_rt_list_triple(...)
logos_rt_map_get_ii / _set_ii / _has_i
logos_rt_push_i64 / _i32 / _f64 / _bool
logos_rt_clear_i64 / _i32 / _f64 / _bool
logos_rt_str_append(...)               logos_rt_memmem(...)  logos_rt_memchr(...)
```

In-process (Phase 1): these are called by absolute address (same process). In the
ELF (Phase 2): they become relocations against the linked runtime staticlib (or
self-contained equivalents). New Supported ops add helpers here rather than
reimplementing Show/Map/Text/BigInt in assembly.

---

## 5. Op-lowering catalog & the correctness scaffold

### 5.1 `op_support` — the static, catalog-complete lock

`tests/native_aot_lock.rs` carries `fn op_support(&Op) -> Supported | Deferred(reason)`
as an **exhaustive match with no `_` arm** — the compiler refuses to build the
file until a newly added `Op` is classified. This is the "no feature silently
escapes" guarantee, the dual of the Futamura coverage guard, enforced by the type
system. It is the prose mirror of the real lowering in `forge/src/native/`.

Lowering disposition per op family (the target end state; phases grow `Deferred → ∅`):

| Op family | Disposition |
|---|---|
| `LoadConst`, `Move`, `Add/Sub/Mul`, `Div/Mod`, `DivPow2`, `MagicDivU`, `BitXor/Shl/Shr`, `Lt/Gt/LtEq/GtEq/Eq/NotEq`, `Not/AndEager/OrEager/JumpIfInt`, `Jump/JumpIfFalse/JumpIfTrue`, `GlobalGet/Set`, `Return/ReturnNothing/Halt` | **Native inline** (the hot path; mostly already in `regalloc.rs`). |
| `Call`, `CallValue`, `MakeClosure` | Native call (Logos fastcall); closure object via seam. |
| `CallBuiltin` (Sqrt/Floor/Ceil/Round/Abs/Min/Max/Pow) | Native inline (SSE2) where bit-exact; `pow_ff`/`pow_fi` via seam for Float results. |
| `NewEmptyList`, `ListPush`, `Index`, `SetIndex`, `Length`, `NewList`, `NewRange`, `Contains`, `SliceOp`, `SeqConcat`, `IterPrepare/Next/Pop` | **Native inline element access**; allocation/growth via `logos_rt_*` (the P2 heap model). |
| `Concat`, `FormatValue` | Runtime seam (byte copy + `fmt_*_into`). |
| `NewStruct`, `StructInsert`, `GetField`, `NewInductive`, `TestArm`, `BindArm` | Seam (heap value model), inline tag compares where cheap. |
| `NewEmptySet/Map`, `SetAdd`, `RemoveFrom`, `UnionOp`, `IntersectOp`, `DeepClone`, `NewTuple` | Seam. |
| `LoadToday/LoadNow` | Seam (`today`/`now`, fixed-clock honored). |
| `CrdtBump/Merge/NewCrdt/Append/Resolve` | Seam (CRDT). |
| `Sleep`, `Chan*`, `Spawn*`, `Task*`, `Select*` | **Deferred** — the pure-Rust M:N scheduler is linkable; lands via the seam + async host. *Deferred, never excluded.* |
| `Net*` | **Deferred** — per-capability transport seam. *Deferred, never excluded.* |
| `CheckPolicy`, `FailWith`, `Args` | Seam / trap-on-fail / argv host. |

### 5.2 Behavioral lock — the biconditional

`native_equals_vm_and_treewalker_over_the_corpus`: for every corpus program,
tree-walker == VM (the base equivalence through `crate::semantics`), and native is
held to a **biconditional** — it compiles **iff** every op it uses is `Supported`,
and when it compiles its output equals the tree-walker **byte-for-byte**. The
backend can therefore neither (a) miscompile, (b) lower a `Deferred` op (desyncs
`op_support` from reality — a RED), nor (c) reject an all-`Supported` program (a
coverage gap to fix in the backend, never a thing to quietly defer — also a RED).
Programs that use a `Deferred` op simply fall back to the VM/Rust path: **no
regression** to anything that runs today.

> ⚠️ You do not get to weaken this file to make a RED pass. A RED means the native
> backend dropped/miscompiled a feature, or a new op slipped the catalog. The fix
> is in `forge/src/native/`, never by relaxing an assertion, moving an op to
> `Deferred`, or adding a wildcard arm. (Same contract as `wasm_aot_lock.rs`.)

### 5.3 Monotone ratchet + fuzz

- `supported_op_count_never_regresses` — coverage is strictly monotone; it can
  only grow until the whole instruction set is native or deliberately Deferred.
- Reuse the forge differential corpus gate (regalloc bit-identical to bytecode)
  for the whole-function path.
- Seeded arithmetic fuzz (à la `wasm_jit_differential.rs`): native vs VM over
  randomized int/float programs, *including* overflow→BigInt promotion behavior
  (the one place where native checked-arith semantics must match the exact VM).

### 5.4 Perf gate

Every Phase-3 lever must move the same-algorithm geomean against `gcc -O3
-march=native -flto` on the benchmark corpus (`benchmarks/run.sh`), with results
logged to `logs/optimization/` (never `/tmp`). Methodology note: state perf as
two numbers — the algorithm-collapse-inflated headline and the same-algorithm
geomean (currently ~1.07× for the JIT vs C; the AOT backend, with *unbounded*
compile time, should push past it). Recalibrate compute-dominated benches; the
~1.05× whole-suite geomean is partly a startup-floor artifact.

---

## 6. Optimization roadmap — "the LLVM things"

The midend is reused; these are the *backend* levers. Each is gated by the lock
staying green and a measured benchmark delta. Because AOT has no compile-time
budget (unlike the JIT), we can run global/iterative passes the JIT cannot.

### 6.1 Encoder completion (`x64asm.rs`)
Already present: `mov` (ri/rr/rm/mr, byte/dword variants), `add/sub/imul`,
`idiv`+`cqo`, unsigned `mul` (magic-div high product), `and/or/xor/not/neg`,
`shl/shr/sar` (imm + cl), `cmp/test`, `setcc`, `jcc/jmp` (rel32 late-bound),
`push/pop/ret`, indirect `call reg`, SSE2 scalar
(`movsd/addsd/subsd/mulsd/divsd/sqrtsd/ucomisd/cvtsi2sd/movq`), packed-double
(`movupd/addpd/subpd/mulpd/divpd/sqrtpd/cmppd/movmskpd/andpd/andnpd/orpd/xorpd`).
**Add:** direct `call rel32`, `lea` (the address-arithmetic workhorse), `cmov`
(branchless select), full `base+index*scale+disp` ModRM/SIB addressing,
rip-relative f64 constant loads (constant pool), `movsd`/`cvttsd2si` rounding
forms as needed, later AVX/AVX2 (VEX prefix).

### 6.2 Instruction selection (`isel.rs`)
Tile NIR to x86: fold loads into ALU operands; `lea` for `i*scale+base+disp`
index math; fused `cmp`+`jcc` (already a stencil idiom); `cmov` for small
if/else selects to kill unpredictable branches; `test r,r` for zero checks;
sub-register byte/dword loads/stores for narrowed Seqs (`movzx`/`movsxd`);
strength-reduce `mul`→`lea`/`shl`, `div`→`MagicDivU` (already lowered).

### 6.3 Register allocation (`ra.rs`)
Linear-scan with live-range splitting + coalescing (move elimination), two
classes (GP/XMM), SysV save/restore + spill slots. Supersedes the current
per-slot global assignment (rank-by-refcount) once the SSA NIR exists. Keep the
existing loop-invariant array ptr/len hoist and scaled-index CSE as RA-aware
rewrites.

### 6.4 Scheduling & layout
Latency/port-aware reordering of independent chains; loop-head 16-byte alignment;
cold-block sinking and likely-path block placement for I-cache; software
pipelining for the tightest hot loops.

### 6.5 Vectorization (`vectorize.rs`)
Extend the existing 2-wide packed-double recognition to AVX/AVX2 and integer SIMD
where Oracle-proven safe. Bit-exact lanes only: packed ops round per-lane
identically to scalar; **no FMA fusion** (two roundings → one is not bit-exact and
would break the lock).

### 6.6 Calling convention & memory
Internal Logos fastcall (§4.3), pinned-register args, AOT tail-call. Load-store
optimization and alias disambiguation reusing the existing `OracleFacts`/effects
(the same facts that feed BCE and the JIT). Bounds-check elision driven by
`index_provably_in_bounds` → `assert_unchecked`/raw access.

---

## 7. ELF emission (Phase 2)

`forge/src/native/elf.rs`. Default path: emit a relocatable object (`.text` =
concatenated function code, symbol table for each Logos function + the `logos_rt_*`
externs, relocation entries from the forge `RelocKind` set) plus a tiny
`_start`/`main` shim, and link the runtime staticlib via the **system linker**
(`cc`/`ld`) — no compiler in the path. The purist north star (recorded, not yet
required): a self-contained static-ELF writer with an embedded minimal native
runtime (bump heap, `Show` via the `write` syscall) so even the linker drops out —
the true x86 analog of `compile_to_wasm`'s "no rustc/cargo/wasm-bindgen". The lock
for this phase **runs the emitted binary** and asserts stdout/exit == tree-walker.

---

## 8. Files

**Create — `crates/logicaffeine_forge/src/native/`**
- `mod.rs` — `compile_program_native(&CompiledProgram) -> NativeImage` (entry).
- `lower.rs` — `Op` → backend IR; boxed-op → `logos_rt_*` call lowering.
- `nir.rs` — SSA Native IR + CFG (Phase 3).
- `isel.rs` — instruction selection / tiling (Phase 3).
- `ra.rs` — register allocator (Phase 3; Phase 1 reuses `regalloc.rs`).
- `elf.rs` — ELF object/executable emitter + relocations (Phase 2).

**Modify**
- `forge/src/x64asm.rs` — encoder completion (§6.1).
- `forge/src/lib.rs` — export native entry points; `JitImage`/`NativeImage`.
- `compile.rs` — `compile_to_native`, `compile_to_native_elf` (beside `compile_to_wasm`).
- `vm/wasm/kind.rs` — lift kind inference into a shared module.

**Create — tests**
- `logicaffeine_compile/tests/native_aot_lock.rs` — the triple lock.

---

## 9. Milestones & ledger

Each item is TDD: the lock/corpus row is the RED; the backend grows to GREEN.
Start and end every phase from all-green (`./scripts/run-all-tests-fast.sh`).

### Phase 0 — design doc
- [x] `ASM_BACKEND.md` authored (this file).

### Phase 1 — in-process scalar/array AOT to first green
- [ ] `native_aot_lock.rs`: `op_support` + `op_name` + `all_op_variants` mirrored from the wasm lock; corpus seeded with P0 scalar programs (RED).
- [ ] `compile_to_native(src) -> NativeImage`: bytecode → `compile_function_regalloc` per function, inter-function calls wired, run via `JitPage`.
- [ ] P0 green: arithmetic, comparisons, control flow, calls, globals, `Show`.
- [ ] Lift `vm/wasm/kind.rs` to a shared kind-inference module.
- [ ] P1 green: `DivPow2`/`MagicDivU`, numeric builtins, temporal.
- [ ] P2 green: Seq/array heap ops (`NewEmptyList`, `ListPush`, `Index`, `SetIndex`, `Length`, `NewList`, `SliceOp`, `SeqConcat`) via the seam; `Text`/`Concat`.
- [ ] Benchmark kernels (fib, tri-loop, `array_build_and_sum`, quicksort, nbody) run via `compile_to_native`; first geomean-vs-C datapoint logged.

### Phase 2 — standalone ELF
- [ ] `forge/src/native/elf.rs`: object emitter + relocations + `_start`/`main` shim.
- [ ] `compile_to_native_elf(src) -> Vec<u8>`; link runtime staticlib (linker-only, zero compiler).
- [ ] Lock variant that **runs the emitted binary**; stdout/exit == tree-walker.

### Phase 3+ — the optimizing backend (open-ended perf)
- [ ] SSA Native IR + CFG (`nir.rs`).
- [ ] Instruction selection (`isel.rs`): LEA / load-fold / cmov / fused cmp-branch.
- [ ] Register allocator (`ra.rs`): linear-scan + splitting + coalescing.
- [ ] Scheduling + machine peephole; loop-head alignment; branch layout.
- [ ] Vectorization to AVX/AVX2 + integer SIMD (bit-exact, no FMA).
- [ ] Internal Logos fastcall + AOT tail-call.
- [ ] Grow `Deferred → ∅`: structs/enums/CRDT, then concurrency & networking.
- [ ] Same-algorithm geomean ≥ `gcc -O3` on the benchmark corpus.

### Progress log (append-only)
- `2026-06-29` — Campaign opened. Reframe established (JIT-to-x86 already ships in
  `logicaffeine_forge`; this is the missing native *AOT* target). Stance locked:
  in-process first then ELF; hybrid runtime (reuse `logos_rt_*` seam for breadth,
  optimizing native path for hot kernels); scalar+array P0–P2 to first green.
  `ASM_BACKEND.md` written. No backend code yet (Phase 0 gate: doc before code).
