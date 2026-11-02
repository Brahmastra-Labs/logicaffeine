// dram_executor — the rung-3 in-order command-stream executor (COLD_HARD_FACTS Wave 3).
// The compile-time plan is delta-encoded ([delta:24][op:3][bank:5][row:32]); this FSM
// waits each delta and fires the command pins. Commands can only issue in strict word
// order — there is no queue or scheduling logic in the netlist to do otherwise. The
// countdown arithmetic is proven exact by our certified SAT (see codegen_sva::dram).
// cmd_ack is a FIFO-POP pulse asserted the cycle a word is CONSUMED (loaded), not the
// cycle it later issues — so the producer's read index advances immediately and a
// zero-delay command can never reload itself (the classic handshake-latency trap).
module dram_executor(
  input clk,
  input rst,
  input [63:0] cmd_word,
  input cmd_valid,
  output reg cmd_ack,
  output reg issue,
  output reg [2:0] op,
  output reg [4:0] bank,
  output reg [31:0] row
);
  reg loaded;
  reg [23:0] wait_left;
  reg [2:0] op_l;
  reg [4:0] bank_l;
  reg [31:0] row_l;
  always @(posedge clk) begin
    if (rst) begin
      loaded <= 1'b0; wait_left <= 24'd0; issue <= 1'b0; cmd_ack <= 1'b0;
      op <= 3'd0; bank <= 5'd0; row <= 32'd0; op_l <= 3'd0; bank_l <= 5'd0; row_l <= 32'd0;
    end else begin
      issue <= 1'b0;
      cmd_ack <= 1'b0;
      if (!loaded) begin
        if (cmd_valid) begin
          wait_left <= cmd_word[63:40];
          op_l <= cmd_word[39:37];
          bank_l <= cmd_word[36:32];
          row_l <= cmd_word[31:0];
          loaded <= 1'b1;
          cmd_ack <= 1'b1;
        end
      end else if (wait_left == 24'd0) begin
        issue <= 1'b1;
        op <= op_l; bank <= bank_l; row <= row_l;
        loaded <= 1'b0;
      end else begin
        wait_left <= wait_left - 24'd1;
      end
    end
  end
endmodule

// dram_executor_wb — the certified DRAM command player behind a Wishbone slave (Caravel user
// macro). The management SoC writes the compile-time plan word + a launch strobe; the countdown
// FSM (proven decode + issue-timing) fires the command, captured for readback + logic-analyzer.
module dram_executor_wb(
`ifdef USE_POWER_PINS
  inout         vccd1,   // User area 1.8V supply — Caravel PDN hook
  inout         vssd1,   // User area digital ground
`endif
  input         wb_clk_i,
  input         wb_rst_i,
  input         wbs_stb_i,
  input         wbs_cyc_i,
  input         wbs_we_i,
  input  [3:0]  wbs_sel_i,
  input  [31:0] wbs_adr_i,
  input  [31:0] wbs_dat_i,
  output reg    wbs_ack_o,
  output reg [31:0] wbs_dat_o,
  output [63:0] la_data_out
);
  wire valid = wbs_cyc_i & wbs_stb_i;
  wire wr    = valid & wbs_we_i;

  reg  [63:0] cmd_word_r;
  reg         cmd_valid_r;
  wire        cmd_ack, issue;
  wire [2:0]  op;  wire [4:0] bank;  wire [31:0] row;

  // The certified countdown executor — instantiated unchanged.
  dram_executor u_exec(
    .clk(wb_clk_i), .rst(wb_rst_i),
    .cmd_word(cmd_word_r), .cmd_valid(cmd_valid_r),
    .cmd_ack(cmd_ack), .issue(issue), .op(op), .bank(bank), .row(row)
  );

  // Latch the last issued command for readback / observation.
  reg [2:0] op_c;  reg [4:0] bank_c;  reg [31:0] row_c;  reg issued_c;

  // POST-FAB HANDSHAKE. `issued_c` alone is a STICKY flag: set on the first issue and cleared
  // only by reset. Firmware that writes a command and polls the status register therefore reads
  // `issued = 1` left over from the PREVIOUS command and cannot tell its own command has gone
  // out — and two identical back-to-back commands are indistinguishable in every visible field.
  // The user area's only observation path after fabrication is the management SoC reading these
  // registers, so that made command sequencing on real silicon unreliable.
  //
  // `issue_count` is the fix: a monotonically increasing counter, one tick per issued command.
  // The protocol is read-modify-poll, with no clear-on-read race and no lost event:
  //     n = read(0x0c)[15:8];  write cmd_word; write cmd_valid;
  //     poll 0x0c until [15:8] != n;  then read op/bank from 0x0c and row from 0x10.
  // It wraps at 256 by design — only INEQUALITY is ever tested, so wrapping is harmless unless
  // firmware sleeps through 256 issues, which the single-command-at-a-time protocol prevents.
  // `issued_c` is retained at its original bit position so existing readers keep working.
  reg [7:0] issue_count;

  always @(posedge wb_clk_i) begin
    if (wb_rst_i) begin
      cmd_word_r <= 64'd0; cmd_valid_r <= 1'b0; wbs_ack_o <= 1'b0;
      op_c <= 3'd0; bank_c <= 5'd0; row_c <= 32'd0; issued_c <= 1'b0;
      issue_count <= 8'd0;
    end else begin
      wbs_ack_o <= valid & ~wbs_ack_o;         // single-cycle Wishbone ack
      if (cmd_ack) cmd_valid_r <= 1'b0;         // the executor consumed the word
      if (issue) begin
        op_c <= op; bank_c <= bank; row_c <= row; issued_c <= 1'b1;
        issue_count <= issue_count + 8'd1;   // the observable event firmware polls on
      end
      if (wr) begin
        case (wbs_adr_i[7:0])
          8'h00: cmd_word_r[31:0]  <= wbs_dat_i;
          8'h04: cmd_word_r[63:32] <= wbs_dat_i;
          8'h08: cmd_valid_r       <= wbs_dat_i[0];
        endcase
      end
    end
  end

  // Registered read data mux.
  always @(posedge wb_clk_i) begin
    case (wbs_adr_i[7:0])
      8'h0c:   wbs_dat_o <= {issued_c, 15'd0, issue_count, bank_c, op_c};
      8'h10:   wbs_dat_o <= row_c;
      default: wbs_dat_o <= 32'd0;
    endcase
  end

  assign la_data_out = {issued_c, 15'd0, issue_count, bank_c, op_c, row_c};
endmodule

// dram_user_project — the full-interface Caravel user macro: the certified Wishbone command
// player (dram_executor_wb) presented on the complete Caravel user interface so the wrapper wires
// every pin (LVS-clean). The command player issues onto la_data_out[63:0]; other I/O is passthrough.
module dram_user_project (
`ifdef USE_POWER_PINS
  inout         vccd1,
  inout         vssd1,
`endif
  input         wb_clk_i,
  input         wb_rst_i,
  input         wbs_stb_i,
  input         wbs_cyc_i,
  input         wbs_we_i,
  input  [3:0]  wbs_sel_i,
  input  [31:0] wbs_dat_i,
  input  [31:0] wbs_adr_i,
  output        wbs_ack_o,
  output [31:0] wbs_dat_o,
  input  [127:0] la_data_in,
  output [127:0] la_data_out,
  input  [127:0] la_oenb,
  input  [37:0]  io_in,
  output [37:0]  io_out,
  output [37:0]  io_oeb,
  input         user_clock2,
  output [2:0]  user_irq
);
  wire [63:0] la_status;
  dram_executor_wb core (
`ifdef USE_POWER_PINS
    .vccd1(vccd1), .vssd1(vssd1),
`endif
    .wb_clk_i(wb_clk_i), .wb_rst_i(wb_rst_i),
    .wbs_stb_i(wbs_stb_i), .wbs_cyc_i(wbs_cyc_i), .wbs_we_i(wbs_we_i),
    .wbs_sel_i(wbs_sel_i), .wbs_adr_i(wbs_adr_i), .wbs_dat_i(wbs_dat_i),
    .wbs_ack_o(wbs_ack_o), .wbs_dat_o(wbs_dat_o),
    .la_data_out(la_status)
  );
  assign la_data_out = {64'd0, la_status};
  assign io_out   = 38'd0;
  assign io_oeb   = {38{1'b1}};   // all user GPIO as inputs (output-enable-bar high)
  assign user_irq = 3'd0;
  // The remaining Caravel inputs are part of the fixed interface (wired 1:1 by the wrapper for a
  // clean LVS) and unused by the command player; reduce them so the ports are kept + connected.
  wire _unused = &{1'b0, la_data_in, la_oenb, io_in, user_clock2};
endmodule
