# UNIVERSAL_TYPES.m

**The Universal Type System for a Language of the Modern Age**

It is absurd that human and physical concepts — seconds, timezones, leap years, metres,
kilograms, money, coordinates, colours, hashes — are left to libraries and floating-point
approximations instead of being *encoded in the type system*. JSON has one number type and it
ruins lives. SQL bolts `INTERVAL` and `TIMESTAMPTZ` on after the fact. Most languages can't tell
you that `2 inches + 5 centimeters` is `42/127` of a foot, exactly, let alone refuse to add a
length to a mass.

Logos already has the substrate to do this right: an **exact numeric tower**
(`Int → BigInt → Rational`, lossless), **first-class temporal values** with calendar-correct
arithmetic, **Word rings** (ℤ/2ⁿ), **CRDTs** with gossip + journal sync, a **kernel proof engine**
that certifies properties, and a **wire codec** with 40+ typed tags, content-addressed schemas,
forward-error-correction, a per-column compression menu, and registry-epoch name elision.

This document specifies the type system as it should have been built the first time. It is a
literate spec: the Logos snippets are runnable and double as compiled doctests.

---

## Table of Contents

- [Part I — Philosophy & the Design Laws](#part-i--philosophy--the-design-laws)
- [Part II — The Number Tower](#part-ii--the-number-tower)
- [Part III — Dimensional Quantities (units of measurement)](#part-iii--dimensional-quantities-units-of-measurement)
- [Part IV — Time (the deepest family)](#part-iv--time-the-deepest-family)
- [Part V — Modern-Age Wire-Native Types](#part-v--modern-age-wire-native-types)
- [Part VI — Blockchain Primitives (seamless, proof-checked)](#part-vi--blockchain-primitives-seamless-proof-checked)
- [Part VII — Algebra Types](#part-vii--algebra-types)
- [Part VIII — Geometry & Shapes](#part-viii--geometry--shapes)
- [Part IX — Wire & Codec Integration](#part-ix--wire--codec-integration)
- [Part X — The Completeness Doctrine](#part-x--the-completeness-doctrine)
- [Part XI — Implementation Roadmap](#part-xi--implementation-roadmap)

Every Part ends with a **Completeness** subsection: the rule that defines "all of them" for that
group, and the coverage tests that mechanically prove it. The doctrine those subsections follow is
stated once, in full, in [Part X](#part-x--the-completeness-doctrine).

---

## Part I — Philosophy & the Design Laws

1. **Human concepts are types, not library structs.** Time, units, money, zones, calendars,
   coordinates, colours, identifiers are first-class, lexed in English-first surface syntax, and
   threaded through the whole Futamura pipeline (tree-walker → bytecode VM → AOT-to-Rust).

2. **Exact by default.** Every magnitude rides the numeric tower (`crates/logicaffeine_base/src/numeric.rs`).
   Because `Rational` is exact, unit/zone/calendar conversions are **lossless**: `1 inch = 127/5000 m`
   *exactly*, never `0.0254…`. `Float` is opt-in, requested, never silent.

3. **"Small then bigger" auto-promotion.** Already true for `Int → BigInt → Rational`. Generalised:
   display auto-scales to the most human unit (`1500 m` shows as `1.5 km`), and any magnitude that
   outgrows `i64` promotes to `BigInt` with no ceremony.

4. **Dimension-checked arithmetic.** Quantities carry a dimension (an exponent vector over base
   dimensions). `+`, `−`, and comparison require *equal* dimensions — otherwise a **compile error**,
   exactly like the existing `Word32`/`Word64` width-mismatch error. `×` and `÷` add/subtract the
   exponent vectors (`Length × Length = Area`).

5. **Affine vs. vector is a universal law.** *Points* (a date, an instant, a temperature reading, a
   coordinate) are distinct from *differences* (a duration, a temperature delta, a displacement):
   `Point − Point = Vector`, `Point + Vector = Point`, `Vector ± Vector = Vector`, and
   `Point + Point` is an error. The temporal types already encode this for time; we **lift it to a
   general law** so every dimension inherits it.

6. **Catalogs are growable DATA, not Rust enums.** Units, timezones, currencies, calendars, named
   colours, and elements live in data tables (JSON / literate `.md`), loaded the way `assets/lexicon.json`
   and the demand-imported stdlib already are. The catalog grows without recompiling the compiler.

7. **Wire-first is a first-class design axis.** Every type has a canonical, content-addressed wire
   form. A whole tier of types — timestamps, UUIDs, hashes, CRDTs, money, blockchain primitives —
   exists *primarily to travel*: they are designed around the codec
   (`crates/logicaffeine_compile/src/concurrency/marshal.rs`), not around in-memory layout.

8. **Proof-checked invariants.** Where it matters — crypto/blockchain laws, dimensional soundness,
   ring/field axioms, conversion round-trips, geometric closure — properties are stated as theorems
   and **kernel-certified** via `verify::prove_certify_check`, not merely unit-tested.

9. **Zero-cost erasure in AOT.** As with `Word32`, the dimension/zone/scale lives only in the static
   type; AOT codegen emits bare numeric-tower arithmetic. The runtime-tagged `Quantity` value is the
   dynamic fallback for the tree-walker and VM — the proven hybrid playbook.

10. **No new crates unless unavoidable; never break green.** New value families slot into the existing
    crates following the Word/temporal playbook. Every wave begins and ends with a green suite
    (`./scripts/run-all-tests-fast.sh`).

11. **Completeness is provable, not asserted.** Every group declares a completeness rule and ships a
    coverage harness that fails the instant a member is missing. We never *claim* coverage — we run
    the test. See [Part X](#part-x--the-completeness-doctrine).

---

## Part II — The Number Tower

The foundation everything else rides on. The tower is **exact upward** and **inexact only on
request**.

| Type | Representation | Role |
|------|----------------|------|
| `Nat` | `u64` | counting; the index/size type |
| `Int` | `i64`, auto-promoting | the default integer; overflows to `BigInt` |
| `BigInt` | sign + base-2⁶⁴ limbs (`numeric.rs`) | arbitrary precision; exact |
| `Rational` | reduced `num/den` over `BigInt` | exact division; the conversion substrate |
| `Decimal` *(new)* | `i128` coefficient + base-10 scale (Rational-backed for unbounded) | money/finance; no binary-float drift |
| `Complex` *(new)* | `(re, im)` over Rational or Float | `√−1`, signal/EE math |
| `Float` | `f64` | opt-in inexact; transcendentals; never silent |
| `Word8/16/32/64` | wrapping `uN` (`word.rs`) | ℤ/2ⁿ rings; crypto/bit substrate |
| `Modular<n>` *(new)* | residue mod arbitrary `n` | number theory / crypto; generalises Word |

**Promotion lattice** (the "small then bigger" law made precise):

```
Int  --overflow-->  BigInt  --exact ÷-->  Rational  --base-10 scale-->  Decimal
                                              |                              |
                                              +--------- √negative / i ------+--> Complex
                                              |
                                              +--------- explicit / transcendental --> Float
Word*/Modular<n> stay in their ring (wrapping); never auto-promote.
```

`add`, `subtract`, `multiply`, `divide` in `crates/logicaffeine_compile/src/semantics/arith.rs`
already implement the `Int/BigInt/Rational/Float` arms; Wave 1 adds `Decimal`, `Complex`, `Modular`
following the same exhaustive-match shape.

```logos
Let big be 9223372036854775807 + 1.   # i64::MAX + 1 → BigInt, exact
Let exact be 1 / 3 + 1 / 3 + 1 / 3.    # → 1 exactly, not 0.999…
Show exact.                            # 1
```

### Completeness — Part II
- **Closed group.** The tower's members are an exhaustive enum; `arith.rs` and the wire codec must
  match all of them (compile-time totality). A test asserts every numeric variant participates in
  every binary op and in the promotion lattice (no missing cross-tier arm).
- **Law tests (to absurdity):** field/ring axioms fuzz-checked per tier (commutativity, associativity,
  distributivity, identity, inverse); promotion is order-independent; `Decimal` arithmetic equals the
  Rational oracle; `Complex` obeys `i² = −1`; `Modular<n>` matches `Word` where `n = 2ᵏ`.

---

## Part III — Dimensional Quantities (units of measurement)

A **quantity** is a magnitude (on the number tower) tagged with a **dimension**. The dimension is an
exponent vector over base axes; arithmetic is dimensional algebra on the exponents and tower
arithmetic on the magnitude — fully decoupled, which is what makes conversion lossless.

**Base dimensions** (the SI seven + tracked extensions):

`Length (L)`, `Mass (M)`, `Time (T)`, `Current (I)`, `Temperature (Θ)`, `Amount (N)`,
`Luminous (J)` — plus `Angle`, `SolidAngle`, `Information` (SI-dimensionless but tracked to catch
mix-ups), and the all-zero vector `Dimensionless`. Exponents are rational (admits `V/√Hz`); the
representation is `Copy + Eq + Hash`, small enough to live inside `LogosType` and to tag a runtime value.

```rust
// crates/logicaffeine_base/src/dimension.rs (Wave 2)
pub enum BaseDim { Length, Mass, Time, Current, Temperature, Amount, Luminous,
                   Angle, SolidAngle, Information }            // COUNT = 10
pub struct Exp { num: i8, den: i8 }                            // rational exponent, reduced
pub struct Dimension { e: [Exp; BaseDim::COUNT] }              // Copy + Eq + Hash
```

**The arithmetic laws:**
- `Quantity(d) ± Quantity(d) = Quantity(d)`; `Quantity(d) ± Quantity(e≠d)` is a **type error**.
- `Quantity(d) × Quantity(e) = Quantity(d·e)` (exponent add); collapses to a plain number when the
  result is dimensionless.
- `Quantity(d) ÷ Quantity(e) = Quantity(d/e)` (exponent subtract).
- scalar `× Quantity(d) = Quantity(d)`.
- comparison requires equal dimensions; ordering is the tower's exact ordering on the SI-normalised
  magnitude.

**Surface syntax.** English unit words after a count (the lexer already turns `28-inch` into
`28 inch` and `5 hours` into a temporal literal); conversion via `in <unit>` / `as <unit>`; display
auto-scales unless a unit is pinned.

```logos
Let d be 2 inches + 5 centimeters.
Show d.            # 63/625 m  (auto-scaled, exact)
Show d in feet.    # 42/127 ft (exact rational — the golden proof)

Let v be 100 kilometers / 2 hours.
Show v in mph.     # exact speed; dimension = Length / Time
```

### The Full Unit Catalog

Both imperial and metric/scientific on **everything**. Stored as data in `assets/units.json`; each
row is `{ id, symbol, aliases/plurals, dim, scale (exact Rational to SI base), offset, prefixes, kind }`.

**SI prefixes (the official BIPM set, applied to any metric base unit):**
quecto(1e-30), ronto, yocto, zepto, atto, femto, pico, nano, micro, milli, centi, deci, **(base)**,
deca, hecto, kilo, mega, giga, tera, peta, exa, zetta, yotta, ronna, quetta(1e30).
**Binary prefixes (for `Information`):** kibi, mebi, gibi, tebi, pebi, exbi, zebi, yobi (2¹⁰…2⁸⁰).

- **Length (L):** metre (+all SI prefixes), inch, foot, yard, mile, nautical mile, fathom, furlong,
  chain, rod/perch, league, ångström, micron, astronomical unit, light-year, parsec, point, pica,
  em, hand, mil/thou, cubit.
- **Mass (M):** gram (+SI), tonne, pound, ounce, stone, ton (short/long/metric), grain, dram, slug,
  carat, hundredweight, dalton/u, troy ounce, troy pound, pennyweight.
- **Time (T) as a measurable quantity:** second (+SI), minute, hour, day, week, fortnight,
  decade, century, millennium, Julian year, sidereal day, jiffy, shake, Planck time. *(Calendar
  month/year are **not** here — they have no exact scale to seconds; they live in [Part IV](#part-iv--time-the-deepest-family) as `Span`.)*
- **Temperature (Θ, affine):** kelvin, Celsius, Fahrenheit, Rankine, Réaumur.
- **Current (I):** ampere (+SI). **Amount (N):** mole. **Luminous intensity (J):** candela.
- **Angle:** radian, degree, arcminute, arcsecond, gradian/gon, turn/revolution, NATO mil.
  **Solid angle:** steradian, square degree.
- **Area (L²):** square metre, square km, hectare, are, acre, square foot/inch/mile/yard, barn.
- **Volume (L³):** cubic metre, litre (+ml/cl/dl/kl), cc, gallon (US/imperial), quart, pint, cup,
  fluid ounce, tablespoon, teaspoon, barrel (oil/dry), bushel, peck, gill, hogshead, cord,
  board-foot, cubic inch/foot/yard, acre-foot, minim, drop.
- **Speed (L/T):** m/s, km/h, mph, knot, mach, ft/s, c (speed of light).
- **Acceleration (L/T²):** m/s², standard gravity g₀, gal.
- **Frequency (1/T):** hertz (+kHz/MHz/GHz/THz), rpm, bpm, becquerel.
- **Force (M·L/T²):** newton, dyne, pound-force, kilogram-force, poundal, kip, ton-force.
- **Pressure (M/L/T²):** pascal (+kPa/MPa/GPa), bar, millibar, atmosphere, torr, mmHg, psi, inHg,
  inH₂O, barye.
- **Energy (M·L²/T²):** joule (+kJ/MJ/GJ), calorie, kilocalorie, electronvolt (eV…TeV), BTU, kWh,
  Wh, erg, foot-pound, therm, ton-of-TNT, hartree, rydberg, quad.
- **Power (M·L²/T³):** watt (+kW/MW/GW), horsepower (mechanical/metric/electrical), BTU/h,
  ton-of-refrigeration, erg/s, volt-ampere, var.
- **Electrical:** coulomb, ampere-hour/mAh (charge); volt (potential); ohm (resistance); siemens
  (conductance); farad/µF/nF/pF (capacitance); henry (inductance); weber, maxwell (flux); tesla,
  gauss (flux density); ampere-turn, gilbert (mmf); ampere/metre, oersted (field strength).
- **Photometry:** lux, foot-candle, phot (illuminance); lumen (flux); nit (cd/m²), stilb, lambert,
  foot-lambert (luminance).
- **Ionising radiation:** gray (absorbed dose), sievert (equivalent dose), rad, rem, roentgen,
  becquerel, curie. **Catalytic activity:** katal.
- **Information:** bit, byte, nibble; decimal bit/byte (kb…Eb, kB…EB); binary (KiB…YiB);
  data rate bps/kbps/Mbps/Gbps and B/s.
- **Dimensionless counts & ratios:** dozen, gross, score, baker's dozen, ream, pair; percent,
  per-mille, ppm, ppb, basis point.
- **Compound / derived:** density (kg/m³, g/cc, lb/ft³, specific gravity); flow (m³/s, L/s, gpm,
  cfm, sverdrup); torque (N·m, lb·ft); dynamic & kinematic viscosity (Pa·s, poise; m²/s, stokes);
  surface tension (N/m); concentration (molar, molal); fuel economy (mpg US/imperial, L/100km, km/L).
- **Logarithmic (special — convert & display only; see below):** decibel, bel, neper, pH, Richter,
  octave, musical cent, stellar magnitude.
- **Physical constants as named quantities:** c, h, ℏ, G, e (elementary charge), k_B, N_A, R,
  electron mass mₑ, proton mass m_p, ε₀, µ₀, fine-structure α, standard gravity g₀, standard atmosphere.
- **(Optional chemistry layer)** periodic-table element molar masses → mole/dalton support.

**Affine units.** Temperature carries an offset, so a *point* converts with scale **and** offset
while a *difference* converts with scale only:

```logos
Let t be 20 Celsius.
Show t in fahrenheit.       # 68 °F  (point: ×9/5 then +32, exact)
Let rise be 20 Celsius as a difference.
Show rise in fahrenheit.    # 36 °F  (vector: ×9/5 only)
```

**Logarithmic units** are *not* `(dimension, scale, offset)` and `dB + dB ≠` magnitude addition.
They are modelled as a distinct `kind = Logarithmic { base, factor, ref }`, support conversion and
display, and make `dB + dB` a **deliberate type error** that directs you to convert to the linear
(power) domain, add, and convert back. Silently doing linear addition would be wrong; the boundary
is documented, not cut.

### Completeness — Part III
- **Closed sub-structure:** `BaseDim` is an exhaustive enum; the SI prefix set and binary prefix set
  are fixed lists with **count assertions** against the BIPM list. Affine vs. linear vs. logarithmic
  is an exhaustive `kind`.
- **Growable catalog (units.json) coverage harness — every row is iterated and must satisfy:**
  *round-trip lossless* (`x → SI → x` is identity over Rational), *dimension consistency* (declared
  dimension equals its base unit's), *alias unambiguity* (no surface word maps to two meanings),
  *display round-trip* (`Showable` output re-parses), and *wire round-trip + reorder/FEC fuzz*.
- **Cross-catalog closure:** every `BaseDim` has ≥1 base unit; every prefix family applies only to
  metric bases; every derived unit's dimension is reachable by `×`/`÷` from base units.
- **Algebra laws:** fuzz `Dimension::{mul,div,powi}` for group laws; `Length × Length = Area` and
  `Energy = Mass · Length² / Time²` checked symbolically against the catalog's declared dimensions.

---

## Part IV — Time (the deepest family)

Time is its own entire type *with types around it*. It is the one dimension that carries timezones,
DST, leap seconds, multiple calendars, relativity, and distributed causality. The organising idea:
**every time value is a lens over an absolute coordinate**, and operations are transforms between
lenses.

### IV.1 — The Time-Scale Tower

| Scale | What it is | Notes |
|-------|-----------|-------|
| `Monotonic` | steady elapsed clock | measurement only; never wall-clock |
| `SmoothUTC` *(default)* | nanos since epoch, **no** leap seconds | every minute = 60 s; matches Postgres/Java/Temporal. The leap second is being abolished by 2035 — smooth is the forward-looking default |
| `LeapUTC` | leap-second-aware UTC | permits `23:59:60`; backed by a growable leap-second table |
| `TAI` | International Atomic Time | no leaps; `TAI − UTC` from the leap table |
| `TT` | Terrestrial Time | `TAI + 32.184 s` |
| `TCG` | Geocentric Coordinate Time | relativistic rate vs. TT |
| `TCB` / `TDB` | Barycentric (solar-system) | the IAU scales for ephemerides |
| `ProperTime` | clock on a worldline | special + general relativistic dilation from velocity & potential |

These are the **real IAU time scales** — not invented. Conversions between them are exact where the
defining relation is exact (TAI↔TT) and table-driven where empirical (UTC↔TAI leaps).

### IV.2 — Timestamp Kinds (the "many kinds of timestamps")

`Instant` (TIMESTAMP — `SmoothUTC` nanos), `Zoned` (TIMESTAMPTZ — `Instant` + IANA zone),
`GeoStamped` (`Instant` + coordinate + stamper's zone — self-describing on arrival), `Local`
(naive wall clock, no zone), `Date`, `Time`, `YearMonth`, `MonthDay`, `Year`, `Quarter`, `Week`,
`Duration` (exact physical elapsed), `Span`/`Period` (calendar months + days — incommensurable with
seconds), `Interval` / `Range<Instant>`, and the **distributed/wire-native clocks** `Lamport`,
`HybridLogicalClock`, `VectorClock`.

The affine/vector law (Design Law 5) governs all of them:
`Instant − Instant = Duration`, `Instant + Duration = Instant`, `Zoned + Duration = Zoned`,
`Date − Date = Span`, `Instant + Instant` is an error.

### IV.3 — Timezones (full IANA, crushed)

Truth is the immutable UTC `Instant`; the zone is *presentation*. We embed the **complete IANA tz
database** and crush it with our own codec: transition timestamps as delta-of-delta, abbreviations
dictionaried, the open-ended future as a POSIX tail rule. A `Zoned` value carries `(utc_nanos, zone)`;
on the wire the zone travels as the interned IANA string, but when two peers share a **tz-epoch**
(the same registry-handshake mechanism the codec already uses to elide struct-schema names) the zone
is sent as a 2-byte id — names elided entirely.

DST-aware arithmetic falls straight out of the existing `Span` vs. `Duration` split:
- `Zoned + Duration` is **physical** — true elapsed time, ignores DST.
- `Zoned + Span` is **civil** — decompose to wall-clock, add in civil space, re-resolve the offset;
  "one day later" preserves wall time across a DST boundary.
- gap (spring-forward) and fold (fall-back) ambiguities resolved by `Fold::{Earlier, Later, Reject}`.

```logos
Let m be 2026-03-08 1:30am in "America/New_York".
Show m in "Asia/Tokyo".     # same instant, Tokyo wall clock
Let later be m + 1 day.     # civil: 2026-03-09 1:30am NY (DST-correct, 23 physical hours)
```

### IV.4 — Calendars & Calendar Units (better calendar units; leap years)

Calendar units: second, minute, hour, day, **week, fortnight, month, quarter, year, decade,
century, millennium**. Leap years are handled by Howard Hinnant's proleptic Gregorian algorithms
(already in `semantics/temporal.rs`, lifted to `base/temporal/gregorian.rs`): `Jan 31 + 1 month →
Feb 28/29` clamps correctly, leap years and all.

Calendar **systems** (the "everything" tier), each a `CalendarSystem` lens over the absolute day
count: **Gregorian**, **ISO-8601 week-date**, **Julian**, and the lunisolar/lunar systems
**Hebrew**, **Islamic/Hijri**, **Chinese** — the last three data-driven over embedded month tables.
Business-day and holiday math; holidays as a `SharedSet of Date` CRDT for collaborative org calendars.

### IV.5 — Distributed Time & the CRDT Clock-Sync Keystone

**Conceptual unification:** the relativistic **light cone is a causality DAG**, and **spacelike
separation is exactly CRDT concurrency**. Two events outside each other's light cones have no shared
"now"; CRDTs are precisely the model for genuinely-concurrent events — no total order required, only
a commutative/associative/idempotent merge. Distributed time *is* a CRDT problem, and at
interplanetary scale it becomes the obvious one.

**`SyncClock` — a CRDT-backed, light-cone-aware Hybrid Logical Clock.**

```rust
struct SyncClock {
    proper_time: i128,                       // this node's worldline clock (nanos)
    hlc_logical: u64,                        // tie-breaker for same-instant events
    knowledge: Map<NodeId, EstimateInterval> // bounded belief about each remote clock
}
struct EstimateInterval { lower: i128, upper: i128 }  // [t_min, t_max] of a remote clock NOW
```

- The interval for a remote node is derived from its last received stamp + locally-elapsed proper
  time + the light-delay bound (`distance / c`). You **cannot** know a remote clock newer than
  `now − distance/c`; the uncertainty is the physics, encoded — not sloppiness.
- **Merge is conflict-free by construction:** HLC components combine by `max` (monotone), knowledge
  intervals by tightest-consistent intersection (an interval CRDT). All three CRDT laws hold, so it
  rides the existing `Merge` trait + gossip mesh + journal **unchanged**.
- **Convergence:** like NTP offset/delay estimation but CRDT-style and relativity-aware — gossip
  round-trips tighten the intervals; the proper-time model (IV.6) corrects for clocks ticking at
  different rates. Clocks never agree *instantaneously* (forbidden by physics), but stay
  **monotonic, conflict-free, and eventually convergent**, with bounded, shrinking uncertainty.
- **Fixes LWW's clock-skew bug at the relativistic limit:** naive `LastWriteWins` is *wrong* when
  there is no shared now. `SyncClock` stamps make "last" mean *causally* last; genuinely
  spacelike-concurrent writes are detected and kept (degrading to `MVRegister`) instead of one
  silently clobbering the other. Any existing CRDT can opt into light-cone-correct timestamps.

### IV.6 — Relativistic / Universal Time (space travel)

`SpacetimeStamp = (coordinate time, reference frame, position)` with Lorentz frame transforms,
proper-time elapsed along a worldline (special + general relativistic dilation), and **light-delay
attenuation**: convert "event time at A" to "observed time at B" by adding `distance / c`
(Earth↔Mars is 3–22 minutes). This reuses the units system (`Length`, `Speed`, the constant `c`) and
the affine/vector law — a `SpacetimeStamp` is a point; the delay is a vector.

```logos
Let launch be now at Earth.
Let seen be launch observed from Mars.   # + light-delay (distance / c)
Show seen − launch in minutes.           # the one-way signal delay, exact-ish
```

### Completeness — Part IV
- **Closed enums:** time scales, timestamp kinds, calendar systems, and `Fold` are exhaustive enums
  with count assertions; every arithmetic combination is a total match (affine/vector law table is
  exhaustive — every `(kind, op, kind)` triple either has a rule or is an explicit typed error).
- **Growable catalogs:** **every IANA zone id** in the embedded tzdata must resolve, every
  transition list must be monotonic, and a parity test asserts our zone-set equals the source
  tzdata release. The leap-second table is checked monotonic and matched to the IERS bulletin set.
- **Behavioural coverage:** DST round-trips for every zone at its known transitions; `Span` vs.
  `Duration` arithmetic across DST; Gregorian↔Julian divergence at known dates; ISO-week of known
  dates; **`SyncClock` convergence** under simulated light-delay regardless of merge order, plus the
  spacelike-concurrent-write-is-a-conflict test; light-delay attenuation against published Earth↔Mars
  figures. The existing green temporal suite is the regression net for the lift-and-shift.

---

## Part V — Modern-Age Wire-Native Types

Types whose primary purpose is to travel, merge, and be content-addressed. Each specifies its
**canonical wire form first**.

- **`Money` = `{ amount: Decimal, currency: ISO-4217 }`.** Exact arithmetic; never float-drifts on
  the wire (`T_MONEY`). Conversion goes through a `Shared` **CRDT rate table**
  (`SharedMap from Text to LastWriteWins of Decimal`, `Sync`'d + `Mount`'d) — live rates that
  converge across replicas using the same machinery as the collaborative calendar.
- **`Uuid` = 16 bytes (`T_UUID`).** v4 (random) and v7 (time-ordered, so a column of them
  delta-compresses). The canonical key for CRDT maps and Merkle stores.
- **`Range<T: Ord>`** — `contains` / `overlaps` / `adjacent`. `Range<Instant>` is the scheduling /
  double-booking primitive; numeric and date ranges mirror Postgres range types.
- **Network:** `IpAddr` (v4/v6), `Cidr`, `MacAddr`, `Url`, `Port`, `Asn` — extends `assets/std/net.md`.
- **Geo:** `Coordinate { lat: Angle, lon: Angle, alt: Length }`, geohash, great-circle distance —
  ties directly to the units system's `Angle` and `Length`, and to `GeoStamped` in Part IV.
- **Color:** one `Color` value with RGB/RGBA/HSL/HSV/CMYK/Lab/XYZ representations, exact conversions,
  and a named-colour catalog (data).

```logos
Let price be $19.99 USD.
Show price + price * 8 percent.       # exact Decimal; no 0.01 drift
Let slot be 9:00am to 9:30am on 2026-03-09.
Show slot overlaps standup's window.  # Range<Instant> overlap
```

### Completeness — Part V
- Growable catalogs (ISO-4217 currencies, named colours) iterate every row: round-trip, wire
  round-trip, and **external parity** (currency set vs. ISO-4217; colour names vs. the CSS/X11 list).
- Closed: UUID versions are an exhaustive enum; colour spaces an exhaustive enum with pairwise
  conversion round-trip tests; `Range` relation algebra (`contains`/`overlaps`/`adjacent`) fuzzed
  for the standard interval laws.

---

## Part VI — Blockchain Primitives (seamless, proof-checked)

Build the crypto substrate first (greenfield), then the ledger types, reusing the distributed
substrate so a chain is *easy* to stand up.

**Crypto substrate** (authored over the existing `Word32`/`Word64` rings, the same way ChaCha20
already is in `assets/std/crypto.lg`): hashes **SHA-256, SHA-3/Keccak, Blake3**; signatures
**Ed25519, ECDSA/secp256k1, Schnorr**; **HMAC, HKDF, CSPRNG**.

**Ledger types:** `Hash`/`Digest` (algorithm-tagged, multihash-style, `T_HASH`), `MerkleTree` +
`MerkleProof`, `HashChain`, `KeyPair`/`PublicKey`/`PrivateKey`, `Address`, `Signature`,
`Transaction`, `Block`, `Ledger`, `Chain`, and consensus primitives (PoW target, PoS stake, BFT vote).

**Seamless = reuse the substrate.** A chain is a **hash-linked append-only journal** (extending
`crates/logicaffeine_system/src/distributed.rs`), gossiped over the existing mesh
(`network/mesh.rs`), content-addressed via the existing schema fingerprinting, erasure-coded via
existing FEC. A wallet is a `Shared` struct; `Sync` and `Mount` already wire replication + persistence.

**Proof-checked = the laws are theorems.** Merkle-inclusion soundness, chain-append immutability,
signature-verification correctness, and no-double-spend are stated as `ProofExpr` goals and
**kernel-certified** by `verify::prove_certify_check` — the same engine that certifies the hardware/SVA
properties.

```logos
A Wallet is Shared and has:
    its ledger, which is a Chain.

Sign tx with key.
Append block to chain.
Show verify chain.      # kernel-checked invariants hold
```

### Completeness — Part VI
- Crypto: each primitive validated against published **RFC/NIST test vectors** (SHA-256 FIPS-180,
  Ed25519 RFC 8032, etc.) — vector coverage is the completeness rule.
- Ledger: the invariant set is an enumerated list of theorems, each of which must return
  `verified == true` from the kernel; a tamper test must flip each to false. Hash/signature algorithm
  ids are an exhaustive enum with wire round-trip.

---

## Part VII — Algebra Types

- **Containers:** `Vector<T, N>`, `Matrix<T, R, C>`, `Quaternion`, `Tensor`, `Polynomial<T>`,
  `Interval<T>` (interval arithmetic), `Modular<n>`. Over the Rational tower these are **exact**.
- **Operations:** dot/cross/norm, matrix multiply/transpose/determinant/inverse, polynomial
  evaluation/roots, quaternion rotation.
- **Algebraic structures, kernel-backed and provable:** `Monoid`, `Group`, `Ring`, `Field`,
  `VectorSpace`, `Module`, `Order`. Expressed as dependent records over the kernel's existing ring
  and order axioms (`crates/logicaffeine_kernel/src/prelude.rs`, `ring.rs`); an **instance carries
  proof obligations** (associativity, identity, inverse, distributivity) that the engine checks.
  This is what makes "types for algebra" real rather than nominal — declaring `ℤ is a Ring` makes you
  *prove* it.

### Completeness — Part VII
- Structure hierarchy is a closed enum (`Monoid ⊂ Group ⊂ Ring ⊂ Field`); each instance must
  discharge **every** axiom of its claimed structure (the axiom list per structure is the rule);
  operations fuzzed against a reference (matrix multiply vs. naive triple loop; polynomial roots
  re-evaluated to ~0). A test asserts every declared structure instance certifies.

---

## Part VIII — Geometry & Shapes

- **Primitives:** `Point2`/`Point3`, `Vector2`/`Vector3` (tie to units — a displacement is a
  `Length` vector), `Angle`, coordinate systems (Cartesian/polar/spherical/cylindrical),
  `Transform` (affine matrix / quaternion).
- **2D:** `Segment`, `Ray`, `Line`, `Polyline`, `Polygon`, `Triangle`, `Rectangle`, `Circle`,
  `Ellipse`, conics; measures length/area/perimeter/centroid.
- **3D:** `Plane`, `Sphere`, `Cylinder`, `Cone`, `Polyhedron`; measures surface-area/volume.
- **Regular polyhedra → the 5 Platonic solids:** `Tetrahedron`, `Cube`, `Octahedron`,
  `Dodecahedron`, `Icosahedron`, with exact/algebraic vertex coordinates where possible, then the
  Archimedean solids and higher regular polytopes as the growable tail.
- **Predicates:** containment, intersection, convexity — connected to the Tarski geometry axioms
  already in `crates/logicaffeine_proof/tests/tarski_geometry.rs` for kernel-certified facts.

### Completeness — Part VIII
- **Closed with a closure theorem:** there are *exactly* 5 Platonic solids and *exactly* 4
  non-degenerate conic sections — stated and **kernel-proved**, not merely listed. `assert_eq!(PlatonicSolid::ALL.len(), 5)`.
- **Invariant coverage:** every polyhedron satisfies Euler `V − E + F = 2` (proved); every solid's
  vertex/edge/face counts and exact measures are pinned; coordinate-system conversions round-trip.

---

## Part IX — Wire & Codec Integration

The contract everything serialises against. The codec
(`crates/logicaffeine_compile/src/concurrency/marshal.rs`) already provides typed tags, content-addressed
schemas (`WireSchemaCache`), per-column compression (delta / delta-of-delta / frame-of-reference /
RLE / dictionary / affine), FEC erasure (`concurrency/fec.rs`), and registry-epoch name elision.

Adding a value kind, end to end:
1. allocate a tag in `marshal.rs` (next free is `T_* = 56+`);
2. add encode/decode arms at the single dispatch site;
3. register struct/enum schemas in `WireTypeRegistry` (canonical, fingerprinted);
4. add a `RtPayload` variant in `crates/logicaffeine_runtime/src/payload.rs`.

New tags this spec introduces: `T_DECIMAL`, `T_COMPLEX`, `T_QUANTITY`, `T_ZONED`, `T_GEOSTAMP`,
`T_TAI`, `T_HLC`, `T_MONEY`, `T_UUID`, `T_RANGE`, `T_IP`, `T_COORD`, `T_COLOR`, `T_HASH`, `T_SIG`.

**Codec-invisible catalogs.** Timezones, units, and currencies each get an **epoch handshake**: when
two peers agree on the catalog version, catalog members travel as small ids and the names are elided
— exactly the struct-schema epoch mechanism, generalised. tzdata, the unit table, and the currency
table are themselves compressed with the per-column menu.

Streaming is inherent (per-value tagging); FEC and compression dials apply to every new type unchanged.

### Completeness — Part IX
- **The tag space is a closed group:** a test asserts every `RuntimeValue` variant and every
  `RtPayload` variant has exactly one wire tag and a registered encode+decode (no value type can
  exist without a wire form). Tag ids are unique and contiguous.
- **Per-type wire conformance** (part of the universal harness in Part X): encode→decode identity,
  interleave-and-reorder resilience, FEC reconstruct-from-any-k, and byte-identical encodings across
  peers (content addressing) — fuzzed against the scalar oracle.

---

## Part X — The Completeness Doctrine

How we *know* a group is fully covered. We never claim coverage; we run the harness that fails the
instant a member is missing. Groups fall into two regimes, plus one universal harness.

### A. Closed groups — finite and provable
Membership is mathematically fixed (SI base dimensions = 7; SI prefixes = 24; non-degenerate conics
= 4; Platonic solids = 5; regular 4-polytopes = 6; crystal systems = 7; Bravais lattices = 14; the
codec tag space).
- **Exhaustive enum + total match.** Model the group as a Rust enum and lean on the compiler's
  exhaustiveness check as a *compile-time* completeness proof — a new variant won't build until every
  site handles it. (This is how `RuntimeValue` and wire-tag dispatch already stay honest.)
- **Count assertion.** A test pins the cardinality (`assert_eq!(GROUP::ALL.len(), N)`); silent
  additions/removals fail.
- **Closure theorem where one exists.** "Exactly 5 Platonic solids", "exactly 4 non-degenerate
  conics", Euler `V − E + F = 2`, the field axioms — stated as `ProofExpr` goals and kernel-certified
  via `verify::prove_certify_check`. The proof *is* the completeness guarantee.

### B. Growable groups — open data catalogs
Units, currencies, IANA timezones, named colours, calendars, elements. The catalog is the single
source of truth; coverage is enforced by harnesses that iterate **every row**:
- **Enumerate-the-catalog tests** — no hand-maintained list that can drift from the data.
- **Per-row conformance:** round-trip lossless (under the exact Rational tower), dimension/scale
  consistency, alias unambiguity (no surface word maps to two meanings), wire round-trip
  (+ reorder/FEC fuzz), display round-trip (`Showable` → re-parse identity).
- **Cross-catalog closure rules:** every `BaseDim` has a base unit; every prefix family applies to
  its declared bases; every currency has a Decimal scale; every IANA zone resolves with monotonic
  transitions.
- **External-reference parity:** assert our row-set matches the authoritative source — IANA tzdata
  release, BIPM prefix list, ISO-4217 currencies, CSS named colours, NIST/RFC crypto vectors.
  Missing/extra rows fail loudly; a **silent cap is forbidden** (`log`/assert what was dropped).

### C. The universal type-conformance harness
One parametric suite every value type must pass, in every group: wire encode/decode identity;
`Showable` ↔ parse identity; arithmetic obeys the affine/vector + dimensional laws (or is a typed
error); ordering total iff claimed; `Merge` laws (commutative/associative/idempotent) for CRDT types;
proof obligations certified for algebraic-structure instances. **Registration is itself checked:** a
test asserts every `RuntimeValue`/wire variant appears in the conformance registry, so a new type
*cannot* be added without conformance.

Each implementation wave's RED set includes the relevant coverage harness; a wave is not complete
until its group's completeness rule is mechanically green.

---

## Part XI — Implementation Roadmap

TDD, RED first; never weaken a RED test; never advance with a red suite. Golden e2e tests run across
**all three tiers** (tree-walker, VM, AOT) for parity in `crates/logicaffeine_tests/tests/`. Run
`./scripts/run-all-tests-fast.sh` at every wave boundary; it must stay green, and
`./scripts/compare-test-runs.sh` must show parity with the cargo baseline.

| Wave | Deliverable | Golden / coverage RED |
|------|-------------|-----------------------|
| **0** | This document | spec complete; doctests compile |
| **1** | Number tower: `Decimal`, `Complex`, `Modular` | exact money; `√−1 = i`; `Decimal` no-drift; cross-tier law fuzz |
| **2** | Dimensional core | `Show (2 inches + 5 centimeters in feet) = 42/127 ft` on all tiers; `Length + Mass` is a compile **and** runtime error |
| **3** | Full unit catalog (`units.json`) | `×`/`÷` algebra; `20 °C in fahrenheit = 68 °F`; catalog coverage harness green; prefix parity vs BIPM |
| **4** | Earth time: zones/DST/calendars | DST round-trips per zone; zone elision over wire; Gregorian/Julian/ISO-week; subsume linear `Duration`/`Moment` (green temporal suite as net) |
| **5** | Deep time + `SyncClock` | TAI−UTC at known dates; Earth↔Mars light-delay; lunisolar calendars; `SyncClock` converges under simulated delay, order-independent; spacelike write = conflict |
| **6** | Wire integration sweep | every new tag: round-trip + reorder + FEC fuzz vs oracle; tzdata crushed-size measured; tag-space completeness test |
| **7** | Money/UUID/Range/Network/Geo/Color | double-booking via `Range<Instant>`; money convert via Sync'd rate table; geo great-circle; catalog parity (ISO-4217, CSS colours) |
| **8** | Crypto + blockchain | RFC/NIST vectors; Merkle inclusion certifies; tamper breaks verify; no-double-spend theorem passes the kernel |
| **9** | Algebra types | matrix laws exact; a `Group`/`Ring` instance's axioms certify; structure-hierarchy coverage |
| **10** | Geometry + Platonic solids | exactly-5 closure theorem; Euler `V−E+F=2` proved; exact solid measures |

### Critical files (reusing the Word / temporal / codec playbooks)

New homes (lowest pure layer, shared by interpreter + AOT):
- `crates/logicaffeine_base/src/dimension.rs` — dimension model (reuses `base::Rational`).
- `crates/logicaffeine_base/src/temporal/{gregorian,tz,tzdata,calendar}.rs` — lifted Hinnant + zones + calendars.
- `crates/logicaffeine_base/src/numeric.rs` — add `Decimal`, `Complex`, `Modular`.
- `crates/logicaffeine_compile/src/semantics/units.rs` — normalize / convert / auto-scale / logarithmic.
- `crates/logicaffeine_*/assets/units.json` + `assets/std/{units,timezone,calendar,money,geo,crypto}.md`.

Threading touchpoints (the "add a first-class type" checklist):
- Lexer/tokens/AST: `crates/logicaffeine_language/src/{lexer.rs, token.rs, ast/stmt.rs, parser/mod.rs}`.
- Static type: `crates/logicaffeine_compile/src/analysis/{types.rs, check.rs, unify.rs, registry.rs}`.
- Runtime: `crates/logicaffeine_compile/src/interpreter.rs` + `semantics/{arith.rs, compare.rs}`.
- VM: `crates/logicaffeine_compile/src/vm/nanbox.rs`. AOT: `crates/logicaffeine_compile/src/codegen/{types.rs, expr.rs}`.
- Display: `crates/logicaffeine_system/src/{units_rt.rs, temporal.rs, io.rs}` (`Showable`).
- Wire: `crates/logicaffeine_compile/src/concurrency/marshal.rs` + `crates/logicaffeine_runtime/src/payload.rs`.
- Proof: `crates/logicaffeine_proof/src/verify.rs` (`prove_certify_check`).
- Distributed: `crates/logicaffeine_system/src/distributed.rs`, `network/mesh.rs`, `relay_proto.rs`.

### Open follow-ups (decide during execution, not blockers)
- `Decimal` backing: `i128`+scale vs. Rational — benchmark in Wave 1.
- IANA tzdata ingestion: build-time compile step vs. committed crushed artifact — Wave 4; must be
  reproducible and tiny.
- Relativistic fidelity ceiling: which IAU scales + which GR potential model — Wave 5.
