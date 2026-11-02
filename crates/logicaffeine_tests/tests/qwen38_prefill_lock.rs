//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! QWEN3.8 PREFILL — a prompt's tokens as ONE step: the chunk is its tokens decoded in turn.
//!
//! A decode step takes one token of one sequence. Serving a prompt of T tokens that way takes T steps, each a full
//! weight pass for one row. `gen_qwen38_prefill_lg` takes the T tokens of ONE sequence at positions `pos0 … pos0+T-1`
//! as one step: their hidden states STACKED into a `T × d` matrix, so every projection, norm, FFN and residual is one
//! weight pass for the whole chunk, while what is sequential in the model stays sequential — row `i` appends its key
//! and value to the sequence's cache and attends up to its own position, and the Gated-DeltaNet conv rings and
//! recurrent memories take the rows one after another. The chunk carries the decode step's own state (`Kc3`, `St0`,
//! `Csq0` …), so a decode step continues exactly where it stops, and only the last row goes through the LM head.
//!
//! Pinned:
//!   1. THE CHUNK IS ITS TOKENS DECODED IN TURN: on the VM, the chunk's next token, its logits and every state tensor it
//!      leaves equal those of T single decode steps, each run on the state the one before it left (read back from the
//!      VM and handed to the next as its fill) — to 1e-12.
//!   2. THE CHUNK CARRIES THE DECODE STEP'S STATE AND READS THE LM HEAD ONCE: it declares exactly the decode step's
//!      state tensors, by name and shape; its one logits row is the last token's.
//!   3. THE CHUNK LOWERS WHOLE ON SN50, CERTIFIED, AND EMITS THE VM'S TOKEN AND STATE: the composed program's
//!      certificate rechecks; evaluated, its next token is the VM's and its logits and every state tensor are within the
//!      whole-forward bf16 bound of the VM's.
//!   4. THE PREFILL BUNDLE (`lower_prefill_streamed`): the embedding, one program per layer kind and the head,
//!      each certified; run program by program as a deployment runs them (each row's token and position from the host,
//!      every layer's state and weights from its own names), it emits the VM chunk's token, and every layer leaves the
//!      VM's state — the decode bundle's state, by name.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test qwen38_prefill_lock`
//!
//! ★ MUTATION PROBES: see the bottom of this header once run.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen38::{
    gen_qwen38_decode_batch_embed_lg, gen_qwen38_decode_step_lg, gen_qwen38_prefill_head_lg,
    gen_qwen38_prefill_layer_lg, gen_qwen38_prefill_lg, Qwen38Cfg,
};
use logicaffeine_compile::spatial::streamed::Storage;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Inputs, Outputs, Value};
use logicaffeine_compile::spatial::ir::{SpatialProgram, TensorId, TensorRole};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::streamed::lower_prefill_streamed;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use std::collections::BTreeMap;

#[path = "common/spatial.rs"]
#[allow(dead_code)]
mod spatial_support;
use spatial_support::inputs_from_fills;

const CTX: usize = 8;
/// The prompt chunk: three tokens of one sequence, the first at position `POS0` (1-based).
const TOKS: [usize; 3] = [5, 2, 7];
const POS0: usize = 3;

fn cfg() -> Qwen38Cfg {
    Qwen38Cfg::tiny()
}

/// Element `i` of tensor `var`'s fill: a hash of the index and the name, a row's own copy of a tensor (`Kc3_s1`)
/// filled as the tensor it copies (`Kc3`).
fn fill_value(var: &str, i: usize) -> f64 {
    let base = var.split("_s").next().unwrap_or(var);
    let salt = base.bytes().map(|b| b as u64).sum::<u64>();
    let h = (i as u64)
        .wrapping_mul(2654435761)
        .wrapping_add(salt.wrapping_mul(40503));
    ((h % 97) as f64 / 97.0 - 0.5) * 0.4
}

/// Every declared `Real` tensor of `src`, with its element count.
fn declared_reals(src: &str) -> Vec<(String, usize, usize)> {
    src.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("Let ")?;
            let (var, tail) = rest.split_once(" be a ")?;
            let dims = tail.strip_suffix(" tensor of Real.")?;
            let (r, c) = dims.split_once(" by ")?;
            Some((var.to_string(), r.parse().ok()?, c.parse().ok()?))
        })
        .collect()
}

/// `src` with every declared `Real` tensor filled: from `carried` when it holds the tensor, else by [`fill_value`].
fn with_fills(src: &str, carried: &BTreeMap<String, Vec<f64>>) -> String {
    let decls: BTreeMap<String, usize> = declared_reals(src)
        .into_iter()
        .map(|(v, r, c)| (v, r * c))
        .collect();
    let mut out = String::new();
    for line in src.lines() {
        out.push_str(line);
        out.push('\n');
        let Some(var) = line
            .strip_prefix("Let ")
            .and_then(|r| r.split_once(" be a "))
            .map(|(v, _)| v)
        else {
            continue;
        };
        let Some(n) = decls.get(var) else { continue };
        for i in 0..*n {
            let v = carried
                .get(var)
                .map(|vs| vs[i])
                .unwrap_or_else(|| fill_value(var, i));
            out.push_str(&format!("Set item {} of {var} to {v:?}.\n", i + 1));
        }
    }
    out
}

/// The decode step's carried state: its KV caches, conv rings and recurrent memories, by name and shape.
fn state_of(src: &str) -> Vec<(String, usize, usize)> {
    let is_state = |v: &str| {
        ["Kc", "Vc", "Csq", "Csk", "Csv", "St"].iter().any(|p| {
            v.strip_prefix(p)
                .is_some_and(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()))
        })
    };
    declared_reals(src)
        .into_iter()
        .filter(|(v, _, _)| is_state(v))
        .collect()
}

/// Run `src` on the VM showing its next token, its logits and every element of `state`; the token, and the logits
/// (as `logits`) with the state, back.
fn next_and_state(
    src: &str,
    state: &[(String, usize, usize)],
) -> (f64, BTreeMap<String, Vec<f64>>) {
    let state: Vec<(String, usize, usize)> =
        std::iter::once(("logits".to_string(), 1, cfg().vocab))
            .chain(state.iter().cloned())
            .collect();
    let state = &state[..];
    let mut shown = src.to_string();
    shown.push_str("Show next.\n");
    for (v, r, c) in state {
        for i in 1..=r * c {
            shown.push_str(&format!("Show item {i} of {v}.\n"));
        }
    }
    let vm = vm_outcome(&shown);
    assert!(vm.error.is_none(), "the VM runs it: {:?}", vm.error);
    let nums: Vec<f64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().expect("a number"))
        .collect();
    assert_eq!(
        nums.len(),
        1 + state.iter().map(|(_, r, c)| r * c).sum::<usize>(),
        "every shown value printed"
    );
    let mut at = 1;
    let mut out = BTreeMap::new();
    for (v, r, c) in state {
        out.insert(v.clone(), nums[at..at + r * c].to_vec());
        at += r * c;
    }
    (nums[0], out)
}

fn step(tok: usize, pos: usize) -> String {
    gen_qwen38_decode_step_lg(&cfg(), "/w", CTX, tok, pos).0
}

fn chunk() -> String {
    gen_qwen38_prefill_lg(&cfg(), "/w", CTX, &TOKS, POS0).0
}

/// The chunk's tokens decoded one step at a time, each step on the state the last one left.
fn decoded_in_turn() -> (f64, BTreeMap<String, Vec<f64>>) {
    let state = state_of(&step(TOKS[0], POS0));
    let (mut next, mut carried) = (0.0, BTreeMap::new());
    for (i, tok) in TOKS.iter().enumerate() {
        (next, carried) = next_and_state(&with_fills(&step(*tok, POS0 + i), &carried), &state);
    }
    (next, carried)
}

// ───────────────────────────── 1 ─────────────────────────────

#[test]
fn a_prefill_chunk_is_its_tokens_decoded_in_turn_on_the_vm() {
    let state = state_of(&step(TOKS[0], POS0));
    assert!(
        state.iter().any(|(v, _, _)| v.starts_with("Kc"))
            && state.iter().any(|(v, _, _)| v.starts_with("St")),
        "the tiny model carries both KV caches and recurrent memories: {state:?}"
    );
    let (want_next, want) = decoded_in_turn();
    let (got_next, got) = next_and_state(&with_fills(&chunk(), &BTreeMap::new()), &state);
    assert_eq!(
        got_next, want_next,
        "the chunk emits the token the last of its steps emits"
    );
    for (v, w) in &want {
        let g = &got[v];
        for (k, (a, b)) in g.iter().zip(w).enumerate() {
            assert!(
                (a - b).abs() <= 1e-12 * b.abs().max(1.0),
                "{v}[{k}]: chunk {a} vs steps {b}"
            );
        }
    }
    assert!(
        got.contains_key("logits") && want.contains_key("logits"),
        "the logits are compared, not the token alone"
    );
    // The chunk moved the state: the steps are not a no-op the comparison could pass on.
    let initial: BTreeMap<String, Vec<f64>> = state
        .iter()
        .map(|(v, r, c)| (v.clone(), (0..r * c).map(|i| fill_value(v, i)).collect()))
        .collect();
    assert!(
        initial.iter().all(|(v, i)| &want[v] != i),
        "every state tensor is updated by the chunk"
    );
}

// ───────────────────────────── 2 ─────────────────────────────

#[test]
fn the_prefill_chunk_carries_the_decode_steps_state_and_reads_the_lm_head_once() {
    let (c, s) = (chunk(), step(TOKS[0], POS0));
    assert_eq!(
        state_of(&c),
        state_of(&s),
        "the chunk declares exactly the decode step's carried state, by name and shape"
    );
    let head: Vec<&str> = c.lines().filter(|l| l.contains("matmul Wout")).collect();
    assert_eq!(
        head,
        ["Let logits be xf matmul Wout."],
        "one LM head, on one row"
    );
    let last = format!("Let xl be row {} of bxo{}.", TOKS.len(), cfg().layers - 1);
    assert!(
        c.lines().any(|l| l == last)
            && c.lines()
                .any(|l| l == "Let xf be rmsnorm of xl with weight on."),
        "the LM head reads the last token's row of the last layer's stack (`{last}`)"
    );
}

// ───────────────────────────── 3 ─────────────────────────────

/// `qwen38_rdu_lock`'s stated bf16 bound for a whole forward, relative to the largest value.
const BF16_WHOLE_FORWARD_REL: f64 = 0.02;

fn composed() -> SpatialProgram {
    let target = RduChip::Sn50.facts().spatial_target();
    let pl = compile_program_to_spatial(&with_fills(&chunk(), &BTreeMap::new()), &target)
        .expect("the walk lifts the chunk");
    pl.whole()
        .unwrap_or_else(|| panic!("the chunk lowers whole: {}", pl.refusal()))
        .clone()
}

#[test]
fn the_prefill_chunk_lowers_whole_on_sn50_and_emits_the_vms_token_and_state() {
    let p = composed();
    assert!(
        p.certificate.as_ref().is_some_and(|c| c.recheck(&p)),
        "the chunk certifies"
    );
    let out =
        eval_choreography_every_tensor(&p, &inputs_from_fills(&p)).expect("the chunk evaluates");
    let state = state_of(&step(TOKS[0], POS0));
    let (want_next, want) = next_and_state(&with_fills(&chunk(), &BTreeMap::new()), &state);
    let value = |name: &str| -> Vec<f64> {
        let t = p
            .tensors
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("`{name}` is in the program"));
        match out
            .tensors
            .get(&t.id)
            .unwrap_or_else(|| panic!("`{name}` is a result"))
        {
            Value::Float(v) => v.clone(),
            Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
        }
    };
    assert_eq!(value("next")[0], want_next, "the chip emits the VM's token");
    for (v, w) in &want {
        let g = value(v);
        let scale = w
            .iter()
            .fold(0.0f64, |m, x| m.max(x.abs()))
            .max(f64::MIN_POSITIVE);
        let drift = g
            .iter()
            .zip(w)
            .map(|(a, b)| (a - b).abs() / scale)
            .fold(0.0, f64::max);
        assert!(
            drift <= BF16_WHOLE_FORWARD_REL,
            "{v}: the chip's state drifts {drift} from the VM's"
        );
    }
}

// ───────────────────────────── 4 ─────────────────────────────

fn declared(src: &str) -> Vec<String> {
    src.lines()
        .filter_map(|l| {
            l.strip_prefix("Let ")
                .and_then(|r| r.split_once(" be "))
                .map(|(v, _)| v.to_string())
        })
        .collect()
}

fn floats(v: &Value) -> Vec<f64> {
    match v {
        Value::Float(f) => f.clone(),
        Value::Int(i) => i.iter().map(|x| *x as f64).collect(),
    }
}

/// The chunk's token ids and its first position, as the host hands them to a program.
fn runtime_input(name: &str) -> Option<Vec<f64>> {
    match name {
        "toks" => Some(TOKS.iter().map(|tok| *tok as f64).collect()),
        "pos_s0" => Some(vec![POS0 as f64]),
        _ => None,
    }
}

/// Run one program of the bundle as a deployment runs it: the hidden stack the previous program produced, each row's
/// token and position from the host, every other model tensor from its bound name's fill.
fn run(
    p: &SpatialProgram,
    model_tensors: &[String],
    bind: &dyn Fn(&str) -> String,
    carried: &[(String, Vec<f64>)],
) -> (Outputs, Vec<(String, TensorId)>) {
    let mut inputs = Inputs::default();
    for t in &p.tensors {
        if t.role == TensorRole::Output {
            continue;
        }
        let bound = bind(&t.name);
        let n = t.elems.rows * t.elems.cols;
        let v = match (
            carried.iter().find(|(name, _)| *name == bound),
            runtime_input(&bound),
        ) {
            (Some((_, v)), _) => v.clone(),
            (None, Some(v)) => v,
            _ if model_tensors.contains(&t.name) => (0..n).map(|i| fill_value(&bound, i)).collect(),
            _ => continue,
        };
        inputs.tensors.insert(t.id, Value::Float(v));
    }
    let out = eval_choreography_every_tensor(p, &inputs)
        .unwrap_or_else(|e| panic!("{}: the program evaluates: {e:?}", p.name));
    let names = p.tensors.iter().map(|t| (bind(&t.name), t.id)).collect();
    (out, names)
}

fn result(out: &Outputs, names: &[(String, TensorId)], name: &str) -> Vec<f64> {
    let id = names
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("`{name}` is in the program"))
        .1;
    floats(
        out.tensors
            .get(&id)
            .unwrap_or_else(|| panic!("`{name}` is a result")),
    )
}

fn drift(got: &[f64], want: &[f64]) -> f64 {
    let scale = want
        .iter()
        .fold(0.0f64, |m, x| m.max(x.abs()))
        .max(f64::MIN_POSITIVE);
    got.iter()
        .zip(want)
        .map(|(a, b)| (a - b).abs() / scale)
        .fold(0.0, f64::max)
}

#[test]
fn one_prefill_chunk_from_the_bundle_is_the_vms_chunk() {
    let c = cfg();
    let t = TOKS.len();
    let fills = |src: &str| with_fills(src, &BTreeMap::new());
    let m =
        lower_prefill_streamed(&c.spec(), Storage::AllQ8, CTX, t, &RduChip::Sn50.facts().spatial_target(), fills)
            .expect("the prefill bundle lowers");
    assert_eq!(m.kinds.len(), 2, "one program per layer kind");
    assert_eq!(m.layers.len(), c.layers, "every layer bound to a kind");
    for p in m
        .embed
        .iter()
        .chain(m.kinds.iter().map(|k| &k.program))
        .chain(std::iter::once(&m.head))
    {
        assert!(
            p.certificate.as_ref().is_some_and(|cert| cert.recheck(p)),
            "{}: the program carries a certificate that rechecks",
            p.name
        );
    }
    let state = state_of(&step(TOKS[0], POS0));
    let (want_next, want) = next_and_state(&with_fills(&chunk(), &BTreeMap::new()), &state);

    let embed_src = declared(&gen_qwen38_decode_batch_embed_lg(&c, "/w", &[1; TOKS.len()]).0);
    let (embed_out, embed_names) = run(
        m.embed.as_ref().expect("an embedding program"),
        &embed_src,
        &|n: &str| n.to_string(),
        &[],
    );
    let mut hidden: (String, Vec<f64>) = ("X".to_string(), result(&embed_out, &embed_names, "X"));
    let mut left: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for binding in &m.layers {
        let kind = &m.kinds[binding.kind];
        let model_tensors =
            declared(&gen_qwen38_prefill_layer_lg(&c, kind.representative, "/w", CTX, 1, t).0);
        let map = binding.tensors.clone();
        let bind = move |name: &str| {
            map.iter()
                .find(|(from, _)| from == name)
                .map(|(_, to)| to.clone())
                .unwrap_or_else(|| name.to_string())
        };
        let (out, names) = run(
            &kind.program,
            &model_tensors,
            &bind,
            std::slice::from_ref(&hidden),
        );
        for (name, _) in names
            .iter()
            .filter(|(n, _)| state.iter().any(|(v, _, _)| v == n))
        {
            left.insert(name.clone(), result(&out, &names, name));
        }
        let next = format!("bxo{}", binding.layer);
        hidden = (next.clone(), result(&out, &names, &next));
    }
    let head_src = declared(&gen_qwen38_prefill_head_lg(&c, "/w", t).0);
    let (head_out, head_names) = run(
        &m.head,
        &head_src,
        &|n: &str| n.to_string(),
        std::slice::from_ref(&hidden),
    );
    assert_eq!(
        result(&head_out, &head_names, "next")[0],
        want_next,
        "the bundle emits the VM chunk's token"
    );
    assert!(
        drift(&result(&head_out, &head_names, "logits"), &want["logits"]) <= BF16_WHOLE_FORWARD_REL,
        "and its logits"
    );
    assert_eq!(
        left.keys().collect::<std::collections::BTreeSet<_>>(),
        state
            .iter()
            .map(|(v, _, _)| v)
            .collect::<std::collections::BTreeSet<_>>(),
        "every layer leaves the decode step's state, by name"
    );
    for (v, got) in &left {
        assert!(
            drift(got, &want[v]) <= BF16_WHOLE_FORWARD_REL,
            "{v}: the bundle's state drifts from the VM chunk's"
        );
    }
}
