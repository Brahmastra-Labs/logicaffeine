// nexys_top_tformer — board top for the baby-transformer inference demo on the Nexys A7-100T.
// Mirrors nexys_top_full: /16 core clock (6.25 MHz) on a BUFG for the full soft-core, power-on reset,
// CLKS_PER_BIT tracking the core clock. Adds the 7-segment display: `sevenseg` runs on the FAST 100 MHz
// board clock (so the refresh doesn't flicker) and shows `disp_val` (the last predicted token id the SoC
// latched), synchronised across the clock domains with a 2-FF sync (disp_val changes only ~once per token).
module nexys_top_tformer (
  input         clk,          // 100 MHz board oscillator (E3)
  input         rst,
  input         uart_rx,
  output        uart_tx,
  output [15:0] led,
  output [6:0]  seg,          // 7-seg cathodes CA..CG (active low)
  output        dp,           // decimal point (active low, kept off)
  output [7:0]  an            // 7-seg anodes AN0..AN7 (active low)
);
  // /16 core clock -> 6.25 MHz on a global buffer (same conservative margin as nexys_top_full).
  reg [3:0] divcnt = 4'd0;
  always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire coreclk;
  BUFG bufg_core (.I(divcnt[3]), .O(coreclk));

  // Power-on reset pulse on the core clock (hold the SoC until it initialises).
  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 1'b1;
  wire soc_rst = rst | ~por[3];

  wire [31:0] disp_val;
  nexys_soc_tformer #(.CLKS_PER_BIT(54)) soc (   // 6.25e6 / 115200 = 54
    .clk(coreclk), .rst(soc_rst), .uart_rx(uart_rx), .uart_tx(uart_tx), .led(led), .disp_val(disp_val)
  );

  // CDC: disp_val (6.25 MHz domain, changes ~once per generated token) -> the 100 MHz display domain.
  reg [31:0] disp_s1 = 32'd0, disp_s2 = 32'd0;
  always @(posedge clk) begin disp_s1 <= disp_val; disp_s2 <= disp_s1; end

  sevenseg #(.REFRESH_BITS(17)) disp (
    .clk(clk), .rst(rst), .val(disp_s2), .blank(1'b0), .seg(seg), .dp(dp), .an(an)
  );
endmodule
