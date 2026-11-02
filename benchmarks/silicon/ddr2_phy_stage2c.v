// ddr2_phy_stage2c — DDR2 PHY bring-up Stage 2c: DATA PATH + read-eye calibration (coarse first pass).
// After init, a command FSM does ACTIVATE -> WRITE -> PRE -> ACTIVATE -> READ over bank0/row0/col0,
// sweeping the DQ IDELAY tap 0..31. To keep the I/O logic light enough for the open router, the write
// holds DQ at a constant pattern for the whole write window (both DDR beats identical, so no per-DQ
// ODDR) and the read captures DQ single-rate through IDELAYE2 into a fabric register (no per-DQ IDDR).
// DQS is a real toggling strobe (one ODDR + IOBUFDS per byte-lane). For each tap it compares the
// readback to the pattern and records the pass window, then streams a UART summary
// {0xC5, best_tap, npass, cap[15:8], cap[7:0], 0x5A}.
module ddr2_phy_stage2c (
  input clk,
  output reg uart_tx,
  output led,
  output [12:0] ddr2_addr, output [2:0] ddr2_ba,
  output ddr2_ras_n, output ddr2_cas_n, output ddr2_we_n, output ddr2_cs_n,
  output ddr2_cke, output ddr2_odt, output [1:0] ddr2_dm,
  output ddr2_ck_p, output ddr2_ck_n,
  inout [15:0] ddr2_dq, inout [1:0] ddr2_dqs_p, inout [1:0] ddr2_dqs_n
);
  wire clkfb, ddr_raw, ref_raw, cap_raw, ck180_raw, pll_locked, ddr_clk, ref200, ddr_clk90, ddr_clk180;
  PLLE2_BASE #(
    .CLKIN1_PERIOD(10.0), .CLKFBOUT_MULT(10), .DIVCLK_DIVIDE(1),
    .CLKOUT0_DIVIDE(8), .CLKOUT1_DIVIDE(5), .CLKOUT2_DIVIDE(8), .CLKOUT2_PHASE(90.0),
    .CLKOUT3_DIVIDE(8), .CLKOUT3_PHASE(180.0)
  ) u_pll (
    .CLKIN1(clk), .CLKFBIN(clkfb), .CLKFBOUT(clkfb),
    .CLKOUT0(ddr_raw), .CLKOUT1(ref_raw), .CLKOUT2(cap_raw), .CLKOUT3(ck180_raw),
    .LOCKED(pll_locked), .RST(1'b0), .PWRDWN(1'b0)
  );
  BUFG bg0(.I(ddr_raw), .O(ddr_clk));
  BUFG bg1(.I(ref_raw), .O(ref200));
  BUFG bg2(.I(cap_raw), .O(ddr_clk90));
  BUFG bg3(.I(ck180_raw), .O(ddr_clk180));

  reg [15:0] por = 16'd0;
  wire rst = (por < 16'd1000) | ~pll_locked;
  always @(posedge ddr_clk) if (por < 16'd1000) por <= por + 16'd1;

  wire idelay_rdy;
  IDELAYCTRL u_idc(.REFCLK(ref200), .RST(~pll_locked), .RDY(idelay_rdy));

  OBUF u_ckp(.I(ddr_clk),    .O(ddr2_ck_p));
  OBUF u_ckn(.I(ddr_clk180), .O(ddr2_ck_n));

  // init FSM owns the command bus until init_done
  wire i_cke, i_cs, i_ras, i_cas, i_we, init_done; wire [1:0] i_ba; wire [12:0] i_a;
  ddr2_init_fsm u_init(.clk(ddr_clk), .rst(rst), .cke(i_cke), .cs_n(i_cs), .ras_n(i_ras),
    .cas_n(i_cas), .we_n(i_we), .ba(i_ba), .a(i_a), .init_done(init_done));

  localparam [15:0] PAT = 16'h1234;
  reg [4:0] tap; reg tap_ld; reg [9:0] cmd_cnt; reg [1:0] phase;
  localparam P_WAIT=2'd0, P_RUN=2'd1, P_NEXT=2'd2, P_REPORT=2'd3;
  reg c_cs, c_ras, c_cas, c_we, c_odt; reg [2:0] c_ba; reg [12:0] c_a;
  reg dq_oe;

  // ---- DQ: IOBUF (drive PAT on write, tri-state on read) + IDELAYE2(tap) ----
  wire [15:0] dq_in_raw, dq_in_dly;
  reg  [15:0] rdcap;
  genvar gi;
  generate for (gi=0; gi<16; gi=gi+1) begin: dqb
    IOBUF io(.IO(ddr2_dq[gi]), .I(PAT[gi]), .T(~dq_oe), .O(dq_in_raw[gi]));
    IDELAYE2 #(.IDELAY_TYPE("VAR_LOAD"), .DELAY_SRC("IDATAIN"), .REFCLK_FREQUENCY(200.0),
      .SIGNAL_PATTERN("DATA"), .HIGH_PERFORMANCE_MODE("TRUE"), .PIPE_SEL("FALSE"),
      .CINVCTRL_SEL("FALSE"), .IDELAY_VALUE(0))
    dl(.C(ddr_clk), .LD(tap_ld), .CE(1'b0), .INC(1'b0), .CINVCTRL(1'b0), .REGRST(1'b0),
      .LDPIPEEN(1'b0), .CNTVALUEIN(5'd16), .IDATAIN(dq_in_raw[gi]), .DATAIN(1'b0),
      .DATAOUT(dq_in_dly[gi]), .CNTVALUEOUT());
  end endgenerate
  always @(posedge ddr_clk90) rdcap <= ~dq_in_dly;  // read polarity correction: measured 1-bit DQ inversion
                                                     // (selfcap proved the pad reads ~driven on this IOB config)

  // ---- DQS: a real DDR-rate write strobe = the DDR clock routed to the pad, edge-aligned to CK
  // (tDQSS~=0), differential via ddr_clk/ddr_clk180. Tri-stated by ~dq_oe so it drives only in the
  // write window; on reads the DRAM owns DQS. Routing a clock net to the IOB I-pin sidesteps the
  // ODDR->IOB placement the open router can't do while still giving both-edge (DDR-rate) strobes. ----
  generate for (gi=0; gi<2; gi=gi+1) begin: dqslane
    IOBUF u_dqsp(.I(ddr_clk),    .T(~dq_oe), .IO(ddr2_dqs_p[gi]), .O());
    IOBUF u_dqsn(.I(ddr_clk180), .T(~dq_oe), .IO(ddr2_dqs_n[gi]), .O());
  end endgenerate
  assign ddr2_dm = 2'b00;

  // command mux: init FSM before init_done, data FSM after
  assign ddr2_cke  = init_done ? 1'b1  : i_cke;
  assign ddr2_cs_n = init_done ? c_cs  : i_cs;
  assign ddr2_ras_n= init_done ? c_ras : i_ras;
  assign ddr2_cas_n= init_done ? c_cas : i_cas;
  assign ddr2_we_n = init_done ? c_we  : i_we;
  assign ddr2_ba   = init_done ? c_ba  : {1'b0, i_ba};
  assign ddr2_addr = init_done ? c_a   : i_a;
  assign ddr2_odt  = init_done ? c_odt : 1'b0;

  // `tap` (0..31) is the CAPTURE-CYCLE offset after READ; IDELAY is fixed at mid (16).
  // Write latency: DDR2 latches write data WL = CL-1 cycles after the WRITE command (CL=5 => WL=4).
  // DQ is held at PAT across a wide data window so any captured read beat equals PAT; DQS is the
  // DDR-rate strobe over the same window. Spacing is generous (125 MHz => huge tRCD/tWR/tRP margins).
  reg [15:0] cap; reg match; reg [31:0] pass_mask; reg [31:0] nonidle_mask; reg [15:0] nicap;
  (* ram_style = "registers" *) reg [15:0] caparr [0:31];   // per-tap read capture, dumped over UART
  reg [15:0] selfcap;   // self-loopback: what the FPGA reads from its OWN driven pad during the write
  localparam [9:0] W_ACT=10'd1, W_WRITE=10'd5, WL=10'd4;
  localparam [9:0] WD_LO=W_WRITE+WL-10'd2, WD_HI=W_WRITE+WL+10'd5;   // drive DQ+DQS cmd_cnt 7..14
  localparam [9:0] W_PRE=10'd26, W_ACT2=10'd32, READ_CYC=10'd38, RUN_END=10'd72;
  always @(posedge ddr_clk) begin
    if (rst) begin
      phase<=P_WAIT; tap<=5'd0; tap_ld<=1'b0; cmd_cnt<=10'd0; dq_oe<=1'b0;
      c_cs<=1'b1; c_ras<=1'b1; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; c_odt<=1'b0;
      match<=1'b0; pass_mask<=32'd0; cap<=16'd0; nonidle_mask<=32'd0; nicap<=16'hffff; selfcap<=16'd0;
    end else begin
      tap_ld<=1'b0; dq_oe<=1'b0; c_cs<=1'b1; c_ras<=1'b1; c_cas<=1'b1; c_we<=1'b1; c_odt<=1'b0;
      case (phase)
        P_WAIT: if (init_done) begin tap<=5'd0; tap_ld<=1'b1; cmd_cnt<=10'd0; phase<=P_RUN; end
        P_RUN: begin
          cmd_cnt<=cmd_cnt+10'd1;
          if (cmd_cnt>=W_WRITE && cmd_cnt<=WD_HI+10'd1) c_odt<=1'b1;                 // ODT around the write
          if (cmd_cnt>=WD_LO && cmd_cnt<=WD_HI) dq_oe<=1'b1;                         // drive DQ(=PAT)+DQS at WL
          if (cmd_cnt==WD_LO+10'd5) selfcap<=rdcap;                                  // self-loopback: read own driven pad
          if (cmd_cnt==W_ACT) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; end       // ACT row0
          else if (cmd_cnt==W_WRITE) begin c_cs<=1'b0; c_ras<=1'b1; c_cas<=1'b0; c_we<=1'b0; c_ba<=3'd0; c_a<=13'd0; end // WRITE col0
          else if (cmd_cnt==W_PRE) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b0; c_a<=13'd0; end               // PRECHARGE
          else if (cmd_cnt==W_ACT2) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; end  // ACT row0 again
          else if (cmd_cnt==READ_CYC) begin c_cs<=1'b0; c_ras<=1'b1; c_cas<=1'b0; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; end// READ col0
          else if (cmd_cnt==READ_CYC+{5'd0,tap}) begin        // capture at READ + tap cycles
            cap<=rdcap; match<=(rdcap==PAT); caparr[tap]<=rdcap;
            if (rdcap==PAT) pass_mask[tap]<=1'b1;
            if (rdcap!=16'hffff) begin nonidle_mask[tap]<=1'b1; nicap<=rdcap; end
          end
          else if (cmd_cnt==RUN_END) phase<=P_NEXT;
        end
        P_NEXT: if (tap==5'd31) phase<=P_REPORT; else begin tap<=tap+5'd1; tap_ld<=1'b1; cmd_cnt<=10'd0; phase<=P_RUN; end
        P_REPORT: ;
      endcase
    end
  end
  assign led = |pass_mask;

  // dump {0xC7, selfcap_hi, selfcap_lo, cap[0][15:8], cap[0][7:0], ..., cap[31][...], 0x5A} = 68 bytes
  localparam CLKS=1085;
  reg [15:0] c; reg [2:0] tst; reg [2:0] bi; reg [6:0] pos; reg [15:0] gap; reg [7:0] by;
  localparam T_IDLE=3'd0, T_LOAD=3'd1, T_START=3'd2, T_DATA=3'd3, T_STOP=3'd4, T_GAP=3'd5;
  wire [4:0] dtap = (pos - 7'd3) >> 1;                 // pos 3..66 -> tap (pos-3)/2
  wire [7:0] outbyte = (pos==7'd0)  ? 8'hC7 :
                       (pos==7'd1)  ? selfcap[15:8] :
                       (pos==7'd2)  ? selfcap[7:0] :
                       (pos==7'd67) ? 8'h5A :
                       (pos[0])     ? caparr[dtap][15:8] : caparr[dtap][7:0];
  always @(posedge ddr_clk) begin
    if (rst) begin uart_tx<=1'b1; c<=0; tst<=T_IDLE; bi<=0; pos<=0; gap<=0; by<=8'hC7; end
    else case (tst)
      T_IDLE:  begin uart_tx<=1'b1; if (phase==P_REPORT) begin pos<=7'd0; tst<=T_LOAD; end end
      T_LOAD:  begin by<=outbyte; c<=0; tst<=T_START; end
      T_START: begin uart_tx<=1'b0; if (c==CLKS-1) begin c<=0; bi<=0; tst<=T_DATA; end else c<=c+1; end
      T_DATA:  begin uart_tx<=(by>>bi)&1'b1; if (c==CLKS-1) begin c<=0; if (bi==3'd7) tst<=T_STOP; else bi<=bi+1; end else c<=c+1; end
      T_STOP:  begin uart_tx<=1'b1; if (c==CLKS-1) begin c<=0; if (pos==7'd67) begin tst<=T_GAP; gap<=0; end else begin pos<=pos+1; tst<=T_LOAD; end end else c<=c+1; end
      T_GAP:   begin uart_tx<=1'b1; if (gap==16'd40000) begin gap<=0; tst<=T_IDLE; end else gap<=gap+1; end
      default: tst<=T_IDLE;
    endcase
  end
endmodule
