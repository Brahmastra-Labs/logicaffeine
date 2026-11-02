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

use super::cfg::{assemble_dispatch_loop, goto_block, set_next_block, Blocks};
use super::encode::*;
use super::kind::{self, FieldLayout, FieldNested, Kind, KindTable, ParamShape};
use super::regsplit;
use super::WasmLowerError;
use crate::semantics::builtins::BuiltinId;
use crate::vm::instruction::{
    BoundaryType, CompiledFunction, CompiledProgram, Constant, EnumTypeDef, Op, StructTypeDef,
};
use crate::vm::reach::{CodeLayout, HostEntries};
use crate::Interner;
use logicaffeine_language::analysis::{PolicyCondition, PolicyRegistry};

mod absence;
mod display;
mod exact;
mod roots;
mod tower;

type R<T> = std::result::Result<T, WasmLowerError>;

/// Initial linear-memory size, in 64 KiB pages, for a heap-using module (4 MiB). The bump allocator
/// never frees, so this is the working-set ceiling; 4 MiB comfortably holds the build-then-scan
/// programs (n-element arrays, `+`-built strings, per-index Text cuts) at realistic sizes.
const HEAP_PAGES: u32 = 64;

/// ★ THE HEAP GIVES BLOCKS BACK NOW. The self-contained allocator was a pure bump — "8-aligned, no
/// free" — so nothing leaked (every page's per-turn cost is FLAT) and yet a page's lifetime was
/// simply `heap ÷ bytes-per-turn`: measured at ~4,850 turns for a page whose list is rewritten and
/// ~37,000 for a counter, after which it dies with "out of bounds memory access". Every shipping
/// path — the release bundler and the dev server both — is this allocator, so that was what real
/// pages ran on.
///
/// A free block holds the next free pointer in its OWN first word, so the list costs no side table:
/// only one head pointer per size class, in a table at a fixed low address. Linear memory is zeroed
/// at instantiation, so every head starts empty with no init code at all.
const FREE_TABLE_BASE: i32 = 16;

/// ★ CLASSES ARE EXACT, NOT POWERS OF TWO, AND THAT IS THE WHOLE TRICK. The textbook scheme rounds
/// a request up to the next power of two, which would have made every page BIGGER before a single
/// free ever fired — a 3-element Int buffer is 24 bytes and would have become 32. Allocations here
/// are already 8-aligned, so an exact 8-byte-granular class is exactly the size the bump consumed
/// anyway: reuse becomes possible at a cost of precisely zero bytes.
///
/// 128 classes covers every block up to 1 KiB, which is everything a MODEST turn allocates — the
/// whole per-turn budget of the worst-shaped page measured is 864 bytes. A `Set`/`Map` that grows
/// past that (a big cache, an instance store keyed by many components) needs a class too, though:
/// its buffer doubles from 4 elements up, so its byte size past 1 KiB is already an exact power of
/// two (2048, 4096, …) and rounding a one-off large allocation UP to the next one costs nothing this
/// scheme does not already accept for the exact tier below it.
const FREE_CLASSES: i32 = 128;

/// Power-of-two classes for blocks over 1 KiB: class `i` holds blocks of exactly `2^(POWER_MIN + i)`
/// bytes. `POWER_MIN` is the first power at or above the exact tier's ceiling (`FREE_CLASSES * 8`
/// bytes = 1024 = 2^10, so the first power class starts at 2^11 — a block of exactly 1024 bytes is
/// still served by the exact tier). 20 classes reaches 2^30 bytes (1 GiB), far past anything a page
/// allocates in one block; a request bigger than that bumps and is not reclaimed, same as before.
const POWER_CLASSES: i32 = 20;
const POWER_MIN: i32 = 11;

/// Every text literal the module mentions, laid out ONCE in a data section.
///
/// ★ A STATIC STRING USED TO ALLOCATE ON EVERY EVALUATION. `lower_text_literal` built "a fresh Text
/// object in linear memory" each time it ran — two allocations (a 16-byte header and an 8-aligned
/// buffer) plus an `i64.const`/`i64.store` pair per 8 bytes — so `<li>x</li>` minted a new Text for
/// `"x"` on every paint, and the bytes of every literal were re-emitted as CODE. Measured at 24
/// bytes of heap per static text child per row per turn.
///
/// A literal cannot change, so it is written into the module's data section at build time and its
/// handle is a CONSTANT. That is two wins at once: the allocation disappears, and the store pairs
/// collapse to one `i32.const` while the bytes move to a section that holds them once.
///
/// Sound because this tier already treats Text as immutable — `cow_clonable` excludes it, so
/// nothing clones a Text and nothing retains one, which means no code path writes into one.
///
/// Self-contained modules only. In LINKER MODE the memory is imported and the heap is seeded from a
/// runtime slab, so a fixed address here would name someone else's bytes.
#[derive(Default)]
struct TextPool {
    by_bytes: std::collections::HashMap<Vec<u8>, i32>,
    /// The static region's image, laid out from [`STATIC_BASE`].
    image: Vec<u8>,
}

impl TextPool {
    /// The address of this literal's header, laying it out on first sight.
    fn intern(&mut self, bytes: &[u8]) -> i32 {
        if let Some(a) = self.by_bytes.get(bytes) {
            return *a;
        }
        let addr = STATIC_BASE + self.image.len() as i32;
        let len = bytes.len() as i32;
        // `[len][cap][data_ptr][refcount]`, matching what the runtime builder wrote — cap is the
        // length (not the padded size), exactly as before, and the refcount starts at zero.
        self.image.extend_from_slice(&len.to_le_bytes());
        self.image.extend_from_slice(&len.to_le_bytes());
        self.image.extend_from_slice(&(addr + 16).to_le_bytes());
        self.image.extend_from_slice(&0i32.to_le_bytes());
        self.image.extend_from_slice(bytes);
        // Pad so the NEXT header stays 8-aligned, as every heap block is.
        while self.image.len() % 8 != 0 {
            self.image.push(0);
        }
        self.by_bytes.insert(bytes.to_vec(), addr);
        addr
    }
}

/// Where the static text region starts: after the free-list class table (the exact tier, then the
/// power-of-two tier).
const STATIC_BASE: i32 = FREE_TABLE_BASE + (FREE_CLASSES + POWER_CLASSES) * 4;

/// Where the bump region starts: after the class table. (It was 16 — "the low 16 bytes stay
/// reserved/null" — and the table now sits in the space above that.)
const HEAP_START: i32 = STATIC_BASE;

/// Reserved linear-memory slot (an `i32`) holding the offline net-inbox FIFO handle — lives in the
/// null-reserved low-16 region (bytes 0..16 are never bump-allocated), so it never collides with the
/// heap. `Listen` writes the handle here; `Send`/`Stream`/`Await` read it (see the net-op lowerings).
const NET_INBOX_ADDR: i32 = 8;

/// ★ ONE ROW PER HOST IMPORT — the enum, the canonical order, the field name, and the
/// signature, from one place.
///
/// THERE WERE FOUR OF THESE, side by side, 199 entries each: the `HostFn` enum, the
/// `HOST_FNS` order that fixes import indices, `field()` naming each one for the host, and
/// `params()`/`results()` giving its signature. Adding one DOM operation meant editing all
/// four, and forgetting one of them did not fail to compile — the module simply failed to
/// INSTANTIATE at runtime, in the browser, with no diagnostic pointing anywhere near here.
///
/// The declaration order and `HOST_FNS` order had already drifted apart (they agreed for 16
/// entries and then did not), which cost nothing only because `HostFn` derives neither `Ord`
/// nor `Hash` and the emitter re-sorts into `HOST_FNS` order before emitting. That is a
/// property nobody was holding on purpose.
///
/// Now a row is the unit: name it once, and it is in the enum, in the order, named, and
/// typed. A row that is missing is a row that does not exist.
macro_rules! host_fns {
    ($( $(#[$attr:meta])* $variant:ident = $field:literal, ($($p:ident),*) -> ($($r:ident),*); )*) => {
        /// A host function the emitted module imports (under the `env` namespace).
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum HostFn { $( $(#[$attr])* $variant, )* }

        /// The canonical order of every host import, which fixes their indices in an emitted
        /// module. Derived from the table, so it can no longer disagree with the enum, and no
        /// length is written down: this was `[HostFn; 188]` and the literal drifted from the
        /// list it sat above, breaking a teammate's whole test pass with "expected an array
        /// with a size of 188, found one with a size of 189" — true, and teaching nothing.
        const HOST_FNS: &[HostFn] = &[ $( HostFn::$variant, )* ];

        impl HostFn {
            fn field(self) -> &'static str {
                match self { $( HostFn::$variant => $field, )* }
            }
            fn params(self) -> Vec<u8> {
                match self { $( HostFn::$variant => vec![$($p),*], )* }
            }
            fn results(self) -> Vec<u8> {
                match self { $( HostFn::$variant => vec![$($r),*], )* }
            }
            /// The positions of the parameters the host KEEPS after the call returns — a
            /// `CALLBACK` it will re-enter through, or a `KEPT` handle it stores. Read off the row's
            /// own parameter spelling, so a new import that keeps a handle says so where it is
            /// declared.
            fn retained_params(self) -> Vec<usize> {
                let spelled: &[&str] = match self { $( HostFn::$variant => &[$(stringify!($p)),*], )* };
                spelled
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| **p == "CALLBACK" || **p == "KEPT")
                    .map(|(i, _)| i)
                    .collect()
            }
            /// Whether the host re-enters the module through a closure handed to this import.
            fn registers_callback(self) -> bool {
                let spelled: &[&str] = match self { $( HostFn::$variant => &[$(stringify!($p)),*], )* };
                spelled.contains(&"CALLBACK")
            }
            /// Whether this import answers with a Text the host allocates in the module's heap
            /// through `__logos_text_new` — read off the row's own result spelling.
            fn returns_host_text(self) -> bool {
                let spelled: &[&str] = match self { $( HostFn::$variant => &[$(stringify!($r)),*], )* };
                spelled.contains(&"HOST_TEXT")
            }
            /// Whether this import answers with a sequence the host builds in the module's heap
            /// through `__logos_seq_new` — read off the row's own result spelling.
            fn returns_host_seq(self) -> bool {
                let spelled: &[&str] = match self { $( HostFn::$variant => &[$(stringify!($r)),*], )* };
                spelled.contains(&"HOST_SEQ")
            }
        }
    };
}

/// A closure handle the host keeps and later calls back through `__logos_call_closure`. An `i32`
/// on the wire; spelled apart in the table so the host's holds are part of the ABI.
const CALLBACK: u8 = I32;

/// A Text the HOST writes into the module's heap through `__logos_text_new` and hands over — an
/// element's text, a response body, the route. An `i32` handle on the wire; spelled apart in the
/// table so a module that takes one carries the allocator the host writes it through.
const HOST_TEXT: u8 = I32;

/// A sequence the HOST builds in the module's heap through `__logos_seq_new` and hands over — a
/// dropped file's bytes. An `i32` handle on the wire, one ordinary `Seq` header over 8-byte slots;
/// spelled apart in the table so a module that takes one carries the allocator the host fills it
/// through. The element kind is the importing builtin's declared result kind, never the host's.
const HOST_SEQ: u8 = I32;

host_fns! {
    PrintI64 = "print_i64", (I64) -> ();
    PrintBool = "print_bool", (I32) -> ();
    /// `print_char(code: i64)` — display one Unicode scalar. The host reconstructs `char` from the
    /// code point and emits its UTF-8 bytes (a `RuntimeValue::Char` displays as the character
    /// itself), so `Show \`a\`` prints `a`, not the numeric code point.
    PrintChar = "print_char", (I64) -> ();
    PrintF64 = "print_f64", (F64) -> ();
    /// `print_date(days: i32)` / `print_moment(nanos: i64)` — display a temporal value (the host
    /// formats via `RuntimeValue::Date`/`Moment`'s `to_display_string`, so no reimplementation).
    PrintDate = "print_date", (I32) -> ();
    PrintMoment = "print_moment", (I64) -> ();
    /// `print_duration(nanos: i64)` / `print_time(nanos: i64)` — display a `RuntimeValue::Duration`
    /// (magnitude-bucketed `5s`/`3h`/…) / `Time` (`HH:MM:SS[.frac]`), host-formatted like the VM.
    PrintDuration = "print_duration", (I64) -> ();
    PrintTime = "print_time", (I64) -> ();
    /// `pow_ff(base: f64, exp: f64) -> f64` — `f64::powf`, for a Float exponent.
    PowFf = "pow_ff", (F64,F64) -> (F64);
    /// `pow_fi(base: f64, exp: i64) -> f64` — `f64::powi`, for an Int exponent (exact repeated
    /// multiplication, NOT `powf` — the two differ in the last bit).
    PowFi = "pow_fi", (F64,I64) -> (F64);
    /// `today() -> i32` (days since epoch) / `now() -> i64` (nanos since epoch) — the clock,
    /// honoring the test fixed-clock so tw==vm==wasm.
    Today = "today", () -> (I32);
    Now = "now", () -> (I64);
    /// `random_int(lo: i64, hi: i64) -> i64` — `randomInt(lo, hi)`: a uniform draw from `lo..=hi`.
    /// Randomness is the host's to supply (a browser's `crypto.getRandomValues`, a test host's seeded
    /// generator); the bounds are the program's.
    RandomInt = "random_int", (I64,I64) -> (I64);
    /// `print_seq_i64(handle: i32)` / `print_seq_f64(handle: i32)` — display a whole sequence. The
    /// host reads the stable header `[len][cap][data_ptr]` + the element buffer out of the
    /// exported linear memory and formats `[e0, e1, …]` exactly as `RuntimeValue::List`'s
    /// `to_display_string` (each element by its scalar display), so no format reimplementation.
    PrintSeqI64 = "print_seq_i64", (I32) -> ();
    /// `print_seq_bool(handle: i32)` — like [`PrintSeqI64`] but each i64-0/1 element renders as
    /// `true`/`false` (a `Seq of Bool` = `RuntimeValue::List` of `ListRepr::Bools`), so the whole
    /// sequence displays `[true, false, …]` rather than `[1, 0, …]`.
    PrintSeqBool = "print_seq_bool", (I32) -> ();
    /// `print_seq_word32(handle: i32)` / `print_seq_word64(handle: i32)` — display a whole `Seq of
    /// Word32`/`Word64` (a crypto state array) as `[u0, u1, …]` with each element its UNSIGNED decimal
    /// (`4294967295`, not `-1`). Word32 elements ride the low word of their 8-byte slot; Word64 the full
    /// slot — matching `RuntimeValue::List` of `Word`.
    PrintSeqWord32 = "print_seq_word32", (I32) -> ();
    PrintSeqWord64 = "print_seq_word64", (I32) -> ();
    PrintSeqF64 = "print_seq_f64", (I32) -> ();
    /// `print_text(handle: i32)` — display a UTF-8 string. The host reads the header `[len][cap]
    /// [data_ptr]` + the `len` bytes at `data_ptr` and emits them verbatim (a `RuntimeValue::Text`
    /// displays as its raw contents), so no formatting reimplementation.
    PrintText = "print_text", (I32) -> ();
    /// `print_seq_text(handle: i32)` — display a whole sequence of Text. The host reads the seq
    /// header + each element (a Text handle) and formats `[s0, s1, …]` (the elements unquoted,
    /// comma-space separated), matching `RuntimeValue::List`'s `to_display_string` for Text elements.
    PrintSeqText = "print_seq_text", (I32) -> ();
    /// `print_set_i64(handle: i32)` — display a whole `Set of Int` as `{e0, e1, …}`. The VM's `Set`
    /// is an insertion-ordered `Vec` (NOT a hashset), and the AOT set stores elements in that same
    /// order, so the display is deterministic and byte-identical. (A whole `Map` displays the same way
    /// via `lower_show_map`: the VM's `MapStorage` is an insertion-ordered `IndexMap`, matching the
    /// AOT's append order, so `{k: v, …}` is byte-identical too.)
    PrintSetI64 = "print_set_i64", (I32) -> ();
    /// `fmt_i64_into(buf: i32, val: i64) -> i32` — write the decimal of `val`
    /// (`RuntimeValue::Int(val).to_display_string()`) into `buf` and return the byte length. Used
    /// to stringify an Int operand of a `Concat` (string interpolation `"… {n} …"`). `buf` is a
    /// module-allocated 24-byte scratch (an i64 decimal is ≤ 20 chars + sign).
    FmtI64Into = "fmt_i64_into", (I32,I64) -> (I32);
    /// `fmt_f64_into(buf: i32, val: f64) -> i32` — like [`FmtI64Into`] for a Float operand (the
    /// shared shortest-round-trip display, `logicaffeine_data::fmt::fmt_f64`); `buf` is a 340-byte
    /// scratch (the widest possible output, the smallest subnormal, is ~326 bytes — spec-locked
    /// by `fmt::tests::worst_case_width_fits_wasm_scratch`).
    FmtF64Into = "fmt_f64_into", (I32,F64) -> (I32);
    /// `fmt_bool_into(buf: i32, val: i32) -> i32` — writes `"true"`/`"false"`; `buf` is 8 bytes.
    FmtBoolInto = "fmt_bool_into", (I32,I64) -> (I32);
    /// `fmt_f64_prec_into(buf: i32, val: f64, prec: i32) -> i32` — write `format!("{:.prec}", val)`
    /// (the interpolation `.N` precision spec `"{x:.9}"` → `apply_format_spec`) and return the length.
    /// `buf` is a `340 + prec`-byte scratch (worst-case `f64` integer width + the decimals).
    FmtF64PrecInto = "fmt_f64_prec_into", (I32,F64,I32) -> (I32);
    /// `fmt_align_into(buf: i32, text: i32, width: i32, align: i32) -> i32` — pad the `Text` `text`'s
    /// display to `width` (space fill, char-counted) into `buf`, returning the byte length. `align`
    /// selects `format!("{:>w$}", …)` (0, right — also the bare-width `{x:6}`), `{:<w$}` (1, left), or
    /// `{:^w$}` (2, center) — the SAME Rust `format!` `apply_format_spec` runs, so bit-identical. `buf`
    /// is sized `text_len + width` (padding adds at most `width` single-byte spaces).
    FmtAlignInto = "fmt_align_into", (I32,I32,I32,I32) -> (I32);
    /// `args() -> i32` — the command-line arguments as a `Seq of Text` handle (the host builds the
    /// argv sequence in the module's linear memory and returns its handle), so a program reading its
    /// problem size from argv (`parseInt(item 2 of args())`) compiles to standalone wasm.
    Args = "args", () -> (I32);
    /// `read_reals(path: i32) -> i32` — `readReals(path)`: the file's little-endian binary64 values as a
    /// `Seq of Real` handle the host builds in a page it grows, refused as `logicaffeine_system::file::read_reals`
    /// refuses (a file it cannot read, one that is not a whole number of values).
    ReadReals = "read_reals", (I32) -> (I32);
    /// `parse_int(handle: i32) -> i64` — parse a `Text` (UTF-8 in linear memory) to an Int, exactly as
    /// the VM's `BuiltinId::ParseInt` (`str::parse::<i64>`), trapping on a non-numeric string.
    ParseInt = "parse_int", (I32) -> (I64);
    /// `text_contains(haystack: i32, needle: i32) -> i32` — 1 iff the `needle` Text is a substring of
    /// the `haystack` Text, exactly as the VM's `BuiltinId::Contains` (`str::contains`).
    TextContains = "text_contains", (I32,I32) -> (I64);
    /// `fmt_seq_i64_into(buf: i32, handle: i32) -> i32` — write a whole `Seq of Int`'s display
    /// `[e0, e1, …]` (`RuntimeValue::List::to_display_string`) into `buf` and return the byte length.
    /// Used to stringify a SEQUENCE operand of a `+`/`format` (`"Positives: " + positives`). `buf` is
    /// sized `len*24 + 8` (each i64 ≤ 20 digits + sign + `", "`, plus the brackets).
    FmtSeqI64Into = "fmt_seq_i64_into", (I32,I32) -> (I32);
    /// `fmt_seq_bool_into(buf: i32, handle: i32) -> i32` — like [`FmtSeqI64Into`] for a `Seq of Bool`,
    /// rendering each i64-0/1 element as `true`/`false` (`buf` sized `len*7 + 8` — `"false"` + `", "`).
    /// Used to stringify a whole bool sequence operand of a `+`/`format`.
    FmtSeqBoolInto = "fmt_seq_bool_into", (I32,I32) -> (I32);
    /// `fmt_set_i64_into(buf: i32, handle: i32) -> i32` — like [`FmtSeqI64Into`] for a `Set of Int`,
    /// whose display is `{e0, e1, …}` (insertion order, matching the VM's `Vec`-backed Set).
    FmtSetI64Into = "fmt_set_i64_into", (I32,I32) -> (I32);
    /// `print_set_text(handle: i32)` — display a whole `Set of Text` as `{s0, s1, …}` (elements
    /// unquoted, insertion order). Like [`PrintSetI64`] but each slot's low word is a `Text` handle
    /// the host reads out of memory — matching `RuntimeValue::Set::to_display_string` for Text.
    PrintSetText = "print_set_text", (I32) -> ();
    /// `print_rational(num: i64, den: i64)` — display an exact `Rational` as `num/den`, or just `num`
    /// when `den == 1` (matching the VM, whose `from_rational` downsizes a whole quotient to an `Int`).
    PrintRational = "print_rational", (I64,I64) -> ();
    /// `print_nothing()` — display the Optional null value as "nothing" (matching the tree-walker's
    /// `RuntimeValue::Nothing` display). The `Some` arm of an `Optional` `Show` uses the inner scalar's
    /// own sink instead; this covers the null-handle arm.
    PrintNothing = "print_nothing", () -> ();
    /// `parse_float(handle: i32) -> f64` — parse a `Text` to a Float, exactly as the VM's
    /// `BuiltinId::ParseFloat` (`str::trim().parse::<f64>`), trapping on a non-numeric string.
    ParseFloat = "parse_float", (I32) -> (F64);
    /// `parse_timestamp(handle: i32) -> i64` — parse an RFC-3339 `Text` to a `Moment` (nanoseconds
    /// since the epoch) via `temporal::parse_rfc3339`, trapping on a malformed timestamp.
    ParseTimestamp = "parse_timestamp", (I32) -> (I64);
    /// `temporal_component(nanos: i64, which: i32) -> i64` — one calendar/clock component of a `Moment`
    /// (`which`: 0 year, 1 month, 2 day, 3 hour, 4 minute, 5 second, 6 weekday, 7 iso-week, 8 quarter),
    /// computed by the SAME `temporal::civil_from_unix_nanos`/`weekday_from_days`/`iso_week_from_days`
    /// the VM uses — so `the year of m` is bit-identical across tiers.
    TemporalComponent = "temporal_component", (I64,I32) -> (I64);
    /// `temporal_component_date(days: i32, which: i32) -> i64` — the calendar component of a `Date`
    /// (days since the epoch), `which` in {0 year, 1 month, 2 day, 6 weekday, 7 iso-week, 8 quarter}.
    /// Computed straight from `temporal::civil_from_days`/`weekday_from_days`/`iso_week_from_days` (the
    /// VM's exact Date path) over the full `i32` day range — no `Moment` nanos round-trip (which would
    /// overflow `i64` past ~year 2262). Clock components (hour/minute/second) don't apply to a `Date`
    /// and are refused at lowering, matching the VM's runtime error.
    TemporalComponentDate = "temporal_component_date", (I32,I32) -> (I64);
    /// `print_word(v: i64)` — display a Word as its UNSIGNED decimal (`(v as u64).to_string()`, matching
    /// `WordVal`'s Display). A `Word32` is zero-extended to `i64` (`i64.extend_i32_u`) before the call,
    /// so its top 32 bits are zero and the `u64` reading is exactly the `u32` value.
    PrintWord = "print_word", (I64) -> ();
    /// `logos_rt_bigint_from_i64(x: i64) -> i32` — the linked `logicaffeine_base::BigInt` runtime: box a
    /// scalar into a BigInt handle. LINKER MODE ONLY: an undefined symbol `rust-lld` resolves against the
    /// prebuilt base runtime object (there is no `env` host behind it), so it never appears in a
    /// standalone module. The three `Bigint*` sinks together lower an overflowing integer `Op::Pow`.
    BigintFromI64 = "logos_rt_bigint_from_i64", (I64) -> (I32);
    /// `logos_rt_bigint_pow(base: i32, exp: i64) -> i32` — raise a BigInt handle to an integer power,
    /// returning a fresh handle (the exact big integer, no overflow). Linker mode only.
    BigintPow = "logos_rt_bigint_pow", (I32,I64) -> (I32);
    /// `logos_rt_bigint_to_text(h: i32) -> i32` — render a BigInt handle to a `Text` handle laid out in
    /// the emitter's `[len][cap][data_ptr][refcount]` ABI in the shared linear memory, so a `Show` reads
    /// it through the ordinary `print_text` path. Linker mode only.
    BigintToText = "logos_rt_bigint_to_text", (I32) -> (I32);
    /// `logos_rt_bigint_mul(a: i32, b: i32) -> i32` — exact big-integer multiply of two BigInt handles,
    /// returning a fresh handle. Lets `(2^100) * (3^50)` keep computing on real BigInts. Linker only.
    BigintMul = "logos_rt_bigint_mul", (I32,I32) -> (I32);
    /// `logos_rt_bigint_add(a: i32, b: i32) -> i32` — exact big-integer addition of two handles. Linker only.
    BigintAdd = "logos_rt_bigint_add", (I32,I32) -> (I32);
    /// `logos_rt_bigint_sub(a: i32, b: i32) -> i32` — exact big-integer subtraction (may be negative;
    /// `to_text` renders the sign). Linker only.
    BigintSub = "logos_rt_bigint_sub", (I32,I32) -> (I32);
    /// `logos_rt_bigint_div(a: i32, b: i32) -> i32` — exact big-integer quotient (`div_rem().0`, the SAME
    /// truncating division the VM uses), traps on a zero divisor. Linker only.
    BigintDiv = "logos_rt_bigint_div", (I32,I32) -> (I32);
    /// `logos_rt_bigint_mod(a: i32, b: i32) -> i32` — exact big-integer remainder (`div_rem().1`, matching
    /// the VM's `the remainder of a and b`), traps on a zero divisor. Linker only.
    BigintMod = "logos_rt_bigint_mod", (I32,I32) -> (I32);
    /// `logos_rt_complex_from_i64(re: i64, im: i64) -> i32` — build an EXACT `Complex` (Rational
    /// components) from two integer parts, returning an i32 handle. `logos_rt_complex_{add,sub,mul}(a,
    /// b) -> i32` do exact complex arithmetic; `logos_rt_complex_to_text(h) -> i32` renders it (`re±imi`)
    /// to a Text handle. Linker mode only (the exact Rational-backed runtime, mirroring the BigInt ABI).
    ComplexFromI64 = "logos_rt_complex_from_i64", (I64,I64) -> (I32);
    ComplexAdd = "logos_rt_complex_add", (I32,I32) -> (I32);
    ComplexSub = "logos_rt_complex_sub", (I32,I32) -> (I32);
    ComplexMul = "logos_rt_complex_mul", (I32,I32) -> (I32);
    ComplexToText = "logos_rt_complex_to_text", (I32) -> (I32);
    /// `logos_rt_complex_abs(h: i32) -> f64` — the `f64` modulus `|z|`. The one complex op returning a
    /// Float rather than an i32 handle.
    ComplexAbs = "logos_rt_complex_abs", (I32) -> (F64);
    /// The FLOAT-complex tier (`complex(realExpr, realExpr)` → a `ComplexF`): `logos_rt_complex_from_f64(
    /// re: f64, im: f64) -> i32` builds the handle (parts as exact dyadic Rationals), and
    /// `logos_rt_complex_re_f64`/`_im_f64(h: i32) -> f64` extract the parts for `re`/`im`.
    ComplexFromF64 = "logos_rt_complex_from_f64", (F64,F64) -> (I32);
    ComplexReF64 = "logos_rt_complex_re_f64", (I32) -> (F64);
    ComplexImF64 = "logos_rt_complex_im_f64", (I32) -> (F64);
    /// `logos_rt_modular_from_i64(v: i64, n: i64) -> i32` / `_add`/`_sub`/`_mul`/`_to_text` — the ℤ/nℤ
    /// analog of the Complex ABI over `logicaffeine_base::Modular` (i32 handle). Linker mode only.
    ModularFromI64 = "logos_rt_modular_from_i64", (I64,I64) -> (I32);
    ModularAdd = "logos_rt_modular_add", (I32,I32) -> (I32);
    ModularSub = "logos_rt_modular_sub", (I32,I32) -> (I32);
    ModularMul = "logos_rt_modular_mul", (I32,I32) -> (I32);
    ModularDiv = "logos_rt_modular_div", (I32,I32) -> (I32);
    ModularPow = "logos_rt_modular_pow", (I32,I64) -> (I32);
    ModularToText = "logos_rt_modular_to_text", (I32) -> (I32);
    /// `logos_rt_decimal_from_text(h)` parses a Text handle → exact Decimal; `_from_i64(x)` promotes an
    /// Int; `_add`/`_sub`/`_mul`/`_to_text` are the exact base-10 ABI over `base::Decimal`. Linker only.
    DecimalFromText = "logos_rt_decimal_from_text", (I32) -> (I32);
    DecimalFromI64 = "logos_rt_decimal_from_i64", (I64) -> (I32);
    DecimalAdd = "logos_rt_decimal_add", (I32,I32) -> (I32);
    DecimalSub = "logos_rt_decimal_sub", (I32,I32) -> (I32);
    DecimalMul = "logos_rt_decimal_mul", (I32,I32) -> (I32);
    DecimalToText = "logos_rt_decimal_to_text", (I32) -> (I32);
    /// `logos_rt_money_from_decimal(dec, cur)` / `_from_i64(v, cur)` build a Money (currency Text read
    /// from shared memory); `_add`/`_sub` require matching currency; `_to_text` renders it. Linker only.
    MoneyFromDecimal = "logos_rt_money_from_decimal", (I32,I32) -> (I32);
    MoneyFromI64 = "logos_rt_money_from_i64", (I64,I32) -> (I32);
    MoneyAdd = "logos_rt_money_add", (I32,I32) -> (I32);
    MoneySub = "logos_rt_money_sub", (I32,I32) -> (I32);
    MoneyToText = "logos_rt_money_to_text", (I32) -> (I32);
    QuantityOfI64 = "logos_rt_quantity_of_i64", (I64,I32) -> (I32);
    QuantityConvert = "logos_rt_quantity_convert", (I32,I32) -> (I32);
    QuantityAdd = "logos_rt_quantity_add", (I32,I32) -> (I32);
    QuantitySub = "logos_rt_quantity_sub", (I32,I32) -> (I32);
    QuantityMul = "logos_rt_quantity_mul", (I32,I32) -> (I32);
    QuantityDiv = "logos_rt_quantity_div", (I32,I32) -> (I32);
    QuantityToText = "logos_rt_quantity_to_text", (I32) -> (I32);
    RationalFromI64 = "logos_rt_rational_from_i64", (I64) -> (I32);
    RationalFromBigint = "logos_rt_rational_from_bigint", (I32) -> (I32);
    RationalAdd = "logos_rt_rational_add", (I32,I32) -> (I32);
    RationalSub = "logos_rt_rational_sub", (I32,I32) -> (I32);
    RationalMul = "logos_rt_rational_mul", (I32,I32) -> (I32);
    RationalDiv = "logos_rt_rational_div", (I32,I32) -> (I32);
    RationalToText = "logos_rt_rational_to_text", (I32) -> (I32);
    RationalFloor = "logos_rt_rational_floor", (I32) -> (I32);
    RationalCeil = "logos_rt_rational_ceil", (I32) -> (I32);
    RationalRound = "logos_rt_rational_round", (I32) -> (I32);
    RationalAbs = "logos_rt_rational_abs", (I32) -> (I32);
    UuidParse = "logos_rt_uuid_parse", (I32) -> (I32);
    UuidNil = "logos_rt_uuid_nil", () -> (I32);
    UuidMax = "logos_rt_uuid_max", () -> (I32);
    UuidDns = "logos_rt_uuid_dns", () -> (I32);
    UuidUrl = "logos_rt_uuid_url", () -> (I32);
    UuidOid = "logos_rt_uuid_oid", () -> (I32);
    UuidX500 = "logos_rt_uuid_x500", () -> (I32);
    UuidVersion = "logos_rt_uuid_version", (I32) -> (I64);
    UuidEq = "logos_rt_uuid_eq", (I32,I32) -> (I32);
    UuidToText = "logos_rt_uuid_to_text", (I32) -> (I32);
    UuidFromPtr = "logos_rt_uuid_from_ptr", (I32) -> (I32);
    PrintSpan = "print_span", (I64) -> ();
    MomentAddSpan = "logos_rt_moment_add_span", (I64,I32,I32) -> (I64);
    DateAddSpan = "logos_rt_date_add_span", (I32,I32,I32) -> (I32);
    FormatTimestampRt = "logos_rt_format_timestamp", (I64) -> (I32);
    MonthsBetweenRt = "logos_rt_months_between", (I64,I64) -> (I64);
    YearsBetweenRt = "logos_rt_years_between", (I64,I64) -> (I64);
    MakeDateRt = "logos_rt_make_date", (I64,I64,I64) -> (I32);
    MakeMomentRt = "logos_rt_make_moment", (I64,I64,I64,I64,I64,I64) -> (I64);
    InZoneRt = "logos_rt_in_zone", (I64,I32) -> (I32);
    LocalInstantRt = "logos_rt_local_instant", (I64,I32) -> (I64);
    Lanes16FromBytes = "logos_rt_lanes16_from_bytes", (I32) -> (I32);
    Lanes8FromWords = "logos_rt_lanes8_from_words", (I32) -> (I32);
    Lanes4W64FromWords = "logos_rt_lanes4w64_from_words", (I32) -> (I32);
    LanesSplat16 = "logos_rt_lanes_splat16", (I64) -> (I32);
    LanesSplat8 = "logos_rt_lanes_splat8", (I64) -> (I32);
    LanesToSeq = "logos_rt_lanes_to_seq", (I32) -> (I32);
    LanesShuffle = "logos_rt_lanes_shuffle", (I32,I32) -> (I32);
    LanesInterleaveLo = "logos_rt_lanes_interleave_lo", (I32,I32) -> (I32);
    LanesInterleaveHi = "logos_rt_lanes_interleave_hi", (I32,I32) -> (I32);
    LanesByteAdd = "logos_rt_lanes_byte_add", (I32,I32) -> (I32);
    LanesMaddubs = "logos_rt_lanes_maddubs", (I32,I32) -> (I32);
    LanesPackus = "logos_rt_lanes_packus", (I32,I32) -> (I32);
    LanesShrBytes = "logos_rt_lanes_shr_bytes", (I32,I64) -> (I32);
    DecimalToRational = "logos_rt_decimal_to_rational", (I32) -> (I32);
    MoneySetRate = "logos_rt_set_rate", (I32,I32) -> (I32);
    MoneyToCurrency = "logos_rt_to_currency", (I32,I32) -> (I32);
    MoneySetRatesInt = "logos_rt_set_rates_int", (I32) -> (I32);
    MoneySetRatesRational = "logos_rt_set_rates_rational", (I32) -> (I32);
    MoneySetRatesDecimal = "logos_rt_set_rates_decimal", (I32) -> (I32);
    WriteWireResidual = "write_wire_residual", (I32,I32) -> (I64);
    /// `logos_rt_fail(text) -> ()` — the message of a runtime failure, handed to the host
    /// immediately before the `unreachable` that ends the program. The trap is the contract
    /// (`wasm_traps_where_treewalker_errors`); this only stops the host having to guess WHY.
    LogosRtFail = "logos_rt_fail", (I32) -> ();
    /// `logos_rt_fail_at(code: i32, value: i64) -> ()` — an INLINE check failing (a bounds
    /// test, a division guard). Its message needs a value that only exists at run time, so
    /// there is no constant to hand over: the module reports the code and the value, and the
    /// host renders the sentence through `semantics::failcode::message`.
    LogosRtFailAt = "logos_rt_fail_at", (I32,I64) -> ();
    WireBytesInt = "logos_rt_wire_bytes_int", (I64) -> (I32);
    WireBytesBool = "logos_rt_wire_bytes_bool", (I64) -> (I32);
    WireBytesFloat = "logos_rt_wire_bytes_float", (F64) -> (I32);
    WireBytesText = "logos_rt_wire_bytes_text", (I32) -> (I32);
    ReadWireFrame = "read_wire_frame", (I32,I32) -> (I32);
    ReadWireProgramRt = "logos_rt_read_wire_program", (I32,I32) -> (I32);
    DynamicToText = "logos_rt_dynamic_to_text", (I32) -> (I32);
    RunAccepted = "logos_rt_run_accepted", (I32,I64,I64,I64) -> (I64);
    Sha1Rnds4 = "logos_rt_sha1rnds4", (I32,I32,I64) -> (I32);
    Sha1Msg1 = "logos_rt_sha1msg1", (I32,I32) -> (I32);
    Sha1Msg2 = "logos_rt_sha1msg2", (I32,I32) -> (I32);
    Sha1Nexte = "logos_rt_sha1nexte", (I32,I32) -> (I32);
    Lanes4Add = "logos_rt_lanes4_add", (I32,I32) -> (I32);
    Lanes4Xor = "logos_rt_lanes4_xor", (I32,I32) -> (I32);
    /// `logos_rt_alloc(size: i32) -> i32` — a raw 8-aligned block from the runtime's allocator. Linker
    /// mode seeds the emitter's bump allocator (`__heap_ptr`) from ONE such SLAB at a `main` prologue, so
    /// the emitter's heap and the runtime's `dlmalloc` never overlap in the shared linear memory.
    RtAlloc = "logos_rt_alloc", (I32) -> (I32);
    /// `obs(site_id: i32)` — emit the observation recorded at `site_id` (Wave 13,
    /// OBSERVED profile only). The host resolves the category / message / span kind
    /// from the module's `observation_sites` table (it compiled the program), so
    /// only the site index crosses the boundary — the browser-tracing analog of
    /// the VM's `Op::Obs`. (Payload VALUES are a follow-up: they need linear-memory
    /// marshaling; the event STREAM — categories, messages, balanced spans —
    /// emits here.)
    Obs = "obs", (I32,I64,I64,I64) -> ();
    // --- DOM (the browser tier). Element/Event handles are i64; Text args are i32
    // handles into linear memory. A `MockDom` provides these in tests; the real
    // browser DOM in production. ---
    /// `dom_body() -> i64` — the document body handle.
    DomBody = "dom_body", () -> (I64);
    /// `dom_create_element(tag: i32 Text) -> i64` — `document.createElement`.
    DomCreateElement = "dom_create_element", (I32) -> (I64);
    /// `dom_create_text_node(text: i32 Text) -> i64` — `document.createTextNode`.
    DomCreateTextNode = "dom_create_text_node", (I32) -> (I64);
    /// `dom_set_text(el: i64, text: i32 Text)` — set `textContent`.
    DomSetText = "dom_set_text", (I64,I32) -> ();
    /// `dom_set_html(el: i64, html: i32 Text)` — set `innerHTML` (raw markup).
    DomSetHtml = "dom_set_html", (I64,I32) -> ();
    /// `dom_clear_children(el: i64)` — remove every child (re-render primitive).
    DomClearChildren = "dom_clear_children", (I64) -> ();
    /// `dom_on_message(topic: i32, handler: i32)` — subscribe to a relay topic.
    ///
    /// The last trigger with no tier. The relay itself was already built and shared between the
    /// native node and the browser (`logicaffeine_system::relay`), so this carries a topic and a
    /// closure and nothing else: which topic, and what a message means, are compiled LOGOS.
    DomOnMessage = "dom_on_message", (I32, CALLBACK) -> ();
    /// `dom_set_attribute(el: i64, name: i32 Text, value: i32 Text)` — `setAttribute`.
    DomSetAttribute = "dom_set_attribute", (I64,I32,I32) -> ();
    /// `dom_set_bool_attribute(el: i64, name: i32 Text, on: i64)` — an HTML boolean attribute
    /// (`disabled`/`checked`/`required`/…, see `lgx/html.rs::is_boolean_attribute`) is a PRESENCE,
    /// not a string: `setAttribute(name, "")` when `on`, `removeAttribute(name)` otherwise (writing
    /// `"false"` through the string-valued `dom_set_attribute` renders `disabled="false"`, which
    /// every browser reads as disabled — the exact bug this import exists to avoid). `lgx_lower.rs`
    /// deliberately hands a boolean attribute's raw Bool value to the SAME `setAttribute` builtin
    /// call (see its own comment there); this backend reads the argument's KIND at the call site
    /// (`Op::CallBuiltin`'s used-import scan and `lower_dom_call`'s dispatch both switch on it) and
    /// routes a Bool-valued one here instead of `DomSetAttribute`, since a `Bool` is an `i64` in
    /// this backend's value model and `dom_set_attribute`'s own ABI is fixed at `i32 Text`.
    DomSetBoolAttribute = "dom_set_bool_attribute", (I64,I32,I64) -> ();
    /// `dom_append_child(parent: i64, child: i64)` — `appendChild`.
    DomAppendChild = "dom_append_child", (I64,I64) -> ();
    /// `dom_get_text(el: i64) -> i32 Text` — read `textContent`.
    DomGetText = "dom_get_text", (I64) -> (HOST_TEXT);
    /// `dom_get_value(el: i64) -> i32 Text` — read a form input's `value`.
    DomGetValue = "dom_get_value", (I64) -> (HOST_TEXT);
    /// `dom_set_value(el: i64, value: i32 Text)` — set a form input's `value`.
    DomSetValue = "dom_set_value", (I64,I32) -> ();
    /// `dom_set_checked(el, on)` — a checkbox's or radio's live CHECKED state.
    ///
    /// Not `dom_set_attribute(el, "checked", …)`: the attribute is `defaultChecked`, so the host
    /// must write the PROPERTY or the box a visitor sees never changes. `on` is a Bool, which is
    /// an `i64` in the value model.
    DomSetChecked = "dom_set_checked", (I64,I64) -> ();
    DomPreventDefault = "dom_prevent_default", (I64,I32) -> ();
    /// `storage_set(area, key, value)` / `storage_get(area, key)` / `storage_remove(area, key)` —
    /// browser storage. WITHOUT THESE A PAGE CANNOT PERSIST ANYTHING: a `## Store … persisted as`
    /// kept its state in the module's own memory, which dies with the page, while the sentence
    /// said otherwise. `storage_get` answers the i32 Text handle, or `0` for a key that is not
    /// there — an absence a caller must be able to tell from a stored empty string.
    StorageSet = "storage_set", (I32,I32,I32) -> ();
    StorageGet = "storage_get", (I32,I32) -> (HOST_TEXT);
    StorageRemove = "storage_remove", (I32,I32) -> ();
    /// `dom_apply(patches)` — apply a whole DOM patch stream in ONE call.
    ///
    /// The bulk form the browser host is built around: the module hands over a buffer of
    /// operations and the host walks it, deciding nothing. `patches` is the handle of a
    /// `Seq of Int` whose elements are bytes — the same seq layout `dom_fetch_send` already
    /// reads its headers through, so the host needs no new way to see into memory.
    DomApply = "dom_apply", (I32) -> ();
    /// `dom_apply_words(words: i32 Seq of Int, word_count: i64, strings: i32 Seq of Text,
    /// string_count: i64) -> i64` — the word-format (wire v2) twin of `dom_apply`: the first
    /// `word_count` words, whose text operands index the first `string_count` strings, applied by
    /// the node ids they name. Answers how many adopted nodes did not match.
    DomApplyWords = "dom_apply_words", (I32,I64,I32,I64) -> (I64);
    /// `dom_query_selector(sel: i32 Text) -> i64` — first match (`-1` if none).
    DomQuerySelector = "dom_query_selector", (I32) -> (I64);
    /// `dom_add_event_listener(el: i64, event: i32 Text, handler: i32 closure)` —
    /// register the closure handle; the host fires it via `__logos_call_closure`.
    DomAddEventListener = "dom_add_event_listener", (I64,I32,CALLBACK) -> ();
    /// `dom_listen_flags(el: i64, event: i32 Text, flags: i64)` — how the browser attaches `el`'s
    /// listener for `event`: the modifier bits `semantics::dom::LISTEN_*` name (prevent 1, stop 2,
    /// once 4, capture 8, passive 16), replacing earlier flags. A browser can only choose the
    /// phase and passivity at `addEventListener` time, so the bits cross as data and the host
    /// applies them; which modifiers a handler carries is decided in compiled LOGOS.
    DomListenFlags = "dom_listen_flags", (I64,I32,I64) -> ();
    /// `dom_event_value(event: i64) -> i32 Text` — the dispatching input's value.
    DomEventValue = "dom_event_value", (I64) -> (HOST_TEXT);
    /// `dom_event_key(event: i64) -> i32 Text` — the key of a keyboard event, `""` otherwise.
    DomEventKey = "dom_event_key", (I64) -> (HOST_TEXT);
    /// `dom_event_target(event: i64) -> i64` — the dispatching element.
    DomEventTarget = "dom_event_target", (I64) -> (I64);
    /// `dom_fetch_start(url: i32 Text, handler: i32 closure)` — fetch `url`; on
    /// resolve the host fires `handler` via `__logos_call_closure`.
    DomFetchStart = "dom_fetch_start", (I32,CALLBACK) -> ();
    /// `dom_response_text() -> i32 Text` — the body of the delivering fetch.
    DomResponseText = "dom_response_text", () -> (HOST_TEXT);
    // --- The correlated fetch contract (append-only tail — import indices are
    // positional). `dom_fetch_send` mints the request id every accessor is keyed
    // by; the host delivers per-request, so concurrent fetches never collide. ---
    /// `dom_fetch_send(method: i32 Text, url: i32 Text, body: i32 Text,
    /// headers: i32 Seq-of-Text (alternating name,value), handler: i32 closure)
    /// -> i64` — start a request; the host later fires `handler(reqId)` via
    /// `__logos_call_closure`.
    DomFetchSend = "dom_fetch_send", (I32,I32,I32,I32,CALLBACK) -> (I64);
    /// `dom_response_status(req: i64) -> i64` — HTTP status (0 pending/failed).
    DomResponseStatus = "dom_response_status", (I64) -> (I64);
    /// `dom_response_body(req: i64) -> i32 Text` — the delivered body.
    DomResponseBody = "dom_response_body", (I64) -> (HOST_TEXT);
    /// `dom_response_header(req: i64, name: i32 Text) -> i32 Text` — a response
    /// header, case-insensitive ("" absent).
    DomResponseHeader = "dom_response_header", (I64,I32) -> (HOST_TEXT);
    /// `dom_response_error(req: i64) -> i32 Text` — "" | "timeout" | "abort" |
    /// "toolarge" | "network: <detail>".
    DomResponseError = "dom_response_error", (I64) -> (HOST_TEXT);
    /// `dom_fetch_abort(req: i64)` — final "abort"; late responses never deliver.
    DomFetchAbort = "dom_fetch_abort", (I64) -> ();
    /// `dom_fetch_set_timeout(req: i64, millis: i64)` — arm (>0) / clear (0).
    DomFetchSetTimeout = "dom_fetch_set_timeout", (I64,I64) -> ();
    // --- The history/location router (append-only tail — import indices are
    // positional). `dom_route_path`/`dom_route_query` read `window.location`;
    // `dom_navigate`/`dom_replace_route` are history.pushState/replaceState; the host
    // fires the popstate handler registered by `dom_on_route` via the closure
    // trampoline (the browser popstate listener; a MockDom `deliver_pop` in tests). ---
    /// `dom_route_path() -> i32 Text` — the current location pathname.
    DomRoutePath = "dom_route_path", () -> (HOST_TEXT);
    /// `dom_route_query(name: i32 Text) -> i32 Text` — a query param ("" absent).
    DomRouteQuery = "dom_route_query", (I32) -> (HOST_TEXT);
    /// `dom_navigate(path: i32 Text)` — history.pushState + fire the router.
    DomNavigate = "dom_navigate", (I32) -> ();
    /// `dom_assign_location(url: i32 Text)` — `location.assign(url)`: load another document. What
    /// `Navigate to` a path outside this module's own routes lowers to; the page that asked is gone.
    DomAssignLocation = "dom_assign_location", (I32) -> ();
    /// `dom_replace_route(path: i32 Text)` — history.replaceState.
    DomReplaceRoute = "dom_replace_route", (I32) -> ();
    /// `dom_on_route(handler: i32 closure)` — register the popstate handler; the host
    /// later fires `handler(pathText)` via `__logos_call_closure`.
    DomOnRoute = "dom_on_route", (CALLBACK) -> ();
    // --- Page-level commands (append-only tail — import indices are positional). A page sets its
    // own title; the browser's `document.title` is the only place that can hold it, and there is
    // no decision here for the host to take. ---
    /// `dom_set_title(title: i32 Text)` — `document.title = title`.
    DomSetTitle = "dom_set_title", (I32) -> ();
    /// `dom_set_head_html(fragment: i32 Text)` — a `Head:` block's rendered `<title>`/`<meta>`/
    /// `<link>`/`<base>` markup, merged into `document.head` over whatever is already there (a tag
    /// replaces the existing tag of the same identity — see `runtime.js`'s `head_identity`, ported
    /// from `web_host.rs`'s SSR merge of the same name; everything else is added).
    DomSetHeadHtml = "dom_set_head_html", (I32) -> ();
    /// `dom_reload()` — `location.reload()`. The page starts over; nothing survives it.
    DomReload = "dom_reload", () -> ();
    /// `dom_go_back()` — `history.back()`. The browser fires popstate, which the router already
    /// listens for, so nothing here has to re-deliver it.
    DomGoBack = "dom_go_back", () -> ();
    /// `dom_get_attribute(el: i64, name: i32 Text) -> i32 Text` — the READ half of the attribute
    /// pair (`""` when absent). One import instead of `dom_add_class`/`dom_remove_class`, so
    /// which class words survive stays in compiled LOGOS where every tier runs the same rule.
    DomGetAttribute = "dom_get_attribute", (I64,I32) -> (HOST_TEXT);
    /// `dom_on_window_event(event: i32 Text, handler: i32 closure)` — `window.addEventListener`.
    /// One import for every event that happens to the PAGE rather than to an element: `offline`,
    /// `online`, `resize`. They fire on `window` and do not bubble, so a listener on `body` would
    /// be registered and never called.
    DomOnWindowEvent = "dom_on_window_event", (I32,CALLBACK) -> ();
    /// `dom_scroll_gap(el: i64) -> i64` — pixels of `el`'s content below the visible area; node 0
    /// means the page. A reading of three numbers only a browser holds; whether the gap counts as
    /// "the bottom" is decided in compiled LOGOS.
    DomScrollGap = "dom_scroll_gap", (I64) -> (I64);
    /// `dom_set_interval(ms: i64, handler: i32 closure)` — ONE repeating callback for the page.
    /// Not one per timer: which of the page's timers a tick makes due is compiled LOGOS.
    DomSetInterval = "dom_set_interval", (I64,CALLBACK) -> ();
    /// `dom_event_file_name(ev: i64) -> i32 Text` — the name of the first file a `drop` carried.
    /// A READ: what the page does about the file is compiled LOGOS.
    DomEventFileName = "dom_event_file_name", (I64) -> (HOST_TEXT);
    /// `dom_event_file_size(ev: i64) -> i64` — the byte size of the first file a `drop` carried,
    /// the `size` of the `File` record the page binds it to. A READ, like its name beside it.
    DomEventFileSize = "dom_event_file_size", (I64) -> (I64);
    /// `dom_event_file_type(ev: i64) -> i32 Text` — the first file's MIME type (`""` when the
    /// browser names none), the `type` of the same `File` record.
    DomEventFileType = "dom_event_file_type", (I64) -> (HOST_TEXT);
    /// `dom_event_file_bytes(ev: i64) -> i32 Seq of Int` — the first file's content, one byte per
    /// slot, the `bytes` of the same `File` record. The host reads the file before it raises the
    /// drop, so the turn runs with the bytes already in hand.
    DomEventFileBytes = "dom_event_file_bytes", (I64) -> (HOST_SEQ);
    /// `dom_element_command(el: i64, command: i32 Text)` — one named operation on one element:
    /// `focus`, `blur`, `hover`, `scrollto`, `selecttext`. One import for the family, because they
    /// differ only in which single DOM call is made and none of them decides anything.
    DomElementCommand = "dom_element_command", (I64,I32) -> ();
    /// `dom_clipboard_write(text: i32 Text)` — `navigator.clipboard.writeText`.
    DomClipboardWrite = "dom_clipboard_write", (I32) -> ();
    /// `dom_download_file(content: i32 Text, name: i32 Text)` — offer a file to the viewer.
    DomDownloadFile = "dom_download_file", (I32,I32) -> ();
    /// `dom_notify(message: i32 Text, tone: i32 Text)` — raise a notification.
    DomNotify = "dom_notify", (I32,I32) -> ();
    /// `dom_publish(topic: i32 Text, message: i32 Text)` — send a frame on a relay topic.
    ///
    /// The outbound twin of `dom_on_message`, and the half that construct shipped without: a page
    /// could subscribe and had no way to SEND, so it could hear a server and never answer it, and
    /// two tabs of one app could not talk. The relay had the other side complete the whole time
    /// (`RelayFrame::Publish` fans out to every local subscriber and gossips on to native nodes).
    ///
    /// It carries ONLY the leg that leaves the page. A publish also reaches this page's own
    /// subscribers, and that delivery is compiled LOGOS — dispatched through the same table
    /// `Raise` drives — so who hears a publish, and when, is decided in the module. The relay
    /// excludes the originating connection, so the two legs never deliver the same frame twice.
    DomPublish = "dom_publish", (I32,I32) -> ();
    /// `dom_note_turn(name: i32 Text, key: i32 Text, changes: i32 Seq of Text, failure: i32 Text)` —
    /// the current turn's journal entry: the action's name, the instance key it ran on, the state it
    /// changed as flattened `[field, before, after]` display strings, and the failure it stopped
    /// with (`""` for none). A browser keeps nothing; a recording host reads all four.
    DomNoteTurn = "dom_note_turn", (I32,I32,I32,I32) -> ();
    /// `dom_journal_detail() -> i64 Bool` — whether this host records the journal's detail (each
    /// turn's before/after state), so a lowered turn captures it only where someone reads it: a
    /// recording host answers true, a shipped browser page false.
    DomJournalDetail = "dom_journal_detail", () -> (I64);
    /// `dom_wake_after(ms: i64)` — call the page's `__lgx_tick` with the time that passed once `ms`
    /// more milliseconds have gone by, replacing any earlier request; a negative `ms` cancels it.
    DomWakeAfter = "dom_wake_after", (I64) -> ();
    /// `dom_enter_value(verb: i32 Text, selector: i32 Text, value: i32 Text) -> i64 Bool` — a
    /// program's own `Type`/`Choose` (`"type"`/`"choose"`): the browser's default action of entering
    /// `value` into the control `selector` names, answering whether the control took it (a text
    /// field a visitor can reach, a `<select>` offering the option).
    DomEnterValue = "dom_enter_value", (I32, I32, I32) -> (I64);
    /// `dom_page_entry(node: i64, event: i32 Text, value: i32 Text) -> i64 Bool` — a driving
    /// verb the program runs on its own page: the host enters the page's `__lgx_dispatch_node`
    /// as an entry of its own, so a turn that fails there is contained to it and heard through the
    /// page's failure door, and the program runs on. Answers whether the page heard the event; a
    /// failed turn was heard.
    DomPageEntry = "dom_page_entry", (I64, I32, I32) -> (I64);
    // --- Exact integers (append-only tail — import indices are positional). An `Int` the program can
    // make outgrow a machine word rides a TAGGED WORD: an even `i64` `w` is the integer `w >> 1`; an odd
    // one, `(h << 1) | 1`, names integer `h` the host holds. The host always answers with the small form
    // for an integer from -2^62 up to 2^62 and holds every other, and a held integer is OWNED: every
    // answer is a handle of its own, given back through `exact_drop`. The arithmetic is the reference
    // tier's (`semantics::arith`), each import called only where the module's inline path cannot answer. ---
    /// `exact_box(v: i64) -> i64` — the tagged word of `v`, an `i64` too wide to sit beside the tag.
    ExactBox = "exact_box", (I64) -> (I64);
    /// `exact_narrow(t: i64) -> i64` — the `i64` a held integer is, or `0` when it is past one (a held
    /// integer is never zero, so the answer is unambiguous).
    ExactNarrow = "exact_narrow", (I64) -> (I64);
    /// `exact_add`/`exact_sub`/`exact_mul(a: i64, b: i64) -> i64` — the tagged sum, difference, product.
    ExactAdd = "exact_add", (I64, I64) -> (I64);
    ExactSub = "exact_sub", (I64, I64) -> (I64);
    ExactMul = "exact_mul", (I64, I64) -> (I64);
    /// `exact_div`/`exact_mod`/`exact_floordiv(a: i64, b: i64) -> i64` — the truncated quotient, the
    /// remainder with the dividend's sign, the floored quotient. The module has already refused a zero divisor.
    ExactDiv = "exact_div", (I64, I64) -> (I64);
    ExactMod = "exact_mod", (I64, I64) -> (I64);
    ExactFloorDiv = "exact_floordiv", (I64, I64) -> (I64);
    /// `exact_pow(base: i64, exp: i64) -> i64` — `base` (tagged) to the power `exp`, a machine word the
    /// module has already bounded to `0..=u32::MAX`.
    ExactPow = "exact_pow", (I64, I64) -> (I64);
    /// `exact_cmp(a: i64, b: i64) -> i32` — `-1`, `0` or `1` as `a` is below, equal to or above `b`.
    ExactCmp = "exact_cmp", (I64, I64) -> (I32);
    /// `exact_f64(t: i64) -> f64` — the binary64 nearest the integer, ties to even, as an `Int` widens.
    ExactF64 = "exact_f64", (I64) -> (F64);
    /// `exact_clone(t: i64) -> i64` — a held integer's handle of its own.
    ExactClone = "exact_clone", (I64) -> (I64);
    /// `exact_drop(t: i64)` — give a held integer's handle back.
    ExactDrop = "exact_drop", (I64) -> ();
    /// `print_exact(t: i64)` — display the integer, as `Show` does.
    PrintExact = "print_exact", (I64) -> ();
    /// `exact_text(t: i64) -> i32 Text` — the integer's decimal as a Text, for an interpolation.
    ExactText = "exact_text", (I64) -> (HOST_TEXT);
    /// `env_len(name: i32 Text) -> i32` — the byte length of the variable's text in the program's
    /// environment, `-1` when it is unset: env.lg's `get(name)`, which the VM expands into whether the
    /// variable is set, its text and an ordinary `Option`. A browser has no environment, so there every
    /// name is unset.
    EnvLen = "env_len", (I32) -> (I32);
    /// `env_into(buf: i32, name: i32 Text) -> i32` — the variable's text written into `buf`, a buffer
    /// the module sized by [`HostFn::EnvLen`]; answers the bytes written (`0` when it is unset). The
    /// module allocates and the host only writes, so a linked module needs no allocator export.
    EnvInto = "env_into", (I32, I32) -> (I32);
    /// `time_now() -> i64` — time.lg's `now()`: the Unix time in milliseconds from the real clock, not
    /// the test-fixed moment [`HostFn::Now`] answers.
    TimeNow = "time_now", () -> (I64);
    /// The exact extension's value runtime (`logicaffeine_data::abi`, the C ABI the C backends link), linked into
    /// the module: an i32 handle per value ([`kind::Kind::Tower`]). A constructor answers a fresh handle; an
    /// operation borrows its operands and answers a fresh handle, null when it has no answer, which
    /// [`HostFn::ValueChecked`] turns into the runtime's sentence and a trap.
    ValueInt = "logos_rt_value_int", (I64) -> (I32);
    ValueFloat = "logos_rt_value_float", (F64) -> (I32);
    ValueBool = "logos_rt_value_bool", (I32) -> (I32);
    ValueNothing = "logos_rt_value_nothing", () -> (I32);
    ValueDate = "logos_rt_value_date", (I32) -> (I32);
    ValueMoment = "logos_rt_value_moment", (I64) -> (I32);
    ValueTime = "logos_rt_value_time", (I64) -> (I32);
    ValueSpan = "logos_rt_value_span", (I32, I32) -> (I32);
    ValueList = "logos_rt_value_list", (I32, I32) -> (I32);
    ValueClone = "logos_rt_value_clone", (I32) -> (I32);
    /// `logos_rt_value_push(list, v) -> status` — and `set_item`'s: `1`, or `0` with the sentence kept.
    ValuePush = "logos_rt_value_push", (I32, I32) -> (I32);
    ValueSetItem = "logos_rt_value_set_item", (I32, I32, I32) -> (I32);
    ValuePop = "logos_rt_value_pop", (I32) -> (I32);
    ValueItems = "logos_rt_value_items", (I32) -> (I32);
    ValueItemAt = "logos_rt_value_item_at", (I32, I64) -> (I32);
    ValueSlice = "logos_rt_value_slice", (I32, I32, I32) -> (I32);
    ValueAdd = "logos_rt_value_add", (I32, I32) -> (I32);
    ValueSub = "logos_rt_value_sub", (I32, I32) -> (I32);
    ValueMul = "logos_rt_value_mul", (I32, I32) -> (I32);
    ValueDiv = "logos_rt_value_div", (I32, I32) -> (I32);
    ValueExactDiv = "logos_rt_value_exact_div", (I32, I32) -> (I32);
    ValueFloorDiv = "logos_rt_value_floor_div", (I32, I32) -> (I32);
    ValueRem = "logos_rt_value_rem", (I32, I32) -> (I32);
    ValuePow = "logos_rt_value_pow", (I32, I32) -> (I32);
    ValueApproxEq = "logos_rt_value_approx_eq", (I32, I32) -> (I32);
    ValueSeqConcat = "logos_rt_value_seq_concat", (I32, I32) -> (I32);
    ValueBitAnd = "logos_rt_value_bit_and", (I32, I32) -> (I32);
    ValueBitOr = "logos_rt_value_bit_or", (I32, I32) -> (I32);
    ValueBitXor = "logos_rt_value_bit_xor", (I32, I32) -> (I32);
    ValueShl = "logos_rt_value_shl", (I32, I32) -> (I32);
    ValueShr = "logos_rt_value_shr", (I32, I32) -> (I32);
    ValueMin = "logos_rt_value_min", (I32, I32) -> (I32);
    ValueMax = "logos_rt_value_max", (I32, I32) -> (I32);
    ValuePower = "logos_rt_value_power", (I32, I32) -> (I32);
    ValueItem = "logos_rt_value_item", (I32, I32) -> (I32);
    ValueEq = "logos_rt_value_eq", (I32, I32) -> (I32);
    ValueNe = "logos_rt_value_ne", (I32, I32) -> (I32);
    ValueLt = "logos_rt_value_lt", (I32, I32) -> (I32);
    ValueGt = "logos_rt_value_gt", (I32, I32) -> (I32);
    ValueLe = "logos_rt_value_le", (I32, I32) -> (I32);
    ValueGe = "logos_rt_value_ge", (I32, I32) -> (I32);
    ValueNot = "logos_rt_value_not", (I32) -> (I32);
    ValueAbs = "logos_rt_value_abs", (I32) -> (I32);
    ValueSqrt = "logos_rt_value_sqrt", (I32) -> (I32);
    ValueFloor = "logos_rt_value_floor", (I32) -> (I32);
    ValueCeil = "logos_rt_value_ceil", (I32) -> (I32);
    ValueRound = "logos_rt_value_round", (I32) -> (I32);
    /// The value of a number this runtime holds in Rust (a big integer, a rational, a decimal); a tuple of a list's
    /// items; and `apply(index, args)`, the builtin of the extension at `index` of `extension::Builtin::ALL`
    /// applied to the items of the list `args`.
    ValueOfBigint = "logos_rt_value_of_bigint", (I32) -> (I32);
    ValueOfRational = "logos_rt_value_of_rational", (I32) -> (I32);
    ValueOfDecimal = "logos_rt_value_of_decimal", (I32) -> (I32);
    ValueTupleOf = "logos_rt_value_tuple_of", (I32) -> (I32);
    ValueApply = "logos_rt_value_apply", (I32, I32) -> (I32);
    /// `define(kind, base, name, body) -> handle` — the real number the closure body `body` defines by its digits
    /// in `base` or its approximations (`kind`, `name` Texts): the runtime reads it back through the module's
    /// `logos_program_defines(body, n)`.
    ValueDefine = "logos_rt_value_define", (I32, I64, I32, I32) -> (I32);
    /// The linked runtime's adapters of that ABI to the module's (`link.rs`): `checked(h)` answers `h`, or stops
    /// the program with the runtime's sentence when `h` is null (a status of `0` likewise).
    ValueChecked = "logos_rt_value_checked", (I32) -> (I32);
    /// `to_text(v) -> Text` — how the value prints, as a Text the runtime lays out.
    ValueToText = "logos_rt_value_to_text", (I32) -> (I32);
    ValueOfText = "logos_rt_value_of_text", (I32) -> (I32);
    ValueLengthI64 = "logos_rt_value_length_i64", (I32) -> (I64);
    /// `truth_i32(v)` — the truth a condition reads from the value, refused for an undecided answer.
    ValueTruthI32 = "logos_rt_value_truth_i32", (I32) -> (I32);
    ValueContainsI64 = "logos_rt_value_contains_i64", (I32, I32) -> (I64);
    /// `to_i64(v)` — the `Int` the value is, where a machine-integer key is read; refused for any other value.
    ValueToI64 = "logos_rt_value_to_i64", (I32) -> (I64);
    /// `integer_at(v, place)` — the `Int` the value is where a machine integer is read at `place` (an
    /// `IntegerPlace::code`); an integer past i64 or a fraction refused in the VM's words there.
    ValueIntegerAt = "logos_rt_value_integer_at", (I32, I32) -> (I64);
    /// `logos_rt_rational_cmp(a, b) -> i32` — `-1`, `0` or `1` as the rational `a` is below, equal to or above `b`.
    RationalCmp = "logos_rt_rational_cmp", (I32, I32) -> (I32);
    /// `logos_rt_rational_quotient(a, b)` — `a / b` under the VM's rules: two whole numbers as integers, truncating.
    RationalQuotient = "logos_rt_rational_quotient", (I32, I32) -> (I32);
    /// `logos_rt_rational_floor_quotient(a, b)` — `a // b`, the floor of the exact quotient.
    RationalFloorQuotient = "logos_rt_rational_floor_quotient", (I32, I32) -> (I32);
    /// `logos_rt_rational_remainder(a, b)` — `a % b`: two whole numbers' integer remainder; a fraction refused.
    RationalRemainder = "logos_rt_rational_remainder", (I32, I32) -> (I32);
    /// `logos_rt_rational_power(a, b)` — `a ** b`: a whole base takes the integer's power.
    RationalPower = "logos_rt_rational_power", (I32, I32) -> (I32);
    /// `logos_rt_rational_bitwise(a, b, op) -> i64` — `& | ^ << >>` (op 0..=4) of two whole numbers.
    RationalBitwise = "logos_rt_rational_bitwise", (I32, I32, I32) -> (I64);
    /// `logos_rt_rational_cmp_f64(a, f) -> i32` — the rational's order against the real's exact value, 2 for a NaN.
    RationalCmpF64 = "logos_rt_rational_cmp_f64", (I32, F64) -> (I32);
    /// `logos_rt_rational_to_f64(a) -> f64` — the double the VM computes with when a rational meets a real.
    RationalToF64 = "logos_rt_rational_to_f64", (I32) -> (F64);
    /// `as_i64(a, place)` — the machine integer a whole rational within i64 is where one is read at `place` (an
    /// `IntegerPlace::code`); else the VM's refusal there.
    RationalAsI64 = "logos_rt_rational_as_i64", (I32, I32) -> (I64);
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
        Op::SetIndex {
            collection, value, ..
        } if aliases.contains(collection) => kinds.get(*value as usize),
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

/// The host import a DOM, storage or journal builtin lowers to — one import per builtin, whatever its
/// arguments' kinds — or `None`. `setAttribute` picks its import by the value's kind and is not here.
/// The import scan notes what this names, and [`pure_read_operands`] reads which of the builtin's
/// arguments the host keeps off the same row ([`HostFn::retained_params`]).
fn dom_import_of(b: BuiltinId) -> Option<HostFn> {
    Some(match b {
        BuiltinId::DomBody => HostFn::DomBody,
        BuiltinId::DomCreateElement => HostFn::DomCreateElement,
        BuiltinId::DomCreateTextNode => HostFn::DomCreateTextNode,
        BuiltinId::DomSetText => HostFn::DomSetText,
        BuiltinId::DomSetHtml => HostFn::DomSetHtml,
        BuiltinId::DomClearChildren => HostFn::DomClearChildren,
        BuiltinId::DomOnMessage => HostFn::DomOnMessage,
        BuiltinId::DomAppendChild => HostFn::DomAppendChild,
        BuiltinId::DomGetText => HostFn::DomGetText,
        BuiltinId::DomGetValue => HostFn::DomGetValue,
        BuiltinId::DomSetValue => HostFn::DomSetValue,
        BuiltinId::DomSetChecked => HostFn::DomSetChecked,
        BuiltinId::DomPreventDefault => HostFn::DomPreventDefault,
        BuiltinId::StorageSet => HostFn::StorageSet,
        BuiltinId::StorageGet => HostFn::StorageGet,
        BuiltinId::StorageGetOr => HostFn::StorageGet,
        BuiltinId::StorageRemove => HostFn::StorageRemove,
        BuiltinId::DomApplyPatches => HostFn::DomApply,
        BuiltinId::DomApplyWords => HostFn::DomApplyWords,
        BuiltinId::DomQuerySelector => HostFn::DomQuerySelector,
        BuiltinId::DomAddEventListener => HostFn::DomAddEventListener,
        BuiltinId::DomListenFlags => HostFn::DomListenFlags,
        BuiltinId::DomEnterValue => HostFn::DomEnterValue,
        BuiltinId::DomDispatchEntry => HostFn::DomPageEntry,
        BuiltinId::DomEventValue => HostFn::DomEventValue,
        BuiltinId::DomEventKey => HostFn::DomEventKey,
        BuiltinId::DomEventTarget => HostFn::DomEventTarget,
        BuiltinId::DomFetchText => HostFn::DomFetchStart,
        BuiltinId::DomResponseText => HostFn::DomResponseText,
        BuiltinId::DomFetchSend => HostFn::DomFetchSend,
        BuiltinId::DomResponseStatus => HostFn::DomResponseStatus,
        BuiltinId::DomResponseBody => HostFn::DomResponseBody,
        BuiltinId::DomResponseHeader => HostFn::DomResponseHeader,
        BuiltinId::DomResponseError => HostFn::DomResponseError,
        BuiltinId::DomFetchAbort => HostFn::DomFetchAbort,
        BuiltinId::DomFetchSetTimeout => HostFn::DomFetchSetTimeout,
        BuiltinId::DomRoutePath => HostFn::DomRoutePath,
        BuiltinId::DomRouteQuery => HostFn::DomRouteQuery,
        BuiltinId::DomNavigate => HostFn::DomNavigate,
        BuiltinId::DomAssignLocation => HostFn::DomAssignLocation,
        BuiltinId::DomSetTitle => HostFn::DomSetTitle,
        BuiltinId::DomSetHeadHtml => HostFn::DomSetHeadHtml,
        BuiltinId::DomReload => HostFn::DomReload,
        BuiltinId::DomGoBack => HostFn::DomGoBack,
        BuiltinId::DomGetAttribute => HostFn::DomGetAttribute,
        BuiltinId::DomOnWindowEvent => HostFn::DomOnWindowEvent,
        BuiltinId::DomScrollGap => HostFn::DomScrollGap,
        BuiltinId::DomSetInterval => HostFn::DomSetInterval,
        BuiltinId::DomEventFileName => HostFn::DomEventFileName,
        BuiltinId::DomEventFileSize => HostFn::DomEventFileSize,
        BuiltinId::DomEventFileType => HostFn::DomEventFileType,
        BuiltinId::DomEventFileBytes => HostFn::DomEventFileBytes,
        BuiltinId::DomElementCommand => HostFn::DomElementCommand,
        BuiltinId::DomClipboardWrite => HostFn::DomClipboardWrite,
        BuiltinId::DomDownloadFile => HostFn::DomDownloadFile,
        BuiltinId::DomNotify => HostFn::DomNotify,
        BuiltinId::DomPublish => HostFn::DomPublish,
        BuiltinId::NoteTurn => HostFn::DomNoteTurn,
        BuiltinId::DomJournalDetail => HostFn::DomJournalDetail,
        BuiltinId::DomWakeAfter => HostFn::DomWakeAfter,
        BuiltinId::DomReplaceRoute => HostFn::DomReplaceRoute,
        BuiltinId::DomOnRoute => HostFn::DomOnRoute,
        _ => return None,
    })
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

/// Every host import the emitted modules can name, in catalog order — the whole LOGOS↔host
/// boundary as text.
///
/// This surface is the framework's contract with the browser, and it is the thing that must NOT
/// quietly grow: LOGOS runs the page as WebAssembly, so an import is the platform forcing our
/// hand (wasm cannot touch the DOM), while framework LOGIC belongs in compiled LOGOS where every
/// tier can run it. Exposing the list is what lets a lock hold that line.
pub fn host_import_names() -> Vec<&'static str> {
    HOST_FNS.iter().map(|h| h.field()).collect()
}

/// Every name an emitted module exports that is the module's own rather than a lowered
/// `lgx_lower::LGX_ENTRIES` entry: `main`, the memory and heap pointer, the closure trampoline, the
/// immediate-mode `render`, and each runtime function a host calls.
pub fn runtime_export_names() -> Vec<&'static str> {
    // Exhaustive, so a new runtime function is placed here or answered `None` by its `export`.
    fn listed(f: RuntimeFn) -> RuntimeFn {
        match f {
            RuntimeFn::Alloc
            | RuntimeFn::Free
            | RuntimeFn::HeapAllocated
            | RuntimeFn::HeapUsed
            | RuntimeFn::IterUnwind
            | RuntimeFn::TextNew
            | RuntimeFn::SeqNew
            | RuntimeFn::SeqSlot
            | RuntimeFn::SeqPushSlot
            | RuntimeFn::SeqClone
            | RuntimeFn::SeqOwn
            | RuntimeFn::TextEq
            | RuntimeFn::TextConcat
            | RuntimeFn::MapClone
            | RuntimeFn::MapOwn
            | RuntimeFn::MapPut
            | RuntimeFn::MapFind
            | RuntimeFn::MapAt
            | RuntimeFn::AddChecked
            | RuntimeFn::SubChecked
            | RuntimeFn::MulChecked
            | RuntimeFn::ExactAdd
            | RuntimeFn::ExactSub
            | RuntimeFn::ExactMul
            | RuntimeFn::ExactAddW
            | RuntimeFn::ExactSubW
            | RuntimeFn::ExactMulW
            | RuntimeFn::ExactTag
            | RuntimeFn::ExactRelease
            | RuntimeFn::ExactNarrow
            | RuntimeFn::ExactClone
            | RuntimeFn::ExactSeqPromote
            | RuntimeFn::ExactCell
            | RuntimeFn::ExactCompare
            | RuntimeFn::ExactSeqAt
            | RuntimeFn::ExactSeqStore
            | RuntimeFn::ExactMapPromoteKey
            | RuntimeFn::ExactMapPromoteValue
            | RuntimeFn::NewHeader
            | RuntimeFn::Share
            | RuntimeFn::TurnRetain
            | RuntimeFn::Retain
            | RuntimeFn::RetainChildren
            | RuntimeFn::Release
            | RuntimeFn::Safepoint
            | RuntimeFn::FreeObject
            | RuntimeFn::Census
            | RuntimeFn::CensusVisit
            | RuntimeFn::ClosureRetain
            | RuntimeFn::ClosureRelease => f,
        }
    }
    let runtime = [
        RuntimeFn::Alloc,
        RuntimeFn::Free,
        RuntimeFn::HeapAllocated,
        RuntimeFn::HeapUsed,
        RuntimeFn::IterUnwind,
        RuntimeFn::TextNew,
        RuntimeFn::SeqNew,
        RuntimeFn::SeqSlot,
        RuntimeFn::SeqPushSlot,
        RuntimeFn::SeqClone,
        RuntimeFn::SeqOwn,
        RuntimeFn::TextEq,
        RuntimeFn::TextConcat,
        RuntimeFn::MapClone,
        RuntimeFn::MapOwn,
        RuntimeFn::MapPut,
        RuntimeFn::MapFind,
        RuntimeFn::MapAt,
        RuntimeFn::AddChecked,
        RuntimeFn::SubChecked,
        RuntimeFn::MulChecked,
        RuntimeFn::ExactAdd,
        RuntimeFn::ExactSub,
        RuntimeFn::ExactMul,
        RuntimeFn::ExactAddW,
        RuntimeFn::ExactSubW,
        RuntimeFn::ExactMulW,
        RuntimeFn::ExactTag,
        RuntimeFn::ExactRelease,
        RuntimeFn::ExactNarrow,
        RuntimeFn::ExactClone,
        RuntimeFn::ExactSeqPromote,
        RuntimeFn::ExactCell,
        RuntimeFn::ExactCompare,
        RuntimeFn::ExactSeqAt,
        RuntimeFn::ExactSeqStore,
        RuntimeFn::ExactMapPromoteKey,
        RuntimeFn::ExactMapPromoteValue,
        RuntimeFn::NewHeader,
        RuntimeFn::Share,
        RuntimeFn::TurnRetain,
        RuntimeFn::Retain,
        RuntimeFn::RetainChildren,
        RuntimeFn::Release,
        RuntimeFn::Safepoint,
        RuntimeFn::FreeObject,
        RuntimeFn::Census,
        RuntimeFn::CensusVisit,
        RuntimeFn::ClosureRetain,
        RuntimeFn::ClosureRelease,
    ];
    [
        "main",
        "memory",
        "__heap_ptr",
        "__logos_call_closure",
        "__logos_render",
    ]
    .into_iter()
    .chain(runtime.into_iter().filter_map(|f| listed(f).export()))
    .collect()
}

/// Every host import that answers with a Text the host writes into the module's heap through
/// `__logos_text_new`, in catalog order — the imports that make a module carry that allocator.
pub fn host_text_import_names() -> Vec<&'static str> {
    HOST_FNS
        .iter()
        .filter(|h| h.returns_host_text())
        .map(|h| h.field())
        .collect()
}

/// Every host import that answers with a sequence the host builds in the module's heap through
/// `__logos_seq_new`, in catalog order — the imports that make a module carry that allocator.
pub fn host_seq_import_names() -> Vec<&'static str> {
    HOST_FNS
        .iter()
        .filter(|h| h.returns_host_seq())
        .map(|h| h.field())
        .collect()
}

/// Every host import as `(name, params, results)`, in canonical order — the SHAPE of the ABI,
/// not just its spelling.
///
/// `host_import_names` froze the names, and a name is the half that a rename breaks loudly.
/// The other half — how many arguments an import takes and what it gives back — was written
/// down three times in two crates and read by no test at all, so changing one host's arity
/// left the emitted module and its hand-written JS host disagreeing, and the module simply
/// failed to instantiate with nothing to point at.
pub fn host_import_signatures() -> Vec<(&'static str, Vec<u8>, Vec<u8>)> {
    HOST_FNS
        .iter()
        .map(|h| (h.field(), h.params(), h.results()))
        .collect()
}

impl HostFn {
    /// The sink that displays a value of kind `k` — matching the tree-walker's
    /// `to_display_string` for that scalar kind. `None` for a kind without a scalar sink yet
    /// (e.g. a sequence, whose formatted display lands with the heap value model).
    fn for_show(k: Kind) -> Option<HostFn> {
        Some(match k {
            Kind::Nothing => HostFn::PrintNothing,
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
            // A sequence of boxed elements has no single scalar sink, for the same reason a whole
            // `Seq of <record>` does not: displaying it means walking the elements and displaying
            // each, which is the heap value model rather than one host call.
            Kind::SeqHandle => return None,
            Kind::Text => HostFn::PrintText,
            // A whole struct's `TypeName { field: val, … }` display is assembled by `lower_show_struct`
            // (fields in DETERMINISTIC alphabetical order, matching the VM's now-sorted `HashMap`
            // display) — handled by the `Op::Show` dispatch, not this per-kind scalar-sink table.
            Kind::Struct => return None,
            // A whole `Set of Int` is insertion-ordered in both the VM (a `Vec`) and the AOT backend,
            // so it displays deterministically: `{e0, e1, …}`.
            Kind::Set => HostFn::PrintSetI64,
            Kind::SetText | Kind::CrdtSetText => HostFn::PrintSetText,
            // A `ConvergentCount`/`Tally` counter object is never `Show`n as an object: `Show obj's
            // field` projects it to its Int TOTAL at `GetField` (`lower_get_counter_total`), so this
            // per-kind sink never sees the raw counter — it has no direct display form.
            Kind::CrdtCounter => return None,
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
            // A value of the exact extension prints through the runtime's text ([`tower`]).
            Kind::Tower => return None,
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
        if let Some(i) = self
            .sigs
            .iter()
            .position(|(p, r)| *p == params && *r == results)
        {
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
    /// Per op position, for an `IterNext`, the register its loop walks (the `IterPrepare`↔`IterNext`
    /// pairing), so the step can tell a sequence's slots from a Text's characters.
    iter_iterable: Vec<Option<usize>>,
    /// Numeric ranges consumed only by the immediately following iterator. Nothing can observe
    /// a sequence representation, so the loop stores the captured bounds instead.
    range_loops: std::collections::HashMap<u16, (u16, u16)>,
    /// The place lowering's record-field moves (see [`PlaceSites`]).
    place: PlaceSites,
    /// Per register: every read of it is transient (see [`transient_reads`]), so the value it holds
    /// needs no alias retain.
    transient: Vec<bool>,
    /// Per register, the DECLARED type of the value it holds wherever one is known — a parameter's,
    /// a declared construction's or binding's, one the layout pass carried to it — which is what a
    /// display of the value renders ([`display`]).
    declared: std::collections::HashMap<u16, BoundaryType>,
    /// The `mutable` parameters (registers below `num_params`): each one's final value is a result of the
    /// function after its declared one, which the caller stores into the argument register it lent, as the VM's
    /// register window leaves it (`CompiledFunction::mutable_param_regs`).
    inout: Vec<u16>,
    /// Per register: it holds a tagged word, the exact integer [`exact`] decided it can outgrow i64 for.
    /// Empty in a module no integer of which can.
    exact: Vec<bool>,
    /// Per parameter: its argument arrives tagged ([`exact::Signatures`]).
    params_tagged: Vec<bool>,
    /// The function's result is tagged.
    ret_tagged: bool,
    absence: bool,
    absence_regs: Vec<bool>,
    absence_globals: Vec<bool>,
    /// What the plan does with values of the exact extension ([`tower`]).
    tower: tower::Towers,
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
            iter_iterable: Vec::new(),
            range_loops: std::collections::HashMap::new(),
            place: PlaceSites::default(),
            transient: Vec::new(),
            declared: std::collections::HashMap::new(),
            inout: Vec::new(),
            exact: Vec::new(),
            params_tagged: Vec::new(),
            ret_tagged: false,
            absence: false,
            absence_regs: Vec::new(),
            absence_globals: Vec::new(),
            tower: tower::Towers::default(),
            stub: true,
        }
    }
}

/// Only erase a range when its one definition is followed immediately by its only use,
/// IterPrepare. A returned/stored/read range remains an ordinary sequence.
fn range_loop_registers(
    ops: &[Op],
    functions: &[CompiledFunction],
) -> std::collections::HashMap<u16, (u16, u16)> {
    let mut defs = std::collections::HashMap::<u16, usize>::new();
    let mut uses = std::collections::HashMap::<u16, usize>::new();
    for op in ops {
        let (d, u) = crate::vm::instruction::op_def_uses(op, functions);
        for r in d {
            *defs.entry(r).or_default() += 1;
        }
        for r in u {
            *uses.entry(r).or_default() += 1;
        }
    }
    // The constructor must be the iterator entry's sole predecessor. A branch may otherwise
    // skip NewRange and supply a pre-existing sequence in the same register.
    let mut incoming = vec![0_usize; ops.len()];
    for (pc, op) in ops.iter().enumerate() {
        for next in crate::vm::instruction::op_successors(op, pc, ops.len()) {
            if let Some(count) = incoming.get_mut(next) {
                *count += 1;
            }
        }
    }
    ops.windows(2)
        .enumerate()
        .filter_map(|(pc, pair)| match (&pair[0], &pair[1]) {
            (Op::NewRange { dst, start, end }, Op::IterPrepare { iterable })
                if dst == iterable
                    && defs.get(dst) == Some(&1)
                    && uses.get(dst) == Some(&1)
                    && incoming[pc + 1] == 1 =>
            {
                Some((*dst, (*start, *end)))
            }
            _ => None,
        })
        .collect()
}

/// How the VM's place lowering (`Compiler::compile_place_mutation`) moves one level of a place out
/// and back in, recognised per op position so wasm can lower it as the MOVE it is.
///
/// A mutation through `r's f` compiles to: `GetField d = r.f`, then `SetIndex r["f"] = nothing`, then
/// the mutation of `d`, then `SetIndex r["f"] = d`; one through `item i of c` to the same shape with
/// `Index d = c[i]` and `SetIndex c[i] = …`, and one of a compiler-written module-level binding with
/// `GlobalGet d = g` and `GlobalSet g = …`. The `nothing` write exists for the VM alone — it drops
/// the parent's own reference so the VM's ownership check on `d` counts only real aliases. The wasm
/// equivalent is not to count the extraction as an alias: the level is TAKEN (no retain, so mutating
/// `d` copies only when something else really holds it), the clear stores nothing (its
/// copy-on-write of the parent still runs, so the put-back lands in the parent's own storage), and
/// PUTTING it back is not a second holder either (no retain). A read not followed by its clear stays
/// an ordinary aliasing read.
#[derive(Default)]
struct PlaceSites {
    /// Per `SetIndex`: the Text constant its index register holds there — the field a record write
    /// names — when the block defines that register with a `LoadConst` before the write.
    field_at: Vec<Option<u32>>,
    /// Per `GetField`/`Index`: the level is taken out by the place lowering.
    take: Vec<bool>,
    /// Per `SetIndex`: the `nothing` that clears a level just taken out.
    clear: Vec<bool>,
    /// Per `SetIndex`: the value written is the level taken out earlier, put back.
    put_back: Vec<bool>,
}

/// Where a taken level sat in its parent: a record field (by name), an item (by index register), or
/// a module-level binding (by global index; its "parent" is the globals store).
#[derive(Clone, Copy, PartialEq)]
enum PlaceKey {
    Field(u32),
    Item(u16),
    Global(u16),
}

impl PlaceSites {
    fn of(ops: &[Op], constants: &[Constant], functions: &[CompiledFunction]) -> PlaceSites {
        let n = ops.len();
        let Some(blocks) = super::cfg::Blocks::new(ops) else {
            return PlaceSites::default();
        };
        // The constant `reg` holds just before `pc`: its latest definition in `pc`'s own block, when
        // that definition is a `LoadConst`. A definition in another block has other paths into
        // `pc` and answers nothing — the place lowering loads each name right beside its use.
        let const_at = |pc: usize, reg: u16| -> Option<u32> {
            let mut j = pc;
            while j > 0 && !blocks.is_leader(j) {
                j -= 1;
                if let Op::LoadConst { dst, idx } = ops[j] {
                    if dst == reg {
                        return Some(idx);
                    }
                    continue;
                }
                if crate::vm::instruction::op_def_uses(&ops[j], functions)
                    .0
                    .contains(&reg)
                {
                    return None;
                }
            }
            None
        };
        let text_const = |c: u32| matches!(constants.get(c as usize), Some(Constant::Text(_)));
        let is_nothing = |c: Option<u32>| {
            c.is_some_and(|c| matches!(constants.get(c as usize), Some(Constant::Nothing)))
        };
        let same_name = |a: u32, b: u32| matches!((constants.get(a as usize), constants.get(b as usize)), (Some(Constant::Text(x)), Some(Constant::Text(y))) if x == y);
        let mut field_at = vec![None; n];
        for (pc, op) in ops.iter().enumerate() {
            if let Op::SetIndex { index, .. } | Op::SetIndexUnchecked { index, .. } = *op {
                field_at[pc] = const_at(pc, index).filter(|&c| text_const(c));
            }
        }
        // Whether the op at `at` writes `(parent, key)` — a field by its name, an item by the very
        // index register the read used, a global by its index.
        let writes = |at: usize, parent: u16, key: PlaceKey| -> bool {
            match (ops[at], key) {
                (
                    Op::SetIndex { collection, .. } | Op::SetIndexUnchecked { collection, .. },
                    PlaceKey::Field(f),
                ) => collection == parent && field_at[at].is_some_and(|c| same_name(c, f)),
                (
                    Op::SetIndex {
                        collection, index, ..
                    }
                    | Op::SetIndexUnchecked {
                        collection, index, ..
                    },
                    PlaceKey::Item(r),
                ) => collection == parent && index == r,
                (Op::GlobalSet { idx, .. }, PlaceKey::Global(g)) => idx == g,
                _ => false,
            }
        };
        let set_value = |at: usize| match ops[at] {
            Op::SetIndex { value, .. } | Op::SetIndexUnchecked { value, .. } => Some(value),
            Op::GlobalSet { src, .. } => Some(src),
            _ => None,
        };
        let mut take = vec![false; n];
        let mut clear = vec![false; n];
        let mut put_back = vec![false; n];
        let mut taken: Vec<(u16, PlaceKey, u16)> = Vec::new();
        for (pc, op) in ops.iter().enumerate() {
            let read = match *op {
                Op::GetField { dst, obj, field } => Some((dst, obj, PlaceKey::Field(field))),
                Op::Index {
                    dst,
                    collection,
                    index,
                }
                | Op::IndexUnchecked {
                    dst,
                    collection,
                    index,
                } => Some((dst, collection, PlaceKey::Item(index))),
                Op::GlobalGet { dst, idx } => Some((dst, 0, PlaceKey::Global(idx))),
                _ => None,
            };
            if let Some((dst, parent, key)) = read {
                let next = ops[pc + 1..]
                    .iter()
                    .position(|o| !matches!(o, Op::LoadConst { .. }))
                    .map(|i| pc + 1 + i);
                if let Some(at) = next {
                    if writes(at, parent, key)
                        && is_nothing(set_value(at).and_then(|v| const_at(at, v)))
                    {
                        take[pc] = true;
                        clear[at] = true;
                        taken.push((parent, key, dst));
                        continue;
                    }
                }
                // A global read that is stored straight back — the same register, in straight-line
                // code, with nothing between that could read the global or run other code — is a take
                // with no clear: the mutation between is the global's own.
                if let PlaceKey::Global(g) = key {
                    if stored_back(ops, pc, g, dst) {
                        take[pc] = true;
                        taken.push((parent, key, dst));
                    }
                }
                continue;
            }
            if clear[pc] {
                continue;
            }
            if let Some(value) = set_value(pc) {
                let back = taken
                    .iter()
                    .position(|&(parent, key, d)| d == value && writes(pc, parent, key));
                if let Some(i) = back {
                    put_back[pc] = true;
                    taken.remove(i);
                }
            }
        }
        PlaceSites {
            field_at,
            take,
            clear,
            put_back,
        }
    }
}

/// Per register, the declared type of what it holds: each parameter's, then each declaration the region
/// seeded, then whatever the layout pass carried to a register the first two leave unknown.
fn declared_registers(
    structs: &kind::StructLayout,
    seeded: Vec<(usize, u16, BoundaryType)>,
    param_types: &[Option<BoundaryType>],
) -> std::collections::HashMap<u16, BoundaryType> {
    let mut declared: std::collections::HashMap<u16, BoundaryType> = param_types
        .iter()
        .enumerate()
        .filter_map(|(i, t)| t.clone().map(|t| (i as u16, t)))
        .collect();
    for (_, reg, bt) in seeded {
        declared.entry(reg).or_insert(bt);
    }
    for (reg, bt) in &structs.reg_declared {
        declared.entry(*reg).or_insert_with(|| bt.clone());
    }
    declared
}

/// Per register: whether every read of it is TRANSIENT — it only looks at the value in place, and
/// nothing can have changed a heap value between the register's definition and the read.
///
/// A read that takes a handle out of a global, a field or an item RETAINS it (word 12), so a later
/// mutation through either holder copies first — that is value semantics for a handle that may be
/// mutated or kept. A register that is only read — indexed, measured, compared, shown, handed to a
/// call (which retains what it passes) — within the block that defined it, with no op in between that
/// can change a heap value, is neither mutated nor kept: retaining it would only make the next
/// mutation of the source copy the whole value, which is what made reading a global list and then
/// writing it cost a copy of the list every time.
///
/// The reads are enumerated per op ([`pure_read_operands`]); any other use of a register — a move, a
/// store, an iteration, a return, a mutation — makes it non-transient. A register read in a block
/// other than the one that last defined it is non-transient too.
fn transient_reads(ops: &[Op], num_regs: u32, functions: &[CompiledFunction]) -> Vec<bool> {
    let mut transient = vec![true; num_regs as usize];
    let Some(blocks) = super::cfg::Blocks::new(ops) else {
        return vec![false; num_regs as usize];
    };
    // Registers defined in the current block since its start and since the last op that can change a
    // heap value.
    let mut clean: std::collections::HashSet<crate::vm::instruction::Reg> =
        std::collections::HashSet::new();
    for (pc, op) in ops.iter().enumerate() {
        if blocks.is_leader(pc) {
            clean.clear();
        }
        let (defs, uses) = crate::vm::instruction::op_def_uses(op, functions);
        let pure = pure_read_operands(op, functions);
        for r in uses {
            if !pure.contains(&r) || !clean.contains(&r) {
                if let Some(t) = transient.get_mut(r as usize) {
                    *t = false;
                }
            }
        }
        if !leaves_the_heap_alone(op) {
            clean.clear();
        }
        for r in defs {
            clean.insert(r);
        }
    }
    transient
}

/// The operands `op` only READS in place, keeping nothing: the registers [`transient_reads`] can
/// count a transient use. A call's arguments are here because the call retains every argument it
/// passes, and so are a host import's — the host takes them by value — except each one its row says
/// the host KEEPS past the call ([`HostFn::retained_params`]: a closure it re-enters, a handle it
/// stores). Anything that stores, moves, iterates, returns or mutates a register is not.
fn pure_read_operands(op: &Op, functions: &[CompiledFunction]) -> Vec<crate::vm::instruction::Reg> {
    match *op {
        Op::CallBuiltin {
            builtin,
            args_start,
            arg_count,
            ..
        } => match dom_import_of(builtin) {
            Some(host) => {
                let kept = host.retained_params();
                (0..arg_count)
                    .filter(|i| !kept.contains(&usize::from(*i)))
                    .map(|i| args_start + i)
                    .collect()
            }
            None => Vec::new(),
        },
        Op::Index {
            collection, index, ..
        }
        | Op::IndexUnchecked {
            collection, index, ..
        } => vec![collection, index],
        Op::Length { collection, .. } => vec![collection],
        Op::Contains {
            collection, value, ..
        } => vec![collection, value],
        Op::GetField { obj, .. } => vec![obj],
        Op::Show { src }
        | Op::Not { src, .. }
        | Op::FormatValue { src, .. }
        | Op::DeepClone { src, .. } => vec![src],
        Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => vec![cond],
        Op::SliceOp {
            collection,
            start,
            end,
            ..
        } => vec![collection, start, end],
        Op::Add { lhs, rhs, .. }
        | Op::Sub { lhs, rhs, .. }
        | Op::Mul { lhs, rhs, .. }
        | Op::Div { lhs, rhs, .. }
        | Op::ExactDiv { lhs, rhs, .. }
        | Op::FloorDiv { lhs, rhs, .. }
        | Op::Mod { lhs, rhs, .. }
        | Op::Pow { lhs, rhs, .. }
        | Op::Lt { lhs, rhs, .. }
        | Op::Gt { lhs, rhs, .. }
        | Op::LtEq { lhs, rhs, .. }
        | Op::GtEq { lhs, rhs, .. }
        | Op::Eq { lhs, rhs, .. }
        | Op::NotEq { lhs, rhs, .. }
        | Op::ApproxEq { lhs, rhs, .. }
        | Op::Concat { lhs, rhs, .. }
        | Op::SeqConcat { lhs, rhs, .. }
        | Op::UnionOp { lhs, rhs, .. }
        | Op::IntersectOp { lhs, rhs, .. } => vec![lhs, rhs],
        Op::Call { .. } => crate::vm::instruction::op_def_uses(op, functions).1,
        _ => Vec::new(),
    }
}

/// Whether `op` cannot change any heap value in place — the ops a transient read may sit across.
fn leaves_the_heap_alone(op: &Op) -> bool {
    matches!(
        op,
        Op::LoadConst { .. }
            | Op::Move { .. }
            | Op::Add { .. }
            | Op::Sub { .. }
            | Op::Mul { .. }
            | Op::Div { .. }
            | Op::ExactDiv { .. }
            | Op::FloorDiv { .. }
            | Op::Mod { .. }
            | Op::Pow { .. }
            | Op::DivPow2 { .. }
            | Op::MagicDivU { .. }
            | Op::Lt { .. }
            | Op::Gt { .. }
            | Op::LtEq { .. }
            | Op::GtEq { .. }
            | Op::Eq { .. }
            | Op::NotEq { .. }
            | Op::ApproxEq { .. }
            | Op::Not { .. }
            | Op::Concat { .. }
            | Op::SeqConcat { .. }
            | Op::BitXor { .. }
            | Op::BitAnd { .. }
            | Op::BitOr { .. }
            | Op::Shl { .. }
            | Op::Shr { .. }
            | Op::Index { .. }
            | Op::IndexUnchecked { .. }
            | Op::Length { .. }
            | Op::Contains { .. }
            | Op::GetField { .. }
            | Op::GlobalGet { .. }
            | Op::FormatValue { .. }
            | Op::SliceOp { .. }
            | Op::DeepClone { .. }
            | Op::UnionOp { .. }
            | Op::IntersectOp { .. }
            | Op::NewList { .. }
            | Op::NewEmptyList { .. }
            | Op::NewEmptyListWithCapacity { .. }
            | Op::NewEmptyListI32 { .. }
            | Op::NewEmptyListFixed { .. }
            | Op::NewEmptySet { .. }
            | Op::NewEmptyMap { .. }
            | Op::NewEmptyMapWithCapacity { .. }
            | Op::NewRange { .. }
            | Op::NewTuple { .. }
            | Op::NewStruct { .. }
            | Op::NewInductive { .. }
            | Op::TestArm { .. }
            | Op::BindArm { .. }
            | Op::Show { .. }
            | Op::LoadToday { .. }
            | Op::LoadNow { .. }
            | Op::Jump { .. }
            | Op::JumpIfFalse { .. }
            | Op::JumpIfTrue { .. }
    )
}

/// Whether the value `GlobalGet dst = g` at `pc` read is stored back into `g` from `dst` before anything
/// reads `g` again or control leaves straight-line code.
fn stored_back(ops: &[Op], pc: usize, g: u16, dst: u16) -> bool {
    for op in &ops[pc + 1..] {
        match *op {
            Op::GlobalSet { idx, src } if idx == g && src == dst => return true,
            Op::GlobalGet { idx, .. } if idx == g => return false,
            Op::Call { .. }
            | Op::CallValue { .. }
            | Op::CallBuiltin { .. }
            | Op::Jump { .. }
            | Op::JumpIfFalse { .. }
            | Op::JumpIfTrue { .. }
            | Op::IterNext { .. }
            | Op::Return { .. }
            | Op::ReturnNothing
            | Op::Halt => return false,
            _ => {}
        }
    }
    false
}

/// The slot of `field` (a field-name constant index) in the record held by `obj` — its declared layout
/// wherever the record's type is known, else the local layout of a record built here without one.
fn record_field_slot(plan: &Plan, obj: u16, field: u32) -> Option<usize> {
    if let Some(ParamShape::Struct(layout)) = plan.reg_shape.get(&obj) {
        return layout.iter().position(|(f, _, _)| *f == field);
    }
    plan.structs
        .reg_layout
        .get(&obj)?
        .iter()
        .position(|(f, _)| *f == field)
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
    global_layout: &dyn Fn(u16) -> Option<FieldLayout>,
    global_declared: &dyn Fn(u16) -> Option<BoundaryType>,
    linked: bool,
    functions: &[crate::vm::instruction::CompiledFunction],
    inout: &[u16],
) -> R<(KindTable, Vec<bool>, Vec<bool>)> {
    let blocks = Blocks::new(ops).ok_or(WasmLowerError::Unsupported(
        "jump target escapes the function",
    ))?;
    let (pc_reach, block_reach) = reachability(ops, &blocks);
    // The general Int-overflow→BigInt promotion set (empty unless linked): registers only observed or
    // fed into more BigInt arithmetic, computed once over this region's post-regsplit ops.
    let bigint_demand = kind::bigint_demanded_regs(ops, functions, linked);
    // A loan clears the caller's slot, then restores it before any read. Keep
    // the clear in emitted code (including for heap roots), but do not unify
    // an unobserved absence with the slot's payload type.
    let dead_clears = regsplit::unobserved_nothing_writes(ops, constants, functions, inout);
    let kind_reach: Vec<_> = pc_reach
        .iter()
        .enumerate()
        .map(|(pc, &reachable)| reachable && !dead_clears.contains(&pc))
        .collect();
    let kinds = kind::infer(
        ops,
        constants,
        struct_types,
        enum_types,
        fn_return_types,
        num_regs,
        seeds,
        ret_of,
        global_of,
        closure_ret,
        ret_layout,
        fn_return_closure,
        param_layouts,
        param_closures,
        global_layout,
        global_declared,
        &kind_reach,
        linked,
        &bigint_demand,
    )?;
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
            Op::Return { .. }
            | Op::ReturnNothing
            | Op::Halt
            | Op::FailWith { .. }
            | Op::CallBuiltin {
                builtin:
                    BuiltinId::Fail
                    | BuiltinId::FailReason
                    | BuiltinId::VfsMount
                    | BuiltinId::VfsWriteFile,
                ..
            } => return vec![],
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

/// The failure a browser page reports for `Mount … at …` and `Write … to file …`: the file system
/// those statements read and write is the compiled tiers', and a page has none.
pub const VFS_IN_THE_BROWSER: &str =
    "a page in the browser has no file system: mounting a journal and writing a file run on the compiled tiers";

/// Lower a whole program to a self-contained WebAssembly module (the bytes of a `.wasm` file).
/// Returns [`WasmLowerError::Unsupported`] for any program outside the scalar fragment — a
/// sound refusal, never a wrong module.
pub fn assemble_program(
    program: &CompiledProgram,
    policies: &PolicyRegistry,
    interner: &Interner,
) -> R<Vec<u8>> {
    assemble_program_impl(
        program,
        policies,
        interner,
        false,
        Reclaim::Counted,
        &HostEntries::default(),
    )
}

/// [`assemble_program`] for a host that also enters the module by name beyond `main` — each
/// component in `entries` stays in the module (reached from its mount as `main` reaches the page's
/// own) and its `__mount_<C>` is exported under that name. With the default entries this IS
/// [`assemble_program`].
pub fn assemble_program_with(
    program: &CompiledProgram,
    policies: &PolicyRegistry,
    interner: &Interner,
    entries: &HostEntries,
) -> R<Vec<u8>> {
    assemble_program_impl(
        program,
        policies,
        interner,
        false,
        Reclaim::Counted,
        entries,
    )
}

/// [`assemble_program_with`] as a check-mode module (`Reclaim::Checked`): the same module with the
/// heap census exported (`__logos_heap_census`), which a heap test reads every true count back
/// through. A shipped page never carries it.
pub fn assemble_program_checked(
    program: &CompiledProgram,
    policies: &PolicyRegistry,
    interner: &Interner,
    entries: &HostEntries,
) -> R<Vec<u8>> {
    assemble_program_impl(
        program,
        policies,
        interner,
        false,
        Reclaim::Checked,
        entries,
    )
}

/// Reclamation for standalone modules. Pages collect at the end of a host turn; batch
/// programs pin live caller registers and collect at loop backedges and main's exit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reclaim {
    /// Blocks are reused only where their death is structural — a grown buffer's old one.
    Off,
    /// True holder counts, the turn's retained list, and the safepoint that frees what nothing holds
    /// once the host's turn ends.
    Counted,
    /// [`Reclaim::Counted`] plus `__logos_heap_census`, the check-mode instrument that recomputes
    /// every count from the program's roots.
    Checked,
}

/// Linker-mode assembly: an integer `Op::Pow` lowers to the real `logicaffeine_base::BigInt` runtime
/// (`logos_rt_bigint_from_i64`→`_pow`→`_to_text`) yielding a `Text` handle, so an overflowing
/// `x to the power of y` computes the exact big integer the VM's BigInt promotion prints instead of
/// trapping. The module imports `env.__linear_memory` (the linker supplies one shared memory) and the
/// three `logos_rt_bigint_*` functions by undefined symbol; [`super::reloc::module_to_relocatable`] +
/// `rust-lld` link it against the prebuilt base runtime. Emitter-side heap allocation is refused (two
/// allocators over one linear memory would corrupt it — the only heap value is the runtime-built Text).
pub(crate) fn assemble_program_linked(
    program: &CompiledProgram,
    policies: &PolicyRegistry,
    interner: &Interner,
) -> R<Vec<u8>> {
    assemble_program_impl(
        program,
        policies,
        interner,
        true,
        Reclaim::Off,
        &HostEntries::default(),
    )
}

/// A refusal that names where it happened: the function whose planning or lowering stopped (`Main`
/// for the program's own top level). The backend's own words say what it could not do; the name is
/// what a reader can go and look at.
fn within_function(
    e: WasmLowerError,
    program: &CompiledProgram,
    interner: &Interner,
    fi: Option<usize>,
) -> WasmLowerError {
    let name = match fi {
        None => "Main".to_string(),
        Some(fi) => match program.functions.get(fi).map(|f| interner.resolve(f.name)) {
            Some(name) if !name.is_empty() => format!("`{name}`"),
            _ => format!("function #{fi}"),
        },
    };
    let what = match e {
        WasmLowerError::Unsupported(what) => what.to_string(),
        WasmLowerError::UnsupportedOwned(what) => what,
    };
    WasmLowerError::UnsupportedOwned(format!("{what} (in {name})"))
}

fn assemble_program_impl(
    program: &CompiledProgram,
    policies: &PolicyRegistry,
    interner: &Interner,
    linked: bool,
    reclaim: Reclaim,
    entries: &HostEntries,
) -> R<Vec<u8>> {
    if !linked && crate::vm::tower::reaches(program) {
        return Err(WasmLowerError::UnsupportedOwned(tower::REFUSAL.to_string()));
    }
    // ---- 1. Plan Main + every user function (rebase, infer kinds, resolve result kind) ----
    let layout = crate::vm::reach::code_layout(program).map_err(WasmLowerError::Unsupported)?;
    let ret_of = |fi: usize| -> Option<Kind> { declared_result(program, fi) };
    let no_closure = |_: usize| -> Option<Kind> { None };
    let no_ret_layout = |_: u16| -> Option<FieldLayout> { None };
    let no_ret_closure = |_: u16| -> Option<u16> { None };
    let no_param_origin = |_: usize, _: usize| -> Option<u16> { None };
    let no_exact_param = |_: usize, _: usize| false;

    // Plan Main once with no resolvers — it types both the values stored into globals (read below)
    // AND the local registers a `MakeClosure` captures (the cross-scope closure-capture flow below).
    let num_globals = program.globals.len();
    let no_globals = |_: u16| None;
    let no_global_layout = |_: u16| -> Option<FieldLayout> { None };
    // The bootstrap pass runs BEFORE global types are known, so it answers "nothing known" for a
    // global map exactly as it does for a global record — the strict pass below is where the real
    // answers arrive.
    let no_global_declared = |_: u16| -> Option<BoundaryType> { None };
    let no_caps: Vec<Vec<Option<Kind>>> = Vec::new();
    let main_p1 = plan_main(
        program,
        &layout,
        &ret_of,
        &no_globals,
        &no_closure,
        &no_ret_layout,
        &no_ret_closure,
        &no_global_layout,
        &no_global_declared,
        linked,
    )?;
    // ★ A GLOBAL'S KIND IS ITS DECLARATION'S, where it has one. What Main stored says only what the
    // value looked like when it went in, and `a new Seq of Text` goes in EMPTY: Main never pushes to
    // a page's outbox or a store's list, so its kind there is an element-less sequence, and every
    // function reading it — the only place anything is ever read out — was refused ("Show of an
    // unknown-kind value") for a list whose element type is written in the declaration. The
    // inferred kind stays the answer for a global with no declaration, and wins a disagreement in
    // width (a value can only be stored at the width it has).
    let global_kinds_cell: std::cell::RefCell<Vec<Option<Kind>>> = std::cell::RefCell::new({
        let mut gk = vec![None; num_globals];
        for (pc, op) in main_p1.ops.iter().enumerate() {
            // Taking and returning a mutable loan do not redeclare the global's payload.
            // The first planning pass has not resolved GlobalGet yet, so a put-back's
            // temporary register can still have the fallback integer kind.
            if main_p1.place.clear[pc] || main_p1.place.put_back[pc] {
                continue;
            }
            if let Op::GlobalSet { idx, src } = *op {
                if let Some(k) = main_p1.kinds.get(src as usize) {
                    if k != Kind::Nothing {
                        gk[idx as usize] = Some(k);
                    }
                }
            }
        }
        for (idx, k) in gk.iter_mut().enumerate() {
            let declared = program
                .global_types
                .get(idx)
                .and_then(|t| t.as_ref())
                .and_then(kind::boundary_to_kind);
            if let Some(d) = declared {
                if k.map_or(true, |k| k.wasm_valtype() == d.wasm_valtype()) {
                    *k = Some(d);
                }
            }
        }
        gk
    });
    let global_of =
        |idx: u16| -> Option<Kind> { global_kinds_cell.borrow().get(idx as usize).copied().flatten() };
    // ★ Per global, the RESOLVED FIELD LAYOUT of the record it holds — the global analog of a
    // struct parameter's `ParamShape::Struct`, and the thing that lets a FUNCTION read a
    // module-level record's field. Main builds the record, so its declared type is known there;
    // a function that only reads the global has no value register for the fields and, without
    // this, resolved no static slot at all ("field access with no static slot"). That refusal
    // took out every `## Store` on this tier — a store IS a record global, read by every action.
    let global_layouts: Vec<Option<FieldLayout>> = {
        let mut gl = vec![None; num_globals];
        for op in &main_p1.ops {
            if let Op::GlobalSet { idx, src } = *op {
                if main_p1.kinds.get(src as usize) == Some(Kind::Struct) {
                    gl[idx as usize] = global_struct_layout(
                        &main_p1,
                        src,
                        &program.struct_types,
                        &program.constants,
                    );
                }
            }
        }
        gl
    };
    let global_layout_of =
        |idx: u16| -> Option<FieldLayout> { global_layouts.get(idx as usize).cloned().flatten() };
    // ★ AND THE GENERAL ANALOG: the global's DECLARED TYPE, handed over whole.
    //
    // A function that only READS a global has no construction to trace, so every layout it needs
    // must come from the declaration — the same position a struct parameter is in, answered the
    // same way. This used to be THREE resolvers, each flattening the declared type into one
    // pre-chewed answer: the element layout of a `Seq of <record>`, the value kind and layout of a
    // `Map of K to <record>`, and finally the element layout of a `Map of K to (Seq of <record>)`.
    //
    // ★ Each was added after a real program was refused, and each left the next nesting broken. The
    // last one existed because per-instance component state wraps every field in a map, so a page's
    // `Seq of Memo` arrives here as `Map of Text to (Seq of Memo)` — and a list of lists, a map of
    // maps and a list of maps were all still refused with "field access with no static slot" after
    // it landed. A type is a TREE; three flattenings of it are three chances to have missed a
    // branch. Passing the tree costs a clone per index and cannot miss one.
    let global_declared_of = |idx: u16| -> Option<BoundaryType> {
        program.global_types.get(idx as usize).cloned().flatten()
    };
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
    let global_closure_of =
        |idx: u16| -> Option<u16> { global_closures.get(idx as usize).copied().flatten() };

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
            // ★ THE WARM-UP TOLERATES A FAILURE, which is what this pass's own note says it does
            // and what the strict pass below exists to be the backstop for. It propagated instead,
            // and the difference is not academic: this round passes `no_global_layout` for BOTH
            // global shapes, so a function reading a composite GLOBAL — which is every component's
            // render function, since `State:` lives in module bindings — cannot resolve it HERE and
            // resolves it perfectly THERE. A page was therefore refused by the pass that is not
            // allowed to refuse anything, with a message about register kinds that the strict pass
            // would never have produced.
            plans[fi] = plan_function(
                program,
                fi,
                &layout,
                &ret_of,
                &global_of,
                &no_closure,
                &no_ret_layout,
                &no_ret_closure,
                &no_global_layout,
                &no_global_declared,
                &no_param_origin,
                &no_exact_param,
                &no_caps,
                &no_shapes,
                &no_capture_closures,
                false,
                linked,
            )
            .ok();
        }
    }
    let mut capture_kinds: Vec<Vec<Option<Kind>>> = Vec::new();
    let mut capture_shapes: Vec<Vec<Option<ParamShape>>> = Vec::new();
    let mut capture_closures: Vec<Vec<Option<u16>>> = Vec::new();
    let mut exact_args: Vec<Vec<bool>> = program
        .functions
        .iter()
        .map(|f| vec![false; f.param_count as usize])
        .collect();
    for _ in 0..(program.functions.len() + 4) {
        let (ck, cs, cc) =
            extract_capture_kinds(program, &main_p1, &plans, &global_of, &global_closure_of);
        capture_kinds = ck;
        capture_shapes = cs;
        capture_closures = cc;
        let cur_results: Vec<Option<Kind>> = plans
            .iter()
            .map(|p| p.as_ref().and_then(|p| p.result))
            .collect();
        let cur_layouts: Vec<Option<FieldLayout>> = plans
            .iter()
            .map(|p| {
                p.as_ref().and_then(|p| {
                    fn_return_struct_layout(p, &program.struct_types, &program.constants)
                })
            })
            .collect();
        let cur_ret_closures: Vec<Option<u16>> = plans
            .iter()
            .map(|p| p.as_ref().and_then(|p| p.return_closure))
            .collect();
        let closure_ret = |fi: usize| cur_results.get(fi).copied().flatten();
        let ret_of_i = |fi: usize| {
            cur_results
                .get(fi)
                .copied()
                .flatten()
                .or_else(|| declared_result(program, fi))
        };
        let ret_layout_of = |func: u16| cur_layouts.get(func as usize).cloned().flatten();
        let ret_closure_of = |func: u16| cur_ret_closures.get(func as usize).copied().flatten();
        // CLOSURES AS ARGUMENTS. Re-plan Main with this round's resolvers so its `closure_of` reflects
        // a returned closure bound to a local (then passed straight into a function); attribute every
        // call's closure arguments to the parameters they feed, so `f(args)` resolves when `f` is a
        // parameter. Recomputed each round — a param's origin can resolve transitively as callers plan.
        let main_iter = plan_main(
            program,
            &layout,
            &ret_of_i,
            &global_of,
            &closure_ret,
            &ret_layout_of,
            &ret_closure_of,
            &global_layout_of,
            &global_declared_of,
            linked,
        )
        .ok();
        let mut scan_plans: Vec<&Plan> = vec![main_iter.as_ref().unwrap_or(&main_p1)];
        scan_plans.extend(plans.iter().flatten());
        let param_origins = compute_param_origins(program, &scan_plans);
        let param_origin = |fi: usize, pi: usize| {
            param_origins
                .get(fi)
                .and_then(|v| v.get(pi))
                .copied()
                .flatten()
        };
        let mut progress = exact_arguments(program, &scan_plans, &mut exact_args);
        let exact_param = |fi: usize, pi: usize| exact_args.get(fi).and_then(|v| v.get(pi)).copied().unwrap_or(false);
        for fi in 0..program.functions.len() {
            if let Ok(p) = plan_function(
                program,
                fi,
                &layout,
                &ret_of_i,
                &global_of,
                &closure_ret,
                &ret_layout_of,
                &ret_closure_of,
                &global_layout_of,
                &global_declared_of,
                &param_origin,
                &exact_param,
                &capture_kinds,
                &capture_shapes,
                &capture_closures,
                false,
                linked,
            ) {
                let changed = plans[fi].as_ref().map_or(true, |x| {
                    x.result != p.result || x.return_closure != p.return_closure
                });
                if changed {
                    progress = true;
                }
                plans[fi] = Some(p);
            }
        }
        // A global any region stores a value of the exact extension into holds one: every region is planned
        // again reading it as one, and a value of another kind stored into it lands converted.
        if linked {
            let regions = main_iter.iter().chain(plans.iter().flatten());
            progress |= tower::exact_globals(regions, &mut global_kinds_cell.borrow_mut());
        }
        if !progress {
            break;
        }
    }
    // ★ A FUNCTION THE WARM-UP COULD NOT PLAN IS NOT AN ERROR HERE. This round deliberately runs
    // WITHOUT the global layouts and capture shapes, so a function that needs them resolves only in
    // the strict pass below — which re-plans every function anyway and is the one allowed to refuse.
    // A stub contributes no result kind and no return layout, which is exactly "not known yet"; its
    // callers simply resolve less in this round and re-resolve in the next.
    let fns1: Vec<Plan> = plans
        .into_iter()
        .map(|p| p.unwrap_or_else(Plan::stub))
        .collect();

    // Pass 2 (STRICT) re-plans every function WITH the resolvers (a call's inferred result kind +
    // struct-return field layout), so cross-region `f(…)'s field` resolves and a genuinely unknown
    // return is rejected rather than deferred. `capture_kinds` carries through so closure bodies keep
    // their capture typing. Resolvers affect only `GetField`, so pass-1 results/layouts stay valid.
    let fn_results_p1: Vec<Option<Kind>> = fns1.iter().map(|p| p.result).collect();
    let ret_layouts: Vec<Option<FieldLayout>> = fns1
        .iter()
        .map(|p| fn_return_struct_layout(p, &program.struct_types, &program.constants))
        .collect();
    let ret_closures: Vec<Option<u16>> = fns1.iter().map(|p| p.return_closure).collect();
    let closure_ret = |fi: usize| -> Option<Kind> { fn_results_p1.get(fi).copied().flatten() };
    let ret_of_inferred = |fi: usize| -> Option<Kind> {
        fn_results_p1
            .get(fi)
            .copied()
            .flatten()
            .or_else(|| declared_result(program, fi))
    };
    let ret_layout_of =
        |func: u16| -> Option<FieldLayout> { ret_layouts.get(func as usize).cloned().flatten() };
    let ret_closure_of =
        |func: u16| -> Option<u16> { ret_closures.get(func as usize).copied().flatten() };
    // Main is planned first (it doesn't depend on the strict `fns`, only on the converged resolvers)
    // so it can join the param-origin scan — a closure argument is frequently passed from Main.
    let mut main = plan_main(
        program,
        &layout,
        &ret_of_inferred,
        &global_of,
        &closure_ret,
        &ret_layout_of,
        &ret_closure_of,
        &global_layout_of,
        &global_declared_of,
        linked,
    )
    .map_err(|e| within_function(e, program, interner, None))?;
    let scan_plans: Vec<&Plan> = std::iter::once(&main).chain(fns1.iter()).collect();
    let param_origins = compute_param_origins(program, &scan_plans);
    let param_origin = |fi: usize, pi: usize| {
        param_origins
            .get(fi)
            .and_then(|v| v.get(pi))
            .copied()
            .flatten()
    };
    exact_arguments(program, &scan_plans, &mut exact_args);
    let exact_param = |fi: usize, pi: usize| exact_args.get(fi).and_then(|v| v.get(pi)).copied().unwrap_or(false);
    // Function-level DCE: a function that cannot run (see `vm::reach`, the analysis the
    // unlinkable-code gate also judges by) is dropped to a trap stub instead of being strictly
    // planned, so a program can demand a large stdlib module whose unused functions use ops this
    // backend does not lower.
    let reachable = crate::vm::reach::reach_with(program, &layout, interner, entries).functions;
    let mut fns: Vec<Plan> = {
        let mut v = Vec::with_capacity(program.functions.len());
        for fi in 0..program.functions.len() {
            if !reachable[fi] {
                if std::env::var_os("LOGOS_WASM_TRACE_FIELD").is_some() {
                    if let Some(f) = program.functions.get(fi) {
                        eprintln!(
                            "[wasm] stubbed (unreachable from Main): '{}'",
                            interner.resolve(f.name)
                        );
                    }
                }
                v.push(Plan::stub());
                continue;
            }
            match plan_function(
                program,
                fi,
                &layout,
                &ret_of_inferred,
                &global_of,
                &closure_ret,
                &ret_layout_of,
                &ret_closure_of,
                &global_layout_of,
                &global_declared_of,
                &param_origin,
                &exact_param,
                &capture_kinds,
                &capture_shapes,
                &capture_closures,
                true,
                linked,
            ) {
                Ok(p) => v.push(p),
                Err(e) => return Err(within_function(e, program, interner, Some(fi))),
            }
        }
        v
    };
    if tower::decide(program, &mut main, &mut fns, &global_kinds_cell.borrow()) && !linked {
        return Err(WasmLowerError::UnsupportedOwned(tower::RATIONAL_LISTS.to_string()));
    }
    // Which integers outgrow a machine word, and the tagged words that hold them ([`exact`]). A function
    // the host enters by name keeps machine words at its boundary: the module does not see that call.
    let entry_names = entries.names();
    let raw_boundary: Vec<bool> = program
        .functions
        .iter()
        .map(|f| entry_names.iter().any(|n| interner.resolve(f.name) == n))
        .collect();
    let exact_sig = exact::decide(
        program,
        &mut main,
        &mut fns,
        &raw_boundary,
        num_globals,
        linked,
    );
    let presence_enabled =
        absence::decide(&mut main, &mut fns, &program.constants, num_globals, linked);

    // Whether the program uses the emitter's heap value model (a linear memory + bump allocator), an
    // iterator stack, or closures — computed HERE (before the import scan) so linker mode can import the
    // runtime allocator for the slab + refuse the shapes it can't yet share.
    // A heap op, OR a `LoadConst` of a Text literal (which materializes a Text object in memory).
    // A `FailWith` materializes its message Text before trapping, exactly as a `LoadConst` of a
    // Text does — so it needs the heap too. Missing this emitted a module whose `emit_alloc`
    // reached for a `__heap_ptr` global that was never declared: the bytes COMPILED and did not
    // VALIDATE, which `compile_to_wasm` reports as success.
    let loads_text = |op: &Op| match op {
        Op::LoadConst { idx, .. } | Op::FailWith { msg: idx } => {
            matches!(
                program.constants.get(*idx as usize),
                Some(Constant::Text(_))
            )
        }
        // A failure reason that is not already Text is materialized as one (its rendering, or the
        // refusal of a non-Text contract reason) before the host call.
        Op::CallBuiltin {
            builtin: BuiltinId::Fail | BuiltinId::FailReason,
            ..
        } => true,
        Op::CallBuiltin {
            builtin: BuiltinId::VfsMount | BuiltinId::VfsWriteFile,
            ..
        } => true,
        _ => false,
    };
    // A Text-typed `+`/`+=` (string concatenation) allocates a fresh Text, so it needs the heap even
    // if the program has no Text literal (it builds from a Text-valued variable).
    let concats_text = |p: &Plan, op: &Op| match *op {
        Op::Add { dst, .. } | Op::AddAssign { dst, .. } => {
            p.kinds.get(dst as usize) == Some(Kind::Text)
        }
        _ => false,
    };
    // Every composite value a module renders as Text renders through its shape's display function.
    let shown_shapes = {
        let plans: Vec<&Plan> = std::iter::once(&main).chain(fns.iter()).collect();
        display::ShownTable::new(display::ShownTable::collect(
            &plans,
            &program.struct_types,
            &program.enum_types,
        ))
    };
    let uses_heap = shown_shapes.len() > 0
        || std::iter::once(&main).chain(fns.iter()).any(|p| p.tower.holds())
        || std::iter::once(&main).chain(fns.iter()).any(|p| {
            p.ops
                .iter()
                .any(|op| op_uses_heap(op) || loads_text(op) || concats_text(p, op))
        });
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
    tower::note_imports(std::iter::once(&main).chain(fns.iter()), &mut used);
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
    let note_operand_fmt = |plan: &Plan, r: u16, used: &mut Vec<HostFn>| {
        note_fmt_kind(plan.kinds.get(r as usize), used)
    };
    for plan in std::iter::once(&main).chain(fns.iter()) {
        for (pc, op) in plan.ops.iter().enumerate() {
            if linked {
                let field_write = match *op {
                    Op::StructInsert { obj, value, .. } => plan
                        .structs
                        .slot
                        .get(pc)
                        .copied()
                        .flatten()
                        .map(|slot| (obj, slot as usize, value)),
                    Op::SetIndex {
                        collection, value, ..
                    }
                    | Op::SetIndexUnchecked {
                        collection, value, ..
                    } => plan
                        .place
                        .field_at
                        .get(pc)
                        .copied()
                        .flatten()
                        .and_then(|field| record_field_slot(plan, collection, field))
                        .map(|slot| (collection, slot, value)),
                    _ => None,
                };
                if let Some((obj, slot, value)) = field_write {
                    let rational_field = declared_record_slot_kind(
                        &plan.structs,
                        &program.struct_types,
                        &program.constants,
                        obj,
                        slot,
                    ) == Some(Kind::Rational);
                    match plan.kinds.get(value as usize) {
                        Some(Kind::Int) if rational_field => note(HostFn::RationalFromI64, &mut used),
                        Some(Kind::BigInt) if rational_field => note(HostFn::RationalFromBigint, &mut used),
                        _ => {}
                    }
                }
                // An integer handed to a rational parameter is the rational it equals.
                if let Op::Call {
                    func,
                    args_start,
                    arg_count,
                    ..
                }
                | Op::Spawn {
                    func,
                    args_start,
                    arg_count,
                }
                | Op::SpawnHandle {
                    func,
                    args_start,
                    arg_count,
                    ..
                } = *op
                {
                    let callee = fns.get(func as usize);
                    for a in 0..arg_count {
                        if callee.and_then(|f| f.kinds.get(a as usize)) == Some(Kind::Rational) {
                            match plan.kinds.get((args_start + a) as usize) {
                                Some(Kind::Int) => note(HostFn::RationalFromI64, &mut used),
                                Some(Kind::BigInt) => note(HostFn::RationalFromBigint, &mut used),
                                _ => {}
                            }
                        }
                    }
                }
                // A comparison over a rational orders both sides as rationals ([`lower_rational_compare`]).
                if let Op::Lt { lhs, rhs, .. }
                | Op::Gt { lhs, rhs, .. }
                | Op::LtEq { lhs, rhs, .. }
                | Op::GtEq { lhs, rhs, .. }
                | Op::Eq { lhs, rhs, .. }
                | Op::NotEq { lhs, rhs, .. } = *op
                {
                    let kind = |r: u16| plan.kinds.get(r as usize);
                    if (kind(lhs) == Some(Kind::Rational) || kind(rhs) == Some(Kind::Rational))
                        && (kind(lhs) == Some(Kind::Float) || kind(rhs) == Some(Kind::Float))
                    {
                        note(HostFn::RationalCmpF64, &mut used);
                    } else if kind(lhs) == Some(Kind::Rational) || kind(rhs) == Some(Kind::Rational) {
                        note(HostFn::RationalCmp, &mut used);
                        for r in [lhs, rhs] {
                            match kind(r) {
                                Some(Kind::Int) => note(HostFn::RationalFromI64, &mut used),
                                Some(Kind::BigInt) => note(HostFn::RationalFromBigint, &mut used),
                                _ => {}
                            }
                        }
                    }
                }
            }
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
                        // the key AND value formatters, from the kinds its lowering will use.
                        note(HostFn::PrintText, &mut used);
                        if let MapShowKinds::Entries(k, v) = map_show_kinds(plan, src) {
                            note_fmt_kind(k, &mut used);
                            note_fmt_kind(v, &mut used);
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
                        let inner = plan
                            .structs
                            .opt_inner
                            .get(&src)
                            .copied()
                            .unwrap_or(Kind::Int);
                        if let Some(h) = HostFn::for_show(inner) {
                            note(h, &mut used);
                        }
                    } else if matches!(
                        plan.kinds.get(src as usize),
                        Some(Kind::Word32) | Some(Kind::Word64)
                    ) {
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
                    } else if let Some(h) = plan.kinds.get(src as usize).and_then(HostFn::for_show)
                    {
                        note(h, &mut used);
                    }
                }
                Op::CallBuiltin {
                    builtin: BuiltinId::Pow,
                    args_start,
                    ..
                } => {
                    let bk = plan.kinds.get(args_start as usize);
                    let ek = plan.kinds.get((args_start + 1) as usize);
                    if let Some(h) = pow_host_for(bk, ek) {
                        note(h, &mut used);
                    }
                }
                // `a ** b` (the operator) shares `pow`'s host `pow_ff`/`pow_fi` for a Float result.
                // In LINKER mode an integer power drives the `logos_rt_bigint_*` runtime, leaving a
                // BigInt HANDLE (its `to_text` render is noted by the `Show` arm, not here).
                Op::Pow { dst, lhs, rhs } if plan.kinds.get(dst as usize) != Some(Kind::Rational) => {
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
                    if plan.kinds.get(lhs as usize) == Some(Kind::Int)
                        || plan.kinds.get(rhs as usize) == Some(Kind::Int)
                    {
                        note(HostFn::BigintFromI64, &mut used);
                    }
                }
                // `+ - *` producing a Complex (linker mode) call the matching `logos_rt_complex_*` sink;
                // an `Int` operand is promoted with `from_i64` (real `n + 0i`).
                Op::Add { dst, lhs, rhs }
                | Op::Sub { dst, lhs, rhs }
                | Op::Mul { dst, lhs, rhs }
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
                    if plan.kinds.get(lhs as usize) == Some(Kind::Int)
                        || plan.kinds.get(rhs as usize) == Some(Kind::Int)
                    {
                        note(HostFn::ComplexFromI64, &mut used);
                    }
                }
                // `complex(re, im)` builds a Complex handle: the int-part builder for two Int parts,
                // else the float-part builder (a `ComplexF`, parts as exact dyadic Rationals).
                Op::CallBuiltin {
                    builtin: BuiltinId::Complex,
                    args_start,
                    ..
                } if linked => {
                    if plan.kinds.get(args_start as usize) == Some(Kind::Int)
                        && plan.kinds.get((args_start + 1) as usize) == Some(Kind::Int)
                    {
                        note(HostFn::ComplexFromI64, &mut used);
                    } else {
                        note(HostFn::ComplexFromF64, &mut used);
                    }
                }
                // `abs(z)` of a Complex → its `f64` modulus via `logos_rt_complex_abs`.
                Op::CallBuiltin {
                    builtin: BuiltinId::Abs,
                    args_start,
                    ..
                } if linked && plan.kinds.get(args_start as usize) == Some(Kind::Complex) => {
                    note(HostFn::ComplexAbs, &mut used);
                }
                // `re(z)` / `im(z)` of a Complex → its parts as `f64`.
                Op::CallBuiltin {
                    builtin: BuiltinId::RealPart,
                    args_start,
                    ..
                } if linked && plan.kinds.get(args_start as usize) == Some(Kind::Complex) => {
                    note(HostFn::ComplexReF64, &mut used);
                }
                Op::CallBuiltin {
                    builtin: BuiltinId::ImagPart,
                    args_start,
                    ..
                } if linked && plan.kinds.get(args_start as usize) == Some(Kind::Complex) => {
                    note(HostFn::ComplexImF64, &mut used);
                }
                Op::Add { dst, lhs, rhs }
                | Op::Sub { dst, lhs, rhs }
                | Op::Mul { dst, lhs, rhs }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Modular) =>
                {
                    note(
                        match *op {
                            Op::Add { .. } => HostFn::ModularAdd,
                            Op::Sub { .. } => HostFn::ModularSub,
                            _ => HostFn::ModularMul,
                        },
                        &mut used,
                    );
                    if plan.kinds.get(lhs as usize) == Some(Kind::Int)
                        || plan.kinds.get(rhs as usize) == Some(Kind::Int)
                    {
                        note(HostFn::ModularFromI64, &mut used);
                    }
                }
                Op::CallBuiltin {
                    builtin: BuiltinId::Modular,
                    ..
                } if linked => note(HostFn::ModularFromI64, &mut used),
                // `modular ÷ modular` (multiply by the inverse) and `pow(m, e)` modular exponentiation.
                Op::Div { dst, .. } | Op::ExactDiv { dst, .. }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Modular) =>
                {
                    note(HostFn::ModularDiv, &mut used);
                }
                Op::Add { dst, lhs, rhs }
                | Op::Sub { dst, lhs, rhs }
                | Op::Mul { dst, lhs, rhs }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Decimal) =>
                {
                    note(
                        match *op {
                            Op::Add { .. } => HostFn::DecimalAdd,
                            Op::Sub { .. } => HostFn::DecimalSub,
                            _ => HostFn::DecimalMul,
                        },
                        &mut used,
                    );
                    if plan.kinds.get(lhs as usize) == Some(Kind::Int)
                        || plan.kinds.get(rhs as usize) == Some(Kind::Int)
                    {
                        note(HostFn::DecimalFromI64, &mut used);
                    }
                }
                Op::CallBuiltin {
                    builtin: BuiltinId::Decimal,
                    ..
                } if linked => note(HostFn::DecimalFromText, &mut used),
                Op::Add { dst, lhs, rhs } | Op::Sub { dst, lhs, rhs }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Money) =>
                {
                    let _ = (lhs, rhs);
                    note(
                        match *op {
                            Op::Add { .. } => HostFn::MoneyAdd,
                            _ => HostFn::MoneySub,
                        },
                        &mut used,
                    );
                }
                Op::CallBuiltin {
                    builtin: BuiltinId::Money,
                    ..
                } if linked => {
                    note(HostFn::MoneyFromDecimal, &mut used);
                    note(HostFn::MoneyFromI64, &mut used);
                }
                Op::Add { dst, lhs, rhs }
                | Op::Sub { dst, lhs, rhs }
                | Op::Mul { dst, lhs, rhs }
                | Op::Div { dst, lhs, rhs }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Quantity) =>
                {
                    let _ = (lhs, rhs);
                    note(
                        match *op {
                            Op::Add { .. } => HostFn::QuantityAdd,
                            Op::Sub { .. } => HostFn::QuantitySub,
                            Op::Mul { .. } => HostFn::QuantityMul,
                            _ => HostFn::QuantityDiv,
                        },
                        &mut used,
                    );
                }
                Op::CallBuiltin {
                    builtin: BuiltinId::Quantity,
                    ..
                } if linked => note(HostFn::QuantityOfI64, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::Convert,
                    ..
                } if linked => note(HostFn::QuantityConvert, &mut used),
                // `+ - * /` on a Rational operand (linker): the BigInt-backed runtime op + a `from_i64`/
                // `from_bigint` promotion for any Int/BigInt operand that must widen to a Rational first.
                // (`/` here is a bare `Div` on two Rationals — the exact-division literal is `ExactDiv`.)
                Op::Add { dst, lhs, rhs }
                | Op::Sub { dst, lhs, rhs }
                | Op::Mul { dst, lhs, rhs }
                | Op::Div { dst, lhs, rhs }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Rational) =>
                {
                    note(
                        match *op {
                            Op::Add { .. } => HostFn::RationalAdd,
                            Op::Sub { .. } => HostFn::RationalSub,
                            Op::Mul { .. } => HostFn::RationalMul,
                            _ => HostFn::RationalQuotient,
                        },
                        &mut used,
                    );
                    for r in [lhs, rhs] {
                        match plan.kinds.get(r as usize) {
                            Some(Kind::Int) => note(HostFn::RationalFromI64, &mut used),
                            Some(Kind::BigInt) => note(HostFn::RationalFromBigint, &mut used),
                            _ => {}
                        }
                    }
                }
                // `%`, `//`, `**` on a Rational (linker): the VM's rule for exact numbers in the runtime, with the
                // same promotions of an Int/BigInt operand.
                Op::Mod { dst, lhs, rhs } | Op::FloorDiv { dst, lhs, rhs } | Op::Pow { dst, lhs, rhs }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Rational) =>
                {
                    note(
                        match *op {
                            Op::Mod { .. } => HostFn::RationalRemainder,
                            Op::FloorDiv { .. } => HostFn::RationalFloorQuotient,
                            _ => HostFn::RationalPower,
                        },
                        &mut used,
                    );
                    for r in [lhs, rhs] {
                        match plan.kinds.get(r as usize) {
                            Some(Kind::Int) => note(HostFn::RationalFromI64, &mut used),
                            Some(Kind::BigInt) => note(HostFn::RationalFromBigint, &mut used),
                            _ => {}
                        }
                    }
                }
                // A bitwise operator over a rational (linker): the integers two whole ones make.
                Op::BitXor { lhs, rhs, .. } | Op::BitAnd { lhs, rhs, .. } | Op::BitOr { lhs, rhs, .. }
                    if linked
                        && (plan.kinds.get(lhs as usize) == Some(Kind::Rational)
                            || plan.kinds.get(rhs as usize) == Some(Kind::Rational)) =>
                {
                    note(HostFn::RationalBitwise, &mut used);
                    for r in [lhs, rhs] {
                        match plan.kinds.get(r as usize) {
                            Some(Kind::Int) => note(HostFn::RationalFromI64, &mut used),
                            Some(Kind::BigInt) => note(HostFn::RationalFromBigint, &mut used),
                            _ => {}
                        }
                    }
                }
                // A real beside a rational (linker): the rational's double.
                Op::Add { dst, lhs, rhs }
                | Op::Sub { dst, lhs, rhs }
                | Op::Mul { dst, lhs, rhs }
                | Op::Div { dst, lhs, rhs }
                    if linked
                        && plan.kinds.get(dst as usize) == Some(Kind::Float)
                        && (plan.kinds.get(lhs as usize) == Some(Kind::Rational)
                            || plan.kinds.get(rhs as usize) == Some(Kind::Rational)) =>
                {
                    note(HostFn::RationalToF64, &mut used);
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
                Op::CallBuiltin {
                    builtin:
                        b @ (BuiltinId::Floor | BuiltinId::Ceil | BuiltinId::Round | BuiltinId::Abs),
                    args_start,
                    ..
                } if linked && plan.kinds.get(args_start as usize) == Some(Kind::Rational) => {
                    note(
                        match b {
                            BuiltinId::Floor => HostFn::RationalFloor,
                            BuiltinId::Ceil => HostFn::RationalCeil,
                            BuiltinId::Round => HostFn::RationalRound,
                            _ => HostFn::RationalAbs,
                        },
                        &mut used,
                    );
                }
                // The `Uuid`-producing / -reading builtins (linker): each maps to its `logos_rt_uuid_*` sink.
                Op::CallBuiltin {
                    builtin:
                        b @ (BuiltinId::Uuid
                        | BuiltinId::UuidNil
                        | BuiltinId::UuidMax
                        | BuiltinId::UuidDns
                        | BuiltinId::UuidUrl
                        | BuiltinId::UuidOid
                        | BuiltinId::UuidX500
                        | BuiltinId::UuidVersion),
                    ..
                } if linked => {
                    note(
                        match b {
                            BuiltinId::Uuid => HostFn::UuidParse,
                            BuiltinId::UuidNil => HostFn::UuidNil,
                            BuiltinId::UuidMax => HostFn::UuidMax,
                            BuiltinId::UuidDns => HostFn::UuidDns,
                            BuiltinId::UuidUrl => HostFn::UuidUrl,
                            BuiltinId::UuidOid => HostFn::UuidOid,
                            BuiltinId::UuidX500 => HostFn::UuidX500,
                            _ => HostFn::UuidVersion,
                        },
                        &mut used,
                    );
                }
                // `uuid_from_bytes(seq)` boxes a Uuid from a packed 16-byte block via the runtime.
                Op::CallBuiltin {
                    builtin: BuiltinId::UuidFromBytes,
                    ..
                } if linked => note(HostFn::UuidFromPtr, &mut used),
                // The four SHA-1 SHA-NI ops call the `logos_rt_sha1*` runtime (base::sha_ops spec).
                Op::CallBuiltin {
                    builtin:
                        b @ (BuiltinId::Sha1Rnds4
                        | BuiltinId::Sha1Msg1
                        | BuiltinId::Sha1Msg2
                        | BuiltinId::Sha1Nexte),
                    ..
                } if linked => {
                    note(
                        match b {
                            BuiltinId::Sha1Rnds4 => HostFn::Sha1Rnds4,
                            BuiltinId::Sha1Msg1 => HostFn::Sha1Msg1,
                            BuiltinId::Sha1Msg2 => HostFn::Sha1Msg2,
                            _ => HostFn::Sha1Nexte,
                        },
                        &mut used,
                    );
                }
                // Lane-wise `Lanes + Lanes` / `lanes += lanes` → the `logos_rt_lanes4_add` runtime.
                Op::Add { lhs, .. }
                    if linked && plan.kinds.get(lhs as usize) == Some(Kind::Lanes) =>
                {
                    note(HostFn::Lanes4Add, &mut used)
                }
                Op::AddAssign { dst, .. }
                    if linked && plan.kinds.get(dst as usize) == Some(Kind::Lanes) =>
                {
                    note(HostFn::Lanes4Add, &mut used)
                }
                // Lane-wise `Lanes xor Lanes` → the `logos_rt_lanes4_xor` runtime.
                Op::BitXor { lhs, .. }
                    if linked && plan.kinds.get(lhs as usize) == Some(Kind::Lanes) =>
                {
                    note(HostFn::Lanes4Xor, &mut used)
                }
                // `Moment/Date ± Span` calendar arithmetic (linker) — the base's width picks the sink.
                Op::Add { lhs, rhs, .. } | Op::Sub { lhs, rhs, .. }
                    if linked
                        && (plan.kinds.get(lhs as usize) == Some(Kind::Span)
                            || plan.kinds.get(rhs as usize) == Some(Kind::Span)) =>
                {
                    let base = if plan.kinds.get(lhs as usize) == Some(Kind::Span) {
                        rhs
                    } else {
                        lhs
                    };
                    note(
                        if plan.kinds.get(base as usize) == Some(Kind::Date) {
                            HostFn::DateAddSpan
                        } else {
                            HostFn::MomentAddSpan
                        },
                        &mut used,
                    );
                }
                // `uuid == uuid` (equality on two Uuid operands, linker) — the 16-byte `logos_rt_uuid_eq`.
                Op::Eq { lhs, rhs, .. } | Op::NotEq { lhs, rhs, .. }
                    if linked
                        && plan.kinds.get(lhs as usize) == Some(Kind::Uuid)
                        && plan.kinds.get(rhs as usize) == Some(Kind::Uuid) =>
                {
                    note(HostFn::UuidEq, &mut used);
                }
                Op::LoadToday { .. } => note(HostFn::Today, &mut used),
                Op::LoadNow { .. } => note(HostFn::Now, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::RandomInt,
                    ..
                } => note(HostFn::RandomInt, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::EnvHas,
                    ..
                } => note(HostFn::EnvLen, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::EnvValue,
                    ..
                } => {
                    note(HostFn::EnvLen, &mut used);
                    note(HostFn::EnvInto, &mut used);
                }
                Op::CallBuiltin {
                    builtin: BuiltinId::TimeNow,
                    ..
                } => note(HostFn::TimeNow, &mut used),
                Op::Args { .. } => note(HostFn::Args, &mut used),
                // Every runtime failure hands its message to the host before trapping, so any
                // program that CAN fail declares the import. `FailWith` is reachable from
                // ordinary code (a bad index, a failed `Check`), so this is most programs.
                Op::FailWith { .. } => note(HostFn::LogosRtFail, &mut used),
                // `Fail with …` / a contract's reason: the same host call, and a non-Text value is
                // stringified by its formatter first.
                Op::CallBuiltin {
                    builtin: BuiltinId::Fail | BuiltinId::FailReason,
                    args_start,
                    ..
                } => {
                    note(HostFn::LogosRtFail, &mut used);
                    note_operand_fmt(plan, args_start, &mut used);
                }
                // A page has no file system: mounting a journal or writing a file fails with the
                // sentence that says so, through the same host call.
                Op::CallBuiltin {
                    builtin: BuiltinId::VfsMount | BuiltinId::VfsWriteFile,
                    ..
                } => note(HostFn::LogosRtFail, &mut used),
                // An indexing op emits an inline bounds check, and that check now reports WHICH
                // index before it traps. Over-approximating here is free: an unused import is
                // a name in the table, while a missing one is a module that cannot instantiate.
                Op::Div { .. }
                | Op::Mod { .. }
                | Op::Index { .. }
                | Op::IndexUnchecked { .. }
                | Op::SetIndex { .. }
                | Op::SetIndexUnchecked { .. } => note(HostFn::LogosRtFailAt, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::ParseInt,
                    ..
                } => note(HostFn::ParseInt, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::Contains,
                    ..
                } => note(HostFn::TextContains, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::DomSetAttribute,
                    args_start,
                    ..
                } => {
                    if plan.kinds.get(args_start as usize + 2) == Some(Kind::Bool) {
                        note(HostFn::DomSetBoolAttribute, &mut used)
                    } else {
                        note(HostFn::DomSetAttribute, &mut used)
                    }
                }
                Op::CallBuiltin { builtin, .. } if dom_import_of(builtin).is_some() => note(
                    dom_import_of(builtin).expect("guarded by the arm"),
                    &mut used,
                ),
                Op::CallBuiltin {
                    builtin: BuiltinId::ParseFloat,
                    ..
                } => note(HostFn::ParseFloat, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::ReadReals,
                    ..
                } => note(HostFn::ReadReals, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::ParseTimestamp,
                    ..
                } => note(HostFn::ParseTimestamp, &mut used),
                // `writeWireResidual(text)` frames the Text (`[len:u32][bytes]`) out to the host sink.
                Op::CallBuiltin {
                    builtin: BuiltinId::WriteWireResidual,
                    ..
                } => note(HostFn::WriteWireResidual, &mut used),
                // LINKER-mode extended temporal (calendar logic in `base::temporal`): `format_timestamp`
                // → a `Text` handle, `months_between`/`years_between` → an `Int`.
                Op::CallBuiltin {
                    builtin: BuiltinId::FormatTimestamp,
                    ..
                } if linked => note(HostFn::FormatTimestampRt, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::MonthsBetween,
                    ..
                } if linked => note(HostFn::MonthsBetweenRt, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::YearsBetween,
                    ..
                } if linked => note(HostFn::YearsBetweenRt, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::MakeDate,
                    ..
                } if linked => note(HostFn::MakeDateRt, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::MakeMoment,
                    ..
                } if linked => note(HostFn::MakeMomentRt, &mut used),
                // `in_zone(m, "zone")` → a `Text` (local wall-clock), `local_instant(m, "zone")` → a `Moment`.
                Op::CallBuiltin {
                    builtin: BuiltinId::InZone,
                    ..
                } if linked => note(HostFn::InZoneRt, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::LocalInstant,
                    ..
                } if linked => note(HostFn::LocalInstantRt, &mut used),
                // The general SIMD lane vocabulary (`base::LanesVal`) — the SSE byte/word-lane ops a Logos
                // codec compiles to. Each notes its `logos_rt_lanes_*` runtime fn (linker mode only).
                Op::CallBuiltin { builtin: b, .. } if linked && lanes_v_host_fn(b).is_some() => {
                    note(lanes_v_host_fn(b).unwrap(), &mut used)
                }
                // Money FX (linker): `to_currency`→convert; `set_rate` installs one rate (coercing the
                // rate arg Int→Rational / Decimal→Rational); `set_rates` installs a whole Map (the runtime
                // reads it — all three value-kind variants are noted since the map's value kind is resolved
                // at lowering; an imported-but-uncalled runtime fn is harmless, all are defined).
                Op::CallBuiltin {
                    builtin: BuiltinId::ToCurrency,
                    ..
                } if linked => note(HostFn::MoneyToCurrency, &mut used),
                Op::CallBuiltin {
                    builtin: BuiltinId::SetRate,
                    args_start,
                    ..
                } if linked => {
                    note(HostFn::MoneySetRate, &mut used);
                    match plan.kinds.get((args_start + 1) as usize) {
                        Some(Kind::Int) => note(HostFn::RationalFromI64, &mut used),
                        Some(Kind::Decimal) => note(HostFn::DecimalToRational, &mut used),
                        _ => {}
                    }
                }
                Op::CallBuiltin {
                    builtin: BuiltinId::SetRates,
                    ..
                } if linked => {
                    note(HostFn::MoneySetRatesInt, &mut used);
                    note(HostFn::MoneySetRatesRational, &mut used);
                    note(HostFn::MoneySetRatesDecimal, &mut used);
                }
                // `wireBytes(value)` — marshal the value via the REAL codec (by the arg's kind).
                Op::CallBuiltin {
                    builtin: BuiltinId::WireBytes,
                    args_start,
                    ..
                } if linked => {
                    if let Some(h) = wire_bytes_host_fn(plan.kinds.get(args_start as usize)) {
                        note(h, &mut used);
                    }
                }
                // `readWireProgram()` reads a host frame then DECODES it to a dynamic value; `run_accepted`
                // sandbox-evals a wire-received shipped function through the acceptance contract.
                Op::CallBuiltin {
                    builtin: BuiltinId::ReadWireProgram,
                    ..
                } if linked => {
                    note(HostFn::ReadWireFrame, &mut used);
                    note(HostFn::ReadWireProgramRt, &mut used);
                    // `readWireProgram` bump-allocs a receive buffer via `emit_alloc`, which in LINKER mode
                    // calls `logos_rt_alloc` — it MUST be imported, else `emit_alloc` falls back to the
                    // `__heap_ptr` global (undeclared in a linked module → an invalid global relocation).
                    note(HostFn::RtAlloc, &mut used);
                }
                Op::CallBuiltin {
                    builtin: BuiltinId::RunAccepted,
                    ..
                } if linked => note(HostFn::RunAccepted, &mut used),
                Op::CallBuiltin {
                    builtin:
                        BuiltinId::YearOf
                        | BuiltinId::MonthOf
                        | BuiltinId::DayOf
                        | BuiltinId::WeekdayOf
                        | BuiltinId::HourOf
                        | BuiltinId::MinuteOf
                        | BuiltinId::SecondOf
                        | BuiltinId::WeekOf
                        | BuiltinId::QuarterOf,
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
                Op::CallBuiltin {
                    builtin: BuiltinId::Format,
                    args_start,
                    arg_count,
                    ..
                } if arg_count > 0 => {
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
                // via `fmt_align_into`; a NO-spec debug piece (`"{x=}"`) just stringifies the value.
                // The lowering re-derives the exact spec; here we mirror EXACTLY which hosts each case
                // reaches, so no import is declared that the body never calls.
                Op::FormatValue { src, spec, .. } => {
                    let spec_s = match spec {
                        u32::MAX => None,
                        idx => match program.constants.get(idx as usize) {
                            Some(Constant::Text(s)) => Some(s.as_str()),
                            _ => None,
                        },
                    };
                    match spec_s {
                        Some(s) if s.starts_with('.') => note(HostFn::FmtF64PrecInto, &mut used),
                        Some(_) => {
                            note(HostFn::FmtAlignInto, &mut used);
                            note_fmt_kind(plan.kinds.get(src as usize), &mut used);
                        }
                        None => note_fmt_kind(plan.kinds.get(src as usize), &mut used),
                    }
                }
                // Observation (Wave 13, OBSERVED profile) — one host import; the
                // host resolves the site's category/message/label.
                Op::Obs { .. } => note(HostFn::Obs, &mut used),
                _ => {}
            }
        }
    }
    // A Text the HOST hands over — an element's text, a response body, the route, and every Text
    // argument it passes when it enters the page by name (an event and its value to the node
    // dispatcher, a failure's message to the failure handler) — is written into the module's heap
    // through `__logos_text_new`. A module that takes one has a heap and carries that allocator, even
    // when it builds no heap value of its own.
    for k in shown_shapes.leaf_kinds(&program.struct_types, &program.enum_types) {
        note_fmt_kind(Some(k), &mut used);
    }
    {
        let plans: Vec<&Plan> = std::iter::once(&main).chain(fns.iter()).collect();
        exact::note_imports(&plans, &mut |h| note(h, &mut used));
        if presence_enabled {
            note(HostFn::PrintNothing, &mut used);
        }
        if exact_sig.sequence_ints() {
            note(HostFn::ExactText, &mut used);
            note(HostFn::PrintText, &mut used);
        }
    }
    let host_entered: Vec<usize> = program
        .functions
        .iter()
        .enumerate()
        .filter(|(_, f)| entry_names.iter().any(|n| interner.resolve(f.name) == n))
        .map(|(at, _)| at)
        .collect();
    let takes_host_text = used.iter().any(|h| h.returns_host_text())
        || host_entered.iter().any(|&at| {
            let plan = &fns[at];
            (0..plan.num_params as usize).any(|r| plan.kinds.get(r) == Some(Kind::Text))
        });
    // A sequence the host hands over is built the same way, through `__logos_seq_new`.
    let takes_host_seq = used.iter().any(|h| h.returns_host_seq());
    let uses_heap = uses_heap || takes_host_text || takes_host_seq;
    // LINKER MODE with an emitter heap or an iterator stack imports the runtime allocator, to seed each
    // one's slab at the `main` prologue.
    if linked && (uses_heap || uses_iter) {
        note(HostFn::RtAlloc, &mut used);
    }
    // A self-contained heap grows `memory` on demand and reports failure the same way any other
    // inline check does — a program need not divide or index anything to exhaust its heap, so this
    // import cannot wait to be noticed by a specific op the way the others above are.
    if uses_heap && !linked {
        note(HostFn::LogosRtFailAt, &mut used);
    }
    // Re-sort into the canonical HOST_FNS order so indices are deterministic.
    let imports: Vec<HostFn> = HOST_FNS
        .iter()
        .copied()
        .filter(|h| used.contains(h))
        .collect();
    let host_index =
        |h: HostFn| -> Option<u32> { imports.iter().position(|x| *x == h).map(|i| i as u32) };
    let num_imports = imports.len() as u32;
    let main_index = num_imports;
    let fn_base = num_imports + 1; // wasm index of program.functions[0]

    // ---- 3. Type table: host functions, then Main, then each function ----
    let mut types = TypeTable::default();
    let host_type: Vec<u32> = imports
        .iter()
        .map(|h| types.intern(h.params(), h.results()))
        .collect();
    let main_type = types.intern(vec![], result_valtypes(main.result));
    let mut fn_types: Vec<u32> = Vec::with_capacity(fns.len());
    let fn_stubbed: Vec<bool> = fns.iter().map(|p| p.stub).collect();
    let mut fn_param_valtypes: Vec<Vec<u8>> = Vec::with_capacity(fns.len());
    let mut fn_param_kinds: Vec<Vec<Option<Kind>>> = Vec::with_capacity(fns.len());
    for p in &fns {
        let params: Vec<u8> = (0..p.num_params)
            .map(|r| p.kinds.valtype(r as usize))
            .collect();
        fn_types.push(types.intern(params.clone(), plan_results(p)));
        fn_param_valtypes.push(params);
        fn_param_kinds.push((0..p.num_params).map(|r| p.kinds.get(r as usize)).collect());
    }
    // The exported closure trampoline (`__logos_call_closure(handle:i32, arg:i64)->i64`)
    // shares the funcref table it `call_indirect`s through, so it exists only in a
    // self-contained module that has closures. Its type is interned here (before the
    // type section is encoded); its body + index are built after the function bodies.
    let emit_trampoline = has_closures && !linked;
    let trampoline_type = emit_trampoline.then(|| types.intern(vec![I32, I64], vec![I64]));
    // The reader the linked value runtime calls back for a closure the program defines a real number by
    // (`logos_program_defines(body: i32, n: i64) -> i32`), the module's last function.
    let emit_definer = linked && std::iter::once(&main).chain(fns.iter()).any(tower::defines_reals);
    let definer_type = emit_definer.then(|| types.intern(vec![I32, I64], vec![I32]));
    // The display functions follow the trampoline: `(handle: i32) -> i32 Text`, one per shape.
    let mut shown_shapes = shown_shapes;
    shown_shapes.place(fn_base + fns.len() as u32 + u32::from(emit_trampoline));
    let shown_type = (shown_shapes.len() > 0).then(|| types.intern(vec![I32], vec![I32]));
    // The runtime functions follow the trampoline in the index space, and their indices have to be
    // known BEFORE any body is lowered because every allocation site calls them.
    let emit_heap_fns = uses_heap && !linked;
    // A page is interactive when its host RE-ENTERS it after `main`: through a callback it handed
    // the host, or through a function the host enters by name (an element's event arrives at the
    // node dispatcher, a timer's tick at `__lgx_tick`).
    let interactive = emit_heap_fns
        && (imports.iter().any(|h| h.registers_callback()) || !host_entered.is_empty());
    // S8's true counts, turn list and safepoint on every interactive page; the census only in a
    // check-mode module (`Reclaim::Checked`). See `Runtime::layout`.
    let runtime = emit_heap_fns.then(|| {
        Runtime::layout(
            num_imports + 1 + fns.len() as u32 + u32::from(emit_trampoline) + shown_shapes.len(),
            interactive,
            takes_host_text,
            takes_host_seq,
            uses_iter,
            exact::on(&main),
            reclaim,
            &mut types,
        )
    });

    // ---- 4. Emit each function body, with the call/host context resolved ----
    let heap_global = num_globals as u32; // `__heap_ptr` follows the user globals
    let iter_global = heap_global + u32::from(uses_heap); // `__iter_sp` follows `__heap_ptr`
                                                          // The S8 turn-retained list's three globals (`data_ptr`, `len`, `cap`) follow `__iter_sp` —
                                                          // present exactly when `interactive` AND `reclaim` (batch modules never run a safepoint, so
                                                          // nothing would ever drain the list; see `Runtime::layout`).
    let retain_global = iter_global + u32::from(uses_iter);
    let retain_base = (emit_heap_fns && reclaim != Reclaim::Off).then_some(retain_global);
    let uses_counters = std::iter::once(&main).chain(fns.iter()).any(|p| {
        p.ops
            .iter()
            .any(|op| matches!(op, Op::NewCrdt { kind: 4 | 5, .. }))
    });
    let replica_global =
        uses_counters.then_some(retain_global + 3 * u32::from(retain_base.is_some()));
    // Live registration list and monotonic identities, independent of reusable heap addresses.
    let callback_base = (retain_base.is_some() && has_closures)
        .then_some(retain_global + 3 * u32::from(retain_base.is_some()) + u32::from(uses_counters));
    let fn_results: Vec<Option<Kind>> = fns.iter().map(|p| p.result).collect();
    let fn_inout: Vec<Vec<u16>> = fns.iter().map(|p| p.inout.clone()).collect();
    // `capture_kinds` (computed in the planning pass, global + local) is the same source the closure
    // body signatures were seeded from, so `MakeClosure`'s store / `CallValue`'s load / the signature
    // all agree on each capture's valtype.
    let rt_alloc = if linked && uses_heap {
        host_index(HostFn::RtAlloc)
    } else {
        None
    };
    let global_kinds: Vec<Option<Kind>> = global_kinds_cell.borrow().clone();
    let global_valtypes: Vec<u8> = global_kinds
        .iter()
        .map(|gk| gk.map(Kind::wasm_valtype).unwrap_or(I64))
        .collect();
    let text_pool_cell = std::cell::RefCell::new(TextPool::default());
    let text_pool = (!linked).then_some(&text_pool_cell);
    let shared_maps = std::iter::once(&main).chain(fns.iter()).any(|p| {
        p.ops
            .iter()
            .any(|op| matches!(op, Op::NewCrdt { kind: 6, .. }))
    });
    let absence_base = presence_enabled.then_some(
        num_globals as u32
            + u32::from(uses_heap)
            + u32::from(uses_iter)
            + 3 * u32::from(retain_base.is_some())
            + u32::from(uses_counters)
            + 2 * u32::from(callback_base.is_some()),
    );
    let may_safepoint = may_safepoint(&fns);
    let ctx = Ctx {
        absence_base,
        constants: &program.constants,
        global_valtypes: &global_valtypes,
        global_kinds: &global_kinds,
        runtime: runtime.as_ref(),
        shared_maps,
        text_pool,
        host_index: &host_index,
        fn_base,
        heap_global,
        iter_global,
        replica_global,
        uses_iter,
        fn_type: &fn_types,
        fn_param_valtypes: &fn_param_valtypes,
        fn_param_kinds: &fn_param_kinds,
        fn_stubbed: &fn_stubbed,
        fn_results: &fn_results,
        fn_inout: &fn_inout,
        functions: &program.functions,
        capture_kinds: &capture_kinds,
        enum_types: &program.enum_types,
        struct_types: &program.struct_types,
        policies,
        interner,
        linked,
        rt_alloc,
        retain_base,
        shown: &shown_shapes,
        exact: &exact_sig,
        hot: std::cell::Cell::new(false),
        batch_reclaim: !interactive && retain_base.is_some(),
        may_safepoint: &may_safepoint,
    };
    let main_body =
        emit_body(&main, &ctx).map_err(|e| within_function(e, program, interner, None))?;
    let mut fn_bodies = Vec::with_capacity(fns.len());
    for (fi, p) in fns.iter().enumerate() {
        fn_bodies
            .push(emit_body(p, &ctx).map_err(|e| within_function(e, program, interner, Some(fi)))?);
    }
    let mut shown_bodies = Vec::with_capacity(shown_shapes.len() as usize);
    for shape in shown_shapes.shapes() {
        shown_bodies.push(display::emit_display_fn(shape, &ctx)?);
    }
    // Build the trampoline body + its wasm function index (after all user functions).
    let trampoline_body = emit_trampoline
        .then(|| emit_closure_trampoline(&fns, program, &fn_types, &ctx, callback_base))
        .transpose()?;
    let trampoline_index = fn_base + fns.len() as u32;
    let definer_body = emit_definer.then(|| tower::emit_defining_reader(&fns, &ctx)).transpose()?;

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

    // Function section (id 3): Main + each user function (type indices) + the trampoline + the
    // runtime functions.
    let runtime_count = runtime.as_ref().map_or(0, |r| r.fns.len() as u32);
    let mut func = Vec::new();
    leb_u32(
        &mut func,
        1 + fns.len() as u32
            + u32::from(emit_trampoline)
            + shown_shapes.len()
            + runtime_count
            + u32::from(emit_definer),
    );
    leb_u32(&mut func, main_type);
    for t in &fn_types {
        leb_u32(&mut func, *t);
    }
    if let Some(tt) = trampoline_type {
        leb_u32(&mut func, tt);
    }
    if let Some(st) = shown_type {
        for _ in 0..shown_shapes.len() {
            leb_u32(&mut func, st);
        }
    }
    if let Some(r) = &runtime {
        for t in &r.types {
            leb_u32(&mut func, *t);
        }
    }
    if let Some(dt) = definer_type {
        leb_u32(&mut func, dt);
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

    // The dynamic heap starts above the interned text. The pool is filled while bodies are lowered,
    // which is finished by now, so this is the first moment the static region's size is known.
    let heap_base = HEAP_START + text_pool_cell.borrow().image.len() as i32;

    // Global section (id 6): one mutable global per promoted Main binding (zero-initialized),
    // plus the bump-allocator pointer `__heap_ptr` when the program uses the heap.
    if num_globals > 0 || uses_heap || uses_iter || retain_base.is_some() || uses_counters {
        let mut glob = Vec::new();
        leb_u32(
            &mut glob,
            num_globals as u32
                + u32::from(uses_heap)
                + u32::from(uses_iter)
                + 3 * u32::from(retain_base.is_some())
                + u32::from(uses_counters)
                + 2 * u32::from(callback_base.is_some())
                + if presence_enabled {
                    num_globals as u32
                } else {
                    0
                },
        );
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
            i32_const(&mut glob, if linked { 0 } else { heap_base });
            glob.push(0x0B);
        }
        if uses_iter {
            // `__iter_sp`: mutable i32, the top `Repeat` frame (a heap block, see `FRAME_PREV`);
            // 0 while no loop runs.
            glob.push(I32);
            glob.push(0x01); // mutable
            i32_const(&mut glob, 0);
            glob.push(0x0B);
        }
        if retain_base.is_some() {
            // The S8 turn-retained list: `data_ptr`/`len`/`cap`, all i32, all zero (empty) at
            // start-up — grown like every other buffer in this backend on first use.
            for _ in 0..3 {
                glob.push(I32);
                glob.push(0x01); // mutable
                i32_const(&mut glob, 0);
                glob.push(0x0B);
            }
        }
        if uses_counters {
            glob.push(I64);
            glob.push(0x01);
            i64c(&mut glob, 0);
            glob.push(0x0B);
        }
        if callback_base.is_some() {
            for _ in 0..2 {
                glob.push(I32);
                glob.push(0x01);
                i32_const(&mut glob, 0);
                glob.push(0x0B);
            }
        }
        if presence_enabled {
            for _ in 0..num_globals {
                glob.extend([I32, 0x01]);
                i32_const(&mut glob, 1);
                glob.push(0x0b);
            }
        }
        section(&mut module, 6, &glob);
    }

    // Export section (id 7): the synthesized top-level body as `main`, plus `memory` if this module
    // DEFINES one. In linker mode the memory is imported (the linker re-exports it), so we don't.
    let export_memory = uses_heap && !linked;
    // Export the user's `render` function as `__logos_render` so the browser host
    // can re-invoke the view after a reactive signal changes (immediate-mode
    // re-render). It is an ordinary compiled function at `fn_base + its position`;
    // we only give it a stable export name.
    let render_index = program
        .functions
        .iter()
        .position(|f| interner.resolve(f.name) == "render")
        .map(|p| fn_base + p as u32);
    // `__heap_ptr` is exported whenever the module has a heap, so a host can read how much of the
    // address space the page has claimed; an export widens nothing a host must implement.
    // Every function a host enters by name (`HostEntries::names`, from `lgx_lower::LGX_ENTRIES`) that
    // the program defines, under its own name — the page entries always, and each component the host
    // may mount on demand. A shipped page mounts none of its own.
    let host_entries: Vec<(String, u32)> = entries
        .names()
        .into_iter()
        .filter_map(|name| {
            let at = program
                .functions
                .iter()
                .position(|f| interner.resolve(f.name) == name)?;
            Some((name, fn_base + at as u32))
        })
        .collect();
    const EXPORT_FUNC: u8 = 0x00;
    const EXPORT_MEMORY: u8 = 0x02;
    const EXPORT_GLOBAL: u8 = 0x03;
    let mut exports: Vec<(&str, u8, u32)> = vec![("main", EXPORT_FUNC, main_index)];
    if export_memory {
        exports.push(("memory", EXPORT_MEMORY, 0));
    }
    if emit_trampoline {
        exports.push(("__logos_call_closure", EXPORT_FUNC, trampoline_index));
    }
    if let Some(ri) = render_index {
        exports.push(("__logos_render", EXPORT_FUNC, ri));
    }
    for (name, index) in &host_entries {
        exports.push((name.as_str(), EXPORT_FUNC, *index));
    }
    if uses_heap && !linked {
        exports.push(("__heap_ptr", EXPORT_GLOBAL, heap_global));
    }
    if let Some(r) = &runtime {
        for (i, f) in r.fns.iter().enumerate() {
            if let Some(name) = f.export() {
                exports.push((name, EXPORT_FUNC, r.first + i as u32));
            }
        }
    }
    if emit_definer {
        let definer_index =
            fn_base + fns.len() as u32 + u32::from(emit_trampoline) + shown_shapes.len() + runtime_count;
        exports.push((tower::DEFINER_SYMBOL, EXPORT_FUNC, definer_index));
    }
    let mut export = Vec::new();
    leb_u32(&mut export, exports.len() as u32);
    for (name, kind, index) in &exports {
        encode_name(&mut export, name);
        export.push(*kind);
        leb_u32(&mut export, *index);
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

    // Code section (id 10): Main, then each function (each prefixed by its byte length),
    // then the trampoline, then the runtime functions.
    let mut code = Vec::new();
    leb_u32(
        &mut code,
        1 + fn_bodies.len() as u32
            + u32::from(emit_trampoline)
            + shown_shapes.len()
            + runtime_count
            + u32::from(emit_definer),
    );
    for entry in std::iter::once(&main_body).chain(fn_bodies.iter()) {
        leb_u32(&mut code, entry.len() as u32);
        code.extend_from_slice(entry);
    }
    if let Some(tb) = &trampoline_body {
        leb_u32(&mut code, tb.len() as u32);
        code.extend_from_slice(tb);
    }
    for body in &shown_bodies {
        leb_u32(&mut code, body.len() as u32);
        code.extend_from_slice(body);
    }
    if let Some(r) = &runtime {
        let env = RuntimeEnv {
            heap_global,
            iter_global,
            heap_base,
            fail_at: host_index(HostFn::LogosRtFailAt)
                .expect("a self-contained heap always imports logos_rt_fail_at"),
            exact_drop: host_index(HostFn::ExactDrop),
            exact_clone: host_index(HostFn::ExactClone),
            exact_cmp: host_index(HostFn::ExactCmp),
            exact_host: (|| {
                Some([
                    host_index(HostFn::ExactBox)?,
                    host_index(HostFn::ExactAdd)?,
                    host_index(HostFn::ExactSub)?,
                    host_index(HostFn::ExactMul)?,
                    host_index(HostFn::ExactNarrow)?,
                    host_index(HostFn::ExactClone)?,
                ])
            })(),
            retain_base,
            callback_base,
            global_kinds: global_kinds.clone(),
            shared_maps,
            runtime: r,
        };
        for body in r.bodies(&env) {
            leb_u32(&mut code, body.len() as u32);
            code.extend_from_slice(&body);
        }
    }
    if let Some(db) = &definer_body {
        leb_u32(&mut code, db.len() as u32);
        code.extend_from_slice(db);
    }
    section(&mut module, 10, &code);

    // Data section (id 11): the interned text literals, one active segment at `STATIC_BASE`.
    // Emitted after the code section, which is where the spec puts it and where the pool is full.
    {
        let image = text_pool_cell.borrow();
        if !image.image.is_empty() {
            let mut data = Vec::new();
            leb_u32(&mut data, 1); // one segment
            leb_u32(&mut data, 0); // active, memory 0
            i32_const(&mut data, STATIC_BASE);
            data.push(0x0B); // end of the offset expression
            leb_u32(&mut data, image.image.len() as u32);
            data.extend_from_slice(&image.image);
            section(&mut module, 11, &data);
        }
    }

    Ok(module)
}

/// The declared result kind of `program.functions[fi]` (`None` for void / undeclared) — used
/// to type `Op::Call` results before the callee's own body is inferred.
fn declared_result(program: &CompiledProgram, fi: usize) -> Option<Kind> {
    let f = program.functions.get(fi)?;
    f.return_type
        .as_ref()
        .and_then(kind::boundary_to_kind)
        .or_else(|| f.ret_kind.map(Kind::from_slot))
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
                && !closure_ranges.iter().any(|&(e2, t2)| {
                    (e2, t2) != (e, t) && e2 <= e && t <= t2 && e2 >= start && t2 <= end
                })
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
            Err(WasmLowerError::Unsupported(
                "jump target escapes the function",
            ))
        }
    };
    let mut ops = Vec::with_capacity(kept.len());
    for &pc in &kept {
        ops.push(match code[pc] {
            Op::Jump { target } => Op::Jump {
                target: rebase(target)?,
            },
            Op::JumpIfFalse { cond, target } => Op::JumpIfFalse {
                cond,
                target: rebase(target)?,
            },
            Op::JumpIfTrue { cond, target } => Op::JumpIfTrue {
                cond,
                target: rebase(target)?,
            },
            Op::IterNext { dst, exit } => Op::IterNext {
                dst,
                exit: rebase(exit)?,
            },
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
    global_layout: &dyn Fn(u16) -> Option<FieldLayout>,
    global_declared: &dyn Fn(u16) -> Option<BoundaryType>,
    linked: bool,
) -> R<Plan> {
    let holes = child_holes(&layout.closure_ranges, 0, layout.main_end);
    let ops = extract_region(&program.code, 0, layout.main_end, &holes)?;
    let num_regs = program.register_count as u32;
    // Split any register the VM reused across disjoint live ranges of conflicting wasm types into one
    // local per range (Main has no parameters to pin). Identity unless a real conflict exists.
    let (mut ops, num_regs, placed) = regsplit::split_registers(&ops, num_regs, 0, &program.functions);
    let fn_return_types = fn_return_types(program);
    // Main has no parameters, but its values have DECLARED types — an empty `a new Seq of Todo`
    // has no element to trace, and a loop over it must still resolve a field. Fed through the same
    // seed channels a parameter uses, because it is the same fact.
    let mut main_kind_seeds: Vec<Option<Kind>> = vec![None; num_regs as usize];
    let mut main_seeds: Vec<(u16, ParamShape)> = Vec::new();
    let main_declared = seed_declarations(
        program,
        &ops,
        &region_declarations(program, 0, layout.main_end, &holes, &placed),
        &mut main_kind_seeds,
        &mut main_seeds,
    );
    let (kinds, _pc_reach, reachable_blocks) = infer_with_reachability(
        &ops,
        &program.constants,
        &program.struct_types,
        &program.enum_types,
        &fn_return_types,
        num_regs as usize,
        &main_kind_seeds,
        ret_of,
        global_of,
        closure_ret,
        ret_layout,
        fn_return_closure,
        &main_seeds,
        &[],
        global_layout,
        global_declared,
        linked,
        &program.functions,
        &[],
    )?;
    let structs = kind::struct_layout(
        &ops,
        &program.constants,
        &program.struct_types,
        &program.enum_types,
        &fn_return_types,
        ret_layout,
        fn_return_closure,
        &main_seeds,
        &[],
        global_layout,
        global_declared,
        &main_declared,
    );
    let cow_inserts = cow_struct_inserts(&ops, &kinds, num_regs, &program.functions);
    let reg_shape = complete_reg_shape(&structs, &kinds, program);
    let iter_iterable = kind::iter_pairing(&ops);
    let place = PlaceSites::of(&ops, &program.constants, &program.functions);
    let transient = transient_reads(&ops, num_regs, &program.functions);
    let declared = declared_registers(&structs, main_declared, &[]);
    let range_loops = range_loop_registers(&ops, &program.functions);
    if linked {
        accumulate_as_add(&mut ops, &kinds);
    }
    let mut plan = Plan {
        range_loops,
        ops,
        kinds,
        num_params: 0,
        num_regs,
        result: None,
        reachable_blocks,
        structs,
        cow_inserts,
        reg_shape,
        return_closure: None,
        iter_iterable,
        place,
        transient,
        declared,
        inout: Vec::new(),
        exact: Vec::new(),
        params_tagged: Vec::new(),
        ret_tagged: false,
        absence: false,
        absence_regs: Vec::new(),
        absence_globals: Vec::new(),
        tower: tower::Towers::default(),
        stub: false,
    };
    tower::settle(&mut plan, program, linked)?;
    Ok(plan)
}

/// Linker mode: `Set x to x + y` on a number the runtime holds — a rational, a big integer, a complex number, a
/// residue, a decimal, money, a quantity — is the sum an `Add` computes; the bytecode's in-place `AddAssign` names
/// the same value, so it lowers as that `Add`, with that `Add`'s imports.
fn accumulate_as_add(ops: &mut [Op], kinds: &KindTable) {
    for op in ops.iter_mut() {
        if let Op::AddAssign { dst, src } = *op {
            if matches!(
                kinds.get(dst as usize),
                Some(
                    Kind::Rational
                        | Kind::BigInt
                        | Kind::Complex
                        | Kind::Modular
                        | Kind::Decimal
                        | Kind::Money
                        | Kind::Quantity
                )
            ) {
                *op = Op::Add {
                    dst,
                    lhs: dst,
                    rhs: src,
                };
            }
        }
    }
}

/// Each function's declared RETURN type, indexed by function index — for typing a caller's inline use
/// of a returned composite (`item k of f()`, `Inspect f()`).
fn fn_return_types(program: &CompiledProgram) -> Vec<Option<BoundaryType>> {
    program
        .functions
        .iter()
        .map(|f| f.return_type.clone())
        .collect()
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
                // No declared layout here, and none needed: a map BUILT in this region resolves a
                // field on its values through the register that wrote them (`map_value`), which is
                // the path that always worked. The layout slot exists for the two cases that have
                // no such register — a map PARAMETER and a map GLOBAL.
                let key = structs
                    .map_set_key
                    .get(reg)
                    .and_then(|k| kinds.get(*k as usize));
                out.insert(
                    *reg,
                    ParamShape::Map {
                        key,
                        value: vk,
                        layout: None,
                    },
                );
            }
        }
    }
    for (reg, elems) in &structs.tuple_layouts {
        if !out.contains_key(reg) {
            if let Some(ks) = elems
                .iter()
                .map(|e| kinds.get(*e as usize))
                .collect::<Option<Vec<Kind>>>()
            {
                // Heterogeneous only — a homogeneous tuple lays out as a self-describing `Seq`.
                if !ks.windows(2).all(|w| w[0] == w[1]) {
                    out.insert(*reg, ParamShape::Tuple(ks));
                }
            }
        }
    }
    for (reg, name) in &structs.ind_type_of {
        if !out.contains_key(reg) {
            if let Some(shape) = kind::enum_shape_of_name(name, &program.enum_types) {
                out.insert(*reg, ParamShape::Enum(shape));
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
) -> (
    Vec<Vec<Option<Kind>>>,
    Vec<Vec<Option<ParamShape>>>,
    Vec<Vec<Option<u16>>>,
) {
    let mut kinds: Vec<Vec<Option<Kind>>> = program
        .functions
        .iter()
        .map(|f| {
            f.captures
                .iter()
                .map(|(_, g)| g.and_then(|gi| global_of(gi)))
                .collect()
        })
        .collect();
    let mut shapes: Vec<Vec<Option<ParamShape>>> = program
        .functions
        .iter()
        .map(|f| vec![None; f.captures.len()])
        .collect();
    // Per closure fi, per capture k: the body function index of a captured CLOSURE value (so calling
    // the captured closure resolves — closure composition like `(n) -> add1(add1(n))`). A captured
    // GLOBAL closure (a Main-`Let` closure promoted to a global) is resolved here; a captured LOCAL
    // closure is filled from the build-site `closure_of` in the loop below.
    let mut closures: Vec<Vec<Option<u16>>> = program
        .functions
        .iter()
        .map(|f| {
            f.captures
                .iter()
                .map(|(_, g)| g.and_then(|gi| global_closure_of(gi)))
                .collect()
        })
        .collect();
    for plan in std::iter::once(main_p1).chain(region_plans.iter().flatten()) {
        for op in &plan.ops {
            if let Op::MakeClosure {
                func, locals_start, ..
            } = *op
            {
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
    global_layout: &dyn Fn(u16) -> Option<FieldLayout>,
    global_declared: &dyn Fn(u16) -> Option<BoundaryType>,
    param_origin: &dyn Fn(usize, usize) -> Option<u16>,
    exact_param: &dyn Fn(usize, usize) -> bool,
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
    // A parameter some call hands a value of the exact extension holds one ([`exact_arguments`]), as a value of
    // another kind lands in a register that holds one.
    for (i, seed) in seeds.iter_mut().enumerate().take(f.param_count as usize) {
        if exact_param(fi, i) && seed.map_or(true, Kind::lands_in_tower) {
            *seed = Some(Kind::Tower);
        }
    }
    let num_regs = f.register_count as u32;
    // Split any register reused across disjoint live ranges of conflicting wasm types; parameters
    // (and capture/range members) are pinned so the function signature and range operands are
    // untouched. Identity unless a real conflict exists.
    let (mut ops, num_regs, placed) =
        regsplit::split_registers(&ops, num_regs, num_params, &program.functions);
    // Struct PARAMETERS' field layouts, resolved from the bytecode's `struct_types` — so `p's field`
    // resolves inside a `f(p: Point)` body (parameters are pinned by the splitter, so their registers
    // are unchanged). Only non-capturing functions have declared parameter types.
    let mut param_layouts = if cap_n == 0 {
        param_seeds(f, program)
    } else {
        Vec::new()
    };
    // ★ WHAT THE FUNCTION DECLARES IT RETURNS is what every `Return` hands back — the operand of each
    // takes that kind, so a value whose kind nothing in the body shows (a generic `T` monomorphized to
    // `Int`, a field read the backend could not type) still returns at the declared width.
    if let Some(k) = f.return_type.as_ref().and_then(kind::boundary_to_kind) {
        for op in &ops {
            if let Op::Return { src } = *op {
                if seeds.len() <= src as usize {
                    seeds.resize(src as usize + 1, None);
                }
                seeds[src as usize].get_or_insert(k);
            }
        }
    }
    // ★ AND EVERY VALUE THE BODY DECLARES: its kind (what an empty `a new Seq of Text` lacks), and
    // its shape where it has one (a `Seq of <record>`'s row layout, a `Map`'s value kind).
    let body_declared = seed_declarations(
        program,
        &ops,
        &region_declarations(program, start, end, &holes, &placed),
        &mut seeds,
        &mut param_layouts,
    );
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
    let mut param_closures: Vec<(u16, u16)> = (0..f.param_count)
        .filter_map(|i| param_origin(fi, i as usize).map(|c| (i, c)))
        .collect();
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
    // A CHANNEL PARAMETER (`Launch a task to worker with ch` passes a `Pipe of T` into `worker(ch: T)`)
    // is declared with its ELEMENT scalar type `T`, so `function_param_seeds` seeds `ch` as the scalar
    // `Kind::T` (an i64/f64). But the AOT models a channel as a Seq HANDLE (i32): every `Receive`/`Send`/
    // select recv-arm on `ch` uses it as a `chan` operand, and the caller passes the channel's i32
    // handle. Re-seed such a param to the channel's Seq kind (from its declared element) so the param
    // valtype matches the argument and the body's channel ops resolve — the wasm analog of the C tier's
    // channel-index param (`emit.rs`: a `Pipe of Int` param is live as the channel handle, not an Int).
    for reg in channel_param_regs(&ops, f.param_count) {
        if let Some(slot) = seeds.get_mut(reg as usize) {
            let seq = f
                .param_types
                .get(reg as usize)
                .and_then(|t| t.as_ref())
                .and_then(chan_param_seq_kind);
            if let Some(seq) = seq {
                *slot = Some(seq);
            }
        }
    }
    let fn_rt = fn_return_types(program);
    let (kinds, pc_reach, reachable_blocks) = infer_with_reachability(
        &ops,
        &program.constants,
        &program.struct_types,
        &program.enum_types,
        &fn_rt,
        num_regs as usize,
        &seeds,
        ret_of,
        global_of,
        closure_ret,
        ret_layout,
        fn_return_closure,
        &param_layouts,
        &param_closures,
        global_layout,
        global_declared,
        linked,
        &program.functions,
        &f.mutable_param_regs,
    )?;

    // Result kind: the declared `ret_kind` (a scalar `SlotKind`) when present, EXCEPT a non-scalar
    // (handle) return — a `-> Point` etc. — cannot be a `SlotKind`, so its `ret_kind` defaults to
    // Int; the inferred kind (from the `Return` operands) then wins on a value-type disagreement.
    let inferred = infer_result(&ops, &kinds, &pc_reach, strict)?;
    let result = match (f.ret_kind.map(Kind::from_slot), inferred) {
        (Some(d), Some(i)) if d.wasm_valtype() != i.wasm_valtype() => Some(i),
        (Some(d), _) => Some(d),
        (None, i) => i,
    };
    let structs = kind::struct_layout(
        &ops,
        &program.constants,
        &program.struct_types,
        &program.enum_types,
        &fn_rt,
        ret_layout,
        fn_return_closure,
        &param_layouts,
        &param_closures,
        global_layout,
        global_declared,
        &body_declared,
    );
    let cow_inserts = cow_struct_inserts(&ops, &kinds, num_regs, &program.functions);
    let reg_shape = complete_reg_shape(&structs, &kinds, program);
    let return_closure = infer_return_closure(&ops, &kinds, &structs, &pc_reach);
    let iter_iterable = kind::iter_pairing(&ops);
    let place = PlaceSites::of(&ops, &program.constants, &program.functions);
    let transient = transient_reads(&ops, num_regs, &program.functions);
    let declared = declared_registers(&structs, body_declared, &f.param_types);
    let range_loops = range_loop_registers(&ops, &program.functions);
    let inout: Vec<u16> = f
        .mutable_param_regs
        .iter()
        .copied()
        .filter(|p| u32::from(*p) < num_params as u32)
        .collect();
    if !inout.is_empty() && !f.captures.is_empty() {
        return Err(WasmLowerError::Unsupported(
            "a function value's body with a `mutable` parameter",
        ));
    }
    if linked {
        accumulate_as_add(&mut ops, &kinds);
    }
    let mut plan = Plan {
        range_loops,
        ops,
        kinds,
        num_params,
        num_regs,
        result,
        reachable_blocks,
        structs,
        cow_inserts,
        reg_shape,
        return_closure,
        iter_iterable,
        place,
        transient,
        declared,
        inout,
        exact: Vec::new(),
        params_tagged: Vec::new(),
        ret_tagged: false,
        absence: false,
        absence_regs: Vec::new(),
        absence_globals: Vec::new(),
        tower: tower::Towers::default(),
        stub: false,
    };
    tower::settle(&mut plan, program, linked)?;
    Ok(plan)
}

/// Parameter registers (`0..num_params`) USED as a channel operand anywhere in the body — the `chan`
/// side of a `Send`/`Receive`/`Try to send`/`Try to receive`/`Close` or a `select` recv arm. Such a
/// param arrived as a `Pipe of T` handle (a Seq in the AOT), not the scalar `T` its declaration seeds,
/// so its seed is re-typed to the channel Seq kind (see the call site).
fn channel_param_regs(ops: &[Op], num_params: u16) -> Vec<u16> {
    let mut regs = Vec::new();
    let mut note = |chan: u16| {
        if chan < num_params && !regs.contains(&chan) {
            regs.push(chan);
        }
    };
    for op in ops {
        match op {
            Op::ChanSend { chan, .. }
            | Op::ChanRecv { chan, .. }
            | Op::ChanTrySend { chan, .. }
            | Op::ChanTryRecv { chan, .. }
            | Op::ChanClose { chan }
            | Op::SelectArmRecv { chan, .. } => note(*chan),
            _ => {}
        }
    }
    regs
}

/// The Seq handle kind an AOT channel of declared element type `bt` presents — the parameter-seed
/// counterpart of `kind::infer`'s `ChanNew` element→seq mapping. Bool rides the `SeqInt` i64-0/1
/// layout; an element with no self-describing seq kind leaves the param unmodeled (`None`).
fn chan_param_seq_kind(bt: &BoundaryType) -> Option<Kind> {
    Some(match bt {
        BoundaryType::Int | BoundaryType::Bool => Kind::SeqInt,
        BoundaryType::Float => Kind::SeqFloat,
        BoundaryType::Text => Kind::SeqText,
        _ => return None,
    })
}

/// WHICH closure (body function index) this function returns, if any — the closure-valued analog of
/// [`fn_return_struct_layout`]. Scans REACHABLE closure-typed `Return`s (a nested closure's inline
/// body is unreachable here, so its `Return` is ignored); yields `Some(c)` iff they all agree on the
/// same statically-traced origin `c` (via `closure_of`, which is `Move`-aliased). `None` if it
/// returns no closure or more than one — a caller then cannot resolve a `CallValue` on the result.
fn infer_return_closure(
    ops: &[Op],
    kinds: &KindTable,
    structs: &kind::StructLayout,
    pc_reach: &[bool],
) -> Option<u16> {
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
/// Mark in `exact` every parameter some direct call in `plans` hands a value of the exact extension, so the callee
/// holds one there; answers whether a mark is new. Marks only accumulate: a parameter that once received one is
/// planned holding one for good.
fn exact_arguments(program: &CompiledProgram, plans: &[&Plan], exact: &mut [Vec<bool>]) -> bool {
    let mut grew = false;
    for plan in plans {
        for op in &plan.ops {
            let (func, args_start, arg_count) = match *op {
                Op::Call {
                    func,
                    args_start,
                    arg_count,
                    ..
                }
                | Op::Spawn {
                    func,
                    args_start,
                    arg_count,
                }
                | Op::SpawnHandle {
                    func,
                    args_start,
                    arg_count,
                    ..
                } => (func as usize, args_start, arg_count),
                _ => continue,
            };
            let params = program.functions.get(func).map_or(0, |f| f.param_count);
            for i in 0..arg_count.min(params) {
                if plan.kinds.get((args_start + i) as usize) == Some(Kind::Tower) {
                    if let Some(mark) = exact.get_mut(func).and_then(|v| v.get_mut(i as usize)) {
                        grew |= !*mark;
                        *mark = true;
                    }
                }
            }
        }
    }
    grew
}

fn compute_param_origins(program: &CompiledProgram, plans: &[&Plan]) -> Vec<Vec<Option<u16>>> {
    use std::collections::{HashMap, HashSet};
    let mut obs: HashMap<(usize, usize), HashSet<Option<u16>>> = HashMap::new();
    for plan in plans {
        for (pc, op) in plan.ops.iter().enumerate() {
            let (func, args_start, arg_count) = match *op {
                Op::Call {
                    func,
                    args_start,
                    arg_count,
                    ..
                } => (Some(func as usize), args_start, arg_count),
                Op::CallValue {
                    args_start,
                    arg_count,
                    ..
                } => (
                    plan.structs
                        .callee_func
                        .get(pc)
                        .copied()
                        .flatten()
                        .map(|f| f as usize),
                    args_start,
                    arg_count,
                ),
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
        BoundaryType::Struct(name) => {
            kind::resolve_named_layout(name, &program.struct_types, &program.constants)
                .map(ParamShape::Struct)
        }
        // ★ A MAP CARRIES ITS VALUES' LAYOUT when those values are records, so `(item k of m)'s f`
        // resolves. Without it a map of records could be indexed and the record could not be read.
        BoundaryType::Map(key, value) => kind::boundary_to_kind(value).map(|vk| {
            let layout = match value.as_ref() {
                BoundaryType::Struct(name) => {
                    kind::resolve_named_layout(name, &program.struct_types, &program.constants)
                }
                _ => None,
            };
            ParamShape::Map {
                key: kind::boundary_to_kind(key),
                value: vk,
                layout,
            }
        }),
        // A named enum resolves its shape through `enum_types`; a `Result` carries its own. Both
        // arrive as one `ParamShape::Enum`, so nothing downstream has to know which it was.
        BoundaryType::Enum(_) | BoundaryType::Result(_, _) => {
            kind::enum_shape_of(bt, &program.enum_types).map(ParamShape::Enum)
        }
        BoundaryType::Tuple(elems) => elems
            .iter()
            .map(kind::boundary_to_kind)
            .collect::<Option<Vec<Kind>>>()
            .map(ParamShape::Tuple),
        // ★ `Seq of <record>` — the ELEMENT's layout, so a loop variable taken out of the list can
        // resolve a field. Without it, a function handed a list of records could read no field of
        // any row: the layout walk traces where a record was BUILT, and a parameter was not built
        // here. Reading a row of a table is the most ordinary thing a program does with one.
        BoundaryType::Seq(elem) => match elem.as_ref() {
            BoundaryType::Struct(name) => {
                kind::resolve_named_layout(name, &program.struct_types, &program.constants)
                    .map(ParamShape::SeqOfStruct)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Each non-scalar PARAMETER's access-resolution shape (parameter register `i` → [`ParamShape`]),
/// from the bytecode's `param_types`/`struct_types`. Seeded into `struct_layout` so `param's field`
/// (struct) and `item k of m` (map) resolve like a cross-region access. A struct/map whose layout
/// or value kind is unresolvable is simply omitted, leaving the access soundly rejected.
fn param_seeds(f: &CompiledFunction, program: &CompiledProgram) -> Vec<(u16, ParamShape)> {
    f.param_types
        .iter()
        .enumerate()
        .filter_map(|(i, pt)| {
            pt.as_ref()
                .and_then(|bt| boundary_to_param_shape(bt, program))
                .map(|shape| (i as u16, shape))
        })
        .collect()
}

/// Every declaration ([`CompiledProgram::declared_types`]) whose op lies in the region
/// `[start, end)` outside `holes`, at that op's index in the region's final op stream: the index
/// [`extract_region`] gives it, carried through `placed` ([`regsplit::split_registers`]'s placement
/// of each extracted op) — so a declaration names the same op after both as before them.
fn region_declarations<'p>(
    program: &'p CompiledProgram,
    start: usize,
    end: usize,
    holes: &[(usize, usize)],
    placed: &[usize],
) -> Vec<(usize, &'p BoundaryType)> {
    let first = program
        .declared_types
        .partition_point(|(pc, _)| (*pc as usize) < start);
    program.declared_types[first..]
        .iter()
        .take_while(|(pc, _)| (*pc as usize) < end)
        .filter(|(pc, _)| {
            !holes
                .iter()
                .any(|&(s, e)| (*pc as usize) >= s && (*pc as usize) < e)
        })
        .filter_map(|(pc, bt)| {
            let pc = *pc as usize;
            let excised: usize = holes
                .iter()
                .filter(|&&(s, _)| s < pc)
                .map(|&(s, e)| e.min(pc) - s.max(start))
                .sum();
            Some((*placed.get(pc - start - excised)?, bt))
        })
        .collect()
}

/// Seed each declared value's register: its KIND into `kinds` (a declaration the inference has
/// not yet refined wins; a register the region already seeds keeps its seed), and its SHAPE into
/// `shapes` where the type has one. The register is the one the declaring op defines.
fn seed_declarations(
    program: &CompiledProgram,
    ops: &[Op],
    declared: &[(usize, &BoundaryType)],
    kinds: &mut Vec<Option<Kind>>,
    shapes: &mut Vec<(u16, ParamShape)>,
) -> Vec<(usize, u16, BoundaryType)> {
    let mut seeded = Vec::new();
    for &(at, bt) in declared {
        let Some(op) = ops.get(at) else { continue };
        let defs = crate::vm::instruction::op_def_uses(op, &program.functions).0;
        let [dst] = defs[..] else { continue };
        if let Some(k) = kind::boundary_to_kind(bt) {
            if kinds.len() <= dst as usize {
                kinds.resize(dst as usize + 1, None);
            }
            kinds[dst as usize].get_or_insert(k);
        }
        if let Some(shape) = boundary_to_param_shape(bt, program) {
            shapes.push((dst, shape));
        }
        seeded.push((at, dst, bt.clone()));
    }
    seeded
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
                None => {
                    return Err(WasmLowerError::Unsupported(
                        "return of an unknown-kind value",
                    ))
                }
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
                // A function that returns a value of the exact extension on one path returns every value as one.
                Some(Kind::Tower) if k.lands_in_tower() => {}
                Some(prev) if k == Kind::Tower && prev.lands_in_tower() => result = Some(Kind::Tower),
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
/// The resolved field layout of the record a `GlobalSet` put in a global — the same resolution
/// [`fn_return_struct_layout`] does for a returned struct, keyed on the register STORED rather
/// than the register returned. The declared type is authoritative: it carries the nested-composite
/// info (a Map field's value kind, an enum field's variants) that a nameless `Kind` cannot.
fn global_struct_layout(
    plan: &Plan,
    src: u16,
    struct_types: &[StructTypeDef],
    constants: &[Constant],
) -> Option<FieldLayout> {
    if let Some(name) = plan.structs.struct_name_of.get(&src) {
        if let Some(layout) = kind::resolve_named_layout(name, struct_types, constants) {
            return Some(layout);
        }
    }
    // A locally-built record whose declared type did not resolve: fall back to each field's kind
    // from its defining value register, exactly as a returned struct does.
    if let Some(layout) = plan.structs.reg_layout.get(&src) {
        return local_field_layout(plan, layout);
    }
    plan.structs.call_result.get(&src).cloned()
}

/// A locally-built record's layout resolved from the registers that filled each slot — the fallback
/// for a record with no resolvable declaration. A field's inner shape comes from what the register
/// that filled it was declared as (`a new Seq of Note`, a typed parameter), else a struct-typed field
/// is named from the plan's `struct_name_of`; anything else stays [`FieldNested::None`], its inner
/// shape unrecoverable from a nameless `Kind`. `None` unless every slot was filled with a value of
/// known kind.
fn local_field_layout(plan: &Plan, layout: &kind::LocalLayout) -> Option<FieldLayout> {
    layout
        .iter()
        .map(|&(f, vr)| {
            let vr = vr?;
            let k = plan.kinds.get(vr as usize)?;
            let nested = match (
                k,
                plan.declared.get(&vr),
                plan.structs.struct_name_of.get(&vr),
            ) {
                (_, Some(t), _) => kind::nested_of(t),
                (Kind::Struct, None, Some(name)) => FieldNested::Struct(name.clone()),
                _ => FieldNested::None,
            };
            Some((f, k, nested))
        })
        .collect()
}

fn fn_return_struct_layout(
    plan: &Plan,
    struct_types: &[StructTypeDef],
    constants: &[Constant],
) -> Option<FieldLayout> {
    for op in &plan.ops {
        if let Op::Return { src } = *op {
            if plan.kinds.get(src as usize) == Some(Kind::Struct) {
                // A locally-built struct with a KNOWN declared type: publish its DECLARED field layout
                // (from the struct type), which carries the full nested-composite info — a `Map`/`Enum`/
                // `Tuple` field's value kind / variant set / positional kinds. This is what lets a caller
                // resolve a `When V (binds)` on `f(…)'s enumfield` or `item k of (f(…)'s mapfield)`: the
                // returned struct's own value registers are in the callee's region (nameless `Kind` only),
                // so the DECLARED type is the authoritative source for the field's inner shape.
                if let Some(name) = plan.structs.struct_name_of.get(&src) {
                    if let Some(layout) = kind::resolve_named_layout(name, struct_types, constants)
                    {
                        return Some(layout);
                    }
                }
                // A LOCALLY-BUILT struct without a resolvable declared type: resolve each field's kind
                // from its defining value register ([`local_field_layout`]).
                if let Some(layout) = plan.structs.reg_layout.get(&src) {
                    return local_field_layout(plan, layout);
                }
                // A returned struct PARAMETER (or a cross-region call result passed straight through):
                // never built here, so it has no `reg_layout`, but its resolved field layout is in
                // `call_result` (seeded from the declared struct type / threaded from the callee).
                // Publish it verbatim so a caller resolves `f(p)'s field` exactly like a local struct.
                if let Some(resolved) = plan.structs.call_result.get(&src) {
                    return Some(resolved.clone());
                }
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

/// A function's wasm results: its declared result, then each `mutable` parameter's final value ([`Plan::inout`]).
fn plan_results(p: &Plan) -> Vec<u8> {
    let mut results = result_valtypes(p.result);
    results.extend(p.inout.iter().map(|r| p.kinds.valtype(*r as usize)));
    results
}

/// Push each `mutable` parameter's final value, after the declared result, before a function returns.
fn push_inout_results(code: &mut Vec<u8>, plan: &Plan) {
    for p in &plan.inout {
        local_get(code, *p as u32);
    }
}

/// Per-function emission context: the constant pool, how to resolve a host sink's import
/// index, and the wasm index of `program.functions[0]` (so `Op::Call { func }` →
/// `call (fn_base + func)`).
struct Ctx<'a> {
    absence_base: Option<u32>,
    constants: &'a [Constant],
    /// The display function of every composite value shape the module renders as Text.
    shown: &'a display::ShownTable,
    host_index: &'a dyn Fn(HostFn) -> Option<u32>,
    fn_base: u32,
    /// The wasm global index of the bump-allocator pointer `__heap_ptr` (the heap value model's
    /// linear-memory cursor); it follows the user globals.
    heap_global: u32,
    /// The wasm global index of the iterator-stack pointer `__iter_sp` (it follows `__heap_ptr`),
    /// for `Repeat` iteration. Grows *down* from the top of memory toward the up-growing heap;
    /// each live `Repeat` owns one 12-byte frame `[snapshot_ptr:i32][cursor:i32][len:i32]`.
    iter_global: u32,
    /// A monotonic replica identity independent of reusable heap addresses.
    replica_global: Option<u32>,
    /// Whether the module iterates at all — whether `__iter_sp` exists.
    uses_iter: bool,
    /// The wasm type index of each `program.functions[i]` (by function index), so a `CallValue`'s
    /// `call_indirect` can name the callee closure body's signature (= that function's own type).
    fn_type: &'a [u32],
    /// Each `program.functions[i]`'s ACTUAL emitted parameter value types (from its plan), so a direct
    /// `Call` matches the real signature — including a `Closure` parameter that lowers to i32, which the
    /// declared-type `function_param_seeds` cannot see (there is no `BoundaryType::Closure`).
    fn_param_valtypes: &'a [Vec<u8>],
    /// Each `program.functions[i]`'s parameter kinds (from its plan), so a direct `Call` hands a parameter that
    /// holds a value of the exact extension one ([`tower::push_argument`]).
    fn_param_kinds: &'a [Vec<Option<Kind>>],
    /// `fn_stubbed[i]` — whether `program.functions[i]` was dropped to a trap stub because it cannot
    /// run (`vm::reach`). A call into one sits in live code only where the reach analysis proved it
    /// never executes (a page entry's arm for a component nothing draws), so it lowers to a trap.
    fn_stubbed: &'a [bool],
    /// Each `program.functions[i]`'s result kind (`None` = void) — tells a `CallValue` whether the
    /// `call_indirect` leaves a value to bind into its destination.
    fn_results: &'a [Option<Kind>],
    /// Each function's `mutable` parameters ([`Plan::inout`]): results of the call after its declared one.
    fn_inout: &'a [Vec<u16>],
    /// Which parameters, results and globals hold tagged words ([`exact`]).
    exact: &'a exact::Signatures,
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
    /// Every text literal, interned into the data section. `None` in LINKER MODE, where a fixed
    /// address would name someone else's bytes and literals are built at run time as before.
    text_pool: Option<&'a std::cell::RefCell<TextPool>>,
    /// The runtime functions the emitter writes itself — the allocator every allocation site calls,
    /// and in an interactive page the release protocol. `None` in LINKER MODE, where allocation goes
    /// to the runtime's `dlmalloc` instead.
    runtime: Option<&'a Runtime>,
    /// Each user global's emitted wasm value type, so a `GlobalSet` can tell a REAL store from the
    /// VM's ownership clear (which arrives carrying a different width) and spell the clear at the
    /// global's own width. Without it the clear stored an `i64` zero into an `i32` handle global and
    /// the module failed to VALIDATE — see the `Op::GlobalSet` arm.
    global_valtypes: &'a [u8],
    /// Each user global's kind, by global index: what a store into one converts a value to (a value of the exact
    /// extension, in linker mode, for a global any region stores one into).
    global_kinds: &'a [Option<Kind>],
    /// Whether the program builds a `SharedMap` (`NewCrdt` kind 6). Only then can a map carry
    /// [`SHARED_MAP_MARK`], so only then do copy-on-write and map insertion test for it.
    shared_maps: bool,
    /// Linker mode: an integer `Op::Pow` lowers to the `logos_rt_bigint_*` runtime (a `Text` handle)
    /// rather than the self-contained i64 exponentiation-by-squaring. Set only by
    /// [`assemble_program_linked`]; the standalone emitter leaves it `false`.
    linked: bool,
    /// Linker mode + emitter heap: the import index of `logos_rt_alloc`. When `Some`, [`emit_alloc`] draws
    /// each block straight from the runtime allocator (`dlmalloc`, growing linear memory on demand) rather
    /// than a fixed bump slab — so the emitter heap is UNBOUNDED. `None` = the standalone bump path.
    rt_alloc: Option<u32>,
    /// The wasm global index of the S8 turn-retained list's `data_ptr` (`+1` = `len`, `+2` = `cap`)
    /// — `Some` exactly on an interactive, self-contained, heap-using module (mirrors
    /// `Runtime::layout`'s `TurnRetain`/`Retain`/`Release`/`Safepoint` gate). [`emit_alloc`] and every
    /// true-count call site check `ctx.heap_counting()` (this being `Some`) before emitting anything.
    retain_base: Option<u32>,
    /// Whether the op being lowered sits inside a loop of its function's bytecode ([`loop_pcs`]).
    /// There a sequence read, write or push keeps its mechanism inline — the bounds check and the
    /// address, the push's room test, the copy-on-write test — and calls the runtime only for its
    /// slow path (a clone, a growth, a trap): measured on V8, a tight loop of calls to
    /// `__logos_seq_slot` runs 9–17% slower than the same loop inline. Everywhere else a site is
    /// the call.
    hot: std::cell::Cell<bool>,
    /// Batch loops may collect while user frames are suspended; those frames pin their live values.
    batch_reclaim: bool,
    /// Per function: whether a call to it can reach a safepoint ([`may_safepoint`]).
    may_safepoint: &'a [bool],
}

impl<'a> Ctx<'a> {
    /// Whether this module carries S8's true-count machinery at all — the single gate every new
    /// retain/release/turn-list call site checks, so a non-interactive or linked module stays
    /// exactly as it was before S8 (batch programs "stay byte-identical", per the plan).
    fn heap_counting(&self) -> bool {
        self.retain_base.is_some()
    }
}

/// Whether an op terminated its basic block (a branch / return), so the block emitter stops.
#[derive(PartialEq, Eq)]
enum Flow {
    Straight,
    Terminated,
}

/// Per function: whether a call to it can reach a safepoint, the only place a batch module frees anything —
/// a loop's back edge in its body, a closure it calls, a builtin it hands a closure, or a call to a function
/// that can (a task runs to completion where it is started, so starting one is a call). A call that cannot
/// leaves its caller's roots as they are.
fn may_safepoint(fns: &[Plan]) -> Vec<bool> {
    let direct = |plan: &Plan| {
        plan.ops.iter().enumerate().any(|(pc, op)| match *op {
            Op::Jump { target }
            | Op::JumpIfFalse { target, .. }
            | Op::JumpIfTrue { target, .. } => target <= pc,
            Op::CallValue { .. } => true,
            Op::CallBuiltin {
                builtin: BuiltinId::TextFromBytes,
                ..
            } => true,
            Op::CallBuiltin {
                args_start,
                arg_count,
                ..
            } => (0..arg_count)
                .any(|a| plan.kinds.get((args_start + a) as usize) == Some(Kind::Closure)),
            _ => false,
        })
    };
    let mut may: Vec<bool> = fns.iter().map(|p| !p.stub && direct(p)).collect();
    loop {
        let reaches: Vec<usize> = (0..fns.len())
            .filter(|&fi| {
                !may[fi]
                    && fns[fi].ops.iter().any(|op| {
                        matches!(*op, Op::Call { func, .. } | Op::Spawn { func, .. }
                            | Op::SpawnHandle { func, .. }
                            if may.get(func as usize).copied().unwrap_or(true))
                    })
            })
            .collect();
        if reaches.is_empty() {
            break;
        }
        for fi in reaches {
            may[fi] = true;
        }
    }
    may
}

/// Emit one function's complete Code-section entry (its locals declaration followed by the
/// dispatch-loop body), from a [`Plan`].
fn emit_body(plan: &Plan, _ctx: &Ctx) -> R<Vec<u8>> {
    // A dropped (unreachable-from-Main) function: no locals, a lone `unreachable` trap. It is never
    // called (that's why it was dropped), so its `() -> ()` type is inert — this just lets the module
    // link when an imported stdlib carries functions the AOT can't lower but the program never uses.
    if plan.stub {
        return Ok(vec![
            0x00, /* 0 local groups */
            0x00, /* unreachable */
            0x0B, /* end */
        ]);
    }
    let ctx = _ctx;
    let blocks = Blocks::new(&plan.ops).ok_or(WasmLowerError::Unsupported(
        "jump target escapes the function",
    ))?;
    let pc_local = plan.num_regs;
    let in_loop = loop_pcs(&plan.ops);
    let roots = ctx
        .batch_reclaim
        .then(|| roots::Frame::new(plan, ctx.functions, ctx.may_safepoint));

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
            // Exact collections already share their representation and ownership machinery.
            // Keep the surrounding bounds/COW/word checks shared too: otherwise each
            // reconciliation loop repeats those mechanisms at every collection site.
            ctx.hot
                .set(in_loop[pc] && !ctx.exact.sequence_ints() && !ctx.exact.map_ints);
            if let Some(roots) = &roots {
                roots.before(&mut code, plan, ctx, pc);
            }
            let flow = tower::lower(pc, plan, ctx, &blocks, k, &mut code).inspect_err(|e| {
                if std::env::var_os("LOGOS_WASM_TRACE_FIELD").is_some() {
                    let (defs, uses) = crate::vm::instruction::op_def_uses(&plan.ops[pc], ctx.functions);
                    let kinds: Vec<String> = defs
                        .iter()
                        .chain(&uses)
                        .map(|r| format!("R{r} {:?}", plan.kinds.get(*r as usize)))
                        .collect();
                    eprintln!("[wasm] op {pc} `{:?}` refused: {e} ({})", plan.ops[pc], kinds.join(", "));
                }
            })?;
            if flow == Flow::Terminated {
                terminated = true;
                break;
            }
        }
        if !terminated {
            let end = blocks.end(k);
            if end >= plan.ops.len() {
                // Fell off the end of the function: void returns, a typed function cannot.
                match plan.result {
                    None => {
                        if let Some(roots) = &roots {
                            roots.leave(&mut code, ctx, end - 1);
                        }
                        emit_leave_loops(&mut code, plan, ctx);
                        exact::leave(&mut code, plan, ctx)?;
                        code.push(0x0F); // return
                    }
                    Some(_) => code.push(0x00), // unreachable
                }
            } else {
                goto_block(&mut code, pc_local, &blocks, k, blocks.block_of(end));
            }
        }
        blocks_code.push(code);
    }
    ctx.hot.set(false);

    let mut body = Vec::new();
    if runs_loops(plan) {
        // The loops this call starts sit above the frame that was on top when it was entered;
        // every way out unwinds back to it (`emit_leave_loops`).
        global_get(&mut body, ctx.iter_global);
        local_set(&mut body, loop_base_local(plan));
    }
    exact::prologue(&mut body, plan, ctx)?;
    body.extend(assemble_dispatch_loop(pc_local, &blocks_code));
    if !plan_results(plan).is_empty() {
        body.push(0x00); // unreachable: a function with results always returns inside a block
    }
    body.push(0x0B); // end function

    let mut entry = encode_locals(plan, roots.as_ref().map_or(0, roots::Frame::len));
    entry.extend(body);
    Ok(super::stackify::stackify(entry))
}

/// Per pc of `ops`: whether it lies inside a loop — between a backward jump's target and the jump,
/// inclusive. The compiler lays every loop out contiguously (a `While`'s test, its body, the jump
/// back; a `Repeat`'s `IterNext`, its body, the jump back), so the span a back edge closes is the
/// loop, nested ones included.
fn loop_pcs(ops: &[Op]) -> Vec<bool> {
    let mut depth = vec![0i32; ops.len() + 1];
    for (pc, op) in ops.iter().enumerate() {
        if let Op::Jump { target }
        | Op::JumpIfFalse { target, .. }
        | Op::JumpIfTrue { target, .. } = *op
        {
            if target <= pc {
                depth[target] += 1;
                depth[pc + 1] -= 1;
            }
        }
    }
    let mut open = 0;
    depth[..ops.len()]
        .iter()
        .map(|d| {
            open += d;
            open > 0
        })
        .collect()
}

/// Lower one op into the current block's code, returning whether it terminated the block.
fn lower_op(
    pc: usize,
    plan: &Plan,
    ctx: &Ctx,
    blocks: &Blocks,
    k: usize,
    code: &mut Vec<u8>,
) -> R<Flow> {
    let kinds = &plan.kinds;
    match plan.ops[pc] {
        Op::LoadConst { dst, idx } => {
            // A register the inference promoted to Float (an Int-initialized accumulator, `Let sum
            // be 0`) holds an `f64` local, so an Int/Bool literal flowing into it materializes as
            // `f64.const` — the exact promotion the VM performs at the first float op.
            let dst_float = kinds.get(dst as usize) == Some(Kind::Float);
            match ctx
                .constants
                .get(idx as usize)
                .ok_or(WasmLowerError::Unsupported("constant index out of range"))?
            {
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
                    // Observable absence has Kind::Nothing. An unobserved loan
                    // clear retains the payload local's physical type, and still
                    // clears the slot so it cannot keep a heap object rooted.
                    match kinds.valtype(dst as usize) {
                        F64 => {
                            code.push(0x44);
                            code.extend_from_slice(&0.0f64.to_le_bytes());
                        }
                        I32 => i32_const(code, 0),
                        _ => {
                            code.push(0x42);
                            leb_i64(code, 0);
                        }
                    }
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
            // promotion); same-kind moves are a plain copy. Any OTHER valtype mismatch is refused
            // rather than mis-emitted — a bare `local_get`/`local_set` pair is only well-typed when
            // both locals share a wasm valtype (`Op::RevSwap`'s own guardrail, below, is the same
            // check for the same reason: an unresolved kind must never reach a raw stack copy).
            if kinds.get(dst as usize) == Some(Kind::Float)
                && kinds.get(src as usize) != Some(Kind::Float)
            {
                push_as_f64(code, src, kinds.get(src as usize))?;
            } else if kinds.valtype(dst as usize) != kinds.valtype(src as usize) {
                return Err(WasmLowerError::Unsupported(
                    "move between differently-typed registers",
                ));
            } else {
                local_get(code, src as u32);
            }
            local_set(code, dst as u32);
            // Aliasing a mutable heap object (`Let cs be c's items`) gains a second holder — RETAIN so
            // a later mutation of either copy-on-writes instead of clobbering the other.
            if counts_aliases(kinds.get(dst as usize)) {
                emit_retain(code, ctx, dst);
            }
            Ok(Flow::Straight)
        }
        // The zero-garbage reversible register exchange (`Swap a and b`): swap the two locals through the
        // wasm operand stack with no scratch local — `get b; get a; set b; set a` leaves `a`'s old value
        // in `b` and vice versa. The `set` order (`b` before `a`) is what makes it a swap, not two moves.
        // Both locals carry the same LOGOS kind (the compiler only emits `RevSwap` for two same-typed
        // locals; a promoted global falls back to a temp-mediated Move pair), so their wasm valtypes
        // match and the stack sequence is well-typed — a mismatch is refused rather than mis-emitted.
        Op::RevSwap { a, b } => {
            if kinds.valtype(a as usize) != kinds.valtype(b as usize) {
                return Err(WasmLowerError::Unsupported(
                    "swap of two differently-typed registers",
                ));
            }
            local_get(code, b as u32);
            local_get(code, a as u32);
            local_set(code, b as u32);
            local_set(code, a as u32);
            Ok(Flow::Straight)
        }
        // The delta-carrying reversible / adiabatic-core ops (the zero-garbage abelian group updates a
        // `Reversibly:` block emits). Their FORWARD semantics are the plain in-place update — `place +=
        // delta`, `place -= delta`, `place ^= delta`, `place = -place` — over the modular ring
        // (ℤ/2⁶⁴, +): the VM runs them with WRAPPING arithmetic (`i64::wrapping_add`/`wrapping_sub`/
        // `wrapping_neg`), so WASM lowers to the native wrapping `i64.add`/`i64.sub`/`i64.xor`
        // opcodes — NOT the checked `Op::Add` path (which traps to promote to BigInt) — keeping the
        // reversible group law exact and tape-free on every tier. `place != delta` holds by
        // construction (the compiler rides the delta on a fresh scratch register).
        Op::RevAdd { place, delta } => {
            arith(code, 0x7C, place, place, delta); // i64.add (wrapping)
            Ok(Flow::Straight)
        }
        Op::RevSub { place, delta } => {
            arith(code, 0x7D, place, place, delta); // i64.sub (wrapping)
            Ok(Flow::Straight)
        }
        Op::RevXor { place, delta } => {
            arith(code, 0x85, place, place, delta); // i64.xor
            Ok(Flow::Straight)
        }
        Op::RevNeg { place } => {
            // `place = 0 - place` — modular negation (`i64::wrapping_neg`), self-inverse.
            code.push(0x42); // i64.const 0
            leb_i64(code, 0);
            local_get(code, place as u32);
            code.push(0x7D); // i64.sub
            local_set(code, place as u32);
            Ok(Flow::Straight)
        }
        // A Text-typed `+` (`add_join` resolved the result to Text) is string concatenation — route to
        // `lower_concat`, which stringifies both operands and joins them. Everything else is numeric.
        // `+ - *` on a BigInt result (linker mode) are exact big-integer arithmetic via the runtime; an
        // `Int` operand promotes to a BigInt. Checked FIRST so a `BigInt` `+` never falls to the numeric
        // (or string-concat) path. The result kind is `BigInt` iff `op_kind_effect` saw a BigInt operand.
        Op::Add { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::BigInt) =>
        {
            lower_bigint_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::BigintAdd)
        }
        Op::Sub { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::BigInt) =>
        {
            lower_bigint_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::BigintSub)
        }
        Op::Mul { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::BigInt) =>
        {
            lower_bigint_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::BigintMul)
        }
        Op::Div { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::BigInt) =>
        {
            lower_bigint_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::BigintDiv)
        }
        Op::Mod { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::BigInt) =>
        {
            lower_bigint_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::BigintMod)
        }
        Op::Add { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Complex) =>
        {
            lower_complex_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::ComplexAdd)
        }
        Op::Sub { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Complex) =>
        {
            lower_complex_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::ComplexSub)
        }
        Op::Mul { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Complex) =>
        {
            lower_complex_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::ComplexMul)
        }
        Op::Add { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Modular) =>
        {
            lower_modular_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::ModularAdd)
        }
        Op::Sub { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Modular) =>
        {
            lower_modular_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::ModularSub)
        }
        Op::Mul { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Modular) =>
        {
            lower_modular_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::ModularMul)
        }
        Op::Div { dst, lhs, rhs } | Op::ExactDiv { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Modular) =>
        {
            lower_modular_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::ModularDiv)
        }
        Op::Add { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Decimal) =>
        {
            lower_decimal_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::DecimalAdd)
        }
        Op::Sub { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Decimal) =>
        {
            lower_decimal_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::DecimalSub)
        }
        Op::Mul { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Decimal) =>
        {
            lower_decimal_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::DecimalMul)
        }
        Op::Add { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Money) => {
            lower_money_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::MoneyAdd)
        }
        Op::Sub { dst, lhs, rhs } if ctx.linked && kinds.get(dst as usize) == Some(Kind::Money) => {
            lower_money_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::MoneySub)
        }
        Op::Add { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Quantity) =>
        {
            lower_quantity_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::QuantityAdd)
        }
        Op::Sub { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Quantity) =>
        {
            lower_quantity_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::QuantitySub)
        }
        Op::Mul { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Quantity) =>
        {
            lower_quantity_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::QuantityMul)
        }
        Op::Div { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Quantity) =>
        {
            lower_quantity_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::QuantityDiv)
        }
        // `Moment/Date + Span` (calendar arithmetic, commutes): the Span operand carries the months/days,
        // the other is the base. Guarded on the SPAN operand (not dst) so it beats the `Moment + Duration`
        // i64-add arm above (whose result is also Moment).
        Op::Add { dst, lhs, rhs }
            if ctx.linked
                && (kinds.get(lhs as usize) == Some(Kind::Span)
                    || kinds.get(rhs as usize) == Some(Kind::Span)) =>
        {
            let (base, span) = if kinds.get(lhs as usize) == Some(Kind::Span) {
                (rhs, lhs)
            } else {
                (lhs, rhs)
            };
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
            let idx = (ctx.host_index)(HostFn::Lanes4Add)
                .ok_or(WasmLowerError::Unsupported("lanes4_add not imported"))?;
            local_get(code, lhs as u32);
            local_get(code, rhs as u32);
            code.push(0x10);
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        Op::Add { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Rational) =>
        {
            lower_rational_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::RationalAdd)
        }
        Op::Sub { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Rational) =>
        {
            lower_rational_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::RationalSub)
        }
        Op::Mul { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Rational) =>
        {
            lower_rational_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::RationalMul)
        }
        Op::Div { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Rational) =>
        {
            lower_rational_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::RationalQuotient)
        }
        Op::Mod { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Rational) =>
        {
            lower_rational_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::RationalRemainder)
        }
        Op::FloorDiv { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Rational) =>
        {
            lower_rational_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::RationalFloorQuotient)
        }
        Op::Pow { dst, lhs, rhs }
            if ctx.linked && kinds.get(dst as usize) == Some(Kind::Rational) =>
        {
            lower_rational_binop(code, kinds, ctx, dst, lhs, rhs, HostFn::RationalPower)
        }
        Op::BitXor { dst, lhs, rhs } | Op::BitAnd { dst, lhs, rhs } | Op::BitOr { dst, lhs, rhs }
            if ctx.linked
                && (kinds.get(lhs as usize) == Some(Kind::Rational)
                    || kinds.get(rhs as usize) == Some(Kind::Rational)) =>
        {
            let which = match plan.ops[pc] {
                Op::BitAnd { .. } => 0,
                Op::BitOr { .. } => 1,
                _ => 2,
            };
            let idx = (ctx.host_index)(HostFn::RationalBitwise)
                .ok_or(WasmLowerError::Unsupported("rational_bitwise not imported"))?;
            push_rational_operand(code, kinds, ctx, lhs)?;
            push_rational_operand(code, kinds, ctx, rhs)?;
            i32_const(code, which);
            code.push(0x10);
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        Op::Add { dst, lhs, rhs } => {
            if kinds.get(dst as usize) == Some(Kind::Text) {
                lower_concat(code, plan, ctx, dst, lhs, rhs)?;
                Ok(Flow::Straight)
            } else {
                lower_arith(code, ctx, kinds, dst, lhs, rhs, ArithOp::Add)
            }
        }
        Op::Sub { dst, lhs, rhs } => lower_arith(code, ctx, kinds, dst, lhs, rhs, ArithOp::Sub),
        Op::Mul { dst, lhs, rhs } => lower_arith(code, ctx, kinds, dst, lhs, rhs, ArithOp::Mul),
        Op::Div { dst, lhs, rhs } => lower_arith(code, ctx, kinds, dst, lhs, rhs, ArithOp::Div),
        Op::FloorDiv { dst, lhs, rhs } => {
            lower_floordiv_regs(code, kinds, plan.num_regs, dst, lhs, rhs)
        }
        Op::Mod { dst, lhs, rhs } => lower_arith(code, ctx, kinds, dst, lhs, rhs, ArithOp::Mod),
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
        Op::MagicDivU {
            dst,
            lhs,
            magic,
            more,
            mul_back,
            divisor: _,
            wrap: _,
        } => {
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
                let (a, b) = (plan.num_regs + 8, plan.num_regs + 9);
                local_get(code, dst as u32);
                local_set(code, a);
                display::emit_display_of(code, plan, ctx, src, b)?;
                emit_text_append(code, ctx, plan.num_regs, a, b, dst as u32);
                Ok(Flow::Straight)
            } else if ctx.linked && kinds.get(dst as usize) == Some(Kind::Lanes) {
                // `lanes += lanes` (the SHA-1 `Set e0 to e0 + m0`) — lane-wise add into `dst`.
                let idx = (ctx.host_index)(HostFn::Lanes4Add)
                    .ok_or(WasmLowerError::Unsupported("lanes4_add not imported"))?;
                local_get(code, dst as u32);
                local_get(code, src as u32);
                code.push(0x10);
                leb_u32(code, idx);
                local_set(code, dst as u32);
                Ok(Flow::Straight)
            } else {
                lower_arith(code, ctx, kinds, dst, dst, src, ArithOp::Add)
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
                let idx = (ctx.host_index)(HostFn::Lanes4Xor)
                    .ok_or(WasmLowerError::Unsupported("lanes4_xor not imported"))?;
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
            _ => Err(WasmLowerError::Unsupported(
                "`&` of a non-Int/Bool/Word value",
            )),
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
            _ => Err(WasmLowerError::Unsupported(
                "`|` of a non-Int/Bool/Word value",
            )),
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
        Op::Lt { dst, lhs, rhs } => lower_compare(code, ctx, kinds, dst, lhs, rhs, Cmp::Lt),
        Op::Gt { dst, lhs, rhs } => lower_compare(code, ctx, kinds, dst, lhs, rhs, Cmp::Gt),
        Op::LtEq { dst, lhs, rhs } => lower_compare(code, ctx, kinds, dst, lhs, rhs, Cmp::Le),
        Op::GtEq { dst, lhs, rhs } => lower_compare(code, ctx, kinds, dst, lhs, rhs, Cmp::Ge),
        Op::ApproxEq { dst, lhs, rhs } => lower_approx_eq(code, kinds, dst, lhs, rhs),
        Op::Eq { dst, lhs, rhs } => {
            if kinds.get(lhs as usize) == Some(Kind::Text)
                && kinds.get(rhs as usize) == Some(Kind::Text)
            {
                lower_text_eq(code, ctx, plan.num_regs, dst, lhs, rhs, false);
                Ok(Flow::Straight)
            } else if ctx.linked
                && kinds.get(lhs as usize) == Some(Kind::Uuid)
                && kinds.get(rhs as usize) == Some(Kind::Uuid)
            {
                lower_uuid_eq(code, ctx, dst, lhs, rhs, false)
            } else {
                lower_compare(code, ctx, kinds, dst, lhs, rhs, Cmp::Eq)
            }
        }
        Op::NotEq { dst, lhs, rhs } => {
            if kinds.get(lhs as usize) == Some(Kind::Text)
                && kinds.get(rhs as usize) == Some(Kind::Text)
            {
                lower_text_eq(code, ctx, plan.num_regs, dst, lhs, rhs, true);
                Ok(Flow::Straight)
            } else if ctx.linked
                && kinds.get(lhs as usize) == Some(Kind::Uuid)
                && kinds.get(rhs as usize) == Some(Kind::Uuid)
            {
                lower_uuid_eq(code, ctx, dst, lhs, rhs, true)
            } else {
                lower_compare(code, ctx, kinds, dst, lhs, rhs, Cmp::Ne)
            }
        }
        Op::Jump { target } => {
            goto_block(code, plan.num_regs, blocks, k, blocks.block_of(target));
            Ok(Flow::Terminated)
        }
        Op::JumpIfFalse { cond, target } => {
            emit_cond_jump(
                code,
                cond,
                true,
                blocks.block_of(target),
                blocks.block_of(pc + 1),
                plan.num_regs,
                blocks.br_loop(k),
            );
            Ok(Flow::Terminated)
        }
        Op::JumpIfTrue { cond, target } => {
            emit_cond_jump(
                code,
                cond,
                false,
                blocks.block_of(target),
                blocks.block_of(pc + 1),
                plan.num_regs,
                blocks.br_loop(k),
            );
            Ok(Flow::Terminated)
        }
        Op::GlobalGet { dst, idx } => {
            code.push(0x23); // global.get
            leb_u32(code, idx as u32);
            local_set(code, dst as u32);
            // Reading a global heap handle aliases the global's object — RETAIN so a mutation of the
            // read copy copy-on-writes instead of clobbering the global. A transient read keeps
            // nothing ([`transient_reads`]), and a take moves the global's own hold ([`PlaceSites`]).
            let aliases = !plan.place.take.get(pc).copied().unwrap_or(false)
                && !plan.transient.get(dst as usize).copied().unwrap_or(false);
            if counts_aliases(kinds.get(dst as usize)) && aliases {
                emit_retain(code, ctx, dst);
            }
            Ok(Flow::Straight)
        }
        Op::LoadToday { dst } => {
            let idx = (ctx.host_index)(HostFn::Today)
                .ok_or(WasmLowerError::Unsupported("today not imported"))?;
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
        Op::NewEmptyListFixed { dst, capacity } => {
            emit_fixed_header(
                code,
                ctx,
                plan.num_regs,
                dst as u32,
                empty_collection_class(kinds.get(dst as usize)),
                capacity as u32,
            );
            Ok(Flow::Straight)
        }
        Op::NewEmptyListWithCapacity { dst, capacity } => {
            emit_capacity_header(
                code,
                ctx,
                plan.num_regs,
                dst as u32,
                capacity as u32,
                empty_collection_class(kinds.get(dst as usize)),
                8,
            );
            Ok(Flow::Straight)
        }
        Op::NewEmptyMapWithCapacity { dst, capacity } => {
            emit_capacity_header(
                code,
                ctx,
                plan.num_regs,
                dst as u32,
                capacity as u32,
                empty_collection_class(kinds.get(dst as usize)),
                16,
            );
            Ok(Flow::Straight)
        }
        Op::NewEmptyList { dst }
        | Op::NewEmptyListI32 { dst }
        | Op::NewEmptyMap { dst }
        | Op::NewEmptySet { dst } => {
            emit_empty_header(
                code,
                ctx,
                plan.num_regs,
                dst as u32,
                empty_collection_class(kinds.get(dst as usize)),
            );
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
            if counts_aliases(kinds.get(value as usize)) {
                emit_retain(code, ctx, value);
            }
            // S8's true-count RETAIN — a push never overwrites anything, so there is no matching
            // release to pair here (only a later `ListPop`/the container's own eventual free ends
            // this hold; see `lower_remove_from`'s doc for why nothing else in this session's scope
            // removes a single interior element of a `Seq`).
            if counted_kind(kinds.get(value as usize)) {
                emit_true_retain(code, ctx, value);
            }
            Ok(Flow::Straight)
        }
        Op::ListPushField { obj, field, src } => {
            let cow = plan.cow_inserts.get(pc).copied().unwrap_or(true);
            lower_list_push_field(code, plan, kinds, ctx, cow, obj, field, src)?;
            if counts_aliases(kinds.get(src as usize)) {
                emit_retain(code, ctx, src);
            }
            if counted_kind(kinds.get(src as usize)) {
                emit_true_retain(code, ctx, src);
            }
            Ok(Flow::Straight)
        }
        Op::ListPop { list, dst } => {
            emit_cow(code, kinds, &plan.structs, ctx, plan.num_regs, list)?;
            lower_list_pop(code, kinds, dst, list)?;
            // S8's true count: the list no longer holds what it popped; the register it went to is no
            // holder, so a value nothing else keeps is freed at the next safepoint.
            if counted_kind(kinds.get(list as usize).and_then(Kind::seq_elem)) {
                emit_true_release(code, ctx, |c| local_get(c, dst as u32));
            }
            // The popped element's slot stays physically in the (now-shorter) buffer, so `dst`
            // aliases it until a later push overwrites it — RETAIN a clonable handle so mutating
            // `dst` copy-on-writes (same discipline as an `Index` extract).
            if counts_aliases(kinds.get(dst as usize)) {
                emit_retain(code, ctx, dst);
            }
            Ok(Flow::Straight)
        }
        // `IndexUnchecked` (the Oracle-proven-in-bounds optimizer form) executes IDENTICALLY to `Index`
        // in the VM — the "unchecked" only drops the bounds branch inside a native region. The AOT keeps
        // its bounds check (a safe superset), so the two lower to the same code.
        Op::Index {
            dst,
            collection,
            index,
        }
        | Op::IndexUnchecked {
            dst,
            collection,
            index,
        } => {
            match kinds.get(collection as usize) {
                Some(Kind::Map) => {
                    lower_map_get(code, kinds, ctx, plan.num_regs, dst, collection, index)?
                }
                Some(Kind::Text) => {
                    lower_text_index(code, ctx, plan.num_regs, dst, collection, index)?
                }
                _ => lower_index(code, ctx, kinds, dst, collection, index)?,
            }
            // `Let row be item k of matrix` extracts a handle that aliases the element in the
            // container — RETAIN so mutating the extracted row copy-on-writes. An item the place
            // lowering TAKES out, or a transient read ([`transient_reads`]), is no new alias.
            let aliases = !plan.place.take.get(pc).copied().unwrap_or(false)
                && !plan.transient.get(dst as usize).copied().unwrap_or(false);
            if counts_aliases(kinds.get(dst as usize)) && aliases {
                emit_retain(code, ctx, dst);
            }
            Ok(Flow::Straight)
        }
        // `SetIndexUnchecked` is the Oracle-proven-in-bounds form of `SetIndex` (identical in the VM);
        // lowered the same (the AOT keeps its bounds check).
        Op::SetIndex {
            collection,
            index,
            value,
        }
        | Op::SetIndexUnchecked {
            collection,
            index,
            value,
        } => {
            emit_cow(code, kinds, &plan.structs, ctx, plan.num_regs, collection)?;
            if kinds.get(collection as usize) == Some(Kind::Struct) {
                let cow = plan.cow_inserts.get(pc).copied().unwrap_or(true);
                lower_record_field_set(code, plan, kinds, ctx, pc, cow, collection, value)?;
            } else if plan.place.clear.get(pc).copied().unwrap_or(false) {
                // The place lowering's clear of an item it took out stores nothing ([`PlaceSites`]).
                return Ok(Flow::Straight);
            } else if kinds.get(collection as usize) == Some(Kind::Map) {
                lower_map_insert(code, kinds, ctx, plan.num_regs, collection, index, value)?;
            } else {
                lower_set_index(code, ctx, kinds, plan.num_regs, collection, index, value)?;
            }
            // A clear stored nothing: the slot still holds what the take read, in both counts.
            if plan.place.clear.get(pc).copied().unwrap_or(false) {
                return Ok(Flow::Straight);
            }
            // Storing a mutable heap VALUE as an element/map-value (a second holder) — RETAIN so a
            // later mutation of the original copy-on-writes instead of mutating the stored one. A
            // level put back where the place lowering took it from is the same holder, not a second.
            if counts_aliases(kinds.get(value as usize))
                && !plan.place.put_back.get(pc).copied().unwrap_or(false)
            {
                emit_retain(code, ctx, value);
            }
            // S8's true count: every store holds what it stores. A put-back is such a store — what
            // comes back may be the copy a mutation in between made, not the value the take read —
            // and the store released what it overwrote.
            if counted_kind(kinds.get(value as usize)) {
                emit_true_retain(code, ctx, value);
            }
            Ok(Flow::Straight)
        }
        Op::NewRange { dst, start, end } => {
            if ctx.linked || !plan.range_loops.contains_key(&dst) {
                lower_new_range(code, ctx, plan.num_regs, dst, start, end);
            } else {
                // Capture the bounds here, while exact::on_words has narrowed them. The
                // immediately following IterPrepare is this constructor's only use and
                // has no other predecessor (range_loop_registers).
                lower_range_iter_prepare(code, ctx, plan.num_regs, start, end);
            }
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
                // S8's true count: the tuple holds each counted value it was built from.
                for i in 0..count {
                    if counted_kind(kinds.get((start + i) as usize)) {
                        emit_true_retain(code, ctx, start + i);
                    }
                }
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
            if ctx.linked || !plan.range_loops.contains_key(&iterable) {
                lower_iter_prepare(code, kinds, ctx, plan.num_regs, iterable)?;
            }
            Ok(Flow::Straight)
        }
        Op::IterNext { dst, exit } => {
            lower_iter_next(code, plan, ctx, blocks, k, dst, exit, pc)?;
            Ok(Flow::Terminated)
        }
        Op::SetAdd { set, value } => {
            emit_cow(code, kinds, &plan.structs, ctx, plan.num_regs, set)?;
            lower_set_add(code, kinds, ctx, plan.num_regs, set, value)?;
            Ok(Flow::Straight)
        }
        Op::RemoveFrom { collection, value } => {
            emit_cow(code, kinds, &plan.structs, ctx, plan.num_regs, collection)?;
            lower_remove_from(code, kinds, ctx, plan.num_regs, collection, value)?;
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
        Op::Contains {
            dst,
            collection,
            value,
        } => {
            if kinds.get(collection as usize) == Some(Kind::Map) {
                lower_map_contains(code, kinds, ctx, plan.num_regs, dst, collection, value)?;
            } else {
                lower_contains(code, kinds, ctx, plan.num_regs, dst, collection, value)?;
            }
            Ok(Flow::Straight)
        }
        Op::SliceOp {
            dst,
            collection,
            start,
            end,
        } => {
            lower_slice(code, kinds, ctx, plan.num_regs, dst, collection, start, end)?;
            Ok(Flow::Straight)
        }
        Op::SeqConcat { dst, lhs, rhs } => {
            lower_seq_concat(code, kinds, ctx, plan.num_regs, dst, lhs, rhs)?;
            Ok(Flow::Straight)
        }
        Op::Concat { dst, lhs, rhs } => {
            // ★ `"" ++ x` IS JUST `x` RENDERED, and every interpolation begins with one. An
            // interpolated string compiles to `LoadConst "" -> dst` followed by a `Concat` per
            // piece, so `{i}` — the most common thing a view contains — became a concat whose left
            // side is empty. `lower_concat` stringifies BOTH sides and then allocates a third Text
            // to copy them into, so a one-piece hole cost four allocations, two of them garbage the
            // instant they were made. Measured at 80 bytes of heap per hole per row per turn.
            //
            // Recognised as the ADJACENT PAIR the compiler emits, and only when this `Concat` is not
            // a block leader — if it were, some branch could land on it and the preceding op would
            // not be its only predecessor. `is_leader` makes that a proof rather than an assumption
            // about how interpolations happen to be emitted.
            // ★ NOT `lhs == dst`. The accumulator starts as the destination, but copy-propagation
            // rewrites it, so the pair arrives as `Concat { dst: 98, lhs: 43 }` after a
            // `LoadConst { dst: 43 }`. Requiring the two to be equal made this fold silently never
            // fire — the trace said `-> false` on the one site it was written for. The fold does not
            // need them equal: `"" ++ x` is `x` rendered, whichever register holds the empty side.
            let folds = pc > 0
                && !blocks.is_leader(pc)
                && matches!(
                    plan.ops.get(pc - 1),
                    Some(Op::LoadConst { dst: d, idx })
                        if *d == lhs
                            && matches!(
                                ctx.constants.get(*idx as usize),
                                Some(Constant::Text(t)) if t.is_empty()
                            )
                );
            if folds {
                display::emit_display_of(code, plan, ctx, rhs, dst as u32)?;
                return Ok(Flow::Straight);
            }
            lower_concat(code, plan, ctx, dst, lhs, rhs)?;
            Ok(Flow::Straight)
        }
        Op::FormatValue {
            dst,
            src,
            spec,
            debug_prefix,
        } => {
            lower_format_value(code, plan, ctx, dst, src, spec, debug_prefix)?;
            Ok(Flow::Straight)
        }
        Op::DeepClone { dst, src } => {
            lower_deep_clone(code, kinds, &plan.structs, ctx, plan.num_regs, dst, src)?;
            Ok(Flow::Straight)
        }
        // `copy(x)` — the builtin form of a deep clone (an independent copy of a heap value; the value
        // itself for a scalar), the same lowering as `Op::DeepClone`.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Copy,
            args_start,
            ..
        } => {
            lower_deep_clone(
                code,
                kinds,
                &plan.structs,
                ctx,
                plan.num_regs,
                dst,
                args_start,
            )?;
            Ok(Flow::Straight)
        }
        Op::NewStruct { dst, .. } => {
            let count = plan.structs.count.get(pc).copied().flatten().unwrap_or(0);
            lower_new_struct(code, ctx, plan.num_regs, count, dst);
            Ok(Flow::Straight)
        }
        Op::StructInsert { obj, value, .. } => {
            let slot =
                plan.structs
                    .slot
                    .get(pc)
                    .copied()
                    .flatten()
                    .ok_or(WasmLowerError::Unsupported(
                        "struct insert with no static slot",
                    ))?;
            let cow = plan.cow_inserts.get(pc).copied().unwrap_or(true);
            let held = tower::field_value(code, plan, ctx, obj, usize::from(slot), value)?;
            lower_struct_insert(
                code,
                kinds,
                &plan.structs,
                ctx,
                plan.num_regs,
                slot,
                obj,
                value,
                cow,
                held,
            )?;
            // The field is a second holder of a value that counts its holders — RETAIN, so a later
            // write through the register that built it copies first.
            if counts_aliases(kinds.get(value as usize)) {
                emit_retain(code, ctx, value);
            }
            // The record owns its field independently of the register that supplied it.
            // Replacement and record reclamation release this ownership.
            if counted_kind(kinds.get(value as usize)) {
                emit_true_retain(code, ctx, value);
            }
            Ok(Flow::Straight)
        }
        Op::GetField { dst, obj, field } => {
            // A refusal here names the backend's internals, not the author's code. `LOGOS_WASM_TRACE_FIELD`
            // prints WHICH access could not resolve, which is the difference between a bisect and a look.
            if plan.structs.slot.get(pc).copied().flatten().is_none()
                && std::env::var_os("LOGOS_WASM_TRACE_FIELD").is_some()
            {
                let name = match ctx.constants.get(field as usize) {
                    Some(Constant::Text(t)) => t.clone(),
                    other => format!("{other:?}"),
                };
                eprintln!(
                    "[wasm] unresolved field access: '{name}' (pc {pc}, obj reg {obj}, kind {:?})",
                    kinds.get(obj as usize)
                );
            }
            let slot =
                plan.structs
                    .slot
                    .get(pc)
                    .copied()
                    .flatten()
                    .ok_or(WasmLowerError::Unsupported(
                        "field access with no static slot",
                    ))?;
            // A `ConvergentCount`/`Tally` counter field stores a per-replica counter OBJECT, but reads
            // as its Int total (`Σinc − Σdec`): project it here rather than raw-loading the pointer.
            let is_counter = plan
                .structs
                .field_value
                .get(pc)
                .copied()
                .flatten()
                .map_or(false, |vr| {
                    kinds.get(vr as usize) == Some(Kind::CrdtCounter)
                });
            if is_counter {
                lower_get_counter_total(code, plan, slot, dst, obj)?;
            } else {
                lower_get_field(code, kinds, slot, dst, obj)?;
                // `Let cs be c's items` aliases the field's mutable heap object — RETAIN so a mutation of
                // the extracted handle copy-on-writes rather than mutating the field in place. A field
                // the place lowering TAKES out, or a transient read ([`transient_reads`]), is no new alias.
                let aliases = !plan.place.take.get(pc).copied().unwrap_or(false)
                    && !plan.transient.get(dst as usize).copied().unwrap_or(false);
                if counts_aliases(kinds.get(dst as usize)) && aliases {
                    emit_retain(code, ctx, dst);
                }
            }
            Ok(Flow::Straight)
        }
        Op::CheckPolicy {
            subject,
            predicate,
            is_capability,
            object,
            ..
        } => {
            lower_check_policy(code, plan, ctx, subject, predicate, is_capability, object)?;
            Ok(Flow::Straight)
        }
        Op::CrdtBump {
            obj,
            field,
            amount,
            negate,
        } => {
            lower_crdt_bump(code, plan, ctx, obj, field, amount, negate)?;
            Ok(Flow::Straight)
        }
        // `Merge`/`Converge` of two same-typed counter-CRDT structs. The WASM tier only supports counter
        // (`ConvergentCount`/`Tally`) fields, whose merge IS the per-replica-MAX lattice join — so `CrdtJoin`
        // (the idempotent join) and `CrdtMerge` are identical here and share `lower_crdt_merge`. A bare-`Int`
        // field (where they would differ, MAX vs ADD) is soundly refused by `lower_crdt_merge` for both.
        Op::CrdtMerge { target, source } | Op::CrdtJoin { target, source } => {
            lower_crdt_merge(code, plan, ctx, target, source)?;
            Ok(Flow::Straight)
        }
        Op::NewCrdt { dst, kind } => {
            // A fresh CRDT is empty, and single-replica it IS its underlying collection: an OR-Set
            // (0/3) and an RGA sequence (1) are a `[0][0][0]` header, a divergent register (2) an
            // empty `Text`, a counter (4/5) a per-replica record object, and a map (6) a map header
            // marked SHARED, which keeps it sorted by key and exempt from copy-on-write.
            match kind {
                0 | 1 | 3 => emit_empty_header(
                    code,
                    ctx,
                    plan.num_regs,
                    dst as u32,
                    empty_collection_class(kinds.get(dst as usize)),
                ),
                2 => {
                    lower_text_literal(code, ctx, plan.num_regs, b"");
                    local_set(code, dst as u32);
                }
                // A `ConvergentCount` (4) / `Tally` (5) is a STATE-BASED per-replica counter object,
                // idempotent under merge (see `emit_new_counter`). Both allocate the same shape (a
                // G-Counter is a PN-counter whose `dec` never moves), so a single arm serves both.
                4 | 5 => emit_new_counter(code, ctx, plan.num_regs, dst as u32),
                6 => {
                    // A shared map has the same owned entries and true count as an ordinary map.
                    // The sign controls sharing; its low byte still identifies the heap layout.
                    emit_empty_header(
                        code,
                        ctx,
                        plan.num_regs,
                        dst as u32,
                        AllocClass::MapScalarScalar,
                    );
                    local_get(code, dst as u32);
                    i32_const(
                        code,
                        SHARED_MAP_MARK | i32::from(AllocClass::MapScalarScalar.code()),
                    );
                    i32_store(code, 12);
                }
                _ => {
                    return Err(WasmLowerError::Unsupported(
                        "a CRDT kind this backend does not know",
                    ))
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
            emit_empty_header(
                code,
                ctx,
                plan.num_regs,
                dst as u32,
                empty_collection_class(kinds.get(dst as usize)),
            );
            Ok(Flow::Straight)
        }
        Op::ChanSend { chan, val } => {
            lower_list_push(code, kinds, ctx, plan.num_regs, chan, val)?;
            // S8's true count: the pipe holds the value until it is received.
            if counted_kind(kinds.get(val as usize)) {
                emit_true_retain(code, ctx, val);
            }
            Ok(Flow::Straight)
        }
        Op::ChanRecv { dst, chan } => {
            lower_chan_recv(code, kinds, plan.num_regs, dst, chan)?;
            // The pipe no longer holds what it handed over.
            if counted_kind(kinds.get(chan as usize).and_then(Kind::seq_elem)) {
                emit_true_release(code, ctx, |c| local_get(c, dst as u32));
            }
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
            if counted_kind(kinds.get(val as usize)) {
                emit_true_retain(code, ctx, val);
            }
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
        Op::Spawn {
            func,
            args_start,
            arg_count,
        } => {
            if emit_sync_call(code, plan, ctx, func, args_start, arg_count)? {
                code.push(0x1A); // drop the (discarded) result
            }
            Ok(Flow::Straight)
        }
        Op::SpawnHandle {
            dst,
            func,
            args_start,
            arg_count,
        } => {
            if emit_sync_call(code, plan, ctx, func, args_start, arg_count)? {
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
            // Create the empty local inbox FIFO; stash its handle at the reserved slot. A compiler
            // scratch slot, not a normally-kinded register — `Raw` (its element kind is not tracked
            // the way a declared `Seq`/`Pipe` register's is).
            emit_empty_header(code, ctx, plan.num_regs, plan.num_regs + 8, AllocClass::Raw);
            i32_const(code, NET_INBOX_ADDR);
            local_get(code, plan.num_regs + 8);
            i32_store(code, 0);
            Ok(Flow::Straight)
        }
        Op::NetSend { msg, .. } => {
            let elem = kinds.get(msg as usize).ok_or(WasmLowerError::Unsupported(
                "Send of an unknown-kind message",
            ))?;
            i32_const(code, NET_INBOX_ADDR);
            i32_load(code, 0);
            local_set(code, plan.num_regs + 8);
            lower_list_push_at(code, elem, ctx, plan.num_regs, plan.num_regs + 8, msg)?;
            if counts_aliases(kinds.get(msg as usize)) {
                emit_retain(code, ctx, msg);
            }
            // S8's true count: the inbox holds the message until it is received.
            if counted_kind(kinds.get(msg as usize)) {
                emit_true_retain(code, ctx, msg);
            }
            Ok(Flow::Straight)
        }
        Op::NetStream { values, .. } => {
            // A batch STREAM delivers the whole list; loopback pushes the list HANDLE (single-node, no
            // framing) so `Await stream` pops the same list — byte-faithful to the frame/deframe round-trip.
            let elem = kinds
                .get(values as usize)
                .ok_or(WasmLowerError::Unsupported(
                    "Stream of an unknown-kind list",
                ))?;
            i32_const(code, NET_INBOX_ADDR);
            i32_load(code, 0);
            local_set(code, plan.num_regs + 8);
            lower_list_push_at(code, elem, ctx, plan.num_regs, plan.num_regs + 8, values)?;
            if counts_aliases(kinds.get(values as usize)) {
                emit_retain(code, ctx, values);
            }
            if counted_kind(kinds.get(values as usize)) {
                emit_true_retain(code, ctx, values);
            }
            Ok(Flow::Straight)
        }
        Op::NetAwait { dst, .. } => {
            let elem = kinds.get(dst as usize).ok_or(WasmLowerError::Unsupported(
                "Await of an unknown-kind message",
            ))?;
            i32_const(code, NET_INBOX_ADDR);
            i32_load(code, 0);
            local_set(code, plan.num_regs + 8);
            emit_pop_front(code, elem, plan.num_regs + 8, plan.num_regs + 5, dst as u32)?;
            if counts_aliases(kinds.get(dst as usize)) {
                emit_retain(code, ctx, dst);
            }
            Ok(Flow::Straight)
        }
        Op::NetMakePeer { dst, addr } => {
            local_get(code, addr as u32);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        Op::NewInductive {
            dst,
            ctor,
            args_start,
            count,
            ..
        } => {
            lower_new_inductive(
                code,
                kinds,
                ctx,
                plan.num_regs,
                dst,
                ctor,
                args_start,
                count,
            )?;
            // S8's true count: the variant holds each counted payload value it was built from.
            for i in 0..count {
                if counted_kind(kinds.get((args_start + i) as usize)) {
                    emit_true_retain(code, ctx, args_start + i);
                }
            }
            Ok(Flow::Straight)
        }
        Op::TestArm {
            dst,
            target,
            variant,
        } => {
            lower_test_arm(code, kinds, ctx, dst, target, variant)?;
            Ok(Flow::Straight)
        }
        Op::BindArm {
            dst, target, index, ..
        } => {
            lower_bind_arm(code, kinds, ctx, dst, target, index)?;
            Ok(Flow::Straight)
        }
        Op::MakeClosure {
            dst,
            func,
            locals_start,
        } => {
            lower_make_closure(code, ctx, plan.num_regs, dst, func, locals_start)?;
            Ok(Flow::Straight)
        }
        Op::CallValue {
            dst,
            callee,
            args_start,
            arg_count,
            ..
        } => {
            // A heap argument to a closure call gains the closure body's parameter as a second holder
            // — RETAIN, like a direct `Call` (value semantics across the closure boundary).
            for a in 0..arg_count {
                let arg = args_start + a;
                if counts_aliases(kinds.get(arg as usize)) {
                    emit_retain(code, ctx, arg);
                }
            }
            lower_call_value(code, plan, ctx, pc, dst, callee, args_start, arg_count)?;
            Ok(Flow::Straight)
        }
        Op::IterPop => {
            // Unwind exactly this frame, including its snapshot's owned handles.
            global_get(code, ctx.iter_global);
            i32_load(code, FRAME_PREV);
            if let Some(unwind) = ctx.runtime.and_then(|r| r.index(RuntimeFn::IterUnwind)) {
                code.push(0x10);
                leb_u32(code, unwind);
            } else {
                global_set(code, ctx.iter_global);
            }
            Ok(Flow::Straight)
        }
        Op::LoadNow { dst } => {
            let idx = (ctx.host_index)(HostFn::Now)
                .ok_or(WasmLowerError::Unsupported("now not imported"))?;
            code.push(0x10); // call now -> i64 (Moment)
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        Op::GlobalSet { idx, src } => {
            // A width mismatch here is the VM's ownership clear: writing one element of a global
            // collection compiles to TAKE, MUTATE, PUT BACK, and the take's `nothing` arrives as an
            // `i64` zero aimed at the global's own width. It stores a null handle at that width, and
            // the global stops being a holder: its true count (S8) drops here and the put-back
            // restores it. The copy-on-write counter (word 12) is untouched by a recognised take
            // ([`PlaceSites`]) — the hold moves to the taken register and back, so a mutation between
            // them sees only real holders; a clear with no recognised take releases the retain its
            // read made.
            let want = ctx
                .global_valtypes
                .get(idx as usize)
                .copied()
                .unwrap_or(I64);
            if kinds.valtype(src as usize) != want {
                if want == I32 {
                    if !plan.place.clear.get(pc).copied().unwrap_or(false) {
                        // `if (g != 0) g.extra_refs -= 1` — a null global holds nothing to release, and
                        // word 12 of address 0 is not ours to write.
                        code.push(0x23); // global.get
                        leb_u32(code, idx as u32);
                        i32_const(code, 0);
                        code.push(0x47); // i32.ne
                        code.push(0x04); // if
                        code.push(0x40); // void
                        code.push(0x23);
                        leb_u32(code, idx as u32);
                        code.push(0x23);
                        leb_u32(code, idx as u32);
                        i32_load(code, 12);
                        i32_const(code, 0x100); // extra_refs occupies bits 8-31; class (bits 0-7) untouched
                        code.push(0x6B); // i32.sub
                        i32_store(code, 12);
                        code.push(0x0B); // end
                    }
                    emit_true_release(code, ctx, |c| {
                        c.push(0x23); // global.get idx
                        leb_u32(c, idx as u32);
                    });
                    i32_const(code, 0);
                } else if want == F64 {
                    code.push(0x44);
                    code.extend_from_slice(&0f64.to_le_bytes());
                } else {
                    code.push(0x42); // i64.const 0
                    code.push(0x00);
                }
                code.push(0x24); // global.set
                leb_u32(code, idx as u32);
                return Ok(Flow::Straight);
            }
            // Storing a heap handle into a global makes the global a second holder — RETAIN so a later
            // mutation through another register copy-on-writes rather than clobbering the global. A
            // put-back of what a take moved out is the same holder returning ([`PlaceSites`]).
            if counts_aliases(kinds.get(src as usize))
                && !plan.place.put_back.get(pc).copied().unwrap_or(false)
            {
                emit_retain(code, ctx, src);
            }
            // S8's true count: a DIRECT overwrite (`Set g to newValue`, no preceding take/mutate/
            // put-back null) never goes through the width-mismatch branch above, so its OWN release
            // of whatever `g` held lives here instead — before the store, so it reads the value
            // ABOUT to be replaced, not the new one.
            if want == I32 && counted_kind(kinds.get(src as usize)) {
                emit_true_release(code, ctx, |c| {
                    c.push(0x23);
                    leb_u32(c, idx as u32);
                });
            }
            local_get(code, src as u32);
            code.push(0x24); // global.set
            leb_u32(code, idx as u32);
            if counted_kind(kinds.get(src as usize)) {
                emit_true_retain(code, ctx, src);
            }
            Ok(Flow::Straight)
        }
        Op::Call {
            dst,
            func,
            args_start,
            arg_count,
        } => {
            if ctx.fn_stubbed.get(func as usize).copied().unwrap_or(false) {
                code.push(0x00); // unreachable — the reach analysis proved this call never runs
                return Ok(Flow::Straight);
            }
            // Value semantics: a heap argument gains the callee's parameter as a SECOND holder, so
            // RETAIN it (bump word 12) before the call. A mutation inside the callee then copy-on-writes
            // instead of clobbering the caller's value — the VM's `Rc`-clone-on-argument-bind. A `mutable`
            // parameter is no second holder: it is the caller's variable, lent for the call.
            let inout = ctx.fn_inout.get(func as usize).cloned().unwrap_or_default();
            for a in 0..arg_count {
                let arg = args_start + a;
                if shares_argument(plan, ctx, func, a, arg) && !inout.contains(&a) {
                    emit_retain(code, ctx, arg);
                }
            }
            let pvts = ctx
                .fn_param_valtypes
                .get(func as usize)
                .ok_or(WasmLowerError::Unsupported("call of unknown function"))?;
            for a in 0..arg_count {
                push_call_argument(code, plan, ctx, func, a, args_start + a)?;
            }
            code.push(0x10); // call
            leb_u32(code, ctx.fn_base + func as u32);
            // Each `mutable` parameter's final value sits above the declared result: it goes back into the
            // argument register it was lent from, last first.
            for p in inout.iter().rev() {
                let arg = args_start + p;
                if kinds.valtype(arg as usize) != pvts.get(*p as usize).copied().unwrap_or(I64) {
                    return Err(WasmLowerError::Unsupported(
                        "a `mutable` argument of another type than its parameter",
                    ));
                }
                local_set(code, arg as u32);
            }
            // A void callee leaves nothing on the stack; only bind a result when it returns one.
            if ctx
                .fn_results
                .get(func as usize)
                .copied()
                .flatten()
                .is_some()
            {
                local_set(code, dst as u32);
            }
            Ok(Flow::Straight)
        }
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Pow,
            args_start,
            arg_count,
        } => lower_pow(code, kinds, ctx, plan.num_regs, dst, args_start, arg_count),
        // `parseInt(text)` — a host call: push the Text handle, `call parse_int` → i64.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::ParseInt,
            args_start,
            ..
        } => {
            let idx = (ctx.host_index)(HostFn::ParseInt)
                .ok_or(WasmLowerError::Unsupported("parse_int host not imported"))?;
            local_get(code, args_start as u32);
            code.push(0x10); // call parse_int
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `contains(haystack, needle)` — push both Text handles (i32), `call text_contains` → i32 (0/1).
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Contains,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::TextContains, args_start, 2, true),
        // DOM: push the arg registers (Element handles i64, Text handles i32) and
        // call the `dom_*` host. Creators/getters return a value into `dst`; effect
        // ops set `dst` to Nothing (0).
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomBody,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomBody, 0, 0, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomCreateElement,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomCreateElement,
            args_start,
            1,
            true,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomCreateTextNode,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomCreateTextNode,
            args_start,
            1,
            true,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomQuerySelector,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomQuerySelector,
            args_start,
            1,
            true,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomGetText,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomGetText, args_start, 1, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomGetValue,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomGetValue, args_start, 1, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomSetText,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomSetText, args_start, 2, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomSetHtml,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomSetHtml, args_start, 2, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomClearChildren,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomClearChildren,
            args_start,
            1,
            false,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomOnMessage,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomOnMessage, args_start, 2, false),
        // `beginPaint` and `moveChildren` are retired: the front end refuses a call to either before
        // anything lowers, and their host imports are gone.
        Op::CallBuiltin {
            builtin: BuiltinId::DomBeginPaint,
            ..
        } => Err(WasmLowerError::Unsupported("beginPaint is retired")),
        Op::CallBuiltin {
            builtin: BuiltinId::DomMoveChildren,
            ..
        } => Err(WasmLowerError::Unsupported("moveChildren is retired")),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomAppendChild,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomAppendChild, args_start, 2, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomSetValue,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomSetValue, args_start, 2, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomSetChecked,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomSetChecked, args_start, 2, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomPreventDefault,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomPreventDefault,
            args_start,
            2,
            false,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::StorageSet,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::StorageSet, args_start, 3, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::StorageRemove,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::StorageRemove, args_start, 2, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::StorageGet,
            args_start,
            ..
        } => lower_storage_get(code, ctx, plan.num_regs, dst, args_start),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::StorageGetOr,
            args_start,
            ..
        } => lower_storage_get_or(code, ctx, plan.num_regs, dst, args_start),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomApplyPatches,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomApply, args_start, 1, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomApplyWords,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomApplyWords, args_start, 4, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomSetAttribute,
            args_start,
            ..
        } => {
            // A boolean HTML attribute (`disabled={not formValid}`) hands its raw `Bool` value
            // straight to this SAME builtin by design (`lgx_lower.rs`'s own comment) — `Bool` is
            // `i64` here, and `dom_set_attribute`'s ABI is fixed at `i32 Text`, so that specific
            // shape routes to `DomSetBoolAttribute` instead. See its own doc for why.
            let host = if kinds.get(args_start as usize + 2) == Some(Kind::Bool) {
                HostFn::DomSetBoolAttribute
            } else {
                HostFn::DomSetAttribute
            };
            lower_dom_call(code, ctx, dst, host, args_start, 3, false)
        }
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomAddEventListener,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomAddEventListener,
            args_start,
            3,
            false,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomListenFlags,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomListenFlags, args_start, 3, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomEnterValue,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomEnterValue, args_start, 3, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomDispatchEntry,
            args_start,
            ..
        } => lower_dispatch_entry(code, ctx, plan.num_regs, dst, args_start),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomEventValue,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomEventValue, args_start, 1, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomEventKey,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomEventKey, args_start, 1, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomEventTarget,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomEventTarget, args_start, 1, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomFetchText,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomFetchStart, args_start, 2, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomResponseText,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomResponseText, 0, 0, true),
        // The correlated fetch contract — the same push-args-and-call shape.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomFetchSend,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomFetchSend, args_start, 5, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomResponseStatus,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomResponseStatus,
            args_start,
            1,
            true,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomResponseBody,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomResponseBody, args_start, 1, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomResponseHeader,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomResponseHeader,
            args_start,
            2,
            true,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomResponseError,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomResponseError,
            args_start,
            1,
            true,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomFetchAbort,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomFetchAbort, args_start, 1, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomFetchSetTimeout,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomFetchSetTimeout,
            args_start,
            2,
            false,
        ),
        // The history/location router — the same push-args-and-call shape. path/query
        // return a Text handle into `dst`; navigate/replace/on_route are effects.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomRoutePath,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomRoutePath, 0, 0, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomRouteQuery,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomRouteQuery, args_start, 1, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomNavigate,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomNavigate, args_start, 1, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomAssignLocation,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomAssignLocation,
            args_start,
            1,
            false,
        ),
        // ★ THE USED-WALK ARM ABOVE AND THIS ONE MUST NAME THE SAME `HostFn`. A copy-paste that
        // notes one import and lowers another is invisible to every lock and calls the wrong host.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomSetTitle,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomSetTitle, args_start, 1, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomSetHeadHtml,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomSetHeadHtml, args_start, 1, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomReload,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomReload, args_start, 0, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomGoBack,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomGoBack, args_start, 0, false),
        // RETURNS A VALUE — the `true` here and `Kind::Text` in `kind.rs` must agree, or the
        // destination local is typed for a handle the call never leaves on the stack.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomGetAttribute,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomGetAttribute, args_start, 2, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomOnWindowEvent,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomOnWindowEvent,
            args_start,
            2,
            false,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomScrollGap,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomScrollGap, args_start, 1, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomSetInterval,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomSetInterval, args_start, 2, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomEventFileName,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomEventFileName,
            args_start,
            1,
            true,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomEventFileSize,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomEventFileSize,
            args_start,
            1,
            true,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomEventFileType,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomEventFileType,
            args_start,
            1,
            true,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomEventFileBytes,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomEventFileBytes,
            args_start,
            1,
            true,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomElementCommand,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomElementCommand,
            args_start,
            2,
            false,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomClipboardWrite,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomClipboardWrite,
            args_start,
            1,
            false,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomDownloadFile,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomDownloadFile,
            args_start,
            2,
            false,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomNotify,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomNotify, args_start, 2, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomPublish,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomPublish, args_start, 2, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::NoteTurn,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomNoteTurn, args_start, 4, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomJournalDetail,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomJournalDetail,
            args_start,
            0,
            true,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomWakeAfter,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomWakeAfter, args_start, 1, false),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::RandomInt,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::RandomInt, args_start, 2, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::EnvHas,
            args_start,
            ..
        } => lower_env_has(code, ctx, dst, args_start),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::EnvValue,
            args_start,
            ..
        } => lower_env_value(code, ctx, plan.num_regs, dst, args_start),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::TimeNow,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::TimeNow, args_start, 0, true),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomReplaceRoute,
            args_start,
            ..
        } => lower_dom_call(
            code,
            ctx,
            dst,
            HostFn::DomReplaceRoute,
            args_start,
            1,
            false,
        ),
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomOnRoute,
            args_start,
            ..
        } => lower_dom_call(code, ctx, dst, HostFn::DomOnRoute, args_start, 1, false),
        // The reactive-signal dialect is retired: these fifteen `BuiltinId`s are unreachable from
        // any program written today (`builtin_from_name` maps no spelling to them), kept only so
        // bincode does not renumber every `BuiltinId` after them (`builtin_wire_id_lock`). Their
        // WASM host imports are gone too, so lowering one refuses rather than looking one up.
        Op::CallBuiltin {
            builtin:
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
                | BuiltinId::SignalSetFloat,
            ..
        } => Err(WasmLowerError::Unsupported(
            "the reactive-signal dialect is retired",
        )),
        // `parseFloat(text) -> Float` — the host parses the `Text` handle (`str::parse::<f64>` after a
        // trim, matching the VM), returning the f64 directly.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::ParseFloat,
            args_start,
            ..
        } => {
            let idx = (ctx.host_index)(HostFn::ParseFloat)
                .ok_or(WasmLowerError::Unsupported("parse_float host not imported"))?;
            local_get(code, args_start as u32);
            code.push(0x10); // call parse_float
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `readReals(path) -> Seq of Real` — the host reads the file and returns the sequence's handle.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::ReadReals,
            args_start,
            ..
        } => {
            let idx = (ctx.host_index)(HostFn::ReadReals)
                .ok_or(WasmLowerError::Unsupported("read_reals host not imported"))?;
            local_get(code, args_start as u32);
            code.push(0x10); // call read_reals
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `chr(code) -> Text` — a one-character Text built inline from the Unicode scalar value
        // (`lower_chr`: UTF-8 encode + a fresh Text object), trapping on an invalid code point.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Chr,
            args_start,
            ..
        } => {
            lower_chr(code, ctx, plan.num_regs, dst, args_start);
            Ok(Flow::Straight)
        }
        // `parse_timestamp(text) -> Moment` — the host parses the RFC-3339 `Text` handle to nanoseconds.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::ParseTimestamp,
            args_start,
            ..
        } => {
            let idx = (ctx.host_index)(HostFn::ParseTimestamp).ok_or(
                WasmLowerError::Unsupported("parse_timestamp host not imported"),
            )?;
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
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::WriteWireResidual,
            args_start,
            ..
        } => {
            if kinds.get(args_start as usize) != Some(Kind::Text) {
                return Err(WasmLowerError::Unsupported(
                    "writeWireResidual requires a Text argument",
                ));
            }
            let idx = (ctx.host_index)(HostFn::WriteWireResidual).ok_or(
                WasmLowerError::Unsupported("write_wire_residual host not imported"),
            )?;
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
                    let idx = (ctx.host_index)(HostFn::TemporalComponent).ok_or(
                        WasmLowerError::Unsupported("temporal_component host not imported"),
                    )?;
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
                    if matches!(
                        b,
                        BuiltinId::HourOf | BuiltinId::MinuteOf | BuiltinId::SecondOf
                    ) {
                        return Err(WasmLowerError::Unsupported("clock component (hour/minute/second) of a Date — a Date has no time-of-day"));
                    }
                    let idx = (ctx.host_index)(HostFn::TemporalComponentDate).ok_or(
                        WasmLowerError::Unsupported("temporal_component_date host not imported"),
                    )?;
                    local_get(code, args_start as u32); // the Date (i32 days since epoch)
                    i32_const(code, which);
                    code.push(0x10); // call temporal_component_date
                    leb_u32(code, idx);
                    local_set(code, dst as u32);
                }
                _ => {
                    return Err(WasmLowerError::Unsupported(
                        "temporal component of a non-temporal value",
                    ));
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
            builtin:
                b @ (BuiltinId::SecondsBetween
                | BuiltinId::AddSeconds
                | BuiltinId::DateOf
                | BuiltinId::TimeOf),
            args_start,
            ..
        } => {
            const NANOS_PER_DAY: i64 = 86_400_000_000_000;
            const NANOS_PER_SEC: i64 = 1_000_000_000;
            if kinds.get(args_start as usize) != Some(Kind::Moment) {
                return Err(WasmLowerError::Unsupported(
                    "temporal arithmetic requires a Moment argument",
                ));
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
            builtin:
                b @ (BuiltinId::FormatTimestamp
                | BuiltinId::MonthsBetween
                | BuiltinId::YearsBetween
                | BuiltinId::InZone
                | BuiltinId::LocalInstant),
            args_start,
            ..
        } => {
            if !ctx.linked {
                return Err(WasmLowerError::Unsupported("format_timestamp/months_between/years_between/in_zone/local_instant need the linked runtime (base::temporal)"));
            }
            if kinds.get(args_start as usize) != Some(Kind::Moment) {
                return Err(WasmLowerError::Unsupported(
                    "extended temporal requires a Moment argument",
                ));
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
                return Err(WasmLowerError::Unsupported(
                    "in_zone/local_instant require a Text zone name",
                ));
            }
            let idx = (ctx.host_index)(rt).ok_or(WasmLowerError::Unsupported(
                "extended temporal runtime fn not imported",
            ))?;
            local_get(code, args_start as u32);
            if two_args {
                local_get(code, (args_start + 1) as u32);
            }
            code.push(0x10); // call the logos_rt_* runtime fn
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // Civil-component constructors (LINKER MODE): `makeDate(y,m,d)`→a `Date`, `makeMoment(y,m,d,h,mi,s)`
        // →a `Moment`. All arguments are `Int`s; the calendar core (`days_from_civil`) lives in
        // `base::temporal`, so a self-contained module refuses them (like `format_timestamp`/`months_between`),
        // never mis-lowering. Each is a single `logos_rt_make_*` call, so the tiers cannot drift.
        Op::CallBuiltin {
            dst,
            builtin: b @ (BuiltinId::MakeDate | BuiltinId::MakeMoment),
            args_start,
            ..
        } => {
            if !ctx.linked {
                return Err(WasmLowerError::Unsupported(
                    "makeDate/makeMoment need the linked runtime (base::temporal)",
                ));
            }
            let (rt, argc) = match b {
                BuiltinId::MakeDate => (HostFn::MakeDateRt, 3u32),
                BuiltinId::MakeMoment => (HostFn::MakeMomentRt, 6u32),
                _ => unreachable!(),
            };
            let idx = (ctx.host_index)(rt).ok_or(WasmLowerError::Unsupported(
                "date constructor runtime fn not imported",
            ))?;
            for i in 0..argc {
                local_get(code, args_start as u32 + i);
            }
            code.push(0x10); // call the logos_rt_make_* runtime fn
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // The general SIMD lane vocabulary (`base::LanesVal`, LINKER MODE): constructors (`lanes16Word8`/
        // `lanes8Word32`/`lanes4Word64` from a `Seq`, `splat16Word8`/`splat8Word32` from an `Int`) and the
        // extractors (`seqOfLanes16W8`/`seqOfLanes8` → a `Seq`) take one argument; the byte/word-lane ops
        // (`shuffle16`/`interleave*`/`byteAdd16`/`maddubs16`/`packus16`) and `shrBytes16` take two. Each is
        // a single `logos_rt_lanes_*` call delegating to the pure-Rust `base::word` spec.
        Op::CallBuiltin {
            dst,
            builtin: b,
            args_start,
            ..
        } if ctx.linked && lanes_v_host_fn(b).is_some() => {
            let rt = lanes_v_host_fn(b).expect("guarded by lanes_v_host_fn(b).is_some()");
            let idx = (ctx.host_index)(rt).ok_or(WasmLowerError::Unsupported(
                "lane-vector runtime fn not imported",
            ))?;
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
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::SetRate,
            args_start,
            ..
        } if ctx.linked => {
            let set_idx = (ctx.host_index)(HostFn::MoneySetRate).ok_or(
                WasmLowerError::Unsupported("set_rate runtime fn not imported"),
            )?;
            local_get(code, args_start as u32); // the currency-code Text handle
            match kinds.get((args_start + 1) as usize) {
                Some(Kind::Rational) => local_get(code, (args_start + 1) as u32),
                Some(Kind::Int) => {
                    let c = (ctx.host_index)(HostFn::RationalFromI64).ok_or(
                        WasmLowerError::Unsupported("rational_from_i64 not imported"),
                    )?;
                    local_get(code, (args_start + 1) as u32);
                    code.push(0x10);
                    leb_u32(code, c);
                }
                Some(Kind::Decimal) => {
                    let c = (ctx.host_index)(HostFn::DecimalToRational).ok_or(
                        WasmLowerError::Unsupported("decimal_to_rational not imported"),
                    )?;
                    local_get(code, (args_start + 1) as u32);
                    code.push(0x10);
                    leb_u32(code, c);
                }
                _ => {
                    return Err(WasmLowerError::Unsupported(
                        "set_rate rate must be an Int, Decimal, or Rational",
                    ))
                }
            }
            code.push(0x10); // call set_rate → 0 (Nothing)
            leb_u32(code, set_idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `to_currency(money, code)` — convert a `Money` into the named currency via the ambient rates.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::ToCurrency,
            args_start,
            ..
        } if ctx.linked => {
            let idx = (ctx.host_index)(HostFn::MoneyToCurrency).ok_or(
                WasmLowerError::Unsupported("to_currency runtime fn not imported"),
            )?;
            local_get(code, args_start as u32); // the Money handle
            local_get(code, (args_start + 1) as u32); // the currency-code Text handle
            code.push(0x10);
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `set_rates(map)` — install a whole `Map of <code> to <rate>`, dispatched on the map's VALUE kind
        // (resolved by scanning the region for the `SetIndex` that populated it), returning `Nothing` (0).
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::SetRates,
            args_start,
            ..
        } if ctx.linked => {
            let vk = set_rates_value_kind(&plan.ops, args_start, kinds);
            let rt = match vk {
                Some(Kind::Int) => HostFn::MoneySetRatesInt,
                Some(Kind::Rational) => HostFn::MoneySetRatesRational,
                Some(Kind::Decimal) => HostFn::MoneySetRatesDecimal,
                _ => return Err(WasmLowerError::Unsupported("set_rates needs a Map of currency code to an Int/Decimal/Rational rate whose value kind is statically known")),
            };
            let idx = (ctx.host_index)(rt).ok_or(WasmLowerError::Unsupported(
                "set_rates runtime fn not imported",
            ))?;
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
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::WireBytes,
            args_start,
            ..
        } if ctx.linked => {
            let h = wire_bytes_host_fn(kinds.get(args_start as usize)).ok_or(
                WasmLowerError::Unsupported(
                    "wireBytes of this value kind is not yet reconstructed for the wire codec",
                ),
            )?;
            let idx = (ctx.host_index)(h).ok_or(WasmLowerError::Unsupported(
                "wire_bytes runtime fn not imported",
            ))?;
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
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::ReadWireProgram,
            ..
        } if ctx.linked => {
            const WIRE_BUF: i32 = 1 << 16;
            let frame = (ctx.host_index)(HostFn::ReadWireFrame).ok_or(
                WasmLowerError::Unsupported("read_wire_frame host not imported"),
            )?;
            let decode = (ctx.host_index)(HostFn::ReadWireProgramRt).ok_or(
                WasmLowerError::Unsupported("read_wire_program runtime fn not imported"),
            )?;
            let buf = plan.num_regs + 8; // an i32 heap scratch local (num_regs+5..+11)
            i32_const(code, WIRE_BUF);
            emit_alloc(code, ctx, buf, AllocClass::Raw); // buf = bump-alloc(WIRE_BUF)
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
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::RunAccepted,
            args_start,
            ..
        } if ctx.linked => {
            if kinds.get(args_start as usize) != Some(Kind::Dynamic) {
                return Err(WasmLowerError::Unsupported(
                    "run_accepted requires a wire-received (dynamic) shipped function",
                ));
            }
            let idx = (ctx.host_index)(HostFn::RunAccepted).ok_or(WasmLowerError::Unsupported(
                "run_accepted runtime fn not imported",
            ))?;
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
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Format,
            args_start,
            arg_count,
        } => {
            if arg_count == 0 {
                lower_text_literal(code, ctx, plan.num_regs, b""); // leaves the handle on the stack
                local_set(code, dst as u32);
            } else {
                // `emit_stringify` writes the Text handle straight into the `dst` local.
                display::emit_display_of(code, plan, ctx, args_start, dst as u32)?;
            }
            Ok(Flow::Straight)
        }
        // `Fail with <value>.` / a broken contract's `, otherwise <reason>`: hand the host the reason as
        // Text, then trap — the report every other tier gives. A Text reason goes as-is; `Fail with`
        // stringifies any other value exactly as `format` would; a contract reason that is not Text is
        // refused with the same sentence the other tiers use (`semantics::builtins::contract_reason_text`).
        Op::CallBuiltin {
            builtin: b @ (BuiltinId::Fail | BuiltinId::FailReason),
            args_start,
            ..
        } => {
            let fail = (ctx.host_index)(HostFn::LogosRtFail).ok_or(WasmLowerError::Unsupported(
                "logos_rt_fail host not imported",
            ))?;
            let kind = kinds.get(args_start as usize);
            if kind == Some(Kind::Text) {
                local_get(code, args_start as u32);
            } else if b == BuiltinId::Fail {
                // The handle is an i32; `dst` is typed by its own kind, so the rendering goes through an
                // i32 heap scratch local (+8 — `emit_stringify` itself uses +5..+7).
                let text = plan.num_regs + 8;
                display::emit_display_of(code, plan, ctx, args_start, text)?;
                local_get(code, text);
            } else {
                let type_name = match kind {
                    Some(Kind::Int) => "Int",
                    Some(Kind::Float) => "Real",
                    Some(Kind::Bool) => "Bool",
                    _ => {
                        return Err(WasmLowerError::Unsupported(
                            "a contract reason of this kind",
                        ))
                    }
                };
                let refusal =
                    format!("The reason after `otherwise` must be Text, but it was {type_name}.");
                lower_text_literal(code, ctx, plan.num_regs, refusal.as_bytes());
            }
            code.push(0x10); // call logos_rt_fail
            leb_u32(code, fail);
            code.push(0x00); // unreachable → trap
            Ok(Flow::Terminated)
        }
        // `Mount … at …` / `Write … to file …`: the tree-walker and the VM run them against the sandboxed
        // virtual file system; a page in the browser has none, so the turn fails with the sentence that
        // names what is missing (`VFS_IN_THE_BROWSER`), and the program still builds.
        Op::CallBuiltin {
            builtin: BuiltinId::VfsMount | BuiltinId::VfsWriteFile,
            ..
        } => {
            let fail = (ctx.host_index)(HostFn::LogosRtFail).ok_or(WasmLowerError::Unsupported(
                "logos_rt_fail host not imported",
            ))?;
            lower_text_literal(code, ctx, plan.num_regs, VFS_IN_THE_BROWSER.as_bytes());
            code.push(0x10); // call logos_rt_fail
            leb_u32(code, fail);
            code.push(0x00); // unreachable → trap
            Ok(Flow::Terminated)
        }
        // `repeatSeq(x, n)` — a fresh `n`-element sequence of the scalar `x` (`[x] * n`).
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::RepeatSeq,
            args_start,
            arg_count,
        } => {
            if arg_count != 2 {
                return Err(WasmLowerError::Unsupported("repeatSeq arity"));
            }
            lower_repeat_seq(code, kinds, ctx, plan.num_regs, dst, args_start)?;
            Ok(Flow::Straight)
        }
        // `money(amount, "USD")` — build an EXACT Money handle. `amount` is a Decimal handle or an
        // Int (whole units); the currency is a Text code resolved by `currency::by_code` in the runtime.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Money,
            args_start,
            arg_count,
        } if ctx.linked && arg_count == 2 => {
            if kinds.get((args_start + 1) as usize) != Some(Kind::Text) {
                return Err(WasmLowerError::Unsupported(
                    "money(amount, currency) with a non-Text currency",
                ));
            }
            let host = match kinds.get(args_start as usize) {
                Some(Kind::Decimal) => HostFn::MoneyFromDecimal,
                Some(Kind::Int) => HostFn::MoneyFromI64,
                _ => {
                    return Err(WasmLowerError::Unsupported(
                        "money amount must be an Int or Decimal",
                    ))
                }
            };
            let make = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported(
                "money constructor not imported",
            ))?;
            local_get(code, args_start as u32);
            local_get(code, (args_start + 1) as u32);
            code.push(0x10);
            leb_u32(code, make);
            local_set(code, dst as u32);
            return Ok(Flow::Straight);
        }
        // `quantity(v, "unit")` — build an EXACT Quantity handle: an Int magnitude + a unit name Text
        // resolved by `units::by_name` in the runtime (the `5 meters` literal lowers to this too).
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Quantity,
            args_start,
            arg_count,
        } if ctx.linked && arg_count == 2 => {
            if kinds.get((args_start + 1) as usize) != Some(Kind::Text) {
                return Err(WasmLowerError::Unsupported(
                    "quantity(value, unit) with a non-Text unit",
                ));
            }
            if kinds.get(args_start as usize) != Some(Kind::Int) {
                return Err(WasmLowerError::Unsupported(
                    "quantity magnitude must be an Int",
                ));
            }
            let make = (ctx.host_index)(HostFn::QuantityOfI64).ok_or(
                WasmLowerError::Unsupported("quantity constructor not imported"),
            )?;
            local_get(code, args_start as u32);
            local_get(code, (args_start + 1) as u32);
            code.push(0x10);
            leb_u32(code, make);
            local_set(code, dst as u32);
            return Ok(Flow::Straight);
        }
        // `convert(q, "unit")` (the surface `X in <unit>`) — re-express a Quantity in a new display unit
        // of the SAME dimension (dimension-checked at compile time); the runtime keeps the SI magnitude.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Convert,
            args_start,
            arg_count,
        } if ctx.linked && arg_count == 2 => {
            if kinds.get(args_start as usize) != Some(Kind::Quantity) {
                return Err(WasmLowerError::Unsupported("convert() requires a Quantity"));
            }
            if kinds.get((args_start + 1) as usize) != Some(Kind::Text) {
                return Err(WasmLowerError::Unsupported(
                    "convert(q, unit) with a non-Text unit",
                ));
            }
            let make = (ctx.host_index)(HostFn::QuantityConvert)
                .ok_or(WasmLowerError::Unsupported("quantity convert not imported"))?;
            local_get(code, args_start as u32);
            local_get(code, (args_start + 1) as u32);
            code.push(0x10);
            leb_u32(code, make);
            local_set(code, dst as u32);
            return Ok(Flow::Straight);
        }
        // `decimal("…")` — parse a Text arg into an exact Decimal via the runtime.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Decimal,
            args_start,
            arg_count,
        } if ctx.linked && arg_count == 1 => {
            if kinds.get(args_start as usize) != Some(Kind::Text) {
                return Err(WasmLowerError::Unsupported(
                    "decimal(x) with a non-Text argument",
                ));
            }
            let from = (ctx.host_index)(HostFn::DecimalFromText).ok_or(
                WasmLowerError::Unsupported("decimal_from_text not imported"),
            )?;
            local_get(code, args_start as u32);
            code.push(0x10);
            leb_u32(code, from);
            local_set(code, dst as u32);
            return Ok(Flow::Straight);
        }
        // `complex(re, im)` — build an EXACT Complex handle from two Int components via the runtime.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Modular,
            args_start,
            arg_count,
        } if ctx.linked && arg_count == 2 => {
            if kinds.get(args_start as usize) != Some(Kind::Int)
                || kinds.get((args_start + 1) as usize) != Some(Kind::Int)
            {
                return Err(WasmLowerError::Unsupported(
                    "modular(v, n) with non-Int components",
                ));
            }
            let from = (ctx.host_index)(HostFn::ModularFromI64)
                .ok_or(WasmLowerError::Unsupported("modular_from_i64 not imported"))?;
            local_get(code, args_start as u32);
            local_get(code, (args_start + 1) as u32);
            code.push(0x10);
            leb_u32(code, from);
            local_set(code, dst as u32);
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Complex,
            args_start,
            arg_count,
        } if ctx.linked && arg_count == 2 => {
            let re_k = kinds.get(args_start as usize);
            let im_k = kinds.get((args_start + 1) as usize);
            if re_k == Some(Kind::Int) && im_k == Some(Kind::Int) {
                let from = (ctx.host_index)(HostFn::ComplexFromI64)
                    .ok_or(WasmLowerError::Unsupported("complex_from_i64 not imported"))?;
                local_get(code, args_start as u32);
                local_get(code, (args_start + 1) as u32);
                code.push(0x10); // call logos_rt_complex_from_i64
                leb_u32(code, from);
                local_set(code, dst as u32);
                return Ok(Flow::Straight);
            }
            // Float-part complex (a `ComplexF`) → a dyadic-Rational handle via `logos_rt_complex_from_f64`.
            // Each part is loaded as an f64: a Float part directly, an Int part converted (`f64.convert_i64_s`).
            if !matches!(re_k, Some(Kind::Int) | Some(Kind::Float))
                || !matches!(im_k, Some(Kind::Int) | Some(Kind::Float))
            {
                return Err(WasmLowerError::Unsupported(
                    "complex(re, im) with non-numeric components",
                ));
            }
            let from = (ctx.host_index)(HostFn::ComplexFromF64)
                .ok_or(WasmLowerError::Unsupported("complex_from_f64 not imported"))?;
            local_get(code, args_start as u32);
            if re_k == Some(Kind::Int) {
                code.push(0xB9);
            } // f64.convert_i64_s
            local_get(code, (args_start + 1) as u32);
            if im_k == Some(Kind::Int) {
                code.push(0xB9);
            } // f64.convert_i64_s
            code.push(0x10); // call logos_rt_complex_from_f64
            leb_u32(code, from);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `abs` of a LINKED `Complex`: its `f64` modulus `|z|` via `logos_rt_complex_abs` (the one
        // complex op returning a Float, not a handle). Handled here (not `lower_builtin`) for `ctx`.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Abs,
            args_start,
            ..
        } if ctx.linked && kinds.get(args_start as usize) == Some(Kind::Complex) => {
            let host = (ctx.host_index)(HostFn::ComplexAbs)
                .ok_or(WasmLowerError::Unsupported("complex_abs not imported"))?;
            local_get(code, args_start as u32);
            code.push(0x10); // call logos_rt_complex_abs
            leb_u32(code, host);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `re(z)` / `im(z)` of a LINKED Complex → its real / imaginary part as an `f64` (the dyadic
        // parts round-trip). The float-complex tier's extraction; both return a Float, not a handle.
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::RealPart,
            args_start,
            ..
        } if ctx.linked && kinds.get(args_start as usize) == Some(Kind::Complex) => {
            let host = (ctx.host_index)(HostFn::ComplexReF64)
                .ok_or(WasmLowerError::Unsupported("complex_re_f64 not imported"))?;
            local_get(code, args_start as u32);
            code.push(0x10); // call logos_rt_complex_re_f64
            leb_u32(code, host);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::ImagPart,
            args_start,
            ..
        } if ctx.linked && kinds.get(args_start as usize) == Some(Kind::Complex) => {
            let host = (ctx.host_index)(HostFn::ComplexImF64)
                .ok_or(WasmLowerError::Unsupported("complex_im_f64 not imported"))?;
            local_get(code, args_start as u32);
            code.push(0x10); // call logos_rt_complex_im_f64
            leb_u32(code, host);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        // `floor`/`ceil`/`round`/`abs` of a LINKED `Rational`: EXACT rounding on the BigInt-backed
        // fraction (`logos_rt_rational_*`) — floor/ceil/round yield a BigInt handle, abs a Rational one —
        // never the lossy `f64` path. Handled here (not `lower_builtin`) for the `ctx` host table.
        Op::CallBuiltin {
            dst,
            builtin: b @ (BuiltinId::Floor | BuiltinId::Ceil | BuiltinId::Round | BuiltinId::Abs),
            args_start,
            ..
        } if ctx.linked && kinds.get(args_start as usize) == Some(Kind::Rational) => {
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
        Op::CallBuiltin {
            dst,
            builtin:
                b @ (BuiltinId::Uuid
                | BuiltinId::UuidNil
                | BuiltinId::UuidMax
                | BuiltinId::UuidDns
                | BuiltinId::UuidUrl
                | BuiltinId::UuidOid
                | BuiltinId::UuidX500
                | BuiltinId::UuidVersion),
            args_start,
            ..
        } if ctx.linked => {
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
            let idx = (ctx.host_index)(host)
                .ok_or(WasmLowerError::Unsupported("uuid builtin not imported"))?;
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
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::TextBytes,
            args_start,
            ..
        } => {
            lower_text_bytes(code, ctx, plan.num_regs, dst, args_start);
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::UuidBytes,
            args_start,
            ..
        } if ctx.linked => {
            lower_uuid_bytes(code, ctx, plan.num_regs, dst, args_start);
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::TextFromBytes,
            args_start,
            ..
        } => {
            lower_text_from_bytes(code, ctx, plan.num_regs, dst, args_start)?;
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::UuidFromBytes,
            args_start,
            ..
        } if ctx.linked => {
            return lower_uuid_from_bytes(code, ctx, plan.num_regs, dst, args_start);
        }
        // The SHA-1 SHA-NI lane vocabulary (linker): construction/unpack are inline, the four rounds call
        // the `logos_rt_sha1*` runtime (which delegates to `base::sha_ops`).
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Lanes4Of,
            args_start,
            ..
        } if ctx.linked => {
            lower_lanes4_of(
                code,
                ctx,
                plan.num_regs,
                dst,
                [args_start, args_start + 1, args_start + 2, args_start + 3],
            );
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::Lanes4Word32Make,
            args_start,
            ..
        } if ctx.linked => {
            lower_lanes4_word32(code, ctx, plan.num_regs, dst, args_start);
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::SeqOfLanes4W32,
            args_start,
            ..
        } if ctx.linked => {
            lower_seq_of_lanes4(code, ctx, plan.num_regs, dst, args_start);
            return Ok(Flow::Straight);
        }
        Op::CallBuiltin {
            dst,
            builtin:
                b @ (BuiltinId::Sha1Rnds4
                | BuiltinId::Sha1Msg1
                | BuiltinId::Sha1Msg2
                | BuiltinId::Sha1Nexte),
            args_start,
            ..
        } if ctx.linked => {
            let (host, ternary) = match b {
                BuiltinId::Sha1Rnds4 => (HostFn::Sha1Rnds4, true),
                BuiltinId::Sha1Msg1 => (HostFn::Sha1Msg1, false),
                BuiltinId::Sha1Msg2 => (HostFn::Sha1Msg2, false),
                _ => (HostFn::Sha1Nexte, false),
            };
            return lower_sha1_op(code, ctx, dst, args_start, host, ternary);
        }
        Op::CallBuiltin {
            dst,
            builtin,
            args_start,
            arg_count,
        } => lower_builtin(code, kinds, dst, builtin, args_start, arg_count),
        // `args()` — the host returns the argv `Seq of Text` handle (built in this module's memory).
        Op::Args { dst } => {
            let idx = (ctx.host_index)(HostFn::Args)
                .ok_or(WasmLowerError::Unsupported("args host not imported"))?;
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
            let kind = kinds
                .get(src as usize)
                .ok_or(WasmLowerError::Unsupported("Show of an unknown-kind value"))?;
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
            // the self-contained value loads its scalar buffer (num@0, den@8) and hands it to the
            // `print_rational` host, which renders `num/den` (or `num` when `den == 1`).
            if kind == Kind::Rational {
                if ctx.linked {
                    let to_text = (ctx.host_index)(HostFn::RationalToText)
                        .ok_or(WasmLowerError::Unsupported("rational_to_text not imported"))?;
                    let print_text = (ctx.host_index)(HostFn::PrintText)
                        .ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                    local_get(code, src as u32);
                    code.push(0x10);
                    leb_u32(code, to_text);
                    code.push(0x10);
                    leb_u32(code, print_text);
                    return Ok(Flow::Straight);
                }
                let idx = (ctx.host_index)(HostFn::PrintRational)
                    .ok_or(WasmLowerError::Unsupported("Show sink not imported"))?;
                local_get(code, src as u32);
                i32_load(code, 8);
                i64_load(code, 0);
                local_get(code, src as u32);
                i32_load(code, 8);
                i64_load(code, 8);
                code.push(0x10); // call print_rational
                leb_u32(code, idx);
                return Ok(Flow::Straight);
            }
            // A `BigInt` handle (linker mode): render it to a decimal `Text` now
            // (`logos_rt_bigint_to_text` — deferred until here so a Pow/`*` chain could keep computing on
            // real BigInts) and print that Text.
            if kind == Kind::BigInt {
                let to_text = (ctx.host_index)(HostFn::BigintToText)
                    .ok_or(WasmLowerError::Unsupported("bigint_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText)
                    .ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32); // i32 BigInt handle
                code.push(0x10);
                leb_u32(code, to_text); // -> i32 Text handle
                code.push(0x10);
                leb_u32(code, print_text); // print the decimal
                return Ok(Flow::Straight);
            }
            // A `Complex` handle: render `re±imi` to a Text via the runtime, then print it.
            if kind == Kind::Complex {
                let to_text = (ctx.host_index)(HostFn::ComplexToText)
                    .ok_or(WasmLowerError::Unsupported("complex_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText)
                    .ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32); // i32 Complex handle
                code.push(0x10);
                leb_u32(code, to_text); // -> i32 Text handle
                code.push(0x10);
                leb_u32(code, print_text);
                return Ok(Flow::Straight);
            }
            // A `Modular` handle: render `v (mod n)` via the runtime, then print it.
            if kind == Kind::Modular {
                let to_text = (ctx.host_index)(HostFn::ModularToText)
                    .ok_or(WasmLowerError::Unsupported("modular_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText)
                    .ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32);
                code.push(0x10);
                leb_u32(code, to_text);
                code.push(0x10);
                leb_u32(code, print_text);
                return Ok(Flow::Straight);
            }
            if kind == Kind::Decimal {
                let to_text = (ctx.host_index)(HostFn::DecimalToText)
                    .ok_or(WasmLowerError::Unsupported("decimal_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText)
                    .ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32);
                code.push(0x10);
                leb_u32(code, to_text);
                code.push(0x10);
                leb_u32(code, print_text);
                return Ok(Flow::Straight);
            }
            if kind == Kind::Money {
                let to_text = (ctx.host_index)(HostFn::MoneyToText)
                    .ok_or(WasmLowerError::Unsupported("money_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText)
                    .ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
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
                let to_text = (ctx.host_index)(HostFn::QuantityToText)
                    .ok_or(WasmLowerError::Unsupported("quantity_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText)
                    .ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32);
                code.push(0x10);
                leb_u32(code, to_text);
                code.push(0x10);
                leb_u32(code, print_text);
                return Ok(Flow::Straight);
            }
            // A `Uuid` handle: render the canonical lowercase form via `logos_rt_uuid_to_text`, then print.
            if kind == Kind::Uuid {
                let to_text = (ctx.host_index)(HostFn::UuidToText)
                    .ok_or(WasmLowerError::Unsupported("uuid_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText)
                    .ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
                local_get(code, src as u32);
                code.push(0x10);
                leb_u32(code, to_text);
                code.push(0x10);
                leb_u32(code, print_text);
                return Ok(Flow::Straight);
            }
            // A wire-decoded DYNAMIC value: render its boxed `RuntimeValue` via `to_display_string`, print.
            if kind == Kind::Dynamic {
                let to_text = (ctx.host_index)(HostFn::DynamicToText)
                    .ok_or(WasmLowerError::Unsupported("dynamic_to_text not imported"))?;
                let print_text = (ctx.host_index)(HostFn::PrintText)
                    .ok_or(WasmLowerError::Unsupported("print_text not imported"))?;
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
                let nothing = (ctx.host_index)(HostFn::PrintNothing)
                    .ok_or(WasmLowerError::Unsupported("Show sink not imported"))?;
                let inner = plan
                    .structs
                    .opt_inner
                    .get(&src)
                    .copied()
                    .unwrap_or(Kind::Int);
                let some_host = HostFn::for_show(inner).ok_or(WasmLowerError::Unsupported(
                    "Show of an optional with a non-scalar inner",
                ))?;
                let some_idx = (ctx.host_index)(some_host)
                    .ok_or(WasmLowerError::Unsupported("Show sink not imported"))?;
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
                let idx = (ctx.host_index)(HostFn::PrintWord)
                    .ok_or(WasmLowerError::Unsupported("Show sink not imported"))?;
                local_get(code, src as u32);
                if kind == Kind::Word32 {
                    code.push(0xAD); // i64.extend_i32_u
                }
                code.push(0x10); // call print_word
                leb_u32(code, idx);
                return Ok(Flow::Straight);
            }
            let host = HostFn::for_show(kind)
                .ok_or(WasmLowerError::Unsupported("Show of a non-scalar value"))?;
            let idx = (ctx.host_index)(host)
                .ok_or(WasmLowerError::Unsupported("Show sink not imported"))?;
            if kind != Kind::Nothing {
                local_get(code, src as u32);
            }
            if kind == Kind::Bool {
                code.push(0xA7); // i32.wrap_i64 — print_bool takes an i32
            }
            code.push(0x10); // call print_*
            leb_u32(code, idx);
            Ok(Flow::Straight)
        }
        Op::Return { src } => {
            if plan.result.is_none() {
                return Err(WasmLowerError::Unsupported(
                    "value return from a void function",
                ));
            }
            emit_leave_loops(code, plan, ctx);
            local_get(code, src as u32);
            push_inout_results(code, plan);
            code.push(0x0F); // return
            Ok(Flow::Terminated)
        }
        Op::ReturnNothing => {
            match plan.result {
                None => {
                    emit_leave_loops(code, plan, ctx);
                    push_inout_results(code, plan);
                    code.push(0x0F); // return (void)
                }
                Some(_) => code.push(0x00), // unreachable (typed function: never returns nothing)
            }
            Ok(Flow::Terminated)
        }
        Op::Halt => {
            emit_leave_loops(code, plan, ctx);
            if ctx.batch_reclaim {
                code.push(0x10);
                leb_u32(
                    code,
                    ctx.runtime
                        .and_then(|r| r.index(RuntimeFn::Safepoint))
                        .expect("batch safepoint"),
                );
            }
            code.push(0x0F); // return — Main is void
            Ok(Flow::Terminated)
        }
        // A runtime failure (`FailWith`, e.g. a bad index or an explicit fail). The module still
        // TRAPS — that is the documented error contract, and `wasm_traps_where_treewalker_errors`
        // proves tw-errors ⟺ wasm-traps — but it now hands the host the message FIRST.
        //
        // ★ THE MESSAGE WAS AUTHORED AND THEN THROWN AWAY. The compiler writes real prose for
        // these ("no element matches that selector. Create the mount point before mounting…"),
        // interns it as a `Constant::Text`, and names it in the op. The tree-walker prints it and
        // the VM returns it verbatim; this backend used to drop it, so on the one tier a visitor
        // actually runs, every failure surfaced as `RuntimeError: unreachable` — no message, no
        // line, no source.
        //
        // The call goes BEFORE the `unreachable`, never instead of it: a host that returned
        // normally where the trap used to be would turn a failed `Check` into a program that
        // silently runs on.
        Op::FailWith { msg } => {
            if let Some(Constant::Text(text)) = ctx.constants.get(msg as usize) {
                let idx = (ctx.host_index)(HostFn::LogosRtFail).ok_or(
                    WasmLowerError::Unsupported("logos_rt_fail host not imported"),
                )?;
                lower_text_literal(code, ctx, plan.num_regs, text.as_bytes());
                code.push(0x10); // call logos_rt_fail
                leb_u32(code, idx);
            }
            code.push(0x00); // unreachable → trap
            Ok(Flow::Terminated)
        }
        // Observation (Wave 13, OBSERVED profile): call
        // `obs(site_id, p0, p1, p2)`. The host has the module's
        // `observation_sites`, so it resolves the category / message / span kind
        // from the site index; the three i64 args carry the observation's scalar
        // Int payload VALUES (a non-Int or absent slot is a zero placeholder —
        // rich-type payload rendering across the WASM FFI is a follow-up, and
        // those payloads emit in full on the tree-walker / VM / AOT tiers).
        Op::Obs {
            site,
            args_start,
            args_count,
        } => {
            let idx = (ctx.host_index)(HostFn::Obs)
                .ok_or(WasmLowerError::Unsupported("obs host import not present"))?;
            i32_const(
                code,
                i32::try_from(site).expect("an observation site index fits an i32"),
            );
            for slot in 0..3usize {
                let reg = args_start as usize + slot;
                if slot < args_count as usize && kinds.get(reg) == Some(Kind::Int) {
                    local_get(code, reg as u32); // the payload's i64 value
                } else {
                    code.push(0x42); // i64.const 0 (absent / non-scalar slot)
                    code.push(0x00);
                }
            }
            code.push(0x10); // call obs
            leb_u32(code, idx);
            Ok(Flow::Straight)
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
fn lower_arith(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    kinds: &KindTable,
    dst: u16,
    lhs: u16,
    rhs: u16,
    op: ArithOp,
) -> R<Flow> {
    match kinds.get(dst as usize) {
        Some(Kind::Int) => {
            // Integer arithmetic needs two `i64` operands; a Float operand with an `Int` result is a
            // kind inconsistency — reject rather than emit an `i64` op on an `f64` (invalid wasm).
            if kinds.valtype(lhs as usize) != I64 || kinds.valtype(rhs as usize) != I64 {
                return Err(WasmLowerError::Unsupported(
                    "integer arithmetic with a non-integer operand",
                ));
            }
            let helper = match op {
                ArithOp::Add => Some(RuntimeFn::AddChecked),
                ArithOp::Sub => Some(RuntimeFn::SubChecked),
                ArithOp::Mul => Some(RuntimeFn::MulChecked),
                ArithOp::Div | ArithOp::Mod => None,
            };
            // Outside a loop ([`Ctx::hot`]) the overflow check is the runtime's; inside one it stays
            // inline, where a call would be paid per iteration.
            let call = helper
                .filter(|_| !ctx.hot.get())
                .and_then(|f| ctx.runtime.and_then(|r| r.index(f)));
            if let Some(f) = call {
                local_get(code, lhs as u32);
                local_get(code, rhs as u32);
                code.push(0x10);
                leb_u32(code, f);
                local_set(code, dst as u32);
                return Ok(Flow::Straight);
            }
            match op {
                ArithOp::Add => emit_checked_addsub(code, false, dst, lhs, rhs),
                ArithOp::Sub => emit_checked_addsub(code, true, dst, lhs, rhs),
                ArithOp::Mul => emit_checked_mul(code, dst, lhs, rhs),
                // `i64.div_s`/`rem_s` TRAP on a zero divisor all by themselves, so the program
                // already stopped correctly — it just stopped anonymously. Reporting first costs a
                // compare and a well-predicted branch in front of an instruction that takes tens of
                // cycles, which is noise, and buys the reference tier's own sentence instead of a
                // bare trap. (`i64::MIN / -1` still traps unreported: the wording differs there and
                // guessing at it would be worse than saying nothing.)
                ArithOp::Div | ArithOp::Mod => {
                    if let Some(fail_at) = (ctx.host_index)(HostFn::LogosRtFailAt) {
                        local_get(code, rhs as u32);
                        code.push(0x50); // i64.eqz
                        code.push(0x04);
                        code.push(0x40); // if
                        i32_const(
                            code,
                            crate::semantics::failcode::FailCode::DivisionByZero.code(),
                        );
                        code.push(0x42); // i64.const — this failure needs no value
                        leb_i64(code, 0);
                        code.push(0x10); // call logos_rt_fail_at(code, value)
                        leb_u32(code, fail_at);
                        code.push(0x00); // unreachable
                        code.push(0x0B); // end
                    }
                    let opcode = if matches!(op, ArithOp::Div) {
                        0x7F
                    } else {
                        0x81
                    };
                    arith(code, opcode, dst, lhs, rhs)
                }
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
            // promotion), so `3 + 1.5` etc. compile instead of emitting an `f64` op on an `i64`; a rational
            // operand is the double the VM computes it as.
            push_number_as_f64(code, ctx, lhs, kinds.get(lhs as usize))?;
            push_number_as_f64(code, ctx, rhs, kinds.get(rhs as usize))?;
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
                _ => {
                    return Err(WasmLowerError::Unsupported(
                        "only + and - on temporal values",
                    ))
                }
            };
            arith(code, opcode, dst, lhs, rhs);
        }
        _ => {
            return Err(WasmLowerError::Unsupported(
                "arithmetic on a non-numeric value",
            ))
        }
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
fn lower_compare(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    kinds: &KindTable,
    dst: u16,
    lhs: u16,
    rhs: u16,
    cmp: Cmp,
) -> R<Flow> {
    let (lk, rk) = (kinds.get(lhs as usize), kinds.get(rhs as usize));
    if ctx.linked && (lk == Some(Kind::Rational) || rk == Some(Kind::Rational)) {
        return lower_rational_compare(code, kinds, ctx, dst, lhs, rhs, cmp);
    }
    if lk == Some(Kind::Nothing) || rk == Some(Kind::Nothing) {
        if !matches!(cmp, Cmp::Eq | Cmp::Ne) {
            return Err(WasmLowerError::Unsupported(
                "ordering comparison with nothing",
            ));
        }
        let other = if lk == Some(Kind::Nothing) { rhs } else { lhs };
        let other_kind = kinds
            .get(other as usize)
            .ok_or(WasmLowerError::Unsupported(
                "comparison of unknown-kind values",
            ))?;
        if other_kind == Kind::Optional {
            local_get(code, other as u32);
            code.push(0x45); // i32.eqz — the optional box is absent
        } else {
            i32_const(code, i32::from(other_kind == Kind::Nothing));
        }
        if matches!(cmp, Cmp::Ne) {
            code.push(0x45); // invert equality
        }
        code.push(0xAD); // i64.extend_i32_u
        local_set(code, dst as u32);
        return Ok(Flow::Straight);
    }
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
    if lk == Some(Kind::Optional) || rk == Some(Kind::Optional) {
        return Err(WasmLowerError::Unsupported(
            "comparison of present optional payloads requires their inner value kinds",
        ));
    }
    let operand = kinds.get(lhs as usize).or_else(|| kinds.get(rhs as usize));
    match operand {
        Some(Kind::Nothing) => unreachable!("absence comparisons handled above"),
        // A `Char` compares by code point (`char`'s own ordering), so an `i64` compare of the
        // stored `char as u32` is byte-identical to the VM's `Char` comparison.
        Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Char) | Some(Kind::Duration)
        | Some(Kind::Time) | Some(Kind::Span) => {
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
        Some(Kind::SeqInt)
        | Some(Kind::SeqBool)
        | Some(Kind::SeqFloat)
        | Some(Kind::SeqText)
        | Some(Kind::SeqStruct)
        | Some(Kind::SeqEnum)
        | Some(Kind::SeqSeqInt)
        | Some(Kind::SeqHandle)
        | Some(Kind::SeqAny)
        | Some(Kind::SeqWord32)
        | Some(Kind::SeqWord64) => {
            return Err(WasmLowerError::Unsupported("comparison of sequences"))
        }
        Some(Kind::Text) => return Err(WasmLowerError::Unsupported("comparison of text values")),
        Some(Kind::Tower) => {
            return Err(WasmLowerError::Unsupported(
                "comparison of an exact value outside the value runtime",
            ))
        }
        Some(Kind::Struct) => {
            return Err(WasmLowerError::Unsupported("comparison of struct values"))
        }
        Some(Kind::Map) => return Err(WasmLowerError::Unsupported("comparison of map values")),
        Some(Kind::Set) | Some(Kind::SetText) | Some(Kind::CrdtSetText) => {
            return Err(WasmLowerError::Unsupported("comparison of set values"))
        }
        // A raw counter object never reaches a compare — `obj's counter` projects to its Int total at
        // GetField, so a relational test sees the `Int`. A direct compare of the counter object is
        // soundly refused (the native AOT / VM compare counters as their value; the WASM tier does so
        // only through the projected Int).
        Some(Kind::CrdtCounter) => {
            return Err(WasmLowerError::Unsupported(
                "comparison of counter CRDT values",
            ))
        }
        Some(Kind::Enum) => return Err(WasmLowerError::Unsupported("comparison of enum values")),
        Some(Kind::Closure) => {
            return Err(WasmLowerError::Unsupported("comparison of closure values"))
        }
        Some(Kind::Tuple) => return Err(WasmLowerError::Unsupported("comparison of tuple values")),
        Some(Kind::Rational) => {
            return Err(WasmLowerError::Unsupported("comparison of rational values"))
        }
        Some(Kind::BigInt) => {
            return Err(WasmLowerError::Unsupported("comparison of bigint values"))
        }
        Some(Kind::Complex) => {
            return Err(WasmLowerError::Unsupported("comparison of complex values"))
        }
        Some(Kind::Modular) => {
            return Err(WasmLowerError::Unsupported("comparison of modular values"))
        }
        Some(Kind::Decimal) => {
            return Err(WasmLowerError::Unsupported("comparison of decimal values"))
        }
        Some(Kind::Money) => return Err(WasmLowerError::Unsupported("comparison of money values")),
        Some(Kind::Quantity) => {
            return Err(WasmLowerError::Unsupported("comparison of quantity values"))
        }
        Some(Kind::Uuid) => {
            return Err(WasmLowerError::Unsupported(
                "ordering comparison of uuid values",
            ))
        }
        Some(Kind::LanesV) => {
            return Err(WasmLowerError::Unsupported(
                "comparison of lane-vector values",
            ))
        }
        Some(Kind::Dynamic) => {
            return Err(WasmLowerError::Unsupported(
                "comparison of dynamic wire values",
            ))
        }
        Some(Kind::Lanes) => return Err(WasmLowerError::Unsupported("comparison of lane vectors")),
        // Optional comparisons are handled or refused before scalar dispatch.
        Some(Kind::Optional) => {
            unreachable!("optional comparisons handled by the is-nothing path above")
        }
        None => {
            return Err(WasmLowerError::Unsupported(
                "comparison of unknown-kind values",
            ))
        }
    }
    Ok(Flow::Straight)
}

/// Push a register as an `f64` on the wasm stack — directly for a Float, via `f64.convert_i64_s`
/// for an Int.
/// `a is approximately b` — the shared isclose semantics
/// (`logicaffeine_data::ops::logos_approx_eq`), lowered as pure f64
/// instructions so the result is bit-identical to every other engine:
/// `(a == b) || |a - b| <= max(1e-9 * max(|a|, |b|), 1e-12)`.
/// `dst = lhs <cmp> rhs` over exact rationals (linker mode): both operands as `Rational` handles (an `Int` or
/// `BigInt` widens), ordered by the runtime's `Rational::cmp` — exact, never through a float.
fn lower_rational_compare(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    dst: u16,
    lhs: u16,
    rhs: u16,
    cmp: Cmp,
) -> R<Flow> {
    let real = |r: u16| kinds.get(r as usize) == Some(Kind::Float);
    if real(lhs) || real(rhs) {
        // A rational against a real, by the real's exact value: -1, 0, 1, or 2 for a NaN, which orders against
        // nothing and equals nothing.
        let (rational, f, flipped) = if real(rhs) { (lhs, rhs, false) } else { (rhs, lhs, true) };
        let idx = (ctx.host_index)(HostFn::RationalCmpF64)
            .ok_or(WasmLowerError::Unsupported("rational_cmp_f64 not imported"))?;
        local_get(code, rational as u32);
        local_get(code, f as u32);
        code.push(0x10);
        leb_u32(code, idx);
        let cmp = match (cmp, flipped) {
            (Cmp::Lt, true) => Cmp::Gt,
            (Cmp::Gt, true) => Cmp::Lt,
            (Cmp::Le, true) => Cmp::Ge,
            (Cmp::Ge, true) => Cmp::Le,
            (c, _) => c,
        };
        match cmp {
            Cmp::Eq => code.push(0x45), // i32.eqz
            Cmp::Ne => code.extend([0x45, 0x45]),
            Cmp::Lt => {
                i32_const(code, 0);
                code.push(0x48); // i32.lt_s
            }
            Cmp::Gt => {
                i32_const(code, 1);
                code.push(0x46); // i32.eq
            }
            Cmp::Le => {
                i32_const(code, 1);
                code.push(0x48); // i32.lt_s
            }
            Cmp::Ge => {
                i32_const(code, 2);
                code.push(0x49); // i32.lt_u: 0 or 1; -1 and NaN's 2 are not
            }
        }
        code.push(0xAD); // i64.extend_i32_u
        local_set(code, dst as u32);
        return Ok(Flow::Straight);
    }
    push_rational_operand(code, kinds, ctx, lhs)?;
    push_rational_operand(code, kinds, ctx, rhs)?;
    let order = (ctx.host_index)(HostFn::RationalCmp)
        .ok_or(WasmLowerError::Unsupported("rational_cmp not imported"))?;
    code.push(0x10);
    leb_u32(code, order);
    match cmp {
        Cmp::Eq => code.push(0x45), // i32.eqz
        Cmp::Ne => code.extend([0x45, 0x45]),
        Cmp::Lt | Cmp::Gt | Cmp::Le | Cmp::Ge => {
            i32_const(code, 0);
            code.push(match cmp {
                Cmp::Lt => 0x48, // i32.lt_s
                Cmp::Gt => 0x4A, // i32.gt_s
                Cmp::Le => 0x4C, // i32.le_s
                _ => 0x4E,       // i32.ge_s
            });
        }
    }
    code.push(0xAD); // i64.extend_i32_u — the VM's truthy-Int boolean width
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

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

/// [`push_as_f64`], a linked rational as the double the VM computes with when it meets a real.
fn push_number_as_f64(code: &mut Vec<u8>, ctx: &Ctx, reg: u16, kind: Option<Kind>) -> R<()> {
    if kind == Some(Kind::Rational) {
        let idx = (ctx.host_index)(HostFn::RationalToF64)
            .ok_or(WasmLowerError::Unsupported("rational_to_f64 not imported"))?;
        local_get(code, reg as u32);
        code.push(0x10);
        leb_u32(code, idx);
        return Ok(());
    }
    push_as_f64(code, reg, kind)
}

fn push_as_f64(code: &mut Vec<u8>, reg: u16, kind: Option<Kind>) -> R<()> {
    local_get(code, reg as u32);
    match kind {
        Some(Kind::Float) => {}
        Some(Kind::Int) => code.push(0xB9), // f64.convert_i64_s
        _ => {
            return Err(WasmLowerError::Unsupported(
                "numeric builtin on a non-number",
            ))
        }
    }
    Ok(())
}

/// Lower a numeric builtin call, bit-exactly matching the VM (`semantics/builtins.rs`).
fn lower_builtin(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    dst: u16,
    builtin: BuiltinId,
    args_start: u16,
    arg_count: u16,
) -> R<Flow> {
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
        // `leading_zeros(n)` / `trailing_zeros(n)` → the Int's high/low zero-bit
        // count (`i64.clz` / `i64.ctz`; both give 64 for 0), matching the VM.
        BuiltinId::LeadingZeros => {
            local_get(code, arg as u32);
            code.push(0x79); // i64.clz
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        BuiltinId::TrailingZeros => {
            local_get(code, arg as u32);
            code.push(0x7A); // i64.ctz
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
                _ => {
                    return Err(WasmLowerError::Unsupported(
                        "word_and/or of a non-Word value",
                    ))
                }
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
        other => {
            // Name the builtin under the trace — "builtin not yet lowered" describes the backend
            // and leaves the reader with no idea which sentence in their page caused it.
            if std::env::var_os("LOGOS_WASM_TRACE_FIELD").is_some() {
                eprintln!("[wasm] builtin not yet lowered: {other:?}");
            }
            Err(WasmLowerError::Unsupported("builtin not yet lowered"))
        }
    }
}

/// `floor`/`ceil` (`fop` = `f64.floor`/`f64.ceil`): a Float rounds then truncates with the
/// saturating cast to match the VM's `as i64`; an already-whole Int is returned unchanged.
fn lower_floor_ceil(
    code: &mut Vec<u8>,
    arg_kind: Option<Kind>,
    dst: u16,
    arg: u16,
    fop: u8,
) -> R<Flow> {
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
fn lower_minmax(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    dst: u16,
    args_start: u16,
    arg_count: u16,
    is_max: bool,
) -> R<Flow> {
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
            let fop = if is_max { 0xA5 } else { 0xA4 }; // f64.max / f64.min
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
        // `pow(m, e)` on a ℤ/nℤ base (linker-only kind) → modular exponentiation.
        (Some(Kind::Modular), Some(Kind::Int)) => Some(HostFn::ModularPow),
        _ => None,
    }
}

/// Lower `pow(base, exp)` bit-exactly. Float-result cases (any Float operand) defer to the host
/// `pow_ff`/`pow_fi` (so `powf` vs `powi` exactly match the VM). `Int^Int` is computed in-module
/// by [`lower_int_pow`] (Int result, overflow-trapping). A negative Int exponent (VM → Float)
/// traps, since a Float cannot ride the Int result slot.
fn lower_pow(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    args_start: u16,
    arg_count: u16,
) -> R<Flow> {
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
            let from_i64 = (ctx.host_index)(HostFn::BigintFromI64)
                .ok_or(WasmLowerError::Unsupported("bigint_from_i64 not imported"))?;
            local_get(code, reg as u32);
            code.push(0x10);
            leb_u32(code, from_i64);
        }
        _ => {
            return Err(WasmLowerError::Unsupported(
                "bigint arithmetic operand must be a BigInt or an Int",
            ))
        }
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as an exact big-integer binary operation (linker mode) — `op` is the
/// `logos_rt_bigint_{mul,add,sub}` sink. Both operands are brought to BigInt handles (an `Int` operand
/// promoted via `from_i64`); the resulting handle binds `dst`.
fn lower_bigint_binop(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    dst: u16,
    lhs: u16,
    rhs: u16,
    host: HostFn,
) -> R<Flow> {
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
            let from = (ctx.host_index)(HostFn::ComplexFromI64)
                .ok_or(WasmLowerError::Unsupported("complex_from_i64 not imported"))?;
            local_get(code, reg as u32);
            code.push(0x42); // i64.const 0 — the imaginary part of a promoted real
            leb_i64(code, 0);
            code.push(0x10);
            leb_u32(code, from);
        }
        _ => {
            return Err(WasmLowerError::Unsupported(
                "complex arithmetic operand must be a Complex or an Int",
            ))
        }
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as exact complex arithmetic (linker mode) — `op` is the
/// `logos_rt_complex_{add,sub,mul}` sink; both operands become Complex handles (an `Int` promoted).
fn lower_complex_binop(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    dst: u16,
    lhs: u16,
    rhs: u16,
    host: HostFn,
) -> R<Flow> {
    let op =
        (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("complex op not imported"))?;
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
        _ => {
            return Err(WasmLowerError::Unsupported(
                "modular arithmetic operand must be a Modular",
            ))
        }
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as exact ℤ/nℤ arithmetic (linker mode) — `op` is the `logos_rt_modular_*` sink.
fn lower_modular_binop(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    dst: u16,
    lhs: u16,
    rhs: u16,
    host: HostFn,
) -> R<Flow> {
    let op =
        (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("modular op not imported"))?;
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
            let from = (ctx.host_index)(HostFn::DecimalFromI64)
                .ok_or(WasmLowerError::Unsupported("decimal_from_i64 not imported"))?;
            local_get(code, reg as u32);
            code.push(0x10);
            leb_u32(code, from);
        }
        _ => {
            return Err(WasmLowerError::Unsupported(
                "decimal arithmetic operand must be a Decimal or an Int",
            ))
        }
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as exact base-10 arithmetic (linker mode) — `op` is the `logos_rt_decimal_*` sink.
fn lower_decimal_binop(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    dst: u16,
    lhs: u16,
    rhs: u16,
    host: HostFn,
) -> R<Flow> {
    let op =
        (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("decimal op not imported"))?;
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
        _ => {
            return Err(WasmLowerError::Unsupported(
                "money arithmetic operand must be a Money value",
            ))
        }
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as exact currency arithmetic (linker mode) — `op` is the `logos_rt_money_*` sink.
fn lower_money_binop(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    dst: u16,
    lhs: u16,
    rhs: u16,
    host: HostFn,
) -> R<Flow> {
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
        _ => {
            return Err(WasmLowerError::Unsupported(
                "quantity arithmetic operand must be a Quantity value",
            ))
        }
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as exact dimensional arithmetic (linker mode) — `op` is the `logos_rt_quantity_*`
/// sink. `+`/`-` keep the left display unit; `×`/`÷` combine dimensions and render in SI/dimension form.
fn lower_quantity_binop(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    dst: u16,
    lhs: u16,
    rhs: u16,
    host: HostFn,
) -> R<Flow> {
    let op =
        (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("quantity op not imported"))?;
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
            let from = (ctx.host_index)(HostFn::RationalFromI64).ok_or(
                WasmLowerError::Unsupported("rational_from_i64 not imported"),
            )?;
            local_get(code, reg as u32);
            code.push(0x10);
            leb_u32(code, from);
        }
        Some(Kind::BigInt) => {
            let from = (ctx.host_index)(HostFn::RationalFromBigint).ok_or(
                WasmLowerError::Unsupported("rational_from_bigint not imported"),
            )?;
            local_get(code, reg as u32);
            code.push(0x10);
            leb_u32(code, from);
        }
        _ => {
            return Err(WasmLowerError::Unsupported(
                "rational arithmetic operand must be a Rational, Int, or BigInt",
            ))
        }
    }
    Ok(())
}

/// `dst = lhs <op> rhs` as exact BigInt-backed rational arithmetic (linker mode) — `op` is the
/// `logos_rt_rational_*` sink. Operands promote to Rational handles first, so num/den stay exact past i64.
fn lower_rational_binop(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    dst: u16,
    lhs: u16,
    rhs: u16,
    host: HostFn,
) -> R<Flow> {
    let op =
        (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("rational op not imported"))?;
    push_rational_operand(code, kinds, ctx, lhs)?;
    push_rational_operand(code, kinds, ctx, rhs)?;
    code.push(0x10);
    leb_u32(code, op);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// `dst = <op>(arg)` on a Rational handle (linker mode) — `op` is a unary `logos_rt_rational_*` sink
/// (`floor`/`ceil`/`round` returning a BigInt handle, `abs` a Rational handle).
fn lower_rational_unary(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    dst: u16,
    arg: u16,
    host: HostFn,
) -> R<Flow> {
    let op = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported(
        "rational unary op not imported",
    ))?;
    local_get(code, arg as u32);
    code.push(0x10);
    leb_u32(code, op);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// `dst = (lhs == rhs)` (or `!=` when `negate`) on two Uuid handles (linker mode) — `logos_rt_uuid_eq`
/// compares the 16 bytes and returns an i32 0/1, extended to the i64 a `Bool` register holds (matching
/// [`lower_text_eq`]'s i64-boolean convention).
fn lower_uuid_eq(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    dst: u16,
    lhs: u16,
    rhs: u16,
    negate: bool,
) -> R<Flow> {
    let eq = (ctx.host_index)(HostFn::UuidEq)
        .ok_or(WasmLowerError::Unsupported("uuid_eq not imported"))?;
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
fn lower_span_add(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    base: u16,
    span: u16,
    is_date: bool,
    negate: bool,
) -> R<Flow> {
    let host = if is_date {
        HostFn::DateAddSpan
    } else {
        HostFn::MomentAddSpan
    };
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

fn lower_pow_regs(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    base: u16,
    exp: u16,
) -> R<Flow> {
    let (bk, ek) = (kinds.get(base as usize), kinds.get(exp as usize));
    match (bk, ek) {
        (_, Some(Kind::Float)) => {
            push_as_f64(code, base, bk)?;
            local_get(code, exp as u32); // exp is already f64
            let idx = (ctx.host_index)(HostFn::PowFf)
                .ok_or(WasmLowerError::Unsupported("pow_ff not imported"))?;
            code.push(0x10);
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        (Some(Kind::Float), Some(Kind::Int)) => {
            local_get(code, base as u32);
            local_get(code, exp as u32);
            let idx = (ctx.host_index)(HostFn::PowFi)
                .ok_or(WasmLowerError::Unsupported("pow_fi not imported"))?;
            code.push(0x10);
            leb_u32(code, idx);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        (Some(Kind::Modular), Some(Kind::Int)) if ctx.linked => {
            // `pow(m, e)` on a ℤ/nℤ base — modular exponentiation via the linked runtime. Push the
            // Modular handle (i32) and the i64 exponent, `call logos_rt_modular_pow` → a Modular handle.
            let pow = (ctx.host_index)(HostFn::ModularPow)
                .ok_or(WasmLowerError::Unsupported("modular_pow not imported"))?;
            local_get(code, base as u32); // i32 Modular handle
            local_get(code, exp as u32); // i64 exponent
            code.push(0x10);
            leb_u32(code, pow);
            local_set(code, dst as u32);
            Ok(Flow::Straight)
        }
        (Some(Kind::Int), Some(Kind::Int)) if ctx.linked => {
            // LINKER MODE: compute the exact big integer via the real `logicaffeine_base::BigInt`
            // runtime — `from_i64(base)` → `pow(handle, exp)` — leaving a BigInt HANDLE in `dst`. No
            // overflow, no trap. The handle stays a handle (rendered to a decimal `Text` only at `Show`,
            // via `lower_show`), so a downstream `*` keeps multiplying on real BigInts.
            let from_i64 = (ctx.host_index)(HostFn::BigintFromI64)
                .ok_or(WasmLowerError::Unsupported("bigint_from_i64 not imported"))?;
            let pow = (ctx.host_index)(HostFn::BigintPow)
                .ok_or(WasmLowerError::Unsupported("bigint_pow not imported"))?;
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
fn lower_floordiv_regs(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    num_regs: u32,
    dst: u16,
    lhs: u16,
    rhs: u16,
) -> R<Flow> {
    match kinds.get(dst as usize) {
        Some(Kind::Int) => {
            if kinds.valtype(lhs as usize) != I64 || kinds.valtype(rhs as usize) != I64 {
                return Err(WasmLowerError::Unsupported(
                    "integer floor division with a non-integer operand",
                ));
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
        _ => {
            return Err(WasmLowerError::Unsupported(
                "floor division on a non-numeric value",
            ))
        }
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

/// A function the emitter writes itself rather than compiling from LOGOS.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RuntimeFn {
    /// `__logos_alloc(size) -> ptr` — see [`emit_alloc_helper`].
    Alloc,
    /// `__logos_free(ptr, size)` — see [`emit_free_helper`].
    Free,
    /// `__logos_heap_allocated() -> bytes` — the GROSS heap: every byte the allocator has claimed
    /// from linear memory, live or waiting on a free list. It only grows.
    HeapAllocated,
    /// `__logos_heap_used() -> bytes` — the NET heap: the gross heap less the blocks sitting on the
    /// free lists, which is what the page's values actually occupy.
    HeapUsed,
    /// `__logos_iter_unwind(base)` — pop and free `Repeat` frames down to `base`; see
    /// [`emit_leave_loops`].
    IterUnwind,
    /// `__logos_text_new(len) -> handle` — a Text block for the HOST to write `len` bytes into,
    /// allocated through the same allocator every module-side Text uses. Gives the host a way to
    /// put a string on the module's own heap instead of bump-allocating pages of its own above it.
    TextNew,
    /// `__logos_seq_new(len, handles) -> handle` — a `Seq` of `len` 8-byte slots for the HOST to
    /// fill, the list analog of [`RuntimeFn::TextNew`]: the header an ordinary `Seq` has (its class
    /// from `handles`, and — on a module that counts holders — a true count of 1 credited to the
    /// turn, exactly as a module-built sequence is born), so nothing downstream can tell a list the
    /// host built from one the module did.
    SeqNew,
    /// `__logos_seq_slot(handle, index) -> address` — the 8-byte slot of the 1-based `index` in a
    /// sequence (or a tuple: the same header and slots), trapping through `logos_rt_fail_at` with
    /// the index out of bounds. Every `item i of xs` and `Set item i of xs to v` is a call to it and
    /// the load or store at the element's width. See `emit_seq_slot_helper`.
    SeqSlot,
    /// `__logos_seq_push_slot(handle) -> address` — a sequence one element longer, its buffer
    /// doubled first when full (the old buffer freed), and the address of the new last slot, for
    /// a push to store its element at the element's width. See `emit_seq_push_slot_helper`.
    SeqPushSlot,
    /// `__logos_seq_clone(handle, mode) -> handle` — a fresh sequence with `handle`'s elements
    /// (8-byte slots, copied as they are), stamped the class in `mode`'s low byte, and born counted
    /// when the module counts holders and the class carries a count; `mode`'s [`CLONE_RETAINS`] and
    /// [`CLONE_SHARES`] bits say what the copy's elements gain. The copy of a sequence, a set or a
    /// record. See `emit_seq_clone_helper`.
    SeqClone,
    /// `__logos_seq_own(handle, mode) -> handle` — the copy-on-write of a sequence about to be
    /// written: `handle` itself when nothing else holds it (or it is a `SharedMap` replica), else
    /// `__logos_seq_clone(handle, mode)`. Every mutation of a sequence or set is a call to it.
    SeqOwn,
    /// `__logos_text_eq(a, b) -> i32` — byte equality of two Texts. See `emit_text_eq_helper`.
    TextEq,
    /// `__logos_text_concat(a, b) -> handle` — a fresh Text of `a`'s bytes then `b`'s. See
    /// `emit_text_concat_helper`.
    TextConcat,
    /// `__logos_map_clone(handle) -> handle` — a fresh map holding `handle`'s entries, its class
    /// (and a `SharedMap`'s mark) kept. See `emit_map_clone_helper`.
    MapClone,
    /// `__logos_map_own(handle) -> handle` — the copy-on-write of a map about to be written, as
    /// [`RuntimeFn::SeqOwn`] is a sequence's.
    MapOwn,
    /// `__logos_map_put(map, key, class) -> address` — the entry `key` names, appended (growing the
    /// entry buffer) when the map has none, its value slot empty. See `emit_map_put_helper`.
    MapPut,
    /// `__logos_map_find(map, key, text_key) -> address` — the entry `key` names, or 0. See
    /// `emit_map_find_helper`.
    MapFind,
    /// `__logos_map_at(map, key, text_key) -> address` — the entry `key` names; a key the map does
    /// not hold traps, as `item k of m` on an absent key does.
    MapAt,
    /// `__logos_add_checked(a, b) -> i64` — `a + b`, trapping on signed overflow as the inline check
    /// does ([`super::encode::emit_checked_addsub`]). With [`RuntimeFn::SubChecked`] and
    /// [`RuntimeFn::MulChecked`], checked Int arithmetic outside a loop is a call.
    AddChecked,
    /// `__logos_sub_checked(a, b) -> i64` — `a - b`, trapping on signed overflow.
    SubChecked,
    /// `__logos_mul_checked(a, b) -> i64` — `a * b`, trapping on signed overflow
    /// ([`super::encode::emit_checked_mul`]).
    MulChecked,
    /// `__logos_exact_add(a, b) -> i64` — the sum of two tagged words as a tagged word of its own:
    /// inline on two small ones, the host's `exact_add` otherwise. With [`RuntimeFn::ExactSub`] and
    /// [`RuntimeFn::ExactMul`], exact Int arithmetic outside a loop is a call
    /// ([`exact::emit_ring_helper`]).
    ExactAdd,
    /// `__logos_exact_sub(a, b) -> i64` — the difference of two tagged words.
    ExactSub,
    /// `__logos_exact_mul(a, b) -> i64` — the product of two tagged words.
    ExactMul,
    /// `__logos_exact_add_w(a, w) -> i64` — the sum of the tagged word `a` and the machine word `w`, which the
    /// helper tags and gives back itself, so a site beside a machine-word operand holds no tagged word of its
    /// own ([`exact::emit_ring_word_helper`]).
    ExactAddW,
    /// `__logos_exact_sub_w(a, w) -> i64` — `a` less the machine word `w`.
    ExactSubW,
    /// `__logos_exact_mul_w(a, w) -> i64` — `a` times the machine word `w`.
    ExactMulW,
    /// `__logos_exact_tag(w) -> i64` — a machine word as a tagged word: shifted beside the tag when it
    /// fits, held by the host when not. Outside a loop, tagging is a call.
    ExactTag,
    /// `__logos_exact_release(t)` — give back the integer a tagged word names when the host holds it.
    /// Outside a loop, giving one up is a call.
    ExactRelease,
    /// `__logos_exact_narrow(t, why) -> i64` — a tagged word's integer as a machine word, failing with the code
    /// `why` when it is past one. Outside a loop, narrowing is a call.
    ExactNarrow,
    /// `__logos_exact_clone(t) -> i64` — a tagged word of its own for `t`: the host's clone of a held integer,
    /// a small one as it is. Outside a loop, cloning is a call.
    ExactClone,
    /// Shared exact collection representation and cell-allocation mechanisms.
    ExactSeqPromote,
    ExactCell,
    ExactCompare,
    ExactSeqAt,
    ExactSeqStore,
    ExactMapPromoteKey,
    ExactMapPromoteValue,
    /// `__logos_new_header(class) -> handle` — a fresh, empty 16-byte collection header
    /// `[len=0][cap=0][data_ptr=0]` stamped `class`, born a true holder count of 1 credited to the
    /// turn when that class carries one, as [`emit_alloc`] births every counted block. An empty
    /// `Seq`/`Set`/`Map` constructor is a call to it.
    NewHeader,
    /// `__logos_share(handle)` — one more copy-on-write holder of `handle`: word 12's count (bits
    /// 8–31) grows by one, its class byte untouched ([`emit_retain`]'s call form).
    Share,
    /// `__logos_turn_retain(handle)` — append `handle` (already carrying a true holder count, see
    /// [`AllocClass::is_counted`]) to the per-turn retained list, growing it (doubling, like every
    /// other buffer in this backend) when full. Called from [`emit_alloc`] for every fresh counted
    /// block (the turn is its first, implicit holder) and from a mid-turn release that would
    /// otherwise take a count to zero (S8's "a mid-turn drop to 0 re-lists it on the turn").
    TurnRetain,
    /// `__logos_retain(handle)` — `mem[handle+16] += 1` (a no-op on a null handle). The true-count
    /// analogue of [`emit_retain`] (which stays on word 12, COW-only, untouched by S8): called
    /// whenever a counted value becomes held by a SECOND real, heap-resident holder (a global, a
    /// collection element, a struct field, a closure capture) — never for a register-only borrow
    /// (a `Call` argument, an `Index`/`Pop` extraction), matching the plan's "registers borrow".
    Retain,
    /// `__logos_retain_children(handle)` — a container has just become a further holder of every
    /// handle it holds (a copy sharing its source's elements or entries, a literal built from
    /// registers): [`RuntimeFn::Retain`] on each handle its class says it holds. One call per site,
    /// so a copy or a literal of any length costs the same code. See `emit_retain_children_helper`.
    RetainChildren,
    /// `__logos_release(handle)` — the MID-TURN release: `mem[handle+16] -= 1`, UNLESS that would
    /// reach 0, in which case the count is left alone and `handle` is pushed back onto the turn's
    /// retained list instead (deferred to the next safepoint — "nothing is freed mid-turn"). A
    /// no-op on a null handle. Called wherever a real holder stops holding a value mid-turn: a
    /// `GlobalSet`/`SetIndex`/map-insert overwriting what was there, `RemoveFrom`.
    Release,
    /// `__logos_safepoint()` — run when the host's own re-entry depth returns to 0 (today called
    /// directly by a test/host driver; the runtime.js wiring lands separately). Drains the turn's
    /// retained list: each entry's count drops by exactly 1 (the turn's own implicit hold ending);
    /// whatever reaches 0 is handed to the free cascade. Nothing is freed anywhere else.
    Safepoint,
    /// `__logos_free_object(handle)` — the cascade worker: given a handle whose true count JUST
    /// reached 0, walk its class-specific children (a `HandleHeader`'s element slots) through
    /// [`RuntimeFn::Release`]'s AUTHORITATIVE sibling (an unconditional decrement-and-enqueue, no
    /// mid-turn deferral — see `emit_free_object_helper`), free its data buffer(s) and its own
    /// 20-byte header, and POISON the freed range. Only ever called from inside
    /// [`RuntimeFn::Safepoint`]'s own worklist loop, never mid-turn.
    FreeObject,
    /// `__logos_heap_census() -> i32` — a CHECK-MODE instrument, not part of a page's normal
    /// operation: walks every counted value reachable from the program's own globals, and for each
    /// one INDEPENDENTLY recomputes how many of those roots/slots point at it, comparing the
    /// recomputed number to the block's own stored true count. Returns 0 when every reachable
    /// object's stored count exactly equals its recomputed in-degree, or the address of the first
    /// mismatch otherwise. See `emit_heap_census_helper`.
    Census,
    /// Count one incoming edge, visiting a container's children only on its first visit.
    CensusVisit,
    /// Register a closure with the host, returning a fresh opaque callback identity.
    /// Each registry entry owns one true reference, independently of other registrations.
    ClosureRetain,
    /// Exported as `__logos_release`: remove the host registration named by its identity,
    /// release its closure reference, and return the registry entry's storage to the allocator.
    /// Unknown or already-released identities own nothing and have no effect.
    ClosureRelease,
}

impl RuntimeFn {
    fn signature(self) -> (Vec<u8>, Vec<u8>) {
        match self {
            RuntimeFn::Alloc
            | RuntimeFn::TextNew
            | RuntimeFn::SeqPushSlot
            | RuntimeFn::MapClone
            | RuntimeFn::MapOwn
            | RuntimeFn::ClosureRetain => (vec![I32], vec![I32]),
            RuntimeFn::SeqNew
            | RuntimeFn::SeqClone
            | RuntimeFn::SeqOwn
            | RuntimeFn::TextEq
            | RuntimeFn::TextConcat => (vec![I32, I32], vec![I32]),
            RuntimeFn::SeqSlot => (vec![I32, I64], vec![I32]),
            RuntimeFn::MapPut | RuntimeFn::MapFind | RuntimeFn::MapAt => {
                (vec![I32, I64, I32], vec![I32])
            }
            RuntimeFn::AddChecked
            | RuntimeFn::SubChecked
            | RuntimeFn::MulChecked
            | RuntimeFn::ExactAdd
            | RuntimeFn::ExactSub
            | RuntimeFn::ExactMul
            | RuntimeFn::ExactAddW
            | RuntimeFn::ExactSubW
            | RuntimeFn::ExactMulW => (vec![I64, I64], vec![I64]),
            RuntimeFn::ExactTag | RuntimeFn::ExactClone => (vec![I64], vec![I64]),
            RuntimeFn::ExactCell => (vec![I64], vec![I32]),
            RuntimeFn::ExactCompare => (vec![I64, I64], vec![I32]),
            RuntimeFn::ExactSeqAt => (vec![I32, I64], vec![I64]),
            RuntimeFn::ExactSeqStore => (vec![I32, I64], vec![]),
            RuntimeFn::ExactSeqPromote
            | RuntimeFn::ExactMapPromoteKey
            | RuntimeFn::ExactMapPromoteValue => (vec![I32], vec![]),
            RuntimeFn::ExactRelease => (vec![I64], vec![]),
            RuntimeFn::ExactNarrow => (vec![I64, I32], vec![I64]),
            RuntimeFn::NewHeader => (vec![I32], vec![I32]),
            RuntimeFn::Share => (vec![I32], vec![]),
            RuntimeFn::Free => (vec![I32, I32], vec![]),
            RuntimeFn::HeapAllocated | RuntimeFn::HeapUsed => (vec![], vec![I32]),
            RuntimeFn::IterUnwind => (vec![I32], vec![]),
            RuntimeFn::TurnRetain
            | RuntimeFn::Retain
            | RuntimeFn::RetainChildren
            | RuntimeFn::Release
            | RuntimeFn::FreeObject
            | RuntimeFn::ClosureRelease => (vec![I32], vec![]),
            RuntimeFn::Safepoint => (vec![], vec![]),
            RuntimeFn::Census => (vec![], vec![I32]),
            RuntimeFn::CensusVisit => (vec![I32, I32], vec![]),
        }
    }

    /// The name a host calls this by, or `None` for a function only the module calls.
    fn export(self) -> Option<&'static str> {
        match self {
            RuntimeFn::Alloc
            | RuntimeFn::Free
            | RuntimeFn::IterUnwind
            | RuntimeFn::SeqSlot
            | RuntimeFn::SeqPushSlot
            | RuntimeFn::SeqClone
            | RuntimeFn::SeqOwn
            | RuntimeFn::TextEq
            | RuntimeFn::TextConcat
            | RuntimeFn::MapClone
            | RuntimeFn::MapOwn
            | RuntimeFn::MapPut
            | RuntimeFn::MapFind
            | RuntimeFn::MapAt
            | RuntimeFn::AddChecked
            | RuntimeFn::SubChecked
            | RuntimeFn::MulChecked
            | RuntimeFn::ExactAdd
            | RuntimeFn::ExactSub
            | RuntimeFn::ExactMul
            | RuntimeFn::ExactAddW
            | RuntimeFn::ExactSubW
            | RuntimeFn::ExactMulW
            | RuntimeFn::ExactTag
            | RuntimeFn::ExactRelease
            | RuntimeFn::ExactNarrow
            | RuntimeFn::ExactClone
            | RuntimeFn::ExactSeqPromote
            | RuntimeFn::ExactCell
            | RuntimeFn::ExactCompare
            | RuntimeFn::ExactSeqAt
            | RuntimeFn::ExactSeqStore
            | RuntimeFn::ExactMapPromoteKey
            | RuntimeFn::ExactMapPromoteValue
            | RuntimeFn::NewHeader
            | RuntimeFn::Share
            | RuntimeFn::TurnRetain
            | RuntimeFn::Retain
            | RuntimeFn::RetainChildren
            | RuntimeFn::Release
            | RuntimeFn::FreeObject
            | RuntimeFn::CensusVisit
            | RuntimeFn::ClosureRetain => None,
            RuntimeFn::HeapAllocated => Some("__logos_heap_allocated"),
            RuntimeFn::HeapUsed => Some("__logos_heap_used"),
            RuntimeFn::TextNew => Some("__logos_text_new"),
            RuntimeFn::SeqNew => Some("__logos_seq_new"),
            RuntimeFn::Safepoint => Some("__logos_safepoint"),
            RuntimeFn::Census => Some("__logos_heap_census"),
            // `runtime.js`'s `releaseHandle` already calls `inst.exports.__logos_release(h)` when
            // the HOST stops holding a closure it registered (a replaced listener, a finished
            // fetch, a superseded route handler) — the Wave 6 plan's "the host calls
            // `__logos_release(h)` once per dropped handle". It has always guarded on the export
            // existing and silently no-opped until this session, since nothing exported it. THIS
            // export is closure-specific (see `RuntimeFn::ClosureRelease`'s doc) — a DIFFERENT,
            // narrower true-count than the header count `RuntimeFn::Release` (internal only, never
            // called by the host) maintains for `Seq`/`Set`.
            RuntimeFn::ClosureRelease => Some("__logos_release"),
        }
    }

    fn body(self, env: &RuntimeEnv) -> Vec<u8> {
        match self {
            RuntimeFn::Alloc => emit_alloc_helper(env.heap_global, env.fail_at),
            RuntimeFn::Free => emit_free_helper(),
            RuntimeFn::HeapAllocated => emit_heap_allocated_helper(env),
            RuntimeFn::HeapUsed => emit_heap_used_helper(env),
            RuntimeFn::IterUnwind => emit_iter_unwind_helper(env),
            RuntimeFn::TextNew => emit_text_new_helper(env),
            RuntimeFn::SeqNew => emit_seq_new_helper(env),
            RuntimeFn::SeqSlot => emit_seq_slot_helper(env),
            RuntimeFn::SeqPushSlot => emit_seq_push_slot_helper(env),
            RuntimeFn::SeqClone => emit_seq_clone_helper(env),
            RuntimeFn::SeqOwn => emit_seq_own_helper(env),
            RuntimeFn::TextEq => emit_text_eq_helper(),
            RuntimeFn::TextConcat => emit_text_concat_helper(env),
            RuntimeFn::MapClone => emit_map_clone_helper(env),
            RuntimeFn::MapOwn => emit_map_own_helper(env),
            RuntimeFn::MapPut => emit_map_put_helper(env),
            RuntimeFn::MapFind => emit_map_find_helper(env),
            RuntimeFn::MapAt => emit_map_at_helper(env),
            RuntimeFn::AddChecked => emit_checked_arith_helper(ArithOp::Add),
            RuntimeFn::SubChecked => emit_checked_arith_helper(ArithOp::Sub),
            RuntimeFn::MulChecked => emit_checked_arith_helper(ArithOp::Mul),
            RuntimeFn::ExactAdd => exact::emit_ring_helper(env, ArithOp::Add),
            RuntimeFn::ExactSub => exact::emit_ring_helper(env, ArithOp::Sub),
            RuntimeFn::ExactMul => exact::emit_ring_helper(env, ArithOp::Mul),
            RuntimeFn::ExactAddW => exact::emit_ring_word_helper(env, ArithOp::Add),
            RuntimeFn::ExactSubW => exact::emit_ring_word_helper(env, ArithOp::Sub),
            RuntimeFn::ExactMulW => exact::emit_ring_word_helper(env, ArithOp::Mul),
            RuntimeFn::ExactTag => exact::emit_tag_helper(env),
            RuntimeFn::ExactRelease => exact::emit_release_helper(env),
            RuntimeFn::ExactNarrow => exact::emit_narrow_helper(env),
            RuntimeFn::ExactClone => exact::emit_clone_helper(env),
            RuntimeFn::ExactSeqPromote => exact::emit_seq_promote_helper(env),
            RuntimeFn::ExactCell => exact::emit_cell_helper(env),
            RuntimeFn::ExactCompare => exact::emit_compare_helper(env),
            RuntimeFn::ExactSeqAt => exact::emit_seq_at_helper(env),
            RuntimeFn::ExactSeqStore => exact::emit_seq_store_helper(env),
            RuntimeFn::ExactMapPromoteKey => exact::emit_map_promote_helper(env, 0),
            RuntimeFn::ExactMapPromoteValue => exact::emit_map_promote_helper(env, 8),
            RuntimeFn::NewHeader => emit_new_header_helper(env),
            RuntimeFn::Share => emit_share_helper(),
            RuntimeFn::TurnRetain => emit_turn_retain_helper(env),
            RuntimeFn::Retain => emit_true_retain_helper(),
            RuntimeFn::RetainChildren => emit_retain_children_helper(env),
            RuntimeFn::Release => emit_true_release_helper(env),
            RuntimeFn::Safepoint => emit_safepoint_helper(env),
            RuntimeFn::FreeObject => emit_free_object_helper(env),
            RuntimeFn::Census => emit_heap_census_helper(env),
            RuntimeFn::CensusVisit => emit_heap_census_visit_helper(env),
            RuntimeFn::ClosureRetain => emit_closure_retain_helper(env),
            RuntimeFn::ClosureRelease => emit_closure_release_helper(env),
        }
    }
}

/// What a runtime function's body needs that is only known once the program's own bodies are
/// lowered.
struct RuntimeEnv<'r> {
    heap_global: u32,
    iter_global: u32,
    /// Where the dynamic heap begins: above the free-list table and the interned text.
    heap_base: i32,
    /// The host import index of `logos_rt_fail_at`, for the allocator's own out-of-memory report.
    fail_at: u32,
    /// Releases the host value owned by an exact integer cell, when this module uses them.
    exact_drop: Option<u32>,
    exact_clone: Option<u32>,
    exact_cmp: Option<u32>,
    /// The host's exact-integer imports the out-of-line exact helpers call, when the module holds
    /// integers exact: `exact_box`, `exact_add`, `exact_sub`, `exact_mul`, `exact_narrow`, `exact_clone`.
    exact_host: Option<[u32; 6]>,
    /// The wasm global index of the turn-retained list's `data_ptr`; `+1` is its `len`, `+2` its
    /// `cap` (S8). `None` on a non-interactive module (see [`Runtime::layout`]).
    retain_base: Option<u32>,
    /// Every user global's kind (by global index), so the census can tell which globals are ROOTS
    /// of the counted heap graph — `None`/scalar globals are skipped.
    global_kinds: Vec<Option<Kind>>,
    callback_base: Option<u32>,
    /// Whether the program builds a `SharedMap`, whose copy stays a replica ([`SHARED_MAP_MARK`]).
    shared_maps: bool,
    runtime: &'r Runtime,
}

impl RuntimeEnv<'_> {
    /// The wasm function index of the runtime function `f` one runtime function's body calls —
    /// which makes `f` a function the module carries in full ([`Runtime::index`]).
    fn call(&self, f: RuntimeFn) -> u32 {
        self.runtime.index(f).unwrap_or_else(|| {
            panic!("a runtime body calls {f:?}, which this module's runtime does not lay out")
        })
    }
}

/// The runtime functions a module carries, laid out after the closure trampoline in a fixed order
/// so every index is known before any body is lowered.
struct Runtime {
    /// The wasm function index of `fns[0]`.
    first: u32,
    fns: Vec<RuntimeFn>,
    /// The type index of each of `fns`.
    types: Vec<u32>,
    /// Bit `i` set: something the module emits calls `fns[i]`. A laid-out function nothing calls and
    /// no host is handed keeps its index and type but carries an `unreachable` body.
    called: std::cell::Cell<u64>,
}

impl Runtime {
    /// Every self-contained heap module carries the allocator. An INTERACTIVE one — a page the host
    /// re-enters after `main`, through a callback it registered or a function it enters by name —
    /// also exports the heap readings a host measures
    /// it by; a program that runs `main` once and ends has no host left to read them. Either one
    /// that takes a Text from its host (`host_text`) exports the allocator the host writes it
    /// through, `__logos_text_new`; one that takes a sequence from its host (`host_seq`) exports
    /// `__logos_seq_new`.
    fn layout(
        first: u32,
        interactive: bool,
        host_text: bool,
        host_seq: bool,
        loops: bool,
        exact: bool,
        reclaim: Reclaim,
        types: &mut TypeTable,
    ) -> Runtime {
        let mut fns = vec![
            RuntimeFn::Alloc,
            RuntimeFn::Free,
            RuntimeFn::SeqSlot,
            RuntimeFn::SeqPushSlot,
            RuntimeFn::SeqClone,
            RuntimeFn::SeqOwn,
            RuntimeFn::TextEq,
            RuntimeFn::TextConcat,
            RuntimeFn::MapClone,
            RuntimeFn::MapOwn,
            RuntimeFn::MapPut,
            RuntimeFn::MapFind,
            RuntimeFn::MapAt,
            RuntimeFn::AddChecked,
            RuntimeFn::SubChecked,
            RuntimeFn::MulChecked,
            RuntimeFn::NewHeader,
            RuntimeFn::Share,
        ];
        if !interactive && host_text {
            fns.push(RuntimeFn::TextNew);
        }
        if interactive {
            fns.extend([
                RuntimeFn::HeapAllocated,
                RuntimeFn::HeapUsed,
                RuntimeFn::TextNew,
            ]);
        }
        if reclaim != Reclaim::Off {
            fns.extend([
                RuntimeFn::TurnRetain,
                RuntimeFn::Retain,
                RuntimeFn::RetainChildren,
                RuntimeFn::Release,
                RuntimeFn::Safepoint,
                RuntimeFn::FreeObject,
                RuntimeFn::ClosureRetain,
                RuntimeFn::ClosureRelease,
            ]);
        }
        if reclaim == Reclaim::Checked {
            fns.extend([RuntimeFn::Census, RuntimeFn::CensusVisit]);
        }
        if host_seq {
            fns.push(RuntimeFn::SeqNew);
        }
        if loops {
            fns.push(RuntimeFn::IterUnwind);
        }
        if exact {
            fns.extend([
                RuntimeFn::ExactAdd,
                RuntimeFn::ExactSub,
                RuntimeFn::ExactMul,
                RuntimeFn::ExactAddW,
                RuntimeFn::ExactSubW,
                RuntimeFn::ExactMulW,
                RuntimeFn::ExactTag,
                RuntimeFn::ExactRelease,
                RuntimeFn::ExactNarrow,
                RuntimeFn::ExactClone,
                RuntimeFn::ExactSeqPromote,
                RuntimeFn::ExactCell,
                RuntimeFn::ExactCompare,
                RuntimeFn::ExactSeqAt,
                RuntimeFn::ExactSeqStore,
                RuntimeFn::ExactMapPromoteKey,
                RuntimeFn::ExactMapPromoteValue,
            ]);
        }
        let types = fns
            .iter()
            .map(|f| {
                let (p, r) = f.signature();
                types.intern(p, r)
            })
            .collect();
        Runtime {
            first,
            fns,
            types,
            called: std::cell::Cell::new(0),
        }
    }

    /// The wasm function index of `f`, for a call to it: asking marks `f` called, so the module
    /// carries its body.
    fn index(&self, f: RuntimeFn) -> Option<u32> {
        let i = self.fns.iter().position(|x| *x == f)?;
        self.called.set(self.called.get() | 1 << i);
        Some(self.first + i as u32)
    }

    /// Every laid-out function's body, in layout order: in full for one a host is handed or
    /// something the module emits calls — its callers' bodies included, until no body calls one
    /// more — and `unreachable` for the rest, which nothing reaches.
    fn bodies(&self, env: &RuntimeEnv<'_>) -> Vec<Vec<u8>> {
        let mut bodies: Vec<Option<Vec<u8>>> = vec![None; self.fns.len()];
        loop {
            let mut grew = false;
            for (i, f) in self.fns.iter().enumerate() {
                if bodies[i].is_none()
                    && (f.export().is_some() || self.called.get() & (1 << i) != 0)
                {
                    bodies[i] = Some(f.body(env));
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        bodies
            .into_iter()
            .map(|b| b.unwrap_or_else(|| vec![0x00, 0x00, 0x0B]))
            .collect()
    }
}

/// `__logos_heap_allocated() -> i32`: the bump pointer's distance from the heap base.
fn emit_heap_allocated_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = vec![0x00]; // no locals
    global_get(&mut c, env.heap_global);
    i32_const(&mut c, env.heap_base);
    c.push(0x6B); // i32.sub
    c.push(0x0B);
    c
}

/// `__logos_heap_used() -> i32`: the gross heap less every block on a free list. Walks the lists,
/// so it costs nothing on the allocation path — it is a reading, not a counter.
fn emit_heap_used_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 0 = class, 1 = block, 2 = used
    c.push(0x01);
    leb_u32(&mut c, 3);
    c.push(I32);
    global_get(&mut c, env.heap_global);
    i32_const(&mut c, env.heap_base);
    c.push(0x6B);
    local_set(&mut c, 2);
    c.push(0x02);
    c.push(0x40); // block $classes_done
    c.push(0x03);
    c.push(0x40); // loop $classes
    local_get(&mut c, 0);
    i32_const(&mut c, FREE_CLASSES + POWER_CLASSES);
    c.push(0x4E); // i32.ge_s
    c.push(0x0D);
    leb_u32(&mut c, 1); // br_if $classes_done
                        // block = table[class]
    local_get(&mut c, 0);
    i32_const(&mut c, 4);
    c.push(0x6C);
    i32_load(&mut c, FREE_TABLE_BASE as u32);
    local_set(&mut c, 1);
    c.push(0x02);
    c.push(0x40); // block $list_done
    c.push(0x03);
    c.push(0x40); // loop $list
    local_get(&mut c, 1);
    c.push(0x45); // i32.eqz
    c.push(0x0D);
    leb_u32(&mut c, 1); // br_if $list_done
                        // used -= this class's block size: the exact tier's is (class+1)*8; the power tier's
                        // (class >= FREE_CLASSES) is 2^(POWER_MIN + class - FREE_CLASSES).
    local_get(&mut c, 0);
    i32_const(&mut c, FREE_CLASSES);
    c.push(0x49); // i32.lt_u
    c.push(0x04);
    c.push(0x40); // if
    {
        local_get(&mut c, 2);
        local_get(&mut c, 0);
        i32_const(&mut c, 1);
        c.push(0x6A);
        i32_const(&mut c, 8);
        c.push(0x6C);
        c.push(0x6B);
        local_set(&mut c, 2);
    }
    c.push(0x05); // else
    {
        local_get(&mut c, 2);
        i32_const(&mut c, 1);
        i32_const(&mut c, POWER_MIN);
        local_get(&mut c, 0);
        i32_const(&mut c, FREE_CLASSES);
        c.push(0x6B); // class - FREE_CLASSES
        c.push(0x6A); // POWER_MIN + (class - FREE_CLASSES)
        c.push(0x74); // i32.shl → 1 << exponent
        c.push(0x6B);
        local_set(&mut c, 2);
    }
    c.push(0x0B); // end if
    local_get(&mut c, 1);
    i32_load(&mut c, 0); // next
    local_set(&mut c, 1);
    c.push(0x0C);
    leb_u32(&mut c, 0); // br $list
    c.push(0x0B);
    c.push(0x0B);
    local_get(&mut c, 0);
    i32_const(&mut c, 1);
    c.push(0x6A);
    local_set(&mut c, 0);
    c.push(0x0C);
    leb_u32(&mut c, 0); // br $classes
    c.push(0x0B);
    c.push(0x0B);
    local_get(&mut c, 2);
    c.push(0x0B);
    c
}

/// `__logos_alloc(size: i32) -> i32` — the self-contained allocator, emitted ONCE and called from
/// every allocation site: a call is four bytes where the free-list check inline would be forty.
///
/// Take the head of this size's free list when there is one; otherwise bump. Up to `FREE_CLASSES`
/// units the class IS `u - 1` — exact, so nothing is rounded up. Above that, the class is the
/// smallest power of two at or above the request (`2^(32 - clz(size - 1))`, the usual round-up-to-
/// power-of-two identity); a request past the last power class still bumps, sized to its own exact
/// rounded need, but is never reclaimed — the same as any size ever was before this tier existed.
fn emit_alloc_helper(heap_global: u32, fail_at: u32) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 1 = u, 2 = slot address, 3 = head, 4 = result, 5 = this block's class size in bytes,
    // 6 = whether a free list exists for this size at all (0/1), 7 = new_top (the bump path's
    // candidate `__heap_ptr`), 8 = memory's current size in PAGES, 9 = pages needed to fit `new_top`
    c.push(0x01); // one local group
    leb_u32(&mut c, 9);
    c.push(I32);

    // A request for nothing (an empty list's or Text's buffer) is answered with the null block: no
    // byte of it is ever read or written, and `__logos_free` frees nothing at null. A zero-sized block
    // handed out at the bump pointer would be the very address the next request is handed.
    local_get(&mut c, 0);
    c.push(0x45); // i32.eqz
    c.push(0x04);
    c.push(0x40);
    {
        i32_const(&mut c, 0);
        c.push(0x0F); // return
    }
    c.push(0x0B);

    // u = (size + 7) >> 3
    local_get(&mut c, 0);
    i32_const(&mut c, 7);
    c.push(0x6A); // i32.add
    i32_const(&mut c, 3);
    c.push(0x76); // i32.shr_u
    local_set(&mut c, 1);

    i32_const(&mut c, 0);
    local_set(&mut c, 4); // result = 0 (nothing taken from a list yet)

    local_get(&mut c, 1);
    i32_const(&mut c, FREE_CLASSES);
    c.push(0x4D); // i32.le_u
    c.push(0x04);
    c.push(0x40); // if (exact tier)
    {
        // slot = FREE_TABLE_BASE + (u - 1) * 4; class size = u * 8; a list always exists here
        local_get(&mut c, 1);
        i32_const(&mut c, 1);
        c.push(0x6B); // i32.sub
        i32_const(&mut c, 4);
        c.push(0x6C); // i32.mul
        i32_const(&mut c, FREE_TABLE_BASE);
        c.push(0x6A); // i32.add
        local_set(&mut c, 2);
        local_get(&mut c, 1);
        i32_const(&mut c, 8);
        c.push(0x6C);
        local_set(&mut c, 5);
        i32_const(&mut c, 1);
        local_set(&mut c, 6);
    }
    c.push(0x05); // else (power-of-two tier)
    {
        // power_idx = (32 - clz(size - 1)) - POWER_MIN, reusing local 1 (`u` is not needed again)
        i32_const(&mut c, 32 - POWER_MIN);
        local_get(&mut c, 0);
        i32_const(&mut c, 1);
        c.push(0x6B); // size - 1
        c.push(0x67); // i32.clz
        c.push(0x6B); // (32 - POWER_MIN) - clz(size - 1) = power_idx
        local_set(&mut c, 1);
        local_get(&mut c, 1);
        i32_const(&mut c, POWER_CLASSES);
        c.push(0x49); // i32.lt_u → power_idx < POWER_CLASSES
        local_set(&mut c, 6); // has_class
        local_get(&mut c, 6);
        c.push(0x04);
        c.push(0x40); // if (has a class)
        {
            local_get(&mut c, 1);
            i32_const(&mut c, FREE_CLASSES);
            c.push(0x6A);
            i32_const(&mut c, 4);
            c.push(0x6C);
            i32_const(&mut c, FREE_TABLE_BASE);
            c.push(0x6A);
            local_set(&mut c, 2); // slot
                                  // class size = 1 << (POWER_MIN + power_idx)
            i32_const(&mut c, 1);
            i32_const(&mut c, POWER_MIN);
            local_get(&mut c, 1);
            c.push(0x6A);
            c.push(0x74); // i32.shl
            local_set(&mut c, 5);
        }
        c.push(0x05); // else — past the last power class: bump the exact rounded size, no list
        {
            local_get(&mut c, 0);
            i32_const(&mut c, 7);
            c.push(0x6A);
            i32_const(&mut c, -8);
            c.push(0x71);
            local_set(&mut c, 5);
        }
        c.push(0x0B); // end if
    }
    c.push(0x0B); // end if/else

    local_get(&mut c, 6);
    c.push(0x04);
    c.push(0x40); // if (has_class)
    {
        local_get(&mut c, 2);
        i32_load(&mut c, 0);
        local_tee(&mut c, 3); // head
        c.push(0x04);
        c.push(0x40); // if head != 0
        {
            // table[slot] = *head  — the next free block, kept in the block's own first word
            local_get(&mut c, 2);
            local_get(&mut c, 3);
            i32_load(&mut c, 0);
            i32_store(&mut c, 0);
            local_get(&mut c, 3);
            local_set(&mut c, 4);
        }
        c.push(0x0B); // end if
    }
    c.push(0x0B); // end if

    // Nothing reusable: bump by this block's class size — growing linear memory first when the bump
    // would land past what is currently mapped, so a page's heap is bounded only by what the host
    // (or, under `StoreLimits`, the embedder) is willing to give it, not by a fixed slab size.
    local_get(&mut c, 4);
    c.push(0x45); // i32.eqz
    c.push(0x04);
    c.push(0x40); // if
    {
        global_get(&mut c, heap_global);
        i32_const(&mut c, 7);
        c.push(0x6A);
        i32_const(&mut c, -8);
        c.push(0x71); // i32.and → aligned p
        local_tee(&mut c, 4);
        local_get(&mut c, 5);
        c.push(0x6A); // p + class size
        local_set(&mut c, 7); // new_top

        // need_grow = (cur_pages * 65536) < new_top
        c.push(0x3F); // memory.size
        c.push(0x00); // memory index 0
        local_tee(&mut c, 8); // cur_pages
        i32_const(&mut c, 16);
        c.push(0x74); // i32.shl → cur_bytes
        local_get(&mut c, 7); // new_top
        c.push(0x49); // i32.lt_u
        c.push(0x04);
        c.push(0x40); // if (need_grow)
        {
            // pages_needed = ceil((new_top - cur_bytes) / 65536)
            local_get(&mut c, 7);
            local_get(&mut c, 8);
            i32_const(&mut c, 16);
            c.push(0x74); // i32.shl → cur_bytes (recomputed; cur_pages is still valid)
            c.push(0x6B); // i32.sub → new_top - cur_bytes
            i32_const(&mut c, 65535);
            c.push(0x6A);
            i32_const(&mut c, 16);
            c.push(0x76); // i32.shr_u
            local_set(&mut c, 9); // pages_needed

            // grow_by = max(pages_needed, cur_pages) — never less than doubling, so a run of small
            // allocations amortizes to O(1) `memory.grow` calls rather than one per block.
            local_get(&mut c, 9);
            local_get(&mut c, 8);
            c.push(0x49); // i32.lt_u → pages_needed < cur_pages
            c.push(0x04);
            c.push(0x40); // if
            {
                local_get(&mut c, 8);
                local_set(&mut c, 9); // grow_by = cur_pages
            }
            c.push(0x0B); // end if (local 9 already holds pages_needed otherwise)

            local_get(&mut c, 9);
            c.push(0x40); // memory.grow
            c.push(0x00);
            i32_const(&mut c, -1);
            c.push(0x46); // i32.eq — the host refused to grow
            c.push(0x04);
            c.push(0x40); // if
            {
                i32_const(
                    &mut c,
                    crate::semantics::failcode::FailCode::OutOfMemory.code(),
                );
                c.push(0x42); // i64.const 0 (the value this code ignores)
                leb_i64(&mut c, 0);
                c.push(0x10); // call logos_rt_fail_at(code, value)
                leb_u32(&mut c, fail_at);
                c.push(0x00); // unreachable
            }
            c.push(0x0B); // end if
        }
        c.push(0x0B); // end if (need_grow)

        local_get(&mut c, 7);
        global_set(&mut c, heap_global);
    }
    c.push(0x0B); // end if

    local_get(&mut c, 4);
    c.push(0x0B); // end function
    c
}

/// `__logos_seq_new(len: i32, handles: i32) -> i32` — a `Seq` handle over `len` zeroed 8-byte slots
/// for the HOST to fill: `len` scalars (`handles == 0`, a `ScalarHeader`) or `len` handles in each
/// slot's low word (`handles != 0`, a `HandleHeader`). Built the way [`emit_alloc`] builds every
/// module-side sequence — the class byte at word 12, and, where the module counts holders, a
/// 20-byte header with a true count of 1 credited to the turn — so the host's list is an ordinary
/// value the moment it is handed over.
fn emit_seq_new_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 2 = data, 3 = hdr, 4 = i
    c.push(0x01);
    leb_u32(&mut c, 3);
    c.push(I32);

    local_get(&mut c, 0);
    i32_const(&mut c, 8);
    c.push(0x6C); // i32.mul — 8-byte slots
    c.push(0x10); // call __logos_alloc(len * 8)
    leb_u32(&mut c, env.call(RuntimeFn::Alloc));
    local_set(&mut c, 2);

    // Zero the slots: a reused free-list block carries whatever it held before.
    i32_const(&mut c, 0);
    local_set(&mut c, 4);
    c.push(0x02);
    c.push(0x40); // block
    c.push(0x03);
    c.push(0x40); // loop
    local_get(&mut c, 4);
    local_get(&mut c, 0);
    c.push(0x4E); // i32.ge_s
    c.push(0x0D);
    leb_u32(&mut c, 1); // br_if block
    local_get(&mut c, 2);
    local_get(&mut c, 4);
    i32_const(&mut c, 8);
    c.push(0x6C);
    c.push(0x6A); // data + i*8
    c.push(0x42);
    leb_i64(&mut c, 0);
    i64_store(&mut c, 0);
    local_get(&mut c, 4);
    i32_const(&mut c, 1);
    c.push(0x6A);
    local_set(&mut c, 4);
    c.push(0x0C);
    leb_u32(&mut c, 0); // br loop
    c.push(0x0B); // end loop
    c.push(0x0B); // end block

    let counted = env.retain_base.is_some();
    i32_const(&mut c, if counted { 20 } else { 16 });
    c.push(0x10); // call __logos_alloc(header)
    leb_u32(&mut c, env.call(RuntimeFn::Alloc));
    local_set(&mut c, 3);

    local_get(&mut c, 3);
    local_get(&mut c, 0);
    i32_store(&mut c, 0); // header.len = len
    local_get(&mut c, 3);
    local_get(&mut c, 0);
    i32_store(&mut c, 4); // header.cap = len
    local_get(&mut c, 3);
    local_get(&mut c, 2);
    i32_store(&mut c, 8); // header.data_ptr = data
    local_get(&mut c, 3);
    i32_const(&mut c, AllocClass::HandleHeader.code() as i32);
    i32_const(&mut c, AllocClass::ScalarHeader.code() as i32);
    local_get(&mut c, 1);
    c.push(0x1B); // select — the class byte from `handles`
    i32_store(&mut c, 12);
    if counted {
        local_get(&mut c, 3);
        i32_const(&mut c, 1);
        i32_store(&mut c, 16); // a true count of 1, held by the turn
        local_get(&mut c, 3);
        c.push(0x10); // call __logos_turn_retain(hdr)
        leb_u32(&mut c, env.call(RuntimeFn::TurnRetain));
    }

    local_get(&mut c, 3); // return hdr
    c.push(0x0B); // end function
    c
}

/// `__logos_seq_slot(hdr: i32, index: i64) -> i32` — the address of the 1-based `index`'s 8-byte
/// slot, `data_ptr + (index - 1) * 8`, after the bounds check: an index below 1 or past `len`
/// reports `IndexOutOfBounds` with the index through `logos_rt_fail_at` (the host renders the
/// reference tier's sentence from the code and the value) and traps.
fn emit_seq_slot_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = vec![0x00]; // no locals
    local_get(&mut c, 1);
    c.push(0x42);
    leb_i64(&mut c, 1);
    c.push(0x53); // i64.lt_s → index < 1
    local_get(&mut c, 1);
    local_get(&mut c, 0);
    i32_load(&mut c, 0);
    c.push(0xAD); // i64.extend_i32_u → len
    c.push(0x55); // i64.gt_s → index > len
    c.push(0x72); // i32.or
    c.push(0x04);
    c.push(0x40);
    {
        i32_const(
            &mut c,
            crate::semantics::failcode::FailCode::IndexOutOfBounds.code(),
        );
        local_get(&mut c, 1);
        c.push(0x10);
        leb_u32(&mut c, env.fail_at);
        c.push(0x00); // unreachable
    }
    c.push(0x0B);
    local_get(&mut c, 0);
    i32_load(&mut c, 8); // data_ptr
    local_get(&mut c, 1);
    c.push(0x42);
    leb_i64(&mut c, 1);
    c.push(0x7D); // i64.sub
    c.push(0xA7); // i32.wrap_i64
    i32_const(&mut c, 8);
    c.push(0x6C);
    c.push(0x6A);
    c.push(0x0B); // end function
    c
}

/// `__logos_seq_push_slot(hdr: i32) -> i32` — the sequence one element longer, and the address of
/// its new last slot. AMORTIZED growth: when `len` has reached `cap` the buffer doubles (4 slots
/// from none), the live slots are copied across and the old buffer goes back to the allocator —
/// dead by construction, since a data buffer belongs to exactly one header and a shallow copy of
/// the sequence shares the header, not the buffer — so n pushes cost O(n) work, not O(n²).
fn emit_seq_push_slot_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 1 = len, 2 = cap, 3 = the new buffer, 4 = the new cap
    c.push(0x01);
    leb_u32(&mut c, 4);
    c.push(I32);
    local_get(&mut c, 0);
    i32_load(&mut c, 0);
    local_tee(&mut c, 1);
    local_get(&mut c, 0);
    i32_load(&mut c, 4);
    local_tee(&mut c, 2);
    c.push(0x4E); // i32.ge_s → full
    c.push(0x04);
    c.push(0x40);
    {
        i32_const(&mut c, 4);
        local_get(&mut c, 2);
        i32_const(&mut c, 2);
        c.push(0x6C);
        local_get(&mut c, 2);
        c.push(0x45); // i32.eqz
        c.push(0x1B); // select → cap == 0 ? 4 : cap * 2
        local_tee(&mut c, 4);
        i32_const(&mut c, 8);
        c.push(0x6C);
        c.push(0x10);
        leb_u32(&mut c, env.call(RuntimeFn::Alloc));
        local_tee(&mut c, 3);
        local_get(&mut c, 0);
        i32_load(&mut c, 8);
        local_get(&mut c, 1);
        i32_const(&mut c, 8);
        c.push(0x6C);
        memory_copy(&mut c); // every live slot
        local_get(&mut c, 0);
        i32_load(&mut c, 8);
        local_get(&mut c, 2);
        i32_const(&mut c, 8);
        c.push(0x6C);
        c.push(0x10); // call __logos_free(old data_ptr, cap * 8) — null frees nothing
        leb_u32(&mut c, env.call(RuntimeFn::Free));
        local_get(&mut c, 0);
        local_get(&mut c, 3);
        i32_store(&mut c, 8);
        local_get(&mut c, 0);
        local_get(&mut c, 4);
        i32_store(&mut c, 4);
    }
    c.push(0x0B);
    local_get(&mut c, 0);
    local_get(&mut c, 1);
    i32_const(&mut c, 1);
    c.push(0x6A);
    i32_store(&mut c, 0); // len + 1
    local_get(&mut c, 0);
    i32_load(&mut c, 8);
    local_get(&mut c, 1);
    i32_const(&mut c, 8);
    c.push(0x6C);
    c.push(0x6A); // data_ptr + len * 8
    c.push(0x0B); // end function
    c
}

/// `__logos_seq_clone` `mode` bit: each element of the copy gains the copy as a holder of its true
/// count (`__logos_retain_children`, on a module that counts holders).
const CLONE_RETAINS: i32 = 0x100;
/// `__logos_seq_clone` `mode` bit: each element of the copy gains a copy-on-write hold (word 12), so
/// a write to a record or collection through either list copies it first.
const CLONE_SHARES: i32 = 0x200;

/// `__logos_seq_clone(src: i32, mode: i32) -> i32` — a fresh, independent sequence (or set, or
/// record: the same `[len][cap][data_ptr][word 12]` header over 8-byte slots) holding `src`'s slots
/// as they are, `len` = `cap` = `src`'s `len`, its header stamped the class in `mode`'s low byte —
/// and, on a module that counts holders, born with a true count of 1 credited to the turn when that
/// class carries one, as [`emit_alloc`] births every counted block. The copy's elements gain what
/// `mode` says: a true count ([`CLONE_RETAINS`]), a copy-on-write hold ([`CLONE_SHARES`]); a deep
/// copy of each is the caller's.
fn emit_seq_clone_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 2 = hdr, 3 = data, 4 = i, 5 = len, 6 = counted (0/1)
    c.push(0x01);
    leb_u32(&mut c, 5);
    c.push(I32);
    let counting = env.retain_base.is_some();
    // A tagged integer buffer keeps its actual storage class across a copy.
    local_get(&mut c, 0);
    i32_load(&mut c, 12);
    i32_const(&mut c, 0xff);
    c.push(0x71);
    i32_const(&mut c, AllocClass::ExactIntegerHeader.code() as i32);
    c.extend([0x46, 0x04, 0x40]);
    local_get(&mut c, 1);
    i32_const(&mut c, !0xff);
    c.push(0x71);
    i32_const(&mut c, AllocClass::ExactIntegerHeader.code() as i32);
    c.push(0x72);
    local_set(&mut c, 1);
    c.push(0x0b);
    if counting {
        local_get(&mut c, 1);
        emit_class_is_counted(&mut c, 4);
        local_set(&mut c, 6);
    }
    i32_const(&mut c, 16);
    if counting {
        local_get(&mut c, 6);
        i32_const(&mut c, 2);
        c.push(0x74); // i32.shl → 4 bytes more for the count word
        c.push(0x6A);
    }
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::Alloc));
    local_set(&mut c, 2);
    local_get(&mut c, 2);
    local_get(&mut c, 1);
    i32_const(&mut c, 0xFF);
    c.push(0x71);
    i32_store(&mut c, 12); // the class byte; no copy-on-write hold yet
    if counting {
        local_get(&mut c, 6);
        c.push(0x04);
        c.push(0x40);
        {
            local_get(&mut c, 2);
            i32_const(&mut c, 1);
            i32_store(&mut c, 16);
            local_get(&mut c, 2);
            c.push(0x10);
            leb_u32(&mut c, env.call(RuntimeFn::TurnRetain));
        }
        c.push(0x0B);
    }
    local_get(&mut c, 0);
    i32_load(&mut c, 0);
    local_tee(&mut c, 5);
    if counting {
        // Records retain their field-kind bytes in addition to their eight-byte values.
        i32_const(&mut c, 9);
        i32_const(&mut c, 8);
        local_get(&mut c, 1);
        i32_const(&mut c, 0xFF);
        c.push(0x71);
        i32_const(&mut c, i32::from(AllocClass::RecordHeader.code()));
        c.push(0x46);
        c.push(0x1B);
    } else {
        i32_const(&mut c, 8);
    }
    c.push(0x6C);
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::Alloc));
    local_tee(&mut c, 3);
    local_get(&mut c, 0);
    i32_load(&mut c, 8);
    local_get(&mut c, 5);
    if counting {
        // Records retain their field-kind bytes in addition to their eight-byte values.
        i32_const(&mut c, 9);
        i32_const(&mut c, 8);
        local_get(&mut c, 1);
        i32_const(&mut c, 0xFF);
        c.push(0x71);
        i32_const(&mut c, i32::from(AllocClass::RecordHeader.code()));
        c.push(0x46);
        c.push(0x1B);
    } else {
        i32_const(&mut c, 8);
    }
    c.push(0x6C);
    memory_copy(&mut c);
    for off in [0u32, 4] {
        local_get(&mut c, 2);
        local_get(&mut c, 5);
        i32_store(&mut c, off);
    }
    local_get(&mut c, 2);
    local_get(&mut c, 3);
    i32_store(&mut c, 8);
    if counting {
        local_get(&mut c, 1);
        i32_const(&mut c, CLONE_RETAINS);
        c.push(0x71);
        c.push(0x04);
        c.push(0x40);
        local_get(&mut c, 2);
        c.push(0x10);
        leb_u32(&mut c, env.call(RuntimeFn::RetainChildren));
        c.push(0x0B);
    }
    local_get(&mut c, 1);
    i32_const(&mut c, CLONE_SHARES);
    c.push(0x71);
    c.push(0x04);
    c.push(0x40);
    emit_cow_retain_each_element(&mut c, 2, 4);
    c.push(0x0B);
    if let Some(clone) = env.exact_clone {
        emit_exact_buffer_holds(&mut c, 2, 4, 6, clone, true);
    }
    local_get(&mut c, 2);
    c.push(0x0B); // end function
    c
}

/// `__logos_add_checked` / `__logos_sub_checked` / `__logos_mul_checked(a: i64, b: i64) -> i64` — the
/// Int result, or a trap on signed overflow: the very check a site inlines inside a loop
/// ([`super::encode::emit_checked_addsub`], [`super::encode::emit_checked_mul`]), on the parameters.
fn emit_checked_arith_helper(op: ArithOp) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 2 = the result
    c.push(0x01);
    leb_u32(&mut c, 1);
    c.push(I64);
    match op {
        ArithOp::Add => emit_checked_addsub(&mut c, false, 2, 0, 1),
        ArithOp::Sub => emit_checked_addsub(&mut c, true, 2, 0, 1),
        ArithOp::Mul => emit_checked_mul(&mut c, 2, 0, 1),
        ArithOp::Div | ArithOp::Mod => {
            unreachable!("a division lowers inline; the runtime has no checked helper for it")
        }
    }
    local_get(&mut c, 2);
    c.push(0x0B); // end function
    c
}

/// `__logos_new_header(class: i32) -> i32` — see [`RuntimeFn::NewHeader`]: the header
/// [`emit_empty_header`] builds inline, once, over a class the call site passes.
fn emit_new_header_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 1 = hdr, 2 = counted (0/1), 3 = the class byte
    c.push(0x01);
    leb_u32(&mut c, 3);
    c.push(I32);
    let counting = env.retain_base.is_some();
    if counting {
        local_get(&mut c, 0);
        emit_class_is_counted(&mut c, 3);
        local_set(&mut c, 2);
    }
    i32_const(&mut c, 16);
    if counting {
        local_get(&mut c, 2);
        i32_const(&mut c, 2);
        c.push(0x74); // i32.shl → 4 bytes more for the count word
        c.push(0x6A);
    }
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::Alloc));
    local_set(&mut c, 1);
    for off in [0u32, 4, 8] {
        local_get(&mut c, 1);
        i32_const(&mut c, 0);
        i32_store(&mut c, off);
    }
    local_get(&mut c, 1);
    local_get(&mut c, 0);
    i32_store(&mut c, 12); // the class byte; no copy-on-write hold yet
    if counting {
        local_get(&mut c, 2);
        c.push(0x04);
        c.push(0x40);
        {
            local_get(&mut c, 1);
            i32_const(&mut c, 1);
            i32_store(&mut c, 16);
            local_get(&mut c, 1);
            c.push(0x10);
            leb_u32(&mut c, env.call(RuntimeFn::TurnRetain));
        }
        c.push(0x0B);
    }
    local_get(&mut c, 1);
    c.push(0x0B); // end function
    c
}

/// `__logos_share(handle: i32)` — see [`RuntimeFn::Share`].
fn emit_share_helper() -> Vec<u8> {
    let mut c = vec![0x00]; // no locals
    emit_share_inline(&mut c, |c| local_get(c, 0));
    c.push(0x0B); // end function
    c
}

/// `memory.copy` (bulk memory): `[dst, src, bytes]` on the stack, the bytes copied.
fn memory_copy(c: &mut Vec<u8>) {
    c.extend_from_slice(&[0xFC, 0x0A, 0x00, 0x00]);
}

/// `__logos_text_eq(a: i32, b: i32) -> i32` — `1` when the two Texts hold the same bytes (the same
/// handle does), else `0`: unequal lengths differ at once, equal ones compare byte by byte.
fn emit_text_eq_helper() -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 2 = i, 3 = len
    c.push(0x01);
    leb_u32(&mut c, 2);
    c.push(I32);
    local_get(&mut c, 0);
    local_get(&mut c, 1);
    c.push(0x46); // i32.eq — one handle
    c.push(0x04);
    c.push(0x40);
    i32_const(&mut c, 1);
    c.push(0x0F);
    c.push(0x0B);
    local_get(&mut c, 0);
    i32_load(&mut c, 0);
    local_tee(&mut c, 3);
    local_get(&mut c, 1);
    i32_load(&mut c, 0);
    c.push(0x47); // i32.ne — lengths differ
    c.push(0x04);
    c.push(0x40);
    i32_const(&mut c, 0);
    c.push(0x0F);
    c.push(0x0B);
    i32_const(&mut c, 0);
    local_set(&mut c, 2);
    c.push(0x02);
    c.push(0x40);
    c.push(0x03);
    c.push(0x40);
    local_get(&mut c, 2);
    local_get(&mut c, 3);
    c.push(0x4E); // i32.ge_s → every byte matched
    c.push(0x0D);
    leb_u32(&mut c, 1);
    for side in [0u32, 1] {
        local_get(&mut c, side);
        i32_load(&mut c, 8);
        local_get(&mut c, 2);
        c.push(0x6A);
        i32_load8_u(&mut c, 0);
    }
    c.push(0x47); // i32.ne
    c.push(0x04);
    c.push(0x40);
    i32_const(&mut c, 0);
    c.push(0x0F);
    c.push(0x0B);
    local_get(&mut c, 2);
    i32_const(&mut c, 1);
    c.push(0x6A);
    local_set(&mut c, 2);
    c.push(0x0C);
    leb_u32(&mut c, 0);
    c.push(0x0B);
    c.push(0x0B);
    i32_const(&mut c, 1);
    c.push(0x0B); // end function
    c
}

/// `__logos_text_concat(a: i32, b: i32) -> i32` — a fresh Text of `a`'s bytes then `b`'s: its header
/// classed [`AllocClass::TextHeader`] (born counted, credited to the turn, on a module that counts
/// holders — as [`emit_alloc`] births one), then its buffer of exactly `len(a) + len(b)` bytes.
fn emit_text_concat_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 2 = hdr, 3 = data, 4 = len(a)
    c.push(0x01);
    leb_u32(&mut c, 3);
    c.push(I32);
    let counting = env.retain_base.is_some();
    i32_const(&mut c, if counting { 20 } else { 16 });
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::Alloc));
    local_tee(&mut c, 2);
    i32_const(&mut c, i32::from(AllocClass::TextHeader.code()));
    i32_store(&mut c, 12);
    if counting {
        local_get(&mut c, 2);
        i32_const(&mut c, 1);
        i32_store(&mut c, 16);
        local_get(&mut c, 2);
        c.push(0x10);
        leb_u32(&mut c, env.call(RuntimeFn::TurnRetain));
    }
    local_get(&mut c, 0);
    i32_load(&mut c, 0);
    local_tee(&mut c, 4);
    local_get(&mut c, 1);
    i32_load(&mut c, 0);
    c.push(0x6A);
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::Alloc));
    local_tee(&mut c, 3);
    local_get(&mut c, 0);
    i32_load(&mut c, 8);
    local_get(&mut c, 4);
    memory_copy(&mut c); // a's bytes at [0, len(a))
    local_get(&mut c, 3);
    local_get(&mut c, 4);
    c.push(0x6A);
    local_get(&mut c, 1);
    i32_load(&mut c, 8);
    local_get(&mut c, 1);
    i32_load(&mut c, 0);
    memory_copy(&mut c); // b's bytes after them
    for off in [0u32, 4] {
        local_get(&mut c, 2);
        local_get(&mut c, 4);
        local_get(&mut c, 1);
        i32_load(&mut c, 0);
        c.push(0x6A);
        i32_store(&mut c, off);
    }
    local_get(&mut c, 2);
    local_get(&mut c, 3);
    i32_store(&mut c, 8);
    local_get(&mut c, 2);
    c.push(0x0B); // end function
    c
}

/// `__logos_map_clone(src: i32) -> i32` — a fresh, independent map holding `src`'s `[key][value]`
/// entries as they are (`num` = `cap` = `src`'s `num`), with `src`'s class byte and, in a program
/// that builds `SharedMap`s, `src`'s replica mark ([`SHARED_MAP_MARK`]): a `SharedMap`'s copy is
/// another replica of the same map, classified like it. On a module that counts holders every
/// handle key and value gains the copy as a holder (`__logos_retain_children`).
fn emit_map_clone_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 1 = hdr, 2 = data
    c.push(0x01);
    leb_u32(&mut c, 2);
    c.push(I32);
    i32_const(&mut c, i32::from(AllocClass::MapScalarScalar.code()));
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::NewHeader));
    local_set(&mut c, 1);
    local_get(&mut c, 0);
    i32_load(&mut c, 0);
    i32_const(&mut c, 16);
    c.push(0x6C);
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::Alloc));
    local_tee(&mut c, 2);
    local_get(&mut c, 0);
    i32_load(&mut c, 8);
    local_get(&mut c, 0);
    i32_load(&mut c, 0);
    i32_const(&mut c, 16);
    c.push(0x6C);
    memory_copy(&mut c);
    for off in [0u32, 4] {
        local_get(&mut c, 1);
        local_get(&mut c, 0);
        i32_load(&mut c, 0);
        i32_store(&mut c, off);
    }
    local_get(&mut c, 1);
    local_get(&mut c, 2);
    i32_store(&mut c, 8);
    local_get(&mut c, 1);
    if env.shared_maps {
        i32_const(&mut c, SHARED_MAP_MARK);
        i32_const(&mut c, 0);
        local_get(&mut c, 0);
        i32_load(&mut c, 12);
        i32_const(&mut c, 0);
        c.push(0x48); // i32.lt_s — a replica
        c.push(0x1B); // select
    } else {
        i32_const(&mut c, 0);
    }
    local_get(&mut c, 0);
    i32_load(&mut c, 12);
    i32_const(&mut c, 0xFF);
    c.push(0x71);
    c.push(0x72); // i32.or
    i32_store(&mut c, 12);
    if env.retain_base.is_some() {
        local_get(&mut c, 1);
        c.push(0x10);
        leb_u32(&mut c, env.call(RuntimeFn::RetainChildren));
    }
    local_get(&mut c, 1);
    c.push(0x0B); // end function
    c
}

/// `__logos_map_own(m: i32) -> i32` — the copy-on-write of a map about to be written: `m` when no
/// other holder shares it (or it is a `SharedMap` replica), else `__logos_map_clone(m)`.
fn emit_map_own_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = vec![0x00]; // no locals
    emit_should_clone(&mut c, 0);
    c.push(0x04);
    c.push(I32);
    local_get(&mut c, 0);
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::MapClone));
    c.push(0x05);
    local_get(&mut c, 0);
    c.push(0x0B);
    c.push(0x0B); // end function
    c
}

/// `__logos_map_find(m: i32, key: i64, text_key: i32) -> i32` — the address of the entry whose key
/// is `key` (a Text key by bytes, `key`'s low word its handle; an Int key by value), or 0.
fn emit_map_find_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 3 = i, 4 = the entry's address
    c.push(0x01);
    leb_u32(&mut c, 2);
    c.push(I32);
    i32_const(&mut c, 0);
    local_set(&mut c, 3);
    c.push(0x02);
    c.push(0x40);
    c.push(0x03);
    c.push(0x40);
    local_get(&mut c, 3);
    local_get(&mut c, 0);
    i32_load(&mut c, 0);
    c.push(0x4E); // i32.ge_s → every entry scanned
    c.push(0x0D);
    leb_u32(&mut c, 1);
    local_get(&mut c, 0);
    i32_load(&mut c, 8);
    local_get(&mut c, 3);
    i32_const(&mut c, 16);
    c.push(0x6C);
    c.push(0x6A);
    local_set(&mut c, 4);
    local_get(&mut c, 2);
    c.push(0x04);
    c.push(I32);
    {
        if let Some(cmp) = env.exact_cmp {
            local_get(&mut c, 1);
            c.push(0xa7);
            i32_load(&mut c, 12);
            i32_const(&mut c, 0xff);
            c.push(0x71);
            i32_const(&mut c, AllocClass::ExactIntegerHeader.code() as i32);
            c.extend([0x46, 0x04, I32]);
            local_get(&mut c, 4);
            i32_load(&mut c, 0);
            i32_load(&mut c, 8);
            i64_load(&mut c, 0);
            local_get(&mut c, 1);
            c.push(0xa7);
            i32_load(&mut c, 8);
            i64_load(&mut c, 0);
            c.push(0x10);
            leb_u32(&mut c, cmp);
            c.extend([0x45, 0x05]);
        }
        local_get(&mut c, 4);
        i32_load(&mut c, 0);
        local_get(&mut c, 1);
        c.push(0xA7); // i32.wrap_i64
        c.push(0x10);
        leb_u32(&mut c, env.call(RuntimeFn::TextEq));
        if env.exact_cmp.is_some() {
            c.push(0x0b);
        }
    }
    c.push(0x05);
    {
        local_get(&mut c, 4);
        i64_load(&mut c, 0);
        local_get(&mut c, 1);
        c.push(0x51); // i64.eq
    }
    c.push(0x0B);
    c.push(0x04);
    c.push(0x40);
    local_get(&mut c, 4);
    c.push(0x0F);
    c.push(0x0B);
    local_get(&mut c, 3);
    i32_const(&mut c, 1);
    c.push(0x6A);
    local_set(&mut c, 3);
    c.push(0x0C);
    leb_u32(&mut c, 0);
    c.push(0x0B);
    c.push(0x0B);
    i32_const(&mut c, 0);
    c.push(0x0B); // end function
    c
}

/// `__logos_map_at(m: i32, key: i64, text_key: i32) -> i32` — [`emit_map_find_helper`]'s entry,
/// trapping when the map holds no such key (the tree-walker's "Key not found").
fn emit_map_at_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 3 = the entry's address
    c.push(0x01);
    leb_u32(&mut c, 1);
    c.push(I32);
    local_get(&mut c, 0);
    local_get(&mut c, 1);
    local_get(&mut c, 2);
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::MapFind));
    local_tee(&mut c, 3);
    c.push(0x45); // i32.eqz
    c.push(0x04);
    c.push(0x40);
    c.push(0x00); // unreachable
    c.push(0x0B);
    local_get(&mut c, 3);
    c.push(0x0B); // end function
    c
}

/// `__logos_map_put(m: i32, key: i64, class: i32) -> i32` — the address of the entry `key` names in
/// `m`, which the caller stores the value into at +8. `class` (the map's key/value handle-ness,
/// [`AllocClass::of_map`]) is stamped into the class byte first — the first insert is where a map's
/// kinds are first visible, and every later one writes the same byte — and says whether the key is
/// a Text. An absent key is appended: the entry buffer doubles when full (4 entries from none; the
/// old buffer goes back to the allocator, dead by construction), the key is written, the value slot
/// starts empty, and on a module that counts holders a Text key gains the map as its holder. A
/// `SharedMap`'s sorted insert is not this function's (its callers insert inline).
fn emit_map_put_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 3 = text key (0/1), 4 = the entry's address, 5 = the new buffer, 6 = the new cap
    c.push(0x01);
    leb_u32(&mut c, 4);
    c.push(I32);
    local_get(&mut c, 0);
    local_get(&mut c, 0);
    i32_load(&mut c, 12);
    i32_const(&mut c, !0xFFu32 as i32);
    c.push(0x71);
    local_get(&mut c, 2);
    c.push(0x72);
    i32_store(&mut c, 12);
    let key_handle: i32 = AllocClass::ALL
        .iter()
        .filter(|k| k.held_handles().0 && **k != AllocClass::HandleHeader)
        .map(|k| 1i32 << k.code())
        .sum();
    i32_const(&mut c, key_handle);
    local_get(&mut c, 2);
    c.push(0x76); // i32.shr_u
    i32_const(&mut c, 1);
    c.push(0x71);
    local_set(&mut c, 3);
    local_get(&mut c, 0);
    local_get(&mut c, 1);
    local_get(&mut c, 3);
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::MapFind));
    local_tee(&mut c, 4);
    c.push(0x04);
    c.push(0x40);
    local_get(&mut c, 4);
    c.push(0x0F); // an entry already holds the key
    c.push(0x0B);
    local_get(&mut c, 0);
    i32_load(&mut c, 0);
    local_get(&mut c, 0);
    i32_load(&mut c, 4);
    c.push(0x4E); // i32.ge_s → full
    c.push(0x04);
    c.push(0x40);
    {
        i32_const(&mut c, 4);
        local_get(&mut c, 0);
        i32_load(&mut c, 4);
        i32_const(&mut c, 2);
        c.push(0x6C);
        local_get(&mut c, 0);
        i32_load(&mut c, 4);
        c.push(0x45);
        c.push(0x1B); // select → cap == 0 ? 4 : cap * 2
        local_tee(&mut c, 6);
        i32_const(&mut c, 16);
        c.push(0x6C);
        c.push(0x10);
        leb_u32(&mut c, env.call(RuntimeFn::Alloc));
        local_tee(&mut c, 5);
        local_get(&mut c, 0);
        i32_load(&mut c, 8);
        local_get(&mut c, 0);
        i32_load(&mut c, 0);
        i32_const(&mut c, 16);
        c.push(0x6C);
        memory_copy(&mut c);
        local_get(&mut c, 0);
        i32_load(&mut c, 8);
        local_get(&mut c, 0);
        i32_load(&mut c, 4);
        i32_const(&mut c, 16);
        c.push(0x6C);
        c.push(0x10); // call __logos_free(old data_ptr, cap * 16) — null frees nothing
        leb_u32(&mut c, env.call(RuntimeFn::Free));
        local_get(&mut c, 0);
        local_get(&mut c, 5);
        i32_store(&mut c, 8);
        local_get(&mut c, 0);
        local_get(&mut c, 6);
        i32_store(&mut c, 4);
    }
    c.push(0x0B);
    local_get(&mut c, 0);
    i32_load(&mut c, 8);
    local_get(&mut c, 0);
    i32_load(&mut c, 0);
    i32_const(&mut c, 16);
    c.push(0x6C);
    c.push(0x6A);
    local_set(&mut c, 4);
    local_get(&mut c, 3);
    c.push(0x04);
    c.push(0x40);
    {
        local_get(&mut c, 4);
        local_get(&mut c, 1);
        c.push(0xA7);
        i32_store(&mut c, 0);
        if env.retain_base.is_some() {
            local_get(&mut c, 1);
            c.push(0xA7);
            c.push(0x10);
            leb_u32(&mut c, env.call(RuntimeFn::Retain));
        }
    }
    c.push(0x05);
    {
        local_get(&mut c, 4);
        local_get(&mut c, 1);
        i64_store(&mut c, 0);
    }
    c.push(0x0B);
    local_get(&mut c, 4);
    c.push(0x42);
    leb_i64(&mut c, 0);
    i64_store(&mut c, 8); // the value slot starts empty
    local_get(&mut c, 0);
    local_get(&mut c, 0);
    i32_load(&mut c, 0);
    i32_const(&mut c, 1);
    c.push(0x6A);
    i32_store(&mut c, 0);
    local_get(&mut c, 4);
    c.push(0x0B); // end function
    c
}

/// `__logos_seq_own(hdr: i32, mode: i32) -> i32` — the copy-on-write of a sequence about to be
/// written. [`emit_should_clone`]'s test: a `SharedMap` replica (word 12 negative) and a block no
/// other holder shares (no count above the class byte) are written in place, and `hdr` comes back;
/// anything else comes back as `__logos_seq_clone(hdr, mode)`, the writer's own copy.
fn emit_seq_own_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = vec![0x00]; // no locals
    emit_should_clone(&mut c, 0);
    c.push(0x04);
    c.push(I32); // if (result i32)
    local_get(&mut c, 0);
    local_get(&mut c, 1);
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::SeqClone));
    c.push(0x05); // else
    local_get(&mut c, 0);
    c.push(0x0B);
    c.push(0x0B); // end function
    c
}

/// `__logos_text_new(len: i32) -> i32` — a Text handle the HOST writes `len` raw bytes into, built
/// from the same allocator every module-side Text uses: a `len`-byte data buffer, then a
/// `[len][cap][data_ptr][extra_refs|class]` header pointed at it, classed [`AllocClass::TextHeader`]
/// (with a true-count word after it when the module reclaims).
fn emit_text_new_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 1 = data, 2 = hdr
    c.push(0x01);
    leb_u32(&mut c, 2);
    c.push(I32);

    local_get(&mut c, 0); // len
    c.push(0x10); // call __logos_alloc(len)
    leb_u32(&mut c, env.call(RuntimeFn::Alloc));
    local_set(&mut c, 1); // data

    // A counted Text (S12, reclaim on): the header carries the count word, the turn is its first
    // holder, and a safepoint frees it once nothing the module keeps holds it — which is how the
    // host's per-event texts (the event name and value it writes for every dispatch) come back.
    let counted = env.retain_base.is_some();
    i32_const(&mut c, if counted { 20 } else { 16 });
    c.push(0x10); // call __logos_alloc(header)
    leb_u32(&mut c, env.call(RuntimeFn::Alloc));
    local_set(&mut c, 2); // hdr

    local_get(&mut c, 2);
    local_get(&mut c, 0);
    i32_store(&mut c, 0); // header.len = len
    local_get(&mut c, 2);
    local_get(&mut c, 0);
    i32_store(&mut c, 4); // header.cap = len
    local_get(&mut c, 2);
    local_get(&mut c, 1);
    i32_store(&mut c, 8); // header.data_ptr = data
    local_get(&mut c, 2);
    i32_const(&mut c, AllocClass::TextHeader.code() as i32);
    i32_store(&mut c, 12);
    if counted {
        local_get(&mut c, 2);
        i32_const(&mut c, 1);
        i32_store(&mut c, 16); // a true count of 1, held by the turn
        local_get(&mut c, 2);
        c.push(0x10); // call __logos_turn_retain(hdr)
        leb_u32(&mut c, env.call(RuntimeFn::TurnRetain));
    }

    local_get(&mut c, 2); // return hdr
    c.push(0x0B); // end function
    c
}

/// `__logos_free(ptr: i32, size: i32)` — push a block back onto its size class's list.
///
/// ★ CALLED ONLY WHERE A BLOCK IS DEAD BY CONSTRUCTION. This is the allocator half of the work; the
/// release protocol that decides when a VALUE dies is a separate, larger piece. Until that lands the
/// one caller is the sequence-growth path, which allocates a bigger buffer, copies into it and
/// repoints `data_ptr` — the old buffer is an internal buffer of that one header and nothing else
/// can be holding it, so freeing it needs no refcount to be sound.
fn emit_free_helper() -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 2 = u, 3 = slot address, 4 = whether a free list exists for this size at all,
    // 5 = the power tier's class index (kept apart from `u`, which the 16-byte guard still needs)
    c.push(0x01);
    leb_u32(&mut c, 4);
    c.push(I32);

    local_get(&mut c, 0); // ptr — a null frees nothing
    c.push(0x04);
    c.push(0x40);
    {
        local_get(&mut c, 1);
        i32_const(&mut c, 7);
        c.push(0x6A);
        i32_const(&mut c, 3);
        c.push(0x76);
        local_set(&mut c, 2); // u = (size + 7) >> 3
        i32_const(&mut c, 0);
        local_set(&mut c, 4); // has_class = 0

        // 1 <= u <= FREE_CLASSES: the exact tier, always has a list.
        local_get(&mut c, 2);
        i32_const(&mut c, 0);
        c.push(0x4B); // i32.gt_u → u > 0
        local_get(&mut c, 2);
        i32_const(&mut c, FREE_CLASSES);
        c.push(0x4D); // i32.le_u
        c.push(0x71); // i32.and
        c.push(0x04);
        c.push(0x40); // if (exact tier)
        {
            local_get(&mut c, 2);
            i32_const(&mut c, 1);
            c.push(0x6B);
            i32_const(&mut c, 4);
            c.push(0x6C);
            i32_const(&mut c, FREE_TABLE_BASE);
            c.push(0x6A);
            local_set(&mut c, 3); // slot
            i32_const(&mut c, 1);
            local_set(&mut c, 4); // has_class = 1
        }
        c.push(0x05); // else — u > FREE_CLASSES: the power-of-two tier, same size math as the alloc side
        {
            // power_idx = (32 - POWER_MIN) - clz(size - 1)
            i32_const(&mut c, 32 - POWER_MIN);
            local_get(&mut c, 1);
            i32_const(&mut c, 1);
            c.push(0x6B); // size - 1
            c.push(0x67); // i32.clz
            c.push(0x6B);
            local_tee(&mut c, 5); // power_idx
            i32_const(&mut c, POWER_CLASSES);
            c.push(0x49); // i32.lt_u
            c.push(0x04);
            c.push(0x40); // if (has a class)
            {
                local_get(&mut c, 5);
                i32_const(&mut c, FREE_CLASSES);
                c.push(0x6A);
                i32_const(&mut c, 4);
                c.push(0x6C);
                i32_const(&mut c, FREE_TABLE_BASE);
                c.push(0x6A);
                local_set(&mut c, 3); // slot
                i32_const(&mut c, 1);
                local_set(&mut c, 4); // has_class = 1
            }
            c.push(0x0B); // end if — else past the last power class: no list, the block is dropped
        }
        c.push(0x0B); // end if/else

        local_get(&mut c, 4);
        c.push(0x04);
        c.push(0x40); // if (has_class)
        {
            // ★ A RECYCLED BLOCK MUST NOT CARRY A STALE REFCOUNT. An object header keeps its
            // `extra_refs` at word 12 and the builders write words 0, 4 and 8 only — they rely on
            // fresh bump memory being zero, which was true when nothing was ever reused. A block
            // handed back and later used as a header would otherwise start with whatever sat at
            // word 12, and for a recycled DATA BUFFER that is an element's value: `emit_cow` would
            // then read a huge count and clone on every mutation for the life of that object. Not
            // unsound — a clone is always safe — but it would have quietly undone the reuse this
            // allocator exists for, and only shown up as a page that got slower the longer it ran.
            //
            // Zeroed here rather than in `alloc` because only the free path knows the block is at
            // least 16 bytes: writing offset 12 of a smaller block would scribble on its neighbour.
            // A power-tier block is always at least 2 KiB, so this always fires there too.
            local_get(&mut c, 2);
            i32_const(&mut c, 2);
            c.push(0x4F); // i32.ge_u → the block is at least 16 bytes
            c.push(0x04);
            c.push(0x40); // if
            {
                local_get(&mut c, 0);
                i32_const(&mut c, 0);
                i32_store(&mut c, 12);
            }
            c.push(0x0B);

            // *ptr = table[slot]; table[slot] = ptr
            local_get(&mut c, 0);
            local_get(&mut c, 3);
            i32_load(&mut c, 0);
            i32_store(&mut c, 0);
            local_get(&mut c, 3);
            local_get(&mut c, 0);
            i32_store(&mut c, 0);
        }
        c.push(0x0B);
    }
    c.push(0x0B);

    c.push(0x0B); // end function
    c
}

/// S8's UAF-detection value: overwritten across a freed object's own header (never its data buffer
/// — `__logos_free` immediately threads word 0 with the free list's own "next" pointer regardless,
/// so poisoning it is moot; words 4/8/16 are not touched by anything else and stay this value until
/// the block is handed back out by a fresh `__logos_alloc`, which zeroes it again).
const HEAP_POISON: i32 = 0xDEAD_BEEFu32 as i32;

/// `__logos_turn_retain(handle)` — append `handle` to the S8 turn-retained list, growing its buffer
/// (double, `alloc`/`free`, the same amortized shape every other growable buffer in this backend
/// uses) when full. Two callers: [`emit_alloc`] for a fresh counted block (the turn is its first,
/// implicit holder) and [`emit_true_release_helper`] when a mid-turn release would otherwise take a
/// count to zero (deferred to the next safepoint instead of freed on the spot).
fn emit_turn_retain_helper(env: &RuntimeEnv) -> Vec<u8> {
    let Some(base) = env.retain_base else {
        return vec![0x00, 0x0B];
    };
    let (ptr_g, len_g, cap_g) = (base, base + 1, base + 2);
    let mut c = Vec::new();
    c.push(0x01);
    leb_u32(&mut c, 3);
    c.push(I32); // locals: 1 = new_cap, 2 = new_ptr, 3 = copy-loop i
    global_get(&mut c, len_g);
    global_get(&mut c, cap_g);
    c.push(0x48); // i32.lt_s → len < cap
    c.push(0x04);
    c.push(0x40); // if (room already): store and return
    {
        global_get(&mut c, ptr_g);
        global_get(&mut c, len_g);
        i32_const(&mut c, 4);
        c.push(0x6C);
        c.push(0x6A);
        local_get(&mut c, 0);
        i32_store(&mut c, 0);
        global_get(&mut c, len_g);
        i32_const(&mut c, 1);
        c.push(0x6A);
        global_set(&mut c, len_g);
        c.push(0x0F); // return
    }
    c.push(0x0B); // end if
                  // new_cap = cap == 0 ? 4 : cap * 2
    i32_const(&mut c, 4);
    global_get(&mut c, cap_g);
    i32_const(&mut c, 2);
    c.push(0x6C);
    global_get(&mut c, cap_g);
    c.push(0x45); // i32.eqz
    c.push(0x1B); // select
    local_set(&mut c, 1);
    // new_ptr = alloc(new_cap * 4)
    local_get(&mut c, 1);
    i32_const(&mut c, 4);
    c.push(0x6C);
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::Alloc));
    local_set(&mut c, 2);
    // for i in 0..len: new_ptr[i] = old_ptr[i]
    i32_const(&mut c, 0);
    local_set(&mut c, 3);
    c.push(0x02);
    c.push(0x40); // block
    c.push(0x03);
    c.push(0x40); // loop
    local_get(&mut c, 3);
    global_get(&mut c, len_g);
    c.push(0x4E); // i32.ge_s
    c.push(0x0D);
    leb_u32(&mut c, 1); // br_if exit
    local_get(&mut c, 2);
    local_get(&mut c, 3);
    i32_const(&mut c, 4);
    c.push(0x6C);
    c.push(0x6A);
    global_get(&mut c, ptr_g);
    local_get(&mut c, 3);
    i32_const(&mut c, 4);
    c.push(0x6C);
    c.push(0x6A);
    i32_load(&mut c, 0);
    i32_store(&mut c, 0);
    local_get(&mut c, 3);
    i32_const(&mut c, 1);
    c.push(0x6A);
    local_set(&mut c, 3);
    c.push(0x0C);
    leb_u32(&mut c, 0); // br loop
    c.push(0x0B); // end loop
    c.push(0x0B); // end block
                  // free the old buffer, if it had one
    global_get(&mut c, cap_g);
    c.push(0x04);
    c.push(0x40);
    {
        global_get(&mut c, ptr_g);
        global_get(&mut c, cap_g);
        i32_const(&mut c, 4);
        c.push(0x6C);
        c.push(0x10);
        leb_u32(&mut c, env.call(RuntimeFn::Free));
    }
    c.push(0x0B);
    local_get(&mut c, 2);
    global_set(&mut c, ptr_g);
    local_get(&mut c, 1);
    global_set(&mut c, cap_g);
    // store handle at [ptr, len]; len += 1
    global_get(&mut c, ptr_g);
    global_get(&mut c, len_g);
    i32_const(&mut c, 4);
    c.push(0x6C);
    c.push(0x6A);
    local_get(&mut c, 0);
    i32_store(&mut c, 0);
    global_get(&mut c, len_g);
    i32_const(&mut c, 1);
    c.push(0x6A);
    global_set(&mut c, len_g);
    c.push(0x0B); // end function
    c
}

/// `__logos_retain(handle)` — `mem[handle+16] += 1` for a block whose class carries a true count
/// ([`emit_class_is_counted`]), a no-op on a null handle and on every other block. The S8 true-count
/// analogue of [`emit_retain`] (word 12, COW-only, untouched by this) — called wherever a counted
/// value becomes held by a SECOND real, heap-resident holder. The class byte, not the caller's static
/// kind, decides: a map or a `Raw` block has no word 16 of its own, and a kind that names the wrong
/// shape must not write one into whatever follows it.
fn emit_true_retain_helper() -> Vec<u8> {
    let mut c = Vec::new();
    c.push(0x01);
    leb_u32(&mut c, 1);
    c.push(I32); // local 1 = class
    local_get(&mut c, 0);
    c.push(0x04);
    c.push(0x40); // if handle != 0
    {
        local_get(&mut c, 0);
        i32_load(&mut c, 12);
        emit_class_is_counted(&mut c, 1);
        c.push(0x04);
        c.push(0x40); // if the block carries a true count
        {
            local_get(&mut c, 0);
            local_get(&mut c, 0);
            i32_load(&mut c, 16);
            i32_const(&mut c, 1);
            c.push(0x6A);
            i32_store(&mut c, 16);
        }
        c.push(0x0B);
    }
    c.push(0x0B);
    c.push(0x0B); // end function
    c
}

/// `__logos_retain_children(handle)` — every handle the block holds gains it as a holder. The class
/// byte names the shape, as it does for the free cascade: a `HandleHeader`'s 8-byte slots up to
/// `len`; a map's 16-byte `[key][value]` entries up to `num`, the key word when its class names
/// handle keys and the value word when it names handle values ([`AllocClass::held_handles`]); no
/// slot at all for any other class. Each handle goes through `__logos_retain`, which leaves a block
/// whose own class carries no count (a record, a pooled literal) as it is. A no-op on a null handle.
/// Emit a visit to each owned child in a sequence or map. Each callback consumes one handle;
/// locals are private to this traversal and survive calls into recursive runtime helpers.

fn emit_class_has_field_kinds(c: &mut Vec<u8>, class: u32) {
    for (i, kind) in [
        AllocClass::RecordHeader,
        AllocClass::EnumHeader,
        AllocClass::ClosureHeader,
    ]
    .iter()
    .enumerate()
    {
        local_get(c, class);
        i32_const(c, i32::from(kind.code()));
        c.push(0x46);
        if i != 0 {
            c.push(0x72);
        }
    }
}

fn emit_heap_child_count(c: &mut Vec<u8>, handle: u32, class: u32) {
    local_get(c, handle);
    i32_load(c, 4);
    local_get(c, handle);
    i32_load(c, 0);
    local_get(c, class);
    i32_const(c, i32::from(AllocClass::EnumHeader.code()));
    c.push(0x46);
    local_get(c, class);
    i32_const(c, i32::from(AllocClass::ClosureHeader.code()));
    c.push(0x46);
    c.push(0x72);
    c.push(0x1B); // enum tags and closure function indices are not payload lengths
}

fn emit_each_heap_child(
    c: &mut Vec<u8>,
    handle: u32,
    locals: [u32; 5],
    visit: impl Fn(&mut Vec<u8>),
) {
    let [class, stride, flags, index, slot] = locals;
    let mask = |second: bool| -> i32 {
        AllocClass::ALL
            .iter()
            .filter(|k| {
                let words = k.held_handles();
                if second {
                    words.1
                } else {
                    words.0
                }
            })
            .map(|k| 1i32 << k.code())
            .sum()
    };
    local_get(c, handle);
    i32_load(c, 12);
    i32_const(c, 0xFF);
    c.push(0x71);
    local_set(c, class);
    for (bit, m) in [(0, mask(false)), (1, mask(true))] {
        i32_const(c, m);
        local_get(c, class);
        c.push(0x76);
        i32_const(c, 1);
        c.push(0x71);
        if bit == 1 {
            i32_const(c, 1);
            c.push(0x74);
            c.push(0x72);
        }
    }
    local_set(c, flags);
    local_get(c, class);
    i32_const(c, AllocClass::ALL.len() as i32);
    c.push(0x49);
    c.push(0x04);
    c.push(0x40);
    local_get(c, flags);
    c.push(0x04);
    c.push(0x40);
    i32_const(c, 8);
    i32_const(c, 16);
    local_get(c, class);
    i32_const(c, i32::from(AllocClass::HandleHeader.code()));
    c.push(0x46);
    emit_class_has_field_kinds(c, class);
    c.push(0x72);
    c.push(0x1B);
    local_set(c, stride);
    i32_const(c, 0);
    local_set(c, index);
    c.push(0x02);
    c.push(0x40);
    c.push(0x03);
    c.push(0x40);
    local_get(c, index);
    emit_heap_child_count(c, handle, class);
    c.push(0x4F);
    c.push(0x0D);
    leb_u32(c, 1);
    local_get(c, handle);
    i32_load(c, 8);
    local_get(c, index);
    local_get(c, stride);
    c.push(0x6C);
    c.push(0x6A);
    local_set(c, slot);
    for (bit, off) in [(1, 0), (2, 8)] {
        local_get(c, flags);
        i32_const(c, bit);
        c.push(0x71);
        c.push(0x45);
        c.push(0x45); // normalize either handle bit (1 or 2) before combining with a field flag
        emit_class_has_field_kinds(c, class);
        c.push(0x04);
        c.push(I32);
        local_get(c, handle);
        i32_load(c, 8);
        emit_heap_child_count(c, handle, class);
        i32_const(c, 16);
        i32_const(c, 8);
        local_get(c, class);
        i32_const(c, i32::from(AllocClass::ClosureHeader.code()));
        c.push(0x46);
        c.push(0x1B); // closures place present flags between values and ownership bytes
        c.push(0x6C);
        c.push(0x6A);
        local_get(c, index);
        c.push(0x6A);
        i32_load8_u(c, 0);
        c.push(0x05);
        i32_const(c, 1);
        c.push(0x0B);
        c.push(0x71);
        c.push(0x04);
        c.push(0x40);
        local_get(c, slot);
        i32_load(c, off);
        visit(c);
        c.push(0x0B);
    }
    local_get(c, index);
    i32_const(c, 1);
    c.push(0x6A);
    local_set(c, index);
    c.push(0x0C);
    leb_u32(c, 0);
    c.push(0x0B);
    c.push(0x0B);
    c.push(0x0B);
    c.push(0x0B);
}

fn emit_retain_children_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = vec![0x01];
    leb_u32(&mut c, 5);
    c.push(I32);
    local_get(&mut c, 0);
    c.push(0x04);
    c.push(0x40);
    emit_each_heap_child(&mut c, 0, [1, 2, 3, 4, 5], |c| {
        c.push(0x10);
        leb_u32(c, env.call(RuntimeFn::Retain));
    });
    c.push(0x0B);
    c.push(0x0B);
    c
}

/// A new host registration owns its closure before the imported host is entered.
/// The registry stores `[identity, closure, next]`; released entries may be reused, identities may not.
fn emit_closure_retain_helper(env: &RuntimeEnv) -> Vec<u8> {
    let Some(base) = env.callback_base else {
        return vec![0x00, 0x00, 0x0B];
    };
    let mut c = vec![0x01];
    leb_u32(&mut c, 1);
    c.push(I32); // local 1: registry entry
    global_get(&mut c, base + 1);
    i32_const(&mut c, -1);
    c.push(0x46);
    c.extend([0x04, 0x40, 0x00, 0x0B]); // never wrap and reuse an identity
    i32_const(&mut c, 12);
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::Alloc));
    local_set(&mut c, 1);
    global_get(&mut c, base + 1);
    i32_const(&mut c, 1);
    c.push(0x6A);
    global_set(&mut c, base + 1);
    local_get(&mut c, 1);
    global_get(&mut c, base + 1);
    i32_store(&mut c, 0);
    local_get(&mut c, 1);
    local_get(&mut c, 0);
    i32_store(&mut c, 4);
    local_get(&mut c, 1);
    global_get(&mut c, base);
    i32_store(&mut c, 8);
    local_get(&mut c, 1);
    global_set(&mut c, base);
    local_get(&mut c, 0);
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::Retain));
    global_get(&mut c, base + 1);
    c.push(0x0B);
    c
}

/// Remove one host registration. Other registrations of the same closure keep their own references.
/// The closure's final release joins the ordinary turn worklist; no captured value is freed mid-turn.
fn emit_closure_release_helper(env: &RuntimeEnv) -> Vec<u8> {
    let Some(base) = env.callback_base else {
        return vec![0x00, 0x0B];
    };
    let mut c = vec![0x01];
    leb_u32(&mut c, 2);
    c.push(I32); // locals 1: entry, 2: preceding entry
    global_get(&mut c, base);
    local_set(&mut c, 1);
    c.extend([0x02, 0x40, 0x03, 0x40]);
    local_get(&mut c, 1);
    c.push(0x45);
    c.push(0x0D);
    leb_u32(&mut c, 1);
    local_get(&mut c, 1);
    i32_load(&mut c, 0);
    local_get(&mut c, 0);
    c.push(0x46);
    c.extend([0x04, 0x40]);
    local_get(&mut c, 2);
    c.extend([0x04, 0x40]);
    local_get(&mut c, 2);
    local_get(&mut c, 1);
    i32_load(&mut c, 8);
    i32_store(&mut c, 8);
    c.push(0x05);
    local_get(&mut c, 1);
    i32_load(&mut c, 8);
    global_set(&mut c, base);
    c.push(0x0B);
    local_get(&mut c, 1);
    i32_load(&mut c, 4);
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::Release));
    local_get(&mut c, 1);
    i32_const(&mut c, 12);
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::Free));
    c.push(0x0F);
    c.push(0x0B);
    local_get(&mut c, 1);
    local_set(&mut c, 2);
    local_get(&mut c, 1);
    i32_load(&mut c, 8);
    local_set(&mut c, 1);
    c.push(0x0C);
    leb_u32(&mut c, 0);
    c.extend([0x0B, 0x0B, 0x0B]);
    c
}

/// `__logos_release(handle)` — the MID-TURN release: `mem[handle+16] -= 1`, UNLESS that would reach
/// 0, in which case the count is left alone and `handle` is pushed back onto the turn's retained
/// list instead — "a mid-turn drop to 0 re-lists it on the turn" (S8's design). A no-op on a null
/// handle and on a block whose class carries no true count ([`emit_true_retain_helper`]'s rule —
/// a map listed here would be freed at the safepoint with its entries still reachable). Never frees
/// anything itself; only [`emit_safepoint_helper`]'s drain does that.
fn emit_true_release_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    c.push(0x01);
    leb_u32(&mut c, 1);
    c.push(I32); // local 1 = n
    local_get(&mut c, 0);
    c.push(0x04);
    c.push(0x40); // if handle != 0
    {
        local_get(&mut c, 0);
        i32_load(&mut c, 12);
        emit_class_is_counted(&mut c, 1);
        c.push(0x45); // i32.eqz
        c.push(0x0D); // br_if 0 — leave the `if`: nothing to release
        leb_u32(&mut c, 0);
        local_get(&mut c, 0);
        i32_load(&mut c, 16);
        local_set(&mut c, 1);
        local_get(&mut c, 1);
        i32_const(&mut c, 1);
        c.push(0x4C); // i32.le_s → n <= 1
        c.push(0x04);
        c.push(0x40); // if (would hit 0): defer to the turn list
        {
            local_get(&mut c, 0);
            c.push(0x10);
            leb_u32(&mut c, env.call(RuntimeFn::TurnRetain));
        }
        c.push(0x05); // else: a real remaining holder, plain decrement
        {
            local_get(&mut c, 0);
            local_get(&mut c, 1);
            i32_const(&mut c, 1);
            c.push(0x6B);
            i32_store(&mut c, 16);
        }
        c.push(0x0B); // end if/else
    }
    c.push(0x0B); // end outer if
    c.push(0x0B); // end function
    c
}

/// `__logos_free_object(handle)` — the free cascade's own worker, called ONLY from
/// [`emit_safepoint_helper`]'s worklist (never mid-turn): `handle`'s true count has JUST reached 0.
/// For a `HandleHeader` (class 2), each element slot (an i32 handle, one per 8-byte slot up to
/// `num`) is released AUTHORITATIVELY — an unconditional decrement, recursing into this SAME
/// function when THAT reaches 0 too. A `Raw`-classified child (an immortal `Text`/`Struct`/`Enum`
/// element, class byte 0) is left alone: it was never counted, so it is never freed by this either
/// — the exact, permanent, SAFE leak `AllocClass::is_counted`'s doc names, not a bug.
///
/// The recursion here is ordinary wasm `call`s, not an explicit worklist array, because this
/// session's counted classes cannot nest more than one level deep: a `HandleHeader`'s own elements
/// are either `Raw` (no further children at all) or `ScalarHeader` (counted, but itself a LEAF —
/// nothing inside a plain numeric/boolean sequence is ever a handle). `Map` — the one shape that
/// COULD recurse arbitrarily — is not itself counted this session (see `AllocClass::is_counted`),
/// so it can never appear as a child here. An unboundedly deep counted graph is out of today's
/// scope, not silently mishandled.
///
/// Frees the data buffer (`cap` slots at 8 bytes each) and the 20-byte header, POISONing the header
/// first (see [`HEAP_POISON`]).
/// `[word 12] -> [counted]`: whether a header's class byte (the low byte of word 12) names a class
/// that carries a true holder count at word 16 ([`AllocClass::is_counted`]). A `Raw` block, a map
/// header (classified, never counted) and anything else answers 0 — none of them has a word 16 of its
/// own, so nothing may read or write one there. `class_local` is a free i32 local; it is left holding
/// the class byte.
fn emit_class_is_counted(c: &mut Vec<u8>, class_local: u32) {
    let mask: u32 = AllocClass::ALL
        .iter()
        .filter(|k| k.is_counted())
        .map(|k| 1u32 << k.code())
        .sum();
    let classes = AllocClass::ALL.len() as i32;
    i32_const(c, 0xFF);
    c.push(0x71); // i32.and → class
    local_tee(c, class_local);
    i32_const(c, classes);
    c.push(0x49); // i32.lt_u → a class this backend stamps
    i32_const(c, mask as i32);
    local_get(c, class_local);
    c.push(0x76); // i32.shr_u
    i32_const(c, 1);
    c.push(0x71); // i32.and → its bit in the counted mask
    c.push(0x71); // i32.and
}

fn emit_exact_buffer_holds(
    code: &mut Vec<u8>,
    hdr: u32,
    i: u32,
    slot: u32,
    host: u32,
    cloning: bool,
) {
    local_get(code, hdr);
    i32_load(code, 12);
    i32_const(code, 0xff);
    code.push(0x71);
    i32_const(code, AllocClass::ExactIntegerHeader.code() as i32);
    code.extend([0x46, 0x04, 0x40]);
    i32_const(code, 0);
    local_set(code, i);
    code.extend([0x02, 0x40, 0x03, 0x40]);
    local_get(code, i);
    local_get(code, hdr);
    i32_load(code, 0);
    code.extend([0x4f, 0x0d]);
    leb_u32(code, 1);
    local_get(code, hdr);
    i32_load(code, 8);
    local_get(code, i);
    i32_const(code, 8);
    code.extend([0x6c, 0x6a]);
    local_tee(code, slot);
    i64_load(code, 0);
    code.push(0x42);
    leb_i64(code, 1);
    code.extend([0x83, 0xa7, 0x04, 0x40]);
    if cloning {
        local_get(code, slot);
    }
    local_get(code, slot);
    i64_load(code, 0);
    code.push(0x10);
    leb_u32(code, host);
    if cloning {
        i64_store(code, 0);
    }
    code.push(0x0b);
    local_get(code, i);
    i32_const(code, 1);
    code.push(0x6a);
    local_set(code, i);
    code.push(0x0c);
    leb_u32(code, 0);
    code.extend([0x0b, 0x0b, 0x0b]);
}

fn emit_free_object_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 1 = class, 2 = stride, 3 = flags/data, 4 = i, 5 = child, 6 = n, 7 = slot
    c.push(0x01);
    leb_u32(&mut c, 7);
    c.push(I32);
    local_get(&mut c, 0);
    c.push(0x04);
    c.push(0x40); // if handle != 0
    {
        local_get(&mut c, 0);
        i32_load(&mut c, 12);
        i32_const(&mut c, 0xFF);
        c.push(0x71); // i32.and → class
        local_set(&mut c, 1);
        emit_each_heap_child(&mut c, 0, [1, 2, 3, 4, 7], |c| {
            local_set(c, 5); // child
            local_get(c, 5);
            c.push(0x04);
            c.push(0x40); // if child != 0
            {
                local_get(c, 5);
                i32_load(c, 12);
                emit_class_is_counted(c, 6);
                c.push(0x04);
                c.push(0x40); // if the child's own class carries a true count
                {
                    local_get(c, 5);
                    local_get(c, 5);
                    i32_load(c, 16);
                    i32_const(c, 1);
                    c.push(0x6B); // n - 1
                    local_tee(c, 6);
                    i32_store(c, 16);
                    local_get(c, 6);
                    i32_const(c, 0);
                    c.push(0x4C); // i32.le_s → n <= 0
                    c.push(0x04);
                    c.push(0x40);
                    {
                        local_get(c, 5);
                        c.push(0x10); // call __logos_free_object(child) — recursion
                        leb_u32(c, env.call(RuntimeFn::FreeObject));
                    }
                    c.push(0x0B); // end if n<=0
                }
                c.push(0x0B); // end if class!=Raw
            }
            c.push(0x0B); // end if child!=0
        });
        if let Some(drop) = env.exact_drop {
            emit_exact_buffer_holds(&mut c, 0, 4, 7, drop, false);
        }
        // Free the data buffer — `emit_free_helper` no-ops a null ptr. A list's holds `cap` 8-byte
        // slots; a Text's holds `cap` bytes, unless it is an append's growable buffer, which other
        // views may still be reading: that loses one view, and goes with its last.
        local_get(&mut c, 1);
        i32_const(&mut c, AllocClass::TextHeader.code() as i32);
        c.push(0x46); // i32.eq
        c.push(0x04);
        c.push(0x40); // if a Text
        {
            local_get(&mut c, 0);
            i32_load(&mut c, 4);
            i32_const(&mut c, TEXT_GROWABLE);
            c.push(0x71);
            c.push(0x45); // i32.eqz → not growable
            c.push(0x04);
            c.push(0x40);
            {
                local_get(&mut c, 0);
                i32_load(&mut c, 8);
                local_get(&mut c, 0);
                i32_load(&mut c, 4);
                c.push(0x10);
                leb_u32(&mut c, env.call(RuntimeFn::Free));
            }
            c.push(0x05); // else: one view fewer reads the buffer; the last frees it
            {
                local_get(&mut c, 0);
                i32_load(&mut c, 8);
                i32_const(&mut c, 8);
                c.push(0x6B);
                local_set(&mut c, 3); // the buffer's base (its view count)
                local_get(&mut c, 3);
                local_get(&mut c, 3);
                i32_load(&mut c, 0);
                i32_const(&mut c, 1);
                c.push(0x6B);
                local_tee(&mut c, 6);
                i32_store(&mut c, 0);
                local_get(&mut c, 6);
                i32_const(&mut c, 0);
                c.push(0x4C); // i32.le_s → no view left
                c.push(0x04);
                c.push(0x40);
                {
                    local_get(&mut c, 3);
                    local_get(&mut c, 0);
                    i32_load(&mut c, 4);
                    i32_const(&mut c, i32::MAX);
                    c.push(0x71); // the capacity bytes
                    i32_const(&mut c, 8);
                    c.push(0x6A); // + the view count and mark words
                    c.push(0x10);
                    leb_u32(&mut c, env.call(RuntimeFn::Free));
                }
                c.push(0x0B);
            }
            c.push(0x0B);
        }
        c.push(0x05); // else: a list
        {
            local_get(&mut c, 0);
            i32_load(&mut c, 8);
            local_get(&mut c, 0);
            i32_load(&mut c, 4);
            i32_const(&mut c, 17);
            i32_const(&mut c, CRDT_RECORD_BYTES);
            i32_const(&mut c, 16);
            i32_const(&mut c, 9);
            i32_const(&mut c, 8);
            emit_class_has_field_kinds(&mut c, 1);
            c.push(0x1B);
            local_get(&mut c, 1);
            i32_const(&mut c, i32::from(AllocClass::MapScalarScalar.code()));
            c.push(0x4F);
            local_get(&mut c, 1);
            i32_const(&mut c, i32::from(AllocClass::MapHandleHandle.code()));
            c.push(0x4D);
            c.push(0x71);
            c.push(0x1B); // maps own 16-byte entries, sequences own 8-byte slots
            local_get(&mut c, 1);
            i32_const(&mut c, i32::from(AllocClass::CounterHeader.code()));
            c.push(0x46);
            c.push(0x1B); // counters own 24-byte replica records
            local_get(&mut c, 1);
            i32_const(&mut c, i32::from(AllocClass::ClosureHeader.code()));
            c.push(0x46);
            c.push(0x1B); // each capture has a value, a present flag, and an ownership byte
            c.push(0x6C);
            c.push(0x10);
            leb_u32(&mut c, env.call(RuntimeFn::Free));
        }
        c.push(0x0B);
        // POISON the header (word 0/4/8/16; word 0 is about to be overwritten by the free list
        // linkage regardless — see `HEAP_POISON`'s doc), then free the header itself (20 bytes).
        for off in [0u32, 4, 8, 16] {
            local_get(&mut c, 0);
            i32_const(&mut c, HEAP_POISON);
            i32_store(&mut c, off);
        }
        local_get(&mut c, 0);
        i32_const(&mut c, 20);
        c.push(0x10);
        leb_u32(&mut c, env.call(RuntimeFn::Free));
    }
    c.push(0x0B); // end outer if
    c.push(0x0B); // end function
    c
}

/// `__logos_safepoint()` — run when the host's own re-entry depth returns to 0 (S8; the wiring that
/// calls this from `runtime.js`'s own depth tracking lands separately — a test/host driver calls it
/// directly today). Drains the ENTIRE turn-retained list into a local snapshot first (so a deferred
/// re-list from inside this very drain — S8's "count back to 1, listed once" — starts a fresh list
/// for the NEXT turn rather than being walked twice this one): each entry's count drops by exactly
/// 1 (the turn's own implicit hold ending); whatever reaches 0 is freed via
/// [`emit_free_object_helper`]. Nothing is freed anywhere else.
fn emit_safepoint_helper(env: &RuntimeEnv) -> Vec<u8> {
    let Some(base) = env.retain_base else {
        return vec![0x00, 0x0B];
    };
    let (ptr_g, len_g, cap_g) = (base, base + 1, base + 2);
    let mut c = Vec::new();
    // `__logos_safepoint` takes NO params: locals 0 = snapshot cap, 1 = snapshot ptr, 2 = snapshot
    // len, 3 = i, 4 = handle, 5 = n.
    c.push(0x01);
    leb_u32(&mut c, 6);
    c.push(I32);
    global_get(&mut c, ptr_g);
    local_set(&mut c, 1);
    global_get(&mut c, len_g);
    local_set(&mut c, 2);
    global_get(&mut c, cap_g);
    local_set(&mut c, 0); // the snapshot's capacity — what its buffer was allocated for
                          // reset the list to empty BEFORE processing — this buffer becomes the NEXT turn's own list; a
                          // deferred re-list mid-drain (`emit_true_release_helper` calling `__logos_turn_retain` again,
                          // reached only through a cascade this safepoint itself triggers — not expected today, since the
                          // cascade uses the unconditional `emit_free_object_helper` path, not the mid-turn one, but kept
                          // safe regardless) grows this fresh buffer rather than corrupting the snapshot being walked.
    i32_const(&mut c, 0);
    global_set(&mut c, ptr_g);
    i32_const(&mut c, 0);
    global_set(&mut c, len_g);
    i32_const(&mut c, 0);
    global_set(&mut c, cap_g);
    i32_const(&mut c, 0);
    local_set(&mut c, 3); // i
    c.push(0x02);
    c.push(0x40); // block $exit
    c.push(0x03);
    c.push(0x40); // loop $loop
    local_get(&mut c, 3);
    local_get(&mut c, 2);
    c.push(0x4E); // i32.ge_s → i >= snapshot len
    c.push(0x0D);
    leb_u32(&mut c, 1); // br_if $exit
    local_get(&mut c, 1);
    local_get(&mut c, 3);
    i32_const(&mut c, 4);
    c.push(0x6C);
    c.push(0x6A);
    i32_load(&mut c, 0);
    local_set(&mut c, 4); // handle
    local_get(&mut c, 4);
    c.push(0x04);
    c.push(0x40); // if handle != 0
    {
        local_get(&mut c, 4);
        local_get(&mut c, 4);
        i32_load(&mut c, 16);
        i32_const(&mut c, 1);
        c.push(0x6B); // n - 1
        local_tee(&mut c, 5);
        i32_store(&mut c, 16);
        local_get(&mut c, 5);
        i32_const(&mut c, 0);
        c.push(0x4C); // i32.le_s → n <= 0
        c.push(0x04);
        c.push(0x40);
        {
            local_get(&mut c, 4);
            c.push(0x10);
            leb_u32(&mut c, env.call(RuntimeFn::FreeObject));
        }
        c.push(0x0B); // end if
    }
    c.push(0x0B); // end if handle!=0
    local_get(&mut c, 3);
    i32_const(&mut c, 1);
    c.push(0x6A);
    local_set(&mut c, 3);
    c.push(0x0C);
    leb_u32(&mut c, 0); // br $loop
    c.push(0x0B); // end loop
    c.push(0x0B); // end block
                  // free the drained snapshot buffer itself (its slots are gone — every entry was consumed above),
                  // at the size it was allocated for, its capacity
    local_get(&mut c, 1);
    local_get(&mut c, 0);
    i32_const(&mut c, 4);
    c.push(0x6C);
    c.push(0x10);
    leb_u32(&mut c, env.call(RuntimeFn::Free));
    c.push(0x0B); // end function
    c
}

/// `__logos_heap_census() -> i32` — CHECK-MODE ONLY, not part of a page's normal operation. Walks
/// every counted value reachable from the program's own globals and, for each one, INDEPENDENTLY
/// recomputes how many roots/slots point at it (a scratch array indexed by `(handle - heap_base) /
/// 8`, sized to the current heap and laid just above the bump pointer, outside the heap), then
/// compares that recomputed
/// in-degree to the block's own stored true count. Returns 0 when every reachable object's stored
/// count exactly equals its recomputed in-degree, or the address of the first mismatch otherwise.
///
/// Two full root-walks share one Rust closure (`visit`) rather than duplicating the traversal by
/// hand: the first increments the scratch slot for every handle visited, the second compares each
/// visited slot to its header's stored count, stopping at the first disagreement. The traversal
/// itself never needs to recurse more than one level (see [`emit_free_object_helper`]'s doc for why
/// a `HandleHeader`'s own elements cannot nest further this session).
fn emit_heap_census_visit_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = vec![0x01];
    // params: 0 = handle, 1 = scratch; locals 2..6 traverse children, 7 = cell, 8 = old count.
    leb_u32(&mut c, 7);
    c.push(I32);
    local_get(&mut c, 0);
    c.push(0x45);
    c.push(0x04);
    c.push(0x40);
    c.push(0x0F);
    c.push(0x0B);
    local_get(&mut c, 0);
    i32_load(&mut c, 12);
    emit_class_is_counted(&mut c, 2);
    c.push(0x45);
    c.push(0x04);
    c.push(0x40);
    c.push(0x0F);
    c.push(0x0B);
    local_get(&mut c, 1);
    local_get(&mut c, 0);
    i32_const(&mut c, env.heap_base);
    c.push(0x6B);
    i32_const(&mut c, 3);
    c.push(0x76);
    i32_const(&mut c, 4);
    c.push(0x6C);
    c.push(0x6A);
    local_tee(&mut c, 7);
    i32_load(&mut c, 0);
    local_set(&mut c, 8);
    local_get(&mut c, 7);
    local_get(&mut c, 8);
    i32_const(&mut c, 1);
    c.push(0x6A);
    i32_store(&mut c, 0);
    local_get(&mut c, 8);
    c.push(0x04);
    c.push(0x40);
    c.push(0x0F); // an already-visited container's owned edges were counted once
    c.push(0x0B);
    emit_each_heap_child(&mut c, 0, [2, 3, 4, 5, 6], |c| {
        local_get(c, 1);
        c.push(0x10);
        leb_u32(c, env.call(RuntimeFn::CensusVisit));
    });
    c.push(0x0B);
    c
}

fn emit_heap_census_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 0 = scratch_ptr, 1 = scratch_words, 2 = mismatch, 3 = handle, 4 = class, 5 = num,
    // 6 = data, 7 = i, 8 = child, 9 = slot, 10 = tmp
    c.push(0x01);
    leb_u32(&mut c, 11);
    c.push(I32);
    // scratch_words = (heap_ptr - heap_base) / 8 + 1, one per 8-byte slot of the heap. The scratch
    // sits just ABOVE the bump pointer, which never moves for it: the census observes the heap
    // without allocating in it, so it neither perturbs the layout it checks nor grows the heap it
    // will measure next time. Memory grows to cover it when it must.
    global_get(&mut c, env.heap_global);
    i32_const(&mut c, env.heap_base);
    c.push(0x6B);
    i32_const(&mut c, 3);
    c.push(0x76); // i32.shr_u → /8
    i32_const(&mut c, 1);
    c.push(0x6A);
    local_set(&mut c, 1); // scratch_words
    global_get(&mut c, env.heap_global);
    i32_const(&mut c, 7);
    c.push(0x6A);
    i32_const(&mut c, -8);
    c.push(0x71); // i32.and → 8-aligned
    local_set(&mut c, 0); // scratch_ptr
                          // tmp = scratch end; grow memory while it lies past the last page.
    local_get(&mut c, 0);
    local_get(&mut c, 1);
    i32_const(&mut c, 4);
    c.push(0x6C);
    c.push(0x6A);
    local_set(&mut c, 10);
    c.push(0x3F); // memory.size
    c.push(0x00);
    i32_const(&mut c, 16);
    c.push(0x74); // i32.shl → bytes
    local_get(&mut c, 10);
    c.push(0x49); // i32.lt_u → size < end
    c.push(0x04);
    c.push(0x40); // if
    {
        local_get(&mut c, 10);
        c.push(0x3F);
        c.push(0x00);
        i32_const(&mut c, 16);
        c.push(0x74);
        c.push(0x6B); // end - size
        i32_const(&mut c, 0xFFFF);
        c.push(0x6A);
        i32_const(&mut c, 16);
        c.push(0x76); // pages = ceil((end - size) / 64 KiB)
        c.push(0x40); // memory.grow
        c.push(0x00);
        i32_const(&mut c, -1);
        c.push(0x46); // i32.eq
        c.push(0x04);
        c.push(0x40); // if the grow failed: the allocator's own out-of-memory report
        {
            i32_const(&mut c, 4);
            c.push(0x42);
            c.push(0x00); // i64.const 0
            c.push(0x10);
            leb_u32(&mut c, env.fail_at);
            c.push(0x00); // unreachable
        }
        c.push(0x0B);
    }
    c.push(0x0B);
    // Zero the scratch before the walk, and again after it: the bump allocator hands out the memory
    // above its pointer on the promise that it is zero.
    let zero_scratch = |c: &mut Vec<u8>| {
        i32_const(c, 0);
        local_set(c, 9); // i (the zero-loop counter; its "slot" role is only inside the walk)
        c.push(0x02);
        c.push(0x40);
        c.push(0x03);
        c.push(0x40);
        local_get(c, 9);
        local_get(c, 1);
        c.push(0x4E);
        c.push(0x0D);
        leb_u32(c, 1);
        local_get(c, 0);
        local_get(c, 9);
        i32_const(c, 4);
        c.push(0x6C);
        c.push(0x6A);
        i32_const(c, 0);
        i32_store(c, 0);
        local_get(c, 9);
        i32_const(c, 1);
        c.push(0x6A);
        local_set(c, 9);
        c.push(0x0C);
        leb_u32(c, 0);
        c.push(0x0B);
        c.push(0x0B);
    };
    zero_scratch(&mut c);

    for (i, kind) in env.global_kinds.iter().enumerate() {
        if counted_kind(*kind) {
            global_get(&mut c, i as u32);
            local_get(&mut c, 0);
            c.push(0x10);
            leb_u32(&mut c, env.call(RuntimeFn::CensusVisit));
        }
    }

    if let Some(base) = env.callback_base {
        global_get(&mut c, base);
        local_set(&mut c, 7);
        c.extend([0x02, 0x40, 0x03, 0x40]);
        local_get(&mut c, 7);
        c.push(0x45);
        c.push(0x0D);
        leb_u32(&mut c, 1);
        local_get(&mut c, 7);
        i32_load(&mut c, 4);
        local_get(&mut c, 0);
        c.push(0x10);
        leb_u32(&mut c, env.call(RuntimeFn::CensusVisit));
        local_get(&mut c, 7);
        i32_load(&mut c, 8);
        local_set(&mut c, 7);
        c.push(0x0C);
        leb_u32(&mut c, 0);
        c.extend([0x0B, 0x0B]);
    }
    // Compare every visited object once. A zero scratch cell is not an object we reached.
    i32_const(&mut c, 0);
    local_set(&mut c, 9);
    c.push(0x02);
    c.push(0x40);
    c.push(0x03);
    c.push(0x40);
    local_get(&mut c, 9);
    local_get(&mut c, 1);
    c.push(0x4F);
    c.push(0x0D);
    leb_u32(&mut c, 1);
    local_get(&mut c, 0);
    local_get(&mut c, 9);
    i32_const(&mut c, 4);
    c.push(0x6C);
    c.push(0x6A);
    i32_load(&mut c, 0);
    local_tee(&mut c, 10);
    c.push(0x04);
    c.push(0x40);
    local_get(&mut c, 9);
    i32_const(&mut c, 8);
    c.push(0x6C);
    i32_const(&mut c, env.heap_base);
    c.push(0x6A);
    local_tee(&mut c, 3);
    i32_load(&mut c, 16);
    local_get(&mut c, 10);
    c.push(0x47);
    c.push(0x04);
    c.push(0x40);
    local_get(&mut c, 3);
    local_set(&mut c, 2);
    c.push(0x0C);
    leb_u32(&mut c, 3);
    c.push(0x0B);
    c.push(0x0B);
    local_get(&mut c, 9);
    i32_const(&mut c, 1);
    c.push(0x6A);
    local_set(&mut c, 9);
    c.push(0x0C);
    leb_u32(&mut c, 0);
    c.push(0x0B);
    c.push(0x0B);
    zero_scratch(&mut c);
    local_get(&mut c, 2);
    c.push(0x0B); // end function
    c
}

/// What a heap-resident block IS, for the census a later step reads out of the low byte of its
/// header's word 12 (packed `count:24 | class:8` — see the design record in `heap.md`'s S7 entry).
/// Only ever stamped on a 16-byte `[len/num][cap][data_ptr][count|class]` header, never on a raw data
/// buffer or anything smaller: those stay [`AllocClass::Raw`], which is the zeroed default every
/// fresh OR freshly-reused block already starts at (`__logos_free` zeroes word 12 on any block of at
/// least 16 bytes; a never-freed bump block is zero from instantiation) — so a `Raw` classification
/// costs no emitted store at all.
///
/// Records, heterogeneous tuples, and enums carry ownership bytes alongside their slots.
/// This separates scalar payload bits from child handles during retain, release, and census.
/// Raw value layouts outside this protocol remain part of the reclamation audit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AllocClass {
    /// A raw byte/word buffer or a value layout outside the counted-header protocol.
    /// The allocator writes no header for this class.
    Raw,
    /// A 16-byte collection header whose `data_ptr` buffer holds ONLY scalar 8-byte slots (`Seq of
    /// Int/Float/Bool`, plain `Set`) — nothing in it for a census to recurse into.
    ScalarHeader,
    /// A 16-byte collection header whose `data_ptr` buffer holds ONLY handle slots, one per 8 bytes
    /// (`Seq of Text`/of a record, `Set of Text`) — a census walks every slot up to `len`.
    HandleHeader,
    /// A 16-byte Map header: `data_ptr` holds 16-byte key/value pairs; both sides scalar.
    MapScalarScalar,
    /// A Map header whose key is scalar and whose value is a handle.
    MapScalarHandle,
    /// A Map header whose key is a handle and whose value is scalar.
    MapHandleScalar,
    /// A Map header whose key AND value are both handles.
    MapHandleHandle,
    /// A `Text` header: `data_ptr` holds `cap` raw bytes (or, with the growable bit in `cap`, a
    /// buffer an append may share with other views of it). Counted (S12): a Text is freed at a
    /// safepoint once nothing holds it, like a list.
    TextHeader,
    /// A record's fields occupy 8-byte slots followed by one ownership byte per field.
    /// The byte is 1 for a handle and 0 for a scalar; numeric bits are never read as pointers.
    RecordHeader,
    /// An enum keeps its tag at word 0 and its payload length at word 4; payloads have field-kind bytes.
    EnumHeader,
    /// A counter owns scalar replica records, each three eight-byte words.
    CounterHeader,
    /// A closure records its function index, capture count, and typed capture buffer.
    ClosureHeader,
    /// An integer buffer owns one tagged host value per eight-byte slot (a record cell has length one).
    ExactIntegerHeader,
}

impl AllocClass {
    const ALL: [AllocClass; 13] = [
        AllocClass::Raw,
        AllocClass::ScalarHeader,
        AllocClass::HandleHeader,
        AllocClass::MapScalarScalar,
        AllocClass::MapScalarHandle,
        AllocClass::MapHandleScalar,
        AllocClass::MapHandleHandle,
        AllocClass::TextHeader,
        AllocClass::RecordHeader,
        AllocClass::EnumHeader,
        AllocClass::CounterHeader,
        AllocClass::ClosureHeader,
        AllocClass::ExactIntegerHeader,
    ];

    /// The byte a census reads back. `Raw` is 0, the same value dead/zeroed memory already carries —
    /// see the type doc for why that means `Raw` needs no emitted store.
    fn code(self) -> u8 {
        match self {
            AllocClass::Raw => 0,
            AllocClass::ScalarHeader => 1,
            AllocClass::HandleHeader => 2,
            AllocClass::MapScalarScalar => 3,
            AllocClass::MapScalarHandle => 4,
            AllocClass::MapHandleScalar => 5,
            AllocClass::MapHandleHandle => 6,
            AllocClass::TextHeader => 7,
            AllocClass::RecordHeader => 8,
            AllocClass::EnumHeader => 9,
            AllocClass::CounterHeader => 10,
            AllocClass::ClosureHeader => 11,
            AllocClass::ExactIntegerHeader => 12,
        }
    }

    /// A collection's class from whether its elements are handles — `Set`/`Seq` share this rule
    /// (their `Kind` alone says which; see `kind::Kind::is_seq`'s siblings).
    fn of_elements(elements_are_handles: bool) -> AllocClass {
        if elements_are_handles {
            AllocClass::HandleHeader
        } else {
            AllocClass::ScalarHeader
        }
    }

    /// A Map's class from its key/value handle-ness (see `lower_map_insert`, which is the only site
    /// that reliably knows both — an empty map's eventual key/value kind is not yet chosen).
    fn of_map(key_is_handle: bool, value_is_handle: bool) -> AllocClass {
        match (key_is_handle, value_is_handle) {
            (false, false) => AllocClass::MapScalarScalar,
            (false, true) => AllocClass::MapScalarHandle,
            (true, false) => AllocClass::MapHandleScalar,
            (true, true) => AllocClass::MapHandleHandle,
        }
    }

    /// Which words of each slot of this block's data buffer hold a handle the block keeps:
    /// `(word 0, word 8)`. A sequence's slot is its 8-byte element; a map's is its 16-byte
    /// `[key][value]` entry.
    fn held_handles(self) -> (bool, bool) {
        match self {
            AllocClass::HandleHeader
            | AllocClass::MapHandleScalar
            | AllocClass::RecordHeader
            | AllocClass::EnumHeader
            | AllocClass::ClosureHeader => (true, false),
            AllocClass::MapScalarHandle => (false, true),
            AllocClass::MapHandleHandle => (true, true),
            AllocClass::Raw
            | AllocClass::ScalarHeader
            | AllocClass::MapScalarScalar
            | AllocClass::TextHeader
            | AllocClass::CounterHeader
            | AllocClass::ExactIntegerHeader => (false, false),
        }
    }

    /// Whether the header has a true holder count at word 16 and participates in the
    /// turn-retained worklist and free-at-zero cascade. Empty maps are counted before insertion.
    fn is_counted(self) -> bool {
        matches!(
            self,
            AllocClass::ScalarHeader
                | AllocClass::HandleHeader
                | AllocClass::TextHeader
                | AllocClass::MapScalarScalar
                | AllocClass::MapScalarHandle
                | AllocClass::MapHandleScalar
                | AllocClass::MapHandleHandle
                | AllocClass::RecordHeader
                | AllocClass::EnumHeader
                | AllocClass::CounterHeader
                | AllocClass::ClosureHeader
                | AllocClass::ExactIntegerHeader
        )
    }
}

/// Kinds whose standalone representation participates in true retain/release ownership.
/// Register roots, globals, captures, and container stores use this independently of COW:
/// immutable values still need to live until their last holder releases them.
/// Unresolved SeqAny and the remaining raw layouts require separate reclamation work.
fn counted_kind(k: Option<Kind>) -> bool {
    matches!(
        k,
        Some(Kind::SeqInt)
            | Some(Kind::SeqBool)
            | Some(Kind::SeqFloat)
            | Some(Kind::SeqWord32)
            | Some(Kind::SeqWord64)
            | Some(Kind::SeqText)
            | Some(Kind::SeqStruct)
            | Some(Kind::SeqEnum)
            | Some(Kind::SeqSeqInt)
            | Some(Kind::SeqHandle)
            | Some(Kind::Set)
            | Some(Kind::SetText)
            | Some(Kind::CrdtSetText)
            | Some(Kind::Text)
            | Some(Kind::Map)
            | Some(Kind::Struct)
            | Some(Kind::Tuple)
            | Some(Kind::Enum)
            | Some(Kind::Rational)
            | Some(Kind::CrdtCounter)
            | Some(Kind::Closure)
    )
}

/// Whether a value of this `Kind`, held in a SLOT (a map's value, a future struct/tuple field, a
/// `Seq`/`Set` element), is an `i32` HANDLE to a separate heap object rather than a plain scalar
/// embedded in the slot itself. Exhaustive on purpose — an unmatched kind is a compile error, not a
/// silently-wrong guess, the way `k.wasm_valtype() == I32` alone would be: [`Kind::Date`] and
/// [`Kind::Word32`] are BOTH `i32`-valtype SCALARS (a day count, a wrapping ring word), the only two
/// exceptions to "i32 valtype means handle" in the whole enum.
///
/// The nine kinds documented "LINKER MODE ONLY" (`BigInt`/`Complex`/`Modular`/`Decimal`/`Money`/
/// `Quantity`/`Uuid`/`LanesV`/`Dynamic`) never reach a SELF-CONTAINED module at all, so their answer
/// here is moot — [`emit_alloc`] never stamps a class in linker mode regardless. They are still
/// matched by name, not `_`, so a kind added to the enum later cannot silently fall through unnoticed.
fn kind_is_handle(k: Kind) -> bool {
    match k {
        Kind::Nothing
        | Kind::Int
        | Kind::Bool
        | Kind::Char
        | Kind::Float
        | Kind::Date
        | Kind::Moment
        | Kind::Duration
        | Kind::Time
        | Kind::Span
        | Kind::Word32
        | Kind::Word64 => false,
        Kind::SeqInt
        | Kind::SeqBool
        | Kind::SeqFloat
        | Kind::SeqText
        | Kind::SeqStruct
        | Kind::SeqEnum
        | Kind::SeqSeqInt
        | Kind::SeqHandle
        | Kind::SeqAny
        | Kind::SeqWord32
        | Kind::SeqWord64
        | Kind::Text
        | Kind::Struct
        | Kind::Map
        | Kind::Set
        | Kind::SetText
        | Kind::CrdtSetText
        | Kind::CrdtCounter
        | Kind::Enum
        | Kind::Closure
        | Kind::Tuple
        | Kind::Rational
        | Kind::Optional
        | Kind::Lanes
        | Kind::BigInt
        | Kind::Complex
        | Kind::Modular
        | Kind::Decimal
        | Kind::Money
        | Kind::Quantity
        | Kind::Uuid
        | Kind::LanesV
        | Kind::Dynamic
        | Kind::Tower => true,
    }
}

/// The class a freshly-created EMPTY collection header should carry, from its OWN `Kind` (the
/// register the header's handle is bound to — `Op::NewEmptyList`/`NewEmptySet`/a CRDT set/seq). A
/// `Kind::Map` or an unresolved [`Kind::SeqAny`] (refined by its first `Push`/insert) is empty and
/// inert either way, so `Raw` is exact, not merely safe, until that first write — see
/// `lower_map_insert` for a map's real, insert-time classification.
fn empty_collection_class(kind: Option<Kind>) -> AllocClass {
    match kind {
        // An empty map has no entries to scan; the first insert refines this class.
        // Its header must already carry a count before that insert.
        Some(Kind::Map) => AllocClass::MapScalarScalar,
        Some(Kind::SeqInt)
        | Some(Kind::SeqBool)
        | Some(Kind::SeqFloat)
        | Some(Kind::SeqWord32)
        | Some(Kind::SeqWord64)
        | Some(Kind::Set) => AllocClass::ScalarHeader,
        Some(Kind::SeqText)
        | Some(Kind::SeqStruct)
        | Some(Kind::SeqEnum)
        | Some(Kind::SeqSeqInt)
        | Some(Kind::SeqHandle)
        | Some(Kind::SetText)
        | Some(Kind::CrdtSetText) => AllocClass::HandleHeader,
        _ => AllocClass::Raw,
    }
}

/// Every allocation site names its `AllocClass` explicitly — `wasm_alloc_site_lock::
/// every_allocation_site_is_classified` census-checks the source for exactly that.
fn emit_alloc(code: &mut Vec<u8>, ctx: &Ctx, dst_scratch: u32, class: AllocClass) {
    // stack: [size]
    if let Some(rt_alloc) = ctx.rt_alloc {
        // LINKER MODE: each block comes straight from the runtime allocator (`dlmalloc`, which grows
        // linear memory on demand), so the emitter heap is UNBOUNDED — a program allocating past any fixed
        // slab can't run off the end or collide with the runtime's region. The census/class scheme is a
        // self-contained-heap concern only (Tristen's ruling); `dlmalloc`'s blocks carry no class byte.
        code.push(0x10); // call logos_rt_alloc(size)
        leb_u32(code, rt_alloc);
        local_set(code, dst_scratch); // dst_scratch = ptr
    } else if let Some(alloc) = ctx.runtime.and_then(|r| r.index(RuntimeFn::Alloc)) {
        // Self-contained: `__logos_alloc(size)` — the free list first, then the bump.
        //
        // S8: a COUNTED class (`AllocClass::is_counted`) is born a true holder count of 1, credited
        // to THE TURN (the caller of whatever lowered op reaches this point) — so its header is 4
        // bytes wider than its class alone would need, requested here (before the call) rather than
        // at every one of this class's ~dozen call sites, and the fresh block is immediately listed
        // on the turn's retained list so a safepoint before it is ever stored anywhere durable still
        // frees it. Gated on `ctx.heap_counting()` (an interactive, self-contained, heap-using
        // module) so a batch module — which never runs a safepoint at all — pays nothing extra.
        let counted = class.is_counted() && ctx.heap_counting();
        if counted {
            i32_const(code, 4);
            code.push(0x6A); // i32.add — the header grows by exactly the new count word
        }
        code.push(0x10); // call
        leb_u32(code, alloc);
        local_set(code, dst_scratch);
        if class != AllocClass::Raw {
            local_get(code, dst_scratch);
            i32_const(code, class.code() as i32);
            i32_store(code, 12);
        }
        if counted {
            local_get(code, dst_scratch);
            i32_const(code, 1);
            i32_store(code, 16);
            if let Some(push) = ctx.runtime.and_then(|r| r.index(RuntimeFn::TurnRetain)) {
                local_get(code, dst_scratch);
                code.push(0x10);
                leb_u32(code, push);
            }
        }
    } else {
        // No heap helpers (a module that allocates nothing still reaches here harmlessly).
        global_get(code, ctx.heap_global);
        i32_const(code, 7);
        code.push(0x6A);
        i32_const(code, -8);
        code.push(0x71);
        local_tee(code, dst_scratch);
        code.push(0x6A);
        global_set(code, ctx.heap_global);
    }
}

/// Give a block back. `ptr` and `size` are LOCALS; a no-op wherever the heap has no free list
/// (linker mode, where `dlmalloc` owns the memory and its `free` is not yet wired).
fn emit_free(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    push_ptr: impl Fn(&mut Vec<u8>),
    push_size: impl Fn(&mut Vec<u8>),
) {
    let Some(free) = ctx.runtime.and_then(|r| r.index(RuntimeFn::Free)) else {
        return;
    };
    push_ptr(code);
    push_size(code);
    code.push(0x10); // call
    leb_u32(code, free);
}

/// `Push value to seq` (Int sequence): append `value`, growing the data buffer when it is full and
/// updating the header in place (so the handle is stable).
///
/// ★ THE REFINEMENT THIS COMMENT USED TO PROMISE HAS LANDED. It said "O(n) per push — correctness
/// first; geometric capacity is a later refinement", and [`lower_list_push_at`] has tracked `cap`
/// and doubled for some time: a push with a slot free is O(1), and n pushes are O(n) in total. The
/// stale half sent a later reader (me) hunting an O(n²) that is not there. A comment that was true
/// once is not evidence.
fn lower_list_push(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    list: u16,
    value: u16,
) -> R<()> {
    let elem =
        kinds
            .get(list as usize)
            .and_then(Kind::seq_elem)
            .ok_or(WasmLowerError::Unsupported(
                "push to a sequence of unknown element kind",
            ))?;
    lower_list_push_at(code, elem, ctx, num_regs, list as u32, value)
}

/// The core of [`lower_list_push`], parameterized by the element kind and the LOCAL holding the seq
/// handle (rather than deriving them from a register) — so a struct FIELD seq (`ListPushField`, whose
/// handle lives in a struct slot, kind in the struct layout) can push through the same amortized path.
fn lower_list_push_at(
    code: &mut Vec<u8>,
    elem: Kind,
    ctx: &Ctx,
    num_regs: u32,
    lst: u32,
    value: u16,
) -> R<()> {
    // Int/Float/Text(handle) elements all occupy an 8-byte slot — only the load/store opcode differs.
    let elem_load = seq_elem_load(elem)?;
    let elem_store = seq_elem_store(elem)?;
    if let Some(push_slot) = ctx.runtime.and_then(|r| r.index(RuntimeFn::SeqPushSlot)) {
        if ctx.hot.get() {
            // In a loop ([`Ctx::hot`]): a push with room takes its slot inline; only a full buffer
            // calls the runtime to grow.
            let len = num_regs + 5;
            local_get(code, lst);
            i32_load(code, 0);
            local_tee(code, len);
            local_get(code, lst);
            i32_load(code, 4);
            code.push(0x48); // i32.lt_s → room
            code.push(0x04);
            code.push(I32);
            local_get(code, lst);
            local_get(code, len);
            i32_const(code, 1);
            code.push(0x6A);
            i32_store(code, 0); // len + 1
            local_get(code, lst);
            i32_load(code, 8);
            local_get(code, len);
            i32_const(code, 3);
            code.push(0x74); // i32.shl → len * 8
            code.push(0x6A);
            code.push(0x05);
            local_get(code, lst);
            code.push(0x10);
            leb_u32(code, push_slot);
            code.push(0x0B);
        } else {
            local_get(code, lst);
            code.push(0x10);
            leb_u32(code, push_slot);
        }
        local_get(code, value as u32);
        elem_store(code, 0);
        return Ok(());
    }
    // A linked module has no runtime of its own: the growth inline.
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
        emit_alloc(code, ctx, hs_new, AllocClass::Raw);
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
        // ★ AND THE BUFFER THIS GROWTH LEAVES BEHIND GOES BACK TO THE ALLOCATOR. Read from the
        // header BEFORE it is repointed, so both the pointer and the size are still the old ones.
        // It is dead by construction: a data buffer belongs to exactly one header, this header is
        // about to stop pointing at it, and a shallow copy of the sequence shares the HEADER (so it
        // follows the new pointer too). No refcount is needed to know that, which is why this is
        // the first free the allocator gets — everything else waits for the release protocol.
        //
        // A `cap` of 0 means there is no buffer yet and `data_ptr` is null; the helper drops a null.
        emit_free(
            code,
            ctx,
            |c| {
                local_get(c, lst);
                i32_load(c, 8); // the OLD data_ptr
            },
            |c| {
                local_get(c, lst);
                i32_load(c, 4); // the OLD cap
                i32_const(c, 8);
                c.push(0x6C); // * 8 bytes an element
            },
        );
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
/// shrinks length without freeing). A source program never pops an empty list here: `resolve_program`
/// stops it first with the sentence naming the list. Hand-built bytecode can, and the guarded `else`
/// keeps the load in bounds and yields a typed zero.
fn lower_list_pop(code: &mut Vec<u8>, kinds: &KindTable, dst: u16, list: u16) -> R<()> {
    let elem =
        kinds
            .get(list as usize)
            .and_then(Kind::seq_elem)
            .ok_or(WasmLowerError::Unsupported(
                "pop from a sequence of unknown element kind",
            ))?;
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
fn emit_seq_elem_addr(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    kinds: &KindTable,
    collection: u16,
    index: u16,
) -> R<()> {
    // Every element (scalar OR handle) occupies an 8-byte slot, so the address arithmetic is the
    // same — only the caller's load/store width differs. A heterogeneous `Tuple` has the identical
    // header+slot layout (the caller picks the width from the static position kind).
    match kinds.get(collection as usize) {
        Some(Kind::Tuple) => {}
        other if other.and_then(Kind::seq_elem).is_some() => {}
        _ => {
            return Err(WasmLowerError::Unsupported(
                "index of a non-scalar sequence",
            ))
        }
    }
    let col = collection as u32;
    let slot = if ctx.hot.get() {
        None
    } else {
        ctx.runtime.and_then(|r| r.index(RuntimeFn::SeqSlot))
    };
    if let Some(slot) = slot {
        local_get(code, col);
        local_get(code, index as u32);
        code.push(0x10);
        leb_u32(code, slot);
        return Ok(());
    }
    // In a loop ([`Ctx::hot`]), or in a linked module (no runtime of its own): the check and the
    // address inline.
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
                     // Say WHICH index, in the reference tier's own words. There is no message constant to hand
                     // over here — the sentence needs the value, which only exists now — so the module reports the
                     // CODE and the VALUE and the host renders it through `semantics::failcode::message`, the one
                     // definition the interpreter also formats with.
    if let Some(fail_at) = (ctx.host_index)(HostFn::LogosRtFailAt) {
        i32_const(
            code,
            crate::semantics::failcode::FailCode::IndexOutOfBounds.code(),
        );
        local_get(code, index as u32);
        code.push(0x10); // call logos_rt_fail_at(code, value)
        leb_u32(code, fail_at);
    }
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
    kinds
        .get(collection as usize)
        .and_then(Kind::seq_elem)
        .ok_or(WasmLowerError::Unsupported(
            "sequence of unknown element kind",
        ))
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
fn lower_index(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    kinds: &KindTable,
    dst: u16,
    collection: u16,
    index: u16,
) -> R<()> {
    let elem = if kinds.get(collection as usize) == Some(Kind::Tuple) {
        kinds
            .get(dst as usize)
            .ok_or(WasmLowerError::Unsupported("tuple element of unknown kind"))?
    } else {
        seq_elem_kind(kinds, collection)?
    };
    let load = seq_elem_load(elem)?;
    emit_seq_elem_addr(code, ctx, kinds, collection, index)?;
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
fn lower_text_index(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    collection: u16,
    index: u16,
) -> R<()> {
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
    code.push(0xAD); // i64.extend_i32_u
    code.push(0x55); // i64.gt_s → index > len
    code.push(0x72); // i32.or
    code.push(0x04);
    code.push(0x40); // if
    code.push(0x00); // unreachable
    code.push(0x0B); // end
                     // hdr = alloc(16); data = alloc(1)
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr, AllocClass::TextHeader);
    i32_const(code, 1);
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
fn emit_bytes_to_seq(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    base_reg: u32,
    len_reg: u32,
) {
    let (hdr, data, i) = (num_regs + 7, num_regs + 8, num_regs + 9);
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr, AllocClass::ScalarHeader); // always builds a `Seq of Int`
                                                          // data = alloc(len * 8) — one i64 element per byte.
    local_get(code, len_reg);
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
/// Resolve the wasm function index of a compiled LOGOS function by name, checking that it has the
/// signature the caller intends to use — because "declarer wins" lets a program define its own
/// function of that name, and the loader then never splices the stdlib's. A compiler-written call
/// that lands on somebody else's function of the same name is worse than no call at all.
/// The signature is compared as WASM TYPES — the parameter valtypes and the result valtype — not as
/// `Kind` labels. A `-> Bool` may plan as `Kind::Int`: both ride an `i64`, the two collapse in the
/// register unifier wherever nothing observes the label, and a caller emitting `call` needs the
/// types to line up, not the names.
fn compiled_fn_by_name(ctx: &Ctx, name: &str, params: &[u8], result_valtype: u8) -> Option<u32> {
    let idx = ctx
        .functions
        .iter()
        .position(|f| ctx.interner.resolve(f.name) == name)?;
    if ctx.fn_param_valtypes.get(idx).map(Vec::as_slice) != Some(params) {
        return None;
    }
    if ctx.fn_inout.get(idx).is_some_and(|ps| !ps.is_empty()) {
        return None;
    }
    if ctx
        .fn_results
        .get(idx)
        .copied()
        .flatten()
        .map(Kind::wasm_valtype)
        != Some(result_valtype)
    {
        return None;
    }
    Some(ctx.fn_base + idx as u32)
}

fn lower_text_from_bytes(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    seq: u16,
) -> R<()> {
    // ★ THE ERROR-PARITY OBLIGATION. The tree-walker and the VM raise `text_from_bytes(): invalid
    // UTF-8` and the AOT tier panics with the same sentence, so a standalone module must TRAP —
    // that is `wasm_aot_lock::wasm_traps_where_treewalker_errors`, and this lowering used to be its
    // one violation: it copied the low byte of each element into linear memory and validated
    // nothing, so a browser answered U+FFFD where every other tier stopped.
    //
    // The rules are subtle — the 0xC2 floor, the 0xF4 ceiling, overlong forms, the surrogate hole —
    // and they are ALREADY WRITTEN, in LOGOS: `strings.lg`'s `validUtf8 (data: Seq of Int) -> Bool`
    // takes exactly the value this builtin is handed. So the module CALLS them rather than carrying
    // a second copy of the table to rot beside the first. `validUtf8` is demanded by the loader the
    // way `matchRoute` is (see `loader.rs`) — the BUILTIN is the reference, and the author never
    // types the name.
    let check = compiled_fn_by_name(ctx, crate::vm::reach::UTF8_VALIDATOR, &[I32], I64)
        .ok_or(WasmLowerError::Unsupported(
            "text_from_bytes cannot validate: `validUtf8 (data: Seq of Int) -> Bool` was not loaded. \
             The loader demands it whenever `text_from_bytes` is reachable (see `loader.rs`); if a \
             program declares its own `validUtf8` with another shape, that one wins and this refuses \
             rather than calling it.",
        ))?;
    // The callee becomes a second holder of the sequence, exactly as `Op::Call` treats every
    // heap argument. `validUtf8` only reads it — but that is a premise held in another file, and
    // an unnecessary copy-on-write is a cost while a missed one is a wrong answer.
    emit_retain(code, ctx, seq);
    local_get(code, seq as u32);
    code.push(0x10); // call validUtf8 → i64 (Bool)
    leb_u32(code, check);
    code.push(0x50); // i64.eqz — "not valid"
    code.push(0x04);
    code.push(0x40); // if
    if let Some(fail_at) = (ctx.host_index)(HostFn::LogosRtFailAt) {
        i32_const(
            code,
            crate::semantics::failcode::FailCode::InvalidUtf8.code(),
        );
        code.push(0x42);
        leb_i64(code, 0); // no value: the sentence needs none
        code.push(0x10); // call logos_rt_fail_at
        leb_u32(code, fail_at);
    }
    code.push(0x00); // unreachable
    code.push(0x0B); // end
    let (hdr, data, i, len, src) = (
        num_regs + 7,
        num_regs + 8,
        num_regs + 9,
        num_regs + 6,
        num_regs + 5,
    );
    local_get(code, seq as u32);
    i32_load(code, 0); // element count
    local_set(code, len);
    local_get(code, seq as u32);
    i32_load(code, 8); // seq data_ptr
    local_set(code, src);
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr, AllocClass::TextHeader);
    local_get(code, len); // data = alloc(len) — one byte each
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
    if ctx.exact.sequence_ints() {
        i64_load(code, 0);
        exact::narrow_sequence_slot(code, ctx, num_regs, seq as u32)?;
        code.push(0xa7);
    } else {
        i32_load8_u(code, 0);
    }
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
    Ok(())
}

/// `uuid_from_bytes(seq)` — pack the low byte of the first 16 `Seq of Int` elements into a contiguous
/// 16-byte block, then hand it to `logos_rt_uuid_from_ptr` to box a `base::Uuid` (LINKER MODE only).
fn lower_uuid_from_bytes(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    seq: u16,
) -> R<Flow> {
    let from_ptr = (ctx.host_index)(HostFn::UuidFromPtr)
        .ok_or(WasmLowerError::Unsupported("uuid_from_ptr not imported"))?;
    let (block, i, src) = (num_regs + 7, num_regs + 9, num_regs + 5);
    local_get(code, seq as u32);
    i32_load(code, 8); // seq data_ptr
    local_set(code, src);
    i32_const(code, 16);
    emit_alloc(code, ctx, block, AllocClass::Raw);
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
    emit_alloc(code, ctx, block, AllocClass::Raw);
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
    emit_alloc(code, ctx, block, AllocClass::Raw);
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
    emit_alloc(code, ctx, hdr, AllocClass::ScalarHeader); // `Seq of Word32` — scalar elements
    i32_const(code, 32); // 4 elements × 8-byte slots
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
fn lower_sha1_op(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    dst: u16,
    args_start: u16,
    host: HostFn,
    ternary: bool,
) -> R<Flow> {
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
fn lower_set_index(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    kinds: &KindTable,
    num_regs: u32,
    collection: u16,
    index: u16,
    value: u16,
) -> R<()> {
    let elem = seq_elem_kind(kinds, collection)?;
    let store = seq_elem_store(elem)?;
    let elem_is_counted = counted_kind(Some(elem));
    let addr_local = num_regs + 5;
    emit_seq_elem_addr(code, ctx, kinds, collection, index)?; // [addr]
    if ctx.heap_counting() && elem_is_counted {
        local_tee(code, addr_local); // keep the address for the store below too
                                     // S8's true-count release: the element about to be OVERWRITTEN stops being held by this
                                     // slot right here — read as a plain i32 handle (this arm only runs for a `Seq`, never a
                                     // `Map`, and every counted element kind is `i32`-valtype by `kind_is_handle`'s own contract).
        i32_load(code, 0);
        emit_true_release(code, ctx, |_| {});
        local_get(code, addr_local);
    }
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
    emit_alloc(code, ctx, hdr, AllocClass::ScalarHeader); // a numeric range is always `Seq of Int`
    local_get(code, cnt);
    code.push(0xA7); // i32.wrap_i64
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
fn lower_repeat_seq(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    args_start: u16,
) -> R<()> {
    let value = args_start; // the element x
    let n = args_start + 1; // the count (i64)
    let is_float = matches!(kinds.get(value as usize), Some(Kind::Float));
    if !matches!(
        kinds.get(value as usize),
        Some(Kind::Int) | Some(Kind::Float)
    ) {
        return Err(WasmLowerError::Unsupported(
            "repeatSeq of a non-scalar element (deep-copy)",
        ));
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
    emit_alloc(code, ctx, hdr, AllocClass::ScalarHeader); // repeatSeq refuses a non-scalar element above
    local_get(code, cnt);
    code.push(0xA7); // i32.wrap_i64
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
fn lower_new_list(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    start: u16,
    count: u16,
) -> R<()> {
    let want = if count > 0 {
        kinds
            .get(start as usize)
            .ok_or(WasmLowerError::Unsupported(
                "list literal of unknown element kind",
            ))?
    } else {
        Kind::Int
    };
    let elem_store = seq_elem_store(want)?;
    for j in 0..count {
        if kinds.get((start + j) as usize) != Some(want) {
            return Err(WasmLowerError::Unsupported(
                "list literal with mixed element kinds",
            ));
        }
    }
    let (hdr, data) = (num_regs + 5, num_regs + 6);
    i32_const(code, 16);
    emit_alloc(
        code,
        ctx,
        hdr,
        AllocClass::of_elements(kind_is_handle(want)),
    );
    i32_const(code, i32::from(count) * 8);
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
    // S8's true count, when the ELEMENT kind is itself counted (a matrix literal `[[1,2],[3,4]]`'s
    // rows, a list of Texts): this fresh header is a brand-new holder of each element the moment the
    // literal is built. One walk of the header it just stamped, so a literal of a thousand elements
    // costs the code of one.
    if count > 0 && counted_kind(Some(want)) {
        emit_retain_children(code, ctx, hdr);
    }
    local_get(code, hdr);
    local_set(code, dst as u32);
    Ok(())
}

/// `Let (a, b, …) be t` (`DestructureTuple`) — bind each destructured register `start + i` to tuple
/// slot `i`, loaded at that target's own width (`emit_slot_load` by the register's kind). Both a
/// heterogeneous tuple and a homogeneous one (which rides a `SeqX`) lay their elements out at 8-byte
/// slots, so `data_ptr + i*8` is the slot address in both.
fn lower_destructure_tuple(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    src: u16,
    start: u16,
    count: u16,
) -> R<()> {
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
fn lower_new_tuple_het(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    start: u16,
    count: u16,
) -> R<()> {
    let (hdr, data) = (num_regs + 5, num_regs + 6);
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr, AllocClass::RecordHeader);
    i32_const(
        code,
        i32::from(count) * if ctx.heap_counting() { 9 } else { 8 },
    );
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
        if ctx.heap_counting() {
            local_get(code, data);
            i32_const(
                code,
                i32::from(kinds.get((start + j) as usize).is_some_and(kind_is_handle)),
            );
            i32_store8(code, u32::from(count) * 8 + u32::from(j));
        }
    }
    local_get(code, hdr);
    local_set(code, dst as u32);
    Ok(())
}

/// Turn the enum handle on the stack into the address its payload slots are read from: a standalone
/// enum's data buffer (its header holds the pointer at 8), or the linked object itself, whose payload
/// follows the inline tag. [`enum_payload_offset`] gives each payload's byte offset from it.
fn emit_enum_payload_base(code: &mut Vec<u8>, ctx: &Ctx) {
    if !ctx.linked {
        i32_load(code, 8);
    }
}

/// The byte offset of payload `index` from the address [`emit_enum_payload_base`] leaves.
fn enum_payload_offset(ctx: &Ctx, index: u32) -> u32 {
    8 * (index + u32::from(ctx.linked))
}

/// Construct an enum. A standalone EnumHeader keeps tag@0, payload length@4, data@8,
/// class@12, and (when counting) holders@16. The payload buffer has eight-byte values
/// followed by one ownership byte per value. Linked modules retain their inline-payload ABI.
fn lower_new_inductive(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    ctor: u32,
    args_start: u16,
    count: u16,
) -> R<()> {
    let (hdr, data) = (num_regs + 5, num_regs + 6);
    if ctx.linked {
        // The linked runtime's enum ABI remains tag + inline payload.
        i32_const(code, 8 * (1 + i32::from(count)));
        emit_alloc(code, ctx, hdr, AllocClass::Raw);
        local_get(code, hdr);
        i32_const(code, ctor as i32);
        i32_store(code, 0);
        for k in 0..count {
            let arg = args_start + k;
            local_get(code, hdr);
            local_get(code, arg as u32);
            emit_slot_store(code, kinds.get(arg as usize), 8 * (1 + u32::from(k)))?;
        }
    } else {
        // A standalone enum has a counted header. The payload's kinds are kept separately
        // from its bits, so an integer payload can never impersonate a reference count.
        i32_const(code, 16);
        emit_alloc(code, ctx, hdr, AllocClass::EnumHeader);
        if count == 0 {
            i32_const(code, 0);
            local_set(code, data);
        } else {
            i32_const(
                code,
                i32::from(count) * if ctx.heap_counting() { 9 } else { 8 },
            );
            emit_alloc(code, ctx, data, AllocClass::Raw);
        }
        local_get(code, hdr);
        i32_const(code, ctor as i32);
        i32_store(code, 0);
        local_get(code, hdr);
        i32_const(code, i32::from(count));
        i32_store(code, 4);
        local_get(code, hdr);
        local_get(code, data);
        i32_store(code, 8);
        for k in 0..count {
            let arg = args_start + k;
            let kind = kinds.get(arg as usize);
            local_get(code, data);
            local_get(code, arg as u32);
            emit_slot_store(code, kind, u32::from(k) * 8)?;
            if ctx.heap_counting() {
                local_get(code, data);
                i32_const(code, i32::from(kind.is_some_and(kind_is_handle)));
                i32_store8(code, u32::from(count) * 8 + u32::from(k));
            }
        }
    }
    local_get(code, hdr);
    local_set(code, dst as u32);
    Ok(())
}

/// Read a matched enum payload at its inferred value width. Standalone payloads live in
/// the header's data buffer; linked payloads follow the inline tag. Transparent Options
/// retain their existing value/sentinel representation.
fn lower_bind_arm(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    dst: u16,
    target: u16,
    index: u16,
) -> R<()> {
    // ★ AN `Option` IS TRANSPARENT, so its payload is the value itself.
    //
    // `Expr::OptionSome { value }` compiles to `compile_expr_into(value, dst)` — `some 3` IS `3`,
    // with `nothing` as a sentinel — so an `Option` register never carries `Kind::Enum` and this
    // check refused every `Inspect` over one. That took out `{inspect}` in a `View:` and the
    // ordinary statement alike; `lgx_wasm_lock` proved the fault was the BACKEND's by showing
    // hand-written LOGOS failing identically, which is exactly the evidence that said where to fix
    // it. (`Inspect` over a `Result` always worked: a `Result` IS a real enum with real slots.)
    //
    // For the transparent encoding the bind is a copy of the same value at the same valtype — the
    // arm only runs on the matching variant, and `Some`'s single payload is the register itself.
    // Restricted to index 0 so a genuine multi-payload bind on a non-enum target is still refused
    // rather than silently reading the target as its own first field.
    //
    // ★ COMPARED BY VALTYPE, NOT BY KIND, and the first attempt got this wrong. The two registers
    // hold the SAME value, so what the copy needs is that they are the same wasm type — the `Kind`
    // LABELS can legitimately differ, because the unifier types the binding from its uses while the
    // scrutinee is typed from its construction. Demanding equal kinds refused the very case this
    // arm exists for and reported it as a backend gap.
    let target_kind = kinds.get(target as usize);
    if target_kind != Some(Kind::Enum) {
        // Gated to `I32` to agree with `lower_test_arm`: an `Option` whose arms cannot be told
        // apart must not have a binding lowered for it either, or the refusal moves from the test
        // to whatever the bound name is used for and stops naming the real reason.
        if index == 0
            && kinds.valtype(target as usize) == I32
            && kinds.valtype(target as usize) == kinds.valtype(dst as usize)
        {
            local_get(code, target as u32);
            local_set(code, dst as u32);
            return Ok(());
        }
        return Err(WasmLowerError::Unsupported(
            "payload binding on a non-enum target",
        ));
    }
    // A `When V (binds)` arm for a variant the target is NOT runs dead — the preceding `TestArm` +
    // `JumpIfFalse` skip it — and may bind an index past the actual variant's arity, so the bound
    // payload has no kind. The load never executes; default it to the `i64` the `None`-kind local is
    // declared as (`valtype`) so the slot load is merely valtype-consistent, not a refusal. (A live
    // bind always resolves a concrete kind from the construction site, so this only hits dead arms —
    // register splitting can isolate such a dead bind whose kind register-reuse formerly supplied.)
    let kind = kinds.get(dst as usize).or(Some(Kind::Int));
    local_get(code, target as u32);
    emit_enum_payload_base(code, ctx);
    emit_slot_load(code, kind, enum_payload_offset(ctx, u32::from(index)))?;
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

fn lower_make_closure(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    func: u16,
    locals_start: u16,
) -> R<()> {
    let caps = ctx
        .functions
        .get(func as usize)
        .map(|f| f.captures.as_slice())
        .ok_or(WasmLowerError::Unsupported("closure of unknown function"))?;
    let cap_n = caps.len() as u32;
    let hs = num_regs + 5;
    let data = num_regs + 6;
    i32_const(
        code,
        if ctx.linked {
            8 * (1 + 2 * cap_n) as i32
        } else {
            16
        },
    );
    emit_alloc(
        code,
        ctx,
        hs,
        if ctx.linked {
            AllocClass::Raw
        } else {
            AllocClass::ClosureHeader
        },
    );
    local_get(code, hs);
    i32_const(code, i32::from(func));
    i32_store(code, 0);
    if ctx.linked {
        local_get(code, hs);
        i32_const(code, 8);
        code.push(0x6A);
        local_set(code, data);
    } else {
        local_get(code, hs);
        i32_const(code, cap_n as i32);
        i32_store(code, 4);
        if cap_n != 0 {
            i32_const(code, (17 * cap_n) as i32);
            emit_alloc(code, ctx, data, AllocClass::Raw);
        } else {
            i32_const(code, 0);
            local_set(code, data);
        }
        local_get(code, hs);
        local_get(code, data);
        i32_store(code, 8);
    }
    let mut local_k = 0_u16;
    for (k, (_sym, global)) in caps.iter().enumerate() {
        local_get(code, data);
        match global {
            Some(gidx) => global_get(code, u32::from(*gidx)),
            None => {
                local_get(code, u32::from(locals_start + local_k));
                local_k += 1;
            }
        }
        let off = 8 * k as u32;
        match capture_valtype(ctx, func, k) {
            I32 => i32_store(code, off),
            F64 => f64_store(code, off),
            _ => i64_store(code, off),
        }
        local_get(code, data);
        i64c(code, 1);
        i64_store(code, 8 * cap_n + off);
        let kind = ctx
            .capture_kinds
            .get(func as usize)
            .and_then(|v| v.get(k))
            .copied()
            .flatten();
        if !ctx.linked {
            local_get(code, data);
            i32_const(code, i32::from(counted_kind(kind)));
            i32_store8(code, 16 * cap_n + k as u32);
        }
        if counted_kind(kind) {
            if let Some(f) = ctx.runtime.and_then(|r| r.index(RuntimeFn::Retain)) {
                match global {
                    Some(gidx) => global_get(code, u32::from(*gidx)),
                    None => local_get(code, u32::from(locals_start + local_k - 1)),
                }
                code.push(0x10);
                leb_u32(code, f);
            }
        }
    }
    local_get(code, hs);
    local_set(code, u32::from(dst));
    Ok(())
}

/// `f(args)` (`CallValue`) — push the arguments, then (for a capturing closure) each capture's
/// stored value and present-flag, then the closure's function index (the table slot), and
/// `call_indirect` through the module's function table. The push order matches the callee body's
/// parameter layout `[args][values][flags]`. The static signature is the callee body's own type
/// (resolved via the closure's statically-traced construction site); a value result binds to `dst`.
fn lower_call_value(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    pc: usize,
    dst: u16,
    callee: u16,
    args_start: u16,
    arg_count: u16,
) -> R<()> {
    let func =
        plan.structs
            .callee_func
            .get(pc)
            .copied()
            .flatten()
            .ok_or(WasmLowerError::Unsupported(
                "indirect call to a closure of unknown origin",
            ))?;
    let type_idx = *ctx
        .fn_type
        .get(func as usize)
        .ok_or(WasmLowerError::Unsupported(
            "closure call: unknown callee type",
        ))?;
    let cap_n = ctx
        .functions
        .get(func as usize)
        .map(|f| f.captures.len() as u32)
        .unwrap_or(0);
    for k in 0..arg_count {
        exact::push_closure_argument(code, plan, ctx, func, k, args_start + k)?;
    }
    // capture values (each at its own kind), then present flags, from the closure object
    for k in 0..cap_n {
        local_get(code, callee as u32);
        if !ctx.linked {
            i32_load(code, 8);
        }
        let off = u32::from(ctx.linked) * 8 + 8 * k;
        match capture_valtype(ctx, func, k as usize) {
            I32 => i32_load(code, off),
            F64 => f64_load(code, off),
            _ => i64_load(code, off),
        }
    }
    for k in 0..cap_n {
        local_get(code, callee as u32);
        if !ctx.linked {
            i32_load(code, 8);
        }
        i64_load(code, u32::from(ctx.linked) * 8 + 8 * cap_n + 8 * k);
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
    if ctx
        .fn_results
        .get(func as usize)
        .copied()
        .flatten()
        .is_some()
    {
        exact::take_closure_result(code, plan, ctx, func, dst)?;
    }
    Ok(())
}

/// The exported `__logos_call_closure(identity: i32, arg: i64) -> i64` callback bridge.
/// Counted modules resolve an opaque host-registration identity to its live closure before dispatch.
/// The closure records its function index and points to capture values followed by present flags.
/// Unknown or released identities trap, including after the allocator reuses their old storage.
/// Uncounted modules retain direct handles. Linked modules call statically resolved closure bodies.
fn emit_closure_trampoline(
    fns: &[Plan],
    program: &CompiledProgram,
    fn_types: &[u32],
    ctx: &Ctx,
    callback_base: Option<u32>,
) -> R<Vec<u8>> {
    let n = fns.len();
    let mut body = Vec::new();
    // Exact argument/result helpers own i64 locals 2 and 3; registration lookup uses its own i32.
    let registration = if fns.iter().any(exact::on) {
        body.extend_from_slice(&[0x02, 0x02, I64, 0x01, I32]);
        4
    } else {
        body.extend_from_slice(&[0x01, 0x01, I32]);
        2
    };
    if let Some(base) = callback_base {
        global_get(&mut body, base);
        local_set(&mut body, registration);
        body.extend([0x02, 0x40, 0x03, 0x40]);
        local_get(&mut body, registration);
        body.push(0x45);
        body.extend([0x04, 0x40, 0x00, 0x0B]); // unknown or released ticket traps
        local_get(&mut body, registration);
        i32_load(&mut body, 0);
        local_get(&mut body, 0);
        body.push(0x46);
        body.push(0x0D);
        leb_u32(&mut body, 1);
        local_get(&mut body, registration);
        i32_load(&mut body, 8);
        local_set(&mut body, registration);
        body.push(0x0C);
        leb_u32(&mut body, 0);
        body.extend([0x0B, 0x0B]);
        local_get(&mut body, registration);
        i32_load(&mut body, 4);
        local_set(&mut body, 0);
    }
    for _ in 0..=n {
        body.extend([0x02, 0x40]);
    }
    local_get(&mut body, 0);
    i32_load(&mut body, 0);
    body.push(0x0E);
    leb_u32(&mut body, n as u32);
    for i in 0..n as u32 {
        leb_u32(&mut body, i);
    }
    leb_u32(&mut body, n as u32);
    for fi in 0..n {
        body.push(0x0B); // end block → arm fi lands here
        emit_trampoline_arm(&mut body, fi, fns, program, fn_types, ctx)?;
    }
    body.push(0x0B); // end default block
    body.push(0x00); // unreachable: unknown func_idx
    body.push(0x0B); // end function
    Ok(body)
}

/// One `br_table` arm: invoke closure `fi` with the event arg (local 1) and `fi`'s
/// captures loaded from the object at `handle` (local 0), then widen the result.
fn emit_trampoline_arm(
    body: &mut Vec<u8>,
    fi: usize,
    fns: &[Plan],
    program: &CompiledProgram,
    fn_types: &[u32],
    ctx: &Ctx,
) -> R<()> {
    let f = &program.functions[fi];
    let plan = &fns[fi];
    let param_count = f.param_count as usize;
    let cap_n = f.captures.len();
    // Only single-`Int` (`i64`) argument closures are callable through the fixed
    // trampoline signature; any other shape traps rather than mismatch its type.
    if param_count != 1 || plan.kinds.valtype(0) != I64 {
        body.push(0x00); // unreachable
        return Ok(());
    }
    // Push the event argument (the closure's sole user parameter), in its parameter's form.
    local_get(body, 1);
    exact::trampoline_argument(body, ctx, fi, 2)?;
    // Push each capture VALUE, then each present-FLAG — exactly as `lower_call_value`
    // frames a call to `fi`, but read from the object at `handle`.
    for k in 0..cap_n {
        local_get(body, 0);
        i32_load(body, 8);
        let off = 8 * k as u32;
        match plan.kinds.valtype(param_count + k) {
            v if v == I32 => i32_load(body, off),
            v if v == F64 => f64_load(body, off),
            _ => i64_load(body, off),
        }
    }
    for k in 0..cap_n {
        local_get(body, 0);
        i32_load(body, 8);
        i64_load(body, 8 * cap_n as u32 + 8 * k as u32);
    }
    // call_indirect `fi` through the table, using its real signature.
    local_get(body, 0);
    i32_load(body, 0); // func_idx = table slot
    body.push(0x11); // call_indirect
    leb_u32(body, fn_types[fi]);
    leb_u32(body, 0); // table 0
                      // Widen the closure's result to the trampoline's i64 (void → 0).
    match result_valtypes(plan.result).first().copied() {
        None => {
            body.push(0x42); // i64.const 0
            leb_i64(body, 0);
        }
        Some(v) if v == I64 => exact::trampoline_result(body, ctx, fi, 2, 3)?,
        Some(v) if v == I32 => body.push(0xAD), // i64.extend_i32_u
        Some(v) if v == F64 => body.push(0xBD), // i64.reinterpret_f64
        Some(_) => {}
    }
    body.push(0x0F); // return
    Ok(())
}

/// Lower a DOM builtin to its `dom_*` host call: push `arg_count` argument
/// registers, `call` the host, then bind `dst` — the returned value for a
/// creator/getter, or Nothing (`0`) for an effect op whose host returns void.

fn lower_dom_call(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    dst: u16,
    host: HostFn,
    args_start: u16,
    arg_count: u16,
    returns_value: bool,
) -> R<Flow> {
    let idx = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported("dom host not imported"))?;
    for i in 0..arg_count {
        local_get(code, args_start as u32 + i as u32);
        if host
            .retained_params()
            .into_iter()
            .any(|kept| kept == usize::from(i))
        {
            if let Some(retain) = ctx.runtime.and_then(|r| r.index(RuntimeFn::ClosureRetain)) {
                code.push(0x10);
                leb_u32(code, retain);
            }
        }
    }
    // Registration exists before the host can synchronously invoke the callback.
    code.push(0x10);
    leb_u32(code, idx);
    if !returns_value {
        i32_const(code, 0);
    }
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// `dispatchEntry(dispatcher, context, hook, node, event, value)` — the page's dispatcher entered
/// through its host (`dom_page_entry(node, event, value)`), which runs it as an entry of its
/// own and hears a failure through the page's failure door by the module's exports; the three
/// function indices are the bytecode tier's own. An entry that traps leaves the loop frames it had
/// pushed on the iterator stack, so the stack's top is put back to where it stood before it.
fn lower_dispatch_entry(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    args_start: u16,
) -> R<Flow> {
    let idx = (ctx.host_index)(HostFn::DomPageEntry)
        .ok_or(WasmLowerError::Unsupported("dom host not imported"))?;
    let saved = num_regs + 5;
    if ctx.uses_iter {
        global_get(code, ctx.iter_global);
        local_set(code, saved);
    }
    for k in 3..6 {
        local_get(code, u32::from(args_start) + k);
    }
    code.push(0x10); // call
    leb_u32(code, idx);
    local_set(code, u32::from(dst));
    if ctx.uses_iter {
        local_get(code, saved);
        global_set(code, ctx.iter_global);
    }
    Ok(Flow::Straight)
}

/// `storageGet(area, key)` — the host's answer, boxed as the `Optional` this backend represents
/// an `Option` with: `0` means `Nothing`, and anything else is a pointer to an 8-byte box holding
/// the present value (the same shape a non-blocking `Try to receive` produces).
///
/// ★ THE ABSENCE IS THE WHOLE POINT. A host that answered `""` for a missing key would make a
/// page unable to tell "nothing is saved" from "an empty string is saved" — and an empty draft is
/// exactly what a cleared form field stores. So the host answers handle `0` and this turns it
/// into a real `Nothing` that `Inspect` and `is none` can see.
fn lower_storage_get(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    args_start: u16,
) -> R<Flow> {
    let idx = (ctx.host_index)(HostFn::StorageGet)
        .ok_or(WasmLowerError::Unsupported("storage_get host not imported"))?;
    let boxp = num_regs + 6; // i32 Optional box pointer
    let val = num_regs + 7; // i32 Text handle the host answered
    local_get(code, args_start as u32);
    local_get(code, args_start as u32 + 1);
    code.push(0x10); // call storage_get
    leb_u32(code, idx);
    local_set(code, val);
    local_get(code, val);
    code.push(0x45); // i32.eqz — was the key absent?
    code.push(0x04);
    code.push(0x40); // if (void)
    i32_const(code, 0);
    local_set(code, dst as u32); // Nothing
    code.push(0x05); // else
    i32_const(code, 8);
    emit_alloc(code, ctx, boxp, AllocClass::Raw);
    local_get(code, boxp);
    local_get(code, val);
    emit_slot_store(code, Some(Kind::Text), 0)?;
    local_get(code, boxp);
    local_set(code, dst as u32);
    code.push(0x0B); // end if
    Ok(Flow::Straight)
}

/// Whether the environment sets the variable the Text in `args_start` names: `env_len` answers `-1`
/// for one it does not, so the Bool is `env_len(name) >= 0`.
fn lower_env_has(code: &mut Vec<u8>, ctx: &Ctx, dst: u16, args_start: u16) -> R<Flow> {
    let idx = (ctx.host_index)(HostFn::EnvLen)
        .ok_or(WasmLowerError::Unsupported("env_len host not imported"))?;
    local_get(code, args_start as u32);
    code.push(0x10); // call env_len
    leb_u32(code, idx);
    i32_const(code, 0);
    code.push(0x4E); // i32.ge_s
    code.push(0xAD); // i64.extend_i32_u → Bool
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// The text of the variable the Text in `args_start` names, `""` when it is unset: the module sizes a
/// buffer by `env_len`, the host writes the bytes with `env_into`, and the module builds the Text
/// header over them — the shape `fmt_align_into` uses, so a linked module needs no allocator export.
fn lower_env_value(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    args_start: u16,
) -> R<Flow> {
    let len_idx = (ctx.host_index)(HostFn::EnvLen)
        .ok_or(WasmLowerError::Unsupported("env_len host not imported"))?;
    let into_idx = (ctx.host_index)(HostFn::EnvInto)
        .ok_or(WasmLowerError::Unsupported("env_into host not imported"))?;
    let (hdr, data, len) = (num_regs + 5, num_regs + 6, num_regs + 7);
    // len = max(env_len(name), 0)
    local_get(code, args_start as u32);
    code.push(0x10); // call env_len
    leb_u32(code, len_idx);
    local_set(code, len);
    local_get(code, len);
    i32_const(code, 0);
    local_get(code, len);
    i32_const(code, 0);
    code.push(0x4A); // i32.gt_s
    code.push(0x1B); // select
    local_set(code, len);
    // data = alloc(len); the host writes the bytes and answers how many
    local_get(code, len);
    emit_alloc(code, ctx, data, AllocClass::Raw);
    local_get(code, data);
    local_get(code, args_start as u32);
    code.push(0x10); // call env_into
    leb_u32(code, into_idx);
    code.push(0x1A); // drop — the length the module sized the buffer by
    // hdr = alloc(16); header = [len, cap = len, data_ptr = data]
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr, AllocClass::TextHeader);
    local_get(code, hdr);
    local_get(code, len);
    i32_store(code, 0);
    local_get(code, hdr);
    local_get(code, len);
    i32_store(code, 4);
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8);
    local_get(code, hdr);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// `storageGetOr(area, key, fallback)` — the stored Text handle, or the fallback's handle when
/// the host answers `0`. NO NEW IMPORT: it is `storage_get` and a test of the handle it answered.
///
/// This is how a lowering that must USE a stored value reads one. An `Option of Text` is a boxed
/// handle on this tier — its presence can be tested but its value cannot be taken out — so a
/// store's read-back goes through here and no Option crosses the boundary at all.
fn lower_storage_get_or(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    args_start: u16,
) -> R<Flow> {
    let idx = (ctx.host_index)(HostFn::StorageGet)
        .ok_or(WasmLowerError::Unsupported("storage_get host not imported"))?;
    let val = num_regs + 7; // i32 Text handle the host answered
    local_get(code, args_start as u32);
    local_get(code, args_start as u32 + 1);
    code.push(0x10); // call storage_get
    leb_u32(code, idx);
    local_set(code, val);
    local_get(code, val);
    code.push(0x45); // i32.eqz — was the key absent?
    code.push(0x04);
    code.push(0x7F); // if (result i32)
    local_get(code, args_start as u32 + 2); // the fallback's handle
    code.push(0x05); // else
    local_get(code, val);
    code.push(0x0B); // end
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// `it is Variant` (`TestArm`) — compare the target's tag (constructor constant index) against
/// `variant` (the same constant index for that name, by dedup) → Bool. An `i32.eq` on tags is the
/// VM's constructor-name string compare.
fn lower_test_arm(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    dst: u16,
    target: u16,
    variant: u32,
) -> R<()> {
    // ★ A TRANSPARENT `Option` CARRIES NO TAG TO READ, so the enum path below is not merely
    // unhelpful for one — it is wrong in a way that VALIDATES. `some x` IS `x` (`vm/compiler.rs`:
    // `Expr::OptionSome { value } => compile_expr_into(value, dst)`), so an `Option of Text` reaches
    // here as a bare Text handle. `i32_load(target, 0)` then reads that Text object's own first
    // word as if it were a variant tag and compares it to a constant index. The types line up, the
    // module passes validation, and the arm chosen is arbitrary. That is the worst shape a backend
    // gap takes: a wrong answer with nothing to catch it.
    //
    // The reference tier's rule is presence, not tag (`machine.rs`'s `Op::TestArm`):
    // `RuntimeValue::Nothing` matches `None`, and anything else matches `Some`.
    let target_kind = kinds.get(target as usize);
    if target_kind != Some(Kind::Enum) {
        let name = match ctx.constants.get(variant as usize) {
            Some(Constant::Text(t)) => t.as_str(),
            _ => {
                return Err(WasmLowerError::Unsupported(
                    "an arm's variant name is not a Text constant",
                ))
            }
        };
        // ★ AND THE SENTINEL IS WHERE THIS STOPS. `nothing` lowers to the scalar `0`
        // (`Constant::Nothing` → `i64.const 0`), so for an `Option of Int` the value `some 0` and
        // `nothing` are the SAME BITS and no test can separate them. A heap handle cannot collide:
        // the bump allocator never hands out 0, so `!= 0` is exactly "present".
        //
        // So the handle kinds lower — which covers the case the VM's own comment calls
        // load-bearing, `Option of <record>` and `Option of Text`, the shape a middleware
        // `-> Option of Response` has — and a scalar payload is refused BY NAME rather than
        // answered with a coin flip. Closing that needs a boxed `Option` in the BYTECODE, which is
        // a representation change the tree-walker and VM share; it is not a wasm-local decision.
        if kinds.valtype(target as usize) != I32 {
            return Err(WasmLowerError::Unsupported(
                "`Inspect` over an `Option` of a scalar (Int/Bool/Float): `nothing` and `some 0` are \
                 the same bits in a compiled module, so the arms cannot be told apart. Inspect an \
                 `Option` of a record, of Text, or of a sequence — or use a `Result`, which carries a \
                 real tag.",
            ));
        }
        match name {
            "Some" | "None" => {
                local_get(code, target as u32);
                i32_const(code, 0);
                code.push(if name == "None" { 0x46 } else { 0x47 }); // i32.eq : i32.ne
                code.push(0xAD); // i64.extend_i32_u → Bool
                local_set(code, dst as u32);
                return Ok(());
            }
            _ => return Err(WasmLowerError::Unsupported(
                "a variant arm on a value that carries no variant tag — only `Some` and `None` \
                     match a transparent `Option`",
            )),
        }
    }
    local_get(code, target as u32);
    i32_load(code, 0); // tag
    i32_const(code, variant as i32);
    code.push(0x46); // i32.eq
    code.push(0xAD); // i64.extend_i32_u → Bool
    local_set(code, dst as u32);
    Ok(())
}

/// Bump-allocate a zeroed 16-byte collection header `[len/num=0][cap=0][data_ptr=0]` and store its
/// handle in `dst` — the empty form of a sequence or a map (their element/entry shape differs only
/// at use, not at creation). `class` is the caller's best knowledge at THIS point: a Map passes
/// `Raw` regardless (its key/value handle-ness is only known at the first insert — see
/// `lower_map_insert`), harmless since an empty map's `len` is 0 either way.
fn emit_empty_header(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u32, class: AllocClass) {
    if let Some(f) = ctx
        .rt_alloc
        .is_none()
        .then(|| ctx.runtime.and_then(|r| r.index(RuntimeFn::NewHeader)))
        .flatten()
    {
        i32_const(code, class.code() as i32);
        code.push(0x10);
        leb_u32(code, f);
        local_set(code, dst);
        return;
    }
    let hs = num_regs + 5;
    i32_const(code, 16);
    emit_alloc(code, ctx, hs, class);
    for off in [0u32, 4, 8] {
        local_get(code, hs);
        i32_const(code, 0);
        i32_store(code, off);
    }
    local_get(code, hs);
    local_set(code, dst);
}

/// Allocate an empty sequence header plus its proof-sized element buffer.
/// Unlike the generic empty-header path, this preserves the shared fixed-size
/// proof in both linked and self-contained modules, so its pushes never grow.
fn emit_fixed_header(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    dst: u32,
    class: AllocClass,
    capacity: u32,
) {
    let header = num_regs + 5;
    let data = num_regs + 6;
    i32_const(code, 16);
    emit_alloc(code, ctx, header, class);
    i32_const(code, (capacity as i64 * 8) as i32);
    emit_alloc(code, ctx, data, AllocClass::Raw);
    local_get(code, header);
    i32_const(code, 0);
    i32_store(code, 0);
    local_get(code, header);
    i32_const(code, capacity as i32);
    i32_store(code, 4);
    local_get(code, header);
    local_get(code, data);
    i32_store(code, 8);
    local_get(code, header);
    local_set(code, dst);
}

/// Allocate an empty collection header plus a buffer sized by a runtime Int
/// capacity. Negative hints become zero; oversized hints clamp to the largest
/// representable linear-memory allocation for this element stride.
fn emit_capacity_header(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    dst: u32,
    capacity: u32,
    class: AllocClass,
    stride: i32,
) {
    let header = num_regs + 5;
    let data = num_regs + 6;
    let cap = num_regs + 7;
    let limit = i32::MAX / stride;

    local_get(code, capacity);
    code.push(0x42); // i64.const 0
    leb_i64(code, 0);
    code.push(0x55); // i64.gt_s
    code.push(0x04); // if (result i32)
    code.push(I32);
    local_get(code, capacity);
    code.push(0x42); // i64.const limit
    leb_i64(code, limit as i64);
    code.push(0x53); // i64.lt_s
    code.push(0x04); // if (result i32)
    code.push(I32);
    local_get(code, capacity);
    code.push(0xA7); // i32.wrap_i64
    code.push(0x05); // else
    i32_const(code, limit);
    code.push(0x0B); // end inner if
    code.push(0x05); // else
    i32_const(code, 0);
    code.push(0x0B); // end outer if
    local_set(code, cap);

    i32_const(code, 16);
    emit_alloc(code, ctx, header, class);
    local_get(code, cap);
    i32_const(code, stride);
    code.push(0x6C); // i32.mul
    emit_alloc(code, ctx, data, AllocClass::Raw);
    local_get(code, header);
    i32_const(code, 0);
    i32_store(code, 0);
    local_get(code, header);
    local_get(code, cap);
    i32_store(code, 4);
    local_get(code, header);
    local_get(code, data);
    i32_store(code, 8);
    local_get(code, header);
    local_set(code, dst);
}

/// COPY-ON-WRITE reference counting mirrors the VM's `Rc` so value semantics (`LOGOS_VALUE_SEMANTICS`,
/// now default-on in the tree-walker/VM) holds in native wasm. The 4th header word (offset 12, the
/// spare slot past `[len][cap][data_ptr]`) holds the count of EXTRA references beyond the owner in
/// bits 8–31 — so 0 means "uniquely owned": a header is born with its class byte and a zero count, and
/// `__logos_free` zeroes word 12 of every block it takes back, so a recycled header starts at zero too.
/// A `Call` argument RETAINS (`++`, gaining the callee's parameter as a second holder); a mutation op
/// checks the count and clones first when it is nonzero, so the write can't be seen through the other
/// holder. Outside a loop ([`Ctx::hot`]) the retain is a call to `__logos_share`; inside one it stays
/// inline, where a call would be paid per iteration.
fn emit_retain(code: &mut Vec<u8>, ctx: &Ctx, reg: u16) {
    if let Some(f) = (!ctx.hot.get())
        .then(|| ctx.runtime.and_then(|r| r.index(RuntimeFn::Share)))
        .flatten()
    {
        local_get(code, reg as u32);
        code.push(0x10);
        leb_u32(code, f);
        return;
    }
    emit_share_inline(code, |c| local_get(c, reg as u32));
}

/// `extra_refs + 1` at word 12 of the header `push` leaves on the stack.
fn emit_share_inline(code: &mut Vec<u8>, push: impl Fn(&mut Vec<u8>)) {
    push(code);
    push(code);
    i32_load(code, 12);
    i32_const(code, 0x100); // extra_refs occupies bits 8-31; the class byte (bits 0-7) is untouched
    code.push(0x6A); // extra_refs + 1
    i32_store(code, 12);
}

/// S8's true-count RETAIN: `call __logos_retain(reg)` — a NEW, second holder of a counted value at
/// a genuine heap-resident slot (a global, a `Seq`/`Set` element, a struct field, a closure
/// capture). Deliberately NOT the same call as [`emit_retain`] above (word 12, COW-only, gated on
/// [`cow_clonable`] — a narrower, register-aliasing concern this is not): this is gated on
/// [`counted_kind`] and a no-op module-wide when `!ctx.heap_counting()` (a batch/linked module).
fn emit_true_retain(code: &mut Vec<u8>, ctx: &Ctx, reg: u16) {
    emit_true_retain_of(code, ctx, |c| local_get(c, reg as u32));
}

/// [`emit_true_retain`] of the handle `push_handle` leaves on the stack.
fn emit_true_retain_of(code: &mut Vec<u8>, ctx: &Ctx, push_handle: impl Fn(&mut Vec<u8>)) {
    let Some(f) = ctx.runtime.and_then(|r| r.index(RuntimeFn::Retain)) else {
        return;
    };
    push_handle(code);
    code.push(0x10);
    leb_u32(code, f);
}

/// Every element handle of the list in local `hdr` gains a copy-on-write hold (header word 12, the
/// count above the class byte) — the other list sharing it is a second holder. `counter` is a free
/// i32 scratch local.
fn emit_cow_retain_each_element(code: &mut Vec<u8>, hdr: u32, counter: u32) {
    i32_const(code, 0);
    local_set(code, counter);
    code.push(0x02);
    code.push(0x40); // block
    code.push(0x03);
    code.push(0x40); // loop
    local_get(code, counter);
    local_get(code, hdr);
    i32_load(code, 0); // element count
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if exit
    for _ in 0..2 {
        local_get(code, hdr);
        i32_load(code, 8); // data_ptr
        local_get(code, counter);
        i32_const(code, 8);
        code.push(0x6C);
        code.push(0x6A);
        i32_load(code, 0); // the element handle, twice: address and value
    }
    i32_load(code, 12);
    i32_const(code, 0x100);
    code.push(0x6A);
    i32_store(code, 12);
    local_get(code, counter);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, counter);
    code.push(0x0C);
    leb_u32(code, 0); // br loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
}

/// S8's true count for a container that has just become a further holder of everything it holds —
/// a copy sharing its source's elements or entries, a literal built from registers: `call
/// __logos_retain_children(local hdr)`, which walks the slots the block's class names. One call
/// whatever the length. A no-op module-wide when `!ctx.heap_counting()`.
fn emit_retain_children(code: &mut Vec<u8>, ctx: &Ctx, hdr: u32) {
    let Some(f) = ctx.runtime.and_then(|r| r.index(RuntimeFn::RetainChildren)) else {
        return;
    };
    local_get(code, hdr);
    code.push(0x10);
    leb_u32(code, f);
}

/// S8's true-count MID-TURN RELEASE: `call __logos_release(handle)`, `handle` computed by
/// `push_handle` (a closure so the caller can load a value OUT OF MEMORY — the slot a store is
/// about to overwrite — rather than only ever a register). A no-op module-wide when
/// `!ctx.heap_counting()`.
fn emit_true_release(code: &mut Vec<u8>, ctx: &Ctx, push_handle: impl Fn(&mut Vec<u8>)) {
    let Some(f) = ctx.runtime.and_then(|r| r.index(RuntimeFn::Release)) else {
        return;
    };
    push_handle(code);
    code.push(0x10);
    leb_u32(code, f);
}

/// The kinds whose values count their extra holders (header word 12) — every copy-on-write kind, and
/// a record, whose writes copy it only while another holder shares it.
fn counts_aliases(k: Option<Kind>) -> bool {
    cow_clonable(k) || k == Some(Kind::Struct)
}

/// The mutable heap kinds a copy-on-write clone can currently reproduce (everything `lower_deep_clone`
/// handles). A kind outside this set skips the COW guard and mutates in place — sound as long as no
/// aliasing test exercises it (`Map` COW is added in a follow-up).
fn cow_clonable(k: Option<Kind>) -> bool {
    matches!(
        k,
        Some(Kind::SeqInt)
            | Some(Kind::SeqBool)
            | Some(Kind::SeqFloat)
            | Some(Kind::SeqWord32)
            | Some(Kind::SeqWord64)
            | Some(Kind::SeqAny)
            | Some(Kind::Set)
            | Some(Kind::SetText)
            | Some(Kind::SeqSeqInt)
            | Some(Kind::Map)
            | Some(Kind::SeqText)
            | Some(Kind::SeqStruct)
            | Some(Kind::SeqEnum)
            | Some(Kind::SeqHandle)
    )
}

/// The `__logos_seq_clone` mode of a kind whose copy is its slots as they are — its header's class,
/// and what each element gains from the copy (the same holds [`emit_collection_clone`]'s inline arms
/// give) — or `None` for one whose copy is not flat: a Text's bytes, a map's entries, a matrix's rows
/// each copied.
fn seq_clone_mode(k: Kind) -> Option<i32> {
    let handles = i32::from(AllocClass::HandleHeader.code()) | CLONE_RETAINS;
    Some(match k {
        Kind::SeqInt
        | Kind::SeqBool
        | Kind::SeqFloat
        | Kind::SeqWord32
        | Kind::SeqWord64
        | Kind::Set => i32::from(AllocClass::ScalarHeader.code()),
        Kind::SeqAny | Kind::SetText | Kind::SeqText | Kind::SeqEnum => handles,
        Kind::SeqStruct | Kind::SeqHandle => handles | CLONE_SHARES,
        _ => return None,
    })
}

/// `cow(reg)` — the copy-on-write guard emitted before a mutation. If the object has any extra
/// reference (header word 12 nonzero) it is replaced by a fresh deep clone (word 12 == 0), so the
/// mutation stays private — exactly as the VM's `maybe_cow` clones an `Rc` whose `strong_count > 1`.
/// A uniquely-owned object mutates in place (the common build-then-scan case pays only a load+branch).
fn emit_cow(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    structs: &kind::StructLayout,
    ctx: &Ctx,
    num_regs: u32,
    reg: u16,
) -> R<()> {
    if !cow_clonable(kinds.get(reg as usize)) {
        return Ok(());
    }
    if let Some(mode) = kinds.get(reg as usize).and_then(seq_clone_mode) {
        if ctx.hot.get() {
            // In a loop ([`Ctx::hot`]): the test inline, the copy the runtime's.
            if let Some(clone) = ctx.runtime.and_then(|r| r.index(RuntimeFn::SeqClone)) {
                emit_should_clone(code, reg as u32);
                code.push(0x04);
                code.push(0x40);
                local_get(code, reg as u32);
                i32_const(code, mode);
                code.push(0x10);
                leb_u32(code, clone);
                local_set(code, reg as u32);
                code.push(0x0B);
                return Ok(());
            }
        } else if let Some(own) = ctx.runtime.and_then(|r| r.index(RuntimeFn::SeqOwn)) {
            local_get(code, reg as u32);
            i32_const(code, mode);
            code.push(0x10);
            leb_u32(code, own);
            local_set(code, reg as u32);
            return Ok(());
        }
    }
    if kinds.get(reg as usize) == Some(Kind::Map) {
        if let Some(own) = ctx.runtime.and_then(|r| r.index(RuntimeFn::MapOwn)) {
            local_get(code, reg as u32);
            code.push(0x10);
            leb_u32(code, own);
            local_set(code, reg as u32);
            return Ok(());
        }
    }
    emit_should_clone(code, reg as u32);
    code.push(0x04);
    code.push(0x40); // if should_clone
    lower_deep_clone(code, kinds, structs, ctx, num_regs, reg, reg)?;
    code.push(0x0B); // end
    Ok(())
}

/// `[] -> [should_clone]` for the copy-on-write value whose handle is in `local`: word 12 ≥ 256,
/// signed. A `SharedMap`'s word 12 is deeply negative (`SHARED_MAP_MARK`) and never clones whatever
/// its extra-refs count; anything else clones exactly when its extra-refs count (bits 8-31, above the
/// class byte in bits 0-7) is non-zero — which, for a non-negative word, is the word reaching 256. One
/// signed compare reads both: an ordinary map's class byte (`MapScalarScalar` = 3 through
/// `MapHandleHandle` = 6) alone stays below 256, so it is not mistaken for an extra holder.
fn emit_should_clone(code: &mut Vec<u8>, local: u32) {
    local_get(code, local);
    i32_load(code, 12);
    i32_const(code, 0x100);
    code.push(0x4E); // i32.ge_s
}

/// A record's copy-on-write, for a write to the record in `obj` that another holder may share: when
/// its count of extra holders (header word 12) is nonzero, `obj` takes a fresh record with the same
/// fields. Every field whose value counts its holders (a list, a set, a map, a record) is then held
/// by both records and gains a hold, so a later write through either record's field copies that
/// value first. The field `overwritten` names — which the write that needed the copy replaces in the
/// copy — is not shared by the copy and gains nothing. A record nobody else holds is written in place.
fn emit_record_copy(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    kinds: &KindTable,
    structs: &kind::StructLayout,
    num_regs: u32,
    obj: u16,
    overwritten: Option<usize>,
) -> R<()> {
    let (hdr, data, idx) = (num_regs + 5, num_regs + 6, num_regs + 7);
    emit_should_clone(code, obj as u32);
    code.push(0x04);
    code.push(0x40); // if another holder shares the record
                     // A copy carries the source's field-kind bytes alongside its heterogeneous values.
    emit_buffer_clone(
        code,
        ctx,
        hdr,
        data,
        idx,
        obj as u32,
        false,
        AllocClass::RecordHeader,
    );
    local_get(code, hdr);
    local_set(code, obj as u32);
    if ctx.heap_counting() {
        emit_retain_children(code, ctx, obj as u32);
    }
    if let Some(slot_kinds) = record_slot_kinds(kinds, structs, obj) {
        for (slot, k) in slot_kinds.iter().enumerate() {
            // A field holding a value of the exact extension is a handle the copy owns: it takes a copy
            // of the value, so a change in place through either record reaches only its own.
            if *k == Some(Kind::Tower) && overwritten != Some(slot) {
                tower::own_field(code, ctx, obj, slot as u32)?;
                continue;
            }
            // The overwritten field gains no copy-on-write hold (the copy will not share it), but it
            // does gain its true count: the write releases the value it overwrites, from the copy.
            let cow_hold = counts_aliases(*k) && overwritten != Some(slot);
            if !cow_hold {
                continue;
            }
            local_get(code, obj as u32);
            i32_load(code, 8); // data_ptr
            i32_load(code, slot as u32 * 8);
            local_tee(code, idx);
            code.push(0x04);
            code.push(0x40); // if the field holds a value
            if cow_hold {
                local_get(code, idx);
                local_get(code, idx);
                i32_load(code, 12);
                i32_const(code, 0x100); // extra_refs occupies bits 8-31; the class byte is untouched
                code.push(0x6A);
                i32_store(code, 12);
            }
            code.push(0x0B);
        }
    }
    code.push(0x0B); // end if shared
    Ok(())
}

/// The copy-on-write count a `SharedMap` starts with. A CRDT map is one replica shared by every
/// holder (the reference tiers keep it behind an `Rc<RefCell>`), so a write through any holder must
/// land in it rather than in a private copy. A count this far below zero stays negative under every
/// retain and release a program can perform, and [`emit_cow`] clones only a positive count. The same
/// mark keeps the map's entries sorted by key ([`lower_map_insert`]), which is the order a CRDT map
/// displays in on every tier.
const SHARED_MAP_MARK: i32 = -(1 << 30);

/// Is a `Map`'s key an `Int` (i64, false) or `Text` (i32 handle, true)? Other key kinds are refused.
fn map_key_text(kinds: &KindTable, key: u16) -> R<bool> {
    match kinds.get(key as usize) {
        Some(Kind::Int) => Ok(false),
        Some(Kind::Text) => Ok(true),
        _ => Err(WasmLowerError::Unsupported("map key must be Int or Text")),
    }
}

/// Leave on the stack `1`/`0` for whether map entry `idx`'s key equals the query register `key` —
/// `i64.eq` for an Int key, or a `Text` byte-equality ([`emit_text_eq`], the entry's key in scratch
/// +8) for a Text key. `idx_local` is the entry index local.
fn emit_map_key_compare(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    key_text: bool,
    m: u32,
    idx_local: u32,
    key: u16,
) {
    if key_text {
        let entry_key = num_regs + 8;
        emit_map_entry_addr(code, m, idx_local);
        i32_load(code, 0); // entry key handle
        local_set(code, entry_key);
        emit_text_eq(code, ctx, num_regs, key as u32, entry_key);
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
        Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Moment) | Some(Kind::Duration)
        | Some(Kind::Time) | Some(Kind::Span) => Ok(i64_store),
        // A handle-valued map (`Map of K to Seq`/Struct/Text/…): the i32 handle rides the low word of
        // the 8-byte value slot; the entry copy in `emit_map_clone` moves the whole slot as an i64.
        Some(vk) if vk.wasm_valtype() == I32 => Ok(i32_store),
        _ => Err(WasmLowerError::Unsupported(
            "map value of an unsupported kind",
        )),
    }
}

/// The load opcode mirroring [`map_value_store`] (the value's kind is the `Index` destination's).
fn map_value_load(k: Option<Kind>) -> R<fn(&mut Vec<u8>, u32)> {
    match k {
        Some(Kind::Float) => Ok(f64_load),
        Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Moment) | Some(Kind::Duration)
        | Some(Kind::Time) | Some(Kind::Span) => Ok(i64_load),
        Some(vk) if vk.wasm_valtype() == I32 => Ok(i32_load),
        _ => Err(WasmLowerError::Unsupported(
            "map value of unknown/unsupported kind",
        )),
    }
}

/// Push the map key in register `key` as the i64 the map runtime takes: an Int as it is, a Text's
/// handle zero-extended.
fn emit_map_key_i64(code: &mut Vec<u8>, key: u16, key_text: bool) {
    local_get(code, key as u32);
    if key_text {
        code.push(0xAD); // i64.extend_i32_u
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
fn lower_map_insert(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    map: u16,
    key: u16,
    value: u16,
) -> R<()> {
    let key_text = map_key_text(kinds, key)?;
    let val_store = map_value_store(kinds.get(value as usize))?;
    let m = map as u32;
    // A map's key/value handle-ness is not known at `NewEmptyMap` (it starts empty and inert either
    // way) — the first insert is where it is FIRST reliably visible, from the actual key/value
    // registers being inserted, so it is stamped/re-stamped here. Idempotent: a statically-typed
    // map's key/value KINDS never change across inserts, so every call writes the same class. Only
    // the low byte (bits 0-7) is touched — bits 8-31 keep whatever the header already carries there,
    // an ordinary count or (for a `SharedMap`) the deeply-negative `SHARED_MAP_MARK` sign.
    let value_is_handle = kinds
        .get(value as usize)
        .map(|k| kind_is_handle(k))
        .unwrap_or(false);
    let class = AllocClass::of_map(key_text, value_is_handle);
    let put = if ctx.shared_maps {
        None
    } else {
        ctx.runtime.and_then(|r| r.index(RuntimeFn::MapPut))
    };
    if let Some(put) = put {
        local_get(code, m);
        emit_map_key_i64(code, key, key_text);
        i32_const(code, i32::from(class.code()));
        code.push(0x10);
        leb_u32(code, put);
        if ctx.heap_counting() && counted_kind(kinds.get(value as usize)) {
            // S8's true count: the entry stops holding the value it overwrites (none, for an entry
            // the put just appended: its value slot starts empty).
            let entry = num_regs + 5;
            local_tee(code, entry);
            emit_true_release(code, ctx, |c| {
                local_get(c, entry);
                i32_load(c, 8);
            });
        }
        local_get(code, value as u32);
        val_store(code, 8);
        return Ok(());
    }
    // A linked module, or a program with `SharedMap`s (whose entries stay sorted), inserts inline.
    local_get(code, m);
    local_get(code, m);
    i32_load(code, 12);
    i32_const(code, !0xFFu32 as i32); // 0xFFFFFF00 — clears the class byte, keeps count/sign
    code.push(0x71); // i32.and
    i32_const(code, class.code() as i32);
    code.push(0x72); // i32.or
    i32_store(code, 12);
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
    emit_map_key_compare(code, ctx, num_regs, key_text, m, idx, key);
    code.push(0x04);
    code.push(0x40); // if (key matches)
    if ctx.heap_counting() && counted_kind(kinds.get(value as usize)) {
        // S8's true count: the entry stops holding the value it is about to overwrite. (A copy of the
        // map holds that value too, and counted it when it was made — `emit_map_clone`.)
        emit_true_release(code, ctx, |c| {
            emit_map_entry_addr(c, m, idx);
            i32_load(c, 8);
        });
    }
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
                     // if not found: append a new entry — write in place when a slot is free (num < cap);
                     // otherwise double the capacity, copy the old entries across, and free the buffer growth
                     // abandons — dead by construction, exactly as `lower_list_push_at` grows a sequence.
    let new_cap = num_regs + 23; // dedicated, alongside the matching risk in `emit_set_add_elem`
    local_get(code, found);
    code.push(0x45); // i32.eqz
    code.push(0x04);
    code.push(0x40); // if (!found)
    local_get(code, m);
    i32_load(code, 0); // num
    local_get(code, m);
    i32_load(code, 4); // cap
    code.push(0x4E); // i32.ge_s → num >= cap
    code.push(0x04);
    code.push(0x40); // if (grow)
    {
        // new_cap = cap == 0 ? 4 : cap * 2
        i32_const(code, 4);
        local_get(code, m);
        i32_load(code, 4);
        i32_const(code, 2);
        code.push(0x6C);
        local_get(code, m);
        i32_load(code, 4);
        code.push(0x45); // i32.eqz → cap == 0
        code.push(0x1B); // select
        local_set(code, new_cap);
        // new = alloc(new_cap * 16)
        local_get(code, new_cap);
        i32_const(code, 16);
        code.push(0x6C);
        emit_alloc(code, ctx, new, AllocClass::Raw);
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
        // The old entry buffer is dead by construction: this header is the only holder and is
        // about to stop pointing at it, read before the header is repointed.
        emit_free(
            code,
            ctx,
            |c| {
                local_get(c, m);
                i32_load(c, 8); // the OLD data_ptr
            },
            |c| {
                local_get(c, m);
                i32_load(c, 4); // the OLD cap
                i32_const(c, 16);
                c.push(0x6C);
            },
        );
        // header: data_ptr = new; cap = new_cap
        local_get(code, m);
        local_get(code, new);
        i32_store(code, 8);
        local_get(code, m);
        local_get(code, new_cap);
        i32_store(code, 4);
    }
    code.push(0x0B); // end if (grow)
                     // entry[num] = (key, value) — the key is an i64 (Int) or an i32 handle (Text); `m`'s
                     // data_ptr already names the buffer with room, whichever branch above ran.
    local_get(code, m);
    i32_load(code, 8);
    local_get(code, m);
    i32_load(code, 0);
    i32_const(code, 16);
    code.push(0x6C);
    code.push(0x6A); // addr of new entry slot
    local_get(code, key as u32);
    if key_text {
        i32_store(code, 0);
        // S8's true count: the map holds the Text key it now keeps (a map never releases what it
        // holds, so a counted key simply lives as long as it could be read).
        if ctx.heap_counting() {
            emit_true_retain(code, ctx, key);
        }
    } else {
        i64_store(code, 0);
    }
    local_get(code, m);
    i32_load(code, 8);
    local_get(code, m);
    i32_load(code, 0);
    i32_const(code, 16);
    code.push(0x6C);
    code.push(0x6A);
    local_get(code, value as u32);
    val_store(code, 8);
    // header: num_entries += 1
    local_get(code, m);
    local_get(code, m);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6A);
    i32_store(code, 0);
    if ctx.shared_maps {
        // A shared (CRDT) map keeps its entries sorted by key: move the new entry down to its place.
        local_get(code, m);
        i32_load(code, 12);
        i32_const(code, 0);
        code.push(0x48); // i32.lt_s — marked shared
        code.push(0x04);
        code.push(0x40); // if
        emit_sink_last_entry_to_sorted_place(code, num_regs, m, key as u32, key_text, None);
        code.push(0x0B); // end if
    }
    code.push(0x0B); // end if (!found)
    Ok(())
}

/// Move the entry just appended to the map in local `m` (its last slot; key in register `key`) down
/// to its sorted place: every earlier entry whose key is greater moves up one slot, whole — key and
/// value. A Text key orders by bytes, an Int numerically, as `CrdtScalar`'s `Ord` does. Scratch
/// `+11` is the slot being filled, the key it is compared against waits in `+8` (Text) or `+16`
/// (Int), and the new entry's value rides the i64 scratch `+15` across the shift.
fn emit_sink_last_entry_to_sorted_place(
    code: &mut Vec<u8>,
    num_regs: u32,
    m: u32,
    key: u32,
    key_text: bool,
    exact_cmp: Option<u32>,
) {
    let at = num_regs + 11;
    let prev_key = if key_text || exact_cmp.is_some() {
        num_regs + 8
    } else {
        num_regs + 16
    };
    let new_value = num_regs + 15;
    let entry_at = |code: &mut Vec<u8>, minus_one: bool| {
        local_get(code, m);
        i32_load(code, 8);
        local_get(code, at);
        if minus_one {
            i32_const(code, 1);
            code.push(0x6B);
        }
        i32_const(code, 16);
        code.push(0x6C);
        code.push(0x6A);
    };
    // at = num - 1; hold the new entry's value slot (8 raw bytes) while earlier entries shift up
    local_get(code, m);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6B);
    local_set(code, at);
    entry_at(code, false);
    i64_load(code, 8);
    local_set(code, new_value);
    code.push(0x02);
    code.push(0x40); // block $placed
    code.push(0x03);
    code.push(0x40); // loop $sink
    local_get(code, at);
    code.push(0x45); // i32.eqz
    code.push(0x0D);
    leb_u32(code, 1); // br_if $placed
    entry_at(code, true);
    if let Some(cmp) = exact_cmp {
        i32_load(code, 0);
        i32_load(code, 8);
        i64_load(code, 0);
        local_get(code, key);
        i32_load(code, 8);
        i64_load(code, 0);
        code.push(0x10);
        leb_u32(code, cmp);
        i32_const(code, 0);
        code.push(0x48);
    } else if key_text {
        i32_load(code, 0);
        local_set(code, prev_key);
        emit_text_handles_lt(code, num_regs, prev_key, key as u32);
    } else {
        i64_load(code, 0);
        local_set(code, prev_key);
        local_get(code, prev_key);
        local_get(code, key as u32);
        code.push(0x53); // i64.lt_s
    }
    code.push(0x0D);
    leb_u32(code, 1); // br_if $placed — the previous key sorts first
    for off in [0u32, 8] {
        entry_at(code, false);
        entry_at(code, true);
        i64_load(code, off);
        i64_store(code, off);
    }
    local_get(code, at);
    i32_const(code, 1);
    code.push(0x6B);
    local_set(code, at);
    code.push(0x0C);
    leb_u32(code, 0); // br $sink
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
                     // entry[at] = (key, value)
    entry_at(code, false);
    local_get(code, key as u32);
    if key_text || exact_cmp.is_some() {
        i32_store(code, 0);
    } else {
        i64_store(code, 0);
    }
    entry_at(code, false);
    local_get(code, new_value);
    i64_store(code, 8);
}

/// `Map of Int to Int` get (`item key of m`): linear-scan for `key`, load its value into `dst`;
/// trap if absent (the tree-walker raises "Key not found", and the standalone module has no VM).
fn lower_map_get(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    map: u16,
    key: u16,
) -> R<()> {
    let key_text = map_key_text(kinds, key)?;
    let val_load = map_value_load(kinds.get(dst as usize))?;
    let m = map as u32;
    if let Some(at) = ctx.runtime.and_then(|r| r.index(RuntimeFn::MapAt)) {
        local_get(code, m);
        emit_map_key_i64(code, key, key_text);
        i32_const(code, i32::from(key_text));
        code.push(0x10);
        leb_u32(code, at);
        val_load(code, 8);
        local_set(code, dst as u32);
        return Ok(());
    }
    // A linked module has no runtime of its own: the scan inline.
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
    emit_map_key_compare(code, ctx, num_regs, key_text, m, idx, key);
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
fn lower_map_contains(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    map: u16,
    key: u16,
) -> R<()> {
    let key_text = map_key_text(kinds, key)?;
    let m = map as u32;
    if let Some(find) = ctx.runtime.and_then(|r| r.index(RuntimeFn::MapFind)) {
        local_get(code, m);
        emit_map_key_i64(code, key, key_text);
        i32_const(code, i32::from(key_text));
        code.push(0x10);
        leb_u32(code, find);
        i32_const(code, 0);
        code.push(0x47); // i32.ne — an entry holds the key
        code.push(0xAD); // i64.extend_i32_u
        local_set(code, dst as u32);
        return Ok(());
    }
    // A linked module has no runtime of its own: the scan inline.
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
    emit_map_key_compare(code, ctx, num_regs, key_text, m, idx, key);
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
/// wasm stack, so it drops into a scan's comparison exactly where an `i64.eq` would: a call of
/// `__logos_text_eq`, or — in a linked module, which has no runtime of its own — the byte loop
/// inline, in the `+9` (byte index) and `+10` (result) scratch, distinct from a set scan's
/// `+5`/`+6`/`+7` and the `+8` slot-handle temp.
fn emit_text_eq(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, a: u32, b: u32) {
    if let Some(eq) = ctx.runtime.and_then(|r| r.index(RuntimeFn::TextEq)) {
        local_get(code, a);
        local_get(code, b);
        code.push(0x10);
        leb_u32(code, eq);
        return;
    }
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
fn emit_set_elem_eq(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    set: u32,
    idx: u32,
    value: u32,
    is_text: bool,
) {
    local_get(code, set);
    i32_load(code, 8); // data_ptr
    local_get(code, idx);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A); // slot addr = data_ptr + idx*8
    if is_text {
        i32_load(code, 0); // the slot's Text handle
        local_set(code, num_regs + 8);
        emit_text_eq(code, ctx, num_regs, num_regs + 8, value);
    } else {
        i64_load(code, 0); // the slot's i64 element
        local_get(code, value);
        code.push(0x51); // i64.eq
    }
}

/// `Remove value from c` (`RemoveFrom`) on a `Set of Int`/`Set of Text` or a `Map` keyed by Int or
/// Text — linear-scan for the value (a Set element, or a Map key at entry offset 0); if found, close the gap
/// by moving every later slot down one and decrement the count. The elements keep their order — a
/// `Set`/`Map` displays in insertion order and a CRDT set in sorted order, and a removal changes
/// neither. A Text element or key compares by BYTE equality, not handle identity.
fn lower_remove_from(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    collection: u16,
    value: u16,
) -> R<()> {
    let set_text = matches!(
        kinds.get(collection as usize),
        Some(Kind::SetText) | Some(Kind::CrdtSetText)
    );
    let map_text_key = kinds.get(collection as usize) == Some(Kind::Map)
        && kinds.get(value as usize) == Some(Kind::Text);
    if !set_text && !map_text_key && kinds.get(value as usize) != Some(Kind::Int) {
        return Err(WasmLowerError::Unsupported("remove of a non-Int value"));
    }
    let stride: i32 = match kinds.get(collection as usize) {
        Some(Kind::Set) | Some(Kind::SetText) | Some(Kind::CrdtSetText) => 8,
        Some(Kind::Map) => 16,
        _ => {
            return Err(WasmLowerError::Unsupported(
                "remove from a non-set/map collection",
            ))
        }
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
                      // data[idx] (element / key at offset 0) == value ? — a Text element or key compares by bytes,
                      // an Int set / Map key by `i64.eq` at its stride.
    if set_text {
        emit_set_elem_eq(code, ctx, num_regs, c, idx, value as u32, true);
    } else if map_text_key {
        emit_map_key_compare(code, ctx, num_regs, true, c, idx, value);
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
                     // Removing a slot ends its ownership of each handle before the following slots move.
                     // Read the collection's class: the value kind is not present on RemoveFrom itself.
    if ctx.heap_counting() {
        for off in (0..stride as u32).step_by(8) {
            let mask: i32 = AllocClass::ALL
                .iter()
                .filter(|class| {
                    let (first, second) = class.held_handles();
                    if off == 0 {
                        first
                    } else {
                        second
                    }
                })
                .map(|class| 1i32 << class.code())
                .sum();
            i32_const(code, mask);
            local_get(code, c);
            i32_load(code, 12);
            i32_const(code, 0xFF);
            code.push(0x71);
            code.push(0x76); // i32.shr_u
            i32_const(code, 1);
            code.push(0x71);
            code.push(0x04);
            code.push(0x40);
            emit_true_release(code, ctx, |code| {
                local_get(code, c);
                i32_load(code, 8);
                local_get(code, idx);
                i32_const(code, stride);
                code.push(0x6C);
                code.push(0x6A);
                i32_load(code, off);
            });
            code.push(0x0B);
        }
    }
    // Close the gap: for idx in idx..num-1, slot[idx] = slot[idx+1] (one i64 for a Set, two for a
    // Map entry). `idx` is the loop counter from here; the scan is over.
    code.push(0x02);
    code.push(0x40); // block $shifted
    code.push(0x03);
    code.push(0x40); // loop $shift
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_get(code, c);
    i32_load(code, 0);
    code.push(0x4E); // i32.ge_s → idx + 1 >= num
    code.push(0x0D);
    leb_u32(code, 1); // br_if $shifted
    let offs: &[u32] = if stride == 16 { &[0, 8] } else { &[0] };
    for &off in offs {
        local_get(code, c);
        i32_load(code, 8);
        local_get(code, idx);
        i32_const(code, stride);
        code.push(0x6C);
        code.push(0x6A); // slot idx
        local_get(code, c);
        i32_load(code, 8);
        local_get(code, idx);
        i32_const(code, 1);
        code.push(0x6A);
        i32_const(code, stride);
        code.push(0x6C);
        code.push(0x6A); // slot idx + 1
        i64_load(code, off);
        i64_store(code, off);
    }
    local_get(code, idx);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, idx);
    code.push(0x0C);
    leb_u32(code, 0); // br $shift
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
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
fn lower_set_add(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    set: u16,
    value: u16,
) -> R<()> {
    let is_text = matches!(
        kinds.get(set as usize),
        Some(Kind::SetText) | Some(Kind::CrdtSetText)
    );
    if is_text {
        if kinds.get(value as usize) != Some(Kind::Text) {
            return Err(WasmLowerError::Unsupported(
                "adding a non-Text value to a Set of Text",
            ));
        }
    } else if kinds.get(value as usize) != Some(Kind::Int) {
        return Err(WasmLowerError::Unsupported(
            "set with a non-Int value (only Set of Int/Text)",
        ));
    }
    let order = if kinds.get(set as usize) == Some(Kind::CrdtSetText) {
        SetOrder::Sorted
    } else {
        SetOrder::Insertion
    };
    emit_set_add_elem(
        code,
        ctx,
        num_regs,
        set as u32,
        value as u32,
        is_text,
        order,
    );
    Ok(())
}

/// Where a set keeps a new element: at the end (a `Set`, which displays in insertion order), or at its
/// sorted position (a CRDT set, whose replicas agree on contents and not on arrival order, and which
/// displays sorted on every tier).
#[derive(Clone, Copy, PartialEq, Eq)]
enum SetOrder {
    Insertion,
    Sorted,
}

/// The add-if-absent core of [`lower_set_add`], over raw locals: scan the set whose handle is in
/// `set` for the value already held in `elem`; if absent, append it (realloc, like `ListPush`) and,
/// for a [`SetOrder::Sorted`] set, move it down to its sorted place. Uses the `+5`/`+6`/`+7` i32
/// scratch (`idx`/`found`/`new`), and `+8..+11` for a sorted insert, over an 8-byte stride. `set`
/// and `elem` must be locals untouched by those scratch slots — so union/intersect can drive it in a
/// loop with their own outer counter (`+9`) and loaded element (`+1`). `is_text` selects BYTE
/// equality (a `Set of Text`; the slot's low word is a Text handle) over `i64.eq`; a sorted set is a
/// set of Text, ordered by bytes as `String`'s `Ord` is.
fn emit_set_add_elem(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    s: u32,
    elem: u32,
    is_text: bool,
    order: SetOrder,
) {
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
    emit_set_elem_eq(code, ctx, num_regs, s, idx, elem, is_text); // set[idx] == elem ?
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
    // if not present: append — write in place when a slot is free (num < cap); otherwise double
    // the capacity, copy the existing elements across, and free the buffer growth abandons —
    // dead by construction, exactly as `lower_list_push_at` grows a sequence.
    //
    // ★ +8/+9 ARE LIVE ACROSS THIS CALL IN A CALLER, NOT MERELY SCRATCH. `a union b` /
    // `a intersection b` (`emit_set_copy_loop`) build their result set through `s` itself
    // aliasing +8 and keep their outer scan counter in +9 across every element add — a growth
    // scratch at either would corrupt the set being built rather than a spare temp, which is
    // why the new capacity gets its own dedicated local instead.
    let new_cap = num_regs + 23;
    local_get(code, found);
    code.push(0x45); // i32.eqz
    code.push(0x04);
    code.push(0x40); // if (!present)
    local_get(code, s);
    i32_load(code, 0); // num
    local_get(code, s);
    i32_load(code, 4); // cap
    code.push(0x4E); // i32.ge_s → num >= cap
    code.push(0x04);
    code.push(0x40); // if (grow)
    {
        // new_cap = cap == 0 ? 4 : cap * 2
        i32_const(code, 4);
        local_get(code, s);
        i32_load(code, 4);
        i32_const(code, 2);
        code.push(0x6C);
        local_get(code, s);
        i32_load(code, 4);
        code.push(0x45); // i32.eqz → cap == 0
        code.push(0x1B); // select
        local_set(code, new_cap);
        local_get(code, new_cap);
        i32_const(code, 8);
        code.push(0x6C);
        emit_alloc(code, ctx, new, AllocClass::Raw); // new = alloc(new_cap * 8)
        emit_seq_copy(code, idx, new, s, s, false); // copy the num existing values
                                                    // The old element buffer is dead by construction: this header is the only holder and is
                                                    // about to stop pointing at it, read before the header is repointed.
        emit_free(
            code,
            ctx,
            |c| {
                local_get(c, s);
                i32_load(c, 8); // the OLD data_ptr
            },
            |c| {
                local_get(c, s);
                i32_load(c, 4); // the OLD cap
                i32_const(c, 8);
                c.push(0x6C);
            },
        );
        local_get(code, s);
        local_get(code, new);
        i32_store(code, 8); // data_ptr = new
        local_get(code, s);
        local_get(code, new_cap);
        i32_store(code, 4); // cap = new_cap
    }
    code.push(0x0B); // end if (grow)
                     // slot[num] = elem — `s`'s data_ptr already names the buffer with room, whichever branch
                     // above ran. A Text value's slot high word is never read (only its low-word handle is), so
                     // whatever a reused block leaves there does not matter.
    local_get(code, s);
    i32_load(code, 8);
    local_get(code, s);
    i32_load(code, 0);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    local_get(code, elem);
    if is_text {
        i32_store(code, 0); // slot[num] = handle
                            // S8's true count: the set is a further holder of the Text it now keeps.
        if ctx.heap_counting() {
            emit_true_retain_of(code, ctx, |c| local_get(c, elem));
        }
    } else {
        i64_store(code, 0); // slot[num] = value
    }
    local_get(code, s);
    local_get(code, s);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6A);
    i32_store(code, 0); // num += 1
    if order == SetOrder::Sorted {
        emit_sink_last_to_sorted_place(code, num_regs, s, elem, is_text);
    }
    code.push(0x0B); // end if (!present)
}

/// Move the element just appended to the set in `s` (its last slot, value in `elem`) down to its
/// sorted place: every earlier element greater than it moves up one slot. A Text orders by bytes, an
/// Int numerically — `CrdtScalar`'s `Ord`. Scratch `+11` is the slot being filled; the previous
/// element sits in `+8` (a Text handle) or `+16` (an Int), and `+9`/`+10` serve the byte comparison.
fn emit_sink_last_to_sorted_place(
    code: &mut Vec<u8>,
    num_regs: u32,
    s: u32,
    elem: u32,
    is_text: bool,
) {
    let at = num_regs + 11;
    let prev = if is_text { num_regs + 8 } else { num_regs + 16 };
    let slot_at = |code: &mut Vec<u8>, minus_one: bool| {
        local_get(code, s);
        i32_load(code, 8);
        local_get(code, at);
        if minus_one {
            i32_const(code, 1);
            code.push(0x6B);
        }
        i32_const(code, 8);
        code.push(0x6C);
        code.push(0x6A);
    };
    // at = num - 1
    local_get(code, s);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6B);
    local_set(code, at);
    code.push(0x02);
    code.push(0x40); // block $placed
    code.push(0x03);
    code.push(0x40); // loop $sink
    local_get(code, at);
    code.push(0x45); // i32.eqz → at == 0
    code.push(0x0D);
    leb_u32(code, 1); // br_if $placed
                      // prev = slot[at - 1]; stop once prev < elem (no two elements are equal: the add scanned first)
    slot_at(code, true);
    if is_text {
        i32_load(code, 0);
        local_set(code, prev);
        emit_text_handles_lt(code, num_regs, prev, elem);
    } else {
        i64_load(code, 0);
        local_set(code, prev);
        local_get(code, prev);
        local_get(code, elem);
        code.push(0x53); // i64.lt_s
    }
    code.push(0x0D);
    leb_u32(code, 1); // br_if $placed
                      // slot[at] = prev; at -= 1
    slot_at(code, false);
    local_get(code, prev);
    if is_text {
        i32_store(code, 0);
    } else {
        i64_store(code, 0);
    }
    local_get(code, at);
    i32_const(code, 1);
    code.push(0x6B);
    local_set(code, at);
    code.push(0x0C);
    leb_u32(code, 0); // br $sink
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
                     // slot[at] = elem
    slot_at(code, false);
    local_get(code, elem);
    if is_text {
        i32_store(code, 0);
    } else {
        i64_store(code, 0);
    }
}

/// Whether the Text in local `a` sorts before the Text in local `b`, as an i32 on the stack: the
/// first differing byte decides (compared unsigned, so UTF-8 orders as code points), and a proper
/// prefix sorts first — `String`'s `Ord`. Scratch `+9` (byte index) and `+10` (answer).
fn emit_text_handles_lt(code: &mut Vec<u8>, num_regs: u32, a: u32, b: u32) {
    let (i, res) = (num_regs + 9, num_regs + 10);
    i32_const(code, 0);
    local_set(code, i);
    code.push(0x02);
    code.push(0x40); // block $decided
    code.push(0x03);
    code.push(0x40); // loop $bytes
                     // one of them ran out: the shorter sorts first
    local_get(code, i);
    local_get(code, a);
    i32_load(code, 0);
    code.push(0x4F); // i32.ge_u → i >= len_a
    local_get(code, i);
    local_get(code, b);
    i32_load(code, 0);
    code.push(0x4F); // i32.ge_u → i >= len_b
    code.push(0x72); // i32.or
    code.push(0x04);
    code.push(0x40); // if
    local_get(code, a);
    i32_load(code, 0);
    local_get(code, b);
    i32_load(code, 0);
    code.push(0x49); // i32.lt_u → len_a < len_b
    local_set(code, res);
    code.push(0x0C);
    leb_u32(code, 2); // br $decided
    code.push(0x0B); // end if
                     // the bytes at i differ: that byte decides
    local_get(code, a);
    i32_load(code, 8);
    local_get(code, i);
    code.push(0x6A);
    i32_load8_u(code, 0);
    local_get(code, b);
    i32_load(code, 8);
    local_get(code, i);
    code.push(0x6A);
    i32_load8_u(code, 0);
    code.push(0x47); // i32.ne
    code.push(0x04);
    code.push(0x40); // if
    local_get(code, a);
    i32_load(code, 8);
    local_get(code, i);
    code.push(0x6A);
    i32_load8_u(code, 0);
    local_get(code, b);
    i32_load(code, 8);
    local_get(code, i);
    code.push(0x6A);
    i32_load8_u(code, 0);
    code.push(0x49); // i32.lt_u
    local_set(code, res);
    code.push(0x0C);
    leb_u32(code, 2); // br $decided
    code.push(0x0B); // end if
    local_get(code, i);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, i);
    code.push(0x0C);
    leb_u32(code, 0); // br $bytes
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    local_get(code, res);
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
fn emit_set_copy_loop(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    result: u32,
    src: u32,
    filter: Option<u32>,
) {
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
        None => emit_set_add_elem(
            code,
            ctx,
            num_regs,
            result,
            elem,
            false,
            SetOrder::Insertion,
        ),
        Some(other) => {
            emit_set_contains_to(code, other, elem, cfound, cidx);
            local_get(code, cfound);
            code.push(0x04);
            code.push(0x40); // if (in `other`)
            emit_set_add_elem(
                code,
                ctx,
                num_regs,
                result,
                elem,
                false,
                SetOrder::Insertion,
            );
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
fn lower_union(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    lhs: u16,
    rhs: u16,
) -> R<()> {
    require_set(kinds, lhs)?;
    require_set(kinds, rhs)?;
    let result = num_regs + 8;
    // `require_set` only accepts plain `Kind::Set` (Int elements) — `SetText` union/intersect is
    // refused above — so the result is always a scalar-element header.
    emit_empty_header(code, ctx, num_regs, result, AllocClass::ScalarHeader);
    emit_set_copy_loop(code, ctx, num_regs, result, lhs as u32, None);
    emit_set_copy_loop(code, ctx, num_regs, result, rhs as u32, None);
    local_get(code, result);
    local_set(code, dst as u32);
    Ok(())
}

/// `a intersection b` — a fresh `Set` of `a`'s elements (in `a`'s order) that are also in `b`,
/// matching `semantics::collections::intersection`. Same build-into-`+8`-then-bind discipline as
/// [`lower_union`].
fn lower_intersect(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    lhs: u16,
    rhs: u16,
) -> R<()> {
    require_set(kinds, lhs)?;
    require_set(kinds, rhs)?;
    let result = num_regs + 8;
    emit_empty_header(code, ctx, num_regs, result, AllocClass::ScalarHeader);
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
        Err(WasmLowerError::Unsupported(
            "union/intersect of a non-Set value",
        ))
    }
}

/// `seq contains value` / `set contains value` — a linear membership scan (value equality,
/// matching the tree-walker's `List::position` / set membership): `dst = 1` if any element equals
/// `value`, else `0`. Int/Set-of-Int compare with `i64.eq`, Float with `f64.eq`.
fn lower_contains(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    collection: u16,
    value: u16,
) -> R<()> {
    let set_text = matches!(
        kinds.get(collection as usize),
        Some(Kind::SetText) | Some(Kind::CrdtSetText)
    );
    let elem = match kinds.get(collection as usize) {
        Some(Kind::Set) => Kind::Int, // a Set of Int holds i64 values
        Some(Kind::SetText) | Some(Kind::CrdtSetText) => Kind::Text, // a Set of Text scans by byte equality
        _ => seq_elem_kind(kinds, collection)?,
    };
    // A `Set of Text` `contains` DOES value (byte) equality (below); a plain `seq of Text contains
    // text` is still deferred (the scalar membership scan would wrongly compare Text handle pointers).
    if elem == Kind::Text && !set_text {
        return Err(WasmLowerError::Unsupported(
            "contains over a sequence of Text (needs byte-equality)",
        ));
    }
    let (elem_load, elem_eq): (fn(&mut Vec<u8>, u32), u8) = if elem == Kind::Float {
        (f64_load, 0x61)
    } else {
        (i64_load, 0x51)
    };
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
        emit_set_elem_eq(code, ctx, num_regs, col, idx, value as u32, true);
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
fn lower_concat(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, dst: u16, lhs: u16, rhs: u16) -> R<()> {
    // Render each operand into a Text handle held in a dedicated scratch local (the byte-copy below
    // reads their headers; rendering only borrows the +5/+6/+7 temps or calls a display function, so
    // the two handles in +8/+9 survive across both and the copy).
    let (a, b) = (plan.num_regs + 8, plan.num_regs + 9);
    display::emit_display_of(code, plan, ctx, lhs, a)?;
    display::emit_display_of(code, plan, ctx, rhs, b)?;
    emit_text_concat(code, ctx, plan.num_regs, a, b, dst as u32);
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

/// `"{x:.9}"` / `"{x:>6}"` / `"{x=}"` — a formatted interpolation piece. The VM's `FormatValue`
/// renders `debug_prefix? + format(src, spec?)`: an optional `name=` prefix (the `{x=}` debug form),
/// then the value under its spec (or its default display when there is none). This lowers the value
/// piece into `dst` via [`lower_format_value_piece`], then — for a debug piece — prepends the constant
/// `name=` `Text` (concat `prefix ++ dst -> dst`), byte-identical to the VM's `String` assembly. The
/// prefix is a compile-time `Text` constant; the `$` currency spec is still refused (a documented deferral).
fn lower_format_value(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    dst: u16,
    src: u16,
    spec: u32,
    debug_prefix: u32,
) -> R<()> {
    let num_regs = plan.num_regs;
    // The `{x=}` debug piece prepends `name=` to the value. Resolve the prefix `Text` constant now,
    // append the literal `=`, and materialize it — after the value is formatted into `dst`, concat it
    // in front, matching the VM's `out.push_str(prefix); out.push('='); out.push_str(value)`.
    let prefix_bytes = match debug_prefix {
        u32::MAX => None,
        idx => match ctx.constants.get(idx as usize) {
            Some(Constant::Text(s)) => Some(format!("{s}=")),
            _ => {
                return Err(WasmLowerError::Unsupported(
                    "debug prefix is not a text constant",
                ))
            }
        },
    };
    lower_format_value_piece(code, plan, ctx, dst, src, spec)?;
    if let Some(prefix) = prefix_bytes {
        // prefix in +8, formatted value in +9 (i32 handle scratch, disjoint from concat's +5/+6/+7);
        // out = prefix ++ value → dst.
        let (pfx, val) = (num_regs + 8, num_regs + 9);
        local_get(code, dst as u32);
        local_set(code, val);
        lower_text_literal(code, ctx, num_regs, prefix.as_bytes());
        local_set(code, pfx);
        emit_text_concat(code, ctx, num_regs, pfx, val, dst as u32);
    }
    Ok(())
}

/// Render `src` under its format spec into a fresh `Text` handle stored in `dst` (the value piece of
/// [`lower_format_value`], no debug prefix). A `None` spec (`u32::MAX`) uses the value's default display
/// (`to_display_string` via [`emit_stringify`]); `.N` precision and `>N`/`<N`/`^N`/bare-`N` alignment/width
/// run the same host formatters the VM's `apply_format_spec` does.
fn lower_format_value_piece(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    dst: u16,
    src: u16,
    spec: u32,
) -> R<()> {
    let (kinds, num_regs) = (&plan.kinds, plan.num_regs);
    let spec_s = match spec {
        // No spec: the value's default display, exactly the VM's `reg(src).to_display_string()`.
        u32::MAX => {
            display::emit_display_of(code, plan, ctx, src, dst as u32)?;
            return Ok(());
        }
        idx => match ctx.constants.get(idx as usize) {
            Some(Constant::Text(s)) => s.as_str(),
            _ => {
                return Err(WasmLowerError::Unsupported(
                    "format spec is not a text constant",
                ))
            }
        },
    };
    // Alignment / bare-width spec: stringify the value (any stringifiable kind), then pad to `width`
    // with spaces via `fmt_align_into` (the SAME Rust `format!` `apply_format_spec` runs).
    if !spec_s.starts_with('.') {
        let (align, width) = parse_align_spec(spec_s)
            .ok_or(WasmLowerError::Unsupported("unsupported format spec"))?;
        let fidx = (ctx.host_index)(HostFn::FmtAlignInto)
            .ok_or(WasmLowerError::Unsupported("align formatter not imported"))?;
        let (hdr, data, len, text) = (num_regs + 5, num_regs + 6, num_regs + 7, num_regs + 9);
        // text = stringify(src) — a Text handle (`fmt_align_into` reads its bytes)
        display::emit_display_of(code, plan, ctx, src, text)?;
        // data = alloc(text.len + width) — padding adds at most `width` single-byte spaces
        local_get(code, text);
        i32_load(code, 0);
        i32_const(code, width as i32);
        code.push(0x6A); // i32.add
        emit_alloc(code, ctx, data, AllocClass::Raw);
        // len = fmt_align_into(data, text, width, align)
        local_get(code, data);
        local_get(code, text);
        i32_const(code, width as i32);
        i32_const(code, align);
        code.push(0x10); // call
        leb_u32(code, fidx);
        local_set(code, len);
        // hdr = alloc(16); header = [len, cap = the buffer's size, data_ptr=data] — the capacity is
        // what a free returns, so it names the whole buffer, not the bytes written into it.
        i32_const(code, 16);
        emit_alloc(code, ctx, hdr, AllocClass::TextHeader);
        local_get(code, hdr);
        local_get(code, len);
        i32_store(code, 0);
        local_get(code, hdr);
        local_get(code, text);
        i32_load(code, 0);
        i32_const(code, width as i32);
        code.push(0x6A); // i32.add
        i32_store(code, 4);
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
        .ok_or(WasmLowerError::Unsupported(
            "unsupported format spec (only `.N` precision)",
        ))?;
    match kinds.get(src as usize) {
        Some(Kind::Float) | Some(Kind::Int) | Some(Kind::Bool) => {}
        _ => {
            return Err(WasmLowerError::Unsupported(
                "format `.N` on a non-numeric value",
            ))
        }
    }
    let (hdr, data, len) = (num_regs + 5, num_regs + 6, num_regs + 7);
    // hdr = alloc(16); data = alloc(340 + prec) bytes (worst-case f64 integer width + the decimals)
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr, AllocClass::TextHeader);
    i32_const(code, (340 + prec) as i32);
    emit_alloc(code, ctx, data, AllocClass::Raw);
    // len = fmt_f64_prec_into(data, src as f64, prec)
    let fidx = (ctx.host_index)(HostFn::FmtF64PrecInto).ok_or(WasmLowerError::Unsupported(
        "precision formatter not imported",
    ))?;
    local_get(code, data);
    push_as_f64(code, src, kinds.get(src as usize))?;
    i32_const(code, prec as i32);
    code.push(0x10); // call
    leb_u32(code, fidx);
    local_set(code, len);
    // header: len; cap = the buffer's size (what a free returns); data_ptr = data
    local_get(code, hdr);
    local_get(code, len);
    i32_store(code, 0);
    local_get(code, hdr);
    i32_const(code, (340 + prec) as i32);
    i32_store(code, 4);
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
    if let Some(concat) = ctx.runtime.and_then(|r| r.index(RuntimeFn::TextConcat)) {
        local_get(code, a);
        local_get(code, b);
        code.push(0x10);
        leb_u32(code, concat);
        local_set(code, out);
        return;
    }
    // A linked module has no runtime of its own: the copy inline.
    let (hdr, data, idx) = (num_regs + 5, num_regs + 6, num_regs + 7);
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr, AllocClass::TextHeader);
    // data = alloc(len_a + len_b) bytes (the allocator re-aligns the next pointer, so the byte
    // buffer need not be padded)
    local_get(code, a);
    i32_load(code, 0);
    local_get(code, b);
    i32_load(code, 0);
    code.push(0x6A); // len_a + len_b
    emit_alloc(code, ctx, data, AllocClass::Raw);
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

/// The bit of a Text header's capacity word (word 4) that marks a GROWABLE buffer: one whose 4 bytes
/// before `data_ptr` hold its high-water mark, the length of the longest Text written into it so far,
/// and the 4 before those its VIEW COUNT, how many Text headers read it (an append in place adds a
/// view; the free cascade frees the buffer with its last view). Only [`emit_text_append`] allocates
/// one; every other Text's capacity is its buffer's size.
const TEXT_GROWABLE: i32 = i32::MIN;

/// `out = a ++ b` as an APPEND to `a`: a fresh 16-byte header over a buffer that grows by doubling.
///
/// A Text is an immutable view `[len][cap][data_ptr]`, and several views may share one buffer. `a`'s
/// bytes are extended IN PLACE when its buffer is growable, has room, and `a` is its frontier — no view
/// has written past `a`'s length (the buffer's high-water mark equals `len(a)`). Then nothing any view
/// can read is overwritten: every existing view ends at or before the mark. Otherwise the append copies
/// `a` and `b` into a fresh growable buffer of twice the new length. A loop appending to one Text is
/// therefore linear in what it writes, as the VM's in-place `+=` is, and correct however the Text is
/// aliased.
fn emit_text_append(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, a: u32, b: u32, out: u32) {
    let (hdr, data, idx, cap, len) = (
        num_regs + 5,
        num_regs + 6,
        num_regs + 7,
        num_regs + 10,
        num_regs + 11,
    );
    local_get(code, a);
    i32_load(code, 0);
    local_get(code, b);
    i32_load(code, 0);
    code.push(0x6A); // len = len(a) + len(b)
    local_set(code, len);
    local_get(code, a);
    i32_load(code, 4);
    local_set(code, cap);
    code.push(0x02); // block $built
    code.push(0x40);
    code.push(0x02); // block $copy
    code.push(0x40);
    // In place only for a growable buffer ...
    local_get(code, cap);
    i32_const(code, TEXT_GROWABLE);
    code.push(0x71); // i32.and
    code.push(0x45); // i32.eqz
    code.push(0x0D); // br_if $copy
    leb_u32(code, 0);
    // ... with room for the new length ...
    local_get(code, cap);
    i32_const(code, i32::MAX);
    code.push(0x71); // capacity bytes
    local_get(code, len);
    code.push(0x49); // i32.lt_u
    code.push(0x0D); // br_if $copy
    leb_u32(code, 0);
    // ... whose high-water mark is `a`'s own end.
    local_get(code, a);
    i32_load(code, 8);
    local_set(code, data);
    local_get(code, data);
    i32_const(code, 4);
    code.push(0x6B); // data - 4
    i32_load(code, 0);
    local_get(code, a);
    i32_load(code, 0);
    code.push(0x47); // i32.ne
    code.push(0x0D); // br_if $copy
    leb_u32(code, 0);
    emit_byte_copy(code, idx, data, a, b, true); // b's bytes at [len(a), len)
                                                 // One more view reads the buffer (the header built below).
    local_get(code, data);
    i32_const(code, 8);
    code.push(0x6B);
    local_get(code, data);
    i32_const(code, 8);
    code.push(0x6B);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6A);
    i32_store(code, 0);
    code.push(0x0C); // br $built
    leb_u32(code, 1);
    code.push(0x0B); // end $copy
                     // A fresh growable buffer: its view count, its mark word, then twice the new length (at least
                     // 16 bytes).
    local_get(code, len);
    i32_const(code, 1);
    code.push(0x74); // i32.shl — len * 2
    local_tee(code, cap);
    i32_const(code, 16);
    local_get(code, cap);
    i32_const(code, 16);
    code.push(0x4B); // i32.gt_u
    code.push(0x1B); // select — max(len * 2, 16)
    local_set(code, cap);
    local_get(code, cap);
    i32_const(code, 8);
    code.push(0x6A);
    emit_alloc(code, ctx, data, AllocClass::Raw);
    local_get(code, data);
    i32_const(code, 1);
    i32_store(code, 0); // one view: the header built below
    local_get(code, data);
    i32_const(code, 8);
    code.push(0x6A);
    local_set(code, data);
    emit_byte_copy(code, idx, data, a, a, false); // a's bytes at [0, len(a))
    emit_byte_copy(code, idx, data, a, b, true); // b's bytes at [len(a), len)
    local_get(code, cap);
    i32_const(code, TEXT_GROWABLE);
    code.push(0x72); // i32.or — mark the buffer growable
    local_set(code, cap);
    code.push(0x0B); // end $built
                     // The buffer's high-water mark is now this Text's end.
    local_get(code, data);
    i32_const(code, 4);
    code.push(0x6B);
    local_get(code, len);
    i32_store(code, 0);
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr, AllocClass::TextHeader);
    local_get(code, hdr);
    local_get(code, len);
    i32_store(code, 0);
    local_get(code, hdr);
    local_get(code, cap);
    i32_store(code, 4);
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
        Some(Kind::Nothing) | Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Char)
        | Some(Kind::Moment) | Some(Kind::Duration) | Some(Kind::Time) | Some(Kind::Span)
        | Some(Kind::Word64) => i64_store(code, off),
        Some(Kind::Date)
        | Some(Kind::SeqInt)
        | Some(Kind::SeqBool)
        | Some(Kind::SeqFloat)
        | Some(Kind::SeqText)
        | Some(Kind::SeqStruct)
        | Some(Kind::SeqEnum)
        | Some(Kind::SeqSeqInt)
        | Some(Kind::SeqHandle)
        | Some(Kind::SeqAny)
        | Some(Kind::Text)
        | Some(Kind::Struct)
        | Some(Kind::Map)
        | Some(Kind::Set)
        | Some(Kind::SetText)
        | Some(Kind::CrdtSetText)
        | Some(Kind::CrdtCounter)
        | Some(Kind::Enum)
        | Some(Kind::Closure)
        | Some(Kind::Tuple)
        | Some(Kind::Rational)
        | Some(Kind::Optional)
        | Some(Kind::Word32)
        | Some(Kind::SeqWord32)
        | Some(Kind::SeqWord64)
        | Some(Kind::BigInt)
        | Some(Kind::Complex)
        | Some(Kind::Modular)
        | Some(Kind::Decimal)
        | Some(Kind::Money)
        | Some(Kind::Quantity)
        | Some(Kind::Uuid)
        | Some(Kind::Lanes)
        | Some(Kind::LanesV)
        | Some(Kind::Dynamic)
        | Some(Kind::Tower) => i32_store(code, off),
        None => return Err(WasmLowerError::Unsupported("struct field of unknown kind")),
    }
    Ok(())
}

/// Load opcode for an 8-byte struct field slot — the mirror of [`emit_slot_store`].
fn emit_slot_load(code: &mut Vec<u8>, k: Option<Kind>, off: u32) -> R<()> {
    match k {
        Some(Kind::Float) => f64_load(code, off),
        Some(Kind::Nothing) | Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Char)
        | Some(Kind::Moment) | Some(Kind::Duration) | Some(Kind::Time) | Some(Kind::Span)
        | Some(Kind::Word64) => i64_load(code, off),
        Some(Kind::Date)
        | Some(Kind::SeqInt)
        | Some(Kind::SeqBool)
        | Some(Kind::SeqFloat)
        | Some(Kind::SeqText)
        | Some(Kind::SeqStruct)
        | Some(Kind::SeqEnum)
        | Some(Kind::SeqSeqInt)
        | Some(Kind::SeqHandle)
        | Some(Kind::SeqAny)
        | Some(Kind::Text)
        | Some(Kind::Struct)
        | Some(Kind::Map)
        | Some(Kind::Set)
        | Some(Kind::SetText)
        | Some(Kind::CrdtSetText)
        | Some(Kind::CrdtCounter)
        | Some(Kind::Enum)
        | Some(Kind::Closure)
        | Some(Kind::Tuple)
        | Some(Kind::Rational)
        | Some(Kind::Optional)
        | Some(Kind::Word32)
        | Some(Kind::SeqWord32)
        | Some(Kind::SeqWord64)
        | Some(Kind::BigInt)
        | Some(Kind::Complex)
        | Some(Kind::Modular)
        | Some(Kind::Decimal)
        | Some(Kind::Money)
        | Some(Kind::Quantity)
        | Some(Kind::Uuid)
        | Some(Kind::Lanes)
        | Some(Kind::LanesV)
        | Some(Kind::Dynamic)
        | Some(Kind::Tower) => i32_load(code, off),
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
    emit_alloc(code, ctx, hdr, AllocClass::RecordHeader);
    i32_const(
        code,
        i32::from(count) * if ctx.heap_counting() { 9 } else { 8 },
    );
    emit_alloc(code, ctx, data, AllocClass::Raw);
    local_get(code, hdr);
    i32_const(code, i32::from(count));
    i32_store(code, 0); // num_fields
    local_get(code, hdr);
    i32_const(code, i32::from(count));
    i32_store(code, 4); // cap
    local_get(code, hdr);
    local_get(code, data);
    i32_store(code, 8); // data_ptr
                        // Every slot starts empty when writes count holders: a block the allocator hands back from its
                        // free list still carries whatever was there, and a field's first write releases what the slot
                        // held.
    if ctx.heap_counting() {
        for k in 0..u32::from(count) {
            local_get(code, data);
            code.push(0x42); // i64.const 0
            code.push(0x00);
            i64_store(code, k * 8);
            local_get(code, data);
            i32_const(code, 0);
            i32_store8(code, u32::from(count) * 8 + k);
        }
    }
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
/// op's def/use footprint, via the exhaustive [`crate::vm::instruction::op_def_uses`], so no aliasing operand is
/// missed). State is reset at every basic-block leader, a conservative join (a struct owned on only
/// some incoming paths is treated as shared → it copies, never miscompiles). So construction and
/// reads cost nothing; only a write to a possibly-shared struct pays one copy.
fn cow_struct_inserts(
    ops: &[Op],
    kinds: &KindTable,
    num_regs: u32,
    functions: &[CompiledFunction],
) -> Vec<bool> {
    // `Set collection[index] to value` (`SetIndex`) is a STRUCT field write, same as `StructInsert`,
    // exactly when its collection is a struct handle and its index a Text — the compiler's own
    // place-mutation write-back for a nested field (`w's stock["k"]` reassembles `w`'s "stock" field
    // this way after mutating the extracted map), matching `machine.rs`'s identical `is_struct_text`
    // dispatch. Every other `SetIndex` (Map/Seq/Set) keeps its own COW gate ([`emit_cow`]).
    let is_struct_write = |op: &Op| match op {
        Op::SetIndex { collection, .. } | Op::SetIndexUnchecked { collection, .. } => {
            kinds.get(*collection as usize) == Some(Kind::Struct)
        }
        _ => false,
    };
    let mut cow = vec![false; ops.len()];
    let Some(blocks) = Blocks::new(ops) else {
        // Not self-contained (rejected downstream); be safe and copy every struct write.
        for (pc, op) in ops.iter().enumerate() {
            if matches!(op, Op::StructInsert { .. }) || is_struct_write(op) {
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
            Op::NewStruct { dst, .. }
            | Op::DeepClone { dst, .. }
            | Op::CallBuiltin {
                dst,
                builtin: BuiltinId::Copy,
                ..
            } => set_owned(&mut owned, *dst, true),
            Op::StructInsert { obj, value, .. } => {
                if !is_owned(&owned, *obj) {
                    cow[pc] = true; // mutating a possibly-shared struct → copy first
                }
                set_owned(&mut owned, *value, false); // the stored value is now aliased by obj.field
                set_owned(&mut owned, *obj, true); // obj is uniquely owned after (fresh copy, or already)
            }
            Op::SetIndex {
                collection,
                index,
                value,
            }
            | Op::SetIndexUnchecked {
                collection,
                index,
                value,
            } if is_struct_write(op) => {
                if !is_owned(&owned, *collection) {
                    cow[pc] = true;
                }
                set_owned(&mut owned, *index, false);
                set_owned(&mut owned, *value, false);
                set_owned(&mut owned, *collection, true);
            }
            // A push onto a record's list field may give the record a list of its own (the field's
            // copy-on-write), which is a write to the record.
            Op::ListPushField { obj, src, .. } => {
                if !is_owned(&owned, *obj) {
                    cow[pc] = true;
                }
                set_owned(&mut owned, *src, false);
                set_owned(&mut owned, *obj, true);
            }
            Op::GetField { dst, .. } => set_owned(&mut owned, *dst, false), // a borrowed field handle
            _ => {
                let (defs, uses) = crate::vm::instruction::op_def_uses(op, functions);
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
#[allow(clippy::too_many_arguments)]
/// The declared representation of a record slot, including records passed across functions.
fn declared_record_slot_kind(
    structs: &kind::StructLayout,
    types: &[StructTypeDef],
    constants: &[Constant],
    obj: u16,
    slot: usize,
) -> Option<Kind> {
    if let Some(ParamShape::Struct(layout)) = structs.reg_shape.get(&obj) {
        return layout.get(slot).map(|(_, k, _)| *k);
    }
    if let Some(layout) = structs
        .struct_name_of
        .get(&obj)
        .and_then(|name| kind::resolve_named_layout(name, types, constants))
    {
        if let Some((field, _)) = structs.reg_layout.get(&obj).and_then(|v| v.get(slot)) {
            return layout
                .iter()
                .find(|(f, _, _)| f == field)
                .map(|(_, k, _)| *k);
        }
        return layout.get(slot).map(|(_, k, _)| *k);
    }
    structs
        .call_result
        .get(&obj)
        .and_then(|v| v.get(slot))
        .map(|(_, k, _)| *k)
}

/// Materialize an integer as a Rational before storing it in a Rational field.
/// Its source register keeps its own scalar representation.
fn promote_rational_field(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    structs: &kind::StructLayout,
    ctx: &Ctx,
    num_regs: u32,
    obj: u16,
    slot: usize,
    value: u16,
) -> R<Option<u32>> {
    if declared_record_slot_kind(structs, ctx.struct_types, ctx.constants, obj, slot) != Some(Kind::Rational) {
        return Ok(None);
    }
    // The language holds nothing in a rational field a record is built without, which a compiled rational cannot
    // hold.
    if kinds.get(value as usize) == Some(Kind::Nothing) {
        return Err(WasmLowerError::Unsupported(
            "a record's rational field holding nothing: a compiled rational holds no nothing",
        ));
    }
    let widened = if ctx.linked { matches!(kinds.get(value as usize), Some(Kind::Int | Kind::BigInt)) } else { kinds.get(value as usize) == Some(Kind::Int) };
    if !widened {
        return Ok(None);
    }
    let handle = num_regs + 8;
    if ctx.linked {
        push_rational_operand(code, kinds, ctx, value)?;
        local_set(code, handle);
    } else {
        let data = num_regs + 9;
        i32_const(code, 16);
        emit_alloc(code, ctx, handle, AllocClass::ScalarHeader);
        i32_const(code, 16);
        emit_alloc(code, ctx, data, AllocClass::Raw);
        for off in [0u32, 4] {
            local_get(code, handle);
            i32_const(code, 2);
            i32_store(code, off);
        }
        local_get(code, handle);
        local_get(code, data);
        i32_store(code, 8);
        local_get(code, data);
        local_get(code, value as u32);
        i64_store(code, 0);
        local_get(code, data);
        i64c(code, 1);
        i64_store(code, 8);
    }
    // The new field owns the promoted value beyond the allocating turn.
    emit_true_retain_of(code, ctx, |c| local_get(c, handle));
    Ok(Some(handle))
}

/// Store `value` into `obj`'s field at `slot`; `held` is the local holding the field's own handle where the field or
/// the value is one of the exact extension ([`tower::field_value`]).
fn lower_struct_insert(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    structs: &kind::StructLayout,
    ctx: &Ctx,
    num_regs: u32,
    slot: u16,
    obj: u16,
    value: u16,
    cow: bool,
    held: Option<u32>,
) -> R<()> {
    // The target must be a known Struct handle (an i32). A struct param has no static layout, so it
    // is typed as a scalar here — reject rather than treat the scalar as a heap pointer (which the
    // copy-on-write below would, emitting invalid wasm).
    if kinds.get(obj as usize) != Some(Kind::Struct) {
        return Err(WasmLowerError::Unsupported(
            "struct field set on a value with no known struct layout",
        ));
    }
    if cow {
        emit_record_copy(
            code,
            ctx,
            kinds,
            structs,
            num_regs,
            obj,
            Some(usize::from(slot)),
        )?;
    }
    let promoted = match held {
        Some(h) => Some(h),
        None => promote_rational_field(
            code,
            kinds,
            structs,
            ctx,
            num_regs,
            obj,
            usize::from(slot),
            value,
        )?,
    };
    let stored_kind = if held.is_some() {
        Some(Kind::Tower)
    } else if promoted.is_some() {
        Some(Kind::Rational)
    } else {
        kinds.get(value as usize)
    };
    let off = u32::from(slot) * 8;
    // S8's true count: the field stops holding the value it is about to overwrite (a fresh record's
    // slots start empty — `lower_new_struct`).
    if ctx.heap_counting() && counted_kind(stored_kind) {
        emit_true_release(code, ctx, |c| {
            local_get(c, obj as u32);
            i32_load(c, 8); // data_ptr
            i32_load(c, off);
        });
    }
    local_get(code, obj as u32);
    i32_load(code, 8); // data_ptr
    local_get(code, promoted.unwrap_or(value as u32));
    emit_slot_store(code, stored_kind, off)?;
    emit_record_field_kind(code, ctx, obj, u32::from(slot), stored_kind);
    Ok(())
}

/// Write the field's ownership byte in the trailer, separate from its full-width scalar value.
fn emit_record_field_kind(code: &mut Vec<u8>, ctx: &Ctx, obj: u16, slot: u32, kind: Option<Kind>) {
    emit_record_field_is_handle(code, ctx, obj, slot, kind.is_some_and(kind_is_handle));
}

fn emit_record_field_is_handle(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    obj: u16,
    slot: u32,
    is_handle: bool,
) {
    if !ctx.heap_counting() {
        return;
    }
    local_get(code, obj as u32);
    i32_load(code, 8);
    local_get(code, obj as u32);
    i32_load(code, 0);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    i32_const(code, i32::from(is_handle));
    i32_store8(code, slot);
}

/// `obj's field` (`GetField`) — load the value from its static slot, at the width of the field's
/// kind (the inferred kind of `dst`).
fn lower_get_field(code: &mut Vec<u8>, kinds: &KindTable, slot: u16, dst: u16, obj: u16) -> R<()> {
    if kinds.get(obj as usize) != Some(Kind::Struct) {
        return Err(WasmLowerError::Unsupported(
            "field access on a value with no known struct layout",
        ));
    }
    let off = u32::from(slot) * 8;
    local_get(code, obj as u32);
    i32_load(code, 8); // data_ptr
    emit_slot_load(code, kinds.get(dst as usize), off)?;
    local_set(code, dst as u32);
    Ok(())
}

/// `Set collection[index] to value` where `collection` is a RECORD — `machine.rs`'s `is_struct_text`
/// overload of `SetIndex` (`st.fields.insert(field, new_val)`), value semantics like `StructInsert`.
/// The VM's place lowering writes a field back this way after mutating what it took out
/// (`w's stock["k"] = v` reassembles `w`'s `stock`), naming the field by a register that holds the
/// field's name as a constant loaded beside the write ([`PlaceSites::field_at`]) — so the slot is as
/// static as a `StructInsert`'s. The `nothing` the VM writes to clear the field before the mutation
/// stores nothing here ([`PlaceSites`]); its copy-on-write still runs, so the put-back that follows
/// lands in this record's own slots, never in a record it shares them with.
fn lower_record_field_set(
    code: &mut Vec<u8>,
    plan: &Plan,
    kinds: &KindTable,
    ctx: &Ctx,
    pc: usize,
    cow: bool,
    collection: u16,
    value: u16,
) -> R<()> {
    let field =
        plan.place
            .field_at
            .get(pc)
            .copied()
            .flatten()
            .ok_or(WasmLowerError::Unsupported(
                "record field written by a name computed at run time",
            ))?;
    let slot = record_field_slot(plan, collection, field).ok_or(WasmLowerError::Unsupported(
        "record field write on a value with no known record layout",
    ))?;
    if cow {
        // A write replaces the field in the copy, so the copy does not share what the field held. A
        // clear does not: the value it empties the field of goes on in the take's register, and the
        // original record — which another holder still has — still points at it.
        let clear = plan.place.clear.get(pc).copied().unwrap_or(false);
        let overwritten = (!clear).then_some(slot);
        emit_record_copy(
            code,
            ctx,
            kinds,
            &plan.structs,
            plan.num_regs,
            collection,
            overwritten,
        )?;
    }
    let clear = plan.place.clear.get(pc).copied().unwrap_or(false);
    // S8's true count: the field stops holding the value it held — a clear's value goes on in the
    // take's register (the put-back holds it again), a write's is replaced.
    let declared = declared_record_slot_kind(
        &plan.structs,
        ctx.struct_types,
        ctx.constants,
        collection,
        slot,
    );
    let field_kind = if declared == Some(Kind::Rational) {
        declared
    } else if clear {
        record_slot_kinds(kinds, &plan.structs, collection)
            .and_then(|ks| ks.get(slot).copied().flatten())
    } else {
        kinds.get(value as usize)
    };
    if ctx.heap_counting() && counted_kind(field_kind) {
        emit_true_release(code, ctx, |c| {
            local_get(c, collection as u32);
            i32_load(c, 8); // data_ptr
            i32_load(c, (slot as u32) * 8);
        });
    }
    if clear {
        // The field's value is out in the take's register until the put-back: the slot holds
        // nothing meanwhile, so a copy of the record made in between does not share the value
        // being mutated (and so does not make its next write copy it).
        local_get(code, collection as u32);
        i32_load(code, 8); // data_ptr
        code.push(0x42); // i64.const 0
        code.push(0x00);
        i64_store(code, (slot as u32) * 8);
        emit_record_field_kind(code, ctx, collection, slot as u32, None);
        return Ok(());
    }
    // A field of the exact extension is set by name only by a put-back, which returns the handle its take moved
    // out (a copy of the field's own); a value of another kind there has no lowering here.
    if declared == Some(Kind::Tower) && kinds.get(value as usize) != Some(Kind::Tower) {
        return Err(WasmLowerError::Unsupported(
            "a field of an exact type set by name to a value of another kind",
        ));
    }
    let promoted = promote_rational_field(
        code,
        kinds,
        &plan.structs,
        ctx,
        plan.num_regs,
        collection,
        slot,
        value,
    )?;
    let stored_kind = if promoted.is_some() {
        Some(Kind::Rational)
    } else {
        kinds.get(value as usize)
    };
    local_get(code, collection as u32);
    i32_load(code, 8); // data_ptr
    local_get(code, promoted.unwrap_or(value as u32));
    emit_slot_store(code, stored_kind, (slot as u32) * 8)?;
    emit_record_field_kind(code, ctx, collection, slot as u32, stored_kind);
    Ok(())
}

/// Whether a value of `kind` is its own deep clone — a scalar with no sub-structure to copy
/// recursively (`Int`/`Bool`/`Char`/`Float`/`Date`/`Moment`). A handle kind (Text/Seq/Struct/…) is not.
fn is_clone_trivial(kind: Option<Kind>) -> bool {
    matches!(
        kind,
        Some(
            Kind::Int
                | Kind::Bool
                | Kind::Char
                | Kind::Word32
                | Kind::Word64
                | Kind::Float
                | Kind::Date
                | Kind::Moment
                | Kind::Duration
                | Kind::Time
                | Kind::Span
        )
    )
}

/// A struct field this backend can deep-clone: a scalar (copied flat with the field buffer), a
/// `Text`/`Set`/scalar-sequence handle (clonable into an independent buffer one level down), or a
/// `Map` of scalar or Text entries (its entry buffer copied, a `SharedMap`'s mark with it). A field
/// that is itself a struct / enum / nested-handle sequence needs deeper recursion and is not cloned
/// here (the struct clone is then refused, soundly).
fn field_clone_ok(kind: Option<Kind>) -> bool {
    is_clone_trivial(kind)
        || matches!(
            kind,
            Some(
                Kind::Rational
                    | Kind::Text
                    | Kind::SeqInt
                    | Kind::SeqBool
                    | Kind::SeqFloat
                    | Kind::SeqWord32
                    | Kind::SeqWord64
                    | Kind::SeqAny
                    | Kind::Set
                    | Kind::Map
                    | Kind::Struct
            )
        )
}

/// Clone a `Text` (byte buffer) or sequence/`Set` (8-byte-slot buffer) whose handle is in `src_local`
/// into a fresh, independent object, leaving the new handle in `hdr`. `is_text` selects the copy
/// stride; `hdr`/`data`/`idx` are three scratch locals, and `data` does not survive the call (a slot
/// copy is `__logos_seq_clone`'s). The fresh buffer is why a later in-place mutation of the clone (or
/// the original) cannot be seen through the other. `class` is the CLONE's own class — always the
/// same as the source's, since a clone never changes kind; the caller already knows it, so this
/// function does not have to re-derive it from `is_text` alone.
fn emit_buffer_clone(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    hdr: u32,
    data: u32,
    idx: u32,
    src_local: u32,
    is_text: bool,
    class: AllocClass,
) {
    if !is_text {
        if let Some(clone) = ctx.runtime.and_then(|r| r.index(RuntimeFn::SeqClone)) {
            local_get(code, src_local);
            i32_const(code, class.code() as i32);
            code.push(0x10);
            leb_u32(code, clone);
            local_set(code, hdr);
            return;
        }
    }
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr, class);
    local_get(code, src_local);
    i32_load(code, 0); // len (bytes for Text, element count otherwise)
    if !is_text {
        i32_const(code, 8);
        code.push(0x6C); // i32.mul — 8-byte slots
    }
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
    if let Some(clone) = ctx.runtime.and_then(|r| r.index(RuntimeFn::MapClone)) {
        local_get(code, src_local);
        code.push(0x10);
        leb_u32(code, clone);
        local_set(code, hdr);
        return;
    }
    // A linked module has no runtime of its own: the copy inline.
    i32_const(code, 16);
    // The class stamped here is irrelevant — word 12 is fully overwritten below from `src_local`'s
    // own word 12, which already carries the map's real class (stamped at its first insert).
    emit_alloc(code, ctx, hdr, AllocClass::Raw);
    // data = alloc(num_entries * 16)
    local_get(code, src_local);
    i32_load(code, 0); // num_entries
    i32_const(code, 16);
    code.push(0x6C);
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
    // word 12 = (sign mark, if this is a program with SharedMaps and `src` is one) | src's class byte
    // — a plain map's clone keeps its class; a `SharedMap`'s clone stays another replica of the same
    // map (the sign mark), classified exactly like its source.
    local_get(code, hdr);
    if ctx.shared_maps {
        i32_const(code, SHARED_MAP_MARK);
        i32_const(code, 0);
        local_get(code, src_local);
        i32_load(code, 12);
        i32_const(code, 0);
        code.push(0x48); // i32.lt_s
        code.push(0x1B); // select
    } else {
        i32_const(code, 0);
    }
    local_get(code, src_local);
    i32_load(code, 12);
    i32_const(code, 0xFF);
    code.push(0x71); // i32.and — src's class byte
    code.push(0x72); // i32.or
    i32_store(code, 12);
    // S8's true count: the copy shares every entry — each handle key and value gains it as a holder.
    emit_retain_children(code, ctx, hdr);
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
    inner_class: AllocClass,
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
    emit_buffer_clone(code, ctx, ihdr, idata, iidx, isrc, is_text, inner_class);
    // data[counter*8] = ihdr (the fresh, independent inner handle)
    local_get(code, data);
    local_get(code, counter);
    i32_const(code, 8);
    code.push(0x6C); // i32.mul
    code.push(0x6A); // i32.add
    local_get(code, ihdr);
    i32_store(code, 0);
    // S8's true count: the slot is the fresh element's holder beyond the turn that made it.
    if ctx.heap_counting() {
        emit_true_retain_of(code, ctx, |c| local_get(c, ihdr));
    }
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

/// Each slot's kind of the record in `src`, in slot order: its DECLARED layout wherever the record's
/// type is known (built here, a parameter, a call's result), else the kinds of the registers that
/// filled a record built here with no declaration. An unfilled or unknown slot is `None`.
fn record_slot_kinds(
    kinds: &KindTable,
    structs: &kind::StructLayout,
    src: u16,
) -> Option<Vec<Option<Kind>>> {
    if let Some(ParamShape::Struct(layout)) = structs.reg_shape.get(&src) {
        return Some(layout.iter().map(|(_, k, _)| Some(*k)).collect());
    }
    let layout = structs.reg_layout.get(&src)?;
    Some(
        layout
            .iter()
            .map(|&(_, vr)| vr.and_then(|vr| kinds.get(vr as usize)))
            .collect(),
    )
}

/// `a copy of x` (`DeepClone`) — an independent deep copy. A scalar is its own value (copied); a
/// `Text` gets a fresh byte buffer; a sequence/struct of trivially-cloneable (scalar) elements gets
/// a fresh element buffer. The new buffer means a later mutation of the clone (or the original)
/// cannot be seen through the other — value semantics, matching the tree-walker's `deep_clone`. A
/// composite holding HANDLE sub-values (a struct with a Text/Seq field, a Seq of handles) needs a
/// recursive clone (a generated per-type helper); deferred — soundly rejected, never miscopied.
fn lower_deep_clone(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    structs: &kind::StructLayout,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    src: u16,
) -> R<()> {
    let (hdr, data, idx) = (num_regs + 5, num_regs + 6, num_regs + 7);
    let s = src as u32;
    match kinds.get(src as usize) {
        // Scalars and Rational values are immutable; copying preserves the same value.
        Some(Kind::Rational) | Some(Kind::Int) | Some(Kind::Bool) | Some(Kind::Word32)
        | Some(Kind::Word64) | Some(Kind::Float) | Some(Kind::Date) | Some(Kind::Moment)
        | Some(Kind::Duration) | Some(Kind::Time) | Some(Kind::Span) => {
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
            if record_slot_kinds(kinds, structs, src)
                .is_some_and(|ks| ks.iter().all(|&k| field_clone_ok(k))) =>
        {
            // A record's header and field slots have a sequence's shape: `num_fields` 8-byte slots.
            emit_buffer_clone(
                code,
                ctx,
                hdr,
                data,
                idx,
                s,
                false,
                AllocClass::RecordHeader,
            );
            local_get(code, hdr);
            i32_load(code, 8);
            local_set(code, data);
            // Recursively clone each handle field IN PLACE in the cloned buffer (`data`), overwriting
            // its flat-copied shared handle with a fresh one. Scratch `+8..+11` is disjoint from the
            // struct's `+5..+7`. The layout is compile-time, so this unrolls per handle field.
            let (fsrc, fhdr, fdata, fidx) =
                (num_regs + 8, num_regs + 9, num_regs + 10, num_regs + 11);
            let slot_kinds =
                record_slot_kinds(kinds, structs, src).expect("guarded by is_some_and above");
            for (slot, &fk) in slot_kinds.iter().enumerate() {
                if is_clone_trivial(fk) {
                    if fk == Some(Kind::Int) && ctx.exact.record_ints() {
                        exact::retain_record_integer(code, ctx, hdr, slot as u16);
                    }
                    continue; // immutable cells and scalars preserve their value in the flat copy
                }
                let off = slot as u32 * 8;
                if matches!(fk, Some(Kind::Rational | Kind::Struct)) {
                    // Nested records share their value until a field write copies it.
                    if fk == Some(Kind::Struct) {
                        emit_share_inline(code, |code| {
                            local_get(code, data);
                            i32_load(code, off);
                        });
                    }
                    emit_true_retain_of(code, ctx, |code| {
                        local_get(code, data);
                        i32_load(code, off);
                    });
                    continue;
                }
                local_get(code, data);
                i32_load(code, off); // the shared handle the flat copy duplicated
                local_set(code, fsrc);
                if fk == Some(Kind::Map) {
                    emit_map_clone(code, ctx, fhdr, fdata, fidx, fsrc);
                } else {
                    let is_text = fk == Some(Kind::Text);
                    // `field_clone_ok` (guarding this whole branch) only ever admits Text/SeqInt/
                    // SeqBool/SeqFloat/SeqAny/Set/Map here; of those, `SeqAny` is the one handle-
                    // element case.
                    let class = if is_text {
                        AllocClass::TextHeader
                    } else {
                        AllocClass::of_elements(fk == Some(Kind::SeqAny))
                    };
                    emit_buffer_clone(code, ctx, fhdr, fdata, fidx, fsrc, is_text, class);
                    if class == AllocClass::HandleHeader {
                        emit_retain_children(code, ctx, fhdr);
                    }
                }
                local_get(code, data);
                local_get(code, fhdr); // the fresh, independent handle
                i32_store(code, off);
                // S8's true count: the clone's field is the fresh copy's holder beyond its turn.
                if ctx.heap_counting() {
                    emit_true_retain_of(code, ctx, |c| local_get(c, fhdr));
                }
            }
            local_get(code, hdr);
            local_set(code, dst as u32);
            return Ok(());
        }
        Some(k) if is_collection_clonable(k) => {
            emit_collection_clone(code, ctx, num_regs, k, s);
            local_get(code, hdr);
            local_set(code, dst as u32);
            Ok(())
        }
        _ => Err(WasmLowerError::Unsupported(
            "deep clone of an unsupported value kind",
        )),
    }
}

/// The kinds [`emit_collection_clone`] copies.
fn is_collection_clonable(k: Kind) -> bool {
    matches!(
        k,
        Kind::Text
            | Kind::SeqInt
            | Kind::SeqBool
            | Kind::SeqFloat
            | Kind::SeqWord32
            | Kind::SeqWord64
            | Kind::SeqAny
            | Kind::Set
            | Kind::SetText
            | Kind::Map
            | Kind::SeqSeqInt
            | Kind::SeqText
            | Kind::SeqStruct
            | Kind::SeqEnum
            | Kind::SeqHandle
    )
}

/// A fresh, independent copy of the `k`-kinded value whose handle is in local `src`, left in local
/// `num_regs + 5`. Scratch: `+5..+7`, and `+8..+11` for a `Seq of Seq`; `src` must be none of them
/// but `+8`, which the copy only reuses once it has finished reading `src`.
fn emit_collection_clone(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, k: Kind, src: u32) {
    let (hdr, data, idx) = (num_regs + 5, num_regs + 6, num_regs + 7);
    if let Some(mode) = seq_clone_mode(k) {
        if let Some(clone) = ctx.runtime.and_then(|r| r.index(RuntimeFn::SeqClone)) {
            local_get(code, src);
            i32_const(code, mode);
            code.push(0x10);
            leb_u32(code, clone);
            local_set(code, hdr);
            return;
        }
    }
    match k {
        // Text → fresh byte buffer; a sequence or `Set of Int` → fresh 8-byte-element buffer (a Set
        // is a flat scalar buffer like `SeqInt`, so the same copy is an independent deep clone).
        // A `SetText` clones like a `Set` (flat 8-byte-slot copy) — the shared `Text` element handles
        // are immutable in Logos, so the clone sharing them is sound (no deeper recursion needed).
        Kind::Text
        | Kind::SeqInt
        | Kind::SeqBool
        | Kind::SeqFloat
        | Kind::SeqWord32
        | Kind::SeqWord64
        | Kind::SeqAny
        | Kind::Set
        | Kind::SetText => {
            let is_text = k == Kind::Text;
            let class = if is_text {
                AllocClass::TextHeader
            } else {
                AllocClass::of_elements(matches!(k, Kind::SeqAny | Kind::SetText))
            };
            emit_buffer_clone(code, ctx, hdr, data, idx, src, is_text, class);
            if class == AllocClass::HandleHeader {
                emit_retain_children(code, ctx, hdr);
            }
        }
        // A Map — its `[key][value]` entry buffer flat-copied into a fresh, independent map.
        Kind::Map => emit_map_clone(code, ctx, hdr, data, idx, src),
        // A Seq of Seq (an Int matrix): clone the outer handle buffer, then clone EACH inner sequence
        // so the rows are independent (a flat copy would share the row handles). `idx` is free again
        // after the outer copy, so it doubles as the per-row loop counter; `+8..+11` clone each row.
        Kind::SeqSeqInt => {
            // The outer buffer holds one `Seq of Int` HANDLE per row; each row itself holds scalars.
            emit_buffer_clone(
                code,
                ctx,
                hdr,
                data,
                idx,
                src,
                false,
                AllocClass::HandleHeader,
            );
            local_get(code, hdr);
            i32_load(code, 8);
            local_set(code, data); // the copy's row handles, each replaced by a copy of its row
            let (isrc, ihdr, idata, iidx) =
                (num_regs + 8, num_regs + 9, num_regs + 10, num_regs + 11);
            emit_clone_each_element(
                code,
                ctx,
                hdr,
                data,
                idx,
                isrc,
                ihdr,
                idata,
                iidx,
                false,
                AllocClass::ScalarHeader,
            );
        }
        // A list of Texts, records, variants or collections: a flat copy of the element handles, each
        // element now held by both lists. A record or a collection gains a copy-on-write hold, so a
        // write to it through either list copies it first; a Text is immutable and a variant is
        // rebuilt, never written in place, so theirs are shared as they are.
        Kind::SeqText | Kind::SeqStruct | Kind::SeqEnum | Kind::SeqHandle => {
            emit_buffer_clone(
                code,
                ctx,
                hdr,
                data,
                idx,
                src,
                false,
                AllocClass::HandleHeader,
            );
            emit_retain_children(code, ctx, hdr);
            if matches!(k, Kind::SeqStruct | Kind::SeqHandle) {
                emit_cow_retain_each_element(code, hdr, idx);
            }
        }
        _ => unreachable!("emit_collection_clone of {k:?}: callers check is_collection_clonable"),
    }
}

/// `lhs equals rhs` / `lhs is not rhs` on two `Text` values — byte equality (unequal length ⇒ not
/// equal, else compare bytes). Result is a `Bool` i64 0/1 in `dst` (`negate` flips it for `!=`).
fn lower_text_eq(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    lhs: u16,
    rhs: u16,
    negate: bool,
) {
    emit_text_eq(code, ctx, num_regs, lhs as u32, rhs as u32);
    if negate {
        code.push(0x45); // i32.eqz
    }
    code.push(0xAD); // i64.extend_i32_u
    local_set(code, dst as u32);
}

/// Materialize the value in local `src` (of kind `kind`) as a `Text` handle stored in local `out`.
/// A `Text` is itself (just copied); an `Int`/`Float`/`Bool` is formatted into a fresh buffer by
/// the matching host formatter (which returns the byte length) and wrapped in a header. Uses the
/// +5/+6/+7 scratch as temps. Value-based (a local + kind, not a register) so it serves both
/// `Concat`'s operands and `Show`'s slot-loaded struct fields.
fn emit_stringify(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    src: u32,
    kind: Option<Kind>,
    out: u32,
) -> R<()> {
    if kind == Some(Kind::SeqInt) && ctx.exact.sequence_ints() {
        return exact::sequence_text(code, ctx, num_regs, src, out);
    }
    match kind {
        Some(Kind::Text) => {
            local_get(code, src);
            local_set(code, out);
        }
        // A value of the exact extension joins text as it prints.
        Some(Kind::Tower) => tower::text_of(code, ctx, src, out)?,
        // A `BigInt` operand of a concat (`"x = " + (2^200)`): render it to a decimal `Text` via the
        // runtime, then stringify AS that Text — matching the VM, which appends the decimal to the text.
        Some(Kind::BigInt) => {
            let to_text = (ctx.host_index)(HostFn::BigintToText)
                .ok_or(WasmLowerError::Unsupported("bigint_to_text not imported"))?;
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
                _ => (HostFn::FmtBoolInto, 8),            // "true"/"false"
            };
            let (h, data, tmp) = (num_regs + 5, num_regs + 6, num_regs + 7);
            i32_const(code, 16);
            emit_alloc(code, ctx, h, AllocClass::TextHeader);
            i32_const(code, bufsize);
            emit_alloc(code, ctx, data, AllocClass::Raw);
            // len = fmt_*_into(data, src)
            let fidx = (ctx.host_index)(host)
                .ok_or(WasmLowerError::Unsupported("text formatter not imported"))?;
            local_get(code, data);
            local_get(code, src);
            code.push(0x10); // call
            leb_u32(code, fidx);
            local_set(code, tmp); // tmp = len
                                  // header: len = tmp; cap = the buffer's size (what a free returns); data_ptr = data
            local_get(code, h);
            local_get(code, tmp);
            i32_store(code, 0);
            local_get(code, h);
            i32_const(code, bufsize);
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
            emit_alloc(code, ctx, h, AllocClass::TextHeader);
            // data = alloc(len*24 + 8)
            local_get(code, src);
            i32_load(code, 0); // len
            i32_const(code, 24);
            code.push(0x6C); // i32.mul
            i32_const(code, 8);
            code.push(0x6A); // + 8
            emit_alloc(code, ctx, data, AllocClass::Raw);
            // len = fmt_{seq,set}_i64_into(data, src)
            let fidx = (ctx.host_index)(host).ok_or(WasmLowerError::Unsupported(
                "collection formatter not imported",
            ))?;
            local_get(code, data);
            local_get(code, src);
            code.push(0x10); // call
            leb_u32(code, fidx);
            local_set(code, tmp);
            local_get(code, h);
            local_get(code, tmp);
            i32_store(code, 0);
            local_get(code, h);
            local_get(code, src);
            i32_load(code, 0);
            i32_const(code, 24);
            code.push(0x6C);
            i32_const(code, 8);
            code.push(0x6A); // cap = the buffer's size, len*24 + 8
            i32_store(code, 4);
            local_get(code, h);
            local_get(code, data);
            i32_store(code, 8);
            local_get(code, h);
            local_set(code, out);
        }
        _ => {
            return Err(WasmLowerError::Unsupported(
                "concat operand kind cannot be stringified yet",
            ))
        }
    }
    Ok(())
}

/// Build the display `Text` of the struct in `handle` (type `def`) into `out` — `TypeName { f: v, … }`
/// (`RuntimeValue::Struct::to_display_string`) with the fields in DETERMINISTIC alphabetical order (the
/// VM sorts its `HashMap` fields by name; this sorts the DECLARED fields the same way). Each field's
/// value is loaded from its declared slot (`data_ptr + slot*8`) at the field type's width and
/// stringified. An empty struct is just its name. `part`/`field_i32` are caller-supplied scratch kept
/// distinct from `out`. The reusable core of [`lower_show_struct`] and [`lower_show_seqstruct`].
fn emit_struct_display(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    def: &StructTypeDef,
    handle: u32,
    out: u32,
    part: u32,
    field_i32: u32,
) -> R<()> {
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
        if ctx.exact.record_ints() && ek == Some(Kind::Int) {
            exact::record_integer_text(code, ctx, num_regs, handle, i as u16, part)?;
        } else if let Some(shown) =
            display::Shown::of_type(bt).filter(|s| s.inline_kind().is_none())
        {
            local_get(code, handle);
            i32_load(code, 8);
            i32_load(code, (i as u32) * 8);
            local_set(code, field_i32);
            display::emit_shown(code, ctx, num_regs, field_i32, &shown, part)?;
        } else {
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
        }
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
        .or_else(|| match plan.structs.reg_declared.get(&src) {
            Some(BoundaryType::Struct(name)) => Some(name),
            _ => None,
        })
        .ok_or(WasmLowerError::Unsupported(
            "Show of a struct whose type is not statically known",
        ))?;
    let def = ctx
        .struct_types
        .iter()
        .find(|s| &s.name == type_name)
        .ok_or(WasmLowerError::Unsupported(
            "Show of an unknown struct type",
        ))?;
    let print_idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported(
        "Show struct: print_text not imported",
    ))?;
    let num_regs = plan.num_regs;
    let out = num_regs + 8;
    emit_struct_display(
        code,
        ctx,
        num_regs,
        def,
        src as u32,
        out,
        num_regs + 9,
        num_regs + 10,
    )?;
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
    let type_name =
        plan.structs
            .seq_elem_struct_name
            .get(&src)
            .ok_or(WasmLowerError::Unsupported(
                "Show of a Seq of Struct whose element type is not statically known",
            ))?;
    let def = ctx
        .struct_types
        .iter()
        .find(|s| &s.name == type_name)
        .ok_or(WasmLowerError::Unsupported(
            "Show seq-struct: unknown element struct type",
        ))?;
    let print_idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported(
        "Show seq-struct: print_text not imported",
    ))?;
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
    let print_idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported(
        "Show seq-of-seq: print_text not imported",
    ))?;
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
/// entries in STORED order: INSERTION order for a `Map` (the VM's `MapStorage` is an `IndexMap`, the
/// order the linear map appends in) and key order for a `SharedMap` (kept sorted on insert), so the
/// rendering is byte-identical to the reference tier's. A RUNTIME loop over `0..num_entries`, each
/// 16-byte entry `[key@0][value@8]` stringified by its kinds and joined as `k: v`. The kinds come
/// from the last `SetIndex`'s registers in this region, or from the map's declared type.
///
/// A map with neither is a locally-declared map nothing ever wrote — every entry is created by a
/// `SetIndex` — so it holds zero entries and displays as `{}`, the VM's empty `IndexMap`. A declared
/// map type whose key kind this backend cannot represent is refused.
/// What a whole-map `Show` of `src` renders from — one answer for the import scan and the lowering.
enum MapShowKinds {
    /// Entries of these key and value kinds.
    Entries(Option<Kind>, Option<Kind>),
    /// A locally-declared map nothing ever wrote: `{}`.
    Empty,
    /// A declared map whose key or value kind this backend cannot represent.
    Unknown,
}

/// The key and value kinds of the map in `src`: the registers of a `SetIndex` on it in this region,
/// or else its DECLARED type (a parameter, a global, a record's field, a call result).
fn map_show_kinds(plan: &Plan, src: u16) -> MapShowKinds {
    match plan.structs.map_set_key.get(&src) {
        Some(&key_reg) => match plan.structs.map_set_value.get(&src) {
            Some(&val_reg) => MapShowKinds::Entries(
                plan.kinds.get(key_reg as usize),
                plan.kinds.get(val_reg as usize),
            ),
            None => MapShowKinds::Unknown,
        },
        None => match plan.structs.reg_shape.get(&src) {
            Some(ParamShape::Map {
                key: Some(k),
                value,
                ..
            }) => MapShowKinds::Entries(Some(*k), Some(*value)),
            Some(ParamShape::Map { key: None, .. }) => MapShowKinds::Unknown,
            _ => MapShowKinds::Empty,
        },
    }
}

fn lower_show_map(
    code: &mut Vec<u8>,
    plan: &Plan,
    _kinds: &KindTable,
    ctx: &Ctx,
    src: u16,
) -> R<()> {
    let (key_kind, val_kind) = match map_show_kinds(plan, src) {
        MapShowKinds::Entries(k, v) => (k, v),
        MapShowKinds::Empty => return lower_show_empty_map(code, plan, ctx),
        MapShowKinds::Unknown => {
            return Err(WasmLowerError::Unsupported(
                "Show of a map whose key or value kind is not statically known",
            ))
        }
    };
    let key_text = match key_kind {
        Some(Kind::Text) => true,
        Some(Kind::Int) => false,
        _ => {
            return Err(WasmLowerError::Unsupported(
                "Show of a map with a non-Int/Text key",
            ))
        }
    };
    let val_load = map_value_load(val_kind)?; // the value's slot load at its kind's width
    let print_idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported(
        "Show map: print_text not imported",
    ))?;
    let num_regs = plan.num_regs;
    let m = src as u32;
    // acc = accumulator Text; part = each stringified piece; i = the entry loop counter. Key/value
    // temps borrow width-matched scratch (idle during a Show): the key is fully stringified before the
    // value is loaded, so they may share a slot.
    let (acc, part, i) = (num_regs + 8, num_regs + 9, num_regs + 10);
    let key_tmp = if key_text {
        num_regs + 11
    } else {
        num_regs + 1
    };
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
    if !key_text && ctx.exact.map_ints {
        exact::map_integer_text(code, ctx, num_regs, m, 0, part)?;
    } else {
        if key_text {
            i32_load(code, 0);
        } else {
            i64_load(code, 0);
        }
        local_set(code, key_tmp);
        emit_stringify(code, ctx, num_regs, key_tmp, key_kind, part)?;
    }
    emit_text_concat(code, ctx, num_regs, acc, part, acc);
    // ": "
    lower_text_literal(code, ctx, num_regs, b": ");
    local_set(code, part);
    emit_text_concat(code, ctx, num_regs, acc, part, acc);
    // value = entry[i].value (offset 8), stringified, appended
    emit_map_entry_addr(code, m, i);
    if val_kind == Some(Kind::Int) && ctx.exact.map_ints {
        exact::map_integer_text(code, ctx, num_regs, m, 8, part)?;
    } else {
        val_load(code, 8);
        local_set(code, val_tmp);
        emit_stringify(code, ctx, num_regs, val_tmp, val_kind, part)?;
    }
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

/// `Show <empty map>` — a locally-declared map that never received a `SetIndex` holds zero entries,
/// so its display is the fixed `{}` (the VM's rendering of an empty `IndexMap`), emitted directly as a
/// `Text` literal without an entry loop (no key/value kind is needed because none is ever stringified).
fn lower_show_empty_map(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx) -> R<()> {
    let print_idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported(
        "Show map: print_text not imported",
    ))?;
    lower_text_literal(code, ctx, plan.num_regs, b"{}");
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
        .ok_or(WasmLowerError::Unsupported(
            "Show of a tuple with no static layout",
        ))?;
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
        if ctx.exact.sequence_ints() && plan.kinds.get(src as usize) == Some(Kind::SeqInt) {
            local_get(code, elem_tmp);
            code.push(0x10);
            leb_u32(
                code,
                (ctx.host_index)(HostFn::ExactText)
                    .ok_or(WasmLowerError::Unsupported("exact text import"))?,
            );
            local_set(code, part);
        } else {
            emit_stringify(code, ctx, num_regs, elem_tmp, ek, part)?;
        }
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
    let idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported(
        "Show tuple: print_text not imported",
    ))?;
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
fn emit_enum_display(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    def: &EnumTypeDef,
    handle: u32,
    out: u32,
    part: u32,
    field_i32: u32,
) -> R<()> {
    for v in &def.variants {
        // The variant's tag = the constant-pool index of its name `Text` (constant dedup makes this the
        // exact value `NewInductive` stored). A variant NEVER constructed anywhere is absent from the
        // pool — it can't be the runtime value, so it needs no branch.
        let Some(tag) = ctx
            .constants
            .iter()
            .position(|c| matches!(c, Constant::Text(n) if *n == v.name))
        else {
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
            // Append each payload value using this module's enum layout.
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
                emit_enum_payload_base(code, ctx);
                emit_slot_load(code, ek, enum_payload_offset(ctx, i as u32))?;
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
        .ok_or(WasmLowerError::Unsupported(
            "Show of an enum whose type is not statically known",
        ))?;
    let def = ctx
        .enum_types
        .iter()
        .find(|e| &e.name == type_name)
        .ok_or(WasmLowerError::Unsupported("Show of an unknown enum type"))?;
    let print_idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported(
        "Show enum: print_text not imported",
    ))?;
    let num_regs = plan.num_regs;
    // out = the assembled display; +9/+10 are the piece + field-i32 scratch (the classic `Show` pool).
    let out = num_regs + 8;
    emit_enum_display(
        code,
        ctx,
        num_regs,
        def,
        src as u32,
        out,
        num_regs + 9,
        num_regs + 10,
    )?;
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
        .ok_or(WasmLowerError::Unsupported(
            "Show of a Seq of Enum whose element type is not statically known",
        ))?;
    let def =
        ctx.enum_types
            .iter()
            .find(|e| &e.name == type_name)
            .ok_or(WasmLowerError::Unsupported(
                "Show seq-enum: unknown element enum type",
            ))?;
    let print_idx = (ctx.host_index)(HostFn::PrintText).ok_or(WasmLowerError::Unsupported(
        "Show seq-enum: print_text not imported",
    ))?;
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
fn policy_field_slot<'a>(
    ctx: &'a Ctx,
    type_name: &str,
    field_name: &str,
) -> Option<(u16, &'a BoundaryType)> {
    let def = ctx.struct_types.iter().find(|s| s.name == type_name)?;
    def.fields
        .iter()
        .position(|(n, _)| n == field_name)
        .map(|i| (i as u16, &def.fields[i].1))
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
fn emit_policy_condition(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    cond: &PolicyCondition,
    subject: u16,
    object: u16,
) -> R<()> {
    let (f8, f11) = (plan.num_regs + 8, plan.num_regs + 11);
    match cond {
        PolicyCondition::FieldEquals {
            field,
            value,
            is_string_literal,
        } => {
            if !is_string_literal {
                return Err(WasmLowerError::Unsupported(
                    "policy: non-Text field comparison",
                ));
            }
            let subj_type =
                plan.structs
                    .struct_name_of
                    .get(&subject)
                    .ok_or(WasmLowerError::Unsupported(
                        "policy: subject has no struct type",
                    ))?;
            let field_name = ctx.interner.resolve(*field);
            let (slot, bt) = policy_field_slot(ctx, subj_type, field_name).ok_or(
                WasmLowerError::Unsupported("policy: field not in subject type"),
            )?;
            if !matches!(bt, BoundaryType::Text) {
                return Err(WasmLowerError::Unsupported("policy: field is not Text"));
            }
            let value_str = ctx.interner.resolve(*value).to_string();
            emit_load_text_field(code, subject, slot, f8);
            lower_text_literal(code, ctx, plan.num_regs, value_str.as_bytes());
            local_set(code, f11);
            emit_text_eq(code, ctx, plan.num_regs, f8, f11);
        }
        PolicyCondition::Predicate { predicate, .. } => {
            let subj_type =
                plan.structs
                    .struct_name_of
                    .get(&subject)
                    .ok_or(WasmLowerError::Unsupported(
                        "policy: subject has no struct type",
                    ))?;
            let subj_sym = ctx
                .interner
                .lookup(subj_type)
                .ok_or(WasmLowerError::Unsupported(
                    "policy: subject type not interned",
                ))?;
            let preds =
                ctx.policies
                    .get_predicates(subj_sym)
                    .ok_or(WasmLowerError::Unsupported(
                        "policy: no predicates for subject type",
                    ))?;
            let pred = preds
                .iter()
                .find(|p| p.predicate_name == *predicate)
                .ok_or(WasmLowerError::Unsupported(
                    "policy: referenced predicate not found",
                ))?;
            emit_policy_condition(code, plan, ctx, &pred.condition, subject, object)?;
        }
        PolicyCondition::SubjectFieldEqualsObjectField {
            subject_field,
            object_field,
            ..
        }
        | PolicyCondition::ObjectFieldEquals {
            subject: subject_field,
            field: object_field,
            ..
        } => {
            if object == u16::MAX {
                return Err(WasmLowerError::Unsupported(
                    "policy: cross-field compare needs an object",
                ));
            }
            let subj_type =
                plan.structs
                    .struct_name_of
                    .get(&subject)
                    .ok_or(WasmLowerError::Unsupported(
                        "policy: subject has no struct type",
                    ))?;
            let obj_type =
                plan.structs
                    .struct_name_of
                    .get(&object)
                    .ok_or(WasmLowerError::Unsupported(
                        "policy: object has no struct type",
                    ))?;
            let sf = ctx.interner.resolve(*subject_field);
            let of = ctx.interner.resolve(*object_field);
            let (s_slot, s_bt) = policy_field_slot(ctx, subj_type, sf).ok_or(
                WasmLowerError::Unsupported("policy: subject field not found"),
            )?;
            let (o_slot, o_bt) = policy_field_slot(ctx, obj_type, of).ok_or(
                WasmLowerError::Unsupported("policy: object field not found"),
            )?;
            if !matches!(s_bt, BoundaryType::Text) || !matches!(o_bt, BoundaryType::Text) {
                return Err(WasmLowerError::Unsupported(
                    "policy: cross-field compare of non-Text fields",
                ));
            }
            emit_load_text_field(code, subject, s_slot, f8);
            emit_load_text_field(code, object, o_slot, f11);
            emit_text_eq(code, ctx, plan.num_regs, f8, f11);
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
            return Err(WasmLowerError::Unsupported(
                "policy: boolean field condition",
            ));
        }
    }
    Ok(())
}

/// `Check that <subject> is <predicate>` / `… can <action> <object>` (`CheckPolicy`) — resolve the
/// predicate/capability's condition from the `## Policy` registry, compile it inline, and TRAP
/// (`unreachable`) when it is false (the standalone module's analog of the VM's `check_policy` error);
/// when it holds, execution falls through to the following statements. Mirrors the VM semantics.
fn lower_check_policy(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    subject: u16,
    predicate: crate::Symbol,
    is_capability: bool,
    object: u16,
) -> R<()> {
    let subj_type =
        plan.structs
            .struct_name_of
            .get(&subject)
            .ok_or(WasmLowerError::Unsupported(
                "CheckPolicy on a non-struct subject",
            ))?;
    let subj_sym = ctx
        .interner
        .lookup(subj_type)
        .ok_or(WasmLowerError::Unsupported(
            "CheckPolicy subject type not interned",
        ))?;
    let cond = if is_capability {
        let caps = ctx
            .policies
            .get_capabilities(subj_sym)
            .ok_or(WasmLowerError::Unsupported(
                "CheckPolicy: no capabilities for subject type",
            ))?;
        caps.iter()
            .find(|c| c.action == predicate)
            .map(|c| &c.condition)
            .ok_or(WasmLowerError::Unsupported(
                "CheckPolicy: capability not found",
            ))?
    } else {
        let preds = ctx
            .policies
            .get_predicates(subj_sym)
            .ok_or(WasmLowerError::Unsupported(
                "CheckPolicy: no predicates for subject type",
            ))?;
        preds
            .iter()
            .find(|p| p.predicate_name == predicate)
            .map(|p| &p.condition)
            .ok_or(WasmLowerError::Unsupported(
                "CheckPolicy: predicate not found",
            ))?
    };
    emit_policy_condition(code, plan, ctx, cond, subject, object)?;
    code.push(0x45); // i32.eqz → condition is FALSE
    code.push(0x04);
    code.push(0x40); // if (failed)
    code.push(0x00); // unreachable — the check failed (the VM errors here)
    code.push(0x0B); // end if
    Ok(())
}

/// The byte stride of one per-replica counter record: `[replica_id:i64 @0][inc:i64 @8][dec:i64 @16]`.
/// A `ConvergentCount` (G-Counter) never touches `dec`; a `Tally` (PN-counter) uses both.
const CRDT_RECORD_BYTES: i32 = 24;

/// Allocate a counted counter header over a scalar replica buffer.
/// A monotonic identity stays distinct after the allocator reuses a dead counter's storage,
/// including while another counter still retains the old replica's merged contribution.
fn emit_new_counter(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u32) {
    let (hdr, data) = (num_regs + 5, num_regs + 6); // i32 heap scratch
                                                    // hdr = alloc(16); data = alloc(24)
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr, AllocClass::CounterHeader);
    i32_const(code, CRDT_RECORD_BYTES);
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
    // An address may be reused after collection; an identity retained in a merged replica may not.
    let replica = ctx.replica_global.expect("counter identity global");
    global_get(code, replica);
    i64c(code, 1);
    code.push(0x7C);
    global_set(code, replica);
    global_get(code, replica);
    code.push(0x50); // i64.eqz: do not wrap the identity space
    code.push(0x04);
    code.push(0x40);
    code.push(0x00);
    code.push(0x0B);
    local_get(code, data);
    global_get(code, replica);
    i64_store(code, 0);
    local_get(code, data);
    code.push(0x42); // i64.const 0
    leb_i64(code, 0);
    i64_store(code, 8); // inc
    local_get(code, data);
    code.push(0x42);
    leb_i64(code, 0);
    i64_store(code, 16); // dec
    local_get(code, hdr);
    local_set(code, dst);
}

/// `obj's <counter-field>` — project the per-replica counter OBJECT stored in the field slot to its
/// Int total `Σinc − Σdec`, into `dst` (an `i64` local). Loads the counter handle from the field's
/// slot, then a loop accumulates each record's `inc` and subtracts its `dec`. The single-replica case
/// runs the body once (`inc − dec`), matching the tree-walker's counter value byte-for-byte.
fn lower_get_counter_total(
    code: &mut Vec<u8>,
    plan: &Plan,
    slot: u16,
    dst: u16,
    obj: u16,
) -> R<()> {
    if plan.kinds.get(obj as usize) != Some(Kind::Struct) {
        return Err(WasmLowerError::Unsupported(
            "counter field access on a value with no known struct layout",
        ));
    }
    let (cnt, acc, i) = (plan.num_regs + 17, plan.num_regs + 15, plan.num_regs + 18); // cnt/i i32, acc i64
    let off = u32::from(slot) * 8;
    // cnt = obj.data_ptr[slot]  (the counter object handle)
    local_get(code, obj as u32);
    i32_load(code, 8);
    i32_load(code, off);
    local_set(code, cnt);
    // acc = 0; i = 0
    code.push(0x42);
    leb_i64(code, 0);
    local_set(code, acc);
    i32_const(code, 0);
    local_set(code, i);
    code.push(0x02);
    code.push(0x40); // block $exit
    code.push(0x03);
    code.push(0x40); // loop $loop
                     // if i >= len break
    local_get(code, i);
    local_get(code, cnt);
    i32_load(code, 0); // len
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if $exit
                      // rec = data_ptr + i * 24
                      // acc += rec.inc - rec.dec
    local_get(code, acc);
    // rec base address on the stack twice (for inc and dec loads)
    local_get(code, cnt);
    i32_load(code, 8); // data_ptr
    local_get(code, i);
    i32_const(code, CRDT_RECORD_BYTES);
    code.push(0x6C); // i32.mul
    code.push(0x6A); // data_ptr + i*24 = rec
    local_tee(code, plan.num_regs + 19); // rec cursor (i32 scratch)
    i64_load(code, 8); // inc
    code.push(0x7C); // acc + inc
    local_get(code, plan.num_regs + 19);
    i64_load(code, 16); // dec
    code.push(0x7D); // (acc+inc) - dec
    local_set(code, acc);
    // i += 1
    local_get(code, i);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, i);
    code.push(0x0C);
    leb_u32(code, 0); // br $loop
    code.push(0x0B); // end loop
    code.push(0x0B); // end block
    local_get(code, acc);
    local_set(code, dst as u32);
    Ok(())
}

/// `Increase/Decrease <obj>'s <field> by <amount>` (`CrdtBump`) on a `Shared` struct's
/// `ConvergentCount`/`Tally` field. The field holds a per-replica counter OBJECT; this bumps THIS
/// replica's own slot (record 0, allocated by `emit_new_counter`) — `Increase` adds to its `inc`,
/// `Decrease` adds to its `dec` (a `Tally`; a G-Counter never emits a decrement, rejected upstream).
/// The per-replica accounting is what makes a later `Merge` idempotent — the same remote state
/// re-merged MAXes equal values and never double-counts.
fn lower_crdt_bump(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    obj: u16,
    field_const: u32,
    amount: u16,
    negate: bool,
) -> R<()> {
    let _ = ctx;
    // The counter's slot = its position in the struct's field layout (matched by the field-name const
    // index the op carries). `reg_layout` is populated from the actual `NewStruct`/`StructInsert` ops,
    // so it covers a `Shared` struct too (whose type may not be in the plain `struct_types`).
    let layout = plan
        .structs
        .reg_layout
        .get(&obj)
        .ok_or(WasmLowerError::Unsupported(
            "CrdtBump on a value with no struct layout",
        ))?;
    let slot =
        layout
            .iter()
            .position(|(fc, _)| *fc == field_const)
            .ok_or(WasmLowerError::Unsupported(
                "CrdtBump: field not in struct layout",
            ))? as u32;
    let off = slot * 8;
    // `Increase` accumulates into record 0's `inc`, `Decrease` into its `dec` (offset 8 vs 16).
    let rec_off = if negate { 16 } else { 8 };
    let rec = plan.num_regs + 17; // i32 scratch: the counter object's record-0 base (== data_ptr)
                                  // rec = obj.data_ptr[slot].data_ptr   (record 0 is at data_ptr + 0)
    local_get(code, obj as u32);
    i32_load(code, 8);
    i32_load(code, off); // the counter object handle
    i32_load(code, 8); // its data_ptr
    local_set(code, rec);
    // rec[rec_off] += amount
    local_get(code, rec);
    local_get(code, rec);
    i64_load(code, rec_off);
    local_get(code, amount as u32);
    code.push(0x7C); // i64.add (amount is a non-negative magnitude for both Increase and Decrease)
    i64_store(code, rec_off);
    Ok(())
}

/// `Merge <source> into <target>` (`CrdtMerge`) of two same-typed `Shared` structs whose fields are
/// `ConvergentCount`/`Tally` counters. Each counter field is a per-replica counter OBJECT; the merge is
/// the state-based LATTICE JOIN — per-replica MAX on both `inc` and `dec`. IDEMPOTENT: a redelivered
/// source replica finds its record already present in the target and MAXes equal values, so it never
/// double-counts. Mirrors `logicaffeine_data::crdt::{GCounter, PNCounter}::merge` (the interpreter /
/// register VM / native AOT hold those exact types). A field that is NOT a counter CRDT needs a
/// different merge object and is soundly refused.
fn lower_crdt_merge(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, target: u16, source: u16) -> R<()> {
    let layout = plan
        .structs
        .reg_layout
        .get(&target)
        .ok_or(WasmLowerError::Unsupported(
            "CrdtMerge on a value with no struct layout",
        ))?;
    let fields = layout.clone();
    for (slot, &(_fc, value_reg)) in fields.iter().enumerate() {
        let field_kind = value_reg.and_then(|r| plan.kinds.get(r as usize));
        if field_kind != Some(Kind::CrdtCounter) {
            // Name the author's own construct, not this backend's internal op — the same
            // convention every other backend's CRDT refusal already uses
            // (`crate::semantics::crdt::CrdtKind::refusal`), just for MERGING specifically: wasm
            // does lower a single-replica `SharedMap`/`SharedSet`/`SharedSequence` (this field's
            // OWN read/write ops), it merges none of them.
            let kind_name = match field_kind {
                Some(Kind::Map) => "SharedMap",
                Some(Kind::SetText) | Some(Kind::CrdtSetText) => "SharedSet",
                Some(Kind::SeqSeqInt) | Some(Kind::SeqInt) => "SharedSequence",
                _ => "a CRDT",
            };
            return Err(WasmLowerError::UnsupportedOwned(format!(
                "`{kind_name}` is a CRDT the wasm backend does not implement merging for — run \
                 the program on the interpreter or the VM, or build it with the Rust tier \
                 (`largo build`)"
            )));
        }
        emit_counter_field_merge(code, plan, ctx, target, source, (slot as u32) * 8);
    }
    Ok(())
}

/// Per-replica MAX merge of ONE counter field: for each SOURCE record, find the matching replica in
/// the TARGET (by replica id) and MAX its `inc`/`dec`; if the replica is new, append the record. This
/// is the join that makes the counter idempotent. Scratch: `+17` target counter handle, `+18` source
/// counter handle, `+19` outer (source) index, `+20` source record base, `+15` source replica id,
/// `+21` matched target record base (0 = not found), `+22` inner (target) index.
fn emit_counter_field_merge(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    target: u16,
    source: u16,
    off: u32,
) {
    let nr = plan.num_regs;
    let (t_cnt, s_cnt) = (nr + 17, nr + 18); // target / source counter object handles (i32)
    let (j, s_rec) = (nr + 19, nr + 20); // source index (i32), source record base (i32)
    let s_id = nr + 15; // source replica id (i64)
    let (found, k) = (nr + 21, nr + 22); // matched target record base (i32, 0=none), target index (i32)
                                         // t_cnt = target.data_ptr[slot]; s_cnt = source.data_ptr[slot]
    local_get(code, target as u32);
    i32_load(code, 8);
    i32_load(code, off);
    local_set(code, t_cnt);
    local_get(code, source as u32);
    i32_load(code, 8);
    i32_load(code, off);
    local_set(code, s_cnt);
    // for j in 0..s_cnt.len
    i32_const(code, 0);
    local_set(code, j);
    code.push(0x02);
    code.push(0x40); // block $outer_exit
    code.push(0x03);
    code.push(0x40); // loop $outer
    local_get(code, j);
    local_get(code, s_cnt);
    i32_load(code, 0); // s len
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if $outer_exit
                      // s_rec = s_cnt.data_ptr + j*24; s_id = s_rec.id
    local_get(code, s_cnt);
    i32_load(code, 8);
    local_get(code, j);
    i32_const(code, CRDT_RECORD_BYTES);
    code.push(0x6C);
    code.push(0x6A);
    local_tee(code, s_rec);
    i64_load(code, 0);
    local_set(code, s_id);
    // found = 0; for k in 0..t_cnt.len: if t_rec.id == s_id { found = t_rec; break }
    i32_const(code, 0);
    local_set(code, found);
    i32_const(code, 0);
    local_set(code, k);
    code.push(0x02);
    code.push(0x40); // block $inner_exit
    code.push(0x03);
    code.push(0x40); // loop $inner
    local_get(code, k);
    local_get(code, t_cnt);
    i32_load(code, 0); // t len
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1); // br_if $inner_exit
                      // t_rec = t_cnt.data_ptr + k*24  (reuse `found` as the running candidate)
    local_get(code, t_cnt);
    i32_load(code, 8);
    local_get(code, k);
    i32_const(code, CRDT_RECORD_BYTES);
    code.push(0x6C);
    code.push(0x6A);
    local_tee(code, found);
    // if t_rec.id == s_id break (found holds the match)
    i64_load(code, 0);
    local_get(code, s_id);
    code.push(0x51); // i64.eq
    code.push(0x04);
    code.push(0x40); // if
    code.push(0x0C);
    leb_u32(code, 2); // br $inner_exit (out of the if, then the inner loop block)
    code.push(0x0B); // end if
                     // not a match this iteration → clear `found` so a fall-through past the last record means "none"
    i32_const(code, 0);
    local_set(code, found);
    local_get(code, k);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, k);
    code.push(0x0C);
    leb_u32(code, 0); // br $inner
    code.push(0x0B); // end $inner loop
    code.push(0x0B); // end $inner_exit block
                     // if found != 0 { MAX inc + MAX dec } else { append (s_id, s_inc, s_dec) to target }
    local_get(code, found);
    code.push(0x45); // i32.eqz → found == 0
    code.push(0x04);
    code.push(0x40); // if (append path)
    emit_counter_append(code, plan, ctx, t_cnt, s_rec, s_id);
    code.push(0x05); // else (MAX path)
    emit_counter_record_max(code, found, s_rec, 8); // inc
    emit_counter_record_max(code, found, s_rec, 16); // dec
    code.push(0x0B); // end if
                     // j += 1
    local_get(code, j);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, j);
    code.push(0x0C);
    leb_u32(code, 0); // br $outer
    code.push(0x0B); // end $outer loop
    code.push(0x0B); // end $outer_exit block
}

/// `target[off] = max(target[off], source[off])` on an i64 record field (signed). No wasm i64.max, so
/// `select` on `i64.gt_s`.
fn emit_counter_record_max(code: &mut Vec<u8>, t_rec: u32, s_rec: u32, off: u32) {
    local_get(code, t_rec);
    // max(t, s) = t > s ? t : s
    local_get(code, t_rec);
    i64_load(code, off);
    local_get(code, s_rec);
    i64_load(code, off);
    local_get(code, t_rec);
    i64_load(code, off);
    local_get(code, s_rec);
    i64_load(code, off);
    code.push(0x55); // i64.gt_s
    code.push(0x1B); // select → (t>s) ? t : s
    i64_store(code, off);
}

/// Append a fresh replica record `[s_id, s_rec.inc, s_rec.dec]` to the target counter object `t_cnt`,
/// growing its buffer geometrically and freeing the one growth abandons (mirrors `lower_list_push`,
/// but a 24-byte record stride). Reached only when a source replica is NOT already present in the
/// target — so the new state is genuinely new, and a later redelivery finds it present (idempotent).
fn emit_counter_append(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    t_cnt: u32,
    s_rec: u32,
    s_id: u32,
) {
    let nr = plan.num_regs;
    // `dst_rec` = nr+22 (the merge's `k`, free in the append branch); MUST avoid `s_rec` (nr+20) and
    // `s_id` (nr+15), which are read to fill the new record after the buffer may have grown.
    let (hs_new, hs_i, hs_cap, dst_rec) = (nr + 5, nr + 6, nr + 7, nr + 22);
    // if len < cap { in place } else { grow }
    local_get(code, t_cnt);
    i32_load(code, 0); // len
    local_get(code, t_cnt);
    i32_load(code, 4); // cap
    code.push(0x48); // i32.lt_s
    code.push(0x04);
    code.push(0x40); // if (in place)
    {
        // dst_rec = data_ptr + len*24
        local_get(code, t_cnt);
        i32_load(code, 8);
        local_get(code, t_cnt);
        i32_load(code, 0);
        i32_const(code, CRDT_RECORD_BYTES);
        code.push(0x6C);
        code.push(0x6A);
        local_set(code, dst_rec);
    }
    code.push(0x05); // else (grow)
    {
        // new_cap = cap == 0 ? 1 : cap*2
        i32_const(code, 1);
        local_get(code, t_cnt);
        i32_load(code, 4);
        i32_const(code, 2);
        code.push(0x6C);
        local_get(code, t_cnt);
        i32_load(code, 4);
        code.push(0x45); // i32.eqz → cap == 0
        code.push(0x1B); // select
        local_set(code, hs_cap);
        // new = alloc(new_cap * 24)
        local_get(code, hs_cap);
        i32_const(code, CRDT_RECORD_BYTES);
        code.push(0x6C);
        emit_alloc(code, ctx, hs_new, AllocClass::Raw);
        // copy old records: for i in 0..len*24 step 8 (word copy) — copy len*3 i64 words
        i32_const(code, 0);
        local_set(code, hs_i);
        code.push(0x02);
        code.push(0x40); // block $copy_exit
        code.push(0x03);
        code.push(0x40); // loop $copy
        local_get(code, hs_i);
        local_get(code, t_cnt);
        i32_load(code, 0);
        i32_const(code, CRDT_RECORD_BYTES);
        code.push(0x6C); // len*24 (byte count)
        code.push(0x4E); // i32.ge_s → i >= len*24
        code.push(0x0D);
        leb_u32(code, 1); // br_if $copy_exit
                          // new[i] = old[i]  (i64 word copy at byte offset hs_i)
        local_get(code, hs_new);
        local_get(code, hs_i);
        code.push(0x6A);
        local_get(code, t_cnt);
        i32_load(code, 8); // old data_ptr
        local_get(code, hs_i);
        code.push(0x6A);
        i64_load(code, 0);
        i64_store(code, 0);
        local_get(code, hs_i);
        i32_const(code, 8);
        code.push(0x6A);
        local_set(code, hs_i);
        code.push(0x0C);
        leb_u32(code, 0); // br $copy
        code.push(0x0B); // end $copy loop
        code.push(0x0B); // end $copy_exit block
                         // dst_rec = new + len*24
        local_get(code, hs_new);
        local_get(code, t_cnt);
        i32_load(code, 0);
        i32_const(code, CRDT_RECORD_BYTES);
        code.push(0x6C);
        code.push(0x6A);
        local_set(code, dst_rec);
        // The old record buffer is dead by construction: this counter object is the only holder
        // and is about to stop pointing at it — read before the header is repointed, exactly as
        // `lower_list_push_at` frees the buffer a sequence's growth abandons.
        emit_free(
            code,
            ctx,
            |c| {
                local_get(c, t_cnt);
                i32_load(c, 8); // the OLD data_ptr
            },
            |c| {
                local_get(c, t_cnt);
                i32_load(c, 4); // the OLD cap
                i32_const(c, CRDT_RECORD_BYTES);
                c.push(0x6C);
            },
        );
        // header: data_ptr = new, cap = new_cap
        local_get(code, t_cnt);
        local_get(code, hs_new);
        i32_store(code, 8);
        local_get(code, t_cnt);
        local_get(code, hs_cap);
        i32_store(code, 4);
    }
    code.push(0x0B); // end if
                     // write the new record: [s_id, s_rec.inc, s_rec.dec]
    local_get(code, dst_rec);
    local_get(code, s_id);
    i64_store(code, 0);
    local_get(code, dst_rec);
    local_get(code, s_rec);
    i64_load(code, 8);
    i64_store(code, 8);
    local_get(code, dst_rec);
    local_get(code, s_rec);
    i64_load(code, 16);
    i64_store(code, 16);
    // header: len += 1
    local_get(code, t_cnt);
    local_get(code, t_cnt);
    i32_load(code, 0);
    i32_const(code, 1);
    code.push(0x6A);
    i32_store(code, 0);
}

/// `Resolve <obj>'s <field> to <value>` (`CrdtResolve`) — a single-replica Divergent register just
/// takes the new value (the VM overwrites the field: `s.fields.insert(field, v)`). So this stores the
/// value handle into the field's slot (resolved from `reg_layout` by the field-name const), matching
/// a plain field write — byte-identical for one replica. (A merged multi-value register is deferred.)
fn lower_crdt_resolve(
    code: &mut Vec<u8>,
    plan: &Plan,
    kinds: &KindTable,
    obj: u16,
    field_const: u32,
    value: u16,
) -> R<()> {
    let layout = plan
        .structs
        .reg_layout
        .get(&obj)
        .ok_or(WasmLowerError::Unsupported(
            "CrdtResolve on a value with no struct layout",
        ))?;
    let slot =
        layout
            .iter()
            .position(|(fc, _)| *fc == field_const)
            .ok_or(WasmLowerError::Unsupported(
                "CrdtResolve: field not in struct layout",
            ))? as u32;
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
/// header update the aliasing field sees). An OR-Set append routes to the dedup set add, which keeps a
/// CRDT set sorted.
fn lower_crdt_append(
    code: &mut Vec<u8>,
    plan: &Plan,
    kinds: &KindTable,
    ctx: &Ctx,
    seq: u16,
    value: u16,
) -> R<()> {
    match kinds.get(seq as usize) {
        Some(Kind::SeqText) | Some(Kind::SeqInt) | Some(Kind::SeqBool) | Some(Kind::SeqFloat)
        | Some(Kind::SeqAny) => lower_list_push(code, kinds, ctx, plan.num_regs, seq, value),
        Some(Kind::SetText) | Some(Kind::CrdtSetText) => {
            emit_set_add_elem(
                code,
                ctx,
                plan.num_regs,
                seq as u32,
                value as u32,
                true,
                SetOrder::Sorted,
            );
            Ok(())
        }
        Some(Kind::Set) => {
            emit_set_add_elem(
                code,
                ctx,
                plan.num_regs,
                seq as u32,
                value as u32,
                false,
                SetOrder::Sorted,
            );
            Ok(())
        }
        _ => Err(WasmLowerError::Unsupported(
            "CrdtAppend to a non-collection CRDT",
        )),
    }
}

/// `Push src to obj's field` (`ListPushField`) — the direct field-seq push (`Push x to p's items`).
/// Resolve the field's slot (`reg_layout`, matched by the field-name const) and its element kind (from
/// the register that defined the field's value), load the field's seq handle, and push through the
/// shared amortized [`lower_list_push_at`]. The seq's header address is stable across the push, so the
/// struct's field slot keeps pointing at it (no write-back). COW `obj` first (value-semantic struct
/// mutation); the exercised programs own `obj` uniquely (a nested aliased field-seq is a COW frontier).
/// `Push src to obj's field` — onto the list in the field of the record `obj` holds, value
/// semantics on both levels: a record another value may share (`cow`, [`cow_struct_inserts`]) is
/// copied first, and a list another holder shares is copied into this record's field before the push.
#[allow(clippy::too_many_arguments)]
fn lower_list_push_field(
    code: &mut Vec<u8>,
    plan: &Plan,
    kinds: &KindTable,
    ctx: &Ctx,
    cow: bool,
    obj: u16,
    field_const: u32,
    src: u16,
) -> R<()> {
    let slot = record_field_slot(plan, obj, field_const).ok_or(WasmLowerError::Unsupported(
        "push to a field of a value with no known record layout",
    ))?;
    if cow {
        emit_record_copy(code, ctx, kinds, &plan.structs, plan.num_regs, obj, None)?;
    }
    // The element width comes from the PUSHED value's kind (the field seq may have been default-filled
    // empty, leaving its declared element kind unrefined) — an Int rides an i64 slot, a Text/handle an
    // i32 in the low word, all 8-byte slots.
    let elem = kinds.get(src as usize).ok_or(WasmLowerError::Unsupported(
        "ListPushField: unknown pushed-value kind",
    ))?;
    let off = (slot as u32) * 8;
    let handle = plan.num_regs + 8; // i32 scratch, distinct from lower_list_push_at's +5/+6/+7
                                    // handle = obj.data_ptr[slot] (the field seq's i32 handle)
    local_get(code, obj as u32);
    i32_load(code, 8);
    i32_load(code, off);
    local_set(code, handle);
    let field_kind =
        record_slot_kinds(kinds, &plan.structs, obj).and_then(|ks| ks.get(slot).copied().flatten());
    if let Some(fk) = field_kind.filter(|&k| cow_clonable(Some(k)) && is_collection_clonable(k)) {
        emit_should_clone(code, handle);
        code.push(0x04);
        code.push(0x40); // if the list is shared: this record's field takes a copy of its own
        {
            emit_collection_clone(code, ctx, plan.num_regs, fk, handle);
            local_get(code, obj as u32);
            i32_load(code, 8);
            local_get(code, plan.num_regs + 5);
            i32_store(code, off);
            local_get(code, plan.num_regs + 5);
            local_set(code, handle);
            // S8's true count: the field holds the fresh copy beyond the turn that made it.
            if ctx.heap_counting() && counted_kind(Some(fk)) {
                emit_true_retain_of(code, ctx, |c| local_get(c, handle));
            }
        }
        code.push(0x0B);
    }
    if elem == Kind::Int && ctx.exact.sequence_ints() {
        exact::promote_sequence(code, ctx, plan.num_regs, handle)?;
        exact::push_sequence_word(code, plan, ctx, handle, src)
    } else {
        lower_list_push_at(code, elem, ctx, plan.num_regs, handle, src)
    }
}

/// Whether argument `a` of a call of `func`, register `arg`, is a heap value the callee's parameter becomes a second
/// holder of: one the parameter takes as it is, not one converted into the runtime's value.
fn shares_argument(plan: &Plan, ctx: &Ctx, func: u16, a: u16, arg: u16) -> bool {
    let param = ctx
        .fn_param_kinds
        .get(func as usize)
        .and_then(|ks| ks.get(a as usize))
        .copied()
        .flatten();
    counts_aliases(plan.kinds.get(arg as usize)) && param != Some(Kind::Tower)
}

/// Push argument `a` of a call of `func`, register `arg`, as the callee's parameter takes it: the runtime's value
/// for a parameter holding a value of the exact extension ([`tower::push_argument`]), the rational an integer
/// equals for a rational parameter, the `f64` of an `Int` for a `Float` parameter (`half(9)` → `half(9.0)`), and
/// anything else at its own value type, which must be the parameter's.
fn push_call_argument(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, func: u16, a: u16, arg: u16) -> R<()> {
    let kinds = &plan.kinds;
    let param = ctx
        .fn_param_kinds
        .get(func as usize)
        .and_then(|ks| ks.get(a as usize))
        .copied()
        .flatten();
    if tower::push_argument(code, plan, ctx, arg, param)?.is_some() {
        return Ok(());
    }
    let arg_vt = kinds.valtype(arg as usize);
    let param_vt = ctx
        .fn_param_valtypes
        .get(func as usize)
        .ok_or(WasmLowerError::Unsupported("call of unknown function"))?
        .get(a as usize)
        .copied()
        .unwrap_or(I64);
    if param == Some(Kind::Rational) && matches!(kinds.get(arg as usize), Some(Kind::Int | Kind::BigInt)) {
        push_rational_operand(code, kinds, ctx, arg)
    } else if arg_vt == param_vt {
        local_get(code, arg as u32);
        Ok(())
    } else if arg_vt == I64 && param_vt == F64 {
        push_as_f64(code, arg, kinds.get(arg as usize))
    } else {
        Err(WasmLowerError::Unsupported(
            "call argument type does not match the parameter",
        ))
    }
}

/// Emit a task's argument marshaling + `call` (the shared core of `Spawn` and `SpawnHandle`) — retain clonable
/// heap args (value semantics), pass each as the callee's parameter takes it ([`push_call_argument`]). Returns
/// whether the callee leaves a RESULT on the stack (dropped for a fire-and-forget spawn).
fn emit_sync_call(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    func: u16,
    args_start: u16,
    arg_count: u16,
) -> R<bool> {
    if ctx.fn_stubbed.get(func as usize).copied().unwrap_or(false) {
        code.push(0x00); // unreachable — the reach analysis proved this call never runs
        return Ok(false);
    }
    if ctx
        .fn_inout
        .get(func as usize)
        .is_some_and(|ps| !ps.is_empty())
    {
        return Err(WasmLowerError::Unsupported(
            "a task that lends a variable to a `mutable` parameter",
        ));
    }
    for a in 0..arg_count {
        let arg = args_start + a;
        if shares_argument(plan, ctx, func, a, arg) {
            emit_retain(code, ctx, arg);
        }
    }
    for a in 0..arg_count {
        push_call_argument(code, plan, ctx, func, a, args_start + a)?;
    }
    code.push(0x10); // call
    leb_u32(code, ctx.fn_base + func as u32);
    Ok(ctx
        .fn_results
        .get(func as usize)
        .copied()
        .flatten()
        .is_some())
}

/// `Receive <dst> from <chan>` (`ChanRecv`) on a single-threaded FIFO channel — pop the FRONT element:
/// Non-blocking `Try to receive` (`ChanTryRecv`) → an `Optional`: a non-empty queue pops its front
/// element and boxes it (`Some` — a fresh 8-byte heap box holding the inner scalar; handle != 0); an
/// empty queue yields `Nothing` (handle `0`). There is no blocking/trap path — the deterministic
/// single-task scheduler resumes a try-recv immediately either way (`scheduler::do_try_recv`). The
/// present inner kind (for a later `Show`) is carried out-of-band in `opt_inner` from this channel.
fn lower_chan_try_recv(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    chan: u16,
) -> R<()> {
    let elem =
        kinds
            .get(chan as usize)
            .and_then(Kind::seq_elem)
            .ok_or(WasmLowerError::Unsupported(
                "try-receive from a channel of unknown element kind",
            ))?;
    let ch = chan as u32;
    let idx = num_regs + 5; // i32 pop-front shift-loop scratch
    let boxp = num_regs + 6; // i32 Optional box pointer
                             // A width-matched scratch holds the popped value before it is stored into the box.
    let val = match elem {
        Kind::Float => num_regs + 12, // f64 scratch
        Kind::Int
        | Kind::Bool
        | Kind::Char
        | Kind::Moment
        | Kind::Duration
        | Kind::Time
        | Kind::Span => num_regs + 1, // i64 scratch
        _ => num_regs + 7,            // i32-handle scratch
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
    emit_alloc(code, ctx, boxp, AllocClass::Raw);
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
fn lower_chan_recv(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    num_regs: u32,
    dst: u16,
    chan: u16,
) -> R<()> {
    let elem =
        kinds
            .get(chan as usize)
            .and_then(Kind::seq_elem)
            .ok_or(WasmLowerError::Unsupported(
                "receive from a channel of unknown element kind",
            ))?;
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
fn lower_select_wait(
    code: &mut Vec<u8>,
    plan: &Plan,
    kinds: &KindTable,
    blocks: &Blocks,
    k: usize,
    pc: usize,
    dst_arm: u16,
) -> R<()> {
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
            let elem = kinds.get(chan as usize).and_then(Kind::seq_elem).ok_or(
                WasmLowerError::Unsupported("select recv arm on a channel of unknown element kind"),
            )?;
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
fn lower_magic_div(
    code: &mut Vec<u8>,
    num_regs: u32,
    dst: u16,
    lhs: u16,
    magic: u64,
    more: u8,
    mul_back: i64,
) {
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
/// sign (den > 0), reduces by `gcd(|num|, den)`, allocates a counted scalar pair, and
/// leaves its handle in `dst`. A whole quotient reduces to `den == 1` (so `Show` renders just the
/// integer, matching the VM's downsize). Traps on a zero divisor (the VM errors "Division by zero").
fn lower_exact_div(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, dst: u16, lhs: u16, rhs: u16) {
    let (num, den, a, b) = (num_regs + 1, num_regs + 2, num_regs + 3, num_regs + 4);
    let (handle, data) = (num_regs + 5, num_regs + 6);
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
    // An immutable pair owns two scalar slots through the ordinary counted header.
    i32_const(code, 16);
    emit_alloc(code, ctx, handle, AllocClass::ScalarHeader);
    i32_const(code, 16);
    emit_alloc(code, ctx, data, AllocClass::Raw);
    for off in [0u32, 4] {
        local_get(code, handle);
        i32_const(code, 2);
        i32_store(code, off);
    }
    local_get(code, handle);
    local_get(code, data);
    i32_store(code, 8);
    local_get(code, data);
    local_get(code, num);
    i64_store(code, 0);
    local_get(code, data);
    local_get(code, den);
    i64_store(code, 8);
    local_get(code, handle);
    local_set(code, dst as u32);
}

/// Emit `for i in 0..len(src): dest[(base_off + i)] = src_bytes[i]`, a single-byte copy loop (for
/// `Text`). `offset_by_a` shifts the destination by `len(a_for_offset)` (to append after the first
/// operand of a concat); otherwise the copy is index-aligned.
fn emit_byte_copy(
    code: &mut Vec<u8>,
    idx: u32,
    dest_data: u32,
    a_for_offset: u32,
    src: u32,
    offset_by_a: bool,
) {
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
    // Interned: the literal already exists in the data section, so its handle is a constant and
    // nothing is built at run time. See [`TextPool`].
    if let Some(pool) = ctx.text_pool {
        let addr = pool.borrow_mut().intern(bytes);
        i32_const(code, addr);
        return;
    }
    let len = bytes.len();
    let cap8 = (len + 7) & !7; // 8-aligned buffer so the last i64.store stays in bounds
    let (hdr, data) = (num_regs + 5, num_regs + 6);
    i32_const(code, 16);
    emit_alloc(code, ctx, hdr, AllocClass::TextHeader);
    i32_const(code, cap8 as i32);
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
    emit_alloc(code, ctx, hdr, AllocClass::TextHeader);
    i32_const(code, 8);
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
fn lower_seq_concat(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    lhs: u16,
    rhs: u16,
) -> R<()> {
    let elem = seq_elem_kind(kinds, lhs)?;
    seq_elem_kind(kinds, rhs)?;
    let (a, b) = (lhs as u32, rhs as u32);
    let exact_ints = elem == Kind::Int && ctx.exact.sequence_ints();
    if exact_ints {
        exact::promote_sequence(code, ctx, num_regs, lhs as u32)?;
        exact::promote_sequence(code, ctx, num_regs, rhs as u32)?;
    }
    let (hdr, data, idx) = (num_regs + 5, num_regs + 6, num_regs + 7);
    // hdr = alloc(16); data = alloc((len_a + len_b) * 8)
    i32_const(code, 16);
    emit_alloc(
        code,
        ctx,
        hdr,
        if exact_ints {
            AllocClass::ExactIntegerHeader
        } else {
            AllocClass::of_elements(kind_is_handle(elem))
        },
    );
    local_get(code, a);
    i32_load(code, 0);
    local_get(code, b);
    i32_load(code, 0);
    code.push(0x6A); // len_a + len_b
    i32_const(code, 8);
    code.push(0x6C);
    emit_alloc(code, ctx, data, AllocClass::Raw);
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
    // S8's true count: the new list is a further holder of every element handle it copied.
    if kind_is_handle(elem) {
        emit_retain_children(code, ctx, hdr);
    }
    if exact_ints {
        let clone = (ctx.host_index)(HostFn::ExactClone)
            .ok_or(WasmLowerError::Unsupported("exact clone import"))?;
        emit_exact_buffer_holds(code, hdr, num_regs + 21, num_regs + 22, clone, true);
    }
    local_get(code, hdr);
    local_set(code, dst as u32);
    Ok(())
}

/// Emit `for i in 0..len(src): dest[(base_off + i)*8] = src_data[i*8]`, a raw 8-byte element copy
/// loop. When `offset_by_a` the destination index is shifted by `len(a_for_offset)` (used to append
/// the second operand of a concat after the first); otherwise the copy is index-aligned.
fn emit_seq_copy(
    code: &mut Vec<u8>,
    idx: u32,
    dest_data: u32,
    a_for_offset: u32,
    src: u32,
    offset_by_a: bool,
) {
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
fn lower_slice(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    dst: u16,
    collection: u16,
    start: u16,
    end: u16,
) -> R<()> {
    let elem = seq_elem_kind(kinds, collection)?; // require a scalar sequence; the copy is raw 8-byte either way
    let col = collection as u32;
    let (s0, ee) = (num_regs + 1, num_regs + 2); // i64 scratch: start0, end_excl
    let exact_ints = elem == Kind::Int && ctx.exact.sequence_ints();
    if exact_ints {
        exact::promote_sequence(code, ctx, num_regs, collection as u32)?;
    }
    let (hdr, data, idx) = (num_regs + 5, num_regs + 6, num_regs + 7); // i32 scratch
                                                                       // hdr = alloc(16)
    i32_const(code, 16);
    emit_alloc(
        code,
        ctx,
        hdr,
        if exact_ints {
            AllocClass::ExactIntegerHeader
        } else {
            AllocClass::of_elements(kind_is_handle(elem))
        },
    );
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
        emit_alloc(code, ctx, data, AllocClass::Raw);
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
                     // S8's true count: the new list is a further holder of every element handle it copied.
    if kind_is_handle(elem) {
        emit_retain_children(code, ctx, hdr);
    }
    if exact_ints {
        let clone = (ctx.host_index)(HostFn::ExactClone)
            .ok_or(WasmLowerError::Unsupported("exact clone import"))?;
        emit_exact_buffer_holds(code, hdr, num_regs + 21, num_regs + 22, clone, true);
    }
    local_get(code, hdr);
    local_set(code, dst as u32);
    Ok(())
}

/// A running `Repeat` loop's frame — one heap block from `IterPrepare` to `IterPop`, linked to the
/// frame below it:
///
/// ```text
/// [len @0][cursor @4][prev @8][size @12][payload @16…]
/// ```
///
/// Over a sequence the payload is the SNAPSHOT of its elements (8-byte slots), so a write to the
/// sequence inside the loop does not perturb the walk — the tree-walker's `iteration_snapshot`;
/// `len`/`cursor` count elements. Over a `Text` the payload is the text's handle (a Text is
/// immutable, so it needs no copy) and `len`/`cursor` count bytes. `size` is the block's byte size,
/// so whatever frees a frame needs nothing else.
const FRAME_LEN: u32 = 0;
const FRAME_CURSOR: u32 = 4;
const FRAME_PREV: u32 = 8;
const FRAME_SIZE: u32 = 12;
const FRAME_KIND: u32 = 16; // 0 = scalar/range, 1 = one Text handle, 2 = handle slots
const FRAME_PAYLOAD: u32 = 24;

/// Whether a function starts `Repeat` loops, and so must unwind them on every way out.
fn runs_loops(plan: &Plan) -> bool {
    plan.ops
        .iter()
        .any(|op| matches!(op, Op::IterPrepare { .. }))
}

/// The local holding the loop frame that was on top when the function was entered (see
/// [`encode_locals`]).
fn loop_base_local(plan: &Plan) -> u32 {
    plan.num_regs + 24
}

/// Leave the function's loops: pop every frame above the one it was entered with, as the VM
/// truncates its iterator stack at a return. Self-contained, each frame goes back to the allocator
/// (`__logos_iter_unwind`); linked, the frames are the runtime's and the pointer is simply reset.
fn emit_leave_loops(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx) {
    if !runs_loops(plan) {
        return;
    }
    local_get(code, loop_base_local(plan));
    match ctx.runtime.and_then(|r| r.index(RuntimeFn::IterUnwind)) {
        Some(unwind) => {
            code.push(0x10); // call __logos_iter_unwind(base)
            leb_u32(code, unwind);
        }
        None => global_set(code, ctx.iter_global),
    }
}

/// `__logos_iter_unwind(base)`: pop and free loop frames until `base` is on top.
fn emit_iter_unwind_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = Vec::new();
    // locals: 1 = frame, 2 = index, 3 = number of held handles
    c.push(0x01);
    leb_u32(&mut c, 4);
    c.push(I32);
    c.push(0x02);
    c.push(0x40); // block $done
    c.push(0x03);
    c.push(0x40); // loop $pop
    global_get(&mut c, env.iter_global);
    local_tee(&mut c, 1);
    local_get(&mut c, 0);
    c.push(0x46); // i32.eq — back to the base
    local_get(&mut c, 1);
    c.push(0x45); // i32.eqz — no frame at all
    c.push(0x72); // i32.or
    c.push(0x0D);
    leb_u32(&mut c, 1); // br_if $done
    if env.retain_base.is_some() {
        local_get(&mut c, 1);
        i32_load(&mut c, FRAME_KIND);
        c.push(0x04);
        c.push(0x40);
        i32_const(&mut c, 1);
        local_get(&mut c, 1);
        i32_load(&mut c, FRAME_LEN);
        local_get(&mut c, 1);
        i32_load(&mut c, FRAME_KIND);
        i32_const(&mut c, 1);
        c.push(0x46);
        c.push(0x1B); // a Text snapshot holds one handle; a sequence holds len handles
        local_set(&mut c, 3);
        i32_const(&mut c, 0);
        local_set(&mut c, 2);
        c.push(0x02);
        c.push(0x40);
        c.push(0x03);
        c.push(0x40);
        local_get(&mut c, 2);
        local_get(&mut c, 3);
        c.push(0x4F);
        c.push(0x0D);
        leb_u32(&mut c, 1);
        local_get(&mut c, 1);
        local_get(&mut c, 2);
        i32_const(&mut c, 8);
        c.push(0x6C);
        c.push(0x6A);
        if let Some(drop) = env.exact_drop {
            local_set(&mut c, 4);
            local_get(&mut c, 1);
            i32_load(&mut c, FRAME_KIND);
            i32_const(&mut c, 3);
            c.extend([0x46, 0x04, 0x40]);
            local_get(&mut c, 4);
            i64_load(&mut c, FRAME_PAYLOAD);
            c.push(0x42);
            leb_i64(&mut c, 1);
            c.extend([0x83, 0xa7, 0x04, 0x40]);
            local_get(&mut c, 4);
            i64_load(&mut c, FRAME_PAYLOAD);
            c.push(0x10);
            leb_u32(&mut c, drop);
            c.extend([0x0b, 0x05]);
            local_get(&mut c, 4);
            i32_load(&mut c, FRAME_PAYLOAD);
            c.push(0x10);
            leb_u32(&mut c, env.call(RuntimeFn::Release));
            c.push(0x0b);
        } else {
            i32_load(&mut c, FRAME_PAYLOAD);
            c.push(0x10);
            leb_u32(&mut c, env.call(RuntimeFn::Release));
        }
        local_get(&mut c, 2);
        i32_const(&mut c, 1);
        c.push(0x6A);
        local_set(&mut c, 2);
        c.push(0x0C);
        leb_u32(&mut c, 0);
        c.push(0x0B);
        c.push(0x0B);
        c.push(0x0B);
    }
    local_get(&mut c, 1);
    i32_load(&mut c, FRAME_PREV);
    global_set(&mut c, env.iter_global);
    local_get(&mut c, 1);
    local_get(&mut c, 1);
    i32_load(&mut c, FRAME_SIZE);
    c.push(0x10); // call __logos_free(frame, size)
    leb_u32(&mut c, env.call(RuntimeFn::Free));
    c.push(0x0C);
    leb_u32(&mut c, 0); // br $pop
    c.push(0x0B);
    c.push(0x0B);
    c.push(0x0B); // end function
    c
}

/// A range-only iterator owns two captured i64 bounds, never an array of its elements.
fn lower_range_iter_prepare(code: &mut Vec<u8>, ctx: &Ctx, num_regs: u32, start: u16, end: u16) {
    let frame = num_regs + 5;
    i32_const(code, FRAME_PAYLOAD as i32 + 16);
    emit_alloc(code, ctx, frame, AllocClass::Raw);
    local_get(code, frame);
    local_get(code, start as u32);
    i64_store(code, FRAME_PAYLOAD);
    local_get(code, frame);
    local_get(code, end as u32);
    i64_store(code, FRAME_PAYLOAD + 8);
    // len is a has-next flag. An empty range never enters its body.
    local_get(code, frame);
    local_get(code, start as u32);
    local_get(code, end as u32);
    code.push(0x57); // i64.le_s
    i32_store(code, FRAME_LEN);
    local_get(code, frame);
    i32_const(code, 0);
    i32_store(code, FRAME_CURSOR);
    local_get(code, frame);
    i32_const(code, 0);
    i32_store(code, FRAME_KIND);
    local_get(code, frame);
    i32_const(code, FRAME_PAYLOAD as i32 + 16);
    i32_store(code, FRAME_SIZE);
    local_get(code, frame);
    global_get(code, ctx.iter_global);
    i32_store(code, FRAME_PREV);
    local_get(code, frame);
    global_set(code, ctx.iter_global);
}

#[allow(clippy::too_many_arguments)]
fn lower_range_iter_next(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    blocks: &Blocks,
    k: usize,
    dst: u16,
    exit: usize,
    pc: usize,
) -> R<()> {
    global_get(code, ctx.iter_global);
    i32_load(code, FRAME_LEN);
    code.push(0x04);
    code.push(0x40);
    global_get(code, ctx.iter_global);
    i64_load(code, FRAME_PAYLOAD);
    exact::set_word(code, plan, ctx, dst)?;
    // Never increment the last value: MAX..MAX runs once without overflowing.
    global_get(code, ctx.iter_global);
    i64_load(code, FRAME_PAYLOAD);
    global_get(code, ctx.iter_global);
    i64_load(code, FRAME_PAYLOAD + 8);
    code.push(0x51); // i64.eq
    code.push(0x04);
    code.push(0x40);
    global_get(code, ctx.iter_global);
    i32_const(code, 0);
    i32_store(code, FRAME_LEN);
    code.push(0x05);
    global_get(code, ctx.iter_global);
    global_get(code, ctx.iter_global);
    i64_load(code, FRAME_PAYLOAD);
    code.push(0x42);
    leb_i64(code, 1);
    code.push(0x7C);
    i64_store(code, FRAME_PAYLOAD);
    code.push(0x0B);
    set_next_block(code, plan.num_regs, blocks.block_of(pc + 1));
    code.push(0x05);
    set_next_block(code, plan.num_regs, blocks.block_of(exit));
    code.push(0x0B);
    code.push(0x0C);
    leb_u32(code, blocks.br_loop(k));
    Ok(())
}

/// `IterPrepare`: open a loop frame over the sequence or Text in `iterable` and make it the top one.
fn lower_iter_prepare(
    code: &mut Vec<u8>,
    kinds: &KindTable,
    ctx: &Ctx,
    num_regs: u32,
    iterable: u16,
) -> R<()> {
    let exact_ints =
        ctx.exact.sequence_ints() && kinds.get(iterable as usize) == Some(Kind::SeqInt);
    if exact_ints {
        exact::promote_sequence(code, ctx, num_regs, iterable as u32)?;
    }
    let it = iterable as u32;
    let (frame, idx) = (num_regs + 5, num_regs + 6); // i32 scratch
    let text = kinds.get(iterable as usize) == Some(Kind::Text);
    let handles = empty_collection_class(kinds.get(iterable as usize))
        .held_handles()
        .0;
    if !text
        && !kinds
            .get(iterable as usize)
            .map(Kind::is_seq)
            .unwrap_or(false)
    {
        return Err(WasmLowerError::Unsupported(
            "iteration over a value that is neither a sequence nor a Text",
        ));
    }
    if text {
        i32_const(code, FRAME_PAYLOAD as i32 + 8);
        emit_alloc(code, ctx, frame, AllocClass::Raw);
        local_get(code, frame);
        local_get(code, it);
        i32_store(code, FRAME_PAYLOAD);
        emit_true_retain(code, ctx, iterable);
    } else {
        // frame = alloc(16 + len * 8); copy the elements into its payload
        local_get(code, it);
        i32_load(code, 0);
        i32_const(code, 8);
        code.push(0x6C);
        i32_const(code, FRAME_PAYLOAD as i32);
        code.push(0x6A);
        emit_alloc(code, ctx, frame, AllocClass::Raw);
        i32_const(code, 0);
        local_set(code, idx);
        code.push(0x02);
        code.push(0x40); // block $exit
        code.push(0x03);
        code.push(0x40); // loop $loop
        local_get(code, idx);
        local_get(code, it);
        i32_load(code, 0);
        code.push(0x4E); // i32.ge_s → i >= len
        code.push(0x0D);
        leb_u32(code, 1); // br_if $exit
        local_get(code, frame);
        local_get(code, idx);
        i32_const(code, 8);
        code.push(0x6C);
        code.push(0x6A);
        local_get(code, it);
        i32_load(code, 8);
        local_get(code, idx);
        i32_const(code, 8);
        code.push(0x6C);
        code.push(0x6A);
        i64_load(code, 0);
        if exact_ints {
            exact::clone_sequence_word(code, ctx, num_regs)?;
        }
        i64_store(code, FRAME_PAYLOAD);
        if handles {
            emit_true_retain_of(code, ctx, |c| {
                local_get(c, frame);
                local_get(c, idx);
                i32_const(c, 8);
                c.push(0x6C);
                c.push(0x6A);
                i32_load(c, FRAME_PAYLOAD);
            });
        }
        local_get(code, idx);
        i32_const(code, 1);
        code.push(0x6A);
        local_set(code, idx);
        code.push(0x0C);
        leb_u32(code, 0); // br $loop
        code.push(0x0B);
        code.push(0x0B);
    }
    local_get(code, frame);
    i32_const(
        code,
        if text {
            1
        } else if exact_ints {
            3
        } else if handles {
            2
        } else {
            0
        },
    );
    i32_store(code, FRAME_KIND);
    // frame.len = the iterable's length (elements, or a Text's bytes); cursor = 0
    local_get(code, frame);
    local_get(code, it);
    i32_load(code, 0);
    i32_store(code, FRAME_LEN);
    local_get(code, frame);
    i32_const(code, 0);
    i32_store(code, FRAME_CURSOR);
    // frame.size = the block's size
    local_get(code, frame);
    if text {
        i32_const(code, FRAME_PAYLOAD as i32 + 8);
    } else {
        local_get(code, it);
        i32_load(code, 0);
        i32_const(code, 8);
        code.push(0x6C);
        i32_const(code, FRAME_PAYLOAD as i32);
        code.push(0x6A);
    }
    i32_store(code, FRAME_SIZE);
    // frame.prev = the old top; the new top = frame
    local_get(code, frame);
    global_get(code, ctx.iter_global);
    i32_store(code, FRAME_PREV);
    local_get(code, frame);
    global_set(code, ctx.iter_global);
    Ok(())
}

/// `IterNext { dst, exit }`: a conditional block terminator. If the top frame's `cursor < len`, bind
/// the next item to `dst`, advance, and fall through to the loop body; otherwise branch to `exit`
/// (the matching `IterPop`). Over a sequence the item is the snapshot slot, loaded at the loop
/// variable's width; over a Text it is a fresh one-character Text of the UTF-8 sequence at the
/// cursor, whose length the lead byte gives. Either way it sets the dispatch "next block" local and
/// `br`s to `$loop`, like every other branch terminator.
#[allow(clippy::too_many_arguments)]
fn lower_iter_next(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    blocks: &Blocks,
    k: usize,
    dst: u16,
    exit: usize,
    pc: usize,
) -> R<()> {
    if !ctx.linked
        && plan
            .iter_iterable
            .get(pc)
            .copied()
            .flatten()
            .is_some_and(|r| plan.range_loops.contains_key(&(r as u16)))
    {
        return lower_range_iter_next(code, plan, ctx, blocks, k, dst, exit, pc);
    }
    let (kinds, num_regs) = (&plan.kinds, plan.num_regs);
    let text = plan
        .iter_iterable
        .get(pc)
        .copied()
        .flatten()
        .is_some_and(|r| kinds.get(r) == Some(Kind::Text));
    let ig = ctx.iter_global;
    let pc_local = num_regs;
    let fallthrough = blocks.block_of(pc + 1) as u32;
    let exit_block = blocks.block_of(exit) as u32;
    // cursor < len ?
    global_get(code, ig);
    i32_load(code, FRAME_CURSOR);
    global_get(code, ig);
    i32_load(code, FRAME_LEN);
    code.push(0x48); // i32.lt_s
    code.push(0x04);
    code.push(0x40); // if (void)
    if text {
        let (src, bytes, hdr, buf, i) = (
            num_regs + 5,
            num_regs + 6,
            num_regs + 7,
            num_regs + 8,
            num_regs + 9,
        );
        // src = the Text's bytes at the cursor
        global_get(code, ig);
        i32_load(code, FRAME_PAYLOAD);
        i32_load(code, 8);
        global_get(code, ig);
        i32_load(code, FRAME_CURSOR);
        code.push(0x6A);
        local_set(code, src);
        // bytes = 1 / 2 / 3 / 4 by the lead byte (< 0x80, < 0xE0, < 0xF0, else)
        i32_const(code, 1);
        i32_const(code, 2);
        i32_const(code, 3);
        i32_const(code, 4);
        local_get(code, src);
        i32_load8_u(code, 0);
        i32_const(code, 0xF0);
        code.push(0x49); // i32.lt_u
        code.push(0x1B); // select → b0 < 0xF0 ? 3 : 4
        local_get(code, src);
        i32_load8_u(code, 0);
        i32_const(code, 0xE0);
        code.push(0x49);
        code.push(0x1B); // select → b0 < 0xE0 ? 2 : …
        local_get(code, src);
        i32_load8_u(code, 0);
        i32_const(code, 0x80);
        code.push(0x49);
        code.push(0x1B); // select → b0 < 0x80 ? 1 : …
        local_set(code, bytes);
        // dst = a fresh Text of those bytes
        i32_const(code, 16);
        emit_alloc(code, ctx, hdr, AllocClass::TextHeader);
        local_get(code, bytes);
        emit_alloc(code, ctx, buf, AllocClass::Raw);
        i32_const(code, 0);
        local_set(code, i);
        code.push(0x02);
        code.push(0x40); // block
        code.push(0x03);
        code.push(0x40); // loop
        local_get(code, i);
        local_get(code, bytes);
        code.push(0x4E); // i32.ge_s
        code.push(0x0D);
        leb_u32(code, 1);
        local_get(code, buf);
        local_get(code, i);
        code.push(0x6A);
        local_get(code, src);
        local_get(code, i);
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
        for off in [0u32, 4] {
            local_get(code, hdr);
            local_get(code, bytes);
            i32_store(code, off);
        }
        local_get(code, hdr);
        local_get(code, buf);
        i32_store(code, 8);
        local_get(code, hdr);
        local_set(code, dst as u32);
        // cursor += bytes
        global_get(code, ig);
        global_get(code, ig);
        i32_load(code, FRAME_CURSOR);
        local_get(code, bytes);
        code.push(0x6A);
        i32_store(code, FRAME_CURSOR);
    } else {
        // The loop variable's kind IS the element kind; load it at its width (`f64`/`i64`, or `i32`
        // for a heap handle like Text/Struct/Enum).
        let elem_load: fn(&mut Vec<u8>, u32) = match kinds.get(dst as usize).map(Kind::wasm_valtype)
        {
            Some(F64) => f64_load,
            Some(I64) => i64_load,
            _ => i32_load,
        };
        // dst = payload[cursor]
        global_get(code, ig);
        global_get(code, ig);
        i32_load(code, FRAME_CURSOR);
        i32_const(code, 8);
        code.push(0x6C); // i32.mul
        code.push(0x6A); // i32.add → element addr - payload offset
        elem_load(code, FRAME_PAYLOAD);
        if ctx.exact.sequence_ints()
            && plan
                .iter_iterable
                .get(pc)
                .copied()
                .flatten()
                .is_some_and(|r| kinds.get(r) == Some(Kind::SeqInt))
        {
            exact::take_sequence_tagged(code, plan, ctx, dst)?;
        } else {
            exact::set_word(code, plan, ctx, dst)?;
        }
        // cursor++
        global_get(code, ig);
        global_get(code, ig);
        i32_load(code, FRAME_CURSOR);
        i32_const(code, 1);
        code.push(0x6A); // i32.add
        i32_store(code, FRAME_CURSOR);
    }
    // next block = fallthrough (the loop body)
    set_next_block(code, pc_local, fallthrough as usize);
    code.push(0x05); // else
                     // exhausted: next block = exit (the IterPop)
    set_next_block(code, pc_local, exit_block as usize);
    code.push(0x0B); // end if
    code.push(0x0C); // br $loop
    leb_u32(code, blocks.br_loop(k));
    Ok(())
}

/// Whether an op touches the heap value model's linear memory (so the module needs a memory +
/// `__heap_ptr` global). Grows as heap ops are lowered.
fn op_uses_heap(op: &Op) -> bool {
    matches!(
        op,
        Op::NewEmptyList { .. }
            | Op::NewEmptyListWithCapacity { .. }
            | Op::NewEmptyListI32 { .. }
            | Op::NewEmptyListFixed { .. }
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
            | Op::CrdtJoin { .. }
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
            | Op::NewEmptyMapWithCapacity { .. }
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
            // `get`'s text half allocates the buffer its host writes into, and the Text header over it.
            | Op::CallBuiltin { builtin: BuiltinId::EnvValue, .. }
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
fn encode_locals(plan: &Plan, root_count: usize) -> Vec<u8> {
    let mut groups: Vec<(u32, u8)> = Vec::new();
    let push = |vt: u8, groups: &mut Vec<(u32, u8)>| match groups.last_mut() {
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
    // (+5/+6/+7), two operand-handle holders for `Concat`'s stringify-then-byte-copy (+8/+9), a
    // Text-keyed-Map per-entry key compare's +8/+9/+10, and a Text append's capacity and new length
    // (+10/+11) — so seven cover the deepest user.
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
    // ── CRDT COUNTER scratch — the per-replica `ConvergentCount`/`Tally` object ops (project the
    //    Int total, bump this replica's slot, per-replica MAX merge). The merge NESTS an inner search
    //    over the target's records inside the outer scan over the source's records, so it needs locals
    //    disjoint from every op above. Two i64 (num_regs+15/+16): the running total / the max operands
    //    and record ids. Six i32 (num_regs+17..=+22): outer index, inner index, target data_ptr, source
    //    data_ptr, a matched-slot cursor, and a found flag — the deepest user is the merge.
    push(I64, &mut groups); // +15
    push(I64, &mut groups); // +16
    push(I32, &mut groups); // +17
    push(I32, &mut groups); // +18
    push(I32, &mut groups); // +19
    push(I32, &mut groups); // +20
    push(I32, &mut groups); // +21
    push(I32, &mut groups); // +22
                            // A Map/Set insert's growth doubling (num_regs+23): the new capacity, held across the alloc
                            // and copy that grow its buffer. Dedicated rather than sharing +5..+11 because `a union b` /
                            // `a intersection b` build their result through `emit_set_add_elem` with `s` ITSELF aliasing
                            // +8 and the outer scan counter live in +9 across the call — either would corrupt the set
                            // being built rather than merely a scratch value.
    push(I32, &mut groups); // +23
                            // The `Repeat` frame on top when the function was entered (`loop_base_local`), which every
                            // return unwinds back to.
    push(I32, &mut groups); // +24
                            // A module carrying tagged words: two operands and a result, then one saved word per exact register
                            // ([`exact`]).
    for _ in 0..exact::extra_locals(plan) {
        push(I64, &mut groups);
    }
    for _ in 0..absence::extra_locals(plan) {
        push(I32, &mut groups);
    }
    for _ in 0..tower::extra_locals(plan) {
        push(I32, &mut groups);
    }
    for _ in 0..root_count {
        push(I32, &mut groups);
    } // owning shadow roots, initially null

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

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/module/alloc_class_tests.rs"]
mod alloc_class_tests;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/module/transient_reads_tests.rs"]
mod transient_reads_tests;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/module/range_storage_tests.rs"]
mod range_storage_tests;
