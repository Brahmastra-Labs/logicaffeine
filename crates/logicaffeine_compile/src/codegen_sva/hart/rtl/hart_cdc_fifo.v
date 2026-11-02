// hart_cdc_fifo — a first-in first-out queue between two unrelated clocks: 2^DEPTH_BITS entries (DEPTH_BITS ≥ 2),
// written on wclk (`push` while not `full`) and read on rclk (`head` while not `empty`, `pop` takes it). Each side
// sees the other's pointer in Gray code through two flip-flops, so a pointer caught mid-change is off by at most one
// entry, and only ever towards caution: the writer may think the queue fuller than it is, the reader emptier.
module hart_cdc_fifo #(
  parameter WIDTH = 8,
  parameter DEPTH_BITS = 4
) (
  input              wclk,
  input              wrst,
  input              push,
  input  [WIDTH-1:0] din,
  output             full,
  input              rclk,
  input              rrst,
  input              pop,
  output [WIDTH-1:0] head,
  output             empty
);
  (* ram_style = "registers" *) reg [WIDTH-1:0] mem [0:(1 << DEPTH_BITS) - 1];
  reg [DEPTH_BITS:0] wbin, wgray, rbin, rgray;
  (* ASYNC_REG = "TRUE" *) reg [DEPTH_BITS:0] rgray_w1, rgray_w2;
  (* ASYNC_REG = "TRUE" *) reg [DEPTH_BITS:0] wgray_r1, wgray_r2;

  wire [DEPTH_BITS:0] wbin_next = wbin + 1'b1;
  wire [DEPTH_BITS:0] rbin_next = rbin + 1'b1;
  assign full = wgray == {~rgray_w2[DEPTH_BITS:DEPTH_BITS - 1], rgray_w2[DEPTH_BITS - 2:0]};
  assign empty = rgray == wgray_r2;
  assign head = mem[rbin[DEPTH_BITS - 1:0]];

  always @(posedge wclk) begin
    if (wrst) begin
      wbin <= 0;
      wgray <= 0;
      rgray_w1 <= 0;
      rgray_w2 <= 0;
    end else begin
      rgray_w1 <= rgray;
      rgray_w2 <= rgray_w1;
      if (push && !full) begin
        mem[wbin[DEPTH_BITS - 1:0]] <= din;
        wbin <= wbin_next;
        wgray <= wbin_next ^ (wbin_next >> 1);
      end
    end
  end

  always @(posedge rclk) begin
    if (rrst) begin
      rbin <= 0;
      rgray <= 0;
      wgray_r1 <= 0;
      wgray_r2 <= 0;
    end else begin
      wgray_r1 <= wgray;
      wgray_r2 <= wgray_r1;
      if (pop && !empty) begin
        rbin <= rbin_next;
        rgray <= rbin_next ^ (rbin_next >> 1);
      end
    end
  end
endmodule
