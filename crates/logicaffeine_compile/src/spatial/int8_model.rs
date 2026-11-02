//! An int8 transformer as a LOGOS decode step: one token in, its logits and the next token out, the KV cache carried.
//!
//! The model is integer arithmetic throughout — int8 weights, a requant `(acc · M) >> S` with int8 saturation between
//! layers, an exp-table softmax whose context is a floored weighted mean, a ReLU FFN, a tied head and an arg-max — so
//! the step is written in the integer tensor sugar (`X times M shifted right by S`, `X clamped between LO and HI`,
//! `relu of X`, `lut attention of …`) beside the shared forms (`row tok of E`, `matmul`, `plus`, `Set row`,
//! `argmax of`). The parser desugars every one into ordinary loops, so the VM's run of the step IS its
//! meaning, and the spatial compiler lowers the same text onto the integer fabric.

use std::collections::BTreeMap;

use super::tensor_parallel::{SliceAxis, TpCollective, TpManifest, TpSlice};

/// A one-layer int8 transformer: its shapes, its requant pairs, its tables.
#[derive(Clone, Debug)]
pub struct Int8Model {
    pub d: usize,
    pub heads: usize,
    pub d_ff: usize,
    pub vocab: usize,
    /// Positions the cache holds — the positional table's rows.
    pub ctx: usize,
    /// Each requant's `(M, S)`, by its name in the model (`r_qkv`, `M_q`, `M_score`, …).
    pub requant: BTreeMap<String, (i64, u32)>,
    /// The int8 weights, row-major, by name: `Wq Wk Wv Wo` (`d × d`), `W1` (`d × d_ff`), `W2` (`d_ff × d`), `Eq`
    /// (`vocab × d`, the tied embedding the head multiplies by its transpose).
    pub weights: BTreeMap<String, Vec<i64>>,
    /// The token embedding as the stream reads it (`vocab × d`).
    pub embed: Vec<i64>,
    /// The positional embedding (`ctx × d`).
    pub positions: Vec<i64>,
    /// The softmax's exp table, indexed by the requanted distance from the row's maximum.
    pub exp_lut: Vec<i64>,
}

const REQUANTS: [&str; 12] = [
    "r_qkv", "r_ffn", "r_out", "r_ctx", "r_hid", "M_q", "M_k", "M_v", "M_o", "M_h", "M_f", "M_score",
];
const WEIGHTS: [&str; 7] = ["Wq", "Wk", "Wv", "Wo", "W1", "W2", "Eq"];

impl Int8Model {
    /// The model `scripts/train_tiny_transformer.py` exports (`benchmarks/silicon/tiny_transformer.json`).
    pub fn from_json(j: &serde_json::Value) -> Result<Int8Model, String> {
        let size = |k: &str| -> Result<usize, String> {
            j["config"][k]
                .as_u64()
                .map(|v| v as usize)
                .ok_or_else(|| format!("config.{k} is not a size"))
        };
        let ints = |v: &serde_json::Value, what: &str| -> Result<Vec<i64>, String> {
            v.as_array()
                .ok_or_else(|| format!("{what} is not an array"))?
                .iter()
                .map(|x| x.as_i64().ok_or_else(|| format!("{what} holds a non-integer")))
                .collect()
        };
        let (d, heads, d_ff, vocab, ctx) = (
            size("d_model")?,
            size("n_heads")?,
            size("d_ff")?,
            size("vocab")?,
            size("seq_len")?,
        );
        if heads == 0 || d % heads != 0 {
            return Err(format!("{heads} heads do not divide d_model {d}"));
        }
        let mut requant = BTreeMap::new();
        for k in REQUANTS {
            let p = ints(&j["requant"][k], &format!("requant.{k}"))?;
            let [m, s] = p[..] else {
                return Err(format!("requant.{k} is not an (M, S) pair"));
            };
            let s = u32::try_from(s)
                .ok()
                .filter(|s| *s <= 63)
                .ok_or_else(|| format!("requant.{k}'s shift {s} is outside 0..=63"))?;
            requant.insert(k.to_string(), (m, s));
        }
        let mut weights = BTreeMap::new();
        for (k, len) in WEIGHTS.into_iter().zip([d * d, d * d, d * d, d * d, d * d_ff, d_ff * d, vocab * d]) {
            let w = ints(&j["weights_int8"][k], &format!("weights_int8.{k}"))?;
            if w.len() != len {
                return Err(format!("weights_int8.{k} holds {} values, not {len}", w.len()));
            }
            weights.insert(k.to_string(), w);
        }
        let embed = ints(&j["embed_stream"]["EinA"], "embed_stream.EinA")?;
        let positions = ints(&j["embed_stream"]["PinA"], "embed_stream.PinA")?;
        if embed.len() != vocab * d || positions.len() != ctx * d {
            return Err("the embedding tables are not vocab × d and seq_len × d".into());
        }
        let exp_lut = ints(&j["exp_lut"], "exp_lut")?;
        if exp_lut.is_empty() {
            return Err("an empty exp table".into());
        }
        Ok(Int8Model {
            d,
            heads,
            d_ff,
            vocab,
            ctx,
            requant,
            weights,
            embed,
            positions,
            exp_lut,
        })
    }

    /// What a decode step's host reads, besides the caches the step carries: `next`, the token it feeds back as the next
    /// step's `tok`. A step lowered returning these ([`crate::spatial::pipeline::compile_program_to_spatial_returning`])
    /// forwards every other value only its next statement reads.
    pub fn step_reads() -> std::collections::BTreeSet<String> {
        ["next".to_string()].into()
    }

    /// The hand-written CUDA reference of the decode step: one warp generating every token of the trace for `prompt` on the
    /// device in one launch, the `generated` tokens past the prompt printed (`GEN i token`), then the whole trace repeated
    /// (its first argument's count, 2,000 by default) between two events and the time per token printed (`US_PER_TOKEN`).
    /// The GPU's best for this model: the kernel written for the part by hand.
    pub fn reference_cuda(&self, prompt: &[i64], generated: usize) -> Result<String, String> {
        let (d, h, dff, v, ctx) = (self.d, self.heads, self.d_ff, self.vocab, self.ctx);
        if !(d <= 32 && dff <= 32 && v <= 32 && ctx <= 32) {
            return Err(format!("one warp holds every row: d {d}, d_ff {dff}, vocab {v} and ctx {ctx} are each at most 32"));
        }
        let table = |name: &str, v: &[i64]| format!("__constant__ int {name}[{}] = {{{}}};\n", v.len().max(1), v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","));
        let rq = |k: &str| -> String {
            let (mm, s) = self.requant[k];
            format!("{mm}LL, {s}")
        };
        let eq = &self.weights["Eq"];
        let wout: Vec<i64> = (0..d * v).map(|i| eq[(i % v) * d + i / v]).collect();
        let mut cu = String::from("#include <cstdio>\n#include <cstdlib>\n");
        cu += &format!(
            "#define D {d}\n#define H {h}\n#define HD {}\n#define DFF {dff}\n#define V {v}\n#define CTX {ctx}\n#define NLUT {}\n",
            d / h,
            self.exp_lut.len()
        );
        for (k, name) in [
            ("r_qkv", "R_QKV"),
            ("M_q", "M_Q"),
            ("M_k", "M_K"),
            ("M_v", "M_V"),
            ("M_score", "M_SCORE"),
            ("r_ctx", "R_CTX"),
            ("M_o", "M_O"),
            ("r_ffn", "R_FFN"),
            ("M_h", "M_H"),
            ("r_hid", "R_HID"),
            ("M_f", "M_F"),
            ("r_out", "R_OUT"),
        ] {
            cu += &format!("#define {name} {}\n", rq(k));
        }
        cu += &format!(
            "#define NP {}\n#define TOTAL {}\n#define PROMPT {}\n",
            prompt.len(),
            prompt.len() + generated - 1,
            prompt.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(",")
        );
        for (name, vals) in [
            ("EinA", &self.embed),
            ("PinA", &self.positions),
            ("Wq", &self.weights["Wq"]),
            ("Wk", &self.weights["Wk"]),
            ("Wv", &self.weights["Wv"]),
            ("Wo", &self.weights["Wo"]),
            ("W1", &self.weights["W1"]),
            ("W2", &self.weights["W2"]),
            ("E", &self.exp_lut),
            ("Wout", &wout),
        ] {
            cu += &table(name, vals);
        }
        cu += r#"
__device__ __forceinline__ long long rq(long long x, long long mm, int s) { return (x * mm) >> s; }
__device__ __forceinline__ long long c8(long long x) { return x < -128 ? -128 : (x > 127 ? 127 : x); }
__device__ __forceinline__ long long fdiv(long long a, long long b) { long long q = a / b; if ((a % b != 0) && ((a < 0) != (b < 0))) q -= 1; return q; }
__device__ __forceinline__ long long mv(const long long* in, const int* W, int n, int cols, int lane) {
  long long s = 0; if (lane < cols) for (int e = 0; e < n; ++e) s += in[e] * (long long)W[e * cols + lane]; return s;
}
__global__ void decode(const int* prompt, int np, int total, int* out, int reps) {
  __shared__ long long K[CTX][D], Vc[CTX][D], x[D], xi[D], q[D], ci[D], hi[DFF], fi[DFF], oi[D], s1[D];
  __shared__ int nextv;
  const int lane = threadIdx.x;
  for (int rep = 0; rep < reps; ++rep) {
    int tok = prompt[0];
    for (int step = 0; step < total; ++step) {
      const int pos = step;
      if (lane < D) { x[lane] = (long long)EinA[tok * D + lane] + PinA[pos * D + lane]; xi[lane] = c8(rq(x[lane], R_QKV)); }
      __syncwarp();
      long long qa = mv(xi, Wq, D, D, lane), ka = mv(xi, Wk, D, D, lane), va = mv(xi, Wv, D, D, lane);
      if (lane < D) { q[lane] = c8(rq(qa, M_Q)); K[pos][lane] = c8(rq(ka, M_K)); Vc[pos][lane] = c8(rq(va, M_V)); }
      __syncwarp();
      for (int g = 0; g < H; ++g) {
        long long s = 0;
        if (lane <= pos) for (int e = 0; e < HD; ++e) s += q[g * HD + e] * K[lane][g * HD + e];
        long long mx = (lane <= pos) ? s : (long long)-0x7fffffffffffffffLL;
        for (int o = 16; o > 0; o >>= 1) { long long t = __shfl_xor_sync(0xffffffffu, mx, o); mx = t > mx ? t : mx; }
        long long w = 0;
        if (lane <= pos) { long long idx = rq(mx - s, M_SCORE); idx = idx < 0 ? 0 : (idx > NLUT - 1 ? NLUT - 1 : idx); w = E[idx]; }
        long long den = w;
        for (int o = 16; o > 0; o >>= 1) den += __shfl_xor_sync(0xffffffffu, den, o);
        for (int e = 0; e < HD; ++e) {
          long long num = (lane <= pos) ? w * Vc[lane][g * HD + e] : 0;
          for (int o = 16; o > 0; o >>= 1) num += __shfl_xor_sync(0xffffffffu, num, o);
          if (lane == 0) ci[g * HD + e] = c8(rq(fdiv(num, den), R_CTX));
        }
      }
      __syncwarp();
      long long oa = mv(ci, Wo, D, D, lane);
      if (lane < D) { s1[lane] = x[lane] + rq(oa, M_O); hi[lane] = c8(rq(s1[lane], R_FFN)); }
      __syncwarp();
      long long ha = mv(hi, W1, D, DFF, lane);
      if (lane < DFF) { long long r = rq(ha, M_H); r = r < 0 ? 0 : r; fi[lane] = c8(rq(r, R_HID)); }
      __syncwarp();
      long long fa = mv(fi, W2, DFF, D, lane);
      if (lane < D) oi[lane] = c8(rq(s1[lane] + rq(fa, M_F), R_OUT));
      __syncwarp();
      long long lg = mv(oi, Wout, D, V, lane);
      long long best = lane < V ? lg : (long long)-0x7fffffffffffffffLL; int bi = lane;
      for (int o = 16; o > 0; o >>= 1) {
        long long t = __shfl_xor_sync(0xffffffffu, best, o); int ti = __shfl_xor_sync(0xffffffffu, bi, o);
        if (t > best || (t == best && ti < bi)) { best = t; bi = ti; }
      }
      if (lane == 0) nextv = bi;
      __syncwarp();
      if (step + 1 >= np && rep == 0 && lane == 0) out[step + 1 - np] = nextv;
      tok = (step + 1 < np) ? prompt[step + 1] : nextv;
    }
  }
}
int main(int argc, char** argv) {
  const int np = NP, total = TOTAL, ng = total + 1 - np;
  int hp[NP] = {PROMPT};
  int *dp, *dout; cudaMalloc(&dp, sizeof(hp)); cudaMalloc(&dout, sizeof(int) * ng);
  cudaMemcpy(dp, hp, sizeof(hp), cudaMemcpyHostToDevice);
  decode<<<1, 32>>>(dp, np, total, dout, 1);
  int* got = (int*)malloc(sizeof(int) * ng); cudaMemcpy(got, dout, sizeof(int) * ng, cudaMemcpyDeviceToHost);
  for (int i = 0; i < ng; ++i) printf("GEN %d %d\n", i, got[i]);
  const int reps = (argc > 1) ? atoi(argv[1]) : 2000;
  cudaEvent_t a, b; cudaEventCreate(&a); cudaEventCreate(&b);
  decode<<<1, 32>>>(dp, np, total, dout, 10); cudaDeviceSynchronize();
  cudaEventRecord(a); decode<<<1, 32>>>(dp, np, total, dout, reps); cudaEventRecord(b); cudaEventSynchronize(b);
  float ms; cudaEventElapsedTime(&ms, a, b);
  printf("US_PER_TOKEN %.3f\n", ms * 1000.0 / (reps * (double)total));
  return 0;
}
"#    ;
        Ok(cu)
    }

    /// The next token, 0-based, each step over `prompt` predicts on `target`: the decode step at each position lowered
    /// and run on the spatial evaluator, the cache each step writes carried into the next. What a board stepped through
    /// the prompt replies, and the token after the prompt's last is the trainer's first generated one.
    pub fn prompt_next_tokens(&self, prompt: &[usize], target: &super::target::SpatialTarget) -> Result<Vec<usize>, String> {
        use super::eval::{eval_choreography, Inputs, Value};
        let (mut kc, mut vc, mut next) = (Vec::new(), Vec::new(), Vec::new());
        for (pos, tok) in prompt.iter().enumerate() {
            let lowered = super::pipeline::compile_program_to_spatial(&self.decode_step_lg(*tok, pos, &kc, &vc), target)
                .map_err(|e| format!("the step at position {pos}: {e:?}"))?;
            let p = lowered
                .program
                .ok_or_else(|| format!("the step at position {pos} does not lower whole: {:?}", lowered.missing.first()))?;
            let out = eval_choreography(&p, &Inputs::from_fills(&p)?).map_err(|e| format!("the step at position {pos}: {e:?}"))?;
            let read = |name: &str| -> Result<Vec<i64>, String> {
                let t = p.tensors.iter().find(|t| t.name == name).ok_or_else(|| format!("the step has no `{name}`"))?;
                match &out.tensors[&t.id] {
                    Value::Int(v) => Ok(v.clone()),
                    Value::Float(_) => Err(format!("`{name}` evaluated through floats")),
                }
            };
            kc = read("Kc")?[..(pos + 1) * self.d].to_vec();
            vc = read("Vc")?[..(pos + 1) * self.d].to_vec();
            let n = read("next")?[0];
            next.push(usize::try_from(n - 1).map_err(|_| format!("the step at position {pos} predicts token {n}"))?);
        }
        Ok(next)
    }

    /// The decode step for token `tok` at position `pos` (both 0-based), the cache holding the key and value rows of
    /// positions `0..pos` (`pos · d` values each, row-major). The step binds `logits` (`1 × vocab`) and `next`, the
    /// 1-based index of the first maximal logit, and writes this token's rows into `Kc` and `Vc` as row `pos + 1`.
    pub fn decode_step_lg(&self, tok: usize, pos: usize, kc: &[i64], vc: &[i64]) -> String {
        let (d, h, dff, v) = (self.d, self.heads, self.d_ff, self.vocab);
        let mut s = String::from("## Main\n");
        s.push_str(&format!("Let tok be {}.\nLet pos be {}.\n", tok + 1, pos + 1));
        table_lg(&mut s, "EinA", v, d, &self.embed);
        table_lg(&mut s, "PinA", self.ctx, d, &self.positions);
        for w in ["Wq", "Wk", "Wv", "Wo"] {
            table_lg(&mut s, w, d, d, &self.weights[w]);
        }
        table_lg(&mut s, "W1", d, dff, &self.weights["W1"]);
        table_lg(&mut s, "W2", dff, d, &self.weights["W2"]);
        table_lg(&mut s, "Wout", d, v, &self.head_weights());
        table_lg(&mut s, "E", 1, self.exp_lut.len(), &self.exp_lut);
        table_lg(&mut s, "Kc", self.ctx, d, kc);
        table_lg(&mut s, "Vc", self.ctx, d, vc);
        s.push_str("Let xe be row tok of EinA.\nLet xp be row pos of PinA.\nLet x be xe plus xp.\n");
        s.push_str(&self.to8_lg("xi", "x", "r_qkv"));
        for (out, w, k) in [("q", "Wq", "M_q"), ("kt", "Wk", "M_k"), ("vt", "Wv", "M_v")] {
            s.push_str(&format!("Let {out}_a be xi matmul {w}.\n"));
            s.push_str(&self.to8_lg(out, &format!("{out}_a"), k));
        }
        s.push_str("Set row pos of Kc to kt.\nSet row pos of Vc to vt.\n");
        s.push_str(&self.attention_lg("c", "q", "Kc", "Vc", h, "E"));
        s.push_str(&self.to8_lg("ci", "c", "r_ctx"));
        s.push_str("Let o_a be ci matmul Wo.\n");
        s.push_str(&self.rq_lg("o", "o_a", "M_o"));
        s.push_str("Let s1 be x plus o.\n");
        s.push_str(&self.to8_lg("hi", "s1", "r_ffn"));
        s.push_str("Let hid_a be hi matmul W1.\n");
        s.push_str(&self.rq_lg("hid_r", "hid_a", "M_h"));
        s.push_str("Let hid be relu of hid_r.\n");
        s.push_str(&self.to8_lg("fi", "hid", "r_hid"));
        s.push_str("Let f_a be fi matmul W2.\n");
        s.push_str(&self.rq_lg("f", "f_a", "M_f"));
        s.push_str("Let s2 be s1 plus f.\n");
        s.push_str(&self.to8_lg("oi", "s2", "r_out"));
        s.push_str("Let logits be oi matmul Wout.\nLet next be argmax of logits.\n");
        s
    }

    /// The decode step of [`Int8Model::decode_step_lg`] split over `tp` chips, Megatron's way, as one program and the
    /// manifest of where each of its tensors lives.
    ///
    /// Chip `c` owns heads `c·H/tp ..` whole: its columns of `Wq`, `Wk`, `Wv` and of the KV cache, its heads' rows of
    /// `Wo`, an exp table of its own, and FFN columns `c·d_ff/tp ..` of `W1` with their rows of `W2`. Every chip-local
    /// table is named `<table>_c<chip>` and is a [`TpSlice`] of the model's. A chip's output projection is a partial
    /// accumulator (`o_a_c<chip>`, `f_a_c<chip>`), and an all-reduce, spelled as the fold of `plus` it means, sums the
    /// partials before the requant, so the step is the unsplit step bit for bit. The embedding, the residuals and the
    /// head are the replicated spine. On one chip the accumulators are whole and nothing is all-reduced.
    pub fn decode_step_tp_lg(
        &self,
        tok: usize,
        pos: usize,
        kc: &[i64],
        vc: &[i64],
        tp: usize,
    ) -> Result<(String, TpManifest), String> {
        let (d, h, dff, v) = (self.d, self.heads, self.d_ff, self.vocab);
        if tp == 0 {
            return Err("a split over no chips".into());
        }
        if h % tp != 0 {
            return Err(format!("{h} heads do not split over {tp} chips"));
        }
        if dff % tp != 0 {
            return Err(format!("{dff} FFN columns do not split over {tp} chips"));
        }
        let (hc, w, fc) = (h / tp, d / tp, dff / tp);
        let mut m = TpManifest::new(tp);
        let mut s = String::from("## Main\n");
        s.push_str(&format!("Let tok be {}.\nLet pos be {}.\n", tok + 1, pos + 1));
        table_lg(&mut s, "EinA", v, d, &self.embed);
        table_lg(&mut s, "PinA", self.ctx, d, &self.positions);
        let pad = |rows: &[i64]| -> Vec<i64> {
            let mut full = rows.to_vec();
            full.resize(self.ctx * d, 0);
            full
        };
        let (kc_full, vc_full) = (pad(kc), pad(vc));
        for c in 0..tp {
            let heads = vec![(c * w, w)];
            let cols = vec![(c * fc, fc)];
            for (of, shape, axis, ranges, full) in [
                ("Wq", (d, d), SliceAxis::Cols, &heads, &self.weights["Wq"]),
                ("Wk", (d, d), SliceAxis::Cols, &heads, &self.weights["Wk"]),
                ("Wv", (d, d), SliceAxis::Cols, &heads, &self.weights["Wv"]),
                ("Wo", (d, d), SliceAxis::Rows, &heads, &self.weights["Wo"]),
                ("W1", (d, dff), SliceAxis::Cols, &cols, &self.weights["W1"]),
                ("W2", (dff, d), SliceAxis::Rows, &cols, &self.weights["W2"]),
                ("E", (1, self.exp_lut.len()), SliceAxis::Cols, &vec![(0, self.exp_lut.len())], &self.exp_lut),
                ("Kc", (self.ctx, d), SliceAxis::Cols, &heads, &kc_full),
                ("Vc", (self.ctx, d), SliceAxis::Cols, &heads, &vc_full),
            ] {
                let slice = TpSlice { of: of.to_string(), of_shape: shape, axis, ranges: ranges.clone() };
                let (rows, cols) = slice.shape();
                let vals: Vec<i64> = (0..rows * cols).map(|i| full[slice.full_index(i / cols, i % cols)]).collect();
                let name = format!("{of}_c{c}");
                table_lg(&mut s, &name, rows, cols, &vals);
                m.place(&name, c, Some(slice));
            }
        }
        table_lg(&mut s, "Wout", d, v, &self.head_weights());
        // A chip's accumulator of a row-parallel projection: a partial over several chips, the whole on one.
        let partial = |out: &str, c: usize| if tp == 1 { out.to_string() } else { format!("{out}_c{c}") };
        s.push_str("Let xe be row tok of EinA.\nLet xp be row pos of PinA.\nLet x be xe plus xp.\n");
        s.push_str(&self.to8_lg("xi", "x", "r_qkv"));
        let mut o_parts = Vec::new();
        for c in 0..tp {
            let mut local = Vec::new();
            for (out, wt, k) in [("q", "Wq", "M_q"), ("kt", "Wk", "M_k"), ("vt", "Wv", "M_v")] {
                let (o, a) = (format!("{out}_c{c}"), format!("{out}_a_c{c}"));
                s.push_str(&format!("Let {a} be xi matmul {wt}_c{c}.\n"));
                s.push_str(&self.to8_lg(&o, &a, k));
                local.extend([a, format!("{o}_r"), o]);
            }
            s.push_str(&format!("Set row pos of Kc_c{c} to kt_c{c}.\nSet row pos of Vc_c{c} to vt_c{c}.\n"));
            let (ctx_c, ci) = (format!("c_c{c}"), format!("ci_c{c}"));
            s.push_str(&self.attention_lg(&ctx_c, &format!("q_c{c}"), &format!("Kc_c{c}"), &format!("Vc_c{c}"), hc, &format!("E_c{c}")));
            s.push_str(&self.to8_lg(&ci, &ctx_c, "r_ctx"));
            let o_a = partial("o_a", c);
            s.push_str(&format!("Let {o_a} be {ci} matmul Wo_c{c}.\n"));
            local.extend([ctx_c, format!("{ci}_r"), ci, o_a.clone()]);
            o_parts.push(o_a);
            for name in local {
                m.place(&name, c, None);
            }
        }
        fold_lg(&mut s, &mut m, "o_a", o_parts);
        s.push_str(&self.rq_lg("o", "o_a", "M_o"));
        s.push_str("Let s1 be x plus o.\n");
        s.push_str(&self.to8_lg("hi", "s1", "r_ffn"));
        let mut f_parts = Vec::new();
        for c in 0..tp {
            let (ha, hr, hd, fi) = (format!("hid_a_c{c}"), format!("hid_r_c{c}"), format!("hid_c{c}"), format!("fi_c{c}"));
            s.push_str(&format!("Let {ha} be hi matmul W1_c{c}.\n"));
            s.push_str(&self.rq_lg(&hr, &ha, "M_h"));
            s.push_str(&format!("Let {hd} be relu of {hr}.\n"));
            s.push_str(&self.to8_lg(&fi, &hd, "r_hid"));
            let f_a = partial("f_a", c);
            s.push_str(&format!("Let {f_a} be {fi} matmul W2_c{c}.\n"));
            for name in [ha, hr, hd, format!("{fi}_r"), fi, f_a.clone()] {
                m.place(&name, c, None);
            }
            f_parts.push(f_a);
        }
        fold_lg(&mut s, &mut m, "f_a", f_parts);
        s.push_str(&self.rq_lg("f", "f_a", "M_f"));
        s.push_str("Let s2 be s1 plus f.\n");
        s.push_str(&self.to8_lg("oi", "s2", "r_out"));
        s.push_str("Let logits be oi matmul Wout.\nLet next be argmax of logits.\n");
        Ok((s, m))
    }

    /// The head's weights: the tied embedding's transpose, stored that way, as the matmul reads it, rather than
    /// transposed again by every step.
    fn head_weights(&self) -> Vec<i64> {
        let (d, v) = (self.d, self.vocab);
        let eq = &self.weights["Eq"];
        (0..d * v).map(|i| eq[(i % v) * d + i / v]).collect()
    }

    /// `out = x · M >> S` for the requant `k`.
    fn rq_lg(&self, out: &str, x: &str, k: &str) -> String {
        let (m, sh) = self.requant[k];
        format!("Let {out} be {x} times {m} shifted right by {sh}.\n")
    }

    /// `out` = the requant `k` of `x`, saturated to int8 (through `out_r`).
    fn to8_lg(&self, out: &str, x: &str, k: &str) -> String {
        format!(
            "{}Let {out} be {out}_r clamped between -128 and 127.\n",
            self.rq_lg(&format!("{out}_r"), x, k)
        )
    }

    /// `out` = the exp-table attention of `q` over `kc` and `vc` with `heads` heads, up to `pos`, by table `e`.
    fn attention_lg(&self, out: &str, q: &str, kc: &str, vc: &str, heads: usize, e: &str) -> String {
        let (m, sh) = self.requant["M_score"];
        format!(
            "Let {out} be lut attention of {q} over {kc} and {vc} with {heads} heads {heads} groups up to pos with table {e} and scale {m} shifted right by {sh}.\n"
        )
    }
}

/// `Let name be a R by C tensor of Int.`, then a `Set item` for every nonzero value.
fn table_lg(s: &mut String, name: &str, rows: usize, cols: usize, vals: &[i64]) {
    s.push_str(&format!("Let {name} be a {rows} by {cols} tensor of Int.\n"));
    for (i, x) in vals.iter().enumerate() {
        if *x != 0 {
            s.push_str(&format!("Set item {} of {name} to {x}.\n", i + 1));
        }
    }
}

/// `out` = the sum of `partials`, one per chip in chip order, as the left fold of `plus` an all-reduce means, recorded
/// in the manifest. One partial is the sum itself: nothing is folded.
fn fold_lg(s: &mut String, m: &mut TpManifest, out: &str, partials: Vec<String>) {
    if partials.len() < 2 {
        return;
    }
    let mut folds = Vec::new();
    let mut acc = partials[0].clone();
    for (i, p) in partials.iter().enumerate().skip(1) {
        let binder = if i + 1 == partials.len() { out.to_string() } else { format!("{out}_f{i}") };
        s.push_str(&format!("Let {binder} be {acc} plus {p}.\n"));
        folds.push(binder.clone());
        acc = binder;
    }
    m.collectives.push(TpCollective { out: out.to_string(), partials, folds });
}
