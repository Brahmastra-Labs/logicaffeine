//! Unpack the real Qwen3.8-27B Q8_0 checkpoint into the per-tensor files the emitted forward `fopen`s.
//!
//! Usage: extract_qwen38 <gguf> <out_dir>
//!
//! Two things happen here that are not a plain copy, and both exist to make the decode step FAST:
//!
//!  * **Splitting.** The checkpoint fuses Q with its output gate (`attn_q` is `[d, 2·aq]`) and q|k|v into one
//!    tensor (`attn_qkv` is `[d, 10240]`). The `.lg` reads them as ordinary separate projections, so they are
//!    split here.
//!  * **Concatenating.** Projections that share an activation are then written as ONE matrix, so four GEMVs
//!    become one perfectly-coalesced launch with no per-row branching: `qkvz` = q|k|v|z, `qgkv` = q|gate|k|v,
//!    `gu` = ffn_gate|ffn_up. A Q8_0 matrix is `out` rows of `in` values (plus `out·in/32` scales), so
//!    concatenating along `out` is just appending — the layout is already right.
//!
//! The conv weight also gets TRANSPOSED: the GGUF stores `ssm_conv1d` as `[taps, channels]` with taps
//! contiguous (i.e. channel-major), and the kernel wants tap-major so a thread owning a channel strides by C.

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::codegen_cuda::qwen38::Qwen38Cfg;
use std::path::Path;

fn write_i8(p: &Path, v: &[i8]) {
    std::fs::write(p, v.iter().map(|&q| q as u8).collect::<Vec<u8>>())
        .unwrap_or_else(|e| panic!("{p:?}: {e}"));
}
fn write_f32(p: &Path, v: &[f32]) {
    let mut b = Vec::with_capacity(v.len() * 4);
    for x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
    std::fs::write(p, b).unwrap_or_else(|e| panic!("{p:?}: {e}"));
}

/// Append rows `[from, to)` of a Q8_0 matrix (`out` rows of `inn`) onto the running concatenation.
fn take_rows(
    q: &[i8],
    s: &[f32],
    inn: usize,
    from: usize,
    to: usize,
    dq: &mut Vec<i8>,
    ds: &mut Vec<f32>,
) {
    let nb = inn / 32;
    dq.extend_from_slice(&q[from * inn..to * inn]);
    ds.extend_from_slice(&s[from * nb..to * nb]);
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: extract_qwen38 <gguf> <out_dir>");
        std::process::exit(2);
    }
    let (gguf, dir) = (&a[1], Path::new(&a[2]));
    std::fs::create_dir_all(dir).unwrap();
    let mut g = Gguf::open(Path::new(gguf)).unwrap_or_else(|e| panic!("open {gguf}: {e}"));

    // Drive every dimension from the checkpoint; only the layer schedule is structural.
    let cfg = Qwen38Cfg {
        d: g.arch.embedding_length as usize,
        layers: g.arch.model_layers() as usize,
        full_attn_every: g.arch.full_attention_interval as usize,
        dff: g.arch.feed_forward_length as usize,
        vocab: g.dims2("output.weight").expect("output dims").1,
        h: g.arch.head_count as usize,
        kv: g.arch.head_count_kv as usize,
        hd: g.arch.head_dim as usize,
        rope_dim: g.arch.rope_dimension_count as usize,
        rope_theta: g.arch.rope_freq_base as f64,
        rms_eps: g.arch.rms_eps as f64,
        gdn_k_heads: g.arch.ssm_group_count as usize,
        gdn_v_heads: g.arch.ssm_time_step_rank as usize,
        gdn_head_dim: g.arch.ssm_state_size as usize,
        conv_taps: g.arch.ssm_conv_kernel as usize,
        ctx_len: g.arch.context_length as usize,
    };
    eprintln!(
        "arch {} · d={} layers={} (attn every {}) dff={} vocab={} · attn {}/{}x{} rope {}of{} @{:.0} · gdn {}k/{}v x{} conv{}",
        g.arch.arch, cfg.d, cfg.layers, cfg.full_attn_every, cfg.dff, cfg.vocab,
        cfg.h, cfg.kv, cfg.hd, cfg.rope_dim, cfg.hd, cfg.rope_theta,
        cfg.gdn_k_heads, cfg.gdn_v_heads, cfg.gdn_head_dim, cfg.conv_taps
    );

    let (qk, vw, akv) = (cfg.gdn_qk(), cfg.gdn_v(), cfg.akv());
    let q8 = |g: &mut Gguf, name: &str| {
        g.q8_0_matrix(name)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    };

    // ── global ──
    {
        let (q, s) = q8(&mut g, "token_embd.weight");
        write_i8(&dir.join("emb.q"), &q);
        write_f32(&dir.join("emb.s"), &s);
        let (q, s) = q8(&mut g, "output.weight");
        write_i8(&dir.join("Wout.q"), &q);
        write_f32(&dir.join("Wout.s"), &s);
        write_f32(
            &dir.join("on.f"),
            &g.f32_tensor("output_norm.weight").unwrap(),
        );
    }

    for l in 0..cfg.layers {
        write_f32(
            &dir.join(format!("an{l}.f")),
            &g.f32_tensor(&format!("blk.{l}.attn_norm.weight")).unwrap(),
        );
        write_f32(
            &dir.join(format!("fn{l}.f")),
            &g.f32_tensor(&format!("blk.{l}.post_attention_norm.weight"))
                .unwrap(),
        );

        if cfg.is_attn(l) {
            // q | gate | k | v — one [d, 2·aq + 2·akv] matrix, one GEMV launch.
            let (qg, qgs) = q8(&mut g, &format!("blk.{l}.attn_q.weight"));
            let (kq, ks) = q8(&mut g, &format!("blk.{l}.attn_k.weight"));
            let (vq, vs) = q8(&mut g, &format!("blk.{l}.attn_v.weight"));
            let (mut cq, mut cs) = (Vec::new(), Vec::new());
            // Q and its gate are INTERLEAVED PER HEAD in the checkpoint — `[q_h0, gate_h0, q_h1, gate_h1, …]`
            // with stride 2·head_dim (llama.cpp reads them as two strided views of one tensor). De-interleave
            // into contiguous q ++ gate here so the emitted kernels keep flat offsets.
            for half in 0..2 {
                for hh in 0..cfg.h {
                    let base = hh * 2 * cfg.hd + half * cfg.hd;
                    take_rows(&qg, &qgs, cfg.d, base, base + cfg.hd, &mut cq, &mut cs);
                }
            }
            take_rows(&kq, &ks, cfg.d, 0, akv, &mut cq, &mut cs);
            take_rows(&vq, &vs, cfg.d, 0, akv, &mut cq, &mut cs);
            write_i8(&dir.join(format!("qgkv{l}.q")), &cq);
            write_f32(&dir.join(format!("qgkv{l}.s")), &cs);
            write_f32(
                &dir.join(format!("Qn{l}.f")),
                &g.f32_tensor(&format!("blk.{l}.attn_q_norm.weight"))
                    .unwrap(),
            );
            write_f32(
                &dir.join(format!("Kn{l}.f")),
                &g.f32_tensor(&format!("blk.{l}.attn_k_norm.weight"))
                    .unwrap(),
            );
            let (q, s) = q8(&mut g, &format!("blk.{l}.attn_output.weight"));
            write_i8(&dir.join(format!("Wo{l}.q")), &q);
            write_f32(&dir.join(format!("Wo{l}.s")), &s);
        } else {
            // q | k | v | z — the fused checkpoint tensor already holds q|k|v adjacently; append the gate.
            let (qkv, qkvs) = q8(&mut g, &format!("blk.{l}.attn_qkv.weight"));
            let (zq, zs) = q8(&mut g, &format!("blk.{l}.attn_gate.weight"));
            let (mut cq, mut cs) = (Vec::new(), Vec::new());
            take_rows(&qkv, &qkvs, cfg.d, 0, 2 * qk + vw, &mut cq, &mut cs);
            take_rows(&zq, &zs, cfg.d, 0, vw, &mut cq, &mut cs);
            write_i8(&dir.join(format!("qkvz{l}.q")), &cq);
            write_f32(&dir.join(format!("qkvz{l}.s")), &cs);

            // beta | alpha — small f32 projections, concatenated into one [d, 2·NV].
            let mut ba = g.f32_tensor(&format!("blk.{l}.ssm_beta.weight")).unwrap();
            ba.extend_from_slice(&g.f32_tensor(&format!("blk.{l}.ssm_alpha.weight")).unwrap());
            write_f32(&dir.join(format!("ba{l}.f")), &ba);

            // conv weight: GGUF stores [taps, channels] with taps contiguous; the kernel wants tap-major.
            let cw = g.f32_tensor(&format!("blk.{l}.ssm_conv1d.weight")).unwrap();
            let (t, c) = (cfg.conv_taps, 2 * qk + vw);
            assert_eq!(cw.len(), t * c, "conv weight {l}: {} != {t}x{c}", cw.len());
            let mut tw = vec![0f32; t * c];
            for ch in 0..c {
                for tap in 0..t {
                    tw[tap * c + ch] = cw[ch * t + tap];
                }
            }
            write_f32(&dir.join(format!("Wc{l}.f")), &tw);

            write_f32(
                &dir.join(format!("Al{l}.f")),
                &g.f32_tensor(&format!("blk.{l}.ssm_a")).unwrap(),
            );
            write_f32(
                &dir.join(format!("Dt{l}.f")),
                &g.f32_tensor(&format!("blk.{l}.ssm_dt.bias")).unwrap(),
            );
            write_f32(
                &dir.join(format!("Gn{l}.f")),
                &g.f32_tensor(&format!("blk.{l}.ssm_norm.weight")).unwrap(),
            );
            let (q, s) = q8(&mut g, &format!("blk.{l}.ssm_out.weight"));
            write_i8(&dir.join(format!("Wo{l}.q")), &q);
            write_f32(&dir.join(format!("Wo{l}.s")), &s);
        }

        // ffn_gate | ffn_up — one [d, 2·dff] matrix, one launch; then the down projection.
        let (gq, gs) = q8(&mut g, &format!("blk.{l}.ffn_gate.weight"));
        let (uq, us) = q8(&mut g, &format!("blk.{l}.ffn_up.weight"));
        let (mut cq, mut cs) = (Vec::new(), Vec::new());
        take_rows(&gq, &gs, cfg.d, 0, cfg.dff, &mut cq, &mut cs);
        take_rows(&uq, &us, cfg.d, 0, cfg.dff, &mut cq, &mut cs);
        write_i8(&dir.join(format!("gu{l}.q")), &cq);
        write_f32(&dir.join(format!("gu{l}.s")), &cs);
        let (q, s) = q8(&mut g, &format!("blk.{l}.ffn_down.weight"));
        write_i8(&dir.join(format!("Wd{l}.q")), &q);
        write_f32(&dir.join(format!("Wd{l}.s")), &s);

        if l % 8 == 0 {
            eprintln!(
                "  layer {l}/{} ({})",
                cfg.layers,
                if cfg.is_attn(l) { "attn" } else { "gdn" }
            );
        }
    }

    // ── the MTP (multi-token prediction) block, blk.64 ──────────────────────────────────────────
    // `nextn_predict_layers = 1`, and the tensor census settles what it is: the count-17 attention patterns
    // cover 16 real attention layers PLUS this block, and the count-65 FFN patterns cover it too. So blk.64
    // is a full GATED-ATTENTION layer + FFN, fronted by the `nextn` adapter that mixes the last hidden state
    // with the embedding of the token just produced:
    //     x = eh_proj · concat( rmsnorm(embed(t), enorm), rmsnorm(h, hnorm) )
    // then the block runs normally and reuses the model's own output head. Note `is_attn(64)` is FALSE under
    // the interval rule — the MTP block is attention regardless, so it is extracted explicitly rather than
    // by falling out of the layer loop.
    if std::env::var("LOGOS_Q38_MTP").is_ok() {
        let l = cfg.layers; // 64
        eprintln!("  MTP block blk.{l} (gated attention + FFN + nextn adapter)");
        let (qg, qgs) = q8(&mut g, &format!("blk.{l}.attn_q.weight"));
        let (kq, ks) = q8(&mut g, &format!("blk.{l}.attn_k.weight"));
        let (vq, vs) = q8(&mut g, &format!("blk.{l}.attn_v.weight"));
        let (mut cq, mut cs) = (Vec::new(), Vec::new());
        for half in 0..2 {
            for hh in 0..cfg.h {
                let base = hh * 2 * cfg.hd + half * cfg.hd;
                take_rows(&qg, &qgs, cfg.d, base, base + cfg.hd, &mut cq, &mut cs);
            }
        }
        take_rows(&kq, &ks, cfg.d, 0, akv, &mut cq, &mut cs);
        take_rows(&vq, &vs, cfg.d, 0, akv, &mut cq, &mut cs);
        write_i8(&dir.join(format!("qgkv{l}.q")), &cq);
        write_f32(&dir.join(format!("qgkv{l}.s")), &cs);

        let (oq, os) = q8(&mut g, &format!("blk.{l}.attn_output.weight"));
        write_i8(&dir.join(format!("Wo{l}.q")), &oq);
        write_f32(&dir.join(format!("Wo{l}.s")), &os);

        // gate ++ up, the same concatenation the ordinary layers use
        let (gq, gs_) = q8(&mut g, &format!("blk.{l}.ffn_gate.weight"));
        let (uq, us) = q8(&mut g, &format!("blk.{l}.ffn_up.weight"));
        let (mut fq, mut fs) = (gq.clone(), gs_.clone());
        fq.extend_from_slice(&uq);
        fs.extend_from_slice(&us);
        write_i8(&dir.join(format!("gu{l}.q")), &fq);
        write_f32(&dir.join(format!("gu{l}.s")), &fs);
        let (dq, ds) = q8(&mut g, &format!("blk.{l}.ffn_down.weight"));
        write_i8(&dir.join(format!("Wd{l}.q")), &dq);
        write_f32(&dir.join(format!("Wd{l}.s")), &ds);

        for (name, file) in [
            ("attn_norm", format!("an{l}")),
            ("post_attention_norm", format!("fn{l}")),
            ("attn_q_norm", format!("Qn{l}")),
            ("attn_k_norm", format!("Kn{l}")),
        ] {
            let v = g.f32_tensor(&format!("blk.{l}.{name}.weight")).expect(name);
            write_f32(&dir.join(format!("{file}.f")), &v);
        }
        // the nextn adapter itself
        let (eh, ehs) = q8(&mut g, &format!("blk.{l}.nextn.eh_proj.weight"));
        write_i8(&dir.join("ehproj.q"), &eh);
        write_f32(&dir.join("ehproj.s"), &ehs);
        for (name, file) in [
            ("enorm", "enorm"),
            ("hnorm", "hnorm"),
            ("shared_head_norm", "shnorm"),
        ] {
            let v = g
                .f32_tensor(&format!("blk.{l}.nextn.{name}.weight"))
                .expect(name);
            write_f32(&dir.join(format!("{file}.f")), &v);
        }
        eprintln!("  MTP block extracted");
    }

    eprintln!("extracted {} layers to {}", cfg.layers, dir.display());
}
