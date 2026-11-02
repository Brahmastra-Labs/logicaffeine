// nexys_stream64_fast — the 64x64 streaming edge detector with a FAST link: a 50 MHz core clock
// (CLK_DIV=1) and CLKS_PER_BIT=25 → 2.0 Mbaud, an exact FTDI rate (48 MHz / 24). That is ~17x the
// 115200 default, turning the ~200 KB / 62x62 readback from ~17 s into ~1 s. (100 MHz / 4 Mbaud is
// achievable in timing but the open router struggles to close the 9-BRAM design at 10 ns; 50 MHz
// routes comfortably.) Host must open the port at --baud 2000000. Build:
//   TOP=nexys_stream64_fast SRC="nexys_stream64_fast.v nexys_stream_soc.v fabric_sobel3.v" BIT=... openxc7-build.sh
module nexys_stream64_fast (
  input        clk,
  input        rst,
  input        uart_rx,
  output       uart_tx,
  output [15:0] led
);
  nexys_stream_soc #(.IMG_W(64), .IMG_H(64), .CLK_DIV(1), .CLKS_PER_BIT(25)) u (
    .clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx), .led(led)
  );
endmodule
