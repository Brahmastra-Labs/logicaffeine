// ddr2_phy_stage1 — DDR2 PHY bring-up Stage 1: prove the SSTL18 / differential DDR2 I/O routes
// through the open flow (yosys -> nextpnr-xilinx -> prjxray) on the Nexys A7-100T. Drives the DDR2
// differential clock (ODDR -> OBUFDS), holds the command bus at a safe idle (CKE low = power-down,
// DESELECT), tri-states DQ/DQS, and streams a 0xA5 UART heartbeat so we can confirm on silicon that
// the design is alive and the DDR2 pins placed+routed. (Init + read calibration are Stage 2/3.)
module ddr2_phy_stage1 (
  input clk,
  output reg uart_tx,
  output led,
  output [12:0] ddr2_addr, output [2:0] ddr2_ba,
  output ddr2_ras_n, output ddr2_cas_n, output ddr2_we_n, output ddr2_cs_n,
  output ddr2_cke, output ddr2_odt, output [1:0] ddr2_dm,
  output ddr2_ck_p, output ddr2_ck_n,
  inout [15:0] ddr2_dq, inout [1:0] ddr2_dqs_p, inout [1:0] ddr2_dqs_n
);
  reg [7:0] por = 8'd0;
  wire rst = (por < 8'd64);
  always @(posedge clk) if (por < 8'd64) por <= por + 8'd1;

  // DDR2 differential clock. Stage 1 forwards a plain clk/2 toggle through OBUFDS (proves the
  // differential SSTL18 output routes); true ODDR clock-forwarding at tCK is a Stage-2 concern.
  reg ckreg = 1'b0;
  always @(posedge clk) ckreg <= ~ckreg;
  OBUFDS u_ckbuf(.I(ckreg), .O(ddr2_ck_p), .OB(ddr2_ck_n));

  // Command bus held at a safe idle: CKE low (DRAM stays in power-down), DESELECT (cs_n high).
  assign ddr2_cke  = 1'b0;
  assign ddr2_cs_n = 1'b1;
  assign ddr2_ras_n = 1'b1; assign ddr2_cas_n = 1'b1; assign ddr2_we_n = 1'b1;
  assign ddr2_ba = 3'b0; assign ddr2_addr = 13'b0; assign ddr2_odt = 1'b0; assign ddr2_dm = 2'b0;
  // Data + strobes tri-stated in Stage 1 (not yet driven/sampled).
  assign ddr2_dq = 16'bz;
  assign ddr2_dqs_p = 2'bz; assign ddr2_dqs_n = 2'bz;

  assign led = ~rst;

  // UART heartbeat: 0xA5 repeated (8N1, 115200 @ 100 MHz -> CLKS_PER_BIT = 868).
  localparam CLKS = 868;
  reg [15:0] c; reg [1:0] st; reg [2:0] bi; reg [15:0] gap;
  localparam ST_START=2'd0, ST_DATA=2'd1, ST_STOP=2'd2, ST_GAP=2'd3;
  always @(posedge clk) begin
    if (rst) begin uart_tx<=1'b1; c<=16'd0; st<=ST_START; bi<=3'd0; gap<=16'd0; end
    else case (st)
      ST_START: begin uart_tx<=1'b0; if (c==CLKS-1) begin c<=16'd0; bi<=3'd0; st<=ST_DATA; end else c<=c+16'd1; end
      ST_DATA:  begin uart_tx<=(8'hA5 >> bi) & 1'b1;
                  if (c==CLKS-1) begin c<=16'd0; if (bi==3'd7) st<=ST_STOP; else bi<=bi+3'd1; end else c<=c+16'd1; end
      ST_STOP:  begin uart_tx<=1'b1; if (c==CLKS-1) begin c<=16'd0; st<=ST_GAP; gap<=16'd0; end else c<=c+16'd1; end
      ST_GAP:   begin uart_tx<=1'b1; if (gap==16'd20000) begin gap<=16'd0; st<=ST_START; end else gap<=gap+16'd1; end
    endcase
  end
endmodule
