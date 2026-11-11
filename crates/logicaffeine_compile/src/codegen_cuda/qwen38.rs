//! Qwen3.8-27B authored in LOGOS.
//!
//! The model is a **hybrid**: of its 64 layers, 48 are Gated DeltaNet (a fixed-size associative memory,
//! rewritten per token) and 16 are gated attention (a growing KV cache). `full_attention_interval = 4` puts
//! the attention layers at indices 3, 7, 11 … 63. That 3:1 ratio is why the model caches only a quarter of
//! its depth, and it is what makes the checkpoint cheap to run relative to its parameter count.
//!
//! [`gen_qwen38_lg`] emits the whole forward as a `.lg` program — the artifact the claim "authored in LOGOS"
//! rests on. Every line is ordinary LOGOS tensor sugar; nothing here is a compiler intrinsic. Following the
//! Qwen3-30B precedent the `.lg` is the SINGLE-TOKEN forward (KV caches declared one row deep), and the
//! generation harness is what grows the cache and replays the step.

use crate::model::author::{self, Authored, RowState, TpAuthored};

/// Everything the emitter needs, read from the checkpoint rather than assumed. Field names mirror the GGUF
/// metadata keys so a mismatch is easy to spot.
#[derive(Debug, Clone)]
pub struct Qwen38Cfg {
    pub d: usize,               // embedding_length            5120
    pub layers: usize,          // model layers (block_count − nextn_predict_layers)  64
    pub full_attn_every: usize, // full_attention_interval     4
    pub dff: usize,             // feed_forward_length         17408
    pub vocab: usize,           // 248320, untied
    // gated attention
    pub h: usize,        // attention.head_count        24
    pub kv: usize,       // attention.head_count_kv     4
    pub hd: usize,       // attention.key_length        256
    pub rope_dim: usize, // rope.dimension_count        64  (PARTIAL: 64 of 256)
    pub rope_theta: f64, // rope.freq_base              1e7
    pub rms_eps: f64,    // 1e-6
    // gated deltanet
    pub gdn_k_heads: usize,  // ssm.group_count             16
    pub gdn_v_heads: usize,  // ssm.time_step_rank          48
    pub gdn_head_dim: usize, // ssm.state_size              128
    pub conv_taps: usize,    // ssm.conv_kernel             4
    pub ctx_len: usize,      // context_length              262144
}

impl Qwen38Cfg {
    /// The real Qwen3.8-27B shape, as read from `Qwen3.8-27B-Q8_0.gguf` (see `benchmarks/results/qwen38_arch.json`).
    pub fn real() -> Self {
        Qwen38Cfg {
            d: 5120,
            layers: 64,
            full_attn_every: 4,
            dff: 17408,
            vocab: 248320,
            h: 24,
            kv: 4,
            hd: 256,
            rope_dim: 64,
            rope_theta: 1.0e7,
            rms_eps: 1e-6,
            gdn_k_heads: 16,
            gdn_v_heads: 48,
            gdn_head_dim: 128,
            conv_taps: 4,
            ctx_len: 262144,
        }
    }
    /// A structurally faithful miniature — same op sequence, same hybrid schedule, tiny dimensions. Used to
    /// exercise the whole authored program on every backend without 29 GB of weights.
    pub fn tiny() -> Self {
        Qwen38Cfg {
            d: 8,
            layers: 4,
            full_attn_every: 4,
            dff: 16,
            vocab: 32,
            h: 2,
            kv: 1,
            hd: 4,
            rope_dim: 2,
            rope_theta: 1.0e7,
            rms_eps: 1e-6,
            gdn_k_heads: 1,
            gdn_v_heads: 2,
            gdn_head_dim: 4,
            conv_taps: 4,
            ctx_len: 64,
        }
    }
    /// [`Qwen38Cfg::tiny`] with the real model's HEAD PATTERN at a size that splits: 8 query heads over 2 KV heads and
    /// 4/8 Gated-DeltaNet heads, so it admits 2 and 4 chips of tensor parallelism — at 4 each KV head is replicated on
    /// two chips, as the real model's 4 KV heads are over its 8.
    pub fn mini() -> Self {
        Qwen38Cfg {
            h: 8,
            kv: 2,
            gdn_k_heads: 4,
            gdn_v_heads: 8,
            ..Qwen38Cfg::tiny()
        }
    }
    /// GDN q/k width — `k_heads · head_dim`.
    pub fn gdn_qk(&self) -> usize {
        self.gdn_k_heads * self.gdn_head_dim
    }
    /// GDN v/gate width — `v_heads · head_dim` (`ssm.inner_size`).
    pub fn gdn_v(&self) -> usize {
        self.gdn_v_heads * self.gdn_head_dim
    }
    /// Attention query width — `h · hd`.
    pub fn aq(&self) -> usize {
        self.h * self.hd
    }
    /// Attention K/V width — `kv · hd`.
    pub fn akv(&self) -> usize {
        self.kv * self.hd
    }
    /// Layer `l` is full attention when `(l+1) % full_attn_every == 0` — indices 3, 7, 11 … for interval 4.
    /// The configuration of a checkpoint's own spec: a Qwen3.5/3.8 hybrid, its attention every `full_attn_every`-th
    /// layer, every attention layer alike, every Gated-DeltaNet layer alike, every FFN a dense SwiGLU. Refused by name
    /// for anything else — and refused unless the configuration writes the checkpoint's own program, the decode step
    /// the author writes for `spec` (`model::program`), so no bundle lowered from it can be another model's.
    pub fn from_spec(spec: &crate::model::spec::ModelSpec) -> Result<Qwen38Cfg, author::NotYet> {
        use crate::model::spec::{Act, Ffn, Mixer, RopeScaling};
        use author::NotYet;
        if spec.arch != "qwen35" {
            return Err(NotYet("a model other than a Qwen3.5/3.8 hybrid, which the spatial lowering is written for"));
        }
        let every = spec
            .layers
            .iter()
            .position(|l| matches!(l.mixer, Mixer::Attention(_)))
            .map(|l| l + 1)
            .ok_or(NotYet("a hybrid with no attention layer"))?;
        let attention = spec.layers.iter().find_map(|l| match &l.mixer {
            Mixer::Attention(a) => Some(a.clone()),
            _ => None,
        });
        let delta = spec.layers.iter().find_map(|l| match &l.mixer {
            Mixer::GatedDelta(g) => Some(g.clone()),
            _ => None,
        });
        let (Some(a), Some(g)) = (attention, delta) else {
            return Err(NotYet("a hybrid without both attention and Gated-DeltaNet layers"));
        };
        let rope = a.rope.as_ref().ok_or(NotYet("attention without a rotation"))?;
        if !rope.neox || rope.scaling != RopeScaling::None {
            return Err(NotYet("a rotation other than an unscaled NEOX one"));
        }
        let dff = match &spec.layers[0].ffn {
            Ffn::Dense { dff, act: Act::Silu } => *dff,
            _ => return Err(NotYet("a feed-forward block other than a dense SwiGLU")),
        };
        for (l, layer) in spec.layers.iter().enumerate() {
            let alike = match &layer.mixer {
                Mixer::Attention(x) => (l + 1) % every == 0 && *x == a,
                Mixer::GatedDelta(x) => (l + 1) % every != 0 && *x == g,
                Mixer::Mla(_) | Mixer::Recurrent(_) | Mixer::None => false,
            };
            if !alike || layer.ffn != (Ffn::Dense { dff, act: Act::Silu }) || !layer.pre_norms || layer.post_mixer_norm || layer.post_ffn_norm {
                return Err(NotYet("a layer unlike the hybrid's pattern of alike attention and Gated-DeltaNet layers"));
            }
        }
        let cfg = Qwen38Cfg {
            d: spec.d,
            layers: spec.layers.len(),
            full_attn_every: every,
            dff,
            vocab: spec.vocab,
            h: a.heads,
            kv: a.kv_heads,
            hd: a.head_dim,
            rope_dim: rope.dims,
            rope_theta: rope.theta,
            rms_eps: spec.rms_eps,
            gdn_k_heads: g.k_heads,
            gdn_v_heads: g.v_heads,
            gdn_head_dim: g.head_dim,
            conv_taps: g.conv_taps,
            ctx_len: spec.ctx_len,
        };
        let step = crate::model::program::Cut::Decode { ctx: 64 };
        if crate::model::program::written(&cfg.spec(), step).1 != crate::model::program::written(spec, step).1 {
            return Err(NotYet("a hybrid whose configuration would write a program other than the checkpoint's"));
        }
        Ok(cfg)
    }

    pub fn is_attn(&self, l: usize) -> bool {
        (l + 1) % self.full_attn_every == 0
    }
    /// How many of the layers are attention (the rest are GDN).
    pub fn attn_layers(&self) -> usize {
        (0..self.layers).filter(|&l| self.is_attn(l)).count()
    }

    /// The model this configuration describes, as a `ModelSpec`: every `full_attn_every`-th layer gated attention
    /// (partial NEOX rotation, QK-norm, a sigmoid output gate), the rest Gated DeltaNet, each with a dense SiLU FFN.
    pub fn spec(&self) -> crate::model::spec::ModelSpec {
        self.spec_of(self.layers)
    }

    /// The spec of this configuration reaching layer `l`: its layers, or as many more of its pattern as layer `l`
    /// needs — the model a single-layer form of layer `l` is written from.
    pub fn layer_spec(&self, l: usize) -> crate::model::spec::ModelSpec {
        self.spec_of(self.layers.max(l + 1))
    }

    /// The spec of this configuration's first `layers` layers.
    fn spec_of(&self, layers: usize) -> crate::model::spec::ModelSpec {
        use crate::model::spec::{
            Act, Attention, Ffn, GateActivation, GatedDelta, Layer, Mixer, ModelSpec, Rope, RopeScaling,
        };
        let attention = Attention {
            heads: self.h,
            kv_heads: self.kv,
            head_dim: self.hd,
            value_dim: self.hd,
            rope: Some(Rope {
                dims: self.rope_dim,
                theta: self.rope_theta,
                neox: true,
                scaling: RopeScaling::None,
                sections: None,
            }),
            qk_norm: true,
            output_gate: true,
            fused_qkv: false,
            bias: false,
            output_bias: false,
            window: None,
            sinks: false,
            softcap: None,
            scale: 1.0 / (self.hd as f64).sqrt(),
            temperature: None,
            qk_unit_norm: false,
            qk_norm_whole: false,
        };
        let gdn = GatedDelta {
            k_heads: self.gdn_k_heads,
            v_heads: self.gdn_v_heads,
            head_dim: self.gdn_head_dim,
            conv_taps: self.conv_taps,
            gate: GateActivation::Silu,
        };
        ModelSpec {
            arch: "qwen35".to_string(),
            d: self.d,
            vocab: self.vocab,
            ctx_len: self.ctx_len,
            rms_eps: self.rms_eps,
            embedding_scale: None,
            tied_output: false,
            final_logit_softcap: None,
            layers: (0..layers)
                .map(|l| Layer {
                    mixer: if self.is_attn(l) {
                        Mixer::Attention(attention.clone())
                    } else {
                        Mixer::GatedDelta(gdn.clone())
                    },
                    ffn: Ffn::Dense {
                        dff: self.dff,
                        act: Act::Silu,
                    },
                    pre_norms: true,
                    post_mixer_norm: false,
                    post_ffn_norm: false,
                })
                .collect(),
            mtp_blocks: 0,
            ffn_norm: "post_attention_norm",
            hyper: None,
            ple: None,
            indexer: None,
            residual_scale: None,
            logit_scale: None,
            norm: crate::model::spec::Norm::Rms,
            head_bias: false,
            parts: Vec::new(),
        }
    }
}

/// The rotary cache for `rows` positions, `rope_dim / 2` pairs a row: row `p` (1-based) holds the cosines and the sines
/// of `(p − 1)·θ^(−2i/rope_dim)` — the angles `gk_qk_norm_rope_part` turns 0-based position `p − 1` by, over the
/// rotary width — each computed in f64 and rounded once.
pub fn qwen38_rope_cache(cfg: &Qwen38Cfg, rows: usize) -> (Vec<f32>, Vec<f32>) {
    let half = cfg.rope_dim / 2;
    let angle =
        |p: usize, i: usize| p as f64 * cfg.rope_theta.powf(-2.0 * i as f64 / cfg.rope_dim as f64);
    let (mut cos, mut sin) = (
        Vec::with_capacity(rows * half),
        Vec::with_capacity(rows * half),
    );
    for p in 0..rows {
        for i in 0..half {
            let a = angle(p, i);
            cos.push(a.cos() as f32);
            sin.push(a.sin() as f32);
        }
    }
    (cos, sin)
}

/// The authored forward ([`gen_qwen38_lg`]) with every declared tensor filled DETERMINISTICALLY right after
/// its declaration (a hash of the element index salted by the tensor's name, in (-0.2, 0.2)) and the first
/// and last logit shown — the self-contained program the demo corpus (`examples/rdu_demo/qwen38_tiny.lg`)
/// and the tier-agreement locks run without a weight file. Fills follow declarations because the weights
/// are declared per block, interleaved with the compute that reads them.
pub fn gen_qwen38_filled_lg(cfg: &Qwen38Cfg, dir: &str) -> String {
    let (authored, _f32w, _q8w) = gen_qwen38_lg(cfg, dir);
    let mut src = String::new();
    for line in authored.lines() {
        src.push_str(line);
        src.push('\n');
        let Some(rest) = line.strip_prefix("Let ") else {
            continue;
        };
        let Some((var, tail)) = rest.split_once(" be a ") else {
            continue;
        };
        let Some(dims) = tail.strip_suffix(" tensor of Real.") else {
            continue;
        };
        let (r, c) = dims.split_once(" by ").expect("R by C");
        let n = r.parse::<usize>().expect("rows") * c.parse::<usize>().expect("cols");
        let salt = var.bytes().map(|b| b as u64).sum::<u64>();
        for i in 0..n {
            let h = (i as u64)
                .wrapping_mul(2654435761)
                .wrapping_add(salt.wrapping_mul(40503));
            let v = ((h % 97) as f64 / 97.0 - 0.5) * 0.4;
            src.push_str(&format!("Set item {} of {var} to {v:?}.\n", i + 1));
        }
    }
    src.push_str("Show round(item 1 of logits * 1000000).\n");
    src.push_str(&format!(
        "Show round(item {} of logits * 1000000).\n",
        cfg.vocab
    ));
    src
}

// ── The Qwen3.8 programs ───────────────────────────────────────────────────────────────────────────────────────────
//
// Each form is the generic author's program for this configuration's spec (`model::author`, `Qwen38Cfg::spec`); the
// names stay the model's entry points. `model_author_migration_lock` pins every one of them to the text they had
// before the author took them over.

/// The whole Qwen3.8 forward as a LOGOS program — the single token with no history — and its weight manifest.
pub fn gen_qwen38_lg(cfg: &Qwen38Cfg, dir: &str) -> Authored {
    author::forward(&cfg.spec(), dir)
}

/// One decode step: token `tok` at position `pos` (1-based) in, the next token out, the KV caches and the
/// Gated-DeltaNet state carried. See [`author::decode_step`].
pub fn gen_qwen38_decode_step_lg(cfg: &Qwen38Cfg, dir: &str, ctx: usize, tok: usize, pos: usize) -> Authored {
    author::decode_step(&cfg.spec(), dir, ctx, tok, pos)
}

/// A prefill chunk of one sequence from position `pos0`. See [`author::prefill`].
pub fn gen_qwen38_prefill_lg(cfg: &Qwen38Cfg, dir: &str, ctx: usize, toks: &[usize], pos0: usize) -> Authored {
    author::prefill(&cfg.spec(), dir, ctx, toks, pos0)
}

/// B decode steps at once, each sequence with its own state. See [`author::decode_batch`].
pub fn gen_qwen38_decode_batch_lg(cfg: &Qwen38Cfg, dir: &str, ctx: usize, seqs: &[(usize, usize)]) -> Authored {
    author::decode_batch(&cfg.spec(), dir, ctx, seqs)
}

/// A batched decode step's embedding lookups alone.
pub fn gen_qwen38_decode_batch_embed_lg(cfg: &Qwen38Cfg, dir: &str, toks: &[usize]) -> Authored {
    author::decode_batch_embed(&cfg.spec(), dir, toks)
}

/// Layer `l` of a batched decode step alone.
pub fn gen_qwen38_decode_batch_layer_lg(cfg: &Qwen38Cfg, l: usize, dir: &str, ctx: usize, poss: &[usize]) -> Authored {
    author::stacked_layer(&cfg.layer_spec(l), l, dir, ctx, poss, RowState::PerSequence)
}

/// Layer `l` of a prefill chunk of `chunk` rows from `pos0` alone.
pub fn gen_qwen38_prefill_layer_lg(cfg: &Qwen38Cfg, l: usize, dir: &str, ctx: usize, pos0: usize, chunk: usize) -> Authored {
    let poss: Vec<usize> = (pos0..pos0 + chunk).collect();
    author::stacked_layer(&cfg.layer_spec(l), l, dir, ctx, &poss, RowState::Shared)
}

/// A prefill chunk's head alone.
pub fn gen_qwen38_prefill_head_lg(cfg: &Qwen38Cfg, dir: &str, chunk: usize) -> Authored {
    author::prefill_head(&cfg.spec(), dir, chunk)
}

/// A batched decode step's head alone.
pub fn gen_qwen38_decode_batch_head_lg(cfg: &Qwen38Cfg, dir: &str, bsz: usize) -> Authored {
    author::decode_batch_head(&cfg.spec(), dir, bsz)
}

/// A decode step's embedding lookup alone.
pub fn gen_qwen38_decode_embed_lg(cfg: &Qwen38Cfg, dir: &str, tok: usize) -> Authored {
    author::decode_embed(&cfg.spec(), dir, tok)
}

/// A decode step's head alone: the final norm, the LM head and the arg-max.
pub fn gen_qwen38_decode_head_lg(cfg: &Qwen38Cfg, dir: &str) -> Authored {
    author::decode_head(&cfg.spec(), dir)
}

/// Layer `l` alone, as the forward writes it.
pub fn gen_qwen38_layer_lg(cfg: &Qwen38Cfg, l: usize, dir: &str) -> Authored {
    author::layer(&cfg.layer_spec(l), l, dir)
}

/// Layer `l` of a decode step alone, its attention reading a KV cache of `ctx` positions up to `pos`.
pub fn gen_qwen38_decode_layer_lg(cfg: &Qwen38Cfg, l: usize, dir: &str, ctx: usize, pos: usize) -> Authored {
    author::decode_layer(&cfg.layer_spec(l), l, dir, ctx, pos)
}

/// The final norm and LM head alone.
pub fn gen_qwen38_head_lg(cfg: &Qwen38Cfg, dir: &str) -> Authored {
    author::head(&cfg.spec(), dir)
}

/// The spec split over `tp` chips; `tp` must be a width the model admits (`spatial::fit::qwen38_admits_tp`).
fn split(cfg: &Qwen38Cfg, tp: usize, form: Result<TpAuthored, author::NotYet>) -> TpAuthored {
    form.unwrap_or_else(|e| panic!("Qwen3.8 does not split over {tp} chips ({cfg:?}): {e}"))
}

/// A whole decode step, tensor-parallel over `tp` chips. See [`author::tp_decode_step`].
pub fn gen_qwen38_tp_decode_step_lg(cfg: &Qwen38Cfg, dir: &str, ctx: usize, tok: usize, pos: usize, tp: usize) -> TpAuthored {
    split(cfg, tp, author::tp_decode_step(&cfg.spec(), dir, ctx, tok, pos, tp))
}

/// A batched decode step, tensor-parallel over `tp` chips. See [`author::tp_decode_batch`].
pub fn gen_qwen38_decode_batch_tp_lg(cfg: &Qwen38Cfg, dir: &str, ctx: usize, seqs: &[(usize, usize)], tp: usize) -> TpAuthored {
    split(cfg, tp, author::tp_decode_batch(&cfg.spec(), dir, ctx, seqs, tp))
}

/// Layer `l` of a batched tensor-parallel decode step alone.
pub fn gen_qwen38_decode_batch_tp_layer_lg(cfg: &Qwen38Cfg, l: usize, dir: &str, ctx: usize, poss: &[usize], tp: usize) -> TpAuthored {
    split(cfg, tp, author::tp_stacked_layer(&cfg.layer_spec(l), l, dir, ctx, poss, tp))
}

/// Layer `l` of a tensor-parallel decode step alone.
pub fn gen_qwen38_tp_decode_layer_lg(cfg: &Qwen38Cfg, l: usize, dir: &str, ctx: usize, pos: usize, tp: usize) -> TpAuthored {
    split(cfg, tp, author::tp_decode_layer(&cfg.layer_spec(l), l, dir, ctx, pos, tp))
}

/// Emit a self-contained CUDA program that generates from the real Qwen3.8-27B weights and reports throughput.
///
/// ## Where the speed comes from
///
/// A dense 27B at Q8_0 reads ~27 GB of weights per token, so batch-1 decode is bandwidth-bound and the GEMV
/// is ~90% of the work. Three things follow, and the emitter is built around them:
///
///  1. **One GEMV per activation, not four.** `extract_qwen38` concatenates every projection that shares an
///     activation into a single matrix (`qkvz`, `qgkv`, `gu`), so a GDN layer's q/k/v/z is ONE launch over
///     16384 output rows with no branching, and the FFN's gate+up is one launch over 34816.
///  2. **One CUDA graph for the whole token.** ~800 kernels per token at a few µs of launch overhead each is
///     milliseconds against a ~24 ms budget. The decode step is captured once and replayed, with the sampled
///     token and the position advanced ON DEVICE (`gk_argmax_adv`) so no replay needs a host round trip.
///  3. **Fusion past what a per-operator graph can express.** The conv carries its silu, both GDN gates come
///     from one launch, the recurrence touches its memory once (see `gdn.rs`), and the residual add, the next
///     RMSNorm and the activation re-quantization are a single kernel.
///
/// `precise` swaps the split-K flash attention for the byte-exact multi-pass kernel — slower, but it does not
/// reorder float additions, so it is the lane to check a token stream against llama.cpp with.
/// `deq` selects how much of the decode keeps its activations in f32 instead of quantizing them to int8:
/// 0 = none (dp4a everywhere), 1 = the LM head only, 2 = every Q8_0 GEMV.
///
/// At batch 1 this is nearly free. A GEMV reads a whole weight matrix and ONE activation vector, so the
/// activation contributes no measurable traffic and quantizing it saves no bandwidth — it only lets the
/// inner loop use `dp4a`. The dequantizing form costs ~3 float ops per weight instead of a quarter of a
/// dp4a, which at 1400 GB/s works out to about 4 TFLOP/s against the A100's 19.5: still memory bound. What
/// it buys is the removal of the activation quantization error entirely.
pub fn emit_qwen38_generate(
    cfg: &Qwen38Cfg,
    wdir: &str,
    prompt: &[usize],
    n_gen: usize,
    precise: bool,
    deq: u8,
) -> String {
    let debug = std::env::var("LOGOS_Q38_DEBUG").is_ok();
    let (d, dff, taps) = (cfg.d, cfg.dff, cfg.conv_taps);
    let (qk, vw) = (cfg.gdn_qk(), cfg.gdn_v());
    let (aq, akv) = (cfg.aq(), cfg.akv());
    let (nk, nv, ghd) = (cfg.gdn_k_heads, cfg.gdn_v_heads, cfg.gdn_head_dim);
    let convc = 2 * qk + vw; // channels the conv covers: q|k|v
    let projw = 2 * qk + 2 * vw; // q|k|v|z
    let attnw = 2 * aq + 2 * akv; // q|gate|k|v
    let maxseq = prompt.len() + n_gen + 128;
    let scale = 1.0f64 / (cfg.hd as f64).sqrt();
    let prompt_c = prompt
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let attn_split = 4usize;

    // ── weight tables, per layer, loaded from the files the extractor wrote ──
    let mut decls = String::new();
    let mut loads = String::new();
    let q8 = |decls: &mut String, loads: &mut String, var: &str, out: usize, inn: usize| {
        let (qn, sn) = (out * inn, out * inn / 32);
        decls.push_str(&format!(
            "static signed char* {var}_q; static __half* {var}_s;\n"
        ));
        loads.push_str(&format!(
            "  LDQ({var}, {qn}, {sn}, \"{wdir}/{var}.q\", \"{wdir}/{var}.s\");\n"
        ));
    };
    let f32t = |decls: &mut String, loads: &mut String, var: &str, n: usize| {
        decls.push_str(&format!("static float* {var}_f;\n"));
        loads.push_str(&format!("  LDF({var}, {n}, \"{wdir}/{var}.f\");\n"));
    };
    q8(&mut decls, &mut loads, "emb", cfg.vocab, d);
    q8(&mut decls, &mut loads, "Wout", cfg.vocab, d);
    f32t(&mut decls, &mut loads, "on", d);
    for l in 0..cfg.layers {
        f32t(&mut decls, &mut loads, &format!("an{l}"), d);
        f32t(&mut decls, &mut loads, &format!("fn{l}"), d);
        if cfg.is_attn(l) {
            q8(&mut decls, &mut loads, &format!("qgkv{l}"), attnw, d);
            f32t(&mut decls, &mut loads, &format!("Qn{l}"), cfg.hd);
            f32t(&mut decls, &mut loads, &format!("Kn{l}"), cfg.hd);
            q8(&mut decls, &mut loads, &format!("Wo{l}"), d, aq);
        } else {
            q8(&mut decls, &mut loads, &format!("qkvz{l}"), projw, d);
            f32t(&mut decls, &mut loads, &format!("ba{l}"), 2 * nv * d);
            f32t(&mut decls, &mut loads, &format!("Wc{l}"), taps * convc);
            f32t(&mut decls, &mut loads, &format!("Al{l}"), nv);
            f32t(&mut decls, &mut loads, &format!("Dt{l}"), nv);
            f32t(&mut decls, &mut loads, &format!("Gn{l}"), ghd);
            q8(&mut decls, &mut loads, &format!("Wo{l}"), d, vw);
        }
        q8(&mut decls, &mut loads, &format!("gu{l}"), 2 * dff, d);
        q8(&mut decls, &mut loads, &format!("Wd{l}"), d, dff);
    }

    // ── per-layer state: KV caches for the 16 attention layers, memories + conv rings for the 48 GDN ones ──
    let mut state_decl = String::new();
    let mut state_alloc = String::new();
    for l in 0..cfg.layers {
        if cfg.is_attn(l) {
            state_decl.push_str(&format!("static float *kc{l}, *vc{l};\n"));
            state_alloc.push_str(&format!(
                "  cudaMalloc(&kc{l},(size_t){maxseq}*{akv}*4); cudaMalloc(&vc{l},(size_t){maxseq}*{akv}*4);\n"
            ));
        } else {
            state_decl.push_str(&format!("static float *st{l}, *cs{l};\n"));
            state_alloc.push_str(&format!(
                "  cudaMalloc(&st{l},(size_t){vw}*{ghd}*4); cudaMemset(st{l},0,(size_t){vw}*{ghd}*4);\n\
                 \x20 cudaMalloc(&cs{l},(size_t){taps}*{convc}*4); cudaMemset(cs{l},0,(size_t){taps}*{convc}*4);\n"
            ));
        }
    }

    // ── the decode step: one token through 64 layers, emitted as straight-line launches on stream `s` ──
    let fast_attn = if precise {
        format!("gk_attn<<<{h},256,(size_t){maxseq}*4,s>>>(attnb,pbuf,kc{{L}},vc{{L}},d_pos,{h},{kv},{hd},SCALEV);", h = cfg.h, kv = cfg.kv, hd = cfg.hd)
    } else {
        format!("gk_attn_part<<<{h}*{p},256,0,s>>>(pbuf,kc{{L}},vc{{L}},d_pos,{h},{kv},{hd},SCALEV,{p},acc_part,m_part,d_part); \
                 gk_attn_comb<<<{h},128,0,s>>>(attnb,{h},{hd},{p},acc_part,m_part,d_part);", h = cfg.h, kv = cfg.kv, hd = cfg.hd, p = attn_split)
    };
    // deq==2 keeps EVERY activation in f32. Each Q8 GEMV is paired with the f32 buffer its int8 operand was
    // quantized from, so the pairing is mechanical: whatever kernel produced `aqE/asE` also produced `x_f32`,
    // `attnb` is what `gk_quant_act` read, `gnbuf` is what the gate norm wrote, `x2n` is the post-attention
    // norm, and the SwiGLU hadamard lands in `hfbuf`. The quantization steps they replace disappear.
    let all_deq = deq == 2;
    // deq==3 is the two-pass int8 lane: every activation carries a second int8 word, so the GEMV stays on
    // dp4a while the activation it reads is 254x closer to the f32 value the norm actually produced.
    let hl = deq == 3;
    // Two output rows per warp. Off by default until a chip has been measured with it.
    let wide = std::env::var("LOGOS_Q38_WIDE").is_ok();
    // Unroll two K-blocks per iteration. Bit-identical, so it is a pure scheduling choice.
    let unroll = std::env::var("LOGOS_Q38_U2").is_ok();
    let unroll4 = std::env::var("LOGOS_Q38_U4").is_ok();
    // Split-K, applied ONLY to projections narrow enough to leave the machine idle. The wide matrices
    // (qkvz N=16384, gu N=34816, the LM head N=248320) already saturate and gain nothing from splitting.
    let splitk: usize = std::env::var("LOGOS_Q38_SPLITK")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let nval = move |n: &str| -> usize {
        if n == "E" {
            d
        } else {
            n.parse().unwrap_or(usize::MAX)
        }
    };
    let narrow = move |n: &str| splitk > 1 && nval(n) <= 8192;
    let gemv = |y: &str, w: &str, aq: &str, f32src: &str, k: &str, n: &str| -> String {
        if all_deq {
            format!("gk_gemv_deq_h<<<Gw({n}),256,0,s>>>({y},{w}_q,{w}_s,{f32src},{k},{n});")
        } else if hl {
            // `aq` names the "<hi>,<scale>" pair; the low word is the hi buffer's name with an `l` suffix.
            let (h, sc) = aq.split_once(',').expect("activation pair");
            if narrow(n) {
                format!("gk_gemv_hl_sk<<<dim3(Gw({n}),{splitk}),256,0,s>>>(skbuf,{w}_q,{w}_s,{h},{h}l,{sc},{k},{n},{splitk});\n\
                 \x20 gk_sk_combine<<<Gs({n}),256,0,s>>>({y},skbuf,{n},{splitk});")
            } else if wide {
                format!("gk_gemv_hl2<<<Gw2({n}),256,0,s>>>({y},{w}_q,{w}_s,{h},{h}l,{sc},{k},{n});")
            } else if unroll4 {
                format!("gk_gemv_hlu4<<<Gw({n}),256,0,s>>>({y},{w}_q,{w}_s,{h},{h}l,{sc},{k},{n});")
            } else if unroll {
                format!("gk_gemv_hlu<<<Gw({n}),256,0,s>>>({y},{w}_q,{w}_s,{h},{h}l,{sc},{k},{n});")
            } else {
                format!("gk_gemv_hl<<<Gw({n}),256,0,s>>>({y},{w}_q,{w}_s,{h},{h}l,{sc},{k},{n});")
            }
        } else if wide {
            format!("gk_gemv_h2<<<Gw2({n}),256,0,s>>>({y},{w}_q,{w}_s,{aq},{k},{n});")
        } else if unroll {
            format!("gk_gemv_hu<<<Gw({n}),256,0,s>>>({y},{w}_q,{w}_s,{aq},{k},{n});")
        } else {
            format!("gk_gemv_h<<<Gw({n}),256,0,s>>>({y},{w}_q,{w}_s,{aq},{k},{n});")
        }
    };
    let mut step = String::new();
    for l in 0..cfg.layers {
        // Layer 0 is entered straight from the embedding; every other layer is entered from the previous
        // layer's residual, and the entry norm absorbs that add rather than paying a launch for it.
        let entry = if hl {
            // Every entry norm emits the f32 side output too: the two-word split is computed from it, and
            // the GDN body already wanted it for the tiny f32-weight gate projection.
            if l == 0 {
                format!("gk_rmsnorm_q8f_hl<<<1,256,0,s>>>(x,aqE,aqEl,asE,x_f32,an{l}_f,E,EPSV);")
            } else {
                format!("gk_add_rmsnorm_q8f_hl<<<1,256,0,s>>>(x,tmp,aqE,aqEl,asE,x_f32,an{l}_f,E,EPSV);")
            }
        } else if cfg.is_attn(l) && all_deq {
            if l == 0 {
                format!("gk_rmsnorm_q8f<<<1,256,0,s>>>(x,aqE,asE,x_f32,an{l}_f,E,EPSV);")
            } else {
                format!("gk_add_rmsnorm_q8f<<<1,256,0,s>>>(x,tmp,aqE,asE,x_f32,an{l}_f,E,EPSV);")
            }
        } else if cfg.is_attn(l) {
            if l == 0 {
                format!("gk_rmsnorm_q8<<<1,256,0,s>>>(x,aqE,asE,an{l}_f,E,EPSV);")
            } else {
                format!("gk_add_rmsnorm_q8a<<<1,256,0,s>>>(x,tmp,aqE,asE,an{l}_f,E,EPSV);")
            }
        } else if l == 0 {
            format!("gk_rmsnorm_q8f<<<1,256,0,s>>>(x,aqE,asE,x_f32,an{l}_f,E,EPSV);")
        } else {
            format!("gk_add_rmsnorm_q8f<<<1,256,0,s>>>(x,tmp,aqE,asE,x_f32,an{l}_f,E,EPSV);")
        };
        if cfg.is_attn(l) {
            step.push_str(&format!(
                "  {entry}\n\
                 \x20 {g_qgkv}\n\
                 \x20 gk_kv_store<<<Gs({akv}*2),256,0,s>>>(kc{l},vc{l},d_pos,{akv},pbuf+{two_aq});\n\
                 \x20 gk_qk_norm_rope_part<<<{h},128,0,s>>>(pbuf,d_zero,0,Qn{l}_f,{h},{hd},{rot},ROPEV,EPSV);\n\
                 \x20 gk_qk_norm_rope_part<<<{kv},128,0,s>>>(kc{l},d_pos,{akv},Kn{l}_f,{kv},{hd},{rot},ROPEV,EPSV);\n\
                 \x20 {attn}\n\
                 \x20 gk_gate_mul<<<Gs({aq}),256,0,s>>>(attnb,pbuf+{aq},{aq});\n\
                 \x20 {q_attn}{g_wo_attn}\n",
                attn = fast_attn.replace("{L}", &l.to_string()),
                two_aq = 2 * aq, h = cfg.h, kv = cfg.kv, hd = cfg.hd, rot = cfg.rope_dim, entry = entry,
                g_qgkv = gemv("pbuf", &format!("qgkv{l}"), "aqE,asE", "x_f32", "E", &attnw.to_string()),
                q_attn = if all_deq { String::new() }
                    else if hl { format!("gk_quant_act_hl<<<Gs({aq}),256,0,s>>>(attnb,aqQ,aqQl,asQ,{aq});\n  ") }
                    else { format!("gk_quant_act<<<Gs({aq}),256,0,s>>>(attnb,aqQ,asQ,{aq});\n  ") },
                g_wo_attn = gemv("tmp", &format!("Wo{l}"), "aqQ,asQ", "attnb", &aq.to_string(), "E"),
            ));
        } else {
            step.push_str(&format!(
                "  {entry}\n\
                 \x20 {p0}\
                 \x20 {g_qkvz}\n\
                 \x20 gk_gemv_f32w<<<Gw({two_nv}),256,0,s>>>(gbuf,ba{l}_f,x_f32,E,{two_nv});\n\
                 \x20 {p1}gk_conv_silu_l2<<<{convc}/{ghd},{ghd},0,s>>>(pbuf,Wc{l}_f,cs{l},cbuf,{convc},{taps},{two_nk},EPSV);\n\
                 \x20 {p2}\
                 \x20 gk_gdn_gates<<<Gs({nv}),256,0,s>>>(gbuf,gbuf+{nv},Dt{l}_f,Al{l}_f,betab,decayb,{nv});\n\
                 \x20 {p3}\
                                  \x20 gdn_step<<<dim3({nv},({ghd}+3)/4),128,0,s>>>(cbuf,cbuf+{qk},cbuf+{two_qk},betab,decayb,st{l},obuf,{ghd},{nk});\n\
                 \x20 {gate_norm}\n\
                 \x20 {p4}\
                 \x20 {g_wo_gdn}\n\
                 \x20 {p5}\n",
                p1 = if debug && l == 0 { "gk_dbg3<<<1,32,0,s>>>(pbuf,dbg+3*72); " } else { "" },
                p3 = if debug && l == 0 { "gk_dbg3<<<1,32,0,s>>>(betab,dbg+3*74); gk_dbg3<<<1,32,0,s>>>(decayb,dbg+3*75);" } else { "" },
                p4 = if debug && l == 0 { "gk_dbg3<<<1,32,0,s>>>(obuf,dbg+3*76); gk_dbg3<<<1,32,0,s>>>(gnbuf,dbg+3*77);" } else { "" },
                p5 = if debug && l == 0 { "gk_dbg3<<<1,32,0,s>>>(tmp,dbg+3*78);" } else { "" },
                p2 = if debug && l == 0 { "gk_dbg3<<<1,32,0,s>>>(cbuf,dbg+3*73);" } else { "" },
                p0 = if debug && l == 0 {
                    "gk_dbg3<<<1,32,0,s>>>(x,dbg+3*70); gk_dbg3<<<1,32,0,s>>>(x_f32,dbg+3*71);"
                } else { "" },
                two_nv = 2 * nv, two_nk = 2 * nk, two_qk = 2 * qk, entry = entry,
                g_qkvz = gemv("pbuf", &format!("qkvz{l}"), "aqE,asE", "x_f32", "E", &projw.to_string()),
                g_wo_gdn = gemv("tmp", &format!("Wo{l}"), "aqQ,asQ", "gnbuf", &vw.to_string(), "E"),
                gate_norm = if hl {
                    format!("gdn_gate_norm_hl<<<{nv},{ghd},0,s>>>(pbuf+{off},Gn{l}_f,obuf,gnbuf,aqQ,aqQl,asQ,{ghd},EPSV);", off = 2 * qk + vw)
                } else if all_deq {
                    format!("gdn_gate_norm<<<{nv},{ghd},0,s>>>(pbuf+{off},Gn{l}_f,obuf,gnbuf,{ghd},EPSV);", off = 2 * qk + vw)
                } else {
                    format!("gdn_gate_norm_q<<<{nv},{ghd},0,s>>>(pbuf+{off},Gn{l}_f,obuf,gnbuf,aqQ,asQ,{ghd},EPSV);", off = 2 * qk + vw)
                },
            ));
        }
        // shared tail — residual + post-norm + requantize in ONE kernel, then the dense SwiGLU FFN
        step.push_str(&format!(
            "  {tail_norm}\n\
             \x20 {g_gu}\n\
             \x20 {had}\n\
             \x20 {g_wd}\n\
             \x20 {resid}",
            tail_norm = if hl { format!("gk_add_rmsnorm_q8_hl<<<1,256,0,s>>>(x,tmp,x2n,fn{l}_f,ffQ,ffQl,ffS,E,EPSV);") }
                        else { format!("gk_add_rmsnorm_q8<<<1,256,0,s>>>(x,tmp,x2n,fn{l}_f,ffQ,ffS,E,EPSV);") },
            g_gu = gemv("ffbuf", &format!("gu{l}"), "ffQ,ffS", "x2n", "E", &(2 * dff).to_string()),
            g_wd = gemv("tmp", &format!("Wd{l}"), "hqQ,hqS", "hfbuf", &dff.to_string(), "E"),
            had = if all_deq { format!("gk_silu_had_f<<<Gs({dff}),256,0,s>>>(ffbuf,ffbuf+{dff},hfbuf,{dff});") }
                  else if hl { format!("gk_silu_had_q8_hl<<<Gs({dff}),256,0,s>>>(ffbuf,ffbuf+{dff},hqQ,hqQl,hqS,{dff});") }
                  else { format!("gk_silu_had_q8<<<Gs({dff}),256,0,s>>>(ffbuf,ffbuf+{dff},hqQ,hqS,{dff});") },
            // Folded into the next layer's entry norm; only the final layer still pays for its own add.
            resid = if l + 1 == cfg.layers { "gk_add<<<Gs(E),256,0,s>>>(x,tmp,E);\n" } else { "\n" },
        ));
        if debug {
            step.push_str(&format!("  gk_dbg3<<<1,32,0,s>>>(x,dbg+3*{l});\n"));
        }
    }

    let prov = super::provenance::header(super::provenance::Provenance::HandWritten {
        model: "Qwen3.8-27B",
        lane: "generation",
    });
    format!(
        "{prov}// Qwen3.8-27B generation.\n\
         #include <cstdio>\n#include <cstdlib>\n#include <cstring>\n#include <cmath>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {gdn_k}{q38_k}{gen_k}{extra_k}\n\
         // The dimension macros come AFTER the kernels on purpose: the shared kernel set takes `int E` as a\n\
         // parameter name, and a `#define E 5120` above it would rewrite that into `int 5120`.\n\
         #define E {d}\n#define VOCAB {vocab}\n#define LAYERS {layers}\n#define MAXSEQ {maxseq}\n\
         #define EPSV {eps:e}f\n#define ROPEV {rope:e}f\n\
         static const float SCALEV = {scale}f;\n\
         {decls}\n{state}\n\
         static float* x; static float* x_f32; static float* x2n; static float* tmp;\n\
         static float *pbuf,*cbuf,*obuf,*gnbuf,*gbuf,*betab,*decayb,*qbuf,*attnb,*ffbuf,*hfbuf,*skbuf,*logits;\n\
         static float *acc_part,*m_part,*d_part;\n\
         static signed char *aqE,*aqQ,*ffQ,*hqQ,*aqEl,*aqQl,*ffQl,*hqQl; static float *asE,*asQ,*ffS,*hqS;\n\
         static int *d_pos,*d_tok,*d_zero; static float* dbg;\n\
         static inline int Gw(int n){{ return (n*32+255)/256; }}\n\
         static inline int Gw2(int n){{ return (((n+1)/2)*32+255)/256; }}\n\
         static inline int Gs(int n){{ return (n+255)/256; }}\n\
         #define LDQ(v,QN,SN,QP,SP) do{{ \\\n\
         \x20 signed char* hq=(signed char*)malloc((size_t)QN); float* hs=(float*)malloc((size_t)SN*4); \\\n\
         \x20 FILE* f=fopen(QP,\"rb\"); if(!f||fread(hq,1,(size_t)QN,f)!=(size_t)QN){{fprintf(stderr,\"load %s\\n\",QP);return 1;}} fclose(f); \\\n\
         \x20 f=fopen(SP,\"rb\"); if(!f||fread(hs,4,(size_t)SN,f)!=(size_t)SN){{fprintf(stderr,\"load %s\\n\",SP);return 1;}} fclose(f); \\\n\
         \x20 cudaMalloc(&v##_q,(size_t)QN); cudaMemcpy(v##_q,hq,(size_t)QN,cudaMemcpyHostToDevice); \\\n\
         \x20 __half* hh=(__half*)malloc((size_t)(SN)*2); for(long i=0;i<(long)(SN);++i) hh[i]=__float2half(hs[i]); \\\n\
         \x20 cudaMalloc(&v##_s,(size_t)(SN)*2); cudaMemcpy(v##_s,hh,(size_t)(SN)*2,cudaMemcpyHostToDevice); \\\n\
         \x20 free(hq); free(hs); free(hh); }}while(0)\n\
         #define LDF(v,N,P) do{{ \\\n\
         \x20 float* h=(float*)malloc((size_t)N*4); FILE* f=fopen(P,\"rb\"); \\\n\
         \x20 if(!f||fread(h,4,(size_t)N,f)!=(size_t)N){{fprintf(stderr,\"load %s\\n\",P);return 1;}} fclose(f); \\\n\
         \x20 cudaMalloc(&v##_f,(size_t)N*4); cudaMemcpy(v##_f,h,(size_t)N*4,cudaMemcpyHostToDevice); free(h); }}while(0)\n\
         static void decode_step(cudaStream_t s){{\n\
         \x20 gk_embed_h<<<Gs(E),256,0,s>>>(x,emb_q,emb_s,d_tok,E);\n\
         {step}\
         \x20 {final_norm}\n\
         \x20 {head}\n\
         \x20 gk_argmax_adv<<<1,256,0,s>>>(logits,VOCAB,d_tok,d_pos);\n}}\n\
         int main(){{\n{loads}\n{alloc}\n\
         \x20 cudaMalloc(&x,E*4); cudaMalloc(&x_f32,E*4); cudaMalloc(&x2n,E*4); cudaMalloc(&tmp,E*4);\n\
         \x20 cudaMalloc(&pbuf,{pmax}*4); cudaMalloc(&cbuf,{convc}*4); cudaMalloc(&obuf,{vw}*4); cudaMalloc(&gnbuf,{vw}*4);\n\
         \x20 cudaMalloc(&gbuf,{two_nv}*4); cudaMalloc(&betab,{nv}*4); cudaMalloc(&decayb,{nv}*4);\n\
         \x20 cudaMalloc(&qbuf,{aq}*4); cudaMalloc(&attnb,{aq}*4); cudaMalloc(&ffbuf,{two_dff}*4);\n\
         \x20 cudaMalloc(&logits,(size_t)VOCAB*4); cudaMalloc(&hfbuf,(size_t){dff}*4);\n\
         \x20 cudaMalloc(&skbuf,(size_t)8*8192*4);\n\
         \x20 cudaMalloc(&acc_part,(size_t){h}*{p}*{hd}*4); cudaMalloc(&m_part,(size_t){h}*{p}*4); cudaMalloc(&d_part,(size_t){h}*{p}*4);\n\
         \x20 cudaMalloc(&aqE,E); cudaMalloc(&asE,(size_t)(E/32)*4); cudaMalloc(&aqEl,E);\n\
         \x20 cudaMalloc(&aqQ,{amax}); cudaMalloc(&asQ,(size_t)({amax}/32)*4); cudaMalloc(&aqQl,{amax});\n\
         \x20 cudaMalloc(&ffQ,E); cudaMalloc(&ffS,(size_t)(E/32)*4); cudaMalloc(&ffQl,E);\n\
         \x20 cudaMalloc(&hqQ,{dff}); cudaMalloc(&hqS,(size_t)({dff}/32)*4); cudaMalloc(&hqQl,{dff});\n\
         \x20 cudaMalloc(&d_pos,4); cudaMalloc(&d_tok,4); cudaMalloc(&d_zero,4); cudaMalloc(&dbg,(size_t)(LAYERS+16)*12);\n\
         \x20 {{ int z=0; cudaMemcpy(d_zero,&z,4,cudaMemcpyHostToDevice); }}\n\
         \x20 cudaStream_t s; cudaStreamCreate(&s);\n\
         \x20 static const int PROMPT[] = {{{prompt_c}}}; const int NP = sizeof(PROMPT)/sizeof(int);\n\
         \x20 // ── prefill: the same step, one token at a time, growing the caches and the memories ──\n\
         \x20 for(int i=0;i<NP;++i){{ int p=i; cudaMemcpy(d_pos,&p,4,cudaMemcpyHostToDevice);\n\
         \x20   cudaMemcpy(d_tok,&PROMPT[i],4,cudaMemcpyHostToDevice); decode_step(s); cudaStreamSynchronize(s); {dbgrep} }}\n\
         // The logit vector at the LAST prompt position -- the one that chooses the first generated
         // token, and the same vector llama.cpp returns from `llama_get_logits_ith(n_prompt-1)`. Dumped
         // raw so accuracy is settled against an f64 oracle rather than by whether two argmaxes agree.
         \x20 if(const char* lp = getenv(\"LOGOS_Q38_LOGITS\")){{ static float hl[VOCAB];
         \x20   cudaMemcpy(hl,logits,(size_t)VOCAB*4,cudaMemcpyDeviceToHost);
         \x20   FILE* lf=fopen(lp,\"wb\"); if(lf){{ fwrite(hl,4,VOCAB,lf); fclose(lf);
         \x20     fprintf(stderr,\"WROTE_LOGITS %s (%d)\\n\", lp, VOCAB); }} }}\n\
         \x20 // ── capture ONE decode step and replay it: ~800 launches per token become one graph launch ──\n\
         \x20 cudaGraph_t g; cudaGraphExec_t ge;\n\
         \x20 if(cudaStreamBeginCapture(s,cudaStreamCaptureModeGlobal)!=cudaSuccess){{fprintf(stderr,\"capture\\n\");return 1;}}\n\
         \x20 decode_step(s);\n\
         \x20 if(cudaStreamEndCapture(s,&g)!=cudaSuccess){{fprintf(stderr,\"endcapture\\n\");return 1;}}\n\
         \x20 if(cudaGraphInstantiate(&ge,g,0)!=cudaSuccess){{fprintf(stderr,\"inst\\n\");return 1;}}\n\
         \x20 cudaEvent_t t0,t1; cudaEventCreate(&t0); cudaEventCreate(&t1);\n\
         \x20 int tok=0; cudaMemcpy(&tok,d_tok,4,cudaMemcpyDeviceToHost);\n\
         \x20 printf(\"GENERATED_TOKENS: %d\", tok);\n\
         \x20 float genms=0.f; int steps=0;\n\
         \x20 for(int i=1;i<{n_gen};++i){{ cudaEventRecord(t0,s); cudaGraphLaunch(ge,s); cudaEventRecord(t1,s);\n\
         \x20   cudaStreamSynchronize(s); float ms; cudaEventElapsedTime(&ms,t0,t1); genms+=ms; steps++;\n\
         \x20   cudaMemcpy(&tok,d_tok,4,cudaMemcpyDeviceToHost); printf(\" %d\", tok); }}\n\
         \x20 printf(\"\\n\");\n\
         \x20 if(steps) fprintf(stderr,\"GEN_TOK_S %.2f (%.3f ms/token, %d steps)\\n\", 1000.0*steps/genms, genms/steps, steps);\n\
         \x20 {{ int z=0; cudaMemcpy(d_pos,&z,4,cudaMemcpyHostToDevice); cudaStreamSynchronize(s);\n\
         \x20   cudaEventRecord(t0,s); for(int i=0;i<128;++i) cudaGraphLaunch(ge,s); cudaEventRecord(t1,s);\n\
         \x20   cudaStreamSynchronize(s); float ms; cudaEventElapsedTime(&ms,t0,t1);\n\
         \x20   fprintf(stderr,\"GEN_TOK_S_TG128 %.2f (128 replays from an empty cache = llama-bench tg128)\\n\", 128000.0/ms); }}\n\
         \x20 return 0;\n}}\n",
        vocab = cfg.vocab, layers = cfg.layers, eps = cfg.rms_eps, rope = cfg.rope_theta,
        gdn_k = crate::codegen_cuda::gdn::GDN_KERNELS,
        q38_k = crate::codegen_cuda::gdn::QWEN38_KERNELS,
        final_norm = if deq == 3 {
            "gk_rmsnorm_q8f_hl<<<1,256,0,s>>>(x,aqE,aqEl,asE,x_f32,on_f,E,EPSV);"
        } else {
            "gk_rmsnorm_q8f<<<1,256,0,s>>>(x,aqE,asE,x_f32,on_f,E,EPSV);"
        },
        head = if hl && unroll4 {
            "gk_gemv_hlu4<<<Gw(VOCAB),256,0,s>>>(logits,Wout_q,Wout_s,aqE,aqEl,asE,E,VOCAB);"
        } else if hl && unroll {
            "gk_gemv_hlu<<<Gw(VOCAB),256,0,s>>>(logits,Wout_q,Wout_s,aqE,aqEl,asE,E,VOCAB);"
        } else if hl && wide {
            "gk_gemv_hl2<<<Gw2(VOCAB),256,0,s>>>(logits,Wout_q,Wout_s,aqE,aqEl,asE,E,VOCAB);"
        } else if hl {
            "gk_gemv_hl<<<Gw(VOCAB),256,0,s>>>(logits,Wout_q,Wout_s,aqE,aqEl,asE,E,VOCAB);"
        } else if deq >= 1 {
            "gk_gemv_deq_h<<<Gw(VOCAB),256,0,s>>>(logits,Wout_q,Wout_s,x_f32,E,VOCAB);"
        } else {
            "gk_gemv_h<<<Gw(VOCAB),256,0,s>>>(logits,Wout_q,Wout_s,aqE,asE,E,VOCAB);"
        },
        gen_k = crate::codegen_cuda::qwen_emit::GEN_DEVICE_KERNELS,
        extra_k = QWEN38_EXTRA_KERNELS,
        state = state_decl, alloc = state_alloc,
        pmax = projw.max(attnw), two_nv = 2 * nv, two_dff = 2 * dff,
        amax = vw.max(aq), h = cfg.h, hd = cfg.hd, p = attn_split,
        dbgrep = if debug {
            "if(i==0){ float hb[(LAYERS+16)*3]; cudaMemcpy(hb,dbg,(size_t)(LAYERS+16)*12,cudaMemcpyDeviceToHost); for(int L=0;L<4;++L) fprintf(stderr,\"DBG l_out-%d=[%.4f,%.4f,%.4f]\\n\",L,hb[3*L],hb[3*L+1],hb[3*L+2]); const char* nm[9]={\"embed\",\"xn0\",\"qkv0\",\"conv0\",\"beta\",\"decay\",\"gdnout\",\"gatenorm\",\"oproj\"}; for(int L=0;L<9;++L) fprintf(stderr,\"DBG %s=[%.6f,%.6f,%.6f]\\n\",nm[L],hb[3*(70+L)],hb[3*(70+L)+1],hb[3*(70+L)+2]); } if(i==NP-1){ float* al=(float*)malloc((size_t)VOCAB*4); cudaMemcpy(al,logits,(size_t)VOCAB*4,cudaMemcpyDeviceToHost); int bi=0; for(int L=1;L<VOCAB;++L) if(al[L]>al[bi]) bi=L; int rank=0; for(int L=0;L<VOCAB;++L) if(al[L]>al[11751]) rank++; fprintf(stderr,\"DBG argmax=%d(%.4g) golden11751=%.4g rank=%d\\n\",bi,al[bi],al[11751],rank); free(al); }"
        } else { "" },
    )
}

/// The last two kernels the hybrid needs: an f32-weight GEMV for GDN's tiny gate projections (quantizing a
/// `[d, 48]` matrix would risk the gates for a rounding error that buys 0.26% of bandwidth), a KV-cache store
/// that splits the fused projection's k and v halves into their caches, and the SwiGLU epilogue fused with the
/// activation re-quantization so the FFN's middle never round-trips through global memory as f32.
pub const QWEN38_EXTRA_KERNELS: &str = r#"

// A plain argmax that does NOT advance the position — the MTP draft is a guess about the future, not a step
// through it, so it must not touch the decode state it is speculating on.
__global__ void gk_argmax_only(const float* logits, int vocab, int* out){
  __shared__ float bv[256]; __shared__ int bi[256]; float bestv=-1e30f; int besti=0;
  for(int i=threadIdx.x;i<vocab;i+=blockDim.x){ float v=logits[i]; if(v>bestv){bestv=v;besti=i;} }
  bv[threadIdx.x]=bestv; bi[threadIdx.x]=besti; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k && bv[threadIdx.x+k]>bv[threadIdx.x]){ bv[threadIdx.x]=bv[threadIdx.x+k]; bi[threadIdx.x]=bi[threadIdx.x+k]; } __syncthreads(); }
  if(threadIdx.x==0) *out=bi[0];
}
// Concatenate the two normed halves the nextn adapter consumes: [ rmsnorm(embed(t),enorm) | rmsnorm(h,hnorm) ].
__global__ void gk_mtp_cat(float* __restrict__ out, const float* __restrict__ e, const float* __restrict__ h,
                           const float* __restrict__ we, const float* __restrict__ wh, int n, float eps){
  __shared__ float sh[256];
  const float* src = blockIdx.x == 0 ? e : h;
  const float* w   = blockIdx.x == 0 ? we : wh;
  float* dst       = out + (long)blockIdx.x * n;
  float s=0.f; for(int i=threadIdx.x;i<n;i+=blockDim.x) s+=src[i]*src[i];
  sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  float sc=rsqrtf(sh[0]/n+eps);
  for(int i=threadIdx.x;i<n;i+=blockDim.x) dst[i]=src[i]*sc*w[i];
}


// ══ SPLIT-K FOR THE NARROW PROJECTIONS ══════════════════════════════════════════════════════════
// One warp per output row means the o-projection and the down-projection (both N=5120) launch only 640
// blocks — 38.8 of an H100's 64 warps per SM, against 100% for the wide matrices. Those two are 31.7% of the
// model's weights, so a third of every token runs at 61% occupancy. Splitting K into S chunks multiplies the
// block count by S and fills the machine.
//
// The combine is a SEPARATE kernel summing the partials in FIXED ascending order, not an atomicAdd: float
// addition is not associative, so an atomic would make the result depend on scheduler order and the same
// prompt could produce different tokens on different runs. Determinism is not negotiable here.
//
// This is NOT bit-identical to the rolled form — chunking K re-associates the sum — so it is gated on the
// golden token stream and on distance from the f64 oracle, never adopted on throughput alone.
__global__ void gk_gemv_hl_sk(float* __restrict__ part, const signed char* __restrict__ wq,
                              const __half* __restrict__ ws, const signed char* __restrict__ xh,
                              const signed char* __restrict__ xl, const float* __restrict__ xs,
                              int K, int N, int S){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return;
  int sl=blockIdx.y, lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K);
  const int4* h16=(const int4*)xh; const int4* l16=(const int4*)xl;
  const __half* wsr=ws+(long)o*(K/32);
  int nq=K/512, per=(nq+S-1)/S, b0=sl*per, be=b0+per<nq?b0+per:nq;
  float acc=0.f;
  for(int b=b0;b<be;b++){
    int4 w=w16[b*32+lane], hh=h16[b*32+lane], ll=l16[b*32+lane];
    int blk=b*16+lane/2;
    int dh=__dp4a(w.x,hh.x,0)+__dp4a(w.y,hh.y,0)+__dp4a(w.z,hh.z,0)+__dp4a(w.w,hh.w,0);
    int dl=__dp4a(w.x,ll.x,0)+__dp4a(w.y,ll.y,0)+__dp4a(w.z,ll.z,0)+__dp4a(w.w,ll.w,0);
    acc += ((float)dh + (float)dl*(1.0f/127.0f)) * (__half2float(wsr[blk])*xs[blk]);
  }
  if(sl==S-1){ int done=nq*512;
    for(int j=done+lane;j<K;j+=32){ int blk=j/32;
      float a=(float)xh[j] + (float)xl[j]*(1.0f/127.0f);
      acc += a*(float)wq[(long)o*K+j]*(__half2float(wsr[blk])*xs[blk]); } }
  for(int t=16;t>0;t>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,t);
  if(lane==0) part[(long)sl*N+o]=acc;
}
// Fixed ascending order over the K-chunks: same inputs always give the same bits.
__global__ void gk_sk_combine(float* __restrict__ y, const float* __restrict__ part, int N, int S){
  int o=blockIdx.x*blockDim.x+threadIdx.x; if(o>=N) return;
  float a=0.f;
  for(int s=0;s<S;++s) a+=part[(long)s*N+o];
  y[o]=a;
}


// Four K-blocks in flight. Two measured +5.2% on the two-pass lane (38.30 -> 40.29); this is the same idea
// pushed one step further, and stops there because the tail handling and register pressure both start to
// cost more than the extra loads return. Accumulation order is unchanged, so it is still bit-identical.
__global__ void gk_gemv_hlu4(float* y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
                             const signed char* __restrict__ xh, const signed char* __restrict__ xl,
                             const float* __restrict__ xs, int K, int N){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K);
  const int4* h16=(const int4*)xh; const int4* l16=(const int4*)xl;
  const __half* wsr=ws+(long)o*(K/32);
  float acc=0.f; int nq=K/512; int b=0;
  for(; b+3<nq; b+=4){
    int4 w0=w16[b*32+lane],     h0=h16[b*32+lane],     l0=l16[b*32+lane];
    int4 w1=w16[(b+1)*32+lane], h1=h16[(b+1)*32+lane], l1=l16[(b+1)*32+lane];
    int4 w2=w16[(b+2)*32+lane], h2=h16[(b+2)*32+lane], l2=l16[(b+2)*32+lane];
    int4 w3=w16[(b+3)*32+lane], h3=h16[(b+3)*32+lane], l3=l16[(b+3)*32+lane];
    int k0=b*16+lane/2, k1=(b+1)*16+lane/2, k2=(b+2)*16+lane/2, k3=(b+3)*16+lane/2;
    int a0=__dp4a(w0.x,h0.x,0)+__dp4a(w0.y,h0.y,0)+__dp4a(w0.z,h0.z,0)+__dp4a(w0.w,h0.w,0);
    int c0=__dp4a(w0.x,l0.x,0)+__dp4a(w0.y,l0.y,0)+__dp4a(w0.z,l0.z,0)+__dp4a(w0.w,l0.w,0);
    int a1=__dp4a(w1.x,h1.x,0)+__dp4a(w1.y,h1.y,0)+__dp4a(w1.z,h1.z,0)+__dp4a(w1.w,h1.w,0);
    int c1=__dp4a(w1.x,l1.x,0)+__dp4a(w1.y,l1.y,0)+__dp4a(w1.z,l1.z,0)+__dp4a(w1.w,l1.w,0);
    int a2=__dp4a(w2.x,h2.x,0)+__dp4a(w2.y,h2.y,0)+__dp4a(w2.z,h2.z,0)+__dp4a(w2.w,h2.w,0);
    int c2=__dp4a(w2.x,l2.x,0)+__dp4a(w2.y,l2.y,0)+__dp4a(w2.z,l2.z,0)+__dp4a(w2.w,l2.w,0);
    int a3=__dp4a(w3.x,h3.x,0)+__dp4a(w3.y,h3.y,0)+__dp4a(w3.z,h3.z,0)+__dp4a(w3.w,h3.w,0);
    int c3=__dp4a(w3.x,l3.x,0)+__dp4a(w3.y,l3.y,0)+__dp4a(w3.z,l3.z,0)+__dp4a(w3.w,l3.w,0);
    acc += ((float)a0 + (float)c0*(1.0f/127.0f)) * (__half2float(wsr[k0])*xs[k0]);
    acc += ((float)a1 + (float)c1*(1.0f/127.0f)) * (__half2float(wsr[k1])*xs[k1]);
    acc += ((float)a2 + (float)c2*(1.0f/127.0f)) * (__half2float(wsr[k2])*xs[k2]);
    acc += ((float)a3 + (float)c3*(1.0f/127.0f)) * (__half2float(wsr[k3])*xs[k3]);
  }
  for(; b<nq; b++){
    int4 w=w16[b*32+lane], hh=h16[b*32+lane], ll=l16[b*32+lane]; int blk=b*16+lane/2;
    int dh=__dp4a(w.x,hh.x,0)+__dp4a(w.y,hh.y,0)+__dp4a(w.z,hh.z,0)+__dp4a(w.w,hh.w,0);
    int dl=__dp4a(w.x,ll.x,0)+__dp4a(w.y,ll.y,0)+__dp4a(w.z,ll.z,0)+__dp4a(w.w,ll.w,0);
    acc += ((float)dh + (float)dl*(1.0f/127.0f)) * (__half2float(wsr[blk])*xs[blk]);
  }
  int done=nq*512;
  for(int j=done+lane;j<K;j+=32){ int blk=j/32;
    float a=(float)xh[j] + (float)xl[j]*(1.0f/127.0f);
    acc += a*(float)wq[(long)o*K+j]*(__half2float(wsr[blk])*xs[blk]); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}


// ══ TWO K-BLOCKS IN FLIGHT ══════════════════════════════════════════════════════════════════════
// Widening in N (two output rows per warp) was MEASURED slower on A100 -- 41.10 against 42.28 -- because it
// halves the warp count, and this GEMV needs the parallelism more than it needs the reuse. Unrolling in K
// costs no warps: the same warp issues the loads for two consecutive 512-weight blocks before consuming
// either, so there are twice as many loads in flight per warp with the same occupancy.
//
// The accumulation ORDER is deliberately unchanged -- one accumulator, blocks folded in ascending order --
// so this is bit-identical to the rolled form and cannot move a token.
__global__ void gk_gemv_hu(float* y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
                           const signed char* __restrict__ xq, const float* __restrict__ xs, int K, int N){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq;
  const __half* wsr=ws+(long)o*(K/32);
  float acc=0.f; int nq=K/512; int b=0;
  for(; b+1<nq; b+=2){
    int4 wa=w16[b*32+lane],     xa=x16[b*32+lane];
    int4 wb=w16[(b+1)*32+lane], xb=x16[(b+1)*32+lane];
    int ba=b*16+lane/2, bb=(b+1)*16+lane/2;
    int da=__dp4a(wa.x,xa.x,0)+__dp4a(wa.y,xa.y,0)+__dp4a(wa.z,xa.z,0)+__dp4a(wa.w,xa.w,0);
    int db=__dp4a(wb.x,xb.x,0)+__dp4a(wb.y,xb.y,0)+__dp4a(wb.z,xb.z,0)+__dp4a(wb.w,xb.w,0);
    acc += (float)da * (__half2float(wsr[ba])*xs[ba]);
    acc += (float)db * (__half2float(wsr[bb])*xs[bb]);
  }
  for(; b<nq; b++){
    int4 w=w16[b*32+lane], xx=x16[b*32+lane]; int blk=b*16+lane/2;
    int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
    acc += (float)d * (__half2float(wsr[blk])*xs[blk]);
  }
  int done=nq*512; for(int j=done+lane;j<K;j+=32){ int blk=j/32;
    acc += (float)((int)xq[j]*(int)wq[(long)o*K+j])*(__half2float(wsr[blk])*xs[blk]); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}

// The same unrolling for the two-pass lane, where each iteration already has three streams to keep in flight.
__global__ void gk_gemv_hlu(float* y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
                            const signed char* __restrict__ xh, const signed char* __restrict__ xl,
                            const float* __restrict__ xs, int K, int N){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K);
  const int4* h16=(const int4*)xh; const int4* l16=(const int4*)xl;
  const __half* wsr=ws+(long)o*(K/32);
  float acc=0.f; int nq=K/512; int b=0;
  for(; b+1<nq; b+=2){
    int4 wa=w16[b*32+lane],     ha=h16[b*32+lane],     la=l16[b*32+lane];
    int4 wb=w16[(b+1)*32+lane], hb=h16[(b+1)*32+lane], lb=l16[(b+1)*32+lane];
    int ba=b*16+lane/2, bb=(b+1)*16+lane/2;
    int dha=__dp4a(wa.x,ha.x,0)+__dp4a(wa.y,ha.y,0)+__dp4a(wa.z,ha.z,0)+__dp4a(wa.w,ha.w,0);
    int dla=__dp4a(wa.x,la.x,0)+__dp4a(wa.y,la.y,0)+__dp4a(wa.z,la.z,0)+__dp4a(wa.w,la.w,0);
    int dhb=__dp4a(wb.x,hb.x,0)+__dp4a(wb.y,hb.y,0)+__dp4a(wb.z,hb.z,0)+__dp4a(wb.w,hb.w,0);
    int dlb=__dp4a(wb.x,lb.x,0)+__dp4a(wb.y,lb.y,0)+__dp4a(wb.z,lb.z,0)+__dp4a(wb.w,lb.w,0);
    acc += ((float)dha + (float)dla*(1.0f/127.0f)) * (__half2float(wsr[ba])*xs[ba]);
    acc += ((float)dhb + (float)dlb*(1.0f/127.0f)) * (__half2float(wsr[bb])*xs[bb]);
  }
  for(; b<nq; b++){
    int4 w=w16[b*32+lane], hh=h16[b*32+lane], ll=l16[b*32+lane]; int blk=b*16+lane/2;
    int dh=__dp4a(w.x,hh.x,0)+__dp4a(w.y,hh.y,0)+__dp4a(w.z,hh.z,0)+__dp4a(w.w,hh.w,0);
    int dl=__dp4a(w.x,ll.x,0)+__dp4a(w.y,ll.y,0)+__dp4a(w.z,ll.z,0)+__dp4a(w.w,ll.w,0);
    acc += ((float)dh + (float)dl*(1.0f/127.0f)) * (__half2float(wsr[blk])*xs[blk]);
  }
  int done=nq*512;
  for(int j=done+lane;j<K;j+=32){ int blk=j/32;
    float a=(float)xh[j] + (float)xl[j]*(1.0f/127.0f);
    acc += a*(float)wq[(long)o*K+j]*(__half2float(wsr[blk])*xs[blk]); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}


// ══ TWO OUTPUT ROWS PER WARP ════════════════════════════════════════════════════════════════════
// A batch-1 GEMV at these shapes is not bandwidth-saturated -- it is LATENCY bound: one row per warp gives
// each warp a single outstanding weight load per iteration, and the measured 1050 GB/s against an A100 that
// delivers ~1500 is the gap that leaves. Giving a warp two adjacent rows doubles the loads in flight, and
// the activation word (and its scale) is fetched once and used twice, which halves the L2 traffic the
// activation generates across the 7760 warps that read it.
__global__ void gk_gemv_h2(float* y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
                           const signed char* __restrict__ xq, const float* __restrict__ xs, int K, int N){
  int r=((blockIdx.x*blockDim.x+threadIdx.x)>>5)*2; if(r>=N) return; int lane=threadIdx.x&31;
  bool r1 = (r+1)<N;
  const int4* x16=(const int4*)xq;
  const int4* wa=(const int4*)(wq+(long)r*K);
  const int4* wb=(const int4*)(wq+(long)(r+1)*K);
  const __half* sa=ws+(long)r*(K/32); const __half* sb=ws+(long)(r+1)*(K/32);
  float acc0=0.f, acc1=0.f; int nq=K/512;
  for(int b=0;b<nq;b++){
    int4 xx=x16[b*32+lane]; int blk=b*16+lane/2; float xsv=xs[blk];
    int4 w0=wa[b*32+lane];
    int d0=__dp4a(w0.x,xx.x,0)+__dp4a(w0.y,xx.y,0)+__dp4a(w0.z,xx.z,0)+__dp4a(w0.w,xx.w,0);
    acc0 += (float)d0 * (__half2float(sa[blk])*xsv);
    if(r1){ int4 w1=wb[b*32+lane];
      int d1=__dp4a(w1.x,xx.x,0)+__dp4a(w1.y,xx.y,0)+__dp4a(w1.z,xx.z,0)+__dp4a(w1.w,xx.w,0);
      acc1 += (float)d1 * (__half2float(sb[blk])*xsv); }
  }
  int done=nq*512;
  for(int j=done+lane;j<K;j+=32){ int blk=j/32; float xv=(float)xq[j]*xs[blk];
    acc0 += xv*(float)wq[(long)r*K+j]*__half2float(sa[blk]);
    if(r1) acc1 += xv*(float)wq[(long)(r+1)*K+j]*__half2float(sb[blk]); }
  for(int s=16;s>0;s>>=1){ acc0+=__shfl_down_sync(0xffffffffu,acc0,s); acc1+=__shfl_down_sync(0xffffffffu,acc1,s); }
  if(lane==0){ y[r]=acc0; if(r1) y[r+1]=acc1; }
}

// The same widening for the two-pass lane, where it pays double: the activation's HIGH and LOW words are both
// fetched once per iteration and used for both rows.
__global__ void gk_gemv_hl2(float* y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
                            const signed char* __restrict__ xh, const signed char* __restrict__ xl,
                            const float* __restrict__ xs, int K, int N){
  int r=((blockIdx.x*blockDim.x+threadIdx.x)>>5)*2; if(r>=N) return; int lane=threadIdx.x&31;
  bool r1 = (r+1)<N;
  const int4* h16=(const int4*)xh; const int4* l16=(const int4*)xl;
  const int4* wa=(const int4*)(wq+(long)r*K);
  const int4* wb=(const int4*)(wq+(long)(r+1)*K);
  const __half* sa=ws+(long)r*(K/32); const __half* sb=ws+(long)(r+1)*(K/32);
  float acc0=0.f, acc1=0.f; int nq=K/512;
  for(int b=0;b<nq;b++){
    int4 hh=h16[b*32+lane], ll=l16[b*32+lane]; int blk=b*16+lane/2; float xsv=xs[blk];
    int4 w0=wa[b*32+lane];
    int dh0=__dp4a(w0.x,hh.x,0)+__dp4a(w0.y,hh.y,0)+__dp4a(w0.z,hh.z,0)+__dp4a(w0.w,hh.w,0);
    int dl0=__dp4a(w0.x,ll.x,0)+__dp4a(w0.y,ll.y,0)+__dp4a(w0.z,ll.z,0)+__dp4a(w0.w,ll.w,0);
    acc0 += ((float)dh0 + (float)dl0*(1.0f/127.0f)) * (__half2float(sa[blk])*xsv);
    if(r1){ int4 w1=wb[b*32+lane];
      int dh1=__dp4a(w1.x,hh.x,0)+__dp4a(w1.y,hh.y,0)+__dp4a(w1.z,hh.z,0)+__dp4a(w1.w,hh.w,0);
      int dl1=__dp4a(w1.x,ll.x,0)+__dp4a(w1.y,ll.y,0)+__dp4a(w1.z,ll.z,0)+__dp4a(w1.w,ll.w,0);
      acc1 += ((float)dh1 + (float)dl1*(1.0f/127.0f)) * (__half2float(sb[blk])*xsv); }
  }
  int done=nq*512;
  for(int j=done+lane;j<K;j+=32){ int blk=j/32;
    float a=((float)xh[j] + (float)xl[j]*(1.0f/127.0f))*xs[blk];
    acc0 += a*(float)wq[(long)r*K+j]*__half2float(sa[blk]);
    if(r1) acc1 += a*(float)wq[(long)(r+1)*K+j]*__half2float(sb[blk]); }
  for(int s=16;s>0;s>>=1){ acc0+=__shfl_down_sync(0xffffffffu,acc0,s); acc1+=__shfl_down_sync(0xffffffffu,acc1,s); }
  if(lane==0){ y[r]=acc0; if(r1) y[r+1]=acc1; }
}


// ══ TWO-PASS INT8 ACTIVATIONS ═══════════════════════════════════════════════════════════════════
// A batch-1 GEMV reads a whole weight matrix and ONE activation vector, so the activation costs no
// measurable bandwidth -- but rounding it onto a single int8 grid costs accuracy at every one of the 300+
// projections in a token. Splitting it into a high and a low int8 word represents it as
//     a  ~=  s * (qh + ql/127),      | a/s - (qh + ql/127) |  <=  0.5/127
// which is 254x tighter, and keeps `dp4a`. That last part is the point: the obvious alternative -- feeding
// the GEMV f32 activations -- was MEASURED at 32.27 tok/s against 42.28, because one int-to-float convert
// per weight is quarter rate on this chip. Two words of int8 double the dp4a count instead, on a kernel
// that is memory bound, and the weight matrix is still read exactly once.

__global__ void gk_gemv_hl(float* y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
                           const signed char* __restrict__ xh, const signed char* __restrict__ xl,
                           const float* __restrict__ xs, int K, int N){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K);
  const int4* h16=(const int4*)xh; const int4* l16=(const int4*)xl;
  const __half* wsr=ws+(long)o*(K/32);
  float acc=0.f; int nq=K/512;
  for(int b=0;b<nq;b++){
    int4 w=w16[b*32+lane], hh=h16[b*32+lane], ll=l16[b*32+lane];
    int blk=b*16+lane/2;
    int dh=__dp4a(w.x,hh.x,0)+__dp4a(w.y,hh.y,0)+__dp4a(w.z,hh.z,0)+__dp4a(w.w,hh.w,0);
    int dl=__dp4a(w.x,ll.x,0)+__dp4a(w.y,ll.y,0)+__dp4a(w.z,ll.z,0)+__dp4a(w.w,ll.w,0);
    acc += ((float)dh + (float)dl*(1.0f/127.0f)) * (__half2float(wsr[blk])*xs[blk]);
  }
  int done=nq*512;
  for(int j=done+lane;j<K;j+=32){ int blk=j/32;
    float a=(float)xh[j] + (float)xl[j]*(1.0f/127.0f);
    acc += a*(float)wq[(long)o*K+j]*(__half2float(wsr[blk])*xs[blk]); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}

// The producers, each the single-word kernel with its final rounding replaced by the two-word split. The
// scale and every float value are computed exactly as before, so the high word is bit-identical to what the
// single-word lane produced and the low word is pure addition.
__global__ void gk_quant_act_hl(const float* x, signed char* xq, signed char* xl, float* xs, int K){
  int blk=blockIdx.x*(blockDim.x/32)+threadIdx.x/32; if((long)blk*32>=K) return; int lane=threadIdx.x&31;
  float v=x[blk*32+lane], a=fabsf(v);
  for(int s=16;s>0;s>>=1) a=fmaxf(a,__shfl_down_sync(0xffffffffu,a,s)); a=__shfl_sync(0xffffffffu,a,0);
  float sc=a>0?a/127.0f:1.0f; if(lane==0) xs[blk]=sc;
  lgx_q8hl(v/sc, xq, xl, (long)blk*32+lane);
}
__global__ void gk_rmsnorm_q8f_hl(const float* __restrict__ x, signed char* __restrict__ xq,
                                  signed char* __restrict__ xl, float* __restrict__ xs,
                                  float* __restrict__ xf, const float* __restrict__ w, int n, float eps){
  __shared__ float sh[256];
  float s=0.f; for(int i=threadIdx.x;i<n;i+=blockDim.x) s+=x[i]*x[i];
  sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  float sc=rsqrtf(sh[0]/n+eps); __syncthreads();
  for(int i=threadIdx.x;i<n;i+=blockDim.x) xf[i]=x[i]*sc*w[i];
  __syncthreads();
  int nb=n/32;
  for(int b=threadIdx.x;b<nb;b+=blockDim.x){
    float amax=1e-30f;
    for(int j=0;j<32;++j){ float a=fabsf(xf[b*32+j]); if(a>amax) amax=a; }
    float q=amax/127.0f; xs[b]=q;
    for(int j=0;j<32;++j) lgx_q8hl(xf[b*32+j]/q, xq, xl, (long)b*32+j);
  }
}
__global__ void gk_add_rmsnorm_q8f_hl(float* __restrict__ x, const float* __restrict__ y,
                                      signed char* __restrict__ xq, signed char* __restrict__ xl,
                                      float* __restrict__ xs, float* __restrict__ xf,
                                      const float* __restrict__ w, int n, float eps){
  __shared__ float sh[256];
  float s=0.f;
  for(int i=threadIdx.x;i<n;i+=blockDim.x){ float v=x[i]+y[i]; x[i]=v; s+=v*v; }
  sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  float sc=rsqrtf(sh[0]/n+eps); __syncthreads();
  for(int i=threadIdx.x;i<n;i+=blockDim.x) xf[i]=x[i]*sc*w[i];
  __syncthreads();
  int nb=n/32;
  for(int b=threadIdx.x;b<nb;b+=blockDim.x){
    float amax=1e-30f;
    for(int j=0;j<32;++j){ float a=fabsf(xf[b*32+j]); if(a>amax) amax=a; }
    float q=amax/127.0f; xs[b]=q;
    for(int j=0;j<32;++j) lgx_q8hl(xf[b*32+j]/q, xq, xl, (long)b*32+j);
  }
}
__global__ void gk_add_rmsnorm_q8_hl(float* x, const float* y, float* out, const float* w,
                                     signed char* oq, signed char* ol, float* os_, int n, float eps){
  __shared__ float sh[256]; float s=0; for(int i=threadIdx.x;i<n;i+=blockDim.x){ float v=x[i]+y[i]; x[i]=v; s+=v*v; }
  sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  float scale=rsqrtf(sh[0]/n+eps); for(int i=threadIdx.x;i<n;i+=blockDim.x) out[i]=x[i]*scale*w[i];
  __syncthreads();
  int warp=threadIdx.x>>5, lane=threadIdx.x&31, nb=n/32;
  for(int b=warp;b<nb;b+=(blockDim.x>>5)){ float v=out[b*32+lane], av=fabsf(v);
    for(int t=16;t>0;t>>=1) av=fmaxf(av,__shfl_down_sync(0xffffffffu,av,t));
    av=__shfl_sync(0xffffffffu,av,0); float amax=av>1e-30f?av:1e-30f; float qs=amax/127.0f;
    if(lane==0) os_[b]=qs; lgx_q8hl(v/qs, oq, ol, (long)b*32+lane); }
}
__global__ void gk_silu_had_q8_hl(const float* __restrict__ g, const float* __restrict__ u,
                                  signed char* __restrict__ hq, signed char* __restrict__ hl_,
                                  float* __restrict__ hs, int n){
  int blk = blockIdx.x * blockDim.x + threadIdx.x; int nb = n / 32; if(blk >= nb) return;
  float amax = 1e-30f; float t[32];
  for(int j = 0; j < 32; ++j){ float gv = g[blk*32+j]; float h = (gv / (1.0f + __expf(-gv))) * u[blk*32+j];
                               t[j] = h; float a = fabsf(h); if(a > amax) amax = a; }
  float sc = amax / 127.0f; hs[blk] = sc;
  for(int j = 0; j < 32; ++j) lgx_q8hl(t[j]/sc, hq, hl_, (long)blk*32+j);
}


// The Q8_0 GEMV that keeps its ACTIVATION in f32: the weights are still int8 with a __half block scale, so
// the memory traffic — the thing that decides batch-1 decode time — is byte for byte the same as the dp4a
// form. Only the arithmetic changes, from a quarter of a dp4a per weight to about three float ops, which
// leaves the kernel comfortably memory bound. What it removes is the activation quantization error, which at
// batch 1 was never buying anything: one activation vector against a whole weight matrix is not where the
// bandwidth goes.
__global__ void gk_gemv_deq_h(float* y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
                              const float* __restrict__ x, int K, int N){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const __half* wsr=ws+(long)o*(K/32);
  const float4* x4=(const float4*)x;
  float acc=0.f; int nq=K/512;
  // The four weight words stay in REGISTERS and are unpacked by shifting. Reading them back through a
  // `(signed char*)&wv` alias instead puts the int4 in local memory, and the resulting spill cost 12% of
  // throughput -- on a kernel whose whole premise was that it would be free.
#define LGX_W(W,S) ((float)(int)(signed char)(((W)>>(S))&0xff))
  for(int b=0;b<nq;b++){
    int4 wv=w16[b*32+lane];
    int base4=b*128+lane*4;
    float4 a0=x4[base4], a1=x4[base4+1], a2=x4[base4+2], a3=x4[base4+3];
    float p = LGX_W(wv.x,0)*a0.x + LGX_W(wv.x,8)*a0.y + LGX_W(wv.x,16)*a0.z + LGX_W(wv.x,24)*a0.w
            + LGX_W(wv.y,0)*a1.x + LGX_W(wv.y,8)*a1.y + LGX_W(wv.y,16)*a1.z + LGX_W(wv.y,24)*a1.w
            + LGX_W(wv.z,0)*a2.x + LGX_W(wv.z,8)*a2.y + LGX_W(wv.z,16)*a2.z + LGX_W(wv.z,24)*a2.w
            + LGX_W(wv.w,0)*a3.x + LGX_W(wv.w,8)*a3.y + LGX_W(wv.w,16)*a3.z + LGX_W(wv.w,24)*a3.w;
    acc += p*__half2float(wsr[b*16+lane/2]);
  }
#undef LGX_W
  int done=nq*512;
  for(int j=done+lane;j<K;j+=32){ int blk=j/32; acc += (float)wq[(long)o*K+j]*__half2float(wsr[blk])*x[j]; }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}


// The Q8_0 GEMV with the block scale read as __half — identical arithmetic, 2 bytes per 32 weights instead
// of 4. At batch 1 the whole decode is a stream of these, so the 5.6% of weight traffic this returns is
// 5.6% off the token time. The scale was an f16 in the checkpoint; widening it on the way to the GPU only
// ever cost bandwidth.
__global__ void gk_gemv_h(float* y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
                          const signed char* __restrict__ xq, const float* __restrict__ xs, int K, int N){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return; int lane=threadIdx.x&31;
  const int4* w16=(const int4*)(wq+(long)o*K); const int4* x16=(const int4*)xq; const __half* wsr=ws+(long)o*(K/32);
  float acc=0.f; int nq=K/512;
  for(int b=0;b<nq;b++){ int4 w=w16[b*32+lane], xx=x16[b*32+lane]; int blk=b*16+lane/2;
    int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
    acc += (float)d * (__half2float(wsr[blk])*xs[blk]); }
  int done=nq*512; for(int j=done+lane;j<K;j+=32){ int blk=j/32;
    acc += (float)((int)xq[j]*(int)wq[(long)o*K+j])*(__half2float(wsr[blk])*xs[blk]); }
  for(int s=16;s>0;s>>=1) acc+=__shfl_down_sync(0xffffffffu,acc,s); if(lane==0) y[o]=acc;
}
__global__ void gk_embed_h(float* x, const signed char* wq, const __half* ws, const int* d_tok, int E){
  int t=blockIdx.x*blockDim.x+threadIdx.x; if(t>=E) return;
  long tok=*d_tok; x[t]=(float)wq[tok*E+t]*__half2float(ws[tok*(E/32)+t/32]);
}

// RMSNorm that emits BOTH the int8-quantized activation (for the big Q8_0 GEMVs) and the f32 normed vector
// (for GDN's two small f32-weight gate projections). Emitting both from one pass costs a 20 KB store and
// saves re-reading and re-normalizing the hidden state.
// The same, with the RESIDUAL ADD fused in. Every layer but the first is entered from a residual add, and
// the add is a full pass over the hidden state that the norm is about to read again — so the layer entry
// folds `x += y` into the reduction it already does. Removes one launch and one round-trip per layer.
__global__ void gk_add_rmsnorm_q8f(float* __restrict__ x, const float* __restrict__ y,
                                   signed char* __restrict__ xq, float* __restrict__ xs,
                                   float* __restrict__ xf, const float* __restrict__ w, int n, float eps){
  __shared__ float sh[256];
  float s = 0.f;
  for(int i = threadIdx.x; i < n; i += blockDim.x){ float v = x[i] + y[i]; x[i] = v; s += v * v; }
  sh[threadIdx.x] = s; __syncthreads();
  for(int k = blockDim.x / 2; k > 0; k >>= 1){ if(threadIdx.x < k) sh[threadIdx.x] += sh[threadIdx.x + k]; __syncthreads(); }
  float sc = rsqrtf(sh[0] / n + eps); __syncthreads();
  for(int i = threadIdx.x; i < n; i += blockDim.x) xf[i] = x[i] * sc * w[i];
  __syncthreads();
  int nb = n / 32;
  for(int b = threadIdx.x; b < nb; b += blockDim.x){
    float amax = 1e-30f;
    for(int j = 0; j < 32; ++j){ float a = fabsf(xf[b*32+j]); if(a > amax) amax = a; }
    float q = amax / 127.0f; xs[b] = q;
    for(int j = 0; j < 32; ++j) xq[b*32+j] = (signed char)__float2int_rn(xf[b*32+j] / q);
  }
}
__global__ void gk_rmsnorm_q8f(const float* __restrict__ x, signed char* __restrict__ xq, float* __restrict__ xs,
                               float* __restrict__ xf, const float* __restrict__ w, int n, float eps){
  __shared__ float sh[256];
  float s = 0.f; for(int i = threadIdx.x; i < n; i += blockDim.x) s += x[i] * x[i];
  sh[threadIdx.x] = s; __syncthreads();
  for(int k = blockDim.x / 2; k > 0; k >>= 1){ if(threadIdx.x < k) sh[threadIdx.x] += sh[threadIdx.x + k]; __syncthreads(); }
  float sc = rsqrtf(sh[0] / n + eps); __syncthreads();
  for(int i = threadIdx.x; i < n; i += blockDim.x) xf[i] = x[i] * sc * w[i];
  __syncthreads();
  int nb = n / 32;
  for(int b = threadIdx.x; b < nb; b += blockDim.x){
    float amax = 1e-30f;
    for(int j = 0; j < 32; ++j){ float a = fabsf(xf[b*32+j]); if(a > amax) amax = a; }
    float q = amax / 127.0f; xs[b] = q;
    for(int j = 0; j < 32; ++j) xq[b*32+j] = (signed char)__float2int_rn(xf[b*32+j] / q);
  }
}
// Diagnostic: L2 norm of a vector, so a debug build can report where the hidden state collapses or explodes.
__global__ void gk_dbg3(const float* __restrict__ v, float* __restrict__ out){
  if(threadIdx.x < 3) out[threadIdx.x] = v[threadIdx.x];
}
__global__ void gk_l2probe(const float* __restrict__ v, int n, float* __restrict__ out){
  __shared__ float sh[256]; float s = 0.f;
  for(int i = threadIdx.x; i < n; i += blockDim.x) s += v[i] * v[i];
  sh[threadIdx.x] = s; __syncthreads();
  for(int k = blockDim.x / 2; k > 0; k >>= 1){ if(threadIdx.x < k) sh[threadIdx.x] += sh[threadIdx.x + k]; __syncthreads(); }
  if(threadIdx.x == 0) *out = sqrtf(sh[0]);
}
__global__ void gk_gemv_f32w(float* __restrict__ y, const float* __restrict__ w,
                             const float* __restrict__ x, int K, int N){
  int o = (blockIdx.x * blockDim.x + threadIdx.x) >> 5; if(o >= N) return;
  int lane = threadIdx.x & 31;
  const float* wr = w + (long)o * K;
  float acc = 0.f;
  for(int j = lane; j < K; j += 32) acc += wr[j] * x[j];
  for(int s = 16; s > 0; s >>= 1) acc += __shfl_down_sync(0xffffffffu, acc, s);
  if(lane == 0) y[o] = acc;
}
// Split the fused projection's k|v halves into their caches at the current position.
__global__ void gk_kv_store(float* __restrict__ kc, float* __restrict__ vc, const int* __restrict__ d_pos,
                            int KD, const float* __restrict__ kv){
  int i = blockIdx.x * blockDim.x + threadIdx.x; if(i >= 2 * KD) return;
  long base = (long)(*d_pos) * KD;
  if(i < KD) kc[base + i] = kv[i]; else vc[base + (i - KD)] = kv[i];
}
// silu(gate) * up, with the int8 re-quantization fused into the epilogue.
// The same SwiGLU epilogue, but leaving the result in f32 instead of an int8 grid. Used by the deq lane,
// where the projection that consumes it reads f32 activations.
__global__ void gk_silu_had_f(const float* __restrict__ g, const float* __restrict__ u,
                              float* __restrict__ out, int n){
  int i = blockIdx.x * blockDim.x + threadIdx.x; if(i >= n) return;
  float gv = g[i];
  out[i] = (gv / (1.0f + __expf(-gv))) * u[i];
}
__global__ void gk_silu_had_q8(const float* __restrict__ g, const float* __restrict__ u,
                               signed char* __restrict__ hq, float* __restrict__ hs, int n){
  int blk = blockIdx.x * blockDim.x + threadIdx.x; int nb = n / 32; if(blk >= nb) return;
  float amax = 1e-30f; float t[32];
  for(int j = 0; j < 32; ++j){ float gv = g[blk*32+j]; float h = (gv / (1.0f + __expf(-gv))) * u[blk*32+j];
                               t[j] = h; float a = fabsf(h); if(a > amax) amax = a; }
  float sc = amax / 127.0f; hs[blk] = sc;
  for(int j = 0; j < 32; ++j) hq[blk*32+j] = (signed char)__float2int_rn(t[j] / sc);
}
"#;

/// Emit the MTP ACCEPT-RATE PROBE.
///
/// Multi-token prediction only pays if the draft head is usually right: the speedup is
/// `(1+alpha) * full / (full + draft)`, and with the draft costing 7% of a forward that is 1.24x at
/// alpha=0.33 and 1.59x at alpha=0.70. The expensive half of MTP is the two-token VERIFY step — GEMVs batched
/// over two positions while the recurrence still advances sequentially — so it is worth measuring alpha
/// BEFORE building it, not after.
///
/// This probe does exactly that and nothing more. It walks the known golden continuation, and at each step
/// runs the draft head on (last hidden state, token just produced) and asks whether it predicted the NEXT
/// golden token. No verification path, no speculation machinery — just the accept rate, which is the number
/// that decides whether the rest is worth writing.
///
/// The draft must not disturb the decode it speculates on: it has its own KV cache and uses an argmax that
/// does not advance the position.
pub fn emit_qwen38_mtp_probe(
    cfg: &Qwen38Cfg,
    wdir: &str,
    prompt: &[usize],
    golden: &[usize],
) -> String {
    let base = emit_qwen38_generate(cfg, wdir, prompt, golden.len(), true, 3);
    let (d, dff) = (cfg.d, cfg.dff);
    let (aq, akv) = (cfg.aq(), cfg.akv());
    let attnw = 2 * aq + 2 * akv;
    let l = cfg.layers; // the MTP block index, 64
    let two_d = 2 * d;

    // weights the draft head needs, on top of everything the main model already loaded
    let mut decls = String::new();
    let mut loads = String::new();
    for (v, rows, inn, f) in [
        (format!("qgkv{l}"), attnw, d, format!("qgkv{l}")),
        (format!("Wo{l}"), d, aq, format!("Wo{l}")),
        (format!("gu{l}"), 2 * dff, d, format!("gu{l}")),
        (format!("Wd{l}"), d, dff, format!("Wd{l}")),
        ("ehproj".into(), d, two_d, "ehproj".into()),
    ] {
        decls.push_str(&format!(
            "static signed char* {v}_q; static __half* {v}_s;\n"
        ));
        loads.push_str(&format!(
            "  LDQ({v},(size_t){rows}*{inn},(size_t){rows}*({inn}/32),\"{wdir}/{f}.q\",\"{wdir}/{f}.s\");\n"));
    }
    for (v, n, f) in [
        (format!("an{l}"), d, format!("an{l}")),
        (format!("fn{l}"), d, format!("fn{l}")),
        (format!("Qn{l}"), cfg.hd, format!("Qn{l}")),
        (format!("Kn{l}"), cfg.hd, format!("Kn{l}")),
        ("enorm".into(), d, "enorm".into()),
        ("hnorm".into(), d, "hnorm".into()),
        ("shnorm".into(), d, "shnorm".into()),
    ] {
        decls.push_str(&format!("static float* {v}_f;\n"));
        loads.push_str(&format!("  LDF({v},(size_t){n},\"{wdir}/{f}.f\");\n"));
    }

    let draft = format!(
"static float *mx,*mtmp,*m2n,*mcat,*membed,*mf32,*dlogits,*mkc,*mvc;
static signed char *maq,*maql; static float *mas;
static int *d_draft;
{decls}
// One draft step: the nextn adapter, then blk.{l} as an ordinary gated-attention layer + FFN, then the
// model's own output head. `x` is the main model's residual stream — the hidden state MTP conditions on.
static void mtp_draft(cudaStream_t s){{
  gk_embed_h<<<Gs(E),256,0,s>>>(membed,emb_q,emb_s,d_tok,E);
  gk_mtp_cat<<<2,256,0,s>>>(mcat,membed,x,enorm_f,hnorm_f,E,EPSV);
  gk_quant_act_hl<<<Gs({two_d}),256,0,s>>>(mcat,maq,maql,mas,{two_d});
  gk_gemv_hl<<<Gw(E),256,0,s>>>(mx,ehproj_q,ehproj_s,maq,maql,mas,{two_d},E);
  gk_rmsnorm_q8f_hl<<<1,256,0,s>>>(mx,maq,maql,mas,mf32,an{l}_f,E,EPSV);
  gk_gemv_hl<<<Gw({attnw}),256,0,s>>>(pbuf,qgkv{l}_q,qgkv{l}_s,maq,maql,mas,E,{attnw});
  gk_kv_store<<<Gs({akv}*2),256,0,s>>>(mkc,mvc,d_pos,{akv},pbuf+{two_aq});
  gk_qk_norm_rope_part<<<{h},128,0,s>>>(pbuf,d_zero,0,Qn{l}_f,{h},{hd},{rot},ROPEV,EPSV);
  gk_qk_norm_rope_part<<<{kv},128,0,s>>>(mkc,d_pos,{akv},Kn{l}_f,{kv},{hd},{rot},ROPEV,EPSV);
  gk_attn<<<{h},256,(size_t)MAXSEQ*4,s>>>(attnb,pbuf,mkc,mvc,d_pos,{h},{kv},{hd},SCALEV);
  gk_gate_mul<<<Gs({aq}),256,0,s>>>(attnb,pbuf+{aq},{aq});
  gk_quant_act_hl<<<Gs({aq}),256,0,s>>>(attnb,maq,maql,mas,{aq});
  gk_gemv_hl<<<Gw(E),256,0,s>>>(mtmp,Wo{l}_q,Wo{l}_s,maq,maql,mas,{aq},E);
  gk_add_rmsnorm_q8_hl<<<1,256,0,s>>>(mx,mtmp,m2n,fn{l}_f,maq,maql,mas,E,EPSV);
  gk_gemv_hl<<<Gw({two_dff}),256,0,s>>>(ffbuf,gu{l}_q,gu{l}_s,maq,maql,mas,E,{two_dff});
  gk_silu_had_q8_hl<<<Gs({dff}),256,0,s>>>(ffbuf,ffbuf+{dff},maq,maql,mas,{dff});
  gk_gemv_hl<<<Gw(E),256,0,s>>>(mtmp,Wd{l}_q,Wd{l}_s,maq,maql,mas,{dff},E);
  gk_add<<<Gs(E),256,0,s>>>(mx,mtmp,E);
  gk_rmsnorm_q8f_hl<<<1,256,0,s>>>(mx,maq,maql,mas,mf32,shnorm_f,E,EPSV);
  gk_gemv_hl<<<Gw(VOCAB),256,0,s>>>(dlogits,Wout_q,Wout_s,maq,maql,mas,E,VOCAB);
  gk_argmax_only<<<1,256,0,s>>>(dlogits,VOCAB,d_draft);
}}
",
        two_aq = 2 * aq, two_dff = 2 * dff, h = cfg.h, kv = cfg.kv, hd = cfg.hd, rot = cfg.rope_dim);

    let mscratch = two_d.max(dff).max(aq);
    let alloc = format!(
"  cudaMalloc(&mx,(size_t)E*4); cudaMalloc(&mtmp,(size_t)E*4); cudaMalloc(&m2n,(size_t)E*4);
  cudaMalloc(&mcat,(size_t){two_d}*4); cudaMalloc(&membed,(size_t)E*4); cudaMalloc(&mf32,(size_t)E*4);
  cudaMalloc(&dlogits,(size_t)VOCAB*4); cudaMalloc(&d_draft,4);
  cudaMalloc(&mkc,(size_t)MAXSEQ*{akv}*4); cudaMalloc(&mvc,(size_t)MAXSEQ*{akv}*4);
  cudaMalloc(&maq,(size_t){mscratch}); cudaMalloc(&maql,(size_t){mscratch}); cudaMalloc(&mas,(size_t)({mscratch}/32)*4);
{loads}");

    // splice the draft into the generated program, and replace its generation loop with the probe
    let mut out = base.replace(
        "static void decode_step(cudaStream_t s){",
        &format!("{draft}\nstatic void decode_step(cudaStream_t s){{"),
    );
    out = out.replace(
        "  cudaStream_t s; cudaStreamCreate(&s);",
        &format!("{alloc}\n  cudaStream_t s; cudaStreamCreate(&s);"),
    );
    let gl: Vec<String> = golden.iter().map(|t| t.to_string()).collect();
    let probe = format!(
"  static const int GOLD[] = {{{}}}; const int NG = sizeof(GOLD)/sizeof(int);
  int accepted = 0, checked = 0;
  for(int i=0;i<NG;++i){{
    // the draft head speculates on the token the main model just produced
    mtp_draft(s);
    if(cudaStreamSynchronize(s)!=cudaSuccess){{ fprintf(stderr,\"MTP DRAFT FAILED: %s\\n\",cudaGetErrorString(cudaGetLastError())); return 1; }}
    int dt=0, mt=0; cudaMemcpy(&dt,d_draft,4,cudaMemcpyDeviceToHost);
    cudaMemcpy(&mt,d_tok,4,cudaMemcpyDeviceToHost);
    if(mt!=GOLD[i]){{ fprintf(stderr,\"MAIN MODEL DIVERGED at %d: got %d want %d — accept rate would be meaningless\\n\",i,mt,GOLD[i]); return 1; }}
    if(i+1<NG){{ ++checked; if(dt==GOLD[i+1]) ++accepted;
      printf(\"step %2d: main=%d  draft=%d  next_golden=%d  %s\\n\", i, mt, dt, GOLD[i+1],
             dt==GOLD[i+1] ? \"ACCEPT\" : \"reject\"); }}
    if(i+1<NG){{ cudaGraphLaunch(ge,s); cudaStreamSynchronize(s); }}
  }}
  printf(\"MTP_ACCEPT_RATE %d/%d = %.3f\\n\", accepted, checked, checked? (double)accepted/checked : 0.0);
  {{ double f=25.85, dr=1.80, a = checked? (double)accepted/checked : 0.0;
     printf(\"MTP_PROJECTED_SPEEDUP %.3fx  (draft is %.1f%% of a forward)\\n\", (1+a)*f/(f+dr), 100*dr/f); }}
", gl.join(","));
    // cut the original generation loop, keep everything up to it
    if let Some(cut) = out.find("  printf(\"GENERATED_TOKENS:") {
        let end = out[cut..]
            .find("\n  {")
            .map(|e| cut + e)
            .unwrap_or(out.len());
        out.replace_range(cut..end, &probe);
    }
    out
}
