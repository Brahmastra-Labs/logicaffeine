// fabric_horner_seq — the combinational compiled-residual datapath (fabric_horner) wrapped in
// input and output registers so the RTL-to-GDSII flow can TIME it. The reg-to-reg path IS the
// fabric's single-cycle latency: Fmax = 1 / (datapath propagation delay). No program counter, no
// fetch, no opcode dispatch — the whole residual resolves in ONE clock. fabric_horner itself is
// proven == the running VM by iverilog (tb_fabric_horner); this wrapper only adds the pipe flops
// the static-timing flow needs to report a clock.
module fabric_horner_seq(input clk, input [63:0] in0, output reg [63:0] result);
  reg  [63:0] in0_r;
  wire [63:0] y;
  fabric_horner u_fab(.in0(in0_r), .result(y));
  always @(posedge clk) begin
    in0_r  <= in0;
    result <= y;
  end
endmodule
