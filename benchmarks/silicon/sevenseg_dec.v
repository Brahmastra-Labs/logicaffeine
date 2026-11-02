// sevenseg_dec — the Nexys A7-100T 8-digit 7-segment display driver, SIGNED DECIMAL. Shows a 32-bit signed
// value as decimal across the 8 digits: magnitude via double-dabble (binary→BCD), a leading minus for
// negatives, leading zeros blanked, and the value 0 shown as a lit "0" on the rightmost digit — so the
// display is ALWAYS lit (never fully dark), which is the point on stage. Time-multiplexed off the 100 MHz
// board clock; segments + anodes ACTIVE-LOW (Digilent convention). Same refresh/anode structure as the
// proven `sevenseg.v`; only the per-digit character selection differs (decimal + sign + blank).
module sevenseg_dec #(parameter REFRESH_BITS = 17) (
  input             clk,
  input             rst,
  input      [31:0] val,        // the value to display (interpreted signed)
  output reg [6:0]  seg,        // active-low cathodes {g,f,e,d,c,b,a} -> CG..CA
  output reg        dp,         // active-low decimal point (kept off)
  output reg [7:0]  an          // active-low digit anodes (one-hot-low)
);
  reg [REFRESH_BITS+2:0] refresh;
  wire [2:0] digit = refresh[REFRESH_BITS+2:REFRESH_BITS];

  wire        neg = val[31];
  wire [31:0] mag = neg ? (~val + 32'd1) : val;

  // SEQUENTIAL double-dabble: one shift/add-3 step per clock (a short register-to-register path, so the
  // display logic closes timing cleanly at the 100 MHz board clock — no combinational 32-stage chain). The
  // conversion free-runs (~34 clocks) and latches into `bcd`/`neg_r`; the value updates rarely, so `bcd` is
  // always fresh long before the ~kHz refresh reads it.
  function [3:0] d3; input [3:0] n; d3 = (n >= 4'd5) ? (n + 4'd3) : n; endfunction
  reg [5:0]  step;
  reg [31:0] sh;
  reg [31:0] bw;
  reg [31:0] bcd; reg neg_r;
  wire [31:0] adj = {d3(bw[31:28]), d3(bw[27:24]), d3(bw[23:20]), d3(bw[19:16]),
                     d3(bw[15:12]), d3(bw[11:8]),  d3(bw[7:4]),   d3(bw[3:0])};
  always @(posedge clk) begin
    if (rst) begin step <= 6'd0; sh <= 32'd0; bw <= 32'd0; bcd <= 32'd0; neg_r <= 1'b0; end
    else if (step == 6'd0) begin sh <= mag; bw <= 32'd0; neg_r <= neg; step <= 6'd1; end
    else if (step <= 6'd32) begin bw <= {adj[30:0], sh[31]}; sh <= sh << 1; step <= step + 6'd1; end
    else begin bcd <= bw; step <= 6'd0; end
  end

  // most-significant nonzero digit (default 0 so zero shows a lit "0").
  reg [2:0] msd;
  always @(*) begin
    msd = 3'd0;
    if (bcd[7:4]   != 4'd0) msd = 3'd1;
    if (bcd[11:8]  != 4'd0) msd = 3'd2;
    if (bcd[15:12] != 4'd0) msd = 3'd3;
    if (bcd[19:16] != 4'd0) msd = 3'd4;
    if (bcd[23:20] != 4'd0) msd = 3'd5;
    if (bcd[27:24] != 4'd0) msd = 3'd6;
    if (bcd[31:28] != 4'd0) msd = 3'd7;
  end

  reg [3:0] nib; reg is_blank; reg is_minus;
  always @(*) begin
    case (digit)
      3'd0: nib = bcd[3:0];   3'd1: nib = bcd[7:4];   3'd2: nib = bcd[11:8];  3'd3: nib = bcd[15:12];
      3'd4: nib = bcd[19:16]; 3'd5: nib = bcd[23:20]; 3'd6: nib = bcd[27:24]; default: nib = bcd[31:28];
    endcase
    is_minus = neg_r && (digit == msd + 3'd1) && (msd != 3'd7);
    is_blank = (digit > msd) && !is_minus;
  end

  // decimal digit (0-9) → active-low 7-seg, seg = {g,f,e,d,c,b,a}
  function [6:0] dec7;
    input [3:0] h;
    case (h)
      4'd0: dec7 = 7'b1000000; 4'd1: dec7 = 7'b1111001; 4'd2: dec7 = 7'b0100100; 4'd3: dec7 = 7'b0110000;
      4'd4: dec7 = 7'b0011001; 4'd5: dec7 = 7'b0010010; 4'd6: dec7 = 7'b0000010; 4'd7: dec7 = 7'b1111000;
      4'd8: dec7 = 7'b0000000; default: dec7 = 7'b0010000; // 9
    endcase
  endfunction

  always @(posedge clk) begin
    if (rst) begin
      refresh <= 0; seg <= 7'b1111111; dp <= 1'b1; an <= 8'hFF;
    end else begin
      refresh <= refresh + 1'b1;
      dp <= 1'b1;
      if (is_blank)      seg <= 7'b1111111;      // dark
      else if (is_minus) seg <= 7'b0111111;      // minus sign (segment g only)
      else               seg <= dec7(nib);       // a decimal digit
      an <= ~(8'd1 << digit);                     // active-low one-hot: light this digit
    end
  end
endmodule
