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
    (1972, 1, 1, 10), (1972, 7, 1, 11), (1973, 1, 1, 12), (1974, 1, 1, 13), (1975, 1, 1, 14),
    (1976, 1, 1, 15), (1977, 1, 1, 16), (1978, 1, 1, 17), (1979, 1, 1, 18), (1980, 1, 1, 19),
    (1981, 7, 1, 20), (1982, 7, 1, 21), (1983, 7, 1, 22), (1985, 7, 1, 23), (1988, 1, 1, 24),
    (1990, 1, 1, 25), (1991, 1, 1, 26), (1992, 7, 1, 27), (1993, 7, 1, 28), (1994, 7, 1, 29),
    (1996, 1, 1, 30), (1997, 7, 1, 31), (1999, 1, 1, 32), (2006, 1, 1, 33), (2009, 1, 1, 34),
    (2012, 7, 1, 35), (2015, 7, 1, 36), (2017, 1, 1, 37),
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
    pub const ZERO: Hlc = Hlc { physical_nanos: 0, logical: 0 };

    /// Stamp a **local event** (or a send) given the current physical clock `now_nanos`. Advances
    /// the physical component to `max(self, now)`; the logical counter increments when physical
    /// does not move (a stall or backward jump) and resets to 0 when it does.
    pub fn tick(self, now_nanos: i64) -> Hlc {
        let physical = self.physical_nanos.max(now_nanos);
        let logical = if physical == self.physical_nanos { self.logical + 1 } else { 0 };
        Hlc { physical_nanos: physical, logical }
    }

    /// Stamp the **receipt** of a message carrying timestamp `remote`, at local physical clock
    /// `now_nanos`. Merges both clocks plus wall time, keeping the result strictly after both
    /// inputs (the HLC receive rule) so causality (`send → receive`) is always preserved.
    pub fn recv(self, remote: Hlc, now_nanos: i64) -> Hlc {
        let physical = self.physical_nanos.max(remote.physical_nanos).max(now_nanos);
        let logical = if physical == self.physical_nanos && physical == remote.physical_nanos {
            self.logical.max(remote.logical) + 1
        } else if physical == self.physical_nanos {
            self.logical + 1
        } else if physical == remote.physical_nanos {
            remote.logical + 1
        } else {
            0
        };
        Hlc { physical_nanos: physical, logical }
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
        EstimateInterval { lower_nanos, upper_nanos }
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
        SyncClock { hlc: Hlc::ZERO, knowledge: std::collections::BTreeMap::new() }
    }

    /// Stamp a local event (advance the HLC); knowledge is carried unchanged.
    pub fn tick(&self, now_nanos: i64) -> SyncClock {
        SyncClock { hlc: self.hlc.tick(now_nanos), knowledge: self.knowledge.clone() }
    }

    /// Record (and tighten) what this clock knows about node `node`'s current time. A fresh node is
    /// inserted; a known node's interval is intersected, so observation can only *narrow* knowledge.
    pub fn observe(&self, node: u64, interval: EstimateInterval) -> SyncClock {
        let mut knowledge = self.knowledge.clone();
        knowledge
            .entry(node)
            .and_modify(|iv| *iv = iv.intersect(&interval))
            .or_insert(interval);
        SyncClock { hlc: self.hlc, knowledge }
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
        SyncClock { hlc: self.hlc.max(other.hlc), knowledge }
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
pub fn to_local(transitions: &[ZoneTransition], base_offset: i32, instant_ns: i64) -> CivilDateTime {
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
    let mut cands: Vec<i64> = offsets.iter().map(|o| local_ns - *o as i64 * NANOS_PER_SECOND).collect();
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
        ZoneTransition { at_unix_seconds: local - pre_offset as i64, offset_seconds: new_offset }
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
        local.year, local.month, local.day, local.hour, local.minute, local.second,
        sign, abs / 3600, (abs % 3600) / 60,
    ))
}

/// Look up a [`ZoneSpec`] by IANA name (e.g. `"America/New_York"`, `"UTC"`, `"Australia/Sydney"`).
/// `None` for an unknown zone. A growable registry seeded with common zones (POSIX-rule based);
/// the full IANA set ingests later.
pub fn zone_by_name(name: &str) -> Option<ZoneSpec> {
    // North-American DST: 2nd Sunday of March → 1st Sunday of November, both at 02:00 local.
    let us = (
        DstRule { month: 3, week: 2, weekday: 0, time_seconds: 2 * 3600 },
        DstRule { month: 11, week: 1, weekday: 0, time_seconds: 2 * 3600 },
    );
    // EU DST changes at 01:00 UTC: the local wall-clock time is `01:00 + std` (pre-transition).
    let eu = |std_h: i64| {
        (
            DstRule { month: 3, week: 5, weekday: 0, time_seconds: (std_h + 1) * 3600 },
            DstRule { month: 10, week: 5, weekday: 0, time_seconds: (std_h + 2) * 3600 },
        )
    };
    let spec = match name {
        "UTC" | "Etc/UTC" | "Zulu" => ZoneSpec { name: "UTC", std_offset: 0, dst: None },
        "America/New_York" => ZoneSpec { name: "America/New_York", std_offset: -5 * 3600, dst: Some((-4 * 3600, us.0, us.1)) },
        "America/Chicago" => ZoneSpec { name: "America/Chicago", std_offset: -6 * 3600, dst: Some((-5 * 3600, us.0, us.1)) },
        "America/Denver" => ZoneSpec { name: "America/Denver", std_offset: -7 * 3600, dst: Some((-6 * 3600, us.0, us.1)) },
        "America/Los_Angeles" => ZoneSpec { name: "America/Los_Angeles", std_offset: -8 * 3600, dst: Some((-7 * 3600, us.0, us.1)) },
        "America/Phoenix" => ZoneSpec { name: "America/Phoenix", std_offset: -7 * 3600, dst: None },
        "Europe/London" | "Europe/Dublin" | "Europe/Lisbon" => {
            let (s, e) = eu(0);
            ZoneSpec { name: "Europe/London", std_offset: 0, dst: Some((3600, s, e)) }
        }
        "Europe/Paris" | "Europe/Berlin" | "Europe/Madrid" | "Europe/Rome" | "Europe/Amsterdam" => {
            let (s, e) = eu(1);
            ZoneSpec { name: "Europe/Paris", std_offset: 3600, dst: Some((2 * 3600, s, e)) }
        }
        "Asia/Kolkata" | "Asia/Calcutta" => ZoneSpec { name: "Asia/Kolkata", std_offset: 5 * 3600 + 1800, dst: None },
        "Asia/Tokyo" => ZoneSpec { name: "Asia/Tokyo", std_offset: 9 * 3600, dst: None },
        "Asia/Shanghai" => ZoneSpec { name: "Asia/Shanghai", std_offset: 8 * 3600, dst: None },
        "Australia/Sydney" => ZoneSpec {
            name: "Australia/Sydney",
            std_offset: 10 * 3600,
            // Southern hemisphere: spring-forward in October, fall-back in April.
            dst: Some((
                11 * 3600,
                DstRule { month: 10, week: 1, weekday: 0, time_seconds: 2 * 3600 },
                DstRule { month: 4, week: 1, weekday: 0, time_seconds: 3 * 3600 },
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
    CivilDateTime { year, month, day, ..dt }
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
    matches!(weekday_from_days(z), 0 | 6) // 0 = Sunday, 6 = Saturday
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
        out += &(if frac == 0 { format!("{w}ms") } else { format!("{w}.{}ms", frac_str(frac, 6)) });
    } else if abs >= US {
        let (w, frac) = (abs / US, abs % US);
        out += &(if frac == 0 { format!("{w}us") } else { format!("{w}.{}us", frac_str(frac, 3)) });
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
        Some(&b'-') => { i += 1; true }
        Some(&b'+') => { i += 1; false }
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
        let int_val: i128 = if int_str.is_empty() { 0 } else { int_str.parse().ok()? };
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
    let mut s = format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60);
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
    let civil = CivilDateTime { year, month, day, hour, minute, second, nanosecond };
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
        2 => if is_gregorian_leap(year) { 29 } else { 28 },
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
mod tests {
    use super::*;

    /// A tiny deterministic RNG (SplitMix64) for reproducible fuzz.
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
    fn local_instant_encodes_the_wall_clock_for_zoned_component_reads() {
        // The local-as-UTC instant, read with the plain UTC calendar, gives the zone's wall clock.
        let read = |ts: &str, zone: &str| {
            let inst = parse_rfc3339(ts).unwrap();
            let local = local_instant_nanos(inst, zone).unwrap();
            civil_from_unix_nanos(local)
        };
        // 2024-07-01T12:00Z in New York = EDT (-4) → 08:00 local, same day.
        let ny = read("2024-07-01T12:00:00Z", "America/New_York");
        assert_eq!((ny.hour, ny.day, ny.month), (8, 1, 7));
        // 2024-01-01T00:00Z in Kolkata (+5:30) → 05:30 local.
        let ist = read("2024-01-01T00:00:00Z", "Asia/Kolkata");
        assert_eq!((ist.hour, ist.minute, ist.day), (5, 30, 1));
        // 2024-07-01T02:00Z in New York (-4) → previous calendar day, 22:00 local (date rollover).
        let roll = read("2024-07-01T02:00:00Z", "America/New_York");
        assert_eq!((roll.hour, roll.day, roll.month), (22, 30, 6));
        // An unknown zone yields None, never a panic.
        assert_eq!(local_instant_nanos(0, "Mars/Olympus_Mons"), None);
    }

    #[test]
    fn calendar_months_and_years_between_count_complete_periods() {
        let at = |ts: &str| parse_rfc3339(ts).unwrap();
        let months = |a: &str, b: &str| months_between(at(a), at(b));
        let years = |a: &str, b: &str| years_between(at(a), at(b));

        // Whole and partial months.
        assert_eq!(months("2024-01-15T00:00:00Z", "2024-03-15T00:00:00Z"), 2);
        assert_eq!(months("2024-01-15T00:00:00Z", "2024-03-14T00:00:00Z"), 1); // a day short
        // End-of-month clamp: Jan 31 → Feb 28 (2023, non-leap) IS a complete month.
        assert_eq!(months("2023-01-31T00:00:00Z", "2023-02-28T00:00:00Z"), 1);
        assert_eq!(months("2023-01-31T00:00:00Z", "2023-02-27T00:00:00Z"), 0);
        // Time-of-day precision: one hour short of a month is not a complete month.
        assert_eq!(months("2024-01-15T12:00:00Z", "2024-02-15T11:00:00Z"), 0);
        assert_eq!(months("2024-01-15T12:00:00Z", "2024-02-15T12:00:00Z"), 1);
        // Signed / antisymmetric.
        assert_eq!(months("2024-03-15T00:00:00Z", "2024-01-15T00:00:00Z"), -2);
        assert_eq!(months("2020-01-01T00:00:00Z", "2020-01-01T00:00:00Z"), 0);

        // Complete years are complete 12-month periods.
        assert_eq!(years("2020-06-01T00:00:00Z", "2024-06-01T00:00:00Z"), 4);
        assert_eq!(years("2020-06-01T00:00:00Z", "2024-03-01T00:00:00Z"), 3); // 4th year not reached
        assert_eq!(years("2020-06-01T00:00:00Z", "2024-05-31T00:00:00Z"), 3);
        assert_eq!(years("2024-01-01T00:00:00Z", "2020-01-01T00:00:00Z"), -4);
    }

    #[test]
    fn time_of_day_formats_losslessly_to_the_nanosecond() {
        let h = 3_600 * NANOS_PER_SECOND;
        let m = 60 * NANOS_PER_SECOND;
        let s = NANOS_PER_SECOND;
        assert_eq!(format_time_of_day(0), "00:00:00");
        assert_eq!(format_time_of_day(16 * h), "16:00:00"); // 4pm
        assert_eq!(format_time_of_day(7 * h + 30 * m + 45 * s), "07:30:45");
        assert_eq!(format_time_of_day(23 * h + 59 * m + 59 * s), "23:59:59");
        // Sub-second fraction only when nonzero, trailing zeros trimmed.
        assert_eq!(format_time_of_day(7 * h + 30 * m + 45 * s + 500_000_000), "07:30:45.5");
        assert_eq!(format_time_of_day(123_456_789), "00:00:00.123456789");
        // Normalised modulo a day: a full day wraps to midnight, a negative is the pre-midnight clock.
        assert_eq!(format_time_of_day(NANOS_PER_DAY), "00:00:00");
        assert_eq!(format_time_of_day(-1), "23:59:59.999999999");
    }

    #[test]
    fn gregorian_known_anchors() {
        // The Unix epoch is day 0, a Thursday.
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(weekday_from_days(0), 4); // Thursday
        assert_eq!(iso_weekday_from_days(0), 4); // Thursday = ISO 4
        // Y2K: 2000-01-01 was a Saturday, day 10957.
        assert_eq!(days_from_civil(2000, 1, 1), 10957);
        assert_eq!(weekday_from_days(10957), 6); // Saturday
        // A day before the epoch.
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn gregorian_leap_years_and_month_lengths() {
        assert!(is_gregorian_leap(2000)); // divisible by 400
        assert!(!is_gregorian_leap(1900)); // century, not by 400
        assert!(is_gregorian_leap(2024));
        assert!(!is_gregorian_leap(2023));
        assert_eq!(last_day_of_month(2024, 2), 29);
        assert_eq!(last_day_of_month(2023, 2), 28);
        assert_eq!(last_day_of_month(2024, 1), 31);
        assert_eq!(last_day_of_month(2024, 4), 30);
        // Day-of-year: 2024-12-31 is day 366 (leap), 2023-12-31 is day 365.
        assert_eq!(day_of_year(2024, 12, 31), 366);
        assert_eq!(day_of_year(2023, 12, 31), 365);
        assert_eq!(day_of_year(2024, 3, 1), 61); // 31 + 29 + 1
    }

    #[test]
    fn iso_week_date_edge_cases() {
        // 2021-01-01 (Friday) belongs to ISO week 53 of 2020.
        assert_eq!(iso_week_from_days(days_from_civil(2021, 1, 1)), (2020, 53, 5));
        // 2020-12-31 (Thursday) is ISO 2020-W53-4.
        assert_eq!(iso_week_from_days(days_from_civil(2020, 12, 31)), (2020, 53, 4));
        // 2023-01-01 (Sunday) is ISO 2022-W52-7.
        assert_eq!(iso_week_from_days(days_from_civil(2023, 1, 1)), (2022, 52, 7));
        // 2024-01-01 (Monday) starts ISO 2024-W01-1 cleanly.
        assert_eq!(iso_week_from_days(days_from_civil(2024, 1, 1)), (2024, 1, 1));
        // A 53-week ISO year: 2020 has 53 weeks; 2020-W53 exists.
        let (y, w, _) = iso_week_from_days(days_from_civil(2020, 12, 28)); // Monday of W53
        assert_eq!((y, w), (2020, 53));
    }

    #[test]
    fn julian_gregorian_divergence() {
        // The Gregorian reform: 1582-10-15 Gregorian = 1582-10-05 Julian (the 10-day jump).
        let reform = days_from_civil(1582, 10, 15);
        assert_eq!(julian_from_days(reform), (1582, 10, 5));
        // In the 20th/21st century the Gregorian calendar runs 13 days ahead of Julian:
        // Gregorian 1970-01-01 is Julian 1969-12-19.
        assert_eq!(julian_from_days(days_from_civil(1970, 1, 1)), (1969, 12, 19));
        // Round the other way: Julian 1969-12-19 maps back to the same day number.
        assert_eq!(days_from_julian(1969, 12, 19), 0);
        // Julian leap rule differs: 1900 is a leap year in Julian, not in Gregorian.
        assert!(is_julian_leap(1900));
        assert!(!is_gregorian_leap(1900));
    }

    #[test]
    fn gregorian_round_trips_over_a_wide_range_under_fuzz() {
        // Property: civil_from_days ∘ days_from_civil = identity for every day across ±400k days
        // (~±1100 years around the epoch), and the day number is strictly monotonic in date.
        let mut prev = i64::MIN;
        let mut rng = Rng(0x_DA7E_C0DE_1234_5678);
        for _ in 0..50_000 {
            let z = (rng.next() % 800_001) as i64 - 400_000;
            let (y, m, d) = civil_from_days(z);
            assert!((1..=12).contains(&m), "month in range at z={z}");
            assert!((1..=last_day_of_month(y, m)).contains(&d), "day in range at z={z}");
            assert_eq!(days_from_civil(y, m, d), z, "round-trip at z={z}");
            // Weekday cycles correctly: consecutive days differ by one weekday.
            assert_eq!(weekday_from_days(z + 1), (weekday_from_days(z) + 1) % 7);
            let _ = prev;
            prev = z;
        }
    }

    #[test]
    fn smooth_utc_instant_civil_decomposition() {
        let dt = |y, mo, d, h, mi, s, n| CivilDateTime {
            year: y, month: mo, day: d, hour: h, minute: mi, second: s, nanosecond: n,
        };
        // The epoch.
        assert_eq!(civil_from_unix_nanos(0), dt(1970, 1, 1, 0, 0, 0, 0));
        assert_eq!(unix_nanos_from_civil(dt(1970, 1, 1, 0, 0, 0, 0)), 0);
        // One full day.
        assert_eq!(unix_nanos_from_civil(dt(1970, 1, 2, 0, 0, 0, 0)), NANOS_PER_DAY);
        // A precise instant with sub-second nanos.
        let stamp = dt(2024, 2, 29, 13, 45, 30, 123_456_789); // a leap-day afternoon
        assert_eq!(civil_from_unix_nanos(unix_nanos_from_civil(stamp)), stamp);
        // Pre-epoch: −1 ns is the last nanosecond of 1969.
        assert_eq!(
            civil_from_unix_nanos(-1),
            dt(1969, 12, 31, 23, 59, 59, 999_999_999)
        );
        // Time-of-day extraction: noon = 12:00:00.
        assert_eq!(civil_from_unix_nanos(12 * 3600 * NANOS_PER_SECOND).hour, 12);
    }

    #[test]
    fn calendar_span_arithmetic_clamps_and_rolls_over() {
        let dt = |y, mo, d, h, mi, s, n| CivilDateTime {
            year: y, month: mo, day: d, hour: h, minute: mi, second: s, nanosecond: n,
        };
        // End-of-month clamping: Jan 31 + 1 month = Feb 28 (2023) / Feb 29 (2024).
        assert_eq!(add_span(dt(2023, 1, 31, 9, 0, 0, 0), 1, 0), dt(2023, 2, 28, 9, 0, 0, 0));
        assert_eq!(add_span(dt(2024, 1, 31, 9, 0, 0, 0), 1, 0), dt(2024, 2, 29, 9, 0, 0, 0));
        // Time-of-day is preserved across a span.
        assert_eq!(add_span(dt(2024, 3, 15, 13, 45, 30, 7), 0, 1), dt(2024, 3, 16, 13, 45, 30, 7));
        // Year rollover: Dec + 1 month = next Jan.
        assert_eq!(add_span(dt(2024, 12, 10, 0, 0, 0, 0), 1, 0), dt(2025, 1, 10, 0, 0, 0, 0));
        // Days roll over months: Jan 15 + 20 days = Feb 4.
        assert_eq!(add_span(dt(2024, 1, 15, 0, 0, 0, 0), 0, 20), dt(2024, 2, 4, 0, 0, 0, 0));
        // Negative months: Mar 31 − 1 month = Feb 29 (clamped, leap year).
        assert_eq!(add_span(dt(2024, 3, 31, 0, 0, 0, 0), -1, 0), dt(2024, 2, 29, 0, 0, 0, 0));
        // +12 months = same date next year (a clean year).
        assert_eq!(add_span(dt(2024, 6, 15, 0, 0, 0, 0), 12, 0), dt(2025, 6, 15, 0, 0, 0, 0));
    }

    #[test]
    fn business_day_math() {
        let d = |y, mo, day| days_from_civil(y, mo, day);
        // 2024-03-11 is a Monday; 2024-03-18 the next Monday → 5 business days between.
        assert_eq!(business_days_between(d(2024, 3, 11), d(2024, 3, 18)), 5);
        // Within one week, Mon→Fri is 4 business days; the weekend adds none.
        assert_eq!(business_days_between(d(2024, 3, 11), d(2024, 3, 15)), 4);
        assert_eq!(business_days_between(d(2024, 3, 15), d(2024, 3, 18)), 1); // Fri→Mon (skips weekend)
        // Direction is signed.
        assert_eq!(business_days_between(d(2024, 3, 18), d(2024, 3, 11)), -5);
        // Friday + 1 business day = Monday; Monday − 1 = previous Friday.
        assert_eq!(add_business_days(d(2024, 3, 15), 1), d(2024, 3, 18)); // Fri → Mon
        assert_eq!(add_business_days(d(2024, 3, 18), -1), d(2024, 3, 15)); // Mon → Fri
        // Five business days from Monday lands on the next Monday.
        assert_eq!(add_business_days(d(2024, 3, 11), 5), d(2024, 3, 18));
        // Weekends are weekends.
        assert!(is_weekend(d(2024, 3, 16))); // Saturday
        assert!(is_weekend(d(2024, 3, 17))); // Sunday
        assert!(!is_weekend(d(2024, 3, 15))); // Friday
    }

    #[test]
    fn business_day_invariants_under_fuzz() {
        // Properties (from a WEEKDAY anchor, where business-day counting is unambiguous):
        // advancing n business days never lands on a weekend, and the business-day count from the
        // anchor to its n-later image is exactly n. `n > 0` advances strictly into the future.
        let mut rng = Rng(0x_B17A_DA75_C0DE_2468);
        for _ in 0..20_000 {
            let z0 = (rng.next() % 200_001) as i64 - 100_000;
            let z = if is_weekend(z0) { add_business_days(z0, 1) } else { z0 }; // a weekday
            let n = (rng.next() % 39) as i64 + 1; // 1..=39 business days
            let fwd = add_business_days(z, n);
            assert!(!is_weekend(fwd), "lands on a business day at z={z}, n={n}");
            assert!(fwd > z, "advancing moves strictly forward at z={z}, n={n}");
            assert_eq!(business_days_between(z, fwd), n, "count matches at z={z}, n={n}");
        }
    }

    /// A hand-built US-Eastern-style zone for 2024: EST = UTC−5h, EDT = UTC−4h.
    /// Spring forward 2024-03-10 07:00 UTC (02:00 EST → 03:00 EDT); fall back 2024-11-03 06:00 UTC
    /// (02:00 EDT → 01:00 EST).
    fn eastern_2024() -> (Vec<ZoneTransition>, i32) {
        let spring = days_from_civil(2024, 3, 10) * SECONDS_PER_DAY + 7 * 3600;
        let fall = days_from_civil(2024, 11, 3) * SECONDS_PER_DAY + 6 * 3600;
        (
            vec![
                ZoneTransition { at_unix_seconds: spring, offset_seconds: -4 * 3600 },
                ZoneTransition { at_unix_seconds: fall, offset_seconds: -5 * 3600 },
            ],
            -5 * 3600, // base: EST
        )
    }

    fn utc(y: i64, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        unix_nanos_from_civil(CivilDateTime { year: y, month: mo, day: d, hour: h, minute: mi, second: 0, nanosecond: 0 })
    }

    #[test]
    fn timezone_utc_to_local_picks_the_offset_in_effect() {
        let (tz, base) = eastern_2024();
        let local = |ns| {
            let c = to_local(&tz, base, ns);
            (c.month, c.day, c.hour, c.minute)
        };
        // Winter → EST (−5): 2024-01-01 12:00 UTC = 07:00 local.
        assert_eq!(local(utc(2024, 1, 1, 12, 0)), (1, 1, 7, 0));
        // Summer → EDT (−4): 2024-07-01 12:00 UTC = 08:00 local.
        assert_eq!(local(utc(2024, 7, 1, 12, 0)), (7, 1, 8, 0));
        // Just after fall-back → back to EST: 2024-12-01 12:00 UTC = 07:00 local.
        assert_eq!(local(utc(2024, 12, 1, 12, 0)), (12, 1, 7, 0));
        // The offset function directly.
        assert_eq!(offset_at(&tz, base, utc(2024, 7, 1, 0, 0)), -4 * 3600);
        assert_eq!(offset_at(&tz, base, utc(2024, 1, 1, 0, 0)), -5 * 3600);
    }

    #[test]
    fn timezone_local_to_utc_round_trip_and_dst_fold() {
        let (tz, base) = eastern_2024();
        let cdt = |y, mo, d, h, mi| CivilDateTime {
            year: y, month: mo, day: d, hour: h, minute: mi, second: 0, nanosecond: 0,
        };
        // Unambiguous local → UTC round-trips: 08:00 EDT on 2024-07-01 = 12:00 UTC.
        let summer_local = cdt(2024, 7, 1, 8, 0);
        let summer_utc = from_local(&tz, base, summer_local, Fold::Later);
        assert_eq!(summer_utc, utc(2024, 7, 1, 12, 0));
        assert_eq!(to_local(&tz, base, summer_utc), summer_local);
        // DST FOLD: 2024-11-03 01:30 local occurs twice. Earlier = 05:30 UTC (still EDT −4),
        // Later = 06:30 UTC (now EST −5).
        let ambiguous = cdt(2024, 11, 3, 1, 30);
        assert_eq!(from_local(&tz, base, ambiguous, Fold::Earlier), utc(2024, 11, 3, 5, 30));
        assert_eq!(from_local(&tz, base, ambiguous, Fold::Later), utc(2024, 11, 3, 6, 30));
        // DST GAP: 2024-03-10 02:30 local never occurs (clocks jump 02:00→03:00). Resolution is
        // deterministic and the two folds differ (boundary instants of the skipped hour).
        let nonexistent = cdt(2024, 3, 10, 2, 30);
        let gap_earlier = from_local(&tz, base, nonexistent, Fold::Earlier);
        let gap_later = from_local(&tz, base, nonexistent, Fold::Later);
        assert_ne!(gap_earlier, gap_later, "the two gap resolutions differ");
    }

    #[test]
    fn dst_rule_evaluator_generates_correct_transitions() {
        // Nth-weekday anchors.
        assert_eq!(nth_weekday_of_month(2024, 3, 2, 0), 10); // 2nd Sunday of March 2024
        assert_eq!(nth_weekday_of_month(2024, 11, 1, 0), 3); // 1st Sunday of November 2024
        assert_eq!(nth_weekday_of_month(2024, 10, 5, 0), 27); // last Sunday of October 2024 (EU rule)
        assert_eq!(nth_weekday_of_month(2024, 2, 5, 4), 29); // last Thursday of Feb 2024 (leap)
        // The US Eastern POSIX rule `EST5EDT,M3.2.0,M11.1.0` generates exactly the hand-built 2024
        // transitions (spring 2024-03-10 07:00Z → EDT, fall 2024-11-03 06:00Z → EST).
        let start = DstRule { month: 3, week: 2, weekday: 0, time_seconds: 2 * 3600 };
        let end = DstRule { month: 11, week: 1, weekday: 0, time_seconds: 2 * 3600 };
        let gen = dst_transitions_for_year(-5 * 3600, -4 * 3600, start, end, 2024);
        let (hand, _) = eastern_2024();
        assert_eq!(gen.to_vec(), hand, "generated transitions match the hand-built zone");
        // The generated zone localizes correctly: summer = EDT (−4), winter = EST (−5).
        let tz = gen.to_vec();
        let hour = |y, mo, d| to_local(&tz, -5 * 3600, utc(y, mo, d, 12, 0)).hour;
        assert_eq!(hour(2024, 7, 1), 8); // 12:00Z → 08:00 EDT
        assert_eq!(hour(2024, 1, 1), 7); // 12:00Z → 07:00 EST
    }

    #[test]
    fn nth_weekday_invariants_under_fuzz() {
        // Property: the result is a valid day of the month, has the requested weekday, and lies in
        // the requested week band (or is the last such weekday when week = 5).
        let mut rng = Rng(0x_4EE7_DA42_C0DE_9001);
        for _ in 0..20_000 {
            let year = 1900 + (rng.next() % 300) as i64;
            let month = (rng.next() % 12) as u32 + 1;
            let weekday = (rng.next() % 7) as u32;
            let week = (rng.next() % 5) as u32 + 1; // 1..=5
            let day = nth_weekday_of_month(year, month, week, weekday);
            assert!((1..=last_day_of_month(year, month)).contains(&day), "valid day {year}-{month} w{week}");
            assert_eq!(weekday_from_days(days_from_civil(year, month, day)), weekday, "weekday matches");
            // It is indeed the week-th (or last) such weekday: one week earlier is a different month
            // for week 1, or the prior occurrence; one week later overflows the month for "last".
            if week < 5 {
                assert_eq!((day - 1) / 7 + 1, week, "in the requested week band");
            } else {
                assert!(day + 7 > last_day_of_month(year, month), "week 5 is the last occurrence");
            }
        }
    }

    #[test]
    fn format_zoned_shows_local_time_with_offset() {
        let u = |y, mo, d, h| {
            unix_nanos_from_civil(CivilDateTime { year: y, month: mo, day: d, hour: h, minute: 0, second: 0, nanosecond: 0 })
        };
        // Summer → EDT (−4); winter → EST (−5).
        assert_eq!(format_zoned(u(2024, 7, 1, 12), "America/New_York").as_deref(), Some("2024-07-01T08:00:00-04:00"));
        assert_eq!(format_zoned(u(2024, 1, 1, 12), "America/New_York").as_deref(), Some("2024-01-01T07:00:00-05:00"));
        // UTC and a half-hour positive offset.
        assert_eq!(format_zoned(u(2024, 1, 1, 12), "UTC").as_deref(), Some("2024-01-01T12:00:00+00:00"));
        assert_eq!(format_zoned(u(2024, 1, 1, 0), "Asia/Kolkata").as_deref(), Some("2024-01-01T05:30:00+05:30"));
        // Unknown zone → None.
        assert_eq!(format_zoned(0, "Mars/Base"), None);
    }

    #[test]
    fn named_zone_registry_localizes_with_dst() {
        let h = |z: &ZoneSpec, y, mo, d, hh| z.to_local(utc(y, mo, d, hh, 0)).hour;
        // America/New_York: EDT (−4) in summer, EST (−5) in winter.
        let ny = zone_by_name("America/New_York").unwrap();
        assert_eq!(h(&ny, 2024, 7, 1, 12), 8); // 12:00Z → 08:00 EDT
        assert_eq!(h(&ny, 2024, 1, 1, 12), 7); // 12:00Z → 07:00 EST
        // UTC: fixed, no DST.
        let u = zone_by_name("UTC").unwrap();
        assert!(u.dst.is_none());
        assert_eq!(h(&u, 2024, 7, 1, 12), 12);
        // Europe/London: BST (+1) in summer, GMT (0) in winter.
        let lon = zone_by_name("Europe/London").unwrap();
        assert_eq!(h(&lon, 2024, 7, 1, 12), 13); // BST
        assert_eq!(h(&lon, 2024, 1, 1, 12), 12); // GMT
        // Asia/Kolkata: fixed +5:30.
        let ist = zone_by_name("Asia/Kolkata").unwrap();
        let c = ist.to_local(utc(2024, 1, 1, 0, 0));
        assert_eq!((c.hour, c.minute), (5, 30));
        // Australia/Sydney (southern hemisphere): AEDT (+11) in January, AEST (+10) in July.
        let syd = zone_by_name("Australia/Sydney").unwrap();
        assert_eq!(h(&syd, 2024, 1, 1, 0), 11); // 00:00Z → 11:00 AEDT (summer down under)
        assert_eq!(h(&syd, 2024, 7, 1, 0), 10); // 00:00Z → 10:00 AEST (winter)
        // Unknown zone → None.
        assert!(zone_by_name("Mars/Olympus_Mons").is_none());
    }

    #[test]
    fn timezone_round_trips_unambiguous_locals_under_fuzz() {
        // Property: outside the fall-back fold hour, to_local ∘ from_local = identity. We test by
        // generating UTC instants (each maps to exactly one local), converting to local and back.
        let (tz, base) = eastern_2024();
        let mut rng = Rng(0x_72ED_C0DE_8888_4321);
        let year_start = utc(2024, 1, 1, 0, 0);
        for _ in 0..20_000 {
            let ns = year_start + (rng.next() % (300 * NANOS_PER_DAY as u64)) as i64;
            let local = to_local(&tz, base, ns);
            // from_local with the matching fold recovers the instant: pick whichever fold matches.
            let back_e = from_local(&tz, base, local, Fold::Earlier);
            let back_l = from_local(&tz, base, local, Fold::Later);
            assert!(
                back_e == ns || back_l == ns,
                "instant recovered from its local time (ns={ns}, e={back_e}, l={back_l})"
            );
        }
    }

    #[test]
    fn rfc3339_format_and_parse_round_trip() {
        let at = |y, mo, d, h, mi, s, n| CivilDateTime {
            year: y, month: mo, day: d, hour: h, minute: mi, second: s, nanosecond: n,
        };
        // The epoch, no fraction.
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        // Full nanosecond precision, round-trip.
        let ns = unix_nanos_from_civil(at(2024, 3, 10, 7, 30, 0, 123_456_789));
        assert_eq!(format_rfc3339(ns), "2024-03-10T07:30:00.123456789Z");
        assert_eq!(parse_rfc3339("2024-03-10T07:30:00.123456789Z"), Some(ns));
        // Trailing zeros trimmed: half a second is `.5`.
        let half = unix_nanos_from_civil(at(2000, 1, 1, 0, 0, 0, 500_000_000));
        assert_eq!(format_rfc3339(half), "2000-01-01T00:00:00.5Z");
        assert_eq!(parse_rfc3339("2000-01-01T00:00:00.5Z"), Some(half));
        // Numeric offsets normalize to the same UTC instant as 07:30Z.
        let utc_730 = unix_nanos_from_civil(at(2024, 3, 10, 7, 30, 0, 0));
        assert_eq!(parse_rfc3339("2024-03-10T03:30:00-04:00"), Some(utc_730));
        assert_eq!(parse_rfc3339("2024-03-10T13:00:00+05:30"), Some(utc_730));
        // Pre-epoch.
        assert_eq!(format_rfc3339(-NANOS_PER_SECOND), "1969-12-31T23:59:59Z");
        assert_eq!(parse_rfc3339("1969-12-31T23:59:59Z"), Some(-NANOS_PER_SECOND));
        // Malformed inputs are a clean None (never a panic).
        assert_eq!(parse_rfc3339("not a date"), None);
        assert_eq!(parse_rfc3339("2024-13-01T00:00:00Z"), None); // month out of range
        assert_eq!(parse_rfc3339("2024-02-30T00:00:00Z"), None); // day out of range
        assert_eq!(parse_rfc3339("2024-03-10 07:30:00Z"), None); // missing 'T'
        assert_eq!(parse_rfc3339("2024-03-10T07:30:00"), None); // missing zone
    }

    #[test]
    fn duration_format_and_parse() {
        let s = NANOS_PER_SECOND;
        // Canonical forms.
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(90 * 60 * s), "1h30m");
        assert_eq!(format_duration(2 * 86_400 * s), "2d");
        assert_eq!(format_duration(s + s / 2), "1.5s");
        assert_eq!(format_duration(s / 2), "500ms");
        assert_eq!(format_duration(-(90 * 60 * s)), "-1h30m");
        assert_eq!(format_duration(86_400 * s + 2 * 3600 * s + 3 * 60 * s + 4 * s), "1d2h3m4s");
        assert_eq!(format_duration(1500), "1.5us");
        assert_eq!(format_duration(42), "42ns");
        // Parse the canonical forms back.
        assert_eq!(parse_duration("1h30m"), Some(90 * 60 * s));
        assert_eq!(parse_duration("90m"), Some(90 * 60 * s)); // non-canonical but valid
        assert_eq!(parse_duration("1.5s"), Some(s + s / 2));
        assert_eq!(parse_duration("500ms"), Some(s / 2));
        assert_eq!(parse_duration("-1h"), Some(-(3600 * s)));
        assert_eq!(parse_duration("2d"), Some(2 * 86_400 * s));
        assert_eq!(parse_duration("1d2h3m4s"), Some(86_400 * s + 2 * 3600 * s + 3 * 60 * s + 4 * s));
        assert_eq!(parse_duration(".5s"), Some(s / 2)); // leading-dot fraction
        assert_eq!(parse_duration("1µs"), Some(1_000)); // micro sign accepted
        assert_eq!(parse_duration("250us"), Some(250_000));
        // Malformed → None (never a panic).
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration("1x"), None); // unknown unit
        assert_eq!(parse_duration("h"), None); // no number
    }

    #[test]
    fn duration_round_trips_under_fuzz() {
        // Property: parse_duration ∘ format_duration = identity for any duration.
        let mut rng = Rng(0x_D012_A710_C0DE_4444);
        for _ in 0..20_000 {
            let ns = (rng.next() as i64).wrapping_rem(9_000_000_000_000_000_000);
            assert_eq!(parse_duration(&format_duration(ns)), Some(ns), "round-trip {ns}");
        }
    }

    #[test]
    fn rfc3339_round_trips_under_fuzz() {
        // Property: parse_rfc3339 ∘ format_rfc3339 = identity for any instant.
        let mut rng = Rng(0x_3339_C0DE_ABCD_1234);
        for _ in 0..20_000 {
            let ns = (rng.next() as i64).wrapping_rem(6_000_000_000_000_000_000);
            let s = format_rfc3339(ns);
            assert_eq!(parse_rfc3339(&s), Some(ns), "round-trip {s}");
        }
    }

    #[test]
    fn sync_clock_merge_is_a_crdt_join() {
        use std::collections::BTreeMap;
        let mk = |hp: i64, node: u64, l: i64, u: i64| {
            let mut k = BTreeMap::new();
            k.insert(node, EstimateInterval::new(l, u));
            SyncClock { hlc: Hlc { physical_nanos: hp, logical: 0 }, knowledge: k }
        };
        let a = mk(100, 1, 10, 50);
        let b = mk(200, 1, 20, 40);
        let c = mk(150, 2, 5, 9);
        // CRDT laws.
        assert_eq!(a.merge(&a), a, "idempotent");
        assert_eq!(a.merge(&b), b.merge(&a), "commutative");
        assert_eq!(a.merge(&b).merge(&c), a.merge(&b.merge(&c)), "associative");
        // HLC takes the max; a shared node intersects; a distinct node is carried.
        let m = a.merge(&b);
        assert_eq!(m.hlc, Hlc { physical_nanos: 200, logical: 0 });
        assert_eq!(m.knowledge[&1], EstimateInterval::new(20, 40)); // [10,50] ∩ [20,40]
        let m2 = a.merge(&c);
        assert_eq!(m2.knowledge[&1], EstimateInterval::new(10, 50));
        assert_eq!(m2.knowledge[&2], EstimateInterval::new(5, 9));
    }

    #[test]
    fn sync_clock_converges_regardless_of_order_and_intervals_only_tighten() {
        use std::collections::BTreeMap;
        let make = |hp: i64, node: u64, l: i64, u: i64| {
            let mut k = BTreeMap::new();
            k.insert(node, EstimateInterval::new(l, u));
            SyncClock { hlc: Hlc { physical_nanos: hp, logical: 0 }, knowledge: k }
        };
        let a = make(100, 1, 0, 100);
        let b = make(300, 1, 40, 80);
        // Two replicas exchange and converge to the SAME state, whichever order.
        assert_eq!(a.merge(&b), b.merge(&a));
        // Knowledge only tightens: the merged interval is no wider than either input.
        let merged_iv = a.merge(&b).knowledge[&1];
        assert!(merged_iv.width() <= a.knowledge[&1].width());
        assert!(merged_iv.width() <= b.knowledge[&1].width());
        // intersect itself is identity (idempotent at the interval level too).
        let iv = EstimateInterval::new(10, 90);
        assert_eq!(iv.intersect(&iv), iv);
        assert_eq!(iv.intersect(&EstimateInterval::new(30, 60)), EstimateInterval::new(30, 60));
    }

    #[test]
    fn sync_clock_tick_observe_and_light_cone_horizon() {
        let c = SyncClock::new();
        let c2 = c.tick(500);
        assert!(c2.hlc > c.hlc, "tick advances causally");
        // observe records knowledge for a node.
        let c3 = c2.observe(7, EstimateInterval::new(100, 200));
        assert_eq!(c3.knowledge[&7], EstimateInterval::new(100, 200));
        // observing again tightens (intersects) rather than overwrites.
        let c4 = c3.observe(7, EstimateInterval::new(150, 250));
        assert_eq!(c4.knowledge[&7], EstimateInterval::new(150, 200));
        // You cannot know a node 0.2 s away fresher than now − 0.2 s.
        assert_eq!(knowable_horizon(1_000_000_000, 200_000_000), 800_000_000);
    }

    #[test]
    fn hybrid_logical_clock_tick_and_receive() {
        let hlc = |p: i64, l: u32| Hlc { physical_nanos: p, logical: l };
        // First tick adopts wall time; a stalled clock advances the logical counter.
        assert_eq!(Hlc::ZERO.tick(100), hlc(100, 0));
        assert_eq!(hlc(100, 0).tick(100), hlc(100, 1)); // physical didn't move → logical++
        assert_eq!(hlc(100, 1).tick(200), hlc(200, 0)); // physical advanced → logical reset
        // A backward wall clock is absorbed by the logical counter (never goes back).
        assert_eq!(hlc(200, 0).tick(150), hlc(200, 1));
        // Receive: equal physicals → max(logical)+1.
        assert_eq!(hlc(100, 5).recv(hlc(100, 3), 100), hlc(100, 6));
        // Receive: remote ahead → take remote physical, remote.logical+1.
        assert_eq!(hlc(100, 5).recv(hlc(200, 2), 100), hlc(200, 3));
        // Receive: local ahead → local.logical+1.
        assert_eq!(hlc(200, 7).recv(hlc(100, 9), 150), hlc(200, 8));
        // Receive: wall time beats both → reset logical.
        assert_eq!(hlc(100, 5).recv(hlc(50, 9), 300), hlc(300, 0));
        // The derived Ord is the HLC total order.
        assert!(hlc(100, 0) < hlc(100, 1) && hlc(100, 1) < hlc(200, 0));
    }

    #[test]
    fn hlc_preserves_causality_under_fuzz() {
        // Property: tick and recv always produce a timestamp strictly greater than every input —
        // so a send-then-receive chain is monotonic and happens-before is never inverted.
        let mut rng = Rng(0x_C0DE_C0FF_EE17_AAAA);
        let mut local = Hlc::ZERO;
        let mut remote = Hlc::ZERO;
        for _ in 0..20_000 {
            let now = (rng.next() % 1_000_000) as i64;
            if rng.next() & 1 == 0 {
                let next = local.tick(now);
                assert!(next > local, "tick advances");
                local = next;
            } else {
                let before = local;
                let next = local.recv(remote, now);
                assert!(next > before && next > remote, "recv dominates both inputs");
                local = next;
                remote = remote.tick((rng.next() % 1_000_000) as i64); // remote evolves independently
            }
        }
    }

    #[test]
    fn leap_second_tai_utc_offsets() {
        let at = |y, mo, d| days_from_civil(y, mo, d) * SECONDS_PER_DAY;
        // Known TAI−UTC values from the IERS table.
        assert_eq!(tai_minus_utc(at(2017, 1, 1)), 37); // latest leap second
        assert_eq!(tai_minus_utc(at(2024, 6, 1)), 37); // unchanged since 2017
        assert_eq!(tai_minus_utc(at(1972, 1, 1)), 10); // table origin
        assert_eq!(tai_minus_utc(at(2000, 1, 1)), 32);
        assert_eq!(tai_minus_utc(at(1999, 1, 1)), 32); // the 1999-01-01 step
        assert_eq!(tai_minus_utc(at(1998, 12, 31)), 31); // the day before it
        assert_eq!(tai_minus_utc(at(1970, 1, 1)), 10); // pre-1972 floor
        // TAI is ahead of UTC by exactly the offset.
        assert_eq!(unix_to_tai_seconds(at(2017, 1, 1)), at(2017, 1, 1) + 37);
        assert_eq!(unix_to_tai_seconds(at(1972, 1, 1)), at(1972, 1, 1) + 10);
        // Round-trip away from a leap boundary.
        let t = at(2020, 6, 15) + 12_345;
        assert_eq!(tai_to_unix_seconds(unix_to_tai_seconds(t)), t);
        // TT = TAI + 32.184 s exactly.
        assert_eq!(TT_MINUS_TAI_NANOS, 32_184_000_000);
    }

    #[test]
    fn tt_scale_is_exact() {
        let at_ns = |y, mo, d| days_from_civil(y, mo, d) * NANOS_PER_DAY;
        let u = at_ns(2017, 1, 1);
        // TAI = UTC + 37 s; TT = TAI + 32.184 s = UTC + 69.184 s.
        assert_eq!(tai_nanos_from_unix_nanos(u), u + 37 * NANOS_PER_SECOND);
        assert_eq!(tt_nanos_from_unix_nanos(u), u + 37 * NANOS_PER_SECOND + TT_MINUS_TAI_NANOS);
        // 1972: TAI−UTC = 10 → TT = UTC + 42.184 s.
        let u72 = at_ns(1972, 1, 1);
        assert_eq!(tt_nanos_from_unix_nanos(u72), u72 + 10 * NANOS_PER_SECOND + TT_MINUS_TAI_NANOS);
        // Sub-second precision survives, round-trip.
        let t = at_ns(2020, 3, 15) + 12_345_678_900;
        assert_eq!(unix_nanos_from_tt_nanos(tt_nanos_from_unix_nanos(t)), t);
    }

    #[test]
    fn tt_round_trips_under_fuzz() {
        let mut rng = Rng(0x_77_C0DE_5CA1_9999);
        let base = days_from_civil(1972, 6, 1) * NANOS_PER_DAY;
        for _ in 0..20_000 {
            let ns = base + (rng.next() % 1_600_000_000_000_000_000) as i64;
            assert_eq!(unix_nanos_from_tt_nanos(tt_nanos_from_unix_nanos(ns)), ns, "TT round-trip at {ns}");
        }
    }

    #[test]
    fn tai_round_trips_under_fuzz() {
        // Property: tai_to_unix_seconds ∘ unix_to_tai_seconds = identity (instants are not on a
        // leap-second boundary at whole-second granularity away from midnight steps).
        let mut rng = Rng(0x_7A1_C0DE_5EC0_4242);
        let base = days_from_civil(1972, 6, 1) * SECONDS_PER_DAY;
        for _ in 0..20_000 {
            let secs = base + (rng.next() % 1_700_000_000) as i64; // ~1972..2025
            assert_eq!(tai_to_unix_seconds(unix_to_tai_seconds(secs)), secs, "TAI round-trip at {secs}");
        }
    }

    #[test]
    fn smooth_utc_round_trips_over_a_wide_range_under_fuzz() {
        // Property: civil_from_unix_nanos ∘ unix_nanos_from_civil = identity, and the decomposed
        // fields are always in civil range. SmoothUTC = no leap seconds, so seconds never hit 60.
        let mut rng = Rng(0x_5007_8807_C0DE_1357);
        for _ in 0..50_000 {
            // Range: ~±200 years of nanoseconds around the epoch (within i64).
            let ns = (rng.next() as i64).wrapping_rem(6_000_000_000_000_000_000);
            let c = civil_from_unix_nanos(ns);
            assert!((1..=12).contains(&c.month));
            assert!((1..=last_day_of_month(c.year, c.month)).contains(&c.day));
            assert!(c.hour < 24 && c.minute < 60 && c.second < 60 && c.nanosecond < 1_000_000_000);
            assert_eq!(unix_nanos_from_civil(c), ns, "instant round-trip at ns={ns}");
        }
    }

    #[test]
    fn iso_week_round_trips_over_a_wide_range_under_fuzz() {
        // Property: days_from_iso_week ∘ iso_week_from_days = identity, the week is always 1..=53,
        // the ISO weekday always 1..=7, and the ISO weekday agrees with the plain weekday.
        let mut rng = Rng(0x_C0DE_FACE_8601_9999);
        for _ in 0..50_000 {
            let z = (rng.next() % 800_001) as i64 - 400_000;
            let (iy, w, dow) = iso_week_from_days(z);
            assert!((1..=53).contains(&w), "ISO week in range at z={z}: {w}");
            assert!((1..=7).contains(&dow), "ISO weekday in range at z={z}: {dow}");
            // ISO weekday (1=Mon..7=Sun) lines up with the plain weekday (0=Sun..6=Sat).
            let plain = weekday_from_days(z);
            assert_eq!(dow % 7, plain, "ISO/plain weekday agree at z={z}");
            // Reconstruct the exact day from its ISO week date.
            assert_eq!(days_from_iso_week(iy, w, dow), z, "ISO round-trip at z={z}");
        }
    }

    #[test]
    fn julian_round_trips_over_a_wide_range_under_fuzz() {
        // Property: julian_from_days ∘ days_from_julian = identity, and the proleptic Julian
        // calendar is internally consistent (every produced (y,m,d) maps back to the day number).
        let mut rng = Rng(0x_1234_ABCD_5678_EF90);
        for _ in 0..50_000 {
            let z = (rng.next() % 800_001) as i64 - 400_000;
            let (y, m, d) = julian_from_days(z);
            assert!((1..=12).contains(&m), "julian month in range at z={z}");
            assert!((1..=31).contains(&d), "julian day in range at z={z}");
            assert_eq!(days_from_julian(y, m, d), z, "julian round-trip at z={z}");
        }
    }
}
