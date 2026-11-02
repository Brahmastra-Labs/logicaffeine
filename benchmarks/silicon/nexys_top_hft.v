// nexys_top_hft — board top for the HFT tick-to-trade SoC on the Nexys A7-100T. Identical clocking to
// nexys_top_full (100 MHz /16 -> 6.25 MHz core on a BUFG, power-on reset pulse, CLKS_PER_BIT = 54 for
// 115200 baud), wrapping nexys_soc_hft (512-word imem + 2-byte UART length) so the full pipeline loads.
// Ports match benchmarks/silicon/nexys_soc.xdc: clk (E3), rst (J15), uart_rx (C4), uart_tx (D4), led[15:0].
module nexys_top_hft (
  input         clk,
  input         rst,
  input         uart_rx,
  output        uart_tx,
  output [15:0] led
);
  reg [3:0] divcnt = 4'd0;
  always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire coreclk;
  BUFG bufg_core (.I(divcnt[3]), .O(coreclk));

  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 1'b1;
  wire soc_rst = rst | ~por[3];

  nexys_soc_hft #(.CLKS_PER_BIT(54)) soc (   // 6.25e6 / 115200 = 54
    .clk(coreclk), .rst(soc_rst), .uart_rx(uart_rx), .uart_tx(uart_tx), .led(led)
  );
endmodule
