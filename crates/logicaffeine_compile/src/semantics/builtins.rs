//! Built-in functions over already-evaluated values.
//!
//! `show` is NOT here — output is an engine concern. Arity is checked by the
//! caller BEFORE evaluating arguments (via [`check_arity`]) to preserve the
//! tree-walker's error ordering: a wrong-arity call reports the arity error
//! even when an argument expression would itself error.

use std::cell::RefCell;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::interpreter::{ListRepr, RuntimeValue};
use logicaffeine_base::{Decimal, LanesVal, Word16, Word32, Word64, WordVal};

/// Read a `Seq of Int` as raw bytes (each element masked to 0–255) — the byte-array convention the
/// UUID/hash builtins share with `uuid.lg` so the version constructors can be written in LOGOS.
fn byte_seq(v: &RuntimeValue) -> Result<Vec<u8>, String> {
    match v {
        RuntimeValue::List(l) => {
            let l = l.borrow();
            let mut out = Vec::with_capacity(l.len());
            for i in 0..l.len() {
                match l.get(i) {
                    Some(RuntimeValue::Int(n)) => out.push((n & 0xff) as u8),
                    _ => return Err(format!("expected a Seq of Int (bytes); element {} is not an Int", i + 1)),
                }
            }
            Ok(out)
        }
        _ => Err(format!("expected a Seq of Int (bytes), got {}", v.type_name())),
    }
}

/// Build a `Seq of Int` from raw bytes (the packed-`i64` list repr).
fn bytes_to_seq(bytes: &[u8]) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(
        bytes.iter().map(|&b| b as i64).collect(),
    ))))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuiltinId {
    Length,
    Format,
    ParseInt,
    ParseFloat,
    Chr,
    Abs,
    Sqrt,
    Min,
    Max,
    Floor,
    Ceil,
    Round,
    Pow,
    /// `decimal(text)` / `decimal(int)` — construct an exact base-10 fixed-point (money)
    /// from its literal text (`decimal("19.99")`) or an integer. The non-breaking entry
    /// into the `Decimal` tower (it does NOT change how `19.99` literals parse).
    Decimal,
    /// `complex(re, im)` — construct an exact complex number from two exact reals.
    /// `complex(0, 1)` is the imaginary unit `i`; `complex(0,1) * complex(0,1) = −1`.
    Complex,
    /// `modular(value, modulus)` — construct an element of ℤ/nℤ (the value reduced mod n).
    Modular,
    /// `quantity(value, "unit")` — construct a dimensioned physical quantity (`quantity(2, "inch")`).
    /// The magnitude rides the exact rational tower; the unit name resolves through the catalog.
    Quantity,
    /// `money(amount, "USD")` — construct an exact monetary amount, quantised to the currency's
    /// minor unit. The amount rides the Decimal tower (never float-drifts); the code resolves through
    /// the ISO-4217 catalog.
    Money,
    /// `set_rate("EUR", 1.10)` — install/replace one exchange rate (vs the reference) in the ambient
    /// rate context that `<money> in <currency>` reads. A side effect; returns nothing.
    SetRate,
    /// `to_currency(money, "EUR")` — convert money to another currency via the ambient rate context
    /// (exact). Errors if no rates are in scope or the currency is unknown. Surface: `<money> in EUR`.
    ToCurrency,
    /// `set_rates(map)` — bulk-install a whole exchange-rate table from a `Map of Text to <number>`
    /// (code → rate vs the reference) into the ambient rate context. The bridge a literal,
    /// network-synced, or fetched rate table feeds. A side effect; returns nothing.
    SetRates,
    /// `uuid("550e8400-…")` — parse a UUID from text (canonical/simple/braced/urn); error on a bad id.
    Uuid,
    /// `uuid_nil()` / `uuid_max()` — the two special ids.
    UuidNil,
    UuidMax,
    /// `uuid_version(u)` — the version nibble (0 nil, 1–8, 15 max).
    UuidVersion,
    /// The well-known namespace ids: `uuid_dns()`, `uuid_url()`, `uuid_oid()`, `uuid_x500()`.
    UuidDns,
    UuidUrl,
    UuidOid,
    UuidX500,
    /// Byte-level primitives that let the UUID version constructors be written IN LOGOS (uuid.lg):
    /// `text_bytes` is a Text's UTF-8 bytes; `uuid_bytes`/`uuid_from_bytes` convert a Uuid to/from its
    /// 16 bytes. (`md5`/`sha1`/`uuid_v3`/`uuid_v5` are Logos stdlib functions now, not builtins.)
    TextBytes,
    /// `text_from_bytes(seq)` — the exact inverse of `text_bytes`: rebuild a Text from its UTF-8
    /// `Seq of Int`. The wire medium for the Core-IR codec (`Seq of Int`) carries Text this way.
    TextFromBytes,
    /// `wireBytes(value)` — the value's plain wire form (peer codec `encode_value_raw`) as a
    /// `Seq of Int`. Lets the host serialize a program AST for a compile-once native PE, byte-
    /// identical to what the native binary's generated `wire_decode` reads.
    WireBytes,
    /// `readWireProgram()` — read ONE length-framed `CProgram` from stdin (`u32` LE length, then
    /// that many wire bytes, decoded via the generated `wire_decode`) and return it; on EOF the
    /// process exits cleanly (the resident-server loop terminator). AOT-only (returns the generated
    /// `CProgram`); the interpreter variant rebuilds a `RuntimeValue` for parity.
    ReadWireProgram,
    /// `writeWireResidual(text)` — write `text` back as a length-framed residual (`u32` LE length,
    /// then the UTF-8 bytes) to stdout and flush, returning the byte count. The response half of the
    /// resident PE server's request/response protocol.
    WriteWireResidual,
    UuidBytes,
    UuidFromBytes,
    /// `convert(quantity, "unit")` — re-express a quantity in another unit of the SAME dimension
    /// (`convert(q, "foot")`); a different dimension is a clean error (the forbidden cast).
    Convert,
    /// `parse_timestamp("2024-03-10T07:30:00Z")` — parse an RFC 3339 / ISO 8601 timestamp into a
    /// `Moment` (nanoseconds since the epoch). The wire-first "the timestamp is its own type" entry.
    ParseTimestamp,
    /// `format_timestamp(moment)` — render a `Moment` as an RFC 3339 / ISO 8601 UTC string.
    FormatTimestamp,
    /// Calendar component extractors on a `Moment` (UTC), each returning an `Int`:
    /// `year_of` / `month_of` (1–12) / `day_of` (1–31) / `weekday_of` (0 = Sunday … 6 = Saturday).
    YearOf,
    MonthOf,
    DayOf,
    WeekdayOf,
    HourOf,
    MinuteOf,
    SecondOf,
    /// `week_of(moment)` — the ISO-8601 week number (1..=53) of the Moment/Date (UTC).
    WeekOf,
    /// `quarter_of(moment)` — the calendar quarter (1..=4) of the Moment/Date (UTC).
    QuarterOf,
    /// `date_of(moment)` — the calendar day (a `Date`) the Moment falls on (UTC); identity on a Date.
    DateOf,
    /// `time_of(moment)` — the wall-clock time-of-day (a `Time`) of the Moment (UTC).
    TimeOf,
    /// `local_instant(moment, "zone")` — the local-as-UTC instant in a named zone (the lowering
    /// target for `the <comp> of <m> in "<zone>"`, so every extractor reads the LOCAL component).
    LocalInstant,
    /// `seconds_between(a, b)` — whole seconds from Moment `a` to Moment `b` (signed `Int`).
    SecondsBetween,
    /// `months_between(a, b)` / `years_between(a, b)` — complete calendar months/years (signed `Int`).
    MonthsBetween,
    YearsBetween,
    /// `add_seconds(moment, n)` — the Moment `n` seconds after `moment`.
    AddSeconds,
    /// `in_zone(moment, "America/New_York")` — the local wall-clock time (with offset) of a Moment
    /// in a named IANA zone, as Text. The natural surface is `<moment> in "<zone>"`.
    InZone,
    Copy,
    CountOnes,
    RunAccepted,
    /// `word32(n)` / `word64(n)` — construct a fixed-width wrapping integer from an `Int`.
    Word32,
    Word64,
    /// `rotl(w, n)` / `rotr(w, n)` — rotate a word left/right by `n` bits.
    Rotl,
    Rotr,
    /// `word_and(a, b)` / `word_or(a, b)` / `word_not(a)` — bitwise ops on the Word ring, CONSISTENT
    /// across every tier (the `and`/`or` keywords are logical short-circuit on the VM, so word crypto
    /// written in Logos — MD5/SHA-1 round functions — uses these instead).
    Wand,
    Wor,
    Wnot,
    /// The SHA-1 SHA-NI lane vocabulary: `lanes4Word32(s)`/`seqOfLanes4W32(v)` pack/unpack a
    /// `Lanes4Word32` (128-bit), and `sha1rnds4`/`sha1msg1`/`sha1msg2`/`sha1nexte` are the four SHA-1
    /// operations — the primitives a SHA-1 written IN LOGOS is built from (AOT → hardware `sha1rnds4`).
    Lanes4Word32Make,
    /// Pack four `Word32`s straight into a lane register — the alloc-free constructor (no `Seq`).
    Lanes4Of,
    SeqOfLanes4W32,
    Sha1Rnds4,
    Sha1Msg1,
    Sha1Msg2,
    Sha1Nexte,
    /// Byte-shuffle lane (`Lanes16Word8` = one `__m128i`): pack/unpack + `shuffle` (pshufb), per-byte
    /// shift, and the two interleaves — the vocabulary a SIMD hex codec WRITTEN in Logos is built from.
    Lanes16Word8Make,
    SeqOfLanes16W8,
    Splat16Word8,
    Shuffle16,
    ShrBytes16,
    InterleaveLo16,
    InterleaveHi16,
    ByteAdd16,
    Maddubs16,
    Packus16,
    /// `lanes8Word32(s)` — pack the first 8 `Word32`s of a Seq into a SIMD lane vector.
    Lanes8Word32,
    /// `seqOfLanes8(v)` — unpack a `Lanes8Word32` back into a Seq of 8 `Word32`.
    SeqOfLanes8,
    /// `splat8Word32(x)` — broadcast a `Word32` into all 8 lanes.
    Splat8Word32,
    /// `intOfWord32(w)` — the unsigned value of a `Word32` as an `Int` (for byte serialization).
    IntOfWord32,
    /// `intOfWord64(w)` — the value of a `Word64` as an `Int` (byte-masked lanes in Keccak squeeze).
    IntOfWord64,
    /// `word64Shl(w, n)` — logical shift-left of a `Word64` (Keccak lane byte-packing).
    Word64Shl,
    /// `word64Shr(w, n)` — logical shift-right of a `Word64` (Keccak squeeze byte-extract).
    Word64Shr,
    /// `word64And(a, b)` — bitwise AND of two `Word64`s (Keccak χ's `¬b ∧ c`).
    Word64And,
    /// `word32Shr(w, n)` — logical shift-right of a `Word32` (SHA-256's `σ0`/`σ1`, a non-rotating
    /// shift where the vacated high bits are zero).
    Word32Shr,
    /// `word16(n)` — the low 16 bits of an `Int` as a `Word16` (ℤ/2¹⁶ coefficient).
    Word16Make,
    /// `intOfWord16(w)` — the unsigned value of a `Word16` as an `Int` (0..2¹⁶−1).
    IntOfWord16,
    /// `lanes4Word64(s)` — pack the first 4 `Word64`/`Int`s of a Seq into a `Lanes4Word64`.
    Lanes4Word64,
    /// `seqOfLanes4(v)` — unpack a `Lanes4Word64` into a Seq of 4 `Int` lanes.
    SeqOfLanes4,
    /// `mul32x32to64(a, b)` — lane-wise widening multiply of the low 32 bits (`vpmuludq`).
    Mul32x32To64,
    /// `hsumLanes4(v)` — the horizontal sum of a lane vector's lanes as an `Int`.
    HsumLanes4,
    /// `splat4Word64(x)` — broadcast a `Word64` into all 4 Keccak lanes (ι constant, χ all-ones).
    Splat4Word64,
    /// `andNot4(a, b)` — 4-way Keccak χ's `(¬a) ∧ b` in one lane op (`vpandn`).
    AndNot4,
    /// `lanes16Word16(s)` — pack the first 16 `Word16`/`Int`s of a Seq into a `Lanes16Word16`.
    Lanes16Word16,
    /// `seqOfLanes16(v)` — unpack a `Lanes16Word16` into a Seq of 16 `Int` lanes.
    SeqOfLanes16,
    /// `splat16Word16(x)` — broadcast a `Word16`/`Int` into all 16 lanes.
    Splat16Word16,
    /// `mulhi16(a, b)` — lane-wise SIGNED high-16 multiply (`vpmulhw`, the Montgomery `mulhi`).
    Mulhi16,
    /// `montmul32(a, b, q, qinv)` — the signed i32 Montgomery multiply over 8 lanes (`vpmuldq`), the
    /// ML-DSA (Dilithium) NTT butterfly multiply (`q`/`qinv` broadcast).
    Montmul32,
    /// `nttBcastLo(v, h)` — broadcast each `2h`-block's low `h` lanes into both halves (the
    /// within-vector NTT source-low duplication; `vperm2i128`/`vpshufd` by stride).
    NttBcastLo,
    /// `nttBcastHi(v, h)` — broadcast each `2h`-block's high `h` lanes into both halves.
    NttBcastHi,
    /// `nttBlend(a, b, h)` — each `2h`-block's low `h` from `a`, high `h` from `b` (the butterfly
    /// half-recombine; `vperm2i128`/`vpblendd` by stride).
    NttBlend,
    /// `mapOf(k1, v1, k2, v2, …)` — construct a Map from flat key/value pairs
    /// in INSERTION order (the `{k: v, …}` literal's lowering). A duplicate
    /// key keeps its first position with the last value, like repeated
    /// `Set item k of m` writes.
    MapOf,
    /// `setOf(a, b, …)` — construct a Set from elements in insertion order,
    /// deduplicating by value equality (the `{a, b, …}` literal's lowering).
    SetOf,
    /// `repeatSeq(x, n)` — a fresh sequence of `n` slots, each an INDEPENDENT
    /// deep copy of `x` (the `n copies of x` / `[x] * n` fill; a repeated
    /// inner collection is n rows, never n aliases). `n ≤ 0` is empty.
    RepeatSeq,
}

/// Resolve a function name to a builtin, if it is one.
pub fn builtin_from_name(name: &str) -> Option<BuiltinId> {
    Some(match name {
        "length" => BuiltinId::Length,
        "format" => BuiltinId::Format,
        "mapOf" => BuiltinId::MapOf,
        "setOf" => BuiltinId::SetOf,
        "repeatSeq" => BuiltinId::RepeatSeq,
        "parseInt" => BuiltinId::ParseInt,
        "parseFloat" => BuiltinId::ParseFloat,
        "chr" => BuiltinId::Chr,
        "abs" => BuiltinId::Abs,
        "sqrt" => BuiltinId::Sqrt,
        "min" => BuiltinId::Min,
        "max" => BuiltinId::Max,
        "floor" => BuiltinId::Floor,
        "ceil" => BuiltinId::Ceil,
        "round" => BuiltinId::Round,
        "pow" => BuiltinId::Pow,
        "decimal" => BuiltinId::Decimal,
        "complex" => BuiltinId::Complex,
        "modular" => BuiltinId::Modular,
        "quantity" => BuiltinId::Quantity,
        "money" => BuiltinId::Money,
        "set_rate" => BuiltinId::SetRate,
        "set_rates" => BuiltinId::SetRates,
        "to_currency" => BuiltinId::ToCurrency,
        "uuid" => BuiltinId::Uuid,
        "uuid_nil" => BuiltinId::UuidNil,
        "uuid_max" => BuiltinId::UuidMax,
        "uuid_version" => BuiltinId::UuidVersion,
        "uuid_dns" => BuiltinId::UuidDns,
        "uuid_url" => BuiltinId::UuidUrl,
        "uuid_oid" => BuiltinId::UuidOid,
        "uuid_x500" => BuiltinId::UuidX500,
        "text_bytes" => BuiltinId::TextBytes,
        "text_from_bytes" => BuiltinId::TextFromBytes,
        "wireBytes" => BuiltinId::WireBytes,
        "readWireProgram" => BuiltinId::ReadWireProgram,
        "writeWireResidual" => BuiltinId::WriteWireResidual,
        "uuid_bytes" => BuiltinId::UuidBytes,
        "uuid_from_bytes" => BuiltinId::UuidFromBytes,
        "convert" => BuiltinId::Convert,
        "parse_timestamp" => BuiltinId::ParseTimestamp,
        "format_timestamp" => BuiltinId::FormatTimestamp,
        "year_of" => BuiltinId::YearOf,
        "month_of" => BuiltinId::MonthOf,
        "day_of" => BuiltinId::DayOf,
        "weekday_of" => BuiltinId::WeekdayOf,
        "hour_of" => BuiltinId::HourOf,
        "minute_of" => BuiltinId::MinuteOf,
        "second_of" => BuiltinId::SecondOf,
        "week_of" => BuiltinId::WeekOf,
        "quarter_of" => BuiltinId::QuarterOf,
        "date_of" => BuiltinId::DateOf,
        "time_of" => BuiltinId::TimeOf,
        "local_instant" => BuiltinId::LocalInstant,
        "seconds_between" => BuiltinId::SecondsBetween,
        "months_between" => BuiltinId::MonthsBetween,
        "years_between" => BuiltinId::YearsBetween,
        "add_seconds" => BuiltinId::AddSeconds,
        "in_zone" => BuiltinId::InZone,
        "copy" => BuiltinId::Copy,
        "count_ones" => BuiltinId::CountOnes,
        "run_accepted" => BuiltinId::RunAccepted,
        "word32" => BuiltinId::Word32,
        "word64" => BuiltinId::Word64,
        "lanes8Word32" => BuiltinId::Lanes8Word32,
        "seqOfLanes8" => BuiltinId::SeqOfLanes8,
        "splat8Word32" => BuiltinId::Splat8Word32,
        "intOfWord32" => BuiltinId::IntOfWord32,
        "intOfWord64" => BuiltinId::IntOfWord64,
        "word64Shl" => BuiltinId::Word64Shl,
        "word64Shr" => BuiltinId::Word64Shr,
        "word32Shr" => BuiltinId::Word32Shr,
        "word64And" => BuiltinId::Word64And,
        "word16" => BuiltinId::Word16Make,
        "intOfWord16" => BuiltinId::IntOfWord16,
        "lanes4Word64" => BuiltinId::Lanes4Word64,
        "seqOfLanes4" => BuiltinId::SeqOfLanes4,
        "mul32x32to64" => BuiltinId::Mul32x32To64,
        "hsumLanes4" => BuiltinId::HsumLanes4,
        "splat4Word64" => BuiltinId::Splat4Word64,
        "andNot4" => BuiltinId::AndNot4,
        "lanes16Word16" => BuiltinId::Lanes16Word16,
        "seqOfLanes16" => BuiltinId::SeqOfLanes16,
        "splat16Word16" => BuiltinId::Splat16Word16,
        "mulhi16" => BuiltinId::Mulhi16,
        "montmul32" => BuiltinId::Montmul32,
        "nttBcastLo" => BuiltinId::NttBcastLo,
        "nttBcastHi" => BuiltinId::NttBcastHi,
        "nttBlend" => BuiltinId::NttBlend,
        "rotl" => BuiltinId::Rotl,
        "word_and" => BuiltinId::Wand,
        "word_or" => BuiltinId::Wor,
        "word_not" => BuiltinId::Wnot,
        "lanes4Word32" => BuiltinId::Lanes4Word32Make,
        "lanes4Of" => BuiltinId::Lanes4Of,
        "seqOfLanes4W32" => BuiltinId::SeqOfLanes4W32,
        "sha1rnds4" => BuiltinId::Sha1Rnds4,
        "sha1msg1" => BuiltinId::Sha1Msg1,
        "sha1msg2" => BuiltinId::Sha1Msg2,
        "sha1nexte" => BuiltinId::Sha1Nexte,
        "lanes16Word8" => BuiltinId::Lanes16Word8Make,
        "seqOfLanes16W8" => BuiltinId::SeqOfLanes16W8,
        "splat16Word8" => BuiltinId::Splat16Word8,
        "shuffle16" => BuiltinId::Shuffle16,
        "shrBytes16" => BuiltinId::ShrBytes16,
        "interleaveLo16" => BuiltinId::InterleaveLo16,
        "interleaveHi16" => BuiltinId::InterleaveHi16,
        "byteAdd16" => BuiltinId::ByteAdd16,
        "maddubs16" => BuiltinId::Maddubs16,
        "packus16" => BuiltinId::Packus16,
        "rotr" => BuiltinId::Rotr,
        _ => return None,
    })
}

/// Check the call's arity BEFORE evaluating arguments. `format` accepts any
/// arity (it reads only its first argument, or none).
pub fn check_arity(id: BuiltinId, n: usize) -> Result<(), String> {
    let expected: usize = match id {
        BuiltinId::Format => return Ok(()),
        BuiltinId::MapOf => {
            if n == 0 || n % 2 != 0 {
                return Err(format!(
                    "mapOf takes flat key/value pairs (an even, nonzero number of arguments), got {}",
                    n
                ));
            }
            return Ok(());
        }
        BuiltinId::SetOf => {
            if n == 0 {
                return Err("setOf takes at least one element (an empty set is `{} of T`)".to_string());
            }
            return Ok(());
        }
        BuiltinId::Min | BuiltinId::Max | BuiltinId::Pow => 2,
        BuiltinId::RepeatSeq => 2,
        BuiltinId::Complex => 2,
        BuiltinId::Modular => 2,
        BuiltinId::Quantity | BuiltinId::Convert | BuiltinId::Money => 2,
        BuiltinId::SetRate | BuiltinId::ToCurrency => 2,
        BuiltinId::SetRates => 1,
        BuiltinId::UuidNil
        | BuiltinId::UuidMax
        | BuiltinId::UuidDns
        | BuiltinId::UuidUrl
        | BuiltinId::UuidOid
        | BuiltinId::ReadWireProgram
        | BuiltinId::UuidX500 => 0,
        BuiltinId::Uuid | BuiltinId::UuidVersion => 1,
        BuiltinId::TextBytes | BuiltinId::TextFromBytes | BuiltinId::WireBytes | BuiltinId::WriteWireResidual | BuiltinId::UuidBytes | BuiltinId::UuidFromBytes => 1,
        BuiltinId::SecondsBetween | BuiltinId::AddSeconds | BuiltinId::InZone => 2,
        BuiltinId::MonthsBetween | BuiltinId::YearsBetween => 2,
        BuiltinId::LocalInstant => 2,
        BuiltinId::Rotl | BuiltinId::Rotr => 2,
        BuiltinId::Wand | BuiltinId::Wor => 2,
        BuiltinId::Wnot => 1,
        BuiltinId::Lanes4Word32Make | BuiltinId::SeqOfLanes4W32 => 1,
        BuiltinId::AndNot4 => 2,
        BuiltinId::Lanes4Of => 4,
        BuiltinId::Sha1Rnds4 => 3,
        BuiltinId::Sha1Msg1 | BuiltinId::Sha1Msg2 | BuiltinId::Sha1Nexte => 2,
        BuiltinId::Lanes16Word8Make | BuiltinId::SeqOfLanes16W8 | BuiltinId::Splat16Word8 => 1,
        BuiltinId::Shuffle16 | BuiltinId::ShrBytes16 => 2,
        BuiltinId::InterleaveLo16 | BuiltinId::InterleaveHi16 => 2,
        BuiltinId::ByteAdd16 | BuiltinId::Maddubs16 | BuiltinId::Packus16 => 2,
        BuiltinId::Mul32x32To64 => 2,
        BuiltinId::Mulhi16 => 2,
        BuiltinId::Montmul32 => 4,
        BuiltinId::Word64Shl | BuiltinId::Word64Shr | BuiltinId::Word64And => 2,
        BuiltinId::Word32Shr => 2,
        BuiltinId::NttBcastLo | BuiltinId::NttBcastHi => 2,
        BuiltinId::NttBlend => 3,
        // run_accepted(fn, arg, lo, hi): the shipped computation + the argument + the
        // inclusive bounds of the acceptance contract.
        BuiltinId::RunAccepted => 4,
        _ => 1,
    };
    if n != expected {
        let name = match id {
            BuiltinId::Length => "length",
            BuiltinId::Format => unreachable!(),
            // MapOf/SetOf return early above (variadic with their own errors).
            BuiltinId::MapOf | BuiltinId::SetOf => unreachable!(),
            BuiltinId::RepeatSeq => "repeatSeq",
            BuiltinId::ParseInt => "parseInt",
            BuiltinId::ParseFloat => "parseFloat",
            BuiltinId::Chr => "chr",
            BuiltinId::Abs => "abs",
            BuiltinId::Sqrt => "sqrt",
            BuiltinId::Min => "min",
            BuiltinId::Max => "max",
            BuiltinId::Floor => "floor",
            BuiltinId::Ceil => "ceil",
            BuiltinId::Round => "round",
            BuiltinId::Pow => "pow",
            BuiltinId::Decimal => "decimal",
            BuiltinId::Complex => "complex",
            BuiltinId::Modular => "modular",
            BuiltinId::Quantity => "quantity",
            BuiltinId::Money => "money",
            BuiltinId::SetRate => "set_rate",
            BuiltinId::SetRates => "set_rates",
            BuiltinId::ToCurrency => "to_currency",
            BuiltinId::Uuid => "uuid",
            BuiltinId::UuidNil => "uuid_nil",
            BuiltinId::UuidMax => "uuid_max",
            BuiltinId::UuidVersion => "uuid_version",
            BuiltinId::UuidDns => "uuid_dns",
            BuiltinId::UuidUrl => "uuid_url",
            BuiltinId::UuidOid => "uuid_oid",
            BuiltinId::UuidX500 => "uuid_x500",
            BuiltinId::TextBytes => "text_bytes",
            BuiltinId::TextFromBytes => "text_from_bytes",
            BuiltinId::WireBytes => "wireBytes",
            BuiltinId::ReadWireProgram => "readWireProgram",
            BuiltinId::WriteWireResidual => "writeWireResidual",
            BuiltinId::UuidBytes => "uuid_bytes",
            BuiltinId::UuidFromBytes => "uuid_from_bytes",
            BuiltinId::Convert => "convert",
            BuiltinId::ParseTimestamp => "parse_timestamp",
            BuiltinId::FormatTimestamp => "format_timestamp",
            BuiltinId::YearOf => "year_of",
            BuiltinId::MonthOf => "month_of",
            BuiltinId::DayOf => "day_of",
            BuiltinId::WeekdayOf => "weekday_of",
            BuiltinId::HourOf => "hour_of",
            BuiltinId::MinuteOf => "minute_of",
            BuiltinId::SecondOf => "second_of",
            BuiltinId::WeekOf => "week_of",
            BuiltinId::QuarterOf => "quarter_of",
            BuiltinId::DateOf => "date_of",
            BuiltinId::TimeOf => "time_of",
            BuiltinId::LocalInstant => "local_instant",
            BuiltinId::SecondsBetween => "seconds_between",
            BuiltinId::MonthsBetween => "months_between",
            BuiltinId::YearsBetween => "years_between",
            BuiltinId::AddSeconds => "add_seconds",
            BuiltinId::InZone => "in_zone",
            BuiltinId::Copy => "copy",
            BuiltinId::CountOnes => "count_ones",
            BuiltinId::RunAccepted => "run_accepted",
            BuiltinId::Word32 => "word32",
            BuiltinId::Word64 => "word64",
            BuiltinId::Lanes8Word32 => "lanes8Word32",
            BuiltinId::SeqOfLanes8 => "seqOfLanes8",
            BuiltinId::Splat8Word32 => "splat8Word32",
            BuiltinId::IntOfWord32 => "intOfWord32",
            BuiltinId::IntOfWord64 => "intOfWord64",
            BuiltinId::Word64Shl => "word64Shl",
            BuiltinId::Word64Shr => "word64Shr",
            BuiltinId::Word32Shr => "word32Shr",
            BuiltinId::Word64And => "word64And",
            BuiltinId::Word16Make => "word16",
            BuiltinId::IntOfWord16 => "intOfWord16",
            BuiltinId::Lanes4Word64 => "lanes4Word64",
            BuiltinId::SeqOfLanes4 => "seqOfLanes4",
            BuiltinId::Mul32x32To64 => "mul32x32to64",
            BuiltinId::HsumLanes4 => "hsumLanes4",
            BuiltinId::Splat4Word64 => "splat4Word64",
            BuiltinId::AndNot4 => "andNot4",
            BuiltinId::Lanes16Word16 => "lanes16Word16",
            BuiltinId::SeqOfLanes16 => "seqOfLanes16",
            BuiltinId::Splat16Word16 => "splat16Word16",
            BuiltinId::Mulhi16 => "mulhi16",
            BuiltinId::Montmul32 => "montmul32",
            BuiltinId::NttBcastLo => "nttBcastLo",
            BuiltinId::NttBcastHi => "nttBcastHi",
            BuiltinId::NttBlend => "nttBlend",
            BuiltinId::Rotl => "rotl",
            BuiltinId::Wand => "word_and",
            BuiltinId::Wor => "word_or",
            BuiltinId::Wnot => "word_not",
            BuiltinId::Lanes4Word32Make => "lanes4Word32",
            BuiltinId::Lanes4Of => "lanes4Of",
            BuiltinId::SeqOfLanes4W32 => "seqOfLanes4W32",
            BuiltinId::Sha1Rnds4 => "sha1rnds4",
            BuiltinId::Sha1Msg1 => "sha1msg1",
            BuiltinId::Sha1Msg2 => "sha1msg2",
            BuiltinId::Sha1Nexte => "sha1nexte",
            BuiltinId::Lanes16Word8Make => "lanes16Word8",
            BuiltinId::SeqOfLanes16W8 => "seqOfLanes16W8",
            BuiltinId::Splat16Word8 => "splat16Word8",
            BuiltinId::Shuffle16 => "shuffle16",
            BuiltinId::ShrBytes16 => "shrBytes16",
            BuiltinId::InterleaveLo16 => "interleaveLo16",
            BuiltinId::InterleaveHi16 => "interleaveHi16",
            BuiltinId::ByteAdd16 => "byteAdd16",
            BuiltinId::Maddubs16 => "maddubs16",
            BuiltinId::Packus16 => "packus16",
            BuiltinId::Rotr => "rotr",
        };
        return Err(format!(
            "{}() takes exactly {} argument{}",
            name,
            expected,
            if expected == 1 { "" } else { "s" }
        ));
    }
    Ok(())
}

/// Apply a builtin to already-evaluated arguments. The caller has already
/// validated arity with [`check_arity`].
/// Validate a within-vector NTT stride argument: an `Int` that is one of the supported half-widths
/// 8/4/2 (the AVX2 shuffle path is defined only at these granularities).
fn ntt_stride(v: RuntimeValue, name: &str) -> Result<usize, String> {
    match v {
        // i16 within-vector strides are 8/4/2; the i32 (Lanes8Word32) ones are 4/2/1.
        RuntimeValue::Int(n @ (8 | 4 | 2 | 1)) => Ok(n as usize),
        RuntimeValue::Int(n) => Err(format!("{name} stride must be 8, 4, 2, or 1, got {n}")),
        other => Err(format!("{name} stride must be an Int, got {}", other.type_name())),
    }
}

pub fn call_builtin(id: BuiltinId, args: Vec<RuntimeValue>) -> Result<RuntimeValue, String> {
    let mut args = args;
    match id {
        BuiltinId::MapOf => {
            // Flat key/value pairs, insertion-ordered; a duplicate key keeps
            // its first position with the last value (IndexMap `insert`).
            let mut m = crate::interpreter::MapStorage::default();
            let mut it = args.into_iter();
            while let (Some(k), Some(v)) = (it.next(), it.next()) {
                crate::semantics::collections::assert_hashable_key(&k)?;
                m.insert(k, v);
            }
            Ok(RuntimeValue::Map(Rc::new(RefCell::new(m))))
        }
        BuiltinId::SetOf => {
            // Elements in insertion order, deduplicated by value equality —
            // the same semantics as consecutive `Add x to s` statements.
            let set = RuntimeValue::Set(Rc::new(RefCell::new(Vec::new())));
            for v in args {
                crate::semantics::collections::set_add(&set, v)?;
            }
            Ok(set)
        }
        BuiltinId::RepeatSeq => {
            // The element evaluates ONCE; each slot deep-copies it, so a
            // repeated inner collection is n independent rows.
            let count = args.remove(1);
            let element = args.remove(0);
            let n = match count {
                RuntimeValue::Int(n) => n.max(0) as usize,
                other => return Err(format!("repeatSeq count must be an Int, got {}", other.type_name())),
            };
            let slots: Vec<RuntimeValue> = (0..n).map(|_| element.deep_clone()).collect();
            Ok(RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(slots)))))
        }
        BuiltinId::Length => {
            let val = args.remove(0);
            match &val {
                RuntimeValue::List(items) => Ok(RuntimeValue::Int(items.borrow().len() as i64)),
                RuntimeValue::Text(s) => Ok(RuntimeValue::Int(s.len() as i64)),
                RuntimeValue::Map(map) => Ok(RuntimeValue::Int(map.borrow().len() as i64)),
                _ => Err(format!("Cannot get length of {}", val.type_name())),
            }
        }
        BuiltinId::Format => {
            if args.is_empty() {
                return Ok(RuntimeValue::Text(Rc::new(String::new())));
            }
            let val = args.remove(0);
            Ok(RuntimeValue::Text(Rc::new(val.to_display_string())))
        }
        BuiltinId::ParseInt => {
            let val = args.remove(0);
            if let RuntimeValue::Text(s) = &val {
                Ok(RuntimeValue::Int(
                    s.trim()
                        .parse::<i64>()
                        .map_err(|_| format!("Cannot parse '{}' as Int", s))?,
                ))
            } else {
                Err("parseInt requires a Text argument".to_string())
            }
        }
        BuiltinId::ParseFloat => {
            let val = args.remove(0);
            if let RuntimeValue::Text(s) = &val {
                Ok(RuntimeValue::Float(
                    s.trim()
                        .parse::<f64>()
                        .map_err(|_| format!("Cannot parse '{}' as Float", s))?,
                ))
            } else {
                Err("parseFloat requires a Text argument".to_string())
            }
        }
        BuiltinId::Chr => {
            let val = args.remove(0);
            if let RuntimeValue::Int(code) = val {
                match char::from_u32(code as u32) {
                    Some(c) => Ok(RuntimeValue::Text(Rc::new(c.to_string()))),
                    None => Err(format!("Invalid character code: {}", code)),
                }
            } else {
                Err("chr() requires an Int argument".to_string())
            }
        }
        BuiltinId::Abs => {
            let val = args.remove(0);
            match val {
                // |i64::MIN| = 2^63 does not fit i64, so abs is EXACT via promotion
                // (wrapping_abs would have returned i64::MIN — a sign bug).
                RuntimeValue::Int(n) => Ok(match n.checked_abs() {
                    Some(a) => RuntimeValue::Int(a),
                    None => RuntimeValue::from_bigint(logicaffeine_base::BigInt::from_i64(n).abs()),
                }),
                RuntimeValue::BigInt(b) => Ok(RuntimeValue::from_bigint(b.abs())),
                // |·| of a rational stays a rational (`|-7/2| = 7/2`), downsized if whole.
                RuntimeValue::Rational(r) => Ok(RuntimeValue::from_rational(r.abs())),
                // |·| of a decimal stays a decimal, scale preserved (`|-0.05| = 0.05`).
                RuntimeValue::Decimal(d) => Ok(RuntimeValue::Decimal(Rc::new(d.abs()))),
                // |z| of a complex is its modulus √(re²+im²) — generally irrational, a Float view.
                RuntimeValue::Complex(c) => Ok(RuntimeValue::Float(c.abs_f64())),
                RuntimeValue::Float(f) => Ok(RuntimeValue::Float(f.abs())),
                _ => Err(format!("abs() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Sqrt => {
            let val = args.remove(0);
            match val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Float(f.sqrt())),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Float((n as f64).sqrt())),
                RuntimeValue::BigInt(b) => Ok(RuntimeValue::Float(b.to_f64().sqrt())),
                _ => Err(format!("sqrt() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Min => {
            let b = args.remove(1);
            let a = args.remove(0);
            match (&a, &b) {
                (RuntimeValue::Int(x), RuntimeValue::Int(y)) => Ok(RuntimeValue::Int(*x.min(y))),
                (RuntimeValue::Float(x), RuntimeValue::Float(y)) => {
                    Ok(RuntimeValue::Float(x.min(*y)))
                }
                (RuntimeValue::Int(x), RuntimeValue::Float(y)) => {
                    Ok(RuntimeValue::Float((*x as f64).min(*y)))
                }
                (RuntimeValue::Float(x), RuntimeValue::Int(y)) => {
                    Ok(RuntimeValue::Float(x.min(*y as f64)))
                }
                // Exact decimal min (value-based, scale of the chosen operand preserved).
                (RuntimeValue::Decimal(x), RuntimeValue::Decimal(y)) => {
                    Ok(RuntimeValue::Decimal(if x <= y { x.clone() } else { y.clone() }))
                }
                _ => Err("min() requires numbers".to_string()),
            }
        }
        BuiltinId::Max => {
            let b = args.remove(1);
            let a = args.remove(0);
            match (&a, &b) {
                (RuntimeValue::Int(x), RuntimeValue::Int(y)) => Ok(RuntimeValue::Int(*x.max(y))),
                (RuntimeValue::Float(x), RuntimeValue::Float(y)) => {
                    Ok(RuntimeValue::Float(x.max(*y)))
                }
                (RuntimeValue::Int(x), RuntimeValue::Float(y)) => {
                    Ok(RuntimeValue::Float((*x as f64).max(*y)))
                }
                (RuntimeValue::Float(x), RuntimeValue::Int(y)) => {
                    Ok(RuntimeValue::Float(x.max(*y as f64)))
                }
                // Exact decimal max (value-based, scale of the chosen operand preserved).
                (RuntimeValue::Decimal(x), RuntimeValue::Decimal(y)) => {
                    Ok(RuntimeValue::Decimal(if x >= y { x.clone() } else { y.clone() }))
                }
                _ => Err("max() requires numbers".to_string()),
            }
        }
        BuiltinId::Floor => {
            let val = args.remove(0);
            // An exact integer (Int/BigInt) is already whole; a Rational rounds toward
            // −∞ to an exact integer — the explicit floor of `floor(7 / 2) → 3`.
            match &val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Int(f.floor() as i64)),
                RuntimeValue::Int(_) | RuntimeValue::BigInt(_) => Ok(val.clone()),
                RuntimeValue::Rational(r) => Ok(RuntimeValue::from_bigint(r.floor())),
                RuntimeValue::Decimal(d) => Ok(RuntimeValue::from_bigint(d.to_rational().floor())),
                _ => Err(format!("floor() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Ceil => {
            let val = args.remove(0);
            match &val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Int(f.ceil() as i64)),
                RuntimeValue::Int(_) | RuntimeValue::BigInt(_) => Ok(val.clone()),
                RuntimeValue::Rational(r) => Ok(RuntimeValue::from_bigint(r.ceil())),
                RuntimeValue::Decimal(d) => Ok(RuntimeValue::from_bigint(d.to_rational().ceil())),
                _ => Err(format!("ceil() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Round => {
            let val = args.remove(0);
            match &val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Int(f.round() as i64)),
                RuntimeValue::Int(_) | RuntimeValue::BigInt(_) => Ok(val.clone()),
                RuntimeValue::Rational(r) => Ok(RuntimeValue::from_bigint(r.round())),
                RuntimeValue::Decimal(d) => Ok(RuntimeValue::from_bigint(d.to_rational().round())),
                _ => Err(format!("round() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Pow => {
            let exp = args.remove(1);
            let base = args.remove(0);
            match (&base, &exp) {
                // EXACT integer power: on i64 overflow, promote to BigInt rather than
                // wrapping (e.g. 2^63 is the value, not i64::MIN). A negative exponent
                // is a fractional power, so it falls to f64 as before.
                (RuntimeValue::Int(b), RuntimeValue::Int(e)) => {
                    if *e >= 0 {
                        Ok(match b.checked_pow(*e as u32) {
                            Some(p) => RuntimeValue::Int(p),
                            None => RuntimeValue::from_bigint(logicaffeine_base::BigInt::from_i64(*b).pow(*e as u32)),
                        })
                    } else {
                        Ok(RuntimeValue::Float((*b as f64).powi(*e as i32)))
                    }
                }
                (RuntimeValue::BigInt(b), RuntimeValue::Int(e)) => {
                    if *e >= 0 {
                        Ok(RuntimeValue::from_bigint(b.pow(*e as u32)))
                    } else {
                        Ok(RuntimeValue::Float(b.to_f64().powi(*e as i32)))
                    }
                }
                (RuntimeValue::Float(b), RuntimeValue::Int(e)) => {
                    Ok(RuntimeValue::Float(b.powi(*e as i32)))
                }
                (RuntimeValue::Float(b), RuntimeValue::Float(e)) => {
                    Ok(RuntimeValue::Float(b.powf(*e)))
                }
                (RuntimeValue::Int(b), RuntimeValue::Float(e)) => {
                    Ok(RuntimeValue::Float((*b as f64).powf(*e)))
                }
                // Modular exponentiation: x^e in ℤ/nℤ (fast square-and-multiply). Exponent ≥ 0.
                (RuntimeValue::Modular(b), RuntimeValue::Int(e)) if *e >= 0 => {
                    Ok(RuntimeValue::Modular(Rc::new(b.pow(*e as u64))))
                }
                _ => Err("pow() requires numbers".to_string()),
            }
        }
        BuiltinId::Decimal => {
            let val = args.remove(0);
            match &val {
                // Parse the exact base-10 value from its literal text (`decimal("19.99")`).
                RuntimeValue::Text(s) => Decimal::parse(s.trim())
                    .map(|d| RuntimeValue::Decimal(Rc::new(d)))
                    .ok_or_else(|| format!("Cannot parse '{}' as Decimal", s)),
                // An integer is already exact — widen it to a scale-0 Decimal.
                RuntimeValue::Int(n) => Ok(RuntimeValue::Decimal(Rc::new(Decimal::from_i64(*n)))),
                // Already a Decimal — identity.
                RuntimeValue::Decimal(_) => Ok(val.clone()),
                _ => Err(format!("decimal() requires a Text or Int, got {}", val.type_name())),
            }
        }
        BuiltinId::Complex => {
            let im = args.remove(1);
            let re = args.remove(0);
            // Each part must be an EXACT real (Int/BigInt/Rational/Decimal); a Float would
            // make the complex inexact, so it is refused rather than silently coerced.
            let to_rat = |v: &RuntimeValue| -> Option<logicaffeine_base::Rational> {
                match v {
                    RuntimeValue::Int(n) => Some(logicaffeine_base::Rational::from_i64(*n)),
                    RuntimeValue::BigInt(b) => Some(logicaffeine_base::Rational::from_bigint((**b).clone())),
                    RuntimeValue::Rational(r) => Some((**r).clone()),
                    RuntimeValue::Decimal(d) => Some(d.to_rational()),
                    _ => None,
                }
            };
            match (to_rat(&re), to_rat(&im)) {
                (Some(re_r), Some(im_r)) => Ok(RuntimeValue::Complex(Rc::new(
                    logicaffeine_base::Complex::new(re_r, im_r),
                ))),
                _ => Err(format!(
                    "complex() requires two exact numbers, got {} and {}",
                    re.type_name(),
                    im.type_name()
                )),
            }
        }
        BuiltinId::Modular => {
            let modulus = args.remove(1);
            let value = args.remove(0);
            let to_int = |v: &RuntimeValue| -> Option<logicaffeine_base::BigInt> {
                match v {
                    RuntimeValue::Int(n) => Some(logicaffeine_base::BigInt::from_i64(*n)),
                    RuntimeValue::BigInt(b) => Some((**b).clone()),
                    _ => None,
                }
            };
            match (to_int(&value), to_int(&modulus)) {
                (Some(v), Some(n)) => match logicaffeine_base::Modular::new(v, n) {
                    Some(m) => Ok(RuntimeValue::Modular(Rc::new(m))),
                    None => Err("modular() requires a positive modulus".to_string()),
                },
                _ => Err(format!(
                    "modular() requires two integers, got {} and {}",
                    value.type_name(),
                    modulus.type_name()
                )),
            }
        }
        BuiltinId::Quantity => {
            let unit_arg = args.remove(1);
            let value = args.remove(0);
            // The magnitude must be an EXACT number (Int/BigInt/Rational/Decimal) — a Float would
            // make the quantity inexact, defeating lossless conversion, so it is refused.
            let magnitude = match &value {
                RuntimeValue::Int(n) => logicaffeine_base::Rational::from_i64(*n),
                RuntimeValue::BigInt(b) => logicaffeine_base::Rational::from_bigint((**b).clone()),
                RuntimeValue::Rational(r) => (**r).clone(),
                RuntimeValue::Decimal(d) => d.to_rational(),
                _ => return Err(format!("quantity() requires an exact number, got {}", value.type_name())),
            };
            let unit = match &unit_arg {
                RuntimeValue::Text(s) => logicaffeine_base::quantity::units::by_name(s)
                    .ok_or_else(|| format!("Unknown unit '{}'", s))?,
                _ => return Err(format!("quantity() requires a unit name (Text), got {}", unit_arg.type_name())),
            };
            let q = logicaffeine_base::Quantity::of(magnitude, &unit);
            Ok(RuntimeValue::Quantity(Rc::new(crate::interpreter::QuantityValue { q, unit })))
        }
        BuiltinId::Money => {
            let code_arg = args.remove(1);
            let value = args.remove(0);
            // The amount must be EXACT base-10 (Int or Decimal); a Float or non-terminating Rational
            // would not be representable money, so it is refused.
            let amount = match &value {
                RuntimeValue::Int(n) => logicaffeine_base::Decimal::from_i64(*n),
                RuntimeValue::Decimal(d) => (**d).clone(),
                _ => return Err(format!("money() requires an exact base-10 amount (Int or Decimal), got {}", value.type_name())),
            };
            let currency = match &code_arg {
                RuntimeValue::Text(s) => logicaffeine_base::money::currency::by_code(s)
                    .ok_or_else(|| format!("Unknown currency '{}'", s))?,
                _ => return Err(format!("money() requires a currency code (Text), got {}", code_arg.type_name())),
            };
            Ok(RuntimeValue::Money(Rc::new(logicaffeine_base::Money::of(amount, currency))))
        }
        BuiltinId::SetRate => {
            let rate_arg = args.remove(1);
            let code_arg = args.remove(0);
            let code = match &code_arg {
                RuntimeValue::Text(s) => s.to_string(),
                _ => return Err(format!("set_rate() requires a currency code (Text), got {}", code_arg.type_name())),
            };
            let rate = match &rate_arg {
                RuntimeValue::Int(n) => logicaffeine_base::Rational::from_i64(*n),
                RuntimeValue::Decimal(d) => d.to_rational(),
                RuntimeValue::Rational(r) => (**r).clone(),
                _ => return Err(format!("set_rate() requires an exact rate (Int/Decimal), got {}", rate_arg.type_name())),
            };
            logicaffeine_base::money::set_ambient_rate(&code, rate);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::SetRates => {
            let table = args.remove(0);
            let map = match &table {
                RuntimeValue::Map(m) => m,
                _ => {
                    return Err(format!(
                        "set_rates() requires a Map of currency code to rate, got {}",
                        table.type_name()
                    ))
                }
            };
            for (key, value) in map.borrow().iter() {
                let code = match key {
                    RuntimeValue::Text(s) => s.to_string(),
                    _ => return Err(format!("set_rates() keys must be currency codes (Text), got {}", key.type_name())),
                };
                let rate = match value {
                    RuntimeValue::Int(n) => logicaffeine_base::Rational::from_i64(*n),
                    RuntimeValue::Decimal(d) => d.to_rational(),
                    RuntimeValue::Rational(r) => (**r).clone(),
                    _ => return Err(format!("set_rates() values must be exact rates (Int/Decimal), got {}", value.type_name())),
                };
                logicaffeine_base::money::set_ambient_rate(&code, rate);
            }
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::Uuid => {
            let arg = args.remove(0);
            match &arg {
                RuntimeValue::Text(s) => logicaffeine_base::Uuid::parse(s)
                    .map(|u| RuntimeValue::Uuid(Rc::new(u)))
                    .ok_or_else(|| format!("invalid UUID '{}'", s)),
                _ => Err(format!("uuid() requires text, got {}", arg.type_name())),
            }
        }
        BuiltinId::UuidNil => Ok(RuntimeValue::Uuid(Rc::new(logicaffeine_base::Uuid::NIL))),
        BuiltinId::UuidMax => Ok(RuntimeValue::Uuid(Rc::new(logicaffeine_base::Uuid::MAX))),
        BuiltinId::UuidDns => Ok(RuntimeValue::Uuid(Rc::new(logicaffeine_base::Uuid::NAMESPACE_DNS))),
        BuiltinId::UuidUrl => Ok(RuntimeValue::Uuid(Rc::new(logicaffeine_base::Uuid::NAMESPACE_URL))),
        BuiltinId::UuidOid => Ok(RuntimeValue::Uuid(Rc::new(logicaffeine_base::Uuid::NAMESPACE_OID))),
        BuiltinId::UuidX500 => Ok(RuntimeValue::Uuid(Rc::new(logicaffeine_base::Uuid::NAMESPACE_X500))),
        BuiltinId::UuidVersion => {
            let arg = args.remove(0);
            match &arg {
                RuntimeValue::Uuid(u) => Ok(RuntimeValue::Int(u.version() as i64)),
                _ => Err(format!("uuid_version() requires a Uuid, got {}", arg.type_name())),
            }
        }
        BuiltinId::TextBytes => {
            let arg = args.remove(0);
            match &arg {
                RuntimeValue::Text(s) => Ok(bytes_to_seq(s.as_bytes())),
                _ => Err(format!("text_bytes() requires text, got {}", arg.type_name())),
            }
        }
        BuiltinId::TextFromBytes => {
            let bytes = byte_seq(&args.remove(0))?;
            match String::from_utf8(bytes) {
                Ok(s) => Ok(RuntimeValue::Text(Rc::new(s))),
                Err(e) => Err(format!("text_from_bytes(): invalid UTF-8: {}", e)),
            }
        }
        BuiltinId::WireBytes => {
            let arg = args.remove(0);
            match crate::concurrency::marshal::encode_value_raw(&arg) {
                Ok(bytes) => Ok(bytes_to_seq(&bytes)),
                Err(e) => Err(format!("wireBytes(): {}", e)),
            }
        }
        BuiltinId::ReadWireProgram => {
            use std::io::Read;
            let mut len = [0u8; 4];
            if std::io::stdin().read_exact(&mut len).is_err() {
                std::process::exit(0);
            }
            let n = u32::from_le_bytes(len) as usize;
            let mut buf = vec![0u8; n];
            std::io::stdin()
                .read_exact(&mut buf)
                .map_err(|e| format!("readWireProgram(): frame read failed: {e}"))?;
            crate::concurrency::marshal::decode_value_raw(&buf)
                .ok_or_else(|| "readWireProgram(): malformed wire program".to_string())
        }
        BuiltinId::WriteWireResidual => {
            use std::io::Write;
            let arg = args.remove(0);
            let s = match &arg {
                RuntimeValue::Text(t) => (**t).clone(),
                _ => return Err("writeWireResidual() expects text".to_string()),
            };
            let b = s.as_bytes();
            let out = std::io::stdout();
            let mut h = out.lock();
            h.write_all(&(b.len() as u32).to_le_bytes())
                .and_then(|_| h.write_all(b))
                .and_then(|_| h.flush())
                .map_err(|e| format!("writeWireResidual(): {e}"))?;
            Ok(RuntimeValue::Int(b.len() as i64))
        }
        BuiltinId::UuidBytes => {
            let arg = args.remove(0);
            match &arg {
                RuntimeValue::Uuid(u) => Ok(bytes_to_seq(u.as_bytes())),
                _ => Err(format!("uuid_bytes() requires a Uuid, got {}", arg.type_name())),
            }
        }
        BuiltinId::UuidFromBytes => {
            let bytes = byte_seq(&args.remove(0))?;
            if bytes.len() < 16 {
                return Err(format!("uuid_from_bytes() needs 16 bytes, got {}", bytes.len()));
            }
            let mut b = [0u8; 16];
            b.copy_from_slice(&bytes[..16]);
            Ok(RuntimeValue::Uuid(Rc::new(logicaffeine_base::Uuid::from_bytes(b))))
        }
        BuiltinId::ToCurrency => {
            let code_arg = args.remove(1);
            let money = args.remove(0);
            match (&money, &code_arg) {
                (RuntimeValue::Money(m), RuntimeValue::Text(code)) => {
                    let to = logicaffeine_base::money::currency::by_code(code)
                        .ok_or_else(|| format!("Unknown currency '{}'", code))?;
                    logicaffeine_base::money::ambient_convert(m, to)
                        .map(|c| RuntimeValue::Money(Rc::new(c)))
                        .ok_or_else(|| {
                            if logicaffeine_base::money::has_ambient_rates() {
                                format!("no exchange rate for {} or {}", m.currency.code, to.code)
                            } else {
                                "no exchange rates in scope (set a rate first)".to_string()
                            }
                        })
                }
                _ => Err(format!(
                    "to_currency() requires money and a currency code, got {} and {}",
                    money.type_name(),
                    code_arg.type_name()
                )),
            }
        }
        BuiltinId::Convert => {
            let unit_arg = args.remove(1);
            let value = args.remove(0);
            let unit = match &unit_arg {
                RuntimeValue::Text(s) => logicaffeine_base::quantity::units::by_name(s)
                    .ok_or_else(|| format!("Unknown unit '{}'", s))?,
                _ => return Err(format!("convert() requires a unit name (Text), got {}", unit_arg.type_name())),
            };
            match &value {
                // Same dimension → re-express (the SI magnitude is unchanged, only the display unit);
                // a different dimension is the forbidden cast.
                RuntimeValue::Quantity(qv) => {
                    if qv.q.dimension() != unit.dimension {
                        return Err(format!(
                            "cannot convert a {} quantity to '{}' — different dimension",
                            qv.unit.symbol, unit.symbol
                        ));
                    }
                    Ok(RuntimeValue::Quantity(Rc::new(crate::interpreter::QuantityValue {
                        q: qv.q.clone(),
                        unit,
                    })))
                }
                _ => Err(format!("convert() requires a quantity, got {}", value.type_name())),
            }
        }
        BuiltinId::ParseTimestamp => {
            let val = args.remove(0);
            match &val {
                RuntimeValue::Text(s) => logicaffeine_base::temporal::parse_rfc3339(s.trim())
                    .map(RuntimeValue::Moment)
                    .ok_or_else(|| format!("Cannot parse '{}' as an RFC 3339 timestamp", s)),
                _ => Err(format!("parse_timestamp() requires a Text, got {}", val.type_name())),
            }
        }
        BuiltinId::FormatTimestamp => {
            let val = args.remove(0);
            match &val {
                RuntimeValue::Moment(nanos) => {
                    Ok(RuntimeValue::Text(Rc::new(logicaffeine_base::temporal::format_rfc3339(*nanos))))
                }
                _ => Err(format!("format_timestamp() requires a Moment, got {}", val.type_name())),
            }
        }
        BuiltinId::YearOf
        | BuiltinId::MonthOf
        | BuiltinId::DayOf
        | BuiltinId::WeekdayOf
        | BuiltinId::WeekOf
        | BuiltinId::QuarterOf
        | BuiltinId::HourOf
        | BuiltinId::MinuteOf
        | BuiltinId::SecondOf => {
            let val = args.remove(0);
            match &val {
                RuntimeValue::Moment(nanos) => {
                    use logicaffeine_base::temporal;
                    let civil = temporal::civil_from_unix_nanos(*nanos);
                    let component = match id {
                        BuiltinId::YearOf => civil.year,
                        BuiltinId::MonthOf => civil.month as i64,
                        BuiltinId::DayOf => civil.day as i64,
                        BuiltinId::HourOf => civil.hour as i64,
                        BuiltinId::MinuteOf => civil.minute as i64,
                        BuiltinId::SecondOf => civil.second as i64,
                        BuiltinId::WeekdayOf => {
                            temporal::weekday_from_days(nanos.div_euclid(temporal::NANOS_PER_DAY)) as i64
                        }
                        BuiltinId::WeekOf => {
                            temporal::iso_week_from_days(nanos.div_euclid(temporal::NANOS_PER_DAY)).1 as i64
                        }
                        BuiltinId::QuarterOf => (civil.month as i64 - 1) / 3 + 1,
                        _ => unreachable!(),
                    };
                    Ok(RuntimeValue::Int(component))
                }
                // The same accessors on a calendar Date (no time-of-day, so hour/minute/second
                // don't apply) — `the year of 2024-03-10`.
                RuntimeValue::Date(days) => {
                    use logicaffeine_base::temporal;
                    let (y, m, d) = temporal::civil_from_days(*days as i64);
                    let component = match id {
                        BuiltinId::YearOf => y,
                        BuiltinId::MonthOf => m as i64,
                        BuiltinId::DayOf => d as i64,
                        BuiltinId::WeekdayOf => temporal::weekday_from_days(*days as i64) as i64,
                        BuiltinId::WeekOf => temporal::iso_week_from_days(*days as i64).1 as i64,
                        BuiltinId::QuarterOf => (m as i64 - 1) / 3 + 1,
                        BuiltinId::HourOf | BuiltinId::MinuteOf | BuiltinId::SecondOf => {
                            return Err("a Date has no time-of-day — use a timestamp/Moment".to_string());
                        }
                        _ => unreachable!(),
                    };
                    Ok(RuntimeValue::Int(component))
                }
                _ => Err(format!("a date component extractor requires a Moment or Date, got {}", val.type_name())),
            }
        }
        BuiltinId::DateOf => {
            let val = args.remove(0);
            match &val {
                // The calendar day a Moment falls on (UTC); a Date is already a date (identity).
                RuntimeValue::Moment(nanos) => Ok(RuntimeValue::Date(
                    nanos.div_euclid(logicaffeine_base::temporal::NANOS_PER_DAY) as i32,
                )),
                RuntimeValue::Date(days) => Ok(RuntimeValue::Date(*days)),
                _ => Err(format!("date_of() requires a Moment or Date, got {}", val.type_name())),
            }
        }
        BuiltinId::TimeOf => {
            let val = args.remove(0);
            match &val {
                // The wall-clock time-of-day a Moment falls on (UTC). A Date has no time-of-day.
                RuntimeValue::Moment(nanos) => Ok(RuntimeValue::Time(
                    nanos.rem_euclid(logicaffeine_base::temporal::NANOS_PER_DAY),
                )),
                RuntimeValue::Date(_) => {
                    Err("a Date has no time-of-day — use a timestamp/Moment".to_string())
                }
                _ => Err(format!("time_of() requires a Moment, got {}", val.type_name())),
            }
        }
        BuiltinId::SecondsBetween => {
            let b = args.remove(1);
            let a = args.remove(0);
            match (&a, &b) {
                (RuntimeValue::Moment(a), RuntimeValue::Moment(b)) => {
                    Ok(RuntimeValue::Int((b - a) / 1_000_000_000))
                }
                _ => Err(format!(
                    "seconds_between() requires two Moments, got {} and {}",
                    a.type_name(),
                    b.type_name()
                )),
            }
        }
        BuiltinId::MonthsBetween | BuiltinId::YearsBetween => {
            let b = args.remove(1);
            let a = args.remove(0);
            match (&a, &b) {
                (RuntimeValue::Moment(a), RuntimeValue::Moment(b)) => {
                    let n = if matches!(id, BuiltinId::MonthsBetween) {
                        logicaffeine_base::temporal::months_between(*a, *b)
                    } else {
                        logicaffeine_base::temporal::years_between(*a, *b)
                    };
                    Ok(RuntimeValue::Int(n))
                }
                _ => {
                    let name = if matches!(id, BuiltinId::MonthsBetween) {
                        "months_between"
                    } else {
                        "years_between"
                    };
                    Err(format!(
                        "{name}() requires two Moments, got {} and {}",
                        a.type_name(),
                        b.type_name()
                    ))
                }
            }
        }
        BuiltinId::AddSeconds => {
            let secs = args.remove(1);
            let moment = args.remove(0);
            match (&moment, &secs) {
                (RuntimeValue::Moment(nanos), RuntimeValue::Int(n)) => {
                    Ok(RuntimeValue::Moment(nanos + n * 1_000_000_000))
                }
                _ => Err(format!(
                    "add_seconds() requires a Moment and an Int, got {} and {}",
                    moment.type_name(),
                    secs.type_name()
                )),
            }
        }
        BuiltinId::InZone => {
            let zone = args.remove(1);
            let moment = args.remove(0);
            match (&moment, &zone) {
                (RuntimeValue::Moment(nanos), RuntimeValue::Text(z)) => {
                    logicaffeine_base::temporal::format_zoned(*nanos, z)
                        .map(|s| RuntimeValue::Text(Rc::new(s)))
                        .ok_or_else(|| format!("Unknown time zone '{}'", z))
                }
                _ => Err(format!(
                    "in_zone() requires a Moment and a zone name (Text), got {} and {}",
                    moment.type_name(),
                    zone.type_name()
                )),
            }
        }
        BuiltinId::LocalInstant => {
            let zone = args.remove(1);
            let moment = args.remove(0);
            match (&moment, &zone) {
                (RuntimeValue::Moment(nanos), RuntimeValue::Text(z)) => {
                    logicaffeine_base::temporal::local_instant_nanos(*nanos, z)
                        .map(RuntimeValue::Moment)
                        .ok_or_else(|| format!("Unknown time zone '{}'", z))
                }
                _ => Err(format!(
                    "local_instant() requires a Moment and a zone name (Text), got {} and {}",
                    moment.type_name(),
                    zone.type_name()
                )),
            }
        }
        BuiltinId::Copy => {
            let val = args.remove(0);
            Ok(val.deep_clone())
        }
        BuiltinId::CountOnes => {
            let val = args.remove(0);
            match val {
                // Two's-complement faithful: count set bits over the full 64-bit
                // pattern (matches codegen's `(x as u64).count_ones()`).
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int((n as u64).count_ones() as i64)),
                _ => Err(format!(
                    "count_ones() requires an Int, got {}",
                    val.type_name()
                )),
            }
        }
        BuiltinId::Word32 => {
            let n = args.remove(0);
            match n {
                RuntimeValue::Int(v) => Ok(RuntimeValue::Word(WordVal::W32(Word32(v as u32)))),
                RuntimeValue::Word(w) => Ok(RuntimeValue::Word(WordVal::W32(Word32(w.to_u64() as u32)))),
                _ => Err(format!("word32() requires an Int, got {}", n.type_name())),
            }
        }
        BuiltinId::Word64 => {
            let n = args.remove(0);
            match n {
                RuntimeValue::Int(v) => Ok(RuntimeValue::Word(WordVal::W64(Word64(v as u64)))),
                RuntimeValue::Word(w) => Ok(RuntimeValue::Word(WordVal::W64(Word64(w.to_u64())))),
                _ => Err(format!("word64() requires an Int, got {}", n.type_name())),
            }
        }
        BuiltinId::Rotl | BuiltinId::Rotr => {
            let w = args.remove(0);
            let n = args.remove(0);
            let count = match n {
                RuntimeValue::Int(c) => c as u32,
                RuntimeValue::Word(c) => c.to_u64() as u32,
                _ => return Err(format!("rotate count must be an Int, got {}", n.type_name())),
            };
            match w {
                RuntimeValue::Word(word) => {
                    let r = if matches!(id, BuiltinId::Rotl) { word.rotl(count) } else { word.rotr(count) };
                    Ok(RuntimeValue::Word(r))
                }
                // A SIMD lane vector rotates lane-wise (the ChaCha diffusion op). Only left rotation
                // is part of the lane vocabulary so far.
                RuntimeValue::Lanes(lanes) if matches!(id, BuiltinId::Rotl) => {
                    match lanes.rotl(count) {
                        Some(v) => Ok(RuntimeValue::Lanes(Rc::new(v))),
                        None => Err(format!("rotl is not defined for {}", lanes.type_name())),
                    }
                }
                _ => Err(format!("rotate requires a Word, got {}", w.type_name())),
            }
        }
        BuiltinId::Wand | BuiltinId::Wor => {
            let b = args.remove(1);
            let a = args.remove(0);
            match (&a, &b) {
                (RuntimeValue::Word(x), RuntimeValue::Word(y)) => {
                    let r = if id == BuiltinId::Wand { x.bitand(*y) } else { x.bitor(*y) };
                    match r {
                        Some(w) => Ok(RuntimeValue::Word(w)),
                        None => Err(format!("word_and/or width mismatch: {} vs {}", a.type_name(), b.type_name())),
                    }
                }
                // Lane-vector bitwise mixing (the MD5 F/G functions written over `Lanes8Word32`).
                (RuntimeValue::Lanes(x), RuntimeValue::Lanes(y)) => {
                    let r = if id == BuiltinId::Wand { (**x).bitand(**y) } else { (**x).bitor(**y) };
                    match r {
                        Some(v) => Ok(RuntimeValue::Lanes(Rc::new(v))),
                        None => Err(format!(
                            "word_and/or lane-config mismatch: {} vs {}",
                            a.type_name(),
                            b.type_name()
                        )),
                    }
                }
                _ => Err(format!("word_and/or requires two Words, got {} and {}", a.type_name(), b.type_name())),
            }
        }
        BuiltinId::Wnot => {
            let a = args.remove(0);
            match &a {
                RuntimeValue::Word(x) => Ok(RuntimeValue::Word(x.not())),
                RuntimeValue::Lanes(x) => match (**x).lane_not() {
                    Some(v) => Ok(RuntimeValue::Lanes(Rc::new(v))),
                    None => Err(format!("word_not: lane config has no complement: {}", a.type_name())),
                },
                _ => Err(format!("word_not requires a Word, got {}", a.type_name())),
            }
        }
        BuiltinId::Lanes4Word32Make => {
            // Pack a Seq of (up to) 4 Word32 into the 128-bit SHA-1 lane register.
            let s = args.remove(0);
            match s {
                RuntimeValue::List(items) => {
                    let vals = items.borrow().to_values();
                    let mut words = Vec::with_capacity(vals.len());
                    for v in &vals {
                        match v {
                            RuntimeValue::Word(WordVal::W32(w)) => words.push(*w),
                            RuntimeValue::Int(n) => words.push(Word32(*n as u32)),
                            other => {
                                return Err(format!(
                                    "lanes4Word32() needs a Seq of Word32, found {}",
                                    other.type_name()
                                ))
                            }
                        }
                    }
                    Ok(RuntimeValue::Lanes(Rc::new(LanesVal::L4W32(
                        logicaffeine_base::Lanes4Word32::from_words(&words),
                    ))))
                }
                other => Err(format!("lanes4Word32() requires a Seq of Word32, got {}", other.type_name())),
            }
        }
        BuiltinId::Lanes4Of => {
            // Pack four Word32 values straight into the lane register — no Seq, no heap. This is the
            // per-round constructor the Logos SHA-1 uses, so it must not allocate.
            let mut w = [Word32(0); 4];
            for slot in w.iter_mut() {
                *slot = match args.remove(0) {
                    RuntimeValue::Word(WordVal::W32(x)) => x,
                    RuntimeValue::Int(n) => Word32(n as u32),
                    other => {
                        return Err(format!("lanes4Of() needs four Word32, found {}", other.type_name()))
                    }
                };
            }
            Ok(RuntimeValue::Lanes(Rc::new(LanesVal::L4W32(
                logicaffeine_base::Lanes4Word32::from_words(&w),
            ))))
        }
        BuiltinId::SeqOfLanes4W32 => {
            let v = args.remove(0);
            match v {
                RuntimeValue::Lanes(lanes) => match *lanes {
                    LanesVal::L4W32(lv) => {
                        let vals: Vec<RuntimeValue> = lv
                            .to_words()
                            .iter()
                            .map(|w| RuntimeValue::Word(WordVal::W32(*w)))
                            .collect();
                        Ok(RuntimeValue::List(Rc::new(std::cell::RefCell::new(
                            crate::interpreter::ListRepr::from_values(vals),
                        ))))
                    }
                    other => Err(format!("seqOfLanes4W32() requires a Lanes4Word32, got {}", other.type_name())),
                },
                other => Err(format!("seqOfLanes4W32() requires a Lanes4Word32, got {}", other.type_name())),
            }
        }
        BuiltinId::Sha1Rnds4 => {
            let func = args.remove(2);
            let msg = args.remove(1);
            let abcd = args.remove(0);
            let f = match &func {
                RuntimeValue::Int(n) => *n as u32,
                _ => return Err(format!("sha1rnds4() func must be an Int, got {}", func.type_name())),
            };
            match (&abcd, &msg) {
                (RuntimeValue::Lanes(a), RuntimeValue::Lanes(b)) => (**a)
                    .sha1rnds4(**b, f)
                    .map(|r| RuntimeValue::Lanes(Rc::new(r)))
                    .ok_or_else(|| "sha1rnds4() requires two Lanes4Word32".to_string()),
                _ => Err(format!(
                    "sha1rnds4() requires two Lanes4Word32, got {} and {}",
                    abcd.type_name(),
                    msg.type_name()
                )),
            }
        }
        BuiltinId::Sha1Msg1 | BuiltinId::Sha1Msg2 | BuiltinId::Sha1Nexte => {
            let b = args.remove(1);
            let a = args.remove(0);
            match (&a, &b) {
                (RuntimeValue::Lanes(x), RuntimeValue::Lanes(y)) => {
                    let r = match id {
                        BuiltinId::Sha1Msg1 => (**x).sha1msg1(**y),
                        BuiltinId::Sha1Msg2 => (**x).sha1msg2(**y),
                        _ => (**x).sha1nexte(**y),
                    };
                    r.map(|v| RuntimeValue::Lanes(Rc::new(v)))
                        .ok_or_else(|| "sha1msg/nexte requires two Lanes4Word32".to_string())
                }
                _ => Err(format!(
                    "sha1 message op requires two Lanes4Word32, got {} and {}",
                    a.type_name(),
                    b.type_name()
                )),
            }
        }
        BuiltinId::Lanes16Word8Make => {
            // Pack a Seq of Int (bytes) into one byte-shuffle register — the SIMD hex codec loads its
            // 16 working bytes this way.
            let bytes = byte_seq(&args.remove(0))?;
            Ok(RuntimeValue::Lanes(Rc::new(LanesVal::L16W8(
                logicaffeine_base::Lanes16Word8::from_bytes(&bytes),
            ))))
        }
        BuiltinId::SeqOfLanes16W8 => {
            let v = args.remove(0);
            match v {
                RuntimeValue::Lanes(lanes) => match *lanes {
                    LanesVal::L16W8(lv) => Ok(bytes_to_seq(&lv.to_bytes())),
                    other => Err(format!(
                        "seqOfLanes16W8() requires a Lanes16Word8, got {}",
                        other.type_name()
                    )),
                },
                other => Err(format!(
                    "seqOfLanes16W8() requires a Lanes16Word8, got {}",
                    other.type_name()
                )),
            }
        }
        BuiltinId::Splat16Word8 => match args.remove(0) {
            RuntimeValue::Int(n) => Ok(RuntimeValue::Lanes(Rc::new(LanesVal::L16W8(
                logicaffeine_base::Lanes16Word8::splat(n as u8),
            )))),
            other => Err(format!("splat16Word8() requires an Int, got {}", other.type_name())),
        },
        BuiltinId::Shuffle16
        | BuiltinId::InterleaveLo16
        | BuiltinId::InterleaveHi16
        | BuiltinId::ByteAdd16
        | BuiltinId::Maddubs16
        | BuiltinId::Packus16 => {
            let b = args.remove(1);
            let a = args.remove(0);
            match (&a, &b) {
                (RuntimeValue::Lanes(x), RuntimeValue::Lanes(y)) => {
                    let r = match id {
                        BuiltinId::Shuffle16 => (**x).shuffle(**y),
                        BuiltinId::InterleaveLo16 => (**x).interleave_lo(**y),
                        BuiltinId::InterleaveHi16 => (**x).interleave_hi(**y),
                        BuiltinId::ByteAdd16 => (**x).byte_add(**y),
                        BuiltinId::Maddubs16 => (**x).maddubs_bytes(**y),
                        _ => (**x).packus_bytes(**y),
                    };
                    r.map(|v| RuntimeValue::Lanes(Rc::new(v)))
                        .ok_or_else(|| "byte-lane op requires two Lanes16Word8".to_string())
                }
                _ => Err(format!(
                    "byte-lane op requires two Lanes16Word8, got {} and {}",
                    a.type_name(),
                    b.type_name()
                )),
            }
        }
        BuiltinId::ShrBytes16 => {
            let n = args.remove(1);
            let v = args.remove(0);
            match (&v, &n) {
                (RuntimeValue::Lanes(x), RuntimeValue::Int(k)) => (**x)
                    .shr_bytes(*k as u32)
                    .map(|r| RuntimeValue::Lanes(Rc::new(r)))
                    .ok_or_else(|| "shrBytes16() requires a Lanes16Word8".to_string()),
                _ => Err(format!(
                    "shrBytes16() requires a Lanes16Word8 and an Int, got {} and {}",
                    v.type_name(),
                    n.type_name()
                )),
            }
        }
        BuiltinId::Lanes8Word32 => {
            // Pack a Seq of Word32 (or Int) into one 8-lane SIMD vector — the constructor a Logos
            // lane kernel uses to load its working state.
            let s = args.remove(0);
            match s {
                RuntimeValue::List(items) => {
                    let vals = items.borrow().to_values();
                    let mut words = Vec::with_capacity(vals.len());
                    for v in &vals {
                        match v {
                            RuntimeValue::Word(WordVal::W32(w)) => words.push(*w),
                            RuntimeValue::Int(n) => words.push(Word32(*n as u32)),
                            other => {
                                return Err(format!(
                                    "lanes8Word32() needs a Seq of Word32, found {}",
                                    other.type_name()
                                ))
                            }
                        }
                    }
                    Ok(RuntimeValue::Lanes(Rc::new(LanesVal::L8W32(
                        logicaffeine_base::Lanes8Word32::from_words(&words),
                    ))))
                }
                other => Err(format!(
                    "lanes8Word32() requires a Seq of Word32, got {}",
                    other.type_name()
                )),
            }
        }
        BuiltinId::IntOfWord32 => {
            // The unsigned value of a Word32 as an Int (0..2³²−1) — for byte serialization.
            let x = args.remove(0);
            match x {
                RuntimeValue::Word(w) => Ok(RuntimeValue::Int(w.to_u64() as i64)),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n)),
                other => Err(format!("intOfWord32() requires a Word32, got {}", other.type_name())),
            }
        }
        BuiltinId::IntOfWord64 => {
            // The value of a Word64 as an Int (Keccak byte-masked lanes).
            let x = args.remove(0);
            match x {
                RuntimeValue::Word(w) => Ok(RuntimeValue::Int(w.to_u64() as i64)),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n)),
                other => Err(format!("intOfWord64() requires a Word64, got {}", other.type_name())),
            }
        }
        BuiltinId::Word64Shl | BuiltinId::Word64Shr => {
            let is_shl = matches!(id, BuiltinId::Word64Shl);
            let w = args.remove(0);
            let n = args.remove(0);
            let wv = match w {
                RuntimeValue::Word(w) => w.to_u64(),
                RuntimeValue::Int(v) => v as u64,
                other => return Err(format!("word64 shift requires a Word64, got {}", other.type_name())),
            };
            let nv = match n {
                RuntimeValue::Int(v) => v as u32,
                other => return Err(format!("word64 shift amount requires an Int, got {}", other.type_name())),
            };
            let r = if is_shl { wv.wrapping_shl(nv) } else { wv.wrapping_shr(nv) };
            Ok(RuntimeValue::Word(WordVal::W64(Word64(r))))
        }
        BuiltinId::Word64And => {
            let a = args.remove(0);
            let b = args.remove(0);
            let av = match a {
                RuntimeValue::Word(w) => w.to_u64(),
                RuntimeValue::Int(v) => v as u64,
                other => return Err(format!("word64And requires a Word64, got {}", other.type_name())),
            };
            let bv = match b {
                RuntimeValue::Word(w) => w.to_u64(),
                RuntimeValue::Int(v) => v as u64,
                other => return Err(format!("word64And requires a Word64, got {}", other.type_name())),
            };
            Ok(RuntimeValue::Word(WordVal::W64(Word64(av & bv))))
        }
        BuiltinId::Word32Shr => {
            let w = args.remove(0);
            let n = args.remove(0);
            let wv = match w {
                RuntimeValue::Word(word) => word.to_u64() as u32,
                RuntimeValue::Int(v) => v as u32,
                other => return Err(format!("word32Shr requires a Word32, got {}", other.type_name())),
            };
            let nv = match n {
                RuntimeValue::Int(v) => v as u32,
                other => return Err(format!("word32Shr amount requires an Int, got {}", other.type_name())),
            };
            Ok(RuntimeValue::Word(WordVal::W32(Word32(wv.wrapping_shr(nv)))))
        }
        BuiltinId::Word16Make => {
            // Low 16 bits of an Int as a Word16 (ℤ/2¹⁶). No distinct W16 runtime carrier, so it is
            // held in a Word32 with the value in [0, 2¹⁶) — exactly how lanes16Word16/intOfWord16 read it.
            let n = args.remove(0);
            match n {
                RuntimeValue::Int(v) => Ok(RuntimeValue::Word(WordVal::W32(Word32(v as u16 as u32)))),
                RuntimeValue::Word(w) => Ok(RuntimeValue::Word(WordVal::W32(Word32(w.to_u64() as u16 as u32)))),
                other => Err(format!("word16() requires an Int, got {}", other.type_name())),
            }
        }
        BuiltinId::IntOfWord16 => {
            // The unsigned value of a Word16 as an Int (0..2¹⁶−1).
            let x = args.remove(0);
            match x {
                RuntimeValue::Word(w) => Ok(RuntimeValue::Int(w.to_u64() as u16 as i64)),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n as u16 as i64)),
                other => Err(format!("intOfWord16() requires a Word16, got {}", other.type_name())),
            }
        }
        BuiltinId::Lanes4Word64 => {
            // Pack a Seq of Word64/Int into one 4-lane u64 vector (the Poly1305 limb working set).
            let s = args.remove(0);
            match s {
                RuntimeValue::List(items) => {
                    let vals = items.borrow().to_values();
                    let mut words = Vec::with_capacity(vals.len());
                    for v in &vals {
                        match v {
                            RuntimeValue::Word(WordVal::W64(w)) => words.push(*w),
                            RuntimeValue::Int(n) => words.push(Word64(*n as u64)),
                            other => {
                                return Err(format!(
                                    "lanes4Word64() needs a Seq of Word64/Int, found {}",
                                    other.type_name()
                                ))
                            }
                        }
                    }
                    Ok(RuntimeValue::Lanes(Rc::new(LanesVal::L4W64(
                        logicaffeine_base::Lanes4Word64::from_words(&words),
                    ))))
                }
                other => Err(format!("lanes4Word64() requires a Seq, got {}", other.type_name())),
            }
        }
        BuiltinId::SeqOfLanes4 => {
            // Unpack a lane vector into a Seq of its lanes as Int.
            let v = args.remove(0);
            match v {
                RuntimeValue::Lanes(lanes) => {
                    let vals: Vec<RuntimeValue> = (0..lanes.lanes())
                        .map(|i| RuntimeValue::Int(lanes.lane(i) as i64))
                        .collect();
                    Ok(RuntimeValue::List(Rc::new(std::cell::RefCell::new(
                        crate::interpreter::ListRepr::from_values(vals),
                    ))))
                }
                other => Err(format!("seqOfLanes4() requires a lane vector, got {}", other.type_name())),
            }
        }
        BuiltinId::Mul32x32To64 => {
            // Lane-wise widening multiply of the low 32 bits (vpmuludq) — the Poly1305 limb product.
            let a = args.remove(0);
            let b = args.remove(0);
            match (a, b) {
                (RuntimeValue::Lanes(la), RuntimeValue::Lanes(lb)) => match la.mul_lo32_wide(*lb) {
                    Some(v) => Ok(RuntimeValue::Lanes(Rc::new(v))),
                    None => Err(format!(
                        "mul32x32to64 requires two Lanes4Word64, got {} and {}",
                        la.type_name(),
                        lb.type_name()
                    )),
                },
                (a, b) => Err(format!(
                    "mul32x32to64 requires two lane vectors, got {} and {}",
                    a.type_name(),
                    b.type_name()
                )),
            }
        }
        BuiltinId::HsumLanes4 => {
            // The horizontal sum of a lane vector's lanes, as an Int.
            let v = args.remove(0);
            match v {
                RuntimeValue::Lanes(lanes) => Ok(RuntimeValue::Int(lanes.hsum())),
                other => Err(format!("hsumLanes4 requires a lane vector, got {}", other.type_name())),
            }
        }
        BuiltinId::Splat4Word64 => {
            // Broadcast a Word64 into all 4 Keccak lanes. The 4-way lane Keccak is an AOT-only speed
            // path (the tree-walker/VM run the scalar `keccakF` over Word64 instead).
            Err("splat4Word64 compiles to an AVX2 lane broadcast — AOT only, not the interpreter".to_string())
        }
        BuiltinId::AndNot4 => {
            Err("andNot4 compiles to an AVX2 lane vpandn — AOT only, not the interpreter".to_string())
        }
        BuiltinId::Lanes16Word16 => {
            // Pack a Seq of Word16/Int into one 16-lane NTT coefficient vector.
            let s = args.remove(0);
            match s {
                RuntimeValue::List(items) => {
                    let vals = items.borrow().to_values();
                    let mut words = Vec::with_capacity(vals.len());
                    for v in &vals {
                        match v {
                            RuntimeValue::Word(WordVal::W32(w)) => words.push(Word16(w.0 as u16)),
                            RuntimeValue::Int(n) => words.push(Word16(*n as u16)),
                            other => {
                                return Err(format!(
                                    "lanes16Word16() needs a Seq of Word16/Int, found {}",
                                    other.type_name()
                                ))
                            }
                        }
                    }
                    Ok(RuntimeValue::Lanes(Rc::new(LanesVal::L16W16(
                        logicaffeine_base::Lanes16Word16::from_words(&words),
                    ))))
                }
                other => Err(format!("lanes16Word16() requires a Seq, got {}", other.type_name())),
            }
        }
        BuiltinId::SeqOfLanes16 => {
            // Unpack a lane vector into a Seq of its lanes as Int (same as seqOfLanes4, any config).
            let v = args.remove(0);
            match v {
                RuntimeValue::Lanes(lanes) => {
                    let vals: Vec<RuntimeValue> = (0..lanes.lanes())
                        .map(|i| RuntimeValue::Int(lanes.lane(i) as i64))
                        .collect();
                    Ok(RuntimeValue::List(Rc::new(std::cell::RefCell::new(
                        crate::interpreter::ListRepr::from_values(vals),
                    ))))
                }
                other => Err(format!("seqOfLanes16() requires a lane vector, got {}", other.type_name())),
            }
        }
        BuiltinId::Splat16Word16 => {
            // Broadcast a Word16/Int into all 16 lanes (the NTT loads a shared zeta/constant).
            let x = args.remove(0);
            let w = match x {
                RuntimeValue::Word(WordVal::W32(w)) => w.0 as u16,
                RuntimeValue::Int(n) => n as u16,
                other => {
                    return Err(format!("splat16Word16() requires a Word16/Int, got {}", other.type_name()))
                }
            };
            Ok(RuntimeValue::Lanes(Rc::new(LanesVal::L16W16(
                logicaffeine_base::Lanes16Word16::splat(w),
            ))))
        }
        BuiltinId::Mulhi16 => {
            // Lane-wise SIGNED high-16 multiply (vpmulhw) — the Montgomery butterfly's mulhi.
            let a = args.remove(0);
            let b = args.remove(0);
            match (a, b) {
                (RuntimeValue::Lanes(la), RuntimeValue::Lanes(lb)) => match la.mulhi(*lb) {
                    Some(v) => Ok(RuntimeValue::Lanes(Rc::new(v))),
                    None => Err(format!(
                        "mulhi16 requires two Lanes16Word16, got {} and {}",
                        la.type_name(),
                        lb.type_name()
                    )),
                },
                (a, b) => Err(format!(
                    "mulhi16 requires two lane vectors, got {} and {}",
                    a.type_name(),
                    b.type_name()
                )),
            }
        }
        BuiltinId::Montmul32 => {
            // The signed i32 Montgomery multiply (ML-DSA butterfly) — montmul32(a, b, q, qinv).
            let a = args.remove(0);
            let b = args.remove(0);
            let q = args.remove(0);
            let qi = args.remove(0);
            match (a, b, q, qi) {
                (
                    RuntimeValue::Lanes(la),
                    RuntimeValue::Lanes(lb),
                    RuntimeValue::Lanes(lq),
                    RuntimeValue::Lanes(lqi),
                ) => match la.montmul32(*lb, *lq, *lqi) {
                    Some(v) => Ok(RuntimeValue::Lanes(Rc::new(v))),
                    None => Err(format!(
                        "montmul32 requires four Lanes8Word32, got {}, {}, {}, {}",
                        la.type_name(),
                        lb.type_name(),
                        lq.type_name(),
                        lqi.type_name()
                    )),
                },
                (a, b, q, qi) => Err(format!(
                    "montmul32 requires four lane vectors, got {}, {}, {}, {}",
                    a.type_name(),
                    b.type_name(),
                    q.type_name(),
                    qi.type_name()
                )),
            }
        }
        BuiltinId::NttBcastLo | BuiltinId::NttBcastHi => {
            // The within-vector NTT source duplications, at stride h (vperm2i128/vpshufd).
            let is_low = matches!(id, BuiltinId::NttBcastLo);
            let name = if is_low { "nttBcastLo" } else { "nttBcastHi" };
            let v = args.remove(0);
            let h = ntt_stride(args.remove(0), name)?;
            match v {
                RuntimeValue::Lanes(lv) => {
                    let r = if is_low { lv.ntt_bcast_lo(h) } else { lv.ntt_bcast_hi(h) };
                    match r {
                        Some(out) => Ok(RuntimeValue::Lanes(Rc::new(out))),
                        None => Err(format!("{} requires a Lanes16Word16, got {}", name, lv.type_name())),
                    }
                }
                other => Err(format!("{} requires a lane vector, got {}", name, other.type_name())),
            }
        }
        BuiltinId::NttBlend => {
            // Recombine the +/− halves of the within-vector butterfly, at stride h (vperm2i128/vpblendd).
            let a = args.remove(0);
            let b = args.remove(0);
            let h = ntt_stride(args.remove(0), "nttBlend")?;
            match (a, b) {
                (RuntimeValue::Lanes(la), RuntimeValue::Lanes(lb)) => match la.ntt_blend(*lb, h) {
                    Some(v) => Ok(RuntimeValue::Lanes(Rc::new(v))),
                    None => Err(format!(
                        "nttBlend requires two Lanes16Word16, got {} and {}",
                        la.type_name(),
                        lb.type_name()
                    )),
                },
                (a, b) => Err(format!(
                    "nttBlend requires two lane vectors, got {} and {}",
                    a.type_name(),
                    b.type_name()
                )),
            }
        }
        BuiltinId::Splat8Word32 => {
            // Broadcast a Word32 (or Int) into all 8 lanes.
            let x = args.remove(0);
            let w = match x {
                RuntimeValue::Word(WordVal::W32(w)) => w,
                RuntimeValue::Int(n) => Word32(n as u32),
                other => {
                    return Err(format!("splat8Word32() requires a Word32, got {}", other.type_name()))
                }
            };
            Ok(RuntimeValue::Lanes(Rc::new(LanesVal::L8W32(
                logicaffeine_base::Lanes8Word32::splat(w.0),
            ))))
        }
        BuiltinId::SeqOfLanes8 => {
            // Unpack a lane vector back into a Seq of 8 Word32 — read its lanes out.
            let v = args.remove(0);
            match v {
                RuntimeValue::Lanes(lanes) => match *lanes {
                    LanesVal::L8W32(lv) => {
                        let vals: Vec<RuntimeValue> = lv
                            .to_words()
                            .iter()
                            .map(|w| RuntimeValue::Word(WordVal::W32(*w)))
                            .collect();
                        Ok(RuntimeValue::List(Rc::new(std::cell::RefCell::new(
                            crate::interpreter::ListRepr::from_values(vals),
                        ))))
                    }
                    other => Err(format!(
                        "seqOfLanes8() requires a Lanes8Word32, got {}",
                        other.type_name()
                    )),
                },
                other => Err(format!(
                    "seqOfLanes8() requires a Lanes8Word32, got {}",
                    other.type_name()
                )),
            }
        }
        BuiltinId::RunAccepted => {
            // run_accepted(fn, arg, lo, hi): the receiver's typed, bounded acceptance
            // contract for a SHIPPED computation — validate the function's shape and the
            // argument's range, then evaluate in the sandbox. Out-of-range is REFUSED, never
            // clamped; an ordinary (non-shipped) closure is refused at the signature check.
            let int_arg = |v: &RuntimeValue, what: &str| -> Result<i64, String> {
                match v {
                    RuntimeValue::Int(n) => Ok(*n),
                    other => Err(format!("run_accepted: {what} must be an Int, got {}", other.type_name())),
                }
            };
            let arg = int_arg(&args[1], "the argument")?;
            let lo = int_arg(&args[2], "the lower bound")?;
            let hi = int_arg(&args[3], "the upper bound")?;
            let contract = crate::semantics::acceptance::AcceptanceContract::new(lo, hi);
            contract.apply(&args[0], arg).map(RuntimeValue::Int)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every temporal extractor, on a Moment AND a Date, plus every error path — the interpreter
    /// dispatch shared by the tree-walker and the bytecode VM. If any accessor silently returns the
    /// wrong number or fails to reject a bad input, this fails.
    #[test]
    fn temporal_extractors_cover_every_component_and_reject_bad_inputs() {
        use logicaffeine_base::temporal;
        let call = |id, v: &RuntimeValue| call_builtin(id, vec![v.clone()]);
        let int = |id, v: &RuntimeValue| match call(id, v) {
            Ok(RuntimeValue::Int(n)) => n,
            other => panic!("{id:?} on {:?} -> {other:?}", v.type_name()),
        };

        // 2024-03-10T07:30:45Z — a Sunday (weekday 0), ISO week 10, Q1.
        let m = RuntimeValue::Moment(temporal::parse_rfc3339("2024-03-10T07:30:45Z").unwrap());
        assert_eq!(int(BuiltinId::YearOf, &m), 2024);
        assert_eq!(int(BuiltinId::MonthOf, &m), 3);
        assert_eq!(int(BuiltinId::DayOf, &m), 10);
        assert_eq!(int(BuiltinId::WeekdayOf, &m), 0);
        assert_eq!(int(BuiltinId::HourOf, &m), 7);
        assert_eq!(int(BuiltinId::MinuteOf, &m), 30);
        assert_eq!(int(BuiltinId::SecondOf, &m), 45);
        assert_eq!(int(BuiltinId::WeekOf, &m), 10);
        assert_eq!(int(BuiltinId::QuarterOf, &m), 1);
        // date_of / time_of project onto the right runtime types.
        assert!(matches!(call(BuiltinId::DateOf, &m), Ok(RuntimeValue::Date(_))));
        assert!(matches!(call(BuiltinId::TimeOf, &m), Ok(RuntimeValue::Time(_))));

        // The calendar accessors work identically on a bare Date...
        let days = (temporal::parse_rfc3339("2024-03-10T07:30:45Z").unwrap())
            .div_euclid(temporal::NANOS_PER_DAY) as i32;
        let d = RuntimeValue::Date(days);
        assert_eq!(int(BuiltinId::YearOf, &d), 2024);
        assert_eq!(int(BuiltinId::MonthOf, &d), 3);
        assert_eq!(int(BuiltinId::DayOf, &d), 10);
        assert_eq!(int(BuiltinId::WeekdayOf, &d), 0);
        assert_eq!(int(BuiltinId::WeekOf, &d), 10);
        assert_eq!(int(BuiltinId::QuarterOf, &d), 1);
        assert!(matches!(call(BuiltinId::DateOf, &d), Ok(RuntimeValue::Date(x)) if x == days));

        // ...but a Date has NO time-of-day: clock accessors must ERROR, not silently return 0.
        for id in [BuiltinId::HourOf, BuiltinId::MinuteOf, BuiltinId::SecondOf, BuiltinId::TimeOf] {
            assert!(call(id, &d).unwrap_err().contains("no time-of-day"), "{id:?} should reject a Date");
        }

        // Non-temporal inputs are rejected with a typed error — never a bogus number.
        let bogus = RuntimeValue::Int(5);
        for id in [BuiltinId::YearOf, BuiltinId::WeekOf, BuiltinId::QuarterOf, BuiltinId::DateOf, BuiltinId::TimeOf] {
            assert!(call(id, &bogus).is_err(), "{id:?} should reject a non-temporal value");
        }

        // seconds_between requires two Moments, in either position.
        assert!(call_builtin(BuiltinId::SecondsBetween, vec![m.clone(), bogus.clone()]).is_err());
        assert!(call_builtin(BuiltinId::SecondsBetween, vec![bogus.clone(), m.clone()]).is_err());

        // Pre-epoch (negative) Moments decompose by FLOOR division — no negative hour/second.
        let pre = RuntimeValue::Moment(temporal::parse_rfc3339("1969-12-31T23:59:58Z").unwrap());
        assert_eq!(int(BuiltinId::YearOf, &pre), 1969);
        assert_eq!(int(BuiltinId::MonthOf, &pre), 12);
        assert_eq!(int(BuiltinId::DayOf, &pre), 31);
        assert_eq!(int(BuiltinId::HourOf, &pre), 23);
        assert_eq!(int(BuiltinId::MinuteOf, &pre), 59);
        assert_eq!(int(BuiltinId::SecondOf, &pre), 58);
    }

    /// `date_of` is the inverse-projection partner of the component accessors: extracting the date
    /// then re-reading its components must agree with reading them off the Moment directly.
    #[test]
    fn date_of_then_components_agree_with_the_moment() {
        use logicaffeine_base::temporal;
        for ts in ["2024-03-10T07:30:45Z", "2000-02-29T00:00:00Z", "1969-12-31T23:59:58Z"] {
            let m = RuntimeValue::Moment(temporal::parse_rfc3339(ts).unwrap());
            let d = call_builtin(BuiltinId::DateOf, vec![m.clone()]).unwrap();
            for id in [BuiltinId::YearOf, BuiltinId::MonthOf, BuiltinId::DayOf, BuiltinId::WeekdayOf, BuiltinId::WeekOf, BuiltinId::QuarterOf] {
                let from_moment = call_builtin(id, vec![m.clone()]).unwrap();
                let from_date = call_builtin(id, vec![d.clone()]).unwrap();
                assert_eq!(from_moment, from_date, "{id:?} disagrees for {ts}");
            }
        }
    }

    #[test]
    fn run_accepted_validates_then_runs_a_shipped_computation() {
        use crate::concurrency::marshal::GenExpr;
        use crate::interpreter::ClosureValue;
        use std::collections::HashMap;
        use std::rc::Rc;
        // A shipped `3·x + 1` computation (what a peer ships via `Send computed`).
        let mk = || {
            let gen = GenExpr::Add(
                Box::new(GenExpr::Mul(Box::new(GenExpr::Index), Box::new(GenExpr::Const(3)))),
                Box::new(GenExpr::Const(1)),
            );
            RuntimeValue::Function(Box::new(ClosureValue {
                body_index: usize::MAX,
                captured_env: HashMap::default(),
                param_names: vec![logicaffeine_base::Symbol::from_index(0)],
                generated: Some(Rc::new(gen)),
            }))
        };
        // In-bounds (5 ∈ [0,1000]) → 3·5 + 1 = 16, run in the sandbox.
        match call_builtin(
            BuiltinId::RunAccepted,
            vec![mk(), RuntimeValue::Int(5), RuntimeValue::Int(0), RuntimeValue::Int(1000)],
        ) {
            Ok(RuntimeValue::Int(n)) => assert_eq!(n, 16),
            other => panic!("in-bounds run_accepted should return Int(16), got {other:?}"),
        }
        // Out-of-bounds (5000 ∉ [0,1000]) → refused, not clamped.
        assert!(
            call_builtin(
                BuiltinId::RunAccepted,
                vec![mk(), RuntimeValue::Int(5000), RuntimeValue::Int(0), RuntimeValue::Int(1000)],
            )
            .is_err(),
            "an out-of-range argument must be refused at the contract"
        );
    }

    #[test]
    fn arity_messages_match_treewalker() {
        assert_eq!(
            check_arity(BuiltinId::Length, 2).unwrap_err(),
            "length() takes exactly 1 argument"
        );
        assert_eq!(
            check_arity(BuiltinId::Min, 1).unwrap_err(),
            "min() takes exactly 2 arguments"
        );
        assert!(check_arity(BuiltinId::Format, 0).is_ok());
        assert!(check_arity(BuiltinId::Format, 5).is_ok());
    }

    #[test]
    fn parse_and_chr_messages() {
        let e = call_builtin(
            BuiltinId::ParseInt,
            vec![RuntimeValue::Text(Rc::new("zz".to_string()))],
        )
        .unwrap_err();
        assert_eq!(e, "Cannot parse 'zz' as Int");
        let e = call_builtin(BuiltinId::Chr, vec![RuntimeValue::Int(-1)]).unwrap_err();
        assert_eq!(e, "Invalid character code: -1");
        let r = call_builtin(BuiltinId::Chr, vec![RuntimeValue::Int(65)]).unwrap();
        assert!(matches!(&r, RuntimeValue::Text(s) if **s == "A"));
    }

    #[test]
    fn numeric_builtins_coerce_like_treewalker() {
        assert!(matches!(
            call_builtin(BuiltinId::Sqrt, vec![RuntimeValue::Int(9)]).unwrap(),
            RuntimeValue::Float(f) if f == 3.0
        ));
        assert!(matches!(
            call_builtin(BuiltinId::Min, vec![RuntimeValue::Int(3), RuntimeValue::Float(2.5)]).unwrap(),
            RuntimeValue::Float(f) if f == 2.5
        ));
        assert!(matches!(
            call_builtin(BuiltinId::Floor, vec![RuntimeValue::Float(2.9)]).unwrap(),
            RuntimeValue::Int(2)
        ));
        assert!(matches!(
            call_builtin(BuiltinId::Pow, vec![RuntimeValue::Int(2), RuntimeValue::Int(10)]).unwrap(),
            RuntimeValue::Int(1024)
        ));
        // Negative Int exponent goes to float.
        assert!(matches!(
            call_builtin(BuiltinId::Pow, vec![RuntimeValue::Int(2), RuntimeValue::Int(-1)]).unwrap(),
            RuntimeValue::Float(f) if f == 0.5
        ));
    }

    #[test]
    fn decimal_builtin_constructs_exact_money() {
        // decimal("19.99") is an exact, scale-preserving Decimal — the non-breaking entry
        // into the money tower.
        let d = call_builtin(BuiltinId::Decimal, vec![RuntimeValue::Text(Rc::new("19.99".into()))]).unwrap();
        assert!(matches!(&d, RuntimeValue::Decimal(_)));
        assert_eq!(d.to_display_string(), "19.99");
        // An Int widens to a scale-0 Decimal.
        let i = call_builtin(BuiltinId::Decimal, vec![RuntimeValue::Int(5)]).unwrap();
        assert!(matches!(&i, RuntimeValue::Decimal(_)));
        assert_eq!(i.to_display_string(), "5");
        // Garbage text is a clean error, never a panic.
        let e = call_builtin(BuiltinId::Decimal, vec![RuntimeValue::Text(Rc::new("zz".into()))]).unwrap_err();
        assert_eq!(e, "Cannot parse 'zz' as Decimal");
        // Arity is exactly one.
        assert_eq!(check_arity(BuiltinId::Decimal, 2).unwrap_err(), "decimal() takes exactly 1 argument");
    }

    #[test]
    fn complex_builtin_constructs_exact_complex_numbers() {
        // complex(0, 1) is the imaginary unit i.
        let i = call_builtin(BuiltinId::Complex, vec![RuntimeValue::Int(0), RuntimeValue::Int(1)]).unwrap();
        assert!(matches!(&i, RuntimeValue::Complex(_)));
        assert_eq!(i.to_display_string(), "i");
        // complex(3, 4) → 3+4i; complex(0, -1) → -i.
        let z = call_builtin(BuiltinId::Complex, vec![RuntimeValue::Int(3), RuntimeValue::Int(4)]).unwrap();
        assert_eq!(z.to_display_string(), "3+4i");
        let neg_i = call_builtin(BuiltinId::Complex, vec![RuntimeValue::Int(0), RuntimeValue::Int(-1)]).unwrap();
        assert_eq!(neg_i.to_display_string(), "-i");
        let _ = i; // the imaginary unit; its i·i = −1 is covered in the arith tests.
        // An inexact Float part is refused — exactness preserved.
        assert!(call_builtin(BuiltinId::Complex, vec![RuntimeValue::Float(1.0), RuntimeValue::Int(1)]).is_err());
        // Arity is exactly two.
        assert_eq!(check_arity(BuiltinId::Complex, 1).unwrap_err(), "complex() takes exactly 2 arguments");
    }

    #[test]
    fn decimal_and_complex_pass_through_the_numeric_builtins() {
        let d = |s: &str| RuntimeValue::Decimal(Rc::new(Decimal::parse(s).unwrap()));
        // abs preserves Decimal + scale; floor/ceil/round give the exact Int; min/max value-based.
        assert_eq!(call_builtin(BuiltinId::Abs, vec![d("-0.05")]).unwrap().to_display_string(), "0.05");
        assert_eq!(call_builtin(BuiltinId::Floor, vec![d("19.99")]).unwrap().to_display_string(), "19");
        assert_eq!(call_builtin(BuiltinId::Ceil, vec![d("19.01")]).unwrap().to_display_string(), "20");
        assert_eq!(call_builtin(BuiltinId::Round, vec![d("2.5")]).unwrap().to_display_string(), "3");
        assert_eq!(call_builtin(BuiltinId::Round, vec![d("-19.99")]).unwrap().to_display_string(), "-20");
        assert_eq!(call_builtin(BuiltinId::Min, vec![d("0.10"), d("0.2")]).unwrap().to_display_string(), "0.10");
        assert_eq!(call_builtin(BuiltinId::Max, vec![d("0.10"), d("0.2")]).unwrap().to_display_string(), "0.2");
        // |3+4i| = 5 (the modulus, a Float view of a generally-irrational magnitude).
        let z = RuntimeValue::Complex(Rc::new(logicaffeine_base::Complex::new(
            logicaffeine_base::Rational::from_i64(3),
            logicaffeine_base::Rational::from_i64(4),
        )));
        match call_builtin(BuiltinId::Abs, vec![z]).unwrap() {
            RuntimeValue::Float(f) => assert!((f - 5.0).abs() < 1e-12),
            other => panic!("expected a Float magnitude, got {other:?}"),
        }
    }

    #[test]
    fn modular_builtin_constructs_and_exponentiates_in_the_ring() {
        // modular(10, 7) reduces to 3 (mod 7).
        let x = call_builtin(BuiltinId::Modular, vec![RuntimeValue::Int(10), RuntimeValue::Int(7)]).unwrap();
        assert!(matches!(&x, RuntimeValue::Modular(_)));
        assert_eq!(x.to_display_string(), "3 (mod 7)");
        // pow(modular(3,5), 4) = 81 ≡ 1 (mod 5) — fast modular exponentiation via the builtin.
        let base = call_builtin(BuiltinId::Modular, vec![RuntimeValue::Int(3), RuntimeValue::Int(5)]).unwrap();
        let p = call_builtin(BuiltinId::Pow, vec![base, RuntimeValue::Int(4)]).unwrap();
        assert_eq!(p.to_display_string(), "1 (mod 5)");
        // A non-positive modulus is refused; arity is exactly two.
        assert!(call_builtin(BuiltinId::Modular, vec![RuntimeValue::Int(3), RuntimeValue::Int(0)]).is_err());
        assert_eq!(check_arity(BuiltinId::Modular, 1).unwrap_err(), "modular() takes exactly 2 arguments");
    }

    #[test]
    fn quantity_builtins_construct_convert_and_compute_exactly() {
        use crate::semantics::arith::{add, divide, multiply, subtract};
        let q = |v: i64, u: &str| {
            call_builtin(BuiltinId::Quantity, vec![RuntimeValue::Int(v), RuntimeValue::Text(Rc::new(u.into()))]).unwrap()
        };
        let conv = |x: RuntimeValue, u: &str| {
            call_builtin(BuiltinId::Convert, vec![x, RuntimeValue::Text(Rc::new(u.into()))])
        };
        // Construction carries the display unit.
        assert!(matches!(&q(2, "inch"), RuntimeValue::Quantity(_)));
        assert_eq!(q(2, "inch").to_display_string(), "2 in");
        assert_eq!(q(20, "celsius").to_display_string(), "20 °C");
        // THE GOLDEN: 2 inches + 5 centimeters, in feet, is EXACTLY 42/127 ft — no float.
        let sum = add(q(2, "inch"), q(5, "centimeter")).unwrap();
        assert_eq!(conv(sum, "foot").unwrap().to_display_string(), "42/127 ft");
        // Same-dimension subtraction keeps the left operand's unit; the magnitude is exact.
        assert_eq!(subtract(q(1, "meter"), q(50, "centimeter")).unwrap().to_display_string(), "1/2 m");
        // Scaling by a dimensionless number preserves the unit (× and ÷).
        assert_eq!(multiply(q(2, "inch"), RuntimeValue::Int(3)).unwrap().to_display_string(), "6 in");
        assert_eq!(multiply(RuntimeValue::Int(3), q(2, "inch")).unwrap().to_display_string(), "6 in");
        assert_eq!(divide(q(6, "inch"), RuntimeValue::Int(2)).unwrap().to_display_string(), "3 in");
        // Length × Length = Area, shown in dimension form until a named compound unit exists.
        assert_eq!(multiply(q(3, "meter"), q(4, "meter")).unwrap().to_display_string(), "12 L^2");
        // Length ÷ Time = Speed.
        assert_eq!(divide(q(100, "meter"), q(10, "second")).unwrap().to_display_string(), "10 L·T^-1");
        // Quantity equality is PHYSICAL: 100 cm == 1 m (display unit is presentation only).
        assert_eq!(q(100, "centimeter"), q(1, "meter"));
        // Dimension mismatch on + is a clean typed error (Length + Mass), never a silent coercion.
        assert!(add(q(1, "meter"), q(1, "kilogram")).is_err());
        // Converting across dimensions is the forbidden cast.
        assert!(conv(q(1, "meter"), "kilogram").is_err());
        // Unknown unit names and Float magnitudes are clean errors (exactness preserved).
        assert!(call_builtin(BuiltinId::Quantity,
            vec![RuntimeValue::Int(1), RuntimeValue::Text(Rc::new("zorgle".into()))]).is_err());
        assert!(call_builtin(BuiltinId::Quantity,
            vec![RuntimeValue::Float(1.5), RuntimeValue::Text(Rc::new("meter".into()))]).is_err());
        // Arity is exactly two for both builtins.
        assert_eq!(check_arity(BuiltinId::Quantity, 1).unwrap_err(), "quantity() takes exactly 2 arguments");
        assert_eq!(check_arity(BuiltinId::Convert, 3).unwrap_err(), "convert() takes exactly 2 arguments");
    }

    #[test]
    fn abs_and_pow_are_exact_promoting_past_i64() {
        // Arrange / Act / Assert: |i64::MIN| = 2^63 has no i64 representation, so abs
        // promotes to the EXACT BigInt rather than wrapping back to i64::MIN.
        let abs_min = call_builtin(BuiltinId::Abs, vec![RuntimeValue::Int(i64::MIN)]).unwrap();
        assert_eq!(abs_min.to_display_string(), "9223372036854775808");
        assert_eq!(abs_min.type_name(), "Int", "a BigInt is still an integer type");

        // 2^63 overflows i64 → exact BigInt (not the wrapped i64::MIN).
        let two_pow_63 = call_builtin(BuiltinId::Pow, vec![RuntimeValue::Int(2), RuntimeValue::Int(63)]).unwrap();
        assert_eq!(two_pow_63.to_display_string(), "9223372036854775808");

        // A far-larger power stays exact (2^100 = 31 digits).
        let two_pow_100 = call_builtin(BuiltinId::Pow, vec![RuntimeValue::Int(2), RuntimeValue::Int(100)]).unwrap();
        assert_eq!(two_pow_100.to_display_string(), "1267650600228229401496703205376");

        // In-range results stay narrow `Int` (downsizing is automatic).
        assert!(matches!(
            call_builtin(BuiltinId::Pow, vec![RuntimeValue::Int(3), RuntimeValue::Int(4)]).unwrap(),
            RuntimeValue::Int(81)
        ));
        // abs of an ordinary negative is unchanged.
        assert!(matches!(
            call_builtin(BuiltinId::Abs, vec![RuntimeValue::Int(-5)]).unwrap(),
            RuntimeValue::Int(5)
        ));
    }

    #[test]
    fn copy_is_deep() {
        use std::cell::RefCell;
        let inner = std::rc::Rc::new(RefCell::new(
            crate::interpreter::ListRepr::from_values(vec![RuntimeValue::Int(1)]),
        ));
        let original = RuntimeValue::List(inner.clone());
        let copied = call_builtin(BuiltinId::Copy, vec![original]).unwrap();
        if let RuntimeValue::List(copied_items) = &copied {
            inner.borrow_mut().push(RuntimeValue::Int(2));
            assert_eq!(copied_items.borrow().len(), 1, "copy must not share the allocation");
        } else {
            panic!("copy changed the type");
        }
    }
}
