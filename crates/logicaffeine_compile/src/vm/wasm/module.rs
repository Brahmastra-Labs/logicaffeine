//! The direct AOT backend: a whole [`CompiledProgram`] → one self-contained `.wasm` module,
//! with no rustc/cargo/wasm-bindgen.
//!
//! The module exports `main` (the synthesized top-level body) and one wasm function per user
//! function, and imports only the host's `env.print_*` output sinks. Function index space is
//! `[imports 0..K][main = K][functions[i] = K+1+i]`, so `Op::Call { func }` lowers to a plain
//! `call (K+1+func)`. The per-function body reuses the shared dispatch-loop lowering
//! ([`super::cfg`]); types come from [`super::kind`] (the bytecode's arithmetic/`Show` are
//! runtime-polymorphic, so the AOT path infers them statically).
//!
//! Everything outside the supported scalar fragment is rejected with [`WasmLowerError`] — the
//! backend is total on what it accepts and never miscompiles; the corpus lock turns each
//! refusal into a tracked, shrinking gap rather than a silent skip.

use super::cfg::{assemble_dispatch_loop, Blocks};
use super::encode::*;
use super::kind::{self, FieldLayout, FieldNested, Kind, KindTable, ParamShape};
use super::regsplit;
use super::WasmLowerError;
use crate::semantics::builtins::BuiltinId;
use crate::vm::instruction::{BoundaryType, CompiledFunction, CompiledProgram, Constant, EnumTypeDef, Op, StructTypeDef};
use crate::Interner;
use logicaffeine_language::analysis::{PolicyCondition, PolicyRegistry};

type R<T> = std::result::Result<T, WasmLowerError>;

/// Initial linear-memory size, in 64 KiB pages, for a heap-using module (4 MiB). The bump allocator
/// never frees, so this is the working-set ceiling; 4 MiB comfortably holds the build-then-scan
/// programs (n-element arrays, `+`-built strings, per-index Text cuts) at realistic sizes.
const HEAP_PAGES: u32 = 64;

/// Reserved linear-memory slot (an `i32`) holding the offline net-inbox FIFO handle — lives in the
/// null-reserved low-16 region (bytes 0..16 are never bump-allocated), so it never collides with the
/// heap. `Listen` writes the handle here; `Send`/`Stream`/`Await` read it (see the net-op lowerings).
const NET_INBOX_ADDR: i32 = 8;

/// A host function the emitted module imports (under the `env` namespace) to display a `Show`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HostFn {
    PrintI64,
    PrintBool,
    /// `print_char(code: i64)` — display one Unicode scalar. The host reconstructs `char` from the
    /// code point and emits its UTF-8 bytes (a `RuntimeValue::Char` displays as the character
    /// itself), so `Show \`a\`` prints `a`, not the numeric code point.
    PrintChar,
    PrintF64,
    /// `print_date(days: i32)` / `print_moment(nanos: i64)` — display a temporal value (the host
    /// formats via `RuntimeValue::Date`/`Moment`'s `to_display_string`, so no reimplementation).
    PrintDate,
    PrintMoment,
    /// `print_duration(nanos: i64)` / `print_time(nanos: i64)` — display a `RuntimeValue::Duration`
    /// (magnitude-bucketed `5s`/`3h`/…) / `Time` (`HH:MM:SS[.frac]`), host-formatted like the VM.
    PrintDuration,
    PrintTime,
    /// `pow_ff(base: f64, exp: f64) -> f64` — `f64::powf`, for a Float exponent.
    PowFf,
    /// `pow_fi(base: f64, exp: i64) -> f64` — `f64::powi`, for an Int exponent (exact repeated
    /// multiplication, NOT `powf` — the two differ in the last bit).
    PowFi,
    /// `today() -> i32` (days since epoch) / `now() -> i64` (nanos since epoch) — the clock,
    /// honoring the test fixed-clock so tw==vm==wasm.
    Today,
    Now,
    /// `print_seq_i64(handle: i32)` / `print_seq_f64(handle: i32)` — display a whole sequence. The
    /// host reads the stable header `[len][cap][data_ptr]` + the element buffer out of the
    /// exported linear memory and formats `[e0, e1, …]` exactly as `RuntimeValue::List`'s
    /// `to_display_string` (each element by its scalar display), so no format reimplementation.
    PrintSeqI64,
    /// `print_seq_bool(handle: i32)` — like [`PrintSeqI64`] but each i64-0/1 element renders as
    /// `true`/`false` (a `Seq of Bool` = `RuntimeValue::List` of `ListRepr::Bools`), so the whole
    /// sequence displays `[true, false, …]` rather than `[1, 0, …]`.
    PrintSeqBool,
    /// `print_seq_word32(handle: i32)` / `print_seq_word64(handle: i32)` — display a whole `Seq of
    /// Word32`/`Word64` (a crypto state array) as `[u0, u1, …]` with each element its UNSIGNED decimal
    /// (`4294967295`, not `-1`). Word32 elements ride the low word of their 8-byte slot; Word64 the full
    /// slot — matching `RuntimeValue::List` of `Word`.
    PrintSeqWord32,
    PrintSeqWord64,
    PrintSeqF64,
    /// `print_text(handle: i32)` — display a UTF-8 string. The host reads the header `[len][cap]
    /// [data_ptr]` + the `len` bytes at `data_ptr` and emits them verbatim (a `RuntimeValue::Text`
    /// displays as its raw contents), so no formatting reimplementation.
    PrintText,
    /// `print_seq_text(handle: i32)` — display a whole sequence of Text. The host reads the seq
    /// header + each element (a Text handle) and formats `[s0, s1, …]` (the elements unquoted,
    /// comma-space separated), matching `RuntimeValue::List`'s `to_display_string` for Text elements.
    PrintSeqText,
    /// `print_set_i64(handle: i32)` — display a whole `Set of Int` as `{e0, e1, …}`. The VM's `Set`
    /// is an insertion-ordered `Vec` (NOT a hashset), and the AOT set stores elements in that same
    /// order, so the display is deterministic and byte-identical. (A whole `Map` displays the same way
    /// via `lower_show_map`: the VM's `MapStorage` is an insertion-ordered `IndexMap`, matching the
    /// AOT's append order, so `{k: v, …}` is byte-identical too.)
    PrintSetI64,
    /// `print_set_text(handle: i32)` — display a whole `Set of Text` as `{s0, s1, …}` (elements
    /// unquoted, insertion order). Like [`PrintSetI64`] but each slot's low word is a `Text` handle
    /// the host reads out of memory — matching `RuntimeValue::Set::to_display_string` for Text.
    PrintSetText,
    /// `fmt_i64_into(buf: i32, val: i64) -> i32` — write the decimal of `val`
    /// (`RuntimeValue::Int(val).to_display_string()`) into `buf` and return the byte length. Used
    /// to stringify an Int operand of a `Concat` (string interpolation `"… {n} …"`). `buf` is a
    /// module-allocated 24-byte scratch (an i64 decimal is ≤ 20 chars + sign).
    FmtI64Into,
    /// `fmt_f64_into(buf: i32, val: f64) -> i32` — like [`FmtI64Into`] for a Float operand (the
    /// shared shortest-round-trip display, `logicaffeine_data::fmt::fmt_f64`); `buf` is a 340-byte
    /// scratch (the widest possible output, the smallest subnormal, is ~326 bytes — spec-locked
    /// by `fmt::tests::worst_case_width_fits_wasm_scratch`).
    FmtF64Into,
    /// `fmt_bool_into(buf: i32, val: i32) -> i32` — writes `"true"`/`"false"`; `buf` is 8 bytes.
    FmtBoolInto,
    /// `fmt_f64_prec_into(buf: i32, val: f64, prec: i32) -> i32` — write `format!("{:.prec}", val)`
    /// (the interpolation `.N` precision spec `"{x:.9}"` → `apply_format_spec`) and return the length.
    /// `buf` is a `340 + prec`-byte scratch (worst-case `f64` integer width + the decimals).
    FmtF64PrecInto,
    /// `fmt_align_into(buf: i32, text: i32, width: i32, align: i32) -> i32` — pad the `Text` `text`'s
    /// display to `width` (space fill, char-counted) into `buf`, returning the byte length. `align`
    /// selects `format!("{:>w$}", …)` (0, right — also the bare-width `{x:6}`), `{:<w$}` (1, left), or
    /// `{:^w$}` (2, center) — the SAME Rust `format!` `apply_format_spec` runs, so bit-identical. `buf`
    /// is sized `text_len + width` (padding adds at most `width` single-byte spaces).
    FmtAlignInto,
    /// `args() -> i32` — the command-line arguments as a `Seq of Text` handle (the host builds the
    /// argv sequence in the module's linear memory and returns its handle), so a program reading its
    /// problem size from argv (`parseInt(item 2 of args())`) compiles to standalone wasm.
    Args,
    /// `parse_int(handle: i32) -> i64` — parse a `Text` (UTF-8 in linear memory) to an Int, exactly as
    /// the VM's `BuiltinId::ParseInt` (`str::parse::<i64>`), trapping on a non-numeric string.
    ParseInt,
    /// `parse_float(handle: i32) -> f64` — parse a `Text` to a Float, exactly as the VM's
    /// `BuiltinId::ParseFloat` (`str::trim().parse::<f64>`), trapping on a non-numeric string.
    ParseFloat,
    /// `parse_timestamp(handle: i32) -> i64` — parse an RFC-3339 `Text` to a `Moment` (nanoseconds
    /// since the epoch) via `temporal::parse_rfc3339`, trapping on a malformed timestamp.
    ParseTimestamp,
    /// `temporal_component(nanos: i64, which: i32) -> i64` — one calendar/clock component of a `Moment`
    /// (`which`: 0 year, 1 month, 2 day, 3 hour, 4 minute, 5 second, 6 weekday, 7 iso-week, 8 quarter),
    /// computed by the SAME `temporal::civil_from_unix_nanos`/`weekday_from_days`/`iso_week_from_days`
    /// the VM uses — so `the year of m` is bit-identical across tiers.
    TemporalComponent,
    /// `temporal_component_date(days: i32, which: i32) -> i64` — the calendar component of a `Date`
    /// (days since the epoch), `which` in {0 year, 1 month, 2 day, 6 weekday, 7 iso-week, 8 quarter}.
    /// Computed straight from `temporal::civil_from_days`/`weekday_from_days`/`iso_week_from_days` (the
    /// VM's exact Date path) over the full `i32` day range — no `Moment` nanos round-trip (which would
    /// overflow `i64` past ~year 2262). Clock components (hour/minute/second) don't apply to a `Date`
    /// and are refused at lowering, matching the VM's runtime error.
    TemporalComponentDate,
    /// `fmt_seq_i64_into(buf: i32, handle: i32) -> i32` — write a whole `Seq of Int`'s display
    /// `[e0, e1, …]` (`RuntimeValue::List::to_display_string`) into `buf` and return the byte length.
    /// Used to stringify a SEQUENCE operand of a `+`/`format` (`"Positives: " + positives`). `buf` is
    /// sized `len*24 + 8` (each i64 ≤ 20 digits + sign + `", "`, plus the brackets).
    FmtSeqI64Into,
    /// `fmt_seq_bool_into(buf: i32, handle: i32) -> i32` — like [`FmtSeqI64Into`] for a `Seq of Bool`,
    /// rendering each i64-0/1 element as `true`/`false` (`buf` sized `len*7 + 8` — `"false"` + `", "`).
    /// Used to stringify a whole bool sequence operand of a `+`/`format`.
    FmtSeqBoolInto,
    /// `fmt_set_i64_into(buf: i32, handle: i32) -> i32` — like [`FmtSeqI64Into`] for a `Set of Int`,
    /// whose display is `{e0, e1, …}` (insertion order, matching the VM's `Vec`-backed Set).
    FmtSetI64Into,
    /// `print_rational(num: i64, den: i64)` — display an exact `Rational` as `num/den`, or just `num`
    /// when `den == 1` (matching the VM, whose `from_rational` downsizes a whole quotient to an `Int`).
    PrintRational,
    /// `print_nothing()` — display the Optional null value as "nothing" (matching the tree-walker's
    /// `RuntimeValue::Nothing` display). The `Some` arm of an `Optional` `Show` uses the inner scalar's
    /// own sink instead; this covers the null-handle arm.
    PrintNothing,
    /// `print_word(v: i64)` — display a Word as its UNSIGNED decimal (`(v as u64).to_string()`, matching
    /// `WordVal`'s Display). A `Word32` is zero-extended to `i64` (`i64.extend_i32_u`) before the call,
    /// so its top 32 bits are zero and the `u64` reading is exactly the `u32` value.
    PrintWord,
    /// `logos_rt_bigint_from_i64(x: i64) -> i32` — the linked `logicaffeine_base::BigInt` runtime: box a
    /// scalar into a BigInt handle. LINKER MODE ONLY: an undefined symbol `rust-lld` resolves against the
    /// prebuilt base runtime object (there is no `env` host behind it), so it never appears in a
    /// standalone module. The three `Bigint*` sinks together lower an overflowing integer `Op::Pow`.
    BigintFromI64,
    /// `logos_rt_bigint_pow(base: i32, exp: i64) -> i32` — raise a BigInt handle to an integer power,
    /// returning a fresh handle (the exact big integer, no overflow). Linker mode only.
    BigintPow,
    /// `logos_rt_bigint_to_text(h: i32) -> i32` — render a BigInt handle to a `Text` handle laid out in
    /// the emitter's `[len][cap][data_ptr][refcount]` ABI in the shared linear memory, so a `Show` reads
    /// it through the ordinary `print_text` path. Linker mode only.
    BigintToText,
    /// `logos_rt_bigint_mul(a: i32, b: i32) -> i32` — exact big-integer multiply of two BigInt handles,
    /// returning a fresh handle. Lets `(2^100) * (3^50)` keep computing on real BigInts. Linker only.
    BigintMul,
    /// `logos_rt_bigint_add(a: i32, b: i32) -> i32` — exact big-integer addition of two handles. Linker only.
    BigintAdd,
    /// `logos_rt_bigint_sub(a: i32, b: i32) -> i32` — exact big-integer subtraction (may be negative;
    /// `to_text` renders the sign). Linker only.
    BigintSub,
    /// `logos_rt_bigint_div(a: i32, b: i32) -> i32` — exact big-integer quotient (`div_rem().0`, the SAME
    /// truncating division the VM uses), traps on a zero divisor. Linker only.
    BigintDiv,
    /// `logos_rt_bigint_mod(a: i32, b: i32) -> i32` — exact big-integer remainder (`div_rem().1`, matching
    /// the VM's `the remainder of a and b`), traps on a zero divisor. Linker only.
    BigintMod,
    /// `logos_rt_complex_from_i64(re: i64, im: i64) -> i32` — build an EXACT `Complex` (Rational
    /// components) from two integer parts, returning an i32 handle. `logos_rt_complex_{add,sub,mul}(a,
    /// b) -> i32` do exact complex arithmetic; `logos_rt_complex_to_text(h) -> i32` renders it (`re±imi`)
    /// to a Text handle. Linker mode only (the exact Rational-backed runtime, mirroring the BigInt ABI).
    ComplexFromI64,
    ComplexAdd,
    ComplexSub,
    ComplexMul,
    ComplexToText,
    /// `logos_rt_modular_from_i64(v: i64, n: i64) -> i32` / `_add`/`_sub`/`_mul`/`_to_text` — the ℤ/nℤ
    /// analog of the Complex ABI over `logicaffeine_base::Modular` (i32 handle). Linker mode only.
    ModularFromI64,
    ModularAdd,
    ModularSub,
    ModularMul,
    ModularToText,
    /// `logos_rt_decimal_from_text(h)` parses a Text handle → exact Decimal; `_from_i64(x)` promotes an
    /// Int; `_add`/`_sub`/`_mul`/`_to_text` are the exact base-10 ABI over `base::Decimal`. Linker only.
    DecimalFromText,
    DecimalFromI64,
    DecimalAdd,
    DecimalSub,
    DecimalMul,
    DecimalToText,
    /// `logos_rt_money_from_decimal(dec, cur)` / `_from_i64(v, cur)` build a Money (currency Text read
    /// from shared memory); `_add`/`_sub` require matching currency; `_to_text` renders it. Linker only.
    MoneyFromDecimal,
    MoneyFromI64,
    MoneyAdd,
    MoneySub,
    MoneyToText,
    QuantityOfI64,
    QuantityConvert,
    QuantityAdd,
    QuantitySub,
    QuantityMul,
    QuantityDiv,
    QuantityToText,
    RationalFromI64,
    RationalFromBigint,
    RationalAdd,
    RationalSub,
    RationalMul,
    RationalDiv,
    RationalToText,
    RationalFloor,
    RationalCeil,
    RationalRound,
    RationalAbs,
    UuidParse,
    UuidNil,
    UuidMax,
    UuidDns,
    UuidUrl,
    UuidOid,
    UuidX500,
    UuidVersion,
    UuidEq,
    UuidToText,
    UuidFromPtr,
    PrintSpan,
    MomentAddSpan,
    DateAddSpan,
    FormatTimestampRt,
    MonthsBetweenRt,
    YearsBetweenRt,
    InZoneRt,
    LocalInstantRt,
    Lanes16FromBytes,
    Lanes8FromWords,
    Lanes4W64FromWords,
    LanesSplat16,
    LanesSplat8,
    LanesToSeq,
    LanesShuffle,
    LanesInterleaveLo,
    LanesInterleaveHi,
    LanesByteAdd,
    LanesMaddubs,
    LanesPackus,
    LanesShrBytes,
    DecimalToRational,
    MoneySetRate,
    MoneyToCurrency,
    MoneySetRatesInt,
    MoneySetRatesRational,
    MoneySetRatesDecimal,
    WriteWireResidual,
    WireBytesInt,
    WireBytesBool,
    WireBytesFloat,
    WireBytesText,
    ReadWireFrame,
    ReadWireProgramRt,
    DynamicToText,
    RunAccepted,
    Sha1Rnds4,
    Sha1Msg1,
    Sha1Msg2,
    Sha1Nexte,
    Lanes4Add,
    Lanes4Xor,
    /// `logos_rt_alloc(size: i32) -> i32` — a raw 8-aligned block from the runtime's allocator. Linker
    /// mode seeds the emitter's bump allocator (`__heap_ptr`) from ONE such SLAB at a `main` prologue, so
    /// the emitter's heap and the runtime's `dlmalloc` never overlap in the shared linear memory.
    RtAlloc,
}

/// Stable order — also the order host functions take their wasm import indices.
/// A `set_rates(map)`'s VALUE kind — the kind of a `SetIndex` value written into the map, resolved by
/// following `Move` aliases of `target` (the call arg is a Move-copy of the built map register) to any
/// register a `SetIndex` populated. `None` if no populating `SetIndex` is statically visible.
fn set_rates_value_kind(ops: &[Op], target: u16, kinds: &KindTable) -> Option<Kind> {
    let mut aliases = std::collections::HashSet::new();
    aliases.insert(target);
    let mut changed = true;
    while changed {
        changed = false;
        for op in ops {
            if let Op::Move { dst, src } = op {
                if aliases.contains(dst) && aliases.insert(*src) {
                    changed = true;
                }
                if aliases.contains(src) && aliases.insert(*dst) {
                    changed = true;
                }
            }
        }
    }
    ops.iter().find_map(|op| match op {
        Op::SetIndex { collection, value, .. } if aliases.contains(collection) => kinds.get(*value as usize),
        _ => None,
    })
}

/// The `logos_rt_wire_bytes_*` runtime host for `wireBytes(value)` (linker mode) by the ARGUMENT'S kind
/// — each reconstructs the corresponding `RuntimeValue` and marshals it via the REAL codec. `None` for a
/// kind not yet reconstructed (a composite: soundly refused). Shared by the import scan and the lowering.
fn wire_bytes_host_fn(arg_kind: Option<Kind>) -> Option<HostFn> {
    match arg_kind {
        Some(Kind::Int) => Some(HostFn::WireBytesInt),
        Some(Kind::Bool) => Some(HostFn::WireBytesBool),
        Some(Kind::Float) => Some(HostFn::WireBytesFloat),
        Some(Kind::Text) => Some(HostFn::WireBytesText),
        _ => None,
    }
}

/// The `logos_rt_lanes_*` runtime host for a general-`LanesVal` SIMD builtin (linker mode), or `None`
/// if `b` is not one of them. Shared by the import scan and the lowering so both agree on the op→host
/// map. Distinct from the SHA-1 `Kind::Lanes` (inline `Lanes4Word32`) ops, which are their own hosts.
fn lanes_v_host_fn(b: BuiltinId) -> Option<HostFn> {
    Some(match b {
        BuiltinId::Lanes16Word8Make => HostFn::Lanes16FromBytes,
        BuiltinId::Lanes8Word32 => HostFn::Lanes8FromWords,
        BuiltinId::Lanes4Word64 => HostFn::Lanes4W64FromWords,
        BuiltinId::Splat16Word8 => HostFn::LanesSplat16,
        BuiltinId::Splat8Word32 => HostFn::LanesSplat8,
        BuiltinId::SeqOfLanes16W8 | BuiltinId::SeqOfLanes8 => HostFn::LanesToSeq,
        BuiltinId::Shuffle16 => HostFn::LanesShuffle,
        BuiltinId::InterleaveLo16 => HostFn::LanesInterleaveLo,
        BuiltinId::InterleaveHi16 => HostFn::LanesInterleaveHi,
        BuiltinId::ByteAdd16 => HostFn::LanesByteAdd,
        BuiltinId::Maddubs16 => HostFn::LanesMaddubs,
        BuiltinId::Packus16 => HostFn::LanesPackus,
        BuiltinId::ShrBytes16 => HostFn::LanesShrBytes,
        _ => return None,
    })
}

const HOST_FNS: [HostFn; 139] = [
    HostFn::PrintI64,
    HostFn::PrintBool,
    HostFn::PrintChar,
    HostFn::PrintF64,
    HostFn::PrintDate,
    HostFn::PrintMoment,
    HostFn::PrintDuration,
    HostFn::PrintTime,
    HostFn::PowFf,
    HostFn::PowFi,
    HostFn::Today,
    HostFn::Now,
    HostFn::PrintSeqI64,
    HostFn::PrintSeqBool,
    HostFn::PrintSeqWord32,
    HostFn::PrintSeqWord64,
    HostFn::PrintSeqF64,
    HostFn::PrintText,
    HostFn::PrintSeqText,
    HostFn::PrintSetI64,
    HostFn::FmtI64Into,
    HostFn::FmtF64Into,
    HostFn::FmtBoolInto,
    HostFn::FmtF64PrecInto,
    HostFn::FmtAlignInto,
    HostFn::Args,
    HostFn::ParseInt,
    HostFn::FmtSeqI64Into,
    HostFn::FmtSeqBoolInto,
    HostFn::FmtSetI64Into,
    HostFn::PrintSetText,
    HostFn::PrintRational,
    HostFn::PrintNothing,
    HostFn::ParseFloat,
    HostFn::ParseTimestamp,
    HostFn::TemporalComponent,
    HostFn::TemporalComponentDate,
    HostFn::PrintWord,
    HostFn::BigintFromI64,
    HostFn::BigintPow,
    HostFn::BigintToText,
    HostFn::BigintMul,
    HostFn::BigintAdd,
    HostFn::BigintSub,
    HostFn::BigintDiv,
    HostFn::BigintMod,
    HostFn::ComplexFromI64,
    HostFn::ComplexAdd,
    HostFn::ComplexSub,
    HostFn::ComplexMul,
    HostFn::ComplexToText,
    HostFn::ModularFromI64,
    HostFn::ModularAdd,
    HostFn::ModularSub,
    HostFn::ModularMul,
    HostFn::ModularToText,
    HostFn::DecimalFromText,
    HostFn::DecimalFromI64,
    HostFn::DecimalAdd,
    HostFn::DecimalSub,
    HostFn::DecimalMul,
    HostFn::DecimalToText,
    HostFn::MoneyFromDecimal,
    HostFn::MoneyFromI64,
    HostFn::MoneyAdd,
    HostFn::MoneySub,
    HostFn::MoneyToText,
    HostFn::QuantityOfI64,
    HostFn::QuantityConvert,
    HostFn::QuantityAdd,
    HostFn::QuantitySub,
    HostFn::QuantityMul,
    HostFn::QuantityDiv,
    HostFn::QuantityToText,
    HostFn::RationalFromI64,
    HostFn::RationalFromBigint,
    HostFn::RationalAdd,
    HostFn::RationalSub,
    HostFn::RationalMul,
    HostFn::RationalDiv,
    HostFn::RationalToText,
    HostFn::RationalFloor,
    HostFn::RationalCeil,
    HostFn::RationalRound,
    HostFn::RationalAbs,
    HostFn::UuidParse,
    HostFn::UuidNil,
    HostFn::UuidMax,
    HostFn::UuidDns,
    HostFn::UuidUrl,
    HostFn::UuidOid,
    HostFn::UuidX500,
    HostFn::UuidVersion,
    HostFn::UuidEq,
    HostFn::UuidToText,
    HostFn::UuidFromPtr,
    HostFn::PrintSpan,
    HostFn::MomentAddSpan,
    HostFn::DateAddSpan,
    HostFn::FormatTimestampRt,
    HostFn::MonthsBetweenRt,
    HostFn::YearsBetweenRt,
    HostFn::InZoneRt,
    HostFn::LocalInstantRt,
    HostFn::Lanes16FromBytes,
    HostFn::Lanes8FromWords,
    HostFn::Lanes4W64FromWords,
    HostFn::LanesSplat16,
    HostFn::LanesSplat8,
    HostFn::LanesToSeq,
    HostFn::LanesShuffle,
    HostFn::LanesInterleaveLo,
    HostFn::LanesInterleaveHi,
    HostFn::LanesByteAdd,
    HostFn::LanesMaddubs,
    HostFn::LanesPackus,
    HostFn::LanesShrBytes,
    HostFn::DecimalToRational,
    HostFn::MoneySetRate,
    HostFn::MoneyToCurrency,
    HostFn::MoneySetRatesInt,
    HostFn::MoneySetRatesRational,
    HostFn::MoneySetRatesDecimal,
    HostFn::WriteWireResidual,
    HostFn::WireBytesInt,
    HostFn::WireBytesBool,
    HostFn::WireBytesFloat,
    HostFn::WireBytesText,
    HostFn::ReadWireFrame,
    HostFn::ReadWireProgramRt,
    HostFn::DynamicToText,
    HostFn::RunAccepted,
    HostFn::Sha1Rnds4,
    HostFn::Sha1Msg1,
    HostFn::Sha1Msg2,
    HostFn::Sha1Nexte,
    HostFn::Lanes4Add,
    HostFn::Lanes4Xor,
    HostFn::RtAlloc,
];

impl HostFn {
    fn field(self) -> &'static str {
        match self {
            HostFn::PrintI64 => "print_i64",
            HostFn::PrintBool => "print_bool",
            HostFn::PrintChar => "print_char",
            HostFn::PrintF64 => "print_f64",
            HostFn::PrintDate => "print_date",
            HostFn::PrintMoment => "print_moment",
            HostFn::PrintDuration => "print_duration",
            HostFn::PrintTime => "print_time",
            HostFn::PowFf => "pow_ff",
            HostFn::PowFi => "pow_fi",
            HostFn::Today => "today",
            HostFn::Now => "now",
            HostFn::PrintSeqI64 => "print_seq_i64",
            HostFn::PrintSeqBool => "print_seq_bool",
            HostFn::PrintSeqWord32 => "print_seq_word32",
            HostFn::PrintSeqWord64 => "print_seq_word64",
            HostFn::PrintSeqF64 => "print_seq_f64",
            HostFn::PrintText => "print_text",
            HostFn::PrintSeqText => "print_seq_text",
            HostFn::PrintSetI64 => "print_set_i64",
            HostFn::PrintSetText => "print_set_text",
            HostFn::FmtI64Into => "fmt_i64_into",
            HostFn::FmtF64Into => "fmt_f64_into",
            HostFn::FmtBoolInto => "fmt_bool_into",
            HostFn::FmtF64PrecInto => "fmt_f64_prec_into",
            HostFn::FmtAlignInto => "fmt_align_into",
            HostFn::Args => "args",
            HostFn::ParseInt => "parse_int",
            HostFn::ParseFloat => "parse_float",
            HostFn::ParseTimestamp => "parse_timestamp",
            HostFn::TemporalComponent => "temporal_component",
            HostFn::TemporalComponentDate => "temporal_component_date",
            HostFn::PrintWord => "print_word",
            HostFn::FmtSeqI64Into => "fmt_seq_i64_into",
            HostFn::FmtSeqBoolInto => "fmt_seq_bool_into",
            HostFn::FmtSetI64Into => "fmt_set_i64_into",
            HostFn::PrintRational => "print_rational",
            HostFn::PrintNothing => "print_nothing",
            HostFn::BigintFromI64 => "logos_rt_bigint_from_i64",
            HostFn::BigintPow => "logos_rt_bigint_pow",
            HostFn::BigintToText => "logos_rt_bigint_to_text",
            HostFn::BigintMul => "logos_rt_bigint_mul",
            HostFn::BigintAdd => "logos_rt_bigint_add",
            HostFn::BigintSub => "logos_rt_bigint_sub",
            HostFn::BigintDiv => "logos_rt_bigint_div",
            HostFn::BigintMod => "logos_rt_bigint_mod",
            HostFn::ComplexFromI64 => "logos_rt_complex_from_i64",
            HostFn::ComplexAdd => "logos_rt_complex_add",
            HostFn::ComplexSub => "logos_rt_complex_sub",
            HostFn::ComplexMul => "logos_rt_complex_mul",
            HostFn::ComplexToText => "logos_rt_complex_to_text",
            HostFn::ModularFromI64 => "logos_rt_modular_from_i64",
            HostFn::ModularAdd => "logos_rt_modular_add",
            HostFn::ModularSub => "logos_rt_modular_sub",
            HostFn::ModularMul => "logos_rt_modular_mul",
            HostFn::ModularToText => "logos_rt_modular_to_text",
            HostFn::DecimalFromText => "logos_rt_decimal_from_text",
            HostFn::DecimalFromI64 => "logos_rt_decimal_from_i64",
            HostFn::DecimalAdd => "logos_rt_decimal_add",
            HostFn::DecimalSub => "logos_rt_decimal_sub",
            HostFn::DecimalMul => "logos_rt_decimal_mul",
            HostFn::DecimalToText => "logos_rt_decimal_to_text",
            HostFn::MoneyFromDecimal => "logos_rt_money_from_decimal",
            HostFn::MoneyFromI64 => "logos_rt_money_from_i64",
            HostFn::MoneyAdd => "logos_rt_money_add",
            HostFn::MoneySub => "logos_rt_money_sub",
            HostFn::MoneyToText => "logos_rt_money_to_text",
            HostFn::QuantityOfI64 => "logos_rt_quantity_of_i64",
            HostFn::QuantityConvert => "logos_rt_quantity_convert",
            HostFn::QuantityAdd => "logos_rt_quantity_add",
            HostFn::QuantitySub => "logos_rt_quantity_sub",
            HostFn::QuantityMul => "logos_rt_quantity_mul",
            HostFn::QuantityDiv => "logos_rt_quantity_div",
            HostFn::QuantityToText => "logos_rt_quantity_to_text",
            HostFn::RationalFromI64 => "logos_rt_rational_from_i64",
            HostFn::RationalFromBigint => "logos_rt_rational_from_bigint",
            HostFn::RationalAdd => "logos_rt_rational_add",
            HostFn::RationalSub => "logos_rt_rational_sub",
            HostFn::RationalMul => "logos_rt_rational_mul",
            HostFn::RationalDiv => "logos_rt_rational_div",
            HostFn::RationalToText => "logos_rt_rational_to_text",
            HostFn::RationalFloor => "logos_rt_rational_floor",
            HostFn::RationalCeil => "logos_rt_rational_ceil",
            HostFn::RationalRound => "logos_rt_rational_round",
            HostFn::RationalAbs => "logos_rt_rational_abs",
            HostFn::UuidParse => "logos_rt_uuid_parse",
            HostFn::UuidNil => "logos_rt_uuid_nil",
            HostFn::UuidMax => "logos_rt_uuid_max",
            HostFn::UuidDns => "logos_rt_uuid_dns",
            HostFn::UuidUrl => "logos_rt_uuid_url",
            HostFn::UuidOid => "logos_rt_uuid_oid",
            HostFn::UuidX500 => "logos_rt_uuid_x500",
            HostFn::UuidVersion => "logos_rt_uuid_version",
            HostFn::UuidEq => "logos_rt_uuid_eq",
            HostFn::UuidToText => "logos_rt_uuid_to_text",
            HostFn::UuidFromPtr => "logos_rt_uuid_from_ptr",
            HostFn::PrintSpan => "print_span",
            HostFn::MomentAddSpan => "logos_rt_moment_add_span",
            HostFn::DateAddSpan => "logos_rt_date_add_span",
            HostFn::FormatTimestampRt => "logos_rt_format_timestamp",
            HostFn::MonthsBetweenRt => "logos_rt_months_between",
            HostFn::YearsBetweenRt => "logos_rt_years_between",
            HostFn::InZoneRt => "logos_rt_in_zone",
            HostFn::LocalInstantRt => "logos_rt_local_instant",
            HostFn::Lanes16FromBytes => "logos_rt_lanes16_from_bytes",
            HostFn::Lanes8FromWords => "logos_rt_lanes8_from_words",
            HostFn::Lanes4W64FromWords => "logos_rt_lanes4w64_from_words",
            HostFn::LanesSplat16 => "logos_rt_lanes_splat16",
            HostFn::LanesSplat8 => "logos_rt_lanes_splat8",
            HostFn::LanesToSeq => "logos_rt_lanes_to_seq",
            HostFn::LanesShuffle => "logos_rt_lanes_shuffle",
            HostFn::LanesInterleaveLo => "logos_rt_lanes_interleave_lo",
            HostFn::LanesInterleaveHi => "logos_rt_lanes_interleave_hi",
            HostFn::LanesByteAdd => "logos_rt_lanes_byte_add",
            HostFn::LanesMaddubs => "logos_rt_lanes_maddubs",
            HostFn::LanesPackus => "logos_rt_lanes_packus",
            HostFn::LanesShrBytes => "logos_rt_lanes_shr_bytes",
            HostFn::DecimalToRational => "logos_rt_decimal_to_rational",
            HostFn::MoneySetRate => "logos_rt_set_rate",
            HostFn::MoneyToCurrency => "logos_rt_to_currency",
            HostFn::MoneySetRatesInt => "logos_rt_set_rates_int",
            HostFn::MoneySetRatesRational => "logos_rt_set_rates_rational",
            HostFn::MoneySetRatesDecimal => "logos_rt_set_rates_decimal",
            HostFn::WriteWireResidual => "write_wire_residual",
            HostFn::WireBytesInt => "logos_rt_wire_bytes_int",
            HostFn::WireBytesBool => "logos_rt_wire_bytes_bool",
            HostFn::WireBytesFloat => "logos_rt_wire_bytes_float",
            HostFn::WireBytesText => "logos_rt_wire_bytes_text",
            HostFn::ReadWireFrame => "read_wire_frame",
            HostFn::ReadWireProgramRt => "logos_rt_read_wire_program",
            HostFn::DynamicToText => "logos_rt_dynamic_to_text",
            HostFn::RunAccepted => "logos_rt_run_accepted",
            HostFn::Sha1Rnds4 => "logos_rt_sha1rnds4",
            HostFn::Sha1Msg1 => "logos_rt_sha1msg1",
            HostFn::Sha1Msg2 => "logos_rt_sha1msg2",
            HostFn::Sha1Nexte => "logos_rt_sha1nexte",
            HostFn::Lanes4Add => "logos_rt_lanes4_add",
            HostFn::Lanes4Xor => "logos_rt_lanes4_xor",
            HostFn::RtAlloc => "logos_rt_alloc",
        }
    }

    /// The wasm parameter value-types this host function takes.
    fn params(self) -> Vec<u8> {
        match self {
            HostFn::PrintI64 | HostFn::PrintChar => vec![I64],
            HostFn::PrintBool | HostFn::PrintDate => vec![I32],
            HostFn::PrintF64 => vec![F64],
            HostFn::PrintMoment | HostFn::PrintDuration | HostFn::PrintTime | HostFn::PrintSpan => vec![I64],
            HostFn::MomentAddSpan => vec![I64, I32, I32],
            HostFn::DateAddSpan => vec![I32, I32, I32],
            HostFn::FormatTimestampRt => vec![I64],
            HostFn::MonthsBetweenRt | HostFn::YearsBetweenRt => vec![I64, I64],
            HostFn::InZoneRt | HostFn::LocalInstantRt => vec![I64, I32],
            HostFn::Lanes16FromBytes | HostFn::Lanes8FromWords | HostFn::Lanes4W64FromWords | HostFn::LanesToSeq => vec![I32],
            HostFn::LanesSplat16 | HostFn::LanesSplat8 => vec![I64],
            HostFn::LanesShuffle | HostFn::LanesInterleaveLo | HostFn::LanesInterleaveHi | HostFn::LanesByteAdd | HostFn::LanesMaddubs | HostFn::LanesPackus => vec![I32, I32],
            HostFn::LanesShrBytes => vec![I32, I64],
            HostFn::DecimalToRational | HostFn::MoneySetRatesInt | HostFn::MoneySetRatesRational | HostFn::MoneySetRatesDecimal => vec![I32],
            HostFn::MoneySetRate | HostFn::MoneyToCurrency => vec![I32, I32],
            HostFn::WriteWireResidual => vec![I32, I32],
            HostFn::WireBytesInt | HostFn::WireBytesBool => vec![I64],
            HostFn::WireBytesFloat => vec![F64],
            HostFn::WireBytesText => vec![I32],
            HostFn::ReadWireFrame | HostFn::ReadWireProgramRt => vec![I32, I32],
            HostFn::DynamicToText => vec![I32],
            HostFn::RunAccepted => vec![I32, I64, I64, I64],
            HostFn::Sha1Rnds4 => vec![I32, I32, I64],
            HostFn::Sha1Msg1 | HostFn::Sha1Msg2 | HostFn::Sha1Nexte | HostFn::Lanes4Add | HostFn::Lanes4Xor => vec![I32, I32],
            HostFn::PowFf => vec![F64, F64],
            HostFn::PowFi => vec![F64, I64],
            HostFn::Today | HostFn::Now => vec![],
            HostFn::PrintSeqI64 | HostFn::PrintSeqBool | HostFn::PrintSeqWord32 | HostFn::PrintSeqWord64 | HostFn::PrintSeqF64 | HostFn::PrintText | HostFn::PrintSeqText | HostFn::PrintSetI64 | HostFn::PrintSetText => vec![I32],
            HostFn::FmtI64Into => vec![I32, I64],
            HostFn::FmtF64Into => vec![I32, F64],
            HostFn::FmtBoolInto => vec![I32, I64], // a Bool rides an i64 local (0/1)
            HostFn::FmtF64PrecInto => vec![I32, F64, I32],
            HostFn::FmtAlignInto => vec![I32, I32, I32, I32],
            HostFn::FmtSeqI64Into | HostFn::FmtSeqBoolInto | HostFn::FmtSetI64Into => vec![I32, I32],
            HostFn::Args => vec![],
            HostFn::ParseInt | HostFn::ParseFloat | HostFn::ParseTimestamp => vec![I32],
            HostFn::PrintRational => vec![I64, I64],
            HostFn::PrintNothing => vec![],
            HostFn::TemporalComponent => vec![I64, I32],
            HostFn::TemporalComponentDate => vec![I32, I32],
            HostFn::PrintWord => vec![I64],
            HostFn::BigintFromI64 => vec![I64],
            HostFn::BigintPow => vec![I32, I64],
            HostFn::BigintToText => vec![I32],
            HostFn::BigintMul | HostFn::BigintAdd | HostFn::BigintSub | HostFn::BigintDiv | HostFn::BigintMod => vec![I32, I32],
            HostFn::ComplexFromI64 => vec![I64, I64],
            HostFn::ComplexAdd | HostFn::ComplexSub | HostFn::ComplexMul => vec![I32, I32],
            HostFn::ComplexToText => vec![I32],
            HostFn::ModularFromI64 => vec![I64, I64],
            HostFn::ModularAdd | HostFn::ModularSub | HostFn::ModularMul => vec![I32, I32],
            HostFn::ModularToText => vec![I32],
            HostFn::DecimalFromI64 => vec![I64],
            HostFn::DecimalFromText | HostFn::DecimalToText => vec![I32],
            HostFn::DecimalAdd | HostFn::DecimalSub | HostFn::DecimalMul => vec![I32, I32],
            HostFn::MoneyFromI64 => vec![I64, I32],
            HostFn::MoneyFromDecimal | HostFn::MoneyAdd | HostFn::MoneySub => vec![I32, I32],
            HostFn::MoneyToText => vec![I32],
            HostFn::QuantityOfI64 => vec![I64, I32],
            HostFn::QuantityConvert
            | HostFn::QuantityAdd
            | HostFn::QuantitySub
            | HostFn::QuantityMul
            | HostFn::QuantityDiv => vec![I32, I32],
            HostFn::QuantityToText => vec![I32],
            HostFn::RationalFromI64 => vec![I64],
            HostFn::RationalFromBigint
            | HostFn::RationalToText
            | HostFn::RationalFloor
            | HostFn::RationalCeil
            | HostFn::RationalRound
            | HostFn::RationalAbs => vec![I32],
            HostFn::RationalAdd | HostFn::RationalSub | HostFn::RationalMul | HostFn::RationalDiv => vec![I32, I32],
            HostFn::UuidNil | HostFn::UuidMax | HostFn::UuidDns | HostFn::UuidUrl | HostFn::UuidOid | HostFn::UuidX500 => vec![],
            HostFn::UuidParse | HostFn::UuidVersion | HostFn::UuidToText | HostFn::UuidFromPtr => vec![I32],
            HostFn::UuidEq => vec![I32, I32],
            HostFn::RtAlloc => vec![I32],
        }
    }

    /// The wasm result value-types this host function returns.
    fn results(self) -> Vec<u8> {
        match self {
            HostFn::PrintI64
            | HostFn::PrintBool
            | HostFn::PrintChar
            | HostFn::PrintF64
            | HostFn::PrintDate
            | HostFn::PrintMoment
            | HostFn::PrintDuration
            | HostFn::PrintTime
            | HostFn::PrintSpan
            | HostFn::PrintSeqI64
            | HostFn::PrintSeqBool
            | HostFn::PrintSeqWord32
            | HostFn::PrintSeqWord64
            | HostFn::PrintSeqF64
            | HostFn::PrintText
            | HostFn::PrintSeqText
            | HostFn::PrintSetI64
            | HostFn::PrintSetText
            | HostFn::PrintRational
            | HostFn::PrintNothing
            | HostFn::PrintWord => vec![],
            HostFn::PowFf | HostFn::PowFi | HostFn::ParseFloat => vec![F64],
            HostFn::Today | HostFn::FmtI64Into | HostFn::FmtF64Into | HostFn::FmtBoolInto | HostFn::FmtF64PrecInto | HostFn::FmtAlignInto | HostFn::FmtSeqI64Into | HostFn::FmtSeqBoolInto | HostFn::FmtSetI64Into | HostFn::Args
            | HostFn::BigintFromI64 | HostFn::BigintPow | HostFn::BigintToText | HostFn::BigintMul | HostFn::BigintAdd | HostFn::BigintSub | HostFn::BigintDiv | HostFn::BigintMod | HostFn::ComplexFromI64 | HostFn::ComplexAdd | HostFn::ComplexSub | HostFn::ComplexMul | HostFn::ComplexToText | HostFn::ModularFromI64 | HostFn::ModularAdd | HostFn::ModularSub | HostFn::ModularMul | HostFn::ModularToText | HostFn::DecimalFromText | HostFn::DecimalFromI64 | HostFn::DecimalAdd | HostFn::DecimalSub | HostFn::DecimalMul | HostFn::DecimalToText | HostFn::MoneyFromDecimal | HostFn::MoneyFromI64 | HostFn::MoneyAdd | HostFn::MoneySub | HostFn::MoneyToText | HostFn::QuantityOfI64 | HostFn::QuantityConvert | HostFn::QuantityAdd | HostFn::QuantitySub | HostFn::QuantityMul | HostFn::QuantityDiv | HostFn::QuantityToText | HostFn::RationalFromI64 | HostFn::RationalFromBigint | HostFn::RationalAdd | HostFn::RationalSub | HostFn::RationalMul | HostFn::RationalDiv | HostFn::RationalToText | HostFn::RationalFloor | HostFn::RationalCeil | HostFn::RationalRound | HostFn::RationalAbs | HostFn::UuidParse | HostFn::UuidNil | HostFn::UuidMax | HostFn::UuidDns | HostFn::UuidUrl | HostFn::UuidOid | HostFn::UuidX500 | HostFn::UuidEq | HostFn::UuidToText | HostFn::UuidFromPtr | HostFn::DateAddSpan | HostFn::Sha1Rnds4 | HostFn::Sha1Msg1 | HostFn::Sha1Msg2 | HostFn::Sha1Nexte | HostFn::Lanes4Add | HostFn::Lanes4Xor | HostFn::RtAlloc | HostFn::FormatTimestampRt | HostFn::InZoneRt | HostFn::Lanes16FromBytes | HostFn::Lanes8FromWords | HostFn::Lanes4W64FromWords | HostFn::LanesSplat16 | HostFn::LanesSplat8 | HostFn::LanesToSeq | HostFn::LanesShuffle | HostFn::LanesInterleaveLo | HostFn::LanesInterleaveHi | HostFn::LanesByteAdd | HostFn::LanesMaddubs | HostFn::LanesPackus | HostFn::LanesShrBytes | HostFn::DecimalToRational | HostFn::MoneySetRate | HostFn::MoneyToCurrency | HostFn::MoneySetRatesInt | HostFn::MoneySetRatesRational | HostFn::MoneySetRatesDecimal | HostFn::WireBytesInt | HostFn::WireBytesBool | HostFn::WireBytesFloat | HostFn::WireBytesText | HostFn::ReadWireFrame | HostFn::ReadWireProgramRt | HostFn::DynamicToText => vec![I32],
            HostFn::Now | HostFn::ParseInt | HostFn::ParseTimestamp | HostFn::TemporalComponent | HostFn::TemporalComponentDate | HostFn::UuidVersion | HostFn::MomentAddSpan | HostFn::MonthsBetweenRt | HostFn::YearsBetweenRt | HostFn::LocalInstantRt | HostFn::WriteWireResidual | HostFn::RunAccepted => vec![I64],
        }
    }

    /// The sink that displays a value of kind `k` — matching the tree-walker's
    /// `to_display_string` for that scalar kind. `None` for a kind without a scalar sink yet
    /// (e.g. a sequence, whose formatted display lands with the heap value model).
    fn for_show(k: Kind) -> Option<HostFn> {
        Some(match k {
            Kind::Int => HostFn::PrintI64,
            Kind::Bool => HostFn::PrintBool,
            Kind::Char => HostFn::PrintChar,
            Kind::Float => HostFn::PrintF64,
            Kind::Date => HostFn::PrintDate,
            Kind::Moment => HostFn::PrintMoment,
            Kind::Duration => HostFn::PrintDuration,
            Kind::Time => HostFn::PrintTime,
            Kind::Span => HostFn::PrintSpan,
            Kind::SeqInt => HostFn::PrintSeqI64,
            Kind::SeqBool => HostFn::PrintSeqBool,
            Kind::SeqWord32 => HostFn::PrintSeqWord32,
            Kind::SeqWord64 => HostFn::PrintSeqWord64,
            Kind::SeqFloat => HostFn::PrintSeqF64,
            Kind::SeqText => HostFn::PrintSeqText,
            // A never-refined empty sequence formats as `[]`; the i64 sink reads zero elements.
            Kind::SeqAny => HostFn::PrintSeqI64,
            Kind::Text => HostFn::PrintText,
            // A whole struct's `TypeName { field: val, … }` display is assembled by `lower_show_struct`
            // (fields in DETERMINISTIC alphabetical order, matching the VM's now-sorted `HashMap`
            // display) — handled by the `Op::Show` dispatch, not this per-kind scalar-sink table.
            Kind::Struct => return None,
            // A whole `Set of Int` is insertion-ordered in both the VM (a `Vec`) and the AOT backend,
            // so it displays deterministically: `{e0, e1, …}`.
            Kind::Set => HostFn::PrintSetI64,
            Kind::SetText | Kind::CrdtSetText => HostFn::PrintSetText,
            // A whole Map's `{k: v, …}` display is assembled by `lower_show_map` (a runtime entry loop
            // in insertion order, matching the VM's `IndexMap`), not a single scalar sink — so the
            // `Op::Show` dispatch handles it directly, not this per-kind host table.
            Kind::Map => return None,
            // Showing an enum value (`Ctor` / `Ctor(args)`) lands with the argument payload model.
            Kind::Enum => return None,
            // A closure has no display form.
            Kind::Closure => return None,
            // A `BigInt` `Show` is a TWO-step lowering (`logos_rt_bigint_to_text` → `print_text`), not a
            // single sink, so it is handled directly by `lower_show` / the import scan — not here.
            Kind::BigInt => return None,
            // A `Complex` `Show` likewise renders via `logos_rt_complex_to_text` → `print_text` (dispatch).
            Kind::Complex => return None,
            // A `Modular` `Show` renders via `logos_rt_modular_to_text` → `print_text` (dispatch).
            Kind::Modular => return None,
            // A `Decimal` `Show` renders via `logos_rt_decimal_to_text` → `print_text` (dispatch).
            Kind::Decimal => return None,
            // A `Money` `Show` renders via `logos_rt_money_to_text` → `print_text` (dispatch).
            Kind::Money => return None,
            Kind::Quantity => return None,
            Kind::Uuid => return None,
            Kind::Lanes => return None,
            // A SIMD lane vector is not `Show`n directly (it is unpacked back to a `Seq` first).
            Kind::LanesV => return None,
            // A dynamic (wire-decoded) value `Show`s via a two-step `logos_rt_dynamic_to_text` → `print_text`
            // dispatch (like the numeric `to_text` handles), NOT a single scalar sink.
            Kind::Dynamic => return None,
            // A whole heterogeneous tuple's display (mixed element formats) is deferred; element
            // access (`item N of t`) at each position's kind works.
            Kind::Tuple => return None,
            // A whole sequence of structs would need each struct's display, which is non-
            // deterministic (struct field order); element access (`item N of xs`) works.
            Kind::SeqStruct => return None,
            // A whole sequence of enums (each `Ctor`/`Ctor(args)`) is deferred; iteration +
            // `Inspect`/`TestArm` on each element works.
            Kind::SeqEnum => return None,
            // A whole nested sequence's display is deferred (needs a per-row formatter); row access
            // (`item N of m` → a `SeqInt`) and element access work.
            Kind::SeqSeqInt => return None,
            // A `Rational` is Shown via a dedicated two-arg (`num`, `den`) path in `Op::Show`, not this
            // single-value sink — so it never reaches here.
            Kind::Rational => return None,
            // An `Optional` is Shown via a dedicated null-check path in `Op::Show` (null → "nothing",
            // else the boxed inner) — so it never reaches this single-value sink.
            Kind::Optional => return None,
            // A `Word32`/`Word64` Shows as its UNSIGNED value via a dedicated `print_word` path in
            // `Op::Show` (Word32 zero-extends to i64 first) — never this signed single-value sink.
            Kind::Word32 | Kind::Word64 => return None,
        })
    }
}

/// Interns `(params, results)` value-type signatures into a deduped Type section.
#[derive(Default)]
struct TypeTable {
    sigs: Vec<(Vec<u8>, Vec<u8>)>,
}

impl TypeTable {
    fn intern(&mut self, params: Vec<u8>, results: Vec<u8>) -> u32 {
        if let Some(i) = self.sigs.iter().position(|(p, r)| *p == params && *r == results) {
            return i as u32;
        }
        self.sigs.push((params, results));
        (self.sigs.len() - 1) as u32
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        leb_u32(&mut out, self.sigs.len() as u32);
        for (params, results) in &self.sigs {
            out.push(0x60); // func type
            leb_u32(&mut out, params.len() as u32);
            out.extend_from_slice(params);
            leb_u32(&mut out, results.len() as u32);
            out.extend_from_slice(results);
        }
        out
    }
}

/// One function the assembler will emit: its rebased ops, inferred register kinds, parameter
/// count, register frame size, result kind (`None` = returns nothing / void), and which basic
/// blocks are statically reachable (dead blocks — e.g. the monomorphized-out branch of an
/// `and`/`or` runtime type-dispatch — are emitted as a single `unreachable`).
struct Plan {
    ops: Vec<Op>,
    kinds: KindTable,
    num_params: u32,
    num_regs: u32,
    result: Option<Kind>,
    reachable_blocks: Vec<bool>,
    /// Per-op struct slot/field/count map (see [`kind::struct_layout`]) — drives `NewStruct`
    /// sizing and `StructInsert`/`GetField` slot addressing.
    structs: kind::StructLayout,
    /// Per-pc: whether this `StructInsert` must copy-on-write its target to preserve struct value
    /// semantics (see [`cow_struct_inserts`]).
    cow_inserts: Vec<bool>,
    /// Per composite-handle register, its resolved access SHAPE — `structs.reg_shape` (params + local
    /// structs) COMPLETED post-inference with locally-built map/enum/het-tuple shapes (which need the
    /// register kinds inference produced). Read at each `MakeClosure` to type a captured composite.
    reg_shape: std::collections::HashMap<u16, kind::ParamShape>,
    /// If this function `Return`s a single statically-known closure (every reachable closure `Return`
    /// agreeing on one body function index), that index — so a caller of this function can resolve a
    /// `CallValue` on the returned handle. `None` if it returns no closure, or returns more than one.
    return_closure: Option<u16>,
    /// A STUB: a function unreachable from `Main` that the AOT dropped (its body is a single `unreachable`
    /// trap, its type `() -> ()`). This lets a program import a large stdlib module and compile even when
    /// the UNUSED functions use ops the AOT can't lower (e.g. `uuid_v5` pulls in `uuid.lg`, whose unused
    /// `uuidParse`/`decodeNibbles` use a `Lanes16Word8` SIMD vocabulary the backend doesn't have).
    stub: bool,
}

impl Plan {
    /// A dropped (unreachable) function — a `() -> ()` body that just traps.
    fn stub() -> Plan {
        Plan {
            ops: Vec::new(),
            kinds: KindTable::empty(),
            num_params: 0,
            num_regs: 0,
            result: None,
            reachable_blocks: Vec::new(),
            structs: kind::StructLayout::default(),
            cow_inserts: Vec::new(),
            reg_shape: std::collections::HashMap::new(),
            return_closure: None,
            stub: true,
        }
    }
}

/// Infer register kinds with reachability-DCE: walk the CFG to find the reachable
/// blocks, then run the strict kind inference over reachable ops only (so a dead
/// branch's writes never poison a register). Returns the kinds plus the per-block
/// reachability the emitter uses.
fn infer_with_reachability(
    ops: &[Op],
    constants: &[Constant],
    struct_types: &[StructTypeDef],
    enum_types: &[EnumTypeDef],
    fn_return_types: &[Option<BoundaryType>],
    num_regs: usize,
    seeds: &[Option<Kind>],
    ret_of: &dyn Fn(usize) -> Option<Kind>,
    global_of: &dyn Fn(u16) -> Option<Kind>,
    closure_ret: &dyn Fn(usize) -> Option<Kind>,
    ret_layout: &dyn Fn(u16) -> Option<FieldLayout>,
    fn_return_closure: &dyn Fn(u16) -> Option<u16>,
    param_layouts: &[(u16, ParamShape)],
    param_closures: &[(u16, u16)],
    linked: bool,
    functions: &[crate::vm::instruction::CompiledFunction],
) -> R<(KindTable, Vec<bool>, Vec<bool>)> {
    let blocks = Blocks::new(ops).ok_or(WasmLowerError::Unsupported("jump target escapes the function"))?;
    let (pc_reach, block_reach) = reachability(ops, &blocks);
    // The general Int-overflow→BigInt promotion set (empty unless linked): registers only observed or
    // fed into more BigInt arithmetic, computed once over this region's post-regsplit ops.
    let bigint_demand = kind::bigint_demanded_regs(ops, functions, linked);
    let kinds = kind::infer(ops, constants, struct_types, enum_types, fn_return_types, num_regs, seeds, ret_of, global_of, closure_ret, ret_layout, fn_return_closure, param_layouts, param_closures, &pc_reach, linked, &bigint_demand)?;
    Ok((kinds, pc_reach, block_reach))
}

/// Compute reachability from the entry block. Returns `(per-pc reachable,
/// per-block reachable)`.
fn reachability(ops: &[Op], blocks: &Blocks) -> (Vec<bool>, Vec<bool>) {
    let nb = blocks.num_blocks();
    let mut block_reach = vec![false; nb];
    let mut stack = vec![0usize];
    while let Some(k) = stack.pop() {
        if block_reach[k] {
            continue;
        }
        block_reach[k] = true;
        for s in block_successors(ops, blocks, k) {
            if !block_reach[s] {
                stack.push(s);
            }
        }
    }
    let mut pc_reach = vec![false; ops.len()];
    for (k, &live) in block_reach.iter().enumerate() {
        if live {
            for pc in blocks.start(k)..blocks.end(k) {
                pc_reach[pc] = true;
            }
        }
    }
    (pc_reach, block_reach)
}

/// The successor blocks of block `k`.
fn block_successors(ops: &[Op], blocks: &Blocks, k: usize) -> Vec<usize> {
    let n = ops.len();
    let fallthrough = |pc: usize| -> Vec<usize> {
        if pc + 1 < n {
            vec![blocks.block_of(pc + 1)]
        } else {
            vec![]
        }
    };
    for pc in blocks.start(k)..blocks.end(k) {
        match ops[pc] {
            Op::Jump { target } => return vec![blocks.block_of(target)],
            Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
                let mut s = vec![blocks.block_of(target)];
                s.extend(fallthrough(pc));
                return s;
            }
            // `IterNext` branches to `exit` (the matching `IterPop`) when the snapshot is
            // exhausted, else falls through to the loop body — both blocks are reachable.
            Op::IterNext { exit, .. } => {
                let mut s = vec![blocks.block_of(exit)];
                s.extend(fallthrough(pc));
                return s;
            }
            Op::Return { .. } | Op::ReturnNothing | Op::Halt | Op::FailWith { .. } => return vec![],
            _ => {}
        }
    }
    // Fell through the block end with no terminator.
    if blocks.end(k) < n {
        vec![blocks.block_of(blocks.end(k))]
    } else {
        vec![]
    }
}

/// Lower a whole program to a self-contained WebAssembly module (the bytes of a `.wasm` file).
/// Returns [`WasmLowerError::Unsupported`] for any program outside the scalar fragment — a
/// sound refusal, never a wrong module.
pub fn assemble_program(program: &CompiledProgram, policies: &PolicyRegistry, interner: &Interner) -> R<Vec<u8>> {
    assemble_program_impl(program, policies, interner, false)
}

/// Linker-mode assembly: an integer `Op::Pow` lowers to the real `logicaffeine_base::BigInt` runtime
/// (`logos_rt_bigint_from_i64`→`_pow`→`_to_text`) yielding a `Text` handle, so an overflowing
/// `x to the power of y` computes the exact big integer the VM's BigInt promotion prints instead of
/// trapping. The module imports `env.__linear_memory` (the linker supplies one shared memory) and the
/// three `logos_rt_bigint_*` functions by undefined symbol; [`super::reloc::module_to_relocatable`] +
/// `rust-lld` link it against the prebuilt base runtime. Emitter-side heap allocation is refused (two
/// allocators over one linear memory would corrupt it — the only heap value is the runtime-built Text).
pub(crate) fn assemble_program_linked(program: &CompiledProgram, policies: &PolicyRegistry, interner: &Interner) -> R<Vec<u8>> {
    assemble_program_impl(program, policies, interner, true)
}

fn assemble_program_impl(program: &CompiledProgram, policies: &PolicyRegistry, interner: &Interner, linked: bool) -> R<Vec<u8>> {
    // ---- 1. Plan Main + every user function (rebase, infer kinds, resolve result kind) ----
    let layout = code_layout(program)?;
    let ret_of = |fi: usize| -> Option<Kind> { declared_result(program, fi) };
    let no_closure = |_: usize| -> Option<Kind> { None };
    let no_ret_layout = |_: u16| -> Option<FieldLayout> { None };
    let no_ret_closure = |_: u16| -> Option<u16> { None };
    let no_param_origin = |_: usize, _: usize| -> Option<u16> { None };

    // Plan Main once with no resolvers — it types both the values stored into globals (read below)
    // AND the local registers a `MakeClosure` captures (the cross-scope closure-capture flow below).
    let num_globals = program.globals.len();
    let no_globals = |_: u16| None;
    let no_caps: Vec<Vec<Option<Kind>>> = Vec::new();
    let main_p1 = plan_main(program, &layout, &ret_of, &no_globals, &no_closure, &no_ret_layout, &no_ret_closure, linked)?;
    let global_kinds: Vec<Option<Kind>> = {
        let mut gk = vec![None; num_globals];
        for op in &main_p1.ops {
            if let Op::GlobalSet { idx, src } = *op {
                if let Some(k) = main_p1.kinds.get(src as usize) {
                    gk[idx as usize] = Some(k);
                }
            }
        }
        gk
    };
    let global_of = |idx: u16| -> Option<Kind> { global_kinds.get(idx as usize).copied().flatten() };
    // Per global, the body function index of the CLOSURE it holds (a Main `Let f be (…) -> …` used in a
    // function/closure is promoted to a global), so a closure capturing such a global resolves the call
    // to the captured closure — the global analog of the local captured-closure trace.
    let global_closures: Vec<Option<u16>> = {
        let mut gc = vec![None; num_globals];
        for op in &main_p1.ops {
            if let Op::GlobalSet { idx, src } = *op {
                if let Some(&c) = main_p1.structs.closure_of.get(&src) {
                    gc[idx as usize] = Some(c);
                }
            }
        }
        gc
    };
    let global_closure_of = |idx: u16| -> Option<u16> { global_closures.get(idx as usize).copied().flatten() };

    // CROSS-SCOPE + NESTED CLOSURE PLANNING — a FIXPOINT. A closure's capture kinds come from where it
    // is BUILT (its `MakeClosure`); its result kind flows UP to callers. A NESTED closure (built inside
    // another closure body) crosses BOTH directions over more levels than the old 2-pass could resolve
    // (the inner's captures need the outer planned; the outer's result needs the inner planned). So:
    // plan the non-closure functions (no captures → clean), then ITERATE — extract capture kinds/shapes
    // from EVERY currently-planned region (so a closure built inside an already-planned closure body
    // gets its captures) and re-plan every function with the latest captures + inferred call-result
    // kinds, TOLERATING a closure whose captures aren't resolvable yet (`.ok()`; it succeeds a later
    // round once its build-region is planned). Converges in O(nesting depth) rounds; the final STRICT
    // pass below is the backstop that rejects a genuinely-unsupported function.
    let no_shapes: Vec<Vec<Option<ParamShape>>> = Vec::new();
    let no_capture_closures: Vec<Vec<Option<u16>>> = Vec::new();
    let mut plans: Vec<Option<Plan>> = (0..program.functions.len()).map(|_| None).collect();
    for fi in 0..program.functions.len() {
        if !layout.is_closure[fi] {
            plans[fi] = Some(plan_function(program, fi, &layout, &ret_of, &global_of, &no_closure, &no_ret_layout, &no_ret_closure, &no_param_origin, &no_caps, &no_shapes, &no_capture_closures, false, linked)?);
        }
    }
    let mut capture_kinds: Vec<Vec<Option<Kind>>> = Vec::new();
    let mut capture_shapes: Vec<Vec<Option<ParamShape>>> = Vec::new();
    let mut capture_closures: Vec<Vec<Option<u16>>> = Vec::new();
    for _ in 0..(program.functions.len() + 4) {
        let (ck, cs, cc) = extract_capture_kinds(program, &main_p1, &plans, &global_of, &global_closure_of);
        capture_kinds = ck;
        capture_shapes = cs;
        capture_closures = cc;
        let cur_results: Vec<Option<Kind>> = plans.iter().map(|p| p.as_ref().and_then(|p| p.result)).collect();
        let cur_layouts: Vec<Option<FieldLayout>> = plans.iter().map(|p| p.as_ref().and_then(fn_return_struct_layout)).collect();
        let cur_ret_closures: Vec<Option<u16>> = plans.iter().map(|p| p.as_ref().and_then(|p| p.return_closure)).collect();
        let closure_ret = |fi: usize| cur_results.get(fi).copied().flatten();
        let ret_of_i =
            |fi: usize| cur_results.get(fi).copied().flatten().or_else(|| declared_result(program, fi));
        let ret_layout_of = |func: u16| cur_layouts.get(func as usize).cloned().flatten();
        let ret_closure_of = |func: u16| cur_ret_closures.get(func as usize).copied().flatten();
        // CLOSURES AS ARGUMENTS. Re-plan Main with this round's resolvers so its `closure_of` reflects
        // a returned closure bound to a local (then passed straight into a function); attribute every
        // call's closure arguments to the parameters they feed, so `f(args)` resolves when `f` is a
        // parameter. Recomputed each round — a param's origin can resolve transitively as callers plan.
        let main_iter = plan_main(program, &layout, &ret_of_i, &global_of, &closure_ret, &ret_layout_of, &ret_closure_of, linked).ok();
        let mut scan_plans: Vec<&Plan> = vec![main_iter.as_ref().unwrap_or(&main_p1)];
        scan_plans.extend(plans.iter().flatten());
        let param_origins = compute_param_origins(program, &scan_plans);
        let param_origin = |fi: usize, pi: usize| param_origins.get(fi).and_then(|v| v.get(pi)).copied().flatten();
        let mut progress = false;
        for fi in 0..program.functions.len() {
            if let Ok(p) = plan_function(program, fi, &layout, &ret_of_i, &global_of, &closure_ret, &ret_layout_of, &ret_closure_of, &param_origin, &capture_kinds, &capture_shapes, &capture_closures, false, linked) {
                let changed = plans[fi]
                    .as_ref()
                    .map_or(true, |x| x.result != p.result || x.return_closure != p.return_closure);
                if changed {
                    progress = true;
                }
                plans[fi] = Some(p);
            }
        }
        if !progress {
            break;
        }
    }
    let fns1: Vec<Plan> = plans.into_iter().map(|p| p.expect("every function planned by the fixpoint")).collect();

    // Pass 2 (STRICT) re-plans every function WITH the resolvers (a call's inferred result kind +
    // struct-return field layout), so cross-region `f(…)'s field` resolves and a genuinely unknown
    // return is rejected rather than deferred. `capture_kinds` carries through so closure bodies keep
    // their capture typing. Resolvers affect only `GetField`, so pass-1 results/layouts stay valid.
    let fn_results_p1: Vec<Option<Kind>> = fns1.iter().map(|p| p.result).collect();
    let ret_layouts: Vec<Option<FieldLayout>> = fns1.iter().map(fn_return_struct_layout).collect();
    let ret_closures: Vec<Option<u16>> = fns1.iter().map(|p| p.return_closure).collect();
    let closure_ret = |fi: usize| -> Option<Kind> { fn_results_p1.get(fi).copied().flatten() };
    let ret_of_inferred =
        |fi: usize| -> Option<Kind> { fn_results_p1.get(fi).copied().flatten().or_else(|| declared_result(program, fi)) };
    let ret_layout_of = |func: u16| -> Option<FieldLayout> { ret_layouts.get(func as usize).cloned().flatten() };
    let ret_closure_of = |func: u16| -> Option<u16> { ret_closures.get(func as usize).copied().flatten() };
    // Main is planned first (it doesn't depend on the strict `fns`, only on the converged resolvers)
    // so it can join the param-origin scan — a closure argument is frequently passed from Main.
    let main = plan_main(program, &layout, &ret_of_inferred, &global_of, &closure_ret, &ret_layout_of, &ret_closure_of, linked)?;
    let scan_plans: Vec<&Plan> = std::iter::once(&main).chain(fns1.iter()).collect();
    let param_origins = compute_param_origins(program, &scan_plans);
    let param_origin = |fi: usize, pi: usize| param_origins.get(fi).and_then(|v| v.get(pi)).copied().flatten();
    // Function-level DCE: a function unreachable from `Main` (via direct `Call` / closure `MakeClosure`
    // edges, transitively) is dropped to a trap stub instead of being strictly planned. This lets a
    // program demand-import a large stdlib module and compile even when its UNUSED functions use ops the
    // backend can't lower — `uuid_v5` pulls in all of `uuid.lg`, whose unused `uuidParse`/`decodeNibbles`
    // need a `Lanes16Word8` SIMD vocabulary the AOT doesn't have; dropping them is sound (never called).
    let reachable = {
        // Main's region is `[0..main_end)` — the boundary where the REGULAR functions begin. It must be
        // `main_end`, NOT the min entry_pc over all functions: an INLINE closure (`Let f be (x)->…` in
        // Main) is emitted inside Main's region with a smaller entry_pc, so keying on the min would
        // truncate Main's slice before the `MakeClosure` op that references it — stubbing a live closure
        // body into a trap. A reached regular function's own region (`func_region[fi]`, which spans its
        // interleaved inline-closure holes) then collects its inline closures transitively below.
        let main_end = layout.main_end.min(program.code.len());
        let mut r = vec![false; program.functions.len()];
        let mut stack: Vec<usize> = Vec::new();
        // Every op carrying a `func` edge (the complete call graph): a direct `Call`, a `MakeClosure`
        // body, and the two task-spawn forms (`Spawn`/`SpawnHandle`). Missing any would stub a live
        // callee into a trap.
        let collect = |ops: &[Op], stack: &mut Vec<usize>| {
            for op in ops {
                if let Op::Call { func, .. }
                | Op::MakeClosure { func, .. }
                | Op::Spawn { func, .. }
                | Op::SpawnHandle { func, .. } = op
                {
                    stack.push(*func as usize);
                }
            }
        };
        collect(&program.code[0..main_end], &mut stack);
        while let Some(fi) = stack.pop() {
            if fi >= r.len() || r[fi] {
                continue;
            }
            r[fi] = true;
            let (s, e) = layout.func_region[fi];
            if s <= e && e <= program.code.len() {
                collect(&program.code[s..e], &mut stack);
            }
        }
        r
    };
    let fns: Vec<Plan> = {
        let mut v = Vec::with_capacity(program.functions.len());
        for fi in 0..program.functions.len() {
            if !reachable[fi] {
                v.push(Plan::stub());
                continue;
            }
            v.push(plan_function(program, fi, &layout, &ret_of_inferred, &global_of, &closure_ret, &ret_layout_of, &ret_closure_of, &param_origin, &capture_kinds, &capture_shapes, &capture_closures, true, linked)?);
        }
        v
    };

    // Whether the program uses the emitter's heap value model (a linear memory + bump allocator), an
    // iterator stack, or closures — computed HERE (before the import scan) so linker mode can import the
    // runtime allocator for the slab + refuse the shapes it can't yet share.
    // A heap op, OR a `LoadConst` of a Text literal (which materializes a Text object in memory).
    let loads_text = |op: &Op| matches!(op, Op::LoadConst { idx, .. } if matches!(program.constants.get(*idx as usize), Some(Constant::Text(_))));
    // A Text-typed `+`/`+=` (string concatenation) allocates a fresh Text, so it needs the heap even
    // if the program has no Text literal (it builds from a Text-valued variable).
    let concats_text = |p: &Plan, op: &Op| match *op {
        Op::Add { dst, .. } | Op::AddAssign { dst, .. } => p.kinds.get(dst as usize) == Some(Kind::Text),
        _ => false,
    };
    let uses_heap = std::iter::once(&main)
        .chain(fns.iter())
        .any(|p| p.ops.iter().any(|op| op_uses_heap(op) || loads_text(op) || concats_text(p, op)));
    let uses_iter = std::iter::once(&main)
        .chain(fns.iter())
        .any(|p| p.ops.iter().any(|op| matches!(op, Op::IterPrepare { .. })));
    // Closures need a function table (for `call_indirect`) in the self-contained path; emit it only
    // when the program has one. LINKER MODE lowers a closure `CallValue` to a DIRECT `call` instead (the
    // callee is statically resolved, or already refused), so it needs NEITHER a table NOR an element
    // section — nothing the reloc transform can't handle. (A closure captured through a truly dynamic
    // value stays refused inside `lower_call_value`.)
    let has_closures = layout.is_closure.iter().any(|&c| c);

    // ---- 2. Which host functions are used (in stable order) → their import indices ----
    let mut used = Vec::new();
    let note = |h: HostFn, used: &mut Vec<HostFn>| {
        if !used.contains(&h) {
            used.push(h);
        }
    };
    // The host formatter a value of a given kind is stringified through (in a `Concat`, a Text-typed
    // `+`, or a whole-tuple / payload-enum `Show`). A kind with no scalar formatter notes nothing —
    // the per-op lowering soundly refuses it, so no import is wasted on a shape it can't emit.
    let note_fmt_kind = |k: Option<Kind>, used: &mut Vec<HostFn>| match k {
        Some(Kind::Int) => note(HostFn::FmtI64Into, used),
        Some(Kind::Float) => note(HostFn::FmtF64Into, used),
        Some(Kind::Bool) => note(HostFn::FmtBoolInto, used),
        // A whole `Seq of Int` / `Set of Int` operand is stringified by its collection formatter.
        Some(Kind::SeqInt | Kind::SeqAny) => note(HostFn::FmtSeqI64Into, used),
        // A whole `Seq of Bool` operand renders `[true, false, …]` via its own formatter.
        Some(Kind::SeqBool) => note(HostFn::FmtSeqBoolInto, used),
        Some(Kind::Set) => note(HostFn::FmtSetI64Into, used),
        // A BigInt operand of a concat is stringified to its decimal Text via the runtime.
        Some(Kind::BigInt) => note(HostFn::BigintToText, used),
        _ => {}
    };
    // A stringified operand (in a `Concat` or a Text-typed `+`) needs its scalar host formatter.
    let note_operand_fmt = |plan: &Plan, r: u16, used: &mut Vec<HostFn>| note_fmt_kind(plan.kinds.get(r as usize), used);
    for plan in std::iter::once(&main).chain(fns.iter()) {
        for op in &plan.ops {
            match *op {
                // Note the sink for any Show whose kind is known. An unknown-kind Show is NOT an
                // error here: it is either in a statically-dead block (never lowered — e.g. the
                // `Show` after an unbound-variable `FailWith`) or a genuinely unsupported kind that
                // the reachability-respecting per-block lowering will reject. Erroring here would
                // wrongly reject a program whose only unknown-kind Show is dead code.
                Op::Show { src } => {
                    if let Some(elems) = plan.structs.tuple_layouts.get(&src) {
                        // A whole-tuple `Show` (any tuple, homogeneous or not) assembles its `(…)`
                        // display as a Text and prints it, stringifying each element by its formatter.
                        note(HostFn::PrintText, &mut used);
                        for &e in elems {
                            note_operand_fmt(plan, e, &mut used);
                        }
                    } else if plan.kinds.get(src as usize) == Some(Kind::Enum) {
                        // A whole-enum `Show` prints the live variant's name via `print_text`; a
                        // PAYLOAD variant additionally stringifies each field inline (`Ctor(f0, …)`),
                        // so note every field type's formatter across the enum's variants.
                        note(HostFn::PrintText, &mut used);
                        if let Some(def) = plan
                            .structs
                            .ind_type_of
                            .get(&src)
                            .and_then(|tn| program.enum_types.iter().find(|e| &e.name == tn))
                        {
                            for v in &def.variants {
                                for ft in &v.field_types {
                                    note_fmt_kind(kind::boundary_to_kind(ft), &mut used);
                                }
                            }
                        }
                    } else if plan.kinds.get(src as usize) == Some(Kind::Map) {
                        // A whole-map `Show` assembles `{k: v, …}` and prints it; note `print_text` plus
                        // the key AND value formatters (resolved from the last SetIndex's registers).
                        note(HostFn::PrintText, &mut used);
                        if let Some(&kr) = plan.structs.map_set_key.get(&src) {
                            note_fmt_kind(plan.kinds.get(kr as usize), &mut used);
                        }
                        if let Some(&vr) = plan.structs.map_set_value.get(&src) {
                            note_fmt_kind(plan.kinds.get(vr as usize), &mut used);
                        }
                    } else if plan.kinds.get(src as usize) == Some(Kind::SeqSeqInt) {
                        // A nested int-seq `Show` prints `[[…], …]`: `print_text` for the assembled
                        // outer string, and the scalar seq formatter for each inner `Seq of Int`.
                        note(HostFn::PrintText, &mut used);
                        note(HostFn::FmtSeqI64Into, &mut used);
                    } else if plan.kinds.get(src as usize) == Some(Kind::SeqEnum) {
                        // A whole `Seq of Enum` `Show` prints `[e0, …]` via `print_text`; each element's
                        // payload fields (if any) stringify through their own formatters.
                        note(HostFn::PrintText, &mut used);
                        if let Some(def) = plan
                            .structs
                            .seq_elem_ind_type
                            .get(&src)
                            .and_then(|tn| program.enum_types.iter().find(|e| &e.name == tn))
                        {
                            for v in &def.variants {
                                for ft in &v.field_types {
                                    note_fmt_kind(kind::boundary_to_kind(ft), &mut used);
                                }
                            }
                        }
                    } else if plan.kinds.get(src as usize) == Some(Kind::Struct) {
                        // A whole struct `Show` prints `TypeName { … }` via `print_text`; each field
                        // stringifies through its own formatter.
                        note(HostFn::PrintText, &mut used);
                        if let Some(def) = plan
                            .structs
                            .struct_name_of
                            .get(&src)
                            .and_then(|tn| program.struct_types.iter().find(|s| &s.name == tn))
                        {
                            for (_, bt) in &def.fields {
                                note_fmt_kind(kind::boundary_to_kind(bt), &mut used);
                            }
                        }
                    } else if plan.kinds.get(src as usize) == Some(Kind::SeqStruct) {
                        // A whole `Seq of Struct` `Show` — `print_text` plus each element struct's field
                        // formatters (resolved from the seq's element struct type).
                        note(HostFn::PrintText, &mut used);
                        if let Some(def) = plan
                            .structs
                            .seq_elem_struct_name
                            .get(&src)
                            .and_then(|tn| program.struct_types.iter().find(|s| &s.name == tn))
                        {
                            for (_, bt) in &def.fields {
                                note_fmt_kind(kind::boundary_to_kind(bt), &mut used);
                            }
                        }
                    } else if plan.kinds.get(src as usize) == Some(Kind::Rational) {
                        // A `Rational` `Show`: LINKER mode renders the BigInt-backed handle via
                        // `logos_rt_rational_to_text`→`print_text`; the self-contained i64/i64 value uses
                        // the dedicated two-arg (num, den) `print_rational` host sink.
                        if linked {
                            note(HostFn::RationalToText, &mut used);
                            note(HostFn::PrintText, &mut used);
                        } else {
                            note(HostFn::PrintRational, &mut used);
                        }
                    } else if plan.kinds.get(src as usize) == Some(Kind::Optional) {
                        // An `Optional` `Show` uses `print_nothing` for the null handle, and the boxed
                        // inner scalar's own sink for the present (`Some`) case.
                        note(HostFn::PrintNothing, &mut used);
                        let inner = plan.structs.opt_inner.get(&src).copied().unwrap_or(Kind::Int);
                        if let Some(h) = HostFn::for_show(inner) {
                            note(h, &mut used);
                        }
                    } else if matches!(plan.kinds.get(src as usize), Some(Kind::Word32) | Some(Kind::Word64)) {
                        // A `Word` `Show` prints its unsigned value via `print_word`.
                        note(HostFn::PrintWord, &mut used);
                    } else if plan.kinds.get(src as usize) == Some(Kind::BigInt) {
                        // A `BigInt` `Show` renders the handle to a decimal Text then prints it.
                        note(HostFn::BigintToText, &mut used);
                        note(HostFn::PrintText, &mut used);
                    } else if plan.kinds.get(src as usize) == Some(Kind::Complex) {
                        // A `Complex` `Show` renders the handle to `re±imi` Text then prints it.
                        note(HostFn::ComplexToText, &mut used);
                        note(HostFn::PrintText, &mut used);
                    } else if plan.kinds.get(src as usize) == Some(Kind::Modular) {
                        note(HostFn::ModularToText, &mut used);
                        note(HostFn::PrintText, &mut used);
                    } else if plan.kinds.get(src as usize) == Some(Kind::Decimal) {
                        note(HostFn::DecimalToText, &mut used);
                        note(HostFn::PrintText, &mut used);
                    } else if plan.kinds.get(src as usize) == Some(Kind::Money) {
                        note(HostFn::MoneyToText, &mut used);
                        note(HostFn::PrintText, &mut used);
                    } else if plan.kinds.get(src as usize) == Some(Kind::Quantity) {
                        note(HostFn::QuantityToText, &mut used);
                        note(HostFn::PrintText, &mut used);
                    } else if plan.kinds.get(src as usize) == Some(Kind::Uuid) {
                        note(HostFn::UuidToText, &mut used);
                        note(HostFn::PrintText, &mut used);
                    } else if plan.kinds.get(src as usize) == Some(Kind::Dynamic) {
                        // A wire-decoded DYNAMIC value renders via `to_display_string` then prints.
                        note(HostFn::DynamicToText, &mut used);
                        note(HostFn::PrintText, &mut used);
                    } else if let Some(h) = plan.kinds.get(src as usize).and_then(HostFn::for_show) {
                        note(h, &mut used);
                    }
                }
                Op::CallBuiltin { builtin: BuiltinId::Pow, args_start, .. } => {
                    let bk = plan.kinds.get(args_start as usize);
                    let ek = plan.kinds.get((args_start + 1) as usize);
                    if let Some(h) = pow_host_for(bk, ek) {
                        note(h, &mut used);
                    }
                }
                // `a ** b` (the operator) shares `pow`'s host `pow_ff`/`pow_fi` for a Float result.
                // In LINKER mode an integer power drives the `logos_rt_bigint_*` runtime, leaving a
                // BigInt HANDLE (its `to_text` render is noted by the `Show` arm, not here).
                Op::Pow { lhs, rhs, .. } => {
                    let bk = plan.kinds.get(lhs as usize);
                    let ek = plan.kinds.get(rhs as usize);
                    if linked && bk == Some(Kind::Int) && ek == Some(Kind::Int) {
                        note(HostFn::BigintFromI64, &mut used);
                        note(HostFn::BigintPow, &mut used);
                    } else if let Some(h) = pow_host_for(bk, ek) {
                        note(h, &mut used);
                    }
                }
                // `+ - * / %` producing a BigInt (linker mode) call the matching `logos_rt_bigint_*` sink;
                // an `Int` operand is promoted with `from_i64`. Keyed on the result kind (which is BigInt
                // iff an operand was), so it wins over the numeric / Text-concat `Add` arms below.
                Op::Mul { dst, lhs, rhs }
                | Op::Add { dst, lhs, rhs }
                | Op::Sub { dst, lhs, rhs }
                | Op::Div { dst, lhs, rhs }
                | Op::Mod { dst, lhs, rhs }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::BigInt) =>
                {
                    note(
                        match *op {
                            Op::Add { .. } => HostFn::BigintAdd,
                            Op::Sub { .. } => HostFn::BigintSub,
                            Op::Div { .. } => HostFn::BigintDiv,
                            Op::Mod { .. } => HostFn::BigintMod,
                            _ => HostFn::BigintMul,
                        },
                        &mut used,
                    );
                    if plan.kinds.get(lhs as usize) == Some(Kind::Int) || plan.kinds.get(rhs as usize) == Some(Kind::Int) {
                        note(HostFn::BigintFromI64, &mut used);
                    }
                }
                // `+ - *` producing a Complex (linker mode) call the matching `logos_rt_complex_*` sink;
                // an `Int` operand is promoted with `from_i64` (real `n + 0i`).
                Op::Add { dst, lhs, rhs } | Op::Sub { dst, lhs, rhs } | Op::Mul { dst, lhs, rhs }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Complex) =>
                {
                    note(
                        match *op {
                            Op::Add { .. } => HostFn::ComplexAdd,
                            Op::Sub { .. } => HostFn::ComplexSub,
                            _ => HostFn::ComplexMul,
                        },
                        &mut used,
                    );
                    if plan.kinds.get(lhs as usize) == Some(Kind::Int) || plan.kinds.get(rhs as usize) == Some(Kind::Int) {
                        note(HostFn::ComplexFromI64, &mut used);
                    }
                }
                // `complex(re, im)` builds a Complex handle via the runtime.
                Op::CallBuiltin { builtin: BuiltinId::Complex, .. } if linked => note(HostFn::ComplexFromI64, &mut used),
                Op::Add { dst, lhs, rhs } | Op::Sub { dst, lhs, rhs } | Op::Mul { dst, lhs, rhs }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Modular) =>
                {
                    note(match *op { Op::Add { .. } => HostFn::ModularAdd, Op::Sub { .. } => HostFn::ModularSub, _ => HostFn::ModularMul }, &mut used);
                    if plan.kinds.get(lhs as usize) == Some(Kind::Int) || plan.kinds.get(rhs as usize) == Some(Kind::Int) { note(HostFn::ModularFromI64, &mut used); }
                }
                Op::CallBuiltin { builtin: BuiltinId::Modular, .. } if linked => note(HostFn::ModularFromI64, &mut used),
                Op::Add { dst, lhs, rhs } | Op::Sub { dst, lhs, rhs } | Op::Mul { dst, lhs, rhs }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Decimal) =>
                {
                    note(match *op { Op::Add { .. } => HostFn::DecimalAdd, Op::Sub { .. } => HostFn::DecimalSub, _ => HostFn::DecimalMul }, &mut used);
                    if plan.kinds.get(lhs as usize) == Some(Kind::Int) || plan.kinds.get(rhs as usize) == Some(Kind::Int) { note(HostFn::DecimalFromI64, &mut used); }
                }
                Op::CallBuiltin { builtin: BuiltinId::Decimal, .. } if linked => note(HostFn::DecimalFromText, &mut used),
                Op::Add { dst, lhs, rhs } | Op::Sub { dst, lhs, rhs }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Money) =>
                {
                    let _ = (lhs, rhs);
                    note(match *op { Op::Add { .. } => HostFn::MoneyAdd, _ => HostFn::MoneySub }, &mut used);
                }
                Op::CallBuiltin { builtin: BuiltinId::Money, .. } if linked => { note(HostFn::MoneyFromDecimal, &mut used); note(HostFn::MoneyFromI64, &mut used); }
                Op::Add { dst, lhs, rhs } | Op::Sub { dst, lhs, rhs } | Op::Mul { dst, lhs, rhs } | Op::Div { dst, lhs, rhs }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Quantity) =>
                {
                    let _ = (lhs, rhs);
                    note(match *op {
                        Op::Add { .. } => HostFn::QuantityAdd,
                        Op::Sub { .. } => HostFn::QuantitySub,
                        Op::Mul { .. } => HostFn::QuantityMul,
                        _ => HostFn::QuantityDiv,
                    }, &mut used);
                }
                Op::CallBuiltin { builtin: BuiltinId::Quantity, .. } if linked => note(HostFn::QuantityOfI64, &mut used),
                Op::CallBuiltin { builtin: BuiltinId::Convert, .. } if linked => note(HostFn::QuantityConvert, &mut used),
                // `+ - * /` on a Rational operand (linker): the BigInt-backed runtime op + a `from_i64`/
                // `from_bigint` promotion for any Int/BigInt operand that must widen to a Rational first.
                // (`/` here is a bare `Div` on two Rationals — the exact-division literal is `ExactDiv`.)
                Op::Add { dst, lhs, rhs } | Op::Sub { dst, lhs, rhs } | Op::Mul { dst, lhs, rhs } | Op::Div { dst, lhs, rhs }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Rational) =>
                {
                    note(match *op {
                        Op::Add { .. } => HostFn::RationalAdd,
                        Op::Sub { .. } => HostFn::RationalSub,
                        Op::Mul { .. } => HostFn::RationalMul,
                        _ => HostFn::RationalDiv,
                    }, &mut used);
                    for r in [lhs, rhs] {
                        match plan.kinds.get(r as usize) {
                            Some(Kind::Int) => note(HostFn::RationalFromI64, &mut used),
                            Some(Kind::BigInt) => note(HostFn::RationalFromBigint, &mut used),
                            _ => {}
                        }
                    }
                }
                // `a / b` in a Rational context (`ExactDiv`, linker): the BigInt-backed division + the same
                // operand promotions (an Int/Int `7 / 2` promotes both, `r / 2` promotes only the Int).
                Op::ExactDiv { lhs, rhs, .. } if linked => {
                    note(HostFn::RationalDiv, &mut used);
                    for r in [lhs, rhs] {
                        match plan.kinds.get(r as usize) {
                            Some(Kind::Int) => note(HostFn::RationalFromI64, &mut used),
                            Some(Kind::BigInt) => note(HostFn::RationalFromBigint, &mut used),
                            _ => {}
                        }
                    }
                }
                // `floor`/`ceil`/`round`/`abs` of a Rational (linker): the exact `logos_rt_rational_*` sink.
                Op::CallBuiltin { builtin: b @ (BuiltinId::Floor | BuiltinId::Ceil | BuiltinId::Round | BuiltinId::Abs), args_start, .. }
                    if linked && plan.kinds.get(args_start as usize) == Some(Kind::Rational) =>
                {
                    note(match b {
                        BuiltinId::Floor => HostFn::RationalFloor,
                        BuiltinId::Ceil => HostFn::RationalCeil,
                        BuiltinId::Round => HostFn::RationalRound,
                        _ => HostFn::RationalAbs,
                    }, &mut used);
                }
                // The `Uuid`-producing / -reading builtins (linker): each maps to its `logos_rt_uuid_*` sink.
                Op::CallBuiltin { builtin: b @ (BuiltinId::Uuid | BuiltinId::UuidNil | BuiltinId::UuidMax | BuiltinId::UuidDns | BuiltinId::UuidUrl | BuiltinId::UuidOid | BuiltinId::UuidX500 | BuiltinId::UuidVersion), .. } if linked => {
                    note(match b {
                        BuiltinId::Uuid => HostFn::UuidParse,
                        BuiltinId::UuidNil => HostFn::UuidNil,
                        BuiltinId::UuidMax => HostFn::UuidMax,
                        BuiltinId::UuidDns => HostFn::UuidDns,
                        BuiltinId::UuidUrl => HostFn::UuidUrl,
                        BuiltinId::UuidOid => HostFn::UuidOid,
                        BuiltinId::UuidX500 => HostFn::UuidX500,
                        _ => HostFn::UuidVersion,
                    }, &mut used);
                }
                // `uuid_from_bytes(seq)` boxes a Uuid from a packed 16-byte block via the runtime.
                Op::CallBuiltin { builtin: BuiltinId::UuidFromBytes, .. } if linked => note(HostFn::UuidFromPtr, &mut used),
                // The four SHA-1 SHA-NI ops call the `logos_rt_sha1*` runtime (base::sha_ops spec).
                Op::CallBuiltin { builtin: b @ (BuiltinId::Sha1Rnds4 | BuiltinId::Sha1Msg1 | BuiltinId::Sha1Msg2 | BuiltinId::Sha1Nexte), .. } if linked => {
                    note(match b {
                        BuiltinId::Sha1Rnds4 => HostFn::Sha1Rnds4,
                        BuiltinId::Sha1Msg1 => HostFn::Sha1Msg1,
                        BuiltinId::Sha1Msg2 => HostFn::Sha1Msg2,
                        _ => HostFn::Sha1Nexte,
                    }, &mut used);
                }
                // Lane-wise `Lanes + Lanes` / `lanes += lanes` → the `logos_rt_lanes4_add` runtime.
                Op::Add { lhs, .. } if linked && plan.kinds.get(lhs as usize) == Some(Kind::Lanes) => note(HostFn::Lanes4Add, &mut used),
                Op::AddAssign { dst, .. } if linked && plan.kinds.get(dst as usize) == Some(Kind::Lanes) => note(HostFn::Lanes4Add, &mut used),
                // Lane-wise `Lanes xor Lanes` → the `logos_rt_lanes4_xor` runtime.
                Op::BitXor { lhs, .. } if linked && plan.kinds.get(lhs as usize) == Some(Kind::Lanes) => note(HostFn::Lanes4Xor, &mut used),
                // `Moment/Date ± Span` calendar arithmetic (linker) — the base's width picks the sink.
                Op::Add { lhs, rhs, .. } | Op::Sub { lhs, rhs, .. }
                    if linked && (plan.kinds.get(lhs as usize) == Some(Kind::Span) || plan.kinds.get(rhs as usize) == Some(Kind::Span)) =>
                {
                    let base = if plan.kinds.get(lhs as usize) == Some(Kind::Span) { rhs } else { lhs };
                    note(if plan.kinds.get(base as usize) == Some(Kind::Date) { HostFn::DateAddSpan } else { HostFn::MomentAddSpan }, &mut used);
                }
                // `uuid == uuid` (equality on two Uuid operands, linker) — the 16-byte `logos_rt_uuid_eq`.
                Op::Eq { lhs, rhs, .. } | Op::NotEq { lhs, rhs, .. }
                    if linked && plan.kinds.get(lhs as usize) == Some(Kind::Uuid) && plan.kinds.get(rhs as usize) == Some(Kind::Uuid) =>
                {
                    note(HostFn::UuidEq, &mut used);
                }
                Op::LoadToday { .. } => note(HostFn::Today, &mut used),
                Op::LoadNow { .. } => note(HostFn::Now, &mut used),
                Op::Args { .. } => note(HostFn::Args, &mut used),
                Op::CallBuiltin { builtin: BuiltinId::ParseInt, .. } => note(HostFn::ParseInt, &mut used),
                Op::CallBuiltin { builtin: BuiltinId::ParseFloat, .. } => note(HostFn::ParseFloat, &mut used),
                Op::CallBuiltin { builtin: BuiltinId::ParseTimestamp, .. } => note(HostFn::ParseTimestamp, &mut used),
                // `writeWireResidual(text)` frames the Text (`[len:u32][bytes]`) out to the host sink.
                Op::CallBuiltin { builtin: BuiltinId::WriteWireResidual, .. } => note(HostFn::WriteWireResidual, &mut used),
                // LINKER-mode extended temporal (calendar logic in `base::temporal`): `format_timestamp`
                // → a `Text` handle, `months_between`/`years_between` → an `Int`.
                Op::CallBuiltin { builtin: BuiltinId::FormatTimestamp, .. } if linked => note(HostFn::FormatTimestampRt, &mut used),
                Op::CallBuiltin { builtin: BuiltinId::MonthsBetween, .. } if linked => note(HostFn::MonthsBetweenRt, &mut used),
                Op::CallBuiltin { builtin: BuiltinId::YearsBetween, .. } if linked => note(HostFn::YearsBetweenRt, &mut used),
                // `in_zone(m, "zone")` → a `Text` (local wall-clock), `local_instant(m, "zone")` → a `Moment`.
                Op::CallBuiltin { builtin: BuiltinId::InZone, .. } if linked => note(HostFn::InZoneRt, &mut used),
                Op::CallBuiltin { builtin: BuiltinId::LocalInstant, .. } if linked => note(HostFn::LocalInstantRt, &mut used),
                // The general SIMD lane vocabulary (`base::LanesVal`) — the SSE byte/word-lane ops a Logos
                // codec compiles to. Each notes its `logos_rt_lanes_*` runtime fn (linker mode only).
                Op::CallBuiltin { builtin: b, .. } if linked && lanes_v_host_fn(b).is_some() => {
                    note(lanes_v_host_fn(b).unwrap(), &mut used)
                }
                // Money FX (linker): `to_currency`→convert; `set_rate` installs one rate (coercing the
                // rate arg Int→Rational / Decimal→Rational); `set_rates` installs a whole Map (the runtime
                // reads it — all three value-kind variants are noted since the map's value kind is resolved
                // at lowering; an imported-but-uncalled runtime fn is harmless, all are defined).
                Op::CallBuiltin { builtin: BuiltinId::ToCurrency, .. } if linked => note(HostFn::MoneyToCurrency, &mut used),
                Op::CallBuiltin { builtin: BuiltinId::SetRate, args_start, .. } if linked => {
                    note(HostFn::MoneySetRate, &mut used);
                    match plan.kinds.get((args_start + 1) as usize) {
                        Some(Kind::Int) => note(HostFn::RationalFromI64, &mut used),
                        Some(Kind::Decimal) => note(HostFn::DecimalToRational, &mut used),
                        _ => {}
                    }
                }
                Op::CallBuiltin { builtin: BuiltinId::SetRates, .. } if linked => {
                    note(HostFn::MoneySetRatesInt, &mut used);
                    note(HostFn::MoneySetRatesRational, &mut used);
                    note(HostFn::MoneySetRatesDecimal, &mut used);
                }
                // `wireBytes(value)` — marshal the value via the REAL codec (by the arg's kind).
                Op::CallBuiltin { builtin: BuiltinId::WireBytes, args_start, .. } if linked => {
                    if let Some(h) = wire_bytes_host_fn(plan.kinds.get(args_start as usize)) {
                        note(h, &mut used);
                    }
                }
                // `readWireProgram()` reads a host frame then DECODES it to a dynamic value; `run_accepted`
                // sandbox-evals a wire-received shipped function through the acceptance contract.
                Op::CallBuiltin { builtin: BuiltinId::ReadWireProgram, .. } if linked => {
                    note(HostFn::ReadWireFrame, &mut used);
                    note(HostFn::ReadWireProgramRt, &mut used);
                    // `readWireProgram` bump-allocs a receive buffer via `emit_alloc`, which in LINKER mode
                    // calls `logos_rt_alloc` — it MUST be imported, else `emit_alloc` falls back to the
                    // `__heap_ptr` global (undeclared in a linked module → an invalid global relocation).
                    note(HostFn::RtAlloc, &mut used);
                }
                Op::CallBuiltin { builtin: BuiltinId::RunAccepted, .. } if linked => note(HostFn::RunAccepted, &mut used),
                Op::CallBuiltin {
                    builtin:
                        BuiltinId::YearOf | BuiltinId::MonthOf | BuiltinId::DayOf | BuiltinId::WeekdayOf | BuiltinId::HourOf
                        | BuiltinId::MinuteOf | BuiltinId::SecondOf | BuiltinId::WeekOf | BuiltinId::QuarterOf,
                    args_start,
                    ..
                } => {
                    // A `Date` argument uses the day-based `temporal_component_date`; a `Moment` uses the
                    // nanos-based `temporal_component`. (An unknown-kind arg in dead code notes nothing.)
                    if plan.kinds.get(args_start as usize) == Some(Kind::Date) {
                        note(HostFn::TemporalComponentDate, &mut used);
                    } else {
                        note(HostFn::TemporalComponent, &mut used);
                    }
                }
                // `format(x)` stringifies its argument with the same host formatter a Concat operand uses.
                Op::CallBuiltin { builtin: BuiltinId::Format, args_start, arg_count, .. } if arg_count > 0 => {
                    note_operand_fmt(plan, args_start, &mut used);
                }
                // A `Concat` — or a Text-typed `+`/`+=` (string concatenation) — stringifies its
                // operands; a non-Text operand needs its host formatter.
                Op::Concat { lhs, rhs, .. } => {
                    note_operand_fmt(plan, lhs, &mut used);
                    note_operand_fmt(plan, rhs, &mut used);
                }
                Op::Add { dst, lhs, rhs } if plan.kinds.get(dst as usize) == Some(Kind::Text) => {
                    note_operand_fmt(plan, lhs, &mut used);
                    note_operand_fmt(plan, rhs, &mut used);
                }
                Op::AddAssign { dst, src } if plan.kinds.get(dst as usize) == Some(Kind::Text) => {
                    note_operand_fmt(plan, dst, &mut used);
                    note_operand_fmt(plan, src, &mut used);
                }
                // A formatted piece: a `.N` precision spec (`"{x:.9}"`) uses the float-precision host; an
                // alignment/width spec (`"{x:>6}"`) stringifies the value (its own formatter) then pads
                // via `fmt_align_into`. The lowering re-derives the exact spec; here we just ensure every
                // host it can reach is imported.
                Op::FormatValue { src, spec, .. } => {
                    let spec_s = match spec {
                        u32::MAX => None,
                        idx => match program.constants.get(idx as usize) {
                            Some(Constant::Text(s)) => Some(s.as_str()),
                            _ => None,
                        },
                    };
                    if matches!(spec_s, Some(s) if s.starts_with('.')) {
                        note(HostFn::FmtF64PrecInto, &mut used);
                    } else {
                        note(HostFn::FmtAlignInto, &mut used);
                        note_fmt_kind(plan.kinds.get(src as usize), &mut used);
                    }
                }
                _ => {}
            }
        }
    }
    // LINKER MODE with an emitter heap or an iterator stack imports the runtime allocator, to seed each
    // one's slab at the `main` prologue.
    if linked && (uses_heap || uses_iter) {
        note(HostFn::RtAlloc, &mut used);
    }
    // Re-sort into the canonical HOST_FNS order so indices are deterministic.
    let imports: Vec<HostFn> = HOST_FNS.iter().copied().filter(|h| used.contains(h)).collect();
    let host_index = |h: HostFn| -> Option<u32> { imports.iter().position(|x| *x == h).map(|i| i as u32) };
    let num_imports = imports.len() as u32;
    let main_index = num_imports;
    let fn_base = num_imports + 1; // wasm index of program.functions[0]

    // ---- 3. Type table: host functions, then Main, then each function ----
    let mut types = TypeTable::default();
    let host_type: Vec<u32> = imports.iter().map(|h| types.intern(h.params(), h.results())).collect();
    let main_type = types.intern(vec![], result_valtypes(main.result));
    let mut fn_types: Vec<u32> = Vec::with_capacity(fns.len());
    let mut fn_param_valtypes: Vec<Vec<u8>> = Vec::with_capacity(fns.len());
    for p in &fns {
        let params: Vec<u8> = (0..p.num_params).map(|r| p.kinds.valtype(r as usize)).collect();
        fn_types.push(types.intern(params.clone(), result_valtypes(p.result)));
        fn_param_valtypes.push(params);
    }

    // ---- 4. Emit each function body, with the call/host context resolved ----
    let heap_global = num_globals as u32; // `__heap_ptr` follows the user globals
    let iter_global = heap_global + u32::from(uses_heap); // `__iter_sp` follows `__heap_ptr`
    let fn_results: Vec<Option<Kind>> = fns.iter().map(|p| p.result).collect();
    // `capture_kinds` (computed in the planning pass, global + local) is the same source the closure
    // body signatures were seeded from, so `MakeClosure`'s store / `CallValue`'s load / the signature
    // all agree on each capture's valtype.
    let rt_alloc = if linked && uses_heap { host_index(HostFn::RtAlloc) } else { None };
    let ctx = Ctx { constants: &program.constants, host_index: &host_index, fn_base, heap_global, iter_global, fn_type: &fn_types, fn_param_valtypes: &fn_param_valtypes, fn_results: &fn_results, functions: &program.functions, capture_kinds: &capture_kinds, enum_types: &program.enum_types, struct_types: &program.struct_types, policies, interner, linked, rt_alloc };
    let mut main_body = emit_body(&main, &ctx)?;
    // LINKER MODE iterator-stack prologue: the emitter heap draws each block straight from the runtime
    // allocator (see [`emit_alloc`]), so it is UNBOUNDED and needs no slab. The ITERATOR STACK, though, is
    // a contiguous DOWN-growing region, so it's seeded from the TOP of one runtime `logos_rt_alloc` SLAB
    // (which `dlmalloc` owns → no collision). Spliced right after `main`'s local declarations
    // (`encode_locals` is the body's exact prefix), so it runs before any iteration: `__iter_sp` inits to
    // 0 in the global section, then this sets it to `slab_base + SLAB` = the slab top.
    if linked && uses_iter {
        const SLAB: i32 = (HEAP_PAGES * 65536) as i32;
        let rt_alloc = host_index(HostFn::RtAlloc).ok_or(WasmLowerError::Unsupported("logos_rt_alloc not imported"))?;
        let locals_len = encode_locals(&main).len();
        let mut prologue = Vec::new();
        i32_const(&mut prologue, SLAB);
        prologue.push(0x10); // call logos_rt_alloc(SLAB)
        leb_u32(&mut prologue, rt_alloc);
        i32_const(&mut prologue, SLAB);
        prologue.push(0x6A); // i32.add → slab base + SLAB = the slab TOP
        prologue.push(0x24); // global.set __iter_sp (= slab top; the iterator stack grows down)
        leb_u32(&mut prologue, iter_global);
        main_body.splice(locals_len..locals_len, prologue);
    }
    let mut fn_bodies = Vec::with_capacity(fns.len());
    for p in &fns {
        fn_bodies.push(emit_body(p, &ctx)?);
    }

    // ---- 5. Assemble the module ----
    let mut module = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
    section(&mut module, 1, &types.encode());

    // Import section (id 2): each host sink as a function import in `env`. In LINKER mode the program
    // also imports the ONE shared linear memory — `rust-lld` creates the output memory and resolves this
    // import (and the base runtime object's own `env.__linear_memory`) to it. A memory import occupies no
    // function-index slot, so the host functions keep their `host_index` indices regardless.
    let mut imp = Vec::new();
    leb_u32(&mut imp, imports.len() as u32 + u32::from(linked));
    for (i, h) in imports.iter().enumerate() {
        encode_name(&mut imp, "env");
        encode_name(&mut imp, h.field());
        imp.push(0x00); // import kind: function
        leb_u32(&mut imp, host_type[i]);
    }
    if linked {
        encode_name(&mut imp, "env");
        encode_name(&mut imp, "__linear_memory");
        imp.push(0x02); // import kind: memory
        imp.push(0x00); // limits: min only
        leb_u32(&mut imp, 0); // min 0 pages — lld unions this with the runtime's need + grows on demand
    }
    section(&mut module, 2, &imp);

    // Function section (id 3): Main + each user function (type indices).
    let mut func = Vec::new();
    leb_u32(&mut func, 1 + fns.len() as u32);
    leb_u32(&mut func, main_type);
    for t in &fn_types {
        leb_u32(&mut func, *t);
    }
    section(&mut module, 3, &func);

    // Table section (id 4): one funcref table holding every user function, so a closure's
    // `CallValue` can `call_indirect` it by index. (Emitted only when the program has closures AND is
    // self-contained — linker mode direct-calls closures, so it needs no table.)
    if has_closures && !linked {
        let mut table = Vec::new();
        leb_u32(&mut table, 1); // one table
        table.push(0x70); // elemtype: funcref
        table.push(0x00); // limits: min only
        leb_u32(&mut table, fns.len() as u32); // min = number of functions
        section(&mut module, 4, &table);
    }

    // Memory section (id 5): one linear memory for the heap value model, exported as "memory". The
    // bump allocator never frees, so a build-then-scan program (a 1000-element array, an n-character
    // string built by `+`, a search that cuts a one-char Text per index) needs real headroom — one
    // page (64 KiB) overflows on any non-tiny input. `HEAP_PAGES` * 64 KiB is the address space; the
    // iterator stack grows down from its top, the heap up from 16, so they meet only at exhaustion.
    // LINKER MODE does NOT define a memory — it IMPORTS the shared one (above) and the bump allocator
    // draws from a runtime slab, so the runtime's `dlmalloc` owns the address space.
    if uses_heap && !linked {
        let mut mem = Vec::new();
        leb_u32(&mut mem, 1); // one memory
        mem.push(0x00); // limits: min only
        leb_u32(&mut mem, HEAP_PAGES); // min HEAP_PAGES pages
        section(&mut module, 5, &mem);
    }

    // Global section (id 6): one mutable global per promoted Main binding (zero-initialized),
    // plus the bump-allocator pointer `__heap_ptr` when the program uses the heap.
    if num_globals > 0 || uses_heap || uses_iter {
        let mut glob = Vec::new();
        leb_u32(&mut glob, num_globals as u32 + u32::from(uses_heap) + u32::from(uses_iter));
        for gk in &global_kinds {
            let vt = gk.map(Kind::wasm_valtype).unwrap_or(I64);
            glob.push(vt);
            glob.push(0x01); // mutable
            // The zero-initializer MUST match the global's valtype — an `i32` (handle-kind) global
            // with an `i64.const 0` init is invalid wasm. `i32` covers any heap handle (struct, enum,
            // seq, map, …) a promoted Main binding may hold.
            if vt == F64 {
                glob.push(0x44); // f64.const 0
                glob.extend_from_slice(&0f64.to_le_bytes());
            } else if vt == I32 {
                i32_const(&mut glob, 0);
            } else {
                glob.push(0x42); // i64.const 0
                leb_i64(&mut glob, 0);
            }
            glob.push(0x0B); // end of init expr
        }
        if uses_heap {
            // `__heap_ptr`: mutable i32. Self-contained: init 16 (the low 16 bytes stay reserved/null).
            // LINKER MODE: init 0 — the `main` prologue seeds it from a runtime `logos_rt_alloc` SLAB, so
            // the bump region lives inside memory the runtime's `dlmalloc` owns (no collision).
            glob.push(I32);
            glob.push(0x01); // mutable
            i32_const(&mut glob, if linked { 0 } else { 16 });
            glob.push(0x0B);
        }
        if uses_iter {
            // `__iter_sp`: mutable i32. Self-contained: init to the memory top; each `IterPrepare`
            // decrements it by a 12-byte frame, so it grows down toward the up-growing heap. LINKER MODE:
            // init 0 — the `main` prologue seeds it from the TOP of a runtime `logos_rt_alloc` SLAB.
            glob.push(I32);
            glob.push(0x01); // mutable
            i32_const(&mut glob, if linked { 0 } else { (HEAP_PAGES * 65536) as i32 });
            glob.push(0x0B);
        }
        section(&mut module, 6, &glob);
    }

    // Export section (id 7): the synthesized top-level body as `main`, plus `memory` if this module
    // DEFINES one. In linker mode the memory is imported (the linker re-exports it), so we don't.
    let export_memory = uses_heap && !linked;
    let mut export = Vec::new();
    leb_u32(&mut export, 1 + u32::from(export_memory));
    encode_name(&mut export, "main");
    export.push(0x00); // export kind: function
    leb_u32(&mut export, main_index);
    if export_memory {
        encode_name(&mut export, "memory");
        export.push(0x02); // export kind: memory
        leb_u32(&mut export, 0); // memory index 0
    }
    section(&mut module, 7, &export);

    // Element section (id 9): an active segment filling table slot `i` with function `i`'s funcref
    // (wasm index `fn_base + i`), so a closure storing function index `i` `call_indirect`s table[i].
    // Self-contained only — linker mode direct-calls closures (no table to fill).
    if has_closures && !linked {
        let mut elem = Vec::new();
        leb_u32(&mut elem, 1); // one segment
        leb_u32(&mut elem, 0); // flags 0: active, table 0, MVP funcref vec
        elem.push(0x41); // i32.const 0 (offset)
        leb_i32(&mut elem, 0);
        elem.push(0x0B); // end
        leb_u32(&mut elem, fns.len() as u32);
        for i in 0..fns.len() as u32 {
            leb_u32(&mut elem, fn_base + i);
        }
        section(&mut module, 9, &elem);
    }

    // Code section (id 10): Main, then each function (each prefixed by its byte length).
    let mut code = Vec::new();
    leb_u32(&mut code, 1 + fn_bodies.len() as u32);
    for entry in std::iter::once(&main_body).chain(fn_bodies.iter()) {
        leb_u32(&mut code, entry.len() as u32);
        code.extend_from_slice(entry);
    }
    section(&mut module, 10, &code);

    Ok(module)
}

/// The declared result kind of `program.functions[fi]` (`None` for void / undeclared) — used
/// to type `Op::Call` results before the callee's own body is inferred.
fn declared_result(program: &CompiledProgram, fi: usize) -> Option<Kind> {
    program.functions.get(fi).and_then(|f| f.ret_kind).map(Kind::from_slot)
}

/// How the flat `code` stream partitions into wasm functions. Regular (`## To`) functions are
/// appended after Main as a contiguous, entry-sorted tail. A *closure body* (the target of a
/// `MakeClosure`) is an INLINE lambda: it is emitted in the middle of its enclosing region, jumped
/// over by a `Jump` at `entry_pc - 1` (the "jover") whose target is the body's end. So Main is no
/// longer a clean prefix — it is `[0, main_end)` with the inline closure bodies excised.
struct CodeLayout {
    /// Main's region end (exclusive): the first regular-function entry, or `code.len()`.
    main_end: usize,
    /// Per-function `[start, end)` in the code stream.
    func_region: Vec<(usize, usize)>,
    /// Every closure body's `[start, end)` — the holes to excise from an enclosing region.
    closure_ranges: Vec<(usize, usize)>,
    /// `is_closure[fi]` — whether function `fi` is a closure body (a `MakeClosure` target).
    is_closure: Vec<bool>,
}

/// Resolve [`CodeLayout`] from the program: classify each function (a `MakeClosure` target is a
/// closure body), find each closure body's `[entry, jover_target)` range, and place the regular
/// functions as the contiguous tail after `main_end`.
fn code_layout(program: &CompiledProgram) -> R<CodeLayout> {
    let code_len = program.code.len();
    let nf = program.functions.len();
    let mut is_closure = vec![false; nf];
    for op in &program.code {
        if let Op::MakeClosure { func, .. } = *op {
            if (func as usize) < nf {
                is_closure[func as usize] = true;
            }
        }
    }
    let mut func_region = vec![(0usize, 0usize); nf];
    let mut closure_ranges = Vec::new();
    for (fi, f) in program.functions.iter().enumerate() {
        if is_closure[fi] {
            let entry = f.entry_pc;
            let end = match (entry >= 1).then(|| program.code.get(entry - 1)).flatten() {
                Some(Op::Jump { target }) if *target > entry && *target <= code_len => *target,
                _ => return Err(WasmLowerError::Unsupported("closure body without a jump-over")),
            };
            func_region[fi] = (entry, end);
            closure_ranges.push((entry, end));
        }
    }
    // Regular functions: the entry-sorted tail. Each ends at the next regular entry (closures
    // interleaved inside it become holes), the last at `code_len`.
    let mut regular: Vec<usize> = (0..nf).filter(|&fi| !is_closure[fi]).collect();
    regular.sort_by_key(|&fi| program.functions[fi].entry_pc);
    let main_end = regular.first().map(|&fi| program.functions[fi].entry_pc).unwrap_or(code_len);
    for w in 0..regular.len() {
        let fi = regular[w];
        let start = program.functions[fi].entry_pc;
        let end = regular.get(w + 1).map(|&g| program.functions[g].entry_pc).unwrap_or(code_len);
        func_region[fi] = (start, end);
    }
    Ok(CodeLayout { main_end, func_region, closure_ranges, is_closure })
}

/// The maximal closure ranges contained in `[start, end)` (the DIRECT inline-closure children of a
/// region) — a closure nested inside another closure is excised with its parent, not here.
fn child_holes(closure_ranges: &[(usize, usize)], start: usize, end: usize) -> Vec<(usize, usize)> {
    closure_ranges
        .iter()
        .copied()
        .filter(|&(e, t)| {
            e >= start
                && t <= end
                && (e, t) != (start, end)
                && !closure_ranges
                    .iter()
                    .any(|&(e2, t2)| (e2, t2) != (e, t) && e2 <= e && t <= t2 && e2 >= start && t2 <= end)
        })
        .collect()
}

/// Extract one wasm function's op slice from `code[start..end)`: drop the `holes` (inline closure
/// bodies, each a separate wasm function) and rebase every jump-like target to the kept ops'
/// 0-based indices. A target that escapes the region or lands inside a hole is a hard error.
fn extract_region(code: &[Op], start: usize, end: usize, holes: &[(usize, usize)]) -> R<Vec<Op>> {
    let in_hole = |pc: usize| holes.iter().any(|&(s, e)| pc >= s && pc < e);
    let mut new_index = vec![usize::MAX; end];
    let mut kept = Vec::new();
    for pc in start..end {
        if !in_hole(pc) {
            new_index[pc] = kept.len();
            kept.push(pc);
        }
    }
    let rebase = |t: usize| -> R<usize> {
        if t >= start && t < end && new_index[t] != usize::MAX {
            Ok(new_index[t])
        } else {
            Err(WasmLowerError::Unsupported("jump target escapes the function"))
        }
    };
    let mut ops = Vec::with_capacity(kept.len());
    for &pc in &kept {
        ops.push(match code[pc] {
            Op::Jump { target } => Op::Jump { target: rebase(target)? },
            Op::JumpIfFalse { cond, target } => Op::JumpIfFalse { cond, target: rebase(target)? },
            Op::JumpIfTrue { cond, target } => Op::JumpIfTrue { cond, target: rebase(target)? },
            Op::IterNext { dst, exit } => Op::IterNext { dst, exit: rebase(exit)? },
            other => other,
        });
    }
    Ok(ops)
}

/// Plan the synthesized top-level `main`: `code[0 .. main_end)` with inline closure bodies excised,
/// the Main register frame, no parameters, void result (it ends in `Halt`).
fn plan_main(
    program: &CompiledProgram,
    layout: &CodeLayout,
    ret_of: &dyn Fn(usize) -> Option<Kind>,
    global_of: &dyn Fn(u16) -> Option<Kind>,
    closure_ret: &dyn Fn(usize) -> Option<Kind>,
    ret_layout: &dyn Fn(u16) -> Option<FieldLayout>,
    fn_return_closure: &dyn Fn(u16) -> Option<u16>,
    linked: bool,
) -> R<Plan> {
    let holes = child_holes(&layout.closure_ranges, 0, layout.main_end);
    let ops = extract_region(&program.code, 0, layout.main_end, &holes)?;
    let num_regs = program.register_count as u32;
    // Split any register the VM reused across disjoint live ranges of conflicting wasm types into one
    // local per range (Main has no parameters to pin). Identity unless a real conflict exists.
    let (ops, num_regs) = regsplit::split_registers(&ops, num_regs, 0, &program.functions);
    let fn_return_types = fn_return_types(program);
    let (kinds, _pc_reach, reachable_blocks) =
        infer_with_reachability(&ops, &program.constants, &program.struct_types, &program.enum_types, &fn_return_types, num_regs as usize, &[], ret_of, global_of, closure_ret, ret_layout, fn_return_closure, &[], &[], linked, &program.functions)?;
    let structs = kind::struct_layout(&ops, &program.constants, &program.struct_types, &program.enum_types, &fn_return_types, ret_layout, fn_return_closure, &[], &[]);
    let cow_inserts = cow_struct_inserts(&ops, num_regs, &program.functions);
    let reg_shape = complete_reg_shape(&structs, &kinds, program);
    Ok(Plan { ops, kinds, num_params: 0, num_regs, result: None, reachable_blocks, structs, cow_inserts, reg_shape, return_closure: None, stub: false })
}

/// Each function's declared RETURN type, indexed by function index — for typing a caller's inline use
/// of a returned composite (`item k of f()`, `Inspect f()`).
fn fn_return_types(program: &CompiledProgram) -> Vec<Option<BoundaryType>> {
    program.functions.iter().map(|f| f.return_type.clone()).collect()
}

/// Complete a plan's per-register access SHAPE: `struct_layout`'s `reg_shape` (parameter + cross-region
/// + locally-built struct, all kind-free) EXTENDED with locally-built map / enum / heterogeneous-tuple
/// shapes — which need the inferred register kinds, unavailable inside `struct_layout`. A param shape
/// already present is never overridden. A captured local-built composite then resolves like any other.
fn complete_reg_shape(
    structs: &kind::StructLayout,
    kinds: &KindTable,
    program: &CompiledProgram,
) -> std::collections::HashMap<u16, ParamShape> {
    let mut out = structs.reg_shape.clone();
    for (reg, value_reg) in &structs.map_set_value {
        if !out.contains_key(reg) {
            if let Some(vk) = kinds.get(*value_reg as usize) {
                out.insert(*reg, ParamShape::Map(vk));
            }
        }
    }
    for (reg, elems) in &structs.tuple_layouts {
        if !out.contains_key(reg) {
            if let Some(ks) = elems.iter().map(|e| kinds.get(*e as usize)).collect::<Option<Vec<Kind>>>() {
                // Heterogeneous only — a homogeneous tuple lays out as a self-describing `Seq`.
                if !ks.windows(2).all(|w| w[0] == w[1]) {
                    out.insert(*reg, ParamShape::Tuple(ks));
                }
            }
        }
    }
    for (reg, name) in &structs.ind_type_of {
        if !out.contains_key(reg) {
            if let Some(variants) = kind::resolve_enum_variants(name, &program.enum_types) {
                out.insert(*reg, ParamShape::Enum(variants));
            }
        }
    }
    out
}

/// Each closure's captured VALUE kinds (by function index, then capture index), read from where the
/// closure is BUILT (`MakeClosure`) — in Main or a non-closure function (the `region_plans`, by
/// function index; closures are `None` there). A captured GLOBAL resolves via `global_of`; a captured
/// LOCAL from the enclosing region's inferred register kind. A capture not reachable here (e.g. one
/// built inside another closure body, which this pre-pass doesn't plan) stays `None` → seeded `Int`,
/// so a composite such capture rejects soundly rather than miscompiling.
fn extract_capture_kinds(
    program: &CompiledProgram,
    main_p1: &Plan,
    region_plans: &[Option<Plan>],
    global_of: &dyn Fn(u16) -> Option<Kind>,
    global_closure_of: &dyn Fn(u16) -> Option<u16>,
) -> (Vec<Vec<Option<Kind>>>, Vec<Vec<Option<ParamShape>>>, Vec<Vec<Option<u16>>>) {
    let mut kinds: Vec<Vec<Option<Kind>>> = program
        .functions
        .iter()
        .map(|f| f.captures.iter().map(|(_, g)| g.and_then(|gi| global_of(gi))).collect())
        .collect();
    let mut shapes: Vec<Vec<Option<ParamShape>>> =
        program.functions.iter().map(|f| vec![None; f.captures.len()]).collect();
    // Per closure fi, per capture k: the body function index of a captured CLOSURE value (so calling
    // the captured closure resolves — closure composition like `(n) -> add1(add1(n))`). A captured
    // GLOBAL closure (a Main-`Let` closure promoted to a global) is resolved here; a captured LOCAL
    // closure is filled from the build-site `closure_of` in the loop below.
    let mut closures: Vec<Vec<Option<u16>>> = program
        .functions
        .iter()
        .map(|f| f.captures.iter().map(|(_, g)| g.and_then(|gi| global_closure_of(gi))).collect())
        .collect();
    for plan in std::iter::once(main_p1).chain(region_plans.iter().flatten()) {
        for op in &plan.ops {
            if let Op::MakeClosure { func, locals_start, .. } = *op {
                let fi = func as usize;
                let mut local_k: u16 = 0;
                for (k, (_sym, global)) in program.functions[fi].captures.iter().enumerate() {
                    if global.is_none() {
                        let reg = locals_start + local_k;
                        local_k += 1;
                        if let Some(kind) = plan.kinds.get(reg as usize) {
                            kinds[fi][k] = Some(kind);
                        }
                        // A captured composite's SHAPE comes from the enclosing region's unified
                        // `reg_shape` at the capture register — covering a function PARAMETER (struct/
                        // map/enum/het-tuple) and a locally-built struct, so the closure body resolves
                        // `capturedstruct's field` / `item k of capturedmap` / `Inspect capturedenum`
                        // exactly as a composite parameter does.
                        if let Some(shape) = plan.reg_shape.get(&reg) {
                            shapes[fi][k] = Some(shape.clone());
                        }
                        // A captured CLOSURE value: its statically-traced body function index at the
                        // build site, so the closure body can `call_indirect` the captured closure.
                        if let Some(&c) = plan.structs.closure_of.get(&reg) {
                            closures[fi][k] = Some(c);
                        }
                    }
                }
            }
        }
    }
    (kinds, shapes, closures)
}

/// Plan `program.functions[fi]`: extract its region (regular functions are the appended tail; a
/// closure body is its inline `[entry, jover_target)` slice), excise any inline closures nested
/// inside it, rebase jump targets to 0-based, seed parameter kinds, infer register kinds, and
/// resolve its result kind (the declared `ret_kind`, else inferred from its `Return` operands).
/// A *capturing* closure (one that reads an enclosing local) needs the capture ABI and is deferred.
#[allow(clippy::too_many_arguments)]
fn plan_function(
    program: &CompiledProgram,
    fi: usize,
    layout: &CodeLayout,
    ret_of: &dyn Fn(usize) -> Option<Kind>,
    global_of: &dyn Fn(u16) -> Option<Kind>,
    closure_ret: &dyn Fn(usize) -> Option<Kind>,
    ret_layout: &dyn Fn(u16) -> Option<FieldLayout>,
    fn_return_closure: &dyn Fn(u16) -> Option<u16>,
    param_origin: &dyn Fn(usize, usize) -> Option<u16>,
    capture_kinds: &[Vec<Option<Kind>>],
    capture_shapes: &[Vec<Option<ParamShape>>],
    capture_closures: &[Vec<Option<u16>>],
    strict: bool,
    linked: bool,
) -> R<Plan> {
    let f = &program.functions[fi];
    let (start, end) = layout.func_region[fi];
    if start >= end || end > program.code.len() {
        return Err(WasmLowerError::Unsupported("malformed function bounds"));
    }
    let holes = child_holes(&layout.closure_ranges, start, end);
    let ops = extract_region(&program.code, start, end, &holes)?;

    // A capturing closure body receives, after its real parameters, each capture's VALUE and a
    // present-FLAG (regs `p..p+cap_n` and `p+cap_n..p+2·cap_n`; see `CallValue`'s frame setup).
    // Real params + flags are Int by the closure contract; each capture VALUE is typed by its source
    // kind (`capture_kinds[fi]`, computed once from the `MakeClosure` site — a captured global's kind
    // or a captured local's enclosing-region kind), so closing over a composite (a `Seq`/`Map`/struct/
    // … handle) works whether it's global or function-local. `CallValue` passes them; `MakeClosure`
    // fills the closure object they are loaded from. A non-capturing function uses its declared kinds.
    let cap_n = f.captures.len() as u32;
    let num_params = f.param_count as u32 + 2 * cap_n;
    let mut seeds: Vec<Option<Kind>> = if cap_n > 0 {
        let mut s = vec![Some(Kind::Int); num_params as usize];
        // Real params take their DECLARED kind (a `(n: Float)` capturing closure must seed `n` as f64,
        // not the all-Int flag/entry-guard default) — the capturing-closure analog of the non-capturing
        // `function_param_seeds`. Capture VALUES then take their source kind; the present-FLAGS stay Int.
        for i in 0..f.param_count as usize {
            if let Some(Some(bt)) = f.param_types.get(i) {
                if let Some(k) = kind::boundary_to_kind(bt) {
                    s[i] = Some(k);
                }
            }
        }
        let caps = capture_kinds.get(fi).map(|v| v.as_slice()).unwrap_or(&[]);
        for k in 0..f.captures.len() {
            if let Some(kind) = caps.get(k).copied().flatten() {
                s[f.param_count as usize + k] = Some(kind);
            }
        }
        s
    } else {
        kind::function_param_seeds(f)
    };
    let num_regs = f.register_count as u32;
    // Split any register reused across disjoint live ranges of conflicting wasm types; parameters
    // (and capture/range members) are pinned so the function signature and range operands are
    // untouched. Identity unless a real conflict exists.
    let (ops, num_regs) = regsplit::split_registers(&ops, num_regs, num_params, &program.functions);
    // Struct PARAMETERS' field layouts, resolved from the bytecode's `struct_types` — so `p's field`
    // resolves inside a `f(p: Point)` body (parameters are pinned by the splitter, so their registers
    // are unchanged). Only non-capturing functions have declared parameter types.
    let mut param_layouts = if cap_n == 0 { param_seeds(f, program) } else { Vec::new() };
    // A capture of a composite GLOBAL is typed with that global's SHAPE (the capture analog of a
    // composite parameter), so `item k of capturedmap` / `capturedstruct's field` / `Inspect
    // capturedenum` resolve in the closure body. The capture VALUE register (after the real params)
    // already has the right Kind seed; this adds its layout/value-kind/variant resolution.
    let local_shapes = capture_shapes.get(fi).map(|v| v.as_slice()).unwrap_or(&[]);
    for (k, (_sym, global)) in f.captures.iter().enumerate() {
        let shape = if let Some(gidx) = global {
            // A captured GLOBAL composite is typed by the global's resolved type (`global_types`).
            program
                .global_types
                .get(*gidx as usize)
                .and_then(|t| t.as_ref())
                .and_then(|bt| boundary_to_param_shape(bt, program))
        } else {
            // A captured LOCAL composite is typed by its shape read from the enclosing region's plan.
            local_shapes.get(k).cloned().flatten()
        };
        if let Some(shape) = shape {
            param_layouts.push((f.param_count + k as u16, shape));
        }
    }
    // A closure PARAMETER whose single statically-known origin a whole-program pass resolved: its
    // register (= the param index, pinned by the splitter) is pre-bound to that closure body, so
    // `f(args)` inside this function resolves its callee like a returned/local closure (closures as
    // arguments). Only the real params (`0..param_count`) — captures/flags are never closure args.
    let mut param_closures: Vec<(u16, u16)> =
        (0..f.param_count).filter_map(|i| param_origin(fi, i as usize).map(|c| (i, c))).collect();
    // A captured CLOSURE value (register `param_count + k`) is bound to its traced body function, so
    // calling the captured closure resolves its callee like a local one — closure composition.
    if let Some(caps) = capture_closures.get(fi) {
        for (k, c) in caps.iter().enumerate() {
            if let Some(c) = c {
                param_closures.push((f.param_count + k as u16, *c));
            }
        }
    }
    // A closure PARAMETER carries an i32 handle, not the i64 its `Closure` declared type would otherwise
    // seed (there is no `BoundaryType::Closure`). The origin pass already proved this param IS a closure,
    // so type it `Kind::Closure` — the closure-handle argument then matches and lowers/loads as i32.
    for &(reg, _) in &param_closures {
        if let Some(slot) = seeds.get_mut(reg as usize) {
            *slot = Some(Kind::Closure);
        }
    }
    let fn_rt = fn_return_types(program);
    let (kinds, pc_reach, reachable_blocks) =
        infer_with_reachability(&ops, &program.constants, &program.struct_types, &program.enum_types, &fn_rt, num_regs as usize, &seeds, ret_of, global_of, closure_ret, ret_layout, fn_return_closure, &param_layouts, &param_closures, linked, &program.functions)?;

    // Result kind: the declared `ret_kind` (a scalar `SlotKind`) when present, EXCEPT a non-scalar
    // (handle) return — a `-> Point` etc. — cannot be a `SlotKind`, so its `ret_kind` defaults to
    // Int; the inferred kind (from the `Return` operands) then wins on a value-type disagreement.
    let inferred = infer_result(&ops, &kinds, &pc_reach, strict)?;
    let result = match (f.ret_kind.map(Kind::from_slot), inferred) {
        (Some(d), Some(i)) if d.wasm_valtype() != i.wasm_valtype() => Some(i),
        (Some(d), _) => Some(d),
        (None, i) => i,
    };
    let structs = kind::struct_layout(&ops, &program.constants, &program.struct_types, &program.enum_types, &fn_rt, ret_layout, fn_return_closure, &param_layouts, &param_closures);
    let cow_inserts = cow_struct_inserts(&ops, num_regs, &program.functions);
    let reg_shape = complete_reg_shape(&structs, &kinds, program);
    let return_closure = infer_return_closure(&ops, &kinds, &structs, &pc_reach);
    Ok(Plan { ops, kinds, num_params, num_regs, result, reachable_blocks, structs, cow_inserts, reg_shape, return_closure, stub: false })
}

/// WHICH closure (body function index) this function returns, if any — the closure-valued analog of
/// [`fn_return_struct_layout`]. Scans REACHABLE closure-typed `Return`s (a nested closure's inline
/// body is unreachable here, so its `Return` is ignored); yields `Some(c)` iff they all agree on the
/// same statically-traced origin `c` (via `closure_of`, which is `Move`-aliased). `None` if it
/// returns no closure or more than one — a caller then cannot resolve a `CallValue` on the result.
fn infer_return_closure(ops: &[Op], kinds: &KindTable, structs: &kind::StructLayout, pc_reach: &[bool]) -> Option<u16> {
    let mut found: Option<u16> = None;
    for (pc, op) in ops.iter().enumerate() {
        if !pc_reach.get(pc).copied().unwrap_or(true) {
            continue;
        }
        if let Op::Return { src } = *op {
            if kinds.get(src as usize) != Some(Kind::Closure) {
                continue;
            }
            match structs.closure_of.get(&src) {
                Some(&c) if found.is_none() => found = Some(c),
                Some(&c) if found == Some(c) => {}
                _ => return None,
            }
        }
    }
    found
}

/// WHICH closure each function PARAMETER is always passed — the param-side, whole-program analog of
/// [`infer_return_closure`]. For every `Call`/`CallValue` in every planned region, attribute each
/// argument register's statically-traced closure origin (the caller's `closure_of`) to the callee's
/// matching parameter index. A parameter is bound to closure `c` iff EVERY observed call passes that
/// same `c` and nothing else; any disagreement (a different closure, or an untraced argument) yields
/// `None`, so a genuinely polymorphic / opaque closure parameter stays soundly rejected. Result
/// indexed `[fi][param_idx]`. Lets `f(args)` resolve its callee when `f` is a closure ARGUMENT.
/// `plans` is every region to scan (Main + the planned functions).
fn compute_param_origins(program: &CompiledProgram, plans: &[&Plan]) -> Vec<Vec<Option<u16>>> {
    use std::collections::{HashMap, HashSet};
    let mut obs: HashMap<(usize, usize), HashSet<Option<u16>>> = HashMap::new();
    for plan in plans {
        for (pc, op) in plan.ops.iter().enumerate() {
            let (func, args_start, arg_count) = match *op {
                Op::Call { func, args_start, arg_count, .. } => (Some(func as usize), args_start, arg_count),
                Op::CallValue { args_start, arg_count, .. } => {
                    (plan.structs.callee_func.get(pc).copied().flatten().map(|f| f as usize), args_start, arg_count)
                }
                _ => continue,
            };
            let Some(func) = func else { continue };
            for i in 0..arg_count {
                let origin = plan.structs.closure_of.get(&(args_start + i)).copied();
                obs.entry((func, i as usize)).or_default().insert(origin);
            }
        }
    }
    (0..program.functions.len())
        .map(|fi| {
            let pc = program.functions[fi].param_count as usize;
            (0..pc)
                .map(|i| match obs.get(&(fi, i)) {
                    Some(set) if set.len() == 1 => set.iter().next().copied().flatten(),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

/// Resolve a composite [`BoundaryType`] to its access-resolution [`ParamShape`] — the shared bridge
/// used to seed a parameter (its declared type), a closure capture (the captured global's type), or
/// any other cross-region composite. `None` for a scalar / self-describing type (no shape needed) or
/// one whose layout/kinds don't resolve (the access then stays soundly rejected).
fn boundary_to_param_shape(bt: &BoundaryType, program: &CompiledProgram) -> Option<ParamShape> {
    match bt {
        BoundaryType::Struct(name) => kind::resolve_named_layout(name, &program.struct_types, &program.constants)
            .map(ParamShape::Struct),
        BoundaryType::Map(_, value) => kind::boundary_to_kind(value).map(ParamShape::Map),
        BoundaryType::Enum(name) => kind::resolve_enum_variants(name, &program.enum_types).map(ParamShape::Enum),
        BoundaryType::Tuple(elems) => {
            elems.iter().map(kind::boundary_to_kind).collect::<Option<Vec<Kind>>>().map(ParamShape::Tuple)
        }
        _ => None,
    }
}

/// Each non-scalar PARAMETER's access-resolution shape (parameter register `i` → [`ParamShape`]),
/// from the bytecode's `param_types`/`struct_types`. Seeded into `struct_layout` so `param's field`
/// (struct) and `item k of m` (map) resolve like a cross-region access. A struct/map whose layout
/// or value kind is unresolvable is simply omitted, leaving the access soundly rejected.
fn param_seeds(f: &CompiledFunction, program: &CompiledProgram) -> Vec<(u16, ParamShape)> {
    let mut out = Vec::new();
    for (i, pt) in f.param_types.iter().enumerate() {
        if let Some(shape) = pt.as_ref().and_then(|bt| boundary_to_param_shape(bt, program)) {
            out.push((i as u16, shape));
        }
    }
    out
}

/// Infer a function's result kind from its `Return` operands (they must agree); `None` (void) if it
/// only `ReturnNothing`s or never returns a value. With `strict == false` (the first planning pass),
/// a `Return` of a not-yet-known kind is DEFERRED (skipped), not an error — it may resolve in the
/// second pass once callee return layouts are threaded (a function returning the result of a
/// struct-returning function). The strict (final) pass errors on a genuinely unknown return.
fn infer_result(ops: &[Op], kinds: &KindTable, pc_reach: &[bool], strict: bool) -> R<Option<Kind>> {
    let mut result = None;
    for (pc, op) in ops.iter().enumerate() {
        // A nested closure's body is emitted INLINE in this region (its parent jumps over it; it is
        // reached only through the closure call into its own separate function). That inline copy is
        // unreachable HERE, so its `Return` must not contribute to — or poison — this region's result.
        if !pc_reach.get(pc).copied().unwrap_or(true) {
            continue;
        }
        if let Op::Return { src } = *op {
            let k = match kinds.get(src as usize) {
                Some(k) => k,
                None if !strict => continue,
                None => return Err(WasmLowerError::Unsupported("return of an unknown-kind value")),
            };
            match result {
                None => result = Some(k),
                Some(prev) if prev == k => {}
                // A `SeqAny` return (an empty `new Seq of T` return path, or a recursive-call result
                // not yet refined during the fixpoint) refines to a concrete sibling sequence — the
                // same rule `unify_strict` applies to register kinds. So a `mergeSort` returning
                // `arr` (SeqInt) on the base case and `result` (SeqAny until the recursion resolves)
                // on the recursive case agrees on SeqInt instead of falsely reading as mixed.
                Some(Kind::SeqAny) if k.is_seq() => result = Some(k),
                Some(prev) if prev.is_seq() && k == Kind::SeqAny => {}
                Some(_) => return Err(WasmLowerError::Unsupported("function returns mixed kinds")),
            }
        }
    }
    Ok(result)
}

/// A function's RETURN struct layout, resolved to `(field name-const-idx, field kind)` — `None`
/// unless the function returns a `Struct` whose every field kind is known. Lets a caller resolve
/// `f(…)'s field` cross-region (the returned struct is built in this — the callee's — region, so
/// its field-defining registers are not visible to the caller; the resolved kinds bridge that).
fn fn_return_struct_layout(plan: &Plan) -> Option<FieldLayout> {
    for op in &plan.ops {
        if let Op::Return { src } = *op {
            if plan.kinds.get(src as usize) == Some(Kind::Struct) {
                let layout = plan.structs.reg_layout.get(&src)?;
                // The field KINDS bridge the region boundary; a struct-typed field is additionally
                // named from the callee's `struct_name_of` (the field value's `NewStruct` type), so a
                // caller re-seeds it and resolves `f(…)'s inner's v` — symmetric with the param path.
                // A map/enum field of a RETURNED struct stays `FieldNested::None` (its value kind /
                // variant layout isn't recoverable from the plan's nameless `Kind`) — a narrower gap.
                let resolved: FieldLayout = layout
                    .iter()
                    .filter_map(|&(f, vr)| {
                        plan.kinds.get(vr as usize).map(|k| {
                            let nested = match (k, plan.structs.struct_name_of.get(&vr)) {
                                (Kind::Struct, Some(name)) => FieldNested::Struct(name.clone()),
                                _ => FieldNested::None,
                            };
                            (f, k, nested)
                        })
                    })
                    .collect();
                return (resolved.len() == layout.len()).then_some(resolved);
            }
        }
    }
    None
}

/// The wasm result-type vector for a function/result kind (`[]` for void, else `[valtype]`).
fn result_valtypes(result: Option<Kind>) -> Vec<u8> {
    match result {
        Some(k) => vec![k.wasm_valtype()],
        None => vec![],
    }
}

/// Per-function emission context: the constant pool, how to resolve a host sink's import
/// index, and the wasm index of `program.functions[0]` (so `Op::Call { func }` →
/// `call (fn_base + func)`).
struct Ctx<'a> {
    constants: &'a [Constant],
    host_index: &'a dyn Fn(HostFn) -> Option<u32>,
    fn_base: u32,
    /// The wasm global index of the bump-allocator pointer `__heap_ptr` (the heap value model's
    /// linear-memory cursor); it follows the user globals.
    heap_global: u32,
    /// The wasm global index of the iterator-stack pointer `__iter_sp` (it follows `__heap_ptr`),
    /// for `Repeat` iteration. Grows *down* from the top of memory toward the up-growing heap;
    /// each live `Repeat` owns one 12-byte frame `[snapshot_ptr:i32][cursor:i32][len:i32]`.
    iter_global: u32,
    /// The wasm type index of each `program.functions[i]` (by function index), so a `CallValue`'s
    /// `call_indirect` can name the callee closure body's signature (= that function's own type).
    fn_type: &'a [u32],
    /// Each `program.functions[i]`'s ACTUAL emitted parameter value types (from its plan), so a direct
    /// `Call` matches the real signature — including a `Closure` parameter that lowers to i32, which the
    /// declared-type `function_param_seeds` cannot see (there is no `BoundaryType::Closure`).
    fn_param_valtypes: &'a [Vec<u8>],
    /// Each `program.functions[i]`'s result kind (`None` = void) — tells a `CallValue` whether the
    /// `call_indirect` leaves a value to bind into its destination.
    fn_results: &'a [Option<Kind>],
    /// The program's functions — `MakeClosure`/`CallValue` read the callee's `captures` (count and
    /// each capture's local-register-vs-global source) to fill / pass the closure object.
    functions: &'a [crate::vm::instruction::CompiledFunction],
    /// Each function's capture VALUE kinds (by function index, then capture index) — a captured
    /// global's kind, else `None` (a local capture, stored/loaded as Int). `MakeClosure` stores and
    /// `CallValue` loads each capture slot at this kind's width, matching the body's seeded signature.
    capture_kinds: &'a [Vec<Option<Kind>>],
    /// The program's enum type definitions (variant names + payload field types) — a whole-enum
    /// `Show` reads the variant set to emit its tag→name dispatch (`lower_show_enum`).
    enum_types: &'a [EnumTypeDef],
    /// The program's struct type definitions (field names in declaration order) — a `CheckPolicy`
    /// resolves a policy condition's `field` to its slot index via the subject's declared fields.
    struct_types: &'a [StructTypeDef],
    /// The `## Policy` registry (predicate + capability conditions) — `CheckPolicy` compiles the
    /// resolved condition inline (field access + `text_eq` + and/or), trapping when it is false.
    policies: &'a PolicyRegistry,
    /// The interner, to resolve a policy condition's `Symbol` field/value/predicate names to the
    /// strings the struct-field lookup + `text_eq` literal need.
    interner: &'a Interner,
    /// Linker mode: an integer `Op::Pow` lowers to the `logos_rt_bigint_*` runtime (a `Text` handle)
    /// rather than the self-contained i64 exponentiation-by-squaring. Set only by
    /// [`assemble_program_linked`]; the standalone emitter leaves it `false`.
    linked: bool,
    /// Linker mode + emitter heap: the import index of `logos_rt_alloc`. When `Some`, [`emit_alloc`] draws
    /// each block straight from the runtime allocator (`dlmalloc`, growing linear memory on demand) rather
    /// than a fixed bump slab — so the emitter heap is UNBOUNDED. `None` = the standalone bump path.
    rt_alloc: Option<u32>,
}

/// Whether an op terminated its basic block (a branch / return), so the block emitter stops.
#[derive(PartialEq, Eq)]
enum Flow {
    Straight,
    Terminated,
}

/// Emit one function's complete Code-section entry (its locals declaration followed by the
/// dispatch-loop body), from a [`Plan`].
fn emit_body(plan: &Plan, _ctx: &Ctx) -> R<Vec<u8>> {
    // A dropped (unreachable-from-Main) function: no locals, a lone `unreachable` trap. It is never
    // called (that's why it was dropped), so its `() -> ()` type is inert — this just lets the module
    // link when an imported stdlib carries functions the AOT can't lower but the program never uses.
    if plan.stub {
        return Ok(vec![0x00 /* 0 local groups */, 0x00 /* unreachable */, 0x0B /* end */]);
    }
    let ctx = _ctx;
    let blocks = Blocks::new(&plan.ops).ok_or(WasmLowerError::Unsupported("jump target escapes the function"))?;
    let pc_local = plan.num_regs;

    let mut blocks_code: Vec<Vec<u8>> = Vec::with_capacity(blocks.num_blocks());
    for k in 0..blocks.num_blocks() {
        // A statically-dead block (e.g. the monomorphized-out branch of an `and`/`or` runtime
        // type-dispatch) is never branched to — emit a lone `unreachable` rather than lower its
        // ops, which may reference registers whose kinds were (correctly) never inferred.
        if !plan.reachable_blocks.get(k).copied().unwrap_or(true) {
            blocks_code.push(vec![0x00]); // unreachable
            continue;
        }
        let mut code = Vec::new();
        let mut terminated = false;
        for pc in blocks.start(k)..blocks.end(k) {
            if lower_op(pc, plan, ctx, &blocks, k, &mut code)? == Flow::Terminated {
                terminated = true;
                break;
            }
        }
        if !terminated {
            let end = blocks.end(k);
            if end >= plan.ops.len() {
                // Fell off the end of the function: void returns, a typed function cannot.
                match plan.result {
                    None => code.push(0x0F),   // return
                    Some(_) => code.push(0x00), // unreachable
                }
            } else {
                let next = blocks.block_of(end);
                code.push(0x41); // i32.const next-block
                leb_u32(&mut code, next as u32);
                local_set(&mut code, pc_local);
                code.push(0x0C); // br $loop
                leb_u32(&mut code, blocks.br_loop(k));
            }
        }
        blocks_code.push(code);
    }

    let mut body = assemble_dispatch_loop(pc_local, &blocks_code);
    if plan.result.is_some() {
        body.push(0x00); // unreachable: a typed function always returns inside a block
    }
    body.push(0x0B); // end function

    let mut entry = encode_locals(plan);
    entry.extend(body);
    Ok(entry)
}

/// Lower one op into the current block's code, returning whether it terminated the block.
fn lower_op(pc: usize, plan: &Plan, ctx: &Ctx, blocks: &Blocks, k: usize, code: &mut Vec<u8>) -> R<Flow> {
    let kinds = &plan.kinds;
    match plan.ops[pc] {
        Op::LoadConst { dst, idx } => {
            // A register the inference promoted to Float (an Int-initialized accumulator, `Let sum
            // be 0`) holds an `f64` local, so an Int/Bool literal flowing into it materializes as
            // `f64.const` — the exact promotion the VM performs at the first float op.
            let dst_float = kinds.get(dst as usize) == Some(Kind::Float);
            match ctx.constants.get(idx as usize).ok_or(WasmLowerError::Unsupported("constant index out of range"))? {
                Constant::Int(v) if dst_float => {
                    code.push(0x44); // f64.const
                    code.extend_from_slice(&(*v as f64).to_le_bytes());
                }
                Constant::Int(v) => {
                    code.push(0x42); // i64.const
                    leb_i64(code, *v);
                }
                Constant::Bool(b) if dst_float => {
                    code.push(0x44); // f64.const
                    code.extend_from_slice(&(if *b { 1.0f64 } else { 0.0f64 }).to_le_bytes());
                }
                Constant::Bool(b) => {
                    code.push(0x42); // i64.const (truthy-Int boolean: 0/1)
                    leb_i64(code, i64::from(*b));
                }
                Constant::Char(c) => {
                    code.push(0x42); // i64.const — the Unicode code point (`char as u32`)
                    leb_i64(code, i64::from(*c as u32));
                }
                Constant::Nothing => {
                    // Reads as the Int `0` (an i64 local): the read-as-zero CRDT-counter/dead-`Zone`-name
                    // default, and the `nothing` literal of `x is equal to nothing` (the compare special-
                    // cases the `Optional` side against this `0`). A genuine `Optional` comes from
                    // `ChanTryRecv`, not this const.
                    code.push(0x42); // i64.const 0
                    leb_i64(code, 0);
                }
                Constant::Float(f) => {
                    code.push(0x44); // f64.const
                    code.extend_from_slice(&f.to_le_bytes());
                }
                // Temporal scalars ride a single i64 (tick/day count). A `select`'s `After N seconds`
                // ticks register is the only one the corpus loads — it is dead in the deterministic
                // model (`SelectArmTimeout` is a no-op; the timeout fires whenever no recv arm is ready)
                // but still materializes so its local declares.
                Constant::Duration(v) | Constant::Moment(v) | Constant::Time(v) => {
                    code.push(0x42); // i64.const
                    leb_i64(code, *v);
                }
                Constant::Date(v) => {
                    // A `Date` is days-since-epoch in an `i32` local (`Kind::Date` → i32, matching
                    // `LoadToday`'s i32 host result), so the literal materializes as `i32.const` — an
                    // `i64.const` would mismatch the declared local valtype. `i32_const` signs the LEB,
                    // so pre-1970 dates (negative days) encode correctly.
                    i32_const(code, *v);
                }
                // A `Span` packs its two i32 fields into one i64 local: `months` in the high word, `days`
                // in the low word (the calendar-arith lowering unpacks them for the runtime call).
                Constant::Span { months, days } => {
                    let packed = ((*months as i64) << 32) | ((*days as u32) as i64);
                    code.push(0x42); // i64.const
                    leb_i64(code, packed);
                }
                Constant::Text(s) => {
                    // Build a fresh Text object in linear memory; it leaves the handle on the
                    // stack for the `local_set` below.
                    lower_text_literal(code, ctx, plan.num_regs, s.as_bytes());
                }
                _ => return Err(WasmLowerError::Unsupported("non-scalar constant")),
            }
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // unobservable (its source is dead). The RETAIN below is a harmless over-retain
        // (COW resolves it) — correctness identical to `Move`.
        // `EnsureOwned` is the interpreter's call-site COW barrier; the WASM AOT
        // enforces value semantics by copy-on-write at each element write, so the
        // barrier is a no-op here (correctness is preserved without it).
        Op::EnsureOwned { .. } => Ok(Flow::Straight),
        Op::Move { dst, src } => {
            // Moving an `i64` source into a Float-promoted destination converts (the VM's Int→Float
            // promotion); same-kind moves are a plain copy.
            if kinds.get(dst as usize) == Some(Kind::Float) && kinds.get(src as usize) != Some(Kind::Float) {
                push_as_f64(code, src, kinds.get(src as usize))?;
            } else {
                local_get(code, src as u32);
            }
            local_set(code, dst as u32);
            // Aliasing a mutable heap object (`Let cs be c's items`) gains a second holder — RETAIN so
            // a later mutation of either copy-on-writes instead of clobbering the other.
            if cow_clonable(kinds.get(dst as usize)) {
                emit_retain(code, dst);
            }
            Ok(Flow::Straight)
        }
        // A Text-typed `+` (`add_join` resolved the result to Text) is string concatenation — route to
        // `lower_concat`, which stringifies both operands and joins them. Everything else is numeric.
        // `+ - *` on a BigInt result (linker mode) are exact big-integer arithmetic via the runtime; an
        // `Int` operand promotes to a BigInt. Checked FIRST so a `BigInt` `+` never falls to the numeric
        // (or string-concat) path. The result kind is `BigInt` iff `op_kind_effect` saw a BigInt operand.
        Op::Add { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::BigInt) => {
            lower_bigint_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::BigintAdd)
        }
        Op::Sub { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::BigInt) => {
            lower_bigint_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::BigintSub)
        }
        Op::Mul { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::BigInt) => {
            lower_bigint_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::BigintMul)
        }
        Op::Div { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::BigInt) => {
            lower_bigint_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::BigintDiv)
        }
        Op::Mod { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::BigInt) => {
            lower_bigint_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::BigintMod)
        }
        Op::Add { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Complex) => {
            lower_complex_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::ComplexAdd)
        }
        Op::Sub { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Complex) => {
            lower_complex_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::ComplexSub)
        }
        Op::Mul { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Complex) => {
            lower_complex_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::ComplexMul)
        }
        Op::Add { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Modular) => {
            lower_modular_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::ModularAdd)
        }
        Op::Sub { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Modular) => {
            lower_modular_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::ModularSub)
        }
        Op::Mul { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Modular) => {
            lower_modular_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::ModularMul)
        }
        Op::Add { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Decimal) => {
            lower_decimal_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::DecimalAdd)
        }
        Op::Sub { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Decimal) => {
            lower_decimal_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::DecimalSub)
        }
        Op::Mul { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Decimal) => {
            lower_decimal_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::DecimalMul)
        }
        Op::Add { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Money) => {
            lower_money_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::MoneyAdd)
        }
        Op::Sub { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Money) => {
            lower_money_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::MoneySub)
        }
        Op::Add { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Quantity) => {
            lower_quantity_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::QuantityAdd)
        }
        Op::Sub { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Quantity) => {
            lower_quantity_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::QuantitySub)
        }
        Op::Mul { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Quantity) => {
            lower_quantity_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::QuantityMul)
        }
        Op::Div { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Quantity) => {
            lower_quantity_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::QuantityDiv)
        }
        // `Moment/Date + Span` (calendar arithmetic, commutes): the Span operand carries the months/days,
        // the other is the base. Guarded on the SPAN operand (not dst) so it beats the `Moment + Duration`
        // i64-add arm above (whose result is also Moment).
        Op::Add { dst, lhs, rhs }
            if ctx.linked && (kinds.get(lhs as usize) == Some(Kind::Span) || kinds.get(rhs as usize) == Some(Kind::Span)) =>
        {
            let (base, span) = if kinds.get(lhs as usize) == Some(Kind::Span) { (rhs, lhs) } else { (lhs, rhs) };
            let is_date = kinds.get(base as usize) == Some(Kind::Date);
            return lower_span_add(code, ctx, plan.num_regs, dst, base, span, is_date, false);
        }
        // `Moment/Date - Span` steps the calendar backward — the span (rhs) is negated.
        Op::Sub { dst, lhs, rhs } if ctx.linked && kinds.get(rhs as usize) == Some(Kind::Span) => {
            let is_date = kinds.get(lhs as usize) == Some(Kind::Date);
            return lower_span_add(code, ctx, plan.num_regs, dst, lhs, rhs, is_date, true);
        }
        // Lane-wise `Lanes + Lanes` (the SHA-1 block fold `st + abcdSave`) — a `logos_rt_lanes4_add` call.
        Op::Add { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Lanes) => {
            let idx = (ctx.host_index)(HostFn::Lanes4Add).ok_or(WasmLowerError::Unsupported("lanes4_add not imported"))?;
            local_get(code, lhs as u32);
            local_get(code, rhs as u32);
            code.push(0x10);
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        Op::Add { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Rational) => {
            lower_rational_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::RationalAdd)
        }
        Op::Sub { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Rational) => {
            lower_rational_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::RationalSub)
        }
        Op::Mul { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Rational) => {
            lower_rational_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::RationalMul)
        }
        Op::Div { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Rational) => {
            lower_rational_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::RationalDiv)
        }
        Op::Add { dst, lhs, rhs } => {
            if kinds.get(dst as usize) == Some(Kind::Text) {
                lower_concat(code, kinds, ctx, plan.num_regs, dst, lhs, rhs)?;
                Ok(Flow::Straight)
            } else {
                lower_arith(code, kinds, dst, lhs, rhs, ArithOp::Add)
            }
        }
        Op::Sub { dst, lhs, rhs } => lower_arith(code, kinds, dst, lhs, rhs, ArithOp::Sub),
        Op::Mul { dst, lhs, rhs } => lower_arith(code, kinds, dst, lhs, rhs, ArithOp::Mul),
        Op::Div { dst, lhs, rhs } => lower_arith(code, kinds, dst, lhs, rhs, ArithOp::Div),
        Op::FloorDiv { dst, lhs, rhs } => lower_floordiv_regs(code, kinds, plan.num_regs, dst, lhs, rhs),
        Op::Mod { dst, lhs, rhs } => lower_arith(code, kinds, dst, lhs, rhs, ArithOp::Mod),
        // `a ** b` — exponentiation. Float-result cases use the host `pow_ff`/`pow_fi`; `Int^Int`
        // is the in-module overflow-trapping squaring loop (a `2**100`-style overflow traps, the
        // BigInt-promoting frontier, matching `Mul`). A negative Int exponent traps (VM → Float).
        Op::Pow { dst, lhs, rhs } => lower_pow_regs(code, kinds, ctx, plan.num_regs, dst, lhs, rhs),
        // `lhs / 2^k` (the Oracle's power-of-two division form) = a plain signed division by the
        // constant `1<<k` — `i64.div_s` matches the VM's `lhs.div(1<<k)` (truncate toward zero, and
        // `2^k > 0` so no INT_MIN/-1 trap). Emitted only for an Oracle-proven `Int` lhs (i64).
        Op::DivPow2 { dst, lhs, k } => {
            local_get(code, lhs as u32);
            code.push(0x42); // i64.const 2^k
            leb_i64(code, 1i64 << k);
            code.push(0x7F); // i64.div_s
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `lhs / c` or `lhs % c` by the precomputed magic reciprocal — mirror the VM's `magic_eval`
        // (Granlund–Montgomery mul-high + shift) bit-for-bit.
        Op::MagicDivU { dst, lhs, magic, more, mul_back } => {
            lower_magic_div(code, plan.num_regs, dst, lhs, magic, more, mul_back);
            Ok(Flow::Straight)
        }
        // `a / b` in a `Rational` context → an exact reduced `Rational`. LINKER mode uses the
        // BigInt-backed runtime (`logos_rt_rational_div`, arbitrary precision, promoting Int/BigInt
        // operands to Rational first); self-contained mode uses the inline i64/i64 reduce.
        Op::ExactDiv { dst, lhs, rhs } if ctx.linked => {
            lower_rational_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::RationalDiv)
        }
        Op::ExactDiv { dst, lhs, rhs } => {
            lower_exact_div(code, ctx, plan.num_regs, dst, lhs, rhs);
            Ok(Flow::Straight)
        }
        // Pure native-region metadata — the AOT's accesses are checked, so the bound guard is a no-op
        // (exactly as the VM, where `RegionBoundsGuard` is a `pc += 1`).
        Op::RegionBoundsGuard { .. } => Ok(Flow::Straight),
        Op::AddAssign { dst, src } => {
            if kinds.get(dst as usize) == Some(Kind::Text) {
                lower_concat(code, kinds, ctx, plan.num_regs, dst, dst, src)?;
                Ok(Flow::Straight)
            } else if ctx.linked && kinds.get(dst as usize) == Some(Kind::Lanes) {
                // `lanes += lanes` (the SHA-1 `Set e0 to e0 + m0`) — lane-wise add into `dst`.
                let idx = (ctx.host_index)(HostFn::Lanes4Add).ok_or(WasmLowerError::Unsupported("lanes4_add not imported"))?;
                local_get(code, dst as u32);
                local_get(code, src as u32);
                code.push(0x10);
                leb_u32(code, idx);
                local_set(code, dst as u32);
                Ok(Flow::Straight)
            } else {
                lower_arith(code, kinds, dst, dst, src, ArithOp::Add)
            }
        }
        // `^ & |` scalar lowering only — a Set operand (set algebra) has no register-scalar
        // form here yet, so it fails loud rather than i64-punning a handle.
        // Bitwise `xor`/`&`/`|` also close over the Word ring (`Word32` → `i32.*`, `Word64` → `i64.*`),
        // so crypto written with the `xor` keyword compiles alongside the `word_and`/`word_or` builtins.
        Op::BitXor { dst, lhs, rhs } => match kinds.get(lhs as usize) {
            Some(Kind::Int) | Some(Kind::Word64) => {
                arith(code, 0x85, dst, lhs, rhs); // i64.xor
                Ok(Flow::Straight)
            }
            Some(Kind::Word32) => {
                arith(code, 0x73, dst, lhs, rhs); // i32.xor
                Ok(Flow::Straight)
            }
            // Lane-wise `Lanes xor Lanes` (the SHA-1 message-schedule fold) via the runtime.
            Some(Kind::Lanes) if ctx.linked => {
                let idx = (ctx.host_index)(HostFn::Lanes4Xor).ok_or(WasmLowerError::Unsupported("lanes4_xor not imported"))?;
                local_get(code, lhs as u32);
                local_get(code, rhs as u32);
                code.push(0x10);
                leb_u32(code, idx);
                local_set(code, dst as u32);
                Ok(Flow::Straight)
            }
            _ => Err(WasmLowerError::Unsupported("`^` of a non-Int/Word value")),
        },
        Op::BitAnd { dst, lhs, rhs } => match kinds.get(lhs as usize) {
            Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Word64) => {
                arith(code, 0x83, dst, lhs, rhs); // i64.and
                Ok(Flow::Straight)
            }
            Some(Kind::Word32) => {
                arith(code, 0x71, dst, lhs, rhs); // i32.and
                Ok(Flow::Straight)
            }
            _ => Err(WasmLowerError::Unsupported("`&` of a non-Int/Bool/Word value")),
        },
        Op::BitOr { dst, lhs, rhs } => match kinds.get(lhs as usize) {
            Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Word64) => {
                arith(code, 0x84, dst, lhs, rhs); // i64.or
                Ok(Flow::Straight)
            }
            Some(Kind::Word32) => {
                arith(code, 0x72, dst, lhs, rhs); // i32.or
                Ok(Flow::Straight)
            }
            _ => Err(WasmLowerError::Unsupported("`|` of a non-Int/Bool/Word value")),
        },
        Op::Shl { dst, lhs, rhs } => {
            arith(code, 0x86, dst, lhs, rhs); // i64.shl
            Ok(Flow::Straight)
        }
        Op::Shr { dst, lhs, rhs } => {
            arith(code, 0x87, dst, lhs, rhs); // i64.shr_s
            Ok(Flow::Straight)
        }
        Op::Not { dst, src } => {
            // `not` is LOGICAL — truthiness in, Bool out (`~` lowers to `x ^ -1`
            // in the parser, never through here). Bool and Int share the zero
            // test: `x == 0` (i64.eqz → i32) widened back to i64 0/1.
            match kinds.get(src as usize) {
                Some(Kind::Bool) | Some(Kind::Int) => {
                    local_get(code, src as u32);
                    code.push(0x50); // i64.eqz
                    code.push(0xAD); // i64.extend_i32_u
                    local_set(code, dst as u32);
                }
                _ => return Err(WasmLowerError::Unsupported("Not of a non-Int/Bool value")),
            }
            Ok(Flow::Straight)
        }
        Op::Lt { dst, lhs, rhs } => lower_compare(code, kinds, dst, lhs, rhs, Cmp::Lt),
        Op::Gt { dst, lhs, rhs } => lower_compare(code, kinds, dst, lhs, rhs, Cmp::Gt),
        Op::LtEq { dst, lhs, rhs } => lower_compare(code, kinds, dst, lhs, rhs, Cmp::Le),
        Op::GtEq { dst, lhs, rhs } => lower_compare(code, kinds, dst, lhs, rhs, Cmp::Ge),
        Op::ApproxEq { dst, lhs, rhs } => lower_approx_eq(code, kinds, dst, lhs, rhs),
        Op::Eq { dst, lhs, rhs } => {
            if kinds.get(lhs as usize) == Some(Kind::Text) && kinds.get(rhs as usize) == Some(Kind::Text) {
                lower_text_eq(code, plan.num_regs, dst, lhs, rhs, false);
                Ok(Flow::Straight)
            } else if ctx.linked && kinds.get(lhs as usize) == Some(Kind::Uuid) && kinds.get(rhs as usize) == Some(Kind::Uuid) {
                lower_uuid_eq(code, ctx, dst, lhs, rhs, false)
            } else {
                lower_compare(code, kinds, dst, lhs, rhs, Cmp::Eq)
            }
        }
        Op::NotEq { dst, lhs, rhs } => {
            if kinds.get(lhs as usize) == Some(Kind::Text) && kinds.get(rhs as usize) == Some(Kind::Text) {
                lower_text_eq(code, plan.num_regs, dst, lhs, rhs, true);
                Ok(Flow::Straight)
            } else if ctx.linked && kinds.get(lhs as usize) == Some(Kind::Uuid) && kinds.get(rhs as usize) == Some(Kind::Uuid) {
                lower_uuid_eq(code, ctx, dst, lhs, rhs, true)
            } else {
                lower_compare(code, kinds, dst, lhs, rhs, Cmp::Ne)
            }
        }
        Op::Jump { target } => {
            code.push(0x41);
            leb_u32(code, blocks.block_of(target) as u32);
            local_set(code, plan.num_regs);
            code.push(0x0C);
            leb_u32(code, blocks.br_loop(k));
            Ok(Flow::Terminated)
        }
        Op::JumpIfFalse { cond, target } => {
            emit_cond_jump(code, cond, true, blocks.block_of(target), blocks.block_of(pc + 1), plan.num_regs, blocks.br_loop(k));
            Ok(Flow::Terminated)
        }
        Op::JumpIfTrue { cond, target } => {
            emit_cond_jump(code, cond, false, blocks.block_of(target), blocks.block_of(pc + 1), plan.num_regs, blocks.br_loop(k));
            Ok(Flow::Terminated)
        }
        Op::GlobalGet { dst, idx } => {
            code.push(0x23); // global.get
            leb_u32(code, idx as u32);
            local_set(code, dst as u32);
            // Reading a global heap handle aliases the global's object — RETAIN so a mutation of the
            // read copy copy-on-writes instead of clobbering the global.
            if cow_clonable(kinds.get(dst as usize)) {
                emit_retain(code, dst);
            }
            Ok(Flow::Straight)
        }
        Op::LoadToday { dst } => {
            let idx = (ctx.host_index)(HostFn::Today).ok_or(WasmLowerError::Unsupported("today not imported"))?;
            code.push(0x10); // call today -> i32 (Date)
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // A new empty sequence: bump-allocate a 16-byte header `[len=0][cap=0][data_ptr=0]` and
        // hand back its (stable) pointer. Growth on push reallocs the data buffer, not the header,
        // so the register holding this handle never needs updating.
        // A new empty sequence or map — both are a zeroed 16-byte header `[len/num=0][cap=0]
        // [data_ptr=0]`; the element/entry shape differs only at use.
        Op::NewEmptyList { dst } | Op::NewEmptyListI32 { dst } | Op::NewEmptyMap { dst } | Op::NewEmptySet { dst } => {
            emit_empty_header(code, ctx, plan.num_regs, dst as u32);
            Ok(Flow::Straight)
        }
        // `length of seq` = the header's `len` field (an Int).
        Op::Length { dst, collection } => {
            local_get(code, collection as u32);
            i32_load(code, 0); // header len (i32)
            code.push(0xAD); // i64.extend_i32_u → Int
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        Op::ListPush { list, value } => {
            emit_cow(code, kinds, &plan.structs, ctx, plan.num_regs, list)?;
            lower_list_push(code, kinds, ctx, plan.num_regs, list, value)?;
            // Pushing a mutable heap VALUE stores it as an element (a second holder) — RETAIN so a
            // later mutation of the original copy-on-writes instead of mutating the stored element.
            if cow_clonable(kinds.get(value as usize)) {
                emit_retain(code, value);
            }
            Ok(Flow::Straight)
        }
        Op::ListPushField { obj, field, src } => {
            lower_list_push_field(code, plan, kinds, ctx, obj, field, src)?;
            if cow_clonable(kinds.get(src as usize)) {
                emit_retain(code, src);
            }
            Ok(Flow::Straight)
        }
        Op::ListPop { list, dst } => {
            emit_cow(code, kinds, &plan.structs, ctx, plan.num_regs, list)?;
            lower_list_pop(code, kinds, dst, list)?;
            // The popped element's slot stays physically in the (now-shorter) buffer, so `dst`
            // aliases it until a later push overwrites it — RETAIN a clonable handle so mutating
            // `dst` copy-on-writes (same discipline as an `Index` extract).
            if cow_clonable(kinds.get(dst as usize)) {
                emit_retain(code, dst);
            }
            Ok(Flow::Straight)
        }
        // `IndexUnchecked` (the Oracle-proven-in-bounds optimizer form) executes IDENTICALLY to `Index`
        // in the VM — the "unchecked" only drops the bounds branch inside a native region. The AOT keeps
        // its bounds check (a safe superset), so the two lower to the same code.
        Op::Index { dst, collection, index } | Op::IndexUnchecked { dst, collection, index } => {
            match kinds.get(collection as usize) {
                Some(Kind::Map) => lower_map_get(code, kinds, plan.num_regs, dst, collection, index)?,
                Some(Kind::Text) => lower_text_index(code, ctx, plan.num_regs, dst, collection, index)?,
                _ => lower_index(code, kinds, dst, collection, index)?,
            }
            // `Let row be item k of matrix` extracts a handle that aliases the element in the
            // container — RETAIN so mutating the extracted row copy-on-writes.
            if cow_clonable(kinds.get(dst as usize)) {
                emit_retain(code, dst);
            }
            Ok(Flow::Straight)
        }
        // `SetIndexUnchecked` is the Oracle-proven-in-bounds form of `SetIndex` (identical in the VM);
        // lowered the same (the AOT keeps its bounds check).
        Op::SetIndex { collection, index, value } | Op::SetIndexUnchecked { collection, index, value } => {
            emit_cow(code, kinds, &plan.structs, ctx, plan.num_regs, collection)?;
            if kinds.get(collection as usize) == Some(Kind::Map) {
                lower_map_insert(code, kinds, ctx, plan.num_regs, collection, index, value)?;
            } else {
                lower_set_index(code, kinds, collection, index, value)?;
            }
            // Storing a mutable heap VALUE as an element/map-value (a second holder) — RETAIN so a
            // later mutation of the original copy-on-writes instead of mutating the stored one.
            if cow_clonable(kinds.get(value as usize)) {
                emit_retain(code, value);
            }
            Ok(Flow::Straight)
        }
        Op::NewRange { dst, start, end } => {
            lower_new_range(code, ctx, plan.num_regs, dst, start, end);
            Ok(Flow::Straight)
        }
        // A list literal `[…]` and a homogeneous tuple `(…)` allocate the same buffer via
        // `lower_new_list`; a heterogeneous tuple (inferred `Kind::Tuple`) stores each element at
        // its own width via `lower_new_tuple_het`.
        Op::NewList { dst, start, count } => {
            lower_new_list(code, kinds, ctx, plan.num_regs, dst, start, count)?;
            Ok(Flow::Straight)
        }
        Op::NewTuple { dst, start, count } => {
            if kinds.get(dst as usize) == Some(Kind::Tuple) {
                lower_new_tuple_het(code, kinds, ctx, plan.num_regs, dst, start, count)?;
            } else {
                lower_new_list(code, kinds, ctx, plan.num_regs, dst, start, count)?;
            }
            Ok(Flow::Straight)
        }
        Op::DestructureTuple { src, start, count } => {
            lower_destructure_tuple(code, kinds, src, start, count)?;
            Ok(Flow::Straight)
        }
        Op::IterPrepare { iterable } => {
            lower_iter_prepare(code, kinds, ctx, plan.num_regs, iterable)?;
            Ok(Flow::Straight)
        }
        Op::IterNext { dst, exit } => {
            lower_iter_next(code, kinds, ctx, blocks, k, plan.num_regs, dst, exit, pc);
            Ok(Flow::Terminated)
        }
        Op::SetAdd { set, value } => {
            emit_cow(code, kinds, &plan.structs, ctx, plan.num_regs, set)?;
            lower_set_add(code, kinds, ctx, plan.num_regs, set, value)?;
            Ok(Flow::Straight)
        }
        Op::RemoveFrom { collection, value } => {
            emit_cow(code, kinds, &plan.structs, ctx, plan.num_regs, collection)?;
            lower_remove_from(code, kinds, plan.num_regs, collection, value)?;
            Ok(Flow::Straight)
        }
        Op::UnionOp { dst, lhs, rhs } => {
            lower_union(code, kinds, ctx, plan.num_regs, dst, lhs, rhs)?;
            Ok(Flow::Straight)
        }
        Op::IntersectOp { dst, lhs, rhs } => {
            lower_intersect(code, kinds, ctx, plan.num_regs, dst, lhs, rhs)?;
            Ok(Flow::Straight)
        }
        Op::Contains { dst, collection, value } => {
            if kinds.get(collection as usize) == Some(Kind::Map) {
                lower_map_contains(code, kinds, plan.num_regs, dst, collection, value)?;
            } else {
                lower_contains(code, kinds, plan.num_regs, dst, collection, value)?;
            }
            Ok(Flow::Straight)
        }
        Op::SliceOp { dst, collection, start, end } => {
            lower_slice(code, kinds, ctx, plan.num_regs, dst, collection, start, end)?;
            Ok(Flow::Straight)
        }
        Op::SeqConcat { dst, lhs, rhs } => {
            lower_seq_concat(code, kinds, ctx, plan.num_regs, dst, lhs, rhs)?;
            Ok(Flow::Straight)
        }
        Op::Concat { dst, lhs, rhs } => {
            lower_concat(code, kinds, ctx, plan.num_regs, dst, lhs, rhs)?;
            Ok(Flow::Straight)
        }
        Op::FormatValue { dst, src, spec, debug_prefix } => {
            lower_format_value(code, kinds, ctx, plan.num_regs, dst, src, spec, debug_prefix)?;
            Ok(Flow::Straight)
        }
        Op::DeepClone { dst, src } => {
            lower_deep_clone(code, kinds, &plan.structs, ctx, plan.num_regs, dst, src)?;
            Ok(Flow::Straight)
        }
        // `copy(x)` — the builtin form of a deep clone (an independent copy of a heap value; the value
        // itself for a scalar), the same lowering as `Op::DeepClone`.
        Op::CallBuiltin { dst, builtin: BuiltinId::Copy, args_start, .. } => {
            lower_deep_clone(code, kinds, &plan.structs, ctx, plan.num_regs, dst, args_start)?;
            Ok(Flow::Straight)
        }
        Op::NewStruct { dst, .. } => {
            let count = plan.structs.count.get(pc).copied().flatten().unwrap_or(0);
            lower_new_struct(code, ctx, plan.num_regs, count, dst);
            Ok(Flow::Straight)
        }
        Op::StructInsert { obj, value, .. } => {
            let slot = plan.structs.slot.get(pc).copied().flatten().ok_or(WasmLowerError::Unsupported("struct insert with no static slot"))?;
            let cow = plan.cow_inserts.get(pc).copied().unwrap_or(true);
            lower_struct_insert(code, kinds, ctx, plan.num_regs, slot, obj, value, cow)?;
            Ok(Flow::Straight)
        }
        Op::GetField { dst, obj, .. } => {
            let slot = plan.structs.slot.get(pc).copied().flatten().ok_or(WasmLowerError::Unsupported("field access with no static slot"))?;
            lower_get_field(code, kinds, slot, dst, obj)?;
            // `Let cs be c's items` aliases the field's mutable heap object — RETAIN so a mutation of
            // the extracted handle copy-on-writes rather than mutating the field in place.
            if cow_clonable(kinds.get(dst as usize)) {
                emit_retain(code, dst);
            }
            Ok(Flow::Straight)
        }
        Op::CheckPolicy { subject, predicate, is_capability, object, .. } => {
            lower_check_policy(code, plan, ctx, subject, predicate, is_capability, object)?;
            Ok(Flow::Straight)
        }
        Op::CrdtBump { obj, field, amount, negate } => {
            lower_crdt_bump(code, plan, ctx, obj, field, amount, negate)?;
            Ok(Flow::Straight)
        }
        Op::CrdtMerge { target, source } => {
            lower_crdt_merge(code, plan, target, source)?;
            Ok(Flow::Straight)
        }
        Op::NewCrdt { dst, kind } => {
            // A fresh CRDT collection is empty: an RGA/sequence (kind 1) gets a `[0][0][0]` header, a
            // divergent register (else) an empty `Text`. Single-replica, these ARE the underlying
            // collection (mutated via the in-place `CrdtAppend`/`CrdtResolve` ops). An OR-Set (0/3) is
            // REFUSED for now: `Add X to <obj>'s <set-field>` compiles to `SetAdd` on a `GetField`
            // result, which the value-semantics COW clones — so the add would not reach the shared
            // field. That needs field-collection mutation to bypass COW (the retain/release-placement
            // soundness obligation), so an OR-Set field `Shared` struct stays deferred, not miscompiled.
            match kind {
                0 | 1 | 3 => emit_empty_header(code, ctx, plan.num_regs, dst as u32),
                _ => {
                    lower_text_literal(code, ctx, plan.num_regs, b"");
                    local_set(code, dst as u32);
                }
            }
            Ok(Flow::Straight)
        }
        Op::CrdtResolve { obj, field, value } => {
            lower_crdt_resolve(code, plan, kinds, obj, field, value)?;
            Ok(Flow::Straight)
        }
        Op::CrdtAppend { seq, value } => {
            lower_crdt_append(code, plan, kinds, ctx, seq, value)?;
            Ok(Flow::Straight)
        }
        // ── DETERMINISTIC SINGLE-THREAD CONCURRENCY (matches the seeded cooperative scheduler for the
        //    non-blocking guide shapes; the corpus lock proves tw == VM(driven) == AOT). ──
        // A `Pipe`/channel is a FIFO queue: `new Pipe` → empty seq, `Send` → append (in place,
        // mutable-shared, no COW), `Receive` → pop the FRONT.
        Op::ChanNew { dst, .. } => {
            emit_empty_header(code, ctx, plan.num_regs, dst as u32);
            Ok(Flow::Straight)
        }
        Op::ChanSend { chan, val } => {
            lower_list_push(code, kinds, ctx, plan.num_regs, chan, val)?;
            Ok(Flow::Straight)
        }
        Op::ChanRecv { dst, chan } => {
            lower_chan_recv(code, kinds, plan.num_regs, dst, chan)?;
            Ok(Flow::Straight)
        }
        // Non-blocking `Try to receive`: a non-empty pipe pops its front value into a fresh Optional
        // box (`Some`, handle != 0); an empty pipe yields `Nothing` (handle 0). The single-task
        // scheduler never parks a try-recv, so there is no blocking/trap path (unlike `ChanRecv`).
        Op::ChanTryRecv { dst, chan } => {
            lower_chan_try_recv(code, kinds, ctx, plan.num_regs, dst, chan)?;
            Ok(Flow::Straight)
        }
        // Non-blocking `Try to send`: the unbounded FIFO always has room, so the value is appended (a
        // plain queue push) and the success result is `Bool(true)` (an i64 `1`), matching the
        // scheduler's `do_try_send`.
        Op::ChanTrySend { dst, chan, val } => {
            lower_list_push(code, kinds, ctx, plan.num_regs, chan, val)?;
            code.push(0x42); // i64.const 1 — Bool(true) rides an i64 local (0/1)
            leb_i64(code, 1);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `Close` a pipe: the scheduler's close resumes the closer with `Nothing` and never mutates the
        // queue (a closed-and-empty recv still just yields `Nothing`). With no result register and no
        // observable queue effect in the deterministic single-task model, it lowers to nothing.
        Op::ChanClose { .. } => Ok(Flow::Straight),
        // `Launch a task to f(args)` — a fire-and-forget task runs SYNCHRONOUSLY (the deterministic
        // scheduler runs each launched task to completion in launch order, which for these
        // non-blocking bodies is exactly a call). `SpawnHandle` also yields a dead `Int` dummy handle.
        Op::Spawn { func, args_start, arg_count } => {
            if emit_sync_call(code, kinds, ctx, func, args_start, arg_count)? {
                code.push(0x1A); // drop the (discarded) result
            }
            Ok(Flow::Straight)
        }
        Op::SpawnHandle { dst, func, args_start, arg_count } => {
            if emit_sync_call(code, kinds, ctx, func, args_start, arg_count)? {
                code.push(0x1A); // drop
            }
            code.push(0x42); // i64.const 0 — the dummy handle
            leb_i64(code, 0);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `Stop <job>` — the task already ran to completion synchronously, so cancelling is a no-op.
        Op::TaskAbort { .. } => Ok(Flow::Straight),
        // `Await <task>` for its value — the task ran to completion synchronously at its `SpawnHandle`, so
        // awaiting is just reading its handle register (`SpawnHandle` leaves an i64 handle there). No
        // compiler emit site produces this today (the language's `Await` lowers to `Select`/`Net` await),
        // so it is never exercised — but the emitter handles it (a pass-through), never refuses it.
        Op::TaskAwait { dst, handle } => {
            local_get(code, handle as u32);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `Sleep N` — under the deterministic scheduler a sleep advances VIRTUAL time only (like a
        // `Select` `After` arm's tick), with no observable output effect on the non-racing shapes, so
        // the AOT lowers it to a no-op — matching the seeded scheduler that drives `vm_outcome_concurrent`.
        Op::Sleep { .. } => Ok(Flow::Straight),
        // ── DETERMINISTIC `select` (`Await the first of: …`). Each arm registers via a
        //    `SelectArm*` op (no code — the winning `SelectWait` reads them back); `SelectWait`
        //    resolves the winner exactly as the seeded scheduler does for the non-racing shapes: a
        //    recv arm whose queue is non-empty wins (pop-front into its var); otherwise the timeout
        //    arm fires. The following per-arm `Eq`/jump dispatch (emitted by the compiler) runs the
        //    winning branch's body. ──
        Op::SelectArmRecv { .. } | Op::SelectArmTimeout { .. } => Ok(Flow::Straight),
        Op::SelectWait { dst_arm } => {
            lower_select_wait(code, plan, kinds, blocks, k, pc, dst_arm)?;
            Ok(Flow::Straight)
        }
        // ── DETERMINISTIC OFFLINE NETWORKING with LOOPBACK delivery (matches the interpreter/VM offline
        //    `NetInbox` loopback: with no relay, a `Send`/`Stream` delivers into our OWN local inbox and
        //    a matching `Await` reads it back — the oracle output is transport-independent). The AOT
        //    models the inbox as ONE local FIFO queue whose handle lives in a reserved memory slot
        //    (`NET_INBOX_ADDR`, inside the null-reserved low-16 region): `Listen` creates it, `Send`/
        //    `Stream` push, `Await` pops. `Connect`/`Sync` remain single-node no-ops. ──
        Op::NetConnect { .. } | Op::NetSync { .. } => Ok(Flow::Straight),
        Op::NetListen { .. } => {
            // Create the empty local inbox FIFO; stash its handle at the reserved slot.
            emit_empty_header(code, ctx, plan.num_regs, plan.num_regs + 8);
            i32_const(code, NET_INBOX_ADDR);
            local_get(code, plan.num_regs + 8);
            i32_store(code, 0);
            Ok(Flow::Straight)
        }
        Op::NetSend { msg, .. } => {
            let elem = kinds.get(msg as usize).ok_or(WasmLowerError::Unsupported("Send of an unknown-kind message"))?;
            i32_const(code, NET_INBOX_ADDR);
            i32_load(code, 0);
            local_set(code, plan.num_regs + 8);
            lower_list_push_at(code, elem, ctx, plan.num_regs, plan.num_regs + 8, msg)?;
            if cow_clonable(kinds.get(msg as usize)) {
                emit_retain(code, msg);
            }
            Ok(Flow::Straight)
        }
        Op::NetStream { values, .. } => {
            // A batch STREAM delivers the whole list; loopback pushes the list HANDLE (single-node, no
            // framing) so `Await stream` pops the same list — byte-faithful to the frame/deframe round-trip.
            let elem = kinds.get(values as usize).ok_or(WasmLowerError::Unsupported("Stream of an unknown-kind list"))?;
            i32_const(code, NET_INBOX_ADDR);
            i32_load(code, 0);
            local_set(code, plan.num_regs + 8);
            lower_list_push_at(code, elem, ctx, plan.num_regs, plan.num_regs + 8, values)?;
            if cow_clonable(kinds.get(values as usize)) {
                emit_retain(code, values);
            }
            Ok(Flow::Straight)
        }
        Op::NetAwait { dst, .. } => {
            let elem = kinds.get(dst as usize).ok_or(WasmLowerError::Unsupported("Await of an unknown-kind message"))?;
            i32_const(code, NET_INBOX_ADDR);
            i32_load(code, 0);
            local_set(code, plan.num_regs + 8);
            emit_pop_front(code, elem, plan.num_regs + 8, plan.num_regs + 5, dst as u32)?;
            if cow_clonable(kinds.get(dst as usize)) {
                emit_retain(code, dst);
            }
            Ok(Flow::Straight)
        }
        Op::NetMakePeer { dst, addr } => {
            local_get(code, addr as u32);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        Op::NewInductive { dst, ctor, args_start, count, .. } => {
            lower_new_inductive(code, kinds, ctx, plan.num_regs, dst, ctor, args_start, count)?;
            Ok(Flow::Straight)
        }
        Op::TestArm { dst, target, variant } => {
            lower_test_arm(code, dst, target, variant);
            Ok(Flow::Straight)
        }
        Op::BindArm { dst, target, index, .. } => {
            lower_bind_arm(code, kinds, dst, target, index)?;
            Ok(Flow::Straight)
        }
        Op::MakeClosure { dst, func, locals_start } => {
            lower_make_closure(code, ctx, plan.num_regs, dst, func, locals_start)?;
            Ok(Flow::Straight)
        }
        Op::CallValue { dst, callee, args_start, arg_count, .. } => {
            // A heap argument to a closure call gains the closure body's parameter as a second holder
            // — RETAIN, like a direct `Call` (value semantics across the closure boundary).
            for a in 0..arg_count {
                let arg = args_start + a;
                if cow_clonable(kinds.get(arg as usize)) {
                    emit_retain(code, arg);
                }
            }
            lower_call_value(code, plan, ctx, pc, dst, callee, args_start, arg_count)?;
            Ok(Flow::Straight)
        }
        Op::IterPop => {
            // Drop the top iterator frame: `__iter_sp += 12`.
            global_get(code, ctx.iter_global);
            i32_const(code, 12);
            code.push(0x6A); // i32.add
            global_set(code, ctx.iter_global);
            Ok(Flow::Straight)
        }
        Op::LoadNow { dst } => {
            let idx = (ctx.host_index)(HostFn::Now).ok_or(WasmLowerError::Unsupported("now not imported"))?;
            code.push(0x10); // call now -> i64 (Moment)
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        Op::GlobalSet { idx, src } => {
            // Storing a heap handle into a global makes the global a second holder — RETAIN so a later
            // mutation through another register copy-on-writes rather than clobbering the global.
            if cow_clonable(kinds.get(src as usize)) {
                emit_retain(code, src);
            }
            local_get(code, src as u32);
            code.push(0x24); // global.set
            leb_u32(code, idx as u32);
            Ok(Flow::Straight)
        }
        Op::Call { dst, func, args_start, arg_count } => {
            // Value semantics: a heap argument gains the callee's parameter as a SECOND holder, so
            // RETAIN it (bump word 12) before the call. A mutation inside the callee then copy-on-writes
            // instead of clobbering the caller's value — the VM's `Rc`-clone-on-argument-bind.
            for a in 0..arg_count {
                let arg = args_start + a;
                if cow_clonable(kinds.get(arg as usize)) {
                    emit_retain(code, arg);
                }
            }
            // Pass each argument at the callee's declared parameter value type, promoting an `Int`
            // argument to `f64` for a `Float` parameter (`half(9)` → `half(9.0)`) instead of pushing
            // an `i64` where the signature wants `f64` (invalid wasm).
            let pvts = ctx.fn_param_valtypes.get(func as usize).ok_or(WasmLowerError::Unsupported("call of unknown function"))?;
            for a in 0..arg_count {
                let arg = args_start + a;
                let arg_vt = kinds.valtype(arg as usize);
                let param_vt = pvts.get(a as usize).copied().unwrap_or(I64);
                if arg_vt == param_vt {
                    local_get(code, arg as u32);
                } else if arg_vt == I64 && param_vt == F64 {
                    push_as_f64(code, arg, kinds.get(arg as usize))?;
                } else {
                    return Err(WasmLowerError::Unsupported("call argument type does not match the parameter"));
                }
            }
            code.push(0x10); // call
            leb_u32(code, ctx.fn_base + func as u32);
            // A void callee leaves nothing on the stack; only bind a result when it returns one.
            if ctx.fn_results.get(func as usize).copied().flatten().is_some() {
                local_set(code, dst as u32);
            }
            Ok(Flow::Straight)
        }
        Op::CallBuiltin { dst, builtin: BuiltinId::Pow, args_start, arg_count } => {
            lower_pow(code, kinds, ctx, plan.num_regs, dst, args_start, arg_count)
        }
        // `parseInt(text)` — a host call: push the Text handle, `call parse_int` → i64.
        Op::CallBuiltin { dst, builtin: BuiltinId::ParseInt, args_start, .. } => {
            let idx = (ctx.host_index)(HostFn::ParseInt).ok_or(WasmLowerError::Unsupported("parse_int host not imported"))?;
            local_get(code, args_start as u32);
            code.push(0x10); // call parse_int
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `parseFloat(text) -> Float` — the host parses the `Text` handle (`str::parse::<f64>` after a
        // trim, matching the VM), returning the f64 directly.
        Op::CallBuiltin { dst, builtin: BuiltinId::ParseFloat, args_start, .. } => {
            let idx = (ctx.host_index)(HostFn::ParseFloat).ok_or(WasmLowerError::Unsupported("parse_float host not imported"))?;
            local_get(code, args_start as u32);
            code.push(0x10); // call parse_float
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `chr(code) -> Text` — a one-character Text built inline from the Unicode scalar value
        // (`lower_chr`: UTF-8 encode + a fresh Text object), trapping on an invalid code point.
        Op::CallBuiltin { dst, builtin: BuiltinId::Chr, args_start, .. } => {
            lower_chr(code, ctx, plan.num_regs, dst, args_start);
            Ok(Flow::Straight)
        }
        // `parse_timestamp(text) -> Moment` — the host parses the RFC-3339 `Text` handle to nanoseconds.
        Op::CallBuiltin { dst, builtin: BuiltinId::ParseTimestamp, args_start, .. } => {
            let idx = (ctx.host_index)(HostFn::ParseTimestamp).ok_or(WasmLowerError::Unsupported("parse_timestamp host not imported"))?;
            local_get(code, args_start as u32);
            code.push(0x10); // call parse_timestamp
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `writeWireResidual(text) -> Int` — frame the Text's bytes (`[len:u32][bytes]`) out to the host
        // wire sink (`write_wire_residual(data_ptr, len)`, returning the byte count), the residual-emit
        // half of the wire-program protocol. Requires a `Text` argument (its `[len@0][…][data_ptr@8]`
        // header is read in place); a non-Text is refused, never mis-lowered.
        Op::CallBuiltin { dst, builtin: BuiltinId::WriteWireResidual, args_start, .. } => {
            if kinds.get(args_start as usize) != Some(Kind::Text) {
                return Err(WasmLowerError::Unsupported("writeWireResidual requires a Text argument"));
            }
            let idx = (ctx.host_index)(HostFn::WriteWireResidual).ok_or(WasmLowerError::Unsupported("write_wire_residual host not imported"))?;
            local_get(code, args_start as u32);
            i32_load(code, 8); // the Text's data_ptr
            local_get(code, args_start as u32);
            i32_load(code, 0); // the Text's byte length
            code.push(0x10); // call write_wire_residual → the byte count
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // Calendar/clock component extractors on a `Moment` (`the year of m`, …) → the single
        // `temporal_component(nanos, which)` host. Each builtin passes a distinct `which` selector; the
        // host computes via the same `temporal::*` the VM uses. A `Date` argument (whose hour/min/sec
        // error and whose others need a days→civil path) is not yet lowered — it is refused, not
        // mis-lowered, so the corpus biconditional stays sound.
        Op::CallBuiltin {
            dst,
            builtin:
                b @ (BuiltinId::YearOf
                | BuiltinId::MonthOf
                | BuiltinId::DayOf
                | BuiltinId::WeekdayOf
                | BuiltinId::HourOf
                | BuiltinId::MinuteOf
                | BuiltinId::SecondOf
                | BuiltinId::WeekOf
                | BuiltinId::QuarterOf),
            args_start,
            ..
        } => {
            let which: i32 = match b {
                BuiltinId::YearOf => 0,
                BuiltinId::MonthOf => 1,
                BuiltinId::DayOf => 2,
                BuiltinId::HourOf => 3,
                BuiltinId::MinuteOf => 4,
                BuiltinId::SecondOf => 5,
                BuiltinId::WeekdayOf => 6,
                BuiltinId::WeekOf => 7,
                BuiltinId::QuarterOf => 8,
                _ => unreachable!(),
            };
            match kinds.get(args_start as usize) {
                Some(Kind::Moment) => {
                    let idx = (ctx.host_index)(HostFn::TemporalComponent).ok_or(WasmLowerError::Unsupported("temporal_component host not imported"))?;
                    local_get(code, args_start as u32); // the Moment (i64 nanoseconds)
                    i32_const(code, which);
                    code.push(0x10); // call temporal_component
                    leb_u32(code, idx);
                    local_set(code, dst as u32);
                }
                // A `Date` (days since epoch): the day-based components go through
                // `temporal_component_date`; hour/minute/second have no meaning on a Date (the VM
                // errors), so they are refused rather than mis-lowered.
                Some(Kind::Date) => {
                    if matches!(b, BuiltinId::HourOf | BuiltinId::MinuteOf | BuiltinId::SecondOf) {
                        return Err(WasmLowerError::Unsupported("clock component (hour/minute/second) of a Date — a Date has no time-of-day"));
                    }
                    let idx = (ctx.host_index)(HostFn::TemporalComponentDate).ok_or(WasmLowerError::Unsupported("temporal_component_date host not imported"))?;
                    local_get(code, args_start as u32); // the Date (i32 days since epoch)
                    i32_const(code, which);
                    code.push(0x10); // call temporal_component_date
                    leb_u32(code, idx);
                    local_set(code, dst as u32);
                }
                _ => {
                    return Err(WasmLowerError::Unsupported("temporal component of a non-temporal value"));
                }
            }
            Ok(Flow::Straight)
        }
        // Moment arithmetic + calendar/clock extraction — SELF-CONTAINED i64/i32, matching the VM's
        // `builtins.rs` exactly (no host, no runtime):
        //   `seconds_between(a, b) = (b - a) / 1e9`             (truncating i64 div) -> Int
        //   `add_seconds(m, n)     = m + n * 1e9`                                    -> Moment
        //   `date_of(m)            = m.div_euclid(NANOS_PER_DAY) as i32`  (FLOOR div) -> Date
        //   `time_of(m)            = m.rem_euclid(NANOS_PER_DAY)`  (non-neg remainder) -> Time
        // Each requires a `Moment` first argument (the VM errors otherwise), so a non-Moment is refused,
        // never mis-lowered. `div_euclid`/`rem_euclid` are open-coded branchlessly (the divisor is the
        // positive constant `NANOS_PER_DAY`): `floor = trunc - (rem < 0)`, `euclid_rem = rem + D·(rem < 0)`.
        Op::CallBuiltin {
            dst,
            builtin: b @ (BuiltinId::SecondsBetween | BuiltinId::AddSeconds | BuiltinId::DateOf | BuiltinId::TimeOf),
            args_start,
            ..
        } => {
            const NANOS_PER_DAY: i64 = 86_400_000_000_000;
            const NANOS_PER_SEC: i64 = 1_000_000_000;
            if kinds.get(args_start as usize) != Some(Kind::Moment) {
                return Err(WasmLowerError::Unsupported("temporal arithmetic requires a Moment argument"));
            }
            let i64c = |code: &mut Vec<u8>, v: i64| {
                code.push(0x42); // i64.const
                leb_i64(code, v);
            };
            match b {
                BuiltinId::SecondsBetween => {
                    local_get(code, (args_start + 1) as u32); // b
                    local_get(code, args_start as u32); // a
                    code.push(0x7D); // i64.sub  → b - a
                    i64c(code, NANOS_PER_SEC);
                    code.push(0x7F); // i64.div_s → (b - a) / 1e9
                }
                BuiltinId::AddSeconds => {
                    local_get(code, args_start as u32); // m
                    local_get(code, (args_start + 1) as u32); // n
                    i64c(code, NANOS_PER_SEC);
                    code.push(0x7E); // i64.mul → n * 1e9
                    code.push(0x7C); // i64.add → m + n*1e9
                }
                BuiltinId::DateOf => {
                    local_get(code, args_start as u32);
                    i64c(code, NANOS_PER_DAY);
                    code.push(0x7F); // i64.div_s → q_trunc
                    local_get(code, args_start as u32);
                    i64c(code, NANOS_PER_DAY);
                    code.push(0x81); // i64.rem_s → r_trunc
                    i64c(code, 0);
                    code.push(0x53); // i64.lt_s → (r_trunc < 0):i32
                    code.push(0xAD); // i64.extend_i32_u → 0/1 : i64
                    code.push(0x7D); // i64.sub → q_trunc - (r_trunc<0) = floor(m/D)
                    code.push(0xA7); // i32.wrap_i64 → i32 days-since-epoch (Date)
                }
                BuiltinId::TimeOf => {
                    local_get(code, args_start as u32);
                    i64c(code, NANOS_PER_DAY);
                    code.push(0x81); // i64.rem_s → r_trunc
                    local_get(code, args_start as u32);
                    i64c(code, NANOS_PER_DAY);
                    code.push(0x81); // i64.rem_s → r_trunc (again, as the addend base)
                    i64c(code, 0);
                    code.push(0x53); // i64.lt_s → (r_trunc < 0):i32
                    code.push(0xAD); // i64.extend_i32_u → 0/1 : i64
                    i64c(code, NANOS_PER_DAY);
                    code.push(0x7E); // i64.mul → D·(r_trunc<0)
                    code.push(0x7C); // i64.add → r_trunc + D·(r_trunc<0) = rem_euclid
                }
                _ => unreachable!(),
            }
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // LINKER-mode extended temporal — the calendar logic lives in `base::temporal`, so these
        // delegate to it via a runtime call (guaranteeing bit-identity with the VM): `format_timestamp(m)`
        // → a `Text` handle (RFC-3339 UTC), `months_between`/`years_between`(a, b) → an `Int`. Each needs a
        // `Moment` argument and the linker; a self-contained module refuses them rather than mis-lowering.
        Op::CallBuiltin {
            dst,
            builtin: b @ (BuiltinId::FormatTimestamp | BuiltinId::MonthsBetween | BuiltinId::YearsBetween | BuiltinId::InZone | BuiltinId::LocalInstant),
            args_start,
            ..
        } => {
            if !ctx.linked {
                return Err(WasmLowerError::Unsupported("format_timestamp/months_between/years_between/in_zone/local_instant need the linked runtime (base::temporal)"));
            }
            if kinds.get(args_start as usize) != Some(Kind::Moment) {
                return Err(WasmLowerError::Unsupported("extended temporal requires a Moment argument"));
            }
            let (rt, two_args) = match b {
                BuiltinId::FormatTimestamp => (HostFn::FormatTimestampRt, false),
                BuiltinId::MonthsBetween => (HostFn::MonthsBetweenRt, true),
                BuiltinId::YearsBetween => (HostFn::YearsBetweenRt, true),
                // `in_zone`/`local_instant` take a Moment + a zone-name Text (the runtime reads the Text
                // handle from the shared memory), so the 2nd argument must be a `Text`.
                BuiltinId::InZone => (HostFn::InZoneRt, true),
                BuiltinId::LocalInstant => (HostFn::LocalInstantRt, true),
                _ => unreachable!(),
            };
            if matches!(b, BuiltinId::InZone | BuiltinId::LocalInstant)
                && kinds.get((args_start + 1) as usize) != Some(Kind::Text)
            {
                return Err(WasmLowerError::Unsupported("in_zone/local_instant require a Text zone name"));
            }
            let idx = (ctx.host_index)(rt).ok_or(WasmLowerError::Unsupported("extended temporal runtime fn not imported"))?;
            local_get(code, args_start as u32);
            if two_args {
                local_get(code, (args_start + 1) as u32);
            }
            code.push(0x10); // call the logos_rt_* runtime fn
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // The general SIMD lane vocabulary (`base::LanesVal`, LINKER MODE): constructors (`lanes16Word8`/
        // `lanes8Word32`/`lanes4Word64` from a `Seq`, `splat16Word8`/`splat8Word32` from an `Int`) and the
        // extractors (`seqOfLanes16W8`/`seqOfLanes8` → a `Seq`) take one argument; the byte/word-lane ops
        // (`shuffle16`/`interleave*`/`byteAdd16`/`maddubs16`/`packus16`) and `shrBytes16` take two. Each is
        // a single `logos_rt_lanes_*` call delegating to the pure-Rust `base::word` spec.
        Op::CallBuiltin { dst, builtin: b, args_start, .. } if ctx.linked && lanes_v_host_fn(b).is_some() => {
            let rt = lanes_v_host_fn(b).expect("guarded by lanes_v_host_fn(b).is_some()");
            let idx = (ctx.host_index)(rt).ok_or(WasmLowerError::Unsupported("lane-vector runtime fn not imported"))?;
            let two = matches!(
                b,
                BuiltinId::Shuffle16
                    | BuiltinId::InterleaveLo16
                    | BuiltinId::InterleaveHi16
                    | BuiltinId::ByteAdd16
                    | BuiltinId::Maddubs16
                    | BuiltinId::Packus16
                    | BuiltinId::ShrBytes16
            );
            local_get(code, args_start as u32);
            if two {
                local_get(code, (args_start + 1) as u32);
            }
            code.push(0x10); // call the logos_rt_lanes_* runtime fn
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // Money FX (LINKER MODE, over `base::money`'s ambient rate table): `set_rate(code, rate)` installs
        // one rate (coercing the rate arg to a `Rational` handle IN-PLACE by kind — `Int`→`rational_from_i64`,
        // `Decimal`→`decimal_to_rational`, `Rational`→as-is), returning the `Nothing` handle (0).
        Op::CallBuiltin { dst, builtin: BuiltinId::SetRate, args_start, .. } if ctx.linked => {
            let set_idx = (ctx.host_index)(HostFn::MoneySetRate).ok_or(WasmLowerError::Unsupported("set_rate runtime fn not imported"))?;
            local_get(code, args_start as u32); // the currency-code Text handle
            match kinds.get((args_start + 1) as usize) {
                Some(Kind::Rational) => local_get(code, (args_start + 1) as u32),
                Some(Kind::Int) => {
                    let c = (ctx.host_index)(HostFn::RationalFromI64).ok_or(WasmLowerError::Unsupported("rational_from_i64 not imported"))?;
                    local_get(code, (args_start + 1) as u32);
                    code.push(0x10);
                    leb_u32(code, c);
                }
                Some(Kind::Decimal) => {
                    let c = (ctx.host_index)(HostFn::DecimalToRational).ok_or(WasmLowerError::Unsupported("decimal_to_rational not imported"))?;
                    local_get(code, (args_start + 1) as u32);
                    code.push(0x10);
                    leb_u32(code, c);
                }
                _ => return Err(WasmLowerError::Unsupported("set_rate rate must be an Int, Decimal, or Rational")),
            }
            code.push(0x10); // call set_rate → 0 (Nothing)
            leb_u32(code, set_idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `to_currency(money, code)` — convert a `Money` into the named currency via the ambient rates.
        Op::CallBuiltin { dst, builtin: BuiltinId::ToCurrency, args_start, .. } if ctx.linked => {
            let idx = (ctx.host_index)(HostFn::MoneyToCurrency).ok_or(WasmLowerError::Unsupported("to_currency runtime fn not imported"))?;
            local_get(code, args_start as u32); // the Money handle
            local_get(code, (args_start + 1) as u32); // the currency-code Text handle
            code.push(0x10);
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `set_rates(map)` — install a whole `Map of <code> to <rate>`, dispatched on the map's VALUE kind
        // (resolved by scanning the region for the `SetIndex` that populated it), returning `Nothing` (0).
        Op::CallBuiltin { dst, builtin: BuiltinId::SetRates, args_start, .. } if ctx.linked => {
            let vk = set_rates_value_kind(&plan.ops, args_start, kinds);
            let rt = match vk {
                Some(Kind::Int) => HostFn::MoneySetRatesInt,
                Some(Kind::Rational) => HostFn::MoneySetRatesRational,
                Some(Kind::Decimal) => HostFn::MoneySetRatesDecimal,
                _ => return Err(WasmLowerError::Unsupported("set_rates needs a Map of currency code to an Int/Decimal/Rational rate whose value kind is statically known")),
            };
            let idx = (ctx.host_index)(rt).ok_or(WasmLowerError::Unsupported("set_rates runtime fn not imported"))?;
            local_get(code, args_start as u32); // the Map handle
            code.push(0x10);
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `wireBytes(value) -> Seq of Int` (LINKER MODE) — marshal the value to its wire bytes via the REAL
        // `logicaffeine_compile::concurrency::marshal::encode_value_raw` (a `logos_rt_wire_bytes_*` runtime
        // fn reconstructs the `RuntimeValue` from the AOT value, by kind, and encodes), byte-identical to
        // the VM's `bytes_to_seq(encode_value_raw(v))`. A composite value's reconstruction is not yet wired
        // (soundly refused). `gc-sections` keeps the compiler out of any module that never calls this.
        Op::CallBuiltin { dst, builtin: BuiltinId::WireBytes, args_start, .. } if ctx.linked => {
            let h = wire_bytes_host_fn(kinds.get(args_start as usize))
                .ok_or(WasmLowerError::Unsupported("wireBytes of this value kind is not yet reconstructed for the wire codec"))?;
            let idx = (ctx.host_index)(h).ok_or(WasmLowerError::Unsupported("wire_bytes runtime fn not imported"))?;
            local_get(code, args_start as u32); // the value (i64 / f64 / i32 Text handle by kind)
            code.push(0x10);
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `readWireProgram() -> a DYNAMIC value` (LINKER MODE) — alloc a scratch buffer, have the host
        // (`read_wire_frame`) write the wire frame into it (returning the byte length), then DECODE it via
        // the REAL `decode_value_raw` to a leaked `Box<RuntimeValue>` (Kind::Dynamic). The buffer size caps
        // the received program. The result's concrete type is only known at runtime — that's why it's boxed.
        Op::CallBuiltin { dst, builtin: BuiltinId::ReadWireProgram, .. } if ctx.linked => {
            const WIRE_BUF: i32 = 1 << 16;
            let frame = (ctx.host_index)(HostFn::ReadWireFrame).ok_or(WasmLowerError::Unsupported("read_wire_frame host not imported"))?;
            let decode = (ctx.host_index)(HostFn::ReadWireProgramRt).ok_or(WasmLowerError::Unsupported("read_wire_program runtime fn not imported"))?;
            let buf = plan.num_regs + 8; // an i32 heap scratch local (num_regs+5..+11)
            i32_const(code, WIRE_BUF);
            emit_alloc(code, ctx, buf); // buf = bump-alloc(WIRE_BUF)
            local_get(code, buf); // decode's 1st arg
            local_get(code, buf);
            i32_const(code, WIRE_BUF);
            code.push(0x10); // read_wire_frame(buf, WIRE_BUF) -> len
            leb_u32(code, frame);
            code.push(0x10); // logos_rt_read_wire_program(buf, len) -> dynamic handle
            leb_u32(code, decode);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `run_accepted(fn, arg, lo, hi) -> Int` (LINKER MODE) — sandbox-eval a wire-received SHIPPED
        // function through `AcceptanceContract::apply`. `fn` MUST be a `Dynamic` (a `readWireProgram`
        // result holding a `Function{generated}`); an ordinary compiled closure is not a shipped function,
        // so a non-Dynamic first arg is refused (the VM refuses ordinary closures at runtime, likewise).
        Op::CallBuiltin { dst, builtin: BuiltinId::RunAccepted, args_start, .. } if ctx.linked => {
            if kinds.get(args_start as usize) != Some(Kind::Dynamic) {
                return Err(WasmLowerError::Unsupported("run_accepted requires a wire-received (dynamic) shipped function"));
            }
            let idx = (ctx.host_index)(HostFn::RunAccepted).ok_or(WasmLowerError::Unsupported("run_accepted runtime fn not imported"))?;
            local_get(code, args_start as u32); // the dynamic function handle (i32)
            local_get(code, (args_start + 1) as u32); // arg (i64)
            local_get(code, (args_start + 2) as u32); // lo (i64)
            local_get(code, (args_start + 3) as u32); // hi (i64)
            code.push(0x10); // logos_rt_run_accepted(fn, arg, lo, hi) -> i64
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `format(x) -> Text` — `x.to_display_string()` as a Text, the SAME materialization a `+`
        // concat performs on a non-Text operand (an empty `format()` yields an empty Text).
        Op::CallBuiltin { dst, builtin: BuiltinId::Format, args_start, arg_count } => {
            if arg_count == 0 {
                lower_text_literal(code, ctx, plan.num_regs, b""); // leaves the handle on the stack
                local_set(code, dst as u32);
            } else {
                // `emit_stringify` writes the Text handle straight into the `dst` local.
                emit_stringify(code, ctx, plan.num_regs, args_start as u32, kinds.get(args_start as usize), dst as u32)?;
            }
            Ok(Flow::Straight)
        }
        // `repeatSeq(x, n)` — a fresh `n`-element sequence of the scalar `x` (`[x] * n`).
        Op::CallBuiltin { dst, builtin: BuiltinId::RepeatSeq, args_start, arg_count } => {
            if arg_count != 2 {
                return Err(WasmLowerError::Unsupported("repeatSeq arity"));
            }
            lower_repeat_seq(code, kinds, ctx, plan.num_regs, dst, args_start)?;
            Ok(Flow::Straight)
        }
        // `money(amount, "USD")` — build an EXACT Money handle. `amount` is a Decimal handle or an
        // Int (whole units); the currency is a Text code resolved by `currency::by_code` in the runtime.
        Op::CallBuiltin { dst, builtin: BuiltinId::Money, args_start, arg_count } if ctx.linked && arg_count == 2 => {
            if kinds.get((args_start + 1) as usize) != Some(Kind::Text) {
                return Err(WasmLowerError::Unsupported("money(amount, currency) with a non-Text currency"));
            }
            let host = match kinds.get(args_start as usize) {
                Some(Kind::Decimal) => HostFn::MoneyFromDecimal,
                Some(Kind::Int) => HostFn::MoneyFromI64,
                _ => return Err(WasmLowerError::Unsupported("money amount must be an Int or Decimal")),
            };
            let make = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("money constructor not imported"))?;
            local_get(code, args_start as u32);
            local_get(code, (args_start + 1) as u32);
            code.push(0x10);
            leb_u32(code, make);
            local_set(code, dst as u32);
            return Ok(Flow::Straight);
        }
        // `quantity(v, "unit")` — build an EXACT Quantity handle: an Int magnitude + a unit name Text
        // resolved by `units::by_name` in the runtime (the `5 meters` literal lowers to this too).
        Op::CallBuiltin { dst, builtin: BuiltinId::Quantity, args_start, arg_count } if ctx.linked && arg_count == 2 => {
            if kinds.get((args_start + 1) as usize) != Some(Kind::Text) {
                return Err(WasmLowerError::Unsupported("quantity(value, unit) with a non-Text unit"));
            }
            if kinds.get(args_start as usize) != Some(Kind::Int) {
                return Err(WasmLowerError::Unsupported("quantity magnitude must be an Int"));
            }
            let make = (ctx.host_index)(HostFn::QuantityOfI64).ok_or(WasmLowerError::Unsupported("quantity constructor not imported"))?;
            local_get(code, args_start as u32);
            local_get(code, (args_start + 1) as u32);
            code.push(0x10);
            leb_u32(code, make);
            local_set(code, dst as u32);
            return Ok(Flow::Straight);
        }
        // `convert(q, "unit")` (the surface `X in <unit>`) — re-express a Quantity in a new display unit
        // of the SAME dimension (dimension-checked at compile time); the runtime keeps the SI magnitude.
        Op::CallBuiltin { dst, builtin: BuiltinId::Convert, args_start, arg_count } if ctx.linked && arg_count == 2 => {
            if kinds.get(args_start as usize) != Some(Kind::Quantity) {
                return Err(WasmLowerError::Unsupported("convert() requires a Quantity"));
            }
            if kinds.get((args_start + 1) as usize) != Some(Kind::Text) {
                return Err(WasmLowerError::Unsupported("convert(q, unit) with a non-Text unit"));
            }
            let make = (ctx.host_index)(HostFn::QuantityConvert).ok_or(WasmLowerError::Unsupported("quantity convert not imported"))?;
            local_get(code, args_start as u32);
            local_get(code, (args_start + 1) as u32);
            code.push(0x10);
            leb_u32(code, make);
            local_set(code, dst as u32);
            return Ok(Flow::Straight);
        }
        // `decimal("…")` — parse a Text arg into an exact Decimal via the runtime.
        Op::CallBuiltin { dst, builtin: BuiltinId::Decimal, args_start, arg_count } if ctx.linked && arg_count == 1 => {
            if kinds.get(args_start as usize) != Some(Kind::Text) {
                return Err(WasmLowerError::Unsupported("decimal(x) with a non-Text argument"));
            }
            let from = (ctx.host_index)(HostFn::DecimalFromText).ok_or(WasmLowerError::Unsupported("decimal_from_text not imported"))?;
            local_get(code, args_start as u32);
            code.push(0x10);
            leb_u32(code, from);
            local_set(code, dst as u32);
            return Ok(Flow::Straight);
        }
        // `complex(re, im)` — build an EXACT Complex handle from two Int components via the runtime.
        Op::CallBuiltin { dst, builtin: BuiltinId::Modular, args_start, arg_count } if ctx.linked && arg_count == 2 => {
            if kinds.get(args_start as usize) != Some(Kind::Int) || kinds.get((args_start + 1) as usize) != Some(Kind::Int) {
                return Err(WasmLowerError::Unsupported("modular(v, n) with non-Int components"));
            }
            let from = (ctx.host_index)(HostFn::ModularFromI64).ok_or(WasmLowerError::Unsupported("modular_from_i64 not imported"))?;
            local_get(code, args_start as u32);
            local_get(code, (args_start + 1) as u32);
            code.push(0x10);
            leb_u32(code, from);
            local_set(code, dst as u32);
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin { dst, builtin: BuiltinId::Complex, args_start, arg_count } if ctx.linked && arg_count == 2 => {
            if kinds.get(args_start as usize) != Some(Kind::Int) || kinds.get((args_start + 1) as usize) != Some(Kind::Int) {
                return Err(WasmLowerError::Unsupported("complex(re, im) with non-Int components"));
            }
            let from = (ctx.host_index)(HostFn::ComplexFromI64).ok_or(WasmLowerError::Unsupported("complex_from_i64 not imported"))?;
            local_get(code, args_start as u32);
            local_get(code, (args_start + 1) as u32);
            code.push(0x10); // call logos_rt_complex_from_i64
            leb_u32(code, from);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `floor`/`ceil`/`round`/`abs` of a LINKED `Rational`: EXACT rounding on the BigInt-backed
        // fraction (`logos_rt_rational_*`) — floor/ceil/round yield a BigInt handle, abs a Rational one —
        // never the lossy `f64` path. Handled here (not `lower_builtin`) for the `ctx` host table.
        Op::CallBuiltin { dst, builtin: b @ (BuiltinId::Floor | BuiltinId::Ceil | BuiltinId::Round | BuiltinId::Abs), args_start, .. }
            if ctx.linked && kinds.get(args_start as usize) == Some(Kind::Rational) =>
        {
            let host = match b {
                BuiltinId::Floor => HostFn::RationalFloor,
                BuiltinId::Ceil => HostFn::RationalCeil,
                BuiltinId::Round => HostFn::RationalRound,
                _ => HostFn::RationalAbs,
            };
            lower_rational_unary(code, ctx, dst, args_start, host)
        }
        // The LINKED `Uuid` builtins: `uuid("…")` parse + `uuid_version` take one arg; the `uuid_nil`/
        // `uuid_max`/`uuid_dns`/… constants take none. Each is a direct `logos_rt_uuid_*` call.
        Op::CallBuiltin { dst, builtin: b @ (BuiltinId::Uuid | BuiltinId::UuidNil | BuiltinId::UuidMax | BuiltinId::UuidDns | BuiltinId::UuidUrl | BuiltinId::UuidOid | BuiltinId::UuidX500 | BuiltinId::UuidVersion), args_start, .. }
            if ctx.linked =>
        {
            let host = match b {
                BuiltinId::Uuid => HostFn::UuidParse,
                BuiltinId::UuidNil => HostFn::UuidNil,
                BuiltinId::UuidMax => HostFn::UuidMax,
                BuiltinId::UuidDns => HostFn::UuidDns,
                BuiltinId::UuidUrl => HostFn::UuidUrl,
                BuiltinId::UuidOid => HostFn::UuidOid,
                BuiltinId::UuidX500 => HostFn::UuidX500,
                _ => HostFn::UuidVersion,
            };
            let idx = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("uuid builtin not imported"))?;
            // `uuid(text)` and `uuid_version(u)` consume the one arg; the constants take none.
            if matches!(b, BuiltinId::Uuid | BuiltinId::UuidVersion) {
                local_get(code, args_start as u32);
            }
            code.push(0x10);
            leb_u32(code, idx);
            local_set(code, dst as u32);
            return Ok(Flow::Straight);
        }
        // Byte interop: `text_bytes`/`uuid_bytes` build a `Seq of Int` of raw bytes; `text_from_bytes`
        // rebuilds a `Text`; `uuid_from_bytes` packs 16 bytes and boxes a `Uuid` (linker). Emitter-heap
        // seq/Text construction — no host except `uuid_from_bytes`'s `logos_rt_uuid_from_ptr`.
        Op::CallBuiltin { dst, builtin: BuiltinId::TextBytes, args_start, .. } => {
            lower_text_bytes(code, ctx, plan.num_regs, dst, args_start);
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin { dst, builtin: BuiltinId::UuidBytes, args_start, .. } if ctx.linked => {
            lower_uuid_bytes(code, ctx, plan.num_regs, dst, args_start);
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin { dst, builtin: BuiltinId::TextFromBytes, args_start, .. } => {
            lower_text_from_bytes(code, ctx, plan.num_regs, dst, args_start);
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin { dst, builtin: BuiltinId::UuidFromBytes, args_start, .. } if ctx.linked => {
            return lower_uuid_from_bytes(code, ctx, plan.num_regs, dst, args_start);
        }
        // The SHA-1 SHA-NI lane vocabulary (linker): construction/unpack are inline, the four rounds call
        // the `logos_rt_sha1*` runtime (which delegates to `base::sha_ops`).
        Op::CallBuiltin { dst, builtin: BuiltinId::Lanes4Of, args_start, .. } if ctx.linked => {
            lower_lanes4_of(code, ctx, plan.num_regs, dst, [args_start, args_start + 1, args_start + 2, args_start + 3]);
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin { dst, builtin: BuiltinId::Lanes4Word32Make, args_start, .. } if ctx.linked => {
            lower_lanes4_word32(code, ctx, plan.num_regs, dst, args_start);
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin { dst, builtin: BuiltinId::SeqOfLanes4W32, args_start, .. } if ctx.linked => {
            lower_seq_of_lanes4(code, ctx, plan.num_regs, dst, args_start);
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin { dst, builtin: b @ (BuiltinId::Sha1Rnds4 | BuiltinId::Sha1Msg1 | BuiltinId::Sha1Msg2 | BuiltinId::Sha1Nexte), args_start, .. } if ctx.linked => {
            let (host, ternary) = match b {
                BuiltinId::Sha1Rnds4 => (HostFn::Sha1Rnds4, true),
                BuiltinId::Sha1Msg1 => (HostFn::Sha1Msg1, false),
                BuiltinId::Sha1Msg2 => (HostFn::Sha1Msg2, false),
                _ => (HostFn::Sha1Nexte, false),
            };
            return lower_sha1_op(code, ctx, dst, args_start, host, ternary);
        }
        Op::CallBuiltin { dst, builtin, args_start, arg_count } => {
            lower_builtin(code, kinds, dst, builtin, args_start, arg_count)
        }
        // `args()` — the host returns the argv `Seq of Text` handle (built in this module's memory).
        Op::Args { dst } => {
            let idx = (ctx.host_index)(HostFn::Args).ok_or(WasmLowerError::Unsupported("args host not imported"))?;
            code.push(0x10); // call args
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        Op::Show { src } => {
            // A whole tuple assembles its `(e0, e1, …)` display inline from the static layout
            // (deterministic order), then prints it — no per-kind host sink exists for it. This is
            // keyed on the tuple LAYOUT (populated by `NewTuple`), not the Kind: a HOMOGENEOUS tuple
            // like `(10, 20)` collapses to `Kind::SeqInt` for its element machinery yet must still
            // display with tuple parens `(10, 20)`, not list brackets `[10, 20]`. A real list literal
            // (`NewList`) is absent from `tuple_layouts`, so it keeps its `[…]` sink.
            if plan.structs.tuple_layouts.contains_key(&src) {
                lower_show_tuple(code, plan, ctx, src)?;
                return Ok(Flow::Straight);
            }
            let kind = kinds.get(src as usize).ok_or(WasmLowerError::Unsupported("Show of an unknown-kind value"))?;
            // A whole enum (`North`, `Ctor`) prints its variant name via a tag→name dispatch built
            // from the enum type's variant set — no per-kind host sink exists for it.
            if kind == Kind::Enum {
                lower_show_enum(code, plan, ctx, src)?;
                return Ok(Flow::Straight);
            }
            // A whole struct (`Point { x: 1, y: 2 }`) — fields in deterministic alphabetical order,
            // matching the VM's now-sorted `HashMap` display.
            if kind == Kind::Struct {
                lower_show_struct(code, plan, ctx, src)?;
                return Ok(Flow::Straight);
            }
            // A whole `Seq of Struct` (`[Point { x: 1, y: 2 }, …]`) — each element struct rendered in
            // the same deterministic field order, concatenated into the `[…]` list display.
            if kind == Kind::SeqStruct {
                lower_show_seqstruct(code, plan, ctx, src)?;
                return Ok(Flow::Straight);
            }
            // A whole Map assembles its `{k0: v0, k1: v1, …}` display inline by iterating its entries
            // in stored order — which is INSERTION order, matching the VM's `IndexMap` (they share the
            // same `MapStorage`), so the rendering is byte-identical.
            // A NESTED int sequence (`[[1, 2], [3, 4]]`) assembles `[[…], […]]` by iterating the outer
            // seq and rendering each inner `Seq of Int` with the scalar seq formatter — deterministic
            // (both tiers store lists in insertion order).
            if kind == Kind::SeqSeqInt {
                lower_show_seqseq(code, ctx, plan.num_regs, src)?;
                return Ok(Flow::Straight);
            }
            // A whole `Seq of Enum` (`[North, South]`, `[Circle(5), Dot]`) renders `[e0, e1, …]`, each
            // element by the enum's tag→name dispatch (nullary name or `Ctor(fields)`), insertion order.
            if kind == Kind::SeqEnum {
                lower_show_seqenum(code, plan, ctx, src)?;
                return Ok(Flow::Straight);
            }
            if kind == Kind::Map {
                lower_show_map(code, plan, kinds, ctx, src)?;
                return Ok(Flow::Straight);
            }
            // A `Rational`: LINKER mode renders the BigInt-backed handle via `logos_rt_rational_to_text`
            // (`num/den`, or `num` when whole — exactly the VM's `Rational::to_string`) then prints it;
            // the self-contained i64/i64 value loads its two words (num@0, den@8) and hands them to the
            // `print_rational` host, which renders `num/den` (or `num` when `den == 1`).
            if kind == Kind::Rational {
                if ctx.linked {
                    let to_text = (ctx.host_index)(HostFn::RationalToText).ok_or(WasmLowerError::Unsupported("rational_to_text not imported"))?;
                    let print_text = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                    local_get(code, src as u32);
                    code.push(0x10);
                    leb_u32(code, to_text);
                    code.push(0x10);
                    leb_u32(code, print_text);
                    return Ok(Flow::Straight);
                }
                let idx = (ctx.host_index)(HostFn::PrintRational).ok_or(WasmLowerError::Unsupported("Show sink not imported"))?;
                local_get(code, src as u32);
                i64_load(code, 0);
                local_get(code, src as u32);
                i64_load(code, 8);
                code.push(0x10); // call print_rational
                leb_u32(code, idx);
                return Ok(Flow::Straight);
            }
            // A `BigInt` handle (linker mode): render it to a decimal `Text` now
            // (`logos_rt_bigint_to_text` — deferred until here so a Pow/`*` chain could keep computing on
            // real BigInts) and print that Text.
            if kind == Kind::BigInt {
                let to_text = (ctx.host_index)(HostFn::BigintToText).ok_or(WasmLowerError::Unsupported("bigint_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32); // i32 BigInt handle
                code.push(0x10);
                leb_u32(code, to_text); // -> i32 Text handle
                code.push(0x10);
                leb_u32(code, print_text); // print the decimal
                return Ok(Flow::Straight);
            }
            // A `Complex` handle: render `re±imi` to a Text via the runtime, then print it.
            if kind == Kind::Complex {
                let to_text = (ctx.host_index)(HostFn::ComplexToText).ok_or(WasmLowerError::Unsupported("complex_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32); // i32 Complex handle
                code.push(0x10);
                leb_u32(code, to_text); // -> i32 Text handle
                code.push(0x10);
                leb_u32(code, print_text);
                return Ok(Flow::Straight);
            }
            // A `Modular` handle: render `v (mod n)` via the runtime, then print it.
            if kind == Kind::Modular {
                let to_text = (ctx.host_index)(HostFn::ModularToText).ok_or(WasmLowerError::Unsupported("modular_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32);
                code.push(0x10);
                leb_u32(code, to_text);
                code.push(0x10);
                leb_u32(code, print_text);
                return Ok(Flow::Straight);
            }
            if kind == Kind::Decimal {
                let to_text = (ctx.host_index)(HostFn::DecimalToText).ok_or(WasmLowerError::Unsupported("decimal_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32);
                code.push(0x10);
                leb_u32(code, to_text);
                code.push(0x10);
                leb_u32(code, print_text);
                return Ok(Flow::Straight);
            }
            if kind == Kind::Money {
                let to_text = (ctx.host_index)(HostFn::MoneyToText).ok_or(WasmLowerError::Unsupported("money_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32);
                code.push(0x10);
                leb_u32(code, to_text);
                code.push(0x10);
                leb_u32(code, print_text);
                return Ok(Flow::Straight);
            }
            // A `Quantity` handle: render `<magnitude> <symbol>` (or the dimension signature) via the
            // runtime, mirroring the interpreter's `QuantityValue::display`, then print it.
            if kind == Kind::Quantity {
                let to_text = (ctx.host_index)(HostFn::QuantityToText).ok_or(WasmLowerError::Unsupported("quantity_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32);
                code.push(0x10);
                leb_u32(code, to_text);
                code.push(0x10);
                leb_u32(code, print_text);
                return Ok(Flow::Straight);
            }
            // A `Uuid` handle: render the canonical lowercase form via `logos_rt_uuid_to_text`, then print.
            if kind == Kind::Uuid {
                let to_text = (ctx.host_index)(HostFn::UuidToText).ok_or(WasmLowerError::Unsupported("uuid_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32);
                code.push(0x10);
                leb_u32(code, to_text);
                code.push(0x10);
                leb_u32(code, print_text);
                return Ok(Flow::Straight);
            }
            // A wire-decoded DYNAMIC value: render its boxed `RuntimeValue` via `to_display_string`, print.
            if kind == Kind::Dynamic {
                let to_text = (ctx.host_index)(HostFn::DynamicToText).ok_or(WasmLowerError::Unsupported("dynamic_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32);
                code.push(0x10);
                leb_u32(code, to_text);
                code.push(0x10);
                leb_u32(code, print_text);
                return Ok(Flow::Straight);
            }
            // An `Optional` handle: a null (`0`) handle prints "nothing"; otherwise the boxed inner
            // scalar (`box[0]`) is loaded at its own width and printed via its own sink. The present
            // inner kind comes from `opt_inner` (the producing channel's element kind).
            if kind == Kind::Optional {
                let nothing = (ctx.host_index)(HostFn::PrintNothing).ok_or(WasmLowerError::Unsupported("Show sink not imported"))?;
                let inner = plan.structs.opt_inner.get(&src).copied().unwrap_or(Kind::Int);
                let some_host = HostFn::for_show(inner).ok_or(WasmLowerError::Unsupported("Show of an optional with a non-scalar inner"))?;
                let some_idx = (ctx.host_index)(some_host).ok_or(WasmLowerError::Unsupported("Show sink not imported"))?;
                local_get(code, src as u32);
                code.push(0x45); // i32.eqz → is the handle null (Nothing)?
                code.push(0x04);
                code.push(0x40); // if (void)
                code.push(0x10); // call print_nothing
                leb_u32(code, nothing);
                code.push(0x05); // else (Some)
                local_get(code, src as u32);
                emit_slot_load(code, Some(inner), 0)?; // box[0] → the inner value at its width
                if inner == Kind::Bool {
                    code.push(0xA7); // i32.wrap_i64 — print_bool takes an i32
                }
                code.push(0x10); // call print_<inner>
                leb_u32(code, some_idx);
                code.push(0x0B); // end if
                return Ok(Flow::Straight);
            }
            // A `Word32`/`Word64` Shows as its UNSIGNED value via `print_word` — a `Word32` is
            // zero-extended to `i64` first so the host's `u64` reading equals the `u32` value.
            if kind == Kind::Word32 || kind == Kind::Word64 {
                let idx = (ctx.host_index)(HostFn::PrintWord).ok_or(WasmLowerError::Unsupported("Show sink not imported"))?;
                local_get(code, src as u32);
                if kind == Kind::Word32 {
                    code.push(0xAD); // i64.extend_i32_u
                }
                code.push(0x10); // call print_word
                leb_u32(code, idx);
                return Ok(Flow::Straight);
            }
            let host = HostFn::for_show(kind).ok_or(WasmLowerError::Unsupported("Show of a non-scalar value"))?;
            let idx = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("Show sink not imported"))?;
            local_get(code, src as u32);
            if kind == Kind::Bool {
                code.push(0xA7); // i32.wrap_i64 — print_bool takes an i32
            }
            code.push(0x10); // call print_*
            leb_u32(code, idx);
            Ok(Flow::Straight)
        }
        Op::Return { src } => {
            if plan.result.is_none() {
                return Err(WasmLowerError::Unsupported("value return from a void function"));
            }
            local_get(code, src as u32);
            code.push(0x0F); // return
            Ok(Flow::Terminated)
        }
        Op::ReturnNothing => {
            match plan.result {
                None => code.push(0x0F),    // return (void)
                Some(_) => code.push(0x00), // unreachable (typed function: never returns nothing)
            }
            Ok(Flow::Terminated)
        }
        Op::Halt => {
            code.push(0x0F); // return — Main is void
            Ok(Flow::Terminated)
        }
        // A runtime failure (`FailWith`, e.g. an undefined variable or an explicit fail). A
        // standalone module has no VM to surface the message, so it traps — the documented
        // error contract (the `wasm_traps_where_treewalker_errors` lock proves tw-errors ⟺
        // wasm-traps). The message constant is intentionally dropped.
        Op::FailWith { .. } => {
            code.push(0x00); // unreachable → trap
            Ok(Flow::Terminated)
        }
        other => Err(unsupported_op(&other)),
    }
}

#[derive(Clone, Copy)]
enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

/// Lower a binary arithmetic op, dispatching on the result kind: checked `i64` for `Int`
/// (traps on signed overflow, matching the VM's exact-int → BigInt contract), native `f64` for
/// `Float`.
fn lower_arith(code: &mut Vec<u8>, kinds: &KindTable, dst: u16, lhs: u16, rhs: u16, op: ArithOp) -> R<Flow> {
    match kinds.get(dst as usize) {
        Some(Kind::Int) => {
            // Integer arithmetic needs two `i64` operands; a Float operand with an `Int` result is a
            // kind inconsistency — reject rather than emit an `i64` op on an `f64` (invalid wasm).
            if kinds.valtype(lhs as usize) != I64 || kinds.valtype(rhs as usize) != I64 {
                return Err(WasmLowerError::Unsupported("integer arithmetic with a non-integer operand"));
            }
            match op {
                ArithOp::Add => emit_checked_addsub(code, false, dst, lhs, rhs),
                ArithOp::Sub => emit_checked_addsub(code, true, dst, lhs, rhs),
                ArithOp::Mul => emit_checked_mul(code, dst, lhs, rhs),
                ArithOp::Div => arith(code, 0x7F, dst, lhs, rhs), // i64.div_s (traps on /0, MIN/-1)
                ArithOp::Mod => arith(code, 0x81, dst, lhs, rhs), // i64.rem_s
            }
        }
        // Word arithmetic is the ℤ/2ⁿ ring: native wasm `i32`/`i64` ops WRAP by definition (no overflow
        // check, unlike `Int`'s checked path), and division/remainder are UNSIGNED — matching `WordVal`.
        Some(Kind::Word32) => {
            let opcode = match op {
                ArithOp::Add => 0x6A, // i32.add
                ArithOp::Sub => 0x6B, // i32.sub
                ArithOp::Mul => 0x6C, // i32.mul
                ArithOp::Div => 0x6E, // i32.div_u
                ArithOp::Mod => 0x70, // i32.rem_u
            };
            arith(code, opcode, dst, lhs, rhs);
        }
        Some(Kind::Word64) => {
            let opcode = match op {
                ArithOp::Add => 0x7C, // i64.add
                ArithOp::Sub => 0x7D, // i64.sub
                ArithOp::Mul => 0x7E, // i64.mul
                ArithOp::Div => 0x80, // i64.div_u
                ArithOp::Mod => 0x82, // i64.rem_u
            };
            arith(code, opcode, dst, lhs, rhs);
        }
        Some(Kind::Float) => {
            let opcode = match op {
                ArithOp::Add => 0xA0,
                ArithOp::Sub => 0xA1,
                ArithOp::Mul => 0xA2,
                ArithOp::Div => 0xA3,
                ArithOp::Mod => return Err(WasmLowerError::Unsupported("float modulo")),
            };
            // Promote an `Int` operand to `f64` (matching the tree-walker's mixed-expression
            // promotion), so `3 + 1.5` etc. compile instead of emitting an `f64` op on an `i64`.
            push_as_f64(code, lhs, kinds.get(lhs as usize))?;
            push_as_f64(code, rhs, kinds.get(rhs as usize))?;
            code.push(opcode);
            local_set(code, dst as u32);
        }
        // Temporal arithmetic (`Duration ± Duration = Duration`, `Moment ± Duration = Moment`) is i64
        // nanos that WRAP (matching the VM's `wrapping_add`/`wrapping_sub`) — no overflow check, and only
        // `+`/`-` (the kind arms never route `× ÷ %` here).
        Some(Kind::Duration) | Some(Kind::Time) | Some(Kind::Moment) => {
            let opcode = match op {
                ArithOp::Add => 0x7C, // i64.add
                ArithOp::Sub => 0x7D, // i64.sub
                _ => return Err(WasmLowerError::Unsupported("only + and - on temporal values")),
            };
            arith(code, opcode, dst, lhs, rhs);
        }
        _ => return Err(WasmLowerError::Unsupported("arithmetic on a non-numeric value")),
    }
    Ok(Flow::Straight)
}

#[derive(Clone, Copy)]
enum Cmp {
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
}

/// Lower a comparison, dispatching on the *operand* kind (the result is always a `Bool` i64
/// 0/1). `i64` signed compares for `Int`, ordered `f64` compares for `Float`.
fn lower_compare(code: &mut Vec<u8>, kinds: &KindTable, dst: u16, lhs: u16, rhs: u16, cmp: Cmp) -> R<Flow> {
    let lf = kinds.valtype(lhs as usize) == F64;
    let rf = kinds.valtype(rhs as usize) == F64;
    if lf || rf {
        // Mixed Int/Float equality is EXACT — mathematical values (`1 equals
        // 1.0` is true, but 2^53+1 never equals the float 2^53). The test:
        // convert-both-ways equality (i→f64 rounds, f→i64 truncates — both
        // agreeing pins the exact value) with an upper guard at 2^63 where
        // the saturating truncation would alias i64::MAX. NaN fails the f64
        // compare; a fractional f fails the i64 compare.
        if lf != rf {
            if matches!(cmp, Cmp::Eq | Cmp::Ne) {
                let (int_reg, float_reg) = if lf { (rhs, lhs) } else { (lhs, rhs) };
                // c1: (int as f64) == f
                local_get(code, int_reg as u32);
                code.push(0xB9); // f64.convert_i64_s
                local_get(code, float_reg as u32);
                code.push(0x61); // f64.eq
                // c2: int == trunc_sat(f)
                local_get(code, int_reg as u32);
                local_get(code, float_reg as u32);
                code.push(0xFC); // saturating-truncation prefix
                code.push(0x06); // i64.trunc_sat_f64_s
                code.push(0x51); // i64.eq
                code.push(0x71); // i32.and
                // c3: f < 2^63 (above it trunc_sat aliases i64::MAX)
                local_get(code, float_reg as u32);
                code.push(0x44); // f64.const 2^63
                code.extend_from_slice(&9223372036854775808.0f64.to_le_bytes());
                code.push(0x63); // f64.lt
                code.push(0x71); // i32.and
                if matches!(cmp, Cmp::Ne) {
                    code.push(0x45); // i32.eqz — negate
                }
                code.push(0xAD); // i64.extend_i32_u
                local_set(code, dst as u32);
                return Ok(Flow::Straight);
            }
        }
        let opcode = match cmp {
            Cmp::Lt => 0x63, // f64.lt
            Cmp::Gt => 0x64, // f64.gt
            Cmp::Le => 0x65, // f64.le
            Cmp::Ge => 0x66, // f64.ge
            Cmp::Eq => 0x61, // f64.eq (both Float)
            Cmp::Ne => 0x62, // f64.ne (both Float)
        };
        push_as_f64(code, lhs, kinds.get(lhs as usize))?;
        push_as_f64(code, rhs, kinds.get(rhs as usize))?;
        code.push(opcode);
        code.push(0xAD); // i64.extend_i32_u — keep the VM's truthy-Int boolean width
        local_set(code, dst as u32);
        return Ok(Flow::Straight);
    }
    // `x is (not) equal to nothing` — the only comparison an `Optional` takes part in. Read the
    // Optional operand's i32 handle and test it against the null (`0`) handle; the other operand is
    // the `nothing` literal (the Int `0`), whose own width is irrelevant, so we compare against a
    // fresh `i32.const 0` rather than it. Ordering (`<`/`>`) on an Optional is nonsensical → rejected.
    let (lk, rk) = (kinds.get(lhs as usize), kinds.get(rhs as usize));
    if lk == Some(Kind::Optional) || rk == Some(Kind::Optional) {
        let opt = if lk == Some(Kind::Optional) { lhs } else { rhs };
        let opcode = match cmp {
            Cmp::Eq => 0x46, // i32.eq → handle == 0 (is nothing)
            Cmp::Ne => 0x47, // i32.ne → handle != 0 (is present)
            _ => return Err(WasmLowerError::Unsupported("ordering comparison of optional values")),
        };
        local_get(code, opt as u32);
        i32_const(code, 0);
        code.push(opcode);
        code.push(0xAD); // i64.extend_i32_u — the VM's truthy-Int boolean width
        local_set(code, dst as u32);
        return Ok(Flow::Straight);
    }
    let operand = kinds.get(lhs as usize).or_else(|| kinds.get(rhs as usize));
    match operand {
        // A `Char` compares by code point (`char`'s own ordering), so an `i64` compare of the
        // stored `char as u32` is byte-identical to the VM's `Char` comparison.
        Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Char) | Some(Kind::Duration) | Some(Kind::Time) | Some(Kind::Span) => {
            let opcode = match cmp {
                Cmp::Lt => 0x53, // i64.lt_s
                Cmp::Gt => 0x55, // i64.gt_s
                Cmp::Le => 0x57, // i64.le_s
                Cmp::Ge => 0x59, // i64.ge_s
                Cmp::Eq => 0x51, // i64.eq
                Cmp::Ne => 0x52, // i64.ne
            };
            compare(code, opcode, dst, lhs, rhs);
        }
        // Words compare by their UNSIGNED value (the ℤ/2ⁿ ring order) — `Word32` as `i32` unsigned,
        // `Word64` as `i64` unsigned, matching `WordVal`'s `to_u64`-based comparison.
        Some(Kind::Word32) => {
            let opcode = match cmp {
                Cmp::Lt => 0x49, // i32.lt_u
                Cmp::Gt => 0x4B, // i32.gt_u
                Cmp::Le => 0x4D, // i32.le_u
                Cmp::Ge => 0x4F, // i32.ge_u
                Cmp::Eq => 0x46, // i32.eq
                Cmp::Ne => 0x47, // i32.ne
            };
            compare(code, opcode, dst, lhs, rhs);
        }
        Some(Kind::Word64) => {
            let opcode = match cmp {
                Cmp::Lt => 0x54, // i64.lt_u
                Cmp::Gt => 0x56, // i64.gt_u
                Cmp::Le => 0x58, // i64.le_u
                Cmp::Ge => 0x5A, // i64.ge_u
                Cmp::Eq => 0x51, // i64.eq
                Cmp::Ne => 0x52, // i64.ne
            };
            compare(code, opcode, dst, lhs, rhs);
        }
        // A Float operand has value type `F64`, so it always took the promotion path above.
        Some(Kind::Float) => unreachable!("float comparison handled by the f64 promotion path"),
        Some(Kind::Date) | Some(Kind::Moment) => {
            return Err(WasmLowerError::Unsupported("comparison of temporal values"))
        }
        Some(Kind::SeqInt) | Some(Kind::SeqBool) | Some(Kind::SeqFloat) | Some(Kind::SeqText) | Some(Kind::SeqStruct) | Some(Kind::SeqEnum) | Some(Kind::SeqSeqInt) | Some(Kind::SeqAny) | Some(Kind::SeqWord32) | Some(Kind::SeqWord64) => {
            return Err(WasmLowerError::Unsupported("comparison of sequences"))
        }
        Some(Kind::Text) => return Err(WasmLowerError::Unsupported("comparison of text values")),
        Some(Kind::Struct) => return Err(WasmLowerError::Unsupported("comparison of struct values")),
        Some(Kind::Map) => return Err(WasmLowerError::Unsupported("comparison of map values")),
        Some(Kind::Set) | Some(Kind::SetText) | Some(Kind::CrdtSetText) => return Err(WasmLowerError::Unsupported("comparison of set values")),
        Some(Kind::Enum) => return Err(WasmLowerError::Unsupported("comparison of enum values")),
        Some(Kind::Closure) => return Err(WasmLowerError::Unsupported("comparison of closure values")),
        Some(Kind::Tuple) => return Err(WasmLowerError::Unsupported("comparison of tuple values")),
        Some(Kind::Rational) => return Err(WasmLowerError::Unsupported("comparison of rational values")),
        Some(Kind::BigInt) => return Err(WasmLowerError::Unsupported("comparison of bigint values")),
        Some(Kind::Complex) => return Err(WasmLowerError::Unsupported("comparison of complex values")),
        Some(Kind::Modular) => return Err(WasmLowerError::Unsupported("comparison of modular values")),
        Some(Kind::Decimal) => return Err(WasmLowerError::Unsupported("comparison of decimal values")),
        Some(Kind::Money) => return Err(WasmLowerError::Unsupported("comparison of money values")),
        Some(Kind::Quantity) => return Err(WasmLowerError::Unsupported("comparison of quantity values")),
        Some(Kind::Uuid) => return Err(WasmLowerError::Unsupported("ordering comparison of uuid values")),
        Some(Kind::LanesV) => return Err(WasmLowerError::Unsupported("comparison of lane-vector values")),
        Some(Kind::Dynamic) => return Err(WasmLowerError::Unsupported("comparison of dynamic wire values")),
        Some(Kind::Lanes) => return Err(WasmLowerError::Unsupported("comparison of lane vectors")),
        // An `Optional` operand is fully handled by the is-nothing special case above (either operand
        // being `Optional` returns early), so it never reaches this by-operand-kind dispatch.
        Some(Kind::Optional) => unreachable!("optional comparisons handled by the is-nothing path above"),
        None => return Err(WasmLowerError::Unsupported("comparison of unknown-kind values")),
    }
    Ok(Flow::Straight)
}

/// Push a register as an `f64` on the wasm stack — directly for a Float, via `f64.convert_i64_s`
/// for an Int.
/// `a is approximately b` — the shared isclose semantics
/// (`logicaffeine_data::ops::logos_approx_eq`), lowered as pure f64
/// instructions so the result is bit-identical to every other engine:
/// `(a == b) || |a - b| <= max(1e-9 * max(|a|, |b|), 1e-12)`.
fn lower_approx_eq(code: &mut Vec<u8>, kinds: &KindTable, dst: u16, lhs: u16, rhs: u16) -> R<Flow> {
    let lk = kinds.get(lhs as usize);
    let rk = kinds.get(rhs as usize);
    // c1: a == b (the exact fast path — also makes inf ≈ inf hold).
    push_as_f64(code, lhs, lk)?;
    push_as_f64(code, rhs, rk)?;
    code.push(0x61); // f64.eq → i32
    // diff = |a - b|
    push_as_f64(code, lhs, lk)?;
    push_as_f64(code, rhs, rk)?;
    code.push(0xA1); // f64.sub
    code.push(0x99); // f64.abs
    // tol = max(1e-9 * max(|a|, |b|), 1e-12)
    push_as_f64(code, lhs, lk)?;
    code.push(0x99); // f64.abs
    push_as_f64(code, rhs, rk)?;
    code.push(0x99); // f64.abs
    code.push(0xA5); // f64.max
    code.push(0x44); // f64.const 1e-9
    code.extend_from_slice(&1e-9f64.to_le_bytes());
    code.push(0xA2); // f64.mul
    code.push(0x44); // f64.const 1e-12
    code.extend_from_slice(&1e-12f64.to_le_bytes());
    code.push(0xA5); // f64.max
    // c2: diff <= tol
    code.push(0x65); // f64.le → i32
    code.push(0x72); // i32.or (c1 | c2)
    code.push(0xAD); // i64.extend_i32_u — the VM's truthy-Int boolean width
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

fn push_as_f64(code: &mut Vec<u8>, reg: u16, kind: Option<Kind>) -> R<()> {
    local_get(code, reg as u32);
    match kind {
        Some(Kind::Float) => {}
        Some(Kind::Int) => code.push(0xB9), // f64.convert_i64_s
        _ => return Err(WasmLowerError::Unsupported("numeric builtin on a non-number")),
    }
    Ok(())
}

/// Lower a numeric builtin call, bit-exactly matching the VM (`semantics/builtins.rs`).
fn lower_builtin(code: &mut Vec<u8>, kinds: &KindTable, dst: u16, builtin: BuiltinId, args_start: u16, arg_count: u16) -> R<Flow> {
    let arg = args_start;
    let ak = kinds.get(arg as usize);
    match builtin {
        // `sqrt` → Float (Int converts first), matching `(n as f64).sqrt()`.
        BuiltinId::Sqrt => {
            push_as_f64(code, arg, ak)?;
            code.push(0x9F); // f64.sqrt
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `floor`/`ceil` → Int via the SATURATING truncation (matches `f.floor() as i64`), or the
        // identity on an already-whole Int. (A LINKED `Rational` arg is handled in the main `lower_op`
        // match, which has the `ctx` host table — the exact `logos_rt_rational_*` rounding.)
        BuiltinId::Floor => lower_floor_ceil(code, ak, dst, arg, 0x9C), // f64.floor
        BuiltinId::Ceil => lower_floor_ceil(code, ak, dst, arg, 0x9B),  // f64.ceil
        // `round` → round-half-AWAY-from-zero (Rust's `f64::round`, NOT wasm's round-half-even):
        // `trunc(x + copysign(0.5, x))`, then the saturating cast. Identity on a whole Int.
        BuiltinId::Round => match ak {
            Some(Kind::Float) => {
                local_get(code, arg as u32);
                code.push(0x44); // f64.const 0.5
                code.extend_from_slice(&0.5f64.to_le_bytes());
                local_get(code, arg as u32);
                code.push(0xA6); // f64.copysign → 0.5 with x's sign
                code.push(0xA0); // f64.add → x + copysign(0.5, x)
                code.push(0x9D); // f64.trunc
                code.push(0xFC); // saturating-truncation prefix
                leb_u32(code, 6); // i64.trunc_sat_f64_s
                local_set(code, dst as u32);
                Ok(Flow::Straight)
            }
            Some(Kind::Int) => {
                local_get(code, arg as u32);
                local_set(code, dst as u32);
                Ok(Flow::Straight)
            }
            _ => Err(WasmLowerError::Unsupported("round of a non-number")),
        },
        // `abs`: f64.abs for a Float; for an Int, `x < 0 ? -x : x` with an i64::MIN overflow trap
        // (|i64::MIN| does not fit i64 — the VM promotes to BigInt; the standalone module traps,
        // matching the checked-arithmetic contract).
        BuiltinId::Abs => match ak {
            Some(Kind::Float) => {
                local_get(code, arg as u32);
                code.push(0x99); // f64.abs
                local_set(code, dst as u32);
                Ok(Flow::Straight)
            }
            Some(Kind::Int) => {
                // trap if arg == i64::MIN
                local_get(code, arg as u32);
                code.push(0x42); // i64.const i64::MIN
                leb_i64(code, i64::MIN);
                code.push(0x51); // i64.eq
                code.push(0x04);
                code.push(0x40); // if (void)
                code.push(0x00); // unreachable
                code.push(0x0B); // end
                // x < 0 ? -x : x
                code.push(0x42);
                leb_i64(code, 0); // i64.const 0
                local_get(code, arg as u32);
                code.push(0x7D); // i64.sub → -x  (value-if-true)
                local_get(code, arg as u32); // x  (value-if-false)
                local_get(code, arg as u32);
                code.push(0x42);
                leb_i64(code, 0);
                code.push(0x53); // i64.lt_s → x < 0 (selector)
                code.push(0x1B); // select
                local_set(code, dst as u32);
                Ok(Flow::Straight)
            }
            _ => Err(WasmLowerError::Unsupported("abs of a non-number")),
        },
        BuiltinId::Min => lower_minmax(code, kinds, dst, args_start, arg_count, false),
        BuiltinId::Max => lower_minmax(code, kinds, dst, args_start, arg_count, true),
        // `count_ones(n)` → the population count of the Int's u64 bit pattern (`i64.popcnt`), an Int.
        // Matches the VM's `(n as u64).count_ones() as i64`.
        BuiltinId::CountOnes => {
            local_get(code, arg as u32);
            code.push(0x7B); // i64.popcnt
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // ── Word ring (ℤ/2ⁿ) construct / extract ──
        // `word32(n)` = the low 32 bits of the Int (`i32.wrap_i64`); `word64(n)` = the Int's bits
        // unchanged (an `i64` IS the u64 representation).
        BuiltinId::Word32 => {
            local_get(code, arg as u32);
            code.push(0xA7); // i32.wrap_i64
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        BuiltinId::Word64 => {
            local_get(code, arg as u32);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `intOfWord32(w)` = zero-extend the u32 to Int (`i64.extend_i32_u`); `intOfWord64(w)` = the
        // 64-bit pattern unchanged (Int is i64).
        BuiltinId::IntOfWord32 => {
            local_get(code, arg as u32);
            code.push(0xAD); // i64.extend_i32_u
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        BuiltinId::IntOfWord64 => {
            local_get(code, arg as u32);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // ── Word rotate (`rotl`/`rotr`) — native `i32.rotl`/`i64.rotl` etc., dispatched on the word
        //    operand's width. The rotate count (an Int) narrows to `i32` for a `Word32`. ──
        BuiltinId::Rotl | BuiltinId::Rotr => {
            let is_l = matches!(builtin, BuiltinId::Rotl);
            match kinds.get(args_start as usize) {
                Some(Kind::Word32) => {
                    local_get(code, args_start as u32);
                    local_get(code, (args_start + 1) as u32);
                    code.push(0xA7); // i32.wrap_i64 — count as i32
                    code.push(if is_l { 0x77 } else { 0x78 }); // i32.rotl / i32.rotr
                }
                Some(Kind::Word64) => {
                    local_get(code, args_start as u32);
                    local_get(code, (args_start + 1) as u32);
                    code.push(if is_l { 0x89 } else { 0x8A }); // i64.rotl / i64.rotr
                }
                _ => return Err(WasmLowerError::Unsupported("rotate of a non-Word value")),
            }
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // ── Word bitwise `word_and`/`word_or`/`word_not` — native and/or/xor, dispatched on width. ──
        BuiltinId::Wand | BuiltinId::Wor => {
            let is_and = matches!(builtin, BuiltinId::Wand);
            let op32 = if is_and { 0x71 } else { 0x72 }; // i32.and / i32.or
            let op64 = if is_and { 0x83 } else { 0x84 }; // i64.and / i64.or
            match kinds.get(args_start as usize) {
                Some(Kind::Word32) => {
                    local_get(code, args_start as u32);
                    local_get(code, (args_start + 1) as u32);
                    code.push(op32);
                }
                Some(Kind::Word64) => {
                    local_get(code, args_start as u32);
                    local_get(code, (args_start + 1) as u32);
                    code.push(op64);
                }
                _ => return Err(WasmLowerError::Unsupported("word_and/or of a non-Word value")),
            }
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `word_not(w)` = XOR with all-ones (wasm has no `i32.not`).
        BuiltinId::Wnot => {
            match kinds.get(arg as usize) {
                Some(Kind::Word32) => {
                    local_get(code, arg as u32);
                    i32_const(code, -1);
                    code.push(0x73); // i32.xor
                }
                Some(Kind::Word64) => {
                    local_get(code, arg as u32);
                    i64c(code, -1);
                    code.push(0x85); // i64.xor
                }
                _ => return Err(WasmLowerError::Unsupported("word_not of a non-Word value")),
            }
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // ── `word64Shl`/`word64Shr`/`word64And` — the Word64 shift/mask primitives (Keccak). ──
        BuiltinId::Word64Shl | BuiltinId::Word64Shr | BuiltinId::Word64And => {
            local_get(code, args_start as u32);
            local_get(code, (args_start + 1) as u32);
            code.push(match builtin {
                BuiltinId::Word64Shl => 0x86, // i64.shl
                BuiltinId::Word64Shr => 0x88, // i64.shr_u (Word64 is unsigned)
                _ => 0x83,                    // i64.and
            });
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // ── `word32Shr` — logical right-shift of a Word32 (SHA-256 `σ0`/`σ1`). The shift amount is an
        //    Int (i64) so it narrows to i32; `i32.shr_u` is unsigned, so the vacated high bits are 0. ──
        BuiltinId::Word32Shr => {
            local_get(code, args_start as u32);
            local_get(code, (args_start + 1) as u32);
            code.push(0xA7); // i32.wrap_i64 — shift amount as i32
            code.push(0x76); // i32.shr_u
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        _ => Err(WasmLowerError::Unsupported("builtin not yet lowered")),
    }
}

/// `floor`/`ceil` (`fop` = `f64.floor`/`f64.ceil`): a Float rounds then truncates with the
/// saturating cast to match the VM's `as i64`; an already-whole Int is returned unchanged.
fn lower_floor_ceil(code: &mut Vec<u8>, arg_kind: Option<Kind>, dst: u16, arg: u16, fop: u8) -> R<Flow> {
    match arg_kind {
        Some(Kind::Float) => {
            local_get(code, arg as u32);
            code.push(fop); // f64.floor / f64.ceil
            code.push(0xFC); // saturating-truncation prefix
            leb_u32(code, 6); // i64.trunc_sat_f64_s
            local_set(code, dst as u32);
        }
        Some(Kind::Int) => {
            local_get(code, arg as u32);
            local_set(code, dst as u32);
        }
        _ => return Err(WasmLowerError::Unsupported("floor/ceil of a non-number")),
    }
    Ok(Flow::Straight)
}

/// `min`/`max` (`is_max` selects max). `Int×Int` is an `i64` compare + `select`. Any Float
/// operand promotes both to `f64` and uses `f64.min`/`f64.max` — but with explicit NaN guards,
/// because Rust's `f64::min`/`max` return the NON-NaN argument whereas raw `f64.min`/`f64.max`
/// return NaN. The `±0` case already agrees between the two.
fn lower_minmax(code: &mut Vec<u8>, kinds: &KindTable, dst: u16, args_start: u16, arg_count: u16, is_max: bool) -> R<Flow> {
    if arg_count != 2 {
        return Err(WasmLowerError::Unsupported("min/max arity"));
    }
    let (a, b) = (args_start, args_start + 1);
    let (ak, bk) = (kinds.get(a as usize), kinds.get(b as usize));
    match (ak, bk) {
        (Some(Kind::Int), Some(Kind::Int)) => {
            local_get(code, a as u32); // value-if-true
            local_get(code, b as u32); // value-if-false
            local_get(code, a as u32);
            local_get(code, b as u32);
            code.push(if is_max { 0x55 } else { 0x53 }); // i64.gt_s / i64.lt_s
            code.push(0x1B); // select
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        (a_some, b_some) if a_some.is_some() && b_some.is_some() => {
            let fop = if is_max { 0x98 } else { 0x97 }; // f64.max / f64.min
            // result = a_nan ? b_f : (b_nan ? a_f : fop(a_f, b_f))
            push_as_f64(code, b, bk)?; // [b_f]              (value-if-true for the outer select)
            push_as_f64(code, a, ak)?; // [b_f, a_f]         (value-if-true for the inner select)
            push_as_f64(code, a, ak)?;
            push_as_f64(code, b, bk)?;
            code.push(fop); // [b_f, a_f, fop(a_f,b_f)]      (value-if-false for the inner select)
            push_as_f64(code, b, bk)?;
            push_as_f64(code, b, bk)?;
            code.push(0x62); // f64.ne → b_nan               (inner selector)
            code.push(0x1B); // select → [b_f, inner]
            push_as_f64(code, a, ak)?;
            push_as_f64(code, a, ak)?;
            code.push(0x62); // f64.ne → a_nan                (outer selector)
            code.push(0x1B); // select → [result]
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        _ => Err(WasmLowerError::Unsupported("min/max of non-numbers")),
    }
}

/// The host `pow` import a `pow(base, exp)` needs, by operand kinds: any `^Float` uses
/// `pow_ff` (`f64::powf`); `Float^Int` uses `pow_fi` (`f64::powi`); `Int^Int` uses neither (an
/// integer exponentiation-by-squaring loop, [`lower_int_pow`]).
fn pow_host_for(base: Option<Kind>, exp: Option<Kind>) -> Option<HostFn> {
    match (base, exp) {
        (_, Some(Kind::Float)) => Some(HostFn::PowFf),
        (Some(Kind::Float), Some(Kind::Int)) => Some(HostFn::PowFi),
        _ => None,
    }
}

/// Lower `pow(base, exp)` bit-exactly. Float-result cases (any Float operand) defer to the host
/// `pow_ff`/`pow_fi` (so `powf` vs `powi` exactly match the VM). `Int^Int` is computed in-module
/// by [`lower_int_pow`] (Int result, overflow-trapping). A negative Int exponent (VM → Float)
/// traps, since a Float cannot ride the Int result slot.
fn lower_pow(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, dst: u16, args_start: u16, arg_count: u16) -> R<Flow> {
    if arg_count != 2 {
        return Err(WasmLowerError::Unsupported("pow arity"));
    }
    lower_pow_regs(code, kinds, ctx, num_regs, dst, args_start, args_start + 1)
}

/// Shared core of `pow(base, exp)` and the `**` operator (`Op::Pow`): the two operands are given as
/// explicit registers (adjacent for the builtin, arbitrary for the operator). Float-result cases
/// defer to the host `pow_ff`/`pow_fi`; `Int^Int` is the in-module overflow-trapping squaring loop.
/// Push an i32 `BigInt` handle for `reg` onto the wasm stack: a `Kind::BigInt` register pushes its
/// handle directly; an `Int` register is promoted with `logos_rt_bigint_from_i64`. Brings a mixed
/// `BigInt <op> Int` operand to a common handle type before a `logos_rt_bigint_*` call.
fn push_bigint_operand(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, reg: u16) -> R<()> {
    match kinds.get(reg as usize) {
        Some(Kind::BigInt) => local_get(code, reg as u32),
        Some(Kind::Int) => {
            let from_i64 = (ctx.host_index)(HostFn::BigintFromI64).ok_or(WasmLowerError::Unsupported("bigint_from_i64 not imported"))?;
            local_get(code, reg as u32);
            code.push(0x10);
            leb_u32(code, from_i64);
        }
        _ => return Err(WasmLowerError::Unsupported("bigint arithmetic operand must be a BigInt or an Int")),
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as an exact big-integer binary operation (linker mode) — `op` is the
/// `logos_rt_bigint_{mul,add,sub}` sink. Both operands are brought to BigInt handles (an `Int` operand
/// promoted via `from_i64`); the resulting handle binds `dst`.
fn lower_bigint_binop(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, dst: u16, lhs: u16, rhs: u16, host: HostFn) -> R<Flow> {
    let op = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("bigint op not imported"))?;
    push_bigint_operand(code, kinds, ctx, lhs)?;
    push_bigint_operand(code, kinds, ctx, rhs)?;
    code.push(0x10);
    leb_u32(code, op);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// Push an i32 `Complex` handle for `reg`: a `Kind::Complex` register pushes its handle directly; an
/// `Int` operand `n` promotes to the real Complex `n + 0i` via `logos_rt_complex_from_i64(n, 0)`.
fn push_complex_operand(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, reg: u16) -> R<()> {
    match kinds.get(reg as usize) {
        Some(Kind::Complex) => local_get(code, reg as u32),
        Some(Kind::Int) => {
            let from = (ctx.host_index)(HostFn::ComplexFromI64).ok_or(WasmLowerError::Unsupported("complex_from_i64 not imported"))?;
            local_get(code, reg as u32);
            code.push(0x42); // i64.const 0 — the imaginary part of a promoted real
            leb_i64(code, 0);
            code.push(0x10);
            leb_u32(code, from);
        }
        _ => return Err(WasmLowerError::Unsupported("complex arithmetic operand must be a Complex or an Int")),
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as exact complex arithmetic (linker mode) — `op` is the
/// `logos_rt_complex_{add,sub,mul}` sink; both operands become Complex handles (an `Int` promoted).
fn lower_complex_binop(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, dst: u16, lhs: u16, rhs: u16, host: HostFn) -> R<Flow> {
    let op = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("complex op not imported"))?;
    push_complex_operand(code, kinds, ctx, lhs)?;
    push_complex_operand(code, kinds, ctx, rhs)?;
    code.push(0x10);
    leb_u32(code, op);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// Push an i32 `Modular` handle for `reg` (a `Kind::Modular` register). An `Int` operand can NOT be
/// promoted (the ring modulus is unknown), so a non-Modular operand is soundly refused.
fn push_modular_operand(code: &mut Vec<u8>, kinds: &KindTable, reg: u16) -> R<()> {
    match kinds.get(reg as usize) {
        Some(Kind::Modular) => local_get(code, reg as u32),
        _ => return Err(WasmLowerError::Unsupported("modular arithmetic operand must be a Modular")),
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as exact ℤ/nℤ arithmetic (linker mode) — `op` is the `logos_rt_modular_*` sink.
fn lower_modular_binop(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, dst: u16, lhs: u16, rhs: u16, host: HostFn) -> R<Flow> {
    let op = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("modular op not imported"))?;
    push_modular_operand(code, kinds, lhs)?;
    push_modular_operand(code, kinds, rhs)?;
    code.push(0x10);
    leb_u32(code, op);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// Push an i32 `Decimal` handle for `reg`: a `Kind::Decimal` register pushes its handle; an `Int`
/// operand `n` promotes to the exact Decimal `n` via `logos_rt_decimal_from_i64(n)` (`price * 3`).
fn push_decimal_operand(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, reg: u16) -> R<()> {
    match kinds.get(reg as usize) {
        Some(Kind::Decimal) => local_get(code, reg as u32),
        Some(Kind::Int) => {
            let from = (ctx.host_index)(HostFn::DecimalFromI64).ok_or(WasmLowerError::Unsupported("decimal_from_i64 not imported"))?;
            local_get(code, reg as u32);
            code.push(0x10);
            leb_u32(code, from);
        }
        _ => return Err(WasmLowerError::Unsupported("decimal arithmetic operand must be a Decimal or an Int")),
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as exact base-10 arithmetic (linker mode) — `op` is the `logos_rt_decimal_*` sink.
fn lower_decimal_binop(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, dst: u16, lhs: u16, rhs: u16, host: HostFn) -> R<Flow> {
    let op = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("decimal op not imported"))?;
    push_decimal_operand(code, kinds, ctx, lhs)?;
    push_decimal_operand(code, kinds, ctx, rhs)?;
    code.push(0x10);
    leb_u32(code, op);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

fn push_money_operand(code: &mut Vec<u8>, kinds: &KindTable, _ctx: &Ctx, reg: u16) -> R<()> {
    match kinds.get(reg as usize) {
        Some(Kind::Money) => local_get(code, reg as u32),
        _ => return Err(WasmLowerError::Unsupported("money arithmetic operand must be a Money value")),
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as exact currency arithmetic (linker mode) — `op` is the `logos_rt_money_*` sink.
fn lower_money_binop(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, dst: u16, lhs: u16, rhs: u16, host: HostFn) -> R<Flow> {
    let op = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("money op not imported"))?;
    push_money_operand(code, kinds, ctx, lhs)?;
    push_money_operand(code, kinds, ctx, rhs)?;
    code.push(0x10);
    leb_u32(code, op);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

fn push_quantity_operand(code: &mut Vec<u8>, kinds: &KindTable, _ctx: &Ctx, reg: u16) -> R<()> {
    match kinds.get(reg as usize) {
        Some(Kind::Quantity) => local_get(code, reg as u32),
        // Scalar-scaling a Quantity by a bare number (`q * 2`) has no linked runtime sink yet — refuse
        // it cleanly rather than pass a scalar where the runtime expects a Quantity handle.
        _ => return Err(WasmLowerError::Unsupported("quantity arithmetic operand must be a Quantity value")),
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as exact dimensional arithmetic (linker mode) — `op` is the `logos_rt_quantity_*`
/// sink. `+`/`-` keep the left display unit; `×`/`÷` combine dimensions and render in SI/dimension form.
fn lower_quantity_binop(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, dst: u16, lhs: u16, rhs: u16, host: HostFn) -> R<Flow> {
    let op = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("quantity op not imported"))?;
    push_quantity_operand(code, kinds, ctx, lhs)?;
    push_quantity_operand(code, kinds, ctx, rhs)?;
    code.push(0x10);
    leb_u32(code, op);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// Push a Rational operand as a `logos_rt_rational` handle: a Rational rides as-is, an Int widens via
/// `from_i64`, a BigInt via `from_bigint` (matching the VM's `rat_of` view of every exact number as a
/// Rational). Any other operand is refused (a Rational never mixes with a Float or a heap value).
fn push_rational_operand(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, reg: u16) -> R<()> {
    match kinds.get(reg as usize) {
        Some(Kind::Rational) => local_get(code, reg as u32),
        Some(Kind::Int) => {
            let from = (ctx.host_index)(HostFn::RationalFromI64).ok_or(WasmLowerError::Unsupported("rational_from_i64 not imported"))?;
            local_get(code, reg as u32);
            code.push(0x10);
            leb_u32(code, from);
        }
        Some(Kind::BigInt) => {
            let from = (ctx.host_index)(HostFn::RationalFromBigint).ok_or(WasmLowerError::Unsupported("rational_from_bigint not imported"))?;
            local_get(code, reg as u32);
            code.push(0x10);
            leb_u32(code, from);
        }
        _ => return Err(WasmLowerError::Unsupported("rational arithmetic operand must be a Rational, Int, or BigInt")),
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as exact BigInt-backed rational arithmetic (linker mode) — `op` is the
/// `logos_rt_rational_*` sink. Operands promote to Rational handles first, so num/den stay exact past i64.
fn lower_rational_binop(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, dst: u16, lhs: u16, rhs: u16, host: HostFn) -> R<Flow> {
    let op = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("rational op not imported"))?;
    push_rational_operand(code, kinds, ctx, lhs)?;
    push_rational_operand(code, kinds, ctx, rhs)?;
    code.push(0x10);
    leb_u32(code, op);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// `dst = <op>(arg)` on a Rational handle (linker mode) — `op` is a unary `logos_rt_rational_*` sink
/// (`floor`/`ceil`/`round` returning a BigInt handle, `abs` a Rational handle).
fn lower_rational_unary(code: &mut Vec<u8>, ctx: &Ctx, dst: u16, arg: u16, host: HostFn) -> R<Flow> {
    let op = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("rational unary op not imported"))?;
    local_get(code, arg as u32);
    code.push(0x10);
    leb_u32(code, op);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// `dst = (lhs == rhs)` (or `!=` when `negate`) on two Uuid handles (linker mode) — `logos_rt_uuid_eq`
/// compares the 16 bytes and returns an i32 0/1, extended to the i64 a `Bool` register holds (matching
/// [`lower_text_eq`]'s i64-boolean convention).
fn lower_uuid_eq(code: &mut Vec<u8>, ctx: &Ctx, dst: u16, lhs: u16, rhs: u16, negate: bool) -> R<Flow> {
    let eq = (ctx.host_index)(HostFn::UuidEq).ok_or(WasmLowerError::Unsupported("uuid_eq not imported"))?;
    local_get(code, lhs as u32);
    local_get(code, rhs as u32);
    code.push(0x10);
    leb_u32(code, eq); // → i32 0/1
    if negate {
        code.push(0x45); // i32.eqz → logical NOT
    }
    code.push(0xAD); // i64.extend_i32_u → the i64 a Bool register holds
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// `dst = base <±> span` calendar arithmetic (linker mode): unpack the `Span` i64 (`months` high word,
/// `days` low word), negate for subtraction, then call `logos_rt_moment_add_span` (Moment base, i64) or
/// `logos_rt_date_add_span` (Date base, i32). `base`/`dst` share the base's width (Moment i64 / Date i32).
fn lower_span_add(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, base: u16, span: u16, is_date: bool, negate: bool) -> R<Flow> {
    let host = if is_date { HostFn::DateAddSpan } else { HostFn::MomentAddSpan };
    let idx = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("span add not imported"))?;
    let (months, days) = (num_regs + 5, num_regs + 6);
    // months = (span >> 32) as i32 (arithmetic — a Span's months can be negative)
    local_get(code, span as u32);
    code.push(0x42);
    leb_i64(code, 32);
    code.push(0x87); // i64.shr_s
    code.push(0xA7); // i32.wrap_i64
    local_set(code, months);
    // days = span as i32 (low word)
    local_get(code, span as u32);
    code.push(0xA7); // i32.wrap_i64
    local_set(code, days);
    if negate {
        for r in [months, days] {
            i32_const(code, 0);
            local_get(code, r);
            code.push(0x6B); // i32.sub → 0 - r
            local_set(code, r);
        }
    }
    // dst = host(base, months, days)
    local_get(code, base as u32);
    local_get(code, months);
    local_get(code, days);
    code.push(0x10);
    leb_u32(code, idx);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

fn lower_pow_regs(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, dst: u16, base: u16, exp: u16) -> R<Flow> {
    let (bk, ek) = (kinds.get(base as usize), kinds.get(exp as usize));
    match (bk, ek) {
        (_, Some(Kind::Float)) => {
            push_as_f64(code, base, bk)?;
            local_get(code, exp as u32); // exp is already f64
            let idx = (ctx.host_index)(HostFn::PowFf).ok_or(WasmLowerError::Unsupported("pow_ff not imported"))?;
            code.push(0x10);
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        (Some(Kind::Float), Some(Kind::Int)) => {
            local_get(code, base as u32);
            local_get(code, exp as u32);
            let idx = (ctx.host_index)(HostFn::PowFi).ok_or(WasmLowerError::Unsupported("pow_fi not imported"))?;
            code.push(0x10);
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        (Some(Kind::Int), Some(Kind::Int)) if ctx.linked => {
            // LINKER MODE: compute the exact big integer via the real `logicaffeine_base::BigInt`
            // runtime — `from_i64(base)` → `pow(handle, exp)` — leaving a BigInt HANDLE in `dst`. No
            // overflow, no trap. The handle stays a handle (rendered to a decimal `Text` only at `Show`,
            // via `lower_show`), so a downstream `*` keeps multiplying on real BigInts.
            let from_i64 = (ctx.host_index)(HostFn::BigintFromI64).ok_or(WasmLowerError::Unsupported("bigint_from_i64 not imported"))?;
            let pow = (ctx.host_index)(HostFn::BigintPow).ok_or(WasmLowerError::Unsupported("bigint_pow not imported"))?;
            local_get(code, base as u32); // i64 base
            code.push(0x10);
            leb_u32(code, from_i64); // -> i32 BigInt handle
            local_get(code, exp as u32); // i64 exponent
            code.push(0x10);
            leb_u32(code, pow); // (handle, exp) -> i32 BigInt handle
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        (Some(Kind::Int), Some(Kind::Int)) => {
            lower_int_pow(code, num_regs, dst, base, exp);
            Ok(Flow::Straight)
        }
        _ => Err(WasmLowerError::Unsupported("pow of non-numbers")),
    }
}

/// `a // b` — floor division (toward negative infinity). `Int`: `i64.div_s` truncates toward
/// zero, so correct by one when the remainder is nonzero AND the operands differ in sign —
/// `q - ((r != 0) & ((r ^ b) < 0))`, borrowing one pow i64 scratch (`num_regs+1`) for `r`.
/// `i64.div_s`/`rem_s` trap on `/0` and `i64::MIN // -1` (the BigInt-promotion frontier,
/// matching `Op::Div`). `Float`: `f64.floor(a / b)`, promoting an Int operand. `Word`: unsigned
/// `div_u` (floor == truncation on non-negative). Mirrors `arith::floor_divide`.
fn lower_floordiv_regs(code: &mut Vec<u8>, kinds: &KindTable, num_regs: u32, dst: u16, lhs: u16, rhs: u16) -> R<Flow> {
    match kinds.get(dst as usize) {
        Some(Kind::Int) => {
            if kinds.valtype(lhs as usize) != I64 || kinds.valtype(rhs as usize) != I64 {
                return Err(WasmLowerError::Unsupported("integer floor division with a non-integer operand"));
            }
            let s_r = num_regs + 1; // borrow a pow i64 scratch to hold the remainder
            // s_r = a % b   (rem_s traps on b == 0)
            local_get(code, lhs as u32);
            local_get(code, rhs as u32);
            code.push(0x81); // i64.rem_s
            local_set(code, s_r);
            // q = a / b     (div_s traps on b == 0 and i64::MIN / -1)
            local_get(code, lhs as u32);
            local_get(code, rhs as u32);
            code.push(0x7F); // i64.div_s
            // corr = (r != 0) & ((r ^ b) < 0)  → 1 when the truncated quotient overshot
            local_get(code, s_r);
            code.push(0x42);
            leb_i64(code, 0); // i64.const 0
            code.push(0x52); // i64.ne → i32
            local_get(code, s_r);
            local_get(code, rhs as u32);
            code.push(0x85); // i64.xor
            code.push(0x42);
            leb_i64(code, 0); // i64.const 0
            code.push(0x53); // i64.lt_s → i32
            code.push(0x71); // i32.and
            code.push(0xAD); // i64.extend_i32_u → i64 corr
            code.push(0x7D); // i64.sub → q - corr
            local_set(code, dst as u32);
        }
        Some(Kind::Word32) => arith(code, 0x6E, dst, lhs, rhs), // i32.div_u
        Some(Kind::Word64) => arith(code, 0x80, dst, lhs, rhs), // i64.div_u
        Some(Kind::Float) => {
            push_as_f64(code, lhs, kinds.get(lhs as usize))?;
            push_as_f64(code, rhs, kinds.get(rhs as usize))?;
            code.push(0xA3); // f64.div
            code.push(0x9C); // f64.floor
            local_set(code, dst as u32);
        }
        _ => return Err(WasmLowerError::Unsupported("floor division on a non-numeric value")),
    }
    Ok(Flow::Straight)
}

/// `base^exp` for two Ints, by exponentiation-by-squaring, using three reserved scratch locals
/// (`num_regs+1..=num_regs+3`). Each multiply is overflow-checked (traps → matches the VM's
/// promote-to-BigInt contract). A negative exponent traps (the VM yields a Float).
fn lower_int_pow(code: &mut Vec<u8>, num_regs: u32, dst: u16, base: u16, exp: u16) {
    let result = (num_regs + 1) as u16;
    let base_s = (num_regs + 2) as u16;
    let exp_s = (num_regs + 3) as u16;
    // A product temp distinct from result/base_s — `emit_checked_mul` writes its dst BEFORE
    // re-reading lhs for the overflow check, so dst must not alias lhs (else the check sees the
    // product and falsely traps).
    let tmp = (num_regs + 4) as u16;
    // if exp < 0: trap
    local_get(code, exp as u32);
    code.push(0x42);
    leb_i64(code, 0);
    code.push(0x53); // i64.lt_s
    code.push(0x04);
    code.push(0x40); // if
    code.push(0x00); // unreachable
    code.push(0x0B); // end
    // result = 1; base_s = base; exp_s = exp
    code.push(0x42);
    leb_i64(code, 1);
    local_set(code, result as u32);
    local_get(code, base as u32);
    local_set(code, base_s as u32);
    local_get(code, exp as u32);
    local_set(code, exp_s as u32);
    // block $exit { loop $loop {
    code.push(0x02);
    code.push(0x40);
    code.push(0x03);
    code.push(0x40);
    //   if exp_s == 0 → br $exit  (while exp_s != 0)
    local_get(code, exp_s as u32);
    code.push(0x50); // i64.eqz
    code.push(0x0D);
    leb_u32(code, 1); // br_if $exit
    //   if (exp_s & 1) != 0: result = result * base_s  (checked)
    local_get(code, exp_s as u32);
    code.push(0x42);
    leb_i64(code, 1);
    code.push(0x83); // i64.and
    code.push(0xA7); // i32.wrap_i64 (low bit as the i32 condition)
    code.push(0x04);
    code.push(0x40); // if
    emit_checked_mul(code, tmp, result, base_s); // tmp = result * base_s (no aliasing)
    local_get(code, tmp as u32);
    local_set(code, result as u32); // result = tmp
    code.push(0x0B); // end if
    //   exp_s >>= 1
    local_get(code, exp_s as u32);
    code.push(0x42);
    leb_i64(code, 1);
    code.push(0x87); // i64.shr_s
    local_set(code, exp_s as u32);
    //   if exp_s != 0: base_s = base_s * base_s  (checked; skip the unused final square)
    local_get(code, exp_s as u32);
    code.push(0x50); // i64.eqz
    code.push(0x45); // i32.eqz → exp_s != 0
    code.push(0x04);
    code.push(0x40); // if
    emit_checked_mul(code, tmp, base_s, base_s); // tmp = base_s * base_s (no aliasing)
    local_get(code, tmp as u32);
    local_set(code, base_s as u32); // base_s = tmp
    code.push(0x0B); // end if
    code.push(0x0C);
    leb_u32(code, 0); // br $loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    // dst = result
    local_get(code, result as u32);
    local_set(code, dst as u32);
}

/// Bump-allocate `size` bytes (the `size` is the i32 already on the stack). Leaves the
/// 8-aligned pointer in `dst_scratch` and advances `__heap_ptr` past it. No free (a finite-run
/// leak; growth-on-push leaks the old buffer).
fn emit_alloc(code: &mut Vec<u8>, ctx: &Ctx, dst_scratch: u32) {
    // stack: [size]
    if let Some(rt_alloc) = ctx.rt_alloc {
        // LINKER MODE: each block comes straight from the runtime allocator (`dlmalloc`, which grows
        // linear memory on demand), so the emitter heap is UNBOUNDED — a program allocating past any fixed
        // slab can't run off the end or collide with the runtime's region.
        code.push(0x10); // call logos_rt_alloc(size)
        leb_u32(code, rt_alloc);
        local_set(code, dst_scratch); // dst_scratch = ptr
    } else {
        // Self-contained: bump the `__heap_ptr` global (8-aligned, no free).
        global_get(code, ctx.heap_global);
        i32_const(code, 7);
        code.push(0x6A); // i32.add
        i32_const(code, -8);
        code.push(0x71); // i32.and → aligned p
        local_tee(code, dst_scratch); // dst_scratch = p; stack [size, p]
        code.push(0x6A); // i32.add → p + size
        global_set(code, ctx.heap_global);
    }
}

/// `Push value to seq` (Int sequence): reallocate the data buffer to `(len+1)` elements, copy
/// the old elements, append `value`, and update the header in place (so the handle is stable).
/// O(n) per push — correctness first; geometric capacity is a later refinement.
fn lower_list_push(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, list: u16, value: u16) -> R<()> {
    let elem = kinds.get(list as usize).and_then(Kind::seq_elem).ok_or(WasmLowerError::Unsupported("push to a sequence of unknown element kind"))?;
    lower_list_push_at(code, elem, ctx, num_regs, list as u32, value)
}

/// The core of [`lower_list_push`], parameterized by the element kind and the LOCAL holding the seq
/// handle (rather than deriving them from a register) — so a struct FIELD seq (`ListPushField`, whose
/// handle lives in a struct slot, kind in the struct layout) can push through the same amortized path.
fn lower_list_push_at(code: &mut Vec<u8>, elem: Kind, ctx: &Ctx, num_regs: u32, lst: u32, value: u16) -> R<()> {
    // Int/Float/Text(handle) elements all occupy an 8-byte slot — only the load/store opcode differs.
    let elem_load = seq_elem_load(elem)?;
    let elem_store = seq_elem_store(elem)?;
    let (hs_new, hs_i, hs_cap) = (num_regs + 5, num_regs + 6, num_regs + 7);
    // AMORTIZED growth. The header tracks `cap` (allocated element slots) alongside `len`; both
    // NewEmptyList (cap 0) and NewList (cap = count) set it to the true buffer size, so this stays
    // sound. When a slot is free we write in place (O(1)); otherwise we double the capacity, copy,
    // and repoint `data_ptr` — total work O(n) over n pushes, not the O(n²) a copy-every-push does
    // (which exhausts a small linear memory on a build-then-scan array like counting_sort's counts).
    //
    // if len < cap { in place } else { grow }
    local_get(code, lst);
    i32_load(code, 0); // len
    local_get(code, lst);
    i32_load(code, 4); // cap
    code.push(0x48); // i32.lt_s → len < cap
    code.push(0x04);
    code.push(0x40); // if (void)
    {
        // data_ptr + len*8 = value
        local_get(code, lst);
        i32_load(code, 8); // data_ptr
        local_get(code, lst);
        i32_load(code, 0); // len
        i32_const(code, 8);
        code.push(0x6C);
        code.push(0x6A); // data_ptr + len*8
        local_get(code, value as u32);
        elem_store(code, 0);
    }
    code.push(0x05); // else
    {
        // new_cap = cap == 0 ? 4 : cap * 2
        i32_const(code, 4);
        local_get(code, lst);
        i32_load(code, 4);
        i32_const(code, 2);
        code.push(0x6C); // cap * 2
        local_get(code, lst);
        i32_load(code, 4);
        code.push(0x45); // i32.eqz → cap == 0
        code.push(0x1B); // select → (cap==0) ? 4 : cap*2
        local_set(code, hs_cap);
        // new = alloc(new_cap * 8)
        local_get(code, hs_cap);
        i32_const(code, 8);
        code.push(0x6C);
        emit_alloc(code, ctx,hs_new);
        // for i in 0..len: new[i] = old[i]
        i32_const(code, 0);
        local_set(code, hs_i);
        code.push(0x02);
        code.push(0x40); // block $exit
        code.push(0x03);
        code.push(0x40); // loop $loop
        local_get(code, hs_i);
        local_get(code, lst);
        i32_load(code, 0); // len
        code.push(0x4E); // i32.ge_s → i >= len
        code.push(0x0D);
        leb_u32(code, 1); // br_if $exit
        local_get(code, hs_new);
        local_get(code, hs_i);
        i32_const(code, 8);
        code.push(0x6C);
        code.push(0x6A); // new + i*8
        local_get(code, lst);
        i32_load(code, 8); // old data_ptr
        local_get(code, hs_i);
        i32_const(code, 8);
        code.push(0x6C);
        code.push(0x6A); // old_data + i*8
        elem_load(code, 0);
        elem_store(code, 0); // new[i] = old[i]
        local_get(code, hs_i);
        i32_const(code, 1);
        code.push(0x6A);
        local_set(code, hs_i);
        code.push(0x0C);
        leb_u32(code, 0); // br $loop
        code.push(0x0B); // end loop
        code.push(0x0B); // end block
        // new[len] = value
        local_get(code, hs_new);
        local_get(code, lst);
        i32_load(code, 0); // len
        i32_const(code, 8);
        code.push(0x6C);
        code.push(0x6A);
        local_get(code, value as u32);
        elem_store(code, 0);
        // header: data_ptr = new
        local_get(code, lst);
        local_get(code, hs_new);
        i32_store(code, 8);
        // header: cap = new_cap
        local_get(code, lst);
        local_get(code, hs_cap);
        i32_store(code, 4);
    }
    code.push(0x0B); // end if
    // header: len = len + 1
    local_get(code, lst);
    local_get(code, lst);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6A);
    i32_store(code, 0);
    Ok(())
}

/// `Pop from list into dst` (`ListPop`) — remove and return the last element. Mirror of
/// `list_pop`: load `data_ptr[(len-1)]` at the element width, then decrement the header `len`
/// (leaving `cap`/`data_ptr` intact — the slot is simply no longer live, exactly as `Vec::pop`
/// shrinks length without freeing). Popping an empty list has no scalar `Nothing` representation,
/// so the guarded `else` yields a typed zero; the tree-walker's `Nothing` only arises from an
/// over-pop the corpus never performs, and the guard keeps the load in bounds regardless.
fn lower_list_pop(code: &mut Vec<u8>, kinds: &KindTable, dst: u16, list: u16) -> R<()> {
    let elem = kinds.get(list as usize).and_then(Kind::seq_elem).ok_or(WasmLowerError::Unsupported("pop from a sequence of unknown element kind"))?;
    let elem_load = seq_elem_load(elem)?;
    let vt = elem.wasm_valtype();
    let lst = list as u32;
    // if len > 0 { dst = data_ptr[(len-1)*8]; len -= 1 } else { dst = <typed zero> }
    local_get(code, lst);
    i32_load(code, 0); // len
    i32_const(code, 0);
    code.push(0x4A); // i32.gt_s → len > 0
    code.push(0x04);
    code.push(vt); // if (result <element valtype>)
    {
        // data_ptr + (len-1)*8 → load the last element (left on the stack as the `if` result)
        local_get(code, lst);
        i32_load(code, 8); // data_ptr
        local_get(code, lst);
        i32_load(code, 0); // len
        i32_const(code, 1);
        code.push(0x6B); // i32.sub → len-1
        i32_const(code, 8);
        code.push(0x6C); // *8
        code.push(0x6A); // data_ptr + (len-1)*8
        elem_load(code, 0);
        // header: len = len - 1
        local_get(code, lst);
        local_get(code, lst);
        i32_load(code, 0);
        i32_const(code, 1);
        code.push(0x6B); // i32.sub
        i32_store(code, 0);
    }
    code.push(0x05); // else
    match vt {
        F64 => {
            code.push(0x44);
            code.extend_from_slice(&0.0f64.to_le_bytes());
        }
        I64 => {
            code.push(0x42);
            leb_i64(code, 0);
        }
        _ => i32_const(code, 0),
    }
    code.push(0x0B); // end if
    local_set(code, dst as u32);
    Ok(())
}

/// Bounds-check a 1-based `index` into the Int sequence `collection` (trap on `index < 1` or
/// `index > len`, as the standalone module has no VM to surface the error), then leave the
/// element address `data_ptr + (index-1)*8` on the stack.
fn emit_seq_elem_addr(code: &mut Vec<u8>, kinds: &KindTable, collection: u16, index: u16) -> R<()> {
    // Every element (scalar OR handle) occupies an 8-byte slot, so the address arithmetic is the
    // same — only the caller's load/store width differs. A heterogeneous `Tuple` has the identical
    // header+slot layout (the caller picks the width from the static position kind).
    match kinds.get(collection as usize) {
        Some(Kind::Tuple) => {}
        other if other.and_then(Kind::seq_elem).is_some() => {}
        _ => return Err(WasmLowerError::Unsupported("index of a non-scalar sequence")),
    }
    let col = collection as u32;
    // trap if index < 1 || index > len
    local_get(code, index as u32);
    code.push(0x42);
    leb_i64(code, 1);
    code.push(0x53); // i64.lt_s
    local_get(code, index as u32);
    local_get(code, col);
    i32_load(code, 0);
    code.push(0xAD); // len → i64
    code.push(0x55); // i64.gt_s
    code.push(0x72); // i32.or
    code.push(0x04);
    code.push(0x40); // if
    code.push(0x00); // unreachable
    code.push(0x0B); // end
    // address = data_ptr + (index-1)*8
    local_get(code, col);
    i32_load(code, 8); // data_ptr
    local_get(code, index as u32);
    code.push(0x42);
    leb_i64(code, 1);
    code.push(0x7D); // i64.sub
    code.push(0xA7); // i32.wrap_i64
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    code.push(0x6A); // i32.add
    Ok(())
}

/// The element kind of the sequence in `collection` (Int or Float), for load/store width.
fn seq_elem_kind(kinds: &KindTable, collection: u16) -> R<Kind> {
    kinds.get(collection as usize).and_then(Kind::seq_elem).ok_or(WasmLowerError::Unsupported("sequence of unknown element kind"))
}

/// The load opcode for a sequence element of `elem` kind: `Float` → `f64`, `Int`/`Bool`/`Moment` →
/// `i64`, a heap kind (`Text`/`Struct`/…) → `i32` (the handle in the slot's low word). Each slot is
/// 8 bytes regardless (so the `i64`-stride `emit_seq_copy`/`emit_seq_elem_addr` are kind-agnostic).
fn seq_elem_load(elem: Kind) -> R<fn(&mut Vec<u8>, u32)> {
    Ok(match elem.wasm_valtype() {
        F64 => f64_load,
        I64 => i64_load,
        _ => i32_load,
    })
}

/// The store opcode mirroring [`seq_elem_load`].
fn seq_elem_store(elem: Kind) -> R<fn(&mut Vec<u8>, u32)> {
    Ok(match elem.wasm_valtype() {
        F64 => f64_store,
        I64 => i64_store,
        _ => i32_store,
    })
}

/// `item index of seq` / `item N of tuple` — load the (bounds-checked) element at its kind's width.
/// For a heterogeneous `Tuple` the width is the constant position's element kind, which is the
/// inferred kind of `dst` (resolved via the `tuple_value` track); otherwise it's the seq element.
fn lower_index(code: &mut Vec<u8>, kinds: &KindTable, dst: u16, collection: u16, index: u16) -> R<()> {
    let elem = if kinds.get(collection as usize) == Some(Kind::Tuple) {
        kinds.get(dst as usize).ok_or(WasmLowerError::Unsupported("tuple element of unknown kind"))?
    } else {
        seq_elem_kind(kinds, collection)?
    };
    let load = seq_elem_load(elem)?;
    emit_seq_elem_addr(code, kinds, collection, index)?;
    load(code, 0);
    local_set(code, dst as u32);
    Ok(())
}

/// `item index of text` — a one-character `Text` cut from `text` at the (1-based, bounds-checked)
/// BYTE position, matching the VM's ASCII fast path (`item i of "abc"` → `RuntimeValue::Text` of one
/// byte). A fresh 16-byte header + 1-byte data buffer holds the extracted byte; the result compares
/// to a literal (`item i of text equals " "`) through the existing `Text` byte-equality path. (A
/// multi-byte UTF-8 string would need a char decode to match the VM's general path — ASCII only here,
/// which is every string the corpus indexes.)
fn lower_text_index(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, collection: u16, index: u16) -> R<()> {
    let (hdr, data) = (num_regs + 5, num_regs + 6);
    let col = collection as u32;
    // trap if index < 1 || index > byte_len
    local_get(code, index as u32);
    code.push(0x42);
    leb_i64(code, 1);
    code.push(0x53); // i64.lt_s → index < 1
    local_get(code, index as u32);
    local_get(code, col);
    i32_load(code, 0); // byte len
    code.push(0xAD); // i64.extend_i32_s
    code.push(0x55); // i64.gt_s → index > len
    code.push(0x72); // i32.or
    code.push(0x04);
    code.push(0x40); // if
    code.push(0x00); // unreachable
    code.push(0x0B); // end
    // hdr = alloc(16); data = alloc(1)
    i32_const(code, 16);
    emit_alloc(code, ctx,hdr);
    i32_const(code, 1);
    emit_alloc(code, ctx,data);
    // data[0] = byte at (text.data_ptr + (index - 1))
    local_get(code, data); // store8 destination
    local_get(code, col);
    i32_load(code, 8); // text data_ptr
    local_get(code, index as u32);
    code.push(0x42);
    leb_i64(code, 1);
    code.push(0x7D); // i64.sub → index - 1
    code.push(0xA7); // i32.wrap_i64
    code.push(0x6A); // text data_ptr + (index - 1)
    i32_load8_u(code, 0);
    i32_store8(code, 0);
    // header: len = 1, cap = 1, data_ptr = data
    local_get(code, hdr);
    i32_const(code, 1);
    i32_store(code, 0);
    local_get(code, hdr);
    i32_const(code, 1);
    i32_store(code, 4);
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8);
    local_get(code, hdr);
    local_set(code, dst as u32);
    Ok(())
}

/// Build a fresh `Seq of Int` from `len_reg` raw bytes at `base_reg` (each byte → one i64 element), the
/// emitter-heap `[len][cap][data_ptr]` seq with an i64 element buffer. Shared by `text_bytes` (a Text's
/// data_ptr) and `uuid_bytes` (the 16 bytes at a Uuid handle). Leaves the seq handle in `dst`.
fn emit_bytes_to_seq(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, base_reg: u32, len_reg: u32) {
    let (hdr, data, i) = (num_regs + 7, num_regs + 8, num_regs + 9);
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr);
    // data = alloc(len * 8) — one i64 element per byte.
    local_get(code, len_reg);
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    emit_alloc(code, ctx, data);
    // header: len = cap = len_reg, data_ptr = data
    local_get(code, hdr);
    local_get(code, len_reg);
    i32_store(code, 0);
    local_get(code, hdr);
    local_get(code, len_reg);
    i32_store(code, 4);
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8);
    // for i in 0..len { data[i] = (i64) base[i] }
    i32_const(code, 0);
    local_set(code, i);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    local_get(code, i);
    local_get(code, len_reg);
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if block (done)
    // dst addr = data + i*8
    local_get(code, data);
    local_get(code, i);
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    code.push(0x6A); // i32.add
    // value = base[i] zero-extended to i64 (a byte is 0..255)
    local_get(code, base_reg);
    local_get(code, i);
    code.push(0x6A); // i32.add
    i32_load8_u(code, 0);
    code.push(0xAD); // i64.extend_i32_u
    i64_store(code, 0);
    local_get(code, i);
    i32_const(code, 1);
    code.push(0x6A); // i32.add
    local_set(code, i);
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    local_get(code, hdr);
    local_set(code, dst as u32);
}

/// `text_bytes(text)` — the Text's UTF-8 bytes as a `Seq of Int`.
fn lower_text_bytes(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, text: u16) {
    let (base, len) = (num_regs + 5, num_regs + 6);
    local_get(code, text as u32);
    i32_load(code, 8); // text data_ptr
    local_set(code, base);
    local_get(code, text as u32);
    i32_load(code, 0); // text byte len
    local_set(code, len);
    emit_bytes_to_seq(code, ctx, num_regs, dst, base, len);
}

/// `uuid_bytes(u)` — the 16 raw bytes of a Uuid (the handle is a `Box<[u8; 16]>`, bytes at `*handle`).
fn lower_uuid_bytes(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, uuid: u16) {
    let (base, len) = (num_regs + 5, num_regs + 6);
    local_get(code, uuid as u32);
    local_set(code, base);
    i32_const(code, 16);
    local_set(code, len);
    emit_bytes_to_seq(code, ctx, num_regs, dst, base, len);
}

/// `text_from_bytes(seq)` — a `Text` from the low byte of each `Seq of Int` element.
fn lower_text_from_bytes(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, seq: u16) {
    let (hdr, data, i, len, src) = (num_regs + 7, num_regs + 8, num_regs + 9, num_regs + 6, num_regs + 5);
    local_get(code, seq as u32);
    i32_load(code, 0); // element count
    local_set(code, len);
    local_get(code, seq as u32);
    i32_load(code, 8); // seq data_ptr
    local_set(code, src);
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr);
    local_get(code, len); // data = alloc(len) — one byte each
    emit_alloc(code, ctx, data);
    local_get(code, hdr);
    local_get(code, len);
    i32_store(code, 0);
    local_get(code, hdr);
    local_get(code, len);
    i32_store(code, 4);
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8);
    // for i in 0..len { data[i] = low byte of seq[i] (= src[i*8], little-endian) }
    i32_const(code, 0);
    local_set(code, i);
    code.push(0x02);
    code.push(0x40);
    code.push(0x03);
    code.push(0x40);
    local_get(code, i);
    local_get(code, len);
    code.push(0x4E);
    code.push(0x0D);
    leb_u32(code, 1);
    local_get(code, data);
    local_get(code, i);
    code.push(0x6A); // data + i
    local_get(code, src);
    local_get(code, i);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A); // src + i*8
    i32_load8_u(code, 0); // low byte of the i64 element
    i32_store8(code, 0);
    local_get(code, i);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, i);
    code.push(0x0C);
    leb_u32(code, 0);
    code.push(0x0B);
    code.push(0x0B);
    local_get(code, hdr);
    local_set(code, dst as u32);
}

/// `uuid_from_bytes(seq)` — pack the low byte of the first 16 `Seq of Int` elements into a contiguous
/// 16-byte block, then hand it to `logos_rt_uuid_from_ptr` to box a `base::Uuid` (LINKER MODE only).
fn lower_uuid_from_bytes(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, seq: u16) -> R<Flow> {
    let from_ptr = (ctx.host_index)(HostFn::UuidFromPtr).ok_or(WasmLowerError::Unsupported("uuid_from_ptr not imported"))?;
    let (block, i, src) = (num_regs + 7, num_regs + 9, num_regs + 5);
    local_get(code, seq as u32);
    i32_load(code, 8); // seq data_ptr
    local_set(code, src);
    i32_const(code, 16);
    emit_alloc(code, ctx, block);
    // for i in 0..16 { block[i] = low byte of seq[i] }
    i32_const(code, 0);
    local_set(code, i);
    code.push(0x02);
    code.push(0x40);
    code.push(0x03);
    code.push(0x40);
    local_get(code, i);
    i32_const(code, 16);
    code.push(0x4E);
    code.push(0x0D);
    leb_u32(code, 1);
    local_get(code, block);
    local_get(code, i);
    code.push(0x6A);
    local_get(code, src);
    local_get(code, i);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    i32_load8_u(code, 0);
    i32_store8(code, 0);
    local_get(code, i);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, i);
    code.push(0x0C);
    leb_u32(code, 0);
    code.push(0x0B);
    code.push(0x0B);
    // dst = logos_rt_uuid_from_ptr(block)
    local_get(code, block);
    code.push(0x10);
    leb_u32(code, from_ptr);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// `lanes4Of(a, b, c, d)` — pack four `Word32` (i32) into a fresh 16-byte `[u32; 4]` lane block, lane
/// `i` at byte `i*4` (the `base::Lanes4Word32` layout the SHA-1 runtime reads).
fn lower_lanes4_of(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, words: [u16; 4]) {
    let block = num_regs + 7;
    i32_const(code, 16);
    emit_alloc(code, ctx, block);
    for (i, w) in words.iter().enumerate() {
        local_get(code, block);
        local_get(code, *w as u32);
        i32_store(code, (i * 4) as u32);
    }
    local_get(code, block);
    local_set(code, dst as u32);
}

/// `lanes4Word32(seq)` — pack the first four `Word32` elements of a `Seq of Word32` (each an i32 in the
/// low word of its 8-byte slot) into a lane block.
fn lower_lanes4_word32(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, seq: u16) {
    let block = num_regs + 7;
    i32_const(code, 16);
    emit_alloc(code, ctx, block);
    for i in 0..4u32 {
        local_get(code, block);
        // value = seq.data_ptr[i] (i32 at data_ptr + i*8)
        local_get(code, seq as u32);
        i32_load(code, 8);
        i32_const(code, (i * 8) as i32);
        code.push(0x6A); // i32.add
        i32_load(code, 0);
        i32_store(code, i * 4);
    }
    local_get(code, block);
    local_set(code, dst as u32);
}

/// `seqOfLanes4W32(lanes)` — unpack a lane block back into a fresh `Seq of Word32` (4 elements, each the
/// lane's i32 in the low word of an 8-byte slot).
fn lower_seq_of_lanes4(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, lanes: u16) {
    let (hdr, data) = (num_regs + 7, num_regs + 8);
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr);
    i32_const(code, 32); // 4 elements × 8-byte slots
    emit_alloc(code, ctx, data);
    local_get(code, hdr);
    i32_const(code, 4);
    i32_store(code, 0);
    local_get(code, hdr);
    i32_const(code, 4);
    i32_store(code, 4);
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8);
    for i in 0..4u32 {
        local_get(code, data);
        local_get(code, lanes as u32);
        i32_load(code, i * 4); // lane i
        i32_store(code, i * 8); // element i slot
    }
    local_get(code, hdr);
    local_set(code, dst as u32);
}

/// The four SHA-1 SHA-NI ops — direct `logos_rt_sha1*` calls over lane-block handles. `Sha1Rnds4` also
/// takes the round-function selector (an `Int`, i64); the others are binary.
fn lower_sha1_op(code: &mut Vec<u8>, ctx: &Ctx, dst: u16, args_start: u16, host: HostFn, ternary: bool) -> R<Flow> {
    let idx = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("sha1 op not imported"))?;
    local_get(code, args_start as u32);
    local_get(code, (args_start + 1) as u32);
    if ternary {
        local_get(code, (args_start + 2) as u32); // func selector (i64)
    }
    code.push(0x10);
    leb_u32(code, idx);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// `Set item index of seq to value` — store into the (bounds-checked) element.
fn lower_set_index(code: &mut Vec<u8>, kinds: &KindTable, collection: u16, index: u16, value: u16) -> R<()> {
    let elem = seq_elem_kind(kinds, collection)?;
    let store = seq_elem_store(elem)?;
    emit_seq_elem_addr(code, kinds, collection, index)?; // [addr]
    local_get(code, value as u32); // [addr, value]
    store(code, 0);
    Ok(())
}

/// `start to end` (inclusive Int range): bump-allocate a header + a `count`-element data buffer
/// and fill it with `start, start+1, …, end` (empty when `end < start`).
fn lower_new_range(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, start: u16, end: u16) {
    let (hdr, data, idx) = (num_regs + 5, num_regs + 6, num_regs + 7); // i32 scratch
    let cnt = num_regs + 1; // reuse an i64 (pow) scratch
    // cnt = max(0, end - start + 1)
    local_get(code, end as u32);
    local_get(code, start as u32);
    code.push(0x7D); // i64.sub
    code.push(0x42);
    leb_i64(code, 1);
    code.push(0x7C); // i64.add → end-start+1
    local_set(code, cnt);
    local_get(code, cnt);
    code.push(0x42);
    leb_i64(code, 0);
    code.push(0x53); // i64.lt_s
    code.push(0x04);
    code.push(0x40); // if (cnt < 0)
    code.push(0x42);
    leb_i64(code, 0);
    local_set(code, cnt); // cnt = 0
    code.push(0x0B);
    // header = alloc(16); data = alloc(cnt*8)
    i32_const(code, 16);
    emit_alloc(code, ctx,hdr);
    local_get(code, cnt);
    code.push(0xA7); // i32.wrap_i64
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    emit_alloc(code, ctx,data);
    // header: len, cap = cnt; data_ptr = data
    local_get(code, hdr);
    local_get(code, cnt);
    code.push(0xA7);
    i32_store(code, 0);
    local_get(code, hdr);
    local_get(code, cnt);
    code.push(0xA7);
    i32_store(code, 4);
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8);
    // for i in 0..cnt: data[i] = start + i
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    local_get(code, idx);
    local_get(code, cnt);
    code.push(0xA7);
    code.push(0x4E); // i32.ge_s → i >= cnt
    code.push(0x0D);
    leb_u32(code, 1); // br_if exit
    local_get(code, data);
    local_get(code, idx);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A); // data + i*8
    local_get(code, start as u32);
    local_get(code, idx);
    code.push(0xAC); // i64.extend_i32_s
    code.push(0x7C); // i64.add → start + i
    i64_store(code, 0);
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx); // i++
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B);
    code.push(0x0B); // end loop, end block
    local_get(code, hdr);
    local_set(code, dst as u32);
}

/// `repeatSeq(x, n)` — a fresh `n`-element sequence, each slot a copy of the SCALAR `x` (the WASM
/// mirror of `[x] * n` / `n copies of x`). Bump-allocate a header + `n*8` data buffer and fill each
/// 8-byte slot with `x` in a runtime loop (`n < 0` → empty). Scalar element kinds only — a reference
/// element (whose per-slot copy must be an INDEPENDENT deep copy) defers to the VM.
fn lower_repeat_seq(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, dst: u16, args_start: u16) -> R<()> {
    let value = args_start; // the element x
    let n = args_start + 1; // the count (i64)
    let is_float = matches!(kinds.get(value as usize), Some(Kind::Float));
    if !matches!(kinds.get(value as usize), Some(Kind::Int) | Some(Kind::Float)) {
        return Err(WasmLowerError::Unsupported("repeatSeq of a non-scalar element (deep-copy)"));
    }
    let (hdr, data, idx) = (num_regs + 5, num_regs + 6, num_regs + 7); // i32 scratch
    let cnt = num_regs + 1; // reuse an i64 (pow) scratch
    // cnt = max(0, n)
    local_get(code, n as u32);
    local_set(code, cnt);
    local_get(code, cnt);
    code.push(0x42);
    leb_i64(code, 0);
    code.push(0x53); // i64.lt_s
    code.push(0x04);
    code.push(0x40); // if (cnt < 0)
    code.push(0x42);
    leb_i64(code, 0);
    local_set(code, cnt); // cnt = 0
    code.push(0x0B);
    // header = alloc(16); data = alloc(cnt*8)
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr);
    local_get(code, cnt);
    code.push(0xA7); // i32.wrap_i64
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    emit_alloc(code, ctx, data);
    // header: len = cap = cnt; data_ptr = data
    local_get(code, hdr);
    local_get(code, cnt);
    code.push(0xA7);
    i32_store(code, 0);
    local_get(code, hdr);
    local_get(code, cnt);
    code.push(0xA7);
    i32_store(code, 4);
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8);
    // for i in 0..cnt: data[i] = value
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    local_get(code, idx);
    local_get(code, cnt);
    code.push(0xA7);
    code.push(0x4E); // i32.ge_s → i >= cnt
    code.push(0x0D);
    leb_u32(code, 1); // br_if exit
    local_get(code, data);
    local_get(code, idx);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A); // data + i*8
    local_get(code, value as u32);
    if is_float {
        f64_store(code, 0);
    } else {
        i64_store(code, 0);
    }
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx); // i++
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B);
    code.push(0x0B); // end loop, end block
    local_get(code, hdr);
    local_set(code, dst as u32);
    Ok(())
}

/// `[e0, e1, …]` (list / homogeneous tuple literal): bump-allocate a header + a `count`-element data
/// buffer and store the registers `start..start+count` (unrolled, no loop — `count` is a compile
/// constant). Elements are Int, Float, or Text(handle); mixed-kind elements are rejected.
fn lower_new_list(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, dst: u16, start: u16, count: u16) -> R<()> {
    let want = if count > 0 {
        kinds.get(start as usize).ok_or(WasmLowerError::Unsupported("list literal of unknown element kind"))?
    } else {
        Kind::Int
    };
    let elem_store = seq_elem_store(want)?;
    for j in 0..count {
        if kinds.get((start + j) as usize) != Some(want) {
            return Err(WasmLowerError::Unsupported("list literal with mixed element kinds"));
        }
    }
    let (hdr, data) = (num_regs + 5, num_regs + 6);
    i32_const(code, 16);
    emit_alloc(code, ctx,hdr);
    i32_const(code, i32::from(count) * 8);
    emit_alloc(code, ctx,data);
    // header: len = cap = count; data_ptr = data
    local_get(code, hdr);
    i32_const(code, i32::from(count));
    i32_store(code, 0);
    local_get(code, hdr);
    i32_const(code, i32::from(count));
    i32_store(code, 4);
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8);
    // data[j] = R[start+j]  (the store's offset field carries j*8)
    for j in 0..count {
        local_get(code, data);
        local_get(code, (start + j) as u32);
        elem_store(code, u32::from(j) * 8);
    }
    local_get(code, hdr);
    local_set(code, dst as u32);
    Ok(())
}

/// `Let (a, b, …) be t` (`DestructureTuple`) — bind each destructured register `start + i` to tuple
/// slot `i`, loaded at that target's own width (`emit_slot_load` by the register's kind). Both a
/// heterogeneous tuple and a homogeneous one (which rides a `SeqX`) lay their elements out at 8-byte
/// slots, so `data_ptr + i*8` is the slot address in both.
fn lower_destructure_tuple(code: &mut Vec<u8>, kinds: &KindTable, src: u16, start: u16, count: u16) -> R<()> {
    for i in 0..count {
        let dst = start + i;
        local_get(code, src as u32);
        i32_load(code, 8); // data_ptr
        emit_slot_load(code, kinds.get(dst as usize), u32::from(i) * 8)?;
        local_set(code, dst as u32);
    }
    Ok(())
}

/// A HETEROGENEOUS tuple `(a, b, …)` — like [`lower_new_list`] but each slot stores its element at
/// that element's OWN width (`emit_slot_store` by the register's kind); `item N of t` reads it back
/// at the matching width. Header `[len][cap][data_ptr]`, `len = cap = count`.
fn lower_new_tuple_het(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, dst: u16, start: u16, count: u16) -> R<()> {
    let (hdr, data) = (num_regs + 5, num_regs + 6);
    i32_const(code, 16);
    emit_alloc(code, ctx,hdr);
    i32_const(code, i32::from(count) * 8);
    emit_alloc(code, ctx,data);
    for (off, v) in [(0u32, i32::from(count)), (4, i32::from(count))] {
        local_get(code, hdr);
        i32_const(code, v);
        i32_store(code, off);
    }
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8);
    for j in 0..count {
        local_get(code, data);
        local_get(code, (start + j) as u32);
        emit_slot_store(code, kinds.get((start + j) as usize), u32::from(j) * 8)?;
    }
    local_get(code, hdr);
    local_set(code, dst as u32);
    Ok(())
}

/// An enum constructor (`NewInductive`) — allocate a `8*(1+count)`-byte object whose first word is
/// the TAG (the constructor name's constant index) and whose following 8-byte slots hold the
/// `count` argument payloads (`args_start..args_start+count`), each stored at the width of its kind.
/// `BindArm` reads slot `index` back at offset `8*(1+index)`. A nullary constructor (`count == 0`)
/// is just the tag — the layout `TestArm` already reads at offset 0.
fn lower_new_inductive(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, dst: u16, ctor: u32, args_start: u16, count: u16) -> R<()> {
    let hs = num_regs + 5;
    i32_const(code, 8 * (1 + i32::from(count)));
    emit_alloc(code, ctx,hs);
    local_get(code, hs);
    i32_const(code, ctor as i32); // tag = constructor name's constant index
    i32_store(code, 0);
    for k in 0..count {
        let arg = args_start + k;
        local_get(code, hs);
        local_get(code, arg as u32);
        emit_slot_store(code, kinds.get(arg as usize), 8 * (1 + u32::from(k)))?;
    }
    local_get(code, hs);
    local_set(code, dst as u32);
    Ok(())
}

/// `If it is a Circle (radius: r)` (`BindArm`) — load the matched value's `index`-th payload slot
/// (offset `8*(1+index)`) into `dst`, at the width of `dst`'s inferred kind. Only reached on the
/// matching variant (the compiler gates each arm's binds behind its `TestArm`/`JumpIfFalse`), so
/// the slot always holds that constructor's argument.
fn lower_bind_arm(code: &mut Vec<u8>, kinds: &KindTable, dst: u16, target: u16, index: u16) -> R<()> {
    if kinds.get(target as usize) != Some(Kind::Enum) {
        return Err(WasmLowerError::Unsupported("payload binding on a non-enum target"));
    }
    // A `When V (binds)` arm for a variant the target is NOT runs dead — the preceding `TestArm` +
    // `JumpIfFalse` skip it — and may bind an index past the actual variant's arity, so the bound
    // payload has no kind. The load never executes; default it to the `i64` the `None`-kind local is
    // declared as (`valtype`) so the slot load is merely valtype-consistent, not a refusal. (A live
    // bind always resolves a concrete kind from the construction site, so this only hits dead arms —
    // register splitting can isolate such a dead bind whose kind register-reuse formerly supplied.)
    let kind = kinds.get(dst as usize).or(Some(Kind::Int));
    local_get(code, target as u32);
    emit_slot_load(code, kind, 8 * (1 + u32::from(index)))?;
    local_set(code, dst as u32);
    Ok(())
}

/// `(x) -> …` (`MakeClosure`) — bump-allocate the closure object `[func_idx:i32][value_k:i64 ×
/// cap_n][flag_k:i64 × cap_n]` and hand back its handle. `func_idx` (word 0) is the table slot
/// `CallValue` `call_indirect`s. Each capture's VALUE is snapshotted now — a global capture from
/// `GlobalGet`, a local one from its moved-into register `locals_start + local_k` — and its
/// present-FLAG is set to 1 (a wasm global is always initialized, so always "captured"). The body
/// reads value/flag back as trailing parameters (see [`plan_function`]).
/// The wasm valtype of function `func`'s capture `k` — a captured GLOBAL's kind (so a composite
/// handle stores/loads as `i32`), else `I64` (a captured local, or an unknown kind). `MakeClosure`'s
/// store, `CallValue`'s load, and the closure body's seeded signature all read it, so they agree.
fn capture_valtype(ctx: &Ctx, func: u16, k: usize) -> u8 {
    ctx.capture_kinds
        .get(func as usize)
        .and_then(|v| v.get(k))
        .copied()
        .flatten()
        .map(Kind::wasm_valtype)
        .unwrap_or(I64)
}

fn lower_make_closure(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, func: u16, locals_start: u16) -> R<()> {
    let caps = ctx
        .functions
        .get(func as usize)
        .map(|f| f.captures.as_slice())
        .ok_or(WasmLowerError::Unsupported("closure of unknown function"))?;
    let cap_n = caps.len() as u32;
    let hs = num_regs + 5;
    i32_const(code, 8 * (1 + 2 * cap_n) as i32);
    emit_alloc(code, ctx,hs);
    local_get(code, hs);
    i32_const(code, i32::from(func)); // function index = table slot
    i32_store(code, 0);
    let mut local_k: u16 = 0;
    for (k, (_sym, global)) in caps.iter().enumerate() {
        // value_k @ 8 + 8k — stored at the capture's own kind (a captured global may be a composite
        // handle, i32) so it round-trips through the closure object losslessly.
        local_get(code, hs);
        match global {
            Some(gidx) => global_get(code, u32::from(*gidx)),
            None => {
                local_get(code, (locals_start + local_k) as u32);
                local_k += 1;
            }
        }
        let off = 8 + 8 * k as u32;
        match capture_valtype(ctx, func, k) {
            I32 => i32_store(code, off),
            F64 => f64_store(code, off),
            _ => i64_store(code, off),
        }
        // flag_k @ 8 + 8*cap_n + 8k = 1 (present)
        local_get(code, hs);
        code.push(0x42); // i64.const 1
        leb_i64(code, 1);
        i64_store(code, 8 + 8 * cap_n + 8 * k as u32);
    }
    local_get(code, hs);
    local_set(code, dst as u32);
    Ok(())
}

/// `f(args)` (`CallValue`) — push the arguments, then (for a capturing closure) each capture's
/// stored value and present-flag, then the closure's function index (the table slot), and
/// `call_indirect` through the module's function table. The push order matches the callee body's
/// parameter layout `[args][values][flags]`. The static signature is the callee body's own type
/// (resolved via the closure's statically-traced construction site); a value result binds to `dst`.
fn lower_call_value(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, pc: usize, dst: u16, callee: u16, args_start: u16, arg_count: u16) -> R<()> {
    let func = plan
        .structs
        .callee_func
        .get(pc)
        .copied()
        .flatten()
        .ok_or(WasmLowerError::Unsupported("indirect call to a closure of unknown origin"))?;
    let type_idx = *ctx
        .fn_type
        .get(func as usize)
        .ok_or(WasmLowerError::Unsupported("closure call: unknown callee type"))?;
    let cap_n = ctx.functions.get(func as usize).map(|f| f.captures.len() as u32).unwrap_or(0);
    for k in 0..arg_count {
        local_get(code, (args_start + k) as u32);
    }
    // capture values (each at its own kind), then present flags, from the closure object
    for k in 0..cap_n {
        local_get(code, callee as u32);
        let off = 8 + 8 * k;
        match capture_valtype(ctx, func, k as usize) {
            I32 => i32_load(code, off),
            F64 => f64_load(code, off),
            _ => i64_load(code, off),
        }
    }
    for k in 0..cap_n {
        local_get(code, callee as u32);
        i64_load(code, 8 + 8 * cap_n + 8 * k);
    }
    // The callee body `func` is statically known here (an unresolvable origin was already refused), so
    // LINKER MODE emits a DIRECT `call` — no function table, no `call_indirect` type, and therefore no
    // element/table sections the reloc transform can't yet relocate. The self-contained path keeps the
    // fully-general `call_indirect` through the module's table.
    if ctx.linked {
        code.push(0x10); // call fn_base+func (direct)
        leb_u32(code, ctx.fn_base + func as u32);
    } else {
        local_get(code, callee as u32);
        i32_load(code, 0); // closure.func_idx = table slot
        code.push(0x11); // call_indirect
        leb_u32(code, type_idx);
        leb_u32(code, 0); // table 0
    }
    // Bind the result iff the callee returns one (else the call leaves nothing on the stack).
    if ctx.fn_results.get(func as usize).copied().flatten().is_some() {
        local_set(code, dst as u32);
    }
    Ok(())
}

/// `it is Variant` (`TestArm`) — compare the target's tag (constructor constant index) against
/// `variant` (the same constant index for that name, by dedup) → Bool. An `i32.eq` on tags is the
/// VM's constructor-name string compare.
fn lower_test_arm(code: &mut Vec<u8>, dst: u16, target: u16, variant: u32) {
    local_get(code, target as u32);
    i32_load(code, 0); // tag
    i32_const(code, variant as i32);
    code.push(0x46); // i32.eq
    code.push(0xAD); // i64.extend_i32_u → Bool
    local_set(code, dst as u32);
}

/// Bump-allocate a zeroed 16-byte collection header `[len/num=0][cap=0][data_ptr=0]` and store its
/// handle in `dst` — the empty form of a sequence or a map (their element/entry shape differs only
/// at use, not at creation).
fn emit_empty_header(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u32) {
    let hs = num_regs + 5;
    global_get(code, ctx.heap_global);
    i32_const(code, 7);
    code.push(0x6A);
    i32_const(code, -8);
    code.push(0x71);
    local_tee(code, hs);
    i32_const(code, 16);
    code.push(0x6A);
    global_set(code, ctx.heap_global);
    for off in [0u32, 4, 8] {
        local_get(code, hs);
        i32_const(code, 0);
        i32_store(code, off);
    }
    local_get(code, hs);
    local_set(code, dst);
}

/// COPY-ON-WRITE reference counting mirrors the VM's `Rc` so value semantics (`LOGOS_VALUE_SEMANTICS`,
/// now default-on in the tree-walker/VM) holds in native wasm. The 4th header word (offset 12, the
/// spare slot past `[len][cap][data_ptr]`) holds the count of EXTRA references beyond the owner — so 0
/// means "uniquely owned" and needs no initialization (wasm linear memory is zero-initialized and the
/// bump allocator never reuses a slot, so every fresh header's word 12 is already 0). A `Call`
/// argument RETAINS (`++`, gaining the callee's parameter as a second holder); a mutation op checks
/// the count and clones first when it is nonzero, so the write can't be seen through the other holder.
fn emit_retain(code: &mut Vec<u8>, reg: u16) {
    local_get(code, reg as u32);
    local_get(code, reg as u32);
    i32_load(code, 12);
    i32_const(code, 1);
    code.push(0x6A); // extra_refs + 1
    i32_store(code, 12);
}

/// The mutable heap kinds a copy-on-write clone can currently reproduce (everything `lower_deep_clone`
/// handles). A kind outside this set skips the COW guard and mutates in place — sound as long as no
/// aliasing test exercises it (`Map` COW is added in a follow-up).
fn cow_clonable(k: Option<Kind>) -> bool {
    matches!(k, Some(Kind::SeqInt) | Some(Kind::SeqBool) | Some(Kind::SeqFloat) | Some(Kind::SeqAny) | Some(Kind::Set) | Some(Kind::SetText) | Some(Kind::SeqSeqInt) | Some(Kind::Map))
}

/// `cow(reg)` — the copy-on-write guard emitted before a mutation. If the object has any extra
/// reference (header word 12 nonzero) it is replaced by a fresh deep clone (word 12 == 0), so the
/// mutation stays private — exactly as the VM's `maybe_cow` clones an `Rc` whose `strong_count > 1`.
/// A uniquely-owned object mutates in place (the common build-then-scan case pays only a load+branch).
fn emit_cow(code: &mut Vec<u8>, kinds: &KindTable, structs: &kind::StructLayout, ctx: &Ctx, num_regs: u32, reg: u16) -> R<()> {
    if !cow_clonable(kinds.get(reg as usize)) {
        return Ok(());
    }
    local_get(code, reg as u32);
    i32_load(code, 12); // extra_refs
    code.push(0x04);
    code.push(0x40); // if extra_refs != 0
    lower_deep_clone(code, kinds, structs, ctx, num_regs, reg, reg)?;
    code.push(0x0B); // end
    Ok(())
}

/// Byte-equality of the two `Text` handles in locals `a`/`b` → `1`/`0` in local `out` (using local
/// `idx` as a byte-loop counter). The value-local form of [`lower_text_eq`]'s scan, so it can be a
/// per-entry key compare inside a map's element loop.
fn emit_text_eq_to(code: &mut Vec<u8>, a: u32, b: u32, out: u32, idx: u32) {
    i32_const(code, 1);
    local_set(code, out); // assume equal
    local_get(code, a);
    i32_load(code, 0);
    local_get(code, b);
    i32_load(code, 0);
    code.push(0x47); // i32.ne (lengths differ?)
    code.push(0x04);
    code.push(0x40); // if (lengths differ)
    i32_const(code, 0);
    local_set(code, out);
    code.push(0x05); // else: byte-compare
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40);
    code.push(0x03);
    code.push(0x40);
    local_get(code, idx);
    local_get(code, a);
    i32_load(code, 0);
    code.push(0x4E); // i32.ge_s → idx >= len
    code.push(0x0D);
    leb_u32(code, 1); // br_if block (all matched)
    local_get(code, a);
    i32_load(code, 8);
    local_get(code, idx);
    code.push(0x6A);
    i32_load8_u(code, 0);
    local_get(code, b);
    i32_load(code, 8);
    local_get(code, idx);
    code.push(0x6A);
    i32_load8_u(code, 0);
    code.push(0x47); // i32.ne
    code.push(0x04);
    code.push(0x40); // if (byte mismatch)
    i32_const(code, 0);
    local_set(code, out);
    code.push(0x0C);
    leb_u32(code, 2); // br block (not equal)
    code.push(0x0B);
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    code.push(0x0B); // end outer if
}

/// Is a `Map`'s key an `Int` (i64, false) or `Text` (i32 handle, true)? Other key kinds are refused.
fn map_key_text(kinds: &KindTable, key: u16) -> R<bool> {
    match kinds.get(key as usize) {
        Some(Kind::Int) => Ok(false),
        Some(Kind::Text) => Ok(true),
        _ => Err(WasmLowerError::Unsupported("map key must be Int or Text")),
    }
}

/// Leave on the stack `1`/`0` for whether map entry `idx`'s key equals the query register `key` —
/// `i64.eq` for an Int key, or a `Text` byte-equality (via [`emit_text_eq_to`], into scratch
/// +8/+9/+10) for a Text key. `idx_local` is the entry index local.
fn emit_map_key_compare(code: &mut Vec<u8>, num_regs: u32, key_text: bool, m: u32, idx_local: u32, key: u16) {
    if key_text {
        let (entry_key, eq, tidx) = (num_regs + 8, num_regs + 9, num_regs + 10);
        emit_map_entry_addr(code, m, idx_local);
        i32_load(code, 0); // entry key handle
        local_set(code, entry_key);
        emit_text_eq_to(code, key as u32, entry_key, eq, tidx);
        local_get(code, eq); // i32 result
    } else {
        emit_map_entry_addr(code, m, idx_local);
        i64_load(code, 0); // entry key
        local_get(code, key as u32);
        code.push(0x51); // i64.eq
    }
}

/// The store opcode for a map's VALUE slot (offset 8 of a 16-byte entry), by the value's kind: a
/// `Float` is an `f64`, an `Int`/`Bool` an `i64`. Any other kind (a handle-valued map) is deferred.
fn map_value_store(k: Option<Kind>) -> R<fn(&mut Vec<u8>, u32)> {
    match k {
        Some(Kind::Float) => Ok(f64_store),
        Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Moment) | Some(Kind::Duration) | Some(Kind::Time) | Some(Kind::Span) => Ok(i64_store),
        // A handle-valued map (`Map of K to Seq`/Struct/Text/…): the i32 handle rides the low word of
        // the 8-byte value slot; the entry copy in `emit_map_clone` moves the whole slot as an i64.
        Some(vk) if vk.wasm_valtype() == I32 => Ok(i32_store),
        _ => Err(WasmLowerError::Unsupported("map value of an unsupported kind")),
    }
}

/// The load opcode mirroring [`map_value_store`] (the value's kind is the `Index` destination's).
fn map_value_load(k: Option<Kind>) -> R<fn(&mut Vec<u8>, u32)> {
    match k {
        Some(Kind::Float) => Ok(f64_load),
        Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Moment) | Some(Kind::Duration) | Some(Kind::Time) | Some(Kind::Span) => Ok(i64_load),
        Some(vk) if vk.wasm_valtype() == I32 => Ok(i32_load),
        _ => Err(WasmLowerError::Unsupported("map value of unknown/unsupported kind")),
    }
}

/// Leave `data_ptr + idx*16` (the address of map entry `idx`, a `[key:i64][value:i64]` pair) on
/// the stack.
fn emit_map_entry_addr(code: &mut Vec<u8>, m: u32, idx: u32) {
    local_get(code, m);
    i32_load(code, 8); // data_ptr
    local_get(code, idx);
    i32_const(code, 16);
    code.push(0x6C); // i32.mul
    code.push(0x6A); // i32.add
}

/// `Map of {Int,Text} to Int` insert (`Set item key of m to value`): linear-scan for `key` (i64.eq
/// for an Int key, byte-equality for a Text key); if present overwrite its value, else append a new
/// `[key][value]` entry (reallocating the entry buffer, like `ListPush`). The value must be `Int`.
fn lower_map_insert(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, map: u16, key: u16, value: u16) -> R<()> {
    let key_text = map_key_text(kinds, key)?;
    let val_store = map_value_store(kinds.get(value as usize))?;
    let m = map as u32;
    let (idx, found, new) = (num_regs + 5, num_regs + 6, num_regs + 7);
    // scan for an existing key
    i32_const(code, 0);
    local_set(code, idx);
    i32_const(code, 0);
    local_set(code, found);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    local_get(code, idx);
    local_get(code, m);
    i32_load(code, 0); // num_entries
    code.push(0x4E); // i32.ge_s → idx >= num
    code.push(0x0D);
    leb_u32(code, 1); // br_if block
    emit_map_key_compare(code, num_regs, key_text, m, idx, key);
    code.push(0x04);
    code.push(0x40); // if (key matches)
    emit_map_entry_addr(code, m, idx);
    local_get(code, value as u32);
    val_store(code, 8); // entry.value = value (at the value kind's width)
    i32_const(code, 1);
    local_set(code, found);
    code.push(0x0C);
    leb_u32(code, 2); // br block (out of if→loop→block)
    code.push(0x0B); // end if
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    // if not found: append a new entry (realloc the buffer to num+1 entries)
    local_get(code, found);
    code.push(0x45); // i32.eqz
    code.push(0x04);
    code.push(0x40); // if (!found)
    // new = alloc((num+1) * 16)
    local_get(code, m);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6A);
    i32_const(code, 16);
    code.push(0x6C);
    emit_alloc(code, ctx,new);
    // copy old entries: for i in 0..num: new[i] = old[i] (key then value)
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40);
    code.push(0x03);
    code.push(0x40);
    local_get(code, idx);
    local_get(code, m);
    i32_load(code, 0);
    code.push(0x4E);
    code.push(0x0D);
    leb_u32(code, 1);
    for field in [0u32, 8] {
        // new[idx].field = old[idx].field
        local_get(code, new);
        local_get(code, idx);
        i32_const(code, 16);
        code.push(0x6C);
        code.push(0x6A); // new entry addr
        emit_map_entry_addr(code, m, idx); // old entry addr
        i64_load(code, field);
        i64_store(code, field);
    }
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0);
    code.push(0x0B);
    code.push(0x0B);
    // new[num] = (key, value) — the key is an i64 (Int) or an i32 handle (Text)
    local_get(code, new);
    local_get(code, m);
    i32_load(code, 0);
    i32_const(code, 16);
    code.push(0x6C);
    code.push(0x6A); // addr of new entry slot
    local_get(code, key as u32);
    if key_text {
        i32_store(code, 0);
    } else {
        i64_store(code, 0);
    }
    local_get(code, new);
    local_get(code, m);
    i32_load(code, 0);
    i32_const(code, 16);
    code.push(0x6C);
    code.push(0x6A);
    local_get(code, value as u32);
    val_store(code, 8);
    // header: data_ptr = new; num_entries += 1
    local_get(code, m);
    local_get(code, new);
    i32_store(code, 8);
    local_get(code, m);
    local_get(code, m);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6A);
    i32_store(code, 0);
    code.push(0x0B); // end if (!found)
    Ok(())
}

/// `Map of Int to Int` get (`item key of m`): linear-scan for `key`, load its value into `dst`;
/// trap if absent (the tree-walker raises "Key not found", and the standalone module has no VM).
fn lower_map_get(code: &mut Vec<u8>, kinds: &KindTable, num_regs: u32, dst: u16, map: u16, key: u16) -> R<()> {
    let key_text = map_key_text(kinds, key)?;
    let val_load = map_value_load(kinds.get(dst as usize))?;
    let m = map as u32;
    let (idx, found) = (num_regs + 5, num_regs + 6);
    i32_const(code, 0);
    local_set(code, idx);
    i32_const(code, 0);
    local_set(code, found);
    code.push(0x02);
    code.push(0x40);
    code.push(0x03);
    code.push(0x40);
    local_get(code, idx);
    local_get(code, m);
    i32_load(code, 0);
    code.push(0x4E);
    code.push(0x0D);
    leb_u32(code, 1);
    emit_map_key_compare(code, num_regs, key_text, m, idx, key);
    code.push(0x04);
    code.push(0x40); // if (match)
    emit_map_entry_addr(code, m, idx);
    val_load(code, 8); // value (at the value kind's width)
    local_set(code, dst as u32);
    i32_const(code, 1);
    local_set(code, found);
    code.push(0x0C);
    leb_u32(code, 2);
    code.push(0x0B); // end if
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0);
    code.push(0x0B);
    code.push(0x0B);
    // absent key → trap
    local_get(code, found);
    code.push(0x45); // i32.eqz
    code.push(0x04);
    code.push(0x40);
    code.push(0x00); // unreachable
    code.push(0x0B);
    Ok(())
}

/// `m contains key` (Map): linear-scan for the key → Bool i64 0/1 in `dst`.
fn lower_map_contains(code: &mut Vec<u8>, kinds: &KindTable, num_regs: u32, dst: u16, map: u16, key: u16) -> R<()> {
    let key_text = map_key_text(kinds, key)?;
    let m = map as u32;
    let idx = num_regs + 5;
    code.push(0x42);
    leb_i64(code, 0);
    local_set(code, dst as u32); // dst = 0
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40);
    code.push(0x03);
    code.push(0x40);
    local_get(code, idx);
    local_get(code, m);
    i32_load(code, 0);
    code.push(0x4E);
    code.push(0x0D);
    leb_u32(code, 1);
    emit_map_key_compare(code, num_regs, key_text, m, idx, key);
    code.push(0x04);
    code.push(0x40); // if (match)
    code.push(0x42);
    leb_i64(code, 1);
    local_set(code, dst as u32); // dst = 1
    code.push(0x0C);
    leb_u32(code, 2); // break (found)
    code.push(0x0B);
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0);
    code.push(0x0B);
    code.push(0x0B);
    Ok(())
}

/// Byte-equality of two `Text` handles in locals `a`/`b`, leaving an i32 (1 = equal, 0 = not) on the
/// wasm stack — the handle-in-local, on-stack-result form of [`lower_text_eq`], so it drops into a
/// set scan's comparison exactly where an `i64.eq` would. Uses `+9` (byte index) and `+10` (result)
/// scratch, distinct from a set scan's `+5`/`+6`/`+7` and the `+8` slot-handle temp.
fn emit_text_handles_eq(code: &mut Vec<u8>, num_regs: u32, a: u32, b: u32) {
    let (idx, res) = (num_regs + 9, num_regs + 10);
    i32_const(code, 1);
    local_set(code, res); // assume equal
    // if len_a != len_b → not equal
    local_get(code, a);
    i32_load(code, 0);
    local_get(code, b);
    i32_load(code, 0);
    code.push(0x47); // i32.ne
    code.push(0x04);
    code.push(0x40); // if (lengths differ)
    i32_const(code, 0);
    local_set(code, res);
    code.push(0x05); // else — compare bytes
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40); // block $done
    code.push(0x03);
    code.push(0x40); // loop
    local_get(code, idx);
    local_get(code, a);
    i32_load(code, 0); // len
    code.push(0x4E); // i32.ge_s → idx >= len (all matched)
    code.push(0x0D);
    leb_u32(code, 1); // br_if $done
    // a.data[idx] != b.data[idx] ?
    local_get(code, a);
    i32_load(code, 8);
    local_get(code, idx);
    code.push(0x6A);
    i32_load8_u(code, 0);
    local_get(code, b);
    i32_load(code, 8);
    local_get(code, idx);
    code.push(0x6A);
    i32_load8_u(code, 0);
    code.push(0x47); // i32.ne
    code.push(0x04);
    code.push(0x40); // if (byte differs)
    i32_const(code, 0);
    local_set(code, res);
    code.push(0x0C);
    leb_u32(code, 2); // br $done (mismatch)
    code.push(0x0B); // end if
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    code.push(0x0B); // end outer if
    local_get(code, res); // leave the result on the stack
}

/// Emit "does `set[idx]` equal `value`?", leaving an i32 (1/0) on the wasm stack — the element
/// comparison shared by the set scans. Int: `i64.eq` of the slot element and `value`. `SetText`:
/// byte-equality of the slot's `Text` handle and `value`'s (`emit_text_handles_eq`, via the `+8`
/// slot-handle temp). Both read the 8-byte slot at `data_ptr + idx*8`.
fn emit_set_elem_eq(code: &mut Vec<u8>, num_regs: u32, set: u32, idx: u32, value: u32, is_text: bool) {
    local_get(code, set);
    i32_load(code, 8); // data_ptr
    local_get(code, idx);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A); // slot addr = data_ptr + idx*8
    if is_text {
        i32_load(code, 0); // the slot's Text handle
        local_set(code, num_regs + 8);
        emit_text_handles_eq(code, num_regs, num_regs + 8, value);
    } else {
        i64_load(code, 0); // the slot's i64 element
        local_get(code, value);
        code.push(0x51); // i64.eq
    }
}

/// `Remove value from c` (`RemoveFrom`) on a `Set of Int`/`Set of Text` or `Map of Int to Int` —
/// linear-scan for the value (a Set element, or a Map key at entry offset 0); if found, swap-remove
/// it (overwrite the found slot with the last slot and decrement the count). Set/Map are
/// order-independent so swap-remove is byte-identical to the VM for length/contains/get. A `Set of
/// Text` compares by BYTE equality (`emit_set_elem_eq` text path), not handle identity.
fn lower_remove_from(code: &mut Vec<u8>, kinds: &KindTable, num_regs: u32, collection: u16, value: u16) -> R<()> {
    let set_text = matches!(kinds.get(collection as usize), Some(Kind::SetText) | Some(Kind::CrdtSetText));
    if !set_text && kinds.get(value as usize) != Some(Kind::Int) {
        return Err(WasmLowerError::Unsupported("remove of a non-Int value"));
    }
    let stride: i32 = match kinds.get(collection as usize) {
        Some(Kind::Set) | Some(Kind::SetText) | Some(Kind::CrdtSetText) => 8,
        Some(Kind::Map) => 16,
        _ => return Err(WasmLowerError::Unsupported("remove from a non-set/map collection")),
    };
    let c = collection as u32;
    let (idx, found) = (num_regs + 5, num_regs + 6);
    i32_const(code, 0);
    local_set(code, idx);
    i32_const(code, 0);
    local_set(code, found);
    code.push(0x02);
    code.push(0x40);
    code.push(0x03);
    code.push(0x40);
    local_get(code, idx);
    local_get(code, c);
    i32_load(code, 0); // num
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if block
    // data[idx] (element / key at offset 0) == value ? — a `Set of Text` compares by bytes (stride
    // is 8, a set), an Int set / Map key by `i64.eq` at its stride.
    if set_text {
        emit_set_elem_eq(code, num_regs, c, idx, value as u32, true);
    } else {
        local_get(code, c);
        i32_load(code, 8);
        local_get(code, idx);
        i32_const(code, stride);
        code.push(0x6C);
        code.push(0x6A);
        i64_load(code, 0);
        local_get(code, value as u32);
        code.push(0x51); // i64.eq
    }
    code.push(0x04);
    code.push(0x40); // if (match)
    // swap-remove: copy the last slot over slot idx (one i64 for a Set, two for a Map entry)
    let offs: &[u32] = if stride == 16 { &[0, 8] } else { &[0] };
    for &off in offs {
        local_get(code, c);
        i32_load(code, 8);
        local_get(code, idx);
        i32_const(code, stride);
        code.push(0x6C);
        code.push(0x6A); // dst slot base
        local_get(code, c);
        i32_load(code, 8);
        local_get(code, c);
        i32_load(code, 0);
        i32_const(code, 1);
        code.push(0x6B); // i32.sub → num-1
        i32_const(code, stride);
        code.push(0x6C);
        code.push(0x6A); // src (last) slot base
        i64_load(code, off);
        i64_store(code, off);
    }
    // num -= 1
    local_get(code, c);
    local_get(code, c);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6B);
    i32_store(code, 0);
    i32_const(code, 1);
    local_set(code, found);
    code.push(0x0C);
    leb_u32(code, 2); // br block (removed)
    code.push(0x0B); // end if
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0);
    code.push(0x0B);
    code.push(0x0B);
    Ok(())
}

/// `Set of Int` add (`Add value to s`): linear-scan for `value`; if already present do nothing,
/// else append it (reallocating the element buffer, like `ListPush`). `value` must be `Int`.
fn lower_set_add(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, set: u16, value: u16) -> R<()> {
    let is_text = matches!(kinds.get(set as usize), Some(Kind::SetText) | Some(Kind::CrdtSetText));
    if is_text {
        if kinds.get(value as usize) != Some(Kind::Text) {
            return Err(WasmLowerError::Unsupported("adding a non-Text value to a Set of Text"));
        }
    } else if kinds.get(value as usize) != Some(Kind::Int) {
        return Err(WasmLowerError::Unsupported("set with a non-Int value (only Set of Int/Text)"));
    }
    emit_set_add_elem(code, ctx, num_regs, set as u32, value as u32, is_text);
    Ok(())
}

/// The add-if-absent core of [`lower_set_add`], over raw locals: scan the set whose handle is in
/// `set` for the value already held in `elem`; if absent, append it (realloc, like `ListPush`).
/// Uses the `+5`/`+6`/`+7` i32 scratch (`idx`/`found`/`new`) and an 8-byte stride. `set` and
/// `elem` must be locals untouched by those scratch slots — so union/intersect can drive it in a
/// loop with their own outer counter (`+9`) and loaded element (`+1`). `is_text` selects BYTE
/// equality (a `Set of Text`; the slot's low word is a Text handle) over `i64.eq`.
fn emit_set_add_elem(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, s: u32, elem: u32, is_text: bool) {
    let (idx, found, new) = (num_regs + 5, num_regs + 6, num_regs + 7);
    // scan for an existing equal value
    i32_const(code, 0);
    local_set(code, idx);
    i32_const(code, 0);
    local_set(code, found);
    code.push(0x02);
    code.push(0x40);
    code.push(0x03);
    code.push(0x40);
    local_get(code, idx);
    local_get(code, s);
    i32_load(code, 0); // num
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if block
    emit_set_elem_eq(code, num_regs, s, idx, elem, is_text); // set[idx] == elem ?
    code.push(0x04);
    code.push(0x40); // if (present)
    i32_const(code, 1);
    local_set(code, found);
    code.push(0x0C);
    leb_u32(code, 2); // br block (already a member)
    code.push(0x0B);
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0);
    code.push(0x0B);
    code.push(0x0B);
    // if not present: append (realloc to num+1 elements, like ListPush)
    local_get(code, found);
    code.push(0x45); // i32.eqz
    code.push(0x04);
    code.push(0x40); // if (!present)
    local_get(code, s);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6A);
    i32_const(code, 8);
    code.push(0x6C);
    emit_alloc(code, ctx,new); // new = alloc((num+1)*8)
    emit_seq_copy(code, idx, new, s, s, false); // copy the num existing values
    local_get(code, new);
    local_get(code, s);
    i32_load(code, 0);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    local_get(code, elem);
    if is_text {
        // The value is a Text handle: store it in the slot's low word (the freshly-alloc'd buffer's
        // high word is zero, matching how the Int path leaves it).
        i32_store(code, 0); // new[num] = handle
    } else {
        i64_store(code, 0); // new[num] = value
    }
    local_get(code, s);
    local_get(code, new);
    i32_store(code, 8); // data_ptr = new
    local_get(code, s);
    local_get(code, s);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6A);
    i32_store(code, 0); // num += 1
    code.push(0x0B); // end if (!present)
}

/// Set membership of the i64 in `elem` against the set whose handle is in `set` → `1`/`0` in `out`
/// (using `idx` as the byte-loop counter). The value-local form of [`lower_contains`]'s scan, so
/// it can gate per-element copies inside the intersection loop.
fn emit_set_contains_to(code: &mut Vec<u8>, set: u32, elem: u32, out: u32, idx: u32) {
    i32_const(code, 0);
    local_set(code, out); // assume absent
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40); // block $exit
    code.push(0x03);
    code.push(0x40); // loop $loop
    local_get(code, idx);
    local_get(code, set);
    i32_load(code, 0); // num
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if $exit
    local_get(code, set);
    i32_load(code, 8); // data_ptr
    local_get(code, idx);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    i64_load(code, 0); // element
    local_get(code, elem);
    code.push(0x51); // i64.eq
    code.push(0x04);
    code.push(0x40); // if (present)
    i32_const(code, 1);
    local_set(code, out);
    code.push(0x0C);
    leb_u32(code, 2); // br $exit
    code.push(0x0B); // end if
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0); // br $loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
}

/// Walk the source set `src` element-by-element (outer counter in `+9`, each element loaded into the
/// i64 scratch `+1`) and feed each through [`emit_set_add_elem`] into `result` (whose add-if-absent
/// dedups). With `filter = Some(other)`, only elements also present in `other` are copied — the
/// intersection gate. The inner add uses `+5`/`+6`/`+7`; the membership scan uses `+10`/`+11`; none
/// collide with the outer counter, the element, or `result` (`+8`), so the nest is sound.
fn emit_set_copy_loop(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, result: u32, src: u32, filter: Option<u32>) {
    let outer = num_regs + 9;
    let elem = num_regs + 1;
    let (cidx, cfound) = (num_regs + 10, num_regs + 11);
    i32_const(code, 0);
    local_set(code, outer);
    code.push(0x02);
    code.push(0x40); // block $exit
    code.push(0x03);
    code.push(0x40); // loop $loop
    local_get(code, outer);
    local_get(code, src);
    i32_load(code, 0); // len(src)
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if $exit
    // elem = src.data[outer*8]
    local_get(code, src);
    i32_load(code, 8);
    local_get(code, outer);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    i64_load(code, 0);
    local_set(code, elem);
    match filter {
        None => emit_set_add_elem(code, ctx, num_regs, result, elem, false),
        Some(other) => {
            emit_set_contains_to(code, other, elem, cfound, cidx);
            local_get(code, cfound);
            code.push(0x04);
            code.push(0x40); // if (in `other`)
            emit_set_add_elem(code, ctx, num_regs, result, elem, false);
            code.push(0x0B); // end if
        }
    }
    // outer++
    local_get(code, outer);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, outer);
    code.push(0x0C);
    leb_u32(code, 0); // br $loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
}

/// `a union b` — a fresh `Set` of `a`'s elements (in `a`'s order) followed by `b`'s not-already-
/// present elements (in `b`'s order), matching `semantics::collections::union` byte-for-byte
/// (`add`'s dedup makes empty-then-add-all-of-a-then-add-all-of-b equal to clone-a-then-add-b,
/// since a Set has no internal duplicates). Built into the `+8` scratch handle, then bound to
/// `dst` — so a `dst` aliasing `lhs`/`rhs` reads the originals fully before being overwritten.
fn lower_union(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, dst: u16, lhs: u16, rhs: u16) -> R<()> {
    require_set(kinds, lhs)?;
    require_set(kinds, rhs)?;
    let result = num_regs + 8;
    emit_empty_header(code, ctx, num_regs, result);
    emit_set_copy_loop(code, ctx, num_regs, result, lhs as u32, None);
    emit_set_copy_loop(code, ctx, num_regs, result, rhs as u32, None);
    local_get(code, result);
    local_set(code, dst as u32);
    Ok(())
}

/// `a intersection b` — a fresh `Set` of `a`'s elements (in `a`'s order) that are also in `b`,
/// matching `semantics::collections::intersection`. Same build-into-`+8`-then-bind discipline as
/// [`lower_union`].
fn lower_intersect(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, dst: u16, lhs: u16, rhs: u16) -> R<()> {
    require_set(kinds, lhs)?;
    require_set(kinds, rhs)?;
    let result = num_regs + 8;
    emit_empty_header(code, ctx, num_regs, result);
    emit_set_copy_loop(code, ctx, num_regs, result, lhs as u32, Some(rhs as u32));
    local_get(code, result);
    local_set(code, dst as u32);
    Ok(())
}

/// Require a `Set` operand for union/intersection (our Sets only ever hold Int, so a `Set` kind is
/// enough to license the 8-byte-stride element copy).
fn require_set(kinds: &KindTable, r: u16) -> R<()> {
    if kinds.get(r as usize) == Some(Kind::Set) {
        Ok(())
    } else {
        Err(WasmLowerError::Unsupported("union/intersect of a non-Set value"))
    }
}

/// `seq contains value` / `set contains value` — a linear membership scan (value equality,
/// matching the tree-walker's `List::position` / set membership): `dst = 1` if any element equals
/// `value`, else `0`. Int/Set-of-Int compare with `i64.eq`, Float with `f64.eq`.
fn lower_contains(code: &mut Vec<u8>, kinds: &KindTable, num_regs: u32, dst: u16, collection: u16, value: u16) -> R<()> {
    let set_text = matches!(kinds.get(collection as usize), Some(Kind::SetText) | Some(Kind::CrdtSetText));
    let elem = match kinds.get(collection as usize) {
        Some(Kind::Set) => Kind::Int, // a Set of Int holds i64 values
        Some(Kind::SetText) | Some(Kind::CrdtSetText) => Kind::Text, // a Set of Text scans by byte equality
        _ => seq_elem_kind(kinds, collection)?,
    };
    // A `Set of Text` `contains` DOES value (byte) equality (below); a plain `seq of Text contains
    // text` is still deferred (the scalar membership scan would wrongly compare Text handle pointers).
    if elem == Kind::Text && !set_text {
        return Err(WasmLowerError::Unsupported("contains over a sequence of Text (needs byte-equality)"));
    }
    let (elem_load, elem_eq): (fn(&mut Vec<u8>, u32), u8) =
        if elem == Kind::Float { (f64_load, 0x61) } else { (i64_load, 0x51) };
    let idx = num_regs + 5; // i32 scratch
    let col = collection as u32;
    // dst = 0
    code.push(0x42);
    leb_i64(code, 0);
    local_set(code, dst as u32);
    // idx = 0
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40); // block $exit
    code.push(0x03);
    code.push(0x40); // loop $loop
    // if idx >= len: break
    local_get(code, idx);
    local_get(code, col);
    i32_load(code, 0); // len
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if $exit
    // element == value ? — a Set of Text compares bytes, everything else `i64.eq`/`f64.eq`.
    if set_text {
        emit_set_elem_eq(code, num_regs, col, idx, value as u32, true);
    } else {
        local_get(code, col);
        i32_load(code, 8); // data_ptr
        local_get(code, idx);
        i32_const(code, 8);
        code.push(0x6C);
        code.push(0x6A);
        elem_load(code, 0);
        local_get(code, value as u32);
        code.push(elem_eq);
    }
    code.push(0x04);
    code.push(0x40); // if (void)
    code.push(0x42);
    leb_i64(code, 1);
    local_set(code, dst as u32); // dst = 1
    code.push(0x0C);
    leb_u32(code, 2); // br $exit (out of if → loop → block)
    code.push(0x0B); // end if
    // idx++
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0); // br $loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    Ok(())
}

/// `lhs + rhs` (the tree-walker's `arith::concat`): stringify each operand and concatenate the
/// UTF-8 bytes into a fresh `Text`. A `Text` operand is its own bytes; an `Int` operand is
/// formatted by the host `fmt_i64_into` (string interpolation `"… {n} …"` lowers to this). Other
/// operand kinds (Float/Bool stringification) are not yet built.
fn lower_concat(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, dst: u16, lhs: u16, rhs: u16) -> R<()> {
    // Stringify each operand into a Text handle held in a dedicated scratch local (the byte-copy
    // below reads their headers; `emit_stringify` only borrows the +5/+6/+7 temps, so the two
    // handles in +8/+9 survive across both calls and the copy).
    let (a, b) = (num_regs + 8, num_regs + 9);
    emit_stringify(code, ctx, num_regs, lhs as u32, kinds.get(lhs as usize), a)?;
    emit_stringify(code, ctx, num_regs, rhs as u32, kinds.get(rhs as usize), b)?;
    emit_text_concat(code, ctx, num_regs, a, b, dst as u32);
    Ok(())
}

/// The alignment + width of a non-precision format spec: `>N` (right, code 0), `<N` (left, 1), `^N`
/// (center, 2), or a bare width `N` (right — matching `apply_format_spec`'s default). `None` for a spec
/// this backend doesn't lower (e.g. the `$` currency spec).
fn parse_align_spec(spec: &str) -> Option<(i32, u32)> {
    if let Some(first) = spec.as_bytes().first() {
        let align = match first {
            b'>' => Some(0),
            b'<' => Some(1),
            b'^' => Some(2),
            _ => None,
        };
        if let Some(a) = align {
            return spec[1..].parse::<u32>().ok().map(|w| (a, w));
        }
    }
    // A bare width is right-aligned (`format!("{:>w$}", s)`).
    spec.parse::<u32>().ok().map(|w| (0, w))
}

/// `"{x:.9}"` / `"{x:>6}"` — a formatted interpolation piece: render `src` under its format spec into a
/// fresh `Text` (the VM's `apply_format_spec`). Two families are lowered: `.N` precision (numeric →
/// `format!("{:.N}", val)`, the `fmt_f64_prec_into` host) and alignment/width (`>N`/`<N`/`^N`/bare-`N` →
/// stringify the value then pad to width via `fmt_align_into`). The spec is a compile-time Text
/// constant. The `$` currency spec and the debug prefix (`{x=…}`) are refused (a documented deferral).
fn lower_format_value(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    src: u16,
    spec: u32,
    debug_prefix: u32,
) -> R<()> {
    if debug_prefix != u32::MAX {
        return Err(WasmLowerError::Unsupported("formatted value with a debug prefix"));
    }
    let spec_s = match spec {
        u32::MAX => return Err(WasmLowerError::Unsupported("formatted value without a spec")),
        idx => match ctx.constants.get(idx as usize) {
            Some(Constant::Text(s)) => s.as_str(),
            _ => return Err(WasmLowerError::Unsupported("format spec is not a text constant")),
        },
    };
    // Alignment / bare-width spec: stringify the value (any stringifiable kind), then pad to `width`
    // with spaces via `fmt_align_into` (the SAME Rust `format!` `apply_format_spec` runs).
    if !spec_s.starts_with('.') {
        let (align, width) = parse_align_spec(spec_s).ok_or(WasmLowerError::Unsupported("unsupported format spec"))?;
        let fidx = (ctx.host_index)(HostFn::FmtAlignInto).ok_or(WasmLowerError::Unsupported("align formatter not imported"))?;
        let (hdr, data, len, text) = (num_regs + 5, num_regs + 6, num_regs + 7, num_regs + 9);
        // text = stringify(src) — a Text handle (`fmt_align_into` reads its bytes)
        emit_stringify(code, ctx, num_regs, src as u32, kinds.get(src as usize), text)?;
        // data = alloc(text.len + width) — padding adds at most `width` single-byte spaces
        local_get(code, text);
        i32_load(code, 0);
        i32_const(code, width as i32);
        code.push(0x6A); // i32.add
        emit_alloc(code, ctx, data);
        // len = fmt_align_into(data, text, width, align)
        local_get(code, data);
        local_get(code, text);
        i32_const(code, width as i32);
        i32_const(code, align);
        code.push(0x10); // call
        leb_u32(code, fidx);
        local_set(code, len);
        // hdr = alloc(16); header = [len, cap=len, data_ptr=data]
        i32_const(code, 16);
        emit_alloc(code, ctx, hdr);
        for off in [0u32, 4] {
            local_get(code, hdr);
            local_get(code, len);
            i32_store(code, off);
        }
        local_get(code, hdr);
        local_get(code, data);
        i32_store(code, 8);
        local_get(code, hdr);
        local_set(code, dst as u32);
        return Ok(());
    }
    let prec = spec_s
        .strip_prefix('.')
        .and_then(|r| r.parse::<u32>().ok())
        .ok_or(WasmLowerError::Unsupported("unsupported format spec (only `.N` precision)"))?;
    match kinds.get(src as usize) {
        Some(Kind::Float) | Some(Kind::Int) | Some(Kind::Bool) => {}
        _ => return Err(WasmLowerError::Unsupported("format `.N` on a non-numeric value")),
    }
    let (hdr, data, len) = (num_regs + 5, num_regs + 6, num_regs + 7);
    // hdr = alloc(16); data = alloc(340 + prec) bytes (worst-case f64 integer width + the decimals)
    i32_const(code, 16);
    emit_alloc(code, ctx,hdr);
    i32_const(code, (340 + prec) as i32);
    emit_alloc(code, ctx,data);
    // len = fmt_f64_prec_into(data, src as f64, prec)
    let fidx = (ctx.host_index)(HostFn::FmtF64PrecInto).ok_or(WasmLowerError::Unsupported("precision formatter not imported"))?;
    local_get(code, data);
    push_as_f64(code, src, kinds.get(src as usize))?;
    i32_const(code, prec as i32);
    code.push(0x10); // call
    leb_u32(code, fidx);
    local_set(code, len);
    // header: len = cap = len; data_ptr = data
    for off in [0u32, 4] {
        local_get(code, hdr);
        local_get(code, len);
        i32_store(code, off);
    }
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8);
    local_get(code, hdr);
    local_set(code, dst as u32);
    Ok(())
}

/// Concatenate the two `Text` handles in locals `a` and `b` into a fresh `Text` whose handle is
/// stored in local `out`. Uses the +5/+6/+7 scratch as temps (so `a`/`b`/`out` must be other
/// locals); `out` may alias `a` (all reads happen before the final store).
fn emit_text_concat(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, a: u32, b: u32, out: u32) {
    let (hdr, data, idx) = (num_regs + 5, num_regs + 6, num_regs + 7);
    i32_const(code, 16);
    emit_alloc(code, ctx,hdr);
    // data = alloc(len_a + len_b) bytes (the allocator re-aligns the next pointer, so the byte
    // buffer need not be padded)
    local_get(code, a);
    i32_load(code, 0);
    local_get(code, b);
    i32_load(code, 0);
    code.push(0x6A); // len_a + len_b
    emit_alloc(code, ctx,data);
    emit_byte_copy(code, idx, data, a, a, false); // a bytes at [0, len_a)
    emit_byte_copy(code, idx, data, a, b, true); // b bytes at [len_a, len_a+len_b)
    for off in [0u32, 4] {
        local_get(code, hdr);
        local_get(code, a);
        i32_load(code, 0);
        local_get(code, b);
        i32_load(code, 0);
        code.push(0x6A);
        i32_store(code, off);
    }
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8);
    local_get(code, hdr);
    local_set(code, out);
}

/// Store opcode for an 8-byte struct field slot, by the value's kind (`i64`/`f64`/`i32`-handle).
/// An unknown kind cannot be stored at a definite width, so it is refused.
fn emit_slot_store(code: &mut Vec<u8>, k: Option<Kind>, off: u32) -> R<()> {
    match k {
        Some(Kind::Float) => f64_store(code, off),
        Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Char) | Some(Kind::Moment) | Some(Kind::Duration) | Some(Kind::Time) | Some(Kind::Span) | Some(Kind::Word64) => i64_store(code, off),
        Some(Kind::Date) | Some(Kind::SeqInt) | Some(Kind::SeqBool) | Some(Kind::SeqFloat) | Some(Kind::SeqText) | Some(Kind::SeqStruct) | Some(Kind::SeqEnum) | Some(Kind::SeqSeqInt) | Some(Kind::SeqAny) | Some(Kind::Text) | Some(Kind::Struct) | Some(Kind::Map) | Some(Kind::Set) | Some(Kind::SetText) | Some(Kind::CrdtSetText) | Some(Kind::Enum) | Some(Kind::Closure) | Some(Kind::Tuple) | Some(Kind::Rational) | Some(Kind::Optional) | Some(Kind::Word32) | Some(Kind::SeqWord32) | Some(Kind::SeqWord64) | Some(Kind::BigInt) | Some(Kind::Complex) | Some(Kind::Modular) | Some(Kind::Decimal) | Some(Kind::Money) | Some(Kind::Quantity) | Some(Kind::Uuid) | Some(Kind::Lanes) | Some(Kind::LanesV) | Some(Kind::Dynamic) => {
            i32_store(code, off)
        }
        None => return Err(WasmLowerError::Unsupported("struct field of unknown kind")),
    }
    Ok(())
}

/// Load opcode for an 8-byte struct field slot — the mirror of [`emit_slot_store`].
fn emit_slot_load(code: &mut Vec<u8>, k: Option<Kind>, off: u32) -> R<()> {
    match k {
        Some(Kind::Float) => f64_load(code, off),
        Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Char) | Some(Kind::Moment) | Some(Kind::Duration) | Some(Kind::Time) | Some(Kind::Span) | Some(Kind::Word64) => i64_load(code, off),
        Some(Kind::Date) | Some(Kind::SeqInt) | Some(Kind::SeqBool) | Some(Kind::SeqFloat) | Some(Kind::SeqText) | Some(Kind::SeqStruct) | Some(Kind::SeqEnum) | Some(Kind::SeqSeqInt) | Some(Kind::SeqAny) | Some(Kind::Text) | Some(Kind::Struct) | Some(Kind::Map) | Some(Kind::Set) | Some(Kind::SetText) | Some(Kind::CrdtSetText) | Some(Kind::Enum) | Some(Kind::Closure) | Some(Kind::Tuple) | Some(Kind::Rational) | Some(Kind::Optional) | Some(Kind::Word32) | Some(Kind::SeqWord32) | Some(Kind::SeqWord64) | Some(Kind::BigInt) | Some(Kind::Complex) | Some(Kind::Modular) | Some(Kind::Decimal) | Some(Kind::Money) | Some(Kind::Quantity) | Some(Kind::Uuid) | Some(Kind::Lanes) | Some(Kind::LanesV) | Some(Kind::Dynamic) => {
            i32_load(code, off)
        }
        None => return Err(WasmLowerError::Unsupported("struct field of unknown kind")),
    }
    Ok(())
}

/// `a new T with …` (`NewStruct`) — bump-allocate the header + a `count`-slot (8 bytes each) field
/// buffer (`count` from the static layout). Slots are filled by the following `StructInsert`s
/// (every field is inserted, provided or default-filled), so they need no zero-init.
fn lower_new_struct(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, count: u16, dst: u16) {
    let (hdr, data) = (num_regs + 5, num_regs + 6);
    i32_const(code, 16);
    emit_alloc(code, ctx,hdr);
    i32_const(code, i32::from(count) * 8);
    emit_alloc(code, ctx,data);
    local_get(code, hdr);
    i32_const(code, i32::from(count));
    i32_store(code, 0); // num_fields
    local_get(code, hdr);
    i32_const(code, i32::from(count));
    i32_store(code, 4); // cap
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8); // data_ptr
    local_get(code, hdr);
    local_set(code, dst as u32);
}

/// `Set obj's field to value` (`StructInsert`) — store the value into its static slot, at the
/// width of the value's kind.
/// Which `StructInsert`s need COPY-ON-WRITE — a flow-sensitive uniqueness pass that keeps Logos's
/// struct VALUE semantics without cloning on the read path. Structs are heap objects behind shared
/// handles here, but the tree-walker/VM copy a struct on field access and assignment, so mutating an
/// extracted or aliased struct must not write through to the original. Rather than clone on every
/// `GetField` (the hot read path), we clone lazily at the WRITE: a `StructInsert` whose target is not
/// provably a uniquely-owned fresh struct copies first, then mutates the copy.
///
/// `owned` holds registers currently bound to a unique, unaliased struct: set by `NewStruct`/
/// `DeepClone`; preserved across the construction `StructInsert`s that fill it and across `GetField`
/// reads of it (reading a field does not alias the struct); cleared the moment the struct is consumed
/// as a value anywhere else (stored into a field/collection, moved, returned, passed to a call — any
/// op's def/use footprint, via the exhaustive [`regsplit::op_def_uses`], so no aliasing operand is
/// missed). State is reset at every basic-block leader, a conservative join (a struct owned on only
/// some incoming paths is treated as shared → it copies, never miscompiles). So construction and
/// reads cost nothing; only a write to a possibly-shared struct pays one copy.
fn cow_struct_inserts(ops: &[Op], num_regs: u32, functions: &[CompiledFunction]) -> Vec<bool> {
    let mut cow = vec![false; ops.len()];
    let Some(blocks) = Blocks::new(ops) else {
        // Not self-contained (rejected downstream); be safe and copy every struct write.
        for (pc, op) in ops.iter().enumerate() {
            if matches!(op, Op::StructInsert { .. }) {
                cow[pc] = true;
            }
        }
        return cow;
    };
    let mut owned = vec![false; num_regs as usize];
    let is_owned = |owned: &[bool], r: u16| (r as usize) < owned.len() && owned[r as usize];
    let set_owned = |owned: &mut [bool], r: u16, v: bool| {
        if (r as usize) < owned.len() {
            owned[r as usize] = v;
        }
    };
    for (pc, op) in ops.iter().enumerate() {
        if blocks.is_leader(pc) {
            owned.iter_mut().for_each(|o| *o = false);
        }
        match op {
            Op::NewStruct { dst, .. } | Op::DeepClone { dst, .. } => set_owned(&mut owned, *dst, true),
            Op::StructInsert { obj, value, .. } => {
                if !is_owned(&owned, *obj) {
                    cow[pc] = true; // mutating a possibly-shared struct → copy first
                }
                set_owned(&mut owned, *value, false); // the stored value is now aliased by obj.field
                set_owned(&mut owned, *obj, true); // obj is uniquely owned after (fresh copy, or already)
            }
            Op::GetField { dst, .. } => set_owned(&mut owned, *dst, false), // a borrowed field handle
            _ => {
                let (defs, uses) = regsplit::op_def_uses(op, functions);
                for r in defs.into_iter().chain(uses) {
                    set_owned(&mut owned, r, false);
                }
            }
        }
    }
    cow
}

/// `Set obj's field to value` / a construction field-fill (`StructInsert`). When `cow`, the target
/// struct may be shared (extracted by `GetField` or stored elsewhere), so — to honor value semantics
/// — flat-copy it into a fresh object bound back to `obj` and mutate THAT; otherwise mutate in place.
fn lower_struct_insert(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    slot: u16,
    obj: u16,
    value: u16,
    cow: bool,
) -> R<()> {
    // The target must be a known Struct handle (an i32). A struct param has no static layout, so it
    // is typed as a scalar here — reject rather than treat the scalar as a heap pointer (which the
    // copy-on-write below would, emitting invalid wasm).
    if kinds.get(obj as usize) != Some(Kind::Struct) {
        return Err(WasmLowerError::Unsupported("struct field set on a value with no known struct layout"));
    }
    if cow {
        let (hdr, data, idx) = (num_regs + 5, num_regs + 6, num_regs + 7);
        emit_buffer_clone(code, ctx, hdr, data, idx, obj as u32, false);
        local_get(code, hdr);
        local_set(code, obj as u32);
    }
    let off = u32::from(slot) * 8;
    local_get(code, obj as u32);
    i32_load(code, 8); // data_ptr
    local_get(code, value as u32);
    emit_slot_store(code, kinds.get(value as usize), off)
}

/// `obj's field` (`GetField`) — load the value from its static slot, at the width of the field's
/// kind (the inferred kind of `dst`).
fn lower_get_field(code: &mut Vec<u8>, kinds: &KindTable, slot: u16, dst: u16, obj: u16) -> R<()> {
    if kinds.get(obj as usize) != Some(Kind::Struct) {
        return Err(WasmLowerError::Unsupported("field access on a value with no known struct layout"));
    }
    let off = u32::from(slot) * 8;
    local_get(code, obj as u32);
    i32_load(code, 8); // data_ptr
    emit_slot_load(code, kinds.get(dst as usize), off)?;
    local_set(code, dst as u32);
    Ok(())
}

/// Whether a value of `kind` is its own deep clone — a scalar with no sub-structure to copy
/// recursively (`Int`/`Bool`/`Char`/`Float`/`Date`/`Moment`). A handle kind (Text/Seq/Struct/…) is not.
fn is_clone_trivial(kind: Option<Kind>) -> bool {
matches!(kind, Some(Kind::Int | Kind::Bool | Kind::Char | Kind::Float | Kind::Date | Kind::Moment | Kind::Duration | Kind::Time | Kind::Span))
}

/// A struct field this backend can deep-clone: a scalar (copied flat with the field buffer) or a
/// `Text`/`Set`/scalar-sequence handle (clonable into an independent buffer one level down). A field
/// that is itself a struct / map / enum / nested-handle sequence needs deeper recursion and is not
/// cloned here (the struct clone is then refused, soundly).
fn field_clone_ok(kind: Option<Kind>) -> bool {
    is_clone_trivial(kind)
        || matches!(kind, Some(Kind::Text | Kind::SeqInt | Kind::SeqBool | Kind::SeqFloat | Kind::SeqAny | Kind::Set))
}

/// Clone a `Text` (byte buffer) or sequence/`Set` (8-byte-slot buffer) whose handle is in `src_local`
/// into a fresh, independent object, leaving the new handle in `hdr`. `is_text` selects the copy
/// stride; `hdr`/`data`/`idx` are three scratch locals. The fresh buffer is why a later in-place
/// mutation of the clone (or the original) cannot be seen through the other.
fn emit_buffer_clone(code: &mut Vec<u8>, ctx: &Ctx, hdr: u32, data: u32, idx: u32, src_local: u32, is_text: bool) {
    i32_const(code, 16);
    emit_alloc(code, ctx,hdr);
    local_get(code, src_local);
    i32_load(code, 0); // len (bytes for Text, element count otherwise)
    if !is_text {
        i32_const(code, 8);
        code.push(0x6C); // i32.mul — 8-byte slots
    }
    emit_alloc(code, ctx,data);
    if is_text {
        emit_byte_copy(code, idx, data, src_local, src_local, false);
    } else {
        emit_seq_copy(code, idx, data, src_local, src_local, false);
    }
    for off in [0u32, 4] {
        local_get(code, hdr);
        local_get(code, src_local);
        i32_load(code, 0); // len → both len and cap
        i32_store(code, off);
    }
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8); // data_ptr
}

/// Clone a `Map` — a `num_entries × 16`-byte buffer of `[key@0][value@8]` pairs — whose handle is in
/// `src_local`, into a fresh independent map left in `hdr`. Flat entry copy (`num_entries × 2` `i64`
/// slots): the corpus maps have Text/scalar keys and scalar values, so the pairs copy by value; a
/// handle value would stay shared (a deep value clone is a later refinement). `hdr`/`data`/`idx` are
/// three scratch locals. The fresh buffer is why a later `Set item k of map` on the clone (or the
/// original) is invisible through the other holder.
fn emit_map_clone(code: &mut Vec<u8>, ctx: &Ctx, hdr: u32, data: u32, idx: u32, src_local: u32) {
    i32_const(code, 16);
    emit_alloc(code, ctx,hdr);
    // data = alloc(num_entries * 16)
    local_get(code, src_local);
    i32_load(code, 0); // num_entries
    i32_const(code, 16);
    code.push(0x6C);
    emit_alloc(code, ctx,data);
    // copy num_entries*2 i64 slots (each entry = key slot + value slot)
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    local_get(code, idx);
    local_get(code, src_local);
    i32_load(code, 0);
    i32_const(code, 2);
    code.push(0x6C); // num_entries * 2
    code.push(0x4E); // idx >= num*2
    code.push(0x0D);
    leb_u32(code, 1); // br_if block
    local_get(code, data);
    local_get(code, idx);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A); // data + idx*8
    local_get(code, src_local);
    i32_load(code, 8);
    local_get(code, idx);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A); // src_data + idx*8
    i64_load(code, 0);
    i64_store(code, 0);
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    for off in [0u32, 4] {
        local_get(code, hdr);
        local_get(code, src_local);
        i32_load(code, 0);
        i32_store(code, off);
    }
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8);
}

/// After a nested sequence's OUTER handle buffer has been flat-cloned into `data` (its element count
/// in `outer_hdr`'s word 0), replace each element handle with a deep clone of the value it points to,
/// so the clone owns independent inner sequences (a flat copy would share them). `is_text` selects
/// the inner copy stride. A runtime loop over the (statically-unknown) element count; scratch
/// `counter` plus `isrc`/`ihdr`/`idata`/`iidx` (the inner buffer clone) are all disjoint from
/// `outer_hdr`/`data`.
#[allow(clippy::too_many_arguments)]
fn emit_clone_each_element(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    outer_hdr: u32,
    data: u32,
    counter: u32,
    isrc: u32,
    ihdr: u32,
    idata: u32,
    iidx: u32,
    is_text: bool,
) {
    i32_const(code, 0);
    local_set(code, counter);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    local_get(code, counter);
    local_get(code, outer_hdr);
    i32_load(code, 0); // element count
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if exit
    // isrc = data[counter*8] — the inner handle the flat copy duplicated
    local_get(code, data);
    local_get(code, counter);
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    code.push(0x6A); // i32.add
    i32_load(code, 0);
    local_set(code, isrc);
    emit_buffer_clone(code, ctx, ihdr, idata, iidx, isrc, is_text);
    // data[counter*8] = ihdr (the fresh, independent inner handle)
    local_get(code, data);
    local_get(code, counter);
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    code.push(0x6A); // i32.add
    local_get(code, ihdr);
    i32_store(code, 0);
    // counter++
    local_get(code, counter);
    i32_const(code, 1);
    code.push(0x6A); // i32.add
    local_set(code, counter);
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
}

/// `a copy of x` (`DeepClone`) — an independent deep copy. A scalar is its own value (copied); a
/// `Text` gets a fresh byte buffer; a sequence/struct of trivially-cloneable (scalar) elements gets
/// a fresh element buffer. The new buffer means a later mutation of the clone (or the original)
/// cannot be seen through the other — value semantics, matching the tree-walker's `deep_clone`. A
/// composite holding HANDLE sub-values (a struct with a Text/Seq field, a Seq of handles) needs a
/// recursive clone (a generated per-type helper); deferred — soundly rejected, never miscopied.
fn lower_deep_clone(code: &mut Vec<u8>, kinds: &KindTable, structs: &kind::StructLayout, ctx: &Ctx, num_regs: u32, dst: u16, src: u16) -> R<()> {
    let (hdr, data, idx) = (num_regs + 5, num_regs + 6, num_regs + 7);
    let s = src as u32;
    match kinds.get(src as usize) {
        // A scalar value is immutable — the clone is just the value.
        Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Float) | Some(Kind::Date) | Some(Kind::Moment) | Some(Kind::Duration) | Some(Kind::Time) | Some(Kind::Span) => {
            local_get(code, s);
            local_set(code, dst as u32);
            return Ok(());
        }
        // A struct whose fields are scalars and/or clonable handles (`Text`/`Set`/scalar-sequence):
        // clone the header + the field buffer (8-byte slots, count = `num_fields`), then RECURSIVELY
        // clone each handle field one level so the clone owns independent sub-buffers — a flat copy
        // would SHARE a mutable inner sequence, defeating value semantics. Scalar fields are already
        // independent after the flat copy. The field layout flows to `dst` (see `struct_layout`), so
        // `clone's field` still resolves.
        Some(Kind::Struct)
            if structs
                .reg_layout
                .get(&src)
                .is_some_and(|l| l.iter().all(|&(_, vr)| field_clone_ok(kinds.get(vr as usize)))) =>
        {
            i32_const(code, 16);
            emit_alloc(code, ctx,hdr);
            local_get(code, s);
            i32_load(code, 0); // num_fields
            i32_const(code, 8);
            code.push(0x6C); // i32.mul
            emit_alloc(code, ctx,data);
            emit_seq_copy(code, idx, data, s, s, false); // flat-copy num_fields 8-byte slots
            for off in [0u32, 4] {
                local_get(code, hdr);
                local_get(code, s);
                i32_load(code, 0);
                i32_store(code, off);
            }
            local_get(code, hdr);
            local_get(code, data);
            i32_store(code, 8);
            // Recursively clone each handle field IN PLACE in the cloned buffer (`data`), overwriting
            // its flat-copied shared handle with a fresh one. Scratch `+8..+11` is disjoint from the
            // struct's `+5..+7`. The layout is compile-time, so this unrolls per handle field.
            let (fsrc, fhdr, fdata, fidx) = (num_regs + 8, num_regs + 9, num_regs + 10, num_regs + 11);
            let layout = structs.reg_layout.get(&src).expect("guarded by is_some_and above");
            for (slot, &(_, vr)) in layout.iter().enumerate() {
                let fk = kinds.get(vr as usize);
                if is_clone_trivial(fk) {
                    continue; // scalar — the flat copy already produced an independent value
                }
                let off = slot as u32 * 8;
                local_get(code, data);
                i32_load(code, off); // the shared handle the flat copy duplicated
                local_set(code, fsrc);
                emit_buffer_clone(code, ctx, fhdr, fdata, fidx, fsrc, fk == Some(Kind::Text));
                local_get(code, data);
                local_get(code, fhdr); // the fresh, independent handle
                i32_store(code, off);
            }
            local_get(code, hdr);
            local_set(code, dst as u32);
            return Ok(());
        }
        // Text → fresh byte buffer; a sequence or `Set of Int` → fresh 8-byte-element buffer (a Set
        // is a flat scalar buffer like `SeqInt`, so the same copy is an independent deep clone).
        // A `SetText` clones like a `Set` (flat 8-byte-slot copy) — the shared `Text` element handles
        // are immutable in Logos, so the clone sharing them is sound (no deeper recursion needed).
        Some(k @ (Kind::Text | Kind::SeqInt | Kind::SeqBool | Kind::SeqFloat | Kind::SeqAny | Kind::Set | Kind::SetText)) => {
            emit_buffer_clone(code, ctx, hdr, data, idx, s, k == Kind::Text);
            local_get(code, hdr);
            local_set(code, dst as u32);
            Ok(())
        }
        // A Map — its `[key][value]` entry buffer flat-copied into a fresh, independent map.
        Some(Kind::Map) => {
            emit_map_clone(code, ctx, hdr, data, idx, s);
            local_get(code, hdr);
            local_set(code, dst as u32);
            Ok(())
        }
        // A Seq of Seq (an Int matrix): clone the outer handle buffer, then clone EACH inner sequence
        // so the rows are independent (a flat copy would share the row handles). `idx` is free again
        // after the outer copy, so it doubles as the per-row loop counter; `+8..+11` clone each row.
        Some(Kind::SeqSeqInt) => {
            emit_buffer_clone(code, ctx, hdr, data, idx, s, false);
            let (isrc, ihdr, idata, iidx) = (num_regs + 8, num_regs + 9, num_regs + 10, num_regs + 11);
            emit_clone_each_element(code, ctx, hdr, data, idx, isrc, ihdr, idata, iidx, false);
            local_get(code, hdr);
            local_set(code, dst as u32);
            Ok(())
        }
        _ => Err(WasmLowerError::Unsupported("deep clone of an unsupported value kind")),
    }
}

/// `lhs equals rhs` / `lhs is not rhs` on two `Text` values — byte equality (unequal length ⇒ not
/// equal, else compare bytes). Result is a `Bool` i64 0/1 in `dst` (`negate` flips it for `!=`).
fn lower_text_eq(code: &mut Vec<u8>, num_regs: u32, dst: u16, lhs: u16, rhs: u16, negate: bool) {
    let (a, b) = (lhs as u32, rhs as u32);
    let idx = num_regs + 7;
    // dst = 1 (assume equal)
    code.push(0x42);
    leb_i64(code, 1);
    local_set(code, dst as u32);
    // if len_a != len_b → not equal; else compare bytes
    local_get(code, a);
    i32_load(code, 0);
    local_get(code, b);
    i32_load(code, 0);
    code.push(0x47); // i32.ne
    code.push(0x04);
    code.push(0x40); // if (lengths differ)
    code.push(0x42);
    leb_i64(code, 0);
    local_set(code, dst as u32); // dst = 0
    code.push(0x05); // else (same length)
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    local_get(code, idx);
    local_get(code, a);
    i32_load(code, 0);
    code.push(0x4E); // i32.ge_s → idx >= len
    code.push(0x0D);
    leb_u32(code, 1); // br_if block (all matched)
    // a[idx] != b[idx] ?
    local_get(code, a);
    i32_load(code, 8);
    local_get(code, idx);
    code.push(0x6A);
    i32_load8_u(code, 0);
    local_get(code, b);
    i32_load(code, 8);
    local_get(code, idx);
    code.push(0x6A);
    i32_load8_u(code, 0);
    code.push(0x47); // i32.ne
    code.push(0x04);
    code.push(0x40); // if (mismatch)
    code.push(0x42);
    leb_i64(code, 0);
    local_set(code, dst as u32); // dst = 0
    code.push(0x0C);
    leb_u32(code, 2); // br block (out of inner-if → loop → block)
    code.push(0x0B); // end inner if
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    code.push(0x0B); // end outer if
    if negate {
        local_get(code, dst as u32);
        code.push(0x50); // i64.eqz → i32
        code.push(0xAD); // i64.extend_i32_u
        local_set(code, dst as u32);
    }
}

/// Materialize the value in local `src` (of kind `kind`) as a `Text` handle stored in local `out`.
/// A `Text` is itself (just copied); an `Int`/`Float`/`Bool` is formatted into a fresh buffer by
/// the matching host formatter (which returns the byte length) and wrapped in a header. Uses the
/// +5/+6/+7 scratch as temps. Value-based (a local + kind, not a register) so it serves both
/// `Concat`'s operands and `Show`'s slot-loaded struct fields.
fn emit_stringify(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, src: u32, kind: Option<Kind>, out: u32) -> R<()> {
    match kind {
        Some(Kind::Text) => {
            local_get(code, src);
            local_set(code, out);
        }
        // A `BigInt` operand of a concat (`"x = " + (2^200)`): render it to a decimal `Text` via the
        // runtime, then stringify AS that Text — matching the VM, which appends the decimal to the text.
        Some(Kind::BigInt) => {
            let to_text = (ctx.host_index)(HostFn::BigintToText).ok_or(WasmLowerError::Unsupported("bigint_to_text not imported"))?;
            local_get(code, src);
            code.push(0x10); // call
            leb_u32(code, to_text);
            local_set(code, out);
        }
        Some(k @ (Kind::Int | Kind::Float | Kind::Bool)) => {
            // Stringify a scalar via the matching host formatter writing into a fresh buffer.
            let (host, bufsize) = match k {
                Kind::Int => (HostFn::FmtI64Into, 24), // ≤ 20 decimal digits + sign
                Kind::Float => (HostFn::FmtF64Into, 340), // worst-case shortest-round-trip f64 width (~326)
                _ => (HostFn::FmtBoolInto, 8), // "true"/"false"
            };
            let (h, data, tmp) = (num_regs + 5, num_regs + 6, num_regs + 7);
            i32_const(code, 16);
            emit_alloc(code, ctx,h);
            i32_const(code, bufsize);
            emit_alloc(code, ctx,data);
            // len = fmt_*_into(data, src)
            let fidx = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("text formatter not imported"))?;
            local_get(code, data);
            local_get(code, src);
            code.push(0x10); // call
            leb_u32(code, fidx);
            local_set(code, tmp); // tmp = len
            // header: len = cap = tmp; data_ptr = data
            local_get(code, h);
            local_get(code, tmp);
            i32_store(code, 0);
            local_get(code, h);
            local_get(code, tmp);
            i32_store(code, 4);
            local_get(code, h);
            local_get(code, data);
            i32_store(code, 8);
            local_get(code, h);
            local_set(code, out);
        }
        // A whole `Seq of Int` / `Set of Int` operand — the host formats `[e0, …]` / `{e0, …}` out of
        // linear memory into a buffer sized from the collection's length (`len*24 + 8`, worst-case i64
        // decimal + `", "` + brackets), which is then wrapped in a Text header. Matches
        // `RuntimeValue::List`/`Set::to_display_string` (insertion order = the AOT's storage order).
        Some(k @ (Kind::SeqInt | Kind::SeqBool | Kind::SeqAny | Kind::Set)) => {
            let host = match k {
                Kind::Set => HostFn::FmtSetI64Into,
                Kind::SeqBool => HostFn::FmtSeqBoolInto, // renders `[true, false, …]` (`len*24+8` is ample)
                _ => HostFn::FmtSeqI64Into,
            };
            let (h, data, tmp) = (num_regs + 5, num_regs + 6, num_regs + 7);
            i32_const(code, 16);
            emit_alloc(code, ctx,h);
            // data = alloc(len*24 + 8)
            local_get(code, src);
            i32_load(code, 0); // len
            i32_const(code, 24);
            code.push(0x6C); // i32.mul
            i32_const(code, 8);
            code.push(0x6A); // + 8
            emit_alloc(code, ctx,data);
            // len = fmt_{seq,set}_i64_into(data, src)
            let fidx = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("collection formatter not imported"))?;
            local_get(code, data);
            local_get(code, src);
            code.push(0x10); // call
            leb_u32(code, fidx);
            local_set(code, tmp);
            local_get(code, h);
            local_get(code, tmp);
            i32_store(code, 0);
            local_get(code, h);
            local_get(code, tmp);
            i32_store(code, 4);
            local_get(code, h);
            local_get(code, data);
            i32_store(code, 8);
            local_get(code, h);
            local_set(code, out);
        }
        _ => return Err(WasmLowerError::Unsupported("concat operand kind cannot be stringified yet")),
    }
    Ok(())
}

/// Build the display `Text` of the struct in `handle` (type `def`) into `out` — `TypeName { f: v, … }`
/// (`RuntimeValue::Struct::to_display_string`) with the fields in DETERMINISTIC alphabetical order (the
/// VM sorts its `HashMap` fields by name; this sorts the DECLARED fields the same way). Each field's
/// value is loaded from its declared slot (`data_ptr + slot*8`) at the field type's width and
/// stringified. An empty struct is just its name. `part`/`field_i32` are caller-supplied scratch kept
/// distinct from `out`. The reusable core of [`lower_show_struct`] and [`lower_show_seqstruct`].
fn emit_struct_display(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, def: &StructTypeDef, handle: u32, out: u32, part: u32, field_i32: u32) -> R<()> {
    let mut order: Vec<usize> = (0..def.fields.len()).collect();
    order.sort_by(|&a, &b| def.fields[a].0.cmp(&def.fields[b].0));
    if def.fields.is_empty() {
        lower_text_literal(code, ctx, num_regs, def.name.as_bytes());
        local_set(code, out);
        return Ok(());
    }
    lower_text_literal(code, ctx, num_regs, format!("{} {{ ", def.name).as_bytes());
    local_set(code, out);
    let n = order.len();
    for (j, &i) in order.iter().enumerate() {
        let (fname, bt) = &def.fields[i];
        let ek = kind::boundary_to_kind(bt);
        lower_text_literal(code, ctx, num_regs, format!("{fname}: ").as_bytes());
        local_set(code, part);
        emit_text_concat(code, ctx, num_regs, out, part, out);
        // load field `i`'s slot value (declared slot = `i`, at `data_ptr + i*8`)
        let elem_tmp = match ek.map(Kind::wasm_valtype) {
            Some(F64) => num_regs + 12,
            Some(I64) => num_regs + 1,
            _ => field_i32,
        };
        local_get(code, handle);
        i32_load(code, 8); // data_ptr
        emit_slot_load(code, ek, (i as u32) * 8)?;
        local_set(code, elem_tmp);
        emit_stringify(code, ctx, num_regs, elem_tmp, ek, part)?;
        emit_text_concat(code, ctx, num_regs, out, part, out);
        if j + 1 < n {
            lower_text_literal(code, ctx, num_regs, b", ");
            local_set(code, part);
            emit_text_concat(code, ctx, num_regs, out, part, out);
        }
    }
    lower_text_literal(code, ctx, num_regs, b" }");
    local_set(code, part);
    emit_text_concat(code, ctx, num_regs, out, part, out);
    Ok(())
}

fn lower_show_struct(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, src: u16) -> R<()> {
    let type_name = plan
        .structs
        .struct_name_of
        .get(&src)
        .ok_or(WasmLowerError::Unsupported("Show of a struct whose type is not statically known"))?;
    let def = ctx
        .struct_types
        .iter()
        .find(|s| &s.name == type_name)
        .ok_or(WasmLowerError::Unsupported("Show of an unknown struct type"))?;
    let print_idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("Show struct: print_text not imported"))?;
    let num_regs = plan.num_regs;
    let out = num_regs + 8;
    emit_struct_display(code, ctx, num_regs, def, src as u32, out, num_regs + 9, num_regs + 10)?;
    local_get(code, out);
    code.push(0x10); // call print_text
    leb_u32(code, print_idx);
    Ok(())
}

/// `Show <Seq of Struct>` — `[TypeName { … }, …]` over the sequence in insertion order, each element
/// rendered by [`emit_struct_display`]. A RUNTIME loop: element `i` is an i32 struct handle at
/// `data_ptr+i*8`; its display is built into `out` and concatenated onto the outer `[…]` accumulator.
/// The element struct type comes from `seq_elem_struct_name` (the homogeneous list's element).
fn lower_show_seqstruct(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, src: u16) -> R<()> {
    let type_name = plan
        .structs
        .seq_elem_struct_name
        .get(&src)
        .ok_or(WasmLowerError::Unsupported("Show of a Seq of Struct whose element type is not statically known"))?;
    let def = ctx
        .struct_types
        .iter()
        .find(|s| &s.name == type_name)
        .ok_or(WasmLowerError::Unsupported("Show seq-struct: unknown element struct type"))?;
    let print_idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("Show seq-struct: print_text not imported"))?;
    let num_regs = plan.num_regs;
    let m = src as u32;
    let (outer_acc, out, i, elem) = (num_regs + 8, num_regs + 9, num_regs + 10, num_regs + 11);
    let (part, field_i32) = (num_regs + 13, num_regs + 14);
    lower_text_literal(code, ctx, num_regs, b"[");
    local_set(code, outer_acc);
    i32_const(code, 0);
    local_set(code, i);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    local_get(code, i);
    local_get(code, m);
    i32_load(code, 0);
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1);
    // separator ", " before every element after the first
    local_get(code, i);
    code.push(0x45);
    code.push(0x04);
    code.push(0x40);
    code.push(0x05);
    lower_text_literal(code, ctx, num_regs, b", ");
    local_set(code, part);
    emit_text_concat(code, ctx, num_regs, outer_acc, part, outer_acc);
    code.push(0x0B); // end if
    // elem = seq element i (i32 struct handle at data_ptr + i*8)
    local_get(code, m);
    i32_load(code, 8);
    local_get(code, i);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    i32_load(code, 0);
    local_set(code, elem);
    emit_struct_display(code, ctx, num_regs, def, elem, out, part, field_i32)?;
    emit_text_concat(code, ctx, num_regs, outer_acc, out, outer_acc);
    local_get(code, i);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, i);
    code.push(0x0C);
    leb_u32(code, 0);
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    lower_text_literal(code, ctx, num_regs, b"]");
    local_set(code, part);
    emit_text_concat(code, ctx, num_regs, outer_acc, part, outer_acc);
    local_get(code, outer_acc);
    code.push(0x10); // call print_text
    leb_u32(code, print_idx);
    Ok(())
}

/// `Show <Seq of Seq of Int>` — `[[…], […]]` over the outer sequence in stored (insertion) order,
/// matching the VM's `RuntimeValue::List` of `List`s. A RUNTIME loop (outer length is dynamic): each
/// outer element is an `i32` handle (low word of its 8-byte slot) to an inner `Seq of Int`, which the
/// scalar seq formatter (`emit_stringify` of `Kind::SeqInt` → `fmt_seq_i64_into`) renders as `[e0, …]`;
/// the outer wraps them in `[…]` with `", "` separators. Byte-identical to the VM's nested display.
fn lower_show_seqseq(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, src: u16) -> R<()> {
    let print_idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("Show seq-of-seq: print_text not imported"))?;
    let m = src as u32;
    // acc = accumulator Text; part = each inner render; i = outer loop counter; inner = the inner
    // `Seq of Int` handle (all outside the +5/+6/+7 scratch `emit_stringify`/concat clobber).
    let (acc, part, i, inner) = (num_regs + 8, num_regs + 9, num_regs + 10, num_regs + 11);
    // acc = "["
    lower_text_literal(code, ctx, num_regs, b"[");
    local_set(code, acc);
    i32_const(code, 0);
    local_set(code, i);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    // if i >= outer_len: br block
    local_get(code, i);
    local_get(code, m);
    i32_load(code, 0); // outer len
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if block
    // separator: entries after the first get ", "
    local_get(code, i);
    code.push(0x45); // i32.eqz
    code.push(0x04);
    code.push(0x40); // if (i == 0): nothing
    code.push(0x05); // else
    lower_text_literal(code, ctx, num_regs, b", ");
    local_set(code, part);
    emit_text_concat(code, ctx, num_regs, acc, part, acc);
    code.push(0x0B); // end if
    // inner = outer element i (i32 handle at data_ptr + i*8)
    local_get(code, m);
    i32_load(code, 8); // data_ptr
    local_get(code, i);
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    code.push(0x6A); // i32.add
    i32_load(code, 0); // the inner Seq handle
    local_set(code, inner);
    // part = stringify(inner as Seq of Int); acc += part
    emit_stringify(code, ctx, num_regs, inner, Some(Kind::SeqInt), part)?;
    emit_text_concat(code, ctx, num_regs, acc, part, acc);
    // i += 1; br loop
    local_get(code, i);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, i);
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    // acc += "]"; print_text(acc)
    lower_text_literal(code, ctx, num_regs, b"]");
    local_set(code, part);
    emit_text_concat(code, ctx, num_regs, acc, part, acc);
    local_get(code, acc);
    code.push(0x10); // call print_text
    leb_u32(code, print_idx);
    Ok(())
}

/// `Show <map>` — `RuntimeValue::Map::to_display_string` = `{k0: v0, k1: v1, …}` over the map's
/// entries in STORED order, which is INSERTION order (the VM's `MapStorage` is an `IndexMap`, the same
/// order the AOT's linear map appends in), so the rendering is byte-identical. Unlike the tuple/enum
/// Show this is a RUNTIME loop (the entry count is dynamic): iterate `i` in `0..num_entries`, and for
/// each entry `[key@0][value@8]` (16 bytes) stringify the key and value by their kinds and concat
/// `k: v` onto the accumulator (with `", "` between entries). Key/value kinds come from the last
/// `SetIndex`'s registers (`map_set_key`/`map_set_value`) — a LOCALLY-BUILT map with Int/Text keys.
fn lower_show_map(code: &mut Vec<u8>, plan: &Plan, kinds: &KindTable, ctx: &Ctx, src: u16) -> R<()> {
    let key_reg = plan
        .structs
        .map_set_key
        .get(&src)
        .copied()
        .ok_or(WasmLowerError::Unsupported("Show of a map whose key kind is not statically known"))?;
    let val_reg = plan
        .structs
        .map_set_value
        .get(&src)
        .copied()
        .ok_or(WasmLowerError::Unsupported("Show of a map whose value kind is not statically known"))?;
    let key_kind = kinds.get(key_reg as usize);
    let val_kind = kinds.get(val_reg as usize);
    let key_text = match key_kind {
        Some(Kind::Text) => true,
        Some(Kind::Int) => false,
        _ => return Err(WasmLowerError::Unsupported("Show of a map with a non-Int/Text key")),
    };
    let val_load = map_value_load(val_kind)?; // the value's slot load at its kind's width
    let print_idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("Show map: print_text not imported"))?;
    let num_regs = plan.num_regs;
    let m = src as u32;
    // acc = accumulator Text; part = each stringified piece; i = the entry loop counter. Key/value
    // temps borrow width-matched scratch (idle during a Show): the key is fully stringified before the
    // value is loaded, so they may share a slot.
    let (acc, part, i) = (num_regs + 8, num_regs + 9, num_regs + 10);
    let key_tmp = if key_text { num_regs + 11 } else { num_regs + 1 };
    let val_tmp = match val_kind.map(Kind::wasm_valtype) {
        Some(F64) => num_regs + 12,
        Some(I64) => num_regs + 1,
        _ => num_regs + 11,
    };
    // acc = "{"
    lower_text_literal(code, ctx, num_regs, b"{");
    local_set(code, acc);
    // i = 0
    i32_const(code, 0);
    local_set(code, i);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    // if i >= num_entries: br block
    local_get(code, i);
    local_get(code, m);
    i32_load(code, 0); // num_entries
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if block
    // separator: entries after the first are prefixed with ", "
    local_get(code, i);
    code.push(0x45); // i32.eqz → i == 0 ?
    code.push(0x04);
    code.push(0x40); // if (i == 0): nothing
    code.push(0x05); // else (i != 0)
    lower_text_literal(code, ctx, num_regs, b", ");
    local_set(code, part);
    emit_text_concat(code, ctx, num_regs, acc, part, acc);
    code.push(0x0B); // end if
    // key = entry[i].key (offset 0), stringified, appended
    emit_map_entry_addr(code, m, i);
    if key_text {
        i32_load(code, 0);
    } else {
        i64_load(code, 0);
    }
    local_set(code, key_tmp);
    emit_stringify(code, ctx, num_regs, key_tmp, key_kind, part)?;
    emit_text_concat(code, ctx, num_regs, acc, part, acc);
    // ": "
    lower_text_literal(code, ctx, num_regs, b": ");
    local_set(code, part);
    emit_text_concat(code, ctx, num_regs, acc, part, acc);
    // value = entry[i].value (offset 8), stringified, appended
    emit_map_entry_addr(code, m, i);
    val_load(code, 8);
    local_set(code, val_tmp);
    emit_stringify(code, ctx, num_regs, val_tmp, val_kind, part)?;
    emit_text_concat(code, ctx, num_regs, acc, part, acc);
    // i += 1
    local_get(code, i);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, i);
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    // acc += "}"; print_text(acc)
    lower_text_literal(code, ctx, num_regs, b"}");
    local_set(code, part);
    emit_text_concat(code, ctx, num_regs, acc, part, acc);
    local_get(code, acc);
    code.push(0x10); // call print_text
    leb_u32(code, print_idx);
    Ok(())
}

/// `Show tuple` — the tree-walker displays a heterogeneous tuple as `(e0, e1, …)`, each element by
/// its own scalar display. The tuple layout (element registers → kinds, via `structs.tuple_layouts`)
/// is known at compile time, so this UNROLLS: build the `Text` `"("`, then for each element load its
/// 8-byte slot, stringify it, and concat it onto the accumulator (with a `", "` separator between
/// elements), close with `")"`, and `print_text` the assembled string — byte-identical to
/// `RuntimeValue::Tuple::to_display_string`. Deterministic (tuple element order is fixed), unlike a
/// struct/map whose display order the tree-walker randomizes (hence those stay deferred). The
/// accumulator/separator live in the `+8`/`+9` handle scratch (which `emit_text_concat` preserves);
/// the element value is loaded into the `+1` (i64), `+12` (f64), or `+10` (i32-handle) temp by width.
fn lower_show_tuple(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, src: u16) -> R<()> {
    let elems = plan
        .structs
        .tuple_layouts
        .get(&src)
        .ok_or(WasmLowerError::Unsupported("Show of a tuple with no static layout"))?;
    let num_regs = plan.num_regs;
    let (acc, part) = (num_regs + 8, num_regs + 9);
    // acc = "("
    lower_text_literal(code, ctx, num_regs, b"(");
    local_set(code, acc);
    let n = elems.len();
    for (i, &elem_reg) in elems.iter().enumerate() {
        let ek = plan.kinds.get(elem_reg as usize);
        // Load slot i (the element value at its width) into a matching-typed temp local.
        let elem_tmp = match ek.map(Kind::wasm_valtype) {
            Some(F64) => num_regs + 12, // f64 temp
            Some(I64) => num_regs + 1,  // i64 temp (borrows a pow scratch — idle during Show)
            _ => num_regs + 10,         // i32 handle temp (Text/…)
        };
        local_get(code, src as u32);
        i32_load(code, 8); // data_ptr
        emit_slot_load(code, ek, (i as u32) * 8)?;
        local_set(code, elem_tmp);
        // part = stringify(element); acc = acc + part
        emit_stringify(code, ctx, num_regs, elem_tmp, ek, part)?;
        emit_text_concat(code, ctx, num_regs, acc, part, acc);
        // ", " between elements
        if i + 1 < n {
            lower_text_literal(code, ctx, num_regs, b", ");
            local_set(code, part);
            emit_text_concat(code, ctx, num_regs, acc, part, acc);
        }
    }
    // acc = acc + ")"
    lower_text_literal(code, ctx, num_regs, b")");
    local_set(code, part);
    emit_text_concat(code, ctx, num_regs, acc, part, acc);
    // print_text(acc)
    let idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("Show tuple: print_text not imported"))?;
    local_get(code, acc);
    code.push(0x10); // call print_text
    leb_u32(code, idx);
    Ok(())
}

/// `Show <enum>` — a nullary variant displays as just its constructor name (`RuntimeValue::Inductive`
/// with empty args → `ind.constructor.clone()`). The enum handle's first word is the TAG (the
/// constructor name's constant index; `NewInductive`'s `ctor = add_const(Text(name))`), so this emits
/// a tag→name dispatch: for each variant of `src`'s enum type (resolved via `ind_type_of` →
/// `enum_types`), `if stored_tag == const_idx(name) { print_text name }`. Exactly one branch matches
/// (the live variant), so exactly one name prints — byte-identical to the tree-walker. Restricted to
/// ALL-NULLARY enum types; a payload variant (`Ctor(args)`) display is a later increment (soundly
/// refused, so such a Show stays deferred rather than miscompiling).
/// Build the display `Text` of the enum value in `handle` (its type `def`) into `out` — a nullary
/// variant renders as its constructor name, a payload variant as `Ctor(f0, f1, …)`
/// (`format!("{}({})", ctor, join(", "))`), matching `RuntimeValue::Inductive::to_display_string`. A
/// tag→name dispatch (`stored_tag == const_idx(name)`) selects the live variant; exactly one matches,
/// so `out` is always written. `part`/`field_i32` are scratch the CALLER must keep distinct from `out`
/// and (for the sequence case) from its outer accumulator/counter/handle. The reusable core of
/// [`lower_show_enum`] (a scalar `Show`) and [`lower_show_seqenum`] (per element of a `Seq of Enum`).
fn emit_enum_display(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, def: &EnumTypeDef, handle: u32, out: u32, part: u32, field_i32: u32) -> R<()> {
    for v in &def.variants {
        // The variant's tag = the constant-pool index of its name `Text` (constant dedup makes this the
        // exact value `NewInductive` stored). A variant NEVER constructed anywhere is absent from the
        // pool — it can't be the runtime value, so it needs no branch.
        let Some(tag) = ctx.constants.iter().position(|c| matches!(c, Constant::Text(n) if *n == v.name)) else {
            continue;
        };
        let tag = tag as i32;
        local_get(code, handle);
        i32_load(code, 0); // the enum handle's stored tag
        i32_const(code, tag);
        code.push(0x46); // i32.eq → stored_tag == variant_tag
        code.push(0x04);
        code.push(0x40); // if (void)
        // out = the constructor name (the whole display for a nullary variant)
        lower_text_literal(code, ctx, num_regs, v.name.as_bytes());
        local_set(code, out);
        if !v.field_types.is_empty() {
            // Append `(f0, f1, …)` — payload slots are stored INLINE after the tag at offset `8*(1+i)`.
            lower_text_literal(code, ctx, num_regs, b"(");
            local_set(code, part);
            emit_text_concat(code, ctx, num_regs, out, part, out);
            let n = v.field_types.len();
            for (i, ft) in v.field_types.iter().enumerate() {
                let ek = kind::boundary_to_kind(ft);
                let elem_tmp = match ek.map(Kind::wasm_valtype) {
                    Some(F64) => num_regs + 12,
                    Some(I64) => num_regs + 1,
                    _ => field_i32,
                };
                local_get(code, handle);
                emit_slot_load(code, ek, 8 * (1 + i as u32))?;
                local_set(code, elem_tmp);
                emit_stringify(code, ctx, num_regs, elem_tmp, ek, part)?;
                emit_text_concat(code, ctx, num_regs, out, part, out);
                if i + 1 < n {
                    lower_text_literal(code, ctx, num_regs, b", ");
                    local_set(code, part);
                    emit_text_concat(code, ctx, num_regs, out, part, out);
                }
            }
            lower_text_literal(code, ctx, num_regs, b")");
            local_set(code, part);
            emit_text_concat(code, ctx, num_regs, out, part, out);
        }
        code.push(0x0B); // end if
    }
    Ok(())
}

fn lower_show_enum(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, src: u16) -> R<()> {
    let type_name = plan
        .structs
        .ind_type_of
        .get(&src)
        .ok_or(WasmLowerError::Unsupported("Show of an enum whose type is not statically known"))?;
    let def = ctx
        .enum_types
        .iter()
        .find(|e| &e.name == type_name)
        .ok_or(WasmLowerError::Unsupported("Show of an unknown enum type"))?;
    let print_idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("Show enum: print_text not imported"))?;
    let num_regs = plan.num_regs;
    // out = the assembled display; +9/+10 are the piece + field-i32 scratch (the classic `Show` pool).
    let out = num_regs + 8;
    emit_enum_display(code, ctx, num_regs, def, src as u32, out, num_regs + 9, num_regs + 10)?;
    local_get(code, out);
    code.push(0x10); // call print_text
    leb_u32(code, print_idx);
    Ok(())
}

/// `Show <Seq of Enum>` — `[e0, e1, …]` over the sequence in insertion order, each element rendered by
/// [`emit_enum_display`] (nullary name or `Ctor(fields)`). A RUNTIME loop: element `i` is an i32 enum
/// handle at `data_ptr+i*8`; its display is built into `out` and concatenated onto the outer `[…]`
/// accumulator. The element enum type comes from `seq_elem_ind_type` (the homogeneous list's element).
fn lower_show_seqenum(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, src: u16) -> R<()> {
    let type_name = plan
        .structs
        .seq_elem_ind_type
        .get(&src)
        .ok_or(WasmLowerError::Unsupported("Show of a Seq of Enum whose element type is not statically known"))?;
    let def = ctx
        .enum_types
        .iter()
        .find(|e| &e.name == type_name)
        .ok_or(WasmLowerError::Unsupported("Show seq-enum: unknown element enum type"))?;
    let print_idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported("Show seq-enum: print_text not imported"))?;
    let num_regs = plan.num_regs;
    let m = src as u32;
    // Outer loop: `outer_acc` = `[…]`, `i` = counter, `elem` = the current enum handle. The per-element
    // display goes to `out`, using `part`/`field_i32` (+13/+14) kept distinct from all of the above.
    let (outer_acc, out, i, elem) = (num_regs + 8, num_regs + 9, num_regs + 10, num_regs + 11);
    let (part, field_i32) = (num_regs + 13, num_regs + 14);
    lower_text_literal(code, ctx, num_regs, b"[");
    local_set(code, outer_acc);
    i32_const(code, 0);
    local_set(code, i);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    // if i >= len: br block
    local_get(code, i);
    local_get(code, m);
    i32_load(code, 0);
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1);
    // separator ", " before every element after the first
    local_get(code, i);
    code.push(0x45); // i32.eqz
    code.push(0x04);
    code.push(0x40);
    code.push(0x05); // if (i==0) {} else
    lower_text_literal(code, ctx, num_regs, b", ");
    local_set(code, part);
    emit_text_concat(code, ctx, num_regs, outer_acc, part, outer_acc);
    code.push(0x0B); // end if
    // elem = seq element i (i32 enum handle at data_ptr + i*8)
    local_get(code, m);
    i32_load(code, 8);
    local_get(code, i);
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    code.push(0x6A); // i32.add
    i32_load(code, 0);
    local_set(code, elem);
    // out = display(elem); outer_acc += out
    emit_enum_display(code, ctx, num_regs, def, elem, out, part, field_i32)?;
    emit_text_concat(code, ctx, num_regs, outer_acc, out, outer_acc);
    // i += 1; br loop
    local_get(code, i);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, i);
    code.push(0x0C);
    leb_u32(code, 0);
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    lower_text_literal(code, ctx, num_regs, b"]");
    local_set(code, part);
    emit_text_concat(code, ctx, num_regs, outer_acc, part, outer_acc);
    local_get(code, outer_acc);
    code.push(0x10); // call print_text
    leb_u32(code, print_idx);
    Ok(())
}

/// The (slot index, boundary type) of `field_name` in struct type `type_name`, from the DECLARED
/// field order (= the AOT's 8-byte-slot storage order). `None` if the type or field is unknown.
fn policy_field_slot<'a>(ctx: &'a Ctx, type_name: &str, field_name: &str) -> Option<(u16, &'a BoundaryType)> {
    let def = ctx.struct_types.iter().find(|s| s.name == type_name)?;
    def.fields.iter().position(|(n, _)| n == field_name).map(|i| (i as u16, &def.fields[i].1))
}

/// Load struct `reg`'s Text field (an `i32` handle in the low word of slot `slot`) into `dst_local`.
fn emit_load_text_field(code: &mut Vec<u8>, reg: u16, slot: u16, dst_local: u32) {
    local_get(code, reg as u32);
    i32_load(code, 8); // data_ptr
    i32_load(code, u32::from(slot) * 8); // the Text handle at slot*8
    local_set(code, dst_local);
}

/// Compile a policy `condition` against `subject` (and optional `object`, `u16::MAX` = none) into an
/// i32 (1 = holds, 0 = fails) left on the wasm stack — mirroring `evaluate_policy_condition`. Only the
/// Text-field / predicate / and-or / cross-field forms are lowered (they cover the shipping policies);
/// a numeric / boolean field compare is soundly refused (the `CheckPolicy` then stays deferred).
fn emit_policy_condition(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, cond: &PolicyCondition, subject: u16, object: u16) -> R<()> {
    let (f8, f11) = (plan.num_regs + 8, plan.num_regs + 11);
    match cond {
        PolicyCondition::FieldEquals { field, value, is_string_literal } => {
            if !is_string_literal {
                return Err(WasmLowerError::Unsupported("policy: non-Text field comparison"));
            }
            let subj_type = plan.structs.struct_name_of.get(&subject).ok_or(WasmLowerError::Unsupported("policy: subject has no struct type"))?;
            let field_name = ctx.interner.resolve(*field);
            let (slot, bt) = policy_field_slot(ctx, subj_type, field_name).ok_or(WasmLowerError::Unsupported("policy: field not in subject type"))?;
            if !matches!(bt, BoundaryType::Text) {
                return Err(WasmLowerError::Unsupported("policy: field is not Text"));
            }
            let value_str = ctx.interner.resolve(*value).to_string();
            emit_load_text_field(code, subject, slot, f8);
            lower_text_literal(code, ctx, plan.num_regs, value_str.as_bytes());
            local_set(code, f11);
            emit_text_handles_eq(code, plan.num_regs, f8, f11);
        }
        PolicyCondition::Predicate { predicate, .. } => {
            let subj_type = plan.structs.struct_name_of.get(&subject).ok_or(WasmLowerError::Unsupported("policy: subject has no struct type"))?;
            let subj_sym = ctx.interner.lookup(subj_type).ok_or(WasmLowerError::Unsupported("policy: subject type not interned"))?;
            let preds = ctx.policies.get_predicates(subj_sym).ok_or(WasmLowerError::Unsupported("policy: no predicates for subject type"))?;
            let pred = preds.iter().find(|p| p.predicate_name == *predicate).ok_or(WasmLowerError::Unsupported("policy: referenced predicate not found"))?;
            emit_policy_condition(code, plan, ctx, &pred.condition, subject, object)?;
        }
        PolicyCondition::SubjectFieldEqualsObjectField { subject_field, object_field, .. }
        | PolicyCondition::ObjectFieldEquals { subject: subject_field, field: object_field, .. } => {
            if object == u16::MAX {
                return Err(WasmLowerError::Unsupported("policy: cross-field compare needs an object"));
            }
            let subj_type = plan.structs.struct_name_of.get(&subject).ok_or(WasmLowerError::Unsupported("policy: subject has no struct type"))?;
            let obj_type = plan.structs.struct_name_of.get(&object).ok_or(WasmLowerError::Unsupported("policy: object has no struct type"))?;
            let sf = ctx.interner.resolve(*subject_field);
            let of = ctx.interner.resolve(*object_field);
            let (s_slot, s_bt) = policy_field_slot(ctx, subj_type, sf).ok_or(WasmLowerError::Unsupported("policy: subject field not found"))?;
            let (o_slot, o_bt) = policy_field_slot(ctx, obj_type, of).ok_or(WasmLowerError::Unsupported("policy: object field not found"))?;
            if !matches!(s_bt, BoundaryType::Text) || !matches!(o_bt, BoundaryType::Text) {
                return Err(WasmLowerError::Unsupported("policy: cross-field compare of non-Text fields"));
            }
            emit_load_text_field(code, subject, s_slot, f8);
            emit_load_text_field(code, object, o_slot, f11);
            emit_text_handles_eq(code, plan.num_regs, f8, f11);
        }
        PolicyCondition::Or(l, r) => {
            emit_policy_condition(code, plan, ctx, l, subject, object)?;
            emit_policy_condition(code, plan, ctx, r, subject, object)?;
            code.push(0x72); // i32.or
        }
        PolicyCondition::And(l, r) => {
            emit_policy_condition(code, plan, ctx, l, subject, object)?;
            emit_policy_condition(code, plan, ctx, r, subject, object)?;
            code.push(0x71); // i32.and
        }
        PolicyCondition::FieldBool { .. } => {
            return Err(WasmLowerError::Unsupported("policy: boolean field condition"));
        }
    }
    Ok(())
}

/// `Check that <subject> is <predicate>` / `… can <action> <object>` (`CheckPolicy`) — resolve the
/// predicate/capability's condition from the `## Policy` registry, compile it inline, and TRAP
/// (`unreachable`) when it is false (the standalone module's analog of the VM's `check_policy` error);
/// when it holds, execution falls through to the following statements. Mirrors the VM semantics.
fn lower_check_policy(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, subject: u16, predicate: crate::Symbol, is_capability: bool, object: u16) -> R<()> {
    let subj_type = plan.structs.struct_name_of.get(&subject).ok_or(WasmLowerError::Unsupported("CheckPolicy on a non-struct subject"))?;
    let subj_sym = ctx.interner.lookup(subj_type).ok_or(WasmLowerError::Unsupported("CheckPolicy subject type not interned"))?;
    let cond = if is_capability {
        let caps = ctx.policies.get_capabilities(subj_sym).ok_or(WasmLowerError::Unsupported("CheckPolicy: no capabilities for subject type"))?;
        caps.iter().find(|c| c.action == predicate).map(|c| &c.condition).ok_or(WasmLowerError::Unsupported("CheckPolicy: capability not found"))?
    } else {
        let preds = ctx.policies.get_predicates(subj_sym).ok_or(WasmLowerError::Unsupported("CheckPolicy: no predicates for subject type"))?;
        preds.iter().find(|p| p.predicate_name == predicate).map(|p| &p.condition).ok_or(WasmLowerError::Unsupported("CheckPolicy: predicate not found"))?
    };
    emit_policy_condition(code, plan, ctx, cond, subject, object)?;
    code.push(0x45); // i32.eqz → condition is FALSE
    code.push(0x04);
    code.push(0x40); // if (failed)
    code.push(0x00); // unreachable — the check failed (the VM errors here)
    code.push(0x0B); // end if
    Ok(())
}

/// `Increase/Decrease <obj>'s <field> by <amount>` (`CrdtBump`) on a SINGLE-replica `Shared` struct's
/// `ConvergentCount` field. The VM stores such a counter as a plain `Int` struct field and bumps it
/// with `crdt_counter_bump` = `field.wrapping_add(±amount)` (a `Nothing` field reads as 0), so with one
/// replica this is exactly a struct-field read-modify-write — byte-identical. (Multi-replica MERGE,
/// which needs the per-replica CRDT object, stays deferred.) The field's slot is its declared position.
fn lower_crdt_bump(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, obj: u16, field_const: u32, amount: u16, negate: bool) -> R<()> {
    let _ = ctx;
    // The counter's slot = its position in the struct's field layout (matched by the field-name const
    // index the op carries). `reg_layout` is populated from the actual `NewStruct`/`StructInsert` ops,
    // so it covers a `Shared` struct too (whose type may not be in the plain `struct_types`).
    let layout = plan.structs.reg_layout.get(&obj).ok_or(WasmLowerError::Unsupported("CrdtBump on a value with no struct layout"))?;
    let slot = layout.iter().position(|(fc, _)| *fc == field_const).ok_or(WasmLowerError::Unsupported("CrdtBump: field not in struct layout"))? as u32;
    let off = slot * 8;
    // obj.field = obj.field ± amount  (store: [addr, value] → i64.store)
    local_get(code, obj as u32);
    i32_load(code, 8); // data_ptr (store base address)
    local_get(code, obj as u32);
    i32_load(code, 8);
    i64_load(code, off); // current
    local_get(code, amount as u32);
    code.push(if negate { 0x7D } else { 0x7C }); // i64.sub / i64.add
    i64_store(code, off);
    Ok(())
}

/// `Merge <source> into <target>` (`CrdtMerge`) of two same-typed `Shared` structs whose fields are
/// `ConvergentCount`/`Tally` counters. The VM merges field-by-field via `crdt_merge_field`, which for
/// two plain-`Int` counters is a SUM (`wrapping_add`) — so this adds each of the source's counter
/// slots into the target's, byte-identical for the single-replica-per-side case the guide shows. A
/// field that is NOT a plain-Int counter (a per-replica GCounter struct, a Set/Map/register CRDT)
/// needs the per-replica merge object and is soundly refused (that `Merge` stays deferred).
fn lower_crdt_merge(code: &mut Vec<u8>, plan: &Plan, target: u16, source: u16) -> R<()> {
    let layout = plan.structs.reg_layout.get(&target).ok_or(WasmLowerError::Unsupported("CrdtMerge on a value with no struct layout"))?;
    let fields: Vec<(u32, u16)> = layout.clone();
    for (slot, (_fc, value_reg)) in fields.iter().enumerate() {
        if plan.kinds.get(*value_reg as usize) != Some(Kind::Int) {
            return Err(WasmLowerError::Unsupported("CrdtMerge of a non-Int counter field"));
        }
        let off = (slot as u32) * 8;
        // target[slot] = target[slot] + source[slot]
        local_get(code, target as u32);
        i32_load(code, 8); // store base addr
        local_get(code, target as u32);
        i32_load(code, 8);
        i64_load(code, off); // target current
        local_get(code, source as u32);
        i32_load(code, 8);
        i64_load(code, off); // source
        code.push(0x7C); // i64.add
        i64_store(code, off);
    }
    Ok(())
}

/// `Resolve <obj>'s <field> to <value>` (`CrdtResolve`) — a single-replica Divergent register just
/// takes the new value (the VM overwrites the field: `s.fields.insert(field, v)`). So this stores the
/// value handle into the field's slot (resolved from `reg_layout` by the field-name const), matching
/// a plain field write — byte-identical for one replica. (A merged multi-value register is deferred.)
fn lower_crdt_resolve(code: &mut Vec<u8>, plan: &Plan, kinds: &KindTable, obj: u16, field_const: u32, value: u16) -> R<()> {
    let layout = plan.structs.reg_layout.get(&obj).ok_or(WasmLowerError::Unsupported("CrdtResolve on a value with no struct layout"))?;
    let slot = layout.iter().position(|(fc, _)| *fc == field_const).ok_or(WasmLowerError::Unsupported("CrdtResolve: field not in struct layout"))? as u32;
    let off = slot * 8;
    local_get(code, obj as u32);
    i32_load(code, 8); // data_ptr (store base)
    local_get(code, value as u32);
    emit_slot_store(code, kinds.get(value as usize), off)?;
    Ok(())
}

/// `Append <value> to <seq>` (`CrdtAppend`) — a single-replica RGA/sequence is just a growable list,
/// so this appends in place (the VM: a `List` → `list_push`). The CRDT collection is intentionally
/// MUTABLE-SHARED (the VM keeps it behind an `Rc` and says "appending in place propagates — no
/// write-back"), so this must NOT copy-on-write: it drives `lower_list_push` directly (whose in-place
/// header update the aliasing field sees). An OR-Set append routes to the byte-dedup set add.
fn lower_crdt_append(code: &mut Vec<u8>, plan: &Plan, kinds: &KindTable, ctx: &Ctx, seq: u16, value: u16) -> R<()> {
    match kinds.get(seq as usize) {
        Some(Kind::SeqText) | Some(Kind::SeqInt) | Some(Kind::SeqBool) | Some(Kind::SeqFloat) | Some(Kind::SeqAny) => {
            lower_list_push(code, kinds, ctx, plan.num_regs, seq, value)
        }
        Some(Kind::SetText) => {
            emit_set_add_elem(code, ctx, plan.num_regs, seq as u32, value as u32, true);
            Ok(())
        }
        Some(Kind::Set) => {
            emit_set_add_elem(code, ctx, plan.num_regs, seq as u32, value as u32, false);
            Ok(())
        }
        _ => Err(WasmLowerError::Unsupported("CrdtAppend to a non-collection CRDT")),
    }
}

/// `Push src to obj's field` (`ListPushField`) — the direct field-seq push (`Push x to p's items`).
/// Resolve the field's slot (`reg_layout`, matched by the field-name const) and its element kind (from
/// the register that defined the field's value), load the field's seq handle, and push through the
/// shared amortized [`lower_list_push_at`]. The seq's header address is stable across the push, so the
/// struct's field slot keeps pointing at it (no write-back). COW `obj` first (value-semantic struct
/// mutation); the exercised programs own `obj` uniquely (a nested aliased field-seq is a COW frontier).
fn lower_list_push_field(code: &mut Vec<u8>, plan: &Plan, kinds: &KindTable, ctx: &Ctx, obj: u16, field_const: u32, src: u16) -> R<()> {
    emit_cow(code, kinds, &plan.structs, ctx, plan.num_regs, obj)?;
    let layout = plan.structs.reg_layout.get(&obj).ok_or(WasmLowerError::Unsupported("ListPushField on a value with no struct layout"))?;
    let slot = layout
        .iter()
        .position(|(fc, _)| *fc == field_const)
        .ok_or(WasmLowerError::Unsupported("ListPushField: field not in struct layout"))?;
    // The element width comes from the PUSHED value's kind (the field seq may have been default-filled
    // empty, leaving its declared element kind unrefined) — an Int rides an i64 slot, a Text/handle an
    // i32 in the low word, all 8-byte slots.
    let elem = kinds.get(src as usize).ok_or(WasmLowerError::Unsupported("ListPushField: unknown pushed-value kind"))?;
    let off = (slot as u32) * 8;
    let handle = plan.num_regs + 8; // i32 scratch, distinct from lower_list_push_at's +5/+6/+7
    // handle = obj.data_ptr[slot] (the field seq's i32 handle)
    local_get(code, obj as u32);
    i32_load(code, 8);
    i32_load(code, off);
    local_set(code, handle);
    lower_list_push_at(code, elem, ctx, plan.num_regs, handle, src)
}

/// Emit a function's argument marshaling + `call` (the shared core of `Op::Call`, `Spawn`,
/// `SpawnHandle`) — retain clonable heap args (value semantics), pass each at the callee's declared
/// parameter valtype (promoting `Int`→`f64` where the signature wants it). Returns whether the callee
/// leaves a RESULT on the stack (the caller binds it, or drops it for a fire-and-forget spawn).
fn emit_sync_call(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, func: u16, args_start: u16, arg_count: u16) -> R<bool> {
    for a in 0..arg_count {
        let arg = args_start + a;
        if cow_clonable(kinds.get(arg as usize)) {
            emit_retain(code, arg);
        }
    }
    let pvts = ctx.fn_param_valtypes.get(func as usize).ok_or(WasmLowerError::Unsupported("call of unknown function"))?;
    for a in 0..arg_count {
        let arg = args_start + a;
        let arg_vt = kinds.valtype(arg as usize);
        let param_vt = pvts.get(a as usize).copied().unwrap_or(I64);
        if arg_vt == param_vt {
            local_get(code, arg as u32);
        } else if arg_vt == I64 && param_vt == F64 {
            push_as_f64(code, arg, kinds.get(arg as usize))?;
        } else {
            return Err(WasmLowerError::Unsupported("call argument type does not match the parameter"));
        }
    }
    code.push(0x10); // call
    leb_u32(code, ctx.fn_base + func as u32);
    Ok(ctx.fn_results.get(func as usize).copied().flatten().is_some())
}

/// `Receive <dst> from <chan>` (`ChanRecv`) on a single-threaded FIFO channel — pop the FRONT element:
/// Non-blocking `Try to receive` (`ChanTryRecv`) → an `Optional`: a non-empty queue pops its front
/// element and boxes it (`Some` — a fresh 8-byte heap box holding the inner scalar; handle != 0); an
/// empty queue yields `Nothing` (handle `0`). There is no blocking/trap path — the deterministic
/// single-task scheduler resumes a try-recv immediately either way (`scheduler::do_try_recv`). The
/// present inner kind (for a later `Show`) is carried out-of-band in `opt_inner` from this channel.
fn lower_chan_try_recv(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, dst: u16, chan: u16) -> R<()> {
    let elem = kinds
        .get(chan as usize)
        .and_then(Kind::seq_elem)
        .ok_or(WasmLowerError::Unsupported("try-receive from a channel of unknown element kind"))?;
    let ch = chan as u32;
    let idx = num_regs + 5; // i32 pop-front shift-loop scratch
    let boxp = num_regs + 6; // i32 Optional box pointer
    // A width-matched scratch holds the popped value before it is stored into the box.
    let val = match elem {
        Kind::Float => num_regs + 12,                                       // f64 scratch
        Kind::Int | Kind::Bool | Kind::Char | Kind::Moment | Kind::Duration | Kind::Time | Kind::Span => num_regs + 1, // i64 scratch
        _ => num_regs + 7,                                                  // i32-handle scratch
    };
    // if len == 0 { dst = Nothing (0) } else { pop front into `val`; box it; dst = box handle }
    local_get(code, ch);
    i32_load(code, 0);
    code.push(0x45); // i32.eqz → queue empty?
    code.push(0x04);
    code.push(0x40); // if (void block type)
    i32_const(code, 0);
    local_set(code, dst as u32); // Nothing
    code.push(0x05); // else
    emit_pop_front(code, elem, ch, idx, val)?;
    i32_const(code, 8);
    emit_alloc(code, ctx,boxp);
    local_get(code, boxp);
    local_get(code, val);
    emit_slot_store(code, Some(elem), 0)?;
    local_get(code, boxp);
    local_set(code, dst as u32);
    code.push(0x0B); // end if
    Ok(())
}

/// load `data_ptr[0]`, shift every later 8-byte slot down one, decrement `len`. A receive on an EMPTY
/// channel would BLOCK on the scheduler; a standalone module has none, so it traps (`unreachable`) —
/// the non-blocking send-then-receive shape never hits it.
fn lower_chan_recv(code: &mut Vec<u8>, kinds: &KindTable, num_regs: u32, dst: u16, chan: u16) -> R<()> {
    let elem = kinds.get(chan as usize).and_then(Kind::seq_elem).ok_or(WasmLowerError::Unsupported("receive from a channel of unknown element kind"))?;
    let ch = chan as u32;
    let idx = num_regs + 5;
    // if len == 0 → trap (blocking receive, no scheduler to resume it)
    local_get(code, ch);
    i32_load(code, 0);
    code.push(0x45); // i32.eqz
    code.push(0x04);
    code.push(0x40);
    code.push(0x00); // unreachable
    code.push(0x0B);
    emit_pop_front(code, elem, ch, idx, dst as u32)
}

/// Pop the FRONT element of channel/queue `ch` into `dst`: `dst = data[0]`, shift `data[1..len]`
/// down one 8-byte slot, then `len -= 1`. Assumes `len > 0` (the caller guards or traps). Shared by
/// [`lower_chan_recv`] and the winning recv arm of a `select` ([`lower_select_wait`]).
fn emit_pop_front(code: &mut Vec<u8>, elem: Kind, ch: u32, idx: u32, dst: u32) -> R<()> {
    let elem_load = seq_elem_load(elem)?;
    // dst = data_ptr[0]
    local_get(code, ch);
    i32_load(code, 8);
    elem_load(code, 0);
    local_set(code, dst);
    // for i in 0..len-1: data[i] = data[i+1] (8-byte slot copy)
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40);
    code.push(0x03);
    code.push(0x40);
    local_get(code, idx);
    local_get(code, ch);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6B);
    code.push(0x4E); // i32.ge_s → idx >= len-1
    code.push(0x0D);
    leb_u32(code, 1);
    local_get(code, ch);
    i32_load(code, 8);
    local_get(code, idx);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    local_get(code, ch);
    i32_load(code, 8);
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    i64_load(code, 0);
    i64_store(code, 0);
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0);
    code.push(0x0B);
    code.push(0x0B);
    // len -= 1
    local_get(code, ch);
    local_get(code, ch);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6B);
    i32_store(code, 0);
    Ok(())
}

/// Resolve a `select` (`Await the first of …`) deterministically, writing the winning arm's index
/// into `dst_arm` (the following compiler-emitted per-arm `Eq`/jump dispatch then runs that branch).
///
/// The arms were registered by the `SelectArm*` ops preceding this `SelectWait` in the same block
/// (each emits no code); we read them back by scanning the block, resetting at any earlier
/// `SelectWait` so a second `select` in one block sees only its own arms.
///
/// The resolution mirrors the seeded cooperative scheduler for the shapes the AOT models (no true
/// racing): the FIRST recv arm whose FIFO queue is non-empty wins (pop-front into its bound var);
/// if no recv arm is ready, the timeout arm fires. When no recv arm is ready and there is no
/// timeout arm the scheduler would block forever — a deterministic deadlock, emitted as a trap.
fn lower_select_wait(code: &mut Vec<u8>, plan: &Plan, kinds: &KindTable, blocks: &Blocks, k: usize, pc: usize, dst_arm: u16) -> R<()> {
    #[derive(Clone, Copy)]
    enum Arm {
        Recv { chan: u16, var: u16 },
        Timeout,
    }
    let mut arms: Vec<Arm> = Vec::new();
    for j in blocks.start(k)..pc {
        match plan.ops[j] {
            Op::SelectArmRecv { chan, var } => arms.push(Arm::Recv { chan, var }),
            Op::SelectArmTimeout { .. } => arms.push(Arm::Timeout),
            Op::SelectWait { .. } => arms.clear(),
            _ => {}
        }
    }
    let da = dst_arm as u32;
    let timeout_idx = arms.iter().position(|a| matches!(a, Arm::Timeout));

    // dst_arm = -1 (no winner yet).
    code.push(0x42); // i64.const
    leb_i64(code, -1);
    local_set(code, da);

    // First ready recv arm wins: `if dst_arm == -1 && len(chan) > 0 { var = pop_front(chan); dst_arm = i }`.
    for (i, arm) in arms.iter().enumerate() {
        if let Arm::Recv { chan, var } = *arm {
            let elem = kinds
                .get(chan as usize)
                .and_then(Kind::seq_elem)
                .ok_or(WasmLowerError::Unsupported("select recv arm on a channel of unknown element kind"))?;
            local_get(code, da);
            code.push(0x42); // i64.const -1
            leb_i64(code, -1);
            code.push(0x51); // i64.eq → (dst_arm == -1)
            local_get(code, chan as u32);
            i32_load(code, 0); // len(chan)
            i32_const(code, 0);
            code.push(0x4A); // i32.gt_s → (len > 0)
            code.push(0x71); // i32.and
            code.push(0x04);
            code.push(0x40); // if (void)
            emit_pop_front(code, elem, chan as u32, plan.num_regs + 5, var as u32)?;
            code.push(0x42); // i64.const i
            leb_i64(code, i as i64);
            local_set(code, da);
            code.push(0x0B); // end if
        }
    }

    // No recv arm ready → the timeout arm fires (or a deadlock trap if there is none).
    local_get(code, da);
    code.push(0x42); // i64.const -1
    leb_i64(code, -1);
    code.push(0x51); // i64.eq
    code.push(0x04);
    code.push(0x40); // if (void)
    match timeout_idx {
        Some(ti) => {
            code.push(0x42); // i64.const ti
            leb_i64(code, ti as i64);
            local_set(code, da);
        }
        None => code.push(0x00), // unreachable — no ready arm, no timeout: deadlock
    }
    code.push(0x0B); // end if
    Ok(())
}

/// `i64.const v`.
fn i64c(code: &mut Vec<u8>, v: i64) {
    code.push(0x42);
    leb_i64(code, v);
}

/// Lower `Op::MagicDivU` — `lhs / c` (`mul_back == 0`) or `lhs % c` (`mul_back == c`) via the
/// Granlund–Montgomery magic reciprocal, mirroring `vm::compiler::magic_eval` bit-for-bit. `magic`,
/// `more`, and `mul_back` are compile-time constants, so the flag paths are chosen HERE (in Rust) and
/// only the taken sequence is emitted. The 64×64→128 high word (`(magic * n) >> 64`) is computed from
/// 32-bit limbs (wasm has no mul-high). `lhs` is an Oracle-proven non-negative `Int` (i64).
fn lower_magic_div(code: &mut Vec<u8>, num_regs: u32, dst: u16, lhs: u16, magic: u64, more: u8, mul_back: i64) {
    const SHIFT_MASK: u8 = 0x3F;
    const ADD_MARKER: u8 = 0x40;
    const SHIFT_PATH: u8 = 0x80;
    let shift = (more & SHIFT_MASK) as i64;
    let q = num_regs + 1; // i64 scratch (reuses the integer-pow result slot)
    let n_lo = num_regs + 2;
    let n_hi = num_regs + 3;
    let hi = num_regs + 4;
    let mask: i64 = 0xFFFF_FFFF;

    if more & SHIFT_PATH != 0 {
        // q = n >>u shift
        local_get(code, lhs as u32);
        i64c(code, shift);
        code.push(0x88); // i64.shr_u
        local_set(code, q);
    } else {
        // n limbs
        local_get(code, lhs as u32);
        i64c(code, mask);
        code.push(0x83); // i64.and
        local_set(code, n_lo);
        local_get(code, lhs as u32);
        i64c(code, 32);
        code.push(0x88); // shr_u
        local_set(code, n_hi);
        let m_lo = (magic & 0xFFFF_FFFF) as i64;
        let m_hi = (magic >> 32) as i64;
        // cross = (m_lo*n_lo >>u 32) + (m_hi*n_lo & mask) + (m_lo*n_hi & mask)  → into `q`
        i64c(code, m_lo);
        local_get(code, n_lo);
        code.push(0x7E); // mul
        i64c(code, 32);
        code.push(0x88); // >>u 32
        i64c(code, m_hi);
        local_get(code, n_lo);
        code.push(0x7E);
        i64c(code, mask);
        code.push(0x83); // & mask
        code.push(0x7C); // add
        i64c(code, m_lo);
        local_get(code, n_hi);
        code.push(0x7E);
        i64c(code, mask);
        code.push(0x83);
        code.push(0x7C); // add
        local_set(code, q); // q := cross
        // hi = m_hi*n_hi + (m_hi*n_lo >>u 32) + (m_lo*n_hi >>u 32) + (cross >>u 32)
        i64c(code, m_hi);
        local_get(code, n_hi);
        code.push(0x7E); // hi_hi
        i64c(code, m_hi);
        local_get(code, n_lo);
        code.push(0x7E);
        i64c(code, 32);
        code.push(0x88);
        code.push(0x7C); // + (hi_lo>>32)
        i64c(code, m_lo);
        local_get(code, n_hi);
        code.push(0x7E);
        i64c(code, 32);
        code.push(0x88);
        code.push(0x7C); // + (lo_hi>>32)
        local_get(code, q);
        i64c(code, 32);
        code.push(0x88);
        code.push(0x7C); // + (cross>>32)
        local_set(code, hi);
        if more & ADD_MARKER != 0 {
            // q = (((n - hi) >>u 1) + hi) >>u shift
            local_get(code, lhs as u32);
            local_get(code, hi);
            code.push(0x7D); // sub
            i64c(code, 1);
            code.push(0x88); // >>u 1
            local_get(code, hi);
            code.push(0x7C); // + hi
            i64c(code, shift);
            code.push(0x88); // >>u shift
            local_set(code, q);
        } else {
            // q = hi >>u shift
            local_get(code, hi);
            i64c(code, shift);
            code.push(0x88);
            local_set(code, q);
        }
    }

    if mul_back == 0 {
        local_get(code, q);
        local_set(code, dst as u32);
    } else {
        // result = lhs - q * mul_back
        local_get(code, lhs as u32);
        local_get(code, q);
        i64c(code, mul_back);
        code.push(0x7E); // mul
        code.push(0x7D); // sub
        local_set(code, dst as u32);
    }
}

/// Lower `Op::ExactDiv` — `dst = lhs / rhs` as an exact `Rational` (`7 / 2 → 7/2`). Normalizes the
/// sign (den > 0), reduces by `gcd(|num|, den)` (Euclidean), allocs a 16-byte `[num][den]` value, and
/// leaves its handle in `dst`. A whole quotient reduces to `den == 1` (so `Show` renders just the
/// integer, matching the VM's downsize). Traps on a zero divisor (the VM errors "Division by zero").
fn lower_exact_div(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, lhs: u16, rhs: u16) {
    let (num, den, a, b) = (num_regs + 1, num_regs + 2, num_regs + 3, num_regs + 4);
    let handle = num_regs + 5;
    // rhs == 0 → trap
    local_get(code, rhs as u32);
    code.push(0x50); // i64.eqz
    code.push(0x04);
    code.push(0x40);
    code.push(0x00); // unreachable
    code.push(0x0B);
    // num = lhs; den = rhs
    local_get(code, lhs as u32);
    local_set(code, num);
    local_get(code, rhs as u32);
    local_set(code, den);
    // if den < 0 { num = -num; den = -den }
    local_get(code, den);
    i64c(code, 0);
    code.push(0x53); // i64.lt_s
    code.push(0x04);
    code.push(0x40);
    i64c(code, 0);
    local_get(code, num);
    code.push(0x7D); // sub → -num
    local_set(code, num);
    i64c(code, 0);
    local_get(code, den);
    code.push(0x7D);
    local_set(code, den);
    code.push(0x0B); // end if
    // a = |num|
    local_get(code, num);
    local_set(code, a);
    local_get(code, a);
    i64c(code, 0);
    code.push(0x53); // a < 0
    code.push(0x04);
    code.push(0x40);
    i64c(code, 0);
    local_get(code, a);
    code.push(0x7D);
    local_set(code, a);
    code.push(0x0B);
    // b = den; gcd(a, b): while b != 0 { let r = a % b; a = b; b = r }
    local_get(code, den);
    local_set(code, b);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    local_get(code, b);
    code.push(0x50); // i64.eqz
    code.push(0x0D);
    leb_u32(code, 1); // br_if exit (b == 0)
    local_get(code, a);
    local_get(code, b);
    code.push(0x81); // i64.rem_s → a % b (on stack)
    local_get(code, b);
    local_set(code, a); // a = old b
    local_set(code, b); // b = a % b
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    // num /= a (gcd) ; den /= a
    local_get(code, num);
    local_get(code, a);
    code.push(0x7F); // i64.div_s
    local_set(code, num);
    local_get(code, den);
    local_get(code, a);
    code.push(0x7F);
    local_set(code, den);
    // handle = alloc(16); store num@0, den@8
    i32_const(code, 16);
    emit_alloc(code, ctx,handle);
    local_get(code, handle);
    local_get(code, num);
    i64_store(code, 0);
    local_get(code, handle);
    local_get(code, den);
    i64_store(code, 8);
    local_get(code, handle);
    local_set(code, dst as u32);
}

/// Emit `for i in 0..len(src): dest[(base_off + i)] = src_bytes[i]`, a single-byte copy loop (for
/// `Text`). `offset_by_a` shifts the destination by `len(a_for_offset)` (to append after the first
/// operand of a concat); otherwise the copy is index-aligned.
fn emit_byte_copy(code: &mut Vec<u8>, idx: u32, dest_data: u32, a_for_offset: u32, src: u32, offset_by_a: bool) {
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    local_get(code, idx);
    local_get(code, src);
    i32_load(code, 0); // len(src)
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if exit
    // dest addr = dest_data + (offset + i)
    local_get(code, dest_data);
    if offset_by_a {
        local_get(code, a_for_offset);
        i32_load(code, 0); // len(a)
        local_get(code, idx);
        code.push(0x6A);
    } else {
        local_get(code, idx);
    }
    code.push(0x6A);
    // src byte = src_data[i]
    local_get(code, src);
    i32_load(code, 8); // src data_ptr
    local_get(code, idx);
    code.push(0x6A);
    i32_load8_u(code, 0);
    i32_store8(code, 0);
    // i++
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
}

/// A string literal → a fresh `Text` object in linear memory, leaving its `i32` handle on the
/// stack. Bump-allocates a 16-byte header + an 8-aligned byte buffer, writes the UTF-8 bytes as
/// little-endian 8-byte `i64.store` chunks (the trailing chunk zero-padded past `len`, which is
/// never read), and fills the header `[len][cap][data_ptr]` (byte counts). Each execution makes a
/// fresh object, matching the tree-walker's value semantics (immutable, so the waste is benign).
fn lower_text_literal(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, bytes: &[u8]) {
    let len = bytes.len();
    let cap8 = (len + 7) & !7; // 8-aligned buffer so the last i64.store stays in bounds
    let (hdr, data) = (num_regs + 5, num_regs + 6);
    i32_const(code, 16);
    emit_alloc(code, ctx,hdr);
    i32_const(code, cap8 as i32);
    emit_alloc(code, ctx,data);
    for c in 0..(cap8 / 8) {
        let mut v: u64 = 0;
        for j in 0..8 {
            if let Some(&b) = bytes.get(c * 8 + j) {
                v |= (b as u64) << (j * 8);
            }
        }
        local_get(code, data);
        code.push(0x42); // i64.const
        leb_i64(code, v as i64);
        i64_store(code, (c * 8) as u32);
    }
    local_get(code, hdr);
    i32_const(code, len as i32);
    i32_store(code, 0); // len (bytes)
    local_get(code, hdr);
    i32_const(code, len as i32);
    i32_store(code, 4); // cap
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8); // data_ptr
    local_get(code, hdr); // leave the handle on the stack
}

/// `chr(code) -> Text` — a single-character `Text` from a Unicode scalar value. The UTF-8 encoding is
/// computed INLINE (1–4 bytes selected by the code point's range) and packed little-endian into an
/// `i64`, then written into a fresh Text object (`[len][cap][data_ptr]` header + an 8-byte data
/// buffer). An invalid scalar (a surrogate `U+D800..=U+DFFF` or `> U+10FFFF`) traps, matching the
/// VM's `char::from_u32(..)` returning `None`.
fn lower_chr(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, arg: u16) {
    let c = num_regs + 1; // i64 code point
    let packed = num_regs + 2; // i64 UTF-8 bytes, little-endian in the low bytes
    let len = num_regs + 7; // i32 byte count
    let (hdr, data) = (num_regs + 5, num_regs + 6);
    // Continuation byte `0x80 | ((c >> shift) & 0x3F)`, shifted into byte position `pos`, left on stack.
    let cont = |code: &mut Vec<u8>, shift: i64, pos: i64| {
        local_get(code, c);
        i64c(code, shift);
        code.push(0x88); // i64.shr_u
        i64c(code, 0x3F);
        code.push(0x83); // i64.and
        i64c(code, 0x80);
        code.push(0x84); // i64.or
        i64c(code, pos);
        code.push(0x86); // i64.shl
    };
    // Lead byte `mask | (c >> shift)`, left on stack.
    let lead = |code: &mut Vec<u8>, shift: i64, mask: i64| {
        local_get(code, c);
        i64c(code, shift);
        code.push(0x88); // i64.shr_u
        i64c(code, mask);
        code.push(0x84); // i64.or
    };
    // c = arg
    local_get(code, arg as u32);
    local_set(code, c);
    // Trap on an invalid scalar value: (u64)c > 0x10FFFF  ||  (0xD800 <= c <= 0xDFFF).
    local_get(code, c);
    i64c(code, 0x10FFFF);
    code.push(0x56); // i64.gt_u
    local_get(code, c);
    i64c(code, 0xD800);
    code.push(0x5A); // i64.ge_u
    local_get(code, c);
    i64c(code, 0xDFFF);
    code.push(0x58); // i64.le_u
    code.push(0x71); // i32.and → in-surrogate-range
    code.push(0x72); // i32.or  → invalid
    code.push(0x04);
    code.push(0x40); // if (invalid)
    code.push(0x00); // unreachable (trap)
    code.push(0x0B); // end
    // 1 byte: c < 0x80
    local_get(code, c);
    i64c(code, 0x80);
    code.push(0x54); // i64.lt_u
    code.push(0x04);
    code.push(0x40); // if
    local_get(code, c);
    local_set(code, packed);
    i32_const(code, 1);
    local_set(code, len);
    code.push(0x05); // else
    // 2 bytes: c < 0x800
    local_get(code, c);
    i64c(code, 0x800);
    code.push(0x54); // i64.lt_u
    code.push(0x04);
    code.push(0x40); // if
    lead(code, 6, 0xC0);
    cont(code, 0, 8);
    code.push(0x84); // i64.or
    local_set(code, packed);
    i32_const(code, 2);
    local_set(code, len);
    code.push(0x05); // else
    // 3 bytes: c < 0x10000
    local_get(code, c);
    i64c(code, 0x10000);
    code.push(0x54); // i64.lt_u
    code.push(0x04);
    code.push(0x40); // if
    lead(code, 12, 0xE0);
    cont(code, 6, 8);
    code.push(0x84); // i64.or
    cont(code, 0, 16);
    code.push(0x84); // i64.or
    local_set(code, packed);
    i32_const(code, 3);
    local_set(code, len);
    code.push(0x05); // else — 4 bytes
    lead(code, 18, 0xF0);
    cont(code, 12, 8);
    code.push(0x84); // i64.or
    cont(code, 6, 16);
    code.push(0x84); // i64.or
    cont(code, 0, 24);
    code.push(0x84); // i64.or
    local_set(code, packed);
    i32_const(code, 4);
    local_set(code, len);
    code.push(0x0B); // end (3-byte if)
    code.push(0x0B); // end (2-byte if)
    code.push(0x0B); // end (1-byte if)
    // Build the Text: 16-byte header + an 8-byte data buffer (max 4 UTF-8 bytes ≤ 8).
    i32_const(code, 16);
    emit_alloc(code, ctx,hdr);
    i32_const(code, 8);
    emit_alloc(code, ctx,data);
    local_get(code, data);
    local_get(code, packed);
    i64_store(code, 0); // the packed bytes (low `len` are valid, the rest zero)
    local_get(code, hdr);
    local_get(code, len);
    i32_store(code, 0); // len (bytes)
    local_get(code, hdr);
    local_get(code, len);
    i32_store(code, 4); // cap
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8); // data_ptr
    local_get(code, hdr);
    local_set(code, dst as u32);
}

/// `lhs followed by rhs` — concatenate two sequences into a fresh one (the tree-walker's
/// `arith::seq_concat`: lhs's elements then rhs's). Both element kinds match (raw 8-byte copy).
fn lower_seq_concat(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, dst: u16, lhs: u16, rhs: u16) -> R<()> {
    seq_elem_kind(kinds, lhs)?;
    seq_elem_kind(kinds, rhs)?;
    let (a, b) = (lhs as u32, rhs as u32);
    let (hdr, data, idx) = (num_regs + 5, num_regs + 6, num_regs + 7);
    // hdr = alloc(16); data = alloc((len_a + len_b) * 8)
    i32_const(code, 16);
    emit_alloc(code, ctx,hdr);
    local_get(code, a);
    i32_load(code, 0);
    local_get(code, b);
    i32_load(code, 0);
    code.push(0x6A); // len_a + len_b
    i32_const(code, 8);
    code.push(0x6C);
    emit_alloc(code, ctx,data);
    // copy lhs: for i in 0..len_a: data[i*8] = a_data[i*8]
    emit_seq_copy(code, idx, data, a, a, false);
    // copy rhs: for i in 0..len_b: data[(len_a + i)*8] = b_data[i*8]
    emit_seq_copy(code, idx, data, a, b, true);
    // header: len = cap = len_a + len_b; data_ptr = data
    for off in [0u32, 4] {
        local_get(code, hdr);
        local_get(code, a);
        i32_load(code, 0);
        local_get(code, b);
        i32_load(code, 0);
        code.push(0x6A);
        i32_store(code, off);
    }
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8);
    local_get(code, hdr);
    local_set(code, dst as u32);
    Ok(())
}

/// Emit `for i in 0..len(src): dest[(base_off + i)*8] = src_data[i*8]`, a raw 8-byte element copy
/// loop. When `offset_by_a` the destination index is shifted by `len(a_for_offset)` (used to append
/// the second operand of a concat after the first); otherwise the copy is index-aligned.
fn emit_seq_copy(code: &mut Vec<u8>, idx: u32, dest_data: u32, a_for_offset: u32, src: u32, offset_by_a: bool) {
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    local_get(code, idx);
    local_get(code, src);
    i32_load(code, 0); // len(src)
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if exit
    // dest addr = dest_data + (offset + i)*8
    local_get(code, dest_data);
    if offset_by_a {
        local_get(code, a_for_offset);
        i32_load(code, 0); // len(a)
        local_get(code, idx);
        code.push(0x6A); // len(a) + i
    } else {
        local_get(code, idx);
    }
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    // src addr = src_data + i*8
    local_get(code, src);
    i32_load(code, 8);
    local_get(code, idx);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    i64_load(code, 0);
    i64_store(code, 0);
    // i++
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
}

/// `items start through end of seq` (1-based inclusive subsequence) — allocate a fresh sequence
/// and copy the in-range elements. Matches the tree-walker's `collections::slice` byte-for-byte,
/// including its out-of-range → empty rule: with `start0 = (start as usize).saturating_sub(1)` and
/// `end_excl = end as usize`, the result is non-empty iff `start0 < end_excl <= len` (the `usize`
/// casts are reproduced with unsigned i64 compares, so negative indices wrap huge → empty).
fn lower_slice(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, dst: u16, collection: u16, start: u16, end: u16) -> R<()> {
    seq_elem_kind(kinds, collection)?; // require a scalar sequence; the copy is raw 8-byte either way
    let col = collection as u32;
    let (s0, ee) = (num_regs + 1, num_regs + 2); // i64 scratch: start0, end_excl
    let (hdr, data, idx) = (num_regs + 5, num_regs + 6, num_regs + 7); // i32 scratch
    // hdr = alloc(16)
    i32_const(code, 16);
    emit_alloc(code, ctx,hdr);
    // start0 = (start == 0) ? 0 : (start - 1)   [= saturating_sub((start as u64), 1)]
    code.push(0x42);
    leb_i64(code, 0); // 0
    local_get(code, start as u32);
    code.push(0x42);
    leb_i64(code, 1);
    code.push(0x7D); // start - 1
    local_get(code, start as u32);
    code.push(0x50); // i64.eqz(start)
    code.push(0x1B); // select → eqz ? 0 : start-1
    local_set(code, s0);
    // end_excl = end
    local_get(code, end as u32);
    local_set(code, ee);
    // nonempty = (start0 <u end_excl) && (end_excl <=u len)
    local_get(code, s0);
    local_get(code, ee);
    code.push(0x54); // i64.lt_u
    local_get(code, ee);
    local_get(code, col);
    i32_load(code, 0); // len
    code.push(0xAD); // i64.extend_i32_u
    code.push(0x58); // i64.le_u
    code.push(0x71); // i32.and
    code.push(0x04);
    code.push(0x40); // if (nonempty)
    {
        // count*8 → data = alloc(count*8)
        local_get(code, ee);
        local_get(code, s0);
        code.push(0x7D);
        code.push(0xA7); // count (i32)
        i32_const(code, 8);
        code.push(0x6C);
        emit_alloc(code, ctx,data);
        // for i in 0..count: data[i*8] = src_data[(start0+i)*8]  (raw 8-byte copy)
        i32_const(code, 0);
        local_set(code, idx);
        code.push(0x02);
        code.push(0x40); // block
        code.push(0x03);
        code.push(0x40); // loop
        local_get(code, idx);
        local_get(code, ee);
        local_get(code, s0);
        code.push(0x7D);
        code.push(0xA7); // count
        code.push(0x4E); // i32.ge_s
        code.push(0x0D);
        leb_u32(code, 1); // br_if exit
        // dst addr = data + idx*8
        local_get(code, data);
        local_get(code, idx);
        i32_const(code, 8);
        code.push(0x6C);
        code.push(0x6A);
        // src addr = src_data_ptr + (start0_i32 + idx)*8
        local_get(code, col);
        i32_load(code, 8);
        local_get(code, s0);
        code.push(0xA7); // start0 as i32
        local_get(code, idx);
        code.push(0x6A); // start0 + idx
        i32_const(code, 8);
        code.push(0x6C);
        code.push(0x6A);
        i64_load(code, 0);
        i64_store(code, 0);
        // idx++
        local_get(code, idx);
        i32_const(code, 1);
        code.push(0x6A);
        local_set(code, idx);
        code.push(0x0C);
        leb_u32(code, 0); // br loop
        code.push(0x0B); // end loop
        code.push(0x0B); // end block
        // header: len = cap = count; data_ptr = data
        for off in [0u32, 4] {
            local_get(code, hdr);
            local_get(code, ee);
            local_get(code, s0);
            code.push(0x7D);
            code.push(0xA7);
            i32_store(code, off);
        }
        local_get(code, hdr);
        local_get(code, data);
        i32_store(code, 8);
    }
    code.push(0x05); // else (empty)
    for off in [0u32, 4, 8] {
        local_get(code, hdr);
        i32_const(code, 0);
        i32_store(code, off);
    }
    code.push(0x0B); // end if
    local_get(code, hdr);
    local_set(code, dst as u32);
    Ok(())
}

/// `IterPrepare`: snapshot the sequence in `iterable` (a raw byte copy of its `len` 8-byte
/// elements into a fresh buffer — so a mutation inside the loop cannot perturb iteration, exactly
/// as the tree-walker's `iteration_snapshot` materializes a fresh `Vec`) and push a 12-byte frame
/// `[snapshot_ptr:i32 @0][cursor:i32 @4][len:i32 @8]` onto the down-growing iterator stack.
fn lower_iter_prepare(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, num_regs: u32, iterable: u16) -> R<()> {
    if !kinds.get(iterable as usize).map(Kind::is_seq).unwrap_or(false) {
        return Err(WasmLowerError::Unsupported("iteration over a non-sequence value"));
    }
    let it = iterable as u32;
    let (snap, idx) = (num_regs + 5, num_regs + 6); // i32 scratch
    // snap = alloc(len * 8)
    local_get(code, it);
    i32_load(code, 0); // len
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    emit_alloc(code, ctx,snap);
    // for i in 0..len: snap[i*8] = data_ptr[i*8]  (raw 8-byte copy; Int and Float are both 8 wide)
    i32_const(code, 0);
    local_set(code, idx);
    code.push(0x02);
    code.push(0x40); // block $exit
    code.push(0x03);
    code.push(0x40); // loop $loop
    local_get(code, idx);
    local_get(code, it);
    i32_load(code, 0); // len
    code.push(0x4E); // i32.ge_s → i >= len
    code.push(0x0D);
    leb_u32(code, 1); // br_if $exit
    // dst addr = snap + i*8
    local_get(code, snap);
    local_get(code, idx);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    // src value = data_ptr[i*8]
    local_get(code, it);
    i32_load(code, 8); // data_ptr
    local_get(code, idx);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    i64_load(code, 0);
    i64_store(code, 0);
    // i++
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0); // br $loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    // push frame: __iter_sp -= 12
    global_get(code, ctx.iter_global);
    i32_const(code, 12);
    code.push(0x6B); // i32.sub
    global_set(code, ctx.iter_global);
    // frame[0] = snap; frame[4] = 0 (cursor); frame[8] = len
    global_get(code, ctx.iter_global);
    local_get(code, snap);
    i32_store(code, 0);
    global_get(code, ctx.iter_global);
    i32_const(code, 0);
    i32_store(code, 4);
    global_get(code, ctx.iter_global);
    local_get(code, it);
    i32_load(code, 0);
    i32_store(code, 8);
    Ok(())
}

/// `IterNext { dst, exit }`: a conditional block terminator. If the top frame's `cursor < len`,
/// load `snapshot[cursor]` into `dst` (i64 for Int, f64 for Float), advance the cursor, and fall
/// through to the loop body; otherwise branch to `exit` (the matching `IterPop`). Either way it
/// sets the dispatch "next block" local and `br`s to `$loop`, like every other branch terminator.
#[allow(clippy::too_many_arguments)]
fn lower_iter_next(code: &mut Vec<u8>, kinds: &KindTable, ctx: &Ctx, blocks: &Blocks, k: usize, num_regs: u32, dst: u16, exit: usize, pc: usize) {
    // The loop variable's kind IS the element kind; load it at its width (`f64`/`i64`, or `i32` for
    // a heap handle like Text/Struct/Enum).
    let elem_load: fn(&mut Vec<u8>, u32) = match kinds.get(dst as usize).map(Kind::wasm_valtype) {
        Some(F64) => f64_load,
        Some(I64) => i64_load,
        _ => i32_load,
    };
    let ig = ctx.iter_global;
    let pc_local = num_regs;
    let fallthrough = blocks.block_of(pc + 1) as u32;
    let exit_block = blocks.block_of(exit) as u32;
    // cursor < len ?
    global_get(code, ig);
    i32_load(code, 4); // cursor
    global_get(code, ig);
    i32_load(code, 8); // len
    code.push(0x48); // i32.lt_s
    code.push(0x04);
    code.push(0x40); // if (void)
    // dst = snapshot[cursor]
    global_get(code, ig);
    i32_load(code, 0); // snapshot_ptr
    global_get(code, ig);
    i32_load(code, 4); // cursor
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    code.push(0x6A); // i32.add → element addr
    elem_load(code, 0);
    local_set(code, dst as u32);
    // cursor++  (frame[4] = cursor + 1)
    global_get(code, ig);
    global_get(code, ig);
    i32_load(code, 4);
    i32_const(code, 1);
    code.push(0x6A); // i32.add
    i32_store(code, 4);
    // next block = fallthrough (the loop body)
    code.push(0x41); // i32.const
    leb_u32(code, fallthrough);
    local_set(code, pc_local);
    code.push(0x05); // else
    // exhausted: next block = exit (the IterPop)
    code.push(0x41); // i32.const
    leb_u32(code, exit_block);
    local_set(code, pc_local);
    code.push(0x0B); // end if
    code.push(0x0C); // br $loop
    leb_u32(code, blocks.br_loop(k));
}

/// Whether an op touches the heap value model's linear memory (so the module needs a memory +
/// `__heap_ptr` global). Grows as heap ops are lowered.
fn op_uses_heap(op: &Op) -> bool {
    matches!(
        op,
        Op::NewEmptyList { .. }
            | Op::NewEmptyListI32 { .. }
            | Op::Length { .. }
            | Op::ListPush { .. }
            | Op::ListPop { .. }
            | Op::Index { .. }
            | Op::IndexUnchecked { .. }
            | Op::SetIndex { .. }
            | Op::SetIndexUnchecked { .. }
            | Op::ListPushField { .. }
            // `ExactDiv` allocates a 16-byte Rational value in linear memory.
            | Op::ExactDiv { .. }
            | Op::NewRange { .. }
            | Op::NewList { .. }
            | Op::IterPrepare { .. }
            | Op::IterNext { .. }
            | Op::IterPop
            | Op::Contains { .. }
            | Op::SliceOp { .. }
            | Op::SeqConcat { .. }
            | Op::Concat { .. }
            | Op::FormatValue { .. }
            | Op::DeepClone { .. }
            | Op::NewStruct { .. }
            | Op::StructInsert { .. }
            | Op::GetField { .. }
            | Op::CheckPolicy { .. }
            | Op::CrdtBump { .. }
            | Op::CrdtMerge { .. }
            | Op::NewCrdt { .. }
            | Op::CrdtResolve { .. }
            | Op::CrdtAppend { .. }
            | Op::ChanNew { .. }
            | Op::ChanSend { .. }
            | Op::ChanRecv { .. }
            // `Try to send` appends to (reallocs) the FIFO; `Try to receive` allocs an Optional box.
            | Op::ChanTrySend { .. }
            | Op::ChanTryRecv { .. }
            // A `select` recv arm pops from a channel's FIFO queue in linear memory.
            | Op::SelectWait { .. }
            // Offline networking models the inbox as a local FIFO in linear memory.
            | Op::NetListen { .. }
            | Op::NetSend { .. }
            | Op::NetStream { .. }
            | Op::NetAwait { .. }
            | Op::NewEmptyMap { .. }
            | Op::NewEmptySet { .. }
            | Op::SetAdd { .. }
            | Op::RemoveFrom { .. }
            | Op::UnionOp { .. }
            | Op::IntersectOp { .. }
            | Op::NewInductive { .. }
            | Op::BindArm { .. }
            | Op::NewTuple { .. }
            | Op::DestructureTuple { .. }
            | Op::MakeClosure { .. }
            | Op::CallValue { .. }
            // `args()` returns a `Seq of Text` HANDLE the host builds in this module's linear memory,
            // so the module must export a memory for the host to write into.
            | Op::Args { .. }
            // `chr(code)` builds a one-character Text object in linear memory.
            | Op::CallBuiltin { builtin: BuiltinId::Chr, .. }
            // `repeatSeq(x, n)` bump-allocates a fresh `n`-element sequence.
            | Op::CallBuiltin { builtin: BuiltinId::RepeatSeq, .. }
            // Byte interop allocates a fresh seq / Text / 16-byte block in linear memory.
            | Op::CallBuiltin { builtin: BuiltinId::TextBytes, .. }
            | Op::CallBuiltin { builtin: BuiltinId::UuidBytes, .. }
            | Op::CallBuiltin { builtin: BuiltinId::TextFromBytes, .. }
            | Op::CallBuiltin { builtin: BuiltinId::UuidFromBytes, .. }
            | Op::CallBuiltin { builtin: BuiltinId::Lanes4Of, .. }
            | Op::CallBuiltin { builtin: BuiltinId::Lanes4Word32Make, .. }
            | Op::CallBuiltin { builtin: BuiltinId::SeqOfLanes4W32, .. }
            // `readWireProgram` bump-allocates a receive buffer (via `emit_alloc`), so it needs the runtime
            // allocator (`logos_rt_alloc`) imported — else `emit_alloc` falls to the `__heap_ptr` global,
            // undeclared in a linked module (an invalid global relocation in the emitted object).
            | Op::CallBuiltin { builtin: BuiltinId::ReadWireProgram, .. }
    )
}

/// The local-declaration prefix of a Code entry: registers `num_params..num_regs` typed by
/// their inferred kind (coalesced into same-type groups), the i32 dispatch local, four i64
/// scratch locals (integer `pow`), and one i32 heap-allocation scratch.
fn encode_locals(plan: &Plan) -> Vec<u8> {
    let mut groups: Vec<(u32, u8)> = Vec::new();
    let mut push = |vt: u8, groups: &mut Vec<(u32, u8)>| match groups.last_mut() {
        Some((count, t)) if *t == vt => *count += 1,
        _ => groups.push((1, vt)),
    };
    for r in plan.num_params..plan.num_regs {
        push(plan.kinds.valtype(r as usize), &mut groups);
    }
    push(I32, &mut groups); // the dispatch "next block" local at index num_regs
    // Four i64 scratch locals (num_regs+1..=num_regs+4) for integer `pow`'s squaring loop
    // (result, base, exponent, and a product temp distinct from them all).
    push(I64, &mut groups);
    push(I64, &mut groups);
    push(I64, &mut groups);
    push(I64, &mut groups);
    // Seven i32 heap scratch locals (num_regs+5..+11): header/alloc temps and a fill index
    // (+5/+6/+7), two operand-handle holders for `Concat`'s stringify-then-byte-copy (+8/+9), and a
    // Text-keyed-Map per-entry key compare needs +8/+9/+10 — so seven cover the deepest user.
    push(I32, &mut groups);
    push(I32, &mut groups);
    push(I32, &mut groups);
    push(I32, &mut groups);
    push(I32, &mut groups);
    push(I32, &mut groups);
    push(I32, &mut groups);
    // One f64 scratch local (num_regs+12): a Float tuple element loaded from its 8-byte slot needs an
    // `f64` holder before `emit_stringify` (the i64/i32 scratch above cannot type it). `Show`-tuple only.
    push(F64, &mut groups);
    // Two more i32 scratch (num_regs+13/+14): the whole `Seq of Enum` `Show` NESTS an enum display
    // inside the outer sequence loop, so the per-element assembly needs locals distinct from the
    // outer accumulator/counter/handle (+8/+10/+11) — a separator/piece temp and a field-i32 temp.
    push(I32, &mut groups);
    push(I32, &mut groups);

    let mut out = Vec::new();
    leb_u32(&mut out, groups.len() as u32);
    for (count, vt) in groups {
        leb_u32(&mut out, count);
        out.push(vt);
    }
    out
}

/// Encode a UTF-8 name as a wasm name (length-prefixed bytes).
fn encode_name(out: &mut Vec<u8>, name: &str) {
    leb_u32(out, name.len() as u32);
    out.extend_from_slice(name.as_bytes());
}

/// A short, stable name for an op the scalar backend does not lower — what the corpus lock
/// reports as the remaining gap.
fn unsupported_op(op: &Op) -> WasmLowerError {
    let what = match op {
        Op::ExactDiv { .. } => "exact division (Rational)",
        Op::DivPow2 { .. } | Op::MagicDivU { .. } => "oracle division op",
        Op::Concat { .. } => "text op",
        Op::IndexUnchecked { .. } => "unchecked index (IndexUnchecked)",
        Op::CallValue { .. } | Op::MakeClosure { .. } => "closure call",
        _ => "op",
    };
    WasmLowerError::Unsupported(what)
}
