// nexys_video_wpe_soc — the bytecode engine on Board A (Nexys Video, XC7A200T).
//
// The SAME engine Board B runs: `wpe_soc_fpga` with the same caps, the same `CLKS_PER_BIT(81)` on the
// same /128 core clock, streaming the same `.wire` programs. One design, two chips.
//
// What differs is how the host's bytes reach it, and getting that wrong is invisible until the board
// says nothing at all. **Board A has no UART.** Its FT2232H channel A is wired as an ASYNC PARALLEL
// FIFO — the DPTI byte path, `prog_d[7:0]` plus handshake, bank 14 — and the pins a Nexys A7 uses for
// a serial line go nowhere here. A design that drives `uart_tx` on AA19 places, routes, configures and
// loads perfectly, and is silent forever, because nothing is listening to that pin. `ft245_uart_bridge`
// is what reconciles the two: it presents the engine with the serial pair it expects and moves those
// bits over the FIFO, exactly as the streamed-transformer top on this board already does.
module nexys_video_wpe_soc (
  input        clk,                 // R4, 100 MHz
  inout  [7:0] prog_d,              // FT245 async FIFO (DPTI) — bank 14
  input        prog_rxen,           // RXF#  (low = a byte is waiting for us)
  input        prog_txen,           // TXE#  (low = there is room for ours)
  output       prog_rdn,
  output       prog_wrn,
  output       prog_oen,
  output       prog_siwun,
  output       prog_spien,
  output [7:0] led
);
  // /128 -> 781.25 kHz, on a global buffer. Identical to Board B: the engine's cell datapath does nested
  // combinational config-memory reads, and the divisor below tracks this clock (781.25e3 / 9600 = 81).
  reg [6:0] divcnt = 7'd0;
  always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire coreclk;
  BUFG bufg_core (.I(divcnt[6]), .O(coreclk));

  // This board brings out no reset, so the engine's own power-on counter is the only edge there is: it
  // holds the loader FSM at its entry and idles the transmit line high for the first eight core clocks.
  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 1'b1;
  wire soc_rst = ~por[3];

  // Eight lamps where the A7 has sixteen. A visible heartbeat is the cheapest confirmation that the part
  // is configured and clocking with no host attached at all.
  reg [25:0] hb = 26'd0;
  always @(posedge coreclk) hb <= hb + 1'b1;
  assign led = {hb[25:19], ~por[3]};

  wire eng_rx, eng_tx;
  ft245_uart_bridge #(.CLKS_PER_BIT(81), .W(4)) bridge (
    .clk(coreclk), .rst(soc_rst),
    .eng_rx(eng_rx), .eng_tx(eng_tx),
    .prog_d(prog_d), .prog_rxen(prog_rxen), .prog_txen(prog_txen),
    .prog_rdn(prog_rdn), .prog_wrn(prog_wrn), .prog_oen(prog_oen),
    .prog_siwun(prog_siwun), .prog_spien(prog_spien)
  );

  wpe_soc_fpga #(.CLKS_PER_BIT(81), .WITH_FPU(0)) engine (
    .clk(coreclk), .rst(soc_rst), .uart_rx(eng_rx), .uart_tx(eng_tx)
  );
endmodule
