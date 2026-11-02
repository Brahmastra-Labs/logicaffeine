// nexys_video_bridge_loopback — validates ft245_uart_bridge standalone. The bridge's serial feed (eng_rx) is
// tied to its serial read (eng_tx), so a host byte streams in over the FT245 FIFO, gets serialized, looped,
// deserialized, and written back — echoing over /dev/ttyUSB iff the whole bridge (FT245 PHY + both UART FSMs)
// works. Once green, the same bridge wraps the real wpe_soc_fpga engine unchanged.
module nexys_video_bridge_loopback (
  input clk,
  inout [7:0] prog_d,
  input  prog_rxen, input prog_txen,
  output prog_rdn, output prog_wrn, output prog_oen, output prog_siwun, output prog_spien,
  output [7:0] led
);
  reg [1:0] dc = 2'd0; always @(posedge clk) dc <= dc + 1'b1;
  wire coreclk; BUFG b (.I(dc[1]), .O(coreclk));         // /4 -> 25 MHz
  reg [3:0] por = 4'd0; always @(posedge coreclk) if (!por[3]) por <= por + 1'b1;
  wire rstb = ~por[3];
  reg [24:0] hb = 25'd0; always @(posedge coreclk) hb <= hb + 1'b1;

  wire eng_rx_w, eng_tx_w;
  assign eng_tx_w = eng_rx_w;                            // loopback

  ft245_uart_bridge #(.CLKS_PER_BIT(16), .W(4)) br (
    .clk(coreclk), .rst(rstb),
    .eng_rx(eng_rx_w), .eng_tx(eng_tx_w),
    .prog_d(prog_d), .prog_rxen(prog_rxen), .prog_txen(prog_txen),
    .prog_rdn(prog_rdn), .prog_wrn(prog_wrn), .prog_oen(prog_oen),
    .prog_siwun(prog_siwun), .prog_spien(prog_spien)
  );
  assign led = {7'd0, hb[24]};
endmodule
