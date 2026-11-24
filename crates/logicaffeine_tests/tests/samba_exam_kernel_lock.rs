//! samba_exam_kernel_lock — the rows of the Samba book's exam E11 (section 10.11 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! the kernel micro-exam) that hold today, each frozen in `fixtures/frozen_reds.tsv` so no later change weakens it.
//!
//!   • E11.12 — Qwen3.8-Flash-Next's n-gram hash is 64-bit arithmetic on chips whose integers are 32 bits: every
//!     operation it reads, lowered to 32-bit lanes, equals its 64-bit definition on every input class as the decision
//!     fabric's 64-bit ALU computes the lanes' cells, and the table rows it names for tokens at every class's edge,
//!     computed on the SN50 and the SN40L, are `model::ngram_hash`'s.
//!   • E11.8 — one tensor read by 64 consumers, each its own scale of it, the 64 results summed, on the SN50 as the VM
//!     computes it;
//!   • E11.10 — four sequences after prompts of 1, 5, 3 and 7 tokens decode their next token at once on the SN50 batch
//!     bundle, each attending its own length: each sequence's logits are the ones it decodes alone, within the bf16
//!     datapath's bound;
//!   • E11.5 — a LayerNorm over one row of 16,384 whose mean is large beside its spread, every input exact in bf16,
//!     accumulated in fp32;
//!   • E11.6 — a chain of 200 pointwise ops over one row, its results declared as a deployment declares them: one
//!     section, its intermediates never leaving the site;
//!   • E11.9 — a gather of rows with repeated indices, and a scatter of rows with repeated indices written as the VM's
//!     own loop, the last write to a row kept, and the scatter that adds.
//!
//! The rows still open are `samba_exam_kernel_reds_lock`'s.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_kernel_lock`

#![cfg(feature = "codegen")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;
#[path = "common/samba_exam.rs"]
#[allow(dead_code)]
mod samba_exam;
#[path = "common/spatial.rs"]
mod spatial_support;
#[path = "compile_common/tensor_probes.rs"]
#[allow(dead_code)]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_rtl::{eval_decision_fabric_heap, DecisionFabric, Src};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::model::ngram_hash::NgramHash;
use logicaffeine_compile::spatial::eval::{eval_choreography, eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, compile_program_to_spatial_returning};
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::target::SpatialTarget;
use std::collections::BTreeSet;
use logicaffeine_compile::spatial::wide_int::{lower, Op64};
use samba_exam::{decode_batch_on_bundle, fp32_twin};
use samba_handoff::{decode_on_bundle, prefill_on, Model, BF16_LOGIT_REL, CTX, NEXT};
use spatial_support::inputs_from_fills;
use tensor_probes::{oracle_ints, shown_from_outputs};
use tiny_models::Tiny;

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

/// What the SN50 shows for `src` against what the VM shows, within the bf16 datapath's 2% of the largest value.
fn on_the_sn50_as_the_vm(src: &str, what: &str) {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "{what}: the VM: {:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("the VM shows integers");
    let pl = compile_program_to_spatial(src, &SpatialTarget::sn50_rdu()).unwrap_or_else(|e| panic!("{what}: the walk: {e:?}"));
    let sp = pl.whole().cloned().unwrap_or_else(|| panic!("{what}: lowered whole for the SN50: {}", pl.refusal()));
    let out = eval_choreography(&sp, &tensor_probes::inputs_from_fills(&sp).expect("fills")).unwrap_or_else(|e| panic!("{what}: {e:?}"));
    let got = shown_from_outputs(src, &sp, &out).expect("shown");
    assert_eq!(got.len(), want.len(), "{what}: one Show a value");
    let scale = want.iter().fold(1i64, |m, v| m.max(v.abs())) as f64;
    for (i, (g, v)) in got.iter().zip(&want).enumerate() {
        assert!(((g - v).abs() as f64) <= 0.02 * scale, "{what}: Show #{} on the SN50 is {g}, the VM's {v}", i + 1);
    }
}

fn xorshift(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

/// `op`'s lane program as decision-fabric cells, run by the fabric's 64-bit ALU on `a` and `b`, each lane held to 32
/// bits: the 64-bit result its lanes make.
fn on_the_fabric(op: Op64, a: u64, b: u64) -> u64 {
    let program = lower(op);
    let inputs: Vec<Src> = (0..program.inputs()).map(Src::In).collect();
    let mut cells = Vec::new();
    let [lo, hi] = program.cells(&inputs, &mut cells);
    let values: Vec<i64> = [a, a >> 32, b, b >> 32][..program.inputs()].iter().map(|v| i64::from(*v as u32)).collect();
    let lane = |result: Src| {
        let fabric = DecisionFabric { n_in: values.len(), cells: cells.clone(), result, escaped_first_return: false };
        let v = eval_decision_fabric_heap(&fabric, &values, &mut Vec::new());
        assert!((0..=i64::from(u32::MAX)).contains(&v), "{op:?}({a:#x}, {b:#x}): a lane of {v:#x}");
        v as u64
    };
    lane(lo) | lane(hi) << 32
}

/// `name` declared `1 by n` of Int and filled with `values`.
fn ints(name: &str, values: &[u64]) -> String {
    let mut s = format!("Let {name} be a 1 by {} tensor of Int.\n", values.len());
    for (k, v) in values.iter().enumerate() {
        s.push_str(&format!("Set item {} of {name} to {v}.\n", k + 1));
    }
    s
}

#[test]
fn hash_arithmetic_is_exact_on_32_bit_lanes() {
    let (vocab, eos) = (248_320u32, 248_044u32);
    let hash = NgramHash::new(u64::from(vocab), 3, 8, 20_000_000, 128, 0, 1234, eos);
    // The operations the hash reads: the product, the exclusive-or, and the remainder by each head's prime.
    let mut ops = vec![Op64::Mul, Op64::Xor];
    ops.extend(hash.head_primes().iter().map(|&p| Op64::Rem(p as u32)));
    let top = u64::from(vocab - 1);
    let largest_multiplier = *hash.multipliers().iter().max().expect("multipliers");
    let mut edges = vec![0, 1, 2, u64::MAX, u64::MAX - 1, 0x5555_5555_5555_5555, 0xAAAA_AAAA_AAAA_AAAA, top, top + 1, largest_multiplier, largest_multiplier * top];
    for k in [15, 16, 31, 32, 33, 45, 63] {
        let p = 1u64 << k;
        edges.extend([p - 1, p, p + 1]);
    }
    let mut seed = 0x51D0_C0DE_0B5E_55EDu64;
    for op in ops {
        let mut cases: Vec<(u64, u64)> = edges.iter().flat_map(|&a| edges.iter().map(move |&b| (a, b))).collect();
        cases.extend((0..512).map(|_| (xorshift(&mut seed), xorshift(&mut seed))));
        for (a, b) in cases {
            assert_eq!(on_the_fabric(op, a, b), op.apply(a, b), "{op:?}({a:#x}, {b:#x}) on 32-bit lanes");
        }
    }
    // The table rows the hash names on the RDUs: tokens at the ids' edges — the first, the largest, the end of document
    // and its neighbours — after a fresh history and after one that ends a document.
    let row = |t: u32| u64::from(t) + 1;
    let cases: [(&str, Vec<u32>, Vec<u32>); 3] = [
        ("the ids' edges after a fresh history", vec![], vec![0, 1, top as u32, eos - 1, eos + 1, top as u32, 0]),
        ("an end of document among them", vec![], vec![5, eos, top as u32, eos, eos, 7]),
        ("after a history that ends a document", vec![top as u32, eos], vec![1, top as u32]),
    ];
    for target in [SpatialTarget::sn50_rdu(), SpatialTarget::sn40l_rdu()] {
        for (what, context, tokens) in &cases {
            let history: Vec<u64> = if context.is_empty() { vec![0; 2] } else { context.iter().map(|&t| row(t)).collect() };
            let src = format!(
                "## Main\n{}{}{}{}{}Let Ids be the ngram ids of Toks after Hist by Mult modulo Mods from Offs ending at {}.\n",
                ints("Toks", &tokens.iter().map(|&t| row(t)).collect::<Vec<_>>()),
                ints("Hist", &history),
                ints("Mult", hash.multipliers()),
                ints("Mods", hash.head_primes()),
                ints("Offs", hash.head_offsets()),
                row(eos),
            );
            let prog = compile_program_to_spatial(&src, &target)
                .unwrap_or_else(|why| panic!("{} lowers the hash, {what}: {why:?}", target.name))
                .into_whole()
                .unwrap_or_else(|why| panic!("{} lowers the hash whole, {what}: {why}", target.name));
            let values = eval_choreography_every_tensor(&prog, &inputs_from_fills(&prog)).expect("the program evaluates");
            let id = prog.tensors.iter().find(|t| t.name == "Ids").expect("the program holds Ids").id;
            let got: Vec<u64> = match values.tensors.get(&id).expect("the evaluation hands back Ids") {
                Value::Int(v) => v.iter().map(|x| *x as u64).collect(),
                Value::Float(v) => v.iter().map(|x| *x as u64).collect(),
            };
            let want: Vec<u64> = hash.ids(context, tokens).concat().into_iter().map(|id| id + 1).collect();
            assert_eq!(got[..want.len()], want[..], "{} hash rows, {what}", target.name);
        }
    }
}

#[test]
fn ragged_lengths_in_one_batch() {
    // Four sequences after prompts of 1, 5, 3 and 7 tokens decode their next token at once on the batch bundle: each
    // attends its own length, so each sequence's logits are the ones it decodes alone, within the bf16 datapath's bound.
    for t in [Tiny::Qwen3, Tiny::Gemma3] {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let prompts: Vec<Vec<u32>> = [1usize, 5, 3, 7].iter().map(|&n| (0..n as u32).map(|i| (i * 11 + 2) % m.spec.vocab as u32).collect()).collect();
        let handed: Vec<_> = prompts.iter().map(|p| prefill_on(&m, &fp32_twin(), p)).collect();
        let batch = decode_batch_on_bundle(&m, &handed, &[NEXT; 4]);
        for (s, h) in handed.iter().enumerate() {
            let alone = decode_on_bundle(&m, Some(h));
            let scale = alone.iter().fold(0.0f64, |a, v| a.max(v.abs()));
            let worst = batch[s].iter().zip(&alone).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            assert!(
                worst <= BF16_LOGIT_REL * scale,
                "{t:?}: the sequence after {} tokens lies {worst:.5} from its decode alone in a ragged batch, past {:.5}",
                prompts[s].len(),
                BF16_LOGIT_REL * scale
            );
        }
    }
}

#[test]
fn one_tensor_feeds_64_consumers() {
    // X read by 64 statements, each its own scale of X, and the 64 results summed in a chain: 32.5 times X.
    let mut src = filled("X", 1, 64, &|i| seeded("X", i));
    for k in 1..=64 {
        src.push_str(&format!("Let Y{k} be X scaled by {:?}.\n", k as f64 / 64.0));
    }
    src.push_str("Let Z1 be Y1 plus Y2.\n");
    for k in 2..=63 {
        src.push_str(&format!("Let Z{k} be Z{} plus Y{}.\n", k - 1, k + 1));
    }
    on_the_sn50_as_the_vm(&format!("## Main\n{src}{}", shows("Z63", 64)), "one tensor read by 64 consumers");
}

#[test]
fn layernorm_over_16384_accumulates_in_fp32() {
    // A row of 16,384 about a mean of 64 with a spread of a quarter: every input a multiple of 1/4 in [63, 65], which bf16
    // holds exactly, so what is measured is the sums. Over 16,384 terms of about 4,096 the square's sum reaches 2^26,
    // where bf16 keeps 8 bits and fp32 24: a mean and a variance accumulated in bf16 lose the spread entirely.
    let n = 16_384;
    let x = |i: usize| 64.0 + (((i * 7919) % 9) as f64 - 4.0) / 4.0;
    let src = format!(
        "## Main\n{}{}{}Let Y be layernorm of X with weight W and bias B.\n{}",
        filled("X", 1, n, &x),
        filled("W", 1, n, &|i| 1.0 + seeded("W", i) / 4.0),
        filled("B", 1, n, &|i| seeded("B", i) / 8.0),
        shows("Y", 64),
    );
    on_the_sn50_as_the_vm(&src, "a layernorm over 16,384");
}

#[test]
fn gather_and_scatter_with_repeated_indices() {
    // A gather: rows I of T, I naming rows 3, 3, 1, 8, 3 and 1 of eight.
    let index = [3, 3, 1, 8, 3, 1];
    let table = filled("T", 8, 16, &|i| seeded("T", i));
    let gather = format!("## Main\n{table}{}Let G be rows I of T.\n{}", ints("I", &index), shows("G", 6 * 16));
    on_the_sn50_as_the_vm(&gather, "a gather of rows with repeated indices");
    // A scatter: row k of X written at row I[k] of T in turn, so a row named twice keeps the later; and the scatter that
    // adds each row of X into its row of T, so a row named twice takes both.
    let rows_of_x = filled("X", 6, 16, &|i| seeded("X", i));
    let scatter = |body: &str| {
        format!(
            "## Main\n{table}{rows_of_x}{}Repeat for k from 1 to 6:\n    Let at be item k of I.\n{body}{}",
            ints("I", &index),
            shows("T", 8 * 16)
        )
    };
    on_the_sn50_as_the_vm(
        &scatter("    Let r be row k of X.\n    Set row at of T to r.\n"),
        "a scatter of rows with repeated indices, the last kept",
    );
    on_the_sn50_as_the_vm(
        &scatter("    Let r be row k of X.\n    Let old be row at of T.\n    Let sum be old plus r.\n    Set row at of T to sum.\n"),
        "a scatter of rows with repeated indices, each added",
    );
}

/// [`on_the_sn50_as_the_vm`] for a program whose results are DECLARED, as a deployment declares them: what it shows is
/// all its host reads, so a tensor only the next statement reads is forwarded rather than stored
/// (`compile_program_to_spatial_returning`).
fn on_the_sn50_returning(src: &str, what: &str) -> SpatialProgram {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "{what}: the VM: {:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("the VM shows integers");
    let pl = compile_program_to_spatial_returning(src, &SpatialTarget::sn50_rdu(), &BTreeSet::new())
        .unwrap_or_else(|e| panic!("{what}: the walk: {e:?}"));
    let sp = pl.whole().cloned().unwrap_or_else(|| panic!("{what}: lowered whole for the SN50: {}", pl.refusal()));
    let out = eval_choreography(&sp, &tensor_probes::inputs_from_fills(&sp).expect("fills")).unwrap_or_else(|e| panic!("{what}: {e:?}"));
    let got = shown_from_outputs(src, &sp, &out).expect("shown");
    assert_eq!(got.len(), want.len(), "{what}: one Show a value");
    let scale = want.iter().fold(1i64, |m, v| m.max(v.abs())) as f64;
    for (i, (g, v)) in got.iter().zip(&want).enumerate() {
        assert!(((g - v).abs() as f64) <= 0.02 * scale, "{what}: Show #{} on the SN50 is {g}, the VM's {v}", i + 1);
    }
    sp
}

#[test]
fn a_chain_of_200_pointwise_ops_is_one_section() {
    // Two hundred elementwise ops over one row of 64 — sums, products, a sigmoid, a silu and a scale in turn — read one
    // row and write one: a single section, its intermediates never leaving the site. What it shows is all its host reads
    // (Tristen, 2026-11-12: the lock lowers with its results declared, as a deployment does).
    let mut src = format!("## Main\n{}{}Let Y0 be X plus C.\n", filled("X", 1, 64, &|i| seeded("X", i)), filled("C", 1, 64, &|i| seeded("C", i) / 4.0));
    for k in 1..200 {
        let p = k - 1;
        src.push_str(&match k % 5 {
            0 => format!("Let Y{k} be Y{p} plus C.\n"),
            1 => format!("Let Y{k} be Y{p} hadamard C.\n"),
            2 => format!("Let Y{k} be sigmoid of Y{p}.\n"),
            3 => format!("Let Y{k} be silu of Y{p}.\n"),
            _ => format!("Let Y{k} be Y{p} scaled by 2.0.\n"),
        });
    }
    src.push_str(&shows("Y199", 64));
    let sp = on_the_sn50_returning(&src, "a chain of 200 pointwise ops");
    assert_eq!(sp.sections.len(), 1, "a chain of 200 pointwise ops is one section, not {}", sp.sections.len());
}
