// nexys_wpe_loopback — a bring-up diagnostic for the over-the-wire engine SoC. It reproduces EXACTLY the
// engine's infrastructure — the /128 BUFG core clock, the power-on reset, and uart_rx registered through fabric
// flip-flops (so the input uses the same IOI route the engine's RX synchronizer does) — then drives uart_tx
// from it (a registered loopback). If a host sees its bytes echoed back, the clock + both UART pins + the
// patched input route all work, and any no-reply from the engine is engine LOGIC. If nothing echoes, the
// infrastructure itself (clock / I/O routing) is the fault. Same pins/XDC as nexys_wpe_soc.
module nexys_wpe_loopback (
  input         clk,
  input         rst,
  input         uart_rx,
  output reg    uart_tx,
  output [15:0] led
);
  reg [6:0] divcnt = 7'd0;
  always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire coreclk;
  BUFG bufg_core (.I(divcnt[6]), .O(coreclk));

  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 1'b1;

  reg rx1, rx2;
  always @(posedge coreclk) begin
    rx1 <= uart_rx;
    rx2 <= rx1;
    uart_tx <= por[3] ? rx2 : 1'b1;   // idle high during POR, then echo the (registered) input
  end

  reg [24:0] hb = 25'd0;
  always @(posedge coreclk) hb <= hb + 1'b1;
  assign led = {hb[24:10], por[3]};
endmodule
