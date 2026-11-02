//! fabric_handoff_lock — Phase 4: the INTERPRETER ↔ FABRIC hand-off ("Futamura on silicon").
//!
//! The soft-core is a universal fetch-decode-execute INTERPRETER in silicon; the fabric islands (P1
//! division, P2 memory, P3 branch composition) are WIRED datapaths that realize a committed hot function
//! with no fetch. The hand-off's soundness requirement is that they AGREE: interpreting a function must
//! give the bit-identical result to running its fabric island — otherwise dispatching a `Call` to the
//! island would silently change the program's answer.
//!
//! This lock proves that agreement at the model level, across EVERY fabric capability: for the same
//! function and inputs, the cycle-accurate soft-core model (`run_soft_core_typed`, interpreting the exact
//! chip words) equals the fabric evaluator, and both equal the VM. Green here == the soft-core may replace
//! its interpretation of any of these functions with the wired island transparently. (The `is_fabric`
//! decode bit + `fabric_soc` dispatch RTL that physically routes `Op::Call` into the island — pulsing
//! `start`, stalling on `stage_done`, retiring to `R[dst]` — is the silicon follow-on; this pins the
//! semantic contract it must honor.)

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, eval_decision_fabric, eval_loop_fabric_seq,
    eval_program_cfg_fabric, loop_fabric_from_source, program_cfg_fabric,
};
use logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words;
use logicaffeine_compile::codegen_sva::soft_core_model::{run_soft_core_typed, ShowVal};
use logicaffeine_compile::compile::vm_outcome;

// One committed hot function per fabric capability.
const MICROPRICE: &str = "\
## To microprice (bid: Int, ask: Int, bidsz: Int, asksz: Int) -> Int:
    Let num be bid * asksz + ask * bidsz.
    Let den be bidsz + asksz.
    Return num / den.
";
const SEQSUM: &str = "\
## To seqsum (xs: Seq of Int) -> Int:
    Let mutable acc be 0.
    Repeat for x in xs:
        Set acc to acc + x.
    Return acc.
";
const CLAMP: &str = "\
## To clamp (x: Int, lo: Int, hi: Int) -> Int:
    If x is less than lo:
        Return lo.
    If x is greater than hi:
        Return hi.
    Return x.
";

fn arg(n: i64) -> String {
    if n < 0 {
        format!("(0 - {})", -n)
    } else {
        n.to_string()
    }
}

/// The INTERPRETER path: compile the whole `## To f … ## Main: Show f(args).` program to chip words and
/// run the cycle-accurate soft-core model — exactly what the fetch-decode CPU does on silicon.
fn soft_core_interpret(prog: &str) -> i64 {
    let words = compile_to_full_words(prog).unwrap_or_else(|e| panic!("encode: {e}"));
    let shows = run_soft_core_typed(&words, 50_000_000);
    let last = shows.last().expect("the program showed a value");
    match last {
        ShowVal::Int(v) => *v as i64,
        other => panic!("expected an integer Show, got {other:?}"),
    }
}

fn vm_show(prog: &str) -> i64 {
    let out = vm_outcome(prog);
    assert!(out.error.is_none(), "VM error: {:?}", out.error);
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
fn division_island_handoff_matches_the_interpreter() {
    // P1 island: the soft-core interpreting the microprice equals the decision fabric realizing it.
    let df = decision_fabric_from_source(MICROPRICE, "microprice");
    let mut st = 0xD1F1_5107_u64;
    for _ in 0..150 {
        let bid = (mix(&mut st) % 20000) as i64;
        let ask = bid + (mix(&mut st) % 50) as i64;
        let bidsz = 1 + (mix(&mut st) % 500) as i64;
        let asksz = 1 + (mix(&mut st) % 500) as i64;
        let inputs = [bid, ask, bidsz, asksz];
        let call = format!(
            "microprice({}, {}, {}, {})",
            arg(bid),
            arg(ask),
            arg(bidsz),
            arg(asksz)
        );
        let prog = format!("{MICROPRICE}## Main\n    Show {call}.\n");
        let interp = soft_core_interpret(&prog);
        let fabric = eval_decision_fabric(&df, &inputs);
        assert_eq!(
            interp, fabric,
            "microprice: interpreter {interp} != fabric {fabric} on {inputs:?}"
        );
        assert_eq!(
            interp,
            vm_show(&prog),
            "microprice: interpreter != VM on {inputs:?}"
        );
    }
}

#[test]
fn memory_island_handoff_matches_the_interpreter() {
    // P2 island: the soft-core interpreting the sequence fold equals the memory-loop fabric realizing it.
    let lf = loop_fabric_from_source(SEQSUM, "seqsum");
    let params = vec![0i64; lf.n_in];
    let mut st = 0xADD5_E000_u64;
    for _ in 0..150 {
        let n = (mix(&mut st) as usize) % 10;
        let seq: Vec<i64> = (0..n)
            .map(|_| (mix(&mut st) % 4001) as i64 - 2000)
            .collect();
        let build = if seq.is_empty() {
            "a new Seq of Int".to_string()
        } else {
            format!(
                "[{}]",
                seq.iter().map(|e| arg(*e)).collect::<Vec<_>>().join(", ")
            )
        };
        let prog = format!("{SEQSUM}## Main\n    Let xs be {build}.\n    Show seqsum(xs).\n");
        let interp = soft_core_interpret(&prog);
        let fabric = eval_loop_fabric_seq(&lf, &params, &seq);
        assert_eq!(
            interp, fabric,
            "seqsum: interpreter {interp} != fabric {fabric} on {seq:?}"
        );
        assert_eq!(
            interp,
            vm_show(&prog),
            "seqsum: interpreter != VM on {seq:?}"
        );
    }
}

#[test]
fn branch_island_handoff_matches_the_interpreter() {
    // P3 island: the soft-core interpreting the multi-block clamp equals the CFG controller realizing it.
    let pf = program_cfg_fabric(CLAMP, "clamp");
    let mut st = 0xB_A_C_11_u64;
    for _ in 0..150 {
        let x = (mix(&mut st) % 2001) as i64 - 1000;
        let lo = (mix(&mut st) % 401) as i64 - 200;
        let hi = lo + (mix(&mut st) % 400) as i64;
        let call = format!("clamp({}, {}, {})", arg(x), arg(lo), arg(hi));
        let prog = format!("{CLAMP}## Main\n    Show {call}.\n");
        let interp = soft_core_interpret(&prog);
        let fabric = eval_program_cfg_fabric(&pf, &[x, lo, hi]);
        assert_eq!(
            interp, fabric,
            "clamp: interpreter {interp} != fabric {fabric} on ({x},{lo},{hi})"
        );
        assert_eq!(
            interp,
            vm_show(&prog),
            "clamp: interpreter != VM on ({x},{lo},{hi})"
        );
    }
}
