//! Rainbow verification written in LOGOS. The GF(2^8) field and the public-map evaluation live in
//! `assets/std/crypto.lg` with NO native kernel, so `rainbowVerify` runs identically on the
//! tree-walker, the VM, and native code. The faithful Rainbow keygen/sign reference
//! (`logicaffeine_proof::rainbow`, itself validated in that crate) produces a real public key and a
//! real signature; the LOGOS verifier must accept it and reject any tampering — proving Ding &
//! Schmidt's multivariate signature verifies inside the language, at every tier.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{run_all_tiers, run_logos_with_args};
use logicaffeine_base::gf256;
use logicaffeine_proof::rainbow::{self, KeygenInputs, RainbowParams, SecretFlat};

/// Deterministic splitmix64 for building random GF(256) systems.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
}

const PARAMS: RainbowParams = RainbowParams {
    v1: 2,
    o1: 2,
    o2: 2,
};

fn lit(v: &[u8]) -> String {
    v.iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A deterministic message hash of `m` field elements.
fn msg_hash(tag: u64, m: usize) -> Vec<u8> {
    let mut s = tag ^ 0x243F_6A88_85A3_08D3;
    (0..m)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 33) as u8
        })
        .collect()
}

fn verify_program(pk_flat: &(Vec<u8>, Vec<u8>, Vec<u8>), sig: &[u8], msg: &[u8]) -> String {
    let (pubq, publin, pubc) = pk_flat;
    format!(
        "## Main\n\
         Let pubq be [{pubq}].\n\
         Let publin be [{publin}].\n\
         Let pubc be [{pubc}].\n\
         Let x be [{x}].\n\
         Let msg be [{msg}].\n\
         Show rainbowVerify(pubq, publin, pubc, x, {n}, {m}, msg).\n",
        pubq = lit(pubq),
        publin = lit(publin),
        pubc = lit(pubc),
        x = lit(sig),
        msg = lit(msg),
        n = PARAMS.n(),
        m = PARAMS.m(),
    )
}

/// Run on the tree-walker AND the VM, assert they agree, return the single-line verdict.
fn tw_vm_verdict(prog: &str) -> String {
    let agreed = run_all_tiers(prog);
    assert!(
        agreed.error.is_none(),
        "program errored: {}",
        agreed.error.unwrap_or_default()
    );
    agreed.output.trim().to_string()
}

/// Run on the tree-walker AND the VM, assert they agree, parse the `Show` output as ints.
fn tw_vm_ints(prog: &str) -> Vec<i64> {
    let agreed = run_all_tiers(prog);
    assert!(
        agreed.error.is_none(),
        "program errored: {}",
        agreed.error.unwrap_or_default()
    );
    agreed
        .output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse().expect("int"))
        .collect()
}

/// Deterministically search for vinegar values that keep both Oil-and-Vinegar layers invertible.
fn working_vinegars(sk: &rainbow::SecretKey, msg: &[u8], v1: usize) -> Vec<u8> {
    let mut s = 0x1357_9BDF_2468_ACE0u64;
    for _ in 0..2000 {
        let vin: Vec<u8> = (0..v1)
            .map(|_| {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
                (s >> 40) as u8
            })
            .collect();
        if rainbow::sign_with_vinegars(sk, msg, &vin).is_some() {
            return vin;
        }
    }
    panic!("no non-singular vinegars found");
}

/// The LOGOS preamble binding the flattened secret key.
fn secret_preamble(sf: &SecretFlat) -> String {
    format!(
        "Let cq be [{}].\nLet cl be [{}].\nLet cc be [{}].\nLet sinv be [{}].\nLet svec be [{}].\nLet tinv be [{}].\nLet tvec be [{}].\n",
        lit(&sf.central_q),
        lit(&sf.central_lin),
        lit(&sf.central_c),
        lit(&sf.s_inv),
        lit(&sf.s_vec),
        lit(&sf.t_inv),
        lit(&sf.t_vec),
    )
}

/// The `rainbowSign(...)` call with this parameter set's layer sizes.
fn sign_call() -> String {
    format!(
        "rainbowSign(cq, cl, cc, sinv, svec, tinv, tvec, {n}, {m}, {v1}, {o1}, {o2}, msg, vinegars)",
        n = PARAMS.n(),
        m = PARAMS.m(),
        v1 = PARAMS.v1,
        o1 = PARAMS.o1,
        o2 = PARAMS.o2,
    )
}

/// The LOGOS preamble binding the forward affine maps + central map (the keygen inputs).
fn keygen_preamble(ki: &KeygenInputs) -> String {
    format!(
        "Let tmat be [{}].\nLet tvec be [{}].\nLet smat be [{}].\nLet svec be [{}].\nLet cq be [{}].\nLet cl be [{}].\nLet cc be [{}].\n",
        lit(&ki.t_mat),
        lit(&ki.t_vec),
        lit(&ki.s_mat),
        lit(&ki.s_vec),
        lit(&ki.central_q),
        lit(&ki.central_lin),
        lit(&ki.central_c),
    )
}

fn keygen_call() -> String {
    keygen_call_p(PARAMS)
}

/// Parameter-explicit call builders, so the same LOGOS functions can be exercised at asymmetric
/// layer sizes (the coverage that catches `v1`/`o1`/`o2` / `l1`-offset confusion).
fn sign_call_p(p: RainbowParams) -> String {
    format!(
        "rainbowSign(cq, cl, cc, sinv, svec, tinv, tvec, {n}, {m}, {v1}, {o1}, {o2}, msg, vinegars)",
        n = p.n(),
        m = p.m(),
        v1 = p.v1,
        o1 = p.o1,
        o2 = p.o2,
    )
}

fn keygen_call_p(p: RainbowParams) -> String {
    format!(
        "rainbowKeygen(tmat, tvec, smat, svec, cq, cl, cc, {n}, {m})",
        n = p.n(),
        m = p.m()
    )
}

/// Preamble for the whole-scheme roundtrip: keygen inputs + the affine inverses signing needs.
fn full_preamble(ki: &KeygenInputs, sf: &SecretFlat) -> String {
    format!(
        "{}Let sinv be [{}].\nLet tinv be [{}].\n",
        keygen_preamble(ki),
        lit(&sf.s_inv),
        lit(&sf.t_inv)
    )
}

#[test]
fn logos_gf256_field_matches_the_reference_over_the_entire_field_across_tiers() {
    // Exhaustive proof of the LOGOS field primitives (the foundation the whole scheme stands on): a
    // polynomial hash of gf256Mul over ALL 65,536 products and of gf256Inv over all 255 nonzero
    // elements, computed in LOGOS, must equal the same hash of the verified base::gf256 — identically
    // on tree-walker and VM. A single wrong product or inverse changes the hash. (The AOT tier
    // exercises these same primitives compiled to native code via the whole-scheme AOT gates.)
    const MOD: i64 = 1_000_000_007;
    let mut mh: i64 = 0;
    for a in 0..256u16 {
        for b in 0..256u16 {
            mh = (mh * 257 + gf256::mul(a as u8, b as u8) as i64) % MOD;
        }
    }
    let mut ih: i64 = 0;
    for a in 1..256u16 {
        ih = (ih * 257 + gf256::inv(a as u8) as i64) % MOD;
    }
    let prog = "## Main\n\
        Let mutable mh be 0.\n\
        Repeat for a from 0 to 255:\n    Repeat for b from 0 to 255:\n        Set mh to ((mh * 257 + gf256Mul(a, b)) % 1000000007).\n\
        Show mh.\n\
        Let mutable ih be 0.\n\
        Repeat for a from 1 to 255:\n    Set ih to ((ih * 257 + gf256Inv(a)) % 1000000007).\n\
        Show ih.\n";
    assert_eq!(
        tw_vm_ints(prog),
        vec![mh, ih],
        "LOGOS gf256Mul/gf256Inv must match base::gf256 over the ENTIRE field"
    );
}

#[test]
fn logos_gf256_solve_matches_the_reference_on_random_invertible_systems_across_tiers() {
    // The LOGOS Gaussian solver, isolated from Rainbow: for random invertible A and random b over
    // GF(256), gf256Solve must return the reference solution (which also satisfies A·x = b), tw == vm.
    let mut r = Rng(0x0050_1FED_0000_0001);
    for k in 2..=5usize {
        for _ in 0..6 {
            let a: Vec<Vec<u8>> = loop {
                let m: Vec<Vec<u8>> = (0..k).map(|_| (0..k).map(|_| r.byte()).collect()).collect();
                if gf256::invert(&m).is_some() {
                    break m;
                }
            };
            let b: Vec<u8> = (0..k).map(|_| r.byte()).collect();
            let want = gf256::solve(&a, &b).expect("invertible system solves");
            assert_eq!(
                gf256::matvec(&a, &want),
                b,
                "reference solution must satisfy A·x = b"
            );
            let aflat: Vec<u8> = a.iter().flatten().copied().collect();
            let prog = format!(
                "## Main\nLet aflat be [{}].\nLet b be [{}].\nLet x be gf256Solve(aflat, b, {k}).\n\
                 Repeat for i from 1 to {k}:\n    Show item i of x.\n",
                lit(&aflat),
                lit(&b),
                k = k,
            );
            let want_i: Vec<i64> = want.iter().map(|&x| x as i64).collect();
            assert_eq!(
                tw_vm_ints(&prog),
                want_i,
                "LOGOS gf256Solve must match base::gf256::solve (k={k})"
            );
        }
    }
}

#[test]
fn logos_gf256_matvec_matches_the_reference_across_tiers() {
    // gf256Matvec (the affine maps S⁻¹/T⁻¹ applied during signing) proven directly against the
    // verified base::gf256::matvec, at non-square shapes, tw == vm.
    let mut r = Rng(0x0BA7_0000_0000_0001);
    for &(rows, cols) in &[(2usize, 3usize), (4, 4), (5, 2), (3, 6), (6, 5)] {
        for _ in 0..4 {
            let mat: Vec<Vec<u8>> = (0..rows)
                .map(|_| (0..cols).map(|_| r.byte()).collect())
                .collect();
            let v: Vec<u8> = (0..cols).map(|_| r.byte()).collect();
            let want: Vec<i64> = gf256::matvec(&mat, &v).iter().map(|&x| x as i64).collect();
            let mflat: Vec<u8> = mat.iter().flatten().copied().collect();
            let prog = format!(
                "## Main\nLet m be [{}].\nLet v be [{}].\nLet r be gf256Matvec(m, v, {rows}, {cols}).\n\
                 Repeat for i from 1 to {rows}:\n    Show item i of r.\n",
                lit(&mflat),
                lit(&v),
                rows = rows,
                cols = cols,
            );
            assert_eq!(
                tw_vm_ints(&prog),
                want,
                "gf256Matvec {rows}x{cols} must match the reference"
            );
        }
    }
}

#[test]
fn rainbow_verify_accepts_valid_signatures_and_rejects_tampering_across_tiers() {
    let m = PARAMS.m();
    for seed in 0..6u64 {
        let (sk, pk) = rainbow::keygen(PARAMS, 0x5A1B_0000 ^ seed);
        let msg = msg_hash(0xF00D ^ seed, m);
        let sig = rainbow::sign(&sk, &msg).expect("reference sign");
        // Sanity: the reference verifier agrees the signature is valid.
        assert!(
            rainbow::verify(&pk, &msg, &sig),
            "seed {seed}: reference must accept its own signature"
        );
        let flat = rainbow::flatten_public(&pk);

        // A valid signature verifies to 1 on both tiers.
        assert_eq!(
            tw_vm_verdict(&verify_program(&flat, &sig, &msg)),
            "1",
            "seed {seed}: valid must verify"
        );

        // Every single-byte tamper is rejected (verdict 0).
        for pos in 0..sig.len() {
            let mut bad = sig.clone();
            bad[pos] ^= 0x53;
            if bad == sig {
                continue;
            }
            assert_eq!(
                tw_vm_verdict(&verify_program(&flat, &bad, &msg)),
                "0",
                "seed {seed}: tampering byte {pos} must be rejected"
            );
        }

        // A signature for one message must not verify against a different message.
        let other = msg_hash(0xBEEF ^ seed, m);
        if other != msg {
            assert_eq!(
                tw_vm_verdict(&verify_program(&flat, &sig, &other)),
                "0",
                "seed {seed}: message-bound"
            );
        }
    }
}

#[test]
fn rainbow_sign_in_logos_matches_the_reference_and_self_verifies_across_tiers() {
    let (v1, n, m) = (PARAMS.v1, PARAMS.n(), PARAMS.m());
    for seed in 0..4u64 {
        let (sk, pk) = rainbow::keygen(PARAMS, 0x051A_0000 ^ seed);
        let msg = msg_hash(0x5164 ^ seed, m);
        let vinegars = working_vinegars(&sk, &msg, v1);
        let ref_sig =
            rainbow::sign_with_vinegars(&sk, &msg, &vinegars).expect("working vinegars sign");
        assert!(
            rainbow::verify(&pk, &msg, &ref_sig),
            "seed {seed}: reference signature must verify"
        );
        let sf = rainbow::flatten_secret(&sk);

        // (1) LOGOS signing byte-matches the reference, identically on the tree-walker and the VM.
        let sign_prog = format!(
            "## Main\n{pre}Let msg be [{msg}].\nLet vinegars be [{vin}].\nLet sig be {call}.\n\
             Repeat for i from 1 to {n}:\n    Show item i of sig.\n",
            pre = secret_preamble(&sf),
            msg = lit(&msg),
            vin = lit(&vinegars),
            call = sign_call(),
            n = n,
        );
        let logos_sig = tw_vm_ints(&sign_prog);
        let want: Vec<i64> = ref_sig.iter().map(|&b| b as i64).collect();
        assert_eq!(
            logos_sig, want,
            "seed {seed}: LOGOS rainbowSign must match the reference byte-for-byte"
        );

        // (2) The LOGOS-produced signature verifies under LOGOS rainbowVerify — a full in-language
        //     sign → verify roundtrip.
        let (pubq, publin, pubc) = rainbow::flatten_public(&pk);
        let roundtrip = format!(
            "## Main\n{pre}Let msg be [{msg}].\nLet vinegars be [{vin}].\nLet sig be {call}.\n\
             Let pubq be [{pq}].\nLet publin be [{pl}].\nLet pubc be [{pc}].\n\
             Show rainbowVerify(pubq, publin, pubc, sig, {n}, {mm}, msg).\n",
            pre = secret_preamble(&sf),
            msg = lit(&msg),
            vin = lit(&vinegars),
            call = sign_call(),
            pq = lit(&pubq),
            pl = lit(&publin),
            pc = lit(&pubc),
            n = n,
            mm = m,
        );
        assert_eq!(
            tw_vm_verdict(&roundtrip),
            "1",
            "seed {seed}: a LOGOS-signed signature must LOGOS-verify"
        );
    }
}

#[test]
fn rainbow_keygen_in_logos_matches_the_reference_public_key() {
    // Pit-of-success lock: the LOGOS composition P = S∘F∘T must equal the reference public key,
    // byte-for-byte, for every random (S, F, T) — identically on the tree-walker and the VM.
    for seed in 0..5u64 {
        let (sk, pk) = rainbow::keygen(PARAMS, 0x7E3E_0000 ^ seed);
        let ki = rainbow::flatten_keygen(&sk);
        let prog = format!(
            "## Main\n{pre}Let pk be {call}.\nRepeat for i from 1 to length of pk:\n    Show item i of pk.\n",
            pre = keygen_preamble(&ki),
            call = keygen_call(),
        );
        let logos_pub = tw_vm_ints(&prog);
        let (pq, pl, pc) = rainbow::flatten_public(&pk);
        let want: Vec<i64> = pq.iter().chain(&pl).chain(&pc).map(|&b| b as i64).collect();
        assert_eq!(
            logos_pub, want,
            "seed {seed}: LOGOS keygen must equal the reference public key"
        );
    }
}

#[test]
fn rainbow_whole_scheme_keygen_sign_verify_runs_in_logos_across_tiers() {
    // The WHOLE scheme in one LOGOS program: compose the public key, sign a message, verify the
    // signature against that LOGOS-composed key — all in-language, tree-walker == VM.
    let (v1, n, m) = (PARAMS.v1, PARAMS.n(), PARAMS.m());
    for seed in 0..3u64 {
        let (sk, pk) = rainbow::keygen(PARAMS, 0x0077_0000 ^ seed);
        let msg = msg_hash(0xF11E ^ seed, m);
        let vinegars = working_vinegars(&sk, &msg, v1);
        let ki = rainbow::flatten_keygen(&sk);
        let sf = rainbow::flatten_secret(&sk);

        let prog = format!(
            "## Main\n{pre}Let msg be [{msg}].\nLet vinegars be [{vin}].\n\
             Let pubkey be {kcall}.\nLet sig be {scall}.\n\
             Show rainbowVerifyPacked(pubkey, sig, {n}, {m}, msg).\n",
            pre = full_preamble(&ki, &sf),
            msg = lit(&msg),
            vin = lit(&vinegars),
            kcall = keygen_call(),
            scall = sign_call(),
            n = n,
            m = m,
        );
        assert_eq!(
            tw_vm_verdict(&prog),
            "1",
            "seed {seed}: the whole LOGOS scheme must self-verify"
        );

        // Sanity that the LOGOS-composed key is the real key: it must reject a tampered signature.
        let sig = rainbow::sign_with_vinegars(&sk, &msg, &vinegars).unwrap();
        assert!(rainbow::verify(&pk, &msg, &sig));
        let mut bad = sig.clone();
        bad[0] ^= 0x53;
        let tamper = format!(
            "## Main\n{pre}Let msg be [{msg}].\nLet pubkey be {kcall}.\nLet sig be [{badsig}].\n\
             Show rainbowVerifyPacked(pubkey, sig, {n}, {m}, msg).\n",
            pre = keygen_preamble(&ki),
            msg = lit(&msg),
            kcall = keygen_call(),
            badsig = lit(&bad),
            n = n,
            m = m,
        );
        assert_eq!(
            tw_vm_verdict(&tamper),
            "0",
            "seed {seed}: LOGOS-composed key must reject tampering"
        );
    }
}

/// Asymmetric layer sizes (all n=6 so the interpreter stays fast) — distinct `m` and layer
/// distributions that expose any index confusion a symmetric `(2,2,2)` instance would hide.
const ASYM_PARAMS: &[RainbowParams] = &[
    RainbowParams {
        v1: 3,
        o1: 2,
        o2: 1,
    },
    RainbowParams {
        v1: 1,
        o1: 3,
        o2: 2,
    },
    RainbowParams {
        v1: 2,
        o1: 1,
        o2: 3,
    },
];

#[test]
fn rainbow_whole_scheme_matches_the_reference_at_asymmetric_parameters_across_tiers() {
    for &p in ASYM_PARAMS {
        let (n, m, v1) = (p.n(), p.m(), p.v1);
        for seed in 0..2u64 {
            let base = 0xA5F0_0000 ^ seed ^ ((p.v1 * 10_000 + p.o1 * 100 + p.o2) as u64);
            let (sk, pk) = rainbow::keygen(p, base);
            let msg = msg_hash(0xC0DE ^ seed, m);
            let vinegars = working_vinegars(&sk, &msg, v1);
            let ki = rainbow::flatten_keygen(&sk);
            let sf = rainbow::flatten_secret(&sk);

            // (1) LOGOS keygen == reference public key.
            let kprog = format!(
                "## Main\n{pre}Let pk be {call}.\nRepeat for i from 1 to length of pk:\n    Show item i of pk.\n",
                pre = keygen_preamble(&ki),
                call = keygen_call_p(p),
            );
            let logos_pub = tw_vm_ints(&kprog);
            let (pq, pl, pc) = rainbow::flatten_public(&pk);
            let want: Vec<i64> = pq.iter().chain(&pl).chain(&pc).map(|&b| b as i64).collect();
            assert_eq!(
                logos_pub, want,
                "{p:?} seed {seed}: LOGOS keygen must match the reference public key"
            );

            // (2) LOGOS sign == reference sign.
            let ref_sig = rainbow::sign_with_vinegars(&sk, &msg, &vinegars).expect("sign");
            let sprog = format!(
                "## Main\n{pre}Let msg be [{msg}].\nLet vinegars be [{vin}].\nLet sig be {scall}.\n\
                 Repeat for i from 1 to {n}:\n    Show item i of sig.\n",
                pre = full_preamble(&ki, &sf),
                msg = lit(&msg),
                vin = lit(&vinegars),
                scall = sign_call_p(p),
                n = n,
            );
            let logos_sig = tw_vm_ints(&sprog);
            let want_sig: Vec<i64> = ref_sig.iter().map(|&b| b as i64).collect();
            assert_eq!(
                logos_sig, want_sig,
                "{p:?} seed {seed}: LOGOS sign must match the reference"
            );

            // (3) Whole scheme keygen → sign → verify in one LOGOS program.
            let prog = format!(
                "## Main\n{pre}Let msg be [{msg}].\nLet vinegars be [{vin}].\n\
                 Let pubkey be {kcall}.\nLet sig be {scall}.\n\
                 Show rainbowVerifyPacked(pubkey, sig, {n}, {m}, msg).\n",
                pre = full_preamble(&ki, &sf),
                msg = lit(&msg),
                vin = lit(&vinegars),
                kcall = keygen_call_p(p),
                scall = sign_call_p(p),
                n = n,
                m = m,
            );
            assert_eq!(
                tw_vm_verdict(&prog),
                "1",
                "{p:?} seed {seed}: whole LOGOS scheme must self-verify"
            );
        }
    }
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT whole-scheme gate"]
fn rainbow_whole_scheme_holds_at_the_aot_tier() {
    let (v1, n, m) = (PARAMS.v1, PARAMS.n(), PARAMS.m());
    let (sk, _pk) = rainbow::keygen(PARAMS, 0x0077_A07);
    let msg = msg_hash(0xF11E_A07, m);
    let vinegars = working_vinegars(&sk, &msg, v1);
    let ki = rainbow::flatten_keygen(&sk);
    let sf = rainbow::flatten_secret(&sk);
    let prog = format!(
        "## Main\n{pre}Let msg be [{msg}].\nLet vinegars be [{vin}].\n\
         Let pubkey be {kcall}.\nLet sig be {scall}.\n\
         Show rainbowVerifyPacked(pubkey, sig, {n}, {m}, msg).\n",
        pre = full_preamble(&ki, &sf),
        msg = lit(&msg),
        vin = lit(&vinegars),
        kcall = keygen_call(),
        scall = sign_call(),
        n = n,
        m = m,
    );
    let out = run_logos_with_args(&prog, &[]);
    assert!(out.success, "AOT failed:\n{}", out.stderr);
    assert_eq!(
        out.stdout.trim(),
        "1",
        "AOT: whole LOGOS scheme must self-verify"
    );
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT dead-crypto-code robustness gate"]
fn a_minimal_gf256_program_compiles_at_aot_under_the_whole_crypto_prelude() {
    // Pit-of-success lock for the ENTIRE emitted crypto surface. Touching a single crypto
    // primitive (`gf256Mul`) demand-splices all of `crypto.lg` — including the whole ML-DSA
    // implementation — into the program, and the AOT backend emits every one of those ~170
    // functions even though this program uses none of them. So a latent codegen fault in ANY
    // unused crypto function (the class of bug this guards: a borrow-optimizer def/call
    // signature divergence in an accumulator like `mldsaIntsToW32`, which once produced an
    // `expected LogosSeq<i64>, found Vec<i64>` E0308) breaks this unrelated program's native
    // build. The smallest possible gf256 program compiling green proves the whole dead-code
    // surface is type-consistent — it fails LOUD (a rustc error) the instant that regresses.
    let out = run_logos_with_args("## Main\nLet x be gf256Mul(2, 3).\nShow x.\n", &[]);
    assert!(
        out.success,
        "AOT compile of a minimal gf256 program failed — a dead crypto function miscompiled:\n{}",
        out.stderr
    );
    assert_eq!(
        out.stdout.trim(),
        "6",
        "gf256Mul(2, 3) must be 6 at the AOT tier"
    );
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT Rainbow-verify gate"]
fn rainbow_verify_holds_at_the_aot_tier() {
    let m = PARAMS.m();
    let (sk, pk) = rainbow::keygen(PARAMS, 0x5A1B_A07);
    let msg = msg_hash(0xF00D_A07, m);
    let sig = rainbow::sign(&sk, &msg).expect("reference sign");
    let flat = rainbow::flatten_public(&pk);

    let good = run_logos_with_args(&verify_program(&flat, &sig, &msg), &[]);
    assert!(good.success, "AOT failed:\n{}", good.stderr);
    assert_eq!(good.stdout.trim(), "1", "AOT: valid signature must verify");

    let mut bad = sig.clone();
    bad[0] ^= 0x53;
    let tampered = run_logos_with_args(&verify_program(&flat, &bad, &msg), &[]);
    assert!(tampered.success, "AOT failed:\n{}", tampered.stderr);
    assert_eq!(
        tampered.stdout.trim(),
        "0",
        "AOT: tampered signature must be rejected"
    );
}
