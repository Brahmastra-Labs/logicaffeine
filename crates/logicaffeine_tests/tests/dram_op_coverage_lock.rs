//! ★ DRAM OP-COVERAGE RATCHET ★ — catalog-complete classification of every VM `Op` for the
//! compute-in-memory (ComputeDRAM) tier. Like the analog tier, DRAM is a CO-PROCESSOR: a few ops
//! run in the sense amps (bulk bitwise + RowClone), and everything else runs on the host control
//! side — so it uses the same FOUR-way split:
//!
//! - **OnPlane** — bitwise/copy the DRAM computes in place, EACH with a witnessed ComputeDRAM
//!   primitive ([`dram_prim`]) proven ≡VM against the `unified_placement` reference;
//! - **Boundary** — staging operand rows in / reading result rows out;
//! - **ControlPlane** — digital-exact ops the host runs VM-identically (no ALU/sequencer in DRAM);
//! - **OffPlane(reason)** — refuses the whole offload (concurrency / networking).
//!
//! [`codegen_dram::fragment`] is EXHAUSTIVE on `Op` (no `_` arm): a new VM instruction fails to
//! compile the crate until classified. This lock pins the partition three ways — [`KNOWN_ONPLANE`]
//! (grow-only: closing an op is new in-DRAM capability), [`KNOWN_BOUNDARY`] (exact), [`KNOWN_OFFPLANE`]
//! (shrink-only) — proves the LIVE classification over the WHOLE ISA equals those ledgers, proves
//! every OnPlane op lowers to a DRAM-capable primitive that is ≡VM, and proves the catalog agrees
//! with the backend's real refusal ([`dram_refusal`]). Catalog, backend, and ≡VM proof cannot drift.
//!
//! ⚠️ To CLOSE an op (move it into OnPlane) you must: (1) reclassify it in `codegen_dram::fragment`,
//! (2) wire `codegen_dram::dram_prim` to a DRAM-capable primitive, (3) add it to `KNOWN_ONPLANE` and
//! raise `ONPLANE_FLOOR`. The ≡VM half then proves the realization is real.

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_dram::{
    dram_prim, dram_refusal, dram_tier, fragment, unsupported_reason, DramFragment, DramLowerError,
};
use logicaffeine_compile::codegen_sva::unified_placement::{
    exec_dram_ambit, exec_reference, Prim, PrimKind, Row, PRIMS,
};
use logicaffeine_compile::optimize::oracle_analyze_with;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::{CompiledProgram, Compiler, Op};

/// GROW-ONLY ledger: the ops the DRAM sense amps realize in place. Growing this set = a new
/// witnessed in-DRAM primitive; shrinking it is a regression. Copy/AND/OR run on UNMODIFIED
/// ComputeDRAM; BitXor needs the Ambit tier (see `codegen_dram::dram_tier`).
const KNOWN_ONPLANE: &[&str] = &[
    "BitAnd",
    "BitOr",
    "BitXor",
    "IntersectOp",
    "UnionOp",
    "DeepClone",
];

/// The count of closed (OnPlane) ops — only ever rises.
const ONPLANE_FLOOR: usize = 6;

/// EXACT ledger: the operand-staging / result-readback framing ops.
const KNOWN_BOUNDARY: &[&str] = &[
    "LoadConst",
    "Move",
    "Show",
    "NewList",
    "NewEmptyList",
    "NewEmptyListWithCapacity",
    "NewEmptyListFixed",
    "NewEmptyListI32",
    "NewEmptySet",
    "NewEmptyMap",
    "NewEmptyMapWithCapacity",
    "NewRange",
    "ListPush",
    "SetAdd",
    "Index",
    "IndexUnchecked",
    "Length",
    "IterPrepare",
    "IterNext",
    "IterPop",
];

/// SHRINK-ONLY ledger: the ops that refuse the whole DRAM offload. Removing an entry (finding it a
/// home in the memory-device model) is a conscious edit here AND in `codegen_dram::fragment`.
const KNOWN_OFFPLANE: &[&str] = &[
    // channels
    "ChanNew",
    "ChanSend",
    "ChanRecv",
    "ChanTrySend",
    "ChanTryRecv",
    "ChanClose",
    // tasks
    "Spawn",
    "SpawnHandle",
    "TaskAwait",
    "TaskAbort",
    // virtual-time / select
    "Sleep",
    "SelectArmRecv",
    "SelectArmTimeout",
    "SelectWait",
    // networking
    "NetConnect",
    "NetListen",
    "NetSend",
    "NetStream",
    "NetAwait",
    "NetMakePeer",
    "NetSync",
    // tcp.lg client sockets — off-chip IO like the Net ops
    "Tcp",
    // the `Serve` HTTP accept loop — host IO, off-chip like the Net/Tcp ops
    "Serve",
];

// ============================================================
//  Corpus
// ============================================================

/// Bitwise programs: at least one op classifies OnPlane, none OffPlane — the backend CLEARS the
/// refusal gate (a DRAM region exists).
const ONPLANE_CORPUS: &[&str] = &[
    "## Main\n    Let x be 12.\n    Let y be 10.\n    Show x & y.\n",
    "## Main\n    Let x be 12.\n    Let y be 10.\n    Show x | y.\n",
    "## Main\n    Let x be 12.\n    Let y be 10.\n    Show x xor y.\n",
];

/// No-bitwise programs: no OnPlane op — the backend refuses `NoBitwiseRegion`.
const NOBITWISE_CORPUS: &[&str] = &[
    "## Main\n    Show 7.\n",
    "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(10).\n",
];

/// Programs with an OffPlane op: the backend must refuse `Unsupported` with the catalog's reason.
const OFFPLANE_CORPUS: &[&str] = &[
    "## To produce (ch: Int):\n    Send 1 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    Receive a from jobs.\n    Show a.\n",
    "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Show \"connected\".\n",
];

fn compile_program(src: &str) -> CompiledProgram {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _p) = parsed.expect("parse");
        let oracle = oracle_analyze_with(stmts, interner);
        Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle)).expect("compile")
    })
}

fn all_ops(prog: &CompiledProgram) -> Vec<Op> {
    prog.code.clone()
}

fn desc_dram_ambit(kind: PrimKind) -> bool {
    PRIMS
        .iter()
        .find(|d| d.kind == kind)
        .expect("kind in PRIMS")
        .dram_ambit
}

// ============================================================
//  Tests
// ============================================================

/// THE RATCHET: the LIVE classification over the WHOLE ISA equals the pinned ledgers exactly.
/// A drift in `codegen_dram::fragment` (an op added to / removed from any category) fails here until
/// the ledger is consciously edited. This is the "closed ops" count made pit-of-success.
#[test]
fn the_live_classification_equals_the_pinned_ledgers() {
    let mut on = BTreeSet::new();
    let mut bd = BTreeSet::new();
    let mut off = BTreeSet::new();
    let mut cp = BTreeSet::new();
    for op in Op::all_variants() {
        let name = op.name();
        match fragment(&op) {
            DramFragment::OnPlane => on.insert(name),
            DramFragment::Boundary => bd.insert(name),
            DramFragment::OffPlane(_) => off.insert(name),
            DramFragment::ControlPlane => cp.insert(name),
        };
    }

    // Ledgers are duplicate-free.
    assert_eq!(
        KNOWN_ONPLANE.len(),
        KNOWN_ONPLANE.iter().collect::<BTreeSet<_>>().len()
    );
    assert_eq!(
        KNOWN_BOUNDARY.len(),
        KNOWN_BOUNDARY.iter().collect::<BTreeSet<_>>().len()
    );
    assert_eq!(
        KNOWN_OFFPLANE.len(),
        KNOWN_OFFPLANE.iter().collect::<BTreeSet<_>>().len()
    );

    // The live classification equals the ledgers, exactly, category by category.
    assert_eq!(
        on,
        KNOWN_ONPLANE.iter().copied().collect(),
        "OnPlane drifted from KNOWN_ONPLANE"
    );
    assert_eq!(
        bd,
        KNOWN_BOUNDARY.iter().copied().collect(),
        "Boundary drifted from KNOWN_BOUNDARY"
    );
    assert_eq!(
        off,
        KNOWN_OFFPLANE.iter().copied().collect(),
        "OffPlane drifted from KNOWN_OFFPLANE"
    );

    // The partition is TOTAL and DISJOINT over every ISA op (finite total partition).
    let total = on.len() + bd.len() + off.len() + cp.len();
    assert_eq!(
        total,
        Op::all_variants().len(),
        "the four categories do not partition the ISA"
    );
    assert!(on.is_disjoint(&bd) && on.is_disjoint(&off) && on.is_disjoint(&cp));
    assert!(bd.is_disjoint(&off) && bd.is_disjoint(&cp) && off.is_disjoint(&cp));

    // Grow-only floor: closed-op count never regresses.
    assert!(
        on.len() >= ONPLANE_FLOOR,
        "OnPlane count fell to {} (floor {ONPLANE_FLOOR})",
        on.len()
    );
}

/// THE ≡VM HALF: every OnPlane op maps to a DRAM-capable primitive whose in-DRAM execution equals
/// the VM reference over fuzzed rows — and the biconditional holds (only OnPlane ops have a
/// `dram_prim`). You cannot close an op without a real, proven in-DRAM realization.
#[test]
fn every_onplane_op_lowers_to_a_dram_capable_prim_eq_vm() {
    let mut x = 0xdead_beef_cafe_babeu64;
    let mut rng = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut closed = 0usize;
    for op in Op::all_variants() {
        match fragment(&op) {
            DramFragment::OnPlane => {
                closed += 1;
                let kind = dram_prim(&op)
                    .unwrap_or_else(|| panic!("OnPlane op `{}` has no dram_prim", op.name()));
                assert!(
                    desc_dram_ambit(kind),
                    "OnPlane op `{}` maps to non-DRAM prim {kind:?}",
                    op.name()
                );
                assert!(
                    dram_tier(&op).is_some(),
                    "OnPlane op `{}` has no dram_tier",
                    op.name()
                );
                let a: Row = (0..8).map(|_| rng()).collect();
                let b: Row = (0..8).map(|_| rng()).collect();
                let prim = match kind {
                    PrimKind::And => Prim::And { dst: 2, a: 0, b: 1 },
                    PrimKind::Or => Prim::Or { dst: 2, a: 0, b: 1 },
                    PrimKind::Xor => Prim::Xor { dst: 2, a: 0, b: 1 },
                    PrimKind::Copy => Prim::Copy { dst: 2, src: 0 },
                    other => panic!("OnPlane op `{}` unexpected prim {other:?}", op.name()),
                };
                let mut hd = vec![a.clone(), b.clone(), vec![0u64; 8]];
                let mut hr = hd.clone();
                exec_dram_ambit(&mut hd, &prim).expect("DRAM realizes an OnPlane prim");
                exec_reference(&mut hr, &prim);
                assert_eq!(
                    hd[2],
                    hr[2],
                    "in-DRAM `{}` diverged from the VM reference",
                    op.name()
                );
            }
            _ => {
                assert!(
                    dram_prim(&op).is_none(),
                    "non-OnPlane op `{}` has a dram_prim (biconditional broken)",
                    op.name()
                );
                assert!(
                    dram_tier(&op).is_none(),
                    "non-OnPlane op `{}` has a dram_tier (biconditional broken)",
                    op.name()
                );
            }
        }
    }
    assert_eq!(
        closed,
        KNOWN_ONPLANE.len(),
        "closed-op count disagrees with KNOWN_ONPLANE"
    );
}

/// Every op the corpus actually compiles to lands in the ledger its classification claims — the
/// classification is real on compiled programs, not just over `all_variants`.
#[test]
fn classification_matches_the_ledgers_on_compiled_programs() {
    let onplane: BTreeSet<&str> = KNOWN_ONPLANE.iter().copied().collect();
    let boundary: BTreeSet<&str> = KNOWN_BOUNDARY.iter().copied().collect();
    let offplane: BTreeSet<&str> = KNOWN_OFFPLANE.iter().copied().collect();

    let (mut seen_on, mut seen_bd, mut seen_off) = (false, false, false);
    for src in ONPLANE_CORPUS
        .iter()
        .chain(NOBITWISE_CORPUS)
        .chain(OFFPLANE_CORPUS)
    {
        for op in all_ops(&compile_program(src)) {
            let name = op.name();
            match fragment(&op) {
                DramFragment::OnPlane => {
                    assert!(
                        onplane.contains(name),
                        "`{name}` OnPlane but not in KNOWN_ONPLANE"
                    );
                    seen_on = true;
                }
                DramFragment::Boundary => {
                    assert!(
                        boundary.contains(name),
                        "`{name}` Boundary but not in KNOWN_BOUNDARY"
                    );
                    seen_bd = true;
                }
                DramFragment::OffPlane(_) => {
                    assert!(
                        offplane.contains(name),
                        "`{name}` OffPlane but not in KNOWN_OFFPLANE"
                    );
                    seen_off = true;
                }
                DramFragment::ControlPlane => assert!(
                    !onplane.contains(name) && !boundary.contains(name) && !offplane.contains(name),
                    "`{name}` ControlPlane but appears in a ledger"
                ),
            }
        }
    }
    assert!(seen_on, "corpus exercised no OnPlane op — vacuous");
    assert!(seen_bd, "corpus exercised no Boundary op — vacuous");
    assert!(seen_off, "corpus exercised no OffPlane op — vacuous");
}

/// The catalog agrees with the backend refusal gate: OnPlane program clears it, no-bitwise program
/// refuses `NoBitwiseRegion`, OffPlane program refuses `Unsupported` with the catalog's reason.
#[test]
fn fragment_agrees_with_backend_refusal() {
    for src in ONPLANE_CORPUS {
        let prog = compile_program(src);
        assert!(
            all_ops(&prog)
                .iter()
                .any(|op| fragment(op) == DramFragment::OnPlane),
            "bitwise program has no OnPlane op:\n{src}"
        );
        assert!(
            all_ops(&prog)
                .iter()
                .all(|op| !matches!(fragment(op), DramFragment::OffPlane(_))),
            "bitwise program contains an OffPlane op:\n{src}"
        );
        assert_eq!(
            dram_refusal(&prog),
            None,
            "backend refused a valid DRAM program:\n{src}"
        );
    }
    for src in NOBITWISE_CORPUS {
        let prog = compile_program(src);
        assert_eq!(unsupported_reason(&prog), None);
        assert_eq!(
            dram_refusal(&prog),
            Some(DramLowerError::NoBitwiseRegion),
            "no-bitwise program must refuse NoBitwiseRegion:\n{src}"
        );
    }
    for src in OFFPLANE_CORPUS {
        let prog = compile_program(src);
        assert!(
            all_ops(&prog)
                .iter()
                .any(|op| matches!(fragment(op), DramFragment::OffPlane(_))),
            "off-plane corpus program has no OffPlane op per `fragment`:\n{src}"
        );
        let reason = unsupported_reason(&prog);
        assert!(
            reason.is_some(),
            "backend failed to refuse an OffPlane program:\n{src}"
        );
        assert_eq!(
            dram_refusal(&prog),
            Some(DramLowerError::Unsupported(reason.unwrap()))
        );
    }
}
