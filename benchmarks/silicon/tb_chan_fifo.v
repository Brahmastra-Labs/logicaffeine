// tb_chan_fifo — HW bounded FIFO vs the software channel (VecDeque) oracle.
`timescale 1ns/1ps
module tb_chan_fifo;
  localparam N = 200;
  localparam M = 109;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1, push = 0, pop = 0;
  reg [31:0] din;
  reg [7:0]  ctrl [0:199];
  reg [31:0] dvec [0:199];
  reg [31:0] pops [0:108];
  wire [31:0] dout; wire pushed, popped, empty, full; wire [3:0] count;
  integer i, k, errors;
  chan_fifo dut(.clk(clk), .rst(rst), .push(push), .din(din), .pop(pop),
                .dout(dout), .pushed(pushed), .popped(popped),
                .empty(empty), .full(full), .count(count));
  initial begin
    errors = 0; k = 0;
    $readmemh("chan_ctrl.hex", ctrl);
    $readmemh("chan_din.hex", dvec);
    $readmemh("chan_pops.hex", pops);
    repeat (3) @(posedge clk); #1 rst = 0;
    for (i = 0; i < N; i = i + 1) begin
      push = ctrl[i][0]; pop = ctrl[i][1]; din = dvec[i]; #1;   // settle combinational
      if (popped) begin
        if (k >= M || dout !== pops[k]) begin errors = errors + 1; $display("FAIL pop %0d", k); end
        k = k + 1;
      end
      @(posedge clk); #1;                                       // advance FIFO state
    end
    push = 0; pop = 0; #1;
    if (k !== M) begin errors = errors + 1; $display("FAIL: %0d pops, expected %0d", k, M); end
    if (count !== 5) begin errors = errors + 1; $display("FAIL final count %0d != 5", count); end
    if (errors == 0) $display("PASS: 200-step FIFO trace matches the software channel oracle (%0d pops)", k);
    else $display("FAIL: %0d FIFO mismatches", errors);
    $finish;
  end
endmodule
