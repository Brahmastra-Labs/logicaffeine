//! The DDR2 as `hart_ddr2` meets it, for simulation: the MT47H64M16 behind the PHY, at the controller's frame
//! interface (a command in each frame's first CK cycle, four data beats a frame). The model holds data, and refuses —
//! naming the rule — every command the part's data sheet forbids: the power-up order and its waits, the mode
//! registers the controller relies on (BL8, CAS latency, DLL), ACTIVATE / READ / WRITE / PRECHARGE / REFRESH against
//! tRCD, tRP, tRAS, tRC, tRRD, tWR, tWTR, tRTP, tRFC and the refresh interval, auto-precharge as JEDEC times it,
//! and a write's DQS preamble, data and postamble exactly where the write latency puts them. Its timing comes from
//! `hart::ddr2::MT47H64M16` in CK cycles, never from the controller's frame schedule.
//!
//! Reads come back the way a board returns them: each byte lane `SKEW` beats after the CAS latency, inverted or not,
//! through an input delay whose tap moves the sampling point — `TAPS_PER_BEAT` taps to a beat, garbage within
//! `MARGIN` taps of a beat's edge — so calibration has a real eye to find. Everything not driven reads as noise. The
//! eye can drift after calibration (`drift` taps, either way), as a board's does with temperature: a controller that
//! settled at the edge of its eye instead of the middle then reads garbage.
#![allow(dead_code)]

use logicaffeine_compile::codegen_sva::hart::ddr2::{cks, Part, CK_PS};

/// How one simulated board returns reads: per lane, the beats after the CAS latency the data lands, where its eye
/// sits against the delay taps, whether the lane reads inverted, and how far the eye drifts once calibrated.
#[derive(Clone, Copy, Debug)]
pub struct Board {
    pub skew: [u32; 2],
    pub phase: [u32; 2],
    pub invert: [bool; 2],
    pub drift: i32,
    /// No eye at all: every read is noise.
    pub blind: bool,
}

pub const TAPS_PER_BEAT: u32 = 51;
pub const MARGIN: u32 = 6;

pub fn model(p: &Part) -> String {
    format!(
        r#"
module ddr2_device #(
  parameter [7:0] SKEW0 = 8'd0, parameter [7:0] SKEW1 = 8'd0,
  parameter [7:0] PHASE0 = 8'd0, parameter [7:0] PHASE1 = 8'd0,
  parameter INV0 = 0, parameter INV1 = 0, parameter BLIND = 0, parameter SEED = 1
) (
  input             clk,
  input             rst,
  input             cke,
  input             cs_n,
  input             ras_n,
  input             cas_n,
  input             we_n,
  input      [2:0]  ba,
  input      [12:0] a,
  input             dq_oe,
  input      [63:0] dq_out,
  input             dqs_oe,
  input             dqs_toggle,
  output reg [63:0] rd_frame,
  input             dly_ld,
  input      [9:0]  dly_tap,
  input signed [31:0] drift,
  output reg        error,
  output reg [31:0] refreshes,
  output reg [31:0] lines_read,
  output reg [31:0] lines_written
);
  localparam TRCD = {trcd}, TRP = {trp}, TRAS = {tras}, TRC = {trc}, TRRD = {trrd}, TWR = {twr}, TWTR = {twtr},
             TRTP = {trtp}, TRFC = {trfc}, TREFI = {trefi}, POWER_UP = {power_up}, CKE_SETTLE = {cke_settle},
             TMRD = {tmrd}, TDLLK = {tdllk};
  localparam TPB = {taps_per_beat}, MARGIN = {margin};
  integer seed = SEED;
  reg [63:0] f;                 // frames since reset; a command in frame f is at CK 2f
  wire [63:0] now = 2 * f;

  // ── what the part has been told ──
  reg [3:0]  init_step;         // 0..10 through the power-up order, 12 when done
  reg [63:0] cke_low_from, cke_high_at, last_mrs, dll_reset_at;
  reg        mrs_seen;
  reg        cke_was;
  reg [2:0]  cl;
  reg [3:0]  wr;
  reg        bl8, dll_on;
  // ── the banks ──
  reg        open [0:7];
  reg [12:0] row [0:7];
  reg [63:0] act_at [0:7], idle_at [0:7];
  reg        act_seen [0:7];
  reg [63:0] last_act, last_cas, last_wdata_end, ref_busy_until, last_refresh;
  reg        acted, cased, wrote;
  // ── the data ──
  reg [15:0] mem [0:131071];    // rows 0..15 of every bank
  // write data expected at a frame: {{valid, bank, row index, column}}
  reg        wf_valid [0:63];
  reg [16:0] wf_word [0:63];
  // each lane's coming read beats, by half-CK beat index mod 512
  reg [7:0]  beat0 [0:511];
  reg [7:0]  beat1 [0:511];
  reg        live0 [0:511];
  reg        live1 [0:511];
  reg [4:0]  tap0, tap1;

  integer i, k;
  task fail(input [8*72-1:0] why);
    begin
      if (!error) $display("DDR2 MODEL: %0s at CK %0d", why, now);
      error <= 1'b1;
    end
  endtask

  function bank_idle(input [2:0] b);
    bank_idle = !open[b] && idle_at[b] <= now;
  endfunction

  // A lane's sample at half-CK beat index q, through tap t: the beat whose data window holds the sample point, and
  // `noise` near a beat's edge or where no read put data.
  function [7:0] sample(input lane, input [63:0] q, input [4:0] t, input [7:0] noise);
    integer at;
    reg [63:0] src;
    begin
      at = t + (lane ? PHASE1 : PHASE0) + drift;
      src = q - at / TPB;
      if (BLIND || at < 0 || (at % TPB) < MARGIN || (at % TPB) > TPB - MARGIN)
        sample = noise;
      else if (lane ? live1[src % 512] : live0[src % 512])
        sample = (lane ? beat1[src % 512] : beat0[src % 512]) ^ ((lane ? INV1 : INV0) ? 8'hff : 8'h00);
      else
        sample = noise;
    end
  endfunction

  always @(posedge clk) begin
    if (rst) begin
      f <= 64'd0;
      error <= 1'b0;
      init_step <= 4'd0;
      cke_was <= 1'b0;
      cke_low_from <= 64'd0;
      bl8 <= 1'b0; dll_on <= 1'b0; cl <= 3'd0; wr <= 4'd0;
      acted <= 1'b0; cased <= 1'b0; wrote <= 1'b0; mrs_seen <= 1'b0;
      ref_busy_until <= 64'd0; dll_reset_at <= 64'd0;
      refreshes <= 32'd0; lines_read <= 32'd0; lines_written <= 32'd0;
      tap0 <= 5'd0; tap1 <= 5'd0;
      for (i = 0; i < 8; i = i + 1) begin
        open[i] <= 1'b0; idle_at[i] <= 64'd0; act_seen[i] <= 1'b0;
      end
      for (i = 0; i < 64; i = i + 1) wf_valid[i] <= 1'b0;
      for (i = 0; i < 512; i = i + 1) begin live0[i] <= 1'b0; live1[i] <= 1'b0; end
    end else begin
      f <= f + 64'd1;
      if (dly_ld) begin
        tap0 <= dly_tap[4:0];
        tap1 <= dly_tap[9:5];
      end

      // ── CKE and the power-up wait ──
      if (cke && !cke_was) begin
        if (now - cke_low_from < POWER_UP) fail("CKE rose before the power-up wait");
        cke_high_at <= now;
      end
      cke_was <= cke;

      // ── a command ──
      if (cke && !cs_n && !(ras_n && cas_n && we_n)) begin
        if (now - cke_high_at < CKE_SETTLE) fail("a command within 400 ns of CKE rising");
        if (now < ref_busy_until) fail("a command within tRFC of REFRESH");
        if (mrs_seen && now - last_mrs < TMRD) fail("tMRD");
        case ({{ras_n, cas_n, we_n}})
          3'b000: begin // LOAD MODE
            last_mrs <= now;
            mrs_seen <= 1'b1;
            case (init_step)
              4'd1: if (ba != 3'd2) fail("the second power-up command is not EMR2");
              4'd2: if (ba != 3'd3) fail("the third power-up command is not EMR3");
              4'd3: if (ba != 3'd1 || a[0]) fail("the fourth power-up command does not enable the DLL");
              4'd4: if (ba != 3'd0 || !a[8]) fail("the fifth power-up command does not reset the DLL");
              4'd8: if (ba != 3'd0 || a[8]) fail("the ninth power-up command is not the operating MR");
              4'd9: if (ba != 3'd1 || a[9:7] != 3'b111) fail("OCD default does not follow the operating MR");
              4'd10: if (ba != 3'd1 || a[9:7] != 3'b000) fail("OCD exit does not follow OCD default");
              default: fail("a mode register load out of the power-up order");
            endcase
            if (ba == 3'd0) begin
              bl8 <= a[2:0] == 3'b011 && !a[3];
              cl <= a[6:4];
              wr <= {{1'b0, a[11:9]}} + 4'd1;
              if (a[8]) dll_reset_at <= now;
              if (a[2:0] != 3'b011 || a[3]) fail("the mode register is not BL8, sequential");
              if (a[6:4] < 3'd3 || a[6:4] > 3'd6 || a[6:4] % 2 == 0) fail("a CAS latency whose write latency is not whole frames");
            end
            if (ba == 3'd1) begin
              dll_on <= !a[0];
              if (a[5:3] != 3'd0) fail("additive latency");
            end
            if (init_step == 4'd10) begin
              init_step <= 4'd12;
              last_refresh <= now;
            end else init_step <= init_step + 4'd1;
          end
          3'b010: begin // PRECHARGE
            if (a[10]) begin
              if (init_step == 4'd0) init_step <= 4'd1;
              else if (init_step == 4'd5) init_step <= 4'd6;
              else if (init_step != 4'd12) fail("PRECHARGE ALL out of the power-up order");
              for (i = 0; i < 8; i = i + 1) if (open[i]) begin
                if (now - act_at[i] < TRAS) fail("tRAS");
                open[i] <= 1'b0;
                idle_at[i] <= now + TRP;
              end
            end else fail("a single-bank PRECHARGE (the controller precharges automatically)");
          end
          3'b001: begin // REFRESH
            if (init_step == 4'd6 || init_step == 4'd7) init_step <= init_step + 4'd1;
            else if (init_step != 4'd12) fail("REFRESH out of the power-up order");
            for (i = 0; i < 8; i = i + 1) if (!bank_idle(i)) fail("REFRESH with a bank not idle");
            ref_busy_until <= now + TRFC;
            last_refresh <= now;
            if (init_step == 4'd12) refreshes <= refreshes + 32'd1;
          end
          3'b011: begin // ACTIVATE
            if (init_step != 4'd12) fail("ACTIVATE before the power-up sequence finished");
            if (!bank_idle(ba)) fail("ACTIVATE on a bank not idle (tRP, or open)");
            if (act_seen[ba] && now - act_at[ba] < TRC) fail("tRC");
            if (acted && now - last_act < TRRD) fail("tRRD");
            if (a > 13'd15) fail("a row the model does not hold");
            open[ba] <= 1'b1;
            row[ba] <= a;
            act_at[ba] <= now;
            act_seen[ba] <= 1'b1;
            last_act <= now;
            acted <= 1'b1;
          end
          3'b101, 3'b100: begin // READ, WRITE
            if (!open[ba]) fail("READ or WRITE on a closed bank");
            if (now - act_at[ba] < TRCD) fail("tRCD");
            if (cased && now - last_cas < 4) fail("column commands closer than a BL8 burst");
            if (a[2:0] != 3'd0) fail("a burst not aligned to eight columns");
            if (now - dll_reset_at < TDLLK) fail("a column command before the DLL locked");
            last_cas <= now;
            cased <= 1'b1;
            if (!we_n) begin // WRITE: its data in the frames WL puts it
              k = (now + cl - 1) / 2;
              if ((now + cl - 1) % 2 != 0) fail("write data not frame-aligned");
              wf_valid[k % 64] <= 1'b1;
              wf_word[k % 64] <= {{row[ba][3:0], ba, a[9:0]}};
              wf_valid[(k + 1) % 64] <= 1'b1;
              wf_word[(k + 1) % 64] <= {{row[ba][3:0], ba, a[9:0] + 10'd4}};
              last_wdata_end <= now + cl - 1 + 4;
              wrote <= 1'b1;
              if (a[4:3] == 2'd0) lines_written <= lines_written + 32'd1;
              if (a[10]) begin
                open[ba] <= 1'b0;
                idle_at[ba] <= ((now + cl - 1 + 4 + wr > act_at[ba] + TRAS) ? now + cl - 1 + 4 + wr : act_at[ba] + TRAS) + TRP;
              end
            end else begin // READ: its eight beats CL after, each lane where the board puts it
              if (wrote && now < last_wdata_end + TWTR) fail("tWTR");
              for (i = 0; i < 8; i = i + 1) begin
                beat0[(2 * (now + cl) + SKEW0 + i) % 512] <= mem[{{row[ba][3:0], ba, a[9:0] + i[9:0]}}][7:0];
                live0[(2 * (now + cl) + SKEW0 + i) % 512] <= 1'b1;
                beat1[(2 * (now + cl) + SKEW1 + i) % 512] <= mem[{{row[ba][3:0], ba, a[9:0] + i[9:0]}}][15:8];
                live1[(2 * (now + cl) + SKEW1 + i) % 512] <= 1'b1;
              end
              if (a[4:3] == 2'd0) lines_read <= lines_read + 32'd1;
              if (a[10]) begin
                open[ba] <= 1'b0;
                idle_at[ba] <= ((now + 2 + (TRTP > 2 ? TRTP : 2) > act_at[ba] + TRAS)
                                ? now + 2 + (TRTP > 2 ? TRTP : 2) : act_at[ba] + TRAS) + TRP;
              end
            end
          end
          default: ;
        endcase
      end
      if (init_step == 4'd12 && now - last_refresh > TREFI) fail("no REFRESH within tREFI");

      // ── the write data bus, a frame at a time ──
      if (wf_valid[f % 64]) begin
        if (!(dq_oe && dqs_oe && dqs_toggle)) fail("write data missing where the write latency puts it");
        for (i = 0; i < 4; i = i + 1)
          mem[{{wf_word[f % 64][16:10], wf_word[f % 64][9:0] + i[9:0]}}] <= dq_out[16 * i +: 16];
        wf_valid[f % 64] <= 1'b0;
      end else begin
        if (dq_oe) fail("DQ driven outside a write's data");
        if (dqs_toggle) fail("DQS toggling outside a write's data");
        if (wf_valid[(f + 1) % 64] && !dqs_oe) fail("no DQS preamble before write data");
        if (wf_valid[(f + 63) % 64] && !dqs_oe) fail("no DQS postamble after write data");
      end
      if (dq_oe && (live0[(4 * f) % 512] || live1[(4 * f) % 512])) fail("the controller drives DQ over read data");

      // ── the read stream: four beats a frame, through each lane's delay ──
      for (i = 0; i < 4; i = i + 1) begin
        rd_frame[16 * i +: 8] <= sample(1'b0, 4 * f + i, tap0, $random(seed));
        rd_frame[16 * i + 8 +: 8] <= sample(1'b1, 4 * f + i, tap1, $random(seed));
      end
      for (i = 0; i < 4; i = i + 1) begin
        live0[(4 * f + i + 256) % 512] <= 1'b0;
        live1[(4 * f + i + 256) % 512] <= 1'b0;
      end
      if (!cke && cke_was) cke_low_from <= now;
    end
  end
endmodule
"#,
        trcd = cks(p.trcd),
        trp = cks(p.trp),
        tras = cks(p.tras),
        trc = cks(p.trc),
        trrd = cks(p.trrd),
        twr = cks(p.twr),
        twtr = cks(p.twtr),
        trtp = cks(p.trtp),
        trfc = cks(p.trfc),
        trefi = p.trefi / CK_PS,
        power_up = cks(p.power_up),
        cke_settle = cks(p.cke_settle),
        tmrd = p.tmrd_ck,
        tdllk = p.tdllk_ck,
        taps_per_beat = TAPS_PER_BEAT,
        margin = MARGIN,
    )
}
