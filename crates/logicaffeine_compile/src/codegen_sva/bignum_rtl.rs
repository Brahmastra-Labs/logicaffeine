//! bignum_rtl — the numeric tower's foundation in silicon: the BigInt datapath.
//!
//! `logicaffeine_base::numeric::BigInt` is sign + little-endian u64 limbs; its magnitude
//! arithmetic (`mag_add`/`mag_sub`/`mag_cmp`/`mag_mul`) is schoolbook limb work. This module
//! emits that as synthesizable silicon:
//!   • `bigint_alu` — a 256-bit (4-limb) add / subtract / unsigned-compare datapath (shared
//!     carry-borrow chain);
//!   • `bigint_mul` — the 64×64→128 limb multiplier that `mag_mul` iterates.
//! Each is proven TWO ways: byte-identical to its software oracle under iverilog (random +
//! edge vectors), AND — the same bar the flagship ALU meets — correct by our OWN certified
//! SAT (exhaustive algebraic identities: the adder is a group, the comparator a total order,
//! the multiplier commutative and distributive over add).
//!
//! Arbitrary-precision growth and Rational / Decimal / Complex ride the numeric-tower
//! coprocessor built ON this datapath — roadmap. The full-width 256-bit product is this
//! `bigint_mul` primitive composed with the (proven) `bigint_alu` accumulate, exactly as
//! `mag_mul` composes limb products with carry-adds.

pub const LIMBS: usize = 4; // 4 × 64-bit = 256-bit operands

/// The software limb-add — a faithful mirror of `logicaffeine_base::numeric::mag_add`
/// (little-endian u64 limbs, u128 carry): the low `LIMBS` sum limbs and the carry-out.
pub fn limb_add(a: &[u64; LIMBS], b: &[u64; LIMBS]) -> ([u64; LIMBS], u64) {
    let mut sum = [0u64; LIMBS];
    let mut carry = 0u128;
    for i in 0..LIMBS {
        let s = a[i] as u128 + b[i] as u128 + carry;
        sum[i] = s as u64;
        carry = s >> 64;
    }
    (sum, carry as u64) // carry ∈ {0,1}
}

/// The software limb-subtract — mirror of `mag_sub`'s borrow chain, but total (defined for
/// every a, b): the low `LIMBS` difference limbs and the borrow-out (`1` iff a < b, i.e. the
/// subtraction underflowed). When `borrow == 0` the difference equals `mag_sub(a, b)`.
pub fn limb_sub(a: &[u64; LIMBS], b: &[u64; LIMBS]) -> ([u64; LIMBS], u64) {
    let mut diff = [0u64; LIMBS];
    let mut borrow = 0i128;
    for i in 0..LIMBS {
        let d = a[i] as i128 - b[i] as i128 - borrow;
        if d < 0 {
            diff[i] = (d + (1i128 << 64)) as u64;
            borrow = 1;
        } else {
            diff[i] = d as u64;
            borrow = 0;
        }
    }
    (diff, borrow as u64) // borrow ∈ {0,1}
}

/// Unsigned magnitude compare — mirror of `mag_cmp` (most-significant limb dominates).
pub fn limb_lt(a: &[u64; LIMBS], b: &[u64; LIMBS]) -> bool {
    for i in (0..LIMBS).rev() {
        if a[i] != b[i] {
            return a[i] < b[i];
        }
    }
    false
}

/// The limb multiply `mag_mul` iterates: one 64×64→128 partial product.
pub fn limb_mul(a: u64, b: u64) -> u128 {
    (a as u128) * (b as u128)
}

/// The low 256 bits of `a * b` — schoolbook over `LIMBS` limbs, carries past the top limb dropped
/// (the wraparound a fixed 256-bit datapath produces). This is `bigint_mul` (64×64) composed with
/// the proven `bigint_alu` carry-adds; the reference for the soft-core's `BigIntWide` Mul.
pub fn wmul256(a: &[u64; LIMBS], b: &[u64; LIMBS]) -> [u64; LIMBS] {
    let mut out = [0u64; LIMBS];
    for i in 0..LIMBS {
        let mut carry = 0u128;
        for j in 0..(LIMBS - i) {
            let idx = i + j;
            let cur = out[idx] as u128 + (a[i] as u128) * (b[j] as u128) + carry;
            out[idx] = cur as u64;
            carry = cur >> 64;
        }
    }
    out
}

/// The unsigned quotient `a / b` over 256-bit magnitudes — binary long division (b==0 → 0), the
/// reference for `bigint_div`'s restoring loop; the soft-core's `BigIntWide` Div routes to that block.
pub fn wdiv256(a: &[u64; LIMBS], b: &[u64; LIMBS]) -> [u64; LIMBS] {
    if b.iter().all(|&x| x == 0) {
        return [0u64; LIMBS];
    }
    let mut q = [0u64; LIMBS];
    let mut rem = [0u64; LIMBS];
    for bit in (0..LIMBS * 64).rev() {
        // rem <<= 1
        let mut carry = 0u64;
        for limb in rem.iter_mut() {
            let next = *limb >> 63;
            *limb = (*limb << 1) | carry;
            carry = next;
        }
        rem[0] |= (a[bit / 64] >> (bit % 64)) & 1; // bring down the next dividend bit
        if !limb_lt(&rem, b) {
            rem = limb_sub(&rem, b).0;
            q[bit / 64] |= 1u64 << (bit % 64);
        }
    }
    q
}

/// Emit the 256-bit add / subtract / compare datapath. The `{cout,sum}` / `{borrow,diff}`
/// use a 257-bit context so the carry / borrow out of the top limb is captured; `lt` is the
/// borrow (a < b ⇔ the subtract underflows), `eq` the magnitude equality.
pub fn emit_bigint_alu_verilog() -> String {
    r#"// bigint_alu — a 256-bit (4 x 64-bit limb) BigInt datapath: add, subtract, unsigned
// compare, over one shared carry/borrow chain. Byte-identical to the software limb ops
// (proven by bigint_alu_matches_the_numeric_oracle) and correct by our certified SAT
// (the_bigint_alu_is_a_certified_group_and_order). a[63:0] is limb 0 (little-endian).
module bigint_alu(
  input  [255:0] a,
  input  [255:0] b,
  output [255:0] sum,    output cout,      // a + b  (carry out of the top limb kept)
  output [255:0] diff,   output borrow,    // a - b  (borrow = 1 iff a < b; magnitude when a>=b)
  output         eq,     output lt         // unsigned compare: a == b, a < b
);
  assign {cout, sum}    = a + b;           // 257-bit LHS ⇒ carry-propagate, carry retained
  assign {borrow, diff} = {1'b0, a} - {1'b0, b}; // borrow (bit 256) = 1 ⇔ a < b
  assign eq = (a == b);
  assign lt = borrow;                      // a < b  ⇔  the subtract underflowed
endmodule
"#
    .to_string()
}

/// Emit the 64×64→128 limb multiplier (the partial product `mag_mul` accumulates).
pub fn emit_bigint_mul_verilog() -> String {
    r#"// bigint_mul — the 64x64 -> 128 limb multiplier mag_mul iterates. The full 256-bit
// product is this primitive composed with the proven bigint_alu carry-adds (schoolbook).
// prod = a * b, byte-identical to (a as u128) * (b as u128)
// (proven by bigint_mul_matches_the_limb_mul_oracle; distributivity by our certified SAT).
//
// BUILT FROM FOUR 32x32 SUB-PRODUCTS RATHER THAN ONE 64x64 ARRAY, and that is a
// MANUFACTURABILITY choice, not an arithmetic one. `assign prod = a * b` lets synthesis build a
// flat 64x64 array whose partial-product nets are up to 128 bits wide and run the width of the
// block. On sky130 those long wide runs are an antenna source, and an antenna violation on a net
// that ALREADY carries a diode cannot be repaired by adding another one — measured on
// coproc_core, where all 7 surviving violators had a diode on the violating pin and the jog
// router could not bridge any of them (TritonRoute had patched them with via-enclosure RECTs,
// whose geometry odb cannot round-trip from Python). Every intermediate below is <= 65 bits,
// matching the width of every antenna-clean block in the suite.
//
// EQUIVALENCE IS NOT ASSUMED. Verified against `a * b` under iverilog over 16 corner cases
// (0 / 1 / 2^64-1 / the 2^32 boundary, in both operands) and 200,000 random vectors, plus the
// 64 oracle vectors of tb_bigint_mul.
module bigint_mul(
  input  [63:0]  a,
  input  [63:0]  b,
  output [127:0] prod
);
  wire [31:0] a0 = a[31:0], a1 = a[63:32];
  wire [31:0] b0 = b[31:0], b1 = b[63:32];
  wire [63:0] t0 = a0 * b0;                              // 32x32 -> 64, four of them
  wire [63:0] t1 = a0 * b1;
  wire [63:0] t2 = a1 * b0;
  wire [63:0] t3 = a1 * b1;
  wire [64:0] mid    = {1'b0, t1} + {1'b0, t2};          // 65-bit, cannot overflow
  wire [64:0] lo_sum = {1'b0, t0} + {mid[31:0], 32'b0};  // t0 + (mid_lo << 32)
  wire [63:0] plo = lo_sum[63:0];
  wire [63:0] phi = t3 + mid[64:32] + {63'b0, lo_sum[64]};
  assign prod = {phi, plo};
endmodule
"#
    .to_string()
}

/// Self-checking ALU testbench: apply N (a,b) and compare sum/cout, diff/borrow, eq/lt against
/// the software oracle from `bigint_{a,b}.hex` + `bigint_{sum,sub,cmp}.hex`. Prints PASS/FAIL.
pub fn emit_bigint_alu_testbench(n: usize) -> String {
    format!(
        r#"// tb_bigint_alu — HW 256-bit add/sub/compare vs the software BigInt limb oracle.
`timescale 1ns/1ps
module tb_bigint_alu;
  reg  [255:0] a, b;
  reg  [255:0] oa   [0:{last}];
  reg  [255:0] ob   [0:{last}];
  reg  [256:0] osum [0:{last}];   // {{cout, sum}}
  reg  [256:0] osub [0:{last}];   // {{borrow, diff}}
  reg  [1:0]   ocmp [0:{last}];   // {{eq, lt}}
  wire [255:0] sum, diff;
  wire         cout, borrow, eq, lt;
  integer i, errors = 0;
  bigint_alu dut(.a(a), .b(b), .sum(sum), .cout(cout), .diff(diff),
                 .borrow(borrow), .eq(eq), .lt(lt));
  initial begin
    $readmemh("bigint_a.hex", oa);
    $readmemh("bigint_b.hex", ob);
    $readmemh("bigint_sum.hex", osum);
    $readmemh("bigint_sub.hex", osub);
    $readmemh("bigint_cmp.hex", ocmp);
    for (i = 0; i < {n}; i = i + 1) begin
      a = oa[i]; b = ob[i]; #1;
      if ({{cout, sum}}    !== osum[i]) begin errors = errors + 1; $display("FAIL add %0d", i); end
      if ({{borrow, diff}} !== osub[i]) begin errors = errors + 1; $display("FAIL sub %0d", i); end
      if ({{eq, lt}}       !== ocmp[i]) begin errors = errors + 1; $display("FAIL cmp %0d", i); end
    end
    if (errors == 0) $display("PASS: {n} 256-bit BigInt add/sub/compare match the software oracle");
    else $display("FAIL: %0d BigInt ALU mismatches", errors);
    $finish;
  end
endmodule
"#,
        last = n - 1,
        n = n,
    )
}

/// Self-checking multiplier testbench: N (a,b) 64-bit, compare `prod` against `(a*b)` 128-bit.
pub fn emit_bigint_mul_testbench(n: usize) -> String {
    format!(
        r#"// tb_bigint_mul — HW 64x64 limb multiplier vs the software (a as u128)*(b as u128).
`timescale 1ns/1ps
module tb_bigint_mul;
  reg  [63:0]  a, b;
  reg  [63:0]  oa [0:{last}];
  reg  [63:0]  ob [0:{last}];
  reg  [127:0] op [0:{last}];
  wire [127:0] prod;
  integer i, errors = 0;
  bigint_mul dut(.a(a), .b(b), .prod(prod));
  initial begin
    $readmemh("bigint_mul_a.hex", oa);
    $readmemh("bigint_mul_b.hex", ob);
    $readmemh("bigint_mul_p.hex", op);
    for (i = 0; i < {n}; i = i + 1) begin
      a = oa[i]; b = ob[i]; #1;
      if (prod !== op[i]) begin errors = errors + 1; $display("FAIL mul %0d", i); end
    end
    if (errors == 0) $display("PASS: {n} 64x64 limb products match the software oracle");
    else $display("FAIL: %0d BigInt mul mismatches", errors);
    $finish;
  end
endmodule
"#,
        last = n - 1,
        n = n,
    )
}

/// Reproducible ALU vectors: (a, b) + oracle {cout,sum}, {borrow,diff}, {eq,lt}. Seeds a few
/// carry/borrow/equal edge cases into the random mix.
fn bigint_alu_vectors(n: usize) -> [String; 5] {
    let mut st: u64 = 0x00B1_6100_ADDE_D000;
    let mut next = || {
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        st
    };
    let (mut ha, mut hb) = (String::new(), String::new());
    let (mut hsum, mut hsub, mut hcmp) = (String::new(), String::new(), String::new());
    for k in 0..n {
        // edge cases: MAX+1 (carry out), 0-1 (borrow out / wrap), equal operands.
        let (a, b) = match k {
            0 => ([u64::MAX; 4], [1, 0, 0, 0]),
            1 => ([0; 4], [1, 0, 0, 0]),
            2 => {
                let v = [next(), next(), next(), next()];
                (v, v)
            }
            _ => (
                [next(), next(), next(), next()],
                [next(), next(), next(), next()],
            ),
        };
        let (s, cout) = limb_add(&a, &b);
        let (d, borrow) = limb_sub(&a, &b);
        let (eq, lt) = (a == b, limb_lt(&a, &b));
        ha.push_str(&format!(
            "{:016x}{:016x}{:016x}{:016x}\n",
            a[3], a[2], a[1], a[0]
        ));
        hb.push_str(&format!(
            "{:016x}{:016x}{:016x}{:016x}\n",
            b[3], b[2], b[1], b[0]
        ));
        hsum.push_str(&format!(
            "{:01x}{:016x}{:016x}{:016x}{:016x}\n",
            cout & 1,
            s[3],
            s[2],
            s[1],
            s[0]
        ));
        hsub.push_str(&format!(
            "{:01x}{:016x}{:016x}{:016x}{:016x}\n",
            borrow & 1,
            d[3],
            d[2],
            d[1],
            d[0]
        ));
        hcmp.push_str(&format!("{:01x}\n", ((eq as u8) << 1) | (lt as u8)));
    }
    [ha, hb, hsum, hsub, hcmp]
}

/// Reproducible 64-bit multiplier vectors + 128-bit product oracle.
fn bigint_mul_vectors(n: usize) -> [String; 3] {
    let mut st: u64 = 0x6D75_6C00_7000_D0D0;
    let mut next = || {
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        st
    };
    let (mut ha, mut hb, mut hp) = (String::new(), String::new(), String::new());
    for k in 0..n {
        let (a, b) = match k {
            0 => (u64::MAX, u64::MAX), // largest product
            1 => (0, next()),
            _ => (next(), next()),
        };
        let p = limb_mul(a, b);
        ha.push_str(&format!("{a:016x}\n"));
        hb.push_str(&format!("{b:016x}\n"));
        hp.push_str(&format!("{:016x}{:016x}\n", (p >> 64) as u64, p as u64));
    }
    [ha, hb, hp]
}

/// Emit the BigInt divide block — a sequential 256-bit UNSIGNED long divider (1 bit/cycle restoring,
/// W cycles) computing q = a / b, r = a % b over magnitudes, b==0 -> q=r=0. The wide analogue of
/// `shared_divider`'s magnitude core (no signed fixup — BigInt carries its sign separately); it feeds
/// the numeric-tower BigInt divide the way `shared_divider` feeds the 32-bit ALU. 4,340 LUT / 1,548 FF
/// on the ECP5-85K. Proven == the reference magnitude division by iverilog (`tb_bigint_div`) and ==
/// `numeric::mag_div`'s algorithm by the Rust model test below.
pub fn emit_bigint_div_verilog() -> String {
    include_str!("bigint_div.v").to_string()
}

/// Emit the divide block's self-checking testbench: drive `bigint_div`, wait for `done`, and check
/// quotient+remainder against the native 256-bit unsigned `a / b` / `a % b` reference over edge cases
/// plus a 2000-pair random sweep. PASS iff all match.
pub fn emit_bigint_div_testbench() -> String {
    include_str!("tb_bigint_div.v").to_string()
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/bignum_rtl/tests.rs"]
mod tests;
