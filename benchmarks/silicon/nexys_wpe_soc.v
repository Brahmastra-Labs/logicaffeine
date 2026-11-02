// nexys_wpe_soc — board top for the UNIVERSAL over-the-wire whole-program engine on the Nexys A7-100T.
// A host streams ANY fitting program's flat image + params over the UART (WImage::wire_bytes); the on-chip
// loader fills the engine's config memories (the chip synthesizes the program it is handed), the interpret
// st-machine runs it (≡ eval_wprog_config ≡ the VM), and the 64-bit result + run-cycle count stream back.
// This is the ENGINE on real silicon — a config-driven whole-program processor over the wire, not a
// program-specialized netlist and not the sequential soft-core.
//
// Ports match benchmarks/silicon/nexys_soc.xdc: clk (E3 100 MHz), rst (J15), uart_rx (C4), uart_tx (D4),
// led[15:0]. Build: TOP=nexys_wpe_soc SRC="nexys_wpe_soc.v wpe_soc_fpga.v" BIT=wpe_soc.bit bash scripts/openxc7-build.sh
module nexys_wpe_soc #(
  // The IEEE-754 FPU as a TOP-LEVEL knob (an instance parameter here would override any `chparam` on the engine
  // module). Default 0 = the proven hold-safe image; a build passes `PRE="chparam -set WITH_FPU 1 nexys_wpe_soc;"`
  // to openxc7-build.sh to try the float lane on this board (its recipe records which was built).
  parameter WITH_FPU = 0
) (
  input         clk,          // 100 MHz board oscillator (E3)
  input         rst,          // J15
  input         uart_rx,      // C4  (host -> FPGA)
  output        uart_tx,      // D4  (FPGA -> host)
  output [15:0] led
);
  // /128 clock divider -> 781.25 kHz core clock on a global buffer (BUFG). The engine's cell datapath does
  // NESTED combinational memory reads (resolve reads file[liv_reg[...]] — a memory index that is itself a
  // memory read, chained with the cell-field muxes over the config memories), a long path nextpnr-xilinx's
  // approximate 7-series model badly underestimates. Clock deeply conservative so real silicon has margin;
  // CLKS_PER_BIT below tracks it (781.25e3 / 9600 = 81).
  reg [6:0] divcnt = 7'd0;
  always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire coreclk;
  BUFG bufg_core (.I(divcnt[6]), .O(coreclk));

  // Power-on reset PULSE: the SoC needs an edge to put the loader FSM at P_CNT and idle uart_tx HIGH (after
  // configuration the board rst switch alone gives no edge). Hold reset for the first 8 coreclk cycles.
  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 1'b1;
  wire soc_rst = rst | ~por[3];

  // A visible heartbeat proves the board is configured + clocking (the real status is the returned frame).
  reg [25:0] hb = 26'd0;
  always @(posedge coreclk) hb <= hb + 1'b1;
  assign led = {hb[25:11], ~por[3]};   // led[15:1] = blink, led[0] = reset-active

  // WITH_FPU(0) = the DIVIDER-ONLY engine: the full integer surface incl the sequential divider (div/mod/floordiv/
  // pow) runs on real silicon; the FPU (float) is iverilog-proven but its -nodsp 53×53 multiply is a routing
  // hotspot that overwhelms the open-flow router at this board's hold-safe cap profile, so it is excluded here.
  wpe_soc_fpga #(.CLKS_PER_BIT(81), .WITH_FPU(WITH_FPU)) engine (   // 781.25e3 / 9600 = 81 (host: --baud 9600 --clock-mhz 0.78125)
    .clk(coreclk), .rst(soc_rst), .uart_rx(uart_rx), .uart_tx(uart_tx)
  );
endmodule
