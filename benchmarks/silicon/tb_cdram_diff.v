// tb_cdram_diff — prove the EXTENDED ComputeDRAM engine (with Ambit NOT, op 5) computes SET
// DIFFERENCE a\b = MAJ(a, ~b, 0) in-DRAM, ≡ the oracle, under iverilog. The stream
// (computedram_diff.hex) is: NOT(row4 := ~row2), then the triple-row majority MAJ(row1,row4,row3=0).
`timescale 1ns/1ps
module tb_cdram_diff;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1;
  localparam NW = 8;
  reg [63:0] rom [0:NW-1];
  integer idx = 0;
  wire [63:0] cmd_word = rom[idx];
  wire cmd_valid = (idx < NW);
  wire cmd_ack, issue; wire [2:0] op; wire [4:0] bank; wire [31:0] row;
  reg pre_we; reg [2:0] pre_addr; reg [63:0] pre_data; reg [2:0] rd_addr; wire [63:0] rd_out;

  dram_executor exec(.clk(clk), .rst(rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid),
    .cmd_ack(cmd_ack), .issue(issue), .op(op), .bank(bank), .row(row));
  computedram_model #(.AW(3), .DW(64)) dut(.clk(clk), .rst(rst), .issue(issue), .op(op),
    .bank(bank), .row(row), .pre_we(pre_we), .pre_addr(pre_addr), .pre_data(pre_data),
    .rd_addr(rd_addr), .rd_out(rd_out));

  always @(posedge clk) if (cmd_ack) idx <= idx + 1;
  integer nissue = 0;
  always @(posedge clk) if (issue) begin nissue = nissue + 1; $display("  issue #%0d op=%0d bank=%0d row=%0d", nissue, op, bank, row); end

  localparam [63:0] A = 64'hF0F0F0F0F0F0F0F0;
  localparam [63:0] B = 64'hFF00FF00FF00FF00;
  localparam [63:0] EXP = A & ~B; // = 0x00F000F000F000F0

  initial begin
    $readmemh("computedram_diff.hex", rom);
    pre_we = 0; pre_addr = 0; pre_data = 0; rd_addr = 0;
    repeat (3) @(posedge clk);
    // preload operands (pre_we takes effect even under reset). Set signals BEFORE the edge the model
    // samples them on — set-then-wait avoids the sample/drive race.
    pre_we = 1; pre_addr = 3'd1; pre_data = A;    @(posedge clk); #1;
    pre_addr = 3'd2; pre_data = B;                @(posedge clk); #1;
    pre_addr = 3'd3; pre_data = 64'd0;            @(posedge clk); #1;
    pre_we = 0;                                    @(posedge clk); #1;
    rst = 0;                                       @(posedge clk); #1;
    $display("preload: mem1=%h mem2=%h mem3=%h", dut.mem[1], dut.mem[2], dut.mem[3]);
    repeat (200) @(posedge clk);
    $display("after:   mem1=%h mem2=%h mem3=%h mem4=%h  issues=%0d idx=%0d", dut.mem[1], dut.mem[2], dut.mem[3], dut.mem[4], nissue, idx);
    rd_addr = 3'd1; @(posedge clk); @(posedge clk);
    if (rd_out === EXP)
      $display("PASS: in-DRAM difference A\\B = %h (Ambit NOT + majority)", rd_out);
    else
      $display("FAIL: got %h expected %h", rd_out, EXP);
    $finish;
  end
endmodule
