// hart_qspi — the streaming reader of the board's configuration flash (an S25FL128S): a READ (03h) at a 24-bit byte
// address, then byte after byte for as long as the stream lasts, gathered eight to a little-endian word (the first
// byte lowest) into a small queue. When the queue is full the clock waits low before the next word — the flash holds
// its place across a stopped clock. `start` (re)begins a stream at an address and `stop` ends it; either empties the
// queue. The flash's address counter wraps from its last byte to its first.
//
// SPI mode 0, one data line each way. Every bit is a low phase and a high phase of HALF hart cycles each: the command's
// bit is set up as the low phase begins and the flash takes it as the clock rises; the flash's bit is sampled as the
// high phase ends, a whole clock period after the flash launched it on the previous fall. After reset the clock first
// pulses eight times with the chip deselected: the FPGA hands its configuration clock pin to the design only after
// three.
module hart_qspi #(
  parameter HALF = 2
) (
  input             clk,
  input             rst,
  input             start,
  input      [23:0] start_addr,
  input             stop,
  output            word_valid,
  output     [63:0] word,
  input             word_pop,
  output reg        sck,
  output reg        cs_n,
  output reg        mosi,
  input             miso
);
  localparam [2:0] S_WAKE = 3'd0, S_GAP = 3'd1, S_IDLE = 3'd2, S_CMD = 3'd3, S_DATA = 3'd4;
  localparam TW = $clog2(HALF + 1);
  localparam [7:0] READ = 8'h03;

  reg [2:0]    state;
  reg [TW-1:0] tick;
  reg [4:0]    bits;          // command bits sent, or the bit within the byte
  reg [31:0]   out;
  reg [7:0]    in;
  reg [63:0]   gather;
  reg [2:0]    byte_n;
  reg          pending;       // a stream waits to begin once the chip has been deselected long enough
  reg [23:0]   pending_addr;

  wire       word_empty, queue_full;
  wire [7:0] byte_in = {in[6:0], miso};
  wire       phase_end = tick == HALF - 1;
  wire       push = state == S_DATA && sck && phase_end && bits == 5'd7 && byte_n == 3'd7;
  hart_fifo #(.WIDTH(64), .DEPTH_BITS(2)) queue (
    .clk(clk), .rst(rst), .clear(start || stop), .push(push), .din({byte_in, gather[63:8]}),
    .pop(word_pop), .head(word), .empty(word_empty), .full(queue_full));
  assign word_valid = !word_empty;

  // A word begins only while the queue has room for it.
  wire hold = state == S_DATA && bits == 5'd0 && byte_n == 3'd0 && queue_full;

  always @(posedge clk) begin
    if (rst) begin
      state <= S_WAKE;
      sck <= 1'b0;
      cs_n <= 1'b1;
      mosi <= 1'b0;
      tick <= 0;
      bits <= 5'd0;
      pending <= 1'b0;
    end else if ((start || stop) && state != S_WAKE) begin
      sck <= 1'b0;
      cs_n <= 1'b1;
      tick <= 0;
      bits <= 5'd0;
      pending <= start;
      pending_addr <= start_addr;
      state <= S_GAP;
    end else begin
      if (start || stop) begin
        pending <= start;
        pending_addr <= start_addr;
      end
      tick <= phase_end ? {TW{1'b0}} : tick + 1'b1;
      case (state)
        S_WAKE: if (phase_end) begin
          sck <= !sck;
          bits <= bits + 1'b1;
          if (bits == 5'd15) begin
            bits <= 5'd0;
            state <= S_GAP;
          end
        end
        // The chip stays deselected for a whole clock period, then the pending stream's command begins.
        S_GAP: if (phase_end) begin
          if (bits == 5'd0) bits <= 5'd1;
          else if (pending) begin
            pending <= 1'b0;
            cs_n <= 1'b0;
            mosi <= READ[7];
            out <= {READ, pending_addr};
            bits <= 5'd0;
            state <= S_CMD;
          end else state <= S_IDLE;
        end
        S_IDLE: tick <= 0;
        S_CMD: if (phase_end) begin
          if (!sck) sck <= 1'b1;
          else begin
            sck <= 1'b0;
            if (bits == 5'd31) begin
              bits <= 5'd0;
              byte_n <= 3'd0;
              state <= S_DATA;
            end else begin
              bits <= bits + 1'b1;
              mosi <= out[30];
              out <= {out[30:0], 1'b0};
            end
          end
        end
        S_DATA: if (phase_end) begin
          if (!sck) begin
            if (!hold) sck <= 1'b1;
          end else begin
            sck <= 1'b0;
            in <= byte_in;
            if (bits == 5'd7) begin
              bits <= 5'd0;
              gather <= {byte_in, gather[63:8]};
              byte_n <= byte_n + 1'b1;
            end else bits <= bits + 1'b1;
          end
        end
        default: state <= S_IDLE;
      endcase
    end
  end
endmodule
