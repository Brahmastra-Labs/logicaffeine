//! Quantities: an exact magnitude carrying a physical [`Dimension`] and a unit.
//!
//! A *quantity* is a number with a unit — `2 inches`, `5 kilograms`, `9.8 m/s²`. Internally a
//! [`Quantity`] stores its magnitude in the **SI base unit** for its dimension (metres, kilograms,
//! seconds, …) as an exact [`Rational`], so:
//!
//! - addition/subtraction require the SAME dimension and just add magnitudes,
//! - multiplication/division combine dimensions (`Length × Length = Area`),
//! - conversion to another unit of the same dimension is exact (`÷ scale`, offset-aware), and
//! - converting across dimensions (`Length → Mass`) is impossible — the forbidden cast.
//!
//! Because every unit's scale to the SI base is an exact rational (1 inch = 127/5000 m *exactly*),
//! the whole thing is lossless: `2 inches + 5 centimetres` is `63/625 m`, which `in feet` is exactly
//! `42/127`. A [`Unit`] may be *affine* (a nonzero `offset`, like °C/°F), so a temperature converts
//! with scale AND offset; linear units have `offset = 0`.

use crate::dimension::Dimension;
use crate::numeric::Rational;

/// A unit of measurement: its dimension and how to map a value in this unit to the SI base.
/// `si = value · scale + offset`. Linear units have `offset = 0`; affine units (°C, °F) do not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub symbol: &'static str,
    pub dimension: Dimension,
    /// Multiply a value in this unit by `scale` to reach the SI base unit. Exact, nonzero.
    pub scale: Rational,
    /// The affine zero offset, added after scaling (0 for linear units).
    pub offset: Rational,
}

impl Unit {
    /// A linear unit (`si = value · scale`).
    pub fn linear(symbol: &'static str, dimension: Dimension, scale: Rational) -> Unit {
        Unit { symbol, dimension, scale, offset: Rational::zero() }
    }

    /// An affine unit (`si = value · scale + offset`) — temperatures.
    pub fn affine(symbol: &'static str, dimension: Dimension, scale: Rational, offset: Rational) -> Unit {
        Unit { symbol, dimension, scale, offset }
    }
}

/// An exact quantity: a magnitude in the SI base unit, tagged with its dimension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quantity {
    /// The magnitude expressed in the SI base unit of `dimension` (metres, kilograms, …).
    magnitude: Rational,
    dimension: Dimension,
}

impl Quantity {
    /// A quantity of `value` of `unit` — normalized to the SI base (`value·scale + offset`).
    pub fn of(value: Rational, unit: &Unit) -> Quantity {
        Quantity {
            magnitude: value.mul(&unit.scale).add(&unit.offset),
            dimension: unit.dimension,
        }
    }

    /// A dimensionless quantity (a pure number) — for scaling.
    pub fn scalar(value: Rational) -> Quantity {
        Quantity { magnitude: value, dimension: Dimension::DIMENSIONLESS }
    }

    /// A quantity given directly by its SI-base magnitude and dimension — used for physical
    /// constants and any value whose dimension has no single named unit (e.g. the gas constant).
    pub fn si(magnitude: Rational, dimension: Dimension) -> Quantity {
        Quantity { magnitude, dimension }
    }

    /// The magnitude in the SI base unit.
    pub fn magnitude_si(&self) -> &Rational {
        &self.magnitude
    }

    pub fn dimension(&self) -> Dimension {
        self.dimension
    }

    /// The magnitude expressed IN `unit` — `None` if the dimensions differ (the forbidden cast).
    /// Linear: `(si)/scale`; affine: `(si − offset)/scale`. `1 mile in feet` is exactly `5280`.
    pub fn in_unit(&self, unit: &Unit) -> Option<Rational> {
        if self.dimension != unit.dimension {
            return None;
        }
        self.magnitude.sub(&unit.offset).div(&unit.scale)
    }

    /// `self + other` — `None` unless the dimensions match (you cannot add Length to Mass).
    pub fn add(&self, other: &Quantity) -> Option<Quantity> {
        if self.dimension != other.dimension {
            return None;
        }
        Some(Quantity { magnitude: self.magnitude.add(&other.magnitude), dimension: self.dimension })
    }

    /// `self − other` — `None` unless the dimensions match.
    pub fn sub(&self, other: &Quantity) -> Option<Quantity> {
        if self.dimension != other.dimension {
            return None;
        }
        Some(Quantity { magnitude: self.magnitude.sub(&other.magnitude), dimension: self.dimension })
    }

    /// `self × other` — magnitudes multiply, dimensions combine (`Length × Length = Area`).
    pub fn mul(&self, other: &Quantity) -> Quantity {
        Quantity {
            magnitude: self.magnitude.mul(&other.magnitude),
            dimension: self.dimension.mul(other.dimension),
        }
    }

    /// `self ÷ other` — magnitudes divide, dimensions subtract. `None` on a zero divisor magnitude.
    pub fn div(&self, other: &Quantity) -> Option<Quantity> {
        Some(Quantity {
            magnitude: self.magnitude.div(&other.magnitude)?,
            dimension: self.dimension.div(other.dimension),
        })
    }

    /// Choose the most human-readable unit from `ladder` and return `(magnitude_in_it, unit)`.
    /// The winner is the unit whose magnitude has the smallest absolute value that is still `≥ 1`
    /// (so `1500 m → (1.5, km)`, `0.5 km → (500, m)`); if every candidate is below 1 the largest
    /// magnitude (closest to 1 from below) wins. Units of a different dimension are skipped, so a
    /// ladder with no same-dimension unit yields `None`. Exact — the conversion never rounds, and
    /// `Quantity::of(mag, unit)` reconstructs `self`. This is Design Law #3 (auto-scale to the most
    /// human unit), kept pure at the base layer so the interpreter `Showable` and AOT both reuse it.
    pub fn in_best_unit<'a>(&self, ladder: &'a [Unit]) -> Option<(Rational, &'a Unit)> {
        let one = Rational::one();
        let mut best: Option<(Rational, &Unit)> = None;
        for u in ladder {
            let Some(mag) = self.in_unit(u) else { continue };
            best = Some(match best {
                None => (mag, u),
                Some((best_mag, best_u)) => {
                    let (abs, best_abs) = (mag.abs(), best_mag.abs());
                    let take = match (abs >= one, best_abs >= one) {
                        (true, true) => abs < best_abs,   // both ≥ 1: prefer the one closest to 1 from above
                        (true, false) => true,            // candidate qualifies, incumbent doesn't
                        (false, true) => false,           // keep the qualifying incumbent
                        (false, false) => abs > best_abs, // both < 1: prefer the one closest to 1 from below
                    };
                    if take { (mag, u) } else { (best_mag, best_u) }
                }
            });
        }
        best
    }
}

/// The unit catalog — every scale an EXACT rational to the SI base. Organized by dimension.
/// (Irrational-scaled units like `parsec` (648000/π·AU) and `mach` are intentionally omitted
/// until a float-backed quantity exists; everything here is lossless.) A later wave moves this
/// to a growable data file; the math and the names are settled here first.
pub mod units {
    use super::Unit;
    use crate::dimension::Dimension;
    use crate::numeric::Rational;

    fn r(n: i64, d: i64) -> Rational {
        Rational::from_ratio_i64(n, d).expect("nonzero denominator")
    }
    fn i(n: i64) -> Rational {
        Rational::from_i64(n)
    }
    /// `n³` of a rational (exact).
    fn cube(x: Rational) -> Rational {
        x.pow(3).expect("cube exists")
    }
    /// `n²` of a rational (exact).
    fn square(x: Rational) -> Rational {
        x.pow(2).expect("square exists")
    }

    // Shared exact anchors.
    fn inch_m() -> Rational { r(127, 5000) } // 1 inch = 0.0254 m, exactly
    fn foot_m() -> Rational { r(381, 1250) } // 0.3048 m
    fn lb_kg() -> Rational { r(45_359_237, 100_000_000) } // 0.45359237 kg
    fn g0() -> Rational { r(980_665, 100_000) } // standard gravity 9.80665 m/s²
    /// US liquid gallon = 231 in³ (exact), in m³.
    fn us_gallon_m3() -> Rational { cube(inch_m()).mul(&i(231)) }
    /// Imperial gallon = 4.54609 L (exact), in m³.
    fn imp_gallon_m3() -> Rational { r(454_609, 100_000_000) }

    // ============================ LENGTH (base: metre) ============================
    pub fn metre() -> Unit { Unit::linear("m", Dimension::length(), Rational::one()) }
    pub fn kilometre() -> Unit { Unit::linear("km", Dimension::length(), i(1000)) }
    pub fn hectometre() -> Unit { Unit::linear("hm", Dimension::length(), i(100)) }
    pub fn dekametre() -> Unit { Unit::linear("dam", Dimension::length(), i(10)) }
    pub fn decimetre() -> Unit { Unit::linear("dm", Dimension::length(), r(1, 10)) }
    pub fn centimetre() -> Unit { Unit::linear("cm", Dimension::length(), r(1, 100)) }
    pub fn millimetre() -> Unit { Unit::linear("mm", Dimension::length(), r(1, 1000)) }
    pub fn micrometre() -> Unit { Unit::linear("µm", Dimension::length(), r(1, 1_000_000)) }
    pub fn nanometre() -> Unit { Unit::linear("nm", Dimension::length(), r(1, 1_000_000_000)) }
    pub fn angstrom() -> Unit { Unit::linear("Å", Dimension::length(), r(1, 10_000_000_000)) }
    pub fn inch() -> Unit { Unit::linear("in", Dimension::length(), inch_m()) }
    pub fn foot() -> Unit { Unit::linear("ft", Dimension::length(), foot_m()) }
    pub fn yard() -> Unit { Unit::linear("yd", Dimension::length(), r(1143, 1250)) } // 0.9144 m
    pub fn mile() -> Unit { Unit::linear("mi", Dimension::length(), r(201_168, 125)) } // 1609.344 m
    pub fn nautical_mile() -> Unit { Unit::linear("nmi", Dimension::length(), i(1852)) }
    pub fn fathom() -> Unit { Unit::linear("ftm", Dimension::length(), r(1143, 625)) } // 2 yd
    pub fn furlong() -> Unit { Unit::linear("fur", Dimension::length(), r(25_146, 125)) } // 201.168 m
    pub fn chain() -> Unit { Unit::linear("ch", Dimension::length(), r(12_573, 625)) } // 20.1168 m
    pub fn rod() -> Unit { Unit::linear("rod", Dimension::length(), r(12_573, 2500)) } // 5.0292 m
    pub fn hand() -> Unit { Unit::linear("hand", Dimension::length(), r(127, 1250)) } // 4 in
    pub fn point() -> Unit { Unit::linear("pt", Dimension::length(), r(127, 360_000)) } // 1/72 in
    pub fn pica() -> Unit { Unit::linear("pica", Dimension::length(), r(127, 30_000)) } // 1/6 in
    pub fn thou() -> Unit { Unit::linear("thou", Dimension::length(), r(127, 5_000_000)) } // 1/1000 in
    pub fn astronomical_unit() -> Unit { Unit::linear("AU", Dimension::length(), i(149_597_870_700)) }
    pub fn light_year() -> Unit { Unit::linear("ly", Dimension::length(), i(9_460_730_472_580_800)) } // c·Julian yr

    // ============================ MASS (base: kilogram) ============================
    pub fn kilogram() -> Unit { Unit::linear("kg", Dimension::mass(), Rational::one()) }
    pub fn gram() -> Unit { Unit::linear("g", Dimension::mass(), r(1, 1000)) }
    pub fn milligram() -> Unit { Unit::linear("mg", Dimension::mass(), r(1, 1_000_000)) }
    pub fn microgram() -> Unit { Unit::linear("µg", Dimension::mass(), r(1, 1_000_000_000)) }
    pub fn tonne() -> Unit { Unit::linear("t", Dimension::mass(), i(1000)) }
    pub fn pound() -> Unit { Unit::linear("lb", Dimension::mass(), lb_kg()) }
    pub fn ounce() -> Unit { Unit::linear("oz", Dimension::mass(), lb_kg().div(&i(16)).unwrap()) }
    pub fn stone() -> Unit { Unit::linear("st", Dimension::mass(), lb_kg().mul(&i(14))) }
    pub fn short_ton() -> Unit { Unit::linear("ton", Dimension::mass(), lb_kg().mul(&i(2000))) }
    pub fn long_ton() -> Unit { Unit::linear("LT", Dimension::mass(), lb_kg().mul(&i(2240))) }
    pub fn hundredweight() -> Unit { Unit::linear("cwt", Dimension::mass(), lb_kg().mul(&i(100))) }
    pub fn grain() -> Unit { Unit::linear("gr", Dimension::mass(), lb_kg().div(&i(7000)).unwrap()) }
    pub fn dram() -> Unit { Unit::linear("dr", Dimension::mass(), lb_kg().div(&i(256)).unwrap()) } // oz/16
    pub fn carat() -> Unit { Unit::linear("ct", Dimension::mass(), r(1, 5000)) } // 0.2 g
    pub fn troy_ounce() -> Unit { Unit::linear("ozt", Dimension::mass(), r(311_034_768, 10_000_000_000)) } // 31.1034768 g
    pub fn troy_pound() -> Unit { Unit::linear("lbt", Dimension::mass(), r(311_034_768, 10_000_000_000).mul(&i(12))) }

    // ============================ TIME (base: second) ============================
    pub fn second() -> Unit { Unit::linear("s", Dimension::time(), Rational::one()) }
    pub fn millisecond() -> Unit { Unit::linear("ms", Dimension::time(), r(1, 1000)) }
    pub fn microsecond() -> Unit { Unit::linear("µs", Dimension::time(), r(1, 1_000_000)) }
    pub fn nanosecond() -> Unit { Unit::linear("ns", Dimension::time(), r(1, 1_000_000_000)) }
    pub fn minute() -> Unit { Unit::linear("min", Dimension::time(), i(60)) }
    pub fn hour() -> Unit { Unit::linear("h", Dimension::time(), i(3600)) }
    pub fn day() -> Unit { Unit::linear("day", Dimension::time(), i(86_400)) }
    pub fn week() -> Unit { Unit::linear("wk", Dimension::time(), i(604_800)) }
    pub fn fortnight() -> Unit { Unit::linear("fortnight", Dimension::time(), i(1_209_600)) }
    pub fn julian_year() -> Unit { Unit::linear("yr", Dimension::time(), i(31_557_600)) } // 365.25 d
    pub fn decade() -> Unit { Unit::linear("decade", Dimension::time(), i(315_576_000)) }
    pub fn century() -> Unit { Unit::linear("century", Dimension::time(), i(3_155_760_000)) }

    // ====================== TEMPERATURE (base: kelvin), AFFINE ======================
    pub fn kelvin() -> Unit { Unit::affine("K", Dimension::temperature(), Rational::one(), Rational::zero()) }
    pub fn celsius() -> Unit { Unit::affine("°C", Dimension::temperature(), Rational::one(), r(5463, 20)) } // +273.15
    pub fn fahrenheit() -> Unit { Unit::affine("°F", Dimension::temperature(), r(5, 9), r(45967, 180)) }
    pub fn rankine() -> Unit { Unit::affine("°R", Dimension::temperature(), r(5, 9), Rational::zero()) } // K = °R·5/9
    pub fn reaumur() -> Unit { Unit::affine("°Ré", Dimension::temperature(), r(5, 4), r(5463, 20)) } // K = °Ré·5/4 + 273.15

    // ============================ VOLUME (base: m³) ============================
    pub fn cubic_metre() -> Unit { Unit::linear("m³", Dimension::volume(), Rational::one()) }
    pub fn litre() -> Unit { Unit::linear("L", Dimension::volume(), r(1, 1000)) }
    pub fn millilitre() -> Unit { Unit::linear("mL", Dimension::volume(), r(1, 1_000_000)) }
    pub fn centilitre() -> Unit { Unit::linear("cL", Dimension::volume(), r(1, 100_000)) }
    pub fn decilitre() -> Unit { Unit::linear("dL", Dimension::volume(), r(1, 10_000)) }
    pub fn cubic_centimetre() -> Unit { Unit::linear("cc", Dimension::volume(), r(1, 1_000_000)) }
    pub fn cubic_inch() -> Unit { Unit::linear("in³", Dimension::volume(), cube(inch_m())) }
    pub fn cubic_foot() -> Unit { Unit::linear("ft³", Dimension::volume(), cube(foot_m())) }
    // ---- US liquid / COOKING (anchored to the 231-in³ gallon) ----
    pub fn us_gallon() -> Unit { Unit::linear("gal", Dimension::volume(), us_gallon_m3()) }
    pub fn us_quart() -> Unit { Unit::linear("qt", Dimension::volume(), us_gallon_m3().div(&i(4)).unwrap()) }
    pub fn us_pint() -> Unit { Unit::linear("pt", Dimension::volume(), us_gallon_m3().div(&i(8)).unwrap()) }
    pub fn us_cup() -> Unit { Unit::linear("cup", Dimension::volume(), us_gallon_m3().div(&i(16)).unwrap()) }
    pub fn us_gill() -> Unit { Unit::linear("gill", Dimension::volume(), us_gallon_m3().div(&i(32)).unwrap()) }
    pub fn us_fluid_ounce() -> Unit { Unit::linear("fl oz", Dimension::volume(), us_gallon_m3().div(&i(128)).unwrap()) }
    pub fn tablespoon() -> Unit { Unit::linear("tbsp", Dimension::volume(), us_gallon_m3().div(&i(256)).unwrap()) }
    pub fn teaspoon() -> Unit { Unit::linear("tsp", Dimension::volume(), us_gallon_m3().div(&i(768)).unwrap()) }
    pub fn oil_barrel() -> Unit { Unit::linear("bbl", Dimension::volume(), us_gallon_m3().mul(&i(42))) } // 42 US gal
    // ---- Imperial ----
    pub fn imperial_gallon() -> Unit { Unit::linear("imp gal", Dimension::volume(), imp_gallon_m3()) }
    pub fn imperial_pint() -> Unit { Unit::linear("imp pt", Dimension::volume(), imp_gallon_m3().div(&i(8)).unwrap()) }
    pub fn imperial_fluid_ounce() -> Unit { Unit::linear("imp fl oz", Dimension::volume(), imp_gallon_m3().div(&i(160)).unwrap()) }

    // ============================ AREA (base: m²) ============================
    pub fn square_metre() -> Unit { Unit::linear("m²", Dimension::area(), Rational::one()) }
    pub fn square_kilometre() -> Unit { Unit::linear("km²", Dimension::area(), i(1_000_000)) }
    pub fn square_centimetre() -> Unit { Unit::linear("cm²", Dimension::area(), r(1, 10_000)) }
    pub fn square_inch() -> Unit { Unit::linear("in²", Dimension::area(), square(inch_m())) }
    pub fn square_foot() -> Unit { Unit::linear("ft²", Dimension::area(), square(foot_m())) }
    pub fn square_yard() -> Unit { Unit::linear("yd²", Dimension::area(), square(r(1143, 1250))) }
    pub fn square_mile() -> Unit { Unit::linear("mi²", Dimension::area(), square(r(201_168, 125))) }
    pub fn hectare() -> Unit { Unit::linear("ha", Dimension::area(), i(10_000)) }
    pub fn are() -> Unit { Unit::linear("a", Dimension::area(), i(100)) }
    pub fn acre() -> Unit { Unit::linear("ac", Dimension::area(), square(foot_m()).mul(&i(43_560))) } // 43560 ft²

    // ============================ SPEED (base: m/s) ============================
    pub fn metre_per_second() -> Unit { Unit::linear("m/s", Dimension::speed(), Rational::one()) }
    pub fn kilometre_per_hour() -> Unit { Unit::linear("km/h", Dimension::speed(), r(5, 18)) } // 1000/3600
    pub fn mile_per_hour() -> Unit { Unit::linear("mph", Dimension::speed(), r(201_168, 125).div(&i(3600)).unwrap()) }
    pub fn foot_per_second() -> Unit { Unit::linear("ft/s", Dimension::speed(), foot_m()) }
    pub fn knot() -> Unit { Unit::linear("kn", Dimension::speed(), r(1852, 3600)) } // nmi/h

    // ============================ FREQUENCY (base: 1/s) ============================
    pub fn hertz() -> Unit { Unit::linear("Hz", Dimension::frequency(), Rational::one()) }
    pub fn kilohertz() -> Unit { Unit::linear("kHz", Dimension::frequency(), i(1000)) }
    pub fn megahertz() -> Unit { Unit::linear("MHz", Dimension::frequency(), i(1_000_000)) }
    pub fn gigahertz() -> Unit { Unit::linear("GHz", Dimension::frequency(), i(1_000_000_000)) }
    pub fn rpm() -> Unit { Unit::linear("rpm", Dimension::frequency(), r(1, 60)) }

    // ============================ FORCE (base: newton) ============================
    pub fn newton() -> Unit { Unit::linear("N", Dimension::force(), Rational::one()) }
    pub fn kilonewton() -> Unit { Unit::linear("kN", Dimension::force(), i(1000)) }
    pub fn dyne() -> Unit { Unit::linear("dyn", Dimension::force(), r(1, 100_000)) }
    pub fn kilogram_force() -> Unit { Unit::linear("kgf", Dimension::force(), g0()) }
    pub fn pound_force() -> Unit { Unit::linear("lbf", Dimension::force(), lb_kg().mul(&g0())) }

    // ============================ ENERGY (base: joule) ============================
    pub fn joule() -> Unit { Unit::linear("J", Dimension::energy(), Rational::one()) }
    pub fn kilojoule() -> Unit { Unit::linear("kJ", Dimension::energy(), i(1000)) }
    pub fn megajoule() -> Unit { Unit::linear("MJ", Dimension::energy(), i(1_000_000)) }
    pub fn calorie() -> Unit { Unit::linear("cal", Dimension::energy(), r(523, 125)) } // 4.184 J
    pub fn kilocalorie() -> Unit { Unit::linear("kcal", Dimension::energy(), r(4184, 1)) }
    pub fn watt_hour() -> Unit { Unit::linear("Wh", Dimension::energy(), i(3600)) }
    pub fn kilowatt_hour() -> Unit { Unit::linear("kWh", Dimension::energy(), i(3_600_000)) }
    pub fn erg() -> Unit { Unit::linear("erg", Dimension::energy(), r(1, 10_000_000)) }
    pub fn electronvolt() -> Unit {
        // 1.602176634e-19 J = 1602176634 / 10^28 (the 10^28 denominator overflows i64, so build it
        // from BigInt — exact).
        use crate::numeric::BigInt;
        let scale = Rational::new(BigInt::from_i64(1_602_176_634), BigInt::from_i64(10).pow(28))
            .expect("nonzero denominator");
        Unit::linear("eV", Dimension::energy(), scale)
    }

    // ============================ POWER (base: watt) ============================
    pub fn watt() -> Unit { Unit::linear("W", Dimension::power(), Rational::one()) }
    pub fn kilowatt() -> Unit { Unit::linear("kW", Dimension::power(), i(1000)) }
    pub fn megawatt() -> Unit { Unit::linear("MW", Dimension::power(), i(1_000_000)) }
    pub fn horsepower() -> Unit { Unit::linear("hp", Dimension::power(), lb_kg().mul(&g0()).mul(&foot_m()).mul(&i(550))) } // 550 ft·lbf/s

    // ============================ PRESSURE (base: pascal) ============================
    pub fn pascal() -> Unit { Unit::linear("Pa", Dimension::pressure(), Rational::one()) }
    pub fn kilopascal() -> Unit { Unit::linear("kPa", Dimension::pressure(), i(1000)) }
    pub fn bar() -> Unit { Unit::linear("bar", Dimension::pressure(), i(100_000)) }
    pub fn millibar() -> Unit { Unit::linear("mbar", Dimension::pressure(), i(100)) }
    pub fn atmosphere() -> Unit { Unit::linear("atm", Dimension::pressure(), i(101_325)) }
    pub fn psi() -> Unit { Unit::linear("psi", Dimension::pressure(), lb_kg().mul(&g0()).div(&square(inch_m())).unwrap()) } // lbf/in²

    // ============================ INFORMATION (base: bit) ============================
    pub fn bit() -> Unit { Unit::linear("bit", Dimension::information(), Rational::one()) }
    pub fn byte() -> Unit { Unit::linear("B", Dimension::information(), i(8)) }
    pub fn kilobit() -> Unit { Unit::linear("kbit", Dimension::information(), i(1000)) }
    pub fn kilobyte() -> Unit { Unit::linear("kB", Dimension::information(), i(8000)) }
    pub fn megabyte() -> Unit { Unit::linear("MB", Dimension::information(), i(8_000_000)) }
    pub fn gigabyte() -> Unit { Unit::linear("GB", Dimension::information(), i(8_000_000_000)) }
    pub fn kibibyte() -> Unit { Unit::linear("KiB", Dimension::information(), i(8 * 1024)) }
    pub fn mebibyte() -> Unit { Unit::linear("MiB", Dimension::information(), i(8 * 1024 * 1024)) }
    pub fn gibibyte() -> Unit { Unit::linear("GiB", Dimension::information(), i(8 * 1024 * 1024 * 1024)) }

    // ============================ ANGLE (base: radian) ============================
    // (Degrees etc. are exact multiples of a turn; radian-to-degree is irrational, so these are
    // defined against the TURN as the natural rational anchor — full circle = 1 turn.)
    pub fn turn() -> Unit { Unit::linear("turn", Dimension::angle(), Rational::one()) }
    pub fn degree() -> Unit { Unit::linear("°", Dimension::angle(), r(1, 360)) }
    pub fn gradian() -> Unit { Unit::linear("grad", Dimension::angle(), r(1, 400)) }
    pub fn arcminute() -> Unit { Unit::linear("′", Dimension::angle(), r(1, 21_600)) } // degree/60
    pub fn arcsecond() -> Unit { Unit::linear("″", Dimension::angle(), r(1, 1_296_000)) } // degree/3600

    // ============================ ELECTRICAL ============================
    pub fn ampere() -> Unit { Unit::linear("A", Dimension::current(), Rational::one()) }
    pub fn milliampere() -> Unit { Unit::linear("mA", Dimension::current(), r(1, 1000)) }
    pub fn coulomb() -> Unit { Unit::linear("C", Dimension::charge(), Rational::one()) }
    pub fn ampere_hour() -> Unit { Unit::linear("Ah", Dimension::charge(), i(3600)) }
    pub fn milliampere_hour() -> Unit { Unit::linear("mAh", Dimension::charge(), r(18, 5)) } // 3.6 C
    pub fn volt() -> Unit { Unit::linear("V", Dimension::voltage(), Rational::one()) }
    pub fn millivolt() -> Unit { Unit::linear("mV", Dimension::voltage(), r(1, 1000)) }
    pub fn kilovolt() -> Unit { Unit::linear("kV", Dimension::voltage(), i(1000)) }
    pub fn ohm() -> Unit { Unit::linear("Ω", Dimension::resistance(), Rational::one()) }

    // ============================ AMOUNT / LUMINOUS ============================
    pub fn mole() -> Unit { Unit::linear("mol", Dimension::amount(), Rational::one()) }
    pub fn candela() -> Unit { Unit::linear("cd", Dimension::luminous(), Rational::one()) }

    /// `mantissa · 10^pow10` as an exact rational (for values past i64, e.g. stellar masses).
    fn e(mantissa: i64, pow10: u32) -> Rational {
        use crate::numeric::BigInt;
        Rational::from_bigint(BigInt::from_i64(mantissa).mul(&BigInt::from_i64(10).pow(pow10)))
    }

    // ============================ COOKING EXTRAS (volume) ============================
    pub fn stick_of_butter() -> Unit { Unit::linear("stick", Dimension::volume(), us_gallon_m3().div(&i(32)).unwrap()) } // ½ cup = 8 tbsp
    pub fn dash() -> Unit { Unit::linear("dash", Dimension::volume(), us_gallon_m3().div(&i(6144)).unwrap()) } // ⅛ tsp
    pub fn pinch() -> Unit { Unit::linear("pinch", Dimension::volume(), us_gallon_m3().div(&i(12288)).unwrap()) } // ¹⁄₁₆ tsp
    pub fn smidgen() -> Unit { Unit::linear("smidgen", Dimension::volume(), us_gallon_m3().div(&i(24576)).unwrap()) } // ¹⁄₃₂ tsp
    // US DRY volume (anchored to the 2150.42-in³ bushel).
    fn us_bushel_m3() -> Rational { cube(inch_m()).mul(&r(215_042, 100)) } // 2150.42 in³
    pub fn bushel() -> Unit { Unit::linear("bu", Dimension::volume(), us_bushel_m3()) }
    pub fn peck() -> Unit { Unit::linear("pk", Dimension::volume(), us_bushel_m3().div(&i(4)).unwrap()) }
    pub fn dry_gallon() -> Unit { Unit::linear("dry gal", Dimension::volume(), us_bushel_m3().div(&i(8)).unwrap()) }
    pub fn dry_quart() -> Unit { Unit::linear("dry qt", Dimension::volume(), us_bushel_m3().div(&i(32)).unwrap()) }
    pub fn dry_pint() -> Unit { Unit::linear("dry pt", Dimension::volume(), us_bushel_m3().div(&i(64)).unwrap()) }

    // ============================ NAUTICAL (length) ============================
    pub fn cable() -> Unit { Unit::linear("cable", Dimension::length(), r(926, 5)) } // ¹⁄₁₀ nmi = 185.2 m
    pub fn league() -> Unit { Unit::linear("lea", Dimension::length(), r(603_504, 125)) } // 3 statute miles
    pub fn nautical_league() -> Unit { Unit::linear("nl", Dimension::length(), i(5556)) } // 3 nmi

    // ============================ ASTRONOMICAL ============================
    pub fn light_second() -> Unit { Unit::linear("ls", Dimension::length(), i(299_792_458)) } // c·1 s
    pub fn light_minute() -> Unit { Unit::linear("lmin", Dimension::length(), i(299_792_458 * 60)) }
    pub fn light_hour() -> Unit { Unit::linear("lh", Dimension::length(), i(299_792_458 * 3600)) }
    pub fn light_day() -> Unit { Unit::linear("ld", Dimension::length(), i(299_792_458 * 86_400)) }
    pub fn lunar_distance() -> Unit { Unit::linear("LD", Dimension::length(), i(384_399_000)) } // mean Earth–Moon
    pub fn solar_radius() -> Unit { Unit::linear("R☉", Dimension::length(), i(696_340_000)) }
    pub fn earth_radius() -> Unit { Unit::linear("R⊕", Dimension::length(), i(6_371_000)) } // mean
    // Astronomical MASSES (measured values, exact as the conventional decimals given).
    pub fn solar_mass() -> Unit { Unit::linear("M☉", Dimension::mass(), e(198_892, 25)) } // 1.98892e30 kg
    pub fn earth_mass() -> Unit { Unit::linear("M⊕", Dimension::mass(), e(59_722, 20)) } // 5.9722e24 kg
    pub fn jupiter_mass() -> Unit { Unit::linear("M♃", Dimension::mass(), e(18_982, 23)) } // 1.8982e27 kg

    // ============================ RADIATION ============================
    pub fn gray() -> Unit { Unit::linear("Gy", Dimension::absorbed_dose(), Rational::one()) } // J/kg
    pub fn sievert() -> Unit { Unit::linear("Sv", Dimension::absorbed_dose(), Rational::one()) } // J/kg (equiv dose)
    pub fn rad_unit() -> Unit { Unit::linear("rad", Dimension::absorbed_dose(), r(1, 100)) } // 0.01 Gy
    pub fn rem() -> Unit { Unit::linear("rem", Dimension::absorbed_dose(), r(1, 100)) } // 0.01 Sv
    pub fn becquerel() -> Unit { Unit::linear("Bq", Dimension::radioactivity(), Rational::one()) } // 1/s
    pub fn curie() -> Unit { Unit::linear("Ci", Dimension::radioactivity(), i(37_000_000_000)) } // 3.7e10 Bq
    pub fn roentgen() -> Unit { Unit::linear("R", Dimension::exposure(), r(129, 500_000)) } // 2.58e-4 C/kg
    pub fn katal() -> Unit { Unit::linear("kat", Dimension::catalytic_activity(), Rational::one()) } // mol/s

    // ============================ ILLUMINATION (photometry) ============================
    pub fn lumen() -> Unit { Unit::linear("lm", Dimension::luminous_flux(), Rational::one()) } // cd·sr
    pub fn lux() -> Unit { Unit::linear("lx", Dimension::illuminance(), Rational::one()) } // lm/m²
    pub fn phot() -> Unit { Unit::linear("ph", Dimension::illuminance(), i(10_000)) } // lm/cm²
    pub fn foot_candle() -> Unit { Unit::linear("fc", Dimension::illuminance(), square(r(1250, 381))) } // lm/ft² = 1/0.3048² lx
    pub fn nit() -> Unit { Unit::linear("nit", Dimension::luminance(), Rational::one()) } // cd/m²

    // ============================ DATA RATE (base: bit/s) ============================
    pub fn bit_per_second() -> Unit { Unit::linear("bit/s", Dimension::data_rate(), Rational::one()) }
    pub fn kilobit_per_second() -> Unit { Unit::linear("kbps", Dimension::data_rate(), i(1000)) }
    pub fn megabit_per_second() -> Unit { Unit::linear("Mbps", Dimension::data_rate(), i(1_000_000)) }
    pub fn gigabit_per_second() -> Unit { Unit::linear("Gbps", Dimension::data_rate(), i(1_000_000_000)) }
    pub fn byte_per_second() -> Unit { Unit::linear("B/s", Dimension::data_rate(), i(8)) }
    pub fn megabyte_per_second() -> Unit { Unit::linear("MB/s", Dimension::data_rate(), i(8_000_000)) }

    // ============================ VOLUMETRIC FLOW (base: m³/s) ============================
    pub fn cubic_metre_per_second() -> Unit { Unit::linear("m³/s", Dimension::volumetric_flow(), Rational::one()) }
    pub fn litre_per_second() -> Unit { Unit::linear("L/s", Dimension::volumetric_flow(), r(1, 1000)) }
    pub fn litre_per_minute() -> Unit { Unit::linear("L/min", Dimension::volumetric_flow(), r(1, 60_000)) }
    pub fn gallon_per_minute() -> Unit { Unit::linear("gpm", Dimension::volumetric_flow(), us_gallon_m3().div(&i(60)).unwrap()) }
    pub fn cubic_foot_per_minute() -> Unit { Unit::linear("cfm", Dimension::volumetric_flow(), cube(foot_m()).div(&i(60)).unwrap()) }

    // ============================ CONCENTRATION (base: mol/m³) ============================
    pub fn molar() -> Unit { Unit::linear("M", Dimension::molar_concentration(), i(1000)) } // mol/L
    pub fn millimolar() -> Unit { Unit::linear("mM", Dimension::molar_concentration(), Rational::one()) }
    pub fn micromolar() -> Unit { Unit::linear("µM", Dimension::molar_concentration(), r(1, 1000)) }
    pub fn molal() -> Unit { Unit::linear("m", Dimension::molality(), Rational::one()) } // mol/kg

    // ============================ VISCOSITY ============================
    pub fn pascal_second() -> Unit { Unit::linear("Pa·s", Dimension::dynamic_viscosity(), Rational::one()) }
    pub fn poise() -> Unit { Unit::linear("P", Dimension::dynamic_viscosity(), r(1, 10)) }
    pub fn centipoise() -> Unit { Unit::linear("cP", Dimension::dynamic_viscosity(), r(1, 1000)) }
    pub fn square_metre_per_second() -> Unit { Unit::linear("m²/s", Dimension::kinematic_viscosity(), Rational::one()) }
    pub fn stokes() -> Unit { Unit::linear("St", Dimension::kinematic_viscosity(), r(1, 10_000)) }
    pub fn centistokes() -> Unit { Unit::linear("cSt", Dimension::kinematic_viscosity(), r(1, 1_000_000)) }

    // ============================ FUEL ECONOMY (base: m/m³ = m⁻²) ============================
    pub fn mile_per_gallon() -> Unit { Unit::linear("mpg", Dimension::fuel_economy(), r(201_168, 125).div(&us_gallon_m3()).unwrap()) }
    pub fn km_per_litre() -> Unit { Unit::linear("km/L", Dimension::fuel_economy(), i(1000).div(&r(1, 1000)).unwrap()) } // 10^6 m⁻²

    // ============================ TORQUE (shares the energy dimension M·L²·T⁻²) ============================
    pub fn newton_metre() -> Unit { Unit::linear("N·m", Dimension::energy(), Rational::one()) }
    pub fn pound_foot() -> Unit { Unit::linear("lb·ft", Dimension::energy(), lb_kg().mul(&g0()).mul(&foot_m())) }

    // ============================ EXTRA SI PREFIXES (extend existing dimensions) ============================
    pub fn megametre() -> Unit { Unit::linear("Mm", Dimension::length(), i(1_000_000)) }
    pub fn gigametre() -> Unit { Unit::linear("Gm", Dimension::length(), i(1_000_000_000)) }
    pub fn picometre() -> Unit { Unit::linear("pm", Dimension::length(), r(1, 1_000_000_000_000)) }
    pub fn femtometre() -> Unit { Unit::linear("fm", Dimension::length(), r(1, 1_000_000_000_000_000)) }
    pub fn nanogram() -> Unit { Unit::linear("ng", Dimension::mass(), r(1, 1_000_000_000_000)) }
    pub fn picosecond() -> Unit { Unit::linear("ps", Dimension::time(), r(1, 1_000_000_000_000)) }
    pub fn gigawatt() -> Unit { Unit::linear("GW", Dimension::power(), i(1_000_000_000)) }
    pub fn terawatt() -> Unit { Unit::linear("TW", Dimension::power(), i(1_000_000_000_000)) }
    pub fn gigajoule() -> Unit { Unit::linear("GJ", Dimension::energy(), i(1_000_000_000)) }
    pub fn terajoule() -> Unit { Unit::linear("TJ", Dimension::energy(), i(1_000_000_000_000)) }
    pub fn terabit() -> Unit { Unit::linear("Tbit", Dimension::information(), i(1_000_000_000_000)) }
    pub fn terabyte() -> Unit { Unit::linear("TB", Dimension::information(), i(8_000_000_000_000)) }
    pub fn petabyte() -> Unit { Unit::linear("PB", Dimension::information(), i(8_000_000_000_000_000)) }

    // ============================ ELECTROMAGNETISM ============================
    pub fn siemens() -> Unit { Unit::linear("S", Dimension::conductance(), Rational::one()) }
    pub fn millisiemens() -> Unit { Unit::linear("mS", Dimension::conductance(), r(1, 1000)) }
    pub fn farad() -> Unit { Unit::linear("F", Dimension::capacitance(), Rational::one()) }
    pub fn microfarad() -> Unit { Unit::linear("µF", Dimension::capacitance(), r(1, 1_000_000)) }
    pub fn nanofarad() -> Unit { Unit::linear("nF", Dimension::capacitance(), r(1, 1_000_000_000)) }
    pub fn picofarad() -> Unit { Unit::linear("pF", Dimension::capacitance(), r(1, 1_000_000_000_000)) }
    pub fn weber() -> Unit { Unit::linear("Wb", Dimension::magnetic_flux(), Rational::one()) }
    pub fn maxwell() -> Unit { Unit::linear("Mx", Dimension::magnetic_flux(), r(1, 100_000_000)) } // 1e-8 Wb
    pub fn henry() -> Unit { Unit::linear("H", Dimension::inductance(), Rational::one()) }
    pub fn millihenry() -> Unit { Unit::linear("mH", Dimension::inductance(), r(1, 1000)) }
    pub fn microhenry() -> Unit { Unit::linear("µH", Dimension::inductance(), r(1, 1_000_000)) }
    pub fn tesla() -> Unit { Unit::linear("T", Dimension::magnetic_flux_density(), Rational::one()) }
    pub fn millitesla() -> Unit { Unit::linear("mT", Dimension::magnetic_flux_density(), r(1, 1000)) }
    pub fn gauss() -> Unit { Unit::linear("G", Dimension::magnetic_flux_density(), r(1, 10_000)) } // 1e-4 T

    // ============================ SURFACE TENSION (base: N/m) ============================
    pub fn newton_per_metre() -> Unit { Unit::linear("N/m", Dimension::surface_tension(), Rational::one()) }
    pub fn dyne_per_centimetre() -> Unit { Unit::linear("dyn/cm", Dimension::surface_tension(), r(1, 1000)) } // 1e-3 N/m

    // ============================ COUNTING & RATIO (dimensionless) ============================
    // The base "unit" is a bare count of one; everything here is a pure dimensionless scale factor.
    pub fn each() -> Unit { Unit::linear("ea", Dimension::DIMENSIONLESS, Rational::one()) }
    pub fn pair() -> Unit { Unit::linear("pair", Dimension::DIMENSIONLESS, i(2)) }
    pub fn dozen() -> Unit { Unit::linear("dz", Dimension::DIMENSIONLESS, i(12)) }
    pub fn baker_dozen() -> Unit { Unit::linear("baker's dz", Dimension::DIMENSIONLESS, i(13)) }
    pub fn score() -> Unit { Unit::linear("score", Dimension::DIMENSIONLESS, i(20)) }
    pub fn gross() -> Unit { Unit::linear("gross", Dimension::DIMENSIONLESS, i(144)) }
    pub fn great_gross() -> Unit { Unit::linear("great gross", Dimension::DIMENSIONLESS, i(1728)) }
    pub fn ream() -> Unit { Unit::linear("ream", Dimension::DIMENSIONLESS, i(500)) }
    pub fn percent() -> Unit { Unit::linear("%", Dimension::DIMENSIONLESS, r(1, 100)) }
    pub fn permille() -> Unit { Unit::linear("‰", Dimension::DIMENSIONLESS, r(1, 1000)) }
    pub fn ppm() -> Unit { Unit::linear("ppm", Dimension::DIMENSIONLESS, r(1, 1_000_000)) }
    pub fn ppb() -> Unit { Unit::linear("ppb", Dimension::DIMENSIONLESS, r(1, 1_000_000_000)) }
    pub fn basis_point() -> Unit { Unit::linear("bp", Dimension::DIMENSIONLESS, r(1, 10_000)) }

    /// Look up a unit by an English name, plural, or symbol (case-insensitive) — the surface-syntax
    /// bridge so `quantity(2, "inch")` / `... in "feet"` resolve to a [`Unit`]. This is the growable
    /// catalog's name index; unknown names return `None` (the front-end turns that into a clean
    /// error). Spelling variants and both singular/plural map to the same unit.
    pub fn by_name(name: &str) -> Option<Unit> {
        let n = name.trim().to_ascii_lowercase();
        let u = match n.as_str() {
            // Length.
            "meter" | "metre" | "meters" | "metres" | "m" => metre(),
            "kilometer" | "kilometre" | "kilometers" | "kilometres" | "km" => kilometre(),
            "centimeter" | "centimetre" | "centimeters" | "centimetres" | "cm" => centimetre(),
            "millimeter" | "millimetre" | "millimeters" | "millimetres" | "mm" => millimetre(),
            "micrometer" | "micrometre" | "micron" | "microns" | "µm" | "um" => micrometre(),
            "nanometer" | "nanometre" | "nanometers" | "nanometres" | "nm" => nanometre(),
            "inch" | "inches" | "in" => inch(),
            "foot" | "feet" | "ft" => foot(),
            "yard" | "yards" | "yd" => yard(),
            "mile" | "miles" | "mi" => mile(),
            "nautical mile" | "nautical miles" | "nmi" => nautical_mile(),
            "angstrom" | "angstroms" | "ångström" | "å" => angstrom(),
            "light year" | "light years" | "lightyear" | "ly" => light_year(),
            "astronomical unit" | "astronomical units" | "au" => astronomical_unit(),
            // Mass.
            "kilogram" | "kilograms" | "kg" => kilogram(),
            "gram" | "grams" | "g" => gram(),
            "milligram" | "milligrams" | "mg" => milligram(),
            "microgram" | "micrograms" | "µg" | "ug" => microgram(),
            "tonne" | "tonnes" | "metric ton" | "metric tons" | "t" => tonne(),
            "pound" | "pounds" | "lb" | "lbs" => pound(),
            "ounce" | "ounces" | "oz" => ounce(),
            "stone" | "stones" | "st" => stone(),
            "carat" | "carats" | "ct" => carat(),
            // Time.
            "second" | "seconds" | "sec" | "secs" | "s" => second(),
            "millisecond" | "milliseconds" | "ms" => millisecond(),
            "microsecond" | "microseconds" | "µs" | "us" => microsecond(),
            "nanosecond" | "nanoseconds" | "ns" => nanosecond(),
            "minute" | "minutes" | "min" | "mins" => minute(),
            "hour" | "hours" | "hr" | "hrs" | "h" => hour(),
            "day" | "days" => day(),
            "week" | "weeks" | "wk" => week(),
            "year" | "years" | "yr" | "yrs" => julian_year(),
            // Temperature (affine).
            "kelvin" | "k" => kelvin(),
            "celsius" | "centigrade" | "°c" | "c" => celsius(),
            "fahrenheit" | "°f" | "f" => fahrenheit(),
            "rankine" | "°r" => rankine(),
            "reaumur" | "réaumur" | "°ré" => reaumur(),
            // Volume (incl. cooking).
            "liter" | "litre" | "liters" | "litres" | "l" => litre(),
            "milliliter" | "millilitre" | "milliliters" | "millilitres" | "ml" => millilitre(),
            "cubic meter" | "cubic metre" | "m3" | "m³" => cubic_metre(),
            "gallon" | "gallons" | "gal" => us_gallon(),
            "quart" | "quarts" | "qt" => us_quart(),
            "pint" | "pints" | "pt" => us_pint(),
            "cup" | "cups" => us_cup(),
            "tablespoon" | "tablespoons" | "tbsp" => tablespoon(),
            "teaspoon" | "teaspoons" | "tsp" => teaspoon(),
            "fluid ounce" | "fluid ounces" | "fl oz" => us_fluid_ounce(),
            // Area.
            "square meter" | "square metre" | "square meters" | "square metres" | "m2" => square_metre(),
            "square foot" | "square feet" | "sq ft" => square_foot(),
            "square inch" | "square inches" | "sq in" => square_inch(),
            "hectare" | "hectares" | "ha" => hectare(),
            "acre" | "acres" => acre(),
            // Speed.
            "meter per second" | "metre per second" | "meters per second" | "m/s" => metre_per_second(),
            "kilometer per hour" | "kilometre per hour" | "km/h" | "kph" => kilometre_per_hour(),
            "mile per hour" | "miles per hour" | "mph" => mile_per_hour(),
            "knot" | "knots" | "kn" => knot(),
            // Frequency.
            "hertz" | "hz" => hertz(),
            "kilohertz" | "khz" => kilohertz(),
            "megahertz" | "mhz" => megahertz(),
            "gigahertz" | "ghz" => gigahertz(),
            // Energy.
            "joule" | "joules" | "j" => joule(),
            "kilojoule" | "kilojoules" | "kj" => kilojoule(),
            "calorie" | "calories" | "cal" => calorie(),
            "kilocalorie" | "kilocalories" | "kcal" => kilocalorie(),
            "watt hour" | "watt hours" | "wh" => watt_hour(),
            "kilowatt hour" | "kilowatt hours" | "kwh" => kilowatt_hour(),
            "electronvolt" | "electronvolts" | "ev" => electronvolt(),
            // Power.
            "watt" | "watts" | "w" => watt(),
            "kilowatt" | "kilowatts" | "kw" => kilowatt(),
            "megawatt" | "megawatts" | "mw" => megawatt(),
            "horsepower" | "hp" => horsepower(),
            // Pressure.
            "pascal" | "pascals" | "pa" => pascal(),
            "kilopascal" | "kilopascals" | "kpa" => kilopascal(),
            "bar" | "bars" => bar(),
            "atmosphere" | "atmospheres" | "atm" => atmosphere(),
            "psi" => psi(),
            // Information.
            "bit" | "bits" => bit(),
            "byte" | "bytes" => byte(),
            "kilobyte" | "kilobytes" | "kb" => kilobyte(),
            "megabyte" | "megabytes" | "mb" => megabyte(),
            "gigabyte" | "gigabytes" | "gb" => gigabyte(),
            // Angle (radian is irrational vs. these and is intentionally absent, like parsec/mach).
            "turn" | "turns" => turn(),
            "degree" | "degrees" | "deg" | "°" => degree(),
            "gradian" | "gradians" | "grad" => gradian(),
            "arcminute" | "arcminutes" => arcminute(),
            "arcsecond" | "arcseconds" => arcsecond(),
            // Radiation & catalysis.
            "gray" | "grays" | "gy" => gray(),
            "sievert" | "sieverts" | "sv" => sievert(),
            "rad" | "rads" => rad_unit(),
            "rem" | "rems" => rem(),
            "becquerel" | "becquerels" | "bq" => becquerel(),
            "curie" | "curies" | "ci" => curie(),
            "roentgen" | "roentgens" => roentgen(),
            "katal" | "katals" | "kat" => katal(),
            // Photometry.
            "lumen" | "lumens" | "lm" => lumen(),
            "lux" | "lx" => lux(),
            "phot" | "phots" => phot(),
            "foot candle" | "foot-candle" | "footcandle" | "fc" => foot_candle(),
            "nit" | "nits" => nit(),
            // Electromagnetism.
            "siemens" => siemens(),
            "farad" | "farads" => farad(),
            "microfarad" | "microfarads" | "µf" | "uf" => microfarad(),
            "nanofarad" | "nanofarads" | "nf" => nanofarad(),
            "picofarad" | "picofarads" | "pf" => picofarad(),
            "weber" | "webers" | "wb" => weber(),
            "maxwell" | "maxwells" | "mx" => maxwell(),
            "henry" | "henries" => henry(),
            "tesla" | "teslas" => tesla(),
            "gauss" => gauss(),
            // Viscosity.
            "pascal second" | "pascal-second" | "pa·s" | "pas" => pascal_second(),
            "poise" => poise(),
            "centipoise" | "cp" => centipoise(),
            "stokes" | "stoke" => stokes(),
            "centistokes" | "cst" => centistokes(),
            // Flow & concentration.
            "gallon per minute" | "gpm" => gallon_per_minute(),
            "cubic foot per minute" | "cfm" => cubic_foot_per_minute(),
            "litre per second" | "liter per second" | "l/s" => litre_per_second(),
            "molar" => molar(),
            "millimolar" | "mm/l" => millimolar(),
            // Data rate.
            "bit per second" | "bits per second" | "bps" => bit_per_second(),
            "kilobit per second" | "kbps" => kilobit_per_second(),
            "megabit per second" | "mbps" => megabit_per_second(),
            "gigabit per second" | "gbps" => gigabit_per_second(),
            "byte per second" | "bytes per second" => byte_per_second(),
            // Surface tension.
            "newton per meter" | "newton per metre" | "n/m" => newton_per_metre(),
            // Cooking extras.
            "stick" | "sticks" | "stick of butter" => stick_of_butter(),
            "dash" | "dashes" => dash(),
            "pinch" | "pinches" => pinch(),
            "smidgen" | "smidgens" => smidgen(),
            // Nautical.
            "fathom" | "fathoms" => fathom(),
            "cable" | "cables" => cable(),
            "league" | "leagues" => league(),
            // Astronomical.
            "light second" | "light-second" | "light seconds" => light_second(),
            "light minute" | "light-minute" | "light minutes" => light_minute(),
            // (parsec is irrational vs. metre — intentionally absent, like mach.)
            // Counting & ratio (dimensionless).
            "each" | "ea" | "count" => each(),
            "pair" | "pairs" => pair(),
            "dozen" | "dozens" | "dz" => dozen(),
            "score" | "scores" => score(),
            "gross" => gross(),
            "ream" | "reams" => ream(),
            "percent" | "percents" | "%" => percent(),
            "permille" | "per mille" | "‰" => permille(),
            "ppm" => ppm(),
            "ppb" => ppb(),
            "basis point" | "basis points" => basis_point(),
            // Extra SI prefixes (the ones beyond the common ladder above).
            "megameter" | "megametre" => megametre(),
            "gigameter" | "gigametre" => gigametre(),
            "picometer" | "picometre" | "pm" => picometre(),
            "nanogram" | "nanograms" => nanogram(),
            "gigawatt" | "gigawatts" | "gw" => gigawatt(),
            "terawatt" | "terawatts" | "tw" => terawatt(),
            "terabyte" | "terabytes" | "tb" => terabyte(),
            "petabyte" | "petabytes" | "pb" => petabyte(),
            _ => return None,
        };
        Some(u)
    }
}

/// Physical constants as first-class [`Quantity`] values — each carries its dimension, so they
/// compose with units under the same dimensional algebra (`E = m·c²` type-checks, and `c` can be
/// asked for `in kilometres_per_hour`). The post-2019-SI defining constants are EXACT rationals
/// (the SI fixes their numeric values); measured constants (G) use their CODATA value.
pub mod constants {
    use super::Quantity;
    use crate::dimension::Dimension;
    use crate::numeric::{BigInt, Rational};

    /// `mantissa · 10^pow10` exactly (for large positive magnitudes like Avogadro's number).
    fn ep(mantissa: i64, pow10: u32) -> Rational {
        Rational::from_bigint(BigInt::from_i64(mantissa).mul(&BigInt::from_i64(10).pow(pow10)))
    }
    /// `mantissa · 10^(−pow10)` exactly (for tiny magnitudes like Planck's constant).
    fn en(mantissa: i64, pow10: u32) -> Rational {
        Rational::new(BigInt::from_i64(mantissa), BigInt::from_i64(10).pow(pow10)).unwrap()
    }
    fn r(n: i64, d: i64) -> Rational {
        Rational::new(BigInt::from_i64(n), BigInt::from_i64(d)).unwrap()
    }

    /// The action dimension (energy × time = M·L²·T⁻¹), used by the Planck constant.
    fn action() -> Dimension { Dimension::energy().mul(Dimension::time()) }

    /// Speed of light in vacuum, `c` = 299 792 458 m/s (exact).
    pub fn speed_of_light() -> Quantity { Quantity::si(ep(299_792_458, 0), Dimension::speed()) }
    /// Planck constant, `h` = 6.626 070 15 × 10⁻³⁴ J·s (exact).
    pub fn planck_constant() -> Quantity { Quantity::si(en(662_607_015, 42), action()) }
    /// Elementary charge, `e` = 1.602 176 634 × 10⁻¹⁹ C (exact).
    pub fn elementary_charge() -> Quantity { Quantity::si(en(1_602_176_634, 28), Dimension::charge()) }
    /// Boltzmann constant, `k_B` = 1.380 649 × 10⁻²³ J/K (exact).
    pub fn boltzmann_constant() -> Quantity {
        Quantity::si(en(1_380_649, 29), Dimension::energy().div(Dimension::temperature()))
    }
    /// Avogadro constant, `N_A` = 6.022 140 76 × 10²³ mol⁻¹ (exact).
    pub fn avogadro_constant() -> Quantity {
        Quantity::si(ep(602_214_076, 15), Dimension::amount().recip())
    }
    /// Molar gas constant, `R = N_A · k_B` = 8.314 462 618… J/(mol·K) (exact, a product of exacts).
    pub fn molar_gas_constant() -> Quantity {
        avogadro_constant().mul(&boltzmann_constant())
    }
    /// Newtonian gravitational constant, `G` = 6.674 30 × 10⁻¹¹ m³·kg⁻¹·s⁻² (CODATA measured value).
    pub fn gravitational_constant() -> Quantity {
        let dim = Dimension::volume().div(Dimension::mass()).div(Dimension::time().powi(2));
        Quantity::si(en(667_430, 16), dim)
    }
    /// Standard gravity, `g₀` = 9.806 65 m/s² (exact by definition).
    pub fn standard_gravity() -> Quantity { Quantity::si(r(980_665, 100_000), Dimension::acceleration()) }
    /// Standard atmosphere, `atm` = 101 325 Pa (exact by definition).
    pub fn standard_atmosphere() -> Quantity { Quantity::si(ep(101_325, 0), Dimension::pressure()) }
}

/// The **light-travel time** across a distance — `distance / c`, exact. Returns a Time quantity, or
/// `None` if `distance` is not a length (the dimensional guard). This is the core of "universal /
/// space-travel time": the delay before an event at distance `d` is observed elsewhere (Earth↔Sun
/// ≈ 499 s, Earth↔Mars ≈ 3–22 min). The result rides the exact rational tower, so it never drifts.
pub fn light_travel_time(distance: &Quantity) -> Option<Quantity> {
    if distance.dimension() != Dimension::length() {
        return None;
    }
    distance.div(&constants::speed_of_light())
}

/// Convert a **Time**-dimensioned quantity to a whole number of nanoseconds (rounded to the nearest),
/// the bridge from the exact `Quantity` world to the instant model. `None` if `q` is not a time, or
/// the result does not fit `i64`.
pub fn time_quantity_to_nanos(q: &Quantity) -> Option<i64> {
    if q.dimension() != Dimension::time() {
        return None;
    }
    // magnitude_si is in seconds; ×10⁹ → nanoseconds (still exact), rounded to the nearest whole.
    q.magnitude_si().mul(&Rational::from_i64(1_000_000_000)).round().to_i64()
}

/// The instant (nanoseconds since the epoch) at which an event happening at `event_nanos` is
/// **observed** by someone a distance away — `event + distance/c`. The relativistic light-delay the
/// space-travel time model is built on. `None` if `distance` is not a length or the result overflows.
pub fn observed_arrival_nanos(event_nanos: i64, distance: &Quantity) -> Option<i64> {
    let delay = time_quantity_to_nanos(&light_travel_time(distance)?)?;
    event_nanos.checked_add(delay)
}

/// A point in 3-D space, coordinates in **metres** (an inertial frame's axes). Euclidean distance
/// is transcendental (a square root), so positions are `f64` — the one place the time tower leaves
/// the exact-rational world, because the geometry of spacetime demands it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Position {
    /// The straight-line distance to another point, in metres.
    pub fn distance_to(self, other: Position) -> f64 {
        let (dx, dy, dz) = (self.x - other.x, self.y - other.y, self.z - other.z);
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

/// A **space-aware timestamp** — *when* (a SmoothUTC instant, nanoseconds) AND *where* (a position
/// in space). This is the heart of the light-cone model: two events can only be causally ordered if
/// their time separation is at least the light-travel time between their *positions*. Closer in time
/// than that and they are **spacelike-separated** — genuinely concurrent, no signal could connect
/// them, which is exactly where CRDTs (conflict-free, order-free merge) are the right tool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpacetimeStamp {
    pub instant_nanos: i64,
    pub position: Position,
}

/// How two [`SpacetimeStamp`]s relate causally. `Before`: `self` is in the past light cone of the
/// other (a signal from `self` could reach it). `After`: the reverse. `Concurrent`: spacelike —
/// neither can have affected the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CausalRelation {
    Before,
    After,
    Concurrent,
}

impl SpacetimeStamp {
    /// The light-travel time (nanoseconds) between the two events' positions — the minimum time
    /// separation for a causal link. Symmetric.
    pub fn light_separation_nanos(self, other: SpacetimeStamp) -> i64 {
        let metres = self.position.distance_to(other.position);
        (metres / 299_792_458.0 * 1e9).round() as i64
    }

    /// The causal relation to `other`, decided by the light cone: causally ordered only when the
    /// time gap covers the spatial (light) gap; otherwise spacelike-concurrent.
    pub fn causal_relation(self, other: SpacetimeStamp) -> CausalRelation {
        // i128 so a far-apart pair of i64 instants can't overflow the difference.
        let dt = other.instant_nanos as i128 - self.instant_nanos as i128;
        let light = self.light_separation_nanos(other) as i128;
        if dt >= light {
            CausalRelation::Before // `other` is in self's forward light cone
        } else if -dt >= light {
            CausalRelation::After // self is in other's forward light cone
        } else {
            CausalRelation::Concurrent // spacelike — |dt| < light, no signal connects them
        }
    }

    /// The instant this event is **observed** from `observer` (its own time + the light delay across
    /// the distance) — the space-aware generalisation of [`observed_arrival_nanos`].
    pub fn observed_at_nanos(self, observer: Position) -> i64 {
        let other = SpacetimeStamp { instant_nanos: self.instant_nanos, position: observer };
        self.instant_nanos + self.light_separation_nanos(other)
    }
}

/// The special-relativistic **Lorentz factor** `γ = 1/√(1 − β²)` for a speed `β` as a fraction of
/// `c`. `None` for `|β| ≥ 1` (or NaN) — nothing reaches or exceeds light speed. Inherently a float
/// (relativity is transcendental), unlike the exact-rational quantity arithmetic.
pub fn lorentz_factor(beta: f64) -> Option<f64> {
    // `!(< 1.0)` also rejects NaN.
    if !(beta.abs() < 1.0) {
        return None;
    }
    Some(1.0 / (1.0 - beta * beta).sqrt())
}

/// The **proper time** (in seconds) a clock experiences over `coordinate_seconds` of coordinate
/// time while moving at speed `β` (fraction of `c`): `coordinate / γ`. A moving clock ticks slow —
/// this is the space-traveller's "time attenuation". `None` for `|β| ≥ 1`.
pub fn proper_time_seconds(coordinate_seconds: f64, beta: f64) -> Option<f64> {
    Some(coordinate_seconds / lorentz_factor(beta)?)
}

/// The time-dilation factor `γ` for a velocity given as a **speed [`Quantity`]** (β derived as
/// `v/c`). `None` if `velocity` is not a speed or is ≥ `c`.
pub fn time_dilation_factor(velocity: &Quantity) -> Option<f64> {
    if velocity.dimension() != Dimension::speed() {
        return None;
    }
    let c = constants::speed_of_light();
    let beta = velocity.magnitude_si().to_f64() / c.magnitude_si().to_f64();
    lorentz_factor(beta)
}

#[cfg(test)]
mod tests {
    use super::units::*;
    use super::*;

    fn r(n: i64, d: i64) -> Rational {
        Rational::from_ratio_i64(n, d).unwrap()
    }
    fn i(n: i64) -> Rational {
        Rational::from_i64(n)
    }

    /// THE GOLDEN PROOF: `2 inches + 5 centimetres in feet = 42/127`, exactly — the value that
    /// motivated this whole type system. Also `= 63/625 m`. No floating-point anywhere.
    #[test]
    fn golden_two_inches_plus_five_cm_in_feet_is_exactly_42_over_127() {
        let a = Quantity::of(i(2), &inch());
        let b = Quantity::of(i(5), &centimetre());
        let sum = a.add(&b).expect("both are Length");
        assert_eq!(sum.dimension(), Dimension::length());
        assert_eq!(sum.in_unit(&metre()).unwrap(), r(63, 625), "= 63/625 m exactly");
        assert_eq!(sum.in_unit(&foot()).unwrap(), r(42, 127), "= 42/127 ft exactly");
        // The lossy float answer a JSON-number language would give is NOT 42/127.
        assert_ne!((42.0_f64 / 127.0) as f32 as f64, 42.0 / 127.0);
    }

    #[test]
    fn exact_unit_conversions_within_a_dimension() {
        assert_eq!(Quantity::of(i(1), &mile()).in_unit(&foot()).unwrap(), i(5280));
        assert_eq!(Quantity::of(i(1), &foot()).in_unit(&inch()).unwrap(), i(12));
        assert_eq!(Quantity::of(i(1), &yard()).in_unit(&foot()).unwrap(), i(3));
        assert_eq!(Quantity::of(i(1), &kilometre()).in_unit(&metre()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &kilogram()).in_unit(&gram()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &pound()).in_unit(&ounce()).unwrap(), i(16));
        assert_eq!(Quantity::of(i(1), &hour()).in_unit(&minute()).unwrap(), i(60));
        // 1 pound = 453.59237 g exactly.
        assert_eq!(Quantity::of(i(1), &pound()).in_unit(&gram()).unwrap(), r(45_359_237, 100_000));
    }

    #[test]
    fn affine_temperature_conversion_is_offset_aware_and_exact() {
        // 0 °C = 273.15 K; 20 °C = 68 °F; 100 °C = 212 °F; −40 °C = −40 °F (the crossover).
        assert_eq!(Quantity::of(i(0), &celsius()).in_unit(&kelvin()).unwrap(), r(5463, 20));
        assert_eq!(Quantity::of(i(20), &celsius()).in_unit(&fahrenheit()).unwrap(), i(68));
        assert_eq!(Quantity::of(i(100), &celsius()).in_unit(&fahrenheit()).unwrap(), i(212));
        assert_eq!(Quantity::of(i(-40), &celsius()).in_unit(&fahrenheit()).unwrap(), i(-40));
        assert_eq!(Quantity::of(i(212), &fahrenheit()).in_unit(&celsius()).unwrap(), i(100));
    }

    #[test]
    fn cross_dimension_operations_are_forbidden() {
        let length = Quantity::of(i(1), &metre());
        let mass = Quantity::of(i(1), &kilogram());
        // Can't add Length to Mass; can't convert Length to a Mass unit.
        assert!(length.add(&mass).is_none(), "Length + Mass is forbidden");
        assert!(length.sub(&mass).is_none(), "Length − Mass is forbidden");
        assert!(length.in_unit(&kilogram()).is_none(), "Length → Mass cast is forbidden");
    }

    #[test]
    fn multiplication_and_division_combine_dimensions() {
        // Length × Length = Area.
        let area = Quantity::of(i(3), &metre()).mul(&Quantity::of(i(4), &metre()));
        assert_eq!(area.dimension(), Dimension::area());
        assert_eq!(area.magnitude_si(), &i(12)); // 12 m²
        // Length ÷ Time = Speed.
        let speed = Quantity::of(i(100), &metre()).div(&Quantity::of(i(10), &second())).unwrap();
        assert_eq!(speed.dimension(), Dimension::speed());
        assert_eq!(speed.magnitude_si(), &i(10)); // 10 m/s
        // Area ÷ Length = Length.
        let back = area.div(&Quantity::of(i(3), &metre())).unwrap();
        assert_eq!(back.dimension(), Dimension::length());
        // Scaling by a dimensionless quantity keeps the dimension.
        let scaled = Quantity::of(i(2), &metre()).mul(&Quantity::scalar(i(3)));
        assert_eq!(scaled.dimension(), Dimension::length());
        assert_eq!(scaled.in_unit(&metre()).unwrap(), i(6));
    }

    #[test]
    fn dimensional_algebra_gauntlet_produces_correct_derived_dimensions_and_magnitudes() {
        // Every combination below checks BOTH the resulting dimension AND the exact SI magnitude, so
        // the algebra can never silently produce the right unit with the wrong number (or vice versa).
        // mass × acceleration = force.
        let accel = Quantity::of(i(3), &metre_per_second()).div(&Quantity::of(i(1), &second())).unwrap();
        let force = Quantity::of(i(2), &kilogram()).mul(&accel);
        assert_eq!(force.dimension(), Dimension::force());
        assert_eq!(force.magnitude_si(), &i(6)); // 6 N
        // force × length = energy.
        let energy = force.mul(&Quantity::of(i(4), &metre()));
        assert_eq!(energy.dimension(), Dimension::energy());
        assert_eq!(energy.magnitude_si(), &i(24)); // 24 J
        // energy ÷ time = power.
        let power = energy.div(&Quantity::of(i(2), &second())).unwrap();
        assert_eq!(power.dimension(), Dimension::power());
        assert_eq!(power.magnitude_si(), &i(12)); // 12 W
        // force ÷ area = pressure.
        let pressure = force.div(&Quantity::of(i(2), &square_metre())).unwrap();
        assert_eq!(pressure.dimension(), Dimension::pressure());
        assert_eq!(pressure.magnitude_si(), &i(3)); // 3 Pa
        // current × time = charge.
        let charge = Quantity::of(i(5), &ampere()).mul(&Quantity::of(i(2), &second()));
        assert_eq!(charge.dimension(), Dimension::charge());
        assert_eq!(charge.magnitude_si(), &i(10)); // 10 C
        // information ÷ time = data rate.
        let rate = Quantity::of(i(16), &bit()).div(&Quantity::of(i(2), &second())).unwrap();
        assert_eq!(rate.dimension(), Dimension::data_rate());
        assert_eq!(rate.in_unit(&bit_per_second()).unwrap(), i(8)); // 8 bit/s
        // volume ÷ time = volumetric flow.
        let flow = Quantity::of(i(6), &cubic_metre()).div(&Quantity::of(i(2), &second())).unwrap();
        assert_eq!(flow.dimension(), Dimension::volumetric_flow());
        assert_eq!(flow.in_unit(&cubic_metre_per_second()).unwrap(), i(3));
        // amount ÷ volume = molar concentration (2 mol/m³ = 2 mM).
        let conc = Quantity::of(i(2), &mole()).div(&Quantity::of(i(1), &cubic_metre())).unwrap();
        assert_eq!(conc.dimension(), Dimension::molar_concentration());
        assert_eq!(conc.in_unit(&millimolar()).unwrap(), i(2));
        // area ÷ time = kinematic viscosity.
        let kv = Quantity::of(i(4), &square_metre()).div(&Quantity::of(i(2), &second())).unwrap();
        assert_eq!(kv.dimension(), Dimension::kinematic_viscosity());
        assert_eq!(kv.in_unit(&square_metre_per_second()).unwrap(), i(2));
        // pressure × time = dynamic viscosity.
        let dv = Quantity::of(i(4), &pascal()).mul(&Quantity::of(i(2), &second()));
        assert_eq!(dv.dimension(), Dimension::dynamic_viscosity());
        assert_eq!(dv.in_unit(&pascal_second()).unwrap(), i(8));
        // length ÷ volume = fuel economy (100 km / 10 L = 10 km/L).
        let fe = Quantity::of(i(100), &kilometre()).div(&Quantity::of(i(10), &litre())).unwrap();
        assert_eq!(fe.dimension(), Dimension::fuel_economy());
        assert_eq!(fe.in_unit(&km_per_litre()).unwrap(), i(10));
        // dynamic viscosity ÷ density = kinematic viscosity (μ/ρ = ν), exact magnitude.
        let mu = Quantity::of(i(6), &pascal_second());
        let rho = Quantity::of(i(2), &kilogram()).div(&Quantity::of(i(1), &cubic_metre())).unwrap();
        let nu = mu.div(&rho).unwrap();
        assert_eq!(nu.dimension(), Dimension::kinematic_viscosity());
        assert_eq!(nu.in_unit(&square_metre_per_second()).unwrap(), i(3)); // 6 / 2 = 3 m²/s
    }

    /// A tiny deterministic RNG (SplitMix64) for the conversion-round-trip gauntlet.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
    }

    #[test]
    fn conversion_round_trips_and_add_commutes_under_fuzz() {
        // Property fuzz: for any value and any two units of the same dimension, converting in and
        // back out is the identity, and addition commutes — all exact.
        let length_units = [metre(), kilometre(), centimetre(), inch(), foot(), yard(), mile()];
        let mass_units = [kilogram(), gram(), pound(), ounce()];
        let temp_units = [kelvin(), celsius(), fahrenheit()];
        let mut rng = Rng(0x_FACE_C0DE_1234_5678);
        for _ in 0..2000 {
            let n = (rng.next() % 4001) as i64 - 2000;
            let d = ((rng.next() % 99) as i64) + 1;
            let value = r(n, d);
            for set in [&length_units[..], &mass_units[..], &temp_units[..]] {
                let u1 = &set[(rng.next() as usize) % set.len()];
                let u2 = &set[(rng.next() as usize) % set.len()];
                let q = Quantity::of(value.clone(), u1);
                // Round-trip: value → SI → value (in the same unit) is exact identity.
                assert_eq!(q.in_unit(u1).unwrap(), value, "round-trip in {}", u1.symbol);
                // Re-expressing in u2 then back to u1 is also identity.
                let via = Quantity::of(q.in_unit(u2).unwrap(), u2);
                assert_eq!(via.in_unit(u1).unwrap(), value, "{} → {} → {}", u1.symbol, u2.symbol, u1.symbol);
                // Same-dimension addition commutes (magnitudes, so always for linear; affine too
                // since it is magnitude addition in SI base).
                let q2 = Quantity::of(value.clone(), u2);
                assert_eq!(q.add(&q2), q2.add(&q), "add commutes");
            }
        }
    }

    #[test]
    fn whole_catalog_round_trips_through_every_sibling_under_fuzz() {
        // Extends the round-trip property to EVERY dimension group in the catalog (not just the
        // length/mass/temp trio above): for random rational values, value → unit → SI → sibling →
        // SI → original-unit is exact identity, and same-dimension conversion is always total.
        let groups = catalog();
        let mut rng = Rng(0x_C0FF_EE_D0_0D_F0_0D);
        for _ in 0..400 {
            let n = (rng.next() % 20_001) as i64 - 10_000;
            let d = ((rng.next() % 199) as i64) + 1;
            let value = r(n, d);
            for group in &groups {
                let u1 = &group[(rng.next() as usize) % group.len()];
                let u2 = &group[(rng.next() as usize) % group.len()];
                let q = Quantity::of(value.clone(), u1);
                let through = q.in_unit(u2).expect("same-dimension conversion is total");
                let back = Quantity::of(through, u2).in_unit(u1).unwrap();
                assert_eq!(back, value, "{} → {} → {}", u1.symbol, u2.symbol, u1.symbol);
            }
        }
    }

    #[test]
    fn affine_temperature_fixed_points_are_exact() {
        // The canonical physics anchors, all exact in the rational tower.
        // Water freezes: 0 °C = 273.15 K = 32 °F = 491.67 °R = 0 °Ré.
        assert_eq!(Quantity::of(i(0), &celsius()).in_unit(&kelvin()).unwrap(), r(27315, 100));
        assert_eq!(Quantity::of(i(0), &celsius()).in_unit(&fahrenheit()).unwrap(), i(32));
        assert_eq!(Quantity::of(i(0), &celsius()).in_unit(&rankine()).unwrap(), r(49167, 100));
        assert_eq!(Quantity::of(i(0), &celsius()).in_unit(&reaumur()).unwrap(), i(0));
        // Water boils: 100 °C = 212 °F = 373.15 K = 80 °Ré.
        assert_eq!(Quantity::of(i(100), &celsius()).in_unit(&fahrenheit()).unwrap(), i(212));
        assert_eq!(Quantity::of(i(100), &celsius()).in_unit(&kelvin()).unwrap(), r(37315, 100));
        assert_eq!(Quantity::of(i(100), &celsius()).in_unit(&reaumur()).unwrap(), i(80));
        // The famous crossover: −40 °C = −40 °F exactly.
        assert_eq!(Quantity::of(i(-40), &celsius()).in_unit(&fahrenheit()).unwrap(), i(-40));
        // Absolute zero: 0 K = −273.15 °C = −459.67 °F = 0 °R.
        assert_eq!(Quantity::of(i(0), &kelvin()).in_unit(&celsius()).unwrap(), r(-27315, 100));
        assert_eq!(Quantity::of(i(0), &kelvin()).in_unit(&fahrenheit()).unwrap(), r(-45967, 100));
        assert_eq!(Quantity::of(i(0), &kelvin()).in_unit(&rankine()).unwrap(), i(0));
        // Body temperature, a non-integer anchor: 37 °C = 98.6 °F.
        assert_eq!(Quantity::of(i(37), &celsius()).in_unit(&fahrenheit()).unwrap(), r(986, 10));
        // Round-trip through every affine scale is identity for an arbitrary fractional reading.
        let v = r(37, 7);
        for u in [kelvin(), celsius(), fahrenheit(), rankine(), reaumur()] {
            assert_eq!(Quantity::of(v.clone(), &u).in_unit(&u).unwrap(), v, "round-trip {}", u.symbol);
        }
    }

    #[test]
    fn cooking_measurements_convert_exactly() {
        // The US kitchen ladder — all exact, all anchored to the 231-in³ gallon.
        assert_eq!(Quantity::of(i(1), &us_cup()).in_unit(&tablespoon()).unwrap(), i(16), "1 cup = 16 tbsp");
        assert_eq!(Quantity::of(i(1), &us_cup()).in_unit(&teaspoon()).unwrap(), i(48), "1 cup = 48 tsp");
        assert_eq!(Quantity::of(i(1), &us_cup()).in_unit(&us_fluid_ounce()).unwrap(), i(8), "1 cup = 8 fl oz");
        assert_eq!(Quantity::of(i(1), &tablespoon()).in_unit(&teaspoon()).unwrap(), i(3), "1 tbsp = 3 tsp");
        assert_eq!(Quantity::of(i(1), &us_fluid_ounce()).in_unit(&tablespoon()).unwrap(), i(2), "1 fl oz = 2 tbsp");
        assert_eq!(Quantity::of(i(1), &us_pint()).in_unit(&us_cup()).unwrap(), i(2), "1 pint = 2 cups");
        assert_eq!(Quantity::of(i(1), &us_quart()).in_unit(&us_cup()).unwrap(), i(4), "1 quart = 4 cups");
        assert_eq!(Quantity::of(i(1), &us_gallon()).in_unit(&us_cup()).unwrap(), i(16), "1 gallon = 16 cups");
        assert_eq!(Quantity::of(i(1), &us_gallon()).in_unit(&teaspoon()).unwrap(), i(768), "1 gallon = 768 tsp");
        assert_eq!(Quantity::of(i(1), &oil_barrel()).in_unit(&us_gallon()).unwrap(), i(42), "1 oil barrel = 42 gal");
        // Cross to metric, exact: 1 US gallon = 3.785411784 L; 1 tsp ≈ 4.92892159375 mL (exact rational).
        assert_eq!(Quantity::of(i(1), &us_gallon()).in_unit(&litre()).unwrap(), r(473_176_473, 125_000_000));
        // A real recipe: 3 teaspoons + 1 tablespoon = 2 tablespoons, exactly.
        let mixed = Quantity::of(i(3), &teaspoon()).add(&Quantity::of(i(1), &tablespoon())).unwrap();
        assert_eq!(mixed.in_unit(&tablespoon()).unwrap(), i(2));
    }

    #[test]
    fn general_cross_unit_conversions_are_exact() {
        // Volume / cubic.
        assert_eq!(Quantity::of(i(1), &cubic_metre()).in_unit(&litre()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &cubic_foot()).in_unit(&cubic_inch()).unwrap(), i(1728));
        assert_eq!(Quantity::of(i(1), &litre()).in_unit(&millilitre()).unwrap(), i(1000));
        // Area.
        assert_eq!(Quantity::of(i(1), &acre()).in_unit(&square_foot()).unwrap(), i(43_560));
        assert_eq!(Quantity::of(i(1), &hectare()).in_unit(&square_metre()).unwrap(), i(10_000));
        assert_eq!(Quantity::of(i(1), &square_foot()).in_unit(&square_inch()).unwrap(), i(144));
        // Speed, time, frequency.
        assert_eq!(Quantity::of(i(1), &kilometre_per_hour()).in_unit(&metre_per_second()).unwrap(), r(5, 18));
        assert_eq!(Quantity::of(i(1), &day()).in_unit(&hour()).unwrap(), i(24));
        assert_eq!(Quantity::of(i(1), &fortnight()).in_unit(&day()).unwrap(), i(14));
        assert_eq!(Quantity::of(i(1), &gigahertz()).in_unit(&hertz()).unwrap(), i(1_000_000_000));
        // Energy / power / pressure.
        assert_eq!(Quantity::of(i(1), &kilocalorie()).in_unit(&calorie()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &kilowatt_hour()).in_unit(&joule()).unwrap(), i(3_600_000));
        assert_eq!(Quantity::of(i(1), &calorie()).in_unit(&joule()).unwrap(), r(523, 125)); // 4.184 J
        assert_eq!(Quantity::of(i(1), &atmosphere()).in_unit(&pascal()).unwrap(), i(101_325));
        // Information (decimal vs binary prefixes).
        assert_eq!(Quantity::of(i(1), &byte()).in_unit(&bit()).unwrap(), i(8));
        assert_eq!(Quantity::of(i(1), &kibibyte()).in_unit(&byte()).unwrap(), i(1024));
        assert_eq!(Quantity::of(i(1), &mebibyte()).in_unit(&kibibyte()).unwrap(), i(1024));
        assert_eq!(Quantity::of(i(1), &megabyte()).in_unit(&byte()).unwrap(), i(1_000_000));
        // Angle.
        assert_eq!(Quantity::of(i(1), &turn()).in_unit(&degree()).unwrap(), i(360));
        assert_eq!(Quantity::of(i(1), &degree()).in_unit(&arcminute()).unwrap(), i(60));
        assert_eq!(Quantity::of(i(1), &arcminute()).in_unit(&arcsecond()).unwrap(), i(60));
        // Charge.
        assert_eq!(Quantity::of(i(1), &ampere_hour()).in_unit(&coulomb()).unwrap(), i(3600));
    }

    #[test]
    fn extended_units_cooking_nautical_astronomical_radiation_illumination_convert_exactly() {
        // --- Cooking extras ---
        assert_eq!(Quantity::of(i(1), &teaspoon()).in_unit(&dash()).unwrap(), i(8), "1 tsp = 8 dashes");
        assert_eq!(Quantity::of(i(1), &teaspoon()).in_unit(&pinch()).unwrap(), i(16), "1 tsp = 16 pinches");
        assert_eq!(Quantity::of(i(1), &teaspoon()).in_unit(&smidgen()).unwrap(), i(32), "1 tsp = 32 smidgens");
        assert_eq!(Quantity::of(i(1), &dash()).in_unit(&pinch()).unwrap(), i(2), "1 dash = 2 pinches");
        assert_eq!(Quantity::of(i(1), &stick_of_butter()).in_unit(&tablespoon()).unwrap(), i(8), "1 stick = 8 tbsp");
        assert_eq!(Quantity::of(i(1), &stick_of_butter()).in_unit(&us_cup()).unwrap(), r(1, 2), "1 stick = ½ cup");
        // --- US dry ---
        assert_eq!(Quantity::of(i(1), &bushel()).in_unit(&peck()).unwrap(), i(4), "1 bushel = 4 pecks");
        assert_eq!(Quantity::of(i(1), &bushel()).in_unit(&dry_gallon()).unwrap(), i(8));
        assert_eq!(Quantity::of(i(1), &dry_gallon()).in_unit(&dry_quart()).unwrap(), i(4));
        // --- Nautical ---
        assert_eq!(Quantity::of(i(1), &nautical_mile()).in_unit(&cable()).unwrap(), i(10), "1 nmi = 10 cables");
        assert_eq!(Quantity::of(i(1), &nautical_league()).in_unit(&nautical_mile()).unwrap(), i(3));
        assert_eq!(Quantity::of(i(1), &league()).in_unit(&mile()).unwrap(), i(3), "1 league = 3 miles");
        // --- Astronomical (the light-time ladder is exact; AU/ly are defined) ---
        assert_eq!(Quantity::of(i(1), &light_minute()).in_unit(&light_second()).unwrap(), i(60));
        assert_eq!(Quantity::of(i(1), &light_hour()).in_unit(&light_minute()).unwrap(), i(60));
        assert_eq!(Quantity::of(i(1), &light_day()).in_unit(&light_hour()).unwrap(), i(24));
        assert_eq!(Quantity::of(i(1), &light_second()).in_unit(&metre()).unwrap(), i(299_792_458));
        // --- Radiation ---
        assert_eq!(Quantity::of(i(1), &gray()).in_unit(&rad_unit()).unwrap(), i(100), "1 Gy = 100 rad");
        assert_eq!(Quantity::of(i(1), &sievert()).in_unit(&rem()).unwrap(), i(100), "1 Sv = 100 rem");
        assert_eq!(Quantity::of(i(1), &curie()).in_unit(&becquerel()).unwrap(), i(37_000_000_000), "1 Ci = 3.7e10 Bq");
        assert_eq!(gray().dimension, Dimension::absorbed_dose());
        assert_eq!(sievert().dimension, Dimension::absorbed_dose()); // Gy and Sv share a dimension
        assert_eq!(becquerel().dimension, Dimension::radioactivity());
        assert_eq!(roentgen().dimension, Dimension::exposure());
        assert_eq!(katal().dimension, Dimension::catalytic_activity());
        // --- Illumination (photometry) ---
        assert_eq!(Quantity::of(i(1), &phot()).in_unit(&lux()).unwrap(), i(10_000), "1 phot = 10000 lux");
        assert_eq!(Quantity::of(i(1), &foot_candle()).in_unit(&lux()).unwrap(), r(1_562_500, 145_161));
        assert_eq!(lumen().dimension, Dimension::luminous_flux());
        assert_eq!(lux().dimension, Dimension::illuminance());
        assert_eq!(nit().dimension, Dimension::luminance());
    }

    #[test]
    fn rate_flow_concentration_viscosity_torque_and_prefixes_convert_exactly() {
        // Data rate.
        assert_eq!(Quantity::of(i(1), &byte_per_second()).in_unit(&bit_per_second()).unwrap(), i(8));
        assert_eq!(Quantity::of(i(1), &gigabit_per_second()).in_unit(&megabit_per_second()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &megabyte_per_second()).in_unit(&bit_per_second()).unwrap(), i(8_000_000));
        // Volumetric flow.
        assert_eq!(Quantity::of(i(1), &cubic_metre_per_second()).in_unit(&litre_per_second()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &litre_per_second()).in_unit(&litre_per_minute()).unwrap(), i(60));
        // Concentration.
        assert_eq!(Quantity::of(i(1), &molar()).in_unit(&millimolar()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &millimolar()).in_unit(&micromolar()).unwrap(), i(1000));
        // Viscosity (dynamic + kinematic).
        assert_eq!(Quantity::of(i(1), &pascal_second()).in_unit(&poise()).unwrap(), i(10));
        assert_eq!(Quantity::of(i(1), &poise()).in_unit(&centipoise()).unwrap(), i(100));
        assert_eq!(Quantity::of(i(1), &square_metre_per_second()).in_unit(&stokes()).unwrap(), i(10_000));
        assert_eq!(Quantity::of(i(1), &stokes()).in_unit(&centistokes()).unwrap(), i(100));
        // Torque shares the energy dimension (N·m ≡ J dimensionally) — convertible to joules.
        assert_eq!(newton_metre().dimension, Dimension::energy());
        assert_eq!(Quantity::of(i(1), &newton_metre()).in_unit(&joule()).unwrap(), i(1));
        // Fuel economy is its own (reciprocal-area) dimension.
        assert_eq!(km_per_litre().dimension, Dimension::fuel_economy());
        assert_eq!(mile_per_gallon().dimension, Dimension::fuel_economy());
        // Extra SI prefixes.
        assert_eq!(Quantity::of(i(1), &gigametre()).in_unit(&megametre()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &megametre()).in_unit(&metre()).unwrap(), i(1_000_000));
        assert_eq!(Quantity::of(i(1), &terawatt()).in_unit(&gigawatt()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &petabyte()).in_unit(&terabyte()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &picometre()).in_unit(&femtometre()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &terajoule()).in_unit(&gigajoule()).unwrap(), i(1000));
    }

    #[test]
    fn electromagnetic_and_surface_tension_units_convert_exactly() {
        // Conductance is dimensionally the reciprocal of resistance (S = Ω⁻¹).
        assert_eq!(siemens().dimension, Dimension::resistance().recip());
        assert_eq!(Quantity::of(i(1), &siemens()).in_unit(&millisiemens()).unwrap(), i(1000));
        // Capacitance ladder.
        assert_eq!(Quantity::of(i(1), &microfarad()).in_unit(&nanofarad()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &nanofarad()).in_unit(&picofarad()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &farad()).in_unit(&picofarad()).unwrap(), i(1_000_000_000_000));
        // Magnetic flux: 1 Wb = 10⁸ maxwell.
        assert_eq!(Quantity::of(i(1), &weber()).in_unit(&maxwell()).unwrap(), i(100_000_000));
        // Inductance ladder.
        assert_eq!(Quantity::of(i(1), &henry()).in_unit(&millihenry()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &millihenry()).in_unit(&microhenry()).unwrap(), i(1000));
        // Magnetic flux density: 1 T = 10⁴ gauss = 10³ mT.
        assert_eq!(Quantity::of(i(1), &tesla()).in_unit(&gauss()).unwrap(), i(10_000));
        assert_eq!(Quantity::of(i(1), &tesla()).in_unit(&millitesla()).unwrap(), i(1000));
        // Surface tension: 1 N/m = 1000 dyn/cm.
        assert_eq!(Quantity::of(i(1), &newton_per_metre()).in_unit(&dyne_per_centimetre()).unwrap(), i(1000));
        // The defining SI relations hold at the dimension level (units can never drift from physics).
        assert_eq!(farad().dimension, Dimension::charge().div(Dimension::voltage()));
        assert_eq!(henry().dimension, Dimension::magnetic_flux().div(Dimension::current()));
        assert_eq!(tesla().dimension, Dimension::magnetic_flux().div(Dimension::area()));
        // Cross-dimension stays a forbidden cast (capacitance ≠ inductance).
        assert!(Quantity::of(i(1), &farad()).in_unit(&henry()).is_none());
    }

    #[test]
    fn dimensionless_counting_and_ratio_units_convert_exactly() {
        // Counting words are exact integer multiples of a single item.
        assert_eq!(Quantity::of(i(1), &dozen()).in_unit(&each()).unwrap(), i(12));
        assert_eq!(Quantity::of(i(1), &gross()).in_unit(&dozen()).unwrap(), i(12)); // 144 = 12 dozen
        assert_eq!(Quantity::of(i(1), &great_gross()).in_unit(&gross()).unwrap(), i(12));
        assert_eq!(Quantity::of(i(1), &score()).in_unit(&each()).unwrap(), i(20));
        assert_eq!(Quantity::of(i(1), &baker_dozen()).in_unit(&each()).unwrap(), i(13));
        assert_eq!(Quantity::of(i(1), &ream()).in_unit(&each()).unwrap(), i(500));
        // Ratios: 1 = 100% = 1000‰ = 10000 bp; ppm/ppb are the metrology fractions.
        assert_eq!(Quantity::of(i(1), &each()).in_unit(&percent()).unwrap(), i(100));
        assert_eq!(Quantity::of(i(1), &each()).in_unit(&permille()).unwrap(), i(1000));
        assert_eq!(Quantity::of(i(1), &each()).in_unit(&basis_point()).unwrap(), i(10_000));
        assert_eq!(Quantity::of(i(1), &percent()).in_unit(&basis_point()).unwrap(), i(100)); // 1% = 100 bp
        assert_eq!(Quantity::of(i(1), &percent()).in_unit(&permille()).unwrap(), i(10));
        assert_eq!(Quantity::of(i(1), &each()).in_unit(&ppm()).unwrap(), i(1_000_000));
        assert_eq!(Quantity::of(i(1), &ppm()).in_unit(&ppb()).unwrap(), i(1000));
        // 1 dozen = 1200% (12 × 100). The dimensionless algebra is exact across counting & ratio.
        assert_eq!(Quantity::of(i(1), &dozen()).in_unit(&percent()).unwrap(), i(1200));
        // Every counting/ratio unit is dimensionless, and converting to a dimensioned unit is forbidden.
        for u in [each(), pair(), dozen(), gross(), percent(), ppm(), basis_point()] {
            assert_eq!(u.dimension, Dimension::DIMENSIONLESS, "{} is dimensionless", u.symbol);
            assert!(Quantity::of(i(1), &u).in_unit(&metre()).is_none(), "{} → m forbidden", u.symbol);
        }
    }

    #[test]
    fn light_travel_time_is_exact_and_dimension_safe() {
        let secs = |q: Quantity| q.in_unit(&second()).unwrap();
        let ltt = |u: Unit| super::light_travel_time(&Quantity::of(i(1), &u)).unwrap();
        // Distances defined in terms of c invert to exact round times.
        assert_eq!(secs(ltt(light_second())), i(1));
        assert_eq!(secs(ltt(light_minute())), i(60));
        assert_eq!(secs(ltt(light_year())), i(31_557_600)); // one Julian year
        // Earth–Sun light time = AU / c: a Time quantity, exact rational ≈ 499 s.
        let sun = ltt(astronomical_unit());
        assert_eq!(sun.dimension(), Dimension::time());
        let sun_secs = secs(sun);
        assert_eq!(sun_secs, r(149_597_870_700, 299_792_458));
        assert!(sun_secs > i(498) && sun_secs < i(500));
        // A non-length distance is the forbidden case (dimensional guard).
        assert!(super::light_travel_time(&Quantity::of(i(5), &kilogram())).is_none());
    }

    #[test]
    fn observed_arrival_adds_light_delay() {
        const NS: i64 = 1_000_000_000;
        // A Time quantity converts to nanoseconds (rounded).
        assert_eq!(super::time_quantity_to_nanos(&Quantity::of(i(1), &second())), Some(NS));
        assert_eq!(super::time_quantity_to_nanos(&Quantity::of(i(1), &millisecond())), Some(1_000_000));
        // A non-time quantity is rejected.
        assert_eq!(super::time_quantity_to_nanos(&Quantity::of(i(1), &metre())), None);
        // An event observed 1 light-minute away arrives exactly 60 s later.
        assert_eq!(
            super::observed_arrival_nanos(0, &Quantity::of(i(1), &light_minute())),
            Some(60 * NS)
        );
        // From a nonzero event time, the delay adds on.
        assert_eq!(
            super::observed_arrival_nanos(5 * NS, &Quantity::of(i(1), &light_second())),
            Some(6 * NS)
        );
        // Earth–Sun: ~499 s of delay (AU / c rounded to the nearest nanosecond).
        let sun = super::observed_arrival_nanos(0, &Quantity::of(i(1), &astronomical_unit())).unwrap();
        assert!(sun > 498 * NS && sun < 500 * NS);
        // It equals the rounded light-travel time exactly.
        let expected = super::time_quantity_to_nanos(&super::light_travel_time(&Quantity::of(i(1), &astronomical_unit())).unwrap());
        assert_eq!(Some(sun), expected);
        // A non-length distance is rejected.
        assert_eq!(super::observed_arrival_nanos(0, &Quantity::of(i(1), &kilogram())), None);
    }

    #[test]
    fn spacetime_stamp_is_space_aware_like_a_timezone_for_position() {
        use super::{CausalRelation, Position, SpacetimeStamp};
        const C: f64 = 299_792_458.0; // metres per second; c·1s = 1 light-second of distance
        let pos = |x: f64| Position { x, y: 0.0, z: 0.0 };
        let at = |ns: i64, x: f64| SpacetimeStamp { instant_nanos: ns, position: pos(x) };
        let here = pos(0.0);
        let one_ls = C; // 1 light-second east

        // Same place → ordered by time alone (zero light separation), like one timezone.
        assert_eq!(at(0, 0.0).causal_relation(at(1_000_000_000, 0.0)), CausalRelation::Before);
        assert_eq!(at(1_000_000_000, 0.0).causal_relation(at(0, 0.0)), CausalRelation::After);
        // 1 light-second apart in space takes ~1 s of light delay (symmetric) — the "offset".
        assert_eq!(at(0, 0.0).light_separation_nanos(at(0, one_ls)), 1_000_000_000);
        // 2 s apart but only 1 light-second away → inside the light cone → causally Before.
        assert_eq!(at(0, 0.0).causal_relation(at(2_000_000_000, one_ls)), CausalRelation::Before);
        // Exactly on the light cone (1 s apart, 1 light-second away) → still causally connected.
        assert_eq!(at(0, 0.0).causal_relation(at(1_000_000_000, one_ls)), CausalRelation::Before);
        // Only 0.5 s apart but 1 light-second away → SPACELIKE: genuinely concurrent (CRDT land).
        assert_eq!(at(0, 0.0).causal_relation(at(500_000_000, one_ls)), CausalRelation::Concurrent);
        // Observed-at: an event here is seen 1 s later from 1 light-second away (the relative read).
        assert_eq!(at(0, 0.0).observed_at_nanos(pos(one_ls)), 1_000_000_000);
        // Euclidean distance is honest 3-D (a 3-4-5 triangle → 5).
        assert!((Position { x: 3.0, y: 4.0, z: 0.0 }.distance_to(here) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn relativistic_time_dilation() {
        let approx = |a: f64, b: f64| (a - b).abs() < 1e-9;
        // No motion → no dilation.
        assert_eq!(super::lorentz_factor(0.0), Some(1.0));
        // Clean betas: √(1−0.36)=0.8 → γ=1.25; √(1−0.64)=0.6 → γ=5/3.
        assert!(approx(super::lorentz_factor(0.6).unwrap(), 1.25));
        assert!(approx(super::lorentz_factor(0.8).unwrap(), 5.0 / 3.0));
        // A clock at 0.6c for 10 coordinate-seconds ages only 8 proper seconds.
        assert!(approx(super::proper_time_seconds(10.0, 0.6).unwrap(), 8.0));
        // Light speed and beyond (and NaN) are unphysical → None.
        assert_eq!(super::lorentz_factor(1.0), None);
        assert_eq!(super::lorentz_factor(1.5), None);
        assert_eq!(super::lorentz_factor(f64::NAN), None);
        // From a speed Quantity: v = (3/5)·c → γ = 1.25.
        let c = super::constants::speed_of_light();
        let v = Quantity::si(c.magnitude_si().mul(&r(3, 5)), Dimension::speed());
        assert!(approx(super::time_dilation_factor(&v).unwrap(), 1.25));
        // A non-speed quantity → None.
        assert_eq!(super::time_dilation_factor(&Quantity::of(i(1), &metre())), None);
    }

    #[test]
    fn physical_constants_carry_correct_dimensions_and_compose() {
        use super::constants::*;
        // c is a speed, exactly 299 792 458 m/s, and exact in km/h (× 18/5).
        let c = speed_of_light();
        assert_eq!(c.dimension(), Dimension::speed());
        assert_eq!(c.in_unit(&metre_per_second()).unwrap(), i(299_792_458));
        assert_eq!(c.in_unit(&kilometre_per_hour()).unwrap(), r(5_396_264_244, 5));
        // E = m·c² type-checks as energy, with exactly 2·c² joules for a 2 kg mass.
        let energy = Quantity::of(i(2), &kilogram()).mul(&c).mul(&c);
        assert_eq!(energy.dimension(), Dimension::energy());
        let c_sq = i(299_792_458).mul(&i(299_792_458));
        assert_eq!(energy.magnitude_si(), &i(2).mul(&c_sq));
        // The gas constant is exactly Avogadro × Boltzmann, dimension J/(mol·K).
        let r_gas = molar_gas_constant();
        assert_eq!(r_gas.dimension(), Dimension::energy().div(Dimension::amount()).div(Dimension::temperature()));
        assert_eq!(r_gas.magnitude_si(), avogadro_constant().mul(&boltzmann_constant()).magnitude_si());
        // Each constant's dimension is its defining one.
        assert_eq!(planck_constant().dimension(), Dimension::energy().mul(Dimension::time())); // action
        assert_eq!(elementary_charge().dimension(), Dimension::charge());
        assert_eq!(boltzmann_constant().dimension(), Dimension::energy().div(Dimension::temperature()));
        assert_eq!(avogadro_constant().dimension(), Dimension::amount().recip());
        assert_eq!(gravitational_constant().dimension(),
                   Dimension::volume().div(Dimension::mass()).div(Dimension::time().powi(2)));
        assert_eq!(standard_gravity().dimension(), Dimension::acceleration());
        // Exact defining values: atm = 101 325 Pa, g₀ = 9.806 65 m/s².
        assert_eq!(standard_atmosphere().in_unit(&pascal()).unwrap(), i(101_325));
        assert_eq!(standard_gravity().in_unit(&metre_per_second()).is_none(), true); // m/s ≠ m/s²
        // The ideal-gas law n·R·T/V has dimension pressure (cross-constant physics composes).
        let n = Quantity::of(i(1), &mole());
        let t = Quantity::of(i(300), &kelvin());
        let v = Quantity::of(i(1), &cubic_metre());
        let p = n.mul(&r_gas).mul(&t).div(&v).unwrap();
        assert_eq!(p.dimension(), Dimension::pressure());
    }

    #[test]
    fn in_best_unit_auto_scales_to_the_most_human_unit() {
        let ladder = [millimetre(), centimetre(), metre(), kilometre()];
        // 1500 m → 1.5 km (smallest magnitude that is still ≥ 1).
        let (mag, u) = Quantity::of(i(1500), &metre()).in_best_unit(&ladder).unwrap();
        assert_eq!((u.symbol, mag), ("km", r(3, 2)));
        // 0.5 km → 500 m (km would be 0.5 < 1, so it drops to metres).
        let (mag, u) = Quantity::of(r(1, 2), &kilometre()).in_best_unit(&ladder).unwrap();
        assert_eq!((u.symbol, mag), ("m", i(500)));
        // 0.003 m → 3 mm.
        let (mag, u) = Quantity::of(r(3, 1000), &metre()).in_best_unit(&ladder).unwrap();
        assert_eq!((u.symbol, mag), ("mm", i(3)));
        // Exactly 1 km stays 1 km (magnitude == 1 qualifies).
        let (mag, u) = Quantity::of(i(1), &kilometre()).in_best_unit(&ladder).unwrap();
        assert_eq!((u.symbol, mag), ("km", i(1)));
        // Below the smallest unit → fall back to the smallest unit (largest magnitude).
        let (mag, u) = Quantity::of(r(1, 10), &millimetre()).in_best_unit(&ladder).unwrap();
        assert_eq!((u.symbol, mag), ("mm", r(1, 10)));
        // Negatives scale by absolute magnitude: −2000 m → −2 km.
        let (mag, u) = Quantity::of(i(-2000), &metre()).in_best_unit(&ladder).unwrap();
        assert_eq!((u.symbol, mag), ("km", i(-2)));
        // A ladder with no same-dimension unit yields None (the dimensions never match).
        assert!(Quantity::of(i(1), &metre()).in_best_unit(&[kilogram(), second()]).is_none());
        // The chosen (magnitude, unit) reconstructs the original SI quantity exactly.
        let q = Quantity::of(i(1500), &metre());
        let (mag, u) = q.in_best_unit(&ladder).unwrap();
        assert_eq!(Quantity::of(mag, u).magnitude_si(), q.magnitude_si());
        // Works across other dimensions too: 2_500_000 g → 2.5 t.
        let mass_ladder = [gram(), kilogram(), tonne()];
        let (mag, u) = Quantity::of(i(2_500_000), &gram()).in_best_unit(&mass_ladder).unwrap();
        assert_eq!((u.symbol, mag), ("t", r(5, 2)));
    }

    #[test]
    fn by_name_resolves_units_from_english_names_plurals_and_symbols() {
        // The golden trio resolve — plurals, British spellings, and symbols all map to one unit.
        for alias in ["inch", "inches", "in", "  Inch  "] {
            assert_eq!(by_name(alias), Some(inch()), "{alias}");
        }
        for alias in ["centimeter", "centimetre", "cm"] {
            assert_eq!(by_name(alias), Some(centimetre()), "{alias}");
        }
        for alias in ["foot", "feet", "ft"] {
            assert_eq!(by_name(alias), Some(foot()), "{alias}");
        }
        // Case-insensitive across dimensions.
        assert_eq!(by_name("KM"), Some(kilometre()));
        assert_eq!(by_name("Kilogram"), Some(kilogram()));
        assert_eq!(by_name("celsius"), Some(celsius()));
        assert_eq!(by_name("tablespoon"), Some(tablespoon()));
        assert_eq!(by_name("MPH"), Some(mile_per_hour()));
        // Unknown / empty names are a clean None, never a panic.
        assert_eq!(by_name("zorgles"), None);
        assert_eq!(by_name(""), None);
        // The golden pipeline routed entirely through by_name: 2 inch + 5 cm in feet = 42/127.
        let a = Quantity::of(i(2), &by_name("inches").unwrap());
        let b = Quantity::of(i(5), &by_name("cm").unwrap());
        let sum = a.add(&b).unwrap();
        assert_eq!(sum.in_unit(&by_name("feet").unwrap()).unwrap(), r(42, 127));
    }

    #[test]
    fn by_name_covers_radiation_photometry_em_dimensionless_and_prefixes() {
        // A representative member from each later-added family resolves.
        assert_eq!(by_name("gray"), Some(gray()));
        assert_eq!(by_name("sievert"), Some(sievert()));
        assert_eq!(by_name("becquerel"), Some(becquerel()));
        assert_eq!(by_name("lux"), Some(lux()));
        assert_eq!(by_name("lumen"), Some(lumen()));
        assert_eq!(by_name("tesla"), Some(tesla()));
        assert_eq!(by_name("farad"), Some(farad()));
        assert_eq!(by_name("henry"), Some(henry()));
        assert_eq!(by_name("poise"), Some(poise()));
        assert_eq!(by_name("molar"), Some(molar()));
        assert_eq!(by_name("Mbps"), Some(megabit_per_second()));
        assert_eq!(by_name("dozen"), Some(dozen()));
        assert_eq!(by_name("percent"), Some(percent()));
        assert_eq!(by_name("ppm"), Some(ppm()));
        assert_eq!(by_name("terabyte"), Some(terabyte()));
        assert_eq!(by_name("pinch"), Some(pinch()));
        assert_eq!(by_name("fathom"), Some(fathom()));
        assert_eq!(by_name("light second"), Some(light_second()));
        // Every name by_name returns is a well-formed unit that round-trips exactly.
        for name in ["gray", "tesla", "poise", "dozen", "percent", "terabyte", "pinch", "light second", "lux", "molar"] {
            let u = by_name(name).unwrap();
            let v = r(3, 1);
            assert_eq!(Quantity::of(v.clone(), &u).in_unit(&u).unwrap(), v, "{name}");
        }
    }

    /// The whole catalog, grouped by dimension — the completeness harness iterates it.
    fn catalog() -> Vec<Vec<Unit>> {
        vec![
            vec![metre(), kilometre(), hectometre(), dekametre(), decimetre(), centimetre(), millimetre(),
                 micrometre(), nanometre(), picometre(), femtometre(), megametre(), gigametre(), angstrom(),
                 inch(), foot(), yard(), mile(), nautical_mile(),
                 fathom(), furlong(), chain(), rod(), hand(), point(), pica(), thou(), astronomical_unit(),
                 light_year(), cable(), league(), nautical_league(), light_second(), light_minute(),
                 light_hour(), light_day(), lunar_distance(), solar_radius(), earth_radius()],
            vec![kilogram(), gram(), milligram(), microgram(), nanogram(), tonne(), pound(), ounce(), stone(), short_ton(),
                 long_ton(), hundredweight(), grain(), dram(), carat(), troy_ounce(), troy_pound(),
                 solar_mass(), earth_mass(), jupiter_mass()],
            vec![second(), millisecond(), microsecond(), nanosecond(), picosecond(), minute(), hour(), day(), week(),
                 fortnight(), julian_year(), decade(), century()],
            vec![kelvin(), celsius(), fahrenheit(), rankine(), reaumur()],
            vec![cubic_metre(), litre(), millilitre(), centilitre(), decilitre(), cubic_centimetre(),
                 cubic_inch(), cubic_foot(), us_gallon(), us_quart(), us_pint(), us_cup(), us_gill(),
                 us_fluid_ounce(), tablespoon(), teaspoon(), oil_barrel(), imperial_gallon(), imperial_pint(),
                 imperial_fluid_ounce(), stick_of_butter(), dash(), pinch(), smidgen(), bushel(), peck(),
                 dry_gallon(), dry_quart(), dry_pint()],
            vec![square_metre(), square_kilometre(), square_centimetre(), square_inch(), square_foot(),
                 square_yard(), square_mile(), hectare(), are(), acre()],
            vec![metre_per_second(), kilometre_per_hour(), mile_per_hour(), foot_per_second(), knot()],
            vec![hertz(), kilohertz(), megahertz(), gigahertz(), rpm()],
            vec![newton(), kilonewton(), dyne(), kilogram_force(), pound_force()],
            vec![joule(), kilojoule(), megajoule(), gigajoule(), terajoule(), calorie(), kilocalorie(),
                 watt_hour(), kilowatt_hour(), erg(), electronvolt(), newton_metre(), pound_foot()],
            vec![watt(), kilowatt(), megawatt(), gigawatt(), terawatt(), horsepower()],
            vec![pascal(), kilopascal(), bar(), millibar(), atmosphere(), psi()],
            vec![bit(), byte(), kilobit(), kilobyte(), megabyte(), gigabyte(), terabit(), terabyte(), petabyte(),
                 kibibyte(), mebibyte(), gibibyte()],
            vec![turn(), degree(), gradian(), arcminute(), arcsecond()],
            vec![ampere(), milliampere()],
            vec![coulomb(), ampere_hour(), milliampere_hour()],
            vec![volt(), millivolt(), kilovolt()],
            vec![ohm()],
            vec![mole()],
            vec![candela()],
            // Radiation.
            vec![gray(), sievert(), rad_unit(), rem()], // absorbed/equivalent dose (J/kg)
            vec![becquerel(), curie()],                  // radioactivity (1/s)
            vec![roentgen()],                            // exposure (C/kg)
            vec![katal()],                               // catalytic activity (mol/s)
            // Photometry.
            vec![lumen()],                               // luminous flux (cd·sr)
            vec![lux(), phot(), foot_candle()],          // illuminance (lm/m²)
            vec![nit()],                                 // luminance (cd/m²)
            // Rates, flow, concentration, viscosity, fuel economy.
            vec![bit_per_second(), kilobit_per_second(), megabit_per_second(), gigabit_per_second(),
                 byte_per_second(), megabyte_per_second()],                                   // data rate (bit/s)
            vec![cubic_metre_per_second(), litre_per_second(), litre_per_minute(),
                 gallon_per_minute(), cubic_foot_per_minute()],                               // volumetric flow (m³/s)
            vec![molar(), millimolar(), micromolar()],                                        // molar concentration (mol/m³)
            vec![molal()],                                                                    // molality (mol/kg)
            vec![pascal_second(), poise(), centipoise()],                                     // dynamic viscosity (Pa·s)
            vec![square_metre_per_second(), stokes(), centistokes()],                         // kinematic viscosity (m²/s)
            vec![mile_per_gallon(), km_per_litre()],                                          // fuel economy (m⁻²)
            // Electromagnetism & surface tension.
            vec![siemens(), millisiemens()],                                                  // conductance (S = Ω⁻¹)
            vec![farad(), microfarad(), nanofarad(), picofarad()],                            // capacitance (F)
            vec![weber(), maxwell()],                                                         // magnetic flux (Wb)
            vec![henry(), millihenry(), microhenry()],                                        // inductance (H)
            vec![tesla(), millitesla(), gauss()],                                             // magnetic flux density (T)
            vec![newton_per_metre(), dyne_per_centimetre()],                                  // surface tension (N/m)
            // Counting & ratio (dimensionless).
            vec![each(), pair(), dozen(), baker_dozen(), score(), gross(), great_gross(), ream(),
                 percent(), permille(), ppm(), ppb(), basis_point()],
        ]
    }

    /// COMPLETENESS HARNESS: every unit in the catalog must (1) round-trip `of(v,u).in_unit(u)==v`,
    /// (2) have a positive scale, (3) be inter-convertible with same-dimension units, and (4) refuse
    /// conversion to a different dimension (the forbidden cast). Adding a unit auto-extends coverage.
    #[test]
    fn every_unit_in_the_catalog_is_well_formed_and_dimension_safe() {
        let groups = catalog();
        let v = r(7, 3); // an arbitrary nonzero value
        for group in &groups {
            for u in group {
                // (1) round-trip identity.
                assert_eq!(Quantity::of(v.clone(), u).in_unit(u).unwrap(), v, "round-trip {}", u.symbol);
                // (2) positive scale.
                assert!(!u.scale.is_zero() && !u.scale.is_negative(), "scale > 0 for {}", u.symbol);
                // (3) inter-convertible within the dimension.
                for other in group {
                    assert!(
                        Quantity::of(v.clone(), u).in_unit(other).is_some(),
                        "{} convertible to {}",
                        u.symbol,
                        other.symbol
                    );
                    assert_eq!(u.dimension, other.dimension, "{} and {} share a dimension", u.symbol, other.symbol);
                }
            }
        }
        // (4) cross-dimension conversion is forbidden across every pair of distinct groups.
        for (gi, ga) in groups.iter().enumerate() {
            for (gj, gb) in groups.iter().enumerate() {
                if gi == gj || ga[0].dimension == gb[0].dimension {
                    continue;
                }
                assert!(
                    Quantity::of(v.clone(), &ga[0]).in_unit(&gb[0]).is_none(),
                    "{} → {} must be a forbidden cross-dimension cast",
                    ga[0].symbol,
                    gb[0].symbol
                );
            }
        }
    }
}
