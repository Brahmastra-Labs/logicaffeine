// Host memory behind the PIM's Avalon-MM read/write port, for simulation: separate read and write channels,
// waitrequest at random on each, read data in order after a random latency, one write response a burst.
module logos_host_memory_model #(parameter AW = 51, parameter DW = 512, parameter BW = 7, parameter LINES_LOG2 = 12,
                                 parameter SEED = 11, parameter WAIT_PCT = 25) (
  input               clk,
  input               rst_n,
  input  [AW-1:0]     rd_address,
  input               rd_read,
  input  [BW-1:0]     rd_burstcount,
  output reg          rd_waitrequest,
  output reg [DW-1:0] rd_readdata,
  output reg          rd_readdatavalid,
  input  [AW-1:0]     wr_address,
  input               wr_write,
  input  [BW-1:0]     wr_burstcount,
  input  [DW-1:0]     wr_writedata,
  input  [DW/8-1:0]   wr_byteenable,
  output reg          wr_waitrequest,
  output reg          wr_writeresponsevalid
);
  reg [DW-1:0] mem [0:(1<<LINES_LOG2)-1];
  integer seed = SEED, cycle = 0, qh = 0, qt = 0, beat = 0, rh = 0, rt = 0;
  reg [AW-1:0] q_addr [0:255];
  reg [BW-1:0] q_len [0:255];
  integer      q_due [0:255];
  integer      r_due [0:255];
  reg [AW-1:0] w_addr;
  reg [BW-1:0] w_left = 0;
  always @(posedge clk) begin
    cycle = cycle + 1;
    rd_readdatavalid <= 1'b0;
    wr_writeresponsevalid <= 1'b0;
    if (rst_n) begin
      if (rd_read && !rd_waitrequest) begin
        q_addr[qt % 256] = rd_address; q_len[qt % 256] = rd_burstcount;
        q_due[qt % 256] = cycle + 20 + (($random(seed) % 40 + 40) % 40); qt = qt + 1;
      end
      if (wr_write && !wr_waitrequest) begin
        if (w_left == 0) begin
          mem[wr_address[LINES_LOG2-1:0]] = wr_writedata; w_addr = wr_address + 1; w_left = wr_burstcount - 1;
        end else begin
          mem[w_addr[LINES_LOG2-1:0]] = wr_writedata; w_addr = w_addr + 1; w_left = w_left - 1;
        end
        if (w_left == 0) begin r_due[rt % 256] = cycle + 5 + (($random(seed) % 30 + 30) % 30); rt = rt + 1; end
      end
      if (qh != qt && q_due[qh % 256] <= cycle) begin
        rd_readdata <= mem[(q_addr[qh % 256] + beat) % (1 << LINES_LOG2)];
        rd_readdatavalid <= 1'b1;
        beat = beat + 1;
        if (beat == q_len[qh % 256]) begin beat = 0; qh = qh + 1; end
      end
      if (rh != rt && r_due[rh % 256] <= cycle) begin wr_writeresponsevalid <= 1'b1; rh = rh + 1; end
      rd_waitrequest <= (($random(seed) % 100 + 100) % 100) < WAIT_PCT;
      wr_waitrequest <= (($random(seed) % 100 + 100) % 100) < WAIT_PCT;
    end else begin
      rd_waitrequest <= 1'b1;
      wr_waitrequest <= 1'b1;
    end
  end
endmodule
