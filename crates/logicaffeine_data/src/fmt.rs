//! The ONE float-display path.
//!
//! Every engine that renders a LOGOS `Float` — the tree-walker, the bytecode
//! VM, the AOT-compiled binary, and the direct-WASM host — must agree on the
//! decimal string for the same `f64`, or the same program prints different
//! answers depending on how it was run. This module is the single authority.
//!
//! The contract of [`fmt_f64`]:
//!
//! - **Shortest round-trip**: the fewest decimal digits that parse back to the
//!   exact same bits (Rust's `{}` Display, Grisu/Dragon in std). A typed
//!   literal echoes as typed; `1.0 / 3.0` shows all 17 significant digits.
//! - **Never scientific notation**: `0.0000001` renders as `0.0000001`, not
//!   `1e-7` — and a nonzero value NEVER renders as `0`.
//! - **Integral floats stay bare**: `2.0` renders as `2`.
//! - **Bounded width**: the longest possible output (the smallest subnormal,
//!   `5e-324`) is under 340 bytes — the direct-WASM host's scratch size.

/// Renders an `f64` exactly as LOGOS displays it on every engine.
pub fn fmt_f64(f: f64) -> String {
    format!("{f}")
}

#[cfg(test)]
mod tests {
    use super::fmt_f64;

    #[test]
    fn shortest_roundtrip_division() {
        assert_eq!(fmt_f64(1.0 / 3.0), "0.3333333333333333");
    }

    #[test]
    fn tiny_nonzero_is_never_zero_and_never_scientific() {
        assert_eq!(fmt_f64(0.0000001), "0.0000001");
    }

    #[test]
    fn typed_pi_echoes() {
        assert_eq!(fmt_f64(3.141592653589793), "3.141592653589793");
    }

    #[test]
    fn float_artifact_is_visible() {
        assert_eq!(fmt_f64(0.1 + 0.2), "0.30000000000000004");
    }

    #[test]
    fn integral_floats_stay_bare() {
        assert_eq!(fmt_f64(2.0), "2");
        assert_eq!(fmt_f64(100.0), "100");
        assert_eq!(fmt_f64(-3.0), "-3");
    }

    #[test]
    fn short_floats_unchanged() {
        assert_eq!(fmt_f64(1.5), "1.5");
        assert_eq!(fmt_f64(0.5), "0.5");
        assert_eq!(fmt_f64(-1.5), "-1.5");
    }

    #[test]
    fn non_finite_values() {
        assert_eq!(fmt_f64(f64::NAN), "NaN");
        assert_eq!(fmt_f64(f64::INFINITY), "inf");
        assert_eq!(fmt_f64(f64::NEG_INFINITY), "-inf");
    }

    #[test]
    fn negative_zero_keeps_its_sign() {
        assert_eq!(fmt_f64(-0.0), "-0");
        assert_eq!(fmt_f64(0.0), "0");
    }

    #[test]
    fn huge_magnitudes_never_go_scientific() {
        let s = fmt_f64(1e300);
        assert!(!s.contains('e') && !s.contains('E'), "scientific leaked: {s}");
        assert_eq!(s.len(), 301);
        assert!(s.starts_with('1'));
        let s = fmt_f64(f64::MAX);
        assert!(!s.contains('e') && !s.contains('E'), "scientific leaked: {s}");
    }

    #[test]
    fn subnormals_never_go_scientific() {
        let s = fmt_f64(5e-324);
        assert!(!s.contains('e') && !s.contains('E'), "scientific leaked: {s}");
        assert!(s.starts_with("0.000"));
    }

    /// The direct-WASM host writes float display into a 340-byte scratch
    /// buffer (`vm/wasm/module.rs`); the widest possible outputs must fit.
    #[test]
    fn worst_case_width_fits_wasm_scratch() {
        for f in [5e-324, -5e-324, f64::MAX, f64::MIN, 1e300, -1e300] {
            let s = fmt_f64(f);
            assert!(s.len() <= 340, "{} bytes for {f:e}", s.len());
        }
    }

    /// Shortest round-trip means parsing the output recovers the exact bits.
    #[test]
    fn output_roundtrips_bit_exactly() {
        for f in [
            1.0 / 3.0,
            0.1 + 0.2,
            3.141592653589793,
            0.0000001,
            -0.0,
            1e300,
            5e-324,
            f64::MAX,
            f64::MIN_POSITIVE,
            123456789.123456789,
        ] {
            let parsed: f64 = fmt_f64(f).parse().unwrap();
            assert_eq!(parsed.to_bits(), f.to_bits(), "round-trip drift for {f:e}");
        }
    }
}
