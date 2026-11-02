//! fabric_coverage_lock — Lock 1 of the "Language ASIC" campaign: the COVERAGE RATCHET.
//!
//! The spatial fabric realizes a growing subset of the ISA (`codegen_sva::fabric_tile::fabric_tile` is the
//! single source of truth); the interpreter runs the rest. This lock makes coverage HONEST and monotone:
//!
//!  A. `fabric_coverage_floor_is_monotone` — the realized-op count is pinned EXACTLY to a floor (so both a
//!     lost tile AND an un-bumped addition go RED), and the ISA op count is pinned (a new op must be
//!     classified). Raise the floor as ops graduate; never lower it (rule 14).
//!  B. `every_fabric_realized_op_is_proven_equivalent_to_the_vm` — a witness registry whose keys must EQUAL
//!     the compute-tile op set, and every witness is INVOKED (a fuzzed `eval_decision_fabric == VM`). So a
//!     coverage claim can never outrun its proof: marking an op realized without shipping its witness is
//!     RED, and a wrong tile is RED when the witness runs. Two RED gates in series.
//!  C. `the_uncovered_ops_are_explicit_and_shrinking` — the interpreter-fallback frontier is listed by
//!     name, bounded by a ceiling that only falls, and asserted to PARTITION the whole ISA with the covered
//!     set (no op unaccounted for).
//!
//! As each campaign phase lowers an op category, its arm moves into `fabric_tile`, its witness joins the
//! registry, the floor rises and the ceiling falls — all in one atomic RED→GREEN→LOCK step.

#[path = "common/fabric_witnesses.rs"]
mod fabric_witnesses;

use fabric_witnesses::*;
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_whole_program, function_op_names, whole_program_fabric_from_program,
    whole_program_fabric_from_source,
};
use logicaffeine_compile::codegen_sva::fabric_tile::{fabric_realized_ops, fabric_tile};
use logicaffeine_compile::compile::{
    program_outcome_concurrent, program_outcome_scheduled, vm_outcome, vm_outcome_net,
};
use logicaffeine_compile::intern::Symbol;
use logicaffeine_compile::semantics::builtins::BuiltinId;
use logicaffeine_compile::vm::{ChanElem, CompiledFunction, CompiledProgram, Constant, Op, Vm};
use std::collections::BTreeSet;

/// The ISA op count — the coverage denominator. A new op changes this, forcing a `fabric_tile` disposition.
// 115 → 116: `Op::Tcp` (tcp.lg client sockets) joined the ISA. Real host sockets have no
// spatial-fabric realization (and no offline model yet — unlike the Net relay's loopback
// FIFO), so it sits on the interpreter-fallback frontier (`fabric_tile` = None), the first
// frontier entry since the ISA reached 115/115. Graduating it needs a sealed-world tcp
// model (the file/random testworld-seam precedent), not a silent no-op tile.
// 116 → 117: `Op::Serve` (the HTTP accept loop) joined the ISA. The accept loop is host IO
// (bind/accept/read/write), with no spatial-fabric realization — it sits on the same
// interpreter-fallback frontier as `Tcp` (`fabric_tile` = None).
// 117 → 120: fixed and dynamic Seq/Map capacities became explicit bytecode instead of
// being silently erased. All map to the fixed-reserve Mem tile; hints have no physical encoding.
const TOTAL_OPS: usize = 120;
/// The realized-op floor: RAISE as ops graduate onto a tile, NEVER lower (rule 14).
/// 67 prior + GlobalGet/GlobalSet (wires) + JumpIfTrue (short-circuit) + Sleep (no-op) + RevXor/RevAdd/RevSub/
/// RevNeg (reversible ALU) + CrdtJoin (per-field MAX) + ChanClose (no-op) + IndexUnchecked/SetIndexUnchecked
/// (in-bounds reads/writes) + ExactDiv/DivPow2 (Oracle divisions) + RegionBoundsGuard (guard no-op)
/// + MagicDivU (magic-reciprocal division, MulHiU func unit) + NewEmptyListI32 (half-width empty seq)
/// + NetConnect/NetListen/NetSend/NetStream/NetSync/NetMakePeer (offline networking no-ops)
/// + ChanTryRecv (non-blocking recv = dequeue on a non-empty channel)
/// + Spawn/SpawnHandle/TaskAwait/TaskAbort (the scheduler: inline-producer + instant join/abort)
/// + DestructureTuple (tuple unpack) + Args (empty on a spatial circuit)
/// + NetAwait (offline loopback drain)
/// + SelectArmRecv/SelectArmTimeout/SelectWait (the priority select state machine)
/// + FormatValue (default rendering to text) + ApproxEq (numeric approximate equality)
/// + FailWith (halts the output tape at the error) + EnsureOwned (COW isolation clone)
/// + IterPop (loop-end iterator no-op) + Obs (observability side-channel, no output effect)
/// + MakeClosure/CallValue (closure capture + static-body dispatch)
/// + CallBuiltin (the float lane: Sqrt/Floor + int Abs/Min/Max)
/// + BigIntWide (the 256-bit lane: FromInt/Add/Mul/Low with overflow-to-BigInt tracking)
/// + CheckPolicy (the security guard: single-field grant predicate + conditional tape halt)
/// + LoadNow/LoadToday (clock value-loads reading the injected fixed clock; fields via div/mod)
/// + NewCrdt (fresh CRDT value: a counter/PN-counter is 0)
/// + NewEmptyListFixed/NewEmptyListWithCapacity/NewEmptyMapWithCapacity (fixed-reserve Mem tile)
/// = 118 realized ops.
const FABRIC_TILE_FLOOR: usize = 118;
/// The interpreter-fallback ceiling: LOWER as ops graduate, never raise.
const UNCOVERED_CEILING: usize = TOTAL_OPS - FABRIC_TILE_FLOOR;

// ── A. the floor ratchet ────────────────────────────────────────────────────────────────────────────
#[test]
fn fabric_coverage_floor_is_monotone() {
    assert_eq!(
        Op::all_variants().len(),
        TOTAL_OPS,
        "the ISA op count changed — a new op must get a `fabric_tile` disposition (Some tile or None) and \
         this pin + the ratchet floor/ceiling updated"
    );
    let realized = fabric_realized_ops();
    assert_eq!(
        realized.len(),
        FABRIC_TILE_FLOOR,
        "fabric-tile floor changed — RAISE it as ops graduate onto a tile, NEVER lower it (rule 14). \
         realized = {realized:?}"
    );
    println!(
        "fabric coverage: {}/{} ops on the spatial datapath = {:.1}%",
        realized.len(),
        TOTAL_OPS,
        realized.len() as f64 / TOTAL_OPS as f64 * 100.0
    );
}

// ── B. claim welded to proof ──────────────────────────────────────────────────────────────────────────
#[test]
fn every_fabric_realized_op_is_proven_equivalent_to_the_vm() {
    // The compute tiles (independently-fuzzable datapath) MUST each carry a witness; structural tiles
    // (Wire/Control) are framework plumbing exercised by every fabric, so they carry no standalone witness.
    let compute: BTreeSet<String> = Op::all_variants()
        .iter()
        .filter(|o| fabric_tile(o).is_some_and(|t| t.is_compute()))
        .map(|o| o.name().to_string())
        .collect();
    let mut proven: BTreeSet<String> = FABRIC_EQUIV.iter().map(|(n, _)| n.to_string()).collect();

    let orphan: Vec<&String> = proven.difference(&compute).collect();
    assert!(
        orphan.is_empty(),
        "witness {orphan:?} references an op that is not a compute tile"
    );

    // The LIFT: a compute op is proven by a per-op witness OR by any corpus program that DEMONSTRABLY
    // contains it (function_op_names decodes the ground truth — a claim can't ride on a program that
    // doesn't use the op). One rich program threads many needles at once.
    for cf in CORPUS {
        for op in function_op_names(cf.src, cf.fn_name) {
            proven.insert(op.to_string());
        }
    }
    let unproven: Vec<&String> = compute.difference(&proven).collect();
    assert!(
        unproven.is_empty(),
        "compute op(s) {unproven:?} claim a fabric tile but NO witness or corpus program exercises them — \
         a coverage claim MUST come with its proof (add a witness/corpus program, or drop the tile)"
    );

    // Actually EXERCISE everything — declaration is not proof.
    for (name, run) in FABRIC_EQUIV {
        println!("witness: {name}");
        run();
    }
    for cf in CORPUS {
        println!("corpus: {}", cf.fn_name);
        run_corpus_fn(cf);
    }
    for src in WHOLE_CORPUS {
        println!("whole-program differential");
        run_whole_fn(src);
    }
}

// ── C. the honest, shrinking frontier ─────────────────────────────────────────────────────────────────
#[test]
fn the_uncovered_ops_are_explicit_and_shrinking() {
    let uncovered: Vec<String> = Op::all_variants()
        .iter()
        .filter(|o| fabric_tile(o).is_none())
        .map(|o| o.name().to_string())
        .collect();
    println!(
        "── {} ops still on the interpreter fallback (no fabric tile yet) ──\n{uncovered:#?}",
        uncovered.len()
    );
    assert!(
        uncovered.len() <= UNCOVERED_CEILING,
        "the uncovered set GREW to {} (ceiling {UNCOVERED_CEILING}) — a fabric tile regressed off an op, \
         or a new op is unclassified",
        uncovered.len()
    );
    assert_eq!(
        uncovered.len() + fabric_realized_ops().len(),
        TOTAL_OPS,
        "covered + uncovered must PARTITION the whole ISA — no op unaccounted for"
    );
}

// ── ChanClose: a structural (Control) tile, so it carries no compute witness — but it is a scheduler-only
// op no LOGOS source emits, so this dedicated differential EXERCISES it: enqueue, dequeue, CLOSE, show the
// dequeued value. On the deterministic scheduler the close is output-transparent (the dequeued value is
// unaffected), so the fabric no-op tape must equal the RevCoop oracle. (rtl.rs proves the same for its RTL.)
#[test]
fn chan_close_is_an_output_transparent_no_op_matching_the_scheduler() {
    let mut st = 0xC105E_D00D_u64;
    for _ in 0..100 {
        let a = (mix(&mut st) % 20_000) as i64 - 10_000;
        let prog = CompiledProgram {
            constants: vec![Constant::Int(a)],
            code: vec![
                Op::ChanNew {
                    dst: 0,
                    cap: -1,
                    elem: ChanElem::Int,
                },
                Op::LoadConst { dst: 1, idx: 0 },
                Op::ChanSend { chan: 0, val: 1 }, // enqueue a
                Op::ChanRecv { dst: 2, chan: 0 }, // dequeue a
                Op::ChanClose { chan: 0 },        // close — output-transparent
                Op::Show { src: 2 },              // Show a
                Op::Halt,
            ],
            register_count: 3,
            ..Default::default()
        };
        let oracle = program_outcome_scheduled(&prog);
        assert!(
            oracle.error.is_none(),
            "ChanClose scheduler oracle error: {:?}",
            oracle.error
        );
        let want: Vec<String> = oracle.output.lines().map(str::to_string).collect();
        let wpf = whole_program_fabric_from_program(&prog);
        let got = eval_whole_program(&wpf);
        assert_eq!(
            got, want,
            "ChanClose fabric tape != scheduler oracle on ({a})"
        );
    }
}

// ── Offline networking: Connect/Listen/Send/Stream/Sync/MakePeer are structural (Control/Wire) tiles — in
// single-node offline mode they have no observable effect (Sync is offline-identity; MakePeer is an opaque
// handle passed only to no-ops). This differential runs source net programs through BOTH the offline net
// oracle (`vm_outcome_net`, set_net_offline) and the whole-program fabric, proving the fabric's no-op model
// equals the deterministic offline semantics (the same tw==VM==AOT model the AOT networking lock proves) —
// never "realized by ignoring". Each program's ops are handled by exec_ops (a miss would panic loudly).
const NET_CORPUS: &[&str] = &[
    // Connect + Listen — pure no-ops around a Show.
    "## Main\n    Connect to \"ws://127.0.0.1:7777\".\n    Listen on \"me\".\n    Show 42.\n",
    // Stream a list to a topic — no observable effect on output.
    "## Main\n    Connect to \"ws://127.0.0.1:7777\".\n    Listen on \"me\".\n    Let items be [1, 2, 3].\n    Stream items to \"sink\".\n    Show 7.\n",
    // Sync a Shared counter — OFFLINE identity: 5 + 3 = 8, the sync leaves it unchanged.
    "## A Counter is Shared and has:\n    A total: Int.\n\n## Main\n    Connect to \"ws://127.0.0.1:7777\".\n    Listen on \"me\".\n    Let mutable c be a new Counter with total 5.\n    Increase c's total by 3.\n    Sync c on \"room\".\n    Show c's total.\n",
    // MakePeer + Send — the peer handle is opaque, the send a no-op; output is independent of both.
    "## Main\n    Connect to \"ws://127.0.0.1:7777\".\n    Listen on \"me\".\n    Let p be a PeerAgent at \"friend\".\n    Send 9 to p.\n    Show 5.\n",
    // LOOPBACK: Send to self, then Await it back — the deterministic single-node round-trip. Listen
    // establishes the inbox, Send publishes 7, Await drains it into `got` = 7 ≡ the net oracle.
    "## Main\n    Listen on \"self\".\n    Let me_peer be a PeerAgent at \"self\".\n    Send 7 to me_peer.\n    Await response from me_peer into got.\n    Show got.\n",
];

#[test]
fn offline_networking_ops_match_the_net_oracle() {
    for src in NET_CORPUS {
        let oracle = vm_outcome_net(src);
        assert!(
            oracle.error.is_none(),
            "net oracle error on:\n{src}\n{:?}",
            oracle.error
        );
        let want: Vec<String> = oracle.output.lines().map(str::to_string).collect();
        let wpf = whole_program_fabric_from_source(src);
        let got = eval_whole_program(&wpf);
        assert_eq!(got, want, "offline-net fabric != net oracle for:\n{src}");
    }
}

// ── FailWith: a structural (Control) tile. The VM RETURNS an error at `FailWith`, so its output is exactly
// what it printed BEFORE the failure; the fabric halts its output tape there. This differential builds
// `Show a ; FailWith ; Show b` and asserts the fabric tape equals the VM's output-up-to-the-error (= [a]).
#[test]
fn fail_with_halts_the_output_tape_matching_the_vm() {
    let mut st = 0xFA11_ED_u64;
    for _ in 0..60 {
        let a = (mix(&mut st) % 20_000) as i64 - 10_000;
        let b = (mix(&mut st) % 20_000) as i64 - 10_000;
        let prog = CompiledProgram {
            constants: vec![
                Constant::Int(a),
                Constant::Int(b),
                Constant::Text("boom".into()),
            ],
            code: vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::Show { src: 0 }, // printed before the failure
                Op::FailWith { msg: 2 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::Show { src: 1 }, // never reached on the VM; suppressed on the fabric
                Op::Halt,
            ],
            register_count: 2,
            ..Default::default()
        };
        let mut vm = Vm::new(&prog);
        assert!(vm.run().is_err(), "FailWith should error the VM");
        let want: Vec<String> = vm.output().lines().map(str::to_string).collect();
        let wpf = whole_program_fabric_from_program(&prog);
        let got = eval_whole_program(&wpf);
        assert_eq!(
            got, want,
            "FailWith fabric tape != VM output-before-failure on ({a}, {b})"
        );
    }
}

// ── LoadNow / LoadToday: clock value-loads (Wire tiles). A combinational fabric has NO wall clock, so a
// deterministic differential injects a FIXED clock (`temporal::set_fixed_clock`) that BOTH the VM and the
// fabric read — the fabric bakes the same instant as a Const. The instant only becomes OBSERVABLE through a
// date-component extractor: `the hour of now` decodes nanos-of-day, `the weekday of today` decodes days,
// each a pure div/mod the fabric computes bit-identically to the VM's `civil_from_unix_nanos` /
// `weekday_from_days`. Rendering the raw Moment/Date (a formatted date string) is a follow-on, as are the
// calendar fields (year/month/day, which need the full civil-from-days algorithm). Fuzzed across several
// positive instants (different hours, minutes, seconds, weekdays) — so the extraction, not one lucky value,
// is welded to the VM. This is the honest realization of a clock read: the fabric TRULY computes the field
// from the injected instant, never constant-folds the answer.
#[test]
fn clock_reads_flow_through_field_extraction_matching_the_vm() {
    use logicaffeine_compile::semantics::temporal::{clear_fixed_clock, set_fixed_clock};
    let ns = 1_000_000_000i64; // NANOS_PER_SECOND
    let day = 86_400 * ns; // NANOS_PER_DAY
                           // (days-since-epoch, nanos-since-epoch) — nanos = days*NANOS_PER_DAY + time-of-day, so `now` and `today`
                           // agree on the day (their weekday matches), spanning midnight, noon, and end-of-day time components.
    let clocks: [(i32, i64); 5] = [
        (19792, 19792 * day + (7 * 3600 + 30 * 60 + 15) * ns), // 2024-03-10 07:30:15 (a Sunday)
        (20000, 20000 * day + (23 * 3600 + 59 * 60 + 59) * ns),
        (18000, 18000 * day + 1 * ns), // 00:00:01
        (19000, 19000 * day + (12 * 3600 + 34 * 60 + 56) * ns),
        (0, 12 * 3600 * ns), // the epoch day, 1970-01-01 12:00:00 (a Thursday)
    ];
    let programs = [
        "## Main\nLet m be now.\nShow the hour of m.\n",
        "## Main\nLet m be now.\nShow the minute of m.\n",
        "## Main\nLet m be now.\nShow the second of m.\n",
        "## Main\nLet m be now.\nShow the weekday of m.\n",
        "## Main\nLet d be today.\nShow the weekday of d.\n",
    ];
    for (days, nanos) in clocks {
        for src in programs {
            // Inject the SAME instant into both engines, build both, THEN restore the clock. The fabric bakes
            // the instant into its cells at build, so evaluating after the restore still uses that instant.
            set_fixed_clock(days, nanos);
            let out = vm_outcome(src);
            let wpf = whole_program_fabric_from_source(src);
            clear_fixed_clock();
            assert!(
                out.error.is_none(),
                "vm error {:?} for clock ({days},{nanos}):\n{src}",
                out.error
            );
            let want: Vec<String> = out.output.lines().map(str::to_string).collect();
            let got = eval_whole_program(&wpf);
            assert_eq!(
                got, want,
                "clock field-extraction fabric != VM for clock ({days},{nanos}):\n{src}"
            );
        }
    }
}

// ── CheckPolicy: a structural (Control) tile, but a SECURITY-CRITICAL one — a no-op would be a security
// hole (it would let a DENIED program keep printing). The VM RETURNS an error at a failed `Check` (output
// is exactly what printed BEFORE the check); a GRANTED check is transparent. The fabric extracts each
// program's single-field policy condition at build and compiles a per-`Check` GRANT predicate that mirrors
// the VM's compare EXACTLY: `subject.field == n` for a Bool/Int field, a bounded char-list equality for a
// Text field (`role equals "admin"` — the common case), and a constant-0 (always-deny) for a literal the VM
// can't parse to the field type (`level equals 5` — the LOGOS policy parser doesn't capture the bare number,
// so the VM's `"unknown".parse::<i64>()` fails and the check ALWAYS denies; the fabric matches that rather
// than diverging). Every later `Show` carries the running AND of grants, and the tape HALTS at the first
// `Show` whose guard is false. Each program prints ONCE BEFORE the check and ONCE AFTER, so the tape pins
// the EXACT halt point: a no-op impl prints both lines (too much → RED), an always-deny impl prints neither
// (too little → RED) — only the correct conditional halt equals the VM's output-up-to-the-error. Grant AND
// deny are exercised across Bool, Text, and (always-deny) Int, welding every extraction path to the VM.
#[test]
fn check_policy_conditionally_halts_the_output_tape_matching_the_vm() {
    let bool_prog = |val: &str| {
        format!(
            "## Definition\nA User has:\n    a verified, which is Bool.\n\n\
             ## Policy\nA User is trusted if the user's verified equals true.\n\n\
             ## Main\nLet u be a new User with verified {val}.\n\
             Show \"before\".\nCheck that the u is trusted.\nShow \"after\".\n"
        )
    };
    // A Text field policy (`role equals "admin"`) — the common real case, exercising the char-list compare.
    let text_prog = |val: &str| {
        format!(
            "## Definition\nA User has:\n    a role, which is Text.\n\n\
             ## Policy\nA User is admin if the user's role equals \"admin\".\n\n\
             ## Main\nLet u be a new User with role \"{val}\".\n\
             Show \"before\".\nCheck that the u is admin.\nShow \"after\".\n"
        )
    };
    // An Int field with a numeric literal — the parser doesn't capture the number, so the VM ALWAYS denies.
    let int_prog = |val: i64| {
        format!(
            "## Definition\nA User has:\n    a level, which is Int.\n\n\
             ## Policy\nA User is senior if the user's level equals 5.\n\n\
             ## Main\nLet u be a new User with level {val}.\n\
             Show \"before\".\nCheck that the u is senior.\nShow \"after\".\n"
        )
    };

    let mut cases: Vec<String> = vec![bool_prog("true"), bool_prog("false")];
    // Text sweep: only "admin" grants — "guest" (same length, different bytes), "admi" (short), "admins"
    // (long), "ADMIN" (case) all deny, so the length + per-byte + case sensitivity of the compare are pinned.
    for role in ["admin", "guest", "admi", "admins", "ADMIN", "user"] {
        cases.push(text_prog(role));
    }
    // Int sweep straddling the "threshold": every case denies (the VM can't parse the literal), so the fabric
    // must always-deny too — a robustness lock against the divergence a naive refusal would introduce.
    for k in 0..12i64 {
        cases.push(int_prog(k));
    }

    for src in &cases {
        // The VM is the oracle: on GRANT `error` is None and both lines print; on DENY the VM returns an
        // Err at the check, so `output` is exactly the pre-check line(s). Either way the fabric must match.
        let out = vm_outcome(src);
        let want: Vec<String> = out.output.lines().map(str::to_string).collect();
        let wpf = whole_program_fabric_from_source(src);
        let got = eval_whole_program(&wpf);
        assert_eq!(
            got, want,
            "CheckPolicy fabric tape != VM output-before-error for:\n{src}"
        );
    }

    // Belt-and-suspenders: assert the DENY cases genuinely SUPPRESS the post-check line (not merely that the
    // tapes agree) — the would-be security hole is a denied program that keeps printing. Each must be EXACTLY
    // ["before"], never ["before", "after"]; each GRANT must be transparent (both lines survive).
    let deny_expected = vec!["before".to_string()];
    let grant_expected = vec!["before".to_string(), "after".to_string()];
    for denied_src in [bool_prog("false"), text_prog("guest"), int_prog(5)] {
        let denied = eval_whole_program(&whole_program_fabric_from_source(&denied_src));
        assert_eq!(
            denied, deny_expected,
            "SECURITY: a DENIED Check must halt the tape — the post-check Show leaked through:\n{denied_src}"
        );
    }
    for grant_src in [bool_prog("true"), text_prog("admin")] {
        let granted = eval_whole_program(&whole_program_fabric_from_source(&grant_src));
        assert_eq!(
            granted, grant_expected,
            "a GRANTED Check must be transparent:\n{grant_src}"
        );
    }
}

// ── CLOSURES (MakeClosure/CallValue): structural tiles. `MakeClosure` captures a free variable into a
// tracked closure; `CallValue` resolves the closure's STATIC body and inlines it with the args seeding the
// params and the captures seeding the capture frame. This differential hand-builds a closure that captures a
// local `cap`, calls it with `arg`, and returns `arg + cap*2` (using BOTH the param and the capture) — the
// fabric's Futamura dispatch ≡ the VM's closure call.
#[test]
fn closures_capture_and_dispatch_matching_the_vm() {
    let body = CompiledFunction {
        name: Symbol::from_index(0),
        entry_pc: 6,
        param_count: 1,
        register_count: 6,
        captures: vec![(Symbol::from_index(0), None)], // one local capture
        named_regs: vec![],
        param_kinds: vec![],
        ret_kind: None,
        param_types: vec![],
        return_type: None,
        mutable_param_regs: vec![],
        cow_exempt_regs: vec![],
    };
    let mut st = 0xC105_11_u64;
    for _ in 0..120 {
        let cap = (mix(&mut st) % 4000) as i64 - 2000;
        let arg = (mix(&mut st) % 4000) as i64 - 2000;
        let prog = CompiledProgram {
            constants: vec![Constant::Int(cap), Constant::Int(arg), Constant::Int(2)],
            code: vec![
                Op::LoadConst { dst: 0, idx: 0 }, // r0 = cap (captured local)
                Op::MakeClosure {
                    dst: 1,
                    func: 0,
                    locals_start: 0,
                },
                Op::LoadConst { dst: 2, idx: 1 }, // r2 = arg
                Op::CallValue {
                    dst: 3,
                    callee: 1,
                    args_start: 2,
                    arg_count: 1,
                    name_for_err: u32::MAX,
                },
                Op::Show { src: 3 },
                Op::Halt,
                // [6] body: r0=arg (param), r1=cap (capture), r2=flag → arg + cap*2
                Op::LoadConst { dst: 3, idx: 2 },
                Op::Mul {
                    dst: 4,
                    lhs: 1,
                    rhs: 3,
                },
                Op::Add {
                    dst: 5,
                    lhs: 0,
                    rhs: 4,
                },
                Op::Return { src: 5 },
            ],
            register_count: 6,
            functions: vec![body.clone()],
            ..Default::default()
        };
        let (got, want) = hand_bytecode_tapes_prog(prog);
        assert_eq!(got, want, "closure fabric != VM on (cap={cap}, arg={arg})");
    }
}

// ── Obs: an observability event that feeds a trace sink (off by default), NEVER stdout. So it has no effect
// on the program's output tape — a structural no-op. This differential interposes an `Obs` between two
// `Show`s and asserts the output is unchanged (= the VM's, which no-ops `Obs` when nothing is capturing).
#[test]
fn obs_is_a_no_op_on_the_output_tape_matching_the_vm() {
    let mut st = 0x0B5_ED_u64;
    for _ in 0..60 {
        let a = (mix(&mut st) % 20_000) as i64 - 10_000;
        let b = (mix(&mut st) % 20_000) as i64 - 10_000;
        let (got, want) = hand_bytecode_tapes(
            vec![Constant::Int(a), Constant::Int(b)],
            vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::Show { src: 0 },
                Op::Obs {
                    site: 0,
                    args_start: 0,
                    args_count: 0,
                }, // no sink capturing → no-op
                Op::LoadConst { dst: 1, idx: 1 },
                Op::Show { src: 1 },
                Op::Halt,
            ],
            2,
        );
        assert_eq!(got, want, "Obs changed the output tape on ({a}, {b})");
    }
}

// ── The SCHEDULER (spatial coroutine): Spawn/SpawnHandle/TaskAwait/TaskAbort are structural (Control) tiles.
// Spawn INLINES a producer child's straight-line side effects at the spawn point — a child that enqueues to
// a channel the parent then drains yields the SAME observable as the cooperative schedule (parent blocks on
// the empty channel, child runs, parent resumes). TaskAwait joins instantly (the child already ran) and
// TaskAbort is a no-op. No LOGOS source emits these on the fabric path, so — like rtl.rs's soft-core oracle —
// the witness is a hand-built program driven through the RevCoop scheduler (`program_outcome_scheduled`).
fn scheduler_child(entry_pc: usize, param_count: u16) -> CompiledFunction {
    CompiledFunction {
        name: Symbol::from_index(0),
        entry_pc,
        param_count,
        register_count: 4,
        captures: vec![],
        named_regs: vec![],
        param_kinds: vec![],
        ret_kind: None,
        param_types: vec![],
        return_type: None,
        mutable_param_regs: vec![],
        cow_exempt_regs: vec![],
    }
}

#[test]
fn the_spawn_scheduler_ops_match_the_cooperative_schedule() {
    let mut st = 0x59A_C0DE_u64;
    for _ in 0..60 {
        let a = (mix(&mut st) % 20_000) as i64 - 10_000;
        // Spawn: a producer child enqueues `a`; the parent dequeues + shows it.
        let producer = CompiledProgram {
            constants: vec![Constant::Int(a)],
            code: vec![
                Op::ChanNew {
                    dst: 0,
                    cap: -1,
                    elem: ChanElem::Int,
                },
                Op::LoadConst { dst: 1, idx: 0 },
                Op::Spawn {
                    func: 0,
                    args_start: 0,
                    arg_count: 2,
                },
                Op::ChanRecv { dst: 2, chan: 0 },
                Op::Show { src: 2 },
                Op::Halt,
                Op::ChanTrySend {
                    dst: 2,
                    chan: 0,
                    val: 1,
                }, // child: enqueue r1 into r0
                Op::ReturnNothing,
            ],
            register_count: 3,
            functions: vec![scheduler_child(6, 2)],
            ..Default::default()
        };
        // SpawnHandle + TaskAwait: a child that just returns; the join passes control through to `Show a`.
        let joiner = CompiledProgram {
            constants: vec![Constant::Int(a)],
            code: vec![
                Op::SpawnHandle {
                    dst: 0,
                    func: 0,
                    args_start: 0,
                    arg_count: 0,
                },
                Op::TaskAwait { dst: 1, handle: 0 },
                Op::LoadConst { dst: 2, idx: 0 },
                Op::Show { src: 2 },
                Op::Halt,
                Op::ReturnNothing,
            ],
            register_count: 3,
            functions: vec![scheduler_child(5, 0)],
            ..Default::default()
        };
        // SpawnHandle + TaskAbort: aborting an already-complete child is a no-op; `Show a` still runs. The
        // reversible RevCoop scheduler cannot reverse `TaskAbort`, so this one uses the FULL scheduler oracle.
        let aborter = CompiledProgram {
            constants: vec![Constant::Int(a)],
            code: vec![
                Op::SpawnHandle {
                    dst: 0,
                    func: 0,
                    args_start: 0,
                    arg_count: 0,
                },
                Op::TaskAbort { handle: 0 },
                Op::LoadConst { dst: 2, idx: 0 },
                Op::Show { src: 2 },
                Op::Halt,
                Op::ReturnNothing,
            ],
            register_count: 3,
            functions: vec![scheduler_child(5, 0)],
            ..Default::default()
        };
        // `full` picks the full cooperative scheduler (TaskAbort) vs the reversible RevCoop (the rest).
        let checks: [(&str, CompiledProgram, bool); 3] = [
            ("producer", producer, false),
            ("joiner", joiner, false),
            ("aborter", aborter, true),
        ];
        for (tag, prog, full) in checks {
            let oracle = if full {
                program_outcome_concurrent(&prog)
            } else {
                program_outcome_scheduled(&prog)
            };
            assert!(
                oracle.error.is_none(),
                "{tag} scheduler oracle error: {:?}",
                oracle.error
            );
            let want: Vec<String> = oracle.output.lines().map(str::to_string).collect();
            let wpf = whole_program_fabric_from_program(&prog);
            let got = eval_whole_program(&wpf);
            assert_eq!(
                got, want,
                "scheduler {tag} fabric != cooperative schedule on ({a})"
            );
        }
    }
}

// ── RegionBoundsGuard: a structural (Control) tile with no data effect (the VM runs it as `pc += 1`). No
// source emits it on the no-oracle fabric path, so this differential hand-builds a program that computes
// `a + b`, interposes the guard, and shows the sum — the fabric no-op tape must equal the VM.
#[test]
fn region_bounds_guard_is_a_no_op_matching_the_vm() {
    let mut st = 0xB0DD_6A6D_u64;
    for _ in 0..100 {
        let a = (mix(&mut st) % 4000) as i64 - 2000;
        let b = (mix(&mut st) % 4000) as i64 - 2000;
        let (got, want) = hand_bytecode_tapes(
            vec![Constant::Int(a), Constant::Int(b)],
            vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::Add {
                    dst: 2,
                    lhs: 0,
                    rhs: 1,
                },
                Op::RegionBoundsGuard {
                    array: 0,
                    bound: 1,
                    iv: 2,
                    add_max: 0,
                    add_min: 0,
                },
                Op::Show { src: 2 },
                Op::Halt,
            ],
            3,
        );
        assert_eq!(
            got, want,
            "RegionBoundsGuard changed the result on ({a}, {b})"
        );
    }
}

// ── STAGE X5 (XPU): the HARD BOUNDARY lock. The fabric realizes the whole 115-op ISA + the NUMERIC builtins
// (Sqrt/Floor/Abs/Min/Max), and Stages X1–X4 proved every tile TYPE (combinational Func/division, the state
// plane, the float lane, control flow) places + evaluates on the reconfigurable grid — so the reversible-ALU,
// CRDT, and channel op-classes (Alu/Mem tiles) are grid-realized by construction. But a reconfigurable compute
// tile is the WRONG home for the heavy CRYPTOGRAPHIC / COMPRESSION / erasure-coding / DOM / real-IO builtins:
// those are software or DEDICATED accelerators (a Keccak/NTT/PQC block per MAKE_A_CHIP §4), NOT grid tiles.
// This lock PINS that edge — the fabric lowering REFUSES each boundary builtin LOUDLY (it panics rather than
// silently emitting a wrong/no-op tile; a silent no-op for `seal` would be a security hole), while the numeric
// builtins lower cleanly. So the "everything on the fabric" claim can never silently overrun its honest
// boundary. This is the pit-of-success ratchet for the XPU's edge (rule 14).
#[test]
fn the_fabric_refuses_the_hard_boundary_builtins() {
    use logicaffeine_compile::codegen_sva::fabric_rtl::whole_program_fabric_from_program;

    // A minimal program: load one arg, call the builtin, show, halt. The fabric refuses at the builtin match,
    // so the (type-wrong) integer arg is irrelevant — the point under test is the REFUSAL, not a computation.
    let build = |builtin: BuiltinId| CompiledProgram {
        constants: vec![Constant::Int(0)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::CallBuiltin {
                dst: 1,
                builtin,
                args_start: 0,
                arg_count: 1,
            },
            Op::Show { src: 1 },
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };

    // The hard boundary: cryptography, compression, erasure coding, and DOM/real-IO — NOT reconfigurable tiles.
    let boundary = [
        BuiltinId::SealBytes,
        BuiltinId::OpenBytes,
        BuiltinId::CompressBytes,
        BuiltinId::DecompressBytes,
        BuiltinId::ReedSolomonEncode,
        BuiltinId::FecProtect,
        BuiltinId::DomBody,
        BuiltinId::DomBeginPaint,
        BuiltinId::DomSetChecked,
        BuiltinId::DomApplyPatches,
        BuiltinId::DomApplyWords,
        BuiltinId::DomWakeAfter,
    ];

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {})); // silence the expected-panic backtraces
    for b in boundary {
        let prog = build(b);
        let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = whole_program_fabric_from_program(&prog);
        }))
        .is_err();
        assert!(
            refused,
            "the fabric must REFUSE the hard-boundary builtin {b:?} (a silent tile would overclaim / be a \
             security hole) — it belongs on the soft-core or a dedicated accelerator, not a grid tile"
        );
    }
    // Positive control: a NUMERIC builtin (Abs) lowers WITHOUT panicking — the boundary is a genuine partition,
    // not an over-refusal that would also reject the realized numeric lane.
    let abs = build(BuiltinId::Abs);
    let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = whole_program_fabric_from_program(&abs);
    }))
    .is_ok();
    std::panic::set_hook(prev);
    assert!(ok, "a numeric builtin (Abs) must lower cleanly — the boundary must not over-refuse the realized lane");

    println!("Stage X5: the fabric REALIZES the 115-op ISA + numeric builtins but REFUSES the crypto/compression/FEC/DOM boundary builtins (software/accelerators, not grid tiles) — the honest XPU edge, locked");
}
