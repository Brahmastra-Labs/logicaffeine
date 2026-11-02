//! systolic — the weight-stationary spatial GEMM tile (Phase 1).
//!
//! Today's GEMM lowering is a 1-D unrolled dot-product **reduction** (P multiply tiles → one accumulator →
//! one output per operand pass): a MAC *lane*. A real accelerator (TPU MXU, a SambaNova PCU in GEMM mode)
//! keeps the weights **stationary** in a 2-D array of PEs, streams activation rows through, and produces a
//! whole output row **in parallel** — so each weight is read once and reused across the entire activation
//! stream (the operand-reuse the reduction lane lacks), and N outputs settle per cycle instead of 1.
//!
//! A [`Systolic`] tile is `K` rows × `N` cols of int8 multiply-accumulate PEs. PE(r, c) holds the stationary
//! weight `W[r][c]`. Each cycle one activation row `A[i][0..K]` is applied and one output row
//! `O[i][0..N]`, with `O[i][c] = Σ_r int8(A[i][r]) · int8(W[r][c])`, is registered out. [`eval_systolic`] is
//! the reference semantics; [`emit_systolic_verilog`] is the registered array; the per-PE datapath
//! `psum + a·w` is SAT-certified ≡ its abstract form by the existing placer + bitblast (see the lock).
//!
//! The datatype is int8 operands accumulated in wrapping `i64` (the VM's `Int`), so for int8-range inputs the
//! tile is byte-identical to a plain integer GEMM — and to the VM. Operand truncation is `x as i8 as i64`,
//! matched exactly by the RTL's `$signed(x[7:0])` sign-extension, so eval ≡ RTL for ALL inputs.

/// A weight-stationary GEMM tile: `k` contraction rows × `n` output columns of int8 MAC PEs, weights baked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Systolic {
    /// Contraction depth (array rows / dot length).
    pub k: usize,
    /// Output width (array cols / outputs per cycle).
    pub n: usize,
    /// Stationary weights, `k*n` row-major: `weights[r*n + c] = W[r][c]` (int8 range).
    pub weights: Vec<i64>,
}

impl Systolic {
    /// Build a `k×n` weight-stationary tile; `weights` is `k*n` row-major.
    pub fn new(k: usize, n: usize, weights: Vec<i64>) -> Systolic {
        assert_eq!(weights.len(), k * n, "weights must be k*n row-major");
        Systolic { k, n, weights }
    }

    /// The stationary weight held by PE(r, c).
    pub fn w(&self, r: usize, c: usize) -> i64 {
        self.weights[r * self.n + c]
    }

    /// Weight-reads to run an `m`-row activation stream: `k*n` — each weight loaded ONCE and reused across all
    /// `m` rows (the reuse a reduction lane lacks, which re-fetches `k*n*m`).
    pub fn weight_reads(&self) -> usize {
        self.k * self.n
    }

    /// Cycles to stream `m` activation rows through the tile: `m` (one row in, one row out per cycle) — versus
    /// `m*n*k` for a 1-cycle-per-MAC scalar reduction computing the same `m*n` outputs.
    pub fn cycles(&self, m: usize) -> usize {
        m
    }
}

/// Sign-extend `x` from its low `bits` (the int-precision truncation; `bits >= 64` is a no-op). Matches the
/// RTL's `$signed(x[bits-1:0])`.
pub fn trunc_signed(x: i64, bits: u32) -> i64 {
    if bits >= 64 {
        x
    } else {
        let sh = 64 - bits;
        (x << sh) >> sh
    }
}

/// Reference semantics: `O(M×N) = A(M×K) · W(K×N)`, int8 operands accumulated in wrapping `i64`.
/// `acts` is `m*k` row-major (`A[i][r] = acts[i*k + r]`); returns `O` as `m*n` row-major.
pub fn eval_systolic(sys: &Systolic, acts: &[i64], m: usize) -> Vec<i64> {
    eval_systolic_bits(sys, acts, m, 8)
}

/// Precision-parameterized integer semantics: operands truncated to `bits` (8/16/32/64), products
/// accumulated in wrapping `i64` — the integer precision dial (Phase 4).
pub fn eval_systolic_bits(sys: &Systolic, acts: &[i64], m: usize, bits: u32) -> Vec<i64> {
    assert_eq!(acts.len(), m * sys.k, "activations must be m*k row-major");
    let mut out = vec![0i64; m * sys.n];
    for i in 0..m {
        for c in 0..sys.n {
            let mut acc: i64 = 0;
            for r in 0..sys.k {
                let a = trunc_signed(acts[i * sys.k + r], bits);
                let w = trunc_signed(sys.w(r, c), bits);
                acc = acc.wrapping_add(a.wrapping_mul(w));
            }
            out[i * sys.n + c] = acc;
        }
    }
    out
}

/// Round `x` to bf16 (8-bit exponent, 7-bit mantissa) with round-to-nearest-even — the float precision dial
/// (Phase 4). Idempotent, and a no-op on values already representable in bf16.
pub fn round_bf16(x: f64) -> f64 {
    let f = x as f32;
    if !f.is_finite() {
        return f as f64;
    }
    let bits = f.to_bits();
    let rounding_bias = 0x7fff + ((bits >> 16) & 1); // round-to-nearest-even on the low 16 bits
    let rounded = bits.wrapping_add(rounding_bias) & 0xFFFF_0000;
    f32::from_bits(rounded) as f64
}

/// bf16 GEMM: operands rounded to bf16, products and accumulation in f32 (the standard bf16 MAC), returned as
/// f64. `weights` is `k*n` row-major, `acts` is `m*k` row-major.
pub fn eval_systolic_bf16(weights: &[f64], acts: &[f64], m: usize, k: usize, n: usize) -> Vec<f64> {
    assert_eq!(weights.len(), k * n);
    assert_eq!(acts.len(), m * k);
    let mut out = vec![0f64; m * n];
    for i in 0..m {
        for c in 0..n {
            let mut acc: f32 = 0.0;
            for r in 0..k {
                let a = round_bf16(acts[i * k + r]) as f32;
                let w = round_bf16(weights[r * n + c]) as f32;
                acc += a * w;
            }
            out[i * n + c] = acc as f64;
        }
    }
    out
}

/// fp32 GEMM: operands, products and accumulation in f32 (the PCU's native 32-bit FU MAC — Plasticine's
/// format), returned as f64. `weights` is `k*n` row-major, `acts` is `m*k` row-major.
pub fn eval_systolic_f32(weights: &[f64], acts: &[f64], m: usize, k: usize, n: usize) -> Vec<f64> {
    assert_eq!(weights.len(), k * n);
    assert_eq!(acts.len(), m * k);
    let mut out = vec![0f64; m * n];
    for i in 0..m {
        for c in 0..n {
            let mut acc: f32 = 0.0;
            for r in 0..k {
                acc += (acts[i * k + r] as f32) * (weights[r * n + c] as f32);
            }
            out[i * n + c] = acc as f64;
        }
    }
    out
}

/// Compute a `K×N` GEMM by TILING into `tk×tn` sub-tiles (Phase 2). Each weight sub-block is loaded ONCE
/// into the on-chip scratchpad and reused across all `m` activation rows; `k`-direction tiles are
/// accumulated. Byte-identical to [`eval_systolic`] on the full matrix — so the tiling is proven correct by
/// equality to the untiled reference (which is itself ≡ the oracle ≡ VM). `acts` is `m*k` row-major.
pub fn eval_tiled_gemm(sys: &Systolic, acts: &[i64], m: usize, tk: usize, tn: usize) -> Vec<i64> {
    let (k, n) = (sys.k, sys.n);
    assert!(tk >= 1 && tn >= 1, "tile dims must be positive");
    assert_eq!(acts.len(), m * k, "activations must be m*k row-major");
    let mut out = vec![0i64; m * n];
    let mut c0 = 0;
    while c0 < n {
        let cn = (c0 + tn).min(n);
        let mut r0 = 0;
        while r0 < k {
            let rk = (r0 + tk).min(k);
            let (sub_k, sub_n) = (rk - r0, cn - c0);
            // Weight sub-block resident in the scratchpad for this (k-tile, n-tile), reused across all m rows.
            let mut wsub = Vec::with_capacity(sub_k * sub_n);
            for r in r0..rk {
                for c in c0..cn {
                    wsub.push(sys.w(r, c));
                }
            }
            let sub = Systolic::new(sub_k, sub_n, wsub);
            let mut asub = Vec::with_capacity(m * sub_k);
            for i in 0..m {
                for r in r0..rk {
                    asub.push(acts[i * k + r]);
                }
            }
            let osub = eval_systolic(&sub, &asub, m); // m × sub_n
            for i in 0..m {
                for cc in 0..sub_n {
                    let o = &mut out[i * n + (c0 + cc)];
                    *o = o.wrapping_add(osub[i * sub_n + cc]);
                }
            }
            r0 = rk;
        }
        c0 = cn;
    }
    out
}

/// A fused transformer-block stage over a weight-stationary GEMM tile (Phase 3): the GEMM, its fixed-point
/// requantization `(acc·M) >> S` (arithmetic shift = floor-divide by `2^S`), a per-column bias add, and a
/// ReLU — all fused into ONE spatial pass so the intermediates are streamed, never materialized to memory.
#[derive(Clone, Debug)]
pub struct FusedBlock {
    pub sys: Systolic,
    pub requant_m: i64,
    pub requant_s: u32,
    /// Per-output-column bias (`n` entries).
    pub bias: Vec<i64>,
}

/// Evaluate the fused block: `O[i][c] = ReLU( ((A·W)[i][c] · M) >> S + bias[c] )` — one streamed pass, no
/// intermediate tensor written back. Byte-identical to computing the four stages separately (proven by the
/// lock), so the fusion is correctness-preserving; the win is that nothing lands in DRAM between stages.
pub fn eval_fused_block(fb: &FusedBlock, acts: &[i64], m: usize) -> Vec<i64> {
    let n = fb.sys.n;
    assert_eq!(
        fb.bias.len(),
        n,
        "bias must have one entry per output column"
    );
    let gemm = eval_systolic(&fb.sys, acts, m);
    let mut out = vec![0i64; m * n];
    for i in 0..m {
        for c in 0..n {
            let rq = fb.requant_m.wrapping_mul(gemm[i * n + c]) >> fb.requant_s; // arithmetic shift = requant
            let biased = rq.wrapping_add(fb.bias[c]);
            out[i * n + c] = biased.max(0); // ReLU
        }
    }
    out
}

/// The UNFUSED reference: the same computation with each stage materialized into its own buffer (a DRAM
/// round-trip per stage). Equal to [`eval_fused_block`] — the composition proof — but it writes `2·m·n`
/// intermediate elements the fused pass never does. Returns `(result, intermediate_writes)`.
pub fn eval_unfused_block(fb: &FusedBlock, acts: &[i64], m: usize) -> (Vec<i64>, usize) {
    let n = fb.sys.n;
    let gemm = eval_systolic(&fb.sys, acts, m); // materialized tensor #1 (m·n writes)
    let mut requant = vec![0i64; m * n]; // materialized tensor #2 (m·n writes)
    for i in 0..m {
        for c in 0..n {
            requant[i * n + c] = fb.requant_m.wrapping_mul(gemm[i * n + c]) >> fb.requant_s;
        }
    }
    let mut out = vec![0i64; m * n];
    for i in 0..m {
        for c in 0..n {
            out[i * n + c] = requant[i * n + c].wrapping_add(fb.bias[c]).max(0);
        }
    }
    (out, 2 * m * n) // two intermediate tensors written+read that the fused pass streams instead
}

/// The systolic microarchitecture dial. Both schedules compute the identical GEMM (`eval_systolic`); they
/// trade against each other rather than dominate, so both stay live (see the dominance characterization in
/// `pipelined_systolic_lock`). [`SystolicSchedule::Combinational`] accumulates each column with a K-deep adder
/// tree — one output row per clock, zero fill latency, but the critical path grows with K. [`Pipelined`](SystolicSchedule::Pipelined)
/// registers the activation (east) and partial sum (south) at every PE so the only path between flip-flops is
/// one mul + one add — the critical path is K-INDEPENDENT (a real MXU stays at GHz at 128×128), at the cost of
/// `K+N−1` fill latency and the extra registers. Default is `Combinational` (smallest; correct for low clocks).
///
/// [`Pipelined`]: SystolicSchedule::Pipelined
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SystolicSchedule {
    #[default]
    Combinational,
    Pipelined,
}

/// Emit the weight-stationary array under the chosen [`SystolicSchedule`] at precision `bits`. Both schedules
/// are observationally equivalent (`eval_systolic`); this is the single retargeting seam an emitter/target
/// selects the microarchitecture through.
pub fn emit_systolic_scheduled(
    name: &str,
    sys: &Systolic,
    schedule: SystolicSchedule,
    bits: u32,
) -> String {
    match schedule {
        SystolicSchedule::Combinational => emit_systolic_verilog_bits(name, sys, bits),
        SystolicSchedule::Pipelined => emit_systolic_pipelined_verilog_bits(name, sys, bits),
    }
}

/// Emit the registered weight-stationary array as synthesizable Verilog. Ports: `clk`, `rst`, the `k`
/// activation inputs `a0..a{k-1}` (64-bit; the low 8 bits are the int8 operand), and the `n` registered
/// outputs `o0..o{n-1}`. Weights are baked as signed 64-bit constants (the sign-extended int8). One output
/// row is produced per clock; `o{c}` on cycle `t+1` is the dot of the activation row applied at cycle `t`.
pub fn emit_systolic_verilog(name: &str, sys: &Systolic) -> String {
    emit_systolic_verilog_bits(name, sys, 8)
}

/// Emit the weight-stationary array at a chosen integer precision `bits` (8/16/32/64) — the precision dial's
/// RTL (Phase 4). Operands are the low `bits` of each input, sign-extended; weights baked as sign-extended
/// two's-complement constants. Matches [`eval_systolic_bits`] for ALL inputs.
pub fn emit_systolic_verilog_bits(name: &str, sys: &Systolic, bits: u32) -> String {
    let (k, n) = (sys.k, sys.n);
    let hi = bits.min(64) - 1;
    let mut s = String::new();
    s.push_str(&format!("module {name}(\n  input clk,\n  input rst,\n"));
    for r in 0..k {
        s.push_str(&format!("  input signed [63:0] a{r},\n"));
    }
    for c in 0..n {
        let comma = if c + 1 < n { "," } else { "" };
        s.push_str(&format!("  output reg signed [63:0] o{c}{comma}\n"));
    }
    s.push_str(");\n");
    // operand = low `bits`, sign-extended to 64 (matches `trunc_signed(x, bits)` in eval_systolic_bits).
    for r in 0..k {
        s.push_str(&format!(
            "  wire signed [63:0] sa{r} = $signed(a{r}[{hi}:0]);\n"
        ));
    }
    s.push_str("  always @(posedge clk) begin\n    if (rst) begin\n");
    for c in 0..n {
        s.push_str(&format!("      o{c} <= 64'sd0;\n"));
    }
    s.push_str("    end else begin\n");
    for c in 0..n {
        let terms: Vec<String> = (0..k)
            .map(|r| {
                let w = trunc_signed(sys.w(r, c), bits);
                format!("sa{r} * 64'sh{:016X}", w as u64)
            })
            .collect();
        s.push_str(&format!("      o{c} <= {};\n", terms.join(" + ")));
    }
    s.push_str("    end\n  end\nendmodule\n");
    s
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// PIPELINED, register-staged weight-stationary systolic array (the TPU-MXU microarchitecture). Unlike the
// combinational tile above (a K-deep adder tree per column → critical path grows with K), each PE registers
// its activation (flowing EAST) and its partial sum (flowing SOUTH), so the only combinational path between
// flip-flops is ONE multiply + ONE add — the critical path is K-INDEPENDENT (why a real MXU stays at GHz at
// 128×128). On-chip input skew (row r delayed r cycles) + output de-skew (col c delayed N-1-c) keep the clean
// row-in/row-out interface; row i's output emerges at cycle i + L, L = K+N-1.
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// The fill latency of the pipelined array: row `i`'s output emerges at cycle `i + L`, `L = K + N − 1`.
/// Shared by the golden model, the RTL emitter, and the lock so they cannot disagree.
pub fn pipelined_latency(k: usize, n: usize) -> usize {
    k + n - 1
}

/// Cycle-accurate golden model of the pipelined weight-stationary array — steps the register planes exactly
/// as the emitted RTL does (activation EAST, partial-sum SOUTH, on-chip skew/de-skew), so the RTL must match
/// it cycle-for-cycle. Returns `(O row-major, L)` where `O` ≡ [`eval_systolic_bits`] (the math is unchanged;
/// only the timing differs). `acts` is `m*k` row-major.
pub fn eval_systolic_pipelined(
    sys: &Systolic,
    acts: &[i64],
    m: usize,
    bits: u32,
) -> (Vec<i64>, usize) {
    eval_systolic_pipelined_acc(sys, acts, m, bits, ACC_BITS_FULL)
}

/// [`eval_systolic_pipelined`] with an explicit accumulator width `acc_bits` — the partial-sum register wraps
/// (two's-complement) at `acc_bits`, exactly like the `acc_bits`-wide `ps_reg` in the emitted RTL. With
/// `acc_bits = 64` this is bit-identical to [`eval_systolic_pipelined`]; with [`ACC_BITS_MXU`] it models the
/// int32-accumulator tape and is lossless whenever the true accumulation fits (guarded in the lock).
pub fn eval_systolic_pipelined_acc(
    sys: &Systolic,
    acts: &[i64],
    m: usize,
    bits: u32,
    acc_bits: u32,
) -> (Vec<i64>, usize) {
    assert_eq!(acts.len(), m * sys.k, "activations must be m*k row-major");
    let (k, n) = (sys.k, sys.n);
    let l = pipelined_latency(k, n);
    let w = |r: usize, c: usize| trunc_signed(sys.w(r, c), bits);
    let mut a_reg = vec![0i64; k * n]; // activation register per PE (flows EAST)
    let mut ps_reg = vec![0i64; k * n]; // partial-sum register per PE (flows SOUTH)
    let mut skew: Vec<Vec<i64>> = (0..k).map(|r| vec![0i64; r]).collect(); // input skew: row r depth r
    let mut deskew: Vec<Vec<i64>> = (0..n).map(|c| vec![0i64; n - 1 - c]).collect(); // output de-skew: col c depth n-1-c
    let mut out = vec![0i64; m * n];
    for t in 0..(m + l + 1) {
        // present activation row t (zeros while draining) into the input-skew delay lines
        let a_top: Vec<i64> = (0..k)
            .map(|r| {
                if t < m {
                    trunc_signed(acts[t * k + r], bits)
                } else {
                    0
                }
            })
            .collect();
        let mut a_skew = vec![0i64; k];
        for r in 0..k {
            if r == 0 {
                a_skew[0] = a_top[0];
            } else {
                a_skew[r] = skew[r][r - 1]; // emerges after r cycles
                for j in (1..r).rev() {
                    skew[r][j] = skew[r][j - 1];
                }
                skew[r][0] = a_top[r];
            }
        }
        // PE next-state, computed from CURRENT registers (non-blocking semantics), then committed
        let mut a_next = vec![0i64; k * n];
        let mut ps_next = vec![0i64; k * n];
        for r in 0..k {
            for c in 0..n {
                let a_in = if c == 0 {
                    a_skew[r]
                } else {
                    a_reg[r * n + (c - 1)]
                };
                let ps_in = if r == 0 { 0 } else { ps_reg[(r - 1) * n + c] };
                a_next[r * n + c] = a_in;
                ps_next[r * n + c] =
                    trunc_signed(ps_in.wrapping_add(a_in.wrapping_mul(w(r, c))), acc_bits);
            }
        }
        // output de-skew: sample the current bottom-row partial sum, delay col c by n-1-c
        let mut o_now = vec![0i64; n];
        for c in 0..n {
            let bottom = ps_reg[(k - 1) * n + c];
            let d = n - 1 - c;
            if d == 0 {
                o_now[c] = bottom;
            } else {
                o_now[c] = deskew[c][d - 1];
                for j in (1..d).rev() {
                    deskew[c][j] = deskew[c][j - 1];
                }
                deskew[c][0] = bottom;
            }
        }
        // row i's aligned output emerges at cycle i + L
        if t >= l && (t - l) < m {
            let i = t - l;
            out[i * n..i * n + n].copy_from_slice(&o_now);
        }
        a_reg = a_next;
        ps_reg = ps_next;
    }
    (out, l)
}

/// [`eval_systolic_pipelined`] without the latency — the `O` matrix only, for equivalence tests.
pub fn eval_systolic_pipelined_o(sys: &Systolic, acts: &[i64], m: usize, bits: u32) -> Vec<i64> {
    eval_systolic_pipelined(sys, acts, m, bits).0
}

/// Emit the PIPELINED weight-stationary array (int8) as synthesizable Verilog. See
/// [`emit_systolic_pipelined_verilog_bits`]; the critical path is ONE mul + ONE add (K-independent).
pub fn emit_systolic_pipelined_verilog(name: &str, sys: &Systolic) -> String {
    emit_systolic_pipelined_verilog_bits(name, sys, 8)
}

/// Default accumulator width — 64-bit, so [`emit_systolic_pipelined_verilog_bits`] is bit-identical to the
/// i64 model for EVERY input. Real int8 MXUs (and the taped tiles) use a narrower [`ACC_BITS_MXU`] accumulator.
pub const ACC_BITS_FULL: u32 = 64;
/// The TPU-MXU-standard int8 accumulator width (int32). Narrows the inter-PE partial-sum bus so a systolic
/// grid routes without congestion; lossless for int8 with contraction depth K where |Σ a·w| < 2³¹.
pub const ACC_BITS_MXU: u32 = 32;

/// Emit the pipelined weight-stationary array at precision `bits` with a full (64-bit) accumulator.
pub fn emit_systolic_pipelined_verilog_bits(name: &str, sys: &Systolic, bits: u32) -> String {
    emit_systolic_pipelined_verilog_bits_acc(name, sys, bits, ACC_BITS_FULL)
}

/// Emit the pipelined, register-staged weight-stationary systolic array at operand precision `bits` and
/// accumulator width `acc_bits`. Structure (matches [`eval_systolic_pipelined_acc`] register-for-register):
/// per PE an activation register (`a_reg`, flows EAST, `bits` wide) and a partial-sum register (`ps_reg`,
/// flows SOUTH, `acc_bits` wide) with `ps_reg[r][c] <= ps_in + a_in·W[r][c]` — the only combinational path
/// between flip-flops is one `bits×bits` multiply + one add, so the critical path does NOT grow with K.
/// On-chip input-skew (row r delayed r cycles) and output de-skew (col c delayed N-1-c) keep the clean
/// `clk/rst/a0..a{k-1}/o0..o{n-1}` interface; `o{c}` on cycle `t` holds output row `t-L` (L=K+N-1). Narrowing
/// `acc_bits` to [`ACC_BITS_MXU`] shrinks the inter-PE bus for a congestion-free tape and is lossless while
/// the true accumulation fits `acc_bits` (guarded in the lock).
pub fn emit_systolic_pipelined_verilog_bits_acc(
    name: &str,
    sys: &Systolic,
    bits: u32,
    acc_bits: u32,
) -> String {
    pe_array_verilog(name, sys.k, sys.n, bits, acc_bits, WeightSource::Baked(sys))
}

/// Where a PE array's weights come from: baked into the netlist as constants, or loaded at run time through
/// a row port (`w_load`, `w_row`, `w_data` = one packed row of `n` weights) — the spatial IR's
/// `Compute(MatmulTiles)` stage on our fabric loads its weight block, then streams activations.
#[derive(Clone, Copy)]
pub enum WeightSource<'a> {
    Baked(&'a Systolic),
    Loaded(WeightStart),
}

/// What a loaded array's weight registers hold before their rows are first loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeightStart {
    /// What the register powers up with: every row is loaded before the array multiplies, as in a GEMM as deep as the
    /// array.
    PowerUp,
    /// Zero, the register's initial value: a matmul shallower than the array loads only its first rows, and the rest
    /// multiply the zero activations it streams through them into zero, never into an unknown a register powered up
    /// with. A 7-series configuration and a simulator start a register at its initial value; an ASIC flop holds
    /// whatever it powers up with, which the zero activations still multiply into zero.
    Zero,
}

/// A weight-stationary PE array whose weights are loaded through a port and hold what they power up with until then:
/// `pe_array_verilog(.., Loaded(PowerUp))`. Cycle-for-cycle identical to the baked array once the same weights are
/// loaded (locked).
pub fn emit_systolic_pipelined_loadable_verilog(
    name: &str,
    k: usize,
    n: usize,
    bits: u32,
    acc_bits: u32,
) -> String {
    emit_systolic_pipelined_loadable_verilog_starting(name, k, n, bits, acc_bits, WeightStart::PowerUp)
}

/// [`emit_systolic_pipelined_loadable_verilog`] with its weight registers starting at `start`.
pub fn emit_systolic_pipelined_loadable_verilog_starting(
    name: &str,
    k: usize,
    n: usize,
    bits: u32,
    acc_bits: u32,
    start: WeightStart,
) -> String {
    pe_array_verilog(name, k, n, bits, acc_bits, WeightSource::Loaded(start))
}

fn pe_array_verilog(
    name: &str,
    k: usize,
    n: usize,
    bits: u32,
    acc_bits: u32,
    weights: WeightSource,
) -> String {
    let hi = bits.min(64) - 1;
    // narrow the STREAMING operand to its real precision so synthesis infers a `bits×bits` multiplier, not a
    // 64×64 one — an int8 tile then routes with an 8×8 multiplier per PE.
    let aw = bits.min(64); // activation datapath width
    let pw = (2 * bits).min(acc_bits); // product width
    let ah = acc_bits.min(64) - 1; // accumulator MSB
    let mut s = String::new();
    s.push_str(&format!("module {name}(\n  input clk,\n  input rst,\n"));
    if matches!(weights, WeightSource::Loaded(_)) {
        let rw = (usize::BITS - (k.max(2) - 1).leading_zeros()).max(1);
        s.push_str(&format!(
            "  input w_load,\n  input [{}:0] w_row,\n  input [{}:0] w_data,\n",
            rw - 1,
            n as u32 * bits - 1
        ));
    }
    for r in 0..k {
        s.push_str(&format!("  input signed [63:0] a{r},\n"));
    }
    for c in 0..n {
        let comma = if c + 1 < n { "," } else { "" };
        s.push_str(&format!("  output signed [63:0] o{c}{comma}\n"));
    }
    s.push_str(");\n");
    for r in 0..k {
        s.push_str(&format!(
            "  wire signed [{}:0] sa{r} = $signed(a{r}[{hi}:0]);\n",
            aw - 1
        ));
    }
    // input-skew registers (row r has depth r) and the skewed-activation reads (activation-width)
    for r in 0..k {
        for j in 0..r {
            s.push_str(&format!("  reg signed [{}:0] skew_{r}_{j};\n", aw - 1));
        }
    }
    for r in 0..k {
        if r == 0 {
            s.push_str(&format!("  wire signed [{}:0] a_sk0 = sa0;\n", aw - 1));
        } else {
            s.push_str(&format!(
                "  wire signed [{}:0] a_sk{r} = skew_{r}_{};\n",
                aw - 1,
                r - 1
            ));
        }
    }
    // PE registers + combinational in-wires: activation (east) is activation-width, partial sum (south) is 64-bit
    for r in 0..k {
        for c in 0..n {
            s.push_str(&format!("  reg signed [{}:0] a_reg_{r}_{c};\n", aw - 1));
            s.push_str(&format!("  reg signed [{ah}:0] ps_reg_{r}_{c};\n"));
            let a_in = if c == 0 {
                format!("a_sk{r}")
            } else {
                format!("a_reg_{r}_{}", c - 1)
            };
            s.push_str(&format!(
                "  wire signed [{}:0] a_in_{r}_{c} = {a_in};\n",
                aw - 1
            ));
            let ps_in = if r == 0 {
                format!("{acc_bits}'sd0")
            } else {
                format!("ps_reg_{}_{c}", r - 1)
            };
            s.push_str(&format!(
                "  wire signed [{ah}:0] ps_in_{r}_{c} = {ps_in};\n"
            ));
            // bits×bits product (exact int8 → `pw` bits), sign-extended into the acc_bits accumulate
            match weights {
                WeightSource::Baked(sys) => {
                    let w = trunc_signed(sys.w(r, c), bits);
                    let wmask = if bits >= 64 {
                        u64::MAX
                    } else {
                        (1u64 << bits) - 1
                    };
                    let wv = (w as u64) & wmask;
                    s.push_str(&format!(
                        "  wire signed [{}:0] prod_{r}_{c} = a_in_{r}_{c} * {bits}'sh{wv:X};\n",
                        pw - 1
                    ));
                }
                WeightSource::Loaded(start) => {
                    let init = match start {
                        WeightStart::PowerUp => String::new(),
                        WeightStart::Zero => format!(" = {bits}'sd0"),
                    };
                    s.push_str(&format!("  reg signed [{}:0] w_reg_{r}_{c}{init};\n", bits - 1));
                    s.push_str(&format!(
                        "  wire signed [{}:0] prod_{r}_{c} = a_in_{r}_{c} * w_reg_{r}_{c};\n",
                        pw - 1
                    ));
                }
            }
        }
    }
    // output de-skew registers (col c has depth n-1-c) and the output reads
    for c in 0..n {
        let d = n - 1 - c;
        for j in 0..d {
            s.push_str(&format!("  reg signed [{ah}:0] deskew_{c}_{j};\n"));
        }
        if d == 0 {
            s.push_str(&format!("  assign o{c} = ps_reg_{}_{c};\n", k - 1));
        } else {
            s.push_str(&format!("  assign o{c} = deskew_{c}_{};\n", d - 1));
        }
    }
    s.push_str("  always @(posedge clk) begin\n    if (rst) begin\n");
    for r in 0..k {
        for j in 0..r {
            s.push_str(&format!("      skew_{r}_{j} <= {aw}'sd0;\n"));
        }
    }
    for r in 0..k {
        for c in 0..n {
            s.push_str(&format!("      a_reg_{r}_{c} <= {aw}'sd0;\n"));
            s.push_str(&format!("      ps_reg_{r}_{c} <= {acc_bits}'sd0;\n"));
        }
    }
    for c in 0..n {
        for j in 0..(n - 1 - c) {
            s.push_str(&format!("      deskew_{c}_{j} <= {acc_bits}'sd0;\n"));
        }
    }
    s.push_str("    end else begin\n");
    for r in 1..k {
        s.push_str(&format!("      skew_{r}_0 <= sa{r};\n"));
        for j in 1..r {
            s.push_str(&format!("      skew_{r}_{j} <= skew_{r}_{};\n", j - 1));
        }
    }
    for r in 0..k {
        for c in 0..n {
            s.push_str(&format!("      a_reg_{r}_{c} <= a_in_{r}_{c};\n"));
            s.push_str(&format!(
                "      ps_reg_{r}_{c} <= ps_in_{r}_{c} + prod_{r}_{c};\n"
            ));
        }
    }
    for c in 0..n {
        let d = n - 1 - c;
        if d > 0 {
            s.push_str(&format!("      deskew_{c}_0 <= ps_reg_{}_{c};\n", k - 1));
            for j in 1..d {
                s.push_str(&format!("      deskew_{c}_{j} <= deskew_{c}_{};\n", j - 1));
            }
        }
    }
    s.push_str("    end\n  end\n");
    if matches!(weights, WeightSource::Loaded(_)) {
        s.push_str("  always @(posedge clk) begin\n    if (w_load) begin\n");
        for r in 0..k {
            s.push_str(&format!("      if (w_row == {r}) begin\n"));
            for c in 0..n {
                s.push_str(&format!(
                    "        w_reg_{r}_{c} <= $signed(w_data[{}:{}]);\n",
                    (c as u32 + 1) * bits - 1,
                    c as u32 * bits
                ));
            }
            s.push_str("      end\n");
        }
        s.push_str("    end\n  end\n");
    }
    s.push_str("endmodule\n");
    s
}

/// A bench that loads `sys`'s weights into the loadable array while both arrays sit in reset, releases both
/// on the same edge, streams `acts` (m rows) through both, and compares every output every cycle. Prints
/// `OUT o <i*n+c> <value>` for the loadable array's aligned output rows (row i sampled after edge i+L-1),
/// `LATENCY <L>`, and `PASS:` iff the two arrays never differed.
pub fn emit_loadable_vs_baked_testbench(
    baked: &str,
    loadable: &str,
    sys: &Systolic,
    acts: &[i64],
    m: usize,
    bits: u32,
    acc_bits: u32,
) -> String {
    let (k, n) = (sys.k, sys.n);
    let (_, l) = eval_systolic_pipelined_acc(sys, acts, m, bits, acc_bits);
    let rw = (usize::BITS - (k.max(2) - 1).leading_zeros()).max(1);
    let wmask: u64 = if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let mut tb =
        String::from("`timescale 1ns/1ps\nmodule tb_pe;\n  reg clk = 0, rst = 1, w_load = 0;\n");
    tb.push_str(&format!(
        "  reg [{}:0] w_row = 0;\n  reg [{}:0] w_data = 0;\n",
        rw - 1,
        n as u32 * bits - 1
    ));
    for r in 0..k {
        tb.push_str(&format!("  reg signed [63:0] a{r} = 0;\n"));
    }
    for c in 0..n {
        tb.push_str(&format!("  wire signed [63:0] ob{c}, ol{c};\n"));
    }
    let ac: Vec<String> = (0..k).map(|r| format!(".a{r}(a{r})")).collect();
    let ob: Vec<String> = (0..n).map(|c| format!(".o{c}(ob{c})")).collect();
    let ol: Vec<String> = (0..n).map(|c| format!(".o{c}(ol{c})")).collect();
    tb.push_str(&format!(
        "  {baked} u_baked(.clk(clk), .rst(rst), {}, {});\n",
        ac.join(", "),
        ob.join(", ")
    ));
    tb.push_str(&format!("  {loadable} u_loadable(.clk(clk), .rst(rst), .w_load(w_load), .w_row(w_row), .w_data(w_data), {}, {});\n", ac.join(", "), ol.join(", ")));
    tb.push_str("  integer fails = 0;\n  always #5 clk = ~clk;\n  initial begin\n    rst = 1; @(posedge clk); #1;\n");
    for r in 0..k {
        // Pack row r: weight c at bits [c*bits +: bits].
        let mut packed = String::new();
        for c in (0..n).rev() {
            let w = (trunc_signed(sys.w(r, c), bits) as u64) & wmask;
            packed.push_str(&format!("{:0width$b}", w, width = bits as usize));
        }
        tb.push_str(&format!(
            "    w_load = 1; w_row = {r}; w_data = {}'b{packed}; @(posedge clk); #1;\n",
            n as u32 * bits
        ));
    }
    tb.push_str("    w_load = 0; @(posedge clk); #1 rst = 0;\n");
    let cmp: Vec<String> = (0..n).map(|c| format!("ob{c} !== ol{c}")).collect();
    for e in 0..(m + l) {
        for r in 0..k {
            let v = if e < m { acts[e * k + r] } else { 0 };
            tb.push_str(&format!("    a{r} = 64'sh{:016X};\n", v as u64));
        }
        tb.push_str("    @(posedge clk); #1;\n");
        tb.push_str(&format!(
            "    if ({}) begin $display(\"MISMATCH at edge {e}\"); fails = fails + 1; end\n",
            cmp.join(" || ")
        ));
        if e + 1 >= l && (e + 1 - l) < m {
            let i = e + 1 - l;
            for c in 0..n {
                tb.push_str(&format!(
                    "    $display(\"OUT o {} %0d\", ol{c});\n",
                    i * n + c
                ));
            }
        }
    }
    tb.push_str(&format!("    $display(\"LATENCY {l}\");\n"));
    tb.push_str("    if (fails == 0) $display(\"PASS: loadable == baked\"); else $display(\"FAIL: %0d cycles differ\", fails);\n    $finish;\n  end\nendmodule\n");
    tb
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// OUTPUT-STATIONARY variant — the SN40L PCU's other dataflow. An R×C array of stationary accumulators;
// activations stream EAST, weights stream SOUTH, and PE(i,j) accumulates O[i][j] = Σ_t A[i][t]·W[t][j]
// in place over K contraction beats. Same PE core (`acc + a·w`), same K-independent pipelined critical path.
// ══════════════════════════════════════════════════════════════════════════════════════════════════

/// An output-stationary tile: an `r×c` grid of accumulators computing `O = A·W` with contraction depth `k`
/// (`A` is r×k row-major, `W` is k×c row-major, `O` is r×c row-major). Both operands stream in; O stays put.
#[derive(Clone, Debug)]
pub struct SystolicOs {
    pub r: usize,
    pub c: usize,
    pub k: usize,
}

impl SystolicOs {
    pub fn new(r: usize, c: usize, k: usize) -> Self {
        SystolicOs { r, c, k }
    }
}

/// Reference int8 GEMM for the output-stationary tile: `O[i][j] = Σ_t A[i][t]·W[t][j]` (wrapping i64).
pub fn eval_systolic_os(os: &SystolicOs, a: &[i64], w: &[i64], bits: u32) -> Vec<i64> {
    let (r, c, k) = (os.r, os.c, os.k);
    let mut o = vec![0i64; r * c];
    for i in 0..r {
        for j in 0..c {
            let mut acc = 0i64;
            for t in 0..k {
                let ai = trunc_signed(a[i * k + t], bits);
                let wj = trunc_signed(w[t * c + j], bits);
                acc = acc.wrapping_add(ai.wrapping_mul(wj));
            }
            o[i * c + j] = acc;
        }
    }
    o
}

/// The cycle at which the last accumulator (`i=r-1, j=c-1`) has absorbed its final product and is drainable.
pub fn pipelined_os_latency(r: usize, c: usize, k: usize) -> usize {
    k + r + c - 2
}

/// Cycle-accurate output-stationary model — mirrors the RTL register-for-register. Activation `A[i][t]` enters
/// row `i` skewed by `i`, flows east; weight `W[t][j]` enters col `j` skewed by `j`, flows south; every cycle
/// each PE does `acc <= acc + a_in·w_in` (skew/drain fill zeros contribute nothing). At PE(i,j) on cycle `T`
/// both operands carry contraction index `t = T-i-j`, so `acc[i][j]` converges to `O[i][j]`. Returns
/// `(O, latency)` with `latency = pipelined_os_latency`.
pub fn eval_systolic_os_pipelined(
    os: &SystolicOs,
    a: &[i64],
    w: &[i64],
    bits: u32,
) -> (Vec<i64>, usize) {
    eval_systolic_os_pipelined_acc(os, a, w, bits, ACC_BITS_FULL)
}

/// [`eval_systolic_os_pipelined`] with an explicit accumulator width `acc_bits` — each PE's accumulator wraps
/// (two's-complement) at `acc_bits`, exactly like the `acc_bits`-wide `acc_reg` in the emitted OS RTL.
pub fn eval_systolic_os_pipelined_acc(
    os: &SystolicOs,
    a: &[i64],
    w: &[i64],
    bits: u32,
    acc_bits: u32,
) -> (Vec<i64>, usize) {
    let (r, c, k) = (os.r, os.c, os.k);
    let mut acc = vec![0i64; r * c];
    let mut a_reg = vec![0i64; r * c];
    let mut w_reg = vec![0i64; r * c];
    let l = pipelined_os_latency(r, c, k);
    // run through the last drainable cycle
    for t_cyc in 0..=l {
        // skewed operand injections at the array edges
        let a_west = |i: usize| -> i64 {
            // A[i][t] enters row i at cycle t, delayed by i → present at cycle t_cyc when t_cyc-i ∈ [0,k)
            if t_cyc >= i && t_cyc - i < k {
                trunc_signed(a[i * k + (t_cyc - i)], bits)
            } else {
                0
            }
        };
        let w_north = |j: usize| -> i64 {
            if t_cyc >= j && t_cyc - j < k {
                trunc_signed(w[(t_cyc - j) * c + j], bits)
            } else {
                0
            }
        };
        let mut a_next = vec![0i64; r * c];
        let mut w_next = vec![0i64; r * c];
        let mut acc_next = acc.clone();
        for i in 0..r {
            for j in 0..c {
                let a_in = if j == 0 {
                    a_west(i)
                } else {
                    a_reg[i * c + (j - 1)]
                };
                let w_in = if i == 0 {
                    w_north(j)
                } else {
                    w_reg[(i - 1) * c + j]
                };
                a_next[i * c + j] = a_in;
                w_next[i * c + j] = w_in;
                acc_next[i * c + j] = trunc_signed(
                    acc[i * c + j].wrapping_add(a_in.wrapping_mul(w_in)),
                    acc_bits,
                );
            }
        }
        a_reg = a_next;
        w_reg = w_next;
        acc = acc_next;
    }
    (acc, l)
}

/// [`eval_systolic_os_pipelined`] without the latency — the `O` matrix only.
pub fn eval_systolic_os_pipelined_o(os: &SystolicOs, a: &[i64], w: &[i64], bits: u32) -> Vec<i64> {
    eval_systolic_os_pipelined(os, a, w, bits).0
}

/// Emit the output-stationary pipelined array (int8) as synthesizable Verilog. See
/// [`emit_systolic_os_pipelined_verilog_bits`].
pub fn emit_systolic_os_pipelined_verilog(name: &str, os: &SystolicOs) -> String {
    emit_systolic_os_pipelined_verilog_bits(name, os, 8)
}

/// Emit the output-stationary pipelined array at precision `bits` with a full (64-bit) accumulator.
pub fn emit_systolic_os_pipelined_verilog_bits(name: &str, os: &SystolicOs, bits: u32) -> String {
    emit_systolic_os_pipelined_verilog_bits_acc(name, os, bits, ACC_BITS_FULL)
}

/// Emit the register-staged output-stationary array at precision `bits`. Ports: `clk/rst`, `aw{i}` (west
/// activation stream for row i), `wn{j}` (north weight stream for col j), and `acc{i}_{j}` (the stationary
/// accumulator outputs, continuously readable). Per PE: `a_reg` (flows east), `w_reg` (flows south),
/// `acc_reg <= acc_reg + a_in·w_in`. The only combinational path between flip-flops is one mul + one add —
/// K-independent, exactly like the weight-stationary pipelined tile. Skew of the streamed operands is applied
/// on-chip (row i / col j delayed i / j cycles), so the caller drives un-skewed streams.
pub fn emit_systolic_os_pipelined_verilog_bits_acc(
    name: &str,
    os: &SystolicOs,
    bits: u32,
    acc_bits: u32,
) -> String {
    let (r, c) = (os.r, os.c);
    let hi = bits.min(64) - 1;
    // both operands stream, so both are narrowed to their real precision → a `bits×bits` multiplier per PE;
    // the accumulator is `acc_bits` wide (bit-identical to the acc_bits model; lossless while it fits).
    let ow = bits.min(64); // streamed-operand width
    let pw = (2 * bits).min(acc_bits); // product width
    let ah = acc_bits.min(64) - 1; // accumulator MSB
    let mut s = String::new();
    s.push_str(&format!("module {name}(\n  input clk,\n  input rst,\n"));
    for i in 0..r {
        s.push_str(&format!("  input signed [63:0] aw{i},\n"));
    }
    for j in 0..c {
        s.push_str(&format!("  input signed [63:0] wn{j},\n"));
    }
    for i in 0..r {
        for j in 0..c {
            let last = i + 1 == r && j + 1 == c;
            let comma = if last { "" } else { "," };
            s.push_str(&format!("  output signed [63:0] acc{i}_{j}{comma}\n"));
        }
    }
    s.push_str(");\n");
    // truncated edge injections (operand-width)
    for i in 0..r {
        s.push_str(&format!(
            "  wire signed [{}:0] saw{i} = $signed(aw{i}[{hi}:0]);\n",
            ow - 1
        ));
    }
    for j in 0..c {
        s.push_str(&format!(
            "  wire signed [{}:0] swn{j} = $signed(wn{j}[{hi}:0]);\n",
            ow - 1
        ));
    }
    // input skew: row i activation delayed i cycles; col j weight delayed j cycles (operand-width)
    for i in 0..r {
        for d in 0..i {
            s.push_str(&format!("  reg signed [{}:0] askew_{i}_{d};\n", ow - 1));
        }
    }
    for j in 0..c {
        for d in 0..j {
            s.push_str(&format!("  reg signed [{}:0] wskew_{j}_{d};\n", ow - 1));
        }
    }
    for i in 0..r {
        if i == 0 {
            s.push_str(&format!("  wire signed [{}:0] a_edge0 = saw0;\n", ow - 1));
        } else {
            s.push_str(&format!(
                "  wire signed [{}:0] a_edge{i} = askew_{i}_{};\n",
                ow - 1,
                i - 1
            ));
        }
    }
    for j in 0..c {
        if j == 0 {
            s.push_str(&format!("  wire signed [{}:0] w_edge0 = swn0;\n", ow - 1));
        } else {
            s.push_str(&format!(
                "  wire signed [{}:0] w_edge{j} = wskew_{j}_{};\n",
                ow - 1,
                j - 1
            ));
        }
    }
    // PE registers + in-wires: activation (east) + weight (south) are operand-width, accumulator is 64-bit
    for i in 0..r {
        for j in 0..c {
            s.push_str(&format!("  reg signed [{}:0] a_reg_{i}_{j};\n", ow - 1));
            s.push_str(&format!("  reg signed [{}:0] w_reg_{i}_{j};\n", ow - 1));
            s.push_str(&format!("  reg signed [{ah}:0] acc_reg_{i}_{j};\n"));
            let a_in = if j == 0 {
                format!("a_edge{i}")
            } else {
                format!("a_reg_{i}_{}", j - 1)
            };
            let w_in = if i == 0 {
                format!("w_edge{j}")
            } else {
                format!("w_reg_{}_{j}", i - 1)
            };
            s.push_str(&format!(
                "  wire signed [{}:0] a_in_{i}_{j} = {a_in};\n",
                ow - 1
            ));
            s.push_str(&format!(
                "  wire signed [{}:0] w_in_{i}_{j} = {w_in};\n",
                ow - 1
            ));
            s.push_str(&format!(
                "  wire signed [{}:0] prod_{i}_{j} = a_in_{i}_{j} * w_in_{i}_{j};\n",
                pw - 1
            ));
            s.push_str(&format!("  assign acc{i}_{j} = acc_reg_{i}_{j};\n"));
        }
    }
    s.push_str("  always @(posedge clk) begin\n    if (rst) begin\n");
    for i in 0..r {
        for d in 0..i {
            s.push_str(&format!("      askew_{i}_{d} <= {ow}'sd0;\n"));
        }
    }
    for j in 0..c {
        for d in 0..j {
            s.push_str(&format!("      wskew_{j}_{d} <= {ow}'sd0;\n"));
        }
    }
    for i in 0..r {
        for j in 0..c {
            s.push_str(&format!("      a_reg_{i}_{j} <= {ow}'sd0;\n"));
            s.push_str(&format!("      w_reg_{i}_{j} <= {ow}'sd0;\n"));
            s.push_str(&format!("      acc_reg_{i}_{j} <= {acc_bits}'sd0;\n"));
        }
    }
    s.push_str("    end else begin\n");
    for i in 1..r {
        s.push_str(&format!("      askew_{i}_0 <= saw{i};\n"));
        for d in 1..i {
            s.push_str(&format!("      askew_{i}_{d} <= askew_{i}_{};\n", d - 1));
        }
    }
    for j in 1..c {
        s.push_str(&format!("      wskew_{j}_0 <= swn{j};\n"));
        for d in 1..j {
            s.push_str(&format!("      wskew_{j}_{d} <= wskew_{j}_{};\n", d - 1));
        }
    }
    for i in 0..r {
        for j in 0..c {
            s.push_str(&format!("      a_reg_{i}_{j} <= a_in_{i}_{j};\n"));
            s.push_str(&format!("      w_reg_{i}_{j} <= w_in_{i}_{j};\n"));
            s.push_str(&format!(
                "      acc_reg_{i}_{j} <= acc_reg_{i}_{j} + prod_{i}_{j};\n"
            ));
        }
    }
    s.push_str("    end\n  end\nendmodule\n");
    s
}

/// The stationary weights a signed-off weight-stationary tile bakes: a fixed int8 pattern, `k*n` row-major.
pub fn bist_tile_weights(k: usize, n: usize) -> Vec<i64> {
    (0..k * n)
        .map(|i| {
            let mut x = (i as u64).wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(1);
            x ^= x >> 29;
            ((x % 255) as i64) - 127
        })
        .collect()
}

/// The self-test wrapper a weight-stationary tile `core` is signed off in: a 64-bit LFSR drives the `k` int8
/// activation lanes and the `n` registered outputs fold into a MISR, so only `clk`, `rst` and `sig` cross the block's
/// boundary and no pad shapes its timing.
pub fn emit_systolic_bist_wrapper(core: &str, k: usize, n: usize) -> String {
    let a_wires: String = (0..k)
        .map(|r| format!("  wire signed [63:0] a{r} = $signed(lfsr[{}:{}]);\n", r * 8 % 57 + 7, r * 8 % 57))
        .collect();
    let o_wires: String = (0..n).map(|c| format!("  wire signed [63:0] o{c};\n")).collect();
    let aconn: Vec<String> = (0..k).map(|r| format!(".a{r}(a{r})")).collect();
    let oconn: Vec<String> = (0..n).map(|c| format!(".o{c}(o{c})")).collect();
    let ofold: String = (0..n).map(|c| format!("o{c}")).collect::<Vec<_>>().join(" ^ ");
    format!(
        "module {core}_realbist(input clk, input rst, output sig);\n\
         \x20 reg [63:0] lfsr;\n  reg [63:0] misr;\n{a_wires}{o_wires}\
         \x20 {core} arr(.clk(clk), .rst(rst), {aconn}, {oconn});\n\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin\n      lfsr <= 64'hD1CEF00D12345678;\n      misr <= 64'd0;\n    end else begin\n\
         \x20     lfsr <= {{lfsr[62:0], lfsr[63]^lfsr[62]^lfsr[60]^lfsr[59]}};\n\
         \x20     misr <= {{misr[62:0], misr[63]}} ^ ({ofold});\n    end\n  end\n\
         \x20 assign sig = ^misr;\nendmodule\n",
        aconn = aconn.join(", "),
        oconn = oconn.join(", "),
    )
}

/// The self-test wrapper an output-stationary tile `core` is signed off in: the LFSR drives the `r` west activation
/// and `c` north weight lanes, and the `r × c` stationary accumulators fold into the MISR through registers, row by
/// row. Each row's accumulators fold into a row word, the row words into two half-folds (`foldA`, `foldB`), and the
/// MISR XORs those. The fold stays off the critical path, so the block's critical path is the array's PE, and no
/// register reads more than one row's accumulators, so no fold gathers every accumulator at one point of the die.
pub fn emit_systolic_os_bist_wrapper(core: &str, r: usize, c: usize) -> String {
    let aw: String = (0..r)
        .map(|i| format!("  wire signed [63:0] aw{i} = $signed(lfsr[{}:{}]);\n", i * 8 % 57 + 7, i * 8 % 57))
        .collect();
    let wn: String = (0..c)
        .map(|j| format!("  wire signed [63:0] wn{j} = $signed(lfsr[{}:{}]);\n", (j * 8 + 4) % 57 + 7, (j * 8 + 4) % 57))
        .collect();
    let accw: String = (0..r)
        .flat_map(|i| (0..c).map(move |j| format!("  wire signed [63:0] acc{i}_{j};\n")))
        .collect();
    let conns: Vec<String> = (0..r)
        .map(|i| format!(".aw{i}(aw{i})"))
        .chain((0..c).map(|j| format!(".wn{j}(wn{j})")))
        .chain((0..r).flat_map(|i| (0..c).map(move |j| format!(".acc{i}_{j}(acc{i}_{j})"))))
        .collect();
    let fold = |rows: std::ops::Range<usize>| {
        if rows.is_empty() {
            "64'd0".to_string()
        } else {
            rows.map(|i| format!("rowfold{i}")).collect::<Vec<_>>().join(" ^ ")
        }
    };
    let half = r.div_ceil(2);
    let (fold_a, fold_b) = (fold(0..half), fold(half..r));
    let row_regs: String = (0..r).map(|i| format!("  reg [63:0] rowfold{i};\n")).collect();
    let row_resets: String = (0..r).map(|i| format!("      rowfold{i} <= 64'd0;\n")).collect();
    let row_steps: String = (0..r)
        .map(|i| {
            let row = (0..c).map(|j| format!("acc{i}_{j}")).collect::<Vec<_>>().join(" ^ ");
            format!("      rowfold{i} <= {row};\n")
        })
        .collect();
    format!(
        "module {core}_realbist(input clk, input rst, output sig);\n\
         \x20 reg [63:0] lfsr;\n  reg [63:0] misr;\n  reg [63:0] foldA;\n  reg [63:0] foldB;\n{row_regs}{aw}{wn}{accw}\
         \x20 {core} arr(.clk(clk), .rst(rst), {conns});\n\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin\n      lfsr <= 64'hD1CEF00D12345678;\n      misr <= 64'd0;\n      foldA <= 64'd0;\n      foldB <= 64'd0;\n{row_resets}    end else begin\n\
         \x20     lfsr <= {{lfsr[62:0], lfsr[63]^lfsr[62]^lfsr[60]^lfsr[59]}};\n\
{row_steps}\
         \x20     foldA <= {fold_a};\n      foldB <= {fold_b};\n\
         \x20     misr <= {{misr[62:0], misr[63]}} ^ foldA ^ foldB;\n    end\n  end\n\
         \x20 assign sig = ^misr;\nendmodule\n",
        conns = conns.join(", "),
    )
}

/// When row `i`'s results of a streamed tile are readable: `period` cycles after the tile's first beat
/// the next tile's first beat arrives and moves each PE's finished sum into its result register as it
/// passes, `i + j` cycles in, where it stays `period` cycles. Every column of row `i` holds it
/// `period + i + c` cycles after the tile's first beat: column `j` from `period + i + j + 1` until
/// `2·period + i + j`, delayed where that window would close before column `c - 1`'s opens
/// ([`os_streaming_deskew`]).
pub fn os_streaming_result_offset(period: usize, i: usize, c: usize) -> usize {
    period + i + c
}

/// The cycles column `j`'s results wait so a whole row is readable in one cycle at
/// [`os_streaming_result_offset`]: `c - k - j` where that is positive. A tile's period is at least its
/// contraction `k`, and column `j` holds its result for a period from `period + i + j + 1`, so it covers
/// the row's read cycle with this delay at every period a tile can have; an array no wider than its
/// contraction waits for nothing.
pub fn os_streaming_deskew(os: &SystolicOs, j: usize) -> usize {
    os.c.saturating_sub(os.k + j)
}

/// Emit the output-stationary array for tile after tile. Ports: `clk/rst`, `first` (high on each tile's
/// first contraction beat), `aw{i}` / `wn{j}` (the un-skewed operand streams of [`emit_systolic_os_pipelined_verilog_bits_acc`])
/// and `res{i}_{j}` (the finished sums of the previous tile). `first` rides the activation wavefront, so
/// it reaches PE(i,j) with the tile's first products: there the PE moves its sum into `res_reg` and
/// restarts from that product — no reset and no idle beat between tiles. Column `j`'s result waits
/// [`os_streaming_deskew`] cycles, so each row is readable all at once at [`os_streaming_result_offset`].
pub fn emit_systolic_os_streaming_verilog(name: &str, os: &SystolicOs, bits: u32, acc_bits: u32) -> String {
    let (r, c) = (os.r, os.c);
    let hi = bits.min(64) - 1;
    let ow = bits.min(64);
    let pw = (2 * bits).min(acc_bits);
    let ah = acc_bits.min(64) - 1;
    let mut s = String::new();
    s.push_str(&format!("module {name}(\n  input clk,\n  input rst,\n  input first,\n"));
    for i in 0..r {
        s.push_str(&format!("  input signed [63:0] aw{i},\n"));
    }
    for j in 0..c {
        s.push_str(&format!("  input signed [63:0] wn{j},\n"));
    }
    for i in 0..r {
        for j in 0..c {
            let comma = if i + 1 == r && j + 1 == c { "" } else { "," };
            s.push_str(&format!("  output signed [63:0] res{i}_{j}{comma}\n"));
        }
    }
    s.push_str(");\n");
    for i in 0..r {
        s.push_str(&format!("  wire signed [{}:0] saw{i} = $signed(aw{i}[{hi}:0]);\n", ow - 1));
    }
    for j in 0..c {
        s.push_str(&format!("  wire signed [{}:0] swn{j} = $signed(wn{j}[{hi}:0]);\n", ow - 1));
    }
    // Input skew: row i's activation and first flag delayed i cycles, column j's weight delayed j.
    for i in 0..r {
        for d in 0..i {
            s.push_str(&format!("  reg signed [{}:0] askew_{i}_{d};\n  reg fskew_{i}_{d};\n", ow - 1));
        }
    }
    for j in 0..c {
        for d in 0..j {
            s.push_str(&format!("  reg signed [{}:0] wskew_{j}_{d};\n", ow - 1));
        }
    }
    for i in 0..r {
        if i == 0 {
            s.push_str(&format!("  wire signed [{}:0] a_edge0 = saw0;\n  wire f_edge0 = first;\n", ow - 1));
        } else {
            s.push_str(&format!(
                "  wire signed [{}:0] a_edge{i} = askew_{i}_{d};\n  wire f_edge{i} = fskew_{i}_{d};\n",
                ow - 1,
                d = i - 1
            ));
        }
    }
    for j in 0..c {
        if j == 0 {
            s.push_str(&format!("  wire signed [{}:0] w_edge0 = swn0;\n", ow - 1));
        } else {
            s.push_str(&format!("  wire signed [{}:0] w_edge{j} = wskew_{j}_{};\n", ow - 1, j - 1));
        }
    }
    // PEs: activation and first flag flow east, weight south; the sum restarts where the flag arrives.
    for i in 0..r {
        for j in 0..c {
            s.push_str(&format!(
                "  reg signed [{o}:0] a_reg_{i}_{j};\n  reg signed [{o}:0] w_reg_{i}_{j};\n  reg f_reg_{i}_{j};\n  reg signed [{ah}:0] acc_reg_{i}_{j};\n  reg signed [{ah}:0] res_reg_{i}_{j};\n",
                o = ow - 1
            ));
            let (a_in, f_in) = if j == 0 {
                (format!("a_edge{i}"), format!("f_edge{i}"))
            } else {
                (format!("a_reg_{i}_{}", j - 1), format!("f_reg_{i}_{}", j - 1))
            };
            let w_in = if i == 0 { format!("w_edge{j}") } else { format!("w_reg_{}_{j}", i - 1) };
            s.push_str(&format!(
                "  wire signed [{o}:0] a_in_{i}_{j} = {a_in};\n  wire f_in_{i}_{j} = {f_in};\n  wire signed [{o}:0] w_in_{i}_{j} = {w_in};\n  wire signed [{}:0] prod_{i}_{j} = a_in_{i}_{j} * w_in_{i}_{j};\n",
                pw - 1,
                o = ow - 1
            ));
            let d = os_streaming_deskew(os, j);
            for k in 0..d {
                s.push_str(&format!("  reg signed [{ah}:0] dres_{i}_{j}_{k};\n"));
            }
            let src = if d == 0 { format!("res_reg_{i}_{j}") } else { format!("dres_{i}_{j}_{}", d - 1) };
            s.push_str(&format!("  assign res{i}_{j} = {src};\n"));
        }
    }
    s.push_str("  always @(posedge clk) begin\n    if (rst) begin\n");
    for i in 0..r {
        for d in 0..i {
            s.push_str(&format!("      askew_{i}_{d} <= {ow}'sd0;\n      fskew_{i}_{d} <= 1'b0;\n"));
        }
    }
    for j in 0..c {
        for d in 0..j {
            s.push_str(&format!("      wskew_{j}_{d} <= {ow}'sd0;\n"));
        }
    }
    for i in 0..r {
        for j in 0..c {
            s.push_str(&format!(
                "      a_reg_{i}_{j} <= {ow}'sd0;\n      w_reg_{i}_{j} <= {ow}'sd0;\n      f_reg_{i}_{j} <= 1'b0;\n      acc_reg_{i}_{j} <= {a}'sd0;\n      res_reg_{i}_{j} <= {a}'sd0;\n",
                a = ah + 1
            ));
            for k in 0..os_streaming_deskew(os, j) {
                s.push_str(&format!("      dres_{i}_{j}_{k} <= {a}'sd0;\n", a = ah + 1));
            }
        }
    }
    s.push_str("    end else begin\n");
    for i in 1..r {
        s.push_str(&format!("      askew_{i}_0 <= saw{i};\n      fskew_{i}_0 <= first;\n"));
        for d in 1..i {
            s.push_str(&format!(
                "      askew_{i}_{d} <= askew_{i}_{p};\n      fskew_{i}_{d} <= fskew_{i}_{p};\n",
                p = d - 1
            ));
        }
    }
    for j in 1..c {
        s.push_str(&format!("      wskew_{j}_0 <= swn{j};\n"));
        for d in 1..j {
            s.push_str(&format!("      wskew_{j}_{d} <= wskew_{j}_{};\n", d - 1));
        }
    }
    for i in 0..r {
        for j in 0..c {
            s.push_str(&format!(
                "      a_reg_{i}_{j} <= a_in_{i}_{j};\n      w_reg_{i}_{j} <= w_in_{i}_{j};\n      f_reg_{i}_{j} <= f_in_{i}_{j};\n\
                 \x20     if (f_in_{i}_{j}) begin\n        res_reg_{i}_{j} <= acc_reg_{i}_{j};\n        acc_reg_{i}_{j} <= prod_{i}_{j};\n      end else begin\n        acc_reg_{i}_{j} <= acc_reg_{i}_{j} + prod_{i}_{j};\n      end\n"
            ));
            let d = os_streaming_deskew(os, j);
            if d > 0 {
                s.push_str(&format!("      dres_{i}_{j}_0 <= res_reg_{i}_{j};\n"));
                for k in 1..d {
                    s.push_str(&format!("      dres_{i}_{j}_{k} <= dres_{i}_{j}_{};\n", k - 1));
                }
            }
        }
    }
    s.push_str("    end\n  end\nendmodule\n");
    s
}
