// Behavioral models of the two Xilinx primitives the Fabric Terminal's top instantiates, for iverilog: a
// tristate pad buffer and a global clock buffer. Both are what the 7-series library documents: IOBUF drives
// IO with I while T is low and floats it while T is high, and reads IO back on O; BUFG passes its input through.
`timescale 1ns/1ps
module IOBUF (inout IO, input I, input T, output O);
  assign IO = T ? 1'bz : I;
  assign O = IO;
endmodule

module BUFG (input I, output O);
  assign O = I;
endmodule
