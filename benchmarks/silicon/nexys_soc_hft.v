// nexys_soc_hft — the board-runnable LOGOS full-integer core for the tick-to-trade demo. Identical to
// nexys_soc_full.v (open-flow UART load/run/readback + on-chip run-cycle count) with two changes so a REAL
// program — the ITCH->book->microprice->risk->order pipeline is ~330 words — actually fits and loads:
//   * imem is 1024 words (soft_core prog_addr is 10-bit) instead of 256;
//   * the program length is a 2-byte big-endian word (1..1024) instead of a single byte (1..255).
// Everything downstream (the run/readback frame, the cycle counter, the tagged Show records) is unchanged,
// so scripts/nexys-host.py --words16 talks to it byte-for-byte.
//
//   host -> FPGA : <N_hi><N_lo> (big-endian, 1..1024), then N*4 program bytes (MSB-first [op][dst][a][b])
//   FPGA -> host : 0xA5, <M=nshow>, per-show <tag><value 4B int / 8B float>, 4 run-cycle bytes, 0x5A
//
// Must be compiled BEFORE soft_core.v so SOFT_CORE_PROG_LOAD is set when it is parsed. NO_BIGINT drops the
// bigint DSP bank (the pipeline is 32-bit only) — the same routable configuration nexys_soc_full uses.
`define SOFT_CORE_PROG_LOAD
`define NO_BIGINT
module nexys_soc_hft #(parameter CLKS_PER_BIT = 868) (
  input         clk,
  input         rst,
  input         uart_rx,
  output reg    uart_tx,
  output [15:0] led
);
  localparam CW = $clog2(CLKS_PER_BIT) + 1;

  // ── the full integer core, held in reset until its program is loaded ─────────────────
  reg         core_rst;
  reg         ld_we;
  reg  [9:0]  ld_addr;
  reg  [31:0] ld_data;
  wire [31:0] core_out;
  wire        core_ov;
  wire [63:0] core_outf;
  wire        core_ovf;
  wire        core_halted;

  soft_core #(.HEAP(2048), .FRAMES(64), .REGS(256), .IMEM_WORDS(512)) u_core (
    .clk(clk), .rst(core_rst),
    .prog_we(ld_we), .prog_addr(ld_addr), .prog_data(ld_data),
    .out(core_out), .out_valid(core_ov),
    .outf(core_outf), .outf_valid(core_ovf),
    .halted(core_halted)
  );

  // ── UART receiver ────────────────────────────────────────────────────────────────────
  localparam R_IDLE = 2'd0, R_START = 2'd1, R_DATA = 2'd2, R_STOP = 2'd3;
  reg [1:0]     r_state;
  reg [CW-1:0]  r_cnt;
  reg [2:0]     r_bit;
  reg [7:0]     r_shift;
  reg           rx_valid;
  reg [7:0]     rx_data;
  reg           rx_d1, rx_d2;

  // ── UART transmitter ─────────────────────────────────────────────────────────────────
  localparam T_IDLE = 2'd0, T_START = 2'd1, T_DATA = 2'd2, T_STOP = 2'd3;
  reg [1:0]     t_state;
  reg [CW-1:0]  t_cnt;
  reg [2:0]     t_bit;
  reg [7:0]     t_shift;
  reg           tx_start;
  reg [7:0]     tx_data;
  wire          tx_busy = (t_state != T_IDLE) || tx_start;

  // ── loader / runner / sender FSM (2-byte length) ─────────────────────────────────────
  localparam L_LEN_HI = 3'd0, L_LEN_LO = 3'd1, L_PROG = 3'd2, L_RUN = 3'd3, L_SEND = 3'd4, L_DONE = 3'd5;
  reg [2:0]  l_state;
  reg [15:0] nprog;                        // program length in words (2-byte big-endian; 1..1024)
  reg [10:0] word_idx;                     // which imem word we are assembling (0..1023)
  reg [1:0]  byte_in_word;
  reg [31:0] word_accum;

  (* ram_style = "registers" *) reg [63:0] show_mem [0:31];
  (* ram_style = "registers" *) reg        show_tag [0:31];
  reg [5:0]  nshow;
  reg [5:0]  send_idx;
  reg [3:0]  send_ph;
  reg [3:0]  val_bp;
  reg [31:0] cycles;

  assign led = {core_halted, l_state, nshow, 6'd0};

  always @(posedge clk) begin
    rx_valid <= 1'b0;
    ld_we    <= 1'b0;
    tx_start <= 1'b0;

    rx_d1 <= uart_rx;  rx_d2 <= rx_d1;

    if (rst) begin
      r_state <= R_IDLE; t_state <= T_IDLE; uart_tx <= 1'b1;
      l_state <= L_LEN_HI; core_rst <= 1'b1; nprog <= 16'd0; word_idx <= 11'd0;
      byte_in_word <= 2'd0; word_accum <= 32'd0; nshow <= 6'd0; send_idx <= 6'd0;
      send_ph <= 4'd0; val_bp <= 4'd0; cycles <= 32'd0; r_cnt <= 0; r_bit <= 3'd0; t_cnt <= 0; t_bit <= 3'd0;
    end else begin
      // ───────── UART RX ─────────
      case (r_state)
        R_IDLE:  if (~rx_d2) begin r_state <= R_START; r_cnt <= 0; end
        R_START: if (r_cnt == (CLKS_PER_BIT-1)/2) begin
                   if (~rx_d2) begin r_cnt <= 0; r_bit <= 3'd0; r_state <= R_DATA; end
                   else r_state <= R_IDLE;
                 end else r_cnt <= r_cnt + 1'b1;
        R_DATA:  if (r_cnt == CLKS_PER_BIT-1) begin
                   r_cnt <= 0; r_shift <= {rx_d2, r_shift[7:1]};
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
        L_LEN_HI: if (rx_valid) begin nprog[15:8] <= rx_data; l_state <= L_LEN_LO; end
        L_LEN_LO: if (rx_valid) begin
                    nprog[7:0] <= rx_data; word_idx <= 11'd0; byte_in_word <= 2'd0;
                    l_state <= ({nprog[15:8], rx_data} == 16'd0) ? L_RUN : L_PROG;
                  end
        L_PROG: if (rx_valid) begin
                  word_accum <= {word_accum[23:0], rx_data};
                  if (byte_in_word == 2'd3) begin
                    ld_we <= 1'b1; ld_addr <= word_idx[9:0]; ld_data <= {word_accum[23:0], rx_data};
                    byte_in_word <= 2'd0;
                    if (word_idx + 1'b1 == nprog) l_state <= L_RUN;
                    else word_idx <= word_idx + 1'b1;
                  end else byte_in_word <= byte_in_word + 1'b1;
                end
        L_RUN: begin
                 core_rst <= 1'b0;
                 if (!core_halted) cycles <= cycles + 1'b1;
                 if (core_ov && nshow != 6'd32) begin
                   show_mem[nshow[4:0]] <= {32'd0, core_out}; show_tag[nshow[4:0]] <= 1'b0; nshow <= nshow + 1'b1;
                 end else if (core_ovf && nshow != 6'd32) begin
                   show_mem[nshow[4:0]] <= core_outf; show_tag[nshow[4:0]] <= 1'b1; nshow <= nshow + 1'b1;
                 end
                 if (core_halted || cycles == 32'h00FF_FFFF) begin
                   l_state <= L_SEND; send_idx <= 6'd0; send_ph <= 4'd0; val_bp <= 4'd0;
                 end
               end
        L_SEND: if (!tx_busy) begin
                  case (send_ph)
                    4'd0: begin tx_data <= 8'hA5; tx_start <= 1'b1; send_ph <= 4'd1; end
                    4'd1: begin tx_data <= {2'd0, nshow}; tx_start <= 1'b1;
                                send_ph <= (nshow == 6'd0) ? 4'd6 : 4'd2; end
                    4'd2: begin tx_data <= {7'd0, show_tag[send_idx[4:0]]}; tx_start <= 1'b1;
                                val_bp <= 4'd0; send_ph <= 4'd3; end
                    4'd3: begin
                            tx_data <= show_tag[send_idx[4:0]]
                                       ? show_mem[send_idx[4:0]][8*(7 - val_bp) +: 8]
                                       : show_mem[send_idx[4:0]][8*(3 - val_bp) +: 8];
                            tx_start <= 1'b1;
                            if (val_bp == (show_tag[send_idx[4:0]] ? 4'd7 : 4'd3)) begin
                              if (send_idx + 1'b1 == nshow) send_ph <= 4'd6;
                              else begin send_idx <= send_idx + 1'b1; send_ph <= 4'd2; end
                            end else val_bp <= val_bp + 1'b1;
                          end
                    4'd6: begin tx_data <= cycles[31:24]; tx_start <= 1'b1; send_ph <= 4'd7; end
                    4'd7: begin tx_data <= cycles[23:16]; tx_start <= 1'b1; send_ph <= 4'd8; end
                    4'd8: begin tx_data <= cycles[15:8];  tx_start <= 1'b1; send_ph <= 4'd9; end
                    4'd9: begin tx_data <= cycles[7:0];   tx_start <= 1'b1; send_ph <= 4'd10; end
                    4'd10: begin tx_data <= 8'h5A; tx_start <= 1'b1; l_state <= L_DONE; end
                    default: ;
                  endcase
                end
        L_DONE: ;
      endcase
    end
  end
endmodule
