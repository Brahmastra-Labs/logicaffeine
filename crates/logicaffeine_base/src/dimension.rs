//! Physical dimensions as an abelian group of rational exponent vectors.
//!
//! A *dimension* is what a quantity measures — length, mass, time, or a product/quotient of
//! them (area = L², speed = L·T⁻¹, force = M·L·T⁻²). It is NOT a unit: metres and feet share
//! the dimension Length. We model a dimension as a vector of rational exponents over the base
//! dimensions, so dimensional algebra is exact vector arithmetic:
//!
//! - `×` adds exponent vectors (`Length × Length = Length²`),
//! - `÷` subtracts them (`Area ÷ Length = Length`),
//! - integer powers scale them, and the `n`th root divides them (so `√(Length²) = Length`).
//!
//! Exponents are *rational* (not just integer) because real derived quantities need fractional
//! powers — noise density is `V·Hz^(−1/2)`. The group's identity is the dimensionless vector;
//! every dimension has an inverse (`recip`). `Dimension` is `Copy + Eq + Hash`, so it is a cheap
//! catalog key, rides inside the compiler's type lattice, and tags a runtime quantity.

use std::fmt;

/// The base dimensions: the SI seven, plus the extensions Logos tracks end-to-end. Plane angle
/// and solid angle are SI-dimensionless but tracked here to catch radian/steradian mix-ups;
/// information (the bit) is tracked so data sizes are first-class. "Dimensionless" is the
/// all-zero vector, not an axis.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(usize)]
pub enum BaseDim {
    Length = 0,
    Mass,
    Time,
    Current,
    Temperature,
    Amount,
    Luminous,
    Angle,
    SolidAngle,
    Information,
}

impl BaseDim {
    /// The number of base axes (the length of a [`Dimension`]'s exponent vector).
    pub const COUNT: usize = 10;

    /// Every base dimension, in canonical order (the order exponents are stored and displayed).
    pub const ALL: [BaseDim; BaseDim::COUNT] = [
        BaseDim::Length,
        BaseDim::Mass,
        BaseDim::Time,
        BaseDim::Current,
        BaseDim::Temperature,
        BaseDim::Amount,
        BaseDim::Luminous,
        BaseDim::Angle,
        BaseDim::SolidAngle,
        BaseDim::Information,
    ];

    /// This axis's index into the exponent vector.
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The conventional one-symbol tag used when displaying a dimension signature.
    pub const fn symbol(self) -> &'static str {
        match self {
            BaseDim::Length => "L",
            BaseDim::Mass => "M",
            BaseDim::Time => "T",
            BaseDim::Current => "I",
            BaseDim::Temperature => "Θ",
            BaseDim::Amount => "N",
            BaseDim::Luminous => "J",
            BaseDim::Angle => "rad",
            BaseDim::SolidAngle => "sr",
            BaseDim::Information => "bit",
        }
    }
}

/// `gcd(|a|, |b|)` by the Euclidean algorithm (`gcd(a, 0) == |a|`).
const fn igcd(mut a: i32, mut b: i32) -> i32 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// A signed rational exponent in lowest terms with a strictly positive denominator — so equal
/// exponents share one representation (`Eq`/`Hash` are structural). Admits fractional powers
/// (`L^(1/2)`) while the common integer case is just `den == 1`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Exp {
    num: i32,
    den: i32,
}

impl Exp {
    /// The zero exponent (an axis that does not appear).
    pub const ZERO: Exp = Exp { num: 0, den: 1 };
    /// The unit exponent.
    pub const ONE: Exp = Exp { num: 1, den: 1 };

    /// Reduce `num/den` to lowest terms with a positive denominator. Panics on a zero denominator.
    pub fn new(num: i32, den: i32) -> Exp {
        assert!(den != 0, "exponent denominator must be nonzero");
        let (mut num, mut den) = (num, den);
        if den < 0 {
            num = -num;
            den = -den;
        }
        if num == 0 {
            return Exp::ZERO;
        }
        let g = igcd(num, den);
        Exp { num: num / g, den: den / g }
    }

    /// The whole exponent `n` (`n/1`).
    #[inline]
    pub const fn int(n: i32) -> Exp {
        Exp { num: n, den: 1 }
    }

    pub const fn numerator(self) -> i32 {
        self.num
    }

    pub const fn denominator(self) -> i32 {
        self.den
    }

    #[inline]
    pub const fn is_zero(self) -> bool {
        self.num == 0
    }

    /// `self + other` (exact rational addition).
    pub fn add(self, other: Exp) -> Exp {
        Exp::new(self.num * other.den + other.num * self.den, self.den * other.den)
    }

    /// `self − other`.
    pub fn sub(self, other: Exp) -> Exp {
        Exp::new(self.num * other.den - other.num * self.den, self.den * other.den)
    }

    /// `−self`.
    pub fn neg(self) -> Exp {
        Exp { num: -self.num, den: self.den }
    }

    /// `self · k` — scaling by an integer (raising a dimension to the `k`th power).
    pub fn scale(self, k: i32) -> Exp {
        Exp::new(self.num * k, self.den)
    }

    /// `self / k` — dividing the exponent by an integer (taking the `k`th root). Panics for `k == 0`.
    pub fn div_int(self, k: i32) -> Exp {
        Exp::new(self.num, self.den * k)
    }
}

impl fmt::Display for Exp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// A physical dimension: a vector of rational exponents over [`BaseDim`]. The group operation is
/// `mul` (axiswise exponent addition); the identity is [`Dimension::DIMENSIONLESS`]; the inverse
/// is [`Dimension::recip`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Dimension {
    e: [Exp; BaseDim::COUNT],
}

impl Dimension {
    /// The identity dimension — the all-zero exponent vector (a pure number, e.g. a count or ratio).
    pub const DIMENSIONLESS: Dimension = Dimension { e: [Exp::ZERO; BaseDim::COUNT] };

    /// The dimension of a single base axis raised to the first power (e.g. `base(Length)` = L).
    pub fn base(d: BaseDim) -> Dimension {
        let mut e = [Exp::ZERO; BaseDim::COUNT];
        e[d.index()] = Exp::ONE;
        Dimension { e }
    }

    /// Build a dimension directly from its exponent vector (in [`BaseDim::ALL`] order) — the inverse
    /// of reading [`exponent`](Self::exponent) for each axis. Used to reconstruct a dimension from
    /// the wire, where the exponents travel as their `(numerator, denominator)` pairs.
    pub fn from_exps(e: [Exp; BaseDim::COUNT]) -> Dimension {
        Dimension { e }
    }

    /// The exponent on a given base axis.
    #[inline]
    pub fn exponent(self, d: BaseDim) -> Exp {
        self.e[d.index()]
    }

    /// True for the dimensionless identity (every exponent zero).
    pub fn is_dimensionless(self) -> bool {
        self.e.iter().all(|x| x.is_zero())
    }

    /// `self × other` — axiswise exponent ADDITION (`Length × Length = Length²`).
    pub fn mul(self, other: Dimension) -> Dimension {
        let mut e = self.e;
        for (i, slot) in e.iter_mut().enumerate() {
            *slot = slot.add(other.e[i]);
        }
        Dimension { e }
    }

    /// `self ÷ other` — axiswise exponent SUBTRACTION (`Area ÷ Length = Length`).
    pub fn div(self, other: Dimension) -> Dimension {
        let mut e = self.e;
        for (i, slot) in e.iter_mut().enumerate() {
            *slot = slot.sub(other.e[i]);
        }
        Dimension { e }
    }

    /// `1 / self` — the multiplicative inverse (negate every exponent). `Frequency = recip(Time)`.
    pub fn recip(self) -> Dimension {
        let mut e = self.e;
        for slot in e.iter_mut() {
            *slot = slot.neg();
        }
        Dimension { e }
    }

    /// `self^k` — the `k`th power (scale every exponent). `powi(0)` is dimensionless.
    pub fn powi(self, k: i32) -> Dimension {
        let mut e = self.e;
        for slot in e.iter_mut() {
            *slot = slot.scale(k);
        }
        Dimension { e }
    }

    /// The `k`th root — divide every exponent by `k` (`nth_root(Area, 2) = Length`). Panics for `k == 0`.
    pub fn nth_root(self, k: i32) -> Dimension {
        let mut e = self.e;
        for slot in e.iter_mut() {
            *slot = slot.div_int(k);
        }
        Dimension { e }
    }

    // ---- The SI base dimensions, by name ----
    pub fn length() -> Dimension { Dimension::base(BaseDim::Length) }
    pub fn mass() -> Dimension { Dimension::base(BaseDim::Mass) }
    pub fn time() -> Dimension { Dimension::base(BaseDim::Time) }
    pub fn current() -> Dimension { Dimension::base(BaseDim::Current) }
    pub fn temperature() -> Dimension { Dimension::base(BaseDim::Temperature) }
    pub fn amount() -> Dimension { Dimension::base(BaseDim::Amount) }
    pub fn luminous() -> Dimension { Dimension::base(BaseDim::Luminous) }
    pub fn angle() -> Dimension { Dimension::base(BaseDim::Angle) }
    pub fn solid_angle() -> Dimension { Dimension::base(BaseDim::SolidAngle) }
    pub fn information() -> Dimension { Dimension::base(BaseDim::Information) }

    // ---- Common derived dimensions, composed from the base ones ----
    pub fn area() -> Dimension { Self::length().powi(2) }
    pub fn volume() -> Dimension { Self::length().powi(3) }
    pub fn speed() -> Dimension { Self::length().div(Self::time()) }
    pub fn acceleration() -> Dimension { Self::speed().div(Self::time()) }
    pub fn frequency() -> Dimension { Self::time().recip() }
    pub fn force() -> Dimension { Self::mass().mul(Self::acceleration()) }
    pub fn energy() -> Dimension { Self::force().mul(Self::length()) }
    pub fn power() -> Dimension { Self::energy().div(Self::time()) }
    pub fn pressure() -> Dimension { Self::force().div(Self::area()) }
    pub fn charge() -> Dimension { Self::current().mul(Self::time()) }
    pub fn voltage() -> Dimension { Self::power().div(Self::current()) }
    pub fn resistance() -> Dimension { Self::voltage().div(Self::current()) }
    pub fn conductance() -> Dimension { Self::resistance().recip() } // siemens: A/V = Ω⁻¹
    pub fn capacitance() -> Dimension { Self::charge().div(Self::voltage()) } // farad: C/V
    pub fn magnetic_flux() -> Dimension { Self::voltage().mul(Self::time()) } // weber: V·s
    pub fn inductance() -> Dimension { Self::magnetic_flux().div(Self::current()) } // henry: Wb/A
    pub fn magnetic_flux_density() -> Dimension { Self::magnetic_flux().div(Self::area()) } // tesla: Wb/m²
    pub fn surface_tension() -> Dimension { Self::force().div(Self::length()) } // N/m = M·T⁻²
    pub fn density() -> Dimension { Self::mass().div(Self::volume()) }
    // Radiation.
    pub fn absorbed_dose() -> Dimension { Self::energy().div(Self::mass()) } // gray, sievert: J/kg = L²·T⁻²
    pub fn radioactivity() -> Dimension { Self::frequency() } // becquerel: decays per second = T⁻¹
    pub fn exposure() -> Dimension { Self::charge().div(Self::mass()) } // roentgen: C/kg
    pub fn catalytic_activity() -> Dimension { Self::amount().div(Self::time()) } // katal: mol/s
    // Photometry.
    pub fn luminous_flux() -> Dimension { Self::luminous().mul(Self::solid_angle()) } // lumen: cd·sr
    pub fn illuminance() -> Dimension { Self::luminous_flux().div(Self::area()) } // lux: lm/m²
    pub fn luminance() -> Dimension { Self::luminous().div(Self::area()) } // nit: cd/m²
    // Rate / flow / concentration / viscosity / efficiency.
    pub fn data_rate() -> Dimension { Self::information().div(Self::time()) } // bit/s
    pub fn volumetric_flow() -> Dimension { Self::volume().div(Self::time()) } // m³/s
    pub fn molar_concentration() -> Dimension { Self::amount().div(Self::volume()) } // mol/m³
    pub fn molality() -> Dimension { Self::amount().div(Self::mass()) } // mol/kg
    pub fn dynamic_viscosity() -> Dimension { Self::pressure().mul(Self::time()) } // Pa·s = M·L⁻¹·T⁻¹
    pub fn kinematic_viscosity() -> Dimension { Self::area().div(Self::time()) } // m²/s = L²·T⁻¹
    pub fn fuel_economy() -> Dimension { Self::length().div(Self::volume()) } // distance/volume (mpg) = L⁻²

    /// Resolve a **named dimension** (case-insensitive) to its exponent vector — the lookup behind
    /// the `Quantity of <Dimension>` type annotation (`Quantity of Length`, `Quantity of Area`). The
    /// names are the human dimension words, not units; `None` for an unknown name.
    pub fn by_name(name: &str) -> Option<Dimension> {
        let n = name.trim().to_ascii_lowercase();
        Some(match n.as_str() {
            "dimensionless" | "scalar" | "number" => Dimension::DIMENSIONLESS,
            // SI base dimensions.
            "length" | "distance" => Self::length(),
            "mass" => Self::mass(),
            "time" | "duration" => Self::time(),
            "current" | "electriccurrent" | "electric current" => Self::current(),
            "temperature" => Self::temperature(),
            "amount" | "amountofsubstance" => Self::amount(),
            "luminous" | "luminousintensity" => Self::luminous(),
            "angle" | "planeangle" => Self::angle(),
            "solidangle" | "solid angle" => Self::solid_angle(),
            "information" | "data" => Self::information(),
            // Common derived dimensions.
            "area" => Self::area(),
            "volume" => Self::volume(),
            "speed" | "velocity" => Self::speed(),
            "acceleration" => Self::acceleration(),
            "frequency" => Self::frequency(),
            "force" | "weight" => Self::force(),
            "energy" | "work" | "heat" => Self::energy(),
            "power" => Self::power(),
            "pressure" | "stress" => Self::pressure(),
            "charge" | "electriccharge" => Self::charge(),
            "voltage" | "potential" | "emf" => Self::voltage(),
            "resistance" => Self::resistance(),
            "conductance" => Self::conductance(),
            "capacitance" => Self::capacitance(),
            "inductance" => Self::inductance(),
            "magneticflux" | "magnetic flux" => Self::magnetic_flux(),
            "magneticfluxdensity" | "fluxdensity" => Self::magnetic_flux_density(),
            "density" => Self::density(),
            "surfacetension" | "surface tension" => Self::surface_tension(),
            "datarate" | "data rate" | "bandwidth" => Self::data_rate(),
            "flow" | "volumetricflow" | "volumetric flow" => Self::volumetric_flow(),
            "concentration" | "molarconcentration" | "molarity" => Self::molar_concentration(),
            "molality" => Self::molality(),
            "absorbeddose" | "dose" => Self::absorbed_dose(),
            "radioactivity" | "activity" => Self::radioactivity(),
            "exposure" => Self::exposure(),
            "catalyticactivity" | "catalysis" => Self::catalytic_activity(),
            "luminousflux" => Self::luminous_flux(),
            "illuminance" => Self::illuminance(),
            "luminance" => Self::luminance(),
            "fueleconomy" | "fuel economy" => Self::fuel_economy(),
            _ => return None,
        })
    }
}

impl fmt::Display for Dimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_dimensionless() {
            return write!(f, "1");
        }
        let mut first = true;
        for d in BaseDim::ALL {
            let x = self.exponent(d);
            if x.is_zero() {
                continue;
            }
            if !first {
                write!(f, "·")?;
            }
            first = false;
            if x == Exp::ONE {
                write!(f, "{}", d.symbol())?;
            } else {
                write!(f, "{}^{}", d.symbol(), x)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(Exp::new(1, 3).add(Exp::new(1, 3)).add(Exp::new(1, 3)), Exp::ONE);
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
        assert_eq!(Dimension::length().mul(Dimension::length()), Dimension::area());
        assert_eq!(Dimension::length().powi(3), Dimension::volume());
        // Area × Length = Volume.
        assert_eq!(Dimension::area().mul(Dimension::length()), Dimension::volume());
    }

    #[test]
    fn division_subtracts_exponents() {
        // Area ÷ Length = Length; Length ÷ Length = dimensionless.
        assert_eq!(Dimension::area().div(Dimension::length()), Dimension::length());
        assert!(Dimension::length().div(Dimension::length()).is_dimensionless());
        // Speed = Length / Time.
        assert_eq!(Dimension::length().div(Dimension::time()), Dimension::speed());
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
        assert_eq!(Dimension::charge(), Dimension::current().mul(Dimension::time()));
        assert_eq!(Dimension::resistance(), Dimension::voltage().div(Dimension::current()));
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
            (Dimension::absorbed_dose(), &[(Length, Exp::int(2)), (Time, Exp::int(-2))]), // Gy = J/kg = L²·T⁻²
            (Dimension::radioactivity(), &[(Time, Exp::int(-1))]),                        // Bq = T⁻¹
            (Dimension::exposure(), &[(Current, Exp::int(1)), (Time, Exp::int(1)), (Mass, Exp::int(-1))]), // C/kg
            (Dimension::catalytic_activity(), &[(Amount, Exp::int(1)), (Time, Exp::int(-1))]),             // kat = mol/s
            // Photometry.
            (Dimension::luminous_flux(), &[(Luminous, Exp::int(1)), (SolidAngle, Exp::int(1))]),           // lm = cd·sr
            (Dimension::illuminance(), &[(Luminous, Exp::int(1)), (SolidAngle, Exp::int(1)), (Length, Exp::int(-2))]), // lx
            (Dimension::luminance(), &[(Luminous, Exp::int(1)), (Length, Exp::int(-2))]),                  // nit = cd/m²
            // Rates, flow, concentration, viscosity, fuel economy.
            (Dimension::data_rate(), &[(Information, Exp::int(1)), (Time, Exp::int(-1))]),                 // bit/s
            (Dimension::volumetric_flow(), &[(Length, Exp::int(3)), (Time, Exp::int(-1))]),                // m³/s
            (Dimension::molar_concentration(), &[(Amount, Exp::int(1)), (Length, Exp::int(-3))]),          // mol/m³
            (Dimension::molality(), &[(Amount, Exp::int(1)), (Mass, Exp::int(-1))]),                       // mol/kg
            (Dimension::dynamic_viscosity(), &[(Mass, Exp::int(1)), (Length, Exp::int(-1)), (Time, Exp::int(-1))]), // Pa·s
            (Dimension::kinematic_viscosity(), &[(Length, Exp::int(2)), (Time, Exp::int(-1))]),            // m²/s
            (Dimension::fuel_economy(), &[(Length, Exp::int(-2))]),                                        // m/m³ = L⁻²
        ];
        for (dim, expected) in cases {
            for axis in BaseDim::ALL {
                let want = expected.iter().find(|(a, _)| *a == axis).map(|(_, e)| *e).unwrap_or(Exp::int(0));
                assert_eq!(dim.exponent(axis), want, "{} exponent on {:?}", dim, axis);
            }
        }
        // Cross-checks against the composition algebra: each derived dim equals the operation that
        // defines it, so the named helpers can never drift from their formulas.
        assert_eq!(Dimension::absorbed_dose(), Dimension::energy().div(Dimension::mass()));
        assert_eq!(Dimension::radioactivity(), Dimension::frequency()); // Bq is dimensionally a frequency
        assert_eq!(Dimension::data_rate(), Dimension::information().div(Dimension::time()));
        assert_eq!(Dimension::volumetric_flow(), Dimension::volume().div(Dimension::time()));
        assert_eq!(Dimension::molar_concentration(), Dimension::amount().div(Dimension::volume()));
        assert_eq!(Dimension::dynamic_viscosity(), Dimension::pressure().mul(Dimension::time()));
        assert_eq!(Dimension::kinematic_viscosity(), Dimension::area().div(Dimension::time()));
        assert_eq!(Dimension::fuel_economy(), Dimension::length().div(Dimension::volume()));
        assert_eq!(Dimension::illuminance(), Dimension::luminous_flux().div(Dimension::area()));
        // Kinematic viscosity = dynamic viscosity / density (the physical relation μ/ρ = ν).
        assert_eq!(Dimension::kinematic_viscosity(), Dimension::dynamic_viscosity().div(Dimension::density()));
        // Electromagnetism: each derived EM dimension equals its defining SI relation.
        assert_eq!(Dimension::conductance(), Dimension::resistance().recip()); // S = Ω⁻¹
        assert!(Dimension::conductance().mul(Dimension::resistance()).is_dimensionless()); // S·Ω = 1
        assert_eq!(Dimension::capacitance(), Dimension::charge().div(Dimension::voltage())); // F = C/V
        assert_eq!(Dimension::magnetic_flux(), Dimension::voltage().mul(Dimension::time())); // Wb = V·s
        assert_eq!(Dimension::inductance(), Dimension::magnetic_flux().div(Dimension::current())); // H = Wb/A
        assert_eq!(Dimension::magnetic_flux_density(), Dimension::magnetic_flux().div(Dimension::area())); // T = Wb/m²
        assert_eq!(Dimension::surface_tension(), Dimension::force().div(Dimension::length())); // N/m
        // Energy can also be reached as charge × voltage (Q·V = J) — a cross-path consistency check.
        assert_eq!(Dimension::energy(), Dimension::charge().mul(Dimension::voltage()));
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
            assert_eq!(a.mul(b).powi(2), a.powi(2).mul(b.powi(2)), "power distributes over ×");
            assert_eq!(a.powi(2).nth_root(2), a, "(a²) √2 = a");
            assert_eq!(a.powi(0), id, "a⁰ = 1");
            // recip is an involution.
            assert_eq!(a.recip().recip(), a);
        }
    }
}
