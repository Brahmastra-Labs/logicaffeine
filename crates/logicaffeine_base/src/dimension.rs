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
        Exp {
            num: num / g,
            den: den / g,
        }
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
        Exp::new(
            self.num * other.den + other.num * self.den,
            self.den * other.den,
        )
    }

    /// `self − other`.
    pub fn sub(self, other: Exp) -> Exp {
        Exp::new(
            self.num * other.den - other.num * self.den,
            self.den * other.den,
        )
    }

    /// `−self`.
    pub fn neg(self) -> Exp {
        Exp {
            num: -self.num,
            den: self.den,
        }
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
    pub const DIMENSIONLESS: Dimension = Dimension {
        e: [Exp::ZERO; BaseDim::COUNT],
    };

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
    pub fn length() -> Dimension {
        Dimension::base(BaseDim::Length)
    }
    pub fn mass() -> Dimension {
        Dimension::base(BaseDim::Mass)
    }
    pub fn time() -> Dimension {
        Dimension::base(BaseDim::Time)
    }
    pub fn current() -> Dimension {
        Dimension::base(BaseDim::Current)
    }
    pub fn temperature() -> Dimension {
        Dimension::base(BaseDim::Temperature)
    }
    pub fn amount() -> Dimension {
        Dimension::base(BaseDim::Amount)
    }
    pub fn luminous() -> Dimension {
        Dimension::base(BaseDim::Luminous)
    }
    pub fn angle() -> Dimension {
        Dimension::base(BaseDim::Angle)
    }
    pub fn solid_angle() -> Dimension {
        Dimension::base(BaseDim::SolidAngle)
    }
    pub fn information() -> Dimension {
        Dimension::base(BaseDim::Information)
    }

    // ---- Common derived dimensions, composed from the base ones ----
    pub fn area() -> Dimension {
        Self::length().powi(2)
    }
    pub fn volume() -> Dimension {
        Self::length().powi(3)
    }
    pub fn speed() -> Dimension {
        Self::length().div(Self::time())
    }
    pub fn acceleration() -> Dimension {
        Self::speed().div(Self::time())
    }
    pub fn frequency() -> Dimension {
        Self::time().recip()
    }
    pub fn force() -> Dimension {
        Self::mass().mul(Self::acceleration())
    }
    pub fn energy() -> Dimension {
        Self::force().mul(Self::length())
    }
    pub fn power() -> Dimension {
        Self::energy().div(Self::time())
    }
    pub fn pressure() -> Dimension {
        Self::force().div(Self::area())
    }
    pub fn charge() -> Dimension {
        Self::current().mul(Self::time())
    }
    pub fn voltage() -> Dimension {
        Self::power().div(Self::current())
    }
    pub fn resistance() -> Dimension {
        Self::voltage().div(Self::current())
    }
    pub fn conductance() -> Dimension {
        Self::resistance().recip()
    } // siemens: A/V = Ω⁻¹
    pub fn capacitance() -> Dimension {
        Self::charge().div(Self::voltage())
    } // farad: C/V
    pub fn magnetic_flux() -> Dimension {
        Self::voltage().mul(Self::time())
    } // weber: V·s
    pub fn inductance() -> Dimension {
        Self::magnetic_flux().div(Self::current())
    } // henry: Wb/A
    pub fn magnetic_flux_density() -> Dimension {
        Self::magnetic_flux().div(Self::area())
    } // tesla: Wb/m²
    pub fn surface_tension() -> Dimension {
        Self::force().div(Self::length())
    } // N/m = M·T⁻²
    pub fn density() -> Dimension {
        Self::mass().div(Self::volume())
    }
    // Radiation.
    pub fn absorbed_dose() -> Dimension {
        Self::energy().div(Self::mass())
    } // gray, sievert: J/kg = L²·T⁻²
    pub fn radioactivity() -> Dimension {
        Self::frequency()
    } // becquerel: decays per second = T⁻¹
    pub fn exposure() -> Dimension {
        Self::charge().div(Self::mass())
    } // roentgen: C/kg
    pub fn catalytic_activity() -> Dimension {
        Self::amount().div(Self::time())
    } // katal: mol/s
      // Photometry.
    pub fn luminous_flux() -> Dimension {
        Self::luminous().mul(Self::solid_angle())
    } // lumen: cd·sr
    pub fn illuminance() -> Dimension {
        Self::luminous_flux().div(Self::area())
    } // lux: lm/m²
    pub fn luminance() -> Dimension {
        Self::luminous().div(Self::area())
    } // nit: cd/m²
      // Rate / flow / concentration / viscosity / efficiency.
    pub fn data_rate() -> Dimension {
        Self::information().div(Self::time())
    } // bit/s
    pub fn volumetric_flow() -> Dimension {
        Self::volume().div(Self::time())
    } // m³/s
    pub fn molar_concentration() -> Dimension {
        Self::amount().div(Self::volume())
    } // mol/m³
    pub fn molality() -> Dimension {
        Self::amount().div(Self::mass())
    } // mol/kg
    pub fn dynamic_viscosity() -> Dimension {
        Self::pressure().mul(Self::time())
    } // Pa·s = M·L⁻¹·T⁻¹
    pub fn kinematic_viscosity() -> Dimension {
        Self::area().div(Self::time())
    } // m²/s = L²·T⁻¹
    pub fn fuel_economy() -> Dimension {
        Self::length().div(Self::volume())
    } // distance/volume (mpg) = L⁻²

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
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/dimension/tests.rs"]
mod tests;
