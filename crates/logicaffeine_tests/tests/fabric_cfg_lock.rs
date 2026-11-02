//! fabric_cfg_lock — Phase 3 (first increment) of the spatial-interpreter campaign: COMPOSITION.
//!
//! P1 built a division functional-unit tile; P2 built a memory-read tile for sequence folds. Each lowers a
//! SINGLE construct. A whole function is a control-flow GRAPH of basic blocks: real kernels loop and THEN
//! transform the result (`Return (sum 1..n) * 2`). This lock pins the first composition: a LOOP island
//! feeding a straight-line EPILOGUE island, sequenced into one whole-function fabric whose evaluation
//! equals the running VM EXACTLY over a broad fuzz — for numeric range loops AND sequence (memory) loops.
//!
//! RED before this increment: `loop_fabric_from_source` panicked ("unexpected post-loop op") on any
//! computation after the loop. GREEN: post-loop straight-line ops are captured as an epilogue island over
//! the loop's final state, and the eval sequences loop→epilogue. The full multi-block controller (branches
//! BETWEEN blocks, an FSM whose states are basic blocks, live-value channels, iverilog + 1-inductive SAT)
//! is the remaining P3 capstone; this proves the composition core: islands sequenced, live value threaded.

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_loop_fabric, eval_loop_fabric_seq, eval_program_cfg_fabric, loop_fabric_from_source,
    program_cfg_fabric, LoopCell, Term,
};
use logicaffeine_compile::compile::vm_outcome;

// ── The general multi-block CFG controller: branchy functions with EARLY RETURNS (multiple return blocks),
//    which the single-output decision fabric cannot if-convert. ──

// A three-way clamp: two early returns then a fall-through return — four basic blocks, two Branches.
const CLAMP: &str = "\
## To clamp (x: Int, lo: Int, hi: Int) -> Int:
    If x is less than lo:
        Return lo.
    If x is greater than hi:
        Return hi.
    Return x.
";

// A sign function: nested branches over one input, three return paths.
const SIGN: &str = "\
## To sign (n: Int) -> Int:
    If n is less than 0:
        Return 0 - 1.
    If n is equal to 0:
        Return 0.
    Return 1.
";

// A NUMERIC range loop with a post-loop transform: (sum 1..n) then `* 2`.
const SUM_DOUBLE: &str = "\
## To sumdouble (n: Int) -> Int:
    Let mutable s be 0.
    Repeat for i from 1 to n:
        Set s to s + i.
    Return s * 2.
";

// A numeric loop with a MULTI-OP epilogue: (sum 1..n) then `* 3 - 1`.
const SUM_AFFINE: &str = "\
## To sumaffine (n: Int) -> Int:
    Let mutable s be 0.
    Repeat for i from 1 to n:
        Set s to s + i.
    Return s * 3 - 1.
";

// A SEQUENCE loop with a post-loop transform: (fold xs) then `* 2` — composes P2's MemRead island with a
// P3 epilogue island.
const SEQSUM_DOUBLE: &str = "\
## To seqdouble (xs: Seq of Int) -> Int:
    Let mutable s be 0.
    Repeat for x in xs:
        Set s to s + x.
    Return s * 2.
";

fn arg(n: i64) -> String {
    if n < 0 {
        format!("(0 - {})", -n)
    } else {
        n.to_string()
    }
}

fn vm_int(func_src: &str, name: &str, call_arg: &str) -> i64 {
    let src = format!("{func_src}## Main\n    Show {name}({call_arg}).\n");
    let out = vm_outcome(&src);
    assert!(
        out.error.is_none(),
        "VM error on {name}({call_arg}): {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int")
}

fn mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[test]
fn loop_with_epilogue_lowers_to_a_composed_fabric() {
    // The whole function lowers to a loop island PLUS a non-empty epilogue island (the `* 2` transform) —
    // the first CFG composition, not a single construct.
    let lf = loop_fabric_from_source(SUM_DOUBLE, "sumdouble");
    assert!(
        !lf.epilogue.is_empty(),
        "the post-loop `* 2` becomes an epilogue island"
    );
    let arith = lf
        .epilogue
        .iter()
        .filter(|c| matches!(c, LoopCell::Arith { .. }))
        .count();
    assert!(
        arith >= 1,
        "the epilogue carries the transform's arith tile(s)"
    );
}

#[test]
fn numeric_loop_epilogue_matches_vm() {
    // RED before P3: post-loop ops panicked. GREEN: loop→epilogue composition folds identically to the VM.
    let df = loop_fabric_from_source(SUM_DOUBLE, "sumdouble");
    let af = loop_fabric_from_source(SUM_AFFINE, "sumaffine");
    let mut st = 0xC0FFEE_C0DE_u64;
    for _ in 0..200 {
        let n = (mix(&mut st) % 200) as i64; // 0..199 (n == 0 → empty loop, s == 0)
        assert_eq!(
            eval_loop_fabric(&df, &[n]),
            vm_int(SUM_DOUBLE, "sumdouble", &arg(n)),
            "sumdouble fabric ≠ VM on n={n}"
        );
        assert_eq!(
            eval_loop_fabric(&af, &[n]),
            vm_int(SUM_AFFINE, "sumaffine", &arg(n)),
            "sumaffine fabric ≠ VM on n={n}"
        );
    }
}

#[test]
fn sequence_loop_epilogue_matches_vm() {
    // The composition also works over a MEMORY loop: fold the sequence (P2's MemRead island), then the
    // epilogue transform (P3). Whole-function ≡ VM over random sequences.
    let lf = loop_fabric_from_source(SEQSUM_DOUBLE, "seqdouble");
    assert_eq!(
        lf.seq_input,
        Some(0),
        "seqdouble iterates its Seq parameter"
    );
    assert!(
        !lf.epilogue.is_empty(),
        "the post-loop `* 2` is an epilogue island atop the memory loop"
    );
    let params = vec![0i64; lf.n_in];
    let mut st = 0x5EED_B00C_u64;
    for _ in 0..300 {
        let n = (mix(&mut st) as usize) % 11; // 0..10 (includes the empty sequence)
        let seq: Vec<i64> = (0..n)
            .map(|_| (mix(&mut st) % 4001) as i64 - 2000)
            .collect();
        let list = if seq.is_empty() {
            "a new Seq of Int".to_string()
        } else {
            format!(
                "[{}]",
                seq.iter().map(|e| arg(*e)).collect::<Vec<_>>().join(", ")
            )
        };
        let want = vm_int(SEQSUM_DOUBLE, "seqdouble", &list);
        let got = eval_loop_fabric_seq(&lf, &params, &seq);
        assert_eq!(
            got, want,
            "seqdouble fabric ≠ VM on {seq:?} (got {got}, want {want})"
        );
    }
}

fn clamp_ref(x: i64, lo: i64, hi: i64) -> i64 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

#[test]
fn early_returns_lower_to_a_multi_block_controller() {
    // The three-way clamp is NOT a single select-DAG — it has multiple return blocks. It lowers to several
    // basic-block islands sequenced by a controller with predicated (Branch) terminators.
    let pf = program_cfg_fabric(CLAMP, "clamp");
    assert!(
        pf.blocks.len() >= 3,
        "clamp has several basic blocks, got {}",
        pf.blocks.len()
    );
    let branches = pf
        .blocks
        .iter()
        .filter(|b| matches!(b.term, Term::Branch { .. }))
        .count();
    let returns = pf
        .blocks
        .iter()
        .filter(|b| matches!(b.term, Term::Return(_)))
        .count();
    assert!(
        branches >= 2,
        "two comparisons ⇒ at least two Branch terminators, got {branches}"
    );
    assert!(
        returns >= 3,
        "three return paths ⇒ at least three Return terminators, got {returns}"
    );
}

#[test]
fn multi_block_branch_controller_matches_vm() {
    // RED before the P3 capstone: the fabric handled one decision or one loop. GREEN: the whole-function
    // block controller walks the CFG (branches + early returns) identically to the VM over a broad fuzz.
    let clamp = program_cfg_fabric(CLAMP, "clamp");
    let sign = program_cfg_fabric(SIGN, "sign");
    let mut st = 0xC1A3_5164_u64;
    for _ in 0..400 {
        let x = (mix(&mut st) % 2001) as i64 - 1000;
        let lo = (mix(&mut st) % 401) as i64 - 200;
        let hi = lo + (mix(&mut st) % 400) as i64; // hi >= lo
        assert_eq!(
            eval_program_cfg_fabric(&clamp, &[x, lo, hi]),
            clamp_ref(x, lo, hi),
            "clamp fabric ≠ reference on ({x},{lo},{hi})"
        );
        assert_eq!(
            eval_program_cfg_fabric(&clamp, &[x, lo, hi]),
            vm_int(
                CLAMP,
                "clamp",
                &format!("{}, {}, {}", arg(x), arg(lo), arg(hi))
            ),
            "clamp fabric ≠ VM on ({x},{lo},{hi})"
        );
        let n = (mix(&mut st) % 21) as i64 - 10; // −10..10 straddles the sign boundaries
        assert_eq!(
            eval_program_cfg_fabric(&sign, &[n]),
            vm_int(SIGN, "sign", &arg(n)),
            "sign fabric ≠ VM on n={n}"
        );
    }
}
