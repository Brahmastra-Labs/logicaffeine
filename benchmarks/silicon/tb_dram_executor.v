// tb_dram_executor — replays a REAL Wave-2 command stream (dram_plan.hex).
`timescale 1ns/1ps
module tb_dram_executor;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1;
  reg [63:0] rom [0:135];
  integer idx = 0;
  integer issues = 0;
  integer errors = 0;
  wire [63:0] cmd_word = rom[idx];
  wire cmd_valid = (idx <= 135);
  wire cmd_ack, issue;
  wire [2:0] op; wire [4:0] bank; wire [31:0] row;
  dram_executor dut(.clk(clk), .rst(rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid),
                    .cmd_ack(cmd_ack), .issue(issue), .op(op), .bank(bank), .row(row));
  reg [63:0] expect_word;
  always @(posedge clk) begin
    if (cmd_ack) idx <= idx + 1;   // pop on CONSUME (load), not issue
    if (issue) begin
      expect_word = rom[issues];   // issues arrive strictly in word order
      if (op !== expect_word[39:37] || bank !== expect_word[36:32] || row !== expect_word[31:0]) begin
        errors = errors + 1;
        $display("FAIL: issue %0d payload mismatch", issues);
      end
      issues = issues + 1;
    end
  end
  initial begin
    $readmemh("dram_plan.hex", rom);
    repeat (4) @(posedge clk);
    rst = 0;
    // Run long enough for every delta in the plan slice.
    repeat (200000) @(posedge clk);
    if (issues !== 136) begin
      $display("FAIL: issued %0d of 136 commands", issues);
    end else if (errors == 0) begin
      $display("PASS: %0d commands issued strictly in order with exact payloads", issues);
    end
    $finish;
  end
endmodule
