// nexys_video_fabric_soc — the reconfigurable fabric SoC on Board A (Nexys Video, XC7A200T).
//
// The fabric and the vector ROM underneath are `nexys_fabric_soc`, unchanged, the same file the Nexys A7
// build uses. What differs is how its bytes reach the host — and this board does not have a UART.
//
// The Nexys Video's FT2232H channel A is an ASYNC PARALLEL FIFO (DPTI): `prog_d[7:0]` plus handshake, in
// bank 14. The serial pins a Nexys A7 uses go nowhere here. An earlier version of this file drove
// `uart_tx` on AA19; it packed, placed, routed 14,033 of 14,033 nets, configured, assembled and loaded
// without a single complaint, and the board said nothing, because nothing on the far side of that pin
// was listening. `ft245_uart_bridge` gives the SoC the serial line it expects and carries those bits
// over the FIFO instead.
module nexys_video_fabric_soc (
  input        clk,                 // R4, 100 MHz
  inout  [7:0] prog_d,
  input        prog_rxen,
  input        prog_txen,
  output       prog_rdn,
  output       prog_wrn,
  output       prog_oen,
  output       prog_siwun,
  output       prog_spien,
  output [7:0] led
);
  // /128 -> 781.25 kHz, the same deeply conservative core clock the bytecode engine uses on this board,
  // so the bridge and the SoC agree on one rate (781.25e3 / 9600 = 81).
  reg [6:0] divcnt = 7'd0;
  always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire coreclk;
  BUFG bufg_core (.I(divcnt[6]), .O(coreclk));

  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 1'b1;
  wire soc_rst = ~por[3];

  wire eng_rx, eng_tx;
  ft245_uart_bridge #(.CLKS_PER_BIT(81), .W(4)) bridge (
    .clk(coreclk), .rst(soc_rst),
    .eng_rx(eng_rx), .eng_tx(eng_tx),
    .prog_d(prog_d), .prog_rxen(prog_rxen), .prog_txen(prog_txen),
    .prog_rdn(prog_rdn), .prog_wrn(prog_wrn), .prog_oen(prog_oen),
    .prog_siwun(prog_siwun), .prog_spien(prog_spien)
  );

  // The SoC drives `led = {state, vec[4:0], 8'd0}`; this board's eight lamps take the informative half.
  wire [15:0] lamps;
  assign led = lamps[15:8];

  nexys_fabric_soc #(.CLKS_PER_BIT(81)) u_soc (
    .clk(coreclk), .rst(soc_rst), .uart_tx(eng_tx), .led(lamps)
  );
endmodule
