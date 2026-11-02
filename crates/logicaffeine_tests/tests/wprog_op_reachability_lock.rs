//! wprog_op_reachability_lock — the EXHAUSTIVE, FINITE "we can't be missing anything" partition for the
//! whole-program fabric, keyed on the 120-op ISA (not on a sample of constructs).
//!
//! `wprog_completeness_lock` proves 11 representative CONSTRUCTS run ≡VM — a sample. This lock is its finite,
//! exhaustive complement: it PARTITIONS all 120 ISA ops by whole-program reachability and proves the partition
//! is TOTAL, so no op can silently fall off the whole-program fabric path. "Send ANY program down the wire and
//! it runs" decomposes into exactly two finite, checkable claims:
//!
//!   (P) OP-REACHABILITY — every ISA op is accounted for: it either lowers into a whole-program config and a
//!       program using it runs ≡VM (proven, welded to `function_op_names` ground truth), or it is EXPLICITLY
//!       classified — structural plumbing / not-source-emittable / off the compute fabric (the X5 boundary) /
//!       a named follow-on the lowering does not yet reach. Nothing is unclassified; the frontier is explicit.
//!   (C) COMPOSITION-FIDELITY — the engine runs any config it produces ≡VM (the `wprog_engine_lock` fuzzers:
//!       2400 branchy + 720 loop + 720 collection programs, every one ≡VM).
//!
//! Together with `fabric_coverage_lock` (118 of 120 ops realized ≡VM on a spatial TILE), this is the
//! op-granularity proof of "any program runs on the fabric ≡VM, or is at an explicit, honest boundary." The
//! partition is a RATCHET: the proven-reachable floor only grows, the follow-on frontier only shrinks.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(wprog_op_reachability_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};

use logicaffeine_compile::codegen_sva::fabric_grid::{
    eval_grid_with_heap, route_loop_body, run_program_cfg_on_grid,
};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, eval_loop_fabric, function_op_names, loop_fabric_from_source,
    program_cfg_fabric, whole_program_op_names,
};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config, program_calls_fabric, serialize_program, wprog_from_decision_fabric,
};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::vm::Op;

// ── the VM oracle ───────────────────────────────────────────────────────────────────────────────────────
fn vm_f(src: &str, x: i64) -> Option<i64> {
    let arg = if x < 0 {
        format!("(0 - {})", -x)
    } else {
        x.to_string()
    };
    let prog = format!("{src}## Main\n    Show f({arg}).\n");
    let out = vm_outcome(&prog);
    if out.error.is_some() {
        return None;
    }
    out.output.split_whitespace().next()?.parse::<i64>().ok()
}

/// Does `f` run on the whole-program fabric ≡VM for all `args`, by ANY of the fabric's proven execution
/// mechanisms? This is the SAME 5-mechanism predicate `wprog_completeness_lock` uses — block-walk, loop
/// fabric, placed decision fabric, call-stack executor, crossbar engine — so a witness that runs here is a
/// witness the census would also accept. `catch_unwind` turns an unsupported op's panic into "does not run".
fn runs_on_fabric_src(src: &str, args: &[i64]) -> bool {
    let all_match = |f: &dyn Fn(i64) -> i64| -> bool {
        catch_unwind(AssertUnwindSafe(|| {
            args.iter()
                .all(|&x| matches!(vm_f(src, x), Some(vm) if f(x) == vm))
        }))
        .unwrap_or(false)
    };
    all_match(&|x| {
        let pf = program_cfg_fabric(src, "f");
        let flat = eval_wprog_config(&serialize_program(&pf), &[x]);
        assert_eq!(
            flat,
            run_program_cfg_on_grid(&pf, &[x]),
            "flat vs grid walk"
        );
        flat
    }) || all_match(&|x| eval_loop_fabric(&loop_fabric_from_source(src, "f"), &[x]))
        || all_match(&|x| {
            let df = decision_fabric_from_source(src, "f");
            let g = route_loop_body(&df.cells, df.n_in, &[df.result]);
            eval_grid_with_heap(&g, &[x])
        })
        || all_match(&|x| eval_wprog_config(&program_calls_fabric(src, "f"), &[x]))
        || all_match(&|x| {
            let df = decision_fabric_from_source(src, "f");
            eval_wprog_config(&wprog_from_decision_fabric(&df), &[x])
        })
}

/// A whole-program witness: a source defining `f`, and inputs it is checked on. Every witness MUST run on the
/// whole-program fabric ≡VM; the ops it DEMONSTRABLY contains (decoded by `function_op_names`) are thereby
/// proven reachable — a coverage claim can never ride on a program that does not actually use the op.
struct Witness {
    src: &'static str,
    args: &'static [i64],
}

/// The witness corpus — proven-running whole-program functions whose decoded ops form the proven-reachable
/// set. These are the `wprog_completeness_lock` census constructs plus a few extra collection/control shapes,
/// so the reachable set is welded to programs that provably run ≡VM (not merely declared).
const WITNESSES: &[Witness] = &[
    // branch — comparison predicate + conditional return.
    Witness { src: "## To f (x: Int) -> Int:\n    If x is greater than 5:\n        Return 1.\n    Return 0.\n", args: &[3, 7] },
    // while-loop — cyclic control flow, mutable accumulate, LtEq predicate, AddAssign.
    Witness { src: "## To f (x: Int) -> Int:\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is at most x:\n        Set s to s + i.\n        Set i to i + 1.\n    Return s.\n", args: &[5, 9] },
    // list — NewList, SetIndex, Index, Add.
    Witness { src: "## To f (x: Int) -> Int:\n    Let mutable xs be [10, x, 30].\n    Set item 1 of xs to 99.\n    Return item 1 of xs + item 2 of xs.\n", args: &[7, 20] },
    // tuple — NewTuple, Index.
    Witness { src: "## To f (x: Int) -> Int:\n    Let t be (10, x, 30).\n    Return item 2 of t.\n", args: &[7, 20] },
    // destructure-tuple — `Repeat for (a, b) in list-of-tuples` spreads each element (DestructureTuple); lowered
    // via `program_calls_fabric`'s iterator + tuple-spread (heap[ptr+j+1] into consecutive result regs).
    Witness { src: "## To f (x: Int) -> Int:\n    Let pairs be [(1, 2), (3, x)].\n    Let mutable acc be 0.\n    Repeat for (a, b) in pairs:\n        Set acc to acc + a + b.\n    Return acc.\n", args: &[7, 20] },
    // call — cross-function Call composition.
    Witness { src: "## To g (n: Int) -> Int:\n    Return n + 1.\n## To f (x: Int) -> Int:\n    Return g(x) + g(x).\n", args: &[7, 20] },
    // division — Div, Mod.
    Witness { src: "## To f (x: Int) -> Int:\n    Return x / 2 + x % 3.\n", args: &[17, 20] },
    // counted range — Int bounds: an LtEq header and the Lt latch that leaves before the increment (no MAX + 1).
    Witness { src: "## To f (x: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for i from 1 to x:\n        Set s to s + i.\n    Return s.\n", args: &[5, 9] },
    // range value — a bound no analysis proves a machine Int (a call's result) keeps NewRange and its iterator.
    Witness { src: "## To g (n: Int) -> Int:\n    Return n * 2.\n## To f (x: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for i from 1 to g(x):\n        Set s to s + i.\n    Return s.\n", args: &[7, 20] },
    // lent iteration — a for-each over a sequence the loop cannot change walks it by position (IndexUnchecked).
    Witness { src: "## To f (x: Int) -> Int:\n    Let xs be [x, 2, 3].\n    Let mutable s be 0.\n    Repeat for v in xs:\n        Set s to s + v.\n    Return s.\n", args: &[7, 20] },
    // double buffer — `Set prev to curr` from a sequence that dies with the iteration exchanges the two (RevSwap).
    Witness { src: "## To f (x: Int) -> Int:\n    Let mutable prev be [x, 1].\n    Let mutable i be 0.\n    While i is less than 2:\n        Let mutable curr be a new Seq of Int.\n        Push item 1 of prev + i to curr.\n        Push x to curr.\n        Set prev to curr.\n        Set i to i + 1.\n    Return item 1 of prev + item 2 of prev.\n", args: &[7, 20] },
    // float — CallBuiltin (floor), the numeric builtin lane.
    Witness { src: "## To f (x: Int) -> Int:\n    Let a be 3.5.\n    Let b be 1.5.\n    Return floor(a + b).\n", args: &[0] },
    // text — LoadConst(Text) → char-list, Length.
    Witness { src: "## To f (x: Int) -> Int:\n    Let s be \"abcd\".\n    Return the length of s.\n", args: &[0] },
    // set — NewEmptySet, SetAdd (dedup), Length (via the crossbar engine).
    Witness { src: "## To f (x: Int) -> Int:\n    Let s be a new Set of Int.\n    Add x to s.\n    Add 5 to s.\n    Add x to s.\n    Return the length of s.\n", args: &[7, 5] },
    // recursion — the frame-stack call executor.
    Witness { src: "## To f (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + f(n - 1).\n", args: &[5, 3] },
    // multiply + subtraction chain — Mul, and a VARIABLE-VARIABLE Sub (constant subtraction folds to Add).
    Witness { src: "## To f (x: Int) -> Int:\n    Let a be x * x.\n    Let b be a - x.\n    Return b.\n", args: &[4, 9] },
    // equality branch — proves Eq (Gt/LtEq are covered above; `<`/`≥` canonicalize away, see NOT_SOURCE_EMITTABLE).
    Witness { src: "## To f (x: Int) -> Int:\n    If x is equal to 3:\n        Return 100.\n    If x is less than 3:\n        Return 200.\n    If x is at least 10:\n        Return 300.\n    Return 500.\n", args: &[3, 1, 10, 5] },
    // bitwise — BitAnd (`&`) / BitOr (`|`) / BitXor (`xor`) on the combinational ALU tiles.
    Witness { src: "## To f (x: Int) -> Int:\n    Let a be x & 6.\n    Let b be x | 1.\n    Let c be x xor 3.\n    Return a + b + c.\n", args: &[5, 12] },
    // shifts — Shl / Shr (small amounts, both agree with the VM under the 6-bit shift mask).
    Witness { src: "## To f (x: Int) -> Int:\n    Let a be x shifted left by 2.\n    Let b be x shifted right by 1.\n    Return a + b.\n", args: &[5, 20] },
    // exponentiation — Pow via the sequential power functional unit.
    Witness { src: "## To f (x: Int) -> Int:\n    Return x to the power of 2.\n", args: &[3, 5] },
    // ── Wave A: collection / set ops (many already lowered or reachable via the crossbar, like SetAdd) ──
    // Contains — membership test folded into a branch predicate.
    Witness { src: "## To f (x: Int) -> Int:\n    Let s be a new Set of Int.\n    Add x to s.\n    Add 5 to s.\n    If s contains 5:\n        Return 1.\n    Return 0.\n", args: &[7, 5] },
    // UnionOp — a fresh set from two.
    Witness { src: "## To f (x: Int) -> Int:\n    Let s be a new Set of Int.\n    Add x to s.\n    Let t be a new Set of Int.\n    Add 5 to t.\n    Let u be s union t.\n    Return the length of u.\n", args: &[7, 5] },
    // IntersectOp — a fresh set of the common elements.
    Witness { src: "## To f (x: Int) -> Int:\n    Let s be a new Set of Int.\n    Add x to s.\n    Add 5 to s.\n    Let t be a new Set of Int.\n    Add 5 to t.\n    Let u be s intersection t.\n    Return the length of u.\n", args: &[7, 5] },
    // RemoveFrom — compacting removal from a set.
    Witness { src: "## To f (x: Int) -> Int:\n    Let s be a new Set of Int.\n    Add x to s.\n    Add 5 to s.\n    Remove 5 from s.\n    Return the length of s.\n", args: &[7, 5] },
    // SeqConcat — `xs followed by ys`.
    Witness { src: "## To f (x: Int) -> Int:\n    Let xs be [x, 2].\n    Let ys be [3, 4].\n    Let zs be xs followed by ys.\n    Return the length of zs.\n", args: &[7, 1] },
    // SliceOp — `items s through e of xs`.
    Witness { src: "## To f (x: Int) -> Int:\n    Let xs be [10, 20, 30, 40, 50].\n    Let ys be items 2 through 4 of xs.\n    Return the length of ys.\n", args: &[0] },
    // DeepClone — `copy of xs`.
    Witness { src: "## To f (x: Int) -> Int:\n    Let xs be [x, 2, 3].\n    Let ys be copy of xs.\n    Return the length of ys.\n", args: &[7, 1] },
    // NewEmptyList — an empty Seq, length 0.
    Witness { src: "## To f (x: Int) -> Int:\n    Let xs be a new Seq of Int.\n    Return the length of xs.\n", args: &[0] },
    // Dynamic capacity remains an explicit op. The physical fabric has a fixed reserve, so it
    // consumes the constructor while treating the hint as non-observable allocation metadata.
    Witness { src: "## To f (x: Int) -> Int:\n    Let mutable xs be a new Seq of Int with capacity x.\n    Push x to xs.\n    Return item 1 of xs.\n", args: &[1, 7] },
    // ── Wave B: structs on the heap state plane — NewStruct / StructInsert / GetField ──
    Witness { src: "## Definition\nA Point has:\n    a public x, which is Int.\n    a public y, which is Int.\n## To f (v: Int) -> Int:\n    Let p be a new Point.\n    Set p's x to v.\n    Set p's y to 5.\n    Return p's x + p's y.\n", args: &[7, 20] },
    // enums / pattern matching — NewInductive (construct) + TestArm/BindArm (Inspect/When).
    Witness { src: "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n## To f (v: Int) -> Int:\n    Let s be a new Circle with radius v.\n    Inspect s:\n        When Circle (r): Return r.\n        When Rectangle (w, h): Return w.\n", args: &[7, 20] },
    // ── Wave C: the correct surfaces (Push/Pop for lists, `//` floor div, `not b` over a bool var) ──
    // ListPush (`Push v to xs`) + ListPop (`Pop from xs into v`).
    Witness { src: "## To f (x: Int) -> Int:\n    Let mutable xs be [x, 2, 3].\n    Push 9 to xs.\n    Pop from xs into v.\n    Return v + the length of xs.\n", args: &[7, 4] },
    // FloorDiv — `//`.
    Witness { src: "## To f (x: Int) -> Int:\n    Return x // 2.\n", args: &[17, 20] },
    // NewEmptyMap — an empty Map, length 0.
    Witness { src: "## To f (x: Int) -> Int:\n    Let m be a new Map of Int to Int.\n    Return the length of m.\n", args: &[0] },
    // Dynamic map capacity has the same fixed-reserve physical limitation, but is never erased
    // from the semantic bytecode or the whole-program coverage census.
    Witness { src: "## To f (x: Int) -> Int:\n    Let mutable m be a new Map of Int to Int with capacity x.\n    Set item x of m to 5.\n    Return item x of m.\n", args: &[1, 7] },
    // ── Wave D: the harder tail ──
    // ListPushField — `Push v to obj's field` (a struct with a Seq field).
    Witness { src: "## Definition\nA Bag has:\n    a public items, which is Seq of Int.\n## To f (x: Int) -> Int:\n    Let bag be a new Bag.\n    Push x to bag's items.\n    Push 5 to bag's items.\n    Return the length of bag's items.\n", args: &[7, 2] },
    // CRDT counter bump — `Increase c's points by v` on a `Shared` struct's `ConvergentCount` field, which now
    // resolves as an Int slot (NewStruct zero-fills, CrdtBump read-add-writes via GetField's resolved slot).
    Witness { src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## To f (v: Int) -> Int:\n    Let mutable c be a new Counter.\n    Increase c's points by v.\n    Return c's points.\n", args: &[7, 20] },
    // CRDT merge — `Merge a into b` is a per-field add over two `Shared` structs (idempotent-lattice join).
    Witness { src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## To f (v: Int) -> Int:\n    Let mutable a be a new Counter.\n    Increase a's points by v.\n    Let mutable b be a new Counter.\n    Increase b's points by 3.\n    Merge a into b.\n    Return b's points.\n", args: &[7, 20] },
    // CRDT resolve — `Resolve r's value to v` overwrites a `Shared` struct field in place (LWW register).
    Witness { src: "## Definition\nA Reg is Shared and has:\n    value: Int.\n## To f (v: Int) -> Int:\n    Let mutable r be a new Reg.\n    Resolve r's value to v.\n    Return r's value.\n", args: &[7, 20] },
    // CRDT append — constructing `a new Room with capacity v` then `Append` grows the SharedSequence field
    // (CrdtAppend) in place; its length reads back through the resolved field slot.
    Witness { src: "## Definition\nA Room is Shared and has:\n    capacity: Int.\n    members: SharedSequence of Int.\n## To f (v: Int) -> Int:\n    Let mutable r be a new Room with capacity v.\n    Append 5 to r's members.\n    Return r's capacity + the length of r's members.\n", args: &[7, 20] },
];

// ── the partition buckets (every op is in EXACTLY one; proven-reachable is computed from the witnesses) ────

/// STRUCTURAL PLUMBING — Wire/Control ops that whole-program execution is BUILT ON (value moves, the result
/// edge, branch/merge, the call and loop scaffolding). Every whole-program run exercises these; they carry no
/// standalone compute witness (exactly as `fabric_coverage_lock` treats Wire/Control tiles).
const WP_PLUMBING: &[&str] = &[
    // Move/LoadConst/Return/Jump/JumpIfFalse/Call/NewRange/IterPrepare/IterPop/IterNext are all WITNESS-PROVEN
    // (they appear inside the witness functions), so they live in proven-reachable, not here. What remains is
    // the structural plumbing NO witness function's body decodes: module globals, the void-return edge, the
    // short-circuit branch, and the program-end halt (emitted by `Main`, not by `f`).
    "GlobalGet",
    "GlobalSet",
    "ReturnNothing",
    "JumpIfTrue",
    "Halt",
];

/// NOT DIRECTLY SOURCE-EMITTABLE — optimizer/Oracle/reversible/hand-bytecode-only ops with no LOGOS source
/// spelling, so a source witness cannot require them to appear. Each is realized ≡VM on a spatial TILE by
/// hand-bytecode in `fabric_coverage_lock`. (name, why-no-source-witness-can-require-it.)
const NOT_SOURCE_EMITTABLE: &[(&str, &str)] = &[
    ("RevXor", "reversible ISA — no source emits it (rev_alu_core, hand-bytecode witnessed)"),
    ("RevAdd", "reversible ISA — no source emits it"),
    ("RevSub", "reversible ISA — no source emits it"),
    ("RevNeg", "reversible ISA — no source emits it"),
    ("ExactDiv", "Oracle exact-operand division form — no source emits it"),
    ("DivPow2", "Oracle power-of-two division form — no source emits it"),
    ("MagicDivU", "Oracle magic-reciprocal division — no source emits it"),
    ("SetIndexUnchecked", "Oracle proven-in-bounds set-index — no source emits it"),
    ("NewEmptyListI32", "narrowing-proven half-width empty Seq — no source emits it"),
    ("NewEmptyListFixed", "optimizer proof-sized empty Seq — no source spelling requires this internal form"),
    ("BigIntWide", "256-bit BigInt lane — hand-bytecode witnessed, no source emits it"),
    ("RegionBoundsGuard", "Oracle bounds-check assertion — no source emits it on the fabric path"),
    ("CrdtJoin", "per-field MAX G-Counter join — no source emits it (hand-bytecode witnessed)"),
    ("ApproxEq", "`is approximately` refuses float operands; the int case is hand-bytecode witnessed"),
    // The whole-program lowering HANDLES these comparisons (cmp_of maps them), but the front end CANONICALIZES
    // `≥` to `≤` with swapped operands and rejects `is not equal to`, so no LOGOS source emits them. (`<` canonicalizes
    // the same way; Lt is emitted by the counted range's latch, witnessed above.)
    // They are realized ≡VM on a spatial Cmp TILE by hand-bytecode in `fabric_coverage_lock` — reachable if
    // emitted, but never produced from source, so they cannot appear on the whole-program-from-SOURCE path.
    ("GtEq", "front end canonicalizes `≥` to `≤` (swapped operands) — no source emits GtEq (tile-realized, witnessed)"),
    ("NotEq", "`is not equal to` is unsupported — no source emits NotEq (tile-realized, hand-bytecode witnessed)"),
    ("Not", "`not b` canonicalizes into the branch/negated comparison at the front end — no source emits a fabric-reachable standalone Op::Not (tile-realized as (src==0)?1:0 in fabric_coverage_lock)"),
    ("EnsureOwned", "the COW barrier the COMPILER inserts at mutable-borrow call sites — not a source construct; always-clone = value-identity (tile-realized in fabric_coverage_lock)"),
];

/// OFF THE COMPUTE FABRIC — the Stage-X5 hard boundary: output rendering, the scheduler, channels, select,
/// networking, time, security/observability, and dynamic closures are NOT whole-program INTEGER-COMPUTE tiles.
/// They are software or dedicated edges (output has its OWN proven model, `eval_whole_program`; the scheduler
/// is deterministic coroutine inlining; IO/net are the offline deterministic model). (name, boundary reason.)
const OFF_COMPUTE_PATH: &[(&str, &str)] = &[
    ("Show", "output edge — proven on the whole-program TAPE model (eval_whole_program == VM stdout), not the integer-compute path"),
    ("Concat", "string interpolation — the output/text rendering path, not integer compute"),
    ("FormatValue", "value→text rendering — the output/text path, not integer compute"),
    ("Spawn", "scheduler — deterministic coroutine inlining, not a grid compute tile"),
    ("SpawnHandle", "scheduler — deterministic coroutine inlining"),
    ("TaskAwait", "scheduler join — structural, not compute"),
    ("TaskAbort", "scheduler abort — structural, not compute"),
    ("ChanNew", "channel FIFO — the scheduler/concurrency path"),
    ("ChanSend", "channel enqueue — the scheduler/concurrency path"),
    ("ChanRecv", "channel dequeue — the scheduler/concurrency path"),
    ("ChanTrySend", "channel non-blocking send — the scheduler/concurrency path"),
    ("ChanTryRecv", "channel non-blocking recv — the scheduler/concurrency path"),
    ("ChanClose", "channel close — output-transparent scheduler edge"),
    ("SelectArmRecv", "select arm registration — the scheduler/concurrency path"),
    ("SelectArmTimeout", "select timeout arm — the scheduler/concurrency path"),
    ("Tcp", "real-socket TCP client (`tcpConnect`/…) — host IO, off the compute fabric (Offload::Net in aot_unsupported)"),
    ("Serve", "the `Serve on port …` HTTP accept loop — host IO, off the compute fabric (Offload::Net in aot_unsupported)"),
    ("SelectWait", "select priority resolver — the scheduler/concurrency path"),
    ("NetConnect", "offline networking — deterministic no-op model, not compute"),
    ("NetListen", "offline networking — deterministic loopback inbox, not compute"),
    ("NetSend", "offline networking — deterministic publish, not compute"),
    ("NetStream", "offline networking — deterministic no-op, not compute"),
    ("NetSync", "offline networking — offline-identity, not compute"),
    ("NetAwait", "offline networking — deterministic drain, not compute"),
    ("NetMakePeer", "offline networking — opaque handle wire, not compute"),
    ("Sleep", "temporal delay — vacuous on the clockless fabric, an IO/time edge"),
    ("LoadNow", "clock read — an injected-time edge, not compute"),
    ("LoadToday", "clock read — an injected-time edge, not compute"),
    ("Obs", "observability event — a trace sink, never stdout, not compute"),
    ("FailWith", "raise error — halts the output tape, a control/IO edge"),
    ("CheckPolicy", "security guard — conditionally halts the output tape, a control edge"),
    ("MakeClosure", "closure capture — the dynamic-dispatch (Futamura) follow-on, not integer compute"),
    ("CallValue", "closure invocation — the dynamic-dispatch (Futamura) follow-on"),
    ("Args", "the command-line arguments — a runtime IO/environment read (empty on a spatial circuit), not integer compute"),
];

/// THE FOLLOW-ON FRONTIER — source-emittable COMPUTE ops the whole-program lowering does not YET reach. This
/// is the honest, explicit gap; it is a RATCHET CEILING that may only SHRINK as each op graduates onto the
/// whole-program path (promote it by adding a witness that exercises it, then remove it here). (name, category.)
const FOLLOW_ON: &[(&str, &str)] = &[
    // EMPTY — the frontier is closed. `NewCrdt` (the last entry) is now proven-reachable: a `Shared` struct's
    // CRDT-field default-fill emits it in global struct-init and the program runs ≡VM (welded via
    // `GLOBAL_INIT_OPS` / `whole_program_op_names`; end-to-end in `lowering_crdt_lock`). `DestructureTuple`
    // graduated earlier (`Repeat for (a,b) in list` via `program_calls_fabric`).
];

/// The ratchet ceiling on the follow-on frontier — it may only SHRINK. Now ZERO: every source-emittable
/// compute op is proven-reachable on the whole-program fabric ≡VM.
const FOLLOW_ON_CEILING: usize = 0;

/// Ops the whole-program witnesses PROVE reachable — decoded from programs that run ≡VM (welded to proof).
/// Non-fatal: a witness that does not run is REPORTED (not panicked) so one run yields the full picture.
/// Ops emitted ONLY in compiler-generated GLOBAL struct-init (a `Shared` struct's CRDT-field default-fill),
/// which run before any user function so `function_op_names(f)` cannot surface them. Welded via
/// `whole_program_op_names` when a witness that emits the op runs ≡VM.
const GLOBAL_INIT_OPS: &[&str] = &["NewCrdt"];

fn classify() -> (BTreeSet<&'static str>, Vec<&'static str>) {
    let mut proven = BTreeSet::new();
    let mut failing: Vec<&'static str> = Vec::new();
    for w in WITNESSES {
        if runs_on_fabric_src(w.src, w.args) {
            for op in function_op_names(w.src, "f") {
                proven.insert(op);
            }
            // Also weld any GLOBAL-INIT op the whole program emits (e.g. `NewCrdt` from a `Shared` struct's
            // CRDT-field default-fill) — the user function's op stream cannot surface it, but the program
            // DOES emit it and runs ≡VM, so it is genuinely proven-reachable.
            let whole = whole_program_op_names(w.src);
            for &g in GLOBAL_INIT_OPS {
                if whole.iter().any(|o| *o == g) {
                    proven.insert(g);
                }
            }
        } else {
            failing.push(w.src);
        }
    }
    (proven, failing)
}

fn all_op_names() -> BTreeSet<&'static str> {
    Op::all_variants().iter().map(|o| o.name()).collect()
}

#[test]
fn the_whole_program_op_partition_is_total_and_welded() {
    let all = all_op_names();
    assert_eq!(
        all.len(),
        120,
        "the ISA op count changed — reclassify the new op in this partition"
    );

    let (proven, failing) = classify();
    let plumbing: BTreeSet<&str> = WP_PLUMBING.iter().copied().collect();
    let not_emit: BTreeSet<&str> = NOT_SOURCE_EMITTABLE.iter().map(|(n, _)| *n).collect();
    let off_path: BTreeSet<&str> = OFF_COMPUTE_PATH.iter().map(|(n, _)| *n).collect();
    let follow_on: BTreeSet<&str> = FOLLOW_ON.iter().map(|(n, _)| *n).collect();
    let statics = [
        ("WP_PLUMBING", &plumbing),
        ("NOT_SOURCE_EMITTABLE", &not_emit),
        ("OFF_COMPUTE_PATH", &off_path),
        ("FOLLOW_ON", &follow_on),
    ];

    // Every static-bucket name is a REAL op.
    let mut phantom: Vec<&str> = Vec::new();
    for (_, bucket) in &statics {
        for n in bucket.iter() {
            if !all.contains(n) {
                phantom.push(n);
            }
        }
    }

    // OVERLAP: a static bucket must not name an op a witness already PROVES reachable (it's proven, not static).
    let mut overlap: Vec<String> = Vec::new();
    for (label, bucket) in &statics {
        for n in bucket.intersection(&proven) {
            overlap.push(format!(
                "{n} in {label} but PROVEN by a witness — remove it from {label}"
            ));
        }
    }

    // TOTALITY + DISJOINTNESS across all five buckets (proven is primary).
    let mut unclassified: Vec<&str> = Vec::new();
    let mut multiclassified: Vec<&str> = Vec::new();
    for op in &all {
        let n = usize::from(proven.contains(op))
            + usize::from(plumbing.contains(op))
            + usize::from(not_emit.contains(op))
            + usize::from(off_path.contains(op))
            + usize::from(follow_on.contains(op));
        if n == 0 {
            unclassified.push(op);
        } else if n > 1 {
            multiclassified.push(op);
        }
    }
    unclassified.sort_unstable();
    multiclassified.sort_unstable();

    // The COMPLEMENT (every op no witness proves) — the exact set the static buckets must partition.
    let mut complement: Vec<&str> = all.difference(&proven).copied().collect();
    complement.sort_unstable();

    println!(
        "── whole-program op partition (all {} ISA ops) ──",
        all.len()
    );
    println!(
        "proven-reachable (welded to witnesses) = {} : {:?}",
        proven.len(),
        {
            let mut v: Vec<&str> = proven.iter().copied().collect();
            v.sort_unstable();
            v
        }
    );
    println!(
        "COMPLEMENT (must be classified by the static buckets) = {} : {complement:?}",
        complement.len()
    );
    println!(
        "  plumbing={} not-emittable={} off-path={} follow-on={}",
        plumbing.len(),
        not_emit.len(),
        off_path.len(),
        follow_on.len()
    );
    if !failing.is_empty() {
        println!("FAILING WITNESSES ({}):", failing.len());
        for s in &failing {
            println!("  ─── does not run ≡VM ───\n{s}");
        }
    }
    if !phantom.is_empty() {
        println!("PHANTOM (not real ops): {phantom:?}");
    }
    if !overlap.is_empty() {
        println!("OVERLAP:\n  {}", overlap.join("\n  "));
    }
    println!("UNCLASSIFIED = {unclassified:?}");
    println!("MULTICLASSIFIED = {multiclassified:?}");

    assert!(
        failing.is_empty(),
        "some witnesses do not run ≡VM (see FAILING WITNESSES above) — fix or drop them"
    );
    assert!(
        phantom.is_empty(),
        "phantom classifications (not real ops): {phantom:?}"
    );
    assert!(
        overlap.is_empty(),
        "static buckets overlap proven-reachable (see OVERLAP above)"
    );
    assert!(
        unclassified.is_empty(),
        "these ops are UNCLASSIFIED — every op must be reachable/plumbing/not-emittable/off-path/follow-on \
         (we can't be missing anything): {unclassified:?}"
    );
    assert!(
        multiclassified.is_empty(),
        "these ops are in MULTIPLE buckets — the partition must be disjoint: {multiclassified:?}"
    );

    println!(
        "\n✓ all {} ISA ops accounted for on the whole-program path: {} proven-reachable (welded) + {} plumbing \
         + {} tile-only + {} off-fabric (X5) + {} explicit follow-on frontier",
        all.len(),
        proven.len(),
        plumbing.len(),
        not_emit.len(),
        off_path.len(),
        follow_on.len(),
    );
}

#[test]
fn the_reachable_frontier_only_grows() {
    assert!(
        FOLLOW_ON.len() <= FOLLOW_ON_CEILING,
        "the follow-on frontier GREW to {} (ceiling {FOLLOW_ON_CEILING}) — coverage must only shrink the gap",
        FOLLOW_ON.len()
    );
    let (proven, _) = classify();
    let mut seen = BTreeSet::new();
    for (n, _) in FOLLOW_ON {
        assert!(seen.insert(*n), "FOLLOW_ON lists `{n}` twice");
        assert!(
            !proven.contains(n),
            "op `{n}` is on the FOLLOW_ON frontier but a witness now proves it reachable — PROMOTE it \
             (remove from FOLLOW_ON and lower the ceiling)"
        );
    }
    println!(
        "follow-on frontier: {}/{} (ceiling {}) — the explicit, shrinking gap",
        FOLLOW_ON.len(),
        FOLLOW_ON_CEILING,
        FOLLOW_ON_CEILING
    );
}
