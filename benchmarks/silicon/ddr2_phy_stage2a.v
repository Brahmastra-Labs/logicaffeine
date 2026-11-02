// ddr2_phy_stage2a — DDR2 PHY bring-up Stage 2a: the CLOCKING infrastructure. An MMCM turns the
// 100 MHz board clock into a 150 MHz DDR2 clock (≥125 MHz so the DDR2 DLL can lock), a 200 MHz
// IDELAYCTRL reference, and a 90°-phase capture clock; IDELAYCTRL is brought up (its RDY gates the
// read path in Stage 2c). The DDR2 clock is forwarded to ck_p/ck_n; the command bus is held idle.
// A UART heartbeat (0xA5) on the 150 MHz domain, encoding MMCM-lock + IDELAY-ready in the low bits of
// a status byte, confirms the whole clocking tree is alive on silicon.
module ddr2_phy_stage2a (
  input clk,
  output reg uart_tx,
  output led,
  output [12:0] ddr2_addr, output [2:0] ddr2_ba,
  output ddr2_ras_n, output ddr2_cas_n, output ddr2_we_n, output ddr2_cs_n,
  output ddr2_cke, output ddr2_odt, output [1:0] ddr2_dm,
  output ddr2_ck_p, output ddr2_ck_n,
  inout [15:0] ddr2_dq, inout [1:0] ddr2_dqs_p, inout [1:0] ddr2_dqs_n
);
  // ---- MMCM: 100 MHz -> 150 MHz (ddr), 200 MHz (idelay ref), 150 MHz@90 (capture) ----
  wire clkfb, ddr_raw, ref_raw, cap_raw, mmcm_locked;
  wire ddr_clk, ref200, ddr_clk90;
  // nextpnr-xilinx supports PLLE2 (not MMCME2) on 7-series — use PLLE2_BASE.
  PLLE2_BASE #(
    .CLKIN1_PERIOD(10.0),
    .CLKFBOUT_MULT(10), .DIVCLK_DIVIDE(1),
    .CLKOUT0_DIVIDE(8),                           // 1000/8 = 125 MHz (DDR2 DLL minimum)
    .CLKOUT1_DIVIDE(5),                           // 1000/5 = 200 MHz (IDELAYCTRL ref)
    .CLKOUT2_DIVIDE(8), .CLKOUT2_PHASE(90.0)      // 125 MHz @ 90 deg (capture)
  ) u_pll (
    .CLKIN1(clk), .CLKFBIN(clkfb), .CLKFBOUT(clkfb),
    .CLKOUT0(ddr_raw), .CLKOUT1(ref_raw), .CLKOUT2(cap_raw),
    .LOCKED(mmcm_locked), .RST(1'b0), .PWRDWN(1'b0)
  );
  BUFG bg0(.I(ddr_raw), .O(ddr_clk));
  BUFG bg1(.I(ref_raw), .O(ref200));
  BUFG bg2(.I(cap_raw), .O(ddr_clk90));

  // (IDELAYCTRL is instantiated together with the read-path IDELAYE2s in Stage 2c — nextpnr rejects
  // a lone IDELAYCTRL with no I/ODELAYs. ref200 is kept for that stage.)

  // power-on reset on the ddr domain, released after the MMCM locks
  reg [7:0] por = 8'd0;
  wire rst = (por < 8'd200) | ~mmcm_locked;
  always @(posedge ddr_clk) if (por < 8'd200) por <= por + 8'd1;

  // forward the DDR2 clock differentially
  OBUFDS u_ckbuf(.I(ddr_clk), .O(ddr2_ck_p), .OB(ddr2_ck_n));

  // command bus idle (CKE low = power-down); data/strobes tri-stated
  assign ddr2_cke=1'b0; assign ddr2_cs_n=1'b1; assign ddr2_ras_n=1'b1; assign ddr2_cas_n=1'b1;
  assign ddr2_we_n=1'b1; assign ddr2_ba=3'b0; assign ddr2_addr=13'b0; assign ddr2_odt=1'b0; assign ddr2_dm=2'b0;
  assign ddr2_dq=16'bz; assign ddr2_dqs_p=2'bz; assign ddr2_dqs_n=2'bz;
  assign led = mmcm_locked;

  // status byte: 0xA7 once the MMCM clocking tree is locked (0xA4 while unlocked).
  wire [7:0] status = mmcm_locked ? 8'hA7 : 8'hA4;
  // UART heartbeat on the 125 MHz domain (125e6/115200 = 1085).
  localparam CLKS = 1085;
  reg [15:0] c; reg [1:0] st; reg [2:0] bi; reg [15:0] gap; reg [7:0] by;
  localparam ST_START=2'd0, ST_DATA=2'd1, ST_STOP=2'd2, ST_GAP=2'd3;
  always @(posedge ddr_clk) begin
    if (rst) begin uart_tx<=1'b1; c<=16'd0; st<=ST_START; bi<=3'd0; gap<=16'd0; by<=8'hA5; end
    else case (st)
      ST_START: begin by<=status; uart_tx<=1'b0; if (c==CLKS-1) begin c<=16'd0; bi<=3'd0; st<=ST_DATA; end else c<=c+16'd1; end
      ST_DATA:  begin uart_tx<=(by >> bi) & 1'b1;
                  if (c==CLKS-1) begin c<=16'd0; if (bi==3'd7) st<=ST_STOP; else bi<=bi+3'd1; end else c<=c+16'd1; end
      ST_STOP:  begin uart_tx<=1'b1; if (c==CLKS-1) begin c<=16'd0; st<=ST_GAP; gap<=16'd0; end else c<=c+16'd1; end
      ST_GAP:   begin uart_tx<=1'b1; if (gap==16'd30000) begin gap<=16'd0; st<=ST_START; end else gap<=gap+16'd1; end
    endcase
  end
endmodule
