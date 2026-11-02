use super::*;

fn lcg(seed: u64) -> impl FnMut() -> u64 {
    let mut x = seed;
    move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }
}

/// CONSTRUCTIVE FUNCTIONAL COMPLETENESS: all 16 two-input boolean functions are realized from the
/// `{AND, OR, NOT}` basis and compute their truth table bit-for-bit over fuzzed rows. This is the
/// proof that the DRAM plane (with Ambit NOT) is boolean-complete via dataflow composition.
#[test]
fn all_sixteen_boolean_functions_compose_from_the_basis() {
    let width = 4;
    let mut rng = lcg(0xa11c_e5_f00d_1234);
    for _ in 0..25 {
        let a: Row = (0..width).map(|_| rng()).collect();
        let b: Row = (0..width).map(|_| rng()).collect();
        for truth in 0u8..16 {
            let mut df = DramDataflow::new(2);
            let out = df.boolean_fn(0, 1, truth);
            let heap = df.run(&[a.clone(), b.clone()], width);
            let expected: Row = a
                .iter()
                .zip(&b)
                .map(|(&aw, &bw)| {
                    let mut ew = 0u64;
                    for p in 0..64 {
                        let ab = ((aw >> p) & 1) as u8;
                        let bb = ((bw >> p) & 1) as u8;
                        if (truth >> (2 * ab + bb)) & 1 == 1 {
                            ew |= 1u64 << p;
                        }
                    }
                    ew
                })
                .collect();
            assert_eq!(
                heap[out], expected,
                "boolean fn truth={truth:#06b} diverged"
            );
        }
    }
}

/// The named composed gates equal their bitwise reference over fuzzed rows.
#[test]
fn composed_named_gates_equal_their_reference() {
    let width = 8;
    let mut rng = lcg(0xbeef_cafe_0042_9999);
    for _ in 0..40 {
        let a: Row = (0..width).map(|_| rng()).collect();
        let b: Row = (0..width).map(|_| rng()).collect();
        let cases: &[(&str, fn(&mut DramDataflow) -> usize, fn(u64, u64) -> u64)] = &[
            ("xor", |df| df.xor(0, 1), |x, y| x ^ y),
            ("nand", |df| df.nand(0, 1), |x, y| !(x & y)),
            ("nor", |df| df.nor(0, 1), |x, y| !(x | y)),
            ("xnor", |df| df.xnor(0, 1), |x, y| !(x ^ y)),
            ("difference", |df| df.difference(0, 1), |x, y| x & !y),
        ];
        for (name, build, refn) in cases {
            let mut df = DramDataflow::new(2);
            let out = build(&mut df);
            let heap = df.run(&[a.clone(), b.clone()], width);
            let expected: Row = a.iter().zip(&b).map(|(&x, &y)| refn(x, y)).collect();
            assert_eq!(
                heap[out], expected,
                "composed `{name}` diverged from reference"
            );
        }
    }
}

fn to_planes(vals: &[u64], bits: usize) -> Vec<Row> {
    (0..bits)
        .map(|i| {
            let mut w = 0u64;
            for (j, &v) in vals.iter().enumerate() {
                if (v >> i) & 1 == 1 {
                    w |= 1u64 << j;
                }
            }
            vec![w]
        })
        .collect()
}

fn from_planes(planes: &[Row], lanes: usize) -> Vec<u64> {
    (0..lanes)
        .map(|j| {
            let mut v = 0u64;
            for (i, p) in planes.iter().enumerate() {
                if (p[0] >> j) & 1 == 1 {
                    v |= 1u64 << i;
                }
            }
            v
        })
        .collect()
}

/// THE ARITHMETIC FLAGSHIP: bit-serial ripple-carry ADD, composed from the boolean basis over
/// bitplanes, computes element-wise integer addition (mod 2^B) across all 64 lanes — ≡ the
/// arithmetic reference. Integer arithmetic emerging from in-DRAM boolean dataflow.
#[test]
fn bit_serial_add_equals_integer_addition_across_lanes() {
    let bits = 8usize;
    let lanes = 64usize;
    let modulus = 1u64 << bits;
    let mut rng = lcg(0x5151_2323_dead_0001);
    for _ in 0..50 {
        let a: Vec<u64> = (0..lanes).map(|_| rng() % modulus).collect();
        let b: Vec<u64> = (0..lanes).map(|_| rng() % modulus).collect();

        let mut df = DramDataflow::new(2 * bits + 1);
        let a_idx: Vec<usize> = (0..bits).collect();
        let b_idx: Vec<usize> = (bits..2 * bits).collect();
        let carry0 = 2 * bits;
        let sum_idx = bit_serial_add(&mut df, &a_idx, &b_idx, carry0);

        let mut inputs: Vec<Row> = Vec::new();
        inputs.extend(to_planes(&a, bits));
        inputs.extend(to_planes(&b, bits));
        inputs.push(vec![0u64]); // carry0 = zero row
        let heap = df.run(&inputs, 1);

        let sum_planes: Vec<Row> = sum_idx.iter().map(|&r| heap[r].clone()).collect();
        let got = from_planes(&sum_planes, lanes);
        for j in 0..lanes {
            assert_eq!(
                got[j],
                (a[j] + b[j]) % modulus,
                "lane {j}: {} + {}",
                a[j],
                b[j]
            );
        }
    }
}

/// Bit-serial two's-complement SUBTRACT computes element-wise integer subtraction (mod 2^B) across
/// all 64 lanes — ≡ the arithmetic reference. Subtraction from the same in-DRAM boolean basis.
#[test]
fn bit_serial_sub_equals_integer_subtraction_across_lanes() {
    let bits = 8usize;
    let lanes = 64usize;
    let modulus = 1u64 << bits;
    let mut rng = lcg(0x7ea5_9001_dead_beef);
    for _ in 0..50 {
        let a: Vec<u64> = (0..lanes).map(|_| rng() % modulus).collect();
        let b: Vec<u64> = (0..lanes).map(|_| rng() % modulus).collect();

        let mut df = DramDataflow::new(2 * bits + 1);
        let a_idx: Vec<usize> = (0..bits).collect();
        let b_idx: Vec<usize> = (bits..2 * bits).collect();
        let ones = 2 * bits;
        let diff_idx = bit_serial_sub(&mut df, &a_idx, &b_idx, ones);

        let mut inputs: Vec<Row> = Vec::new();
        inputs.extend(to_planes(&a, bits));
        inputs.extend(to_planes(&b, bits));
        inputs.push(vec![!0u64]); // carry-in = 1 for every lane (all-ones row)
        let heap = df.run(&inputs, 1);

        let diff_planes: Vec<Row> = diff_idx.iter().map(|&r| heap[r].clone()).collect();
        let got = from_planes(&diff_planes, lanes);
        for j in 0..lanes {
            assert_eq!(
                got[j],
                (a[j] + modulus - b[j]) % modulus,
                "lane {j}: {} - {}",
                a[j],
                b[j]
            );
        }
    }
}

/// Bit-serial shift-add MULTIPLY computes element-wise integer product across all 64 lanes — ≡ the
/// arithmetic reference. Multiplication from the same in-DRAM boolean basis (AND + adds).
#[test]
fn bit_serial_mul_equals_integer_multiplication_across_lanes() {
    let bits = 4usize; // operands in [0,16): product < 256 fits out_width=8 exactly
    let out_width = 8usize;
    let lanes = 64usize;
    let inmod = 1u64 << bits;
    let outmod = 1u64 << out_width;
    let mut rng = lcg(0x3141_5926_5358_9793);
    for _ in 0..40 {
        let a: Vec<u64> = (0..lanes).map(|_| rng() % inmod).collect();
        let b: Vec<u64> = (0..lanes).map(|_| rng() % inmod).collect();

        let mut df = DramDataflow::new(2 * bits + 1);
        let a_idx: Vec<usize> = (0..bits).collect();
        let b_idx: Vec<usize> = (bits..2 * bits).collect();
        let zero = 2 * bits;
        let p_idx = bit_serial_mul(&mut df, &a_idx, &b_idx, out_width, zero);

        let mut inputs: Vec<Row> = Vec::new();
        inputs.extend(to_planes(&a, bits));
        inputs.extend(to_planes(&b, bits));
        inputs.push(vec![0u64]);
        let heap = df.run(&inputs, 1);

        let p_planes: Vec<Row> = p_idx.iter().map(|&r| heap[r].clone()).collect();
        let got = from_planes(&p_planes, lanes);
        for j in 0..lanes {
            assert_eq!(
                got[j],
                (a[j] * b[j]) % outmod,
                "lane {j}: {} * {}",
                a[j],
                b[j]
            );
        }
    }
}
