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
        Unit {
            symbol,
            dimension,
            scale,
            offset: Rational::zero(),
        }
    }

    /// An affine unit (`si = value · scale + offset`) — temperatures.
    pub fn affine(
        symbol: &'static str,
        dimension: Dimension,
        scale: Rational,
        offset: Rational,
    ) -> Unit {
        Unit {
            symbol,
            dimension,
            scale,
            offset,
        }
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
        Quantity {
            magnitude: value,
            dimension: Dimension::DIMENSIONLESS,
        }
    }

    /// A quantity given directly by its SI-base magnitude and dimension — used for physical
    /// constants and any value whose dimension has no single named unit (e.g. the gas constant).
    pub fn si(magnitude: Rational, dimension: Dimension) -> Quantity {
        Quantity {
            magnitude,
            dimension,
        }
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
        Some(Quantity {
            magnitude: self.magnitude.add(&other.magnitude),
            dimension: self.dimension,
        })
    }

    /// `self − other` — `None` unless the dimensions match.
    pub fn sub(&self, other: &Quantity) -> Option<Quantity> {
        if self.dimension != other.dimension {
            return None;
        }
        Some(Quantity {
            magnitude: self.magnitude.sub(&other.magnitude),
            dimension: self.dimension,
        })
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
                        (true, true) => abs < best_abs, // both ≥ 1: prefer the one closest to 1 from above
                        (true, false) => true,          // candidate qualifies, incumbent doesn't
                        (false, true) => false,         // keep the qualifying incumbent
                        (false, false) => abs > best_abs, // both < 1: prefer the one closest to 1 from below
                    };
                    if take {
                        (mag, u)
                    } else {
                        (best_mag, best_u)
                    }
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
    fn inch_m() -> Rational {
        r(127, 5000)
    } // 1 inch = 0.0254 m, exactly
    fn foot_m() -> Rational {
        r(381, 1250)
    } // 0.3048 m
    fn lb_kg() -> Rational {
        r(45_359_237, 100_000_000)
    } // 0.45359237 kg
    fn g0() -> Rational {
        r(980_665, 100_000)
    } // standard gravity 9.80665 m/s²
    /// US liquid gallon = 231 in³ (exact), in m³.
    fn us_gallon_m3() -> Rational {
        cube(inch_m()).mul(&i(231))
    }
    /// Imperial gallon = 4.54609 L (exact), in m³.
    fn imp_gallon_m3() -> Rational {
        r(454_609, 100_000_000)
    }

    // ============================ LENGTH (base: metre) ============================
    pub fn metre() -> Unit {
        Unit::linear("m", Dimension::length(), Rational::one())
    }
    pub fn kilometre() -> Unit {
        Unit::linear("km", Dimension::length(), i(1000))
    }
    pub fn hectometre() -> Unit {
        Unit::linear("hm", Dimension::length(), i(100))
    }
    pub fn dekametre() -> Unit {
        Unit::linear("dam", Dimension::length(), i(10))
    }
    pub fn decimetre() -> Unit {
        Unit::linear("dm", Dimension::length(), r(1, 10))
    }
    pub fn centimetre() -> Unit {
        Unit::linear("cm", Dimension::length(), r(1, 100))
    }
    pub fn millimetre() -> Unit {
        Unit::linear("mm", Dimension::length(), r(1, 1000))
    }
    pub fn micrometre() -> Unit {
        Unit::linear("µm", Dimension::length(), r(1, 1_000_000))
    }
    pub fn nanometre() -> Unit {
        Unit::linear("nm", Dimension::length(), r(1, 1_000_000_000))
    }
    pub fn angstrom() -> Unit {
        Unit::linear("Å", Dimension::length(), r(1, 10_000_000_000))
    }
    pub fn inch() -> Unit {
        Unit::linear("in", Dimension::length(), inch_m())
    }
    pub fn foot() -> Unit {
        Unit::linear("ft", Dimension::length(), foot_m())
    }
    pub fn yard() -> Unit {
        Unit::linear("yd", Dimension::length(), r(1143, 1250))
    } // 0.9144 m
    pub fn mile() -> Unit {
        Unit::linear("mi", Dimension::length(), r(201_168, 125))
    } // 1609.344 m
    pub fn nautical_mile() -> Unit {
        Unit::linear("nmi", Dimension::length(), i(1852))
    }
    pub fn fathom() -> Unit {
        Unit::linear("ftm", Dimension::length(), r(1143, 625))
    } // 2 yd
    pub fn furlong() -> Unit {
        Unit::linear("fur", Dimension::length(), r(25_146, 125))
    } // 201.168 m
    pub fn chain() -> Unit {
        Unit::linear("ch", Dimension::length(), r(12_573, 625))
    } // 20.1168 m
    pub fn rod() -> Unit {
        Unit::linear("rod", Dimension::length(), r(12_573, 2500))
    } // 5.0292 m
    pub fn hand() -> Unit {
        Unit::linear("hand", Dimension::length(), r(127, 1250))
    } // 4 in
    pub fn point() -> Unit {
        Unit::linear("pt", Dimension::length(), r(127, 360_000))
    } // 1/72 in
    pub fn pica() -> Unit {
        Unit::linear("pica", Dimension::length(), r(127, 30_000))
    } // 1/6 in
    pub fn thou() -> Unit {
        Unit::linear("thou", Dimension::length(), r(127, 5_000_000))
    } // 1/1000 in
    pub fn astronomical_unit() -> Unit {
        Unit::linear("AU", Dimension::length(), i(149_597_870_700))
    }
    pub fn light_year() -> Unit {
        Unit::linear("ly", Dimension::length(), i(9_460_730_472_580_800))
    } // c·Julian yr

    // ============================ MASS (base: kilogram) ============================
    pub fn kilogram() -> Unit {
        Unit::linear("kg", Dimension::mass(), Rational::one())
    }
    pub fn gram() -> Unit {
        Unit::linear("g", Dimension::mass(), r(1, 1000))
    }
    pub fn milligram() -> Unit {
        Unit::linear("mg", Dimension::mass(), r(1, 1_000_000))
    }
    pub fn microgram() -> Unit {
        Unit::linear("µg", Dimension::mass(), r(1, 1_000_000_000))
    }
    pub fn tonne() -> Unit {
        Unit::linear("t", Dimension::mass(), i(1000))
    }
    pub fn pound() -> Unit {
        Unit::linear("lb", Dimension::mass(), lb_kg())
    }
    pub fn ounce() -> Unit {
        Unit::linear("oz", Dimension::mass(), lb_kg().div(&i(16)).unwrap())
    }
    pub fn stone() -> Unit {
        Unit::linear("st", Dimension::mass(), lb_kg().mul(&i(14)))
    }
    pub fn short_ton() -> Unit {
        Unit::linear("ton", Dimension::mass(), lb_kg().mul(&i(2000)))
    }
    pub fn long_ton() -> Unit {
        Unit::linear("LT", Dimension::mass(), lb_kg().mul(&i(2240)))
    }
    pub fn hundredweight() -> Unit {
        Unit::linear("cwt", Dimension::mass(), lb_kg().mul(&i(100)))
    }
    pub fn grain() -> Unit {
        Unit::linear("gr", Dimension::mass(), lb_kg().div(&i(7000)).unwrap())
    }
    pub fn dram() -> Unit {
        Unit::linear("dr", Dimension::mass(), lb_kg().div(&i(256)).unwrap())
    } // oz/16
    pub fn carat() -> Unit {
        Unit::linear("ct", Dimension::mass(), r(1, 5000))
    } // 0.2 g
    pub fn troy_ounce() -> Unit {
        Unit::linear("ozt", Dimension::mass(), r(311_034_768, 10_000_000_000))
    } // 31.1034768 g
    pub fn troy_pound() -> Unit {
        Unit::linear(
            "lbt",
            Dimension::mass(),
            r(311_034_768, 10_000_000_000).mul(&i(12)),
        )
    }

    // ============================ TIME (base: second) ============================
    pub fn second() -> Unit {
        Unit::linear("s", Dimension::time(), Rational::one())
    }
    pub fn millisecond() -> Unit {
        Unit::linear("ms", Dimension::time(), r(1, 1000))
    }
    pub fn microsecond() -> Unit {
        Unit::linear("µs", Dimension::time(), r(1, 1_000_000))
    }
    pub fn nanosecond() -> Unit {
        Unit::linear("ns", Dimension::time(), r(1, 1_000_000_000))
    }
    pub fn minute() -> Unit {
        Unit::linear("min", Dimension::time(), i(60))
    }
    pub fn hour() -> Unit {
        Unit::linear("h", Dimension::time(), i(3600))
    }
    pub fn day() -> Unit {
        Unit::linear("day", Dimension::time(), i(86_400))
    }
    pub fn week() -> Unit {
        Unit::linear("wk", Dimension::time(), i(604_800))
    }
    pub fn fortnight() -> Unit {
        Unit::linear("fortnight", Dimension::time(), i(1_209_600))
    }
    pub fn julian_year() -> Unit {
        Unit::linear("yr", Dimension::time(), i(31_557_600))
    } // 365.25 d
    pub fn decade() -> Unit {
        Unit::linear("decade", Dimension::time(), i(315_576_000))
    }
    pub fn century() -> Unit {
        Unit::linear("century", Dimension::time(), i(3_155_760_000))
    }

    // ====================== TEMPERATURE (base: kelvin), AFFINE ======================
    pub fn kelvin() -> Unit {
        Unit::affine(
            "K",
            Dimension::temperature(),
            Rational::one(),
            Rational::zero(),
        )
    }
    pub fn celsius() -> Unit {
        Unit::affine("°C", Dimension::temperature(), Rational::one(), r(5463, 20))
    } // +273.15
    pub fn fahrenheit() -> Unit {
        Unit::affine("°F", Dimension::temperature(), r(5, 9), r(45967, 180))
    }
    pub fn rankine() -> Unit {
        Unit::affine("°R", Dimension::temperature(), r(5, 9), Rational::zero())
    } // K = °R·5/9
    pub fn reaumur() -> Unit {
        Unit::affine("°Ré", Dimension::temperature(), r(5, 4), r(5463, 20))
    } // K = °Ré·5/4 + 273.15

    // ============================ VOLUME (base: m³) ============================
    pub fn cubic_metre() -> Unit {
        Unit::linear("m³", Dimension::volume(), Rational::one())
    }
    pub fn litre() -> Unit {
        Unit::linear("L", Dimension::volume(), r(1, 1000))
    }
    pub fn millilitre() -> Unit {
        Unit::linear("mL", Dimension::volume(), r(1, 1_000_000))
    }
    pub fn centilitre() -> Unit {
        Unit::linear("cL", Dimension::volume(), r(1, 100_000))
    }
    pub fn decilitre() -> Unit {
        Unit::linear("dL", Dimension::volume(), r(1, 10_000))
    }
    pub fn cubic_centimetre() -> Unit {
        Unit::linear("cc", Dimension::volume(), r(1, 1_000_000))
    }
    pub fn cubic_inch() -> Unit {
        Unit::linear("in³", Dimension::volume(), cube(inch_m()))
    }
    pub fn cubic_foot() -> Unit {
        Unit::linear("ft³", Dimension::volume(), cube(foot_m()))
    }
    // ---- US liquid / COOKING (anchored to the 231-in³ gallon) ----
    pub fn us_gallon() -> Unit {
        Unit::linear("gal", Dimension::volume(), us_gallon_m3())
    }
    pub fn us_quart() -> Unit {
        Unit::linear(
            "qt",
            Dimension::volume(),
            us_gallon_m3().div(&i(4)).unwrap(),
        )
    }
    pub fn us_pint() -> Unit {
        Unit::linear(
            "pt",
            Dimension::volume(),
            us_gallon_m3().div(&i(8)).unwrap(),
        )
    }
    pub fn us_cup() -> Unit {
        Unit::linear(
            "cup",
            Dimension::volume(),
            us_gallon_m3().div(&i(16)).unwrap(),
        )
    }
    pub fn us_gill() -> Unit {
        Unit::linear(
            "gill",
            Dimension::volume(),
            us_gallon_m3().div(&i(32)).unwrap(),
        )
    }
    pub fn us_fluid_ounce() -> Unit {
        Unit::linear(
            "fl oz",
            Dimension::volume(),
            us_gallon_m3().div(&i(128)).unwrap(),
        )
    }
    pub fn tablespoon() -> Unit {
        Unit::linear(
            "tbsp",
            Dimension::volume(),
            us_gallon_m3().div(&i(256)).unwrap(),
        )
    }
    pub fn teaspoon() -> Unit {
        Unit::linear(
            "tsp",
            Dimension::volume(),
            us_gallon_m3().div(&i(768)).unwrap(),
        )
    }
    pub fn oil_barrel() -> Unit {
        Unit::linear("bbl", Dimension::volume(), us_gallon_m3().mul(&i(42)))
    } // 42 US gal
      // ---- Imperial ----
    pub fn imperial_gallon() -> Unit {
        Unit::linear("imp gal", Dimension::volume(), imp_gallon_m3())
    }
    pub fn imperial_pint() -> Unit {
        Unit::linear(
            "imp pt",
            Dimension::volume(),
            imp_gallon_m3().div(&i(8)).unwrap(),
        )
    }
    pub fn imperial_fluid_ounce() -> Unit {
        Unit::linear(
            "imp fl oz",
            Dimension::volume(),
            imp_gallon_m3().div(&i(160)).unwrap(),
        )
    }

    // ============================ AREA (base: m²) ============================
    pub fn square_metre() -> Unit {
        Unit::linear("m²", Dimension::area(), Rational::one())
    }
    pub fn square_kilometre() -> Unit {
        Unit::linear("km²", Dimension::area(), i(1_000_000))
    }
    pub fn square_centimetre() -> Unit {
        Unit::linear("cm²", Dimension::area(), r(1, 10_000))
    }
    pub fn square_inch() -> Unit {
        Unit::linear("in²", Dimension::area(), square(inch_m()))
    }
    pub fn square_foot() -> Unit {
        Unit::linear("ft²", Dimension::area(), square(foot_m()))
    }
    pub fn square_yard() -> Unit {
        Unit::linear("yd²", Dimension::area(), square(r(1143, 1250)))
    }
    pub fn square_mile() -> Unit {
        Unit::linear("mi²", Dimension::area(), square(r(201_168, 125)))
    }
    pub fn hectare() -> Unit {
        Unit::linear("ha", Dimension::area(), i(10_000))
    }
    pub fn are() -> Unit {
        Unit::linear("a", Dimension::area(), i(100))
    }
    pub fn acre() -> Unit {
        Unit::linear("ac", Dimension::area(), square(foot_m()).mul(&i(43_560)))
    } // 43560 ft²

    // ============================ SPEED (base: m/s) ============================
    pub fn metre_per_second() -> Unit {
        Unit::linear("m/s", Dimension::speed(), Rational::one())
    }
    pub fn kilometre_per_hour() -> Unit {
        Unit::linear("km/h", Dimension::speed(), r(5, 18))
    } // 1000/3600
    pub fn mile_per_hour() -> Unit {
        Unit::linear(
            "mph",
            Dimension::speed(),
            r(201_168, 125).div(&i(3600)).unwrap(),
        )
    }
    pub fn foot_per_second() -> Unit {
        Unit::linear("ft/s", Dimension::speed(), foot_m())
    }
    pub fn knot() -> Unit {
        Unit::linear("kn", Dimension::speed(), r(1852, 3600))
    } // nmi/h

    // ============================ FREQUENCY (base: 1/s) ============================
    pub fn hertz() -> Unit {
        Unit::linear("Hz", Dimension::frequency(), Rational::one())
    }
    pub fn kilohertz() -> Unit {
        Unit::linear("kHz", Dimension::frequency(), i(1000))
    }
    pub fn megahertz() -> Unit {
        Unit::linear("MHz", Dimension::frequency(), i(1_000_000))
    }
    pub fn gigahertz() -> Unit {
        Unit::linear("GHz", Dimension::frequency(), i(1_000_000_000))
    }
    pub fn rpm() -> Unit {
        Unit::linear("rpm", Dimension::frequency(), r(1, 60))
    }

    // ============================ FORCE (base: newton) ============================
    pub fn newton() -> Unit {
        Unit::linear("N", Dimension::force(), Rational::one())
    }
    pub fn kilonewton() -> Unit {
        Unit::linear("kN", Dimension::force(), i(1000))
    }
    pub fn dyne() -> Unit {
        Unit::linear("dyn", Dimension::force(), r(1, 100_000))
    }
    pub fn kilogram_force() -> Unit {
        Unit::linear("kgf", Dimension::force(), g0())
    }
    pub fn pound_force() -> Unit {
        Unit::linear("lbf", Dimension::force(), lb_kg().mul(&g0()))
    }

    // ============================ ENERGY (base: joule) ============================
    pub fn joule() -> Unit {
        Unit::linear("J", Dimension::energy(), Rational::one())
    }
    pub fn kilojoule() -> Unit {
        Unit::linear("kJ", Dimension::energy(), i(1000))
    }
    pub fn megajoule() -> Unit {
        Unit::linear("MJ", Dimension::energy(), i(1_000_000))
    }
    pub fn calorie() -> Unit {
        Unit::linear("cal", Dimension::energy(), r(523, 125))
    } // 4.184 J
    pub fn kilocalorie() -> Unit {
        Unit::linear("kcal", Dimension::energy(), r(4184, 1))
    }
    pub fn watt_hour() -> Unit {
        Unit::linear("Wh", Dimension::energy(), i(3600))
    }
    pub fn kilowatt_hour() -> Unit {
        Unit::linear("kWh", Dimension::energy(), i(3_600_000))
    }
    pub fn erg() -> Unit {
        Unit::linear("erg", Dimension::energy(), r(1, 10_000_000))
    }
    pub fn electronvolt() -> Unit {
        // 1.602176634e-19 J = 1602176634 / 10^28 (the 10^28 denominator overflows i64, so build it
        // from BigInt — exact).
        use crate::numeric::BigInt;
        let scale = Rational::new(
            BigInt::from_i64(1_602_176_634),
            BigInt::from_i64(10).pow(28),
        )
        .expect("nonzero denominator");
        Unit::linear("eV", Dimension::energy(), scale)
    }

    // ============================ POWER (base: watt) ============================
    pub fn watt() -> Unit {
        Unit::linear("W", Dimension::power(), Rational::one())
    }
    pub fn kilowatt() -> Unit {
        Unit::linear("kW", Dimension::power(), i(1000))
    }
    pub fn megawatt() -> Unit {
        Unit::linear("MW", Dimension::power(), i(1_000_000))
    }
    pub fn horsepower() -> Unit {
        Unit::linear(
            "hp",
            Dimension::power(),
            lb_kg().mul(&g0()).mul(&foot_m()).mul(&i(550)),
        )
    } // 550 ft·lbf/s

    // ============================ PRESSURE (base: pascal) ============================
    pub fn pascal() -> Unit {
        Unit::linear("Pa", Dimension::pressure(), Rational::one())
    }
    pub fn kilopascal() -> Unit {
        Unit::linear("kPa", Dimension::pressure(), i(1000))
    }
    pub fn bar() -> Unit {
        Unit::linear("bar", Dimension::pressure(), i(100_000))
    }
    pub fn millibar() -> Unit {
        Unit::linear("mbar", Dimension::pressure(), i(100))
    }
    pub fn atmosphere() -> Unit {
        Unit::linear("atm", Dimension::pressure(), i(101_325))
    }
    pub fn psi() -> Unit {
        Unit::linear(
            "psi",
            Dimension::pressure(),
            lb_kg().mul(&g0()).div(&square(inch_m())).unwrap(),
        )
    } // lbf/in²

    // ============================ INFORMATION (base: bit) ============================
    pub fn bit() -> Unit {
        Unit::linear("bit", Dimension::information(), Rational::one())
    }
    pub fn byte() -> Unit {
        Unit::linear("B", Dimension::information(), i(8))
    }
    pub fn kilobit() -> Unit {
        Unit::linear("kbit", Dimension::information(), i(1000))
    }
    pub fn kilobyte() -> Unit {
        Unit::linear("kB", Dimension::information(), i(8000))
    }
    pub fn megabyte() -> Unit {
        Unit::linear("MB", Dimension::information(), i(8_000_000))
    }
    pub fn gigabyte() -> Unit {
        Unit::linear("GB", Dimension::information(), i(8_000_000_000))
    }
    pub fn kibibyte() -> Unit {
        Unit::linear("KiB", Dimension::information(), i(8 * 1024))
    }
    pub fn mebibyte() -> Unit {
        Unit::linear("MiB", Dimension::information(), i(8 * 1024 * 1024))
    }
    pub fn gibibyte() -> Unit {
        Unit::linear("GiB", Dimension::information(), i(8 * 1024 * 1024 * 1024))
    }

    // ============================ ANGLE (base: radian) ============================
    // (Degrees etc. are exact multiples of a turn; radian-to-degree is irrational, so these are
    // defined against the TURN as the natural rational anchor — full circle = 1 turn.)
    pub fn turn() -> Unit {
        Unit::linear("turn", Dimension::angle(), Rational::one())
    }
    pub fn degree() -> Unit {
        Unit::linear("°", Dimension::angle(), r(1, 360))
    }
    pub fn gradian() -> Unit {
        Unit::linear("grad", Dimension::angle(), r(1, 400))
    }
    pub fn arcminute() -> Unit {
        Unit::linear("′", Dimension::angle(), r(1, 21_600))
    } // degree/60
    pub fn arcsecond() -> Unit {
        Unit::linear("″", Dimension::angle(), r(1, 1_296_000))
    } // degree/3600

    // ============================ ELECTRICAL ============================
    pub fn ampere() -> Unit {
        Unit::linear("A", Dimension::current(), Rational::one())
    }
    pub fn milliampere() -> Unit {
        Unit::linear("mA", Dimension::current(), r(1, 1000))
    }
    pub fn coulomb() -> Unit {
        Unit::linear("C", Dimension::charge(), Rational::one())
    }
    pub fn ampere_hour() -> Unit {
        Unit::linear("Ah", Dimension::charge(), i(3600))
    }
    pub fn milliampere_hour() -> Unit {
        Unit::linear("mAh", Dimension::charge(), r(18, 5))
    } // 3.6 C
    pub fn volt() -> Unit {
        Unit::linear("V", Dimension::voltage(), Rational::one())
    }
    pub fn millivolt() -> Unit {
        Unit::linear("mV", Dimension::voltage(), r(1, 1000))
    }
    pub fn kilovolt() -> Unit {
        Unit::linear("kV", Dimension::voltage(), i(1000))
    }
    pub fn ohm() -> Unit {
        Unit::linear("Ω", Dimension::resistance(), Rational::one())
    }

    // ============================ AMOUNT / LUMINOUS ============================
    pub fn mole() -> Unit {
        Unit::linear("mol", Dimension::amount(), Rational::one())
    }
    pub fn candela() -> Unit {
        Unit::linear("cd", Dimension::luminous(), Rational::one())
    }

    /// `mantissa · 10^pow10` as an exact rational (for values past i64, e.g. stellar masses).
    fn e(mantissa: i64, pow10: u32) -> Rational {
        use crate::numeric::BigInt;
        Rational::from_bigint(BigInt::from_i64(mantissa).mul(&BigInt::from_i64(10).pow(pow10)))
    }

    // ============================ COOKING EXTRAS (volume) ============================
    pub fn stick_of_butter() -> Unit {
        Unit::linear(
            "stick",
            Dimension::volume(),
            us_gallon_m3().div(&i(32)).unwrap(),
        )
    } // ½ cup = 8 tbsp
    pub fn dash() -> Unit {
        Unit::linear(
            "dash",
            Dimension::volume(),
            us_gallon_m3().div(&i(6144)).unwrap(),
        )
    } // ⅛ tsp
    pub fn pinch() -> Unit {
        Unit::linear(
            "pinch",
            Dimension::volume(),
            us_gallon_m3().div(&i(12288)).unwrap(),
        )
    } // ¹⁄₁₆ tsp
    pub fn smidgen() -> Unit {
        Unit::linear(
            "smidgen",
            Dimension::volume(),
            us_gallon_m3().div(&i(24576)).unwrap(),
        )
    } // ¹⁄₃₂ tsp
      // US DRY volume (anchored to the 2150.42-in³ bushel).
    fn us_bushel_m3() -> Rational {
        cube(inch_m()).mul(&r(215_042, 100))
    } // 2150.42 in³
    pub fn bushel() -> Unit {
        Unit::linear("bu", Dimension::volume(), us_bushel_m3())
    }
    pub fn peck() -> Unit {
        Unit::linear(
            "pk",
            Dimension::volume(),
            us_bushel_m3().div(&i(4)).unwrap(),
        )
    }
    pub fn dry_gallon() -> Unit {
        Unit::linear(
            "dry gal",
            Dimension::volume(),
            us_bushel_m3().div(&i(8)).unwrap(),
        )
    }
    pub fn dry_quart() -> Unit {
        Unit::linear(
            "dry qt",
            Dimension::volume(),
            us_bushel_m3().div(&i(32)).unwrap(),
        )
    }
    pub fn dry_pint() -> Unit {
        Unit::linear(
            "dry pt",
            Dimension::volume(),
            us_bushel_m3().div(&i(64)).unwrap(),
        )
    }

    // ============================ NAUTICAL (length) ============================
    pub fn cable() -> Unit {
        Unit::linear("cable", Dimension::length(), r(926, 5))
    } // ¹⁄₁₀ nmi = 185.2 m
    pub fn league() -> Unit {
        Unit::linear("lea", Dimension::length(), r(603_504, 125))
    } // 3 statute miles
    pub fn nautical_league() -> Unit {
        Unit::linear("nl", Dimension::length(), i(5556))
    } // 3 nmi

    // ============================ ASTRONOMICAL ============================
    pub fn light_second() -> Unit {
        Unit::linear("ls", Dimension::length(), i(299_792_458))
    } // c·1 s
    pub fn light_minute() -> Unit {
        Unit::linear("lmin", Dimension::length(), i(299_792_458 * 60))
    }
    pub fn light_hour() -> Unit {
        Unit::linear("lh", Dimension::length(), i(299_792_458 * 3600))
    }
    pub fn light_day() -> Unit {
        Unit::linear("ld", Dimension::length(), i(299_792_458 * 86_400))
    }
    pub fn lunar_distance() -> Unit {
        Unit::linear("LD", Dimension::length(), i(384_399_000))
    } // mean Earth–Moon
    pub fn solar_radius() -> Unit {
        Unit::linear("R☉", Dimension::length(), i(696_340_000))
    }
    pub fn earth_radius() -> Unit {
        Unit::linear("R⊕", Dimension::length(), i(6_371_000))
    } // mean
      // Astronomical MASSES (measured values, exact as the conventional decimals given).
    pub fn solar_mass() -> Unit {
        Unit::linear("M☉", Dimension::mass(), e(198_892, 25))
    } // 1.98892e30 kg
    pub fn earth_mass() -> Unit {
        Unit::linear("M⊕", Dimension::mass(), e(59_722, 20))
    } // 5.9722e24 kg
    pub fn jupiter_mass() -> Unit {
        Unit::linear("M♃", Dimension::mass(), e(18_982, 23))
    } // 1.8982e27 kg

    // ============================ RADIATION ============================
    pub fn gray() -> Unit {
        Unit::linear("Gy", Dimension::absorbed_dose(), Rational::one())
    } // J/kg
    pub fn sievert() -> Unit {
        Unit::linear("Sv", Dimension::absorbed_dose(), Rational::one())
    } // J/kg (equiv dose)
    pub fn rad_unit() -> Unit {
        Unit::linear("rad", Dimension::absorbed_dose(), r(1, 100))
    } // 0.01 Gy
    pub fn rem() -> Unit {
        Unit::linear("rem", Dimension::absorbed_dose(), r(1, 100))
    } // 0.01 Sv
    pub fn becquerel() -> Unit {
        Unit::linear("Bq", Dimension::radioactivity(), Rational::one())
    } // 1/s
    pub fn curie() -> Unit {
        Unit::linear("Ci", Dimension::radioactivity(), i(37_000_000_000))
    } // 3.7e10 Bq
    pub fn roentgen() -> Unit {
        Unit::linear("R", Dimension::exposure(), r(129, 500_000))
    } // 2.58e-4 C/kg
    pub fn katal() -> Unit {
        Unit::linear("kat", Dimension::catalytic_activity(), Rational::one())
    } // mol/s

    // ============================ ILLUMINATION (photometry) ============================
    pub fn lumen() -> Unit {
        Unit::linear("lm", Dimension::luminous_flux(), Rational::one())
    } // cd·sr
    pub fn lux() -> Unit {
        Unit::linear("lx", Dimension::illuminance(), Rational::one())
    } // lm/m²
    pub fn phot() -> Unit {
        Unit::linear("ph", Dimension::illuminance(), i(10_000))
    } // lm/cm²
    pub fn foot_candle() -> Unit {
        Unit::linear("fc", Dimension::illuminance(), square(r(1250, 381)))
    } // lm/ft² = 1/0.3048² lx
    pub fn nit() -> Unit {
        Unit::linear("nit", Dimension::luminance(), Rational::one())
    } // cd/m²

    // ============================ DATA RATE (base: bit/s) ============================
    pub fn bit_per_second() -> Unit {
        Unit::linear("bit/s", Dimension::data_rate(), Rational::one())
    }
    pub fn kilobit_per_second() -> Unit {
        Unit::linear("kbps", Dimension::data_rate(), i(1000))
    }
    pub fn megabit_per_second() -> Unit {
        Unit::linear("Mbps", Dimension::data_rate(), i(1_000_000))
    }
    pub fn gigabit_per_second() -> Unit {
        Unit::linear("Gbps", Dimension::data_rate(), i(1_000_000_000))
    }
    pub fn byte_per_second() -> Unit {
        Unit::linear("B/s", Dimension::data_rate(), i(8))
    }
    pub fn megabyte_per_second() -> Unit {
        Unit::linear("MB/s", Dimension::data_rate(), i(8_000_000))
    }

    // ============================ VOLUMETRIC FLOW (base: m³/s) ============================
    pub fn cubic_metre_per_second() -> Unit {
        Unit::linear("m³/s", Dimension::volumetric_flow(), Rational::one())
    }
    pub fn litre_per_second() -> Unit {
        Unit::linear("L/s", Dimension::volumetric_flow(), r(1, 1000))
    }
    pub fn litre_per_minute() -> Unit {
        Unit::linear("L/min", Dimension::volumetric_flow(), r(1, 60_000))
    }
    pub fn gallon_per_minute() -> Unit {
        Unit::linear(
            "gpm",
            Dimension::volumetric_flow(),
            us_gallon_m3().div(&i(60)).unwrap(),
        )
    }
    pub fn cubic_foot_per_minute() -> Unit {
        Unit::linear(
            "cfm",
            Dimension::volumetric_flow(),
            cube(foot_m()).div(&i(60)).unwrap(),
        )
    }

    // ============================ CONCENTRATION (base: mol/m³) ============================
    pub fn molar() -> Unit {
        Unit::linear("M", Dimension::molar_concentration(), i(1000))
    } // mol/L
    pub fn millimolar() -> Unit {
        Unit::linear("mM", Dimension::molar_concentration(), Rational::one())
    }
    pub fn micromolar() -> Unit {
        Unit::linear("µM", Dimension::molar_concentration(), r(1, 1000))
    }
    pub fn molal() -> Unit {
        Unit::linear("m", Dimension::molality(), Rational::one())
    } // mol/kg

    // ============================ VISCOSITY ============================
    pub fn pascal_second() -> Unit {
        Unit::linear("Pa·s", Dimension::dynamic_viscosity(), Rational::one())
    }
    pub fn poise() -> Unit {
        Unit::linear("P", Dimension::dynamic_viscosity(), r(1, 10))
    }
    pub fn centipoise() -> Unit {
        Unit::linear("cP", Dimension::dynamic_viscosity(), r(1, 1000))
    }
    pub fn square_metre_per_second() -> Unit {
        Unit::linear("m²/s", Dimension::kinematic_viscosity(), Rational::one())
    }
    pub fn stokes() -> Unit {
        Unit::linear("St", Dimension::kinematic_viscosity(), r(1, 10_000))
    }
    pub fn centistokes() -> Unit {
        Unit::linear("cSt", Dimension::kinematic_viscosity(), r(1, 1_000_000))
    }

    // ============================ FUEL ECONOMY (base: m/m³ = m⁻²) ============================
    pub fn mile_per_gallon() -> Unit {
        Unit::linear(
            "mpg",
            Dimension::fuel_economy(),
            r(201_168, 125).div(&us_gallon_m3()).unwrap(),
        )
    }
    pub fn km_per_litre() -> Unit {
        Unit::linear(
            "km/L",
            Dimension::fuel_economy(),
            i(1000).div(&r(1, 1000)).unwrap(),
        )
    } // 10^6 m⁻²

    // ============================ TORQUE (shares the energy dimension M·L²·T⁻²) ============================
    pub fn newton_metre() -> Unit {
        Unit::linear("N·m", Dimension::energy(), Rational::one())
    }
    pub fn pound_foot() -> Unit {
        Unit::linear(
            "lb·ft",
            Dimension::energy(),
            lb_kg().mul(&g0()).mul(&foot_m()),
        )
    }

    // ============================ EXTRA SI PREFIXES (extend existing dimensions) ============================
    pub fn megametre() -> Unit {
        Unit::linear("Mm", Dimension::length(), i(1_000_000))
    }
    pub fn gigametre() -> Unit {
        Unit::linear("Gm", Dimension::length(), i(1_000_000_000))
    }
    pub fn picometre() -> Unit {
        Unit::linear("pm", Dimension::length(), r(1, 1_000_000_000_000))
    }
    pub fn femtometre() -> Unit {
        Unit::linear("fm", Dimension::length(), r(1, 1_000_000_000_000_000))
    }
    pub fn nanogram() -> Unit {
        Unit::linear("ng", Dimension::mass(), r(1, 1_000_000_000_000))
    }
    pub fn picosecond() -> Unit {
        Unit::linear("ps", Dimension::time(), r(1, 1_000_000_000_000))
    }
    pub fn gigawatt() -> Unit {
        Unit::linear("GW", Dimension::power(), i(1_000_000_000))
    }
    pub fn terawatt() -> Unit {
        Unit::linear("TW", Dimension::power(), i(1_000_000_000_000))
    }
    pub fn gigajoule() -> Unit {
        Unit::linear("GJ", Dimension::energy(), i(1_000_000_000))
    }
    pub fn terajoule() -> Unit {
        Unit::linear("TJ", Dimension::energy(), i(1_000_000_000_000))
    }
    pub fn terabit() -> Unit {
        Unit::linear("Tbit", Dimension::information(), i(1_000_000_000_000))
    }
    pub fn terabyte() -> Unit {
        Unit::linear("TB", Dimension::information(), i(8_000_000_000_000))
    }
    pub fn petabyte() -> Unit {
        Unit::linear("PB", Dimension::information(), i(8_000_000_000_000_000))
    }

    // ============================ ELECTROMAGNETISM ============================
    pub fn siemens() -> Unit {
        Unit::linear("S", Dimension::conductance(), Rational::one())
    }
    pub fn millisiemens() -> Unit {
        Unit::linear("mS", Dimension::conductance(), r(1, 1000))
    }
    pub fn farad() -> Unit {
        Unit::linear("F", Dimension::capacitance(), Rational::one())
    }
    pub fn microfarad() -> Unit {
        Unit::linear("µF", Dimension::capacitance(), r(1, 1_000_000))
    }
    pub fn nanofarad() -> Unit {
        Unit::linear("nF", Dimension::capacitance(), r(1, 1_000_000_000))
    }
    pub fn picofarad() -> Unit {
        Unit::linear("pF", Dimension::capacitance(), r(1, 1_000_000_000_000))
    }
    pub fn weber() -> Unit {
        Unit::linear("Wb", Dimension::magnetic_flux(), Rational::one())
    }
    pub fn maxwell() -> Unit {
        Unit::linear("Mx", Dimension::magnetic_flux(), r(1, 100_000_000))
    } // 1e-8 Wb
    pub fn henry() -> Unit {
        Unit::linear("H", Dimension::inductance(), Rational::one())
    }
    pub fn millihenry() -> Unit {
        Unit::linear("mH", Dimension::inductance(), r(1, 1000))
    }
    pub fn microhenry() -> Unit {
        Unit::linear("µH", Dimension::inductance(), r(1, 1_000_000))
    }
    pub fn tesla() -> Unit {
        Unit::linear("T", Dimension::magnetic_flux_density(), Rational::one())
    }
    pub fn millitesla() -> Unit {
        Unit::linear("mT", Dimension::magnetic_flux_density(), r(1, 1000))
    }
    pub fn gauss() -> Unit {
        Unit::linear("G", Dimension::magnetic_flux_density(), r(1, 10_000))
    } // 1e-4 T

    // ============================ SURFACE TENSION (base: N/m) ============================
    pub fn newton_per_metre() -> Unit {
        Unit::linear("N/m", Dimension::surface_tension(), Rational::one())
    }
    pub fn dyne_per_centimetre() -> Unit {
        Unit::linear("dyn/cm", Dimension::surface_tension(), r(1, 1000))
    } // 1e-3 N/m

    // ============================ COUNTING & RATIO (dimensionless) ============================
    // The base "unit" is a bare count of one; everything here is a pure dimensionless scale factor.
    pub fn each() -> Unit {
        Unit::linear("ea", Dimension::DIMENSIONLESS, Rational::one())
    }
    pub fn pair() -> Unit {
        Unit::linear("pair", Dimension::DIMENSIONLESS, i(2))
    }
    pub fn dozen() -> Unit {
        Unit::linear("dz", Dimension::DIMENSIONLESS, i(12))
    }
    pub fn baker_dozen() -> Unit {
        Unit::linear("baker's dz", Dimension::DIMENSIONLESS, i(13))
    }
    pub fn score() -> Unit {
        Unit::linear("score", Dimension::DIMENSIONLESS, i(20))
    }
    pub fn gross() -> Unit {
        Unit::linear("gross", Dimension::DIMENSIONLESS, i(144))
    }
    pub fn great_gross() -> Unit {
        Unit::linear("great gross", Dimension::DIMENSIONLESS, i(1728))
    }
    pub fn ream() -> Unit {
        Unit::linear("ream", Dimension::DIMENSIONLESS, i(500))
    }
    pub fn percent() -> Unit {
        Unit::linear("%", Dimension::DIMENSIONLESS, r(1, 100))
    }
    pub fn permille() -> Unit {
        Unit::linear("‰", Dimension::DIMENSIONLESS, r(1, 1000))
    }
    pub fn ppm() -> Unit {
        Unit::linear("ppm", Dimension::DIMENSIONLESS, r(1, 1_000_000))
    }
    pub fn ppb() -> Unit {
        Unit::linear("ppb", Dimension::DIMENSIONLESS, r(1, 1_000_000_000))
    }
    pub fn basis_point() -> Unit {
        Unit::linear("bp", Dimension::DIMENSIONLESS, r(1, 10_000))
    }

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
            "square meter" | "square metre" | "square meters" | "square metres" | "m2" => {
                square_metre()
            }
            "square foot" | "square feet" | "sq ft" => square_foot(),
            "square inch" | "square inches" | "sq in" => square_inch(),
            "hectare" | "hectares" | "ha" => hectare(),
            "acre" | "acres" => acre(),
            // Speed.
            "meter per second" | "metre per second" | "meters per second" | "m/s" => {
                metre_per_second()
            }
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
            // Electromagnetism. (The bare "v"/"a" symbols are intentionally absent — they
            // collide with everyday identifiers and the article, like radian/parsec above.)
            "volt" | "volts" => volt(),
            "millivolt" | "millivolts" | "mv" => millivolt(),
            "kilovolt" | "kilovolts" | "kv" => kilovolt(),
            "ampere" | "amperes" | "amp" | "amps" => ampere(),
            "milliampere" | "milliamperes" | "milliamp" | "milliamps" => milliampere(),
            "coulomb" | "coulombs" => coulomb(),
            "ampere hour" | "ampere hours" | "ah" => ampere_hour(),
            "milliampere hour" | "milliampere hours" | "mah" => milliampere_hour(),
            "ohm" | "ohms" | "ω" => ohm(),
            "siemens" => siemens(),
            "millisiemens" => millisiemens(),
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
    fn action() -> Dimension {
        Dimension::energy().mul(Dimension::time())
    }

    /// Speed of light in vacuum, `c` = 299 792 458 m/s (exact).
    pub fn speed_of_light() -> Quantity {
        Quantity::si(ep(299_792_458, 0), Dimension::speed())
    }
    /// Planck constant, `h` = 6.626 070 15 × 10⁻³⁴ J·s (exact).
    pub fn planck_constant() -> Quantity {
        Quantity::si(en(662_607_015, 42), action())
    }
    /// Elementary charge, `e` = 1.602 176 634 × 10⁻¹⁹ C (exact).
    pub fn elementary_charge() -> Quantity {
        Quantity::si(en(1_602_176_634, 28), Dimension::charge())
    }
    /// Boltzmann constant, `k_B` = 1.380 649 × 10⁻²³ J/K (exact).
    pub fn boltzmann_constant() -> Quantity {
        Quantity::si(
            en(1_380_649, 29),
            Dimension::energy().div(Dimension::temperature()),
        )
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
        let dim = Dimension::volume()
            .div(Dimension::mass())
            .div(Dimension::time().powi(2));
        Quantity::si(en(667_430, 16), dim)
    }
    /// Standard gravity, `g₀` = 9.806 65 m/s² (exact by definition).
    pub fn standard_gravity() -> Quantity {
        Quantity::si(r(980_665, 100_000), Dimension::acceleration())
    }
    /// Standard atmosphere, `atm` = 101 325 Pa (exact by definition).
    pub fn standard_atmosphere() -> Quantity {
        Quantity::si(ep(101_325, 0), Dimension::pressure())
    }
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
    q.magnitude_si()
        .mul(&Rational::from_i64(1_000_000_000))
        .round()
        .to_i64()
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
        let other = SpacetimeStamp {
            instant_nanos: self.instant_nanos,
            position: observer,
        };
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
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/quantity/tests.rs"]
mod tests;
