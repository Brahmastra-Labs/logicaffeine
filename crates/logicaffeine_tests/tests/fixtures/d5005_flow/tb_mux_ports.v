// The card's port mix on logos_channel_mux: PORTS fast ports reading and writing beside a reader whose four channels step
// together through a credit a channel. The reader is four logos_image_stream engines, each streaming tiles of LEAST to
// MOST lines off its own channel, their beats taken all four in one clock (as the streaming unit's feeds deal to an array
// that steps once every core has its beat), each engine asking its channel through logos_read_credit. Each fast port is
// a logos_fast_port on granules of 8 lines dealt to the channels in turn: it reads runs of consecutive lines (1 to RUN,
// within its own region of the space) and writes runs (within a region of its own no read reaches), a write a WRITES
// percent of its runs; every read returns in the order it was asked, as the line lies, and every write line reaches its
// channel at the place its line lies, carrying its port and line. Each channel serves the descriptors it took in the
// order it took them: a read's lines on clocks it picks at random, each valid until taken; a write's lines out of the
// channel's write FIFO (WFIFO lines, as the shell's fast write FIFO, which takes every line the multiplexer gives while
// it has room, ahead of its descriptor) on clocks it picks at random, its done token given once its last is out and held
// until the multiplexer takes it. The run ends when every beat, every read and every write is through, or when nothing
// has moved for STALL clocks (hung); a port whose `writing` is ever unknown after the reset is an error, and its writes
// are never through.
`timescale 1ps/1ps
module tb;
  parameter SEED = 1, PORTS = 3, TILES = 6, LEAST = 300, MOST = 1900, OPS = 400, RUN = 64, WRITES = 30, GAP = 30,
    LAT = 60, WFIFO = 64, STALL = 200000;
  localparam AW = 27, LW = 32, DL = 512, MDW = AW + LW + 1, P = PORTS + 1;
  reg clk = 0, rst = 1;
  always #500 clk = ~clk;

  function [DL-1:0] pattern(input [1:0] ch, input [AW-1:0] at);
    integer i;
    begin
      for (i = 0; i < 16; i = i + 1) pattern[32*i +: 32] = {ch, at[25:0], i[3:0]} ^ (32'h9e3779b9 * (i + 1));
    end
  endfunction
  // A write line's word: its port and its line, and the line's pattern beside them.
  function [DL-1:0] written(input [7:0] port, input [31:0] line);
    written = {pattern(line[4:3], line[AW-1:0])} ^ {{(DL-40){1'b0}}, port, line};
  endfunction

  // The multiplexer: ports 0 to PORTS-1 the fast ports', port PORTS the reader's.
  wire [4*P-1:0] pd_valid, pd_ready, pr_valid, pr_ready, pw_valid, pw_ready, pdone_valid, pdone_ready;
  wire [4*P*MDW-1:0] pd_data;
  wire [4*P*DL-1:0] pw_data;
  wire [3:0] md_valid, mw_valid, mdone_ready;
  reg [3:0] md_ready = 0, mr_valid = 0, mdone_valid = 0, mw_ready = 0;
  wire [3:0] mr_ready;
  wire [4*MDW-1:0] md_data;
  wire [4*DL-1:0] mw_data;
  reg [4*DL-1:0] mr_data = 0;
  logos_channel_mux #(.P(P), .NB(4), .AW(AW), .LW(LW), .DL(DL)) mux(.clk(clk), .rst(rst), .pd_valid(pd_valid),
    .pd_ready(pd_ready), .pd_data(pd_data), .pr_valid(pr_valid), .pr_ready(pr_ready), .pw_valid(pw_valid),
    .pw_ready(pw_ready), .pw_data(pw_data), .pdone_valid(pdone_valid), .pdone_ready(pdone_ready), .md_valid(md_valid),
    .md_ready(md_ready), .md_data(md_data), .mr_valid(mr_valid), .mr_ready(mr_ready), .mw_valid(mw_valid),
    .mw_ready(mw_ready), .mw_data(mw_data), .mdone_valid(mdone_valid), .mdone_ready(mdone_ready));

  // The fast ports, on granules of 8 lines dealt to the channels in turn from line 0.
  reg [PORTS-1:0] rq_valid = 0, wq_valid = 0;
  reg [32*PORTS-1:0] rq_line = 0, wq_line = 0;
  reg [DL*PORTS-1:0] wq_data = 0;
  wire [PORTS-1:0] rq_ready, rs_valid, wq_ready, writing, fault;
  wire [DL*PORTS-1:0] rs_data;
  genvar f;
  generate for (f = 0; f < PORTS; f = f + 1) begin : fast
    logos_fast_port #(.NB(4), .AW(AW), .LW(LW), .DL(DL)) port(.clk(clk), .rst(rst), .space(32'd0), .base({AW{1'b0}}),
      .granule_log2(5'd3), .clear(1'b0), .fault(fault[f]), .rq_valid(rq_valid[f]), .rq_ready(rq_ready[f]),
      .rq_line(rq_line[32*f +: 32]), .rs_valid(rs_valid[f]), .rs_data(rs_data[DL*f +: DL]), .wq_valid(wq_valid[f]),
      .wq_ready(wq_ready[f]), .wq_line(wq_line[32*f +: 32]), .wq_data(wq_data[DL*f +: DL]), .writing(writing[f]),
      .md_valid(pd_valid[4*f +: 4]), .md_ready(pd_ready[4*f +: 4]), .md_data(pd_data[4*f*MDW +: 4*MDW]),
      .mr_valid(pr_valid[4*f +: 4]), .mr_ready(pr_ready[4*f +: 4]), .mr_data(mr_data), .mw_valid(pw_valid[4*f +: 4]),
      .mw_ready(pw_ready[4*f +: 4]), .mw_data(pw_data[4*f*DL +: 4*DL]), .mdone_valid(pdone_valid[4*f +: 4]),
      .mdone_ready(pdone_ready[4*f +: 4]));
  end endgenerate
  assign pw_valid[4*PORTS +: 4] = 4'd0;
  assign pw_data[4*PORTS*DL +: 4*DL] = {(4*DL){1'b0}};
  assign pdone_ready[4*PORTS +: 4] = 4'hf;

  // The reader: each channel's engine, fed its tiles in order, through a credit.
  reg [3:0] out_ready = 0;
  wire [3:0] out_valid, tile_ready;
  wire [4*DL-1:0] out_data;
  reg [3:0] tile_valid = 0;
  reg [AW-1:0] tile_line [0:3];
  reg [31:0] tile_lines [0:3];
  genvar g;
  generate for (g = 0; g < 4; g = g + 1) begin : reader
    wire s_md_valid, s_md_ready, s_mr_valid, s_mr_ready;
    wire [MDW-1:0] s_md_data;
    wire [DL-1:0] s_mr_data;
    wire [6:0] keep;
    wire last, crc_error;
    wire [15:0] tensor;
    wire [31:0] row;
    wire [23:0] offset;
    logos_image_stream stream(.clk(clk), .rst_n(!rst), .tile_valid(tile_valid[g]), .tile_ready(tile_ready[g]),
      .tile_line(tile_line[g]), .tile_lines(tile_lines[g]), .tile_bytes({tile_lines[g][17:0], 6'd0}),
      .tile_tensor(16'd0), .tile_row(32'd0), .md_valid(s_md_valid), .md_ready(s_md_ready), .md_data(s_md_data),
      .mr_valid(s_mr_valid), .mr_ready(s_mr_ready), .mr_data(s_mr_data), .out_valid(out_valid[g]),
      .out_ready(out_ready[g]), .out_data(out_data[DL*g +: DL]), .out_keep(keep), .out_last(last),
      .out_tensor(tensor), .out_row(row), .out_offset(offset), .out_crc_error(crc_error));
    logos_read_credit #(.AW(AW), .LW(LW), .DL(DL)) k(.clk(clk), .rst(rst), .in_valid(s_md_valid),
      .in_ready(s_md_ready), .in_data(s_md_data), .out_valid(s_mr_valid), .out_ready(s_mr_ready),
      .out_data(s_mr_data), .md_valid(pd_valid[4*PORTS + g]), .md_ready(pd_ready[4*PORTS + g]),
      .md_data(pd_data[(4*PORTS + g)*MDW +: MDW]), .mr_valid(pr_valid[4*PORTS + g]), .mr_ready(pr_ready[4*PORTS + g]),
      .mr_data(mr_data[DL*g +: DL]));
  end endgenerate

  // The tiles: the same sizes on every channel, each channel's tiles one after another from its own first line.
  integer size [0:TILES-1];
  integer tile_at [0:3], tile_from [0:3], beat_tile [0:3], beat_in [0:3], beat_from [0:3];
  // Each channel's descriptors taken (first line, lines, whether a write, the clock it may start), in order.
  reg [AW-1:0] q_line [0:63];
  reg q_write [0:63];
  integer q_n [0:63], q_ready [0:63], q_head [0:3], q_tail [0:3], q_given [0:3], done_pend [0:3];
  // Each channel's write FIFO.
  reg [DL-1:0] wf_data [0:4*WFIFO-1];
  integer wf_head [0:3], wf_tail [0:3];
  // Each fast port's runs: the one being asked (its kind, next line, lines left), its reads asked and not yet back.
  reg [31:0] asked [0:16383];
  integer ask_head [0:PORTS-1], ask_tail [0:PORTS-1];
  integer op_n [0:PORTS-1], op_left [0:PORTS-1], op_at [0:PORTS-1], reads_back [0:PORTS-1];
  reg op_write [0:PORTS-1];
  integer seed, now, moved, errors, beats, beats_all, reads_all, writes_all, writes_in, c, k, total, p, done_tokens;
  integer reads_done, ops_done;
  reg unknown = 0;
  reg presented [0:3];
  reg [31:0] x, logical;
  reg [AW-1:0] local_line;
  reg [DL-1:0] want;

  initial begin
    seed = SEED; now = 0; moved = 0; errors = 0; beats = 0; total = 0; reads_all = 0; writes_all = 0; writes_in = 0;
    done_tokens = 0;
    for (k = 0; k < TILES; k = k + 1) begin
      size[k] = LEAST + $urandom(seed) % (MOST - LEAST + 1);
      total = total + size[k];
    end
    beats_all = 4 * total;
    for (c = 0; c < 4; c = c + 1) begin
      tile_at[c] = 0; tile_from[c] = (1 << 20) + 65536 * c; beat_tile[c] = 0; beat_in[c] = 0; beat_from[c] = tile_from[c];
      q_head[c] = 0; q_tail[c] = 0; q_given[c] = 0; presented[c] = 0; done_pend[c] = 0; wf_head[c] = 0; wf_tail[c] = 0;
    end
    for (p = 0; p < PORTS; p = p + 1) begin
      ask_head[p] = 0; ask_tail[p] = 0; op_n[p] = 0; op_left[p] = 0; reads_back[p] = 0; op_write[p] = 0;
    end
    repeat (4) @(posedge clk);
    @(negedge clk) rst = 0;
    reads_done = 0; ops_done = 0;
    while (!(beats >= beats_all && ops_done && reads_done && writing === 0) && now - moved < STALL) begin
      @(negedge clk);
      now = now + 1;
      if (^writing === 1'bx && !unknown) begin unknown = 1; errors = errors + 1; end
      // What every side offers this clock.
      for (c = 0; c < 4; c = c + 1) begin
        tile_valid[c] = tile_at[c] < TILES;
        tile_line[c] = tile_from[c];
        tile_lines[c] = tile_at[c] < TILES ? size[tile_at[c]] : 1;
        md_ready[c] = q_tail[c] - q_head[c] < 16 && $urandom(seed) % 100 >= GAP;
        if (!presented[c])
          presented[c] = q_head[c] != q_tail[c] && !q_write[16*c + q_head[c] % 16] && now >= q_ready[16*c + q_head[c] % 16]
            && $urandom(seed) % 100 >= GAP;
        mr_valid[c] = presented[c];
        mr_data[DL*c +: DL] = pattern(c[1:0], q_line[16*c + q_head[c] % 16] + q_given[c]);
        mw_ready[c] = wf_tail[c] - wf_head[c] < WFIFO && $urandom(seed) % 100 >= GAP;
        mdone_valid[c] = done_pend[c] > 0;
      end
      out_ready = {4{&out_valid && $urandom(seed) % 100 >= GAP}};
      for (p = 0; p < PORTS; p = p + 1) begin
        if (op_left[p] == 0 && op_n[p] < OPS) begin
          op_write[p] = $urandom(seed) % 100 < WRITES;
          op_left[p] = 1 + $urandom(seed) % RUN;
          op_at[p] = (op_write[p] ? (1 << 23) : 0) + 65536 * p + $urandom(seed) % (65536 - RUN);
          op_n[p] = op_n[p] + 1;
        end
        rq_valid[p] = op_left[p] != 0 && !op_write[p] && ask_tail[p] - ask_head[p] < 4000 && $urandom(seed) % 100 < 80;
        wq_valid[p] = op_left[p] != 0 && op_write[p] && $urandom(seed) % 100 < 80;
        rq_line[32*p +: 32] = op_at[p];
        wq_line[32*p +: 32] = op_at[p];
        wq_data[DL*p +: DL] = written(p[7:0], op_at[p]);
      end
      #100;
      // What the coming edge moves.
      for (c = 0; c < 4; c = c + 1) begin
        if (tile_valid[c] && tile_ready[c]) begin
          tile_from[c] = tile_from[c] + size[tile_at[c]]; tile_at[c] = tile_at[c] + 1; moved = now;
        end
        if (md_valid[c] && md_ready[c]) begin
          q_write[16*c + q_tail[c] % 16] = md_data[c*MDW + MDW - 1];
          q_line[16*c + q_tail[c] % 16] = md_data[c*MDW + LW +: AW];
          q_n[16*c + q_tail[c] % 16] = md_data[c*MDW +: LW];
          q_ready[16*c + q_tail[c] % 16] = now + $urandom(seed) % (LAT + 1);
          q_tail[c] = q_tail[c] + 1;
          moved = now;
        end
        if (mr_valid[c] && mr_ready[c]) begin
          presented[c] = 0;
          q_given[c] = q_given[c] + 1;
          if (q_given[c] == q_n[16*c + q_head[c] % 16]) begin q_given[c] = 0; q_head[c] = q_head[c] + 1; end
          moved = now;
        end
        if (mdone_valid[c] && mdone_ready[c]) begin done_pend[c] = done_pend[c] - 1; done_tokens = done_tokens + 1; moved = now; end
        // A write at the head takes a line out of the FIFO: the line it lies at, back to its place in the space
        // (granule (local / 8) * 4 + c), carries its port and that place.
        if (q_head[c] != q_tail[c] && q_write[16*c + q_head[c] % 16] && now >= q_ready[16*c + q_head[c] % 16]
            && wf_tail[c] != wf_head[c] && $urandom(seed) % 100 >= GAP) begin
          local_line = q_line[16*c + q_head[c] % 16] + q_given[c];
          logical = (((local_line >> 3) * 4 + c) << 3) | (local_line & 7);
          want = wf_data[WFIFO*c + wf_head[c] % WFIFO] ^ pattern(logical[4:3], logical[AW-1:0]);
          if (want[31:0] !== logical || want[39:32] >= PORTS || want[DL-1:40] !== 0) errors = errors + 1;
          wf_head[c] = wf_head[c] + 1;
          writes_in = writes_in + 1;
          q_given[c] = q_given[c] + 1;
          if (q_given[c] == q_n[16*c + q_head[c] % 16]) begin
            q_given[c] = 0; q_head[c] = q_head[c] + 1; done_pend[c] = done_pend[c] + 1;
          end
          moved = now;
        end
        if (mw_valid[c] && mw_ready[c]) begin
          wf_data[WFIFO*c + wf_tail[c] % WFIFO] = mw_data[DL*c +: DL];
          wf_tail[c] = wf_tail[c] + 1;
          moved = now;
        end
      end
      if (&out_ready) begin
        for (c = 0; c < 4; c = c + 1) begin
          if (out_data[DL*c +: DL] !== pattern(c[1:0], beat_from[c] + beat_in[c])) errors = errors + 1;
          beat_in[c] = beat_in[c] + 1;
          if (beat_in[c] == size[beat_tile[c]]) begin
            beat_from[c] = beat_from[c] + size[beat_tile[c]];
            beat_tile[c] = beat_tile[c] + 1;
            beat_in[c] = 0;
          end
        end
        beats = beats + 4;
        moved = now;
      end
      for (p = 0; p < PORTS; p = p + 1) begin
        if (rq_valid[p] && rq_ready[p]) begin
          asked[4096*p + ask_tail[p] % 4096] = op_at[p];
          ask_tail[p] = ask_tail[p] + 1; reads_all = reads_all + 1;
          op_at[p] = op_at[p] + 1; op_left[p] = op_left[p] - 1;
          moved = now;
        end
        if (wq_valid[p] && wq_ready[p]) begin
          writes_all = writes_all + 1;
          op_at[p] = op_at[p] + 1; op_left[p] = op_left[p] - 1;
          moved = now;
        end
        if (rs_valid[p]) begin
          x = asked[4096*p + ask_head[p] % 4096];
          local_line = (x & 32'd7) | ((x >> 2) & ~32'd7);
          if (rs_data[DL*p +: DL] !== pattern(x[4:3], local_line)) errors = errors + 1;
          ask_head[p] = ask_head[p] + 1; reads_back[p] = reads_back[p] + 1;
          moved = now;
        end
      end
      ops_done = 1;
      reads_done = 1;
      for (p = 0; p < PORTS; p = p + 1) begin
        if (op_n[p] < OPS || op_left[p] != 0) ops_done = 0;
        if (ask_head[p] != ask_tail[p]) reads_done = 0;
      end
    end
    $display("RESULT beats=%0d of %0d reads=%0d writes=%0d of %0d runs=%0d errors=%0d hung=%0d clocks=%0d",
      beats, beats_all, reads_all, writes_in, writes_all, done_tokens, errors, now - moved >= STALL, now);
    $finish;
  end
endmodule
