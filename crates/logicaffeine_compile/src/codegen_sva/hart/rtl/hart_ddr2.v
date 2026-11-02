// hart_ddr2 — the hart's main memory controller: 64-byte lines in and out of the board's DDR2, one hart clock (a
// frame: two CK cycles, four data beats) at a time, on the schedule `hart::ddr2` computes (hart_ddr2_params.vh).
//
// After reset it walks the JEDEC power-up sequence, then calibrates its read capture: it writes a known line, and
// for every input delay tap reads it back raw and finds where in the captured stream each byte lane's first beat (or
// its complement) lands, confirming the other thirty-one; each lane takes the middle of its widest run of taps that
// agree on place and polarity. A read of the line through the calibrated path must then return it exactly, or the
// controller reports that calibration failed and serves nothing. Refresh runs between transactions.
//
// A line is a closed-page transaction: ACTIVATE, four BL8 READs or WRITEs CCD frames apart (the last precharging
// automatically), the bank idle by READ_DONE / WRITE_DONE. A write's eight words are taken first, then written: DQS
// low for a preamble frame, eight frames of data starting two frames after the first WRITE, low for a postamble
// frame. A read returns its eight words in order, a frame apart, once the calibrated latency has passed.
//
// The PHY turns each frame into pins: the command in the frame's first CK cycle (a deselect in its second), CK, DQS
// and the commands through serializers on one clock so their relative timing is fixed by construction, DQ a quarter
// CK later, and the read stream's four beats a frame in arrival order.
`include "hart_ddr2_params.vh"
module hart_ddr2 (
  input             clk,
  input             rst,
  // the line port
  input             req,
  input             we,
  input      [63:0] addr,
  output            gnt,
  input      [63:0] wdata,
  input             wvalid,
  output            wready,
  output reg        rvalid,
  output reg [63:0] rdata,
  // bit 0 calibrated, bit 1 failed, bits 15:8 the read latency in frames
  output     [63:0] status,
  // each lane's {tap, beat position, polarity}: lane 0 in bits 11:0, lane 1 in bits 27:16
  output     [63:0] cal,
  // the PHY, a frame at a time
  output reg        cke,
  output reg        cs_n,
  output reg        ras_n,
  output reg        cas_n,
  output reg        we_n,
  output reg [2:0]  ba,
  output reg [12:0] a,
  output reg        dq_oe,
  output reg [63:0] dq_out,
  output reg        dqs_oe,
  output reg        dqs_toggle,
  input      [63:0] rd_frame,
  output reg        dly_ld,
  output reg [9:0]  dly_tap
);
  localparam RCD = `DDR2_RCD, CCD = `DDR2_CCD;
  localparam [4:0] LAST_CAS = RCD + 3 * CCD;

  reg [44:0] rom [0:`DDR2_INIT_STEPS - 1];
  reg [63:0] calline [0:7];
  initial begin
    `DDR2_INIT_ROM(rom)
    `DDR2_CAL_LINE(calline)
  end

  localparam [3:0] S_INIT = 4'd0, S_CAL_WRITE = 4'd1, S_CAL_TAP = 4'd2, S_CAL_READ = 4'd3, S_CAL_SCAN = 4'd4,
                   S_CAL_PICK = 4'd5, S_CAL_SET = 4'd6, S_CAL_CHECK = 4'd7, S_CAL_VERDICT = 4'd8, S_READY = 4'd9,
                   S_TAKE = 4'd10, S_WRITE = 4'd11, S_FAILED = 4'd12;
  reg [3:0]  state;
  reg [5:0]  step;
  reg [23:0] wait_left;
  reg        calibrated, failed;

  // ── the line transaction: x_active while it runs, t its frame from ACTIVATE ─────────────────────────────────
  reg        x_active, x_we, x_raw, x_verify;
  reg [12:0] x_row;
  reg [2:0]  x_bank;
  reg [4:0]  x_cols;          // the line's column base, in units of 32 columns
  reg [4:0]  t;
  (* ram_style = "registers" *) reg [63:0] wbuf [0:7];
  reg [3:0]  taken;
  reg [4:0]  lat;             // the frame (from ACTIVATE) the first aligned word of a read is ready
  wire [4:0] raw_done = (`DDR2_READ_DONE > RCD + 16) ? `DDR2_READ_DONE : RCD + 16;
  wire [4:0] read_done = (`DDR2_READ_DONE > lat + 8) ? `DDR2_READ_DONE : lat + 8;
  wire [4:0] x_done = x_we ? `DDR2_WRITE_DONE : x_raw ? raw_done : read_done;
  wire [4:0] burst = (t - RCD) / CCD;
  wire       cas_now = t >= RCD && t <= LAST_CAS && (t - RCD) % CCD == 0;

  // Refresh: due every REFI frames once the part is up; issued only between transactions.
  reg [15:0] refi;
  reg        ref_due;
  reg [4:0]  ref_left;
  wire       bus_free = !x_active && ref_left == 5'd0;
  wire       may_start = bus_free && !ref_due;

  // ── read capture: the raw stream (calibration) and each lane's last sixteen beats (every read) ──────────────
  (* ram_style = "registers" *) reg [63:0] cap [0:15];
  reg [127:0] hist0, hist1;   // oldest beat in the low byte
  wire [31:0] frame0 = {rd_frame[55:48], rd_frame[39:32], rd_frame[23:16], rd_frame[7:0]};
  wire [31:0] frame1 = {rd_frame[63:56], rd_frame[47:40], rd_frame[31:24], rd_frame[15:8]};
  reg [3:0]  win0, win1;
  reg        inv0, inv1;
  wire [31:0] lane0 = (hist0 >> {win0, 3'b000}) ^ {32{inv0}};
  wire [31:0] lane1 = (hist1 >> {win1, 3'b000}) ^ {32{inv1}};
  wire [63:0] aligned = {lane1[31:24], lane0[31:24], lane1[23:16], lane0[23:16],
                         lane1[15:8], lane0[15:8], lane1[7:0], lane0[7:0]};

  // ── calibration ────────────────────────────────────────────────────────────────────────────────────────────
  reg [4:0]  tap;
  reg        lane;
  reg [5:0]  s;               // where the scan looks for the lane's first beat in the raw stream
  reg [4:0]  j;               // the beat it is confirming
  reg        s_inv;
  (* ram_style = "registers" *) reg [7:0] res0 [0:31];     // per tap: {found, inverted, position}
  (* ram_style = "registers" *) reg [7:0] res1 [0:31];
  reg [4:0]  pick;
  reg [5:0]  run_len, best_len;
  reg [4:0]  run_start, best_start;
  reg [6:0]  run_key, best_key;
  reg [4:0]  tap0, tap1;
  reg [5:0]  s0, s1;
  reg [3:0]  settle;
  reg        bad;

  function [7:0] raw_beat(input [5:0] i, input l);
    reg [63:0] f;
    begin
      f = cap[i[5:2]];
      raw_beat = f >> {i[1:0], l, 3'b000};
    end
  endfunction
  function [7:0] line_beat(input [4:0] i, input l);
    reg [63:0] f;
    begin
      f = calline[i[4:2]];
      line_beat = f >> {i[1:0], l, 3'b000};
    end
  endfunction
  wire [7:0] got = raw_beat(s + {1'b0, j}, lane);
  wire [7:0] want = line_beat(j, lane);
  wire       scan_end = s == 6'd32;

  // The run of taps the pick has reached, extended (or restarted) by this tap's result, and the widest so far.
  wire [7:0] r = lane ? res1[pick] : res0[pick];
  wire       continues = r[7] && run_len != 6'd0 && r[6:0] == run_key;
  wire [5:0] n_len = !r[7] ? 6'd0 : continues ? run_len + 6'd1 : 6'd1;
  wire [4:0] n_start = continues ? run_start : pick;
  wire       wider = n_len > best_len;
  wire [5:0] f_len = wider ? n_len : best_len;
  wire [4:0] f_start = wider ? n_start : best_start;
  wire [6:0] f_key = wider ? r[6:0] : best_key;
  wire [4:0] f_tap = f_start + ((f_len - 6'd1) >> 1);

  // The latency puts the later lane's first beat in its history's top word; the earlier lane must still be there.
  wire [5:0] s_max = (s0 > s1) ? s0 : s1;
  wire [4:0] lat_for = ((s_max + 6'd7) >> 2) + RCD;
  wire [8:0] w0 = {3'd0, s0} + 9'd16 + 4 * RCD - {2'd0, lat_for, 2'b00};
  wire [8:0] w1 = {3'd0, s1} + 9'd16 + 4 * RCD - {2'd0, lat_for, 2'b00};

  assign gnt = state == S_READY && may_start && req;
  assign wready = state == S_TAKE;
  assign status = {48'd0, 3'd0, lat, 6'd0, failed, calibrated};
  assign cal = {36'd0, tap1, s1, inv1, 4'd0, tap0, s0, inv0};

  // A lane's scan is over (at this tap): keep its result and go on to the other lane, or the next tap.
  task lane_scanned(input [7:0] result);
    begin
      if (lane) res1[tap] <= result; else res0[tap] <= result;
      s <= 6'd0;
      j <= 5'd0;
      lane <= !lane;
      if (lane) begin
        tap <= tap + 5'd1;
        settle <= 4'd0;
        state <= (tap == 5'd31) ? S_CAL_PICK : S_CAL_TAP;
        pick <= 5'd0;
        run_len <= 6'd0;
        best_len <= 6'd0;
      end
    end
  endtask

  task start(input write, input raw, input verify, input [12:0] row, input [2:0] bnk, input [4:0] cols);
    begin
      x_active <= 1'b1;
      x_we <= write;
      x_raw <= raw;
      x_verify <= verify;
      x_row <= row;
      x_bank <= bnk;
      x_cols <= cols;
      t <= 5'd0;
    end
  endtask

  integer ci;
  always @(posedge clk) begin
    // Between commands the bus holds NOP with nothing driven.
    cs_n <= 1'b0; ras_n <= 1'b1; cas_n <= 1'b1; we_n <= 1'b1; ba <= 3'd0; a <= 13'd0;
    dq_oe <= 1'b0; dqs_oe <= 1'b0; dqs_toggle <= 1'b0; dly_ld <= 1'b0; rvalid <= 1'b0;
    hist0 <= {frame0, hist0[127:32]};
    hist1 <= {frame1, hist1[127:32]};
    if (rst) begin
      state <= S_INIT;
      step <= 6'd0;
      wait_left <= rom[0][23:0];
      cke <= 1'b0;
      cs_n <= 1'b1;
      calibrated <= 1'b0;
      failed <= 1'b0;
      x_active <= 1'b0;
      refi <= 16'd0;
      ref_due <= 1'b0;
      ref_left <= 5'd0;
      lat <= 5'd0;
      tap0 <= 5'd0; tap1 <= 5'd0; s0 <= 6'd0; s1 <= 6'd0; inv0 <= 1'b0; inv1 <= 1'b0;
      win0 <= 4'd0; win1 <= 4'd0;
    end else begin
      // ── refresh ──
      if (state != S_INIT) begin
        if (refi == `DDR2_REFI) begin
          refi <= 16'd0;
          ref_due <= 1'b1;
        end else refi <= refi + 16'd1;
        if (ref_left != 5'd0) ref_left <= ref_left - 5'd1;
        else if (ref_due && !x_active) begin
          ras_n <= 1'b0;
          cas_n <= 1'b0;
          ref_due <= 1'b0;
          ref_left <= `DDR2_RFC;
        end
      end

      // ── the line transaction under way ──
      if (x_active) begin
        t <= t + 5'd1;
        if (t == 5'd0) begin
          ras_n <= 1'b0;
          ba <= x_bank;
          a <= x_row;
        end else if (cas_now) begin
          cas_n <= 1'b0;
          we_n <= !x_we;
          ba <= x_bank;
          a <= {2'b00, t == LAST_CAS, x_cols, burst[1:0], 3'b000};
        end
        if (x_we) begin
          if (t >= RCD + 1 && t <= RCD + 10) dqs_oe <= 1'b1;
          if (t >= RCD + 2 && t <= RCD + 9) begin
            dqs_toggle <= 1'b1;
            dq_oe <= 1'b1;
            dq_out <= wbuf[t - (RCD + 2)];
          end
        end else if (x_raw) begin
          if (t >= RCD && t < RCD + 16) cap[t - RCD] <= rd_frame;
        end else if (t >= lat && t < lat + 8) begin
          rvalid <= !x_verify;
          rdata <= aligned;
          if (x_verify && aligned != calline[t - lat]) bad <= 1'b1;
        end
        if (t == x_done) x_active <= 1'b0;
      end

      case (state)
        // ── power-up: each step drives its command for a frame, then waits its frames ──
        S_INIT: begin
          cke <= rom[step][44];
          if (wait_left != 24'd0) begin
            wait_left <= wait_left - 24'd1;
            cs_n <= !rom[step][44];
          end else begin
            cs_n <= rom[step][43]; ras_n <= rom[step][42]; cas_n <= rom[step][41]; we_n <= rom[step][40];
            ba <= rom[step][39:37]; a <= rom[step][36:24];
            if (step == `DDR2_INIT_STEPS - 1) begin
              for (ci = 0; ci < 8; ci = ci + 1) wbuf[ci] <= calline[ci];
              state <= S_CAL_WRITE;
            end else begin
              step <= step + 6'd1;
              wait_left <= rom[step + 6'd1][23:0];
            end
          end
        end

        // ── calibration ──
        S_CAL_WRITE: if (may_start) begin
          start(1'b1, 1'b0, 1'b0, 13'd0, 3'd0, 5'd0);
          tap <= 5'd0;
          lane <= 1'b0;
          s <= 6'd0;
          j <= 5'd0;
          settle <= 4'd0;
          state <= S_CAL_TAP;
        end
        S_CAL_TAP: if (!x_active) begin
          if (settle == 4'd0) begin
            dly_ld <= 1'b1;
            dly_tap <= {tap, tap};
          end
          settle <= settle + 4'd1;
          if (settle == 4'd15) state <= S_CAL_READ;
        end
        S_CAL_READ: if (may_start) begin
          start(1'b0, 1'b1, 1'b0, 13'd0, 3'd0, 5'd0);
          state <= S_CAL_SCAN;
        end
        // Does the line's first beat (or its complement) begin at s, and do the other thirty-one follow?
        S_CAL_SCAN: if (!x_active) begin
          if (j == 5'd0) begin
            if (got == want || got == ~want) begin
              s_inv <= got != want;
              j <= 5'd1;
            end else if (scan_end) lane_scanned(8'd0);
            else s <= s + 6'd1;
          end else if (got == (want ^ {8{s_inv}})) begin
            if (j == 5'd31) lane_scanned({1'b1, s_inv, s});
            else j <= j + 5'd1;
          end else if (scan_end) lane_scanned(8'd0);
          else begin
            j <= 5'd0;
            s <= s + 6'd1;
          end
        end
        // Each lane: the middle of its widest run of taps that found the line at one place and polarity.
        S_CAL_PICK: begin
          run_len <= n_len;
          run_start <= n_start;
          run_key <= r[6:0];
          if (wider) begin
            best_len <= n_len;
            best_start <= n_start;
            best_key <= r[6:0];
          end
          pick <= pick + 5'd1;
          if (pick == 5'd31) begin
            if (f_len < 6'd2) failed <= 1'b1;
            if (lane) begin tap1 <= f_tap; s1 <= f_key[5:0]; inv1 <= f_key[6]; end
            else begin tap0 <= f_tap; s0 <= f_key[5:0]; inv0 <= f_key[6]; end
            run_len <= 6'd0;
            best_len <= 6'd0;
            lane <= !lane;
            if (lane) state <= S_CAL_SET;
          end
        end
        S_CAL_SET: begin
          if (failed || w0 > 9'd12 || w1 > 9'd12) begin
            failed <= 1'b1;
            state <= S_FAILED;
          end else begin
            lat <= lat_for;
            win0 <= w0[3:0];
            win1 <= w1[3:0];
            dly_ld <= 1'b1;
            dly_tap <= {tap1, tap0};
            settle <= 4'd0;
            bad <= 1'b0;
            state <= S_CAL_CHECK;
          end
        end
        S_CAL_CHECK: begin
          settle <= settle + 4'd1;
          if (settle >= 4'd15 && may_start) begin
            start(1'b0, 1'b0, 1'b1, 13'd0, 3'd0, 5'd0);
            state <= S_CAL_VERDICT;
          end
        end
        S_CAL_VERDICT: if (!x_active) begin
          if (bad) begin
            failed <= 1'b1;
            state <= S_FAILED;
          end else begin
            calibrated <= 1'b1;
            state <= S_READY;
          end
        end

        // ── service ──
        S_READY: if (gnt) begin
          x_row <= addr[26:14];
          x_bank <= addr[13:11];
          x_cols <= addr[10:6];
          if (we) begin
            taken <= 4'd0;
            state <= S_TAKE;
          end else start(1'b0, 1'b0, 1'b0, addr[26:14], addr[13:11], addr[10:6]);
        end
        S_TAKE: if (wvalid) begin
          wbuf[taken[2:0]] <= wdata;
          taken <= taken + 4'd1;
          if (taken == 4'd7) state <= S_WRITE;
        end
        S_WRITE: if (may_start) begin
          start(1'b1, 1'b0, 1'b0, x_row, x_bank, x_cols);
          state <= S_READY;
        end
        default: ;
      endcase
    end
  end
endmodule
