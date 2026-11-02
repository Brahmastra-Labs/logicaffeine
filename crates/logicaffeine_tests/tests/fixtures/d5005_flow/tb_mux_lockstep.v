// A reader whose four channels step together, beside a fast port, on logos_channel_mux. The reader is four
// logos_image_stream engines, each streaming tiles of the same sizes (LEAST to MOST lines, drawn at random) off its own
// channel, their beats taken all four in one clock, as the streaming unit's feeds deal to an array that steps once
// every core has its beat. The fast port is a logos_fast_port reading lines across the channels, each read returned in
// the order it was asked. With CREDIT each engine asks its channel through logos_read_credit. Each channel serves the
// descriptors it took in the order it took them, a line on a clock it picks at random, each line valid until taken; a
// line's bytes name its channel and line. The run ends when every beat and every read is back, each checked against
// its line, or when nothing has moved for STALL clocks (hung). Under CREDIT every run the multiplexer takes for the
// reader is checked for 1..32 lines, and the lines asked of a channel and not yet taken by its engine for at most 512.
// With RATE the fast port reads nothing and every channel answers a line every clock, LAT clocks after it takes a
// descriptor: the clocks from the first beat to the last are counted.
`timescale 1ps/1ps
module tb;
  parameter SEED = 1, CREDIT = 1, RATE = 0, TILES = 40, LEAST = 1, MOST = 150, READS = 600, GAP = 30, LAT = 60,
    STALL = 20000;
  localparam AW = 27, LW = 32, DL = 512, MDW = AW + LW + 1;
  reg clk = 0, rst = 1;
  always #500 clk = ~clk;

  function [DL-1:0] pattern(input [1:0] ch, input [AW-1:0] at);
    integer i;
    begin
      for (i = 0; i < 16; i = i + 1) pattern[32*i +: 32] = {ch, at[25:0], i[3:0]} ^ (32'h9e3779b9 * (i + 1));
    end
  endfunction

  // The multiplexer: port 0 the fast port's, port 1 the reader's.
  wire [7:0] pd_valid, pd_ready, pr_valid, pr_ready, pw_valid, pw_ready, pdone_valid, pdone_ready;
  wire [8*MDW-1:0] pd_data;
  wire [8*DL-1:0] pw_data;
  wire [3:0] md_valid, mw_valid, mdone_ready;
  reg [3:0] md_ready = 0, mr_valid = 0, mdone_valid = 0, mw_ready = 4'hf;
  wire [3:0] mr_ready;
  wire [4*MDW-1:0] md_data;
  wire [4*DL-1:0] mw_data;
  reg [4*DL-1:0] mr_data = 0;
  logos_channel_mux #(.P(2), .NB(4), .AW(AW), .LW(LW), .DL(DL)) mux(.clk(clk), .rst(rst), .pd_valid(pd_valid),
    .pd_ready(pd_ready), .pd_data(pd_data), .pr_valid(pr_valid), .pr_ready(pr_ready), .pw_valid(pw_valid),
    .pw_ready(pw_ready), .pw_data(pw_data), .pdone_valid(pdone_valid), .pdone_ready(pdone_ready), .md_valid(md_valid),
    .md_ready(md_ready), .md_data(md_data), .mr_valid(mr_valid), .mr_ready(mr_ready), .mw_valid(mw_valid),
    .mw_ready(mw_ready), .mw_data(mw_data), .mdone_valid(mdone_valid), .mdone_ready(mdone_ready));

  // The fast port, on granules of 8 lines dealt to the channels in turn from line 0.
  reg rq_valid = 0;
  reg [31:0] rq_line = 0;
  wire rq_ready, rs_valid, wq_ready, writing, fault;
  wire [DL-1:0] rs_data;
  logos_fast_port #(.NB(4), .AW(AW), .LW(LW), .DL(DL)) port(.clk(clk), .rst(rst), .space(32'd0), .base({AW{1'b0}}),
    .granule_log2(5'd3), .clear(1'b0), .fault(fault), .rq_valid(rq_valid), .rq_ready(rq_ready), .rq_line(rq_line),
    .rs_valid(rs_valid), .rs_data(rs_data), .wq_valid(1'b0), .wq_ready(wq_ready), .wq_line(32'd0), .wq_data({DL{1'b0}}),
    .writing(writing), .md_valid(pd_valid[3:0]), .md_ready(pd_ready[3:0]), .md_data(pd_data[0 +: 4*MDW]),
    .mr_valid(pr_valid[3:0]), .mr_ready(pr_ready[3:0]), .mr_data(mr_data), .mw_valid(pw_valid[3:0]),
    .mw_ready(pw_ready[3:0]), .mw_data(pw_data[0 +: 4*DL]), .mdone_valid(pdone_valid[3:0]),
    .mdone_ready(pdone_ready[3:0]));
  assign pw_valid[7:4] = 4'd0;
  assign pw_data[4*DL +: 4*DL] = {(4*DL){1'b0}};
  assign pdone_ready[7:4] = 4'hf;

  // The reader: each channel's engine, fed its tiles in order, its channel side straight on the multiplexer's port 1 or
  // through a credit.
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
    if (CREDIT) begin : credit
      logos_read_credit #(.AW(AW), .LW(LW), .DL(DL)) k(.clk(clk), .rst(rst), .in_valid(s_md_valid),
        .in_ready(s_md_ready), .in_data(s_md_data), .out_valid(s_mr_valid), .out_ready(s_mr_ready),
        .out_data(s_mr_data), .md_valid(pd_valid[4 + g]), .md_ready(pd_ready[4 + g]),
        .md_data(pd_data[(4 + g)*MDW +: MDW]), .mr_valid(pr_valid[4 + g]), .mr_ready(pr_ready[4 + g]),
        .mr_data(mr_data[DL*g +: DL]));
    end else begin : direct
      assign pd_valid[4 + g] = s_md_valid;
      assign s_md_ready = pd_ready[4 + g];
      assign pd_data[(4 + g)*MDW +: MDW] = s_md_data;
      assign s_mr_valid = pr_valid[4 + g];
      assign pr_ready[4 + g] = s_mr_ready;
      assign s_mr_data = mr_data[DL*g +: DL];
    end
  end endgenerate

  // The tiles: the same sizes on every channel, each channel's tiles one after another from its own first line.
  integer size [0:TILES-1];
  integer tile_at [0:3], tile_from [0:3], beat_tile [0:3], beat_in [0:3], beat_from [0:3];
  // Each channel's descriptors taken (first line, lines, the clock its first line may come), in order.
  reg [AW-1:0] q_line [0:63];
  integer q_n [0:63], q_ready [0:63], q_head [0:3], q_tail [0:3], q_given [0:3];
  // The fast port's reads asked and not yet back, in order.
  reg [31:0] asked [0:4095];
  integer ask_head, ask_tail, reads_asked;
  reg [31:0] walk;
  // The reader's lines asked of each channel and taken by its engine, under the credit.
  integer lines_asked [0:3], lines_taken [0:3];
  integer seed, now, moved, errors, beats, beats_all, reads_back, over, longest, first_beat, last_beat, c, k, total;
  reg presented [0:3];
  reg [31:0] x;
  reg [AW-1:0] local_line;

  initial begin
    seed = SEED; now = 0; moved = 0; errors = 0; beats = 0; reads_back = 0; over = 0; longest = 0;
    first_beat = -1; last_beat = -1; ask_head = 0; ask_tail = 0; reads_asked = 0; walk = 0; total = 0;
    for (k = 0; k < TILES; k = k + 1) begin
      size[k] = LEAST + $urandom(seed) % (MOST - LEAST + 1);
      total = total + size[k];
    end
    beats_all = 4 * total;
    for (c = 0; c < 4; c = c + 1) begin
      tile_at[c] = 0; tile_from[c] = 4096 + 65536 * c; beat_tile[c] = 0; beat_in[c] = 0; beat_from[c] = tile_from[c];
      q_head[c] = 0; q_tail[c] = 0; q_given[c] = 0; lines_asked[c] = 0; lines_taken[c] = 0; presented[c] = 0;
    end
    repeat (4) @(posedge clk);
    @(negedge clk) rst = 0;
    while ((beats < beats_all || reads_back < READS) && now - moved < STALL) begin
      @(negedge clk);
      now = now + 1;
      // What every side offers this clock.
      for (c = 0; c < 4; c = c + 1) begin
        tile_valid[c] = tile_at[c] < TILES;
        tile_line[c] = tile_from[c];
        tile_lines[c] = tile_at[c] < TILES ? size[tile_at[c]] : 1;
        md_ready[c] = q_tail[c] - q_head[c] < 16 && (RATE || $urandom(seed) % 100 >= GAP);
        if (!presented[c])
          presented[c] = q_head[c] != q_tail[c] && now >= q_ready[16*c + q_head[c] % 16]
            && (RATE || $urandom(seed) % 100 >= GAP);
        mr_valid[c] = presented[c];
        mr_data[DL*c +: DL] = pattern(c[1:0], q_line[16*c + q_head[c] % 16] + q_given[c]);
      end
      out_ready = {4{&out_valid && (RATE || $urandom(seed) % 100 >= GAP)}};
      rq_valid = reads_asked < READS && ask_tail - ask_head < 4000 && $urandom(seed) % 100 < 80;
      if ($urandom(seed) % 100 < 70) walk = walk + 1; else walk = $urandom(seed) % 16384;
      rq_line = walk;
      #100;
      // What the coming edge moves.
      for (c = 0; c < 4; c = c + 1) begin
        if (tile_valid[c] && tile_ready[c]) begin
          tile_from[c] = tile_from[c] + size[tile_at[c]]; tile_at[c] = tile_at[c] + 1; moved = now;
        end
        if (md_valid[c] && md_ready[c]) begin
          if (md_data[c*MDW + MDW - 1]) errors = errors + 1;
          q_line[16*c + q_tail[c] % 16] = md_data[c*MDW + LW +: AW];
          q_n[16*c + q_tail[c] % 16] = md_data[c*MDW +: LW];
          q_ready[16*c + q_tail[c] % 16] = now + (RATE ? LAT : $urandom(seed) % (LAT + 1));
          q_tail[c] = q_tail[c] + 1;
          moved = now;
        end
        if (pd_valid[4 + c] && pd_ready[4 + c]) begin
          k = pd_data[(4 + c)*MDW +: LW];
          if (CREDIT && (k < 1 || k > 32)) errors = errors + 1;
          if (k > longest) longest = k;
          lines_asked[c] = lines_asked[c] + k;
        end
        if (mr_valid[c] && mr_ready[c]) begin
          presented[c] = 0;
          q_given[c] = q_given[c] + 1;
          if (q_given[c] == q_n[16*c + q_head[c] % 16]) begin q_given[c] = 0; q_head[c] = q_head[c] + 1; end
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
          lines_taken[c] = lines_taken[c] + 1;
        end
        beats = beats + 4;
        if (first_beat < 0) first_beat = now;
        last_beat = now;
        moved = now;
      end
      for (c = 0; c < 4; c = c + 1) if (CREDIT && lines_asked[c] - lines_taken[c] > 512) over = over + 1;
      if (rq_valid && rq_ready) begin
        asked[ask_tail % 4096] = rq_line;
        ask_tail = ask_tail + 1;
        reads_asked = reads_asked + 1;
        moved = now;
      end
      if (rs_valid) begin
        x = asked[ask_head % 4096];
        local_line = (x & 32'd7) | ((x >> 2) & ~32'd7);
        if (rs_data !== pattern(x[4:3], local_line)) errors = errors + 1;
        ask_head = ask_head + 1;
        reads_back = reads_back + 1;
        moved = now;
      end
    end
    $display("RESULT beats=%0d of %0d reads=%0d of %0d errors=%0d hung=%0d over=%0d longest=%0d lines=%0d clocks=%0d",
      beats, beats_all, reads_back, READS, errors, now - moved >= STALL, over, longest, total,
      last_beat - first_beat + 1);
    $finish;
  end
endmodule
