// nexys_soc — the full 113-op LOGOS soft-core made BOARD-RUNNABLE on the Nexys A7.
//
// Over a single UART (8N1, LSB-first, 115200 @ 100 MHz -> CLKS_PER_BIT = 868) a host:
//   1. sends one length byte N (number of 32-bit instruction words, 1..64),
//   2. sends N*4 program bytes, MSB-first per word ([opcode][dst][a][b]),
// and the SoC writes them into the core's imem (via the prog_we load port) while holding the
// core in reset. When the last word lands it releases reset; the core runs the loaded program.
// Every `Show` (out_valid) pushes its 32-bit value into a capture FIFO. When the core halts the
// SoC streams the results back: 0xA5 sync, one count byte M, then M*4 result bytes (MSB-first),
// then a 4-byte run-cycle count (MSB-first, cycles the core ran before halting), then 0x5A end.
// `scripts/nexys-host.py` drives the whole round-trip and checks it against the VM.
//
// External pins fit the Nexys A7: clk, rst, uart_rx, uart_tx, led[15:0]. CLKS_PER_BIT is a
// parameter so the testbench can shrink a UART bit to a few cycles.
//
// Enables soft_core's guarded program-load port. This file must be compiled BEFORE soft_core.v
// (as every nexys_soc build does) so the macro is set when soft_core.v is parsed.
`define SOFT_CORE_PROG_LOAD
module nexys_soc #(parameter CLKS_PER_BIT = 868) (
  input         clk,
  input         rst,          // active-high synchronous reset (board: a slide switch)
  input         uart_rx,      // from the host (PC -> FPGA)
  output reg    uart_tx,      // to the host (FPGA -> PC)
  output [15:0] led
);
  localparam CW = $clog2(CLKS_PER_BIT) + 1;

  // ── the core, held in reset until its program is loaded ─────────────────────────────
  reg         core_rst;
  reg         ld_we;
  reg  [5:0]  ld_addr;
  reg  [31:0] ld_data;
  wire [31:0] core_out;
  wire        core_ov;
  wire        core_halted;

  soft_core u_core (
    .clk(clk), .rst(core_rst),
    .prog_we(ld_we), .prog_addr(ld_addr), .prog_data(ld_data),
    .out(core_out), .out_valid(core_ov), .halted(core_halted)
  );

  // ── UART receiver: strobes rx_valid for one cycle with rx_data on each byte ──────────
  localparam R_IDLE = 2'd0, R_START = 2'd1, R_DATA = 2'd2, R_STOP = 2'd3;
  reg [1:0]     r_state;
  reg [CW-1:0]  r_cnt;
  reg [2:0]     r_bit;
  reg [7:0]     r_shift;
  reg           rx_valid;
  reg [7:0]     rx_data;
  reg           rx_d1, rx_d2;              // two-flop synchronizer for the async rx line

  // ── UART transmitter: pulse tx_start with tx_data; tx_busy high until the byte is out ─
  localparam T_IDLE = 2'd0, T_START = 2'd1, T_DATA = 2'd2, T_STOP = 2'd3;
  reg [1:0]     t_state;
  reg [CW-1:0]  t_cnt;
  reg [2:0]     t_bit;
  reg [7:0]     t_shift;
  reg           tx_start;
  reg [7:0]     tx_data;
  wire          tx_busy = (t_state != T_IDLE) || tx_start;

  // ── loader / runner / sender FSM ─────────────────────────────────────────────────────
  localparam L_LEN = 3'd0, L_PROG = 3'd1, L_RUN = 3'd2, L_SEND = 3'd3, L_DONE = 3'd4;
  reg [2:0]  l_state;
  reg [6:0]  nprog;                        // program length in words (from the length byte)
  reg [5:0]  word_idx;                     // which imem word we are assembling
  reg [1:0]  byte_in_word;                 // 0..3 within the current word (MSB-first)
  reg [31:0] word_accum;

  reg [31:0] show_mem [0:31];              // captured Show values
  reg [5:0]  nshow;                        // how many captured (saturates at 32)
  reg [5:0]  send_idx;                     // which show value we are transmitting
  reg [3:0]  send_ph;                      // sub-phase: 0=sync,1=count,2..5=value bytes,6..9=cycle bytes,10=end
  reg [31:0] cycles;                       // core run-cycle count (frozen at halt), returned in the frame

  assign led = {core_halted, l_state, nshow, 6'd0};   // 1 + 3 + 6 + 6 = 16 bits: status on the LEDs

  integer i;
  always @(posedge clk) begin
    // defaults (single-cycle strobes)
    rx_valid <= 1'b0;
    ld_we    <= 1'b0;
    tx_start <= 1'b0;

    rx_d1 <= uart_rx;  rx_d2 <= rx_d1;      // synchronize

    if (rst) begin
      r_state <= R_IDLE; t_state <= T_IDLE; uart_tx <= 1'b1;
      l_state <= L_LEN; core_rst <= 1'b1; nprog <= 7'd0; word_idx <= 6'd0;
      byte_in_word <= 2'd0; word_accum <= 32'd0; nshow <= 6'd0; send_idx <= 6'd0;
      send_ph <= 4'd0; cycles <= 32'd0; r_cnt <= 0; r_bit <= 3'd0; t_cnt <= 0; t_bit <= 3'd0;
    end else begin
      // ───────── UART RX ─────────
      case (r_state)
        R_IDLE:  if (~rx_d2) begin r_state <= R_START; r_cnt <= 0; end
        R_START: if (r_cnt == (CLKS_PER_BIT-1)/2) begin
                   if (~rx_d2) begin r_cnt <= 0; r_bit <= 3'd0; r_state <= R_DATA; end
                   else r_state <= R_IDLE;                 // false start
                 end else r_cnt <= r_cnt + 1'b1;
        R_DATA:  if (r_cnt == CLKS_PER_BIT-1) begin
                   r_cnt <= 0; r_shift <= {rx_d2, r_shift[7:1]};   // LSB first
                   if (r_bit == 3'd7) r_state <= R_STOP; else r_bit <= r_bit + 1'b1;
                 end else r_cnt <= r_cnt + 1'b1;
        R_STOP:  if (r_cnt == CLKS_PER_BIT-1) begin
                   r_cnt <= 0; rx_valid <= 1'b1; rx_data <= r_shift; r_state <= R_IDLE;
                 end else r_cnt <= r_cnt + 1'b1;
      endcase

      // ───────── UART TX ─────────
      case (t_state)
        T_IDLE:  if (tx_start) begin t_shift <= tx_data; t_state <= T_START; t_cnt <= 0; uart_tx <= 1'b0; end
        T_START: if (t_cnt == CLKS_PER_BIT-1) begin t_cnt <= 0; t_bit <= 3'd0; t_state <= T_DATA; uart_tx <= t_shift[0]; end
                 else t_cnt <= t_cnt + 1'b1;
        T_DATA:  if (t_cnt == CLKS_PER_BIT-1) begin
                   t_cnt <= 0;
                   if (t_bit == 3'd7) begin t_state <= T_STOP; uart_tx <= 1'b1; end
                   else begin t_bit <= t_bit + 1'b1; uart_tx <= t_shift[t_bit+1]; end
                 end else t_cnt <= t_cnt + 1'b1;
        T_STOP:  if (t_cnt == CLKS_PER_BIT-1) begin t_cnt <= 0; t_state <= T_IDLE; uart_tx <= 1'b1; end
                 else t_cnt <= t_cnt + 1'b1;
      endcase

      // ───────── loader / runner / sender ─────────
      case (l_state)
        L_LEN: if (rx_valid) begin
                 nprog <= rx_data[6:0]; word_idx <= 6'd0; byte_in_word <= 2'd0;
                 l_state <= (rx_data[6:0] == 7'd0) ? L_RUN : L_PROG;
               end
        L_PROG: if (rx_valid) begin
                  word_accum <= {word_accum[23:0], rx_data};    // MSB-first assembly
                  if (byte_in_word == 2'd3) begin
                    ld_we <= 1'b1; ld_addr <= word_idx; ld_data <= {word_accum[23:0], rx_data};
                    byte_in_word <= 2'd0;
                    if (word_idx + 1'b1 == nprog) l_state <= L_RUN;
                    else word_idx <= word_idx + 1'b1;
                  end else byte_in_word <= byte_in_word + 1'b1;
                end
        L_RUN: begin
                 core_rst <= 1'b0;                             // release the core
                 if (!core_halted) cycles <= cycles + 1'b1;    // count run cycles until halt
                 if (core_ov && nshow != 6'd32) begin
                   show_mem[nshow[4:0]] <= core_out; nshow <= nshow + 1'b1;
                 end
                 if (core_halted) begin l_state <= L_SEND; send_idx <= 6'd0; send_ph <= 3'd0; end
               end
        L_SEND: if (!tx_busy) begin
                  case (send_ph)
                    4'd0: begin tx_data <= 8'hA5; tx_start <= 1'b1; send_ph <= 4'd1; end             // sync
                    4'd1: begin tx_data <= {2'd0, nshow}; tx_start <= 1'b1;                           // count
                                send_ph <= (nshow == 6'd0) ? 4'd6 : 4'd2; end
                    4'd2: begin tx_data <= show_mem[send_idx[4:0]][31:24]; tx_start <= 1'b1; send_ph <= 4'd3; end
                    4'd3: begin tx_data <= show_mem[send_idx[4:0]][23:16]; tx_start <= 1'b1; send_ph <= 4'd4; end
                    4'd4: begin tx_data <= show_mem[send_idx[4:0]][15:8];  tx_start <= 1'b1; send_ph <= 4'd5; end
                    4'd5: begin tx_data <= show_mem[send_idx[4:0]][7:0];   tx_start <= 1'b1;
                                if (send_idx + 1'b1 == nshow) send_ph <= 4'd6;
                                else begin send_idx <= send_idx + 1'b1; send_ph <= 4'd2; end end
                    4'd6: begin tx_data <= cycles[31:24]; tx_start <= 1'b1; send_ph <= 4'd7; end       // run-cycle count
                    4'd7: begin tx_data <= cycles[23:16]; tx_start <= 1'b1; send_ph <= 4'd8; end
                    4'd8: begin tx_data <= cycles[15:8];  tx_start <= 1'b1; send_ph <= 4'd9; end
                    4'd9: begin tx_data <= cycles[7:0];   tx_start <= 1'b1; send_ph <= 4'd10; end
                    4'd10: begin tx_data <= 8'h5A; tx_start <= 1'b1; l_state <= L_DONE; end            // end
                    default: ;
                  endcase
                end
        L_DONE: ;                                              // idle; results delivered
      endcase
    end
  end
endmodule
