// seqprog_lsum — the whole-program SEQUENCED fabric (LATENCY-INSENSITIVE): per-block datapaths with
// multi-cycle division/float TILES + a block-PC FSM that starts each block, waits its done, latches the
// register writes, and takes the terminator. == run_program_cfg_on_grid == the VM.
// seqblk_lsum_b0 — a per-block SEQUENCED datapath (LATENCY-INSENSITIVE): start pulse -> walk the cells
// (combinational in one step, division/float on the shared multi-cycle units, heap on the shared state plane) ->
// done + the block's outputs.
module seqblk_lsum_b0(input clk, input rst, input start, input [63:0] in0, output [63:0] o0, output [63:0] o1, output [63:0] o2, output [63:0] o3, output [63:0] o4, output [63:0] o5, output [63:0] o6, output [63:0] o7, output [63:0] o8, output [63:0] o9, output [63:0] o10, output [63:0] o11, output [63:0] o12, output [63:0] heap_raddr, input [63:0] heap_rdata, output heap_we, output [63:0] heap_waddr, output [63:0] heap_wdata, output alloc_req, output [63:0] alloc_size, input [63:0] hp_in, output done);
  localparam M = 13;
  (* ram_style="registers" *) reg [63:0] R [0:12];
  reg [31:0] step;
  reg running, issued;
  reg is_comb; reg [63:0] comb_y; reg [63:0] ua, ub; reg [3:0] usel;
  reg [63:0] heap_raddr_r, heap_waddr_r, heap_wdata_r, alloc_size_r; reg heap_we_r, alloc_req_r;
  function [63:0] i64_to_f64(input signed [63:0] n);
    reg s; reg [63:0] m; integer lead, i, sh; reg [10:0] be; reg [51:0] frac;
    reg [63:0] shifted, dropped; reg [53:0] sig; reg round_up;
    begin
      if (n == 64'sd0) i64_to_f64 = 64'd0;
      else begin
        s = n[63];
        m = s ? (~n + 64'sd1) : n;
        lead = 0;
        for (i = 63; i >= 0; i = i - 1) if (m[i] && lead == 0) lead = i;
        be = 11'd1023 + lead[10:0];
        if (lead <= 52) begin
          shifted = m << (52 - lead);
          i64_to_f64 = {s, be, shifted[51:0]};
        end else begin
          sh = lead - 52;
          sig = m >> sh;
          dropped = m << (64 - sh);
          round_up = dropped[63] && (|dropped[62:0] || sig[0]);
          if (round_up) sig = sig + 54'd1;
          if (sig[53]) begin be = be + 11'd1; frac = 52'd0; end
          else            frac = sig[51:0];
          i64_to_f64 = {s, be, frac};
        end
      end
    end
  endfunction
  function signed [63:0] f64_floor_i64(input [63:0] x);
    reg s; reg [10:0] e; reg [52:0] sig; integer ue; reg [63:0] mag; reg hasfrac;
    begin
      s = x[63]; e = x[62:52];
      if (e < 11'd1023)
        f64_floor_i64 = (s && !(e == 11'd0 && x[51:0] == 52'd0)) ? -64'sd1 : 64'sd0;
      else begin
        sig = {1'b1, x[51:0]};
        ue = e - 11'd1023;
        if (ue >= 52) begin mag = {11'd0, sig} << (ue - 52); hasfrac = 1'b0; end
        else begin
          mag = {11'd0, sig} >> (52 - ue);
          hasfrac = (({11'd0, sig} << (ue + 12)) != 64'd0);
        end
        if (!s) f64_floor_i64 = mag;
        else    f64_floor_i64 = hasfrac ? -(mag + 64'd1) : -mag;
      end
    end
  endfunction
  always @(*) begin
    is_comb=1'b1; comb_y=64'd0; ua=64'd0; ub=64'd0; usel=4'd0;
    heap_raddr_r=64'd0; heap_we_r=1'b0; heap_waddr_r=64'd0; heap_wdata_r=64'd0; alloc_req_r=1'b0; alloc_size_r=64'd0;
    case (step)
      0: begin is_comb=1'b1; comb_y=$signed(in0) * $signed(64'sd2); end
      1: begin is_comb=1'b1; comb_y=$signed(in0) * $signed(64'sd3); end
      2: begin is_comb=1'b1; comb_y=$signed(in0) + $signed(64'sd7); end
      3: begin is_comb=1'b1; comb_y=hp_in; alloc_req_r=1'b1; alloc_size_r=64'sd5; end
      4: begin is_comb=1'b1; comb_y=64'sd4; heap_we_r=1'b1; heap_waddr_r=R[3]; heap_wdata_r=64'sd4; end
      5: begin is_comb=1'b1; comb_y=$signed(R[3]) + $signed(64'sd1); end
      6: begin is_comb=1'b1; comb_y=in0; heap_we_r=1'b1; heap_waddr_r=R[5]; heap_wdata_r=in0; end
      7: begin is_comb=1'b1; comb_y=$signed(R[3]) + $signed(64'sd2); end
      8: begin is_comb=1'b1; comb_y=R[0]; heap_we_r=1'b1; heap_waddr_r=R[7]; heap_wdata_r=R[0]; end
      9: begin is_comb=1'b1; comb_y=$signed(R[3]) + $signed(64'sd3); end
      10: begin is_comb=1'b1; comb_y=R[1]; heap_we_r=1'b1; heap_waddr_r=R[9]; heap_wdata_r=R[1]; end
      11: begin is_comb=1'b1; comb_y=$signed(R[3]) + $signed(64'sd4); end
      12: begin is_comb=1'b1; comb_y=R[2]; heap_we_r=1'b1; heap_waddr_r=R[11]; heap_wdata_r=R[2]; end
      default: ;
    endcase
  end
  wire is_div   = (usel>=4'd1 && usel<=4'd3);
  wire is_mod   = (usel==4'd2);
  wire is_floor = (usel==4'd3);
  wire is_pow   = (usel==4'd4);
  wire is_fadd  = (usel==4'd5)||(usel==4'd6);
  wire is_fsub = (usel==4'd6);
  wire is_fmul  = (usel==4'd7);
  wire is_fdiv = (usel==4'd8);
  wire is_fsqrt = (usel==4'd9);
  wire is_cmp   = (usel>=4'd10);
  wire pulse = running && !issued && !is_comb;
  wire [63:0] div_y = 64'd0; wire div_done_g = 1'b1;
  wire [63:0] fa_y = 64'd0; wire fa_done = 1'b1;
  wire [63:0] fm_y = 64'd0; wire fm_done = 1'b1;
  wire [63:0] fd_y = 64'd0; wire fd_done = 1'b1;
  wire [63:0] fs_y = 64'd0; wire fs_done = 1'b1;
  wire [63:0] cmp_y = 64'd0;
  wire [63:0] pw_acc = 64'd0; wire pw_done = 1'b1;
  wire        mc_done = is_div ? div_done_g : is_pow ? pw_done : is_cmp ? 1'b1 : is_fadd ? fa_done : is_fmul ? fm_done : is_fdiv ? fd_done : is_fsqrt ? fs_done : 1'b1;
  wire [63:0] mc_y    = is_div ? div_y : is_pow ? pw_acc : is_cmp ? cmp_y : is_fadd ? fa_y : is_fmul ? fm_y : is_fdiv ? fd_y : is_fsqrt ? fs_y : 64'd0;
  always @(posedge clk) begin
    if (rst) begin running<=1'b0; step<=32'd0; issued<=1'b0; end
    else if (start) begin step<=32'd0; running<=(M>0); issued<=1'b0; end
    else if (running) begin
      if (is_comb) begin
        R[step] <= comb_y;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end else if (!issued) begin
        issued <= 1'b1;
      end else if (mc_done) begin
        R[step] <= mc_y; issued<=1'b0;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end
    end
  end
  assign done = !running;
  assign heap_raddr=heap_raddr_r; assign heap_we=running && heap_we_r; assign heap_waddr=heap_waddr_r;
  assign heap_wdata=heap_wdata_r; assign alloc_req=running && alloc_req_r; assign alloc_size=alloc_size_r;
  assign o0 = in0;
  assign o1 = 64'sd2;
  assign o2 = R[0];
  assign o3 = 64'sd3;
  assign o4 = R[1];
  assign o5 = 64'sd7;
  assign o6 = R[2];
  assign o7 = R[3];
  assign o8 = R[3];
  assign o9 = 64'sd0;
  assign o10 = 64'sd0;
  assign o11 = 64'sd1;
  assign o12 = 64'sd1;
endmodule

// seqblk_lsum_b1 — a per-block SEQUENCED datapath (LATENCY-INSENSITIVE): start pulse -> walk the cells
// (combinational in one step, division/float on the shared multi-cycle units, heap on the shared state plane) ->
// done + the block's outputs.
module seqblk_lsum_b1(input clk, input rst, input start, input [63:0] in0, input [63:0] in1, output [63:0] o0, output [63:0] o1, output [63:0] o2, output [63:0] heap_raddr, input [63:0] heap_rdata, output heap_we, output [63:0] heap_waddr, output [63:0] heap_wdata, output alloc_req, output [63:0] alloc_size, input [63:0] hp_in, output done);
  localparam M = 2;
  (* ram_style="registers" *) reg [63:0] R [0:1];
  reg [31:0] step;
  reg running, issued;
  reg is_comb; reg [63:0] comb_y; reg [63:0] ua, ub; reg [3:0] usel;
  reg [63:0] heap_raddr_r, heap_waddr_r, heap_wdata_r, alloc_size_r; reg heap_we_r, alloc_req_r;
  function [63:0] i64_to_f64(input signed [63:0] n);
    reg s; reg [63:0] m; integer lead, i, sh; reg [10:0] be; reg [51:0] frac;
    reg [63:0] shifted, dropped; reg [53:0] sig; reg round_up;
    begin
      if (n == 64'sd0) i64_to_f64 = 64'd0;
      else begin
        s = n[63];
        m = s ? (~n + 64'sd1) : n;
        lead = 0;
        for (i = 63; i >= 0; i = i - 1) if (m[i] && lead == 0) lead = i;
        be = 11'd1023 + lead[10:0];
        if (lead <= 52) begin
          shifted = m << (52 - lead);
          i64_to_f64 = {s, be, shifted[51:0]};
        end else begin
          sh = lead - 52;
          sig = m >> sh;
          dropped = m << (64 - sh);
          round_up = dropped[63] && (|dropped[62:0] || sig[0]);
          if (round_up) sig = sig + 54'd1;
          if (sig[53]) begin be = be + 11'd1; frac = 52'd0; end
          else            frac = sig[51:0];
          i64_to_f64 = {s, be, frac};
        end
      end
    end
  endfunction
  function signed [63:0] f64_floor_i64(input [63:0] x);
    reg s; reg [10:0] e; reg [52:0] sig; integer ue; reg [63:0] mag; reg hasfrac;
    begin
      s = x[63]; e = x[62:52];
      if (e < 11'd1023)
        f64_floor_i64 = (s && !(e == 11'd0 && x[51:0] == 52'd0)) ? -64'sd1 : 64'sd0;
      else begin
        sig = {1'b1, x[51:0]};
        ue = e - 11'd1023;
        if (ue >= 52) begin mag = {11'd0, sig} << (ue - 52); hasfrac = 1'b0; end
        else begin
          mag = {11'd0, sig} >> (52 - ue);
          hasfrac = (({11'd0, sig} << (ue + 12)) != 64'd0);
        end
        if (!s) f64_floor_i64 = mag;
        else    f64_floor_i64 = hasfrac ? -(mag + 64'd1) : -mag;
      end
    end
  endfunction
  always @(*) begin
    is_comb=1'b1; comb_y=64'd0; ua=64'd0; ub=64'd0; usel=4'd0;
    heap_raddr_r=64'd0; heap_we_r=1'b0; heap_waddr_r=64'd0; heap_wdata_r=64'd0; alloc_req_r=1'b0; alloc_size_r=64'd0;
    case (step)
      0: begin is_comb=1'b1; comb_y=heap_rdata; heap_raddr_r=in0; end
      1: begin is_comb=1'b1; comb_y=($signed(in1) <= $signed(R[0])) ? 64'sd1 : 64'sd0; end
      default: ;
    endcase
  end
  wire is_div   = (usel>=4'd1 && usel<=4'd3);
  wire is_mod   = (usel==4'd2);
  wire is_floor = (usel==4'd3);
  wire is_pow   = (usel==4'd4);
  wire is_fadd  = (usel==4'd5)||(usel==4'd6);
  wire is_fsub = (usel==4'd6);
  wire is_fmul  = (usel==4'd7);
  wire is_fdiv = (usel==4'd8);
  wire is_fsqrt = (usel==4'd9);
  wire is_cmp   = (usel>=4'd10);
  wire pulse = running && !issued && !is_comb;
  wire [63:0] div_y = 64'd0; wire div_done_g = 1'b1;
  wire [63:0] fa_y = 64'd0; wire fa_done = 1'b1;
  wire [63:0] fm_y = 64'd0; wire fm_done = 1'b1;
  wire [63:0] fd_y = 64'd0; wire fd_done = 1'b1;
  wire [63:0] fs_y = 64'd0; wire fs_done = 1'b1;
  wire [63:0] cmp_y = 64'd0;
  wire [63:0] pw_acc = 64'd0; wire pw_done = 1'b1;
  wire        mc_done = is_div ? div_done_g : is_pow ? pw_done : is_cmp ? 1'b1 : is_fadd ? fa_done : is_fmul ? fm_done : is_fdiv ? fd_done : is_fsqrt ? fs_done : 1'b1;
  wire [63:0] mc_y    = is_div ? div_y : is_pow ? pw_acc : is_cmp ? cmp_y : is_fadd ? fa_y : is_fmul ? fm_y : is_fdiv ? fd_y : is_fsqrt ? fs_y : 64'd0;
  always @(posedge clk) begin
    if (rst) begin running<=1'b0; step<=32'd0; issued<=1'b0; end
    else if (start) begin step<=32'd0; running<=(M>0); issued<=1'b0; end
    else if (running) begin
      if (is_comb) begin
        R[step] <= comb_y;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end else if (!issued) begin
        issued <= 1'b1;
      end else if (mc_done) begin
        R[step] <= mc_y; issued<=1'b0;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end
    end
  end
  assign done = !running;
  assign heap_raddr=heap_raddr_r; assign heap_we=running && heap_we_r; assign heap_waddr=heap_waddr_r;
  assign heap_wdata=heap_wdata_r; assign alloc_req=running && alloc_req_r; assign alloc_size=alloc_size_r;
  assign o0 = R[0];
  assign o1 = R[1];
  assign o2 = R[1];
endmodule

// seqblk_lsum_b2 — a per-block SEQUENCED datapath (LATENCY-INSENSITIVE): start pulse -> walk the cells
// (combinational in one step, division/float on the shared multi-cycle units, heap on the shared state plane) ->
// done + the block's outputs.
module seqblk_lsum_b2(input clk, input rst, input start, input [63:0] in0, input [63:0] in1, input [63:0] in2, output [63:0] o0, output [63:0] o1, output [63:0] o2, output [63:0] o3, output [63:0] heap_raddr, input [63:0] heap_rdata, output heap_we, output [63:0] heap_waddr, output [63:0] heap_wdata, output alloc_req, output [63:0] alloc_size, input [63:0] hp_in, output done);
  localparam M = 4;
  (* ram_style="registers" *) reg [63:0] R [0:3];
  reg [31:0] step;
  reg running, issued;
  reg is_comb; reg [63:0] comb_y; reg [63:0] ua, ub; reg [3:0] usel;
  reg [63:0] heap_raddr_r, heap_waddr_r, heap_wdata_r, alloc_size_r; reg heap_we_r, alloc_req_r;
  function [63:0] i64_to_f64(input signed [63:0] n);
    reg s; reg [63:0] m; integer lead, i, sh; reg [10:0] be; reg [51:0] frac;
    reg [63:0] shifted, dropped; reg [53:0] sig; reg round_up;
    begin
      if (n == 64'sd0) i64_to_f64 = 64'd0;
      else begin
        s = n[63];
        m = s ? (~n + 64'sd1) : n;
        lead = 0;
        for (i = 63; i >= 0; i = i - 1) if (m[i] && lead == 0) lead = i;
        be = 11'd1023 + lead[10:0];
        if (lead <= 52) begin
          shifted = m << (52 - lead);
          i64_to_f64 = {s, be, shifted[51:0]};
        end else begin
          sh = lead - 52;
          sig = m >> sh;
          dropped = m << (64 - sh);
          round_up = dropped[63] && (|dropped[62:0] || sig[0]);
          if (round_up) sig = sig + 54'd1;
          if (sig[53]) begin be = be + 11'd1; frac = 52'd0; end
          else            frac = sig[51:0];
          i64_to_f64 = {s, be, frac};
        end
      end
    end
  endfunction
  function signed [63:0] f64_floor_i64(input [63:0] x);
    reg s; reg [10:0] e; reg [52:0] sig; integer ue; reg [63:0] mag; reg hasfrac;
    begin
      s = x[63]; e = x[62:52];
      if (e < 11'd1023)
        f64_floor_i64 = (s && !(e == 11'd0 && x[51:0] == 52'd0)) ? -64'sd1 : 64'sd0;
      else begin
        sig = {1'b1, x[51:0]};
        ue = e - 11'd1023;
        if (ue >= 52) begin mag = {11'd0, sig} << (ue - 52); hasfrac = 1'b0; end
        else begin
          mag = {11'd0, sig} >> (52 - ue);
          hasfrac = (({11'd0, sig} << (ue + 12)) != 64'd0);
        end
        if (!s) f64_floor_i64 = mag;
        else    f64_floor_i64 = hasfrac ? -(mag + 64'd1) : -mag;
      end
    end
  endfunction
  always @(*) begin
    is_comb=1'b1; comb_y=64'd0; ua=64'd0; ub=64'd0; usel=4'd0;
    heap_raddr_r=64'd0; heap_we_r=1'b0; heap_waddr_r=64'd0; heap_wdata_r=64'd0; alloc_req_r=1'b0; alloc_size_r=64'd0;
    case (step)
      0: begin is_comb=1'b1; comb_y=$signed(in0) + $signed(in1); end
      1: begin is_comb=1'b1; comb_y=heap_rdata; heap_raddr_r=R[0]; end
      2: begin is_comb=1'b1; comb_y=$signed(in2) + $signed(R[1]); end
      3: begin is_comb=1'b1; comb_y=$signed(in1) + $signed(64'sd1); end
      default: ;
    endcase
  end
  wire is_div   = (usel>=4'd1 && usel<=4'd3);
  wire is_mod   = (usel==4'd2);
  wire is_floor = (usel==4'd3);
  wire is_pow   = (usel==4'd4);
  wire is_fadd  = (usel==4'd5)||(usel==4'd6);
  wire is_fsub = (usel==4'd6);
  wire is_fmul  = (usel==4'd7);
  wire is_fdiv = (usel==4'd8);
  wire is_fsqrt = (usel==4'd9);
  wire is_cmp   = (usel>=4'd10);
  wire pulse = running && !issued && !is_comb;
  wire [63:0] div_y = 64'd0; wire div_done_g = 1'b1;
  wire [63:0] fa_y = 64'd0; wire fa_done = 1'b1;
  wire [63:0] fm_y = 64'd0; wire fm_done = 1'b1;
  wire [63:0] fd_y = 64'd0; wire fd_done = 1'b1;
  wire [63:0] fs_y = 64'd0; wire fs_done = 1'b1;
  wire [63:0] cmp_y = 64'd0;
  wire [63:0] pw_acc = 64'd0; wire pw_done = 1'b1;
  wire        mc_done = is_div ? div_done_g : is_pow ? pw_done : is_cmp ? 1'b1 : is_fadd ? fa_done : is_fmul ? fm_done : is_fdiv ? fd_done : is_fsqrt ? fs_done : 1'b1;
  wire [63:0] mc_y    = is_div ? div_y : is_pow ? pw_acc : is_cmp ? cmp_y : is_fadd ? fa_y : is_fmul ? fm_y : is_fdiv ? fd_y : is_fsqrt ? fs_y : 64'd0;
  always @(posedge clk) begin
    if (rst) begin running<=1'b0; step<=32'd0; issued<=1'b0; end
    else if (start) begin step<=32'd0; running<=(M>0); issued<=1'b0; end
    else if (running) begin
      if (is_comb) begin
        R[step] <= comb_y;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end else if (!issued) begin
        issued <= 1'b1;
      end else if (mc_done) begin
        R[step] <= mc_y; issued<=1'b0;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end
    end
  end
  assign done = !running;
  assign heap_raddr=heap_raddr_r; assign heap_we=running && heap_we_r; assign heap_waddr=heap_waddr_r;
  assign heap_wdata=heap_wdata_r; assign alloc_req=running && alloc_req_r; assign alloc_size=alloc_size_r;
  assign o0 = R[1];
  assign o1 = R[2];
  assign o2 = 64'sd1;
  assign o3 = R[3];
endmodule

// seqblk_lsum_b3 — a per-block SEQUENCED datapath (LATENCY-INSENSITIVE): start pulse -> walk the cells
// (combinational in one step, division/float on the shared multi-cycle units, heap on the shared state plane) ->
// done + the block's outputs.
module seqblk_lsum_b3(input clk, input rst, input start, input [63:0] in0, output [63:0] o0, output [63:0] heap_raddr, input [63:0] heap_rdata, output heap_we, output [63:0] heap_waddr, output [63:0] heap_wdata, output alloc_req, output [63:0] alloc_size, input [63:0] hp_in, output done);
  localparam M = 0;
  (* ram_style="registers" *) reg [63:0] R [0:0];
  reg [31:0] step;
  reg running, issued;
  reg is_comb; reg [63:0] comb_y; reg [63:0] ua, ub; reg [3:0] usel;
  reg [63:0] heap_raddr_r, heap_waddr_r, heap_wdata_r, alloc_size_r; reg heap_we_r, alloc_req_r;
  function [63:0] i64_to_f64(input signed [63:0] n);
    reg s; reg [63:0] m; integer lead, i, sh; reg [10:0] be; reg [51:0] frac;
    reg [63:0] shifted, dropped; reg [53:0] sig; reg round_up;
    begin
      if (n == 64'sd0) i64_to_f64 = 64'd0;
      else begin
        s = n[63];
        m = s ? (~n + 64'sd1) : n;
        lead = 0;
        for (i = 63; i >= 0; i = i - 1) if (m[i] && lead == 0) lead = i;
        be = 11'd1023 + lead[10:0];
        if (lead <= 52) begin
          shifted = m << (52 - lead);
          i64_to_f64 = {s, be, shifted[51:0]};
        end else begin
          sh = lead - 52;
          sig = m >> sh;
          dropped = m << (64 - sh);
          round_up = dropped[63] && (|dropped[62:0] || sig[0]);
          if (round_up) sig = sig + 54'd1;
          if (sig[53]) begin be = be + 11'd1; frac = 52'd0; end
          else            frac = sig[51:0];
          i64_to_f64 = {s, be, frac};
        end
      end
    end
  endfunction
  function signed [63:0] f64_floor_i64(input [63:0] x);
    reg s; reg [10:0] e; reg [52:0] sig; integer ue; reg [63:0] mag; reg hasfrac;
    begin
      s = x[63]; e = x[62:52];
      if (e < 11'd1023)
        f64_floor_i64 = (s && !(e == 11'd0 && x[51:0] == 52'd0)) ? -64'sd1 : 64'sd0;
      else begin
        sig = {1'b1, x[51:0]};
        ue = e - 11'd1023;
        if (ue >= 52) begin mag = {11'd0, sig} << (ue - 52); hasfrac = 1'b0; end
        else begin
          mag = {11'd0, sig} >> (52 - ue);
          hasfrac = (({11'd0, sig} << (ue + 12)) != 64'd0);
        end
        if (!s) f64_floor_i64 = mag;
        else    f64_floor_i64 = hasfrac ? -(mag + 64'd1) : -mag;
      end
    end
  endfunction
  always @(*) begin
    is_comb=1'b1; comb_y=64'd0; ua=64'd0; ub=64'd0; usel=4'd0;
    heap_raddr_r=64'd0; heap_we_r=1'b0; heap_waddr_r=64'd0; heap_wdata_r=64'd0; alloc_req_r=1'b0; alloc_size_r=64'd0;
    case (step)
      default: ;
    endcase
  end
  wire is_div   = (usel>=4'd1 && usel<=4'd3);
  wire is_mod   = (usel==4'd2);
  wire is_floor = (usel==4'd3);
  wire is_pow   = (usel==4'd4);
  wire is_fadd  = (usel==4'd5)||(usel==4'd6);
  wire is_fsub = (usel==4'd6);
  wire is_fmul  = (usel==4'd7);
  wire is_fdiv = (usel==4'd8);
  wire is_fsqrt = (usel==4'd9);
  wire is_cmp   = (usel>=4'd10);
  wire pulse = running && !issued && !is_comb;
  wire [63:0] div_y = 64'd0; wire div_done_g = 1'b1;
  wire [63:0] fa_y = 64'd0; wire fa_done = 1'b1;
  wire [63:0] fm_y = 64'd0; wire fm_done = 1'b1;
  wire [63:0] fd_y = 64'd0; wire fd_done = 1'b1;
  wire [63:0] fs_y = 64'd0; wire fs_done = 1'b1;
  wire [63:0] cmp_y = 64'd0;
  wire [63:0] pw_acc = 64'd0; wire pw_done = 1'b1;
  wire        mc_done = is_div ? div_done_g : is_pow ? pw_done : is_cmp ? 1'b1 : is_fadd ? fa_done : is_fmul ? fm_done : is_fdiv ? fd_done : is_fsqrt ? fs_done : 1'b1;
  wire [63:0] mc_y    = is_div ? div_y : is_pow ? pw_acc : is_cmp ? cmp_y : is_fadd ? fa_y : is_fmul ? fm_y : is_fdiv ? fd_y : is_fsqrt ? fs_y : 64'd0;
  always @(posedge clk) begin
    if (rst) begin running<=1'b0; step<=32'd0; issued<=1'b0; end
    else if (start) begin step<=32'd0; running<=(M>0); issued<=1'b0; end
    else if (running) begin
      if (is_comb) begin
        R[step] <= comb_y;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end else if (!issued) begin
        issued <= 1'b1;
      end else if (mc_done) begin
        R[step] <= mc_y; issued<=1'b0;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end
    end
  end
  assign done = !running;
  assign heap_raddr=heap_raddr_r; assign heap_we=running && heap_we_r; assign heap_waddr=heap_waddr_r;
  assign heap_wdata=heap_wdata_r; assign alloc_req=running && alloc_req_r; assign alloc_size=alloc_size_r;
  assign o0 = in0;
endmodule

// seqblk_lsum_b4 — a per-block SEQUENCED datapath (LATENCY-INSENSITIVE): start pulse -> walk the cells
// (combinational in one step, division/float on the shared multi-cycle units, heap on the shared state plane) ->
// done + the block's outputs.
module seqblk_lsum_b4(input clk, input rst, input start, output [63:0] o0, output [63:0] heap_raddr, input [63:0] heap_rdata, output heap_we, output [63:0] heap_waddr, output [63:0] heap_wdata, output alloc_req, output [63:0] alloc_size, input [63:0] hp_in, output done);
  localparam M = 0;
  (* ram_style="registers" *) reg [63:0] R [0:0];
  reg [31:0] step;
  reg running, issued;
  reg is_comb; reg [63:0] comb_y; reg [63:0] ua, ub; reg [3:0] usel;
  reg [63:0] heap_raddr_r, heap_waddr_r, heap_wdata_r, alloc_size_r; reg heap_we_r, alloc_req_r;
  function [63:0] i64_to_f64(input signed [63:0] n);
    reg s; reg [63:0] m; integer lead, i, sh; reg [10:0] be; reg [51:0] frac;
    reg [63:0] shifted, dropped; reg [53:0] sig; reg round_up;
    begin
      if (n == 64'sd0) i64_to_f64 = 64'd0;
      else begin
        s = n[63];
        m = s ? (~n + 64'sd1) : n;
        lead = 0;
        for (i = 63; i >= 0; i = i - 1) if (m[i] && lead == 0) lead = i;
        be = 11'd1023 + lead[10:0];
        if (lead <= 52) begin
          shifted = m << (52 - lead);
          i64_to_f64 = {s, be, shifted[51:0]};
        end else begin
          sh = lead - 52;
          sig = m >> sh;
          dropped = m << (64 - sh);
          round_up = dropped[63] && (|dropped[62:0] || sig[0]);
          if (round_up) sig = sig + 54'd1;
          if (sig[53]) begin be = be + 11'd1; frac = 52'd0; end
          else            frac = sig[51:0];
          i64_to_f64 = {s, be, frac};
        end
      end
    end
  endfunction
  function signed [63:0] f64_floor_i64(input [63:0] x);
    reg s; reg [10:0] e; reg [52:0] sig; integer ue; reg [63:0] mag; reg hasfrac;
    begin
      s = x[63]; e = x[62:52];
      if (e < 11'd1023)
        f64_floor_i64 = (s && !(e == 11'd0 && x[51:0] == 52'd0)) ? -64'sd1 : 64'sd0;
      else begin
        sig = {1'b1, x[51:0]};
        ue = e - 11'd1023;
        if (ue >= 52) begin mag = {11'd0, sig} << (ue - 52); hasfrac = 1'b0; end
        else begin
          mag = {11'd0, sig} >> (52 - ue);
          hasfrac = (({11'd0, sig} << (ue + 12)) != 64'd0);
        end
        if (!s) f64_floor_i64 = mag;
        else    f64_floor_i64 = hasfrac ? -(mag + 64'd1) : -mag;
      end
    end
  endfunction
  always @(*) begin
    is_comb=1'b1; comb_y=64'd0; ua=64'd0; ub=64'd0; usel=4'd0;
    heap_raddr_r=64'd0; heap_we_r=1'b0; heap_waddr_r=64'd0; heap_wdata_r=64'd0; alloc_req_r=1'b0; alloc_size_r=64'd0;
    case (step)
      default: ;
    endcase
  end
  wire is_div   = (usel>=4'd1 && usel<=4'd3);
  wire is_mod   = (usel==4'd2);
  wire is_floor = (usel==4'd3);
  wire is_pow   = (usel==4'd4);
  wire is_fadd  = (usel==4'd5)||(usel==4'd6);
  wire is_fsub = (usel==4'd6);
  wire is_fmul  = (usel==4'd7);
  wire is_fdiv = (usel==4'd8);
  wire is_fsqrt = (usel==4'd9);
  wire is_cmp   = (usel>=4'd10);
  wire pulse = running && !issued && !is_comb;
  wire [63:0] div_y = 64'd0; wire div_done_g = 1'b1;
  wire [63:0] fa_y = 64'd0; wire fa_done = 1'b1;
  wire [63:0] fm_y = 64'd0; wire fm_done = 1'b1;
  wire [63:0] fd_y = 64'd0; wire fd_done = 1'b1;
  wire [63:0] fs_y = 64'd0; wire fs_done = 1'b1;
  wire [63:0] cmp_y = 64'd0;
  wire [63:0] pw_acc = 64'd0; wire pw_done = 1'b1;
  wire        mc_done = is_div ? div_done_g : is_pow ? pw_done : is_cmp ? 1'b1 : is_fadd ? fa_done : is_fmul ? fm_done : is_fdiv ? fd_done : is_fsqrt ? fs_done : 1'b1;
  wire [63:0] mc_y    = is_div ? div_y : is_pow ? pw_acc : is_cmp ? cmp_y : is_fadd ? fa_y : is_fmul ? fm_y : is_fdiv ? fd_y : is_fsqrt ? fs_y : 64'd0;
  always @(posedge clk) begin
    if (rst) begin running<=1'b0; step<=32'd0; issued<=1'b0; end
    else if (start) begin step<=32'd0; running<=(M>0); issued<=1'b0; end
    else if (running) begin
      if (is_comb) begin
        R[step] <= comb_y;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end else if (!issued) begin
        issued <= 1'b1;
      end else if (mc_done) begin
        R[step] <= mc_y; issued<=1'b0;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end
    end
  end
  assign done = !running;
  assign heap_raddr=heap_raddr_r; assign heap_we=running && heap_we_r; assign heap_waddr=heap_waddr_r;
  assign heap_wdata=heap_wdata_r; assign alloc_req=running && alloc_req_r; assign alloc_size=alloc_size_r;
  assign o0 = 64'sd0;
endmodule

module seqprog_lsum(
    input clk,
    input rst,
    input [63:0] p0,
    output [63:0] result,
    output done
);
  reg [63:0] file [0:17];
  reg [2:0] pc;
  reg done_r; reg [63:0] result_r; reg started;
  assign done = done_r; assign result = result_r;
  wire [63:0] fr_0 = file[0];
  wire [63:0] fr_1 = file[1];
  wire [63:0] fr_2 = file[2];
  wire [63:0] fr_3 = file[3];
  wire [63:0] fr_4 = file[4];
  wire [63:0] fr_5 = file[5];
  wire [63:0] fr_6 = file[6];
  wire [63:0] fr_7 = file[7];
  wire [63:0] fr_8 = file[8];
  wire [63:0] fr_9 = file[9];
  wire [63:0] fr_10 = file[10];
  wire [63:0] fr_11 = file[11];
  wire [63:0] fr_12 = file[12];
  wire [63:0] fr_13 = file[13];
  wire [63:0] fr_14 = file[14];
  wire [63:0] fr_15 = file[15];
  wire [63:0] fr_16 = file[16];
  wire [63:0] fr_17 = file[17];
  (* ram_style="registers" *) reg [63:0] heap [0:63];
  reg [63:0] hp;
  wire [63:0] b0_heap_raddr, b0_heap_waddr, b0_heap_wdata, b0_alloc_size; wire b0_heap_we, b0_alloc_req;
  wire [63:0] b1_heap_raddr, b1_heap_waddr, b1_heap_wdata, b1_alloc_size; wire b1_heap_we, b1_alloc_req;
  wire [63:0] b2_heap_raddr, b2_heap_waddr, b2_heap_wdata, b2_alloc_size; wire b2_heap_we, b2_alloc_req;
  wire [63:0] b3_heap_raddr, b3_heap_waddr, b3_heap_wdata, b3_alloc_size; wire b3_heap_we, b3_alloc_req;
  wire [63:0] b4_heap_raddr, b4_heap_waddr, b4_heap_wdata, b4_alloc_size; wire b4_heap_we, b4_alloc_req;
  reg [63:0] act_raddr, act_waddr, act_wdata, act_size; reg act_we, act_alloc;
  always @(*) begin
    act_raddr=64'd0; act_waddr=64'd0; act_wdata=64'd0; act_size=64'd0; act_we=1'b0; act_alloc=1'b0;
    case (pc)
      3'd0: begin act_raddr=b0_heap_raddr; act_we=b0_heap_we; act_waddr=b0_heap_waddr; act_wdata=b0_heap_wdata; act_alloc=b0_alloc_req; act_size=b0_alloc_size; end
      3'd1: begin act_raddr=b1_heap_raddr; act_we=b1_heap_we; act_waddr=b1_heap_waddr; act_wdata=b1_heap_wdata; act_alloc=b1_alloc_req; act_size=b1_alloc_size; end
      3'd2: begin act_raddr=b2_heap_raddr; act_we=b2_heap_we; act_waddr=b2_heap_waddr; act_wdata=b2_heap_wdata; act_alloc=b2_alloc_req; act_size=b2_alloc_size; end
      3'd3: begin act_raddr=b3_heap_raddr; act_we=b3_heap_we; act_waddr=b3_heap_waddr; act_wdata=b3_heap_wdata; act_alloc=b3_alloc_req; act_size=b3_alloc_size; end
      3'd4: begin act_raddr=b4_heap_raddr; act_we=b4_heap_we; act_waddr=b4_heap_waddr; act_wdata=b4_heap_wdata; act_alloc=b4_alloc_req; act_size=b4_alloc_size; end
      default: ;
    endcase
  end
  wire [63:0] heap_rdata_bus = heap[act_raddr[5:0]];
  integer hj;
  always @(posedge clk) begin
    if (rst) begin hp<=64'd0; for (hj=0; hj<64; hj=hj+1) heap[hj]<=64'd0; end
    else begin
      if (act_we) heap[act_waddr[5:0]] <= act_wdata;
      if (act_alloc) hp <= hp + act_size;
      else if (act_we && ($signed(act_waddr) + 64'sd1 > $signed(hp))) hp <= act_waddr + 64'sd1;
    end
  end
  wire [63:0] b0_o0;
  wire [63:0] b0_o1;
  wire [63:0] b0_o2;
  wire [63:0] b0_o3;
  wire [63:0] b0_o4;
  wire [63:0] b0_o5;
  wire [63:0] b0_o6;
  wire [63:0] b0_o7;
  wire [63:0] b0_o8;
  wire [63:0] b0_o9;
  wire [63:0] b0_o10;
  wire [63:0] b0_o11;
  wire [63:0] b0_o12;
  wire b0_done;
  seqblk_lsum_b0 u_b0(.clk(clk), .rst(rst), .start((pc==3'd0) && !started && !done_r), .in0(fr_0), .o0(b0_o0), .o1(b0_o1), .o2(b0_o2), .o3(b0_o3), .o4(b0_o4), .o5(b0_o5), .o6(b0_o6), .o7(b0_o7), .o8(b0_o8), .o9(b0_o9), .o10(b0_o10), .o11(b0_o11), .o12(b0_o12), .heap_raddr(b0_heap_raddr), .heap_rdata(heap_rdata_bus), .heap_we(b0_heap_we), .heap_waddr(b0_heap_waddr), .heap_wdata(b0_heap_wdata), .alloc_req(b0_alloc_req), .alloc_size(b0_alloc_size), .hp_in(hp), .done(b0_done));
  wire [63:0] b1_o0;
  wire [63:0] b1_o1;
  wire [63:0] b1_o2;
  wire b1_done;
  seqblk_lsum_b1 u_b1(.clk(clk), .rst(rst), .start((pc==3'd1) && !started && !done_r), .in0(fr_9), .in1(fr_13), .o0(b1_o0), .o1(b1_o1), .o2(b1_o2), .heap_raddr(b1_heap_raddr), .heap_rdata(heap_rdata_bus), .heap_we(b1_heap_we), .heap_waddr(b1_heap_waddr), .heap_wdata(b1_heap_wdata), .alloc_req(b1_alloc_req), .alloc_size(b1_alloc_size), .hp_in(hp), .done(b1_done));
  wire [63:0] b2_o0;
  wire [63:0] b2_o1;
  wire [63:0] b2_o2;
  wire [63:0] b2_o3;
  wire b2_done;
  seqblk_lsum_b2 u_b2(.clk(clk), .rst(rst), .start((pc==3'd2) && !started && !done_r), .in0(fr_9), .in1(fr_13), .in2(fr_11), .o0(b2_o0), .o1(b2_o1), .o2(b2_o2), .o3(b2_o3), .heap_raddr(b2_heap_raddr), .heap_rdata(heap_rdata_bus), .heap_we(b2_heap_we), .heap_waddr(b2_heap_waddr), .heap_wdata(b2_heap_wdata), .alloc_req(b2_alloc_req), .alloc_size(b2_alloc_size), .hp_in(hp), .done(b2_done));
  wire [63:0] b3_o0;
  wire b3_done;
  seqblk_lsum_b3 u_b3(.clk(clk), .rst(rst), .start((pc==3'd3) && !started && !done_r), .in0(fr_11), .o0(b3_o0), .heap_raddr(b3_heap_raddr), .heap_rdata(heap_rdata_bus), .heap_we(b3_heap_we), .heap_waddr(b3_heap_waddr), .heap_wdata(b3_heap_wdata), .alloc_req(b3_alloc_req), .alloc_size(b3_alloc_size), .hp_in(hp), .done(b3_done));
  wire [63:0] b4_o0;
  wire b4_done;
  seqblk_lsum_b4 u_b4(.clk(clk), .rst(rst), .start((pc==3'd4) && !started && !done_r), .o0(b4_o0), .heap_raddr(b4_heap_raddr), .heap_rdata(heap_rdata_bus), .heap_we(b4_heap_we), .heap_waddr(b4_heap_waddr), .heap_wdata(b4_heap_wdata), .alloc_req(b4_alloc_req), .alloc_size(b4_alloc_size), .hp_in(hp), .done(b4_done));
  reg cur_done;
  always @(*) begin cur_done = 1'b1; case (pc)
    3'd0: cur_done = b0_done;
    3'd1: cur_done = b1_done;
    3'd2: cur_done = b2_done;
    3'd3: cur_done = b3_done;
    3'd4: cur_done = b4_done;
    default: cur_done = 1'b1;
  endcase end
  always @(posedge clk) begin
    if (rst) begin
      pc<=3'd0; done_r<=1'b0; result_r<=64'd0; started<=1'b0;
      file[0]<=p0;
      file[1]<=64'd0;
      file[2]<=64'd0;
      file[3]<=64'd0;
      file[4]<=64'd0;
      file[5]<=64'd0;
      file[6]<=64'd0;
      file[7]<=64'd0;
      file[8]<=64'd0;
      file[9]<=64'd0;
      file[10]<=64'd0;
      file[11]<=64'd0;
      file[12]<=64'd0;
      file[13]<=64'd0;
      file[14]<=64'd0;
      file[15]<=64'd0;
      file[16]<=64'd0;
      file[17]<=64'd0;
    end else if (!done_r) begin
      if (!started) started<=1'b1;
      else if (cur_done) begin
        started<=1'b0;
        case (pc)
          3'd0: begin
            file[2] <= b0_o0;
            file[6] <= b0_o1;
            file[3] <= b0_o2;
            file[7] <= b0_o3;
            file[4] <= b0_o4;
            file[8] <= b0_o5;
            file[5] <= b0_o6;
            file[1] <= b0_o7;
            file[9] <= b0_o8;
            file[10] <= b0_o9;
            file[11] <= b0_o10;
            file[12] <= b0_o11;
            file[13] <= b0_o12;
            pc <= 3'd1;
          end
          3'd1: begin
            file[15] <= b1_o0;
            file[14] <= b1_o1;
            pc <= (b1_o2 != 64'd0) ? 3'd2 : 3'd3;
          end
          3'd2: begin
            file[16] <= b2_o0;
            file[11] <= b2_o1;
            file[17] <= b2_o2;
            file[13] <= b2_o3;
            pc <= 3'd1;
          end
          3'd3: begin
            result_r <= b3_o0; done_r <= 1'b1;
          end
          3'd4: begin
            result_r <= b4_o0; done_r <= 1'b1;
          end
          default: done_r<=1'b1;
        endcase
      end
    end
  end
endmodule

// nexys_seqprog_lsum — the whole-program SEQUENCED fabric (multi-cycle division/float tiles) on a Nexys A7.
// A baked vector ROM runs seqprog_lsum for each input (drive params -> pulse fab_rst -> wait done -> capture),
// streaming {0xA5, result[63:0], run_cycles[15:0], 0x5A} out the UART per vector. 781.25 kHz core clock.
module nexys_seqprog_lsum(input clk, input rst, output reg uart_tx, output [15:0] led);
  localparam CLKS_PER_BIT = 81;   // 781.25e3 / 9600
  localparam NVEC = 4;
  // /128 core clock on a global buffer.
  reg [6:0] divcnt = 7'd0;
  always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire coreclk;
  BUFG bufg_core(.I(divcnt[6]), .O(coreclk));
  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 1'b1;
  wire soc_rst = rst | ~por[3];

  reg [1:0] vec;
  reg signed [63:0] p0;
  always @(*) begin
    p0=64'sd0;
    case (vec)
      0: begin p0=64'sd1; end
      1: begin p0=64'sd3; end
      2: begin p0=64'sd5; end
      3: begin p0=64'sd8; end
      default: ;
    endcase
  end
  reg fab_rst;
  wire [63:0] fab_result; wire fab_done;
  reg [63:0] result_l; reg [15:0] cyc_l; reg [15:0] runcyc;
  seqprog_lsum u_fab(.clk(coreclk), .rst(fab_rst), .p0(p0), .result(fab_result), .done(fab_done));

  localparam R_POR=3'd0, R_RUN=3'd1, R_WAIT=3'd2, R_LOAD=3'd3, R_START=3'd4, R_DATA=3'd5, R_STOP=3'd6;
  reg [2:0] rstate;
  reg [15:0] clkc; reg [2:0] bidx; reg [3:0] pos; reg [3:0] porc; reg [1:0] rstc;
  reg [7:0] txb;
  assign led = {rstate, vec[4:0], 8'd0};
  wire [3:0] rsel = (pos >= 4'd1 && pos <= 4'd8) ? (4'd8 - pos) : 4'd0;
  wire [7:0] res_byte = result_l[rsel * 8 +: 8];
  always @(posedge coreclk) begin
    if (soc_rst) begin
      rstate<=R_POR; uart_tx<=1'b1; clkc<=16'd0; bidx<=3'd0; vec<=2'd0; pos<=4'd0; porc<=4'd0;
      fab_rst<=1'b1; rstc<=2'd0; runcyc<=16'd0; result_l<=64'd0; cyc_l<=16'd0;
    end else case (rstate)
      R_POR: if (porc==4'd8) begin rstate<=R_RUN; fab_rst<=1'b1; rstc<=2'd0; end else porc<=porc+4'd1;
      R_RUN: begin
        // hold fab_rst 2 coreclks (load params + reset the fabric), then release to run.
        if (rstc==2'd2) begin fab_rst<=1'b0; runcyc<=16'd0; rstate<=R_WAIT; end else rstc<=rstc+2'd1;
      end
      R_WAIT: begin
        runcyc <= runcyc + 16'd1;
        if (fab_done) begin result_l<=fab_result; cyc_l<=runcyc; pos<=4'd0; rstate<=R_LOAD; end
      end
      R_LOAD: begin
        case (pos)
          4'd0:  txb<=8'hA5;
          4'd9:  txb<=cyc_l[15:8];
          4'd10: txb<=cyc_l[7:0];
          4'd11: txb<=8'h5A;
          default: txb<=res_byte;   // pos 1..8 = result[63:0] MSB-first
        endcase
        clkc<=16'd0; bidx<=3'd0; rstate<=R_START;
      end
      R_START: begin uart_tx<=1'b0; if (clkc==CLKS_PER_BIT-1) begin clkc<=16'd0; rstate<=R_DATA; end else clkc<=clkc+16'd1; end
      R_DATA: begin uart_tx<=txb[bidx];
        if (clkc==CLKS_PER_BIT-1) begin clkc<=16'd0; if (bidx==3'd7) rstate<=R_STOP; else bidx<=bidx+3'd1; end else clkc<=clkc+16'd1;
      end
      R_STOP: begin uart_tx<=1'b1;
        if (clkc==CLKS_PER_BIT-1) begin clkc<=16'd0;
          if (pos==4'd11) begin
            pos<=4'd0;
            if (vec==NVEC-1) vec<=2'd0; else vec<=vec+2'd1;
            fab_rst<=1'b1; rstc<=2'd0; rstate<=R_RUN;
          end else begin pos<=pos+4'd1; rstate<=R_LOAD; end
        end else clkc<=clkc+16'd1;
      end
      default: rstate<=R_POR;
    endcase
  end
endmodule
