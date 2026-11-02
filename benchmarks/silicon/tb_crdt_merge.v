// tb_crdt_merge — HW G-Counter merge vs the software CRDT oracle (per-replica max).
`timescale 1ns/1ps
module tb_crdt_merge;
  reg  [127:0] a, b;
  reg  [127:0] oa [0:63];
  reg  [127:0] ob [0:63];
  reg  [127:0] om [0:63];
  wire [127:0] merged;
  wire [33:0]  value;
  reg  [33:0]  ev;
  integer i, errors = 0;
  crdt_merge dut(.a(a), .b(b), .merged(merged), .value(value));
  initial begin
    $readmemh("crdt_a.hex", oa);
    $readmemh("crdt_b.hex", ob);
    $readmemh("crdt_m.hex", om);
    for (i = 0; i < 64; i = i + 1) begin
      a = oa[i]; b = ob[i]; #1;
      ev = {2'b0, om[i][31:0]} + {2'b0, om[i][63:32]} + {2'b0, om[i][95:64]} + {2'b0, om[i][127:96]};
      if (merged !== om[i]) begin errors = errors + 1; $display("FAIL merged %0d", i); end
      if (value  !== ev)    begin errors = errors + 1; $display("FAIL value %0d", i); end
    end
    if (errors == 0) $display("PASS: 64 CRDT merges (per-replica max join + value) match the software oracle");
    else $display("FAIL: %0d CRDT merge mismatches", errors);
    $finish;
  end
endmodule
