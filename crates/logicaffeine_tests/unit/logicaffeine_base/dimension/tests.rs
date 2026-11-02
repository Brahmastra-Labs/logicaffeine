use super::*;

#[test]
fn dimension_by_name_resolves_the_named_dimensions() {
    assert_eq!(Dimension::by_name("Length"), Some(Dimension::length()));
    assert_eq!(Dimension::by_name("length"), Some(Dimension::length())); // case-insensitive
    assert_eq!(Dimension::by_name("Mass"), Some(Dimension::mass()));
    assert_eq!(Dimension::by_name("Area"), Some(Dimension::area()));
    assert_eq!(Dimension::by_name("Volume"), Some(Dimension::volume()));
    assert_eq!(Dimension::by_name("Speed"), Some(Dimension::speed()));
    assert_eq!(Dimension::by_name("Velocity"), Some(Dimension::speed())); // alias
    assert_eq!(Dimension::by_name("Force"), Some(Dimension::force()));
    assert_eq!(Dimension::by_name("Energy"), Some(Dimension::energy()));
    // Area and Volume are distinct (L² vs L³) — the whole point.
    assert_ne!(Dimension::by_name("Area"), Dimension::by_name("Volume"));
    assert_ne!(Dimension::by_name("Length"), Dimension::by_name("Mass"));
    assert_eq!(Dimension::by_name("Frobnicate"), None);
}

/// A tiny deterministic RNG (SplitMix64) — reproducible fuzz with no dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// A random dimension with small integer exponents in [−3, 3] on each axis.
    fn dim(&mut self) -> Dimension {
        let mut e = [Exp::ZERO; BaseDim::COUNT];
        for slot in e.iter_mut() {
            *slot = Exp::int((self.next() % 7) as i32 - 3);
        }
        Dimension { e }
    }
}

// ----------------------------------------------------------------
// Exp — the rational exponent
// ----------------------------------------------------------------

#[test]
fn exp_reduces_and_normalizes_sign() {
    assert_eq!(Exp::new(2, 4), Exp::new(1, 2));
    assert_eq!(Exp::new(-2, 4), Exp::new(1, -2)); // sign moves onto the numerator
    assert_eq!(Exp::new(0, 5), Exp::ZERO);
    assert_eq!(Exp::new(6, 3), Exp::int(2));
    assert!(!Exp::new(1, 2).denominator().is_negative());
}

#[test]
fn exp_arithmetic_is_exact_rational() {
    assert_eq!(Exp::new(1, 2).add(Exp::new(1, 2)), Exp::ONE); // ½ + ½ = 1
    assert_eq!(
        Exp::new(1, 3).add(Exp::new(1, 3)).add(Exp::new(1, 3)),
        Exp::ONE
    );
    assert_eq!(Exp::int(2).sub(Exp::new(1, 2)), Exp::new(3, 2));
    assert_eq!(Exp::new(1, 2).scale(4), Exp::int(2));
    assert_eq!(Exp::int(2).div_int(2), Exp::ONE); // √ of L² exponent
    assert_eq!(Exp::ONE.div_int(2), Exp::new(1, 2)); // √ of L exponent = L^½
    assert_eq!(Exp::new(2, 3).neg(), Exp::new(-2, 3));
}

#[test]
fn exp_displays_as_int_or_fraction() {
    assert_eq!(Exp::int(2).to_string(), "2");
    assert_eq!(Exp::int(-3).to_string(), "-3");
    assert_eq!(Exp::new(1, 2).to_string(), "1/2");
    assert_eq!(Exp::new(-1, 2).to_string(), "-1/2");
}

// ----------------------------------------------------------------
// Dimension — algebra
// ----------------------------------------------------------------

#[test]
fn base_dimensions_are_distinct() {
    let bases: Vec<Dimension> = BaseDim::ALL.iter().map(|&d| Dimension::base(d)).collect();
    for (i, a) in bases.iter().enumerate() {
        for (j, b) in bases.iter().enumerate() {
            assert_eq!(a == b, i == j, "base {i} vs {j}");
        }
        assert!(!a.is_dimensionless());
    }
    assert!(Dimension::DIMENSIONLESS.is_dimensionless());
}

#[test]
fn multiplication_adds_exponents() {
    // Length × Length = Length² = Area.
    assert_eq!(
        Dimension::length().mul(Dimension::length()),
        Dimension::area()
    );
    assert_eq!(Dimension::length().powi(3), Dimension::volume());
    // Area × Length = Volume.
    assert_eq!(
        Dimension::area().mul(Dimension::length()),
        Dimension::volume()
    );
}

#[test]
fn division_subtracts_exponents() {
    // Area ÷ Length = Length; Length ÷ Length = dimensionless.
    assert_eq!(
        Dimension::area().div(Dimension::length()),
        Dimension::length()
    );
    assert!(Dimension::length()
        .div(Dimension::length())
        .is_dimensionless());
    // Speed = Length / Time.
    assert_eq!(
        Dimension::length().div(Dimension::time()),
        Dimension::speed()
    );
}

#[test]
fn reciprocal_is_the_group_inverse() {
    // Frequency = 1 / Time.
    assert_eq!(Dimension::time().recip(), Dimension::frequency());
    // x · x⁻¹ = 1 for every base dimension.
    for d in BaseDim::ALL {
        let x = Dimension::base(d);
        assert!(x.mul(x.recip()).is_dimensionless());
    }
    // double reciprocal is the identity.
    assert_eq!(Dimension::force().recip().recip(), Dimension::force());
}

#[test]
fn powers_and_roots_are_inverse() {
    assert!(Dimension::length().powi(0).is_dimensionless());
    assert_eq!(Dimension::length().powi(-1), Dimension::length().recip());
    // √(Area) = Length; the cube root of Volume = Length.
    assert_eq!(Dimension::area().nth_root(2), Dimension::length());
    assert_eq!(Dimension::volume().nth_root(3), Dimension::length());
    // A fractional dimension survives: √(Frequency) = T^(−1/2) (noise-density shape).
    let root_hz = Dimension::frequency().nth_root(2);
    assert_eq!(root_hz.exponent(BaseDim::Time), Exp::new(-1, 2));
    assert_eq!(root_hz.powi(2), Dimension::frequency());
}

#[test]
fn derived_dimensions_compose_correctly() {
    // Force = M·L·T⁻²  (mass × acceleration).
    let force = Dimension::force();
    assert_eq!(force.exponent(BaseDim::Mass), Exp::int(1));
    assert_eq!(force.exponent(BaseDim::Length), Exp::int(1));
    assert_eq!(force.exponent(BaseDim::Time), Exp::int(-2));
    // Energy = M·L²·T⁻² = Force · Length.
    let energy = Dimension::energy();
    assert_eq!(energy, Dimension::force().mul(Dimension::length()));
    assert_eq!(energy.exponent(BaseDim::Length), Exp::int(2));
    assert_eq!(energy.exponent(BaseDim::Time), Exp::int(-2));
    // Power = Energy / Time = M·L²·T⁻³.
    assert_eq!(Dimension::power().exponent(BaseDim::Time), Exp::int(-3));
    // Pressure = Force / Area = M·L⁻¹·T⁻².
    let p = Dimension::pressure();
    assert_eq!(p.exponent(BaseDim::Length), Exp::int(-1));
    assert_eq!(p.exponent(BaseDim::Mass), Exp::int(1));
    // Charge = I·T; Voltage = Power/Current; Resistance = Voltage/Current.
    assert_eq!(
        Dimension::charge(),
        Dimension::current().mul(Dimension::time())
    );
    assert_eq!(
        Dimension::resistance(),
        Dimension::voltage().div(Dimension::current())
    );
    // Density = M·L⁻³.
    assert_eq!(Dimension::density().exponent(BaseDim::Length), Exp::int(-3));
}

#[test]
fn all_extended_derived_dimensions_have_their_canonical_signatures() {
    use BaseDim::*;
    // A derived dimension's exponent vector must match its physical definition exactly. Each
    // entry: (dimension, [(axis, exponent), …]); every unlisted axis must be zero.
    let cases: &[(Dimension, &[(BaseDim, Exp)])] = &[
        // Radiation & catalysis.
        (
            Dimension::absorbed_dose(),
            &[(Length, Exp::int(2)), (Time, Exp::int(-2))],
        ), // Gy = J/kg = L²·T⁻²
        (Dimension::radioactivity(), &[(Time, Exp::int(-1))]), // Bq = T⁻¹
        (
            Dimension::exposure(),
            &[
                (Current, Exp::int(1)),
                (Time, Exp::int(1)),
                (Mass, Exp::int(-1)),
            ],
        ), // C/kg
        (
            Dimension::catalytic_activity(),
            &[(Amount, Exp::int(1)), (Time, Exp::int(-1))],
        ), // kat = mol/s
        // Photometry.
        (
            Dimension::luminous_flux(),
            &[(Luminous, Exp::int(1)), (SolidAngle, Exp::int(1))],
        ), // lm = cd·sr
        (
            Dimension::illuminance(),
            &[
                (Luminous, Exp::int(1)),
                (SolidAngle, Exp::int(1)),
                (Length, Exp::int(-2)),
            ],
        ), // lx
        (
            Dimension::luminance(),
            &[(Luminous, Exp::int(1)), (Length, Exp::int(-2))],
        ), // nit = cd/m²
        // Rates, flow, concentration, viscosity, fuel economy.
        (
            Dimension::data_rate(),
            &[(Information, Exp::int(1)), (Time, Exp::int(-1))],
        ), // bit/s
        (
            Dimension::volumetric_flow(),
            &[(Length, Exp::int(3)), (Time, Exp::int(-1))],
        ), // m³/s
        (
            Dimension::molar_concentration(),
            &[(Amount, Exp::int(1)), (Length, Exp::int(-3))],
        ), // mol/m³
        (
            Dimension::molality(),
            &[(Amount, Exp::int(1)), (Mass, Exp::int(-1))],
        ), // mol/kg
        (
            Dimension::dynamic_viscosity(),
            &[
                (Mass, Exp::int(1)),
                (Length, Exp::int(-1)),
                (Time, Exp::int(-1)),
            ],
        ), // Pa·s
        (
            Dimension::kinematic_viscosity(),
            &[(Length, Exp::int(2)), (Time, Exp::int(-1))],
        ), // m²/s
        (Dimension::fuel_economy(), &[(Length, Exp::int(-2))]), // m/m³ = L⁻²
    ];
    for (dim, expected) in cases {
        for axis in BaseDim::ALL {
            let want = expected
                .iter()
                .find(|(a, _)| *a == axis)
                .map(|(_, e)| *e)
                .unwrap_or(Exp::int(0));
            assert_eq!(dim.exponent(axis), want, "{} exponent on {:?}", dim, axis);
        }
    }
    // Cross-checks against the composition algebra: each derived dim equals the operation that
    // defines it, so the named helpers can never drift from their formulas.
    assert_eq!(
        Dimension::absorbed_dose(),
        Dimension::energy().div(Dimension::mass())
    );
    assert_eq!(Dimension::radioactivity(), Dimension::frequency()); // Bq is dimensionally a frequency
    assert_eq!(
        Dimension::data_rate(),
        Dimension::information().div(Dimension::time())
    );
    assert_eq!(
        Dimension::volumetric_flow(),
        Dimension::volume().div(Dimension::time())
    );
    assert_eq!(
        Dimension::molar_concentration(),
        Dimension::amount().div(Dimension::volume())
    );
    assert_eq!(
        Dimension::dynamic_viscosity(),
        Dimension::pressure().mul(Dimension::time())
    );
    assert_eq!(
        Dimension::kinematic_viscosity(),
        Dimension::area().div(Dimension::time())
    );
    assert_eq!(
        Dimension::fuel_economy(),
        Dimension::length().div(Dimension::volume())
    );
    assert_eq!(
        Dimension::illuminance(),
        Dimension::luminous_flux().div(Dimension::area())
    );
    // Kinematic viscosity = dynamic viscosity / density (the physical relation μ/ρ = ν).
    assert_eq!(
        Dimension::kinematic_viscosity(),
        Dimension::dynamic_viscosity().div(Dimension::density())
    );
    // Electromagnetism: each derived EM dimension equals its defining SI relation.
    assert_eq!(Dimension::conductance(), Dimension::resistance().recip()); // S = Ω⁻¹
    assert!(Dimension::conductance()
        .mul(Dimension::resistance())
        .is_dimensionless()); // S·Ω = 1
    assert_eq!(
        Dimension::capacitance(),
        Dimension::charge().div(Dimension::voltage())
    ); // F = C/V
    assert_eq!(
        Dimension::magnetic_flux(),
        Dimension::voltage().mul(Dimension::time())
    ); // Wb = V·s
    assert_eq!(
        Dimension::inductance(),
        Dimension::magnetic_flux().div(Dimension::current())
    ); // H = Wb/A
    assert_eq!(
        Dimension::magnetic_flux_density(),
        Dimension::magnetic_flux().div(Dimension::area())
    ); // T = Wb/m²
    assert_eq!(
        Dimension::surface_tension(),
        Dimension::force().div(Dimension::length())
    ); // N/m
       // Energy can also be reached as charge × voltage (Q·V = J) — a cross-path consistency check.
    assert_eq!(
        Dimension::energy(),
        Dimension::charge().mul(Dimension::voltage())
    );
}

#[test]
fn display_renders_the_signature() {
    assert_eq!(Dimension::DIMENSIONLESS.to_string(), "1");
    assert_eq!(Dimension::length().to_string(), "L");
    assert_eq!(Dimension::area().to_string(), "L^2");
    assert_eq!(Dimension::frequency().to_string(), "T^-1");
    assert_eq!(Dimension::speed().to_string(), "L·T^-1");
    // Force in canonical (L, M, T, …) order: L·M·T^-2.
    assert_eq!(Dimension::force().to_string(), "L·M·T^-2");
}

#[test]
fn dimension_is_copy_and_hashes_by_value() {
    use std::collections::HashSet;
    let a = Dimension::force();
    let b = a; // Copy — `a` is still usable below.
    assert_eq!(a, b);
    let mut set = HashSet::new();
    set.insert(Dimension::force());
    assert!(set.contains(&Dimension::mass().mul(Dimension::acceleration())));
    assert!(!set.contains(&Dimension::energy()));
}

#[test]
fn dimensions_form_an_abelian_group_under_fuzz() {
    // Property fuzz: random exponent vectors satisfy the abelian-group axioms exactly, and
    // power/root/distributivity laws hold.
    let mut r = Rng(0x_D1E5_10_4ABE_1234);
    let id = Dimension::DIMENSIONLESS;
    for _ in 0..4000 {
        let (a, b, c) = (r.dim(), r.dim(), r.dim());
        // Commutativity + associativity of ×.
        assert_eq!(a.mul(b), b.mul(a), "× commutes");
        assert_eq!(a.mul(b).mul(c), a.mul(b.mul(c)), "× associates");
        // Identity + inverse.
        assert_eq!(a.mul(id), a, "1 is the identity");
        assert!(a.mul(a.recip()).is_dimensionless(), "x · x⁻¹ = 1");
        // Division agrees with multiply-by-inverse.
        assert_eq!(a.div(b), a.mul(b.recip()), "a / b = a · b⁻¹");
        assert!(a.div(a).is_dimensionless(), "a / a = 1");
        // Powers: aᵏ · aᵐ = aᵏ⁺ᵐ; (a·b)ᵏ = aᵏ · bᵏ; (aᵏ) root k = a.
        assert_eq!(a.powi(2).mul(a.powi(3)), a.powi(5), "exponent law");
        assert_eq!(
            a.mul(b).powi(2),
            a.powi(2).mul(b.powi(2)),
            "power distributes over ×"
        );
        assert_eq!(a.powi(2).nth_root(2), a, "(a²) √2 = a");
        assert_eq!(a.powi(0), id, "a⁰ = 1");
        // recip is an involution.
        assert_eq!(a.recip().recip(), a);
    }
}
