// hart_ddr2_phy — the DDR2's pins on the Artix-7, for `hart_ddr2`'s frames: four data beats, two CK cycles per
// frame clock (c62), serialized at CK = 125 MHz (c125).
//
// CK, the commands and DQS go through OSERDESE2 on c125, so they leave with one latency and their relative timing is
// fixed by construction. CK is 0101 across a frame — it rises in the middle of each command slot, so the command
// pins (slot 0 the frame's command, slot 1 a deselect) have half a CK of setup and of hold. DQS toggles in the same
// phase as CK (tDQSS = 0) and is driven low for the preamble and postamble frames; DQ goes out on c125q, a quarter CK
// later, so every DQS edge lands in the middle of its beat. Reads come in through IDELAYE2 (loaded per lane by the
// controller's calibration) into ISERDESE2 on the capture clock, and leave as a frame of four beats, earliest first.
//
// CAPTURE_QUARTER picks the capture clock (c125q, else c125) and FIRST_BEAT_ON_Q4 the ISERDESE2 output that holds
// a frame's earliest beat; the board's loopback probe measures both.
module hart_ddr2_phy #(
  parameter CAPTURE_QUARTER = 1,
  parameter FIRST_BEAT_ON_Q4 = 1
) (
  input             c125,
  input             c125q,
  input             c62,
  input             c200,
  input             rst,
  output            idelay_ready,
  input             cke,
  input             cs_n,
  input             ras_n,
  input             cas_n,
  input             we_n,
  input      [2:0]  ba,
  input      [12:0] a,
  input             dq_oe,
  input      [63:0] dq_out,
  input             dqs_oe,
  input             dqs_toggle,
  output reg [63:0] rd_frame,
  input             dly_ld,
  input      [9:0]  dly_tap,
  output            ddr2_ck_p,
  output            ddr2_ck_n,
  output            ddr2_cke,
  output            ddr2_cs_n,
  output            ddr2_ras_n,
  output            ddr2_cas_n,
  output            ddr2_we_n,
  output     [2:0]  ddr2_ba,
  output     [12:0] ddr2_addr,
  output            ddr2_odt,
  output     [1:0]  ddr2_dm,
  inout      [15:0] ddr2_dq,
  inout      [1:0]  ddr2_dqs_p,
  inout      [1:0]  ddr2_dqs_n
);
  IDELAYCTRL idelayctrl (.REFCLK(c200), .RST(rst), .RDY(idelay_ready));

  // ── CK and the commands: {slot 0, slot 0, slot 1, slot 1} a frame ────────────────────────────────────────────
  wire ck_p_q, ck_n_q;
  OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(1), .SERDES_MODE("MASTER"))
    ser_ck_p (.CLK(c125), .CLKDIV(c62), .D1(1'b0), .D2(1'b1), .D3(1'b0), .D4(1'b1), .OCE(1'b1), .RST(rst), .OQ(ck_p_q));
  OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(1), .SERDES_MODE("MASTER"))
    ser_ck_n (.CLK(c125), .CLKDIV(c62), .D1(1'b1), .D2(1'b0), .D3(1'b1), .D4(1'b0), .OCE(1'b1), .RST(rst), .OQ(ck_n_q));
  OBUF ob_ck_p (.I(ck_p_q), .O(ddr2_ck_p));
  OBUF ob_ck_n (.I(ck_n_q), .O(ddr2_ck_n));

  // Each command pin: the frame's command in slot 0; in slot 1 a deselect (CS# high), the others holding.
  wire [22:0] slot0 = {cke, 1'b0, cs_n, ras_n, cas_n, we_n, ba, a};
  wire [22:0] slot1 = {cke, 1'b0, 1'b1, ras_n, cas_n, we_n, ba, a};
  wire [22:0] cmd_q;
  genvar i;
  generate for (i = 0; i < 23; i = i + 1) begin : cmd
    OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(1), .SERDES_MODE("MASTER"))
      ser (.CLK(c125), .CLKDIV(c62), .D1(slot0[i]), .D2(slot0[i]), .D3(slot1[i]), .D4(slot1[i]), .OCE(1'b1),
           .RST(rst), .OQ(cmd_q[i]));
  end endgenerate
  OBUF ob_cke (.I(cmd_q[22]), .O(ddr2_cke));
  OBUF ob_odt (.I(cmd_q[21]), .O(ddr2_odt));
  OBUF ob_cs (.I(cmd_q[20]), .O(ddr2_cs_n));
  OBUF ob_ras (.I(cmd_q[19]), .O(ddr2_ras_n));
  OBUF ob_cas (.I(cmd_q[18]), .O(ddr2_cas_n));
  OBUF ob_we (.I(cmd_q[17]), .O(ddr2_we_n));
  generate for (i = 0; i < 3; i = i + 1) begin : bap
    OBUF ob (.I(cmd_q[13 + i]), .O(ddr2_ba[i]));
  end endgenerate
  generate for (i = 0; i < 13; i = i + 1) begin : ap
    OBUF ob (.I(cmd_q[i]), .O(ddr2_addr[i]));
  end endgenerate
  // Every line is written whole: the data masks stay low.
  OBUF ob_dm0 (.I(1'b0), .O(ddr2_dm[0]));
  OBUF ob_dm1 (.I(1'b0), .O(ddr2_dm[1]));

  // ── DQS: toggling in CK's phase while data goes out, low around it, released otherwise ──────────────────────
  generate for (i = 0; i < 2; i = i + 1) begin : dqs
    wire p_q, p_t, n_q, n_t;
    OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(1), .SERDES_MODE("MASTER"))
      ser_p (.CLK(c125), .CLKDIV(c62), .D1(1'b0), .D2(dqs_toggle), .D3(1'b0), .D4(dqs_toggle), .OCE(1'b1), .RST(rst),
             .T1(!dqs_oe), .TCE(1'b1), .OQ(p_q), .TQ(p_t));
    OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(1), .SERDES_MODE("MASTER"))
      ser_n (.CLK(c125), .CLKDIV(c62), .D1(1'b1), .D2(!dqs_toggle), .D3(1'b1), .D4(!dqs_toggle), .OCE(1'b1), .RST(rst),
             .T1(!dqs_oe), .TCE(1'b1), .OQ(n_q), .TQ(n_t));
    IOBUF io_p (.IO(ddr2_dqs_p[i]), .I(p_q), .T(p_t), .O());
    IOBUF io_n (.IO(ddr2_dqs_n[i]), .I(n_q), .T(n_t), .O());
  end endgenerate

  // ── DQ: out a quarter CK after DQS, in through each lane's delay into the deserializer ──────────────────────
  wire c_cap = CAPTURE_QUARTER ? c125q : c125;
  wire [63:0] q;
  generate for (i = 0; i < 16; i = i + 1) begin : dq
    wire oq, tq, din, dly, q1, q2, q3, q4;
    OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(1), .SERDES_MODE("MASTER"))
      ser (.CLK(c125q), .CLKDIV(c62), .D1(dq_out[i]), .D2(dq_out[16 + i]), .D3(dq_out[32 + i]), .D4(dq_out[48 + i]),
           .OCE(1'b1), .RST(rst), .T1(!dq_oe), .TCE(1'b1), .OQ(oq), .TQ(tq));
    IOBUF io (.IO(ddr2_dq[i]), .I(oq), .T(tq), .O(din));
    IDELAYE2 #(.IDELAY_TYPE("VAR_LOAD"), .DELAY_SRC("IDATAIN"), .REFCLK_FREQUENCY(200.0), .SIGNAL_PATTERN("DATA"),
               .HIGH_PERFORMANCE_MODE("TRUE"), .PIPE_SEL("FALSE"), .CINVCTRL_SEL("FALSE"), .IDELAY_VALUE(0))
      delay (.C(c62), .LD(dly_ld), .CE(1'b0), .INC(1'b0), .CNTVALUEIN(i < 8 ? dly_tap[4:0] : dly_tap[9:5]),
             .IDATAIN(din), .DATAOUT(dly));
    ISERDESE2 #(.DATA_RATE("DDR"), .DATA_WIDTH(4), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("IFD"), .NUM_CE(1),
                .SERDES_MODE("MASTER"))
      des (.CLK(c_cap), .CLKB(!c_cap), .CLKDIV(c62), .DDLY(dly), .CE1(1'b1), .RST(rst), .BITSLIP(1'b0),
           .Q1(q1), .Q2(q2), .Q3(q3), .Q4(q4));
    assign q[i]      = FIRST_BEAT_ON_Q4 ? q4 : q1;
    assign q[16 + i] = FIRST_BEAT_ON_Q4 ? q3 : q2;
    assign q[32 + i] = FIRST_BEAT_ON_Q4 ? q2 : q3;
    assign q[48 + i] = FIRST_BEAT_ON_Q4 ? q1 : q4;
  end endgenerate
  always @(posedge c62) rd_frame <= q;
endmodule
