// seqprog_pi — the whole-program SEQUENCED fabric (LATENCY-INSENSITIVE): per-block datapaths with
// multi-cycle division/float TILES + a block-PC FSM that starts each block, waits its done, latches the
// register writes, and takes the terminator. == run_program_cfg_on_grid == the VM.
// seqblk_pi_b0 — a per-block SEQUENCED datapath (LATENCY-INSENSITIVE): start pulse -> walk the cells
// (combinational in one step, division/float on the shared multi-cycle units) -> done + the block's outputs.
module seqblk_pi_b0(input clk, input rst, input start, output [63:0] o0, output [63:0] o1, output [63:0] o2, output [63:0] o3, output [63:0] o4, output [63:0] o5, output done);
  localparam M = 0;
  (* ram_style="registers" *) reg [63:0] R [0:0];
  reg [31:0] step;
  reg running, issued;
  reg is_comb; reg [63:0] comb_y; reg [63:0] ua, ub; reg [3:0] usel;
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
    case (step)
      default: ;
    endcase
  end
  wire is_div   = (usel>=4'd1 && usel<=4'd3);
  wire is_mod   = (usel==4'd2);
  wire is_floor = (usel==4'd3);
  wire is_fadd  = (usel==4'd5)||(usel==4'd6);
  wire is_fsub  = (usel==4'd6);
  wire is_fmul  = (usel==4'd7);
  wire is_fdiv  = (usel==4'd8);
  wire is_fsqrt = (usel==4'd9);
  wire pulse = running && !issued && !is_comb;
  wire d_done; wire [63:0] d_q, d_r;
  shared_divider #(.W(64)) u_div(.clk(clk), .rst(rst), .start(pulse && is_div && (ub!=64'd0)),
                                 .a(ua), .b(ub), .q(d_q), .r(d_r), .done(d_done));
  wire signed [63:0] d_floor_v = $signed(d_q) - (((d_r!=64'd0) && (ua[63]^ub[63])) ? 64'sd1 : 64'sd0);
  wire [63:0] div_y     = (ub==64'd0) ? 64'd0 : is_mod ? d_r : is_floor ? d_floor_v : d_q;
  wire        div_done_g= (ub==64'd0) ? 1'b1 : d_done;
  wire fa_done, fm_done, fd_done, fs_done; wire [63:0] fa_y, fm_y, fd_y, fs_y;
  fp_addsub u_fa(.clk(clk),.rst(rst),.start(pulse && is_fadd),.a(ua),.b_in(ub),.sub(is_fsub),.y(fa_y),.done(fa_done));
  fp_mul    u_fm(.clk(clk),.rst(rst),.start(pulse && is_fmul),.a(ua),.b(ub),.y(fm_y),.done(fm_done));
  fp_div    u_fd(.clk(clk),.rst(rst),.start(pulse && is_fdiv),.a(ua),.b(ub),.y(fd_y),.done(fd_done));
  fp_sqrt   u_fs(.clk(clk),.rst(rst),.start(pulse && is_fsqrt),.a(ua),.y(fs_y),.done(fs_done));
  wire        mc_done = is_div ? div_done_g : is_fadd ? fa_done : is_fmul ? fm_done : is_fdiv ? fd_done : is_fsqrt ? fs_done : 1'b1;
  wire [63:0] mc_y    = is_div ? div_y : is_fadd ? fa_y : is_fmul ? fm_y : is_fdiv ? fd_y : is_fsqrt ? fs_y : 64'd0;
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
  assign o0 = 64'sd0;
  assign o1 = 64'sd0;
  assign o2 = 64'sd4607182418800017408;
  assign o3 = 64'sd4607182418800017408;
  assign o4 = 64'sd0;
  assign o5 = 64'sd0;
endmodule

// seqblk_pi_b1 — a per-block SEQUENCED datapath (LATENCY-INSENSITIVE): start pulse -> walk the cells
// (combinational in one step, division/float on the shared multi-cycle units) -> done + the block's outputs.
module seqblk_pi_b1(input clk, input rst, input start, input [63:0] in0, input [63:0] in1, output [63:0] o0, output [63:0] o1, output done);
  localparam M = 1;
  (* ram_style="registers" *) reg [63:0] R [0:0];
  reg [31:0] step;
  reg running, issued;
  reg is_comb; reg [63:0] comb_y; reg [63:0] ua, ub; reg [3:0] usel;
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
    case (step)
      0: begin is_comb=1'b1; comb_y=($signed(in0) < $signed(in1)) ? 64'sd1 : 64'sd0; end
      default: ;
    endcase
  end
  wire is_div   = (usel>=4'd1 && usel<=4'd3);
  wire is_mod   = (usel==4'd2);
  wire is_floor = (usel==4'd3);
  wire is_fadd  = (usel==4'd5)||(usel==4'd6);
  wire is_fsub  = (usel==4'd6);
  wire is_fmul  = (usel==4'd7);
  wire is_fdiv  = (usel==4'd8);
  wire is_fsqrt = (usel==4'd9);
  wire pulse = running && !issued && !is_comb;
  wire d_done; wire [63:0] d_q, d_r;
  shared_divider #(.W(64)) u_div(.clk(clk), .rst(rst), .start(pulse && is_div && (ub!=64'd0)),
                                 .a(ua), .b(ub), .q(d_q), .r(d_r), .done(d_done));
  wire signed [63:0] d_floor_v = $signed(d_q) - (((d_r!=64'd0) && (ua[63]^ub[63])) ? 64'sd1 : 64'sd0);
  wire [63:0] div_y     = (ub==64'd0) ? 64'd0 : is_mod ? d_r : is_floor ? d_floor_v : d_q;
  wire        div_done_g= (ub==64'd0) ? 1'b1 : d_done;
  wire fa_done, fm_done, fd_done, fs_done; wire [63:0] fa_y, fm_y, fd_y, fs_y;
  fp_addsub u_fa(.clk(clk),.rst(rst),.start(pulse && is_fadd),.a(ua),.b_in(ub),.sub(is_fsub),.y(fa_y),.done(fa_done));
  fp_mul    u_fm(.clk(clk),.rst(rst),.start(pulse && is_fmul),.a(ua),.b(ub),.y(fm_y),.done(fm_done));
  fp_div    u_fd(.clk(clk),.rst(rst),.start(pulse && is_fdiv),.a(ua),.b(ub),.y(fd_y),.done(fd_done));
  fp_sqrt   u_fs(.clk(clk),.rst(rst),.start(pulse && is_fsqrt),.a(ua),.y(fs_y),.done(fs_done));
  wire        mc_done = is_div ? div_done_g : is_fadd ? fa_done : is_fmul ? fm_done : is_fdiv ? fd_done : is_fsqrt ? fs_done : 1'b1;
  wire [63:0] mc_y    = is_div ? div_y : is_fadd ? fa_y : is_fmul ? fm_y : is_fdiv ? fd_y : is_fsqrt ? fs_y : 64'd0;
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
  assign o0 = R[0];
  assign o1 = R[0];
endmodule

// seqblk_pi_b2 — a per-block SEQUENCED datapath (LATENCY-INSENSITIVE): start pulse -> walk the cells
// (combinational in one step, division/float on the shared multi-cycle units) -> done + the block's outputs.
module seqblk_pi_b2(input clk, input rst, input start, input [63:0] in0, input [63:0] in1, input [63:0] in2, output [63:0] o0, output [63:0] o1, output [63:0] o2, output [63:0] o3, output [63:0] o4, output [63:0] o5, output [63:0] o6, output [63:0] o7, output [63:0] o8, output [63:0] o9, output [63:0] o10, output done);
  localparam M = 7;
  (* ram_style="registers" *) reg [63:0] R [0:6];
  reg [31:0] step;
  reg running, issued;
  reg is_comb; reg [63:0] comb_y; reg [63:0] ua, ub; reg [3:0] usel;
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
    case (step)
      0: begin is_comb=1'b1; comb_y=i64_to_f64(in0); end
      1: begin is_comb=1'b0; ua=64'sd4611686018427387904; ub=R[0]; usel=4'd7; end
      2: begin is_comb=1'b0; ua=R[1]; ub=64'sd4607182418800017408; usel=4'd5; end
      3: begin is_comb=1'b0; ua=in1; ub=R[2]; usel=4'd8; end
      4: begin is_comb=1'b0; ua=in2; ub=R[3]; usel=4'd5; end
      5: begin is_comb=1'b0; ua=64'sd0; ub=in1; usel=4'd6; end
      6: begin is_comb=1'b1; comb_y=$signed(in0) + $signed(64'sd1); end
      default: ;
    endcase
  end
  wire is_div   = (usel>=4'd1 && usel<=4'd3);
  wire is_mod   = (usel==4'd2);
  wire is_floor = (usel==4'd3);
  wire is_fadd  = (usel==4'd5)||(usel==4'd6);
  wire is_fsub  = (usel==4'd6);
  wire is_fmul  = (usel==4'd7);
  wire is_fdiv  = (usel==4'd8);
  wire is_fsqrt = (usel==4'd9);
  wire pulse = running && !issued && !is_comb;
  wire d_done; wire [63:0] d_q, d_r;
  shared_divider #(.W(64)) u_div(.clk(clk), .rst(rst), .start(pulse && is_div && (ub!=64'd0)),
                                 .a(ua), .b(ub), .q(d_q), .r(d_r), .done(d_done));
  wire signed [63:0] d_floor_v = $signed(d_q) - (((d_r!=64'd0) && (ua[63]^ub[63])) ? 64'sd1 : 64'sd0);
  wire [63:0] div_y     = (ub==64'd0) ? 64'd0 : is_mod ? d_r : is_floor ? d_floor_v : d_q;
  wire        div_done_g= (ub==64'd0) ? 1'b1 : d_done;
  wire fa_done, fm_done, fd_done, fs_done; wire [63:0] fa_y, fm_y, fd_y, fs_y;
  fp_addsub u_fa(.clk(clk),.rst(rst),.start(pulse && is_fadd),.a(ua),.b_in(ub),.sub(is_fsub),.y(fa_y),.done(fa_done));
  fp_mul    u_fm(.clk(clk),.rst(rst),.start(pulse && is_fmul),.a(ua),.b(ub),.y(fm_y),.done(fm_done));
  fp_div    u_fd(.clk(clk),.rst(rst),.start(pulse && is_fdiv),.a(ua),.b(ub),.y(fd_y),.done(fd_done));
  fp_sqrt   u_fs(.clk(clk),.rst(rst),.start(pulse && is_fsqrt),.a(ua),.y(fs_y),.done(fs_done));
  wire        mc_done = is_div ? div_done_g : is_fadd ? fa_done : is_fmul ? fm_done : is_fdiv ? fd_done : is_fsqrt ? fs_done : 1'b1;
  wire [63:0] mc_y    = is_div ? div_y : is_fadd ? fa_y : is_fmul ? fm_y : is_fdiv ? fd_y : is_fsqrt ? fs_y : 64'd0;
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
  assign o0 = 64'sd4611686018427387904;
  assign o1 = R[1];
  assign o2 = 64'sd4607182418800017408;
  assign o3 = R[2];
  assign o4 = R[3];
  assign o5 = R[4];
  assign o6 = 64'sd0;
  assign o7 = R[5];
  assign o8 = R[5];
  assign o9 = 64'sd1;
  assign o10 = R[6];
endmodule

// seqblk_pi_b3 — a per-block SEQUENCED datapath (LATENCY-INSENSITIVE): start pulse -> walk the cells
// (combinational in one step, division/float on the shared multi-cycle units) -> done + the block's outputs.
module seqblk_pi_b3(input clk, input rst, input start, input [63:0] in0, output [63:0] o0, output [63:0] o1, output [63:0] o2, output [63:0] o3, output [63:0] o4, output [63:0] o5, output done);
  localparam M = 3;
  (* ram_style="registers" *) reg [63:0] R [0:2];
  reg [31:0] step;
  reg running, issued;
  reg is_comb; reg [63:0] comb_y; reg [63:0] ua, ub; reg [3:0] usel;
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
    case (step)
      0: begin is_comb=1'b0; ua=in0; ub=64'sd4616189618054758400; usel=4'd7; end
      1: begin is_comb=1'b0; ua=R[0]; ub=64'sd4696837146684686336; usel=4'd7; end
      2: begin is_comb=1'b1; comb_y=f64_floor_i64(R[1]); end
      default: ;
    endcase
  end
  wire is_div   = (usel>=4'd1 && usel<=4'd3);
  wire is_mod   = (usel==4'd2);
  wire is_floor = (usel==4'd3);
  wire is_fadd  = (usel==4'd5)||(usel==4'd6);
  wire is_fsub  = (usel==4'd6);
  wire is_fmul  = (usel==4'd7);
  wire is_fdiv  = (usel==4'd8);
  wire is_fsqrt = (usel==4'd9);
  wire pulse = running && !issued && !is_comb;
  wire d_done; wire [63:0] d_q, d_r;
  shared_divider #(.W(64)) u_div(.clk(clk), .rst(rst), .start(pulse && is_div && (ub!=64'd0)),
                                 .a(ua), .b(ub), .q(d_q), .r(d_r), .done(d_done));
  wire signed [63:0] d_floor_v = $signed(d_q) - (((d_r!=64'd0) && (ua[63]^ub[63])) ? 64'sd1 : 64'sd0);
  wire [63:0] div_y     = (ub==64'd0) ? 64'd0 : is_mod ? d_r : is_floor ? d_floor_v : d_q;
  wire        div_done_g= (ub==64'd0) ? 1'b1 : d_done;
  wire fa_done, fm_done, fd_done, fs_done; wire [63:0] fa_y, fm_y, fd_y, fs_y;
  fp_addsub u_fa(.clk(clk),.rst(rst),.start(pulse && is_fadd),.a(ua),.b_in(ub),.sub(is_fsub),.y(fa_y),.done(fa_done));
  fp_mul    u_fm(.clk(clk),.rst(rst),.start(pulse && is_fmul),.a(ua),.b(ub),.y(fm_y),.done(fm_done));
  fp_div    u_fd(.clk(clk),.rst(rst),.start(pulse && is_fdiv),.a(ua),.b(ub),.y(fd_y),.done(fd_done));
  fp_sqrt   u_fs(.clk(clk),.rst(rst),.start(pulse && is_fsqrt),.a(ua),.y(fs_y),.done(fs_done));
  wire        mc_done = is_div ? div_done_g : is_fadd ? fa_done : is_fmul ? fm_done : is_fdiv ? fd_done : is_fsqrt ? fs_done : 1'b1;
  wire [63:0] mc_y    = is_div ? div_y : is_fadd ? fa_y : is_fmul ? fm_y : is_fdiv ? fd_y : is_fsqrt ? fs_y : 64'd0;
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
  assign o0 = 64'sd4616189618054758400;
  assign o1 = R[0];
  assign o2 = 64'sd4696837146684686336;
  assign o3 = R[1];
  assign o4 = R[2];
  assign o5 = R[2];
endmodule

// seqblk_pi_b4 — a per-block SEQUENCED datapath (LATENCY-INSENSITIVE): start pulse -> walk the cells
// (combinational in one step, division/float on the shared multi-cycle units) -> done + the block's outputs.
module seqblk_pi_b4(input clk, input rst, input start, output [63:0] o0, output done);
  localparam M = 0;
  (* ram_style="registers" *) reg [63:0] R [0:0];
  reg [31:0] step;
  reg running, issued;
  reg is_comb; reg [63:0] comb_y; reg [63:0] ua, ub; reg [3:0] usel;
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
    case (step)
      default: ;
    endcase
  end
  wire is_div   = (usel>=4'd1 && usel<=4'd3);
  wire is_mod   = (usel==4'd2);
  wire is_floor = (usel==4'd3);
  wire is_fadd  = (usel==4'd5)||(usel==4'd6);
  wire is_fsub  = (usel==4'd6);
  wire is_fmul  = (usel==4'd7);
  wire is_fdiv  = (usel==4'd8);
  wire is_fsqrt = (usel==4'd9);
  wire pulse = running && !issued && !is_comb;
  wire d_done; wire [63:0] d_q, d_r;
  shared_divider #(.W(64)) u_div(.clk(clk), .rst(rst), .start(pulse && is_div && (ub!=64'd0)),
                                 .a(ua), .b(ub), .q(d_q), .r(d_r), .done(d_done));
  wire signed [63:0] d_floor_v = $signed(d_q) - (((d_r!=64'd0) && (ua[63]^ub[63])) ? 64'sd1 : 64'sd0);
  wire [63:0] div_y     = (ub==64'd0) ? 64'd0 : is_mod ? d_r : is_floor ? d_floor_v : d_q;
  wire        div_done_g= (ub==64'd0) ? 1'b1 : d_done;
  wire fa_done, fm_done, fd_done, fs_done; wire [63:0] fa_y, fm_y, fd_y, fs_y;
  fp_addsub u_fa(.clk(clk),.rst(rst),.start(pulse && is_fadd),.a(ua),.b_in(ub),.sub(is_fsub),.y(fa_y),.done(fa_done));
  fp_mul    u_fm(.clk(clk),.rst(rst),.start(pulse && is_fmul),.a(ua),.b(ub),.y(fm_y),.done(fm_done));
  fp_div    u_fd(.clk(clk),.rst(rst),.start(pulse && is_fdiv),.a(ua),.b(ub),.y(fd_y),.done(fd_done));
  fp_sqrt   u_fs(.clk(clk),.rst(rst),.start(pulse && is_fsqrt),.a(ua),.y(fs_y),.done(fs_done));
  wire        mc_done = is_div ? div_done_g : is_fadd ? fa_done : is_fmul ? fm_done : is_fdiv ? fd_done : is_fsqrt ? fs_done : 1'b1;
  wire [63:0] mc_y    = is_div ? div_y : is_fadd ? fa_y : is_fmul ? fm_y : is_fdiv ? fd_y : is_fsqrt ? fs_y : 64'd0;
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
  assign o0 = 64'sd0;
endmodule

module seqprog_pi(
    input clk,
    input rst,
    input [63:0] p0,
    output [63:0] result,
    output done
);
  reg [63:0] file [0:15];
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
  wire [63:0] b0_o0;
  wire [63:0] b0_o1;
  wire [63:0] b0_o2;
  wire [63:0] b0_o3;
  wire [63:0] b0_o4;
  wire [63:0] b0_o5;
  wire b0_done;
  seqblk_pi_b0 u_b0(.clk(clk), .rst(rst), .start((pc==3'd0) && !started && !done_r), .o0(b0_o0), .o1(b0_o1), .o2(b0_o2), .o3(b0_o3), .o4(b0_o4), .o5(b0_o5), .done(b0_done));
  wire [63:0] b1_o0;
  wire [63:0] b1_o1;
  wire b1_done;
  seqblk_pi_b1 u_b1(.clk(clk), .rst(rst), .start((pc==3'd1) && !started && !done_r), .in0(fr_6), .in1(fr_0), .o0(b1_o0), .o1(b1_o1), .done(b1_done));
  wire [63:0] b2_o0;
  wire [63:0] b2_o1;
  wire [63:0] b2_o2;
  wire [63:0] b2_o3;
  wire [63:0] b2_o4;
  wire [63:0] b2_o5;
  wire [63:0] b2_o6;
  wire [63:0] b2_o7;
  wire [63:0] b2_o8;
  wire [63:0] b2_o9;
  wire [63:0] b2_o10;
  wire b2_done;
  seqblk_pi_b2 u_b2(.clk(clk), .rst(rst), .start((pc==3'd2) && !started && !done_r), .in0(fr_6), .in1(fr_4), .in2(fr_2), .o0(b2_o0), .o1(b2_o1), .o2(b2_o2), .o3(b2_o3), .o4(b2_o4), .o5(b2_o5), .o6(b2_o6), .o7(b2_o7), .o8(b2_o8), .o9(b2_o9), .o10(b2_o10), .done(b2_done));
  wire [63:0] b3_o0;
  wire [63:0] b3_o1;
  wire [63:0] b3_o2;
  wire [63:0] b3_o3;
  wire [63:0] b3_o4;
  wire [63:0] b3_o5;
  wire b3_done;
  seqblk_pi_b3 u_b3(.clk(clk), .rst(rst), .start((pc==3'd3) && !started && !done_r), .in0(fr_2), .o0(b3_o0), .o1(b3_o1), .o2(b3_o2), .o3(b3_o3), .o4(b3_o4), .o5(b3_o5), .done(b3_done));
  wire [63:0] b4_o0;
  wire b4_done;
  seqblk_pi_b4 u_b4(.clk(clk), .rst(rst), .start((pc==3'd4) && !started && !done_r), .o0(b4_o0), .done(b4_done));
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
    end else if (!done_r) begin
      if (!started) started<=1'b1;
      else if (cur_done) begin
        started<=1'b0;
        case (pc)
          3'd0: begin
            file[1] <= b0_o0;
            file[2] <= b0_o1;
            file[3] <= b0_o2;
            file[4] <= b0_o3;
            file[5] <= b0_o4;
            file[6] <= b0_o5;
            pc <= 3'd1;
          end
          3'd1: begin
            file[7] <= b1_o0;
            pc <= (b1_o1 != 64'd0) ? 3'd2 : 3'd3;
          end
          3'd2: begin
            file[11] <= b2_o0;
            file[10] <= b2_o1;
            file[12] <= b2_o2;
            file[9] <= b2_o3;
            file[8] <= b2_o4;
            file[2] <= b2_o5;
            file[14] <= b2_o6;
            file[13] <= b2_o7;
            file[4] <= b2_o8;
            file[15] <= b2_o9;
            file[6] <= b2_o10;
            pc <= 3'd1;
          end
          3'd3: begin
            file[11] <= b3_o0;
            file[10] <= b3_o1;
            file[12] <= b3_o2;
            file[9] <= b3_o3;
            file[8] <= b3_o4;
            result_r <= b3_o5; done_r <= 1'b1;
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

// nexys_seqprog_pi — the whole-program SEQUENCED fabric (multi-cycle division/float tiles) on a Nexys A7.
// A baked vector ROM runs seqprog_pi for each input (drive params -> pulse fab_rst -> wait done -> capture),
// streaming {0xA5, result[63:0], run_cycles[15:0], 0x5A} out the UART per vector. 781.25 kHz core clock.
module nexys_seqprog_pi(input clk, input rst, output reg uart_tx, output [15:0] led);
  localparam CLKS_PER_BIT = 81;   // 781.25e3 / 9600
  localparam NVEC = 3;
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
      1: begin p0=64'sd2; end
      2: begin p0=64'sd5; end
      default: ;
    endcase
  end
  reg fab_rst;
  wire [63:0] fab_result; wire fab_done;
  reg [63:0] result_l; reg [15:0] cyc_l; reg [15:0] runcyc;
  seqprog_pi u_fab(.clk(coreclk), .rst(fab_rst), .p0(p0), .result(fab_result), .done(fab_done));

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
