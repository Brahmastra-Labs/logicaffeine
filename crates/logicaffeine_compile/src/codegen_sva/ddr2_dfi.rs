//! ddr2_dfi — `ddr2_dfi_ctrl`, the LOGOS DDR2 controller at the DFI 1:2 rate: a 64-bit native request port (a word a
//! command, write data on its own stream, read data back in request order) and two DFI phases a system clock, the
//! interface LiteDRAM's controller presents to its PHY, so either drives the same PHY.
//!
//! One BL4 burst of the x16 part is a 64-bit word and two DRAM clocks, one system clock: a column command every
//! system clock is the bus's whole bandwidth. The controller keeps one in-order queue of requests and three
//! pointers into it: the tail takes requests, `prep` opens the row of the next request (bank lookahead: a row opens
//! while earlier requests in other banks are still being served), and the head issues column commands, in request
//! order, so read data comes back in request order and no reorder buffer is needed. A row stays open only while a
//! prepared request still needs it or the next request to prepare reads it; otherwise its last column command closes
//! it (auto-precharge), so a request that finds the controller idle pays ACT and the column command, not a
//! precharge first. A row is open only while a prepared request needs it, or for the clock in which the next request
//! to prepare takes it as a hit, so no request ever finds its bank open on another row: the only PRECHARGE the
//! controller issues is the precharge-all before a refresh.
//!
//! Column commands go on phase 0, row and refresh commands on phase 1. Every rule of the clocked part is a
//! down-counter in DRAM clocks from the current system clock's phase 0, decremented by two a clock: a column command
//! may issue when its counters are 0, a row command when they are at most 1. tRRD and tFAW are a window over the last
//! system clocks' ACTs. Refresh is postponed while requests wait — up to the eight JESD79-2 allows — and repaid when
//! the queue is empty or the debt reaches eight: then no new row opens, the prepared requests drain, every bank
//! precharges and the REFRESH issues. A row is therefore never open longer than eight refresh intervals and the
//! drain, inside tRAS max. Read data is delivered without back-pressure, as LiteDRAM's crossbar delivers it.

use super::ddr2_jedec::{Ddr2Timing, REFRESH_POSTPONE_MAX};
use super::ddr2_phy::{ddr2_pins, Ddr2Cmd};

/// The requests `ddr2_dfi_ctrl` holds: the window its bank lookahead sees.
pub const DFI_CTRL_QUEUE: u32 = 4;

/// The fewest bits that hold `v`.
fn bits_for(v: u64) -> u32 {
    (u64::BITS - v.leading_zeros()).max(1)
}

/// `cs#, ras#, cas#, we#` of `cmd` as a Verilog constant.
fn pins(cmd: Ddr2Cmd) -> String {
    let (cs, ras, cas, we) = ddr2_pins(cmd);
    format!("4'b{cs}{ras}{cas}{we}")
}

/// Emit `ddr2_dfi_ctrl` for the clocked part `t`: every rule of `t` baked in, a [`DFI_CTRL_QUEUE`]-deep queue.
///
/// The part must be the one the 1:2 rate and the port assume: x16, eight banks, 1 Kb columns, burst length 4 (one
/// burst a 64-bit word, a system clock of data), column commands two DRAM clocks apart at most, and eight refresh
/// intervals and the queue's drain inside tRAS max: a row opened just after a refresh stays open, at the longest,
/// until the eighth refresh owed forces the queue to drain and every bank to precharge.
pub fn emit_ddr2_dfi_controller_verilog(t: &Ddr2Timing) -> String {
    let g = &t.grade;
    assert_eq!(t.bl, 4, "{}: ddr2_dfi_ctrl carries a 64-bit word as one BL4 burst", g.part);
    assert_eq!((g.dq_bits, g.banks, g.col_bits), (16, 8, 10), "{}: ddr2_dfi_ctrl drives an x16, eight-bank part with 1 Kb columns", g.part);
    assert!(g.row_bits <= 13, "{}: the DDR2 address bus carries 13 row bits", g.part);
    assert!(t.cas_to_cas() <= 2, "{}: a column command every system clock needs CAS-to-CAS of two DRAM clocks", g.part);
    let drain = u64::from(DFI_CTRL_QUEUE) * (t.trc + t.write_to_read()) + t.trpa;
    assert!(
        REFRESH_POSTPONE_MAX * t.trefi + drain < t.tras_max,
        "{}: eight postponed refreshes and a drain would hold a row past tRAS max",
        g.part
    );

    let ap_ready = t.write_to_precharge().max(1 + t.tras) + t.trp;
    let widest = [1 + t.trc, 1 + t.tras, ap_ready, 1 + t.trfc, 1 + t.trpa, t.write_to_read(), t.read_to_write(), t.cas_to_cas(), 1 + t.trcd]
        .into_iter()
        .max()
        .unwrap();
    let cw = bits_for(widest);
    let rcw = bits_for(1 + t.trcd);
    let refi = t.trefi / 2;
    let faw_window = t.tfaw.div_ceil(2).saturating_sub(1);
    let rrd_window = t.trrd.div_ceil(2).saturating_sub(1);
    let history = faw_window.max(rrd_window).max(1);
    let faw_ok = if faw_window >= 4 {
        let w = bits_for(faw_window);
        let ones: Vec<String> = (0..faw_window).map(|i| format!("{{{}'d0, acts[{i}]}}", w - 1)).collect();
        format!("({}) < {w}'d4", ones.join(" + "))
    } else {
        "1'b1".to_string()
    };
    let rrd_ok = if rrd_window > 0 { format!("acts[{}:0] == 0", rrd_window - 1) } else { "1'b1".to_string() };
    let qb = bits_for(u64::from(DFI_CTRL_QUEUE) - 1);

    let v = r#"// ddr2_dfi_ctrl — LOGOS DDR2 controller, %PART% at tCK %TCK% ps, DFI 1:2: an in-order queue whose rows open ahead
// in other banks, a row left open only while a waiting request reads it, refresh postponed while requests wait.
module ddr2_dfi_ctrl(
  input sys_clk, input sys_rst,
  input cmd_valid, output cmd_ready, input cmd_payload_we, input [23:0] cmd_payload_addr,
  input wdata_valid, output wdata_ready, input [63:0] wdata_payload_data, input [7:0] wdata_payload_we,
  output reg rdata_valid, output reg [63:0] rdata_payload_data,
  output reg [12:0] dfi_p0_address, output reg [2:0] dfi_p0_bank,
  output reg dfi_p0_cs_n, output reg dfi_p0_ras_n, output reg dfi_p0_cas_n, output reg dfi_p0_we_n,
  output dfi_p0_cke, output dfi_p0_odt, output dfi_p0_reset_n,
  output reg [31:0] dfi_p0_wrdata, output reg dfi_p0_wrdata_en, output reg [3:0] dfi_p0_wrdata_mask,
  output reg dfi_p0_rddata_en, input [31:0] dfi_p0_rddata, input dfi_p0_rddata_valid,
  output reg [12:0] dfi_p1_address, output reg [2:0] dfi_p1_bank,
  output reg dfi_p1_cs_n, output reg dfi_p1_ras_n, output reg dfi_p1_cas_n, output reg dfi_p1_we_n,
  output dfi_p1_cke, output dfi_p1_odt, output dfi_p1_reset_n,
  output reg [31:0] dfi_p1_wrdata, output reg dfi_p1_wrdata_en, output reg [3:0] dfi_p1_wrdata_mask,
  output reg dfi_p1_rddata_en, input [31:0] dfi_p1_rddata, input dfi_p1_rddata_valid
);
  localparam QB = %QB%, QD = 1 << QB, CW = %CW%, RCW = %RCW%;
  localparam [CW-1:0] T_RCD = %T_RCD%, T_RP = %T_RP%, T_RPA = %T_RPA%, T_RAS = %T_RAS%, T_RC = %T_RC%, T_RFC = %T_RFC%;
  localparam [CW-1:0] RD2PRE = %RD2PRE%, WR2PRE = %WR2PRE%, WR2RD = %WR2RD%, RD2WR = %RD2WR%, CAS2CAS = %CAS2CAS%;
  localparam [%RB%-1:0] REFI = %REFI%;
  localparam [3:0] POSTPONE = %POSTPONE%;
  localparam [3:0] NOP = %NOP%, ACT = %ACT%, PRE = %PRE%, REF = %REF%, RD = %RD%, WR = %WR%;

  assign dfi_p0_cke = 1'b1; assign dfi_p1_cke = 1'b1;
  assign dfi_p0_odt = 1'b1; assign dfi_p1_odt = 1'b1;
  assign dfi_p0_reset_n = 1'b1; assign dfi_p1_reset_n = 1'b1;

  function [CW-1:0] dec2(input [CW-1:0] v); dec2 = v > 2 ? v - 2'd2 : {CW{1'b0}}; endfunction
  function [CW-1:0] mx(input [CW-1:0] a, input [CW-1:0] b); mx = a > b ? a : b; endfunction

  // the requests: the tail takes them, prep opens their rows, the head issues their column commands
  reg q_we [0:QD-1]; reg [2:0] q_bank [0:QD-1]; reg [12:0] q_row [0:QD-1]; reg [7:0] q_col [0:QD-1];
  reg [QB:0] tail, prep, head;
  reg [QD-1:0] ready;
  wire full = tail[QB] != head[QB] && tail[QB-1:0] == head[QB-1:0];
  wire idle = tail == head, has_prep = prep != tail, has_cas = head != prep;
  assign cmd_ready = !full;
  wire [2:0] pb = q_bank[prep[QB-1:0]]; wire [12:0] pr = q_row[prep[QB-1:0]];
  wire [2:0] hb = q_bank[head[QB-1:0]]; wire hwe = q_we[head[QB-1:0]]; wire [7:0] hcol = q_col[head[QB-1:0]];

  // the next write's data, refilled in the clock its WRITE issues
  reg [63:0] w_data; reg [7:0] w_we; reg w_held;

  // the banks: open row, and when each command may next come, in DRAM clocks from this clock's phase 0
  wire [7:0] open;
  wire [8*13-1:0] orow;
  wire [8*CW-1:0] act_w, pre_w;
  wire [8*RCW-1:0] cas_w;
  reg [CW-1:0] ccd_w, rd_w, wr_w, ref_w;
  reg [%H%-1:0] acts;
  reg [%RB%-1:0] refi; reg [3:0] owed; reg ref_mode;

  // a prepared request in the prep request's bank; another prepared request in the head's
  reg p_busy, h_shared; integer i;
  always @(*) begin
    p_busy = 1'b0; h_shared = 1'b0;
    for (i = 0; i < QD; i = i + 1) begin
      if (ready[i] && q_bank[i] == pb) p_busy = 1'b1;
      if (ready[i] && i != head[QB-1:0] && q_bank[i] == hb) h_shared = 1'b1;
    end
  end

  wire force_ref = owed >= POSTPONE;
  wire p_hit = open[pb] && orow[pb * 13 +: 13] == pr;
  wire p_go = has_prep && !ref_mode && !force_ref;
  wire do_hit = p_go && p_hit;
  wire do_act = p_go && !open[pb] && !p_busy && act_w[pb * CW +: CW] <= 1 && %FAW_OK% && %RRD_OK%;

  wire cas_go = has_cas && cas_w[hb * RCW +: RCW] == 0 && ccd_w == 0 && (hwe ? wr_w == 0 && w_held : rd_w == 0);
  wire keep = has_prep && pb == hb && p_hit;
  wire ap = !h_shared && !keep;
  assign wdata_ready = !w_held || cas_go && hwe;

  reg all_pre, all_ready; integer b;
  always @(*) begin
    all_pre = 1'b1; all_ready = 1'b1;
    for (b = 0; b < 8; b = b + 1) begin
      if (open[b] && pre_w[b * CW +: CW] > 1) all_pre = 1'b0;
      if (act_w[b * CW +: CW] > 1) all_ready = 1'b0;
    end
  end
  wire ref_go = !ref_mode && owed != 0 && !has_cas && (idle || force_ref);
  wire pre_all = ref_mode && |open && all_pre;
  wire refresh = ref_mode && !(|open) && all_ready && ref_w <= 1;
  wire tick = refi == 0;
  wire [CW-1:0] ap_ready = mx(hwe ? WR2PRE : RD2PRE, pre_w[hb * CW +: CW]) + T_RP;

  genvar g;
  generate for (g = 0; g < 8; g = g + 1) begin : bank
    reg op; reg [12:0] row; reg [CW-1:0] aw, pw; reg [RCW-1:0] cw;
    assign open[g] = op; assign orow[g * 13 +: 13] = row;
    assign act_w[g * CW +: CW] = aw; assign pre_w[g * CW +: CW] = pw; assign cas_w[g * RCW +: RCW] = cw;
    wire act_here = do_act && pb == g, cas_here = cas_go && hb == g;
    wire [CW-1:0] a_n = act_here ? T_RC + 1'b1 : cas_here && ap ? ap_ready
                      : pre_all && op ? T_RPA + 1'b1 : refresh ? T_RFC + 1'b1 : aw;
    wire [CW-1:0] p_n = act_here ? T_RAS + 1'b1 : cas_here ? (hwe ? WR2PRE : mx(pw, RD2PRE)) : pw;
    always @(posedge sys_clk)
      if (sys_rst) begin op <= 1'b0; aw <= 0; pw <= 0; cw <= 0; end
      else begin
        aw <= dec2(a_n); pw <= dec2(p_n);
        cw <= act_here ? T_RCD - 1'b1 : cw > 2 ? cw - 2'd2 : {RCW{1'b0}};
        if (act_here) op <= 1'b1;
        else if (cas_here && ap || pre_all) op <= 1'b0;
      end
    always @(posedge sys_clk) if (act_here) row <= pr;
  end endgenerate

  always @(posedge sys_clk) begin
    if (sys_rst) begin
      tail <= 0; prep <= 0; head <= 0; ready <= 0; w_held <= 1'b0;
      ccd_w <= 0; rd_w <= 0; wr_w <= 0; ref_w <= 0; acts <= 0; refi <= REFI - 1'b1; owed <= 0; ref_mode <= 1'b0;
      rdata_valid <= 1'b0;
      {dfi_p0_cs_n, dfi_p0_ras_n, dfi_p0_cas_n, dfi_p0_we_n} <= NOP; {dfi_p1_cs_n, dfi_p1_ras_n, dfi_p1_cas_n, dfi_p1_we_n} <= NOP;
      dfi_p0_wrdata_en <= 1'b0; dfi_p1_wrdata_en <= 1'b0; dfi_p0_rddata_en <= 1'b0; dfi_p1_rddata_en <= 1'b0;
    end else begin
      if (cmd_valid && !full) tail <= tail + 1'b1;
      if (wdata_valid && wdata_ready) w_held <= 1'b1; else if (cas_go && hwe) w_held <= 1'b0;
      if (do_hit || do_act) begin prep <= prep + 1'b1; ready[prep[QB-1:0]] <= 1'b1; end
      if (cas_go) begin head <= head + 1'b1; ready[head[QB-1:0]] <= 1'b0; end
      ccd_w <= dec2(cas_go ? CAS2CAS : ccd_w);
      rd_w <= dec2(cas_go && hwe ? mx(rd_w, WR2RD) : rd_w);
      wr_w <= dec2(cas_go && !hwe ? mx(wr_w, RD2WR) : wr_w);
      ref_w <= dec2(pre_all ? T_RPA + 1'b1 : ref_w);
      acts <= {acts, do_act};
      refi <= tick ? REFI - 1'b1 : refi - 1'b1;
      owed <= owed + tick - refresh;
      if (ref_go) ref_mode <= 1'b1; else if (refresh) ref_mode <= 1'b0;
      {dfi_p0_cs_n, dfi_p0_ras_n, dfi_p0_cas_n, dfi_p0_we_n} <= cas_go ? (hwe ? WR : RD) : NOP;
      dfi_p0_wrdata_en <= cas_go && hwe; dfi_p1_wrdata_en <= cas_go && hwe;
      dfi_p0_rddata_en <= cas_go && !hwe; dfi_p1_rddata_en <= 1'b0;
      {dfi_p1_cs_n, dfi_p1_ras_n, dfi_p1_cas_n, dfi_p1_we_n} <= do_act ? ACT : pre_all ? PRE : refresh ? REF : NOP;
      rdata_valid <= dfi_p0_rddata_valid;
    end
  end

  always @(posedge sys_clk) begin
    if (cmd_valid && !full) begin
      q_we[tail[QB-1:0]] <= cmd_payload_we; q_col[tail[QB-1:0]] <= cmd_payload_addr[7:0];
      q_bank[tail[QB-1:0]] <= cmd_payload_addr[10:8]; q_row[tail[QB-1:0]] <= cmd_payload_addr[23:11];
    end
    if (wdata_valid && wdata_ready) begin w_data <= wdata_payload_data; w_we <= wdata_payload_we; end
    dfi_p0_bank <= hb; dfi_p0_address <= {2'b00, ap, hcol, 2'b00};
    dfi_p0_wrdata <= w_data[31:0]; dfi_p1_wrdata <= w_data[63:32];
    dfi_p0_wrdata_mask <= ~w_we[3:0]; dfi_p1_wrdata_mask <= ~w_we[7:4];
    dfi_p1_bank <= pb; dfi_p1_address <= do_act ? pr : {2'b00, pre_all, 10'd0};
    rdata_payload_data <= {dfi_p1_rddata, dfi_p0_rddata};
  end
endmodule
"#;
    let pairs: [(&str, String); 25] = [
        ("%PART%", g.part.to_string()),
        ("%TCK%", t.tck_ps.to_string()),
        ("%QB%", qb.to_string()),
        ("%CW%", cw.to_string()),
        ("%RCW%", rcw.to_string()),
        ("%T_RCD%", t.trcd.to_string()),
        ("%T_RP%", t.trp.to_string()),
        ("%T_RPA%", t.trpa.to_string()),
        ("%T_RAS%", t.tras.to_string()),
        ("%T_RC%", t.trc.to_string()),
        ("%T_RFC%", t.trfc.to_string()),
        ("%RD2PRE%", t.read_to_precharge().to_string()),
        ("%WR2PRE%", t.write_to_precharge().to_string()),
        ("%WR2RD%", t.write_to_read().to_string()),
        ("%RD2WR%", t.read_to_write().to_string()),
        ("%CAS2CAS%", t.cas_to_cas().to_string()),
        ("%RB%", bits_for(refi).to_string()),
        ("%REFI%", refi.to_string()),
        ("%POSTPONE%", REFRESH_POSTPONE_MAX.to_string()),
        ("%H%", history.to_string()),
        ("%FAW_OK%", faw_ok),
        ("%RRD_OK%", rrd_ok),
        ("%NOP%", pins(Ddr2Cmd::Nop)),
        ("%ACT%", pins(Ddr2Cmd::Active)),
        ("%PRE%", pins(Ddr2Cmd::Precharge)),
    ];
    let mut out = v.to_string();
    for (k, val) in pairs.iter().chain([("%REF%", pins(Ddr2Cmd::Refresh)), ("%RD%", pins(Ddr2Cmd::Read)), ("%WR%", pins(Ddr2Cmd::Write))].iter()) {
        out = out.replace(k, val);
    }
    assert!(!out.contains('%'), "an unfilled field in ddr2_dfi_ctrl:\n{out}");
    out
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/ddr2_dfi/tests.rs"]
mod tests;
