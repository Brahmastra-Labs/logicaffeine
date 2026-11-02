// ddr2_init_fsm — DDR2 power-up / mode-register sequencer. The step ROM is GENERATED from
// codegen_sva::ddr2_phy::ddr2_init_sequence (the JEDEC init spec); each entry drives
// {cke, cs_n, ras_n, cas_n, we_n, ba, a} for `wait` cycles, then advances. init_done latches high.
module ddr2_init_fsm(
  input             clk,
  input             rst,
  output reg        cke,
  output reg        cs_n,
  output reg        ras_n,
  output reg        cas_n,
  output reg        we_n,
  output reg [1:0]  ba,
  output reg [12:0] a,
  output reg        init_done
);
  localparam NSTEPS = 24;
  reg [43:0] rom [0:NSTEPS-1];
  initial begin
    rom[0] = 44'h780000061a8; // Deselect
    rom[1] = 44'hb8000000190; // Nop
    rom[2] = 44'h90400000000; // Precharge
    rom[3] = 44'hb8000000005; // Nop
    rom[4] = 44'h84000000000; // LoadMode
    rom[5] = 44'hb8000000002; // Nop
    rom[6] = 44'h86000000000; // LoadMode
    rom[7] = 44'hb8000000002; // Nop
    rom[8] = 44'h82000000000; // LoadMode
    rom[9] = 44'hb8000000002; // Nop
    rom[10] = 44'h80b53000000; // LoadMode
    rom[11] = 44'hb8000000002; // Nop
    rom[12] = 44'h90400000000; // Precharge
    rom[13] = 44'hb8000000005; // Nop
    rom[14] = 44'h88000000000; // Refresh
    rom[15] = 44'hb8000000033; // Nop
    rom[16] = 44'h88000000000; // Refresh
    rom[17] = 44'hb8000000033; // Nop
    rom[18] = 44'h80a53000000; // LoadMode
    rom[19] = 44'hb80000000c8; // Nop
    rom[20] = 44'h82380000000; // LoadMode
    rom[21] = 44'hb8000000002; // Nop
    rom[22] = 44'h82000000000; // LoadMode
    rom[23] = 44'hb8000000002; // Nop
  end
  integer step;
  reg [43:0] cur;
  reg [23:0] wait_left;
  always @(posedge clk) begin
    if (rst) begin
      step <= 0; cur <= rom[0]; wait_left <= rom[0][23:0]; init_done <= 1'b0;
    end else if (!init_done) begin
      if (wait_left == 24'd0) begin
        if (step == NSTEPS-1) init_done <= 1'b1;
        else begin step <= step + 1; cur <= rom[step+1]; wait_left <= rom[step+1][23:0]; end
      end else begin
        wait_left <= wait_left - 24'd1;
      end
    end
  end
  always @(*) begin
    if (init_done) begin
      cke=1'b1; cs_n=1'b0; ras_n=1'b1; cas_n=1'b1; we_n=1'b1; ba=2'b0; a=13'b0; // NOP idle
    end else begin
      cke=cur[43]; cs_n=cur[42]; ras_n=cur[41]; cas_n=cur[40]; we_n=cur[39];
      ba=cur[38:37]; a=cur[36:24];
    end
  end
endmodule
