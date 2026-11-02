//! Exact calendar primitives — the bedrock of the time tower.
//!
//! Everything here is **exact integer arithmetic** over a single absolute coordinate: the day
//! count since the Unix epoch (1970-01-01). Two calendars are projected onto that coordinate —
//! the **proleptic Gregorian** calendar (the civil default) and the **proleptic Julian** calendar
//! (for historical dates and the Gregorian/Julian divergence) — so a date in either calendar is
//! just a different *lens* on the same day number, and converting between them is lossless.
//!
//! The Gregorian conversions are Howard Hinnant's branch-light `days_from_civil`/`civil_from_days`
//! (valid for any year, no lookup tables). The Julian conversions go through the Julian Day Number
//! (`JDN`); the epoch 1970-01-01 is `JDN 2440588`. ISO-8601 week dates are derived from the day
//! number directly. Weekday is `0 = Sunday … 6 = Saturday`; ISO weekday is `1 = Monday … 7 = Sunday`.

/// The Julian Day Number of the Unix epoch (1970-01-01, Gregorian).
pub const UNIX_EPOCH_JDN: i64 = 2_440_588;

/// The IERS leap-second table: `(year, month, day, TAI−UTC seconds)` — the cumulative offset that
/// takes effect at 00:00:00 UTC of the listed date. SmoothUTC (this module's default) ignores leap
/// seconds; this table is the bridge to atomic time (TAI), which has no leaps. Through 2017-01-01
/// (the most recent leap second; none scheduled since — the leap second is being phased out by 2035).
const LEAP_SECONDS: &[(i64, u32, u32, i64)] = &[
    (1972, 1, 1, 10),
    (1972, 7, 1, 11),
    (1973, 1, 1, 12),
    (1974, 1, 1, 13),
    (1975, 1, 1, 14),
    (1976, 1, 1, 15),
    (1977, 1, 1, 16),
    (1978, 1, 1, 17),
    (1979, 1, 1, 18),
    (1980, 1, 1, 19),
    (1981, 7, 1, 20),
    (1982, 7, 1, 21),
    (1983, 7, 1, 22),
    (1985, 7, 1, 23),
    (1988, 1, 1, 24),
    (1990, 1, 1, 25),
    (1991, 1, 1, 26),
    (1992, 7, 1, 27),
    (1993, 7, 1, 28),
    (1994, 7, 1, 29),
    (1996, 1, 1, 30),
    (1997, 7, 1, 31),
    (1999, 1, 1, 32),
    (2006, 1, 1, 33),
    (2009, 1, 1, 34),
    (2012, 7, 1, 35),
    (2015, 7, 1, 36),
    (2017, 1, 1, 37),
];

/// The fixed `TT − TAI` offset: Terrestrial Time runs 32.184 s ahead of atomic time, exactly.
pub const TT_MINUS_TAI_NANOS: i64 = 32_184_000_000;

/// `TAI − UTC` (whole seconds) in effect at a SmoothUTC/Unix instant — the count of leap seconds
/// (plus the 1972 base of 10) inserted on/before it. Constant between leap seconds; `10` before 1972.
pub fn tai_minus_utc(unix_seconds: i64) -> i64 {
    let mut offset = LEAP_SECONDS[0].3; // pre-1972 floor (10)
    for &(y, m, d, value) in LEAP_SECONDS {
        let effective = days_from_civil(y, m, d) * SECONDS_PER_DAY;
        if effective <= unix_seconds {
            offset = value;
        } else {
            break;
        }
    }
    offset
}

/// Convert a SmoothUTC/Unix instant in **nanoseconds** to **TAI** nanoseconds (leap-second exact).
pub fn tai_nanos_from_unix_nanos(unix_ns: i64) -> i64 {
    let secs = unix_ns.div_euclid(NANOS_PER_SECOND);
    unix_ns + tai_minus_utc(secs) * NANOS_PER_SECOND
}

/// Convert a SmoothUTC/Unix instant in **nanoseconds** to **Terrestrial Time (TT)** nanoseconds:
/// `TT = TAI + 32.184 s` — the continuous astronomical scale ephemerides are tabulated against.
pub fn tt_nanos_from_unix_nanos(unix_ns: i64) -> i64 {
    tai_nanos_from_unix_nanos(unix_ns) + TT_MINUS_TAI_NANOS
}

/// The inverse: **TT** nanoseconds back to a SmoothUTC/Unix instant in nanoseconds.
pub fn unix_nanos_from_tt_nanos(tt_ns: i64) -> i64 {
    let tai_ns = tt_ns - TT_MINUS_TAI_NANOS;
    // Recover the UTC second to know which leap offset applies, then subtract it at nanosecond scale.
    let unix_secs = tai_to_unix_seconds(tai_ns.div_euclid(NANOS_PER_SECOND));
    tai_ns - tai_minus_utc(unix_secs) * NANOS_PER_SECOND
}

/// Convert a SmoothUTC/Unix instant (seconds) to **TAI** (atomic time) seconds: `TAI = UTC + (TAI−UTC)`.
pub fn unix_to_tai_seconds(unix_seconds: i64) -> i64 {
    unix_seconds + tai_minus_utc(unix_seconds)
}

/// The inverse: **TAI** seconds back to a SmoothUTC/Unix instant. The offset is monotone in UTC, so
/// a first guess (`tai − offset(tai)`) corrected against `offset(guess)` converges in one step.
pub fn tai_to_unix_seconds(tai_seconds: i64) -> i64 {
    let mut unix = tai_seconds - tai_minus_utc(tai_seconds);
    for _ in 0..2 {
        unix = tai_seconds - tai_minus_utc(unix);
    }
    unix
}

/// A **Hybrid Logical Clock** timestamp: a physical instant (nanoseconds) plus a logical counter
/// that breaks ties and preserves causality when the physical clock stalls, jumps backward, or two
/// events share a nanosecond. The total order is lexicographic `(physical_nanos, logical)` — so the
/// derived `Ord` *is* the HLC order. This is the causally-consistent timestamp the `SyncClock` CRDT
/// keys on: it stays monotonic and orders happens-before correctly even across machines whose wall
/// clocks disagree (the bounded-skew / interplanetary case the campaign targets).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hlc {
    pub physical_nanos: i64,
    pub logical: u32,
}

impl Hlc {
    /// The zero timestamp (before any event).
    pub const ZERO: Hlc = Hlc {
        physical_nanos: 0,
        logical: 0,
    };

    /// Stamp a **local event** (or a send) given the current physical clock `now_nanos`. Advances
    /// the physical component to `max(self, now)`; the logical counter increments when physical
    /// does not move (a stall or backward jump) and resets to 0 when it does.
    pub fn tick(self, now_nanos: i64) -> Hlc {
        let physical = self.physical_nanos.max(now_nanos);
        let logical = if physical == self.physical_nanos {
            self.logical + 1
        } else {
            0
        };
        Hlc {
            physical_nanos: physical,
            logical,
        }
    }

    /// Stamp the **receipt** of a message carrying timestamp `remote`, at local physical clock
    /// `now_nanos`. Merges both clocks plus wall time, keeping the result strictly after both
    /// inputs (the HLC receive rule) so causality (`send → receive`) is always preserved.
    pub fn recv(self, remote: Hlc, now_nanos: i64) -> Hlc {
        let physical = self
            .physical_nanos
            .max(remote.physical_nanos)
            .max(now_nanos);
        let logical = if physical == self.physical_nanos && physical == remote.physical_nanos {
            self.logical.max(remote.logical) + 1
        } else if physical == self.physical_nanos {
            self.logical + 1
        } else if physical == remote.physical_nanos {
            remote.logical + 1
        } else {
            0
        };
        Hlc {
            physical_nanos: physical,
            logical,
        }
    }
}

/// A bound on a remote node's current time: it lies somewhere in `[lower_nanos, upper_nanos]`. The
/// width is the irreducible uncertainty — across a light-delay you can never pin a remote clock
/// tighter than the physics allows. The `intersect` of two valid bounds is a lattice meet (it never
/// widens), which is what makes a [`SyncClock`] conflict-free.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EstimateInterval {
    pub lower_nanos: i64,
    pub upper_nanos: i64,
}

impl EstimateInterval {
    pub fn new(lower_nanos: i64, upper_nanos: i64) -> Self {
        EstimateInterval {
            lower_nanos,
            upper_nanos,
        }
    }
    /// The uncertainty width (`upper − lower`).
    pub fn width(&self) -> i64 {
        self.upper_nanos - self.lower_nanos
    }
    pub fn contains(&self, t: i64) -> bool {
        self.lower_nanos <= t && t <= self.upper_nanos
    }
    /// The tightest interval consistent with both — the meet. Commutative, associative, idempotent.
    pub fn intersect(&self, other: &EstimateInterval) -> EstimateInterval {
        EstimateInterval {
            lower_nanos: self.lower_nanos.max(other.lower_nanos),
            upper_nanos: self.upper_nanos.min(other.upper_nanos),
        }
    }
}

/// The freshest remote time you can possibly *know*: an event happening at a node `light_delay_nanos`
/// away cannot be observed before `now − light_delay`. This is the light-cone horizon — the reason a
/// distributed clock's knowledge is principled-uncertain rather than sloppy.
pub fn knowable_horizon(now_nanos: i64, light_delay_nanos: i64) -> i64 {
    now_nanos - light_delay_nanos
}

/// A **light-cone-aware CRDT clock**: a Hybrid Logical Clock (causal order) plus per-node knowledge
/// intervals (light-cone-bounded estimates of each peer's current time). Its `merge` is a join over
/// a lattice — `max` on the HLC, interval intersection per node — so replicas separated by bounded
/// (even interplanetary) delay converge conflict-free, regardless of message order. This realises
/// "spacelike separation = CRDT concurrency": events with no shared light cone need no total order,
/// only this commutative/associative/idempotent merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncClock {
    pub hlc: Hlc,
    pub knowledge: std::collections::BTreeMap<u64, EstimateInterval>,
}

impl SyncClock {
    pub fn new() -> Self {
        SyncClock {
            hlc: Hlc::ZERO,
            knowledge: std::collections::BTreeMap::new(),
        }
    }

    /// Stamp a local event (advance the HLC); knowledge is carried unchanged.
    pub fn tick(&self, now_nanos: i64) -> SyncClock {
        SyncClock {
            hlc: self.hlc.tick(now_nanos),
            knowledge: self.knowledge.clone(),
        }
    }

    /// Record (and tighten) what this clock knows about node `node`'s current time. A fresh node is
    /// inserted; a known node's interval is intersected, so observation can only *narrow* knowledge.
    pub fn observe(&self, node: u64, interval: EstimateInterval) -> SyncClock {
        let mut knowledge = self.knowledge.clone();
        knowledge
            .entry(node)
            .and_modify(|iv| *iv = iv.intersect(&interval))
            .or_insert(interval);
        SyncClock {
            hlc: self.hlc,
            knowledge,
        }
    }

    /// The CRDT join with another replica — `max` HLC (the lattice join on the derived total order),
    /// per-node interval intersection on shared keys, union of distinct keys. Commutative,
    /// associative, and idempotent, so gossip converges regardless of message order.
    pub fn merge(&self, other: &SyncClock) -> SyncClock {
        let mut knowledge = self.knowledge.clone();
        for (&node, &interval) in &other.knowledge {
            knowledge
                .entry(node)
                .and_modify(|iv| *iv = iv.intersect(&interval))
                .or_insert(interval);
        }
        SyncClock {
            hlc: self.hlc.max(other.hlc),
            knowledge,
        }
    }
}

impl Default for SyncClock {
    fn default() -> Self {
        SyncClock::new()
    }
}

/// Nanoseconds in one SI second.
pub const NANOS_PER_SECOND: i64 = 1_000_000_000;
/// Seconds in one civil day (SmoothUTC: no leap seconds, so every day is exactly 86 400 s).
pub const SECONDS_PER_DAY: i64 = 86_400;
/// Nanoseconds in one civil day.
pub const NANOS_PER_DAY: i64 = SECONDS_PER_DAY * NANOS_PER_SECOND;

/// A civil (wall-clock) date-time, calendar + time-of-day, with no zone — the human-readable
/// decomposition of a **SmoothUTC** instant (nanoseconds since the Unix epoch, leap-second-free).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct CivilDateTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub nanosecond: u32,
}

/// Decompose a SmoothUTC instant (nanoseconds since 1970-01-01T00:00:00) into its civil date-time.
/// Floor division makes pre-epoch instants correct (e.g. `-1 ns` is `1969-12-31T23:59:59.999999999`).
pub fn civil_from_unix_nanos(ns: i64) -> CivilDateTime {
    let day = ns.div_euclid(NANOS_PER_DAY);
    let rem = ns.rem_euclid(NANOS_PER_DAY); // [0, NANOS_PER_DAY)
    let (year, month, d) = civil_from_days(day);
    let secs = rem / NANOS_PER_SECOND;
    CivilDateTime {
        year,
        month,
        day: d,
        hour: (secs / 3600) as u32,
        minute: ((secs % 3600) / 60) as u32,
        second: (secs % 60) as u32,
        nanosecond: (rem % NANOS_PER_SECOND) as u32,
    }
}

/// The inverse: the SmoothUTC instant (nanoseconds since the epoch) of a civil date-time.
pub fn unix_nanos_from_civil(dt: CivilDateTime) -> i64 {
    let days = days_from_civil(dt.year, dt.month, dt.day);
    let secs = dt.hour as i64 * 3600 + dt.minute as i64 * 60 + dt.second as i64;
    days * NANOS_PER_DAY + secs * NANOS_PER_SECOND + dt.nanosecond as i64
}

/// A UTC-offset transition: from `at_unix_seconds` (a UTC instant) onward, the zone's local time is
/// `UTC + offset_seconds`. A timezone is a list of these, sorted ascending — the shape every IANA
/// zone reduces to. DST is just two alternating offsets with transitions twice a year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZoneTransition {
    pub at_unix_seconds: i64,
    pub offset_seconds: i32,
}

/// How to resolve a local civil time that maps ambiguously to UTC: in a DST **fold** (the hour
/// repeated when clocks fall back, so a local time occurs twice) or a **gap** (the hour skipped
/// when clocks spring forward, so a local time never occurs). `Earlier` picks the earlier instant,
/// `Later` the later one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Earlier,
    Later,
}

/// The UTC offset (seconds) a zone has at `instant_ns`. `transitions` must be sorted ascending by
/// `at_unix_seconds`; `base_offset` applies before the first transition.
pub fn offset_at(transitions: &[ZoneTransition], base_offset: i32, instant_ns: i64) -> i32 {
    let secs = instant_ns.div_euclid(NANOS_PER_SECOND);
    let mut off = base_offset;
    for t in transitions {
        if t.at_unix_seconds <= secs {
            off = t.offset_seconds;
        } else {
            break;
        }
    }
    off
}

/// The local civil date-time of a UTC instant in a zone (apply the offset in effect).
pub fn to_local(
    transitions: &[ZoneTransition],
    base_offset: i32,
    instant_ns: i64,
) -> CivilDateTime {
    let off = offset_at(transitions, base_offset, instant_ns);
    civil_from_unix_nanos(instant_ns + off as i64 * NANOS_PER_SECOND)
}

/// The UTC instant of a local civil date-time in a zone. A local time normally maps to exactly one
/// instant; in a DST fold it maps to two (resolved by `fold`); in a DST gap it maps to none (the
/// boundary instants are returned, `fold` choosing which). This is the offset-dependent inverse of
/// [`to_local`], solved by testing each candidate offset for self-consistency.
pub fn from_local(
    transitions: &[ZoneTransition],
    base_offset: i32,
    civil: CivilDateTime,
    fold: Fold,
) -> i64 {
    let local_ns = unix_nanos_from_civil(civil);
    // The distinct offsets the zone ever uses are the only candidates.
    let mut offsets: Vec<i32> = vec![base_offset];
    for t in transitions {
        if !offsets.contains(&t.offset_seconds) {
            offsets.push(t.offset_seconds);
        }
    }
    // A candidate is self-consistent when the offset it assumes is actually the one in effect at
    // the UTC instant it implies: `offset_at(local − offset) == offset`.
    let mut valid: Vec<i64> = Vec::new();
    for o in &offsets {
        let utc = local_ns - *o as i64 * NANOS_PER_SECOND;
        if offset_at(transitions, base_offset, utc) == *o {
            valid.push(utc);
        }
    }
    valid.sort_unstable();
    valid.dedup();
    if valid.len() >= 2 {
        // Fold: the local time occurs twice.
        return match fold {
            Fold::Earlier => valid[0],
            Fold::Later => *valid.last().unwrap(),
        };
    }
    if valid.len() == 1 {
        return valid[0];
    }
    // Gap: the local time never occurs — return a boundary instant per `fold`.
    let mut cands: Vec<i64> = offsets
        .iter()
        .map(|o| local_ns - *o as i64 * NANOS_PER_SECOND)
        .collect();
    cands.sort_unstable();
    match fold {
        Fold::Earlier => cands[0],
        Fold::Later => *cands.last().unwrap(),
    }
}

/// A POSIX-style DST transition rule: the `week`-th `weekday` of `month`, at `time_seconds` local.
/// `week` is `1..=5` (5 = the last occurrence); `weekday` is `0 = Sunday … 6 = Saturday`. This is
/// the `Mm.w.d` form a TZ string uses (e.g. `M3.2.0` = the 2nd Sunday of March), and it generates
/// one transition per year — enough to express any zone with a regular DST schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DstRule {
    pub month: u32,
    pub week: u32,
    pub weekday: u32,
    pub time_seconds: i64,
}

/// The day-of-month of the `week`-th `weekday` of `(year, month)` — e.g. the 2nd Sunday of March,
/// or (with `week = 5`) the last Sunday of October. `week` is `1..=5`, `weekday` `0 = Sun … 6 = Sat`.
pub fn nth_weekday_of_month(year: i64, month: u32, week: u32, weekday: u32) -> u32 {
    let first = days_from_civil(year, month, 1);
    let first_weekday = weekday_from_days(first);
    let to_first = (weekday + 7 - first_weekday) % 7; // days from the 1st to the first `weekday`
    let mut day = 1 + to_first + (week - 1) * 7;
    let last = last_day_of_month(year, month);
    while day > last {
        day -= 7; // `week = 5` past the month's end → the last occurrence
    }
    day
}

/// The two UTC transitions for `year` of a zone with a regular DST schedule: spring-forward
/// (`std_offset → dst_offset`) at `start`, fall-back (`dst_offset → std_offset`) at `end`. Each
/// rule's `time_seconds` is wall-clock local in the offset in effect just before the transition.
pub fn dst_transitions_for_year(
    std_offset: i32,
    dst_offset: i32,
    start: DstRule,
    end: DstRule,
    year: i64,
) -> [ZoneTransition; 2] {
    // A rule's local wall-clock time is in the offset in effect just before the jump, so the UTC
    // instant is `local − pre_offset`. The new offset takes effect from there.
    let transition = |rule: DstRule, pre_offset: i32, new_offset: i32| {
        let day = nth_weekday_of_month(year, rule.month, rule.week, rule.weekday);
        let local = days_from_civil(year, rule.month, day) * SECONDS_PER_DAY + rule.time_seconds;
        ZoneTransition {
            at_unix_seconds: local - pre_offset as i64,
            offset_seconds: new_offset,
        }
    };
    [
        transition(start, std_offset, dst_offset), // spring forward: STD → DST
        transition(end, dst_offset, std_offset),   // fall back: DST → STD
    ]
}

/// A named time zone: its standard UTC offset, and an optional DST schedule `(dst_offset, spring
/// rule, fall rule)`. A zone with `dst = None` is a fixed offset (UTC, IST, JST). DST transitions
/// are *generated* per year from the rules, so a `ZoneSpec` is tiny — no transition table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZoneSpec {
    pub name: &'static str,
    pub std_offset: i32,
    pub dst: Option<(i32, DstRule, DstRule)>,
}

impl ZoneSpec {
    /// The zone's UTC transitions for every year in `start_year..=end_year`, sorted ascending
    /// (empty for a fixed-offset zone).
    pub fn transitions(&self, start_year: i64, end_year: i64) -> Vec<ZoneTransition> {
        let mut v = Vec::new();
        if let Some((dst_offset, start, end)) = self.dst {
            for y in start_year..=end_year {
                let pair = dst_transitions_for_year(self.std_offset, dst_offset, start, end, y);
                v.extend_from_slice(&pair);
            }
            v.sort_by_key(|t| t.at_unix_seconds);
        }
        v
    }

    /// The local civil date-time of a UTC instant in this zone (DST resolved from the rules).
    pub fn to_local(&self, instant_ns: i64) -> CivilDateTime {
        if self.dst.is_none() {
            return civil_from_unix_nanos(instant_ns + self.std_offset as i64 * NANOS_PER_SECOND);
        }
        // Generate transitions for the instant's year ±1 so it is always past the first one (this
        // makes `std_offset` as the pre-first base harmless, including the southern hemisphere).
        let (year, _, _) = civil_from_days(instant_ns.div_euclid(NANOS_PER_DAY));
        let tz = self.transitions(year - 1, year + 1);
        to_local(&tz, self.std_offset, instant_ns)
    }
}

/// The **local-as-UTC instant** of a UTC instant in a named zone: the nanoseconds that, read back
/// with the plain UTC calendar functions, yield the zone's *local* wall-clock components. This is
/// the lowering target for zoned component reads (`the hour of m in "Asia/Tokyo"`): a local clock
/// face encoded as an instant, so every UTC extractor composes onto it unchanged. `None` if the
/// zone is unknown. (Not a real instant — it is the wall clock; do not re-format it with a `Z`.)
pub fn local_instant_nanos(instant_ns: i64, zone_name: &str) -> Option<i64> {
    let zone = zone_by_name(zone_name)?;
    Some(unix_nanos_from_civil(zone.to_local(instant_ns)))
}

/// Format a UTC instant as the **local wall-clock time in a named zone**, with its offset —
/// `2024-07-01T08:00:00-04:00` (the timezone-aware "relative read" of an instant). `None` if the
/// zone is unknown. The space-aware/zoned generalisation of [`format_rfc3339`].
pub fn format_zoned(instant_ns: i64, zone_name: &str) -> Option<String> {
    let zone = zone_by_name(zone_name)?;
    let local = zone.to_local(instant_ns);
    // local-as-UTC minus the true instant is exactly the zone's offset at that moment.
    let offset_secs = (unix_nanos_from_civil(local) - instant_ns) / NANOS_PER_SECOND;
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let abs = offset_secs.abs();
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
        local.year,
        local.month,
        local.day,
        local.hour,
        local.minute,
        local.second,
        sign,
        abs / 3600,
        (abs % 3600) / 60,
    ))
}

/// Look up a [`ZoneSpec`] by IANA name (e.g. `"America/New_York"`, `"UTC"`, `"Australia/Sydney"`).
/// `None` for an unknown zone. A growable registry seeded with common zones (POSIX-rule based);
/// the full IANA set ingests later.
pub fn zone_by_name(name: &str) -> Option<ZoneSpec> {
    // North-American DST: 2nd Sunday of March → 1st Sunday of November, both at 02:00 local.
    let us = (
        DstRule {
            month: 3,
            week: 2,
            weekday: 0,
            time_seconds: 2 * 3600,
        },
        DstRule {
            month: 11,
            week: 1,
            weekday: 0,
            time_seconds: 2 * 3600,
        },
    );
    // EU DST changes at 01:00 UTC: the local wall-clock time is `01:00 + std` (pre-transition).
    let eu = |std_h: i64| {
        (
            DstRule {
                month: 3,
                week: 5,
                weekday: 0,
                time_seconds: (std_h + 1) * 3600,
            },
            DstRule {
                month: 10,
                week: 5,
                weekday: 0,
                time_seconds: (std_h + 2) * 3600,
            },
        )
    };
    let spec = match name {
        "UTC" | "Etc/UTC" | "Zulu" => ZoneSpec {
            name: "UTC",
            std_offset: 0,
            dst: None,
        },
        "America/New_York" => ZoneSpec {
            name: "America/New_York",
            std_offset: -5 * 3600,
            dst: Some((-4 * 3600, us.0, us.1)),
        },
        "America/Chicago" => ZoneSpec {
            name: "America/Chicago",
            std_offset: -6 * 3600,
            dst: Some((-5 * 3600, us.0, us.1)),
        },
        "America/Denver" => ZoneSpec {
            name: "America/Denver",
            std_offset: -7 * 3600,
            dst: Some((-6 * 3600, us.0, us.1)),
        },
        "America/Los_Angeles" => ZoneSpec {
            name: "America/Los_Angeles",
            std_offset: -8 * 3600,
            dst: Some((-7 * 3600, us.0, us.1)),
        },
        "America/Phoenix" => ZoneSpec {
            name: "America/Phoenix",
            std_offset: -7 * 3600,
            dst: None,
        },
        "Europe/London" | "Europe/Dublin" | "Europe/Lisbon" => {
            let (s, e) = eu(0);
            ZoneSpec {
                name: "Europe/London",
                std_offset: 0,
                dst: Some((3600, s, e)),
            }
        }
        "Europe/Paris" | "Europe/Berlin" | "Europe/Madrid" | "Europe/Rome" | "Europe/Amsterdam" => {
            let (s, e) = eu(1);
            ZoneSpec {
                name: "Europe/Paris",
                std_offset: 3600,
                dst: Some((2 * 3600, s, e)),
            }
        }
        "Asia/Kolkata" | "Asia/Calcutta" => ZoneSpec {
            name: "Asia/Kolkata",
            std_offset: 5 * 3600 + 1800,
            dst: None,
        },
        "Asia/Tokyo" => ZoneSpec {
            name: "Asia/Tokyo",
            std_offset: 9 * 3600,
            dst: None,
        },
        "Asia/Shanghai" => ZoneSpec {
            name: "Asia/Shanghai",
            std_offset: 8 * 3600,
            dst: None,
        },
        "Australia/Sydney" => ZoneSpec {
            name: "Australia/Sydney",
            std_offset: 10 * 3600,
            // Southern hemisphere: spring-forward in October, fall-back in April.
            dst: Some((
                11 * 3600,
                DstRule {
                    month: 10,
                    week: 1,
                    weekday: 0,
                    time_seconds: 2 * 3600,
                },
                DstRule {
                    month: 4,
                    week: 1,
                    weekday: 0,
                    time_seconds: 3 * 3600,
                },
            )),
        },
        _ => return None,
    };
    Some(spec)
}

/// Add a **calendar span** (`months` then `days`) to a civil date-time — the *civil* (wall-clock)
/// operation, distinct from adding a physical `Duration`. Months are applied first with
/// **end-of-month clamping** (`Jan 31 + 1 month = Feb 28/29`), then whole `days` roll over exactly
/// through the day number. The time-of-day is preserved. (`Span` arithmetic on a *zoned* value is
/// where it diverges from a physical duration across a DST boundary; in UTC the two agree.)
pub fn add_span(dt: CivilDateTime, months: i64, days: i64) -> CivilDateTime {
    let total = (dt.year * 12 + (dt.month as i64 - 1)) + months;
    let new_year = total.div_euclid(12);
    let new_month = (total.rem_euclid(12) + 1) as u32;
    let clamped_day = dt.day.min(last_day_of_month(new_year, new_month));
    let z = days_from_civil(new_year, new_month, clamped_day) + days;
    let (year, month, day) = civil_from_days(z);
    CivilDateTime {
        year,
        month,
        day,
        ..dt
    }
}

/// Add a **calendar span** (`months`, then `days`) to a date held as days since 1970-01-01 — the
/// civil `Date + Span` every tier computes: months clamp at the end of the month and respect leap
/// years (`Jan 31 + 1 month = Feb 28/29`), then whole days roll over. Month and day arithmetic wraps
/// at `i32`, as `Int` span arithmetic does everywhere. `Date - Span` is this with both parts negated.
pub fn date_add_span(days_since_epoch: i32, months: i32, days: i32) -> i32 {
    let z = days_since_epoch + 719468;
    let era = if z >= 0 {
        z / 146097
    } else {
        (z - 146096) / 146097
    };
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let mut year = y + if m <= 2 { 1 } else { 0 };
    let mut month = m as i32;
    let mut day = d as i32;

    let total_months = year
        .wrapping_mul(12)
        .wrapping_add(month - 1)
        .wrapping_add(months);
    year = total_months / 12;
    month = total_months % 12 + 1;
    if month <= 0 {
        month += 12;
        year -= 1;
    }

    let last = last_day_of_month(year as i64, month as u32) as i32;
    if day > last {
        day = last;
    }

    let yp = year - if month <= 2 { 1 } else { 0 };
    let era2 = if yp >= 0 { yp / 400 } else { (yp - 399) / 400 };
    let yoe2 = (yp - era2 * 400) as u32;
    let mp2 = if month > 2 {
        month as u32 - 3
    } else {
        month as u32 + 9
    };
    let doy2 = (153 * mp2 + 2) / 5 + day as u32 - 1;
    let doe2 = yoe2 * 365 + yoe2 / 4 - yoe2 / 100 + doy2;
    let result = era2 * 146097 + doe2 as i32 - 719468;

    result.wrapping_add(days)
}

/// The number of **complete calendar months** from instant `a` to instant `b` (signed; negative when
/// `b` precedes `a`). A month is complete only when `b` reaches at least `a + k months` under the same
/// end-of-month-clamping, time-of-day-preserving rule as [`add_span`] — so `Jan 15 → Mar 14` is 1
/// month (not 2), `Jan 31 → Feb 28` (non-leap) is a full month, and one hour short of a month is 0.
pub fn months_between(a_nanos: i64, b_nanos: i64) -> i64 {
    if b_nanos < a_nanos {
        return -months_between(b_nanos, a_nanos);
    }
    let a = civil_from_unix_nanos(a_nanos);
    let b = civil_from_unix_nanos(b_nanos);
    let mut months = (b.year - a.year) * 12 + (b.month as i64 - a.month as i64);
    // Matching only year+month can overshoot by the day/time-of-day; if `a + months` lands after `b`
    // the final month has not completed, so step back one. (At most one correction is ever needed.)
    if unix_nanos_from_civil(add_span(a, months, 0)) > b_nanos {
        months -= 1;
    }
    months
}

/// The number of **complete calendar years** from `a` to `b` (signed) — complete 12-month periods,
/// so `2020-06-01 → 2024-03-01` is 3 (the fourth year, ending 2024-06-01, has not arrived).
pub fn years_between(a_nanos: i64, b_nanos: i64) -> i64 {
    months_between(a_nanos, b_nanos) / 12
}

/// True if a day number falls on a weekend (Saturday or Sunday).
pub fn is_weekend(z: i64) -> bool {
    matches!(iso_weekday_from_days(z), 6 | 7) // ISO: 6 = Saturday, 7 = Sunday
}

/// The count of **business days** (Mon–Fri) in the half-open interval between two day numbers.
/// Signed by direction (`b < a` yields a negative count); `[a, b)` so adjacent days differ by 1.
pub fn business_days_between(a: i64, b: i64) -> i64 {
    if a == b {
        return 0;
    }
    let (lo, hi, sign) = if a < b { (a, b, 1) } else { (b, a, -1) };
    let span = hi - lo;
    let full_weeks = span / 7;
    let mut count = full_weeks * 5;
    let mut d = lo + full_weeks * 7;
    while d < hi {
        if !is_weekend(d) {
            count += 1;
        }
        d += 1;
    }
    sign * count
}

/// Advance a day number by `n` **business days** (skipping weekends): `n > 0` moves forward,
/// `n < 0` backward. Landing rule is "next business day in the direction of travel", so
/// `add_business_days(friday, 1)` is the following Monday and `add_business_days(monday, -1)` is
/// the preceding Friday.
pub fn add_business_days(z: i64, n: i64) -> i64 {
    let step = if n >= 0 { 1 } else { -1 };
    let mut remaining = n.abs();
    let mut d = z;
    while remaining > 0 {
        d += step;
        if !is_weekend(d) {
            remaining -= 1;
        }
    }
    d
}

/// Format a **physical duration** (nanoseconds) as a compact human string — `1h30m`, `500ms`,
/// `1.5s`, `2d`, `-45m`, `0s`. Largest unit first; zero components omitted; sub-second fractions
/// trimmed. The inverse of [`parse_duration`]. (This is a *physical* elapsed time, distinct from a
/// calendar `Span`: it has no months — those aren't a fixed number of nanoseconds.)
pub fn format_duration(nanos: i64) -> String {
    if nanos == 0 {
        return "0s".to_string();
    }
    const D: u128 = 86_400_000_000_000;
    const H: u128 = 3_600_000_000_000;
    const M: u128 = 60_000_000_000;
    const S: u128 = 1_000_000_000;
    const MS: u128 = 1_000_000;
    const US: u128 = 1_000;
    let mut out = String::new();
    if nanos < 0 {
        out.push('-');
    }
    let mut abs = (nanos as i128).unsigned_abs();
    // A fractional sub-unit, trimmed of trailing zeros (`frac` has `width` digits).
    let frac_str = |frac: u128, width: usize| -> String {
        let mut f = format!("{:0width$}", frac, width = width);
        while f.ends_with('0') {
            f.pop();
        }
        f
    };
    if abs >= S {
        let d = abs / D;
        abs %= D;
        let h = abs / H;
        abs %= H;
        let m = abs / M;
        abs %= M;
        if d > 0 {
            out += &format!("{d}d");
        }
        if h > 0 {
            out += &format!("{h}h");
        }
        if m > 0 {
            out += &format!("{m}m");
        }
        let (secs, frac) = (abs / S, abs % S);
        if frac == 0 {
            if secs > 0 {
                out += &format!("{secs}s");
            }
        } else {
            out += &format!("{secs}.{}s", frac_str(frac, 9));
        }
    } else if abs >= MS {
        let (w, frac) = (abs / MS, abs % MS);
        out += &(if frac == 0 {
            format!("{w}ms")
        } else {
            format!("{w}.{}ms", frac_str(frac, 6))
        });
    } else if abs >= US {
        let (w, frac) = (abs / US, abs % US);
        out += &(if frac == 0 {
            format!("{w}us")
        } else {
            format!("{w}.{}us", frac_str(frac, 3))
        });
    } else {
        out += &format!("{abs}ns");
    }
    out
}

/// Parse a **physical duration** string into nanoseconds — `1h30m`, `90m`, `1.5s`, `500ms`, `2d`,
/// `-1h`, `1d2h3m4s`. Units: `d h m s ms us`/`µs` `ns`. `None` on malformed input or i64 overflow.
pub fn parse_duration(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let mut i = 0;
    let neg = match b.first() {
        Some(&b'-') => {
            i += 1;
            true
        }
        Some(&b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    if i >= b.len() {
        return None;
    }
    let mut total: i128 = 0;
    let mut saw_any = false;
    while i < b.len() {
        let int_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let int_str = &s[int_start..i];
        let mut frac_str = "";
        if i < b.len() && b[i] == b'.' {
            i += 1;
            let frac_start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            frac_str = &s[frac_start..i];
        }
        if int_str.is_empty() && frac_str.is_empty() {
            return None; // a unit with no number
        }
        let rest = &s[i..];
        let (unit_nanos, ulen): (i128, usize) = if rest.starts_with("ms") {
            (1_000_000, 2)
        } else if rest.starts_with("ns") {
            (1, 2)
        } else if rest.starts_with("us") {
            (1_000, 2)
        } else if rest.starts_with("µs") {
            (1_000, "µs".len())
        } else if rest.starts_with('s') {
            (1_000_000_000, 1)
        } else if rest.starts_with('m') {
            (60_000_000_000, 1)
        } else if rest.starts_with('h') {
            (3_600_000_000_000, 1)
        } else if rest.starts_with('d') {
            (86_400_000_000_000, 1)
        } else {
            return None; // unknown / missing unit
        };
        i += ulen;
        let int_val: i128 = if int_str.is_empty() {
            0
        } else {
            int_str.parse().ok()?
        };
        total = total.checked_add(int_val.checked_mul(unit_nanos)?)?;
        if !frac_str.is_empty() {
            let frac_val: i128 = frac_str.parse().ok()?;
            let mut denom: i128 = 1;
            for _ in 0..frac_str.len() {
                denom *= 10;
            }
            total = total.checked_add(frac_val * unit_nanos / denom)?;
        }
        saw_any = true;
    }
    if !saw_any {
        return None;
    }
    i64::try_from(if neg { -total } else { total }).ok()
}

/// Format a SmoothUTC instant as an **RFC 3339 / ISO 8601** UTC timestamp (`Z` zone), e.g.
/// `1970-01-01T00:00:00Z` or `2024-03-10T07:30:00.123456789Z`. Sub-second precision appears only
/// when nonzero, with trailing zeros trimmed.
pub fn format_rfc3339(instant_ns: i64) -> String {
    let c = civil_from_unix_nanos(instant_ns);
    let mut s = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    );
    if c.nanosecond != 0 {
        let mut frac = format!("{:09}", c.nanosecond);
        while frac.ends_with('0') {
            frac.pop();
        }
        s.push('.');
        s.push_str(&frac);
    }
    s.push('Z');
    s
}

/// Format a **time-of-day** (nanoseconds from midnight) as `HH:MM:SS`, with a trailing sub-second
/// fraction only when nonzero (trailing zeros trimmed) — e.g. `07:30:45`, `16:00:00`,
/// `07:30:45.5`. The clock face is lossless to the nanosecond; the input is normalised modulo a day
/// so a negative or overflowing value still maps onto `[00:00:00, 24:00:00)`.
pub fn format_time_of_day(nanos_from_midnight: i64) -> String {
    let rem = nanos_from_midnight.rem_euclid(NANOS_PER_DAY);
    let secs = rem / NANOS_PER_SECOND;
    let mut s = format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    );
    let nanos = (rem % NANOS_PER_SECOND) as u32;
    if nanos != 0 {
        let mut frac = format!("{:09}", nanos);
        while frac.ends_with('0') {
            frac.pop();
        }
        s.push('.');
        s.push_str(&frac);
    }
    s
}

/// Parse an **RFC 3339 / ISO 8601** timestamp into a SmoothUTC instant (nanoseconds since epoch).
/// Accepts a `Z` zone or a numeric `±HH:MM` offset (normalized to UTC); `None` on malformed input.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    // The fixed `YYYY-MM-DDTHH:MM:SS` prefix is 19 bytes; a zone always follows.
    if b.len() < 20 {
        return None;
    }
    let digits = |start: usize, len: usize| -> Option<i64> {
        let mut v: i64 = 0;
        for i in start..start + len {
            let c = *b.get(i)?;
            if !c.is_ascii_digit() {
                return None;
            }
            v = v * 10 + (c - b'0') as i64;
        }
        Some(v)
    };
    if b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let year = digits(0, 4)?;
    let month = digits(5, 2)? as u32;
    let day = digits(8, 2)? as u32;
    let hour = digits(11, 2)? as u32;
    let minute = digits(14, 2)? as u32;
    let second = digits(17, 2)? as u32;
    if !(1..=12).contains(&month) || !(1..=last_day_of_month(year, month)).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut pos = 19;
    // Optional fractional seconds, scaled to nanoseconds (9 digits; extra digits truncated).
    let mut nanosecond: u32 = 0;
    if b.get(pos) == Some(&b'.') {
        pos += 1;
        let (mut frac, mut count) = (0u64, 0u32);
        while let Some(&c) = b.get(pos) {
            if !c.is_ascii_digit() {
                break;
            }
            if count < 9 {
                frac = frac * 10 + (c - b'0') as u64;
                count += 1;
            }
            pos += 1;
        }
        if count == 0 {
            return None; // a '.' with no digits
        }
        for _ in count..9 {
            frac *= 10;
        }
        nanosecond = frac as u32;
    }
    // Zone: `Z` (UTC) or a numeric `±HH:MM` offset.
    let offset_seconds: i64 = match b.get(pos) {
        Some(&b'Z') => {
            pos += 1;
            0
        }
        Some(&c) if c == b'+' || c == b'-' => {
            let sign = if c == b'+' { 1 } else { -1 };
            let oh = digits(pos + 1, 2)?;
            if b.get(pos + 3) != Some(&b':') {
                return None;
            }
            let om = digits(pos + 4, 2)?;
            if oh > 23 || om > 59 {
                return None;
            }
            pos += 6;
            sign * (oh * 3600 + om * 60)
        }
        _ => return None,
    };
    if pos != b.len() {
        return None; // trailing garbage
    }
    let civil = CivilDateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        nanosecond,
    };
    Some(unix_nanos_from_civil(civil) - offset_seconds * NANOS_PER_SECOND)
}

/// True if `year` is a leap year in the proleptic **Gregorian** calendar (every 4th year, except
/// centuries, except every 400th).
pub fn is_gregorian_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// True if `year` is a leap year in the proleptic **Julian** calendar (every 4th year, no
/// century exception).
pub fn is_julian_leap(year: i64) -> bool {
    year.rem_euclid(4) == 0
}

/// Days since 1970-01-01 for a **Gregorian** civil date. Howard Hinnant's algorithm — exact for
/// any year, `month` in `1..=12`, `day` in `1..=last_day_of_month`.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as i64; // [0, 399]
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// The inverse of [`days_from_civil`]: the **Gregorian** `(year, month, day)` of a day number.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

/// The last day (28–31) of a **Gregorian** month.
pub fn last_day_of_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_gregorian_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// The weekday of a day number: `0 = Sunday … 6 = Saturday`. (1970-01-01 was a Thursday = 4.)
pub fn weekday_from_days(z: i64) -> u32 {
    (z + 4).rem_euclid(7) as u32
}

/// The ISO-8601 weekday of a day number: `1 = Monday … 7 = Sunday`.
pub fn iso_weekday_from_days(z: i64) -> u32 {
    (z + 3).rem_euclid(7) as u32 + 1
}

/// The 1-based day of the **Gregorian** year (`1..=366`).
pub fn day_of_year(year: i64, month: u32, day: u32) -> u32 {
    (days_from_civil(year, month, day) - days_from_civil(year, 1, 1) + 1) as u32
}

/// The **ISO-8601 week date** of a day number: `(iso_year, week, iso_weekday)`, with
/// `week` in `1..=53` and `iso_weekday` in `1..=7` (Monday … Sunday). The ISO year can differ
/// from the civil year near January 1 / December 31 (e.g. 2021-01-01 is ISO 2020-W53-5).
pub fn iso_week_from_days(z: i64) -> (i64, u32, u32) {
    let iso_dow = iso_weekday_from_days(z);
    // The Thursday of this ISO week (days 1..7 = Mon..Sun, so Thursday = day 4) names the ISO year.
    let thursday = z + (4 - iso_dow as i64);
    let (iso_year, _, _) = civil_from_days(thursday);
    // ISO week 1 is the week containing January 4 (equivalently, the year's first Thursday); its
    // Monday anchors week numbering. (Jan 4's *week's* Thursday is always in `iso_year`, unlike
    // Jan 1's, which can fall in the previous year.)
    let jan4 = days_from_civil(iso_year, 1, 4);
    let week1_monday = jan4 - (iso_weekday_from_days(jan4) as i64 - 1);
    let week = ((z - week1_monday) / 7 + 1) as u32;
    (iso_year, week, iso_dow)
}

/// The inverse of [`iso_week_from_days`]: the day number of an ISO-8601 week date
/// `(iso_year, week, iso_weekday)` (week `1..=53`, weekday `1..=7` Monday … Sunday).
pub fn days_from_iso_week(iso_year: i64, week: u32, iso_weekday: u32) -> i64 {
    let jan4 = days_from_civil(iso_year, 1, 4);
    let week1_monday = jan4 - (iso_weekday_from_days(jan4) as i64 - 1);
    week1_monday + (week as i64 - 1) * 7 + (iso_weekday as i64 - 1)
}

/// The Julian Day Number of a **Julian-calendar** date (proleptic, `month` in `1..=12`).
pub fn jdn_from_julian(year: i64, month: u32, day: u32) -> i64 {
    let a = (14 - month as i64) / 12;
    let y = year + 4800 - a;
    let m = month as i64 + 12 * a - 3;
    day as i64 + (153 * m + 2) / 5 + 365 * y + y / 4 - 32083
}

/// The inverse of [`jdn_from_julian`]: the **Julian-calendar** `(year, month, day)` of a JDN.
pub fn julian_from_jdn(jdn: i64) -> (i64, u32, u32) {
    let c = jdn + 32082;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day = (e - (153 * m + 2) / 5 + 1) as u32;
    let month = (m + 3 - 12 * (m / 10)) as u32;
    let year = d - 4800 + m / 10;
    (year, month, day)
}

/// Days since 1970-01-01 for a **Julian-calendar** date.
pub fn days_from_julian(year: i64, month: u32, day: u32) -> i64 {
    jdn_from_julian(year, month, day) - UNIX_EPOCH_JDN
}

/// The inverse: the **Julian-calendar** `(year, month, day)` of a day number.
pub fn julian_from_days(z: i64) -> (i64, u32, u32) {
    julian_from_jdn(z + UNIX_EPOCH_JDN)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/temporal/tests.rs"]
mod tests;
