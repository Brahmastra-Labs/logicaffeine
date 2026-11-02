// nexys_soc_scalar — the ROUTABLE scalar LOGOS core (soft_core_scalar.v) made BOARD-RUNNABLE on the
// Nexys A7 via the fully-OPEN flow (yosys synth_xilinx -> nextpnr-xilinx -> prjxray). Identical UART
// load/run/readback protocol + on-chip run-cycle count as nexys_soc.v, but wraps the scalar core
// (reversible ops, LoadConst/Move, branches, Call/Return recursion, full ALU — NO async-dmem collection
// coprocessor, NO bigint DSP bank), which fits the xc7a100t and PLACES-AND-ROUTES on nextpnr-xilinx where
// the full soft_core (async LUT-RAM memories + a DSP cascade the open packer mishandles) cannot.
//
// imem is 256 words (prog_addr[7:0]); a program is 1..255 instruction words.
//   host -> FPGA : <N=nwords(1..255)>, then N*4 program bytes (MSB-first per word [op][dst][a][b])
//   FPGA -> host : 0xA5, <M=nshow>, M*4 result bytes (MSB-first), 4 run-cycle bytes (MSB-first), 0x5A
//
// Must be compiled BEFORE soft_core_scalar.v so SOFT_CORE_SCALAR_PROG_LOAD is set when it is parsed.
`define SOFT_CORE_SCALAR_PROG_LOAD
module nexys_soc_scalar #(parameter CLKS_PER_BIT = 868) (
  input         clk,
  input         rst,          // active-high synchronous reset (board: a slide switch)
  input         uart_rx,      // from the host (PC -> FPGA)
  output reg    uart_tx,      // to the host (FPGA -> PC)
  output [15:0] led
);
  localparam CW = $clog2(CLKS_PER_BIT) + 1;

  // ── the scalar core, held in reset until its program is loaded ───────────────────────
  reg         core_rst;
  reg         ld_we;
  reg  [7:0]  ld_addr;
  reg  [31:0] ld_data;
  wire [31:0] core_out;
  wire        core_ov;
  wire        core_halted;

  soft_core_scalar u_core (
    .clk(clk), .rst(core_rst),
    .prog_we(ld_we), .prog_addr(ld_addr), .prog_data(ld_data),
    .out(core_out), .out_valid(core_ov), .halted(core_halted)
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

  // ── loader / runner / sender FSM ─────────────────────────────────────────────────────
  localparam L_LEN = 3'd0, L_PROG = 3'd1, L_RUN = 3'd2, L_SEND = 3'd3, L_DONE = 3'd4;
  reg [2:0]  l_state;
  reg [8:0]  nprog;                        // program length in words (from the length byte; 1..255)
  reg [7:0]  word_idx;                     // which imem word we are assembling (0..255)
  reg [1:0]  byte_in_word;
  reg [31:0] word_accum;

  (* ram_style = "registers" *) reg [31:0] show_mem [0:31];   // FF-backed: the open flow drops the second
  reg [5:0]  nshow;                                            // of two consecutive LUT-RAM writes (back-to-back Shows)
  reg [5:0]  send_idx;
  reg [3:0]  send_ph;                      // 0=sync,1=count,2..5=value bytes,6..9=cycle bytes,10=end
  reg [31:0] cycles;                       // core run-cycle count (frozen at halt), returned in the frame

  assign led = {core_halted, l_state, nshow, 6'd0};

  integer i;
  always @(posedge clk) begin
    rx_valid <= 1'b0;
    ld_we    <= 1'b0;
    tx_start <= 1'b0;

    rx_d1 <= uart_rx;  rx_d2 <= rx_d1;

    if (rst) begin
      r_state <= R_IDLE; t_state <= T_IDLE; uart_tx <= 1'b1;
      l_state <= L_LEN; core_rst <= 1'b1; nprog <= 9'd0; word_idx <= 8'd0;
      byte_in_word <= 2'd0; word_accum <= 32'd0; nshow <= 6'd0; send_idx <= 6'd0;
      send_ph <= 4'd0; cycles <= 32'd0; r_cnt <= 0; r_bit <= 3'd0; t_cnt <= 0; t_bit <= 3'd0;
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
        L_LEN: if (rx_valid) begin
                 nprog <= {1'b0, rx_data}; word_idx <= 8'd0; byte_in_word <= 2'd0;
                 l_state <= (rx_data == 8'd0) ? L_RUN : L_PROG;
               end
        L_PROG: if (rx_valid) begin
                  word_accum <= {word_accum[23:0], rx_data};
                  if (byte_in_word == 2'd3) begin
                    ld_we <= 1'b1; ld_addr <= word_idx; ld_data <= {word_accum[23:0], rx_data};
                    byte_in_word <= 2'd0;
                    if (word_idx + 1'b1 == nprog) l_state <= L_RUN;
                    else word_idx <= word_idx + 1'b1;
                  end else byte_in_word <= byte_in_word + 1'b1;
                end
        L_RUN: begin
                 core_rst <= 1'b0;
                 if (!core_halted) cycles <= cycles + 1'b1;
                 if (core_ov && nshow != 6'd32) begin
                   show_mem[nshow[4:0]] <= core_out; nshow <= nshow + 1'b1;
                 end
                 // Reply on halt, OR on a watchdog (~16M cycles) so a non-halting program can never hang
                 // the board — the readback then shows exactly what the core captured before the timeout.
                 if (core_halted || cycles == 32'h00FF_FFFF) begin
                   l_state <= L_SEND; send_idx <= 6'd0; send_ph <= 4'd0;
                 end
               end
        L_SEND: if (!tx_busy) begin
                  case (send_ph)
                    4'd0: begin tx_data <= 8'hA5; tx_start <= 1'b1; send_ph <= 4'd1; end
                    4'd1: begin tx_data <= {2'd0, nshow}; tx_start <= 1'b1;
                                send_ph <= (nshow == 6'd0) ? 4'd6 : 4'd2; end
                    4'd2: begin tx_data <= show_mem[send_idx[4:0]][31:24]; tx_start <= 1'b1; send_ph <= 4'd3; end
                    4'd3: begin tx_data <= show_mem[send_idx[4:0]][23:16]; tx_start <= 1'b1; send_ph <= 4'd4; end
                    4'd4: begin tx_data <= show_mem[send_idx[4:0]][15:8];  tx_start <= 1'b1; send_ph <= 4'd5; end
                    4'd5: begin tx_data <= show_mem[send_idx[4:0]][7:0];   tx_start <= 1'b1;
                                if (send_idx + 1'b1 == nshow) send_ph <= 4'd6;
                                else begin send_idx <= send_idx + 1'b1; send_ph <= 4'd2; end end
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
