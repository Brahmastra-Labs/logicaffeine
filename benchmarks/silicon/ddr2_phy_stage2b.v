// ddr2_phy_stage2b — DDR2 PHY bring-up Stage 2b: run the DDR2 INIT sequence on real silicon. PLLE2
// makes the 125 MHz DDR clock; ddr2_init_fsm (the generated JEDEC power-up + mode-register sequencer,
// 200 us CKE-low then EMRS2/EMRS3/EMRS1[DLL-en]/MRS[DLL-reset]/PRE/2x REF/MRS/EMRS1[OCD]) drives the
// real DDR2 command bus. A UART status byte reports {init_done, pll_locked}: 0xA3 once the DRAM is
// initialized. (Data path / read calibration is Stage 2c.)
module ddr2_phy_stage2b (
  input clk,
  output reg uart_tx,
  output led,
  output [12:0] ddr2_addr, output [2:0] ddr2_ba,
  output ddr2_ras_n, output ddr2_cas_n, output ddr2_we_n, output ddr2_cs_n,
  output ddr2_cke, output ddr2_odt, output [1:0] ddr2_dm,
  output ddr2_ck_p, output ddr2_ck_n,
  inout [15:0] ddr2_dq, inout [1:0] ddr2_dqs_p, inout [1:0] ddr2_dqs_n
);
  wire clkfb, ddr_raw, pll_locked, ddr_clk;
  PLLE2_BASE #(
    .CLKIN1_PERIOD(10.0), .CLKFBOUT_MULT(10), .DIVCLK_DIVIDE(1),
    .CLKOUT0_DIVIDE(8)                            // 1000/8 = 125 MHz
  ) u_pll (
    .CLKIN1(clk), .CLKFBIN(clkfb), .CLKFBOUT(clkfb),
    .CLKOUT0(ddr_raw), .LOCKED(pll_locked), .RST(1'b0), .PWRDWN(1'b0)
  );
  BUFG bg0(.I(ddr_raw), .O(ddr_clk));

  reg [15:0] por = 16'd0;
  wire rst = (por < 16'd1000) | ~pll_locked;
  always @(posedge ddr_clk) if (por < 16'd1000) por <= por + 16'd1;

  // Forward the 125 MHz DDR clock differentially.
  OBUFDS u_ckbuf(.I(ddr_clk), .O(ddr2_ck_p), .OB(ddr2_ck_n));

  // The generated JEDEC init sequencer drives the command bus.
  wire i_cke, i_cs, i_ras, i_cas, i_we, init_done;
  wire [1:0] i_ba; wire [12:0] i_a;
  ddr2_init_fsm u_init(
    .clk(ddr_clk), .rst(rst),
    .cke(i_cke), .cs_n(i_cs), .ras_n(i_ras), .cas_n(i_cas), .we_n(i_we),
    .ba(i_ba), .a(i_a), .init_done(init_done)
  );
  assign ddr2_cke=i_cke; assign ddr2_cs_n=i_cs; assign ddr2_ras_n=i_ras;
  assign ddr2_cas_n=i_cas; assign ddr2_we_n=i_we;
  assign ddr2_ba={1'b0, i_ba}; assign ddr2_addr=i_a; assign ddr2_odt=1'b0; assign ddr2_dm=2'b0;
  assign ddr2_dq=16'bz; assign ddr2_dqs_p=2'bz; assign ddr2_dqs_n=2'bz;
  assign led = init_done;

  // Status byte 0xA0 | {init_done, pll_locked} -> 0xA3 when the DRAM is initialized.
  wire [7:0] status = {6'b101000, init_done, pll_locked};
  localparam CLKS = 1085;
  reg [15:0] c; reg [1:0] st; reg [2:0] bi; reg [15:0] gap; reg [7:0] by;
  localparam ST_START=2'd0, ST_DATA=2'd1, ST_STOP=2'd2, ST_GAP=2'd3;
  always @(posedge ddr_clk) begin
    if (rst) begin uart_tx<=1'b1; c<=16'd0; st<=ST_START; bi<=3'd0; gap<=16'd0; by<=8'hA0; end
    else case (st)
      ST_START: begin by<=status; uart_tx<=1'b0; if (c==CLKS-1) begin c<=16'd0; bi<=3'd0; st<=ST_DATA; end else c<=c+16'd1; end
      ST_DATA:  begin uart_tx<=(by >> bi) & 1'b1;
                  if (c==CLKS-1) begin c<=16'd0; if (bi==3'd7) st<=ST_STOP; else bi<=bi+3'd1; end else c<=c+16'd1; end
      ST_STOP:  begin uart_tx<=1'b1; if (c==CLKS-1) begin c<=16'd0; st<=ST_GAP; gap<=16'd0; end else c<=c+16'd1; end
      ST_GAP:   begin uart_tx<=1'b1; if (gap==16'd30000) begin gap<=16'd0; st<=ST_START; end else gap<=gap+16'd1; end
    endcase
  end
endmodule
