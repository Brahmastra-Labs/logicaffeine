// nexys_video_wpe — board top for the whole-program engine on Board A (Nexys Video, XC7A200T). Identical to
// nexys_wpe_soc (Board B) EXCEPT the byte PHY: the Nexys Video exposes an FT2232H async FIFO (DPTI), not a
// serial UART, so an ft245_uart_bridge adapts the FIFO to the engine's serial uart_rx/uart_tx. The engine
// (wpe_soc_fpga) is byte-for-byte the same as on the Nexys A7 — the LANGUAGE-burned-to-fabric engine runs
// unchanged; only how bytes reach it differs. Host streams .wire files over the DPTI channel's /dev/ttyUSB
// exactly as on Board B (nexys-wpe-host.py). Flash: openFPGALoader -c digilent_b. XDC: nexys_video_fifo.xdc.
module nexys_video_wpe (
  input  clk,                    // R4 100 MHz
  inout  [7:0] prog_d,           // FT245 async FIFO (DPTI) — bank 14
  input  prog_rxen, input prog_txen,
  output prog_rdn, output prog_wrn, output prog_oen, output prog_siwun, output prog_spien,
  output [7:0] led
);
  // /128 -> 781.25 kHz core clock (same deeply-conservative clock as Board B: the engine's nested
  // combinational config-memory reads are a long path the open router's 7-series model underestimates).
  reg [6:0] divcnt = 7'd0; always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire coreclk; BUFG bufg_core (.I(divcnt[6]), .O(coreclk));

  reg [3:0] por = 4'd0; always @(posedge coreclk) if (!por[3]) por <= por + 1'b1;
  wire soc_rst = ~por[3];

  reg [25:0] hb = 26'd0; always @(posedge coreclk) hb <= hb + 1'b1;
  assign led = {hb[25:19], ~por[3]};   // heartbeat + reset-active

  wire eng_rx, eng_tx;
  ft245_uart_bridge #(.CLKS_PER_BIT(81), .W(4)) bridge (
    .clk(coreclk), .rst(soc_rst),
    .eng_rx(eng_rx), .eng_tx(eng_tx),
    .prog_d(prog_d), .prog_rxen(prog_rxen), .prog_txen(prog_txen),
    .prog_rdn(prog_rdn), .prog_wrn(prog_wrn), .prog_oen(prog_oen),
    .prog_siwun(prog_siwun), .prog_spien(prog_spien)
  );

  // WITH_FPU(0) integer engine first (matches Board B's deployed config) — floats come with a WITH_FPU(1)
  // build, which the 200T has the room for.
  wpe_soc_fpga #(.CLKS_PER_BIT(81), .WITH_FPU(0)) engine (
    .clk(coreclk), .rst(soc_rst), .uart_rx(eng_rx), .uart_tx(eng_tx)
  );
endmodule
