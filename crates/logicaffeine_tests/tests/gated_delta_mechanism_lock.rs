//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ★ GATED DELTANET MECHANISM ★ — the Qwen3.8-27B block AUTHORED IN LOGOS as a first-class op.
//!
//! `Let O be gated delta of Q over K and V with gate Z beta B decay A state S norm GN with HV heads HK groups`
//! is linear attention with a *delta rule*. Where `attention` grows a KV cache with the sequence, this carries
//! a fixed `D×D` associative memory per value head and rewrites it in place:
//!
//! ```text
//!   u = S·k                     what the memory currently predicts for this key
//!   S = α·S + β·(v − α·u)·kᵀ    decay, then correct by the prediction error
//!   o = S·q                     read the UPDATED memory
//!   o = rmsnorm(o ⊙ silu(z))·GN gate FIRST, then normalize — the trained order
//! ```
//!
//! That constant-size state is why Qwen3.8-27B caches only 16 of its 64 layers: the other 48 are this.
//!
//! This lock is the mechanism's ground truth. Cross-tier agreement (`tensor_op_coverage_lock`) proves the
//! backends AGREE; it cannot prove they are RIGHT — every tier could share one wrong desugar. So each op here
//! is checked against an independent clean-room f64 reference written from the equations, at several shapes,
//! and the state is checked to actually CARRY between steps. If they diverge the implementation is wrong;
//! the oracle is never edited to match a buggy program.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::gdn::emit_gated_delta_forward;
use logicaffeine_compile::compile::vm_outcome;
use std::process::Command;

fn have_nvcc() -> bool {
    Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

/// A deterministic, well-spread pseudo-weight for element `i` of tensor `salt` — no RNG, fully reproducible.
fn wv(salt: u64, i: usize) -> f64 {
    let h = (i as u64)
        .wrapping_mul(2654435761)
        .wrapping_add(salt.wrapping_mul(40503));
    ((h % 97) as f64 / 97.0 - 0.5) * 0.6
}

fn silu(x: f64) -> f64 {
    x / (1.0 + std::f64::consts::E.powf(-x))
}

fn run_vm(src: &str) -> Vec<f64> {
    let out = vm_outcome(src);
    assert!(
        out.error.is_none(),
        "VM error:\n{}\n--- program ---\n{src}",
        out.error.unwrap_or_default()
    );
    out.output
        .split_whitespace()
        .map(|t| t.parse().expect("f64"))
        .collect()
}

/// Emit `Let <name> be a <r> by <c> tensor of Real.` followed by its `Set item` fill.
fn decl(s: &mut String, name: &str, r: usize, c: usize, vals: &[f64]) {
    s.push_str(&format!("Let {name} be a {r} by {c} tensor of Real.\n"));
    for (i, v) in vals.iter().enumerate() {
        s.push_str(&format!("Set item {} of {name} to {v:?}.\n", i + 1));
    }
}

fn show(s: &mut String, name: &str, n: usize) {
    for i in 0..n {
        s.push_str(&format!("Show item {} of {name}.\n", i + 1));
    }
}

// ════════════════════════════════════════ clean-room references ════════════════════════════════════

/// Gated DeltaNet, written from the equations. Mutates `st` exactly as the LOGOS op mutates its state tensor.
#[allow(clippy::too_many_arguments)]
fn gdn_ref(
    q: &[f64],
    k: &[f64],
    v: &[f64],
    z: &[f64],
    b: &[f64],
    a: &[f64],
    st: &mut [f64],
    gn: &[f64],
    hv: usize,
    hk: usize,
    d: usize,
) -> Vec<f64> {
    let mut out = vec![0.0f64; hv * d];
    for h in 0..hv {
        // keys are TILED across value heads (ggml_repeat), so the pairing is h % hk
        let (kb, vb, sb) = ((h % hk) * d, h * d, h * d * d);
        let (alpha, beta) = (a[h], b[h]);
        // u = S·k, from the memory BEFORE the update
        let u: Vec<f64> = (0..d)
            .map(|r| (0..d).map(|c| st[sb + r * d + c] * k[kb + c]).sum())
            .collect();
        // S = α·S + β·(v − α·u)·kᵀ
        for r in 0..d {
            let coef = beta * (v[vb + r] - alpha * u[r]);
            for c in 0..d {
                st[sb + r * d + c] = alpha * st[sb + r * d + c] + coef * k[kb + c];
            }
        }
        // o = S·q, from the memory AFTER the update
        // llama.cpp scales the recurrence output by 1/sqrt(D); this does NOT cancel under the gated
        // RMSNorm because eps is comparable to the (tiny) scaled variance.
        let gscale = 1.0 / (d as f64).sqrt();
        let o: Vec<f64> = (0..d)
            .map(|r| (0..d).map(|c| st[sb + r * d + c] * q[kb + c]).sum::<f64>() * gscale)
            .collect();
        // RMSNorm the head and scale by the shared weight, THEN gate by silu(z) — llama.cpp's
        // `build_norm_gated` order for this architecture (HF's Qwen3-Next gates first; Qwen3.5 does not).
        let ssq: f64 = o.iter().map(|v| v * v).sum();
        let den = (ssq / d as f64 + 1e-6).sqrt();
        for r in 0..d {
            out[vb + r] = (gn[r] * o[r] / den) * silu(z[vb + r]);
        }
    }
    out
}

/// Depthwise causal conv1d over a `taps`-deep ring, mutating the ring exactly as the LOGOS op does.
fn conv_ref(x: &[f64], wc: &[f64], cs: &mut [f64], taps: usize, chan: usize) -> Vec<f64> {
    for t in 0..taps.saturating_sub(1) {
        for c in 0..chan {
            cs[t * chan + c] = cs[(t + 1) * chan + c];
        }
    }
    cs[(taps - 1) * chan..taps * chan].copy_from_slice(&x[..chan]);
    (0..chan)
        .map(|c| (0..taps).map(|t| cs[t * chan + c] * wc[t * chan + c]).sum())
        .collect()
}

/// Bidirectional multi-head self-attention over `t` positions.
fn full_attn_ref(q: &[f64], k: &[f64], v: &[f64], h: usize, hd: usize, t: usize) -> Vec<f64> {
    let e = std::f64::consts::E;
    let width = h * hd;
    let mut out = vec![0.0f64; t * width];
    let scale = 1.0 / (hd as f64).sqrt();
    for row in 0..t {
        for head in 0..h {
            let hb = head * hd;
            let scores: Vec<f64> = (0..t)
                .map(|j| {
                    (0..hd)
                        .map(|e_| q[row * width + hb + e_] * k[j * width + hb + e_])
                        .sum::<f64>()
                        * scale
                })
                .collect();
            let mx = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let den: f64 = scores.iter().map(|&s| e.powf(s - mx)).sum();
            for e_ in 0..hd {
                out[row * width + hb + e_] = (0..t)
                    .map(|j| e.powf(scores[j] - mx) / den * v[j * width + hb + e_])
                    .sum();
            }
        }
    }
    out
}

// ════════════════════════════════════════════ the locks ════════════════════════════════════════════

/// The GDN mechanism at several `(HV, HK, D)` shapes. Different head/group/dim combinations prove the desugar
/// is size-general — and that GQA-style head sharing (`HV > HK`) maps each value head to the right key head.
#[test]
fn the_gated_delta_op_authored_in_logos_matches_the_clean_room_reference_on_the_vm() {
    for &(hv, hk, d) in &[
        (2usize, 1usize, 2usize),
        (2, 2, 3),
        (6, 2, 2),
        (4, 1, 3),
        (3, 3, 4),
    ] {
        let (qn, vn) = (hk * d, hv * d);
        let q: Vec<f64> = (0..qn).map(|i| wv(1, i)).collect();
        let k: Vec<f64> = (0..qn).map(|i| wv(2, i)).collect();
        let v: Vec<f64> = (0..vn).map(|i| wv(3, i)).collect();
        let z: Vec<f64> = (0..vn).map(|i| wv(4, i)).collect();
        // beta/decay are gate OUTPUTS in the real model, so keep them in (0,1) as sigmoid/exp would.
        let b: Vec<f64> = (0..hv).map(|i| 0.5 + wv(5, i)).collect();
        let a: Vec<f64> = (0..hv).map(|i| 0.5 + wv(6, i)).collect();
        let s0: Vec<f64> = (0..vn * d).map(|i| wv(7, i)).collect();
        let gn: Vec<f64> = (0..d).map(|i| 1.0 + wv(8, i)).collect();

        let mut src = String::from("## Main\n");
        decl(&mut src, "Q", 1, qn, &q);
        decl(&mut src, "K", 1, qn, &k);
        decl(&mut src, "V", 1, vn, &v);
        decl(&mut src, "Z", 1, vn, &z);
        decl(&mut src, "B", 1, hv, &b);
        decl(&mut src, "A", 1, hv, &a);
        decl(&mut src, "S", vn, d, &s0);
        decl(&mut src, "GN", 1, d, &gn);
        src.push_str(&format!(
            "Let O be gated delta of Q over K and V with gate Z beta B decay A state S norm GN with {hv} heads {hk} groups.\n"
        ));
        show(&mut src, "O", vn);
        // The mutated state is part of the contract — read it back too.
        show(&mut src, "S", vn * d);

        let got = run_vm(&src);
        let mut st = s0.clone();
        let want_o = gdn_ref(&q, &k, &v, &z, &b, &a, &mut st, &gn, hv, hk, d);
        assert_eq!(
            got.len(),
            vn + vn * d,
            "(HV={hv},HK={hk},D={d}) output+state length"
        );
        for (i, (g, r)) in got.iter().zip(want_o.iter().chain(st.iter())).enumerate() {
            assert!(
                (g - r).abs() < 1e-9,
                "gated delta (HV={hv},HK={hk},D={d}) slot {i}: VM {g} vs reference {r} (Δ={:e})",
                (g - r).abs()
            );
        }
    }
    println!("gated delta ≡ clean-room reference on the VM across 5 shapes (output AND the mutated memory).");
}

/// The memory must actually CARRY. Two steps in one program must equal the reference threaded through both —
/// and the second step's output must DIFFER from what a fresh memory would produce. Without this, a desugar
/// that silently ignored `S` would still pass a single-step comparison.
#[test]
fn the_gated_delta_memory_carries_between_steps() {
    let (hv, hk, d) = (2usize, 1usize, 2usize);
    let (qn, vn) = (hk * d, hv * d);
    let mk = |salt: u64, n: usize| -> Vec<f64> { (0..n).map(|i| wv(salt, i)).collect() };
    let (q1, k1, v1) = (mk(11, qn), mk(12, qn), mk(13, vn));
    let (q2, k2, v2) = (mk(21, qn), mk(22, qn), mk(23, vn));
    let z = mk(4, vn);
    let b: Vec<f64> = (0..hv).map(|i| 0.5 + wv(5, i)).collect();
    let a: Vec<f64> = (0..hv).map(|i| 0.5 + wv(6, i)).collect();
    let s0 = vec![0.0f64; vn * d];
    let gn: Vec<f64> = (0..d).map(|i| 1.0 + wv(8, i)).collect();

    let mut src = String::from("## Main\n");
    decl(&mut src, "Q1", 1, qn, &q1);
    decl(&mut src, "K1", 1, qn, &k1);
    decl(&mut src, "V1", 1, vn, &v1);
    decl(&mut src, "Q2", 1, qn, &q2);
    decl(&mut src, "K2", 1, qn, &k2);
    decl(&mut src, "V2", 1, vn, &v2);
    decl(&mut src, "Z", 1, vn, &z);
    decl(&mut src, "B", 1, hv, &b);
    decl(&mut src, "A", 1, hv, &a);
    decl(&mut src, "S", vn, d, &s0);
    decl(&mut src, "GN", 1, d, &gn);
    let op = |o: &str, q: &str, k: &str, v: &str| {
        format!("Let {o} be gated delta of {q} over {k} and {v} with gate Z beta B decay A state S norm GN with {hv} heads {hk} groups.\n")
    };
    src.push_str(&op("O1", "Q1", "K1", "V1"));
    src.push_str(&op("O2", "Q2", "K2", "V2"));
    show(&mut src, "O1", vn);
    show(&mut src, "O2", vn);

    let got = run_vm(&src);
    let mut st = s0.clone();
    let r1 = gdn_ref(&q1, &k1, &v1, &z, &b, &a, &mut st, &gn, hv, hk, d);
    let r2 = gdn_ref(&q2, &k2, &v2, &z, &b, &a, &mut st, &gn, hv, hk, d);
    for (i, (g, r)) in got.iter().zip(r1.iter().chain(r2.iter())).enumerate() {
        assert!(
            (g - r).abs() < 1e-9,
            "two-step slot {i}: VM {g} vs reference {r}"
        );
    }
    // …and the carry is OBSERVABLE: replaying step 2 against a fresh memory gives a different answer.
    let mut fresh = s0.clone();
    let r2_fresh = gdn_ref(&q2, &k2, &v2, &z, &b, &a, &mut fresh, &gn, hv, hk, d);
    let moved: f64 = r2.iter().zip(&r2_fresh).map(|(x, y)| (x - y).abs()).sum();
    assert!(
        moved > 1e-6,
        "step 2 is identical with and without step 1's memory — the state is not carrying"
    );
    println!("gated delta memory carries across steps (Σ|Δ| vs a fresh memory = {moved:e}).");
}

/// The depthwise causal conv, including its ring. Three successive steps must match the reference threaded
/// through all three — the ring shift is where an off-by-one would hide.
#[test]
fn the_causal_conv_op_matches_the_reference_and_shifts_its_ring() {
    for &(taps, chan) in &[(2usize, 3usize), (4, 2), (4, 5), (1, 3)] {
        let wc: Vec<f64> = (0..taps * chan).map(|i| wv(31, i)).collect();
        let steps: Vec<Vec<f64>> = (0..3)
            .map(|s| (0..chan).map(|i| wv(40 + s as u64, i)).collect())
            .collect();
        let cs0 = vec![0.0f64; taps * chan];

        let mut src = String::from("## Main\n");
        decl(&mut src, "WC", taps, chan, &wc);
        decl(&mut src, "CS", taps, chan, &cs0);
        for (s, xv) in steps.iter().enumerate() {
            decl(&mut src, &format!("X{s}"), 1, chan, xv);
        }
        for s in 0..steps.len() {
            src.push_str(&format!(
                "Let Y{s} be causal conv of X{s} with weight WC state CS over {taps} taps.\n"
            ));
        }
        for s in 0..steps.len() {
            show(&mut src, &format!("Y{s}"), chan);
        }
        show(&mut src, "CS", taps * chan);

        let got = run_vm(&src);
        let mut cs = cs0.clone();
        let mut want: Vec<f64> = Vec::new();
        for xv in &steps {
            want.extend(conv_ref(xv, &wc, &mut cs, taps, chan));
        }
        want.extend(cs.iter().copied());
        assert_eq!(got.len(), want.len(), "(taps={taps},chan={chan}) length");
        for (i, (g, r)) in got.iter().zip(&want).enumerate() {
            assert!(
                (g - r).abs() < 1e-9,
                "causal conv (taps={taps},chan={chan}) slot {i}: VM {g} vs reference {r}"
            );
        }
    }
    println!(
        "causal conv ≡ reference over 3 successive steps at 4 shapes (outputs AND the final ring)."
    );
}

/// Bidirectional attention — the vision tower's block. Every position must see every other, which is exactly
/// what distinguishes it from the causal `attention` op.
#[test]
fn the_full_attention_op_matches_the_reference_and_is_not_causal() {
    for &(h, hd, t) in &[(1usize, 2usize, 2usize), (2, 3, 4), (3, 2, 3)] {
        let width = h * hd;
        let q: Vec<f64> = (0..t * width).map(|i| wv(51, i)).collect();
        let k: Vec<f64> = (0..t * width).map(|i| wv(52, i)).collect();
        let v: Vec<f64> = (0..t * width).map(|i| wv(53, i)).collect();

        let mut src = String::from("## Main\n");
        decl(&mut src, "Q", t, width, &q);
        decl(&mut src, "K", t, width, &k);
        decl(&mut src, "V", t, width, &v);
        src.push_str(&format!(
            "Let C be full attention of Q over K and V with {h} heads.\n"
        ));
        show(&mut src, "C", t * width);

        let got = run_vm(&src);
        let want = full_attn_ref(&q, &k, &v, h, hd, t);
        for (i, (g, r)) in got.iter().zip(&want).enumerate() {
            assert!(
                (g - r).abs() < 1e-9,
                "full attention (H={h},HD={hd},T={t}) slot {i}: VM {g} vs reference {r}"
            );
        }

        // NOT causal: perturbing the LAST position must change the FIRST position's output. The perturbed
        // case is re-run THROUGH THE VM — comparing two reference evaluations would only prove the oracle is
        // bidirectional, which is not the claim under test.
        if t > 1 {
            let mut v2 = v.clone();
            for e in 0..width {
                v2[(t - 1) * width + e] += 1.0;
            }
            let mut src2 = String::from("## Main\n");
            decl(&mut src2, "Q", t, width, &q);
            decl(&mut src2, "K", t, width, &k);
            decl(&mut src2, "V", t, width, &v2);
            src2.push_str(&format!(
                "Let C be full attention of Q over K and V with {h} heads.\n"
            ));
            show(&mut src2, "C", t * width);
            let got2 = run_vm(&src2);
            let row0: f64 = (0..width).map(|e| (got[e] - got2[e]).abs()).sum();
            assert!(row0 > 1e-9, "(H={h},HD={hd},T={t}) the VM's row 0 ignored the last position — this is causal, not full");
            // and it still tracks the reference on the perturbed input
            for (i, (g, r)) in got2
                .iter()
                .zip(&full_attn_ref(&q, &k, &v2, h, hd, t))
                .enumerate()
            {
                assert!(
                    (g - r).abs() < 1e-9,
                    "perturbed full attention slot {i}: VM {g} vs reference {r}"
                );
            }
        }
    }
    println!("full attention ≡ reference at 3 shapes, and every position genuinely attends to every other.");
}

/// The scalar ops, each against its closed form. `l2norm` is per-ROW, which is what lets one call normalize
/// every head of a `heads by head_dim` tensor independently — GDN's use.
#[test]
fn the_scalar_ops_match_their_closed_forms() {
    let e = std::f64::consts::E;
    let xs: Vec<f64> = vec![-3.0, -0.5, 0.0, 0.25, 2.0, 7.5];
    let n = xs.len();

    let mut src = String::from("## Main\n");
    decl(&mut src, "X", 1, n, &xs);
    src.push_str("Let SG be sigmoid of X.\nLet GL be gelu of X.\n");
    show(&mut src, "SG", n);
    show(&mut src, "GL", n);
    let got = run_vm(&src);

    for (i, &x) in xs.iter().enumerate() {
        let want = 1.0 / (1.0 + e.powf(-x));
        assert!(
            (got[i] - want).abs() < 1e-12,
            "sigmoid({x}): {} vs {want}",
            got[i]
        );
    }
    for (i, &x) in xs.iter().enumerate() {
        // ggml's tanh approximation, in the numerically safe arrangement the desugar uses.
        let z = 0.797_884_560_802_865_4 * (x + 0.044_715 * x * x * x);
        let want = x / (1.0 + e.powf(-2.0 * z));
        assert!(
            (got[n + i] - want).abs() < 1e-12,
            "gelu({x}): {} vs {want}",
            got[n + i]
        );
    }

    // l2norm, per row of a 2×3 tensor: each row is divided by its own norm, not a shared one.
    let rows = vec![3.0, 4.0, 0.0, 1.0, 2.0, 2.0];
    let mut src2 = String::from("## Main\n");
    decl(&mut src2, "M", 2, 3, &rows);
    src2.push_str("Let L be l2norm of M.\n");
    show(&mut src2, "L", 6);
    let got2 = run_vm(&src2);
    for r in 0..2 {
        let den = ((0..3).map(|c| rows[r * 3 + c].powi(2)).sum::<f64>() + 1e-6).sqrt();
        for c in 0..3 {
            let want = rows[r * 3 + c] / den;
            assert!(
                (got2[r * 3 + c] - want).abs() < 1e-12,
                "l2norm row {r} col {c}: {} vs {want}",
                got2[r * 3 + c]
            );
        }
    }
    println!("sigmoid, gelu (ggml tanh form) and per-row l2norm ≡ their closed forms.");
}

/// PARTIAL rotary: Qwen3.8 rotates 64 of each head's 256 dims and passes the rest through untouched. The
/// pass-through half is the part a naive full-rotary implementation would silently corrupt.
#[test]
fn partial_rope_rotates_only_the_leading_slice_and_passes_the_rest_through() {
    let (heads, hd, half) = (2usize, 4usize, 1usize); // rotary width 2 of head_dim 4
    let n = heads * hd;
    let q: Vec<f64> = (0..n).map(|i| wv(61, i) + 1.0).collect();
    let (cos, sin) = (0.6f64, 0.8f64);

    let mut src = String::from("## Main\n");
    decl(&mut src, "Q", 1, n, &q);
    decl(&mut src, "RC", 1, half, &[cos]);
    decl(&mut src, "RS", 1, half, &[sin]);
    src.push_str(&format!(
        "Let Y be rope of Q with cos RC and sin RS across {heads} heads.\n"
    ));
    show(&mut src, "Y", n);
    let got = run_vm(&src);

    for h in 0..heads {
        let b = h * hd;
        // the rotated pair
        assert!(
            (got[b] - (q[b] * cos - q[b + half] * sin)).abs() < 1e-12,
            "head {h} rotated lo"
        );
        assert!(
            (got[b + half] - (q[b] * sin + q[b + half] * cos)).abs() < 1e-12,
            "head {h} rotated hi"
        );
        // …and everything past the slice is carried through EXACTLY
        for i in (2 * half)..hd {
            assert!(
                (got[b + i] - q[b + i]).abs() < 1e-15,
                "head {h} dim {i} was modified but is outside the rotary slice"
            );
        }
    }
    println!("partial rope rotates the leading {} dims of each {hd}-wide head and passes the rest through byte-exact.", 2 * half);
}

/// The FUSED CUDA kernels must equal the reference. This is the throughput lane's correctness gate:
/// `gdn_step` rewrites the recurrence so a single pass over the memory yields `S·k`, `S·q` and `k·q` at once,
/// via the identity `o = α·(S_old·q) + coef·(k·q)`. That is exact algebra, not an approximation, so the GPU
/// must agree to f32 tolerance — including the MUTATED memory, where a wrong fusion shows up first.
///
/// The oracle here is `gdn_ref`, not the VM directly, and deliberately so: at head_dim 128 the memory holds
/// 131 072 floats, and asking a tree-walking interpreter to update and print all of them is not a test, it is
/// a hang. `gdn_ref` is pinned ≡ the VM by
/// `the_gated_delta_op_authored_in_logos_matches_the_clean_room_reference_on_the_vm` at five shapes, so the
/// chain GPU ≡ reference ≡ VM is closed — and one small shape below is still compared against the VM directly
/// so the two legs cannot drift apart silently.
///
/// Skips cleanly without nvcc or a GPU, so CI stays green off-device.
#[test]
fn the_fused_gated_delta_cuda_kernels_match_the_reference() {
    if !have_nvcc() || !have_gpu() {
        eprintln!("SKIP the_fused_gated_delta_cuda_kernels_match_the_reference — needs nvcc + /dev/nvidia0");
        return;
    }
    // Head sharing (HV > HK), no sharing (HV == HK), and the real model's head_dim of 128.
    for &(hv, hk, d) in &[
        (2usize, 1usize, 32usize),
        (6, 2, 64),
        (4, 4, 32),
        (8, 2, 128),
        (48, 16, 128),
    ] {
        let (qn, vn) = (hk * d, hv * d);
        let q: Vec<f64> = (0..qn).map(|i| wv(101, i)).collect();
        let k: Vec<f64> = (0..qn).map(|i| wv(102, i)).collect();
        let v: Vec<f64> = (0..vn).map(|i| wv(103, i)).collect();
        let z: Vec<f64> = (0..vn).map(|i| wv(104, i)).collect();
        let b: Vec<f64> = (0..hv).map(|i| 0.5 + wv(105, i)).collect();
        let a: Vec<f64> = (0..hv).map(|i| 0.5 + wv(106, i)).collect();
        let s0: Vec<f64> = (0..vn * d).map(|i| wv(107, i)).collect();
        let gn: Vec<f64> = (0..d).map(|i| 1.0 + wv(108, i)).collect();

        let mut st = s0.clone();
        let want_o = gdn_ref(&q, &k, &v, &z, &b, &a, &mut st, &gn, hv, hk, d);

        let cu = emit_gated_delta_forward(&q, &k, &v, &z, &b, &a, &s0, &gn, hv, hk, d);
        let dir =
            std::env::temp_dir().join(format!("logos_gdn_{}_{hv}_{hk}_{d}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmpdir");
        let (src_p, bin_p) = (dir.join("g.cu"), dir.join("g"));
        std::fs::write(&src_p, &cu).expect("write .cu");
        let sm = std::env::var("LOGOS_SM").unwrap_or_else(|_| "sm_86".to_string());
        let comp = Command::new("nvcc")
            .args(["-O3", &format!("-arch={sm}")])
            .arg(&src_p)
            .arg("-o")
            .arg(&bin_p)
            .output()
            .expect("run nvcc");
        assert!(
            comp.status.success(),
            "nvcc failed at (HV={hv},HK={hk},D={d}):\n{}",
            String::from_utf8_lossy(&comp.stderr)
        );
        let run = Command::new(&bin_p).output().expect("run gdn binary");
        assert!(
            run.status.success(),
            "gdn binary failed:\n{}",
            String::from_utf8_lossy(&run.stderr)
        );
        let got: Vec<f64> = String::from_utf8_lossy(&run.stdout)
            .split_whitespace()
            .map(|t| t.parse().expect("f32 value"))
            .collect();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(
            got.len(),
            vn + vn * d,
            "(HV={hv},HK={hk},D={d}) output+state length"
        );
        let mut worst = 0.0f64;
        for (i, (g, r)) in got.iter().zip(want_o.iter().chain(st.iter())).enumerate() {
            let delta = (g - r).abs();
            worst = worst.max(delta / r.abs().max(1.0));
            assert!(
                delta < 1e-4 * r.abs().max(1.0),
                "fused GDN CUDA (HV={hv},HK={hk},D={d}) slot {i} ({}): GPU {g} vs reference {r} (Δ={delta:e})",
                if i < vn { "output" } else { "state" },
            );
        }
        println!("  fused GDN CUDA ≡ reference at HV={hv} HK={hk} D={d} — {} values, worst rel Δ {worst:.2e}", got.len());
    }
    println!("the one-pass GDN fusion (o = α·(S·q) + coef·(k·q)) is exact on the GPU, up to the real model's 48/16/128.");
}

/// The GPU leg's oracle is `gdn_ref`, so this pins `gdn_ref` ≡ the VM ≡ the GPU at ONE small shape where the
/// interpreter can actually run — closing the chain without asking it to print 131 072 state values.
#[test]
fn the_gpu_reference_and_the_vm_agree_at_a_shape_all_three_can_run() {
    if !have_nvcc() || !have_gpu() {
        eprintln!("SKIP the_gpu_reference_and_the_vm_agree_at_a_shape_all_three_can_run — needs nvcc + GPU");
        return;
    }
    let (hv, hk, d) = (2usize, 1usize, 4usize);
    let (qn, vn) = (hk * d, hv * d);
    let q: Vec<f64> = (0..qn).map(|i| wv(201, i)).collect();
    let k: Vec<f64> = (0..qn).map(|i| wv(202, i)).collect();
    let v: Vec<f64> = (0..vn).map(|i| wv(203, i)).collect();
    let z: Vec<f64> = (0..vn).map(|i| wv(204, i)).collect();
    let b: Vec<f64> = (0..hv).map(|i| 0.5 + wv(205, i)).collect();
    let a: Vec<f64> = (0..hv).map(|i| 0.5 + wv(206, i)).collect();
    let s0: Vec<f64> = (0..vn * d).map(|i| wv(207, i)).collect();
    let gn: Vec<f64> = (0..d).map(|i| 1.0 + wv(208, i)).collect();

    let mut src = String::from("## Main\n");
    decl(&mut src, "Q", 1, qn, &q);
    decl(&mut src, "K", 1, qn, &k);
    decl(&mut src, "V", 1, vn, &v);
    decl(&mut src, "Z", 1, vn, &z);
    decl(&mut src, "B", 1, hv, &b);
    decl(&mut src, "A", 1, hv, &a);
    decl(&mut src, "S", vn, d, &s0);
    decl(&mut src, "GN", 1, d, &gn);
    src.push_str(&format!(
        "Let O be gated delta of Q over K and V with gate Z beta B decay A state S norm GN with {hv} heads {hk} groups.\n"
    ));
    show(&mut src, "O", vn);
    show(&mut src, "S", vn * d);
    let vm = run_vm(&src);

    let cu = emit_gated_delta_forward(&q, &k, &v, &z, &b, &a, &s0, &gn, hv, hk, d);
    let dir = std::env::temp_dir().join(format!("logos_gdn_tri_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let (src_p, bin_p) = (dir.join("g.cu"), dir.join("g"));
    std::fs::write(&src_p, &cu).expect("write .cu");
    let sm = std::env::var("LOGOS_SM").unwrap_or_else(|_| "sm_86".to_string());
    let comp = Command::new("nvcc")
        .args(["-O3", &format!("-arch={sm}")])
        .arg(&src_p)
        .arg("-o")
        .arg(&bin_p)
        .output()
        .expect("nvcc");
    assert!(
        comp.status.success(),
        "nvcc failed:\n{}",
        String::from_utf8_lossy(&comp.stderr)
    );
    let run = Command::new(&bin_p).output().expect("run");
    let gpu: Vec<f64> = String::from_utf8_lossy(&run.stdout)
        .split_whitespace()
        .map(|t| t.parse().expect("f32"))
        .collect();
    let _ = std::fs::remove_dir_all(&dir);

    let mut st = s0.clone();
    let refo = gdn_ref(&q, &k, &v, &z, &b, &a, &mut st, &gn, hv, hk, d);
    let reference: Vec<f64> = refo.iter().chain(st.iter()).copied().collect();

    assert_eq!(vm.len(), reference.len());
    assert_eq!(gpu.len(), reference.len());
    for i in 0..reference.len() {
        assert!(
            (vm[i] - reference[i]).abs() < 1e-9,
            "VM vs reference slot {i}: {} vs {}",
            vm[i],
            reference[i]
        );
        assert!(
            (gpu[i] - reference[i]).abs() < 1e-4 * reference[i].abs().max(1.0),
            "GPU vs reference slot {i}: {} vs {}",
            gpu[i],
            reference[i]
        );
    }
    println!("VM ≡ reference ≡ GPU at HV={hv} HK={hk} D={d} — the oracle chain is closed.");
}
