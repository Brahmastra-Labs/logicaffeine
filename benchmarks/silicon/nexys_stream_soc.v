// nexys_stream_soc — STREAM a real IMG_H×IMG_W image tile through LINE BUFFERS and edge-detect it on
// a Nexys A7-100T (Artix-7), for ANY M×N size (parameters IMG_H, IMG_W). The host sends the tile
// (IMG_H*IMG_W bytes, row-major) over UART RX; the board raster-streams it one pixel/clock into a
// depth-(2*IMG_W+3) shift register — that register IS the classic two line buffers plus the
// current-row tap: taps sr[2N+2,2N+1,2N / N+2,N+1,N / 2,1,0] (N=IMG_W) are exactly the sliding 3x3
// window (rows r-2..r, cols c-2..c). Each valid interior window feeds `fabric_sobel3`, and the
// (IMG_H-2)*(IMG_W-2) edge pixels are captured, then streamed back out the UART as 13-byte records
// {0xA5, out_index[15:0] MSB-first, result[63:0] MSB-first, cycles[15:0]}. Runtime image data (not a
// baked constant) keeps the line buffers and stencil real silicon; a /4 (25 MHz) BUFG core clock +
// power-on reset gives the silicon-proven UART clocking. Reusable: a new incoming byte during emit
// reloads and reprocesses. Host: stream-host.py / stream-battery.py send tiles and check == the VM's
// Sobel. Streaming CV on silicon at one pixel per clock, any resolution the fabric fits.
module nexys_stream_soc #(
  parameter CLKS_PER_BIT = 217,
  parameter IMG_W = 16,           // image columns (N)
  parameter IMG_H = 16,           // image rows    (M)
  parameter MEM_STYLE = "block",  // obuf frame buffer: "block" (BRAM, scales to large images) or
                                  // "registers"/"distributed". The read is synchronous either way.
  parameter CLK_DIV = 2           // core clock = input clk >> CLK_DIV (0→100 MHz, 1→50, 2→25, 3→12.5).
                                  // The LINK SPEED is (clk >> CLK_DIV) / CLKS_PER_BIT baud — raise the
                                  // clock (lower CLK_DIV) and/or lower CLKS_PER_BIT to go faster.
) (
  input        clk,
  input        rst,
  input        uart_rx,
  output reg   uart_tx,
  output [15:0] led
);
  // Core clock = input clk / 2^CLK_DIV, always through a global buffer (a BUFG'd clock is what makes
  // the timing-critical RX reliable — an unbuffered pin clock corrupts it). CLK_DIV=0 buffers clk
  // directly (100 MHz); CLK_DIV=k divides by 2^k first.
  wire coreclk;
  generate
    if (CLK_DIV == 0) begin : g_coreclk
      BUFG bufg_core (.I(clk), .O(coreclk));
    end else begin : g_coreclk
      reg [CLK_DIV-1:0] dc = 0;
      always @(posedge clk) dc <= dc + 1'b1;
      BUFG bufg_core (.I(dc[CLK_DIV-1]), .O(coreclk));
    end
  endgenerate

  // power-on reset: hold rstb high for the first 8 core cycles so the SoC self-starts with no switch.
  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 4'd1;
  wire rstb = ~por[3];

  localparam NPIX  = IMG_W * IMG_H;
  localparam NOUT  = (IMG_H - 2) * (IMG_W - 2);
  localparam DEPTH = 2 * IMG_W + 3;          // line-buffer length: two full rows + the 3 current taps
  // 3x3 window taps into sr (bottom-right of the window is the most-recently-shifted pixel, sr[0]).
  localparam T0 = 2*IMG_W+2, T1 = 2*IMG_W+1, T2 = 2*IMG_W;   // window row r-2, cols c-2,c-1,c
  localparam T3 = IMG_W+2,   T4 = IMG_W+1,   T5 = IMG_W;      // window row r-1
  localparam T6 = 2,         T7 = 1,         T8 = 0;          // window row r

  // Image buffer + frame buffer filled from UART RX. Runtime data → the whole pipeline stays real
  // hardware. Both big memories use MEM_STYLE (block RAM by default, so they scale to large images)
  // and are read SYNCHRONOUSLY (img_q / obuf_q): a registered read is what block RAM requires AND is
  // inherently free of the async-LUTRAM read hazard. sr is a small fixed-tap shift register (registers).
  (* ram_style = MEM_STYLE   *) reg [7:0]        imgmem [0:NPIX-1];
  (* ram_style = "registers" *) reg signed [63:0] sr    [0:DEPTH-1];   // the line-buffer shift register
  // obuf holds the edge magnitude |Gx|+|Gy| ∈ [0, 2040] (11 bits). The open flow's block RAM drops
  // data bit 8 (the byte-parity / 9th-bit position of a RAMB36 word) at any width. Workaround: PACK
  // the value with a dummy 0 reserved at word bit 8 — {v[10:8], 1'b0, v[7:0]} — so the only bit that
  // gets dropped is the dummy. On read the value is {q[11:9], q[7:0]}. A 12-bit word, still block RAM.
  (* ram_style = MEM_STYLE   *) reg [11:0]       obuf  [0:NOUT-1];    // captured edge frame buffer (packed)
  reg [7:0]  img_q;                  // synchronous read of imgmem: pixel fetched one cycle earlier
  reg [11:0] obuf_q;                 // synchronous read of obuf for the emit path

  reg [15:0] idx;                    // stream pixel index (0..NPIX)
  reg [15:0] oidx;                   // captured output count (0..NOUT)
  reg [15:0] rx_idx;                 // pixels loaded so far (0..NPIX)
  reg [1:0]  phase;                  // 0 = RX-load, 1 = stream+capture, 2 = UART emit
  localparam PH_RX = 2'd0, PH_STREAM = 2'd1, PH_EMIT = 2'd2;
  integer k;

  wire signed [63:0] fab_out;
  fabric_sobel3 u_fab(.in0(sr[T0]), .in1(sr[T1]), .in2(sr[T2]),
                      .in3(sr[T3]), .in4(sr[T4]), .in5(sr[T5]),
                      .in6(sr[T6]), .in7(sr[T7]), .in8(sr[T8]), .result(fab_out));

  // The synchronous imgmem read adds one pipeline cycle, so sr[0] this cycle holds pixel (idx-2) and
  // the window in `sr` ENDS at that pixel.
  wire [15:0] wk = idx - 16'd2;
  wire [15:0] wr = wk / IMG_W;       // row = (idx-2) / N   (constant divisor → shift/mask when N is 2^k)
  wire [15:0] wc = wk % IMG_W;       // col = (idx-2) % N
  wire        win_valid = (phase == PH_STREAM) && (idx >= 16'd2) && (wr >= 16'd2) && (wc >= 16'd2);

  // ---- UART RX (8N1, mid-bit sampling), 2FF-synchronized ----
  reg        rx_m, rx_s;
  reg [1:0]  rxst;
  localparam RX_IDLE = 2'd0, RX_START = 2'd1, RX_DATA = 2'd2, RX_STOP = 2'd3;
  reg [15:0] rx_cnt;
  reg [2:0]  rx_bi;
  reg [7:0]  rx_sh, rx_byte;
  reg        rx_done;

  // ---- UART TX record sequencer for the EMIT phase (13 bytes/output pixel) ----
  localparam [15:0] CYCLES = 16'd1;
  localparam S_LOAD = 3'd0, S_START = 3'd1, S_DATA = 3'd2, S_STOP = 3'd3;
  reg [2:0]  state;
  reg [15:0] clk_cnt;
  reg [2:0]  bit_idx;
  reg [7:0]  tx_byte;
  reg [3:0]  pos;                    // byte position within the 13-byte record (0..12)
  reg [15:0] evec;                   // which output pixel is being emitted (0..NOUT-1)
  assign led = {phase, idx[3:0], oidx[3:0], state, 3'd0};
  wire [10:0] val11   = {obuf_q[11:9], obuf_q[7:0]};   // unpack the reserved-bit-8 layout
  wire [63:0] emit_val = {53'd0, val11};               // zero-extend the edge value to 64 bits
  wire [7:0]  res_byte = emit_val[(10 - pos) * 8 +: 8]; // pos 3..10 → val MSB..LSB (registered read)

  // One synchronous read port on the frame buffer. evec is stable for a whole 13-byte record (each
  // byte spans CLKS_PER_BIT cycles), so obuf_q is long settled before the record's data bytes emit.
  always @(posedge coreclk) obuf_q <= obuf[evec];

  // Synchronous read port on the image buffer: img_q holds imgmem[idx] from one cycle ago, i.e. pixel
  // (idx-1), shifted into the line buffer next cycle.
  always @(posedge coreclk) img_q <= imgmem[idx];

  initial begin
    phase = PH_RX; idx = 16'd0; oidx = 16'd0; rx_idx = 16'd0; uart_tx = 1'b1;
    rxst = RX_IDLE; rx_cnt = 16'd0; rx_bi = 3'd0; rx_sh = 8'd0; rx_byte = 8'd0; rx_done = 1'b0;
    rx_m = 1'b1; rx_s = 1'b1;
    state = S_LOAD; clk_cnt = 16'd0; bit_idx = 3'd0; pos = 4'd0; evec = 16'd0; tx_byte = 8'd0;
    obuf_q = 12'd0; img_q = 8'd0;
    for (k = 0; k < DEPTH; k = k + 1) sr[k]   = 64'sd0;
    for (k = 0; k < NOUT;  k = k + 1) obuf[k] = 12'd0;
    for (k = 0; k < NPIX;  k = k + 1) imgmem[k] = 8'd0;
  end

  // ---- UART RX FSM: recover one byte per host frame, pulse rx_done ----
  always @(posedge coreclk) begin
    rx_m <= uart_rx; rx_s <= rx_m;   // synchronize the asynchronous serial line
    rx_done <= 1'b0;
    if (rst || rstb) begin rxst <= RX_IDLE; rx_cnt <= 16'd0; rx_bi <= 3'd0; end
    else case (rxst)
      RX_IDLE:  if (~rx_s) begin rx_cnt <= 16'd0; rxst <= RX_START; end
      RX_START: if (rx_cnt == (CLKS_PER_BIT >> 1)) begin
                  if (~rx_s) begin rx_cnt <= 16'd0; rx_bi <= 3'd0; rxst <= RX_DATA; end
                  else rxst <= RX_IDLE;                     // false start — line bounced high
                end else rx_cnt <= rx_cnt + 16'd1;
      RX_DATA:  if (rx_cnt == CLKS_PER_BIT - 1) begin
                  rx_cnt <= 16'd0; rx_sh <= {rx_s, rx_sh[7:1]};   // LSB-first
                  if (rx_bi == 3'd7) rxst <= RX_STOP; else rx_bi <= rx_bi + 3'd1;
                end else rx_cnt <= rx_cnt + 16'd1;
      RX_STOP:  if (rx_cnt == CLKS_PER_BIT - 1) begin
                  rx_cnt <= 16'd0; rx_byte <= rx_sh; rx_done <= 1'b1; rxst <= RX_IDLE;
                end else rx_cnt <= rx_cnt + 16'd1;
    endcase
  end

  // ---- Main FSM: load the image, stream it through the line buffers, emit the edge tile ----
  always @(posedge coreclk) begin
   if (rst || rstb) begin
    phase<=PH_RX; idx<=16'd0; oidx<=16'd0; rx_idx<=16'd0; uart_tx<=1'b1;
    state<=S_LOAD; clk_cnt<=16'd0; bit_idx<=3'd0; pos<=4'd0; evec<=16'd0; tx_byte<=8'd0;
    for (k=0;k<DEPTH;k=k+1) sr[k] <= 64'sd0;
   end else case (phase)
    PH_RX: begin
     uart_tx <= 1'b1;                 // idle the TX line high while loading
     if (rx_done) begin
      imgmem[rx_idx] <= rx_byte;
      rx_idx <= rx_idx + 16'd1;
      if (rx_idx == NPIX - 1) begin phase<=PH_STREAM; idx<=16'd0; oidx<=16'd0; end
     end
    end
    PH_STREAM: begin
     uart_tx <= 1'b1;
     // capture the (combinational) edge result for the window that ends at pixel (idx-2)
     if (win_valid) begin obuf[oidx] <= {fab_out[10:8], 1'b0, fab_out[7:0]}; oidx <= oidx + 16'd1; end
     // shift the synchronously-fetched pixel (idx-1) from img_q into the line-buffer register
     if (idx >= 16'd1 && idx <= NPIX) begin
      for (k=DEPTH-1;k>0;k=k-1) sr[k] <= sr[k-1];
      sr[0] <= {56'd0, img_q};
     end
     idx <= idx + 16'd1;
     if (idx == NPIX + 1) begin phase<=PH_EMIT; state<=S_LOAD; pos<=4'd0; evec<=16'd0; end
    end
    PH_EMIT: if (rx_done) begin
     // a new image transfer has begun — reload and process it (reusable streaming processor).
     imgmem[16'd0] <= rx_byte; rx_idx <= 16'd1;
     phase <= PH_RX; idx <= 16'd0; oidx <= 16'd0; uart_tx <= 1'b1;
     for (k=0;k<DEPTH;k=k+1) sr[k] <= 64'sd0;
    end else begin
     // stream the NOUT captured edge pixels, looping so the host can frame-lock.
     case (state)
      S_LOAD: begin
       case (pos)
        4'd0:  tx_byte<=8'hA5;
        4'd1:  tx_byte<=evec[15:8];
        4'd2:  tx_byte<=evec[7:0];
        4'd11: tx_byte<=CYCLES[15:8];
        4'd12: tx_byte<=CYCLES[7:0];
        default: tx_byte<=res_byte;         // pos 3..10 → the 8 result bytes, MSB first
       endcase
       clk_cnt<=16'd0; bit_idx<=3'd0; state<=S_START;
      end
      S_START: begin uart_tx<=1'b0; if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; state<=S_DATA; end else clk_cnt<=clk_cnt+16'd1; end
      S_DATA: begin uart_tx<=tx_byte[bit_idx];
       if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; if (bit_idx==3'd7) state<=S_STOP; else bit_idx<=bit_idx+3'd1; end else clk_cnt<=clk_cnt+16'd1;
      end
      S_STOP: begin uart_tx<=1'b1;
       if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0;
        if (pos==4'd12) begin
         pos<=4'd0;
         if (evec==NOUT-1) evec<=16'd0; else evec<=evec+16'd1;
         state<=S_LOAD;
        end else begin pos<=pos+4'd1; state<=S_LOAD; end
       end else clk_cnt<=clk_cnt+16'd1;
      end
      default: state<=S_LOAD;
     endcase
    end
    default: phase<=PH_RX;
   endcase
  end
endmodule
