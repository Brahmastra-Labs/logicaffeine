//! ★ THE RISC-V AOT PIT OF SUCCESS ★ — the single authoritative lock for the LOGOS → RISC-V tier,
//! mirroring `wasm_aot_lock.rs`. Two complementary halves over ONE corpus:
//!
//! 1. **Static, catalog-complete ratchet.** [`op_support`] is an EXHAUSTIVE match on `Op` (no `_`
//!    arm), so a new VM instruction fails to compile this file until it is classified. A monotone
//!    `SUPPORTED_FLOOR` (raise per phase, never lower) plus a shrink-only `KNOWN_GAPS` set (the live
//!    `Deferred` census, checked for EXACT equality) make it impossible to silently regress coverage
//!    or grow the gap set. Every `Supported` op must appear in a compiling corpus program
//!    (`every_supported_instruction_is_exercised`), so "Supported" can never be vacuous.
//!
//! 2. **Behavioural biconditional.** For each corpus program, the two oracles agree
//!    (`tw_outcome == vm_outcome`); a fully-supported program COMPILES and runs byte-for-byte identical
//!    to the tree-walker under `qemu-riscv64`; an erroring program exits nonzero exactly where the VM
//!    errors (`ERROR_CORPUS`); and a program using a not-yet-lowered op is REFUSED, never miscompiled
//!    (`REFUSE_CORPUS`).
//!
//! 3. **Ownership.** A change to a collection reaches only the holder it was made through
//!    (`OWNERSHIP_CORPUS`, against the tree-walker), and every op the tier lowers as a mutation of a
//!    register's own collection runs a long loop within a capped address space (`LINEAR_MEMORY`, one row
//!    per such op, checked for exact coverage), so a mutation that copies the whole collection each time
//!    fails here instead of growing a benchmark to gigabytes.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If a lock goes RED, fix the emitter (`codegen_riscv`), never
//! the assertion. The behavioural half is gated on the RISC-V toolchain (`riscv64-linux-gnu-gcc` +
//! `qemu-riscv64`): an absent tool is a recorded skip, and a failure where `LOGOS_REQUIRE_TOOLS` names it; the
//! static half always runs.

mod common;
#[path = "support/riscv_runner.rs"]
mod riscv_runner;
use riscv_runner::RvRun;

use std::collections::BTreeSet;

use logicaffeine_compile::analysis::PolicyRegistry;
use logicaffeine_compile::codegen_riscv::{codegen_program_riscv, mutates_own_collection};
use logicaffeine_compile::compile::{
    compile_to_riscv, tw_outcome, tw_outcome_with_args, vm_outcome, vm_outcome_net,
    vm_outcome_with_args,
};
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::Compiler;
use logicaffeine_compile::vm::{
    BoundaryType, ChanElem, CompiledFunction, CompiledProgram, Constant, Op, StructTypeDef, Vm,
};
use logicaffeine_compile::{interpret_for_ui_sync, run_vm_concurrent, Interner};

// ============================================================
//  Static classification — the catalog-complete ratchet
// ============================================================

/// How the RISC-V AOT backend handles an op. `Supported` = lowered + exercised end-to-end ≡ VM;
/// `Unreachable` = a dead op no source syntax emits into an AOT program (so it cannot be exercised);
/// `Deferred` = genuine remaining WORK (a later phase lowers it). The gaps→∅ campaign shrinks the
/// `Deferred` census; concurrency/networking remain a documented permanent gap for a straight-line AOT.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Support {
    Supported,
    Unreachable(&'static str),
    Deferred(&'static str),
}

/// ★ THE SINGLE SOURCE OF TRUTH ★ for which VM ops the RISC-V backend lowers. EXHAUSTIVE on `Op` (no
/// `_` arm): a new instruction cannot escape classification. Keep in lockstep with `codegen_riscv::emit`
/// — the behavioural biconditional proves they agree.
fn op_support(op: &Op) -> Support {
    use Support::{Supported as S, Unreachable as U};
    match op {
        // ── Phase 1: scalar-integer core (lowered + exercised ≡ VM) ─────────────────────────
        Op::LoadConst { .. } => S, // Int/Bool constants (Float/Text refused at the value level → Phase 2/3)
        Op::Move { .. } => S,
        Op::Add { .. } | Op::AddAssign { .. } | Op::Sub { .. } | Op::Mul { .. } => S,
        Op::Div { .. } | Op::Mod { .. } | Op::FloorDiv { .. } | Op::DivPow2 { .. } => S,
        // Exact division in a `Rational` context (`Let x: Rational be 7 / 2`) → an exact [num,den];
        // arithmetic/compare with a Rational promotes an Int to `n/1`. A Float operand → float division.
        Op::ExactDiv { .. } => S,
        Op::BitXor { .. } | Op::BitAnd { .. } | Op::BitOr { .. } => S, // Int/Bool operands (Set algebra → Phase 3)
        Op::Shl { .. } | Op::Shr { .. } => S, // masked wrapping shifts (RV64 sll/sra) — no bigint promotion
        Op::Pow { .. } => S, // Int squaring (overflow = bigint gap) / Float via libm pow
        Op::MagicDivU { .. } => S, // Granlund–Montgomery magic reciprocal (mirrors the VM's magic_eval)
        Op::CallBuiltin { .. } => S, // scalar numerics (abs/sqrt/floor/ceil/round/min/max); others refuse
        Op::EnsureOwned { .. } => S, // no-op (copy-on-write is subsumed by copy-then-mutate)
        // A native-region-JIT hint the AOT compile path never emits; lowered as a no-op if it ever appears.
        Op::RegionBoundsGuard { .. } => S, // native-region-JIT bounds hint → a no-op in the AOT (exercised via bytecode)
        Op::Lt { .. } | Op::Gt { .. } | Op::LtEq { .. } | Op::GtEq { .. } => S,
        Op::Eq { .. } | Op::NotEq { .. } | Op::Not { .. } => S,
        Op::ApproxEq { .. } => S, // `is approximately` — f64 isclose (Phase 2)
        Op::Jump { .. } | Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. } => S,
        Op::Call { .. } | Op::Return { .. } | Op::ReturnNothing | Op::Halt => S,
        // Closures: `MakeClosure` builds a heap `[body, captures…, present…]` object; `CallValue` is an
        // indirect call (args + the closure pointer as a hidden arg, dispatched by body index). Inline
        // closure bodies are emitted as their own regions (main is split around the embedded body).
        Op::MakeClosure { .. } | Op::CallValue { .. } => S,
        Op::GlobalGet { .. } | Op::GlobalSet { .. } => S,
        // `today` / `now` — the injected fixed clock (nondeterministic wall clock pinned for the
        // differential); `Show` renders `YYYY-MM-DD` / `YYYY-MM-DD HH:MM`, byte-for-byte like the VM.
        Op::LoadToday { .. } | Op::LoadNow { .. } => S,
        // Counter CRDTs (per-replica map, idempotent): `NewCrdt` seeds the map, `CrdtBump` bumps this
        // replica's entry, `CrdtMerge` takes the per-replica MAX (double-merge is a no-op, matching the
        // VM). `CrdtAppend` appends in place to a `Seq` field (RGA behind a shared handle).
        Op::NewCrdt { .. }
        | Op::CrdtBump { .. }
        | Op::CrdtMerge { .. }
        | Op::CrdtAppend { .. }
        | Op::CrdtResolve { .. } => S,
        Op::Show { .. } => S, // Int + Bool + Float + Text (non-scalar Show refused → Phase 3b+)
        Op::Concat { .. } => S, // Text concat + interpolation segments (stringifies both operands)
        // Interpolation with a compile-time format spec (currency `$`, precision `.N`, alignment
        // `>N`/`<N`/`^N`, bare width `N`) or a debug prefix (`{name=}`) — mirrors `apply_format_spec`.
        Op::FormatValue { .. } => S,
        // Seq of Int: create / push (copy-on-write) / length / 1-based index / for-each iteration / Show.
        Op::NewEmptyList { .. }
        | Op::NewEmptyListWithCapacity { .. }
        | Op::NewEmptyListFixed { .. }
        | Op::ListPush { .. }
        | Op::Length { .. }
        | Op::Index { .. }
        | Op::IndexUnchecked { .. }
        | Op::IterPrepare { .. }
        | Op::IterNext { .. }
        | Op::IterPop => S,
        // List literals / ranges / concat / slice / membership / indexed-write (COW) / deep clone.
        Op::NewList { .. }
        | Op::NewRange { .. }
        | Op::SeqConcat { .. }
        | Op::SliceOp { .. }
        | Op::Contains { .. }
        | Op::DeepClone { .. }
        | Op::SetIndex { .. }
        | Op::SetIndexUnchecked { .. } => S,
        // Sets of Int/Text: create / add (dedup, COW) / remove / union / intersect / membership / Show.
        Op::NewEmptySet { .. }
        | Op::SetAdd { .. }
        | Op::RemoveFrom { .. }
        | Op::UnionOp { .. }
        | Op::IntersectOp { .. } => S,
        // Maps (key→value insert/get/Show via composite kinds) + tuple literals / homogeneous multi-return
        // (a `(Int, Int)` return rides the Seq layout, extracted via `item N`).
        Op::NewEmptyMap { .. } | Op::NewEmptyMapWithCapacity { .. } | Op::NewTuple { .. } => S,
        // Structs: create / field set (COW) / field get / Show `Name { a: .., b: .. }` (fields sorted).
        Op::NewStruct { .. } | Op::StructInsert { .. } | Op::GetField { .. } => S,
        // Enums (inductive): construct `[tag, payload…]`, Inspect arm tag-test, payload bind (Show(enum)
        // per-variant formatting is still deferred).
        Op::NewInductive { .. } | Op::TestArm { .. } | Op::BindArm { .. } => S,
        // `for (a, b) in seq-of-tuples` destructure + `Pop from xs into v` (value-semantics COW pop; a source
        // program never pops an empty list, `resolve_program` stops it first).
        Op::DestructureTuple { .. } | Op::ListPop { .. } => S,
        // "error WHEN EXECUTED" (non-exhaustive Inspect fallthrough, unbound Set) → exit nonzero.
        Op::FailWith { .. } => S,
        // `Check that <subject> is <predicate>` — the policy condition (from the `## Policy` registry) is
        // compiled inline against the subject's fields; a false result / undefined rule exits nonzero.
        Op::CheckPolicy { .. } => S,
        // `args()` → a Seq of Text: the full argv with argv[0] (the program name) at index 1, so `item 2`
        // is the first real argument. The differential runs the binary and the oracle with a matching
        // single-element argv, so `length of args()` is 1 on both. + struct-field push (`Push v to s's
        // items` — mutates the shared field list in place through every alias).
        Op::Args { .. } | Op::ListPushField { .. } => S,

        // ── Ops NO source emits into an AOT program, but with well-defined FORWARD semantics the VM
        // executes — LOWERED and proven ≡ VM via the hand-built bytecode differential (nothing is a mere
        // refusal). Reversible-computing primitives (forward direction), the BigInt lane (in-i64), the
        // narrow-VM half-width empty list, and the lattice JOIN (per-field MAX). ──
        Op::RevSwap { .. }
        | Op::RevXor { .. }
        | Op::RevNeg { .. }
        | Op::RevAdd { .. }
        | Op::RevSub { .. } => S,
        Op::NewEmptyListI32 { .. } => S,
        Op::BigIntWide { .. } => S,
        Op::CrdtJoin { .. } => S,
        // The OBSERVED-profile observation site — a no-op in the elided AOT profile the differential runs.
        Op::Obs { .. } => S,

        // ── Concurrency: a straight-line-embedded cooperative-coroutine runtime (ucontext) that is a
        // faithful port of `logicaffeine_runtime`'s deterministic FIFO scheduler. `main` runs as the first
        // task; channels rendezvous/queue; Spawn runs the child first. Proven byte-for-byte against the
        // CONCURRENT oracle pair (`interpret_for_ui_sync` + `run_vm_concurrent`) under qemu.
        Op::ChanNew { .. }
        | Op::ChanSend { .. }
        | Op::ChanRecv { .. }
        | Op::ChanTrySend { .. }
        | Op::ChanTryRecv { .. }
        | Op::Spawn { .. }
        | Op::SpawnHandle { .. } => S,
        // Virtual-time `Sleep` + `Await the first of` select (recv-ready / timeout arms) over the logical
        // clock + timer heap. (A >1-ready-recv-arm seed tie-break is a documented gap — corpus avoids it.)
        Op::Sleep { .. }
        | Op::SelectArmRecv { .. }
        | Op::SelectArmTimeout { .. }
        | Op::SelectWait { .. } => S,
        // `Stop handle` — abort a task: the scheduler skips it, deliveries/timers drop it (lazy teardown).
        Op::TaskAbort { .. } => S,
        // Scheduler ops no source emits (0 emit sites) — lowered + proven via hand-built bytecode:
        // `TaskAwait` blocks on a task's completion, `ChanClose` closes a channel.
        Op::TaskAwait { .. } | Op::ChanClose { .. } => S,

        // ── Networking — the OFFLINE loopback transport (the determinism-oracle `net_offline` path):
        // Connect/Listen are no-ops, a peer IS its address, Send/Stream queue into a per-address FIFO and
        // Await drains it (so `Send X to peer; Await from peer` round-trips), Sync is offline-identity.
        // Proven ≡ the offline oracle pair (`tw_outcome` + `vm_outcome_net`) under qemu.
        Op::NetConnect { .. }
        | Op::NetListen { .. }
        | Op::NetSend { .. }
        | Op::NetStream { .. }
        | Op::NetAwait { .. }
        | Op::NetMakePeer { .. }
        | Op::NetSync { .. } => S,

        // tcp.lg client sockets — real host networking, outside the qemu-user
        // single-node oracle. Refused loudly (never a silent Nothing).
        Op::Tcp { .. } => U("tcp.lg sockets — real host networking is outside the RISC-V tier"),
        // The `Serve` HTTP accept loop needs a real host listener, outside the
        // qemu-user single-node oracle. Refused loudly.
        Op::Serve { .. } => U("Serve HTTP — a real host accept loop is outside the RISC-V tier"),
    }
}

/// The count of `Supported` ops — a MONOTONE FLOOR. RAISE it as each phase lands; NEVER lower it.
const SUPPORTED_FLOOR: usize = 118;

/// The injected fixed clock (a deterministic instant): `today`/`now` are nondeterministic wall-clock
/// reads, so the differential harness pins BOTH the VM oracle (via `set_fixed_clock`) and the AOT binary
/// (via the `LOGOS_FIXED_*` env vars the runtime shim reads) to this same instant → 2024-10-04 12:34.
const FIXED_DAYS: i64 = 20000;
const FIXED_NANOS: i64 = 20000 * 86400 * 1_000_000_000 + 45_296 * 1_000_000_000;

/// The EXACT set of currently-`Deferred` op names — a shrink-only ledger. Moving an op from `Deferred`
/// to `Supported` requires removing it here AND raising `SUPPORTED_FLOOR` (both conscious edits); a new
/// deferral must be added here deliberately. The ratchet asserts this equals the live `Deferred` census.
const KNOWN_GAPS: &[&str] = &[];

// ============================================================
//  Corpus
// ============================================================

/// Fully-supported programs — each MUST compile and run byte-for-byte identical to the VM. Together they
/// exercise every `Supported` op (proven by `every_supported_instruction_is_exercised`).
const CORPUS: &[(&str, &str)] = &[
    ("show_int", "## Main\n    Show 7.\n"),
    ("multi_show", "## Main\n    Show 1.\n    Show 2.\n    Show 3.\n"),
    ("copy_move", "## Main\n    Let a be 5.\n    Let b be a.\n    Show b.\n"),
    ("sq_call", "## To sq (x: Int) -> Int:\n    Return x * x.\n## Main\n    Show sq(9).\n"),
    (
        "sumsq",
        "## To sq (x: Int) -> Int:\n    Return x * x.\n\
         ## To sumsq (a: Int) (b: Int) -> Int:\n    Return sq(a) + sq(b).\n\
         ## Main\n    Show sumsq(3, 4).\n",
    ),
    (
        "while_tri",
        "## To tri (n: Int) -> Int:\n    Let acc be 0.\n    Let i be n.\n    \
         While i is greater than 0:\n        Set acc to acc + i.\n        Set i to i - 1.\n    \
         Return acc.\n## Main\n    Show tri(100).\n",
    ),
    (
        "if_clamp",
        "## To clamp (x: Int) -> Int:\n    If x is greater than 5:\n        Return 5.\n    Return x.\n\
         ## Main\n    Show clamp(8).\n    Show clamp(3).\n",
    ),
    (
        "fib",
        "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    \
         Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(10).\n",
    ),
    (
        "divmod",
        "## To dm (a: Int) (b: Int) -> Int:\n    Let q be a / b.\n    Let r be a % b.\n    Return q + r.\n\
         ## Main\n    Show dm(17, 5).\n",
    ),
    (
        "floor_div",
        "## To fd (a: Int) (b: Int) -> Int:\n    Return a // b.\n\
         ## Main\n    Show fd(0 - 7, 2).\n    Show fd(7, 0 - 2).\n    Show fd(0 - 7, 0 - 2).\n",
    ),
    (
        "div_pow2",
        "## To hlf (x: Int) -> Int:\n    Return x / 2.\n## Main\n    Show hlf(9).\n    Show hlf(0 - 9).\n",
    ),
    (
        "global_read",
        "## To over (x: Int) -> Int:\n    Let d be x - limit.\n    Return d.\n\
         ## Main\n    Let limit be 50.\n    Show over(80).\n    Show over(20).\n",
    ),
    (
        "le_ge",
        "## To band (a: Int) (b: Int) -> Int:\n    If a is at most 10:\n        \
         If b is at least 3:\n            Return 1.\n    Return 0.\n\
         ## Main\n    Show band(5, 7).\n    Show band(20, 7).\n",
    ),
    (
        "eq_ne",
        "## To cmp (a: Int) (b: Int) -> Int:\n    If a equals b:\n        Return 1.\n    \
         If a != b:\n        Return 2.\n    Return 0.\n\
         ## Main\n    Show cmp(4, 4).\n    Show cmp(4, 5).\n",
    ),
    (
        "show_bool",
        "## To ask (a: Int) (b: Int):\n    Show a is greater than b.\n\
         ## Main\n    ask(5, 3).\n    ask(2, 8).\n",
    ),
    (
        "and_or",
        "## To pick (a: Int) (b: Int) -> Int:\n    \
         If a is greater than 0 and b is greater than 0:\n        Return 1.\n    \
         If a is greater than 0 or b is greater than 0:\n        Return 2.\n    Return 0.\n\
         ## Main\n    Show pick(1, 1).\n    Show pick(0 - 1, 3).\n    Show pick(0 - 1, 0 - 1).\n",
    ),
    (
        "not_cond",
        "## To neg (a: Int) (b: Int) -> Int:\n    If not (a is greater than b):\n        Return 1.\n    \
         Return 0.\n## Main\n    Show neg(3, 5).\n    Show neg(5, 3).\n",
    ),
    (
        "bitwise",
        "## To bits (a: Int) (b: Int) -> Int:\n    Let x be a & b.\n    Let y be a | b.\n    \
         Let z be a xor b.\n    Return x + y + z.\n## Main\n    Show bits(12, 10).\n    Show bits(255, 1).\n",
    ),
    // ── Phase 2a: f64 arithmetic / comparison / ApproxEq, observed via Int (float Show = Phase 2b) ──
    (
        "float_arith",
        "## To calc (a: Float) (b: Float) -> Int:\n    Let s be a + b.\n    Let d be a - b.\n    \
         Let p be a * b.\n    If p is greater than s:\n        If d is less than b:\n            Return 2.\n        \
         Return 1.\n    Return 0.\n## Main\n    Show calc(3.0, 2.0).\n    Show calc(1.0, 5.0).\n",
    ),
    (
        "float_div",
        "## To fd (a: Float) (b: Float) -> Int:\n    Let q be a / b.\n    If q is greater than 1.0:\n        \
         Return 1.\n    Return 0.\n## Main\n    Show fd(6.0, 2.0).\n    Show fd(1.0, 4.0).\n",
    ),
    (
        "float_cmp",
        "## To fc (a: Float) (b: Float) -> Int:\n    If a is at most b:\n        Return 1.\n    \
         If a is at least b:\n        Return 2.\n    Return 0.\n\
         ## Main\n    Show fc(1.0, 2.0).\n    Show fc(3.0, 2.0).\n    Show fc(2.0, 2.0).\n",
    ),
    (
        "float_eq",
        "## To feq (a: Float) (b: Float) -> Int:\n    If a equals b:\n        Return 1.\n    \
         If a != b:\n        Return 2.\n    Return 0.\n\
         ## Main\n    Show feq(2.0, 2.0).\n    Show feq(2.0, 3.0).\n",
    ),
    (
        "float_approx",
        "## To near (a: Float) (b: Float) -> Int:\n    If a is approximately b:\n        Return 1.\n    \
         Return 0.\n## Main\n    Show near(1.0, 1.0).\n    Show near(1.0, 2.0).\n",
    ),
    // ── Phase 2b: Show(Float) via the runtime shim, byte-for-byte with Rust's Display ──
    ("show_float_simple", "## Main\n    Let x be 1.5.\n    Show x.\n"),
    (
        // Integral floats print bare (2.0→"2"), and a non-integral value from runtime arithmetic
        // (so it isn't const-folded to a literal) exercises the shortest-round-trip formatter.
        "show_float_values",
        "## To q (a: Float) (b: Float) -> Float:\n    Return a / b.\n\
         ## Main\n    Let two be 2.0.\n    Show two.\n    Show q(7.0, 2.0).\n    Show q(1.0, 4.0).\n    \
         Show q(10.0, 5.0).\n",
    ),
    // ── Phase 2c: MIXED int/float arithmetic — the VM promotes the Int with `as f64` (lossy, RNE),
    //    which `fcvt.d.l` reproduces bit-for-bit. (Mixed COMPARISON uses exact semantics → still refused.)
    (
        "mixed_add",
        "## To addf (n: Int) (x: Float) -> Float:\n    Return n + x.\n## Main\n    Show addf(3, 1.5).\n",
    ),
    (
        "mixed_ops",
        "## To fma (n: Int) (x: Float) -> Float:\n    Let p be n * x.\n    Let d be x - n.\n    \
         Return p + d.\n## Main\n    Show fma(2, 1.5).\n    Show fma(4, 2.5).\n",
    ),
    (
        "mixed_div",
        "## To dv (n: Int) (x: Float) -> Float:\n    Return n / x.\n\
         ## Main\n    Show dv(6, 2.0).\n    Show dv(1, 4.0).\n",
    ),
    // ── Phase 2c.2: EXACT mixed int/float comparison (the VM's cmp_i64_f64_exact), covering the
    //    fast path, the reversed order, and the big-int (|i|>2^53) + ±2^63 boundary edges. ──
    (
        "mixed_cmp_order",
        "## To rel (n: Int) (x: Float) -> Int:\n    If n is greater than x:\n        Return 1.\n    \
         If n is less than x:\n        Return 0.\n    Return 2.\n\
         ## Main\n    Show rel(3, 3.5).\n    Show rel(4, 3.5).\n    Show rel(2, 2.0).\n",
    ),
    (
        "mixed_cmp_lege",
        "## To rel (n: Int) (x: Float) -> Int:\n    If n is at most x:\n        Return 1.\n    \
         If n is at least x:\n        Return 2.\n    Return 0.\n\
         ## Main\n    Show rel(2, 3.5).\n    Show rel(5, 3.5).\n    Show rel(3, 3.0).\n",
    ),
    (
        "mixed_cmp_eq",
        "## To eqc (n: Int) (x: Float) -> Int:\n    If n equals x:\n        Return 1.\n    \
         If n != x:\n        Return 2.\n    Return 0.\n\
         ## Main\n    Show eqc(2, 2.0).\n    Show eqc(2, 3.0).\n",
    ),
    (
        // Float operand FIRST (reversed order): `x OP n`.
        "mixed_cmp_reversed",
        "## To rev (x: Float) (n: Int) -> Int:\n    If x is greater than n:\n        Return 1.\n    \
         If x equals n:\n        Return 2.\n    Return 0.\n\
         ## Main\n    Show rev(3.5, 3).\n    Show rev(2.0, 2).\n    Show rev(1.5, 4).\n",
    ),
    (
        // Big-int slow path (|i| > 2^53) and the i64::MAX-vs-2^63 boundary — where a lossy fcvt would
        // diverge from the VM's exact rational comparison.
        "mixed_cmp_bigint",
        "## To rel (n: Int) (x: Float) -> Int:\n    If n is greater than x:\n        Return 1.\n    \
         If n equals x:\n        Return 2.\n    Return 0.\n\
         ## Main\n    Show rel(9007199254740993, 9007199254740992.0).\n    \
         Show rel(9223372036854775807, 9223372036854775808.0).\n",
    ),
    // ── Phase 3a: Text — constants, Show, and interpolation (LoadConst(Text) + FormatValue + Concat),
    //    stringifying Int/Float/Text operands byte-for-byte with the VM. ──
    ("text_show", "## Main\n    Show \"hello\".\n    Show \"world\".\n"),
    ("text_interp_int", "## Main\n    Let n be 42.\n    Show \"n is {n}\".\n"),
    ("text_interp_float", "## Main\n    Let x be 1.5.\n    Show \"x = {x}\".\n"),
    (
        "text_fn",
        "## To greet (name: Text) (n: Int) -> Text:\n    Return \"hi {name} #{n}\".\n\
         ## Main\n    Show greet(\"bob\", 3).\n    Show greet(\"amy\", 7).\n",
    ),
    // ── Phase 3b: Seq of Int — create/push (COW)/length/1-based index/for-each/Show. ──
    (
        "seq_build_sum",
        "## Main\n    Let mutable xs be a new Seq of Int.\n    Push 10 to xs.\n    Push 20 to xs.\n    \
         Push 30 to xs.\n    Show xs.\n    Show length of xs.\n    Show item 2 of xs.\n    \
         Let mutable t be 0.\n    Repeat for x in xs:\n        Set t to t + x.\n    Show t.\n",
    ),
    (
        "seq_dynamic_capacity",
        "## Main\n    Let n be 17.\n    Let mutable xs be a new Seq of Int with capacity n.\n    Push 10 to xs.\n    Push 20 to xs.\n    Show xs.\n    Show length of xs.\n",
    ),
    (
        "seq_param",
        "## To total (xs: Seq of Int) -> Int:\n    Let mutable t be 0.\n    \
         Repeat for x in xs:\n        Set t to t + x.\n    Return t.\n\
         ## Main\n    Let mutable a be a new Seq of Int.\n    Push 1 to a.\n    Push 2 to a.\n    \
         Push 3 to a.\n    Show total(a).\n    Show length of a.\n",
    ),
    (
        // Value semantics: `Let ys be xs` shares, but a push COPIES — so the alias must NOT see it.
        "seq_alias_cow",
        "## Main\n    Let mutable xs be a new Seq of Int.\n    Push 1 to xs.\n    Push 2 to xs.\n    \
         Let mutable ys be xs.\n    Push 3 to ys.\n    Show xs.\n    Show ys.\n",
    ),
    (
        // A runtime (param) index is not provably in-bounds, so it stays a checked `Op::Index`.
        "seq_index_param",
        "## To at (xs: Seq of Int) (k: Int) -> Int:\n    Return item k of xs.\n\
         ## Main\n    Let mutable a be a new Seq of Int.\n    Push 5 to a.\n    Push 6 to a.\n    \
         Push 7 to a.\n    Show at(a, 1).\n    Show at(a, 3).\n",
    ),
    // ── Phase 3b.2: Float- and Text-element sequences (uniform 8-byte cells; element kind → Show/index). ──
    (
        "seq_float",
        "## Main\n    Let mutable xs be a new Seq of Float.\n    Push 1.5 to xs.\n    Push 2.5 to xs.\n    \
         Push 2.0 to xs.\n    Show xs.\n    Show length of xs.\n    Let mutable t be 0.0.\n    \
         Repeat for x in xs:\n        Set t to t + x.\n    Show t.\n",
    ),
    (
        "seq_float_index",
        "## To at (xs: Seq of Float) (k: Int) -> Float:\n    Return item k of xs.\n\
         ## Main\n    Let mutable a be a new Seq of Float.\n    Push 3.5 to a.\n    Push 4.5 to a.\n    \
         Show at(a, 2).\n",
    ),
    (
        "seq_text",
        "## Main\n    Let mutable xs be a new Seq of Text.\n    Push \"a\" to xs.\n    Push \"bb\" to xs.\n    \
         Show xs.\n    Show length of xs.\n    Repeat for s in xs:\n        Show s.\n",
    ),
    // ── Phase 3b (rest): list literals, ranges, concat, slice, membership, indexed write, deep clone. ──
    ("list_literal", "## Main\n    Let xs be [1, 2, 3].\n    Show xs.\n    Show item 2 of xs.\n"),
    (
        // A counted `from A to B` range-for: the counter runs from A to B, no range value in between.
        "range_for",
        "## Main\n    Let mutable s be 0.\n    Repeat for i from 1 to 5:\n        Set s to s + i.\n    Show s.\n",
    ),
    (
        "seq_concat",
        "## Main\n    Let a be [1, 2].\n    Let b be [3, 4].\n    Let c be a followed by b.\n    Show c.\n",
    ),
    (
        "seq_slice",
        "## Main\n    Let xs be [10, 20, 30, 40, 50].\n    Let ys be items 2 through 4 of xs.\n    Show ys.\n",
    ),
    (
        "seq_contains",
        "## Main\n    Let xs be [10, 20, 30].\n    If xs contains 20:\n        Show 1.\n    Otherwise:\n        \
         Show 0.\n    If xs contains 99:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    ),
    (
        // Constant index → SetIndexUnchecked (Oracle proves in-bounds). Both writes are copy-on-write.
        "seq_setindex_const",
        "## Main\n    Let mutable xs be [5, 6, 7].\n    Set item 1 of xs to 99.\n    Set item 3 of xs to 88.\n    \
         Show xs.\n",
    ),
    (
        // A param index is not provably in-bounds → checked Op::SetIndex (COW; the returned seq carries it).
        "seq_setindex_param",
        "## To setat (xs: Seq of Int) (k: Int) -> Seq of Int:\n    Set item k of xs to 99.\n    Return xs.\n\
         ## Main\n    Let a be [1, 2, 3].\n    Let b be setat(a, 2).\n    Show b.\n",
    ),
    (
        "seq_deepclone",
        "## Main\n    Let a be [1, 2].\n    Let b be [3, 4].\n    Let c be a followed by b.\n    \
         Let d be copy of c.\n    Show d.\n",
    ),
    // ── Phase 3d: Sets of Int/Text — create / add (dedup) / remove / union / intersect / membership. ──
    (
        "set_int",
        "## Main\n    Let mutable s be a new Set of Int.\n    Add 1 to s.\n    Add 2 to s.\n    Add 1 to s.\n    \
         Remove 2 from s.\n    Show s.\n    If s contains 1:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    ),
    (
        "set_algebra",
        "## Main\n    Let mutable a be a new Set of Int.\n    Add 1 to a.\n    Add 2 to a.\n    \
         Let mutable b be a new Set of Int.\n    Add 2 to b.\n    Add 3 to b.\n    \
         Let u be a union b.\n    Let i be a intersection b.\n    Show u.\n    Show i.\n",
    ),
    (
        "set_text",
        "## Main\n    Let mutable s be a new Set of Text.\n    Add \"alice\" to s.\n    Add \"bob\" to s.\n    \
         Add \"alice\" to s.\n    Show s.\n    Show length of s.\n",
    ),
    // ── Composite kinds: Maps (key→value) and Tuple multi-return. ──
    (
        "map_text_int",
        "## Main\n    Let mutable m be a new Map of Text to Int.\n    Set item \"alice\" of m to 30.\n    \
         Set item \"bob\" of m to 25.\n    Set item \"alice\" of m to 31.\n    Show m.\n    \
         Show item \"alice\" of m.\n",
    ),
    (
        "map_dynamic_capacity",
        "## Main\n    Let n be 17.\n    Let mutable m be a new Map of Int to Int with capacity n.\n    Set item 5 of m to 50.\n    Show item 5 of m.\n    Show length of m.\n",
    ),
    (
        // Multi-return via a tuple: NewTuple in the function, DestructureTuple at the call site.
        "tuple_multireturn",
        "## To pair (a: Int) (b: Int) -> (Int, Int):\n    Return (a + b, a - b).\n\
         ## Main\n    Let (s, d) be pair(10, 3).\n    Show s.\n    Show d.\n",
    ),
    // ── Phase 4: structs — create / field set (COW) / field get / Show (fields sorted by name). ──
    (
        "struct_point",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet mutable p be a new Point with x 10 and y 20.\n\
         Show p.\nSet p's x to 5.\nShow p's x.\nShow p.\n",
    ),
    (
        // Mixed field kinds + name-sorted Show: `age` sorts before `name`, text value unquoted.
        "struct_person",
        "## A Person has:\n    A name: Text.\n    An age: Int.\n\n## Main\n\
         Let p be a new Person with name \"Bob\" and age 25.\nShow p.\nShow p's name.\n",
    ),
    // ── Phase 4: enums — construct a variant; Inspect arm tag-test (nullary) + payload bind. ──
    (
        "enum_inspect_nullary",
        "## A Color is one of:\n    A Red.\n    A Green.\n    A Blue.\n\n## Main\nLet c be a new Green.\n\
         Inspect c:\n    When Red: Show \"red\".\n    When Green: Show \"green\".\n    When Blue: Show \"blue\".\n",
    ),
    (
        "enum_inspect_payload",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## Main\nLet s be a new Circle with radius 10.\n\
         Inspect s:\n    When Circle (r): Show r.\n    When Rectangle (w, h): Show w.\n",
    ),
    // ── Tail: shifts (masked wrapping) + exponentiation (Int squaring / Float libm). ──
    (
        "shifts",
        "## To sh (a: Int) (b: Int) -> Int:\n    Let l be a shifted left by b.\n    \
         Let r be a shifted right by b.\n    Return l + r.\n\
         ## Main\n    Show sh(1, 4).\n    Show sh(256, 3).\n",
    ),
    (
        "int_pow",
        "## To p (b: Int) (e: Int) -> Int:\n    Return b ** e.\n## Main\n    Show p(3, 4).\n    Show p(2, 10).\n",
    ),
    (
        "float_pow",
        "## To fp (b: Float) (e: Float) -> Float:\n    Return b ** e.\n\
         ## Main\n    Show fp(2.0, 3.0).\n    Show fp(9.0, 0.5).\n",
    ),
    (
        // A mutable-borrow call (`Set a to bump(a, i)`) emits Op::EnsureOwned before the argument; the
        // callee's SetIndex is copy-on-write, so the caller's value semantics hold.
        "mut_borrow",
        "## To bump (arr: Seq of Int) (i: Int) -> Seq of Int:\n    Let mutable r be arr.\n    \
         Set item i of r to 99.\n    Return r.\n\
         ## Main\n    Let mutable a be a new Seq of Int.\n    Push 1 to a.\n    Push 2 to a.\n    \
         Set a to bump(a, 1).\n    Show item 1 of a.\n    Show item 2 of a.\n",
    ),
    (
        // `length of` is Oracle-proven non-negative, so a constant non-power-of-two divisor lowers to
        // the magic reciprocal (Op::MagicDivU), for both `/` and `%`.
        "magic_div",
        "## Main\n    Let mutable xs be a new Seq of Int.\n    Push 10 to xs.\n    Push 20 to xs.\n    \
         Push 30 to xs.\n    Push 40 to xs.\n    Push 50 to xs.\n    Push 60 to xs.\n    Push 70 to xs.\n    \
         Let n be length of xs.\n    Show n / 3.\n    Show n % 3.\n",
    ),
    (
        // Scalar numeric builtins: abs / min / max / floor / ceil / round (Int result) and sqrt (Float).
        "builtins_int",
        "## To fb (n: Int) (x: Float) -> Int:\n    \
         Return abs(n) + min(n, 3) + max(n, 4) + floor(x) + ceil(x) + round(x).\n\
         ## Main\n    Show fb(0 - 5, 2.7).\n    Show fb(8, 1.2).\n",
    ),
    (
        "builtins_sqrt",
        "## To fs (x: Float) -> Float:\n    Return sqrt(x).\n## Main\n    Show fs(9.0).\n    Show fs(2.0).\n",
    ),
    (
        // parseInt(Text) — strict base-10 Text→Int via the `logos_parse_int` runtime shim (Int result),
        // held byte-for-byte against the tree-walker's parse. The args()-driven benchmarks already lean on
        // this lowering; this is the standalone correctness lock.
        "builtins_parseint",
        "## Main\n    Show parseInt(\"42\").\n    Show parseInt(\"100\").\n",
    ),
    ("args_empty", "## Main\n    Let a be args().\n    Show length of a.\n"),
    (
        // Push to a struct's Seq field (Op::ListPushField, copy-on-write), then read it back.
        "list_push_field",
        "## A Bag has:\n    A items: Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items (a new Seq of Int).\n\
         Push 1 to b's items.\nPush 2 to b's items.\nShow length of b's items.\n    Show item 2 of b's items.\n",
    ),
    (
        // CheckPolicy — a predicate (`FieldEquals` on a Text field) that PASSES, and an `Or` predicate
        // that passes on its second disjunct; the compiled condition matches `evaluate_policy_condition`.
        "policy_allowed",
        "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\n\
         A User is admin if the user's role equals \"admin\".\n\
         A User is staff if the user's role equals \"admin\" or the user's role equals \"mod\".\n\n\
         ## To process (u: User):\n    Check that the u is admin.\n    Check that the u is staff.\n    Show \"allowed\".\n\n\
         ## Main\nLet u be a new User with role \"admin\".\nprocess(u).\n",
    ),
    (
        // Single-replica counter CRDTs (Op::NewCrdt + Op::CrdtBump): a grow-only ConvergentCount and a
        // PN-Tally (increase + decrease), each read back as a plain Int count.
        "crdt_counters",
        "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n\
         A Wallet is Shared and has:\n    balance: Tally.\n## Main\n    \
         Let mutable c be a new Counter.\n    Increase c's points by 5.\n    Increase c's points by 3.\n    \
         Show c's points.\n    \
         Let mutable w be a new Wallet.\n    Increase w's balance by 10.\n    Decrease w's balance by 4.\n    \
         Show w's balance.\n",
    ),
    (
        // CRDT counter MERGE (Op::CrdtMerge) — the per-replica-MAX lattice join: a single merge sums two
        // replicas' counts, and a REPEATED merge is idempotent (still 8, never double-counts) like the VM.
        "crdt_merge",
        "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    \
         Let mutable a be a new Counter.\n    Increase a's points by 5.\n    \
         Let mutable b be a new Counter.\n    Increase b's points by 3.\n    \
         Merge b into a.\n    Merge b into a.\n    Show a's points.\n",
    ),
    (
        // CRDT sequence APPEND (Op::CrdtAppend) — `Append` mutates the shared `Seq` field in place.
        "crdt_append",
        "## A Bag has:\n    nums: Seq of Int.\n\n## Main\n    Let d be a new Bag.\n    \
         Append 5 to d's nums.\n    Append 7 to d's nums.\n    Show length of d's nums.\n    Show item 2 of d's nums.\n",
    ),
    (
        // A REAL sequence CRDT (Op::NewCrdt kind=1 + Op::CrdtAppend) — a `SharedSequence of Int` field. On
        // the single replica the converged log IS a plain `Seq` behind a stable handle (the field lays out
        // as its underlying `Seq of T`), so `NewCrdt` seeds an empty seq, `Append` pushes in place through
        // every alias, and `Show length` reads it back — byte-for-byte the VM. (The plain-`Bag` `crdt_append`
        // above emits `NewEmptyList`, not `NewCrdt`; this locks the RGA-seed path.)
        "crdt_shared_sequence_append",
        "## Definition\nA Log is Shared and has:\n    a entries, which is a SharedSequence of Int.\n## Main\n    \
         Let mutable g be a new Log.\n    Append 5 to g's entries.\n    Append 6 to g's entries.\n    \
         Show length of g's entries.\n",
    ),
    (
        // The same RGA-seed path with `Text` elements, rendering the whole converged log — proves
        // `Show <shared-seq>` formats `[a, b, c]` exactly like the VM (the op_backend_matrix gauntlet form).
        "crdt_shared_sequence_show_entries",
        "## Definition\nA Log is Shared and has:\n    a entries, which is a SharedSequence of Text.\n## Main\n    \
         Let mutable g be a new Log.\n    Append \"a\" to g's entries.\n    Append \"b\" to g's entries.\n    \
         Append \"c\" to g's entries.\n    Show g's entries.\n",
    ),
    (
        // CRDT multi-value register RESOLVE (Op::CrdtResolve) — LWW: the last value written wins.
        "crdt_resolve",
        "## Definition\nA Page is Shared and has:\n    rank: Divergent Int.\n## Main\n    \
         Let mutable p be a new Page.\n    Resolve p's rank to 3.\n    Resolve p's rank to 9.\n    Show p's rank.\n",
    ),
    (
        // Temporal (Op::LoadToday + Op::LoadNow) — `today`/`now` under the injected fixed clock render
        // `YYYY-MM-DD` and `YYYY-MM-DD HH:MM` byte-for-byte like the VM.
        "temporal",
        "## Main\n    Let d be today.\n    Show d.\n    Let m be now.\n    Show m.\n",
    ),
    (
        // Closures (Op::MakeClosure + Op::CallValue): a var-stored closure, an inline closure over a
        // dynamic arg, and a closure CAPTURING a runtime local — each applied via a higher-order `apply`.
        "closures",
        "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n## Main\n    \
         Let g be (n: Int) -> n * n.\n    Show apply(g, 7).\n    \
         Let mutable d be 0.\n    Repeat for i from 1 to 6:\n        Set d to d + 1.\n    \
         Show apply((n: Int) -> n * 2, d).\n    \
         Let mutable c be 0.\n    Repeat for i from 1 to 10:\n        Set c to c + 1.\n    \
         Show apply((n: Int) -> n + c, 5).\n",
    ),
    (
        // Rational (Op::ExactDiv) — exact division, arithmetic that keeps it exact (add/sub/mul), Int
        // promotion (`r + 3`), a whole-quotient downsize (Show `num` when den==1), and comparison.
        "rational_exact",
        "## Main\n    Let x: Rational be 7 / 2.\n    Show x.\n    \
         Let a: Rational be 1 / 3 + 1 / 6.\n    Show a.\n    \
         Let r: Rational be 1 / 2.\n    Let s be r + 3.\n    Show s.\n    \
         Let m: Rational be 2 / 3.\n    Show m * 3.\n    \
         Let d: Rational be 3 / 4.\n    Show d - 1 / 4.\n    \
         Let p: Rational be 2 / 3.\n    Show p is greater than r.\n",
    ),
    (
        // FormatValue — every spec branch (precision on Float+Int, currency on Float+Int, the three
        // alignments, bare width) and the `{name=}` debug prefix, each ≡ apply_format_spec.
        "format_spec_all",
        "## Main\n    Let pi be 3.14159.\n    Let n be 42.\n    Let p be 19.5.\n    Let q be 20.\n    \
         Show \"{pi:.2}\".\n    Show \"{n:.2}\".\n    Show \"{p:$}\".\n    Show \"{q:$}\".\n    \
         Show \"[{n:>5}]\".\n    Show \"[{n:<5}]\".\n    Show \"[{n:^5}]\".\n    Show \"[{n:5}]\".\n    \
         Show \"{n=}\".\n",
    ),
    (
        // Tuple-pattern for-each over a Seq of tuples (Op::DestructureTuple; the seq's composite element
        // kind flows so `a` and `b` get their per-position kinds).
        "destructure_tuple_foreach",
        "## Main\n    Let ps be [(1, 2), (3, 4)].\n    Repeat for (a, b) in ps:\n        Show a + b.\n",
    ),
    (
        // `Pop from xs into v` (Op::ListPop) — value-semantics COW pop, then read the popped value and
        // the shortened length.
        "list_pop_into",
        "## Main\n    Let mutable xs be [1, 2, 3].\n    Pop from xs into v.\n    Show v.\n    Show length of xs.\n",
    ),
    (
        // A while-loop indexing an array at the induction variable (checked Index; region hint is a no-op).
        "region_guard",
        "## To total (xs: Seq of Int) -> Int:\n    Let mutable t be 0.\n    Let mutable i be 1.\n    \
         While i is at most length of xs:\n        Set t to t + item i of xs.\n        Set i to i + 1.\n    \
         Return t.\n## Main\n    Let mutable a be a new Seq of Int.\n    Push 5 to a.\n    Push 6 to a.\n    \
         Push 7 to a.\n    Show total(a).\n",
    ),
    (
        // `Show` of a tuple: each position as its own kind displays, `(1, 2)`.
        "shows_tuple",
        "## Main\n    Let t be (1, 2).\n    Show t.\n",
    ),
];

/// Programs that ERROR at runtime on the VM (a runtime divide/modulo by zero). The RISC-V binary must
/// exit NONZERO exactly where the VM errors, with stdout equal to the partial output before the error.
const ERROR_CORPUS: &[(&str, &str)] = &[
    (
        "div_zero",
        "## To dz (a: Int) (b: Int) -> Int:\n    Return a / b.\n\
         ## Main\n    Show 111.\n    Show dz(5, 0).\n",
    ),
    (
        "mod_zero",
        "## To mz (a: Int) (b: Int) -> Int:\n    Return a % b.\n## Main\n    Show mz(5, 0).\n",
    ),
    (
        // Float division by zero ERRORS in LOGOS (never inf/nan). Runtime divisor (param) so it is not
        // folded to a compile-time FailWith; `x` is used so the call is not eliminated.
        "float_div_zero",
        "## To fdz (a: Float) (b: Float) -> Float:\n    Return a / b.\n\
         ## Main\n    Show 5.\n    Let x be fdz(6.0, 0.0).\n    If x is greater than 1.0:\n        Show 1.\n",
    ),
    (
        // Exact (Rational) division by zero ERRORS like the VM. Runtime divisor so it is not folded.
        "rational_div_zero",
        "## Main\n    Show 5.\n    Let z be 0.\n    Let x: Rational be 7 / z.\n    Show x.\n",
    ),
    (
        // A DENIED policy check errors ("Security Check Failed") exactly like the VM → exits nonzero.
        "policy_denied",
        "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\n\
         A User is admin if the user's role equals \"admin\".\n\n\
         ## To process (u: User):\n    Show 1.\n    Check that the u is admin.\n    Show 2.\n\n\
         ## Main\nLet u be a new User with role \"guest\".\nprocess(u).\n",
    ),
    (
        // A contract that does not hold compiles to `FailWith` (no reason: `Assertion failed`).
        "contract_fails",
        "## Main\n    Show 1.\n    Require that 1 equals 2.\n    Show 2.\n",
    ),
];

/// Programs using an op/shape outside the lowered fragment — the backend must REFUSE (return `Err`),
/// never miscompile. Proves the "unsupported ⇒ refuse" direction of the biconditional.
const REFUSE_CORPUS: &[(&str, &str)] = &[
    // Enums (NewInductive/TestArm/BindArm) are not yet lowered → REFUSED. (Phase 4 enums, next.)
    ("uses_enum", "## A Color is one of:\n    A Red.\n    A Green.\n\n## Main\nLet c be a new Red.\nShow c.\n"),
];

/// CONCURRENCY corpus — programs using the cooperative-scheduler ops. These are held against the
/// CONCURRENT oracle pair (`interpret_for_ui_sync` + `run_vm_concurrent`, which share one deterministic
/// seeded scheduler) rather than the plain synchronous oracles, since a concurrency program only runs on
/// the async tree-walker / concurrent VM driver. The RISC-V binary (its ucontext coroutine runtime) must
/// match them byte-for-byte.
const CONC_CORPUS: &[(&str, &str)] = &[
    (
        // Producer task → channel → main consumer (ChanNew + ChanSend + ChanRecv + Spawn). FIFO order.
        "producer_consumer",
        "## To produce (ch: Int):\n    Send 1 into ch.\n    Send 2 into ch.\n    Send 3 into ch.\n\n\
         ## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    \
         Receive a from jobs.\n    Receive b from jobs.\n    Receive c from jobs.\n    Show a.\n    Show b.\n    Show c.\n",
    ),
    (
        // Fire-and-forget (Spawn): the spawned worker runs BEFORE main continues (spawn-to-front).
        "fire_and_forget",
        "## To worker:\n    Show \"worker ran\".\n\n## Main\n    Launch a task to worker.\n    Show \"main ran\".\n",
    ),
    (
        // Spawn WITH a handle (SpawnHandle) — the handle is bound but the worker still runs first.
        "spawn_handle",
        "## To worker:\n    Show \"w\".\n\n## Main\n    Let h be Launch a task to worker.\n    Show \"m\".\n",
    ),
    (
        // Try-send into an unbounded channel succeeds (ChanTrySend), then the value is received.
        "try_send",
        "## Main\n    Let ch be a Pipe of Int.\n    Try to send 99 into ch.\n    Receive x from ch.\n    Show x.\n",
    ),
    (
        // Virtual-time Sleep (Op::Sleep): produces no output, the logical clock advances, execution resumes.
        "sleep",
        "## Main\n    Show 1.\n    Sleep 1.\n    Show 2.\n",
    ),
    (
        // Select where a Receive arm wins over a timeout (SelectArmRecv + SelectArmTimeout + SelectWait).
        "select_recv",
        "## To produce (ch: Int):\n    Send 7 into ch.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
         Launch a task to produce with ch.\n    Await the first of:\n        Receive x from ch:\n            Show x.\n        \
         After 1 seconds:\n            Show \"timeout\".\n",
    ),
    (
        // Select with only a timeout arm: the logical clock advances and the timeout fires.
        "select_timeout",
        "## Main\n    Let p be a Pipe of Text.\n    Await the first of:\n        After 1 seconds:\n            Show \"timeout\".\n",
    ),
    (
        // Try-receive from an EMPTY channel → an Optional that Shows `nothing` (ChanTryRecv).
        "tryrecv_empty",
        "## Main\n    Let ch be a Pipe of Int.\n    Try to receive x from ch.\n    Show x.\n",
    ),
    (
        // Try-receive after a buffered try-send → the Optional Shows the value.
        "tryrecv_present",
        "## Main\n    Let ch be a Pipe of Int.\n    Try to send 5 into ch.\n    Try to receive x from ch.\n    Show x.\n",
    ),
    (
        // Stop a task blocked on Receive (Op::TaskAbort) — its post-receive code never runs.
        "abort_blocked",
        "## To blocker (ch: Int):\n    Receive x from ch.\n    Show \"got\".\n\n## Main\n    Let ch be a Pipe of Int.\n    \
         Let h be Launch a task to blocker with ch.\n    Stop h.\n    Show \"main\".\n",
    ),
];

/// NETWORKING corpus — the offline loopback transport (peer messaging without a live relay). Held
/// against the OFFLINE oracle pair (`tw_outcome` + `vm_outcome_net`, both under `set_net_offline`); the
/// RISC-V binary's value-level loopback must match byte-for-byte.
const NET_CORPUS: &[(&str, &str)] = &[
    (
        // Connect (offline no-op) then continue.
        "net_connect",
        "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Show \"connected\".\n",
    ),
    (
        // Full peer loopback: MakePeer + Send + Await round-trip a message to a self-peer.
        "net_loopback",
        "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Listen on \"me\".\n    \
         Let peer be a PeerAgent at \"me\".\n    Send 7 to peer.\n    Await response from peer into got.\n    Show got.\n",
    ),
    (
        // Stream a Seq to a peer, await the batch back (NetStream + stream NetAwait).
        "net_stream",
        "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Listen on \"me\".\n    \
         Let peer be a PeerAgent at \"me\".\n    Let vals be [1, 2, 3].\n    Stream vals to peer.\n    \
         Await stream from peer into b.\n    Show item 2 of b.\n",
    ),
    (
        // Sync a variable over a topic — offline-identity (no peer state), so it is unchanged.
        "net_sync",
        "## Main\n    Let mutable c be 5.\n    Connect to \"ws://127.0.0.1:9\".\n    Sync c on \"t\".\n    Show c.\n",
    ),
];

// ============================================================
//  Helpers
// ============================================================

fn op_name(op: &Op) -> &'static str {
    op.name()
}

/// Compile `src` to bytecode via the SAME Oracle path `compile_to_riscv` uses, returning its full op
/// stream (Main + every function body), or `None` if it does not reach the VM.
fn program_ops(src: &str) -> Option<Vec<Op>> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.ok()?;
        let oracle = logicaffeine_compile::optimize::oracle_analyze_with(stmts, interner);
        let program =
            Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle)).ok()?;
        Some(program.code.clone())
    })
}

/// The behavioural half's toolchain, through the honest seam: an absent tool is a recorded skip
/// (`LOGOS_TOOL_SKIP`), and a failure when `LOGOS_REQUIRE_TOOLS` names it.
fn have_toolchain() -> bool {
    common::require_tool("riscv64-linux-gnu-gcc") && common::require_tool("qemu-riscv64")
}

/// The fixed clock every RISC-V run reads through the runtime shim (`LOGOS_FIXED_*`), the instant the VM
/// oracle is pinned to.
fn fixed_clock() -> [(&'static str, String); 2] {
    [
        ("LOGOS_FIXED_DAYS", FIXED_DAYS.to_string()),
        ("LOGOS_FIXED_NANOS", FIXED_NANOS.to_string()),
    ]
}

/// Link `asm` and run it once with no arguments, bounded (`riscv_runner::run`).
fn assemble_and_run(asm: &str, label: &str) -> RvRun {
    riscv_runner::run(&riscv_runner::link(asm, label), &[], &fixed_clock())
}

/// The benchmark programs' root, `benchmarks/programs`.
const BENCHMARK_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/programs");

/// The directory of every benchmark program, sorted.
fn benchmark_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = match std::fs::read_dir(BENCHMARK_ROOT) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
        Err(_) => Vec::new(),
    };
    dirs.sort();
    dirs
}

/// Every real benchmark + practice program (`benchmarks/programs/*/{main,interp}.lg`), read from disk.
/// These are the ACTUAL programs the project ships, not hand-written corpus snippets — so this is the
/// honest end-to-end proof that the tier runs real code.
fn benchmark_programs() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for dir in benchmark_dirs() {
        let n = dir.file_name().unwrap().to_string_lossy().to_string();
        for variant in ["main", "interp"] {
            if let Ok(s) = std::fs::read_to_string(dir.join(format!("{variant}.lg"))) {
                out.push((format!("{n}/{variant}"), s));
            }
        }
    }
    out
}

// ============================================================
//  Static ratchet tests (always run — no toolchain needed)
// ============================================================

#[test]
fn supported_op_count_never_regresses() {
    let n = Op::all_variants()
        .iter()
        .filter(|o| matches!(op_support(o), Support::Supported))
        .count();
    assert!(
        n >= SUPPORTED_FLOOR,
        "RISC-V supported-op count fell to {n} (floor {SUPPORTED_FLOOR}). A previously-supported op \
         stopped being supported — fix the emitter, do not lower the floor."
    );
}

#[test]
fn known_gaps_are_shrink_only() {
    let live: BTreeSet<&'static str> = Op::all_variants()
        .iter()
        .filter(|o| matches!(op_support(o), Support::Deferred(_)))
        .map(op_name)
        .collect();
    let declared: BTreeSet<&'static str> = KNOWN_GAPS.iter().copied().collect();

    let newly_appeared: Vec<&&str> = live.difference(&declared).collect();
    assert!(
        newly_appeared.is_empty(),
        "NEW deferral(s) appeared and are not in KNOWN_GAPS: {newly_appeared:?}. Either lower the op \
         (preferred) or add it to KNOWN_GAPS consciously."
    );
    let fixed_but_listed: Vec<&&str> = declared.difference(&live).collect();
    assert!(
        fixed_but_listed.is_empty(),
        "KNOWN_GAPS lists op(s) that are no longer Deferred: {fixed_but_listed:?}. Remove them (and \
         raise SUPPORTED_FLOOR) — KNOWN_GAPS is shrink-only."
    );
}

#[test]
fn every_supported_instruction_is_exercised() {
    let mut exercised: BTreeSet<&'static str> = BTreeSet::new();
    for (_, src) in CORPUS.iter().chain(ERROR_CORPUS).chain(CONC_CORPUS) {
        if let Some(ops) = program_ops(src) {
            for op in &ops {
                exercised.insert(op_name(op));
            }
        }
    }
    for (_, src) in NET_CORPUS {
        if let Some(ops) = program_ops(src) {
            for op in &ops {
                exercised.insert(op_name(op));
            }
        }
    }
    // Hand-built bytecode programs exercise the ops no source emits into an AOT program (reversible
    // primitives, the BigInt lane, narrow-VM list, obs/region no-ops, scheduler-only Chan/Task ops).
    for (_, prog) in bytecode_programs() {
        for op in &prog.code {
            exercised.insert(op_name(op));
        }
    }

    let catalog = Op::all_variants();
    assert!(
        catalog.len() >= 100,
        "the instruction catalog shrank to {} — a variant was dropped",
        catalog.len()
    );

    let mut unexercised: Vec<&'static str> = catalog
        .iter()
        .filter(|o| matches!(op_support(o), Support::Supported) && !exercised.contains(op_name(o)))
        .map(op_name)
        .collect();
    unexercised.sort_unstable();
    unexercised.dedup();
    assert!(
        unexercised.is_empty(),
        "COVERAGE GAP: these ops are classified Supported but NO compiling corpus program exercises \
         them, so RISC-V ≡ VM is UNPROVEN for them: {unexercised:?}. Add a corpus program (or fix the \
         classification)."
    );
}

// ============================================================
//  Behavioural biconditional (toolchain-gated)
// ============================================================

#[test]
fn oracles_agree_over_the_corpus() {
    // Sanity: the tree-walker and VM agree on every corpus program, so "≡ VM" and "≡ tree-walker"
    // are the same target. (Runs without the RISC-V toolchain.)
    // Pin the clock so `today`/`now` are deterministic (the AOT binary reads the same via env vars).
    logicaffeine_compile::semantics::temporal::set_fixed_clock(FIXED_DAYS as i32, FIXED_NANOS);
    for (label, src) in CORPUS.iter().chain(ERROR_CORPUS) {
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert_eq!(
            (tw.output.trim(), tw.error.is_some()),
            (vm.output.trim(), vm.error.is_some()),
            "oracle divergence on `{label}`"
        );
    }
}

#[test]
fn riscv_equals_vm_and_treewalker_over_the_corpus() {
    // Pin the clock so the VM oracle's `today`/`now` match the AOT binary (which reads the same instant
    // from the `LOGOS_FIXED_*` env vars `assemble_and_run` sets on the qemu process).
    logicaffeine_compile::semantics::temporal::set_fixed_clock(FIXED_DAYS as i32, FIXED_NANOS);
    // Every corpus program uses only Supported ops (guard against sneaking a Deferred op in).
    for (label, src) in CORPUS {
        let ops =
            program_ops(src).unwrap_or_else(|| panic!("`{label}` failed to compile to bytecode"));
        let offenders: Vec<&'static str> = ops
            .iter()
            .filter(|o| !matches!(op_support(o), Support::Supported | Support::Unreachable(_)))
            .map(op_name)
            .collect();
        assert!(
            offenders.is_empty(),
            "`{label}` uses non-Supported ops {offenders:?} — fix the corpus"
        );
        assert!(
            compile_to_riscv(src).is_ok(),
            "`{label}` is fully supported but the RISC-V backend refused it"
        );
    }

    if !have_toolchain() {
        return;
    }
    for (label, src) in CORPUS {
        let asm = compile_to_riscv(src).unwrap();
        let run = assemble_and_run(&asm, label);
        // `assemble_and_run` invokes the binary as `qemu <path>`, so the guest sees argc == 1 (argv[0] is
        // the program name). `args()` includes argv[0] on every tier, so the tree-walker oracle must see
        // the SAME single-element argv for the differential to be fair — otherwise an `args()` program
        // reads length 1 on the binary but 0 on a no-argv oracle. The argv[0] STRING differs (a temp path
        // vs "prog"), so a corpus program may only observe its LENGTH, never argv[0]'s content.
        let tw = tw_outcome_with_args(src, &["prog".to_string()]);
        assert!(run.success, "`{label}` should exit cleanly.\n\nAsm:\n{asm}");
        assert_eq!(
            run.stdout.trim(),
            tw.output.trim(),
            "`{label}` RISC-V output != tree-walker"
        );
    }
}

/// Argument sizes fed to a benchmark (as `item 2 of args()`) — small so both oracles stay fast, but
/// spanning the boundary (1), odd/even, and a larger value to shake out size-dependent lowering. Each
/// program runs at the ones no larger than its own smallest declared workload
/// (`riscv_runner::differential_sizes`), so a factorial or exponential program stops below the size it
/// cannot finish at on any tier.
const BENCHMARK_SIZES: &[u64] = &[1, 3, 8, 16];

/// The number of real benchmark files (`main`/`interp` variants) proven to compile AND run byte-for-byte
/// == the VM under qemu. MONOTONE — raise it when more programs pass, NEVER lower it. A regression that
/// makes a previously-runnable benchmark refuse to compile (or diverge) drops below the floor → RED.
const BENCHMARK_FLOOR: usize = 70;

/// ★ THE HONEST END-TO-END PROOF ★ — every real benchmark/practice program the project ships
/// (`benchmarks/programs/*/{main,interp}.lg`) that compiles MUST run byte-for-byte identical to the VM
/// under qemu, across a range of input sizes. This is what "the RISC-V tier actually works" MEANS: not
/// that ops are classified Supported, but that real code produces the real answer. The static half (the
/// compile FLOOR) always runs; the execution half is toolchain-gated.
/// ★ `#[ignore]`: minutes of single-core emulation over the whole corpus (274 runs, measured 134 s at the
/// test profile; the `qemu-corpus` group in `.config/nextest.toml`), so neither the routine baseline nor a
/// per-crate `cargo nextest run` runs it: `scripts/run-qemu-corpus-tests.sh` does, with
/// `--run-ignored all`.
#[test]
#[ignore = "minutes of qemu emulation over the whole benchmark corpus (274 runs). EXCLUDED from the routine baseline by run-all-tests-fast.sh; run on purpose via scripts/run-qemu-corpus-tests.sh, which passes --run-ignored all."]
fn riscv_matches_the_vm_on_the_benchmark_corpus() {
    logicaffeine_compile::semantics::temporal::set_fixed_clock(FIXED_DAYS as i32, FIXED_NANOS);
    let progs = benchmark_programs();
    assert!(
        !progs.is_empty(),
        "found no benchmark programs — is benchmarks/programs/ present?"
    );

    // Static half (no toolchain): the compile floor can only rise.
    let compiling: Vec<&(String, String)> = progs
        .iter()
        .filter(|(_, s)| compile_to_riscv(s).is_ok())
        .collect();
    assert!(
        compiling.len() >= BENCHMARK_FLOOR,
        "benchmark RISC-V compile coverage fell to {} (floor {BENCHMARK_FLOOR}). A previously-runnable \
         program now refuses to compile — fix the emitter, do not lower the floor.",
        compiling.len()
    );

    if !have_toolchain() {
        return;
    }
    // Execution half: every compiling benchmark runs == the VM at each of its sizes, one link per program.
    for (name, src) in &compiling {
        let dir = std::path::Path::new(BENCHMARK_ROOT).join(name.split('/').next().unwrap());
        let linked = riscv_runner::link(&compile_to_riscv(src).unwrap(), name);
        for size in riscv_runner::differential_sizes(&dir, BENCHMARK_SIZES) {
            let run = riscv_runner::run(&linked, &[&size], &fixed_clock());
            let vm = vm_outcome_with_args(src, &["prog".to_string(), size.clone()], None);
            assert_eq!(
                run.success,
                vm.error.is_none(),
                "`{name}`[{size}] exit-status parity broke (vm error = {:?}; RISC-V {})",
                vm.error,
                run.describe()
            );
            assert_eq!(
                run.stdout.trim(),
                vm.output.trim(),
                "`{name}`[{size}] RISC-V output != VM"
            );
        }
    }
}

/// Every benchmark program declares the workload sizes the differentials read: `riscv_runner::declared_sizes`
/// panics, naming the file, on a missing or empty `sizes.txt` or a size that is not a positive integer.
#[test]
fn every_benchmark_declares_its_workload_sizes() {
    let dirs = benchmark_dirs();
    assert!(
        !dirs.is_empty(),
        "found no benchmark programs — is benchmarks/programs/ present?"
    );
    for dir in dirs {
        riscv_runner::declared_sizes(&dir);
    }
}

/// A differential never runs a program above its smallest declared workload: fannkuch's workloads start
/// at 9 (9! permutations), so it stops at 8 — at 16 it is 16! permutations, past what any tier finishes —
/// while fib, whose workloads start in the tens, runs at every size.
#[test]
fn a_benchmark_runs_below_its_smallest_workload() {
    let at = |name: &str| {
        riscv_runner::differential_sizes(
            &std::path::Path::new(BENCHMARK_ROOT).join(name),
            BENCHMARK_SIZES,
        )
    };
    assert_eq!(at("fannkuch"), vec!["1", "3", "8"]);
    assert_eq!(at("fib"), vec!["1", "3", "8", "16"]);
}

#[test]
fn riscv_exits_nonzero_where_the_vm_errors() {
    for (label, src) in ERROR_CORPUS {
        assert!(
            compile_to_riscv(src).is_ok(),
            "`{label}` should compile (its ops are Supported)"
        );
    }
    if !have_toolchain() {
        return;
    }
    for (label, src) in ERROR_CORPUS {
        let tw = tw_outcome(src);
        assert!(
            tw.error.is_some(),
            "`{label}` must error on the tree-walker (bad error-corpus entry)"
        );
        let asm = compile_to_riscv(src).unwrap();
        let run = assemble_and_run(&asm, label);
        assert!(
            !run.success,
            "`{label}` must exit NONZERO where the VM errors"
        );
        assert_eq!(
            run.stdout.trim(),
            tw.output.trim(),
            "`{label}` partial output must match the VM"
        );
    }
}

#[test]
fn riscv_matches_the_concurrent_oracles() {
    // Concurrency programs run on the CONCURRENT oracle pair (shared deterministic scheduler). First
    // confirm the tree-walker and VM concurrent drivers agree (so the target is unambiguous), then hold
    // the RISC-V binary's ucontext-coroutine runtime byte-for-byte against them.
    for (label, src) in CONC_CORPUS {
        let tw = interpret_for_ui_sync(src);
        let vm = run_vm_concurrent(src);
        assert_eq!(
            (&tw.lines, tw.error.is_some()),
            (&vm.lines, vm.error.is_some()),
            "concurrent oracle divergence on `{label}`: tw={:?} vm={:?}",
            tw.lines,
            vm.lines
        );
        assert!(
            compile_to_riscv(src).is_ok(),
            "`{label}` is fully supported but the RISC-V backend refused it"
        );
    }
    if !have_toolchain() {
        eprintln!("SKIP riscv concurrency execution: toolchain not installed");
        return;
    }
    for (label, src) in CONC_CORPUS {
        let tw = interpret_for_ui_sync(src);
        let asm = compile_to_riscv(src).unwrap();
        let run = assemble_and_run(&asm, label);
        assert!(run.success, "`{label}` should exit cleanly.\n\nAsm:\n{asm}");
        let rv: Vec<String> = run.stdout.lines().map(|s| s.to_string()).collect();
        assert_eq!(
            rv, tw.lines,
            "`{label}` RISC-V concurrency output != the deterministic scheduler"
        );
    }
}

// ============================================================
//  Hand-built bytecode differential — for ops NO source emits into an AOT program (reversible-computing
//  primitives, the BigInt lane, the narrow-VM list, the observation/region-hint no-ops, and the
//  scheduler-only Chan/Task ops). Their FORWARD semantics are well-defined and the VM executes them, so
//  we prove RISC-V ≡ VM at the BYTECODE level: build a `CompiledProgram`, run it on the VM, lower it, run
//  under qemu, compare. This is why NOTHING is merely "Unreachable" — every op is lowered AND proven.
// ============================================================

fn bc(code: Vec<Op>, constants: Vec<Constant>, register_count: usize) -> CompiledProgram {
    CompiledProgram {
        code,
        constants,
        register_count,
        ..Default::default()
    }
}

/// Every hand-built program: `(label, program, expected output lines)`. Expected is asserted against the
/// VM first (so the target is real), then against the RISC-V binary.
fn bytecode_programs() -> Vec<(&'static str, CompiledProgram)> {
    use Constant::{Int as I, Text as T};
    use Op::*;
    let mut v: Vec<(&'static str, CompiledProgram)> = vec![
        // Reversible-computing primitives (forward direction).
        (
            "rev_add",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    LoadConst { dst: 1, idx: 1 },
                    RevAdd { place: 0, delta: 1 },
                    Show { src: 0 },
                    Halt,
                ],
                vec![I(5), I(3)],
                2,
            ),
        ),
        (
            "rev_sub",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    LoadConst { dst: 1, idx: 1 },
                    RevSub { place: 0, delta: 1 },
                    Show { src: 0 },
                    Halt,
                ],
                vec![I(5), I(3)],
                2,
            ),
        ),
        (
            "rev_xor",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    LoadConst { dst: 1, idx: 1 },
                    RevXor { place: 0, delta: 1 },
                    Show { src: 0 },
                    Halt,
                ],
                vec![I(5), I(3)],
                2,
            ),
        ),
        (
            "rev_neg",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    RevNeg { place: 0 },
                    Show { src: 0 },
                    Halt,
                ],
                vec![I(5)],
                1,
            ),
        ),
        (
            "rev_swap",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    LoadConst { dst: 1, idx: 1 },
                    RevSwap { a: 0, b: 1 },
                    Show { src: 0 },
                    Show { src: 1 },
                    Halt,
                ],
                vec![I(5), I(3)],
                2,
            ),
        ),
        // A range as a value (`Let r be 2 to 5`): the list of its integers, its length and one of them.
        (
            "new_range",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    LoadConst { dst: 1, idx: 1 },
                    NewRange {
                        dst: 2,
                        start: 0,
                        end: 1,
                    },
                    Show { src: 2 },
                    Length {
                        dst: 3,
                        collection: 2,
                    },
                    Show { src: 3 },
                    LoadConst { dst: 4, idx: 2 },
                    Index {
                        dst: 5,
                        collection: 2,
                        index: 4,
                    },
                    Show { src: 5 },
                    Halt,
                ],
                vec![I(2), I(5), I(3)],
                6,
            ),
        ),
        // The BigInt lane (in-i64) — one program per kind.
        (
            "bigint_fromint",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    BigIntWide {
                        kind: 0,
                        dst: 1,
                        a: 0,
                        b: 0,
                    },
                    Show { src: 1 },
                    Halt,
                ],
                vec![I(5)],
                2,
            ),
        ),
        (
            "bigint_add",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    LoadConst { dst: 1, idx: 1 },
                    BigIntWide {
                        kind: 1,
                        dst: 2,
                        a: 0,
                        b: 1,
                    },
                    Show { src: 2 },
                    Halt,
                ],
                vec![I(5), I(3)],
                3,
            ),
        ),
        (
            "bigint_sub",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    LoadConst { dst: 1, idx: 1 },
                    BigIntWide {
                        kind: 2,
                        dst: 2,
                        a: 0,
                        b: 1,
                    },
                    Show { src: 2 },
                    Halt,
                ],
                vec![I(5), I(3)],
                3,
            ),
        ),
        (
            "bigint_cmp",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    LoadConst { dst: 1, idx: 1 },
                    BigIntWide {
                        kind: 3,
                        dst: 2,
                        a: 0,
                        b: 1,
                    },
                    Show { src: 2 },
                    Halt,
                ],
                vec![I(5), I(3)],
                3,
            ),
        ),
        (
            "bigint_low32",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    BigIntWide {
                        kind: 4,
                        dst: 1,
                        a: 0,
                        b: 0,
                    },
                    Show { src: 1 },
                    Halt,
                ],
                vec![I(0x1_0000_0007)],
                2,
            ),
        ),
        (
            "bigint_mul",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    LoadConst { dst: 1, idx: 1 },
                    BigIntWide {
                        kind: 6,
                        dst: 2,
                        a: 0,
                        b: 1,
                    },
                    Show { src: 2 },
                    Halt,
                ],
                vec![I(5), I(3)],
                3,
            ),
        ),
        // Narrow-VM empty list (half-width Seq of Int) + the two no-op hints.
        (
            "newlist_i32",
            bc(
                vec![
                    NewEmptyListI32 { dst: 0 },
                    Length {
                        dst: 1,
                        collection: 0,
                    },
                    Show { src: 1 },
                    Halt,
                ],
                vec![],
                2,
            ),
        ),
        (
            "obs",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    Obs {
                        site: 0,
                        args_start: 0,
                        args_count: 0,
                    },
                    Show { src: 0 },
                    Halt,
                ],
                vec![I(5)],
                1,
            ),
        ),
        (
            "region_guard",
            bc(
                vec![
                    LoadConst { dst: 0, idx: 0 },
                    RegionBoundsGuard {
                        array: 0,
                        bound: 0,
                        iv: 0,
                        add_max: 0,
                        add_min: 0,
                    },
                    Show { src: 0 },
                    Halt,
                ],
                vec![I(5)],
                1,
            ),
        ),
        // ChanClose (scheduler op, no source emits it): close a channel, then continue.
        (
            "chan_close",
            bc(
                vec![
                    ChanNew {
                        dst: 0,
                        cap: -1,
                        elem: ChanElem::Int,
                    },
                    ChanClose { chan: 0 },
                    LoadConst { dst: 1, idx: 0 },
                    Show { src: 1 },
                    Halt,
                ],
                vec![T("done".into())],
                2,
            ),
        ),
    ];
    // CrdtJoin (per-field MAX over a struct) — a two-struct hand-build with an Int field.
    {
        let mut p = bc(
            vec![
                NewStruct {
                    dst: 0,
                    type_name: 0,
                },
                LoadConst { dst: 1, idx: 2 },
                StructInsert {
                    obj: 0,
                    field: 1,
                    value: 1,
                },
                NewStruct {
                    dst: 2,
                    type_name: 0,
                },
                LoadConst { dst: 3, idx: 3 },
                StructInsert {
                    obj: 2,
                    field: 1,
                    value: 3,
                },
                CrdtJoin {
                    target: 0,
                    source: 2,
                },
                GetField {
                    dst: 4,
                    obj: 0,
                    field: 1,
                },
                Show { src: 4 },
                Halt,
            ],
            vec![T("P".into()), T("f".into()), I(5), I(3)],
            5,
        );
        p.struct_types = vec![StructTypeDef {
            name: "P".into(),
            fields: vec![("f".into(), BoundaryType::Int)],
        }];
        v.push(("crdt_join", p));
    }
    // TaskAwait (scheduler op, no source emits it): spawn a worker, await it, then continue. The worker
    // body lives at pc 5 (after main's 5 ops) as function 0.
    {
        let mut p = bc(
            vec![
                // main (logos_main_body): spawn worker, await, print "done"
                SpawnHandle {
                    dst: 0,
                    func: 0,
                    args_start: 1,
                    arg_count: 0,
                },
                TaskAwait { dst: 1, handle: 0 },
                LoadConst { dst: 2, idx: 0 },
                Show { src: 2 },
                Halt,
                // fn 0 (worker): print "w", return
                LoadConst { dst: 0, idx: 1 },
                Show { src: 0 },
                ReturnNothing,
            ],
            vec![T("done".into()), T("w".into())],
            3,
        );
        p.functions = vec![CompiledFunction {
            name: logicaffeine_compile::Symbol::default(),
            entry_pc: 5,
            param_count: 0,
            register_count: 1,
            captures: vec![],
            named_regs: vec![],
            param_kinds: vec![],
            ret_kind: None,
            param_types: vec![],
            return_type: None,
            mutable_param_regs: vec![],
            cow_exempt_regs: vec![],
        }];
        v.push(("task_await", p));
    }
    v
}

#[test]
fn riscv_matches_the_net_oracles() {
    // Networking programs run on the OFFLINE oracle pair (`tw_outcome` + `vm_outcome_net`, both under
    // `set_net_offline`). Confirm they agree, then hold the RISC-V binary's loopback against them.
    for (label, src) in NET_CORPUS {
        let tw = tw_outcome(src);
        let vm = vm_outcome_net(src);
        assert_eq!(
            (tw.output.trim(), tw.error.is_some()),
            (vm.output.trim(), vm.error.is_some()),
            "net oracle divergence on `{label}`: tw={:?} vm={:?}",
            tw.output,
            vm.output
        );
        assert!(
            compile_to_riscv(src).is_ok(),
            "`{label}` is fully supported but the RISC-V backend refused it"
        );
    }
    if !have_toolchain() {
        eprintln!("SKIP riscv net execution: toolchain not installed");
        return;
    }
    for (label, src) in NET_CORPUS {
        let tw = tw_outcome(src);
        let asm = compile_to_riscv(src).unwrap();
        let run = assemble_and_run(&asm, label);
        assert!(run.success, "`{label}` should exit cleanly.\n\nAsm:\n{asm}");
        assert_eq!(
            run.stdout.trim(),
            tw.output.trim(),
            "`{label}` RISC-V net output != the offline oracle"
        );
    }
}

#[test]
fn riscv_matches_vm_on_hand_built_bytecode() {
    let interner = Interner::new();
    let policies = PolicyRegistry::default();
    // First confirm each program compiles to RISC-V (no refusal) and the VM produces defined output.
    let progs = bytecode_programs();
    for (label, prog) in &progs {
        assert!(
            codegen_program_riscv(prog, &policies, &interner).is_ok(),
            "`{label}` (hand-built bytecode) was refused by the RISC-V backend"
        );
    }
    if !have_toolchain() {
        eprintln!("SKIP riscv bytecode differential: toolchain not installed");
        return;
    }
    for (label, prog) in &progs {
        let asm = codegen_program_riscv(prog, &policies, &interner).unwrap();
        let run = assemble_and_run(&asm, label);
        assert!(
            run.success,
            "`{label}` (bytecode) should exit cleanly.\n\nAsm:\n{asm}"
        );
        let rv: Vec<String> = run.stdout.lines().map(|s| s.to_string()).collect();
        // The plain `Vm::run()` REJECTS channel/scheduler ops ("requires the scheduler driver"), so for
        // the two scheduler-only programs the oracle is the documented deterministic scheduler semantics;
        // every other program is cross-checked against the VM directly.
        let expected: Vec<String> = match *label {
            "chan_close" => vec!["done".to_string()],
            "task_await" => vec!["w".to_string(), "done".to_string()],
            _ => {
                let mut vm = Vm::new(prog);
                let _ = vm.run();
                vm.into_lines()
            }
        };
        assert_eq!(rv, expected, "`{label}` RISC-V bytecode output != expected");
    }
}

#[test]
fn riscv_refuses_unsupported_programs() {
    for (label, src) in REFUSE_CORPUS {
        assert!(
            compile_to_riscv(src).is_err(),
            "`{label}` uses a not-yet-lowered op/shape — the RISC-V backend must REFUSE, not miscompile"
        );
    }
}

/// The failure corpus (`failure_all_tiers.rs`) on RISC-V: the same output before the stop, the SAME
/// reason on stderr, exit status 1 — or a normal exit where the program finishes.
#[test]
fn the_failure_corpus_stops_with_the_same_reason_on_riscv() {
    if !have_toolchain() {
        eprintln!("SKIP riscv failure corpus: riscv64-linux-gnu-gcc / qemu-riscv64 absent");
        return;
    }
    const FAILURES: &[(&str, &str, &str, &str)] = include!("failure_corpus.in");
    let mut diverged = Vec::new();
    for (name, src, expected_out, reason) in FAILURES {
        let asm = match compile_to_riscv(src) {
            Ok(a) => a,
            Err(e) => {
                diverged.push(format!("{name}: refused: {e:?}\n{src}"));
                continue;
            }
        };
        let run = assemble_and_run(&asm, name);
        let ok = run.stdout.trim() == *expected_out
            && if reason.is_empty() {
                run.code == Some(0)
            } else {
                run.code == Some(1) && run.stderr.trim() == *reason
            };
        if !ok {
            diverged.push(format!(
                "{name}: stdout {:?}, stderr {:?}, status {:?}; expected {expected_out:?}, {reason:?}\n{src}",
                run.stdout.trim(),
                run.stderr.trim(),
                run.code
            ));
        }
    }
    assert!(
        diverged.is_empty(),
        "{} of {} failure programs diverge on RISC-V:\n\n{}",
        diverged.len(),
        FAILURES.len(),
        diverged.join("\n\n")
    );
}

// ============================================================
//  Parity — the VM's bytes, the VM's failure, the VM's ending
// ============================================================

/// Programs the RISC-V tier runs exactly as the VM does: the same standard output byte for byte, the same
/// standard error, the same exit status. Nothing is trimmed. The reference is the VM under the driver the
/// program needs (`vm_reference`), as the tier-parity census judges every backend.
const PARITY_CORPUS: &[(&str, &str)] = &[
    // A standard-library function beside the dozens its module splices in that Main never calls.
    ("stdlib_list_argument", "## Main\nShow sumSeq([1, 2, 3]).\n"),
    ("stdlib_slice_argument", "## Main\nShow sumSeq(items 1 through 2 of [1, 2, 3]).\n"),
    (
        "shared_sequence_append",
        "## Definition\nA Document is Shared and has:\n    a lines, which is a SharedSequence of Text.\n\n\
         ## Main\nLet d be a new Document.\nAppend \"Hello\" to d's lines.\nAppend \"World\" to d's lines.\n\
         Show length of d's lines.\n",
    ),
    // Text through its bytes and back.
    ("text_through_bytes", "## Main\nShow upper(\"hi\").\nShow lower(\"MiXeD\").\nShow text_bytes(\"é\").\n"),
    ("text_from_valid_bytes", "## Main\nShow text_from_bytes([104, 195, 169, 240, 159, 152, 128]).\n"),
    ("text_from_a_stray_continuation", "## Main\nShow 1.\nShow text_from_bytes([104, 128, 105]).\nShow 2.\n"),
    ("text_from_a_surrogate", "## Main\nShow text_from_bytes([97, 237, 160, 128]).\n"),
    ("text_from_a_short_four_byte_sequence", "## Main\nShow text_from_bytes([240, 144, 65]).\n"),
    ("text_from_a_broken_fourth_byte", "## Main\nShow text_from_bytes([240, 144, 128, 65]).\n"),
    ("text_from_an_unfinished_sequence", "## Main\nShow text_from_bytes([104, 226, 130]).\n"),
    ("text_from_a_byte_no_sequence_starts_with", "## Main\nShow text_from_bytes([248, 128]).\n"),
    // Constructors matched by name: a declared enum, and the anonymous `Option` and `Result`.
    (
        "result_constructed_in_a_function",
        "## To f (n: Int) -> Result of Int and Text:\n    If n is greater than 5:\n        \
         Return a new Err with error \"big\".\n    \
         Return a new Ok with value n.\n\n## Main\nInspect f(7):\n    When Ok (v):\n        Show v.\n    \
         When Err (e):\n        \
         Show e.\nInspect f(3):\n    When Ok (v):\n        Show v.\n    When Err (e):\n        Show e.\n",
    ),
    (
        "option_some",
        "## Main\nLet v be some 0.\nInspect v:\n    When Some (n):\n        Show \"some\".\n    When None:\n        \
         Show \"none\".\n",
    ),
    (
        "option_none",
        "## Main\nLet v be none.\nInspect v:\n    When Some (n):\n        Show \"some\".\n    When None:\n        \
         Show \"none\".\n",
    ),
    (
        "option_payload",
        "## To find (n: Int) -> Option of Int:\n    If n is greater than 2:\n        Return some n * 10.\n    \
         Return none.\n\n\
         ## Main\nInspect find(5):\n    When Some (v):\n        Show v.\n    When None:\n        Show \"none\".\n\
         Inspect find(1):\n    When Some (v):\n        Show v.\n    When None:\n        Show \"none\".\n",
    ),
    (
        "declared_enum_payloads",
        "## Definition\nA Shape is either:\n    A Circle with a radius, which is Int.\n    \
         A Rect with a w, which is Int, and a h, which is Int.\n\n\
         ## To area (s: Shape) -> Int:\n    Inspect s:\n        When Circle (r):\n            \
         Return 3 * r * r.\n        \
         When Rect (w, h):\n            Return w * h.\n\n## Main\nShow area(a new Rect with w 4 and h 5).\n\
         Show area(a new Circle with radius 2).\n",
    ),
    // Routes, a redirect and a middleware, answered in process.
    ("route_answers", "## Route GET /hi -> Text:\n    Return \"hello\".\n\n## Main\nShow route(\"GET\", \"/hi\").\n"),
    (
        "redirect_status",
        "## Redirect /old -> /new.\n\n## Route GET /new -> Text:\n    Return \"moved\".\n\n## Main\n\
         Show routeStatus(\"GET\", \"/old\").\n",
    ),
    (
        "middleware_declines",
        "## Before each request (request: Request) -> Option of Response:\n    Return none.\n\n\
         ## Route GET /hi -> Text:\n    Return \"hello\".\n\n## Main\nShow route(\"GET\", \"/hi\").\n",
    ),
    // A `Check` that does not hold stops the program with the VM's sentence, whatever stopped it.
    (
        "check_does_not_hold",
        "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\n\
         A User is admin if the user's role equals \"admin\".\n\n\
         ## Main\nLet u be a new User with role \"guest\".\nShow \"before\".\nCheck that the u is admin.\n\
         Show \"ok\".\n",
    ),
    (
        "check_names_no_predicate",
        "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\n\
         A User is admin if the user's role equals \"admin\".\n\n\
         ## Main\nLet u be a new User with role \"admin\".\nCheck that the u is superuser.\nShow \"ok\".\n",
    ),
    (
        "check_capability_does_not_hold",
        "## Definition\nA User has:\n    a name, which is Text.\nA Doc has:\n    an owner, which is Text.\n\n\
         ## Policy\n\
         A User can edit the Doc if the user's name equals the doc's owner.\n\n## Main\n\
         Let u be a new User with name \"ann\".\n\
         Let d be a new Doc with owner \"bob\".\nCheck that the u can edit the d.\nShow \"ok\".\n",
    ),
    (
        "check_capability_holds",
        "## Definition\nA User has:\n    a name, which is Text.\nA Doc has:\n    an owner, which is Text.\n\n\
         ## Policy\n\
         A User can edit the Doc if the user's name equals the doc's owner.\n\n## Main\n\
         Let u be a new User with name \"ann\".\n\
         Let d be a new Doc with owner \"ann\".\nCheck that the u can edit the d.\nShow \"ok\".\n",
    ),
    // Messages through the offline inbox keep their kind; an `Await` needs a `Listen` first.
    (
        "await_a_text_message",
        "## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend \"hello\" to peer.\n\
         Await response from peer into got.\nShow got.\n",
    ),
    (
        "await_a_bool_message",
        "## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend true to peer.\n\
         Await response from peer into got.\nShow got.\n",
    ),
    (
        "await_a_real_message",
        "## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend 2.5 to peer.\n\
         Await response from peer into got.\nShow got.\n",
    ),
    (
        "await_before_any_listen",
        "## Main\nSpawn a Worker called \"w1\".\nSend 7 to \"w1\".\nAwait response from \"w1\" into reply.\n\
         Show reply.\n",
    ),
    (
        "await_messages_in_order",
        "## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend \"a\" to peer.\nSend \"b\" to peer.\n\
         Await response from peer into x.\nAwait response from peer into y.\nShow x + y.\n",
    ),
    (
        "a_message_from_another_sender_is_not_awaited",
        "## Main\nListen on \"a\".\nSend 5 to \"b\".\nListen on \"b\".\nSend 7 to \"b\".\n\
         Await response from \"b\" into x.\n\
         Show x.\n",
    ),
    (
        "a_peer_names_its_topic",
        "## Main\nListen on \" me \".\nLet peer be a PeerAgent at \"me \".\nShow peer.\nShow \"{peer}\".\n\
         Send 5 to \"me\".\n\
         Await response from peer into got.\nShow got.\n",
    ),
    (
        "a_multiaddr_names_its_url",
        "## Main\nListen on \"/ip4/127.0.0.1/tcp/9944\".\nLet peer be a PeerAgent at \"ws://127.0.0.1:9944\".\n\
         Show peer.\n\
         Send 3 to \"/ip4/127.0.0.1/tcp/9944/ws\".\nAwait response from peer into got.\nShow got.\n",
    ),
    (
        "a_stream_and_a_message",
        "## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nLet vals be [1, 2, 3].\nStream vals to peer.\n\
         Send [9] to peer.\nAwait response from peer into one.\nAwait stream from peer into all.\nShow one.\n\
         Show all.\n",
    ),
    // Files live in the sandboxed in-memory VFS, which a program run outside a `## Test` does not keep.
    (
        "file_written_then_read",
        "## Main\nWrite \"hello\" to file \"read.txt\".\nRead contents from file \"read.txt\".\nShow contents.\n",
    ),
    ("file_read_before_any_write", "## Main\nShow \"start\".\nRead contents from file \"notes/missing.txt\".\n\
         Show contents.\n"),
    // A copy of a record is its own, all the way down; a field's item or key is written in the record.
    (
        "copy_of_a_record",
        "## A Bag has:\n    an items: Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items [1, 2].\n\
         Let mutable c be copy of b.\nPush 3 to c's items.\nShow length of b's items.\nShow length of c's items.\n",
    ),
    (
        "copy_of_a_shared_sequence",
        "## Definition\nA Doc is Shared and has:\n    a lines, which is a SharedSequence of Text.\n\n## Main\n\
         Let mutable d be a new Doc.\nAppend \"a\" to d's lines.\nLet mutable c be copy of d.\n\
         Append \"b\" to c's lines.\n\
         Show length of d's lines.\nShow length of c's lines.\n",
    ),
    (
        "copy_call_of_a_record",
        "## A Bag has:\n    an items: Seq of Int.\n    a label: Text.\n\n## Main\n\
         Let mutable b be a new Bag with items [1, 2] and label \"x\".\nLet mutable c be copy(b).\n\
         Push 3 to c's items.\n\
         Show length of b's items.\nShow length of c's items.\nShow c's label.\n",
    ),
    (
        "copy_of_nested_lists",
        "## A Grid has:\n    a rows: Seq of Seq of Int.\n\n## Main\n\
         Let mutable g be a new Grid with rows [[1, 2], [3]].\n\
         Let mutable h be copy of g.\nPush [4] to h's rows.\nShow length of g's rows.\nShow length of h's rows.\n",
    ),
    (
        "a_field_item_set",
        "## A Bag has:\n    an items: Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items [1, 2].\n\
         Let c be b.\nSet item 1 of b's items to 9.\nShow b's items.\nShow c's items.\n",
    ),
    (
        "a_field_map_set",
        "## A Reg has:\n    a m: Map of Text to Int.\n\n## Main\n\
         Let mutable r be a new Reg with m a new Map of Text to Int.\n\
         Set r's m[\"k\"] to 4.\nShow item \"k\" of r's m.\n",
    ),
    // A merge changes the target's own value, field by field as the VM's `crdt_merge_field` does.
    (
        "merge_changes_only_the_target",
        "## Definition\nA Tally has:\n    n: Int.\n\n## Main\nLet mutable a be a new Tally with n 1.\nLet c be a.\n\
         Let mutable b be a new Tally with n 5.\nMerge b into a.\nShow a's n.\nShow c's n.\nConverge b into c.\n\
         Show c's n.\n",
    ),
    (
        "merge_takes_the_incoming_value",
        "## Definition\nA Profile is Shared and has:\n    a name, which is Text.\n    a vip, which is Bool.\n    \
         hits: a ConvergentCount.\n\n\
         ## Main\nLet mutable a be a new Profile with name \"ann\" and vip false.\n\
         Let mutable b be a new Profile with name \"bob\" and vip true.\nIncrease a's hits by 1.\n\
         Increase b's hits by 2.\n\
         Let keep be a.\nMerge b into a.\nShow a's name.\nShow a's vip.\nShow a's hits.\nShow keep's hits.\n\
         Increase keep's hits by 10.\nShow a's hits.\nShow keep's hits.\n",
    ),
    (
        "merge_unions_sets_and_maps",
        "## Definition\nA Pool has:\n    a tags: Set of Text.\n    a counts: Map of Text to Int.\n\n## Main\n\
         Let mutable a be a new Pool.\nLet mutable b be a new Pool.\n\
         Add \"x\" to a's tags.\nAdd \"y\" to b's tags.\nSet a's counts[\"k\"] to 1.\nSet b's counts[\"k\"] to 2.\n\
         Set b's counts[\"j\"] to 7.\nMerge b into a.\nShow a's tags.\nShow a's counts.\nConverge b into a.\n\
         Show a's counts.\n",
    ),
    // An index counts from the back when negative; a failing operation stops with the VM's own sentence.
    ("index_from_the_back", "## Main\nLet xs be [10, 20, 30].\nShow item -1 of xs.\nShow item -3 of xs.\n"),
    (
        "set_item_from_the_back",
        "## Main\nLet mutable xs be [10, 20, 30].\nSet item -1 of xs to 99.\nShow xs.\n",
    ),
    ("index_past_the_end", "## Main\nLet xs be [10, 20, 30].\nShow 1.\nShow item 4 of xs.\n"),
    ("index_zero", "## To at (xs: Seq of Int) and (i: Int) -> Int:\n    Return item i of xs.\n\n## Main\n\
         Show at([1], 0).\n"),
    ("index_past_the_front", "## Main\nLet xs be [10, 20, 30].\nShow item -4 of xs.\n"),
    (
        "division_by_zero",
        "## To q (a: Int) and (b: Int) -> Int:\n    Return a / b.\n\n## Main\nShow q(7, 2).\nShow q(7, 0).\n",
    ),
    ("modulo_by_zero", "## To r (a: Int) and (b: Int) -> Int:\n    Return a % b.\n\n## Main\nShow r(7, 0).\n"),
    (
        "real_division_by_zero",
        "## To q (a: Real) and (b: Real) -> Real:\n    Return a / b.\n\n## Main\nShow q(1.5, 0.0).\n",
    ),
    (
        "missing_map_key",
        "## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nShow item \"b\" of m.\n",
    ),
    (
        "missing_int_map_key",
        "## Main\nLet mutable m be a new Map of Int to Int.\nSet item 1 of m to 1.\nShow item 42 of m.\n",
    ),
    ("parse_int_refuses", "## Main\nShow parseInt(\" 12 \").\nShow parseInt(\"12x\").\n"),
    ("parse_int_out_of_range", "## Main\nShow parseInt(\"99999999999999999999\").\n"),
    ("parse_real_refuses", "## Main\nShow parseFloat(\"2.5\").\nShow parseFloat(\"0x10\").\n"),
    ("parse_real_words", "## Main\nShow parseFloat(\"-inf\").\nShow parseFloat(\"1e3\").\nShow parseFloat(\".5\").\n"),
    ("negative_exponent", "## To p (b: Int) and (e: Int) -> Int:\n    Return b ** e.\n\n## Main\nShow p(2, 3).\n\
         Show p(2, -1).\n"),
    // `t = t + v` changes the Text its register holds, never another holder's.
    (
        "text_append_keeps_aliases",
        "## Main\nLet mutable a be \"x\".\nLet b be a.\nSet a to a + \"y\".\nSet a to a + 5.\nSet a to a + true.\n\
         Show a.\nShow b.\nSet a to a + a.\nShow a.\n",
    ),
    (
        "text_append_aliased_mid_loop",
        "## Main\nLet mutable s be \"\".\nLet mutable keep be \"\".\nRepeat for i from 1 to 6:\n    \
         Set s to s + i.\n    \
         If i equals 3:\n        Set keep to s.\nShow keep.\nShow s.\n",
    ),
    // A collection made empty has the kind its declaration gives it.
    ("empty_collections_show", "## Main\nLet s be a new Set of Int.\nShow s.\nLet m be a new Map of Int to Int.\n\
         Show m.\nLet xs be a new Seq of Text.\nShow xs.\nShow length of s.\n"),
    (
        "empty_collections_combine",
        "## Main\nLet a be a new Seq of Int.\nLet b be a new Seq of Int.\nShow a followed by b.\nLet mutable t be 0.\n\
         Repeat for x in a:\n    Set t to t + x.\nShow t.\nLet mutable s be a new Set of Text.\nRemove \"x\" from s.\n\
         Show s contains \"x\".\nLet u be a new Set of Text.\nShow s union u.\n",
    ),
    ("empty_list_index", "## Main\nLet xs be a new Seq of Int.\nShow item 1 of xs.\n"),
    // Tuples are indexed by position, each at its own kind.
    (
        "tuple_items",
        "## Main\nLet t be (11, \"b\", 2.5).\nShow item 1 of t.\nShow item 2 of t.\nShow item 3 of t.\n\
         Show item -1 of t.\n",
    ),
    ("tuple_index_past_the_end", "## Main\nLet t be (1, 2).\nShow item 3 of t.\n"),
    // `not` and conditions read truthiness: zero, empty and nothing are false.
    (
        "truthiness",
        "## Main\nShow not 0.\nShow not 7.\nShow not \"\".\nShow not \"a\".\nShow not 0.0.\n\
         Let xs be a new Seq of Int.\nShow not xs.\n",
    ),
    // `is approximately` compares numbers of either kind, and refuses anything else as the VM does.
    ("approx_ints", "## Main\nShow 3 is approximately 3.\nShow 3 is approximately 3.0000000001.\n\
         Show 3 is approximately 4.\n"),
    ("approx_texts", "## Main\nShow 1.\nShow \"a\" is approximately \"a\".\n"),
    // Equality is structural: enum values, records, lists, sets and maps compare by what they hold.
    (
        "structural_equality",
        "## A P has:\n    an x: Int.\n    a tag: Text.\n\n## Main\nLet a be some 3.\nLet b be some 3.\nLet c be none.\n\
         Show a is equal to b.\nShow a is equal to c.\nShow c is equal to nothing.\nShow a is equal to nothing.\n\
         Let p be a new P with x 1 and tag \"k\".\nLet q be a new P with x 1 and tag \"k\".\nShow p is equal to q.\n\
         Show [1, 2] is equal to [1, 2].\nShow [\"a\"] is equal to [\"b\"].\n",
    ),
    (
        "set_and_map_equality",
        "## Main\nLet mutable s be a new Set of Int.\nAdd 1 to s.\nAdd 2 to s.\nLet mutable t be a new Set of Int.\n\
         Add 2 to t.\n\
         Add 1 to t.\nShow s is equal to t.\nLet mutable m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\n\
         Set item \"b\" of m to 2.\nLet mutable n be a new Map of Text to Int.\nSet item \"b\" of n to 2.\n\
         Set item \"a\" of n to 1.\nShow m is equal to n.\nSet item \"a\" of n to 5.\nShow m is equal to n.\n",
    ),
    // A slice past either end is empty; a pop from an empty list stops the program with the sentence naming
    // the list.
    (
        "slice_past_the_end",
        "## Main\nLet xs be [10, 20, 30].\nShow items 2 through 9 of xs.\nShow items 2 through 3 of xs.\n\
         Let lo be 1 - 1.\nShow items (lo) through 2 of xs.\n",
    ),
    (
        "pop_from_empty",
        "## Main\nLet mutable xs be a new Seq of Int.\nPush 7 to xs.\nPop from xs into a.\nShow a.\n\
         Pop from xs into b.\nShow b.\n",
    ),
    // Arithmetic on `nothing` stops the program with the VM's sentence for that operation; text takes it in.
    (
        "arithmetic_on_nothing",
        "## To small (x: Int) -> Int:\n    If x is greater than 2:\n        Return x.\n\n## Main\n\
         Let v be small(1).\nShow \"v is \" + v.\nShow v + 1.\n",
    ),
    (
        "comparison_with_nothing",
        "## To small (x: Int) -> Int:\n    If x is greater than 2:\n        Return x.\n\n## Main\n\
         Let v be small(1).\nShow 1.\nShow 2 is less than v.\n",
    ),
    // A ConvergentCount only grows.
    (
        "convergent_count_refuses_a_decrease",
        "## Definition\nA Hits is Shared and has:\n    a total, which is a ConvergentCount.\n\n## Main\n\
         Let mutable h be a new Hits.\n\
         Increase h's total by 2.\nShow h's total.\nDecrease h's total by 1.\nShow h's total.\n",
    ),
    // Iterating a Text walks its characters; a map, its `(key, value)` pairs in order.
    (
        "iterate_text_and_map",
        "## Main\nRepeat for c in \"héllo\":\n    Show c.\nLet mutable m be a new Map of Text to Int.\n\
         Set item \"a\" of m to 1.\n\
         Set item \"b\" of m to 2.\nRepeat for (k, v) in m:\n    Show \"{k}={v}\".\n",
    ),
    (
        "empty_literals_combine",
        "## Main\nLet xs be [].\nLet ys be [].\nLet zs be xs followed by ys.\nShow length of zs.\nLet mutable n be 0.\n\
         For each x in xs:\n    Set n to n + 1.\nShow n.\n",
    ),
    // `randomInt` draws within its bounds, and refuses bounds that cross in the VM's words.
    (
        "random_int_draws_within_bounds",
        "## Main\nLet mutable ok be true.\nRepeat for i from 1 to 50:\n    Let r be randomInt(3, 5).\n    \
         If r is less than 3:\n        \
         Set ok to false.\n    If r is greater than 5:\n        Set ok to false.\nShow ok.\nShow randomInt(7, 7).\n",
    ),
    ("random_int_crossed_bounds", "## Main\nShow randomInt(9, 2).\n"),
    // A closure called where it is made, and through a parameter.
    ("closure_called_in_place", "## Main\nLet f be (x: Int) -> x * 2.\nShow f(21).\n"),
    (
        "closure_called_through_a_parameter",
        "## To apply (f: fn(Int) -> Int) and (x: Int):\n    Show f(x).\n\n## Main\nLet double be (n: Int) -> n * 2.\n\
         Call apply with double and 21.\n",
    ),
    // A function that finishes without returning a value answers `nothing`, through every caller that
    // returns what it answered: an `Option` it declares matches the `None` arm, and the value shows as
    // `nothing`, takes part in text as it, and equals another `nothing`.
    (
        "a_function_that_finishes_without_a_value_answers_nothing",
        "## To maybe (x: Int) -> Option of Int:\n    If x is greater than 2:\n        Return some x.\n\n\
         ## To firstBig (xs: Seq of Int) -> Int:\n    Repeat for x in xs:\n        \
         If x is greater than 10:\n            \
         Return x.\n\n## To relay (xs: Seq of Int) -> Int:\n    Return firstBig(xs).\n\n## Main\n\
         Inspect maybe(1):\n    When Some (s):\n        Show s.\n    When None:\n        Show \"none\".\n\
         Inspect maybe(5):\n    When Some (s):\n        Show s.\n    When None:\n        Show \"none\".\n\
         Show relay([1, 20, 3]).\nShow relay([1, 2, 3]).\nShow \"got \" + relay([4]).\n\
         Let v be relay([5]).\nShow v is equal to relay([6]).\n",
    ),
    (
        "a_closure_answers_what_its_body_answers",
        "## To small (x: Int) -> Int:\n    If x is greater than 2:\n        Return x.\n\n## Main\n\
         Let f be (n: Int) -> small(n).\nShow f(5).\nShow f(1).\nShow \"got \" + f(0).\n",
    ),
    // A pop from an empty list stops the program with the sentence naming the list, after what was shown,
    // whatever the popped value would have fed.
    (
        "a_pop_past_the_last_item_stops_a_running_sum",
        "## Main\nLet mutable xs be [5].\nLet mutable t be 10.\nPop from xs into v.\nSet t to t + v.\nShow t.\n\
         Pop from xs into w.\nSet t to t + w.\nShow t.\n",
    ),
    (
        "a_pop_from_an_empty_list_stops_after_what_was_shown",
        "## Main\nLet mutable xs be a new Seq of Int.\nShow \"before\".\nPop from xs into v.\nShow v - 1.\n",
    ),
    (
        "a_pop_from_an_empty_list_stops_before_a_comparison",
        "## Main\nLet mutable xs be a new Seq of Int.\nPop from xs into v.\nShow v is less than 3.\n",
    ),
    // A call whose `nothing` the very next op fails on stops the program in that op's words.
    (
        "arithmetic_on_a_returned_nothing",
        "## To small (x: Int) -> Int:\n    If x is greater than 2:\n        Return x.\n\n## Main\nShow small(3) + 1.\n\
         Show small(1) + 1.\n",
    ),
    // The standard library's environment lookup answers `nothing` for a variable the program cannot see, and
    // the cookie helpers built on it read it so.
    (
        "an_unset_environment_variable_is_none",
        "## Main\nInspect get(\"LOGOS_PARITY_UNSET_VARIABLE\"):\n    When Some (s):\n        Show s.\n    \
         When None:\n        \
         Show \"none\".\nShow cookie(\"sid\", \"ada\").\n",
    ),
    // A collection nothing is ever put into holds no element: a loop over it never runs its body, whatever the
    // body would do with an element, and an item read from it fails as it fails on any empty collection. What
    // the body would have set keeps the value it had.
    (
        "a_loop_over_an_empty_literal_never_runs",
        "## Main\nLet xs be [].\nLet mutable total be 0.\nLet mutable last be 7.\nFor each x in xs:\n    \
         Set total to total + x.\n    Set last to x.\nShow total.\nShow last.\nLet pairs be [].\n\
         Repeat for (a, b) in pairs:\n    Show a + b.\nShow \"done\".\n",
    ),
    (
        "a_nested_loop_over_an_empty_literal_never_runs",
        "## Main\nLet rows be [].\nLet mutable n be 0.\nFor each row in rows:\n    For each x in row:\n        \
         Set n to n + x.\nShow n.\n",
    ),
    ("an_item_of_an_empty_literal", "## Main\nLet xs be [].\nLet k be 1.\nShow \"before\".\nShow item k of xs.\n"),
    // A collection is known to have no item only where it has none on every path, and only until another
    // collection takes its place.
    (
        "a_literal_replaced_on_one_path",
        "## Main\nLet ys be [5].\nLet mutable xs be [].\nIf randomInt(1, 1) equals 1:\n    Set xs to ys.\nFor each x in xs:\n    \
         Show x.\nShow length of xs.\n",
    ),
    (
        "a_literal_empty_on_one_path_only",
        "## Main\nLet mutable xs be [5].\nLet n be 3.\nIf n is greater than 5:\n    Set xs to [].\n\
         For each x in xs:\n    \
         Show \"item\".\nShow length of xs.\n",
    ),
    // `rotl` rotates a Word32 left by its count modulo 32, as `rotr` rotates it right; the standard library's
    // SHA-256 and HMAC are built on them.
    (
        "rotl_rotates_a_word_left",
        "## Main\nLet w be word32(2147483649).\nShow rotl(w, 1).\nShow rotl(w, 0).\nShow rotl(w, 33).\n\
         Show rotl(w, -1).\n\
         Show rotr(rotl(w, 5), 5).\nShow \"w: \" + rotl(w, 3).\n",
    ),
    (
        "sha256_and_hmac_digests",
        "## Main\nShow sha256(text_bytes(\"abc\")).\n\
         Show hmacSha256(text_bytes(\"key\"), text_bytes(\"The quick brown fox jumps over the lazy dog\")).\n",
    ),
];

/// How the VM runs a program: straight through, under the cooperative scheduler, or under the offline
/// networking host.
enum VmDriver {
    Plain,
    Scheduler,
    Networking,
}

fn vm_driver(src: &str) -> VmDriver {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) =
            parsed.unwrap_or_else(|e| panic!("a parity program does not parse: {e}"));
        if logicaffeine_compile::concurrency::uses_scheduler(stmts) {
            VmDriver::Scheduler
        } else if logicaffeine_compile::interpreter::needs_async(stmts, interner) {
            VmDriver::Networking
        } else {
            VmDriver::Plain
        }
    })
}

/// The VM's run of `src`: every byte it writes to standard output, and the failure it stops with. The
/// clock and the networking mode are the VM's per-thread knobs; each run sets both, so a row's outcome
/// never depends on which thread runs it or on what that thread ran before.
fn vm_reference(src: &str) -> (String, Option<String>) {
    logicaffeine_compile::semantics::temporal::set_fixed_clock(FIXED_DAYS as i32, FIXED_NANOS);
    logicaffeine_compile::concurrency::set_net_offline(true);
    let lines = |lines: Vec<String>| lines.into_iter().map(|l| l + "\n").collect::<String>();
    match vm_driver(src) {
        VmDriver::Plain => {
            let r = vm_outcome(src);
            (r.output, r.error)
        }
        VmDriver::Scheduler => {
            let r = run_vm_concurrent(src);
            (lines(r.lines), r.error)
        }
        VmDriver::Networking => {
            let r =
                futures::executor::block_on(logicaffeine_compile::ui_bridge::run_vm_net_async(src));
            (lines(r.lines), r.error)
        }
    }
}

/// Why `src` does not run on RISC-V as it runs on the VM — the tier refuses it, the VM cannot run it, or the
/// two runs differ — or `None` when they agree. Without the toolchain only the refusal is judged.
fn parity_failure(label: &str, src: &str, toolchain: bool) -> Option<String> {
    let asm = match compile_to_riscv(src) {
        Ok(asm) => asm,
        Err(e) => return Some(format!("the RISC-V tier refuses it: {e:?}")),
    };
    if !toolchain {
        return None;
    }
    let (stdout, failure) = match std::panic::catch_unwind(|| vm_reference(src)) {
        Ok(reference) => reference,
        Err(panic) => {
            let why = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()));
            return Some(format!(
                "the VM panics running it: {}",
                why.unwrap_or_default()
            ));
        }
    };
    let run = assemble_and_run(&asm, label);
    let stderr = failure
        .as_ref()
        .map(|why| format!("{why}\n"))
        .unwrap_or_default();
    let code = if failure.is_some() { 1 } else { 0 };
    (run.stdout != stdout || run.stderr != stderr || run.code != Some(code)).then(|| {
        format!(
            "RISC-V wrote {:?} / {:?} and exited {:?}; the VM wrote {stdout:?} / {stderr:?} and exited {code}",
            run.stdout, run.stderr, run.code
        )
    })
}

/// How many parity programs run at once: each row is a VM run and a `qemu-riscv64` run of its own.
const PARITY_WORKERS: usize = 4;

#[test]
fn riscv_runs_the_parity_corpus_as_the_vm_does() {
    let toolchain = have_toolchain();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let verdicts: Vec<std::sync::Mutex<Option<String>>> = PARITY_CORPUS
        .iter()
        .map(|_| std::sync::Mutex::new(None))
        .collect();
    std::thread::scope(|scope| {
        for _ in 0..PARITY_WORKERS {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((label, src)) = PARITY_CORPUS.get(i) else {
                    break;
                };
                let verdict =
                    parity_failure(label, src, toolchain).map(|why| format!("{label}: {why}"));
                *verdicts[i].lock().unwrap() = verdict;
            });
        }
    });
    let failed: Vec<String> = verdicts
        .into_iter()
        .filter_map(|v| v.into_inner().unwrap())
        .collect();
    assert!(
        failed.is_empty(),
        "{} parity programs do not run on RISC-V as they run on the VM:\n  {}",
        failed.len(),
        failed.join("\n  ")
    );
}

// ============================================================
//  Ownership — a change reaches only the holder it was made through, and happens in place when it can
// ============================================================

/// Programs that share a collection — an alias, a call, a global, a struct field, an iterator, a closure —
/// and then change it through one holder. The RISC-V tier mutates a collection in place only while the
/// register is its only holder (its owned flag), so each must print what the tree-walker prints.
const OWNERSHIP_CORPUS: &[(&str, &str)] = &[
    // A push through an alias leaves the original.
    (
        "alias_push",
        "## Main\n    Let mutable xs be [1, 2].\n    Let mutable ys be xs.\n    Push 3 to ys.\n    \
         Push 4 to xs.\n    Show xs.\n    Show ys.\n",
    ),
    // An indexed write through either holder.
    (
        "alias_setindex",
        "## Main\n    Let mutable xs be [1, 2, 3].\n    Let mutable ys be xs.\n    \
         Set item 1 of ys to 9.\n    Show xs.\n    Show ys.\n    Set item 2 of xs to 8.\n    Show xs.\n    \
         Show ys.\n",
    ),
    // A pop through either holder.
    (
        "alias_pop",
        "## Main\n    Let mutable xs be [1, 2, 3].\n    Let mutable ys be xs.\n    Pop from ys into v.\n    \
         Show v.\n    Show xs.\n    Show ys.\n    Pop from xs into w.\n    Show w.\n    Show xs.\n    \
         Show ys.\n",
    ),
    // A map insert through an alias.
    (
        "alias_map",
        "## Main\n    Let mutable m be a new Map of Text to Int.\n    Set item \"a\" of m to 1.\n    \
         Let mutable n be m.\n    Set item \"a\" of n to 2.\n    Set item \"b\" of m to 3.\n    \
         Show m.\n    Show n.\n",
    ),
    // A set add and remove through either holder.
    (
        "alias_set",
        "## Main\n    Let mutable s be a new Set of Int.\n    Add 1 to s.\n    Add 2 to s.\n    \
         Let mutable t be s.\n    Add 3 to t.\n    Remove 1 from s.\n    Show s.\n    Show t.\n    \
         Remove 2 from t.\n    Show s.\n    Show t.\n",
    ),
    // A field write through either holder of a struct.
    (
        "alias_struct",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\n\
         Let mutable p be a new Point with x 10 and y 20.\nLet mutable q be p.\nSet q's x to 5.\n\
         Show p's x.\nShow q's x.\nSet p's y to 7.\nShow p.\nShow q.\n",
    ),
    // A callee's loop of pushes to its parameter; the caller's list is unchanged and still its own.
    (
        "param_loop",
        "## To grow (xs: Seq of Int) and (k: Int) -> Seq of Int:\n    Repeat for i from 1 to k:\n        \
         Push i to xs.\n    Return xs.\n## Main\n    Let mutable a be [100].\n    Let b be grow(a, 5).\n    \
         Show a.\n    Show b.\n    Push 7 to a.\n    Show a.\n    Show b.\n",
    ),
    // A callee's writes to its parameter never reach the caller.
    (
        "callee_does_not_reach_caller",
        "## To first (xs: Seq of Int) -> Int:\n    Push 100 to xs.\n    Set item 1 of xs to 0.\n    \
         Return item 1 of xs.\n## Main\n    Let mutable a be [5].\n    Show first(a).\n    \
         Push 6 to a.\n    Show a.\n",
    ),
    // A push to the owned list being iterated: the iterator keeps the list it started with.
    (
        "iterate_while_pushing",
        "## Main\n    Let mutable xs be [1, 2, 3].\n    Push 4 to xs.\n    Repeat for x in xs:\n        \
         Push x * 10 to xs.\n    Show xs.\n",
    ),
    // A closure captures the list as it was when the closure was made.
    (
        "closure_snapshot",
        "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n## Main\n    \
         Let mutable xs be [1, 2, 3].\n    Let f be (n: Int) -> n + length of xs.\n    Push 4 to xs.\n    \
         Show apply(f, 0).\n    Show length of xs.\n",
    ),
    // A Main binding read in a function is shared with it.
    (
        "global_alias",
        "## To bump () -> Seq of Int:\n    Let mutable ys be xs.\n    Push 9 to ys.\n    Return ys.\n\
         ## Main\nLet mutable xs be [1, 2].\nLet zs be bump().\nShow xs.\nShow zs.\nPush 3 to xs.\n\
         Show xs.\nShow zs.\n",
    ),
    // A list read out of a struct field is the field's list, shared.
    (
        "struct_field_list",
        "## A Bag has:\n    A items: Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items [1, 2].\n\
         Let mutable xs be b's items.\nPush 3 to xs.\nShow length of b's items.\nShow length of xs.\n",
    ),
    // `copy of` makes a collection its register owns.
    (
        "copy_then_mutate",
        "## Main\n    Let mutable xs be [1, 2].\n    Let mutable ys be copy of xs.\n    Push 3 to ys.\n    \
         Push 4 to xs.\n    Show xs.\n    Show ys.\n",
    ),
    // An alias made on one loop iteration only.
    (
        "alias_on_one_path",
        "## Main\n    Let mutable xs be a new Seq of Int.\n    Let mutable keep be a new Seq of Int.\n    \
         Repeat for i from 1 to 6:\n        Push i to xs.\n        If i equals 3:\n            \
         Set keep to xs.\n    Show keep.\n    Show xs.\n",
    ),
    // A map aliased mid-loop keeps the entries it had.
    (
        "map_loop_alias",
        "## Main\n    Let mutable m be a new Map of Int to Int.\n    \
         Let mutable snap be a new Map of Int to Int.\n    Repeat for i from 1 to 5:\n        \
         Set item i of m to i * i.\n        If i equals 2:\n            Set snap to m.\n    \
         Show length of snap.\n    Show length of m.\n    Show item 2 of snap.\n",
    ),
    // A list aliased mid-way through a loop of pops.
    (
        "pop_loop_alias",
        "## Main\n    Let mutable xs be [1, 2, 3, 4, 5, 6].\n    Let mutable half be a new Seq of Int.\n    \
         Let mutable t be 0.\n    Repeat for i from 1 to 6:\n        Pop from xs into v.\n        \
         Set t to t + v.\n        If i equals 3:\n            Set half to xs.\n    Show t.\n    \
         Show half.\n    Show xs.\n",
    ),
    // A second call of a function that mutates its parameter starts with the parameter unowned again.
    (
        "second_call",
        "## To bump (xs: Seq of Int) -> Int:\n    Push 1 to xs.\n    Return length of xs.\n## Main\n    \
         Let mutable a be [5].\n    Let mutable b be [6, 7].\n    Show bump(a).\n    Show bump(b).\n    \
         Show a.\n    Show b.\n",
    ),
    // A function's read of a Main binding keeps what it read when Main mutates the binding after.
    (
        "global_snapshot",
        "## To peek () -> Seq of Int:\n    Return xs.\n## Main\nLet mutable xs be [1, 2].\n\
         Let snapshot be peek().\nPush 3 to xs.\nShow snapshot.\nShow xs.\nLet later be peek().\n\
         Show later.\n",
    ),
    // A list a function returns is shared with the caller's alias of it.
    (
        "returned_list",
        "## To make (n: Int) -> Seq of Int:\n    Let mutable out be a new Seq of Int.\n    \
         Repeat for i from 1 to n:\n        Push i to out.\n    Return out.\n## Main\n    \
         Let mutable a be make(3).\n    Let b be a.\n    Push 9 to a.\n    Show a.\n    Show b.\n",
    ),
    // A register that held its own list and is then set to another holder's list no longer owns it.
    (
        "overwritten_by_an_alias",
        "## Main\n    Let mutable a be [1, 2].\n    Let mutable b be a new Seq of Int.\n    \
         Push 7 to b.\n    Set b to a.\n    Push 3 to b.\n    Show a.\n    Show b.\n",
    ),
    // A list passed to a function that returns it is shared with the result.
    (
        "a_returned_argument",
        "## To keep (xs: Seq of Int) -> Seq of Int:\n    Return xs.\n## Main\n    \
         Let mutable a be [1].\n    Let b be keep(a).\n    Push 2 to a.\n    Show a.\n    Show b.\n",
    ),
    // A list its register owns, stored in a struct field, is shared with the field from then on.
    (
        "stored_in_a_field",
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable xs be [1, 2].\nPush 0 to xs.\n\
         Let mutable b be a new Bag with items [9].\nSet b's items to xs.\nPush 3 to xs.\n\
         Show length of b's items.\nShow length of xs.\n",
    ),
    // A list its register owns, stored as a map's value, is shared with the map from then on.
    (
        "stored_in_a_map",
        "## Main\n    Let mutable xs be [1, 2].\n    Push 0 to xs.\n    \
         Let mutable n be a new Map of Int to Seq of Int.\n    Set item 1 of n to xs.\n    \
         Push 3 to xs.\n    Show length of item 1 of n.\n",
    ),
    // A list stored in a field is the field's own value: a push through the record changes neither the
    // register it came from nor another holder of the record, nor a record the first is a field of.
    (
        "field_list_from_a_register",
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet xs be [7].\nLet b be a new Bag with items xs.\n\
         Push 1 to b's items.\nShow xs.\nShow b's items.\n",
    ),
    (
        "field_list_of_an_aliased_record",
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items [7].\nLet c be b.\n\
         Push 1 to b's items.\nShow b's items.\nShow c's items.\n",
    ),
    (
        "field_list_of_a_nested_record",
        "## An Inner has:\n    An items: Seq of Int.\n\n## An Outer has:\n    An inner: Inner.\n\n## Main\n\
         Let mutable o be a new Outer with inner a new Inner with items [7].\nLet p be o.\n\
         Let mutable part be o's inner.\n\
         Push 1 to part's items.\nSet o's inner to part.\nLet oi be o's inner.\nLet pi be p's inner.\n\
         Show oi's items.\n\
         Show pi's items.\nShow part's items.\n",
    ),
    (
        "field_list_pushed_in_a_loop",
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet xs be [7].\n\
         Let mutable b be a new Bag with items xs.\n\
         Repeat for i from 1 to 5:\n    Push i to b's items.\nShow xs.\nShow b's items.\n",
    ),
    (
        "field_list_measured_while_pushed",
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items [7].\n\
         Let snap be b's items.\nWhile length of b's items is less than 5:\n    \
         Push length of b's items to b's items.\n\
         Show snap.\nShow b's items.\n",
    ),
    (
        "field_list_iterated_while_pushed",
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items [1, 2].\n\
         Repeat for x in b's items:\n    Push x * 10 to b's items.\nShow b's items.\n",
    ),
    (
        "field_list_stored_elsewhere",
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items [1].\n\
         Push 2 to b's items.\nLet mutable outer be a new Seq of Seq of Int.\nLet t be b's items.\nPush t to outer.\n\
         Push 3 to b's items.\nLet first be item 1 of outer.\nShow first.\nShow b's items.\n",
    ),
    (
        "field_list_of_a_parameter",
        "## A Bag has:\n    An items: Seq of Int.\n\n## To grow (b: Bag) -> Bag:\n    Let mutable c be b.\n    \
         Push 5 to c's items.\n    Return c.\n\n## Main\nLet mutable b be a new Bag with items [1].\n\
         Let g be grow(b).\n\
         Show b's items.\nShow g's items.\n",
    ),
    (
        "field_list_snapshot_after_a_push",
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items [7].\n\
         Push 1 to b's items.\n\
         Let snap be b's items.\nPush 2 to b's items.\nShow snap.\nShow b's items.\n",
    ),
    (
        "field_list_pushed_then_aliased",
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items [7].\n\
         Push 1 to b's items.\nLet c be b.\nPush 2 to b's items.\nShow b's items.\nShow c's items.\n",
    ),
    (
        "field_list_replaced_by_a_register",
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items [7].\n\
         Push 1 to b's items.\nLet xs be [4].\nSet b's items to xs.\nPush 2 to b's items.\nShow xs.\nShow b's items.\n",
    ),
    (
        "field_list_merged",
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable a be a new Bag with items [1].\n\
         Push 2 to a's items.\nLet mutable b be a new Bag with items [5].\nPush 6 to b's items.\nMerge b into a.\n\
         Push 7 to a's items.\nPush 8 to b's items.\nShow a's items.\nShow b's items.\n",
    ),
    // A counter or a divergent register in a record's field changes only the record it was changed through; a
    // shared sequence is one log every holder of the record appends to, as on the VM.
    (
        "counter_of_an_aliased_record",
        "## Definition\nA Hits is Shared and has:\n    a total, which is a ConvergentCount.\n\n## Main\n\
         Let mutable h be a new Hits.\nIncrease h's total by 2.\nLet c be h.\nIncrease h's total by 3.\n\
         Show h's total.\n\
         Show c's total.\n",
    ),
    (
        "two_counters_of_an_aliased_record",
        "## Definition\nA Hits is Shared and has:\n    a total, which is a ConvergentCount.\n    \
         a misses, which is a ConvergentCount.\n\n## Main\nLet mutable h be a new Hits.\nIncrease h's total by 1.\n\
         Increase h's misses by 1.\nLet c be h.\nIncrease h's total by 2.\nIncrease h's misses by 3.\nShow h's total.\n\
         Show h's misses.\nShow c's total.\nShow c's misses.\n",
    ),
    (
        "divergent_of_an_aliased_record",
        "## Definition\nA Reg is Shared and has:\n    a v, which is a Divergent Int.\n\n## Main\n\
         Let mutable r be a new Reg.\n\
         Resolve r's v to 1.\nLet c be r.\nResolve r's v to 2.\nShow r's v.\nShow c's v.\n",
    ),
    (
        "shared_sequence_of_an_aliased_record",
        "## Definition\nA Doc is Shared and has:\n    a lines, which is a SharedSequence of Text.\n\n## Main\n\
         Let mutable d be a new Doc.\nAppend \"a\" to d's lines.\nLet c be d.\nAppend \"b\" to d's lines.\n\
         Show length of d's lines.\nShow length of c's lines.\n",
    ),
];

/// A function record for a hand-built program: `params` parameters of the given boundary types, a frame of
/// `registers`, `captures` local captures, returning `ret`.
fn hand_fn(
    entry_pc: usize,
    params: Vec<Option<BoundaryType>>,
    registers: usize,
    captures: usize,
    ret: Option<BoundaryType>,
) -> CompiledFunction {
    CompiledFunction {
        name: logicaffeine_compile::Symbol::default(),
        entry_pc,
        param_count: params.len() as u16,
        register_count: registers,
        captures: vec![(logicaffeine_compile::Symbol::default(), None); captures],
        named_regs: vec![],
        param_kinds: vec![],
        ret_kind: None,
        param_types: params,
        return_type: ret,
        mutable_param_regs: vec![],
        cow_exempt_regs: vec![],
    }
}

/// Hand-built programs in which a list's own register (owned: made fresh, then pushed to) is handed
/// straight to a global and to a closure — the compiler routes both through a `Move` into a scratch
/// register, so only bytecode can show that the handing-over itself ends the ownership. Each then pushes to
/// the list and shows what the other holder holds. `(label, program)`; the VM is the oracle. (A call
/// argument needs no such program: the VM clears a call's argument registers, so no program reads one
/// after the call.)
fn ownership_bytecode_programs() -> Vec<(&'static str, CompiledProgram)> {
    use Constant::Int as I;
    use Op::*;
    let seq_int = || Some(BoundaryType::Seq(Box::new(BoundaryType::Int)));
    let mut v = Vec::new();
    {
        // r0 = [1]; global 0 = r0; push 2 to r0; show global 0, r0.
        let mut p = bc(
            vec![
                NewEmptyList { dst: 0 },
                LoadConst { dst: 1, idx: 0 },
                ListPush { list: 0, value: 1 },
                GlobalSet { idx: 0, src: 0 },
                LoadConst { dst: 1, idx: 1 },
                ListPush { list: 0, value: 1 },
                GlobalGet { dst: 2, idx: 0 },
                Show { src: 2 },
                Show { src: 0 },
                Halt,
            ],
            vec![I(1), I(2)],
            3,
        );
        p.globals = vec!["xs".into()];
        v.push(("a_global_store", p));
    }
    {
        // r0 = [1]; r1 = a closure capturing r0 that returns what it captured; push 2 to r0;
        // r3 = run(r1), a function that calls the closure; show r3, r0.
        let mut p = bc(
            vec![
                NewEmptyList { dst: 0 },
                LoadConst { dst: 2, idx: 0 },
                ListPush { list: 0, value: 2 },
                MakeClosure {
                    dst: 1,
                    func: 0,
                    locals_start: 0,
                },
                LoadConst { dst: 2, idx: 1 },
                ListPush { list: 0, value: 2 },
                Call {
                    dst: 3,
                    func: 1,
                    args_start: 1,
                    arg_count: 1,
                },
                Show { src: 3 },
                Show { src: 0 },
                Halt,
                Return { src: 0 },
                CallValue {
                    dst: 1,
                    callee: 0,
                    args_start: 2,
                    arg_count: 0,
                    name_for_err: 0,
                },
                Return { src: 1 },
            ],
            vec![I(1), I(2)],
            4,
        );
        p.functions = vec![
            hand_fn(10, vec![], 2, 1, seq_int()),
            hand_fn(11, vec![None], 3, 0, seq_int()),
        ];
        v.push(("a_closure_capture", p));
    }
    v
}

#[test]
fn a_change_reaches_only_the_holder_it_was_made_through() {
    let interner = Interner::new();
    let policies = PolicyRegistry::default();
    let refused: Vec<String> = OWNERSHIP_CORPUS
        .iter()
        .filter_map(|(label, src)| {
            compile_to_riscv(src)
                .err()
                .map(|e| format!("{label}: the RISC-V tier refuses it: {e:?}"))
        })
        .collect();
    let bytecode = ownership_bytecode_programs();
    for (label, prog) in &bytecode {
        assert!(
            codegen_program_riscv(prog, &policies, &interner).is_ok(),
            "`{label}` should compile to RISC-V"
        );
    }
    if !have_toolchain() {
        assert!(
            refused.is_empty(),
            "{} ownership programs do not compile:\n  {}",
            refused.len(),
            refused.join("\n  ")
        );
        return;
    }
    let mut diverged = refused;
    for (label, src) in OWNERSHIP_CORPUS {
        let tw = tw_outcome_with_args(src, &["prog".to_string()]);
        assert!(
            tw.error.is_none(),
            "`{label}` must run on the tree-walker: {:?}",
            tw.error
        );
        let Ok(asm) = compile_to_riscv(src) else {
            continue;
        };
        let run = assemble_and_run(&asm, label);
        if !run.success || run.stdout.trim() != tw.output.trim() {
            diverged.push(format!(
                "{label}: RISC-V wrote {:?} ({}); the tree-walker wrote {:?}",
                run.stdout,
                run.describe(),
                tw.output
            ));
        }
    }
    assert!(
        diverged.is_empty(),
        "{} ownership programs do not run on RISC-V as they run on the tree-walker:\n  {}",
        diverged.len(),
        diverged.join("\n  ")
    );
    for (label, prog) in &bytecode {
        let mut vm = Vm::new(prog);
        let ran = vm.run();
        assert!(ran.is_ok(), "`{label}` must run on the VM: {ran:?}");
        let expected = vm.into_lines();
        assert_eq!(
            expected,
            vec!["[1]".to_string(), "[1, 2]".to_string()],
            "`{label}`: the VM's own answer"
        );
        let asm = codegen_program_riscv(prog, &policies, &interner).unwrap();
        let run = assemble_and_run(&asm, label);
        assert!(
            run.success,
            "`{label}` should exit cleanly ({})",
            run.describe()
        );
        let rv: Vec<String> = run.stdout.lines().map(str::to_string).collect();
        assert_eq!(rv, expected, "`{label}`: RISC-V output != VM");
    }
}

/// At least one row per op this tier lowers as a mutation of a register's own collection
/// ([`mutates_own_collection`]): a loop of that mutation long enough that copying the collection at each
/// step needs well over [`riscv_runner::RUN_CAP_MB`] (every row died at the cap within two seconds when
/// each mutation copied), while mutating in place needs a few megabytes. `(op, label, program)`.
const LINEAR_MEMORY: &[(&str, &str, &str)] = &[
    (
        "ListPush", "push_loop",
        "## Main\n    Let mutable xs be a new Seq of Int.\n    Repeat for i from 1 to 20000:\n        \
         Push i to xs.\n    Show length of xs.\n    Show item 20000 of xs.\n",
    ),
    (
        "ListPop", "pop_loop",
        "## Main\n    Let mutable xs be a new Seq of Int.\n    Repeat for i from 1 to 20000:\n        \
         Push i to xs.\n    Let mutable t be 0.\n    Repeat for i from 1 to 20000:\n        \
         Pop from xs into v.\n        Set t to t + v.\n    Show t.\n    Show length of xs.\n",
    ),
    (
        "SetIndex", "indexed_write_loop",
        "## Main\n    Let mutable xs be a new Seq of Int.\n    Repeat for i from 1 to 20000:\n        \
         Push 0 to xs.\n    Repeat for i from 1 to 20000:\n        Set item i of xs to i * 3.\n    \
         Show item 1 of xs.\n    Show item 20000 of xs.\n",
    ),
    (
        "SetIndex", "map_update_loop",
        "## Main\n    Let mutable m be a new Map of Int to Int.\n    Repeat for k from 1 to 1000:\n        \
         Set item k of m to 0.\n    Repeat for i from 1 to 60000:\n        \
         Let k be (i % 1000) + 1.\n        Set item k of m to (item k of m) + i.\n    \
         Show item 1 of m.\n    Show item 1000 of m.\n",
    ),
    (
        "SetIndexUnchecked", "proven_indexed_write_loop",
        "## Main\n    Let mutable xs be a new Seq of Int.\n    Repeat for i from 1 to 20000:\n        \
         Push 0 to xs.\n    Let mutable i be 1.\n    While i is at most length of xs:\n        \
         Set item i of xs to i * 3.\n        Set i to i + 1.\n    Show item 20000 of xs.\n",
    ),
    (
        "SetAdd", "set_add_loop",
        "## Main\n    Let mutable s be a new Set of Int.\n    Repeat for i from 1 to 16000:\n        \
         Add i to s.\n    Show length of s.\n",
    ),
    (
        "RemoveFrom", "set_remove_loop",
        "## Main\n    Let mutable s be a new Set of Int.\n    Repeat for i from 1 to 16000:\n        \
         Add i to s.\n    Repeat for i from 1 to 15999:\n        Remove i from s.\n    Show s.\n",
    ),
    (
        "StructInsert", "field_write_loop",
        "## A Tally has:\n    An a: Int.\n    A b: Int.\n    A c: Int.\n    A d: Int.\n    An e: Int.\n    \
         A f: Int.\n    A g: Int.\n    A h: Int.\n\n## Main\n\
         Let mutable t be a new Tally with a 0 and b 0 and c 0 and d 0 and e 0 and f 0 and g 0 and h 0.\n\
         Repeat for i from 1 to 6000000:\n    Set t's a to t's a + 1.\nShow t's a.\n",
    ),
    (
        "ListPushField", "field_push_loop",
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet xs be [7].\n\
         Let mutable b be a new Bag with items xs.\n\
         Repeat for i from 1 to 20000:\n    Push i to b's items.\nShow length of b's items.\nShow length of xs.\n",
    ),
    (
        "ListPushField", "field_push_while_measured",
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items [7].\n\
         While length of b's items is less than 20000:\n    Push length of b's items to b's items.\n\
         Show item 20000 of b's items.\n",
    ),
    (
        "CrdtBump", "counter_bump_loop",
        "## Definition\nA Hits is Shared and has:\n    a total, which is a ConvergentCount.\n\n## Main\n\
         Let mutable h be a new Hits.\nRepeat for i from 1 to 6000000:\n    Increase h's total by 1.\n\
         Show h's total.\n",
    ),
    (
        "CrdtResolve", "resolve_loop",
        "## Definition\nA Reg is Shared and has:\n    a v, which is a Divergent Int.\n    a w, which is Int.\n    \
         a x, which is Int.\n    a y, which is Int.\n\n## Main\nLet mutable r be a new Reg with w 0 and x 0 and y 0.\n\
         Repeat for i from 1 to 6000000:\n    Resolve r's v to i.\nShow r's v.\n",
    ),
    (
        "AddAssign", "text_append_loop",
        "## Main\n    Let mutable s be \"\".\n    Repeat for i from 1 to 300000:\n        \
         Set s to s + \"abcdefghij\".\n    Show length of s.\n",
    ),
    (
        "CrdtMerge", "merge_loop",
        "## A Wide has:\n    An a: Int.\n    A b: Int.\n    A c: Int.\n    A d: Int.\n    An e: Int.\n    \
         A f: Int.\n    A g: Int.\n    A h: Int.\n    An i: Int.\n    A j: Int.\n    A k: Int.\n    A l: Int.\n    \
         A m: Int.\n    A n: Int.\n    An o: Int.\n    A p: Int.\n\n## Main\n\
         Let mutable w be a new Wide with a 0 and b 0 and c 0 and d 0 and e 0 and f 0 and g 0 and h 0 and i 0 \
         and j 0 and k 0 and l 0 and m 0 and n 0 and o 0 and p 0.\n\
         Let delta be a new Wide with a 1 and b 0 and c 0 and d 0 and e 0 and f 0 and g 0 and h 0 and i 0 \
         and j 0 and k 0 and l 0 and m 0 and n 0 and o 0 and p 0.\n\
         Repeat for i from 1 to 2000000:\n    Merge delta into w.\nShow w's a.\n",
    ),
    (
        "CrdtJoin", "converge_loop",
        "## A Wide has:\n    An a: Int.\n    A b: Int.\n    A c: Int.\n    A d: Int.\n    An e: Int.\n    \
         A f: Int.\n    A g: Int.\n    A h: Int.\n    An i: Int.\n    A j: Int.\n    A k: Int.\n    A l: Int.\n    \
         A m: Int.\n    A n: Int.\n    An o: Int.\n    A p: Int.\n\n## Main\n\
         Let mutable w be a new Wide with a 0 and b 0 and c 0 and d 0 and e 0 and f 0 and g 0 and h 0 and i 0 \
         and j 0 and k 0 and l 0 and m 0 and n 0 and o 0 and p 0.\n\
         Let delta be a new Wide with a 1 and b 0 and c 0 and d 0 and e 0 and f 0 and g 0 and h 0 and i 0 \
         and j 0 and k 0 and l 0 and m 0 and n 0 and o 0 and p 0.\n\
         Repeat for i from 1 to 2000000:\n    Converge delta into w.\nShow w's a.\n",
    ),
];

#[test]
fn every_owned_mutation_runs_in_linear_memory() {
    let owned: BTreeSet<&str> = Op::all_variants()
        .iter()
        .filter(|o| mutates_own_collection(o))
        .map(op_name)
        .collect();
    let rows: BTreeSet<&str> = LINEAR_MEMORY.iter().map(|(op, _, _)| *op).collect();
    assert_eq!(
        rows, owned,
        "LINEAR_MEMORY needs a row for every op the RISC-V tier lowers as a mutation of a register's own \
         collection, and no other"
    );
    for (op, label, src) in LINEAR_MEMORY {
        let ops =
            program_ops(src).unwrap_or_else(|| panic!("`{label}` failed to compile to bytecode"));
        assert!(
            ops.iter().any(|o| op_name(o) == *op),
            "`{label}` must compile to {op}"
        );
        assert!(
            compile_to_riscv(src).is_ok(),
            "`{label}` should compile to RISC-V"
        );
    }
    if !have_toolchain() {
        return;
    }
    for (op, label, src) in LINEAR_MEMORY {
        let vm = vm_outcome(src);
        assert!(
            vm.error.is_none(),
            "`{label}` must run on the VM: {:?}",
            vm.error
        );
        let run = assemble_and_run(&compile_to_riscv(src).unwrap(), label);
        assert!(
            run.success,
            "`{label}`: a loop of {op} must run within {} MB, as it does when each {op} \
             changes the register's own collection in place ({})",
            riscv_runner::RUN_CAP_MB,
            run.describe()
        );
        assert_eq!(
            run.stdout.trim(),
            vm.output.trim(),
            "`{label}`: RISC-V output != VM"
        );
    }
}

/// Every jump to a stub that stops the program (`.Lfail_<i>`, one a reason, and `.Lnothing`) is a far jump:
/// `jump <stub>, t6`, an `auipc`/`jalr` pair reaching ±2 GiB, behind the check's inverted branch. The stubs sit after
/// the whole program, so a `bnez`/`beqz` (which the assembler widens to a `jal`) or a `j` reaches them only from within
/// 1 MiB of the program's end. The stub never returns, so `t6` is free to carry its address.
#[test]
fn every_jump_to_a_stop_stub_is_a_far_jump() {
    for (label, src) in ERROR_CORPUS {
        let asm = compile_to_riscv(src).unwrap_or_else(|e| panic!("`{label}` compiles: {e:?}"));
        let stubs: Vec<&str> = asm
            .lines()
            .map(str::trim)
            .filter(|l| (l.contains(".Lfail_") || l.contains(".Lnothing")) && !l.ends_with(':'))
            .collect();
        for line in &stubs {
            let far = line
                .strip_prefix("jump ")
                .and_then(|rest| rest.split_once(", "))
                .is_some_and(|(stub, temp)| (stub.starts_with(".Lfail_") || stub == ".Lnothing") && temp == "t6");
            assert!(far, "`{label}`: a jump to a stop stub must be `jump <stub>, t6`, not `{line}`");
        }
    }
    let divides = ERROR_CORPUS.iter().find(|(l, _)| *l == "div_zero").expect("the corpus divides by zero").1;
    assert!(
        compile_to_riscv(divides).unwrap().lines().any(|l| l.trim().starts_with("jump .Lfail_")),
        "a program that divides reaches its stub"
    );
}

/// A program larger than a jump reaches still reaches the stubs that stop it. Its first statement divides by a zero
/// only the run knows (`length of args() - 1`), ahead of 12,000 statements of straight-line code, so the check is more
/// than 1 MiB from the stubs after the program — past a `jal`'s reach, where the link failed with `relocation truncated
/// to fit: R_RISCV_JAL against .Lfail_0` (as Qwen3.5-0.8B's decode step at a 4096-token context did). The program
/// links, and stops where the VM stops, saying what the VM says.
#[test]
fn a_check_stops_a_program_larger_than_a_jump_reaches() {
    if !have_toolchain() {
        return;
    }
    let body = "    Set a to a * 3 + z * 5 - a * 7 + z * 11 - a * 13.\n".repeat(12_000);
    let src = format!(
        "## Main\n    Let z be length of args() - 1.\n    Let q be 10 / z.\n    Let a be z + 2.\n{body}    Show a.\n    Show q.\n"
    );
    let vm = vm_outcome(&src);
    let error = vm.error.expect("the VM stops on the zero divisor");
    let asm = compile_to_riscv(&src).expect("LOGOS→RISC-V");
    let dir = std::env::temp_dir().join(format!("riscv_far_stop_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let runtime = logicaffeine_compile::codegen_riscv::runtime(&asm).expect("the runtime it links");
    std::fs::write(dir.join("main.s"), &asm).unwrap();
    std::fs::write(dir.join("logos_rt.c"), &runtime.c).unwrap();
    let object = std::process::Command::new("riscv64-linux-gnu-gcc")
        .args(["-c", "-o"])
        .arg(dir.join("main.o"))
        .arg(dir.join("main.s"))
        .output()
        .expect("assemble");
    assert!(object.status.success(), "the program assembles:\n{}", String::from_utf8_lossy(&object.stderr));
    let size = std::process::Command::new("riscv64-linux-gnu-size").arg(dir.join("main.o")).output().expect("size");
    let text: u64 = String::from_utf8_lossy(&size.stdout)
        .lines()
        .nth(1)
        .and_then(|l| l.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .expect("the object's text size");
    assert!(text > 1 << 20, "the program's code is {text} bytes; past a jal's 1 MiB is what this test holds");
    let bin = dir.join("main");
    let linked = std::process::Command::new("riscv64-linux-gnu-gcc")
        .args(["-O2", "-static", "-o"])
        .arg(&bin)
        .arg(dir.join("main.o"))
        .arg(dir.join("logos_rt.c"))
        .args(&runtime.link_args)
        .output()
        .expect("link");
    assert!(linked.status.success(), "the program links:\n{}", String::from_utf8_lossy(&linked.stderr));
    let run = std::process::Command::new("timeout").arg("120").arg("qemu-riscv64").arg(&bin).output().expect("qemu-riscv64");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(!run.status.success() && run.status.code() != Some(124), "the program stops on the zero divisor: {:?}", run.status);
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), vm.output.trim(), "the output before the stop is the VM's");
    assert!(stderr.contains(error.trim()), "the stop says what the VM says ({error}): {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
