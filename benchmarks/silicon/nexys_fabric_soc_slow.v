// nexys_fabric_soc_slow — the SAME fabric SoC, given time to settle.
//
// `config_fabric` is combinational and SIXTY-FOUR BITS WIDE, and one of the datapaths it becomes is
// integer division. At the board's 100 MHz that whole path — inputs, through the tile grid, through
// the per-tile config muxes, out to the capture register — has ten nanoseconds to resolve, and it
// does not. On silicon the symptom is exact: `clamp` and `sign`, which are a couple of comparisons
// deep, come back bit-perfect every single lap, while `ratio` and `affine` come back with their low
// bits right and their high bits DIFFERENT ON EVERY LAP. Varying garbage is the signature of a
// setup violation, not of a wrong bit in the bitstream — a miswired design is wrong the same way
// every time.
//
// So this wrapper changes exactly one thing: it divides the board clock by a hundred and
// twenty-eight and scales the UART divisor to match, giving the same logic one and a quarter
// MICROseconds to settle in instead of ten nanoseconds. Nothing about the datapath, the framing or the vectors moves. If the results come back
// correct here and only here, the bitstream was right all along and what is missing upstream is a
// timing model — our placer minimises wirelength and knows nothing about critical paths yet.
module nexys_fabric_soc_slow (
  input         clk,
  input         rst,
  output        uart_tx,
  output [15:0] led
);
  // 100 MHz / 128 = 781.25 kHz, ON A GLOBAL BUFFER — the same idiom the sequencer SoC on this board
  // already uses. The buffer is not decoration. A clock is put on the dedicated network only if
  // every one of its sinks is a flip-flop clock pin, and the divider's own counter bit fails that
  // twice over: it feeds the increment logic as well as the clock pins, so it is not recognised as
  // a clock at all, and a flip-flop is not a clock source the spine can start from. Taking the
  // divided bit through a `BUFG` fixes both — the buffer's output goes nowhere but clock pins, and
  // the buffer is a source the spine knows how to route from.
  reg [6:0] divcnt = 7'd0;
  always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire settled_clk;
  BUFG bufg_settled (.I(divcnt[6]), .O(settled_clk));

  // 781250 / 9600 = 81.4, so 81 — a half-percent short, which is nothing over ten bits. Dividing
  // for 115200 instead would land on 6.8 and have to be rounded to 7, three per cent long, and
  // there is no reason to spend the margin: the whole result stream is a few hundred bytes.
  nexys_fabric_soc #(.CLKS_PER_BIT(81)) u_soc (
    .clk(settled_clk),
    .rst(rst),
    .uart_tx(uart_tx),
    .led(led)
  );
endmodule
