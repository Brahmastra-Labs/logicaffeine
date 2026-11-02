//! caravel_rtl — the Infinichip command player wrapped as an efabless **Caravel user macro**, the
//! step that makes it literally shuttle-integratable.
//!
//! A real Caravel submission keeps efabless's golden `user_project_wrapper.v` VERBATIM and only
//! swaps the user macro instantiated inside it. So the artifact WE own is that macro:
//! **`dram_executor_wb`** — the countdown command player (`codegen_sva::dram`, decode + issue-timing
//! certified by our own SAT) behind a standard **Wishbone** slave, with the issued command exposed
//! on the **logic-analyzer** bus for observation. The management SoC writes the 64-bit compile-time
//! plan word over Wishbone (two 32-bit stores), strobes launch, and the certified FSM issues.
//!
//! Register map (Wishbone byte offsets, caravel user base 0x3000_0000):
//!
//! ```text
//! 0x00  W  cmd_word[31:0]           0x0C  R  {issued, bank, op} status
//! 0x04  W  cmd_word[63:32]          0x10  R  row of the last issued command
//! 0x08  W  bit0 = launch (cmd_valid)
//! ```
//!
//! `la_data_out[63:0]` = {issued, 0, bank, op, row} — live observation on the logic analyzer.
//!
//! Wishbone conventions match `user_proj_example` (`valid = cyc & stb`, single-cycle `ack`).

use super::dram::emit_dram_executor_verilog;

/// The Wishbone slave that fronts the certified command player — the efabless Caravel USER MACRO.
fn dram_executor_wb() -> &'static str {
    r#"// dram_executor_wb — the certified DRAM command player behind a Wishbone slave (Caravel user
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
"#
}

/// Emit the SELF-CONTAINED user macro: the certified `dram_executor` + the `dram_executor_wb`
/// Wishbone front end. This is the design hardened + golden-DRC'd + dropped into Caravel's golden
/// `user_project_wrapper` for a shuttle submission.
pub fn emit_dram_executor_wb_verilog() -> String {
    let mut v = String::new();
    v.push_str(&emit_dram_executor_verilog());
    v.push('\n');
    v.push_str(dram_executor_wb());
    v
}

/// A self-checking Wishbone testbench: the "management SoC" writes a delta-0 plan word
/// (op=1, bank=3, row=7) over Wishbone, strobes launch, and asserts the certified FSM issues that
/// exact command — observed on `la_data_out`. Prints PASS/FAIL; no external vectors.
pub fn emit_dram_executor_wb_testbench() -> String {
    r#"// tb_dram_executor_wb — drive the command player over Wishbone; check it issues the command.
`timescale 1ns/1ps
module tb_dram_executor_wb;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1;
  reg stb = 0, cyc = 0, we = 0;
  reg [3:0] sel = 4'hf;
  reg [31:0] adr = 0, dat = 0;
  wire ack;  wire [31:0] rdat;  wire [63:0] la;
  integer errors = 0;

  dram_executor_wb dut(
    .wb_clk_i(clk), .wb_rst_i(rst), .wbs_stb_i(stb), .wbs_cyc_i(cyc), .wbs_we_i(we),
    .wbs_sel_i(sel), .wbs_adr_i(adr), .wbs_dat_i(dat), .wbs_ack_o(ack), .wbs_dat_o(rdat),
    .la_data_out(la)
  );

  task wb_write(input [31:0] a, input [31:0] d);
    begin
      @(posedge clk); adr <= a; dat <= d; we <= 1; stb <= 1; cyc <= 1;
      @(posedge clk); while (!ack) @(posedge clk);
      stb <= 0; cyc <= 0; we <= 0;
      @(posedge clk);
    end
  endtask

  initial begin
    repeat (3) @(posedge clk); rst = 0; @(posedge clk);
    // delta-0 command: cmd_word = {24'd0, op=1, bank=3, row=7}
    wb_write(32'h00, 32'd7);       // cmd_word[31:0]  = row
    wb_write(32'h04, 32'h23);      // cmd_word[63:32] = {delta=0, op=1(<<5), bank=3}
    wb_write(32'h08, 32'h1);       // launch (cmd_valid)
    repeat (8) @(posedge clk);
    // la_data_out = {issued, 15'd0, issue_count, bank, op, row}
    if (la[63] !== 1'b1)        begin errors = errors + 1; $display("FAIL: never issued"); end
    if (la[47:40] !== 8'd1)     begin errors = errors + 1; $display("FAIL: issue_count %0d != 1 after one command", la[47:40]); end
    if (la[39:35] !== 5'd3)     begin errors = errors + 1; $display("FAIL: bank %0d != 3", la[39:35]); end
    if (la[34:32] !== 3'd1)     begin errors = errors + 1; $display("FAIL: op %0d != 1", la[34:32]); end
    if (la[31:0] !== 32'd7)     begin errors = errors + 1; $display("FAIL: row %0d != 7", la[31:0]); end
    if (errors == 0) $display("PASS: Wishbone-driven command player issued op=1 bank=3 row=7");
    else $display("FAIL: %0d Wishbone integration errors", errors);
    $finish;
  end
endmodule
"#
    .to_string()
}

/// The FULL-INTERFACE Caravel user macro (`dram_user_project`) — exposes the ENTIRE Caravel user
/// interface (Wishbone + 128-bit logic analyzer + 38 GPIO + user_irq + user_clock2) so the golden
/// wrapper can connect every pin 1:1 (LVS-clean), with the locked, certified `dram_executor_wb`
/// Wishbone core inside. The command player drives the low logic-analyzer bits; the rest of the
/// interface is a proper Caravel passthrough (unused inputs kept + connected, unused outputs tied).
fn dram_user_project() -> &'static str {
    r#"// dram_user_project — the full-interface Caravel user macro: the certified Wishbone command
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
"#
}

/// Emit the SELF-CONTAINED full-interface user macro: `dram_executor` + `dram_executor_wb` core +
/// `dram_user_project`. This is the macro hardened (with power pins + tap insertion) and placed as
/// `mprj` inside the golden `user_project_wrapper` for a shuttle submission.
pub fn emit_dram_user_project_verilog() -> String {
    let mut v = emit_dram_executor_wb_verilog();
    v.push('\n');
    v.push_str(dram_user_project());
    v
}

/// Emit our `user_project_wrapper` for the DRAM command player: [`emit_user_project_wrapper_around`]
/// `dram_user_project`.
pub fn emit_user_project_wrapper_verilog() -> String {
    emit_user_project_wrapper_around("dram_user_project", "certified command player")
}

/// Emit a `user_project_wrapper` — the efabless golden Caravel wrapper interface, verbatim ports,
/// instantiating the full-interface user macro `macro_module` as `mprj` and connecting EVERY wrapper
/// pin 1:1 (the passthrough that makes the placed-and-routed wrapper LVS-clean). `carries` names what
/// the macro holds, for the header.
pub fn emit_user_project_wrapper_around(macro_module: &str, carries: &str) -> String {
    r#"// user_project_wrapper — the Caravel user area. Golden interface verbatim; the full-interface
// user macro ({macro_module}, {carries} inside) is instantiated as `mprj` with
// every pin connected 1:1 so the hardened wrapper is LVS-clean.
`define MPRJ_IO_PADS 38
module user_project_wrapper #(parameter BITS = 32) (
`ifdef USE_POWER_PINS
  inout vdda1, inout vdda2, inout vssa1, inout vssa2,
  inout vccd1, inout vccd2, inout vssd1, inout vssd2,
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
  input  [`MPRJ_IO_PADS-1:0] io_in,
  output [`MPRJ_IO_PADS-1:0] io_out,
  output [`MPRJ_IO_PADS-1:0] io_oeb,
  inout  [`MPRJ_IO_PADS-10:0] analog_io,
  input         user_clock2,
  output [2:0]  user_irq
);
  // Full-interface user macro — the {carries} inside, EVERY pin wired 1:1.
  {macro_module} mprj (
`ifdef USE_POWER_PINS
    .vccd1(vccd1), .vssd1(vssd1),
`endif
    .wb_clk_i(wb_clk_i), .wb_rst_i(wb_rst_i),
    .wbs_stb_i(wbs_stb_i), .wbs_cyc_i(wbs_cyc_i), .wbs_we_i(wbs_we_i),
    .wbs_sel_i(wbs_sel_i), .wbs_adr_i(wbs_adr_i), .wbs_dat_i(wbs_dat_i),
    .wbs_ack_o(wbs_ack_o), .wbs_dat_o(wbs_dat_o),
    .la_data_in(la_data_in), .la_data_out(la_data_out), .la_oenb(la_oenb),
    .io_in(io_in), .io_out(io_out), .io_oeb(io_oeb),
    .user_clock2(user_clock2), .user_irq(user_irq)
  );
  // analog_io has no digital use in this macro (left unconnected, as in the empty golden wrapper).
endmodule
"#
    .replace("{macro_module}", macro_module)
    .replace("{carries}", carries)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/caravel_rtl/tests.rs"]
mod tests;
