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
    assert_eq!(
        sum.in_unit(&metre()).unwrap(),
        r(63, 625),
        "= 63/625 m exactly"
    );
    assert_eq!(
        sum.in_unit(&foot()).unwrap(),
        r(42, 127),
        "= 42/127 ft exactly"
    );
    // The lossy float answer a JSON-number language would give is NOT 42/127.
    assert_ne!((42.0_f64 / 127.0) as f32 as f64, 42.0 / 127.0);
}

#[test]
fn exact_unit_conversions_within_a_dimension() {
    assert_eq!(
        Quantity::of(i(1), &mile()).in_unit(&foot()).unwrap(),
        i(5280)
    );
    assert_eq!(Quantity::of(i(1), &foot()).in_unit(&inch()).unwrap(), i(12));
    assert_eq!(Quantity::of(i(1), &yard()).in_unit(&foot()).unwrap(), i(3));
    assert_eq!(
        Quantity::of(i(1), &kilometre()).in_unit(&metre()).unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &kilogram()).in_unit(&gram()).unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &pound()).in_unit(&ounce()).unwrap(),
        i(16)
    );
    assert_eq!(
        Quantity::of(i(1), &hour()).in_unit(&minute()).unwrap(),
        i(60)
    );
    // 1 pound = 453.59237 g exactly.
    assert_eq!(
        Quantity::of(i(1), &pound()).in_unit(&gram()).unwrap(),
        r(45_359_237, 100_000)
    );
}

#[test]
fn affine_temperature_conversion_is_offset_aware_and_exact() {
    // 0 °C = 273.15 K; 20 °C = 68 °F; 100 °C = 212 °F; −40 °C = −40 °F (the crossover).
    assert_eq!(
        Quantity::of(i(0), &celsius()).in_unit(&kelvin()).unwrap(),
        r(5463, 20)
    );
    assert_eq!(
        Quantity::of(i(20), &celsius())
            .in_unit(&fahrenheit())
            .unwrap(),
        i(68)
    );
    assert_eq!(
        Quantity::of(i(100), &celsius())
            .in_unit(&fahrenheit())
            .unwrap(),
        i(212)
    );
    assert_eq!(
        Quantity::of(i(-40), &celsius())
            .in_unit(&fahrenheit())
            .unwrap(),
        i(-40)
    );
    assert_eq!(
        Quantity::of(i(212), &fahrenheit())
            .in_unit(&celsius())
            .unwrap(),
        i(100)
    );
}

#[test]
fn cross_dimension_operations_are_forbidden() {
    let length = Quantity::of(i(1), &metre());
    let mass = Quantity::of(i(1), &kilogram());
    // Can't add Length to Mass; can't convert Length to a Mass unit.
    assert!(length.add(&mass).is_none(), "Length + Mass is forbidden");
    assert!(length.sub(&mass).is_none(), "Length − Mass is forbidden");
    assert!(
        length.in_unit(&kilogram()).is_none(),
        "Length → Mass cast is forbidden"
    );
}

#[test]
fn multiplication_and_division_combine_dimensions() {
    // Length × Length = Area.
    let area = Quantity::of(i(3), &metre()).mul(&Quantity::of(i(4), &metre()));
    assert_eq!(area.dimension(), Dimension::area());
    assert_eq!(area.magnitude_si(), &i(12)); // 12 m²
                                             // Length ÷ Time = Speed.
    let speed = Quantity::of(i(100), &metre())
        .div(&Quantity::of(i(10), &second()))
        .unwrap();
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
    let accel = Quantity::of(i(3), &metre_per_second())
        .div(&Quantity::of(i(1), &second()))
        .unwrap();
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
    let rate = Quantity::of(i(16), &bit())
        .div(&Quantity::of(i(2), &second()))
        .unwrap();
    assert_eq!(rate.dimension(), Dimension::data_rate());
    assert_eq!(rate.in_unit(&bit_per_second()).unwrap(), i(8)); // 8 bit/s
                                                                // volume ÷ time = volumetric flow.
    let flow = Quantity::of(i(6), &cubic_metre())
        .div(&Quantity::of(i(2), &second()))
        .unwrap();
    assert_eq!(flow.dimension(), Dimension::volumetric_flow());
    assert_eq!(flow.in_unit(&cubic_metre_per_second()).unwrap(), i(3));
    // amount ÷ volume = molar concentration (2 mol/m³ = 2 mM).
    let conc = Quantity::of(i(2), &mole())
        .div(&Quantity::of(i(1), &cubic_metre()))
        .unwrap();
    assert_eq!(conc.dimension(), Dimension::molar_concentration());
    assert_eq!(conc.in_unit(&millimolar()).unwrap(), i(2));
    // area ÷ time = kinematic viscosity.
    let kv = Quantity::of(i(4), &square_metre())
        .div(&Quantity::of(i(2), &second()))
        .unwrap();
    assert_eq!(kv.dimension(), Dimension::kinematic_viscosity());
    assert_eq!(kv.in_unit(&square_metre_per_second()).unwrap(), i(2));
    // pressure × time = dynamic viscosity.
    let dv = Quantity::of(i(4), &pascal()).mul(&Quantity::of(i(2), &second()));
    assert_eq!(dv.dimension(), Dimension::dynamic_viscosity());
    assert_eq!(dv.in_unit(&pascal_second()).unwrap(), i(8));
    // length ÷ volume = fuel economy (100 km / 10 L = 10 km/L).
    let fe = Quantity::of(i(100), &kilometre())
        .div(&Quantity::of(i(10), &litre()))
        .unwrap();
    assert_eq!(fe.dimension(), Dimension::fuel_economy());
    assert_eq!(fe.in_unit(&km_per_litre()).unwrap(), i(10));
    // dynamic viscosity ÷ density = kinematic viscosity (μ/ρ = ν), exact magnitude.
    let mu = Quantity::of(i(6), &pascal_second());
    let rho = Quantity::of(i(2), &kilogram())
        .div(&Quantity::of(i(1), &cubic_metre()))
        .unwrap();
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
    let length_units = [
        metre(),
        kilometre(),
        centimetre(),
        inch(),
        foot(),
        yard(),
        mile(),
    ];
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
            assert_eq!(
                via.in_unit(u1).unwrap(),
                value,
                "{} → {} → {}",
                u1.symbol,
                u2.symbol,
                u1.symbol
            );
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
    assert_eq!(
        Quantity::of(i(0), &celsius()).in_unit(&kelvin()).unwrap(),
        r(27315, 100)
    );
    assert_eq!(
        Quantity::of(i(0), &celsius())
            .in_unit(&fahrenheit())
            .unwrap(),
        i(32)
    );
    assert_eq!(
        Quantity::of(i(0), &celsius()).in_unit(&rankine()).unwrap(),
        r(49167, 100)
    );
    assert_eq!(
        Quantity::of(i(0), &celsius()).in_unit(&reaumur()).unwrap(),
        i(0)
    );
    // Water boils: 100 °C = 212 °F = 373.15 K = 80 °Ré.
    assert_eq!(
        Quantity::of(i(100), &celsius())
            .in_unit(&fahrenheit())
            .unwrap(),
        i(212)
    );
    assert_eq!(
        Quantity::of(i(100), &celsius()).in_unit(&kelvin()).unwrap(),
        r(37315, 100)
    );
    assert_eq!(
        Quantity::of(i(100), &celsius())
            .in_unit(&reaumur())
            .unwrap(),
        i(80)
    );
    // The famous crossover: −40 °C = −40 °F exactly.
    assert_eq!(
        Quantity::of(i(-40), &celsius())
            .in_unit(&fahrenheit())
            .unwrap(),
        i(-40)
    );
    // Absolute zero: 0 K = −273.15 °C = −459.67 °F = 0 °R.
    assert_eq!(
        Quantity::of(i(0), &kelvin()).in_unit(&celsius()).unwrap(),
        r(-27315, 100)
    );
    assert_eq!(
        Quantity::of(i(0), &kelvin())
            .in_unit(&fahrenheit())
            .unwrap(),
        r(-45967, 100)
    );
    assert_eq!(
        Quantity::of(i(0), &kelvin()).in_unit(&rankine()).unwrap(),
        i(0)
    );
    // Body temperature, a non-integer anchor: 37 °C = 98.6 °F.
    assert_eq!(
        Quantity::of(i(37), &celsius())
            .in_unit(&fahrenheit())
            .unwrap(),
        r(986, 10)
    );
    // Round-trip through every affine scale is identity for an arbitrary fractional reading.
    let v = r(37, 7);
    for u in [kelvin(), celsius(), fahrenheit(), rankine(), reaumur()] {
        assert_eq!(
            Quantity::of(v.clone(), &u).in_unit(&u).unwrap(),
            v,
            "round-trip {}",
            u.symbol
        );
    }
}

#[test]
fn cooking_measurements_convert_exactly() {
    // The US kitchen ladder — all exact, all anchored to the 231-in³ gallon.
    assert_eq!(
        Quantity::of(i(1), &us_cup())
            .in_unit(&tablespoon())
            .unwrap(),
        i(16),
        "1 cup = 16 tbsp"
    );
    assert_eq!(
        Quantity::of(i(1), &us_cup()).in_unit(&teaspoon()).unwrap(),
        i(48),
        "1 cup = 48 tsp"
    );
    assert_eq!(
        Quantity::of(i(1), &us_cup())
            .in_unit(&us_fluid_ounce())
            .unwrap(),
        i(8),
        "1 cup = 8 fl oz"
    );
    assert_eq!(
        Quantity::of(i(1), &tablespoon())
            .in_unit(&teaspoon())
            .unwrap(),
        i(3),
        "1 tbsp = 3 tsp"
    );
    assert_eq!(
        Quantity::of(i(1), &us_fluid_ounce())
            .in_unit(&tablespoon())
            .unwrap(),
        i(2),
        "1 fl oz = 2 tbsp"
    );
    assert_eq!(
        Quantity::of(i(1), &us_pint()).in_unit(&us_cup()).unwrap(),
        i(2),
        "1 pint = 2 cups"
    );
    assert_eq!(
        Quantity::of(i(1), &us_quart()).in_unit(&us_cup()).unwrap(),
        i(4),
        "1 quart = 4 cups"
    );
    assert_eq!(
        Quantity::of(i(1), &us_gallon()).in_unit(&us_cup()).unwrap(),
        i(16),
        "1 gallon = 16 cups"
    );
    assert_eq!(
        Quantity::of(i(1), &us_gallon())
            .in_unit(&teaspoon())
            .unwrap(),
        i(768),
        "1 gallon = 768 tsp"
    );
    assert_eq!(
        Quantity::of(i(1), &oil_barrel())
            .in_unit(&us_gallon())
            .unwrap(),
        i(42),
        "1 oil barrel = 42 gal"
    );
    // Cross to metric, exact: 1 US gallon = 3.785411784 L; 1 tsp ≈ 4.92892159375 mL (exact rational).
    assert_eq!(
        Quantity::of(i(1), &us_gallon()).in_unit(&litre()).unwrap(),
        r(473_176_473, 125_000_000)
    );
    // A real recipe: 3 teaspoons + 1 tablespoon = 2 tablespoons, exactly.
    let mixed = Quantity::of(i(3), &teaspoon())
        .add(&Quantity::of(i(1), &tablespoon()))
        .unwrap();
    assert_eq!(mixed.in_unit(&tablespoon()).unwrap(), i(2));
}

#[test]
fn general_cross_unit_conversions_are_exact() {
    // Volume / cubic.
    assert_eq!(
        Quantity::of(i(1), &cubic_metre())
            .in_unit(&litre())
            .unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &cubic_foot())
            .in_unit(&cubic_inch())
            .unwrap(),
        i(1728)
    );
    assert_eq!(
        Quantity::of(i(1), &litre()).in_unit(&millilitre()).unwrap(),
        i(1000)
    );
    // Area.
    assert_eq!(
        Quantity::of(i(1), &acre()).in_unit(&square_foot()).unwrap(),
        i(43_560)
    );
    assert_eq!(
        Quantity::of(i(1), &hectare())
            .in_unit(&square_metre())
            .unwrap(),
        i(10_000)
    );
    assert_eq!(
        Quantity::of(i(1), &square_foot())
            .in_unit(&square_inch())
            .unwrap(),
        i(144)
    );
    // Speed, time, frequency.
    assert_eq!(
        Quantity::of(i(1), &kilometre_per_hour())
            .in_unit(&metre_per_second())
            .unwrap(),
        r(5, 18)
    );
    assert_eq!(Quantity::of(i(1), &day()).in_unit(&hour()).unwrap(), i(24));
    assert_eq!(
        Quantity::of(i(1), &fortnight()).in_unit(&day()).unwrap(),
        i(14)
    );
    assert_eq!(
        Quantity::of(i(1), &gigahertz()).in_unit(&hertz()).unwrap(),
        i(1_000_000_000)
    );
    // Energy / power / pressure.
    assert_eq!(
        Quantity::of(i(1), &kilocalorie())
            .in_unit(&calorie())
            .unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &kilowatt_hour())
            .in_unit(&joule())
            .unwrap(),
        i(3_600_000)
    );
    assert_eq!(
        Quantity::of(i(1), &calorie()).in_unit(&joule()).unwrap(),
        r(523, 125)
    ); // 4.184 J
    assert_eq!(
        Quantity::of(i(1), &atmosphere())
            .in_unit(&pascal())
            .unwrap(),
        i(101_325)
    );
    // Information (decimal vs binary prefixes).
    assert_eq!(Quantity::of(i(1), &byte()).in_unit(&bit()).unwrap(), i(8));
    assert_eq!(
        Quantity::of(i(1), &kibibyte()).in_unit(&byte()).unwrap(),
        i(1024)
    );
    assert_eq!(
        Quantity::of(i(1), &mebibyte())
            .in_unit(&kibibyte())
            .unwrap(),
        i(1024)
    );
    assert_eq!(
        Quantity::of(i(1), &megabyte()).in_unit(&byte()).unwrap(),
        i(1_000_000)
    );
    // Angle.
    assert_eq!(
        Quantity::of(i(1), &turn()).in_unit(&degree()).unwrap(),
        i(360)
    );
    assert_eq!(
        Quantity::of(i(1), &degree()).in_unit(&arcminute()).unwrap(),
        i(60)
    );
    assert_eq!(
        Quantity::of(i(1), &arcminute())
            .in_unit(&arcsecond())
            .unwrap(),
        i(60)
    );
    // Charge.
    assert_eq!(
        Quantity::of(i(1), &ampere_hour())
            .in_unit(&coulomb())
            .unwrap(),
        i(3600)
    );
}

#[test]
fn extended_units_cooking_nautical_astronomical_radiation_illumination_convert_exactly() {
    // --- Cooking extras ---
    assert_eq!(
        Quantity::of(i(1), &teaspoon()).in_unit(&dash()).unwrap(),
        i(8),
        "1 tsp = 8 dashes"
    );
    assert_eq!(
        Quantity::of(i(1), &teaspoon()).in_unit(&pinch()).unwrap(),
        i(16),
        "1 tsp = 16 pinches"
    );
    assert_eq!(
        Quantity::of(i(1), &teaspoon()).in_unit(&smidgen()).unwrap(),
        i(32),
        "1 tsp = 32 smidgens"
    );
    assert_eq!(
        Quantity::of(i(1), &dash()).in_unit(&pinch()).unwrap(),
        i(2),
        "1 dash = 2 pinches"
    );
    assert_eq!(
        Quantity::of(i(1), &stick_of_butter())
            .in_unit(&tablespoon())
            .unwrap(),
        i(8),
        "1 stick = 8 tbsp"
    );
    assert_eq!(
        Quantity::of(i(1), &stick_of_butter())
            .in_unit(&us_cup())
            .unwrap(),
        r(1, 2),
        "1 stick = ½ cup"
    );
    // --- US dry ---
    assert_eq!(
        Quantity::of(i(1), &bushel()).in_unit(&peck()).unwrap(),
        i(4),
        "1 bushel = 4 pecks"
    );
    assert_eq!(
        Quantity::of(i(1), &bushel())
            .in_unit(&dry_gallon())
            .unwrap(),
        i(8)
    );
    assert_eq!(
        Quantity::of(i(1), &dry_gallon())
            .in_unit(&dry_quart())
            .unwrap(),
        i(4)
    );
    // --- Nautical ---
    assert_eq!(
        Quantity::of(i(1), &nautical_mile())
            .in_unit(&cable())
            .unwrap(),
        i(10),
        "1 nmi = 10 cables"
    );
    assert_eq!(
        Quantity::of(i(1), &nautical_league())
            .in_unit(&nautical_mile())
            .unwrap(),
        i(3)
    );
    assert_eq!(
        Quantity::of(i(1), &league()).in_unit(&mile()).unwrap(),
        i(3),
        "1 league = 3 miles"
    );
    // --- Astronomical (the light-time ladder is exact; AU/ly are defined) ---
    assert_eq!(
        Quantity::of(i(1), &light_minute())
            .in_unit(&light_second())
            .unwrap(),
        i(60)
    );
    assert_eq!(
        Quantity::of(i(1), &light_hour())
            .in_unit(&light_minute())
            .unwrap(),
        i(60)
    );
    assert_eq!(
        Quantity::of(i(1), &light_day())
            .in_unit(&light_hour())
            .unwrap(),
        i(24)
    );
    assert_eq!(
        Quantity::of(i(1), &light_second())
            .in_unit(&metre())
            .unwrap(),
        i(299_792_458)
    );
    // --- Radiation ---
    assert_eq!(
        Quantity::of(i(1), &gray()).in_unit(&rad_unit()).unwrap(),
        i(100),
        "1 Gy = 100 rad"
    );
    assert_eq!(
        Quantity::of(i(1), &sievert()).in_unit(&rem()).unwrap(),
        i(100),
        "1 Sv = 100 rem"
    );
    assert_eq!(
        Quantity::of(i(1), &curie()).in_unit(&becquerel()).unwrap(),
        i(37_000_000_000),
        "1 Ci = 3.7e10 Bq"
    );
    assert_eq!(gray().dimension, Dimension::absorbed_dose());
    assert_eq!(sievert().dimension, Dimension::absorbed_dose()); // Gy and Sv share a dimension
    assert_eq!(becquerel().dimension, Dimension::radioactivity());
    assert_eq!(roentgen().dimension, Dimension::exposure());
    assert_eq!(katal().dimension, Dimension::catalytic_activity());
    // --- Illumination (photometry) ---
    assert_eq!(
        Quantity::of(i(1), &phot()).in_unit(&lux()).unwrap(),
        i(10_000),
        "1 phot = 10000 lux"
    );
    assert_eq!(
        Quantity::of(i(1), &foot_candle()).in_unit(&lux()).unwrap(),
        r(1_562_500, 145_161)
    );
    assert_eq!(lumen().dimension, Dimension::luminous_flux());
    assert_eq!(lux().dimension, Dimension::illuminance());
    assert_eq!(nit().dimension, Dimension::luminance());
}

#[test]
fn rate_flow_concentration_viscosity_torque_and_prefixes_convert_exactly() {
    // Data rate.
    assert_eq!(
        Quantity::of(i(1), &byte_per_second())
            .in_unit(&bit_per_second())
            .unwrap(),
        i(8)
    );
    assert_eq!(
        Quantity::of(i(1), &gigabit_per_second())
            .in_unit(&megabit_per_second())
            .unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &megabyte_per_second())
            .in_unit(&bit_per_second())
            .unwrap(),
        i(8_000_000)
    );
    // Volumetric flow.
    assert_eq!(
        Quantity::of(i(1), &cubic_metre_per_second())
            .in_unit(&litre_per_second())
            .unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &litre_per_second())
            .in_unit(&litre_per_minute())
            .unwrap(),
        i(60)
    );
    // Concentration.
    assert_eq!(
        Quantity::of(i(1), &molar()).in_unit(&millimolar()).unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &millimolar())
            .in_unit(&micromolar())
            .unwrap(),
        i(1000)
    );
    // Viscosity (dynamic + kinematic).
    assert_eq!(
        Quantity::of(i(1), &pascal_second())
            .in_unit(&poise())
            .unwrap(),
        i(10)
    );
    assert_eq!(
        Quantity::of(i(1), &poise()).in_unit(&centipoise()).unwrap(),
        i(100)
    );
    assert_eq!(
        Quantity::of(i(1), &square_metre_per_second())
            .in_unit(&stokes())
            .unwrap(),
        i(10_000)
    );
    assert_eq!(
        Quantity::of(i(1), &stokes())
            .in_unit(&centistokes())
            .unwrap(),
        i(100)
    );
    // Torque shares the energy dimension (N·m ≡ J dimensionally) — convertible to joules.
    assert_eq!(newton_metre().dimension, Dimension::energy());
    assert_eq!(
        Quantity::of(i(1), &newton_metre())
            .in_unit(&joule())
            .unwrap(),
        i(1)
    );
    // Fuel economy is its own (reciprocal-area) dimension.
    assert_eq!(km_per_litre().dimension, Dimension::fuel_economy());
    assert_eq!(mile_per_gallon().dimension, Dimension::fuel_economy());
    // Extra SI prefixes.
    assert_eq!(
        Quantity::of(i(1), &gigametre())
            .in_unit(&megametre())
            .unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &megametre()).in_unit(&metre()).unwrap(),
        i(1_000_000)
    );
    assert_eq!(
        Quantity::of(i(1), &terawatt())
            .in_unit(&gigawatt())
            .unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &petabyte())
            .in_unit(&terabyte())
            .unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &picometre())
            .in_unit(&femtometre())
            .unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &terajoule())
            .in_unit(&gigajoule())
            .unwrap(),
        i(1000)
    );
}

#[test]
fn electromagnetic_and_surface_tension_units_convert_exactly() {
    // Conductance is dimensionally the reciprocal of resistance (S = Ω⁻¹).
    assert_eq!(siemens().dimension, Dimension::resistance().recip());
    assert_eq!(
        Quantity::of(i(1), &siemens())
            .in_unit(&millisiemens())
            .unwrap(),
        i(1000)
    );
    // Capacitance ladder.
    assert_eq!(
        Quantity::of(i(1), &microfarad())
            .in_unit(&nanofarad())
            .unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &nanofarad())
            .in_unit(&picofarad())
            .unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &farad()).in_unit(&picofarad()).unwrap(),
        i(1_000_000_000_000)
    );
    // Magnetic flux: 1 Wb = 10⁸ maxwell.
    assert_eq!(
        Quantity::of(i(1), &weber()).in_unit(&maxwell()).unwrap(),
        i(100_000_000)
    );
    // Inductance ladder.
    assert_eq!(
        Quantity::of(i(1), &henry()).in_unit(&millihenry()).unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &millihenry())
            .in_unit(&microhenry())
            .unwrap(),
        i(1000)
    );
    // Magnetic flux density: 1 T = 10⁴ gauss = 10³ mT.
    assert_eq!(
        Quantity::of(i(1), &tesla()).in_unit(&gauss()).unwrap(),
        i(10_000)
    );
    assert_eq!(
        Quantity::of(i(1), &tesla()).in_unit(&millitesla()).unwrap(),
        i(1000)
    );
    // Surface tension: 1 N/m = 1000 dyn/cm.
    assert_eq!(
        Quantity::of(i(1), &newton_per_metre())
            .in_unit(&dyne_per_centimetre())
            .unwrap(),
        i(1000)
    );
    // The defining SI relations hold at the dimension level (units can never drift from physics).
    assert_eq!(
        farad().dimension,
        Dimension::charge().div(Dimension::voltage())
    );
    assert_eq!(
        henry().dimension,
        Dimension::magnetic_flux().div(Dimension::current())
    );
    assert_eq!(
        tesla().dimension,
        Dimension::magnetic_flux().div(Dimension::area())
    );
    // Cross-dimension stays a forbidden cast (capacitance ≠ inductance).
    assert!(Quantity::of(i(1), &farad()).in_unit(&henry()).is_none());
}

#[test]
fn dimensionless_counting_and_ratio_units_convert_exactly() {
    // Counting words are exact integer multiples of a single item.
    assert_eq!(
        Quantity::of(i(1), &dozen()).in_unit(&each()).unwrap(),
        i(12)
    );
    assert_eq!(
        Quantity::of(i(1), &gross()).in_unit(&dozen()).unwrap(),
        i(12)
    ); // 144 = 12 dozen
    assert_eq!(
        Quantity::of(i(1), &great_gross())
            .in_unit(&gross())
            .unwrap(),
        i(12)
    );
    assert_eq!(
        Quantity::of(i(1), &score()).in_unit(&each()).unwrap(),
        i(20)
    );
    assert_eq!(
        Quantity::of(i(1), &baker_dozen()).in_unit(&each()).unwrap(),
        i(13)
    );
    assert_eq!(
        Quantity::of(i(1), &ream()).in_unit(&each()).unwrap(),
        i(500)
    );
    // Ratios: 1 = 100% = 1000‰ = 10000 bp; ppm/ppb are the metrology fractions.
    assert_eq!(
        Quantity::of(i(1), &each()).in_unit(&percent()).unwrap(),
        i(100)
    );
    assert_eq!(
        Quantity::of(i(1), &each()).in_unit(&permille()).unwrap(),
        i(1000)
    );
    assert_eq!(
        Quantity::of(i(1), &each()).in_unit(&basis_point()).unwrap(),
        i(10_000)
    );
    assert_eq!(
        Quantity::of(i(1), &percent())
            .in_unit(&basis_point())
            .unwrap(),
        i(100)
    ); // 1% = 100 bp
    assert_eq!(
        Quantity::of(i(1), &percent()).in_unit(&permille()).unwrap(),
        i(10)
    );
    assert_eq!(
        Quantity::of(i(1), &each()).in_unit(&ppm()).unwrap(),
        i(1_000_000)
    );
    assert_eq!(Quantity::of(i(1), &ppm()).in_unit(&ppb()).unwrap(), i(1000));
    // 1 dozen = 1200% (12 × 100). The dimensionless algebra is exact across counting & ratio.
    assert_eq!(
        Quantity::of(i(1), &dozen()).in_unit(&percent()).unwrap(),
        i(1200)
    );
    // Every counting/ratio unit is dimensionless, and converting to a dimensioned unit is forbidden.
    for u in [
        each(),
        pair(),
        dozen(),
        gross(),
        percent(),
        ppm(),
        basis_point(),
    ] {
        assert_eq!(
            u.dimension,
            Dimension::DIMENSIONLESS,
            "{} is dimensionless",
            u.symbol
        );
        assert!(
            Quantity::of(i(1), &u).in_unit(&metre()).is_none(),
            "{} → m forbidden",
            u.symbol
        );
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
    assert_eq!(
        super::time_quantity_to_nanos(&Quantity::of(i(1), &second())),
        Some(NS)
    );
    assert_eq!(
        super::time_quantity_to_nanos(&Quantity::of(i(1), &millisecond())),
        Some(1_000_000)
    );
    // A non-time quantity is rejected.
    assert_eq!(
        super::time_quantity_to_nanos(&Quantity::of(i(1), &metre())),
        None
    );
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
    let expected = super::time_quantity_to_nanos(
        &super::light_travel_time(&Quantity::of(i(1), &astronomical_unit())).unwrap(),
    );
    assert_eq!(Some(sun), expected);
    // A non-length distance is rejected.
    assert_eq!(
        super::observed_arrival_nanos(0, &Quantity::of(i(1), &kilogram())),
        None
    );
}

#[test]
fn spacetime_stamp_is_space_aware_like_a_timezone_for_position() {
    use super::{CausalRelation, Position, SpacetimeStamp};
    const C: f64 = 299_792_458.0; // metres per second; c·1s = 1 light-second of distance
    let pos = |x: f64| Position { x, y: 0.0, z: 0.0 };
    let at = |ns: i64, x: f64| SpacetimeStamp {
        instant_nanos: ns,
        position: pos(x),
    };
    let here = pos(0.0);
    let one_ls = C; // 1 light-second east

    // Same place → ordered by time alone (zero light separation), like one timezone.
    assert_eq!(
        at(0, 0.0).causal_relation(at(1_000_000_000, 0.0)),
        CausalRelation::Before
    );
    assert_eq!(
        at(1_000_000_000, 0.0).causal_relation(at(0, 0.0)),
        CausalRelation::After
    );
    // 1 light-second apart in space takes ~1 s of light delay (symmetric) — the "offset".
    assert_eq!(
        at(0, 0.0).light_separation_nanos(at(0, one_ls)),
        1_000_000_000
    );
    // 2 s apart but only 1 light-second away → inside the light cone → causally Before.
    assert_eq!(
        at(0, 0.0).causal_relation(at(2_000_000_000, one_ls)),
        CausalRelation::Before
    );
    // Exactly on the light cone (1 s apart, 1 light-second away) → still causally connected.
    assert_eq!(
        at(0, 0.0).causal_relation(at(1_000_000_000, one_ls)),
        CausalRelation::Before
    );
    // Only 0.5 s apart but 1 light-second away → SPACELIKE: genuinely concurrent (CRDT land).
    assert_eq!(
        at(0, 0.0).causal_relation(at(500_000_000, one_ls)),
        CausalRelation::Concurrent
    );
    // Observed-at: an event here is seen 1 s later from 1 light-second away (the relative read).
    assert_eq!(at(0, 0.0).observed_at_nanos(pos(one_ls)), 1_000_000_000);
    // Euclidean distance is honest 3-D (a 3-4-5 triangle → 5).
    assert!(
        (Position {
            x: 3.0,
            y: 4.0,
            z: 0.0
        }
        .distance_to(here)
            - 5.0)
            .abs()
            < 1e-9
    );
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
    assert_eq!(
        super::time_dilation_factor(&Quantity::of(i(1), &metre())),
        None
    );
}

#[test]
fn physical_constants_carry_correct_dimensions_and_compose() {
    use super::constants::*;
    // c is a speed, exactly 299 792 458 m/s, and exact in km/h (× 18/5).
    let c = speed_of_light();
    assert_eq!(c.dimension(), Dimension::speed());
    assert_eq!(c.in_unit(&metre_per_second()).unwrap(), i(299_792_458));
    assert_eq!(
        c.in_unit(&kilometre_per_hour()).unwrap(),
        r(5_396_264_244, 5)
    );
    // E = m·c² type-checks as energy, with exactly 2·c² joules for a 2 kg mass.
    let energy = Quantity::of(i(2), &kilogram()).mul(&c).mul(&c);
    assert_eq!(energy.dimension(), Dimension::energy());
    let c_sq = i(299_792_458).mul(&i(299_792_458));
    assert_eq!(energy.magnitude_si(), &i(2).mul(&c_sq));
    // The gas constant is exactly Avogadro × Boltzmann, dimension J/(mol·K).
    let r_gas = molar_gas_constant();
    assert_eq!(
        r_gas.dimension(),
        Dimension::energy()
            .div(Dimension::amount())
            .div(Dimension::temperature())
    );
    assert_eq!(
        r_gas.magnitude_si(),
        avogadro_constant()
            .mul(&boltzmann_constant())
            .magnitude_si()
    );
    // Each constant's dimension is its defining one.
    assert_eq!(
        planck_constant().dimension(),
        Dimension::energy().mul(Dimension::time())
    ); // action
    assert_eq!(elementary_charge().dimension(), Dimension::charge());
    assert_eq!(
        boltzmann_constant().dimension(),
        Dimension::energy().div(Dimension::temperature())
    );
    assert_eq!(avogadro_constant().dimension(), Dimension::amount().recip());
    assert_eq!(
        gravitational_constant().dimension(),
        Dimension::volume()
            .div(Dimension::mass())
            .div(Dimension::time().powi(2))
    );
    assert_eq!(standard_gravity().dimension(), Dimension::acceleration());
    // Exact defining values: atm = 101 325 Pa, g₀ = 9.806 65 m/s².
    assert_eq!(
        standard_atmosphere().in_unit(&pascal()).unwrap(),
        i(101_325)
    );
    assert_eq!(
        standard_gravity().in_unit(&metre_per_second()).is_none(),
        true
    ); // m/s ≠ m/s²
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
    let (mag, u) = Quantity::of(i(1500), &metre())
        .in_best_unit(&ladder)
        .unwrap();
    assert_eq!((u.symbol, mag), ("km", r(3, 2)));
    // 0.5 km → 500 m (km would be 0.5 < 1, so it drops to metres).
    let (mag, u) = Quantity::of(r(1, 2), &kilometre())
        .in_best_unit(&ladder)
        .unwrap();
    assert_eq!((u.symbol, mag), ("m", i(500)));
    // 0.003 m → 3 mm.
    let (mag, u) = Quantity::of(r(3, 1000), &metre())
        .in_best_unit(&ladder)
        .unwrap();
    assert_eq!((u.symbol, mag), ("mm", i(3)));
    // Exactly 1 km stays 1 km (magnitude == 1 qualifies).
    let (mag, u) = Quantity::of(i(1), &kilometre())
        .in_best_unit(&ladder)
        .unwrap();
    assert_eq!((u.symbol, mag), ("km", i(1)));
    // Below the smallest unit → fall back to the smallest unit (largest magnitude).
    let (mag, u) = Quantity::of(r(1, 10), &millimetre())
        .in_best_unit(&ladder)
        .unwrap();
    assert_eq!((u.symbol, mag), ("mm", r(1, 10)));
    // Negatives scale by absolute magnitude: −2000 m → −2 km.
    let (mag, u) = Quantity::of(i(-2000), &metre())
        .in_best_unit(&ladder)
        .unwrap();
    assert_eq!((u.symbol, mag), ("km", i(-2)));
    // A ladder with no same-dimension unit yields None (the dimensions never match).
    assert!(Quantity::of(i(1), &metre())
        .in_best_unit(&[kilogram(), second()])
        .is_none());
    // The chosen (magnitude, unit) reconstructs the original SI quantity exactly.
    let q = Quantity::of(i(1500), &metre());
    let (mag, u) = q.in_best_unit(&ladder).unwrap();
    assert_eq!(Quantity::of(mag, u).magnitude_si(), q.magnitude_si());
    // Works across other dimensions too: 2_500_000 g → 2.5 t.
    let mass_ladder = [gram(), kilogram(), tonne()];
    let (mag, u) = Quantity::of(i(2_500_000), &gram())
        .in_best_unit(&mass_ladder)
        .unwrap();
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
fn by_name_resolves_the_electrical_family() {
    // Voltage, current, charge, resistance, conductance — full words, plurals, and the
    // unambiguous multi-character symbols.
    for alias in ["volt", "volts"] {
        assert_eq!(by_name(alias), Some(volt()), "{alias}");
    }
    for alias in ["millivolt", "millivolts", "mv"] {
        assert_eq!(by_name(alias), Some(millivolt()), "{alias}");
    }
    for alias in ["kilovolt", "kilovolts", "kv"] {
        assert_eq!(by_name(alias), Some(kilovolt()), "{alias}");
    }
    for alias in ["ampere", "amperes", "amp", "amps"] {
        assert_eq!(by_name(alias), Some(ampere()), "{alias}");
    }
    for alias in ["milliampere", "milliamperes", "milliamp", "milliamps"] {
        assert_eq!(by_name(alias), Some(milliampere()), "{alias}");
    }
    for alias in ["coulomb", "coulombs"] {
        assert_eq!(by_name(alias), Some(coulomb()), "{alias}");
    }
    for alias in ["ampere hour", "ampere hours", "ah"] {
        assert_eq!(by_name(alias), Some(ampere_hour()), "{alias}");
    }
    for alias in ["milliampere hour", "milliampere hours", "mah"] {
        assert_eq!(by_name(alias), Some(milliampere_hour()), "{alias}");
    }
    for alias in ["ohm", "ohms", "ω"] {
        assert_eq!(by_name(alias), Some(ohm()), "{alias}");
    }
    assert_eq!(by_name("millisiemens"), Some(millisiemens()));
    // The bare single letters stay ABSENT by design: "v" and "a" collide with everyday
    // identifiers and the article (the radian/parsec precedent for deliberate absence).
    assert_eq!(by_name("v"), None);
    assert_eq!(by_name("a"), None);
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
    for name in [
        "gray",
        "tesla",
        "poise",
        "dozen",
        "percent",
        "terabyte",
        "pinch",
        "light second",
        "lux",
        "molar",
    ] {
        let u = by_name(name).unwrap();
        let v = r(3, 1);
        assert_eq!(
            Quantity::of(v.clone(), &u).in_unit(&u).unwrap(),
            v,
            "{name}"
        );
    }
}

/// The whole catalog, grouped by dimension — the completeness harness iterates it.
fn catalog() -> Vec<Vec<Unit>> {
    vec![
        vec![
            metre(),
            kilometre(),
            hectometre(),
            dekametre(),
            decimetre(),
            centimetre(),
            millimetre(),
            micrometre(),
            nanometre(),
            picometre(),
            femtometre(),
            megametre(),
            gigametre(),
            angstrom(),
            inch(),
            foot(),
            yard(),
            mile(),
            nautical_mile(),
            fathom(),
            furlong(),
            chain(),
            rod(),
            hand(),
            point(),
            pica(),
            thou(),
            astronomical_unit(),
            light_year(),
            cable(),
            league(),
            nautical_league(),
            light_second(),
            light_minute(),
            light_hour(),
            light_day(),
            lunar_distance(),
            solar_radius(),
            earth_radius(),
        ],
        vec![
            kilogram(),
            gram(),
            milligram(),
            microgram(),
            nanogram(),
            tonne(),
            pound(),
            ounce(),
            stone(),
            short_ton(),
            long_ton(),
            hundredweight(),
            grain(),
            dram(),
            carat(),
            troy_ounce(),
            troy_pound(),
            solar_mass(),
            earth_mass(),
            jupiter_mass(),
        ],
        vec![
            second(),
            millisecond(),
            microsecond(),
            nanosecond(),
            picosecond(),
            minute(),
            hour(),
            day(),
            week(),
            fortnight(),
            julian_year(),
            decade(),
            century(),
        ],
        vec![kelvin(), celsius(), fahrenheit(), rankine(), reaumur()],
        vec![
            cubic_metre(),
            litre(),
            millilitre(),
            centilitre(),
            decilitre(),
            cubic_centimetre(),
            cubic_inch(),
            cubic_foot(),
            us_gallon(),
            us_quart(),
            us_pint(),
            us_cup(),
            us_gill(),
            us_fluid_ounce(),
            tablespoon(),
            teaspoon(),
            oil_barrel(),
            imperial_gallon(),
            imperial_pint(),
            imperial_fluid_ounce(),
            stick_of_butter(),
            dash(),
            pinch(),
            smidgen(),
            bushel(),
            peck(),
            dry_gallon(),
            dry_quart(),
            dry_pint(),
        ],
        vec![
            square_metre(),
            square_kilometre(),
            square_centimetre(),
            square_inch(),
            square_foot(),
            square_yard(),
            square_mile(),
            hectare(),
            are(),
            acre(),
        ],
        vec![
            metre_per_second(),
            kilometre_per_hour(),
            mile_per_hour(),
            foot_per_second(),
            knot(),
        ],
        vec![hertz(), kilohertz(), megahertz(), gigahertz(), rpm()],
        vec![
            newton(),
            kilonewton(),
            dyne(),
            kilogram_force(),
            pound_force(),
        ],
        vec![
            joule(),
            kilojoule(),
            megajoule(),
            gigajoule(),
            terajoule(),
            calorie(),
            kilocalorie(),
            watt_hour(),
            kilowatt_hour(),
            erg(),
            electronvolt(),
            newton_metre(),
            pound_foot(),
        ],
        vec![
            watt(),
            kilowatt(),
            megawatt(),
            gigawatt(),
            terawatt(),
            horsepower(),
        ],
        vec![
            pascal(),
            kilopascal(),
            bar(),
            millibar(),
            atmosphere(),
            psi(),
        ],
        vec![
            bit(),
            byte(),
            kilobit(),
            kilobyte(),
            megabyte(),
            gigabyte(),
            terabit(),
            terabyte(),
            petabyte(),
            kibibyte(),
            mebibyte(),
            gibibyte(),
        ],
        vec![turn(), degree(), gradian(), arcminute(), arcsecond()],
        vec![ampere(), milliampere()],
        vec![coulomb(), ampere_hour(), milliampere_hour()],
        vec![volt(), millivolt(), kilovolt()],
        vec![ohm()],
        vec![mole()],
        vec![candela()],
        // Radiation.
        vec![gray(), sievert(), rad_unit(), rem()], // absorbed/equivalent dose (J/kg)
        vec![becquerel(), curie()],                 // radioactivity (1/s)
        vec![roentgen()],                           // exposure (C/kg)
        vec![katal()],                              // catalytic activity (mol/s)
        // Photometry.
        vec![lumen()],                      // luminous flux (cd·sr)
        vec![lux(), phot(), foot_candle()], // illuminance (lm/m²)
        vec![nit()],                        // luminance (cd/m²)
        // Rates, flow, concentration, viscosity, fuel economy.
        vec![
            bit_per_second(),
            kilobit_per_second(),
            megabit_per_second(),
            gigabit_per_second(),
            byte_per_second(),
            megabyte_per_second(),
        ], // data rate (bit/s)
        vec![
            cubic_metre_per_second(),
            litre_per_second(),
            litre_per_minute(),
            gallon_per_minute(),
            cubic_foot_per_minute(),
        ], // volumetric flow (m³/s)
        vec![molar(), millimolar(), micromolar()], // molar concentration (mol/m³)
        vec![molal()],                             // molality (mol/kg)
        vec![pascal_second(), poise(), centipoise()], // dynamic viscosity (Pa·s)
        vec![square_metre_per_second(), stokes(), centistokes()], // kinematic viscosity (m²/s)
        vec![mile_per_gallon(), km_per_litre()],   // fuel economy (m⁻²)
        // Electromagnetism & surface tension.
        vec![siemens(), millisiemens()], // conductance (S = Ω⁻¹)
        vec![farad(), microfarad(), nanofarad(), picofarad()], // capacitance (F)
        vec![weber(), maxwell()],        // magnetic flux (Wb)
        vec![henry(), millihenry(), microhenry()], // inductance (H)
        vec![tesla(), millitesla(), gauss()], // magnetic flux density (T)
        vec![newton_per_metre(), dyne_per_centimetre()], // surface tension (N/m)
        // Counting & ratio (dimensionless).
        vec![
            each(),
            pair(),
            dozen(),
            baker_dozen(),
            score(),
            gross(),
            great_gross(),
            ream(),
            percent(),
            permille(),
            ppm(),
            ppb(),
            basis_point(),
        ],
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
            assert_eq!(
                Quantity::of(v.clone(), u).in_unit(u).unwrap(),
                v,
                "round-trip {}",
                u.symbol
            );
            // (2) positive scale.
            assert!(
                !u.scale.is_zero() && !u.scale.is_negative(),
                "scale > 0 for {}",
                u.symbol
            );
            // (3) inter-convertible within the dimension.
            for other in group {
                assert!(
                    Quantity::of(v.clone(), u).in_unit(other).is_some(),
                    "{} convertible to {}",
                    u.symbol,
                    other.symbol
                );
                assert_eq!(
                    u.dimension, other.dimension,
                    "{} and {} share a dimension",
                    u.symbol, other.symbol
                );
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
