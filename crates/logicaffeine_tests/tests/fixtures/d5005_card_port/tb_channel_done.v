// A hybrid engine's channel path (logos_d5005_channel with its fast port's writes) in front of a memory with random
// waits, the user clock at UCLK_PS against the memory's MEM_PS: the fast side gives NF write runs of LINES lines each,
// back to back, a line a clock; the compute side, stepping one user clock in CDIV, gives NC write runs of a line each,
// one a step. Every done token either side takes is counted; once both sides have given every run and the channel
// has been quiet a while, the counts.
//   RESULT fast_runs=<NF> fast_done=<n> core_runs=<NC> core_done=<n>
`timescale 1ps/1ps
module tb;
  parameter integer UCLK_PS = 20000, MEM_PS = 3334, NF = 400, NC = 64, LINES = 1, CDIV = 8, WAIT_PCT = 0, SEED = 1;
  localparam AW = 27, LW = 32, DWM = 512, MBW = 7, DLW = AW + LW + 1;
  reg uclk = 0, mem_clk = 0, urst_n = 0, mrst_n = 0;
  always #(UCLK_PS / 2) uclk = ~uclk;
  initial begin #(MEM_PS / 3) forever #(MEM_PS / 2) mem_clk = ~mem_clk; end
  // The compute side's step: one user clock in CDIV.
  reg uce = 0;
  integer cstep = 0;
  always @(posedge uclk) begin
    cstep <= (cstep + 1) % CDIV;
    uce <= cstep == CDIV - 2 || CDIV == 1;
  end
  reg            fd_valid = 0, fw_valid = 0, md_valid = 0, mw_valid = 0;
  reg  [DLW-1:0] fd_data = 0, md_data = 0;
  wire           fd_ready, fw_ready, fr_valid, fdone_valid, md_ready, mw_ready, mr_valid, mdone_valid;
  wire [DWM-1:0] fr_data, mr_data;
  wire [15:0]    mr_ecc;
  wire [AW-1:0]  mem_address;
  wire           mem_read, mem_write, mem_waitrequest, mem_readdatavalid;
  wire [MBW-1:0] mem_burstcount;
  wire [DWM-1:0] mem_writedata, mem_readdata;
  wire [DWM/8-1:0] mem_byteenable;
  logos_d5005_channel #(.AW(AW), .DWM(DWM), .ECC(0), .MBW(MBW), .LW(LW), .MAXB(64), .RAW(6), .WAW(4)) ch(
    .uclk(uclk), .uce(uce), .uclk_rst_n(urst_n), .mem_clk(mem_clk), .mem_rst_n(mrst_n),
    .md_valid(md_valid), .md_ready(md_ready), .md_data(md_data), .mr_valid(mr_valid), .mr_ready(1'b1), .mr_data(mr_data),
    .mr_ecc(mr_ecc), .mw_valid(mw_valid), .mw_ready(mw_ready), .mw_data({DWM{1'b1}}), .mw_flip(21'd0),
    .mdone_valid(mdone_valid), .mdone_ready(1'b1),
    .fclk(uclk), .fclk_rst_n(urst_n), .fd_valid(fd_valid), .fd_ready(fd_ready), .fd_data(fd_data),
    .fr_valid(fr_valid), .fr_ready(1'b1), .fr_data(fr_data), .fw_valid(fw_valid), .fw_ready(fw_ready),
    .fw_data({DWM{1'b0}}), .fdone_valid(fdone_valid), .fdone_ready(1'b1),
    .mem_address(mem_address), .mem_read(mem_read), .mem_write(mem_write), .mem_burstcount(mem_burstcount),
    .mem_writedata(mem_writedata), .mem_byteenable(mem_byteenable), .mem_waitrequest(mem_waitrequest),
    .mem_readdata(mem_readdata), .mem_readdatavalid(mem_readdatavalid));
  logos_avmm_memory_model #(.AW(AW), .DW(DWM), .BW(MBW), .LINES_LOG2(14), .SEED(SEED), .WAIT_PCT(WAIT_PCT), .GAP_PCT(0)) memory(
    .clk(mem_clk), .rst_n(mrst_n), .address(mem_address), .read(mem_read), .write(mem_write),
    .burstcount(mem_burstcount), .writedata(mem_writedata), .byteenable(mem_byteenable),
    .waitrequest(mem_waitrequest), .readdata(mem_readdata), .readdatavalid(mem_readdatavalid));

  // The fast side: runs of LINES lines at lines 2·LINES apart, every run's lines given a clock each.
  integer f_given = 0, f_lines = 0, f_done = 0, c_given = 0, c_lines = 0, c_done = 0, quiet = 0;
  reg [AW-1:0] f_at, c_at;
  wire [LW-1:0] run_lines = LINES;
  always @(posedge uclk) begin
    if (urst_n) begin
      if (fdone_valid) f_done = f_done + 1;
      if (fd_valid && fd_ready) f_given = f_given + 1;
      if (fw_valid && fw_ready) f_lines = f_lines + 1;
      f_at = f_given * LINES * 2;
      fd_valid <= f_given < NF;
      fd_data <= {1'b1, f_at, run_lines};
      fw_valid <= f_lines < NF * LINES;
    end
  end
  // The compute side: runs of a line at lines 8192 on, one a step.
  always @(posedge uclk) begin
    if (urst_n && uce) begin
      if (mdone_valid) c_done = c_done + 1;
      if (md_valid && md_ready) c_given = c_given + 1;
      if (mw_valid && mw_ready) c_lines = c_lines + 1;
      c_at = 8192 + c_given * 2;
      md_valid <= c_given < NC;
      md_data <= {1'b1, c_at, 32'd1};
      mw_valid <= c_lines < NC;
    end
  end
  always @(posedge mem_clk) quiet <= (mem_write || mem_read) ? 0 : quiet + 1;
  initial begin
    #(UCLK_PS * 10) mrst_n = 1; urst_n = 1;
    wait (f_given == NF && f_lines == NF * LINES && c_given == NC && c_lines == NC);
    wait (quiet > 4000);
    #(UCLK_PS * CDIV * 64);
    $display("RESULT fast_runs=%0d fast_done=%0d core_runs=%0d core_done=%0d", NF, f_done, NC, c_done);
    $finish;
  end
endmodule
