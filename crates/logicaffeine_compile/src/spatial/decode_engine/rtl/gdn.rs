//! The Gated-DeltaNet row engine as Verilog: one value row of a memory through one token
//! ([`crate::spatial::decode_engine::numerics::gdn_row`]), sixteen elements a cycle.
//!
//! A row streams in as `N/16` beats of sixteen f32 elements, the row's value on its first beat. Each beat feeds
//! sixteen lanes of two dots, element `16·beat + l` into lane `l` in index order — the key's dot and the query's — and
//! is kept. After the last beat the lanes fold by the pairwise tree, the coefficient `beta·(v − decay·(row·k))` and the
//! read-out `(decay·(row·q) + coef·(k·q))·scale` follow, and the kept row streams back out updated, sixteen elements
//! a cycle, each `decay·m + coef·k` rounded to binary16 and back when the state is stored in f16; the read-out rides
//! the last beat.
//!
//! The state's codec is [`super::fp32`]'s binary16 conversions.
//!
//! `logos_gdn_lazy_row` is the row under a lazily flushed memory ([`crate::spatial::decode_engine::numerics::gdn_read`],
//! [`gdn_flush`](crate::spatial::decode_engine::numerics::gdn_flush)): the row streams in with its carried value and
//! its logged values, its dot with the head's folded query in sixteen lanes and the tree gives the read-out
//! `(row·y + carried)·scale`; when the log is full every logged update is replayed over the held row in order — the
//! row's dot with the token's key, the coefficient, the rank-one update, no rounding between — and the row streams back
//! out stored once.
//!
//! `logos_gdn_read_p` is the lazily flushed read-out for the user clock, on the pipelined units: rows stream in beat by
//! beat, row-major, up to a beat a clock, and out comes each row's `(row·y + carried)·scale`, the rows in their order.
//! It gathers rows four at a time into one of two buffers and sends each group through its sixteen lanes beat-major —
//! the four rows' first beats, then their second — so the add's latency of four clocks is exactly the time between a
//! lane's terms for one row, and each lane's sum feeds back into its own add; a row's lanes then fold through the
//! pipelined tree. A group takes its query and scale with its first beat, so heads stream back to back.
//!
//! `logos_gdn_flush_p` is the flush for the user clock: a head's log written beat by beat, then its rows in fours, each
//! with its logged values. For every logged token the group's beats pass through the same sixteen lanes to the tree for
//! each row's dot with the token's key, the coefficient `beta·(v − decay·(row·k))` follows on the pipelined units, and a
//! second pass writes `decay·m + coef·k` back over the held rows; after the last token the rows stream out, stored once.
//!
//! `logos_gdn_fold_p` folds a head's query back through its log for the user clock ([`gdn_fold`](
//! crate::spatial::decode_engine::numerics::gdn_fold)): every logged token's decay through the pipelined exponential first,
//! then the tokens newest first — the key's dot with the folded query a beat each four clocks, so each lane's sum comes
//! back to its add as the next term enters, the tree, the gate's product, then a beat a clock the values collected and
//! the query decayed past the token.

/// The row engine with the f32 units and the codec it is built from.
pub fn gdn_verilog() -> String {
    format!("{}{}\n{}", super::ram::RAMSTYLE, super::fp32::fp32_verilog(), GDN_VERILOG)
}

/// The lines the recurrence's coordinator and each head lane ask ahead of those they have taken (`OUTSTANDING`) unless a
/// build gives more ([`super::engine::Streaming::with_recurrence_credit`]): behind a memory that answers `latency` clocks
/// after an ask, a credit of `c` lines reads at most `c / latency` lines a clock. Each line queue is the power of two past
/// the credit, 64 lines at least.
pub const LANE_CREDIT: usize = 32;

/// The most lines [`LANE_CREDIT`] may be raised to: a channel's read FIFO holds 512 lines, and the channel multiplexer
/// takes 30 runs a channel.
pub const LANE_CREDIT_MAX: usize = 256;

/// Clocks the head lanes' fold (`logos_gdn_fold_m`) takes from a read-back's address (`rd_bank`, `rd_slot`, `rd_beat`)
/// to its words: the address registered beside the banks, then read.
pub const FOLD_M_READ_LATENCY: usize = 2;

/// The recurrence on head lanes (`logos_gdn_lanes`): a coordinator (the side's MODE 1) that counts, appends and reads
/// the log once into each lane's copy of it, and L head lanes (`logos_gdn_lane`), lane k taking value heads k, k + L, …
/// a batch at a time through the fold of several heads at once and its own memory port; emitted beside
/// [`gdn_stream_verilog`]'s units, the side and the fold among them.
pub fn lanes_verilog() -> String {
    let lane = GDN_LANE_VERILOG.replace("@FADD@", &FADD_LATENCY.to_string()).replace("@F16@", &F32_TO_F16_LATENCY.to_string());
    format!("{}{GDN_LANES_VERILOG}\n{lane}\n{}", super::ram::RAMSTYLE, super::ram::bram_verilog(&[512]))
}

/// [`lanes_verilog`] under a trailing flush of `window` tokens a window: the coordinator keeps the log as a ring of two
/// windows and hands the lanes the share of rows each row replays; lane k is `logos_gdn_lane_t`, its window's flush
/// `logos_gdn_flush_m`; each copy of the log holds two windows. The lanes' `TMAX` is the log's two windows.
pub fn lanes_trailing_verilog(window: usize) -> String {
    assert!(window > 0, "a trailing flush replays a window of tokens");
    let edit = |text: &str, from: &str, to: &str| -> String {
        assert_eq!(text.matches(from).count(), 1, "the lanes' `{from}`");
        text.replacen(from, to, 1)
    };
    let mut top = GDN_LANES_VERILOG.to_string();
    top = edit(&top, "// A recurrence job's token rows, each a lazily flushed Gated-DeltaNet block, on L head lanes.", &format!(
        "// A recurrence job's token rows, each a Gated-DeltaNet block flushed a window of {window} tokens behind, on L head lanes\n// (logos_gdn_lane_t, the coordinator's ring and each row's share handed on as co_lo)."));
    top = edit(&top, "  wire co_ready, lb_we, co_flush, co_busy, co_idle, co_reading;", "  wire co_ready, lb_we, co_flush, co_busy, co_idle, co_reading;\n  wire [15:0] co_lo;");
    top = edit(&top, "  wire [39:0] log_need = {32'd0, tokens} * {14'd0, entry[31:6]};", "  wire [39:0] log_need = {31'd0, tokens, 1'b0} * {14'd0, entry[31:6]};");
    top = edit(&top, ".MODE(1), .LBA(LBA)) co(", &format!(".MODE(1), .LBA(LBA), .WINDOW({window})) co("));
    top = edit(&top, "    .log_first(log_first), .log_lines(log_lines), .co_reading(co_reading)", "    .log_first(log_first), .log_lines(log_lines), .co_lo(co_lo), .co_reading(co_reading)");
    top = edit(&top, "      logos_gdn_lane #(.SRL(SRL), .HD(HD), .TMAX(TMAX), .SA(SA), .OUTSTANDING(OUTSTANDING), .W(W), .STEP(L), .LBA(LBA), .FLUSHES(FLUSHES), .COPY_LINES(LOG_LINES))",
        &format!("      logos_gdn_lane_t #(.SRL(SRL), .HD(HD), .TMAX(TMAX), .SA(SA), .OUTSTANDING(OUTSTANDING), .W(W), .STEP(L), .LBA(LBA), .WINDOW({window}), .COPY_LINES(LOG_LINES))"));
    top = edit(&top, ".in_count(co_count), .in_flush(co_flush), .in_par(c_par)", ".in_count(co_count), .in_flush(co_flush), .in_from_row(co_lo), .in_par(c_par)");
    let lane = GDN_LANE_T_VERILOG.replace("@FADD@", &FADD_LATENCY.to_string()).replace("@F16@", &F32_TO_F16_LATENCY.to_string());
    format!("{}{top}\n{lane}\n{}\n{}", super::ram::RAMSTYLE, flush_m_verilog(), super::ram::bram_verilog(&[512]))
}

/// `logos_gdn_gate_p`, a head lane's read-outs gated and normed head by head as the machine's gated norm makes them
/// ([`crate::spatial::decode_engine::machine::DeltaNorm`]): each head's sum of squares in sixteen lanes as it comes in,
/// its r through the tree, the product with recip(HD), the sum with epsilon and the reciprocal square root, then each
/// element's `((o·r)·w)·silu(z)` a clock at a time, on the pipelined units.
pub fn gate_verilog() -> String {
    GDN_GATE_VERILOG
        .replace("@FMUL@", &FMUL_LATENCY.to_string())
        .replace("@FADD@", &FADD_LATENCY.to_string())
        .replace("@TREE16@", &TREE16_LATENCY.to_string())
        .replace("@RSQRT@", &RSQRT_LATENCY.to_string())
        .replace("@EXP@", &EXP_LATENCY.to_string())
        .replace("@RECIP@", &RECIP_LATENCY.to_string())
}

/// [`lanes_verilog`]'s or [`lanes_trailing_verilog`]'s head lanes with each lane's read-outs gated and normed where they
/// are made, as a recurrence job that carries a [`crate::spatial::decode_engine::machine::DeltaNorm`] asks
/// (`Streaming::with_delta_norm`): the job's `norm`, its epsilon, the gate's slot (each row's beat of it, from the rows
/// module, `z_beat`) and the norm's weight's first line (`w_line`) come in beside the job's other fields. The lanes fetch
/// the weight's `hd / 16` lines on the coordinator's port at the job's start, before its first row, and give every lane
/// each as it comes. A lane's loader reads each head's gate beats after its query's into the gate's slot of the head
/// (`logos_gdn_gate_p`, heads taking its NZ slots in turn), loading a batch only while the heads loaded and not yet gated
/// leave the batch room; its read-outs go through the gate, which writes the store; a row is done only once the gate has
/// written its every head. A job without a norm runs as the lanes run it.
pub fn delta_normed_lanes(lanes: &str, hd: usize, fp: FpUnit) -> String {
    let edit = |text: &str, from: &str, to: &str| -> String {
        assert_eq!(text.matches(from).count(), 1, "the normed lanes' `{from}`");
        text.replacen(from, to, 1)
    };
    // A lane loads a batch (at most FADD heads) only while its heads loaded and not yet gated leave the batch room in the
    // gate's 16 slots; two batches loaded ahead of the stream and one in the gate always fit, so the loader never waits on
    // heads that wait on it.
    assert!(16 >= 3 * FADD_LATENCY, "the gate's slots hold two batches loaded ahead and one gated");
    let trailing = lanes.contains("module logos_gdn_lane_t #(");
    assert!(trailing != lanes.contains("module logos_gdn_lane #("), "one kind of head lane");
    let rn = numerics::recip(fp, hd as f32).to_bits();
    let mut t = lanes.to_string();
    // The lanes: the job's norm fields, the weight's fetch, each row's gate beat, each lane's norm ports.
    t = edit(&t, "   input [SA-1:0] gt_beat,\n", "   input [SA-1:0] gt_beat, input norm, input [31:0] eps, input [SA-1:0] z_beat, input [31:0] w_line,\n");
    t = edit(&t, ".rq_valid(rq_valid), .rq_ready(rq_ready), .rq_line(rq_line),\n    .rs_valid(rs_valid), .rs_data(rs_data),",
        ".rq_valid(co_rq_valid), .rq_ready(co_rq_ready), .rq_line(co_rq_line),\n    .rs_valid(co_rs_valid), .rs_data(rs_data),");
    t = edit(&t, "  reg [SA-1:0] c_q, c_out;\n", r#"  reg [SA-1:0] c_q, c_out, c_z;
  // The norm's weight, fetched at the job's start on the coordinator's port before its first row (HD / 16 lines from
  // w_line, asked and taken in order), each line given to every lane as it comes (wf_we, its beat wf_got).
  localparam [15:0] WBEATS = HD / 16;
  reg wf_on;
  reg [15:0] wf_asked, wf_got;
  wire co_rq_valid, co_rs_valid;
  wire [31:0] co_rq_line;
  wire co_rq_ready = !wf_on && rq_ready;
  assign rq_valid = wf_on ? wf_asked != WBEATS : co_rq_valid;
  assign rq_line = wf_on ? w_line + {16'd0, wf_asked} : co_rq_line;
  assign co_rs_valid = !wf_on && rs_valid;
  wire wf_we = wf_on && rs_valid;
  always @(posedge clk) begin
    if (wf_on && rq_valid && rq_ready) wf_asked <= wf_asked + 16'd1;
    if (wf_we) wf_got <= wf_got + 16'd1;
    if (wf_we && wf_got + 16'd1 == WBEATS) wf_on <= 1'b0;
    if (start) begin wf_on <= norm; wf_asked <= 16'd0; wf_got <= 16'd0; end
    if (rst) wf_on <= 1'b0;
  end
"#);
    t = edit(&t, "c_q <= q_beat; c_out <= out_beat;", "c_q <= q_beat; c_out <= out_beat; c_z <= z_beat;");
    t = edit(&t, "  wire co_go = job && !fault && row_valid &&", "  wire co_go = job && !wf_on && !fault && row_valid &&");
    t = edit(&t, "&& written && !co_busy;\n", "&& written && !co_busy && !wf_on;\n");
    t = edit(&t, ".in_out_beat(c_out), .first(FIRST),",
        ".in_out_beat(c_out), .norm(norm), .eps(eps), .in_z_beat(c_z), .w_we(wf_we), .w_wa(wf_got[7:0]), .w_wd(rs_data),\n        .first(FIRST),");
    // A lane: the norm's ports; each head's gate beats read after its query's into the head's slot of the gate; the
    // read-outs through the gate.
    t = edit(&t, "COPY_LINES = 4096, SRL = 1)", "COPY_LINES = 4096, SRL = 1, NZ = 16)");
    t = edit(&t, "   output reg st_we, output reg [SA-1:0] st_wa, output reg [511:0] st_wd,\n   output [LBA:0] lb_ra, input [511:0] lb_rd,",
        "   output st_we, output [SA-1:0] st_wa, output [511:0] st_wd,\n   input norm, input [31:0] eps, input [SA-1:0] in_z_beat, input w_we, input [7:0] w_wa, input [511:0] w_wd,\n   output [LBA:0] lb_ra, input [511:0] lb_rd,");
    t = edit(&t, "  wire can_load = !f_busy && !f_start && !f_run && ", &format!(
        r#"  // The gate's slots: heads take them in turn as they are loaded (zn, the heads loaded), each free again once the gate has
  // written the head (g_gated); a head's gate beats are read after its query's, into its slot.
  localparam ZB = $clog2(NZ), GB = BEATS > 1 ? $clog2(BEATS) : 1;
  localparam [15:0] SLOTS = NZ;
  localparam [31:0] RN = 32'h{rn:08x};
  reg [15:0] zn;
  wire [15:0] g_gated;
  wire g_busy;
  wire [15:0] q_end = norm ? 2 * BEATS : BEATS;
  wire [15:0] z_slot = zn + {{8'd0, q_lo_slot}}, z_at = q_lo_at - BEATS;
  wire g_room = !norm || zn - g_gated + {{8'd0, bs}} <= SLOTS;
  wire can_load = g_room && !f_busy && !f_start && !f_run && "#));
    t = edit(&t, "  assign st_rq = ls == L_Q && qj != BEATS;", "  assign st_rq = ls == L_Q && qj != q_end;");
    t = edit(&t, "      L_Q: if (qj == BEATS) begin", "      L_Q: if (qj == q_end) begin");
    t = edit(&t, "        if (lslot + 8'd1 == bs || lh + STEP >= nv) ls <= L_FOLD;",
        "        if (lslot + 8'd1 == bs || lh + STEP >= nv) begin ls <= L_FOLD; zn <= zn + {8'd0, lslot} + 16'd1; end");
    t = edit(&t, "    if (q_lo) begin f_q_wr <= 1'b1;", "    if (q_lo && q_lo_at < BEATS) begin f_q_wr <= 1'b1;");
    t = edit(&t, "f_start <= 1'b0; st_we <= 1'b0; q_pending", "f_start <= 1'b0; o_we <= 1'b0; q_pending");
    t = edit(&t, "begin st_we <= 1'b1; st_wa <= o_wa[SA-1:0]; st_wd <= {r_out, o_beat[479:0]}; end",
        "begin o_we <= 1'b1; o_wq <= o_wa[SA-1:0]; o_wd <= {r_out, o_beat[479:0]}; end");
    t = edit(&t, "  always @(posedge clk) begin\n    f_log_wr <= 1'b0; f_q_wr <= 1'b0; f_start <= 1'b0;", r#"  // The read-outs into the store: through the gate (logos_gdn_gate_p), which norms each head by the weight and gates it
  // by the head's gate beats, when the job has a norm; as they are made otherwise.
  reg o_we;
  reg [SA-1:0] o_wq;
  reg [511:0] o_wd;
  wire g_we;
  wire [SA-1:0] g_wa;
  wire [511:0] g_wd;
  wire [15:0] g_zwa = {z_slot[ZB-1:0], z_at[GB-1:0]};
  logos_gdn_gate_p #(.HD(HD), .SA(SA), .NZ(NZ), .RN(RN)) gate(.clk(clk), .rst(rst), .eps(eps), .w_we(w_we), .w_wa(w_wa),
    .w_wd(w_wd), .z_we(q_lo && q_lo_at >= BEATS), .z_wa(g_zwa), .z_wd(st_rd), .e_v(norm && r_ov),
    .e_o(r_out), .e_at(o_wa[SA-1:0]), .st_we(g_we), .st_wa(g_wa), .st_wd(g_wd), .gated(g_gated), .busy(g_busy));
  assign st_we = norm ? g_we : o_we;
  assign st_wa = norm ? g_wa : o_wq;
  assign st_wd = norm ? g_wd : o_wd;
  always @(posedge clk) begin
    f_log_wr <= 1'b0; f_q_wr <= 1'b0; f_start <= 1'b0;"#);
    if trailing {
        // Rows in flight: each row's gate beat with its other fields; a row done only once the gate has written every head
        // given it up to the row's last (gin, the heads given; o_mark0/1, the count once each row's last is given).
        t = edit(&t, "  `LOGOS_RAMSTYLE(2) reg [SA-1:0] e_out [0:1];\n", "  `LOGOS_RAMSTYLE(2) reg [SA-1:0] e_out [0:1];\n  `LOGOS_RAMSTYLE(2) reg [SA-1:0] e_z [0:1];\n");
        t = edit(&t, "e_out[in_n[0]] <= in_out_beat;", "e_out[in_n[0]] <= in_out_beat; e_z[in_n[0]] <= in_z_beat;");
        t = edit(&t, "  reg [SA-1:0] l_q;\n", "  reg [SA-1:0] l_q, l_z;\n");
        t = edit(&t, "l_q <= e_q[ld_n[0]];", "l_q <= e_q[ld_n[0]]; l_z <= e_z[ld_n[0]];");
        t = edit(&t, "  wire [31:0] q_ra = l_q + ((lg * HD) >> 4) + qj;", "  wire [31:0] q_ra = qj < BEATS ? l_q + ((lg * HD) >> 4) + qj : l_z + ((lh * HD) >> 4) + qj - BEATS;");
        t = edit(&t, "  wire dn_past = o_n != dn_n && st_n != dn_n && (dn_n[0] ? xr1 == 4'd0 && w_np1 == 5'd0 : xr0 == 4'd0 && w_np0 == 5'd0);",
            "  reg [15:0] gin, o_mark0, o_mark1;\n  wire dn_past = o_n != dn_n && st_n != dn_n && (dn_n[0] ? xr1 == 4'd0 && w_np1 == 5'd0 : xr0 == 4'd0 && w_np0 == 5'd0)\n    && (!norm || g_gated - (dn_n[0] ? o_mark1 : o_mark0) < 16'h8000);");
        t = edit(&t, "  assign busy = in_n != dn_n || w_n != 5'd0;", "  assign busy = in_n != dn_n || w_n != 5'd0 || g_busy;");
        t = edit(&t, "      if (o_row + 16'd1 == HD_ROWS) begin\n        o_row <= 16'd0;\n        if (oh + STEP >= nv) begin\n          o_n <= o_n + 8'd1;",
            "      if (o_row + 16'd1 == HD_ROWS) begin\n        o_row <= 16'd0;\n        gin <= gin + 16'd1;\n        if (oh + STEP >= nv) begin\n          if (o_n[0]) o_mark1 <= gin + 16'd1; else o_mark0 <= gin + 16'd1;\n          o_n <= o_n + 8'd1;");
        t = edit(&t, "    else if (o_set && oh >= nv) begin o_n <= o_n + 8'd1; o_set <= 1'b0; end",
            "    else if (o_set && oh >= nv) begin\n      if (o_n[0]) o_mark1 <= gin; else o_mark0 <= gin;\n      o_n <= o_n + 8'd1; o_set <= 1'b0;\n    end");
        t = edit(&t, "in_n <= 8'd0; ld_n <= 8'd0; st_n <= 8'd0; o_n <= 8'd0; dn_n <= 8'd0;", "in_n <= 8'd0; ld_n <= 8'd0; st_n <= 8'd0; o_n <= 8'd0; dn_n <= 8'd0; zn <= 16'd0; gin <= 16'd0;");
    } else {
        // One row at a time: its gate beat with its other fields; the row ends only once the gate has written every head.
        t = edit(&t, "  reg [SA-1:0] q_beat, out_beat;\n", "  reg [SA-1:0] q_beat, out_beat, z_beat;\n");
        t = edit(&t, "out_beat <= in_out_beat;", "out_beat <= in_out_beat; z_beat <= in_z_beat;");
        t = edit(&t, "  wire [31:0] q_ra = q_beat + ((lg * HD) >> 4) + qj;", "  wire [31:0] q_ra = qj < BEATS ? q_beat + ((lg * HD) >> 4) + qj : z_beat + ((lh * HD) >> 4) + qj - BEATS;");
        t = edit(&t, "  assign busy = running || w_n != 5'd0;", "  assign busy = running || w_n != 5'd0 || g_busy;");
        t = edit(&t, "    if (running && ls == L_DONE && ss == S_DONE && outs_done && (!flush || xh >= nv) && w_n == 5'd0) begin",
            "    if (running && ls == L_DONE && ss == S_DONE && outs_done && (!flush || xh >= nv) && w_n == 5'd0 && !g_busy) begin");
        t = edit(&t, "      running <= 1'b0; ls <= L_IDLE; ss <= S_IDLE; f_run <= 1'b0; t_wr <= {QB{1'b0}};",
            "      running <= 1'b0; ls <= L_IDLE; ss <= S_IDLE; f_run <= 1'b0; zn <= 16'd0; t_wr <= {QB{1'b0}};");
    }
    format!("{t}\n{}", gate_verilog())
}

/// [`rows_verilog`] giving each row's beat of one more vector, the gate of a recurrence job's norm (`z_slot`, `z_beat`),
/// for [`delta_normed_lanes`].
pub fn delta_normed_rows(rows: &str) -> String {
    let edit = |text: &str, from: &str, to: &str| -> String {
        assert_eq!(text.matches(from).count(), 1, "the normed rows' `{from}`");
        text.replacen(from, to, 1)
    };
    let mut t = rows.to_string();
    t = edit(&t, "   input [4:0] gt_slot,\n", "   input [4:0] gt_slot, input [4:0] z_slot,\n");
    t = edit(&t, "output reg [SA-1:0] gt_beat);", "output reg [SA-1:0] gt_beat, output reg [SA-1:0] z_beat);");
    t = edit(&t, "  reg [SA-1:0] q_st, out_st, k_st, v_st, ld_st, gt_st;", "  reg [SA-1:0] q_st, out_st, k_st, v_st, ld_st, gt_st, z_st;");
    t = edit(&t, "              sg = slott[gt_slot];", "              sg = slott[gt_slot], sz = slott[z_slot];");
    t = edit(&t, "      ld_beat <= sl[SA-1:0]; gt_beat <= sg[SA-1:0];", "      ld_beat <= sl[SA-1:0]; gt_beat <= sg[SA-1:0]; z_beat <= sz[SA-1:0];");
    t = edit(&t, "      ld_st <= sl[32+SA-1:32]; gt_st <= sg[32+SA-1:32];", "      ld_st <= sl[32+SA-1:32]; gt_st <= sg[32+SA-1:32]; z_st <= sz[32+SA-1:32];");
    t = edit(&t, "      ld_beat <= ld_beat + ld_st; gt_beat <= gt_beat + gt_st;", "      ld_beat <= ld_beat + ld_st; gt_beat <= gt_beat + gt_st; z_beat <= z_beat + z_st;");
    t
}

/// `logos_gdn_rows`, a recurrence job's token rows given on one after another as the descriptors `logos_gdn_lanes` takes
/// (and the side, a row at a time): each row's region from a copy of the rows table, each vector's store beat from a copy
/// of the slots table, the copies written as the host writes the tables.
pub fn rows_verilog() -> String {
    GDN_ROWS_VERILOG.to_string()
}

/// The streaming units for the user clock with the f32 units and the pipelined exponential they are built from:
/// `logos_gdn_read_p`, a lazily flushed head's read-out a beat a clock; `logos_gdn_flush_p`, its rows' flush;
/// `logos_gdn_fold_p`, its query folded back through its log.
pub fn gdn_stream_verilog() -> String {
    format!("{}{}\n{}", super::ram::RAMSTYLE, super::vector::vector_verilog(), filled(&format!("{GDN_STREAM_VERILOG}{GDN_WIDE_VERILOG}{GDN_FOLD_M_VERILOG}")))
}

/// `logos_gdn_flush_m`, a trailing flush's shares of several heads at once in two banks, emitted beside
/// [`gdn_stream_verilog`]'s units, which it is built from.
pub fn flush_m_verilog() -> String {
    format!("{}{}", super::ram::RAMSTYLE, filled(GDN_FLUSH_M_VERILOG))
}

/// `text` with every unit's latency in its place.
fn filled(text: &str) -> String {
    // Clocks after a beat's operands: its products, its lanes' sums, a row's dot out of the tree; then the read-out's
    // carried value and scale, or the flush's logged value and coefficient; an update's beat.
    let products = FMUL_LATENCY;
    let tree = products + FADD_LATENCY + TREE16_LATENCY;
    let carried = tree + FADD_LATENCY;
    let lag = carried + FMUL_LATENCY;
    let value = tree + FMUL_LATENCY;
    let coef = value + FADD_LATENCY + FMUL_LATENCY;
    let updated = FMUL_LATENCY + FADD_LATENCY;
    let bc = tree + FMUL_LATENCY;
    let folded = FMUL_LATENCY + FADD_LATENCY + FMUL_LATENCY;
    let units = [
        ("@PRODUCTS@", products),
        ("@TREE@", tree),
        ("@CARRIED@", carried),
        ("@LAG@", lag),
        ("@VALUE@", value),
        ("@COEF@", coef),
        ("@UPDATED@", updated),
        ("@BC@", bc),
        ("@FOLDED@", folded),
        ("@FADD@", FADD_LATENCY),
        ("@FMUL@", FMUL_LATENCY),
        ("@EXP@", EXP_LATENCY),
        ("@F16@", F32_TO_F16_LATENCY),
    ]
    .iter()
    .fold(text.to_string(), |t, (k, v)| t.replace(k, &v.to_string()));
    units
}

// `logos_gdn_read_p` sends a group's rows in turn, each lane's sum coming back to its add as the row's next term
// enters: a group is as many rows as the add's clocks.
const _: () = assert!(FADD_LATENCY == 4, "logos_gdn_read_p's groups of four rows are the add's four clocks");

use super::fp32::{F32_TO_F16_LATENCY, FADD_LATENCY, FMUL_LATENCY, TREE16_LATENCY};
use super::super::numerics::{self, FpUnit};
use super::vector::{EXP_LATENCY, RECIP_LATENCY, RSQRT_LATENCY};

const GDN_VERILOG: &str = r#"// One value row of N elements through one token, sixteen elements a cycle.
module logos_gdn_row_engine #(parameter N = 128)
  (input clk, input rst,
   input [32*N-1:0] k, input [32*N-1:0] q, input [31:0] decay, input [31:0] beta, input [31:0] kq, input [31:0] scale,
   input f16,
   input in_valid, input in_first, input [511:0] in_data, input [31:0] in_v,
   output reg out_valid, output reg out_last, output reg [511:0] out_data, output reg [31:0] out_read);
  localparam [15:0] BEATS = N / 16;
  reg [32*N-1:0] kept;
  reg [511:0] acc_k, acc_q;
  reg [31:0] v;
  reg [15:0] beat;
  reg [15:0] out_beat;
  reg emitting;
  // This beat's lane products and sums, against the beat's sixteen key and query elements.
  wire [511:0] beat_k = k[512*beat +: 512], beat_q = q[512*beat +: 512];
  wire [511:0] next_k, next_q;
  genvar l;
  generate
    for (l = 0; l < 16; l = l + 1) begin : lane
      wire [31:0] pk, pq;
      logos_fmul mk(.a(in_data[32*l +: 32]), .b(beat_k[32*l +: 32]), .y(pk));
      logos_fmul mq(.a(in_data[32*l +: 32]), .b(beat_q[32*l +: 32]), .y(pq));
      logos_fadd ak(.a(in_first ? 32'd0 : acc_k[32*l +: 32]), .b(pk), .y(next_k[32*l +: 32]));
      logos_fadd aq(.a(in_first ? 32'd0 : acc_q[32*l +: 32]), .b(pq), .y(next_q[32*l +: 32]));
    end
  endgenerate
  // The folded dots, the coefficient and the read-out.
  wire [31:0] dk, dq, u, vu, coef, dsq, ckq, sum, read;
  logos_tree16 tk(.lanes(acc_k), .sum(dk));
  logos_tree16 tq(.lanes(acc_q), .sum(dq));
  logos_fmul m0(.a(decay), .b(dk), .y(u));
  logos_fadd s0(.a(v), .b({~u[31], u[30:0]}), .y(vu));
  logos_fmul m1(.a(beta), .b(vu), .y(coef));
  logos_fmul m2(.a(decay), .b(dq), .y(dsq));
  logos_fmul m3(.a(coef), .b(kq), .y(ckq));
  logos_fadd s1(.a(dsq), .b(ckq), .y(sum));
  logos_fmul m4(.a(sum), .b(scale), .y(read));
  // The update of the out beat's sixteen kept elements.
  wire [511:0] old = kept[512*out_beat +: 512], key = k[512*out_beat +: 512];
  wire [511:0] updated;
  generate
    for (l = 0; l < 16; l = l + 1) begin : upd
      wire [31:0] dm, ck, nm, back;
      wire [15:0] half;
      logos_fmul md(.a(decay), .b(old[32*l +: 32]), .y(dm));
      logos_fmul mc(.a(coef), .b(key[32*l +: 32]), .y(ck));
      logos_fadd an(.a(dm), .b(ck), .y(nm));
      logos_f32_to_f16 enc(.x(nm), .h(half));
      logos_f16_to_f32 dec(.h(half), .x(back));
      assign updated[32*l +: 32] = f16 ? back : nm;
    end
  endgenerate
  always @(posedge clk) begin
    if (rst) begin
      beat <= 16'd0; out_beat <= 16'd0; emitting <= 1'b0; out_valid <= 1'b0; out_last <= 1'b0;
    end else begin
      out_valid <= 1'b0;
      out_last <= 1'b0;
      if (in_valid) begin
        acc_k <= next_k;
        acc_q <= next_q;
        kept[512*beat +: 512] <= in_data;
        if (in_first) v <= in_v;
        if (beat == BEATS - 1) begin beat <= 16'd0; emitting <= 1'b1; out_beat <= 16'd0; end
        else beat <= beat + 16'd1;
      end else if (emitting) begin
        out_valid <= 1'b1;
        out_data <= updated;
        if (out_beat == BEATS - 1) begin
          out_last <= 1'b1; out_read <= read; emitting <= 1'b0; out_beat <= 16'd0;
        end else out_beat <= out_beat + 16'd1;
      end
    end
  end
endmodule

// One value row of N elements under a lazily flushed memory: its read-out through the folded query y, and when
// flushing, up to T_MAX logged updates replayed over it in order and the row stored once.
module logos_gdn_lazy_row #(parameter N = 128, T_MAX = 16)
  (input clk, input rst,
   input [32*N-1:0] y, input [31:0] scale, input f16, input [7:0] entries, input flush,
   input log_wr, input [7:0] log_at, input [32*N-1:0] log_k, input [31:0] log_decay, input [31:0] log_beta,
   input in_valid, input in_first, input [511:0] in_data, input [31:0] in_carried, input [32*T_MAX-1:0] in_values,
   output reg out_valid, output reg out_last, output reg [511:0] out_data, output reg [31:0] out_read, output busy);
  localparam [15:0] BEATS = N / 16;
  localparam IN = 3'd0, READ = 3'd1, DOT = 3'd2, COEF = 3'd3, UPD = 3'd4, OUT = 3'd5;
  reg [2:0] state;
  reg [32*N-1:0] kept;
  `LOGOS_RAMSTYLE_ASYNC(T_MAX) reg [32*N-1:0] ks [0:T_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(T_MAX) reg [31:0] decays [0:T_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(T_MAX) reg [31:0] betas [0:T_MAX-1];
  reg [32*T_MAX-1:0] values;
  reg [511:0] lanes;
  reg [31:0] carried, coef;
  reg [15:0] beat;
  reg [7:0] e;
  assign busy = (state != IN);
  // The input beat's products with the folded query, and a held beat's with the token's key.
  wire [511:0] y_beat = y[512*beat +: 512];
  wire [511:0] row_beat = kept[512*beat +: 512];
  wire [511:0] k_beat = ks[e][512*beat +: 512];
  wire [511:0] in_next, dot_next, upd;
  genvar l;
  generate
    for (l = 0; l < 16; l = l + 1) begin : lane
      wire [31:0] py, pk, dm, ck;
      logos_fmul my(.a(in_data[32*l +: 32]), .b(y_beat[32*l +: 32]), .y(py));
      logos_fadd ay(.a(in_first ? 32'd0 : lanes[32*l +: 32]), .b(py), .y(in_next[32*l +: 32]));
      logos_fmul mk(.a(row_beat[32*l +: 32]), .b(k_beat[32*l +: 32]), .y(pk));
      logos_fadd ak(.a(beat == 16'd0 ? 32'd0 : lanes[32*l +: 32]), .b(pk), .y(dot_next[32*l +: 32]));
      logos_fmul md(.a(decays[e]), .b(row_beat[32*l +: 32]), .y(dm));
      logos_fmul mc(.a(coef), .b(k_beat[32*l +: 32]), .y(ck));
      logos_fadd au(.a(dm), .b(ck), .y(upd[32*l +: 32]));
    end
  endgenerate
  // The folded lanes: the read-out, or a token's coefficient.
  wire [31:0] folded, with_carried, read, u, vu, c;
  logos_tree16 tf(.lanes(lanes), .sum(folded));
  logos_fadd ac(.a(folded), .b(carried), .y(with_carried));
  logos_fmul ms(.a(with_carried), .b(scale), .y(read));
  logos_fmul mu(.a(decays[e]), .b(folded), .y(u));
  logos_fadd sv(.a(values[32*e +: 32]), .b({~u[31], u[30:0]}), .y(vu));
  logos_fmul mb(.a(betas[e]), .b(vu), .y(c));
  // The held row as the state stores it.
  wire [511:0] stored;
  generate
    for (l = 0; l < 16; l = l + 1) begin : st
      wire [15:0] half;
      wire [31:0] back;
      logos_f32_to_f16 enc(.x(row_beat[32*l +: 32]), .h(half));
      logos_f16_to_f32 dec(.h(half), .x(back));
      assign stored[32*l +: 32] = f16 ? back : row_beat[32*l +: 32];
    end
  endgenerate
  always @(posedge clk) begin
    out_valid <= 1'b0;
    out_last <= 1'b0;
    if (log_wr) begin ks[log_at] <= log_k; decays[log_at] <= log_decay; betas[log_at] <= log_beta; end
    if (rst) begin state <= IN; beat <= 16'd0; end
    else case (state)
      IN: if (in_valid) begin
        lanes <= in_next;
        kept[512*beat +: 512] <= in_data;
        if (in_first) begin carried <= in_carried; values <= in_values; end
        if (beat == BEATS - 1) begin beat <= 16'd0; state <= READ; end else beat <= beat + 16'd1;
      end
      READ: begin
        out_read <= read;
        if (flush && entries != 8'd0) begin e <= 8'd0; state <= DOT; end
        else begin out_valid <= 1'b1; out_last <= 1'b1; state <= IN; end
      end
      DOT: begin
        lanes <= dot_next;
        if (beat == BEATS - 1) begin beat <= 16'd0; state <= COEF; end else beat <= beat + 16'd1;
      end
      COEF: begin coef <= c; state <= UPD; end
      UPD: begin
        kept[512*beat +: 512] <= upd;
        if (beat == BEATS - 1) begin
          beat <= 16'd0;
          if (e + 8'd1 == entries) state <= OUT; else begin e <= e + 8'd1; state <= DOT; end
        end else beat <= beat + 16'd1;
      end
      OUT: begin
        out_valid <= 1'b1;
        out_data <= stored;
        if (beat == BEATS - 1) begin out_last <= 1'b1; beat <= 16'd0; state <= IN; end else beat <= beat + 16'd1;
      end
    endcase
  end
endmodule
"#;

const GDN_STREAM_VERILOG: &str = r#"// A lazily flushed head's read-out a beat a clock: rows of N elements in beats of sixteen f32 (element 16·b + l in
// lane l of beat b), row-major, in fours, each row's carried value with its first beat and the head's folded query y
// and scale with each group's first beat; out, in the rows' order, (row·y + carried)·scale of each row.
module logos_gdn_read_p #(parameter N = 128)
  (input clk, input rst,
   input [32*N-1:0] y, input [31:0] scale,
   input in_valid, output in_ready, input [511:0] in_beat, input [31:0] in_carried,
   output reg out_valid, output reg [31:0] out_read);
  localparam [15:0] BEATS = N / 16;
  localparam [15:0] GB = 4 * BEATS;
  localparam LAG = @LAG@;
  // Two groups of four rows, row-major; each group's carried values, query and scale.
  `LOGOS_RAMSTYLE(2*GB) reg [511:0] gbuf [0:2*GB-1];
  `LOGOS_RAMSTYLE(8) reg [31:0] gcar [0:7];
  `LOGOS_RAMSTYLE_ASYNC(2) reg [32*N-1:0] gy [0:1];
  `LOGOS_RAMSTYLE(2) reg [31:0] gscale [0:1];
  reg [1:0] full;
  // The group being gathered and the group being sent.
  reg wr, rd, sending;
  reg [1:0] wrow, sj;
  reg [15:0] wbeat, sb;
  assign in_ready = !full[wr];
  wire take = in_valid && in_ready;
  wire group_in = take && wrow == 2'd3 && wbeat == BEATS - 1;
  wire group_out = sending && sj == 2'd3 && sb == BEATS - 1;
  always @(posedge clk) begin
    if (take) begin
      gbuf[{wr, wrow} * BEATS + wbeat] <= in_beat;
      if (wbeat == 16'd0) gcar[{wr, wrow}] <= in_carried;
      if (wrow == 2'd0 && wbeat == 16'd0) begin gy[wr] <= y; gscale[wr] <= scale; end
    end
    if (rst) begin
      full <= 2'b00; wr <= 1'b0; rd <= 1'b0; sending <= 1'b0; wrow <= 2'd0; wbeat <= 16'd0; sj <= 2'd0; sb <= 16'd0;
    end else begin
      if (take) begin
        if (wbeat == BEATS - 1) begin wbeat <= 16'd0; wrow <= wrow + 2'd1; end else wbeat <= wbeat + 16'd1;
        if (group_in) wr <= !wr;
      end
      if (group_in) full[wr] <= 1'b1;
      if (group_out) begin full[rd] <= 1'b0; rd <= !rd; sending <= 1'b0; end
      else if (!sending && full[rd]) begin sending <= 1'b1; sj <= 2'd0; sb <= 16'd0; end
      if (sending && !group_out) begin
        if (sj == 2'd3) begin sj <= 2'd0; sb <= sb + 16'd1; end else sj <= sj + 2'd1;
      end
    end
  end
  // The beat at hand, its query beat, and its tags: whether it opens or closes its row, the row's carried and scale.
  reg [511:0] beat_q, yb_q;
  reg v_q, first_q, last_q;
  reg [31:0] car_q, scale_q;
  always @(posedge clk) begin
    beat_q <= gbuf[{rd, sj} * BEATS + sb];
    yb_q <= gy[rd][512*sb +: 512];
    v_q <= !rst && sending;
    first_q <= sb == 16'd0;
    last_q <= sb == BEATS - 1;
    car_q <= gcar[{rd, sj}];
    scale_q <= gscale[rd];
  end
  // The tags through the multiply, the add, the tree, the carried value's add and the scale's multiply.
  reg [LAG:1] v_d, first_d, last_d;
  reg [31:0] car_d [1:LAG];
  reg [31:0] scale_d [1:LAG];
  integer i;
  always @(posedge clk) begin
    v_d <= rst ? {LAG{1'b0}} : {v_d[LAG-1:1], v_q};
    first_d <= {first_d[LAG-1:1], first_q};
    last_d <= {last_d[LAG-1:1], last_q};
    car_d[1] <= car_q;
    scale_d[1] <= scale_q;
    for (i = 2; i <= LAG; i = i + 1) begin car_d[i] <= car_d[i-1]; scale_d[i] <= scale_d[i-1]; end
  end
  // The lanes: a product a clock, each lane's sum for a row back at its add four clocks later as the row's next term
  // enters, a row's first term added to zero.
  wire [511:0] sums;
  genvar l;
  generate
    for (l = 0; l < 16; l = l + 1) begin : lane
      wire [31:0] p;
      logos_fmul_p m(.clk(clk), .a(beat_q[32*l +: 32]), .b(yb_q[32*l +: 32]), .y(p));
      logos_fadd_p a(.clk(clk), .a(first_d[@PRODUCTS@] ? 32'd0 : sums[32*l +: 32]), .b(p), .y(sums[32*l +: 32]));
    end
  endgenerate
  // A row's lanes leave the adds the clock after its last term entered them; the tree, the carried value, the scale.
  wire [31:0] dot, with_carried, read;
  logos_tree16_p t(.clk(clk), .lanes(sums), .sum(dot));
  logos_fadd_p c(.clk(clk), .a(dot), .b(car_d[@TREE@]), .y(with_carried));
  logos_fmul_p s(.clk(clk), .a(with_carried), .b(scale_d[@CARRIED@]), .y(read));
  always @(posedge clk) begin
    out_valid <= !rst && v_d[LAG] && last_d[LAG];
    out_read <= read;
  end
endmodule

// A lazily flushed head's rows through its log, on the pipelined units: the head's log written beat by beat (token
// log_at, beat log_beat: its key's beat and its values' beat), each token's decay exponent and gate with its beat 0,
// each decay through the pipelined exponential; then the head's rows of N elements in beats of sixteen f32, row-major,
// in fours, taken once every decay is in. Every logged update is replayed over each row in the log's order — the row's
// own value of each token, element r of the token's values for the head's row r — no rounding between, and the rows
// stream out row-major, each element stored once (rounded to binary16 and back when f16): a beat leaves the clock after
// each clock out_ready is high.
module logos_gdn_flush_p #(parameter N = 128, T_MAX = 16)
  (input clk, input rst,
   input log_wr, input [7:0] log_at, input [15:0] log_beat, input [511:0] log_k, input [511:0] log_v,
   input [31:0] log_decay, input [31:0] log_beta, input [7:0] entries, input f16,
   input in_valid, output in_ready, input [511:0] in_beat,
   output out_valid, input out_ready, output [511:0] out_beat, output [255:0] out_half);
  localparam [15:0] BEATS = N / 16;
  localparam [15:0] GB = 4 * BEATS;
  localparam IN = 3'd0, DOT = 3'd1, COEF = 3'd2, UPD = 3'd3, FILL = 3'd4, OUT = 3'd5;
  `LOGOS_RAMSTYLE(T_MAX*BEATS) reg [511:0] kmem [0:T_MAX*BEATS-1];
  `LOGOS_RAMSTYLE(T_MAX*BEATS) reg [511:0] vmem [0:T_MAX*BEATS-1];
  `LOGOS_RAMSTYLE_ASYNC(T_MAX) reg [31:0] decays [0:T_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(T_MAX) reg [31:0] betas [0:T_MAX-1];
  // Each token's decay: its exponent through the exponential as its beat 0 is written.
  wire [31:0] ex;
  reg [@EXP@:1] ex_v;
  reg [7:0] ex_at [1:@EXP@];
  reg [7:0] exps;
  logos_exp_p xp(.clk(clk), .x_in(log_decay), .y(ex));
  integer i;
  always @(posedge clk) begin
    if (log_wr) begin
      kmem[log_at * BEATS + log_beat] <= log_k;
      vmem[log_at * BEATS + log_beat] <= log_v;
      if (log_beat == 16'd0) betas[log_at] <= log_beta;
    end
    ex_v <= rst ? {@EXP@{1'b0}} : {ex_v[@EXP@-1:1], log_wr && log_beat == 16'd0};
    ex_at[1] <= log_at;
    for (i = 2; i <= @EXP@; i = i + 1) ex_at[i] <= ex_at[i-1];
    if (ex_v[@EXP@]) decays[ex_at[@EXP@]] <= ex;
    if (rst) exps <= 8'd0;
    else exps <= exps + ((log_wr && log_beat == 16'd0) ? 8'd1 : 8'd0) - (ex_v[@EXP@] ? 8'd1 : 8'd0);
  end
  // The group: four rows of beats, row-major, each row's coefficient; the rows taken since the log was written, the
  // token at hand, the beat the walk has reached.
  `LOGOS_RAMSTYLE(GB) reg [511:0] gbuf [0:GB-1];
  `LOGOS_RAMSTYLE(4) reg [31:0] coef [0:3];
  reg [15:0] row_base;
  reg [2:0] state;
  reg [7:0] e;
  reg [1:0] j;
  reg [15:0] b;
  reg [2:0] coefs;
  reg [15:0] written;
  assign in_ready = state == IN && exps == 8'd0;
  wire take = in_valid && in_ready;
  // A dot's or an update's beat: the row's beat, the token's key beat, and the tags.
  reg [511:0] x_q, k_q;
  reg v_q, upd_q, first_q, last_q;
  reg [1:0] j_q;
  reg [15:0] b_q;
  reg [31:0] coef_q, decay_q;
  always @(posedge clk) begin
    x_q <= gbuf[j * BEATS + b];
    k_q <= kmem[e * BEATS + b];
    v_q <= !rst && (state == DOT || state == UPD);
    upd_q <= state == UPD;
    first_q <= b == 16'd0;
    last_q <= b == BEATS - 1;
    j_q <= j;
    b_q <= b;
    coef_q <= coef[j];
    decay_q <= decays[e];
  end
  // The tags a clock at a time: [k] the beat's tag k clocks after its operands.
  reg [@COEF@:1] v_d, upd_d, first_d, last_d;
  reg [1:0] j_d [1:@COEF@];
  reg [15:0] b_d [1:@UPDATED@];
  always @(posedge clk) begin
    v_d <= rst ? {@COEF@{1'b0}} : {v_d[@COEF@-1:1], v_q};
    upd_d <= rst ? {@COEF@{1'b0}} : {upd_d[@COEF@-1:1], upd_q};
    first_d <= {first_d[@COEF@-1:1], first_q};
    last_d <= {last_d[@COEF@-1:1], last_q};
    j_d[1] <= j_q;
    b_d[1] <= b_q;
    for (i = 2; i <= @COEF@; i = i + 1) j_d[i] <= j_d[i-1];
    for (i = 2; i <= @UPDATED@; i = i + 1) b_d[i] <= b_d[i-1];
  end
  // The lanes: in a dot, each lane's sum for a row back at its add as the row's next term enters; in an update,
  // decay·m + coef·k.
  wire [511:0] sums, upd;
  genvar l;
  generate
    for (l = 0; l < 16; l = l + 1) begin : lane
      wire [31:0] p, dm, ck;
      logos_fmul_p m(.clk(clk), .a(x_q[32*l +: 32]), .b(k_q[32*l +: 32]), .y(p));
      logos_fadd_p a(.clk(clk), .a(first_d[@PRODUCTS@] ? 32'd0 : sums[32*l +: 32]), .b(p), .y(sums[32*l +: 32]));
      logos_fmul_p md(.clk(clk), .a(decay_q), .b(x_q[32*l +: 32]), .y(dm));
      logos_fmul_p mc(.clk(clk), .a(coef_q), .b(k_q[32*l +: 32]), .y(ck));
      logos_fadd_p au(.clk(clk), .a(dm), .b(ck), .y(upd[32*l +: 32]));
    end
  endgenerate
  // A row's lanes leave the adds the clock after its last term entered them; its dot through the tree, then its
  // coefficient beta·(v − decay·dot).
  wire [31:0] dot, u, vu, c;
  logos_tree16_p t(.clk(clk), .lanes(sums), .sum(dot));
  logos_fmul_p mu(.clk(clk), .a(decays[e]), .b(dot), .y(u));
  // The row's own value of the token, read two clocks before it meets decay·dot and its element taken the clock after:
  // element r of the token's values beat.
  wire [15:0] v_row = row_base + {14'd0, j_d[@VALUE@ - 2]};
  reg [511:0] v_line;
  reg [3:0] v_lane;
  reg [31:0] v_val;
  always @(posedge clk) begin
    v_line <= vmem[e * BEATS + (v_row >> 4)]; v_lane <= v_row[3:0];
    v_val <= v_line[32 * v_lane +: 32];
  end
  logos_fadd_p sv(.clk(clk), .a(v_val), .b({~u[31], u[30:0]}), .y(vu));
  logos_fmul_p mb(.clk(clk), .a(betas[e]), .b(vu), .y(c));
  wire coef_in = v_d[@COEF@] && !upd_d[@COEF@] && last_d[@COEF@];
  wire write_back = v_d[@UPDATED@] && upd_d[@UPDATED@];
  always @(posedge clk) begin
    if (take) gbuf[j * BEATS + b] <= in_beat;
    else if (write_back) gbuf[j_d[@UPDATED@] * BEATS + b_d[@UPDATED@]] <= upd;
    if (coef_in) coef[j_d[@COEF@]] <= c;
  end
  always @(posedge clk) begin
    if (rst) begin state <= IN; j <= 2'd0; b <= 16'd0; e <= 8'd0; coefs <= 3'd0; written <= 16'd0; row_base <= 16'd0; end
    else begin
      case (state)
        IN: if (take) begin
          if (b == BEATS - 1) begin
            b <= 16'd0; j <= j + 2'd1;
            if (j == 2'd3) begin e <= 8'd0; state <= (entries == 8'd0) ? OUT : DOT; end
          end else b <= b + 16'd1;
        end
        DOT, UPD: begin
          if (j == 2'd3) begin
            j <= 2'd0;
            if (b == BEATS - 1) begin
              b <= 16'd0;
              if (state == DOT) begin coefs <= 3'd0; state <= COEF; end else state <= FILL;
            end else b <= b + 16'd1;
          end else j <= j + 2'd1;
        end
        COEF: if (coefs == 3'd4) begin written <= 16'd0; state <= UPD; end
        FILL: if (written == GB) begin
          if (e + 8'd1 == entries) state <= OUT; else begin e <= e + 8'd1; state <= DOT; end
        end
        OUT: if (out_ready) begin
          if (b == BEATS - 1) begin
            b <= 16'd0; j <= j + 2'd1;
            if (j == 2'd3) begin row_base <= row_base + 16'd4; state <= IN; end
          end else b <= b + 16'd1;
        end
      endcase
      if (coef_in) coefs <= coefs + 3'd1;
      if (write_back) written <= written + 16'd1;
      if (log_wr) row_base <= 16'd0;
    end
  end
  // The rows out, each element stored: a beat read, then its binary16 rounding (logos_f32_to_f16_p) with the beat held
  // beside it; out_half the stored binary16, out_beat the element as stored.
  reg [511:0] o_beat;
  reg o_v;
  always @(posedge clk) begin
    o_v <= !rst && state == OUT && out_ready;
    o_beat <= gbuf[j * BEATS + b];
  end
  // The beat held beside its rounding, its valid in registers the reset clears.
  wire [511:0] o_raw;
  logos_delay #(.W(512), .D(@F16@)) dob(.clk(clk), .d(o_beat), .q(o_raw));
  reg ov_d [1:@F16@];
  integer ok;
  always @(posedge clk) begin
    ov_d[1] <= !rst && o_v;
    for (ok = 2; ok <= @F16@; ok = ok + 1) ov_d[ok] <= !rst && ov_d[ok-1];
  end
  assign out_valid = ov_d[@F16@] && !rst;
  genvar o;
  generate
    for (o = 0; o < 16; o = o + 1) begin : st
      wire [31:0] back;
      logos_f32_to_f16_p enc(.clk(clk), .x(o_beat[32*o +: 32]), .h(out_half[16*o +: 16]));
      logos_f16_to_f32 dec(.h(out_half[16*o +: 16]), .x(back));
      assign out_beat[32*o +: 32] = f16 ? back : o_raw[32*o +: 32];
    end
  endgenerate
endmodule

// A lazily flushed head's query folded back through its log: the log's tokens written beat by beat (token log_at, beat
// log_beat: its key's beat and its values' beat), each token's log decay and gate with its beat 0, and the query beat by
// beat; then start folds the query through the first `entries` tokens newest first, and y_beat and carried_beat give
// beat rd_beat of the folded query and the collected values, decay_out token rd_token's decay, once busy is low.
module logos_gdn_fold_p #(parameter N = 128, T_MAX = 16)
  (input clk, input rst,
   input log_wr, input [7:0] log_at, input [15:0] log_beat, input [511:0] log_k, input [511:0] log_v,
   input [31:0] log_decay, input [31:0] log_beta,
   input q_wr, input [15:0] q_beat, input [511:0] q_data,
   input start, input [7:0] entries, output busy,
   input [15:0] rd_beat, output [511:0] y_beat, output [511:0] carried_beat, input [7:0] rd_token, output [31:0] decay_out);
  localparam [15:0] BEATS = N / 16;
  localparam IDLE = 3'd0, EXPS = 3'd1, DOT = 3'd2, BCW = 3'd3, UPD = 3'd4, FILL = 3'd5;
  `LOGOS_RAMSTYLE(T_MAX*BEATS) reg [511:0] kmem [0:T_MAX*BEATS-1];
  `LOGOS_RAMSTYLE(T_MAX*BEATS) reg [511:0] vmem [0:T_MAX*BEATS-1];
  `LOGOS_RAMSTYLE_ASYNC(T_MAX) reg [31:0] log_decays [0:T_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(T_MAX) reg [31:0] betas [0:T_MAX-1];
  (* ramstyle = "logic" *) reg [511:0] qmem [0:BEATS-1];
  always @(posedge clk) begin
    if (log_wr) begin
      kmem[log_at * BEATS + log_beat] <= log_k;
      vmem[log_at * BEATS + log_beat] <= log_v;
      if (log_beat == 16'd0) begin log_decays[log_at] <= log_decay; betas[log_at] <= log_beta; end
    end
    if (q_wr) qmem[q_beat] <= q_data;
  end
  (* ramstyle = "logic" *) reg [511:0] ybuf [0:BEATS-1];
  (* ramstyle = "logic" *) reg [511:0] cbuf [0:BEATS-1];
  assign y_beat = ybuf[rd_beat];
  assign carried_beat = cbuf[rd_beat];
  `LOGOS_RAMSTYLE_ASYNC(T_MAX) reg [31:0] decays [0:T_MAX-1];
  assign decay_out = decays[rd_token];
  reg [2:0] state;
  reg [7:0] e, sent, got;
  reg [15:0] b;
  reg [1:0] gap;
  reg [15:0] written;
  reg [31:0] bc;
  assign busy = state != IDLE;
  // The decays: every token's log decay through the exponential, back to back.
  wire [31:0] ex;
  reg [@EXP@:1] ex_v;
  reg [7:0] ex_at [1:@EXP@];
  logos_exp_p xp(.clk(clk), .x_in(log_decays[sent]), .y(ex));
  integer i;
  always @(posedge clk) begin
    ex_v <= rst ? {@EXP@{1'b0}} : {ex_v[@EXP@-1:1], state == EXPS && sent != entries};
    ex_at[1] <= sent;
    for (i = 2; i <= @EXP@; i = i + 1) ex_at[i] <= ex_at[i-1];
    if (ex_v[@EXP@]) decays[ex_at[@EXP@]] <= ex;
  end
  // A dot's or an update's beat: the folded query's, the key's, the values', the collected values'.
  wire dot_issue = state == DOT && gap == 2'd0;
  wire upd_issue = state == UPD;
  reg [511:0] y_q, k_q, v_q, c_q;
  reg d_q, u_q, first_q, last_q;
  reg [15:0] b_q;
  reg [31:0] bc_q, decay_q;
  always @(posedge clk) begin
    y_q <= ybuf[b];
    k_q <= kmem[e * BEATS + b];
    v_q <= vmem[e * BEATS + b];
    c_q <= cbuf[b];
    d_q <= !rst && dot_issue;
    u_q <= !rst && upd_issue;
    first_q <= b == 16'd0;
    last_q <= b == BEATS - 1;
    b_q <= b;
    bc_q <= bc;
    decay_q <= decays[e];
  end
  reg [@BC@:1] d_d, last_d, first_d;
  reg [@FOLDED@:1] u_d;
  reg [15:0] b_d [1:@FOLDED@];
  reg [511:0] c_d [1:@FMUL@];
  reg [511:0] y_d [1:@FMUL@];
  always @(posedge clk) begin
    d_d <= rst ? {@BC@{1'b0}} : {d_d[@BC@-1:1], d_q};
    last_d <= {last_d[@BC@-1:1], last_q};
    first_d <= {first_d[@BC@-1:1], first_q};
    u_d <= rst ? {@FOLDED@{1'b0}} : {u_d[@FOLDED@-1:1], u_q};
    b_d[1] <= b_q;
    for (i = 2; i <= @FOLDED@; i = i + 1) b_d[i] <= b_d[i-1];
    c_d[1] <= c_q;
    y_d[1] <= y_q;
    for (i = 2; i <= @FMUL@; i = i + 1) begin c_d[i] <= c_d[i-1]; y_d[i] <= y_d[i-1]; end
  end
  // The lanes: the key's dot with the folded query, each lane's sum back at its add as its next term enters; and the
  // update, carried + bc·v and decay·(y − bc·k).
  wire [511:0] sums, cnew, ynew;
  genvar l;
  generate
    for (l = 0; l < 16; l = l + 1) begin : lane
      wire [31:0] p, cv, bk, t;
      logos_fmul_p m(.clk(clk), .a(k_q[32*l +: 32]), .b(y_q[32*l +: 32]), .y(p));
      logos_fadd_p a(.clk(clk), .a(first_d[@PRODUCTS@] ? 32'd0 : sums[32*l +: 32]), .b(p), .y(sums[32*l +: 32]));
      logos_fmul_p mv(.clk(clk), .a(bc_q), .b(v_q[32*l +: 32]), .y(cv));
      logos_fadd_p ac(.clk(clk), .a(c_d[@FMUL@][32*l +: 32]), .b(cv), .y(cnew[32*l +: 32]));
      logos_fmul_p mk(.clk(clk), .a(bc_q), .b(k_q[32*l +: 32]), .y(bk));
      logos_fadd_p sy(.clk(clk), .a(y_d[@FMUL@][32*l +: 32]), .b({~bk[31], bk[30:0]}), .y(t));
      logos_fmul_p md(.clk(clk), .a(decay_q), .b(t), .y(ynew[32*l +: 32]));
    end
  endgenerate
  // The dot through the tree and the gate's product; the collected values written back an add after their products,
  // the query a multiply after that.
  wire [31:0] dot, bcw;
  logos_tree16_p tr(.clk(clk), .lanes(sums), .sum(dot));
  logos_fmul_p mb(.clk(clk), .a(betas[e]), .b(dot), .y(bcw));
  wire bc_in = d_d[@BC@] && last_d[@BC@];
  wire c_back = u_d[@UPDATED@];
  wire y_back = u_d[@FOLDED@];
  always @(posedge clk) begin
    if (state == IDLE && start) for (i = 0; i < N / 16; i = i + 1) begin ybuf[i] <= qmem[i]; cbuf[i] <= 512'd0; end
    else begin
      if (c_back) cbuf[b_d[@UPDATED@]] <= cnew;
      if (y_back) ybuf[b_d[@FOLDED@]] <= ynew;
    end
  end
  always @(posedge clk) begin
    if (rst) begin state <= IDLE; e <= 8'd0; sent <= 8'd0; got <= 8'd0; b <= 16'd0; gap <= 2'd0; written <= 16'd0; end
    else begin
      case (state)
        IDLE: if (start) begin sent <= 8'd0; got <= 8'd0; state <= EXPS; end
        EXPS: begin
          if (sent != entries) sent <= sent + 8'd1;
          if (got == entries) begin
            if (entries == 8'd0) state <= IDLE; else begin e <= entries - 8'd1; b <= 16'd0; gap <= 2'd0; state <= DOT; end
          end
        end
        DOT: begin
          gap <= gap + 2'd1;
          if (gap == 2'd0) begin
            if (b == BEATS - 1) begin b <= 16'd0; state <= BCW; end else b <= b + 16'd1;
          end
        end
        BCW: if (bc_in) begin bc <= bcw; written <= 16'd0; state <= UPD; end
        UPD: if (b == BEATS - 1) begin b <= 16'd0; state <= FILL; end else b <= b + 16'd1;
        FILL: if (written == BEATS) begin
          if (e == 8'd0) state <= IDLE; else begin e <= e - 8'd1; gap <= 2'd0; state <= DOT; end
        end
      endcase
      if (ex_v[@EXP@]) got <= got + 8'd1;
      if (y_back) written <= written + 16'd1;
    end
  end
endmodule

// A token row's lazily flushed Gated-DeltaNet block for the user clock: the log's count from its line; the row's entry —
// its key heads, value heads, decay exponents and gates from the store at k_beat, v_beat, ld_beat and gt_beat, each
// field's lanes past its elements zeroed — written after the log's entries as the state stores it, in whole lines, and
// the rest once every write is written (`written`); then for each value head h (its key group h mod nk) the group's
// query from the store and the head's log from memory into the fold, the query folded; each of its value rows from
// memory through the read-out and, once the log holds `tokens` entries, the flush; the read-outs into the store a beat
// at a time from out_beat, the flushed rows back where they were read; and the count written back, none once flushed.
// The block's place is byte offsets within the row's region (`region`, a line): its memory, its log (the count's line,
// then the entries), an entry's bytes, its fields' elements; the state in binary16 when f16, f32 otherwise. Memory is
// logical 64-byte lines: a read's lines come back in order, at most OUTSTANDING of them owed. The store reads a clock
// after its address. Busy until its last line is out. W is the read-out's and the flush's sixteens of elements a clock:
// at 1 logos_gdn_read_p and logos_gdn_flush_p take half a binary16 line a clock, at 2 logos_gdn_read_w and
// logos_gdn_flush_w take a whole one (two f32 lines a beat), the flush its log two of the fold's beats at a time.
// MODE 0 is the whole block. MODE 1 is logos_gdn_lanes' coordinator: the count and the entry as MODE 0, then the log's
// count entries read from its first line (log_first, log_lines lines) into the lanes' log buffer (lb_*), the count and
// whether the row flushes given to the lanes (co_count, co_flush) while co_ready, and the count written back once
// co_taken (every lane has taken the row); it keeps no fold, read-out or flush. It reads the store only while appending
// (co_reading), and takes the next row once idle (co_idle). The row's region and store beats are taken at its start. A coordinator of a trailing flush of WINDOW tokens (MODE 1, WINDOW
// above 0) keeps the log as a ring of 2·WINDOW slots, the count line's second word the slot of its oldest token: the
// row's entry goes to the slot after its newest, the log is read into the buffer oldest token first, and once the log
// holds a window past the first the row replays (co_flush) the first window into the share of each head's rows from
// row co_lo (share count − WINDOW − 1 of HD / WINDOW rows); the count is written back with its oldest, a window
// dropped once the log holds two.
module logos_gdn_side #(parameter HD = 128, TMAX = 16, SA = 10, parameter [8:0] OUTSTANDING = 32, parameter W = 1,
                        parameter MODE = 0, parameter LBA = 13, WINDOW = 0, SRL = 1)
  (input clk, input rst,
   input start, output busy,
   input [31:0] region, input [31:0] mem_off, input [31:0] log_off, input [31:0] entry, input [31:0] nk, input [31:0] nv,
   input [31:0] fval, input [31:0] fdec, input [31:0] fgate, input [31:0] scale, input [7:0] tokens, input f16,
   input [SA-1:0] q_beat, input [SA-1:0] out_beat, input [SA-1:0] k_beat, input [SA-1:0] v_beat, input [SA-1:0] ld_beat,
   input [SA-1:0] gt_beat,
   output [SA-1:0] st_ra, input [511:0] st_rd, output reg st_we, output reg [SA-1:0] st_wa, output reg [511:0] st_wd,
   output rq_valid, input rq_ready, output [31:0] rq_line, input rs_valid, input [511:0] rs_data,
   output wq_valid, input wq_ready, output [31:0] wq_line, output [511:0] wq_data, input written,
   output co_ready, input co_taken, output reg lb_we, output reg [LBA-1:0] lb_wa, output reg [511:0] lb_wd,
   output [7:0] co_count, output co_flush, output [31:0] log_first, output [31:0] log_lines, output [15:0] co_lo,
   output co_reading, output co_idle);
  localparam [15:0] BEATS = HD / 16, HD_ROWS = HD;
  localparam IDLE = 4'd0, QLOAD = 4'd1, LOG = 4'd2, WAITF = 4'd3, COPY = 4'd4, ROWS = 4'd5, DRAIN = 4'd6, COUNT = 4'd7,
             APPEND = 4'd8, SETTLE = 4'd9, KEEP = 4'd10, LOGRD = 4'd11, LWAIT = 4'd12;
  localparam K_DEC = 3'd0, K_BETA = 3'd1, K_KEY = 3'd2, K_VAL = 3'd3, K_ROW = 3'd4, K_COUNT = 3'd5, K_LOGB = 3'd6;
  reg [3:0] state;
  // The row's own fields, taken at its start: its region and the store beats of its vectors.
  reg [31:0] rw_region;
  reg [SA-1:0] rw_q, rw_out, rw_k, rw_v, rw_ld, rw_gt;
  // The log's entries with the row's, and whether they flush the memory.
  reg [7:0] count;
  reg flush;
  // A trailing flush's ring: the slot of the log's oldest token, and a slot past the ring's end brought back into it.
  localparam [8:0] SLOTS2 = 2 * WINDOW;
  localparam [7:0] WINDOW8 = WINDOW;
  localparam [15:0] SHARE = WINDOW > 0 ? HD / WINDOW : 0;
  reg [7:0] oldest;
  function [7:0] ring(input [8:0] x);
    reg [8:0] r;
    begin
      r = (x >= SLOTS2) ? x - SLOTS2 : x;
      ring = r[7:0];
    end
  endfunction
  reg [4:0] w_n;
  assign busy = state != IDLE || w_n != 5'd0;
  // The head whose rows are walked (h), the head whose query and log are (lh, its key group g): a token the log does
  // not flush reads the next head's log and folds its query while this head's rows come back.
  reg [31:0] h, g, lh;
  // Log and row lines asked and not yet taken; whether this head's rows are still to be asked after the next head's
  // log; the fold armed (its log asked), running, and done.
  reg [9:0] l_owed, r_owed;
  reg rows_due, fold_arm, fold_run, fold_done;
  reg [7:0] ge;
  reg [15:0] gi, row_req;
  wire [47:0] base = {16'd0, rw_region} * 48'd64;
  wire [31:0] eb = f16 ? 32'd2 : 32'd4;
  wire [31:0] row_bytes = HD * eb;
  wire [15:0] row_lines = row_bytes[21:6];
  // The line a walk reads next: in the log, each token's decay exponent and gate, then its key's and values' beats in
  // turn; in the rows, each row's lines.
  wire [2:0] g_kind = gi == 16'd0 ? K_DEC : gi == 16'd1 ? K_BETA : gi[0] ? K_VAL : K_KEY;
  wire [15:0] g_dj = (gi - 16'd2) >> 1;
  wire [47:0] g_entry = base + log_off + 48'd64 + ge * entry;
  wire [47:0] g_byte =
    g_kind == K_DEC ? g_entry + (fdec + lh) * eb :
    g_kind == K_BETA ? g_entry + (fgate + lh) * eb :
    g_kind == K_KEY ? g_entry + (g * HD + 16 * g_dj) * eb :
    g_entry + (fval + lh * HD + 16 * g_dj) * eb;
  wire [47:0] r_byte = base + mem_off + (h * HD + row_req) * HD * eb + 64 * gi;
  // The log's count line.
  wire [47:0] c_byte = base + log_off;
  // A coordinator's log, read line by line into the lanes' log buffer: its count entries from the first's line, a
  // trailing one's oldest token first (lr_e its token, lr_l its line in it).
  reg [15:0] lr_i, lr_l;
  reg [7:0] lr_e;
  wire [7:0] lr_slot = WINDOW > 0 ? ring({1'b0, oldest} + {1'b0, lr_e}) : lr_e;
  wire [31:0] lr_line = WINDOW > 0 ? {24'd0, lr_slot} * (entry >> 6) + {16'd0, lr_l} : {16'd0, lr_i};
  wire [47:0] lr_first_byte = base + log_off + 48'd64;
  assign log_first = lr_first_byte[37:6];
  assign log_lines = {8'd0, count} * (entry >> 6);
  assign co_count = count;
  assign co_flush = flush;
  assign co_lo = (WINDOW > 0 && flush) ? ({8'd0, count} - {8'd0, WINDOW8} - 16'd1) * SHARE : 16'd0;
  assign co_ready = MODE == 1 && state == LWAIT;
  assign co_idle = state == IDLE;
  wire lr_walk = MODE == 1 && state == LOGRD && {16'd0, lr_i} != log_lines;
  wire [47:0] req_byte = state == ROWS ? r_byte : state == COUNT ? c_byte : state == LOGRD ? {log_first + lr_line, 6'd0} : g_byte;
  wire log_walk = MODE != 1 && state == LOG && ge != count;
  wire row_walk = MODE != 1 && state == ROWS && row_req != HD_ROWS;
  reg c_asked;
  wire count_walk = state == COUNT && !c_asked;
  reg [9:0] owed;
  assign rq_valid = (log_walk || row_walk || count_walk || lr_walk) && owed < {1'b0, OUTSTANDING};
  assign rq_line = req_byte[37:6];
  wire rq_take = rq_valid && rq_ready;
  // Every read's tag — its kind, its token or row, its beat or line, its byte within the line — in order; and every line
  // as the side takes it, in order: an arriving line registered with its tag, a clock on its binary16 halves widened to
  // f32 — the low half's beat in lo, the high half's in hi — and the element at its tag's byte widened, the line itself
  // in lo where its elements are f32 or it is a count's or a log buffer's line.
  localparam QB = OUTSTANDING > 9'd32 ? $clog2(OUTSTANDING) + 1 : 6;
  localparam QD = 1 << QB;
  reg [37:0] tags [0:QD-1];
  reg [511:0] lo_q [0:QD-1];
  reg [511:0] hi_q [0:QD-1];
  reg [31:0] el_q [0:QD-1];
  reg [QB-1:0] t_wr, t_rd, l_in, l_wr;
  wire [37:0] t_new = state == ROWS ? {K_ROW, row_req[7:0], gi, req_byte[5:0], 5'd0} :
    state == COUNT ? {K_COUNT, 35'd0} : state == LOGRD ? {K_LOGB, 8'd0, lr_i, 11'd0} : {g_kind, ge, g_dj, req_byte[5:0], 5'd0};
  // The key group of the head after this one, its group's key heads over the row's.
  wire [31:0] g_after = (g + 32'd1 == nk) ? 32'd0 : g + 32'd1;
  // The arriving line, a clock on: its tag's kind and its element at the tag's byte, then its widened form.
  reg a1_v;
  reg [511:0] a1_d;
  reg [2:0] a1_kind;
  reg [31:0] a1_e;
  wire [37:0] tag_in = tags[l_in];
  wire [511:0] in_lo, in_hi;
  wire [31:0] in_e;
  genvar l;
  generate
    if (MODE == 1) begin : raw_lines
      assign in_lo = 512'd0;
      assign in_hi = 512'd0;
      assign in_e = 32'd0;
    end else begin : widened_lines
      for (l = 0; l < 16; l = l + 1) begin : widen
        logos_f16_to_f32 lo(.h(a1_d[16*l +: 16]), .x(in_lo[32*l +: 32]));
        logos_f16_to_f32 hi(.h(a1_d[256 + 16*l +: 16]), .x(in_hi[32*l +: 32]));
      end
      logos_f16_to_f32 we(.h(a1_e[15:0]), .x(in_e));
    end
  endgenerate
  wire a1_raw = !f16 || a1_kind == K_COUNT || a1_kind == K_LOGB;
  wire [511:0] in_lo_q = a1_raw ? a1_d : in_lo;
  wire [31:0] in_el = f16 ? in_e : a1_e;
  always @(posedge clk) begin
    if (rq_take) tags[t_wr] <= t_new;
    a1_v <= !rst && rs_valid;
    a1_d <= rs_data;
    a1_kind <= tag_in[37:35];
    a1_e <= f16 ? {16'd0, rs_data[8 * tag_in[10:5] +: 16]} : rs_data[8 * tag_in[10:5] +: 32];
    if (a1_v) begin lo_q[l_wr] <= in_lo_q; hi_q[l_wr] <= in_hi; el_q[l_wr] <= in_el; end
  end
  wire have = l_wr != t_rd;
  // The head's tag and line, read a clock ahead (below, once the head's pop is known).
  reg [37:0] tag;
  wire [2:0] t_kind = tag[37:35];
  wire [7:0] t_e = tag[34:27];
  wire [15:0] t_j = tag[26:11];
  wire [5:0] t_off = tag[10:5];
  wire [511:0] lo, hi;
  wire [31:0] elem;
  wire [37:0] tag_n;
  wire [511:0] beat_at = f16 ? (t_off[5] ? hi : lo) : lo;
  reg [31:0] ldec_h, beta_h;
  reg [511:0] key_h;
  // The fold, and the flush beside it: the head's log, written into both as it is read.
  reg f_log_wr, f_q_wr, f_start;
  reg [7:0] f_log_at;
  reg [15:0] f_log_beat, f_q_beat, f_rd_beat;
  reg [511:0] f_log_k, f_log_v, f_q_data;
  reg [31:0] f_log_decay, f_log_beta;
  wire f_busy;
  wire [511:0] f_y, f_c;
  wire [31:0] f_decay;
  generate
    if (MODE == 1) begin : coordinating_fold
      assign f_busy = 1'b0;
      assign f_y = 512'd0;
      assign f_c = 512'd0;
      assign f_decay = 32'd0;
    end else begin : folding
      logos_gdn_fold_p #(.N(HD), .T_MAX(TMAX)) fold(.clk(clk), .rst(rst), .log_wr(f_log_wr), .log_at(f_log_at),
        .log_beat(f_log_beat), .log_k(f_log_k), .log_v(f_log_v), .log_decay(f_log_decay), .log_beta(f_log_beta), .q_wr(f_q_wr),
        .q_beat(f_q_beat), .q_data(f_q_data), .start(f_start), .entries(count), .busy(f_busy), .rd_beat(f_rd_beat), .y_beat(f_y),
        .carried_beat(f_c), .rd_token(8'd0), .decay_out(f_decay));
    end
  endgenerate
  // The folded query and the collected values, held for the head's rows.
  reg [32*HD-1:0] ybuf;
  `LOGOS_RAMSTYLE_ASYNC(BEATS) reg [511:0] cbuf [0:BEATS-1];
  // A beat of the fold's out, copied in a clock after it is read: the folded query's and the collected values'.
  reg cp_v;
  reg [15:0] cp_at;
  reg [511:0] cp_y, cp_c;
  always @(posedge clk) begin
    cp_v <= !rst && state == COPY;
    cp_at <= f_rd_beat;
    cp_y <= f_y;
    cp_c <= f_c;
    if (cp_v) cbuf[cp_at] <= cp_c;
  end
  genvar cq;
  generate
    for (cq = 0; cq < BEATS; cq = cq + 1) begin : copy_in
      always @(posedge clk) if (cp_v && cp_at == cq) ybuf[512*cq +: 512] <= cp_y;
    end
  endgenerate
  // A row's lines a beat a clock to the read-out and, when flushing, the flush; its carried value with each beat (the
  // read-out takes it with a row's first).
  reg half;
  wire row_line = have && t_kind == K_ROW;
  wire [511:0] c_beat = cbuf[t_e >> 4];
  wire [31:0] row_carried = c_beat[32 * t_e[3:0] +: 32];
  wire r_ready, x_ready, r_ov, x_ov;
  wire [31:0] r_out;
  wire [512*W-1:0] x_out;
  wire [256*W-1:0] x_halves;
  // A row line into the units: at W 1 a beat a half line in binary16, a line in f32; at W 2 a binary16 line, or two f32
  // lines, the first held (row_lo) and not fed.
  wire held_line = W > 1 && !f16 && !half;
  wire feed = row_line && (held_line || (r_ready && (!flush || x_ready)));
  wire to_units = row_line && !held_line;
  wire last_half = W > 1 || !f16 || half;
  reg [511:0] row_lo;
  wire [512*W-1:0] row_beat;
  // Writes the flush's out-stream leaves: at W 2 in f32 two lines a beat, the second the clock after the first, so the
  // stream is asked for a beat every other clock. A beat's lines leave XD and XD + 1 clocks after the ask the stream
  // takes (its read, then its binary16 rounding), so whether the stream is asked next clock (x_ask, a register) is
  // decided on this clock's queue: it holds room for every line of every ask of the last XD + 1 clocks and this one,
  // whose lines may not be in it yet, and of the next — an ask's one line (half a line a beat at W 1 in binary16), or
  // at W 2 in f32 its two.
  localparam XD = 1 + @F16@;
  reg x_pend, x_tog, x_hi, x_ask;
  reg [255:0] x_low;
  reg [511:0] x_out_hi;
  reg [XD:0] x_hist;
  reg [4:0] x_way;
  integer xi;
  always @* begin
    x_way = {4'd0, x_ask};
    for (xi = 0; xi <= XD; xi = xi + 1) x_way = x_way + {4'd0, x_hist[xi]};
  end
  wire x_two = W > 1 && !f16;
  wire [5:0] x_owed = x_two ? {x_way, 1'b0} + 6'd2 : {1'b0, x_way} + 6'd1;
  // The flush's log at W 2: two of the fold's beats, key and values, the even held.
  reg fw_wr;
  reg [15:0] fw_beat;
  reg [511:0] lo_k, lo_v;
  reg [1023:0] fw_k, fw_v;
  // The flushed line written this clock, and whether one is.
  wire [511:0] x_line_data;
  wire x_line_push;
  generate
    if (MODE == 1) begin : coordinating
      assign row_beat = {512*W{1'b0}};
      assign r_ready = 1'b0;
      assign r_ov = 1'b0;
      assign r_out = 32'd0;
      assign x_ready = 1'b0;
      assign x_ov = 1'b0;
      assign x_out = {512*W{1'b0}};
      assign x_halves = {256*W{1'b0}};
      assign x_line_data = 512'd0;
      assign x_line_push = 1'b0;
    end else if (W == 1) begin : narrow
      assign row_beat = f16 ? (half ? hi : lo) : lo;
      logos_gdn_read_p #(.N(HD)) rd(.clk(clk), .rst(rst), .y(ybuf), .scale(scale), .in_valid(row_line && (!flush || x_ready)),
        .in_ready(r_ready), .in_beat(row_beat), .in_carried(row_carried), .out_valid(r_ov), .out_read(r_out));
      logos_gdn_flush_p #(.N(HD), .T_MAX(TMAX)) fl(.clk(clk), .rst(rst), .log_wr(f_log_wr), .log_at(f_log_at), .log_beat(f_log_beat),
        .log_k(f_log_k), .log_v(f_log_v), .log_decay(f_log_decay), .log_beta(f_log_beta), .entries(count), .f16(f16),
        .in_valid(flush && row_line && r_ready), .in_ready(x_ready), .in_beat(row_beat),
        .out_valid(x_ov), .out_ready(x_ask), .out_beat(x_out), .out_half(x_halves));
      assign x_line_data = f16 ? {x_halves, x_low} : x_out;
      assign x_line_push = x_ov && (!f16 || x_hi);
    end else begin : wide
      assign row_beat = f16 ? {hi, lo} : {lo, row_lo};
      logos_gdn_read_w #(.N(HD), .W(W)) rd(.clk(clk), .rst(rst), .y(ybuf), .scale(scale), .in_valid(to_units && (!flush || x_ready)),
        .in_ready(r_ready), .in_beat(row_beat), .in_carried(row_carried), .out_valid(r_ov), .out_read(r_out));
      logos_gdn_flush_w #(.N(HD), .W(W), .T_MAX(TMAX)) fl(.clk(clk), .rst(rst), .log_wr(fw_wr), .log_at(f_log_at),
        .log_beat(fw_beat), .log_k(fw_k[512*W-1:0]), .log_v(fw_v[512*W-1:0]), .log_decay(f_log_decay), .log_beta(f_log_beta),
        .entries(count), .f16(f16), .in_valid(flush && to_units && r_ready), .in_ready(x_ready), .in_beat(row_beat),
        .out_valid(x_ov), .out_ready(x_ask), .out_beat(x_out), .out_half(x_halves));
      assign x_line_data = f16 ? x_halves[511:0] : (x_pend ? x_out_hi : x_out[511:0]);
      assign x_line_push = x_ov || x_pend;
    end
  endgenerate
  // A tag's line consumed: a log line at once, a row's line with its last beat.
  wire pop = have && (t_kind != K_ROW || (feed && last_half));
  // The next head read into the head's registers every clock: the tag and line after this one when this one is
  // consumed, this one again when it is not; a tag or a line written the clock it is read taken as it is written.
  wire [QB-1:0] t_next = rst ? {QB{1'b0}} : t_rd + {{(QB-1){1'b0}}, pop};
  assign tag_n = (rq_take && t_wr == t_next) ? t_new : tags[t_next];
  wire landing = a1_v && l_wr == t_next;
  // The head's line: the queue's word at the next head, registered as block RAM reads it, or the line landing there
  // that clock, registered beside it.
  reg [511:0] lo_m, hi_m, lo_b, hi_b;
  reg [31:0] el_m, el_b;
  reg land_d;
  always @(posedge clk) begin
    tag <= tag_n;
    lo_m <= lo_q[t_next];
    hi_m <= hi_q[t_next];
    el_m <= el_q[t_next];
    lo_b <= in_lo_q;
    hi_b <= in_hi;
    el_b <= in_el;
    land_d <= landing;
  end
  assign lo = land_d ? lo_b : lo_m;
  assign hi = land_d ? hi_b : hi_m;
  assign elem = land_d ? el_b : el_m;
  // The flushed rows packed into lines, two beats a line in binary16 (the flush's stored halves) and one in f32, and
  // queued for memory.
  `LOGOS_RAMSTYLE_ASYNC(16) reg [511:0] wfifo_d [0:15];
  `LOGOS_RAMSTYLE_ASYNC(16) reg [31:0] wfifo_l [0:15];
  reg [3:0] w_wr, w_rd;
  // The row's log entry from the store a beat a cycle — its key heads, value heads, decay exponents and gates, each
  // field's lanes past its elements zeroed — packed into whole lines from the entry's first, as the state stores them
  // (a head a multiple of 32 wide, so a binary16 entry is an even number of beats, two a line), and the log's count line
  // once the row is done.
  wire [31:0] k_beats = (nk * HD) >> 4, v_beats = (nv * HD) >> 4, s_beats = (nv + 32'd15) >> 4;
  wire [31:0] e_beats = k_beats + v_beats + 2 * s_beats;
  reg [31:0] ab, a_line;
  reg a_pend, a_hi;
  reg [4:0] a_lanes;
  // An entry's beat asked (a_pend and its field's lanes, the clock after its address) and out of the store SRL clocks
  // after its address (a_lo, a_lo_lanes).
  wire a_lo;
  wire [4:0] a_lo_lanes;
  wire a_flight;
  logos_landing #(.W(5), .D(SRL - 1)) aland(.clk(clk), .rst(rst), .in_v(a_pend), .in_d(a_lanes), .out_v(a_lo),
    .out_d(a_lo_lanes), .flight(a_flight));
  reg [255:0] a_low;
  // Beats asked of the store and not yet out of the encoder: each one line's room in the queue, held from its asking.
  reg [2:0] a_fly;
  wire a_key = ab < k_beats, a_val = !a_key && ab < k_beats + v_beats, a_dec = !a_key && !a_val && ab < k_beats + v_beats + s_beats;
  wire [31:0] a_off = a_key ? ab : a_val ? ab - k_beats : a_dec ? ab - k_beats - v_beats : ab - k_beats - v_beats - s_beats;
  wire [31:0] a_len = (a_key ? nk * HD : a_val ? nv * HD : nv) - 16 * a_off;
  wire [SA-1:0] a_ra = (a_key ? rw_k : a_val ? rw_v : a_dec ? rw_ld : rw_gt) + a_off[SA-1:0];
  wire a_issue = state == APPEND && ab != e_beats && w_n + {2'd0, a_fly} < 5'd6;
  assign co_reading = MODE == 1 && a_issue;
  // A beat read: its lanes past the field's elements zeroed, registered, then in binary16 @F16@ clocks on
  // (logos_f32_to_f16_p), the beat carried beside and its valid in registers the reset clears.
  wire [511:0] a_lanes_in;
  reg [511:0] a_masked;
  reg a_m;
  wire a_v;
  wire [511:0] a_beat;
  wire [255:0] a_halves;
  generate
    for (l = 0; l < 16; l = l + 1) begin : field
      assign a_lanes_in[32*l +: 32] = l < a_lo_lanes ? st_rd[32*l +: 32] : 32'd0;
    end
    for (l = 0; l < 16; l = l + 1) begin : encode
      logos_f32_to_f16_p n(.clk(clk), .x(a_masked[32*l +: 32]), .h(a_halves[16*l +: 16]));
    end
  endgenerate
  logos_delay #(.W(512), .D(@F16@)) adl(.clk(clk), .d(a_masked), .q(a_beat));
  generate
  endgenerate
  reg av_d [1:@F16@];
  integer ak;
  always @(posedge clk) begin
    av_d[1] <= !rst && a_m;
    for (ak = 2; ak <= @F16@; ak = ak + 1) av_d[ak] <= !rst && av_d[ak-1];
  end
  assign a_v = av_d[@F16@];
  wire a_push = a_v && (!f16 || a_hi);
  wire [511:0] a_data = f16 ? {a_halves, a_low} : a_beat;
  wire [7:0] a_slot = WINDOW > 0 ? ring({1'b0, oldest} + {1'b0, count}) : count;
  wire [47:0] a_byte = base + log_off + 48'd64 + a_slot * entry + 64 * a_line;
  wire c_push = state == KEEP && w_n < 5'd16;
  // What a trailing flush's count line keeps: a window dropped, and its oldest a window on, once the log holds two.
  wire dropping = WINDOW > 0 && {1'b0, count} == SLOTS2;
  wire [7:0] keep_count = dropping ? count - WINDOW8 : count;
  wire [7:0] keep_oldest = dropping ? ring({1'b0, oldest} + {1'b0, WINDOW8}) : oldest;
  reg [15:0] x_row, x_line;
  wire [47:0] x_byte = base + mem_off + (h * HD + x_row) * HD * eb + 64 * x_line;
  wire x_push = x_line_push;
  assign wq_valid = w_n != 5'd0;
  assign wq_line = wfifo_l[w_rd];
  assign wq_data = wfifo_d[w_rd];
  wire w_pop = wq_valid && wq_ready;
  // The read-outs gathered into store beats.
  reg [511:0] o_beat;
  // The read-outs come out head by head behind the walk, which reads the next head's log once a head's rows are asked:
  // the head and row the next read-out is of.
  reg [31:0] oh;
  reg [15:0] o_row;
  wire [31:0] o_wa = rw_out + ((oh * HD + o_row) >> 4);
  // The store's query beats while a head's query loads.
  reg [15:0] qj, q_at;
  reg q_pending;
  // A query beat asked (q_pending and its beat, the clock after its address) and out of the store SRL clocks after its
  // address (q_lo, q_lo_at); the head's fold waits until none is on its way.
  wire q_lo, q_flight;
  wire [15:0] q_lo_at;
  logos_landing #(.W(16), .D(SRL - 1)) qland(.clk(clk), .rst(rst), .in_v(q_pending), .in_d(q_at), .out_v(q_lo),
    .out_d(q_lo_at), .flight(q_flight));
  wire [31:0] q_ra = rw_q + ((g * HD) >> 4) + qj;
  assign st_ra = state == APPEND ? a_ra : q_ra[SA-1:0];
  always @(posedge clk) begin
    f_log_wr <= 1'b0; f_q_wr <= 1'b0; f_start <= 1'b0; st_we <= 1'b0; q_pending <= 1'b0; a_pend <= 1'b0; fw_wr <= 1'b0;
    lb_we <= 1'b0;
    x_tog <= !x_tog;
    x_hist <= {x_hist[XD-1:0], x_ask};
    x_ask <= !rst && (!x_two || !x_tog) && {1'b0, w_n} + x_owed <= 6'd16;
    if (rq_take) t_wr <= t_wr + 1'b1;
    if (rs_valid) l_in <= l_in + 1'b1;
    if (a1_v) l_wr <= l_wr + 1'b1;
    owed <= owed + (rq_take ? 10'd1 : 10'd0) - (pop ? 10'd1 : 10'd0);
    l_owed <= l_owed + ((rq_take && state == LOG) ? 10'd1 : 10'd0) - ((have && t_kind != K_ROW && t_kind != K_COUNT) ? 10'd1 : 10'd0);
    r_owed <= r_owed + ((rq_take && state == ROWS) ? 10'd1 : 10'd0) - ((pop && t_kind == K_ROW) ? 10'd1 : 10'd0);
    // The fold starts once its head's log lines are all in, and is done when it falls idle after starting.
    if (fold_arm && l_owed == 10'd0) begin f_start <= 1'b1; fold_arm <= 1'b0; fold_run <= 1'b1; end
    if (fold_run && !f_start && !f_busy) begin fold_run <= 1'b0; fold_done <= 1'b1; end
    // The log's lines: the decay exponent and the gate held, a key beat held, a value beat written with it into the
    // fold and the flush.
    if (have && t_kind != K_ROW) begin
      t_rd <= t_rd + 1'b1;
      case (t_kind)
        K_DEC: ldec_h <= elem;
        K_BETA: beta_h <= elem;
        K_KEY: key_h <= beat_at;
        K_COUNT: begin count <= lo[7:0]; oldest <= WINDOW > 0 ? lo[39:32] : 8'd0; end
        K_LOGB: begin lb_we <= 1'b1; lb_wa <= t_j[LBA-1:0]; lb_wd <= lo; end
        default: begin
          f_log_wr <= 1'b1; f_log_at <= t_e; f_log_beat <= t_j; f_log_k <= key_h; f_log_v <= beat_at;
          f_log_decay <= ldec_h; f_log_beta <= beta_h;
          if (!t_j[0]) begin lo_k <= key_h; lo_v <= beat_at; end
          else begin fw_wr <= W > 1; fw_beat <= t_j >> 1; fw_k <= {key_h, lo_k}; fw_v <= {beat_at, lo_v}; end
        end
      endcase
    end
    if (feed) begin
      if (W > 1) begin
        t_rd <= t_rd + 1'b1;
        if (!f16) begin half <= !half; if (held_line) row_lo <= lo; end
      end else if (last_half) begin half <= 1'b0; t_rd <= t_rd + 1'b1; end else half <= 1'b1;
    end
    if (x_line_push) begin
      wfifo_d[w_wr] <= x_line_data;
      wfifo_l[w_wr] <= x_byte[37:6];
      w_wr <= w_wr + 4'd1;
      if (x_line + 16'd1 == row_lines) begin x_line <= 16'd0; x_row <= x_row + 16'd1; end else x_line <= x_line + 16'd1;
    end
    if (W == 1 && x_ov) begin
      if (f16 && !x_hi) begin x_low <= x_halves[255:0]; x_hi <= 1'b1; end else x_hi <= 1'b0;
    end
    x_pend <= W > 1 && x_ov && !f16;
    if (x_ov) x_out_hi <= x_out[512*W-1:512*W-512];
    if (a_issue) begin a_pend <= 1'b1; a_lanes <= a_len >= 32'd16 ? 5'd16 : a_len[4:0]; ab <= ab + 32'd1; end
    a_m <= !rst && a_lo;
    a_masked <= a_lanes_in;
    a_fly <= a_fly + (a_issue ? 3'd1 : 3'd0) - (a_v ? 3'd1 : 3'd0);
    if (a_v && f16 && !a_hi) begin a_low <= a_halves; a_hi <= 1'b1; end
    if (a_push) begin
      a_hi <= 1'b0;
      wfifo_d[w_wr] <= a_data;
      wfifo_l[w_wr] <= a_byte[37:6];
      w_wr <= w_wr + 4'd1;
      a_line <= a_line + 32'd1;
    end
    if (c_push) begin
      wfifo_d[w_wr] <= WINDOW > 0 ? {472'd0, keep_oldest, 24'd0, keep_count} : {504'd0, flush ? 8'd0 : count};
      wfifo_l[w_wr] <= c_byte[37:6];
      w_wr <= w_wr + 4'd1;
    end
    if (w_pop) w_rd <= w_rd + 4'd1;
    w_n <= w_n + ((x_push || a_push || c_push) ? 5'd1 : 5'd0) - (w_pop ? 5'd1 : 5'd0);
    if (r_ov) begin
      o_beat[32 * o_row[3:0] +: 32] <= r_out;
      if (o_row[3:0] == 4'd15) begin st_we <= 1'b1; st_wa <= o_wa[SA-1:0]; st_wd <= {r_out, o_beat[479:0]}; end
      if (o_row + 16'd1 == HD_ROWS) begin o_row <= 16'd0; oh <= oh + 32'd1; end else o_row <= o_row + 16'd1;
    end
    if (q_lo) begin f_q_wr <= 1'b1; f_q_beat <= q_lo_at; f_q_data <= st_rd; end
    case (state)
      IDLE: if (start) begin
        rw_region <= region; rw_q <= q_beat; rw_out <= out_beat; rw_k <= k_beat; rw_v <= v_beat; rw_ld <= ld_beat; rw_gt <= gt_beat;
        qj <= 16'd0; c_asked <= 1'b0; o_row <= 16'd0; rows_due <= 1'b0;
        h <= 32'd0; lh <= 32'd0; g <= 32'd0; oh <= 32'd0; state <= COUNT;
      end
      // The count, then the row's entry after it; the log's lines read once the entry is written.
      COUNT: begin
        if (rq_take) c_asked <= 1'b1;
        if (c_asked && owed == 10'd0) begin ab <= 32'd0; a_line <= 32'd0; a_hi <= 1'b0; state <= APPEND; end
      end
      APPEND: if (ab == e_beats && a_fly == 3'd0) state <= SETTLE;
      SETTLE: if (w_n == 4'd0 && written) begin
        count <= count + 8'd1; flush <= WINDOW > 0 ? count + 8'd1 > WINDOW8 : count + 8'd1 >= tokens;
        lr_i <= 16'd0; lr_e <= 8'd0; lr_l <= 16'd0; state <= (MODE == 1) ? LOGRD : QLOAD;
      end
      // A coordinator's log into the lanes' buffer, every line in; the row handed to every lane; then the count.
      LOGRD: begin
        if (rq_take) begin
          lr_i <= lr_i + 16'd1;
          if ({16'd0, lr_l} + 32'd1 == (entry >> 6)) begin lr_l <= 16'd0; lr_e <= lr_e + 8'd1; end else lr_l <= lr_l + 16'd1;
        end
        if ({16'd0, lr_i} == log_lines && owed == 10'd0) state <= LWAIT;
      end
      LWAIT: if (co_taken) state <= KEEP;
      KEEP: if (c_push) state <= IDLE;
      // Head lh's query into the fold, then its log asked; then, when head h's rows are due, they are asked behind it.
      QLOAD: begin
        if (qj == 16'd0) begin x_row <= 16'd0; x_line <= 16'd0; x_hi <= 1'b0; end
        if (qj == BEATS) begin qj <= 16'd0; ge <= 8'd0; gi <= 16'd0; state <= LOG; end
        else begin q_pending <= 1'b1; q_at <= qj; qj <= qj + 16'd1; end
      end
      LOG: begin
        if (rq_take) begin
          if (gi == 16'd1 + 2 * BEATS) begin gi <= 16'd0; ge <= ge + 8'd1; end else gi <= gi + 16'd1;
        end
        if (ge == count && (SRL == 1 || (!q_pending && !q_flight))) begin
          fold_arm <= 1'b1; gi <= 16'd0; row_req <= 16'd0; state <= rows_due ? ROWS : WAITF;
        end
      end
      // Head lh's fold done and head h's rows all taken: the folded query and the collected values copied in.
      WAITF: if (fold_done && r_owed == 10'd0) begin fold_done <= 1'b0; f_rd_beat <= 16'd0; state <= COPY; end
      COPY: begin
        if (f_rd_beat + 16'd1 == BEATS) begin
          f_rd_beat <= 16'd0; h <= lh;
          if (!flush && lh + 32'd1 < nv) begin
            lh <= lh + 32'd1; g <= g_after; qj <= 16'd0; rows_due <= 1'b1; state <= QLOAD;
          end else begin gi <= 16'd0; row_req <= 16'd0; state <= ROWS; end
        end else f_rd_beat <= f_rd_beat + 16'd1;
      end
      // Head h's rows asked: behind the next head's log, then its fold awaited; or, the last head's or a flushed one's,
      // then its read-outs and flushed rows in DRAIN.
      ROWS: begin
        if (rq_take) begin
          if (gi + 16'd1 == row_lines) begin gi <= 16'd0; row_req <= row_req + 16'd1; end else gi <= gi + 16'd1;
        end
        if (row_req == HD_ROWS) begin
          if (rows_due) begin rows_due <= 1'b0; state <= WAITF; end
          else if (owed == 10'd0) state <= DRAIN;
        end
      end
      DRAIN: if (oh == h + 32'd1 && (!flush || (x_row == HD_ROWS && w_n == 4'd0))) begin
        if (h + 32'd1 >= nv) state <= KEEP;
        else begin lh <= h + 32'd1; g <= g_after; qj <= 16'd0; state <= QLOAD; end
      end
      default: state <= IDLE;
    endcase
    if (rst) begin
      state <= IDLE; t_wr <= {QB{1'b0}}; t_rd <= {QB{1'b0}}; l_in <= {QB{1'b0}}; l_wr <= {QB{1'b0}}; w_wr <= 4'd0; w_rd <= 4'd0; w_n <= 5'd0; half <= 1'b0;
      owed <= 10'd0; a_fly <= 3'd0; x_pend <= 1'b0; x_hist <= {(XD+1){1'b0}}; x_tog <= 1'b0; l_owed <= 10'd0; r_owed <= 10'd0;
      fold_arm <= 1'b0; fold_run <= 1'b0; fold_done <= 1'b0;
    end
  end
endmodule
"#;

/// A head lane of the recurrence on head lanes (`logos_gdn_lane`).
const GDN_LANE_VERILOG: &str = r#"
// A head lane of logos_gdn_lanes: value heads first, first + STEP, … below nv of a token row whose log is in one of the
// lane's two copies of it (lb_ra, a line the clock after its address; each copy LBA bits of address, the second
// COPY_LINES lines in) with `count` entries, flushing the memory when `flush`. A
// loader takes the heads a batch at a time — H heads of an ordinary token (logos_gdn_fold_m's slots), one of a
// flushing token — each head's query from the store on the clocks st_gnt grants the read it asks (st_rq) and its log
// from the copy a line a clock into its slot (and, flushing, into the flush), then folds the batch into a bank; the
// next batch is loaded and folded into the other bank while this one's rows stream (a flushing token's after its head
// is flushed, the flush holding one head's log). A streamer takes each head of a folded batch: its folded query and
// collected values copied out of the bank, then its rows from memory (rq/rs, in the order asked, at most OUTSTANDING
// lines owed, no wait taken on an answer) through the read-out and, flushing, the flush, the flushed rows back where
// they were read (wq); each sixteen rows' read-outs leave as a store beat (st_we). Busy from its start until its last
// read-out and flushed line are out.
module logos_gdn_lane #(parameter HD = 128, TMAX = 16, SA = 10, parameter [8:0] OUTSTANDING = 32, parameter W = 1,
                        parameter [31:0] STEP = 4, parameter LBA = 13, FLUSHES = 1, COPY_LINES = 4096, SRL = 1)
  (input clk, input rst,
   input start, output busy, output room, output reg done,
   input [31:0] in_region, input [31:0] mem_off, input [31:0] entry, input [31:0] nk, input [31:0] nv,
   input [31:0] fval, input [31:0] fdec, input [31:0] fgate, input [31:0] scale, input f16,
   input [SA-1:0] in_q_beat, input [SA-1:0] in_out_beat,
   input [31:0] first, input [7:0] in_count, input in_flush, input in_par,
   output st_rq, input st_gnt, output [SA-1:0] st_ra, input [511:0] st_rd,
   output reg st_we, output reg [SA-1:0] st_wa, output reg [511:0] st_wd,
   output [LBA:0] lb_ra, input [511:0] lb_rd,
   output rq_valid, input rq_ready, output [31:0] rq_line, input rs_valid, input [511:0] rs_data,
   output wq_valid, input wq_ready, output [31:0] wq_line, output [511:0] wq_data);
  localparam H = @FADD@;
  localparam [15:0] BEATS = HD / 16, HD_ROWS = HD;
  localparam K_DEC = 2'd0, K_BETA = 2'd1, K_KEY = 2'd2, K_VAL = 2'd3;
  // The row's fields, taken at its start: its region, its query's and read-outs' store beats, its log's count, whether it
  // replays, and the copy of the log it reads (its first line COPY_LINES lines in for the second).
  reg [31:0] region;
  reg [SA-1:0] q_beat, out_beat;
  reg [7:0] count;
  reg flush, par;
  always @(posedge clk)
    if (start) begin region <= in_region; q_beat <= in_q_beat; out_beat <= in_out_beat; count <= in_count; flush <= in_flush; par <= in_par; end
  wire [47:0] base = {16'd0, region} * 48'd64;
  wire [31:0] eb = f16 ? 32'd2 : 32'd4;
  wire [31:0] row_bytes = HD * eb;
  wire [15:0] row_lines = row_bytes[21:6];
  localparam [7:0] HS = H;
  wire [7:0] bs = flush ? 8'd1 : HS;
  // The key group a head's step moves by, and the first head's, each a register of the job's fields.
  function [31:0] step_mod(input [31:0] k);
    reg [31:0] r;
    integer si;
    begin
      r = STEP;
      for (si = 0; si < 4; si = si + 1) if (r >= k) r = r - k;
      step_mod = r;
    end
  endfunction
  reg [31:0] g_step, first_g;
  always @(posedge clk) begin
    g_step <= step_mod(nk);
    first_g <= (first >= nk) ? ((first - nk >= nk) ? ((first - 32'd2 * nk >= nk) ? first - 32'd3 * nk : first - 32'd2 * nk) : first - nk) : first;
  end
  reg running;
  // Batches: loaded (their folds started), folded (finished), copied out (their bank free) and drained (every row of
  // theirs through, flushed rows written).
  reg [7:0] loaded, folded, copied, drained;
  // ── The loader ──────────────────────────────────────────────────────────────────────────────────────────────────
  localparam [2:0] L_IDLE = 3'd0, L_Q = 3'd1, L_LOG = 3'd2, L_TAIL = 3'd3, L_FOLD = 3'd4, L_DONE = 3'd5;
  reg [2:0] ls;
  reg [31:0] lh, lg;
  reg [7:0] lslot;
  reg [15:0] qj;
  reg q_pending;
  reg [15:0] q_at;
  // A query beat asked (q_pending, its beat and slot, the clock after its address) and out of the store SRL clocks after
  // its address (q_lo, q_lo_at, q_lo_slot); a batch's fold waits until none is on its way.
  wire q_lo, q_flight;
  wire [15:0] q_lo_at;
  wire [7:0] q_lo_slot;
  logos_landing #(.W(24), .D(SRL - 1)) qland(.clk(clk), .rst(rst), .in_v(q_pending), .in_d({q_at, lslot}), .out_v(q_lo),
    .out_d({q_lo_at, q_lo_slot}), .flight(q_flight));
  wire f_busy;
  reg f_start, f_run;
  wire can_load = !f_busy && !f_start && !f_run && (flush ? loaded == drained : loaded - copied < 8'd2);
  wire [31:0] q_ra = q_beat + ((lg * HD) >> 4) + qj;
  assign st_rq = ls == L_Q && qj != BEATS;
  assign st_ra = q_ra[SA-1:0];
  // The log walk of head lh (key group lg): for each entry its decay exponent's line, its gate's, then its key's and its
  // values' beats in turn; each line's place in the copy and the byte of its field within it.
  reg [7:0] ge;
  reg [15:0] gi;
  reg [31:0] e_at;
  wire [1:0] g_kind = gi == 16'd0 ? K_DEC : gi == 16'd1 ? K_BETA : gi[0] ? K_VAL : K_KEY;
  wire [15:0] g_dj = (gi - 16'd2) >> 1;
  wire [31:0] g_off = g_kind == K_DEC ? (fdec + lh) * eb : g_kind == K_BETA ? (fgate + lh) * eb :
                      g_kind == K_KEY ? (lg * HD + 16 * g_dj) * eb : (fval + lh * HD + 16 * g_dj) * eb;
  wire [31:0] g_byte = e_at + g_off;
  wire walking = ls == L_LOG && ge != count;
  assign lb_ra = g_byte[LBA+6:6];
  // A copy line's tag beside it: its kind, entry, beat and byte; the line a clock after its address, registered with its
  // tag the clock after that, then widened.
  reg w1_v, w2_v;
  reg [1:0] w1_k, w2_k;
  reg [7:0] w1_e, w2_e;
  reg [15:0] w1_j, w2_j;
  reg [5:0] w1_o, w2_o;
  reg [511:0] w2_d;
  wire [511:0] w_lo, w_hi;
  wire [31:0] w_e16;
  genvar l;
  generate
    for (l = 0; l < 16; l = l + 1) begin : walk_widen
      logos_f16_to_f32 lo(.h(w2_d[16*l +: 16]), .x(w_lo[32*l +: 32]));
      logos_f16_to_f32 hi(.h(w2_d[256 + 16*l +: 16]), .x(w_hi[32*l +: 32]));
    end
  endgenerate
  wire [31:0] w_raw = w2_d[8 * w2_o +: 32];
  logos_f16_to_f32 we(.h(w2_d[8 * w2_o +: 16]), .x(w_e16));
  wire [511:0] w_beat = f16 ? (w2_o[5] ? w_hi : w_lo) : w2_d;
  wire [31:0] w_elem = f16 ? w_e16 : w_raw;
  always @(posedge clk) begin
    w1_v <= !rst && walking;
    w1_k <= g_kind; w1_e <= ge; w1_j <= g_dj; w1_o <= g_byte[5:0];
    w2_v <= !rst && w1_v;
    w2_k <= w1_k; w2_e <= w1_e; w2_j <= w1_j; w2_o <= w1_o; w2_d <= lb_rd;
  end
  // The fold's and the flush's log writes: a value beat with the key beat, decay exponent and gate before it.
  reg [31:0] ldec_h, beta_h;
  reg [511:0] key_h;
  reg f_log_wr, f_q_wr, fw_wr;
  reg [7:0] f_log_at, f_q_slot;
  reg [15:0] f_log_beat, f_q_beat, fw_beat;
  reg [511:0] f_log_k, f_log_v, f_q_data, lo_k, lo_v;
  reg [1023:0] fw_k, fw_v;
  reg [31:0] f_log_decay, f_log_beta;
  reg fold_bank;
  // ── The streamer ────────────────────────────────────────────────────────────────────────────────────────────────
  localparam [2:0] S_IDLE = 3'd0, S_COPY = 3'd1, S_ROWS = 3'd2, S_NEXT = 3'd3, S_DRAIN = 3'd4, S_DONE = 3'd5;
  reg [2:0] ss;
  reg [31:0] sh;
  reg [7:0] sslot;
  reg [15:0] cb;
  wire [511:0] f_y, f_c;
  wire [31:0] f_decay;
  logos_gdn_fold_m #(.N(HD), .T_MAX(TMAX)) fold(.clk(clk), .rst(rst), .log_wr(f_log_wr), .log_slot(lslot), .log_at(f_log_at),
    .log_beat(f_log_beat), .log_k(f_log_k), .log_v(f_log_v), .log_decay(f_log_decay), .log_beta(f_log_beta), .q_wr(f_q_wr),
    .q_slot(f_q_slot), .q_beat(f_q_beat), .q_data(f_q_data), .start(f_start), .bank(fold_bank), .entries(count), .older(8'd0),
    .busy(f_busy), .rd_bank(copied[0]), .rd_slot(sslot), .rd_beat(cb), .y_beat(f_y), .carried_beat(f_c),
    .rd_token(8'd0), .decay_out(f_decay));
  // The streamed head's folded query and collected values, into buffer sb, the buffer the head before did not take: a
  // beat of each asked a clock (cb), out of the fold's bank two clocks after, copied in the clock after that; the copy
  // over once its last beat is in.
  reg sb;
  reg [32*HD-1:0] ybuf0, ybuf1;
  `LOGOS_RAMSTYLE_ASYNC(BEATS) reg [511:0] cbuf0 [0:BEATS-1];
  `LOGOS_RAMSTYLE_ASYNC(BEATS) reg [511:0] cbuf1 [0:BEATS-1];
  reg cp_a, cp_m, cp_v, cp_b;
  reg [15:0] cp_ab, cp_mb, cp_at;
  reg [511:0] cp_y, cp_c;
  always @(posedge clk) begin
    cp_a <= !rst && ss == S_COPY && cb < BEATS;
    cp_ab <= cb;
    cp_m <= !rst && cp_a;
    cp_mb <= cp_ab;
    cp_v <= !rst && cp_m;
    cp_at <= cp_mb;
    cp_b <= sb;
    cp_y <= f_y;
    cp_c <= f_c;
    if (cp_v && !cp_b) cbuf0[cp_at] <= cp_c;
    if (cp_v && cp_b) cbuf1[cp_at] <= cp_c;
  end
  genvar cq;
  generate
    for (cq = 0; cq < BEATS; cq = cq + 1) begin : copy_in
      always @(posedge clk) begin
        if (cp_v && !cp_b && cp_at == cq) ybuf0[512*cq +: 512] <= cp_y;
        if (cp_v && cp_b && cp_at == cq) ybuf1[512*cq +: 512] <= cp_y;
      end
    end
  endgenerate
  // The rows asked: each its row; lines answered in order, registered as they come, binary16 halves widened a clock
  // on and registered, then into the lines.
  localparam QB = OUTSTANDING > 9'd32 ? $clog2(OUTSTANDING) + 1 : 6;
  localparam QD = 1 << QB;
  reg [15:0] row_req, ri;
  reg [8:0] t_row [0:QD-1];
  reg [511:0] lo_q [0:QD-1];
  reg [511:0] hi_q [0:QD-1];
  reg [QB-1:0] t_wr, t_rd, l_in, l_wr;
  // The lines asked and not yet through the read-out, and of them those of each buffer's head.
  reg [9:0] owed, owed_b0, owed_b1;
  wire [47:0] r_byte = base + mem_off + (sh * HD + row_req) * HD * eb + 64 * ri;
  wire row_walk = ss == S_ROWS && row_req != HD_ROWS;
  assign rq_valid = row_walk && owed < {1'b0, OUTSTANDING};
  assign rq_line = r_byte[37:6];
  wire rq_take = rq_valid && rq_ready;
  reg a1_v;
  reg [511:0] a1_d;
  wire [511:0] in_lo, in_hi;
  generate
    for (l = 0; l < 16; l = l + 1) begin : row_widen
      logos_f16_to_f32 lo(.h(a1_d[16*l +: 16]), .x(in_lo[32*l +: 32]));
      logos_f16_to_f32 hi(.h(a1_d[256 + 16*l +: 16]), .x(in_hi[32*l +: 32]));
    end
  endgenerate
  wire [511:0] in_lo_q = f16 ? in_lo : a1_d;
  // A line widened, registered (a2) before its queue and the head's bypass take it.
  (* maxfan = 64 *) reg a2_v;
  reg [511:0] a2_lo, a2_hi;
  always @(posedge clk) begin
    if (rq_take) t_row[t_wr] <= {sb, row_req[7:0]};
    a1_v <= !rst && rs_valid;
    a1_d <= rs_data;
    a2_v <= !rst && a1_v;
    a2_lo <= in_lo_q;
    a2_hi <= in_hi;
    if (a2_v) begin lo_q[l_wr] <= a2_lo; hi_q[l_wr] <= a2_hi; end
  end
  wire have = l_wr != t_rd;
  reg [7:0] tr;
  reg tb;
  wire [511:0] lo, hi;
  // A row's lines a beat a clock to the read-out and, flushing, the flush; its carried value with each beat.
  reg half;
  wire [511:0] c_beat = tb ? cbuf1[tr >> 4] : cbuf0[tr >> 4];
  wire [32*HD-1:0] ybuf = tb ? ybuf1 : ybuf0;
  wire [31:0] row_carried = c_beat[32 * tr[3:0] +: 32];
  wire r_ready, x_ready, r_ov, x_ov;
  wire [31:0] r_out;
  wire [512*W-1:0] x_out;
  wire [256*W-1:0] x_halves;
  wire held_line = W > 1 && !f16 && !half;
  wire feed = have && (held_line || (r_ready && (!flush || x_ready)));
  wire to_units = have && !held_line;
  wire last_half = W > 1 || !f16 || half;
  reg [511:0] row_lo;
  wire [512*W-1:0] row_beat;
  // The flush's out-stream asked as the side asks it, against a queue of sixteen lines: an ask holds room for the lines it
  // may bring (two at W > 1 in f32, else one) until it has passed the XD + 1 clocks after it, and the stream is asked
  // next clock (x_ask, a register) only while the queue's lines and the room every ask out holds, this clock's ask's
  // included, leave room for one ask more. The room the queue and the asks before this clock's leave is kept a clock
  // ahead for each size of ask (x_free1, x_free2), from the queue's next count, so the ask waits on a compare alone.
  localparam XD = 1 + @F16@;
  reg x_pend, x_tog, x_hi, x_ask;
  reg [255:0] x_low;
  reg [511:0] x_out_hi;
  reg [XD:0] x_hist;
  reg [4:0] w_n;
  reg signed [6:0] x_free1, x_free2;
  // A row open from its start until its last read-out and flushed line are out: done the clock it closes; room for the
  // next once closed (the lanes' top starts a lane once a row).
  reg row_open;
  wire closes = row_open && !start && !running && w_n == 5'd0;
  always @(posedge clk) begin
    done <= !rst && closes;
    if (start) row_open <= 1'b1; else if (closes) row_open <= 1'b0;
    if (rst) row_open <= 1'b0;
  end
  assign room = !row_open;
  // The asks out next clock but the one it makes: this clock's and the history's but its oldest.
  reg [2:0] x_held;
  integer xi;
  always @* begin
    x_held = {2'd0, x_ask};
    for (xi = 0; xi < XD; xi = xi + 1) x_held = x_held + {2'd0, x_hist[xi]};
  end
  wire x_two = W > 1 && !f16;
  wire [511:0] x_line_data;
  wire x_line_push;
  generate
    if (W == 1) begin : narrow
      assign row_beat = f16 ? (half ? hi : lo) : lo;
      logos_gdn_read_p #(.N(HD)) rd(.clk(clk), .rst(rst), .y(ybuf), .scale(scale), .in_valid(have && (!flush || x_ready)),
        .in_ready(r_ready), .in_beat(row_beat), .in_carried(row_carried), .out_valid(r_ov), .out_read(r_out));
      logos_gdn_flush_p #(.N(HD), .T_MAX(TMAX)) fl(.clk(clk), .rst(rst), .log_wr(f_log_wr && flush), .log_at(f_log_at),
        .log_beat(f_log_beat), .log_k(f_log_k), .log_v(f_log_v), .log_decay(f_log_decay), .log_beta(f_log_beta),
        .entries(count), .f16(f16), .in_valid(flush && have && r_ready), .in_ready(x_ready), .in_beat(row_beat),
        .out_valid(x_ov), .out_ready(x_ask), .out_beat(x_out), .out_half(x_halves));
      assign x_line_data = f16 ? {x_halves, x_low} : x_out;
      assign x_line_push = x_ov && (!f16 || x_hi);
    end else begin : wide
      assign row_beat = f16 ? {hi, lo} : {lo, row_lo};
      logos_gdn_read_w #(.N(HD), .W(W)) rd(.clk(clk), .rst(rst), .y(ybuf), .scale(scale), .in_valid(to_units && (!flush || x_ready)),
        .in_ready(r_ready), .in_beat(row_beat), .in_carried(row_carried), .out_valid(r_ov), .out_read(r_out));
      // The head's rows dealt over FLUSHES flushes in order, each holding HD / FLUSHES of them through the same passes;
      // its rows asked of one flush after another, each flush's once the one before has given its last, so they come
      // back in row order: each ask's beat comes the same clocks after it, so at most one flush gives a beat a clock.
      localparam [15:0] FB = HD / FLUSHES * (HD / (16 * W));
      localparam FS = FLUSHES > 1 ? $clog2(FLUSHES) : 1;
      localparam [FS-1:0] FLAST = FLUSHES - 1;
      reg [FS-1:0] f_in, f_out;
      reg [15:0] f_inb, f_outb;
      wire [FLUSHES-1:0] f_ready, f_ov;
      wire [512*W*FLUSHES-1:0] f_beats;
      wire [256*W*FLUSHES-1:0] f_halves;
      wire f_take = flush && to_units && r_ready && x_ready;
      genvar fk;
      for (fk = 0; fk < FLUSHES; fk = fk + 1) begin : part
        logos_gdn_flush_w #(.N(HD), .W(W), .T_MAX(TMAX), .ROWS(HD / FLUSHES), .FIRST(fk * (HD / FLUSHES))) fl(.clk(clk), .rst(rst),
          .log_wr(fw_wr && flush), .log_at(f_log_at), .log_beat(fw_beat), .log_k(fw_k[512*W-1:0]), .log_v(fw_v[512*W-1:0]),
          .log_decay(f_log_decay), .log_beta(f_log_beta), .entries(count), .f16(f16),
          .in_valid(flush && to_units && r_ready && f_in == fk), .in_ready(f_ready[fk]), .in_beat(row_beat),
          .out_valid(f_ov[fk]), .out_ready(x_ask && f_out == fk), .out_beat(f_beats[512*W*fk +: 512*W]),
          .out_half(f_halves[256*W*fk +: 256*W]));
      end
      assign x_ready = f_ready[f_in];
      assign x_ov = |f_ov;
      reg [512*W-1:0] f_beat;
      reg [256*W-1:0] f_half;
      integer fo;
      always @* begin
        f_beat = f_beats[512*W-1:0];
        f_half = f_halves[256*W-1:0];
        for (fo = 1; fo < FLUSHES; fo = fo + 1)
          if (f_ov[fo]) begin f_beat = f_beats[512*W*fo +: 512*W]; f_half = f_halves[256*W*fo +: 256*W]; end
      end
      assign x_out = f_beat;
      assign x_halves = f_half;
      always @(posedge clk) begin
        if (rst) begin f_in <= {FS{1'b0}}; f_inb <= 16'd0; f_out <= {FS{1'b0}}; f_outb <= 16'd0; end
        else begin
          if (f_take) begin
            if (f_inb == FB - 16'd1) begin f_inb <= 16'd0; f_in <= (f_in == FLAST) ? {FS{1'b0}} : f_in + 1'b1; end
            else f_inb <= f_inb + 16'd1;
          end
          if (f_ov[f_out]) begin
            if (f_outb == FB - 16'd1) begin f_outb <= 16'd0; f_out <= (f_out == FLAST) ? {FS{1'b0}} : f_out + 1'b1; end
            else f_outb <= f_outb + 16'd1;
          end
        end
      end
      assign x_line_data = f16 ? x_halves[511:0] : (x_pend ? x_out_hi : x_out[511:0]);
      assign x_line_push = x_ov || x_pend;
    end
  endgenerate
  wire pop = have && feed && last_half;
  wire [QB-1:0] t_next = rst ? {QB{1'b0}} : t_rd + {{(QB-1){1'b0}}, pop};
  wire landing = a2_v && l_wr == t_next;
  wire [8:0] tr_n = (rq_take && t_wr == t_next) ? {sb, row_req[7:0]} : t_row[t_next];
  // The head's line: the queue's word at the next head, registered as block RAM reads it, or the line landing there
  // that clock, registered beside it.
  reg [511:0] lo_m, hi_m, lo_b, hi_b;
  reg land_d;
  always @(posedge clk) begin
    tr <= tr_n[7:0];
    tb <= tr_n[8];
    lo_m <= lo_q[t_next];
    hi_m <= hi_q[t_next];
    lo_b <= a2_lo;
    hi_b <= a2_hi;
    land_d <= landing;
  end
  assign lo = land_d ? lo_b : lo_m;
  assign hi = land_d ? hi_b : hi_m;
  // The flushed rows packed into lines and queued for memory, the flushed head's rows in order.
  `LOGOS_RAMSTYLE_ASYNC(16) reg [511:0] wfifo_d [0:15];
  `LOGOS_RAMSTYLE_ASYNC(16) reg [31:0] wfifo_l [0:15];
  reg [3:0] w_wr, w_rd;
  reg [31:0] xh;
  reg [15:0] x_row, x_line;
  wire [47:0] x_byte = base + mem_off + (xh * HD + x_row) * HD * eb + 64 * x_line;
  assign wq_valid = w_n != 5'd0;
  assign wq_line = wfifo_l[w_rd];
  assign wq_data = wfifo_d[w_rd];
  wire w_pop = wq_valid && wq_ready;
  wire [4:0] w_next = w_n + (x_line_push ? 5'd1 : 5'd0) - (w_pop ? 5'd1 : 5'd0);
  // The read-outs, head by head behind the rows: the head and row the next is of.
  reg [511:0] o_beat;
  reg [31:0] oh;
  reg [15:0] o_row;
  wire [31:0] o_wa = out_beat + ((oh * HD + o_row) >> 4);
  wire outs_done = oh >= nv;
  assign busy = running || w_n != 5'd0;
  always @(posedge clk) begin
    f_log_wr <= 1'b0; f_q_wr <= 1'b0; f_start <= 1'b0; st_we <= 1'b0; q_pending <= 1'b0; fw_wr <= 1'b0;
    x_tog <= !x_tog;
    x_hist <= {x_hist[XD-1:0], x_ask};
    x_ask <= !rst && (!x_two || !x_tog) && (x_two ? x_free2 >= (x_ask ? 7'sd4 : 7'sd2) : x_free1 >= (x_ask ? 7'sd2 : 7'sd1));
    x_free1 <= 7'sd16 - $signed({2'b0, w_next}) - $signed({4'b0, x_held});
    x_free2 <= 7'sd16 - $signed({2'b0, w_next}) - $signed({3'b0, x_held, 1'b0});
    if (rq_take) t_wr <= t_wr + 1'b1;
    if (rs_valid) l_in <= l_in + 1'b1;
    if (a2_v) l_wr <= l_wr + 1'b1;
    owed <= owed + (rq_take ? 10'd1 : 10'd0) - (pop ? 10'd1 : 10'd0);
    owed_b0 <= owed_b0 + ((rq_take && !sb) ? 10'd1 : 10'd0) - ((pop && !tb) ? 10'd1 : 10'd0);
    owed_b1 <= owed_b1 + ((rq_take && sb) ? 10'd1 : 10'd0) - ((pop && tb) ? 10'd1 : 10'd0);
    // The fold's run: started by the loader, finished once it falls idle after its start.
    if (f_start) f_run <= 1'b1;
    else if (f_run && !f_busy) begin f_run <= 1'b0; folded <= folded + 8'd1; end
    // ── the loader ──
    if (q_lo) begin f_q_wr <= 1'b1; f_q_slot <= q_lo_slot; f_q_beat <= q_lo_at; f_q_data <= st_rd; end
    if (w2_v) case (w2_k)
      K_DEC: ldec_h <= w_elem;
      K_BETA: beta_h <= w_elem;
      K_KEY: key_h <= w_beat;
      default: begin
        f_log_wr <= 1'b1; f_log_at <= w2_e; f_log_beat <= w2_j; f_log_k <= key_h; f_log_v <= w_beat;
        f_log_decay <= ldec_h; f_log_beta <= beta_h;
        if (!w2_j[0]) begin lo_k <= key_h; lo_v <= w_beat; end
        else begin fw_wr <= W > 1; fw_beat <= w2_j >> 1; fw_k <= {key_h, lo_k}; fw_v <= {w_beat, lo_v}; end
      end
    endcase
    case (ls)
      L_IDLE: if (running && lh < nv && can_load) begin lslot <= 8'd0; qj <= 16'd0; ls <= L_Q; end
        else if (running && lh >= nv) ls <= L_DONE;
      L_Q: if (qj == BEATS) begin ge <= 8'd0; gi <= 16'd0; e_at <= par ? COPY_LINES * 32'd64 : 32'd0; ls <= L_LOG; end
        else if (st_gnt) begin q_pending <= 1'b1; q_at <= qj; qj <= qj + 16'd1; end
      L_LOG: if (walking) begin
          if (gi == 16'd1 + 2 * BEATS) begin gi <= 16'd0; ge <= ge + 8'd1; e_at <= e_at + entry; end else gi <= gi + 16'd1;
        end else ls <= L_TAIL;
      // The walk's last lines through the copy and the widen, then the batch's next head or its fold.
      L_TAIL: if (!w1_v && !w2_v && !f_log_wr && (SRL == 1 || (!q_pending && !q_flight))) begin
        lh <= lh + STEP;
        lg <= (lg + g_step >= nk) ? lg + g_step - nk : lg + g_step;
        if (lslot + 8'd1 == bs || lh + STEP >= nv) ls <= L_FOLD;
        else begin lslot <= lslot + 8'd1; qj <= 16'd0; ls <= L_Q; end
      end
      L_FOLD: begin f_start <= 1'b1; fold_bank <= loaded[0]; loaded <= loaded + 8'd1; ls <= L_IDLE; end
      default: ;
    endcase
    // ── the streamer ──
    if (feed) begin
      if (W > 1) begin
        t_rd <= t_rd + 1'b1;
        if (!f16) begin half <= !half; if (held_line) row_lo <= lo; end
      end else if (last_half) begin half <= 1'b0; t_rd <= t_rd + 1'b1; end else half <= 1'b1;
    end
    if (x_line_push) begin
      wfifo_d[w_wr] <= x_line_data;
      wfifo_l[w_wr] <= x_byte[37:6];
      w_wr <= w_wr + 4'd1;
      if (x_line + 16'd1 == row_lines) begin
        x_line <= 16'd0;
        if (x_row + 16'd1 == HD_ROWS) begin x_row <= 16'd0; xh <= xh + STEP; end else x_row <= x_row + 16'd1;
      end else x_line <= x_line + 16'd1;
    end
    if (W == 1 && x_ov) begin
      if (f16 && !x_hi) begin x_low <= x_halves[255:0]; x_hi <= 1'b1; end else x_hi <= 1'b0;
    end
    x_pend <= W > 1 && x_ov && !f16;
    if (x_ov) x_out_hi <= x_out[512*W-1:512*W-512];
    if (w_pop) w_rd <= w_rd + 4'd1;
    w_n <= w_next;
    if (r_ov) begin
      o_beat[32 * o_row[3:0] +: 32] <= r_out;
      if (o_row[3:0] == 4'd15) begin st_we <= 1'b1; st_wa <= o_wa[SA-1:0]; st_wd <= {r_out, o_beat[479:0]}; end
      if (o_row + 16'd1 == HD_ROWS) begin o_row <= 16'd0; oh <= oh + STEP; end else o_row <= o_row + 16'd1;
    end
    case (ss)
      // A folded batch's next head, once every line of the head two back, the last to take its buffer, is through the
      // units.
      S_IDLE: if (running && sh < nv && folded != copied && (sb ? owed_b1 : owed_b0) == 10'd0) begin cb <= 16'd0; ss <= S_COPY; end
        else if (running && sh >= nv) ss <= S_DONE;
      S_COPY: if (cb == BEATS + 16'd2) begin cb <= 16'd0; row_req <= 16'd0; ri <= 16'd0; ss <= S_ROWS; end else cb <= cb + 16'd1;
      S_ROWS: begin
        if (rq_take) begin
          if (ri + 16'd1 == row_lines) begin ri <= 16'd0; row_req <= row_req + 16'd1; end else ri <= ri + 16'd1;
        end
        if (row_req == HD_ROWS) ss <= flush ? S_DRAIN : S_NEXT;
      end
      // Flushing: the head's read-outs and flushed rows out before the next head's log goes into the flush.
      S_DRAIN: if (oh == sh + STEP && x_row == 16'd0 && xh == sh + STEP && w_n == 5'd0) ss <= S_NEXT;
      S_NEXT: begin
        sh <= sh + STEP;
        sb <= !sb;
        if (sslot + 8'd1 == bs || sh + STEP >= nv) begin
          sslot <= 8'd0; copied <= copied + 8'd1;
          if (flush) drained <= drained + 8'd1;
        end else sslot <= sslot + 8'd1;
        ss <= S_IDLE;
      end
      default: ;
    endcase
    // The row is done once every head's read-outs and flushed rows are out.
    if (running && ls == L_DONE && ss == S_DONE && outs_done && (!flush || xh >= nv) && w_n == 5'd0) begin
      running <= 1'b0; ls <= L_IDLE; ss <= S_IDLE;
    end
    if (start && !running && first < nv) begin
      running <= 1'b1; lh <= first; lg <= first_g; sh <= first; oh <= first; xh <= first; o_row <= 16'd0;
      x_row <= 16'd0; x_line <= 16'd0; x_hi <= 1'b0; sslot <= 8'd0; sb <= 1'b0;
      loaded <= 8'd0; folded <= 8'd0; copied <= 8'd0; drained <= 8'd0; ls <= L_IDLE; ss <= S_IDLE;
    end
    if (rst) begin
      running <= 1'b0; ls <= L_IDLE; ss <= S_IDLE; f_run <= 1'b0; t_wr <= {QB{1'b0}}; t_rd <= {QB{1'b0}}; l_in <= {QB{1'b0}};
      l_wr <= {QB{1'b0}}; w_wr <= 4'd0; w_rd <= 4'd0; w_n <= 5'd0; half <= 1'b0; owed <= 10'd0; owed_b0 <= 10'd0;
      owed_b1 <= 10'd0; x_pend <= 1'b0;
      x_hist <= {(XD+1){1'b0}}; x_tog <= 1'b0; x_free1 <= 7'sd16; x_free2 <= 7'sd16;
    end
  end
endmodule
"#;

/// The recurrence on head lanes (`logos_gdn_lanes`): a coordinator (`logos_gdn_side` in MODE 1) and L lanes
/// (`logos_gdn_lane`), the log read once into every lane's copy of it.
const GDN_LANE_T_VERILOG: &str = r#"
// A head lane of logos_gdn_lanes under a trailing flush of WINDOW tokens: value heads first, first + STEP, … below nv of
// each token row it takes (start, while it has room: two rows at most), each row's fields taken with it — its region, its
// query's and read-outs' store beats, its log's count and the copy of the lane's log that holds it (lb_ra, a line the
// clock after its address; each copy LBA bits of address, the second COPY_LINES lines in), and whether it replays the
// log's first WINDOW tokens into each head's share of SHARE = HD / WINDOW rows from its row `in_from_row` (the log holds
// a window past the first). The rows go through back to back, a row's first batch loaded and folded while the last
// row's last batches stream and
// replay; `done` the clock after each row's last read-out and flushed line are out, in order. A loader takes the heads a
// batch at a time, H heads (logos_gdn_fold_m's
// slots): each head's query from the store on the clocks st_gnt grants the read it asks (st_rq) and its log from the
// copy a line a clock into its slot, the log's first WINDOW tokens into the head's slot of the flush's bank for the
// batch as well, then folds the batch into a bank, keeping each head's fold over the tokens after those WINDOW too; the
// next batch is loaded and folded into the other bank while this one's rows stream, once the flush has given back the
// rows of the batch that last held its flush bank. A streamer takes each head of a folded batch: its two folds copied
// out of the bank, then its rows from memory (rq/rs, in the order asked, at most OUTSTANDING lines owed, no wait taken
// on an answer) through the read-out — a row below `from_row` through the fold over the newer tokens, its share replayed —
// and each row of its share into its slot of the flush's bank as the read-out takes it; once a batch's rows are all
// through, its bank is replayed (logos_gdn_flush_m), and its flushed rows go back where they were read (wq); each
// sixteen rows' read-outs leave as a store beat (st_we). Busy while a row it has taken is not done or a flushed line is
// queued.
module logos_gdn_lane_t #(parameter HD = 128, TMAX = 16, SA = 10, parameter [8:0] OUTSTANDING = 32, parameter W = 1,
                          parameter [31:0] STEP = 4, parameter LBA = 13, WINDOW = 8, COPY_LINES = 4096, SRL = 1)
  (input clk, input rst,
   input start, output busy, output room, output reg done,
   input [31:0] in_region, input [31:0] mem_off, input [31:0] entry, input [31:0] nk, input [31:0] nv,
   input [31:0] fval, input [31:0] fdec, input [31:0] fgate, input [31:0] scale, input f16,
   input [SA-1:0] in_q_beat, input [SA-1:0] in_out_beat,
   input [31:0] first, input [7:0] in_count, input in_flush, input [15:0] in_from_row, input in_par,
   output st_rq, input st_gnt, output [SA-1:0] st_ra, input [511:0] st_rd,
   output reg st_we, output reg [SA-1:0] st_wa, output reg [511:0] st_wd,
   output [LBA:0] lb_ra, input [511:0] lb_rd,
   output rq_valid, input rq_ready, output [31:0] rq_line, input rs_valid, input [511:0] rs_data,
   output wq_valid, input wq_ready, output [31:0] wq_line, output [511:0] wq_data);
  localparam H = @FADD@;
  localparam [15:0] BEATS = HD / 16, HD_ROWS = HD;
  localparam [15:0] SHARE = HD / WINDOW;
  localparam [7:0] WINDOW8 = WINDOW;
  localparam K_DEC = 2'd0, K_BETA = 2'd1, K_KEY = 2'd2, K_VAL = 2'd3;
  // The rows the lane has taken, two at most, each row's fields in the entry its count's low bit names: its region, its
  // query's and read-outs' store beats, its log's count, whether it replays, its share's first row, and the copy of the
  // log it reads (its first line COPY_LINES lines in for the second). Rows go through in order — taken (in_n), loaded
  // (ld_n), streamed (st_n), read out (o_n), done (dn_n) — each stage holding its row's fields in registers of its own
  // from the clock it takes the row up.
  reg [7:0] in_n, ld_n, st_n, o_n, dn_n;
  `LOGOS_RAMSTYLE(2) reg [31:0] e_region [0:1];
  `LOGOS_RAMSTYLE(2) reg [SA-1:0] e_q [0:1];
  `LOGOS_RAMSTYLE(2) reg [SA-1:0] e_out [0:1];
  `LOGOS_RAMSTYLE(2) reg [7:0] e_count [0:1];
  `LOGOS_RAMSTYLE(2) reg [15:0] e_from [0:1];
  reg [1:0] e_flush, e_par;
  always @(posedge clk)
    if (start) begin
      e_region[in_n[0]] <= in_region; e_q[in_n[0]] <= in_q_beat; e_out[in_n[0]] <= in_out_beat; e_count[in_n[0]] <= in_count;
      e_flush[in_n[0]] <= in_flush; e_from[in_n[0]] <= in_from_row; e_par[in_n[0]] <= in_par;
    end
  assign room = in_n - dn_n < 8'd2;
  wire [31:0] eb = f16 ? 32'd2 : 32'd4;
  wire [31:0] row_bytes = HD * eb;
  wire [15:0] row_lines = row_bytes[21:6];
  localparam [7:0] HS = H;
  wire [7:0] bs = HS;
  // The key group a head's step moves by, and the first head's, each a register of the job's fields.
  function [31:0] step_mod(input [31:0] k);
    reg [31:0] r;
    integer si;
    begin
      r = STEP;
      for (si = 0; si < 4; si = si + 1) if (r >= k) r = r - k;
      step_mod = r;
    end
  endfunction
  reg [31:0] g_step, first_g;
  always @(posedge clk) begin
    g_step <= step_mod(nk);
    first_g <= (first >= nk) ? ((first - nk >= nk) ? ((first - 32'd2 * nk >= nk) ? first - 32'd3 * nk : first - 32'd2 * nk) : first - nk) : first;
  end
  // Batches, every row's in turn: loaded (their folds started), folded (finished) and copied out (their fold bank free).
  // The folds' two banks go by the batch's count's low bit; the flush's three by the loaded and copied batches' counts
  // mod 3 (lbank, cbank), a bank busy (fb_busy) from the load of a batch that replays until its replay's last line is
  // queued.
  reg [7:0] loaded, folded, copied;
  reg [2:0] fb_busy;
  // ── The loader ──────────────────────────────────────────────────────────────────────────────────────────────────
  localparam [2:0] L_IDLE = 3'd0, L_Q = 3'd1, L_LOG = 3'd2, L_TAIL = 3'd3, L_FOLD = 3'd4;
  reg [2:0] ls;
  reg [31:0] lh, lg;
  reg [7:0] lslot;
  reg [15:0] qj;
  reg q_pending;
  reg [15:0] q_at;
  // A query beat asked (q_pending, its beat and slot, the clock after its address) and out of the store SRL clocks after
  // its address (q_lo, q_lo_at, q_lo_slot); a batch's fold waits until none is on its way.
  wire q_lo, q_flight;
  wire [15:0] q_lo_at;
  wire [7:0] q_lo_slot;
  logos_landing #(.W(24), .D(SRL - 1)) qland(.clk(clk), .rst(rst), .in_v(q_pending), .in_d({q_at, lslot}), .out_v(q_lo),
    .out_d({q_lo_at, q_lo_slot}), .flight(q_flight));
  wire f_busy;
  reg f_start, f_run;
  reg [1:0] lbank, cbank;
  // The loader's row: its log's count, whether it replays, the copy it reads, its query's beats.
  reg l_act, l_flush, l_par;
  reg [7:0] l_count;
  reg [SA-1:0] l_q;
  wire can_load = !f_busy && !f_start && !f_run && loaded - copied < 8'd2 && !(l_flush && fb_busy[lbank]);
  wire [31:0] q_ra = l_q + ((lg * HD) >> 4) + qj;
  assign st_rq = ls == L_Q && qj != BEATS;
  assign st_ra = q_ra[SA-1:0];
  // The log walk of head lh (key group lg): for each entry its decay exponent's line, its gate's, then its key's and its
  // values' beats in turn; each line's place in the copy and the byte of its field within it.
  reg [7:0] ge;
  reg [15:0] gi;
  reg [31:0] e_at;
  wire [1:0] g_kind = gi == 16'd0 ? K_DEC : gi == 16'd1 ? K_BETA : gi[0] ? K_VAL : K_KEY;
  wire [15:0] g_dj = (gi - 16'd2) >> 1;
  wire [31:0] g_off = g_kind == K_DEC ? (fdec + lh) * eb : g_kind == K_BETA ? (fgate + lh) * eb :
                      g_kind == K_KEY ? (lg * HD + 16 * g_dj) * eb : (fval + lh * HD + 16 * g_dj) * eb;
  wire [31:0] g_byte = e_at + g_off;
  wire walking = ls == L_LOG && ge != l_count;
  assign lb_ra = g_byte[LBA+6:6];
  // A copy line's tag beside it: its kind, entry, beat and byte; the line a clock after its address, registered with its
  // tag the clock after that, then widened.
  reg w1_v, w2_v;
  reg [1:0] w1_k, w2_k;
  reg [7:0] w1_e, w2_e;
  reg [15:0] w1_j, w2_j;
  reg [5:0] w1_o, w2_o;
  reg [511:0] w2_d;
  wire [511:0] w_lo, w_hi;
  wire [31:0] w_e16;
  genvar l;
  generate
    for (l = 0; l < 16; l = l + 1) begin : walk_widen
      logos_f16_to_f32 lo(.h(w2_d[16*l +: 16]), .x(w_lo[32*l +: 32]));
      logos_f16_to_f32 hi(.h(w2_d[256 + 16*l +: 16]), .x(w_hi[32*l +: 32]));
    end
  endgenerate
  wire [31:0] w_raw = w2_d[8 * w2_o +: 32];
  logos_f16_to_f32 we(.h(w2_d[8 * w2_o +: 16]), .x(w_e16));
  wire [511:0] w_beat = f16 ? (w2_o[5] ? w_hi : w_lo) : w2_d;
  wire [31:0] w_elem = f16 ? w_e16 : w_raw;
  always @(posedge clk) begin
    w1_v <= !rst && walking;
    w1_k <= g_kind; w1_e <= ge; w1_j <= g_dj; w1_o <= g_byte[5:0];
    w2_v <= !rst && w1_v;
    w2_k <= w1_k; w2_e <= w1_e; w2_j <= w1_j; w2_o <= w1_o; w2_d <= lb_rd;
  end
  // The fold's and the flush's log writes: a value beat with the key beat, decay exponent and gate before it.
  reg [31:0] ldec_h, beta_h;
  reg [511:0] key_h;
  reg f_log_wr, f_q_wr, fw_wr;
  reg [7:0] f_log_at, f_q_slot;
  reg [15:0] f_log_beat, f_q_beat, fw_beat;
  reg [511:0] f_log_k, f_log_v, f_q_data, lo_k, lo_v;
  reg [1023:0] fw_k, fw_v;
  reg [31:0] f_log_decay, f_log_beta;
  reg fold_bank;
  // The fold's run: its batch's tokens and the older ones it keeps a fold past, held through the run.
  reg [7:0] f_entries, f_older;
  // ── The streamer ────────────────────────────────────────────────────────────────────────────────────────────────
  localparam [2:0] S_IDLE = 3'd0, S_COPY = 3'd1, S_ROWS = 3'd2, S_NEXT = 3'd3, S_FLUSH = 3'd4;
  reg [2:0] ss;
  reg [31:0] sh, s_h0;
  reg [7:0] sslot;
  // The streamer's row: its region, whether it replays, its share's first row, its parity.
  reg s_act, s_flush, s_par;
  reg [31:0] s_region;
  reg [15:0] s_from;
  wire [47:0] s_base = {16'd0, s_region} * 48'd64;
  wire batch_end = sslot + 8'd1 == bs || sh + STEP >= nv;
  reg [15:0] cb;
  wire [511:0] f_y, f_c;
  wire [31:0] f_decay;
  wire [511:0] f_yn, f_cn;
  logos_gdn_fold_m #(.N(HD), .T_MAX(TMAX), .TRAILING(1)) fold(.clk(clk), .rst(rst), .log_wr(f_log_wr), .log_slot(lslot), .log_at(f_log_at),
    .log_beat(f_log_beat), .log_k(f_log_k), .log_v(f_log_v), .log_decay(f_log_decay), .log_beta(f_log_beta), .q_wr(f_q_wr),
    .q_slot(f_q_slot), .q_beat(f_q_beat), .q_data(f_q_data), .start(f_start), .bank(fold_bank), .entries(f_entries), .older(f_older),
    .busy(f_busy), .rd_bank(copied[0]), .rd_slot(sslot), .rd_beat(cb), .y_beat(f_y), .carried_beat(f_c),
    .ynew_beat(f_yn), .cnew_beat(f_cn), .rd_token(8'd0), .decay_out(f_decay));
  // The streamed head's folded query and collected values: a beat of each asked a clock (cb), out of the fold's bank two
  // clocks after, copied in the clock after that; the copy over once its last beat is in.
  // Each head's folds into buffer sb, the buffer the head before did not take.
  reg sb;
  reg [32*HD-1:0] ybuf0, ybuf1, ybuf_n0, ybuf_n1;
  `LOGOS_RAMSTYLE_ASYNC(BEATS) reg [511:0] cbuf0 [0:BEATS-1];
  `LOGOS_RAMSTYLE_ASYNC(BEATS) reg [511:0] cbuf1 [0:BEATS-1];
  `LOGOS_RAMSTYLE_ASYNC(BEATS) reg [511:0] cbuf_n0 [0:BEATS-1];
  `LOGOS_RAMSTYLE_ASYNC(BEATS) reg [511:0] cbuf_n1 [0:BEATS-1];
  reg cp_a, cp_m, cp_v, cp_b;
  reg [15:0] cp_ab, cp_mb, cp_at;
  reg [511:0] cp_y, cp_c, cp_yn, cp_cn;
  always @(posedge clk) begin
    cp_a <= !rst && ss == S_COPY && cb < BEATS;
    cp_ab <= cb;
    cp_m <= !rst && cp_a;
    cp_mb <= cp_ab;
    cp_v <= !rst && cp_m;
    cp_at <= cp_mb;
    cp_b <= sb;
    cp_y <= f_y;
    cp_c <= f_c;
    cp_yn <= f_yn;
    cp_cn <= f_cn;
    if (cp_v && !cp_b) begin cbuf0[cp_at] <= cp_c; cbuf_n0[cp_at] <= cp_cn; end
    if (cp_v && cp_b) begin cbuf1[cp_at] <= cp_c; cbuf_n1[cp_at] <= cp_cn; end
  end
  genvar cq;
  generate
    for (cq = 0; cq < BEATS; cq = cq + 1) begin : copy_in
      always @(posedge clk) begin
        if (cp_v && !cp_b && cp_at == cq) begin ybuf0[512*cq +: 512] <= cp_y; ybuf_n0[512*cq +: 512] <= cp_yn; end
        if (cp_v && cp_b && cp_at == cq) begin ybuf1[512*cq +: 512] <= cp_y; ybuf_n1[512*cq +: 512] <= cp_yn; end
      end
    end
  endgenerate
  // The rows asked: each its row; lines answered in order, registered as they come, binary16 halves widened a clock
  // on and registered, then into the lines.
  localparam QB = OUTSTANDING > 9'd32 ? $clog2(OUTSTANDING) + 1 : 6;
  localparam QD = 1 << QB;
  reg [15:0] row_req, ri;
  reg [26:0] t_row [0:QD-1];
  reg [511:0] lo_q [0:QD-1];
  reg [511:0] hi_q [0:QD-1];
  reg [QB-1:0] t_wr, t_rd, l_in, l_wr;
  // The lines asked and not yet through the read-out, and of them those of each buffer's head.
  reg [9:0] owed, owed_b0, owed_b1;
  wire row_walk = ss == S_ROWS && row_req != HD_ROWS;
  assign rq_valid = row_walk && owed < {1'b0, OUTSTANDING};
  wire rq_take = rq_valid && rq_ready;
  // The walk: it starts at its copy's end, and each line taken steps it a line, its row's last to the next row's first.
  // A row's line tagged with where its row falls — below the share (read through the newer fold), in it (a share row the
  // replay takes, and its place there) — and its head's slot, buffer and row. An ask's line (r_byte) and its tag (t_new)
  // are registers the walk moves with: the line a line on with each line taken (a head's rows are whole lines, one after
  // another), the tag its row's or the next row's, each computed from registers alone. The batch's head, region, slot,
  // buffer and share hold from before its copy to the walk's end.
  wire walk_starts = ss == S_COPY && cb == BEATS + 16'd2;
  wire walk_takes = ss == S_ROWS && rq_take;
  wire row_ends = walk_takes && ri + 16'd1 == row_lines;
  wire [15:0] row_first = 16'd0, row_next = row_req + 16'd1;
  wire [26:0] tag_first = {row_first[7:0] - s_from[7:0], s_flush && row_first >= s_from && row_first < s_from + SHARE,
                           row_first < s_from, sslot, sb, row_first[7:0]};
  wire [26:0] tag_here = {row_req[7:0] - s_from[7:0], s_flush && row_req >= s_from && row_req < s_from + SHARE,
                          row_req < s_from, sslot, sb, row_req[7:0]};
  wire [26:0] tag_next = {row_next[7:0] - s_from[7:0], s_flush && row_next >= s_from && row_next < s_from + SHARE,
                          row_next < s_from, sslot, sb, row_next[7:0]};
  wire [47:0] walk_first = s_base + mem_off + sh * HD * HD * eb;
  reg [47:0] r_byte;
  reg [26:0] t_new;
  always @(posedge clk) begin
    r_byte <= walk_starts ? walk_first : walk_takes ? r_byte + 48'd64 : r_byte;
    t_new <= walk_starts ? tag_first : row_ends ? tag_next : tag_here;
  end
  assign rq_line = r_byte[37:6];
  reg a1_v;
  reg [511:0] a1_d;
  wire [511:0] in_lo, in_hi;
  generate
    for (l = 0; l < 16; l = l + 1) begin : row_widen
      logos_f16_to_f32 lo(.h(a1_d[16*l +: 16]), .x(in_lo[32*l +: 32]));
      logos_f16_to_f32 hi(.h(a1_d[256 + 16*l +: 16]), .x(in_hi[32*l +: 32]));
    end
  endgenerate
  wire [511:0] in_lo_q = f16 ? in_lo : a1_d;
  // A line widened, registered (a2) before its queue and the head's bypass take it.
  (* maxfan = 64 *) reg a2_v;
  reg [511:0] a2_lo, a2_hi;
  always @(posedge clk) begin
    if (rq_take) t_row[t_wr] <= t_new;
    a1_v <= !rst && rs_valid;
    a1_d <= rs_data;
    a2_v <= !rst && a1_v;
    a2_lo <= in_lo_q;
    a2_hi <= in_hi;
    if (a2_v) begin lo_q[l_wr] <= a2_lo; hi_q[l_wr] <= a2_hi; end
  end
  wire have = l_wr != t_rd;
  reg [7:0] tr, tj;
  // The line's tag bits, the newer and the buffer choosing each of the read-out query's 8,192 bits, copied a copy each
  // 64 loads.
  (* maxfan = 64 *) reg tb, tn;
  reg tsh;
  reg [7:0] ts;
  wire [511:0] lo, hi;
  // A row's lines a beat a clock to the read-out and, flushing, the flush; its carried value with each beat.
  reg half;
  // A row below the share replayed reads through the fold over the newer tokens; the read-outs take a group's fold with
  // its first beat, and a share is whole groups.
  wire newer = tn;
  wire [511:0] c_beat = tb ? cbuf1[tr >> 4] : cbuf0[tr >> 4], c_beat_n = tb ? cbuf_n1[tr >> 4] : cbuf_n0[tr >> 4];
  wire [31:0] row_carried = newer ? c_beat_n[32 * tr[3:0] +: 32] : c_beat[32 * tr[3:0] +: 32];
  wire [32*HD-1:0] row_y = newer ? (tb ? ybuf_n1 : ybuf_n0) : (tb ? ybuf1 : ybuf0);
  wire r_ready, x_ready, r_ov, x_ov;
  wire [31:0] r_out;
  wire [512*W-1:0] x_out;
  wire [256*W-1:0] x_halves;
  wire held_line = W > 1 && !f16 && !half;
  wire feed = have && (held_line || r_ready);
  wire to_units = have && !held_line;
  wire last_half = W > 1 || !f16 || half;
  reg [511:0] row_lo;
  wire [512*W-1:0] row_beat;
  // The flush's out-stream asked as the side asks it, against a queue of sixteen lines: an ask holds room for the lines it
  // may bring (two at W > 1 in f32, else one) until it has passed the XD + 1 clocks after it (logos_gdn_flush_m's beat
  // comes XD clocks after its ask), and the stream is asked
  // next clock (x_ask, a register) only while the queue's lines and the room every ask out holds, this clock's ask's
  // included, leave room for one ask more. The room the queue and the asks before this clock's leave is kept a clock
  // ahead for each size of ask (x_free1, x_free2), from the queue's next count, so the ask waits on a compare alone.
  localparam XD = 2 + @F16@;
  reg x_pend, x_tog, x_hi, x_ask;
  reg [255:0] x_low;
  reg [511:0] x_out_hi;
  reg [XD:0] x_hist;
  reg [4:0] w_n;
  reg signed [6:0] x_free1, x_free2;
  // The asks out next clock but the one it makes: this clock's and the history's but its oldest.
  reg [2:0] x_held;
  integer xi;
  always @* begin
    x_held = {2'd0, x_ask};
    for (xi = 0; xi < XD; xi = xi + 1) x_held = x_held + {2'd0, x_hist[xi]};
  end
  wire x_two = W > 1 && !f16;
  wire [511:0] x_line_data;
  wire x_line_push;
  // The share's rows: each beat the read-out takes of a row the token replays, written the clock after into the slot of
  // its head in the streamed batch's flush bank at its place in the share (its line's tag).
  localparam [15:0] UB = HD / (16 * W);
  reg [15:0] u_bt;
  reg fm_wr, fm_start;
  reg [1:0] fm_wbank, fm_sbank;
  reg [7:0] fm_slot, fm_slots;
  reg [15:0] fm_j, fm_beat;
  reg [512*W-1:0] fm_data;
  wire [2:0] fm_free;
  wire u_take = (W > 1 ? to_units : have) && r_ready;
  wire in_share = tsh;
  always @(posedge clk) begin
    fm_wr <= !rst && u_take && in_share;
    fm_slot <= ts; fm_j <= {8'd0, tj}; fm_beat <= u_bt; fm_data <= row_beat; fm_wbank <= cbank;
    if (rst) u_bt <= 16'd0;
    else if (u_take) u_bt <= (u_bt == UB - 16'd1) ? 16'd0 : u_bt + 16'd1;
  end
  // The flush's log: the log's first WINDOW tokens of each head the loader walks of a row that replays, into its slot of
  // the loaded batch's bank, at the flush's beat (two of the fold's at W > 1).
  wire fl_wr = l_flush && f_log_at < WINDOW8 && (W > 1 ? fw_wr : f_log_wr);
  reg [15:0] fm_first;
  logos_gdn_flush_m #(.N(HD), .W(W), .T_MAX(WINDOW), .SHARE(SHARE), .SLOTS(H)) fl(.clk(clk), .rst(rst),
    .log_wr(fl_wr), .log_bank(lbank), .log_slot(lslot), .log_at(f_log_at), .log_beat(W > 1 ? fw_beat : f_log_beat),
    .log_k(W > 1 ? fw_k[512*W-1:0] : f_log_k[512*W-1:0]), .log_v(W > 1 ? fw_v[512*W-1:0] : f_log_v[512*W-1:0]),
    .log_decay(f_log_decay), .log_beta(f_log_beta),
    .row_wr(fm_wr), .row_bank(fm_wbank), .row_slot(fm_slot), .row_j(fm_j), .row_beat(fm_beat), .row_data(fm_data),
    .start(fm_start), .bank(fm_sbank), .entries(WINDOW8), .slots(fm_slots), .first(fm_first), .f16(f16), .free(fm_free),
    .out_valid(x_ov), .out_ready(x_ask), .out_beat(x_out), .out_half(x_halves));
  generate
    if (W == 1) begin : narrow
      assign row_beat = f16 ? (half ? hi : lo) : lo;
      logos_gdn_read_p #(.N(HD)) rd(.clk(clk), .rst(rst), .y(row_y), .scale(scale), .in_valid(have),
        .in_ready(r_ready), .in_beat(row_beat), .in_carried(row_carried), .out_valid(r_ov), .out_read(r_out));
      assign x_line_data = f16 ? {x_halves, x_low} : x_out;
      assign x_line_push = x_ov && (!f16 || x_hi);
    end else begin : wide
      assign row_beat = f16 ? {hi, lo} : {lo, row_lo};
      logos_gdn_read_w #(.N(HD), .W(W)) rd(.clk(clk), .rst(rst), .y(row_y), .scale(scale), .in_valid(to_units),
        .in_ready(r_ready), .in_beat(row_beat), .in_carried(row_carried), .out_valid(r_ov), .out_read(r_out));
      assign x_line_data = f16 ? x_halves[511:0] : (x_pend ? x_out_hi : x_out[511:0]);
      assign x_line_push = x_ov || x_pend;
    end
  endgenerate
  wire pop = have && feed && last_half;
  wire [QB-1:0] t_next = rst ? {QB{1'b0}} : t_rd + {{(QB-1){1'b0}}, pop};
  wire landing = a2_v && l_wr == t_next;
  wire [26:0] tr_n = (rq_take && t_wr == t_next) ? t_new : t_row[t_next];
  // The head's line: the queue's word at the next head, registered as block RAM reads it, or the line landing there
  // that clock, registered beside it.
  reg [511:0] lo_m, hi_m, lo_b, hi_b;
  reg land_d;
  always @(posedge clk) begin
    tr <= tr_n[7:0];
    tb <= tr_n[8];
    ts <= tr_n[16:9];
    tn <= tr_n[17];
    tsh <= tr_n[18];
    tj <= tr_n[26:19];
    lo_m <= lo_q[t_next];
    hi_m <= hi_q[t_next];
    lo_b <= a2_lo;
    hi_b <= a2_hi;
    land_d <= landing;
  end
  assign lo = land_d ? lo_b : lo_m;
  assign hi = land_d ? hi_b : hi_m;
  // The flushed rows packed into lines and queued for memory, the flushed head's rows in order, each line with its row's
  // parity (w_np: the lines queued of each).
  `LOGOS_RAMSTYLE_ASYNC(16) reg [511:0] wfifo_d [0:15];
  `LOGOS_RAMSTYLE_ASYNC(16) reg [31:0] wfifo_l [0:15];
  `LOGOS_RAMSTYLE_ASYNC(16) reg wfifo_p [0:15];
  reg [3:0] w_wr, w_rd;
  reg [4:0] w_np0, w_np1;
  // The replayed batches, in the order replayed (xq: each one's region, share's first row, flush bank, first head, heads
  // and row parity), and of each row parity those whose lines are not all queued (xr0, xr1); the flush writer's batch:
  // the head and row its next line is of.
  `LOGOS_RAMSTYLE(4) reg [31:0] xq_region [0:3];
  `LOGOS_RAMSTYLE_ASYNC(4) reg [15:0] xq_from [0:3];
  `LOGOS_RAMSTYLE(4) reg [1:0] xq_bank [0:3];
  `LOGOS_RAMSTYLE_ASYNC(4) reg [31:0] xq_h0 [0:3];
  `LOGOS_RAMSTYLE(4) reg [7:0] xq_slots [0:3];
  `LOGOS_RAMSTYLE(4) reg xq_par [0:3];
  reg [1:0] xq_wr, xq_rd;
  reg [2:0] xq_n;
  reg [3:0] xr0, xr1;
  reg x_act, x_par;
  reg [31:0] x_region;
  reg [15:0] x_from;
  reg [1:0] x_bank;
  reg [7:0] x_slots;
  reg [31:0] xh;
  reg [15:0] x_row, x_line;
  reg [7:0] x_hb;
  wire [47:0] x_base = {16'd0, x_region} * 48'd64;
  wire [47:0] x_byte = x_base + mem_off + (xh * HD + x_row) * HD * eb + 64 * x_line;
  assign wq_valid = w_n != 5'd0;
  assign wq_line = wfifo_l[w_rd];
  assign wq_data = wfifo_d[w_rd];
  wire w_pop = wq_valid && wq_ready;
  wire [4:0] w_next = w_n + (x_line_push ? 5'd1 : 5'd0) - (w_pop ? 5'd1 : 5'd0);
  // The read-outs, head by head behind the rows, row after row: the head and row the next is of, and its row's read-outs'
  // beats (o_set once the row's are taken up).
  reg [511:0] o_beat;
  reg [31:0] oh;
  reg [15:0] o_row;
  reg o_set;
  reg [SA-1:0] o_out;
  wire [31:0] o_wa = o_out + ((oh * HD + o_row) >> 4);
  // Row dn_n done once it is streamed and read out, none of its batches waits to replay, and none of its lines is queued.
  wire dn_past = o_n != dn_n && st_n != dn_n && (dn_n[0] ? xr1 == 4'd0 && w_np1 == 5'd0 : xr0 == 4'd0 && w_np0 == 5'd0);
  assign busy = in_n != dn_n || w_n != 5'd0;
  always @(posedge clk) begin
    f_log_wr <= 1'b0; f_q_wr <= 1'b0; f_start <= 1'b0; st_we <= 1'b0; q_pending <= 1'b0; fw_wr <= 1'b0; fm_start <= 1'b0;
    x_tog <= !x_tog;
    x_hist <= {x_hist[XD-1:0], x_ask};
    x_ask <= !rst && (!x_two || !x_tog) && (x_two ? x_free2 >= (x_ask ? 7'sd4 : 7'sd2) : x_free1 >= (x_ask ? 7'sd2 : 7'sd1));
    x_free1 <= 7'sd16 - $signed({2'b0, w_next}) - $signed({4'b0, x_held});
    x_free2 <= 7'sd16 - $signed({2'b0, w_next}) - $signed({3'b0, x_held, 1'b0});
    if (rq_take) t_wr <= t_wr + 1'b1;
    if (rs_valid) l_in <= l_in + 1'b1;
    if (a2_v) l_wr <= l_wr + 1'b1;
    owed <= owed + (rq_take ? 10'd1 : 10'd0) - (pop ? 10'd1 : 10'd0);
    owed_b0 <= owed_b0 + ((rq_take && !sb) ? 10'd1 : 10'd0) - ((pop && !tb) ? 10'd1 : 10'd0);
    owed_b1 <= owed_b1 + ((rq_take && sb) ? 10'd1 : 10'd0) - ((pop && tb) ? 10'd1 : 10'd0);
    if (start) in_n <= in_n + 8'd1;
    done <= !rst && dn_past;
    if (dn_past) dn_n <= dn_n + 8'd1;
    // The fold's run: started by the loader, finished once it falls idle after its start.
    if (f_start) f_run <= 1'b1;
    else if (f_run && !f_busy) begin f_run <= 1'b0; folded <= folded + 8'd1; end
    // ── the loader ──
    if (q_lo) begin f_q_wr <= 1'b1; f_q_slot <= q_lo_slot; f_q_beat <= q_lo_at; f_q_data <= st_rd; end
    if (w2_v) case (w2_k)
      K_DEC: ldec_h <= w_elem;
      K_BETA: beta_h <= w_elem;
      K_KEY: key_h <= w_beat;
      default: begin
        f_log_wr <= 1'b1; f_log_at <= w2_e; f_log_beat <= w2_j; f_log_k <= key_h; f_log_v <= w_beat;
        f_log_decay <= ldec_h; f_log_beta <= beta_h;
        if (!w2_j[0]) begin lo_k <= key_h; lo_v <= w_beat; end
        else begin fw_wr <= W > 1; fw_beat <= w2_j >> 1; fw_k <= {key_h, lo_k}; fw_v <= {w_beat, lo_v}; end
      end
    endcase
    case (ls)
      // The loader's row taken up once taken, its heads loaded a batch at a time, then the next row.
      L_IDLE: if (l_act && lh < nv && can_load) begin
          lslot <= 8'd0; qj <= 16'd0; ls <= L_Q;
          if (l_flush) fb_busy[lbank] <= 1'b1;
        end else if (l_act && lh >= nv) begin ld_n <= ld_n + 8'd1; l_act <= 1'b0; end
        else if (!l_act && ld_n != in_n) begin
          l_count <= e_count[ld_n[0]]; l_flush <= e_flush[ld_n[0]]; l_par <= e_par[ld_n[0]]; l_q <= e_q[ld_n[0]];
          lh <= first; lg <= first_g; l_act <= 1'b1;
        end
      L_Q: if (qj == BEATS) begin ge <= 8'd0; gi <= 16'd0; e_at <= l_par ? COPY_LINES * 32'd64 : 32'd0; ls <= L_LOG; end
        else if (st_gnt) begin q_pending <= 1'b1; q_at <= qj; qj <= qj + 16'd1; end
      L_LOG: if (walking) begin
          if (gi == 16'd1 + 2 * BEATS) begin gi <= 16'd0; ge <= ge + 8'd1; e_at <= e_at + entry; end else gi <= gi + 16'd1;
        end else ls <= L_TAIL;
      // The walk's last lines through the copy and the widen, then the batch's next head or its fold.
      L_TAIL: if (!w1_v && !w2_v && !f_log_wr && (SRL == 1 || (!q_pending && !q_flight))) begin
        lh <= lh + STEP;
        lg <= (lg + g_step >= nk) ? lg + g_step - nk : lg + g_step;
        if (lslot + 8'd1 == bs || lh + STEP >= nv) ls <= L_FOLD;
        else begin lslot <= lslot + 8'd1; qj <= 16'd0; ls <= L_Q; end
      end
      L_FOLD: begin
        f_start <= 1'b1; fold_bank <= loaded[0]; loaded <= loaded + 8'd1; lbank <= (lbank == 2'd2) ? 2'd0 : lbank + 2'd1;
        f_entries <= l_count; f_older <= l_flush ? WINDOW8 : 8'd0;
        ls <= L_IDLE;
      end
      default: ;
    endcase
    // ── the streamer ──
    if (feed) begin
      if (W > 1) begin
        t_rd <= t_rd + 1'b1;
        if (!f16) begin half <= !half; if (held_line) row_lo <= lo; end
      end else if (last_half) begin half <= 1'b0; t_rd <= t_rd + 1'b1; end else half <= 1'b1;
    end
    // The flush writer: a replayed batch's lines queued head by head through its share's rows; its bank free and the
    // next batch taken up with its last line, or once one is queued.
    if (x_line_push) begin
      wfifo_d[w_wr] <= x_line_data;
      wfifo_l[w_wr] <= x_byte[37:6];
      wfifo_p[w_wr] <= x_par;
      w_wr <= w_wr + 4'd1;
      if (x_line + 16'd1 == row_lines) begin
        x_line <= 16'd0;
        if (x_row + 16'd1 == x_from + SHARE) begin
          x_row <= x_from; xh <= xh + STEP;
          if (x_hb + 8'd1 == x_slots) begin
            x_hb <= 8'd0; x_act <= 1'b0; fb_busy[x_bank] <= 1'b0;
            if (x_par) xr1 <= xr1 - 4'd1; else xr0 <= xr0 - 4'd1;
          end else x_hb <= x_hb + 8'd1;
        end else x_row <= x_row + 16'd1;
      end else x_line <= x_line + 16'd1;
    end
    if (xq_n != 3'd0 && (!x_act || (x_line_push && x_line + 16'd1 == row_lines && x_row + 16'd1 == x_from + SHARE && x_hb + 8'd1 == x_slots))) begin
      x_region <= xq_region[xq_rd]; x_from <= xq_from[xq_rd]; x_row <= xq_from[xq_rd]; x_bank <= xq_bank[xq_rd];
      xh <= xq_h0[xq_rd]; x_slots <= xq_slots[xq_rd]; x_par <= xq_par[xq_rd]; x_hb <= 8'd0; x_line <= 16'd0; x_act <= 1'b1;
      xq_rd <= xq_rd + 2'd1;
    end
    if (W == 1 && x_ov) begin
      if (f16 && !x_hi) begin x_low <= x_halves[255:0]; x_hi <= 1'b1; end else x_hi <= 1'b0;
    end
    x_pend <= W > 1 && x_ov && !f16;
    if (x_ov) x_out_hi <= x_out[512*W-1:512*W-512];
    if (w_pop) w_rd <= w_rd + 4'd1;
    w_n <= w_next;
    w_np0 <= w_np0 + ((x_line_push && !x_par) ? 5'd1 : 5'd0) - ((w_pop && !wfifo_p[w_rd]) ? 5'd1 : 5'd0);
    w_np1 <= w_np1 + ((x_line_push && x_par) ? 5'd1 : 5'd0) - ((w_pop && wfifo_p[w_rd]) ? 5'd1 : 5'd0);
    // The read-outs into the store; past a row's last head, the next row's read-out beats taken up at once when the row
    // is already taken, or once it is.
    if (r_ov) begin
      o_beat[32 * o_row[3:0] +: 32] <= r_out;
      if (o_row[3:0] == 4'd15) begin st_we <= 1'b1; st_wa <= o_wa[SA-1:0]; st_wd <= {r_out, o_beat[479:0]}; end
      if (o_row + 16'd1 == HD_ROWS) begin
        o_row <= 16'd0;
        if (oh + STEP >= nv) begin
          o_n <= o_n + 8'd1; oh <= first; o_out <= e_out[~o_n[0]]; o_set <= o_n + 8'd1 != in_n;
        end else oh <= oh + STEP;
      end else o_row <= o_row + 16'd1;
    end else if (!o_set && o_n != in_n) begin o_out <= e_out[o_n[0]]; oh <= first; o_set <= 1'b1; end
    else if (o_set && oh >= nv) begin o_n <= o_n + 8'd1; o_set <= 1'b0; end
    case (ss)
      // A folded batch's next head, once every line of the head two back, the last to take its buffer, is through the
      // units.
      // The streamer's row taken up once taken, its batches streamed, then the next row.
      S_IDLE: if (s_act && sh < nv && folded != copied && (sb ? owed_b1 : owed_b0) == 10'd0) begin
          cb <= 16'd0; ss <= S_COPY;
          if (sslot == 8'd0) s_h0 <= sh;
        end else if (s_act && sh >= nv) begin st_n <= st_n + 8'd1; s_act <= 1'b0; end
        else if (!s_act && st_n != in_n) begin
          s_region <= e_region[st_n[0]]; s_flush <= e_flush[st_n[0]]; s_from <= e_from[st_n[0]]; s_par <= st_n[0];
          sh <= first; s_act <= 1'b1;
        end
      S_COPY: if (cb == BEATS + 16'd2) begin cb <= 16'd0; row_req <= 16'd0; ri <= 16'd0; ss <= S_ROWS; end else cb <= cb + 16'd1;
      S_ROWS: begin
        if (rq_take) begin
          if (ri + 16'd1 == row_lines) begin ri <= 16'd0; row_req <= row_req + 16'd1; end else ri <= ri + 16'd1;
        end
        if (row_req == HD_ROWS) ss <= S_NEXT;
      end
      // The batch's next head, its rows asked while the last head's are still on their way, each line carrying its head's
      // slot; a token that replays waits at the batch's end until every line is through the read-out, so each share row
      // went into its head's slot, then replays the share.
      S_NEXT: if (!(s_flush && batch_end) || owed == 10'd0) begin
        sh <= sh + STEP;
        sb <= !sb;
        if (batch_end) begin
          if (s_flush) ss <= S_FLUSH;
          else begin sslot <= 8'd0; copied <= copied + 8'd1; cbank <= (cbank == 2'd2) ? 2'd0 : cbank + 2'd1; ss <= S_IDLE; end
        end else begin sslot <= sslot + 8'd1; ss <= S_IDLE; end
      end
      // The batch's bank replayed once its last share row is written; the batch queued for the flush writer.
      S_FLUSH: if (!fm_wr) begin
        fm_start <= 1'b1; fm_sbank <= cbank; fm_slots <= sslot + 8'd1; fm_first <= s_from; sslot <= 8'd0;
        copied <= copied + 8'd1; cbank <= (cbank == 2'd2) ? 2'd0 : cbank + 2'd1; ss <= S_IDLE;
        xq_region[xq_wr] <= s_region; xq_from[xq_wr] <= s_from; xq_bank[xq_wr] <= cbank; xq_h0[xq_wr] <= s_h0;
        xq_slots[xq_wr] <= sslot + 8'd1; xq_par[xq_wr] <= s_par; xq_wr <= xq_wr + 2'd1;
        if (s_par) xr1 <= xr1 + 4'd1; else xr0 <= xr0 + 4'd1;
      end
      default: ;
    endcase
    xq_n <= xq_n + (ss == S_FLUSH && !fm_wr ? 3'd1 : 3'd0)
      - ((xq_n != 3'd0 && (!x_act || (x_line_push && x_line + 16'd1 == row_lines && x_row + 16'd1 == x_from + SHARE && x_hb + 8'd1 == x_slots))) ? 3'd1 : 3'd0);
    if (rst) begin
      in_n <= 8'd0; ld_n <= 8'd0; st_n <= 8'd0; o_n <= 8'd0; dn_n <= 8'd0; l_act <= 1'b0; s_act <= 1'b0; o_set <= 1'b0;
      x_act <= 1'b0; xq_wr <= 2'd0; xq_rd <= 2'd0; xq_n <= 3'd0; xr0 <= 4'd0; xr1 <= 4'd0; fb_busy <= 3'd0; w_np0 <= 5'd0;
      w_np1 <= 5'd0; loaded <= 8'd0; folded <= 8'd0; copied <= 8'd0; lbank <= 2'd0; cbank <= 2'd0; sslot <= 8'd0; sb <= 1'b0;
      o_row <= 16'd0; x_hb <= 8'd0; x_line <= 16'd0; x_hi <= 1'b0; done <= 1'b0;
      ls <= L_IDLE; ss <= S_IDLE; f_run <= 1'b0; t_wr <= {QB{1'b0}}; t_rd <= {QB{1'b0}}; l_in <= {QB{1'b0}};
      l_wr <= {QB{1'b0}}; w_wr <= 4'd0; w_rd <= 4'd0; w_n <= 5'd0; half <= 1'b0; owed <= 10'd0; owed_b0 <= 10'd0;
      owed_b1 <= 10'd0; x_pend <= 1'b0;
      x_hist <= {(XD+1){1'b0}}; x_tog <= 1'b0; x_free1 <= 7'sd16; x_free2 <= 7'sd16;
    end
  end
endmodule
"#;

const GDN_ROWS_VERILOG: &str = r#"
// A recurrence job's token rows given on in order as descriptors (row_valid, taken when row_ready): the job (start) names
// its rows (`rows`) and the slots of its vectors (the query's, the read-outs', the key heads', the value heads' — the
// values from element v_at of theirs — the decay exponents' and the gates'); row k's region is the rows table's entry k
// and each vector's beat its slot's first plus k times the slot's beats a row, as
// the core's element_beat lays them. The tables are copies the host's writes keep: the rows table (table 1, an entry's
// region its low word) and the slots table (table 2, a slot's first beat and its beats a row). Each row's fields are
// registers, the next row's laid the clock its row is taken.
module logos_gdn_rows #(parameter SA = 10, ROW_MAX = 1)
  (input clk, input rst, input start, input [15:0] rows,
   input [4:0] q_slot, input [4:0] out_slot, input [4:0] k_slot, input [4:0] v_slot, input [19:0] v_at, input [4:0] ld_slot,
   input [4:0] gt_slot,
   input host_we, input [2:0] host_table, input [31:0] host_addr, input [511:0] host_wdata,
   output reg row_valid, input row_ready, output reg [31:0] region, output reg [SA-1:0] q_beat, output reg [SA-1:0] out_beat,
   output reg [SA-1:0] k_beat, output reg [SA-1:0] v_beat, output reg [SA-1:0] ld_beat, output reg [SA-1:0] gt_beat);
  localparam RB = ROW_MAX > 1 ? $clog2(ROW_MAX) : 1;
  `LOGOS_RAMSTYLE_ASYNC(ROW_MAX) reg [31:0] rowt [0:ROW_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(32) reg [63:0] slott [0:31];
  always @(posedge clk)
    if (host_we) case (host_table)
      3'd1: rowt[host_addr[RB-1:0]] <= host_wdata[31:0];
      3'd2: slott[host_addr[4:0]] <= host_wdata[63:0];
      default: ;
    endcase
  // The job's rows given (k of n) and the next row's entry in the rows table (r_at); each vector's beats a row.
  reg [15:0] k, n;
  reg [RB-1:0] r_at;
  reg [SA-1:0] q_st, out_st, k_st, v_st, ld_st, gt_st;
  reg laying;
  wire [63:0] sq = slott[q_slot], so = slott[out_slot], sk = slott[k_slot], sv = slott[v_slot], sl = slott[ld_slot],
              sg = slott[gt_slot];
  always @(posedge clk) begin
    if (laying) begin
      q_beat <= sq[SA-1:0]; out_beat <= so[SA-1:0]; k_beat <= sk[SA-1:0]; v_beat <= sv[SA-1:0] + v_at[SA+3:4];
      ld_beat <= sl[SA-1:0]; gt_beat <= sg[SA-1:0];
      q_st <= sq[32+SA-1:32]; out_st <= so[32+SA-1:32]; k_st <= sk[32+SA-1:32]; v_st <= sv[32+SA-1:32];
      ld_st <= sl[32+SA-1:32]; gt_st <= sg[32+SA-1:32];
      region <= rowt[r_at]; r_at <= r_at + 1'b1; row_valid <= 1'b1; laying <= 1'b0;
    end else if (row_valid && row_ready) begin
      if (k + 16'd1 == n) row_valid <= 1'b0;
      k <= k + 16'd1;
      region <= rowt[r_at]; r_at <= r_at + 1'b1;
      q_beat <= q_beat + q_st; out_beat <= out_beat + out_st; k_beat <= k_beat + k_st; v_beat <= v_beat + v_st;
      ld_beat <= ld_beat + ld_st; gt_beat <= gt_beat + gt_st;
    end
    if (start) begin
      k <= 16'd0; n <= rows; row_valid <= 1'b0; laying <= rows != 16'd0;
      r_at <= {RB{1'b0}};
    end
    if (rst) begin row_valid <= 1'b0; laying <= 1'b0; end
  end
endmodule
"#;

const GDN_LANES_VERILOG: &str = r#"
// A recurrence job's token rows, each a lazily flushed Gated-DeltaNet block, on L head lanes. The job (start) runs `rows`
// rows, each given as a descriptor while row_valid and taken when row_ready: its region and the store beats of its
// query, read-outs, key heads, value heads, decay exponents and gates. The coordinator (logos_gdn_side in MODE 1) takes
// a row once the copy of the log its parity names is free — every lane done with the row two before it, which wrote that
// copy — and a row of the region of the row before it (two positions of one sequence) only once that row is done and
// every write written. It reads the log's count and appends the row's entry on the memory port (rq_*, wq_*, written) as
// the side does, reads the log's entries once into that copy of every lane's (LOG_LINES lines each, addressed by LBA
// bits), hands the row to each lane as the lane has room, writes the count back, and takes the next row while the lanes
// run this one. Lane k (logos_gdn_lane) takes value heads k, k + L, … of each row, its log from its copy and its rows
// from its own memory port (l_*, lane k's at k), its queries from the store on the clocks the store's read is its — the
// coordinator's appending reads first, then the lanes asking in turn, each keeping the read for a query's beats — and
// its read-outs into the store through a queue of its own, the queues written a beat a clock in turn. A lane's memory
// port is logical as the side's: lines back in the order asked, no wait taken on them. Value heads one granule each, on
// four channels, with L = 4 put all of lane k's heads on one channel. Busy from the job's start until every lane has done
// every row, every read-out is in the store, the coordinator is idle and every write is written. A job whose log, at
// `tokens` entries, would take more lines than a copy holds takes its rows and runs none: `fault` from its start until
// the next.
module logos_gdn_lanes #(parameter HD = 128, TMAX = 16, SA = 10, parameter [8:0] OUTSTANDING = 32, parameter W = 1,
                         parameter L = 4, LBA = 13, LOG_LINES = 8192, FLUSHES = 1, SRL = 1)
  (input clk, input rst,
   input start, output busy, input [15:0] rows, input row_valid, output row_ready,
   input [31:0] region, input [31:0] mem_off, input [31:0] log_off, input [31:0] entry, input [31:0] nk, input [31:0] nv,
   input [31:0] fval, input [31:0] fdec, input [31:0] fgate, input [31:0] scale, input [7:0] tokens, input f16,
   input [SA-1:0] q_beat, input [SA-1:0] out_beat, input [SA-1:0] k_beat, input [SA-1:0] v_beat, input [SA-1:0] ld_beat,
   input [SA-1:0] gt_beat,
   output [SA-1:0] st_ra, input [511:0] st_rd, output reg st_we, output reg [SA-1:0] st_wa, output reg [511:0] st_wd,
   output rq_valid, input rq_ready, output [31:0] rq_line, input rs_valid, input [511:0] rs_data,
   output wq_valid, input wq_ready, output [31:0] wq_line, output [511:0] wq_data, input written, output reg fault,
   output [L-1:0] l_rq_valid, input [L-1:0] l_rq_ready, output [32*L-1:0] l_rq_line, input [L-1:0] l_rs_valid,
   input [512*L-1:0] l_rs_data, output [L-1:0] l_wq_valid, input [L-1:0] l_wq_ready, output [32*L-1:0] l_wq_line,
   output [512*L-1:0] l_wq_data, input [L-1:0] l_written);
  localparam LB = L > 1 ? $clog2(L) : 1;
  localparam [LB-1:0] LAST = L - 1;
  localparam [39:0] HOLDS = LOG_LINES;
  localparam [LBA:0] SECOND = LOG_LINES;
  wire co_ready, lb_we, co_flush, co_busy, co_idle, co_reading;
  wire [LBA-1:0] lb_wa;
  wire [511:0] lb_wd;
  wire [7:0] co_count;
  wire [31:0] log_first, log_lines;
  wire [SA-1:0] co_ra;
  // What the coordinator does not drive: store writes.
  wire co_we;
  wire [SA-1:0] co_wa;
  wire [511:0] co_wd;
  wire [39:0] log_need = {32'd0, tokens} * {14'd0, entry[31:6]};
  wire too_long = log_need > HOLDS;
  // The job: its rows, those the coordinator has taken, and those every lane has done (lane k's count in l_fin).
  wire [L-1:0] l_busy, l_room, l_done, want, s_have;
  reg job;
  reg [15:0] n_rows, taken, done_all;
  reg [16*L-1:0] l_fin;
  integer di;
  always @* begin
    done_all = l_fin[15:0];
    for (di = 1; di < L; di = di + 1) if (l_fin[16*di +: 16] < done_all) done_all = l_fin[16*di +: 16];
  end
  // The row the coordinator runs: the copy it writes (its parity) and the fields each lane takes with it; and the region
  // of the row before.
  reg c_par;
  reg [31:0] c_region, last_region;
  reg [SA-1:0] c_q, c_out;
  wire copy_free = taken < 16'd2 || done_all + 16'd1 >= taken;
  wire same = taken != 16'd0 && region == last_region;
  wire quiet = l_busy == {L{1'b0}} && s_have == {L{1'b0}} && &l_written && written && !co_busy;
  wire co_go = job && !fault && row_valid && taken != n_rows && co_idle && copy_free && (!same || (done_all == taken && quiet));
  wire skip = job && fault && row_valid && taken != n_rows;
  assign row_ready = co_go || skip;
  // The row handed to each lane as the lane has room, once the log's last line is in every copy (co_ready a clock
  // old); taken once every lane has it.
  reg handing;
  reg [L-1:0] took;
  wire [L-1:0] l_go = {L{co_ready && handing}} & ~took & l_room;
  wire co_taken = co_ready && handing && &(took | l_go);
  assign busy = job;
  integer fi;
  always @(posedge clk) begin
    handing <= !rst && co_ready && !co_taken;
    took <= co_taken ? {L{1'b0}} : took | l_go;
    for (fi = 0; fi < L; fi = fi + 1) if (l_done[fi]) l_fin[16*fi +: 16] <= l_fin[16*fi +: 16] + 16'd1;
    if (co_go) begin c_par <= taken[0]; c_region <= region; c_q <= q_beat; c_out <= out_beat; last_region <= region; end
    if (row_ready) taken <= taken + 16'd1;
    if (job && (fault ? taken == n_rows : done_all == n_rows) && quiet) job <= 1'b0;
    if (start) begin job <= 1'b1; n_rows <= rows; taken <= 16'd0; fault <= too_long; l_fin <= {16*L{1'b0}}; end
    if (rst) begin job <= 1'b0; fault <= 1'b0; took <= {L{1'b0}}; taken <= 16'd0; n_rows <= 16'd0; l_fin <= {16*L{1'b0}}; end
  end
  logos_gdn_side #(.SRL(SRL), .HD(HD), .TMAX(TMAX), .SA(SA), .OUTSTANDING(OUTSTANDING), .W(W), .MODE(1), .LBA(LBA)) co(.clk(clk),
    .rst(rst), .start(co_go), .busy(co_busy), .region(region), .mem_off(mem_off), .log_off(log_off), .entry(entry), .nk(nk),
    .nv(nv), .fval(fval), .fdec(fdec), .fgate(fgate), .scale(scale), .tokens(tokens), .f16(f16), .q_beat(q_beat),
    .out_beat(out_beat), .k_beat(k_beat), .v_beat(v_beat), .ld_beat(ld_beat), .gt_beat(gt_beat), .st_ra(co_ra),
    .st_rd(st_rd), .st_we(co_we), .st_wa(co_wa), .st_wd(co_wd), .rq_valid(rq_valid), .rq_ready(rq_ready), .rq_line(rq_line),
    .rs_valid(rs_valid), .rs_data(rs_data), .wq_valid(wq_valid), .wq_ready(wq_ready), .wq_line(wq_line), .wq_data(wq_data),
    .written(written), .co_ready(co_ready),
    .co_taken(co_taken), .lb_we(lb_we), .lb_wa(lb_wa), .lb_wd(lb_wd), .co_count(co_count), .co_flush(co_flush),
    .log_first(log_first), .log_lines(log_lines), .co_reading(co_reading), .co_idle(co_idle));
  // The coordinator's copy writes, a clock on, into the copy its row's parity names (the second LOG_LINES lines in).
  reg cw_we;
  reg [LBA:0] cw_wa;
  reg [511:0] cw_wd;
  always @(posedge clk) begin
    cw_we <= !rst && lb_we;
    cw_wa <= c_par ? {1'b0, lb_wa} + SECOND : {1'b0, lb_wa};
    cw_wd <= lb_wd;
  end
  // The store's read: the coordinator's while it appends; otherwise a lane's while it holds it (held, holder — taken by
  // the first lane asking the clock after the last holder stops), a lane's read granted on the clocks it holds it and
  // asks and the coordinator does not read.
  wire [SA*L-1:0] l_ra;
  reg [LB-1:0] holder, pick;
  reg held, any;
  integer pi;
  always @* begin
    any = 1'b0;
    pick = {LB{1'b0}};
    for (pi = 0; pi < L; pi = pi + 1) if (!any && want[pi]) begin any = 1'b1; pick = pi[LB-1:0]; end
  end
  wire keeps = held && want[holder];
  assign st_ra = co_reading ? co_ra : held ? l_ra[SA*holder +: SA] : co_ra;
  always @(posedge clk) begin
    if (!keeps) begin held <= !rst && any; holder <= pick; end
    if (rst) held <= 1'b0;
  end
  // Each lane's read-outs queued a beat at a time and written into the store on its own clocks, lane k's when the slot
  // counter is k: a lane makes a beat at most every sixteen clocks (a beat is sixteen rows' read-outs, a read-out a
  // clock) and has a slot every L ≤ 4, so no queue holds more than one.
  wire [(SA+512)*L-1:0] s_head;
  reg [LB-1:0] s_turn;
  always @(posedge clk) begin
    st_we <= !rst && s_have[s_turn];
    {st_wa, st_wd} <= s_head[(SA+512)*s_turn +: SA+512];
    s_turn <= (s_turn == LAST) ? {LB{1'b0}} : s_turn + 1'b1;
    if (rst) s_turn <= {LB{1'b0}};
  end
  genvar k;
  generate
    for (k = 0; k < L; k = k + 1) begin : lane
      localparam [31:0] FIRST = k;
      localparam [LB-1:0] LANE = k;
      wire we;
      wire [SA-1:0] wa;
      wire [511:0] wd;
      wire [LBA:0] b_ra;
      wire [511:0] b_rd;
      logos_gdn_lane #(.SRL(SRL), .HD(HD), .TMAX(TMAX), .SA(SA), .OUTSTANDING(OUTSTANDING), .W(W), .STEP(L), .LBA(LBA), .FLUSHES(FLUSHES), .COPY_LINES(LOG_LINES))
        lane(.clk(clk),
        .rst(rst), .start(l_go[k]), .busy(l_busy[k]), .room(l_room[k]), .done(l_done[k]), .in_region(c_region), .mem_off(mem_off),
        .entry(entry), .nk(nk), .nv(nv), .fval(fval), .fdec(fdec), .fgate(fgate), .scale(scale), .f16(f16), .in_q_beat(c_q),
        .in_out_beat(c_out), .first(FIRST), .in_count(co_count), .in_flush(co_flush), .in_par(c_par), .st_rq(want[k]),
        .st_gnt(keeps && holder == LANE && !co_reading),
        .st_ra(l_ra[SA*k +: SA]), .st_rd(st_rd), .st_we(we), .st_wa(wa), .st_wd(wd), .lb_ra(b_ra), .lb_rd(b_rd),
        .rq_valid(l_rq_valid[k]), .rq_ready(l_rq_ready[k]), .rq_line(l_rq_line[32*k +: 32]), .rs_valid(l_rs_valid[k]),
        .rs_data(l_rs_data[512*k +: 512]), .wq_valid(l_wq_valid[k]), .wq_ready(l_wq_ready[k]), .wq_line(l_wq_line[32*k +: 32]),
        .wq_data(l_wq_data[512*k +: 512]));
      // The lane's two copies of the log, written as the coordinator reads it, read a clock after their address.
      logos_bram_w512 #(.D(2 * LOG_LINES), .A(LBA + 1)) copy(.clk(clk), .ra(b_ra), .rd(b_rd), .we(cw_we), .wa(cw_wa), .wd(cw_wd),
        .wm({64{1'b1}}));
      `LOGOS_RAMSTYLE_ASYNC(4) reg [SA+511:0] q [0:3];
      reg [1:0] q_wr, q_rd;
      reg [2:0] q_n;
      wire pop = s_turn == LANE && s_have[k];
      assign s_have[k] = q_n != 3'd0;
      assign s_head[(SA+512)*k +: SA+512] = q[q_rd];
      always @(posedge clk) begin
        if (we) begin q[q_wr] <= {wa, wd}; q_wr <= q_wr + 2'd1; end
        if (pop) q_rd <= q_rd + 2'd1;
        q_n <= q_n + (we ? 3'd1 : 3'd0) - (pop ? 3'd1 : 3'd0);
        if (rst) begin q_wr <= 2'd0; q_rd <= 2'd0; q_n <= 3'd0; end
      end
    end
  endgenerate
endmodule
"#;

/// `logos_gdn_fold_m`: the fold of as many heads at once as the add takes clocks, a beat of each in turn.
const GDN_FOLD_M_VERILOG: &str = r#"
// H heads' queries folded back through their logs at once, each as gdn_fold folds it: the heads in lockstep, a beat of
// each in turn, so with H the add's clocks a lane's running dot of a head comes back to its add as the head's next beat
// enters, and one head's dot, tree, gate and update are the other heads' issues rather than gaps. Each head's log (its
// tokens' keys, values, log decays and gates) and its query are written into its slot; a fold of `entries` tokens,
// newest first, folds every slot into one of two banks (`bank`, taken at its start), the first token's update taking
// the query and no collected values; the folded queries and collected values are read back from either bank, by slot
// (a bank not being folded into), two clocks after their address, and the decays of the last fold by slot. Every memory
// is written a word a clock and read into a register, as block RAM reads; a bank's address is a register beside it. TRAILING keeps, beside each bank, the fold
// once it has passed every token after the log's oldest `older` (taken at its start): the fold over the newer tokens
// alone, read back as the bank is (ynew_beat, cnew_beat); without it those are the bank's own.
module logos_gdn_fold_m #(parameter N = 128, T_MAX = 16, TRAILING = 0)
  (input clk, input rst,
   input log_wr, input [7:0] log_slot, input [7:0] log_at, input [15:0] log_beat, input [511:0] log_k, input [511:0] log_v,
   input [31:0] log_decay, input [31:0] log_beta,
   input q_wr, input [7:0] q_slot, input [15:0] q_beat, input [511:0] q_data,
   input start, input bank, input [7:0] entries, input [7:0] older, output busy,
   input rd_bank, input [7:0] rd_slot, input [15:0] rd_beat, output [511:0] y_beat, output [511:0] carried_beat,
   output [511:0] ynew_beat, output [511:0] cnew_beat,
   input [7:0] rd_token, output [31:0] decay_out);
  localparam H = @FADD@;
  localparam [7:0] HS = H;
  localparam [15:0] HW = H;
  localparam [15:0] BEATS = N / 16, ISSUES = BEATS * HW;
  localparam IDLE = 3'd0, EXPS = 3'd1, DOT = 3'd2, BCW = 3'd3, UPD = 3'd4, FILL = 3'd5, BARE = 3'd6;
  `LOGOS_RAMSTYLE(H*T_MAX*BEATS) reg [511:0] kmem [0:H*T_MAX*BEATS-1];
  `LOGOS_RAMSTYLE(H*T_MAX*BEATS) reg [511:0] vmem [0:H*T_MAX*BEATS-1];
  `LOGOS_RAMSTYLE(H*T_MAX) reg [31:0] log_decays [0:H*T_MAX-1];
  `LOGOS_RAMSTYLE(H*T_MAX) reg [31:0] betas [0:H*T_MAX-1];
  `LOGOS_RAMSTYLE(H*BEATS) reg [511:0] qmem [0:H*BEATS-1];
  always @(posedge clk) begin
    if (log_wr) begin
      kmem[(log_slot * T_MAX + log_at) * BEATS + log_beat] <= log_k;
      vmem[(log_slot * T_MAX + log_at) * BEATS + log_beat] <= log_v;
      if (log_beat == 16'd0) begin log_decays[log_slot * T_MAX + log_at] <= log_decay; betas[log_slot * T_MAX + log_at] <= log_beta; end
    end
    if (q_wr) qmem[q_slot * BEATS + q_beat] <= q_data;
  end
  // The two banks of folded queries and collected values.
  `LOGOS_RAMSTYLE(H*BEATS) reg [511:0] y0 [0:H*BEATS-1];
  `LOGOS_RAMSTYLE(H*BEATS) reg [511:0] y1 [0:H*BEATS-1];
  `LOGOS_RAMSTYLE(H*BEATS) reg [511:0] c0 [0:H*BEATS-1];
  `LOGOS_RAMSTYLE(H*BEATS) reg [511:0] c1 [0:H*BEATS-1];
  wire [15:0] rd_at = rd_slot * BEATS + rd_beat;
  `LOGOS_RAMSTYLE_ASYNC(H*T_MAX) reg [31:0] decays [0:H*T_MAX-1];
  assign decay_out = decays[rd_slot * T_MAX + rd_token];
  reg [2:0] state;
  reg fb;
  reg [7:0] e, xs, xt, got, fo;
  reg [15:0] i, written;
  reg [7:0] caught;
  `LOGOS_RAMSTYLE(H) reg [31:0] bc [0:H-1];
  reg bare_p, bare_r;
  assign busy = state != IDLE || bare_p || bare_r;
  // The decays: every slot's tokens' log decays through the exponential, back to back, slot by slot, each read a clock
  // before it enters.
  wire [31:0] ex;
  reg [@EXP@:1] ex_v;
  reg [15:0] ex_at [1:@EXP@];
  reg [31:0] x_r;
  reg x_v;
  reg [15:0] x_at;
  logos_exp_p xp(.clk(clk), .x_in(x_r), .y(ex));
  integer k;
  wire exp_issue = state == EXPS && xs != HS;
  always @(posedge clk) begin
    x_r <= log_decays[xs * T_MAX + xt];
    x_v <= !rst && exp_issue;
    x_at <= xs * T_MAX + xt;
    ex_v <= rst ? {@EXP@{1'b0}} : {ex_v[@EXP@-1:1], x_v};
    ex_at[1] <= x_at;
    for (k = 2; k <= @EXP@; k = k + 1) ex_at[k] <= ex_at[k-1];
    if (ex_v[@EXP@]) decays[ex_at[@EXP@]] <= ex;
  end
  // An issue's slot and beat: beat-major, so a slot's beats are H issues apart. The newest token's issues take the
  // query and no collected values; later ones the bank's.
  wire [15:0] s_w = i % HW;
  wire [7:0] s = s_w[7:0];
  wire [15:0] b = i / HW;
  wire [15:0] at = s * BEATS + b;
  wire fresh = e + 8'd1 == entries;
  // Each bank read at one address, the fold's while it folds into the bank, the read-back's otherwise, the query beside
  // them at the fold's: the address taken into a register copied near the banks it reaches (_p), the word registered the
  // clock after. An issue's marks take the same register stage below, so each of its reads and writes lands a clock
  // later than its choice, every one in the order it was.
  wire [15:0] a0 = (state != IDLE && !fb) ? at : rd_at, a1 = (state != IDLE && fb) ? at : rd_at;
  (* maxfan = 64 *) reg [15:0] a0_p, a1_p, q_ap, rd_at_p;
  reg rb_p;
  reg [511:0] y0_r, y1_r, c0_r, c1_r, q_r;
  reg rb_r;
  always @(posedge clk) begin
    a0_p <= a0;
    a1_p <= a1;
    q_ap <= at;
    rd_at_p <= rd_at;
    rb_p <= rd_bank;
    y0_r <= y0[a0_p];
    y1_r <= y1[a1_p];
    c0_r <= c0[a0_p];
    c1_r <= c1[a1_p];
    q_r <= qmem[q_ap];
    rb_r <= rb_p;
  end
  assign y_beat = rb_r ? y1_r : y0_r;
  assign carried_beat = rb_r ? c1_r : c0_r;
  wire dot_issue = state == DOT;
  wire upd_issue = state == UPD;
  wire bare = state == BARE;
  // An issue the clock its address is registered (_p), the clock its words are read (_r), then its operands.
  reg d_p, u_p, fresh_p, first_p, last_p;
  reg [15:0] at_p, b_p;
  reg [7:0] s_p;
  reg d_r, u_r, fresh_r, first_r, last_r;
  reg [15:0] at_r, b_r;
  reg [7:0] s_r;
  always @(posedge clk) begin
    d_p <= !rst && dot_issue;
    u_p <= !rst && upd_issue;
    bare_p <= !rst && bare;
    fresh_p <= fresh;
    first_p <= b == 16'd0;
    last_p <= b == BEATS - 1;
    at_p <= at;
    b_p <= b;
    s_p <= s;
    d_r <= !rst && d_p;
    u_r <= !rst && u_p;
    bare_r <= !rst && bare_p;
    fresh_r <= fresh_p;
    first_r <= first_p;
    last_r <= last_p;
    at_r <= at_p;
    b_r <= b_p;
    s_r <= s_p;
  end
  reg [511:0] y_q, k_q, v_q, c_q;
  reg d_q, u_q, first_q, last_q;
  reg [15:0] at_q;
  reg [7:0] s_q;
  reg [31:0] bc_q, decay_q;
  always @(posedge clk) begin
    y_q <= fresh_r ? q_r : fb ? y1_r : y0_r;
    k_q <= kmem[(s_r * T_MAX + e) * BEATS + b_r];
    v_q <= vmem[(s_r * T_MAX + e) * BEATS + b_r];
    c_q <= fresh_r ? 512'd0 : fb ? c1_r : c0_r;
    d_q <= !rst && d_r;
    u_q <= !rst && u_r;
    first_q <= first_r;
    last_q <= last_r;
    at_q <= at_r;
    s_q <= s_r;
    bc_q <= bc[s_r];
    decay_q <= decays[s_r * T_MAX + e];
  end
  reg [@BC@:1] d_d, last_d, first_d;
  reg [7:0] s_d [1:@BC@];
  reg [@FOLDED@:1] u_d;
  reg [15:0] at_d [1:@FOLDED@];
  reg [511:0] c_d [1:@FMUL@];
  reg [511:0] y_d [1:@FMUL@];
  // An update's decay beside its y − bc·k, which reaches the last multiply a multiply and an add after the issue.
  localparam DY = @FMUL@ + @FADD@;
  reg [31:0] decay_d [1:DY];
  always @(posedge clk) begin
    decay_d[1] <= decay_q;
    for (k = 2; k <= DY; k = k + 1) decay_d[k] <= decay_d[k-1];
    d_d <= rst ? {@BC@{1'b0}} : {d_d[@BC@-1:1], d_q};
    last_d <= {last_d[@BC@-1:1], last_q};
    first_d <= {first_d[@BC@-1:1], first_q};
    s_d[1] <= s_q;
    for (k = 2; k <= @BC@; k = k + 1) s_d[k] <= s_d[k-1];
    u_d <= rst ? {@FOLDED@{1'b0}} : {u_d[@FOLDED@-1:1], u_q};
    at_d[1] <= at_q;
    for (k = 2; k <= @FOLDED@; k = k + 1) at_d[k] <= at_d[k-1];
    c_d[1] <= c_q;
    y_d[1] <= y_q;
    for (k = 2; k <= @FMUL@; k = k + 1) begin c_d[k] <= c_d[k-1]; y_d[k] <= y_d[k-1]; end
  end
  // The lanes: each slot's key's dot with its folded query, the slot's running sum back at its add as its next term
  // enters; and the update, carried + bc·v and decay·(y − bc·k).
  wire [511:0] sums, cnew, ynew;
  genvar l;
  generate
    for (l = 0; l < 16; l = l + 1) begin : lane
      wire [31:0] p, cv, bk, t;
      logos_fmul_p m(.clk(clk), .a(k_q[32*l +: 32]), .b(y_q[32*l +: 32]), .y(p));
      logos_fadd_p a(.clk(clk), .a(first_d[@PRODUCTS@] ? 32'd0 : sums[32*l +: 32]), .b(p), .y(sums[32*l +: 32]));
      logos_fmul_p mv(.clk(clk), .a(bc_q), .b(v_q[32*l +: 32]), .y(cv));
      logos_fadd_p ac(.clk(clk), .a(c_d[@FMUL@][32*l +: 32]), .b(cv), .y(cnew[32*l +: 32]));
      logos_fmul_p mk(.clk(clk), .a(bc_q), .b(k_q[32*l +: 32]), .y(bk));
      logos_fadd_p sy(.clk(clk), .a(y_d[@FMUL@][32*l +: 32]), .b({~bk[31], bk[30:0]}), .y(t));
      logos_fmul_p md(.clk(clk), .a(decay_d[DY]), .b(t), .y(ynew[32*l +: 32]));
    end
  endgenerate
  // Each slot's dot through the tree and its gate's product, its slot beside it.
  wire [31:0] dot, bcw;
  reg [31:0] beta_r;
  always @(posedge clk) beta_r <= betas[s_d[@TREE@-1] * T_MAX + e];
  logos_tree16_p tr(.clk(clk), .lanes(sums), .sum(dot));
  logos_fmul_p mb(.clk(clk), .a(beta_r), .b(dot), .y(bcw));
  wire bc_in = d_d[@BC@] && last_d[@BC@];
  wire c_back = u_d[@UPDATED@];
  wire y_back = u_d[@FOLDED@];
  // A fold of no tokens: the queries into the bank as they are and no collected values, a beat a clock, each the
  // clock after its query is read.
  always @(posedge clk) begin
    if (bare_r ? !fb : c_back && !fb) c0[bare_r ? at_r : at_d[@UPDATED@]] <= bare_r ? 512'd0 : cnew;
    if (bare_r ? fb : c_back && fb) c1[bare_r ? at_r : at_d[@UPDATED@]] <= bare_r ? 512'd0 : cnew;
    if (bare_r ? !fb : y_back && !fb) y0[bare_r ? at_r : at_d[@FOLDED@]] <= bare_r ? q_r : ynew;
    if (bare_r ? fb : y_back && fb) y1[bare_r ? at_r : at_d[@FOLDED@]] <= bare_r ? q_r : ynew;
    if (bc_in) bc[s_d[@BC@]] <= bcw;
  end
  // The fold over the newer tokens: the bank's writes of the token at `older` (each landing while that token is the
  // fold's, its last before the walk moves on) and of a fold of no tokens, kept beside the bank.
  generate
    if (TRAILING) begin : behind
      `LOGOS_RAMSTYLE(H*BEATS) reg [511:0] ys0 [0:H*BEATS-1];
      `LOGOS_RAMSTYLE(H*BEATS) reg [511:0] ys1 [0:H*BEATS-1];
      `LOGOS_RAMSTYLE(H*BEATS) reg [511:0] cs0 [0:H*BEATS-1];
      `LOGOS_RAMSTYLE(H*BEATS) reg [511:0] cs1 [0:H*BEATS-1];
      reg [511:0] ys0_r, ys1_r, cs0_r, cs1_r;
      wire past = e == fo;
      always @(posedge clk) begin
        if (bare_r ? !fb : c_back && !fb && past) cs0[bare_r ? at_r : at_d[@UPDATED@]] <= bare_r ? 512'd0 : cnew;
        if (bare_r ? fb : c_back && fb && past) cs1[bare_r ? at_r : at_d[@UPDATED@]] <= bare_r ? 512'd0 : cnew;
        if (bare_r ? !fb : y_back && !fb && past) ys0[bare_r ? at_r : at_d[@FOLDED@]] <= bare_r ? q_r : ynew;
        if (bare_r ? fb : y_back && fb && past) ys1[bare_r ? at_r : at_d[@FOLDED@]] <= bare_r ? q_r : ynew;
        ys0_r <= ys0[rd_at_p];
        ys1_r <= ys1[rd_at_p];
        cs0_r <= cs0[rd_at_p];
        cs1_r <= cs1[rd_at_p];
      end
      assign ynew_beat = rb_r ? ys1_r : ys0_r;
      assign cnew_beat = rb_r ? cs1_r : cs0_r;
    end else begin : whole
      assign ynew_beat = y_beat;
      assign cnew_beat = carried_beat;
    end
  endgenerate
  always @(posedge clk) begin
    if (rst) begin state <= IDLE; e <= 8'd0; xs <= 8'd0; xt <= 8'd0; got <= 8'd0; i <= 16'd0; written <= 16'd0; caught <= 8'd0; end
    else begin
      case (state)
        IDLE: if (start) begin fb <= bank; fo <= older; xs <= 8'd0; xt <= 8'd0; got <= 8'd0; i <= 16'd0; state <= EXPS; end
        EXPS: begin
          if (exp_issue) begin
            if (xt + 8'd1 >= entries) begin xt <= 8'd0; xs <= xs + 8'd1; end else xt <= xt + 8'd1;
          end
          if ({8'd0, got} == HW * {8'd0, entries}) begin
            if (entries == 8'd0) state <= BARE; else begin e <= entries - 8'd1; i <= 16'd0; state <= DOT; end
          end
        end
        BARE: if (i + 16'd1 == ISSUES) begin i <= 16'd0; state <= IDLE; end else i <= i + 16'd1;
        DOT: if (i + 16'd1 == ISSUES) begin i <= 16'd0; caught <= 8'd0; state <= BCW; end else i <= i + 16'd1;
        BCW: if (caught == HS) begin written <= 16'd0; state <= UPD; end
        UPD: if (i + 16'd1 == ISSUES) begin i <= 16'd0; state <= FILL; end else i <= i + 16'd1;
        FILL: if (written == ISSUES) begin
          if (e == 8'd0) state <= IDLE; else begin e <= e - 8'd1; state <= DOT; end
        end
        default: state <= IDLE;
      endcase
      if (ex_v[@EXP@]) got <= got + 8'd1;
      if (y_back) written <= written + 16'd1;
      if (bc_in) caught <= caught + 8'd1;
    end
  end
endmodule
"#;

const GDN_FLUSH_M_VERILOG: &str = r#"
// A trailing flush's shares of up to SLOTS heads at once, each replayed as gdn_flush replays its log. Each of three banks
// holds, per slot, a head's log — its tokens' keys and values a beat of 16·W elements at a time, each token's decay
// exponent and gate with its beat 0, the decay through the pipelined exponential as it is written — and its share of
// rows: SHARE rows of N elements in beats of 16·W f32, written by row and beat. `start` replays a bank's first
// `entries` tokens over the shares of its first `slots` slots, slot s's rows the head's rows from `first` (a row's
// value of a token its own element of the token's values), once every decay written to it is in and the bank started
// before it is replayed. The shares are groups of W·@FADD@ rows, slot-major, a group's rows one slot's: for every token
// in order each group has a dot pass (its rows' dots with the token's key, beat-major so a lane's chain sum comes back
// as the row's next beat enters), its rows' coefficients beta·(v − decay·dot), and an update pass decay·m + coef·k, no
// rounding between; the dot passes run back to back on their lanes and the update passes on theirs, a group's next dot
// once its update is written, so the other slots' groups fill each group's wait for its coefficients. A replayed
// bank's rows stream out slot by slot, row-major, each element stored once (rounded to binary16 and back when f16), the
// banks replayed in the order started and streamed in the order replayed, whichever bank each is: a beat leaves the
// clock after each clock out_ready is high. A bank is free from its last beat out until it is started again; the lane
// fills only a free bank, so one bank takes a batch's log while another takes the batch before's rows and the third
// replays the batch before that.
module logos_gdn_flush_m #(parameter N = 128, W = 2, T_MAX = 8, SHARE = 16, SLOTS = 4)
  (input clk, input rst,
   input log_wr, input [1:0] log_bank, input [7:0] log_slot, input [7:0] log_at, input [15:0] log_beat,
   input [512*W-1:0] log_k, input [512*W-1:0] log_v, input [31:0] log_decay, input [31:0] log_beta,
   input row_wr, input [1:0] row_bank, input [7:0] row_slot, input [15:0] row_j, input [15:0] row_beat, input [512*W-1:0] row_data,
   input start, input [1:0] bank, input [7:0] entries, input [7:0] slots, input [15:0] first, input f16,
   output [2:0] free,
   output out_valid, input out_ready, output [512*W-1:0] out_beat, output [256*W-1:0] out_half);
  localparam [15:0] BEATS = N / (16 * W);
  localparam [15:0] G = W * @FADD@;
  localparam [15:0] GS = SHARE / G;
  localparam [15:0] PASS = G * BEATS;
  localparam [15:0] SB = SHARE * BEATS;
  localparam RW = SLOTS * SHARE * BEATS;
  localparam LW = SLOTS * T_MAX * BEATS;
  localparam CHAIN = W * @FADD@, TREE = @FMUL@ + CHAIN + 4 * @FADD@, VALUE = TREE + @FMUL@;
  localparam GATED = VALUE + @FADD@, COEF = GATED + @FMUL@, UPDATED = @FMUL@ + @FADD@;
  localparam EW = $clog2(16 * W);
  localparam [15:0] SHARE_W = SHARE, T_MAX_W = T_MAX;
  // The coefficients' ring: four slots, an item's at its index's two low bits.
  localparam [15:0] RING = 16'd4;
  // Each bank's logs: every slot's tokens' keys, twice (one copy for each pass), and values; their decays and gates.
  reg [512*W-1:0] ka0 [0:LW-1];
  reg [512*W-1:0] ka1 [0:LW-1];
  reg [512*W-1:0] ka2 [0:LW-1];
  reg [512*W-1:0] kb0 [0:LW-1];
  reg [512*W-1:0] kb1 [0:LW-1];
  reg [512*W-1:0] kb2 [0:LW-1];
  reg [512*W-1:0] va0 [0:LW-1];
  reg [512*W-1:0] va1 [0:LW-1];
  reg [512*W-1:0] va2 [0:LW-1];
  `LOGOS_RAMSTYLE_ASYNC(SLOTS*T_MAX) reg [31:0] dc0 [0:SLOTS*T_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(SLOTS*T_MAX) reg [31:0] dc1 [0:SLOTS*T_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(SLOTS*T_MAX) reg [31:0] dc2 [0:SLOTS*T_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(SLOTS*T_MAX) reg [31:0] bt0 [0:SLOTS*T_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(SLOTS*T_MAX) reg [31:0] bt1 [0:SLOTS*T_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(SLOTS*T_MAX) reg [31:0] bt2 [0:SLOTS*T_MAX-1];
  // Each bank's shares, slot-major and row-major, twice: the dot pass reads one copy and the update pass the other.
  reg [512*W-1:0] ga0 [0:RW-1];
  reg [512*W-1:0] ga1 [0:RW-1];
  reg [512*W-1:0] ga2 [0:RW-1];
  reg [512*W-1:0] gb0 [0:RW-1];
  reg [512*W-1:0] gb1 [0:RW-1];
  reg [512*W-1:0] gb2 [0:RW-1];
  // A log write's places as the logs' addresses hold them: a place past the last slot wraps as it does in block RAM.
  localparam LA = LW > 1 ? $clog2(LW) : 1, TA = SLOTS * T_MAX > 1 ? $clog2(SLOTS * T_MAX) : 1;
  wire [15:0] la_at = ({8'd0, log_slot} * T_MAX_W + {8'd0, log_at}) * BEATS + log_beat;
  wire [15:0] lt_at = {8'd0, log_slot} * T_MAX_W + {8'd0, log_at};
  wire [LA-1:0] la = la_at[LA-1:0];
  wire [TA-1:0] lt = lt_at[TA-1:0];
  // The decays: each token's log decay through the exponential as its beat 0 is written, its bank and place beside;
  // the decays of each bank still on their way.
  wire [31:0] ex;
  reg [@EXP@:1] ex_v;
  reg [1:0] ex_b [1:@EXP@];
  reg [TA-1:0] ex_at [1:@EXP@];
  reg [7:0] exps0, exps1, exps2;
  logos_exp_p xp(.clk(clk), .x_in(log_decay), .y(ex));
  integer i;
  wire ex_in = log_wr && log_beat == 16'd0;
  always @(posedge clk) begin
    if (log_wr && log_bank == 2'd0) begin ka0[la] <= log_k; kb0[la] <= log_k; va0[la] <= log_v; end
    if (log_wr && log_bank == 2'd1) begin ka1[la] <= log_k; kb1[la] <= log_k; va1[la] <= log_v; end
    if (log_wr && log_bank == 2'd2) begin ka2[la] <= log_k; kb2[la] <= log_k; va2[la] <= log_v; end
    if (ex_in && log_bank == 2'd0) bt0[lt] <= log_beta;
    if (ex_in && log_bank == 2'd1) bt1[lt] <= log_beta;
    if (ex_in && log_bank == 2'd2) bt2[lt] <= log_beta;
    ex_v <= rst ? {@EXP@{1'b0}} : {ex_v[@EXP@-1:1], ex_in};
    ex_b[1] <= log_bank;
    ex_at[1] <= lt;
    for (i = 2; i <= @EXP@; i = i + 1) begin ex_b[i] <= ex_b[i-1]; ex_at[i] <= ex_at[i-1]; end
    if (ex_v[@EXP@] && ex_b[@EXP@] == 2'd0) dc0[ex_at[@EXP@]] <= ex;
    if (ex_v[@EXP@] && ex_b[@EXP@] == 2'd1) dc1[ex_at[@EXP@]] <= ex;
    if (ex_v[@EXP@] && ex_b[@EXP@] == 2'd2) dc2[ex_at[@EXP@]] <= ex;
    if (rst) begin exps0 <= 8'd0; exps1 <= 8'd0; exps2 <= 8'd0; end
    else begin
      exps0 <= exps0 + ((ex_in && log_bank == 2'd0) ? 8'd1 : 8'd0) - ((ex_v[@EXP@] && ex_b[@EXP@] == 2'd0) ? 8'd1 : 8'd0);
      exps1 <= exps1 + ((ex_in && log_bank == 2'd1) ? 8'd1 : 8'd0) - ((ex_v[@EXP@] && ex_b[@EXP@] == 2'd1) ? 8'd1 : 8'd0);
      exps2 <= exps2 + ((ex_in && log_bank == 2'd2) ? 8'd1 : 8'd0) - ((ex_v[@EXP@] && ex_b[@EXP@] == 2'd2) ? 8'd1 : 8'd0);
    end
  end
  // The banks: started and waiting their turn (pend, in the order started: rq0 first, rqn of them), replayed and
  // waiting to stream out or streaming (outq, in the order replayed: oq0 first, oqn of them); the bank replaying (rb,
  // while running) and the bank streaming out (ob_bank, while outing); each bank's run as started.
  reg [2:0] pend, outq;
  reg [1:0] rqn, oqn;
  reg running, outing;
  reg [1:0] rb, rq0, rq1, rq2, ob_bank, oq0, oq1, oq2;
  `LOGOS_RAMSTYLE_ASYNC(3) reg [7:0] b_entries [0:2];
  `LOGOS_RAMSTYLE_ASYNC(3) reg [7:0] b_slots [0:2];
  `LOGOS_RAMSTYLE_ASYNC(3) reg [15:0] b_first [0:2];
  reg [7:0] r_slots;
  reg [15:0] r_first, ng, total, ob, o_words;
  assign free = ~(pend | outq | (running ? 3'b001 << rb : 3'b000) | (outing ? 3'b001 << ob_bank : 3'b000));
  wire [7:0] exps_next = (rq0 == 2'd0) ? exps0 : (rq0 == 2'd1) ? exps1 : exps2;
  wire run_go = !running && rqn != 2'd0 && exps_next == 8'd0;
  // The dot pass's item (a group and a token, the group the faster) and its beat; the update pass's; the items whose
  // updates are written.
  reg [15:0] di, dg, de, dp, ui, ug, ue, up, written;
  wire [15:0] dj = dp % G, dbt = dp / G, uj = up % G, ubt = up / G;
  wire [15:0] dslot = dg / GS, uslot = ug / GS;
  // Each of the ring's four item slots: its rows' coefficients and how many are in.
  `LOGOS_RAMSTYLE(4*G) reg [31:0] coefs [0:4*G-1];
  reg [15:0] have [0:3];
  wire dot_go = running && di < total && (de == 16'd0 || written + ng > di) && di < ui + RING;
  wire upd_go = running && ui < di && have[ui[1:0]] == G;
  // A dot's beat: the row's beat and the token's key beat out of the running bank, and its tags.
  wire [15:0] xa = (dg * G + dj) * BEATS + dbt, ka_a = (dslot * T_MAX_W + de) * BEATS + dbt;
  reg [512*W-1:0] xa0_r, xa1_r, xa2_r, ka0_r, ka1_r, ka2_r;
  reg [1:0] rb_q;
  reg v_q, first_q, last_q;
  reg [15:0] row_q, e_q, j_q, slot_q;
  reg [1:0] ring_q;
  always @(posedge clk) begin
    xa0_r <= ga0[xa];
    xa1_r <= ga1[xa];
    xa2_r <= ga2[xa];
    ka0_r <= ka0[ka_a];
    ka1_r <= ka1[ka_a];
    ka2_r <= ka2[ka_a];
    rb_q <= rb;
    v_q <= !rst && dot_go;
    first_q <= dbt == 16'd0;
    last_q <= dbt == BEATS - 1;
    row_q <= dg * G + dj;
    e_q <= de;
    j_q <= dj;
    slot_q <= dslot;
    ring_q <= di[1:0];
  end
  wire [512*W-1:0] x_q = (rb_q == 2'd0) ? xa0_r : (rb_q == 2'd1) ? xa1_r : xa2_r;
  wire [512*W-1:0] k_q = (rb_q == 2'd0) ? ka0_r : (rb_q == 2'd1) ? ka1_r : ka2_r;
  reg [COEF:1] v_d, first_d, last_d;
  reg [15:0] row_d [1:COEF];
  reg [15:0] e_d [1:COEF];
  reg [15:0] j_d [1:COEF];
  reg [15:0] slot_d [1:COEF];
  reg [1:0] ring_d [1:COEF];
  always @(posedge clk) begin
    v_d <= rst ? {COEF{1'b0}} : {v_d[COEF-1:1], v_q};
    first_d <= {first_d[COEF-1:1], first_q};
    last_d <= {last_d[COEF-1:1], last_q};
    row_d[1] <= row_q; e_d[1] <= e_q; j_d[1] <= j_q; slot_d[1] <= slot_q; ring_d[1] <= ring_q;
    for (i = 2; i <= COEF; i = i + 1) begin
      row_d[i] <= row_d[i-1]; e_d[i] <= e_d[i-1]; j_d[i] <= j_d[i-1]; slot_d[i] <= slot_d[i-1]; ring_d[i] <= ring_d[i-1];
    end
  end
  // The dot's lanes, as logos_gdn_flush_w's: a beat's W products through a chain of W adds a lane.
  wire [511:0] sums;
  genvar l, j;
  generate
    for (l = 0; l < 16; l = l + 1) begin : lane
      wire [31:0] s [0:W];
      assign s[0] = first_d[@FMUL@] ? 32'd0 : s[W];
      for (j = 0; j < W; j = j + 1) begin : term
        wire [31:0] p, pd;
        logos_fmul_p m(.clk(clk), .a(x_q[512*j + 32*l +: 32]), .b(k_q[512*j + 32*l +: 32]), .y(p));
        if (j == 0) begin : now
          assign pd = p;
        end else begin : later
          logos_delay #(.W(32), .D(j * @FADD@)) dl(.clk(clk), .d(p), .q(pd));
        end
        logos_fadd_p a(.clk(clk), .a(s[j]), .b(pd), .y(s[j+1]));
      end
      assign sums[32*l +: 32] = s[W];
    end
  endgenerate
  // A row's dot through the tree, decay·dot, the row's own value of the token less it, and the gate's product: the
  // row's coefficient, into its item's ring slot.
  wire [31:0] dot, u, vu, c;
  logos_tree16_p t(.clk(clk), .lanes(sums), .sum(dot));
  // The token's decay and gate where the dot and its difference reach their multiplies, read as logos_gdn_flush_w reads
  // them (a slot's few tokens each), each place registered the clock before.
  reg [15:0] dt_at, bt_at;
  always @(posedge clk) begin
    dt_at <= slot_d[TREE-1] * T_MAX_W + e_d[TREE-1];
    bt_at <= slot_d[GATED-1] * T_MAX_W + e_d[GATED-1];
  end
  wire [31:0] decay_t = (rb_q == 2'd0) ? dc0[dt_at] : (rb_q == 2'd1) ? dc1[dt_at] : dc2[dt_at];
  wire [31:0] beta_g = (rb_q == 2'd0) ? bt0[bt_at] : (rb_q == 2'd1) ? bt1[bt_at] : bt2[bt_at];
  logos_fmul_p mu(.clk(clk), .a(decay_t), .b(dot), .y(u));
  // The row's place in its head: its value's element, its line read (its place registered the clock before) and its
  // lane taken as logos_gdn_flush_w's.
  wire [15:0] v_row = r_first + (row_d[VALUE-2] % SHARE_W), v_row_e = r_first + (row_d[VALUE-3] % SHARE_W);
  reg [15:0] va_a;
  reg [512*W-1:0] va0_r, va1_r, va2_r;
  reg [EW-1:0] v_lane;
  reg [31:0] v_val;
  always @(posedge clk) begin
    va_a <= (slot_d[VALUE-3] * T_MAX_W + e_d[VALUE-3]) * BEATS + (v_row_e >> EW);
    va0_r <= va0[va_a];
    va1_r <= va1[va_a];
    va2_r <= va2[va_a];
    v_lane <= v_row[EW-1:0];
    v_val <= (rb_q == 2'd0) ? va0_r[32 * v_lane +: 32] : (rb_q == 2'd1) ? va1_r[32 * v_lane +: 32] : va2_r[32 * v_lane +: 32];
  end
  logos_fadd_p sv(.clk(clk), .a(v_val), .b({~u[31], u[30:0]}), .y(vu));
  logos_fmul_p mb(.clk(clk), .a(beta_g), .b(vu), .y(c));
  wire coef_in = v_d[COEF] && last_d[COEF];
  // An update's beat: the row's beat and the token's key beat out of the running bank, the row's coefficient and the
  // token's decay; its place.
  wire [15:0] ua = (ug * G + uj) * BEATS + ubt, kb_a = (uslot * T_MAX_W + ue) * BEATS + ubt, ud_at = uslot * T_MAX_W + ue;
  // The update's reads of the second copy, or a streamed bank's out of it, each bank's one address.
  wire [15:0] gb0_a = (running && rb == 2'd0) ? ua : ob, gb1_a = (running && rb == 2'd1) ? ua : ob;
  wire [15:0] gb2_a = (running && rb == 2'd2) ? ua : ob;
  reg [512*W-1:0] gb0_r, gb1_r, gb2_r, kb0_r, kb1_r, kb2_r;
  reg [31:0] uc_q, ud0_r, ud1_r, ud2_r;
  reg uv_q, ul_q;
  reg [1:0] urb_q;
  reg [15:0] ua_q;
  always @(posedge clk) begin
    gb0_r <= gb0[gb0_a];
    gb1_r <= gb1[gb1_a];
    gb2_r <= gb2[gb2_a];
    kb0_r <= kb0[kb_a];
    kb1_r <= kb1[kb_a];
    kb2_r <= kb2[kb_a];
    uc_q <= coefs[ui[1:0] * G + uj];
    ud0_r <= dc0[ud_at];
    ud1_r <= dc1[ud_at];
    ud2_r <= dc2[ud_at];
    urb_q <= rb;
    uv_q <= !rst && upd_go;
    ul_q <= !rst && upd_go && up == PASS - 1;
    ua_q <= ua;
  end
  wire [512*W-1:0] ux_q = (urb_q == 2'd0) ? gb0_r : (urb_q == 2'd1) ? gb1_r : gb2_r;
  wire [512*W-1:0] uk_q = (urb_q == 2'd0) ? kb0_r : (urb_q == 2'd1) ? kb1_r : kb2_r;
  wire [31:0] ud_q = (urb_q == 2'd0) ? ud0_r : (urb_q == 2'd1) ? ud1_r : ud2_r;
  wire [512*W-1:0] upd;
  generate
    for (l = 0; l < 16 * W; l = l + 1) begin : elem
      wire [31:0] dm, ck;
      logos_fmul_p md(.clk(clk), .a(ud_q), .b(ux_q[32*l +: 32]), .y(dm));
      logos_fmul_p mc(.clk(clk), .a(uc_q), .b(uk_q[32*l +: 32]), .y(ck));
      logos_fadd_p au(.clk(clk), .a(dm), .b(ck), .y(upd[32*l +: 32]));
    end
  endgenerate
  // The update's beats written back where they were read in the running bank, an item's last marking its update
  // written; a share's rows written by address into the bank being filled.
  reg [UPDATED:1] wv_d, wl_d;
  reg [1:0] wb_d [1:UPDATED];
  reg [15:0] wa_d [1:UPDATED];
  always @(posedge clk) begin
    wv_d <= rst ? {UPDATED{1'b0}} : {wv_d[UPDATED-1:1], uv_q};
    wl_d <= rst ? {UPDATED{1'b0}} : {wl_d[UPDATED-1:1], ul_q};
    wb_d[1] <= urb_q;
    wa_d[1] <= ua_q;
    for (i = 2; i <= UPDATED; i = i + 1) begin wb_d[i] <= wb_d[i-1]; wa_d[i] <= wa_d[i-1]; end
  end
  wire [15:0] ra = ({8'd0, row_slot} * SHARE_W + row_j) * BEATS + row_beat;
  wire wb = wv_d[UPDATED];
  always @(posedge clk) begin
    if (row_wr && row_bank == 2'd0) begin ga0[ra] <= row_data; gb0[ra] <= row_data; end
    else if (wb && wb_d[UPDATED] == 2'd0) begin ga0[wa_d[UPDATED]] <= upd; gb0[wa_d[UPDATED]] <= upd; end
    if (row_wr && row_bank == 2'd1) begin ga1[ra] <= row_data; gb1[ra] <= row_data; end
    else if (wb && wb_d[UPDATED] == 2'd1) begin ga1[wa_d[UPDATED]] <= upd; gb1[wa_d[UPDATED]] <= upd; end
    if (row_wr && row_bank == 2'd2) begin ga2[ra] <= row_data; gb2[ra] <= row_data; end
    else if (wb && wb_d[UPDATED] == 2'd2) begin ga2[wa_d[UPDATED]] <= upd; gb2[wa_d[UPDATED]] <= upd; end
    if (coef_in) coefs[ring_d[COEF] * G + j_d[COEF]] <= c;
  end
  // A run ends once its last update is written and nothing of it is left on the lanes; a replayed bank streams out
  // once none is streaming.
  wire run_end = running && written == total && wv_d == {UPDATED{1'b0}} && v_d == {COEF{1'b0}};
  wire out_go = !outing && oqn != 2'd0;
  integer r;
  always @(posedge clk) begin
    if (rst) begin
      pend <= 3'b000; outq <= 3'b000; rqn <= 2'd0; oqn <= 2'd0; running <= 1'b0; rb <= 2'd0; outing <= 1'b0; ob_bank <= 2'd0;
      ob <= 16'd0;
      di <= 16'd0; dg <= 16'd0; de <= 16'd0; dp <= 16'd0; ui <= 16'd0; ug <= 16'd0; ue <= 16'd0; up <= 16'd0; written <= 16'd0;
      for (r = 0; r < 4; r = r + 1) have[r] <= 16'd0;
    end else begin
      if (start) begin pend[bank] <= 1'b1; b_entries[bank] <= entries; b_slots[bank] <= slots; b_first[bank] <= first; end
      // The banks to replay, in the order started: a start joins the back, a run takes the front.
      case ({start, run_go})
        2'b10: begin
          if (rqn == 2'd0) rq0 <= bank; else if (rqn == 2'd1) rq1 <= bank; else rq2 <= bank;
          rqn <= rqn + 2'd1;
        end
        2'b01: begin rq0 <= rq1; rq1 <= rq2; rqn <= rqn - 2'd1; end
        2'b11:
          if (rqn == 2'd1) rq0 <= bank;
          else if (rqn == 2'd2) begin rq0 <= rq1; rq1 <= bank; end
          else begin rq0 <= rq1; rq1 <= rq2; rq2 <= bank; end
        default: ;
      endcase
      // The banks to stream out, in the order replayed: a run's end joins the back, a stream takes the front.
      case ({run_end, out_go})
        2'b10: begin
          if (oqn == 2'd0) oq0 <= rb; else if (oqn == 2'd1) oq1 <= rb; else oq2 <= rb;
          oqn <= oqn + 2'd1;
        end
        2'b01: begin oq0 <= oq1; oq1 <= oq2; oqn <= oqn - 2'd1; end
        2'b11:
          if (oqn == 2'd1) oq0 <= rb;
          else if (oqn == 2'd2) begin oq0 <= oq1; oq1 <= rb; end
          else begin oq0 <= oq1; oq1 <= oq2; oq2 <= rb; end
        default: ;
      endcase
      for (r = 0; r < 4; r = r + 1)
        have[r] <= have[r] + ((coef_in && ring_d[COEF] == r[1:0]) ? 16'd1 : 16'd0)
                   - ((upd_go && up == PASS - 1 && ui[1:0] == r[1:0]) ? G : 16'd0);
      if (dot_go) begin
        if (dp == PASS - 1) begin
          dp <= 16'd0; di <= di + 16'd1;
          if (dg + 16'd1 == ng) begin dg <= 16'd0; de <= de + 16'd1; end else dg <= dg + 16'd1;
        end else dp <= dp + 16'd1;
      end
      if (upd_go) begin
        if (up == PASS - 1) begin
          up <= 16'd0; ui <= ui + 16'd1;
          if (ug + 16'd1 == ng) begin ug <= 16'd0; ue <= ue + 16'd1; end else ug <= ug + 16'd1;
        end else up <= up + 16'd1;
      end
      if (wl_d[UPDATED]) written <= written + 16'd1;
      if (run_go) begin
        running <= 1'b1; rb <= rq0; pend[rq0] <= 1'b0;
        r_slots <= b_slots[rq0]; r_first <= b_first[rq0];
        ng <= {8'd0, b_slots[rq0]} * GS;
        total <= {8'd0, b_entries[rq0]} * ({8'd0, b_slots[rq0]} * GS);
        di <= 16'd0; dg <= 16'd0; de <= 16'd0; dp <= 16'd0; ui <= 16'd0; ug <= 16'd0; ue <= 16'd0; up <= 16'd0; written <= 16'd0;
      end else if (run_end) begin
        running <= 1'b0; outq[rb] <= 1'b1;
      end
      if (out_go) begin
        outing <= 1'b1; ob_bank <= oq0; ob <= 16'd0; o_words <= {8'd0, b_slots[oq0]} * SB;
        if ({8'd0, b_slots[oq0]} * SB == 16'd0) begin outing <= 1'b0; outq[oq0] <= 1'b0; end
      end else if (outing && out_ready) begin
        if (ob + 16'd1 == o_words) begin outing <= 1'b0; outq[ob_bank] <= 1'b0; end else ob <= ob + 16'd1;
      end
    end
  end
  // The rows out, each element stored: a beat read, then its binary16 rounding (logos_f32_to_f16_p) with the beat held
  // beside it; out_half the stored binary16, out_beat the element as stored.
  reg o_v;
  reg [1:0] o_b;
  always @(posedge clk) begin
    o_v <= !rst && outing && out_ready;
    o_b <= ob_bank;
  end
  wire [512*W-1:0] o_beat = (o_b == 2'd0) ? gb0_r : (o_b == 2'd1) ? gb1_r : gb2_r;
  wire [512*W-1:0] o_raw;
  reg [512*W-1:0] o_hold;
  always @(posedge clk) o_hold <= o_beat;
  logos_delay #(.W(512*W), .D(@F16@)) dob(.clk(clk), .d(o_hold), .q(o_raw));
  reg ov_d [0:@F16@];
  integer ok;
  always @(posedge clk) begin
    ov_d[0] <= !rst && o_v;
    for (ok = 1; ok <= @F16@; ok = ok + 1) ov_d[ok] <= !rst && ov_d[ok-1];
  end
  assign out_valid = ov_d[@F16@] && !rst;
  genvar o;
  generate
    for (o = 0; o < 16 * W; o = o + 1) begin : st
      wire [31:0] back;
      logos_f32_to_f16_p enc(.clk(clk), .x(o_hold[32*o +: 32]), .h(out_half[16*o +: 16]));
      logos_f16_to_f32 dec(.h(out_half[16*o +: 16]), .x(back));
      assign out_beat[32*o +: 32] = f16 ? back : o_raw[32*o +: 32];
    end
  endgenerate
endmodule
"#;

// The read-out and the flush at 16·W elements a clock, a binary16 line a clock at W = 2. A lane takes a beat's W
// terms in order through W chained adds, so element i meets lane i mod 16 in index order exactly as numerics::fdot adds
// it, and a group is as many rows as the chain's clocks, so a lane's sum for a row comes back to the chain's first add
// as the row's next beat enters.
const GDN_WIDE_VERILOG: &str = r#"
// logos_gdn_read_p at 16·W elements a clock: rows of N elements in beats of 16·W f32 (element 16·W·b + 16·j + l in
// lane l of the beat's sixteen j), row-major, in groups of W·@FADD@, each row's carried value with its first beat and
// the head's folded query y and scale with each group's first beat; out, in the rows' order, (row·y + carried)·scale
// of each row.
module logos_gdn_read_w #(parameter N = 128, W = 2)
  (input clk, input rst,
   input [32*N-1:0] y, input [31:0] scale,
   input in_valid, output in_ready, input [512*W-1:0] in_beat, input [31:0] in_carried,
   output reg out_valid, output reg [31:0] out_read);
  localparam [15:0] BEATS = N / (16 * W);
  localparam [15:0] G = W * @FADD@;
  localparam [15:0] GB = G * BEATS;
  localparam CHAIN = W * @FADD@, TREE = @FMUL@ + CHAIN + 4 * @FADD@, CARRIED = TREE + @FADD@, LAG = CARRIED + @FMUL@;
  // Two groups of rows, row-major; each group's carried values, query and scale.
  `LOGOS_RAMSTYLE(2*GB) reg [512*W-1:0] gbuf [0:2*GB-1];
  `LOGOS_RAMSTYLE(2*G) reg [31:0] gcar [0:2*G-1];
  `LOGOS_RAMSTYLE_ASYNC(2) reg [32*N-1:0] gy [0:1];
  `LOGOS_RAMSTYLE(2) reg [31:0] gscale [0:1];
  reg [1:0] full;
  // The group being gathered and the group being sent.
  reg wr, rd, sending;
  reg [15:0] wrow, sj, wbeat, sb;
  assign in_ready = !full[wr];
  wire take = in_valid && in_ready;
  wire group_in = take && wrow == G - 1 && wbeat == BEATS - 1;
  wire group_out = sending && sj == G - 1 && sb == BEATS - 1;
  always @(posedge clk) begin
    if (take) begin
      gbuf[(wr * G + wrow) * BEATS + wbeat] <= in_beat;
      if (wbeat == 16'd0) gcar[wr * G + wrow] <= in_carried;
      if (wrow == 16'd0 && wbeat == 16'd0) begin gy[wr] <= y; gscale[wr] <= scale; end
    end
    if (rst) begin
      full <= 2'b00; wr <= 1'b0; rd <= 1'b0; sending <= 1'b0; wrow <= 16'd0; wbeat <= 16'd0; sj <= 16'd0; sb <= 16'd0;
    end else begin
      if (take) begin
        if (wbeat == BEATS - 1) begin wbeat <= 16'd0; wrow <= (wrow == G - 1) ? 16'd0 : wrow + 16'd1; end
        else wbeat <= wbeat + 16'd1;
        if (group_in) wr <= !wr;
      end
      if (group_in) full[wr] <= 1'b1;
      if (group_out) begin full[rd] <= 1'b0; rd <= !rd; sending <= 1'b0; end
      else if (!sending && full[rd]) begin sending <= 1'b1; sj <= 16'd0; sb <= 16'd0; end
      if (sending && !group_out) begin
        if (sj == G - 1) begin sj <= 16'd0; sb <= sb + 16'd1; end else sj <= sj + 16'd1;
      end
    end
  end
  // The beat at hand, its query beat, and its tags: whether it opens or closes its row, the row's carried and scale.
  reg [512*W-1:0] beat_q, yb_q;
  reg v_q, first_q, last_q;
  reg [31:0] car_q, scale_q;
  always @(posedge clk) begin
    beat_q <= gbuf[(rd * G + sj) * BEATS + sb];
    yb_q <= gy[rd][512*W*sb +: 512*W];
    v_q <= !rst && sending;
    first_q <= sb == 16'd0;
    last_q <= sb == BEATS - 1;
    car_q <= gcar[rd * G + sj];
    scale_q <= gscale[rd];
  end
  // The tags through the products, the chain, the tree, the carried value's add and the scale's multiply.
  reg [LAG:1] v_d, first_d, last_d;
  reg [31:0] car_d [1:LAG];
  reg [31:0] scale_d [1:LAG];
  integer i;
  always @(posedge clk) begin
    v_d <= rst ? {LAG{1'b0}} : {v_d[LAG-1:1], v_q};
    first_d <= {first_d[LAG-1:1], first_q};
    last_d <= {last_d[LAG-1:1], last_q};
    car_d[1] <= car_q;
    scale_d[1] <= scale_q;
    for (i = 2; i <= LAG; i = i + 1) begin car_d[i] <= car_d[i-1]; scale_d[i] <= scale_d[i-1]; end
  end
  // The lanes: each beat's W products, the j-th entering the chain's j-th add j·@FADD@ clocks after the first; the
  // chain's end is the lane's sum for the row, back at its first add as the row's next beat enters, a row's first
  // beat added to zero.
  wire [511:0] sums;
  genvar l, j;
  generate
    for (l = 0; l < 16; l = l + 1) begin : lane
      wire [31:0] s [0:W];
      assign s[0] = first_d[@FMUL@] ? 32'd0 : s[W];
      for (j = 0; j < W; j = j + 1) begin : term
        wire [31:0] p, pd;
        logos_fmul_p m(.clk(clk), .a(beat_q[512*j + 32*l +: 32]), .b(yb_q[512*j + 32*l +: 32]), .y(p));
        if (j == 0) begin : now
          assign pd = p;
        end else begin : later
          logos_delay #(.W(32), .D(j * @FADD@)) dl(.clk(clk), .d(p), .q(pd));
        end
        logos_fadd_p a(.clk(clk), .a(s[j]), .b(pd), .y(s[j+1]));
      end
      assign sums[32*l +: 32] = s[W];
    end
  endgenerate
  // A row's lanes leave the chain the clock after its last beat's last term entered it; the tree, the carried value,
  // the scale.
  wire [31:0] dot, with_carried, read;
  logos_tree16_p t(.clk(clk), .lanes(sums), .sum(dot));
  logos_fadd_p c(.clk(clk), .a(dot), .b(car_d[TREE]), .y(with_carried));
  logos_fmul_p sc(.clk(clk), .a(with_carried), .b(scale_d[CARRIED]), .y(read));
  always @(posedge clk) begin
    out_valid <= !rst && v_d[LAG] && last_d[LAG];
    out_read <= read;
  end
endmodule

// logos_gdn_flush_p at 16·W elements a clock over a whole head: the head's log written beat by beat (token log_at,
// beat log_beat of 16·W elements: its key's beat and its values' beat), each token's decay exponent and gate with its
// beat 0, each decay through the pipelined exponential; then ROWS of the head's rows from its row FIRST (all N of them
// unless given), each N elements in beats of 16·W f32, row-major, all held, taken once every decay is in. The rows are ROWS / (W·@FADD@)
// groups of W·@FADD@, a row's update its own, so a head's rows split over several flushes flush as one flush's. For every logged token in order,
// each group has a dot pass — every row's dot with the token's key, the group's beats issued beat-major so a lane's
// chain sum comes back as the row's next beat enters — then each row's coefficient beta·(v − decay·dot), v the row's own
// value of the token, then an update pass decay·m + coef·k over the group's rows, no rounding between. The dot passes
// run back to back on their lanes and the update passes on theirs: a group's update once its coefficients are in, its
// next dot once that update is written. After the last token the rows stream out row-major, each element stored once
// (rounded to binary16 and back when f16): a beat leaves the clock after each clock out_ready is high.
module logos_gdn_flush_w #(parameter N = 128, W = 2, T_MAX = 16, ROWS = N, FIRST = 0)
  (input clk, input rst,
   input log_wr, input [7:0] log_at, input [15:0] log_beat, input [512*W-1:0] log_k, input [512*W-1:0] log_v,
   input [31:0] log_decay, input [31:0] log_beta, input [7:0] entries, input f16,
   input in_valid, output in_ready, input [512*W-1:0] in_beat,
   output out_valid, input out_ready, output [512*W-1:0] out_beat, output [256*W-1:0] out_half);
  localparam [15:0] BEATS = N / (16 * W);
  localparam [15:0] G = W * @FADD@;
  localparam [15:0] NG = ROWS / G;
  localparam [15:0] PASS = G * BEATS;
  localparam [15:0] RB = ROWS * BEATS;
  localparam CHAIN = W * @FADD@, TREE = @FMUL@ + CHAIN + 4 * @FADD@, VALUE = TREE + @FMUL@;
  localparam GATED = VALUE + @FADD@, COEF = GATED + @FMUL@, UPDATED = @FMUL@ + @FADD@;
  localparam EW = $clog2(16 * W);
  // The coefficients' ring: four slots, a token's at its index's two low bits.
  localparam [15:0] RING = 16'd4;
  localparam IN = 2'd0, RUN = 2'd1, OUT = 2'd2;
  // The log: each token's key, twice (one copy for each pass), and values, its decay and its gate.
  `LOGOS_RAMSTYLE(T_MAX*BEATS) reg [512*W-1:0] ka [0:T_MAX*BEATS-1];
  `LOGOS_RAMSTYLE(T_MAX*BEATS) reg [512*W-1:0] kb [0:T_MAX*BEATS-1];
  `LOGOS_RAMSTYLE(T_MAX*BEATS) reg [512*W-1:0] vmem [0:T_MAX*BEATS-1];
  `LOGOS_RAMSTYLE_ASYNC(T_MAX) reg [31:0] decays [0:T_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(T_MAX) reg [31:0] betas [0:T_MAX-1];
  wire [31:0] ex;
  reg [@EXP@:1] ex_v;
  reg [7:0] ex_at [1:@EXP@];
  reg [7:0] exps;
  logos_exp_p xp(.clk(clk), .x_in(log_decay), .y(ex));
  integer i;
  always @(posedge clk) begin
    if (log_wr) begin
      ka[log_at * BEATS + log_beat] <= log_k;
      kb[log_at * BEATS + log_beat] <= log_k;
      vmem[log_at * BEATS + log_beat] <= log_v;
      if (log_beat == 16'd0) betas[log_at] <= log_beta;
    end
    ex_v <= rst ? {@EXP@{1'b0}} : {ex_v[@EXP@-1:1], log_wr && log_beat == 16'd0};
    ex_at[1] <= log_at;
    for (i = 2; i <= @EXP@; i = i + 1) ex_at[i] <= ex_at[i-1];
    if (ex_v[@EXP@]) decays[ex_at[@EXP@]] <= ex;
    if (rst) exps <= 8'd0;
    else exps <= exps + ((log_wr && log_beat == 16'd0) ? 8'd1 : 8'd0) - (ex_v[@EXP@] ? 8'd1 : 8'd0);
  end
  // The head's rows, row-major, twice: the dot pass reads one copy and the update pass the other, and both are written.
  `LOGOS_RAMSTYLE(RB) reg [512*W-1:0] ga [0:RB-1];
  `LOGOS_RAMSTYLE(RB) reg [512*W-1:0] gb [0:RB-1];
  reg [1:0] state;
  reg [15:0] inb, ob;
  wire [15:0] total = {8'd0, entries} * NG;
  assign in_ready = state == IN && exps == 8'd0;
  wire take = in_valid && in_ready;
  // The dot pass's item (a group and a token) and its beat; the update pass's; the items whose updates are written.
  reg [15:0] di, dp, ui, up, written;
  wire [15:0] dg = di % NG, de = di / NG, ug = ui % NG, ue = ui / NG;
  wire [15:0] dj = dp % G, dbt = dp / G, uj = up % G, ubt = up / G;
  // Each of the ring's four item slots: its rows' coefficients and how many are in.
  `LOGOS_RAMSTYLE(4*G) reg [31:0] coefs [0:4*G-1];
  reg [15:0] have [0:3];
  wire dot_go = state == RUN && di < total && (de == 16'd0 || written + NG > di) && di < ui + RING;
  wire upd_go = state == RUN && ui < di && have[ui[1:0]] == G;
  // A dot's beat: the row's beat, the token's key beat, and its tags.
  reg [512*W-1:0] x_q, k_q;
  reg v_q, first_q, last_q;
  reg [15:0] row_q, e_q, j_q;
  reg [1:0] slot_q;
  always @(posedge clk) begin
    x_q <= ga[(dg * G + dj) * BEATS + dbt];
    k_q <= ka[de * BEATS + dbt];
    v_q <= !rst && dot_go;
    first_q <= dbt == 16'd0;
    last_q <= dbt == BEATS - 1;
    row_q <= dg * G + dj;
    e_q <= de;
    j_q <= dj;
    slot_q <= di[1:0];
  end
  reg [COEF:1] v_d, first_d, last_d;
  reg [15:0] row_d [1:COEF];
  reg [15:0] e_d [1:COEF];
  reg [15:0] j_d [1:COEF];
  reg [1:0] slot_d [1:COEF];
  always @(posedge clk) begin
    v_d <= rst ? {COEF{1'b0}} : {v_d[COEF-1:1], v_q};
    first_d <= {first_d[COEF-1:1], first_q};
    last_d <= {last_d[COEF-1:1], last_q};
    row_d[1] <= row_q; e_d[1] <= e_q; j_d[1] <= j_q; slot_d[1] <= slot_q;
    for (i = 2; i <= COEF; i = i + 1) begin row_d[i] <= row_d[i-1]; e_d[i] <= e_d[i-1]; j_d[i] <= j_d[i-1]; slot_d[i] <= slot_d[i-1]; end
  end
  // The dot's lanes, as logos_gdn_read_w's: a beat's W products through a chain of W adds a lane.
  wire [511:0] sums;
  genvar l, j;
  generate
    for (l = 0; l < 16; l = l + 1) begin : lane
      wire [31:0] s [0:W];
      assign s[0] = first_d[@FMUL@] ? 32'd0 : s[W];
      for (j = 0; j < W; j = j + 1) begin : term
        wire [31:0] p, pd;
        logos_fmul_p m(.clk(clk), .a(x_q[512*j + 32*l +: 32]), .b(k_q[512*j + 32*l +: 32]), .y(p));
        if (j == 0) begin : now
          assign pd = p;
        end else begin : later
          logos_delay #(.W(32), .D(j * @FADD@)) dl(.clk(clk), .d(p), .q(pd));
        end
        logos_fadd_p a(.clk(clk), .a(s[j]), .b(pd), .y(s[j+1]));
      end
      assign sums[32*l +: 32] = s[W];
    end
  endgenerate
  // A row's dot through the tree, decay·dot, the row's own value of the token less it, and the gate's product: the
  // row's coefficient, into its item's slot.
  wire [31:0] dot, u, vu, c;
  logos_tree16_p t(.clk(clk), .lanes(sums), .sum(dot));
  logos_fmul_p mu(.clk(clk), .a(decays[e_d[TREE]]), .b(dot), .y(u));
  reg [512*W-1:0] v_line;
  reg [EW-1:0] v_lane;
  // The row's place in the head, its value's element.
  localparam [15:0] FIRST_ROW = FIRST;
  wire [15:0] v_row = row_d[VALUE-2] + FIRST_ROW;
  reg [31:0] v_val;
  always @(posedge clk) begin
    v_line <= vmem[e_d[VALUE-2] * BEATS + (v_row >> EW)];
    v_lane <= v_row[EW-1:0];
    v_val <= v_line[32 * v_lane +: 32];
  end
  logos_fadd_p sv(.clk(clk), .a(v_val), .b({~u[31], u[30:0]}), .y(vu));
  logos_fmul_p mb(.clk(clk), .a(betas[e_d[GATED]]), .b(vu), .y(c));
  wire coef_in = v_d[COEF] && last_d[COEF];
  // An update's beat: the row's beat, the token's key beat, the row's coefficient and the token's decay; its place.
  reg [512*W-1:0] ux_q, uk_q;
  reg [31:0] uc_q, ud_q;
  reg uv_q, ul_q;
  reg [15:0] ua_q;
  always @(posedge clk) begin
    ux_q <= gb[(ug * G + uj) * BEATS + ubt];
    uk_q <= kb[ue * BEATS + ubt];
    uc_q <= coefs[ui[1:0] * G + uj];
    ud_q <= decays[ue];
    uv_q <= !rst && upd_go;
    ul_q <= !rst && upd_go && up == PASS - 1;
    ua_q <= (ug * G + uj) * BEATS + ubt;
  end
  wire [512*W-1:0] upd;
  generate
    for (l = 0; l < 16 * W; l = l + 1) begin : elem
      wire [31:0] dm, ck;
      logos_fmul_p md(.clk(clk), .a(ud_q), .b(ux_q[32*l +: 32]), .y(dm));
      logos_fmul_p mc(.clk(clk), .a(uc_q), .b(uk_q[32*l +: 32]), .y(ck));
      logos_fadd_p au(.clk(clk), .a(dm), .b(ck), .y(upd[32*l +: 32]));
    end
  endgenerate
  // The update's beats written back where they were read, an item's last marking its update written.
  reg [UPDATED:1] wv_d, wl_d;
  reg [15:0] wa_d [1:UPDATED];
  always @(posedge clk) begin
    wv_d <= {wv_d[UPDATED-1:1], uv_q};
    wl_d <= {wl_d[UPDATED-1:1], ul_q};
    wa_d[1] <= ua_q;
    for (i = 2; i <= UPDATED; i = i + 1) wa_d[i] <= wa_d[i-1];
    if (rst) begin wv_d <= {UPDATED{1'b0}}; wl_d <= {UPDATED{1'b0}}; end
  end
  always @(posedge clk) begin
    if (take) begin ga[inb] <= in_beat; gb[inb] <= in_beat; end
    else if (wv_d[UPDATED]) begin ga[wa_d[UPDATED]] <= upd; gb[wa_d[UPDATED]] <= upd; end
    if (coef_in) coefs[slot_d[COEF] * G + j_d[COEF]] <= c;
  end
  integer r;
  always @(posedge clk) begin
    if (rst) begin
      state <= IN; inb <= 16'd0; ob <= 16'd0; di <= 16'd0; dp <= 16'd0; ui <= 16'd0; up <= 16'd0; written <= 16'd0;
      for (r = 0; r < 4; r = r + 1) have[r] <= 16'd0;
    end else begin
      for (r = 0; r < 4; r = r + 1)
        have[r] <= have[r] + ((coef_in && slot_d[COEF] == r[1:0]) ? 16'd1 : 16'd0)
                   - ((upd_go && up == PASS - 1 && ui[1:0] == r[1:0]) ? G : 16'd0);
      if (dot_go) begin
        if (dp == PASS - 1) begin dp <= 16'd0; di <= di + 16'd1; end else dp <= dp + 16'd1;
      end
      if (upd_go) begin
        if (up == PASS - 1) begin up <= 16'd0; ui <= ui + 16'd1; end else up <= up + 16'd1;
      end
      if (wl_d[UPDATED]) written <= written + 16'd1;
      case (state)
        IN: if (take) begin
          if (inb == RB - 1) begin inb <= 16'd0; di <= 16'd0; dp <= 16'd0; ui <= 16'd0; up <= 16'd0; written <= 16'd0; state <= RUN; end
          else inb <= inb + 16'd1;
        end
        RUN: if (written == total && !wv_d[UPDATED]) begin ob <= 16'd0; state <= OUT; end
        OUT: if (out_ready) begin
          if (ob == RB - 1) state <= IN; else ob <= ob + 16'd1;
        end
        default: state <= IN;
      endcase
    end
  end
  // The rows out, each element stored: a beat read, then its binary16 rounding (logos_f32_to_f16_p) with the beat held
  // beside it; out_half the stored binary16, out_beat the element as stored.
  reg [512*W-1:0] o_beat;
  reg o_v;
  always @(posedge clk) begin
    o_v <= !rst && state == OUT && out_ready;
    o_beat <= gb[ob];
  end
  // The beat held beside its rounding, its valid in registers the reset clears.
  wire [512*W-1:0] o_raw;
  logos_delay #(.W(512*W), .D(@F16@)) dob(.clk(clk), .d(o_beat), .q(o_raw));
  reg ov_d [1:@F16@];
  integer ok;
  always @(posedge clk) begin
    ov_d[1] <= !rst && o_v;
    for (ok = 2; ok <= @F16@; ok = ok + 1) ov_d[ok] <= !rst && ov_d[ok-1];
  end
  assign out_valid = ov_d[@F16@] && !rst;
  genvar o;
  generate
    for (o = 0; o < 16 * W; o = o + 1) begin : st
      wire [31:0] back;
      logos_f32_to_f16_p enc(.clk(clk), .x(o_beat[32*o +: 32]), .h(out_half[16*o +: 16]));
      logos_f16_to_f32 dec(.h(out_half[16*o +: 16]), .x(back));
      assign out_beat[32*o +: 32] = f16 ? back : o_raw[32*o +: 32];
    end
  endgenerate
endmodule
"#;

const GDN_GATE_VERILOG: &str = r#"
// A lane's read-outs gated and normed head by head as the machine's gated norm makes them: each head RMS-normed by the
// norm's weight w (numerics::rmsnorm) and each element then times silu of the head's gate z. A head's HD read-outs come
// in a clock or more apart (e_v, e_o), the head's first with the store beat its read-outs fill from (e_at); its gate's
// beats are written beforehand into the slot the head takes (z_we, z_wa = {slot, beat}, the beat in BA bits; heads take
// slots 0..NZ-1 in turn), the weight's before the job's first head (w_we, its beat w_wa). Each read-out is kept and its
// square added into lane j mod 16 of the head's sum from +0, fdot's order, in one of two banks by the head's parity;
// once its last is in, the head's sum through the tree, times RN (recip(HD) as the build's numerics make it), plus eps
// and through the reciprocal square root is its r. Then each element's ((o·r)·w)·silu(z) a clock at a time, the heads
// in turn, gathered into beats written from the head's store beat in order (st_we). `gated` counts the heads whose
// every beat has been written; a head's slot is free again once it is gated.
module logos_gdn_gate_p #(parameter HD = 128, SA = 10, NZ = 16, parameter [31:0] RN = 32'h3c00_0000)
  (input clk, input rst,
   input [31:0] eps,
   input w_we, input [7:0] w_wa, input [511:0] w_wd,
   input z_we, input [15:0] z_wa, input [511:0] z_wd,
   input e_v, input [31:0] e_o, input [SA-1:0] e_at,
   output reg st_we, output reg [SA-1:0] st_wa, output reg [511:0] st_wd,
   output reg [15:0] gated, output busy);
  localparam BEATS = HD / 16, BA = BEATS > 1 ? $clog2(BEATS) : 1, ZB = $clog2(NZ), JB = $clog2(HD);
  localparam [31:0] HD_LAST = HD - 1;
  localparam [JB-1:0] J_LAST = HD_LAST[JB-1:0];
  localparam [31:0] ONE = 32'h3f80_0000;
  localparam FMUL = @FMUL@, FADD = @FADD@, TREE = @TREE16@, RSQRT = @RSQRT@, EXP = @EXP@, RECIP = @RECIP@;
  localparam SIL = EXP + FADD + RECIP + FMUL, SCALE = TREE + FMUL + FADD + RSQRT;
  // The weight, the gates and the read-outs kept, each read a clock after its address.
  `LOGOS_RAMSTYLE(BEATS) reg [511:0] wm [0:BEATS-1];
  `LOGOS_RAMSTYLE(NZ << BA) reg [511:0] zm [0:(NZ << BA)-1];
  `LOGOS_RAMSTYLE(NZ << JB) reg [31:0] om [0:(NZ << JB)-1];
  `LOGOS_RAMSTYLE_ASYNC(NZ) reg [SA-1:0] at [0:NZ-1];
  `LOGOS_RAMSTYLE_ASYNC(NZ) reg [31:0] rr [0:NZ-1];
  always @(posedge clk) begin
    if (w_we) wm[w_wa[BA-1:0]] <= w_wd;
    if (z_we) zm[z_wa[ZB+BA-1:0]] <= z_wd;
  end

  // ── The sums ─────────────────────────────────────────────────────────────────────────────────────────────────────
  // The head coming in: its slot, its element; each element's square, then its lane's sum, tagged by lane, bank, first
  // beat and last element.
  reg [15:0] a_n;
  reg [JB-1:0] a_j;
  wire [ZB-1:0] a_s = a_n[ZB-1:0];
  always @(posedge clk) begin
    if (e_v) begin
      om[{a_s, a_j}] <= e_o;
      if (a_j == {JB{1'b0}}) at[a_s] <= e_at;
      a_j <= a_j + 1'b1;
      if (a_j == J_LAST) a_n <= a_n + 16'd1;
    end
    if (rst) begin a_n <= 16'd0; a_j <= {JB{1'b0}}; end
  end
  wire [31:0] sq;
  logos_fmul_p usq(.clk(clk), .a(e_o), .b(e_o), .y(sq));
  // Each tag's landing's flight is the gate's busy's own reckoning (its heads in and gated) and is not read.
  wire sq_flight, su_flight, r_flight, y_flight;
  wire [3:0] sq_lane;
  wire sq_v, sq_bank, sq_first, sq_last;
  wire [ZB-1:0] sq_slot;
  logos_landing #(.W(7 + ZB), .D(FMUL)) dsq(.clk(clk), .rst(rst), .in_v(e_v),
    .in_d({a_j[3:0], a_s[0], (a_j >> 4) == {JB{1'b0}}, a_j == J_LAST, a_s}), .out_v(sq_v), .out_d({sq_lane, sq_bank, sq_first, sq_last, sq_slot}),
    .flight(sq_flight));
  reg [511:0] acc0, acc1;
  wire [511:0] acc_q = sq_bank ? acc1 : acc0;
  wire [31:0] sum;
  logos_fadd_p uacc(.clk(clk), .a(sq_first ? 32'd0 : acc_q[32*sq_lane +: 32]), .b(sq), .y(sum));
  wire [3:0] su_lane;
  wire su_v, su_bank, su_last;
  wire [ZB-1:0] su_slot;
  logos_landing #(.W(6 + ZB), .D(FADD)) dsu(.clk(clk), .rst(rst), .in_v(sq_v),
    .in_d({sq_lane, sq_bank, sq_last, sq_slot}), .out_v(su_v), .out_d({su_lane, su_bank, su_last, su_slot}), .flight(su_flight));
  always @(posedge clk) begin
    if (su_v && !su_bank) acc0[32*su_lane +: 32] <= sum;
    if (su_v && su_bank) acc1[32*su_lane +: 32] <= sum;
  end

  // ── The scale ────────────────────────────────────────────────────────────────────────────────────────────────────
  // A clock after a head's last sum: its bank through the tree, times recip(HD), plus eps, through rsqrt: its r.
  reg t_v, t_bank;
  reg [ZB-1:0] t_slot;
  always @(posedge clk) begin
    t_v <= !rst && su_v && su_last;
    t_bank <= su_bank;
    t_slot <= su_slot;
  end
  wire [31:0] tsum, tm, te, r;
  logos_tree16_p utr(.clk(clk), .lanes(t_bank ? acc1 : acc0), .sum(tsum));
  logos_fmul_p utm(.clk(clk), .a(tsum), .b(RN), .y(tm));
  logos_fadd_p ute(.clk(clk), .a(tm), .b(eps), .y(te));
  logos_rsqrt_p utq(.clk(clk), .x_in(te), .y(r));
  wire r_v;
  wire [ZB-1:0] r_slot;
  logos_landing #(.W(ZB), .D(SCALE)) dr(.clk(clk), .rst(rst), .in_v(t_v), .in_d(t_slot), .out_v(r_v), .out_d(r_slot), .flight(r_flight));
  reg [15:0] scaled;
  always @(posedge clk) begin
    if (r_v) rr[r_slot] <= r;
    scaled <= rst ? 16'd0 : scaled + (r_v ? 16'd1 : 16'd0);
  end

  // ── The products ─────────────────────────────────────────────────────────────────────────────────────────────────
  // The head going out: its slot, its element; issued once the head's r is kept, element after element; each read a
  // clock after its address, chosen a clock after that.
  reg [15:0] b_n;
  reg [JB-1:0] b_j;
  wire [JB-1:0] b_hi = b_j >> 4;
  reg b_on;
  wire [ZB-1:0] b_s = b_n[ZB-1:0];
  wire b_go = b_on || scaled != b_n;
  reg [31:0] b_r;
  always @(posedge clk) begin
    if (b_go) begin
      b_j <= b_j + 1'b1;
      if (b_j == J_LAST) b_n <= b_n + 16'd1;
      b_on <= b_j != J_LAST;
    end
    if (!b_on) b_r <= rr[b_s];
    if (rst) begin b_n <= 16'd0; b_j <= {JB{1'b0}}; b_on <= 1'b0; end
  end
  reg [31:0] o1, r1;
  reg [511:0] w1, z1;
  reg [3:0] k1;
  reg v1;
  reg [31:0] o2, w2, z2, r2;
  reg [3:0] k2;
  reg v2, last1, last2;
  reg [BA-1:0] bt1, bt2;
  reg [ZB-1:0] s1, s2;
  always @(posedge clk) begin
    v1 <= !rst && b_go; k1 <= b_j[3:0]; bt1 <= b_hi[BA-1:0]; s1 <= b_s; last1 <= b_j == J_LAST;
    o1 <= om[{b_s, b_j}]; w1 <= wm[b_hi[BA-1:0]]; z1 <= zm[{b_s, b_hi[BA-1:0]}]; r1 <= b_on ? b_r : rr[b_s];
    v2 <= !rst && v1; k2 <= k1; bt2 <= bt1; s2 <= s1; last2 <= last1;
    o2 <= o1; r2 <= r1; w2 <= w1[32*k1 +: 32]; z2 <= z1[32*k1 +: 32];
  end
  // ((o·r)·w), and beside it silu(z) = z·recip(1 + exp(-ftz(z))), then their product.
  wire [31:0] zf = `LOGOS_FTZ(z2);
  wire [31:0] p1, p2, p2d, w2d, e, s, sig, z2d, g, y;
  logos_fmul_p up1(.clk(clk), .a(o2), .b(r2), .y(p1));
  logos_delay #(.W(32), .D(FMUL)) dw(.clk(clk), .d(w2), .q(w2d));
  logos_fmul_p up2(.clk(clk), .a(p1), .b(w2d), .y(p2));
  logos_exp_p ux(.clk(clk), .x_in({~zf[31], zf[30:0]}), .y(e));
  logos_fadd_p ua(.clk(clk), .a(ONE), .b(e), .y(s));
  logos_recip_p ur(.clk(clk), .x_in(s), .y(sig));
  logos_delay #(.W(32), .D(EXP + FADD + RECIP)) dz(.clk(clk), .d(z2), .q(z2d));
  logos_fmul_p ug(.clk(clk), .a(z2d), .b(sig), .y(g));
  logos_delay #(.W(32), .D(SIL - 2 * FMUL)) dp(.clk(clk), .d(p2), .q(p2d));
  logos_fmul_p uy(.clk(clk), .a(p2d), .b(g), .y(y));
  wire y_v, y_last;
  wire [3:0] y_k;
  wire [BA-1:0] y_bt;
  wire [ZB-1:0] y_s;
  logos_landing #(.W(5 + BA + ZB), .D(SIL + FMUL)) dy(.clk(clk), .rst(rst), .in_v(v2), .in_d({k2, last2, bt2, s2}),
    .out_v(y_v), .out_d({y_k, y_last, y_bt, y_s}), .flight(y_flight));
  // The beats out: sixteen elements gathered, written at the head's store beat plus the beat's place.
  reg [479:0] gath;
  reg st_last;
  always @(posedge clk) begin
    st_we <= 1'b0;
    if (y_v) begin
      if (y_k != 4'd15) gath[32*y_k +: 32] <= y;
      else begin st_we <= !rst; st_last <= y_last; st_wa <= at[y_s] + {{(SA-BA){1'b0}}, y_bt}; st_wd <= {y, gath}; end
    end
    gated <= rst ? 16'd0 : gated + ((st_we && st_last) ? 16'd1 : 16'd0);
  end
  assign busy = a_n != gated || a_j != {JB{1'b0}};
endmodule
"#;
