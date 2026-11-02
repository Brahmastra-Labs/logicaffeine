//! LOGOS-LAPACK matrix logarithm — `logm` (inverse of `expm`). Inverse scaling-and-squaring: take
//! matrix square roots until the argument is near `I`, sum the series `log(I+X)=X−X²/2+X³/3−…`, then
//! scale by `2ᵏ`. Pure native LOGOS — no scalar log, just `sqrtm` + matrix multiplies. Locked by the
//! round-trip `logm(expm(A)) == A` and a nilpotent closed form, via a tolerance harness.

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
    format!(
        "[{}]",
        xs.iter()
            .map(|x| float_lit(*x))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn parse_seq_floats(out: &str) -> Vec<f64> {
    let t = out.trim();
    let inner = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .expect("bracketed Seq");
    inner
        .split(',')
        .map(|s| s.trim().parse::<f64>().expect("float"))
        .collect()
}
fn assert_close(got: &[f64], want: &[f64], tol: f64) {
    assert_eq!(got.len(), want.len(), "len: {got:?} vs {want:?}");
    for (i, (g, e)) in got.iter().zip(want.iter()).enumerate() {
        assert!(
            (g - e).abs() <= tol * (1.0 + e.abs()),
            "elem {i}: {g} vs {e}"
        );
    }
}
fn run_seq(src: &str) -> Vec<f64> {
    let out = run_all_tiers(src);
    assert!(out.error.is_none(), "errored: {:?}\n{src}", out.error);
    parse_seq_floats(&out.output)
}

#[test]
fn logm_of_unit_upper_is_nilpotent() {
    // [[1,1],[0,1]] = expm([[0,1],[0,0]]) → logm = [[0,1],[0,0]].
    let out = run_seq("## Main\nShow logm(2, [1.0, 1.0, 0.0, 1.0]).\n");
    assert_close(&out, &[0.0, 1.0, 0.0, 0.0], 1e-7);
}

#[test]
fn logm_inverts_expm() {
    // logm(expm(A)) ≈ A for a small well-conditioned A.
    let a = [0.4, 0.3, 0.1, 0.2];
    let src = format!(
        "## Main\nLet ea be expm(2, {}).\nShow logm(2, ea).\n",
        seq_literal(&a)
    );
    assert_close(&run_seq(&src), &a, 1e-6);
}

#[test]
fn logm_inverts_expm_3x3() {
    let a = [0.2, 0.1, 0.0, 0.0, 0.3, 0.1, 0.1, 0.0, 0.25];
    let src = format!(
        "## Main\nLet ea be expm(3, {}).\nShow logm(3, ea).\n",
        seq_literal(&a)
    );
    assert_close(&run_seq(&src), &a, 1e-6);
}
