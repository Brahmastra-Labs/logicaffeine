//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! qwen4_exp_ngram_lock — the n-gram hash of Qwen3.8-Flash-Next's per-layer embedding (QWEN4-3 of
//! `benchmarks/PREPARE_FOR_SAMBA.md`).
//!
//!     `Let I be the ngram ids of T after R by M modulo V from O ending at E`
//!
//! Over token ROWS, as a program's `row t of Emb` reads them (a token's id plus one): for each row of `T`, for each order
//! `n` from 2 to `N = |M|`, `t₀·M₀ ⊕ t₁·M₁ ⊕ … ⊕ t₍ₙ₋₁₎·M₍ₙ₋₁₎` over the ids (`t_k` the token `k` back; the end of
//! document where no token was, and once an end of document — row `E` — lies between), modulo each of the order's heads'
//! `V`, plus its `O`, plus one: the rows of one table the token's n-grams name. `R`, the `N − 1` token rows before the
//! first, is read and written — a decode step's carried state, `0` where no token was, so a fresh sequence's is zero
//! as every carried state's is — and ends holding the last `N − 1` token rows.
//!
//!   • the VM and the tree-walker compute exactly what `model::ngram_hash` (transformers' `Qwen4ExpTextNGramEmbedding`)
//!     does, on every input class: one token, a prompt shorter than the order, an end of document within a prompt, as
//!     the token, twice in a row, the largest token id, a decode step after a prompt and after an end of document, long
//!     prompts with ends of document among them — with Qwen3.8-Flash-Next's own hash and with orders of 2 and 4;
//!   • the history ends as the last `N − 1` tokens, so a prompt hashed in pieces names the rows the whole prompt does;
//!   • the front end lifts it as `TensorOpSpec::NgramIds` with its operands, the history its carried state, and the
//!     graph's evaluator computes the ids and the history the VM does;
//!   • a history, multipliers or heads that do not fit, and tokens that are not integers, are refused by name;
//!   • the SN50 and the SN40L compute the rows and the history the VM does, exactly, on every input class: the hash's
//!     64-bit products, exclusive-ors and remainders as KERNEL-6's lane programs on 32-bit lanes (each proved);
//!   • every shim without the hash refuses it by name, and every spatial shim and the reference resolve it;
//!   • every file of the compiler that names it is named here with what it does with it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test qwen4_exp_ngram_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/spatial.rs"]
mod spatial_support;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::codegen_cuda::forward_shim::ForwardShim;
use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane_shim::LaneShim;
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::model::ngram_hash::NgramHash;
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::shim::EngineShim;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::wide_int::{prove, Op64};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::shim::SpatialShim;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_graph::eval::{eval, Data};
use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
use logicaffeine_compile::tensor_graph::link::resolve;
use logicaffeine_compile::tensor_graph::shim::{ReferenceShim, Shim};
use spatial_support::inputs_from_fills;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tiny_models::Tiny;

/// One layer's hash and the token that ends a document.
struct Hash {
    what: &'static str,
    hash: NgramHash,
    vocab: u32,
    eos: u32,
}

impl Hash {
    fn ngram(&self) -> usize {
        self.hash.multipliers().len()
    }

    fn heads(&self) -> usize {
        self.hash.head_primes().len()
    }
}

/// Qwen3.8-Flash-Next's own (`ngram_size` 3, 8 heads an order, its vocabulary and end of document), and two of other
/// orders: 4, two heads an order; and 2 — no history shift at all — with the end of document token 0.
fn hashes() -> Vec<Hash> {
    vec![
        Hash { what: "Qwen3.8-Flash-Next's", hash: NgramHash::new(248320, 3, 8, 20_000_000, 128, 0, 1234, 248044), vocab: 248320, eos: 248044 },
        Hash { what: "an order of 4", hash: NgramHash::new(1000, 4, 2, 50, 8, 1, 7, 999), vocab: 1000, eos: 999 },
        Hash { what: "an order of 2", hash: NgramHash::new(300, 2, 1, 31, 1, 0, 3, 0), vocab: 300, eos: 0 },
    ]
}

fn xorshift(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

/// The input classes, each `(what, context, tokens)`: `context` the last `N − 1` tokens before (empty for a fresh
/// history of ends of document).
fn classes(h: &Hash) -> Vec<(String, Vec<u32>, Vec<u32>)> {
    let (e, keep) = (h.eos, h.ngram() - 1);
    let other = |t: u32| if t == e { t + 1 } else { t };
    // The largest token id that does not end a document.
    let top = if h.vocab - 1 == e { h.vocab - 2 } else { h.vocab - 1 };
    let mut out = vec![
        ("one token".to_string(), vec![], vec![other(17)]),
        ("a prompt shorter than the order".to_string(), vec![], (0..keep as u32).map(|k| other(5 + k)).collect()),
        ("an end of document within a prompt".to_string(), vec![], vec![other(100 % h.vocab), e, other(7), other(8), other(9)]),
        ("an end of document as the token".to_string(), vec![], vec![other(3), other(4), e]),
        ("two ends of document in a row".to_string(), vec![], vec![other(11), e, e, other(12), other(13)]),
        ("the largest token id".to_string(), vec![], vec![other(top); 5]),
        ("the token whose row is the end of document's id".to_string(), vec![], vec![other(e.saturating_sub(1)), other((e + 1) % h.vocab), other(e.saturating_sub(1))]),
        ("a decode step after a prompt".to_string(), (0..keep as u32).map(|k| other(20 + k)).collect(), vec![other(33)]),
        ("a decode step after an end of document".to_string(), [vec![other(55); keep - 1], vec![e]].concat(), vec![other(66)]),
    ];
    let mut seed = 0x2545_F491_4F6C_DD1Du64 ^ h.vocab as u64;
    for p in 0..4 {
        let tokens: Vec<u32> = (0..48).map(|_| if xorshift(&mut seed) % 9 == 0 { e } else { (xorshift(&mut seed) % h.vocab as u64) as u32 }).collect();
        out.push((format!("a prompt of 48 with ends of document among them ({p})"), vec![], tokens));
    }
    out
}

/// `name` declared `1 by n` of Int and filled with `values`.
fn ints(name: &str, values: &[u64]) -> String {
    let mut s = format!("Let {name} be a 1 by {} tensor of Int.\n", values.len());
    for (k, v) in values.iter().enumerate() {
        s.push_str(&format!("Set item {} of {name} to {v}.\n", k + 1));
    }
    s
}

/// A token's row.
fn row(t: u32) -> u64 {
    u64::from(t) + 1
}

/// The history rows a call starts from: `context`'s rows, or none (`N − 1` zeros) for a fresh sequence.
fn history(h: &Hash, context: &[u32]) -> Vec<u64> {
    if context.is_empty() {
        vec![0; h.ngram() - 1]
    } else {
        context.iter().map(|&t| row(t)).collect()
    }
}

fn program_with(h: &Hash, context: &[u32], tokens: &[u32], tail: &str) -> String {
    let t: Vec<u64> = tokens.iter().map(|&t| row(t)).collect();
    format!(
        "## Main\n{}{}{}{}{}Let Ids be the ngram ids of Toks after Hist by Mult modulo Mods from Offs ending at {}.\n{tail}",
        ints("Toks", &t),
        ints("Hist", &history(h, context)),
        ints("Mult", h.hash.multipliers()),
        ints("Mods", h.hash.head_primes()),
        ints("Offs", h.hash.head_offsets()),
        row(h.eos),
    )
}

fn program(h: &Hash, context: &[u32], tokens: &[u32]) -> String {
    program_with(h, context, tokens, "")
}

/// The table rows and the history rows after, as the VM shows them — the tree-walker agreeing.
fn on_the_vm(h: &Hash, context: &[u32], tokens: &[u32]) -> (Vec<u64>, Vec<u64>) {
    let mut shows = String::new();
    for i in 1..=tokens.len() * h.heads() {
        shows.push_str(&format!("Show item {i} of Ids.\n"));
    }
    for i in 1..h.ngram() {
        shows.push_str(&format!("Show item {i} of Hist.\n"));
    }
    let src = program_with(h, context, tokens, &shows);
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "the VM runs the program: {:?}", vm.error);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "the tree-walker runs the program: {:?}", tw.error);
    assert_eq!(tw.output.trim_end(), vm.output.trim_end(), "the tree-walker agrees with the VM");
    let all: Vec<u64> = vm.output.split_whitespace().map(|t| t.parse().expect("an integer")).collect();
    let (ids, hist) = all.split_at(tokens.len() * h.heads());
    (ids.to_vec(), hist.to_vec())
}

/// What the hash names: its ids' table rows, and the last `N − 1` rows of the history and the tokens.
fn oracle(h: &Hash, context: &[u32], tokens: &[u32]) -> (Vec<u64>, Vec<u64>) {
    let rows = h.hash.ids(context, tokens).concat().into_iter().map(|id| id + 1).collect();
    let mut seen = history(h, context);
    seen.extend(tokens.iter().map(|&t| row(t)));
    (rows, seen[seen.len() - (h.ngram() - 1)..].to_vec())
}

#[test]
fn the_hash_equals_the_reference_on_every_input_class() {
    for h in hashes() {
        for (what, context, tokens) in classes(&h) {
            let (ids, hist) = on_the_vm(&h, &context, &tokens);
            let (want_ids, want_hist) = oracle(&h, &context, &tokens);
            assert_eq!(ids, want_ids, "{} hash, {what}: the ids", h.what);
            assert_eq!(hist, want_hist, "{} hash, {what}: the history after", h.what);
            let rows = h.hash.rows();
            assert!(ids.iter().all(|&r| (1..=rows).contains(&r)), "{} hash, {what}: every id a row of the table's {rows}", h.what);
        }
    }
}

#[test]
fn a_prompt_hashed_in_pieces_names_the_rows_the_whole_prompt_does() {
    for h in hashes() {
        let (_, _, prompt) = classes(&h).pop().expect("a long prompt");
        let (whole, _) = on_the_vm(&h, &[], &prompt);
        let (mut pieces, mut context) = (Vec::new(), Vec::new());
        for piece in prompt.chunks(7) {
            let (ids, hist) = on_the_vm(&h, &context, piece);
            pieces.extend(ids);
            context = hist.iter().map(|&r| (r - 1) as u32).collect();
        }
        assert_eq!(pieces, whole, "{} hash: the prompt in pieces of 7 after the history each leaves", h.what);
    }
}

#[test]
fn the_front_end_lifts_it_and_the_evaluator_computes_the_ids_and_the_history_the_vm_does() {
    for h in hashes() {
        for (what, context, tokens) in classes(&h) {
            let src = program(&h, &context, &tokens);
            let rp = recognize_stages(&src).expect("parses");
            assert!(rp.unrecognized.is_empty(), "{what} lifts whole: {:?}", rp.unrecognized);
            let st = rp.stages.iter().find(|s| s.out == "Ids").expect("a stage writes Ids");
            let TensorOpSpec::NgramIds(spec) = &st.op else { panic!("{what} lifts as {:?}", st.op) };
            assert_eq!(
                (spec.tokens, spec.ngram, spec.per_order, spec.heads(), spec.eos),
                (tokens.len(), h.ngram(), h.heads() / (h.ngram() - 1), h.heads(), row(h.eos) as i64),
                "{} hash, {what}: the spec",
                h.what
            );
            assert_eq!(st.ins, ["Toks", "Hist", "Mult", "Mods", "Offs"], "{what}: its operands, in order");
            assert_eq!(st.op.state_inputs(), [1], "{what}: the history its carried state");
            let g = TensorGraph::lift(&rp, None, None, &|_| Storage::Values).unwrap_or_else(|why| panic!("{what} lifts: {why:?}"));
            let env = eval(&g, &BTreeMap::new()).unwrap_or_else(|why| panic!("the evaluator computes {what}: {why}"));
            let (want_ids, want_hist) = on_the_vm(&h, &context, &tokens);
            let as_u64 = |name: &str| match env.get(name) {
                Some(Data::Int(v)) => v.iter().map(|&x| x as u64).collect::<Vec<u64>>(),
                other => panic!("{what}: the evaluator's `{name}` is {other:?}"),
            };
            assert_eq!(as_u64("Ids"), want_ids, "{} hash, {what}: the evaluator's ids", h.what);
            assert_eq!(as_u64("Hist")[..h.ngram() - 1], want_hist[..], "{} hash, {what}: the evaluator's history after", h.what);
        }
    }
}

#[test]
fn a_history_multipliers_or_heads_that_do_not_fit_are_refused_by_name() {
    let tail = "Let Ids be the ngram ids of Toks after Hist by Mult modulo Mods from Offs ending at 9.\n";
    let reals = "Let Toks be a 1 by 2 tensor of Real.\n";
    for (decls, what, names) in [
        (format!("{}{}{}{}{}", ints("Toks", &[1, 2]), ints("Hist", &[9]), ints("Mult", &[3, 5, 7]), ints("Mods", &[11, 13]), ints("Offs", &[0, 11])), "a history of 1 for an order of 3", "history"),
        (format!("{}{}{}{}{}", ints("Toks", &[1, 2]), ints("Hist", &[9]), ints("Mult", &[3]), ints("Mods", &[11]), ints("Offs", &[0])), "one multiplier", "multiplier"),
        (format!("{}{}{}{}{}", ints("Toks", &[1, 2]), ints("Hist", &[9, 9]), ints("Mult", &[3, 5, 7]), ints("Mods", &[11, 13, 17]), ints("Offs", &[0, 11, 24])), "three heads for two orders", "heads"),
        (format!("{}{}{}{}{}", ints("Toks", &[1, 2]), ints("Hist", &[9, 9]), ints("Mult", &[3, 5, 7]), ints("Mods", &[11, 13]), ints("Offs", &[0])), "two moduli and one offset", "offsets"),
        (format!("{reals}{}{}{}{}", ints("Hist", &[9, 9]), ints("Mult", &[3, 5, 7]), ints("Mods", &[11, 13]), ints("Offs", &[0, 11])), "tokens of Real", "Int"),
    ] {
        let src = format!("## Main\n{decls}{tail}");
        let refusal = format!("{:?}", recognize_stages(&src).err().unwrap_or_else(|| panic!("{what} is refused")));
        assert!(refusal.contains(names), "{what}: the refusal names its {names}: {refusal}");
    }
    // Rows count from one: an end of document at row 0 is no row.
    let decls = format!("{}{}{}{}{}", ints("Toks", &[1, 2]), ints("Hist", &[9, 9]), ints("Mult", &[3, 5, 7]), ints("Mods", &[11, 13]), ints("Offs", &[0, 11]));
    let src = format!("## Main\n{decls}{}", tail.replace("ending at 9", "ending at 0"));
    let refusal = format!("{:?}", recognize_stages(&src).err().expect("an end of document at row 0 is refused"));
    assert!(refusal.contains("from 1"), "the refusal says rows count from 1: {refusal}");
}

fn shims() -> Vec<Box<dyn Shim>> {
    let mut out: Vec<Box<dyn Shim>> = Vec::new();
    for activations in [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        for fuse_quant in [true, false] {
            for batch in [false, true] {
                let knobs = CudaKnobs { activations, fuse_quant, ..CudaKnobs::default() };
                out.push(Box::new(LaneShim { knobs, batch, held: Vec::new() }));
            }
        }
        out.push(Box::new(ForwardShim { kind: activations }));
    }
    for tiny in [Tiny::Qwen35, Tiny::Qwen3] {
        out.push(Box::new(EngineShim { spec: Box::leak(Box::new(tiny.spec(24))), numerics: EngineNumerics::precise() }));
    }
    out
}

#[test]
fn every_shim_without_it_refuses_it_by_name_and_every_spatial_shim_resolves_it() {
    let h = &hashes()[0];
    let src = program(h, &[], &[5, 17, 2, 30]);
    let g = TensorGraph::lift(&recognize_stages(&src).expect("parses"), None, None, &|_| Storage::Values).expect("lifts");
    for shim in shims() {
        let unresolved = resolve(&g, shim.as_ref()).expect_err(&format!("{} refuses the n-gram hash", shim.name()));
        let refused = unresolved.iter().find(|u| u.kind == "NgramIds").unwrap_or_else(|| panic!("{} refuses the NgramIds: {unresolved:?}", shim.name()));
        assert!(["n-gram", "NgramIds"].iter().any(|w| refused.why.contains(w)), "{} names the n-gram hash it refuses: {}", shim.name(), refused.why);
    }
    for target in [SpatialTarget::wormhole(), SpatialTarget::sky_fabric(64, 64), SpatialTarget::sn50_rdu(), SpatialTarget::rtx3060()] {
        let shim = SpatialShim { target: Box::leak(Box::new(target)) };
        resolve(&g, &shim).unwrap_or_else(|why| panic!("{} resolves the n-gram hash: {why:?}", shim.name()));
    }
    resolve(&g, &ReferenceShim).unwrap_or_else(|why| panic!("the reference resolves the n-gram hash: {why:?}"));
}

/// The values the chip leaves in the tensor `name`.
fn on_the_chip(prog: &SpatialProgram, name: &str) -> Vec<u64> {
    let out = eval_choreography_every_tensor(prog, &inputs_from_fills(prog)).expect("the program evaluates");
    let t = prog.tensors.iter().rev().find(|t| t.name == name).unwrap_or_else(|| panic!("the program holds `{name}`"));
    match out.tensors.get(&t.id).unwrap_or_else(|| panic!("the evaluation hands back `{name}`")) {
        Value::Int(v) => v.iter().map(|x| *x as u64).collect(),
        Value::Float(v) => v.iter().map(|x| *x as u64).collect(),
    }
}

#[test]
fn the_rdus_compute_the_rows_and_the_history_exactly_on_every_input_class() {
    for target in [SpatialTarget::sn50_rdu(), SpatialTarget::sn40l_rdu()] {
        for h in hashes() {
            for (what, context, tokens) in classes(&h) {
                let prog = compile_program_to_spatial(&program(&h, &context, &tokens), &target)
                    .unwrap_or_else(|why| panic!("{} lowers the {} hash, {what}: {why:?}", target.name, h.what))
                    .into_whole()
                    .unwrap_or_else(|why| panic!("{why}"));
                let (rows, hist) = on_the_vm(&h, &context, &tokens);
                let where_ = format!("{} hash, {what}, on {}", h.what, target.name);
                assert_eq!(on_the_chip(&prog, "Ids")[..rows.len()], rows[..], "{where_}: the rows");
                assert_eq!(on_the_chip(&prog, "Hist")[..hist.len()], hist[..], "{where_}: the history after");
            }
        }
    }
}

/// KERNEL-6's proof of each lane program the heads of `hashes` read, equal to its 64-bit definition.
fn heads_are_proved(hashes: &[Hash]) {
    for op in [Op64::Mul, Op64::Xor] {
        prove(op).unwrap_or_else(|why| panic!("{op:?} is proved: {why}"));
    }
    for h in hashes {
        for &p in h.hash.head_primes() {
            prove(Op64::Rem(p as u32)).unwrap_or_else(|why| panic!("the {} hash's head modulo {p} is proved: {why}", h.what));
        }
    }
}

#[test]
fn every_head_reads_a_proved_lane_program() {
    // The product, the exclusive-or, and every head's remainder of the hashes of orders 4 and 2 (Qwen3.8-Flash-Next's
    // sixteen, each its own proof, are the ignored test after this one).
    heads_are_proved(&hashes()[1..]);
}

#[test]
#[ignore = "heavy: sixteen remainder proofs, one for each of Qwen3.8-Flash-Next's heads"]
fn every_head_of_qwen38s_hash_reads_a_proved_lane_program() {
    heads_are_proved(&hashes()[..1]);
}

const CONSUMERS: &[(&str, &str)] = &[
    ("tensor_recognize/qwen.rs", "lifts"),
    ("tensor_graph/front.rs", "lifts"),
    ("tensor_graph/graph.rs", "routes"),
    ("tensor_graph/eval.rs", "computes"),
    ("tensor_graph/vjp.rs", "refuses"),
    ("spatial/link.rs", "computes"),
    ("spatial/passes/from_qwen_ops.rs", "computes"),
    ("codegen_cpu/link.rs", "computes"),
    ("codegen_cuda/forward_shim.rs", "refuses"),
    ("codegen_cuda/lane_link.rs", "refuses"),
    ("codegen_cuda/lane_shim.rs", "refuses"),
    ("spatial/decode_engine/shim.rs", "refuses"),
];

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn every_consumer_of_the_ngram_hash_is_named() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let found: BTreeSet<String> = files
        .iter()
        .filter(|p| std::fs::read_to_string(p).unwrap().contains("NgramIds"))
        .map(|p| p.strip_prefix(&src).unwrap().to_string_lossy().to_string())
        .collect();
    let named: BTreeSet<String> = CONSUMERS.iter().map(|(f, _)| f.to_string()).collect();
    assert_eq!(found, named, "the files that name the n-gram hash are the ones this census names");
}
