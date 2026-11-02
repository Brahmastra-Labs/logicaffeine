//! R1 behavioural lock — `forward ; reverse ≡ identity`. A recording forward run captures the
//! minimal destroyed-bit tape; [`Vm::run_reverse`] consumes it to reconstruct the EXACT
//! pre-run state (registers + globals + output), bit-for-bit. Straight-line scalar subset;
//! control flow / collections / effects extend the subset in R2/R3.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS FILE TO MAKE A RED CASE PASS.  ⚠️
//!  A RED means the reverse executor lost a bit — the fix is in `record_op`/`run_reverse`, never
//!  by relaxing the identity assertion.
use super::*;

/// The full observable machine state, formatted bit-exactly (`Value`/`Option<Value>` via `Debug`
/// round-trip their scalar contents). The comparator for `forward ; reverse ≡ identity`.
fn observe_state(s: &super::DebugVmState) -> String {
    format!(
        "regs={:?} base={} globals={:?} lines={:?} iters={:?}",
        s.registers, s.base, s.globals, s.lines, s.iter_stack
    )
}

/// The observable (named-register) projection of a `DebugVmState` — used where a program sets
/// `named_regs` and elision may leave scratch registers unrestored (concurrency elision).
fn observe_named(s: &super::DebugVmState, named: &[bool]) -> String {
    let regs: Vec<&Value> = s
        .registers
        .iter()
        .enumerate()
        .filter(|(i, _)| named.is_empty() || named.get(*i).copied().unwrap_or(true))
        .map(|(_, v)| v)
        .collect();
    format!(
        "regs={:?} globals={:?} lines={:?}",
        regs, s.globals, s.lines
    )
}

#[test]
fn concurrency_scheduler_elides_task_scratch_and_reverses() {
    // PIECE C — elision on the RevCoop concurrency scheduler. Each task VM records under the
    // Observable pebble plan (tasks have independent register frames ⇒ per-task elision is sound).
    // Here Main's `Mul` temporary is pure scratch that no reverse step reads — it is ELIDED — and
    // the main NAMED state still reverses through the channel round-trip.
    use super::super::reverse::ReverseMode;
    let prog = CompiledProgram {
        constants: vec![Constant::Int(7)],
        code: vec![
            Op::ChanNew {
                dst: 0,
                cap: -1,
                elem: crate::vm::ChanElem::Int,
            }, // r0 = channel
            Op::LoadConst { dst: 1, idx: 0 }, // r1 = 7 (named)
            Op::Mul {
                dst: 2,
                lhs: 1,
                rhs: 1,
            }, // r2 = 49 (SCRATCH — elided)
            Op::ChanSend { chan: 0, val: 1 }, // send 7
            Op::ChanRecv { dst: 3, chan: 0 }, // r3 = 7 (named)
            Op::Halt,
        ],
        register_count: 4,
        named_regs: vec![true, true, false, true], // r2 is scratch
        ..Default::default()
    };
    // The scheduler's per-task plan elides the scratch.
    let report = super::super::pebble::report(&prog, ReverseMode::Observable);
    assert!(
        report.elided > 0,
        "the scheduler's task plan elides scratch"
    );

    // And the whole thing reverses through the scheduler — main named state restored.
    let named = &prog.named_regs;
    let initial = observe_named(&Vm::new(&prog).save_debug_state(), named);
    let mut coop = RevCoop::record(Vm::new(&prog)).expect("concurrency program is reversible");
    coop.reverse().expect("reverse succeeds");
    assert_eq!(
        initial,
        observe_named(&coop.main_state(), named),
        "main named state reverses through the scheduler WITH per-task elision"
    );
    assert_eq!(coop.channel_count(), 0, "the channel is reversed away");
}

#[test]
fn concurrency_scheduler_recomputes_task_scratch_under_adiabatic() {
    // ★ CORNER 3 — concurrency Adiabatic ★. Each task VM now records under an explicit ReverseMode.
    // Under `Adiabatic`, a task's reverse-live scratch (`t = a+b`, a `RevAdd` delta) is RECOMPUTED
    // on reverse instead of journaled — the concurrent tape drops below Observable — and every
    // register still reverses bit-exact through the channel round-trip.
    use super::super::reverse::ReverseMode;
    let prog = CompiledProgram {
        constants: vec![Constant::Int(3), Constant::Int(4), Constant::Int(10)],
        code: vec![
            Op::ChanNew {
                dst: 0,
                cap: -1,
                elem: crate::vm::ChanElem::Int,
            }, // r0 = channel
            Op::LoadConst { dst: 1, idx: 0 }, // a = 3 (named)
            Op::LoadConst { dst: 2, idx: 1 }, // b = 4 (named)
            Op::LoadConst { dst: 3, idx: 2 }, // place = 10 (named)
            Op::Add {
                dst: 4,
                lhs: 1,
                rhs: 2,
            }, // t = a+b = 7 (r4 SCRATCH, reverse-live)
            Op::RevAdd { place: 3, delta: 4 }, // place += t = 17
            Op::ChanSend { chan: 0, val: 1 }, // send a
            Op::ChanRecv { dst: 5, chan: 0 }, // r5 = 3 (named)
            Op::Halt,
        ],
        register_count: 6,
        named_regs: vec![true, true, true, true, false, true], // r4 scratch (reverse-live)
        ..Default::default()
    };
    let tape_bytes = |mode| {
        RevCoop::record_mode(Vm::new(&prog), mode)
            .expect("reversible")
            .main_erased_bytes()
    };
    assert!(
        tape_bytes(ReverseMode::Adiabatic) < tape_bytes(ReverseMode::Observable),
        "Adiabatic recomputes the task's reverse-live scratch below Observable ({} vs {})",
        tape_bytes(ReverseMode::Adiabatic),
        tape_bytes(ReverseMode::Observable),
    );
    // Full fidelity through the scheduler: EVERY main register reverses (scratch via recompute).
    let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
    let mut coop =
        RevCoop::record_mode(Vm::new(&prog), ReverseMode::Adiabatic).expect("reversible");
    coop.reverse().expect("reverse succeeds");
    assert_eq!(
        all_initial,
        observe_state(&coop.main_state()),
        "Adiabatic reverses every task register through the scheduler"
    );
    assert_eq!(coop.channel_count(), 0, "the channel is reversed away");
}

fn observable(vm: &Vm) -> String {
    // The reverse CONTRACT is over the program-OBSERVABLE state: named user variables (plus
    // globals, output, iterators). Compiler scratch is program-invisible, so under pebbling it
    // need not be restored. Empty `named_regs` (hand-built bytecode) ⇒ compare ALL registers —
    // nothing is elided there, so the reverse executor's full-fidelity tests are unchanged.
    let s = vm.save_debug_state();
    let named = &vm.program.named_regs;
    let regs: Vec<&Value> = s
        .registers
        .iter()
        .enumerate()
        .filter(|(i, _)| named.is_empty() || named.get(*i).copied().unwrap_or(true))
        .map(|(_, v)| v)
        .collect();
    format!(
        "regs={:?} base={} globals={:?} lines={:?} iters={:?}",
        regs, s.base, s.globals, s.lines, s.iter_stack
    )
}

fn roundtrip(prog: &CompiledProgram) {
    let initial = observable(&Vm::new(prog));

    let mut vm = Vm::new(prog);
    vm.run_recording()
        .expect("program is in the reversible subset");
    let after_forward = observable(&vm);
    assert_ne!(
        initial, after_forward,
        "the forward run must actually change state"
    );

    let tape = vm.take_journal().expect("run_recording leaves a tape");
    assert!(
        !tape.is_empty(),
        "a journaled (non-abelian) program records deltas"
    );
    assert!(
        tape.erased_bytes() > 0,
        "value overwrites erase real data onto the tape"
    );

    vm.run_reverse(tape).expect("reverse succeeds");
    let after_reverse = observable(&vm);
    assert_eq!(
        initial, after_reverse,
        "forward ; reverse must reconstruct the EXACT initial state"
    );
}

#[test]
fn scalar_registers_and_output_roundtrip() {
    // LoadConst / Add / Mul into fresh registers + one Show — reverse restores every register to
    // Nothing and truncates the output back to empty.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(3), Constant::Int(4)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 3
            Op::LoadConst { dst: 1, idx: 1 }, // R1 = 4
            Op::Add {
                dst: 2,
                lhs: 0,
                rhs: 1,
            }, // R2 = 7
            Op::Mul {
                dst: 3,
                lhs: 2,
                rhs: 0,
            }, // R3 = 21
            Op::Show { src: 3 },              // output "21"
            Op::Halt,
        ],
        register_count: 4,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn global_write_and_read_roundtrip() {
    // GlobalSet (first-definition, old == None) + GlobalGet — reverse un-defines the global and
    // restores the register.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(10)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 10
            Op::GlobalSet { idx: 0, src: 0 }, // g0 = 10  (was undefined)
            Op::GlobalGet { dst: 1, idx: 0 }, // R1 = g0
            Op::Halt,
        ],
        register_count: 2,
        globals: vec!["x".to_string()],
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn in_place_overwrite_roundtrips() {
    // Overwrite the SAME register twice (`AddAssign`) — the tape must restore the intermediate,
    // not just the initial, so the LIFO discipline is load-bearing.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(5), Constant::Int(2)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 5
            Op::LoadConst { dst: 1, idx: 1 }, // R1 = 2
            Op::AddAssign { dst: 0, src: 1 }, // R0 = 7  (destroys the 5)
            Op::AddAssign { dst: 0, src: 1 }, // R0 = 9  (destroys the 7)
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn swap_reverses_with_zero_garbage() {
    // `RevSwap` is a bijection: it reverses by self-application with NO history-tape entry — the
    // adiabatic zero-erasure signature. Only the two setup `LoadConst`s journal a delta; the two
    // swaps add nothing, yet `forward ; reverse` still restores the exact initial state.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(5), Constant::Int(9)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 5   (journals)
            Op::LoadConst { dst: 1, idx: 1 }, // R1 = 9   (journals)
            Op::RevSwap { a: 0, b: 1 },       // R0=9 R1=5 (zero garbage)
            Op::RevSwap { a: 0, b: 1 },       // R0=5 R1=9 (zero garbage)
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };

    let initial = observable(&Vm::new(&prog));
    let mut vm = Vm::new(&prog);
    vm.run_recording().expect("swap program is reversible");
    let tape = vm.take_journal().expect("tape present");
    // The load-bearing zero-erasure claim: the two swaps recorded NOTHING — only the two loads.
    assert_eq!(
        tape.len(),
        2,
        "the two RevSwaps must contribute ZERO tape entries"
    );
    vm.run_reverse(tape).expect("reverse succeeds");
    assert_eq!(
        initial,
        observable(&vm),
        "forward ; reverse restores the exact initial state"
    );
}

#[test]
fn xor_reverses_with_zero_garbage() {
    // `RevXor` is 𝔽₂ⁿ self-inverse: `R[place] ^= R[delta]` reverses by re-application with NO
    // tape entry. `R[delta]` is restored to its forward value by the reverse-order walk before
    // the inverse re-runs, so `(x ⊕ e) ⊕ e = x` holds exactly.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(0b1100), Constant::Int(0b1010)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },  // R0 = 12   (journals)
            Op::LoadConst { dst: 1, idx: 1 },  // R1 = 10   (journals)
            Op::RevXor { place: 0, delta: 1 }, // R0 = 12 ^ 10 = 6   (zero garbage)
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };

    let initial = observable(&Vm::new(&prog));
    let mut vm = Vm::new(&prog);
    vm.run_recording().expect("xor program is reversible");
    let tape = vm.take_journal().expect("tape present");
    assert_eq!(
        tape.len(),
        2,
        "the RevXor must contribute ZERO tape entries (only the 2 loads)"
    );
    vm.run_reverse(tape).expect("reverse succeeds");
    assert_eq!(
        initial,
        observable(&vm),
        "forward ; reverse restores the exact initial state"
    );
}

#[test]
fn additive_group_reverses_with_zero_garbage() {
    // The (ℤ/2⁶⁴, +) reversible additive ops: RevNeg (self-inverse), and RevAdd/RevSub as mutual
    // inverses. None records a tape entry — only the three setup loads do — yet the reverse walk
    // (RevSub⁻¹=RevAdd, RevAdd⁻¹=RevSub, RevNeg⁻¹=RevNeg) restores the exact initial state.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(100), Constant::Int(7), Constant::Int(3)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },  // R0 = 100  (journals)
            Op::LoadConst { dst: 1, idx: 1 },  // R1 = 7    (journals)
            Op::LoadConst { dst: 2, idx: 2 },  // R2 = 3    (journals)
            Op::RevAdd { place: 0, delta: 1 }, // R0 = 107  (zero garbage)
            Op::RevSub { place: 0, delta: 2 }, // R0 = 104  (zero garbage)
            Op::RevNeg { place: 0 },           // R0 = -104 (zero garbage)
            Op::Halt,
        ],
        register_count: 3,
        ..Default::default()
    };

    let initial = observable(&Vm::new(&prog));
    let mut vm = Vm::new(&prog);
    vm.run_recording().expect("additive program is reversible");
    let tape = vm.take_journal().expect("tape present");
    assert_eq!(
        tape.len(),
        3,
        "the 3 additive ops record NOTHING — only the 3 loads journal"
    );
    vm.run_reverse(tape).expect("reverse succeeds");
    assert_eq!(
        initial,
        observable(&vm),
        "forward ; reverse restores the exact initial state"
    );
}

#[test]
fn modular_wraparound_reverses_exactly() {
    // RevAdd/RevSub are WRAPPING (ℤ/2⁶⁴) — overflow is part of the group, not an error, so the
    // inverse recovers the exact bit pattern even across i64::MAX.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(i64::MAX), Constant::Int(1000)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },  // R0 = i64::MAX
            Op::LoadConst { dst: 1, idx: 1 },  // R1 = 1000
            Op::RevAdd { place: 0, delta: 1 }, // R0 wraps past i64::MAX
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn single_swap_roundtrips_through_the_reverse_walk() {
    // A single swap survives the code-walking reverse pass (bijection inverse interleaved with
    // the tape restores of the surrounding loads, in strict reverse order).
    let prog = CompiledProgram {
        constants: vec![Constant::Int(1), Constant::Int(2)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 1 },
            Op::RevSwap { a: 0, b: 1 },
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    roundtrip(&prog);
}

/// An `if cond then R1=10 else R1=20` in bytecode. `cond_const` selects the branch taken.
fn if_else_program(cond_const: i64) -> CompiledProgram {
    CompiledProgram {
        constants: vec![
            Constant::Int(cond_const),
            Constant::Int(10),
            Constant::Int(20),
        ],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },       // R0 = cond
            Op::JumpIfFalse { cond: 0, target: 4 }, // if !cond goto else
            Op::LoadConst { dst: 1, idx: 1 },       // then: R1 = 10
            Op::Jump { target: 5 },                 // skip else
            Op::LoadConst { dst: 1, idx: 2 },       // else: R1 = 20
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    }
}

#[test]
fn if_then_branch_reverses_by_following_the_trace() {
    // cond = 1 (truthy) — the THEN path executes; reverse follows the recorded step trace back
    // through the taken branch, not code order.
    roundtrip(&if_else_program(1));
}

#[test]
fn if_else_branch_reverses_by_following_the_trace() {
    // cond = 0 (falsy) — the ELSE path executes; reverse follows the OTHER recorded path back.
    roundtrip(&if_else_program(0));
}

#[test]
fn reversible_loop_body_is_zero_garbage_regardless_of_trip_count() {
    // A countdown loop whose body is PURELY abelian (RevAdd/RevSub). It runs 3 iterations, yet the
    // data tape holds ONLY the 3 setup loads — the loop body erased ZERO bits across every
    // iteration (the adiabatic dream: a reversible loop costs no Landauer energy). The control
    // trace records the path (13 executed ops), but that is re-derivable overhead, not erasure.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(0), Constant::Int(1), Constant::Int(3)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },       // R0 = 0   (accumulator)
            Op::LoadConst { dst: 1, idx: 1 },       // R1 = 1   (step)
            Op::LoadConst { dst: 2, idx: 2 },       // R2 = 3   (counter)
            Op::JumpIfFalse { cond: 2, target: 7 }, // while R2 != 0
            Op::RevAdd { place: 0, delta: 1 },      //   R0 += 1   (zero garbage)
            Op::RevSub { place: 2, delta: 1 },      //   R2 -= 1   (zero garbage)
            Op::Jump { target: 3 },                 //   loop back
            Op::Halt,
        ],
        register_count: 3,
        ..Default::default()
    };

    let initial = observable(&Vm::new(&prog));
    let mut vm = Vm::new(&prog);
    vm.run_recording().expect("reversible loop is supported");
    // R0 counted up to 3, R2 counted down to 0.
    let tape = vm.take_journal().expect("tape present");
    assert_eq!(
        tape.len(),
        3,
        "only the 3 setup loads erase data — the loop body erases nothing"
    );
    assert_eq!(tape.erased_bytes(), 3 * std::mem::size_of::<Value>());
    assert!(
        tape.step_count() > tape.len(),
        "the control trace records the full executed path"
    );
    vm.run_reverse(tape).expect("reverse succeeds");
    assert_eq!(
        initial,
        observable(&vm),
        "forward ; reverse restores the exact initial state"
    );
}

/// Deterministic SplitMix64 — a seeded generator so the fuzz corpus is reproducible (no ambient
/// entropy, per the project's determinism discipline).
struct SplitMix64(u64);
impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// A random program: load `nregs` Int registers, then a random sequence of abelian ops (every
/// one distinct-index-well-formed), then Halt. Returns the program and its load count.
fn gen_abelian_program(rng: &mut SplitMix64) -> (CompiledProgram, usize) {
    let nregs = 2 + rng.below(4) as usize; // 2..=5 registers
    let mut constants = Vec::new();
    let mut code = Vec::new();
    for r in 0..nregs {
        constants.push(Constant::Int(rng.next_u64() as i64));
        code.push(Op::LoadConst {
            dst: r as u16,
            idx: (constants.len() - 1) as u32,
        });
    }
    let nops = rng.below(24);
    for _ in 0..nops {
        let a = rng.below(nregs as u64) as u16;
        let mut b = rng.below(nregs as u64) as u16;
        while b == a {
            b = rng.below(nregs as u64) as u16; // enforce place != delta
        }
        code.push(match rng.below(5) {
            0 => Op::RevSwap { a, b },
            1 => Op::RevXor { place: a, delta: b },
            2 => Op::RevNeg { place: a },
            3 => Op::RevAdd { place: a, delta: b },
            _ => Op::RevSub { place: a, delta: b },
        });
    }
    code.push(Op::Halt);
    (
        CompiledProgram {
            constants,
            code,
            register_count: nregs,
            ..Default::default()
        },
        nregs,
    )
}

#[test]
fn fuzz_abelian_compositions_reverse_with_zero_garbage() {
    // ROBUST TO ABSURDITY: 4000 random compositions of the abelian group ops (swap / xor / neg /
    // add / sub). Every one must (a) reverse to the EXACT initial state, and (b) erase ZERO data
    // beyond the setup loads — the entire composition is zero-garbage no matter how deep. This is
    // the algebraic heart of the adiabatic claim, hammered.
    let mut rng = SplitMix64(0xD1CE_5EED_C0FF_EE00);
    for _ in 0..4000 {
        let (prog, nregs) = gen_abelian_program(&mut rng);
        let initial = observable(&Vm::new(&prog));
        let mut vm = Vm::new(&prog);
        vm.run_recording()
            .expect("abelian program is in the reversible subset");
        let tape = vm.take_journal().expect("tape present");
        assert_eq!(
            tape.len(),
            nregs,
            "only the {nregs} loads erase data; every abelian op is zero-garbage"
        );
        vm.run_reverse(tape).expect("reverse succeeds");
        assert_eq!(
            initial,
            observable(&vm),
            "forward ; reverse must restore the exact initial state for this composition"
        );
    }
}

#[test]
fn collection_construction_and_reads_roundtrip() {
    // Build a list and a range, read length / index / slice into fresh registers. None mutates an
    // existing collection, so each reverses by restoring dst — the fresh objects simply drop.
    let prog = CompiledProgram {
        constants: vec![
            Constant::Int(1),
            Constant::Int(2),
            Constant::Int(3),
            Constant::Int(4),
        ],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 1
            Op::LoadConst { dst: 1, idx: 1 }, // R1 = 2
            Op::LoadConst { dst: 2, idx: 2 }, // R2 = 3
            Op::LoadConst { dst: 3, idx: 3 }, // R3 = 4
            Op::NewList {
                dst: 4,
                start: 0,
                count: 4,
            }, // R4 = [1,2,3,4]
            Op::NewRange {
                dst: 5,
                start: 0,
                end: 3,
            }, // R5 = [1..4]
            Op::Length {
                dst: 6,
                collection: 4,
            }, // R6 = 4
            Op::Index {
                dst: 7,
                collection: 4,
                index: 1,
            }, // R7 = R4[R1=2] = 2 (1-based)
            Op::SliceOp {
                dst: 8,
                collection: 4,
                start: 0,
                end: 1,
            }, // R8 = R4[1..2] = [1,2]
            Op::Halt,
        ],
        register_count: 9,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn struct_and_enum_construction_roundtrip() {
    // NewStruct / NewInductive build fresh heap objects into a dst; TestArm reads a tag into a
    // Bool. All reverse by restoring dst (the fresh objects drop) — no existing state is touched.
    let prog = CompiledProgram {
        constants: vec![
            Constant::Text("Point".to_string()), // struct type name
            Constant::Text("Some".to_string()),  // enum type name
            Constant::Text("Some".to_string()),  // constructor
        ],
        code: vec![
            Op::NewStruct {
                dst: 0,
                type_name: 0,
            }, // R0 = Point{}
            Op::NewInductive {
                dst: 1,
                type_name: 1,
                ctor: 2,
                args_start: 0,
                count: 0,
            }, // R1 = Some
            Op::TestArm {
                dst: 2,
                target: 1,
                variant: 2,
            }, // R2 = (R1 is Some) = true
            Op::Halt,
        ],
        register_count: 3,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn list_push_reverses_in_place() {
    // Build a list and push onto it (sole owner). Reverse restores the exact pre-push list.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(7), Constant::Int(8), Constant::Int(9)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 7
            Op::NewList {
                dst: 1,
                start: 0,
                count: 1,
            }, // R1 = [7]
            Op::LoadConst { dst: 2, idx: 1 }, // R2 = 8
            Op::ListPush { list: 1, value: 2 }, // R1 = [7, 8]
            Op::LoadConst { dst: 3, idx: 2 }, // R3 = 9
            Op::ListPush { list: 1, value: 3 }, // R1 = [7, 8, 9]
            Op::Halt,
        ],
        register_count: 4,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn list_push_reverses_bit_identically_under_aliasing() {
    // The hard case: `R2` aliases `R1`'s list (value semantics). A push to `R1` copy-on-writes,
    // leaving `R2` untouched. Reverse must restore `R1` to the ORIGINAL shared handle so `R1` and
    // `R2` re-alias exactly — not merely a value-equal clone.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(7), Constant::Int(8)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 7
            Op::NewList {
                dst: 1,
                start: 0,
                count: 1,
            }, // R1 = [7]
            Op::Move { dst: 2, src: 1 },      // R2 = R1  (aliases the list)
            Op::LoadConst { dst: 3, idx: 1 }, // R3 = 8
            Op::ListPush { list: 1, value: 3 }, // R1 = [7,8]; R2 stays [7] (COW)
            Op::Halt,
        ],
        register_count: 4,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn list_pop_reverses() {
    // Pop the last element into a fresh register. Reverse restores the pristine pre-pop list
    // (COW-forced, so it still holds the popped element) and the destination's prior value.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(1), Constant::Int(2), Constant::Int(3)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 1
            Op::LoadConst { dst: 1, idx: 1 }, // R1 = 2
            Op::LoadConst { dst: 2, idx: 2 }, // R2 = 3
            Op::NewList {
                dst: 3,
                start: 0,
                count: 3,
            }, // R3 = [1,2,3]
            Op::ListPop { list: 3, dst: 4 },  // R4 = 3; R3 = [1,2]
            Op::Halt,
        ],
        register_count: 5,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn set_index_reverses() {
    // Overwrite an element in place. Reverse restores the pristine pre-write list.
    let prog = CompiledProgram {
        constants: vec![
            Constant::Int(1),
            Constant::Int(2),
            Constant::Int(3),
            Constant::Int(99),
        ],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 1
            Op::LoadConst { dst: 1, idx: 1 }, // R1 = 2   (used as index)
            Op::LoadConst { dst: 2, idx: 2 }, // R2 = 3
            Op::LoadConst { dst: 3, idx: 3 }, // R3 = 99  (new value)
            Op::NewList {
                dst: 4,
                start: 0,
                count: 3,
            }, // R4 = [1,2,3]
            Op::SetIndex {
                collection: 4,
                index: 1,
                value: 3,
            }, // R4[R1=2] = 99 → [1,99,3]
            Op::Halt,
        ],
        register_count: 5,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn struct_field_write_reverses() {
    // Build a struct, then set two fields (in-place inserts on the by-value Box). Reverse restores
    // the pristine pre-insert struct snapshots.
    let prog = CompiledProgram {
        constants: vec![
            Constant::Text("Point".to_string()), // type name
            Constant::Text("x".to_string()),     // field
            Constant::Text("y".to_string()),     // field
            Constant::Int(3),
            Constant::Int(4),
        ],
        code: vec![
            Op::NewStruct {
                dst: 0,
                type_name: 0,
            }, // R0 = Point{}
            Op::LoadConst { dst: 1, idx: 3 }, // R1 = 3
            Op::StructInsert {
                obj: 0,
                field: 1,
                value: 1,
            }, // R0.x = 3
            Op::LoadConst { dst: 2, idx: 4 }, // R2 = 4
            Op::StructInsert {
                obj: 0,
                field: 2,
                value: 2,
            }, // R0.y = 4
            Op::Halt,
        ],
        register_count: 3,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn function_call_reverses() {
    // Main calls f(5); f computes 5 + 1 = 6 and returns it into R1. Reverse rebuilds the callee's
    // register window (regrow + teardown-undo: return value, arg-clear, frame restore) and walks
    // the callee body backward — all the way to the exact initial state.
    use super::super::instruction::CompiledFunction;
    let f = CompiledFunction {
        name: crate::intern::Symbol::from_index(0),
        entry_pc: 3, // f's body starts at code[3]
        param_count: 1,
        register_count: 3, // f uses R0 (param), R1, R2
        captures: vec![],
        named_regs: vec![],
        param_kinds: vec![],
        ret_kind: None,
        param_types: vec![],
        return_type: None,
        mutable_param_regs: vec![],
        cow_exempt_regs: vec![],
    };
    let prog = CompiledProgram {
        constants: vec![Constant::Int(5), Constant::Int(1)],
        code: vec![
            // ── Main ──
            Op::LoadConst { dst: 2, idx: 0 }, // pc0: R2 = 5  (the argument, at args_start = 2)
            Op::Call {
                dst: 1,
                func: 0,
                args_start: 2,
                arg_count: 1,
            }, // pc1: R1 = f(R2)
            Op::Halt,                         // pc2
            // ── f (entry_pc = 3): its param is callee R0 (= the caller's R2) ──
            Op::LoadConst { dst: 1, idx: 1 }, // pc3: R1 = 1
            Op::Add {
                dst: 2,
                lhs: 0,
                rhs: 1,
            }, // pc4: R2 = R0 + R1 = 6
            Op::Return { src: 2 },            // pc5: return R2
        ],
        register_count: 3,
        functions: vec![f],
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn repeat_loop_over_a_list_reverses() {
    // `Repeat for x in [10,20,30]: sum += x` — the full iterator machinery (IterPrepare / IterNext
    // × 3 + exhausted / IterPop) plus an abelian accumulator. Reverse reconstructs the iterator
    // stack and unwinds every element back to the exact initial state.
    let prog = CompiledProgram {
        constants: vec![
            Constant::Int(10),
            Constant::Int(20),
            Constant::Int(30),
            Constant::Int(0),
        ],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 10
            Op::LoadConst { dst: 1, idx: 1 }, // R1 = 20
            Op::LoadConst { dst: 2, idx: 2 }, // R2 = 30
            Op::NewList {
                dst: 3,
                start: 0,
                count: 3,
            }, // R3 = [10,20,30]
            Op::LoadConst { dst: 4, idx: 3 }, // R4 = 0  (accumulator)
            Op::IterPrepare { iterable: 3 },  // pc5: snapshot R3
            Op::IterNext { dst: 5, exit: 9 }, // pc6: R5 = next, or jump to 9
            Op::RevAdd { place: 4, delta: 5 }, // pc7: sum += x  (zero garbage)
            Op::Jump { target: 6 },           // pc8: loop back
            Op::IterPop,                      // pc9
            Op::Halt,                         // pc10
        ],
        register_count: 6,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn closure_call_reverses() {
    // MakeClosure builds a (capture-free) closure into R1; CallValue invokes it as f(5) → 6.
    // Reverse unwinds the closure frame (CallValueUndo truncate) and the closure construction.
    use super::super::instruction::CompiledFunction;
    let f = CompiledFunction {
        name: crate::intern::Symbol::from_index(0),
        entry_pc: 4,
        param_count: 1,
        register_count: 3,
        captures: vec![],
        named_regs: vec![],
        param_kinds: vec![],
        ret_kind: None,
        param_types: vec![],
        return_type: None,
        mutable_param_regs: vec![],
        cow_exempt_regs: vec![],
    };
    let prog = CompiledProgram {
        constants: vec![Constant::Int(5), Constant::Int(1)],
        code: vec![
            Op::LoadConst { dst: 3, idx: 0 }, // pc0: R3 = 5 (arg at args_start = 3)
            Op::MakeClosure {
                dst: 1,
                func: 0,
                locals_start: 0,
            }, // pc1: R1 = closure over f
            Op::CallValue {
                dst: 2,
                callee: 1,
                args_start: 3,
                arg_count: 1,
                name_for_err: u32::MAX,
            }, // pc2
            Op::Halt,                         // pc3
            Op::LoadConst { dst: 1, idx: 1 }, // pc4: f body: R1 = 1
            Op::Add {
                dst: 2,
                lhs: 0,
                rhs: 1,
            }, // pc5: R2 = R0 + R1 = 6
            Op::Return { src: 2 },            // pc6
        ],
        register_count: 4,
        functions: vec![f],
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn clock_reads_reverse_and_capture_the_instant() {
    // LoadNow/LoadToday read the external clock into a register. Reading does not MUTATE the
    // world, so they reverse by restoring dst — and the drawn instant is CAPTURED on the tape
    // (the nondeterminism is recorded, not silently discarded — the honest reversible treatment),
    // so `forward ; reverse ≡ initial` holds whatever the wall clock reads.
    let prog = CompiledProgram {
        constants: vec![],
        code: vec![Op::LoadNow { dst: 0 }, Op::LoadToday { dst: 1 }, Op::Halt],
        register_count: 2,
        ..Default::default()
    };

    let initial = observable(&Vm::new(&prog));
    let mut vm = Vm::new(&prog);
    vm.run_recording().expect("clock reads are reversible");
    let tape = vm.take_journal().expect("tape present");
    // The two drawn instants are captured on the tape (entropy recorded, not discarded).
    assert_eq!(
        tape.nondet_count(),
        2,
        "each clock read records its drawn instant"
    );
    vm.run_reverse(tape).expect("reverse succeeds");
    assert_eq!(
        initial,
        observable(&vm),
        "forward ; reverse restores the exact initial state"
    );
}

#[test]
fn args_and_region_guard_reverse() {
    // Args reads the program argv into a fresh register (reversible by restoring dst — the argv
    // isn't mutated); RegionBoundsGuard is a pure interpreter no-op.
    let prog = CompiledProgram {
        constants: vec![],
        code: vec![
            Op::Args { dst: 0 }, // R0 = argv (empty seq in a fresh VM)
            Op::RegionBoundsGuard {
                array: 0,
                bound: 0,
                iv: 0,
                add_max: 0,
                add_min: 0,
            }, // no-op
            Op::Halt,
        ],
        register_count: 1,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn ensure_owned_reverses() {
    // EnsureOwned is a copy-on-write barrier: journaling the handle forces the clone and reverse
    // restores the original — bit-identical.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(1)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 1
            Op::NewList {
                dst: 1,
                start: 0,
                count: 1,
            }, // R1 = [1]
            Op::EnsureOwned { reg: 1 },       // COW barrier on R1
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn destructure_tuple_reverses() {
    // DestructureTuple writes MULTIPLE registers (a tuple's elements). Reverse restores all of
    // them from the multi-slot journal.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(10), Constant::Int(20)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 10
            Op::LoadConst { dst: 1, idx: 1 }, // R1 = 20
            Op::NewTuple {
                dst: 2,
                start: 0,
                count: 2,
            }, // R2 = (10, 20)
            Op::DestructureTuple {
                src: 2,
                start: 3,
                count: 2,
            }, // R3, R4 = 10, 20
            Op::Halt,
        ],
        register_count: 5,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn crdt_counter_bump_reverses() {
    // A single-replica counter bump (`Increase count by 5`) mutates a by-value Box struct field
    // in place. Reverse restores the pristine pre-bump struct snapshot.
    let prog = CompiledProgram {
        constants: vec![
            Constant::Text("Counter".to_string()),
            Constant::Text("count".to_string()),
            Constant::Int(0),
            Constant::Int(5),
        ],
        code: vec![
            Op::NewStruct {
                dst: 0,
                type_name: 0,
            }, // R0 = Counter{}
            Op::LoadConst { dst: 1, idx: 2 }, // R1 = 0
            Op::StructInsert {
                obj: 0,
                field: 1,
                value: 1,
            }, // R0.count = 0
            Op::LoadConst { dst: 2, idx: 3 }, // R2 = 5
            Op::CrdtBump {
                obj: 0,
                field: 1,
                amount: 2,
                negate: false,
            }, // R0.count += 5
            Op::Halt,
        ],
        register_count: 3,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn pure_builtin_call_reverses() {
    // A pure builtin (abs) writes its result to dst AND clears its argument registers. Reverse
    // restores both from the multi-slot journal.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(-5)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = -5
            Op::CallBuiltin {
                dst: 1,
                builtin: crate::semantics::builtins::BuiltinId::Abs,
                args_start: 0,
                arg_count: 1,
            }, // R1 = 5; R0 cleared
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn recording_accumulates_across_a_block_boundary() {
    // The per-task recording PRIMITIVE for a reversible scheduler: `run_until_block_recording`
    // journals the ops executed before a concurrency block (here a Sleep) and suspends, so the
    // scheduler can later reverse each task's slice from its accumulated tape.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(5), Constant::Int(100)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 5   (recorded)
            Op::LoadConst { dst: 1, idx: 1 }, // R1 = 100 (recorded)
            Op::Sleep { duration: 1 },        // suspends the task
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    let mut vm = Vm::new(&prog);
    let step = vm.run_until_block_recording().expect("runs to the block");
    assert!(matches!(step, VmStep::Blocked), "Sleep suspends the task");
    let tape = vm.take_journal().expect("recording tape present");
    assert_eq!(tape.len(), 2, "the two ops before the block were recorded");
    assert_eq!(
        tape.step_count(),
        3,
        "the step trace covers both loads + the Sleep"
    );
}

#[test]
fn single_task_sleep_reverses_through_the_driver() {
    // The driven recording path runs a single task THROUGH a concurrency block (a Sleep — virtual
    // time, no observable state change) and continues. `forward ; reverse ≡ initial` across the
    // block, proving the drive-loop + tape accumulation + reverse compose correctly.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(7)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // R0 = 7
            Op::Sleep { duration: 0 },        // block (sleeps R0 ticks; virtual time)
            Op::RevNeg { place: 0 },          // R0 = -7 (after the block)
            Op::Halt,
        ],
        register_count: 1,
        ..Default::default()
    };
    let initial = observable(&Vm::new(&prog));
    let mut vm = Vm::new(&prog);
    vm.run_recording_driven().expect("driven run succeeds");
    let tape = vm.take_journal().expect("tape present");
    vm.run_reverse(tape).expect("reverse succeeds");
    assert_eq!(
        initial,
        observable(&vm),
        "forward ; reverse restores the initial state across a block"
    );
}

#[test]
fn fire_and_forget_spawn_reverses_through_the_scheduler() {
    // Main spawns a child (fire-and-forget) that runs a computation. The reversible cooperative
    // scheduler drives BOTH task VMs, records the interleaved schedule + per-task tapes, then
    // reverses the whole execution schedule-backward — Main returns to its exact initial state and
    // the child is unwound to its post-spawn state (then dropped).
    use super::super::instruction::CompiledFunction;
    let child = CompiledFunction {
        name: crate::intern::Symbol::from_index(0),
        entry_pc: 3,
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
    };
    let prog = CompiledProgram {
        constants: vec![Constant::Int(5), Constant::Int(9)],
        code: vec![
            Op::Spawn {
                func: 0,
                args_start: 0,
                arg_count: 0,
            }, // pc0: fire-and-forget child()
            Op::LoadConst { dst: 0, idx: 0 }, // pc1: main: R0 = 5
            Op::Halt,                         // pc2
            Op::LoadConst { dst: 0, idx: 1 }, // pc3: child: R0 = 9
            Op::ReturnNothing,                // pc4
        ],
        register_count: 1,
        functions: vec![child],
        ..Default::default()
    };

    let initial = observable(&Vm::new(&prog));
    let mut coop = RevCoop::record(Vm::new(&prog)).expect("cooperative recording run succeeds");
    coop.reverse().expect("reverse succeeds");
    assert_eq!(
        initial,
        observe_state(&coop.main_state()),
        "forward ; reverse restores Main's exact initial state across the spawned task"
    );
}

#[test]
fn channel_send_recv_reverses_through_the_scheduler() {
    // A channel round-trip: create a channel, send 42, receive it back. The reversible scheduler
    // records the channel deltas + the delivered handle/payload, then reverses everything — the
    // registers AND the channel topology return to initial (no channels left).
    let prog = CompiledProgram {
        constants: vec![Constant::Int(42)],
        code: vec![
            Op::ChanNew {
                dst: 0,
                cap: -1,
                elem: crate::vm::ChanElem::Int,
            }, // R0 = channel
            Op::LoadConst { dst: 1, idx: 0 }, // R1 = 42
            Op::ChanSend { chan: 0, val: 1 }, // send R1 → R0's channel
            Op::ChanRecv { dst: 2, chan: 0 }, // R2 = recv from R0's channel = 42
            Op::Halt,
        ],
        register_count: 3,
        ..Default::default()
    };

    let initial = observable(&Vm::new(&prog));
    let mut coop = RevCoop::record(Vm::new(&prog)).expect("channel program is reversible");
    coop.reverse().expect("reverse succeeds");
    assert_eq!(
        initial,
        observe_state(&coop.main_state()),
        "forward ; reverse restores the exact register state across the channel round-trip"
    );
    assert_eq!(
        coop.channel_count(),
        0,
        "the created channel is reversed away"
    );
}

#[test]
fn cross_task_channel_communication_reverses() {
    // REAL concurrency: Main creates a channel, spawns a child (passing the channel handle as an
    // arg), and sends 42; the child receives it. The reversible scheduler drives both tasks,
    // records the cross-task channel delta + the child's delivered payload, and reverses the whole
    // interleaved execution — Main returns to initial and the channel is reversed away.
    use super::super::instruction::CompiledFunction;
    let child = CompiledFunction {
        name: crate::intern::Symbol::from_index(0),
        entry_pc: 5,
        param_count: 1,
        register_count: 2, // R0 = channel (param), R1 = received value
        captures: vec![],
        named_regs: vec![],
        param_kinds: vec![],
        ret_kind: None,
        param_types: vec![],
        return_type: None,
        mutable_param_regs: vec![],
        cow_exempt_regs: vec![],
    };
    let prog = CompiledProgram {
        constants: vec![Constant::Int(42)],
        code: vec![
            Op::ChanNew {
                dst: 0,
                cap: -1,
                elem: crate::vm::ChanElem::Int,
            }, // pc0: R0 = channel
            Op::Spawn {
                func: 0,
                args_start: 0,
                arg_count: 1,
            }, // pc1: spawn child(R0)
            Op::LoadConst { dst: 1, idx: 0 }, // pc2: R1 = 42
            Op::ChanSend { chan: 0, val: 1 }, // pc3: send 42 → channel
            Op::Halt,                         // pc4
            Op::ChanRecv { dst: 1, chan: 0 }, // pc5: child: R1 = recv (channel at R0) = 42
            Op::ReturnNothing,                // pc6
        ],
        register_count: 2,
        functions: vec![child],
        ..Default::default()
    };

    let initial = observable(&Vm::new(&prog));
    let mut coop =
        RevCoop::record(Vm::new(&prog)).expect("cross-task channel program is reversible");
    coop.reverse().expect("reverse succeeds");
    assert_eq!(
        initial,
        observe_state(&coop.main_state()),
        "forward ; reverse restores Main's exact state across the cross-task channel round-trip"
    );
    assert_eq!(coop.channel_count(), 0, "the channel is reversed away");
}

#[test]
fn blocking_receive_parks_then_reverses_through_the_scheduler() {
    // The GENERAL concurrency case: the consumer is spawned FIRST, so it is picked before the
    // producer and BLOCKS on the empty channel (parks). The producer then sends — a direct handoff
    // delivers the payload straight into the parked consumer's register and wakes it. The
    // reversible scheduler records the park + the cross-task handoff, then reverses the whole
    // interleaving: Main returns to initial and the channel is reversed away.
    use super::super::instruction::CompiledFunction;
    let func = |entry_pc| CompiledFunction {
        name: crate::intern::Symbol::from_index(0),
        entry_pc,
        param_count: 1,    // R0 = channel (param)
        register_count: 2, // R1 = value
        captures: vec![],
        named_regs: vec![],
        param_kinds: vec![],
        ret_kind: None,
        param_types: vec![],
        return_type: None,
        mutable_param_regs: vec![],
        cow_exempt_regs: vec![],
    };
    let prog = CompiledProgram {
        constants: vec![Constant::Int(42)],
        code: vec![
            Op::ChanNew {
                dst: 0,
                cap: -1,
                elem: crate::vm::ChanElem::Int,
            }, // pc0: R0 = channel
            Op::Spawn {
                func: 0,
                args_start: 0,
                arg_count: 1,
            }, // pc1: spawn consumer(R0) — tid 1
            Op::Spawn {
                func: 1,
                args_start: 0,
                arg_count: 1,
            }, // pc2: spawn producer(R0) — tid 2
            Op::Halt,                         // pc3
            Op::ChanRecv { dst: 1, chan: 0 }, // pc4: consumer: R1 = recv (parks — channel empty)
            Op::ReturnNothing,                // pc5
            Op::LoadConst { dst: 1, idx: 0 }, // pc6: producer: R1 = 42
            Op::ChanSend { chan: 0, val: 1 }, // pc7: send 42 → hands off to parked consumer
            Op::ReturnNothing,                // pc8
        ],
        register_count: 2,
        functions: vec![func(4), func(6)],
        ..Default::default()
    };

    let initial = observable(&Vm::new(&prog));
    let mut coop = RevCoop::record(Vm::new(&prog)).expect("blocking-recv program is reversible");
    coop.reverse().expect("reverse succeeds");
    assert_eq!(
        initial,
        observe_state(&coop.main_state()),
        "forward ; reverse restores Main across a park + cross-task handoff interleaving"
    );
    assert_eq!(coop.channel_count(), 0, "the channel is reversed away");
}

#[test]
fn spawn_handle_await_result_reverses_through_the_scheduler() {
    // Task handles: Main spawns a child WITH A HANDLE, then awaits it. The await parks Main (the
    // child hasn't run yet); when the child finishes returning 99, its `Done` slice hands the
    // result straight into Main's await register and wakes it. The reversible scheduler records the
    // handle delivery + the cross-task result handoff, then reverses the whole join — Main returns
    // to initial (both the handle register and the result register cleared).
    use super::super::instruction::CompiledFunction;
    let child = CompiledFunction {
        name: crate::intern::Symbol::from_index(0),
        entry_pc: 3,
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
    };
    let prog = CompiledProgram {
        constants: vec![Constant::Int(99)],
        code: vec![
            Op::SpawnHandle {
                dst: 0,
                func: 0,
                args_start: 0,
                arg_count: 0,
            }, // pc0: R0 = handle
            Op::TaskAwait { dst: 1, handle: 0 }, // pc1: park; R1 = child result (99)
            Op::Halt,                            // pc2
            Op::LoadConst { dst: 0, idx: 0 },    // pc3: child: R0 = 99
            Op::Return { src: 0 },               // pc4: return 99
        ],
        register_count: 2, // R0 = handle, R1 = awaited result
        functions: vec![child],
        ..Default::default()
    };

    let initial = observable(&Vm::new(&prog));
    let mut coop =
        RevCoop::record(Vm::new(&prog)).expect("spawn-handle/await program is reversible");
    coop.reverse().expect("reverse succeeds");
    assert_eq!(
        initial,
        observe_state(&coop.main_state()),
        "forward ; reverse restores Main across a spawn-handle + await-join interleaving"
    );
}

#[test]
fn select_first_ready_arm_reverses_through_the_scheduler() {
    // `Select` over two channels: Main sends 42 to channel A, then selects over {recv A, recv B}.
    // Channel A is ready, so arm 0 wins — the scheduler delivers the received value (42) into the
    // arm's var register AND the winning arm index (0) into the SelectWait destination, and
    // dequeues A. The reversible scheduler records BOTH deliveries + the dequeue, then reverses
    // the whole thing: every register returns to initial and both channels are reversed away.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(42)],
        code: vec![
            Op::ChanNew {
                dst: 0,
                cap: -1,
                elem: crate::vm::ChanElem::Int,
            }, // pc0: R0 = channel A
            Op::ChanNew {
                dst: 1,
                cap: -1,
                elem: crate::vm::ChanElem::Int,
            }, // pc1: R1 = channel B
            Op::LoadConst { dst: 2, idx: 0 },      // pc2: R2 = 42
            Op::ChanSend { chan: 0, val: 2 },      // pc3: send 42 → channel A
            Op::SelectArmRecv { chan: 0, var: 3 }, // pc4: arm 0 — recv A into R3
            Op::SelectArmRecv { chan: 1, var: 4 }, // pc5: arm 1 — recv B into R4
            Op::SelectWait { dst_arm: 5 },         // pc6: R5 = winning arm index (0)
            Op::Halt,                              // pc7
        ],
        register_count: 6,
        ..Default::default()
    };

    let initial = observable(&Vm::new(&prog));
    let mut coop = RevCoop::record(Vm::new(&prog)).expect("select program is reversible");
    coop.reverse().expect("reverse succeeds");
    assert_eq!(
        initial,
        observe_state(&coop.main_state()),
        "forward ; reverse restores Main across a Select (value + arm-index deliveries + dequeue)"
    );
    assert_eq!(coop.channel_count(), 0, "both channels are reversed away");
}

#[test]
fn crdt_sequence_append_reverses_via_deep_interior_snapshot() {
    // A CRDT sequence (an `Rc<RefCell<CrdtValue>>`) is created and appended to. The append mutates
    // the shared interior IN PLACE, so an Rc-bump snapshot would share the mutation — reverse
    // instead restores a DEEP pre-append snapshot THROUGH the live handle. `NewCrdt`'s drawn
    // replica id reverses by dst-restore (the clock / captured-nondeterminism pattern).
    let prog = CompiledProgram {
        constants: vec![Constant::Int(42)],
        code: vec![
            Op::NewCrdt { dst: 0, kind: 1 },     // R0 = new CRDT sequence
            Op::LoadConst { dst: 1, idx: 0 },    // R1 = 42
            Op::CrdtAppend { seq: 0, value: 1 }, // append 42 to the sequence
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn non_blocking_channel_ops_reverse_through_the_scheduler() {
    // The non-blocking channel surface: TrySend (Bool(true) + enqueue), TryRecv (dequeues the
    // value, then Nothing when empty), and Close (a reversible closed flag). The scheduler records
    // each resume + channel delta and reverses the whole sequence — Main returns to initial and
    // the channel is reversed away.
    let prog = CompiledProgram {
        constants: vec![Constant::Int(42)],
        code: vec![
            Op::ChanNew {
                dst: 0,
                cap: -1,
                elem: crate::vm::ChanElem::Int,
            }, // R0 = channel
            Op::LoadConst { dst: 1, idx: 0 }, // R1 = 42
            Op::ChanTrySend {
                dst: 2,
                chan: 0,
                val: 1,
            }, // R2 = true; enqueue 42
            Op::ChanTryRecv { dst: 3, chan: 0 }, // R3 = 42 (dequeue)
            Op::ChanTryRecv { dst: 4, chan: 0 }, // R4 = Nothing (empty)
            Op::ChanClose { chan: 0 },        // close the channel
            Op::Halt,
        ],
        register_count: 5,
        ..Default::default()
    };
    let initial = observable(&Vm::new(&prog));
    let mut coop =
        RevCoop::record(Vm::new(&prog)).expect("non-blocking channel program is reversible");
    coop.reverse().expect("reverse succeeds");
    assert_eq!(
        initial,
        observe_state(&coop.main_state()),
        "forward ; reverse restores Main across TrySend / TryRecv / Close"
    );
    assert_eq!(coop.channel_count(), 0, "the channel is reversed away");
}

#[test]
fn crdt_merge_reverses_via_box_snapshot() {
    // A CRDT merge folds the source struct's fields into the target by REPLACING values with their
    // joins (Int ⊔ Int → wrapping add) — never mutating a nested `Rc` in place — so a deep Box
    // snapshot of the target reverses it exactly.
    let prog = CompiledProgram {
        constants: vec![
            Constant::Text("Counter".to_string()), // 0: type name
            Constant::Text("x".to_string()),       // 1: field name
            Constant::Int(3),                      // 2
            Constant::Int(4),                      // 3
        ],
        code: vec![
            Op::NewStruct {
                dst: 0,
                type_name: 0,
            }, // R0 = Counter{} (target)
            Op::LoadConst { dst: 1, idx: 2 }, // R1 = 3
            Op::StructInsert {
                obj: 0,
                field: 1,
                value: 1,
            }, // R0.x = 3
            Op::NewStruct {
                dst: 2,
                type_name: 0,
            }, // R2 = Counter{} (source)
            Op::LoadConst { dst: 3, idx: 3 }, // R3 = 4
            Op::StructInsert {
                obj: 2,
                field: 1,
                value: 3,
            }, // R2.x = 4
            Op::CrdtMerge {
                target: 0,
                source: 2,
            }, // R0.x = 3 ⊔ 4
            Op::Halt,
        ],
        register_count: 4,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn crdt_resolve_on_a_register_field_reverses_via_deep_field_snapshot() {
    // A resolve onto a nested CRDT REGISTER field mutates that field's `Rc<RefCell<CrdtValue>>`
    // in place; reverse restores a deep pre-resolve snapshot THROUGH the field's live handle (so
    // any alias observes it). `NewCrdt`'s replica id reverses by dst-restore.
    let prog = CompiledProgram {
        constants: vec![
            Constant::Text("Doc".to_string()), // 0: type name
            Constant::Text("val".to_string()), // 1: field name
            Constant::Int(5),                  // 2: resolved value
        ],
        code: vec![
            Op::NewStruct {
                dst: 0,
                type_name: 0,
            }, // R0 = Doc{}
            Op::NewCrdt { dst: 1, kind: 2 }, // R1 = LWW register
            Op::StructInsert {
                obj: 0,
                field: 1,
                value: 1,
            }, // R0.val = register
            Op::LoadConst { dst: 2, idx: 2 }, // R2 = 5
            Op::CrdtResolve {
                obj: 0,
                field: 1,
                value: 2,
            }, // resolve R0.val to 5
            Op::Halt,
        ],
        register_count: 3,
        ..Default::default()
    };
    roundtrip(&prog);
}

#[test]
fn reverse_is_faithful_across_the_tree_walker_tier() {
    // R2-tail — reverse-execution parity across tiers. The reverse executor is a VM (chip-ISA)
    // feature; this proves it is FAITHFUL to the tier-invariant language semantics. For each
    // reversible-subset source program: (1) the tree-walker and the VM produce byte-identical
    // FORWARD output (tier invariance), and (2) the VM's forward;reverse recovers the EXACT
    // initial observable state. Together: whatever the tree-walker computes, the VM computes AND
    // fully reverses — the reversal corrupts nothing the other tier could observe.
    let programs = [
        ("## Main\nShow 42.\n", "42"),
        ("## Main\nLet x be 10.\nShow x.\n", "10"),
        (
            "## Main\nLet mutable total be 5.\nSet total to total + 37.\nShow total.\n",
            "42",
        ),
        (
            "## Main\nLet mutable n be 100.\nSet n to n - 58.\nShow n.\n",
            "42",
        ),
    ];
    for (src, expected) in programs {
        let tw = crate::compile::tw_outcome_with_args(src, &[]);
        assert_eq!(
            tw.error, None,
            "tree-walker error on {src:?}: {:?}",
            tw.error
        );
        assert_eq!(tw.output.trim(), expected, "tree-walker output on {src:?}");

        crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
            let (stmts, _types, _policies) = parsed.expect("source parses");
            let prog = crate::vm::Compiler::compile(stmts, interner).expect("source compiles");
            let initial = observable(&Vm::new(&prog));
            let mut m = Vm::new(&prog);
            m.run_recording()
                .expect("program is in the reversible subset");
            assert_eq!(
                m.output().trim(),
                tw.output.trim(),
                "VM forward output must equal the tree-walker's on {src:?}"
            );
            let tape = m.take_journal().expect("recording leaves a tape");
            m.run_reverse(tape).expect("reverse succeeds");
            assert_eq!(
                initial,
                observable(&m),
                "VM forward ; reverse must recover the exact initial state on {src:?}"
            );
        });
    }
}

#[test]
fn english_swap_statement_compiles_to_the_zero_garbage_revswap_and_reverses() {
    // ★ LANGUAGE-LEVEL REVERSIBILITY (English) ★ — `Swap x and y.` is the FIRST LOGOS source syntax
    // that emits the zero-garbage abelian ISA (`Op::RevSwap`). The tree-walker and the VM agree on
    // the forward result, the compiled program contains RevSwap (not a journaled move), and the
    // whole program reverses to its initial state: a reversible primitive you can WRITE in English.
    let src = "## Main\n    Let mutable x be 10.\n    Let mutable y be 20.\n    Swap x and y.\n    Show x.\n    Show y.\n";
    let tw = crate::compile::tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker error: {:?}", tw.error);
    assert_eq!(tw.output.trim(), "20\n10", "Swap exchanges the two values");
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _t, _p) = parsed.expect("source parses");
        let prog = crate::vm::Compiler::compile(stmts, interner).expect("source compiles");
        assert!(
            prog.code.iter().any(|op| matches!(op, Op::RevSwap { .. })),
            "`Swap x and y` compiles to the zero-garbage RevSwap, not a journaled move"
        );
        let initial = observable(&Vm::new(&prog));
        let mut vm = Vm::new(&prog);
        vm.run_recording()
            .expect("the swap program is in the reversible subset");
        assert_eq!(
            vm.output().trim(),
            "20\n10",
            "VM forward output matches the tree-walker"
        );
        let tape = vm.take_journal().unwrap();
        vm.run_reverse(tape).expect("reverse");
        assert_eq!(
            initial,
            observable(&vm),
            "the swap program reverses to its initial state"
        );
    });
}

#[test]
fn observable_mode_elides_pure_scratch_and_still_reverses() {
    // PEBBLING P1 — a pure scratch temporary (`r2 = r0*r1`, feeding a Journaled `Move`) is
    // never read by any reverse step, so its journal is pure garbage. `FullState` records it;
    // `Observable` ELIDES it — a smaller tape — yet the observable (named-variable) state still
    // reverses exactly. This is the recomputable-scratch garbage leaving the tape.
    use super::super::pebble::{self, JournalDecision};
    use super::super::reverse::ReverseMode;
    let prog = CompiledProgram {
        constants: vec![Constant::Int(6), Constant::Int(7)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // r0 = 6  (named a)
            Op::LoadConst { dst: 1, idx: 1 }, // r1 = 7  (named b)
            Op::Mul {
                dst: 2,
                lhs: 0,
                rhs: 1,
            }, // r2 = 42 (SCRATCH temporary)
            Op::Move { dst: 3, src: 2 },      // r3 = 42 (named y)
            Op::Show { src: 3 },
            Op::Halt,
        ],
        register_count: 4,
        named_regs: vec![true, true, false, true], // r2 is compiler scratch
        ..Default::default()
    };

    // The plan: FullState keeps the scratch write; Observable elides it.
    let full = pebble::plan(&prog, ReverseMode::FullState);
    let obs = pebble::plan(&prog, ReverseMode::Observable);
    assert_eq!(
        full[2],
        JournalDecision::Keep,
        "FullState journals the scratch Mul"
    );
    assert_eq!(
        obs[2],
        JournalDecision::Elide,
        "Observable elides the pure-scratch Mul"
    );

    // The tape actually shrinks.
    let bytes = |mode| {
        let mut vm = Vm::new(&prog);
        vm.run_recording_mode(mode).expect("reversible subset");
        vm.take_journal().unwrap().erased_bytes()
    };
    assert!(
        bytes(ReverseMode::Observable) < bytes(ReverseMode::FullState),
        "Observable elides scratch: {} < {}",
        bytes(ReverseMode::Observable),
        bytes(ReverseMode::FullState)
    );

    // And the observable (named) state still reverses EXACTLY under Observable elision.
    let initial = observable(&Vm::new(&prog));
    let mut vm = Vm::new(&prog);
    vm.run_recording_mode(ReverseMode::Observable)
        .expect("reversible subset");
    let tape = vm.take_journal().unwrap();
    vm.run_reverse(tape).expect("reverse succeeds");
    assert_eq!(
        initial,
        observable(&vm),
        "Observable forward;reverse restores the named state"
    );
}

#[test]
fn pebbling_optionality_fullstate_is_bit_exact_observable_is_named() {
    // The fidelity ⇄ space knob: `FullState` restores EVERY register bit-exactly (scratch too);
    // `Observable` restores the named state at a smaller tape. The two AGREE on the named
    // projection — the "best of both worlds" is a mode choice, not a correctness compromise.
    use super::super::reverse::ReverseMode;
    let build = || CompiledProgram {
        constants: vec![Constant::Int(6), Constant::Int(7)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 1 },
            Op::Mul {
                dst: 2,
                lhs: 0,
                rhs: 1,
            }, // r2 SCRATCH
            Op::Move { dst: 3, src: 2 },
            Op::Halt,
        ],
        register_count: 4,
        named_regs: vec![true, true, false, true],
        ..Default::default()
    };
    let prog = build();

    // FullState: every register (incl. the scratch r2) returns to initial — `observe_state`
    // compares ALL registers.
    let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
    let mut vm = Vm::new(&prog);
    vm.run_recording_mode(ReverseMode::FullState)
        .expect("reversible");
    let tape = vm.take_journal().unwrap();
    vm.run_reverse(tape).expect("reverse");
    assert_eq!(
        all_initial,
        observe_state(&vm.save_debug_state()),
        "FullState restores EVERY register"
    );

    // Observable: the named state returns to initial (scratch may not — it is invisible).
    let named_initial = observable(&Vm::new(&prog));
    let mut vm = Vm::new(&prog);
    vm.run_recording_mode(ReverseMode::Observable)
        .expect("reversible");
    let tape = vm.take_journal().unwrap();
    vm.run_reverse(tape).expect("reverse");
    assert_eq!(
        named_initial,
        observable(&vm),
        "Observable restores the named state"
    );
}

#[test]
fn pebbling_shrinks_the_tape_on_a_compiled_program() {
    // End-to-end on REAL LOGOS: `(a*b)+(c*d)` compiles to two scratch multiplies feeding a named
    // result. Pebbling elides that pure scratch — a strictly smaller tape — and the named state
    // still reverses exactly.
    use super::super::reverse::ReverseMode;
    let src = "## Main\nLet a be 6.\nLet b be 7.\nLet c be 2.\nLet d be 3.\nLet y be (a * b) + (c * d).\nShow y.\n";
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("source parses");
        let prog = crate::vm::Compiler::compile(stmts, interner).expect("source compiles");
        let bytes = |mode| {
            let mut vm = Vm::new(&prog);
            vm.run_recording_mode(mode).expect("reversible subset");
            vm.take_journal().unwrap().erased_bytes()
        };
        assert!(
            bytes(ReverseMode::Observable) < bytes(ReverseMode::FullState),
            "pebbling shrinks a compiled program's tape: Observable {} < FullState {}",
            bytes(ReverseMode::Observable),
            bytes(ReverseMode::FullState)
        );
        let initial = observable(&Vm::new(&prog));
        let mut vm = Vm::new(&prog);
        vm.run_recording_mode(ReverseMode::Observable)
            .expect("reversible subset");
        let tape = vm.take_journal().unwrap();
        vm.run_reverse(tape).expect("reverse succeeds");
        assert_eq!(
            initial,
            observable(&vm),
            "Observable reverse restores the named state"
        );
    });
}

#[test]
fn pebbling_pure_computation_into_output_has_zero_landauer_cost() {
    // PEBBLING P2 — the honest floor. A PURE computation whose only output is a `Show` erases
    // ZERO information: pebbling elides the scratch multiplies, the named first-writes overwrite
    // the default (`Nothing`, no information), and `Show` is a re-derivable 0-byte line. So the
    // TRUE Landauer number — `landauer_bytes` — is 0, even though the raw tape still has entries.
    use super::super::reverse::ReverseMode;
    let src = "## Main\nLet a be 6.\nLet b be 7.\nLet c be 2.\nLet d be 3.\nLet y be (a * b) + (c * d).\nShow y.\n";
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("source parses");
        let prog = crate::vm::Compiler::compile(stmts, interner).expect("source compiles");
        let mut vm = Vm::new(&prog);
        vm.run_recording_mode(ReverseMode::Observable)
            .expect("reversible subset");
        let tape = vm.take_journal().unwrap();
        assert_eq!(
            tape.landauer_bytes(),
            0,
            "a pure computation into output erases zero information (raw tape was {} bytes)",
            tape.erased_bytes()
        );
    });
}

#[test]
fn pebbling_in_place_division_records_only_the_remainder() {
    // PEBBLING P2 — a GENUINELY-lossy op. `a = a / b` discards the remainder; the quotient is
    // kept. Reconstruct `a_old = q·b + r` from the kept quotient + the live divisor, so the tape
    // records ONLY the remainder — its Landauer cost is the remainder's few bits, NOT the whole
    // 16-byte dividend word — and it still reverses exactly.
    use super::super::reverse::ReverseMode;
    let prog = CompiledProgram {
        constants: vec![Constant::Int(20), Constant::Int(3)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // a = 20 (named)
            Op::LoadConst { dst: 1, idx: 1 }, // b = 3  (named)
            Op::Div {
                dst: 0,
                lhs: 0,
                rhs: 1,
            }, // a = a / b = 6  (remainder 2 discarded, IN-PLACE)
            Op::Halt,
        ],
        register_count: 2,
        named_regs: vec![true, true],
        ..Default::default()
    };
    let initial = observable(&Vm::new(&prog));
    let mut vm = Vm::new(&prog);
    vm.run_recording_mode(ReverseMode::FullState)
        .expect("reversible");
    let tape = vm.take_journal().unwrap();
    // The division's Landauer cost is the remainder's bits — far below a full Value word.
    assert!(
        tape.landauer_bytes() < std::mem::size_of::<super::Value>(),
        "in-place division records only the remainder, not the dividend: {} bytes",
        tape.landauer_bytes()
    );
    vm.run_reverse(tape).expect("reverse");
    assert_eq!(
        initial,
        observable(&vm),
        "in-place division reverses exactly via reconstruction"
    );
}

/// Drive one hand-built IN-PLACE lossy program (`a OP= …`), assert the tape's Landauer cost is
/// below a full 16-byte `Value` word (only the genuinely-destroyed bits are journaled), and that
/// it reverses to the exact initial observable state. The shared corner-1 minimal-bit lock.
fn assert_in_place_minimal_reverse(prog: CompiledProgram) {
    use super::super::reverse::ReverseMode;
    let initial = observable(&Vm::new(&prog));
    let mut vm = Vm::new(&prog);
    vm.run_recording_mode(ReverseMode::FullState)
        .expect("reversible");
    let tape = vm.take_journal().unwrap();
    assert!(
        tape.landauer_bytes() < std::mem::size_of::<super::Value>(),
        "an in-place lossy op journals only its destroyed bits, not the whole word: {} bytes",
        tape.landauer_bytes()
    );
    assert!(
        tape.landauer_bytes() <= tape.erased_bytes(),
        "Landauer floor never exceeds peak tape"
    );
    vm.run_reverse(tape).expect("reverse");
    assert_eq!(
        initial,
        observable(&vm),
        "the in-place lossy op reverses exactly via reconstruction"
    );
}

#[test]
fn pebbling_in_place_shift_right_records_only_the_dropped_bits() {
    // `a = a >> k` destroys only the low `k` bits; reverse rebuilds `a_old = (a << k) | dropped`
    // from the kept result. The tape records `k` bits, not the 16-byte dividend word.
    assert_in_place_minimal_reverse(CompiledProgram {
        constants: vec![Constant::Int(45), Constant::Int(3)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // a = 45  = 0b101101 (named)
            Op::LoadConst { dst: 1, idx: 1 }, // k = 3            (named)
            Op::Shr {
                dst: 0,
                lhs: 0,
                rhs: 1,
            }, // a = a >> 3 = 5   (low 0b101 dropped, IN-PLACE)
            Op::Halt,
        ],
        register_count: 2,
        named_regs: vec![true, true],
        ..Default::default()
    });
}

#[test]
fn pebbling_in_place_shift_left_records_only_the_dropped_bits() {
    // `a = a << k` destroys only the top `k` bits (shifted out); reverse rebuilds them from the
    // kept low bits + the `k` saved high bits.
    assert_in_place_minimal_reverse(CompiledProgram {
        constants: vec![Constant::Int(0x2000_0000_0000_0001), Constant::Int(3)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // a with top bits set (named)
            Op::LoadConst { dst: 1, idx: 1 }, // k = 3            (named)
            Op::Shl {
                dst: 0,
                lhs: 0,
                rhs: 1,
            }, // a = a << 3       (top 3 bits shifted out, IN-PLACE)
            Op::Halt,
        ],
        register_count: 2,
        named_regs: vec![true, true],
        ..Default::default()
    });
}

#[test]
fn pebbling_in_place_bitand_records_only_the_masked_out_bits() {
    // `a = a & mask` destroys the bits where `mask = 0`; reverse re-merges the saved bits under the
    // complementary mask. Landauer cost = the count of cleared positions, not the whole word.
    assert_in_place_minimal_reverse(CompiledProgram {
        constants: vec![Constant::Int(0x0123_4567_89AB_CDEF), Constant::Int(0xFF)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // a (named)
            Op::LoadConst { dst: 1, idx: 1 }, // mask = 0xFF (named)
            Op::BitAnd {
                dst: 0,
                lhs: 0,
                rhs: 1,
            }, // a = a & 0xFF (high 56 bits dropped, IN-PLACE)
            Op::Halt,
        ],
        register_count: 2,
        named_regs: vec![true, true],
        ..Default::default()
    });
}

#[test]
fn pebbling_in_place_bitor_records_only_the_set_bits() {
    // `a = a | mask` destroys the bits where `mask = 1`; reverse restores them from the saved bits.
    assert_in_place_minimal_reverse(CompiledProgram {
        constants: vec![Constant::Int(0x0123_4567_89AB_CDEF), Constant::Int(0xFF)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // a (named)
            Op::LoadConst { dst: 1, idx: 1 }, // mask = 0xFF (named)
            Op::BitOr {
                dst: 0,
                lhs: 0,
                rhs: 1,
            }, // a = a | 0xFF (low 8 bits forced, IN-PLACE)
            Op::Halt,
        ],
        register_count: 2,
        named_regs: vec![true, true],
        ..Default::default()
    });
}

#[test]
fn pebbling_in_place_mod_records_only_the_quotient() {
    // `a = a % b` keeps the remainder, discards the QUOTIENT; reverse rebuilds `a_old = q·b + r`
    // from the saved quotient + the live divisor. Landauer cost = the quotient's few bits.
    assert_in_place_minimal_reverse(CompiledProgram {
        constants: vec![Constant::Int(20), Constant::Int(3)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // a = 20 (named)
            Op::LoadConst { dst: 1, idx: 1 }, // b = 3  (named)
            Op::Mod {
                dst: 0,
                lhs: 0,
                rhs: 1,
            }, // a = a % b = 2 (quotient 6 discarded, IN-PLACE)
            Op::Halt,
        ],
        register_count: 2,
        named_regs: vec![true, true],
        ..Default::default()
    });
}

#[test]
fn adiabatic_mode_is_full_fidelity_at_the_landauer_floor() {
    // PEBBLING P3 — the best of both worlds. `Adiabatic` restores EVERY register bit-exactly (like
    // `FullState`) but at the ELIDED `Observable` tape (below `FullState`): non-reverse-read scratch
    // is not journaled — it is recomputed as its known default (`Nothing`) on reverse — yet full
    // machine fidelity holds. Full reversibility at the Landauer floor: the adiabatic chip contract.
    use super::super::reverse::ReverseMode;
    let build = || CompiledProgram {
        constants: vec![Constant::Int(6), Constant::Int(7)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 1 },
            Op::Mul {
                dst: 2,
                lhs: 0,
                rhs: 1,
            }, // r2 SCRATCH (elided under Observable/Adiabatic)
            Op::Move { dst: 3, src: 2 },
            Op::Halt,
        ],
        register_count: 4,
        named_regs: vec![true, true, false, true],
        ..Default::default()
    };
    let prog = build();
    let tape_bytes = |mode| {
        let mut vm = Vm::new(&prog);
        vm.run_recording_mode(mode).expect("reversible");
        vm.take_journal().unwrap().erased_bytes()
    };
    assert!(
        tape_bytes(ReverseMode::Adiabatic) < tape_bytes(ReverseMode::FullState),
        "Adiabatic sits at the Observable floor: {} < FullState {}",
        tape_bytes(ReverseMode::Adiabatic),
        tape_bytes(ReverseMode::FullState)
    );

    // Yet it restores EVERY register (scratch r2 too) — `observe_state` compares ALL registers.
    let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
    let mut vm = Vm::new(&prog);
    vm.run_recording_mode(ReverseMode::Adiabatic)
        .expect("reversible");
    let tape = vm.take_journal().unwrap();
    vm.run_reverse(tape).expect("reverse");
    assert_eq!(
        all_initial,
        observe_state(&vm.save_debug_state()),
        "Adiabatic restores EVERY register bit-exactly at the Landauer floor"
    );
}

#[test]
fn pebbling_fuzz_every_mode_reverses_its_projection() {
    // ★ THE ROBUSTNESS LOCK ★ — over 300 random reversible-subset programs with random
    // named/scratch bitmaps: `FullState` and `Adiabatic` restore EVERY register bit-exact, and
    // `Observable` restores the named projection. If any mode's elision ever corrupts a value the
    // mode is contracted to restore, this fails. (Deterministic LCG — no `Math.random`.)
    use super::super::reverse::ReverseMode;
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rng = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    let mut ran = 0usize;
    let mut elided = 0usize;
    for _ in 0..300 {
        let nregs = 3 + (rng() % 5) as usize;
        let named: Vec<bool> = (0..nregs).map(|_| rng() % 2 == 0).collect();
        let mut consts = Vec::new();
        let mut code = Vec::new();
        // Initialise every register (no uninitialised reads), with small values (no overflow).
        for r in 0..nregs {
            let idx = consts.len();
            consts.push(Constant::Int((rng() % 9) as i64 + 1));
            code.push(Op::LoadConst {
                dst: r as u16,
                idx: idx as u32,
            });
        }
        for _ in 0..(2 + rng() % 7) {
            let a = (rng() as usize % nregs) as u16;
            let d = (rng() as usize % nregs) as u16;
            // A distinct delta for the abelian ops — `place += place` breaks the independence
            // invariant (it is not self-inverse), so pick a delta register other than `place`.
            let delta = ((d as usize + 1 + rng() as usize % (nregs - 1)) % nregs) as u16;
            code.push(match rng() % 12 {
                0 => Op::Add {
                    dst: d,
                    lhs: a,
                    rhs: delta,
                },
                1 => Op::Sub {
                    dst: d,
                    lhs: a,
                    rhs: delta,
                },
                2 => Op::Move { dst: d, src: a },
                3 => Op::RevAdd { place: d, delta },
                4 => Op::RevSub { place: d, delta },
                5 => Op::RevXor { place: d, delta },
                // In-place division (the P2 reconstruct path) — `delta` may be 0 at runtime, in
                // which case the run errors and this program is skipped.
                6 => Op::Div {
                    dst: d,
                    lhs: d,
                    rhs: delta,
                },
                // The corner-1 in-place minimal-bit family (`dst == lhs`): shifts, masks, modulo.
                // Each journals only its destroyed bits and must reverse bit-exact; `Mod`/`Shr` with
                // a runtime-0 divisor/shift just fall back to a full overwrite (still exact).
                7 => Op::Shr {
                    dst: d,
                    lhs: d,
                    rhs: delta,
                },
                8 => Op::Shl {
                    dst: d,
                    lhs: d,
                    rhs: delta,
                },
                9 => Op::BitAnd {
                    dst: d,
                    lhs: d,
                    rhs: delta,
                },
                10 => Op::BitOr {
                    dst: d,
                    lhs: d,
                    rhs: delta,
                },
                _ => Op::Mod {
                    dst: d,
                    lhs: d,
                    rhs: delta,
                },
            });
        }
        code.push(Op::Halt);
        let prog = CompiledProgram {
            constants: consts,
            code,
            register_count: nregs,
            named_regs: named,
            ..Default::default()
        };
        // Skip any program that errors (e.g. an arithmetic edge) — only reversible RUNS are pinned.
        let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
        let named_initial = observable(&Vm::new(&prog));
        let mut full_bytes = 0;
        let mut ok = true;
        for mode in [ReverseMode::FullState, ReverseMode::Adiabatic] {
            let mut vm = Vm::new(&prog);
            if vm.run_recording_mode(mode).is_err() {
                ok = false;
                break;
            }
            let tape = vm.take_journal().unwrap();
            if mode == ReverseMode::FullState {
                full_bytes = tape.erased_bytes();
            }
            // The genuinely-erased Landauer floor never exceeds the peak-tape byte count — the
            // minimal-bit family (corner 1) records at most a word, and counts only destroyed bits.
            assert!(
                tape.landauer_bytes() <= tape.erased_bytes(),
                "{mode:?}: Landauer floor {} must not exceed peak tape {}",
                tape.landauer_bytes(),
                tape.erased_bytes()
            );
            vm.run_reverse(tape).expect("reverse succeeds");
            assert_eq!(
                all_initial,
                observe_state(&vm.save_debug_state()),
                "{mode:?} must restore EVERY register"
            );
        }
        if !ok {
            continue;
        }
        let mut vm = Vm::new(&prog);
        vm.run_recording_mode(ReverseMode::Observable).unwrap();
        let obs_tape = vm.take_journal().unwrap();
        let obs_bytes = obs_tape.erased_bytes();
        vm.run_reverse(obs_tape).expect("reverse succeeds");
        assert_eq!(
            named_initial,
            observable(&vm),
            "Observable must restore the named projection"
        );
        if obs_bytes < full_bytes {
            elided += 1; // elision actually fired on this program
        }
        ran += 1;
    }
    assert!(
        ran > 100,
        "the fuzz must actually exercise many programs, ran {ran}"
    );
    assert!(
        elided > 20,
        "elision must ACTUALLY fire across the corpus, fired {elided} times"
    );
}

#[test]
fn pebbling_elides_function_body_scratch_soundly() {
    // PIECE 1 — the frame-overlap analysis. `blend`'s `a*b`/`a+b` temporaries alias absolute slots
    // BEYOND Main's frame (callee registers past the caller), so they ELIDE; its result register
    // aliases Main's named `y`, so it is KEPT. Function-body scratch is elided — a smaller tape —
    // AND the named state (including `y`, the slot the aliasing bug corrupted) still reverses.
    use super::super::reverse::ReverseMode;
    let src = "## To blend (a: Int, b: Int) -> Int:\n    Return (a * b) + (a + b).\n## Main\n    Let x be 5.\n    Let y be blend(x, 3).\n    Show y.\n";
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("source parses");
        let prog = crate::vm::Compiler::compile(stmts, interner).expect("source compiles");
        let bytes = |mode| {
            let mut vm = Vm::new(&prog);
            vm.run_recording_mode(mode).expect("reversible");
            vm.take_journal().unwrap().erased_bytes()
        };
        assert!(
            bytes(ReverseMode::Observable) < bytes(ReverseMode::FullState),
            "function-body scratch is elided: Observable {} < FullState {}",
            bytes(ReverseMode::Observable),
            bytes(ReverseMode::FullState)
        );
        let initial = observable(&Vm::new(&prog));
        let mut vm = Vm::new(&prog);
        vm.run_recording_mode(ReverseMode::Observable)
            .expect("reversible");
        let tape = vm.take_journal().unwrap();
        vm.run_reverse(tape).expect("reverse");
        assert_eq!(
            initial,
            observable(&vm),
            "the named state (incl. y) reverses even with function-body elision — aliasing stays fixed"
        );
    });
}

#[test]
fn recursive_function_reverses_soundly_with_bounded_elision() {
    // CORNER 4 (re-spec of the former "elision disabled" lock): a NON-tail recursion (`sumdown`
    // recurses then adds) is a call-graph cycle. The relational base analysis now BOUNDS the frame
    // bases to `[0, main_size)` (a base ≥ main_size can never alias a Main-observable slot), so the
    // fixpoint TERMINATES under recursion instead of bailing — elision is SOUND, not disabled. The
    // Observable tape is never larger than FullState, and the recursive nest reverses EXACTLY under
    // every mode.
    use super::super::reverse::ReverseMode;
    let src = "## To sumdown (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Let rest be sumdown(n - 1).\n    Return n + rest.\n## Main\n    Let r be sumdown(3).\n    Show r.\n";
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("source parses");
        let prog = crate::vm::Compiler::compile(stmts, interner).expect("source compiles");
        let bytes = |mode| {
            let mut vm = Vm::new(&prog);
            vm.run_recording_mode(mode).expect("reversible");
            vm.take_journal().unwrap().erased_bytes()
        };
        assert!(
            bytes(ReverseMode::Observable) <= bytes(ReverseMode::FullState),
            "bounded-base elision under recursion never grows the tape: Observable {} <= FullState {}",
            bytes(ReverseMode::Observable),
            bytes(ReverseMode::FullState)
        );
        let named_initial = observable(&Vm::new(&prog));
        let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
        for mode in [
            ReverseMode::Observable,
            ReverseMode::FullState,
            ReverseMode::Adiabatic,
        ] {
            let mut vm = Vm::new(&prog);
            vm.run_recording_mode(mode).expect("reversible");
            let tape = vm.take_journal().unwrap();
            vm.run_reverse(tape).expect("reverse");
            // Observable restores the NAMED projection (scratch is legitimately elided); FullState
            // and Adiabatic restore EVERY register.
            if mode == ReverseMode::Observable {
                assert_eq!(
                    named_initial,
                    observable(&vm),
                    "Observable: named state restored"
                );
            } else {
                assert_eq!(
                    all_initial,
                    observe_state(&vm.save_debug_state()),
                    "{mode:?}: a recursive call nest must reverse every register"
                );
            }
        }
    });
}

#[test]
fn recursive_function_elides_callee_scratch_soundly() {
    // ★ CORNER 4 — recursion-safe elision ★. A recursive `countup` computes sub-expression scratch
    // (`n * n`, `(n*n) + rest`) in its callee frame; those temporaries alias slots BEYOND Main's
    // frame at every recursion depth (deeper frames sit at higher bases), so they must ELIDE — the
    // relational bounded-base analysis proves it soundly even though the call graph has a cycle. The
    // former analysis bailed on recursion (Observable == FullState); now elision fires (Observable <
    // FullState) AND every register still reverses bit-exact under every mode.
    use super::super::reverse::ReverseMode;
    let src = "## To countup (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Let rest be countup(n - 1).\n    Return (n * n) + rest.\n## Main\n    Let r be countup(3).\n    Show r.\n";
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("source parses");
        let prog = crate::vm::Compiler::compile(stmts, interner).expect("source compiles");
        let bytes = |mode| {
            let mut vm = Vm::new(&prog);
            vm.run_recording_mode(mode).expect("reversible");
            vm.take_journal().unwrap().erased_bytes()
        };
        assert!(
            bytes(ReverseMode::Observable) < bytes(ReverseMode::FullState),
            "recursion no longer disables elision: Observable {} < FullState {}",
            bytes(ReverseMode::Observable),
            bytes(ReverseMode::FullState)
        );
        let named_initial = observable(&Vm::new(&prog));
        let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
        for mode in [
            ReverseMode::Observable,
            ReverseMode::FullState,
            ReverseMode::Adiabatic,
        ] {
            let mut vm = Vm::new(&prog);
            vm.run_recording_mode(mode).expect("reversible");
            let tape = vm.take_journal().unwrap();
            vm.run_reverse(tape).expect("reverse");
            // Observable restores the NAMED projection; FullState/Adiabatic restore EVERY register.
            if mode == ReverseMode::Observable {
                assert_eq!(
                    named_initial,
                    observable(&vm),
                    "Observable: named state restored under recursion"
                );
            } else {
                assert_eq!(
                    all_initial,
                    observe_state(&vm.save_debug_state()),
                    "{mode:?}: recursive elision must still reverse every register bit-exact"
                );
            }
        }
    });
}

#[test]
fn function_call_programs_reverse_soundly_under_every_mode() {
    // ★ FRAME-OVERLAP SOUNDNESS LOCK ★ — a callee runs at `base = args_start`, so its registers
    // ALIAS the caller's slots: a callee-scratch slot can BE a caller-named variable (here blend's
    // result register is the same absolute slot as Main's `y`). Pebbling therefore disables elision
    // when calls are present, so a value-returning function call reverses EXACTLY under EVERY mode.
    // This is the regression test for the aliasing bug the review caught.
    use super::super::reverse::ReverseMode;
    let src = "## To blend (a: Int, b: Int) -> Int:\n    Return (a * b) + (a + b).\n## Main\n    Let x be 5.\n    Let y be blend(x, 3).\n    Show y.\n";
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("source parses");
        let prog = crate::vm::Compiler::compile(stmts, interner).expect("source compiles");
        let named_initial = observable(&Vm::new(&prog));
        let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
        for mode in [
            ReverseMode::Observable,
            ReverseMode::FullState,
            ReverseMode::Adiabatic,
        ] {
            let mut vm = Vm::new(&prog);
            vm.run_recording_mode(mode).expect("reversible");
            let tape = vm.take_journal().unwrap();
            vm.run_reverse(tape).expect("reverse");
            // Observable restores the named projection (incl. `y`, the aliased slot); FullState and
            // Adiabatic restore EVERY register. Either way `y` — the slot the aliasing bug corrupted
            // — comes back correct across the Call/Return boundary.
            if mode == ReverseMode::Observable {
                assert_eq!(
                    named_initial,
                    observable(&vm),
                    "Observable: named state (incl. y) restored"
                );
            } else {
                assert_eq!(
                    all_initial,
                    observe_state(&vm.save_debug_state()),
                    "{mode:?}: a value-returning function call must reverse EVERY register (frame aliasing)"
                );
            }
        }
    });
}

#[test]
fn pebbling_reverses_a_loop_with_scratch() {
    // A loop body with per-iteration scratch (`x*x` into a temp, `sum += temp`): no frame overlap
    // (same frame), so the elided loop scratch stays off the tape while the named accumulator
    // reverses through the full iterator machinery, walking the recorded step trace backward.
    use super::super::reverse::ReverseMode;
    let prog = CompiledProgram {
        constants: vec![
            Constant::Int(2),
            Constant::Int(3),
            Constant::Int(4),
            Constant::Int(0),
        ],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // r0 = 2
            Op::LoadConst { dst: 1, idx: 1 }, // r1 = 3
            Op::LoadConst { dst: 2, idx: 2 }, // r2 = 4
            Op::NewList {
                dst: 3,
                start: 0,
                count: 3,
            }, // r3 = [2,3,4]
            Op::LoadConst { dst: 4, idx: 3 }, // r4 = 0  (named accumulator)
            Op::IterPrepare { iterable: 3 },  // pc5
            Op::IterNext { dst: 5, exit: 10 }, // pc6: r5 = next, or exit to pc10
            Op::Mul {
                dst: 6,
                lhs: 5,
                rhs: 5,
            }, // pc7: r6 = x*x  (SCRATCH, elided)
            Op::Add {
                dst: 4,
                lhs: 4,
                rhs: 6,
            }, // pc8: sum += x*x  (accumulator, kept)
            Op::Jump { target: 6 },           // pc9: loop back
            Op::IterPop,                      // pc10
            Op::Halt,                         // pc11
        ],
        register_count: 7,
        // r4 (accumulator) is the only observable; r0..r3 (list build), r5 (element), r6 (x*x) scratch.
        named_regs: vec![false, false, false, false, true, false, false],
        ..Default::default()
    };
    // FullState + Observable both reverse the named accumulator; Observable elides the r6 scratch.
    let bytes = |mode| {
        let mut vm = Vm::new(&prog);
        vm.run_recording_mode(mode).expect("reversible");
        vm.take_journal().unwrap().erased_bytes()
    };
    assert!(
        bytes(ReverseMode::Observable) < bytes(ReverseMode::FullState),
        "loop-body scratch is elided: Observable {} < FullState {}",
        bytes(ReverseMode::Observable),
        bytes(ReverseMode::FullState)
    );
    let initial = observable(&Vm::new(&prog));
    let mut vm = Vm::new(&prog);
    vm.run_recording_mode(ReverseMode::Observable)
        .expect("reversible");
    let tape = vm.take_journal().unwrap();
    vm.run_reverse(tape).expect("reverse");
    assert_eq!(
        initial,
        observable(&vm),
        "a loop reverses under Observable elision"
    );
}

#[test]
fn in_place_division_reconstructs_under_a_later_overwrite() {
    // Locks the `DivReconstruct` reverse-read soundness: a named in-place division whose `dst` is
    // OVERWRITTEN later still reconstructs — `dst` is pinned in `reverse_reads`, so the later
    // write is journaled and `dst` is restored to the quotient before the reconstruction reads it.
    use super::super::reverse::ReverseMode;
    let prog = CompiledProgram {
        constants: vec![Constant::Int(20), Constant::Int(3), Constant::Int(99)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // a = 20
            Op::LoadConst { dst: 1, idx: 1 }, // b = 3
            Op::Div {
                dst: 0,
                lhs: 0,
                rhs: 1,
            }, // a = a/b = 6 (in-place)
            Op::LoadConst { dst: 0, idx: 2 }, // a = 99 (LATER overwrite of the Div's dst)
            Op::Halt,
        ],
        register_count: 2,
        named_regs: vec![true, true],
        ..Default::default()
    };
    let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
    let mut vm = Vm::new(&prog);
    vm.run_recording_mode(ReverseMode::FullState)
        .expect("reversible");
    let tape = vm.take_journal().unwrap();
    vm.run_reverse(tape).expect("reverse");
    assert_eq!(
        all_initial,
        observe_state(&vm.save_debug_state()),
        "in-place division reconstructs correctly under a later overwrite of its dst"
    );
}

#[test]
fn adiabatic_recomputes_reverse_live_scratch_below_observable() {
    // PIECE 2 — the true Bennett recompute. `t = a+b` is scratch but REVERSE-LIVE (it is a `RevAdd`
    // delta), so `Observable`/`FullState` must JOURNAL it. `Adiabatic` instead RECOMPUTES it: it
    // drops `t`'s journal entry and, during reverse, re-runs `t = a+b` from the (stable) live inputs
    // right before the `RevAdd` reverse reads it. The tape drops STRICTLY BELOW Observable, and
    // every register still reverses bit-exact — full fidelity at a smaller-than-Observable tape.
    use super::super::reverse::ReverseMode;
    let prog = CompiledProgram {
        constants: vec![Constant::Int(3), Constant::Int(4), Constant::Int(10)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // a = 3 (named, single-assignment)
            Op::LoadConst { dst: 1, idx: 1 }, // b = 4 (named, single-assignment)
            Op::LoadConst { dst: 2, idx: 2 }, // place = 10 (named)
            Op::Add {
                dst: 3,
                lhs: 0,
                rhs: 1,
            }, // t = a+b = 7 (SCRATCH, reverse-live)
            Op::RevAdd { place: 2, delta: 3 }, // place += t = 17
            Op::Halt,
        ],
        register_count: 4,
        named_regs: vec![true, true, true, false], // t (r3) is scratch
        ..Default::default()
    };
    let bytes = |mode| {
        let mut vm = Vm::new(&prog);
        vm.run_recording_mode(mode).expect("reversible");
        vm.take_journal().unwrap().erased_bytes()
    };
    assert!(
        bytes(ReverseMode::Adiabatic) < bytes(ReverseMode::Observable),
        "Adiabatic recomputes reverse-live scratch (below the Observable tape): {} < {}",
        bytes(ReverseMode::Adiabatic),
        bytes(ReverseMode::Observable)
    );
    // Full fidelity: EVERY register reverses, with `t` reconstructed by recompute, not journal.
    let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
    let mut vm = Vm::new(&prog);
    vm.run_recording_mode(ReverseMode::Adiabatic)
        .expect("reversible");
    let tape = vm.take_journal().unwrap();
    vm.run_reverse(tape).expect("reverse");
    assert_eq!(
        all_initial,
        observe_state(&vm.save_debug_state()),
        "Adiabatic restores every register — the reverse-live scratch via recompute, not journal"
    );
}

#[test]
fn adiabatic_recomputes_every_pure_two_operand_op() {
    // ★ CORNER 3 — recompute coverage ★. Recompute extends beyond Add/Sub/Mul/BitXor to the WHOLE
    // pure two-operand family. A reverse-live scratch defined by any of these (a `RevAdd` delta) is
    // RECOMPUTED on reverse instead of journaled, so Adiabatic sits strictly below Observable AND
    // reverses every register bit-exact. This closes corner 1's other half: a fresh-dst lossy op
    // with live operands is landauer-free (recomputable), not journaled.
    use super::super::reverse::ReverseMode;
    let run = |name: &str, def: Op| {
        let prog = CompiledProgram {
            constants: vec![Constant::Int(20), Constant::Int(3), Constant::Int(10)],
            code: vec![
                Op::LoadConst { dst: 0, idx: 0 },  // a = 20 (named)
                Op::LoadConst { dst: 1, idx: 1 },  // b = 3  (named)
                Op::LoadConst { dst: 2, idx: 2 },  // place = 10 (named)
                def,                               // t = a OP b (r3 SCRATCH, reverse-live)
                Op::RevAdd { place: 2, delta: 3 }, // place += t
                Op::Halt,
            ],
            register_count: 4,
            named_regs: vec![true, true, true, false],
            ..Default::default()
        };
        let bytes = |mode| {
            let mut vm = Vm::new(&prog);
            vm.run_recording_mode(mode).expect("reversible");
            vm.take_journal().unwrap().erased_bytes()
        };
        assert!(
            bytes(ReverseMode::Adiabatic) < bytes(ReverseMode::Observable),
            "{name}: Adiabatic recomputes the reverse-live scratch below Observable ({} vs {})",
            bytes(ReverseMode::Adiabatic),
            bytes(ReverseMode::Observable)
        );
        let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
        let mut vm = Vm::new(&prog);
        vm.run_recording_mode(ReverseMode::Adiabatic)
            .expect("reversible");
        let tape = vm.take_journal().unwrap();
        vm.run_reverse(tape).expect("reverse");
        assert_eq!(
            all_initial,
            observe_state(&vm.save_debug_state()),
            "{name}: Adiabatic reverses every register via recompute of the pure def"
        );
    };
    run(
        "Div",
        Op::Div {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
    );
    run(
        "FloorDiv",
        Op::FloorDiv {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
    );
    run(
        "Mod",
        Op::Mod {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
    );
    run(
        "Pow",
        Op::Pow {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
    );
    run(
        "BitAnd",
        Op::BitAnd {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
    );
    run(
        "BitOr",
        Op::BitOr {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
    );
    run(
        "Shl",
        Op::Shl {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
    );
    run(
        "Shr",
        Op::Shr {
            dst: 3,
            lhs: 0,
            rhs: 1,
        },
    );
}

#[test]
fn adiabatic_recomputes_a_multi_step_chain_below_observable() {
    // CHAINS — a 3-step recomputable chain: t1=a+b, t2=t1+c, t3=t2+d, place+=t3. t1,t2 feed
    // Journaled adds (elided); t3 is reverse-live (recomputed). Under Adiabatic the WHOLE chain is
    // off the tape — t3 reconstructed from t2, t2 held from t1, t1 held from a,b — so the tape sits
    // BELOW Observable (which must keep t3), at full fidelity. The pebble "recompute" corner.
    use super::super::reverse::ReverseMode;
    let prog = CompiledProgram {
        constants: vec![
            Constant::Int(1),
            Constant::Int(2),
            Constant::Int(3),
            Constant::Int(4),
            Constant::Int(100),
        ],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // a = 1
            Op::LoadConst { dst: 1, idx: 1 }, // b = 2
            Op::LoadConst { dst: 2, idx: 2 }, // c = 3
            Op::LoadConst { dst: 3, idx: 3 }, // d = 4
            Op::LoadConst { dst: 4, idx: 4 }, // place = 100
            Op::Add {
                dst: 5,
                lhs: 0,
                rhs: 1,
            }, // t1 = a+b     (scratch)
            Op::Add {
                dst: 6,
                lhs: 5,
                rhs: 2,
            }, // t2 = t1+c    (scratch)
            Op::Add {
                dst: 7,
                lhs: 6,
                rhs: 3,
            }, // t3 = t2+d    (scratch, reverse-live)
            Op::RevAdd { place: 4, delta: 7 }, // place += t3
            Op::Halt,
        ],
        register_count: 8,
        named_regs: vec![true, true, true, true, true, false, false, false], // t1,t2,t3 scratch
        ..Default::default()
    };
    let bytes = |mode| {
        let mut vm = Vm::new(&prog);
        vm.run_recording_mode(mode).expect("reversible");
        vm.take_journal().unwrap().erased_bytes()
    };
    assert!(
        bytes(ReverseMode::Adiabatic) < bytes(ReverseMode::Observable),
        "the recomputable chain is off the Adiabatic tape: {} < {}",
        bytes(ReverseMode::Adiabatic),
        bytes(ReverseMode::Observable)
    );
    let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
    let mut vm = Vm::new(&prog);
    vm.run_recording_mode(ReverseMode::Adiabatic)
        .expect("reversible");
    let tape = vm.take_journal().unwrap();
    vm.run_reverse(tape).expect("reverse");
    assert_eq!(
        all_initial,
        observe_state(&vm.save_debug_state()),
        "a recomputed chain reverses every register"
    );
}

#[test]
fn pebble_checkpoint_schedule_cuts_recompute_ops_at_the_landauer_floor() {
    // ★ CORNER 2 — the Bennett pebble schedule ★. A reverse-live chain t1→t2→t3 with t3 read FOUR
    // times re-walks the whole chain on every read under budget 0 (recompute-everything). A budget
    // of 1 CHECKPOINT — journaling the chain intermediate t2, whose Landauer cost is 0 (it is a
    // recomputable first-def, so its displaced value is `Nothing`) — bounds each above-checkpoint
    // read to a re-walk of length 1. So `recompute_ops` drops sharply, the Landauer floor is
    // UNCHANGED, and peak tape rises by exactly the checkpointed word: the space-for-time trade.
    use super::super::reverse::ReverseMode;
    let prog = CompiledProgram {
        constants: vec![
            Constant::Int(1),
            Constant::Int(2),
            Constant::Int(3),
            Constant::Int(4),
        ],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // a
            Op::LoadConst { dst: 1, idx: 1 }, // b
            Op::LoadConst { dst: 2, idx: 2 }, // c
            Op::LoadConst { dst: 3, idx: 3 }, // d
            Op::LoadConst { dst: 4, idx: 0 }, // place (named)
            Op::Add {
                dst: 5,
                lhs: 0,
                rhs: 1,
            }, // t1 = a+b (scratch, reverse-live)
            Op::Add {
                dst: 6,
                lhs: 5,
                rhs: 2,
            }, // t2 = t1+c (scratch, reverse-live, INTERMEDIATE)
            Op::Add {
                dst: 7,
                lhs: 6,
                rhs: 3,
            }, // t3 = t2+d (scratch, reverse-live, chain top)
            Op::RevAdd { place: 4, delta: 5 }, // read t1 → reverse-live
            Op::RevAdd { place: 4, delta: 6 }, // read t2 → reverse-live
            Op::RevAdd { place: 4, delta: 7 }, // read t3 (×4)
            Op::RevAdd { place: 4, delta: 7 },
            Op::RevAdd { place: 4, delta: 7 },
            Op::RevAdd { place: 4, delta: 7 },
            Op::Halt,
        ],
        register_count: 8,
        named_regs: vec![true, true, true, true, true, false, false, false],
        ..Default::default()
    };
    let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
    let run = |budget| {
        let mut vm = Vm::new(&prog);
        vm.run_recording_budgeted(ReverseMode::Adiabatic, budget)
            .expect("reversible");
        let tape = vm.take_journal().unwrap();
        let (erased, landauer) = (tape.erased_bytes(), tape.landauer_bytes());
        vm.run_reverse(tape).expect("reverse");
        (
            vm.recompute_ops(),
            erased,
            landauer,
            observe_state(&vm.save_debug_state()),
        )
    };
    let (ops0, er0, lb0, state0) = run(0);
    let (ops1, er1, lb1, state1) = run(1);
    assert!(
        ops1 < ops0,
        "a checkpoint cuts the re-walk: recompute_ops {ops1} (budget 1) < {ops0} (budget 0)"
    );
    assert_eq!(
        lb1, lb0,
        "checkpoints are recomputable ⇒ the Landauer floor is unchanged ({lb1} == {lb0})"
    );
    assert_eq!(
        lb0, 0,
        "the whole recomputable chain has zero Landauer cost"
    );
    assert!(
        er1 > er0,
        "the checkpoint occupies peak tape: erased {er1} (budget 1) > {er0} (budget 0)"
    );
    assert_eq!(state0, all_initial, "budget 0 reverses every register");
    assert_eq!(
        state1, all_initial,
        "budget 1 (checkpointed) reverses every register bit-exact"
    );
}

#[test]
fn adiabatic_recomputes_a_chain_with_a_recomputed_intermediate() {
    // The RECURSIVE case: `t1 = a+b` is itself reverse-live (a `RevXor` delta), and `t2 = t1+c` is
    // reverse-live too (a `RevAdd` delta). BOTH are recomputed. Reconstructing `t2` must first
    // reconstruct `t1` (bottom-up chain), which the recursive `recompute_into` does. Both are off
    // the Adiabatic tape, below Observable, full fidelity.
    use super::super::reverse::ReverseMode;
    let prog = CompiledProgram {
        constants: vec![
            Constant::Int(2),
            Constant::Int(3),
            Constant::Int(5),
            Constant::Int(9),
            Constant::Int(40),
        ],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // a = 2
            Op::LoadConst { dst: 1, idx: 1 }, // b = 3
            Op::LoadConst { dst: 2, idx: 2 }, // c = 5
            Op::LoadConst { dst: 3, idx: 3 }, // x = 9
            Op::LoadConst { dst: 4, idx: 4 }, // place = 40
            Op::Add {
                dst: 5,
                lhs: 0,
                rhs: 1,
            }, // t1 = a+b  (scratch, reverse-live)
            Op::RevXor { place: 3, delta: 5 }, // x ^= t1
            Op::Add {
                dst: 6,
                lhs: 5,
                rhs: 2,
            }, // t2 = t1+c  (scratch, reverse-live)
            Op::RevAdd { place: 4, delta: 6 }, // place += t2
            Op::Halt,
        ],
        register_count: 7,
        named_regs: vec![true, true, true, true, true, false, false], // t1(r5), t2(r6) scratch
        ..Default::default()
    };
    let bytes = |mode| {
        let mut vm = Vm::new(&prog);
        vm.run_recording_mode(mode).expect("reversible");
        vm.take_journal().unwrap().erased_bytes()
    };
    assert!(
        bytes(ReverseMode::Adiabatic) < bytes(ReverseMode::Observable),
        "both chain steps recomputed off the tape: {} < {}",
        bytes(ReverseMode::Adiabatic),
        bytes(ReverseMode::Observable)
    );
    let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
    let mut vm = Vm::new(&prog);
    vm.run_recording_mode(ReverseMode::Adiabatic)
        .expect("reversible");
    let tape = vm.take_journal().unwrap();
    vm.run_reverse(tape).expect("reverse");
    assert_eq!(
        all_initial,
        observe_state(&vm.save_debug_state()),
        "recursive-chain recompute reverses every register"
    );
}

#[test]
fn adiabatic_recomputes_scratch_inside_a_function_frame() {
    // CROSS-FRAME recompute. `f`'s `t = a*b` is reverse-live scratch INSIDE the callee frame (a
    // `RevAdd` delta), at an absolute slot beyond Main's frame (so it aliases nothing observable).
    // Adiabatic recomputes it THERE — dropped from the tape and re-run during `f`'s reverse with
    // `self.base` set to the callee frame — below Observable, and the whole call reverses.
    use super::super::instruction::CompiledFunction;
    use super::super::reverse::ReverseMode;
    let f = CompiledFunction {
        name: crate::intern::Symbol::from_index(0),
        entry_pc: 4,
        param_count: 2,
        register_count: 4,
        captures: vec![],
        named_regs: vec![true, true, true, false], // r0,r1 params; r2 acc; r3 = t (scratch)
        param_kinds: vec![],
        ret_kind: None,
        param_types: vec![],
        return_type: None,
        mutable_param_regs: vec![],
        cow_exempt_regs: vec![],
    };
    let prog = CompiledProgram {
        constants: vec![Constant::Int(5), Constant::Int(6), Constant::Int(0)],
        code: vec![
            Op::LoadConst { dst: 2, idx: 0 }, // Main: a=5 at args_start 2
            Op::LoadConst { dst: 3, idx: 1 }, // Main: b=6
            Op::Call {
                dst: 0,
                func: 0,
                args_start: 2,
                arg_count: 2,
            }, // r0 = f(a,b)
            Op::Halt,
            Op::LoadConst { dst: 2, idx: 2 }, // f pc4: acc = 0
            Op::Mul {
                dst: 3,
                lhs: 0,
                rhs: 1,
            }, // f pc5: t = a*b  (scratch, reverse-live)
            Op::RevAdd { place: 2, delta: 3 }, // f pc6: acc += t
            Op::Return { src: 2 },            // f pc7: return acc
        ],
        register_count: 4,
        named_regs: vec![true, false, false, false], // Main r0 (result) named
        functions: vec![f],
        ..Default::default()
    };
    let bytes = |mode| {
        let mut vm = Vm::new(&prog);
        vm.run_recording_mode(mode).expect("reversible");
        vm.take_journal().unwrap().erased_bytes()
    };
    assert!(
        bytes(ReverseMode::Adiabatic) < bytes(ReverseMode::Observable),
        "callee-frame scratch is recomputed off the tape: {} < {}",
        bytes(ReverseMode::Adiabatic),
        bytes(ReverseMode::Observable)
    );
    let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
    let mut vm = Vm::new(&prog);
    vm.run_recording_mode(ReverseMode::Adiabatic)
        .expect("reversible");
    let tape = vm.take_journal().unwrap();
    vm.run_reverse(tape).expect("reverse");
    assert_eq!(
        all_initial,
        observe_state(&vm.save_debug_state()),
        "cross-frame recompute reverses every register"
    );
}

#[test]
fn adiabatic_recompute_falls_back_when_an_input_is_unstable() {
    // PIECE 2 soundness: if a recompute recipe's INPUT is modified after the def, re-running the
    // def on reverse would reconstruct the WRONG value. The stability check must reject it — `t` is
    // then journaled (no recompute), and it still reverses exactly. Here `a` is reassigned after
    // `t = a+b`, so `t` must NOT be recomputed.
    use super::super::reverse::ReverseMode;
    let prog = CompiledProgram {
        constants: vec![
            Constant::Int(3),
            Constant::Int(4),
            Constant::Int(10),
            Constant::Int(99),
        ],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // a = 3
            Op::LoadConst { dst: 1, idx: 1 }, // b = 4
            Op::LoadConst { dst: 2, idx: 2 }, // place = 10
            Op::Add {
                dst: 3,
                lhs: 0,
                rhs: 1,
            }, // t = a+b = 7 (scratch, reverse-live)
            Op::LoadConst { dst: 0, idx: 3 }, // a = 99  (INPUT modified after t's def!)
            Op::RevAdd { place: 2, delta: 3 }, // place += t
            Op::Halt,
        ],
        register_count: 4,
        named_regs: vec![true, true, true, false],
        ..Default::default()
    };
    // Recompute must NOT fire (unstable input), so Adiabatic journals `t` — same tape as Observable.
    let bytes = |mode| {
        let mut vm = Vm::new(&prog);
        vm.run_recording_mode(mode).expect("reversible");
        vm.take_journal().unwrap().erased_bytes()
    };
    assert_eq!(
        bytes(ReverseMode::Adiabatic),
        bytes(ReverseMode::Observable),
        "an unstable input must disable recompute — t is journaled, not reconstructed"
    );
    // And it still reverses exactly.
    let all_initial = observe_state(&Vm::new(&prog).save_debug_state());
    let mut vm = Vm::new(&prog);
    vm.run_recording_mode(ReverseMode::Adiabatic)
        .expect("reversible");
    let tape = vm.take_journal().unwrap();
    vm.run_reverse(tape).expect("reverse");
    assert_eq!(
        all_initial,
        observe_state(&vm.save_debug_state()),
        "unstable-input program reverses exactly"
    );
}

#[test]
fn unsupported_op_declines_rather_than_mis_reverses() {
    // The honest contract: a program using a TRULY-EXTERNAL effect (here dialing the network
    // relay) is REFUSED — the honest Landauer wall — never silently pretending an un-sendable
    // message can be un-sent.
    let prog = CompiledProgram {
        constants: vec![],
        code: vec![Op::NetConnect { url: 0 }, Op::Halt],
        register_count: 1,
        ..Default::default()
    };
    let mut vm = Vm::new(&prog);
    let err = vm
        .run_recording()
        .expect_err("NetConnect is an external effect, not reversible");
    assert!(err.contains("not yet reversible"), "got: {err}");
    // The barrier is NAMED — the honest Landauer wall, not a generic "unsupported".
    assert!(
        err.contains("network") && err.contains("Landauer"),
        "expected a named network barrier, got: {err}"
    );
}
