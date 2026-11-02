// tb_storage_controller — replays a REAL compiled NAND stream (storage_plan.hex) and
// checks order + payload + TIMING FAITHFULNESS (gap - delta is a fixed pipeline overhead).
`timescale 1ns/1ps
module tb_storage_controller;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1;
  reg [63:0] rom [0:319];
  integer idx = 0;
  integer issues = 0;
  integer errors = 0;
  integer cyc = 0;
  integer prev_issue = -1;
  integer overhead = -1;
  integer dlt = 0;
  wire [63:0] cmd_word = rom[idx];
  wire cmd_valid = (idx <= 319);
  wire cmd_ack, issue;
  wire [2:0] op; wire [4:0] chan; wire [7:0] die;
  storage_controller dut(.clk(clk), .rst(rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid),
                    .cmd_ack(cmd_ack), .issue(issue), .op(op), .chan(chan), .die(die));
  reg [63:0] expect_word;
  always @(posedge clk) begin
    if (!rst) cyc = cyc + 1;         // global cycle counter (origin irrelevant; gaps are what matter)
    if (cmd_ack) idx <= idx + 1;     // pop on CONSUME (load), not issue
    if (issue) begin
      expect_word = rom[issues];     // issues arrive strictly in word order
      dlt = expect_word[63:40];      // this command's compiled delta
      if (op !== expect_word[39:37] || chan !== expect_word[36:32] || die !== expect_word[31:24]) begin
        errors = errors + 1;
        $display("FAIL: issue %0d payload mismatch", issues);
      end
      // TIMING FAITHFULNESS: gap since the previous issue = delta + a constant pipeline overhead.
      if (prev_issue >= 0) begin
        if (overhead == -1) overhead = (cyc - prev_issue) - dlt;
        else if (((cyc - prev_issue) - dlt) != overhead) begin
          errors = errors + 1;
          $display("FAIL: issue %0d timing skew (gap %0d, delta %0d, overhead %0d != %0d)",
                   issues, cyc - prev_issue, dlt, (cyc - prev_issue) - dlt, overhead);
        end
      end
      prev_issue = cyc;
      issues = issues + 1;
    end
  end
  initial begin
    $readmemh("storage_plan.hex", rom);
    repeat (4) @(posedge clk);
    rst = 0;
    repeat (2000000) @(posedge clk);
    if (issues !== 320) begin
      $display("FAIL: issued %0d of 320 commands", issues);
    end else if (errors == 0) begin
      $display("PASS: %0d commands issued in order, exact payloads, timing faithful (overhead %0d cycles)", issues, overhead);
    end
    $finish;
  end
endmodule
