// soft_core_realbist — the routable scalar LOGOS core running the HFT decision over the
// realbist's input ranges (codegen_sva::decision_workload): the CPU twin of decision_hft_realbist.
// Its boot ROM (program.hex, the emitted soft_core_realbist.hex) draws the inputs from a 15-bit LCG,
// decides inline and shows signed_order * 4 + order_side once per decision. Emitted by
// `cargo run -p logicaffeine-compile --example emit_soft_core_realbist`.
module soft_core_realbist(input clk, input rst, output [31:0] out, output out_valid, output halted);
  soft_core_scalar core(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
endmodule
