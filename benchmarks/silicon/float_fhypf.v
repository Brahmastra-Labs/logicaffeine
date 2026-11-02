// float_fhypf — a pure-float-arithmetic datapath as a SEQUENCED controller binding fpu.v. One of each
// multi-cycle IEEE-754 unit (fp_addsub/fp_mul/fp_div/fp_sqrt) is SHARED across cells; a micro-sequencer
// muxes each cell's operands to the selected unit, pulses start, waits done, latches the result. The
// streamed `result` is the f64 BIT-PATTERN of the final float. Proven == the model (eval_grid over f64
// bits) == Rust f64 by iverilog; fpu.v is bit-exact to the VM.
module float_fhypf(input clk, input rst, input [63:0] in0, input [63:0] in1, output [63:0] result, output done);
  localparam M = 4;
  (* ram_style = "registers" *) reg [63:0] R [0:M-1];
  reg [31:0] step;
  reg running, issued;
  reg [63:0] opa, opb; reg [2:0] usel;
  always @(*) begin
   opa = 64'd0; opb = 64'd0; usel = 3'd0;
   case (step)
        0: begin opa = in0; opb = in0; usel = 3'd2; end
        1: begin opa = in1; opb = in1; usel = 3'd2; end
        2: begin opa = R[0]; opb = R[1]; usel = 3'd0; end
        3: begin opa = R[2]; opb = 64'd0; usel = 3'd4; end
    default: ;
   endcase
  end
  wire is_addsub = (usel == 3'd0) || (usel == 3'd1);
  wire is_sub = (usel == 3'd1);
  wire is_mul = (usel == 3'd2);
  wire is_div = (usel == 3'd3);
  wire is_sqrt = (usel == 3'd4);
  // the start pulse fires for exactly the selected unit in the cycle a step is issued.
  wire pulse = running && !issued;
  wire add_done, mul_done, div_done, sqrt_done;
  wire [63:0] add_y, mul_y, div_y, sqrt_y;
  fp_addsub u_add(.clk(clk), .rst(rst), .start(pulse && is_addsub), .a(opa), .b_in(opb), .sub(is_sub), .y(add_y), .done(add_done));
  fp_mul    u_mul(.clk(clk), .rst(rst), .start(pulse && is_mul), .a(opa), .b(opb), .y(mul_y), .done(mul_done));
  fp_div    u_div(.clk(clk), .rst(rst), .start(pulse && is_div), .a(opa), .b(opb), .y(div_y), .done(div_done));
  fp_sqrt   u_sqrt(.clk(clk), .rst(rst), .start(pulse && is_sqrt), .a(opa), .y(sqrt_y), .done(sqrt_done));
  wire sel_done = is_addsub ? add_done : is_mul ? mul_done : is_div ? div_done : sqrt_done;
  wire [63:0] sel_y = is_addsub ? add_y : is_mul ? mul_y : is_div ? div_y : sqrt_y;
  always @(posedge clk) begin
   if (rst) begin step <= 32'd0; running <= (M > 0); issued <= 1'b0; end
   else if (running) begin
    if (!issued) issued <= 1'b1;            // the start pulse fired this cycle
    else if (sel_done) begin
     R[step] <= sel_y; issued <= 1'b0;
     if (step == M - 1) running <= 1'b0; else step <= step + 32'd1;
    end
   end
  end
  assign done = !running;
  assign result = R[3];
endmodule
