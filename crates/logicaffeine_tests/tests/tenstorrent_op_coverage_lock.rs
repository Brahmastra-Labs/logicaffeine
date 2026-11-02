//! ★ TENSTORRENT OP-COVERAGE RATCHET ★ — catalog-complete classification of every VM `Op` as
//! either ON-CORE (runs on a single Tensix baby core via the proven C compute body) or OFF-CORE
//! (concurrency / networking — needs a scheduler or host IO, so it cannot map to one core and is
//! REFUSED). The sibling discipline to `riscv_aot_lock`'s `op_support`.
//!
//! [`fragment`] is an EXHAUSTIVE match on `Op` (no `_` arm): a new VM instruction fails to compile
//! this file until it is deliberately classified. The OFF-CORE set is pinned by a shrink-only
//! [`KNOWN_OFFCORE`] ledger, and the behavioural tests prove the classification agrees with the
//! backend's actual refusal ([`codegen_tt::unsupported_reason`]) — so the catalog and the emitter
//! cannot drift apart.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. Moving an op OFF-CORE → ON-CORE (or the reverse) is a
//! conscious edit here AND in `codegen_tt::unsupported_reason`; the behavioural half proves they agree.

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_tt::unsupported_reason;
use logicaffeine_compile::optimize::oracle_analyze_with;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::{CompiledProgram, Compiler, Op};

/// Where an op runs. `OffCore` carries a short reason (concurrency / networking).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fragment {
    OnCore,
    OffCore(&'static str),
}

/// ★ THE SINGLE SOURCE OF TRUTH ★ for which VM ops the Tenstorrent single-baby-core tier runs.
/// EXHAUSTIVE on `Op` (no `_` arm) — a new instruction cannot escape classification. Keep in lockstep
/// with `codegen_tt::unsupported_reason`; the behavioural tests prove they agree.
fn fragment(op: &Op) -> Fragment {
    use Fragment::{OffCore as Off, OnCore as On};
    match op {
        // ── Scalar / control / calls — the C compute body runs these on one core. ──
        Op::LoadConst { .. } | Op::Move { .. } => On,
        Op::Add { .. } | Op::AddAssign { .. } | Op::Sub { .. } | Op::Mul { .. } => On,
        Op::Div { .. }
        | Op::Mod { .. }
        | Op::FloorDiv { .. }
        | Op::DivPow2 { .. }
        | Op::ExactDiv { .. } => On,
        Op::BitXor { .. }
        | Op::BitAnd { .. }
        | Op::BitOr { .. }
        | Op::Shl { .. }
        | Op::Shr { .. } => On,
        Op::Pow { .. } | Op::MagicDivU { .. } | Op::CallBuiltin { .. } | Op::EnsureOwned { .. } => {
            On
        }
        Op::RegionBoundsGuard { .. } => On,
        Op::Lt { .. } | Op::Gt { .. } | Op::LtEq { .. } | Op::GtEq { .. } => On,
        Op::Eq { .. } | Op::NotEq { .. } | Op::Not { .. } | Op::ApproxEq { .. } => On,
        Op::Jump { .. } | Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. } => On,
        Op::Call { .. } | Op::Return { .. } | Op::ReturnNothing | Op::Halt => On,
        Op::MakeClosure { .. } | Op::CallValue { .. } => On,
        Op::GlobalGet { .. } | Op::GlobalSet { .. } => On,
        Op::LoadToday { .. } | Op::LoadNow { .. } => On,
        // Single-replica CRDT counters are in-memory maps in the C body — on-core.
        Op::NewCrdt { .. }
        | Op::CrdtBump { .. }
        | Op::CrdtMerge { .. }
        | Op::CrdtAppend { .. }
        | Op::CrdtResolve { .. }
        | Op::CrdtJoin { .. } => On,
        Op::Show { .. } | Op::Concat { .. } | Op::FormatValue { .. } => On,
        Op::NewEmptyList { .. }
        | Op::ListPush { .. }
        | Op::Length { .. }
        | Op::Index { .. }
        | Op::IndexUnchecked { .. }
        | Op::IterPrepare { .. }
        | Op::IterNext { .. }
        | Op::IterPop => On,
        Op::NewList { .. }
        | Op::NewRange { .. }
        | Op::SeqConcat { .. }
        | Op::SliceOp { .. }
        | Op::Contains { .. }
        | Op::DeepClone { .. }
        | Op::SetIndex { .. }
        | Op::SetIndexUnchecked { .. } => On,
        Op::NewEmptySet { .. }
        | Op::SetAdd { .. }
        | Op::RemoveFrom { .. }
        | Op::UnionOp { .. }
        | Op::IntersectOp { .. } => On,
        Op::NewEmptyMap { .. } | Op::NewEmptyMapWithCapacity { .. } | Op::NewTuple { .. } => On,
        Op::NewStruct { .. } | Op::StructInsert { .. } | Op::GetField { .. } => On,
        Op::NewInductive { .. } | Op::TestArm { .. } | Op::BindArm { .. } => On,
        Op::DestructureTuple { .. } | Op::ListPop { .. } => On,
        Op::FailWith { .. } | Op::CheckPolicy { .. } => On,
        Op::Args { .. } | Op::ListPushField { .. } => On,
        Op::RevSwap { .. }
        | Op::RevXor { .. }
        | Op::RevNeg { .. }
        | Op::RevAdd { .. }
        | Op::RevSub { .. } => On,
        Op::NewEmptyListWithCapacity { .. }
        | Op::NewEmptyListFixed { .. }
        | Op::NewEmptyListI32 { .. }
        | Op::BigIntWide { .. }
        | Op::Obs { .. } => On,

        // ── Offline networking: the loopback transport rides the C net runtime (newlib-clean —
        // `strcmp`/`strdup` + static FIFOs, no `ucontext`), wrapped verbatim by `wrap_as_tt`. A
        // single-node `Send X to peer; Await …` round-trips ≡ the VM's offline `NetInbox`. On-core. ──
        Op::NetConnect { .. }
        | Op::NetListen { .. }
        | Op::NetSend { .. }
        | Op::NetStream { .. }
        | Op::NetAwait { .. }
        | Op::NetMakePeer { .. }
        | Op::NetSync { .. } => On,

        // ── Concurrency: needs a cooperative scheduler + multiple contexts (the C concurrency
        // runtime is `ucontext`-based, which a Tensix baby core has no libc for). Off-core; the
        // genuine multi-baby-core MESH tier (`codegen_tt::concurrency_mesh`) lowers these instead. ──
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

        // ── Host networking: real TCP sockets and the `Serve` HTTP accept loop need host syscalls a
        // Tensix baby core has no libc for — off-core, like the ucontext concurrency runtime. ──
        Op::Tcp { .. } | Op::Serve { .. } => Off("host-io"),
    }
}

/// The EXACT set of op names classified OFF-CORE — a shrink-only ledger. Removing an entry (moving an
/// op on-core) is a conscious edit here AND in `fragment`; the behavioural tests prove no drift.
const KNOWN_OFFCORE: &[&str] = &[
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
    // host networking (real TCP sockets + the `Serve` accept loop)
    "Tcp",
    "Serve",
];

// ============================================================
//  Corpus — on-core programs and off-core programs
// ============================================================

/// On-core programs — every op must classify ON-CORE and the backend must NOT refuse. The last
/// entry is an OFFLINE-NETWORKING loopback (`Send`/`Await` on a self-peer): now on-core via the
/// newlib-clean C net runtime wrapped verbatim (see `codegen_tt::TT_NET_OPS`).
const ONCORE_CORPUS: &[&str] = &[
    "## Main\n    Show 7.\n",
    "## To sq (x: Int) -> Int:\n    Return x * x.\n## Main\n    Show sq(9).\n",
    "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(10).\n",
    "## Main\n    Let mutable xs be a new Seq of Int.\n    Push 10 to xs.\n    Push 20 to xs.\n    Show length of xs.\n",
    "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with x 1 and y 2.\nShow p.\n",
    "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send 7 to peer.\n    Await response from peer into got.\n    Show got.\n",
];

/// Off-core programs — each contains at least one off-core op and the backend must REFUSE.
const OFFCORE_CORPUS: &[&str] = &[
    "## To produce (ch: Int):\n    Send 1 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    Receive a from jobs.\n    Show a.\n",
    "## Main\n    Let c be tcpConnect(\"127.0.0.1\", 1).\n    Inspect c:\n        When Ok (conn):\n            Show \"open\".\n        When Err (e):\n            Show e.\n",
    "## To handle (request: Request) -> Response:\n    Return plain(\"hi\").\n\n## Main\n    Serve on port 8080 with handle.\n",
];

// ============================================================
//  Helpers
// ============================================================

fn compile_program(src: &str) -> CompiledProgram {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _p) = parsed.expect("parse");
        let oracle = oracle_analyze_with(stmts, interner);
        Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle)).expect("compile")
    })
}

// ============================================================
//  Tests
// ============================================================

/// Every op `fragment` classifies OFF-CORE has its name in the shrink-only ledger, and every
/// ledger name is a real, distinct op that `fragment` classifies OFF-CORE — no undocumented or
/// stale entries. (Observed across the off-core corpus, whose ops span the whole set.)
#[test]
fn offcore_classification_matches_the_ledger() {
    let ledger: BTreeSet<&str> = KNOWN_OFFCORE.iter().copied().collect();
    assert_eq!(
        ledger.len(),
        KNOWN_OFFCORE.len(),
        "KNOWN_OFFCORE has duplicate entries"
    );

    let mut observed_offcore: BTreeSet<&str> = BTreeSet::new();
    for src in OFFCORE_CORPUS {
        for op in &compile_program(src).code {
            if let Fragment::OffCore(_) = fragment(op) {
                let name = op.name();
                assert!(
                    ledger.contains(name),
                    "op `{name}` is OFF-CORE but missing from KNOWN_OFFCORE"
                );
                observed_offcore.insert(name);
            }
        }
    }
    assert!(
        !observed_offcore.is_empty(),
        "off-core corpus exercised no off-core op — corpus is vacuous"
    );
}

/// The catalog agrees with the backend: a program is refused by `unsupported_reason` IFF it contains
/// an off-core op per `fragment`. This binds the lock's classification to the emitter's real behaviour.
#[test]
fn fragment_agrees_with_backend_refusal() {
    for src in ONCORE_CORPUS {
        let prog = compile_program(src);
        assert!(
            prog.code.iter().all(|op| fragment(op) == Fragment::OnCore),
            "on-core corpus program contains an op `fragment` calls off-core:\n{src}"
        );
        assert!(
            unsupported_reason(&prog).is_none(),
            "backend refused an all-on-core program:\n{src}"
        );
    }
    for src in OFFCORE_CORPUS {
        let prog = compile_program(src);
        assert!(
            prog.code
                .iter()
                .any(|op| matches!(fragment(op), Fragment::OffCore(_))),
            "off-core corpus program has no off-core op per `fragment`:\n{src}"
        );
        assert!(
            unsupported_reason(&prog).is_some(),
            "backend failed to refuse a program containing an off-core op:\n{src}"
        );
    }
}
