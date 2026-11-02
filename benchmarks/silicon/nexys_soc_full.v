// nexys_soc_full — the ROUTABLE scalar LOGOS core (soft_core_scalar.v) made BOARD-RUNNABLE on the
// Nexys A7 via the fully-OPEN flow (yosys synth_xilinx -> nextpnr-xilinx -> prjxray). Identical UART
// load/run/readback protocol + on-chip run-cycle count as nexys_soc.v, but wraps the scalar core
// (reversible ops, LoadConst/Move, branches, Call/Return recursion, full ALU — NO async-dmem collection
// coprocessor, NO bigint DSP bank), which fits the xc7a100t and PLACES-AND-ROUTES on nextpnr-xilinx where
// the full soft_core (async LUT-RAM memories + a DSP cascade the open packer mishandles) cannot.
//
// imem is 4096 words (prog_addr[11:0]); a program is 1..4095 instruction words (2-byte-escape length).
//   host -> FPGA : <N=nwords(1..255)>, then N*4 program bytes (MSB-first per word [op][dst][a][b])
//   FPGA -> host : 0xA5, <M=nshow>, then for EACH of the M shows IN ORDER a tagged record
//                    <tag:1> (0=int, 1=float, 2=string)
//                    then 4 value bytes (int, MSB-first) OR 8 value bytes (float, raw IEEE-754 f64 bits,
//                    MSB-first) OR — for a string — 4 length bytes (char count, MSB-first) then that many
//                    4-byte codepoints (each MSB-first),
//                  then 4 run-cycle bytes (MSB-first), then 0x5A.
//   Int Shows drive core.out/out_valid (opcode 89); float Shows drive core.outf/outf_valid (ShowF, opcode
//   212); string Shows drive core.outs/outs_valid/outs_first (ShowText, opcode 217) — the first word carries
//   the char count (outs_first high), then each codepoint. All three are captured interleaved in program
//   order so the host prints them in the order the core showed them; the tag+width let the host pick the
//   int path, fmt_f64(), or the string reassembler per record.
//
// Must be compiled BEFORE soft_core_scalar.v so SOFT_CORE_SCALAR_PROG_LOAD is set when it is parsed.
`define SOFT_CORE_PROG_LOAD
`define NO_BIGINT
module nexys_soc_full #(parameter CLKS_PER_BIT = 868) (
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
  reg  [11:0] ld_addr;
  reg  [31:0] ld_data;
  wire [31:0] core_out;
  wire        core_ov;
  wire [63:0] core_outf;
  wire        core_ovf;
  wire [31:0] core_outs;         // string burst word: length (when core_outs_first) then each codepoint
  wire        core_outs_v;       // pulses high each cycle a ShowText streams a word
  wire        core_outs_first;   // high on the FIRST word of a string (the length word)
  wire [31:0] core_outw;         // UNSIGNED Word32 Show value (ShowWord32, opcode 221) — host formats unsigned
  wire        core_ovw;          // pulses high the cycle a ShowWord32 executes
  wire        core_halted;

  soft_core #(.HEAP(2048), .FRAMES(64), .REGS(256), .IMEM_WORDS(4096)) u_core (
    .clk(clk), .rst(core_rst),
    .prog_we(ld_we), .prog_addr(ld_addr), .prog_data(ld_data),
    .out(core_out), .out_valid(core_ov),
    .outf(core_outf), .outf_valid(core_ovf),
    .outs(core_outs), .outs_valid(core_outs_v), .outs_first(core_outs_first),
    .outw(core_outw), .outw_valid(core_ovw),
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

  // ── loader / runner / sender FSM ─────────────────────────────────────────────────────
  localparam L_LEN = 3'd0, L_PROG = 3'd1, L_RUN = 3'd2, L_SEND = 3'd3, L_DONE = 3'd4,
             L_LEN_HI = 3'd5, L_LEN_LO = 3'd6; // 2-byte-length escape states (see L_LEN below)
  reg [2:0]  l_state;
  reg [11:0] nprog;                        // program length in WORDS (0..4095). 1 length byte 1..254; the
                                           // escape value 8'hFF means "two more bytes follow (MSB,LSB)".
  reg [11:0] word_idx;                     // which imem word we are assembling (0..nprog-1)
  reg [1:0]  byte_in_word;
  reg [31:0] word_accum;

  (* ram_style = "registers" *) reg [63:0] show_mem [0:31];   // FF-backed: the open flow drops the second
  (* ram_style = "registers" *) reg [1:0]  show_tag [0:31];    // of two consecutive LUT-RAM writes (back-to-back Shows).
  reg [5:0]  nshow;                                            // int/float slots hold a 32-bit int (low word) or a 64-bit f64;
  reg [5:0]  send_idx;                                         // a string slot holds {start_off, char_count} into strbuf.
  reg [4:0]  send_ph;                      // send phases: 0=sync,1=count,2=tag,3=int/float bytes,4=str len bytes,
                                           //   5=str codepoint bytes,6..9=cycle bytes,10=end (show_tag[i]=0 int/1 float/2 string)
  reg [3:0]  val_bp;                        // byte position within the current record's value (0..3 int / 0..7 float / 0..3 length or codepoint)
  reg [31:0] cycles;                       // core run-cycle count (frozen at halt), returned in the frame

  // STRING char buffer: a `ShowText` streams its codepoints over core_outs (length first, then each codepoint).
  // We stash every string's codepoints back-to-back here, and the show slot records the {start_off, char_count}
  // so the sender can stream a string record (tag 2). 256 codepoints total covers every benchmark string mix.
  (* ram_style = "registers" *) reg [31:0] strbuf [0:255];
  reg [8:0]  strbuf_wp;                     // write pointer (running across all string shows this run)
  reg [8:0]  str_start;                     // start offset of the string currently streaming in (set on its first word)
  reg [31:0] send_slen;                     // char count of the string record being sent
  reg [31:0] send_ci;                       // codepoint index being sent (0 .. send_slen-1)
  reg [8:0]  send_soff;                     // strbuf read offset of the string record being sent

  assign led = {core_halted, l_state, nshow, 6'd0};

  integer i;
  always @(posedge clk) begin
    rx_valid <= 1'b0;
    ld_we    <= 1'b0;
    tx_start <= 1'b0;

    rx_d1 <= uart_rx;  rx_d2 <= rx_d1;

    if (rst) begin
      r_state <= R_IDLE; t_state <= T_IDLE; uart_tx <= 1'b1;
      l_state <= L_LEN; core_rst <= 1'b1; nprog <= 12'd0; word_idx <= 12'd0;
      byte_in_word <= 2'd0; word_accum <= 32'd0; nshow <= 6'd0; send_idx <= 6'd0;
      send_ph <= 5'd0; val_bp <= 4'd0; cycles <= 32'd0; r_cnt <= 0; r_bit <= 3'd0; t_cnt <= 0; t_bit <= 3'd0;
      strbuf_wp <= 9'd0; str_start <= 9'd0; send_slen <= 32'd0; send_ci <= 32'd0; send_soff <= 9'd0;
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
                 word_idx <= 12'd0; byte_in_word <= 2'd0;
                 if (rx_data == 8'hFF) begin
                   l_state <= L_LEN_HI;               // ESCAPE: 2 more length bytes (MSB,LSB) → up to 4095 words
                 end else begin
                   nprog <= {8'b0, rx_data};          // legacy 1-byte length (0..254) — unchanged
                   l_state <= (rx_data == 8'd0) ? L_RUN : L_PROG;
                 end
               end
        L_LEN_HI: if (rx_valid) begin nprog[11:8] <= rx_data[3:0]; l_state <= L_LEN_LO; end
        L_LEN_LO: if (rx_valid) begin
                    nprog[7:0] <= rx_data;
                    l_state <= ({nprog[11:8], rx_data} == 12'd0) ? L_RUN : L_PROG;
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
                 // Capture int / float / string Shows in program order. At most one strobe fires per cycle: an
                 // int/float Show is a single op; a ShowText holds the pc and bursts its words over successive
                 // cycles (length first with core_outs_first, then each codepoint), so its words never overlap an
                 // int/float Show. Int stores the 32-bit value in the low word (tag 0); float stores the raw f64
                 // bits (tag 1); a string records {start_off, char_count} into the show slot (tag 2) and appends
                 // its codepoints to strbuf. The host uses the tag to pick int / fmt_f64() / string reassembly.
                 if (core_ov && nshow != 6'd32) begin
                   show_mem[nshow[4:0]] <= {32'd0, core_out}; show_tag[nshow[4:0]] <= 2'd0; nshow <= nshow + 1'b1;
                 end else if (core_ovw && nshow != 6'd32) begin
                   // UNSIGNED Word32 Show (SHA-256 hash word): the same 32-bit low-word payload as an int Show,
                   // but tag 3 so the host formats it as an unsigned decimal (a hash word > 2³¹ would misprint
                   // as a negative signed int otherwise).
                   show_mem[nshow[4:0]] <= {32'd0, core_outw}; show_tag[nshow[4:0]] <= 2'd3; nshow <= nshow + 1'b1;
                 end else if (core_ovf && nshow != 6'd32) begin
                   show_mem[nshow[4:0]] <= core_outf; show_tag[nshow[4:0]] <= 2'd1; nshow <= nshow + 1'b1;
                 end else if (core_outs_v && core_outs_first) begin
                   // FIRST word of a string: core_outs = the char count. Record the show slot (tag 2) packing the
                   // strbuf start offset and the length; remember the start so we always allocate the slot even if
                   // nshow is saturated (a saturated string still consumes its codepoints so strbuf_wp stays sane).
                   str_start <= strbuf_wp;
                   if (nshow != 6'd32) begin
                     show_mem[nshow[4:0]] <= {23'd0, strbuf_wp, core_outs}; show_tag[nshow[4:0]] <= 2'd2;
                     nshow <= nshow + 1'b1;
                   end
                 end else if (core_outs_v && !core_outs_first) begin
                   // a codepoint of the current string: append it to strbuf (saturate at the 256-word buffer).
                   if (strbuf_wp != 9'd256) begin strbuf[strbuf_wp[7:0]] <= core_outs; strbuf_wp <= strbuf_wp + 1'b1; end
                 end
                 // Reply on halt, OR on a watchdog (~16M cycles) so a non-halting program can never hang
                 // the board — the readback then shows exactly what the core captured before the timeout.
                 if (core_halted || cycles == 32'h00FF_FFFF) begin
                   l_state <= L_SEND; send_idx <= 6'd0; send_ph <= 5'd0; val_bp <= 4'd0;
                 end
               end
        L_SEND: if (!tx_busy) begin
                  case (send_ph)
                    5'd0: begin tx_data <= 8'hA5; tx_start <= 1'b1; send_ph <= 5'd1; end
                    5'd1: begin tx_data <= {2'd0, nshow}; tx_start <= 1'b1;
                                send_ph <= (nshow == 6'd0) ? 5'd6 : 5'd2; end
                    // per-record: tag byte (0=int,1=float,2=string), then the record body (int 4B / float 8B /
                    // string [4B len then len*4B codepoints], all MSB-first).
                    5'd2: begin tx_data <= {6'd0, show_tag[send_idx[4:0]]}; tx_start <= 1'b1;
                                val_bp <= 4'd0; send_ph <= (show_tag[send_idx[4:0]] == 2'd2) ? 5'd4 : 5'd3; end
                    5'd3: begin
                            // float slot is the full 64 bits (MSB-first byte at val_bp); int slot lives in the low
                            // word, so its MSB-first byte at val_bp reads bits [31:24]..[7:0].
                            tx_data <= (show_tag[send_idx[4:0]] == 2'd1)
                                       ? show_mem[send_idx[4:0]][8*(7 - val_bp) +: 8]
                                       : show_mem[send_idx[4:0]][8*(3 - val_bp) +: 8];
                            tx_start <= 1'b1;
                            if (val_bp == ((show_tag[send_idx[4:0]] == 2'd1) ? 4'd7 : 4'd3)) begin
                              if (send_idx + 1'b1 == nshow) send_ph <= 5'd6;
                              else begin send_idx <= send_idx + 1'b1; send_ph <= 5'd2; end
                            end else val_bp <= val_bp + 1'b1;
                          end
                    // STRING: 4 length bytes (char count, MSB-first) from the low word of the slot. On the last
                    // length byte, latch the count + strbuf start offset for the codepoint stream (phase 5).
                    5'd4: begin
                            tx_data <= show_mem[send_idx[4:0]][8*(3 - val_bp) +: 8];
                            tx_start <= 1'b1;
                            if (val_bp == 4'd3) begin
                              send_slen <= show_mem[send_idx[4:0]][31:0];
                              send_soff <= show_mem[send_idx[4:0]][40:32];   // start offset packed above the length
                              send_ci   <= 32'd0; val_bp <= 4'd0;
                              if (show_mem[send_idx[4:0]][31:0] == 32'd0) begin  // empty string: no codepoints
                                if (send_idx + 1'b1 == nshow) send_ph <= 5'd6;
                                else begin send_idx <= send_idx + 1'b1; send_ph <= 5'd2; end
                              end else send_ph <= 5'd5;
                            end else val_bp <= val_bp + 1'b1;
                          end
                    // STRING codepoints: 4 bytes each (MSB-first) for send_slen codepoints, read from strbuf.
                    5'd5: begin
                            tx_data <= strbuf[(send_soff + send_ci[8:0]) & 9'h0FF][8*(3 - val_bp) +: 8];
                            tx_start <= 1'b1;
                            if (val_bp == 4'd3) begin
                              val_bp <= 4'd0;
                              if (send_ci + 1'b1 == send_slen) begin
                                if (send_idx + 1'b1 == nshow) send_ph <= 5'd6;
                                else begin send_idx <= send_idx + 1'b1; send_ph <= 5'd2; end
                              end else send_ci <= send_ci + 1'b1;
                            end else val_bp <= val_bp + 1'b1;
                          end
                    5'd6: begin tx_data <= cycles[31:24]; tx_start <= 1'b1; send_ph <= 5'd7; end
                    5'd7: begin tx_data <= cycles[23:16]; tx_start <= 1'b1; send_ph <= 5'd8; end
                    5'd8: begin tx_data <= cycles[15:8];  tx_start <= 1'b1; send_ph <= 5'd9; end
                    5'd9: begin tx_data <= cycles[7:0];   tx_start <= 1'b1; send_ph <= 5'd10; end
                    5'd10: begin tx_data <= 8'h5A; tx_start <= 1'b1; l_state <= L_DONE; end
                    default: ;
                  endcase
                end
        L_DONE: ;
      endcase
    end
  end
endmodule
