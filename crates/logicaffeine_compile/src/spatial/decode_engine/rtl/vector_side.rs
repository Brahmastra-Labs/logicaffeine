//! A hybrid engine's vector jobs in the user clock ([`super::engine::Streaming::vectors`]): `logos_vector_side` runs a
//! [`crate::spatial::decode_engine::machine::Job::Vector`] job as the core's vector-job states do — the job's
//! descriptors from the job words, then each pass of a row and a head: the loads into the vector unit's registers, the
//! micro-program on the user clock's vector unit (`logos_simd_p`, [`super::simd::simd_p_verilog`]), the stores — on its
//! own copies of the core's tables, written as the core's are, the token-row store's user-clock port and the fast port
//! onto the channels ([`super::super::physical::fast_port_verilog`]).
//!
//! A load or a store into the token-row store streams a beat a clock. A load from the memory asks for a line whenever
//! the port takes one and writes each line into its register as it comes back, in order; a ring row stored in binary16
//! is widened half a line a beat. A store into the memory reads a beat a clock while the queue its beats wait in has room
//! for every beat on its way, each beat's binary16 halves and int8 codes out of pipelined units; it gathers its beats in
//! one line until a beat leaves the line or the pass ends, taking a beat a clock: a whole line goes to the write queue
//! the clock the queue is free, a part of one is read once every write before it is on the card, merged and written. A ring load
//! whose row's sequence stored a ring row earlier in the job waits until every write before it is on the card, so a
//! sequence's next row reads the ring row its last row wrote; no other load reads what a vector job writes (another
//! sequence's rows touch other rings, tensors and rotary tables are never written), so none waits. A kept load ([`crate::spatial::decode_engine::machine::VectorJob::kept`]: a
//! tensor's span at stride 0 into a register nothing else writes) is read on the job's first pass alone. A job whose rows
//! cross nowhere ([`crate::spatial::decode_engine::machine::VectorJob::chunked`]) and is deeper than the unit runs its
//! passes once for each chunk of `VROWS` beats of its vectors, every load and store that chunk's beats, its kept loads
//! read on each chunk's first pass; so the unit is as deep as the deepest job that reduces, rotates or turns, not the
//! deepest elementwise one. The products
//! of an element's place are split into sixteen-bit halves, each sum truncated to the width the core's expression has,
//! so every place is the core's.
//!
//! A side of `PASSES` above one ([`super::engine::Streaming::vector_passes`]) runs a job whose passes stand alone
//! ([`crate::spatial::decode_engine::machine::VectorJob::grouped`], its header's bit 54) in groups: as many passes as
//! the unit holds rows for, up to `PASSES`, loaded one after another, each into rows of its own; the micro-program
//! issued once over the group, each instruction taking the group's passes side by side (`logos_simd_p`'s `group`), each
//! pass's operations in their own order; then each pass's stores from its rows and scalars, the line they gather in
//! flushed after each pass as between passes. A pass's values are the ones it computes alone. The unit holds `DEPTH`
//! times `VROWS` rows a register ([`super::engine::Streaming::vector_depth`]), so a job whose vectors are `VROWS` beats
//! deep runs `DEPTH` passes a group; its chunks and its float matrices' held inputs stay `VROWS` beats a vector.
//!
//! A float matrix's job (a tensor's row a head dotted with each row's input, one scalar a pass) runs on a path of its
//! own: every row's input held, `FM_ROWS` rows to a group of `16·FM_ROWS` lanes and the rows past them in further
//! groups the same lanes take in turn; eight heads' weight rows streamed a line of each in turn, each line given to the
//! lanes once for each group, every row's products added in `numerics::fdot`'s order, every row's head through the tree
//! into its element. The lines come back into a queue and leave it for the lanes once their head's last sum of the group
//! has left the add, so a group of fewer heads than the add is deep, or lines bunched by the port, still add each head's
//! terms one after another. The weights stream once for every row the side holds.

/// The rows of a float matrix's job a clock of its lanes takes: its lanes `16·FM_ROWS`.
pub const FM_ROWS: usize = 4;

/// Line words a memory store's queue holds (`q_data`): every beat of the stream in flight, and the one being merged.
const MS_DEPTH: usize = 16;

/// The vector side with the vector unit, the block RAM and the functions it is built from.
pub fn vector_side_verilog() -> String {
    use super::fp32::{F32_TO_F16_LATENCY, FADD_LATENCY, FMUL_LATENCY, TREE16_LATENCY};
    use super::simd::SIMD_P_READ_LATENCY;
    use super::vector::RNE_P_LATENCY;
    let narrow = F32_TO_F16_LATENCY.max(RNE_P_LATENCY);
    assert!(narrow >= 2, "a memory store's fields ride a delay line of the narrowing units' latency, two clocks or more");
    assert!(FMUL_LATENCY >= 3, "a float matrix's sums are read the clock before its add takes them, their address registered a clock before that, inside the multiply");
    // A memory store streams a beat a clock while its queue holds a beat being merged beside every stage of the stream.
    assert!(SIMD_P_READ_LATENCY >= 2, "a store's beat waits in a shift of its unit's read latency, two clocks or more");
    assert!(SIMD_P_READ_LATENCY + 1 + 1 + narrow + 1 < MS_DEPTH, "a memory store's queue has room for its stream at a beat a clock");
    let side = [
        ("@FMUL@", FMUL_LATENCY),
        ("@FADD@", FADD_LATENCY),
        ("@SPACE@", FADD_LATENCY + 1),
        ("@TREE@", TREE16_LATENCY),
        ("@NARROW_LESS@", narrow - 1),
        ("@CODES_PAD@", narrow - RNE_P_LATENCY),
        ("@NARROW@", narrow),
        ("@FM_ROWS@", FM_ROWS),
        ("@READ@", SIMD_P_READ_LATENCY),
        ("@MS_OUT@", SIMD_P_READ_LATENCY + 1),
        ("@MS_DEPTH@", MS_DEPTH),
    ]
        .iter()
        .fold(VECTOR_SIDE_VERILOG.to_string(), |t, (k, v)| t.replace(k, &v.to_string()));
    format!("{}\n{}\n{side}", super::ram::bram_verilog(&[64, 128, 512]), super::simd::simd_p_verilog())
}

const VECTOR_SIDE_VERILOG: &str = r#"// A hybrid engine's vector jobs in the user clock: the core hands a vector job over by its header's place in the job
// words (`start`, `job_pc`) with the step's rows and its state's format, and the side runs it as the core's vector-job
// states do — its descriptors read, then each pass of a row and a head: the loads into the vector unit's registers,
// the micro-program, the stores — on its own copies of the core's tables (written as the core's are, at the core's
// step), the token-row store's user-clock port and the fast port onto the channels. A pass's store beats stream a beat
// a clock each way; its memory lines go out as the port takes them and come back in order. Its memory stores gather
// in a line until the next store leaves the line: a whole line written as it is, a part of one read, merged and written
// once every write before it is on the card (`written`); a ring load waits for the writes before it to be on the card
// when its row's sequence stored a ring row earlier in the job, and no other load does. `busy` holds from `start` until the job's last write is on the card; a line the port faults on faults the
// job. A job whose passes stand alone (its header's bit 54) runs in groups of up to PASSES passes, loaded each into rows of
// its own, run as one, stored one after another.
module logos_vector_side #(parameter ROW_MAX = 1, VROWS = 64, PROWS = VROWS, FU = 16, JOB_WORDS = 64, TENSORS = 1,
                           LAYERS = 1, ROPES = 1, SA = 10, MAXL = 16, MAXP = 16, MAXI = 64, FM_ROWS = @FM_ROWS@,
                           PASSES = 1, DEPTH = 1, SRL = 1)
  (input clk, input rst,
   input start, input [31:0] job_pc, input [15:0] rows, input state_f16, output busy, input port_fault, output reg fault,
   input host_we, input [2:0] host_table, input [31:0] host_addr, input [511:0] host_wdata,
   output reg [SA-1:0] st_ra, input [511:0] st_rd, output reg st_we, output reg [SA-1:0] st_wa, output reg [511:0] st_wd,
   output reg [63:0] st_wm,
   output reg rq_valid, input rq_ready, output reg [31:0] rq_line, input rs_valid, input [511:0] rs_data,
   output reg wq_valid, input wq_ready, output reg [31:0] wq_line, output reg [511:0] wq_data, input written);
  localparam JA = (JOB_WORDS > 1) ? $clog2(JOB_WORDS) : 1, TA = (TENSORS > 1) ? $clog2(TENSORS) : 1,
             BA = (LAYERS > 1) ? $clog2(LAYERS) : 1, VA = (VROWS > 1) ? $clog2(VROWS) : 1;

  // The tables, as the host writes the core's.
  `LOGOS_RAMSTYLE_ASYNC(ROW_MAX) reg [127:0] rowt [0:ROW_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(32) reg [63:0] slott [0:31];
  reg [63:0] ropet [0:ROPES-1];
  reg [JA-1:0] j_ra;
  reg [TA-1:0] t_ra;
  reg [BA-1:0] b_ra;
  wire [63:0] jw;
  wire [127:0] tw;
  wire [511:0] bw;
  logos_bram_w64 #(.D(JOB_WORDS), .A(JA)) jobs_ram(.clk(clk), .ra(j_ra), .rd(jw),
    .we(host_we && host_table == 3'd0), .wa(host_addr[JA-1:0]), .wd(host_wdata[63:0]), .wm(8'hff));
  logos_bram_w128 #(.D(TENSORS), .A(TA)) tens_ram(.clk(clk), .ra(t_ra), .rd(tw),
    .we(host_we && host_table == 3'd3), .wa(host_addr[TA-1:0]), .wd(host_wdata[127:0]), .wm(16'hffff));
  logos_bram_w512 #(.D(LAYERS), .A(BA)) block_ram(.clk(clk), .ra(b_ra), .rd(bw),
    .we(host_we && host_table == 3'd5), .wa(host_addr[BA-1:0]), .wd(host_wdata), .wm({64{1'b1}}));
  always @(posedge clk)
    if (host_we) case (host_table)
      3'd1: rowt[host_addr] <= host_wdata[127:0];
      3'd2: slott[host_addr[4:0]] <= host_wdata[63:0];
      3'd4: ropet[host_addr] <= host_wdata[63:0];
      default: ;
    endcase

  // A job whose passes stand alone (its header's bit 54, machine::VectorJob::grouped) runs in groups when the side runs
  // more than one pass a group: its vectors' beats a pass (grows), the most passes whose rows the unit holds (gmax), the
  // pass at hand in the group (gi, its rows from goff) and the group's passes (gn), and the group's first pass (gk0,
  // gh0), its stores' place. A job of one pass a group has gi and goff zero. gmax is a clock behind grows, which is set
  // as the descriptors end, clocks before any pass's loads end.
  reg [15:0] grows, goff, prow;
  reg [7:0] gi, gn;
  reg [15:0] gk0;
  reg [31:0] gh0;
  function [7:0] gfit(input [15:0] rows_a_pass);
    integer q;
    begin
      gfit = 8'd1;
      for (q = 2; q <= PASSES; q = q + 1) if (q * rows_a_pass <= $unsigned(VROWS * DEPTH)) gfit = q[7:0];
    end
  endfunction
  reg [7:0] gmax;

  // The vector unit.
  reg s_issue, s_wr, s_len_wr;
  reg [63:0] s_instr;
  reg [3:0] s_wr_reg, s_rd_reg, s_rd_s;
  reg [15:0] s_wr_row, s_rd_row;
  reg [511:0] s_wr_data;
  reg [19:0] s_len_val;
  wire s_busy;
  wire [511:0] s_rd_data;
  wire [31:0] s_rd_s_val;
  reg [7:0] s_group;
  logos_simd_p #(.ROWS(VROWS * DEPTH), .VREGS(16), .SREGS(8), .FU(FU), .PAIRS(PROWS), .PASSES(PASSES)) vu(.clk(clk), .rst(rst),
    .issue(s_issue), .instr(s_instr), .busy(s_busy), .group(s_group), .group_rows(grows), .wr(s_wr), .wr_reg(s_wr_reg),
    .wr_row(s_wr_row), .wr_data(s_wr_data), .len_wr(s_len_wr), .len_val(s_len_val), .rd_reg(s_rd_reg), .rd_row(s_rd_row),
    .rd_data(s_rd_data), .rd_s(s_rd_s), .rd_s_pass(gi), .rd_s_val(s_rd_s_val));

  localparam [5:0] IDLE = 6'd0, HDR0 = 6'd1, HDR = 6'd2, HEADS0 = 6'd3, HEADS = 6'd4, DESC = 6'd5, PASS = 6'd6,
                   LOAD = 6'd7, LB1 = 6'd8, LB2 = 6'd9, LB3 = 6'd10, LB4 = 6'd11, LBEAT = 6'd12, LLEN = 6'd13, RUN = 6'd14,
                   RWAIT = 6'd15, PUT = 6'd16, PB1 = 6'd17, PB2 = 6'd18, PB3 = 6'd19, PB4 = 6'd20, PBEAT = 6'd21,
                   MS = 6'd22, NEXT = 6'd26, DRAIN = 6'd27, PB5 = 6'd28,
                   LB5 = 6'd30, PB4B = 6'd31, PBF = 6'd32, FM0 = 6'd34, FM_ACT = 6'd35,
                   FM_W = 6'd36, FM_TREE = 6'd37, FM_NEXT = 6'd38, FM_T = 6'd39, GNEXT = 6'd40;
  reg [5:0] state;
  assign busy = state != IDLE;

  // The job: its words' place, the step's rows and state format, its header, its heads, its descriptors.
  reg [31:0] pc;
  reg [15:0] nrows;
  (* maxfan = 256 *) reg f16;
  reg [63:0] hdr;
  wire heads_first = hdr[4];
  wire [7:0] nfresh = hdr[20:13], nload = hdr[28:21], nput = hdr[36:29];
  wire [15:0] nprog = hdr[52:37];
  // A job that runs a chunk of its vectors at a time (its header's bit 53): its passes over every row once a chunk, the
  // chunk VROWS beats of every vector it loads and stores from beat cbase, the vectors jbeats beats long.
  wire chunked = hdr[53];
  wire grouped = PASSES > 1 && hdr[54] && !chunked;
  localparam [31:0] VB = VROWS;
  reg [31:0] cbase, jbeats;
  reg [31:0] heads;
  `LOGOS_RAMSTYLE_ASYNC(MAXL) reg [63:0] ld_a [0:MAXL-1];
  `LOGOS_RAMSTYLE_ASYNC(MAXL) reg [63:0] ld_b [0:MAXL-1];
  `LOGOS_RAMSTYLE_ASYNC(MAXP) reg [63:0] pt_a [0:MAXP-1];
  `LOGOS_RAMSTYLE_ASYNC(MAXP) reg [63:0] pt_b [0:MAXP-1];
  reg [63:0] prog [0:MAXI-1];
  reg [15:0] dp, dt;
  (* maxfan = 256 *) reg dv;
  // A store load's beat asked (lbv, the clock after its address) and out of the store SRL clocks after its address (lbo).
  reg lbv;
  wire lbo, lb_flight, lb_none;
  logos_landing #(.W(1), .D(SRL - 1)) lland(.clk(clk), .rst(rst), .in_v(lbv), .in_d(1'b0), .out_v(lbo), .out_d(lb_none),
    .flight(lb_flight));
  wire [15:0] nwords = {7'd0, nload, 1'b0} + {7'd0, nput, 1'b0} + nprog;
  reg [15:0] k;
  reg [31:0] h;
  // The pass after (k, h) as NEXT takes it, and whether it is past the job's last: a clock behind (k, h), which every
  // reader is at least two clocks after.
  reg [15:0] nk;
  reg [31:0] nh;
  reg nend;
  reg [7:0] li, pi;
  reg [15:0] ii;
  wire [31:0] st_bytes = f16 ? 32'd2 : 32'd4;

  // The load at hand and its place: a store beat, or a line of the memory (and for a binary16 ring row the half of it).
  (* maxfan = 256 *) reg [63:0] la;
  reg [63:0] lbw;
  wire [3:0] l_reg = la[3:0], l_kind = la[7:4];
  wire [15:0] l_what = la[23:8];
  wire [7:0] l_extra = la[31:24];
  wire [31:0] l_len = la[63:32];
  // The load's elements in the chunk at hand: past the chunk's first, as many as a chunk holds; a register a clock behind
  // the entry and the chunk's base, its beats counted once its places are made, five states or more after the entry is read.
  wire [31:0] l_left = l_len - (cbase << 4);
  reg [31:0] l_chunk;
  always @(posedge clk) l_chunk <= (l_left > (VB << 4)) ? (VB << 4) : l_left;
  wire [31:0] l_beats = (l_chunk + 32'd15) >> 4;
  reg [127:0] row_e;
  reg [63:0] slot_e, rope_e;
  reg [47:0] elem_lo, elem_hi, rp_lo, rp_hi;
  reg [31:0] elem, kprod, rprod, ring_sum;
  reg [63:0] ring_prod;
  reg [31:0] lbase;
  reg [63:0] lbyte;
  reg [31:0] lp, lt, lr;
  // A load's beat's line: its tensor's or rotary row's line plus the beat, or its ring row's byte's line.
  function [31:0] l_line(input [31:0] i);
    reg [63:0] byte_at;
    begin
      byte_at = lbyte + {32'd0, i} * (f16 ? 64'd32 : 64'd64);
      l_line = (l_kind == 4'd1 || l_kind == 4'd2) ? lbase + i : byte_at[37:6];
    end
  endfunction
  wire [63:0] l_byte_r = lbyte + {32'd0, lr} * (f16 ? 64'd32 : 64'd64);
  wire [31:0] lp_next = lp + ((rq_valid && rq_ready) ? 32'd1 : 32'd0);
  reg [511:0] rs_r;
  reg rs_v;
  (* maxfan = 256 *) reg rs_half;
  wire [511:0] widened;
  genvar g;
  generate
    for (g = 0; g < 16; g = g + 1) begin : widen
      logos_f16_to_f32 wv(.h(rs_half ? rs_r[256 + 16*g +: 16] : rs_r[16*g +: 16]), .x(widened[32*g +: 32]));
    end
  endgenerate
  wire from_f16 = (l_kind == 4'd3) && f16;

  // The store at hand: its register, its place in the store, or its line and byte in the memory and what it writes.
  reg [63:0] pa, pbw;
  wire [3:0] p_reg = pa[3:0];
  wire p_scalar = pa[4];
  wire [2:0] p_kind = pa[7:5];
  wire [15:0] p_what = pa[23:8];
  wire p_values = pa[24];
  wire [1:0] p_part = pa[26:25];
  wire [31:0] p_len = pa[63:32];
  // The store's elements in the chunk at hand, a register a clock behind the entry and the chunk's base: its beats are
  // counted from it once its place is made, five states or more after the entry is read, and both hold through the store.
  wire [31:0] p_left = p_len - (cbase << 4);
  reg [31:0] p_chunk;
  always @(posedge clk) p_chunk <= (p_left > (VB << 4)) ? (VB << 4) : p_left;
  wire [31:0] p_beats = p_scalar ? 32'd1 : (p_chunk + 32'd15) >> 4;
  reg [47:0] pelem_lo, pelem_hi, cached_lo, cached_hi;
  reg [63:0] cprod_lo, cprod_hi;
  reg [31:0] pelem, pkprod, pbase, cached;
  reg [63:0] pbyte, cprod, prprod;
  reg [31:0] pb, pt;
  // A store into the token-row store's beats out of the unit: whether a beat is on its way a clock after its row is
  // asked, and @READ@ clocks after, when its word is out (ptv); the beat's lanes beside it (pt_mask).
  (* maxfan = 256 *) reg [@READ@-1:0] pt_on;
  reg [16*@READ@-1:0] pt_lanes;
  wire ptv = pt_on[@READ@-1];
  wire [15:0] pt_mask = pt_lanes[16*@READ@-1 -: 16];
  // A beat's lanes: those below the store's length.
  function [15:0] lanes_of(input [31:0] beat);
    reg [31:0] left, lanes;
    begin
      left = p_chunk - (beat << 4);
      lanes = p_scalar ? 32'd1 : (left >= 32'd16 ? 32'd16 : left);
      lanes_of = (lanes >= 32'd16) ? 16'hffff : ((16'd1 << lanes[4:0]) - 16'd1);
    end
  endfunction
  // A memory store's stream, a stage a clock: a beat asked, its row the unit's the clock after (prow) and its value out
  // @READ@ clocks after that (a_v and a_pt carry it @MS_OUT@ clocks, the last the clock its value is out), the value
  // captured with its byte and lanes (pm_*), then its binary16 halves and int8 codes @NARROW@ clocks on, the captured
  // fields carried beside them (d_*); there its line word is queued. A beat is asked only while the queue has room for it
  // and every beat on its way.
  localparam [4:0] MS_DEPTH = @MS_DEPTH@;
  reg [31:0] ms_pt;
  reg [@MS_OUT@:1] a_v;
  reg [32*@MS_OUT@-1:0] a_pt;
  wire a_out = a_v[@MS_OUT@];
  wire [31:0] a_out_pt = a_pt[32*@MS_OUT@-1 -: 32];
  reg pm_v;
  reg [511:0] pm_data;
  reg [15:0] pm_mask;
  reg [63:0] pm_byte;
  reg [@NARROW@:1] d_v;
  reg [511:0] d_val [1:@NARROW@];
  reg [15:0] d_mask [1:@NARROW@];
  reg [37:0] d_byte [1:@NARROW@];
  wire [255:0] halves;
  wire [127:0] codes, codes_n;
  generate
    for (g = 0; g < 16; g = g + 1) begin : narrow
      logos_f32_to_f16_p nh(.clk(clk), .x(pm_data[32*g +: 32]), .h(halves[16*g +: 16]));
      wire signed [31:0] c;
      logos_rne_p nc(.clk(clk), .x(pm_data[32*g +: 32]), .n(c));
      assign codes_n[8*g +: 8] = c[7:0];
    end
  endgenerate
  // The codes held to the halves' latency.
  logos_delay #(.W(128), .D(@CODES_PAD@)) dcodes(.clk(clk), .d(codes_n), .q(codes));
  integer dsh;
  // The beat out of the units: its lanes as bytes (four a lane of f32s, two a lane of binary16s) and its line word.
  wire [15:0] o_mask = d_mask[@NARROW@];
  wire [37:0] o_byte = d_byte[@NARROW@];
  wire [63:0] lanes4;
  wire [31:0] lanes2;
  generate
    for (g = 0; g < 16; g = g + 1) begin : lane_bytes
      assign lanes4[4*g +: 4] = {4{o_mask[g]}};
      assign lanes2[2*g +: 2] = {2{o_mask[g]}};
    end
  endgenerate
  reg [511:0] o_data;
  reg [63:0] o_wmask;
  always @* begin
    if (p_kind == 3'd1 && !f16) begin
      o_data = d_val[@NARROW@];
      o_wmask = lanes4;
    end else if (p_kind == 3'd1 || p_part == 2'd0) begin
      o_data = {256'd0, halves} << (8 * o_byte[5:0]);
      o_wmask = {32'd0, lanes2} << o_byte[5:0];
    end else if (p_part == 2'd1) begin
      o_data = {384'd0, codes} << (8 * o_byte[5:0]);
      o_wmask = {48'd0, o_mask} << o_byte[5:0];
    end else begin
      o_data = {496'd0, halves[15:0]} << (8 * o_byte[5:0]);
      o_wmask = 64'd3 << o_byte[5:0];
    end
  end
  // The queue of line words waiting for the line memory stores gather in.
  (* ramstyle = "MLAB, no_rw_check" *) reg [511:0] q_data [0:MS_DEPTH-1];
  `LOGOS_RAMSTYLE_ASYNC(MS_DEPTH) reg [63:0] q_mask [0:MS_DEPTH-1];
  `LOGOS_RAMSTYLE_ASYNC(MS_DEPTH) reg [31:0] q_line [0:MS_DEPTH-1];
  reg [3:0] q_wr, q_rd;
  reg [4:0] q_n;
  wire [4:0] ms_flight = a_v_count(a_v) + {4'd0, pm_v} + d_v_count(d_v);
  function [4:0] d_v_count(input [@NARROW@:1] v);
    integer dc;
    begin
      d_v_count = 5'd0;
      for (dc = 1; dc <= @NARROW@; dc = dc + 1) d_v_count = d_v_count + {4'd0, v[dc]};
    end
  endfunction
  function [4:0] a_v_count(input [@MS_OUT@:1] v);
    integer ac;
    begin
      a_v_count = 5'd0;
      for (ac = 1; ac <= @MS_OUT@; ac = ac + 1) a_v_count = a_v_count + {4'd0, v[ac]};
    end
  endfunction
  // Whether no word is on its way into the queue: ms_flight is zero exactly when no valid behind it is set.
  wire ms_none = ~|a_v && !pm_v && ~|d_v;
  wire ms_ask = state == MS && ms_pt != p_beats && q_n + ms_flight < MS_DEPTH;
  integer i;

  // The line memory stores gather in: its line, its bytes and which of them are written; a part of a line read,
  // merged and written; a whole one written.
  localparam [2:0] CB_EMPTY = 3'd0, CB_HELD = 3'd1, CB_WAIT = 3'd2, CB_READ = 3'd3, CB_RESP = 3'd4, CB_WRITE = 3'd5;
  (* maxfan = 256 *) reg [2:0] cb;
  reg [31:0] cb_line;
  reg [511:0] cb_data;
  reg [63:0] cb_mask;
  reg cb_flush;
  wire cb_full = &cb_mask;
  // The queue's head and whether the line takes it this clock: into an empty line, into its own held line, or as a
  // fresh line while the held one, full, goes to the write queue.
  wire q_have = q_n != 5'd0;
  wire [31:0] h_line = q_line[q_rd];
  wire [511:0] h_data = q_data[q_rd];
  wire [63:0] h_mask = q_mask[q_rd];
  wire wq_free = !wq_valid || wq_ready;
  wire hand_off = cb == CB_HELD && cb_line != h_line && cb_full && wq_free;
  wire cb_pop = q_have && (cb == CB_EMPTY || (cb == CB_HELD && (cb_line == h_line || hand_off)));
  // Every write the side made is on the card: none gathering, none queued, the port's all done.
  wire settled = cb == CB_EMPTY && !wq_valid && written;
  // Whether a ring row has been stored this job (ring_open) and the region of the row that last stored one: a ring load
  // of a row of that region waits for the card; the row at hand's region a clock behind its index.
  reg ring_open;
  reg [31:0] ring_region, row_region;
  always @(posedge clk) row_region <= rowt[k][31:0];
  wire ring_waits = ld_a[li][7:4] == 4'd3 && ring_open && ring_region == row_region;
  function [511:0] merged(input [511:0] held, input [511:0] taken, input [63:0] m);
    integer b;
    begin
      merged = held;
      for (b = 0; b < 64; b = b + 1) if (m[b]) merged[8*b +: 8] = taken[8*b +: 8];
    end
  endfunction

  // A float matrix's job (`fm`): every head's row of a tensor dotted with every row's input, a scalar a pass put at the
  // head's element of the row's output — a weight row a head, the input a slot's whole row, one Dot, one scalar put.
  // Every row it holds (up to FMM groups of FM_ROWS, row k of the pass group k / FM_ROWS's lanes k mod FM_ROWS) has
  // its input held; eight heads at a time, their weight rows asked from the memory a line of each in turn, at most FQ
  // of them owed, each back into a queue; the queue's head line goes to the lanes once for each group, then leaves the
  // queue, each giving waiting until no line of its head left for its group in the last FADD clocks, so its add reads
  // the sum the one before it wrote; its sixteen elements times every held row's of the group into 16·FM_ROWS lanes,
  // element i of a row into lane i mod 16 in index order from +0, as numerics::fdot adds it; then each row's head
  // through the tree into its output's element.
  wire fm_shape = heads_first && nload == 8'd2 && nput == 8'd1 && nprog == 16'd1 && ld_a[0][7:0] == 8'h10 &&
    ld_a[1][7:0] == 8'h01 && ld_a[0][63:32] == ld_a[1][63:32] && ld_a[0][35:32] == 4'd0 && ld_b[0][31:0] == 32'd0 &&
    ld_b[0][63:32] == ld_a[0][63:32] && ld_b[1] == 64'd0 && prog[0] == 64'h1000 && pt_a[0][7:0] == 8'h10 &&
    pt_a[0][63:32] == 32'd1 && pt_b[0] == {32'd1, 32'd0};
  localparam [31:0] FH = 8, FMR = FM_ROWS;
  // The groups of FM_ROWS rows the inputs and the sums are held for: every row the side's step holds.
  localparam FMM = (ROW_MAX + FM_ROWS - 1) / FM_ROWS;
  localparam [31:0] FMMR = FMM * FM_ROWS;
  localparam FIA = (FMM * VROWS > 1) ? $clog2(FMM * VROWS) : 1;
  // The sums' address: a group's place, at least a bit, above a head's three.
  localparam FSG = (FMM > 1) ? $clog2(FMM) : 1, FSA = FSG + 3;
  wire [31:0] fm_n = ld_a[0][63:32], fm_beats = fm_n >> 4;
  wire [63:0] fm_from = slott[ld_a[1][12:8]], fm_to = slott[pt_a[0][12:8]];
  reg [31:0] fm_k0, fm_h0, fm_rows, fm_heads, fm_tbase, fm_r, fm_b, fm_j, fm_rb, fm_rj, fm_tr, fm_tj;
  reg [31:0] fm_asked, fm_back, fm_pop, fm_gline, fm_hline;
  wire [31:0] fm_lines = fm_beats * fm_heads[3:0];
  reg fm_av;
  reg [31:0] fm_ar, fm_at;
  // A held input's beat asked (fm_av and its place fm_wr_*, the clock after its address) and written the clock its word is
  // out of the store, SRL clocks after its address (fm_ao, fm_ao_*).
  wire fm_ao, fm_aflight;
  wire [31:0] fm_ao_lg, fm_ao_at;
  // The held inputs, a block RAM a lane group, a group of rows at each VROWS beats of it: the row an input's read
  // writes next (its lane group fm_lg, its group's first beat fm_soff), and the beat a read writes, its lane group and
  // its beat in the RAM, a clock behind; the groups the pass's rows fill (fm_subs).
  reg [31:0] fm_lg, fm_soff, fm_subs, fm_wr_lg, fm_wr_at;
  reg [7:0] fm_sub;
  logos_landing #(.W(64), .D(SRL - 1)) fland(.clk(clk), .rst(rst), .in_v(fm_av), .in_d({fm_wr_lg, fm_wr_at}),
    .out_v(fm_ao), .out_d({fm_ao_lg, fm_ao_at}), .flight(fm_aflight));
  // The weight lines back, queued with their head and beat; the queue's head going to the lanes for each group in turn
  // (fq_give, group fq_sub at fq_soff) and leaving once its last group has it (fq_go), each giving held while a line
  // of its head left for the same group in the last FADD + 1 clocks: a head's sums are read a clock before its add takes
  // them, so the next of its lines reads the last's sum once it is written.
  localparam FQA = 5;
  localparam [31:0] FQ = 32'd1 << FQA;
  `LOGOS_RAMSTYLE(FQ) reg [511:0] fq_data [0:FQ-1];
  `LOGOS_RAMSTYLE_ASYNC(FQ) reg [2:0] fq_j [0:FQ-1];
  `LOGOS_RAMSTYLE_ASYNC(FQ) reg [15:0] fq_b [0:FQ-1];
  wire [FQA-1:0] fq_wr = fm_back[FQA-1:0], fq_rd = fm_pop[FQA-1:0];
  wire [2:0] fq_jh = fq_j[fq_rd];
  wire [31:0] fq_bh = {16'd0, fq_b[fq_rd]};
  reg [7:0] fq_sub;
  reg [31:0] fq_soff;
  reg [@SPACE@:1] pq_v;
  reg [2:0] pq_j [1:@SPACE@];
  reg [7:0] pq_s [1:@SPACE@];
  reg fq_clash;
  integer fk;
  always @* begin
    fq_clash = 1'b0;
    for (fk = 1; fk <= @SPACE@; fk = fk + 1) if (pq_v[fk] && pq_j[fk] == fq_jh && pq_s[fk] == fq_sub) fq_clash = 1'b1;
  end
  wire fq_give = state == FM_W && fm_pop != fm_back && !fq_clash;
  wire fq_last = {24'd0, fq_sub} + 32'd1 == fm_subs;
  wire fq_go = fq_give && fq_last;
  // The line given: its sixteen elements, head, group and beat (fw), carried a clock further (fw2) beside the group's held
  // rows' beats of it, whose read address is a register (fm_act_ra) copied near the RAMs it reaches; the products and
  // sums, each lane group's sums a block RAM of every group's heads.
  reg [511:0] fw_q, fw2_q;
  reg fw_v, fw_first, fw2_v, fw2_first;
  reg [2:0] fw_j, fw2_j;
  reg [7:0] fw_s, fw2_s;
  (* maxfan = 64 *) reg [FIA-1:0] fm_act_ra;
  wire [511:0] fa_q [0:FM_ROWS-1];
  reg [@FMUL@:1] fp_v;
  (* maxfan = 256 *) reg [@FMUL@:1] fp_first;
  reg [2:0] fp_j [1:@FMUL@];
  reg [7:0] fp_s [1:@FMUL@];
  reg [@FADD@:1] fs_v;
  reg [2:0] fs_j [1:@FADD@];
  reg [7:0] fs_s [1:@FADD@];
  wire [511:0] fm_sums [0:FM_ROWS-1];
  wire [511:0] fm_held [0:FM_ROWS-1];
  // The sums' read, from a register copied near the RAMs it reaches: the add's head of its group, taken two clocks before
  // the add takes the sums so it is read the clock before; or a row's head the tree takes next, read the clock after it
  // is taken.
  reg [31:0] ft_sub;
  (* maxfan = 64 *) reg [FSA-1:0] fm_acc_ra;
  always @(posedge clk)
    fm_acc_ra <= state == FM_TREE ? {ft_sub[FSG-1:0], fm_tj[2:0]} : {fp_s[@FMUL@-2][FSG-1:0], fp_j[@FMUL@-2]};
  integer fr;
  always @(posedge clk) begin
    if (state == FM_W && rs_valid) begin fq_data[fq_wr] <= rs_data; fq_j[fq_wr] <= fm_rj[2:0]; fq_b[fq_wr] <= fm_rb[15:0]; end
    pq_v <= rst ? {@SPACE@{1'b0}} : {pq_v[@SPACE@-1:1], fq_give};
    pq_j[1] <= fq_jh; pq_s[1] <= fq_sub;
    for (fr = 2; fr <= @SPACE@; fr = fr + 1) begin pq_j[fr] <= pq_j[fr-1]; pq_s[fr] <= pq_s[fr-1]; end
    if (state != FM_W) begin fq_sub <= 8'd0; fq_soff <= 32'd0; end
    else if (fq_give) begin
      fq_sub <= fq_last ? 8'd0 : fq_sub + 8'd1;
      fq_soff <= fq_last ? 32'd0 : fq_soff + VB;
    end
    fw_v <= !rst && fq_give;
    fw_q <= fq_data[fq_rd]; fw_j <= fq_jh; fw_s <= fq_sub; fw_first <= fq_bh == 32'd0;
    fm_act_ra <= fq_soff[FIA-1:0] + fq_bh[FIA-1:0];
    fw2_v <= !rst && fw_v;
    fw2_q <= fw_q; fw2_j <= fw_j; fw2_s <= fw_s; fw2_first <= fw_first;

    fp_v <= rst ? {@FMUL@{1'b0}} : {fp_v[@FMUL@-1:1], fw2_v};
    fp_first <= {fp_first[@FMUL@-1:1], fw2_first};
    fp_j[1] <= fw2_j; fp_s[1] <= fw2_s;
    for (fr = 2; fr <= @FMUL@; fr = fr + 1) begin fp_j[fr] <= fp_j[fr-1]; fp_s[fr] <= fp_s[fr-1]; end
    fs_v <= rst ? {@FADD@{1'b0}} : {fs_v[@FADD@-1:1], fp_v[@FMUL@]};
    fs_j[1] <= fp_j[@FMUL@]; fs_s[1] <= fp_s[@FMUL@];
    for (fr = 2; fr <= @FADD@; fr = fr + 1) begin fs_j[fr] <= fs_j[fr-1]; fs_s[fr] <= fs_s[fr-1]; end
  end
  genvar fmr, fml;
  generate
    for (fmr = 0; fmr < FM_ROWS; fmr = fmr + 1) begin : fm_row
      logos_bram_w512 #(.D(FMM * VROWS), .A(FIA)) act(.clk(clk), .ra(fm_act_ra), .rd(fa_q[fmr]),
        .we(fm_ao && fm_ao_lg == fmr), .wa(fm_ao_at[FIA-1:0]), .wd(st_rd), .wm({64{1'b1}}));
      logos_bram_w512 #(.D(FMM * 8), .A(FSA)) acc(.clk(clk), .ra(fm_acc_ra), .rd(fm_held[fmr]),
        .we(fs_v[@FADD@]), .wa({fs_s[@FADD@][FSG-1:0], fs_j[@FADD@]}), .wd(fm_sums[fmr]), .wm({64{1'b1}}));
      // The row's sums the add takes: its head's of its group, or +0 for a head's first line.
      wire [511:0] acc_in = fp_first[@FMUL@] ? 512'd0 : fm_held[fmr];
      for (fml = 0; fml < 16; fml = fml + 1) begin : lane
        wire [31:0] p;
        logos_fmul_p m(.clk(clk), .a(fw2_q[32*fml +: 32]), .b(fa_q[fmr][32*fml +: 32]), .y(p));
        logos_fadd_p a(.clk(clk), .a(acc_in[32*fml +: 32]), .b(p), .y(fm_sums[fmr][32*fml +: 32]));
      end
    end
  endgenerate
  // A row's head through the tree, the scalar into its output's element: the row's lane group (ft_lg) and group
  // (ft_sub) as FM_TREE walks the rows; its read's address registered a clock later (tr_*) and its word out of the row's
  // lane group's RAM the clock after (tr2_*).
  reg [511:0] ft_lanes;
  reg ft_v, tr_v, tr2_v;
  reg [31:0] ft_r, ft_h, tr_r, tr_h, ft_lg, tr_lg, tr2_r, tr2_h, tr2_lg;
  wire [31:0] ft_sum;
  logos_tree16_p ftree(.clk(clk), .lanes(ft_lanes), .sum(ft_sum));
  reg [@TREE@:1] ftd_v;
  reg [31:0] ftd_r [1:@TREE@];
  reg [31:0] ftd_h [1:@TREE@];
  always @(posedge clk) begin
    tr2_v <= !rst && tr_v; tr2_lg <= tr_lg; tr2_r <= tr_r; tr2_h <= tr_h;
    ft_v <= !rst && tr2_v;
    ft_lanes <= fm_held[tr2_lg]; ft_r <= tr2_r; ft_h <= tr2_h;
    ftd_v <= rst ? {@TREE@{1'b0}} : {ftd_v[@TREE@-1:1], ft_v};
    ftd_r[1] <= ft_r; ftd_h[1] <= ft_h;
    for (fr = 2; fr <= @TREE@; fr = fr + 1) begin ftd_r[fr] <= ftd_r[fr-1]; ftd_h[fr] <= ftd_h[fr-1]; end
  end
  wire fm_out = ftd_v[@TREE@];
  wire [31:0] fm_out_at = fm_to[31:0] + ftd_r[@TREE@] * fm_to[63:32] + (ftd_h[@TREE@] >> 4);

  // The job RAM's read address, and the store's: a pass's store load beat, or a store's beat; the vector unit's beat
  // a store reads; a float matrix's held input's beat.
  always @* begin
    j_ra = pc[JA-1:0];
    if (state == DESC) j_ra = pc[JA-1:0] + dp[JA-1:0];
    st_ra = lbase[SA-1:0] + lp[SA-1:0];
    if (state == FM_ACT) st_ra = fm_from[SA-1:0] + fm_ar[SA-1:0] * fm_from[SA-1+32:32] + fm_at[SA-1:0];
    s_rd_reg = p_reg; s_rd_row = prow; s_rd_s = p_reg;
  end

  always @(posedge clk) begin
    s_issue <= 1'b0; s_wr <= 1'b0; s_len_wr <= 1'b0; st_we <= 1'b0; cb_flush <= 1'b0;
    rs_v <= 1'b0; tr_v <= 1'b0;
    gmax <= gfit(grows);
    nk <= heads_first ? ((k + 16'd1 == nrows) ? 16'd0 : k + 16'd1) : ((h + 32'd1 == heads) ? k + 16'd1 : k);
    nh <= heads_first ? ((k + 16'd1 == nrows) ? h + 32'd1 : h) : ((h + 32'd1 == heads) ? 32'd0 : h + 32'd1);
    nend <= k + 16'd1 == nrows && h + 32'd1 == heads;
    if (fm_out) begin
      st_we <= 1'b1; st_wa <= fm_out_at[SA-1:0]; st_wd <= {480'd0, ft_sum} << (32 * ftd_h[@TREE@][3:0]);
      st_wm <= 64'hf << (4 * ftd_h[@TREE@][3:0]);
    end
    if (rst) begin
      state <= IDLE; fault <= 1'b0; rq_valid <= 1'b0; wq_valid <= 1'b0; cb <= CB_EMPTY; cbase <= 32'd0;
      a_v <= {@MS_OUT@{1'b0}}; pm_v <= 1'b0; d_v <= {@NARROW@{1'b0}}; q_wr <= 4'd0; q_rd <= 4'd0; q_n <= 5'd0;
      gi <= 8'd0; goff <= 16'd0; gn <= 8'd1; lbv <= 1'b0; fm_av <= 1'b0;
    end else begin
      if (port_fault) fault <= 1'b1;
      case (state)
        IDLE: if (start) begin
          pc <= job_pc; nrows <= rows; f16 <= state_f16; fault <= 1'b0; gi <= 8'd0; goff <= 16'd0; gn <= 8'd1; ring_open <= 1'b0;
          state <= HDR0;
        end
        HDR0: state <= HDR;
        HDR: begin hdr <= jw; pc <= pc + 32'd1; state <= HEADS0; end
        HEADS0: state <= HEADS;
        HEADS: begin
          heads <= jw[31:0]; pc <= pc + 32'd1 + {24'd0, nfresh}; dp <= 16'd0; dt <= 16'd0; dv <= 1'b0; state <= DESC;
        end
        // The descriptors, a word a clock: the loads' and stores' two words each, then the instructions.
        DESC: begin
          dv <= dp < nwords;
          if (dp < nwords) dp <= dp + 16'd1;
          if (dv) begin
            if (dt < {7'd0, nload, 1'b0}) begin
              if (dt[0] == 1'b0) ld_a[dt >> 1] <= jw; else ld_b[dt >> 1] <= jw;
              if (dt == 16'd0) jbeats <= (jw[63:32] + 32'd15) >> 4;
            end else if (dt < {7'd0, nload, 1'b0} + {7'd0, nput, 1'b0}) begin
              if (dt[0] == 1'b0) pt_a[(dt - {7'd0, nload, 1'b0}) >> 1] <= jw; else pt_b[(dt - {7'd0, nload, 1'b0}) >> 1] <= jw;
            end else prog[dt - {7'd0, nload, 1'b0} - {7'd0, nput, 1'b0}] <= jw;
            dt <= dt + 16'd1;
          end
          if (dt == nwords) begin
            k <= 16'd0; h <= 32'd0; cbase <= 32'd0; state <= PASS;
            grows <= (ld_a[0][63:32] == 32'd0) ? 16'd1 : ld_a[0][51:36] + {15'd0, ld_a[0][35:32] != 4'd0};
          end
        end
        // Every pass of a row and a head; a chunked job's again for each chunk after its first.
        PASS: begin
          if ((heads_first && h == heads) || (!heads_first && k == nrows) || heads == 32'd0 || nrows == 16'd0) begin
            if (chunked && heads != 32'd0 && nrows != 16'd0 && cbase + VB < jbeats) begin cbase <= cbase + VB; k <= 16'd0; h <= 32'd0; end
            else state <= DRAIN;
          end
          else if (fm_shape) begin fm_k0 <= 32'd0; t_ra <= ld_a[0][8+TA-1:8]; state <= FM_T; end
          else begin li <= 8'd0; gi <= 8'd0; goff <= 16'd0; gk0 <= k; gh0 <= h; state <= LOAD; end
        end
        // A float matrix: its tensor's word read; a group of rows' inputs held, a beat a clock; then its heads eight at a
        // time.
        FM_T: state <= FM0;
        FM0: if (settled) begin
          fm_tbase <= tw[31:0];
          fm_rows <= ({16'd0, nrows} - fm_k0 < FMMR) ? {16'd0, nrows} - fm_k0 : FMMR;
          fm_ar <= fm_k0; fm_at <= 32'd0; fm_r <= 32'd0; fm_av <= 1'b0; fm_h0 <= 32'd0;
          fm_lg <= 32'd0; fm_soff <= 32'd0; fm_sub <= 8'd0; state <= FM_ACT;
        end
        FM_ACT: begin
          fm_av <= fm_r != fm_rows;
          fm_wr_lg <= fm_lg; fm_wr_at <= fm_soff + fm_at;
          if (fm_r != fm_rows) begin
            if (fm_at + 32'd1 == fm_beats) begin
              fm_at <= 32'd0; fm_ar <= fm_ar + 32'd1; fm_r <= fm_r + 32'd1;
              if (fm_lg + 32'd1 == FMR) begin fm_lg <= 32'd0; fm_soff <= fm_soff + VB; fm_sub <= fm_sub + 8'd1; end
              else fm_lg <= fm_lg + 32'd1;
            end
            else fm_at <= fm_at + 32'd1;
          end else if (!fm_av && !fm_aflight) begin
            fm_subs <= {24'd0, fm_sub} + ((fm_lg != 32'd0) ? 32'd1 : 32'd0);
            fm_heads <= (heads - fm_h0 < FH) ? heads - fm_h0 : FH;
            fm_gline <= fm_tbase; fm_hline <= fm_tbase;
            fm_b <= 32'd0; fm_j <= 32'd0; fm_rb <= 32'd0; fm_rj <= 32'd0; fm_asked <= 32'd0; fm_back <= 32'd0;
            fm_pop <= 32'd0; rq_valid <= 1'b0; state <= FM_W;
          end
        end
        // Eight heads' weight rows, a line of each in turn, asked whenever the port takes one and the queue has room
        // for it; each line back into the queue, and from it into the lanes; once every line's sums are written, the
        // trees.
        FM_W: begin
          // The line at hand is head fm_j's beat fm_b: asked until the port takes it, then the next head's of the beat.
          if (rq_valid && rq_ready) begin
            fm_asked <= fm_asked + 32'd1;
            rq_valid <= fm_asked + 32'd1 != fm_lines && fm_asked + 32'd1 - fm_pop < FQ;
            if (fm_j + 32'd1 == fm_heads) begin
              fm_j <= 32'd0; fm_b <= fm_b + 32'd1; fm_hline <= fm_gline; rq_line <= fm_gline + fm_b + 32'd1;
            end else begin
              fm_j <= fm_j + 32'd1; fm_hline <= fm_hline + fm_beats; rq_line <= fm_hline + fm_beats + fm_b;
            end
          end else if (!rq_valid && fm_asked != fm_lines && fm_asked - fm_pop < FQ) begin
            rq_valid <= 1'b1;
            if (fm_asked == 32'd0) rq_line <= fm_hline + fm_b;
          end
          if (fq_go) fm_pop <= fm_pop + 32'd1;
          if (rs_valid) begin
            fm_back <= fm_back + 32'd1;
            if (fm_rj + 32'd1 == fm_heads) begin fm_rj <= 32'd0; fm_rb <= fm_rb + 32'd1; end else fm_rj <= fm_rj + 32'd1;
          end
          if (fm_asked == fm_lines && fm_pop == fm_lines && !fw_v && !fw2_v && fp_v == {@FMUL@{1'b0}} && fs_v == {@FADD@{1'b0}}) begin
            fm_tr <= 32'd0; fm_tj <= 32'd0; ft_lg <= 32'd0; ft_sub <= 32'd0; state <= FM_TREE;
          end
        end
        // Each held row's heads through the tree, a clock each, its sums read the clock before.
        FM_TREE: begin
          tr_v <= fm_tr != fm_rows; tr_lg <= ft_lg; tr_r <= fm_k0 + fm_tr; tr_h <= fm_h0 + fm_tj;
          if (fm_tr != fm_rows) begin
            if (fm_tj + 32'd1 == fm_heads) begin
              fm_tj <= 32'd0; fm_tr <= fm_tr + 32'd1;
              if (ft_lg + 32'd1 == FMR) begin ft_lg <= 32'd0; ft_sub <= ft_sub + 32'd1; end else ft_lg <= ft_lg + 32'd1;
            end
            else fm_tj <= fm_tj + 32'd1;
          end else if (ftd_v == {@TREE@{1'b0}} && !ft_v && !tr2_v && !tr_v) state <= FM_NEXT;
        end
        FM_NEXT: begin
          if (fm_h0 + FH < heads) begin
            fm_h0 <= fm_h0 + FH; fm_heads <= (heads - fm_h0 - FH < FH) ? heads - fm_h0 - FH : FH;
            fm_gline <= fm_gline + FH * fm_beats; fm_hline <= fm_gline + FH * fm_beats;
            fm_b <= 32'd0; fm_j <= 32'd0; fm_rb <= 32'd0; fm_rj <= 32'd0; fm_asked <= 32'd0; fm_back <= 32'd0; fm_pop <= 32'd0;
            state <= FM_W;
          end else if (fm_k0 + FMMR < {16'd0, nrows}) begin fm_k0 <= fm_k0 + FMMR; state <= FM0; end
          else state <= DRAIN;
        end
        // A load: its fields and tables; its element's offset, the row's slot, the rotary row, the ring's row; its
        // first beat's place; then its beats. A kept tensor load (its flag the word's bit 24) is loaded on the job's
        // first pass alone: its register holds it for every pass after.
        LOAD: begin
          if (li == nload) begin
            if (grouped && gi + 8'd1 < gmax && !nend) begin
              k <= nk; h <= nh; gi <= gi + 8'd1; goff <= goff + grows; li <= 8'd0;
            end else begin
              gn <= gi + 8'd1; k <= gk0; h <= gh0; gi <= 8'd0; goff <= 16'd0; ii <= 16'd0; state <= RUN;
            end
          end
          else if (ld_a[li][7:4] == 4'd1 && ld_a[li][24] && (gk0 != 16'd0 || gh0 != 32'd0)) li <= li + 8'd1;
          else if (!ring_waits || settled) begin
            la <= ld_a[li]; lbw <= ld_b[li]; state <= LB1;
          end
        end
        LB1: begin
          t_ra <= l_what[TA-1:0]; b_ra <= l_what[BA-1:0];
          row_e <= rowt[k]; slot_e <= slott[l_what[4:0]]; rope_e <= ropet[l_what];
          elem_lo <= {32'd0, h[15:0]} * {16'd0, lbw[63:32]}; elem_hi <= {32'd0, h[31:16]} * {16'd0, lbw[63:32]};
          state <= LB2;
        end
        LB2: begin
          elem <= lbw[31:0] + elem_lo[31:0] + {elem_hi[15:0], 16'd0};
          kprod <= {16'd0, k} * slot_e[63:32];
          rp_lo <= {16'd0, row_e[46:32], l_extra[0]} * {16'd0, rope_e[63:32]};
          rp_hi <= {32'd0, row_e[62:47]} * {16'd0, rope_e[63:32]};
          ring_sum <= {24'd0, row_e[71:64]} + {24'd0, l_extra};
          state <= LB3;
        end
        LB3: begin
          rprod <= rp_lo[31:0] + {rp_hi[15:0], 16'd0};
          ring_prod <= {32'd0, (ring_sum >= bw[95:64]) ? ring_sum - bw[95:64] : ring_sum} * {32'd0, bw[63:32]};
          state <= LB4;
        end
        LB4: begin
          ring_prod <= f16 ? ring_prod << 1 : ring_prod << 2;
          state <= LB5;
        end
        LB5: begin
          case (l_kind)
            4'd0: lbase <= slot_e[31:0] + kprod + (elem >> 4) + cbase;
            4'd1: lbase <= tw[31:0] + (elem >> 4) + cbase;
            4'd2: lbase <= rope_e[31:0] + (rprod >> 4);
            default: lbase <= 32'd0;
          endcase
          lbyte <= {32'd0, row_e[31:0]} * 64'd64 + {32'd0, bw[31:0]} + ring_prod + {32'd0, cbase} * (f16 ? 64'd32 : 64'd64);
          lp <= 32'd0; lt <= 32'd0; lr <= 32'd0; dv <= 1'b0; lbv <= 1'b0; state <= LBEAT;
        end
        // The beats: a store beat a clock, each written into the register the clock its word is out; or a memory line
        // asked whenever the port takes one, written in as it comes back.
        LBEAT: begin
          if (l_kind == 4'd0) begin
            if (lp != l_beats) lp <= lp + 32'd1;
            if (lbo) begin
              s_wr <= 1'b1; s_wr_reg <= l_reg; s_wr_row <= lt[15:0] + goff; s_wr_data <= st_rd; lt <= lt + 32'd1;
            end
            lbv <= lp != l_beats;
            if (lt == l_beats) state <= LLEN;
          end else begin
            // A line asked until the port takes it, then the next beat's, until every beat's is asked.
            lp <= lp_next;
            if (!rq_valid || rq_ready) begin
              rq_valid <= lp_next != l_beats;
              rq_line <= l_line(lp_next);
            end
            if (rs_valid) begin rs_r <= rs_data; rs_v <= 1'b1; rs_half <= l_byte_r[5]; lr <= lr + 32'd1; end
            if (rs_v) begin
              s_wr <= 1'b1; s_wr_reg <= l_reg; s_wr_row <= lt[15:0] + goff; s_wr_data <= from_f16 ? widened : rs_r; lt <= lt + 32'd1;
            end
            if (lt == l_beats && !rs_v) state <= LLEN;
          end
        end
        LLEN: begin
          s_len_wr <= 1'b1; s_wr_reg <= l_reg; s_len_val <= l_chunk[19:0]; li <= li + 8'd1; dv <= 1'b0; state <= LOAD;
        end
        // The micro-program, an instruction issued once the one before is done.
        RUN: begin
          if (ii == nprog) begin pi <= 8'd0; state <= PUT; end
          else begin s_issue <= 1'b1; s_instr <= prog[ii]; s_group <= gn; ii <= ii + 16'd1; state <= RWAIT; end
        end
        RWAIT: if (!s_issue && !s_busy) state <= RUN;
        // A store: its fields and tables; its element's offset, the row's slot, the ring's or the cache's row; its
        // first beat's place; then its beats.
        PUT: begin
          if (pi == nput) state <= (gi + 8'd1 < gn) ? GNEXT : NEXT;
          else begin pa <= pt_a[pi]; pbw <= pt_b[pi]; state <= PB1; end
        end
        PB1: begin
          b_ra <= p_what[BA-1:0];
          row_e <= rowt[k]; slot_e <= slott[p_what[4:0]];
          pelem_lo <= {32'd0, h[15:0]} * {16'd0, pbw[63:32]}; pelem_hi <= {32'd0, h[31:16]} * {16'd0, pbw[63:32]};
          state <= PB2;
        end
        PB2: begin
          pelem <= pbw[31:0] + pelem_lo[31:0] + {pelem_hi[15:0], 16'd0};
          pkprod <= {16'd0, k} * slot_e[63:32];
          state <= PB3;
        end
        PB3: begin
          cached_lo <= {16'd0, row_e[47:32]} * {16'd0, bw[159:128]}; cached_hi <= {16'd0, row_e[63:48]} * {16'd0, bw[159:128]};
          prprod <= {56'd0, row_e[71:64]} * {32'd0, bw[63:32]};
          state <= PB4;
        end
        PB4: begin
          cached <= cached_lo[31:0] + {cached_hi[15:0], 16'd0} + h;
          prprod <= f16 ? prprod << 1 : prprod << 2;
          state <= PB5;
        end
        PB5: begin
          cprod_lo <= {48'd0, cached[15:0]} * {32'd0, p_values ? bw[223:192] : bw[191:160]};
          cprod_hi <= {48'd0, cached[31:16]} * {32'd0, p_values ? bw[223:192] : bw[191:160]};
          state <= PB4B;
        end
        PB4B: begin
          cprod <= cprod_lo + (cprod_hi << 16);
          state <= PBF;
        end
        PBF: begin
          pbase <= slot_e[31:0] + pkprod + (pelem >> 4) + cbase;
          if (p_kind == 3'd1) begin
            pbyte <= {32'd0, row_e[31:0]} * 64'd64 + {32'd0, bw[31:0]} + prprod + {32'd0, cbase} * (f16 ? 64'd32 : 64'd64);
            ring_open <= 1'b1; ring_region <= row_e[31:0];
          end
          else case (p_part)
            2'd0: pbyte <= {32'd0, row_e[31:0]} * 64'd64 + {32'd0, p_values ? bw[63:32] : bw[31:0]} + cprod * 64'd2;
            2'd1: pbyte <= {32'd0, row_e[31:0]} * 64'd64 + {32'd0, p_values ? bw[63:32] : bw[31:0]} + cprod;
            default: pbyte <= {32'd0, row_e[31:0]} * 64'd64 + {32'd0, p_values ? bw[127:96] : bw[95:64]} + {32'd0, cached} * 64'd2;
          endcase
          pb <= 32'd0; prow <= goff; pt <= 32'd0; pt_on <= {@READ@{1'b0}}; ms_pt <= 32'd0;
          if (p_beats == 32'd0) begin pi <= pi + 8'd1; state <= PUT; end
          else state <= (p_kind == 3'd0) ? PBEAT : MS;
        end
        // A store into the token-row store: the register's beat asked a clock, written the clock it is out.
        PBEAT: begin
          if (pb != p_beats) begin pb <= pb + 32'd1; prow <= prow + 16'd1; end
          pt_on <= {pt_on[@READ@-2:0], pb != p_beats};
          pt_lanes <= {pt_lanes[16*@READ@-17:0], lanes_of(pb)};
          if (ptv) begin
            st_we <= 1'b1; st_wa <= pbase[SA-1:0] + pt[SA-1:0]; st_wd <= 512'd0; st_wm <= 64'd0;
            if (p_scalar) begin
              st_wd[32 * pelem[3:0] +: 32] <= s_rd_s_val; st_wm[4 * pelem[3:0] +: 4] <= 4'hf;
            end else for (i = 0; i < 16; i = i + 1)
              if (pt_mask[i]) begin st_wd[32*i +: 32] <= s_rd_data[32*i +: 32]; st_wm[4*i +: 4] <= 4'hf; end
            pt <= pt + 32'd1;
          end
          if (ptv && pt + 32'd1 == p_beats) begin pi <= pi + 8'd1; state <= PUT; end
        end
        // A store into the memory: its beats asked a clock each while the queue has room (ms_ask, the stream below);
        // done once every beat is asked and through, the queue empty and its last push taken.
        MS: begin
          if (ms_ask) begin ms_pt <= ms_pt + 32'd1; prow <= ms_pt[15:0] + goff; end
          if (ms_pt == p_beats && ms_none && !q_have) begin pi <= pi + 8'd1; state <= PUT; end
        end
        NEXT: begin
          cb_flush <= 1'b1;
          if (heads_first) begin
            if (k + 16'd1 == nrows) begin k <= 16'd0; h <= h + 32'd1; end else k <= k + 16'd1;
          end else begin
            if (h + 32'd1 == heads) begin h <= 32'd0; k <= k + 16'd1; end else h <= h + 32'd1;
          end
          state <= PASS;
        end
        // The group's next pass's stores, its line flushed.
        GNEXT: begin
          cb_flush <= 1'b1; k <= nk; h <= nh; gi <= gi + 8'd1; goff <= goff + grows; pi <= 8'd0; state <= PUT;
        end
        // The job's end: its last line written, every write on the card.
        DRAIN: if (!cb_flush && settled) state <= IDLE;
        default: state <= IDLE;
      endcase

      // The memory store's stream: the asked beat's value out of the unit @MS_OUT@ clocks after its ask, captured with
      // its byte and lanes; carried beside its halves and codes; its line word queued.
      a_v <= {a_v[@MS_OUT@-1:1], ms_ask};
      a_pt <= {a_pt[32*@MS_OUT@-33:0], ms_pt};
      pm_v <= a_out;
      if (a_out) begin
        pm_data <= p_scalar ? {480'd0, s_rd_s_val} : s_rd_data;
        pm_mask <= lanes_of(a_out_pt);
        case (p_kind == 3'd1 ? 2'd3 : p_part)
          2'd0: pm_byte <= pbyte + {32'd0, a_out_pt} * 64'd32;
          2'd1: pm_byte <= pbyte + {32'd0, a_out_pt} * 64'd16;
          2'd2: pm_byte <= pbyte;
          default: pm_byte <= pbyte + {32'd0, a_out_pt} * (f16 ? 64'd32 : 64'd64);
        endcase
      end
      d_v <= {d_v[@NARROW_LESS@:1], pm_v};
      d_val[1] <= pm_data; d_mask[1] <= pm_mask; d_byte[1] <= pm_byte[37:0];
      for (dsh = 2; dsh <= @NARROW@; dsh = dsh + 1) begin d_val[dsh] <= d_val[dsh - 1]; d_mask[dsh] <= d_mask[dsh - 1]; d_byte[dsh] <= d_byte[dsh - 1]; end
      if (d_v[@NARROW@]) begin q_data[q_wr] <= o_data; q_mask[q_wr] <= o_wmask; q_line[q_wr] <= o_byte[37:6]; q_wr <= q_wr + 4'd1; end
      if (cb_pop) q_rd <= q_rd + 4'd1;
      q_n <= q_n + {4'd0, d_v[@NARROW@]} - {4'd0, cb_pop};

      // The line memory stores gather in: the queue's head into an empty line or merged into its own; a full line
      // handed to the write queue in the clock the next line's first beat comes, the queue free; a full line flushed
      // handed over once the queue is free; a part of a line read once every write before it is on the card, merged,
      // and written.
      if (wq_valid && wq_ready) wq_valid <= 1'b0;
      case (cb)
        CB_EMPTY: if (cb_pop) begin cb <= CB_HELD; cb_line <= h_line; cb_data <= h_data; cb_mask <= h_mask; end
        CB_HELD: begin
          if (cb_pop && cb_line == h_line) begin cb_data <= merged(cb_data, h_data, h_mask); cb_mask <= cb_mask | h_mask; end
          else if (cb_pop) begin
            wq_valid <= 1'b1; wq_line <= cb_line; wq_data <= cb_data;
            cb_line <= h_line; cb_data <= h_data; cb_mask <= h_mask;
          end
          else if (cb_flush || (q_have && cb_line != h_line && !cb_full)) cb <= cb_full ? CB_WRITE : CB_WAIT;
        end
        CB_WAIT: if (!wq_valid && written) begin cb <= CB_READ; rq_valid <= 1'b1; rq_line <= cb_line; end
        CB_READ: if (rq_ready) begin rq_valid <= 1'b0; cb <= CB_RESP; end
        CB_RESP: if (rs_valid) begin cb_data <= merged(rs_data, cb_data, cb_mask); cb_mask <= {64{1'b1}}; cb <= CB_WRITE; end
        CB_WRITE: if (!wq_valid || wq_ready) begin wq_valid <= 1'b1; wq_line <= cb_line; wq_data <= cb_data; cb <= CB_EMPTY; end
        default: cb <= CB_EMPTY;
      endcase
    end
  end
endmodule
"#;

/// Cycles from a conv lane's operands to its value (`logos_conv_lane_p`): the products, the four adds one after another,
/// the SiLU.
pub const CONV_LATENCY: usize = {
    use super::fp32::{FADD_LATENCY, FMUL_LATENCY};
    FMUL_LATENCY + 4 * FADD_LATENCY + super::simd::LANE_FUNC_P_LATENCY
};

// The path decides what a narrowed beat does a stage before it is out of the binary16 rounding.
const _: () = assert!(super::fp32::F32_TO_F16_LATENCY >= 2, "the conv path reads the narrowing's stage before its last");

/// `logos_conv_lane_p` and `logos_conv_path`, a convolution job's rows through the vector side
/// ([`super::engine::Streaming::with_conv_path`]).
pub fn conv_path_verilog() -> String {
    CONV_PATH
        .replace("@FADD@", &super::fp32::FADD_LATENCY.to_string())
        .replace("@CVLAT@", &CONV_LATENCY.to_string())
        .replace("@NARROW@", &super::fp32::F32_TO_F16_LATENCY.to_string())
}

const CONV_PATH: &str = r#"// A convolution's lane (the machine's conv job, numerics::conv): a channel's ring values r0..r2, oldest first, and the
// token's x, each by its tap, CVLAT clocks to the SiLU of their sum: the four products side by side, the sum from +0 in
// tap order (each next product held to its add), the SiLU on the function unit — as the conv micro-program's
// multiply-adds and its Map compute them.
module logos_conv_lane_p(input clk, input [31:0] r0, input [31:0] r1, input [31:0] r2, input [31:0] x,
                         input [31:0] k0, input [31:0] k1, input [31:0] k2, input [31:0] k3, output [31:0] y);
  wire [31:0] p0, p1, p2, p3, p1d, p2d, p3d, s0, s1, s2, s3;
  logos_fmul_p m0(.clk(clk), .a(r0), .b(k0), .y(p0));
  logos_fmul_p m1(.clk(clk), .a(r1), .b(k1), .y(p1));
  logos_fmul_p m2(.clk(clk), .a(r2), .b(k2), .y(p2));
  logos_fmul_p m3(.clk(clk), .a(x), .b(k3), .y(p3));
  logos_delay #(.W(32), .D(@FADD@)) d1(.clk(clk), .d(p1), .q(p1d));
  logos_delay #(.W(32), .D(2*@FADD@)) d2(.clk(clk), .d(p2), .q(p2d));
  logos_delay #(.W(32), .D(3*@FADD@)) d3(.clk(clk), .d(p3), .q(p3d));
  logos_fadd_p a0(.clk(clk), .a(32'd0), .b(p0), .y(s0));
  logos_fadd_p a1(.clk(clk), .a(s0), .b(p1d), .y(s1));
  logos_fadd_p a2(.clk(clk), .a(s1), .b(p2d), .y(s2));
  logos_fadd_p a3(.clk(clk), .a(s2), .b(p3d), .y(s3));
  logos_lane_func_p silu(.clk(clk), .f(4'd4), .x(s3), .y(y));
endmodule

// A convolution job's rows through the vector side (the machine's conv job, numerics::conv, its taps four): the kernel's
// taps read from the memory once a job into block RAM, tap j's beat b at word b of its RAM; then each row of the step: its
// sequence's three ring rows (the layer's ring at region·64 + ring_off, row (ring + age) mod ring_taps, oldest first)
// asked a line of each in turn, at most CRQ owed, each back into a queue and three at a time into the slot of two the
// lanes do not read; its x from the token-row store a beat at a time into a queue (at most XQ asked ahead), the head the
// beat at hand's; LANES channels a clock through logos_conv_lane_p, every output beat written into the store once its
// last lane is out; and x, narrowed to the state's format, into the ring's oldest row through the side's line queue (q_v,
// a line word a push). A row of the region whose ring the row before wrote waits for every write to be on the card
// (settled) before it asks a ring line. No capture of a wide word waits on logic: the beat's kernel words and x are read
// in place, each slot fills under a register, the outputs are taken every clock and marked by their valid.
module logos_conv_path #(parameter SA = 10, CVB = 640, LANES = 4, SRL = 1)
  (input clk, input rst, input start, input [15:0] rows, input f16,
   input [31:0] k_line, input [31:0] beats, input [31:0] x_base, input [31:0] x_stride, input [31:0] o_base,
   input [31:0] o_stride, input [31:0] ring_off, input [7:0] ring_taps, input [31:0] row_bytes,
   output reg [15:0] row_k, input [127:0] row_e, input settled, output busy,
   output [SA-1:0] st_ra, input [511:0] st_rd, output reg st_we, output reg [SA-1:0] st_wa, output reg [511:0] st_wd,
   output reg rq_valid, input rq_ready, output reg [31:0] rq_line, input rs_valid, input [511:0] rs_data,
   output reg q_v, output reg [511:0] q_data, output reg [63:0] q_mask, output reg [31:0] q_line, input [4:0] q_n,
   output reg flush);
  localparam CBA = (CVB > 1) ? $clog2(CVB) : 1, CVLAT = @CVLAT@, NARROW = @NARROW@;
  localparam [7:0] S = 16 / LANES;
  localparam [31:0] CRQ = 16;
  localparam [3:0] XQ = 8;
  localparam [3:0] IDLE = 4'd0, KASK = 4'd1, ROW = 4'd2, R1 = 4'd3, R2 = 4'd4, WAITR = 4'd5, RUN = 4'd6, FIN = 4'd7,
                   FIN2 = 4'd8;
  // The state and the job's fields every lane's operands read: registers Quartus copies beside each group of loads.
  (* maxfan = 64 *) reg [3:0] state;
  assign busy = state != IDLE || q_v || flush;
  reg [15:0] r, nrows;
  (* maxfan = 64 *) reg ff16;
  // The card's lines back, a clock on.
  (* maxfan = 64 *) reg rsv;
  reg [511:0] rsd;
  // The kernel: tap j's beat b at word b of kr j; the fill's next line asked (ka_j, ka_b) and the next back (kg_j, kg_b).
  reg [CBA-1:0] k_ra, k_wa;
  reg k_we;
  reg [1:0] k_wj;
  reg [511:0] k_wd;
  wire [511:0] kv0, kv1, kv2, kv3;
  logos_bram_w512 #(.D(CVB), .A(CBA)) kr0(.clk(clk), .ra(k_ra), .rd(kv0), .we(k_we && k_wj == 2'd0), .wa(k_wa), .wd(k_wd), .wm({64{1'b1}}));
  logos_bram_w512 #(.D(CVB), .A(CBA)) kr1(.clk(clk), .ra(k_ra), .rd(kv1), .we(k_we && k_wj == 2'd1), .wa(k_wa), .wd(k_wd), .wm({64{1'b1}}));
  logos_bram_w512 #(.D(CVB), .A(CBA)) kr2(.clk(clk), .ra(k_ra), .rd(kv2), .we(k_we && k_wj == 2'd2), .wa(k_wa), .wd(k_wd), .wm({64{1'b1}}));
  logos_bram_w512 #(.D(CVB), .A(CBA)) kr3(.clk(clk), .ra(k_ra), .rd(kv3), .we(k_we && k_wj == 2'd3), .wa(k_wa), .wd(k_wd), .wm({64{1'b1}}));
  reg [31:0] ka_b, kg_b, kg_n;
  reg [1:0] ka_j, kg_j;
  reg k_done_ask;
  // The row: its ring rows' first lines, the line its x goes to, its x and output beats' first, whether it waits.
  reg [31:0] region, rl0, rl1, rl2, xrow, orow;
  reg [7:0] ring;
  reg ring_open;
  reg [31:0] last_region;
  wire [7:0] i0 = ring, i1 = (ring + 8'd1 >= ring_taps) ? ring + 8'd1 - ring_taps : ring + 8'd1,
             i2 = (ring + 8'd2 >= ring_taps) ? ring + 8'd2 - ring_taps : ring + 8'd2;
  function [31:0] at_row(input [7:0] idx);
    at_row = (idx == 8'd0) ? 32'd0 : (idx == 8'd1) ? row_bytes : row_bytes << 1;
  endfunction
  // Each ring row's first byte, the row's region in lines.
  wire [37:0] rb0 = {region, 6'd0} + {6'd0, ring_off} + {6'd0, at_row(i0)}, rb1 = {region, 6'd0} + {6'd0, ring_off} + {6'd0, at_row(i1)},
              rb2 = {region, 6'd0} + {6'd0, ring_off} + {6'd0, at_row(i2)};
  wire [31:0] lpr = ff16 ? (beats + 32'd1) >> 1 : beats;
  // The ring rows' lines asked: line ra_l of age ra_a; how many asked, back, and taken into the triple.
  reg [31:0] ra_l, ra_n, rg_n, rp_n;
  reg [1:0] ra_a;
  wire [31:0] ra_line = ((ra_a == 2'd0) ? rl0 : (ra_a == 2'd1) ? rl1 : rl2) + ra_l;
  `LOGOS_RAMSTYLE(CRQ) reg [511:0] cq [0:CRQ-1];
  reg [3:0] cq_w, cq_r;
  // The ring triples: two slots, the lanes reading slot cur_s (its line cur_l) while the queue fills the other (nx_n lines
  // in), each line shifted in at the top under the register that marks it the slot's, the oldest at the bottom once three
  // are in.
  reg [511:0] sa0, sa1, sa2, sb0, sb1, sb2, cq_q;
  reg [1:0] nx_n;
  (* maxfan = 64 *) reg cur_s;
  (* maxfan = 64 *) reg fill_a, fill_b;
  reg cur_v, cq_rv;
  reg [31:0] cur_l;
  wire [511:0] cur0 = cur_s ? sb2 : sa2, cur1 = cur_s ? sb1 : sa1, cur2 = cur_s ? sb0 : sa0;
  // x: beats asked from the store (xa_n), out of it SRL clocks on and a clock more into the queue (xlq, xld); the beats
  // asked and not yet taken by a beat's last step (x_out), those on their way from the store and those queued, so a beat
  // is asked only while the queue holds room for every one of them, whatever the store's latency.
  reg [31:0] xa_n;
  reg xa_v;
  reg [3:0] x_out;
  wire x_ask = state == RUN && xa_n != beats && x_out < XQ - 4'd1;
  wire xl_v, xl_flight;
  wire [0:0] xl_none;
  logos_landing #(.W(1), .D(SRL - 1)) xland(.clk(clk), .rst(rst), .in_v(xa_v), .in_d(1'b0), .out_v(xl_v), .out_d(xl_none), .flight(xl_flight));
  assign st_ra = xrow[SA-1:0] + xa_n[SA-1:0];
  (* maxfan = 64 *) reg xlq;
  reg [511:0] xld;
  `LOGOS_RAMSTYLE_ASYNC(XQ) reg [511:0] xq [0:XQ-1];
  reg [2:0] xq_w, xq_r;
  reg [3:0] xq_n;
  wire [511:0] xh = xq[xq_r];
  // The beat at hand (b) and its step (c): the lanes take channels LANES·c on of it; a beat starts with its ring triple,
  // its x at the queue's head, its kernel words out of the RAMs, and room in the line queue for its ring line.
  (* maxfan = 64 *) reg [31:0] b;
  (* maxfan = 64 *) reg [7:0] c;
  reg on;
  reg [31:0] kv_beat;
  wire [31:0] need_l = ff16 ? b >> 1 : b;
  wire beat_go = state == RUN && !on && b != beats && cur_v && cur_l == need_l && xq_n != 4'd0 && kv_beat == b && q_n < 5'd10;
  // The lanes' operands, registered: each lane's three ring values (binary16 widened), x and four taps.
  reg [32*LANES-1:0] lr0, lr1, lr2, lx, lk0, lk1, lk2, lk3;
  wire [32*LANES-1:0] ly;
  wire [511:0] w0, w1, w2;
  genvar g;
  generate
    for (g = 0; g < 16; g = g + 1) begin : widen
      logos_f16_to_f32 v0(.h(b[0] ? cur0[256 + 16*g +: 16] : cur0[16*g +: 16]), .x(w0[32*g +: 32]));
      logos_f16_to_f32 v1(.h(b[0] ? cur1[256 + 16*g +: 16] : cur1[16*g +: 16]), .x(w1[32*g +: 32]));
      logos_f16_to_f32 v2(.h(b[0] ? cur2[256 + 16*g +: 16] : cur2[16*g +: 16]), .x(w2[32*g +: 32]));
    end
    for (g = 0; g < LANES; g = g + 1) begin : lane
      logos_conv_lane_p u(.clk(clk), .r0(lr0[32*g +: 32]), .r1(lr1[32*g +: 32]), .r2(lr2[32*g +: 32]), .x(lx[32*g +: 32]),
        .k0(lk0[32*g +: 32]), .k1(lk1[32*g +: 32]), .k2(lk2[32*g +: 32]), .k3(lk3[32*g +: 32]), .y(ly[32*g +: 32]));
    end
  endgenerate
  wire [511:0] rr0 = ff16 ? w0 : cur0, rr1 = ff16 ? w1 : cur1, rr2 = ff16 ? w2 : cur2;
  // Each step's lanes in flight: its beat and step, its output's lanes gathered into the beat once out.
  (* maxfan = 64 *) reg [CVLAT:0] f_v;
  reg [31:0] f_b [0:CVLAT];
  reg [7:0] f_c [0:CVLAT];
  reg [511:0] ob;
  // A step's lanes shifted in above the steps before it: once a beat's last step is in, its first step's lanes are at
  // the bottom, each lane at its channel's place.
  wire [511:0] ob_next;
  generate
    if (LANES == 16) begin : whole
      assign ob_next = ly;
    end else begin : shifted
      assign ob_next = {ly, ob[511:32*LANES]};
    end
  endgenerate
  integer i;
  // x narrowed for the ring: the beat taken (n_v, its beat), its binary16 halves NARROW clocks on; a line of two halves
  // (or a whole f32 beat) pushed into the side's queue.
  reg nv;
  reg [31:0] n_b;
  reg [511:0] n_x;
  reg [NARROW:1] nd_v;
  reg [31:0] nd_b [1:NARROW];
  reg [511:0] nd_x [1:NARROW];
  wire [255:0] halves;
  generate
    for (g = 0; g < 16; g = g + 1) begin : narrow
      logos_f32_to_f16_p nh(.clk(clk), .x(n_x[32*g +: 32]), .h(halves[16*g +: 16]));
    end
  endgenerate
  reg [255:0] lo_half;
  // What the narrowed beat out next does, decided a stage before it is out: pushed (a whole f32 beat, or a line of two
  // halves or the last alone), or its halves kept for the next.
  reg p_v, p_lo;
  (* maxfan = 64 *) reg p_odd;
  reg [31:0] wrow;
  wire narrowing = nv || nd_v != {NARROW{1'b0}};
  wire lanes_busy = f_v != {(CVLAT+1){1'b0}};
  always @(posedge clk) begin
    flush <= 1'b0; k_we <= 1'b0; xa_v <= 1'b0; nv <= 1'b0;
    // The card's lines, a clock on: the kernel's into its RAMs, the ring's into the queue.
    rsv <= rs_valid; rsd <= rs_data; k_wd <= rsd;
    if (state == KASK && rsv) begin
      k_we <= 1'b1; k_wj <= kg_j; k_wa <= kg_b[CBA-1:0];
      if (kg_b + 32'd1 == beats) begin kg_b <= 32'd0; kg_j <= kg_j + 2'd1; end else kg_b <= kg_b + 32'd1;
      kg_n <= kg_n + 32'd1;
    end
    if (state == RUN && rsv) begin cq[cq_w] <= rsd; cq_w <= cq_w + 4'd1; rg_n <= rg_n + 32'd1; end
    // The slots: the queue's head read a clock, then shifted into the slot the lanes do not read; that slot made the
    // lanes' once its three lines are in and the lanes are past their line.
    cq_rv <= 1'b0; fill_a <= 1'b0; fill_b <= 1'b0;
    if (state == RUN && {1'b0, nx_n} + {1'b0, cq_rv} < 3'd3 && rp_n != rg_n) begin
      cq_q <= cq[cq_r]; cq_r <= cq_r + 4'd1; rp_n <= rp_n + 32'd1; cq_rv <= 1'b1; fill_a <= cur_s; fill_b <= !cur_s;
    end
    if (fill_a) begin sa0 <= cq_q; sa1 <= sa0; sa2 <= sa1; end
    if (fill_b) begin sb0 <= cq_q; sb1 <= sb0; sb2 <= sb1; end
    if (cq_rv) nx_n <= nx_n + 2'd1;
    if (state == RUN && nx_n == 2'd3 && !cq_rv && (!cur_v || (cur_l != need_l && !on))) begin
      cur_s <= !cur_s; cur_v <= 1'b1; cur_l <= cur_v ? cur_l + 32'd1 : 32'd0; nx_n <= 2'd0;
    end
    // x: a beat asked while the queue has room for it beside those on their way; into the queue a clock after it is out.
    if (x_ask) begin xa_v <= 1'b1; xa_n <= xa_n + 32'd1; end
    xlq <= xl_v; xld <= st_rd;
    if (xlq) begin xq[xq_w] <= xld; xq_w <= xq_w + 3'd1; end
    // The kernel's words: the beat at hand's out of the RAMs from before its start through its last step's read, the
    // next beat's address taken at the edge before that read, so its words are out (kv_beat the beat they are) the clock
    // after the beat's last step.
    kv_beat <= {{(32-CBA){1'b0}}, k_ra};
    if (state == RUN && ((beat_go && S == 8'd1) || (on && c + 8'd2 == S))) k_ra <= b[CBA-1:0] + 1'b1;
    // A beat's steps, LANES channels a step; its x popped from the queue at its last.
    if (beat_go) begin
      on <= 1'b1; c <= 8'd0; nv <= 1'b1;
    end else if (on) begin
      if (c + 8'd1 == S) begin on <= 1'b0; b <= b + 32'd1; xq_r <= xq_r + 3'd1; end
      c <= c + 8'd1;
    end
    xq_n <= xq_n + {3'd0, xlq} - {3'd0, on && c + 8'd1 == S};
    x_out <= x_out + {3'd0, x_ask} - {3'd0, on && c + 8'd1 == S};
    n_b <= b; n_x <= xh;
    for (i = 0; i < LANES; i = i + 1) begin
      lr0[32*i +: 32] <= rr0[32*(LANES*c + i) +: 32];
      lr1[32*i +: 32] <= rr1[32*(LANES*c + i) +: 32];
      lr2[32*i +: 32] <= rr2[32*(LANES*c + i) +: 32];
      lx[32*i +: 32] <= xh[32*(LANES*c + i) +: 32];
      lk0[32*i +: 32] <= kv0[32*(LANES*c + i) +: 32];
      lk1[32*i +: 32] <= kv1[32*(LANES*c + i) +: 32];
      lk2[32*i +: 32] <= kv2[32*(LANES*c + i) +: 32];
      lk3[32*i +: 32] <= kv3[32*(LANES*c + i) +: 32];
    end
    // The steps in flight; an output beat written as its last step's lanes are out.
    f_v <= {f_v[CVLAT-1:0], on};
    f_b[0] <= b; f_c[0] <= c;
    for (i = 1; i <= CVLAT; i = i + 1) begin f_b[i] <= f_b[i-1]; f_c[i] <= f_c[i-1]; end
    if (f_v[CVLAT]) ob <= ob_next;
    st_we <= f_v[CVLAT] && f_c[CVLAT] + 8'd1 == S; st_wa <= orow[SA-1:0] + f_b[CVLAT][SA-1:0]; st_wd <= ob_next;
    // x narrowed into the ring, a line of two beats' halves (the last beat of an odd count alone), or a beat of f32s.
    nd_v <= {nd_v[NARROW-1:1], nv};
    nd_b[1] <= n_b; nd_x[1] <= n_x;
    for (i = 2; i <= NARROW; i = i + 1) begin nd_b[i] <= nd_b[i-1]; nd_x[i] <= nd_x[i-1]; end
    p_v <= nd_v[NARROW-1] && (!ff16 || nd_b[NARROW-1][0] || nd_b[NARROW-1] + 32'd1 == beats);
    p_lo <= nd_v[NARROW-1] && ff16 && !nd_b[NARROW-1][0] && nd_b[NARROW-1] + 32'd1 != beats;
    p_odd <= nd_b[NARROW-1][0];
    q_v <= p_v;
    if (p_lo) lo_half <= halves;
    q_line <= wrow + (ff16 ? nd_b[NARROW] >> 1 : nd_b[NARROW]);
    q_data <= !ff16 ? nd_x[NARROW] : p_odd ? {halves, lo_half} : {256'd0, halves};
    q_mask <= (!ff16 || p_odd) ? {64{1'b1}} : {32'd0, {32{1'b1}}};
    if (rst) begin
      state <= IDLE; rq_valid <= 1'b0; f_v <= {(CVLAT+1){1'b0}}; nd_v <= {NARROW{1'b0}}; on <= 1'b0; ring_open <= 1'b0;
      rsv <= 1'b0; xlq <= 1'b0; cq_rv <= 1'b0; fill_a <= 1'b0; fill_b <= 1'b0; p_v <= 1'b0; p_lo <= 1'b0; q_v <= 1'b0;
      st_we <= 1'b0; nv <= 1'b0; cur_s <= 1'b0; x_out <= 4'd0;
    end else case (state)
      IDLE: if (start) begin
        nrows <= rows; ff16 <= f16; r <= 16'd0; ka_b <= 32'd0; ka_j <= 2'd0; kg_b <= 32'd0; kg_j <= 2'd0; kg_n <= 32'd0; ring_open <= 1'b0;
        rq_valid <= 1'b1; rq_line <= k_line; k_done_ask <= 1'b0; state <= KASK;
      end
      // The kernel: tap j's line b at k_line + j·beats + b, asked in turn; done once every line is back.
      KASK: begin
        if (rq_valid && rq_ready) begin
          if (ka_b + 32'd1 == beats) begin
            ka_b <= 32'd0; ka_j <= ka_j + 2'd1;
            if (ka_j == 2'd3) begin rq_valid <= 1'b0; k_done_ask <= 1'b1; end
            else rq_line <= k_line + (({30'd0, ka_j} + 32'd1) * beats);
          end else begin ka_b <= ka_b + 32'd1; rq_line <= rq_line + 32'd1; end
        end
        if (k_done_ask && kg_n == 32'd4 * beats) state <= ROW;
      end
      ROW: if (r == nrows) state <= IDLE; else begin row_k <= r; state <= R1; end
      R1: begin
        region <= row_e[31:0]; ring <= row_e[71:64];
        xrow <= x_base + {16'd0, r} * x_stride; orow <= o_base + {16'd0, r} * o_stride;
        state <= R2;
      end
      R2: begin
        rl0 <= rb0[37:6]; rl1 <= rb1[37:6]; rl2 <= rb2[37:6]; wrow <= rb0[37:6];
        state <= WAITR;
      end
      WAITR: if (!(ring_open && last_region == region) || settled) begin
        ra_l <= 32'd0; ra_a <= 2'd0; ra_n <= 32'd0; rg_n <= 32'd0; rp_n <= 32'd0; cq_w <= 4'd0; cq_r <= 4'd0;
        nx_n <= 2'd0; cur_v <= 1'b0; cur_l <= 32'd0; xa_n <= 32'd0; xq_w <= 3'd0; xq_r <= 3'd0; xq_n <= 4'd0; x_out <= 4'd0; b <= 32'd0;
        k_ra <= {CBA{1'b0}}; rq_valid <= 1'b0; state <= RUN;
      end
      // The row's lines asked whenever the port takes one and the queue has room; done once every output beat is
      // written and every ring line pushed.
      RUN: begin
        if (rq_valid && rq_ready) begin
          ra_n <= ra_n + 32'd1;
          if (ra_a == 2'd2) begin ra_a <= 2'd0; ra_l <= ra_l + 32'd1; end else ra_a <= ra_a + 2'd1;
        end
        rq_valid <= (ra_n + {31'd0, rq_valid && rq_ready}) != 32'd3 * lpr && (ra_n + {31'd0, rq_valid && rq_ready}) - rp_n < CRQ - 2;
        rq_line <= (rq_valid && rq_ready) ? ((ra_a == 2'd2) ? rl0 + ra_l + 32'd1 : ((ra_a == 2'd0) ? rl1 : rl2) + ra_l) : ra_line;
        if (b == beats && !on && !lanes_busy && !narrowing && !q_v) state <= FIN;
      end
      FIN: begin flush <= 1'b1; ring_open <= 1'b1; last_region <= region; r <= r + 16'd1; state <= FIN2; end
      FIN2: state <= ROW;
      default: state <= IDLE;
    endcase
  end
endmodule
"#;
