// nexys_stream32 — a concrete 32x32 top for the parameterized nexys_stream_soc streaming edge
// detector. Instantiates the core at IMG_W=IMG_H=32 (900-pixel output). Build:
//   TOP=nexys_stream32 SRC="nexys_stream32.v nexys_stream_soc.v fabric_sobel3.v" BIT=... openxc7-build.sh
module nexys_stream32 (
  input        clk,
  input        rst,
  input        uart_rx,
  output       uart_tx,
  output [15:0] led
);
  nexys_stream_soc #(.IMG_W(32), .IMG_H(32)) u (
    .clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx), .led(led)
  );
endmodule
