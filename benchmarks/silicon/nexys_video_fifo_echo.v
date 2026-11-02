// nexys_video_fifo_echo — Board-A byte I/O over the FT2232H ASYNC FIFO (Digilent DPTI), which is what the
// Nexys Video exposes instead of a serial UART. The host still reads/writes bytes on /dev/ttyUSB; the FPGA
// speaks the FT245 async-FIFO handshake here. This echoes every received byte, proving the on-board byte
// path works (the equivalent of the UART echo, but over the FIFO). led[0] = heartbeat, led[7:1] = last byte.
//
// FT245 async FIFO (FT2232H): RXF#(rxen) low => a byte is available to read; TXE#(txen) low => space to
// write. Read: OE# low (bus turnaround), then RD# low, sample D, RD#/OE# high. Write: drive D, WR# low then
// high (data committed on WR# rising edge). SPIEN high selects DPTI (parallel), SIWU# high = no flush.
module nexys_video_fifo_echo (
  input clk,
  inout [7:0] prog_d,
  input  prog_rxen,        // RXF#  (low = data to read)
  input  prog_txen,        // TXE#  (low = space to write)
  output reg prog_rdn = 1'b1,
  output reg prog_wrn = 1'b1,
  output reg prog_oen = 1'b1,
  output prog_siwun,       // SIWU# tie high
  output prog_spien,       // DPTI select = high
  output [7:0] led
);
  assign prog_siwun = 1'b1;
  assign prog_spien = 1'b1;

  reg [25:0] hb = 0;
  always @(posedge clk) hb <= hb + 1'b1;

  // synchronize the async FTDI flags
  reg rxf1, rxf2, txe1, txe2;
  always @(posedge clk) begin rxf1 <= prog_rxen; rxf2 <= rxf1; txe1 <= prog_txen; txe2 <= txe1; end
  wire rx_avail =  ~rxf2;   // data available to read
  wire tx_space = ~txe2;    // space to write

  reg drive_bus = 1'b0;
  reg [7:0] tx_data = 8'd0;
  assign prog_d = drive_bus ? tx_data : 8'bzzzzzzzz;

  reg [7:0] last = 8'd0;
  assign led = {last[7:1], hb[25]};

  localparam S_IDLE=4'd0, S_RD_OE=4'd1, S_RD_LOW=4'd2, S_RD_CAP=4'd3, S_RD_HIGH=4'd4,
             S_WR_WAIT=4'd5, S_WR_LOW=4'd6, S_WR_HIGH=4'd7, S_WR_END=4'd8;
  reg [3:0] st = S_IDLE;
  reg [3:0] cnt = 0;
  localparam integer W = 4;   // ~40 ns per phase at 100 MHz — comfortably above FT2232H async minimums

  always @(posedge clk) begin
    case (st)
      S_IDLE: begin
        prog_rdn <= 1'b1; prog_wrn <= 1'b1; prog_oen <= 1'b1; drive_bus <= 1'b0; cnt <= 0;
        if (rx_avail) st <= S_RD_OE;
      end
      S_RD_OE:  begin prog_oen <= 1'b0; if (cnt==W) begin cnt<=0; st<=S_RD_LOW; end else cnt<=cnt+1'b1; end
      S_RD_LOW: begin prog_rdn <= 1'b0; if (cnt==W) begin cnt<=0; st<=S_RD_CAP; end else cnt<=cnt+1'b1; end
      S_RD_CAP: begin last <= prog_d; tx_data <= prog_d; st <= S_RD_HIGH; cnt <= 0; end
      S_RD_HIGH:begin prog_rdn <= 1'b1; prog_oen <= 1'b1; if (cnt==W) begin cnt<=0; st<=S_WR_WAIT; end else cnt<=cnt+1'b1; end
      S_WR_WAIT:begin drive_bus <= 1'b1; if (tx_space) st <= S_WR_LOW; cnt <= 0; end
      S_WR_LOW: begin prog_wrn <= 1'b0; if (cnt==W) begin cnt<=0; st<=S_WR_HIGH; end else cnt<=cnt+1'b1; end
      S_WR_HIGH:begin prog_wrn <= 1'b1; if (cnt==W) begin cnt<=0; st<=S_WR_END; end else cnt<=cnt+1'b1; end
      S_WR_END: begin drive_bus <= 1'b0; st <= S_IDLE; end
      default:  st <= S_IDLE;
    endcase
  end
endmodule
