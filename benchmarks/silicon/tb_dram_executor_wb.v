// tb_dram_executor_wb — drive the command player over Wishbone; check it issues the command.
`timescale 1ns/1ps
module tb_dram_executor_wb;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1;
  reg stb = 0, cyc = 0, we = 0;
  reg [3:0] sel = 4'hf;
  reg [31:0] adr = 0, dat = 0;
  wire ack;  wire [31:0] rdat;  wire [63:0] la;
  integer errors = 0;

  dram_executor_wb dut(
    .wb_clk_i(clk), .wb_rst_i(rst), .wbs_stb_i(stb), .wbs_cyc_i(cyc), .wbs_we_i(we),
    .wbs_sel_i(sel), .wbs_adr_i(adr), .wbs_dat_i(dat), .wbs_ack_o(ack), .wbs_dat_o(rdat),
    .la_data_out(la)
  );

  task wb_write(input [31:0] a, input [31:0] d);
    begin
      @(posedge clk); adr <= a; dat <= d; we <= 1; stb <= 1; cyc <= 1;
      @(posedge clk); while (!ack) @(posedge clk);
      stb <= 0; cyc <= 0; we <= 0;
      @(posedge clk);
    end
  endtask

  initial begin
    repeat (3) @(posedge clk); rst = 0; @(posedge clk);
    // delta-0 command: cmd_word = {24'd0, op=1, bank=3, row=7}
    wb_write(32'h00, 32'd7);       // cmd_word[31:0]  = row
    wb_write(32'h04, 32'h23);      // cmd_word[63:32] = {delta=0, op=1(<<5), bank=3}
    wb_write(32'h08, 32'h1);       // launch (cmd_valid)
    repeat (8) @(posedge clk);
    // la_data_out = {issued, 15'd0, issue_count, bank, op, row}
    if (la[63] !== 1'b1)        begin errors = errors + 1; $display("FAIL: never issued"); end
    if (la[47:40] !== 8'd1)     begin errors = errors + 1; $display("FAIL: issue_count %0d != 1 after one command", la[47:40]); end
    if (la[39:35] !== 5'd3)     begin errors = errors + 1; $display("FAIL: bank %0d != 3", la[39:35]); end
    if (la[34:32] !== 3'd1)     begin errors = errors + 1; $display("FAIL: op %0d != 1", la[34:32]); end
    if (la[31:0] !== 32'd7)     begin errors = errors + 1; $display("FAIL: row %0d != 7", la[31:0]); end
    if (errors == 0) $display("PASS: Wishbone-driven command player issued op=1 bank=3 row=7");
    else $display("FAIL: %0d Wishbone integration errors", errors);
    $finish;
  end
endmodule
