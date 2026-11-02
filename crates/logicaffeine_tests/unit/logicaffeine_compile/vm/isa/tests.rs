use super::*;
use crate::vm::instruction::Constant;

#[test]
fn every_isa_instruction_carries_a_reversibility_contract() {
    // ★ The Tier-5 totality lock ★ — the emitter lowers EVERY opcode in the ISA and annotates each
    // with a reversibility class that is NEVER the unclassified `!!!!` sentinel (census ∅). Because
    // `reversibility` is exhaustive, a new op can't compile until it too is classified and emitted.
    for op in Op::all_variants() {
        let tag = rev_tag(&op);
        assert!(
            !tag.starts_with("!!!!"),
            "op `{}` reached the Silicon emitter unclassified: {tag}",
            op.name()
        );
        // And its mnemonic is non-empty (the emitter has a lowering for it).
        assert!(
            !mnemonic(&op).is_empty(),
            "op `{}` has no ISA mnemonic",
            op.name()
        );
    }
}

#[test]
fn abelian_core_emits_as_zero_garbage_adiabatic() {
    // The `Rev*` core must emit as REV zero-garbage — the adiabatic charge-recovery datapath.
    for op in [
        Op::RevAdd { place: 0, delta: 1 },
        Op::RevSub { place: 0, delta: 1 },
        Op::RevXor { place: 0, delta: 1 },
        Op::RevNeg { place: 0 },
        Op::RevSwap { a: 0, b: 1 },
    ] {
        assert!(
            rev_tag(&op).starts_with("REV"),
            "{} must be adiabatic",
            op.name()
        );
    }
    // A network send is the honest IRREVERSIBLE barrier, not zero-garbage.
    assert!(rev_tag(&Op::NetConnect { url: 0 }).contains("IRREVERSIBLE"));
}

#[test]
fn emits_a_full_program_listing() {
    let prog = CompiledProgram {
        constants: vec![Constant::Int(42)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::RevAdd { place: 0, delta: 1 },
            Op::Show { src: 0 },
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    let asm = emit_isa(&prog);
    assert!(asm.contains("Silicon ISA — Tier 5"));
    assert!(asm.contains("REVADD  r0, r1"));
    assert!(asm.contains("zero-garbage"));
    assert!(asm.contains("reversibility budget:"));
}

#[test]
fn cell_class_is_total_over_the_isa() {
    // Corner 5 (phase 4): every opcode maps to exactly one adiabatic CellClass — exhaustive via the
    // total `reversibility` classifier — and a Bijection is ALWAYS a charge-recovery cell.
    for op in Op::all_variants() {
        let _ = cell_class(&op); // total: never panics, always classified
    }
    for op in [
        Op::RevAdd { place: 0, delta: 1 },
        Op::RevSub { place: 0, delta: 1 },
        Op::RevXor { place: 0, delta: 1 },
        Op::RevNeg { place: 0 },
        Op::RevSwap { a: 0, b: 1 },
    ] {
        assert_eq!(
            cell_class(&op),
            CellClass::ChargeRecovery,
            "{} is adiabatic",
            op.name()
        );
    }
    assert_eq!(
        cell_class(&Op::Add {
            dst: 0,
            lhs: 1,
            rhs: 2
        }),
        CellClass::AncillaWrite
    );
    assert_eq!(cell_class(&Op::Show { src: 0 }), CellClass::EffectLog);
    assert_eq!(cell_class(&Op::Halt), CellClass::ControlTrace);
}

#[test]
fn adiabatic_schedule_agrees_with_the_budget_and_the_listing() {
    let prog = CompiledProgram {
        constants: vec![Constant::Int(5), Constant::Int(3)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 1 },
            Op::RevAdd { place: 0, delta: 1 },
            Op::Show { src: 0 },
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    let sched = adiabatic_schedule(&prog);
    assert_eq!(sched.charge_recovery, 1, "one RevAdd charge-recovery cell");
    assert_eq!(sched.ancilla, 2, "two LoadConst ancilla cells");
    assert_eq!(sched.effect, 1, "one Show effect cell");
    assert_eq!(sched.control, 1, "one Halt control cell");
    assert!(!sched.is_fully_adiabatic(), "it has ancilla + effect cells");
    let asm = emit_isa(&prog);
    assert!(
        asm.contains(".adiabatic"),
        "the listing carries the adiabatic cell budget"
    );
    assert!(
        asm.contains("charge-recovery cells: 1"),
        "and reports the charge-recovery count"
    );
}

#[test]
fn a_fully_adiabatic_program_reverses_at_the_landauer_floor() {
    // The tie between the STATIC cell schedule and the DYNAMIC Landauer floor: a program whose only
    // information-bearing writes are zero-erasure (a LoadConst overwriting the pristine `Nothing`)
    // plus a charge-recovery RevAdd runs at `landauer_bytes == 0` — the adiabatic floor the
    // schedule's charge-recovery core promises.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(5), Constant::Int(3)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 1 },
            Op::RevAdd { place: 0, delta: 1 },
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    assert_eq!(
        adiabatic_schedule(&prog).charge_recovery,
        1,
        "the RevAdd is a charge-recovery cell"
    );
    let mut vm = crate::vm::Vm::new(&prog);
    vm.run_recording().expect("reversible");
    let tape = vm.take_journal().expect("a recording leaves a tape");
    assert_eq!(
        tape.landauer_bytes(),
        0,
        "the charge-recovery core reverses at the Landauer floor"
    );
}

#[test]
fn a_purely_bijective_program_is_statically_fully_adiabatic() {
    // A datapath of only Bijection + Control cells is FULLY ADIABATIC — no ancilla, no effect — so
    // no cell can erase a bit. The static contract is exact: this shape's Landauer floor is 0.
    let prog = CompiledProgram {
        constants: vec![],
        code: vec![
            Op::RevAdd { place: 0, delta: 1 },
            Op::RevSwap { a: 0, b: 1 },
            Op::RevXor { place: 0, delta: 1 },
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    let sched = adiabatic_schedule(&prog);
    assert!(
        sched.is_fully_adiabatic(),
        "all-Bijection/Control ⇒ fully adiabatic"
    );
    assert_eq!(sched.ancilla, 0, "no history cells");
    assert_eq!(sched.charge_recovery, 3, "three charge-recovery cells");
    assert_eq!(
        sched.charge_recovery_fraction(),
        0.75,
        "3 of 4 instructions charge-recover"
    );
}

/// The count of ops in the fabricated REVERSIBLE datapath (`rev_alu_core.v`, wire opcodes 0..4).
/// A MONOTONE FLOOR — never lower it.
const CORE_DATAPATH_FLOOR: usize = 5;

/// The count of ops REALIZED in verified silicon — the reversible core PLUS the general integer
/// ALU (`emit_alu_datapath_verilog`), each op proven correct by our own certified SAT. A MONOTONE
/// FLOOR: RAISE it as each `Datapath` op gets synthesizable RTL + an our-SAT correctness proof;
/// NEVER lower it. This is the "ratchet toward the full surface" progress counter.
/// 5 reversible core + 13 general-ALU ops + `AddAssign` + `Shl`/`Shr` (shift unit) + `Div`/`Mod`
/// (signed restoring divider) + `FloorDiv` + `DivPow2` + `MagicDivU` + `Pow` = 27 datapath ops, plus
/// the 7 soft-core sequencer/movement ops (Move, LoadConst, Jump, JumpIfFalse, JumpIfTrue, Halt, Show)
/// + the function-call trio (Call, Return, ReturnNothing — the register-window frame stack) = 37.
/// 116: `NewEmptyListFixed` runs on the soft core as the empty list it is observably.
const SILICON_REALIZED_FLOOR: usize = 116;

/// FULL-SURFACE SILICON RATCHET. Ensures the chip covers the WHOLE language: every one of the ISA's
/// ops (1) has a silicon home (the exhaustive `silicon_status` — nothing unimplementable), (2) is
/// WIRE-DECODABLE (a stable `binisa` opcode a chip receives), and (3) the realized reversible core
/// datapath is exactly the pinned wire opcodes 0..4 and matches the monotone floor. A new op cannot
/// compile without a home, and cannot escape a wire opcode — so the chip can never silently drop a
/// language feature. Growing the fabricated datapath raises the floor; this lock records the frontier.
#[test]
fn silicon_op_support_ratchet() {
    use crate::vm::binisa;
    let ops = Op::all_variants();

    // (1) Full surface: every op maps to exactly one silicon home (census — nothing unsupported).
    let mut homed = 0usize;
    for op in &ops {
        homed += match silicon_status(op) {
            SiliconStatus::CoreDatapath
            | SiliconStatus::Datapath
            | SiliconStatus::Sequencer
            | SiliconStatus::LoadStore
            | SiliconStatus::Coprocessor => 1,
        };
    }
    assert_eq!(homed, ops.len(), "every ISA op must have a silicon home");

    // (2) Every op is WIRE-DECODABLE — a chip can receive every instruction the language emits.
    for op in &ops {
        assert!(
            binisa::opcode_of_name(op.name()).is_some(),
            "op `{}` has no wire opcode — a chip could not receive it",
            op.name()
        );
    }

    // (3) The realized reversible core datapath == the monotone floor, IS the abelian bijection set,
    //     and IS exactly the pinned wire opcodes 0..4 (== the `rev_alu_core` control-selects).
    let core: Vec<&Op> = ops
        .iter()
        .filter(|o| silicon_status(o) == SiliconStatus::CoreDatapath)
        .collect();
    assert_eq!(
        core.len(),
        CORE_DATAPATH_FLOOR,
        "core datapath size changed — RAISE the floor as RTL grows, never lower it"
    );
    for op in &core {
        assert!(
            matches!(reversibility(op), RevClass::Bijection(_)),
            "core-datapath op `{}` must be a proven bijection (adiabatic, zero-garbage)",
            op.name()
        );
    }
    let mut codes: Vec<u16> = core.iter().map(|o| binisa::opcode(o)).collect();
    codes.sort_unstable();
    assert_eq!(
        codes,
        vec![0, 1, 2, 3, 4],
        "the realized core datapath is wire opcodes 0..4"
    );

    // (4) REALIZATION RATCHET — the count of ops realized in verified silicon (reversible core +
    //     general ALU) is a monotone floor that rises as ops are proven into RTL, never falls.
    let realized: Vec<&Op> = ops.iter().filter(|o| silicon_realized(o)).collect();
    assert_eq!(
        realized.len(),
        SILICON_REALIZED_FLOOR,
        "realized-silicon floor changed — RAISE it as ops are verified into RTL, never lower"
    );
    // Every realized op is either a datapath op (reversible core / general ALU, our-SAT-proven), a
    // soft-core-executed sequencer/movement/output op (the CPU runs it, `soft_core_executes`), OR a
    // load-store op realized in the heap unit (`load_store_realized`, ≡VM differential). Nothing else —
    // a not-yet-realized coprocessor op cannot be counted in the floor.
    for op in &realized {
        assert!(
            matches!(silicon_status(op), SiliconStatus::CoreDatapath | SiliconStatus::Datapath)
                || soft_core_executes(op)
                || load_store_realized(op)
                || coprocessor_realized(op),
            "realized op `{}` must be a datapath op, soft-core-executed, load-store-realized, or coprocessor-realized, not {:?}",
            op.name(),
            silicon_status(op)
        );
        // And it is wire-decodable, so a chip both RECEIVES it and EXECUTES it in verified hardware.
        assert!(binisa::opcode_of_name(op.name()).is_some());
    }
    // The reversible core is a subset of the realized set (it's realized too).
    assert!(
        core.iter().all(|o| silicon_realized(o)),
        "the reversible core is realized silicon"
    );
    assert!(realized.len() >= core.len());
}

/// The SEQUENCER analog of the ALU wire↔silicon keystone: the soft-core RTL artifact
/// (`benchmarks/silicon/soft_core.v`) decodes each movement/control op at its EXACT `binisa` wire
/// opcode — so the byte the network carries is the byte the CPU's decoder acts on, for control flow
/// too. Ties the checked-in RTL to the ISA; a drift in either fails loud.
#[test]
fn soft_core_decodes_the_wire_opcodes() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/silicon/soft_core.v");
    let rtl = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read soft_core.v at {path:?}: {e}"));
    for name in [
        // the reversible/adiabatic core (opcodes 0..4) — now decoded and executed by the integrated CPU
        "RevAdd",
        "RevSub",
        "RevXor",
        "RevNeg",
        "RevSwap",
        "LoadConst",
        "Move",
        "Jump",
        "JumpIfFalse",
        "JumpIfTrue",
        "Show",
        "Halt",
        "Call",
        "Return",
        "ReturnNothing",
        "CallValue",
        "MakeClosure",
        "FailWith",
        // load-store unit — the heap ops the CPU decodes into `dmem`/`hp` reads and writes
        "NewList",
        "NewEmptyList",
        "NewEmptyListI32",
        "NewEmptyMap",
        "ListPush",
        "Index",
        "IndexUnchecked",
        "Length",
        "NewTuple",
        "ListPop",
        "Contains",
        "DestructureTuple",
        "NewStruct",
        "StructInsert",
        "GetField",
        "FormatValue",
        "SetIndex",
        "SetIndexUnchecked",
        "SeqConcat",
        "Concat",
        "DeepClone",
        "IterPrepare",
        "IterNext",
        "IterPop",
        "NewInductive",
        "TestArm",
        "BindArm",
        "NewEmptySet",
        "SetAdd",
        "NewRange",
        "UnionOp",
        "IntersectOp",
        "RemoveFrom",
        "SliceOp",
        "ListPushField",
        "GlobalGet",
        "GlobalSet",
        "ChanNew",
        "ChanSend",
        "ChanRecv",
        "Spawn",
        "CrdtBump",
        "CrdtMerge",
        "CrdtJoin",
        "BigIntWide",
        "NewCrdt",
        "CrdtAppend",
        "CrdtResolve",
        "SelectArmRecv",
        "SelectArmTimeout",
        "SelectWait",
        "ChanTrySend",
        "ChanTryRecv",
        "Sleep",
        "LoadToday",
        "LoadNow",
        "EnsureOwned",
        "RegionBoundsGuard",
        "Obs",
        "SpawnHandle",
        "TaskAbort",
        "TaskAwait",
        "ChanClose",
        "CallBuiltin",
        "Args",
        "NetConnect",
        "NetListen",
        "NetSend",
        "NetStream",
        "NetAwait",
        "NetMakePeer",
        "NetSync",
        "CheckPolicy",
    ] {
        let oc = crate::vm::binisa::opcode_of_name(name)
            .unwrap_or_else(|| panic!("{name} has a wire opcode"));
        // A `case` label ends in `:` when alone or `,` when it heads a shared arm (`8'd59, 8'd60:`).
        assert!(
            rtl.contains(&format!("8'd{oc}:")) || rtl.contains(&format!("8'd{oc},")),
            "soft_core.v must decode {name} at its wire opcode {oc}"
        );
    }
}
