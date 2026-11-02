//! gpu_simt_rtl — the GPU processor edge: a tuneable SIMT data-parallel engine in silicon.
//!
//! The NPU (`fabric_rtl::LoopFabric`) is a REDUCTION — one body streamed by a loop counter,
//! many→one. A GPU is the opposite: **N independent spatial lanes, each running the SAME
//! compiled straight-line kernel (the "shader") on its OWN data, producing N INDEPENDENT
//! outputs (a MAP)**, each lane gated by its own predicate bit (SIMT divergence). The lanes are
//! WIRED (spatial) and all compute in ONE cycle — no warp scheduler, no fetch/decode/dispatch —
//! exactly the deterministic Futamura primitive the other edges use, applied across lanes.
//!
//! One parametrized engine specializes to every mode (the "best of all worlds"):
//!   * **SIMT** — `predicated = true`: a per-lane mask; a masked lane contributes 0 (inert).
//!   * **SIMD** — the special case where every lane is unmasked (`mask` all-ones).
//!   * **map-then-reduce** — `reduce = Some(op)`: the N lane outputs feed a cross-lane
//!     warp-reduce, which is exactly the NPU's accumulator (an `AluOp` fold) placed spatially.
//!   * **uniform vs varying** — each kernel input is either a broadcast scalar shared by every
//!     lane (a GPU "uniform") or a per-lane vector element (a "varying").
//!
//! **Physical interface.** Exposing every lane's I/O as top-level ports would be thousands of
//! pins (unroutable on an FPGA, an absurd pad ring on an ASIC). Like the other accelerators
//! (`dram_executor`, `infinichip`), the tile has a NARROW streaming interface — a 64-bit `din`
//! loaded slot-by-slot into an internal lane register file, a `go` pulse that runs all lanes in
//! parallel (the spatial SIMT step), and a `din`-width `dout` read back by `raddr`. The parallel
//! COMPUTE is the GPU essence; the streaming I/O is how a real accelerator talks to memory.
//!
//! A lane IS the existing certified straight-line fabric residual (`fabric_rtl::FabricOp`), so
//! lane arithmetic is not reinvented: the kernel is compiled from English by the REAL compiler
//! (`residual_from_source`), each lane is a verbatim spatial copy, and the map is proven ≡ the
//! VM lane-by-lane — by direct model comparison and, on a narrow width, EXHAUSTIVELY by our own
//! certified SAT (`fabric_to_bounded` + `prove_unsat`). 64-bit signed wrapping, bit-identical to
//! the VM's `Int` (its fixed-width projection), matching `fabric_rtl`.

use crate::codegen_sva::fabric_rtl::{
    eval_fabric, fabric_to_bounded, residual_from_source, AluOp, FabricOp, Src,
};
use crate::codegen_sva::fanout::bounded_fanout_select;
use crate::codegen_sva::mul_tree::{mul_tree, mul_tree_levels};
use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};

/// The datapath multiply strategy for the lane's `a·x + b` arithmetic — a first-class, tuneable,
/// SAT-certified knob. Every strategy is functionally identical (`a·x + b`, 64-bit wrapping); they
/// trade cycle count against physical (routing / antenna) friendliness.
#[derive(Clone, Debug, PartialEq)]
pub enum LaneArith {
    /// Per-op combinational body — the whole kernel evaluates in ONE cycle. A WIDE combinational
    /// `*`: fast (single-cycle) but met1-antenna-dense on Sky130.
    Combinational,
    /// The 64×64 multiply STRUCTURALLY split into three 32×32 partial products, pipelined over
    /// `stages` registered stages — real GPU-ALU pipelining. Smaller multipliers → shorter nets,
    /// but still a WIDE combinational `*` per partial product.
    DecomposedFma { stages: usize },
    /// A SEQUENTIAL shift-add multiply over the datapath width: ONE reused adder per lane, one
    /// bit of the multiplier consumed per cycle (WIDTH cycles). Bounded fan-out (like a word-serial
    /// multiplier) → NO wide `*` anywhere → antenna-trivial. Only valid for the `a·x+b` kernel.
    ///
    /// `select_slice` is the fan-out / antenna knob. The per-cycle conditional-add partial product
    /// `mp[0] ? mc : 0` is emitted as `select_slice` `(* keep *)` copies of the multiplier LSB, each
    /// gating its OWN `WIDTH / select_slice`-bit group of `mc` — so no single select net spiders
    /// across the full datapath (the met1-antenna root cause). SMALLER `select_slice` = each copy
    /// gates FEWER mux bits = lower fan-out = less antenna, at the cost of MORE `(* keep *)` wires;
    /// LARGER = the reverse. It MUST divide the datapath width evenly (`WIDTH % select_slice == 0`,
    /// asserted at emit time), so the byte-groups tile `mc` exactly with no dropped or misaligned bit.
    SequentialFma { select_slice: usize },
    /// ONE multiply shared by every lane, an aligned tree of adds over the 64 partial products
    /// (`mul_tree`, no `*`) registered after every `every`-th level and after the last. Each lane's `a·x`
    /// enters it in the clock the lane's `x` is loaded, so the products stream out while the rest of
    /// the operands load; as each emerges its lane's masked `+ b` is registered and folded into the
    /// warp-reduce. Only valid for the `a·x+b` kernel with a per-lane `x`.
    Streamed { every: usize },
}

/// A tuneable SIMT data-parallel engine — the GPU tile.
#[derive(Clone, Debug)]
pub struct SimtEngine {
    /// Lane width N (the SIMD/SIMT width). Tuneable — the whole engine is parametric in it.
    pub n_lanes: usize,
    /// The kernel's input arity (parameter count of the compiled lane program).
    pub n_in: usize,
    /// `uniform[k]` = input k is a broadcast scalar shared by every lane (a GPU "uniform");
    /// otherwise input k is a per-lane "varying" (its own value per lane). Length `n_in`.
    pub uniform: Vec<bool>,
    /// The compiled straight-line kernel (the "shader") — one `FabricOp` per SSA value, from
    /// the real compiler's dispatch-free residual. Every lane runs this verbatim.
    pub body: Vec<FabricOp>,
    /// SSA index of the value driving each lane's output.
    pub result: usize,
    /// Per-lane predication (SIMT divergence): a `mask` input gates each lane (masked → 0).
    pub predicated: bool,
    /// Optional cross-lane reduce tail (the warp-reduce) — the NPU accumulator, placed spatially.
    pub reduce: Option<AluOp>,
    /// The lane's multiply strategy — a tuneable, SAT-certified knob (see `LaneArith`). All three
    /// strategies compute the same `a·x + b` (64-bit wrapping); they trade cycle count against
    /// physical friendliness (`SequentialFma` is antenna-trivial, the combinational forms are dense).
    pub arith: LaneArith,
}

impl SimtEngine {
    /// Build a SIMT engine from an English lane program. `fn_name` is the kernel (shader); its
    /// straight-line body becomes the per-lane datapath. `uniform` marks which parameters are
    /// broadcast scalars (shared) vs per-lane vectors; its length must equal the kernel arity.
    pub fn from_lane_source(
        src: &str,
        fn_name: &str,
        n_lanes: usize,
        uniform: Vec<bool>,
        predicated: bool,
        reduce: Option<AluOp>,
    ) -> SimtEngine {
        let (body, result, n_in) = residual_from_source(src, fn_name);
        assert_eq!(
            uniform.len(),
            n_in,
            "uniform flags ({}) must match kernel arity ({n_in})",
            uniform.len()
        );
        SimtEngine {
            n_lanes,
            n_in,
            uniform,
            body,
            result,
            predicated,
            reduce,
            arith: LaneArith::SequentialFma { select_slice: 8 },
        }
    }
}

/// One streamed load slot: a broadcast uniform input, the packed mask word, or a per-lane
/// varying input. The load order (uniforms, then mask, then per-lane varyings) is the single
/// source of truth shared by the RTL load FSM, the testbench, and the golden hex writer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slot {
    Uniform(usize),
    Mask,
    Varying(usize, usize), // (lane j, input k)
}

/// The canonical load-slot sequence for an engine.
pub fn load_slots(eng: &SimtEngine) -> Vec<Slot> {
    let mut s = Vec::new();
    for k in 0..eng.n_in {
        if eng.uniform[k] {
            s.push(Slot::Uniform(k));
        }
    }
    if eng.predicated {
        s.push(Slot::Mask);
    }
    for j in 0..eng.n_lanes {
        for k in 0..eng.n_in {
            if !eng.uniform[k] {
                s.push(Slot::Varying(j, k));
            }
        }
    }
    s
}

/// A residual operand named from lane `j`'s internal registers — a uniform input is the shared
/// reg `u{k}`, a varying input is the per-lane reg `x{k}_l{j}`, an SSA value is `v{k}_l{j}`, a
/// constant is baked in.
fn src_reg(uniform: &[bool], s: Src, j: usize) -> String {
    match s {
        Src::In(k) => {
            if uniform[k] {
                format!("u{k}")
            } else {
                format!("x{k}_l{j}")
            }
        }
        Src::Val(k) => format!("v{k}_l{j}"),
        Src::Const(c) => {
            if c < 0 {
                format!("-64'sd{}", (c as i128).unsigned_abs())
            } else {
                format!("64'sd{c}")
            }
        }
    }
}

/// The Verilog separator for a reduce op (masked lanes already emit 0 = Add's identity).
fn reduce_sep(op: AluOp) -> &'static str {
    match op {
        AluOp::Add => " + ",
        AluOp::Sub => " - ",
        AluOp::Mul => " * ",
        AluOp::And => " & ",
        AluOp::Or => " | ",
        AluOp::Xor => " ^ ",
        AluOp::Shl => " << ",
        AluOp::Shr => " >>> ",
        AluOp::UShr => " >> ",
    }
}

/// Emit the SIMT engine as synthesizable RTL: a narrow streaming interface (a 64-bit `din` loaded
/// slot-by-slot into an internal lane register file, a `go` pulse that runs ALL lanes in one cycle
/// — the spatial SIMT step — and a `dout` read back by `raddr`) wrapped around N verbatim copies
/// of the certified kernel, with per-lane predication and an optional warp-reduce. Module name
/// `gpu_{name}`. Readback: `raddr` in `0..N` selects a lane output; `raddr == N` selects the
/// reduced scalar (when a reduce is present).
pub fn emit_gpu_simt_verilog(name: &str, eng: &SimtEngine) -> String {
    if let LaneArith::Streamed { every } = eng.arith {
        return emit_gpu_simt_streamed_verilog(name, eng, every);
    }
    let n = eng.n_lanes;
    // The i64 datapath width — the `a`/`x`/`b` register width, and the sequential multiply's step
    // count. This is INTENTIONALLY fixed at 64: it is the VM's fixed-width `Int` projection (the same
    // width `fabric_rtl` and the ≡VM/SAT locks certify against), NOT a free tuning knob — moving to
    // 32- or 128-bit is a value-TYPE change (a different `Int` projection), not an emitter parameter.
    const WIDTH: usize = 64;
    // Translate the strategy knob: `p` = pipeline stages of the decomposed FMA (0 = not decomposed),
    // `is_seq` = the sequential shift-add, `select_slice` = its per-cycle-select fan-out slice (only
    // meaningful when `is_seq`). Combinational is the `p == 0 && !is_seq` case.
    let (p, is_seq, select_slice) = match &eng.arith {
        LaneArith::Combinational => (0usize, false, 0usize),
        LaneArith::DecomposedFma { stages } => (*stages, false, 0usize),
        LaneArith::SequentialFma { select_slice } => (0usize, true, *select_slice),
        LaneArith::Streamed { .. } => unreachable!("the streamed engine is emitted above"),
    };
    if is_seq {
        assert_eq!(
            eng.n_in, 3,
            "the sequential (shift-add) FMA needs a 3-input a·x+b kernel"
        );
        // The fan-out slice must tile the datapath exactly — each of the `select_slice` `(* keep *)`
        // copies gates a `WIDTH / select_slice`-bit group of `mc`, so no bit is dropped or misaligned.
        assert!(
            select_slice > 0,
            "SequentialFma select_slice must be positive"
        );
        assert_eq!(
            WIDTH % select_slice,
            0,
            "SequentialFma select_slice ({select_slice}) must divide the datapath width ({WIDTH}) evenly"
        );
    }
    let slots = load_slots(eng);

    // ---- register declarations ----
    let mut decls = String::new();
    for k in 0..eng.n_in {
        if eng.uniform[k] {
            decls.push_str(&format!("  reg [63:0] u{k};\n"));
        }
    }
    if eng.predicated {
        decls.push_str(&format!("  reg [{}:0] mask;\n", n - 1));
    }
    for j in 0..n {
        for k in 0..eng.n_in {
            if !eng.uniform[k] {
                decls.push_str(&format!("  reg [63:0] x{k}_l{j};\n"));
            }
        }
    }
    // Pipelined FMA (p > 0): the 64×64 multiply is STRUCTURALLY split into three 32×32 partial
    // products, held in these stage-1 registers. Smaller multipliers → shorter internal nets →
    // far less antenna (no diode flood) → a clean, fast signoff. Only for the a·x + b kernel.
    if p > 0 {
        assert_eq!(
            eng.n_in, 3,
            "the pipelined (decomposed) FMA needs a 3-input a·x+b kernel"
        );
        for j in 0..n {
            decls.push_str(&format!(
                "  reg [63:0] pll_l{j};\n  reg [63:0] phl_l{j};\n  reg [63:0] plh_l{j};\n"
            ));
        }
    }
    // Sequential FMA: per lane, the running accumulator `acc`, the left-shifting multiplicand `mc`
    // (= a), and the right-shifting multiplier `mp` (= x). ONE reused adder per lane consumes one
    // bit of `mp` per cycle — a word-serial multiplier, bounded fan-out, antenna-trivial. No `*`.
    if is_seq {
        for j in 0..n {
            decls.push_str(&format!(
                "  reg [63:0] acc_l{j};\n  reg [63:0] mc_l{j};\n  reg [63:0] mp_l{j};\n"
            ));
        }
    }
    for j in 0..n {
        decls.push_str(&format!("  reg [63:0] outr{j};\n"));
    }
    if eng.reduce.is_some() {
        decls.push_str("  reg [63:0] redr;\n");
    }
    decls.push_str("  reg [15:0] lc;\n");
    if p > 0 {
        decls.push_str("  reg [7:0] pcnt;\n  reg running;\n");
    }
    if is_seq {
        decls.push_str("  reg [7:0] scnt;\n  reg running;\n");
    }

    // ---- combinational lane datapaths (read the loaded regs) ----
    let mut lanes = String::new();
    for j in 0..n {
        if is_seq {
            // Sequential a·x + b (a=In0, x=In1, b=In2). The multiply is folded into the FSM's
            // shift-add over `acc`/`mc`/`mp`; here the combinational tail just adds `b` and applies
            // the per-lane predicate. `acc` holds a·x after WIDTH steps.
            //
            // The step's conditional-add partial product `mp[0] ? mc : 0` is emitted here as a
            // fanout-bounded, `(* keep *)`-sliced wire `pp_l{j}`: `select_slice` copies of the select
            // bit `mp[0]`, each gating its own WIDTH/select_slice-bit group of `mc`. Every copy ==
            // mp[0], so `pp == (mp[0] ? mc : 0)` bit-for-bit — the FSM add becomes `acc += pp`. This
            // bounds the select net's fanout (the met1-antenna root cause) without changing the value.
            // The slicing IS the reusable `bounded_fanout_select` primitive — this is its founding
            // caller (the pit-of-success helper any wide one-bit select should route through).
            let (msel_decls, pp_expr) = bounded_fanout_select(
                "msel",
                &format!("_l{j}"),
                &format!("mp_l{j}[0]"),
                &format!("mc_l{j}"),
                WIDTH,
                select_slice,
            );
            lanes.push_str(&msel_decls);
            lanes.push_str(&format!("  wire [{}:0] pp_l{j} = {pp_expr};\n", WIDTH - 1));
            let b = src_reg(&eng.uniform, Src::In(2), j);
            lanes.push_str(&format!("  wire [63:0] k_l{j} = acc_l{j} + {b};\n"));
            let g = if eng.predicated {
                format!("mask[{j}] ? k_l{j} : 64'sd0")
            } else {
                format!("k_l{j}")
            };
            lanes.push_str(&format!("  wire [63:0] g_l{j} = {g};\n"));
        } else if p > 0 {
            // Decomposed a·x + b (a=In0, x=In1, b=In2). Split each operand into 32-bit halves; the
            // three 32×32 partial products (pll/phl/plh) are registered in `pipe_latch`. Then, mod
            // 2^64: a·x = al·xl + ((ah·xl + al·xh) mod 2^32) << 32 (the ah·xh·2^64 term vanishes).
            let a = src_reg(&eng.uniform, Src::In(0), j);
            let x = src_reg(&eng.uniform, Src::In(1), j);
            let b = src_reg(&eng.uniform, Src::In(2), j);
            lanes.push_str(&format!(
                "  wire [31:0] al_l{j} = {a}[31:0]; wire [31:0] ah_l{j} = {a}[63:32];\n"
            ));
            lanes.push_str(&format!(
                "  wire [31:0] xl_l{j} = {x}[31:0]; wire [31:0] xh_l{j} = {x}[63:32];\n"
            ));
            lanes.push_str(&format!(
                "  wire [63:0] mid_l{j} = {{(phl_l{j}[31:0] + plh_l{j}[31:0]), 32'h0}};\n"
            ));
            lanes.push_str(&format!("  wire [63:0] ax_l{j} = pll_l{j} + mid_l{j};\n"));
            lanes.push_str(&format!("  wire [63:0] k_l{j} = ax_l{j} + {b};\n"));
            let g = if eng.predicated {
                format!("mask[{j}] ? k_l{j} : 64'sd0")
            } else {
                format!("k_l{j}")
            };
            lanes.push_str(&format!("  wire [63:0] g_l{j} = {g};\n"));
        } else {
            for (i, op) in eng.body.iter().enumerate() {
                lanes.push_str(&format!(
                    "  wire [63:0] v{i}_l{j} = {};\n",
                    op.op.verilog(
                        &src_reg(&eng.uniform, op.a, j),
                        &src_reg(&eng.uniform, op.b, j)
                    )
                ));
            }
            let kres = format!("v{}_l{j}", eng.result);
            let g = if eng.predicated {
                format!("mask[{j}] ? $signed({kres}) : 64'sd0")
            } else {
                format!("$signed({kres})")
            };
            lanes.push_str(&format!("  wire [63:0] g_l{j} = {g};\n"));
        }
    }
    let reduce_decl = if let Some(op) = eng.reduce {
        let terms: Vec<String> = (0..n).map(|j| format!("$signed(g_l{j})")).collect();
        format!("  wire [63:0] creduced = {};\n", terms.join(reduce_sep(op)))
    } else {
        String::new()
    };

    // ---- readback mux: raddr in 0..N selects a lane; raddr==N selects the reduced scalar ----
    let mut dout = String::new();
    for j in 0..n {
        dout.push_str(&format!("raddr == {j} ? outr{j} : "));
    }
    let dout_default = format!("outr{}", n - 1);
    dout.push_str(if eng.reduce.is_some() {
        "redr"
    } else {
        &dout_default
    });

    // ---- load FSM: capture din into the slot addressed by lc ----
    let mut load_case = String::new();
    for (idx, slot) in slots.iter().enumerate() {
        let lhs = match slot {
            Slot::Uniform(k) => format!("u{k} <= din;"),
            Slot::Mask => format!("mask <= din[{}:0];", n - 1),
            Slot::Varying(j, k) => format!("x{k}_l{j} <= din;"),
        };
        load_case.push_str(&format!("        {idx}: {lhs}\n"));
    }

    // ---- free-running stage-1 latches: the three 32×32 partial products ----
    let mut pipe_latch = String::new();
    if p > 0 {
        for j in 0..n {
            pipe_latch.push_str(&format!("      pll_l{j} <= al_l{j} * xl_l{j};\n"));
            pipe_latch.push_str(&format!("      phl_l{j} <= ah_l{j} * xl_l{j};\n"));
            pipe_latch.push_str(&format!("      plh_l{j} <= al_l{j} * xh_l{j};\n"));
        }
    }

    // ---- compute stage: latch every lane output + the reduced scalar together ----
    let mut compute = String::new();
    for j in 0..n {
        compute.push_str(&format!("        outr{j} <= g_l{j};\n"));
    }
    if eng.reduce.is_some() {
        compute.push_str("        redr <= creduced;\n");
    }

    // ---- sequential shift-add step: for each lane, add the shifted multiplicand when the current
    // multiplier bit is set, then shift `mc` up and `mp` down. Runs every cycle while `running` and
    // `scnt > 0` (it depends on `scnt`, so it lives INSIDE the FSM, not free-running). ----
    let mut seq_step = String::new();
    if is_seq {
        for j in 0..n {
            seq_step.push_str(&format!(
                "\x20\x20\x20\x20\x20\x20\x20\x20 acc_l{j} <= acc_l{j} + pp_l{j};\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20 mc_l{j}  <= mc_l{j} << 1;\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20 mp_l{j}  <= mp_l{j} >> 1;\n"
            ));
        }
    }
    // ---- sequential FMA load: zero the accumulator, seed the multiplicand (=a) and multiplier (=x). ----
    let mut seq_load = String::new();
    if is_seq {
        for j in 0..n {
            let a = src_reg(&eng.uniform, Src::In(0), j);
            let x = src_reg(&eng.uniform, Src::In(1), j);
            seq_load.push_str(&format!(
                "\x20\x20\x20\x20\x20\x20\x20 acc_l{j} <= 64'd0; mc_l{j} <= {a}; mp_l{j} <= {x};\n"
            ));
        }
    }

    // ---- go/run FSM: single-cycle (Combinational), drain `pipe` cycles (DecomposedFma), or run a
    // WIDTH-step shift-add (SequentialFma). ----
    let fsm = if is_seq {
        format!(
            "\x20\x20\x20\x20\x20 if (go) begin running <= 1'b1; scnt <= 8'd{WIDTH}; done <= 1'b0;\n\
             {seq_load}\
             \x20\x20\x20\x20\x20 end\n\
             \x20\x20\x20\x20\x20 else if (running) begin\n\
             \x20\x20\x20\x20\x20\x20\x20 if (scnt > 8'd0) begin\n\
             {seq_step}\
             \x20\x20\x20\x20\x20\x20\x20\x20 scnt <= scnt - 8'd1;\n\
             \x20\x20\x20\x20\x20\x20\x20 end else begin\n\
             {compute}\
             \x20\x20\x20\x20\x20\x20\x20\x20 done <= 1'b1; running <= 1'b0;\n\
             \x20\x20\x20\x20\x20\x20\x20 end\n\
             \x20\x20\x20\x20\x20 end\n"
        )
    } else if p > 0 {
        format!(
            "\x20\x20\x20\x20\x20 if (go) begin running <= 1'b1; pcnt <= 8'd{p}; done <= 1'b0; end\n\
             \x20\x20\x20\x20\x20 else if (running) begin\n\
             \x20\x20\x20\x20\x20\x20\x20 if (pcnt > 8'd1) pcnt <= pcnt - 8'd1;\n\
             \x20\x20\x20\x20\x20\x20\x20 else begin\n\
             {compute}\
             \x20\x20\x20\x20\x20\x20\x20\x20 done <= 1'b1; running <= 1'b0;\n\
             \x20\x20\x20\x20\x20\x20\x20 end\n\
             \x20\x20\x20\x20\x20 end\n"
        )
    } else {
        format!(
            "\x20\x20\x20\x20\x20 if (go) begin\n\
             {compute}\
             \x20\x20\x20\x20\x20\x20\x20 done <= 1'b1;\n\
             \x20\x20\x20\x20\x20 end\n"
        )
    };
    let rst_extra = if p > 0 || is_seq {
        " running <= 1'b0;"
    } else {
        ""
    };
    let pipe_note = if is_seq {
        format!(" The per-lane FMA is a SEQUENTIAL shift-add over {WIDTH} bits (one reused adder per lane, bounded fan-out → antenna-trivial); a result is ready {} cycles after `go` (done pulses).", WIDTH + 1)
    } else if p > 0 {
        format!(" The per-lane FMA is PIPELINED {p} stages (real GPU-ALU pipelining); a result is ready `pipe` cycles after `go` (done pulses).")
    } else {
        String::new()
    };

    let pred_note = if eng.predicated {
        ", gated by a per-lane mask (SIMT divergence)"
    } else {
        ""
    };
    let red_note = if eng.reduce.is_some() {
        ", then a cross-lane warp-reduce (raddr==N)"
    } else {
        ""
    };
    format!(
        "// gpu_{name} — a tuneable SIMT data-parallel engine: {n} spatial lanes, each running the\n\
         // SAME compiled straight-line kernel (the shader) on its own data{pred_note}{red_note}.\n\
         // The lanes are WIRED (spatial) — no warp scheduler, no instruction fetch/decode/opcode\n\
         // dispatch.{pipe_note} Narrow streaming I/O: `din` is loaded slot-by-slot into the lane\n\
         // register file, `raddr` reads results back. SIMD = all lanes unmasked; map-then-reduce\n\
         // feeds the NPU accumulator. 64-bit, wrapping like the VM's Int; proven ≡ the VM by our SAT.\n\
         module gpu_{name}(input clk, input rst, input load, input [63:0] din,\n\
         \x20                input go, input [7:0] raddr, output [63:0] dout, output reg done);\n\
         {decls}{lanes}{reduce_decl}  assign dout = {dout};\n\
         \x20 always @(posedge clk) begin\n\
         {pipe_latch}\
         \x20\x20\x20 if (rst) begin\n\
         \x20\x20\x20\x20\x20 lc <= 16'd0; done <= 1'b0;{rst_extra}\n\
         \x20\x20\x20 end else begin\n\
         \x20\x20\x20\x20\x20 if (load) begin\n\
         \x20\x20\x20\x20\x20\x20\x20 case (lc)\n\
         {load_case}\
         \x20\x20\x20\x20\x20\x20\x20\x20 default: ;\n\
         \x20\x20\x20\x20\x20\x20\x20 endcase\n\
         \x20\x20\x20\x20\x20\x20\x20 lc <= lc + 16'd1;\n\
         \x20\x20\x20\x20\x20 end\n\
         {fsm}\
         \x20\x20\x20 end\n\
         \x20 end\n\
         endmodule\n"
    )
}

/// Does the lane kernel compute `a·x + b` (inputs 0, 1, 2), wrapping, on the extremes and a spread of words?
pub(crate) fn computes_affine_fma(eng: &SimtEngine) -> bool {
    let words = [0i64, 1, -1, 2, i64::MAX, i64::MIN, 0x5555_5555_5555_5555, -0x1234_5678_9ABC_DEF1, 1 << 32, 3];
    eng.n_in == 3
        && words.iter().all(|&a| {
            words.iter().all(|&x| {
                let b = a.rotate_left(7) ^ x;
                eval_fabric(&eng.body, &[a, x, b], eng.result) == a.wrapping_mul(x).wrapping_add(b)
            })
        })
}

/// The streamed engine ([`LaneArith::Streamed`]): the same ports and load order as every SIMT engine, its lanes
/// sharing one pipelined multiply that each lane's `x` enters as it loads.
fn emit_gpu_simt_streamed_verilog(name: &str, eng: &SimtEngine, every: usize) -> String {
    let n = eng.n_lanes;
    assert!(every >= 1, "a streamed multiply registers after every `every`-th level, every >= 1");
    assert!(computes_affine_fma(eng), "the streamed multiply serves the a·x + b kernel");
    assert!(!eng.uniform[1], "the streamed multiply issues a lane's a·x when its x loads: x is per-lane");
    let (identity, fold) = match eng.reduce {
        None => ("64'd0", ""),
        Some(AluOp::Add) => ("64'd0", "redr + g"),
        Some(AluOp::Or) => ("64'd0", "redr | g"),
        Some(AluOp::Xor) => ("64'd0", "redr ^ g"),
        Some(AluOp::And) => ("{64{1'b1}}", "redr & g"),
        Some(op) => panic!("the streamed engine folds a warp-reduce of Add, Or, Xor or And, not {op:?}"),
    };
    let levels = mul_tree_levels(64);
    let tree = mul_tree("mt", "ma", "mx", 64, |l| (l + 1) % every == 0 || l + 1 == levels);
    let lat = tree.latency as usize;
    let tag = (usize::BITS - (n.max(2) - 1).leading_zeros()) as usize;
    let a_of = |j: usize| src_reg(&eng.uniform, Src::In(0), j);
    let b_of = |j: usize| src_reg(&eng.uniform, Src::In(2), j);

    let mut decls = String::new();
    for k in [0usize, 2] {
        if eng.uniform[k] {
            decls.push_str(&format!("  reg [63:0] u{k};\n"));
        } else {
            for j in 0..n {
                decls.push_str(&format!("  reg [63:0] x{k}_l{j};\n"));
            }
        }
    }
    if eng.predicated {
        decls.push_str(&format!("  reg [{}:0] mask;\n", n - 1));
    }
    decls.push_str(&format!("  reg [63:0] ma, mx; reg mv; reg [{}:0] ml;\n", tag - 1));
    for (r, bits) in &tree.regs {
        decls.push_str(&format!("  reg [{}:0] {r};\n", bits - 1));
    }
    decls.push_str(&format!("  reg pv [1:{lat}]; reg [{}:0] pl [1:{lat}];\n", tag - 1));
    for j in 0..n {
        decls.push_str(&format!("  reg [63:0] outr{j};\n"));
    }
    decls.push_str("  reg [63:0] g, redr; reg gv, waiting; reg [15:0] lc; reg [7:0] nd;\n");

    let lane = format!("pl[{lat}]");
    let b_lane = if eng.uniform[2] {
        "u2".to_string()
    } else {
        (0..n).rev().fold(String::from("64'd0"), |acc, j| format!("{lane} == {j} ? {} : {acc}", b_of(j)))
    };
    let mut wires = tree.wires.clone();
    wires.push_str(&format!("  wire [63:0] k_out = {} + ({b_lane});\n", tree.product));
    let kept = if eng.predicated { format!("mask[{lane}] ? k_out : 64'd0") } else { "k_out".to_string() };
    wires.push_str(&format!("  wire [63:0] g_out = {kept};\n"));

    let mut load_case = String::new();
    for (idx, slot) in load_slots(eng).iter().enumerate() {
        let lhs = match *slot {
            Slot::Uniform(k) => format!("u{k} <= din;"),
            Slot::Mask => format!("mask <= din[{}:0];", n - 1),
            Slot::Varying(j, 1) => format!("begin ma <= {}; mx <= din; mv <= 1'b1; ml <= {tag}'d{j}; end", a_of(j)),
            Slot::Varying(j, k) => format!("x{k}_l{j} <= din;"),
        };
        load_case.push_str(&format!("        {idx}: {lhs}\n"));
    }
    let moves: String = tree.moves.iter().map(|(r, e)| format!("    {r} <= {e};\n")).collect();
    let mut lanes_out = String::new();
    for j in 0..n {
        lanes_out.push_str(&format!("        if ({lane} == {tag}'d{j}) outr{j} <= g_out;\n"));
    }
    let shift: String = (2..=lat).map(|k| format!("    pv[{k}] <= pv[{}]; pl[{k}] <= pl[{}];\n", k - 1, k - 1)).collect();
    let mut dout = String::new();
    for j in 0..n {
        dout.push_str(&format!("raddr == {j} ? outr{j} : "));
    }
    dout.push_str(if eng.reduce.is_some() { "redr" } else { "64'd0" });
    let fold_line = if eng.reduce.is_some() { format!("      if (gv) redr <= {fold};\n") } else { String::new() };
    format!(
        "// gpu_{name} — a SIMT engine of {n} lanes sharing one pipelined multiply (an aligned tree of adds, {lat} \
         register stages):\n\
         // each lane's a·x enters it in the clock its x loads, and its masked a·x + b is folded into the warp-reduce \
         as it emerges.\n\
         // 64-bit, wrapping like the VM's Int.\n\
         module gpu_{name}(input clk, input rst, input load, input [63:0] din,\n\
         \x20                input go, input [7:0] raddr, output [63:0] dout, output reg done);\n\
         {decls}{wires}  assign dout = {dout};\n\
         \x20 always @(posedge clk) begin\n\
         {moves}\x20   pv[1] <= mv; pl[1] <= ml;\n\
         {shift}\x20   if (rst) begin\n\
         \x20     lc <= 16'd0; done <= 1'b0; waiting <= 1'b0; nd <= 8'd0; mv <= 1'b0; gv <= 1'b0; redr <= {identity};\n\
         \x20   end else begin\n\
         \x20     mv <= 1'b0;\n\
         \x20     if (load) begin\n\
         \x20       case (lc)\n\
         {load_case}\
         \x20       default: ;\n\
         \x20       endcase\n\
         \x20       lc <= lc + 16'd1;\n\
         \x20     end\n\
         \x20     gv <= pv[{lat}];\n\
         \x20     if (pv[{lat}]) begin\n\
         \x20       g <= g_out;\n\
         {lanes_out}\
         \x20     end\n\
         {fold_line}\
         \x20     if (gv) nd <= nd + 8'd1;\n\
         \x20     if (go) waiting <= 1'b1;\n\
         \x20     if ((go || waiting) && nd == 8'd{n}) begin done <= 1'b1; waiting <= 1'b0; end\n\
         \x20   end\n\
         \x20 end\n\
         endmodule\n"
    )
}

/// Evaluate the SIMT engine (the pure-Rust reference the silicon is checked against — N calls to
/// the already-proven `eval_fabric`, one per lane). `lane_inputs[j]` are lane j's `n_in` kernel
/// inputs (uniform inputs are set equal across lanes by the caller); `mask[j]` gates lane j when
/// predicated (masked → 0). Returns the per-lane outputs and the optional warp-reduced scalar.
pub fn eval_gpu_simt(
    eng: &SimtEngine,
    lane_inputs: &[Vec<i64>],
    mask: &[bool],
) -> (Vec<i64>, Option<i64>) {
    assert_eq!(lane_inputs.len(), eng.n_lanes, "one input vector per lane");
    if eng.predicated {
        assert_eq!(mask.len(), eng.n_lanes, "one mask bit per lane");
    }
    let mut outs = Vec::with_capacity(eng.n_lanes);
    for (j, li) in lane_inputs.iter().enumerate() {
        assert_eq!(li.len(), eng.n_in, "lane {j} needs {} inputs", eng.n_in);
        let k = eval_fabric(&eng.body, li, eng.result);
        let active = !eng.predicated || mask[j];
        outs.push(if active { k } else { 0 });
    }
    let reduced = eng.reduce.map(|op| {
        // masked lanes are 0 (Add's identity), so the reduce = the fold over active lanes.
        let seed = match op {
            AluOp::Add
            | AluOp::Sub
            | AluOp::Or
            | AluOp::Xor
            | AluOp::Shl
            | AluOp::Shr
            | AluOp::UShr => 0,
            AluOp::Mul => 1,
            AluOp::And => -1, // identity for bitwise AND (all ones); GPU reductions use Add/Sub/Mul in practice
        };
        outs.iter().copied().fold(seed, |acc, x| op.eval(acc, x))
    });
    (outs, reduced)
}

/// The BoundedExpr of ONE lane's predicated output: `mask ? kernel : 0` — the SIMT divergence
/// atom (a certified select over the compiled kernel). Uses the single-lane input names `in{k}`
/// (all lanes are byte-identical modulo the lane suffix, so one lane's proof covers the array).
pub fn simt_masked_lane_to_bounded(eng: &SimtEngine, width: u32) -> BoundedExpr {
    let kernel = fabric_to_bounded(&eng.body, eng.result, width);
    let cond = BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Eq,
        left: Box::new(BoundedExpr::BitVecVar("mask".to_string(), width)),
        right: Box::new(BoundedExpr::BitVecConst { width, value: 1 }),
    };
    BoundedExpr::BitVecIte {
        cond: Box::new(cond),
        then: Box::new(kernel),
        else_: Box::new(BoundedExpr::BitVecConst { width, value: 0 }),
    }
}

/// The BoundedExpr of the warp-reduce tail — a left fold of `op` over the lane-output variables
/// `out0..out{N-1}`. Lowers the reduce so our SAT can certify it equals an independent fold (that
/// every lane is summed, none dropped or duplicated).
pub fn simt_reduce_to_bounded(eng: &SimtEngine, op: AluOp, width: u32) -> BoundedExpr {
    let bop = match op {
        AluOp::Add => BitVecBoundedOp::Add,
        AluOp::Sub => BitVecBoundedOp::Sub,
        AluOp::Mul => BitVecBoundedOp::Mul,
        AluOp::And => BitVecBoundedOp::And,
        AluOp::Or => BitVecBoundedOp::Or,
        AluOp::Xor => BitVecBoundedOp::Xor,
        AluOp::Shl => BitVecBoundedOp::Shl,
        AluOp::Shr => BitVecBoundedOp::AShr,
        AluOp::UShr => BitVecBoundedOp::Shr,
    };
    let mut acc = BoundedExpr::BitVecVar("out0".to_string(), width);
    for j in 1..eng.n_lanes {
        acc = BoundedExpr::BitVecBinary {
            op: bop,
            left: Box::new(acc),
            right: Box::new(BoundedExpr::BitVecVar(format!("out{j}"), width)),
        };
    }
    acc
}

/// The BoundedExpr of the SEQUENTIAL shift-add multiply `a·x`, UNROLLED over `width` bits, for
/// single-lane inputs `a = in0`, `x = in1`. This is the exact combinational image of the `SequentialFma`
/// FSM's `width`-step recurrence: `acc += (x[i] ? a<<i : 0)` for i in 0..width. Our own certified SAT
/// proves this shift-add equals the hardware `a*x` for ALL inputs — so the strategy switch is FREE
/// (no divergence). `b` and the predicate are the same tail the other strategies share, already
/// certified by `gpu_kernel_is_certified_equal_to_the_vm_element_op` / `gpu_predication_is_certified`.
pub fn simt_sequential_fma_to_bounded(width: u32) -> BoundedExpr {
    let a = BoundedExpr::BitVecVar("in0".to_string(), width);
    let x = BoundedExpr::BitVecVar("in1".to_string(), width);
    let mut acc = BoundedExpr::BitVecConst { width, value: 0 };
    for i in 0..width {
        // The i-th multiplier bit `x[i]` (a 1-bit extract).
        let xi = BoundedExpr::BitVecExtract {
            high: i,
            low: i,
            operand: Box::new(x.clone()),
        };
        let cond = BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::Eq,
            left: Box::new(xi),
            right: Box::new(BoundedExpr::BitVecConst { width: 1, value: 1 }),
        };
        // `a << i` — the multiplicand shifted to the i-th place (the FSM's left-shifting `mc`).
        let shifted = BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::Shl,
            left: Box::new(a.clone()),
            right: Box::new(BoundedExpr::BitVecConst {
                width,
                value: i as u64,
            }),
        };
        let term = BoundedExpr::BitVecIte {
            cond: Box::new(cond),
            then: Box::new(shifted),
            else_: Box::new(BoundedExpr::BitVecConst { width, value: 0 }),
        };
        acc = BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::Add,
            left: Box::new(acc),
            right: Box::new(term),
        };
    }
    acc
}

/// The BoundedExpr of ONE emitted sliced partial product `pp` — the fan-out-bounded image of the
/// single-select mux `mp[0] ? mc : 0`. It models the emitted Verilog exactly:
/// `pp = { {G{s}} & mc[hi:lo] , ... }` over `select_slice` groups of `G = width / select_slice`
/// bits, where every `msel` copy equals the ONE select bit `s = mp0`. Group `g` occupies bits
/// `[g*G .. g*G+G-1]`, and its value is `s==1 ? mc[hi:lo] : 0` (a `G`-wide select). Building `pp`
/// from these per-group selects and proving `pp == (s==1 ? mc : 0)` CERTIFIES the byte-slicing drops
/// or misaligns no bit — closing the gap where the slice was previously only iverilog-checked.
///
/// `s = BitVecVar("mp0", 1)`, `mc = BitVecVar("mc", width)`. `width % select_slice` must be 0 (the
/// same tiling invariant the emitter asserts). Groups are concatenated high-to-low so group 0 lands
/// at the LSB, matching `{ group[N-1], ..., group[1], group[0] }`.
pub fn simt_sliced_pp_to_bounded(select_slice: usize, width: u32) -> BoundedExpr {
    assert!(select_slice > 0, "select_slice must be positive");
    assert_eq!(
        width as usize % select_slice,
        0,
        "select_slice ({select_slice}) must divide width ({width}) evenly"
    );
    let group = width / select_slice as u32;
    let s = BoundedExpr::BitVecVar("mp0".to_string(), 1);
    let mc = BoundedExpr::BitVecVar("mc".to_string(), width);
    // `s == 1` — every `(* keep *)` copy `mselK` is this ONE bit, so one condition covers them all.
    let cond = BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Eq,
        left: Box::new(s),
        right: Box::new(BoundedExpr::BitVecConst { width: 1, value: 1 }),
    };
    // Build each group's G-wide select `s==1 ? mc[hi:lo] : 0`, then concat groups high→low.
    let group_pp = |g: u32| -> BoundedExpr {
        let hi = (g + 1) * group - 1;
        let lo = g * group;
        BoundedExpr::BitVecIte {
            cond: Box::new(cond.clone()),
            then: Box::new(BoundedExpr::BitVecExtract {
                high: hi,
                low: lo,
                operand: Box::new(mc.clone()),
            }),
            else_: Box::new(BoundedExpr::BitVecConst {
                width: group,
                value: 0,
            }),
        }
    };
    // Start from the highest group; fold the rest below it (BitVecConcat(high, low)).
    let top = select_slice as u32 - 1;
    let mut pp = group_pp(top);
    for g in (0..top).rev() {
        pp = BoundedExpr::BitVecConcat(Box::new(pp), Box::new(group_pp(g)));
    }
    pp
}

/// A SOFT signoff advisory (NEVER a lock, NEVER blocks emission): the wide combinational strategies
/// (`Combinational`, `DecomposedFma`) at datapath widths >= 32 are met1-antenna-dense on Sky130 —
/// even after fan-out splitting and low utilization, a handful of met1 area-ratio violations remain
/// (empirically ~10 on the N=4 flagship). `SequentialFma` (one reused adder per lane, bounded
/// fan-out) is antenna-trivial. Returns `Some(warning)` for the dense envelope, `None` otherwise.
pub fn signoff_advisory(arith: &LaneArith, width: u32, n_lanes: usize) -> Option<String> {
    let dense = matches!(
        arith,
        LaneArith::Combinational | LaneArith::DecomposedFma { .. }
    );
    if dense && width >= 32 {
        Some(format!(
            "signoff advisory: {arith:?} at width {width} across {n_lanes} lane(s) is a WIDE \
             combinational multiply — met1-antenna-dense on Sky130 (expect met1 area-ratio \
             violations even at low utilization). Consider LaneArith::SequentialFma (one reused \
             adder per lane, bounded fan-out) for an antenna-trivial signoff. This is advisory \
             only — emission is unaffected."
        ))
    } else {
        None
    }
}

/// The value a load slot carries for a given `(lane_inputs, mask)` vector.
fn slot_value(slot: Slot, lane_inputs: &[Vec<i64>], mask: &[bool]) -> u64 {
    match slot {
        Slot::Uniform(k) => lane_inputs[0][k] as u64,
        Slot::Mask => {
            let mut m: u64 = 0;
            for (j, &b) in mask.iter().enumerate() {
                if b {
                    m |= 1 << j;
                }
            }
            m
        }
        Slot::Varying(j, k) => lane_inputs[j][k] as u64,
    }
}

/// Build the golden input/output hex, computing the outputs with `eval_gpu_simt` — the reference
/// the silicon lock separately certifies bit-identical to the VM (so these goldens ARE the VM
/// oracle). The input hex is one word per load slot per vector (uniforms, mask, then per-lane
/// varyings — the `load_slots` order); the output hex is each lane output then the reduced scalar.
/// `vectors[t]` provides `(lane_inputs, mask)` for vector t. Returns `(in_hex, out_hex)`.
pub fn write_gpu_hex(eng: &SimtEngine, vectors: &[(Vec<Vec<i64>>, Vec<bool>)]) -> (String, String) {
    let slots = load_slots(eng);
    let mut hin = String::new();
    let mut hout = String::new();
    for (lane_inputs, mask) in vectors {
        for &slot in &slots {
            hin.push_str(&format!("{:016x}\n", slot_value(slot, lane_inputs, mask)));
        }
        let (outs, reduced) = eval_gpu_simt(eng, lane_inputs, mask);
        for o in &outs {
            hout.push_str(&format!("{:016x}\n", *o as u64));
        }
        if let Some(r) = reduced {
            hout.push_str(&format!("{:016x}\n", r as u64));
        }
    }
    (hin, hout)
}

/// The per-vector count of input hex words (one per load slot).
fn inputs_per_vector(eng: &SimtEngine) -> usize {
    load_slots(eng).len()
}

/// The per-vector count of output hex words: each lane output, then the reduced scalar (if any).
fn outputs_per_vector(eng: &SimtEngine) -> usize {
    eng.n_lanes + if eng.reduce.is_some() { 1 } else { 0 }
}

/// A self-checking iverilog testbench: for each of `n_vectors` vectors, reset the load counter,
/// stream the load slots from `gpu_{name}_in.hex`, pulse `go` (all lanes compute in one cycle),
/// then read back every lane output + the reduced scalar via `raddr` and compare against the VM
/// oracle in `gpu_{name}_out.hex`. Prints PASS/FAIL. Layout matches `write_gpu_hex`.
pub fn emit_gpu_simt_testbench(name: &str, eng: &SimtEngine, n_vectors: usize) -> String {
    let ipv = inputs_per_vector(eng);
    let opv = outputs_per_vector(eng);
    let in_last = n_vectors * ipv - 1;
    let out_last = n_vectors * opv - 1;
    // Read back the reduced scalar too (at raddr == N) when a reduce is present.
    let n_reads = opv; // == n (+1 if reduce)
                       // Cycles to wait after `go` before the result is latched: Combinational = 0 (single-cycle),
                       // DecomposedFma{s} = s (pipeline drain), SequentialFma = WIDTH+1 = 65 (the shift-add run + latch).
    let wait_cycles = match &eng.arith {
        LaneArith::Combinational => 0,
        LaneArith::DecomposedFma { stages } => *stages,
        LaneArith::SequentialFma { .. } => 65,
        LaneArith::Streamed { .. } => 0,
    };
    let pipe_wait = if matches!(eng.arith, LaneArith::Streamed { .. }) {
        "while (!done) begin @(posedge clk); #1; end ".to_string()
    } else if wait_cycles > 0 {
        format!("repeat ({wait_cycles}) @(posedge clk); ")
    } else {
        String::new()
    };
    format!(
        "// tb_gpu_{name} — the SIMT engine's N-lane map (+ warp-reduce) vs the RUNNING VM's output.\n\
         `timescale 1ns/1ps\n\
         module tb_gpu_{name};\n\
         \x20 reg clk = 0; reg rst, load, go; reg [63:0] din; reg [7:0] raddr;\n\
         \x20 wire [63:0] dout; wire done;\n\
         \x20 reg  [63:0] oin  [0:{in_last}];\n\
         \x20 reg  [63:0] oout [0:{out_last}];\n\
         \x20 integer i, s, r, errors = 0;\n\
         \x20 gpu_{name} dut(.clk(clk), .rst(rst), .load(load), .din(din), .go(go),\n\
         \x20                .raddr(raddr), .dout(dout), .done(done));\n\
         \x20 always #5 clk = ~clk;\n\
         \x20 initial begin\n\
         \x20\x20\x20 $readmemh(\"gpu_{name}_in.hex\", oin);\n\
         \x20\x20\x20 $readmemh(\"gpu_{name}_out.hex\", oout);\n\
         \x20\x20\x20 rst = 1; load = 0; go = 0; din = 0; raddr = 0;\n\
         \x20\x20\x20 for (i = 0; i < {n_vectors}; i = i + 1) begin\n\
         \x20\x20\x20\x20\x20 rst = 1; @(posedge clk); #1 rst = 0;\n\
         \x20\x20\x20\x20\x20 for (s = 0; s < {ipv}; s = s + 1) begin\n\
         \x20\x20\x20\x20\x20\x20\x20 din = oin[i*{ipv}+s]; load = 1; @(posedge clk); #1;\n\
         \x20\x20\x20\x20\x20 end\n\
         \x20\x20\x20\x20\x20 load = 0; go = 1; @(posedge clk); #1 go = 0; {pipe_wait}\n\
         \x20\x20\x20\x20\x20 for (r = 0; r < {n_reads}; r = r + 1) begin\n\
         \x20\x20\x20\x20\x20\x20\x20 raddr = r; #1;\n\
         \x20\x20\x20\x20\x20\x20\x20 if (dout !== oout[i*{opv}+r]) begin errors = errors + 1; $display(\"FAIL vec %0d read %0d\", i, r); end\n\
         \x20\x20\x20\x20\x20 end\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 if (errors == 0) $display(\"PASS: {n_vectors} SIMT evaluations match the VM (N-lane map, no warp scheduler)\");\n\
         \x20\x20\x20 else $display(\"FAIL: %0d SIMT mismatches\", errors);\n\
         \x20\x20\x20 $finish;\n\
         \x20 end\n\
         endmodule\n"
    )
}
