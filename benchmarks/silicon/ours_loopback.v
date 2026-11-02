// ours_loopback — the first design built end-to-end by OUR place-and-route engine.
//
// Chosen so its correctness is OBSERVABLE OVER THE WIRE: a host sends bytes, the design registers
// uart_rx through fabric flip-flops and drives uart_tx from it, and the bytes come back. That is a
// far stronger statement than "the FPGA accepted the configuration" — it proves the clock reaches
// the flops, both IO pins are configured, and the routing between them is real.
//
// Deliberately minimal in PRIMITIVES, not in ambition: three flip-flops, no counters, so no CARRY4
// chains (whose pins our resolver does not model yet) and no clock divider. Running the register
// chain at the raw 100 MHz costs 3 cycles = 30 ns of delay, which against a 9600-baud bit time of
// 104 us is nothing.
module ours_loopback (
  input      clk,
  input      uart_rx,
  output reg uart_tx
);
  reg rx1 = 1'b1, rx2 = 1'b1;
  always @(posedge clk) begin
    rx1     <= uart_rx;
    rx2     <= rx1;
    uart_tx <= rx2;
  end
endmodule
