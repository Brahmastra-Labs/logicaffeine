// The SECDED line coder and decoder against vectors the code's Rust definition wrote: every line coded as secded_line
// codes it (enc.hex: {bus, data}), every bus line read as secded_read reads it (dec.hex: {uncorrectable, corrected,
// data, bus}).
`timescale 1ns/1ps
module tb;
  parameter NENC = 1, NDEC = 1;
  reg  [1087:0] enc [0:NENC-1];
  reg  [1103:0] dec [0:NDEC-1];
  reg  [511:0]  d = 0;
  reg  [575:0]  r = 0;
  wire [575:0]  bus;
  wire [511:0]  q;
  wire [7:0]    corrected, uncorrectable;
  logos_secded_encode coder(.d(d), .bus(bus));
  logos_secded_decode decoder(.bus(r), .d(q), .corrected(corrected), .uncorrectable(uncorrectable));
  integer i, errors = 0;
  initial begin
    $readmemh("enc.hex", enc);
    $readmemh("dec.hex", dec);
    for (i = 0; i < NENC; i = i + 1) begin
      d = enc[i][511:0];
      #1;
      if (bus !== enc[i][1087:512]) begin
        errors = errors + 1;
        if (errors < 8) $display("FAIL coding vector %0d", i);
      end
    end
    for (i = 0; i < NDEC; i = i + 1) begin
      r = dec[i][575:0];
      #1;
      if ({uncorrectable, corrected, q} !== dec[i][1103:576]) begin
        errors = errors + 1;
        if (errors < 8) $display("FAIL reading vector %0d: corrected %h uncorrectable %h", i, corrected, uncorrectable);
      end
    end
    $display("RESULT coded=%0d read=%0d errors=%0d", NENC, NDEC, errors);
    $finish;
  end
endmodule
