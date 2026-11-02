// nexys_heartbeat — a minimal bring-up probe for the Nexys A7 open-flow bitstream. It continuously
// transmits the byte 0x55 ('U') on uart_tx at 115200 baud from the raw 100 MHz clock (CLKS_PER_BIT=868).
// NO reset dependency (registers use init values), NO UART RX, NO clock divider, NO core. If the host
// sees a stream of 0x55 on the board's UART, then the 100 MHz clock, the UART TX pin (D4), the IO/clock
// routing, and the whole yosys->nextpnr-xilinx->prjxray bitstream flow are all working — isolating any
// remaining problem to the RX path or the core/FSM. rst/uart_rx are declared only so the shared
// nexys_soc.xdc (which pins them) applies unchanged; both are unused.
module nexys_heartbeat (
  input         clk,          // 100 MHz (E3)
  input         rst,          // unused
  input         uart_rx,      // unused
  output reg    uart_tx = 1'b1,
  output [15:0] led
);
  localparam integer CPB = 868;   // 100e6 / 115200
  reg [15:0] cnt = 16'd0;
  reg [3:0]  idx = 4'd0;          // 0=start, 1..8=data(LSB-first), 9=stop
  localparam [7:0] DATA = 8'h55;
  always @(posedge clk) begin
    if (cnt == CPB-1) begin
      cnt <= 16'd0;
      case (idx)
        4'd0:    uart_tx <= 1'b0;             // start bit
        4'd9:    uart_tx <= 1'b1;             // stop bit
        default: uart_tx <= DATA[idx-4'd1];   // data bits, LSB first
      endcase
      idx <= (idx == 4'd9) ? 4'd0 : idx + 4'd1;
    end else cnt <= cnt + 16'd1;
  end
  assign led = 16'hAAAA;
endmodule
