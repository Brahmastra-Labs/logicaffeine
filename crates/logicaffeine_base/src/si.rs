//! SI-dimensioned f64 newtypes — the characterized-descriptor surface types.
//!
//! Each type NAMES its unit and KNOWS its physical dimension, checked against the exact
//! [`Dimension`] algebra (rational exponents included — `VoltsPerRootHz` is `V·Hz^(−1/2)`).
//! The payload stays in the NAMED unit — `Nanoseconds(8.0)` is 8 ns, no hidden conversion —
//! so flipping a bare `f64` field to its newtype never moves a pinned value, only the type.
//! Payloads are plain `f64` because the certified engines stay outward-rounded f64; exact
//! `Rational` arithmetic lives at unit-conversion boundaries (`quantity.rs`).

use crate::dimension::Dimension;

macro_rules! si_newtype {
    ($(#[$doc:meta])* $name:ident, $dim:expr) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
        pub struct $name(pub f64);

        impl $name {
            /// The exact physical dimension this type carries.
            pub fn dimension() -> Dimension {
                $dim
            }
        }
    };
}

si_newtype!(
    /// Volts — electric potential.
    Volts,
    Dimension::voltage()
);
si_newtype!(
    /// Seconds — time.
    Seconds,
    Dimension::time()
);
si_newtype!(
    /// Nanoseconds — time, payload in ns (the unit rides the NAME, not a hidden scale).
    Nanoseconds,
    Dimension::time()
);
si_newtype!(
    /// Hertz — frequency (s⁻¹).
    Hertz,
    Dimension::frequency()
);
si_newtype!(
    /// Siemens — conductance (Ω⁻¹), the transconductor's gm unit.
    Siemens,
    Dimension::conductance()
);
si_newtype!(
    /// Kelvin — thermodynamic temperature.
    Kelvin,
    Dimension::temperature()
);
si_newtype!(
    /// Watts — power, the thermal solver's injection unit.
    Watts,
    Dimension::power()
);
si_newtype!(
    /// Per-kelvin — temperature coefficients (tempco), K⁻¹.
    PerKelvin,
    Dimension::temperature().recip()
);
si_newtype!(
    /// Volts per root hertz — noise spectral density, the rational-exponent payoff:
    /// `V·Hz^(−1/2)`.
    VoltsPerRootHz,
    Dimension::voltage().div(Dimension::frequency().nth_root(2))
);
