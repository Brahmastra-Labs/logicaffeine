// fpu_gen.rs — standalone golden-vector generator for the f64 FPU proof.
//
// Emits, for each op, a hex file of lines `a_bits b_bits expected_bits` (raw u64 bit patterns of the
// f64 operands and the *Rust-computed* f64 result). The iverilog testbenches $readmemh these and assert
// the DUT reproduces `expected_bits` EXACTLY. Because the reference is Rust `f64` arithmetic, a green
// run proves the RTL is bit-exact to Rust f64.
//
// Deterministic: a fixed 64-bit LCG drives all random draws (no clock/rng entropy). Build & run with
//   rustc -O fpu_gen.rs -o /tmp/fpu_gen && /tmp/fpu_gen <outdir>
// (no cargo, no workspace — a leaf tool.)

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        // same LCG constants used by the Verilog testbenches (Knuth MMIX)
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }
}

// The set of "interesting" f64 values, as raw bits. Every pair (edge x edge) is exercised, plus the
// benchmark constants, plus a large random sweep.
fn edge_bits() -> Vec<u64> {
    let specials: Vec<f64> = vec![
        0.0, -0.0, 1.0, -1.0, 2.0, -2.0, 0.5, -0.5, 3.0, -3.0, 10.0, 0.1, 0.2, 0.3,
        f64::INFINITY, f64::NEG_INFINITY, f64::NAN,
        f64::MIN_POSITIVE,          // smallest positive normal
        -f64::MIN_POSITIVE,
        f64::MAX, f64::MIN,
        f64::EPSILON,
        // benchmark constants
        9.54791938424326609,        // nbody (day per year sun mass factor etc.)
        365.24,                     // nbody days/year
        4.0 * std::f64::consts::PI * std::f64::consts::PI, // nbody solar mass
        3.141592653589793,          // pi
        1.5, 100.0, 1e300, 1e-300, 1e-310, 1e-320, // near-overflow, subnormal-range
    ];
    let mut v: Vec<u64> = specials.iter().map(|x| x.to_bits()).collect();
    // explicit subnormals
    v.push(1u64);                        // smallest positive subnormal
    v.push(0x8000_0000_0000_0001);       // smallest negative subnormal
    v.push(0x000F_FFFF_FFFF_FFFF);       // largest subnormal
    v.push(0x8010_0000_0000_0000);       // -(smallest normal)
    v.push(0x7FF0_0000_0000_0000);       // +inf again (as bits)
    v.push(0x7FF8_0000_0000_0001);       // a quiet NaN with payload
    v.push(0xFFF0_0000_0000_0001);       // a signaling-ish NaN pattern (neg)
    v
}

// A random finite-biased f64: with some probability produce specials/subnormals, else a random
// bit pattern (which may be Inf/NaN too — that's fine, exercises those paths).
fn rnd_f64(lcg: &mut Lcg) -> u64 {
    let r = lcg.next();
    match r % 16 {
        0 => 0u64,
        1 => 0x8000_0000_0000_0000,        // -0
        2 => (lcg.next() & 0x000F_FFFF_FFFF_FFFF) | 0x0000_0000_0000_0000, // subnormal-ish
        3 => f64::INFINITY.to_bits(),
        4 => f64::NEG_INFINITY.to_bits(),
        _ => {
            // random exponent kept in a "reasonable" range most of the time to hit the normal path,
            // but allow the full range sometimes.
            let mant = lcg.next() & 0x000F_FFFF_FFFF_FFFF;
            let sign = (lcg.next() & 1) << 63;
            let exp: u64 = if r % 3 == 0 {
                (lcg.next() % 2047) & 0x7FF       // full range (incl inf/nan/subnormal edges)
            } else {
                // biased toward the "everyday" exponent band [900,1123] so most ops are normal-normal
                900 + (lcg.next() % 224)
            };
            sign | (exp << 52) | mant
        }
    }
}

fn write_bin<F: Fn(f64, f64) -> f64>(path: &Path, lcg: &mut Lcg, n: usize, op: F) -> (usize, usize) {
    let f = File::create(path).unwrap();
    let mut w = BufWriter::new(f);
    let edges = edge_bits();
    let mut edge_count = 0usize;
    // full edge x edge cross product
    for &ab in &edges {
        for &bb in &edges {
            let a = f64::from_bits(ab);
            let b = f64::from_bits(bb);
            let y = op(a, b);
            writeln!(w, "{:016x} {:016x} {:016x}", ab, bb, y.to_bits()).unwrap();
            edge_count += 1;
        }
    }
    // random sweep + tie-boundary stress (half general random, half tie-stress)
    for i in 0..n {
        let (ab, bb) = if i % 2 == 0 { (rnd_f64(lcg), rnd_f64(lcg)) } else { (tie_f64(lcg), tie_f64(lcg)) };
        let a = f64::from_bits(ab);
        let b = f64::from_bits(bb);
        let y = op(a, b);
        writeln!(w, "{:016x} {:016x} {:016x}", ab, bb, y.to_bits()).unwrap();
    }
    (edge_count, n)
}

fn write_cmp<F: Fn(f64, f64) -> bool>(path: &Path, lcg: &mut Lcg, n: usize, op: F) -> (usize, usize) {
    let f = File::create(path).unwrap();
    let mut w = BufWriter::new(f);
    let edges = edge_bits();
    let mut edge_count = 0usize;
    for &ab in &edges {
        for &bb in &edges {
            let a = f64::from_bits(ab);
            let b = f64::from_bits(bb);
            let y = if op(a, b) { 1u64 } else { 0u64 };
            writeln!(w, "{:016x} {:016x} {:016x}", ab, bb, y).unwrap();
            edge_count += 1;
        }
    }
    for i in 0..n {
        let (ab, bb) = if i % 2 == 0 { (rnd_f64(lcg), rnd_f64(lcg)) } else { (tie_f64(lcg), tie_f64(lcg)) };
        let a = f64::from_bits(ab);
        let b = f64::from_bits(bb);
        let y = if op(a, b) { 1u64 } else { 0u64 };
        writeln!(w, "{:016x} {:016x} {:016x}", ab, bb, y).unwrap();
    }
    (edge_count, n)
}

fn write_unary<F: Fn(f64) -> f64>(path: &Path, lcg: &mut Lcg, n: usize, op: F) -> (usize, usize) {
    let f = File::create(path).unwrap();
    let mut w = BufWriter::new(f);
    let edges = edge_bits();
    let mut edge_count = 0usize;
    for &ab in &edges {
        let a = f64::from_bits(ab);
        let y = op(a);
        // second column is unused (0) so the TB parser is uniform
        writeln!(w, "{:016x} {:016x} {:016x}", ab, 0u64, y.to_bits()).unwrap();
        edge_count += 1;
    }
    for i in 0..n {
        // sqrt: mix general random with tie-stress; force nonnegative most of the time (clear sign) so
        // the real-root rounding path is exercised heavily, but keep some raw draws (incl. negatives).
        let raw = if i % 2 == 0 { rnd_f64(lcg) } else { tie_f64(lcg) };
        let ab = if i % 4 == 0 { raw } else { raw & 0x7FFF_FFFF_FFFF_FFFF };
        let a = f64::from_bits(ab);
        let y = op(a);
        writeln!(w, "{:016x} {:016x} {:016x}", ab, 0u64, y.to_bits()).unwrap();
    }
    (edge_count, n)
}

// A "tie-stress" f64: short mantissas with a random low-bit tail, chosen to make exact-halfway and
// near-halfway rounding results common (the round-to-nearest-ties-to-even boundary is the subtle case).
fn tie_f64(lcg: &mut Lcg) -> u64 {
    let sign = (lcg.next() & 1) << 63;
    // exponent kept in the everyday band so results stay normal-ish and the tie logic is exercised
    let exp = 1000 + (lcg.next() % 48);
    // mantissa: a few high bits set, then zeros, then a random 1-3 bit tail near the LSB — this makes
    // products/quotients frequently land at or just past the guard/round boundary.
    let hi = lcg.next() & 0x000F_FFF0_0000_0000;
    let tail = lcg.next() & ((1u64 << ((lcg.next() % 4) + 1)) - 1);
    let mant = hi | tail;
    sign | (exp << 52) | mant
}

fn main() {
    let outdir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let d = Path::new(&outdir);
    let n_rand = 40000usize;

    let mut lcg = Lcg(0x1234_5678_9ABC_DEF0);
    let (ea, ra) = write_bin(&d.join("fpu_add.hex"), &mut lcg, n_rand, |a, b| a + b);
    let (es, rs) = write_bin(&d.join("fpu_sub.hex"), &mut lcg, n_rand, |a, b| a - b);
    let (em, rm) = write_bin(&d.join("fpu_mul.hex"), &mut lcg, n_rand, |a, b| a * b);
    let (ed, rd) = write_bin(&d.join("fpu_div.hex"), &mut lcg, n_rand, |a, b| a / b);
    let (elt, rlt) = write_cmp(&d.join("fpu_lt.hex"), &mut lcg, n_rand, |a, b| a < b);
    let (eeq, req) = write_cmp(&d.join("fpu_eq.hex"), &mut lcg, n_rand, |a, b| a == b);
    // sqrt: give it its own larger nonneg-biased sweep
    let (eq_, rq) = write_unary(&d.join("fpu_sqrt.hex"), &mut lcg, n_rand, |a| a.sqrt());

    eprintln!("generated vectors:");
    eprintln!("  add : {} edge + {} rand", ea, ra);
    eprintln!("  sub : {} edge + {} rand", es, rs);
    eprintln!("  mul : {} edge + {} rand", em, rm);
    eprintln!("  div : {} edge + {} rand", ed, rd);
    eprintln!("  lt  : {} edge + {} rand", elt, rlt);
    eprintln!("  eq  : {} edge + {} rand", eeq, req);
    eprintln!("  sqrt: {} edge + {} rand", eq_, rq);
}
