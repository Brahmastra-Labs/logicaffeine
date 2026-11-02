//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! QWEN-TINY (FLOAT) LOGOS ≡ ORACLE — Phase 0 of "Qwen authored in LOGOS".
//!
//! The FIRST *float* transformer written in LOGOS `.lg` (`scripts/accel/gen_qwen_tiny_lg.py`) is a faithful
//! structural shrink of the real Qwen3 forward (`codegen_cuda/qwen.rs`): RMSNorm, per-head QK-RMSNorm,
//! RoPE-NEOX, grouped-query causal attention with an `exp` softmax, SwiGLU FFN, residuals, final norm +
//! LM head, argmax next token — every stage in `Real` (f64).
//!
//! This lock proves the LOGOS program runs BYTE-EXACT against an independent Rust f64 oracle: not merely
//! the argmax, but every last-position logit, at every generation step. Byte-exactness is achievable
//! because the only transcendental is the 20-term Taylor `expT` (identical to `assets/std/math.lg`'s
//! `exp`) plus the builtin `sqrt` (IEEE `f64::sqrt`), both mirrored here, and RoPE cos/sin are
//! precomputed host-side and inlined as data (read identically by both sides).
//!
//! The oracle is clean-room (follows `qwen.rs`, not the generator). If they diverge, the IMPLEMENTATION
//! (the `.lg`/generator) is wrong — never edit this oracle to match a buggy program.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::compile::vm_outcome;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

// ── model ──────────────────────────────────────────────────────────────────────────────────────────

struct Cfg {
    d: usize,
    h: usize,
    kv: usize,
    hd: usize,
    dff: usize,
    vocab: usize,
    l: usize,
    eps: f64,
    half: usize,
}

struct Layer {
    attn_norm: Vec<f64>,
    q_norm: Vec<f64>,
    k_norm: Vec<f64>,
    ffn_norm: Vec<f64>,
    wq: Vec<f64>,
    wk: Vec<f64>,
    wv: Vec<f64>,
    wo: Vec<f64>,
    gate: Vec<f64>,
    up: Vec<f64>,
    down: Vec<f64>,
}

struct Model {
    cfg: Cfg,
    emb: Vec<f64>,
    onorm: Vec<f64>,
    outp: Vec<f64>,
    rc: Vec<f64>,
    rs: Vec<f64>,
    layers: Vec<Layer>,
    prompt: Vec<usize>,
    gen_len: usize,
}

fn floats(v: &serde_json::Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn load() -> Model {
    let path = root().join("benchmarks/silicon/qwen_tiny.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e} — run `python3 scripts/accel/gen_qwen_tiny_lg.py --emit-json > {}`",
            path.display(),
            path.display()
        )
    });
    let j: serde_json::Value = serde_json::from_str(&raw).expect("parse qwen_tiny.json");
    let c = &j["config"];
    let cfg = Cfg {
        d: c["d_model"].as_u64().unwrap() as usize,
        h: c["n_heads"].as_u64().unwrap() as usize,
        kv: c["n_kv_heads"].as_u64().unwrap() as usize,
        hd: c["head_dim"].as_u64().unwrap() as usize,
        dff: c["d_ff"].as_u64().unwrap() as usize,
        vocab: c["vocab"].as_u64().unwrap() as usize,
        l: c["n_layers"].as_u64().unwrap() as usize,
        eps: c["eps"].as_f64().unwrap(),
        half: j["rope"]["half"].as_u64().unwrap() as usize,
    };
    let w = &j["weights"];
    let layers = w["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ly| Layer {
            attn_norm: floats(&ly["attn_norm"]),
            q_norm: floats(&ly["q_norm"]),
            k_norm: floats(&ly["k_norm"]),
            ffn_norm: floats(&ly["ffn_norm"]),
            wq: floats(&ly["wq"]),
            wk: floats(&ly["wk"]),
            wv: floats(&ly["wv"]),
            wo: floats(&ly["wo"]),
            gate: floats(&ly["gate"]),
            up: floats(&ly["up"]),
            down: floats(&ly["down"]),
        })
        .collect();
    Model {
        cfg,
        emb: floats(&w["token_embd"]),
        onorm: floats(&w["output_norm"]),
        outp: floats(&w["output"]),
        rc: floats(&j["rope"]["cos"]),
        rs: floats(&j["rope"]["sin"]),
        layers,
        prompt: j["prompt"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap() as usize)
            .collect(),
        gen_len: j["gen_len"].as_u64().unwrap() as usize,
    }
}

// ── the f64 oracle forward (clean-room, mirrors qwen.rs; Taylor exp + f64::sqrt) ────────────────────

/// 20-term Taylor `e^x`, byte-identical to `assets/std/math.lg`'s `exp` and the `.lg`'s `expT`.
fn exp_t(x: f64) -> f64 {
    let mut term = 1.0;
    let mut result = 1.0;
    let mut k = 1.0;
    while k <= 20.0 {
        term = (term * x) / k;
        result += term;
        k += 1.0;
    }
    result
}

fn silu_t(x: f64) -> f64 {
    x / (1.0 + exp_t(0.0 - x))
}

/// RMSNorm of the `dim`-vector `src[base..base+dim]` with gain `w`, appended to `dst`.
fn rmsnorm(dst: &mut Vec<f64>, src: &[f64], base: usize, w: &[f64], dim: usize, eps: f64) {
    let mut ss = 0.0;
    for i in 0..dim {
        let rv = src[base + i];
        ss += rv * rv;
    }
    let scale = 1.0 / ((ss / dim as f64) + eps).sqrt();
    for i in 0..dim {
        let rv = src[base + i];
        dst.push((rv * scale) * w[i]);
    }
}

/// `y[o] = Σ_i W[o*in_dim + i] * x[xbase + i]` (row-major `[out,in]`), appended to `dst`.
fn matvec(dst: &mut Vec<f64>, w: &[f64], out_dim: usize, in_dim: usize, x: &[f64], xbase: usize) {
    for o in 0..out_dim {
        let mut acc = 0.0;
        for i in 0..in_dim {
            acc += w[o * in_dim + i] * x[xbase + i];
        }
        dst.push(acc);
    }
}

/// QK-RMSNorm (in place) then RoPE-NEOX on every head of `seq` (a flat `n × rowstride` buffer).
fn qk_norm_rope(
    seq: &mut [f64],
    n: usize,
    nh: usize,
    rowstride: usize,
    hd: usize,
    half: usize,
    wnorm: &[f64],
    rc: &[f64],
    rs: &[f64],
    eps: f64,
) {
    for p in 0..n {
        for hh in 0..nh {
            let hb = p * rowstride + hh * hd;
            let mut ss = 0.0;
            for i in 0..hd {
                let rv = seq[hb + i];
                ss += rv * rv;
            }
            let scale = 1.0 / ((ss / hd as f64) + eps).sqrt();
            for i in 0..hd {
                let rv = seq[hb + i];
                seq[hb + i] = (rv * scale) * wnorm[i];
            }
            let rb = p * half;
            for ci in 0..half {
                let cc = rc[rb + ci];
                let sinv = rs[rb + ci];
                let x0 = seq[hb + ci];
                let x1 = seq[hb + ci + half];
                seq[hb + ci] = x0 * cc - x1 * sinv;
                seq[hb + ci + half] = x0 * sinv + x1 * cc;
            }
        }
    }
}

/// Last-position logits for `toks` — the whole f64 forward.
fn forward(m: &Model, toks: &[usize]) -> Vec<f64> {
    let c = &m.cfg;
    let (d, h, kv, hd, dff) = (c.d, c.h, c.kv, c.hd, c.dff);
    let kvd = kv * hd;
    let group = h / kv;
    let n = toks.len();
    let ascale = 1.0 / (hd as f64).sqrt();

    let mut x = Vec::with_capacity(n * d);
    for &tok in toks {
        for j in 0..d {
            x.push(m.emb[tok * d + j]);
        }
    }

    for ly in &m.layers {
        let mut xn = Vec::with_capacity(n * d);
        for p in 0..n {
            rmsnorm(&mut xn, &x, p * d, &ly.attn_norm, d, c.eps);
        }
        let mut q = Vec::with_capacity(n * d);
        let mut k = Vec::with_capacity(n * kvd);
        let mut v = Vec::with_capacity(n * kvd);
        for p in 0..n {
            matvec(&mut q, &ly.wq, d, d, &xn, p * d);
        }
        for p in 0..n {
            matvec(&mut k, &ly.wk, kvd, d, &xn, p * d);
        }
        for p in 0..n {
            matvec(&mut v, &ly.wv, kvd, d, &xn, p * d);
        }
        qk_norm_rope(&mut q, n, h, d, hd, c.half, &ly.q_norm, &m.rc, &m.rs, c.eps);
        qk_norm_rope(
            &mut k, n, kv, kvd, hd, c.half, &ly.k_norm, &m.rc, &m.rs, c.eps,
        );

        let mut attn = vec![0.0f64; n * d];
        for head in 0..h {
            let kvh = head / group;
            for qi in 0..n {
                let mut scores = Vec::with_capacity(qi + 1);
                for kj in 0..=qi {
                    let mut dd = 0.0;
                    for e in 0..hd {
                        dd += q[qi * d + head * hd + e] * k[kj * kvd + kvh * hd + e];
                    }
                    scores.push(dd * ascale);
                }
                let mut mx = scores[0];
                for si in 1..=qi {
                    if scores[si] > mx {
                        mx = scores[si];
                    }
                }
                let mut ex = Vec::with_capacity(qi + 1);
                let mut den = 0.0;
                for si in 0..=qi {
                    let ev = exp_t(scores[si] - mx);
                    ex.push(ev);
                    den += ev;
                }
                for e in 0..hd {
                    let mut acc = 0.0;
                    for si in 0..=qi {
                        acc += ex[si] * v[si * kvd + kvh * hd + e];
                    }
                    attn[qi * d + head * hd + e] = acc / den;
                }
            }
        }

        let mut ao = Vec::with_capacity(n * d);
        for p in 0..n {
            matvec(&mut ao, &ly.wo, d, d, &attn, p * d);
        }
        for i in 0..n * d {
            x[i] += ao[i];
        }

        let mut xn2 = Vec::with_capacity(n * d);
        for p in 0..n {
            rmsnorm(&mut xn2, &x, p * d, &ly.ffn_norm, d, c.eps);
        }
        let mut hid = Vec::with_capacity(n * dff);
        for p in 0..n {
            let mut gg = Vec::with_capacity(dff);
            matvec(&mut gg, &ly.gate, dff, d, &xn2, p * d);
            let mut uu = Vec::with_capacity(dff);
            matvec(&mut uu, &ly.up, dff, d, &xn2, p * d);
            for fi in 0..dff {
                hid.push(silu_t(gg[fi]) * uu[fi]);
            }
        }
        let mut dn = Vec::with_capacity(n * d);
        for p in 0..n {
            matvec(&mut dn, &ly.down, d, dff, &hid, p * dff);
        }
        for i in 0..n * d {
            x[i] += dn[i];
        }
    }

    let mut xf = Vec::with_capacity(d);
    rmsnorm(&mut xf, &x, (n - 1) * d, &m.onorm, d, c.eps);
    let mut logits = vec![0.0f64; c.vocab];
    for o in 0..c.vocab {
        let mut acc = 0.0;
        for i in 0..d {
            acc += m.outp[o * d + i] * xf[i];
        }
        logits[o] = acc;
    }
    logits
}

fn argmax(v: &[f64]) -> usize {
    let mut best = 0;
    for i in 1..v.len() {
        if v[i] > v[best] {
            best = i;
        }
    }
    best
}

// ── LOGOS-on-VM leg ─────────────────────────────────────────────────────────────────────────────────

fn python_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn gen_lg(toks: &[usize], logits: bool) -> Option<String> {
    let toks_arg = toks
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let mut cmd = Command::new("python3");
    cmd.current_dir(root())
        .args(["scripts/accel/gen_qwen_tiny_lg.py", "--toks", &toks_arg]);
    if logits {
        cmd.arg("--logits");
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        eprintln!(
            "gen_qwen_tiny_lg.py failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn vm_floats(src: &str) -> Vec<f64> {
    let out = vm_outcome(src);
    assert!(out.error.is_none(), "VM error: {:?}", out.error);
    out.output
        .split_whitespace()
        .map(|t| t.parse::<f64>().expect("float Show"))
        .collect()
}

/// LOGOS float forward ≡ the f64 oracle, BYTE-EXACT: every last-position logit matches bit-for-bit at
/// every generation step, and the greedy token sequence agrees.
#[test]
fn qwen_tiny_float_forward_matches_the_oracle_byte_exact_on_the_vm() {
    let m = load();
    if !python_available() {
        eprintln!("python3 unavailable — skipping the LOGOS-forward VM leg.");
        return;
    }
    let mut toks = m.prompt.clone();
    let mut gen = Vec::new();
    for step in 0..m.gen_len {
        let logits = forward(&m, &toks);

        let src = gen_lg(&toks, true).expect("generate LOGOS forward program");
        let got = vm_floats(&src);
        assert_eq!(got.len(), logits.len(), "step {step}: logit count mismatch");
        for (o, (&g, &r)) in got.iter().zip(&logits).enumerate() {
            assert_eq!(
                g.to_bits(),
                r.to_bits(),
                "step {step}, logit {o}: LOGOS {g} != oracle {r} (bit-exact)"
            );
        }

        let src_arg = gen_lg(&toks, false).expect("generate LOGOS argmax program");
        let got_tok = vm_floats(&src_arg);
        assert_eq!(got_tok.len(), 1, "step {step}: expected one argmax Show");
        let nxt = argmax(&logits);
        assert_eq!(
            got_tok[0] as usize, nxt,
            "step {step}: LOGOS argmax != oracle argmax"
        );

        gen.push(nxt);
        toks.push(nxt);
    }
    println!("Qwen-tiny (float) LOGOS ≡ oracle byte-exact on the VM: generated tokens {gen:?} over {} steps.", m.gen_len);
}
