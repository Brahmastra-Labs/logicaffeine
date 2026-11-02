// hart_muldiv — the M extension's multiply and divide, one operation at a time. `start` latches the operation;
// `done` pulses with `result`, which then holds until the next start.
//
// Multiply: the product of the operands' magnitudes, built from four 64x16 partial products (one a cycle), then the
// sign the operation asks for. Divide: restoring division of the magnitudes, one quotient bit a cycle, then the
// signs; a zero divisor gives a quotient of all ones and the dividend as remainder, exactly as the ISA specifies.
module hart_muldiv (
  input             clk,
  input             rst,
  input             start,
  input      [2:0]  op,      // funct3: mul mulh mulhsu mulhu div divu rem remu
  input             word,    // the 32-bit forms (mulw divw divuw remw remuw)
  input      [63:0] a,
  input      [63:0] b,
  output reg        done,
  output reg [63:0] result
);
  reg         busy;
  reg         is_div;
  reg  [2:0]  r_op;
  reg         r_word;
  reg  [6:0]  count;
  reg         neg;              // the product, or the quotient, is negated
  reg         rem_neg;          // the remainder takes the dividend's sign
  reg         by_zero;
  reg  [63:0] r_a;              // the dividend as given: a zero divisor's remainder
  reg  [63:0] ua, ub;           // magnitudes
  reg  [127:0] acc;
  reg  [63:0] rem, quot;

  wire        sa_signed = (op == 3'd1 || op == 3'd2 || op == 3'd4 || op == 3'd6);
  wire        sb_signed = (op == 3'd1 || op == 3'd4 || op == 3'd6);
  wire [63:0] wa = word ? (sa_signed ? {{32{a[31]}}, a[31:0]} : {32'd0, a[31:0]}) : a;
  wire [63:0] wb = word ? (sb_signed ? {{32{b[31]}}, b[31:0]} : {32'd0, b[31:0]}) : b;
  wire        a_neg = sa_signed && wa[63];
  wire        b_neg = sb_signed && wb[63];
  wire [63:0] mag_a = a_neg ? -wa : wa;
  wire [63:0] mag_b = b_neg ? -wb : wb;

  wire [15:0]  chunk = ub[16 * count[1:0] +: 16];
  wire [79:0]  partial = ua * chunk;
  wire [127:0] acc_next = acc + ({48'd0, partial} << (16 * count[1:0]));
  wire [127:0] product = neg ? -acc_next : acc_next;
  wire [64:0]  shifted = {rem, quot[63]};
  wire [65:0]  trial = {1'b0, shifted} - {2'b00, ub};
  wire [63:0]  q_signed = neg ? -quot : quot;
  wire [63:0]  r_signed = rem_neg ? -rem : rem;

  function [63:0] sext32(input [63:0] v);
    sext32 = {{32{v[31]}}, v[31:0]};
  endfunction

  always @(posedge clk) begin
    if (rst) begin
      busy <= 1'b0;
      done <= 1'b0;
    end else begin
      done <= 1'b0;
      if (start) begin
        busy <= 1'b1;
        is_div <= op[2];
        r_op <= op;
        r_word <= word;
        r_a <= wa;
        count <= 7'd0;
        acc <= 128'd0;
        ua <= mag_a;
        ub <= mag_b;
        neg <= a_neg ^ b_neg;
        rem_neg <= a_neg;
        by_zero <= (wb == 64'd0);
        rem <= 64'd0;
        quot <= word ? {mag_a[31:0], 32'd0} : mag_a;
      end else if (busy && !is_div) begin
        acc <= acc_next;
        count <= count + 7'd1;
        if (count == 7'd3) begin
          busy <= 1'b0;
          done <= 1'b1;
          if (r_op == 3'd0) result <= r_word ? sext32(product[63:0]) : product[63:0];
          else result <= product[127:64];
        end
      end else if (busy && is_div) begin
        if (count == (r_word ? 7'd32 : 7'd64)) begin
          busy <= 1'b0;
          done <= 1'b1;
          if (by_zero) result <= r_op[1] ? (r_word ? sext32(r_a) : r_a) : 64'hffff_ffff_ffff_ffff;
          else if (r_op[1]) result <= r_word ? sext32(r_signed) : r_signed;
          else result <= r_word ? sext32(q_signed) : q_signed;
        end else begin
          if (!trial[65]) begin
            rem <= trial[63:0];
            quot <= {quot[62:0], 1'b1};
          end else begin
            rem <= shifted[63:0];
            quot <= {quot[62:0], 1'b0};
          end
          count <= count + 7'd1;
        end
      end
    end
  end
endmodule
