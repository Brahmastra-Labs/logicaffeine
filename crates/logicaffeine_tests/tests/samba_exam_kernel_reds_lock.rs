//! samba_exam_kernel_reds_lock — the rows of the Samba book's exam E11 (section 10.11 of
//! `benchmarks/PREPARE_FOR_SAMBA.md`, the kernel micro-exam) still open, each a handed RED frozen in
//! `fixtures/frozen_reds.tsv`.
//!
//! A row is one kernel shape at the edge of a unit's capacity, written in LOGOS, lowered for the SN50 by
//! `compile_program_to_spatial` and run through the spatial evaluator, against the VM's f64 run of the same source:
//!
//!   • E11.4 — 64 tokens each routed to 8 of 256 experts, a mixture a token as the author writes a batch: the values
//!     the VM's, and the weights each touched expert holds read once for the batch, not once a token;
//!   • E11.7 — chains of 1,250 and 10,000 pointwise statements, the compile time linear in the statements.
//!
//! A ticket that lands a row moves its test, unchanged, into `samba_exam_kernel_lock`.
//!
//! Run on purpose: `scripts/run-handed-reds-tests.sh`, or
//! `cargo nextest run -p logicaffeine-tests --features handed-reds --cargo-profile test-opt --test samba_exam_kernel_reds_lock`.

#![cfg(feature = "codegen")]
#![cfg(feature = "handed-reds")]

#[path = "compile_common/tensor_probes.rs"]
#[allow(dead_code)]
mod tensor_probes;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::spatial::work::work_of;
use std::collections::BTreeSet;
use std::time::Instant;
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

/// A value in `[-1, 1]` in steps of 1/64 for element `i` of tensor `name`.
fn seeded(name: &str, i: usize) -> f64 {
    let salt = name.bytes().fold(7u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64));
    let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
    ((h >> 40) % 129) as f64 / 64.0 - 1.0
}

/// `name` declared `rows by cols` of Real and filled with `value(i)`.
fn filled(name: &str, rows: usize, cols: usize, value: &dyn Fn(usize) -> f64) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for i in 0..rows * cols {
        s.push_str(&format!("Set item {} of {name} to {:?}.\n", i + 1, value(i)));
    }
    s
}

/// Every entry of `show` (`n` of them) shown as `round(v·1e6)`.
fn shows(show: &str, n: usize) -> String {
    (1..=n).map(|i| format!("Show round(item {i} of {show} * 1000000).\n")).collect()
}

/// `src` lowered whole for `target`, or why not.
fn lowered(src: &str, target: &SpatialTarget) -> Result<SpatialProgram, String> {
    let pl = compile_program_to_spatial(src, target).map_err(|e| format!("the walk: {e:?}"))?;
    pl.whole().cloned().ok_or_else(|| pl.refusal())
}

/// What the SN50 shows for `src` against what the VM shows, within the bf16 datapath's 2% of the largest value.
fn on_the_sn50_as_the_vm(src: &str, what: &str) -> SpatialProgram {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "{what}: the VM: {:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("the VM shows integers");
    let sp = lowered(src, &SpatialTarget::sn50_rdu()).unwrap_or_else(|why| panic!("{what}: lowered whole for the SN50: {why}"));
    let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills")).unwrap_or_else(|e| panic!("{what}: {e:?}"));
    let got = shown_from_outputs(src, &sp, &out).expect("shown");
    assert_eq!(got.len(), want.len(), "{what}: one Show a value");
    let scale = want.iter().fold(1i64, |m, v| m.max(v.abs())) as f64;
    for (i, (g, v)) in got.iter().zip(&want).enumerate() {
        assert!(((g - v).abs() as f64) <= 0.02 * scale, "{what}: Show #{} on the SN50 is {g}, the VM's {v}", i + 1);
    }
    sp
}

/// The mixture E11.4 routes: `d` 32, experts of 32, 256 of them, 8 chosen a token; `tokens` rows of `X`, each through
/// its own mixture statement as the author writes a batch, every result shown.
fn dispatch_program(tokens: usize) -> String {
    let (d, dff, experts) = (32, 32, 256);
    let mut s = format!(
        "## Main\n{}{}{}{}{}",
        filled("X", 64, d, &|i| seeded("X", i)),
        filled("Wr", d, experts, &|i| seeded("Wr", i)),
        filled("Wg", experts * dff, d, &|i| seeded("Wg", i) / 2.0),
        filled("Wu", experts * dff, d, &|i| seeded("Wu", i) / 2.0),
        filled("Wd", experts * d, dff, &|i| seeded("Wd", i) / 2.0),
    );
    for b in 0..tokens {
        s.push_str(&format!(
            "Let x{b} be row {} of X.\nLet m{b} be moe of x{b} with router Wr gate Wg up Wu down Wd choosing 8 of {experts}.\n",
            b + 1
        ));
    }
    for b in 0..tokens {
        s.push_str(&shows(&format!("m{b}"), d));
    }
    s
}

#[test]
fn dispatch_of_256_experts_top_8_at_batch_64() {
    // The experts the batch touches: each token's 8 largest router logits (the softmax keeps their order).
    let (d, experts) = (32, 256);
    let touched: BTreeSet<usize> = (0..64)
        .flat_map(|b| {
            let mut logits: Vec<(f64, usize)> = (0..experts)
                .map(|e| ((0..d).map(|k| seeded("X", b * d + k) * seeded("Wr", k * experts + e)).sum::<f64>(), e))
                .collect();
            logits.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            logits.into_iter().take(8).map(|(_, e)| e).collect::<Vec<_>>()
        })
        .collect();
    let sn50 = SpatialTarget::sn50_rdu();
    let batch = on_the_sn50_as_the_vm(&dispatch_program(64), "64 tokens routed to 8 of 256 experts");
    let one = lowered(&dispatch_program(1), &sn50).unwrap_or_else(|why| panic!("one token's mixture: {why}"));
    let (batch_bytes, one_bytes) = (work_of(&batch, &sn50).dram_bytes, work_of(&one, &sn50).dram_bytes);
    // Dispatched, the batch reads each touched expert's weights once: at most its distinct experts' worth of one
    // token's reads (one token reads 8 experts), with a quarter again for the tokens' own rows. Each token through its
    // own experts reads 64 tokens' worth.
    let bound = 1.25 * touched.len() as f64 / 8.0 * one_bytes as f64;
    assert!(
        batch_bytes as f64 <= bound,
        "64 tokens over {} touched experts read {batch_bytes} DRAM bytes, one token {one_bytes}: past {bound:.0}, the touched experts read once",
        touched.len()
    );
}

/// A chain of `n` pointwise statements over one row of 64.
fn chain(n: usize) -> String {
    let mut src = format!("## Main\n{}Let Y0 be X plus X.\n", filled("X", 1, 64, &|i| seeded("X", i)));
    for k in 1..n {
        src.push_str(&format!("Let Y{k} be Y{} plus X.\n", k - 1));
    }
    src
}

/// `src` lowered for the SN50, its time.
fn lowering_seconds(src: &str, what: &str) -> f64 {
    let t = Instant::now();
    lowered(src, &SpatialTarget::sn50_rdu()).unwrap_or_else(|why| panic!("{what}: {why}"));
    t.elapsed().as_secs_f64()
}

#[test]
fn a_graph_of_10000_nodes_compiles_linearly() {
    // 10,000 statements take at most 12 times what 1,250 take — eight times the work, with half again for the
    // machine's noise — each size timed as the fastest of three runs, so a burst of the box's other work slows no size's
    // measure (Tristen, 2026-11-12). The larger lowering is waited on no longer than three of its bounds, so a
    // superlinear one fails there instead of running on.
    const RUNS: usize = 3;
    let small = (0..RUNS).map(|_| lowering_seconds(&chain(1_250), "1,250 statements")).fold(f64::INFINITY, f64::min);
    let bound = 12.0 * small;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for _ in 0..RUNS {
            if tx.send(lowering_seconds(&chain(10_000), "10,000 statements")).is_err() {
                return;
            }
        }
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(RUNS as f64 * bound);
    let mut fastest = f64::INFINITY;
    for _ in 0..RUNS {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok(large) => fastest = fastest.min(large),
            Err(_) => panic!("10,000 statements are not lowered {RUNS} times within {:.2}s, {RUNS} times twelve times the {small:.2}s 1,250 take", RUNS as f64 * bound),
        }
        if fastest <= bound {
            return;
        }
    }
    panic!("10,000 statements lower in {fastest:.2}s at the fastest of {RUNS} runs, past twelve times the {small:.2}s 1,250 take");
}
