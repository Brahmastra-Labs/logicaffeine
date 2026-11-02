// ddr2_compute_pipeline64 — END-TO-END 64-bit compute over the REAL on-board DDR2, three ops.
// For op in {AND, OR, MAJ}:
//   1. write 64-bit operands A,B (and C for MAJ) into the external Micron MT47H64M16 DDR2 as four
//      16-bit words each (proven PHY: WL=4, DDR-rate DQS, 1-bit read-polarity correction)
//   2. read A,B,C back OUT of the real DDR2
//   3. preload the ComputeDRAM engine: CR_A=A, CR_B=B, CR_Z = 0 (AND) | all-1s (OR) | C (MAJ); the
//      triple-row activation computes the sense-amp MAJORITY maj3(A,B,Z) = A&B / A|B / maj(A,B,C)
//   4. write the 64-bit RESULT back into the real DDR2, then read it back
//   5. verify engine result AND DDR2 read-back == the software oracle, on-chip
// Streams {0xCE, A(8), B(8), C(8), AND_ddr2(8), OR_ddr2(8), MAJ_ddr2(8), status, 0x5A} = 51 bytes,
// MSB-first; status bit0=AND ok, bit1=OR ok, bit2=MAJ ok (each = engine==oracle && ddr2==oracle).
module ddr2_compute_pipeline64 (
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

  localparam [63:0] A_CONST = 64'hF0F0F0F0F0F0F0F0;
  localparam [63:0] B_CONST = 64'hFF00FF00FF00FF00;
  localparam [63:0] C_CONST = 64'h00FFFF0000FFFF00;
  localparam [12:0] ROW0 = 13'd0;
  localparam [12:0] COL_A = 13'd0, COL_B = 13'd32, COL_C = 13'd64, COL_R = 13'd96;
  localparam [2:0] CR_A = 3'd1, CR_B = 3'd2, CR_Z = 3'd3;
  localparam [3:0] NW = 4'd7;

  reg dq_oe; reg c_cs, c_ras, c_cas, c_we, c_odt; reg [2:0] c_ba; reg [12:0] c_a;
  localparam [9:0] WL = 10'd4;

  // ---- DDR2 access sub-FSM: read/write a 64-bit value = ACT + 4 columns + PRE ----
  localparam A_IDLE=3'd0, A_ACT=3'd1, A_OP=3'd2, A_PRE=3'd3;
  reg [2:0] acc_st; reg acc_start, acc_mode; reg [12:0] acc_base; reg [63:0] acc_wdata, acc_rdata;
  reg acc_busy; reg [2:0] wi; reg [9:0] sub;
  wire [15:0] wword = acc_wdata[wi*16 +: 16];
  wire [12:0] acc_col = acc_base + {7'd0, wi, 3'd0};

  wire [15:0] dq_drive = wword;
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
  always @(posedge ddr_clk90) rdcap <= ~dq_in_dly;

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
      acc_st<=A_IDLE; acc_busy<=1'b0; wi<=3'd0; sub<=10'd0; acc_rdata<=64'd0; dq_oe<=1'b0;
      c_cs<=1'b1; c_ras<=1'b1; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; c_odt<=1'b0;
    end else begin
      c_cs<=1'b1; c_ras<=1'b1; c_cas<=1'b1; c_we<=1'b1; c_odt<=1'b0; dq_oe<=1'b0;
      case (acc_st)
        A_IDLE: if (acc_start) begin acc_busy<=1'b1; acc_st<=A_ACT; sub<=10'd0; end
        A_ACT: begin
          if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=ROW0; end
          if (sub==10'd6) begin acc_st<=A_OP; wi<=3'd0; sub<=10'd0; end else sub<=sub+10'd1;
        end
        A_OP: begin
          if (sub==10'd0) begin c_cs<=1'b0; c_ras<=1'b1; c_cas<=1'b0; c_we<=~acc_mode; c_ba<=3'd0; c_a<=acc_col; end
          if (acc_mode && sub>=WL-10'd1 && sub<=WL+10'd4) begin dq_oe<=1'b1; c_odt<=1'b1; end
          if (!acc_mode && sub==10'd8) acc_rdata[wi*16 +: 16] <= rdcap;
          if (sub==10'd13) begin sub<=10'd0; if (wi==3'd3) acc_st<=A_PRE; else wi<=wi+3'd1; end else sub<=sub+10'd1;
        end
        A_PRE: begin
          if (sub==10'd2) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b0; c_a<=13'd0; end
          if (sub==10'd6) begin acc_st<=A_IDLE; acc_busy<=1'b0; end else sub<=sub+10'd1;
        end
      endcase
    end
  end

  // ---- ComputeDRAM engine (DW=64), re-preloaded per op ----
  reg [63:0] rom [0:6];
  initial $readmemh("computedram_and.hex", rom);
  reg pre_we; reg [2:0] pre_addr; reg [63:0] pre_data;
  reg run_en; reg [3:0] cidx;
  wire exec_rst = ~run_en | rst;
  wire cmd_ack, issue; wire [2:0] eop; wire [4:0] ebank; wire [31:0] erow; wire [63:0] cdram_out;
  wire cmd_valid = run_en && (cidx < NW);
  wire [63:0] cmd_word = (cidx < NW) ? rom[cidx] : 64'd0;
  dram_executor exec(.clk(ddr_clk), .rst(exec_rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid),
                     .cmd_ack(cmd_ack), .issue(issue), .op(eop), .bank(ebank), .row(erow));
  computedram_model #(.AW(3), .DW(64)) memdev(.clk(ddr_clk), .rst(exec_rst), .issue(issue), .op(eop),
    .bank(ebank), .row(erow), .pre_we(pre_we), .pre_addr(pre_addr), .pre_data(pre_data),
    .rd_addr(CR_A), .rd_out(cdram_out));

  // ---- orchestration FSM ----
  localparam M_IDLE=5'd0, M_S1=5'd1, M_S2=5'd2, M_S3=5'd3, M_S4=5'd4, M_S5=5'd5, M_S6=5'd6,
    M_PRE=5'd7, M_RUN=5'd8, M_SET=5'd9, M_WRES=5'd10, M_RRES=5'd11, M_STORE=5'd12,
    M_DONE=5'd13, W_START=5'd14, W_DONE=5'd15;
  localparam RT_NONE=2'd0, RT_A=2'd1, RT_B=2'd2, RT_C=2'd3;   // read-target tag (RES handled via opn)
  reg [4:0] pst, ret; reg [1:0] rdtag; reg readingRes;
  reg [63:0] aReg, bReg, cReg, engRes, rRes;
  reg [63:0] andEng, orEng, majEng, andD, orD, majD;
  reg [1:0] opn; reg [3:0] pl; reg [15:0] settle; reg [2:0] status;

  function [63:0] maj3f(input [63:0] a, input [63:0] b, input [63:0] c);
    maj3f = (a & b) | (a & c) | (b & c);
  endfunction
  wire [63:0] oracle = (opn==2'd0) ? (aReg & bReg) : (opn==2'd1) ? (aReg | bReg) : maj3f(aReg,bReg,cReg);
  wire [63:0] zrow   = (opn==2'd0) ? 64'd0 : (opn==2'd1) ? {64{1'b1}} : cReg;

  always @(posedge ddr_clk) begin
    if (rst) begin
      pst<=M_IDLE; ret<=M_IDLE; rdtag<=RT_NONE; readingRes<=1'b0;
      aReg<=0; bReg<=0; cReg<=0; engRes<=0; rRes<=0;
      andEng<=0; orEng<=0; majEng<=0; andD<=0; orD<=0; majD<=0;
      opn<=2'd0; pl<=4'd0; settle<=16'd0; status<=3'd0;
      acc_start<=1'b0; acc_mode<=1'b0; acc_base<=13'd0; acc_wdata<=64'd0;
      pre_we<=1'b0; pre_addr<=3'd0; pre_data<=64'd0; run_en<=1'b0; cidx<=4'd0;
    end else begin
      acc_start<=1'b0; pre_we<=1'b0;
      case (pst)
        M_IDLE: if (init_done) begin
                  acc_mode<=1'b1; acc_base<=COL_A; acc_wdata<=A_CONST; acc_start<=1'b1; ret<=M_S1; pst<=W_START;
                end
        M_S1: begin acc_mode<=1'b1; acc_base<=COL_B; acc_wdata<=B_CONST; acc_start<=1'b1; ret<=M_S2; pst<=W_START; end
        M_S2: begin acc_mode<=1'b1; acc_base<=COL_C; acc_wdata<=C_CONST; acc_start<=1'b1; ret<=M_S3; pst<=W_START; end
        M_S3: begin acc_mode<=1'b0; acc_base<=COL_A; rdtag<=RT_A; acc_start<=1'b1; ret<=M_S4; pst<=W_START; end
        M_S4: begin acc_mode<=1'b0; acc_base<=COL_B; rdtag<=RT_B; acc_start<=1'b1; ret<=M_S5; pst<=W_START; end
        M_S5: begin acc_mode<=1'b0; acc_base<=COL_C; rdtag<=RT_C; acc_start<=1'b1; ret<=M_S6; pst<=W_START; end
        M_S6: begin opn<=2'd0; pl<=4'd0; pst<=M_PRE; end
        M_PRE: begin
          case (pl)
            4'd0: begin pre_we<=1'b1; pre_addr<=CR_A; pre_data<=aReg; pl<=4'd1; end
            4'd1: begin pre_we<=1'b1; pre_addr<=CR_B; pre_data<=bReg; pl<=4'd2; end
            4'd2: begin pre_we<=1'b1; pre_addr<=CR_Z; pre_data<=zrow; pl<=4'd3; end
            4'd3: pl<=4'd4;
            4'd4: pl<=4'd5;
            default: begin pst<=M_RUN; run_en<=1'b1; cidx<=4'd0; end
          endcase
        end
        M_RUN: begin
          if (cmd_ack) cidx<=cidx+4'd1;
          if (cidx==NW) begin pst<=M_SET; settle<=16'd0; end
        end
        M_SET: if (settle>=16'd300) begin engRes<=cdram_out; run_en<=1'b0;
                 acc_mode<=1'b1; acc_base<=COL_R; acc_wdata<=cdram_out; acc_start<=1'b1; ret<=M_RRES; pst<=W_START;
               end else settle<=settle+16'd1;
        M_RRES: begin acc_mode<=1'b0; acc_base<=COL_R; readingRes<=1'b1; acc_start<=1'b1; ret<=M_STORE; pst<=W_START; end
        M_STORE: begin
          if (opn==2'd0) begin andEng<=engRes; andD<=rRes; status[0]<=(engRes==oracle)&&(rRes==oracle); end
          else if (opn==2'd1) begin orEng<=engRes; orD<=rRes; status[1]<=(engRes==oracle)&&(rRes==oracle); end
          else begin majEng<=engRes; majD<=rRes; status[2]<=(engRes==oracle)&&(rRes==oracle); end
          if (opn==2'd2) pst<=M_DONE;
          else begin opn<=opn+2'd1; pl<=4'd0; pst<=M_PRE; end
        end
        M_DONE: ;
        W_START: if (acc_busy) pst<=W_DONE;
        W_DONE: if (!acc_busy) begin
                  if (readingRes) begin rRes<=acc_rdata; readingRes<=1'b0; end
                  else case (rdtag)
                    RT_A: aReg<=acc_rdata; RT_B: bReg<=acc_rdata; RT_C: cReg<=acc_rdata; default: ;
                  endcase
                  rdtag<=RT_NONE; pst<=ret;
                end
        default: pst<=M_IDLE;
      endcase
    end
  end
  assign led = (pst==M_DONE) && (status==3'b111);

  // report {0xCE, A(8),B(8),C(8), AND_ddr2(8),OR_ddr2(8),MAJ_ddr2(8), status, 0x5A} = 51 bytes, MSB-first
  reg [7:0] rec [0:50];
  integer w;
  always @(*) begin
    rec[0]=8'hCE;
    for (w=0;w<8;w=w+1) begin
      rec[1+w]  = aReg[8*(7-w) +: 8];
      rec[9+w]  = bReg[8*(7-w) +: 8];
      rec[17+w] = cReg[8*(7-w) +: 8];
      rec[25+w] = andD[8*(7-w) +: 8];
      rec[33+w] = orD[8*(7-w) +: 8];
      rec[41+w] = majD[8*(7-w) +: 8];
    end
    rec[49]={5'd0,status}; rec[50]=8'h5A;
  end
  localparam CLKS=1085;
  reg [15:0] cc; reg [2:0] tst; reg [2:0] bi; reg [5:0] pos; reg [15:0] gap; reg [7:0] by;
  localparam T_IDLE=3'd0, T_LOAD=3'd1, T_START=3'd2, T_DATA=3'd3, T_STOP=3'd4, T_GAP=3'd5;
  always @(posedge ddr_clk) begin
    if (rst) begin uart_tx<=1'b1; cc<=0; tst<=T_IDLE; bi<=0; pos<=0; gap<=0; by<=8'hCE; end
    else case (tst)
      T_IDLE:  begin uart_tx<=1'b1; if (pst==M_DONE) begin pos<=6'd0; tst<=T_LOAD; end end
      T_LOAD:  begin by<=rec[pos]; cc<=0; tst<=T_START; end
      T_START: begin uart_tx<=1'b0; if (cc==CLKS-1) begin cc<=0; bi<=0; tst<=T_DATA; end else cc<=cc+1; end
      T_DATA:  begin uart_tx<=(by>>bi)&1'b1; if (cc==CLKS-1) begin cc<=0; if (bi==3'd7) tst<=T_STOP; else bi<=bi+1; end else cc<=cc+1; end
      T_STOP:  begin uart_tx<=1'b1; if (cc==CLKS-1) begin cc<=0; if (pos==6'd50) begin tst<=T_GAP; gap<=0; end else begin pos<=pos+1; tst<=T_LOAD; end end else cc<=cc+1; end
      T_GAP:   begin uart_tx<=1'b1; if (gap==16'd40000) begin gap<=0; tst<=T_IDLE; end else gap<=gap+1; end
      default: tst<=T_IDLE;
    endcase
  end
endmodule
