//! ★ ANALOG OP-COVERAGE RATCHET ★ — catalog-complete classification of every VM `Op` for the
//! mixed-signal tier. Analog needs a FOUR-way split, because most ops legitimately run on the
//! digital side of the SoC rather than being refused:
//!
//! - **OnPlane** — float dataflow that can become analog tiles (Kirchhoff sums, gm multiplies,
//!   comparators) inside a mapped region;
//! - **Boundary** — DAC/ADC framing: sequence streaming into the plane, result readback out of it;
//! - **ControlPlane** — digital-exact ops the host/sequencer side runs VM-identically (they are
//!   never mapped to voltages, and they do NOT refuse the compilation);
//! - **OffPlane(reason)** — refuses the ENTIRE analog compilation (concurrency / networking — the
//!   mixed-signal SoC model has no scheduler or host IO story).
//!
//! [`fragment`] is an EXHAUSTIVE match on `Op` (no `_` arm): a new VM instruction fails to compile
//! this file until deliberately classified. The OnPlane/Boundary/OffPlane sets are pinned by exact
//! ledgers ([`KNOWN_ONPLANE`] grow-only, [`KNOWN_BOUNDARY`] exact, [`KNOWN_OFFPLANE`] shrink-only),
//! and the behavioural tests prove the catalog agrees with the backend's real refusal
//! ([`codegen_ana::unsupported_reason`] / [`codegen_ana::analog_refusal`]) — catalog and emitter
//! cannot drift apart.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. Moving an op between categories is a conscious edit here
//! AND in `codegen_ana`; the behavioural half proves they agree.

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_ana::{analog_refusal, unsupported_reason, AnaLowerError};
use logicaffeine_compile::optimize::oracle_analyze_with;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::{CompiledProgram, Compiler, Op};

/// Where an op lives in the mixed-signal SoC model.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fragment {
    OnPlane,
    Boundary,
    ControlPlane,
    OffPlane(&'static str),
}

/// ★ THE SINGLE SOURCE OF TRUTH ★ for how the analog tier classifies every VM op.
/// EXHAUSTIVE on `Op` (no `_` arm) — a new instruction cannot escape classification.
fn fragment(op: &Op) -> Fragment {
    use Fragment::{Boundary as Bd, ControlPlane as Cp, OffPlane as Off, OnPlane as On};
    match op {
        // ── OnPlane: float-capable dataflow — analog tiles inside a mapped region. ──
        // LoadConst/Move are type-agnostic carriers; Add/Sub/Mul map to Kirchhoff sums,
        // differential negation, and gm multipliers; the orderings map to the comparator tile.
        Op::LoadConst { .. } | Op::Move { .. } => On,
        Op::Add { .. } | Op::AddAssign { .. } | Op::Sub { .. } | Op::Mul { .. } => On,
        Op::Lt { .. } | Op::Gt { .. } | Op::LtEq { .. } | Op::GtEq { .. } => On,

        // ── Boundary: DAC/ADC framing — data streamed into the plane and read back out. ──
        Op::Show { .. } => Bd,
        Op::NewEmptyList { .. }
        | Op::NewEmptyListWithCapacity { .. }
        | Op::NewEmptyListFixed { .. }
        | Op::ListPush { .. }
        | Op::Length { .. }
        | Op::Index { .. }
        | Op::IndexUnchecked { .. }
        | Op::IterPrepare { .. }
        | Op::IterNext { .. }
        | Op::IterPop => Bd,
        Op::NewList { .. } | Op::NewRange { .. } => Bd,

        // ── OnPlane growth (O6): the translinear ratio tile witnesses float division —
        // `Div`/`ExactDiv` lower through the Gilbert loop with the divisor's certified box
        // clearing zero. Integer in-region division still refuses at extraction (quotient
        // grids are unforgiving); as a host-side op it runs digitally like everything else. ──
        Op::Div { .. } | Op::ExactDiv { .. } => On,

        // ── ControlPlane: digital-exact — the host/sequencer side runs these VM-identically. ──
        // Continuous equality is measure-zero; bitwise has no voltage semantics — all stay
        // digital, refusing nothing.
        Op::Mod { .. } | Op::FloorDiv { .. } | Op::DivPow2 { .. } => Cp,
        Op::BitXor { .. }
        | Op::BitAnd { .. }
        | Op::BitOr { .. }
        | Op::Shl { .. }
        | Op::Shr { .. } => Cp,
        Op::Pow { .. } | Op::MagicDivU { .. } | Op::CallBuiltin { .. } | Op::EnsureOwned { .. } => {
            Cp
        }
        Op::RegionBoundsGuard { .. } => Cp,
        Op::Eq { .. } | Op::NotEq { .. } | Op::Not { .. } | Op::ApproxEq { .. } => Cp,
        Op::Jump { .. } | Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. } => Cp,
        Op::Call { .. } | Op::Return { .. } | Op::ReturnNothing | Op::Halt => Cp,
        Op::MakeClosure { .. } | Op::CallValue { .. } => Cp,
        Op::GlobalGet { .. } | Op::GlobalSet { .. } => Cp,
        Op::LoadToday { .. } | Op::LoadNow { .. } => Cp,
        Op::NewCrdt { .. }
        | Op::CrdtBump { .. }
        | Op::CrdtMerge { .. }
        | Op::CrdtAppend { .. }
        | Op::CrdtResolve { .. }
        | Op::CrdtJoin { .. } => Cp,
        Op::Concat { .. } | Op::FormatValue { .. } => Cp,
        Op::SeqConcat { .. }
        | Op::SliceOp { .. }
        | Op::Contains { .. }
        | Op::DeepClone { .. }
        | Op::SetIndex { .. }
        | Op::SetIndexUnchecked { .. } => Cp,
        Op::NewEmptySet { .. }
        | Op::SetAdd { .. }
        | Op::RemoveFrom { .. }
        | Op::UnionOp { .. }
        | Op::IntersectOp { .. } => Cp,
        Op::NewEmptyMap { .. } | Op::NewEmptyMapWithCapacity { .. } | Op::NewTuple { .. } => Cp,
        Op::NewStruct { .. } | Op::StructInsert { .. } | Op::GetField { .. } => Cp,
        Op::NewInductive { .. } | Op::TestArm { .. } | Op::BindArm { .. } => Cp,
        Op::DestructureTuple { .. } | Op::ListPop { .. } => Cp,
        Op::FailWith { .. } | Op::CheckPolicy { .. } => Cp,
        Op::Args { .. } | Op::ListPushField { .. } => Cp,
        Op::RevSwap { .. }
        | Op::RevXor { .. }
        | Op::RevNeg { .. }
        | Op::RevAdd { .. }
        | Op::RevSub { .. } => Cp,
        Op::NewEmptyListI32 { .. } | Op::BigIntWide { .. } | Op::Obs { .. } => Cp,

        // ── OffPlane: refuses the whole compilation — no scheduler / host IO in the SoC model. ──
        Op::ChanNew { .. }
        | Op::ChanSend { .. }
        | Op::ChanRecv { .. }
        | Op::ChanTrySend { .. }
        | Op::ChanTryRecv { .. }
        | Op::ChanClose { .. } => Off("channel"),
        Op::Spawn { .. } | Op::SpawnHandle { .. } | Op::TaskAwait { .. } | Op::TaskAbort { .. } => {
            Off("task")
        }
        Op::Sleep { .. }
        | Op::SelectArmRecv { .. }
        | Op::SelectArmTimeout { .. }
        | Op::SelectWait { .. } => Off("virtual-time/select"),
        Op::NetConnect { .. }
        | Op::NetListen { .. }
        | Op::NetSend { .. }
        | Op::NetStream { .. }
        | Op::NetAwait { .. }
        | Op::NetMakePeer { .. }
        | Op::NetSync { .. }
        | Op::Tcp { .. }
        | Op::Serve { .. } => Off("networking"),
    }
}

/// GROW-ONLY ledger: the ops the analog plane can host. Growing this set = new analog capability
/// (a witnessed tile); shrinking it is a regression.
const KNOWN_ONPLANE: &[&str] = &[
    "LoadConst",
    "Move",
    "Add",
    "AddAssign",
    "Sub",
    "Mul",
    "Lt",
    "Gt",
    "LtEq",
    "GtEq",
    // O6: the translinear ratio tile — division joins the plane.
    "Div",
    "ExactDiv",
];

/// EXACT ledger: the DAC/ADC framing ops.
const KNOWN_BOUNDARY: &[&str] = &[
    "Show",
    "NewEmptyList",
    "NewEmptyListWithCapacity",
    "NewEmptyListFixed",
    "ListPush",
    "Length",
    "Index",
    "IndexUnchecked",
    "IterPrepare",
    "IterNext",
    "IterPop",
    "NewList",
    "NewRange",
];

/// SHRINK-ONLY ledger: the ops that refuse the whole analog compilation. Removing an entry
/// (finding it a home in the SoC model) is a conscious edit here AND in `fragment`.
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
    // host networking (real TCP sockets + the `Serve` accept loop)
    "Tcp",
    "Serve",
];

// ============================================================
//  Corpus
// ============================================================

/// Float-dataflow programs: every op classifies OnPlane/Boundary/ControlPlane (never OffPlane),
/// at least one op is OnPlane, and the backend refuses ONLY for missing ranges (`UnboundedInput`).
const FLOAT_CORPUS: &[&str] = &[
    "## Main\n    Let x be 0.5.\n    Let y be x * 2.0 + 0.25.\n    Show y.\n",
    "## Main\n    Let a be 0.125.\n    Let b be 0.5.\n    If a is less than b:\n        Show a.\n    Otherwise:\n        Show b.\n",
    "## Main\n    Let mutable xs be a new Seq of Float.\n    Push 0.5 to xs.\n    Push 0.25 to xs.\n    Let mutable acc be 0.0.\n    Repeat for each x in xs:\n        Set acc to acc + x * 2.0.\n    Show acc.\n",
];

/// Digital-only programs: no OnPlane float region — the backend refuses `NoAnalogRegion`.
const DIGITAL_CORPUS: &[&str] = &[
    "## Main\n    Show 7.\n",
    "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(10).\n",
];

/// Programs with an OffPlane op: the backend must refuse `Unsupported` with the catalog's reason.
const OFFPLANE_CORPUS: &[&str] = &[
    "## To produce (ch: Int):\n    Send 1 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    Receive a from jobs.\n    Show a.\n",
    "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Show \"connected\".\n",
    "## Main\n    Let c be tcpConnect(\"127.0.0.1\", 1).\n    Inspect c:\n        When Ok (conn):\n            Show \"open\".\n        When Err (e):\n            Show e.\n",
    "## To handle (request: Request) -> Response:\n    Return plain(\"hi\").\n\n## Main\n    Serve on port 8080 with handle.\n",
];

fn compile_program(src: &str) -> CompiledProgram {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _p) = parsed.expect("parse");
        let oracle = oracle_analyze_with(stmts, interner);
        Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle)).expect("compile")
    })
}

fn all_ops(prog: &CompiledProgram) -> Vec<Op> {
    // One flat code array — function bodies live in `code` behind their `entry_pc`.
    prog.code.clone()
}

// ============================================================
//  Tests
// ============================================================

/// Every ledger is duplicate-free, and every op the corpus observes lands in the ledger its
/// classification claims — no undocumented or stale entries.
#[test]
fn classification_matches_the_ledgers() {
    let onplane: BTreeSet<&str> = KNOWN_ONPLANE.iter().copied().collect();
    let boundary: BTreeSet<&str> = KNOWN_BOUNDARY.iter().copied().collect();
    let offplane: BTreeSet<&str> = KNOWN_OFFPLANE.iter().copied().collect();
    assert_eq!(
        onplane.len(),
        KNOWN_ONPLANE.len(),
        "KNOWN_ONPLANE has duplicates"
    );
    assert_eq!(
        boundary.len(),
        KNOWN_BOUNDARY.len(),
        "KNOWN_BOUNDARY has duplicates"
    );
    assert_eq!(
        offplane.len(),
        KNOWN_OFFPLANE.len(),
        "KNOWN_OFFPLANE has duplicates"
    );
    assert!(
        onplane.is_disjoint(&boundary)
            && onplane.is_disjoint(&offplane)
            && boundary.is_disjoint(&offplane)
    );

    let mut seen_on = BTreeSet::new();
    let mut seen_bd = BTreeSet::new();
    let mut seen_off = BTreeSet::new();
    for src in FLOAT_CORPUS
        .iter()
        .chain(DIGITAL_CORPUS)
        .chain(OFFPLANE_CORPUS)
    {
        for op in all_ops(&compile_program(src)) {
            let name = op.name();
            match fragment(&op) {
                Fragment::OnPlane => {
                    assert!(
                        onplane.contains(name),
                        "`{name}` is OnPlane but missing from KNOWN_ONPLANE"
                    );
                    seen_on.insert(name);
                }
                Fragment::Boundary => {
                    assert!(
                        boundary.contains(name),
                        "`{name}` is Boundary but missing from KNOWN_BOUNDARY"
                    );
                    seen_bd.insert(name);
                }
                Fragment::ControlPlane => {
                    assert!(
                        !onplane.contains(name)
                            && !boundary.contains(name)
                            && !offplane.contains(name),
                        "`{name}` is ControlPlane but appears in a ledger"
                    );
                }
                Fragment::OffPlane(_) => {
                    assert!(
                        offplane.contains(name),
                        "`{name}` is OffPlane but missing from KNOWN_OFFPLANE"
                    );
                    seen_off.insert(name);
                }
            }
        }
    }
    assert!(
        !seen_on.is_empty(),
        "float corpus exercised no OnPlane op — corpus is vacuous"
    );
    assert!(
        !seen_bd.is_empty(),
        "corpus exercised no Boundary op — corpus is vacuous"
    );
    assert!(
        !seen_off.is_empty(),
        "off-plane corpus exercised no OffPlane op — corpus is vacuous"
    );
}

/// The catalog agrees with the backend: `unsupported_reason` fires IFF an OffPlane op is present,
/// and `analog_refusal` distinguishes the missing-ranges case from the no-region case exactly as
/// the classification predicts.
#[test]
fn fragment_agrees_with_backend_refusal() {
    for src in FLOAT_CORPUS {
        let prog = compile_program(src);
        assert!(
            all_ops(&prog)
                .iter()
                .all(|op| !matches!(fragment(op), Fragment::OffPlane(_))),
            "float corpus program contains an OffPlane op:\n{src}"
        );
        assert!(
            all_ops(&prog)
                .iter()
                .any(|op| fragment(op) == Fragment::OnPlane),
            "float corpus program has no OnPlane op:\n{src}"
        );
        assert_eq!(
            unsupported_reason(&prog),
            None,
            "backend hard-refused a float program:\n{src}"
        );
        assert!(
            matches!(
                analog_refusal(&prog, &[]),
                Some(AnaLowerError::UnboundedInput(_))
            ),
            "float program without ranges must refuse UnboundedInput:\n{src}"
        );
    }
    for src in DIGITAL_CORPUS {
        let prog = compile_program(src);
        assert_eq!(unsupported_reason(&prog), None);
        assert_eq!(
            analog_refusal(&prog, &[]),
            Some(AnaLowerError::NoAnalogRegion),
            "digital-only program must refuse NoAnalogRegion:\n{src}"
        );
    }
    for src in OFFPLANE_CORPUS {
        let prog = compile_program(src);
        assert!(
            all_ops(&prog)
                .iter()
                .any(|op| matches!(fragment(op), Fragment::OffPlane(_))),
            "off-plane corpus program has no OffPlane op per `fragment`:\n{src}"
        );
        let reason = unsupported_reason(&prog);
        assert!(
            reason.is_some(),
            "backend failed to refuse an OffPlane program:\n{src}"
        );
        assert_eq!(
            analog_refusal(&prog, &[]),
            Some(AnaLowerError::Unsupported(reason.unwrap()))
        );
    }
}
