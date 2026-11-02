// Drop-in demo: a small combinational design with genuine logic DEPTH, so that
// after synthesis (synth_xilinx -nowidelut, 6-input LUTs) there are LUT-to-LUT
// nets our router owns — not just IO nets. The 8-input reductions (^sw, &sw, |sw)
// each decompose into a two-LUT cascade, and they fan out to many outputs, so the
// intermediate signals must route between LUTs across the INT switchbox.
//
// LEDs directly show f(switches): purely combinational, no clock (no timing risk).
module board_top(input [7:0] sw, output [7:0] led);
  wire par   = ^sw;                       // 8-input XOR  -> 2 LUT6 cascade
  wire allon = &sw;                       // 8-input AND  -> 2 LUT6 cascade
  wire anyon = |sw;                       // 8-input OR   -> 2 LUT6 cascade
  wire [7:0] mix = sw ^ {sw[3:0], sw[7:4]};

  assign led[0] = par ^ sw[0];
  assign led[1] = allon ^ mix[1];
  assign led[2] = par & anyon;
  assign led[3] = par ^ allon ^ mix[3];
  assign led[4] = (par & sw[4]) | (allon & sw[5]) | (anyon & mix[4]);
  assign led[5] = anyon ^ par ^ mix[5];
  assign led[6] = allon | (par & sw[6]) | mix[6];
  assign led[7] = par ^ anyon ^ allon ^ mix[7];
endmodule
