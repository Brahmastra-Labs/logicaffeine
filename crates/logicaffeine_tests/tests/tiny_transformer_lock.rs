//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! TINY-TRANSFORMER LOCK — the baby char-level LLM's inference is byte-identical to the reference at
//! every hop, the property a dataflow deployment rests on.
//!
//! The model (trained offline by `scripts/train_tiny_transformer.py`, exported to
//! `benchmarks/silicon/tiny_transformer.json`) is a genuine char-level transformer — embedding +
//! positional add, multi-head causal attention with an integer exp-LUT softmax, a ReLU FFN, tied output
//! projection, argmax next-token — all in PURE INTEGER int8 arithmetic (matmul int8→int, requant
//! `(acc*M)>>S` arithmetic-shift, integer argmax). That integer path is the golden oracle.
//!
//! This lock proves, in ascending order of reality:
//!   • RUST ORACLE ≡ the exported golden trace — the arithmetic is implementation-independent (not a
//!     numpy artifact): a from-scratch Rust re-implementation reproduces every per-step logit vector and
//!     the generated string ("mat") byte-for-byte.
//!   • (later tests) the SAME forward pass written in LOGOS runs ≡ the oracle on the VM / tree-walk /
//!     full soft-core, and the int8 matmul kernel lowers to the certified fabric (transformer_fabric_lock).
//!
//! Determinism: `scripts/train_tiny_transformer.py --check` re-derives the JSON from the committed corpus.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words;
use logicaffeine_compile::codegen_sva::soft_core_model::{run_soft_core_typed_preloaded, ShowVal};
use logicaffeine_compile::compile::vm_outcome;

#[path = "common/mod.rs"]
mod common;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

// ── the exported model + golden trace ─────────────────────────────────────────────────────────────

/// The int8 model parsed from `tiny_transformer.json`. Weights are row-major flat `Vec<i64>`; the
/// `(M,S)` requant params are `[mul, shift]` pairs consumed verbatim (the same integers LOGOS uses).
struct Model {
    d: usize,
    h: usize,
    d_ff: usize,
    vocab: usize,
    chars: Vec<char>,
    // requant (M,S)
    r_qkv: (i64, i64),
    r_ffn: (i64, i64),
    r_out: (i64, i64),
    r_ctx: (i64, i64),
    r_hid: (i64, i64),
    m_q: (i64, i64),
    m_k: (i64, i64),
    m_v: (i64, i64),
    m_o: (i64, i64),
    m_h: (i64, i64),
    m_f: (i64, i64),
    m_score: (i64, i64),
    // weights (row-major)
    wq: Vec<i64>,
    wk: Vec<i64>,
    wv: Vec<i64>,
    wo: Vec<i64>,
    w1: Vec<i64>,
    w2: Vec<i64>,
    eq: Vec<i64>,
    ein_a: Vec<i64>,
    pin_a: Vec<i64>,
    exp_lut: Vec<i64>,
}

struct Golden {
    prompt_tokens: Vec<usize>,
    generated_tokens: Vec<usize>,
    generated_text: String,
    step_logits: Vec<Vec<i64>>, // per generated step: the last-position logit vector
}

fn ints(v: &serde_json::Value) -> Vec<i64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect()
}
fn ms(v: &serde_json::Value, k: &str) -> (i64, i64) {
    let a = v[k].as_array().unwrap();
    (a[0].as_i64().unwrap(), a[1].as_i64().unwrap())
}

fn load() -> (Model, Golden) {
    let path = root().join("benchmarks/silicon/tiny_transformer.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e} — run `python3 scripts/train_tiny_transformer.py`",
            path.display()
        )
    });
    let j: serde_json::Value = serde_json::from_str(&raw).expect("parse tiny_transformer.json");
    let cfg = &j["config"];
    let rq = &j["requant"];
    let w = &j["weights_int8"];
    let model = Model {
        d: cfg["d_model"].as_u64().unwrap() as usize,
        h: cfg["n_heads"].as_u64().unwrap() as usize,
        d_ff: cfg["d_ff"].as_u64().unwrap() as usize,
        vocab: cfg["vocab"].as_u64().unwrap() as usize,
        chars: cfg["chars"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().chars().next().unwrap())
            .collect(),
        r_qkv: ms(rq, "r_qkv"),
        r_ffn: ms(rq, "r_ffn"),
        r_out: ms(rq, "r_out"),
        r_ctx: ms(rq, "r_ctx"),
        r_hid: ms(rq, "r_hid"),
        m_q: ms(rq, "M_q"),
        m_k: ms(rq, "M_k"),
        m_v: ms(rq, "M_v"),
        m_o: ms(rq, "M_o"),
        m_h: ms(rq, "M_h"),
        m_f: ms(rq, "M_f"),
        m_score: ms(rq, "M_score"),
        wq: ints(&w["Wq"]),
        wk: ints(&w["Wk"]),
        wv: ints(&w["Wv"]),
        wo: ints(&w["Wo"]),
        w1: ints(&w["W1"]),
        w2: ints(&w["W2"]),
        eq: ints(&w["Eq"]),
        ein_a: ints(&j["embed_stream"]["EinA"]),
        pin_a: ints(&j["embed_stream"]["PinA"]),
        exp_lut: ints(&j["exp_lut"]),
    };
    let g = &j["golden_trace"];
    let golden = Golden {
        prompt_tokens: g["prompt_tokens"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap() as usize)
            .collect(),
        generated_tokens: g["generated_tokens"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap() as usize)
            .collect(),
        generated_text: g["generated_text"].as_str().unwrap().to_string(),
        step_logits: g["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| ints(&s["logits"]))
            .collect(),
    };
    (model, golden)
}

// ── the int8 forward pass, re-implemented from scratch in Rust (the arithmetic spec) ───────────────
// EXACT integer semantics, matching numpy AND LOGOS:
//   `>>` is an arithmetic shift (floor toward −∞ for signed) — Rust signed `>>` already is.
//   the attention normalize `num // den` is FLOOR division — Rust `/` truncates, so use `div_euclid`
//   (den > 0 always, so div_euclid == Python floor //).

fn rq(acc: i64, (m, s): (i64, i64)) -> i64 {
    (acc * m) >> s // arithmetic shift right
}
fn to8(x: i64, mspair: (i64, i64)) -> i64 {
    rq(x, mspair).clamp(-128, 127)
}

/// `out[p][o] = f(Σ_i x[p*din+i] * w[i*dout+o])` — w is row-major `[din, dout]`.
fn matmul(
    x: &[i64],
    w: &[i64],
    n: usize,
    din: usize,
    dout: usize,
    f: impl Fn(i64) -> i64,
) -> Vec<i64> {
    let mut out = vec![0i64; n * dout];
    for p in 0..n {
        for o in 0..dout {
            let mut acc = 0i64;
            for i in 0..din {
                acc += x[p * din + i] * w[i * dout + o];
            }
            out[p * dout + o] = f(acc);
        }
    }
    out
}

/// int8 forward for a token context; returns the last-position logit vector (argmax = next token).
fn forward(m: &Model, toks: &[usize]) -> Vec<i64> {
    let (d, h, dff, v) = (m.d, m.h, m.d_ff, m.vocab);
    let hd = d / h;
    let n = toks.len();
    // embed + pos -> residual stream (wide int, scale sA): a clean integer add
    let mut stream = vec![0i64; n * d];
    for (p, &tok) in toks.iter().enumerate() {
        for j in 0..d {
            stream[p * d + j] = m.ein_a[tok * d + j] + m.pin_a[p * d + j];
        }
    }
    // attention
    let xi: Vec<i64> = stream.iter().map(|&x| to8(x, m.r_qkv)).collect();
    let q = matmul(&xi, &m.wq, n, d, d, |a| to8(a, m.m_q));
    let k = matmul(&xi, &m.wk, n, d, d, |a| to8(a, m.m_k));
    let vv = matmul(&xi, &m.wv, n, d, d, |a| to8(a, m.m_v));
    let mut ctx = vec![0i64; n * d];
    for head in 0..h {
        let lo = head * hd;
        for i in 0..n {
            let mut sc = vec![0i64; i + 1];
            for j in 0..=i {
                let mut a = 0i64;
                for c in 0..hd {
                    a += q[i * d + lo + c] * k[j * d + lo + c];
                }
                sc[j] = a;
            }
            let mx = *sc.iter().max().unwrap();
            let mut num = vec![0i64; hd];
            let mut den = 0i64;
            for j in 0..=i {
                let idx = rq(mx - sc[j], m.m_score).clamp(0, 255) as usize;
                let w = m.exp_lut[idx];
                for c in 0..hd {
                    num[c] += w * vv[j * d + lo + c];
                }
                den += w;
            }
            for c in 0..hd {
                ctx[i * d + lo + c] = if den != 0 { num[c].div_euclid(den) } else { 0 };
            }
        }
    }
    let ci: Vec<i64> = ctx.iter().map(|&x| to8(x, m.r_ctx)).collect();
    let attn_out = matmul(&ci, &m.wo, n, d, d, |a| rq(a, m.m_o)); // -> stream scale (no clip)
    for p in 0..n * d {
        stream[p] += attn_out[p];
    }
    // FFN + ReLU
    let hi: Vec<i64> = stream.iter().map(|&x| to8(x, m.r_ffn)).collect();
    let hid: Vec<i64> = matmul(&hi, &m.w1, n, d, dff, |a| rq(a, m.m_h).max(0)); // requant then ReLU
    let fi: Vec<i64> = hid.iter().map(|&x| to8(x, m.r_hid)).collect();
    let ffn = matmul(&fi, &m.w2, n, dff, d, |a| rq(a, m.m_f));
    for p in 0..n * d {
        stream[p] += ffn[p];
    }
    // tied output projection -> logits (Eq is [vocab, d]; logits[p][o] = Σ_i Oi[p*d+i] * Eq[o*d+i])
    let oi: Vec<i64> = stream.iter().map(|&x| to8(x, m.r_out)).collect();
    let mut logits = vec![0i64; v];
    let last = n - 1;
    for o in 0..v {
        let mut acc = 0i64;
        for i in 0..d {
            acc += oi[last * d + i] * m.eq[o * d + i];
        }
        logits[o] = acc;
    }
    logits
}

fn argmax(v: &[i64]) -> usize {
    let mut best = 0;
    for i in 1..v.len() {
        if v[i] > v[best] {
            best = i;
        }
    }
    best
}

// ════════════════════════════════════════════════════════════════════════════════════════════════

/// RUST ORACLE ≡ golden trace — the int8 inference arithmetic is implementation-independent: a
/// from-scratch Rust re-implementation reproduces every per-step logit vector and the generated string.
#[test]
fn rust_oracle_matches_the_golden_trace() {
    let (m, g) = load();
    let mut toks = g.prompt_tokens.clone();
    let mut gen = Vec::new();
    for (step, expect_logits) in g.step_logits.iter().enumerate() {
        let logits = forward(&m, &toks);
        assert_eq!(
            &logits, expect_logits,
            "step {step}: Rust logits diverge from the golden trace"
        );
        let nxt = argmax(&logits);
        assert_eq!(
            nxt, g.generated_tokens[step],
            "step {step}: argmax token diverges"
        );
        gen.push(nxt);
        toks.push(nxt);
    }
    let text: String = gen.iter().map(|&t| m.chars[t]).collect();
    assert_eq!(
        text, g.generated_text,
        "generated string diverges from the golden trace"
    );
    assert_eq!(
        text, "mat",
        "the baby transformer must complete 'the cat sat on the ' -> 'mat'"
    );
    println!("Rust oracle ≡ golden trace: int8 inference reproduces {text:?} byte-for-byte (impl-independent).");
}

// ── the SAME forward pass, written in LOGOS, run on the VM ─────────────────────────────────────────

/// Generate the LOGOS forward-pass program for a step via the validated generator (reads the committed
/// JSON, emits the int8 forward pass). `logits` = Show every last-position logit; else Show the argmax.
fn gen_lg(step: usize, logits: bool) -> Option<String> {
    let mut cmd = Command::new("python3");
    cmd.current_dir(root())
        .arg("scripts/gen_tiny_transformer_lg.py")
        .arg("--step")
        .arg(step.to_string());
    if logits {
        cmd.arg("--logits");
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        eprintln!(
            "gen_tiny_transformer_lg.py failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn vm_ints(src: &str) -> Vec<i64> {
    let out = vm_outcome(src);
    assert!(out.error.is_none(), "VM error: {:?}", out.error);
    out.output
        .split_whitespace()
        .map(|t| t.parse::<i64>().expect("int Show"))
        .collect()
}

/// LOGOS FORWARD PASS ≡ oracle on the VM, BYTE-EXACT: the LOGOS int8 forward pass reproduces every
/// last-position logit integer of the golden trace at each generation step (not just the argmax).
#[test]
fn logos_forward_reproduces_golden_logits_byte_exact_on_the_vm() {
    let (_m, g) = load();
    if !common::require_tool("python3") {
        return;
    }
    for (step, expect) in g.step_logits.iter().enumerate() {
        let src = gen_lg(step, true).expect("generate LOGOS forward-pass program");
        let got = vm_ints(&src);
        assert_eq!(
            &got, expect,
            "step {step}: LOGOS last-position logits diverge from the golden trace on the VM"
        );
    }
    println!(
        "LOGOS forward pass ≡ golden trace on the VM, byte-exact (every logit, all {} steps).",
        g.step_logits.len()
    );
}

/// LOGOS GENERATES "mat" on the VM: the argmax of the LOGOS forward pass reproduces the golden token at
/// each step, spelling the completion.
#[test]
fn logos_generates_mat_on_the_vm() {
    let (m, g) = load();
    if !common::require_tool("python3") {
        return;
    }
    let mut text = String::new();
    for (step, &tok) in g.generated_tokens.iter().enumerate() {
        let src = gen_lg(step, false).expect("generate LOGOS forward-pass program");
        let got = vm_ints(&src);
        assert_eq!(got.len(), 1, "step {step}: expected a single argmax Show");
        assert_eq!(
            got[0] as usize, tok,
            "step {step}: LOGOS argmax token != golden"
        );
        text.push(m.chars[got[0] as usize]);
    }
    assert_eq!(text, "mat", "LOGOS must generate 'mat' on the VM");
    println!("LOGOS generates {text:?} on the VM — the baby transformer runs end-to-end ≡ the reference.");
}

// ── the ON-CHIP path: weights streamed as a DATA segment, read via `loaded()`, on the soft-core MODEL ──

/// The concatenated weight blob the SoC preloads into the heap (`gen_tiny_transformer_lg.py --blob`).
fn gen_blob() -> Option<Vec<i64>> {
    let out = Command::new("python3")
        .current_dir(root())
        .args(["scripts/gen_tiny_transformer_lg.py", "--blob"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    Some(
        raw.trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().parse::<i64>().expect("blob int"))
            .collect(),
    )
}

/// The on-chip variant program (`--board`): weights via `loaded()` copied from the preloaded blob.
fn gen_board_lg(step: usize) -> Option<String> {
    let out = Command::new("python3")
        .current_dir(root())
        .args([
            "scripts/gen_tiny_transformer_lg.py",
            "--board",
            "--step",
            &step.to_string(),
        ])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// FULL-CORE (silicon twin) ≡ oracle via the DATA-segment loader: the whole board transformer — weights
/// streamed into the heap as a blob and read through `loaded()` (pointer 0), exactly the on-chip path —
/// generates "mat" byte-identical to the golden trace on `run_soft_core_typed` with the heap preloaded.
/// This locks the weight-loading mechanism (the `loaded()` builtin + `dmem` preload) end to end.
#[test]
fn board_transformer_runs_on_the_soft_core_model_via_loaded() {
    let (m, g) = load();
    if !common::require_tool("python3") {
        return;
    }
    let blob = gen_blob().expect("generate the weight blob");
    let mut text = String::new();
    for (step, &tok) in g.generated_tokens.iter().enumerate() {
        let lg = gen_board_lg(step).expect("generate the board program");
        let words = compile_to_full_words(&lg).expect("compile board program to full-core words");
        let out = run_soft_core_typed_preloaded(&words, 8_000_000, &blob);
        let last = out.last().and_then(|s| match s {
            ShowVal::Int(v) => Some(*v),
            _ => None,
        });
        assert_eq!(
            last,
            Some(tok as i64),
            "step {step}: board argmax != golden (via loaded())"
        );
        text.push(m.chars[tok]);
    }
    assert_eq!(
        text, "mat",
        "the board transformer must generate 'mat' on the soft-core model"
    );
    println!("FULL-CORE (silicon twin) via the DATA-segment loader: weights streamed + read through loaded() → {text:?} ≡ golden.");
}
