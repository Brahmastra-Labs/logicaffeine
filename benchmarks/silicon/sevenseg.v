// sevenseg — the Nexys A7-100T 8-digit 7-segment display driver.
//
// Shows a 32-bit value in hexadecimal across the 8 digits (one nibble per digit), time-multiplexed off
// the 100 MHz board clock: a refresh counter walks the 8 common-anode digits, driving one anode low at a
// time and putting that digit's nibble on the shared cathode bus. Segments and anodes are ACTIVE-LOW
// (Digilent convention). `blank` forces all digits dark (used at reset before the first token).
//
// REFRESH_BITS sets the per-digit dwell (2^REFRESH_BITS core clocks); at 100 MHz, 17 bits ≈ 1.3 kHz per
// digit (well above flicker). The testbench overrides it small so the mux cycles in a few clocks.
module sevenseg #(parameter REFRESH_BITS = 17) (
  input             clk,
  input             rst,
  input      [31:0] val,        // the value to display (hex)
  input             blank,      // 1 = all digits dark
  output reg [6:0]  seg,        // active-low cathodes {g,f,e,d,c,b,a} -> CG..CA
  output reg        dp,         // active-low decimal point (kept off)
  output reg [7:0]  an          // active-low digit anodes (one-hot-low)
);
  reg [REFRESH_BITS+2:0] refresh;
  wire [2:0] digit = refresh[REFRESH_BITS+2:REFRESH_BITS];  // which of the 8 digits is active

  // the active digit's nibble
  reg [3:0] nib;
  always @(*) begin
    case (digit)
      3'd0: nib = val[3:0];
      3'd1: nib = val[7:4];
      3'd2: nib = val[11:8];
      3'd3: nib = val[15:12];
      3'd4: nib = val[19:16];
      3'd5: nib = val[23:20];
      3'd6: nib = val[27:24];
      default: nib = val[31:28];
    endcase
  end

  // nibble -> active-low 7-seg pattern, seg = {g,f,e,d,c,b,a}
  function [6:0] hex7;
    input [3:0] h;
    case (h)
      4'h0: hex7 = 7'b1000000;
      4'h1: hex7 = 7'b1111001;
      4'h2: hex7 = 7'b0100100;
      4'h3: hex7 = 7'b0110000;
      4'h4: hex7 = 7'b0011001;
      4'h5: hex7 = 7'b0010010;
      4'h6: hex7 = 7'b0000010;
      4'h7: hex7 = 7'b1111000;
      4'h8: hex7 = 7'b0000000;
      4'h9: hex7 = 7'b0010000;
      4'hA: hex7 = 7'b0001000;
      4'hB: hex7 = 7'b0000011;
      4'hC: hex7 = 7'b1000110;
      4'hD: hex7 = 7'b0100001;
      4'hE: hex7 = 7'b0000110;
      default: hex7 = 7'b0001110; // F
    endcase
  endfunction

  always @(posedge clk) begin
    if (rst) begin
      refresh <= 0;
      seg <= 7'b1111111;   // all cathodes off
      dp  <= 1'b1;
      an  <= 8'hFF;        // all anodes off (dark)
    end else begin
      refresh <= refresh + 1'b1;
      dp <= 1'b1;
      if (blank) begin
        seg <= 7'b1111111;
        an  <= 8'hFF;
      end else begin
        seg <= hex7(nib);
        an  <= ~(8'd1 << digit);   // active-low one-hot: pull the current digit's anode low
      end
    end
  end
endmodule
