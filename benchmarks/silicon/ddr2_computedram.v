// ddr2_computedram — attempt ComputeDRAM RowClone (in-DRAM row copy) on the REAL on-board Micron
// MT47H64M16 DDR2, fully-open flow. For each (T1,T2) in a small sweep it: (1) writes an
// address-in-address pattern PAT_A to bank0/ROW_SRC and its complement PAT_B to bank0/ROW_DST via the
// proven PHY; (2) issues the sub-JEDEC stream ACT(SRC) -T1- PRE -T2- ACT(DST) (T1<tRAS, T2<tRP) that
// ComputeDRAM (Gao et al., MICRO 2019) uses to make the sense amps copy SRC's data into DST; (3) reads
// ROW_DST back and counts how many of NCOL columns now equal PAT_A (copied) instead of PAT_B (unchanged).
// It streams a 30-byte record: {0xCB, T1LO,T1HI,T2LO,T2HI, grid[0..17], lastdst0_hi,lastdst0_lo,
// patA0_hi,patA0_lo, patB0_hi,patB0_lo, 0x5A}. grid[i]=copied-count for trial i. Any grid[i]>0 => this
// DDR2 exhibits in-DRAM copy at that timing. HONEST: a null grid is a valid result (the paper used DDR3).
module ddr2_computedram (
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

  localparam [4:0] NCOL = 5'd4;
  localparam [12:0] ROW_SRC = 13'd0, ROW_DST = 13'd1;
  localparam [3:0] T1LO=4'd1, T1HI=4'd6, T2LO=4'd1, T2HI=4'd3;   // 6*3 = 18 trials
  function [15:0] patA(input [4:0] i); reg [9:0] col; begin col={i,3'd0}; patA={col[7:0], ~col[7:0]}; end endfunction
  function [15:0] patB(input [4:0] i); begin patB = ~patA(i); end endfunction
  wire [15:0] w_patA0 = patA(5'd0);
  wire [15:0] w_patB0 = patB(5'd0);

  reg [3:0] st; reg [4:0] idx; reg [9:0] sub; reg [3:0] t1, t2; reg [4:0] trial; reg [4:0] copied;
  reg dq_oe; reg c_cs, c_ras, c_cas, c_we, c_odt; reg [2:0] c_ba; reg [12:0] c_a;
  reg [7:0] grid [0:17]; reg [15:0] lastdst0;
  localparam [9:0] WL = 10'd4;
  localparam S_IDLE=4'd0, S_WSA=4'd1, S_WS=4'd2, S_WSP=4'd3, S_WDA=4'd4, S_WD=4'd5, S_WDP=4'd6,
             S_CLONE=4'd7, S_RDA=4'd8, S_RD=4'd9, S_RDP=4'd10, S_STORE=4'd11, S_REPORT=4'd12;

  wire [15:0] dq_drive = (st==S_WD) ? patB(idx) : patA(idx);
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

  integer ci;
  always @(posedge ddr_clk) begin
    if (rst) begin
      st<=S_IDLE; idx<=5'd0; sub<=10'd0; t1<=T1LO; t2<=T2LO; trial<=5'd0; copied<=5'd0;
      dq_oe<=1'b0; c_cs<=1'b1; c_ras<=1'b1; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; c_odt<=1'b0;
      lastdst0<=16'd0;
      for (ci=0; ci<18; ci=ci+1) grid[ci] <= 8'd0;
    end else begin
      c_cs<=1'b1; c_ras<=1'b1; c_cas<=1'b1; c_we<=1'b1; c_odt<=1'b0; dq_oe<=1'b0;
      case (st)
        S_IDLE: if (init_done) begin st<=S_WSA; sub<=10'd0; end
        // ---- write PAT_A to ROW_SRC ----
        S_WSA: begin
          if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=ROW_SRC; end
          if (sub==10'd6) begin st<=S_WS; idx<=5'd0; sub<=10'd0; end else sub<=sub+10'd1;
        end
        S_WS: begin
          if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b1; c_cas<=1'b0; c_we<=1'b0; c_ba<=3'd0; c_a<={5'd0, idx, 3'd0}; end
          if (sub>=WL-10'd1 && sub<=WL+10'd4) begin dq_oe<=1'b1; c_odt<=1'b1; end
          if (sub==10'd13) begin sub<=10'd0; if (idx==NCOL-5'd1) st<=S_WSP; else idx<=idx+5'd1; end else sub<=sub+10'd1;
        end
        S_WSP: begin
          if (sub==10'd2) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b0; c_a<=13'd0; end
          if (sub==10'd6) begin st<=S_WDA; sub<=10'd0; end else sub<=sub+10'd1;
        end
        // ---- write PAT_B to ROW_DST ----
        S_WDA: begin
          if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=ROW_DST; end
          if (sub==10'd6) begin st<=S_WD; idx<=5'd0; sub<=10'd0; end else sub<=sub+10'd1;
        end
        S_WD: begin
          if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b1; c_cas<=1'b0; c_we<=1'b0; c_ba<=3'd0; c_a<={5'd0, idx, 3'd0}; end
          if (sub>=WL-10'd1 && sub<=WL+10'd4) begin dq_oe<=1'b1; c_odt<=1'b1; end
          if (sub==10'd13) begin sub<=10'd0; if (idx==NCOL-5'd1) st<=S_WDP; else idx<=idx+5'd1; end else sub<=sub+10'd1;
        end
        S_WDP: begin
          if (sub==10'd2) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b0; c_a<=13'd0; end
          if (sub==10'd6) begin st<=S_CLONE; sub<=10'd0; end else sub<=sub+10'd1;
        end
        // ---- the sub-JEDEC RowClone stream: ACT(SRC) -T1- PRE -T2- ACT(DST) -PRE ----
        S_CLONE: begin
          if (sub==10'd0)                       begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=ROW_SRC; end // ACT SRC
          if (sub=={6'd0,t1})                   begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b0; c_a<=13'd0; end               // PRE (T1 after ACT)
          if (sub=={6'd0,t1}+{6'd0,t2})         begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=ROW_DST; end // ACT DST (T2 after PRE)
          if (sub=={6'd0,t1}+{6'd0,t2}+10'd6)   begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b0; c_a<=13'd0; end               // PRE (close DST)
          if (sub=={6'd0,t1}+{6'd0,t2}+10'd12)  begin st<=S_RDA; sub<=10'd0; end else sub<=sub+10'd1;
        end
        // ---- read ROW_DST back, count copied ----
        S_RDA: begin
          if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=ROW_DST; end
          if (sub==10'd6) begin st<=S_RD; idx<=5'd0; sub<=10'd0; copied<=5'd0; end else sub<=sub+10'd1;
        end
        S_RD: begin
          if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b1; c_cas<=1'b0; c_we<=1'b1; c_ba<=3'd0; c_a<={5'd0, idx, 3'd0}; end
          if (sub==10'd8) begin
            if (idx==5'd0) lastdst0<=rdcap;
            if (rdcap==patA(idx)) copied<=copied+5'd1;
          end
          if (sub==10'd13) begin sub<=10'd0; if (idx==NCOL-5'd1) st<=S_RDP; else idx<=idx+5'd1; end else sub<=sub+10'd1;
        end
        S_RDP: begin
          if (sub==10'd2) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b0; c_a<=13'd0; end
          if (sub==10'd6) begin st<=S_STORE; sub<=10'd0; end else sub<=sub+10'd1;
        end
        S_STORE: begin
          grid[trial] <= {3'd0, copied};
          trial <= trial + 5'd1;
          if (t2==T2HI) begin
            t2 <= T2LO;
            if (t1==T1HI) st<=S_REPORT; else begin t1<=t1+4'd1; st<=S_WSA; sub<=10'd0; end
          end else begin t2<=t2+4'd1; st<=S_WSA; sub<=10'd0; end
        end
        S_REPORT: ;
        default: st<=S_IDLE;
      endcase
    end
  end
  assign led = (st==S_REPORT);

  // report {0xCB, T1LO,T1HI,T2LO,T2HI, grid[0..17], lastdst0_hi,lastdst0_lo, patA0_hi,patA0_lo,
  //          patB0_hi,patB0_lo, 0x5A} = 30 bytes
  reg [7:0] rec [0:29];
  integer k;
  always @(*) begin
    rec[0]=8'hCB; rec[1]={4'd0,T1LO}; rec[2]={4'd0,T1HI}; rec[3]={4'd0,T2LO}; rec[4]={4'd0,T2HI};
    for (k=0;k<18;k=k+1) rec[5+k]=grid[k];
    rec[23]=lastdst0[15:8]; rec[24]=lastdst0[7:0];
    rec[25]=w_patA0[15:8]; rec[26]=w_patA0[7:0];
    rec[27]=w_patB0[15:8]; rec[28]=w_patB0[7:0]; rec[29]=8'h5A;
  end
  localparam CLKS=1085;
  reg [15:0] cc; reg [2:0] tst; reg [2:0] bi; reg [5:0] pos; reg [15:0] gap; reg [7:0] by;
  localparam T_IDLE=3'd0, T_LOAD=3'd1, T_START=3'd2, T_DATA=3'd3, T_STOP=3'd4, T_GAP=3'd5;
  always @(posedge ddr_clk) begin
    if (rst) begin uart_tx<=1'b1; cc<=0; tst<=T_IDLE; bi<=0; pos<=0; gap<=0; by<=8'hCB; end
    else case (tst)
      T_IDLE:  begin uart_tx<=1'b1; if (st==S_REPORT) begin pos<=6'd0; tst<=T_LOAD; end end
      T_LOAD:  begin by<=rec[pos]; cc<=0; tst<=T_START; end
      T_START: begin uart_tx<=1'b0; if (cc==CLKS-1) begin cc<=0; bi<=0; tst<=T_DATA; end else cc<=cc+1; end
      T_DATA:  begin uart_tx<=(by>>bi)&1'b1; if (cc==CLKS-1) begin cc<=0; if (bi==3'd7) tst<=T_STOP; else bi<=bi+1; end else cc<=cc+1; end
      T_STOP:  begin uart_tx<=1'b1; if (cc==CLKS-1) begin cc<=0; if (pos==6'd29) begin tst<=T_GAP; gap<=0; end else begin pos<=pos+1; tst<=T_LOAD; end end else cc<=cc+1; end
      T_GAP:   begin uart_tx<=1'b1; if (gap==16'd40000) begin gap<=0; tst<=T_IDLE; end else gap<=gap+1; end
      default: tst<=T_IDLE;
    endcase
  end
endmodule
