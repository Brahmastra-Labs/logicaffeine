// ours_lut_loop — Board B loopback WITH a LUT in the data path.
//
// The working `ours_loopback` is three flip-flops and no LUTs, so it never exercised our LUT
// configuration (INIT contents, input-pin mapping, output mux). Board A's design has six LUTs and
// does not run. This isolates the variable on the board where a known-good baseline exists: an
// explicitly instantiated LUT6 that passes I0 through unchanged, so the echo must still work if our
// LUT config is right, and must break if it is not.
module ours_lut_loop (
  input      clk,
  input      uart_rx,
  output reg uart_tx
);
  reg rx1 = 1'b1, rx2 = 1'b1;
  wire mix;
  // O = I0 (identity on input 0, ignoring I1..I5): the upper half of the truth table is 1.
  LUT6 #(.INIT(64'hAAAAAAAAAAAAAAAA)) pass (
    .I0(rx2), .I1(1'b0), .I2(1'b0), .I3(1'b0), .I4(1'b0), .I5(1'b0), .O(mix)
  );
  always @(posedge clk) begin
    rx1     <= uart_rx;
    rx2     <= rx1;
    uart_tx <= mix;
  end
endmodule
