//! fabric_mem_lock — Phase 2 of the spatial-interpreter campaign: COLLECTIONS as a spatial resource.
//!
//! A counted range loop (`Repeat for i from lo to hi`) lowers to a CGRA whose induction counter clocks a
//! wired body datapath. Real programs also stream a SEQUENCE (`Repeat for x in xs`): the plan's insight is
//! that `for x in xs` IS a counted loop `iv in 0..=len-1` whose body's first act is a MEMORY READ
//! `xs[iv]` — so the induction variable is the ADDRESS GENERATOR and the sequence rides a scratchpad.
//!
//! This lock pins the lowering's SOUNDNESS on the reproducible (no-simulator) side: a function that folds
//! a `Seq of Int` lowers to a loop fabric flagged as a sequence loop (`seq_input`), with a `MemRead` tile
//! reading `xs[iv]`, and its evaluation equals the running VM EXACTLY over a broad seeded fuzz — random
//! lengths (including the empty sequence, which must run zero iterations) and random signed values.
//!
//! RED before Phase 2: `loop_fabric_from_source` panics ("no NewRange (not a counted loop)") — the fabric
//! took scalar range loops only. GREEN: the sequence-iteration prologue (`IterPrepare`/`IterNext` with no
//! `NewRange`) is recognized and lowered to the memory-read loop. The scratchpad RTL emission (the
//! induction counter driving the address bus) is the documented follow-on; this proves the LOWERING.

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_loop_fabric_seq, loop_fabric_from_source, LoopCell,
};
use logicaffeine_compile::compile::vm_outcome;

const SEQSUM_FN: &str = "\
## To seqsum (xs: Seq of Int) -> Int:
    Let mutable acc be 0.
    Repeat for x in xs:
        Set acc to acc + x.
    Return acc.
";

const SUMSQ_FN: &str = "\
## To sumsq (xs: Seq of Int) -> Int:
    Let mutable acc be 0.
    Repeat for x in xs:
        Set acc to acc + x * x.
    Return acc.
";

fn arg(n: i64) -> String {
    if n < 0 {
        format!("(0 - {})", -n)
    } else {
        n.to_string()
    }
}

/// Run `func(name)` on the VM over a literal `Seq of Int` and return the printed integer.
fn vm_fold(func_src: &str, name: &str, elems: &[i64]) -> i64 {
    let list = elems.iter().map(|e| arg(*e)).collect::<Vec<_>>().join(", ");
    // `a new Seq of Int` gives a typed empty sequence the parser accepts where `[]` would be ambiguous.
    let build = if elems.is_empty() {
        "a new Seq of Int".to_string()
    } else {
        format!("[{list}]")
    };
    let src = format!("{func_src}## Main\n    Let xs be {build}.\n    Show {name}(xs).\n");
    let out = vm_outcome(&src);
    assert!(
        out.error.is_none(),
        "VM error on {name}({elems:?}): {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int")
}

/// Run `func(name)` on the VM and reduce its EXACT (arbitrary-precision) result to the machine word mod 2^64 —
/// precisely what the fixed-width fabric computes. The VM's `Int` is UNBOUNDED (it promotes to BigInt); the
/// fabric is its fast fixed-width projection, so the DIFFERENTIAL LOCK is `fabric (as u64) == VM mod 2^64`,
/// which holds for EVERY input with NO range restriction — the chip and the VM are interchangeable. Keeping
/// elements "small so nothing overflows" would MASK the projection instead of proving the lock.
fn vm_fold_word(func_src: &str, name: &str, elems: &[i64]) -> u64 {
    let list = elems.iter().map(|e| arg(*e)).collect::<Vec<_>>().join(", ");
    let build = if elems.is_empty() {
        "a new Seq of Int".to_string()
    } else {
        format!("[{list}]")
    };
    let src = format!("{func_src}## Main\n    Let xs be {build}.\n    Show {name}(xs).\n");
    let out = vm_outcome(&src);
    assert!(
        out.error.is_none(),
        "VM error on {name}({elems:?}): {:?}",
        out.error
    );
    let s = out.output.split_whitespace().next().expect("a Show");
    let (neg, digits) = s.strip_prefix('-').map_or((false, s), |d| (true, d));
    // wrapping Horner over the (possibly hundreds-of-digits) decimal reduces the exact result mod 2^64.
    let mut acc: u64 = 0;
    for ch in digits.bytes() {
        acc = acc.wrapping_mul(10).wrapping_add(u64::from(ch - b'0'));
    }
    if neg {
        acc = acc.wrapping_neg();
    }
    acc
}

fn mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn random_seq(st: &mut u64, max_len: usize, span: i64) -> Vec<i64> {
    let n = (mix(st) as usize) % (max_len + 1); // 0..=max_len (includes the empty sequence)
    (0..n)
        .map(|_| (mix(st) % (2 * span as u64 + 1)) as i64 - span)
        .collect()
}

#[test]
fn seqsum_lowers_to_a_memory_loop() {
    // The `for x in xs` fold is recognized as a SEQUENCE loop (not a numeric range) and carries a MemRead
    // tile that reads the iterated sequence — collections as a spatial resource.
    let lf = loop_fabric_from_source(SEQSUM_FN, "seqsum");
    assert_eq!(
        lf.seq_input,
        Some(0),
        "seqsum iterates its Seq parameter (input 0)"
    );
    let mem_reads = lf
        .body
        .iter()
        .filter(|c| matches!(c, LoopCell::MemRead { .. }))
        .count();
    assert_eq!(
        mem_reads, 1,
        "the body reads the sequence through exactly one MemRead tile, got {mem_reads}"
    );
}

#[test]
fn seqsum_fabric_matches_vm_over_random_sequences() {
    // RED before Phase 2 (loop_fabric_from_source panicked on a sequence loop). GREEN: the memory-loop
    // fabric folds the sequence identically to the VM — including the empty sequence (zero iterations).
    let lf = loop_fabric_from_source(SEQSUM_FN, "seqsum");
    let params = vec![0i64; lf.n_in]; // the seq slot is a placeholder; the array is supplied separately
    let mut st = 0x5EED_5EED_C0DE_u64;
    for _ in 0..400 {
        let seq = random_seq(&mut st, 12, 5000);
        let got = eval_loop_fabric_seq(&lf, &params, &seq);
        let want = vm_fold(SEQSUM_FN, "seqsum", &seq);
        assert_eq!(
            got, want,
            "seqsum fabric ≠ VM on {seq:?} (got {got}, want {want})"
        );
    }
}

#[test]
fn mapped_fold_fabric_matches_vm_over_random_sequences() {
    // A non-trivial body datapath over the streamed element: `acc + x * x`. The Mul tile reads the MemRead
    // result twice; the fold still equals the VM.
    let lf = loop_fabric_from_source(SUMSQ_FN, "sumsq");
    assert_eq!(lf.seq_input, Some(0), "sumsq iterates its Seq parameter");
    let params = vec![0i64; lf.n_in];
    let mut st = 0xF01D_AB1E_1234_u64;
    for _ in 0..400 {
        // FULL-range elements (|x| up to 2^40): `x*x` OVERFLOWS i64 and the running sum wraps, so this exercises
        // the fixed-width projection — NOT a dodged "keep it small" range. The differential lock is
        // `fabric (as u64) == VM mod 2^64` (the VM promotes to BigInt; the fabric is its fixed-width shadow),
        // which must hold for EVERY input: the chip and the VM are interchangeable.
        let seq = random_seq(&mut st, 12, 1 << 40);
        let got = eval_loop_fabric_seq(&lf, &params, &seq) as u64;
        let want = vm_fold_word(SUMSQ_FN, "sumsq", &seq);
        assert_eq!(
            got, want,
            "sumsq fabric ≠ VM mod 2^64 on {seq:?} (got {got}, want {want})"
        );
    }
}

#[test]
fn empty_sequence_runs_zero_iterations() {
    // A pointed edge case (the address stream is empty): the fold returns its initial accumulator, exactly
    // like the VM iterating an empty sequence.
    let lf = loop_fabric_from_source(SEQSUM_FN, "seqsum");
    let params = vec![0i64; lf.n_in];
    assert_eq!(
        eval_loop_fabric_seq(&lf, &params, &[]),
        0,
        "empty seqsum must be 0 (zero iterations)"
    );
    assert_eq!(
        vm_fold(SEQSUM_FN, "seqsum", &[]),
        0,
        "the VM agrees the empty fold is 0"
    );
}
