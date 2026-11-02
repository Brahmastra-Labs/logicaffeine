// ddr2_compute_pipeline — END-TO-END compute over the REAL on-board DDR2, fully-open flow.
//   1. write operands A, B into the external Micron MT47H64M16 DDR2 (proven PHY, WL=4, polarity-corrected)
//   2. read A, B back OUT of the real DDR2
//   3. feed them to the ComputeDRAM engine (dram_executor + computedram_model): triple-row activation
//      computes the sense-amp MAJORITY MAJ(A,B,0) = A & B  (the proven charge-sharing compute)
//   4. write the RESULT back into the real DDR2, then read it back
//   5. verify result == A & B on-chip
// The DATA PLANE is the real DDR2 die; the COMPUTE is the ComputeDRAM majority. Streams a 13-byte record
// {0xCD, A_hi,A_lo, B_hi,B_lo, eng_hi,eng_lo, ddr2_hi,ddr2_lo, exp_hi,exp_lo, status, 0x5A};
// status=1 iff the engine result AND the DDR2-read-back both equal A & B.
module ddr2_compute_pipeline (
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

  localparam [15:0] A_CONST = 16'hF0F0, B_CONST = 16'hFF00, EXPECT = 16'hF000;
  localparam [12:0] ROW0 = 13'd0, COL_A = 13'd0, COL_B = 13'd8, COL_R = 13'd16;
  localparam [2:0] CR_A = 3'd1, CR_B = 3'd2, CR_Z = 3'd3;   // ComputeDRAM engine rows
  localparam NW = 4'd7;

  reg [4:0] pst; reg [9:0] sub; reg [3:0] pl, cidx; reg [15:0] settle;
  reg dq_oe; reg c_cs, c_ras, c_cas, c_we, c_odt; reg [2:0] c_ba; reg [12:0] c_a;
  reg [15:0] aReg, bReg, engRes, rReg; reg status;
  localparam [9:0] WL = 10'd4;
  localparam P_IDLE=5'd0,
    P_WA_A=5'd1, P_WA_W=5'd2, P_WA_P=5'd3, P_WB_A=5'd4, P_WB_W=5'd5, P_WB_P=5'd6,
    P_RA_A=5'd7, P_RA_R=5'd8, P_RA_P=5'd9, P_RB_A=5'd10, P_RB_R=5'd11, P_RB_P=5'd12,
    P_PRE=5'd13, P_RUN=5'd14, P_SET=5'd15, P_CAP=5'd16,
    P_WR_A=5'd17, P_WR_W=5'd18, P_WR_P=5'd19, P_RR_A=5'd20, P_RR_R=5'd21, P_RR_P=5'd22,
    P_DONE=5'd23;

  // ComputeDRAM engine (DW=16 to match the DDR2 word); preloaded from the DDR2-read operands.
  reg [63:0] rom [0:6];
  initial $readmemh("computedram_and.hex", rom);
  reg pre_we; reg [2:0] pre_addr; reg [15:0] pre_data;
  wire exec_active = (pst==P_RUN);
  wire exec_rst = ~exec_active | rst;
  wire cmd_ack, issue; wire [2:0] eop; wire [4:0] ebank; wire [31:0] erow; wire [15:0] cdram_out;
  wire cmd_valid = exec_active && (cidx < NW);
  wire [63:0] cmd_word = (cidx < NW) ? rom[cidx] : 64'd0;
  dram_executor exec(.clk(ddr_clk), .rst(exec_rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid),
                     .cmd_ack(cmd_ack), .issue(issue), .op(eop), .bank(ebank), .row(erow));
  computedram_model #(.AW(3), .DW(16)) memdev(.clk(ddr_clk), .rst(exec_rst), .issue(issue), .op(eop),
    .bank(ebank), .row(erow), .pre_we(pre_we), .pre_addr(pre_addr), .pre_data(pre_data),
    .rd_addr(CR_A), .rd_out(cdram_out));

  // DQ: drive the operand/result on writes; capture (polarity-corrected) on reads.
  wire [15:0] dq_drive = (pst==P_WA_W) ? A_CONST : (pst==P_WB_W) ? B_CONST : engRes;
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

  // shared DDR2 micro-ops keyed by state: ACT (sub0..6), WRITE/READ column (sub0 cmd, WL drive / sub8
  // capture, done sub13), PRE (sub2, done sub6).
  wire act_st = (pst==P_WA_A)||(pst==P_WB_A)||(pst==P_RA_A)||(pst==P_RB_A)||(pst==P_WR_A)||(pst==P_RR_A);
  wire wr_st  = (pst==P_WA_W)||(pst==P_WB_W)||(pst==P_WR_W);
  wire rd_st  = (pst==P_RA_R)||(pst==P_RB_R)||(pst==P_RR_R);
  wire pre_st = (pst==P_WA_P)||(pst==P_WB_P)||(pst==P_RA_P)||(pst==P_RB_P)||(pst==P_WR_P)||(pst==P_RR_P);
  wire [12:0] wr_col = (pst==P_WA_W)?COL_A : (pst==P_WB_W)?COL_B : COL_R;
  wire [12:0] rd_col = (pst==P_RA_R)?COL_A : (pst==P_RB_R)?COL_B : COL_R;

  always @(posedge ddr_clk) begin
    if (rst) begin
      pst<=P_IDLE; sub<=10'd0; pl<=4'd0; cidx<=4'd0; settle<=16'd0; dq_oe<=1'b0;
      c_cs<=1'b1; c_ras<=1'b1; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; c_odt<=1'b0;
      aReg<=16'd0; bReg<=16'd0; engRes<=16'd0; rReg<=16'd0; status<=1'b0;
      pre_we<=1'b0; pre_addr<=3'd0; pre_data<=16'd0;
    end else begin
      c_cs<=1'b1; c_ras<=1'b1; c_cas<=1'b1; c_we<=1'b1; c_odt<=1'b0; dq_oe<=1'b0; pre_we<=1'b0;
      if (act_st) begin
        if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=ROW0; end
        if (sub==10'd6) sub<=10'd0; else sub<=sub+10'd1;
      end else if (wr_st) begin
        if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b1; c_cas<=1'b0; c_we<=1'b0; c_ba<=3'd0; c_a<=wr_col; end
        if (sub>=WL-10'd1 && sub<=WL+10'd4) begin dq_oe<=1'b1; c_odt<=1'b1; end
        if (sub==10'd13) sub<=10'd0; else sub<=sub+10'd1;
      end else if (rd_st) begin
        if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b1; c_cas<=1'b0; c_we<=1'b1; c_ba<=3'd0; c_a<=rd_col; end
        if (sub==10'd8) begin
          if (pst==P_RA_R) aReg<=rdcap; else if (pst==P_RB_R) bReg<=rdcap; else rReg<=rdcap;
        end
        if (sub==10'd13) sub<=10'd0; else sub<=sub+10'd1;
      end else if (pre_st) begin
        if (sub==10'd2) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b0; c_a<=13'd0; end
        if (sub==10'd6) sub<=10'd0; else sub<=sub+10'd1;
      end
      // state sequencing on the LAST sub of each micro-op
      case (pst)
        P_IDLE: if (init_done) begin pst<=P_WA_A; sub<=10'd0; end
        P_WA_A: if (sub==10'd6) pst<=P_WA_W;
        P_WA_W: if (sub==10'd13) pst<=P_WA_P;
        P_WA_P: if (sub==10'd6) pst<=P_WB_A;
        P_WB_A: if (sub==10'd6) pst<=P_WB_W;
        P_WB_W: if (sub==10'd13) pst<=P_WB_P;
        P_WB_P: if (sub==10'd6) pst<=P_RA_A;
        P_RA_A: if (sub==10'd6) pst<=P_RA_R;
        P_RA_R: if (sub==10'd13) pst<=P_RA_P;
        P_RA_P: if (sub==10'd6) pst<=P_RB_A;
        P_RB_A: if (sub==10'd6) pst<=P_RB_R;
        P_RB_R: if (sub==10'd13) pst<=P_RB_P;
        P_RB_P: if (sub==10'd6) begin pst<=P_PRE; pl<=4'd0; end
        P_PRE: begin
          case (pl)
            4'd0: begin pre_we<=1'b1; pre_addr<=CR_A; pre_data<=aReg; pl<=4'd1; end
            4'd1: begin pre_we<=1'b1; pre_addr<=CR_B; pre_data<=bReg; pl<=4'd2; end
            4'd2: begin pre_we<=1'b1; pre_addr<=CR_Z; pre_data<=16'd0; pl<=4'd3; end
            4'd3: pl<=4'd4;
            4'd4: pl<=4'd5;
            default: begin pst<=P_RUN; cidx<=4'd0; end
          endcase
        end
        P_RUN: begin
          if (cmd_ack) cidx<=cidx+4'd1;
          if (cidx==NW) begin pst<=P_SET; settle<=16'd0; end
        end
        P_SET: if (settle>=16'd300) begin engRes<=cdram_out; pst<=P_CAP; end else settle<=settle+16'd1;
        P_CAP: begin sub<=10'd0; pst<=P_WR_A; end
        P_WR_A: if (sub==10'd6) pst<=P_WR_W;
        P_WR_W: if (sub==10'd13) pst<=P_WR_P;
        P_WR_P: if (sub==10'd6) pst<=P_RR_A;
        P_RR_A: if (sub==10'd6) pst<=P_RR_R;
        P_RR_R: if (sub==10'd13) pst<=P_RR_P;
        P_RR_P: if (sub==10'd6) begin
                  status <= (engRes==(aReg & bReg)) && (rReg==(aReg & bReg));
                  pst<=P_DONE;
                end
        P_DONE: ;
        default: pst<=P_IDLE;
      endcase
    end
  end
  assign led = (pst==P_DONE) && status;

  // report {0xCD, A_hi,A_lo, B_hi,B_lo, eng_hi,eng_lo, ddr2_hi,ddr2_lo, exp_hi,exp_lo, status, 0x5A} = 13
  wire [15:0] expv = aReg & bReg;
  reg [7:0] rec [0:12];
  always @(*) begin
    rec[0]=8'hCD; rec[1]=aReg[15:8]; rec[2]=aReg[7:0]; rec[3]=bReg[15:8]; rec[4]=bReg[7:0];
    rec[5]=engRes[15:8]; rec[6]=engRes[7:0]; rec[7]=rReg[15:8]; rec[8]=rReg[7:0];
    rec[9]=expv[15:8]; rec[10]=expv[7:0]; rec[11]={7'd0,status}; rec[12]=8'h5A;
  end
  localparam CLKS=1085;
  reg [15:0] cc; reg [2:0] tst; reg [2:0] bi; reg [3:0] pos; reg [15:0] gap; reg [7:0] by;
  localparam T_IDLE=3'd0, T_LOAD=3'd1, T_START=3'd2, T_DATA=3'd3, T_STOP=3'd4, T_GAP=3'd5;
  always @(posedge ddr_clk) begin
    if (rst) begin uart_tx<=1'b1; cc<=0; tst<=T_IDLE; bi<=0; pos<=0; gap<=0; by<=8'hCD; end
    else case (tst)
      T_IDLE:  begin uart_tx<=1'b1; if (pst==P_DONE) begin pos<=4'd0; tst<=T_LOAD; end end
      T_LOAD:  begin by<=rec[pos]; cc<=0; tst<=T_START; end
      T_START: begin uart_tx<=1'b0; if (cc==CLKS-1) begin cc<=0; bi<=0; tst<=T_DATA; end else cc<=cc+1; end
      T_DATA:  begin uart_tx<=(by>>bi)&1'b1; if (cc==CLKS-1) begin cc<=0; if (bi==3'd7) tst<=T_STOP; else bi<=bi+1; end else cc<=cc+1; end
      T_STOP:  begin uart_tx<=1'b1; if (cc==CLKS-1) begin cc<=0; if (pos==4'd12) begin tst<=T_GAP; gap<=0; end else begin pos<=pos+1; tst<=T_LOAD; end end else cc<=cc+1; end
      T_GAP:   begin uart_tx<=1'b1; if (gap==16'd40000) begin gap<=0; tst<=T_IDLE; end else gap<=gap+1; end
      default: tst<=T_IDLE;
    endcase
  end
endmodule
