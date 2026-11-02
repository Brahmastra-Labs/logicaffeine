// One memory channel's stream engine against a memory with random waitrequest and latency: lines written in bursts,
// read back through a dual-clock FIFO a slow consumer drains, the FIFO never full when a line arrives.
`timescale 1ps/1ps
module tb;
  parameter AW = 27, DW = 64, BW = 7, LW = 32, RAW = 6, LINES = 300, BASE = 100, SEED = 3, RP = 30, STOP = 0;
  reg clk = 0, rclk = 0, rst_n = 0;
  always #1667 clk = ~clk;
  always #1111 rclk = ~rclk;
  // descriptor source (in clk for this test)
  reg d_valid = 0; reg [AW+LW:0] d_data = 0; wire d_ready;
  // write data source
  reg w_valid = 0; reg [DW-1:0] w_data = 0; wire w_ready;
  // read data FIFO into rclk
  wire r_valid; wire [DW-1:0] r_data; wire [RAW:0] wfree; wire fifo_wready;
  wire q_valid; reg q_ready = 0; reg [7:0] stop_clock = 0; wire [DW-1:0] q_data; wire [RAW:0] rused;
  wire [AW-1:0] address; wire read, write; wire [BW-1:0] burstcount; wire [DW-1:0] writedata; wire [DW/8-1:0] byteenable;
  wire waitrequest, readdatavalid; wire [DW-1:0] readdata; wire [RAW:0] owed; wire busy, wdone;
  logos_stream_avmm #(.AW(AW), .DW(DW), .BW(BW), .LW(LW), .MAXB(16), .RW(RAW+1)) eng(
    .clk(clk), .rst_n(rst_n), .d_valid(d_valid), .d_ready(d_ready), .d_data(d_data),
    .r_valid(r_valid), .r_data(r_data), .r_room(wfree), .w_valid(w_valid), .w_ready(w_ready), .w_data(w_data),
    .w_flip(21'h0), .avm_address(address), .avm_read(read), .avm_write(write), .avm_burstcount(burstcount), .avm_writedata(writedata),
    .avm_byteenable(byteenable), .avm_waitrequest(waitrequest), .avm_readdata(readdata), .avm_readdatavalid(readdatavalid),
    .owed(owed), .busy(busy), .wdone(wdone));
  logos_avmm_memory_model #(.AW(AW), .DW(DW), .BW(BW), .SEED(SEED)) memory(.clk(clk), .rst_n(rst_n), .address(address),
    .read(read), .write(write), .burstcount(burstcount), .writedata(writedata), .byteenable(byteenable),
    .waitrequest(waitrequest), .readdata(readdata), .readdatavalid(readdatavalid));
  logos_dcfifo_m20k #(.W(DW), .AW(RAW)) rfifo(.wclk(clk), .wce(1'b1), .wrst_n(rst_n), .wvalid(r_valid), .wready(fifo_wready), .wdata(r_data), .wfree(wfree),
    .rclk(rclk), .rce(1'b1), .rrst_n(rst_n), .rvalid(q_valid), .rready(q_ready), .rdata(q_data), .rused(rused));
  function [DW-1:0] pattern(input integer line); pattern = {line[31:0] * 32'h9e3779b1, line[31:0] ^ 32'h5a5a5a5a}; endfunction
  integer seed = SEED, errors = 0, written = 0, got = 0;
  always @(posedge clk) if (r_valid && !fifo_wready) begin errors = errors + 1; $display("a read line arrived at a full FIFO"); end
  always @(posedge clk) if (rst_n && (read || write) && (burstcount == 0 || burstcount > 16)) begin errors = errors + 1; $display("burstcount %0d", burstcount); end
  // write data: lines BASE.. in order
  always @(posedge clk) if (rst_n) begin
    if (w_valid && w_ready) begin written = written + 1; end
    if (!w_valid || w_ready) begin
      if (written < LINES && (($random(seed) % 100 + 100) % 100) < 80) begin
        w_valid <= 1; w_data <= pattern(BASE + written);
      end else w_valid <= 0;
    end
  end
  always @(posedge rclk) if (rst_n) begin
    if (q_valid && q_ready) begin
      if (q_data !== pattern(BASE + got)) begin errors = errors + 1; if (errors < 5) $display("line %0d read %h", BASE + got, q_data); end
      got = got + 1;
    end
    // With STOP set the consumer drains for 64 of every 256 of its clocks and then stops dead, so the FIFO fills to
    // its last line and a burst reserved one line too many overflows it.
    stop_clock <= stop_clock + 1;
    q_ready <= ((($random(seed) % 100 + 100) % 100) < RP) && (STOP == 0 || stop_clock[7:6] == 2'd0);
  end
  initial begin
    #20000 rst_n = 1;
    @(posedge clk); d_data <= {1'b1, BASE[AW-1:0], LINES[LW-1:0]}; d_valid <= 1;
    @(posedge clk); while (!d_ready) @(posedge clk); d_valid <= 0;
    @(posedge clk); while (eng.busy || written < LINES) @(posedge clk);
    repeat (5) @(posedge clk);
    d_data <= {1'b0, BASE[AW-1:0], LINES[LW-1:0]}; d_valid <= 1;
    @(posedge clk); while (!d_ready) @(posedge clk); d_valid <= 0;
    wait (got == LINES);
    repeat (20) @(posedge clk);
    $display("RESULT written=%0d got=%0d errors=%0d owed=%0d", written, got, errors, owed);
    $finish;
  end
  initial begin #5000000000; $display("RESULT timeout written=%0d got=%0d", written, got); $finish; end
endmodule
