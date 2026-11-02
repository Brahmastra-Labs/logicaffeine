//! The vector unit as Verilog: sixteen f32 lanes running the instruction set of
//! [`crate::spatial::decode_engine::vector`] over vector and scalar registers, one sixteen-element row a cycle.
//!
//! A vector register holds up to `ROWS` rows of sixteen lanes and its length. An instruction over a vector walks its
//! rows: elementwise instructions write lane by lane, `Dot` and `Sum` accumulate each lane over the rows and fold the
//! lanes by the pairwise tree once the rows are done, `Max` and `AbsMax` keep each lane's largest and fold the lanes
//! the same way, a largest zero `+0`, and `ArgMax` keeps each lane's largest with the first index holding it and folds
//! the lanes to the larger, the lower index of equals. `Hadamard` runs its stages in place: the strides below sixteen
//! within each row, the strides of sixteen and up between rows, a pair of rows read at once and written in two cycles.
//! `Rotate` copies its operand, then turns sixteen pairs from one read of the rows they sit in, and writes those rows
//! one a cycle under lane masks. Scalar instructions take a cycle. Lanes past a register's length stay out of every
//! reduction and every write. The functions run on `FU` function units, a divisor of sixteen, each in
//! [`LANE_FUNC_STAGES`] stages a cycle apart: a `Map`'s rows go in a slice of `FU` lanes a cycle, `16 / FU` cycles a row
//! and one row after another, each row written when its last slice's values are out, and a `ScalarMap` takes the first
//! unit's stages.
//!
//! The registers live in block RAM (`rtl::ram`), a bank a register with three copies written alike: copy A read at
//! the row at hand, or at the engine's row while the unit is idle, copies B and C at a Hadamard partner row and a
//! rotary block's two rows, and one write a bank a cycle.
//!
//! `logos_simd_p` is the same unit for the user clock ([`simd_p_verilog`]): every register and value the same, on the
//! pipelined f32 units and function units (`logos_lane_func_p`, [`LANE_FUNC_P_LATENCY`]), its registers block RAM read
//! on the rising edge. An instruction's rows stream through its units a row a clock and are written as they come out;
//! `Dot` and `Sum` add a row each add's latency, each lane's running sum through one pipelined add, and fold the lanes by
//! the pipelined tree; the maxima take a row a clock and fold in four registered levels; a Hadamard stage streams its
//! rows, one stage after another; a rotary block is read, turned and written before the next.
//!
//! An instruction is 64 bits: `[3:0]` the opcode, `[7:4]` the operation or function (`Rotate`'s NEOX flag), `[11:8]`
//! `a`, `[15:12]` `b` (`Rotate`'s cosines), `[19:16]` `d`, `[23:20]` the scalar operand (`Rotate`'s sines, a
//! multiply-add's vector addend), `[63:32]` a constant ([`encode`]). A multiply-add's product goes into its sum as the
//! multiply's result goes into an add after it.

use crate::spatial::decode_engine::vector::{Bin, Func, Op};

/// The instruction word for `op`.
pub fn encode(op: &Op) -> u64 {
    let bin = |b: Bin| match b {
        Bin::Mul => 0u64,
        Bin::Add => 1,
        Bin::Sub => 2,
    };
    let func = |f: Func| match f {
        Func::Exp => 0u64,
        Func::Recip => 1,
        Func::Rsqrt => 2,
        Func::Sigmoid => 3,
        Func::Silu => 4,
        Func::Softplus => 5,
        Func::Ln1p => 6,
        Func::Neg => 7,
        Func::F16 => 8,
    };
    let word = |opcode: u64, sub: u64, a: u8, b: u8, d: u8, s: u8, k: u32| {
        opcode | sub << 4 | (a as u64) << 8 | (b as u64) << 12 | (d as u64) << 16 | (s as u64) << 20 | (k as u64) << 32
    };
    match *op {
        Op::Dot { a, b, d } => word(0, 0, a, b, d, 0, 0),
        Op::Sum { a, d } => word(1, 0, a, 0, d, 0, 0),
        Op::Max { a, d } => word(2, 0, a, 0, d, 0, 0),
        Op::Vec { op, a, b, d } => word(3, bin(op), a, b, d, 0, 0),
        Op::VecScalar { op, a, s, d } => word(4, bin(op), a, 0, d, s, 0),
        Op::Map { f, a, d } => word(5, func(f), a, 0, d, 0, 0),
        Op::Scalar { op, a, b, d } => word(6, bin(op), a, b, d, 0, 0),
        Op::ScalarMap { f, a, d } => word(7, func(f), a, 0, d, 0, 0),
        Op::Const { value, d } => word(8, 0, 0, 0, d, 0, value.to_bits()),
        Op::Hadamard { a, d } => word(9, 0, a, 0, d, 0, 0),
        Op::Rotate { a, cos, sin, d, neox } => word(10, neox as u64, a, cos, d, sin, 0),
        Op::ArgMax { a, d } => word(11, 0, a, 0, d, 0, 0),
        Op::AbsMax { a, d } => word(12, 0, a, 0, d, 0, 0),
        Op::Codes { a, s, d } => word(13, 0, a, 0, d, s, 0),
        Op::Scale { a, n, d } => word(14, 0, a, 0, d, 0, n as i32 as u32),
        Op::MulAdd { a, b, c, d } => word(15, 0, a, b, d, c, 0),
        Op::MulAddScalar { a, b, s, d } => word(15, 1, a, b, d, s, 0),
    }
}

/// The instruction an instruction word stands for, or `None` for a word no instruction encodes to.
pub fn decode(word: u64) -> Option<Op> {
    let field = |at: u32| (word >> at & 0xf) as u8;
    let (opcode, sub, a, b, d, s) = (field(0), field(4), field(8), field(12), field(16), field(20));
    let k = (word >> 32) as u32;
    let bin = match sub {
        0 => Some(Bin::Mul),
        1 => Some(Bin::Add),
        2 => Some(Bin::Sub),
        _ => None,
    };
    let func = [Func::Exp, Func::Recip, Func::Rsqrt, Func::Sigmoid, Func::Silu, Func::Softplus, Func::Ln1p, Func::Neg, Func::F16].get(sub as usize).copied();
    let op = match opcode {
        0 => Op::Dot { a, b, d },
        1 => Op::Sum { a, d },
        2 => Op::Max { a, d },
        3 => Op::Vec { op: bin?, a, b, d },
        4 => Op::VecScalar { op: bin?, a, s, d },
        5 => Op::Map { f: func?, a, d },
        6 => Op::Scalar { op: bin?, a, b, d },
        7 => Op::ScalarMap { f: func?, a, d },
        8 => Op::Const { value: f32::from_bits(k), d },
        9 => Op::Hadamard { a, d },
        10 if sub <= 1 => Op::Rotate { a, cos: b, sin: s, d, neox: sub == 1 },
        11 => Op::ArgMax { a, d },
        12 => Op::AbsMax { a, d },
        13 => Op::Codes { a, s, d },
        14 => Op::Scale { a, n: k as i32 as i8, d },
        15 if sub == 0 => Op::MulAdd { a, b, c: s, d },
        15 if sub == 1 => Op::MulAddScalar { a, b, s, d },
        _ => return None,
    };
    (encode(&op) == word).then_some(op)
}

/// The vector unit, with the f32 units, the functions, the tree and the block RAM it is built from.
pub fn simd_verilog() -> String {
    format!("{}\n{}\n{}", super::ram::ram_verilog(&[512]), super::vector::vector_verilog(), SIMD_VERILOG)
}

/// The vector unit in the user clock (`logos_simd_p`), with its pipelined function units (`logos_lane_func_p`), the
/// pipelined f32 units, the functions and the block RAM it is built from, beside the vector unit it is held to
/// ([`simd_verilog`]).
pub fn simd_p_verilog() -> String {
    use super::fp32::{F32_TO_F16_LATENCY, FADD_LATENCY, FMUL_LATENCY, TREE16_LATENCY};
    use super::vector::{EXP_LATENCY, I2F_LATENCY, LN_LATENCY, RECIP_LATENCY, RNE_P_LATENCY, RSQRT_LATENCY};
    assert!(LN_LATENCY >= RECIP_LATENCY.max(RSQRT_LATENCY).max(FADD_LATENCY), "the logarithm is the slowest function core");
    // An elementwise row's write: its read, its operands registered, then its code's multiply (its operands chosen and
    // registered first), its nearest integer, its code registered and its f32 — the longest of its values, a sum and a
    // product held to it.
    assert!(FMUL_LATENCY + 1 + RNE_P_LATENCY + 1 + I2F_LATENCY > FADD_LATENCY + 1, "an elementwise row's code is its last value");
    let ew = 4 + FMUL_LATENCY + RNE_P_LATENCY + I2F_LATENCY;
    assert!(FMUL_LATENCY + 1 + FADD_LATENCY + 1 <= ew - 2, "a multiply-add's sum is out by an elementwise row's write");
    // A maximum's fold: four levels of pairs, then the arg-max's index as an f32, then the write.
    let fold_write = 5 + I2F_LATENCY;
    // A scalar product is held to the scalar sum's latency, so a pass a clock writes each pass's value in turn.
    assert!(FADD_LATENCY >= FMUL_LATENCY, "a scalar operation's product waits for its sum's latency");
    assert!(fold_write < 8, "a fold's write is a step of its three-bit level");
    let z = LANE_FUNC_P_LATENCY - 1;
    let lane = [
        ("@RECIP_PAD@", LN_LATENCY - RECIP_LATENCY),
        ("@RSQRT_PAD@", LN_LATENCY - RSQRT_LATENCY),
        ("@UM1_PAD@", LN_LATENCY - FADD_LATENCY),
        ("@TO_Z@", FMUL_LATENCY + FADD_LATENCY),
        ("@YH@", z - 2 - F32_TO_F16_LATENCY),
        ("@EXP@", EXP_LATENCY),
        ("@LN@", LN_LATENCY),
        ("@LANE@", LANE_FUNC_P_LATENCY),
        ("@FMUL@", FMUL_LATENCY),
        ("@FADD@", FADD_LATENCY),
    ]
    .iter()
    .fold(LANE_FUNC_P_VERILOG.to_string(), |t, (k, v)| t.replace(k, &v.to_string()));
    let unit = [
        ("@FMUL@", FMUL_LATENCY),
        ("@FADD@", FADD_LATENCY),
        ("@LANE@", LANE_FUNC_P_LATENCY),
        ("@TREE@", TREE16_LATENCY),
        ("@EW@", ew),
        ("@RNE@", RNE_P_LATENCY),
        ("@SALIGN@", FADD_LATENCY - FMUL_LATENCY),
        ("@ADD@", FADD_LATENCY + 1),
        ("@MAP@", 2 + LANE_FUNC_P_LATENCY),
        ("@FOLD_WRITE@", fold_write),
        ("@READ@", SIMD_P_READ_LATENCY),
    ]
    .iter()
    .fold(SIMD_P_VERILOG.to_string(), |t, (k, v)| t.replace(k, &v.to_string()));
    format!("{}\n{}\n{lane}\n{unit}", super::ram::bram_verilog(&[512]), simd_verilog())
}

/// Clocks the user clock's vector unit (`logos_simd_p`) takes from a row asked (`rd_row`, `rd_reg`) to its word on
/// `rd_data`: the row registered, then read.
pub const SIMD_P_READ_LATENCY: usize = 2;

/// Clocks a pipelined function unit's value (`logos_lane_func_p`) takes from its operand: the operand decoded and
/// registered; the exponential; the add after it; the reciprocal, the logarithm, the reciprocal square root and one
/// less than the sum, each held to the slowest; ln1p's error, its product and its sum; the choice registered.
pub const LANE_FUNC_P_LATENCY: usize = {
    use super::fp32::{FADD_LATENCY, FMUL_LATENCY};
    use super::vector::{EXP_LATENCY, LN_LATENCY};
    1 + EXP_LATENCY + FADD_LATENCY + LN_LATENCY + FADD_LATENCY + FMUL_LATENCY + FADD_LATENCY + 1
};

/// Cycles a function unit's value takes from its operand (`logos_lane_func` with `STAGED`): the exponential; the add
/// after it; the reciprocal, the logarithm, the reciprocal square root and one less than the sum; ln1p's correction
/// and the choice. No stage is deeper than one of the functions' cores, so the vector unit's longest path is a core's,
/// not the chain of them.
pub const LANE_FUNC_STAGES: usize = 4;

const LANE_FUNC_P_VERILOG: &str = r#"
// One lane's function of one element on the pipelined units, an operand a clock: y is logos_lane_func's value of the
// function and operand @LANE@ clocks before, its cores the pipelined ones in the same order — the exponential; the add
// after it; the reciprocal, the logarithm, the reciprocal square root and one less than the sum, each value held to the
// slowest's; ln1p's correction; the choice — what each stage needs of the operand held beside it. A core no chosen
// function uses holds its operand at zero.
module logos_lane_func_p(input clk, input [3:0] f, input [31:0] x, output reg [31:0] y);
  // The operand decoded and registered: the exponential's operand, the binary16 round trip's, the flags.
  wire [31:0] xfc = `LOGOS_FTZ(x);
  wire spa;
  logos_flt top(.a(`LOGOS_SOFTPLUS_TOP), .b(xfc), .y(spa));
  wire is_exp = f == 4'd0, is_recip = f == 4'd1, is_rsqrt = f == 4'd2, is_sig = f == 4'd3, is_silu = f == 4'd4;
  wire is_sp = f == 4'd5, is_l1p = f == 4'd6, is_half = f == 4'd8;
  wire gate_c = is_sig || is_silu, past_c = is_sp && spa, near_c = is_sp && !spa;
  reg [3:0] f1;
  reg [31:0] x1, xf1, eo1, hx1;
  reg [7:0] flags1;
  always @(posedge clk) begin
    f1 <= f; x1 <= x; xf1 <= xfc;
    eo1 <= is_exp ? x : (gate_c || past_c) ? {~xfc[31], xfc[30:0]} : near_c ? xfc : 32'd0;
    hx1 <= is_half ? x : 32'd0;
    flags1 <= {spa, past_c, gate_c, is_l1p || near_c, is_l1p, is_recip, is_rsqrt, is_silu};
  end
  // The binary16 round trip, the rounding on logos_f32_to_f16_p and the widening registered, held to the choice.
  wire [15:0] half;
  wire [31:0] yh_c, yh;
  logos_f32_to_f16_p uh(.clk(clk), .x(hx1), .h(half));
  logos_f16_to_f32 uw(.h(half), .x(yh_c));
  reg [31:0] yh_r;
  always @(posedge clk) yh_r <= yh_c;
  logos_delay #(.W(32), .D(@YH@)) dyh(.clk(clk), .d(yh_r), .q(yh));
  // The exponential.
  wire [31:0] e;
  logos_exp_p ue(.clk(clk), .x_in(eo1), .y(e));
  wire [3:0] f_e;
  wire [31:0] x_e, xf_e;
  wire [7:0] fl_e;
  logos_delay #(.W(76), .D(@EXP@)) d1(.clk(clk), .d({f1, x1, xf1, flags1}), .q({f_e, x_e, xf_e, fl_e}));
  // One plus the exponential for the gates, u plus it past softplus's top; one plus ln1p's operand, softplus's
  // exponential below it.
  wire past_e = fl_e[6], gate_e = fl_e[5], on_e = fl_e[4], l1p_e = fl_e[3];
  wire [31:0] t_e = l1p_e ? xf_e : `LOGOS_FTZ(e);
  wire [31:0] s;
  logos_fadd_p ua(.clk(clk), .a(past_e ? xf_e : 32'h3f80_0000), .b((gate_e || past_e) ? e : on_e ? t_e : 32'd0), .y(s));
  wire [3:0] f_s;
  wire [31:0] x_s, e_s, t_s;
  wire [7:0] fl_s;
  logos_delay #(.W(108), .D(@FADD@)) d2(.clk(clk), .d({f_e, x_e, e, t_e, fl_e}), .q({f_s, x_s, e_s, t_s, fl_s}));
  // The reciprocal, the logarithm, the reciprocal square root, one less than the sum; each held to the logarithm.
  wire gate_s = fl_s[5], on_s = fl_s[4], recip_s = fl_s[2], rsqrt_s = fl_s[1];
  wire [31:0] r_c, lu, um1_c, q_c, r, um1, q;
  logos_recip_p ur(.clk(clk), .x_in(recip_s ? x_s : (gate_s || on_s) ? s : 32'd0), .y(r_c));
  logos_ln_p ul(.clk(clk), .x_in(on_s ? s : 32'd0), .y(lu));
  logos_fadd_p uo(.clk(clk), .a(on_s ? s : 32'd0), .b(32'hbf80_0000), .y(um1_c));
  logos_rsqrt_p uq(.clk(clk), .x_in(rsqrt_s ? x_s : 32'd0), .y(q_c));
  logos_delay #(.W(32), .D(@RECIP_PAD@)) dr(.clk(clk), .d(r_c), .q(r));
  logos_delay #(.W(32), .D(@UM1_PAD@)) du(.clk(clk), .d(um1_c), .q(um1));
  logos_delay #(.W(32), .D(@RSQRT_PAD@)) dq(.clk(clk), .d(q_c), .q(q));
  wire [3:0] f_r;
  wire [31:0] x_r, e_r, t_r, s_r;
  wire [7:0] fl_r;
  logos_delay #(.W(140), .D(@LN@)) d3(.clk(clk), .d({f_s, x_s, e_s, t_s, s, fl_s}), .q({f_r, x_r, e_r, t_r, s_r, fl_r}));
  // silu's product with its sigmoid, or ln1p's correction: the error, then its product with the reciprocal, then the sum
  // with the logarithm.
  wire on_r = fl_r[4];
  wire [31:0] err;
  logos_fadd_p ud(.clk(clk), .a(on_r ? t_r : 32'd0), .b({~um1[31], um1[30:0]}), .y(err));
  wire [3:0] f_m;
  wire [31:0] x_m, e_m, t_m, s_m, r_m, q_m, lu_m;
  wire [7:0] fl_m;
  logos_delay #(.W(236), .D(@FADD@)) d4(.clk(clk), .d({f_r, x_r, e_r, t_r, s_r, r, q, lu, fl_r}),
    .q({f_m, x_m, e_m, t_m, s_m, r_m, q_m, lu_m, fl_m}));
  wire [31:0] p, z, p_z;
  logos_fmul_p um(.clk(clk), .a(fl_m[0] ? x_m : fl_m[4] ? err : 32'd0), .b(r_m), .y(p));
  wire [31:0] lu_p;
  logos_delay #(.W(32), .D(@FMUL@)) dlu(.clk(clk), .d(lu_m), .q(lu_p));
  logos_fadd_p uz(.clk(clk), .a(lu_p), .b(p), .y(z));
  logos_delay #(.W(32), .D(@FADD@)) dp(.clk(clk), .d(p), .q(p_z));
  wire [3:0] f_z;
  wire [31:0] x_z, e_z, t_z, s_z, r_z, q_z;
  wire [7:0] fl_z;
  logos_delay #(.W(204), .D(@TO_Z@)) d5(.clk(clk), .d({f_m, x_m, e_m, t_m, s_m, r_m, q_m, fl_m}),
    .q({f_z, x_z, e_z, t_z, s_z, r_z, q_z, fl_z}));
  // The function chosen.
  wire [31:0] l1p = (s_z == 32'h3f80_0000) ? t_z : z;
  always @(posedge clk) case (f_z)
    4'd0: y <= e_z; 4'd1: y <= r_z; 4'd2: y <= q_z; 4'd3: y <= r_z; 4'd4: y <= p_z; 4'd5: y <= fl_z[7] ? s_z : l1p;
    4'd6: y <= l1p; 4'd8: y <= yh;
    default: y <= {~x_z[31], x_z[30:0]};
  endcase
endmodule
"#;

const SIMD_P_VERILOG: &str = r#"// The vector unit in the user clock: logos_simd's instruction set over the same registers, every value the same, on the
// pipelined units. Each instruction's rows stream through its units a row a clock, written as they come out: a row read
// at a clock, its operands registered the clock after the read, its result written @EW@ clocks after the read for an
// elementwise instruction, its function values @MAP@ clocks after for a Map. Dot and Sum add a row each @ADD@ clocks,
// each lane's running sum through one pipelined add, then fold the lanes by the pipelined tree; Max, AbsMax and ArgMax
// take a row a clock and fold the lanes in four registered levels. A Hadamard stage streams its rows, a pair of rows
// two writes, the stages one after another; a rotary block's three rows are read, turned and written before the next
// block's. A ScalarMap runs on the first function unit, a scalar operation on its own multiply and add. Lanes past a
// register's length stay out of every reduction; an instruction over an empty register walks its first row, as
// logos_simd's does. The registers are block RAM read on the rising edge (logos_bram_w512), three copies a bank:
// copy A at the row at hand, or at the engine's row while the unit is idle; copies B and C at a Hadamard partner row and
// a rotary block's two rows, which are rows of the instruction's operand, so they hold a register's first PAIRS rows,
// every program's Hadamard and rotary operands within them. A row asked is registered (copied near the memories it
// reaches) and read the clock after, its word out the clock after that: @READ@ clocks from the ask, the engine's reads
// too. The one write a clock is registered alike, its banks, row, data and mask a clock after its choice, so the
// memories see every read and write a clock later than they are asked and each read sees the writes it would unregistered.
//
// An instruction runs over a group of passes, `group` of them (one when zero, at most PASSES) taken at its issue: pass
// p's rows of every register from p·group_rows, its scalar registers its own (rd_s_pass reads them). Elementwise
// instructions and Maps walk the passes' rows one pass after another; Dot and Sum take row r of each pass of a subgroup
// of ADD passes in turn, ADD clocks a round, so each pass's running sum takes its next row the clock its last add is out
// and a subgroup of ADD passes adds a row a clock, each subgroup's rows to their end before the next subgroup's; Max, AbsMax and ArgMax fold one pass before the next; a scalar operation or a ScalarMap
// takes a pass a clock and writes each pass's value as it comes out; a constant every pass's. Each pass's values, and
// their order within it, are a group of one's; a group of one is the unit clock for clock. A Hadamard or a rotary
// instruction runs in a group of one.
module logos_simd_p #(parameter ROWS = 320, VREGS = 8, SREGS = 8, FU = 16, PAIRS = ROWS, PASSES = 1)
  (input clk, input rst,
   input issue, input [63:0] instr, output busy, input [7:0] group, input [15:0] group_rows,
   input wr, input [3:0] wr_reg, input [15:0] wr_row, input [511:0] wr_data, input len_wr, input [19:0] len_val,
   input [3:0] rd_reg, input [15:0] rd_row, output [511:0] rd_data, input [3:0] rd_s, input [7:0] rd_s_pass,
   output [31:0] rd_s_val);
  localparam FMUL = @FMUL@, FADD = @FADD@, LANE = @LANE@, TREE = @TREE@, EW = @EW@;
  // A lane's add's clocks from the operands chosen to its sum, and its multiply's to its product: the choice
  // registered, then the pipelined unit.
  localparam ADD = FADD + 1, MUL = FMUL + 1, RNEP = @RNE@, SALIGN = @SALIGN@;
  // A walk's waits and slots at the widths of the counters compared with them.
  localparam [15:0] WAIT_ADD = FADD, WAIT_LANE = LANE;
  localparam [7:0] SLOTS = ADD;
  `LOGOS_RAMSTYLE_ASYNC(VREGS) reg [19:0] len [0:VREGS-1];
  // The scalar registers, SREGS a pass: pass p's register k at p·SREGS + k.
  (* ramstyle = "logic" *) reg [31:0] sr [0:SREGS*PASSES-1];
  assign rd_s_val = sr[rd_s_pass * SREGS + rd_s];

  reg [63:0] ir;
  reg running;
  assign busy = running;
  wire [3:0] opc = ir[3:0], sub = ir[7:4], ra = ir[11:8], rb = ir[15:12], rdd = ir[19:16], rs = ir[23:20];
  // The operand's length, taken at issue, and its rows a pass: an empty register walks its first row.
  reg [19:0] n;
  // The group of passes taken at issue (gp of them, gr rows a pass), the pass a walk reads (pp) and its first row
  // (pbase); a Dot's or Sum's subgroup (its first pass sg, that pass's first row sgbase), its slot in its round of ADD
  // clocks and the slot's pass's first row (sbase); a Map's pass
  // written (o_pp) and its first row (o_pbase); the clocks of a scalar operation's walk (wt).
  wire [7:0] g_in = (group == 8'd0) ? 8'd1 : group;
  reg [7:0] gp, pp, slot, o_pp, sg;
  reg [15:0] gr, pbase, sbase, o_pbase, wt, sgbase;
  wire [19:0] rows_n = (n + 20'd15) >> 4;
  wire [15:0] rows = (rows_n == 20'd0) ? 16'd1 : rows_n[15:0];

  // The registers in block RAM, a bank a register, three copies a bank written alike below PAIRS rows, each read
  // registered. The rows each copy is asked (row_a, row_b, row_c) and the write chosen (w_*), and both registered: the
  // rows read (ra_*), and each bank's enables, the row, the data and the mask written (wq_*).
  localparam RA = (ROWS > 1) ? $clog2(ROWS) : 1;
  localparam PA = (PAIRS > 1) ? $clog2(PAIRS) : 1;
  localparam [16:0] PAIR_ROWS = PAIRS;
  wire [512*VREGS-1:0] qa, qb, qc;
  reg [15:0] row_a, row_b, row_c;
  (* maxfan = 64 *) reg [15:0] ra_a, ra_b, ra_c;
  reg w_en;
  reg [3:0] w_reg;
  reg [15:0] w_row;
  reg [511:0] w_data;
  reg [63:0] w_mask;
  (* maxfan = 64 *) reg [VREGS-1:0] wq_a, wq_p;
  (* maxfan = 64 *) reg [15:0] wq_row;
  (* maxfan = 64 *) reg [63:0] wq_mask;
  reg [511:0] wq_data;
  integer kb;
  always @(posedge clk) begin
    ra_a <= row_a; ra_b <= row_b; ra_c <= row_c;
    for (kb = 0; kb < VREGS; kb = kb + 1) begin
      wq_a[kb] <= !rst && w_en && w_reg == kb[3:0];
      wq_p[kb] <= !rst && w_en && w_reg == kb[3:0] && (PAIRS >= ROWS || {1'b0, w_row} < PAIR_ROWS);
    end
    wq_row <= w_row; wq_data <= w_data; wq_mask <= w_mask;
  end
  genvar l, r, k2;
  generate
    for (r = 0; r < VREGS; r = r + 1) begin : bank
      logos_bram_w512 #(.D(ROWS), .A(RA)) ca(.clk(clk), .ra(ra_a[RA-1:0]), .rd(qa[512*r +: 512]), .we(wq_a[r]),
        .wa(wq_row[RA-1:0]), .wd(wq_data), .wm(wq_mask));
      logos_bram_w512 #(.D(PAIRS), .A(PA)) cb(.clk(clk), .ra(ra_b[PA-1:0]), .rd(qb[512*r +: 512]), .we(wq_p[r]),
        .wa(wq_row[PA-1:0]), .wd(wq_data), .wm(wq_mask));
      logos_bram_w512 #(.D(PAIRS), .A(PA)) cc(.clk(clk), .ra(ra_c[PA-1:0]), .rd(qc[512*r +: 512]), .we(wq_p[r]),
        .wa(wq_row[PA-1:0]), .wd(wq_data), .wm(wq_mask));
    end
  endgenerate
  reg [3:0] rd_q, rd_q0;
  always @(posedge clk) begin rd_q0 <= rd_reg; rd_q <= rd_q0; end
  assign rd_data = qa[512*rd_q +: 512];

  // The phases of an instruction.
  localparam [3:0] P_IDLE = 4'd0, P_EW = 4'd1, P_MAP = 4'd2, P_SUM = 4'd3, P_MAX = 4'd4, P_FOLD = 4'd5, P_WAIT = 4'd6,
                   P_NEAR = 4'd7, P_FAR = 4'd8, P_ROT = 4'd9, P_TURN = 4'd10, P_PAIRS = 4'd11;
  reg [3:0] phase;
  // Reads: the row the next read takes, the slice of it, and whether the phase's reads are done.
  reg [15:0] row;
  reg [3:0] slice;
  reg reading;
  reg [19:0] stride;
  reg [7:0] timer;
  wire [15:0] partner = row + stride[19:4];
  wire row_low = (row & stride[19:4]) == 16'd0;
  wire [19:0] pairs_now = len[rb];
  reg [19:0] pairs;
  wire [19:0] blocks_n = (pairs + 20'd15) >> 4;
  wire [19:0] q_first = {row, 4'd0} + pairs;
  // This clock's read: whether one is asked, and its row and slice; the same a clock later (the row read), two clocks
  // later (the word out) and three (the operands registered).
  // Each read's pass, its row within the pass, and whether it is the walk's last, beside it.
  reg iss, iss_last;
  reg [15:0] iss_row, iss_rel;
  reg [3:0] iss_slice;
  reg [7:0] iss_pass;
  reg t0_v, t1_v, t2_v, t0_last, t1_last, t2_last;
  reg [15:0] t0_row, t1_row, t2_row, t0_rel, t1_rel, t2_rel;
  reg [3:0] t0_slice, t1_slice, t2_slice;
  reg [7:0] t0_pass, t1_pass, t2_pass;
  always @(posedge clk) begin
    t0_v <= iss && !rst; t0_row <= iss_row; t0_slice <= iss_slice;
    t1_v <= t0_v && !rst; t1_row <= t0_row; t1_slice <= t0_slice;
    t2_v <= t1_v && !rst; t2_row <= t1_row; t2_slice <= t1_slice;
    t0_pass <= iss_pass; t0_rel <= iss_rel; t0_last <= iss_last;
    t1_pass <= t0_pass; t1_rel <= t0_rel; t1_last <= t0_last;
    t2_pass <= t1_pass; t2_rel <= t1_rel; t2_last <= t1_last;
  end
  // The operands, registered the clock after their read.
  reg [511:0] va, vb, da, db, dc, sn;
  always @(posedge clk) begin
    va <= qa[512*ra +: 512]; vb <= qa[512*rb +: 512]; da <= qa[512*rdd +: 512]; db <= qb[512*rdd +: 512];
    dc <= qc[512*rdd +: 512]; sn <= qa[512*rs +: 512];
  end
  wire [31:0] sv = sr[t2_pass * SREGS + rs];

  // Each lane's units: four multiplies and two adds every instruction shares, their operands by instruction and
  // phase. A Dot's or Sum's lane adds its running sum (acc_in) to the row's term; a Hadamard pair's sum and difference;
  // a rotary pair's four products, their difference and their sum.
  // A pass's running sums or maxima (pacc[0] a maximum's, every pass's its own for Dot and Sum), and the sums a Dot's
  // or Sum's add takes this clock: its pass's.
  (* ramstyle = "logic" *) reg [511:0] pacc [0:PASSES-1];
  reg [511:0] acc_in;
  wire [511:0] acc = pacc[0];
  wire [511:0] m0v, m1v, m2v, m3v, a0v, a1v, xs;
  wire [511:0] term = (opc == 4'd0) ? m0v : xs;
  logos_delay #(.W(512), .D(MUL)) dxs(.clk(clk), .d(va), .q(xs));
  // A rotary step's rows and pairs, held through the step: lane l turns pair 16·row + l, its elements p and q.
  wire [16*20-1:0] rp, rq;
  wire [15:0] ron;
  wire near = stride < 20'd16;
  wire [3:0] h = stride[3:0];
  wire rot = phase == P_TURN, had = phase == P_NEAR || phase == P_FAR, sum = phase == P_SUM, fma = opc == 4'd15;
  // A multiply-add's addend, MUL clocks on, beside its product: the register the scalar field names, or the pass's
  // scalar.
  wire [511:0] sn_m;
  wire [31:0] sv_m;
  logos_delay #(.W(512), .D(MUL)) dma_v(.clk(clk), .d(sn), .q(sn_m));
  logos_delay #(.W(32), .D(MUL)) dma_s(.clk(clk), .d(sv), .q(sv_m));
  reg [7:0] turn_t;
  generate
    for (l = 0; l < 16; l = l + 1) begin : lane
      wire [31:0] x = va[32*l +: 32], y = vb[32*l +: 32];
      wire [31:0] mine = da[32*l +: 32];
      wire [31:0] other = (h == 4'd1) ? da[32*(l ^ 1) +: 32] : (h == 4'd2) ? da[32*(l ^ 2) +: 32]
                        : (h == 4'd4) ? da[32*(l ^ 4) +: 32] : da[32*(l ^ 8) +: 32];
      wire [31:0] far = db[32*l +: 32];
      wire low = ((l & h) == 0);
      wire [19:0] i = {row, 4'd0} + l;
      assign ron[l] = i < pairs;
      wire [19:0] p = sub[0] ? i : {i[18:0], 1'b0};
      wire [19:0] q = sub[0] ? i + pairs : {i[18:0], 1'b1};
      assign rp[20*l +: 20] = p;
      assign rq[20*l +: 20] = q;
      wire [511:0] wp = (p[19:4] == row_b) ? db : (p[19:4] == row_c) ? dc : da;
      wire [511:0] wq = (q[19:4] == row_b) ? db : (q[19:4] == row_c) ? dc : da;
      wire [31:0] x0 = wp[32*p[3:0] +: 32], x1 = wq[32*q[3:0] +: 32];
      wire [31:0] c = vb[32*l +: 32], s = sn[32*l +: 32];
      wire [31:0] by = (opc == 4'd3 || opc == 4'd0 || fma) ? y : (opc == 4'd4 || opc == 4'd13) ? sv : 32'h3f80_0000;
      wire [31:0] bin_b = (opc == 4'd3) ? y : sv;
      wire [31:0] nb = (sub == 4'd2) ? {~bin_b[31], bin_b[30:0]} : bin_b;
      wire [31:0] m0, m1, m2, m3;
      reg [31:0] m0_a, m0_b, m1_a, m1_b, m2_a, m2_b, m3_a, m3_b;
      always @(posedge clk) begin
        m0_a <= rot ? x0 : x; m0_b <= rot ? c : by;
        m1_a <= rot ? x1 : 32'd0; m1_b <= rot ? s : 32'd0;
        m2_a <= rot ? x0 : 32'd0; m2_b <= rot ? s : 32'd0;
        m3_a <= rot ? x1 : 32'd0; m3_b <= rot ? c : 32'd0;
      end
      logos_fmul_p um0(.clk(clk), .a(m0_a), .b(m0_b), .y(m0));
      logos_fmul_p um1(.clk(clk), .a(m1_a), .b(m1_b), .y(m1));
      logos_fmul_p um2(.clk(clk), .a(m2_a), .b(m2_b), .y(m2));
      logos_fmul_p um3(.clk(clk), .a(m3_a), .b(m3_b), .y(m3));
      assign m0v[32*l +: 32] = m0;
      assign m1v[32*l +: 32] = m1;
      assign m2v[32*l +: 32] = m2;
      assign m3v[32*l +: 32] = m3;
      // A rotary block's adds take its products, held as its operands are; a multiply-add's its product and its addend,
      // MUL clocks after its read; every other add its operands; each add's operands chosen and registered the clock
      // before the add takes them.
      reg [31:0] a0_a, a0_b, a1_a, a1_b;
      always @(posedge clk) begin
        a0_a <= rot ? m2 : fma ? m0 : had ? (near ? (low ? mine : other) : mine) : sum ? acc_in[32*l +: 32] : x;
        a0_b <= rot ? m3 : fma ? (sub[0] ? sv_m : sn_m[32*l +: 32]) : had ? (near ? (low ? other : mine) : far) : sum ? term[32*l +: 32] : nb;
        a1_a <= rot ? m0 : !had ? 32'd0 : near ? other : mine;
        a1_b <= rot ? {~m1[31], m1[30:0]} : !had ? 32'd0 : near ? {~mine[31], mine[30:0]} : {~far[31], far[30:0]};
      end
      logos_fadd_p ua0(.clk(clk), .a(a0_a), .b(a0_b), .y(a0v[32*l +: 32]));
      logos_fadd_p ua1(.clk(clk), .a(a1_a), .b(a1_b), .y(a1v[32*l +: 32]));
    end
  endgenerate
  // A rotary block's sums out: its operands registered the clock after its words, its products MUL clocks on, its sums
  // ADD after those; then its turned elements placed two lanes a clock, and its three rows written.
  localparam [7:0] CAP = @READ@ + MUL + ADD, PLACED = CAP + 8'd8;

  // An elementwise instruction's row, the clock it is written: a product, a sum, a multiply-add's sum, a code, a scaled
  // or copied row.
  wire [511:0] mul_w, add_w, fma_w, va_w, code_w, scale_w;
  logos_delay #(.W(512), .D(EW - 2 - MUL)) dmw(.clk(clk), .d(m0v), .q(mul_w));
  logos_delay #(.W(512), .D(EW - 2 - ADD)) daw(.clk(clk), .d(a0v), .q(add_w));
  logos_delay #(.W(512), .D(EW - 2 - MUL - ADD)) dfw(.clk(clk), .d(a0v), .q(fma_w));
  // The operand a clock short of its write, then registered beside its power-of-two scale, so the scale's shift is a
  // stage of its own.
  wire [511:0] va_w3;
  reg [511:0] va_wr, scale_wr;
  logos_delay #(.W(512), .D(EW - 3)) dvw(.clk(clk), .d(va), .q(va_w3));
  always @(posedge clk) begin va_wr <= va_w3; scale_wr <= scale_w; end
  assign va_w = va_wr;
  wire sv_inf;
  logos_delay #(.W(1), .D(MUL + RNEP)) dsi(.clk(clk), .d((sv[30:23] == 8'hff) && (sv[22:0] == 23'd0)), .q(sv_inf));
  generate
    for (l = 0; l < 16; l = l + 1) begin : code
      // The int8 code of the product under the inverse scale (logos_code8_of in stages): the product's nearest integer
      // on logos_rne_p, its class held beside it; within ±127, a NaN zero, past 127 saturated, every code zero under an
      // infinite scale, registered; then as an f32 on logos_i2f_p.
      wire [31:0] p = m0v[32*l +: 32];
      wire signed [31:0] n1;
      wire nan1, big1, neg1;
      logos_rne_p rn(.clk(clk), .x(p), .n(n1));
      logos_delay #(.W(3), .D(@RNE@)) dcl(.clk(clk), .d({(p[30:23] == 8'hff) && (p[22:0] != 23'd0), p[30:23] >= 8'd134, p[31]}),
        .q({nan1, big1, neg1}));
      wire signed [31:0] near_c = (n1 > 32'sd127) ? 32'sd127 : (n1 < -32'sd127) ? -32'sd127 : n1;
      reg signed [31:0] code;
      always @(posedge clk) code <= (sv_inf || nan1) ? 32'sd0 : big1 ? (neg1 ? -32'sd127 : 32'sd127) : near_c;
      logos_i2f_p ci(.clk(clk), .n(code), .y(code_w[32*l +: 32]));
      logos_ldexp sc(.x(va_w3[32*l +: 32]), .k(ir[47:32]), .y(scale_w[32*l +: 32]));
    end
  endgenerate
  wire [511:0] ew_w = (opc == 4'd3 || opc == 4'd4) ? ((sub == 4'd0) ? mul_w : add_w) : (opc == 4'd13) ? code_w
                    : (opc == 4'd14) ? scale_wr : fma ? fma_w : va_w;
  // The elementwise writes: a read's row EW clocks on.
  wire ew_v, ew_last;
  wire [15:0] ew_row;
  logos_delay #(.W(18), .D(EW - 2)) dew(.clk(clk), .d({t2_v && (phase == P_EW), t2_last, t2_row}), .q({ew_v, ew_last, ew_row}));

  // The pass a scalar operation's walk feeds this clock: its wt-th, the group's last once every pass is fed.
  wire [7:0] fpass = (wt < {8'd0, gp}) ? wt[7:0] : gp - 8'd1;
  // The function units, each on the pipelined lane function: a Map's slices a clock from the operands, each slice's
  // values out LANE clocks later in order, held in fbuf until its row's last slice is out and the row is written
  // whole; a ScalarMap's operand on the first unit.
  localparam [4:0] SLICES = 16 / FU;
  reg sfeed;
  wire f_in = (phase == P_MAP && t2_v) || sfeed;
  wire [32*FU-1:0] fx, fy;
  generate
    for (l = 0; l < FU; l = l + 1) begin : fun
      wire [3:0] fcode = (f_in && (opc == 4'd5 || l == 0)) ? sub : 4'd15;
      wire [32*SLICES-1:0] cand;
      for (k2 = 0; k2 < SLICES; k2 = k2 + 1) begin : at
        assign cand[32*k2 +: 32] = va[32*(FU*k2 + l) +: 32];
      end
      wire [31:0] lane_x = (SLICES == 1) ? cand[31:0] : cand[32*t2_slice +: 32];
      assign fx[32*l +: 32] = (opc == 4'd7 && l == 0) ? sr[fpass * SREGS + ra] : lane_x;
      logos_lane_func_p fu(.clk(clk), .f(fcode), .x(fx[32*l +: 32]), .y(fy[32*l +: 32]));
    end
  endgenerate
  wire f_out;
  logos_delay #(.W(1), .D(LANE)) dfo(.clk(clk), .d(f_in && !rst), .q(f_out));
  // The slice a Map's units give next (o_slice) and whether it is its row's last (o_end); the row a Map writes next
  // (o_row) and its place in the registers (o_at, its pass's first row and o_row).
  reg [3:0] o_slice;
  reg o_end;
  reg [15:0] o_row, o_at;
  reg [511:0] fbuf;
  wire [511:0] func_v;
  generate
    for (l = 0; l < 16; l = l + 1) begin : fsl
      localparam [3:0] MINE = l / FU;
      assign func_v[32*l +: 32] = (f_out && o_slice == MINE) ? fy[32*(l % FU) +: 32] : fbuf[32*l +: 32];
    end
  endgenerate
  wire last_slice = f_out && o_end;

  // A reduction's lanes: the running sums or maxima, the rows' live lanes and, for ArgMax, each lane's index.
  reg [15:0] found;
  `LOGOS_RAMSTYLE_ASYNC(PASSES) reg [15:0] plive [0:PASSES-1];
  reg [PASSES-1:0] phave;
  reg [19:0] aidx [0:15];
  wire [15:0] live2;
  wire [15:0] take;
  wire [511:0] max_next;
  wire largest = (opc == 4'd2) || (opc == 4'd11) || (opc == 4'd12);
  // The row a Dot's or Sum's add takes: its live lanes, MUL clocks after its operands.
  wire s_last;
  wire [15:0] s_live;
  wire [7:0] s_pass;
  logos_delay #(.W(25), .D(MUL)) dsv(.clk(clk), .d({t2_rel + 16'd1 >= rows, live2, t2_pass}), .q({s_last, s_live, s_pass}));
  // A pass's last add out ADD clocks after it is taken: its sums into the tree; their fold out TREE + 1 clocks later.
  // Each valid in registers the reset clears, its pass beside it.
  reg [MUL-1:0] sv_sh;
  reg [ADD-1:0] fold_sh;
  reg [TREE:0] tw_sh;
  wire s_v = sv_sh[MUL-1], fold_v = fold_sh[ADD-1], tw_v = tw_sh[TREE];
  always @(posedge clk) begin
    sv_sh <= rst ? {MUL{1'b0}} : {sv_sh[MUL-2:0], t2_v && phase == P_SUM};
    fold_sh <= rst ? {ADD{1'b0}} : {fold_sh[ADD-2:0], s_v && s_last};
    tw_sh <= rst ? {(TREE+1){1'b0}} : {tw_sh[TREE-1:0], fold_v};
  end
  wire [7:0] fold_pass, tw_pass;
  logos_delay #(.W(8), .D(ADD)) dfd(.clk(clk), .d(s_pass), .q(fold_pass));
  logos_delay #(.W(8), .D(TREE + 1)) dtw(.clk(clk), .d(fold_pass), .q(tw_pass));
  generate
    for (l = 0; l < 16; l = l + 1) begin : red
      assign live2[l] = ({t2_rel, 4'd0} + l) < n;
      wire [31:0] x = va[32*l +: 32];
      wire [31:0] cand = (opc == 4'd12) ? ((x[30:23] == 8'd0) ? 32'd0 : {1'b0, x[30:0]}) : x;
      wire nan = (x[30:23] == 8'hff) && (x[22:0] != 23'd0);
      wire gt;
      logos_fgt g(.a(cand), .b(acc[32*l +: 32]), .y(gt));
      assign take[l] = (opc == 4'd11) ? (live2[l] && !nan && (!found[l] || gt)) : (live2[l] && gt);
      assign max_next[32*l +: 32] = take[l] ? cand : acc[32*l +: 32];
      always @* acc_in[32*l +: 32] = (phave[s_pass] && plive[s_pass][l]) ? a0v[32*l +: 32] : pacc[s_pass][32*l +: 32];
    end
  endgenerate

  // The folds: the sums by the pipelined tree, from the clock the last row's add comes out; the maxima and arg-maxima
  // by four levels of pairs, a level a clock.
  reg [511:0] tree_in;
  wire [31:0] tree_sum;
  logos_tree16_p ts(.clk(clk), .lanes(tree_in), .sum(tree_sum));
  function [31:0] fkey(input [31:0] v);
    fkey = (v[30:0] == 31'd0) ? 32'h8000_0000 : (v[31] ? ~v : {1'b1, v[30:0]});
  endfunction
  function bigger(input [31:0] a, input [31:0] b);
    begin
      bigger = 1'b0;
      if (!((b[30:23] == 8'hff) && (b[22:0] != 23'd0))) begin
        if (a[31] == 1'b0 && b[31] == 1'b0) bigger = b[30:0] > a[30:0];
        else if (a[31] == 1'b1 && b[31] == 1'b1) bigger = b[30:0] < a[30:0];
        else bigger = (b[31] == 1'b0) && !(b[30:0] == 31'd0 && a[30:0] == 31'd0);
      end
    end
  endfunction
  // Whether the lane-pair's upper value (b, its index ib, its holding fb) takes the lower's place: ArgMax's the one
  // holding the larger value, of equal values the lower index; Max's and AbsMax's the larger, a NaN never larger.
  function wins(input arg, input [31:0] a, input [31:0] b, input [19:0] ia, input [19:0] ib, input fa, input fb);
    wins = arg ? fb && (!fa || (fkey(b) > fkey(a)) || (fkey(b) == fkey(a) && ib < ia)) : bigger(a, b);
  endfunction
  reg [31:0] mv [0:15];
  reg [19:0] mi [0:15];
  reg mf [0:15];
  reg [2:0] level;
  integer t;
  // The arg-max's index as an f32, @FOLD_WRITE@ - 5 clocks after the last level leaves it.
  wire [31:0] arg_f;
  logos_i2f_p ia(.clk(clk), .n(mf[0] ? {12'd0, mi[0]} : 32'sd0), .y(arg_f));

  // A scalar operation's multiply and add.
  wire [31:0] s_ra = sr[fpass * SREGS + ra], s_rb = sr[fpass * SREGS + rb];
  wire [31:0] s_m, s_a, s_bin, s_m_at;
  logos_fmul_p sm(.clk(clk), .a(s_ra), .b(s_rb), .y(s_m));
  logos_fadd_p sa(.clk(clk), .a(s_ra), .b(sub == 4'd2 ? {~s_rb[31], s_rb[30:0]} : s_rb), .y(s_a));
  generate
    if (SALIGN > 0) begin : align
      logos_delay #(.W(32), .D(SALIGN)) dsm(.clk(clk), .d(s_m), .q(s_m_at));
    end else begin : same
      assign s_m_at = s_m;
    end
  endgenerate
  assign s_bin = (sub == 4'd0) ? s_m_at : s_a;

  // A Hadamard stage's writes: a near stage's row ADD clocks after its operands; a far stage's pair, its low row's sum
  // and the clock after its partner's difference.
  reg [15:0] partner_t0, partner_t1, partner_t2;
  always @(posedge clk) begin partner_t0 <= partner; partner_t1 <= partner_t0; partner_t2 <= partner_t1; end
  wire hd_v;
  wire [31:0] hd_rows;
  logos_delay #(.W(33), .D(ADD)) dhd(.clk(clk), .d({t2_v && had, partner_t2, t2_row}), .q({hd_v, hd_rows}));
  // A far stage's pairs between their read and their sum's write.
  reg [7:0] in_far;
  always @(posedge clk)
    if (rst || phase != P_FAR) in_far <= 8'd0;
    else in_far <= in_far + (iss ? 8'd1 : 8'd0) - (hd_v ? 8'd1 : 8'd0);
  wire [511:0] hd_near;
  generate
    for (l = 0; l < 16; l = l + 1) begin : hn
      assign hd_near[32*l +: 32] = ((l & h) == 0) ? a0v[32*l +: 32] : a1v[32*l +: 32];
    end
  endgenerate
  reg hi_v;
  reg [15:0] hi_row;
  reg [511:0] hi_d;

  // A rotary block's turned elements as three masked rows, each element in the row it was read from: each lane's pair's
  // rows (0 the row at hand, 1 copy B's, 2 copy C's) and places registered with the block's operands; the rows taken
  // as read and the turned values as they come out; then two lanes a clock placed in lane order, a later lane's element
  // over an earlier one's, as logos_simd's loop places them.
  reg [1:0] p_to [0:15], q_to [0:15];
  reg [3:0] p_at [0:15], q_at [0:15];
  reg [15:0] on_r;
  reg [511:0] ry0, ry1;
  reg [511:0] rw_a, rw_b, rw_c;
  reg [63:0] rw_am, rw_bm, rw_cm;
  reg [511:0] nx_a, nx_b, nx_c;
  reg [63:0] nx_am, nx_bm, nx_cm;
  integer k, kk, j, kp;
  // The block's steps marked a clock ahead, each from the step it marks, so no load or write waits on a compare of the
  // step: its operands' capture (t_cap), its placing steps (t_place), and its three writes (t_w1..t_w3).
  reg t_cap, t_place, t_w1, t_w2, t_w3;
  wire [7:0] t_next = turn_t + 8'd1;
  wire [7:0] t_next_lane = t_next - (CAP + 8'd1);
  // A placing step's two lanes, selected a clock ahead with the step's marks: each lane's whether, its two elements'
  // rows and places (one-hot), and its two values — the sums as they are taken, at the step that takes them — so a
  // place is an enable of a lane, not a shift.
  reg s_on [0:1];
  reg [1:0] s_pto [0:1], s_qto [0:1];
  reg [15:0] s_pat [0:1], s_qat [0:1];
  reg [31:0] s_pv [0:1], s_qv [0:1];
  always @* begin
    nx_a = rw_a; nx_b = rw_b; nx_c = rw_c; nx_am = rw_am; nx_bm = rw_bm; nx_cm = rw_cm;
    for (kk = 0; kk < 2; kk = kk + 1)
      if (s_on[kk])
        for (j = 0; j < 16; j = j + 1) begin
          if (s_pat[kk][j])
            case (s_pto[kk])
              2'd1: begin nx_b[32*j +: 32] = s_pv[kk]; nx_bm[4*j +: 4] = 4'hf; end
              2'd2: begin nx_c[32*j +: 32] = s_pv[kk]; nx_cm[4*j +: 4] = 4'hf; end
              default: begin nx_a[32*j +: 32] = s_pv[kk]; nx_am[4*j +: 4] = 4'hf; end
            endcase
          if (s_qat[kk][j])
            case (s_qto[kk])
              2'd1: begin nx_b[32*j +: 32] = s_qv[kk]; nx_bm[4*j +: 4] = 4'hf; end
              2'd2: begin nx_c[32*j +: 32] = s_qv[kk]; nx_cm[4*j +: 4] = 4'hf; end
              default: begin nx_a[32*j +: 32] = s_qv[kk]; nx_am[4*j +: 4] = 4'hf; end
            endcase
        end
  end

  // The rows asked: copy A at the row at hand while the unit runs, at the engine's row while it is idle; copies B and C
  // at a far Hadamard stage's partner row and a rotary block's two rows.
  always @* begin
    iss = 1'b0; iss_row = pbase + row; iss_slice = slice; iss_rel = row; iss_pass = pp;
    iss_last = row + 16'd1 >= rows && pp + 8'd1 >= gp;
    if (phase == P_SUM) begin iss_row = sbase + row; iss_pass = sg + slot; end
    if (running) case (phase)
      P_EW, P_MAP, P_MAX, P_NEAR: iss = reading;
      P_SUM: iss = reading && sg + slot < gp;
      P_FAR: iss = reading && row_low && timer == 8'd0;
      default: ;
    endcase
    row_a = running ? iss_row : rd_row;
    row_b = 16'd0; row_c = 16'd0;
    if (phase == P_FAR) row_b = partner;
    else if (phase == P_ROT || phase == P_TURN) begin
      if (sub[0]) begin row_b = q_first[19:4]; row_c = q_first[19:4] + 16'd1; end
      else begin row_b = {row[14:0], 1'b0}; row_c = {row[14:0], 1'b1}; end
    end
  end

  // A turn step's rows B and C a clock behind, for its writes: the step holds its rows from its start.
  reg [15:0] wrow_b, wrow_c;
  always @(posedge clk) begin wrow_b <= row_b; wrow_c <= row_c; end

  // The one write a clock: the engine's beat while the unit is idle, or the row the instruction writes.
  always @* begin
    w_en = 1'b0; w_reg = rdd; w_row = row; w_data = 512'd0; w_mask = {64{1'b1}};
    if (wr && !running) begin
      w_en = 1'b1; w_reg = wr_reg; w_row = wr_row; w_data = wr_data;
    end else if (running) begin
      if (ew_v) begin w_en = 1'b1; w_row = ew_row; w_data = ew_w; end
      else if (phase == P_MAP && last_slice) begin w_en = 1'b1; w_row = o_at; w_data = func_v; end
      else if (hd_v && phase == P_NEAR) begin w_en = 1'b1; w_row = hd_rows[15:0]; w_data = hd_near; end
      else if (hd_v && phase == P_FAR) begin w_en = 1'b1; w_row = hd_rows[15:0]; w_data = a0v; end
      else if (hi_v) begin w_en = 1'b1; w_row = hi_row; w_data = hi_d; end
      else if (phase == P_TURN && t_w1) begin w_en = 1'b1; w_row = row; w_data = rw_a; w_mask = rw_am; end
      else if (phase == P_TURN && t_w2) begin w_en = 1'b1; w_row = wrow_b; w_data = rw_b; w_mask = rw_bm; end
      else if (phase == P_TURN && t_w3) begin w_en = 1'b1; w_row = wrow_c; w_data = rw_c; w_mask = rw_cm; end
    end
  end

  always @(posedge clk) begin
    if (rst) begin
      running <= 1'b0; phase <= P_IDLE; reading <= 1'b0; sfeed <= 1'b0; hi_v <= 1'b0;
    end else begin
      sfeed <= 1'b0; hi_v <= 1'b0;
      if (len_wr) len[wr_reg] <= len_val;
      if (issue && !running) begin
        ir <= instr; running <= 1'b1; n <= len[instr[11:8]]; row <= 16'd0; slice <= 4'd0; reading <= 1'b1;
        stride <= 20'd1; timer <= 8'd0; found <= 16'd0; phave <= {PASSES{1'b0}}; o_slice <= 4'd0; o_end <= SLICES == 5'd1;
        o_row <= 16'd0;
        gp <= g_in; gr <= group_rows; pp <= 8'd0; pbase <= 16'd0; slot <= 8'd0; sbase <= 16'd0; o_pp <= 8'd0;
        sg <= 8'd0; sgbase <= 16'd0;
        o_pbase <= 16'd0; o_at <= 16'd0; wt <= 16'd0;
        for (kp = 0; kp < PASSES; kp = kp + 1)
          if (instr[3:0] == 4'd2 || instr[3:0] == 4'd11) for (k = 0; k < 16; k = k + 1) pacc[kp][32*k +: 32] <= 32'hff80_0000;
          else pacc[kp] <= 512'd0;
        case (instr[3:0])
          4'd0, 4'd1: phase <= P_SUM;
          4'd2, 4'd11, 4'd12: phase <= P_MAX;
          4'd5: phase <= P_MAP;
          4'd6: phase <= P_WAIT;
          4'd7: begin phase <= P_WAIT; sfeed <= 1'b1; end
          4'd8: begin
            for (kp = 0; kp < PASSES; kp = kp + 1) if (kp[7:0] < g_in) sr[kp * SREGS + instr[19:16]] <= instr[63:32];
            running <= 1'b0;
          end
          4'd3, 4'd4, 4'd13, 4'd14, 4'd15, 4'd9, 4'd10: phase <= P_EW;
          default: running <= 1'b0;
        endcase
      end else if (running) begin
        case (phase)
          // Elementwise rows, and a Hadamard's or a rotary's copy: a row a clock, done once the last is written.
          P_EW: begin
            len[rdd] <= n;
            if (reading) begin
              if (row + 16'd1 < rows) row <= row + 16'd1;
              else if (pp + 8'd1 < gp) begin row <= 16'd0; pp <= pp + 8'd1; pbase <= pbase + gr; end
              else reading <= 1'b0;
            end else if (ew_v && ew_last) begin
              row <= 16'd0; pp <= 8'd0; pbase <= 16'd0;
              if (opc == 4'd9) begin
                if (n <= 20'd1) running <= 1'b0; else begin phase <= P_NEAR; reading <= 1'b1; end
              end else if (opc == 4'd10) phase <= P_PAIRS;
              else running <= 1'b0;
            end
          end
          // A rotary's pairs, once its copy is written.
          P_PAIRS: begin
            pairs <= pairs_now;
            if (pairs_now == 20'd0) running <= 1'b0; else begin phase <= P_ROT; turn_t <= 8'd0; end
          end
          // A rotary block: its rows asked; the operands registered the clock after their words, and each lane's
          // pair's rows and places; the products MUL clocks after; the sums ADD after them; the rows as read and the
          // sums taken; the elements placed two lanes a clock; then the rows written a row a clock.
          P_ROT: begin
            phase <= P_TURN; turn_t <= 8'd0;
            t_cap <= CAP == 8'd0; t_place <= 1'b0; t_w1 <= 1'b0; t_w2 <= 1'b0; t_w3 <= 1'b0;
          end
          P_TURN: begin
            turn_t <= t_next;
            t_cap <= t_next == CAP; t_place <= t_next > CAP && t_next <= PLACED;
            t_w1 <= t_next == PLACED + 8'd1; t_w2 <= t_next == PLACED + 8'd2; t_w3 <= t_next == PLACED + 8'd3;
            for (kp = 0; kp < 2; kp = kp + 1) begin
              k = 2 * t_next_lane[2:0] + kp;
              s_on[kp] <= on_r[k]; s_pto[kp] <= p_to[k]; s_qto[kp] <= q_to[k];
              s_pat[kp] <= 16'd1 << p_at[k]; s_qat[kp] <= 16'd1 << q_at[k];
              s_pv[kp] <= t_cap ? a1v[32*k +: 32] : ry0[32*k +: 32];
              s_qv[kp] <= t_cap ? a0v[32*k +: 32] : ry1[32*k +: 32];
            end
            if (turn_t == 8'd1)
              for (k = 0; k < 16; k = k + 1) begin
                p_to[k] <= (rp[20*k+4 +: 16] == row_b) ? 2'd1 : (rp[20*k+4 +: 16] == row_c) ? 2'd2 : 2'd0;
                q_to[k] <= (rq[20*k+4 +: 16] == row_b) ? 2'd1 : (rq[20*k+4 +: 16] == row_c) ? 2'd2 : 2'd0;
                p_at[k] <= rp[20*k +: 4]; q_at[k] <= rq[20*k +: 4]; on_r[k] <= ron[k];
              end
            if (t_cap) begin
              rw_a <= da; rw_b <= db; rw_c <= dc; rw_am <= 64'd0; rw_bm <= 64'd0; rw_cm <= 64'd0; ry0 <= a1v; ry1 <= a0v;
            end
            if (t_place) begin
              rw_a <= nx_a; rw_b <= nx_b; rw_c <= nx_c; rw_am <= nx_am; rw_bm <= nx_bm; rw_cm <= nx_cm;
            end
            if (t_w3) begin
              if ({4'd0, row} + 20'd1 >= blocks_n) running <= 1'b0;
              else begin row <= row + 16'd1; phase <= P_ROT; end
            end
          end
          // A Map: slices a clock, rows one after another; each row written when its last slice is out.
          P_MAP: begin
            len[rdd] <= n;
            if (reading) begin
              if ({1'b0, slice} + 5'd1 >= SLICES) begin
                slice <= 4'd0;
                if (row + 16'd1 < rows) row <= row + 16'd1;
                else if (pp + 8'd1 < gp) begin row <= 16'd0; pp <= pp + 8'd1; pbase <= pbase + gr; end
                else reading <= 1'b0;
              end else slice <= slice + 4'd1;
            end
            if (f_out) begin
              if (!last_slice) begin fbuf <= func_v; o_slice <= o_slice + 4'd1; o_end <= {1'b0, o_slice} + 5'd2 >= SLICES; end
              else begin
                o_slice <= 4'd0;
                o_end <= SLICES == 5'd1;
                if (o_row + 16'd1 < rows) begin o_row <= o_row + 16'd1; o_at <= o_at + 16'd1; end
                else if (o_pp + 8'd1 < gp) begin
                  o_row <= 16'd0; o_pp <= o_pp + 8'd1; o_pbase <= o_pbase + gr; o_at <= o_pbase + gr;
                end
                else begin o_row <= 16'd0; running <= 1'b0; end
              end
            end
          end
          // A scalar operation or a ScalarMap: its value the clock its unit's latency ends.
          P_WAIT: begin
            sfeed <= opc == 4'd7 && wt + 16'd1 < {8'd0, gp};
            if (wt >= ((opc == 4'd6) ? WAIT_ADD : WAIT_LANE)) begin
              sr[(wt - ((opc == 4'd6) ? FADD : LANE)) * SREGS + rdd] <= (opc == 4'd6) ? s_bin : fy[31:0];
              if (wt - ((opc == 4'd6) ? FADD : LANE) + 16'd1 >= {8'd0, gp}) running <= 1'b0;
            end
            wt <= wt + 16'd1;
          end
          // Dot and Sum: a row each ADD clocks; each add of a row's terms to the lanes' sums MUL clocks after the
          // row's operands, its sums out ADD clocks later, the next row's add the same clock; a subgroup's rows to their
          // end, then the next subgroup's.
          P_SUM: begin
            if (reading) begin
              if (slot + 8'd1 < SLOTS) begin slot <= slot + 8'd1; sbase <= sbase + gr; end
              else begin
                slot <= 8'd0; sbase <= sgbase;
                if (row + 16'd1 < rows) row <= row + 16'd1;
                else if (sg + SLOTS < gp) begin
                  row <= 16'd0; sg <= sg + SLOTS; sgbase <= sgbase + SLOTS * gr; sbase <= sgbase + SLOTS * gr;
                end else reading <= 1'b0;
              end
            end
            if (s_v) begin pacc[s_pass] <= acc_in; plive[s_pass] <= s_live; phave[s_pass] <= 1'b1; end
            if (fold_v)
              for (k = 0; k < 16; k = k + 1)
                tree_in[32*k +: 32] <= plive[fold_pass][k] ? a0v[32*k +: 32] : pacc[fold_pass][32*k +: 32];
            if (tw_v) begin
              sr[tw_pass * SREGS + rdd] <= tree_sum;
              if (tw_pass + 8'd1 >= gp) running <= 1'b0;
            end
          end
          // Max, AbsMax and ArgMax: a row a clock, each lane keeping its largest.
          P_MAX: begin
            if (reading) begin
              if (row + 16'd1 >= rows) reading <= 1'b0; else row <= row + 16'd1;
            end
            if (t2_v) begin
              pacc[0] <= max_next;
              if (opc == 4'd11)
                for (k = 0; k < 16; k = k + 1)
                  if (take[k]) begin aidx[k] <= {t2_rel, 4'd0} + k[19:0]; found[k] <= 1'b1; end
              if (t2_rel + 16'd1 >= rows) begin phase <= P_FOLD; level <= 3'd0; end
            end
          end
          P_FOLD: begin
            begin
              level <= level + 3'd1;
              case (level)
                3'd0: for (t = 0; t < 16; t = t + 1) begin mv[t] <= acc[32*t +: 32]; mi[t] <= aidx[t]; mf[t] <= found[t]; end
                3'd1: for (t = 0; t < 16; t = t + 2)
                  if (wins(opc == 4'd11, mv[t], mv[t+1], mi[t], mi[t+1], mf[t], mf[t+1])) begin mv[t] <= mv[t+1]; mi[t] <= mi[t+1]; mf[t] <= 1'b1; end
                3'd2: for (t = 0; t < 16; t = t + 4)
                  if (wins(opc == 4'd11, mv[t], mv[t+2], mi[t], mi[t+2], mf[t], mf[t+2])) begin mv[t] <= mv[t+2]; mi[t] <= mi[t+2]; mf[t] <= 1'b1; end
                3'd3: for (t = 0; t < 16; t = t + 8)
                  if (wins(opc == 4'd11, mv[t], mv[t+4], mi[t], mi[t+4], mf[t], mf[t+4])) begin mv[t] <= mv[t+4]; mi[t] <= mi[t+4]; mf[t] <= 1'b1; end
                3'd4:
                  if (wins(opc == 4'd11, mv[0], mv[8], mi[0], mi[8], mf[0], mf[8])) begin mv[0] <= mv[8]; mi[0] <= mi[8]; mf[0] <= 1'b1; end
                3'd@FOLD_WRITE@: begin
                  sr[pp * SREGS + rdd] <= (opc == 4'd11) ? arg_f : (mv[0][30:0] == 31'd0) ? 32'd0 : mv[0];
                  if (pp + 8'd1 < gp) begin
                    pp <= pp + 8'd1; pbase <= pbase + gr; row <= 16'd0; reading <= 1'b1; found <= 16'd0; phase <= P_MAX;
                    if (opc == 4'd11 || opc == 4'd2) for (k = 0; k < 16; k = k + 1) pacc[0][32*k +: 32] <= 32'hff80_0000;
                    else pacc[0] <= 512'd0;
                  end else running <= 1'b0;
                end
                default: ;
              endcase
            end
          end
          // A Hadamard stage within rows: a row a clock; the next stage once the last row is written.
          P_NEAR: begin
            if (reading) begin
              if (row + 16'd1 >= rows) reading <= 1'b0; else row <= row + 16'd1;
            end else if (hd_v && hd_rows[15:0] + 16'd1 >= rows) begin
              row <= 16'd0; reading <= 1'b1; timer <= 8'd0;
              if ((stride << 1) >= len[rdd]) running <= 1'b0;
              else if ((stride << 1) >= 20'd16) phase <= P_FAR;
              stride <= stride << 1;
            end
          end
          // A Hadamard stage between rows: a low row and its partner every other clock, their sum and difference
          // written in turn.
          P_FAR: begin
            if (hd_v) begin hi_v <= 1'b1; hi_row <= hd_rows[31:16]; hi_d <= a1v; end
            if (reading) begin
              if (timer != 8'd0) timer <= timer - 8'd1;
              else begin
                if (row_low) timer <= 8'd1;
                if (row + 16'd1 >= rows) reading <= 1'b0; else row <= row + 16'd1;
              end
            end else if (in_far == 8'd0 && !hi_v) begin
              row <= 16'd0; reading <= 1'b1; timer <= 8'd0;
              if ((stride << 1) >= len[rdd]) running <= 1'b0;
              stride <= stride << 1;
            end
          end
          default: running <= 1'b0;
        endcase
      end
    end
  end
endmodule
"#;

const SIMD_VERILOG: &str = r#"
// One lane's function of one element through one core of each function: sigmoid is recip(1 + exp(-x)) and silu x
// times it, softplus u + exp(-u) above its top and ln1p(exp(u)) below it, ln1p(t) ln(1 + t) corrected by
// recip(1 + t) — each the f32 operations of its module in their order. A core no chosen function uses holds its
// operand at zero. STAGED takes the function in four stages a step apart (LANE_FUNC_STAGES): the exponential; the add
// after it; the reciprocal, the logarithm, the reciprocal square root and one less than the sum; ln1p's correction and
// the choice — an operand taken at a step its value four steps later; without it the value is the operand's at once.
module logos_lane_func #(parameter STAGED = 0) (input clk, input rst, input [3:0] f, input [31:0] x, output [31:0] y);
  // Stage 1: the operand's decoding, the exponential, the binary16 round trip.
  wire [31:0] xf = `LOGOS_FTZ(x);
  wire [31:0] nxf = {~xf[31], xf[30:0]};
  wire is_exp = f == 4'd0, is_recip = f == 4'd1, is_rsqrt = f == 4'd2, is_sig = f == 4'd3, is_silu = f == 4'd4;
  wire is_sp = f == 4'd5, is_l1p = f == 4'd6, is_half = f == 4'd8;
  wire sp_above;
  logos_flt top(.a(`LOGOS_SOFTPLUS_TOP), .b(xf), .y(sp_above));
  wire sp_past = is_sp && sp_above, sp_near = is_sp && !sp_above;
  wire gate = is_sig || is_silu;
  wire on_l1p = is_l1p || sp_near;
  wire [31:0] e;
  logos_exp ue(.x_in(is_exp ? x : (gate || sp_past) ? nxf : sp_near ? xf : 32'd0), .y(e));
  wire [15:0] half;
  wire [31:0] yh;
  logos_f32_to_f16 uh(.x(is_half ? x : 32'd0), .h(half));
  logos_f16_to_f32 uw(.h(half), .x(yh));
  // Each stage's operands: the stage before's values, or its registers a step later.
  wire [3:0] f1, f2, f3;
  wire [31:0] x1, xf1, e1, yh1, x2, e2, yh2, t2, s2, x3, e3, yh3, t3, s3, r3, lu3, um13, q3;
  wire sp_above1, sp_past1, gate1, on_l1p1, is_l1p1, is_recip1, is_rsqrt1, is_silu1;
  wire sp_above2, gate2, on_l1p2, is_recip2, is_rsqrt2, is_silu2, sp_above3, on_l1p3, is_silu3;
  // Stage 2: one plus the exponential for the gates, u plus it past softplus's top; one plus ln1p's operand,
  // softplus's exponential below it.
  wire [31:0] t = is_l1p1 ? xf1 : `LOGOS_FTZ(e1);
  wire [31:0] s;
  logos_fadd ua(.a(sp_past1 ? xf1 : 32'h3f80_0000), .b((gate1 || sp_past1) ? e1 : on_l1p1 ? t : 32'd0), .y(s));
  // Stage 3: the reciprocal, the logarithm, the reciprocal square root, one less than the sum.
  wire [31:0] r, lu, um1, q;
  logos_recip ur(.x_in(is_recip2 ? x2 : (gate2 || on_l1p2) ? s2 : 32'd0), .y(r));
  logos_ln ul(.x_in(on_l1p2 ? s2 : 32'd0), .y(lu));
  logos_fsub uo(.a(on_l1p2 ? s2 : 32'd0), .b(32'h3f80_0000), .y(um1));
  logos_rsqrt uq(.x_in(is_rsqrt2 ? x2 : 32'd0), .y(q));
  // Stage 4: silu's product with its sigmoid, or ln1p's correction, and the function chosen.
  wire [31:0] err, p, z;
  logos_fsub ud(.a(on_l1p3 ? t3 : 32'd0), .b(um13), .y(err));
  logos_fmul um(.a(is_silu3 ? x3 : on_l1p3 ? err : 32'd0), .b(r3), .y(p));
  logos_fadd uz(.a(lu3), .b(p), .y(z));
  wire [31:0] l1p = (s3 == 32'h3f80_0000) ? t3 : z;
  reg [31:0] pick;
  always @* case (f3)
    4'd0: pick = e3; 4'd1: pick = r3; 4'd2: pick = q3; 4'd3: pick = r3; 4'd4: pick = p; 4'd5: pick = sp_above3 ? s3 : l1p;
    4'd6: pick = l1p; 4'd8: pick = yh3;
    default: pick = {~x3[31], x3[30:0]};
  endcase
  generate
    if (STAGED) begin : stages
      reg [3:0] rf1, rf2, rf3;
      reg [31:0] rx1, rxf1, re1, ryh1, rx2, re2, ryh2, rt2, rs2, rx3, re3, ryh3, rt3, rs3, rr3, rlu3, rum13, rq3, ry;
      reg ra1, rp1, rg1, ro1, rl1, rr1, rq1, rs1, ra2, rg2, ro2, rr2, rq2, rsi2, ra3, ro3, rsi3;
      always @(posedge clk) begin
        rf1 <= f; rx1 <= x; rxf1 <= xf; re1 <= e; ryh1 <= yh;
        ra1 <= sp_above; rp1 <= sp_past; rg1 <= gate; ro1 <= on_l1p; rl1 <= is_l1p; rr1 <= is_recip; rq1 <= is_rsqrt; rs1 <= is_silu;
        rf2 <= rf1; rx2 <= rx1; re2 <= re1; ryh2 <= ryh1; rt2 <= t; rs2 <= s;
        ra2 <= ra1; rg2 <= rg1; ro2 <= ro1; rr2 <= rr1; rq2 <= rq1; rsi2 <= rs1;
        rf3 <= rf2; rx3 <= rx2; re3 <= re2; ryh3 <= ryh2; rt3 <= rt2; rs3 <= rs2; rr3 <= r; rlu3 <= lu; rum13 <= um1; rq3 <= q;
        ra3 <= ra2; ro3 <= ro2; rsi3 <= rsi2;
        ry <= pick;
      end
      assign {f1, x1, xf1, e1, yh1, sp_above1, sp_past1, gate1, on_l1p1, is_l1p1, is_recip1, is_rsqrt1, is_silu1} =
        {rf1, rx1, rxf1, re1, ryh1, ra1, rp1, rg1, ro1, rl1, rr1, rq1, rs1};
      assign {f2, x2, e2, yh2, t2, s2, sp_above2, gate2, on_l1p2, is_recip2, is_rsqrt2, is_silu2} =
        {rf2, rx2, re2, ryh2, rt2, rs2, ra2, rg2, ro2, rr2, rq2, rsi2};
      assign {f3, x3, e3, yh3, t3, s3, r3, lu3, um13, q3, sp_above3, on_l1p3, is_silu3} =
        {rf3, rx3, re3, ryh3, rt3, rs3, rr3, rlu3, rum13, rq3, ra3, ro3, rsi3};
      assign y = ry;
    end else begin : at_once
      assign {f1, x1, xf1, e1, yh1, sp_above1, sp_past1, gate1, on_l1p1, is_l1p1, is_recip1, is_rsqrt1, is_silu1} =
        {f, x, xf, e, yh, sp_above, sp_past, gate, on_l1p, is_l1p, is_recip, is_rsqrt, is_silu};
      assign {f2, x2, e2, yh2, t2, s2, sp_above2, gate2, on_l1p2, is_recip2, is_rsqrt2, is_silu2} =
        {f1, x1, e1, yh1, t, s, sp_above1, gate1, on_l1p1, is_recip1, is_rsqrt1, is_silu1};
      assign {f3, x3, e3, yh3, t3, s3, r3, lu3, um13, q3, sp_above3, on_l1p3, is_silu3} =
        {f2, x2, e2, yh2, t2, s2, r, lu, um1, q, sp_above2, on_l1p2, is_silu2};
      assign y = pick;
    end
  endgenerate
endmodule

module logos_lane_bin(input [3:0] op, input [31:0] a, input [31:0] b, output [31:0] y);
  wire [31:0] m, s;
  logos_fmul um(.a(a), .b(b), .y(m));
  logos_fadd ua(.a(a), .b(op == 4'd2 ? {~b[31], b[30:0]} : b), .y(s));
  assign y = (op == 4'd0) ? m : s;
endmodule

// a > b as the maximum takes it: a NaN never larger.
module logos_fgt(input [31:0] a, input [31:0] b, output y);
  wire an = (a[30:23] == 8'hff) && (a[22:0] != 23'd0), bn = (b[30:23] == 8'hff) && (b[22:0] != 23'd0);
  wire lt;
  logos_flt l(.a(b), .b(a), .y(lt));
  assign y = !an && (bn || lt);
endmodule

// The int8 code of x under the inverse scale s, as an f32: x·s to the nearest integer, ties to even, within ±127, a
// NaN product zero, every code zero under an infinite s.
module logos_code8(input [31:0] x, input [31:0] s, output [31:0] y);
  wire [31:0] p;
  logos_fmul m(.a(x), .b(s), .y(p));
  logos_code8_of c(.p(p), .s(s), .y(y));
endmodule

// The int8 code of the product p = x·s under the inverse scale s.
module logos_code8_of(input [31:0] p, input [31:0] s, output [31:0] y);
  wire nan = (p[30:23] == 8'hff) && (p[22:0] != 23'd0);
  // |p| ≥ 128, infinities included, saturates; below it the nearest integer is within logos_rne's range.
  wire big = (p[30:23] >= 8'd134);
  wire signed [31:0] n;
  logos_rne r(.x(p), .n(n));
  wire signed [31:0] near = (n > 32'sd127) ? 32'sd127 : (n < -32'sd127) ? -32'sd127 : n;
  wire signed [31:0] code = nan ? 32'sd0 : big ? (p[31] ? -32'sd127 : 32'sd127) : near;
  wire inf_s = (s[30:23] == 8'hff) && (s[22:0] == 23'd0);
  logos_i2f c(.n(inf_s ? 32'sd0 : code), .y(y));
endmodule

module logos_simd #(parameter ROWS = 320, VREGS = 8, SREGS = 8, FU = 16)
  (input clk, input rst,
   input issue, input [63:0] instr, output busy,
   input wr, input [3:0] wr_reg, input [15:0] wr_row, input [511:0] wr_data, input len_wr, input [19:0] len_val,
   input [3:0] rd_reg, input [15:0] rd_row, output [511:0] rd_data, input [3:0] rd_s, output [31:0] rd_s_val);
  `LOGOS_RAMSTYLE_ASYNC(VREGS) reg [19:0] len [0:VREGS-1];
  `LOGOS_RAMSTYLE_ASYNC(SREGS) reg [31:0] sr [0:SREGS-1];
  assign rd_s_val = sr[rd_s];

  reg [63:0] ir;
  reg running;
  reg [15:0] row;
  reg [19:0] stride;
  reg folding;
  // A row's write steps: a Hadamard pair's low then high row, a rotary block's rows one after another.
  reg [1:0] phase;
  reg [511:0] acc;
  // Each lane's arg-max: whether it holds an element, and the element's index.
  reg [15:0] found;
  reg [19:0] aidx [0:15];
  assign busy = running;
  wire [3:0] opc = ir[3:0], sub = ir[7:4], ra = ir[11:8], rb = ir[15:12], rdd = ir[19:16], rs = ir[23:20];
  wire [19:0] n = len[ra];
  wire [19:0] rows_n = (n + 20'd15) >> 4;
  wire [15:0] rows = rows_n[15:0];
  wire largest = (opc == 4'd2) || (opc == 4'd11) || (opc == 4'd12);

  // The register file in block RAM, a bank a register, three copies a bank written alike: copy A read at the row at
  // hand (the engine's row while the unit is idle), copies B and C at a Hadamard stage's partner row and a rotary
  // block's two rows; one write a cycle into a bank, under a byte mask.
  localparam RA = (ROWS > 1) ? $clog2(ROWS) : 1;
  wire [512*VREGS-1:0] qa, qb, qc;
  reg [15:0] row_b, row_c;
  reg w_en;
  reg [3:0] w_reg;
  reg [15:0] w_row;
  reg [511:0] w_data;
  reg [63:0] w_mask;
  wire [15:0] row_a = running ? row : rd_row;
  genvar l, r, k2;
  generate
    for (r = 0; r < VREGS; r = r + 1) begin : bank
      wire we = w_en && w_reg == r;
      logos_ram_w512 #(.D(ROWS), .A(RA)) ca(.clk(clk), .ra(row_a[RA-1:0]), .rd(qa[512*r +: 512]), .we(we),
        .wa(w_row[RA-1:0]), .wd(w_data), .wm(w_mask));
      logos_ram_w512 #(.D(ROWS), .A(RA)) cb(.clk(clk), .ra(row_b[RA-1:0]), .rd(qb[512*r +: 512]), .we(we),
        .wa(w_row[RA-1:0]), .wd(w_data), .wm(w_mask));
      logos_ram_w512 #(.D(ROWS), .A(RA)) cc(.clk(clk), .ra(row_c[RA-1:0]), .rd(qc[512*r +: 512]), .we(we),
        .wa(w_row[RA-1:0]), .wd(w_data), .wm(w_mask));
    end
  endgenerate
  assign rd_data = qa[512*rd_reg +: 512];
  wire [511:0] va = qa[512*ra +: 512], vb = qa[512*rb +: 512];
  wire [31:0] sv = sr[rs];

  // A Hadamard stage reads d's row at hand and its partner row; a rotary step d's rows its pairs lie in and the turn's
  // tables. Strides below sixteen pair lanes of one row, strides of sixteen and up whole rows. A rotary step's lane l
  // turns pair 16·row + l of d by the tables' cosine and sine at the same place; the block's elements lie in d's row
  // at hand and the two rows copies B and C read: interleaved, rows 2·row and 2·row + 1; split, the rows the q
  // elements 16·row + l + pairs fall in.
  wire [511:0] hd_in = qa[512*rdd +: 512];
  wire [15:0] partner = row + stride[19:4];
  wire [511:0] hd_far = qb[512*rdd +: 512];
  wire near = stride < 20'd16;
  wire [19:0] pairs = len[rb];
  wire [19:0] blocks_n = (pairs + 20'd15) >> 4;
  wire [15:0] blocks = blocks_n[15:0];
  wire [511:0] rc = qa[512*rb +: 512], rsn = qa[512*rs +: 512];
  wire [19:0] q_first = {row, 4'd0} + pairs;
  wire [511:0] d_a = qa[512*rdd +: 512], d_b = qb[512*rdd +: 512], d_c = qc[512*rdd +: 512];
  wire rot = opc == 4'd10, had = opc == 4'd9, dot = opc == 4'd0, sum = opc == 4'd1, fma = opc == 4'd15;

  // The lanes of this row, each with one set of four multiplies and two adds that every instruction shares: a product
  // or a sum of the operation, a multiply-add's product and its sum, a dot's product and its running sum, the code's
  // product, a Hadamard pair's sum and difference, a rotary pair's four products, its difference and its sum.
  wire [511:0] bin_vv, bin_vs, fma_v, acc_next, max_next, codes_v, scale_v, hd_near, hd_lo, hd_hi, ry0, ry1;
  wire [16*20-1:0] rp, rq;
  wire [15:0] take, ron;
  generate
    for (l = 0; l < 16; l = l + 1) begin : lane
      wire live = ({row, 4'd0} + l) < n;
      wire [31:0] x = va[32*l +: 32];
      wire [31:0] y = vb[32*l +: 32];
      // Lane l's Hadamard pair: lane l xor the stride, the lower of the two taking the sum; or the partner row's lane.
      wire [3:0] h = stride[3:0];
      wire low = ((l & h) == 0);
      wire [31:0] mine = hd_in[32*l +: 32];
      wire [31:0] other = (h == 4'd1) ? hd_in[32*(l ^ 1) +: 32] : (h == 4'd2) ? hd_in[32*(l ^ 2) +: 32]
                        : (h == 4'd4) ? hd_in[32*(l ^ 4) +: 32] : hd_in[32*(l ^ 8) +: 32];
      wire [31:0] far = hd_far[32*l +: 32];
      // Lane l's rotary pair.
      wire [19:0] i = {row, 4'd0} + l;
      assign ron[l] = i < pairs;
      wire [19:0] p = sub[0] ? i : {i[18:0], 1'b0};
      wire [19:0] q = sub[0] ? i + pairs : {i[18:0], 1'b1};
      assign rp[20*l +: 20] = p;
      assign rq[20*l +: 20] = q;
      wire [511:0] wp = (p[19:4] == row_b) ? d_b : (p[19:4] == row_c) ? d_c : d_a;
      wire [511:0] wq = (q[19:4] == row_b) ? d_b : (q[19:4] == row_c) ? d_c : d_a;
      wire [31:0] x0 = wp[32*p[3:0] +: 32], x1 = wq[32*q[3:0] +: 32];
      wire [31:0] c = rc[32*l +: 32], s = rsn[32*l +: 32];
      // The shared units, their operands by instruction; a unit no instruction at hand uses holds its operands at zero.
      wire [31:0] m0, m1, m2, m3, a0, a1;
      wire [31:0] by = (opc == 4'd3 || fma || dot) ? y : (opc == 4'd4 || opc == 4'd13) ? sv : 32'h3f80_0000;
      logos_fmul um0(.a(rot ? x0 : x), .b(rot ? c : by), .y(m0));
      logos_fmul um1(.a(rot ? x1 : 32'd0), .b(rot ? s : 32'd0), .y(m1));
      logos_fmul um2(.a(rot ? x0 : 32'd0), .b(rot ? s : 32'd0), .y(m2));
      logos_fmul um3(.a(rot ? x1 : 32'd0), .b(rot ? c : 32'd0), .y(m3));
      wire [31:0] bin_b = (opc == 4'd3) ? y : sv;
      wire [31:0] nb = (sub == 4'd2) ? {~bin_b[31], bin_b[30:0]} : bin_b;
      logos_fadd ua0(.a(rot ? m2 : fma ? m0 : had ? (near ? (low ? mine : other) : mine) : (dot || sum) ? acc[32*l +: 32] : x),
        .b(rot ? m3 : fma ? (sub[0] ? sv : s) : had ? (near ? (low ? other : mine) : far) : dot ? m0 : sum ? x : nb), .y(a0));
      logos_fadd ua1(.a(rot ? m0 : !had ? 32'd0 : near ? other : mine),
        .b(rot ? {~m1[31], m1[30:0]} : !had ? 32'd0 : near ? {~mine[31], mine[30:0]} : {~far[31], far[30:0]}), .y(a1));
      assign bin_vv[32*l +: 32] = (sub == 4'd0) ? m0 : a0;
      assign bin_vs[32*l +: 32] = (sub == 4'd0) ? m0 : a0;
      assign fma_v[32*l +: 32] = a0;
      assign acc_next[32*l +: 32] = live ? a0 : acc[32*l +: 32];
      assign hd_near[32*l +: 32] = low ? a0 : a1;
      assign hd_lo[32*l +: 32] = a0;
      assign hd_hi[32*l +: 32] = a1;
      assign ry0[32*l +: 32] = a1;
      assign ry1[32*l +: 32] = a0;
      logos_code8_of cd(.p(m0), .s(sv), .y(codes_v[32*l +: 32]));
      logos_ldexp sc(.x(x), .k(ir[47:32]), .y(scale_v[32*l +: 32]));
      // The magnitude AbsMax reads: a denormal a zero, the sign dropped.
      wire [31:0] cand = (opc == 4'd12) ? ((x[30:23] == 8'd0) ? 32'd0 : {1'b0, x[30:0]}) : x;
      wire nan = (x[30:23] == 8'hff) && (x[22:0] != 23'd0);
      wire gt;
      logos_fgt g(.a(cand), .b(acc[32*l +: 32]), .y(gt));
      assign take[l] = (opc == 4'd11) ? (live && !nan && (!found[l] || gt)) : (live && gt);
      assign max_next[32*l +: 32] = take[l] ? cand : acc[32*l +: 32];
    end
  endgenerate
  // The function units, each four stages a step apart (logos_lane_func_s): a Map's rows a slice of FU lanes a step into
  // them, one row's after another's with no gap, each slice's values out four steps later in the order they went in,
  // held in fbuf until its row's last slice is out and the row is written whole; a ScalarMap's operand on the first
  // unit. A unit computes only while an instruction that uses it issues to it, its function otherwise the one that
  // holds its cores' operands at zero.
  localparam [4:0] SLICES = 16 / FU;
  localparam STAGES = 4;
  reg [3:0] slice;
  reg issuing;
  reg [511:0] fbuf;
  // A slice in each stage, and its row and place: the last stage's the values out this step.
  reg [STAGES-1:0] fv;
  reg [20*STAGES-1:0] ft;
  wire f_out = fv[STAGES-1];
  wire [3:0] f_slice = ft[20*(STAGES-1) +: 4];
  wire [15:0] f_row = ft[20*(STAGES-1) + 4 +: 16];
  wire f_issue = running && issuing && (opc == 4'd5 || opc == 4'd7);
  wire [32*FU-1:0] fx, fy;
  generate
    for (l = 0; l < FU; l = l + 1) begin : fun
      wire [3:0] fcode = (f_issue && (opc == 4'd5 || l == 0)) ? sub : 4'd15;
      // The unit's lane of each slice, each a fixed part of the row; the slice at hand picks one.
      wire [32*SLICES-1:0] cand;
      for (k2 = 0; k2 < SLICES; k2 = k2 + 1) begin : at
        assign cand[32*k2 +: 32] = va[32*(FU*k2 + l) +: 32];
      end
      wire [31:0] lane_x = (SLICES == 1) ? cand[31:0] : cand[32*slice +: 32];
      assign fx[32*l +: 32] = (opc == 4'd7 && l == 0) ? sr[ra] : lane_x;
      logos_lane_func #(.STAGED(1)) fu(.clk(clk), .rst(rst), .f(fcode), .x(fx[32*l +: 32]), .y(fy[32*l +: 32]));
    end
  endgenerate
  // The row with the slice out this step in place: lane l from unit l mod FU when its slice is out.
  wire [511:0] func_v;
  generate
    for (l = 0; l < 16; l = l + 1) begin : fsl
      localparam [3:0] MINE = l / FU;
      assign func_v[32*l +: 32] = (f_out && f_slice == MINE) ? fy[32*(l % FU) +: 32] : fbuf[32*l +: 32];
    end
  endgenerate
  wire last_slice = f_out && {1'b0, f_slice} + 5'd1 >= SLICES;

  // Whether this row is the low half of its stride's pair.
  wire row_low = (stride >= 20'd16) ? ((row & stride[19:4]) == 16'd0) : 1'b1;

  // The lanes folded by the pairwise tree, a sum or a maximum.
  wire [31:0] tree_sum;
  logos_tree16 ts(.lanes(acc), .sum(tree_sum));
  reg [31:0] tmax;
  reg [31:0] mlv [0:15];
  integer t, w;
  reg big;
  always @* begin
    for (t = 0; t < 16; t = t + 1) mlv[t] = acc[32*t +: 32];
    for (w = 1; w < 16; w = w * 2)
      for (t = 0; t < 16; t = t + 2*w) begin
        big = 1'b0;
        if (!((mlv[t+w][30:23] == 8'hff) && (mlv[t+w][22:0] != 23'd0))) begin
          if (mlv[t][31] == 1'b0 && mlv[t+w][31] == 1'b0) big = mlv[t+w][30:0] > mlv[t][30:0];
          else if (mlv[t][31] == 1'b1 && mlv[t+w][31] == 1'b1) big = mlv[t+w][30:0] < mlv[t][30:0];
          else big = (mlv[t+w][31] == 1'b0) && !(mlv[t+w][30:0] == 31'd0 && mlv[t][30:0] == 31'd0);
        end
        if (big) mlv[t] = mlv[t+w];
      end
    tmax = (mlv[0][30:0] == 31'd0) ? 32'd0 : mlv[0];
  end

  // The lanes' arg-maxes folded: the one holding the larger value, of equal values the lower index. The values a lane
  // holds are never NaN, so they order by the key that makes the two zeros one.
  function [31:0] fkey(input [31:0] v);
    fkey = (v[30:0] == 31'd0) ? 32'h8000_0000 : (v[31] ? ~v : {1'b1, v[30:0]});
  endfunction
  reg [31:0] av [0:15];
  reg [19:0] ai [0:15];
  reg af [0:15];
  reg beat;
  always @* begin
    for (t = 0; t < 16; t = t + 1) begin av[t] = acc[32*t +: 32]; ai[t] = aidx[t]; af[t] = found[t]; end
    for (w = 1; w < 16; w = w * 2)
      for (t = 0; t < 16; t = t + 2*w) begin
        beat = af[t+w] && (!af[t] || (fkey(av[t+w]) > fkey(av[t]))
                                  || (fkey(av[t+w]) == fkey(av[t]) && ai[t+w] < ai[t]));
        if (beat) begin av[t] = av[t+w]; ai[t] = ai[t+w]; af[t] = 1'b1; end
      end
  end
  wire [31:0] arg_f;
  logos_i2f ia(.n(af[0] ? {12'd0, ai[0]} : 32'sd0), .y(arg_f));

  // Scalar operations.
  wire [31:0] s_bin;
  wire [31:0] s_func = fy[31:0];
  logos_lane_bin sb(.op(sub), .a(sr[ra]), .b(sr[rb]), .y(s_bin));

  // The block's turned elements as three masked rows, each element in the row it was read from.
  reg [511:0] rec_a, rec_b, rec_c;
  reg [63:0] rec_am, rec_bm, rec_cm;
  integer k;
  always @* begin
    rec_a = d_a; rec_b = d_b; rec_c = d_c; rec_am = 64'd0; rec_bm = 64'd0; rec_cm = 64'd0;
    for (k = 0; k < 16; k = k + 1)
      if (ron[k]) begin
        if (rp[20*k+4 +: 16] == row_b) begin rec_b[32*rp[20*k +: 4] +: 32] = ry0[32*k +: 32]; rec_bm[4*rp[20*k +: 4] +: 4] = 4'hf; end
        else if (rp[20*k+4 +: 16] == row_c) begin rec_c[32*rp[20*k +: 4] +: 32] = ry0[32*k +: 32]; rec_cm[4*rp[20*k +: 4] +: 4] = 4'hf; end
        else begin rec_a[32*rp[20*k +: 4] +: 32] = ry0[32*k +: 32]; rec_am[4*rp[20*k +: 4] +: 4] = 4'hf; end
        if (rq[20*k+4 +: 16] == row_b) begin rec_b[32*rq[20*k +: 4] +: 32] = ry1[32*k +: 32]; rec_bm[4*rq[20*k +: 4] +: 4] = 4'hf; end
        else if (rq[20*k+4 +: 16] == row_c) begin rec_c[32*rq[20*k +: 4] +: 32] = ry1[32*k +: 32]; rec_cm[4*rq[20*k +: 4] +: 4] = 4'hf; end
        else begin rec_a[32*rq[20*k +: 4] +: 32] = ry1[32*k +: 32]; rec_am[4*rq[20*k +: 4] +: 4] = 4'hf; end
      end
  end
  always @* begin
    row_b = 16'd0; row_c = 16'd0;
    if (opc == 4'd9) row_b = partner;
    else if (opc == 4'd10) begin
      if (sub[0]) begin row_b = q_first[19:4]; row_c = q_first[19:4] + 16'd1; end
      else begin row_b = {row[14:0], 1'b0}; row_c = {row[14:0], 1'b1}; end
    end
  end

  // The rows a step writes after its first: a Hadamard pair's high row, a rotary block's rows B and C.
  reg [511:0] hold_d1, hold_d2;
  reg [63:0] hold_m1, hold_m2;
  reg [15:0] hold_r1, hold_r2;
  // The one write a cycle: the engine's beat, or the row the step at hand writes.
  always @* begin
    w_en = 1'b0; w_reg = rdd; w_row = row; w_data = 512'd0; w_mask = {64{1'b1}};
    if (wr) begin
      w_en = 1'b1; w_reg = wr_reg; w_row = wr_row; w_data = wr_data;
    end else if (running && !rst) case (opc)
      4'd3, 4'd4, 4'd5, 4'd13, 4'd14, 4'd15: begin
        w_en = (opc != 4'd5) || last_slice;
        if (opc == 4'd5) w_row = f_row;
        w_data = (opc == 4'd3) ? bin_vv : (opc == 4'd4) ? bin_vs : (opc == 4'd5) ? func_v : (opc == 4'd13) ? codes_v
               : (opc == 4'd14) ? scale_v : fma_v;
      end
      4'd9:
        if (!folding) begin w_en = 1'b1; w_data = va; end
        else if (stride < 20'd16) begin w_en = 1'b1; w_data = hd_near; end
        else if (phase == 2'd0) begin w_en = row_low; w_data = hd_lo; end
        else begin w_en = 1'b1; w_row = hold_r1; w_data = hold_d1; end
      4'd10:
        if (!folding) begin w_en = 1'b1; w_data = va; end
        else case (phase)
          2'd0: begin w_en = 1'b1; w_data = rec_a; w_mask = rec_am; end
          2'd1: begin w_en = 1'b1; w_row = hold_r1; w_data = hold_d1; w_mask = hold_m1; end
          default: begin w_en = 1'b1; w_row = hold_r2; w_data = hold_d2; w_mask = hold_m2; end
        endcase
      default: ;
    endcase
  end

  always @(posedge clk) begin
    if (rst) begin
      running <= 1'b0; slice <= 4'd0; issuing <= 1'b0; fv <= {STAGES{1'b0}};
    end else begin
      fv <= {fv[STAGES-2:0], f_issue};
      ft <= {ft[20*(STAGES-1)-1:0], row, slice};
      if (len_wr) len[wr_reg] <= len_val;
      if (issue && !running) begin
        ir <= instr; running <= 1'b1; row <= 16'd0; folding <= 1'b0; stride <= 20'd1; found <= 16'd0; phase <= 2'd0;
        slice <= 4'd0; issuing <= 1'b1;
        if (instr[3:0] == 4'd2 || instr[3:0] == 4'd11) for (k = 0; k < 16; k = k + 1) acc[32*k +: 32] <= 32'hff80_0000;
        else acc <= 512'd0;
      end else if (running) begin
        case (opc)
          4'd0, 4'd1, 4'd2, 4'd11, 4'd12: begin
            if (folding) begin
              sr[rdd] <= (opc == 4'd11) ? arg_f : largest ? tmax : tree_sum;
              running <= 1'b0;
            end else begin
              acc <= largest ? max_next : acc_next;
              if (opc == 4'd11)
                for (k = 0; k < 16; k = k + 1)
                  if (take[k]) begin aidx[k] <= {row, 4'd0} + k[19:0]; found[k] <= 1'b1; end
              if (row + 16'd1 >= rows) folding <= 1'b1; else row <= row + 16'd1;
            end
          end
          4'd3, 4'd4, 4'd13, 4'd14, 4'd15: begin
            len[rdd] <= n;
            if (row + 16'd1 >= rows) running <= 1'b0; else row <= row + 16'd1;
          end
          4'd5: begin
            if (issuing) begin
              if ({1'b0, slice} + 5'd1 >= SLICES) begin
                slice <= 4'd0;
                if (row + 16'd1 >= rows) issuing <= 1'b0; else row <= row + 16'd1;
              end else slice <= slice + 4'd1;
            end
            if (f_out) begin
              if (!last_slice) fbuf <= func_v;
              else begin
                len[rdd] <= n;
                if (f_row + 16'd1 >= rows) running <= 1'b0;
              end
            end
          end
          4'd6: begin sr[rdd] <= s_bin; running <= 1'b0; end
          4'd7: begin issuing <= 1'b0; if (f_out) begin sr[rdd] <= s_func; running <= 1'b0; end end
          4'd8: begin sr[rdd] <= ir[63:32]; running <= 1'b0; end
          4'd9: begin
            // The first pass copies a into d; then each stage walks d's rows, a pair of rows two writes.
            if (!folding) begin
              len[rdd] <= n;
              if (row + 16'd1 >= rows) begin
                row <= 16'd0;
                if (n <= 20'd1) running <= 1'b0; else folding <= 1'b1;
              end else row <= row + 16'd1;
            end else if (stride >= 20'd16 && phase == 2'd0 && row_low) begin
              phase <= 2'd1; hold_r1 <= partner; hold_d1 <= hd_hi;
            end else begin
              phase <= 2'd0;
              if (row + 16'd1 >= rows) begin
                row <= 16'd0;
                if ((stride << 1) >= len[rdd]) running <= 1'b0;
                stride <= stride << 1;
              end else row <= row + 16'd1;
            end
          end
          4'd10: begin
            // a copied into d; then each block of sixteen pairs turned from one read, its rows written in turn.
            if (!folding) begin
              len[rdd] <= n;
              if (row + 16'd1 >= rows) begin
                row <= 16'd0;
                if (pairs == 20'd0) running <= 1'b0; else folding <= 1'b1;
              end else row <= row + 16'd1;
            end else if (phase == 2'd0) begin
              phase <= 2'd1;
              hold_r1 <= row_b; hold_d1 <= rec_b; hold_m1 <= rec_bm;
              hold_r2 <= row_c; hold_d2 <= rec_c; hold_m2 <= rec_cm;
            end else if (phase == 2'd1) phase <= 2'd2;
            else begin
              phase <= 2'd0;
              if (row + 16'd1 >= blocks) running <= 1'b0; else row <= row + 16'd1;
            end
          end
          default: running <= 1'b0;
        endcase
      end
    end
  end
endmodule
"#;
