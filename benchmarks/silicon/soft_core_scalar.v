`timescale 1ns/1ps
// soft_core_scalar.v — the ROUTABLE scalar LOGOS core.
//
// A fetch-decode-execute CPU over the SCALAR subset of the binisa ISA: the reversible/adiabatic core
// (wire opcodes 0..4), LoadConst/Move, the branches (Jump/JumpIfFalse/JumpIfTrue), Show/Halt/FailWith,
// Call/Return/ReturnNothing (register-window frame stack → real recursion), and every arithmetic /
// bitwise / comparison op via the our-SAT-proven `alu.v`. Instruction and opcode encodings are IDENTICAL
// to soft_core.v (chip word `[op:8][dst:8][a:8][b:8]`; LoadConst/Jump `[op][dst][imm:16]`; opcodes ARE
// binisa opcodes), so on a scalar program this core executes bit-identically to soft_core.v — and hence
// ≡ the VM (locked by the tb_scalar_*.v differential, whose vectors come from the run_soft_core model).
//
// It deliberately carries NO structured-collection coprocessor (no heap `dmem`, no microcode FSMs). That
// coprocessor is exactly what keeps the full soft_core.v from routing: its ASYNCHRONOUS multi-port dmem
// reads cannot map to synchronous ECP5 block RAM (see SILICON_COMPLETION.md E2d/E2f). With that excluded,
// this core PLACES-AND-ROUTES to a real ECP5 bitstream — the LOGOS CPU core, on silicon, running real
// bytecode. The sync-read/BRAM refactor that brings the collection ops onto fabric is E2f (in progress).
module soft_core_scalar #(parameter FRAMES = 8) (
  input clk, input rst,
`ifdef SOFT_CORE_SCALAR_PROG_LOAD
  input        prog_we,      // SoC program-load strobe: imem[prog_addr]<=prog_data (streamed over UART)
  input  [7:0] prog_addr,    //   256-word imem, guarded so the standalone core stays a $readmemh boot ROM
  input [31:0] prog_data,
`endif
  output reg [31:0] out, output reg out_valid, output reg halted
);
`ifdef SOFT_CORE_SCALAR_PROG_LOAD
  (* ram_style = "registers" *) reg [31:0] imem [0:255];   // FF-backed: the open flow mis-synthesizes a
                                                           // $readmemh-init'd, async-read, write-ported LUTRAM
`else
  reg [31:0] imem [0:255];
`endif
  reg [7:0]  pc;
  reg [31:0] regs [0:15];
  reg [31:0] savedregs [0:FRAMES-1][0:15];   // register-window save on Call
  reg [7:0]  retpc  [0:FRAMES-1];
  reg [3:0]  retdst [0:FRAMES-1];
  reg [4:0]  sp;
  reg        trapped;

  // Decode the instruction at the PC (identical field layout to soft_core.v).
  wire [31:0] instr  = imem[pc];
  wire [7:0]  opcode = instr[31:24];
  wire [7:0]  dst    = instr[23:16];
  wire [7:0]  ra     = instr[15:8];
  wire [7:0]  rb     = instr[7:0];
  wire [15:0] imm    = instr[15:0];
  wire [31:0] av     = regs[ra[3:0]];
  wire [31:0] bv     = regs[rb[3:0]];

  // The proven general ALU computes every arithmetic/bitwise/comparison op straight from the opcode.
  wire [31:0] alu_y;
  alu the_alu(.opcode(opcode), .a(av), .b(bv), .y(alu_y));

  // SINGLE register write port (defaults each cycle; arms set rf_we/rf_wa/rf_wd) — mirrors soft_core.v.
  reg        rf_we;
  reg [3:0]  rf_wa;
  reg [31:0] rf_wd;

  integer j, k;
  initial begin
    pc = 0; halted = 0; out_valid = 0; out = 0; sp = 0; trapped = 0;
    for (k = 0; k < 16; k = k + 1) regs[k] = 0;
`ifndef SOFT_CORE_SCALAR_PROG_LOAD
    $readmemh("program.hex", imem);   // boot ROM (standalone core / harness TBs overwrite imem)
`endif
  end

  always @(posedge clk) begin
    if (rst) begin
      pc <= 0; halted <= 0; out_valid <= 0; sp <= 0; trapped <= 0;
    end else begin
      out_valid <= 1'b0;
      if (!halted) begin
        rf_we = 1'b0; rf_wa = dst[3:0]; rf_wd = 32'd0;
        case (opcode)
          8'd0:  begin rf_we = 1'b1; rf_wd = regs[dst[3:0]] + av;        pc <= pc + 8'd1; end // RevAdd
          8'd1:  begin rf_we = 1'b1; rf_wd = regs[dst[3:0]] - av;        pc <= pc + 8'd1; end // RevSub
          8'd2:  begin rf_we = 1'b1; rf_wd = regs[dst[3:0]] ^ av;        pc <= pc + 8'd1; end // RevXor
          8'd3:  begin rf_we = 1'b1; rf_wd = (~regs[dst[3:0]]) + 32'd1;  pc <= pc + 8'd1; end // RevNeg
          8'd4:  begin rf_we = 1'b1; rf_wd = av; regs[ra[3:0]] <= regs[dst[3:0]]; pc <= pc + 8'd1; end // RevSwap
          8'd5:  begin rf_we = 1'b1; rf_wd = {{16{imm[15]}}, imm};      pc <= pc + 8'd1; end // LoadConst (SIGN-extended 16-bit immediate — a negative constant like -1 loads as 0xFFFFFFFF, not 0x0000FFFF)
          8'd6:  begin rf_we = 1'b1; rf_wd = av;                        pc <= pc + 8'd1; end // Move
          8'd34: begin pc <= dst; end                                                       // Jump
          8'd35: begin if (av == 32'd0) pc <= dst; else pc <= pc + 8'd1; end                 // JumpIfFalse
          8'd36: begin if (av != 32'd0) pc <= dst; else pc <= pc + 8'd1; end                 // JumpIfTrue
          8'd89: begin out <= av; out_valid <= 1'b1; pc <= pc + 8'd1; end                    // Show
          8'd43: begin halted <= 1'b1; end                                                  // Halt
          8'd91: begin halted <= 1'b1; trapped <= 1'b1; end                                  // FailWith (trap)
          8'd37: begin                                                                      // Call (entry=dst, result=ra)
            for (j = 0; j < 16; j = j + 1) savedregs[sp][j] <= regs[j];
            retpc[sp] <= pc + 8'd1; retdst[sp] <= ra[3:0]; sp <= sp + 5'd1; pc <= dst;
          end
          8'd41: begin                                                                      // Return {src=ra}
            for (j = 0; j < 16; j = j + 1) regs[j] <= savedregs[sp - 5'd1][j];
            regs[retdst[sp - 5'd1]] <= regs[ra[3:0]]; sp <= sp - 5'd1; pc <= retpc[sp - 5'd1];
          end
          8'd42: begin                                                                      // ReturnNothing
            for (j = 0; j < 16; j = j + 1) regs[j] <= savedregs[sp - 5'd1][j];
            sp <= sp - 5'd1; pc <= retpc[sp - 5'd1];
          end
          default: begin rf_we = 1'b1; rf_wd = alu_y; pc <= pc + 8'd1; end                   // ALU: arith/bitwise/compare
        endcase
        if (rf_we) regs[rf_wa] <= rf_wd;
      end
    end
  end

`ifdef SOFT_CORE_SCALAR_PROG_LOAD
  // SoC program-load port: stream instruction words into imem while the core is held in reset. imem has
  // no other writer, so there is no port conflict. Guarded so the standalone core keeps imem a boot ROM.
  always @(posedge clk) if (prog_we) imem[prog_addr] <= prog_data;
`endif
endmodule
