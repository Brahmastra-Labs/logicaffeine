// The crossing between the engine clock the shell runs in (sclk) and the user clock it is divided from (fclk), every
// register of it in the user clock (fedge and fmid high the user cycles that end with the engine clock's rising and
// falling edges): each value the shell gives the engine taken straight into a register (cap_*) two user cycles after the
// rising edge, each value the engine gives the shell taken straight into a register (held_*) two user cycles after the
// falling edge. With the engine clock 6 user cycles or more, a path from the shell into a cap_* register has two
// user cycles and one from a held_* register into the shell half an engine cycle less two user cycles.
module logos_engine_bridge
  (input sclk, input fclk, input frst, input fedge, input fmid,
   input s_rst, output f_rst,
   input s_start, output f_start,
   input [31:0] s_job_at, output [31:0] f_job_at,
   input [31:0] s_job_end, output [31:0] f_job_end,
   input [15:0] s_rows, output [15:0] f_rows,
   input s_state_f16, output f_state_f16,
   input s_cache_int8, output f_cache_int8,
   input s_two_words, output f_two_words,
   input s_per_block, output f_per_block,
   output s_busy, input f_busy,
   output s_fault, input f_fault,
   output [31:0] s_pc, input [31:0] f_pc,
   input s_host_wr, output f_host_wr,
   input [2:0] s_host_table, output [2:0] f_host_table,
   input [31:0] s_host_addr, output [31:0] f_host_addr,
   input [511:0] s_host_wdata, output [511:0] f_host_wdata,
   input [31:0] s_host_raddr, output [31:0] f_host_raddr,
   output [511:0] s_host_rdata, input [511:0] f_host_rdata,
   output s_mem_rd, input f_mem_rd,
   output [31:0] s_mem_rd_line, input [31:0] f_mem_rd_line,
   input s_mem_rd_valid, output f_mem_rd_valid,
   input [511:0] s_mem_rd_data, output [511:0] f_mem_rd_data,
   output s_mem_wr, input f_mem_wr,
   output [31:0] s_mem_wr_line, input [31:0] f_mem_wr_line,
   output [511:0] s_mem_wr_data, input [511:0] f_mem_wr_data,
   output [63:0] s_mem_wr_mask, input [63:0] f_mem_wr_mask,
   input s_mem_wr_ready, output f_mem_wr_ready);
  reg cap_rst;
  reg cap_start;
  reg [31:0] cap_job_at;
  reg [31:0] cap_job_end;
  reg [15:0] cap_rows;
  reg cap_state_f16;
  reg cap_cache_int8;
  reg cap_two_words;
  reg cap_per_block;
  reg held_busy;
  reg held_fault;
  reg [31:0] held_pc;
  reg cap_host_wr;
  reg [2:0] cap_host_table;
  reg [31:0] cap_host_addr;
  reg [511:0] cap_host_wdata;
  reg [31:0] cap_host_raddr;
  reg [511:0] held_host_rdata;
  reg held_mem_rd;
  reg [31:0] held_mem_rd_line;
  reg cap_mem_rd_valid;
  reg [511:0] cap_mem_rd_data;
  reg held_mem_wr;
  reg [31:0] held_mem_wr_line;
  reg [511:0] held_mem_wr_data;
  reg [63:0] held_mem_wr_mask;
  reg cap_mem_wr_ready;
  reg emark1, emark2, mark1, mark2;
  always @(posedge fclk) begin
    emark1 <= fedge;
    emark2 <= emark1;
    mark1 <= fmid;
    mark2 <= mark1;
    if (frst) cap_rst <= 1'b1; else if (emark2) cap_rst <= s_rst;
    if (emark2) cap_start <= s_start;
    if (emark2) cap_job_at <= s_job_at;
    if (emark2) cap_job_end <= s_job_end;
    if (emark2) cap_rows <= s_rows;
    if (emark2) cap_state_f16 <= s_state_f16;
    if (emark2) cap_cache_int8 <= s_cache_int8;
    if (emark2) cap_two_words <= s_two_words;
    if (emark2) cap_per_block <= s_per_block;
    if (emark2) cap_host_wr <= s_host_wr;
    if (emark2) cap_host_table <= s_host_table;
    if (emark2) cap_host_addr <= s_host_addr;
    if (emark2) cap_host_wdata <= s_host_wdata;
    if (emark2) cap_host_raddr <= s_host_raddr;
    if (emark2) cap_mem_rd_valid <= s_mem_rd_valid;
    if (emark2) cap_mem_rd_data <= s_mem_rd_data;
    if (emark2) cap_mem_wr_ready <= s_mem_wr_ready;
    if (mark2) held_busy <= f_busy;
    if (mark2) held_fault <= f_fault;
    if (mark2) held_pc <= f_pc;
    if (mark2) held_host_rdata <= f_host_rdata;
    if (mark2) held_mem_rd <= f_mem_rd;
    if (mark2) held_mem_rd_line <= f_mem_rd_line;
    if (mark2) held_mem_wr <= f_mem_wr;
    if (mark2) held_mem_wr_line <= f_mem_wr_line;
    if (mark2) held_mem_wr_data <= f_mem_wr_data;
    if (mark2) held_mem_wr_mask <= f_mem_wr_mask;
  end
  assign f_rst = cap_rst;
  assign f_start = cap_start;
  assign f_job_at = cap_job_at;
  assign f_job_end = cap_job_end;
  assign f_rows = cap_rows;
  assign f_state_f16 = cap_state_f16;
  assign f_cache_int8 = cap_cache_int8;
  assign f_two_words = cap_two_words;
  assign f_per_block = cap_per_block;
  assign s_busy = held_busy;
  assign s_fault = held_fault;
  assign s_pc = held_pc;
  assign f_host_wr = cap_host_wr;
  assign f_host_table = cap_host_table;
  assign f_host_addr = cap_host_addr;
  assign f_host_wdata = cap_host_wdata;
  assign f_host_raddr = cap_host_raddr;
  assign s_host_rdata = held_host_rdata;
  assign s_mem_rd = held_mem_rd;
  assign s_mem_rd_line = held_mem_rd_line;
  assign f_mem_rd_valid = cap_mem_rd_valid;
  assign f_mem_rd_data = cap_mem_rd_data;
  assign s_mem_wr = held_mem_wr;
  assign s_mem_wr_line = held_mem_wr_line;
  assign s_mem_wr_data = held_mem_wr_data;
  assign s_mem_wr_mask = held_mem_wr_mask;
  assign f_mem_wr_ready = cap_mem_wr_ready;
endmodule

module logos_fpo_mul0(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul1(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul2(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul3(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul4(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul5(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul6(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul7(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul8(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul9(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul10(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul11(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul12(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul13(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul14(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul15(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul16(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul17(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul18(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul19(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul20(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul21(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul22(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul23(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul24(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : {s, 31'd0})
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul25(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : {s, 31'd0})
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul26(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : {s, 31'd0})
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul27(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : {s, 31'd0})
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul28(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : {s, 31'd0})
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul29(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : {s, 31'd0})
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul30(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : 32'd0)
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul31(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : 32'd0)
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul32(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : 32'd0)
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul33(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : 32'd0)
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul34(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : 32'd0)
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul35(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : 32'd0)
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul36(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul37(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul38(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul39(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul40(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul41(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  wire        s = sa ^ sb;
  wire [47:0] p = {1'b1, ma} * {1'b1, mb};
  wire [5:0]  lz = p[47] ? 6'd0 : 6'd1;
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul42(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul43(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul44(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul45(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul46(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul47(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul48(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul49(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul50(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul51(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul52(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul53(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul54(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul55(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul56(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul57(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul58(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul59(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? {s, 31'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul60(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul61(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul62(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul63(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul64(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul65(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? 32'd0
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul66(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : {s, 31'd0})
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul67(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : {s, 31'd0})
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul68(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : {s, 31'd0})
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul69(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : {s, 31'd0})
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul70(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : {s, 31'd0})
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul71(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : {s, 31'd0})
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul72(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : 32'd0)
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul73(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : 32'd0)
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul74(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : 32'd0)
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul75(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : 32'd0)
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul76(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : 32'd0)
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul77(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : (e < 12'sd1) ? ((e == 12'sd0 && cn) ? {s, 8'd1, 23'd0} : 32'd0)
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul78(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul79(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffc00000
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul80(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'h7fffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul81(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? 32'hffffffff
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul82(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_mul83(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  wire        s = sa ^ sb;
  wire [47:0] whole = {1'b1, ma} * {1'b1, mb};
  wire [47:0] p = whole - (da ? {24'd0, 1'b1, mb} << 23 : 48'd0) - (db ? {24'd0, 1'b1, ma} << 23 : 48'd0)
                   + ((da && db) ? 48'd1 << 46 : 48'd0);
  reg  [5:0]  lz_d;
  integer     i;
  always @* begin
    lz_d = 6'd48;
    for (i = 0; i <= 47; i = i + 1) if (p[i]) lz_d = 6'd47 - i[5:0];
  end
  wire [5:0]  lz = (da || db) ? lz_d : (p[47] ? 6'd0 : 6'd1);
  wire [47:0] pn = p << lz;
  wire signed [11:0] e = $signed({4'd0, xa}) + $signed({4'd0, xb}) - 12'sd126 - $signed({6'd0, lz});
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd48) ? 7'd48 : sh_raw[6:0];
  wire [95:0] wide = {pn, 48'd0} >> sh;
  wire [23:0] keep = wide[95:72];
  wire        guard = wide[71];
  wire        sticky = |wide[70:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[47:24] && pn[23];
  assign y = (na || nb || (ia && zb) || (ib && za)) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia || ib) ? {s, 8'hff, 23'd0}
    : (za || zb) ? {s, 31'd0}
    : (ef >= 12'sd255) ? {s, 8'hff, 23'd0}
    : {s, ef[7:0], mf};
endmodule
module logos_fpo_add0(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'h7fc00000
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s1, 31'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add1(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'hffc00000
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s1, 31'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add2(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'h7fffffff
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s1, 31'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add3(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'hffffffff
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s1, 31'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add4(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s1, 31'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add5(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s1, 31'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add6(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'h7fc00000
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add7(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'hffc00000
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add8(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'h7fffffff
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add9(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'hffffffff
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add10(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add11(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add12(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'h7fc00000
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add13(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'hffc00000
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add14(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'h7fffffff
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add15(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'hffffffff
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add16(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add17(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0), zb = (eb == 8'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire [23:0] fa = {1'b1, ma}, fb = {1'b1, mb};
  wire [7:0]  xa = ea, xb = eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = z2 ? 24'd0 : (swap ? fa : fb);
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add18(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'h7fc00000
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? ((eb == 8'd0) ? {sb, 31'd0} : b)
    : (zb) ? ((ea == 8'd0) ? {sa, 31'd0} : a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s1, 31'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add19(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'hffc00000
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? ((eb == 8'd0) ? {sb, 31'd0} : b)
    : (zb) ? ((ea == 8'd0) ? {sa, 31'd0} : a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s1, 31'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add20(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'h7fffffff
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? ((eb == 8'd0) ? {sb, 31'd0} : b)
    : (zb) ? ((ea == 8'd0) ? {sa, 31'd0} : a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s1, 31'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add21(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'hffffffff
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? ((eb == 8'd0) ? {sb, 31'd0} : b)
    : (zb) ? ((ea == 8'd0) ? {sa, 31'd0} : a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s1, 31'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add22(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? ((eb == 8'd0) ? {sb, 31'd0} : b)
    : (zb) ? ((ea == 8'd0) ? {sa, 31'd0} : a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s1, 31'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add23(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? ((eb == 8'd0) ? {sb, 31'd0} : b)
    : (zb) ? ((ea == 8'd0) ? {sa, 31'd0} : a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? {s1, 31'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add24(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'h7fc00000
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? ((eb == 8'd0) ? 32'd0 : b)
    : (zb) ? ((ea == 8'd0) ? 32'd0 : a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add25(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'hffc00000
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? ((eb == 8'd0) ? 32'd0 : b)
    : (zb) ? ((ea == 8'd0) ? 32'd0 : a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add26(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'h7fffffff
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? ((eb == 8'd0) ? 32'd0 : b)
    : (zb) ? ((ea == 8'd0) ? 32'd0 : a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add27(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'hffffffff
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? ((eb == 8'd0) ? 32'd0 : b)
    : (zb) ? ((ea == 8'd0) ? 32'd0 : a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add28(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? ((eb == 8'd0) ? 32'd0 : b)
    : (zb) ? ((ea == 8'd0) ? 32'd0 : a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add29(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? ((eb == 8'd0) ? 32'd0 : b)
    : (zb) ? ((ea == 8'd0) ? 32'd0 : a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : (ef <= 12'sd0) ? 32'd0
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add30(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'h7fc00000
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add31(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'hffc00000
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add32(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'h7fffffff
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add33(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? 32'hffffffff
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add34(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'h7fc00000)
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : {s1, ef[7:0], mf};
endmodule
module logos_fpo_add35(input [31:0] a, input [31:0] b, output [31:0] y);
  wire        sa = a[31], sb = b[31];
  wire [7:0]  ea = a[30:23], eb = b[30:23];
  wire [22:0] ma = a[22:0], mb = b[22:0];
  wire        za = (ea == 8'd0) && (ma == 23'd0), zb = (eb == 8'd0) && (mb == 23'd0);
  wire        ia = (ea == 8'hff) && (ma == 23'd0), ib = (eb == 8'hff) && (mb == 23'd0);
  wire        na = (ea == 8'hff) && (ma != 23'd0), nb = (eb == 8'hff) && (mb != 23'd0);
  wire        da = (ea == 8'd0) && (ma != 23'd0), db = (eb == 8'd0) && (mb != 23'd0);
  wire [23:0] fa = {!da, ma}, fb = {!db, mb};
  wire [7:0]  xa = da ? 8'd1 : ea, xb = db ? 8'd1 : eb;
  // The larger magnitude first.
  wire        swap = {ea, ma} < {eb, mb};
  wire        s1 = swap ? sb : sa, s2 = swap ? sa : sb;
  wire [7:0]  x1 = swap ? xb : xa, x2 = swap ? xa : xb;
  wire        z2 = swap ? za : zb;
  wire [23:0] f1 = swap ? fb : fa;
  wire [23:0] f2 = swap ? fa : fb;
  wire [7:0]  d = x1 - x2;
  // Three places below the significand: guard, round, and a sticky of everything shifted past them.
  wire [50:0] aligned = {f2, 27'd0} >> ((d > 8'd50) ? 8'd50 : d);
  wire [26:0] g2 = {aligned[50:25], |aligned[24:0]};
  wire [26:0] g1 = {f1, 3'd0};
  wire        sub = s1 ^ s2;
  wire [27:0] sum = sub ? {1'b0, g1} - {1'b0, g2} : {1'b0, g1} + {1'b0, g2};
  reg  [4:0]  lz;
  integer     i;
  always @* begin
    lz = 5'd27;
    for (i = 0; i <= 26; i = i + 1) if (sum[i]) lz = 5'd26 - i[4:0];
  end
  wire        c = sum[27];
  wire signed [11:0] e = c ? $signed({4'd0, x1}) + 12'sd1 : $signed({4'd0, x1}) - $signed({7'd0, lz});
  // Normalized with the leading one at bit 26, the sticky kept through a right shift.
  wire [26:0] pn = c ? {sum[27:2], sum[1] | sum[0]} : (sum[26:0] << lz);
  wire [11:0] sh_raw = (e < 12'sd1) ? (12'd1 - e) : 12'd0;
  wire [6:0]  sh = (sh_raw > 12'd27) ? 7'd27 : sh_raw[6:0];
  wire [53:0] wide = {pn, 27'd0} >> sh;
  wire [23:0] keep = wide[53:30];
  wire        guard = wide[29];
  wire        sticky = |wide[28:0];
  wire        up = guard && (sticky || keep[0]);
  wire [24:0] r = {1'b0, keep} + {24'd0, up};
  wire        carry = r[24];
  wire signed [11:0] ef = (e < 12'sd1) ? (r[23] ? 12'sd1 : 12'sd0) : (carry ? e + 12'sd1 : e);
  wire [22:0] mf = carry ? r[23:1] : r[22:0];
  wire        cn = &pn[26:3] && pn[2];
  wire        zero = (sum == 28'd0);
  assign y = (na || nb || (ia && ib && (sa != sb))) ? (na ? {sa, 8'hff, 1'b1, ma[21:0]} : nb ? {sb, 8'hff, 1'b1, mb[21:0]} : 32'hffc00000)
    : (ia) ? {sa, 8'hff, 23'd0}
    : (ib) ? {sb, 8'hff, 23'd0}
    : (za && zb) ? {sa & sb, 31'd0}
    : (za) ? (b)
    : (zb) ? (a)
    : (zero) ? 32'd0
    : (ef >= 12'sd255) ? {s1, 8'hff, 23'd0}
    : {s1, ef[7:0], mf};
endmodule
// The oracle's vectors, (C, B, A) a word: the multiply's and the add's discriminators, then the multiply-add's.
module logos_fpo_vectors(input [9:0] i, output reg [95:0] v);
  always @* case (i)
    10'd0: v = 96'h00000000_00000000_00000000;
    10'd1: v = 96'h00000000_80000000_00000000;
    10'd2: v = 96'h00000000_00000001_00000000;
    10'd3: v = 96'h00000000_80000001_00000000;
    10'd4: v = 96'h00000000_007fffff_00000000;
    10'd5: v = 96'h00000000_807fffff_00000000;
    10'd6: v = 96'h00000000_00800000_00000000;
    10'd7: v = 96'h00000000_80800000_00000000;
    10'd8: v = 96'h00000000_3f800000_00000000;
    10'd9: v = 96'h00000000_bf800000_00000000;
    10'd10: v = 96'h00000000_7f7fffff_00000000;
    10'd11: v = 96'h00000000_ff7fffff_00000000;
    10'd12: v = 96'h00000000_7f800000_00000000;
    10'd13: v = 96'h00000000_ff800000_00000000;
    10'd14: v = 96'h00000000_7fc00000_00000000;
    10'd15: v = 96'h00000000_ffc00001_00000000;
    10'd16: v = 96'h00000000_7f800001_00000000;
    10'd17: v = 96'h00000000_7fbfffff_00000000;
    10'd18: v = 96'h00000000_00400000_00000000;
    10'd19: v = 96'h00000000_34000001_00000000;
    10'd20: v = 96'h00000000_00000000_80000000;
    10'd21: v = 96'h00000000_80000000_80000000;
    10'd22: v = 96'h00000000_00000001_80000000;
    10'd23: v = 96'h00000000_80000001_80000000;
    10'd24: v = 96'h00000000_007fffff_80000000;
    10'd25: v = 96'h00000000_807fffff_80000000;
    10'd26: v = 96'h00000000_00800000_80000000;
    10'd27: v = 96'h00000000_80800000_80000000;
    10'd28: v = 96'h00000000_3f800000_80000000;
    10'd29: v = 96'h00000000_bf800000_80000000;
    10'd30: v = 96'h00000000_7f7fffff_80000000;
    10'd31: v = 96'h00000000_ff7fffff_80000000;
    10'd32: v = 96'h00000000_7f800000_80000000;
    10'd33: v = 96'h00000000_ff800000_80000000;
    10'd34: v = 96'h00000000_7fc00000_80000000;
    10'd35: v = 96'h00000000_ffc00001_80000000;
    10'd36: v = 96'h00000000_7f800001_80000000;
    10'd37: v = 96'h00000000_7fbfffff_80000000;
    10'd38: v = 96'h00000000_00400000_80000000;
    10'd39: v = 96'h00000000_34000001_80000000;
    10'd40: v = 96'h00000000_00000000_00000001;
    10'd41: v = 96'h00000000_80000000_00000001;
    10'd42: v = 96'h00000000_00000001_00000001;
    10'd43: v = 96'h00000000_80000001_00000001;
    10'd44: v = 96'h00000000_007fffff_00000001;
    10'd45: v = 96'h00000000_807fffff_00000001;
    10'd46: v = 96'h00000000_00800000_00000001;
    10'd47: v = 96'h00000000_80800000_00000001;
    10'd48: v = 96'h00000000_3f800000_00000001;
    10'd49: v = 96'h00000000_bf800000_00000001;
    10'd50: v = 96'h00000000_7f7fffff_00000001;
    10'd51: v = 96'h00000000_ff7fffff_00000001;
    10'd52: v = 96'h00000000_7f800000_00000001;
    10'd53: v = 96'h00000000_ff800000_00000001;
    10'd54: v = 96'h00000000_7fc00000_00000001;
    10'd55: v = 96'h00000000_ffc00001_00000001;
    10'd56: v = 96'h00000000_7f800001_00000001;
    10'd57: v = 96'h00000000_7fbfffff_00000001;
    10'd58: v = 96'h00000000_00400000_00000001;
    10'd59: v = 96'h00000000_34000001_00000001;
    10'd60: v = 96'h00000000_00000000_80000001;
    10'd61: v = 96'h00000000_80000000_80000001;
    10'd62: v = 96'h00000000_00000001_80000001;
    10'd63: v = 96'h00000000_80000001_80000001;
    10'd64: v = 96'h00000000_007fffff_80000001;
    10'd65: v = 96'h00000000_807fffff_80000001;
    10'd66: v = 96'h00000000_00800000_80000001;
    10'd67: v = 96'h00000000_80800000_80000001;
    10'd68: v = 96'h00000000_3f800000_80000001;
    10'd69: v = 96'h00000000_bf800000_80000001;
    10'd70: v = 96'h00000000_7f7fffff_80000001;
    10'd71: v = 96'h00000000_ff7fffff_80000001;
    10'd72: v = 96'h00000000_7f800000_80000001;
    10'd73: v = 96'h00000000_ff800000_80000001;
    10'd74: v = 96'h00000000_7fc00000_80000001;
    10'd75: v = 96'h00000000_ffc00001_80000001;
    10'd76: v = 96'h00000000_7f800001_80000001;
    10'd77: v = 96'h00000000_7fbfffff_80000001;
    10'd78: v = 96'h00000000_00400000_80000001;
    10'd79: v = 96'h00000000_34000001_80000001;
    10'd80: v = 96'h00000000_00000000_007fffff;
    10'd81: v = 96'h00000000_80000000_007fffff;
    10'd82: v = 96'h00000000_00000001_007fffff;
    10'd83: v = 96'h00000000_80000001_007fffff;
    10'd84: v = 96'h00000000_007fffff_007fffff;
    10'd85: v = 96'h00000000_807fffff_007fffff;
    10'd86: v = 96'h00000000_00800000_007fffff;
    10'd87: v = 96'h00000000_80800000_007fffff;
    10'd88: v = 96'h00000000_3f800000_007fffff;
    10'd89: v = 96'h00000000_bf800000_007fffff;
    10'd90: v = 96'h00000000_7f7fffff_007fffff;
    10'd91: v = 96'h00000000_ff7fffff_007fffff;
    10'd92: v = 96'h00000000_7f800000_007fffff;
    10'd93: v = 96'h00000000_ff800000_007fffff;
    10'd94: v = 96'h00000000_7fc00000_007fffff;
    10'd95: v = 96'h00000000_ffc00001_007fffff;
    10'd96: v = 96'h00000000_7f800001_007fffff;
    10'd97: v = 96'h00000000_7fbfffff_007fffff;
    10'd98: v = 96'h00000000_00400000_007fffff;
    10'd99: v = 96'h00000000_34000001_007fffff;
    10'd100: v = 96'h00000000_00000000_807fffff;
    10'd101: v = 96'h00000000_80000000_807fffff;
    10'd102: v = 96'h00000000_00000001_807fffff;
    10'd103: v = 96'h00000000_80000001_807fffff;
    10'd104: v = 96'h00000000_007fffff_807fffff;
    10'd105: v = 96'h00000000_807fffff_807fffff;
    10'd106: v = 96'h00000000_00800000_807fffff;
    10'd107: v = 96'h00000000_80800000_807fffff;
    10'd108: v = 96'h00000000_3f800000_807fffff;
    10'd109: v = 96'h00000000_bf800000_807fffff;
    10'd110: v = 96'h00000000_7f7fffff_807fffff;
    10'd111: v = 96'h00000000_ff7fffff_807fffff;
    10'd112: v = 96'h00000000_7f800000_807fffff;
    10'd113: v = 96'h00000000_ff800000_807fffff;
    10'd114: v = 96'h00000000_7fc00000_807fffff;
    10'd115: v = 96'h00000000_ffc00001_807fffff;
    10'd116: v = 96'h00000000_7f800001_807fffff;
    10'd117: v = 96'h00000000_7fbfffff_807fffff;
    10'd118: v = 96'h00000000_00400000_807fffff;
    10'd119: v = 96'h00000000_34000001_807fffff;
    10'd120: v = 96'h00000000_00000000_00800000;
    10'd121: v = 96'h00000000_80000000_00800000;
    10'd122: v = 96'h00000000_00000001_00800000;
    10'd123: v = 96'h00000000_80000001_00800000;
    10'd124: v = 96'h00000000_007fffff_00800000;
    10'd125: v = 96'h00000000_807fffff_00800000;
    10'd126: v = 96'h00000000_00800000_00800000;
    10'd127: v = 96'h00000000_80800000_00800000;
    10'd128: v = 96'h00000000_3f800000_00800000;
    10'd129: v = 96'h00000000_bf800000_00800000;
    10'd130: v = 96'h00000000_7f7fffff_00800000;
    10'd131: v = 96'h00000000_ff7fffff_00800000;
    10'd132: v = 96'h00000000_7f800000_00800000;
    10'd133: v = 96'h00000000_ff800000_00800000;
    10'd134: v = 96'h00000000_7fc00000_00800000;
    10'd135: v = 96'h00000000_ffc00001_00800000;
    10'd136: v = 96'h00000000_7f800001_00800000;
    10'd137: v = 96'h00000000_7fbfffff_00800000;
    10'd138: v = 96'h00000000_00400000_00800000;
    10'd139: v = 96'h00000000_34000001_00800000;
    10'd140: v = 96'h00000000_00000000_80800000;
    10'd141: v = 96'h00000000_80000000_80800000;
    10'd142: v = 96'h00000000_00000001_80800000;
    10'd143: v = 96'h00000000_80000001_80800000;
    10'd144: v = 96'h00000000_007fffff_80800000;
    10'd145: v = 96'h00000000_807fffff_80800000;
    10'd146: v = 96'h00000000_00800000_80800000;
    10'd147: v = 96'h00000000_80800000_80800000;
    10'd148: v = 96'h00000000_3f800000_80800000;
    10'd149: v = 96'h00000000_bf800000_80800000;
    10'd150: v = 96'h00000000_7f7fffff_80800000;
    10'd151: v = 96'h00000000_ff7fffff_80800000;
    10'd152: v = 96'h00000000_7f800000_80800000;
    10'd153: v = 96'h00000000_ff800000_80800000;
    10'd154: v = 96'h00000000_7fc00000_80800000;
    10'd155: v = 96'h00000000_ffc00001_80800000;
    10'd156: v = 96'h00000000_7f800001_80800000;
    10'd157: v = 96'h00000000_7fbfffff_80800000;
    10'd158: v = 96'h00000000_00400000_80800000;
    10'd159: v = 96'h00000000_34000001_80800000;
    10'd160: v = 96'h00000000_00000000_3f800000;
    10'd161: v = 96'h00000000_80000000_3f800000;
    10'd162: v = 96'h00000000_00000001_3f800000;
    10'd163: v = 96'h00000000_80000001_3f800000;
    10'd164: v = 96'h00000000_007fffff_3f800000;
    10'd165: v = 96'h00000000_807fffff_3f800000;
    10'd166: v = 96'h00000000_00800000_3f800000;
    10'd167: v = 96'h00000000_80800000_3f800000;
    10'd168: v = 96'h00000000_3f800000_3f800000;
    10'd169: v = 96'h00000000_bf800000_3f800000;
    10'd170: v = 96'h00000000_7f7fffff_3f800000;
    10'd171: v = 96'h00000000_ff7fffff_3f800000;
    10'd172: v = 96'h00000000_7f800000_3f800000;
    10'd173: v = 96'h00000000_ff800000_3f800000;
    10'd174: v = 96'h00000000_7fc00000_3f800000;
    10'd175: v = 96'h00000000_ffc00001_3f800000;
    10'd176: v = 96'h00000000_7f800001_3f800000;
    10'd177: v = 96'h00000000_7fbfffff_3f800000;
    10'd178: v = 96'h00000000_00400000_3f800000;
    10'd179: v = 96'h00000000_34000001_3f800000;
    10'd180: v = 96'h00000000_00000000_bf800000;
    10'd181: v = 96'h00000000_80000000_bf800000;
    10'd182: v = 96'h00000000_00000001_bf800000;
    10'd183: v = 96'h00000000_80000001_bf800000;
    10'd184: v = 96'h00000000_007fffff_bf800000;
    10'd185: v = 96'h00000000_807fffff_bf800000;
    10'd186: v = 96'h00000000_00800000_bf800000;
    10'd187: v = 96'h00000000_80800000_bf800000;
    10'd188: v = 96'h00000000_3f800000_bf800000;
    10'd189: v = 96'h00000000_bf800000_bf800000;
    10'd190: v = 96'h00000000_7f7fffff_bf800000;
    10'd191: v = 96'h00000000_ff7fffff_bf800000;
    10'd192: v = 96'h00000000_7f800000_bf800000;
    10'd193: v = 96'h00000000_ff800000_bf800000;
    10'd194: v = 96'h00000000_7fc00000_bf800000;
    10'd195: v = 96'h00000000_ffc00001_bf800000;
    10'd196: v = 96'h00000000_7f800001_bf800000;
    10'd197: v = 96'h00000000_7fbfffff_bf800000;
    10'd198: v = 96'h00000000_00400000_bf800000;
    10'd199: v = 96'h00000000_34000001_bf800000;
    10'd200: v = 96'h00000000_00000000_7f7fffff;
    10'd201: v = 96'h00000000_80000000_7f7fffff;
    10'd202: v = 96'h00000000_00000001_7f7fffff;
    10'd203: v = 96'h00000000_80000001_7f7fffff;
    10'd204: v = 96'h00000000_007fffff_7f7fffff;
    10'd205: v = 96'h00000000_807fffff_7f7fffff;
    10'd206: v = 96'h00000000_00800000_7f7fffff;
    10'd207: v = 96'h00000000_80800000_7f7fffff;
    10'd208: v = 96'h00000000_3f800000_7f7fffff;
    10'd209: v = 96'h00000000_bf800000_7f7fffff;
    10'd210: v = 96'h00000000_7f7fffff_7f7fffff;
    10'd211: v = 96'h00000000_ff7fffff_7f7fffff;
    10'd212: v = 96'h00000000_7f800000_7f7fffff;
    10'd213: v = 96'h00000000_ff800000_7f7fffff;
    10'd214: v = 96'h00000000_7fc00000_7f7fffff;
    10'd215: v = 96'h00000000_ffc00001_7f7fffff;
    10'd216: v = 96'h00000000_7f800001_7f7fffff;
    10'd217: v = 96'h00000000_7fbfffff_7f7fffff;
    10'd218: v = 96'h00000000_00400000_7f7fffff;
    10'd219: v = 96'h00000000_34000001_7f7fffff;
    10'd220: v = 96'h00000000_00000000_ff7fffff;
    10'd221: v = 96'h00000000_80000000_ff7fffff;
    10'd222: v = 96'h00000000_00000001_ff7fffff;
    10'd223: v = 96'h00000000_80000001_ff7fffff;
    10'd224: v = 96'h00000000_007fffff_ff7fffff;
    10'd225: v = 96'h00000000_807fffff_ff7fffff;
    10'd226: v = 96'h00000000_00800000_ff7fffff;
    10'd227: v = 96'h00000000_80800000_ff7fffff;
    10'd228: v = 96'h00000000_3f800000_ff7fffff;
    10'd229: v = 96'h00000000_bf800000_ff7fffff;
    10'd230: v = 96'h00000000_7f7fffff_ff7fffff;
    10'd231: v = 96'h00000000_ff7fffff_ff7fffff;
    10'd232: v = 96'h00000000_7f800000_ff7fffff;
    10'd233: v = 96'h00000000_ff800000_ff7fffff;
    10'd234: v = 96'h00000000_7fc00000_ff7fffff;
    10'd235: v = 96'h00000000_ffc00001_ff7fffff;
    10'd236: v = 96'h00000000_7f800001_ff7fffff;
    10'd237: v = 96'h00000000_7fbfffff_ff7fffff;
    10'd238: v = 96'h00000000_00400000_ff7fffff;
    10'd239: v = 96'h00000000_34000001_ff7fffff;
    10'd240: v = 96'h00000000_00000000_7f800000;
    10'd241: v = 96'h00000000_80000000_7f800000;
    10'd242: v = 96'h00000000_00000001_7f800000;
    10'd243: v = 96'h00000000_80000001_7f800000;
    10'd244: v = 96'h00000000_007fffff_7f800000;
    10'd245: v = 96'h00000000_807fffff_7f800000;
    10'd246: v = 96'h00000000_00800000_7f800000;
    10'd247: v = 96'h00000000_80800000_7f800000;
    10'd248: v = 96'h00000000_3f800000_7f800000;
    10'd249: v = 96'h00000000_bf800000_7f800000;
    10'd250: v = 96'h00000000_7f7fffff_7f800000;
    10'd251: v = 96'h00000000_ff7fffff_7f800000;
    10'd252: v = 96'h00000000_7f800000_7f800000;
    10'd253: v = 96'h00000000_ff800000_7f800000;
    10'd254: v = 96'h00000000_7fc00000_7f800000;
    10'd255: v = 96'h00000000_ffc00001_7f800000;
    10'd256: v = 96'h00000000_7f800001_7f800000;
    10'd257: v = 96'h00000000_7fbfffff_7f800000;
    10'd258: v = 96'h00000000_00400000_7f800000;
    10'd259: v = 96'h00000000_34000001_7f800000;
    10'd260: v = 96'h00000000_00000000_ff800000;
    10'd261: v = 96'h00000000_80000000_ff800000;
    10'd262: v = 96'h00000000_00000001_ff800000;
    10'd263: v = 96'h00000000_80000001_ff800000;
    10'd264: v = 96'h00000000_007fffff_ff800000;
    10'd265: v = 96'h00000000_807fffff_ff800000;
    10'd266: v = 96'h00000000_00800000_ff800000;
    10'd267: v = 96'h00000000_80800000_ff800000;
    10'd268: v = 96'h00000000_3f800000_ff800000;
    10'd269: v = 96'h00000000_bf800000_ff800000;
    10'd270: v = 96'h00000000_7f7fffff_ff800000;
    10'd271: v = 96'h00000000_ff7fffff_ff800000;
    10'd272: v = 96'h00000000_7f800000_ff800000;
    10'd273: v = 96'h00000000_ff800000_ff800000;
    10'd274: v = 96'h00000000_7fc00000_ff800000;
    10'd275: v = 96'h00000000_ffc00001_ff800000;
    10'd276: v = 96'h00000000_7f800001_ff800000;
    10'd277: v = 96'h00000000_7fbfffff_ff800000;
    10'd278: v = 96'h00000000_00400000_ff800000;
    10'd279: v = 96'h00000000_34000001_ff800000;
    10'd280: v = 96'h00000000_00000000_7fc00000;
    10'd281: v = 96'h00000000_80000000_7fc00000;
    10'd282: v = 96'h00000000_00000001_7fc00000;
    10'd283: v = 96'h00000000_80000001_7fc00000;
    10'd284: v = 96'h00000000_007fffff_7fc00000;
    10'd285: v = 96'h00000000_807fffff_7fc00000;
    10'd286: v = 96'h00000000_00800000_7fc00000;
    10'd287: v = 96'h00000000_80800000_7fc00000;
    10'd288: v = 96'h00000000_3f800000_7fc00000;
    10'd289: v = 96'h00000000_bf800000_7fc00000;
    10'd290: v = 96'h00000000_7f7fffff_7fc00000;
    10'd291: v = 96'h00000000_ff7fffff_7fc00000;
    10'd292: v = 96'h00000000_7f800000_7fc00000;
    10'd293: v = 96'h00000000_ff800000_7fc00000;
    10'd294: v = 96'h00000000_7fc00000_7fc00000;
    10'd295: v = 96'h00000000_ffc00001_7fc00000;
    10'd296: v = 96'h00000000_7f800001_7fc00000;
    10'd297: v = 96'h00000000_7fbfffff_7fc00000;
    10'd298: v = 96'h00000000_00400000_7fc00000;
    10'd299: v = 96'h00000000_34000001_7fc00000;
    10'd300: v = 96'h00000000_00000000_ffc00001;
    10'd301: v = 96'h00000000_80000000_ffc00001;
    10'd302: v = 96'h00000000_00000001_ffc00001;
    10'd303: v = 96'h00000000_80000001_ffc00001;
    10'd304: v = 96'h00000000_007fffff_ffc00001;
    10'd305: v = 96'h00000000_807fffff_ffc00001;
    10'd306: v = 96'h00000000_00800000_ffc00001;
    10'd307: v = 96'h00000000_80800000_ffc00001;
    10'd308: v = 96'h00000000_3f800000_ffc00001;
    10'd309: v = 96'h00000000_bf800000_ffc00001;
    10'd310: v = 96'h00000000_7f7fffff_ffc00001;
    10'd311: v = 96'h00000000_ff7fffff_ffc00001;
    10'd312: v = 96'h00000000_7f800000_ffc00001;
    10'd313: v = 96'h00000000_ff800000_ffc00001;
    10'd314: v = 96'h00000000_7fc00000_ffc00001;
    10'd315: v = 96'h00000000_ffc00001_ffc00001;
    10'd316: v = 96'h00000000_7f800001_ffc00001;
    10'd317: v = 96'h00000000_7fbfffff_ffc00001;
    10'd318: v = 96'h00000000_00400000_ffc00001;
    10'd319: v = 96'h00000000_34000001_ffc00001;
    10'd320: v = 96'h00000000_00000000_7f800001;
    10'd321: v = 96'h00000000_80000000_7f800001;
    10'd322: v = 96'h00000000_00000001_7f800001;
    10'd323: v = 96'h00000000_80000001_7f800001;
    10'd324: v = 96'h00000000_007fffff_7f800001;
    10'd325: v = 96'h00000000_807fffff_7f800001;
    10'd326: v = 96'h00000000_00800000_7f800001;
    10'd327: v = 96'h00000000_80800000_7f800001;
    10'd328: v = 96'h00000000_3f800000_7f800001;
    10'd329: v = 96'h00000000_bf800000_7f800001;
    10'd330: v = 96'h00000000_7f7fffff_7f800001;
    10'd331: v = 96'h00000000_ff7fffff_7f800001;
    10'd332: v = 96'h00000000_7f800000_7f800001;
    10'd333: v = 96'h00000000_ff800000_7f800001;
    10'd334: v = 96'h00000000_7fc00000_7f800001;
    10'd335: v = 96'h00000000_ffc00001_7f800001;
    10'd336: v = 96'h00000000_7f800001_7f800001;
    10'd337: v = 96'h00000000_7fbfffff_7f800001;
    10'd338: v = 96'h00000000_00400000_7f800001;
    10'd339: v = 96'h00000000_34000001_7f800001;
    10'd340: v = 96'h00000000_00000000_7fbfffff;
    10'd341: v = 96'h00000000_80000000_7fbfffff;
    10'd342: v = 96'h00000000_00000001_7fbfffff;
    10'd343: v = 96'h00000000_80000001_7fbfffff;
    10'd344: v = 96'h00000000_007fffff_7fbfffff;
    10'd345: v = 96'h00000000_807fffff_7fbfffff;
    10'd346: v = 96'h00000000_00800000_7fbfffff;
    10'd347: v = 96'h00000000_80800000_7fbfffff;
    10'd348: v = 96'h00000000_3f800000_7fbfffff;
    10'd349: v = 96'h00000000_bf800000_7fbfffff;
    10'd350: v = 96'h00000000_7f7fffff_7fbfffff;
    10'd351: v = 96'h00000000_ff7fffff_7fbfffff;
    10'd352: v = 96'h00000000_7f800000_7fbfffff;
    10'd353: v = 96'h00000000_ff800000_7fbfffff;
    10'd354: v = 96'h00000000_7fc00000_7fbfffff;
    10'd355: v = 96'h00000000_ffc00001_7fbfffff;
    10'd356: v = 96'h00000000_7f800001_7fbfffff;
    10'd357: v = 96'h00000000_7fbfffff_7fbfffff;
    10'd358: v = 96'h00000000_00400000_7fbfffff;
    10'd359: v = 96'h00000000_34000001_7fbfffff;
    10'd360: v = 96'h00000000_00000000_00400000;
    10'd361: v = 96'h00000000_80000000_00400000;
    10'd362: v = 96'h00000000_00000001_00400000;
    10'd363: v = 96'h00000000_80000001_00400000;
    10'd364: v = 96'h00000000_007fffff_00400000;
    10'd365: v = 96'h00000000_807fffff_00400000;
    10'd366: v = 96'h00000000_00800000_00400000;
    10'd367: v = 96'h00000000_80800000_00400000;
    10'd368: v = 96'h00000000_3f800000_00400000;
    10'd369: v = 96'h00000000_bf800000_00400000;
    10'd370: v = 96'h00000000_7f7fffff_00400000;
    10'd371: v = 96'h00000000_ff7fffff_00400000;
    10'd372: v = 96'h00000000_7f800000_00400000;
    10'd373: v = 96'h00000000_ff800000_00400000;
    10'd374: v = 96'h00000000_7fc00000_00400000;
    10'd375: v = 96'h00000000_ffc00001_00400000;
    10'd376: v = 96'h00000000_7f800001_00400000;
    10'd377: v = 96'h00000000_7fbfffff_00400000;
    10'd378: v = 96'h00000000_00400000_00400000;
    10'd379: v = 96'h00000000_34000001_00400000;
    10'd380: v = 96'h00000000_00000000_34000001;
    10'd381: v = 96'h00000000_80000000_34000001;
    10'd382: v = 96'h00000000_00000001_34000001;
    10'd383: v = 96'h00000000_80000001_34000001;
    10'd384: v = 96'h00000000_007fffff_34000001;
    10'd385: v = 96'h00000000_807fffff_34000001;
    10'd386: v = 96'h00000000_00800000_34000001;
    10'd387: v = 96'h00000000_80800000_34000001;
    10'd388: v = 96'h00000000_3f800000_34000001;
    10'd389: v = 96'h00000000_bf800000_34000001;
    10'd390: v = 96'h00000000_7f7fffff_34000001;
    10'd391: v = 96'h00000000_ff7fffff_34000001;
    10'd392: v = 96'h00000000_7f800000_34000001;
    10'd393: v = 96'h00000000_ff800000_34000001;
    10'd394: v = 96'h00000000_7fc00000_34000001;
    10'd395: v = 96'h00000000_ffc00001_34000001;
    10'd396: v = 96'h00000000_7f800001_34000001;
    10'd397: v = 96'h00000000_7fbfffff_34000001;
    10'd398: v = 96'h00000000_00400000_34000001;
    10'd399: v = 96'h00000000_34000001_34000001;
    10'd400: v = 96'h00000000_336aeefe_0c8b7a4c;
    10'd401: v = 96'h00000000_1cbf02bb_a32b8cf6;
    10'd402: v = 96'h00000000_00000000_00000000;
    10'd403: v = 96'h00000000_80000000_00000000;
    10'd404: v = 96'h00000000_00000001_00000000;
    10'd405: v = 96'h00000000_80000001_00000000;
    10'd406: v = 96'h00000000_007fffff_00000000;
    10'd407: v = 96'h00000000_807fffff_00000000;
    10'd408: v = 96'h00000000_00800000_00000000;
    10'd409: v = 96'h00000000_80800000_00000000;
    10'd410: v = 96'h00000000_3f800000_00000000;
    10'd411: v = 96'h00000000_bf800000_00000000;
    10'd412: v = 96'h00000000_7f7fffff_00000000;
    10'd413: v = 96'h00000000_ff7fffff_00000000;
    10'd414: v = 96'h00000000_7f800000_00000000;
    10'd415: v = 96'h00000000_ff800000_00000000;
    10'd416: v = 96'h00000000_7fc00000_00000000;
    10'd417: v = 96'h00000000_ffc00001_00000000;
    10'd418: v = 96'h00000000_7f800001_00000000;
    10'd419: v = 96'h00000000_7fbfffff_00000000;
    10'd420: v = 96'h00000000_00400000_00000000;
    10'd421: v = 96'h00000000_34000001_00000000;
    10'd422: v = 96'h00000000_00000000_80000000;
    10'd423: v = 96'h00000000_80000000_80000000;
    10'd424: v = 96'h00000000_00000001_80000000;
    10'd425: v = 96'h00000000_80000001_80000000;
    10'd426: v = 96'h00000000_007fffff_80000000;
    10'd427: v = 96'h00000000_807fffff_80000000;
    10'd428: v = 96'h00000000_00800000_80000000;
    10'd429: v = 96'h00000000_80800000_80000000;
    10'd430: v = 96'h00000000_3f800000_80000000;
    10'd431: v = 96'h00000000_bf800000_80000000;
    10'd432: v = 96'h00000000_7f7fffff_80000000;
    10'd433: v = 96'h00000000_ff7fffff_80000000;
    10'd434: v = 96'h00000000_7f800000_80000000;
    10'd435: v = 96'h00000000_ff800000_80000000;
    10'd436: v = 96'h00000000_7fc00000_80000000;
    10'd437: v = 96'h00000000_ffc00001_80000000;
    10'd438: v = 96'h00000000_7f800001_80000000;
    10'd439: v = 96'h00000000_7fbfffff_80000000;
    10'd440: v = 96'h00000000_00400000_80000000;
    10'd441: v = 96'h00000000_34000001_80000000;
    10'd442: v = 96'h00000000_00000000_00000001;
    10'd443: v = 96'h00000000_80000000_00000001;
    10'd444: v = 96'h00000000_00000001_00000001;
    10'd445: v = 96'h00000000_80000001_00000001;
    10'd446: v = 96'h00000000_007fffff_00000001;
    10'd447: v = 96'h00000000_807fffff_00000001;
    10'd448: v = 96'h00000000_00800000_00000001;
    10'd449: v = 96'h00000000_80800000_00000001;
    10'd450: v = 96'h00000000_3f800000_00000001;
    10'd451: v = 96'h00000000_bf800000_00000001;
    10'd452: v = 96'h00000000_7f7fffff_00000001;
    10'd453: v = 96'h00000000_ff7fffff_00000001;
    10'd454: v = 96'h00000000_7f800000_00000001;
    10'd455: v = 96'h00000000_ff800000_00000001;
    10'd456: v = 96'h00000000_7fc00000_00000001;
    10'd457: v = 96'h00000000_ffc00001_00000001;
    10'd458: v = 96'h00000000_7f800001_00000001;
    10'd459: v = 96'h00000000_7fbfffff_00000001;
    10'd460: v = 96'h00000000_00400000_00000001;
    10'd461: v = 96'h00000000_34000001_00000001;
    10'd462: v = 96'h00000000_00000000_80000001;
    10'd463: v = 96'h00000000_80000000_80000001;
    10'd464: v = 96'h00000000_00000001_80000001;
    10'd465: v = 96'h00000000_80000001_80000001;
    10'd466: v = 96'h00000000_007fffff_80000001;
    10'd467: v = 96'h00000000_807fffff_80000001;
    10'd468: v = 96'h00000000_00800000_80000001;
    10'd469: v = 96'h00000000_80800000_80000001;
    10'd470: v = 96'h00000000_3f800000_80000001;
    10'd471: v = 96'h00000000_bf800000_80000001;
    10'd472: v = 96'h00000000_7f7fffff_80000001;
    10'd473: v = 96'h00000000_ff7fffff_80000001;
    10'd474: v = 96'h00000000_7f800000_80000001;
    10'd475: v = 96'h00000000_ff800000_80000001;
    10'd476: v = 96'h00000000_7fc00000_80000001;
    10'd477: v = 96'h00000000_ffc00001_80000001;
    10'd478: v = 96'h00000000_7f800001_80000001;
    10'd479: v = 96'h00000000_7fbfffff_80000001;
    10'd480: v = 96'h00000000_00400000_80000001;
    10'd481: v = 96'h00000000_34000001_80000001;
    10'd482: v = 96'h00000000_00000000_007fffff;
    10'd483: v = 96'h00000000_80000000_007fffff;
    10'd484: v = 96'h00000000_00000001_007fffff;
    10'd485: v = 96'h00000000_80000001_007fffff;
    10'd486: v = 96'h00000000_007fffff_007fffff;
    10'd487: v = 96'h00000000_807fffff_007fffff;
    10'd488: v = 96'h00000000_00800000_007fffff;
    10'd489: v = 96'h00000000_80800000_007fffff;
    10'd490: v = 96'h00000000_3f800000_007fffff;
    10'd491: v = 96'h00000000_bf800000_007fffff;
    10'd492: v = 96'h00000000_7f7fffff_007fffff;
    10'd493: v = 96'h00000000_ff7fffff_007fffff;
    10'd494: v = 96'h00000000_7f800000_007fffff;
    10'd495: v = 96'h00000000_ff800000_007fffff;
    10'd496: v = 96'h00000000_7fc00000_007fffff;
    10'd497: v = 96'h00000000_ffc00001_007fffff;
    10'd498: v = 96'h00000000_7f800001_007fffff;
    10'd499: v = 96'h00000000_7fbfffff_007fffff;
    10'd500: v = 96'h00000000_00400000_007fffff;
    10'd501: v = 96'h00000000_34000001_007fffff;
    10'd502: v = 96'h00000000_00000000_807fffff;
    10'd503: v = 96'h00000000_80000000_807fffff;
    10'd504: v = 96'h00000000_00000001_807fffff;
    10'd505: v = 96'h00000000_80000001_807fffff;
    10'd506: v = 96'h00000000_007fffff_807fffff;
    10'd507: v = 96'h00000000_807fffff_807fffff;
    10'd508: v = 96'h00000000_00800000_807fffff;
    10'd509: v = 96'h00000000_80800000_807fffff;
    10'd510: v = 96'h00000000_3f800000_807fffff;
    10'd511: v = 96'h00000000_bf800000_807fffff;
    10'd512: v = 96'h00000000_7f7fffff_807fffff;
    10'd513: v = 96'h00000000_ff7fffff_807fffff;
    10'd514: v = 96'h00000000_7f800000_807fffff;
    10'd515: v = 96'h00000000_ff800000_807fffff;
    10'd516: v = 96'h00000000_7fc00000_807fffff;
    10'd517: v = 96'h00000000_ffc00001_807fffff;
    10'd518: v = 96'h00000000_7f800001_807fffff;
    10'd519: v = 96'h00000000_7fbfffff_807fffff;
    10'd520: v = 96'h00000000_00400000_807fffff;
    10'd521: v = 96'h00000000_34000001_807fffff;
    10'd522: v = 96'h00000000_00000000_00800000;
    10'd523: v = 96'h00000000_80000000_00800000;
    10'd524: v = 96'h00000000_00000001_00800000;
    10'd525: v = 96'h00000000_80000001_00800000;
    10'd526: v = 96'h00000000_007fffff_00800000;
    10'd527: v = 96'h00000000_807fffff_00800000;
    10'd528: v = 96'h00000000_00800000_00800000;
    10'd529: v = 96'h00000000_80800000_00800000;
    10'd530: v = 96'h00000000_3f800000_00800000;
    10'd531: v = 96'h00000000_bf800000_00800000;
    10'd532: v = 96'h00000000_7f7fffff_00800000;
    10'd533: v = 96'h00000000_ff7fffff_00800000;
    10'd534: v = 96'h00000000_7f800000_00800000;
    10'd535: v = 96'h00000000_ff800000_00800000;
    10'd536: v = 96'h00000000_7fc00000_00800000;
    10'd537: v = 96'h00000000_ffc00001_00800000;
    10'd538: v = 96'h00000000_7f800001_00800000;
    10'd539: v = 96'h00000000_7fbfffff_00800000;
    10'd540: v = 96'h00000000_00400000_00800000;
    10'd541: v = 96'h00000000_34000001_00800000;
    10'd542: v = 96'h00000000_00000000_80800000;
    10'd543: v = 96'h00000000_80000000_80800000;
    10'd544: v = 96'h00000000_00000001_80800000;
    10'd545: v = 96'h00000000_80000001_80800000;
    10'd546: v = 96'h00000000_007fffff_80800000;
    10'd547: v = 96'h00000000_807fffff_80800000;
    10'd548: v = 96'h00000000_00800000_80800000;
    10'd549: v = 96'h00000000_80800000_80800000;
    10'd550: v = 96'h00000000_3f800000_80800000;
    10'd551: v = 96'h00000000_bf800000_80800000;
    10'd552: v = 96'h00000000_7f7fffff_80800000;
    10'd553: v = 96'h00000000_ff7fffff_80800000;
    10'd554: v = 96'h00000000_7f800000_80800000;
    10'd555: v = 96'h00000000_ff800000_80800000;
    10'd556: v = 96'h00000000_7fc00000_80800000;
    10'd557: v = 96'h00000000_ffc00001_80800000;
    10'd558: v = 96'h00000000_7f800001_80800000;
    10'd559: v = 96'h00000000_7fbfffff_80800000;
    10'd560: v = 96'h00000000_00400000_80800000;
    10'd561: v = 96'h00000000_34000001_80800000;
    10'd562: v = 96'h00000000_00000000_3f800000;
    10'd563: v = 96'h00000000_80000000_3f800000;
    10'd564: v = 96'h00000000_00000001_3f800000;
    10'd565: v = 96'h00000000_80000001_3f800000;
    10'd566: v = 96'h00000000_007fffff_3f800000;
    10'd567: v = 96'h00000000_807fffff_3f800000;
    10'd568: v = 96'h00000000_00800000_3f800000;
    10'd569: v = 96'h00000000_80800000_3f800000;
    10'd570: v = 96'h00000000_3f800000_3f800000;
    10'd571: v = 96'h00000000_bf800000_3f800000;
    10'd572: v = 96'h00000000_7f7fffff_3f800000;
    10'd573: v = 96'h00000000_ff7fffff_3f800000;
    10'd574: v = 96'h00000000_7f800000_3f800000;
    10'd575: v = 96'h00000000_ff800000_3f800000;
    10'd576: v = 96'h00000000_7fc00000_3f800000;
    10'd577: v = 96'h00000000_ffc00001_3f800000;
    10'd578: v = 96'h00000000_7f800001_3f800000;
    10'd579: v = 96'h00000000_7fbfffff_3f800000;
    10'd580: v = 96'h00000000_00400000_3f800000;
    10'd581: v = 96'h00000000_34000001_3f800000;
    10'd582: v = 96'h00000000_00000000_bf800000;
    10'd583: v = 96'h00000000_80000000_bf800000;
    10'd584: v = 96'h00000000_00000001_bf800000;
    10'd585: v = 96'h00000000_80000001_bf800000;
    10'd586: v = 96'h00000000_007fffff_bf800000;
    10'd587: v = 96'h00000000_807fffff_bf800000;
    10'd588: v = 96'h00000000_00800000_bf800000;
    10'd589: v = 96'h00000000_80800000_bf800000;
    10'd590: v = 96'h00000000_3f800000_bf800000;
    10'd591: v = 96'h00000000_bf800000_bf800000;
    10'd592: v = 96'h00000000_7f7fffff_bf800000;
    10'd593: v = 96'h00000000_ff7fffff_bf800000;
    10'd594: v = 96'h00000000_7f800000_bf800000;
    10'd595: v = 96'h00000000_ff800000_bf800000;
    10'd596: v = 96'h00000000_7fc00000_bf800000;
    10'd597: v = 96'h00000000_ffc00001_bf800000;
    10'd598: v = 96'h00000000_7f800001_bf800000;
    10'd599: v = 96'h00000000_7fbfffff_bf800000;
    10'd600: v = 96'h00000000_00400000_bf800000;
    10'd601: v = 96'h00000000_34000001_bf800000;
    10'd602: v = 96'h00000000_00000000_7f7fffff;
    10'd603: v = 96'h00000000_80000000_7f7fffff;
    10'd604: v = 96'h00000000_00000001_7f7fffff;
    10'd605: v = 96'h00000000_80000001_7f7fffff;
    10'd606: v = 96'h00000000_007fffff_7f7fffff;
    10'd607: v = 96'h00000000_807fffff_7f7fffff;
    10'd608: v = 96'h00000000_00800000_7f7fffff;
    10'd609: v = 96'h00000000_80800000_7f7fffff;
    10'd610: v = 96'h00000000_3f800000_7f7fffff;
    10'd611: v = 96'h00000000_bf800000_7f7fffff;
    10'd612: v = 96'h00000000_7f7fffff_7f7fffff;
    10'd613: v = 96'h00000000_ff7fffff_7f7fffff;
    10'd614: v = 96'h00000000_7f800000_7f7fffff;
    10'd615: v = 96'h00000000_ff800000_7f7fffff;
    10'd616: v = 96'h00000000_7fc00000_7f7fffff;
    10'd617: v = 96'h00000000_ffc00001_7f7fffff;
    10'd618: v = 96'h00000000_7f800001_7f7fffff;
    10'd619: v = 96'h00000000_7fbfffff_7f7fffff;
    10'd620: v = 96'h00000000_00400000_7f7fffff;
    10'd621: v = 96'h00000000_34000001_7f7fffff;
    10'd622: v = 96'h00000000_00000000_ff7fffff;
    10'd623: v = 96'h00000000_80000000_ff7fffff;
    10'd624: v = 96'h00000000_00000001_ff7fffff;
    10'd625: v = 96'h00000000_80000001_ff7fffff;
    10'd626: v = 96'h00000000_007fffff_ff7fffff;
    10'd627: v = 96'h00000000_807fffff_ff7fffff;
    10'd628: v = 96'h00000000_00800000_ff7fffff;
    10'd629: v = 96'h00000000_80800000_ff7fffff;
    10'd630: v = 96'h00000000_3f800000_ff7fffff;
    10'd631: v = 96'h00000000_bf800000_ff7fffff;
    10'd632: v = 96'h00000000_7f7fffff_ff7fffff;
    10'd633: v = 96'h00000000_ff7fffff_ff7fffff;
    10'd634: v = 96'h00000000_7f800000_ff7fffff;
    10'd635: v = 96'h00000000_ff800000_ff7fffff;
    10'd636: v = 96'h00000000_7fc00000_ff7fffff;
    10'd637: v = 96'h00000000_ffc00001_ff7fffff;
    10'd638: v = 96'h00000000_7f800001_ff7fffff;
    10'd639: v = 96'h00000000_7fbfffff_ff7fffff;
    10'd640: v = 96'h00000000_00400000_ff7fffff;
    10'd641: v = 96'h00000000_34000001_ff7fffff;
    10'd642: v = 96'h00000000_00000000_7f800000;
    10'd643: v = 96'h00000000_80000000_7f800000;
    10'd644: v = 96'h00000000_00000001_7f800000;
    10'd645: v = 96'h00000000_80000001_7f800000;
    10'd646: v = 96'h00000000_007fffff_7f800000;
    10'd647: v = 96'h00000000_807fffff_7f800000;
    10'd648: v = 96'h00000000_00800000_7f800000;
    10'd649: v = 96'h00000000_80800000_7f800000;
    10'd650: v = 96'h00000000_3f800000_7f800000;
    10'd651: v = 96'h00000000_bf800000_7f800000;
    10'd652: v = 96'h00000000_7f7fffff_7f800000;
    10'd653: v = 96'h00000000_ff7fffff_7f800000;
    10'd654: v = 96'h00000000_7f800000_7f800000;
    10'd655: v = 96'h00000000_ff800000_7f800000;
    10'd656: v = 96'h00000000_7fc00000_7f800000;
    10'd657: v = 96'h00000000_ffc00001_7f800000;
    10'd658: v = 96'h00000000_7f800001_7f800000;
    10'd659: v = 96'h00000000_7fbfffff_7f800000;
    10'd660: v = 96'h00000000_00400000_7f800000;
    10'd661: v = 96'h00000000_34000001_7f800000;
    10'd662: v = 96'h00000000_00000000_ff800000;
    10'd663: v = 96'h00000000_80000000_ff800000;
    10'd664: v = 96'h00000000_00000001_ff800000;
    10'd665: v = 96'h00000000_80000001_ff800000;
    10'd666: v = 96'h00000000_007fffff_ff800000;
    10'd667: v = 96'h00000000_807fffff_ff800000;
    10'd668: v = 96'h00000000_00800000_ff800000;
    10'd669: v = 96'h00000000_80800000_ff800000;
    10'd670: v = 96'h00000000_3f800000_ff800000;
    10'd671: v = 96'h00000000_bf800000_ff800000;
    10'd672: v = 96'h00000000_7f7fffff_ff800000;
    10'd673: v = 96'h00000000_ff7fffff_ff800000;
    10'd674: v = 96'h00000000_7f800000_ff800000;
    10'd675: v = 96'h00000000_ff800000_ff800000;
    10'd676: v = 96'h00000000_7fc00000_ff800000;
    10'd677: v = 96'h00000000_ffc00001_ff800000;
    10'd678: v = 96'h00000000_7f800001_ff800000;
    10'd679: v = 96'h00000000_7fbfffff_ff800000;
    10'd680: v = 96'h00000000_00400000_ff800000;
    10'd681: v = 96'h00000000_34000001_ff800000;
    10'd682: v = 96'h00000000_00000000_7fc00000;
    10'd683: v = 96'h00000000_80000000_7fc00000;
    10'd684: v = 96'h00000000_00000001_7fc00000;
    10'd685: v = 96'h00000000_80000001_7fc00000;
    10'd686: v = 96'h00000000_007fffff_7fc00000;
    10'd687: v = 96'h00000000_807fffff_7fc00000;
    10'd688: v = 96'h00000000_00800000_7fc00000;
    10'd689: v = 96'h00000000_80800000_7fc00000;
    10'd690: v = 96'h00000000_3f800000_7fc00000;
    10'd691: v = 96'h00000000_bf800000_7fc00000;
    10'd692: v = 96'h00000000_7f7fffff_7fc00000;
    10'd693: v = 96'h00000000_ff7fffff_7fc00000;
    10'd694: v = 96'h00000000_7f800000_7fc00000;
    10'd695: v = 96'h00000000_ff800000_7fc00000;
    10'd696: v = 96'h00000000_7fc00000_7fc00000;
    10'd697: v = 96'h00000000_ffc00001_7fc00000;
    10'd698: v = 96'h00000000_7f800001_7fc00000;
    10'd699: v = 96'h00000000_7fbfffff_7fc00000;
    10'd700: v = 96'h00000000_00400000_7fc00000;
    10'd701: v = 96'h00000000_34000001_7fc00000;
    10'd702: v = 96'h00000000_00000000_ffc00001;
    10'd703: v = 96'h00000000_80000000_ffc00001;
    10'd704: v = 96'h00000000_00000001_ffc00001;
    10'd705: v = 96'h00000000_80000001_ffc00001;
    10'd706: v = 96'h00000000_007fffff_ffc00001;
    10'd707: v = 96'h00000000_807fffff_ffc00001;
    10'd708: v = 96'h00000000_00800000_ffc00001;
    10'd709: v = 96'h00000000_80800000_ffc00001;
    10'd710: v = 96'h00000000_3f800000_ffc00001;
    10'd711: v = 96'h00000000_bf800000_ffc00001;
    10'd712: v = 96'h00000000_7f7fffff_ffc00001;
    10'd713: v = 96'h00000000_ff7fffff_ffc00001;
    10'd714: v = 96'h00000000_7f800000_ffc00001;
    10'd715: v = 96'h00000000_ff800000_ffc00001;
    10'd716: v = 96'h00000000_7fc00000_ffc00001;
    10'd717: v = 96'h00000000_ffc00001_ffc00001;
    10'd718: v = 96'h00000000_7f800001_ffc00001;
    10'd719: v = 96'h00000000_7fbfffff_ffc00001;
    10'd720: v = 96'h00000000_00400000_ffc00001;
    10'd721: v = 96'h00000000_34000001_ffc00001;
    10'd722: v = 96'h00000000_00000000_7f800001;
    10'd723: v = 96'h00000000_80000000_7f800001;
    10'd724: v = 96'h00000000_00000001_7f800001;
    10'd725: v = 96'h00000000_80000001_7f800001;
    10'd726: v = 96'h00000000_007fffff_7f800001;
    10'd727: v = 96'h00000000_807fffff_7f800001;
    10'd728: v = 96'h00000000_00800000_7f800001;
    10'd729: v = 96'h00000000_80800000_7f800001;
    10'd730: v = 96'h00000000_3f800000_7f800001;
    10'd731: v = 96'h00000000_bf800000_7f800001;
    10'd732: v = 96'h00000000_7f7fffff_7f800001;
    10'd733: v = 96'h00000000_ff7fffff_7f800001;
    10'd734: v = 96'h00000000_7f800000_7f800001;
    10'd735: v = 96'h00000000_ff800000_7f800001;
    10'd736: v = 96'h00000000_7fc00000_7f800001;
    10'd737: v = 96'h00000000_ffc00001_7f800001;
    10'd738: v = 96'h00000000_7f800001_7f800001;
    10'd739: v = 96'h00000000_7fbfffff_7f800001;
    10'd740: v = 96'h00000000_00400000_7f800001;
    10'd741: v = 96'h00000000_34000001_7f800001;
    10'd742: v = 96'h00000000_00000000_7fbfffff;
    10'd743: v = 96'h00000000_80000000_7fbfffff;
    10'd744: v = 96'h00000000_00000001_7fbfffff;
    10'd745: v = 96'h00000000_80000001_7fbfffff;
    10'd746: v = 96'h00000000_007fffff_7fbfffff;
    10'd747: v = 96'h00000000_807fffff_7fbfffff;
    10'd748: v = 96'h00000000_00800000_7fbfffff;
    10'd749: v = 96'h00000000_80800000_7fbfffff;
    10'd750: v = 96'h00000000_3f800000_7fbfffff;
    10'd751: v = 96'h00000000_bf800000_7fbfffff;
    10'd752: v = 96'h00000000_7f7fffff_7fbfffff;
    10'd753: v = 96'h00000000_ff7fffff_7fbfffff;
    10'd754: v = 96'h00000000_7f800000_7fbfffff;
    10'd755: v = 96'h00000000_ff800000_7fbfffff;
    10'd756: v = 96'h00000000_7fc00000_7fbfffff;
    10'd757: v = 96'h00000000_ffc00001_7fbfffff;
    10'd758: v = 96'h00000000_7f800001_7fbfffff;
    10'd759: v = 96'h00000000_7fbfffff_7fbfffff;
    10'd760: v = 96'h00000000_00400000_7fbfffff;
    10'd761: v = 96'h00000000_34000001_7fbfffff;
    10'd762: v = 96'h00000000_00000000_00400000;
    10'd763: v = 96'h00000000_80000000_00400000;
    10'd764: v = 96'h00000000_00000001_00400000;
    10'd765: v = 96'h00000000_80000001_00400000;
    10'd766: v = 96'h00000000_007fffff_00400000;
    10'd767: v = 96'h00000000_807fffff_00400000;
    10'd768: v = 96'h00000000_00800000_00400000;
    10'd769: v = 96'h00000000_80800000_00400000;
    10'd770: v = 96'h00000000_3f800000_00400000;
    10'd771: v = 96'h00000000_bf800000_00400000;
    10'd772: v = 96'h00000000_7f7fffff_00400000;
    10'd773: v = 96'h00000000_ff7fffff_00400000;
    10'd774: v = 96'h00000000_7f800000_00400000;
    10'd775: v = 96'h00000000_ff800000_00400000;
    10'd776: v = 96'h00000000_7fc00000_00400000;
    10'd777: v = 96'h00000000_ffc00001_00400000;
    10'd778: v = 96'h00000000_7f800001_00400000;
    10'd779: v = 96'h00000000_7fbfffff_00400000;
    10'd780: v = 96'h00000000_00400000_00400000;
    10'd781: v = 96'h00000000_34000001_00400000;
    10'd782: v = 96'h00000000_00000000_34000001;
    10'd783: v = 96'h00000000_80000000_34000001;
    10'd784: v = 96'h00000000_00000001_34000001;
    10'd785: v = 96'h00000000_80000001_34000001;
    10'd786: v = 96'h00000000_007fffff_34000001;
    10'd787: v = 96'h00000000_807fffff_34000001;
    10'd788: v = 96'h00000000_00800000_34000001;
    10'd789: v = 96'h00000000_80800000_34000001;
    10'd790: v = 96'h00000000_3f800000_34000001;
    10'd791: v = 96'h00000000_bf800000_34000001;
    10'd792: v = 96'h00000000_7f7fffff_34000001;
    10'd793: v = 96'h00000000_ff7fffff_34000001;
    10'd794: v = 96'h00000000_7f800000_34000001;
    10'd795: v = 96'h00000000_ff800000_34000001;
    10'd796: v = 96'h00000000_7fc00000_34000001;
    10'd797: v = 96'h00000000_ffc00001_34000001;
    10'd798: v = 96'h00000000_7f800001_34000001;
    10'd799: v = 96'h00000000_7fbfffff_34000001;
    10'd800: v = 96'h00000000_00400000_34000001;
    10'd801: v = 96'h00000000_34000001_34000001;
    10'd802: v = 96'h00000000_838017af_037f2d1e;
    10'd803: v = 96'hc06290fa_3fed9c28_3ff41a02;
    10'd804: v = 96'hbfbd935a_3fad14d4_3f8c32b6;
    10'd805: v = 96'hc05deba3_3fe6ae2c_3ff64753;
    10'd806: v = 96'hc0000000_3fe336ec_3ff71f1d;
    10'd807: v = 96'hc02ecfbd_3ff93120_3fb39663;
    10'd808: v = 96'hbfe3c71b_3fa10c14_3fb5098f;
    10'd809: v = 96'hc03ffdd6_3fe6e266_3fd4e05c;
    10'd810: v = 96'h40000000_3f96d597_3ffdde9c;
    10'd811: v = 96'hbfeb01c3_3f9665ed_3fc80221;
    10'd812: v = 96'hbfdec677_3f8c4fbc_3fcb3a65;
    10'd813: v = 96'hc00caee3_3fa11497_3fdf9551;
    10'd814: v = 96'hc0000000_3fefea2e_3f9c5e3b;
    10'd815: v = 96'hc003774a_3f9e5a8e_3fd48847;
    10'd816: v = 96'hbfc51d82_3f803206_3fc4d097;
    10'd817: v = 96'hc02a9310_3fba1fab_3fea9cf6;
    10'd818: v = 96'hc0000000_3fbd875f_3fd4b44f;
    10'd819: v = 96'hbf9437bd_3f8a8c60_3f88eee7;
    10'd820: v = 96'hc0025e93_3f8d0726_3feca6fa;
    10'd821: v = 96'hc02860ea_3fbb0096_3fe68137;
    10'd822: v = 96'hc0000000_3fb31aca_3f9be1f5;
    10'd823: v = 96'hbff18446_3fb19f2b_3fae0b76;
    10'd824: v = 96'hbfb0c374_3f8112fb_3faf4ae0;
    10'd825: v = 96'hbfe2b8f9_3f9edd7d_3fb6ac75;
    10'd826: v = 96'hc0000000_3fa3a789_3fec7c6d;
    10'd827: v = 96'hbfe29e93_3fd6abcb_3f871fd5;
    10'd828: v = 96'hbfac84d0_3f9c50c2_3f8d44aa;
    10'd829: v = 96'hc001e283_3f9a817b_3fd734a3;
    10'd830: v = 96'h40000000_3fde4148_3fccd20a;
    10'd831: v = 96'hc01b9b0d_3fa391ce_3ff3892e;
    10'd832: v = 96'hc047fefc_3ff6453c_3fcfe5bc;
    10'd833: v = 96'hbff3db45_3f8f74a6_3fd99577;
    10'd834: v = 96'hc0000000_3fa51ab0_3fe49b9d;
    10'd835: v = 96'hbf974a4f_3f810703_3f9615ea;
    10'd836: v = 96'hbf9f790e_3f805ed3_3f9f0341;
    10'd837: v = 96'hc015a52b_3fc2a8d7_3fc4cd07;
    10'd838: v = 96'hc0000000_3fc7ee42_3fe4e55d;
    10'd839: v = 96'hc02a583d_3fd5e9fb_3fcbdbd7;
    10'd840: v = 96'hbfc1bfba_3f85ccd4_3fb959a0;
    10'd841: v = 96'hc0181e7b_3fffc27a_3f984313;
    10'd842: v = 96'hc0000000_3f8b3dda_3ff2d755;
    10'd843: v = 96'hc01a7529_3fca7bf6_3fc347bb;
    10'd844: v = 96'hc00871ac_3f93cfbd_3fec4ff4;
    10'd845: v = 96'hc0417bf1_3fd95362_3fe3ea6b;
    10'd846: v = 96'hc0000000_3fdcf094_3fe4d77f;
    10'd847: v = 96'hc0071f9e_3fa9759f_3fcc2111;
    10'd848: v = 96'hbfe7f58b_3f99f270_3fc0dd13;
    10'd849: v = 96'hbfcdd983_3fa32e1e_3fa17869;
    10'd850: v = 96'hc0000000_3fba9330_3fd000d9;
    10'd851: v = 96'hbfdf6dd2_3f80fd93_3fddb698;
    10'd852: v = 96'hbfc8fad0_3fa22138_3f9eabf5;
    10'd853: v = 96'hc052a353_3fd7de7c_3ff9cbe2;
    10'd854: v = 96'hc0000000_3fc95371_3fc080f5;
    10'd855: v = 96'hbff9c5ec_3fb00a30_3fb59cb4;
    10'd856: v = 96'hc006052f_3ff9c97e_3f895a8d;
    10'd857: v = 96'hbfece752_3f9f59d9_3fbe4b78;
    10'd858: v = 96'h40000000_3ffe4248_3fdae9ee;
    10'd859: v = 96'hc03075b7_3fbc4c55_3fefe7bd;
    10'd860: v = 96'hc01a101d_3faa649a_3fe77732;
    10'd861: v = 96'hc004d6f5_3fae355d_3fc3355c;
    10'd862: v = 96'h40000000_3fcbb80b_3ffd6436;
    10'd863: v = 96'hc032632e_3fc8e36d_3fe3537e;
    10'd864: v = 96'hbfa5a289_3f92ae39_3f908a58;
    10'd865: v = 96'hc018956e_3fae31f7_3fe03d4c;
    10'd866: v = 96'h40000000_3ff3b689_3fd32e36;
    default: v = 96'd0;
  endcase
endmodule

// x delayed d steps, 0 to 15, at the compute step; every register clear from reset.
module logos_fpo_late(input clk, input step_en, input rst, input [3:0] d, input [39:0] x, output [39:0] y);
  reg [39:0] line [1:15];
  integer i;
  always @(posedge clk) if (step_en) begin
    if (rst) for (i = 1; i < 16; i = i + 1) line[i] <= 40'd0;
    else begin
      line[1] <= x;
      for (i = 2; i < 16; i = i + 1) line[i] <= line[i-1];
    end
  end
  assign y = d == 4'd0 ? x : line[d];
endmodule

// A block-RAM memory of 512-bit words: the read registered on the rising edges cr is high on and held between them, writes on the rising
// edges ce is high on under a byte mask.
module logos_ram_ce_w512 #(parameter D = 1024, A = 10)
  (input clk,
   input ce, input cr, input [A-1:0] ra, output reg [511:0] rd,
   input we, input [A-1:0] wa, input [511:0] wd, input [63:0] wm);
  reg [511:0] mem [0:D-1];
  always @(posedge clk) if (cr) rd <= mem[ra];
  always @(posedge clk)
    if (ce && we) begin
      if (wm[0]) mem[wa][7:0] <= wd[7:0];
      if (wm[1]) mem[wa][15:8] <= wd[15:8];
      if (wm[2]) mem[wa][23:16] <= wd[23:16];
      if (wm[3]) mem[wa][31:24] <= wd[31:24];
      if (wm[4]) mem[wa][39:32] <= wd[39:32];
      if (wm[5]) mem[wa][47:40] <= wd[47:40];
      if (wm[6]) mem[wa][55:48] <= wd[55:48];
      if (wm[7]) mem[wa][63:56] <= wd[63:56];
      if (wm[8]) mem[wa][71:64] <= wd[71:64];
      if (wm[9]) mem[wa][79:72] <= wd[79:72];
      if (wm[10]) mem[wa][87:80] <= wd[87:80];
      if (wm[11]) mem[wa][95:88] <= wd[95:88];
      if (wm[12]) mem[wa][103:96] <= wd[103:96];
      if (wm[13]) mem[wa][111:104] <= wd[111:104];
      if (wm[14]) mem[wa][119:112] <= wd[119:112];
      if (wm[15]) mem[wa][127:120] <= wd[127:120];
      if (wm[16]) mem[wa][135:128] <= wd[135:128];
      if (wm[17]) mem[wa][143:136] <= wd[143:136];
      if (wm[18]) mem[wa][151:144] <= wd[151:144];
      if (wm[19]) mem[wa][159:152] <= wd[159:152];
      if (wm[20]) mem[wa][167:160] <= wd[167:160];
      if (wm[21]) mem[wa][175:168] <= wd[175:168];
      if (wm[22]) mem[wa][183:176] <= wd[183:176];
      if (wm[23]) mem[wa][191:184] <= wd[191:184];
      if (wm[24]) mem[wa][199:192] <= wd[199:192];
      if (wm[25]) mem[wa][207:200] <= wd[207:200];
      if (wm[26]) mem[wa][215:208] <= wd[215:208];
      if (wm[27]) mem[wa][223:216] <= wd[223:216];
      if (wm[28]) mem[wa][231:224] <= wd[231:224];
      if (wm[29]) mem[wa][239:232] <= wd[239:232];
      if (wm[30]) mem[wa][247:240] <= wd[247:240];
      if (wm[31]) mem[wa][255:248] <= wd[255:248];
      if (wm[32]) mem[wa][263:256] <= wd[263:256];
      if (wm[33]) mem[wa][271:264] <= wd[271:264];
      if (wm[34]) mem[wa][279:272] <= wd[279:272];
      if (wm[35]) mem[wa][287:280] <= wd[287:280];
      if (wm[36]) mem[wa][295:288] <= wd[295:288];
      if (wm[37]) mem[wa][303:296] <= wd[303:296];
      if (wm[38]) mem[wa][311:304] <= wd[311:304];
      if (wm[39]) mem[wa][319:312] <= wd[319:312];
      if (wm[40]) mem[wa][327:320] <= wd[327:320];
      if (wm[41]) mem[wa][335:328] <= wd[335:328];
      if (wm[42]) mem[wa][343:336] <= wd[343:336];
      if (wm[43]) mem[wa][351:344] <= wd[351:344];
      if (wm[44]) mem[wa][359:352] <= wd[359:352];
      if (wm[45]) mem[wa][367:360] <= wd[367:360];
      if (wm[46]) mem[wa][375:368] <= wd[375:368];
      if (wm[47]) mem[wa][383:376] <= wd[383:376];
      if (wm[48]) mem[wa][391:384] <= wd[391:384];
      if (wm[49]) mem[wa][399:392] <= wd[399:392];
      if (wm[50]) mem[wa][407:400] <= wd[407:400];
      if (wm[51]) mem[wa][415:408] <= wd[415:408];
      if (wm[52]) mem[wa][423:416] <= wd[423:416];
      if (wm[53]) mem[wa][431:424] <= wd[431:424];
      if (wm[54]) mem[wa][439:432] <= wd[439:432];
      if (wm[55]) mem[wa][447:440] <= wd[447:440];
      if (wm[56]) mem[wa][455:448] <= wd[455:448];
      if (wm[57]) mem[wa][463:456] <= wd[463:456];
      if (wm[58]) mem[wa][471:464] <= wd[471:464];
      if (wm[59]) mem[wa][479:472] <= wd[479:472];
      if (wm[60]) mem[wa][487:480] <= wd[487:480];
      if (wm[61]) mem[wa][495:488] <= wd[495:488];
      if (wm[62]) mem[wa][503:496] <= wd[503:496];
      if (wm[63]) mem[wa][511:504] <= wd[511:504];
    end
endmodule
// The oracle's core: each step an operand triple (C, B, A) from the run's source into the three hard blocks and a delay
// line, the soft classes reading the line at the run's tap, so each meets the hard results of its own operands; a counter
// a class, one more for each result of it the block's differs from; the steps no class explains counted and their
// records stored, or every step's in a recording run; then the header and the counters stored, and the core idle.
module logos_fp_oracle
  (input clk, input step_en, input read_en, input rst,
   input start, input [31:0] job_at, input [31:0] job_end, input [15:0] rows, input state_f16, input cache_int8, input two_words, input per_block,
   output busy, output reg fault, output reg [31:0] pc,
   input host_wr, input [2:0] host_table, input [31:0] host_addr, input [511:0] host_wdata,
   input [31:0] host_raddr, output [511:0] host_rdata,
   output mem_rd, output [31:0] mem_rd_line, input mem_rd_valid, input [511:0] mem_rd_data,
   output mem_wr, output [31:0] mem_wr_line, output [511:0] mem_wr_data, output [63:0] mem_wr_mask, input mem_wr_ready);
  localparam NM = 84, NA = 36, NV = 867, TAPS = 16, RECORDS = 16, CAP = 4080;
  assign mem_rd = 1'b0;
  assign mem_rd_line = 32'd0;
  assign mem_wr = 1'b0;
  assign mem_wr_line = 32'd0;
  assign mem_wr_data = 512'd0;
  assign mem_wr_mask = 64'd0;

  localparam IDLE = 2'd0, RUN = 2'd1, DUMP = 2'd2;
  reg [1:0] state;
  assign busy = state != IDLE;
  reg [31:0] word, left;
  reg [5:0] drain;
  reg [63:0] seed, s;
  reg [9:0] at;
  reg [63:0] steps, lone_m, lone_a, records;
  reg [11:0] beat;

  // The stream: two xorshift words a step.
  function [63:0] xs(input [63:0] x);
    reg [63:0] y;
    begin
      y = x ^ (x << 13);
      y = y ^ (y >> 7);
      xs = y ^ (y << 17);
    end
  endfunction
  wire [63:0] r1 = xs(s), r2 = xs(r1);
  wire [95:0] rom_v;
  logos_fpo_vectors rom(.i(at), .v(rom_v));
  // Near: a's exponent zero, one, two, 255, 254 or any. Half the time b's places the product within -4 to 3 binades of
  // the least normal (127 - ea + d), a's then from the lower half when any, which b's can meet; a quarter, b's is a's
  // on a coin or within as many binades of it (ea + d), its significand a's but for the low byte and its sign the
  // other, so the sum cancels deep; else b's is any, or one of the ends.
  wire [1:0] kb = r2[7:6];
  wire [2:0] ka = r2[2:0];
  wire [7:0] ea = ka == 3'd0 ? 8'd0 : ka == 3'd1 ? 8'd1 : ka == 3'd2 ? 8'd2 : ka == 3'd3 ? 8'd255 : ka == 3'd4 ? 8'd254
                : kb[1] ? r1[30:23] : {1'b0, r1[29:23]};
  wire signed [10:0] d = $signed({{8{r2[5]}}, r2[5:3]});
  wire cancel = kb == 2'd2;
  wire [7:0] eend = r2[9:8] == 2'd0 ? 8'd0 : r2[9:8] == 2'd1 ? 8'd1 : r2[9:8] == 2'd2 ? 8'd254 : 8'd255;
  wire signed [10:0] eb_w = !kb[1] ? 11'sd127 - $signed({3'd0, ea}) + d : cancel ? $signed({3'd0, ea}) + (r2[11] ? 11'sd0 : d)
                          : r2[10] ? $signed({3'd0, r1[62:55]}) : $signed({3'd0, eend});
  wire [7:0] eb = eb_w < 11'sd0 ? 8'd0 : eb_w > 11'sd255 ? 8'd255 : eb_w[7:0];
  wire [22:0] mb = cancel ? r1[22:0] ^ {15'd0, r1[39:32]} : r1[54:32];
  wire [95:0] near_v = {r2[63:32], cancel ? !r1[31] : r1[63], eb, mb, r1[31], ea, r1[22:0]};
  wire [95:0] next_v = word[1:0] == 2'd0 ? rom_v : word[1:0] == 2'd1 ? {r2[31:0], r1} : near_v;

  // The triple the hard blocks take this step, and the delay line the soft classes read at the tap.
  reg [95:0] opr;
  reg ov;
  reg [95:0] dl [0:TAPS-1];
  reg [TAPS-1:0] dv;
  wire [3:0] tap = word[7:4];
  wire [95:0] t = dl[tap];
  wire tv = dv[tap];

  // The hard blocks' results and flags as they leave them, then each delayed the run's steps for its block.
  wire [31:0] hm_now, ha_now, hf_now, hm1_now, ha1_now;
  wire [7:0] fm_now, fa_now, ff_now, fm1_now, fa1_now;
  wire [31:0] hmul_chain;
  wire hmul_dft;
  fourteennm_fp_mac #(.accumulate_clock("NONE"), .ax_clock("NONE"), .ay_clock("0"), .az_clock("0"), .accum_pipeline_clock("NONE"), .ax_chainin_pl_clock("NONE"), .mult_pipeline_clock("0"), .accum_2nd_pipeline_clock("NONE"), .ax_chainin_2nd_pl_clock("NONE"), .mult_2nd_pipeline_clock("0"), .accum_adder_clock("NONE"), .adder_input_clock("NONE"), .output_clock("0"), .clear_type("NONE"), .use_chainin("false"), .operation_mode("sp_mult"), .adder_subtract("false")) hmul(
    .clk({2'b00, clk}), .ena({2'b00, step_en}), .clr(2'b00), .ay(opr[31:0]), .az(opr[63:32]), .accumulate(1'b0), .chainin(32'd0), .resulta(hm_now), .chainout(hmul_chain),
    .mult_inexact(fm_now[0]), .mult_invalid(fm_now[1]), .mult_overflow(fm_now[2]), .mult_underflow(fm_now[3]),
    .adder_inexact(fm_now[4]), .adder_invalid(fm_now[5]), .adder_overflow(fm_now[6]), .adder_underflow(fm_now[7]), .dftout(hmul_dft));
  wire [31:0] hadd_chain;
  wire hadd_dft;
  fourteennm_fp_mac #(.accumulate_clock("NONE"), .ax_clock("0"), .ay_clock("0"), .az_clock("NONE"), .accum_pipeline_clock("NONE"), .ax_chainin_pl_clock("NONE"), .mult_pipeline_clock("NONE"), .accum_2nd_pipeline_clock("NONE"), .ax_chainin_2nd_pl_clock("NONE"), .mult_2nd_pipeline_clock("NONE"), .accum_adder_clock("NONE"), .adder_input_clock("0"), .output_clock("0"), .clear_type("NONE"), .use_chainin("false"), .operation_mode("sp_add"), .adder_subtract("false")) hadd(
    .clk({2'b00, clk}), .ena({2'b00, step_en}), .clr(2'b00), .ax(opr[31:0]), .ay(opr[63:32]), .accumulate(1'b0), .chainin(32'd0), .resulta(ha_now), .chainout(hadd_chain),
    .mult_inexact(fa_now[0]), .mult_invalid(fa_now[1]), .mult_overflow(fa_now[2]), .mult_underflow(fa_now[3]),
    .adder_inexact(fa_now[4]), .adder_invalid(fa_now[5]), .adder_overflow(fa_now[6]), .adder_underflow(fa_now[7]), .dftout(hadd_dft));
  wire [31:0] hfma_chain;
  wire hfma_dft;
  fourteennm_fp_mac #(.accumulate_clock("NONE"), .ax_clock("0"), .ay_clock("0"), .az_clock("0"), .accum_pipeline_clock("NONE"), .ax_chainin_pl_clock("0"), .mult_pipeline_clock("0"), .accum_2nd_pipeline_clock("NONE"), .ax_chainin_2nd_pl_clock("0"), .mult_2nd_pipeline_clock("0"), .accum_adder_clock("NONE"), .adder_input_clock("0"), .output_clock("0"), .clear_type("NONE"), .use_chainin("false"), .operation_mode("sp_mult_add"), .adder_subtract("false")) hfma(
    .clk({2'b00, clk}), .ena({2'b00, step_en}), .clr(2'b00), .ax(opr[95:64]), .ay(opr[31:0]), .az(opr[63:32]), .accumulate(1'b0), .chainin(32'd0), .resulta(hf_now), .chainout(hfma_chain),
    .mult_inexact(ff_now[0]), .mult_invalid(ff_now[1]), .mult_overflow(ff_now[2]), .mult_underflow(ff_now[3]),
    .adder_inexact(ff_now[4]), .adder_invalid(ff_now[5]), .adder_overflow(ff_now[6]), .adder_underflow(ff_now[7]), .dftout(hfma_dft));
  wire [31:0] hmul1_chain;
  wire hmul1_dft;
  fourteennm_fp_mac #(.accumulate_clock("NONE"), .ax_clock("NONE"), .ay_clock("NONE"), .az_clock("NONE"), .accum_pipeline_clock("NONE"), .ax_chainin_pl_clock("NONE"), .mult_pipeline_clock("NONE"), .accum_2nd_pipeline_clock("NONE"), .ax_chainin_2nd_pl_clock("NONE"), .mult_2nd_pipeline_clock("NONE"), .accum_adder_clock("NONE"), .adder_input_clock("NONE"), .output_clock("0"), .clear_type("NONE"), .use_chainin("false"), .operation_mode("sp_mult"), .adder_subtract("false")) hmul1(
    .clk({2'b00, clk}), .ena({2'b00, step_en}), .clr(2'b00), .ay(opr[31:0]), .az(opr[63:32]), .accumulate(1'b0), .chainin(32'd0), .resulta(hm1_now), .chainout(hmul1_chain),
    .mult_inexact(fm1_now[0]), .mult_invalid(fm1_now[1]), .mult_overflow(fm1_now[2]), .mult_underflow(fm1_now[3]),
    .adder_inexact(fm1_now[4]), .adder_invalid(fm1_now[5]), .adder_overflow(fm1_now[6]), .adder_underflow(fm1_now[7]), .dftout(hmul1_dft));
  wire [31:0] hadd1_chain;
  wire hadd1_dft;
  fourteennm_fp_mac #(.accumulate_clock("NONE"), .ax_clock("NONE"), .ay_clock("NONE"), .az_clock("NONE"), .accum_pipeline_clock("NONE"), .ax_chainin_pl_clock("NONE"), .mult_pipeline_clock("NONE"), .accum_2nd_pipeline_clock("NONE"), .ax_chainin_2nd_pl_clock("NONE"), .mult_2nd_pipeline_clock("NONE"), .accum_adder_clock("NONE"), .adder_input_clock("NONE"), .output_clock("0"), .clear_type("NONE"), .use_chainin("false"), .operation_mode("sp_add"), .adder_subtract("false")) hadd1(
    .clk({2'b00, clk}), .ena({2'b00, step_en}), .clr(2'b00), .ax(opr[31:0]), .ay(opr[63:32]), .accumulate(1'b0), .chainin(32'd0), .resulta(ha1_now), .chainout(hadd1_chain),
    .mult_inexact(fa1_now[0]), .mult_invalid(fa1_now[1]), .mult_overflow(fa1_now[2]), .mult_underflow(fa1_now[3]),
    .adder_inexact(fa1_now[4]), .adder_invalid(fa1_now[5]), .adder_overflow(fa1_now[6]), .adder_underflow(fa1_now[7]), .dftout(hadd1_dft));

  wire [31:0] hm, ha, hf;
  wire [7:0] fm, fa, ff;
  logos_fpo_late lm(.clk(clk), .step_en(step_en), .rst(rst), .d(word[11:8]), .x({fm_now, hm_now}), .y({fm, hm}));
  logos_fpo_late la(.clk(clk), .step_en(step_en), .rst(rst), .d(word[15:12]), .x({fa_now, ha_now}), .y({fa, ha}));
  logos_fpo_late lf(.clk(clk), .step_en(step_en), .rst(rst), .d(word[19:16]), .x({ff_now, hf_now}), .y({ff, hf}));
  wire [31:0] pm0;
  logos_fpo_mul0 um0(.a(t[31:0]), .b(t[63:32]), .y(pm0));
  wire [31:0] pm1;
  logos_fpo_mul1 um1(.a(t[31:0]), .b(t[63:32]), .y(pm1));
  wire [31:0] pm2;
  logos_fpo_mul2 um2(.a(t[31:0]), .b(t[63:32]), .y(pm2));
  wire [31:0] pm3;
  logos_fpo_mul3 um3(.a(t[31:0]), .b(t[63:32]), .y(pm3));
  wire [31:0] pm4;
  logos_fpo_mul4 um4(.a(t[31:0]), .b(t[63:32]), .y(pm4));
  wire [31:0] pm5;
  logos_fpo_mul5 um5(.a(t[31:0]), .b(t[63:32]), .y(pm5));
  wire [31:0] pm6;
  logos_fpo_mul6 um6(.a(t[31:0]), .b(t[63:32]), .y(pm6));
  wire [31:0] pm7;
  logos_fpo_mul7 um7(.a(t[31:0]), .b(t[63:32]), .y(pm7));
  wire [31:0] pm8;
  logos_fpo_mul8 um8(.a(t[31:0]), .b(t[63:32]), .y(pm8));
  wire [31:0] pm9;
  logos_fpo_mul9 um9(.a(t[31:0]), .b(t[63:32]), .y(pm9));
  wire [31:0] pm10;
  logos_fpo_mul10 um10(.a(t[31:0]), .b(t[63:32]), .y(pm10));
  wire [31:0] pm11;
  logos_fpo_mul11 um11(.a(t[31:0]), .b(t[63:32]), .y(pm11));
  wire [31:0] pm12;
  logos_fpo_mul12 um12(.a(t[31:0]), .b(t[63:32]), .y(pm12));
  wire [31:0] pm13;
  logos_fpo_mul13 um13(.a(t[31:0]), .b(t[63:32]), .y(pm13));
  wire [31:0] pm14;
  logos_fpo_mul14 um14(.a(t[31:0]), .b(t[63:32]), .y(pm14));
  wire [31:0] pm15;
  logos_fpo_mul15 um15(.a(t[31:0]), .b(t[63:32]), .y(pm15));
  wire [31:0] pm16;
  logos_fpo_mul16 um16(.a(t[31:0]), .b(t[63:32]), .y(pm16));
  wire [31:0] pm17;
  logos_fpo_mul17 um17(.a(t[31:0]), .b(t[63:32]), .y(pm17));
  wire [31:0] pm18;
  logos_fpo_mul18 um18(.a(t[31:0]), .b(t[63:32]), .y(pm18));
  wire [31:0] pm19;
  logos_fpo_mul19 um19(.a(t[31:0]), .b(t[63:32]), .y(pm19));
  wire [31:0] pm20;
  logos_fpo_mul20 um20(.a(t[31:0]), .b(t[63:32]), .y(pm20));
  wire [31:0] pm21;
  logos_fpo_mul21 um21(.a(t[31:0]), .b(t[63:32]), .y(pm21));
  wire [31:0] pm22;
  logos_fpo_mul22 um22(.a(t[31:0]), .b(t[63:32]), .y(pm22));
  wire [31:0] pm23;
  logos_fpo_mul23 um23(.a(t[31:0]), .b(t[63:32]), .y(pm23));
  wire [31:0] pm24;
  logos_fpo_mul24 um24(.a(t[31:0]), .b(t[63:32]), .y(pm24));
  wire [31:0] pm25;
  logos_fpo_mul25 um25(.a(t[31:0]), .b(t[63:32]), .y(pm25));
  wire [31:0] pm26;
  logos_fpo_mul26 um26(.a(t[31:0]), .b(t[63:32]), .y(pm26));
  wire [31:0] pm27;
  logos_fpo_mul27 um27(.a(t[31:0]), .b(t[63:32]), .y(pm27));
  wire [31:0] pm28;
  logos_fpo_mul28 um28(.a(t[31:0]), .b(t[63:32]), .y(pm28));
  wire [31:0] pm29;
  logos_fpo_mul29 um29(.a(t[31:0]), .b(t[63:32]), .y(pm29));
  wire [31:0] pm30;
  logos_fpo_mul30 um30(.a(t[31:0]), .b(t[63:32]), .y(pm30));
  wire [31:0] pm31;
  logos_fpo_mul31 um31(.a(t[31:0]), .b(t[63:32]), .y(pm31));
  wire [31:0] pm32;
  logos_fpo_mul32 um32(.a(t[31:0]), .b(t[63:32]), .y(pm32));
  wire [31:0] pm33;
  logos_fpo_mul33 um33(.a(t[31:0]), .b(t[63:32]), .y(pm33));
  wire [31:0] pm34;
  logos_fpo_mul34 um34(.a(t[31:0]), .b(t[63:32]), .y(pm34));
  wire [31:0] pm35;
  logos_fpo_mul35 um35(.a(t[31:0]), .b(t[63:32]), .y(pm35));
  wire [31:0] pm36;
  logos_fpo_mul36 um36(.a(t[31:0]), .b(t[63:32]), .y(pm36));
  wire [31:0] pm37;
  logos_fpo_mul37 um37(.a(t[31:0]), .b(t[63:32]), .y(pm37));
  wire [31:0] pm38;
  logos_fpo_mul38 um38(.a(t[31:0]), .b(t[63:32]), .y(pm38));
  wire [31:0] pm39;
  logos_fpo_mul39 um39(.a(t[31:0]), .b(t[63:32]), .y(pm39));
  wire [31:0] pm40;
  logos_fpo_mul40 um40(.a(t[31:0]), .b(t[63:32]), .y(pm40));
  wire [31:0] pm41;
  logos_fpo_mul41 um41(.a(t[31:0]), .b(t[63:32]), .y(pm41));
  wire [31:0] pm42;
  logos_fpo_mul42 um42(.a(t[31:0]), .b(t[63:32]), .y(pm42));
  wire [31:0] pm43;
  logos_fpo_mul43 um43(.a(t[31:0]), .b(t[63:32]), .y(pm43));
  wire [31:0] pm44;
  logos_fpo_mul44 um44(.a(t[31:0]), .b(t[63:32]), .y(pm44));
  wire [31:0] pm45;
  logos_fpo_mul45 um45(.a(t[31:0]), .b(t[63:32]), .y(pm45));
  wire [31:0] pm46;
  logos_fpo_mul46 um46(.a(t[31:0]), .b(t[63:32]), .y(pm46));
  wire [31:0] pm47;
  logos_fpo_mul47 um47(.a(t[31:0]), .b(t[63:32]), .y(pm47));
  wire [31:0] pm48;
  logos_fpo_mul48 um48(.a(t[31:0]), .b(t[63:32]), .y(pm48));
  wire [31:0] pm49;
  logos_fpo_mul49 um49(.a(t[31:0]), .b(t[63:32]), .y(pm49));
  wire [31:0] pm50;
  logos_fpo_mul50 um50(.a(t[31:0]), .b(t[63:32]), .y(pm50));
  wire [31:0] pm51;
  logos_fpo_mul51 um51(.a(t[31:0]), .b(t[63:32]), .y(pm51));
  wire [31:0] pm52;
  logos_fpo_mul52 um52(.a(t[31:0]), .b(t[63:32]), .y(pm52));
  wire [31:0] pm53;
  logos_fpo_mul53 um53(.a(t[31:0]), .b(t[63:32]), .y(pm53));
  wire [31:0] pm54;
  logos_fpo_mul54 um54(.a(t[31:0]), .b(t[63:32]), .y(pm54));
  wire [31:0] pm55;
  logos_fpo_mul55 um55(.a(t[31:0]), .b(t[63:32]), .y(pm55));
  wire [31:0] pm56;
  logos_fpo_mul56 um56(.a(t[31:0]), .b(t[63:32]), .y(pm56));
  wire [31:0] pm57;
  logos_fpo_mul57 um57(.a(t[31:0]), .b(t[63:32]), .y(pm57));
  wire [31:0] pm58;
  logos_fpo_mul58 um58(.a(t[31:0]), .b(t[63:32]), .y(pm58));
  wire [31:0] pm59;
  logos_fpo_mul59 um59(.a(t[31:0]), .b(t[63:32]), .y(pm59));
  wire [31:0] pm60;
  logos_fpo_mul60 um60(.a(t[31:0]), .b(t[63:32]), .y(pm60));
  wire [31:0] pm61;
  logos_fpo_mul61 um61(.a(t[31:0]), .b(t[63:32]), .y(pm61));
  wire [31:0] pm62;
  logos_fpo_mul62 um62(.a(t[31:0]), .b(t[63:32]), .y(pm62));
  wire [31:0] pm63;
  logos_fpo_mul63 um63(.a(t[31:0]), .b(t[63:32]), .y(pm63));
  wire [31:0] pm64;
  logos_fpo_mul64 um64(.a(t[31:0]), .b(t[63:32]), .y(pm64));
  wire [31:0] pm65;
  logos_fpo_mul65 um65(.a(t[31:0]), .b(t[63:32]), .y(pm65));
  wire [31:0] pm66;
  logos_fpo_mul66 um66(.a(t[31:0]), .b(t[63:32]), .y(pm66));
  wire [31:0] pm67;
  logos_fpo_mul67 um67(.a(t[31:0]), .b(t[63:32]), .y(pm67));
  wire [31:0] pm68;
  logos_fpo_mul68 um68(.a(t[31:0]), .b(t[63:32]), .y(pm68));
  wire [31:0] pm69;
  logos_fpo_mul69 um69(.a(t[31:0]), .b(t[63:32]), .y(pm69));
  wire [31:0] pm70;
  logos_fpo_mul70 um70(.a(t[31:0]), .b(t[63:32]), .y(pm70));
  wire [31:0] pm71;
  logos_fpo_mul71 um71(.a(t[31:0]), .b(t[63:32]), .y(pm71));
  wire [31:0] pm72;
  logos_fpo_mul72 um72(.a(t[31:0]), .b(t[63:32]), .y(pm72));
  wire [31:0] pm73;
  logos_fpo_mul73 um73(.a(t[31:0]), .b(t[63:32]), .y(pm73));
  wire [31:0] pm74;
  logos_fpo_mul74 um74(.a(t[31:0]), .b(t[63:32]), .y(pm74));
  wire [31:0] pm75;
  logos_fpo_mul75 um75(.a(t[31:0]), .b(t[63:32]), .y(pm75));
  wire [31:0] pm76;
  logos_fpo_mul76 um76(.a(t[31:0]), .b(t[63:32]), .y(pm76));
  wire [31:0] pm77;
  logos_fpo_mul77 um77(.a(t[31:0]), .b(t[63:32]), .y(pm77));
  wire [31:0] pm78;
  logos_fpo_mul78 um78(.a(t[31:0]), .b(t[63:32]), .y(pm78));
  wire [31:0] pm79;
  logos_fpo_mul79 um79(.a(t[31:0]), .b(t[63:32]), .y(pm79));
  wire [31:0] pm80;
  logos_fpo_mul80 um80(.a(t[31:0]), .b(t[63:32]), .y(pm80));
  wire [31:0] pm81;
  logos_fpo_mul81 um81(.a(t[31:0]), .b(t[63:32]), .y(pm81));
  wire [31:0] pm82;
  logos_fpo_mul82 um82(.a(t[31:0]), .b(t[63:32]), .y(pm82));
  wire [31:0] pm83;
  logos_fpo_mul83 um83(.a(t[31:0]), .b(t[63:32]), .y(pm83));
  wire [31:0] pa0;
  logos_fpo_add0 ua0(.a(t[31:0]), .b(t[63:32]), .y(pa0));
  wire [31:0] pa1;
  logos_fpo_add1 ua1(.a(t[31:0]), .b(t[63:32]), .y(pa1));
  wire [31:0] pa2;
  logos_fpo_add2 ua2(.a(t[31:0]), .b(t[63:32]), .y(pa2));
  wire [31:0] pa3;
  logos_fpo_add3 ua3(.a(t[31:0]), .b(t[63:32]), .y(pa3));
  wire [31:0] pa4;
  logos_fpo_add4 ua4(.a(t[31:0]), .b(t[63:32]), .y(pa4));
  wire [31:0] pa5;
  logos_fpo_add5 ua5(.a(t[31:0]), .b(t[63:32]), .y(pa5));
  wire [31:0] pa6;
  logos_fpo_add6 ua6(.a(t[31:0]), .b(t[63:32]), .y(pa6));
  wire [31:0] pa7;
  logos_fpo_add7 ua7(.a(t[31:0]), .b(t[63:32]), .y(pa7));
  wire [31:0] pa8;
  logos_fpo_add8 ua8(.a(t[31:0]), .b(t[63:32]), .y(pa8));
  wire [31:0] pa9;
  logos_fpo_add9 ua9(.a(t[31:0]), .b(t[63:32]), .y(pa9));
  wire [31:0] pa10;
  logos_fpo_add10 ua10(.a(t[31:0]), .b(t[63:32]), .y(pa10));
  wire [31:0] pa11;
  logos_fpo_add11 ua11(.a(t[31:0]), .b(t[63:32]), .y(pa11));
  wire [31:0] pa12;
  logos_fpo_add12 ua12(.a(t[31:0]), .b(t[63:32]), .y(pa12));
  wire [31:0] pa13;
  logos_fpo_add13 ua13(.a(t[31:0]), .b(t[63:32]), .y(pa13));
  wire [31:0] pa14;
  logos_fpo_add14 ua14(.a(t[31:0]), .b(t[63:32]), .y(pa14));
  wire [31:0] pa15;
  logos_fpo_add15 ua15(.a(t[31:0]), .b(t[63:32]), .y(pa15));
  wire [31:0] pa16;
  logos_fpo_add16 ua16(.a(t[31:0]), .b(t[63:32]), .y(pa16));
  wire [31:0] pa17;
  logos_fpo_add17 ua17(.a(t[31:0]), .b(t[63:32]), .y(pa17));
  wire [31:0] pa18;
  logos_fpo_add18 ua18(.a(t[31:0]), .b(t[63:32]), .y(pa18));
  wire [31:0] pa19;
  logos_fpo_add19 ua19(.a(t[31:0]), .b(t[63:32]), .y(pa19));
  wire [31:0] pa20;
  logos_fpo_add20 ua20(.a(t[31:0]), .b(t[63:32]), .y(pa20));
  wire [31:0] pa21;
  logos_fpo_add21 ua21(.a(t[31:0]), .b(t[63:32]), .y(pa21));
  wire [31:0] pa22;
  logos_fpo_add22 ua22(.a(t[31:0]), .b(t[63:32]), .y(pa22));
  wire [31:0] pa23;
  logos_fpo_add23 ua23(.a(t[31:0]), .b(t[63:32]), .y(pa23));
  wire [31:0] pa24;
  logos_fpo_add24 ua24(.a(t[31:0]), .b(t[63:32]), .y(pa24));
  wire [31:0] pa25;
  logos_fpo_add25 ua25(.a(t[31:0]), .b(t[63:32]), .y(pa25));
  wire [31:0] pa26;
  logos_fpo_add26 ua26(.a(t[31:0]), .b(t[63:32]), .y(pa26));
  wire [31:0] pa27;
  logos_fpo_add27 ua27(.a(t[31:0]), .b(t[63:32]), .y(pa27));
  wire [31:0] pa28;
  logos_fpo_add28 ua28(.a(t[31:0]), .b(t[63:32]), .y(pa28));
  wire [31:0] pa29;
  logos_fpo_add29 ua29(.a(t[31:0]), .b(t[63:32]), .y(pa29));
  wire [31:0] pa30;
  logos_fpo_add30 ua30(.a(t[31:0]), .b(t[63:32]), .y(pa30));
  wire [31:0] pa31;
  logos_fpo_add31 ua31(.a(t[31:0]), .b(t[63:32]), .y(pa31));
  wire [31:0] pa32;
  logos_fpo_add32 ua32(.a(t[31:0]), .b(t[63:32]), .y(pa32));
  wire [31:0] pa33;
  logos_fpo_add33 ua33(.a(t[31:0]), .b(t[63:32]), .y(pa33));
  wire [31:0] pa34;
  logos_fpo_add34 ua34(.a(t[31:0]), .b(t[63:32]), .y(pa34));
  wire [31:0] pa35;
  logos_fpo_add35 ua35(.a(t[31:0]), .b(t[63:32]), .y(pa35));

  wire [NM-1:0] agree_m = {pm83 == hm, pm82 == hm, pm81 == hm, pm80 == hm, pm79 == hm, pm78 == hm, pm77 == hm, pm76 == hm, pm75 == hm, pm74 == hm, pm73 == hm, pm72 == hm, pm71 == hm, pm70 == hm, pm69 == hm, pm68 == hm, pm67 == hm, pm66 == hm, pm65 == hm, pm64 == hm, pm63 == hm, pm62 == hm, pm61 == hm, pm60 == hm, pm59 == hm, pm58 == hm, pm57 == hm, pm56 == hm, pm55 == hm, pm54 == hm, pm53 == hm, pm52 == hm, pm51 == hm, pm50 == hm, pm49 == hm, pm48 == hm, pm47 == hm, pm46 == hm, pm45 == hm, pm44 == hm, pm43 == hm, pm42 == hm, pm41 == hm, pm40 == hm, pm39 == hm, pm38 == hm, pm37 == hm, pm36 == hm, pm35 == hm, pm34 == hm, pm33 == hm, pm32 == hm, pm31 == hm, pm30 == hm, pm29 == hm, pm28 == hm, pm27 == hm, pm26 == hm, pm25 == hm, pm24 == hm, pm23 == hm, pm22 == hm, pm21 == hm, pm20 == hm, pm19 == hm, pm18 == hm, pm17 == hm, pm16 == hm, pm15 == hm, pm14 == hm, pm13 == hm, pm12 == hm, pm11 == hm, pm10 == hm, pm9 == hm, pm8 == hm, pm7 == hm, pm6 == hm, pm5 == hm, pm4 == hm, pm3 == hm, pm2 == hm, pm1 == hm, pm0 == hm};
  wire [NA-1:0] agree_a = {pa35 == ha, pa34 == ha, pa33 == ha, pa32 == ha, pa31 == ha, pa30 == ha, pa29 == ha, pa28 == ha, pa27 == ha, pa26 == ha, pa25 == ha, pa24 == ha, pa23 == ha, pa22 == ha, pa21 == ha, pa20 == ha, pa19 == ha, pa18 == ha, pa17 == ha, pa16 == ha, pa15 == ha, pa14 == ha, pa13 == ha, pa12 == ha, pa11 == ha, pa10 == ha, pa9 == ha, pa8 == ha, pa7 == ha, pa6 == ha, pa5 == ha, pa4 == ha, pa3 == ha, pa2 == ha, pa1 == ha, pa0 == ha};
  wire lonely = !(|agree_m) || !(|agree_a);
  wire keep = tv && records < 64'd4080 && (word[2] || lonely);
  reg [47:0] cm0;
  reg [47:0] cm1;
  reg [47:0] cm2;
  reg [47:0] cm3;
  reg [47:0] cm4;
  reg [47:0] cm5;
  reg [47:0] cm6;
  reg [47:0] cm7;
  reg [47:0] cm8;
  reg [47:0] cm9;
  reg [47:0] cm10;
  reg [47:0] cm11;
  reg [47:0] cm12;
  reg [47:0] cm13;
  reg [47:0] cm14;
  reg [47:0] cm15;
  reg [47:0] cm16;
  reg [47:0] cm17;
  reg [47:0] cm18;
  reg [47:0] cm19;
  reg [47:0] cm20;
  reg [47:0] cm21;
  reg [47:0] cm22;
  reg [47:0] cm23;
  reg [47:0] cm24;
  reg [47:0] cm25;
  reg [47:0] cm26;
  reg [47:0] cm27;
  reg [47:0] cm28;
  reg [47:0] cm29;
  reg [47:0] cm30;
  reg [47:0] cm31;
  reg [47:0] cm32;
  reg [47:0] cm33;
  reg [47:0] cm34;
  reg [47:0] cm35;
  reg [47:0] cm36;
  reg [47:0] cm37;
  reg [47:0] cm38;
  reg [47:0] cm39;
  reg [47:0] cm40;
  reg [47:0] cm41;
  reg [47:0] cm42;
  reg [47:0] cm43;
  reg [47:0] cm44;
  reg [47:0] cm45;
  reg [47:0] cm46;
  reg [47:0] cm47;
  reg [47:0] cm48;
  reg [47:0] cm49;
  reg [47:0] cm50;
  reg [47:0] cm51;
  reg [47:0] cm52;
  reg [47:0] cm53;
  reg [47:0] cm54;
  reg [47:0] cm55;
  reg [47:0] cm56;
  reg [47:0] cm57;
  reg [47:0] cm58;
  reg [47:0] cm59;
  reg [47:0] cm60;
  reg [47:0] cm61;
  reg [47:0] cm62;
  reg [47:0] cm63;
  reg [47:0] cm64;
  reg [47:0] cm65;
  reg [47:0] cm66;
  reg [47:0] cm67;
  reg [47:0] cm68;
  reg [47:0] cm69;
  reg [47:0] cm70;
  reg [47:0] cm71;
  reg [47:0] cm72;
  reg [47:0] cm73;
  reg [47:0] cm74;
  reg [47:0] cm75;
  reg [47:0] cm76;
  reg [47:0] cm77;
  reg [47:0] cm78;
  reg [47:0] cm79;
  reg [47:0] cm80;
  reg [47:0] cm81;
  reg [47:0] cm82;
  reg [47:0] cm83;
  reg [47:0] ca0;
  reg [47:0] ca1;
  reg [47:0] ca2;
  reg [47:0] ca3;
  reg [47:0] ca4;
  reg [47:0] ca5;
  reg [47:0] ca6;
  reg [47:0] ca7;
  reg [47:0] ca8;
  reg [47:0] ca9;
  reg [47:0] ca10;
  reg [47:0] ca11;
  reg [47:0] ca12;
  reg [47:0] ca13;
  reg [47:0] ca14;
  reg [47:0] ca15;
  reg [47:0] ca16;
  reg [47:0] ca17;
  reg [47:0] ca18;
  reg [47:0] ca19;
  reg [47:0] ca20;
  reg [47:0] ca21;
  reg [47:0] ca22;
  reg [47:0] ca23;
  reg [47:0] ca24;
  reg [47:0] ca25;
  reg [47:0] ca26;
  reg [47:0] ca27;
  reg [47:0] ca28;
  reg [47:0] ca29;
  reg [47:0] ca30;
  reg [47:0] ca31;
  reg [47:0] ca32;
  reg [47:0] ca33;
  reg [47:0] ca34;
  reg [47:0] ca35;

  // A record: (B, A), (the hard product, C), (the hard multiply-add, the hard sum), the three blocks' flags {underflow,
  // overflow, invalid, inexact} of the multiply then the add, the product classes' agreement and the least-registered
  // multiply's result as it leaves it, the sum classes' agreement and the least-registered add's.
  wire [95:0] am = agree_m, aa = agree_a;
  wire [511:0] record = {ha1_now, aa, hm1_now, am, 40'd0, ff, fa, fm, hf, ha, hm, t[95:64], t[63:0]};
  reg [511:0] dump_word;
  always @* case (beat)
      0: dump_word = {64'd36, 64'd84, {32'd0, word}, records, lone_a, lone_m, steps, 64'h4f5046534f474f4c};
      1: dump_word = {{16'd0, cm7}, {16'd0, cm6}, {16'd0, cm5}, {16'd0, cm4}, {16'd0, cm3}, {16'd0, cm2}, {16'd0, cm1}, {16'd0, cm0}};
      2: dump_word = {{16'd0, cm15}, {16'd0, cm14}, {16'd0, cm13}, {16'd0, cm12}, {16'd0, cm11}, {16'd0, cm10}, {16'd0, cm9}, {16'd0, cm8}};
      3: dump_word = {{16'd0, cm23}, {16'd0, cm22}, {16'd0, cm21}, {16'd0, cm20}, {16'd0, cm19}, {16'd0, cm18}, {16'd0, cm17}, {16'd0, cm16}};
      4: dump_word = {{16'd0, cm31}, {16'd0, cm30}, {16'd0, cm29}, {16'd0, cm28}, {16'd0, cm27}, {16'd0, cm26}, {16'd0, cm25}, {16'd0, cm24}};
      5: dump_word = {{16'd0, cm39}, {16'd0, cm38}, {16'd0, cm37}, {16'd0, cm36}, {16'd0, cm35}, {16'd0, cm34}, {16'd0, cm33}, {16'd0, cm32}};
      6: dump_word = {{16'd0, cm47}, {16'd0, cm46}, {16'd0, cm45}, {16'd0, cm44}, {16'd0, cm43}, {16'd0, cm42}, {16'd0, cm41}, {16'd0, cm40}};
      7: dump_word = {{16'd0, cm55}, {16'd0, cm54}, {16'd0, cm53}, {16'd0, cm52}, {16'd0, cm51}, {16'd0, cm50}, {16'd0, cm49}, {16'd0, cm48}};
      8: dump_word = {{16'd0, cm63}, {16'd0, cm62}, {16'd0, cm61}, {16'd0, cm60}, {16'd0, cm59}, {16'd0, cm58}, {16'd0, cm57}, {16'd0, cm56}};
      9: dump_word = {{16'd0, cm71}, {16'd0, cm70}, {16'd0, cm69}, {16'd0, cm68}, {16'd0, cm67}, {16'd0, cm66}, {16'd0, cm65}, {16'd0, cm64}};
      10: dump_word = {{16'd0, cm79}, {16'd0, cm78}, {16'd0, cm77}, {16'd0, cm76}, {16'd0, cm75}, {16'd0, cm74}, {16'd0, cm73}, {16'd0, cm72}};
      11: dump_word = {{16'd0, ca3}, {16'd0, ca2}, {16'd0, ca1}, {16'd0, ca0}, {16'd0, cm83}, {16'd0, cm82}, {16'd0, cm81}, {16'd0, cm80}};
      12: dump_word = {{16'd0, ca11}, {16'd0, ca10}, {16'd0, ca9}, {16'd0, ca8}, {16'd0, ca7}, {16'd0, ca6}, {16'd0, ca5}, {16'd0, ca4}};
      13: dump_word = {{16'd0, ca19}, {16'd0, ca18}, {16'd0, ca17}, {16'd0, ca16}, {16'd0, ca15}, {16'd0, ca14}, {16'd0, ca13}, {16'd0, ca12}};
      14: dump_word = {{16'd0, ca27}, {16'd0, ca26}, {16'd0, ca25}, {16'd0, ca24}, {16'd0, ca23}, {16'd0, ca22}, {16'd0, ca21}, {16'd0, ca20}};
      15: dump_word = {{16'd0, ca35}, {16'd0, ca34}, {16'd0, ca33}, {16'd0, ca32}, {16'd0, ca31}, {16'd0, ca30}, {16'd0, ca29}, {16'd0, ca28}};
      default: dump_word = 512'd0;
  endcase
  wire st_we = (state == RUN && keep) || state == DUMP;
  wire [12-1:0] st_wa = state == DUMP ? beat[12-1:0] : RECORDS + records[12-1:0];
  wire [511:0] st_wd = state == DUMP ? dump_word : record;
  logos_ram_ce_w512 #(.D(4096), .A(12)) store(.clk(clk), .ce(step_en), .cr(read_en), .ra(host_raddr[12-1:0]),
    .rd(host_rdata), .we(st_we), .wa(st_wa), .wd(st_wd), .wm({64{1'b1}}));

  integer i;
  always @(posedge clk) if (step_en) begin
    if (rst) begin
      state <= IDLE; fault <= 1'b0; pc <= 32'd0; seed <= 64'd0; opr <= 96'd0; ov <= 1'b0; dv <= {TAPS{1'b0}};
    end else begin
      if (host_wr && host_table == 3'd0 && host_addr == 32'd0) seed <= host_wdata[63:0];
      case (state)
        IDLE: if (start) begin
          state <= RUN; word <= job_at; left <= job_end; drain <= 17;
          s <= seed == 64'd0 ? 64'h9e37_79b9_7f4a_7c15 : seed; at <= 0;
          steps <= 0; lone_m <= 0; lone_a <= 0; records <= 0; pc <= 32'd0; ov <= 1'b0; dv <= {TAPS{1'b0}};
          cm0 <= 48'd0; cm1 <= 48'd0; cm2 <= 48'd0; cm3 <= 48'd0; cm4 <= 48'd0; cm5 <= 48'd0; cm6 <= 48'd0; cm7 <= 48'd0; cm8 <= 48'd0; cm9 <= 48'd0; cm10 <= 48'd0; cm11 <= 48'd0; cm12 <= 48'd0; cm13 <= 48'd0; cm14 <= 48'd0; cm15 <= 48'd0; cm16 <= 48'd0; cm17 <= 48'd0; cm18 <= 48'd0; cm19 <= 48'd0; cm20 <= 48'd0; cm21 <= 48'd0; cm22 <= 48'd0; cm23 <= 48'd0; cm24 <= 48'd0; cm25 <= 48'd0; cm26 <= 48'd0; cm27 <= 48'd0; cm28 <= 48'd0; cm29 <= 48'd0; cm30 <= 48'd0; cm31 <= 48'd0; cm32 <= 48'd0; cm33 <= 48'd0; cm34 <= 48'd0; cm35 <= 48'd0; cm36 <= 48'd0; cm37 <= 48'd0; cm38 <= 48'd0; cm39 <= 48'd0; cm40 <= 48'd0; cm41 <= 48'd0; cm42 <= 48'd0; cm43 <= 48'd0; cm44 <= 48'd0; cm45 <= 48'd0; cm46 <= 48'd0; cm47 <= 48'd0; cm48 <= 48'd0; cm49 <= 48'd0; cm50 <= 48'd0; cm51 <= 48'd0; cm52 <= 48'd0; cm53 <= 48'd0; cm54 <= 48'd0; cm55 <= 48'd0; cm56 <= 48'd0; cm57 <= 48'd0; cm58 <= 48'd0; cm59 <= 48'd0; cm60 <= 48'd0; cm61 <= 48'd0; cm62 <= 48'd0; cm63 <= 48'd0; cm64 <= 48'd0; cm65 <= 48'd0; cm66 <= 48'd0; cm67 <= 48'd0; cm68 <= 48'd0; cm69 <= 48'd0; cm70 <= 48'd0; cm71 <= 48'd0; cm72 <= 48'd0; cm73 <= 48'd0; cm74 <= 48'd0; cm75 <= 48'd0; cm76 <= 48'd0; cm77 <= 48'd0; cm78 <= 48'd0; cm79 <= 48'd0; cm80 <= 48'd0; cm81 <= 48'd0; cm82 <= 48'd0; cm83 <= 48'd0; ca0 <= 48'd0; ca1 <= 48'd0; ca2 <= 48'd0; ca3 <= 48'd0; ca4 <= 48'd0; ca5 <= 48'd0; ca6 <= 48'd0; ca7 <= 48'd0; ca8 <= 48'd0; ca9 <= 48'd0; ca10 <= 48'd0; ca11 <= 48'd0; ca12 <= 48'd0; ca13 <= 48'd0; ca14 <= 48'd0; ca15 <= 48'd0; ca16 <= 48'd0; ca17 <= 48'd0; ca18 <= 48'd0; ca19 <= 48'd0; ca20 <= 48'd0; ca21 <= 48'd0; ca22 <= 48'd0; ca23 <= 48'd0; ca24 <= 48'd0; ca25 <= 48'd0; ca26 <= 48'd0; ca27 <= 48'd0; ca28 <= 48'd0; ca29 <= 48'd0; ca30 <= 48'd0; ca31 <= 48'd0; ca32 <= 48'd0; ca33 <= 48'd0; ca34 <= 48'd0; ca35 <= 48'd0;
        end
        RUN: begin
          if (left != 32'd0) begin
            opr <= next_v; ov <= 1'b1; left <= left - 32'd1;
            if (word[1:0] == 2'd0) at <= at == 10'd866 ? 10'd0 : at + 1'b1;
            else s <= r2;
          end else begin
            ov <= 1'b0;
            if (drain == 6'd0) begin state <= DUMP; beat <= 12'd0; end else drain <= drain - 6'd1;
          end
          dl[0] <= opr;
          for (i = 1; i < TAPS; i = i + 1) dl[i] <= dl[i-1];
          dv <= {dv[TAPS-2:0], ov};
          if (tv) begin
            steps <= steps + 64'd1;
            pc <= pc + 32'd1;
            if (!(|agree_m)) lone_m <= lone_m + 64'd1;
            if (!(|agree_a)) lone_a <= lone_a + 64'd1;
          if (!agree_m[0]) cm0 <= cm0 + 48'd1;
          if (!agree_m[1]) cm1 <= cm1 + 48'd1;
          if (!agree_m[2]) cm2 <= cm2 + 48'd1;
          if (!agree_m[3]) cm3 <= cm3 + 48'd1;
          if (!agree_m[4]) cm4 <= cm4 + 48'd1;
          if (!agree_m[5]) cm5 <= cm5 + 48'd1;
          if (!agree_m[6]) cm6 <= cm6 + 48'd1;
          if (!agree_m[7]) cm7 <= cm7 + 48'd1;
          if (!agree_m[8]) cm8 <= cm8 + 48'd1;
          if (!agree_m[9]) cm9 <= cm9 + 48'd1;
          if (!agree_m[10]) cm10 <= cm10 + 48'd1;
          if (!agree_m[11]) cm11 <= cm11 + 48'd1;
          if (!agree_m[12]) cm12 <= cm12 + 48'd1;
          if (!agree_m[13]) cm13 <= cm13 + 48'd1;
          if (!agree_m[14]) cm14 <= cm14 + 48'd1;
          if (!agree_m[15]) cm15 <= cm15 + 48'd1;
          if (!agree_m[16]) cm16 <= cm16 + 48'd1;
          if (!agree_m[17]) cm17 <= cm17 + 48'd1;
          if (!agree_m[18]) cm18 <= cm18 + 48'd1;
          if (!agree_m[19]) cm19 <= cm19 + 48'd1;
          if (!agree_m[20]) cm20 <= cm20 + 48'd1;
          if (!agree_m[21]) cm21 <= cm21 + 48'd1;
          if (!agree_m[22]) cm22 <= cm22 + 48'd1;
          if (!agree_m[23]) cm23 <= cm23 + 48'd1;
          if (!agree_m[24]) cm24 <= cm24 + 48'd1;
          if (!agree_m[25]) cm25 <= cm25 + 48'd1;
          if (!agree_m[26]) cm26 <= cm26 + 48'd1;
          if (!agree_m[27]) cm27 <= cm27 + 48'd1;
          if (!agree_m[28]) cm28 <= cm28 + 48'd1;
          if (!agree_m[29]) cm29 <= cm29 + 48'd1;
          if (!agree_m[30]) cm30 <= cm30 + 48'd1;
          if (!agree_m[31]) cm31 <= cm31 + 48'd1;
          if (!agree_m[32]) cm32 <= cm32 + 48'd1;
          if (!agree_m[33]) cm33 <= cm33 + 48'd1;
          if (!agree_m[34]) cm34 <= cm34 + 48'd1;
          if (!agree_m[35]) cm35 <= cm35 + 48'd1;
          if (!agree_m[36]) cm36 <= cm36 + 48'd1;
          if (!agree_m[37]) cm37 <= cm37 + 48'd1;
          if (!agree_m[38]) cm38 <= cm38 + 48'd1;
          if (!agree_m[39]) cm39 <= cm39 + 48'd1;
          if (!agree_m[40]) cm40 <= cm40 + 48'd1;
          if (!agree_m[41]) cm41 <= cm41 + 48'd1;
          if (!agree_m[42]) cm42 <= cm42 + 48'd1;
          if (!agree_m[43]) cm43 <= cm43 + 48'd1;
          if (!agree_m[44]) cm44 <= cm44 + 48'd1;
          if (!agree_m[45]) cm45 <= cm45 + 48'd1;
          if (!agree_m[46]) cm46 <= cm46 + 48'd1;
          if (!agree_m[47]) cm47 <= cm47 + 48'd1;
          if (!agree_m[48]) cm48 <= cm48 + 48'd1;
          if (!agree_m[49]) cm49 <= cm49 + 48'd1;
          if (!agree_m[50]) cm50 <= cm50 + 48'd1;
          if (!agree_m[51]) cm51 <= cm51 + 48'd1;
          if (!agree_m[52]) cm52 <= cm52 + 48'd1;
          if (!agree_m[53]) cm53 <= cm53 + 48'd1;
          if (!agree_m[54]) cm54 <= cm54 + 48'd1;
          if (!agree_m[55]) cm55 <= cm55 + 48'd1;
          if (!agree_m[56]) cm56 <= cm56 + 48'd1;
          if (!agree_m[57]) cm57 <= cm57 + 48'd1;
          if (!agree_m[58]) cm58 <= cm58 + 48'd1;
          if (!agree_m[59]) cm59 <= cm59 + 48'd1;
          if (!agree_m[60]) cm60 <= cm60 + 48'd1;
          if (!agree_m[61]) cm61 <= cm61 + 48'd1;
          if (!agree_m[62]) cm62 <= cm62 + 48'd1;
          if (!agree_m[63]) cm63 <= cm63 + 48'd1;
          if (!agree_m[64]) cm64 <= cm64 + 48'd1;
          if (!agree_m[65]) cm65 <= cm65 + 48'd1;
          if (!agree_m[66]) cm66 <= cm66 + 48'd1;
          if (!agree_m[67]) cm67 <= cm67 + 48'd1;
          if (!agree_m[68]) cm68 <= cm68 + 48'd1;
          if (!agree_m[69]) cm69 <= cm69 + 48'd1;
          if (!agree_m[70]) cm70 <= cm70 + 48'd1;
          if (!agree_m[71]) cm71 <= cm71 + 48'd1;
          if (!agree_m[72]) cm72 <= cm72 + 48'd1;
          if (!agree_m[73]) cm73 <= cm73 + 48'd1;
          if (!agree_m[74]) cm74 <= cm74 + 48'd1;
          if (!agree_m[75]) cm75 <= cm75 + 48'd1;
          if (!agree_m[76]) cm76 <= cm76 + 48'd1;
          if (!agree_m[77]) cm77 <= cm77 + 48'd1;
          if (!agree_m[78]) cm78 <= cm78 + 48'd1;
          if (!agree_m[79]) cm79 <= cm79 + 48'd1;
          if (!agree_m[80]) cm80 <= cm80 + 48'd1;
          if (!agree_m[81]) cm81 <= cm81 + 48'd1;
          if (!agree_m[82]) cm82 <= cm82 + 48'd1;
          if (!agree_m[83]) cm83 <= cm83 + 48'd1;
          if (!agree_a[0]) ca0 <= ca0 + 48'd1;
          if (!agree_a[1]) ca1 <= ca1 + 48'd1;
          if (!agree_a[2]) ca2 <= ca2 + 48'd1;
          if (!agree_a[3]) ca3 <= ca3 + 48'd1;
          if (!agree_a[4]) ca4 <= ca4 + 48'd1;
          if (!agree_a[5]) ca5 <= ca5 + 48'd1;
          if (!agree_a[6]) ca6 <= ca6 + 48'd1;
          if (!agree_a[7]) ca7 <= ca7 + 48'd1;
          if (!agree_a[8]) ca8 <= ca8 + 48'd1;
          if (!agree_a[9]) ca9 <= ca9 + 48'd1;
          if (!agree_a[10]) ca10 <= ca10 + 48'd1;
          if (!agree_a[11]) ca11 <= ca11 + 48'd1;
          if (!agree_a[12]) ca12 <= ca12 + 48'd1;
          if (!agree_a[13]) ca13 <= ca13 + 48'd1;
          if (!agree_a[14]) ca14 <= ca14 + 48'd1;
          if (!agree_a[15]) ca15 <= ca15 + 48'd1;
          if (!agree_a[16]) ca16 <= ca16 + 48'd1;
          if (!agree_a[17]) ca17 <= ca17 + 48'd1;
          if (!agree_a[18]) ca18 <= ca18 + 48'd1;
          if (!agree_a[19]) ca19 <= ca19 + 48'd1;
          if (!agree_a[20]) ca20 <= ca20 + 48'd1;
          if (!agree_a[21]) ca21 <= ca21 + 48'd1;
          if (!agree_a[22]) ca22 <= ca22 + 48'd1;
          if (!agree_a[23]) ca23 <= ca23 + 48'd1;
          if (!agree_a[24]) ca24 <= ca24 + 48'd1;
          if (!agree_a[25]) ca25 <= ca25 + 48'd1;
          if (!agree_a[26]) ca26 <= ca26 + 48'd1;
          if (!agree_a[27]) ca27 <= ca27 + 48'd1;
          if (!agree_a[28]) ca28 <= ca28 + 48'd1;
          if (!agree_a[29]) ca29 <= ca29 + 48'd1;
          if (!agree_a[30]) ca30 <= ca30 + 48'd1;
          if (!agree_a[31]) ca31 <= ca31 + 48'd1;
          if (!agree_a[32]) ca32 <= ca32 + 48'd1;
          if (!agree_a[33]) ca33 <= ca33 + 48'd1;
          if (!agree_a[34]) ca34 <= ca34 + 48'd1;
          if (!agree_a[35]) ca35 <= ca35 + 48'd1;
          end
          if (keep) records <= records + 64'd1;
        end
        default: if (beat == 12'd15) state <= IDLE; else beat <= beat + 12'd1;
      endcase
    end
  end
endmodule

// The oracle as the shell's engine: its core stepping in the user clock (fclk) at the compute clock's rising edges
// (fedge), its store read the user cycle after the falling edges, the shell's values and its own through the
// engine's crossing.
module logos_engine
  (input clk,
   input rst,
   input start,
   input [31:0] job_at,
   input [31:0] job_end,
   input [15:0] rows,
   input state_f16,
   input cache_int8,
   input two_words,
   input per_block,
   output busy,
   output fault,
   output [31:0] pc,
   input host_wr,
   input [2:0] host_table,
   input [31:0] host_addr,
   input [511:0] host_wdata,
   input [31:0] host_raddr,
   output [511:0] host_rdata,
   output mem_rd,
   output [31:0] mem_rd_line,
   input mem_rd_valid,
   input [511:0] mem_rd_data,
   output mem_wr,
   output [31:0] mem_wr_line,
   output [511:0] mem_wr_data,
   output [63:0] mem_wr_mask,
   input mem_wr_ready,
   input fclk, input frst, input fedge, input fmid);
  reg cr;
  always @(posedge fclk) cr <= fmid;
  wire f_rst;
  wire f_start;
  wire [31:0] f_job_at;
  wire [31:0] f_job_end;
  wire [15:0] f_rows;
  wire f_state_f16;
  wire f_cache_int8;
  wire f_two_words;
  wire f_per_block;
  wire f_busy;
  wire f_fault;
  wire [31:0] f_pc;
  wire f_host_wr;
  wire [2:0] f_host_table;
  wire [31:0] f_host_addr;
  wire [511:0] f_host_wdata;
  wire [31:0] f_host_raddr;
  wire [511:0] f_host_rdata;
  wire f_mem_rd;
  wire [31:0] f_mem_rd_line;
  wire f_mem_rd_valid;
  wire [511:0] f_mem_rd_data;
  wire f_mem_wr;
  wire [31:0] f_mem_wr_line;
  wire [511:0] f_mem_wr_data;
  wire [63:0] f_mem_wr_mask;
  wire f_mem_wr_ready;
  logos_engine_bridge bridge(.sclk(clk), .fclk(fclk), .frst(frst), .fedge(fedge), .fmid(fmid),
    .s_rst(rst), .f_rst(f_rst),
    .s_start(start), .f_start(f_start),
    .s_job_at(job_at), .f_job_at(f_job_at),
    .s_job_end(job_end), .f_job_end(f_job_end),
    .s_rows(rows), .f_rows(f_rows),
    .s_state_f16(state_f16), .f_state_f16(f_state_f16),
    .s_cache_int8(cache_int8), .f_cache_int8(f_cache_int8),
    .s_two_words(two_words), .f_two_words(f_two_words),
    .s_per_block(per_block), .f_per_block(f_per_block),
    .s_busy(busy), .f_busy(f_busy),
    .s_fault(fault), .f_fault(f_fault),
    .s_pc(pc), .f_pc(f_pc),
    .s_host_wr(host_wr), .f_host_wr(f_host_wr),
    .s_host_table(host_table), .f_host_table(f_host_table),
    .s_host_addr(host_addr), .f_host_addr(f_host_addr),
    .s_host_wdata(host_wdata), .f_host_wdata(f_host_wdata),
    .s_host_raddr(host_raddr), .f_host_raddr(f_host_raddr),
    .s_host_rdata(host_rdata), .f_host_rdata(f_host_rdata),
    .s_mem_rd(mem_rd), .f_mem_rd(f_mem_rd),
    .s_mem_rd_line(mem_rd_line), .f_mem_rd_line(f_mem_rd_line),
    .s_mem_rd_valid(mem_rd_valid), .f_mem_rd_valid(f_mem_rd_valid),
    .s_mem_rd_data(mem_rd_data), .f_mem_rd_data(f_mem_rd_data),
    .s_mem_wr(mem_wr), .f_mem_wr(f_mem_wr),
    .s_mem_wr_line(mem_wr_line), .f_mem_wr_line(f_mem_wr_line),
    .s_mem_wr_data(mem_wr_data), .f_mem_wr_data(f_mem_wr_data),
    .s_mem_wr_mask(mem_wr_mask), .f_mem_wr_mask(f_mem_wr_mask),
    .s_mem_wr_ready(mem_wr_ready), .f_mem_wr_ready(f_mem_wr_ready));
  logos_fp_oracle core(.clk(fclk), .step_en(fedge), .read_en(cr),
    .rst(f_rst),
    .start(f_start),
    .job_at(f_job_at),
    .job_end(f_job_end),
    .rows(f_rows),
    .state_f16(f_state_f16),
    .cache_int8(f_cache_int8),
    .two_words(f_two_words),
    .per_block(f_per_block),
    .busy(f_busy),
    .fault(f_fault),
    .pc(f_pc),
    .host_wr(f_host_wr),
    .host_table(f_host_table),
    .host_addr(f_host_addr),
    .host_wdata(f_host_wdata),
    .host_raddr(f_host_raddr),
    .host_rdata(f_host_rdata),
    .mem_rd(f_mem_rd),
    .mem_rd_line(f_mem_rd_line),
    .mem_rd_valid(f_mem_rd_valid),
    .mem_rd_data(f_mem_rd_data),
    .mem_wr(f_mem_wr),
    .mem_wr_line(f_mem_wr_line),
    .mem_wr_data(f_mem_wr_data),
    .mem_wr_mask(f_mem_wr_mask),
    .mem_wr_ready(f_mem_wr_ready));
endmodule
