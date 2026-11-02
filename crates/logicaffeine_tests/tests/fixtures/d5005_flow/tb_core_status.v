// The bring-up core alone at its status port: a fill of 200 lines on one channel that takes every line at once, the
// core reporting progress every eight of its cycles. The status port takes nothing while a progress record is owed
// and the fill has not finished, then opens the clock the fill's own record is owed beside a progress record, so both
// stand at the port at once: the fill's record must be the one taken, with its code and the count of completed
// commands, and the core idle after it, no progress record after it.
`timescale 1ps/1ps
module tb;
  parameter NB = 1, AW = 27, DWM = 512, HAW = 42, DWH = 512, LW = 32, LINES = 200;
  reg clk = 0, rst_n = 0;
  always #1000 clk = ~clk;
  reg cmd_valid = 0; wire cmd_ready; reg [255:0] cmd_data = 0;
  wire st_valid, st_ready; wire [255:0] st_data;
  wire [NB-1:0] md_valid, mr_ready, mw_valid, mdone_ready, mr_taken;
  reg [NB-1:0] mdone_valid = 0;
  wire [NB*(AW+LW+1)-1:0] md_data;
  wire [NB*DWM-1:0] mw_data;
  wire [NB*21-1:0] mw_flip;
  wire hrd_valid, hr_ready, hwd_valid, hw_valid, hack_ready, card_mode, eng_start, eng_host_wr, port_clear;
  wire [HAW+LW-1:0] hrd_data, hwd_data;
  wire [DWH-1:0] hw_data, eng_host_wdata;
  wire [31:0] eng_job_at, eng_job_end, eng_host_addr, eng_host_raddr, port_space;
  wire [15:0] eng_rows; wire [3:0] eng_knobs; wire [2:0] eng_host_table; wire [AW-1:0] port_base; wire [4:0] port_granule_log2;
  // The status port opens unless a progress record is owed before the fill has finished.
  assign st_ready = !(u.beat && !u.reported);
  assign mr_taken = mr_ready;
  wire moved = |(mw_valid & {NB{1'b1}});
  logos_bringup_core #(.NB(NB), .AW(AW), .DWM(DWM), .ECC(0), .ENGINE(0), .HAW(HAW), .DWH(DWH), .LW(LW), .HB(3)) u(
    .clk(clk), .ce(1'b1), .rst_n(rst_n), .cmd_valid(cmd_valid), .cmd_ready(cmd_ready), .cmd_data(cmd_data),
    .st_valid(st_valid), .st_ready(st_ready), .st_data(st_data), .md_valid(md_valid), .md_ready({NB{1'b1}}),
    .md_data(md_data), .mr_valid({NB{1'b0}}), .mr_ready(mr_ready), .mr_data({(NB*DWM){1'b0}}), .mr_ecc({(NB*16){1'b0}}),
    .mw_valid(mw_valid), .mw_ready({NB{1'b1}}), .mw_data(mw_data), .mw_flip(mw_flip), .mdone_valid(mdone_valid),
    .mdone_ready(mdone_ready), .hrd_valid(hrd_valid), .hrd_ready(1'b1), .hrd_data(hrd_data), .hr_valid(1'b0),
    .hr_ready(hr_ready), .hr_data({DWH{1'b0}}), .hwd_valid(hwd_valid), .hwd_ready(1'b1), .hwd_data(hwd_data),
    .hw_valid(hw_valid), .hw_ready(1'b1), .hw_data(hw_data), .hack_valid(1'b0), .hack_ready(hack_ready),
    .mr_taken(mr_taken), .moved(moved), .card_mode(card_mode), .eng_start(eng_start), .eng_job_at(eng_job_at),
    .eng_job_end(eng_job_end), .eng_rows(eng_rows), .eng_knobs(eng_knobs), .eng_busy(1'b0), .eng_fault(1'b0),
    .eng_pc(32'd0), .eng_host_wr(eng_host_wr), .eng_host_table(eng_host_table), .eng_host_addr(eng_host_addr),
    .eng_host_wdata(eng_host_wdata), .eng_host_raddr(eng_host_raddr), .eng_host_rdata({DWH{1'b0}}),
    .port_space(port_space), .port_base(port_base), .port_granule_log2(port_granule_log2), .port_clear(port_clear),
    .port_fault(1'b0));
  integer written = 0, records = 0, progress = 0, finals = 0, after = 0, both = 0, errors = 0;
  // The channel takes every line at once and reports the write descriptor done once it has the fill's every line.
  always @(posedge clk) begin
    mdone_valid <= 0;
    if (mw_valid[0]) begin
      written = written + 1;
      if (written == LINES) mdone_valid <= 1;
    end
  end
  always @(posedge clk) if (rst_n) begin
    if (u.beat && u.reported && st_valid && st_ready) both = both + 1;
    if (st_valid && st_ready) begin
      records = records + 1;
      if (finals != 0) begin after = after + 1; $display("FAIL a record after the fill's own: %h", st_data[127:64]); end
      if (st_data[71:64] == 8'h01) begin
        progress = progress + 1;
        if (st_data[63:0] != 64'd0) begin errors = errors + 1; $display("FAIL a progress record counted %0d commands complete", st_data[63:0]); end
      end else begin
        finals = finals + 1;
        if (st_data[127:64] != {56'h77, 8'h00} || st_data[63:0] != 64'd1) begin
          errors = errors + 1; $display("FAIL the fill's record: status %h, count %0d", st_data[127:64], st_data[63:0]);
        end
      end
    end
  end
  initial begin
    #5000 rst_n = 1;
    @(negedge clk);
    // FILL (1), tag 0x77: channel mask 1, from line 0, LINES lines of the pattern seeded 0x55.
    cmd_data = {32'h55, LINES[31:0], 64'd0, 64'd1, 56'h77, 8'd1};
    // The core is idle, so it takes the command at the next edge.
    cmd_valid = 1;
    @(negedge clk); cmd_valid = 0;
    repeat (2000) @(negedge clk);
    if (finals != 1) begin errors = errors + 1; $display("FAIL %0d records of the fill's own", finals); end
    if (both == 0) begin errors = errors + 1; $display("FAIL the fill's record never stood beside a progress record"); end
    if (!cmd_ready) begin errors = errors + 1; $display("FAIL the core is still busy"); end
    $display("RESULT records=%0d progress=%0d finals=%0d after=%0d both=%0d errors=%0d", records, progress, finals, after, both, errors);
    $finish;
  end
endmodule
