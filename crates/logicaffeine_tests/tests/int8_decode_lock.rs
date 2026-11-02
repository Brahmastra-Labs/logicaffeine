//! int8_decode_lock — the tiny int8 transformer's decode step, written in the integer tensor sugar, emits the
//! reference's logits and token at every position of the golden run, and appends the reference's key and value rows.
//!
//! The model is `benchmarks/silicon/tiny_transformer.json`: a char-level transformer whose whole inference is integer
//! arithmetic (int8 weights, `(acc · M) >> S` requant with int8 saturation, an exp-table softmax with a floored mean,
//! a ReLU FFN, a tied head, arg-max). Its golden trace — the prompt, the three generated tokens and their logits — is
//! the byte-exact reference.
//!
//! The step `spatial::int8_model::Int8Model::decode_step_lg` writes is ONE token in, its logits and the next token
//! out, the KV cache carried: this token's key and value land as row `pos` and the attention reads the first `pos`.
//! This lock holds, in ascending order of reality:
//!   1. THE REFERENCE. An incremental integer decode written here, independently of the step, reproduces the golden
//!      trace's tokens and logits — so decoding against a cache is the full recompute's last row, exactly.
//!   2. THE VM. At every position of the run the authored step's logits, token and appended rows are the reference's.
//!   3. THE SPATIAL EVALUATOR. The step lowers whole onto the decode fabric and its logits and token are the VM's.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test int8_decode_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

fn json() -> serde_json::Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    serde_json::from_str(&std::fs::read_to_string(&p).expect("the tiny transformer"))
        .expect("the tiny transformer is JSON")
}

fn ints(v: &serde_json::Value) -> Vec<i64> {
    v.as_array()
        .expect("an array")
        .iter()
        .map(|x| x.as_i64().expect("an integer"))
        .collect()
}

/// The integer fabric the decode step is built for (`int_tensor_sugar_lock`'s): an accumulator deep enough for a whole
/// flat row of the model's widest stage.
fn decode_fabric() -> SpatialTarget {
    SpatialTarget::sky_fabric(16, 64)
}

// ── THE REFERENCE: the model's integer arithmetic, decoding against a cache ────────────────────────────────────────

struct Reference {
    d: usize,
    h: usize,
    d_ff: usize,
    vocab: usize,
    rq: std::collections::HashMap<String, (i64, u32)>,
    w: std::collections::HashMap<String, Vec<i64>>,
    embed: Vec<i64>,
    pos: Vec<i64>,
    lut: Vec<i64>,
}

/// One position's output: this token's key and value rows, the logits, and the arg-max.
#[derive(Clone, Debug)]
struct Step {
    k: Vec<i64>,
    v: Vec<i64>,
    logits: Vec<i64>,
    next: usize,
}

impl Reference {
    fn of(j: &serde_json::Value) -> Reference {
        let c = &j["config"];
        let u = |k: &str| c[k].as_u64().expect("a size") as usize;
        let rq = j["requant"]
            .as_object()
            .expect("requant")
            .iter()
            .map(|(k, v)| {
                let p = ints(v);
                (k.clone(), (p[0], p[1] as u32))
            })
            .collect();
        let w = j["weights_int8"]
            .as_object()
            .expect("weights")
            .iter()
            .map(|(k, v)| (k.clone(), ints(v)))
            .collect();
        Reference {
            d: u("d_model"),
            h: u("n_heads"),
            d_ff: u("d_ff"),
            vocab: u("vocab"),
            rq,
            w,
            embed: ints(&j["embed_stream"]["EinA"]),
            pos: ints(&j["embed_stream"]["PinA"]),
            lut: ints(&j["exp_lut"]),
        }
    }

    fn rq(&self, x: i64, k: &str) -> i64 {
        let (m, s) = self.rq[k];
        (x * m) >> s
    }

    fn to8(&self, x: i64, k: &str) -> i64 {
        self.rq(x, k).clamp(-128, 127)
    }

    /// `x (1 × rows) · W (rows × cols)`, W row-major.
    fn matvec(x: &[i64], w: &[i64], cols: usize) -> Vec<i64> {
        (0..cols)
            .map(|o| x.iter().enumerate().map(|(i, xi)| xi * w[i * cols + o]).sum())
            .collect()
    }

    /// Decode `tok` at position `p` (0-based) against the cache rows of positions `0..p`.
    fn step(&self, tok: usize, p: usize, kc: &[Vec<i64>], vc: &[Vec<i64>]) -> Step {
        let (d, h) = (self.d, self.h);
        let hd = d / h;
        let stream: Vec<i64> = (0..d).map(|j| self.embed[tok * d + j] + self.pos[p * d + j]).collect();
        let xi: Vec<i64> = stream.iter().map(|&x| self.to8(x, "r_qkv")).collect();
        let proj = |wn: &str, m: &str| -> Vec<i64> {
            Self::matvec(&xi, &self.w[wn], d).into_iter().map(|a| self.to8(a, m)).collect()
        };
        let (q, k, v) = (proj("Wq", "M_q"), proj("Wk", "M_k"), proj("Wv", "M_v"));
        let keys: Vec<&Vec<i64>> = kc.iter().chain(std::iter::once(&k)).collect();
        let vals: Vec<&Vec<i64>> = vc.iter().chain(std::iter::once(&v)).collect();
        let mut ctx = vec![0i64; d];
        for head in 0..h {
            let sl = head * hd..(head + 1) * hd;
            let scores: Vec<i64> = keys
                .iter()
                .map(|kj| sl.clone().map(|c| q[c] * kj[c]).sum())
                .collect();
            let mx = *scores.iter().max().expect("a position");
            let mut num = vec![0i64; hd];
            let mut den = 0i64;
            for (j, s) in scores.iter().enumerate() {
                let idx = self.rq(mx - s, "M_score").clamp(0, self.lut.len() as i64 - 1) as usize;
                let wgt = self.lut[idx];
                for (c, n) in num.iter_mut().enumerate() {
                    *n += wgt * vals[j][head * hd + c];
                }
                den += wgt;
            }
            for c in 0..hd {
                ctx[head * hd + c] = num[c].div_euclid(den);
            }
        }
        let ci: Vec<i64> = ctx.iter().map(|&x| self.to8(x, "r_ctx")).collect();
        let o: Vec<i64> = Self::matvec(&ci, &self.w["Wo"], d).into_iter().map(|a| self.rq(a, "M_o")).collect();
        let s1: Vec<i64> = stream.iter().zip(&o).map(|(a, b)| a + b).collect();
        let hi: Vec<i64> = s1.iter().map(|&x| self.to8(x, "r_ffn")).collect();
        let hid: Vec<i64> = Self::matvec(&hi, &self.w["W1"], self.d_ff)
            .into_iter()
            .map(|a| self.rq(a, "M_h").max(0))
            .collect();
        let fi: Vec<i64> = hid.iter().map(|&x| self.to8(x, "r_hid")).collect();
        let f: Vec<i64> = Self::matvec(&fi, &self.w["W2"], d).into_iter().map(|a| self.rq(a, "M_f")).collect();
        let s2: Vec<i64> = s1.iter().zip(&f).map(|(a, b)| a + b).collect();
        let oi: Vec<i64> = s2.iter().map(|&x| self.to8(x, "r_out")).collect();
        let eq = &self.w["Eq"];
        let logits: Vec<i64> = (0..self.vocab)
            .map(|t| (0..d).map(|j| oi[j] * eq[t * d + j]).sum())
            .collect();
        let mut next = 0;
        for (t, l) in logits.iter().enumerate() {
            if *l > logits[next] {
                next = t;
            }
        }
        Step { k, v, logits, next }
    }

    /// The whole golden run: every prompt token, then each generated one, decoded in turn.
    fn run(&self, prompt: &[usize], gen_len: usize) -> Vec<(usize, Step)> {
        let (mut kc, mut vc) = (Vec::new(), Vec::new());
        let mut out = Vec::new();
        let mut tok = prompt[0];
        for p in 0..prompt.len() + gen_len - 1 {
            let s = self.step(tok, p, &kc, &vc);
            kc.push(s.k.clone());
            vc.push(s.v.clone());
            out.push((tok, s.clone()));
            tok = if p + 1 < prompt.len() { prompt[p + 1] } else { s.next };
        }
        out
    }
}

fn golden() -> (Reference, Vec<usize>, Vec<usize>, Vec<Vec<i64>>) {
    let j = json();
    let g = &j["golden_trace"];
    let prompt: Vec<usize> = ints(&g["prompt_tokens"]).into_iter().map(|t| t as usize).collect();
    let generated: Vec<usize> = ints(&g["generated_tokens"]).into_iter().map(|t| t as usize).collect();
    let logits: Vec<Vec<i64>> = g["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .map(|s| ints(&s["logits"]))
        .collect();
    (Reference::of(&j), prompt, generated, logits)
}

/// The reference run with its cache rows: per position `(token, step, the cache rows before it)`.
fn reference_run() -> Vec<(usize, usize, Step, Vec<Vec<i64>>, Vec<Vec<i64>>)> {
    let (r, prompt, generated, _) = golden();
    let run = r.run(&prompt, generated.len());
    let mut out = Vec::new();
    for (p, (tok, s)) in run.iter().enumerate() {
        let kc: Vec<Vec<i64>> = run[..p].iter().map(|(_, s)| s.k.clone()).collect();
        let vc: Vec<Vec<i64>> = run[..p].iter().map(|(_, s)| s.v.clone()).collect();
        out.push((p, *tok, s.clone(), kc, vc));
    }
    out
}

#[test]
fn the_reference_decode_reproduces_the_golden_trace() {
    let (r, prompt, generated, logits) = golden();
    let run = r.run(&prompt, generated.len());
    let tail = &run[prompt.len() - 1..];
    let tokens: Vec<usize> = tail.iter().map(|(_, s)| s.next).collect();
    assert_eq!(tokens, generated, "the generated tokens");
    for (i, (_, s)) in tail.iter().enumerate() {
        assert_eq!(s.logits, logits[i], "the logits of generation step {i}");
    }
}

// ── THE STEP ON THE VM ───────────────────────────────────────────────────────────────────────────────────────────────

/// The step for position `p`, with every tensor it computes shown: the logits, the token and this position's rows.
fn shown_step(m: &Int8Model, tok: usize, p: usize, kc: &[Vec<i64>], vc: &[Vec<i64>]) -> String {
    let flat = |rows: &[Vec<i64>]| rows.iter().flatten().copied().collect::<Vec<i64>>();
    let mut s = m.decode_step_lg(tok, p, &flat(kc), &flat(vc));
    let d = m.d;
    s.push_str(&format!(
        "Repeat for k from 1 to {}:\n    Show item k of logits.\nShow next.\n",
        m.vocab
    ));
    for t in ["Kc", "Vc"] {
        s.push_str(&format!(
            "Repeat for k from 1 to {d}:\n    Show item ({p} * {d} + k) of {t}.\n"
        ));
    }
    s
}

fn vm_ints(src: &str) -> Vec<i64> {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "the VM refused the step: {:?}", vm.error);
    vm.output.lines().map(|l| l.trim().parse().expect("an Int")).collect()
}

#[test]
fn the_authored_step_is_the_reference_on_the_vm_at_every_position() {
    let m = Int8Model::from_json(&json()).expect("the model");
    for (p, tok, want, kc, vc) in reference_run() {
        let got = vm_ints(&shown_step(&m, tok, p, &kc, &vc));
        let v = m.vocab;
        assert_eq!(&got[..v], &want.logits[..], "position {p}: the logits");
        assert_eq!(got[v] as usize, want.next + 1, "position {p}: the 1-based token");
        assert_eq!(&got[v + 1..v + 1 + m.d], &want.k[..], "position {p}: the appended key row");
        assert_eq!(&got[v + 1 + m.d..], &want.v[..], "position {p}: the appended value row");
    }
}

// ── THE STEP ON THE SPATIAL EVALUATOR ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_step_lowers_whole_on_the_decode_fabric() {
    let m = Int8Model::from_json(&json()).expect("the model");
    let run = reference_run();
    let (p, tok, _, kc, vc) = &run[5];
    let flat = |rows: &[Vec<i64>]| rows.iter().flatten().copied().collect::<Vec<i64>>();
    let src = m.decode_step_lg(*tok, *p, &flat(kc), &flat(vc));
    let l = compile_program_to_spatial(&src, &decode_fabric()).expect("a tensor program");
    assert!(
        l.program.is_some(),
        "the whole step lowers and certifies: {:?}",
        l.missing
    );
}

/// The positions the spatial evaluator is held at: the first, an early one, the last of the prompt and every
/// generated one — the masked attention's two ends and the logits the golden trace records.
const SPATIAL_POSITIONS: [usize; 6] = [0, 1, 7, 18, 19, 20];

#[test]
fn the_step_on_the_spatial_evaluator_is_the_vms_logits_and_token() {
    let m = Int8Model::from_json(&json()).expect("the model");
    let run = reference_run();
    let flat = |rows: &[Vec<i64>]| rows.iter().flatten().copied().collect::<Vec<i64>>();
    // `logits` is an ACTIVATION of the step — the arg-max consumes it, and an evaluation hands back results and carried
    // state only — so it is read from the same step lowered without its last statement, where it is the result.
    let result = |src: &str, name: &str, p: usize| -> Vec<i64> {
        let l = compile_program_to_spatial(src, &decode_fabric()).expect("a tensor program");
        let prog = l.program.unwrap_or_else(|| panic!("position {p} lowers: {:?}", l.missing.first()));
        let out = eval_choreography(&prog, &inputs_from_fills(&prog)).expect("the step evaluates");
        let t = prog.tensors.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("{name}")).id;
        match out.tensors.get(&t).unwrap_or_else(|| panic!("position {p}: the evaluation hands back `{name}`")) {
            Value::Int(v) => v.clone(),
            Value::Float(v) => panic!("{name} evaluated through floats: {v:?}"),
        }
    };
    for &p in &SPATIAL_POSITIONS {
        let (_, tok, want, kc, vc) = &run[p];
        let src = m.decode_step_lg(*tok, p, &flat(kc), &flat(vc));
        let open = src.replace("Let next be argmax of logits.\n", "");
        assert_ne!(open, src, "the step ends with its arg-max");
        assert_eq!(result(&open, "logits", p), want.logits, "position {p}: the logits");
        assert_eq!(result(&src, "next", p), vec![want.next as i64 + 1], "position {p}: the 1-based token");
    }
}
