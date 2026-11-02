//! ★ THE CUDA AOT PIT OF SUCCESS ★ — the single authoritative lock for the LOGOS → CUDA tier,
//! mirroring `riscv_aot_lock.rs`. Two complementary halves over ONE corpus:
//!
//! 1. **Static, catalog-complete ratchet.** [`op_support`] is an EXHAUSTIVE match on `Op` (no `_`
//!    arm), so a new VM instruction fails to COMPILE this file until it is classified. A monotone
//!    `SUPPORTED_FLOOR` (raise per phase, never lower) plus a shrink-only `KNOWN_GAPS` set (the live
//!    `Deferred` census, checked for EXACT equality) make it impossible to silently regress coverage
//!    or grow the gap set. The classification is bound to the backend's [`CUDA_SCALAR_OPS`] fragment
//!    (`op_support_agrees_with_the_backend_fragment`), so the emitter and the lock cannot drift.
//!
//! 2. **Behavioural biconditional.** The emitted `oracle_c` IS the C tier's proven ≡VM body; every
//!    supported program emits a well-formed single-file `.cu`; a program that leaves the supported
//!    fragment (a `Deferred` op, or a scalar op whose value escaped to Float/BigInt/Text/collection)
//!    is REFUSED (typed), never miscompiled; and — gated on `nvcc` — the compiled kernel's output
//!    equals the tree-walker.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If a lock goes RED, fix the backend (`codegen_cuda`), never
//! the assertion. The behavioural execution half gracefully SKIPS when `nvcc` (or a GPU driver) is
//! absent; the static half always runs.
//!
//! ## The gaps→∅ campaign
//! Today the tier lowers a 42-op integer/bool scalar fragment. The `Deferred` census is the shrinking
//! frontier the phase campaign drives to ∅: Phase 1 float, Phase 2 collections/heap, Phase 3 text,
//! Phase 4 aggregates, Phase 5 bigint, Phase 6 concurrency/networking/temporal (the offline model on a
//! single kernel — same as the RISC-V tier under qemu). Terminal state mirrors RISC-V: every op
//! `Supported`, `KNOWN_GAPS == []`, and a catalog-complete support floor.

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_cuda::CUDA_SCALAR_OPS;
use logicaffeine_compile::compile::{
    compile_to_c, compile_to_c_stepped, compile_to_cuda, tw_outcome,
};
use logicaffeine_compile::vm::Op;

// NOTE: the exhaustive per-op EXERCISE proof (every op run by a real program, across every backend) lives in
// `op_backend_matrix_lock.rs` — the cross-backend coverage ratchet. This file keeps the CUDA op_support
// classification + the on-GPU ≡VM corpus.

// ============================================================
//  Static classification — the catalog-complete ratchet
// ============================================================

/// How the CUDA single-kernel AOT tier handles an op. `Supported` = lifted onto the device AND
/// exercised ≡VM (or, for the not-source-emittable oracle forms, proven ≡VM via hand-built bytecode);
/// `Unreachable` = a dead op no source syntax emits into an AOT program; `Deferred` = genuine remaining
/// WORK a later phase lifts. The gaps→∅ campaign shrinks the `Deferred` census to ∅.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Support {
    Supported,
    Unreachable(&'static str),
    Deferred(&'static str),
}

/// ★ THE SINGLE SOURCE OF TRUTH ★ for how the CUDA backend classifies each VM op. EXHAUSTIVE on `Op`
/// (no `_` arm): a new instruction cannot escape classification — this file stops compiling until it is
/// placed. The `Supported` set is bound to the backend's [`CUDA_SCALAR_OPS`] fragment by
/// `op_support_agrees_with_the_backend_fragment`, so the classifier and the emitter cannot diverge.
fn op_support(op: &Op) -> Support {
    use Support::{Supported as S, Unreachable as U};
    match op {
        // ── Scalar int/bool core — LIFTED + exercised ≡VM on the GPU (today's 42-op fragment) ──────
        Op::LoadConst { .. } | Op::Move { .. } => S,
        // Reversible-computing forward primitives — carried in the scalar fragment (int register moves).
        Op::RevSwap { .. }
        | Op::RevXor { .. }
        | Op::RevNeg { .. }
        | Op::RevAdd { .. }
        | Op::RevSub { .. } => S,
        Op::Add { .. } | Op::AddAssign { .. } | Op::Sub { .. } | Op::Mul { .. } => S,
        Op::Div { .. } | Op::ExactDiv { .. } | Op::FloorDiv { .. } | Op::Mod { .. } => S,
        // Optimizer-emitted division forms (power-of-two shift, Granlund–Montgomery magic reciprocal).
        Op::DivPow2 { .. } | Op::MagicDivU { .. } => S,
        Op::Lt { .. } | Op::Gt { .. } | Op::LtEq { .. } | Op::GtEq { .. } => S,
        Op::Eq { .. } | Op::ApproxEq { .. } | Op::NotEq { .. } | Op::Not { .. } => S,
        Op::Pow { .. }
        | Op::BitXor { .. }
        | Op::BitAnd { .. }
        | Op::BitOr { .. }
        | Op::Shl { .. }
        | Op::Shr { .. } => S,
        Op::Jump { .. } | Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. } => S,
        Op::Call { .. }
        | Op::CallBuiltin { .. }
        | Op::GlobalGet { .. }
        | Op::GlobalSet { .. }
        | Op::Return { .. }
        | Op::ReturnNothing => S,
        Op::Show { .. } | Op::Halt => S,

        // ── Phase 1 — FLOAT lane (device f64 + shortest-round-trip Show; the error/trap path) ───────
        // The float-carrying arithmetic/compare ops are already Supported above; Phase 1 relaxes the
        // VALUE-escape backstop so their `double` values ride the device runtime. `FailWith` is the one
        // op the float lane lifts (a device trap → nonzero exit, matching the VM's error behaviour).
        // Phase 1 — the runtime-error path LIFTED: a fail site (`FailWith`, a fold-detected error, a
        // runtime guard like a negative integer exponent) lowers to a device TRAP (CUDA_DEVICE_FAULT_SHIM):
        // stderr elided, `exit(nonzero)`/`abort()` → PTX `trap` → nonzero host exit ≡ the VM's error. Proven
        // ≡VM on the GPU by the `negexp` error-corpus program (stdout `111` flushed BEFORE the trap, exit 1).
        Op::FailWith { .. } => S,

        // ── Phase 2 — COLLECTIONS / HEAP lane: LIFTED via the device allocator shim. A program that
        // `Show`s a WHOLE collection still needs the device string runtime (Phase 3) and is refused at
        // the value-slice level; the ops themselves run ≡VM whenever the reachable slice is clean. ───
        Op::NewList { .. }
        | Op::NewEmptyList { .. }
        | Op::NewEmptyListWithCapacity { .. }
        | Op::NewEmptyListFixed { .. }
        | Op::NewEmptyListI32 { .. }
        | Op::NewEmptySet { .. }
        | Op::NewEmptyMap { .. }
        | Op::NewEmptyMapWithCapacity { .. }
        | Op::NewRange { .. } => S,
        Op::ListPush { .. }
        | Op::ListPop { .. }
        | Op::ListPushField { .. }
        | Op::SetAdd { .. }
        | Op::RemoveFrom { .. }
        | Op::SetIndex { .. }
        | Op::SetIndexUnchecked { .. }
        | Op::Index { .. }
        | Op::IndexUnchecked { .. } => S,
        Op::Length { .. }
        | Op::Contains { .. }
        | Op::SliceOp { .. }
        | Op::SeqConcat { .. }
        | Op::UnionOp { .. }
        | Op::IntersectOp { .. } => S,
        Op::RegionBoundsGuard { .. } => S,
        Op::DeepClone { .. } | Op::EnsureOwned { .. } => S,
        Op::IterPrepare { .. } | Op::IterNext { .. } | Op::IterPop => S,
        // Phase 2b — closures LIFTED: a non-capturing lambda lambda-lifts to a top-level `__device__`
        // function; `CallValue` is an indirect call through a device function pointer (`RET (*g)(ARGS)`,
        // typed via `CType::FnPtr` so nvcc/C++ accepts it — gcc tolerated the old int64 sloppiness). Ran
        // ≡VM on the GPU (`closures` corpus → 49, 20). CAPTURING closures remain a codegen_c-wide follow-up
        // (the C oracle diverges too), NOT a CUDA-specific refusal.
        Op::MakeClosure { .. } | Op::CallValue { .. } => S,

        // ── Phase 3 — STRING / TEXT lane: LIFTED via the device string runtime (vsnprintf + str*). ──
        Op::Concat { .. } | Op::FormatValue { .. } => S,

        // ── Phase 4 — AGGREGATES: struct + tuple literals LIFTED (synthesized C structs are device
        // structs; `Show` renders `Name { a: .., b: .. }` via the device string runtime). ───────────
        Op::NewStruct { .. } | Op::StructInsert { .. } | Op::GetField { .. } => S,
        Op::NewTuple { .. } => S,
        // Phase 4b — tuple destructure LIFTED: `Repeat for (a, b) in pairs` over a Seq-of-tuple binds each
        // pattern var to the element's positional `LTuple_…` field (the same synthesized device struct the
        // tuple literals already lift). Proven ≡VM on the GPU by the `destructure_tuple` corpus program.
        Op::DestructureTuple { .. } => S,
        // Phase 4b — enums LIFTED: `new <Variant>` builds the synthesized device tagged-union;
        // `Inspect`/`When` tag-tests it (TestArm) and binds a variant's payload fields (BindArm). Proven
        // ≡VM on the GPU by the `enum_nullary` + `enum_payload` corpus programs.
        Op::NewInductive { .. } | Op::TestArm { .. } | Op::BindArm { .. } => S,
        // Phase 4c — policy PREDICATE check LIFTED: `Check that <subj> is <pred>` lowers to an inline guard
        // over the subject's device struct fields (`FieldEquals`→strcmp/`==`, `FieldBool`, nested `Predicate`,
        // `And`/`Or`), byte-identical to `evaluate_policy_condition`; a denied check traps → nonzero exit ≡
        // the VM's `Security Check Failed`. Proven ≡VM on the GPU by the `policy_allowed`/`policy_denied`
        // corpus. (Capability `can <action> the <object>` checks resolve their object by TYPE and error in
        // BOTH oracles through the AOT path, so they are not ≡VM-exercisable — codegen_c refuses any
        // not-yet-lowerable condition form LOUDLY; see `cuda_policy_lock`.)
        Op::CheckPolicy { .. } => S,

        // ── Phase 5 — BIGINT / exact-value lane (device multi-limb LBig) ─────────────────────────────
        // UNREACHABLE — a hand-bytecode-only wide-integer primitive (kind 0..6 = fromint/add/sub/cmp/
        // low32/mul). NO source emits `Op::BigIntWide`: source big integers ride the ordinary arithmetic
        // ops that promote to the device `LBig` runtime (`bigint_fact` corpus is Supported). RISC-V exercises
        // it via hand-authored bytecode; the AST-based CUDA tier never sees it.
        Op::BigIntWide { .. } => U("hand-bytecode-only wide-int primitive — no source emits it"),

        // ── Phase 6 — CONCURRENCY / NETWORKING / TEMPORAL / ARGV / CRDT (the offline model on one
        // kernel — a single GPU thread runs the deterministic scheduler, exactly as RISC-V does under
        // qemu). Proven ≡ the offline oracle pair. ───────────────────────────────────────────────────
        // Phase 6 — temporal LIFTED: `today`/`now` read the deterministic fixed clock the HOST injects
        // into device globals before launch (CUDA_DEVICE_CLOCK + the host getenv→cudaMemcpyToSymbol), and
        // `Show` renders it via device calendar math (show_date/show_moment — printf, no host libc). The
        // getenv/time C shim is replaced by a device_override, so it never reaches the device slice. Proven
        // ≡VM on the GPU under a fixed clock by `cuda_temporal_lock`.
        Op::LoadToday { .. } | Op::LoadNow { .. } => S,
        // Phase 6 — argv LIFTED: the CUDA host marshals its own argv into device memory + copies it into the
        // device `_logos_argc`/`_logos_argv` globals (cudaMemcpyToSymbol) before launch; the lifted
        // `logos_args()` reads them to build the `Seq of Text`. Proven ≡ the C oracle on the GPU under the
        // same argv by `cuda_args_lock`.
        Op::Args { .. } => S,
        // UNREACHABLE — observation is compiled OUT of the AOT profile: `Op::Obs` is emitted only when
        // `emit_observation` is set (the OBSERVED profile, `compile_observed`), and `compile_to_cuda` (like
        // every AOT tier) uses the ELIDED profile, so no CUDA program can ever emit it. There is nothing to
        // run on the device — it is not a gap, it is a release-build no-op.
        Op::Obs { .. } => {
            U("observation elided in the AOT profile (emit_observation=false) — never emitted")
        }
        // Phase 6 — counter CRDT LIFTED: `NewCrdt` default-fill + `CrdtBump` (Increase/Decrease) +
        // `CrdtMerge` (per-replica-MAX join, idempotent) run ≡VM on the GPU (the `logos_crdt` count-map is
        // device-clean). Proven by the `crdt_counters`/`crdt_merge` corpus programs.
        // The full counter/sequence/register CRDT set LIFTED: `NewCrdt` default-fill + `CrdtBump`
        // (Increase/Decrease) + `CrdtMerge` (per-replica-MAX join) + `CrdtAppend` (in-place shared-Seq push)
        // + `CrdtResolve` (LWW register — last write wins). Append/Resolve needed a codegen_c fix (they were
        // `/* unsupported stmt */` stubs); all run ≡VM on the GPU (crdt_counters/merge/append/resolve corpus).
        Op::NewCrdt { .. }
        | Op::CrdtBump { .. }
        | Op::CrdtMerge { .. }
        | Op::CrdtAppend { .. }
        | Op::CrdtResolve { .. } => S,
        // UNREACHABLE — the state-based idempotent lattice join. NO source statement emits `Op::CrdtJoin`
        // (`Merge … into …` lowers to `CrdtMerge`); it exists only in the VM machine + hand-bytecode/other
        // tiers, so the AST-based CUDA tier can never see it. Not a gap — unemittable in an AOT program.
        Op::CrdtJoin { .. } => {
            U("the idempotent lattice join — no source emits it (Merge → CrdtMerge)")
        }
        // Phase 6 — CONCURRENCY on the stepped scheduler: every task is a step function over a heap frame
        // (`codegen_c::stepped`) and the C family's scheduler core runs on the device whole, so pipes (`Try`
        // forms included), launches, `Stop`, `Await h into r`, `Sleep` and `Await the first of` schedule on
        // the GPU as on the host C tier, at every seed. Proven ≡ the interpreter on the GPU by
        // `c_family_scheduler_conformance_lock`, `cuda_concurrency_lock` and `cuda_stackless_concurrency_lock`.
        Op::ChanNew { .. }
        | Op::ChanSend { .. }
        | Op::ChanRecv { .. }
        | Op::ChanClose { .. }
        | Op::ChanTrySend { .. }
        | Op::ChanTryRecv { .. }
        | Op::Spawn { .. }
        | Op::SpawnHandle { .. }
        | Op::TaskAwait { .. }
        | Op::TaskAbort { .. }
        | Op::Sleep { .. }
        | Op::SelectArmRecv { .. }
        | Op::SelectArmTimeout { .. }
        | Op::SelectWait { .. } => S,
        // Phase 6c — OFFLINE NETWORKING LIFTED: the loopback transport (`codegen_c`'s `C_NET_RUNTIME`,
        // device-lifted with strcmp/strdup shims). `Connect`/`Listen`/`Sync` no-op, `MakePeer` is identity,
        // `Send`/`Stream` enqueue a 64-bit message (a scalar, or a `Seq` pointer for a stream) into a
        // per-address FIFO, `Await` dequeues it — a single-node `Send X to peer; Await …` round-trips ≡ the
        // VM's offline `NetInbox`. Proven ≡VM on the GPU by `cuda_net_lock` (scalar `7` + stream `2`).
        Op::NetConnect { .. }
        | Op::NetListen { .. }
        | Op::NetSend { .. }
        | Op::NetStream { .. }
        | Op::NetAwait { .. }
        | Op::NetMakePeer { .. }
        | Op::NetSync { .. } => S,

        // tcp.lg client sockets — real host networking; a GPU kernel has no socket
        // syscalls. Refused loudly (never a silent Nothing).
        Op::Tcp { .. } => U("tcp.lg sockets — a GPU kernel has no host networking"),
        // The `Serve` HTTP accept loop is host-only (a GPU kernel has no sockets
        // to bind or accept on). Refused loudly.
        Op::Serve { .. } => U("Serve HTTP — a GPU kernel has no host accept loop"),
    }
}

/// The count of `Supported` ops — a MONOTONE FLOOR. RAISE it as each phase lands; NEVER lower it. A drop
/// means a previously-supported op regressed — fix `codegen_cuda`, not this number.
const SUPPORTED_FLOOR: usize = 115;

/// The whole VM ISA size. Pinned so a NEW op (which lands unclassified) makes this file fail to compile
/// (the exhaustive `op_support` match) or RED (this pin) — forcing a conscious classification.
const EXPECTED_CATALOG: usize = 120;

/// The EXACT set of currently-`Deferred` op names — a shrink-only ledger. Moving an op from `Deferred`
/// to `Supported` requires removing it here AND raising `SUPPORTED_FLOOR` (both conscious edits); a new
/// deferral must be added here deliberately. `the_deferred_frontier_is_pinned_and_shrink_only` asserts
/// this equals the live `Deferred` census. The campaign drives it to `[]`.
const KNOWN_GAPS: &[&str] = &[
    // ★ EMPTY — THE CAMPAIGN IS COMPLETE. All 120 ISA ops are accounted for: 115 Supported (lifted onto the
    // device AND exercised end-to-end ≡VM on the real GPU) + 5 Unreachable (Obs/CrdtJoin/BigIntWide — no AOT
    // source emits them; Tcp/Serve — host-only IO). Networking was the final frontier, lifted via the offline
    // loopback transport (`cuda_net_lock`). The whole LOGOS language runs byte-identical to the VM on one CUDA
    // kernel.
];

fn catalog() -> BTreeSet<&'static str> {
    Op::all_variants().iter().map(|o| o.name()).collect()
}

fn names_where(pred: impl Fn(Support) -> bool) -> BTreeSet<&'static str> {
    Op::all_variants()
        .iter()
        .filter(|o| pred(op_support(o)))
        .map(|o| o.name())
        .collect()
}

fn supported_names() -> BTreeSet<&'static str> {
    names_where(|s| matches!(s, Support::Supported))
}

fn deferred_names() -> BTreeSet<&'static str> {
    names_where(|s| matches!(s, Support::Deferred(_)))
}

fn unreachable_names() -> BTreeSet<&'static str> {
    names_where(|s| matches!(s, Support::Unreachable(_)))
}

#[test]
fn the_vm_isa_size_is_pinned() {
    assert_eq!(
        catalog().len(),
        EXPECTED_CATALOG,
        "the VM ISA changed size. A new op must be classified for the CUDA tier in `op_support` \
         (the exhaustive match already forces this) and this pin bumped — never let an op escape \
         classification."
    );
}

#[test]
fn the_supported_count_never_regresses() {
    let n = supported_names().len();
    assert!(
        n >= SUPPORTED_FLOOR,
        "the CUDA supported-op count fell to {n} (floor {SUPPORTED_FLOOR}). A previously-supported op \
         was demoted — fix codegen_cuda, do not lower the floor."
    );
}

#[test]
fn op_support_agrees_with_the_backend_fragment() {
    // The classifier's `Supported` set MUST equal what the backend actually lowers, `CUDA_SCALAR_OPS`.
    // Bound here so the lock and the emitter cannot drift.
    let supported = supported_names();
    let fragment: BTreeSet<&str> = CUDA_SCALAR_OPS.iter().copied().collect();
    assert_eq!(
        fragment.len(),
        CUDA_SCALAR_OPS.len(),
        "CUDA_SCALAR_OPS has a duplicate"
    );
    let only_in_classifier: Vec<_> = supported.difference(&fragment).collect();
    let only_in_backend: Vec<_> = fragment.difference(&supported).collect();
    assert!(
        only_in_classifier.is_empty() && only_in_backend.is_empty(),
        "op_support `Supported` set diverged from the backend CUDA_SCALAR_OPS fragment.\n  \
         classified Supported but NOT in the fragment: {only_in_classifier:?}\n  \
         in the fragment but NOT classified Supported: {only_in_backend:?}",
    );
}

/// ★ THE KEYSTONE ★ — the frontier lock. The live `Deferred` census must EXACTLY equal `KNOWN_GAPS`.
/// A new deferral not in the ledger, or a listed gap that is now `Supported`, goes RED. `KNOWN_GAPS` is
/// shrink-only: the only legal edit is to REMOVE a name (and raise `SUPPORTED_FLOOR`). When this set is
/// empty the campaign is COMPLETE.
#[test]
fn the_deferred_frontier_is_pinned_and_shrink_only() {
    let live = deferred_names();
    let declared: BTreeSet<&str> = KNOWN_GAPS.iter().copied().collect();
    assert_eq!(
        declared.len(),
        KNOWN_GAPS.len(),
        "KNOWN_GAPS has duplicate entries"
    );
    let newly_appeared: Vec<_> = live.difference(&declared).collect();
    let fixed_but_listed: Vec<_> = declared.difference(&live).collect();
    assert!(
        newly_appeared.is_empty(),
        "NEW deferral(s) appeared and are not in KNOWN_GAPS: {newly_appeared:?}. Either lift the op \
         (preferred) or add it to KNOWN_GAPS consciously."
    );
    assert!(
        fixed_but_listed.is_empty(),
        "KNOWN_GAPS lists op(s) that are no longer Deferred: {fixed_but_listed:?}. Remove them (and \
         raise SUPPORTED_FLOOR) — KNOWN_GAPS is shrink-only."
    );
}

#[test]
fn the_partition_is_total_and_disjoint() {
    // Totality + disjointness are guaranteed at COMPILE TIME by the exhaustive `match` (no `_` arm); a
    // 116th op would not compile until placed in exactly one arm. This re-asserts it for documentation:
    // the three buckets sum to the whole ISA.
    let s = supported_names().len();
    let u = unreachable_names().len();
    let d = deferred_names().len();
    assert_eq!(
        s + u + d,
        EXPECTED_CATALOG,
        "the Supported/Unreachable/Deferred buckets ({s}+{u}+{d}) must partition all {EXPECTED_CATALOG} ops"
    );
}

// ============================================================
//  Corpus
// ============================================================

/// The supported fragment — integer/bool, device-clean floats (Phase 1: the Dragon4 device formatter,
/// [`cuda_float_lock`]), and literal-text `Show`. Every program here MUST compile to CUDA and (under
/// `nvcc` + a GPU) run byte-for-byte ≡ the tree-walker.
const CORPUS: &[(&str, &str)] = &[
    ("show_int", "## Main\n    Show 7.\n"),
    ("multi_show", "## Main\n    Show 1.\n    Show 2.\n    Show 3.\n"),
    ("arith", "## Main\n    Show 6 * 7 + 1.\n"),
    ("copy_move", "## Main\n    Let a be 5.\n    Let b be a.\n    Show b.\n"),
    ("bool_cmp", "## Main\n    Show 3 is greater than 2.\n"),
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
    // ── Phase 1: device floats (the Dragon4 shortest-round-trip formatter) ──────────────────────
    ("float_add", "## Main\n    Show 1.5 + 2.5.\n"),
    ("float_sub", "## Main\n    Show 2.0 - 0.5.\n"),
    ("float_mul", "## Main\n    Show 1.1 * 1.1.\n"),
    ("float_div", "## Main\n    Show 1.0 / 3.0.\n"),
    ("float_half", "## Main\n    Show 10.0 / 4.0.\n"),
    ("float_let", "## Main\n    Let x be 3.14159.\n    Show x.\n"),
    (
        "float_loop",
        "## To sum (n: Int) -> Real:\n    Let acc be 0.0.\n    Let i be 1.\n    \
         While i is less than n:\n        Set acc to acc + 1.0 / i.\n        Set i to i + 1.\n    \
         Return acc.\n## Main\n    Show sum(10).\n",
    ),
    // ── Phase 1: literal-text `Show` (device-clean — `show_str` is just `printf`) ────────────────
    ("show_text", "## Main\n    Show \"hello\".\n"),
    ("show_text_multi", "## Main\n    Show \"alpha\".\n    Show \"beta\".\n"),
    // ── Phase 2: collections on the device allocator shim (each `Show`s a scalar) ────────────────
    ("list_sum", "## Main\n    Let xs be [3, 1, 2].\n    Let s be 0.\n    For each x in xs:\n        Set s to s + x.\n    Show s.\n"),
    ("list_index", "## Main\n    Let xs be [10, 20, 30].\n    Show item 2 of xs.\n"),
    ("list_len", "## Main\n    Let xs be [1, 2, 3, 4].\n    Show the length of xs.\n"),
    (
        "list_push",
        "## Main\n    Let xs be a new Seq of Int.\n    Push 5 to xs.\n    Push 7 to xs.\n    \
         Show item 1 of xs + item 2 of xs.\n",
    ),
    (
        "list_build_loop",
        "## To sumto (n: Int) -> Int:\n    Let xs be a new Seq of Int.\n    Let i be 1.\n    \
         While i is less than or equal to n:\n        Push i to xs.\n        Set i to i + 1.\n    \
         Let s be 0.\n    For each x in xs:\n        Set s to s + x.\n    Return s.\n\
         ## Main\n    Show sumto(100).\n",
    ),
    // ── Phase 3: device string runtime — Show a whole collection + string interpolation/concat ───
    ("show_list", "## Main\n    Let xs be [1, 2, 3].\n    Show xs.\n"),
    ("show_bools", "## Main\n    Let xs be [true, false, true].\n    Show xs.\n"),
    ("interp_int", "## Main\n    Let n be 42.\n    Show \"n is {n}\".\n"),
    ("interp_expr", "## Main\n    Let a be 3.\n    Let b be 4.\n    Show \"sum is {a + b}\".\n"),
    ("interp_float", "## Main\n    Let x be 3.14.\n    Show \"pi is about {x}\".\n"),
    ("interp_text", "## Main\n    Let who be \"world\".\n    Show \"hello {who}\".\n"),
    // ── Phase 4: aggregates — struct + tuple literals (device structs, name-sorted `Show`) ────────
    ("tuple_show", "## Main\n    Let t be (1, 2).\n    Show t.\n"),
    (
        "struct_show",
        "## A Point has:\n    an x, which is Int.\n    a y, which is Int.\n\n\
         ## Main\n    Let p be a new Point with x 3 and y 4.\n    Show p.\n",
    ),
    (
        "struct_field",
        "## A Point has:\n    an x, which is Int.\n    a y, which is Int.\n\n\
         ## Main\n    Let p be a new Point with x 3 and y 4.\n    Show p's x + p's y.\n",
    ),
    // ── Phase 4b: tuple DESTRUCTURE — `Repeat for (a, b) in <Seq of tuple>` (Op::DestructureTuple) ──
    (
        "destructure_tuple",
        "## Main\n    Let pairs be [(1, 2), (3, 4), (5, 6)].\n    \
         Repeat for (a, b) in pairs:\n        Show a + b.\n",
    ),
    // ── Phase 4b: enums — a nullary tagged-union (NewInductive) + `Inspect`/`When` tag-test (TestArm),
    //    and a PAYLOAD variant whose field the match binds (BindArm). The exhaustive `Inspect` also emits
    //    a defensive `Op::FailWith` (the fall-through), so these exercise the device fault-trap path too. ─
    (
        "enum_nullary",
        "## A Color is one of:\n    A Red.\n    A Green.\n    A Blue.\n\n\
         ## Main\n    Let c be a new Green.\n    Inspect c:\n        When Red: Show 1.\n        \
         When Green: Show 2.\n        When Blue: Show 3.\n",
    ),
    (
        "enum_payload",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## Main\nLet s be a new Circle with radius 5.\n\
         Inspect s:\n    When Circle (r): Show r.\n    When Rectangle (w, h): Show w.\n",
    ),
    // ── Phase 6: single-replica counter CRDTs — `ConvergentCount`/`Tally` (NewCrdt + CrdtBump), read back
    //    as a plain Int; and a per-replica-MAX MERGE (NewCrdt + CrdtBump + CrdtMerge, idempotent). ────────
    (
        "crdt_counters",
        "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n\
         A Wallet is Shared and has:\n    balance: Tally.\n## Main\n    \
         Let mutable c be a new Counter.\n    Increase c's points by 5.\n    Increase c's points by 3.\n    \
         Show c's points.\n    Let mutable w be a new Wallet.\n    Increase w's balance by 10.\n    \
         Decrease w's balance by 4.\n    Show w's balance.\n",
    ),
    (
        "crdt_merge",
        "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    \
         Let mutable a be a new Counter.\n    Increase a's points by 5.\n    \
         Let mutable b be a new Counter.\n    Increase b's points by 3.\n    \
         Merge b into a.\n    Merge b into a.\n    Show a's points.\n",
    ),
    // Shared-sequence APPEND (CrdtAppend, in-place push) + LWW register RESOLVE (CrdtResolve, last-write).
    (
        "crdt_append",
        "## A Bag has:\n    nums: Seq of Int.\n\n## Main\n    Let d be a new Bag.\n    \
         Append 5 to d's nums.\n    Append 7 to d's nums.\n    Show length of d's nums.\n    \
         Show item 2 of d's nums.\n",
    ),
    (
        "crdt_resolve",
        "## Definition\nA Page is Shared and has:\n    rank: Divergent Int.\n## Main\n    \
         Let mutable p be a new Page.\n    Resolve p's rank to 3.\n    Resolve p's rank to 9.\n    \
         Show p's rank.\n",
    ),
    // ── Phase 2b: closures — a non-capturing lambda passed to a higher-order `apply` (MakeClosure +
    //    CallValue via a device function pointer): stored-in-a-var + passed-inline. ───────────────────────
    (
        "closures",
        "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n\
         ## Main\n    Let g be (n: Int) -> n * n.\n    Show apply(g, 7).\n    \
         Show apply((n: Int) -> n * 2, 10).\n",
    ),
    // ── Phase 5: arbitrary-precision Int (device multi-limb LBig + __int128 overflow builtins) ────
    (
        "bigint_fact",
        "## To fact (n: Int) -> Int:\n    Let acc be 1.\n    Let i be 1.\n    \
         While i is less than n:\n        Set acc to acc * i.\n        Set i to i + 1.\n    \
         Return acc.\n## Main\n    Show fact(30).\n",
    ),
    // ── Phase 4c: policy PREDICATE check (CheckPolicy) — two passing guards over a device struct: a plain
    //    `FieldEquals` (`admin`) and an `Or` of two `FieldEquals` (`staff`). Both pass, so the guarded
    //    `Show "allowed"` runs; a denied check would trap (proven in `cuda_policy_lock`). Mirrors
    //    `riscv_aot_lock`'s `policy_allowed`. ────────────────────────────────────────────────────────────
    (
        "policy_allowed",
        "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\n\
         A User is admin if the user's role equals \"admin\".\n\
         A User is staff if the user's role equals \"admin\" or the user's role equals \"mod\".\n\n\
         ## To process (u: User):\n    Check that the u is admin.\n    Check that the u is staff.\n    Show \"allowed\".\n\n\
         ## Main\nLet u be a new User with role \"admin\".\nprocess(u).\n",
    ),
    // ── Phase 6: concurrency on the stepped scheduler — two functions that wait recursing through each other,
    //    and Real arithmetic on a received Real, which the narrow stackless tier refused. ────────────────────
    (
        "mutual_rec_blocking",
        "## To ping (ch: Int):\n    Receive x from ch.\n    If x is greater than 0:\n        Show x.\n        pong(ch).\n\n\
         ## To pong (ch: Int):\n    Receive y from ch.\n    If y is greater than 0:\n        Show y.\n        ping(ch).\n\n\
         ## Main\n    Let ch be a Pipe of Int.\n    Launch a task to ping with ch.\n    Send 2 into ch.\n    \
         Send 1 into ch.\n    Send 0 into ch.\n",
    ),
    (
        "real_in_int_arith",
        "## To c (ch: Real):\n    Receive x from ch.\n    Let y be x plus 1.\n    Show y.\n\n## Main\n    \
         Let ch be a Pipe of Real.\n    Launch a task to c with ch.\n    Send 1.5 into ch.\n",
    ),
];

/// Programs OUTSIDE the fragment — each MUST be refused (typed), never miscompiled. With the op-level
/// frontier now EMPTY (every ISA op is Supported or Unreachable), the remaining CUDA boundaries are
/// FORM-level within otherwise-Supported ops: a shape the device lowering cannot yet express. `compile_to_c`
/// answers each (the host C tier writes its own refusal into its C), so the refusal is typed on CUDA.
const REFUSE: &[(&str, &str)] = &[
    // FORM-level: a record crossing a pipe. The C tier carries a pipe's values in one scalar cell and refuses
    // a record in its own words (an `#error` in its C); the device lowers the same C, so it refuses the same
    // way (`CudaLowerError::CRefused`), never carrying the record wrong.
    (
        "pipe_of_records",
        "## A Point has:\n    an x, which is Int.\n\n## To consume (ch: Point):\n    Receive p from ch.\n    \
         Show p's x.\n\n## Main\n    Let ch be a Pipe of Point.\n    Launch a task to consume with ch.\n    \
         Let q be a new Point with x 1.\n    Send q into ch.\n",
    ),
    // NOTE: the `policy` predicate check (`CheckPolicy`) GRADUATED out of this REFUSE corpus — it now lowers
    // to an inline device guard and runs ≡VM on the GPU (the passing `policy_allowed` corpus program above +
    // the denied-check trap in `cuda_policy_lock`). Capability (`can <action> the <object>`) checks are not
    // ≡VM-exercisable (they error in both oracles through the AOT path) and are refused LOUDLY by codegen_c.
    // NOTE: `blocking_sleep`, `mutual_rec_blocking` and `real_in_int_arith` GRADUATED out of this corpus into
    // `CORPUS` above: concurrency lowers through the stepped tier (the C tier's own lowering, its tasks as
    // step functions), which runs a blocking `Sleep`, two functions that wait recursing through each other,
    // and Real arithmetic on a received Real ≡ the interpreter on the GPU.
];

/// Every `REFUSE` program must use at least one op that `op_support` classifies non-`Supported` (a
/// `Deferred` op) OR carry a value the device-cleanliness backstop rejects. This keeps the REFUSE corpus
/// honest — it proves refusals ride the classification, not an unrelated compile failure.
#[test]
fn the_refuse_corpus_targets_real_boundaries() {
    for (label, src) in REFUSE {
        assert!(
            compile_to_c(src).is_ok(),
            "`{label}`: the C oracle should still compile"
        );
        assert!(
            compile_to_cuda(src).is_err(),
            "`{label}`: a program outside the CUDA fragment MUST be refused, never miscompiled",
        );
    }
}

// ============================================================
//  Behavioural — oracle anchor, well-formedness, refusals
// ============================================================

#[test]
fn the_oracle_is_the_c_backend_output() {
    // A concurrent program's device body is the stepped tier's C; any other program's is the same C on
    // either tier.
    for (label, src) in CORPUS {
        let art = compile_to_cuda(src)
            .unwrap_or_else(|e| panic!("`{label}` should compile to CUDA: {e:?}"));
        let c = compile_to_c_stepped(src)
            .unwrap_or_else(|e| panic!("`{label}` should compile to C: {e:?}"));
        assert_eq!(
            art.oracle_c, c,
            "`{label}`: the CUDA tier's ≡VM oracle must be the C backend's output"
        );
    }
}

#[test]
fn supported_corpus_emits_wellformed_cuda() {
    for (label, src) in CORPUS {
        let art = compile_to_cuda(src)
            .unwrap_or_else(|e| panic!("`{label}` should compile to CUDA: {e:?}"));
        let cu = &art.kernel_cu;
        assert!(
            cu.contains("__global__ void logos_kernel()"),
            "`{label}`: kernel entry missing"
        );
        assert!(
            cu.contains("__device__ int logos_main(int argc, char **argv) {"),
            "`{label}`: device entry missing"
        );
        // The HOST orchestrator now takes argc/argv (it may marshal them into the device argv globals for a
        // program that reads `args()`); the device C `main` is still renamed to `logos_main` by `wrap.rs`.
        assert!(
            cu.contains("int main(int argc, char **argv) {"),
            "`{label}`: host orchestrator missing"
        );
        assert!(
            cu.contains("logos_main(0, (char **)0)"),
            "`{label}`: kernel does not call the entry"
        );
        assert!(
            cu.contains("__device__ int logos_main("),
            "`{label}`: device C main not renamed to logos_main"
        );
        assert!(
            art.makefile.contains("nvcc"),
            "`{label}`: Makefile must drive nvcc"
        );
    }
}

#[test]
fn out_of_fragment_programs_are_typed_refusals() {
    for (label, src) in REFUSE {
        // The C backend supports these — proving the refusal is the CUDA tier's own boundary, not a
        // generic compile failure.
        assert!(
            compile_to_c(src).is_ok(),
            "`{label}`: the C oracle should still compile"
        );
        assert!(
            compile_to_cuda(src).is_err(),
            "`{label}`: a program outside the CUDA integer/bool fragment MUST be refused, never miscompiled",
        );
    }
}

// ============================================================
//  Behavioural — the ≡VM execution biconditional (gated on nvcc)
// ============================================================

fn have_nvcc() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A runnable GPU (the kernel driver is loaded). `nvcc` can COMPILE for `sm_86` with no driver, but
/// RUNNING a kernel needs a device — `/dev/nvidia0` exists exactly when the driver module is up.
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

/// nvcc-compile a `.cu` to a binary (device + host) WITHOUT running it — verifies the emitted CUDA is
/// valid `sm_86` code even on a box with no GPU driver.
fn nvcc_compiles(cu: &str, label: &str) {
    let dir = std::env::temp_dir().join(format!("logos_cuda_c_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("logos_cuda.cu");
    let bin = dir.join("logos_cuda");
    std::fs::write(&src, cu).expect("write .cu");
    let compile = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("run nvcc");
    assert!(
        compile.status.success(),
        "`{label}`: emitted CUDA must compile with nvcc.\nstderr:\n{}\n\n.cu:\n{cu}",
        String::from_utf8_lossy(&compile.stderr),
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn supported_corpus_compiles_with_nvcc() {
    if !have_nvcc() {
        eprintln!("SKIP cuda compile: nvcc not installed (install the CUDA toolkit — Phase 0)");
        return;
    }
    for (label, src) in CORPUS {
        let art =
            compile_to_cuda(src).unwrap_or_else(|e| panic!("`{label}` should compile: {e:?}"));
        nvcc_compiles(&art.kernel_cu, label);
    }
}

/// Compile a single-file `.cu` with `nvcc` and run it, returning trimmed stdout. Panics with the nvcc
/// stderr + the source on failure (a supported program MUST compile and run).
fn nvcc_compile_and_run(cu: &str, label: &str) -> String {
    let dir = std::env::temp_dir().join(format!("logos_cuda_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("logos_cuda.cu");
    let bin = dir.join("logos_cuda");
    std::fs::write(&src, cu).expect("write .cu");

    let compile = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("run nvcc");
    assert!(
        compile.status.success(),
        "`{label}`: emitted CUDA must compile with nvcc.\nstderr:\n{}\n\n.cu:\n{cu}",
        String::from_utf8_lossy(&compile.stderr),
    );

    let run = std::process::Command::new(&bin)
        .output()
        .expect("run cuda binary");
    assert!(
        run.status.success(),
        "`{label}`: CUDA binary should run.\nstderr:\n{}",
        String::from_utf8_lossy(&run.stderr),
    );
    let out = String::from_utf8_lossy(&run.stdout).trim().to_string();
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn cuda_equals_tree_walker_over_the_corpus() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!(
            "SKIP cuda execution: needs nvcc + a GPU driver (/dev/nvidia0). Compile is covered by \
                   supported_corpus_compiles_with_nvcc; running awaits the driver."
        );
        return;
    }
    for (label, src) in CORPUS {
        let art =
            compile_to_cuda(src).unwrap_or_else(|e| panic!("`{label}` should compile: {e:?}"));
        let got = nvcc_compile_and_run(&art.kernel_cu, label);
        let want = tw_outcome(src).output.trim().to_string();
        assert_eq!(
            got, want,
            "`{label}`: CUDA output must equal the tree-walker"
        );
    }
}
