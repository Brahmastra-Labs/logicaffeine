// ours_fifo_tx — the Board-A (Nexys Video, XC7A200T) bring-up vehicle for OUR place-and-route.
//
// Board A's byte channel is the FT2232H async FIFO (DPTI), not a UART, so this drives the FT245
// write handshake: whenever the FIFO reports room (TXE# low) it presents a byte and strobes WR#.
// The byte alternates 0x00 / 0xFF, so a host that reads BOTH values has proof the design is
// sequencing — neither a floating bus nor a stuck driver produces that.
//
// SIZED TO EXACTLY ONE SLICE, and that is the whole point. We synthesize `-noclkbuf`, so the clock
// rides ordinary fabric routing rather than the global spine (BUFH / CLK_HROW / HCLK), which our
// backend cannot yet emit. On the other board a THREE-FLOP, ONE-SLICE design runs happily that way
// at 100 MHz, while versions here spanning three, six and twelve slices did not — the difference
// being clock skew across slices at a 10 ns period. So this is trimmed to fit a single slice's
// budget of 8 flip-flops and 4 LUTs:
//
//     ring   4 flops  (rotating one-hot; rotation is pure wiring, no logic)
//     phase  1 flop   (strobe low / release)
//     wrn    1 flop
//     tog    1 flop   (the data byte is eight copies of it)
//
// No counters anywhere, so nothing infers a CARRY4 our resolver cannot place. TXE# is sampled
// directly rather than through a synchronizer — two flops we cannot spare, and acceptable for a
// vehicle whose only job is to prove the bitstream works.
module ours_fifo_tx (
  input        clk,            // 100 MHz (R4)
  output [7:0] prog_d,
  input        prog_txen,      // TXE#: low = there is room for a byte
  output reg   prog_wrn  = 1'b1,
  output       prog_rdn,
  output       prog_oen,
  output       prog_siwun,
  output       prog_spien
);
  assign prog_rdn   = 1'b1;    // we never read
  assign prog_oen   = 1'b1;    // so the FTDI must never drive the bus
  assign prog_siwun = 1'b1;
  assign prog_spien = 1'b1;    // select DPTI (parallel FIFO)

  reg tog = 1'b0;
  assign prog_d = {8{tog}};    // 0x00 / 0xFF — one flop drives all eight pins

  // 4-deep rotating one-hot ring: a tick every 4 clocks (40 ns), past the FT245's 30 ns minimum
  // WR# pulse.
  reg [3:0] ring = 4'b0001;
  always @(posedge clk) ring <= {ring[2:0], ring[3]};
  wire tick = ring[3];

  reg phase = 1'b0;
  always @(posedge clk) if (tick) begin
    if (!phase) begin
      if (!prog_txen) begin prog_wrn <= 1'b0; phase <= 1'b1; end
    end else begin
      prog_wrn <= 1'b1;
      tog      <= ~tog;
      phase    <= 1'b0;
    end
  end
endmodule
