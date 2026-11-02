// hart_rvc — the RV64C decompressor: a 16-bit instruction in, the 32-bit instruction it stands for out, or
// `valid` low for an encoding the C extension reserves. The same table as `hart::rvc::expand`, which
// hart_rtl_lock holds it to over all 65,536 encodings.
module hart_rvc (
  input      [15:0] c,
  output reg [31:0] inst,
  output reg        valid
);
  wire [2:0] funct3 = c[15:13];
  wire [4:0] rd     = c[11:7];
  wire [4:0] rs2    = c[6:2];
  wire [4:0] rdp    = {2'b01, c[4:2]};
  wire [4:0] rs1p   = {2'b01, c[9:7]};
  wire [11:0] imm6  = {{7{c[12]}}, c[6:2]};
  wire [5:0] shamt  = {c[12], c[6:2]};

  localparam [6:0] OP_LOAD = 7'h03, OP_LOAD_FP = 7'h07, OP_IMM = 7'h13, OP_IMM_32 = 7'h1b, OP_STORE = 7'h23,
                   OP_STORE_FP = 7'h27, OP = 7'h33, OP_LUI = 7'h37, OP_32 = 7'h3b, OP_BRANCH = 7'h63,
                   OP_JALR = 7'h67, OP_JAL = 7'h6f, OP_SYSTEM = 7'h73;

  function [31:0] i_type(input [11:0] imm, input [4:0] rs1, input [2:0] f3, input [4:0] d, input [6:0] op);
    i_type = {imm, rs1, f3, d, op};
  endfunction
  function [31:0] s_type(input [11:0] imm, input [4:0] s2, input [4:0] rs1, input [2:0] f3, input [6:0] op);
    s_type = {imm[11:5], s2, rs1, f3, imm[4:0], op};
  endfunction
  function [31:0] r_type(input [6:0] f7, input [4:0] s2, input [4:0] rs1, input [2:0] f3, input [4:0] d,
                         input [6:0] op);
    r_type = {f7, s2, rs1, f3, d, op};
  endfunction
  function [31:0] b_type(input [12:0] imm, input [4:0] s2, input [4:0] rs1, input [2:0] f3, input [6:0] op);
    b_type = {imm[12], imm[10:5], s2, rs1, f3, imm[4:1], imm[11], op};
  endfunction
  function [31:0] j_type(input [20:0] imm, input [4:0] d, input [6:0] op);
    j_type = {imm[20], imm[10:1], imm[11], imm[19:12], d, op};
  endfunction

  wire [11:0] addi4spn = {2'b00, c[10:7], c[12:11], c[5], c[6], 2'b00};
  wire [11:0] uimm_d   = {4'b0000, c[6:5], c[12:10], 3'b000};
  wire [11:0] uimm_w   = {5'b00000, c[5], c[12:10], c[6], 2'b00};
  wire [11:0] addi16sp = {{3{c[12]}}, c[4:3], c[5], c[2], c[6], 4'b0000};
  wire [19:0] lui_imm  = {{14{c[12]}}, c[12], c[6:2]};
  wire [20:0] j_imm    = {{10{c[12]}}, c[8], c[10:9], c[6], c[7], c[2], c[11], c[5:3], 1'b0};
  wire [12:0] b_imm    = {{5{c[12]}}, c[6:5], c[2], c[11:10], c[4:3], 1'b0};
  wire [11:0] ldsp     = {3'b000, c[4:2], c[12], c[6:5], 3'b000};
  wire [11:0] lwsp     = {4'b0000, c[3:2], c[12], c[6:4], 2'b00};
  wire [11:0] sdsp     = {3'b000, c[9:7], c[12:10], 3'b000};
  wire [11:0] swsp     = {4'b0000, c[8:7], c[12:9], 2'b00};

  always @(*) begin
    inst = 32'd0;
    valid = 1'b1;
    case (c[1:0])
      2'b00: case (funct3)
        3'd0: begin inst = i_type(addi4spn, 5'd2, 3'd0, rdp, OP_IMM); valid = addi4spn != 12'd0; end
        3'd1: inst = i_type(uimm_d, rs1p, 3'd3, rdp, OP_LOAD_FP);
        3'd2: inst = i_type(uimm_w, rs1p, 3'd2, rdp, OP_LOAD);
        3'd3: inst = i_type(uimm_d, rs1p, 3'd3, rdp, OP_LOAD);
        3'd4: valid = 1'b0;
        3'd5: inst = s_type(uimm_d, rdp, rs1p, 3'd3, OP_STORE_FP);
        3'd6: inst = s_type(uimm_w, rdp, rs1p, 3'd2, OP_STORE);
        default: inst = s_type(uimm_d, rdp, rs1p, 3'd3, OP_STORE);
      endcase
      2'b01: case (funct3)
        3'd0: inst = i_type(imm6, rd, 3'd0, rd, OP_IMM);
        3'd1: begin inst = i_type(imm6, rd, 3'd0, rd, OP_IMM_32); valid = rd != 5'd0; end
        3'd2: inst = i_type(imm6, 5'd0, 3'd0, rd, OP_IMM);
        3'd3: if (rd == 5'd2) begin
                inst = i_type(addi16sp, 5'd2, 3'd0, 5'd2, OP_IMM);
                valid = {c[12], c[6:2]} != 6'd0;
              end else begin
                inst = {lui_imm, rd, OP_LUI};
                valid = {c[12], c[6:2]} != 6'd0;
              end
        3'd4: case (c[11:10])
          2'd0: inst = i_type({6'b000000, shamt}, rs1p, 3'd5, rs1p, OP_IMM);
          2'd1: inst = i_type({6'b010000, shamt}, rs1p, 3'd5, rs1p, OP_IMM);
          2'd2: inst = i_type(imm6, rs1p, 3'd7, rs1p, OP_IMM);
          default: case ({c[12], c[6:5]})
            3'b000: inst = r_type(7'h20, rdp, rs1p, 3'd0, rs1p, OP);
            3'b001: inst = r_type(7'h00, rdp, rs1p, 3'd4, rs1p, OP);
            3'b010: inst = r_type(7'h00, rdp, rs1p, 3'd6, rs1p, OP);
            3'b011: inst = r_type(7'h00, rdp, rs1p, 3'd7, rs1p, OP);
            3'b100: inst = r_type(7'h20, rdp, rs1p, 3'd0, rs1p, OP_32);
            3'b101: inst = r_type(7'h00, rdp, rs1p, 3'd0, rs1p, OP_32);
            default: valid = 1'b0;
          endcase
        endcase
        3'd5: inst = j_type(j_imm, 5'd0, OP_JAL);
        3'd6: inst = b_type(b_imm, 5'd0, rs1p, 3'd0, OP_BRANCH);
        default: inst = b_type(b_imm, 5'd0, rs1p, 3'd1, OP_BRANCH);
      endcase
      2'b10: case (funct3)
        3'd0: inst = i_type({6'b000000, shamt}, rd, 3'd1, rd, OP_IMM);
        3'd1: inst = i_type(ldsp, 5'd2, 3'd3, rd, OP_LOAD_FP);
        3'd2: begin inst = i_type(lwsp, 5'd2, 3'd2, rd, OP_LOAD); valid = rd != 5'd0; end
        3'd3: begin inst = i_type(ldsp, 5'd2, 3'd3, rd, OP_LOAD); valid = rd != 5'd0; end
        3'd4: if (!c[12]) begin
                if (rs2 == 5'd0) begin inst = i_type(12'd0, rd, 3'd0, 5'd0, OP_JALR); valid = rd != 5'd0; end
                else inst = r_type(7'h00, rs2, 5'd0, 3'd0, rd, OP);
              end else begin
                if (rs2 == 5'd0 && rd == 5'd0) inst = i_type(12'd1, 5'd0, 3'd0, 5'd0, OP_SYSTEM);
                else if (rs2 == 5'd0) inst = i_type(12'd0, rd, 3'd0, 5'd1, OP_JALR);
                else inst = r_type(7'h00, rs2, rd, 3'd0, rd, OP);
              end
        3'd5: inst = s_type(sdsp, rs2, 5'd2, 3'd3, OP_STORE_FP);
        3'd6: inst = s_type(swsp, rs2, 5'd2, 3'd2, OP_STORE);
        default: inst = s_type(sdsp, rs2, 5'd2, 3'd3, OP_STORE);
      endcase
      default: valid = 1'b0;
    endcase
  end
endmodule
