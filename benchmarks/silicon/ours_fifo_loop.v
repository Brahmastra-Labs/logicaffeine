// ours_fifo_loop — the Board-A (Nexys Video, XC7A200T) counterpart of ours_loopback.
//
// Board A's byte channel is not a UART: it is the FT2232H ASYNC FIFO (DPTI), so a host write only
// moves if the FPGA performs the RXF#/RD#/OE# read handshake and then hands the byte back with
// TXE#/WR#. A design that ignores those pins makes the host's write block forever — which is
// exactly what a naive register-chain loopback does here.
//
// Deliberately arithmetic-free: the state is ONE-HOT and the inter-phase delay is a rotating ring
// of flip-flops rather than a counter, so nothing infers a CARRY4. Our engine does not model carry
// chains yet, and a design it cannot place is no use as a bring-up vehicle.
module ours_fifo_loop (
  input        clk,
  inout  [7:0] prog_d,
  input        prog_rxen,      // RXF#: low = a byte is waiting for us
  input        prog_txen,      // TXE#: low = there is room for a byte
  output reg   prog_rdn  = 1'b1,
  output reg   prog_wrn  = 1'b1,
  output reg   prog_oen  = 1'b1,
  output       prog_siwun,
  output       prog_spien
);
  assign prog_siwun = 1'b1;
  assign prog_spien = 1'b1;    // select DPTI (parallel FIFO)

  reg rxf1 = 1'b1, rxf2 = 1'b1, txe1 = 1'b1, txe2 = 1'b1;
  always @(posedge clk) begin
    rxf1 <= prog_rxen; rxf2 <= rxf1;
    txe1 <= prog_txen; txe2 <= txe1;
  end
  wire rx_avail = ~rxf2;
  wire tx_space = ~txe2;

  reg [7:0] dbus  = 8'd0;
  reg       drive = 1'b0;
  assign prog_d = drive ? dbus : 8'bzzzzzzzz;

  // A 16-deep rotating one-hot ring: `tick` is high once every 16 clocks (160 ns at 100 MHz),
  // comfortably beyond the FT245's ~50 ns strobe requirements, and costs 16 flip-flops, no logic.
  reg [15:0] ring = 16'h0001;
  always @(posedge clk) ring <= {ring[14:0], ring[15]};
  wire tick = ring[15];

  // One-hot state: IDLE, OE, RD, CAP, RDHI, WRWAIT, WRLO, WRHI.
  reg [7:0] st = 8'b0000_0001;
  always @(posedge clk) if (tick) begin
    case (1'b1)
      st[0]: if (rx_avail) begin prog_oen <= 1'b0;                st <= 8'b0000_0010; end
      st[1]: begin               prog_rdn <= 1'b0;                st <= 8'b0000_0100; end
      st[2]: begin               dbus     <= prog_d;              st <= 8'b0000_1000; end
      st[3]: begin               prog_rdn <= 1'b1;                st <= 8'b0001_0000; end
      st[4]: begin               prog_oen <= 1'b1;                st <= 8'b0010_0000; end
      st[5]: if (tx_space) begin drive    <= 1'b1;                st <= 8'b0100_0000; end
      st[6]: begin               prog_wrn <= 1'b0;                st <= 8'b1000_0000; end
      st[7]: begin               prog_wrn <= 1'b1; drive <= 1'b0; st <= 8'b0000_0001; end
      default:                                                    st <= 8'b0000_0001;
    endcase
  end
endmodule
