// zcu104_top — board top that carries a Nexys UART SoC onto the ZCU104 (Zynq UltraScale+ XCZU7EV) unchanged.
//
// The inner SoC is picked at synthesis time with `INNER` (default nexys_wpe_soc): nexys_wpe_soc (the universal
// whole-program engine), nexys_soc / nexys_soc_tformer (the full 113-op core, which carries the XPU units
// bigint_alu / bigint_div / crdt_merge and the FPU), or nexys_fabric_soc (the reconfigurable CGRA; define
// INNER_NO_RX, it has no receive line). Same wire protocol, same host scripts: only the pins and the clock differ.
//
// Clock: the PL's 300 MHz SI5341B pair (AH18/AH17) through an MMCM to 100 MHz, the Nexys oscillator rate, so every
// CLKS_PER_BIT (868 → 115200 baud) and on-chip divider carries over untouched. VCO = 300 × 4 = 1200 MHz, inside the
// UltraScale+ MMCM's 800–1600 MHz range; ÷ 12 = 100 MHz.
// UART: uart2_PL (FPGA TX C19, RX A20, LVCMOS18), the FT4232's PL channel, seen by the host as /dev/ttyUSB3.
// Reset: held until the MMCM locks, then follows GPIO_PB_SW0 (B4); synchronized into the 100 MHz domain.
// LEDs: the inner SoC's led[3:0] on GPIO_LED_0..3 (D5 D6 A5 B5).
`ifndef INNER
`define INNER nexys_wpe_soc
`endif
module zcu104_top (
  input        clk_300_p,
  input        clk_300_n,
  input        rst_btn,
  input        uart_rx,
  output       uart_tx,
  output [3:0] led
);
  wire clk_300;
  IBUFDS u_ibuf (.I(clk_300_p), .IB(clk_300_n), .O(clk_300));

  wire clk_fb, clk_100_unbuf, locked, clk;
  MMCME4_BASE #(
    .CLKIN1_PERIOD(3.333),
    .CLKFBOUT_MULT_F(4.0),
    .DIVCLK_DIVIDE(1),
    .CLKOUT0_DIVIDE_F(12.0)
  ) u_mmcm (
    .CLKIN1(clk_300), .CLKFBIN(clk_fb), .CLKFBOUT(clk_fb),
    .CLKOUT0(clk_100_unbuf), .LOCKED(locked), .RST(1'b0), .PWRDWN(1'b0)
  );
  BUFG u_bufg (.I(clk_100_unbuf), .O(clk));

  reg [3:0] rst_sync = 4'hF;
  always @(posedge clk) rst_sync <= {rst_sync[2:0], ~locked | rst_btn};
  wire rst = rst_sync[3];

  wire [15:0] led16;
  `INNER u_soc (
    .clk(clk), .rst(rst),
`ifndef INNER_NO_RX
    .uart_rx(uart_rx),
`endif
    .uart_tx(uart_tx), .led(led16)
  );
  assign led = led16[3:0];
endmodule
