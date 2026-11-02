use super::*;
use logicaffeine_base::Lanes4Word64 as L;

/// 4-way Keccak-f[1600] written on the `Lanes4Word64` newtype ops (`bitxor`/`rotl`/`andnot`/`splat`)
/// — the exact form Logos `crypto.lg` would compile to. Fully unrolled; θ/ρ·π/χ/ι over 4 states.
fn keccak_f1600_x4_lanes(a: &mut [L; 25]) {
    for &rc in RC.iter() {
        let c0 = a[0].bitxor(a[5]).bitxor(a[10]).bitxor(a[15]).bitxor(a[20]);
        let c1 = a[1].bitxor(a[6]).bitxor(a[11]).bitxor(a[16]).bitxor(a[21]);
        let c2 = a[2].bitxor(a[7]).bitxor(a[12]).bitxor(a[17]).bitxor(a[22]);
        let c3 = a[3].bitxor(a[8]).bitxor(a[13]).bitxor(a[18]).bitxor(a[23]);
        let c4 = a[4].bitxor(a[9]).bitxor(a[14]).bitxor(a[19]).bitxor(a[24]);
        let d0 = c4.bitxor(c1.rotl(1));
        let d1 = c0.bitxor(c2.rotl(1));
        let d2 = c1.bitxor(c3.rotl(1));
        let d3 = c2.bitxor(c4.rotl(1));
        let d4 = c3.bitxor(c0.rotl(1));
        a[0] = a[0].bitxor(d0);
        a[5] = a[5].bitxor(d0);
        a[10] = a[10].bitxor(d0);
        a[15] = a[15].bitxor(d0);
        a[20] = a[20].bitxor(d0);
        a[1] = a[1].bitxor(d1);
        a[6] = a[6].bitxor(d1);
        a[11] = a[11].bitxor(d1);
        a[16] = a[16].bitxor(d1);
        a[21] = a[21].bitxor(d1);
        a[2] = a[2].bitxor(d2);
        a[7] = a[7].bitxor(d2);
        a[12] = a[12].bitxor(d2);
        a[17] = a[17].bitxor(d2);
        a[22] = a[22].bitxor(d2);
        a[3] = a[3].bitxor(d3);
        a[8] = a[8].bitxor(d3);
        a[13] = a[13].bitxor(d3);
        a[18] = a[18].bitxor(d3);
        a[23] = a[23].bitxor(d3);
        a[4] = a[4].bitxor(d4);
        a[9] = a[9].bitxor(d4);
        a[14] = a[14].bitxor(d4);
        a[19] = a[19].bitxor(d4);
        a[24] = a[24].bitxor(d4);
        let b0 = a[0];
        let b1 = a[6].rotl(44);
        let b2 = a[12].rotl(43);
        let b3 = a[18].rotl(21);
        let b4 = a[24].rotl(14);
        let b5 = a[3].rotl(28);
        let b6 = a[9].rotl(20);
        let b7 = a[10].rotl(3);
        let b8 = a[16].rotl(45);
        let b9 = a[22].rotl(61);
        let b10 = a[1].rotl(1);
        let b11 = a[7].rotl(6);
        let b12 = a[13].rotl(25);
        let b13 = a[19].rotl(8);
        let b14 = a[20].rotl(18);
        let b15 = a[4].rotl(27);
        let b16 = a[5].rotl(36);
        let b17 = a[11].rotl(10);
        let b18 = a[17].rotl(15);
        let b19 = a[23].rotl(56);
        let b20 = a[2].rotl(62);
        let b21 = a[8].rotl(55);
        let b22 = a[14].rotl(39);
        let b23 = a[15].rotl(41);
        let b24 = a[21].rotl(2);
        a[0] = b0.bitxor(b1.andnot(b2));
        a[1] = b1.bitxor(b2.andnot(b3));
        a[2] = b2.bitxor(b3.andnot(b4));
        a[3] = b3.bitxor(b4.andnot(b0));
        a[4] = b4.bitxor(b0.andnot(b1));
        a[5] = b5.bitxor(b6.andnot(b7));
        a[6] = b6.bitxor(b7.andnot(b8));
        a[7] = b7.bitxor(b8.andnot(b9));
        a[8] = b8.bitxor(b9.andnot(b5));
        a[9] = b9.bitxor(b5.andnot(b6));
        a[10] = b10.bitxor(b11.andnot(b12));
        a[11] = b11.bitxor(b12.andnot(b13));
        a[12] = b12.bitxor(b13.andnot(b14));
        a[13] = b13.bitxor(b14.andnot(b10));
        a[14] = b14.bitxor(b10.andnot(b11));
        a[15] = b15.bitxor(b16.andnot(b17));
        a[16] = b16.bitxor(b17.andnot(b18));
        a[17] = b17.bitxor(b18.andnot(b19));
        a[18] = b18.bitxor(b19.andnot(b15));
        a[19] = b19.bitxor(b15.andnot(b16));
        a[20] = b20.bitxor(b21.andnot(b22));
        a[21] = b21.bitxor(b22.andnot(b23));
        a[22] = b22.bitxor(b23.andnot(b24));
        a[23] = b23.bitxor(b24.andnot(b20));
        a[24] = b24.bitxor(b20.andnot(b21));
        a[0] = a[0].bitxor(L::splat(rc));
    }
}

#[test]
fn x4_lanes_matches_raw_and_ab() {
    use std::arch::x86_64::*;
    use std::time::Instant;
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut rnd = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed
    };
    let states: [[u64; 25]; 4] = std::array::from_fn(|_| std::array::from_fn(|_| rnd()));
    let mut lanes: [L; 25] =
        std::array::from_fn(|i| L([states[0][i], states[1][i], states[2][i], states[3][i]]));
    unsafe {
        let mut raw: [__m256i; 25] = std::array::from_fn(|i| {
            _mm256_set_epi64x(
                states[3][i] as i64,
                states[2][i] as i64,
                states[1][i] as i64,
                states[0][i] as i64,
            )
        });
        keccak_f1600_x4(&mut raw);
        keccak_f1600_x4_lanes(&mut lanes);
        for i in 0..25 {
            let mut got = [0u64; 4];
            _mm256_storeu_si256(got.as_mut_ptr() as *mut __m256i, raw[i]);
            assert_eq!(
                lanes[i].0, got,
                "Lanes4Word64 x4 Keccak must equal raw keccak_f1600_x4 (lane {i})"
            );
        }
        // Also confirm lane 0 equals a single scalar keccak_f1600 of state 0.
        let mut sc = states[0];
        keccak_f1600(&mut sc);
        let l0: [u64; 25] = std::array::from_fn(|i| lanes[i].0[0]);
        assert_eq!(
            l0, sc,
            "lane 0 of the x4 must equal one scalar keccak_f1600"
        );

        // A/B: raw intrinsics vs lane-newtype, same run.
        macro_rules! t {
            ($l:expr, $init:expr, $body:expr) => {{
                for _ in 0..100 {
                    let mut s = $init;
                    std::hint::black_box($body(&mut s));
                }
                let start = Instant::now();
                for _ in 0..20000u32 {
                    let mut s = $init;
                    std::hint::black_box($body(&mut s));
                }
                eprintln!(
                    "{:<22} {:>8.1} ns/op",
                    $l,
                    start.elapsed().as_nanos() as f64 / 20000.0
                );
            }};
        }
        let raw0: [__m256i; 25] = std::array::from_fn(|i| {
            _mm256_set_epi64x(
                states[3][i] as i64,
                states[2][i] as i64,
                states[1][i] as i64,
                states[0][i] as i64,
            )
        });
        let lanes0: [L; 25] =
            std::array::from_fn(|i| L([states[0][i], states[1][i], states[2][i], states[3][i]]));
        eprintln!("\n=== 4-way Keccak-f[1600] A/B (this box) ===");
        t!("raw keccak_f1600_x4", raw0, |s: &mut [__m256i; 25]| {
            keccak_f1600_x4(s)
        });
        t!("lanes x4 (Logos form)", lanes0, |s: &mut [L; 25]| {
            keccak_f1600_x4_lanes(s)
        });
    }
}
