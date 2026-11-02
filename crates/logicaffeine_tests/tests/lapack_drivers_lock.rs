//! LOGOS-LAPACK eigensolver + drivers — `syev` (symmetric eig), `det`, `inv`, `lstsq`.
//!
//! `syev` is two-sided cyclic Jacobi (`A ← JᵀAJ`), again pure `sqrt`. The drivers stack on the
//! factorizations: `det` from LU (product of U's diagonal times the pivot sign), `inv` by solving
//! `A·X = I` columnwise through the LU factors, `lstsq` by QR (`x = R⁻¹·Qᵀ·b`). Each is checked by
//! its defining identity — `V·Λ·Vᵀ == A`, `A·inv(A) == I`, an exact least-squares fit — computed
//! in-LOGOS by reusing the BLAS `gemm`/`gemv` kernels. `run_all_tiers` enforces interp==VM.

mod common;
use common::run_all_tiers;

fn float_lit(x: f64) -> String {
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

fn seq_literal(xs: &[f64]) -> String {
    let inner: Vec<String> = xs.iter().map(|x| float_lit(*x)).collect();
    format!("[{}]", inner.join(", "))
}

fn parse_seq_floats(out: &str) -> Vec<f64> {
    let t = out.trim();
    let inner = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or_else(|| panic!("expected a bracketed Seq, got {out:?}"));
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .map(|s| s.trim().parse::<f64>().expect("float element"))
        .collect()
}

fn run_seq(src: &str) -> Vec<f64> {
    let out = run_all_tiers(src);
    assert!(
        out.error.is_none(),
        "program errored: {:?}\n{src}",
        out.error
    );
    parse_seq_floats(&out.output)
}

fn run_scalar(src: &str) -> f64 {
    let out = run_all_tiers(src);
    assert!(
        out.error.is_none(),
        "program errored: {:?}\n{src}",
        out.error
    );
    out.output.trim().parse::<f64>().expect("scalar float")
}

fn assert_close(got: &[f64], want: &[f64], tol: f64) {
    assert_eq!(got.len(), want.len(), "length: got {got:?} want {want:?}");
    for (i, (g, e)) in got.iter().zip(want.iter()).enumerate() {
        assert!(
            (g - e).abs() <= tol * (1.0 + e.abs()),
            "element {i}: got={g} want={e} diff={:.3e}",
            (g - e).abs()
        );
    }
}

// V·Λ·Vᵀ reconstruction: (V·diag(λ)·Vᵀ)[i][l] = Σ_j V[i][j]·λ[j]·V[l][j].
const RECON_SYM: &str = "\
## To reconSym (n: Int, v: Seq of Float, lam: Seq of Float) -> Seq of Float:\n\
\x20   Let mutable out be a new Seq of Float.\n\
\x20   Repeat for i from 1 to n:\n\
\x20       Repeat for l from 1 to n:\n\
\x20           Let mutable acc be 0.0.\n\
\x20           Repeat for j from 1 to n:\n\
\x20               Set acc to acc + (item ((i - 1) * n + j) of v) * (item j of lam) * (item ((l - 1) * n + j) of v).\n\
\x20           Push acc to out.\n\
\x20   Return out.\n";

// ---- syev: symmetric eigendecomposition ----

fn check_syev(n: usize, a: &[f64]) {
    let src = format!(
        "{RECON_SYM}\n## Main\nLet e be syev({n}, {a}).\nLet vv be e's vecs.\nLet ll be e's vals.\n\
         Show reconSym({n}, vv, ll).\n",
        a = seq_literal(a)
    );
    assert_close(&run_seq(&src), a, 1e-8);
}

#[test]
fn syev_reconstructs_2x2() {
    // A=[[2,1],[1,2]] symmetric, eigenvalues 3 and 1.
    check_syev(2, &[2.0, 1.0, 1.0, 2.0]);
}

#[test]
fn syev_reconstructs_3x3() {
    // A=[[2,0,0],[0,3,1],[0,1,3]] symmetric, eigenvalues 2, 2, 4.
    check_syev(3, &[2.0, 0.0, 0.0, 0.0, 3.0, 1.0, 0.0, 1.0, 3.0]);
}

#[test]
fn syev_eigenvalues_of_known_symmetric() {
    // A=[[2,1],[1,2]] → eigenvalues {3, 1} (Jacobi returns them unordered).
    let src = format!(
        "## Main\nLet e be syev(2, {}).\nShow e's vals.\n",
        seq_literal(&[2.0, 1.0, 1.0, 2.0])
    );
    let mut vals = run_seq(&src);
    vals.sort_by(|a, b| b.partial_cmp(a).unwrap());
    assert_close(&vals, &[3.0, 1.0], 1e-8);
}

// ---- det ----

#[test]
fn det_2x2() {
    // det([[1,2],[3,4]]) = 1·4 - 2·3 = -2.
    let d = run_scalar("## Main\nShow det(2, [1.0, 2.0, 3.0, 4.0]).\n");
    assert!((d - (-2.0)).abs() <= 1e-10, "det = {d}");
}

#[test]
fn det_3x3_diagonal() {
    // det(diag(2,3,4)) = 24.
    let d = run_scalar("## Main\nShow det(3, [2.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 4.0]).\n");
    assert!((d - 24.0).abs() <= 1e-10, "det = {d}");
}

// ---- inv: A·inv(A) == I ----

#[test]
fn inv_times_a_is_identity() {
    // A=[[4,7],[2,6]]; verify A·inv(A) ≈ I via gemm.
    let a = seq_literal(&[4.0, 7.0, 2.0, 6.0]);
    let src = format!(
        "## Main\nLet ainv be inv(2, {a}).\nShow gemm(2, 2, 2, 1.0, {a}, ainv, 0.0, [0.0, 0.0, 0.0, 0.0]).\n"
    );
    assert_close(&run_seq(&src), &[1.0, 0.0, 0.0, 1.0], 1e-9);
}

// ---- lstsq: least squares via QR ----

#[test]
fn lstsq_exact_overdetermined_fit() {
    // Fit the line y = 1 + x through (0,1),(1,2),(2,3): A=[[1,0],[1,1],[1,2]], b=[1,2,3] → x=[1,1].
    let src = "## Main\nShow lstsq(3, 2, [1.0, 0.0, 1.0, 1.0, 1.0, 2.0], [1.0, 2.0, 3.0]).\n";
    assert_close(&run_seq(src), &[1.0, 1.0], 1e-9);
}

#[test]
fn lstsq_square_matches_solve() {
    // On a square system, least-squares is the exact solve. A=[[2,1],[1,3]], x=[1,1] → b=[3,4].
    let src = "## Main\nShow lstsq(2, 2, [2.0, 1.0, 1.0, 3.0], [3.0, 4.0]).\n";
    assert_close(&run_seq(src), &[1.0, 1.0], 1e-9);
}
