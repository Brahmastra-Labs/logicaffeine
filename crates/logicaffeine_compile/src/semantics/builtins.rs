//! Built-in functions over already-evaluated values.
//!
//! `show` is NOT here — output is an engine concern. Arity is checked by the
//! caller BEFORE evaluating arguments (via [`check_arity`]) to preserve the
//! tree-walker's error ordering: a wrong-arity call reports the arity error
//! even when an argument expression would itself error.

use std::cell::RefCell;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::interpreter::{InductiveValue, ListRepr, RuntimeValue};
use logicaffeine_base::{Decimal, LanesVal, Word16, Word32, Word64, Word8, WordVal};

/// A stdlib `Result` (core.lg: `Ok (value)` / `Err (error)`) as the executing
/// tiers represent it — the shape `Inspect`'s `When Ok (…)`/`When Err (…)` arms
/// match. The return seam for natives declared `-> Result …`.
pub(crate) fn result_ok(v: RuntimeValue) -> RuntimeValue {
    RuntimeValue::Inductive(std::rc::Rc::new(InductiveValue {
        inductive_type: "Result".to_string(),
        constructor: "Ok".to_string(),
        args: vec![v],
    }))
}

pub(crate) fn result_err(msg: String) -> RuntimeValue {
    RuntimeValue::Inductive(std::rc::Rc::new(InductiveValue {
        inductive_type: "Result".to_string(),
        constructor: "Err".to_string(),
        args: vec![RuntimeValue::Text(Rc::new(msg))],
    }))
}

/// The standard library's system natives that run through [`call_builtin`] — file.lg's `read` and
/// `write`, random.lg's `randomInt` and `randomFloat`, env.lg's `get`, time.lg's `now` — by name and
/// arity. The one table the tree-walker and the VM compiler both consult, so a native cannot be wired
/// on one tier and fall through to its empty body on the other. A caller asks it only for a
/// `## To native` declaration: a program's own function of the same name always wins.
pub fn system_native(name: &str, argc: usize) -> Option<BuiltinId> {
    match (name, argc) {
        ("read", 1) => Some(BuiltinId::FileRead),
        ("write", 2) => Some(BuiltinId::FileWrite),
        ("randomInt", 2) => Some(BuiltinId::RandomInt),
        ("randomFloat", 0) => Some(BuiltinId::RandomFloat),
        ("get", 1) => Some(BuiltinId::EnvGet),
        ("now", 0) => Some(BuiltinId::TimeNow),
        _ => None,
    }
}

/// The natives the Rust backends link from `logicaffeine_system` and no other engine runs, as
/// `(module, function)`: the FIPS-202 hashes, which a program declares native to take the Rust
/// implementation in place of crypto.lg's pure-LOGOS sponge. The Rust backends' table reads this, and
/// every other engine refuses a call to one by the Rust function it names.
pub fn rust_only_native(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "sha3_256" => Some(("keccak", "sha3_256")),
        "sha3_512" => Some(("keccak", "sha3_512")),
        "shake128" => Some(("keccak", "shake128")),
        "shake256" => Some(("keccak", "shake256")),
        _ => None,
    }
}

/// What a call to a `## To native` declaration no engine implements is refused with, on every tier.
/// A declaration naming a Rust path (`is native "crate::module::function"`), or a native only the
/// Rust backends link ([`rust_only_native`]), runs only where Rust links it, as `Escape to Rust:`
/// does; one naming no path and no system native is the Rust backends' own compile-time refusal, in
/// their words.
pub fn unimplemented_native(name: &str, rust_path: Option<&str>) -> String {
    let linked = rust_only_native(name)
        .map(|(module, function)| format!("logicaffeine_system::{module}::{function}"));
    match rust_path.map(str::to_string).or(linked) {
        Some(path) => format!(
            "`{name}` is the Rust function `{path}`, which only the Rust backends compile. \
             Use `largo build` or `largo run` to compile and run this program."
        ),
        None => format!(
            "Unknown system native function: '{name}'. Use `is native \"crate::path\"` syntax for \
             user-defined native functions."
        ),
    }
}

/// The variable `key` names in the environment the program runs in, or `None` when it is unset:
/// the process environment through the system seam, which a sealed test world answers instead. A
/// browser has no process environment, so there only a sealed world can name a variable.
fn environment_variable(key: &str) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        logicaffeine_system::env::get(key.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
        logicaffeine_system::testworld::intercept_env_get(key).flatten()
    }
}

/// Read a `Seq of Int` as raw bytes (each element masked to 0–255) — the byte-array convention the
/// UUID/hash builtins share with `uuid.lg` so the version constructors can be written in LOGOS.
/// Why a file statement's read of the sandboxed file system fails, as every tier reports it.
pub fn vfs_read_failure(e: &logicaffeine_system::fs::VfsError) -> String {
    format!("Read error: {e}")
}

/// Why a file statement's write to the sandboxed file system fails, as every tier reports it.
pub fn vfs_write_failure(e: &logicaffeine_system::fs::VfsError) -> String {
    format!("Write error: {e}")
}

/// The answer of one operation on the sandboxed in-memory file system. `MemVfs` never awaits anything, so
/// its future is complete on its first poll; polling it once keeps a builtin callable from inside an async
/// runner, where blocking on a nested executor would panic.
fn sandboxed<T>(op: impl std::future::Future<Output = T>) -> Result<T, String> {
    use futures::FutureExt as _;
    Box::pin(op)
        .now_or_never()
        .ok_or_else(|| "the sandboxed file system did not answer at once".to_string())
}

pub(crate) fn byte_seq(v: &RuntimeValue) -> Result<Vec<u8>, String> {
    match v {
        RuntimeValue::List(l) => {
            let l = l.borrow();
            let mut out = Vec::with_capacity(l.len());
            for i in 0..l.len() {
                match l.get(i) {
                    Some(RuntimeValue::Int(n)) => out.push((n & 0xff) as u8),
                    _ => {
                        return Err(format!(
                            "expected a Seq of Int (bytes); element {} is not an Int",
                            i + 1
                        ))
                    }
                }
            }
            Ok(out)
        }
        _ => Err(format!(
            "expected a Seq of Int (bytes), got {}",
            v.type_name()
        )),
    }
}

/// A `Seq of Int` argument of `function` as bytes, refused as `crypto.lg`'s `cryptoRequireBytes` refuses
/// it: an item outside 0..=255 is named, never folded into a byte.
pub(crate) fn exact_byte_seq(function: &str, what: &str, v: &RuntimeValue) -> Result<Vec<u8>, String> {
    let RuntimeValue::List(l) = v else {
        return Err(format!(
            "{function}(): the {what} must be a Seq of Int (bytes), got {}",
            v.type_name()
        ));
    };
    let l = l.borrow();
    let mut out = Vec::with_capacity(l.len());
    for i in 0..l.len() {
        match l.get(i) {
            Some(RuntimeValue::Int(n)) => match u8::try_from(n) {
                Ok(b) => out.push(b),
                Err(_) => {
                    return Err(format!(
                        "{function}(): the {what} must be bytes (0 to 255); item {} is {n}",
                        i + 1
                    ))
                }
            },
            _ => {
                return Err(format!(
                    "{function}(): the {what} must be a Seq of Int (bytes); item {} is not an Int",
                    i + 1
                ))
            }
        }
    }
    Ok(out)
}

/// `pbkdf2Sha256`'s arguments as the pure `crypto.lg` body takes them, in its order and with its
/// reports, for the tiers that run it on [`crate::crypto_accel::pbkdf2_sha256`]: a byte password, a byte
/// salt, and an iteration count from 1 to 2^32 − 1 (RFC 8018 §5.2 runs a positive count).
pub(crate) fn pbkdf2_arguments(
    password: &RuntimeValue,
    salt: &RuntimeValue,
    iters: &RuntimeValue,
) -> Result<(Vec<u8>, Vec<u8>, u32), String> {
    let password = exact_byte_seq("pbkdf2Sha256", "password", password)?;
    let salt = exact_byte_seq("pbkdf2Sha256", "salt", salt)?;
    let iters = match iters {
        RuntimeValue::Int(n) => u32::try_from(*n).ok().filter(|&c| c >= 1).ok_or_else(|| {
            format!("pbkdf2Sha256(): the iteration count must be from 1 to 4294967295; it is {n}")
        })?,
        other => {
            return Err(format!(
                "pbkdf2Sha256(): the iteration count must be an Int, got {}",
                other.type_name()
            ))
        }
    };
    Ok((password, salt, iters))
}

/// Read `fetchSend`'s headers argument: a `Seq of Text` alternating name,value —
/// the flat shape that crosses the wasm ABI as one seq handle (a `Map` has no
/// host-side layout there). An odd element count is a hard error at the seam.
fn fetch_header_pairs(v: &RuntimeValue) -> Result<Vec<(String, String)>, String> {
    let RuntimeValue::List(l) = v else {
        return Err(format!(
            "fetchSend headers must be a Seq of Text alternating name,value, got {}",
            v.type_name()
        ));
    };
    let l = l.borrow();
    if l.len() % 2 != 0 {
        return Err(format!(
            "fetchSend headers must alternate name,value ({} elements is odd)",
            l.len()
        ));
    }
    let mut out = Vec::with_capacity(l.len() / 2);
    for i in (0..l.len()).step_by(2) {
        let name = l.get(i).map(|x| x.to_display_string()).unwrap_or_default();
        let value = l
            .get(i + 1)
            .map(|x| x.to_display_string())
            .unwrap_or_default();
        out.push((name, value));
    }
    Ok(out)
}

/// `randomInt(lo, hi)` — a uniform draw from `lo..=hi` through the one system seam (a sealed test
/// world decides it), or the refusal of a range whose low bound is above its high bound, which
/// every host words alike.
pub fn random_int(lo: i64, hi: i64) -> Result<i64, String> {
    if lo > hi {
        return Err(format!(
            "randomInt({lo}, {hi}): the low bound is above the high bound"
        ));
    }
    Ok(logicaffeine_system::random::randomInt(lo, hi))
}

/// A turn's changed fields from `noteTurn`'s flattened `[field, before, after, …]` Seq of Text, as
/// the journal's `(field, before, after)` triples.
fn journal_changes(v: &RuntimeValue) -> Result<Vec<(String, String, String)>, String> {
    let RuntimeValue::List(l) = v else {
        return Err(format!(
            "noteTurn changes must be a Seq of Text [field, before, after, …], got {}",
            v.type_name()
        ));
    };
    let l = l.borrow();
    if l.len() % 3 != 0 {
        return Err(format!(
            "noteTurn changes come in [field, before, after] triples ({} elements is not one)",
            l.len()
        ));
    }
    let text = |i: usize| l.get(i).map(|x| x.to_display_string()).unwrap_or_default();
    Ok((0..l.len())
        .step_by(3)
        .map(|i| (text(i), text(i + 1), text(i + 2)))
        .collect())
}

/// A 32-byte ChaCha20-Poly1305 key from a `Seq of Int`; an item that is not a byte or a wrong length is
/// a hard error at the seam (a key is never masked, padded or truncated into a weaker one).
fn read_aead_key(v: &RuntimeValue, who: &str) -> Result<[u8; 32], String> {
    let bytes = exact_byte_seq(who, "key", v)?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("{who}(): key must be exactly 32 bytes, got {}", bytes.len()))
}

/// A box from a `Seq of Int`: bytes, and at least its nonce and tag long.
fn read_box(v: &RuntimeValue, who: &str) -> Result<Vec<u8>, String> {
    let sealed = exact_byte_seq(who, "box", v)?;
    if sealed.len() < logicaffeine_system::aead::BOX_OVERHEAD {
        return Err(format!(
            "{who}(): the box is {} bytes; a box is at least {}",
            sealed.len(),
            logicaffeine_system::aead::BOX_OVERHEAD
        ));
    }
    Ok(sealed)
}

/// What opening a box that does not authenticate stops with.
fn box_refused(who: &str) -> String {
    format!("{who}(): authentication failed — the box was tampered, truncated, or the key is wrong")
}

/// A 12-byte ChaCha20-Poly1305 nonce from a `Seq of Int`; an item that is not a byte or a wrong length
/// is a hard error.
fn read_aead_nonce(v: &RuntimeValue, who: &str) -> Result<[u8; 12], String> {
    let bytes = exact_byte_seq(who, "nonce", v)?;
    bytes.as_slice().try_into().map_err(|_| {
        format!(
            "{who}(): nonce must be exactly 12 bytes, got {}",
            bytes.len()
        )
    })
}

/// Build a `Seq of Int` from raw bytes (the packed-`i64` list repr).
pub(crate) fn bytes_to_seq(bytes: &[u8]) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(
        bytes.iter().map(|&b| b as i64).collect(),
    ))))
}

/// Read a `Seq of Seq of Int` as FEC shards — each inner `Seq of Int` decoded as bytes via
/// [`byte_seq`]. Equal-length shards are enforced downstream by the codec.
fn read_shards(v: &RuntimeValue) -> Result<Vec<Vec<u8>>, String> {
    match v {
        RuntimeValue::List(l) => {
            let l = l.borrow();
            let mut shards = Vec::with_capacity(l.len());
            for i in 0..l.len() {
                match l.get(i) {
                    Some(elem) => shards.push(byte_seq(&elem)?),
                    None => return Err(format!("shard {} is missing", i + 1)),
                }
            }
            Ok(shards)
        }
        _ => Err(format!(
            "expected a Seq of Seq of Int (shards), got {}",
            v.type_name()
        )),
    }
}

/// Read a `Seq` of (`Seq of Int` | `nothing`) as received FEC shards — `nothing` marks an
/// erased (lost) slot; any other element is a surviving shard decoded via [`byte_seq`].
fn read_received(v: &RuntimeValue) -> Result<Vec<Option<Vec<u8>>>, String> {
    match v {
        RuntimeValue::List(l) => {
            let l = l.borrow();
            let mut out = Vec::with_capacity(l.len());
            for i in 0..l.len() {
                match l.get(i) {
                    Some(RuntimeValue::Nothing) => out.push(None),
                    Some(elem) => out.push(Some(byte_seq(&elem)?)),
                    None => return Err(format!("received slot {} is missing", i + 1)),
                }
            }
            Ok(out)
        }
        _ => Err(format!(
            "expected a Seq of (Seq of Int or nothing), got {}",
            v.type_name()
        )),
    }
}

/// Build a `Seq of Seq of Int` from FEC shards — the inverse of [`read_shards`].
fn shards_to_seq(shards: &[Vec<u8>]) -> RuntimeValue {
    let rows: Vec<RuntimeValue> = shards.iter().map(|s| bytes_to_seq(s)).collect();
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))))
}

/// Read a positive `Int` shard/parity count, with a labelled error on a non-positive or non-Int.
fn read_positive_count(v: RuntimeValue, label: &str) -> Result<usize, String> {
    match v {
        RuntimeValue::Int(n) if n > 0 => Ok(n as usize),
        RuntimeValue::Int(n) => Err(format!("{label} must be positive, got {n}")),
        other => Err(format!("{label} must be an Int, got {}", other.type_name())),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuiltinId {
    Length,
    /// `contains(haystack, needle)` — true iff `needle` is a substring of
    /// `haystack` (both `Text`). The empty needle is contained in everything.
    Contains,
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
    /// `complex(re, im)` — an exact complex from two exact reals, or a floating-point complex (`ComplexF`) if either part is a `Float`.
    /// `complex(0, 1)` is the imaginary unit `i`; `complex(0,1) * complex(0,1) = −1`.
    Complex,
    /// `conjugate(z)` — the complex conjugate `re − im·i` (exact `Complex` stays exact, `ComplexF`
    /// stays float; a real number is its own conjugate). The primitive for Hermitian transpose and
    /// the conjugated dot product `xᴴy` in complex BLAS/LAPACK.
    Conjugate,
    /// `re(z)` — the real part of a complex number as a `Float` (a real number → itself as a Float).
    RealPart,
    /// `im(z)` — the imaginary part of a complex number as a `Float` (a real number → 0.0).
    ImagPart,
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
    /// `compressBytes(seq)` — squeeze a `Seq of Int` byte payload with the SMALLEST of the codec's
    /// built-in compressors (deflate/lz4/zstd, self-describing tag byte); stores raw when nothing
    /// shrinks it, so it never grows by more than one byte. The streaming knob for redundant state
    /// (CRDT deltas, rosters, keypoints) — run it BEFORE `encrypt`, since ciphertext is incompressible,
    /// and never on a payload that carries a secret beside anything an attacker chooses: the compressed
    /// length then gives the secret away a guess at a time (CRIME, BREACH).
    CompressBytes,
    /// `decompressBytes(seq)` — the exact inverse of `compressBytes`: read the codec tag and inflate
    /// (with the decompression-bomb bound). Errors on a malformed frame.
    DecompressBytes,
    /// `sealBytes(key, nonce, plaintext)` — the secure streaming frame: ChaCha20-Poly1305
    /// authenticated-encrypt the plaintext (32-byte key, 12-byte nonce, both `Seq of Int`), which is never
    /// compressed first — a frame's length tells nothing of its plaintext but its length. The 16-byte
    /// Poly1305 tag makes the frame tamper-evident. The caller keeps every nonce under a key distinct;
    /// `sealBox` draws one instead. Inverse: `openBytes`.
    SealBytes,
    /// `openBytes(key, nonce, sealed)` — verify the Poly1305 tag (constant-time) and decrypt, returning
    /// the plaintext. ERRORS on a wrong key, a tampered frame, or truncation — it never yields
    /// unauthenticated bytes.
    OpenBytes,
    /// `fromWireBytes(seq)` — the inverse of `wireBytes`: decode a self-describing wire byte `Seq of
    /// Int` back into the value it encoded (Int/Text/Bool/Seq/Set/Map/Struct/Tuple/…, no schema
    /// needed). Errors on a malformed or trailing-byte frame. Completes the round trip a receiver needs.
    FromWireBytes,
    /// `sealFrame(key, nonce, value)` — the one-call secure streaming frame for a VALUE: wire-encode,
    /// then ChaCha20-Poly1305 authenticated-encrypt. `openFrame` reverses it straight back to the value.
    /// Fuses `wireBytes`+`sealBytes` with no intermediate Seq; `sealValueBox` draws its nonce instead.
    SealFrame,
    /// `openFrame(key, nonce, sealed)` — verify, decrypt, and wire-decode a `sealFrame` back into the
    /// original value. Errors on tamper / wrong key / malformed frame.
    OpenFrame,
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
    /// `year_of` / `month_of` (1–12) / `day_of` (1–31) / `weekday_of` (ISO 8601: 1 = Monday … 7 = Sunday).
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
    /// `makeDate(year, month, day)` — construct a `Date` from civil components via base `days_from_civil`.
    MakeDate,
    /// `makeMoment(year, month, day, hour, minute, second)` — construct a UTC `Moment` from civil
    /// components. Defined to agree with `calendar.timegm` to the second (same `days_from_civil` core).
    MakeMoment,
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
    /// `leading_zeros(x)` — count of high zero bits in the Int's u64 pattern
    /// (`(x as u64).leading_zeros()`; 64 for x = 0). Backs `bitLength`.
    LeadingZeros,
    /// `trailing_zeros(x)` — count of low zero bits in the Int's u64 pattern
    /// (`(x as u64).trailing_zeros()`; 64 for x = 0). Backs `trailingZeros`.
    TrailingZeros,
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
    /// `reedSolomonEncode(data, parity)` — systematic Reed-Solomon FEC over GF(2^8).
    /// `data` is a `Seq of Seq of Int` (byte shards of equal length); returns
    /// `data.count + parity` shards, the first `data.count` being the data unchanged
    /// (systematic), the rest parity. Any `data.count` of the shards reconstruct the data.
    ReedSolomonEncode,
    /// `reedSolomonRecover(received, k)` — reconstruct every shard from `received` (a `Seq`
    /// of `n` slots, each a `Seq of Int` shard or `nothing` for an erased slot); `k` is the
    /// data-shard count. Succeeds when at least `k` of the `n` shards survived.
    ReedSolomonRecover,
    /// `fecProtect(payload, k, m)` — frame a flat byte `Seq of Int`, split into `k` shards, and
    /// Reed-Solomon encode into `k + m` loss-resilient packets (any `k` reconstruct the payload).
    FecProtect,
    /// `fecRestore(received, k)` — recover the payload from `received` packets (`nothing` = lost),
    /// stripping the length header + padding to return the exact original bytes.
    FecRestore,
    // --- DOM (the browser tier) — Element/Event are opaque handles. On the native
    // tiers they land in the thread-local `semantics::dom::DomRecorder`; on wasm
    // they lower to `dom_*` host imports (a `MockDom` in tests, the real DOM in the
    // browser). See the DOM frontend plan. ---
    /// `body()` — the document body (the render root), a stable Element handle.
    DomBody,
    /// `createElement(tag)` — a new element with the given tag name.
    DomCreateElement,
    /// `createTextNode(text)` — a new text node.
    DomCreateTextNode,
    /// `setText(el, text)` — set an element's text content.
    DomSetText,
    /// `setHtml(el, html)` — set an element's raw innerHTML (verbatim markup).
    DomSetHtml,
    /// `clearChildren(el)` — remove every child (the immediate-mode re-render
    /// primitive: the view clears its root, then rebuilds it).
    DomClearChildren,
    /// `onMessage(topic, handler)` — subscribe to a relay topic; the handler runs per message.
    ///
    /// ★ THE TRANSPORT WAS ALREADY BUILT. `logicaffeine_system::relay` carries
    /// `Subscribe`/`Publish`/`Event` frames over one wire protocol shared by the native server,
    /// the native client and the browser WebSocket client. `When a message arrives on "…":` was
    /// the last trigger no tier delivered, and what it was missing was this one call — not a
    /// protocol. The host carries bytes for a topic; which topic, and what to do with the payload,
    /// are compiled LOGOS.
    DomOnMessage,
    /// Retired: `moveChildren(from, to)` reparented a holding node's children for a `{children}`
    /// slot. A slot's children are a range of the retained tree's tape (`vdom.lg`), which the
    /// commit places, so no source spelling resolves to this id ([`retired_builtin`] refuses the
    /// call); the variant stays so every id after it keeps its wire position.
    DomMoveChildren,
    /// Retired: `beginPaint()` opened a host-side staging pass. A repaint keeps its nodes by
    /// drawing through the retained tree (`vdom.lg`), so no source spelling resolves to this id
    /// ([`retired_builtin`] refuses the call); the variant stays so every id after it keeps its
    /// wire position.
    DomBeginPaint,
    /// `setAttribute(el, name, value)` — set an attribute.
    DomSetAttribute,
    /// `appendChild(parent, child)` — append `child` to `parent`.
    DomAppendChild,
    /// `getText(el)` — an element's text content.
    DomGetText,
    /// `getValue(el)` — a form input's value.
    DomGetValue,
    /// `setValue(el, value)` — set a form input's value.
    DomSetValue,
    /// `applyPatches(bytes)` — apply a whole DOM patch stream in ONE call.
    ///
    /// The bulk form: a program builds a buffer of operations and hands it over once, instead of
    /// making a host call per operation. That is what lets the browser host be a `for` loop over
    /// a stream rather than fifty entry points with behaviour behind them — the stream is the
    /// interface, and every decision belongs to the side that emitted it.
    ///
    /// A malformed buffer fails LOUDLY and applies NOTHING: silently applying the prefix that
    /// happened to parse would leave a half-built page with nothing anywhere saying so.
    DomApplyPatches,
    /// `setChecked(el, on)` — set a checkbox's or radio's checked state.
    ///
    /// A checkbox's state is a PRESENCE, not a string, and it lives on the PROPERTY rather than
    /// the attribute — `setAttribute(el, "checked", …)` writes `defaultChecked` and leaves the
    /// box a visitor sees unchanged. So it gets an operation of its own rather than riding on
    /// `setValue` (which a checkbox ignores) or `setAttribute` (which lies).
    DomSetChecked,
    /// `preventDefault(node, event)` — this listener cancels the browser's own action.
    DomPreventDefault,
    /// `storageSet(area, key, value)` / `storageGet(area, key)` / `storageRemove(area, key)` —
    /// browser storage, where `area` is `"local"` (survives closing the tab) or `"session"`
    /// (survives a reload, ends with the tab).
    ///
    /// The AREA IS AN ARGUMENT rather than two families of operation, so a third area later is
    /// not a change to the host boundary; an area nobody defines is refused by name rather than
    /// written somewhere nothing reads. `storageGet` answers an `Option of Text` because a
    /// missing key and a stored empty string are different, and a page that confuses them will
    /// overwrite one with the other.
    StorageSet,
    StorageGet,
    /// `storageGetOr(area, key, fallback)` — the stored text, or `fallback` when the key is not
    /// there. The absence-answering sibling of `storageGet`, and the one a stored value is
    /// usually READ with: "what I saved, or what I start from" is the whole question a page has.
    ///
    /// It exists as its own operation rather than as sugar because an `Option of Text` is a
    /// BOXED handle on the WebAssembly tier — its presence can be tested but its value cannot
    /// yet be taken out — so a lowering that must USE a stored value goes through here, where no
    /// Option crosses the boundary at all. No new host import: it is `storage_get` plus a test
    /// of the answered handle.
    StorageGetOr,
    StorageRemove,
    /// `setStream(el, streamHandle)` — attach an opaque host `MediaStream` to an element's
    /// `srcObject`, rendering a live video/audio track in a `<video>`/`<audio>` tile.
    DomSetStream,
    /// WebRTC browser-P2P leg. `peerConnection()` mints an `RTCPeerConnection`; `dataChannel(pc,
    /// label)` opens a channel; `createOffer`/`createAnswer` produce the SDP; `setLocalDescription`/
    /// `setRemoteDescription` apply it; `addIceCandidate` adds a remote candidate; `channelSend`
    /// pushes bytes down the channel.
    RtcPeerConnection,
    RtcDataChannel,
    RtcCreateOffer,
    RtcCreateAnswer,
    RtcSetLocal,
    RtcSetRemote,
    RtcAddIce,
    RtcSend,
    /// `querySelector(sel)` — the first element matching a bare tag or `#id`
    /// (a `-1` handle if none).
    DomQuerySelector,
    /// `addEventListener(el, event, handler)` — register a LOGOS closure to run
    /// when `event` fires on `el`. The handler rides the B0 `__logos_call_closure`
    /// trampoline on wasm; on native tiers it is stored and invoked to simulate.
    DomAddEventListener,
    /// `eventValue(event)` — the value of the input that dispatched `event`.
    DomEventValue,
    /// `eventKey(event)` — the key of the keyboard event being dispatched (`""` otherwise).
    DomEventKey,
    /// `tryReadBytes(path)` — a file's bytes as `Result of Seq of Int and Text`. The total form
    /// of [`BuiltinId::ReadBytes`]: a missing or unreadable file is a value, not a trap.
    TryReadBytes,
    /// `eventTarget(event)` — the element that dispatched `event`.
    DomEventTarget,
    /// `fetchText(url, onDone)` — fetch `url`; when it resolves, run the completion
    /// closure `onDone(reqId)`, which reads the body via `responseText()`.
    DomFetchText,
    /// `responseText()` — the body of the fetch currently being delivered.
    DomResponseText,
    /// `readBytes(path)` — a file's raw bytes as a `Seq of Int` (for serving
    /// binary content like a `.wasm`). Native tiers only.
    ReadBytes,
    /// `signal(initial)` — a new reactive state cell; returns its `Int` handle.
    SignalNew,
    /// `signalSet(s, value)` — write a signal and mark the store dirty (→ re-render).
    SignalSet,
    /// `signalGetInt(s)` — a signal's current value as an `Int`.
    SignalGetInt,
    /// `signalGetText(s)` — a signal's current value as `Text`.
    SignalGetText,
    /// `beginRender()` — reset the signal hooks cursor; the view's first op on
    /// every (re-)render so its `signal(...)` calls return the same cells.
    SignalBeginRender,
    /// `signalNewText(initial)` — a signal holding `Text` (distinct wasm ABI: the
    /// initial value and writes are a `Text` handle, not an Int).
    SignalNewText,
    /// `signalSetText(s, value)` — write a `Text` signal; marks the store dirty.
    SignalSetText,
    /// `enterComponent(key)` — push a component's hook scope (its per-instance
    /// identity) so its `signal(...)` calls get their own cells.
    SignalEnterComponent,
    /// `exitComponent()` — pop back to the parent component's hook scope.
    SignalExitComponent,
    /// `signalNewSeq(initial)` — a signal holding a `Seq` (a live collection).
    /// Like [`BuiltinId::SignalNewText`] its wasm ABI is a handle (an `i32` Seq handle), the
    /// element kind riding the `SeqText` representation; a live list re-renders a
    /// `for each` view when `Push`/`Set` writes it.
    SignalNewSeq,
    /// `signalGetSeq(s)` — a signal's current value as a `Seq`.
    SignalGetSeq,
    /// `signalSetSeq(s, value)` — write a `Seq` signal; marks the store dirty.
    SignalSetSeq,
    /// `signalNewFloat(initial)` — a signal holding a `Real`.
    ///
    /// A signal's value family is settled at its declaration by the parser, which
    /// is the only place that can see the whole program: a float initial value, an
    /// explicit `Let rate: Real be a signal 2`, or a later fractional write all
    /// name this family. Saying it in the BUILTIN — exactly as `Text` and `Seq`
    /// signals already do — is what lets every function agree. A backend that had
    /// to infer float-ness from a value's kind could only see one function at a
    /// time, so a view that read a cell an event handler wrote would disagree with
    /// it and render the float's raw bits.
    SignalNewFloat,
    /// `signalGetFloat(s)` — a signal's current value as a `Real`.
    SignalGetFloat,
    /// `signalSetFloat(s, value)` — write a `Real` signal; marks the store dirty.
    SignalSetFloat,
    /// file.lg's `## To native read (path) -> Result of Text and Text` — the shared
    /// tree-walker/VM seam. NOT name-mapped in [`builtin_from_name`]: each tier
    /// routes here only when the callee is the EMPTY-bodied native declaration, so
    /// a user-defined `read` with a real body always wins (declarer-wins).
    FileRead,
    /// file.lg's `## To native write (path, content) -> Result of Unit and Text` —
    /// same routing rule as [`BuiltinId::FileRead`].
    FileWrite,
    /// random.lg's `## To native randomInt (min, max) -> Int` — the shared
    /// tree-walker/VM seam over `logicaffeine_system::random`. NOT name-mapped in
    /// [`builtin_from_name`]: each tier routes here only when the callee is the
    /// EMPTY-bodied native declaration, so a user-defined `randomInt` with a real
    /// body always wins (declarer-wins).
    RandomInt,
    /// random.lg's `## To native randomFloat () -> Real` — same routing rule as
    /// [`BuiltinId::RandomInt`].
    RandomFloat,
    // --- The correlated browser fetch contract (append-only tail; see the DOM
    // block above). `fetchSend` mints a REQUEST ID; every response accessor is
    // keyed by it, so concurrent fetches can never overwrite or misdeliver each
    // other. `fetchText`/`responseText` remain as sugar over the same engine. ---
    /// `fetchSend(method, url, body, headers, onDone) -> Int` — start a request
    /// and return its correlation id. `headers` is a `Seq of Text` alternating
    /// name,value (the flat form that crosses the wasm ABI as one seq handle);
    /// names must be on the request-header allowlist. When the response (or a
    /// structured failure) arrives the host runs `onDone(reqId)`.
    DomFetchSend,
    /// `responseStatus(reqId) -> Int` — the HTTP status; `0` while pending or on
    /// any transport failure (`responseError` then names it).
    DomResponseStatus,
    /// `responseBody(reqId) -> Text` — the delivered body (`""` pending/failed).
    DomResponseBody,
    /// `responseHeader(reqId, name) -> Text` — a response header, case-insensitive
    /// (`""` when absent).
    DomResponseHeader,
    /// `responseError(reqId) -> Text` — `""` when none; else `"timeout"` |
    /// `"abort"` | `"toolarge"` | `"network: <detail>"`.
    DomResponseError,
    /// `fetchAbort(reqId)` — the request becomes final with the `"abort"` error;
    /// a late response can never deliver.
    DomFetchAbort,
    /// `fetchSetTimeout(reqId, millis)` — arm (`> 0`) or clear (`0`) a timeout;
    /// firing delivers the `"timeout"` error and runs the callback.
    DomFetchSetTimeout,
    // --- The history/location router (append-only tail; serde-stable variant order).
    // `routePath`/`routeQuery` read `window.location`; `navigate`/`replaceRoute` are
    // history.pushState/replaceState; `onRoute` registers the popstate handler. On the
    // native tiers they land in the recorder's location model; on wasm they lower to
    // `dom_*` router host imports. The `matchRoute`/`routeParam` pattern matcher is
    // pure-LOGOS in web.lg over `routePath`. ---
    /// `routePath() -> Text` — the current location pathname (`"/"` at boot).
    DomRoutePath,
    /// `routeQuery(name) -> Text` — a query param by name (`""` when absent).
    DomRouteQuery,
    /// `navigate(path)` — history.pushState + fires the router (records the navigation,
    /// moves the current path).
    DomNavigate,
    /// `replaceRoute(path)` — history.replaceState (records a REPLACE navigation).
    DomReplaceRoute,
    /// `setTitle(t)` — the document's title. The one page-level property a page sets by name;
    /// `docs/web.md` teaches `Set the title to "Todos".`, so it was taught and unbuildable.
    DomSetTitle,
    /// `clipboardWrite(t)` — put `t` on the system clipboard.
    ///
    /// ★ THREE IMPORTS RATHER THAN ONE FAMILY, unlike the element commands. Those differ only in
    /// which single method is called on the same node; these are three different CAPABILITIES with
    /// three different permission models and three different ways to be denied — a clipboard write
    /// a browser may refuse, a file the viewer may decline, a notification that needs consent.
    /// Folding them into one `pageCommand` would name three decisions with one word.
    DomClipboardWrite,
    /// `downloadFile(content, name)` — offer `content` to the viewer as a file called `name`.
    DomDownloadFile,
    /// `notify(message, tone)` — raise a notification.
    DomNotify,
    /// `publish(topic, message)` — send a frame on a relay topic.
    ///
    /// ★ THE OUTWARD LEG ONLY. A page's own subscribers are reached in compiled LOGOS, by
    /// dispatching `message:<topic>` through the table `Raise` drives; this carries the frame that
    /// genuinely leaves the page. Splitting it that way is what keeps the deciding — who hears a
    /// publish, and when — inside the module rather than in a host nobody can test.
    DomPublish,
    /// `elementCommand(el, command)` — perform a named operation on ONE element: put the cursor in
    /// it, take the cursor out, bring it into view, select its text.
    ///
    /// ★ ONE IMPORT FOR THE FAMILY, because they differ only in which single DOM call is made and
    /// none of them decides anything. Five imports would have said the same thing five times, and
    /// the ABI is a place where saying something twice is how it drifts.
    DomElementCommand,
    /// `eventFileName(ev) -> Text` — the name of the first file a drop carried, `""` when it
    /// carried none. A READ of the drop's file (a browser's file transfer, or in process the file
    /// the host delivering the drop handed over); what the page then does about it is compiled
    /// LOGOS.
    DomEventFileName,
    /// `setInterval(ms, handler)` — the browser's clock, arming ONE repeating callback for the
    /// whole page. Which of the page's timers that makes due is decided in compiled LOGOS
    /// (`__lgx_tick`), so the arithmetic is the same on a tier with no clock at all.
    DomSetInterval,
    /// `scrollGap(el) -> Int` — how many pixels of `el`'s content sit BELOW what is visible.
    ///
    /// ★ A READING, NOT A DECISION. Whether that gap is small enough to count as "the bottom" is
    /// the page's business and stays in compiled LOGOS; the host is only asked for the three
    /// numbers a browser holds and no other tier can know. Node 0 means the page itself.
    DomScrollGap,
    /// `onWindowEvent(event, handler)` — register a handler for something that happens to the
    /// PAGE rather than to an element: going offline, coming back online, the window resizing.
    /// One builtin for all of them, because they differ only in the event's name.
    DomOnWindowEvent,
    /// `getAttribute(el, name) -> Text` — an element's attribute, `""` when it has none.
    /// The READ half of the attribute pair. It exists so `Add class`/`Remove class` can be
    /// COMPILED LOGOS over `setAttribute` rather than two more host functions: a class list is
    /// space-separated words, and deciding which words survive is the framework's job, not the
    /// host's.
    DomGetAttribute,
    /// `goBack()` — step one history entry back, as a visitor's Back button does. Answers
    /// nothing; a page at its first entry simply stays where it is.
    DomGoBack,
    /// `reload()` — the page starts over: every instance, listener and mounted root goes and the
    /// document is rebuilt. A browser does it by reloading the document; the in-process tiers do
    /// it by clearing the same two stores the reference tier clears, so the observable result is
    /// one page that has forgotten everything.
    DomReload,
    /// `onRoute(handler)` — register a popstate/navigation handler; a back/forward fires
    /// it with the new path. The handler rides the closure trampoline on wasm (like
    /// `addEventListener`/`fetchText`'s `onDone`).
    DomOnRoute,
    // ── Appended builtins: the wire format carries a builtin as its POSITION here, so new
    // builtins are added at the END only (locked by `builtin_wire_id_lock`). ──
    /// `Fail with <value>.` — stop with the value's display text as the reason. Reserved name
    /// (`__logos_fail`): the compiler writes the call, an author never does.
    Fail,
    /// A broken contract's `, otherwise <reason>` — stop with the reason, which must be Text
    /// ([`contract_reason_text`]). Reserved name (`__logos_fail_reason`).
    FailReason,
    /// `word8(n)` — the low 8 bits of an `Int` as a `Word8` (the ring ℤ/2⁸).
    Word8Make,
    /// `intOfWord8(w)` — the unsigned value of a `Word8` as an `Int` (0..255).
    IntOfWord8,
    /// A standard-library native kernel (`## To native` in `crypto.lg`): argument 0 is the kernel's
    /// [`NativeKernel::code`], the rest are its arguments. Reserved name (`__logos_native_kernel`): the
    /// VM compiler writes the call for a kernel's declaration, an author never does.
    ///
    /// [`NativeKernel::code`]: crate::semantics::native_kernels::NativeKernel::code
    NativeKernel,
    /// `noteTurn(name: Text, key: Text, changes: Seq of Text, failure: Text)` — records one turn
    /// in the journal natively, so `the journal`/`Replay` work on a lowered tier without a
    /// `## Store`-shaped mechanism to carry it: the action's journal name, the instance it ran
    /// against, every field whose display changed as `[field, before, after, …]` in declaration
    /// order, and the failure that rolled it back (`""` for a turn that committed). Reserved name
    /// (`noteTurn`): the lowering emits the call at the end of every turn, never written by an
    /// author. Browser-only: the journal it writes into is the page's turn history, which a
    /// device tier has none of.
    NoteTurn,
    /// `setHeadHtml(html)` — the document `<head>`'s markup (`<title>`/`<meta>`/…), from a page's
    /// `Head:` section, written through `semantics::dom::set_head_html`. Appended LAST, never
    /// inserted among its `Dom*` siblings above: a builtin's wire id is its position in this enum
    /// (`builtin_wire_id_lock.rs`), so a new one is always added at the END regardless of where it
    /// reads best.
    DomSetHeadHtml,
    /// The ONE place a non-exhaustive `Inspect` with no statically-known enum (a dynamic value —
    /// a kernel inductive, or a compilation with no type registry at all) fails LOUD instead of
    /// falling through to `Fail`'s generic, value-display-only message. Compiler-emitted only
    /// (`vm/compiler.rs`'s `compile_inspect`), never named from LOGOS source — it reads the ONE
    /// scrutinee register `compile_inspect` already evaluated and calls the SAME
    /// `semantics::builtins::inspect_unhandled_message` the tree-walker's own `inspect_unhandled`
    /// calls, so a dynamic non-exhaustive match fails with byte-identical words on both tiers,
    /// closing the class `Fail`'s fallback (a bare `to_display_string()` of the scrutinee, no
    /// variant/type framing) could not.
    InspectUnhandledDynamic,
    /// `eventFileSize(ev) -> Int` — the size in bytes of the first file a drop carried, its
    /// sibling `eventFileName` beside it: a `File`'s `size` is the file's own, never a literal. A
    /// browser answers from the actual event through `dom_event_file_size` (`vm/wasm/module.rs`,
    /// `runtime.js`); an in-process tier answers from the file the host delivering the drop
    /// handed over (`semantics::dom::dropped_file`).
    DomEventFileSize,
    /// `eventFileType(ev) -> Text` — the first file's MIME type, `""` when it carried none or the
    /// browser named none. Answered from the same file as `eventFileSize` beside it.
    DomEventFileType,
    /// `eventFileBytes(ev) -> Seq of Int` — the first file's content, one byte per element. A
    /// browser reads the file's `arrayBuffer()` BEFORE the drop's turn runs and hands it over
    /// through `dom_event_file_bytes`, so the turn only ever runs with the content in hand; an
    /// in-process tier answers from the same file as `eventFileSize` beside it.
    DomEventFileBytes,
    /// `listenFlags(el, event, flags)` — register `event` on `el` with the behavioural modifiers
    /// `flags` names (`semantics::dom::LISTEN_*`: prevent 1, stop 2, once 4, capture 8, passive 16),
    /// replacing any flags an earlier registration gave it. A browser attaches its listener in the
    /// capture phase or not, passive or not, stopping propagation or not; an in-process tier walks an
    /// event by the same flags (`semantics::dom::dispatch_path`). Browser-only.
    DomListenFlags,
    /// `assignLocation(url)` — load another document at `url`, as `location.assign` does, once the
    /// running script is done: then the page that asked is gone, and `url` is where the browser
    /// stands. What `Navigate to` a page outside this module's own pattern lowers to; an in-process
    /// host carries the request out after the call that made it (`semantics::dom::request_load`),
    /// recording the load (`RouteOp::Load`) and starting an empty document there. Browser-only.
    DomAssignLocation,
    /// `journalDetail() -> Bool` — whether this host keeps the journal's detail, so a lowered turn
    /// captures the fields it changed only where something reads them: every in-process host
    /// records the journal and answers true; a browser answers true only under the test harness,
    /// so a shipped page pays nothing for a journal nobody keeps. Reserved name: the lowering
    /// writes the call, an author never does. Browser-only.
    DomJournalDetail,
    /// `applyPatchWords(words, wordCount, strings, stringCount)` — apply the first `wordCount` words
    /// of a wire-v2 patch stream, whose text operands index the first `stringCount` strings, in ONE
    /// call; answers how many adopted nodes did not match. What `vdom.lg`'s `vdApply` hands over.
    /// All or nothing, as `applyPatches` is.
    DomApplyWords,
    /// `wakeAfter(ms)` — ask the host to call the page's `__lgx_tick` with the time that passed once
    /// `ms` more milliseconds have gone by, replacing any earlier request; a negative `ms` cancels
    /// it. The page keeps its own clock and decides every timer in compiled LOGOS; this only says
    /// when its earliest one comes due. Reserved name: the lowering writes the call, an author never
    /// does. Browser-only: an in-process host moves time only by `Advance time by …`.
    DomWakeAfter,
    /// `enterValue(verb, selector, value)` — a visitor's `Type` or `Choose` on the element
    /// `selector` names, as the browser's own default action performs it: whether the control
    /// takes the verb ([`crate::semantics::dom::answers_by_itself`] — a text field a visitor can
    /// reach takes typing, a `<select>` offering `value` takes the choice), and when it does, the
    /// value entered into it ([`crate::semantics::dom::enter_value`]). `verb` is `"type"` or
    /// `"choose"`. The bytecode tier's driving verbs write the call; it has no source name.
    DomEnterValue,
    /// `dispatchEntry(dispatcher, context, hook, node, event, value)` — a driving verb the program
    /// runs on its own page, taken as the HOST's entry into the page: the page's dispatcher
    /// (function `dispatcher`, `__lgx_dispatch_node`) runs as an entry of its own, so a turn that
    /// fails there is contained to it — the page's failure door hears it (the text `context`
    /// answers, then the `hook` handler, each a function index or -1 for none), the first failure
    /// no handler absorbed becomes the program's own failure once nothing is left to run, and the
    /// program's next statement runs. Answers whether the page heard the event; a failed turn was
    /// heard. The bytecode tier's driving verbs write the call; it has no source name, and the
    /// engine running the page carries it out (it calls back into the program).
    DomDispatchEntry,
    /// `Read … from file "…".`'s runtime lookup on the bytecode VM: the SAME sandboxed
    /// [`logicaffeine_system::fs::MemVfs`] the tree-walker's `ReadFrom` reads through, so a file
    /// read errors identically on both tiers. Reserved name: `Stmt::ReadFrom`'s VM compiler arm
    /// writes the call; an author never spells it.
    VfsReadFile,
    /// `Write … to file "…".`'s runtime store on the bytecode VM: the SAME sandboxed
    /// [`logicaffeine_system::fs::MemVfs`] the tree-walker's `WriteFile` writes through, so what a
    /// program wrote on either tier is what it reads back on either. Reserved name:
    /// `Stmt::WriteFile`'s VM compiler arm writes the call; an author never spells it.
    VfsWriteFile,
    /// `Mount … at "…".`'s runtime load on the bytecode VM: the journal file's text from the SAME
    /// sandboxed [`logicaffeine_system::fs::MemVfs`] the tree-walker's `Mount` reads, or empty
    /// Text when there is none yet — the tree-walker's arm exactly. Reserved name: `Stmt::Mount`'s
    /// VM compiler arm writes the call; an author never spells it.
    VfsMount,
    /// env.lg's `## To native get (key) -> Option of Text` — the variable `key` names in the
    /// environment the program runs in, `none` when it is unset, through
    /// [`logicaffeine_system::env::get`] (sealed-world aware). Same routing rule as
    /// [`BuiltinId::FileRead`]: [`system_native`] answers it only for the empty-bodied native
    /// declaration, so a program's own `get` always wins.
    EnvGet,
    /// time.lg's `## To native now () -> Nat` — the Unix time in milliseconds, through
    /// [`logicaffeine_system::time::now`] (sealed-world aware). Not the bare `now` of a moment
    /// (`Op::LoadNow`): this is the stdlib function, same routing rule as [`BuiltinId::EnvGet`].
    TimeNow,
    /// Whether the environment sets the variable `name` — half of the VM compiler's expansion of
    /// [`BuiltinId::EnvGet`] into ops every downstream tier already lowers (`mapOf`'s rule): the
    /// `Option` is then built by `NewInductive`, as any other is. Reserved name: the VM compiler
    /// writes the call; an author never spells it.
    EnvHas,
    /// The text of the environment variable `name`, `""` when it is unset — the other half of the
    /// expansion [`BuiltinId::EnvHas`] describes. Reserved name, as there.
    EnvValue,
    /// `infinitesimal(name, modulus, residue…)` — the program's infinitesimal, the residues its
    /// reciprocal may take modulo `modulus` given (`Let dt be an infinitesimal …`).
    Infinitesimal,
    /// `standardPart(x[, at])` — the standard real infinitely close to a limited `x` (`the standard
    /// part of x`); an infinite value has none. Relative to the infinitesimal `at`, what `x` is
    /// infinitely close to among the values of `at` and the infinitesimals larger than it (`the
    /// standard part of x relative to dt`).
    StandardPart,
    /// `isInfinitesimal(x[, at])` — `x` is zero or smaller than every positive standard number, or
    /// every positive value of `at` and the infinitesimals larger than it.
    IsInfinitesimal,
    /// `isLimited(x[, at])` — `x` is bounded by a standard number, or a value of `at` and the
    /// infinitesimals larger than it.
    IsLimited,
    /// `infinitelyClose(x, y[, at])` — `x − y` is infinitesimal, relative to `at` when it is given.
    InfinitelyClose,
    /// `generatorOf(dt)` — the infinite natural `⌈1/dt⌉`.
    GeneratorOf,
    /// `limitOf(x, rule)` — a residue-dependent `x` resolved under a named limit rule.
    LimitOf,
    /// `provisional(value, cases, revisions)` — the answer of a search no proof settled.
    Provisional,
    /// `atTheLimit(rule)` — the limit rule supertasks below it take (`At the limit, take …`).
    AtTheLimit,
    /// `everyNatural(cases)` — the iterable of `Repeat for every natural n`; the supertask engine
    /// replaces every such loop before a program runs.
    EveryNatural,
    /// `tripCount(t0, step, bound, relation)` — how many times a loop stepped by a clock runs.
    TripCount,
    /// `affinePower(matrix, state, n)` — the state after `n` steps of an affine update.
    AffinePower,
    /// `cycleAt(states, repeated, n)` — the state at step `n` of a run that has entered a cycle.
    CycleAt,
    /// `searchCandidates(terms, periods)` — the naturals a search over every natural examines.
    SearchCandidates,
    /// `stateSize(state)` — the bits a supertask's state occupies, which bounds its cycle search.
    StateSize,
    /// `polynomialFlow(field, state, n)` — the state after `n` Euler steps of a polynomial update.
    PolynomialFlow,
    /// `exactSqrt(x)`, `exactExp(x)`, `exactLn(x)` — the square root, exponential and natural
    /// logarithm held exactly; `sqrt`, `exp` and `ln` are these in a context that declares an exact
    /// type.
    ExactSqrt,
    ExactExp,
    ExactLn,
    /// `exactPi()`, `exactE()` — π and e held exactly; `pi` and `e` are these in an exact context.
    ExactPi,
    ExactE,
    /// `exactOmega()` — ω, the first infinite ordinal; `omega` is this in an `Ordinal` context.
    ExactOmega,
    /// `hybridFlow(model, state, n)` — the state after `n` steps of a loop whose state flows by a
    /// polynomial Euler step between jumps.
    HybridFlow,
    /// `firstExit(t0, step, guard)` — how many passes a loop makes whose guard compares terms linear
    /// in a counter it steps.
    FirstExit,
    /// `definedReal(kind, base, name, f)` — the real number the function `f` defines by its digits in
    /// `base` or by its approximations (`the real number whose digits in base 3 are f`, `the real
    /// number approximated by f`).
    DefinedReal,
    /// `exactPower(base, exponent)` — `base ** exponent` to an exponent that is an exact number, which
    /// `**` is when its exponent is one: a fraction takes a root (`2 ** (1/2)` is `√2`).
    ExactPower,
    /// `orderedInfinitesimal(name, larger, smaller, modulus, residue…)` — an infinitesimal much smaller
    /// than `larger` and much larger than `smaller`, each an infinitesimal or `nothing` (`Let dx be an
    /// infinitesimal much smaller than dt …`).
    OrderedInfinitesimal,
    /// `sealBox(key, bytes)` — XChaCha20-Poly1305 under a 24-byte nonce drawn from the system's
    /// entropy for this seal, carried in front: `nonce ‖ ciphertext ‖ tag`. One key seals any number of
    /// boxes with no nonce for the caller to keep apart. Inverse: `openBox`.
    SealBox,
    /// `openBox(key, box)` — verify the box's tag (constant-time) and decrypt it under the nonce it
    /// carries. ERRORS on a wrong key, a tampered or truncated box, or a value box.
    OpenBox,
    /// `sealValueBox(key, value)` — `sealBox` of the value's wire encoding, under its own associated data
    /// so a value box never opens as a byte box. Inverse: `openValueBox`.
    SealValueBox,
    /// `openValueBox(key, box)` — open a `sealValueBox` and wire-decode it back into the value. ERRORS
    /// on a wrong key, a tampered or truncated box, or a byte box.
    OpenValueBox,
    /// `readReals(path)` — the `Seq of Real` a file of little-endian binary64 values holds, eight bytes a value in
    /// order ([`logicaffeine_system::file::read_reals`]). ERRORS on a file it cannot read or one that is not a whole
    /// number of values. The numbers a program reads at run time rather than carries as code: a model's weights.
    ReadReals,
}

impl BuiltinId {
    /// The builtin of the exact extension this is, answered by [`logicaffeine_base::extension::call`]
    /// on every tier.
    pub fn extension(self) -> Option<logicaffeine_base::extension::Builtin> {
        use logicaffeine_base::extension::Builtin;
        Some(match self {
            BuiltinId::Infinitesimal => Builtin::Infinitesimal,
            BuiltinId::StandardPart => Builtin::StandardPart,
            BuiltinId::IsInfinitesimal => Builtin::IsInfinitesimal,
            BuiltinId::IsLimited => Builtin::IsLimited,
            BuiltinId::InfinitelyClose => Builtin::InfinitelyClose,
            BuiltinId::GeneratorOf => Builtin::GeneratorOf,
            BuiltinId::LimitOf => Builtin::LimitOf,
            BuiltinId::Provisional => Builtin::Provisional,
            BuiltinId::AtTheLimit => Builtin::AtTheLimit,
            BuiltinId::TripCount => Builtin::TripCount,
            BuiltinId::AffinePower => Builtin::AffinePower,
            BuiltinId::PolynomialFlow => Builtin::PolynomialFlow,
            BuiltinId::HybridFlow => Builtin::HybridFlow,
            BuiltinId::FirstExit => Builtin::FirstExit,
            BuiltinId::ExactPower => Builtin::ExactPower,
            BuiltinId::OrderedInfinitesimal => Builtin::OrderedInfinitesimal,
            BuiltinId::CycleAt => Builtin::CycleAt,
            BuiltinId::SearchCandidates => Builtin::SearchCandidates,
            BuiltinId::StateSize => Builtin::StateSize,
            BuiltinId::EveryNatural => Builtin::EveryNatural,
            BuiltinId::ExactSqrt => Builtin::ExactSqrt,
            BuiltinId::ExactExp => Builtin::ExactExp,
            BuiltinId::ExactLn => Builtin::ExactLn,
            BuiltinId::ExactPi => Builtin::ExactPi,
            BuiltinId::ExactE => Builtin::ExactE,
            BuiltinId::ExactOmega => Builtin::ExactOmega,
            _ => return None,
        })
    }

    /// Whether this builtin answers a value of the exact extension: a builtin of it, or `definedReal`,
    /// whose real number every tier makes from the machine values of its kind, base, name and closure.
    pub fn answers_exact(self) -> bool {
        self.extension().is_some() || matches!(self, BuiltinId::DefinedReal)
    }

    /// The width in bits of the fixed-width word this builtin returns, `arg` being the width of its
    /// first argument where the result is a word of that width (`rotr`, `word_and`, …); `None` where it
    /// returns no word. A word's arithmetic wraps at its width, so no tier holds one as an exact `Int`.
    pub fn word_result(self, arg: Option<u8>) -> Option<u8> {
        match self {
            BuiltinId::Word8Make => Some(8),
            BuiltinId::Word16Make => Some(16),
            BuiltinId::Word32 | BuiltinId::Word32Shr => Some(32),
            BuiltinId::Word64
            | BuiltinId::Word64Shl
            | BuiltinId::Word64Shr
            | BuiltinId::Word64And => Some(64),
            BuiltinId::Rotl
            | BuiltinId::Rotr
            | BuiltinId::Wnot
            | BuiltinId::Wand
            | BuiltinId::Wor => arg,
            _ => None,
        }
    }

    /// The width of the words of the sequence this builtin returns, when it returns a sequence of
    /// fixed-width words (`seqOfLanes8` unpacks eight `Word32`s).
    pub fn word_seq_result(self) -> Option<u8> {
        match self {
            BuiltinId::SeqOfLanes8 => Some(32),
            _ => None,
        }
    }

    /// Whether this builtin only exists in a BROWSER — the DOM, the reactive-signal store, and
    /// the WebRTC peer surface.
    ///
    /// A device tier has no document. Before this existed, a page compiled "successfully" to
    /// AOT-Rust, AOT-C and CUDA — 380 bytes of Rust, 121KB of C, a GPU kernel — none of which has
    /// anything to render into, so `largo build --target cuda` on a `## Page` produced an
    /// artifact that BUILT and could only ever draw nothing. The mistake then surfaced at run
    /// time on a device, a long way from the `--target` flag that caused it.
    pub fn is_browser_only(self) -> bool {
        match self {
            BuiltinId::DomBody
            | BuiltinId::DomCreateElement
            | BuiltinId::DomCreateTextNode
            | BuiltinId::DomSetText
            | BuiltinId::DomSetHtml
            | BuiltinId::DomClearChildren
            | BuiltinId::DomMoveChildren
            | BuiltinId::DomOnMessage
            | BuiltinId::DomBeginPaint
            | BuiltinId::DomSetAttribute
            | BuiltinId::DomAppendChild
            | BuiltinId::DomGetText
            | BuiltinId::DomGetValue
            | BuiltinId::DomSetValue
            | BuiltinId::DomSetChecked
            | BuiltinId::DomPreventDefault
            | BuiltinId::DomApplyPatches
            | BuiltinId::DomApplyWords
            | BuiltinId::DomWakeAfter
            | BuiltinId::DomEnterValue
            | BuiltinId::DomDispatchEntry
            | BuiltinId::StorageSet
            | BuiltinId::StorageGet
            | BuiltinId::StorageGetOr
            | BuiltinId::StorageRemove
            | BuiltinId::DomSetStream
            | BuiltinId::RtcPeerConnection
            | BuiltinId::RtcDataChannel
            | BuiltinId::RtcCreateOffer
            | BuiltinId::RtcCreateAnswer
            | BuiltinId::RtcSetLocal
            | BuiltinId::RtcSetRemote
            | BuiltinId::RtcAddIce
            | BuiltinId::RtcSend
            | BuiltinId::DomQuerySelector
            | BuiltinId::DomAddEventListener
            | BuiltinId::DomEventValue
            | BuiltinId::DomEventKey
            | BuiltinId::DomEventTarget
            | BuiltinId::DomFetchText
            | BuiltinId::DomResponseText
            | BuiltinId::SignalNew
            | BuiltinId::SignalSet
            | BuiltinId::SignalGetInt
            | BuiltinId::SignalGetText
            | BuiltinId::SignalBeginRender
            | BuiltinId::SignalNewText
            | BuiltinId::SignalSetText
            | BuiltinId::SignalEnterComponent
            | BuiltinId::SignalExitComponent
            | BuiltinId::SignalNewSeq
            | BuiltinId::SignalGetSeq
            | BuiltinId::SignalSetSeq
            | BuiltinId::SignalNewFloat
            | BuiltinId::SignalGetFloat
            | BuiltinId::SignalSetFloat
            | BuiltinId::DomFetchSend
            | BuiltinId::DomResponseStatus
            | BuiltinId::DomResponseBody
            | BuiltinId::DomResponseHeader
            | BuiltinId::DomResponseError
            | BuiltinId::DomFetchAbort
            | BuiltinId::DomFetchSetTimeout
            | BuiltinId::DomRoutePath
            | BuiltinId::DomRouteQuery
            | BuiltinId::DomNavigate
            | BuiltinId::DomReplaceRoute
            | BuiltinId::DomOnRoute
            | BuiltinId::DomSetTitle
            | BuiltinId::DomSetHeadHtml
            | BuiltinId::DomReload
            | BuiltinId::DomGoBack
            | BuiltinId::DomGetAttribute
            | BuiltinId::DomOnWindowEvent
            | BuiltinId::DomScrollGap
            | BuiltinId::DomSetInterval
            | BuiltinId::DomEventFileName
            | BuiltinId::DomEventFileSize
            | BuiltinId::DomEventFileType
            | BuiltinId::DomEventFileBytes
            | BuiltinId::DomListenFlags
            | BuiltinId::DomAssignLocation
            | BuiltinId::DomJournalDetail
            | BuiltinId::DomElementCommand
            | BuiltinId::DomClipboardWrite
            | BuiltinId::DomDownloadFile
            | BuiltinId::DomNotify
            | BuiltinId::DomPublish
            | BuiltinId::NoteTurn => true,
            _ => false,
        }
    }

    /// Every variant, for exhaustive audits — the Syntax Guide's builtin census
    /// (`crates/logicaffeine_tests/tests/guide_coverage.rs::every_builtin_has_a_guide_home`)
    /// iterates this. Append new variants here; the census's no-wildcard `match`
    /// won't compile until the new variant is classified, and its raise-only
    /// floor asserts this list never shrinks.
    pub const ALL: &'static [BuiltinId] = &[
        BuiltinId::Length,
        BuiltinId::Contains,
        BuiltinId::Format,
        BuiltinId::ParseInt,
        BuiltinId::ParseFloat,
        BuiltinId::Chr,
        BuiltinId::Abs,
        BuiltinId::Sqrt,
        BuiltinId::Min,
        BuiltinId::Max,
        BuiltinId::Floor,
        BuiltinId::Ceil,
        BuiltinId::Round,
        BuiltinId::Pow,
        BuiltinId::Decimal,
        BuiltinId::Complex,
        BuiltinId::Conjugate,
        BuiltinId::RealPart,
        BuiltinId::ImagPart,
        BuiltinId::Modular,
        BuiltinId::Quantity,
        BuiltinId::Money,
        BuiltinId::SetRate,
        BuiltinId::ToCurrency,
        BuiltinId::SetRates,
        BuiltinId::Uuid,
        BuiltinId::UuidNil,
        BuiltinId::UuidMax,
        BuiltinId::UuidVersion,
        BuiltinId::UuidDns,
        BuiltinId::UuidUrl,
        BuiltinId::UuidOid,
        BuiltinId::UuidX500,
        BuiltinId::TextBytes,
        BuiltinId::TextFromBytes,
        BuiltinId::WireBytes,
        BuiltinId::CompressBytes,
        BuiltinId::DecompressBytes,
        BuiltinId::SealBytes,
        BuiltinId::OpenBytes,
        BuiltinId::FromWireBytes,
        BuiltinId::SealFrame,
        BuiltinId::OpenFrame,
        BuiltinId::ReadWireProgram,
        BuiltinId::WriteWireResidual,
        BuiltinId::UuidBytes,
        BuiltinId::UuidFromBytes,
        BuiltinId::Convert,
        BuiltinId::ParseTimestamp,
        BuiltinId::FormatTimestamp,
        BuiltinId::YearOf,
        BuiltinId::MonthOf,
        BuiltinId::DayOf,
        BuiltinId::WeekdayOf,
        BuiltinId::HourOf,
        BuiltinId::MinuteOf,
        BuiltinId::SecondOf,
        BuiltinId::WeekOf,
        BuiltinId::QuarterOf,
        BuiltinId::DateOf,
        BuiltinId::TimeOf,
        BuiltinId::MakeDate,
        BuiltinId::MakeMoment,
        BuiltinId::LocalInstant,
        BuiltinId::SecondsBetween,
        BuiltinId::MonthsBetween,
        BuiltinId::YearsBetween,
        BuiltinId::AddSeconds,
        BuiltinId::InZone,
        BuiltinId::Copy,
        BuiltinId::CountOnes,
        BuiltinId::LeadingZeros,
        BuiltinId::TrailingZeros,
        BuiltinId::RunAccepted,
        BuiltinId::Word32,
        BuiltinId::Word64,
        BuiltinId::Rotl,
        BuiltinId::Rotr,
        BuiltinId::Wand,
        BuiltinId::Wor,
        BuiltinId::Wnot,
        BuiltinId::Lanes4Word32Make,
        BuiltinId::Lanes4Of,
        BuiltinId::SeqOfLanes4W32,
        BuiltinId::Sha1Rnds4,
        BuiltinId::Sha1Msg1,
        BuiltinId::Sha1Msg2,
        BuiltinId::Sha1Nexte,
        BuiltinId::Lanes16Word8Make,
        BuiltinId::SeqOfLanes16W8,
        BuiltinId::Splat16Word8,
        BuiltinId::Shuffle16,
        BuiltinId::ShrBytes16,
        BuiltinId::InterleaveLo16,
        BuiltinId::InterleaveHi16,
        BuiltinId::ByteAdd16,
        BuiltinId::Maddubs16,
        BuiltinId::Packus16,
        BuiltinId::Lanes8Word32,
        BuiltinId::SeqOfLanes8,
        BuiltinId::Splat8Word32,
        BuiltinId::IntOfWord32,
        BuiltinId::IntOfWord64,
        BuiltinId::Word64Shl,
        BuiltinId::Word64Shr,
        BuiltinId::Word64And,
        BuiltinId::Word32Shr,
        BuiltinId::Word16Make,
        BuiltinId::IntOfWord16,
        BuiltinId::Lanes4Word64,
        BuiltinId::SeqOfLanes4,
        BuiltinId::Mul32x32To64,
        BuiltinId::HsumLanes4,
        BuiltinId::Splat4Word64,
        BuiltinId::AndNot4,
        BuiltinId::Lanes16Word16,
        BuiltinId::SeqOfLanes16,
        BuiltinId::Splat16Word16,
        BuiltinId::Mulhi16,
        BuiltinId::Montmul32,
        BuiltinId::NttBcastLo,
        BuiltinId::NttBcastHi,
        BuiltinId::NttBlend,
        BuiltinId::MapOf,
        BuiltinId::SetOf,
        BuiltinId::RepeatSeq,
        BuiltinId::ReedSolomonEncode,
        BuiltinId::ReedSolomonRecover,
        BuiltinId::FecProtect,
        BuiltinId::FecRestore,
        BuiltinId::DomBody,
        BuiltinId::DomCreateElement,
        BuiltinId::DomCreateTextNode,
        BuiltinId::DomSetText,
        BuiltinId::DomSetHtml,
        BuiltinId::DomClearChildren,
        BuiltinId::DomMoveChildren,
        BuiltinId::DomOnMessage,
        BuiltinId::DomBeginPaint,
        BuiltinId::DomSetAttribute,
        BuiltinId::DomAppendChild,
        BuiltinId::DomGetText,
        BuiltinId::DomGetValue,
        BuiltinId::DomSetValue,
        BuiltinId::DomSetChecked,
        BuiltinId::DomPreventDefault,
        BuiltinId::DomApplyPatches,
        BuiltinId::StorageSet,
        BuiltinId::StorageGet,
        BuiltinId::StorageGetOr,
        BuiltinId::StorageRemove,
        BuiltinId::DomSetStream,
        BuiltinId::RtcPeerConnection,
        BuiltinId::RtcDataChannel,
        BuiltinId::RtcCreateOffer,
        BuiltinId::RtcCreateAnswer,
        BuiltinId::RtcSetLocal,
        BuiltinId::RtcSetRemote,
        BuiltinId::RtcAddIce,
        BuiltinId::RtcSend,
        BuiltinId::DomQuerySelector,
        BuiltinId::DomAddEventListener,
        BuiltinId::DomEventValue,
        BuiltinId::DomEventKey,
        BuiltinId::DomEventTarget,
        BuiltinId::DomFetchText,
        BuiltinId::DomResponseText,
        BuiltinId::ReadBytes,
        BuiltinId::TryReadBytes,
        BuiltinId::SignalNew,
        BuiltinId::SignalSet,
        BuiltinId::SignalGetInt,
        BuiltinId::SignalGetText,
        BuiltinId::SignalBeginRender,
        BuiltinId::SignalNewText,
        BuiltinId::SignalSetText,
        BuiltinId::SignalEnterComponent,
        BuiltinId::SignalExitComponent,
        BuiltinId::SignalNewSeq,
        BuiltinId::SignalGetSeq,
        BuiltinId::SignalSetSeq,
        BuiltinId::SignalNewFloat,
        BuiltinId::SignalGetFloat,
        BuiltinId::SignalSetFloat,
        BuiltinId::FileRead,
        BuiltinId::FileWrite,
        BuiltinId::RandomInt,
        BuiltinId::RandomFloat,
        BuiltinId::DomFetchSend,
        BuiltinId::DomResponseStatus,
        BuiltinId::DomResponseBody,
        BuiltinId::DomResponseHeader,
        BuiltinId::DomResponseError,
        BuiltinId::DomFetchAbort,
        BuiltinId::DomFetchSetTimeout,
        BuiltinId::DomRoutePath,
        BuiltinId::DomRouteQuery,
        BuiltinId::DomNavigate,
        BuiltinId::DomReplaceRoute,
        BuiltinId::DomOnRoute,
        BuiltinId::DomSetTitle,
        BuiltinId::DomSetHeadHtml,
        BuiltinId::DomReload,
        BuiltinId::DomGoBack,
        BuiltinId::DomGetAttribute,
        BuiltinId::DomOnWindowEvent,
        BuiltinId::DomScrollGap,
        BuiltinId::DomSetInterval,
        BuiltinId::DomEventFileName,
        BuiltinId::DomElementCommand,
        BuiltinId::DomClipboardWrite,
        BuiltinId::DomDownloadFile,
        BuiltinId::DomNotify,
        BuiltinId::DomPublish,
        BuiltinId::Fail,
        BuiltinId::FailReason,
        BuiltinId::Word8Make,
        BuiltinId::IntOfWord8,
        BuiltinId::NativeKernel,
        BuiltinId::NoteTurn,
        BuiltinId::InspectUnhandledDynamic,
        BuiltinId::DomEventFileSize,
        BuiltinId::DomEventFileType,
        BuiltinId::DomEventFileBytes,
        BuiltinId::DomListenFlags,
        BuiltinId::DomAssignLocation,
        BuiltinId::DomJournalDetail,
        BuiltinId::DomApplyWords,
        BuiltinId::DomWakeAfter,
        BuiltinId::DomEnterValue,
        BuiltinId::DomDispatchEntry,
        BuiltinId::VfsReadFile,
        BuiltinId::VfsWriteFile,
        BuiltinId::VfsMount,
        BuiltinId::EnvGet,
        BuiltinId::TimeNow,
        BuiltinId::EnvHas,
        BuiltinId::EnvValue,
        BuiltinId::Infinitesimal,
        BuiltinId::StandardPart,
        BuiltinId::IsInfinitesimal,
        BuiltinId::IsLimited,
        BuiltinId::InfinitelyClose,
        BuiltinId::GeneratorOf,
        BuiltinId::LimitOf,
        BuiltinId::Provisional,
        BuiltinId::AtTheLimit,
        BuiltinId::EveryNatural,
        BuiltinId::TripCount,
        BuiltinId::AffinePower,
        BuiltinId::CycleAt,
        BuiltinId::SearchCandidates,
        BuiltinId::StateSize,
        BuiltinId::PolynomialFlow,
        BuiltinId::ExactSqrt,
        BuiltinId::ExactExp,
        BuiltinId::ExactLn,
        BuiltinId::ExactPi,
        BuiltinId::ExactE,
        BuiltinId::ExactOmega,
        BuiltinId::HybridFlow,
        BuiltinId::FirstExit,
        BuiltinId::DefinedReal,
        BuiltinId::ExactPower,
        BuiltinId::OrderedInfinitesimal,
        BuiltinId::SealBox,
        BuiltinId::OpenBox,
        BuiltinId::SealValueBox,
        BuiltinId::OpenValueBox,
        BuiltinId::ReadReals,
    ];
}

/// What a call to `beginPaint` is refused with, on every tier.
pub const BEGIN_PAINT_RETIRED: &str = "`beginPaint` is gone: a repaint keeps its nodes by drawing through the retained tree — `vdOpen`/`vdText`/`vdClose`, then `vdCommit` and `vdApply` — which an LGX component's view does for you.";

/// What a call to `moveChildren` is refused with, on every tier.
pub const MOVE_CHILDREN_RETIRED: &str = "`moveChildren` is gone: a caller's children are drawn where the caller is, as a range of the retained tree's tape (`vdRangeOpen`/`vdRangeClose`), and placed where the callee says `{children}` (`vdSlot`) — which an LGX component's view does for you.";

/// Every builtin the language no longer has: its id, which keeps its wire position, the spelling a
/// program called it by, and the sentence that call is refused in.
pub const RETIRED_BUILTINS: &[(BuiltinId, &str, &str)] = &[
    (BuiltinId::DomBeginPaint, "beginPaint", BEGIN_PAINT_RETIRED),
    (
        BuiltinId::DomMoveChildren,
        "moveChildren",
        MOVE_CHILDREN_RETIRED,
    ),
];

/// The sentence a call to a builtin the language no longer has is refused in, before the program
/// runs; `None` for every other name. A program that defines a function of the same name calls its
/// own.
pub fn retired_builtin(name: &str) -> Option<&'static str> {
    RETIRED_BUILTINS
        .iter()
        .find(|(_, spelling, _)| *spelling == name)
        .map(|(_, _, why)| *why)
}

/// Resolve a function name to a builtin, if it is one.
pub fn builtin_from_name(name: &str) -> Option<BuiltinId> {
    Some(match name {
        "length" => BuiltinId::Length,
        "contains" => BuiltinId::Contains,
        "format" => BuiltinId::Format,
        "mapOf" => BuiltinId::MapOf,
        "setOf" => BuiltinId::SetOf,
        "repeatSeq" => BuiltinId::RepeatSeq,
        "reedSolomonEncode" => BuiltinId::ReedSolomonEncode,
        "reedSolomonRecover" => BuiltinId::ReedSolomonRecover,
        "fecProtect" => BuiltinId::FecProtect,
        "fecRestore" => BuiltinId::FecRestore,
        "body" => BuiltinId::DomBody,
        "createElement" => BuiltinId::DomCreateElement,
        "createTextNode" => BuiltinId::DomCreateTextNode,
        "setText" => BuiltinId::DomSetText,
        "setHtml" => BuiltinId::DomSetHtml,
        "clearChildren" => BuiltinId::DomClearChildren,
        "onMessage" => BuiltinId::DomOnMessage,
        "setAttribute" => BuiltinId::DomSetAttribute,
        "appendChild" => BuiltinId::DomAppendChild,
        "getText" => BuiltinId::DomGetText,
        "getValue" => BuiltinId::DomGetValue,
        "setValue" => BuiltinId::DomSetValue,
        "setChecked" => BuiltinId::DomSetChecked,
        "preventDefault" => BuiltinId::DomPreventDefault,
        "applyPatches" => BuiltinId::DomApplyPatches,
        "applyPatchWords" => BuiltinId::DomApplyWords,
        "wakeAfter" => BuiltinId::DomWakeAfter,
        "storageSet" => BuiltinId::StorageSet,
        "storageGet" => BuiltinId::StorageGet,
        "storageGetOr" => BuiltinId::StorageGetOr,
        "storageRemove" => BuiltinId::StorageRemove,
        "setStream" => BuiltinId::DomSetStream,
        "peerConnection" => BuiltinId::RtcPeerConnection,
        "dataChannel" => BuiltinId::RtcDataChannel,
        "createOffer" => BuiltinId::RtcCreateOffer,
        "createAnswer" => BuiltinId::RtcCreateAnswer,
        "setLocalDescription" => BuiltinId::RtcSetLocal,
        "setRemoteDescription" => BuiltinId::RtcSetRemote,
        "addIceCandidate" => BuiltinId::RtcAddIce,
        "channelSend" => BuiltinId::RtcSend,
        "querySelector" => BuiltinId::DomQuerySelector,
        "addEventListener" => BuiltinId::DomAddEventListener,
        "eventValue" => BuiltinId::DomEventValue,
        "eventKey" => BuiltinId::DomEventKey,
        "eventTarget" => BuiltinId::DomEventTarget,
        "fetchText" => BuiltinId::DomFetchText,
        "responseText" => BuiltinId::DomResponseText,
        "fetchSend" => BuiltinId::DomFetchSend,
        "responseStatus" => BuiltinId::DomResponseStatus,
        "responseBody" => BuiltinId::DomResponseBody,
        "responseHeader" => BuiltinId::DomResponseHeader,
        "responseError" => BuiltinId::DomResponseError,
        "fetchAbort" => BuiltinId::DomFetchAbort,
        "fetchSetTimeout" => BuiltinId::DomFetchSetTimeout,
        "routePath" => BuiltinId::DomRoutePath,
        "routeQuery" => BuiltinId::DomRouteQuery,
        "navigate" => BuiltinId::DomNavigate,
        "replaceRoute" => BuiltinId::DomReplaceRoute,
        "onRoute" => BuiltinId::DomOnRoute,
        "setTitle" => BuiltinId::DomSetTitle,
        "setHeadHtml" => BuiltinId::DomSetHeadHtml,
        "reloadPage" => BuiltinId::DomReload,
        "goBack" => BuiltinId::DomGoBack,
        "getAttribute" => BuiltinId::DomGetAttribute,
        "onWindowEvent" => BuiltinId::DomOnWindowEvent,
        "scrollGap" => BuiltinId::DomScrollGap,
        "setInterval" => BuiltinId::DomSetInterval,
        "eventFileName" => BuiltinId::DomEventFileName,
        "eventFileSize" => BuiltinId::DomEventFileSize,
        "eventFileType" => BuiltinId::DomEventFileType,
        "eventFileBytes" => BuiltinId::DomEventFileBytes,
        "listenFlags" => BuiltinId::DomListenFlags,
        "assignLocation" => BuiltinId::DomAssignLocation,
        "journalDetail" => BuiltinId::DomJournalDetail,
        "elementCommand" => BuiltinId::DomElementCommand,
        "clipboardWrite" => BuiltinId::DomClipboardWrite,
        "downloadFile" => BuiltinId::DomDownloadFile,
        "notify" => BuiltinId::DomNotify,
        "publish" => BuiltinId::DomPublish,
        "readBytes" => BuiltinId::ReadBytes,
        "tryReadBytes" => BuiltinId::TryReadBytes,
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
        "conjugate" => BuiltinId::Conjugate,
        "infinitesimal" => BuiltinId::Infinitesimal,
        "standardPart" => BuiltinId::StandardPart,
        "isInfinitesimal" => BuiltinId::IsInfinitesimal,
        "isLimited" => BuiltinId::IsLimited,
        "infinitelyClose" => BuiltinId::InfinitelyClose,
        "generatorOf" => BuiltinId::GeneratorOf,
        "limitOf" => BuiltinId::LimitOf,
        "provisional" => BuiltinId::Provisional,
        "atTheLimit" => BuiltinId::AtTheLimit,
        "everyNatural" => BuiltinId::EveryNatural,
        "exactSqrt" => BuiltinId::ExactSqrt,
        "exactExp" => BuiltinId::ExactExp,
        "exactLn" => BuiltinId::ExactLn,
        "exactPi" => BuiltinId::ExactPi,
        "exactE" => BuiltinId::ExactE,
        "exactOmega" => BuiltinId::ExactOmega,
        "tripCount" => BuiltinId::TripCount,
        "affinePower" => BuiltinId::AffinePower,
        "cycleAt" => BuiltinId::CycleAt,
        "searchCandidates" => BuiltinId::SearchCandidates,
        "stateSize" => BuiltinId::StateSize,
        "polynomialFlow" => BuiltinId::PolynomialFlow,
        "hybridFlow" => BuiltinId::HybridFlow,
        "firstExit" => BuiltinId::FirstExit,
        "exactPower" => BuiltinId::ExactPower,
        "orderedInfinitesimal" => BuiltinId::OrderedInfinitesimal,
        "definedReal" => BuiltinId::DefinedReal,
        "re" => BuiltinId::RealPart,
        "im" => BuiltinId::ImagPart,
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
        "compressBytes" => BuiltinId::CompressBytes,
        "decompressBytes" => BuiltinId::DecompressBytes,
        "sealBytes" => BuiltinId::SealBytes,
        "openBytes" => BuiltinId::OpenBytes,
        "sealBox" => BuiltinId::SealBox,
        "openBox" => BuiltinId::OpenBox,
        "sealValueBox" => BuiltinId::SealValueBox,
        "openValueBox" => BuiltinId::OpenValueBox,
        "readReals" => BuiltinId::ReadReals,
        "fromWireBytes" => BuiltinId::FromWireBytes,
        "sealFrame" => BuiltinId::SealFrame,
        "openFrame" => BuiltinId::OpenFrame,
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
        "makeDate" => BuiltinId::MakeDate,
        "makeMoment" => BuiltinId::MakeMoment,
        "local_instant" => BuiltinId::LocalInstant,
        "seconds_between" => BuiltinId::SecondsBetween,
        "months_between" => BuiltinId::MonthsBetween,
        "years_between" => BuiltinId::YearsBetween,
        "add_seconds" => BuiltinId::AddSeconds,
        "in_zone" => BuiltinId::InZone,
        "copy" => BuiltinId::Copy,
        "__logos_fail" => BuiltinId::Fail,
        "__logos_fail_reason" => BuiltinId::FailReason,
        "count_ones" => BuiltinId::CountOnes,
        "leading_zeros" => BuiltinId::LeadingZeros,
        "trailing_zeros" => BuiltinId::TrailingZeros,
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
        "word8" => BuiltinId::Word8Make,
        "intOfWord8" => BuiltinId::IntOfWord8,
        "__logos_native_kernel" => BuiltinId::NativeKernel,
        "noteTurn" => BuiltinId::NoteTurn,
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
        // The kernel code, then the kernel's own arguments — the kernel checks its arity.
        BuiltinId::NativeKernel => {
            if n == 0 {
                return Err("a native kernel call carries its kernel code".to_string());
            }
            return Ok(());
        }
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
        BuiltinId::Infinitesimal => {
            if n < 2 {
                return Err("infinitesimal takes a name, a modulus and the allowed residues".to_string());
            }
            return Ok(());
        }
        BuiltinId::OrderedInfinitesimal => {
            if n < 4 {
                return Err(
                    "orderedInfinitesimal takes a name, the infinitesimals above and below, a modulus and the allowed residues"
                        .to_string(),
                );
            }
            return Ok(());
        }
        BuiltinId::StandardPart | BuiltinId::IsInfinitesimal | BuiltinId::IsLimited => {
            if !(1..=2).contains(&n) {
                let name = id.extension().expect("a builtin of the exact extension").name();
                return Err(format!(
                    "{name} takes a value and, relative to an infinitesimal, that infinitesimal; got {n} arguments"
                ));
            }
            return Ok(());
        }
        BuiltinId::InfinitelyClose => {
            if !(2..=3).contains(&n) {
                return Err(format!(
                    "infinitelyClose takes two values and, relative to an infinitesimal, that infinitesimal; got {n} arguments"
                ));
            }
            return Ok(());
        }
        BuiltinId::LimitOf | BuiltinId::SearchCandidates | BuiltinId::ExactPower => 2,
        BuiltinId::Provisional
        | BuiltinId::AffinePower
        | BuiltinId::CycleAt
        | BuiltinId::PolynomialFlow
        | BuiltinId::HybridFlow
        | BuiltinId::FirstExit => 3,
        BuiltinId::TripCount | BuiltinId::DefinedReal => 4,
        BuiltinId::ExactPi | BuiltinId::ExactE | BuiltinId::ExactOmega => 0,
        BuiltinId::Min | BuiltinId::Max | BuiltinId::Pow => 2,
        BuiltinId::Contains => 2,
        BuiltinId::RepeatSeq => 2,
        BuiltinId::ReedSolomonEncode | BuiltinId::ReedSolomonRecover => 2,
        BuiltinId::FecRestore => 2,
        BuiltinId::FecProtect => 3,
        BuiltinId::SealBytes | BuiltinId::OpenBytes => 3,
        BuiltinId::SealBox
        | BuiltinId::OpenBox
        | BuiltinId::SealValueBox
        | BuiltinId::OpenValueBox => 2,
        BuiltinId::SealFrame | BuiltinId::OpenFrame => 3,
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
        BuiltinId::TextBytes | BuiltinId::TextFromBytes | BuiltinId::WireBytes | BuiltinId::FromWireBytes | BuiltinId::CompressBytes | BuiltinId::DecompressBytes | BuiltinId::WriteWireResidual | BuiltinId::UuidBytes | BuiltinId::UuidFromBytes => 1,
        BuiltinId::SecondsBetween | BuiltinId::AddSeconds | BuiltinId::InZone => 2,
        BuiltinId::MonthsBetween | BuiltinId::YearsBetween => 2,
        BuiltinId::MakeDate => 3,
        BuiltinId::MakeMoment => 6,
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
        BuiltinId::DomBody => 0,
        BuiltinId::DomCreateElement
        | BuiltinId::DomCreateTextNode
        | BuiltinId::DomGetText
        | BuiltinId::DomGetValue
        | BuiltinId::DomClearChildren
        | BuiltinId::DomApplyPatches
        | BuiltinId::DomWakeAfter
        | BuiltinId::DomQuerySelector => 1,
        BuiltinId::DomApplyWords => 4,
        BuiltinId::DomSetText
        | BuiltinId::DomSetHtml
        | BuiltinId::DomMoveChildren
        | BuiltinId::DomOnMessage
        | BuiltinId::DomAppendChild
        | BuiltinId::DomSetValue
        | BuiltinId::DomSetChecked
        // `preventDefault(element, event)` — the element the listener is on, and which event's
        // default to suppress. Omitting it here left the fall-through default of ONE, so every
        // caller of the `.prevent` lowering was refused before it ran: the VM said so by name and
        // the wasm module trapped at mount. See `prevent_default_arity_lock`.
        | BuiltinId::DomPreventDefault
        | BuiltinId::DomSetStream => 2,
        BuiltinId::RtcPeerConnection => 0,
        BuiltinId::RtcCreateOffer | BuiltinId::RtcCreateAnswer => 1,
        BuiltinId::RtcDataChannel
        | BuiltinId::RtcSetLocal
        | BuiltinId::RtcSetRemote
        | BuiltinId::RtcAddIce
        | BuiltinId::RtcSend => 2,
        BuiltinId::StorageGet | BuiltinId::StorageRemove => 2,
        BuiltinId::StorageSet | BuiltinId::StorageGetOr => 3,
        BuiltinId::DomSetAttribute | BuiltinId::DomAddEventListener | BuiltinId::DomListenFlags | BuiltinId::DomEnterValue => 3,
        BuiltinId::DomDispatchEntry => 6,
        BuiltinId::DomGetAttribute
        | BuiltinId::DomOnWindowEvent
        | BuiltinId::DomElementCommand => 2,
        BuiltinId::DomScrollGap
        | BuiltinId::DomEventFileName
        | BuiltinId::DomClipboardWrite => 1,
        BuiltinId::DomDownloadFile | BuiltinId::DomNotify | BuiltinId::DomPublish => 2,
        BuiltinId::NoteTurn => 4,
        BuiltinId::DomSetInterval => 2,
        BuiltinId::DomEventValue | BuiltinId::DomEventKey | BuiltinId::DomEventTarget => 1,
        BuiltinId::DomFetchText => 2,
        BuiltinId::DomResponseText | BuiltinId::DomBeginPaint => 0,
        BuiltinId::DomFetchSend => 5,
        BuiltinId::DomResponseStatus
        | BuiltinId::DomResponseBody
        | BuiltinId::DomResponseError
        | BuiltinId::DomFetchAbort => 1,
        BuiltinId::DomResponseHeader | BuiltinId::DomFetchSetTimeout => 2,
        BuiltinId::DomRoutePath | BuiltinId::DomReload | BuiltinId::DomGoBack | BuiltinId::DomJournalDetail => 0,
        BuiltinId::DomRouteQuery
        | BuiltinId::DomNavigate
        | BuiltinId::DomReplaceRoute
        | BuiltinId::DomOnRoute
        | BuiltinId::DomSetTitle
        | BuiltinId::DomSetHeadHtml => 1,
        BuiltinId::ReadBytes | BuiltinId::TryReadBytes | BuiltinId::ReadReals => 1,
        BuiltinId::FileRead => 1,
        BuiltinId::FileWrite => 2,
        BuiltinId::VfsWriteFile => 2,
        BuiltinId::RandomInt => 2,
        BuiltinId::RandomFloat => 0,
        BuiltinId::EnvGet | BuiltinId::EnvHas | BuiltinId::EnvValue => 1,
        BuiltinId::TimeNow => 0,
        // The reactive-signal dialect is retired (see `execute_builtin`); wire-frozen, kept exact
        // so an old encoded program's genuinely-correct call is never misreported as wrong-arity.
        BuiltinId::SignalNew | BuiltinId::SignalGetInt | BuiltinId::SignalGetText | BuiltinId::SignalNewText => 1,
        BuiltinId::SignalSet | BuiltinId::SignalSetText => 2,
        BuiltinId::SignalBeginRender | BuiltinId::SignalExitComponent => 0,
        BuiltinId::SignalEnterComponent => 1,
        BuiltinId::SignalNewSeq | BuiltinId::SignalGetSeq => 1,
        BuiltinId::SignalSetSeq => 2,
        BuiltinId::SignalNewFloat | BuiltinId::SignalGetFloat => 1,
        BuiltinId::SignalSetFloat => 2,
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
            BuiltinId::Contains => "contains",
            BuiltinId::Format => unreachable!(),
            // MapOf/SetOf return early above (variadic with their own errors).
            BuiltinId::MapOf | BuiltinId::SetOf => unreachable!(),
            BuiltinId::RepeatSeq => "repeatSeq",
            BuiltinId::ReedSolomonEncode => "reedSolomonEncode",
            BuiltinId::ReedSolomonRecover => "reedSolomonRecover",
            BuiltinId::FecProtect => "fecProtect",
            BuiltinId::FecRestore => "fecRestore",
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
            BuiltinId::Conjugate => "conjugate",
            BuiltinId::Infinitesimal => "infinitesimal",
            BuiltinId::StandardPart => "standardPart",
            BuiltinId::IsInfinitesimal => "isInfinitesimal",
            BuiltinId::IsLimited => "isLimited",
            BuiltinId::InfinitelyClose => "infinitelyClose",
            BuiltinId::GeneratorOf => "generatorOf",
            BuiltinId::LimitOf => "limitOf",
            BuiltinId::Provisional => "provisional",
            BuiltinId::AtTheLimit => "atTheLimit",
            BuiltinId::EveryNatural => "everyNatural",
            BuiltinId::ExactSqrt => "exactSqrt",
            BuiltinId::ExactExp => "exactExp",
            BuiltinId::ExactLn => "exactLn",
            BuiltinId::ExactPi => "exactPi",
            BuiltinId::ExactE => "exactE",
            BuiltinId::ExactOmega => "exactOmega",
            BuiltinId::TripCount => "tripCount",
            BuiltinId::AffinePower => "affinePower",
            BuiltinId::CycleAt => "cycleAt",
            BuiltinId::SearchCandidates => "searchCandidates",
            BuiltinId::StateSize => "stateSize",
            BuiltinId::PolynomialFlow => "polynomialFlow",
            BuiltinId::HybridFlow => "hybridFlow",
            BuiltinId::FirstExit => "firstExit",
            BuiltinId::ExactPower => "exactPower",
            BuiltinId::OrderedInfinitesimal => "orderedInfinitesimal",
            BuiltinId::DefinedReal => "definedReal",
            BuiltinId::RealPart => "re",
            BuiltinId::ImagPart => "im",
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
            BuiltinId::CompressBytes => "compressBytes",
            BuiltinId::DecompressBytes => "decompressBytes",
            BuiltinId::SealBytes => "sealBytes",
            BuiltinId::OpenBytes => "openBytes",
            BuiltinId::SealBox => "sealBox",
            BuiltinId::OpenBox => "openBox",
            BuiltinId::SealValueBox => "sealValueBox",
            BuiltinId::OpenValueBox => "openValueBox",
            BuiltinId::ReadReals => "readReals",
            BuiltinId::FromWireBytes => "fromWireBytes",
            BuiltinId::SealFrame => "sealFrame",
            BuiltinId::OpenFrame => "openFrame",
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
            BuiltinId::MakeDate => "makeDate",
            BuiltinId::MakeMoment => "makeMoment",
            BuiltinId::LocalInstant => "local_instant",
            BuiltinId::SecondsBetween => "seconds_between",
            BuiltinId::MonthsBetween => "months_between",
            BuiltinId::YearsBetween => "years_between",
            BuiltinId::AddSeconds => "add_seconds",
            BuiltinId::InZone => "in_zone",
            BuiltinId::Copy => "copy",
            BuiltinId::Fail => "__logos_fail",
            BuiltinId::FailReason => "__logos_fail_reason",
            BuiltinId::CountOnes => "count_ones",
            BuiltinId::LeadingZeros => "leading_zeros",
            BuiltinId::TrailingZeros => "trailing_zeros",
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
            BuiltinId::Word8Make => "word8",
            BuiltinId::IntOfWord8 => "intOfWord8",
            BuiltinId::NativeKernel => "__logos_native_kernel",
            BuiltinId::NoteTurn => "noteTurn",
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
            BuiltinId::DomBody => "body",
            BuiltinId::DomCreateElement => "createElement",
            BuiltinId::DomCreateTextNode => "createTextNode",
            BuiltinId::DomSetText => "setText",
            BuiltinId::DomSetHtml => "setHtml",
            BuiltinId::DomClearChildren => "clearChildren",
            BuiltinId::DomMoveChildren => "moveChildren",
            BuiltinId::DomOnMessage => "onMessage",
            BuiltinId::DomBeginPaint => "beginPaint",
            BuiltinId::DomSetAttribute => "setAttribute",
            BuiltinId::DomAppendChild => "appendChild",
            BuiltinId::DomGetText => "getText",
            BuiltinId::DomGetValue => "getValue",
            BuiltinId::DomSetValue => "setValue",
            BuiltinId::DomSetChecked => "setChecked",
            BuiltinId::DomPreventDefault => "preventDefault",
            BuiltinId::DomApplyPatches => "applyPatches",
            BuiltinId::DomApplyWords => "applyPatchWords",
            BuiltinId::DomWakeAfter => "wakeAfter",
            BuiltinId::DomEnterValue => "enterValue",
            BuiltinId::DomDispatchEntry => "dispatchEntry",
            BuiltinId::StorageSet => "storageSet",
            BuiltinId::StorageGet => "storageGet",
            BuiltinId::StorageGetOr => "storageGetOr",
            BuiltinId::StorageRemove => "storageRemove",
            BuiltinId::DomSetStream => "setStream",
            BuiltinId::RtcPeerConnection => "peerConnection",
            BuiltinId::RtcDataChannel => "dataChannel",
            BuiltinId::RtcCreateOffer => "createOffer",
            BuiltinId::RtcCreateAnswer => "createAnswer",
            BuiltinId::RtcSetLocal => "setLocalDescription",
            BuiltinId::RtcSetRemote => "setRemoteDescription",
            BuiltinId::RtcAddIce => "addIceCandidate",
            BuiltinId::RtcSend => "channelSend",
            BuiltinId::DomQuerySelector => "querySelector",
            BuiltinId::DomAddEventListener => "addEventListener",
            BuiltinId::DomEventValue => "eventValue",
            BuiltinId::DomEventKey => "eventKey",
            BuiltinId::DomEventTarget => "eventTarget",
            BuiltinId::DomFetchText => "fetchText",
            BuiltinId::DomResponseText => "responseText",
            BuiltinId::DomFetchSend => "fetchSend",
            BuiltinId::DomResponseStatus => "responseStatus",
            BuiltinId::DomResponseBody => "responseBody",
            BuiltinId::DomResponseHeader => "responseHeader",
            BuiltinId::DomResponseError => "responseError",
            BuiltinId::DomFetchAbort => "fetchAbort",
            BuiltinId::DomFetchSetTimeout => "fetchSetTimeout",
            BuiltinId::DomRoutePath => "routePath",
            BuiltinId::DomRouteQuery => "routeQuery",
            BuiltinId::DomNavigate => "navigate",
            BuiltinId::DomReplaceRoute => "replaceRoute",
            BuiltinId::DomOnRoute => "onRoute",
            BuiltinId::DomSetTitle => "setTitle",
            BuiltinId::DomSetHeadHtml => "setHeadHtml",
            BuiltinId::DomReload => "reloadPage",
            BuiltinId::DomGoBack => "goBack",
            BuiltinId::DomGetAttribute => "getAttribute",
            BuiltinId::DomOnWindowEvent => "onWindowEvent",
            BuiltinId::DomScrollGap => "scrollGap",
            BuiltinId::DomSetInterval => "setInterval",
            BuiltinId::DomEventFileName => "eventFileName",
            BuiltinId::DomElementCommand => "elementCommand",
            BuiltinId::DomClipboardWrite => "clipboardWrite",
            BuiltinId::DomDownloadFile => "downloadFile",
            BuiltinId::DomNotify => "notify",
            BuiltinId::DomPublish => "publish",
            BuiltinId::ReadBytes => "readBytes",
            BuiltinId::TryReadBytes => "tryReadBytes",
            BuiltinId::FileRead => "read",
            BuiltinId::FileWrite => "write",
            BuiltinId::VfsReadFile => "__logos_read_from_file",
            BuiltinId::VfsWriteFile => "__logos_write_to_file",
            BuiltinId::VfsMount => "__logos_mount_at",
            BuiltinId::RandomInt => "randomInt",
            BuiltinId::RandomFloat => "randomFloat",
            BuiltinId::EnvGet => "get",
            BuiltinId::TimeNow => "now",
            BuiltinId::EnvHas => "__logos_env_has",
            BuiltinId::EnvValue => "__logos_env_value",
            // The reactive-signal dialect is retired (see `execute_builtin`); these ids are
            // wire-frozen and unreachable from any source a parser emits today, but an old
            // encoded program's wrong-arity call must still name itself in the error.
            BuiltinId::SignalNew => "signal",
            BuiltinId::SignalSet => "signalSet",
            BuiltinId::SignalGetInt => "signalGetInt",
            BuiltinId::SignalGetText => "signalGetText",
            BuiltinId::SignalBeginRender => "beginRender",
            BuiltinId::SignalNewText => "signalNewText",
            BuiltinId::SignalSetText => "signalSetText",
            BuiltinId::SignalEnterComponent => "enterComponent",
            BuiltinId::SignalExitComponent => "exitComponent",
            BuiltinId::SignalNewSeq => "signalNewSeq",
            BuiltinId::SignalGetSeq => "signalGetSeq",
            BuiltinId::SignalSetSeq => "signalSetSeq",
            BuiltinId::SignalNewFloat => "signalNewFloat",
            BuiltinId::SignalGetFloat => "signalGetFloat",
            BuiltinId::SignalSetFloat => "signalSetFloat",
            BuiltinId::InspectUnhandledDynamic => "__logos_inspect_unhandled",
            BuiltinId::DomEventFileSize => "eventFileSize",
            BuiltinId::DomEventFileType => "eventFileType",
            BuiltinId::DomEventFileBytes => "eventFileBytes",
            BuiltinId::DomListenFlags => "listenFlags",
            BuiltinId::DomAssignLocation => "assignLocation",
            BuiltinId::DomJournalDetail => "journalDetail",
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
        other => Err(format!(
            "{name} stride must be an Int, got {}",
            other.type_name()
        )),
    }
}

pub fn call_builtin(id: BuiltinId, args: Vec<RuntimeValue>) -> Result<RuntimeValue, String> {
    if let Some(builtin) = id.extension() {
        return logicaffeine_base::extension::call(builtin, args);
    }
    if id == BuiltinId::DefinedReal {
        return crate::semantics::defined::outside_a_run(args);
    }
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
                other => {
                    return Err(format!(
                        "repeatSeq count must be an Int, got {}",
                        other.type_name()
                    ))
                }
            };
            let slots: Vec<RuntimeValue> = (0..n).map(|_| element.deep_clone()).collect();
            Ok(RuntimeValue::List(Rc::new(RefCell::new(
                ListRepr::from_values(slots),
            ))))
        }
        BuiltinId::ReedSolomonEncode => {
            // Systematic FEC: the first k outputs are the data shards unchanged, followed by
            // `parity` recovery shards (any k of the k+parity shards reconstruct the data).
            let parity = args.remove(1);
            let data = args.remove(0);
            let m = match parity {
                RuntimeValue::Int(n) if n > 0 => n as usize,
                RuntimeValue::Int(n) => {
                    return Err(format!(
                        "reedSolomonEncode parity count must be positive, got {}",
                        n
                    ))
                }
                other => {
                    return Err(format!(
                        "reedSolomonEncode parity count must be an Int, got {}",
                        other.type_name()
                    ))
                }
            };
            let shards = read_shards(&data)?;
            let k = shards.len();
            let rs = logicaffeine_base::fec::ReedSolomon::new(k, m).ok_or_else(|| {
                format!(
                    "reedSolomonEncode: invalid code ({k} data + {m} parity) — need at least one \
                     data shard, at least one parity shard, and a total of at most 256"
                )
            })?;
            let encoded = rs.encode(&shards).ok_or_else(|| {
                format!(
                    "reedSolomonEncode: the {k} data shards must be non-empty and of equal length"
                )
            })?;
            Ok(shards_to_seq(&encoded))
        }
        BuiltinId::ReedSolomonRecover => {
            // Reconstruct every shard from the surviving slots (`nothing` = a loss); the code is
            // rebuilt deterministically from (k data shards, received.count − k parity shards).
            let kv = args.remove(1);
            let received = args.remove(0);
            let k = match kv {
                RuntimeValue::Int(n) if n > 0 => n as usize,
                RuntimeValue::Int(n) => {
                    return Err(format!(
                        "reedSolomonRecover data-shard count must be positive, got {}",
                        n
                    ))
                }
                other => {
                    return Err(format!(
                        "reedSolomonRecover data-shard count must be an Int, got {}",
                        other.type_name()
                    ))
                }
            };
            let slots = read_received(&received)?;
            let n = slots.len();
            if n <= k {
                return Err(format!(
                    "reedSolomonRecover: {n} slots cannot carry {k} data shards plus parity"
                ));
            }
            let rs = logicaffeine_base::fec::ReedSolomon::new(k, n - k).ok_or_else(|| {
                format!(
                    "reedSolomonRecover: invalid code ({k} data + {} parity)",
                    n - k
                )
            })?;
            let recovered = rs.reconstruct(&slots).ok_or_else(|| {
                "reedSolomonRecover: insufficient surviving shards to reconstruct".to_string()
            })?;
            Ok(shards_to_seq(&recovered))
        }
        BuiltinId::FecProtect => {
            // Frame a flat byte payload with a 4-byte big-endian length header, zero-pad to a
            // multiple of `k`, split into `k` shards, and Reed-Solomon encode into `k + m` packets.
            let m_arg = args.remove(2);
            let k_arg = args.remove(1);
            let payload_arg = args.remove(0);
            let k = read_positive_count(k_arg, "fecProtect data-shard count")?;
            let m = read_positive_count(m_arg, "fecProtect parity-shard count")?;
            let payload = byte_seq(&payload_arg)?;
            if payload.len() > u32::MAX as usize {
                return Err("fecProtect: payload is too large to frame (over 4 GiB)".to_string());
            }
            let mut framed = Vec::with_capacity(4 + payload.len());
            framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            framed.extend_from_slice(&payload);
            let shard_len = (framed.len() + k - 1) / k; // ceil, so every shard is equal length
            framed.resize(shard_len * k, 0); // zero-pad the last shard
            let shards: Vec<Vec<u8>> = framed.chunks(shard_len).map(|c| c.to_vec()).collect();
            let rs = logicaffeine_base::fec::ReedSolomon::new(k, m).ok_or_else(|| {
                format!("fecProtect: invalid code ({k} data + {m} parity; total must be ≤ 256)")
            })?;
            let encoded = rs
                .encode(&shards)
                .ok_or_else(|| "fecProtect: shard encoding failed".to_string())?;
            Ok(shards_to_seq(&encoded))
        }
        BuiltinId::FecRestore => {
            // Recover the `k` data shards from the surviving packets, reassemble the framed
            // buffer, and strip the length header + padding to return the exact original payload.
            let k_arg = args.remove(1);
            let received_arg = args.remove(0);
            let k = read_positive_count(k_arg, "fecRestore data-shard count")?;
            let slots = read_received(&received_arg)?;
            let n = slots.len();
            if n <= k {
                return Err(format!(
                    "fecRestore: {n} packets cannot carry {k} data shards plus parity"
                ));
            }
            let rs = logicaffeine_base::fec::ReedSolomon::new(k, n - k)
                .ok_or_else(|| format!("fecRestore: invalid code ({k} data + {} parity)", n - k))?;
            let recovered = rs.reconstruct(&slots).ok_or_else(|| {
                "fecRestore: insufficient surviving packets to reconstruct".to_string()
            })?;
            let mut framed: Vec<u8> = Vec::new();
            for shard in recovered.iter().take(k) {
                framed.extend_from_slice(shard);
            }
            if framed.len() < 4 {
                return Err(
                    "fecRestore: recovered data is too short for a length header".to_string(),
                );
            }
            let len = u32::from_be_bytes([framed[0], framed[1], framed[2], framed[3]]) as usize;
            if 4 + len > framed.len() {
                return Err("fecRestore: length header exceeds the recovered data".to_string());
            }
            Ok(bytes_to_seq(&framed[4..4 + len]))
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
        BuiltinId::Contains => {
            let needle = args.remove(1);
            let haystack = args.remove(0);
            match (&haystack, &needle) {
                (RuntimeValue::Text(h), RuntimeValue::Text(n)) => {
                    Ok(RuntimeValue::Bool(h.contains(n.as_str())))
                }
                _ => Err(format!(
                    "contains expects two Text arguments, got {} and {}",
                    haystack.type_name(),
                    needle.type_name()
                )),
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
                // |z| of a float-complex is its f64 modulus.
                RuntimeValue::ComplexF(c) => Ok(RuntimeValue::Float(c.abs())),
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
                // The floating-point complex sqrt (`√(3+4i) = 2+i`) — the operation the exact
                // Gaussian complex cannot do, and the one the complex decompositions need.
                RuntimeValue::ComplexF(c) => Ok(RuntimeValue::ComplexF(Rc::new(c.sqrt()))),
                // An exact complex has an irrational magnitude, so its sqrt is inexact → `ComplexF`.
                RuntimeValue::Complex(c) => {
                    let cf = logicaffeine_base::ComplexF::new(c.re().to_f64(), c.im().to_f64());
                    Ok(RuntimeValue::ComplexF(Rc::new(cf.sqrt())))
                }
                _ => Err(format!("sqrt() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Infinitesimal
        | BuiltinId::StandardPart
        | BuiltinId::IsInfinitesimal
        | BuiltinId::IsLimited
        | BuiltinId::InfinitelyClose
        | BuiltinId::GeneratorOf
        | BuiltinId::LimitOf
        | BuiltinId::Provisional
        | BuiltinId::AtTheLimit
        | BuiltinId::TripCount
        | BuiltinId::AffinePower
        | BuiltinId::PolynomialFlow
        | BuiltinId::HybridFlow
        | BuiltinId::FirstExit
        | BuiltinId::CycleAt
        | BuiltinId::SearchCandidates
        | BuiltinId::StateSize
        | BuiltinId::EveryNatural
        | BuiltinId::ExactSqrt
        | BuiltinId::ExactExp
        | BuiltinId::ExactLn
        | BuiltinId::ExactPi
        | BuiltinId::ExactE
        | BuiltinId::ExactOmega
        | BuiltinId::ExactPower
        | BuiltinId::OrderedInfinitesimal => unreachable!("answered by logicaffeine_base::extension above"),
        BuiltinId::DefinedReal => unreachable!("refused by semantics::defined above"),
        BuiltinId::Conjugate => {
            let val = args.remove(0);
            match val {
                // The exact conjugate stays in the Gaussian-rational tower; the float conjugate
                // stays a `ComplexF`. Both just negate the imaginary part.
                RuntimeValue::Complex(c) => Ok(RuntimeValue::Complex(Rc::new(c.conjugate()))),
                RuntimeValue::ComplexF(c) => Ok(RuntimeValue::ComplexF(Rc::new(c.conjugate()))),
                // A real number is its own conjugate — pass it through unchanged so `conjugate`
                // is total over the whole numeric tower (a real column of a complex matrix is fine).
                RuntimeValue::Int(_)
                | RuntimeValue::BigInt(_)
                | RuntimeValue::Rational(_)
                | RuntimeValue::Decimal(_)
                | RuntimeValue::Float(_) => Ok(val),
                _ => Err(format!(
                    "conjugate() requires a number, got {}",
                    val.type_name()
                )),
            }
        }
        BuiltinId::RealPart => {
            let val = args.remove(0);
            match val {
                RuntimeValue::ComplexF(c) => Ok(RuntimeValue::Float(c.re())),
                RuntimeValue::Complex(c) => Ok(RuntimeValue::Float(c.re().to_f64())),
                RuntimeValue::Float(f) => Ok(RuntimeValue::Float(f)),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Float(n as f64)),
                RuntimeValue::BigInt(b) => Ok(RuntimeValue::Float(b.to_f64())),
                RuntimeValue::Rational(r) => Ok(RuntimeValue::Float(r.to_f64())),
                _ => Err(format!("re() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::ImagPart => {
            let val = args.remove(0);
            match val {
                RuntimeValue::ComplexF(c) => Ok(RuntimeValue::Float(c.im())),
                RuntimeValue::Complex(c) => Ok(RuntimeValue::Float(c.im().to_f64())),
                // A real number has zero imaginary part.
                RuntimeValue::Float(_)
                | RuntimeValue::Int(_)
                | RuntimeValue::BigInt(_)
                | RuntimeValue::Rational(_) => Ok(RuntimeValue::Float(0.0)),
                _ => Err(format!("im() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Min => {
            let b = args.remove(1);
            let a = args.remove(0);
            match (&a, &b) {
                (RuntimeValue::Int(x), RuntimeValue::Int(y)) => Ok(RuntimeValue::Int(*x.min(y))),
                (RuntimeValue::Float(x), RuntimeValue::Float(y)) => {
                    Ok(RuntimeValue::Float(logicaffeine_base::numeric::float_min(*x, *y)))
                }
                (RuntimeValue::Int(x), RuntimeValue::Float(y)) => {
                    Ok(RuntimeValue::Float(logicaffeine_base::numeric::float_min(*x as f64, *y)))
                }
                (RuntimeValue::Float(x), RuntimeValue::Int(y)) => {
                    Ok(RuntimeValue::Float(logicaffeine_base::numeric::float_min(*x, *y as f64)))
                }
                // Exact decimal min (value-based, scale of the chosen operand preserved).
                (RuntimeValue::Decimal(x), RuntimeValue::Decimal(y)) => {
                    Ok(RuntimeValue::Decimal(if x <= y {
                        x.clone()
                    } else {
                        y.clone()
                    }))
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
                    Ok(RuntimeValue::Float(logicaffeine_base::numeric::float_max(*x, *y)))
                }
                (RuntimeValue::Int(x), RuntimeValue::Float(y)) => {
                    Ok(RuntimeValue::Float(logicaffeine_base::numeric::float_max(*x as f64, *y)))
                }
                (RuntimeValue::Float(x), RuntimeValue::Int(y)) => {
                    Ok(RuntimeValue::Float(logicaffeine_base::numeric::float_max(*x, *y as f64)))
                }
                // Exact decimal max (value-based, scale of the chosen operand preserved).
                (RuntimeValue::Decimal(x), RuntimeValue::Decimal(y)) => {
                    Ok(RuntimeValue::Decimal(if x >= y {
                        x.clone()
                    } else {
                        y.clone()
                    }))
                }
                _ => Err("max() requires numbers".to_string()),
            }
        }
        BuiltinId::Floor => {
            let val = args.remove(0);
            let recurse = |x| call_builtin(BuiltinId::Floor, vec![x]);
            if let Some(r) = crate::semantics::exact::integer_part(&val, crate::semantics::exact::Rounding::Floor, &recurse) {
                return r;
            }
            // An exact integer (Int/BigInt) is already whole; a Rational rounds toward
            // −∞ to an exact integer — the explicit floor of `floor(7 / 2) → 3`.
            match &val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Int(f.floor() as i64)),
                RuntimeValue::Int(_) | RuntimeValue::BigInt(_) => Ok(val.clone()),
                RuntimeValue::Rational(r) => Ok(RuntimeValue::from_bigint(r.floor())),
                RuntimeValue::Decimal(d) => Ok(RuntimeValue::from_bigint(d.to_rational().floor())),
                _ => Err(format!(
                    "floor() requires a number, got {}",
                    val.type_name()
                )),
            }
        }
        BuiltinId::Ceil => {
            let val = args.remove(0);
            let recurse = |x| call_builtin(BuiltinId::Ceil, vec![x]);
            if let Some(r) = crate::semantics::exact::integer_part(&val, crate::semantics::exact::Rounding::Ceil, &recurse) {
                return r;
            }
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
            let recurse = |x| call_builtin(BuiltinId::Round, vec![x]);
            if let Some(r) = crate::semantics::exact::integer_part(&val, crate::semantics::exact::Rounding::Nearest, &recurse) {
                return r;
            }
            match &val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Int(f.round() as i64)),
                RuntimeValue::Int(_) | RuntimeValue::BigInt(_) => Ok(val.clone()),
                RuntimeValue::Rational(r) => Ok(RuntimeValue::from_bigint(r.round())),
                RuntimeValue::Decimal(d) => Ok(RuntimeValue::from_bigint(d.to_rational().round())),
                _ => Err(format!(
                    "round() requires a number, got {}",
                    val.type_name()
                )),
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
                            None => RuntimeValue::from_bigint(
                                logicaffeine_base::BigInt::from_i64(*b).pow(*e as u32),
                            ),
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
                RuntimeValue::Text(s) => Decimal::parse(s)
                    .map(|d| RuntimeValue::Decimal(Rc::new(d)))
                    .ok_or_else(|| logicaffeine_data::failure::not_a_decimal(s)),
                // An integer is already exact — widen it to a scale-0 Decimal, a big one digit for digit.
                RuntimeValue::Int(n) => Ok(RuntimeValue::Decimal(Rc::new(Decimal::from_i64(*n)))),
                RuntimeValue::BigInt(b) => Decimal::parse(&b.to_string())
                    .map(|d| RuntimeValue::Decimal(Rc::new(d)))
                    .ok_or_else(|| logicaffeine_data::failure::not_a_decimal(&b.to_string())),
                // Already a Decimal — identity.
                RuntimeValue::Decimal(_) => Ok(val.clone()),
                _ => Err(logicaffeine_data::failure::decimal_needs_text_or_int(
                    val.type_name(),
                )),
            }
        }
        BuiltinId::Complex => {
            let im = args.remove(1);
            let re = args.remove(0);
            let to_rat = |v: &RuntimeValue| -> Option<logicaffeine_base::Rational> {
                match v {
                    RuntimeValue::Int(n) => Some(logicaffeine_base::Rational::from_i64(*n)),
                    RuntimeValue::BigInt(b) => {
                        Some(logicaffeine_base::Rational::from_bigint((**b).clone()))
                    }
                    RuntimeValue::Rational(r) => Some((**r).clone()),
                    RuntimeValue::Decimal(d) => Some(d.to_rational()),
                    _ => None,
                }
            };
            // A `Float` in EITHER part makes the result a floating-point complex (`ComplexF`) — the
            // same promotion `Rational` meets `Float` with. Two exact parts stay an exact `Complex`
            // (Gaussian rationals). Never the lossy direction silently.
            let has_float =
                matches!(re, RuntimeValue::Float(_)) || matches!(im, RuntimeValue::Float(_));
            if has_float {
                let to_f = |v: &RuntimeValue| -> Option<f64> {
                    if let RuntimeValue::Float(f) = v {
                        Some(*f)
                    } else {
                        to_rat(v).map(|r| r.to_f64())
                    }
                };
                match (to_f(&re), to_f(&im)) {
                    (Some(a), Some(b)) => Ok(RuntimeValue::ComplexF(Rc::new(
                        logicaffeine_base::ComplexF::new(a, b),
                    ))),
                    _ => Err(format!(
                        "complex() requires two real numbers, got {} and {}",
                        re.type_name(),
                        im.type_name()
                    )),
                }
            } else {
                match (to_rat(&re), to_rat(&im)) {
                    (Some(re_r), Some(im_r)) => Ok(RuntimeValue::Complex(Rc::new(
                        logicaffeine_base::Complex::new(re_r, im_r),
                    ))),
                    _ => Err(format!(
                        "complex() requires two real numbers, got {} and {}",
                        re.type_name(),
                        im.type_name()
                    )),
                }
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
                    None => Err(logicaffeine_data::failure::MODULUS_NOT_POSITIVE.to_string()),
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
                _ => {
                    return Err(format!(
                        "quantity() requires an exact number, got {}",
                        value.type_name()
                    ))
                }
            };
            let unit = match &unit_arg {
                RuntimeValue::Text(s) => logicaffeine_base::quantity::units::by_name(s)
                    .ok_or_else(|| logicaffeine_data::failure::unknown_unit(s))?,
                _ => {
                    return Err(format!(
                        "quantity() requires a unit name (Text), got {}",
                        unit_arg.type_name()
                    ))
                }
            };
            let q = logicaffeine_base::Quantity::of(magnitude, &unit);
            Ok(RuntimeValue::Quantity(Rc::new(
                crate::interpreter::QuantityValue { q, unit },
            )))
        }
        BuiltinId::Money => {
            let code_arg = args.remove(1);
            let value = args.remove(0);
            // The amount must be EXACT base-10 (Int or Decimal); a Float or non-terminating Rational
            // would not be representable money, so it is refused.
            let amount = match &value {
                RuntimeValue::Int(n) => logicaffeine_base::Decimal::from_i64(*n),
                RuntimeValue::Decimal(d) => (**d).clone(),
                _ => {
                    return Err(format!(
                        "money() requires an exact base-10 amount (Int or Decimal), got {}",
                        value.type_name()
                    ))
                }
            };
            let currency = match &code_arg {
                RuntimeValue::Text(s) => logicaffeine_base::money::currency::by_code(s)
                    .ok_or_else(|| logicaffeine_data::failure::unknown_currency(s))?,
                _ => {
                    return Err(format!(
                        "money() requires a currency code (Text), got {}",
                        code_arg.type_name()
                    ))
                }
            };
            Ok(RuntimeValue::Money(Rc::new(logicaffeine_base::Money::of(
                amount, currency,
            ))))
        }
        BuiltinId::SetRate => {
            let rate_arg = args.remove(1);
            let code_arg = args.remove(0);
            let code = match &code_arg {
                RuntimeValue::Text(s) => s.to_string(),
                _ => {
                    return Err(format!(
                        "set_rate() requires a currency code (Text), got {}",
                        code_arg.type_name()
                    ))
                }
            };
            let rate = match &rate_arg {
                RuntimeValue::Int(n) => logicaffeine_base::Rational::from_i64(*n),
                RuntimeValue::Decimal(d) => d.to_rational(),
                RuntimeValue::Rational(r) => (**r).clone(),
                _ => {
                    return Err(format!(
                        "set_rate() requires an exact rate (Int/Decimal), got {}",
                        rate_arg.type_name()
                    ))
                }
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
                    _ => {
                        return Err(format!(
                            "set_rates() keys must be currency codes (Text), got {}",
                            key.type_name()
                        ))
                    }
                };
                let rate = match value {
                    RuntimeValue::Int(n) => logicaffeine_base::Rational::from_i64(*n),
                    RuntimeValue::Decimal(d) => d.to_rational(),
                    RuntimeValue::Rational(r) => (**r).clone(),
                    _ => {
                        return Err(format!(
                            "set_rates() values must be exact rates (Int/Decimal), got {}",
                            value.type_name()
                        ))
                    }
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
                    .ok_or_else(|| logicaffeine_data::failure::invalid_uuid(s)),
                _ => Err(format!("uuid() requires text, got {}", arg.type_name())),
            }
        }
        BuiltinId::UuidNil => Ok(RuntimeValue::Uuid(Rc::new(logicaffeine_base::Uuid::NIL))),
        BuiltinId::UuidMax => Ok(RuntimeValue::Uuid(Rc::new(logicaffeine_base::Uuid::MAX))),
        BuiltinId::UuidDns => Ok(RuntimeValue::Uuid(Rc::new(
            logicaffeine_base::Uuid::NAMESPACE_DNS,
        ))),
        BuiltinId::UuidUrl => Ok(RuntimeValue::Uuid(Rc::new(
            logicaffeine_base::Uuid::NAMESPACE_URL,
        ))),
        BuiltinId::UuidOid => Ok(RuntimeValue::Uuid(Rc::new(
            logicaffeine_base::Uuid::NAMESPACE_OID,
        ))),
        BuiltinId::UuidX500 => Ok(RuntimeValue::Uuid(Rc::new(
            logicaffeine_base::Uuid::NAMESPACE_X500,
        ))),
        BuiltinId::UuidVersion => {
            let arg = args.remove(0);
            match &arg {
                RuntimeValue::Uuid(u) => Ok(RuntimeValue::Int(u.version() as i64)),
                _ => Err(format!(
                    "uuid_version() requires a Uuid, got {}",
                    arg.type_name()
                )),
            }
        }
        BuiltinId::TextBytes => {
            let arg = args.remove(0);
            match &arg {
                RuntimeValue::Text(s) => Ok(bytes_to_seq(s.as_bytes())),
                _ => Err(format!(
                    "text_bytes() requires text, got {}",
                    arg.type_name()
                )),
            }
        }
        BuiltinId::TextFromBytes => {
            let bytes = byte_seq(&args.remove(0))?;
            match String::from_utf8(bytes) {
                Ok(s) => Ok(RuntimeValue::Text(Rc::new(s))),
                Err(e) => Err(logicaffeine_data::failure::invalid_utf8(e)),
            }
        }
        BuiltinId::WireBytes => {
            let arg = args.remove(0);
            match crate::concurrency::marshal::encode_value_raw(&arg) {
                Ok(bytes) => Ok(bytes_to_seq(&bytes)),
                Err(e) => Err(format!("wireBytes(): {}", e)),
            }
        }
        BuiltinId::CompressBytes => {
            let bytes = byte_seq(&args.remove(0))?;
            Ok(bytes_to_seq(&crate::concurrency::marshal::compress_best(
                &bytes,
            )))
        }
        BuiltinId::DecompressBytes => {
            let frame = byte_seq(&args.remove(0))?;
            match crate::concurrency::marshal::decompress_any(&frame) {
                Some(bytes) => Ok(bytes_to_seq(&bytes)),
                None => {
                    Err("decompressBytes(): malformed or oversized compressed frame".to_string())
                }
            }
        }
        BuiltinId::SealBytes => {
            let key = read_aead_key(&args.remove(0), "sealBytes")?;
            let nonce = read_aead_nonce(&args.remove(0), "sealBytes")?;
            let plaintext = exact_byte_seq("sealBytes", "plaintext", &args.remove(0))?;
            // Nothing is compressed first: a ciphertext as long as a compressed plaintext tells how
            // compressible the plaintext was, which gives away a secret sealed beside chosen data.
            let sealed =
                logicaffeine_system::aead::chacha20poly1305_seal(&key, &nonce, &[], &plaintext);
            Ok(bytes_to_seq(&sealed))
        }
        BuiltinId::OpenBytes => {
            let key = read_aead_key(&args.remove(0), "openBytes")?;
            let nonce = read_aead_nonce(&args.remove(0), "openBytes")?;
            let sealed = exact_byte_seq("openBytes", "sealed frame", &args.remove(0))?;
            let plaintext = logicaffeine_system::aead::chacha20poly1305_open(&key, &nonce, &[], &sealed)
                .ok_or_else(|| {
                    "openBytes(): authentication failed — the frame was tampered, truncated, or the key is wrong".to_string()
                })?;
            Ok(bytes_to_seq(&plaintext))
        }
        BuiltinId::FromWireBytes => {
            let bytes = byte_seq(&args.remove(0))?;
            crate::concurrency::marshal::decode_value_raw(&bytes)
                .ok_or_else(|| "fromWireBytes(): malformed or trailing-byte wire input".to_string())
        }
        BuiltinId::SealFrame => {
            let key = read_aead_key(&args.remove(0), "sealFrame")?;
            let nonce = read_aead_nonce(&args.remove(0), "sealFrame")?;
            let value = args.remove(0);
            // wire-encode → authenticated-encrypt, fused (no intermediate Logos Seq), and never
            // compressed between, for the reason `sealBytes` gives.
            let wire = crate::concurrency::marshal::encode_value_raw(&value)
                .map_err(|e| format!("sealFrame(): {}", e))?;
            let sealed = logicaffeine_system::aead::chacha20poly1305_seal(&key, &nonce, &[], &wire);
            Ok(bytes_to_seq(&sealed))
        }
        BuiltinId::OpenFrame => {
            let key = read_aead_key(&args.remove(0), "openFrame")?;
            let nonce = read_aead_nonce(&args.remove(0), "openFrame")?;
            let sealed = exact_byte_seq("openFrame", "sealed frame", &args.remove(0))?;
            let wire = logicaffeine_system::aead::chacha20poly1305_open(&key, &nonce, &[], &sealed)
                .ok_or_else(|| {
                    "openFrame(): authentication failed — the frame was tampered, truncated, or the key is wrong".to_string()
                })?;
            crate::concurrency::marshal::decode_value_raw(&wire)
                .ok_or_else(|| "openFrame(): malformed wire value after decryption".to_string())
        }
        BuiltinId::SealBox => {
            let key = read_aead_key(&args.remove(0), "sealBox")?;
            let plaintext = exact_byte_seq("sealBox", "plaintext", &args.remove(0))?;
            let sealed = logicaffeine_system::aead::seal_box(
                &key,
                logicaffeine_system::aead::BOX_BYTES_LABEL,
                &plaintext,
            )
            .map_err(|why| format!("sealBox(): {why}"))?;
            Ok(bytes_to_seq(&sealed))
        }
        BuiltinId::OpenBox => {
            let key = read_aead_key(&args.remove(0), "openBox")?;
            let sealed = read_box(&args.remove(0), "openBox")?;
            let plaintext =
                logicaffeine_system::aead::open_box(&key, logicaffeine_system::aead::BOX_BYTES_LABEL, &sealed)
                    .ok_or_else(|| box_refused("openBox"))?;
            Ok(bytes_to_seq(&plaintext))
        }
        BuiltinId::SealValueBox => {
            let key = read_aead_key(&args.remove(0), "sealValueBox")?;
            let wire = crate::concurrency::marshal::encode_value_raw(&args.remove(0))
                .map_err(|e| format!("sealValueBox(): {e}"))?;
            let sealed =
                logicaffeine_system::aead::seal_box(&key, logicaffeine_system::aead::BOX_VALUE_LABEL, &wire)
                    .map_err(|why| format!("sealValueBox(): {why}"))?;
            Ok(bytes_to_seq(&sealed))
        }
        BuiltinId::OpenValueBox => {
            let key = read_aead_key(&args.remove(0), "openValueBox")?;
            let sealed = read_box(&args.remove(0), "openValueBox")?;
            let wire =
                logicaffeine_system::aead::open_box(&key, logicaffeine_system::aead::BOX_VALUE_LABEL, &sealed)
                    .ok_or_else(|| box_refused("openValueBox"))?;
            crate::concurrency::marshal::decode_value_raw(&wire)
                .ok_or_else(|| "openValueBox(): malformed wire value after decryption".to_string())
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
                _ => Err(format!(
                    "uuid_bytes() requires a Uuid, got {}",
                    arg.type_name()
                )),
            }
        }
        BuiltinId::UuidFromBytes => {
            let bytes = byte_seq(&args.remove(0))?;
            if bytes.len() < 16 {
                return Err(logicaffeine_data::failure::uuid_needs_16_bytes(bytes.len()));
            }
            let mut b = [0u8; 16];
            b.copy_from_slice(&bytes[..16]);
            Ok(RuntimeValue::Uuid(Rc::new(
                logicaffeine_base::Uuid::from_bytes(b),
            )))
        }
        BuiltinId::ToCurrency => {
            let code_arg = args.remove(1);
            let money = args.remove(0);
            match (&money, &code_arg) {
                (RuntimeValue::Money(m), RuntimeValue::Text(code)) => {
                    let to = logicaffeine_base::money::currency::by_code(code)
                        .ok_or_else(|| logicaffeine_data::failure::unknown_currency(code))?;
                    logicaffeine_base::money::ambient_convert(m, to)
                        .map(|c| RuntimeValue::Money(Rc::new(c)))
                        .ok_or_else(|| {
                            if logicaffeine_base::money::has_ambient_rates() {
                                logicaffeine_data::failure::no_exchange_rate(
                                    m.currency.code,
                                    to.code,
                                )
                            } else {
                                logicaffeine_data::failure::NO_EXCHANGE_RATES.to_string()
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
                    .ok_or_else(|| logicaffeine_data::failure::unknown_unit(s))?,
                _ => {
                    return Err(format!(
                        "convert() requires a unit name (Text), got {}",
                        unit_arg.type_name()
                    ))
                }
            };
            match &value {
                // Same dimension → re-express (the SI magnitude is unchanged, only the display unit);
                // a different dimension is the forbidden cast.
                RuntimeValue::Quantity(qv) => {
                    if qv.q.dimension() != unit.dimension {
                        return Err(logicaffeine_data::failure::cannot_convert(
                            qv.unit.symbol,
                            unit.symbol,
                        ));
                    }
                    Ok(RuntimeValue::Quantity(Rc::new(
                        crate::interpreter::QuantityValue {
                            q: qv.q.clone(),
                            unit,
                        },
                    )))
                }
                _ => Err(format!(
                    "convert() requires a quantity, got {}",
                    value.type_name()
                )),
            }
        }
        BuiltinId::ParseTimestamp => {
            let val = args.remove(0);
            match &val {
                RuntimeValue::Text(s) => logicaffeine_base::temporal::parse_rfc3339(s.trim())
                    .map(RuntimeValue::Moment)
                    .ok_or_else(|| logicaffeine_data::failure::not_a_timestamp(s)),
                _ => Err(format!(
                    "parse_timestamp() requires a Text, got {}",
                    val.type_name()
                )),
            }
        }
        BuiltinId::FormatTimestamp => {
            let val = args.remove(0);
            match &val {
                RuntimeValue::Moment(nanos) => Ok(RuntimeValue::Text(Rc::new(
                    logicaffeine_base::temporal::format_rfc3339(*nanos),
                ))),
                _ => Err(format!(
                    "format_timestamp() requires a Moment, got {}",
                    val.type_name()
                )),
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
                        BuiltinId::WeekdayOf => temporal::iso_weekday_from_days(
                            nanos.div_euclid(temporal::NANOS_PER_DAY),
                        ) as i64,
                        BuiltinId::WeekOf => {
                            temporal::iso_week_from_days(nanos.div_euclid(temporal::NANOS_PER_DAY))
                                .1 as i64
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
                        BuiltinId::WeekdayOf => {
                            temporal::iso_weekday_from_days(*days as i64) as i64
                        }
                        BuiltinId::WeekOf => temporal::iso_week_from_days(*days as i64).1 as i64,
                        BuiltinId::QuarterOf => (m as i64 - 1) / 3 + 1,
                        BuiltinId::HourOf | BuiltinId::MinuteOf | BuiltinId::SecondOf => {
                            return Err(
                                "a Date has no time-of-day — use a timestamp/Moment".to_string()
                            );
                        }
                        _ => unreachable!(),
                    };
                    Ok(RuntimeValue::Int(component))
                }
                _ => Err(format!(
                    "a date component extractor requires a Moment or Date, got {}",
                    val.type_name()
                )),
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
                _ => Err(format!(
                    "date_of() requires a Moment or Date, got {}",
                    val.type_name()
                )),
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
                _ => Err(format!(
                    "time_of() requires a Moment, got {}",
                    val.type_name()
                )),
            }
        }
        BuiltinId::MakeDate => {
            let d = args.remove(2);
            let m = args.remove(1);
            let y = args.remove(0);
            match (&y, &m, &d) {
                (RuntimeValue::Int(y), RuntimeValue::Int(mo), RuntimeValue::Int(da)) => Ok(
                    RuntimeValue::Date(logicaffeine_base::temporal::days_from_civil(
                        *y, *mo as u32, *da as u32,
                    ) as i32),
                ),
                _ => Err(format!(
                    "makeDate() requires three Ints (year, month, day), got {}, {}, {}",
                    y.type_name(),
                    m.type_name(),
                    d.type_name()
                )),
            }
        }
        BuiltinId::MakeMoment => {
            let s = args.remove(5);
            let mi = args.remove(4);
            let h = args.remove(3);
            let d = args.remove(2);
            let m = args.remove(1);
            let y = args.remove(0);
            match (&y, &m, &d, &h, &mi, &s) {
                (
                    RuntimeValue::Int(y),
                    RuntimeValue::Int(mo),
                    RuntimeValue::Int(da),
                    RuntimeValue::Int(hr),
                    RuntimeValue::Int(min),
                    RuntimeValue::Int(sec),
                ) => {
                    let days =
                        logicaffeine_base::temporal::days_from_civil(*y, *mo as u32, *da as u32);
                    let nanos = days * logicaffeine_base::temporal::NANOS_PER_DAY
                        + (*hr * 3600 + *min * 60 + *sec)
                            * logicaffeine_base::temporal::NANOS_PER_SECOND;
                    Ok(RuntimeValue::Moment(nanos))
                }
                _ => Err(
                    "makeMoment() requires six Ints (year, month, day, hour, minute, second)"
                        .to_string(),
                ),
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
                        .ok_or_else(|| logicaffeine_data::failure::unknown_time_zone(z))
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
                        .ok_or_else(|| logicaffeine_data::failure::unknown_time_zone(z))
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
        BuiltinId::Fail => Err(args.remove(0).to_display_string()),
        BuiltinId::FailReason => Err(contract_reason_text(args.remove(0))?),
        // G5/S20: the loud message for a non-exhaustive `Inspect` with no compile-time-known
        // enum — `vm/compiler.rs`'s `compile_inspect` emits this call ONLY for its own dynamic
        // fallback arm, passing the already-evaluated scrutinee. Reads the ACTUAL runtime
        // constructor the same way the tree-walker's `inspect_unhandled` does, so the two tiers
        // fail with byte-identical words.
        BuiltinId::InspectUnhandledDynamic => {
            let target = args.remove(0);
            let variant = match &target {
                RuntimeValue::Inductive(i) => i.constructor.as_str(),
                _ => "the value",
            };
            Err(inspect_unhandled_message(target.type_name(), variant))
        }
        BuiltinId::NativeKernel => {
            use crate::semantics::native_kernels::NativeKernel;
            let kernel = match args.first() {
                Some(RuntimeValue::Int(code)) => NativeKernel::from_code(*code)
                    .ok_or_else(|| format!("unknown native kernel code {code}"))?,
                _ => return Err("a native kernel call carries its kernel code".to_string()),
            };
            kernel.call(&args[1..])
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
        BuiltinId::LeadingZeros => {
            let val = args.remove(0);
            match val {
                // High zero bits of the u64 pattern (matches codegen's
                // `(x as u64).leading_zeros()`; 64 for 0).
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int((n as u64).leading_zeros() as i64)),
                _ => Err(format!(
                    "leading_zeros() requires an Int, got {}",
                    val.type_name()
                )),
            }
        }
        BuiltinId::TrailingZeros => {
            let val = args.remove(0);
            match val {
                // Low zero bits of the u64 pattern (matches codegen's
                // `(x as u64).trailing_zeros()`; 64 for 0).
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int((n as u64).trailing_zeros() as i64)),
                _ => Err(format!(
                    "trailing_zeros() requires an Int, got {}",
                    val.type_name()
                )),
            }
        }
        BuiltinId::Word32 => {
            let n = args.remove(0);
            match n {
                RuntimeValue::Int(v) => Ok(RuntimeValue::Word(WordVal::W32(Word32(v as u32)))),
                RuntimeValue::BigInt(v) => Ok(RuntimeValue::Word(WordVal::W32(Word32(
                    v.wrapping_u64() as u32,
                )))),
                RuntimeValue::Word(w) => {
                    Ok(RuntimeValue::Word(WordVal::W32(Word32(w.to_u64() as u32))))
                }
                _ => Err(format!("word32() requires an Int, got {}", n.type_name())),
            }
        }
        BuiltinId::Word64 => {
            let n = args.remove(0);
            match n {
                RuntimeValue::Int(v) => Ok(RuntimeValue::Word(WordVal::W64(Word64(v as u64)))),
                RuntimeValue::BigInt(v) => Ok(RuntimeValue::Word(WordVal::W64(Word64(
                    v.wrapping_u64() as u64,
                )))),
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
                _ => {
                    return Err(format!(
                        "rotate count must be an Int, got {}",
                        n.type_name()
                    ))
                }
            };
            match w {
                RuntimeValue::Word(word) => {
                    let r = if matches!(id, BuiltinId::Rotl) {
                        word.rotl(count)
                    } else {
                        word.rotr(count)
                    };
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
                    let r = if id == BuiltinId::Wand {
                        x.bitand(*y)
                    } else {
                        x.bitor(*y)
                    };
                    match r {
                        Some(w) => Ok(RuntimeValue::Word(w)),
                        None => Err(format!(
                            "word_and/or width mismatch: {} vs {}",
                            a.type_name(),
                            b.type_name()
                        )),
                    }
                }
                // Lane-vector bitwise mixing (the MD5 F/G functions written over `Lanes8Word32`).
                (RuntimeValue::Lanes(x), RuntimeValue::Lanes(y)) => {
                    let r = if id == BuiltinId::Wand {
                        (**x).bitand(**y)
                    } else {
                        (**x).bitor(**y)
                    };
                    match r {
                        Some(v) => Ok(RuntimeValue::Lanes(Rc::new(v))),
                        None => Err(format!(
                            "word_and/or lane-config mismatch: {} vs {}",
                            a.type_name(),
                            b.type_name()
                        )),
                    }
                }
                _ => Err(format!(
                    "word_and/or requires two Words, got {} and {}",
                    a.type_name(),
                    b.type_name()
                )),
            }
        }
        BuiltinId::Wnot => {
            let a = args.remove(0);
            match &a {
                RuntimeValue::Word(x) => Ok(RuntimeValue::Word(x.not())),
                RuntimeValue::Lanes(x) => match (**x).lane_not() {
                    Some(v) => Ok(RuntimeValue::Lanes(Rc::new(v))),
                    None => Err(format!(
                        "word_not: lane config has no complement: {}",
                        a.type_name()
                    )),
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
                other => Err(format!(
                    "lanes4Word32() requires a Seq of Word32, got {}",
                    other.type_name()
                )),
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
                        return Err(format!(
                            "lanes4Of() needs four Word32, found {}",
                            other.type_name()
                        ))
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
                    other => Err(format!(
                        "seqOfLanes4W32() requires a Lanes4Word32, got {}",
                        other.type_name()
                    )),
                },
                other => Err(format!(
                    "seqOfLanes4W32() requires a Lanes4Word32, got {}",
                    other.type_name()
                )),
            }
        }
        BuiltinId::Sha1Rnds4 => {
            let func = args.remove(2);
            let msg = args.remove(1);
            let abcd = args.remove(0);
            let f = match &func {
                RuntimeValue::Int(n) => *n as u32,
                _ => {
                    return Err(format!(
                        "sha1rnds4() func must be an Int, got {}",
                        func.type_name()
                    ))
                }
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
            other => Err(format!(
                "splat16Word8() requires an Int, got {}",
                other.type_name()
            )),
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
                other => Err(format!(
                    "intOfWord32() requires a Word32, got {}",
                    other.type_name()
                )),
            }
        }
        BuiltinId::IntOfWord64 => {
            // The value of a Word64 as an Int (Keccak byte-masked lanes).
            let x = args.remove(0);
            match x {
                RuntimeValue::Word(w) => Ok(RuntimeValue::Int(w.to_u64() as i64)),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n)),
                other => Err(format!(
                    "intOfWord64() requires a Word64, got {}",
                    other.type_name()
                )),
            }
        }
        BuiltinId::Word64Shl | BuiltinId::Word64Shr => {
            let is_shl = matches!(id, BuiltinId::Word64Shl);
            let w = args.remove(0);
            let n = args.remove(0);
            let wv = match w {
                RuntimeValue::Word(w) => w.to_u64(),
                RuntimeValue::Int(v) => v as u64,
                other => {
                    return Err(format!(
                        "word64 shift requires a Word64, got {}",
                        other.type_name()
                    ))
                }
            };
            let nv = match n {
                RuntimeValue::Int(v) => v as u32,
                other => {
                    return Err(format!(
                        "word64 shift amount requires an Int, got {}",
                        other.type_name()
                    ))
                }
            };
            let r = if is_shl {
                wv.wrapping_shl(nv)
            } else {
                wv.wrapping_shr(nv)
            };
            Ok(RuntimeValue::Word(WordVal::W64(Word64(r))))
        }
        BuiltinId::Word64And => {
            let a = args.remove(0);
            let b = args.remove(0);
            let av = match a {
                RuntimeValue::Word(w) => w.to_u64(),
                RuntimeValue::Int(v) => v as u64,
                other => {
                    return Err(format!(
                        "word64And requires a Word64, got {}",
                        other.type_name()
                    ))
                }
            };
            let bv = match b {
                RuntimeValue::Word(w) => w.to_u64(),
                RuntimeValue::Int(v) => v as u64,
                other => {
                    return Err(format!(
                        "word64And requires a Word64, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(RuntimeValue::Word(WordVal::W64(Word64(av & bv))))
        }
        BuiltinId::Word32Shr => {
            let w = args.remove(0);
            let n = args.remove(0);
            let wv = match w {
                RuntimeValue::Word(word) => word.to_u64() as u32,
                RuntimeValue::Int(v) => v as u32,
                other => {
                    return Err(format!(
                        "word32Shr requires a Word32, got {}",
                        other.type_name()
                    ))
                }
            };
            let nv = match n {
                RuntimeValue::Int(v) => v as u32,
                other => {
                    return Err(format!(
                        "word32Shr amount requires an Int, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(RuntimeValue::Word(WordVal::W32(Word32(
                wv.wrapping_shr(nv),
            ))))
        }
        BuiltinId::Word16Make => {
            // Low 16 bits of an Int as a Word16 — the ring ℤ/2¹⁶, wrapping at its own width.
            let n = args.remove(0);
            match n {
                RuntimeValue::Int(v) => Ok(RuntimeValue::Word(WordVal::W16(Word16(v as u16)))),
                RuntimeValue::BigInt(v) => Ok(RuntimeValue::Word(WordVal::W16(Word16(
                    v.wrapping_u64() as u16,
                )))),
                RuntimeValue::Word(w) => {
                    Ok(RuntimeValue::Word(WordVal::W16(Word16(w.to_u64() as u16))))
                }
                other => Err(format!(
                    "word16() requires an Int, got {}",
                    other.type_name()
                )),
            }
        }
        BuiltinId::IntOfWord16 => {
            // The unsigned value of a Word16 as an Int (0..2¹⁶−1).
            let x = args.remove(0);
            match x {
                RuntimeValue::Word(WordVal::W16(w)) => Ok(RuntimeValue::Int(w.0 as i64)),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n as u16 as i64)),
                other => Err(format!(
                    "intOfWord16() requires a Word16, got {}",
                    other.type_name()
                )),
            }
        }
        BuiltinId::Word8Make => {
            // Low 8 bits of an Int as a Word8 — the ring ℤ/2⁸, wrapping at its own width.
            let n = args.remove(0);
            match n {
                RuntimeValue::Int(v) => Ok(RuntimeValue::Word(WordVal::W8(Word8(v as u8)))),
                RuntimeValue::BigInt(v) => Ok(RuntimeValue::Word(WordVal::W8(Word8(
                    v.wrapping_u64() as u8,
                )))),
                RuntimeValue::Word(w) => {
                    Ok(RuntimeValue::Word(WordVal::W8(Word8(w.to_u64() as u8))))
                }
                other => Err(format!(
                    "word8() requires an Int, got {}",
                    other.type_name()
                )),
            }
        }
        BuiltinId::IntOfWord8 => {
            // The unsigned value of a Word8 as an Int (0..255).
            let x = args.remove(0);
            match x {
                RuntimeValue::Word(WordVal::W8(w)) => Ok(RuntimeValue::Int(w.0 as i64)),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n as u8 as i64)),
                other => Err(format!(
                    "intOfWord8() requires a Word8, got {}",
                    other.type_name()
                )),
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
                other => Err(format!(
                    "lanes4Word64() requires a Seq, got {}",
                    other.type_name()
                )),
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
                other => Err(format!(
                    "seqOfLanes4() requires a lane vector, got {}",
                    other.type_name()
                )),
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
                other => Err(format!(
                    "hsumLanes4 requires a lane vector, got {}",
                    other.type_name()
                )),
            }
        }
        BuiltinId::Splat4Word64 => {
            // Broadcast a Word64 into all 4 Keccak lanes. The tree-walker/VM run the same scalar
            // fallback the AOT path uses when AVX2 is off, so the 4-way lane Keccak
            // (keccakF4Logos → keccakSponge4 → ML-DSA ExpandA) stays tw == VM == AOT bit-exact.
            let w = args.remove(0);
            match w {
                RuntimeValue::Word(WordVal::W64(x)) => Ok(RuntimeValue::Lanes(Rc::new(
                    LanesVal::L4W64(logicaffeine_base::Lanes4Word64::splat(x.0)),
                ))),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Lanes(Rc::new(LanesVal::L4W64(
                    logicaffeine_base::Lanes4Word64::splat(n as u64),
                )))),
                other => Err(format!(
                    "splat4Word64() requires a Word64, got {}",
                    other.type_name()
                )),
            }
        }
        BuiltinId::AndNot4 => {
            // Keccak χ's (¬a) ∧ b per lane — the scalar mirror of the AVX2 vpandn.
            let a = args.remove(0);
            let b = args.remove(0);
            match (a, b) {
                (RuntimeValue::Lanes(la), RuntimeValue::Lanes(lb)) => match (&*la, &*lb) {
                    (LanesVal::L4W64(x), LanesVal::L4W64(y)) => Ok(RuntimeValue::Lanes(Rc::new(
                        LanesVal::L4W64((*x).andnot(*y)),
                    ))),
                    _ => Err(format!(
                        "andNot4 requires two Lanes4Word64, got {} and {}",
                        la.type_name(),
                        lb.type_name()
                    )),
                },
                (a, b) => Err(format!(
                    "andNot4 requires two lane vectors, got {} and {}",
                    a.type_name(),
                    b.type_name()
                )),
            }
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
                            RuntimeValue::Word(WordVal::W16(w)) => words.push(*w),
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
                other => Err(format!(
                    "lanes16Word16() requires a Seq, got {}",
                    other.type_name()
                )),
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
                other => Err(format!(
                    "seqOfLanes16() requires a lane vector, got {}",
                    other.type_name()
                )),
            }
        }
        BuiltinId::Splat16Word16 => {
            // Broadcast a Word16/Int into all 16 lanes (the NTT loads a shared zeta/constant).
            let x = args.remove(0);
            let w = match x {
                RuntimeValue::Word(WordVal::W16(w)) => w.0,
                RuntimeValue::Int(n) => n as u16,
                other => {
                    return Err(format!(
                        "splat16Word16() requires a Word16/Int, got {}",
                        other.type_name()
                    ))
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
                    let r = if is_low {
                        lv.ntt_bcast_lo(h)
                    } else {
                        lv.ntt_bcast_hi(h)
                    };
                    match r {
                        Some(out) => Ok(RuntimeValue::Lanes(Rc::new(out))),
                        None => Err(format!(
                            "{} requires a Lanes16Word16, got {}",
                            name,
                            lv.type_name()
                        )),
                    }
                }
                other => Err(format!(
                    "{} requires a lane vector, got {}",
                    name,
                    other.type_name()
                )),
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
                    return Err(format!(
                        "splat8Word32() requires a Word32, got {}",
                        other.type_name()
                    ))
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
                    other => Err(format!(
                        "run_accepted: {what} must be an Int, got {}",
                        other.type_name()
                    )),
                }
            };
            let arg = int_arg(&args[1], "the argument")?;
            let lo = int_arg(&args[2], "the lower bound")?;
            let hi = int_arg(&args[3], "the upper bound")?;
            let contract = crate::semantics::acceptance::AcceptanceContract::new(lo, hi);
            contract.apply(&args[0], arg).map(RuntimeValue::Int)
        }
        // --- DOM (the browser tier): operate on the thread-local `dom::DomRecorder`
        // so the tree-walker and VM agree; the wasm tier lowers the same calls to
        // `dom_*` host imports. Element handles are `Int`; effect ops return Nothing. ---
        BuiltinId::DomBody => Ok(RuntimeValue::Int(crate::semantics::dom::body())),
        BuiltinId::DomCreateElement => {
            let tag = args.remove(0);
            Ok(RuntimeValue::Int(crate::semantics::dom::create_element(
                &tag.to_display_string(),
            )))
        }
        BuiltinId::DomCreateTextNode => {
            let text = args.remove(0);
            Ok(RuntimeValue::Int(crate::semantics::dom::create_text_node(
                &text.to_display_string(),
            )))
        }
        BuiltinId::DomSetText => {
            let el = dom_handle(args.remove(0), "setText")?;
            let text = args.remove(0);
            crate::semantics::dom::set_text(el, &text.to_display_string());
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomSetHtml => {
            let el = dom_handle(args.remove(0), "setHtml")?;
            let html = args.remove(0);
            crate::semantics::dom::set_html(el, &html.to_display_string());
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomBeginPaint => Err(BEGIN_PAINT_RETIRED.to_string()),
        BuiltinId::DomClearChildren => {
            let el = dom_handle(args.remove(0), "clearChildren")?;
            crate::semantics::dom::clear_children(el);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomMoveChildren => Err(MOVE_CHILDREN_RETIRED.to_string()),
        // ★ THE REFERENCE TIER SUBSCRIBES BY REGISTERING, not by calling this. A page under
        // `largo test` has no relay and no network: its subscriptions live in the listener table
        // and a `## Test` causes one with `Raise`. So this is the BROWSER's half, and here it is
        // the no-op that keeps the two tiers running the same program — the same treatment the
        // window-event registration gets.
        BuiltinId::DomOnMessage => Ok(RuntimeValue::Nothing),
        BuiltinId::DomSetAttribute => {
            let el = dom_handle(args.remove(0), "setAttribute")?;
            let name = args.remove(0);
            let value = args.remove(0);
            // Through `set_attribute_value`, so an HTML boolean attribute is a PRESENCE on this
            // tier exactly as it is on the reference tier — `disabled={false}` writes nothing.
            crate::semantics::dom::set_attribute_value(el, &name.to_display_string(), &value);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomAppendChild => {
            let parent = dom_handle(args.remove(0), "appendChild")?;
            let child = dom_handle(args.remove(0), "appendChild")?;
            crate::semantics::dom::append_child(parent, child);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomGetText => {
            let el = dom_handle(args.remove(0), "getText")?;
            Ok(RuntimeValue::Text(Rc::new(
                crate::semantics::dom::get_text(el),
            )))
        }
        BuiltinId::DomGetValue => {
            let el = dom_handle(args.remove(0), "getValue")?;
            Ok(RuntimeValue::Text(Rc::new(
                crate::semantics::dom::get_value(el),
            )))
        }
        BuiltinId::DomSetValue => {
            let el = dom_handle(args.remove(0), "setValue")?;
            let value = args.remove(0);
            crate::semantics::dom::set_value(el, &value.to_display_string());
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomApplyPatches => {
            let buffer = args.remove(0);
            let bytes = crate::semantics::patch::bytes_from_value(&buffer)?;
            // Decode the WHOLE buffer before touching the document: all-or-nothing, so a
            // malformed stream never leaves a half-built page behind.
            let ops = crate::semantics::patch::decode(&bytes)?;
            crate::semantics::patch::apply(&ops)?;
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomApplyWords => {
            let count = |v: RuntimeValue, what: &str| match v {
                RuntimeValue::Int(n) => Ok(n),
                other => Err(format!(
                    "applyPatchWords takes its {what} count as an Int, got {}",
                    other.type_name()
                )),
            };
            let words = args.remove(0);
            let word_count = count(args.remove(0), "word")?;
            let strings = args.remove(0);
            let string_count = count(args.remove(0), "string")?;
            let words = crate::semantics::patch::words_from_value(&words, word_count)?;
            let strings = crate::semantics::patch::strings_from_value(&strings, string_count)?;
            let ops = crate::semantics::patch::decode_words(&words, &strings)?;
            Ok(RuntimeValue::Int(crate::semantics::patch::apply(&ops)?))
        }
        // Text in, text out, and NOTHING IS SPECIAL: the browser API is length-delimited, so a
        // quote, a newline and a NUL are ordinary content with no separator to escape.
        BuiltinId::StorageSet => {
            let area = args.remove(0).to_display_string();
            let key = args.remove(0).to_display_string();
            let value = args.remove(0).to_display_string();
            crate::semantics::storage::set(&area, &key, &value)?;
            Ok(RuntimeValue::Nothing)
        }
        // `Nothing` for an absent key, the Text for a present one — a stored `""` is a value and
        // must not read as an absence.
        BuiltinId::StorageGet => {
            let area = args.remove(0).to_display_string();
            let key = args.remove(0).to_display_string();
            // ★ A REAL OPTION, not the payload-or-sentinel it used to be.
            //
            // This returned a bare `Text` when present and `Nothing` when absent — the transparent
            // encoding — and that worked only while `Inspect`'s catch-all answered a `Some` arm
            // with "any value that is not the sentinel". Giving `Option` a real tag removed that
            // catch-all, and a PRESENT stored value started taking the `None` arm
            // (`storage_lock::a_missing_key_is_none_and_not_an_empty_string`).
            //
            // Display is unchanged by R1 (an Option prints its payload, or `nothing`), and
            // `is equal to nothing` is unchanged by R2, so every existing reader sees what it saw.
            Ok(match crate::semantics::storage::get(&area, &key)? {
                Some(v) => crate::interpreter::option_some(RuntimeValue::Text(Rc::new(v))),
                None => crate::interpreter::option_none(),
            })
        }
        BuiltinId::StorageGetOr => {
            let area = args.remove(0).to_display_string();
            let key = args.remove(0).to_display_string();
            let fallback = args.remove(0);
            Ok(match crate::semantics::storage::get(&area, &key)? {
                Some(v) => RuntimeValue::Text(Rc::new(v)),
                None => fallback,
            })
        }
        BuiltinId::StorageRemove => {
            let area = args.remove(0).to_display_string();
            let key = args.remove(0).to_display_string();
            crate::semantics::storage::remove(&area, &key)?;
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomSetChecked => {
            let el = dom_handle(args.remove(0), "setChecked")?;
            let on = args.remove(0);
            crate::semantics::dom::set_checked(el, on.is_truthy());
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomPreventDefault => {
            let el = dom_handle(args.remove(0), "preventDefault")?;
            let event = args.remove(0).to_display_string();
            crate::semantics::dom::prevent_default(el, &event);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomSetStream => {
            let el = dom_handle(args.remove(0), "setStream")?;
            let stream = match args.remove(0) {
                RuntimeValue::Int(n) => n,
                other => {
                    return Err(format!(
                        "setStream expects an Int media-stream handle, got {}",
                        other.type_name()
                    ))
                }
            };
            crate::semantics::dom::set_stream(el, stream);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::RtcPeerConnection => Ok(RuntimeValue::Int(
            crate::semantics::dom::rtc_peer_connection(),
        )),
        BuiltinId::RtcDataChannel => {
            let pc = dom_handle(args.remove(0), "dataChannel")?;
            let label = args.remove(0);
            Ok(RuntimeValue::Int(crate::semantics::dom::rtc_data_channel(
                pc,
                &label.to_display_string(),
            )))
        }
        BuiltinId::RtcCreateOffer => {
            let pc = dom_handle(args.remove(0), "createOffer")?;
            Ok(RuntimeValue::Text(Rc::new(
                crate::semantics::dom::rtc_create_offer(pc),
            )))
        }
        BuiltinId::RtcCreateAnswer => {
            let pc = dom_handle(args.remove(0), "createAnswer")?;
            Ok(RuntimeValue::Text(Rc::new(
                crate::semantics::dom::rtc_create_answer(pc),
            )))
        }
        BuiltinId::RtcSetLocal => {
            let pc = dom_handle(args.remove(0), "setLocalDescription")?;
            let sdp = args.remove(0);
            crate::semantics::dom::rtc_set_local(pc, &sdp.to_display_string());
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::RtcSetRemote => {
            let pc = dom_handle(args.remove(0), "setRemoteDescription")?;
            let sdp = args.remove(0);
            crate::semantics::dom::rtc_set_remote(pc, &sdp.to_display_string());
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::RtcAddIce => {
            let pc = dom_handle(args.remove(0), "addIceCandidate")?;
            let candidate = args.remove(0);
            crate::semantics::dom::rtc_add_ice(pc, &candidate.to_display_string());
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::RtcSend => {
            let channel = dom_handle(args.remove(0), "channelSend")?;
            let data = args.remove(0);
            let bytes = byte_seq(&data)?.len() as u64;
            crate::semantics::dom::rtc_send(channel, bytes);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomQuerySelector => {
            let sel = args.remove(0);
            Ok(RuntimeValue::Int(crate::semantics::dom::query_selector(
                &sel.to_display_string(),
            )))
        }
        BuiltinId::DomAddEventListener => {
            let el = dom_handle(args.remove(0), "addEventListener")?;
            let event = args.remove(0).to_display_string();
            let handler = args.remove(0); // the LOGOS closure
            crate::semantics::dom::add_event_listener(el, &event, handler);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomEventValue => {
            let _event = args.remove(0);
            Ok(RuntimeValue::Text(Rc::new(
                crate::semantics::dom::event_value(),
            )))
        }
        BuiltinId::DomEventKey => {
            let _event = args.remove(0);
            Ok(RuntimeValue::Text(Rc::new(
                crate::semantics::dom::event_key(),
            )))
        }
        BuiltinId::DomEventTarget => {
            let _event = args.remove(0);
            Ok(RuntimeValue::Int(crate::semantics::dom::event_target()))
        }
        BuiltinId::DomFetchText => {
            let url = args.remove(0).to_display_string();
            let on_done = args.remove(0); // the completion closure
            crate::semantics::dom::fetch_start(&url, on_done);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomResponseText => Ok(RuntimeValue::Text(Rc::new(
            crate::semantics::dom::response_text(),
        ))),
        BuiltinId::DomFetchSend => {
            let method = args.remove(0).to_display_string();
            let url = args.remove(0).to_display_string();
            let body = args.remove(0).to_display_string();
            let headers = fetch_header_pairs(&args.remove(0))?;
            let on_done = args.remove(0); // the completion closure
            Ok(RuntimeValue::Int(crate::semantics::dom::fetch_send(
                &method, &url, &body, headers, on_done,
            )))
        }
        BuiltinId::DomResponseStatus => {
            let req = dom_handle(args.remove(0), "responseStatus")?;
            Ok(RuntimeValue::Int(crate::semantics::dom::response_status(
                req,
            )))
        }
        BuiltinId::DomResponseBody => {
            let req = dom_handle(args.remove(0), "responseBody")?;
            Ok(RuntimeValue::Text(Rc::new(
                crate::semantics::dom::response_body(req),
            )))
        }
        BuiltinId::DomResponseHeader => {
            let req = dom_handle(args.remove(0), "responseHeader")?;
            let name = args.remove(0).to_display_string();
            Ok(RuntimeValue::Text(Rc::new(
                crate::semantics::dom::response_header(req, &name),
            )))
        }
        BuiltinId::DomResponseError => {
            let req = dom_handle(args.remove(0), "responseError")?;
            Ok(RuntimeValue::Text(Rc::new(
                crate::semantics::dom::response_error(req),
            )))
        }
        BuiltinId::DomFetchAbort => {
            let req = dom_handle(args.remove(0), "fetchAbort")?;
            crate::semantics::dom::fetch_abort(req);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomFetchSetTimeout => {
            let req = dom_handle(args.remove(0), "fetchSetTimeout")?;
            let millis = dom_handle(args.remove(0), "fetchSetTimeout")?;
            crate::semantics::dom::fetch_set_timeout(req, millis);
            Ok(RuntimeValue::Nothing)
        }
        // --- The history/location router. `routePath`/`routeQuery` read the location;
        // `navigate`/`replaceRoute` record a navigation; `onRoute` stores the popstate
        // handler closure (fired on command by the recorder's `deliver_pop`). ---
        BuiltinId::DomRoutePath => Ok(RuntimeValue::Text(Rc::new(
            crate::semantics::dom::route_path(),
        ))),
        BuiltinId::DomRouteQuery => {
            let name = args.remove(0).to_display_string();
            Ok(RuntimeValue::Text(Rc::new(
                crate::semantics::dom::route_query(&name),
            )))
        }
        BuiltinId::DomClipboardWrite => {
            let t = args.remove(0).to_display_string();
            crate::semantics::dom::write_clipboard(&t);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomDownloadFile => {
            let content = args.remove(0).to_display_string();
            let name = args.remove(0).to_display_string();
            crate::semantics::dom::offer_download(&name, &content);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomNotify => {
            let message = args.remove(0).to_display_string();
            let tone = args.remove(0).to_display_string();
            crate::semantics::dom::notify(&message, &tone);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomPublish => {
            let topic = args.remove(0).to_display_string();
            let message = args.remove(0).to_display_string();
            crate::semantics::dom::publish(&topic, &message);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomElementCommand => {
            let el = dom_handle(args.remove(0), "elementCommand")?;
            let command = args.remove(0).to_display_string();
            // The in-process tiers hold the focus state the reference tier holds, so a page that
            // asks "who has the cursor" gets the same answer whichever tier ran it. The rest are
            // things only a real viewport can do; recording them is what a recorder is for.
            match command.as_str() {
                "focus" => crate::semantics::dom::element_focus(el),
                "blur" => crate::semantics::dom::element_blur(el),
                "click" => crate::semantics::dom::element_click(el),
                "scrollto" => crate::semantics::dom::scroll_to(el),
                "selecttext" => crate::semantics::dom::select_text(el),
                _ => {}
            }
            Ok(RuntimeValue::Nothing)
        }
        // The file the drop being delivered carries. A browser answers from the drop event itself
        // (`dom_event_file_*`, `runtime.js`, which reads the file's content BEFORE the turn runs);
        // an in-process tier answers from the file the host that delivered the drop handed over
        // (`semantics::dom::set_dropped_file`) — never a literal.
        BuiltinId::DomEventFileName => {
            let _ev = args.remove(0);
            Ok(RuntimeValue::Text(Rc::new(
                crate::semantics::dom::dropped_file().name,
            )))
        }
        BuiltinId::DomEventFileSize => {
            let _ev = args.remove(0);
            Ok(RuntimeValue::Int(
                crate::semantics::dom::dropped_file().size(),
            ))
        }
        BuiltinId::DomEventFileType => {
            let _ev = args.remove(0);
            Ok(RuntimeValue::Text(Rc::new(
                crate::semantics::dom::dropped_file()
                    .mime_type()
                    .to_string(),
            )))
        }
        BuiltinId::DomEventFileBytes => {
            let _ev = args.remove(0);
            Ok(bytes_to_seq(&crate::semantics::dom::dropped_file().bytes))
        }
        BuiltinId::DomListenFlags => {
            let el = dom_handle(args.remove(0), "listenFlags")?;
            let event = args.remove(0).to_display_string();
            let flags = match args.remove(0) {
                RuntimeValue::Int(n) if (0..32).contains(&n) => n as u8,
                RuntimeValue::Int(n) => {
                    return Err(format!(
                    "listenFlags takes flags from 0 to 31 (prevent 1, stop 2, once 4, capture 8, \
                         passive 16), got {n}"
                ))
                }
                other => {
                    return Err(format!(
                        "listenFlags takes its flags as an Int, got {}",
                        other.type_name()
                    ))
                }
            };
            crate::semantics::dom::listen(el, &event, flags);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomAssignLocation => {
            let url = args.remove(0).to_display_string();
            crate::semantics::dom::request_load(&url);
            Ok(RuntimeValue::Nothing)
        }
        // Every in-process host keeps the journal (`world::journal`), so its detail is wanted.
        BuiltinId::DomJournalDetail => Ok(RuntimeValue::Bool(true)),
        // An in-process host moves the page's clock only by `Advance time by …`, which calls
        // `__lgx_tick` itself; there is no wall clock to wake the page from.
        BuiltinId::DomWakeAfter => Ok(RuntimeValue::Nothing),
        BuiltinId::DomEnterValue => {
            let verb = match args.remove(0).to_display_string().as_str() {
                "type" => crate::semantics::drive::Verb::Type,
                "choose" => crate::semantics::drive::Verb::Choose,
                other => {
                    return Err(format!(
                        "enterValue: `{other}` is not a verb that enters a value"
                    ))
                }
            };
            let selector = args.remove(0).to_display_string();
            let value = args.remove(0).to_display_string();
            let takes = crate::semantics::dom::answers_by_itself(verb, &selector, &value);
            crate::semantics::dom::enter_value(verb, &selector, &value);
            Ok(RuntimeValue::Bool(takes))
        }
        // An entry calls back into the program, which only the engine running it can do
        // (`Vm::dispatch_entry`); a table of pure builtins has no program to call.
        BuiltinId::DomDispatchEntry => Err(
            "dispatchEntry: the engine running the page takes the entry; it has no pure form"
                .to_string(),
        ),
        BuiltinId::DomSetInterval => {
            // The interval is the HOST's to fire, as a browser's timer is: recorded here with its
            // period, fired by whoever drives the page when its virtual clock moves
            // (`page_host`'s `Advance time by …`). A tier that fires nothing leaves it recorded
            // and unread.
            let ms = match args.remove(0) {
                RuntimeValue::Int(n) => n,
                other => {
                    return Err(format!(
                        "setInterval takes a period in milliseconds, got {}",
                        other.type_name()
                    ))
                }
            };
            let handler = args.remove(0);
            crate::semantics::drive::arm_interval(ms, handler);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomScrollGap => {
            // A tier with no viewport has nothing below the fold: `0` says "already at the
            // bottom", which is the only honest answer where there is no scrolling at all.
            let _el = args.remove(0);
            Ok(RuntimeValue::Int(0))
        }
        BuiltinId::DomOnWindowEvent => {
            // ★ ON THE IN-PROCESS TIERS THERE IS NO WINDOW, so the handler is registered against
            // the page node — which is exactly where `Raise "offline".` delivers. A browser
            // registers the same handler on `window`, where the event really happens.
            let event = args.remove(0).to_display_string();
            let handler = args.remove(0);
            crate::semantics::dom::add_event_listener(
                crate::semantics::dom::body(),
                &event,
                handler,
            );
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomGetAttribute => {
            let el = dom_handle(args.remove(0), "getAttribute")?;
            let name = args.remove(0).to_display_string();
            // An element with no such attribute reads as EMPTY, never a trap: `Add class` asks
            // every element for its class list, and most elements start without one.
            let v = crate::semantics::dom::attribute(el, &name).unwrap_or_default();
            Ok(RuntimeValue::Text(Rc::new(v)))
        }
        BuiltinId::DomGoBack => {
            // `history.back()`: the location steps back, and the `popstate` that raises is the
            // host's to deliver to every route handler once this turn is done
            // (`semantics::dom::take_popped`), as a browser raises it.
            let _ = crate::semantics::dom::go_back();
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomReload => {
            // `location.reload()` is a request: the host loads the page again once the running
            // turn is done (`semantics::dom::take_reload_request`), as a browser does.
            crate::semantics::dom::request_reload();
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::NoteTurn => {
            let action = args.remove(0).to_display_string();
            let instance = args.remove(0).to_display_string();
            let changes = journal_changes(&args.remove(0))?;
            let failure = args.remove(0).to_display_string();
            crate::semantics::world::journal_push(crate::semantics::world::JournalEntry {
                instance,
                action,
                changes,
                failed: (!failure.is_empty()).then_some(failure),
            });
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomSetTitle => {
            let t = args.remove(0).to_display_string();
            crate::semantics::dom::set_title(&t);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomSetHeadHtml => {
            let t = args.remove(0).to_display_string();
            crate::semantics::dom::set_head_html(&t);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomNavigate => {
            let path = args.remove(0).to_display_string();
            crate::semantics::dom::navigate(&path);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomReplaceRoute => {
            let path = args.remove(0).to_display_string();
            crate::semantics::dom::replace_route(&path);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::DomOnRoute => {
            let handler = args.remove(0); // the LOGOS closure
            crate::semantics::dom::on_route(handler);
            Ok(RuntimeValue::Nothing)
        }
        BuiltinId::ReadBytes => {
            let path = args.remove(0).to_display_string();
            match std::fs::read(&path) {
                Ok(bytes) => Ok(bytes_to_seq(&bytes)),
                Err(e) => Err(logicaffeine_data::failure::cannot_read_bytes(&path, e)),
            }
        }
        // ★ THE SAME READ, ANSWERED INSTEAD OF THROWN. One `match` arm apart from `FileRead`,
        // which has always been total — the asymmetry is the whole bug: a program serving files
        // had a recoverable reader for TEXT and an aborting one for BYTES, so `static.lg` probed
        // with the first before calling the second and carried a race between them.
        BuiltinId::ReadReals => {
            let path = args.remove(0).to_display_string();
            let reals = logicaffeine_system::file::read_reals(path)?;
            Ok(RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(reals)))))
        }
        BuiltinId::TryReadBytes => {
            let path = args.remove(0).to_display_string();
            Ok(match logicaffeine_system::file::read_bytes(path) {
                Ok(bytes) => result_ok(bytes_to_seq(&bytes)),
                Err(e) => result_err(e),
            })
        }
        BuiltinId::FileRead => {
            let path = match args.remove(0) {
                RuntimeValue::Text(t) => t.to_string(),
                other => return Err(format!("read needs a Text path, got {}", other.type_name())),
            };
            Ok(match logicaffeine_system::file::read(path) {
                Ok(content) => result_ok(RuntimeValue::Text(Rc::new(content))),
                Err(e) => result_err(e),
            })
        }
        BuiltinId::FileWrite => {
            let content = match args.remove(1) {
                RuntimeValue::Text(t) => t.to_string(),
                other => {
                    return Err(format!(
                        "write needs Text content, got {}",
                        other.type_name()
                    ))
                }
            };
            let path = match args.remove(0) {
                RuntimeValue::Text(t) => t.to_string(),
                other => {
                    return Err(format!(
                        "write needs a Text path, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(match logicaffeine_system::file::write(path, content) {
                Ok(()) => result_ok(RuntimeValue::Nothing),
                Err(e) => result_err(e),
            })
        }
        // `Stmt::ReadFrom`'s VM compiler arm — the SAME sandboxed VFS the tree-walker's
        // `ReadFrom` reads through (`ui_bridge::run_treewalker`'s `Interpreter::with_vfs`), so a
        // program that never mounted a real filesystem sees the identical "Not found" on both
        // tiers. `MemVfs` has no await point, so its future is complete on its first poll — taken
        // with `now_or_never`, never a nested executor (the VM's own runner is already one).
        BuiltinId::VfsReadFile => {
            use logicaffeine_system::fs::Vfs as _;
            let path = args.remove(0).to_display_string();
            sandboxed(logicaffeine_system::fs::MemVfs::new().read_to_string(&path))?
                .map(|content| RuntimeValue::Text(Rc::new(content)))
                .map_err(|e| vfs_read_failure(&e))
        }
        // `Stmt::WriteFile`'s VM compiler arm — the same VFS, so a file written on the VM is what
        // either tier reads back. Content and path are DISPLAYED, as the tree-walker's arm
        // displays them: a number writes as its text.
        BuiltinId::VfsWriteFile => {
            use logicaffeine_system::fs::Vfs as _;
            let path = args.remove(1).to_display_string();
            let content = args.remove(0).to_display_string();
            sandboxed(logicaffeine_system::fs::MemVfs::new().write(&path, content.as_bytes()))?
                .map(|()| RuntimeValue::Nothing)
                .map_err(|e| vfs_write_failure(&e))
        }
        // `Stmt::Mount`'s VM compiler arm — the journal's text, or empty Text when the file is
        // not there yet, exactly as the tree-walker's `Mount` arm binds it.
        BuiltinId::VfsMount => {
            use logicaffeine_system::fs::Vfs as _;
            let path = args.remove(0).to_display_string();
            let content = sandboxed(logicaffeine_system::fs::MemVfs::new().read_to_string(&path))?
                .unwrap_or_default();
            Ok(RuntimeValue::Text(Rc::new(content)))
        }
        BuiltinId::RandomInt => {
            let hi = match args.remove(1) {
                RuntimeValue::Int(n) => n,
                other => {
                    return Err(format!(
                        "randomInt needs Int bounds, got {}",
                        other.type_name()
                    ))
                }
            };
            let lo = match args.remove(0) {
                RuntimeValue::Int(n) => n,
                other => {
                    return Err(format!(
                        "randomInt needs Int bounds, got {}",
                        other.type_name()
                    ))
                }
            };
            random_int(lo, hi).map(RuntimeValue::Int)
        }
        BuiltinId::RandomFloat => Ok(RuntimeValue::Float(
            logicaffeine_system::random::randomFloat(),
        )),
        BuiltinId::EnvGet => {
            let key = match args.remove(0) {
                RuntimeValue::Text(t) => t.to_string(),
                other => return Err(format!("get needs a Text name, got {}", other.type_name())),
            };
            Ok(match environment_variable(&key) {
                Some(v) => crate::interpreter::option_some(RuntimeValue::Text(Rc::new(v))),
                None => crate::interpreter::option_none(),
            })
        }
        BuiltinId::TimeNow => Ok(RuntimeValue::Int(logicaffeine_system::time::now() as i64)),
        BuiltinId::EnvHas | BuiltinId::EnvValue => {
            let key = match args.remove(0) {
                RuntimeValue::Text(t) => t.to_string(),
                other => return Err(format!("get needs a Text name, got {}", other.type_name())),
            };
            let value = environment_variable(&key);
            Ok(if id == BuiltinId::EnvHas {
                RuntimeValue::Bool(value.is_some())
            } else {
                RuntimeValue::Text(Rc::new(value.unwrap_or_default()))
            })
        }
        // The reactive-signal dialect is retired: `builtin_from_name` maps no source spelling to
        // any of these fifteen ids any more, so no program written today can reach this arm. The
        // variants themselves stay — deleting one would renumber every `BuiltinId` bincode encodes
        // after it (`builtin_wire_id_lock`, position-keyed) — kept wire-frozen, never a silent no-op.
        BuiltinId::SignalNew
        | BuiltinId::SignalSet
        | BuiltinId::SignalGetInt
        | BuiltinId::SignalGetText
        | BuiltinId::SignalBeginRender
        | BuiltinId::SignalNewText
        | BuiltinId::SignalSetText
        | BuiltinId::SignalEnterComponent
        | BuiltinId::SignalExitComponent
        | BuiltinId::SignalNewSeq
        | BuiltinId::SignalGetSeq
        | BuiltinId::SignalSetSeq
        | BuiltinId::SignalNewFloat
        | BuiltinId::SignalGetFloat
        | BuiltinId::SignalSetFloat => Err(
            "the reactive-signal dialect is retired; this builtin id is wire-frozen only"
                .to_string(),
        ),
    }
}

/// Extract an Element/Event handle (an `Int`) from a DOM builtin argument.
fn dom_handle(v: RuntimeValue, what: &str) -> Result<i64, String> {
    match v {
        RuntimeValue::Int(h) => Ok(h),
        other => Err(format!(
            "{what} requires an Element handle, got {}",
            other.type_name()
        )),
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/builtins/tests.rs"]
mod tests;

/// What calling a builtin can do beyond computing its result — the fact the effect analysis
/// (`optimize::effects`) needs to reason about a call it cannot look inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinEffect {
    /// A total function of its arguments: it always returns, the same answer for the same input.
    Pure,
    /// Deterministic, but it can stop the program (bad input, a failed lookup, `Fail with`).
    Fallible,
    /// Its answer is not a function of its arguments (a random draw).
    Nondeterministic,
    /// It touches the world: files, the page, storage, the network, shared host state.
    Effectful,
}

impl BuiltinId {
    /// This builtin's [`BuiltinEffect`]. Exhaustive with no wildcard: a new builtin does not compile
    /// until its effect is decided here. When in doubt a builtin is `Fallible` or `Effectful` — a
    /// builtin wrongly called `Pure` could be reordered or removed.
    pub fn effect(self) -> BuiltinEffect {
        use BuiltinId as B;
        match self {
            B::Length | B::Contains | B::Format | B::Abs | B::Sqrt | B::Min | B::Max | B::Complex
            | B::Conjugate | B::RealPart | B::ImagPart | B::UuidNil | B::UuidMax | B::UuidVersion
            | B::UuidDns | B::UuidUrl | B::UuidOid | B::UuidX500 | B::TextBytes | B::UuidBytes
            | B::FormatTimestamp | B::YearOf | B::MonthOf | B::DayOf | B::WeekdayOf | B::HourOf
            | B::MinuteOf | B::SecondOf | B::WeekOf | B::QuarterOf | B::DateOf | B::TimeOf
            | B::SecondsBetween | B::MonthsBetween | B::YearsBetween | B::AddSeconds | B::Copy
            | B::CountOnes | B::LeadingZeros | B::TrailingZeros | B::Word32 | B::Word64 | B::Rotl
            | B::Rotr | B::Wand | B::Wor | B::Wnot | B::Lanes4Word32Make | B::SeqOfLanes4W32
            | B::Sha1Rnds4 | B::Sha1Msg1 | B::Sha1Msg2 | B::Sha1Nexte | B::SeqOfLanes16W8
            | B::Splat16Word8 | B::Shuffle16 | B::ShrBytes16 | B::InterleaveLo16 | B::InterleaveHi16
            | B::ByteAdd16 | B::Maddubs16 | B::Packus16 | B::SeqOfLanes8 | B::Splat8Word32
            | B::IntOfWord32 | B::IntOfWord64 | B::Word64Shl | B::Word64Shr | B::Word64And
            | B::Word32Shr | B::Word16Make | B::IntOfWord16 | B::Word8Make | B::IntOfWord8 | B::NativeKernel | B::SeqOfLanes4 | B::Mul32x32To64
            | B::HsumLanes4 | B::Splat4Word64 | B::AndNot4 | B::SeqOfLanes16 | B::Splat16Word16
            | B::Mulhi16 | B::Montmul32 | B::NttBcastLo | B::NttBcastHi | B::NttBlend | B::MapOf
            | B::SetOf | B::Provisional | B::StateSize | B::ExactPi | B::ExactE | B::ExactOmega => {
                BuiltinEffect::Pure
            }
            B::Infinitesimal | B::OrderedInfinitesimal | B::StandardPart | B::IsInfinitesimal | B::IsLimited
            | B::InfinitelyClose | B::GeneratorOf | B::LimitOf | B::AtTheLimit | B::EveryNatural
            | B::TripCount | B::AffinePower | B::CycleAt | B::SearchCandidates | B::PolynomialFlow | B::HybridFlow
            | B::FirstExit
            | B::DefinedReal
            | B::ExactSqrt | B::ExactExp | B::ExactLn | B::ExactPower | B::ParseInt | B::ParseFloat | B::Chr | B::Floor | B::Ceil | B::Round | B::Pow
            | B::Decimal | B::Modular | B::Quantity | B::Money | B::ToCurrency | B::Convert
            | B::Uuid | B::TextFromBytes | B::WireBytes | B::CompressBytes | B::DecompressBytes
            | B::SealBytes | B::OpenBytes | B::FromWireBytes | B::SealFrame | B::OpenFrame
            | B::OpenBox | B::OpenValueBox | B::UuidFromBytes | B::ParseTimestamp | B::MakeDate | B::MakeMoment | B::LocalInstant
            | B::InZone | B::RunAccepted | B::Lanes4Of | B::Lanes16Word8Make | B::Lanes8Word32
            | B::Lanes4Word64 | B::Lanes16Word16 | B::RepeatSeq | B::ReedSolomonEncode
            | B::ReedSolomonRecover | B::FecProtect | B::FecRestore | B::Fail | B::FailReason
            | B::InspectUnhandledDynamic => BuiltinEffect::Fallible,
            B::RandomInt | B::RandomFloat | B::TimeNow | B::SealBox | B::SealValueBox => {
                BuiltinEffect::Nondeterministic
            }
            B::SetRate | B::SetRates | B::ReadWireProgram | B::WriteWireResidual | B::FileRead
            | B::FileWrite | B::EnvGet | B::EnvHas | B::EnvValue | B::VfsReadFile | B::VfsWriteFile | B::VfsMount | B::ReadBytes | B::TryReadBytes | B::ReadReals | B::StorageSet | B::StorageGet
            | B::StorageGetOr | B::StorageRemove | B::DomBody | B::DomCreateElement
            | B::DomCreateTextNode | B::DomSetText | B::DomSetHtml | B::DomClearChildren
            | B::DomOnMessage | B::DomMoveChildren | B::DomBeginPaint | B::DomSetAttribute
            | B::DomAppendChild | B::DomGetText | B::DomGetValue | B::DomSetValue
            | B::DomApplyPatches | B::DomApplyWords | B::DomWakeAfter | B::DomEnterValue | B::DomDispatchEntry | B::DomSetChecked | B::DomPreventDefault | B::DomSetStream
            | B::RtcPeerConnection | B::RtcDataChannel | B::RtcCreateOffer | B::RtcCreateAnswer
            | B::RtcSetLocal | B::RtcSetRemote | B::RtcAddIce | B::RtcSend | B::DomQuerySelector
            | B::DomAddEventListener | B::DomEventValue | B::DomEventKey | B::DomEventTarget
            | B::DomFetchText | B::DomResponseText
            | B::DomFetchSend | B::DomResponseStatus | B::DomResponseBody | B::DomResponseHeader
            | B::DomResponseError | B::DomFetchAbort | B::DomFetchSetTimeout | B::DomRoutePath
            | B::DomRouteQuery | B::DomNavigate | B::DomReplaceRoute | B::DomSetTitle | B::DomSetHeadHtml
            | B::DomClipboardWrite | B::DomDownloadFile | B::DomNotify | B::DomPublish
            | B::DomElementCommand | B::DomEventFileName | B::DomEventFileSize | B::DomEventFileType
            | B::DomEventFileBytes | B::DomListenFlags | B::DomAssignLocation | B::DomJournalDetail
            | B::DomSetInterval | B::DomScrollGap
            | B::DomOnWindowEvent | B::DomGetAttribute | B::DomGoBack | B::DomReload | B::DomOnRoute
            | B::NoteTurn
            // The reactive-signal dialect is retired (see `execute_builtin`); wire-frozen only.
            | B::SignalNew | B::SignalSet | B::SignalGetInt | B::SignalGetText | B::SignalBeginRender
            | B::SignalNewText | B::SignalSetText | B::SignalEnterComponent | B::SignalExitComponent
            | B::SignalNewSeq | B::SignalGetSeq | B::SignalSetSeq | B::SignalNewFloat
            | B::SignalGetFloat | B::SignalSetFloat => BuiltinEffect::Effectful,
        }
    }
}

/// The text a broken contract reports: its `, otherwise` reason, which must be Text. Shared by the
/// tree-walker and the VM so both refuse a non-Text reason with the same words.
pub fn contract_reason_text(reason: RuntimeValue) -> Result<String, String> {
    match reason {
        RuntimeValue::Text(t) => Ok(t.to_string()),
        other => Err(format!(
            "The reason after `otherwise` must be Text, but it was {}.",
            other.type_name()
        )),
    }
}

/// The loud message for a non-exhaustive `Inspect`: no `When` arm matched the scrutinee's ACTUAL
/// variant and there was no `Otherwise` — gauntlet finding G5. Shared by the tree-walker and the VM
/// so a mistyped arm (`When Err (message):` against a `JsonResult`, whose real variant is `JErr`)
/// fails with the identical words wherever it runs, naming the value it actually got rather than the
/// value-agnostic "the value" this message used to say before either tier could reach the runtime
/// value that trapped it (`current_site`/`Stmt::Site` already gives both tiers the sentence's own
/// location on top of this, through the ordinary runtime-failure path — this message is only the
/// "what", not the "where").
pub fn inspect_unhandled_message(type_name: &str, variant_name: &str) -> String {
    format!(
        "Inspect has no arm for `{variant_name}` (a `{type_name}`) and no Otherwise (matches must \
         be exhaustive) — add a `When {variant_name} (…):` arm, or an `Otherwise:` to cover it."
    )
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/builtins/builtin_effect_tests.rs"]
mod builtin_effect_tests;
