// hart_core — the board's RV64IMAC hart (Zicsr, Zifencei, machine and user modes, eight PMP entries): a five-stage
// in-order pipeline whose every instruction does exactly what `hart::model` does in one step.
//
//   F  fetch    64-bit words into a queue of 16-bit parcels (a compressed or straddling instruction needs no refetch)
//   D  decode   one instruction from the queue: decompress, decode, read registers, predict (jal, backward branch)
//   E  execute  ALU, branches, CSRs, multiply/divide, address, protection and memory-map checks; EVERY trap is taken
//               here, so M and W never trap and a store or device access is never undone
//   M  memory   loads, stores, atomics, LR/SC through one data port (the cache began the read when E advanced)
//   W  write    the register file
//
// Retirement is counted as an instruction leaves E, because nothing after E can fault. A CSR write, mret, a trap
// and fence.i redirect the fetch to the next instruction, so nothing fetched under the old state survives. A device
// access completes before anything younger leaves E, so a store that stops or restarts the board is the last thing
// the hart does before it stops.
module hart_core #(
  parameter [63:0] ROM_BASE = 64'h0, parameter [63:0] ROM_SIZE = 64'h0,
  parameter [63:0] SRAM_BASE = 64'h0, parameter [63:0] SRAM_SIZE = 64'h0,
  parameter [63:0] IO_BASE  = 64'h0, parameter [63:0] IO_SIZE  = 64'h0,
  parameter [63:0] RAM_BASE = 64'h8000_0000, parameter [63:0] RAM_SIZE = 64'h10_0000
) (
  input             clk,
  input             rst,
  input      [63:0] reset_pc,
  // instruction words: a request is answered (if_ready) on a later cycle; one request is outstanding at a time and
  // the next may be issued in the cycle the answer arrives
  output reg        if_req,
  output reg [63:0] if_addr,
  input             if_ready,
  input      [63:0] if_rdata,
  // data: E announces the access entering M at the coming edge (the cache starts its read then); M holds its
  // request until dm_ready, which completes it in that same cycle
  output            dm_pre_valid,
  output     [63:0] dm_pre_addr,
  output            dm_req,
  output     [63:0] dm_addr,
  output            dm_we,
  output     [7:0]  dm_be,
  output     [63:0] dm_wdata,
  input             dm_ready,
  input      [63:0] dm_rdata,
  // fence.i: write back every dirty data line and forget every instruction line
  output reg        cm_req,
  input             cm_done
);
  // ── architectural constants ─────────────────────────────────────────────────────────────────────────────────
  localparam [63:0] MISA = (64'd2 << 62) | 64'h1 | (64'h1 << 2) | (64'h1 << 8) | (64'h1 << 12) | (64'h1 << 20);
  localparam [1:0] PRIV_U = 2'd0, PRIV_M = 2'd3;
  localparam [3:0] C_FETCH = 4'd1, C_ILLEGAL = 4'd2, C_BREAK = 4'd3, C_MIS_LOAD = 4'd4, C_LOAD = 4'd5,
                   C_MIS_STORE = 4'd6, C_STORE = 4'd7, C_ECALL_U = 4'd8, C_ECALL_M = 4'd11;

  // ── architectural state ────────────────────────────────────────────────────────────────────────────────────
  (* ram_style = "registers" *) reg [63:0] xreg [1:31];
  // The integer registers configure to zero, as the model's start.
  integer xr;
  initial for (xr = 1; xr < 32; xr = xr + 1) xreg[xr] = 64'd0;
  reg [1:0]  priv;
  reg        st_mie, st_mpie, st_mprv, st_tw;
  reg [1:0]  st_mpp;
  reg [63:0] r_mie, r_mtvec, r_mscratch, r_mepc, r_mcause, r_mtval, r_mcycle, r_minstret;
  reg [2:0]  r_mcounteren;
  reg [2:0]  r_mcountinhibit;
  reg [7:0]  pmpcfg [0:7];
  reg [53:0] pmpaddr [0:7];
  reg        resv_valid;
  reg [63:0] resv_addr;

  wire [63:0] mstatus = (64'd2 << 32) | ({63'd0, st_mie} << 3) | ({63'd0, st_mpie} << 7) | ({62'd0, st_mpp} << 11)
                      | ({63'd0, st_mprv} << 17) | ({63'd0, st_tw} << 21);

  // ── physical memory protection: each entry's byte range, recomputed from the CSRs every cycle ──────────────
  reg [57:0] pmp_lo [0:7];
  reg [57:0] pmp_hi [0:7];
  integer pi;
  always @(posedge clk) begin
    for (pi = 0; pi < 8; pi = pi + 1) begin
      case (pmpcfg[pi][4:3])
        2'd1: begin
          pmp_lo[pi] <= (pi == 0) ? 58'd0 : {2'b00, pmpaddr[(pi == 0) ? 0 : pi - 1], 2'b00};
          pmp_hi[pi] <= {2'b00, pmpaddr[pi], 2'b00};
        end
        2'd2: begin
          pmp_lo[pi] <= {2'b00, pmpaddr[pi], 2'b00};
          pmp_hi[pi] <= {2'b00, pmpaddr[pi], 2'b00} + 58'd4;
        end
        2'd3: begin
          pmp_lo[pi] <= {1'b0, {1'b0, pmpaddr[pi]} & ~({1'b0, pmpaddr[pi]} ^ ({1'b0, pmpaddr[pi]} + 55'd1)), 2'b00};
          pmp_hi[pi] <= {1'b0, ({1'b0, pmpaddr[pi]} & ~({1'b0, pmpaddr[pi]} ^ ({1'b0, pmpaddr[pi]} + 55'd1))), 2'b00}
                      + {1'b0, ({1'b0, pmpaddr[pi]} ^ ({1'b0, pmpaddr[pi]} + 55'd1)) + 55'd1, 2'b00};
        end
        default: begin pmp_lo[pi] <= 58'd0; pmp_hi[pi] <= 58'd0; end
      endcase
    end
  end

  // kind: 0 read, 1 write, 2 execute, 3 atomic
  function pmp_ok(input [63:0] a, input [3:0] size, input [1:0] kind, input [1:0] p);
    integer i;
    reg found, ok, any, all;
    reg [64:0] lo, hi, a0, a1;
    reg [7:0] cfg;
    begin
      found = 1'b0;
      ok = (p == PRIV_M);
      a0 = {1'b0, a};
      a1 = {1'b0, a} + {61'd0, size};
      for (i = 0; i < 8; i = i + 1) begin
        cfg = pmpcfg[i];
        lo = {7'd0, pmp_lo[i]};
        hi = {7'd0, pmp_hi[i]};
        any = (cfg[4:3] != 2'd0) && (lo < hi) && (a0 < hi) && (a1 > lo);
        all = (a0 >= lo) && (a1 <= hi);
        if (!found && any) begin
          found = 1'b1;
          if (!all) ok = 1'b0;
          else if (p == PRIV_M && !cfg[7]) ok = 1'b1;
          else case (kind)
            2'd0: ok = cfg[0];
            2'd1: ok = cfg[1];
            2'd2: ok = cfg[2];
            default: ok = cfg[0] & cfg[1];
          endcase
        end
      end
      pmp_ok = ok;
    end
  endfunction

  // ── the physical memory map ────────────────────────────────────────────────────────────────────────────────
  function in_region(input [63:0] a, input [3:0] size, input [63:0] base, input [63:0] rsize);
    reg [64:0] end_a, end_r;
    begin
      end_a = {1'b0, a} + {61'd0, size};
      end_r = {1'b0, base} + {1'b0, rsize};
      in_region = (rsize != 64'd0) && (a >= base) && (end_a <= end_r) && (end_a > {1'b0, a});
    end
  endfunction

  function pma_ok(input [63:0] a, input [3:0] size, input [1:0] kind);
    begin
      pma_ok = (in_region(a, size, ROM_BASE, ROM_SIZE) && (kind == 2'd0 || kind == 2'd2))
            || (in_region(a, size, SRAM_BASE, SRAM_SIZE) && kind != 2'd2)
            || (in_region(a, size, IO_BASE, IO_SIZE) && (kind == 2'd0 || kind == 2'd1) && size == 4'd8)
            ||  in_region(a, size, RAM_BASE, RAM_SIZE);
    end
  endfunction

  // ── pipeline control signals (defined below) ───────────────────────────────────────────────────────────────
  wire        e_redirect;     // E redirects the fetch at this edge
  wire [63:0] e_redirect_pc;
  wire        d_issue;        // D hands an instruction to E at this edge
  wire        d_predict;      // ...and redirects the fetch to its predicted target
  wire [63:0] d_predict_pc;
  wire        redirect = e_redirect || d_predict;
  wire [63:0] redirect_pc = e_redirect ? e_redirect_pc : d_predict_pc;

  // ── F: fetch words into the parcel queue ───────────────────────────────────────────────────────────────────
  reg [15:0] iq [0:11];       // parcels, oldest first
  reg [11:0] iq_fault;        // the parcel's fetch was refused
  reg [3:0]  iq_count;
  reg [63:0] iq_pc;           // address of iq[0]
  reg [63:0] f_req_addr;      // the word the next request names
  reg [1:0]  f_req_skip;      // parcels to drop from that word (a redirect into the middle of a word)
  reg        f_pending;       // a request is outstanding
  reg        f_discard;       // ...and is on an abandoned path
  reg        f_pend_nomem;    // ...and was refused outright, so no memory answers it
  reg [1:0]  f_pend_bad;      // the refused halves: [0] low 32 bits, [1] high 32 bits
  reg [1:0]  f_pend_skip;

  wire [2:0] d_pop;           // parcels D takes this cycle (0, 1 or 2)

  // A word is fetched only from executable memory the privilege may execute. Protection has 4-byte granularity,
  // so each half of the word is checked alone; a word with one permitted half is still read, its other half
  // marked refused. A word with neither half permitted is never sent to the memory.
  wire       f_lo_ok = pma_ok(f_req_addr, 4'd4, 2'd2) && pmp_ok(f_req_addr, 4'd4, 2'd2, priv);
  wire       f_hi_ok = pma_ok(f_req_addr + 64'd4, 4'd4, 2'd2) && pmp_ok(f_req_addr + 64'd4, 4'd4, 2'd2, priv);
  wire       f_answer = f_pending && (f_pend_nomem || if_ready);
  wire       f_keep = f_answer && !f_discard;
  wire [3:0] f_take = 4'd4 - {2'b00, f_pend_skip};
  wire [3:0] iq_after_pop = iq_count - {1'b0, d_pop};
  wire [3:0] iq_next_count = iq_after_pop + (f_keep ? f_take : 4'd0);
  wire       f_room = ({1'b0, iq_next_count} + 5'd4) <= 5'd12;
  wire       f_issue = !rst && (!f_pending || f_answer) && f_room && !redirect;
  always @(*) begin
    if_req = f_issue && (f_lo_ok || f_hi_ok);
    if_addr = f_req_addr;
  end

  wire [63:0] f_word = f_pend_nomem ? 64'd0 : if_rdata;
  integer qi;
  always @(posedge clk) begin
    if (rst) begin
      iq_count <= 4'd0;
      iq_pc <= reset_pc;
      f_req_addr <= {reset_pc[63:3], 3'b000};
      f_req_skip <= reset_pc[2:1];
      f_pending <= 1'b0;
      f_discard <= 1'b0;
    end else if (redirect) begin
      // Every parcel in the queue is on the abandoned path; an answer still on its way is dropped when it comes.
      iq_count <= 4'd0;
      iq_pc <= redirect_pc;
      f_req_addr <= {redirect_pc[63:3], 3'b000};
      f_req_skip <= redirect_pc[2:1];
      if (f_pending && !f_answer) f_discard <= 1'b1;
      else begin f_pending <= 1'b0; f_discard <= 1'b0; end
    end else begin
      // Pop what D took, then append the answered word's parcels behind what remains.
      for (qi = 0; qi < 12; qi = qi + 1) begin
        if (qi + d_pop < 12) begin
          iq[qi] <= iq[qi + d_pop];
          iq_fault[qi] <= iq_fault[qi + d_pop];
        end
      end
      iq_pc <= iq_pc + {60'd0, d_pop, 1'b0};
      if (f_keep) begin
        for (qi = 0; qi < 4; qi = qi + 1) begin
          if (qi >= f_pend_skip) begin
            iq[iq_after_pop + qi - f_pend_skip] <= f_word[16 * qi +: 16];
            iq_fault[iq_after_pop + qi - f_pend_skip] <= (qi < 2) ? f_pend_bad[0] : f_pend_bad[1];
          end
        end
      end
      iq_count <= iq_next_count;
      if (f_answer) begin
        f_pending <= 1'b0;
        f_discard <= 1'b0;
      end
      if (f_issue) begin
        f_pending <= 1'b1;
        f_pend_nomem <= !(f_lo_ok || f_hi_ok);
        f_pend_bad <= {!f_hi_ok, !f_lo_ok};
        f_pend_skip <= f_req_skip;
        f_req_skip <= 2'd0;
        f_req_addr <= f_req_addr + 64'd8;
      end
    end
  end

  // ── D: decode one instruction from the queue ───────────────────────────────────────────────────────────────
  wire [15:0] p0 = iq[0];
  wire [15:0] p1 = iq[1];
  wire        d_comp = p0[1:0] != 2'b11;
  wire        d_avail = (iq_count >= 4'd1) && (d_comp || iq_count >= 4'd2);
  wire [31:0] d_exp;
  wire        d_exp_valid;
  hart_rvc rvc(.c(p0), .inst(d_exp), .valid(d_exp_valid));
  wire [31:0] d_inst = d_comp ? d_exp : {p1, p0};
  wire [31:0] d_raw  = d_comp ? {16'd0, p0} : {p1, p0};
  wire [3:0]  d_len  = d_comp ? 4'd2 : 4'd4;
  wire        d_ffault = iq_fault[0] || (!d_comp && iq_fault[1]);
  wire [63:0] d_ftval = iq_fault[0] ? iq_pc : iq_pc + 64'd2;
  assign d_pop = !d_issue ? 3'd0 : d_comp ? 3'd1 : 3'd2;

  wire [6:0]  d_op  = d_inst[6:0];
  wire [4:0]  d_rd  = d_inst[11:7];
  wire [2:0]  d_f3  = d_inst[14:12];
  wire [4:0]  d_rs1 = d_inst[19:15];
  wire [4:0]  d_rs2 = d_inst[24:20];
  wire [6:0]  d_f7  = d_inst[31:25];
  wire [63:0] imm_i = {{52{d_inst[31]}}, d_inst[31:20]};
  wire [63:0] imm_s = {{52{d_inst[31]}}, d_inst[31:25], d_inst[11:7]};
  wire [63:0] imm_b = {{51{d_inst[31]}}, d_inst[31], d_inst[7], d_inst[30:25], d_inst[11:8], 1'b0};
  wire [63:0] imm_u = {{32{d_inst[31]}}, d_inst[31:12], 12'd0};
  wire [63:0] imm_j = {{43{d_inst[31]}}, d_inst[31], d_inst[19:12], d_inst[20], d_inst[30:21], 1'b0};

  // Classes of instruction E distinguishes.
  localparam [4:0] K_ILLEGAL = 5'd0, K_ALU = 5'd1, K_ALUW = 5'd2, K_LUI = 5'd3, K_AUIPC = 5'd4, K_JAL = 5'd5,
                   K_JALR = 5'd6, K_BRANCH = 5'd7, K_LOAD = 5'd8, K_STORE = 5'd9, K_AMO = 5'd10, K_LR = 5'd11,
                   K_SC = 5'd12, K_MULDIV = 5'd13, K_CSR = 5'd14, K_ECALL = 5'd15, K_EBREAK = 5'd16,
                   K_MRET = 5'd17, K_WFI = 5'd18, K_FENCE = 5'd19, K_FENCEI = 5'd20, K_FFAULT = 5'd21;
  reg [4:0]  d_kind;
  reg [63:0] d_imm;
  reg        d_imm_b;          // operand b is the immediate
  reg        d_reads1, d_reads2, d_writes;
  always @(*) begin
    d_kind = K_ILLEGAL;
    d_imm = imm_i;
    d_imm_b = 1'b0;
    d_reads1 = 1'b0;
    d_reads2 = 1'b0;
    d_writes = 1'b0;
    if (d_ffault) d_kind = K_FFAULT;
    else if (d_comp && !d_exp_valid) d_kind = K_ILLEGAL;
    else case (d_op)
      7'h37: begin d_kind = K_LUI; d_imm = imm_u; d_writes = 1'b1; end
      7'h17: begin d_kind = K_AUIPC; d_imm = imm_u; d_writes = 1'b1; end
      7'h6f: begin d_kind = K_JAL; d_imm = imm_j; d_writes = 1'b1; end
      7'h67: if (d_f3 == 3'd0) begin d_kind = K_JALR; d_reads1 = 1'b1; d_writes = 1'b1; end
      7'h63: if (d_f3 != 3'd2 && d_f3 != 3'd3) begin d_kind = K_BRANCH; d_imm = imm_b; d_reads1 = 1'b1; d_reads2 = 1'b1; end
      7'h03: if (d_f3 != 3'd7) begin d_kind = K_LOAD; d_reads1 = 1'b1; d_writes = 1'b1; end
      7'h23: if (d_f3[2] == 1'b0) begin d_kind = K_STORE; d_imm = imm_s; d_reads1 = 1'b1; d_reads2 = 1'b1; end
      7'h13: begin
        if (d_f3 == 3'd1 ? d_inst[31:26] == 6'd0 : d_f3 == 3'd5 ? (d_inst[31:26] == 6'd0 || d_inst[31:26] == 6'h10) : 1'b1)
          begin d_kind = K_ALU; d_imm_b = 1'b1; d_reads1 = 1'b1; d_writes = 1'b1; end
      end
      7'h1b: begin
        if (d_f3 == 3'd0 || (d_f3 == 3'd1 && d_f7 == 7'd0) || (d_f3 == 3'd5 && (d_f7 == 7'd0 || d_f7 == 7'h20)))
          begin d_kind = K_ALUW; d_imm_b = 1'b1; d_reads1 = 1'b1; d_writes = 1'b1; end
      end
      7'h33: begin
        if (d_f7 == 7'd1) begin d_kind = K_MULDIV; d_reads1 = 1'b1; d_reads2 = 1'b1; d_writes = 1'b1; end
        else if (d_f7 == 7'd0 || (d_f7 == 7'h20 && (d_f3 == 3'd0 || d_f3 == 3'd5)))
          begin d_kind = K_ALU; d_reads1 = 1'b1; d_reads2 = 1'b1; d_writes = 1'b1; end
      end
      7'h3b: begin
        if (d_f7 == 7'd1 && (d_f3 == 3'd0 || d_f3[2]))
          begin d_kind = K_MULDIV; d_reads1 = 1'b1; d_reads2 = 1'b1; d_writes = 1'b1; end
        else if ((d_f7 == 7'd0 && (d_f3 == 3'd0 || d_f3 == 3'd1 || d_f3 == 3'd5)) || (d_f7 == 7'h20 && (d_f3 == 3'd0 || d_f3 == 3'd5)))
          begin d_kind = K_ALUW; d_reads1 = 1'b1; d_reads2 = 1'b1; d_writes = 1'b1; end
      end
      7'h2f: begin
        if (d_f3 == 3'd2 || d_f3 == 3'd3) begin
          case (d_inst[31:27])
            5'h02: if (d_rs2 == 5'd0) begin d_kind = K_LR; d_reads1 = 1'b1; d_writes = 1'b1; end
            5'h03: begin d_kind = K_SC; d_reads1 = 1'b1; d_reads2 = 1'b1; d_writes = 1'b1; end
            5'h00, 5'h01, 5'h04, 5'h08, 5'h0c, 5'h10, 5'h14, 5'h18, 5'h1c:
              begin d_kind = K_AMO; d_reads1 = 1'b1; d_reads2 = 1'b1; d_writes = 1'b1; end
            default: ;
          endcase
          d_imm = 64'd0;
        end
      end
      7'h0f: begin
        if (d_f3 == 3'd0) d_kind = K_FENCE;
        else if (d_f3 == 3'd1) d_kind = K_FENCEI;
      end
      7'h73: begin
        if (d_f3 == 3'd0) begin
          case (d_inst)
            32'h0000_0073: d_kind = K_ECALL;
            32'h0010_0073: d_kind = K_EBREAK;
            32'h3020_0073: d_kind = K_MRET;
            32'h1050_0073: d_kind = K_WFI;
            default: ;
          endcase
        end else if (d_f3 != 3'd4) begin
          d_kind = K_CSR;
          d_reads1 = !d_f3[2];
          d_writes = 1'b1;
        end
      end
      default: ;
    endcase
  end

  // Static prediction: jal always, a backward branch taken.
  wire d_pred_taken = (d_kind == K_JAL) || (d_kind == K_BRANCH && d_inst[31]);
  assign d_predict = d_issue && d_pred_taken;
  assign d_predict_pc = iq_pc + d_imm;

  // Register reads, with the instruction W writes this cycle seen through.
  wire        w_writes;
  wire [4:0]  w_rd;
  wire [63:0] w_value;
  function [63:0] rf_read(input [4:0] r);
    rf_read = (r == 5'd0) ? 64'd0 : (w_writes && w_rd == r) ? w_value : xreg[r];
  endfunction

  // ── D/E register ───────────────────────────────────────────────────────────────────────────────────────────
  reg        e_valid;
  reg [63:0] e_pc, e_imm, e_a_rf, e_b_rf;
  reg [31:0] e_inst, e_raw;
  reg [3:0]  e_len;
  reg [4:0]  e_kind, e_rd, e_rs1, e_rs2;
  reg        e_imm_b, e_reads1, e_reads2, e_writes, e_pred;
  reg [63:0] e_ftval;
  reg [63:0] e_seq, d_seq;

  // ── E/M and M/W registers (declared here for forwarding) ──────────────────────────────────────────────────
  reg        m_valid;
  reg [4:0]  m_kind, m_rd;
  reg        m_writes;
  reg [63:0] m_value, m_addr, m_b, m_pc, m_seq;
  reg [2:0]  m_f3;
  reg [4:0]  m_amo;
  reg        m_phase;          // an atomic's write half
  reg        w_valid;
  reg [4:0]  w_rd_r;
  reg        w_writes_r;
  reg [63:0] w_value_r;
  assign w_writes = w_valid && w_writes_r && w_rd_r != 5'd0;
  assign w_rd = w_rd_r;
  assign w_value = w_value_r;

  // An M-stage result is ready for forwarding unless it comes from memory (a load, an atomic, an SC).
  wire m_late = (m_kind == K_LOAD || m_kind == K_AMO || m_kind == K_LR || m_kind == K_SC);
  wire m_fwd1 = m_valid && m_writes && m_rd != 5'd0 && m_rd == e_rs1;
  wire m_fwd2 = m_valid && m_writes && m_rd != 5'd0 && m_rd == e_rs2;
  wire w_fwd1 = w_writes && w_rd == e_rs1;
  wire w_fwd2 = w_writes && w_rd == e_rs2;
  wire [63:0] a = (e_rs1 == 5'd0) ? 64'd0 : m_fwd1 ? m_value : w_fwd1 ? w_value : e_a_rf;
  wire [63:0] rs2v = (e_rs2 == 5'd0) ? 64'd0 : m_fwd2 ? m_value : w_fwd2 ? w_value : e_b_rf;
  wire [63:0] b = e_imm_b ? e_imm : rs2v;
  wire e_load_use = e_valid && ((e_reads1 && m_fwd1) || (e_reads2 && m_fwd2)) && m_late;

  reg [31:0] m_raw_r;
  reg [1:0]  md_state;        // multiply/divide: 0 idle, 1 running, 2 holding E's result
  reg        fi_wait;         // fence.i: the caches are being made coherent
  wire       m_busy;
  wire       m_device;        // M holds a device access

  // ── E: execute ─────────────────────────────────────────────────────────────────────────────────────────────
  wire [2:0] f3 = e_inst[14:12];
  wire [6:0] f7 = e_inst[31:25];
  wire       sub = (e_kind == K_ALU || e_kind == K_ALUW) && !e_imm_b && f7 == 7'h20;
  wire [5:0] shamt = e_imm_b ? e_inst[25:20] : rs2v[5:0];

  function [63:0] sext32(input [31:0] v);
    sext32 = {{32{v[31]}}, v};
  endfunction

  // Arithmetic shifts are computed on explicitly signed values: in a mixed conditional Verilog makes the whole
  // expression unsigned and `>>>` would shift in zeros.
  wire signed [63:0] a_signed = a;
  wire signed [31:0] a_signed32 = a[31:0];
  wire [63:0] sra64 = a_signed >>> b[5:0];
  wire [31:0] sra32 = a_signed32 >>> b[4:0];
  wire        arith = e_imm_b ? e_inst[30] : f7 == 7'h20;
  reg [63:0] alu;
  always @(*) begin
    case (f3)
      3'd0: alu = sub ? a - b : a + b;
      3'd1: alu = a << b[5:0];
      3'd2: alu = {63'd0, $signed(a) < $signed(b)};
      3'd3: alu = {63'd0, a < b};
      3'd4: alu = a ^ b;
      3'd5: alu = arith ? sra64 : a >> b[5:0];
      3'd6: alu = a | b;
      default: alu = a & b;
    endcase
  end
  wire [31:0] sum32 = sub ? a[31:0] - b[31:0] : a[31:0] + b[31:0];
  wire [31:0] sll32 = a[31:0] << b[4:0];
  wire [31:0] srl32 = a[31:0] >> b[4:0];
  reg [63:0] aluw;
  always @(*) begin
    case (f3)
      3'd0: aluw = sext32(sum32);
      3'd1: aluw = sext32(sll32);
      default: aluw = sext32(arith ? sra32 : srl32);
    endcase
  end

  reg br_taken;
  always @(*) begin
    case (f3)
      3'd0: br_taken = a == rs2v;
      3'd1: br_taken = a != rs2v;
      3'd4: br_taken = $signed(a) < $signed(rs2v);
      3'd5: br_taken = $signed(a) >= $signed(rs2v);
      3'd6: br_taken = a < rs2v;
      default: br_taken = a >= rs2v;
    endcase
  end

  // Multiply and divide.
  wire        md_start;
  wire        md_done;
  wire [63:0] md_result;
  hart_muldiv muldiv(.clk(clk), .rst(rst), .start(md_start), .op(f3), .word(e_inst[3]), .a(a), .b(rs2v),
                     .done(md_done), .result(md_result));

  // Memory access: address, size, alignment, protection and the memory map.
  wire [63:0] maddr = a + ((e_kind == K_STORE || e_kind == K_LOAD) ? e_imm : 64'd0);
  wire [3:0]  msize = (e_kind == K_LOAD || e_kind == K_STORE) ? (4'd1 << f3[1:0]) : (f3 == 3'd2 ? 4'd4 : 4'd8);
  wire        mmis = (maddr & {60'd0, msize - 4'd1}) != 64'd0;
  wire [1:0]  dpriv = (st_mprv && priv == PRIV_M) ? st_mpp : priv;
  wire        is_mem = e_kind == K_LOAD || e_kind == K_STORE || e_kind == K_AMO || e_kind == K_LR || e_kind == K_SC;
  wire [1:0]  mkind = (e_kind == K_LOAD) ? 2'd0 : (e_kind == K_STORE) ? 2'd1 : 2'd3;
  // Whether an SC will find its reservation, foreseen here so a store it would make can fault in E: an LR to the
  // same address finishing in M sets it, an SC in M releases it.
  wire        sc_will_store = (m_valid && m_kind == K_LR) ? (m_addr == maddr)
                            : (m_valid && m_kind == K_SC) ? 1'b0
                            : (resv_valid && resv_addr == maddr);
  wire        mprot = (e_kind == K_LR)
                    ? !(pmp_ok(maddr, msize, 2'd0, dpriv) && pma_ok(maddr, msize, 2'd0) && pma_ok(maddr, msize, 2'd3))
                    : (e_kind == K_SC)
                    ? sc_will_store && !(pmp_ok(maddr, msize, 2'd1, dpriv) && pma_ok(maddr, msize, 2'd3))
                    : !(pmp_ok(maddr, msize, mkind, dpriv) && pma_ok(maddr, msize, mkind));

  // CSR access.
  wire [11:0] csr = e_inst[31:20];
  wire [63:0] csr_operand = f3[2] ? {59'd0, e_rs1} : a;
  wire        csr_writes = (f3[1:0] == 2'd1) || (e_rs1 != 5'd0);
  reg  [63:0] csr_old;
  reg         csr_exists;
  wire [63:0] pmpcfg0 = {pmpcfg[7], pmpcfg[6], pmpcfg[5], pmpcfg[4], pmpcfg[3], pmpcfg[2], pmpcfg[1], pmpcfg[0]};
  always @(*) begin
    csr_exists = 1'b1;
    csr_old = 64'd0;
    if (csr[9:8] > priv) csr_exists = 1'b0;
    else case (csr)
      12'h300: csr_old = mstatus;
      12'h301: csr_old = MISA;
      12'h304: csr_old = r_mie;
      12'h305: csr_old = r_mtvec;
      12'h306: csr_old = {61'd0, r_mcounteren};
      12'h30a: csr_old = 64'd0;
      12'h320: csr_old = {61'd0, r_mcountinhibit};
      12'h340: csr_old = r_mscratch;
      12'h341: csr_old = r_mepc;
      12'h342: csr_old = r_mcause;
      12'h343: csr_old = r_mtval;
      12'h344: csr_old = 64'd0;
      12'h3a0: csr_old = pmpcfg0;
      12'h3a2: csr_old = 64'd0;
      12'hb00: csr_old = r_mcycle;
      12'hb02: csr_old = r_minstret;
      12'hc00: if (priv == PRIV_M || r_mcounteren[0]) csr_old = r_mcycle; else csr_exists = 1'b0;
      12'hc01: if (priv == PRIV_M || r_mcounteren[1]) csr_old = r_mcycle; else csr_exists = 1'b0;
      12'hc02: if (priv == PRIV_M || r_mcounteren[2]) csr_old = r_minstret; else csr_exists = 1'b0;
      default: begin
        if (csr >= 12'h3b0 && csr <= 12'h3bf) csr_old = (csr[3:0] < 4'd8) ? {10'd0, pmpaddr[csr[2:0]]} : 64'd0;
        else if ((csr >= 12'hb03 && csr <= 12'hb1f) || (csr >= 12'h323 && csr <= 12'h33f) || (csr >= 12'hf11 && csr <= 12'hf15))
          csr_old = 64'd0;
        else if (csr >= 12'hc03 && csr <= 12'hc1f) begin
          if (priv != PRIV_M) csr_exists = 1'b0;
        end else csr_exists = 1'b0;
      end
    endcase
  end
  wire [63:0] csr_new = (f3[1:0] == 2'd1) ? csr_operand : (f3[1:0] == 2'd2) ? (csr_old | csr_operand) : (csr_old & ~csr_operand);
  wire        csr_illegal = !csr_exists || (csr_writes && csr[11:10] == 2'b11);

  // The exception E raises, if any.
  reg        x_trap;
  reg [3:0]  x_cause;
  reg [63:0] x_tval;
  always @(*) begin
    x_trap = 1'b0;
    x_cause = C_ILLEGAL;
    x_tval = {32'd0, e_raw};
    case (e_kind)
      K_ILLEGAL: x_trap = 1'b1;
      K_FFAULT: begin x_trap = 1'b1; x_cause = C_FETCH; x_tval = e_ftval; end
      K_ECALL: begin x_trap = 1'b1; x_cause = (priv == PRIV_M) ? C_ECALL_M : C_ECALL_U; x_tval = 64'd0; end
      K_EBREAK: begin x_trap = 1'b1; x_cause = C_BREAK; x_tval = e_pc; end
      K_MRET: if (priv != PRIV_M) x_trap = 1'b1;
      K_WFI: if (priv != PRIV_M && st_tw) x_trap = 1'b1;
      K_CSR: if (csr_illegal) x_trap = 1'b1;
      K_LOAD, K_STORE, K_AMO, K_LR, K_SC: begin
        if (mmis) begin
          x_trap = 1'b1;
          x_cause = (e_kind == K_LOAD || e_kind == K_LR) ? C_MIS_LOAD : C_MIS_STORE;
          x_tval = maddr;
        end else if (mprot) begin
          x_trap = 1'b1;
          x_cause = (e_kind == K_LOAD || e_kind == K_LR) ? C_LOAD : C_STORE;
          x_tval = maddr;
        end
      end
      default: ;
    endcase
  end

  // E completes this cycle unless it waits: on M, on a value M is still loading, on the multiplier, or on fence.i.
  wire e_md = e_kind == K_MULDIV;
  assign md_start = e_valid && e_md && md_state == 2'd0 && !e_load_use;
  wire e_wait = e_valid && (e_load_use || m_device || (e_md && md_state != 2'd2)
                            || (e_kind == K_FENCEI && !x_trap && !(fi_wait && cm_done)));
  wire e_go = e_valid && !e_wait && !m_busy;
  wire e_trap_now = e_go && x_trap;
  wire e_retire = e_go && !x_trap;

  wire [63:0] e_next = e_pc + {60'd0, e_len};
  wire [63:0] e_target = (e_kind == K_JALR) ? ((a + e_imm) & ~64'd1) : (e_pc + e_imm);
  wire        e_taken = (e_kind == K_JAL) || (e_kind == K_JALR) || (e_kind == K_BRANCH && br_taken);
  wire        e_mispredict = (e_kind == K_JALR) || ((e_kind == K_BRANCH || e_kind == K_JAL) && (e_taken != e_pred));
  wire        e_serialize = (e_kind == K_CSR && csr_writes) || e_kind == K_MRET || e_kind == K_FENCEI;
  assign e_redirect = e_go && (x_trap || e_kind == K_MRET || e_serialize || e_mispredict);
  assign e_redirect_pc = x_trap ? {r_mtvec[63:2], 2'b00}
                       : (e_kind == K_MRET) ? r_mepc
                       : (e_kind == K_JALR || (e_taken && e_kind != K_CSR && e_kind != K_FENCEI)) ? e_target
                       : e_next;

  wire [63:0] e_value = (e_kind == K_ALU) ? alu : (e_kind == K_ALUW) ? aluw : (e_kind == K_LUI) ? e_imm
                      : (e_kind == K_AUIPC) ? e_pc + e_imm : (e_kind == K_JAL || e_kind == K_JALR) ? e_next
                      : (e_kind == K_MULDIV) ? md_result : (e_kind == K_CSR) ? csr_old : 64'd0;

  // D hands an instruction to E when E is free or leaving, and nothing redirects.
  wire e_free = !e_valid || e_go;
  assign d_issue = !rst && d_avail && e_free && !e_redirect;

  always @(posedge clk) begin
    if (rst) md_state <= 2'd0;
    else if (md_start) md_state <= 2'd1;
    else if (md_state == 2'd1 && md_done) md_state <= 2'd2;
    else if (md_state == 2'd2 && e_go) md_state <= 2'd0;
  end

  // ── D → E ────────────────────────────────────────────────────────────────────────────────────────────────
  always @(posedge clk) begin
    if (rst) begin
      e_valid <= 1'b0;
      d_seq <= 64'd0;
    end else if (d_issue) begin
      e_valid <= 1'b1;
      e_pc <= iq_pc;
      e_inst <= d_inst;
      e_raw <= d_raw;
      e_len <= d_len;
      e_kind <= d_kind;
      e_imm <= d_imm;
      e_imm_b <= d_imm_b;
      e_rd <= d_rd;
      e_rs1 <= (d_reads1 || d_kind == K_CSR) ? d_rs1 : 5'd0;
      e_rs2 <= d_reads2 ? d_rs2 : 5'd0;
      e_reads1 <= d_reads1;
      e_reads2 <= d_reads2;
      e_writes <= d_writes;
      e_pred <= d_pred_taken;
      e_a_rf <= rf_read(d_rs1);
      e_b_rf <= rf_read(d_rs2);
      e_ftval <= d_ftval;
      e_seq <= d_seq;
      d_seq <= d_seq + 64'd1;
    end else if (e_go || e_redirect) begin
      e_valid <= 1'b0;
    end else begin
      // E waits: its captured operands stay right as producers move from M to W.
      if (w_fwd1 && !m_fwd1) e_a_rf <= w_value;
      if (w_fwd2 && !m_fwd2) e_b_rf <= w_value;
    end
  end

  // ── E: architectural updates (CSRs, privilege, counters, the reservation's release) ───────────────────────
  integer ci;
  always @(posedge clk) begin
    if (rst) begin
      priv <= PRIV_M;
      st_mie <= 1'b0; st_mpie <= 1'b0; st_mpp <= PRIV_U; st_mprv <= 1'b0; st_tw <= 1'b0;
      r_mie <= 64'd0; r_mtvec <= 64'd0; r_mscratch <= 64'd0; r_mepc <= 64'd0; r_mcause <= 64'd0; r_mtval <= 64'd0;
      r_mcounteren <= 3'd0; r_mcountinhibit <= 3'd0; r_mcycle <= 64'd0; r_minstret <= 64'd0;
      for (ci = 0; ci < 8; ci = ci + 1) begin pmpcfg[ci] <= 8'd0; pmpaddr[ci] <= 54'd0; end
      fi_wait <= 1'b0;
      cm_req <= 1'b0;
    end else begin
      // An instruction that writes mcountinhibit is itself counted (or not) under the value it writes.
      if (!r_mcountinhibit[0]) r_mcycle <= r_mcycle + 64'd1;
      if (e_retire && !(e_kind == K_CSR && csr_writes && csr == 12'h320 ? csr_new[2] : r_mcountinhibit[2]))
        r_minstret <= r_minstret + 64'd1;
      if (e_trap_now) begin
        r_mepc <= e_pc;
        r_mcause <= {60'd0, x_cause};
        r_mtval <= x_tval;
        st_mpie <= st_mie;
        st_mie <= 1'b0;
        st_mpp <= priv;
        priv <= PRIV_M;
      end else if (e_retire) begin
        if (e_kind == K_MRET) begin
          priv <= st_mpp;
          st_mie <= st_mpie;
          st_mpie <= 1'b1;
          if (st_mpp != PRIV_M) st_mprv <= 1'b0;
          st_mpp <= PRIV_U;
        end
        if (e_kind == K_CSR && csr_writes) begin
          case (csr)
            12'h300: begin
              st_mie <= csr_new[3];
              st_mpie <= csr_new[7];
              st_mpp <= (csr_new[12:11] == 2'd3) ? PRIV_M : PRIV_U;
              st_mprv <= csr_new[17];
              st_tw <= csr_new[21];
            end
            12'h304: r_mie <= csr_new & 64'h888;
            12'h305: r_mtvec <= {csr_new[63:2], 2'b00};
            12'h306: r_mcounteren <= csr_new[2:0];
            12'h320: r_mcountinhibit <= {csr_new[2], 1'b0, csr_new[0]};
            12'h340: r_mscratch <= csr_new;
            12'h341: r_mepc <= {csr_new[63:1], 1'b0};
            12'h342: r_mcause <= csr_new;
            12'h343: r_mtval <= csr_new;
            12'h3a0: begin
              for (ci = 0; ci < 8; ci = ci + 1) begin
                if (!pmpcfg[ci][7])
                  pmpcfg[ci] <= {csr_new[8 * ci + 7], 2'b00, csr_new[8 * ci + 3 +: 2], csr_new[8 * ci + 2],
                                 csr_new[8 * ci + 1] & csr_new[8 * ci], csr_new[8 * ci]};
              end
            end
            12'hb00: r_mcycle <= csr_new;
            12'hb02: r_minstret <= csr_new;
            default: begin
              if (csr >= 12'h3b0 && csr <= 12'h3b7) begin
                if (!(pmpcfg[csr[2:0]][7] || (csr[2:0] != 3'd7 && pmpcfg[csr[2:0] + 3'd1][7]
                      && pmpcfg[csr[2:0] + 3'd1][4:3] == 2'd1)))
                  pmpaddr[csr[2:0]] <= csr_new[53:0];
              end
            end
          endcase
        end
      end
      // fence.i: once everything older has left M, ask the caches to agree, and finish when they do.
      if (e_valid && e_kind == K_FENCEI && !x_trap && !fi_wait && !m_valid) begin
        fi_wait <= 1'b1;
        cm_req <= 1'b1;
      end
      if (cm_req) cm_req <= 1'b0;
      if (fi_wait && cm_done && e_go) fi_wait <= 1'b0;
    end
  end

  // ── E → M ──────────────────────────────────────────────────────────────────────────────────────────────────
  wire e_to_m = e_retire;
  assign dm_pre_valid = e_to_m && is_mem;
  assign dm_pre_addr = maddr;

  always @(posedge clk) begin
    if (rst) m_valid <= 1'b0;
    else if (!m_busy) begin
      m_valid <= e_to_m;
      m_kind <= e_kind;
      m_rd <= e_rd;
      m_writes <= e_writes;
      m_value <= e_value;
      m_addr <= maddr;
      m_b <= rs2v;
      m_f3 <= f3;
      m_amo <= e_inst[31:27];
      m_pc <= e_pc;
      m_seq <= e_seq;
      m_raw_r <= e_raw;
    end
  end
  // ── M: memory ──────────────────────────────────────────────────────────────────────────────────────────────
  wire [2:0]  lane = m_addr[2:0];
  wire [3:0]  m_size = (m_kind == K_LOAD || m_kind == K_STORE) ? (4'd1 << m_f3[1:0]) : (m_f3 == 3'd2 ? 4'd4 : 4'd8);
  wire [7:0]  size_mask = (m_size == 4'd1) ? 8'h01 : (m_size == 4'd2) ? 8'h03 : (m_size == 4'd4) ? 8'h0f : 8'hff;
  wire [63:0] loaded = dm_rdata >> {lane, 3'b000};
  reg  [63:0] load_value;
  always @(*) begin
    case (m_f3)
      3'd0: load_value = {{56{loaded[7]}}, loaded[7:0]};
      3'd1: load_value = {{48{loaded[15]}}, loaded[15:0]};
      3'd2: load_value = sext32(loaded[31:0]);
      3'd4: load_value = {56'd0, loaded[7:0]};
      3'd5: load_value = {48'd0, loaded[15:0]};
      3'd6: load_value = {32'd0, loaded[31:0]};
      default: load_value = loaded;
    endcase
  end
  reg  [63:0] amo_keep;
  wire [63:0] amo_read = (m_size == 4'd4) ? sext32(loaded[31:0]) : loaded;
  wire [63:0] amo_old = m_phase ? amo_keep : amo_read;
  wire [63:0] amo_b   = (m_size == 4'd4) ? sext32(m_b[31:0]) : m_b;
  reg  [63:0] amo_new;
  always @(*) begin
    case (m_amo)
      5'h01: amo_new = amo_b;
      5'h00: amo_new = amo_old + amo_b;
      5'h04: amo_new = amo_old ^ amo_b;
      5'h0c: amo_new = amo_old & amo_b;
      5'h08: amo_new = amo_old | amo_b;
      5'h10: amo_new = ($signed(amo_old) < $signed(amo_b)) ? amo_old : amo_b;
      5'h14: amo_new = ($signed(amo_old) > $signed(amo_b)) ? amo_old : amo_b;
      5'h18: amo_new = (amo_old < amo_b) ? amo_old : amo_b;
      default: amo_new = (amo_old > amo_b) ? amo_old : amo_b;
    endcase
  end
  wire sc_ok = resv_valid && resv_addr == m_addr;
  wire m_mem = m_valid && (m_kind == K_LOAD || m_kind == K_STORE || m_kind == K_LR || m_kind == K_AMO
                           || (m_kind == K_SC && sc_ok));
  wire m_writing = (m_kind == K_STORE) || (m_kind == K_SC) || (m_kind == K_AMO && m_phase);
  assign m_device = m_mem && in_region(m_addr, 4'd1, IO_BASE, IO_SIZE);
  wire [63:0] store_data = (m_kind == K_AMO) ? amo_new : m_b;
  assign dm_req = m_mem;
  assign dm_addr = {m_addr[63:3], 3'b000};
  assign dm_we = m_writing;
  assign dm_be = size_mask << lane;
  assign dm_wdata = store_data << {lane, 3'b000};
  // An atomic reads, then writes the same word the next cycle.
  assign m_busy = m_mem && (!dm_ready || (m_kind == K_AMO && !m_phase));
  always @(posedge clk) begin
    if (rst) m_phase <= 1'b0;
    else if (m_valid && m_kind == K_AMO && !m_phase && dm_ready) begin
      m_phase <= 1'b1;
      amo_keep <= amo_read;
    end else if (!m_busy) m_phase <= 1'b0;
  end

  wire [63:0] m_result = (m_kind == K_LOAD || m_kind == K_LR) ? load_value
                       : (m_kind == K_AMO) ? amo_keep
                       : (m_kind == K_SC) ? (sc_ok ? 64'd0 : 64'd1)
                       : m_value;

  // The reservation: set by LR and released by SC, by a trap and by mret — whichever comes later in program order
  // wins, and a trap or mret in E is always younger than M.
  always @(posedge clk) begin
    if (rst) resv_valid <= 1'b0;
    else begin
      if (m_valid && !m_busy && m_kind == K_LR) begin resv_valid <= 1'b1; resv_addr <= m_addr; end
      if (m_valid && !m_busy && m_kind == K_SC) resv_valid <= 1'b0;
      if (e_trap_now || (e_retire && e_kind == K_MRET)) resv_valid <= 1'b0;
    end
  end

  // ── M → W ──────────────────────────────────────────────────────────────────────────────────────────────────
  always @(posedge clk) begin
    if (rst) w_valid <= 1'b0;
    else begin
      w_valid <= m_valid && !m_busy;
      w_rd_r <= m_rd;
      w_writes_r <= m_writes;
      w_value_r <= m_result;
    end
  end

  // ── W: the register file ───────────────────────────────────────────────────────────────────────────────────
  always @(posedge clk) begin
    if (w_writes) xreg[w_rd] <= w_value;
  end

`ifdef HART_TRACE
  // The commit trace a lockstep run compares with the model: an instruction with the value it writes as M completes
  // it (C), a store as M completes it (S), a device's answer to a load (L), a trap as E takes it (T). The sequence
  // number orders them in program order.
  always @(posedge clk) begin
    if (!rst) begin
      if (m_valid && !m_busy)
        $display("C %0d %016x %08x %0d %016x", m_seq, m_pc, m_raw_r, (m_writes && m_rd != 5'd0 ? m_rd : 5'd0), m_result);
      if (m_mem && dm_ready && m_writing) $display("S %0d %016x %02x %016x", m_seq, dm_addr, dm_be, dm_wdata);
      if (m_device && dm_ready && !m_writing) $display("L %0d %016x %016x", m_seq, dm_addr, dm_rdata);
      if (e_trap_now) $display("T %0d %016x %0d %016x", e_seq, e_pc, x_cause, x_tval);
    end
  end
`endif
endmodule
