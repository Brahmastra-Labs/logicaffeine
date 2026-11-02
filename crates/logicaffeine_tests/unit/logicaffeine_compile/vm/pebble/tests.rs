use super::*;
use crate::vm::instruction::{Constant, Op};

fn prog(code: Vec<Op>, named: Vec<bool>) -> CompiledProgram {
    CompiledProgram {
        constants: vec![Constant::Int(0)],
        register_count: named.len().max(1),
        code,
        named_regs: named,
        ..Default::default()
    }
}

#[test]
fn fullstate_keeps_every_write() {
    let p = prog(
        vec![Op::LoadConst { dst: 0, idx: 0 }, Op::Halt],
        vec![false],
    );
    assert!(plan(&p, ReverseMode::FullState)
        .iter()
        .all(|d| *d == JournalDecision::Keep));
}

#[test]
fn observable_elides_pure_scratch_but_keeps_named() {
    // r0 is a named user variable; r1 is compiler scratch feeding nothing a reverse reads.
    let p = prog(
        vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 0 },
            Op::Halt,
        ],
        vec![true, false],
    );
    let d = plan(&p, ReverseMode::Observable);
    assert_eq!(d[0], JournalDecision::Keep, "the named write is kept");
    assert_eq!(
        d[1],
        JournalDecision::Elide,
        "the pure-scratch write is elided"
    );
}

#[test]
fn observable_keeps_reverse_read_scratch() {
    // r1 is scratch, but it is a `RevAdd` delta — a REVERSE-READ operand, so its value must
    // survive; eliding it would corrupt the inverse. The pebbler must keep it.
    let p = prog(
        vec![
            Op::LoadConst { dst: 1, idx: 0 },
            Op::RevAdd { place: 0, delta: 1 },
            Op::Halt,
        ],
        vec![true, false],
    );
    assert_eq!(
        plan(&p, ReverseMode::Observable)[0],
        JournalDecision::Keep,
        "a reverse-read scratch write must be kept, never elided"
    );
}

#[test]
fn frame_overlap_keeps_aliased_scratch_elides_beyond_frame() {
    // A function `f` is called at `args_start = 3` (base 3). Its `r2` aliases Main's absolute slot
    // 5 (Main's NAMED `y`) → KEEP; its `r3` aliases slot 6, beyond Main's 6-register frame → ELIDE.
    use crate::vm::instruction::CompiledFunction;
    let f = CompiledFunction {
        name: crate::intern::Symbol::from_index(0),
        entry_pc: 4,
        param_count: 1,
        register_count: 4,
        captures: vec![],
        named_regs: vec![true, false, false, false], // r0 param; r1,r2,r3 scratch
        param_kinds: vec![],
        ret_kind: None,
        param_types: vec![],
        return_type: None,
        mutable_param_regs: vec![],
        cow_exempt_regs: vec![],
    };
    let p = CompiledProgram {
        constants: vec![Constant::Int(0)],
        code: vec![
            Op::LoadConst { dst: 3, idx: 0 }, // pc0: arg at args_start 3
            Op::Call {
                dst: 5,
                func: 0,
                args_start: 3,
                arg_count: 1,
            }, // pc1
            Op::Halt,                         // pc2
            Op::Halt,                         // pc3 (pad; f starts at 4)
            Op::Mul {
                dst: 2,
                lhs: 0,
                rhs: 0,
            }, // pc4: f.r2 → abs slot 3+2 = 5 (Main's y)
            Op::Mul {
                dst: 3,
                lhs: 0,
                rhs: 0,
            }, // pc5: f.r3 → abs slot 3+3 = 6 (beyond Main)
            Op::Return { src: 2 },            // pc6
        ],
        register_count: 6,
        named_regs: vec![false, false, false, false, false, true], // Main slot 5 = y (named)
        functions: vec![f],
        ..Default::default()
    };
    let d = plan(&p, ReverseMode::Observable);
    assert_eq!(
        d[4],
        JournalDecision::Keep,
        "callee scratch aliasing a NAMED caller slot is kept"
    );
    assert_eq!(
        d[5],
        JournalDecision::Elide,
        "callee scratch BEYOND the caller frame is elided"
    );
}

#[test]
fn recursion_keeps_scratch_aliasing_a_main_named_slot_elides_the_rest() {
    // ★ CORNER 4 OVER-ELISION GUARD ★ — a RECURSIVE `f` (self-call at args_start 1) runs at bases
    // {2,3,4,5} (bounded to Main's frame). Its scratch `r2` aliases Main's absolute slot 4 at the
    // shallow base 2 — and slot 4 is Main's NAMED `y` — so eliding it would corrupt `y`; it MUST be
    // kept. Its scratch `r3` aliases only non-named / beyond-Main slots at every base, so it elides.
    // The bounded-base fixpoint TERMINATES on the cycle and still declines the unsound elision.
    use crate::vm::instruction::CompiledFunction;
    let f = CompiledFunction {
        name: crate::intern::Symbol::from_index(0),
        entry_pc: 5,
        param_count: 1,
        register_count: 4,
        captures: vec![],
        named_regs: vec![true, false, false, false], // r0 param; r1,r2,r3 scratch
        param_kinds: vec![],
        ret_kind: None,
        param_types: vec![],
        return_type: None,
        mutable_param_regs: vec![],
        cow_exempt_regs: vec![],
    };
    let p = CompiledProgram {
        constants: vec![Constant::Int(0)],
        code: vec![
            Op::LoadConst { dst: 2, idx: 0 }, // pc0: arg at args_start 2
            Op::Call {
                dst: 4,
                func: 0,
                args_start: 2,
                arg_count: 1,
            }, // pc1: f → Main slot 4 (y)
            Op::Halt,                         // pc2
            Op::Halt,                         // pc3 (pad)
            Op::Halt,                         // pc4 (pad; f starts at 5)
            Op::Mul {
                dst: 2,
                lhs: 0,
                rhs: 0,
            }, // pc5: f.r2 → abs {4,5,6,7}; base 2 → slot 4 = y
            Op::Mul {
                dst: 3,
                lhs: 0,
                rhs: 0,
            }, // pc6: f.r3 → abs {5,6,7,8}; never a Main-named slot
            Op::Call {
                dst: 1,
                func: 0,
                args_start: 1,
                arg_count: 1,
            }, // pc7: RECURSIVE self-call
            Op::Return { src: 2 },            // pc8
        ],
        register_count: 6,
        named_regs: vec![false, false, false, false, true, false], // Main slot 4 = y (named)
        functions: vec![f],
        ..Default::default()
    };
    let d = plan(&p, ReverseMode::Observable);
    assert_eq!(
        d[5],
        JournalDecision::Keep,
        "recursive scratch aliasing a Main-NAMED slot must be kept"
    );
    assert_eq!(
        d[6],
        JournalDecision::Elide,
        "recursive scratch aliasing no Main-named slot elides"
    );
}

#[test]
fn mutual_recursion_terminates_and_produces_a_plan() {
    // The bounded-base fixpoint must TERMINATE on a call-graph CYCLE that is not a self-loop:
    // `f → g → f`. Bases grow by each hop's args_start but are truncated at `main_size`, so the
    // monotone fixpoint converges instead of spinning — `plan` returns (a hang would fail the test).
    use crate::vm::instruction::CompiledFunction;
    let mk = |entry_pc| CompiledFunction {
        name: crate::intern::Symbol::from_index(0),
        entry_pc,
        param_count: 1,
        register_count: 3,
        captures: vec![],
        named_regs: vec![true, false, false],
        param_kinds: vec![],
        ret_kind: None,
        param_types: vec![],
        return_type: None,
        mutable_param_regs: vec![],
        cow_exempt_regs: vec![],
    };
    let p = CompiledProgram {
        constants: vec![Constant::Int(0)],
        code: vec![
            Op::Call {
                dst: 0,
                func: 0,
                args_start: 1,
                arg_count: 1,
            }, // pc0: Main → f
            Op::Halt, // pc1
            Op::Call {
                dst: 0,
                func: 1,
                args_start: 1,
                arg_count: 1,
            }, // pc2: f → g   (f entry 2)
            Op::Return { src: 0 }, // pc3
            Op::Call {
                dst: 0,
                func: 0,
                args_start: 1,
                arg_count: 1,
            }, // pc4: g → f   (g entry 4)
            Op::Return { src: 0 }, // pc5
        ],
        register_count: 8,
        named_regs: vec![true, false, false, false, false, false, false, false],
        functions: vec![mk(2), mk(4)],
        ..Default::default()
    };
    let d = plan(&p, ReverseMode::Observable);
    assert_eq!(
        d.len(),
        p.code.len(),
        "the fixpoint terminates on the f→g→f cycle and yields a plan"
    );
}

#[test]
fn checkpoint_defs_picks_the_chain_intermediate_nearest_the_top() {
    // Corner 2: on the reverse-live chain t1→t2 (both read by a Bijection), `t1` is the INTERMEDIATE
    // (its register feeds `t2`'s def) and `t2` is the chain top. budget 0 checkpoints nothing;
    // budget 1 checkpoints the intermediate `t1` (def pc 2) so a read of `t2` stops its re-walk there.
    let p = CompiledProgram {
        constants: vec![Constant::Int(1)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 }, // a (named)
            Op::LoadConst { dst: 1, idx: 0 }, // c (named)
            Op::Add {
                dst: 2,
                lhs: 0,
                rhs: 1,
            }, // pc2: t1 = a+c  (scratch, reverse-live, INTERMEDIATE)
            Op::Add {
                dst: 3,
                lhs: 2,
                rhs: 0,
            }, // pc3: t2 = t1+a  (scratch, reverse-live, TOP)
            Op::RevAdd { place: 4, delta: 2 }, // read t1
            Op::RevAdd { place: 4, delta: 3 }, // read t2
            Op::Halt,
        ],
        register_count: 5,
        named_regs: vec![true, true, false, false, true], // t1 (r2), t2 (r3) scratch; place named
        ..Default::default()
    };
    assert!(
        checkpoint_defs(&p, 0).is_empty(),
        "budget 0 checkpoints nothing (recompute everything)"
    );
    let cp = checkpoint_defs(&p, 1);
    assert_eq!(cp.len(), 1, "budget 1 checkpoints exactly one node");
    assert!(
        cp.contains(&2),
        "the checkpoint is the chain INTERMEDIATE t1 (def pc 2), not the top t2"
    );
}

#[test]
fn recomputable_finds_stable_reverse_live_scratch_only() {
    // `t = a+b` (scratch r2) is a `RevAdd` delta (reverse-live) with stable inputs → recomputable.
    let good = CompiledProgram {
        constants: vec![Constant::Int(1)],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 0 },
            Op::Add {
                dst: 2,
                lhs: 0,
                rhs: 1,
            }, // t = a+b (r2 scratch)
            Op::RevAdd { place: 3, delta: 2 }, // reverse-live use of t
            Op::Halt,
        ],
        register_count: 4,
        named_regs: vec![true, true, false, true],
        ..Default::default()
    };
    let rec = recomputable(&good);
    assert_eq!(
        rec.len(),
        1,
        "the stable reverse-live scratch is recomputable"
    );
    assert_eq!(
        (rec[0].0, rec[0].1),
        (None, 2),
        "it is Main-frame register r2"
    );

    // Modify an input after the def → NOT recomputable (would reconstruct the wrong value).
    let mut unstable = good.clone();
    unstable.code.insert(4, Op::LoadConst { dst: 0, idx: 0 }); // rewrite `a` after t's def
                                                               // (RevAdd shifts to pc4→pc3 order preserved; a new write to r0 lands after the Add at pc2)
    assert!(
        recomputable(&unstable).is_empty(),
        "an unstable input disables recompute"
    );
}

#[test]
fn adiabatic_elides_like_observable() {
    // `Adiabatic` uses the same minimal tape as `Observable` (it recovers full fidelity at reverse
    // END by resetting elided scratch to its default, not by journaling more).
    let p = prog(
        vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 0 },
            Op::Halt,
        ],
        vec![true, false],
    );
    assert_eq!(
        plan(&p, ReverseMode::Adiabatic),
        plan(&p, ReverseMode::Observable)
    );
}

#[test]
fn empty_named_regs_elides_nothing() {
    // The safe fallback: no name info ⇒ everything observable ⇒ Observable behaves like FullState.
    let p = prog(vec![Op::LoadConst { dst: 0, idx: 0 }, Op::Halt], vec![]);
    assert!(plan(&p, ReverseMode::Observable)
        .iter()
        .all(|d| *d == JournalDecision::Keep));
}

#[test]
fn report_counts_journaled_vs_elided() {
    let p = prog(
        vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 0 },
            Op::Halt,
        ],
        vec![true, false],
    );
    let r = report(&p, ReverseMode::Observable);
    assert_eq!(
        (r.journaled, r.elided),
        (1, 1),
        "one named write kept, one scratch elided"
    );
}
