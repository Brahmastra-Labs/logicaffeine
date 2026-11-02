// gpu_simt — a tuneable SIMT data-parallel engine: 4 spatial lanes, each running the
// SAME compiled straight-line kernel (the shader) on its own data, gated by a per-lane mask (SIMT divergence), then a cross-lane warp-reduce (raddr==N).
// The lanes are WIRED (spatial) — no warp scheduler, no instruction fetch/decode/opcode
// dispatch. The per-lane FMA is a SEQUENTIAL shift-add over 64 bits (one reused adder per lane, bounded fan-out → antenna-trivial); a result is ready 65 cycles after `go` (done pulses). Narrow streaming I/O: `din` is loaded slot-by-slot into the lane
// register file, `raddr` reads results back. SIMD = all lanes unmasked; map-then-reduce
// feeds the NPU accumulator. 64-bit, wrapping like the VM's Int; proven ≡ the VM by our SAT.
module gpu_simt(input clk, input rst, input load, input [63:0] din,
                 input go, input [7:0] raddr, output [63:0] dout, output reg done);
  reg [3:0] mask;
  reg [63:0] x0_l0;
  reg [63:0] x1_l0;
  reg [63:0] x2_l0;
  reg [63:0] x0_l1;
  reg [63:0] x1_l1;
  reg [63:0] x2_l1;
  reg [63:0] x0_l2;
  reg [63:0] x1_l2;
  reg [63:0] x2_l2;
  reg [63:0] x0_l3;
  reg [63:0] x1_l3;
  reg [63:0] x2_l3;
  reg [63:0] acc_l0;
  reg [63:0] mc_l0;
  reg [63:0] mp_l0;
  reg [63:0] acc_l1;
  reg [63:0] mc_l1;
  reg [63:0] mp_l1;
  reg [63:0] acc_l2;
  reg [63:0] mc_l2;
  reg [63:0] mp_l2;
  reg [63:0] acc_l3;
  reg [63:0] mc_l3;
  reg [63:0] mp_l3;
  reg [63:0] outr0;
  reg [63:0] outr1;
  reg [63:0] outr2;
  reg [63:0] outr3;
  reg [63:0] redr;
  reg [15:0] lc;
  reg [7:0] scnt;
  reg running;
  (* keep *) wire msel0_l0 = mp_l0[0];
  (* keep *) wire msel1_l0 = mp_l0[0];
  (* keep *) wire msel2_l0 = mp_l0[0];
  (* keep *) wire msel3_l0 = mp_l0[0];
  (* keep *) wire msel4_l0 = mp_l0[0];
  (* keep *) wire msel5_l0 = mp_l0[0];
  (* keep *) wire msel6_l0 = mp_l0[0];
  (* keep *) wire msel7_l0 = mp_l0[0];
  wire [63:0] pp_l0 = { {8{msel7_l0}} & mc_l0[63:56],
                   {8{msel6_l0}} & mc_l0[55:48],
                   {8{msel5_l0}} & mc_l0[47:40],
                   {8{msel4_l0}} & mc_l0[39:32],
                   {8{msel3_l0}} & mc_l0[31:24],
                   {8{msel2_l0}} & mc_l0[23:16],
                   {8{msel1_l0}} & mc_l0[15:8],
                   {8{msel0_l0}} & mc_l0[7:0] };
  wire [63:0] k_l0 = acc_l0 + x2_l0;
  wire [63:0] g_l0 = mask[0] ? k_l0 : 64'sd0;
  (* keep *) wire msel0_l1 = mp_l1[0];
  (* keep *) wire msel1_l1 = mp_l1[0];
  (* keep *) wire msel2_l1 = mp_l1[0];
  (* keep *) wire msel3_l1 = mp_l1[0];
  (* keep *) wire msel4_l1 = mp_l1[0];
  (* keep *) wire msel5_l1 = mp_l1[0];
  (* keep *) wire msel6_l1 = mp_l1[0];
  (* keep *) wire msel7_l1 = mp_l1[0];
  wire [63:0] pp_l1 = { {8{msel7_l1}} & mc_l1[63:56],
                   {8{msel6_l1}} & mc_l1[55:48],
                   {8{msel5_l1}} & mc_l1[47:40],
                   {8{msel4_l1}} & mc_l1[39:32],
                   {8{msel3_l1}} & mc_l1[31:24],
                   {8{msel2_l1}} & mc_l1[23:16],
                   {8{msel1_l1}} & mc_l1[15:8],
                   {8{msel0_l1}} & mc_l1[7:0] };
  wire [63:0] k_l1 = acc_l1 + x2_l1;
  wire [63:0] g_l1 = mask[1] ? k_l1 : 64'sd0;
  (* keep *) wire msel0_l2 = mp_l2[0];
  (* keep *) wire msel1_l2 = mp_l2[0];
  (* keep *) wire msel2_l2 = mp_l2[0];
  (* keep *) wire msel3_l2 = mp_l2[0];
  (* keep *) wire msel4_l2 = mp_l2[0];
  (* keep *) wire msel5_l2 = mp_l2[0];
  (* keep *) wire msel6_l2 = mp_l2[0];
  (* keep *) wire msel7_l2 = mp_l2[0];
  wire [63:0] pp_l2 = { {8{msel7_l2}} & mc_l2[63:56],
                   {8{msel6_l2}} & mc_l2[55:48],
                   {8{msel5_l2}} & mc_l2[47:40],
                   {8{msel4_l2}} & mc_l2[39:32],
                   {8{msel3_l2}} & mc_l2[31:24],
                   {8{msel2_l2}} & mc_l2[23:16],
                   {8{msel1_l2}} & mc_l2[15:8],
                   {8{msel0_l2}} & mc_l2[7:0] };
  wire [63:0] k_l2 = acc_l2 + x2_l2;
  wire [63:0] g_l2 = mask[2] ? k_l2 : 64'sd0;
  (* keep *) wire msel0_l3 = mp_l3[0];
  (* keep *) wire msel1_l3 = mp_l3[0];
  (* keep *) wire msel2_l3 = mp_l3[0];
  (* keep *) wire msel3_l3 = mp_l3[0];
  (* keep *) wire msel4_l3 = mp_l3[0];
  (* keep *) wire msel5_l3 = mp_l3[0];
  (* keep *) wire msel6_l3 = mp_l3[0];
  (* keep *) wire msel7_l3 = mp_l3[0];
  wire [63:0] pp_l3 = { {8{msel7_l3}} & mc_l3[63:56],
                   {8{msel6_l3}} & mc_l3[55:48],
                   {8{msel5_l3}} & mc_l3[47:40],
                   {8{msel4_l3}} & mc_l3[39:32],
                   {8{msel3_l3}} & mc_l3[31:24],
                   {8{msel2_l3}} & mc_l3[23:16],
                   {8{msel1_l3}} & mc_l3[15:8],
                   {8{msel0_l3}} & mc_l3[7:0] };
  wire [63:0] k_l3 = acc_l3 + x2_l3;
  wire [63:0] g_l3 = mask[3] ? k_l3 : 64'sd0;
  wire [63:0] creduced = $signed(g_l0) + $signed(g_l1) + $signed(g_l2) + $signed(g_l3);
  assign dout = raddr == 0 ? outr0 : raddr == 1 ? outr1 : raddr == 2 ? outr2 : raddr == 3 ? outr3 : redr;
  always @(posedge clk) begin
    if (rst) begin
      lc <= 16'd0; done <= 1'b0; running <= 1'b0;
    end else begin
      if (load) begin
        case (lc)
        0: mask <= din[3:0];
        1: x0_l0 <= din;
        2: x1_l0 <= din;
        3: x2_l0 <= din;
        4: x0_l1 <= din;
        5: x1_l1 <= din;
        6: x2_l1 <= din;
        7: x0_l2 <= din;
        8: x1_l2 <= din;
        9: x2_l2 <= din;
        10: x0_l3 <= din;
        11: x1_l3 <= din;
        12: x2_l3 <= din;
         default: ;
        endcase
        lc <= lc + 16'd1;
      end
      if (go) begin running <= 1'b1; scnt <= 8'd64; done <= 1'b0;
        acc_l0 <= 64'd0; mc_l0 <= x0_l0; mp_l0 <= x1_l0;
        acc_l1 <= 64'd0; mc_l1 <= x0_l1; mp_l1 <= x1_l1;
        acc_l2 <= 64'd0; mc_l2 <= x0_l2; mp_l2 <= x1_l2;
        acc_l3 <= 64'd0; mc_l3 <= x0_l3; mp_l3 <= x1_l3;
      end
      else if (running) begin
        if (scnt > 8'd0) begin
         acc_l0 <= acc_l0 + pp_l0;
         mc_l0  <= mc_l0 << 1;
         mp_l0  <= mp_l0 >> 1;
         acc_l1 <= acc_l1 + pp_l1;
         mc_l1  <= mc_l1 << 1;
         mp_l1  <= mp_l1 >> 1;
         acc_l2 <= acc_l2 + pp_l2;
         mc_l2  <= mc_l2 << 1;
         mp_l2  <= mp_l2 >> 1;
         acc_l3 <= acc_l3 + pp_l3;
         mc_l3  <= mc_l3 << 1;
         mp_l3  <= mp_l3 >> 1;
         scnt <= scnt - 8'd1;
        end else begin
        outr0 <= g_l0;
        outr1 <= g_l1;
        outr2 <= g_l2;
        outr3 <= g_l3;
        redr <= creduced;
         done <= 1'b1; running <= 1'b0;
        end
      end
    end
  end
endmodule
