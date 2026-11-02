// nexys_top_scalar — board top for the scalar LOGOS SoC on the Nexys A7-100T. The design's routed
// critical path (~23 ns, routing-dominated by the Call/Return register window) tops out around 43 MHz,
// so the 100 MHz board oscillator is divided by 4 to a 25 MHz core clock (40 ns period — comfortable
// margin) on a global clock buffer, and the whole SoC runs on it. CLKS_PER_BIT = 25e6/115200 = 217 keeps
// the UART at 115200 baud. On-chip run-cycles are counted at 25 MHz → wall time = cycles / 25 MHz
// (nexys-host.py --clock-mhz 25).
//
// Ports match benchmarks/silicon/nexys_soc.xdc: clk (E3 100 MHz), rst (J15), uart_rx (C4), uart_tx (D4),
// led[15:0]. Build with the open flow: scripts/impl-nexys-soc-openxc7.sh nexys_top_scalar.
module nexys_top_scalar (
  input         clk,          // 100 MHz board oscillator (E3)
  input         rst,
  input         uart_rx,
  output        uart_tx,
  output [15:0] led
);
  // /16 clock divider -> 6.25 MHz core clock, distributed on a global buffer (BUFG). The scalar core's
  // critical path (256-deep async imem read + ALU + regfile) is far slower than nextpnr-xilinx's
  // approximate 7-series timing model reports, so we clock it conservatively; CLKS_PER_BIT below tracks it.
  reg [3:0] divcnt = 4'd0;
  always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire coreclk;
  BUFG bufg_core (.I(divcnt[3]), .O(coreclk));

  // Power-on reset. The SoC needs a reset PULSE to initialize (hold the core, idle uart_tx HIGH, put the
  // loader FSM at L_LEN) — after configuration the board rst switch alone provides no edge, so without
  // this the core would execute garbage imem and the TX line would idle low (silent board). Hold reset for
  // the first 8 coreclk cycles, then release; the external rst (J15) stays an additional manual reset.
  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 1'b1;
  wire soc_rst = rst | ~por[3];

  nexys_soc_scalar #(.CLKS_PER_BIT(54)) soc (   // 6.25e6 / 115200 = 54
    .clk(coreclk), .rst(soc_rst), .uart_rx(uart_rx), .uart_tx(uart_tx), .led(led)
  );
endmodule
