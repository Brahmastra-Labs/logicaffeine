// nexys_stream64 — a concrete 64x64 top for the parameterized nexys_stream_soc streaming edge
// detector. Instantiates the core at IMG_W=IMG_H=64 (62x62 = 3844-pixel output). Build:
//   TOP=nexys_stream64 SRC="nexys_stream64.v nexys_stream_soc.v fabric_sobel3.v" BIT=... openxc7-build.sh
module nexys_stream64 (
  input        clk,
  input        rst,
  input        uart_rx,
  output       uart_tx,
  output [15:0] led
);
  nexys_stream_soc #(.IMG_W(64), .IMG_H(64)) u (
    .clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx), .led(led)
  );
endmodule
