// ddr2_memtest — a real write/read memtest on the on-board Micron MT47H64M16 DDR2, fully-open flow.
// It writes an address-in-address pattern {col,~col} to NCOL BL8-aligned columns of bank0/row0, then
// re-opens the row and reads every column back, comparing on-chip. It reports an 11-byte record
// {0xC9, npass_lo, npass_hi, ntest_lo, ntest_hi, ff_col, ff_got_hi, ff_got_lo, ff_exp_hi, ff_exp_lo, 0x5A}.
// npass==ntest and led on => the DDR2 stores and returns data across many addresses (Phase 1e gate).
// Reuses the proven Stage-2c PHY: 125 MHz DDR clock, WL=4 write, DDR-rate DQS strobe, and the measured
// 1-bit DQ read-polarity correction (rdcap = ~dq_in_dly).
module ddr2_memtest (
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

  wire i_cke, i_cs, i_ras, i_cas, i_we, init_done; wire [1:0] i_ba; wire [12:0] i_a;
  ddr2_init_fsm u_init(.clk(ddr_clk), .rst(rst), .cke(i_cke), .cs_n(i_cs), .ras_n(i_ras),
    .cas_n(i_cas), .we_n(i_we), .ba(i_ba), .a(i_a), .init_done(init_done));

  localparam NCOL = 5'd16;
  function [15:0] patof(input [4:0] i); reg [9:0] col; begin
    col = {i, 3'd0};                    // BL8-aligned column = i*8
    patof = {col[7:0], ~col[7:0]};      // address-in-address
  end endfunction

  reg [2:0] st; reg [4:0] idx; reg [9:0] sub;
  reg dq_oe; reg c_cs, c_ras, c_cas, c_we, c_odt; reg [2:0] c_ba; reg [12:0] c_a;
  reg [15:0] npass, ntest; reg [9:0] ffcol; reg [15:0] ffgot, ffexp; reg failed;
  localparam [9:0] WL = 10'd4;
  localparam S_IDLE=3'd0, S_ACTW=3'd1, S_WRITE=3'd2, S_PRE=3'd3, S_ACTR=3'd4, S_READ=3'd5, S_DONE=3'd6;

  // DQ: drive the current column's pattern on writes; capture (polarity-corrected) on reads.
  wire [15:0] dq_drive = patof(idx);
  wire [15:0] dq_in_raw, dq_in_dly; reg [15:0] rdcap;
  genvar gi;
  generate for (gi=0; gi<16; gi=gi+1) begin: dqb
    IOBUF io(.IO(ddr2_dq[gi]), .I(dq_drive[gi]), .T(~dq_oe), .O(dq_in_raw[gi]));
    IDELAYE2 #(.IDELAY_TYPE("FIXED"), .DELAY_SRC("IDATAIN"), .REFCLK_FREQUENCY(200.0),
      .SIGNAL_PATTERN("DATA"), .HIGH_PERFORMANCE_MODE("TRUE"), .PIPE_SEL("FALSE"),
      .CINVCTRL_SEL("FALSE"), .IDELAY_VALUE(16))
    dl(.C(ddr_clk), .LD(1'b0), .CE(1'b0), .INC(1'b0), .CINVCTRL(1'b0), .REGRST(1'b0),
      .LDPIPEEN(1'b0), .CNTVALUEIN(5'd16), .IDATAIN(dq_in_raw[gi]), .DATAIN(1'b0),
      .DATAOUT(dq_in_dly[gi]), .CNTVALUEOUT());
  end endgenerate
  always @(posedge ddr_clk90) rdcap <= ~dq_in_dly;   // measured 1-bit DQ read-polarity correction

  // DQS: DDR-rate strobe (ddr_clk / ddr_clk180) tri-stated to the write window.
  generate for (gi=0; gi<2; gi=gi+1) begin: dqslane
    IOBUF u_dqsp(.I(ddr_clk),    .T(~dq_oe), .IO(ddr2_dqs_p[gi]), .O());
    IOBUF u_dqsn(.I(ddr_clk180), .T(~dq_oe), .IO(ddr2_dqs_n[gi]), .O());
  end endgenerate
  assign ddr2_dm = 2'b00;

  assign ddr2_cke  = init_done ? 1'b1  : i_cke;
  assign ddr2_cs_n = init_done ? c_cs  : i_cs;
  assign ddr2_ras_n= init_done ? c_ras : i_ras;
  assign ddr2_cas_n= init_done ? c_cas : i_cas;
  assign ddr2_we_n = init_done ? c_we  : i_we;
  assign ddr2_ba   = init_done ? c_ba  : {1'b0, i_ba};
  assign ddr2_addr = init_done ? c_a   : i_a;
  assign ddr2_odt  = init_done ? c_odt : 1'b0;

  always @(posedge ddr_clk) begin
    if (rst) begin
      st<=S_IDLE; idx<=5'd0; sub<=10'd0; dq_oe<=1'b0;
      c_cs<=1'b1; c_ras<=1'b1; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; c_odt<=1'b0;
      npass<=16'd0; ntest<=16'd0; ffcol<=10'd0; ffgot<=16'd0; ffexp<=16'd0; failed<=1'b0;
    end else begin
      c_cs<=1'b1; c_ras<=1'b1; c_cas<=1'b1; c_we<=1'b1; c_odt<=1'b0; dq_oe<=1'b0;
      case (st)
        S_IDLE: if (init_done) begin st<=S_ACTW; sub<=10'd0; end
        S_ACTW: begin
          if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; end // ACT row0
          if (sub==10'd6) begin st<=S_WRITE; idx<=5'd0; sub<=10'd0; end else sub<=sub+10'd1;
        end
        S_WRITE: begin
          if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b1; c_cas<=1'b0; c_we<=1'b0; c_ba<=3'd0; c_a<={5'd0, idx, 3'd0}; end // WRITE col=idx*8
          if (sub>=WL-10'd1 && sub<=WL+10'd4) begin dq_oe<=1'b1; c_odt<=1'b1; end                                        // drive DQ+DQS at WL
          if (sub==10'd13) begin sub<=10'd0; if (idx==NCOL-5'd1) st<=S_PRE; else idx<=idx+5'd1; end else sub<=sub+10'd1;
        end
        S_PRE: begin
          if (sub==10'd2) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b0; c_a<=13'd0; end  // PRECHARGE bank0
          if (sub==10'd6) begin st<=S_ACTR; sub<=10'd0; end else sub<=sub+10'd1;
        end
        S_ACTR: begin
          if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; end // ACT row0 again
          if (sub==10'd6) begin st<=S_READ; idx<=5'd0; sub<=10'd0; end else sub<=sub+10'd1;
        end
        S_READ: begin
          if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b1; c_cas<=1'b0; c_we<=1'b1; c_ba<=3'd0; c_a<={5'd0, idx, 3'd0}; end // READ col=idx*8
          if (sub==10'd8) begin
            ntest<=ntest+16'd1;
            if (rdcap==patof(idx)) npass<=npass+16'd1;
            else if (!failed) begin failed<=1'b1; ffcol<={5'd0,idx,3'd0}; ffgot<=rdcap; ffexp<=patof(idx); end
          end
          if (sub==10'd13) begin sub<=10'd0; if (idx==NCOL-5'd1) st<=S_DONE; else idx<=idx+5'd1; end else sub<=sub+10'd1;
        end
        S_DONE: ;
      endcase
    end
  end
  assign led = (ntest!=16'd0) && (npass==ntest);

  // UART: stream {0xC9, npass_lo, npass_hi, ntest_lo, ntest_hi, ffcol[7:0], ffgot_hi, ffgot_lo,
  // ffexp_hi, ffexp_lo, 0x5A} = 11 bytes, repeated.
  reg [7:0] rec [0:10];
  always @(*) begin
    rec[0]=8'hC9; rec[1]=npass[7:0]; rec[2]=npass[15:8]; rec[3]=ntest[7:0]; rec[4]=ntest[15:8];
    rec[5]=ffcol[7:0]; rec[6]=ffgot[15:8]; rec[7]=ffgot[7:0]; rec[8]=ffexp[15:8]; rec[9]=ffexp[7:0]; rec[10]=8'h5A;
  end
  localparam CLKS=1085;
  reg [15:0] c; reg [2:0] tst; reg [2:0] bi; reg [3:0] pos; reg [15:0] gap; reg [7:0] by;
  localparam T_IDLE=3'd0, T_LOAD=3'd1, T_START=3'd2, T_DATA=3'd3, T_STOP=3'd4, T_GAP=3'd5;
  always @(posedge ddr_clk) begin
    if (rst) begin uart_tx<=1'b1; c<=0; tst<=T_IDLE; bi<=0; pos<=0; gap<=0; by<=8'hC9; end
    else case (tst)
      T_IDLE:  begin uart_tx<=1'b1; if (st==S_DONE) begin pos<=4'd0; tst<=T_LOAD; end end
      T_LOAD:  begin by<=rec[pos]; c<=0; tst<=T_START; end
      T_START: begin uart_tx<=1'b0; if (c==CLKS-1) begin c<=0; bi<=0; tst<=T_DATA; end else c<=c+1; end
      T_DATA:  begin uart_tx<=(by>>bi)&1'b1; if (c==CLKS-1) begin c<=0; if (bi==3'd7) tst<=T_STOP; else bi<=bi+1; end else c<=c+1; end
      T_STOP:  begin uart_tx<=1'b1; if (c==CLKS-1) begin c<=0; if (pos==4'd10) begin tst<=T_GAP; gap<=0; end else begin pos<=pos+1; tst<=T_LOAD; end end else c<=c+1; end
      T_GAP:   begin uart_tx<=1'b1; if (gap==16'd40000) begin gap<=0; tst<=T_IDLE; end else gap<=gap+1; end
      default: tst<=T_IDLE;
    endcase
  end
endmodule
