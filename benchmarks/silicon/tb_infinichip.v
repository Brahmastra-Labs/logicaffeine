// tb_infinichip — self-checking integration of the composed memory subsystem.
`timescale 1ns/1ps
module tb_infinichip;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1;
  reg [63:0] cmd_word = 0; reg cmd_valid = 0;
  wire cmd_ack, issue; wire [2:0] op; wire [4:0] bank; wire [31:0] row;
  reg [63:0] wr_data = 64'hDEAD_BEEF_1234_5678;
  wire [71:0] wr_cw;
  reg [71:0] rd_cw = 0;
  wire [63:0] rd_data; wire rd_corrected, rd_double_err;
  reg swap = 0; wire read_bank, fill_bank;
  integer errors = 0;
  reg saw_issue = 0;

  infinichip dut(.clk(clk), .rst(rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid),
                 .cmd_ack(cmd_ack), .issue(issue), .op(op), .bank(bank), .row(row),
                 .wr_data(wr_data), .wr_cw(wr_cw),
                 .rd_cw(rd_cw), .rd_data(rd_data), .rd_corrected(rd_corrected),
                 .rd_double_err(rd_double_err),
                 .swap(swap), .read_bank(read_bank), .fill_bank(fill_bank));

  // A delta-0 command issues the cycle after it loads; latch and check its payload.
  always @(posedge clk) if (issue) begin
    saw_issue <= 1'b1;
    if (op !== 3'd1 || bank !== 5'd3 || row !== 32'd7) errors = errors + 1;
  end

  initial begin
    repeat (3) @(posedge clk);
    rst = 0;
    @(posedge clk);

    // (1) ECC read path corrects a single-bit fault (position 7 = a Hamming data bit), its
    //     verdict registered one edge after the codeword arrives.
    #1 rd_cw = wr_cw ^ (72'b1 << 7);
    @(posedge clk); #1;
    if (rd_data !== wr_data)    begin errors = errors + 1; $display("FAIL: ECC did not recover data"); end
    if (rd_corrected !== 1'b1)  begin errors = errors + 1; $display("FAIL: ECC did not flag corrected"); end
    if (rd_double_err !== 1'b0) begin errors = errors + 1; $display("FAIL: ECC false double-error"); end

    // (2) Double-buffer disjointness holds now and across a swap.
    if (fill_bank !== ~read_bank) begin errors = errors + 1; $display("FAIL: banks not disjoint"); end
    swap = 1; @(posedge clk); swap = 0; @(posedge clk);
    if (fill_bank !== ~read_bank) begin errors = errors + 1; $display("FAIL: banks not disjoint after swap"); end

    // (3) Command player issues a loaded (delta-0) command with the exact payload.
    cmd_word = {24'd0, 3'd1, 5'd3, 32'd7}; cmd_valid = 1;
    repeat (6) @(posedge clk);
    cmd_valid = 0;
    repeat (2) @(posedge clk);
    if (!saw_issue) begin errors = errors + 1; $display("FAIL: command player never issued"); end

    if (errors == 0)
      $display("PASS: infinichip integration - command player + SECDED ECC + double buffer");
    else
      $display("FAIL: %0d integration errors", errors);
    $finish;
  end
endmodule
