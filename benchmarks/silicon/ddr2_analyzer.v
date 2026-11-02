// ddr2_analyzer — on-chip logic analyzer for the DDR2 bring-up. After init it runs one
// ACT -> WRITE -> PRE -> ACT -> READ sequence and records, for 64 consecutive ddr cycles, a trace
// byte {cke, cs_n, ras_n, cas_n, we_n, dq_oe, dqs0_in, dq0_in}. dq_oe=1 marks the FPGA driving
// (write); dqs0_in/dq0_in during the read window (dq_oe=0) show whether the DRAM is driving the bus.
// The 64-byte trace is streamed over UART as {0xD5, trace[0..63], 0x5A}. This gives pin visibility
// without an external scope: DQS toggling on read => DRAM responds; DQS flat => DRAM not responding.
module ddr2_analyzer (
  input clk,
  output reg uart_tx,
  output led,
  output [12:0] ddr2_addr, output [2:0] ddr2_ba,
  output ddr2_ras_n, output ddr2_cas_n, output ddr2_we_n, output ddr2_cs_n,
  output ddr2_cke, output ddr2_odt, output [1:0] ddr2_dm,
  output ddr2_ck_p, output ddr2_ck_n,
  inout [15:0] ddr2_dq, inout [1:0] ddr2_dqs_p, inout [1:0] ddr2_dqs_n
);
  wire clkfb, ddr_raw, ck180_raw, cap_raw, pll_locked, ddr_clk, ddr_clk180, ddr_clk90;
  PLLE2_BASE #(
    .CLKIN1_PERIOD(10.0), .CLKFBOUT_MULT(10), .DIVCLK_DIVIDE(1),
    .CLKOUT0_DIVIDE(8), .CLKOUT1_DIVIDE(8), .CLKOUT1_PHASE(180.0),
    .CLKOUT2_DIVIDE(8), .CLKOUT2_PHASE(90.0)
  ) u_pll (
    .CLKIN1(clk), .CLKFBIN(clkfb), .CLKFBOUT(clkfb),
    .CLKOUT0(ddr_raw), .CLKOUT1(ck180_raw), .CLKOUT2(cap_raw),
    .LOCKED(pll_locked), .RST(1'b0), .PWRDWN(1'b0)
  );
  BUFG bg0(.I(ddr_raw), .O(ddr_clk));
  BUFG bg1(.I(ck180_raw), .O(ddr_clk180));
  BUFG bg2(.I(cap_raw), .O(ddr_clk90));

  reg [15:0] por = 16'd0;
  wire rst = (por < 16'd1000) | ~pll_locked;
  always @(posedge ddr_clk) if (por < 16'd1000) por <= por + 16'd1;

  // Pseudo-differential clock as two complementary LVCMOS18 outputs (0 deg / 180 deg).
  OBUF u_ckp(.I(ddr_clk),    .O(ddr2_ck_p));
  OBUF u_ckn(.I(ddr_clk180), .O(ddr2_ck_n));

  wire i_cke, i_cs, i_ras, i_cas, i_we, init_done; wire [1:0] i_ba; wire [12:0] i_a;
  ddr2_init_fsm u_init(.clk(ddr_clk), .rst(rst), .cke(i_cke), .cs_n(i_cs), .ras_n(i_ras),
    .cas_n(i_cas), .we_n(i_we), .ba(i_ba), .a(i_a), .init_done(init_done));

  localparam [15:0] PAT = 16'hA5C3;   // write pattern; a read that returns this means the DRAM responded
  reg [9:0] cmd_cnt; reg [1:0] phase;
  localparam P_WAIT=2'd0, P_RUN=2'd1, P_REPORT=2'd2;
  reg c_cs, c_ras, c_cas, c_we, c_odt; reg [2:0] c_ba; reg [12:0] c_a; reg dq_oe;

  // DQ IOBUF (drive PAT on write, read otherwise); capture the received DQ.
  wire [15:0] dq_in;
  genvar gi;
  generate for (gi=0; gi<16; gi=gi+1) begin: dqb
    IOBUF io(.IO(ddr2_dq[gi]), .I(PAT[gi]), .T(~dq_oe), .O(dq_in[gi]));
  end endgenerate
  // DQS IOBUFDS: drive a fabric strobe on write, capture the received DQS on read.
  reg dqs_fab = 1'b0;
  always @(posedge ddr_clk90) dqs_fab <= ~dqs_fab;
  wire [1:0] dqs_in;
  generate for (gi=0; gi<2; gi=gi+1) begin: dqslane
    IOBUF u_dqsp(.I(dqs_fab),  .T(~dq_oe), .IO(ddr2_dqs_p[gi]), .O(dqs_in[gi]));
    IOBUF u_dqsn(.I(~dqs_fab), .T(~dq_oe), .IO(ddr2_dqs_n[gi]), .O());
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

  localparam TRCD=2, TWR=3, TRP=2;
  localparam [9:0] READ_CYC = 10'd1+TRCD+4+TWR+TRP+1+TRCD; // = 15
  (* ram_style = "registers" *) reg [7:0] trace [0:63];
  wire [7:0] tracebyte = {ddr2_cke, ddr2_cs_n, ddr2_ras_n, ddr2_cas_n, ddr2_we_n, dq_oe, dqs_in[0], dq_in[0]};

  always @(posedge ddr_clk) begin
    if (rst) begin
      phase<=P_WAIT; cmd_cnt<=10'd0; dq_oe<=1'b0;
      c_cs<=1'b1; c_ras<=1'b1; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; c_odt<=1'b0;
    end else begin
      dq_oe<=1'b0; c_cs<=1'b1; c_ras<=1'b1; c_cas<=1'b1; c_we<=1'b1; c_odt<=1'b0;
      case (phase)
        P_WAIT: if (init_done) begin cmd_cnt<=10'd0; phase<=P_RUN; end
        P_RUN: begin
          cmd_cnt<=cmd_cnt+10'd1;
          if (cmd_cnt<10'd64) trace[cmd_cnt[5:0]]<=tracebyte;
          if (cmd_cnt==10'd1) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; end            // ACT
          else if (cmd_cnt>=10'd1+TRCD && cmd_cnt<=10'd1+TRCD+3) begin dq_oe<=1'b1;
                 if (cmd_cnt==10'd1+TRCD) begin c_cs<=1'b0; c_ras<=1'b1; c_cas<=1'b0; c_we<=1'b0; c_ba<=3'd0; c_a<=13'd0; c_odt<=1'b1; end end // WRITE
          else if (cmd_cnt==10'd1+TRCD+4+TWR) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b0; c_a<=13'd0; end          // PRE
          else if (cmd_cnt==10'd1+TRCD+4+TWR+TRP+1) begin c_cs<=1'b0; c_ras<=1'b0; c_cas<=1'b1; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; end // ACT
          else if (cmd_cnt==READ_CYC) begin c_cs<=1'b0; c_ras<=1'b1; c_cas<=1'b0; c_we<=1'b1; c_ba<=3'd0; c_a<=13'd0; end      // READ
          else if (cmd_cnt==10'd63) phase<=P_REPORT;
        end
        P_REPORT: ;
      endcase
    end
  end
  assign led = init_done;

  // UART: stream {0xD5, trace[0..63], 0x5A} repeatedly.
  localparam CLKS=1085;
  reg [15:0] c; reg [2:0] tst; reg [2:0] bi; reg [6:0] pos; reg [15:0] gap; reg [7:0] by;
  localparam T_IDLE=3'd0, T_LOAD=3'd1, T_START=3'd2, T_DATA=3'd3, T_STOP=3'd4, T_GAP=3'd5;
  wire [7:0] outbyte = (pos==7'd0) ? 8'hD5 : (pos==7'd65) ? 8'h5A : trace[pos[5:0]-6'd1];
  always @(posedge ddr_clk) begin
    if (rst) begin uart_tx<=1'b1; c<=0; tst<=T_IDLE; bi<=0; pos<=0; gap<=0; by<=8'hD5; end
    else case (tst)
      T_IDLE:  begin uart_tx<=1'b1; if (phase==P_REPORT) begin pos<=7'd0; tst<=T_LOAD; end end
      T_LOAD:  begin by<=outbyte; c<=0; tst<=T_START; end
      T_START: begin uart_tx<=1'b0; if (c==CLKS-1) begin c<=0; bi<=0; tst<=T_DATA; end else c<=c+1; end
      T_DATA:  begin uart_tx<=(by>>bi)&1'b1; if (c==CLKS-1) begin c<=0; if (bi==3'd7) tst<=T_STOP; else bi<=bi+1; end else c<=c+1; end
      T_STOP:  begin uart_tx<=1'b1; if (c==CLKS-1) begin c<=0; if (pos==7'd65) begin tst<=T_GAP; gap<=0; end else begin pos<=pos+1; tst<=T_LOAD; end end else c<=c+1; end
      T_GAP:   begin uart_tx<=1'b1; if (gap==16'd40000) begin gap<=0; tst<=T_IDLE; end else gap<=gap+1; end
      default: tst<=T_IDLE;
    endcase
  end
endmodule
