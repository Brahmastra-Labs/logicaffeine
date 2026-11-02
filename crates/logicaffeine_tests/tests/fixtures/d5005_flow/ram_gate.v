// The forms of memory the RAM gate (scripts/d5005/ram_check.py) tells apart, each a top of its own.

// A row's held inputs as the vector side once held them: a memory a row inside a generate block, written a beat at a
// time and read at a registered address. Quartus built every one in flip-flops.
module ram_gate_scoped #(parameter ROWS = 2, D = 1088) (input clk, input we, input [0:0] row, input [10:0] wa,
  input [511:0] wd, input [10:0] ra, output [512*ROWS-1:0] q);
  genvar g;
  generate
    for (g = 0; g < ROWS; g = g + 1) begin : held
      reg [511:0] act [0:D-1];
      reg [511:0] aq;
      always @(posedge clk) begin
        if (we && row == g) act[wa] <= wd;
        aq <= act[ra];
      end
      assign q[512*g +: 512] = aq;
    end
  endgenerate
endmodule

// The same rows each a block-RAM wrapper (`rtl::ram::bram_verilog`, read beside this file).
module ram_gate_wrapped #(parameter ROWS = 2, D = 1088) (input clk, input we, input [0:0] row, input [10:0] wa,
  input [511:0] wd, input [10:0] ra, output [512*ROWS-1:0] q);
  genvar g;
  generate
    for (g = 0; g < ROWS; g = g + 1) begin : held
      logos_bram_w512 #(.D(D), .A(11)) act(.clk(clk), .ra(ra), .rd(q[512*g +: 512]), .we(we && row == g), .wa(wa), .wd(wd),
        .wm({64{1'b1}}));
    end
  endgenerate
endmodule

// Small memories anywhere: a queue in a generate block and a table at the module's scope, each under the least size and
// in the MLABs its sixteen words fill.
module ram_gate_small #(parameter ROWS = 2) (input clk, input we, input [3:0] wa, input [511:0] wd, input [3:0] ra,
  output [512*ROWS-1:0] q, output reg [63:0] t);
  (* ramstyle = "MLAB" *) reg [63:0] table_ [0:15];
  always @(posedge clk) begin
    if (we) table_[wa] <= wd[63:0];
    t <= table_[ra];
  end
  genvar g;
  generate
    for (g = 0; g < ROWS; g = g + 1) begin : held
      (* ramstyle = "MLAB" *) reg [511:0] fifo [0:15];
      reg [511:0] fq;
      always @(posedge clk) begin
        if (we) fifo[wa] <= wd;
        fq <= fifo[ra];
      end
      assign q[512*g +: 512] = fq;
    end
  endgenerate
endmodule

// A large memory at its module's own scope, one write and a read registered on the rising edge: the form of the fold's,
// the flush's and the read-out's memories, which Quartus builds in M20K blocks.
module ram_gate_owned #(parameter D = 1024) (input clk, input we, input [9:0] wa, input [511:0] wd, input [9:0] ra,
  output reg [511:0] q);
  reg [511:0] kmem [0:D-1];
  always @(posedge clk) begin
    if (we) kmem[wa] <= wd;
    q <= kmem[ra];
  end
endmodule

// Memories of thirty-two words or fewer as a build once left them, asking for nothing or for M20K blocks: a column core's
// ring of two decoded beats read without a register after it, a line queue of sixteen beats, a one-bit table of
// thirty-two words and a delay of eight words asking for M20K. Quartus spent an M20K block on each forty bits of each
// one's width (hybrid9c2: 5,142 blocks for 2.2 Mbit). A memory of sixty-four words is past the rule and passes as it is.
module ram_gate_shallow (input clk, input we, input [5:0] wa, input [5:0] ra, input [5279:0] wd, output [5279:0] held,
  output reg [511:0] lq, output tq, output reg [1183:0] dq, output reg [511:0] deep_q);
  reg [5279:0] ring [0:1];
  reg [511:0] lines [0:15];
  reg flags [0:31];
  (* ramstyle = "M20K" *) reg [1183:0] delay [0:7];
  reg [511:0] deep [0:63];
  always @(posedge clk) begin
    if (we) begin
      ring[wa[0]] <= wd;
      lines[wa[3:0]] <= wd[511:0];
      flags[wa[4:0]] <= wd[0];
      delay[wa[2:0]] <= wd[1183:0];
      deep[wa] <= wd[511:0];
    end
    lq <= lines[ra[3:0]];
    dq <= delay[ra[2:0]];
    deep_q <= deep[ra];
  end
  assign held = ring[ra[0]];
  assign tq = flags[ra[4:0]];
endmodule

// The same memories each asking for what its depth fills: the ring in registers, the queue, the table and the delay in
// MLABs; the sixty-four-word memory asking for nothing.
module ram_gate_shallow_kept (input clk, input we, input [5:0] wa, input [5:0] ra, input [5279:0] wd, output [5279:0] held,
  output reg [511:0] lq, output tq, output reg [1183:0] dq, output reg [511:0] deep_q);
  (* ramstyle = "logic" *) reg [5279:0] ring [0:1];
  (* ramstyle = "MLAB" *) reg [511:0] lines [0:15];
  (* ramstyle = "MLAB, no_rw_check" *) reg flags [0:31];
  (* ramstyle = "MLAB" *) reg [1183:0] delay [0:7];
  reg [511:0] deep [0:63];
  always @(posedge clk) begin
    if (we) begin
      ring[wa[0]] <= wd;
      lines[wa[3:0]] <= wd[511:0];
      flags[wa[4:0]] <= wd[0];
      delay[wa[2:0]] <= wd[1183:0];
      deep[wa] <= wd[511:0];
    end
    lq <= lines[ra[3:0]];
    dq <= delay[ra[2:0]];
    deep_q <= deep[ra];
  end
  assign held = ring[ra[0]];
  assign tq = flags[ra[4:0]];
endmodule

// Memories in MLABs by their reads: a four-word queue whose head is read with no register after it and a sixteen-word
// memory read at a registered address (both seeing a word the clock before wrote it), each asking for MLABs alone, which
// Quartus builds in flip-flops; and sixteen-word memories read into a register, on every clock and where an enable is
// high, which an MLAB's output register holds.
module ram_gate_unchecked (input clk, input rst, input push, input pop, input we, input re, input [3:0] wa, input [3:0] ra,
  input [527:0] d, output [527:0] head, output [511:0] at, output reg [511:0] q, output reg [511:0] q_held);
  (* ramstyle = "MLAB" *) reg [527:0] queue [0:3];
  (* ramstyle = "MLAB" *) reg [511:0] lines [0:15];
  (* ramstyle = "MLAB" *) reg [511:0] kept [0:15];
  (* ramstyle = "MLAB" *) reg [511:0] held [0:15];
  reg [1:0] wr, rd;
  reg [3:0] ra_q;
  assign head = queue[rd];
  assign at = lines[ra_q];
  always @(posedge clk) begin
    if (push) begin queue[wr] <= d; wr <= wr + 2'd1; end
    if (pop) rd <= rd + 2'd1;
    if (rst) begin wr <= 2'd0; rd <= 2'd0; end
    if (we) begin lines[wa] <= d[511:0]; kept[wa] <= d[511:0]; held[wa] <= d[511:0]; end
    ra_q <= ra;
    q <= kept[ra];
    if (re) q_held <= held[ra];
  end
endmodule
