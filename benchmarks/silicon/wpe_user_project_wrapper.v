// user_project_wrapper — the Caravel user area. Golden interface verbatim; the full-interface
// user macro (wpe_user_project, whole-program engine inside) is instantiated as `mprj` with
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
  // Full-interface user macro — the whole-program engine inside, EVERY pin wired 1:1.
  wpe_user_project mprj (
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
