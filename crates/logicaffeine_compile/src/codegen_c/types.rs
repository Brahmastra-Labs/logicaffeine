use std::collections::HashMap;

use crate::analysis::policy::PolicyRegistry;
use crate::analysis::TypeRegistry;
use crate::ast::stmt::*;
use crate::intern::{Interner, Symbol};

pub(super) fn is_c_reserved(name: &str) -> bool {
    matches!(
        name,
        // C keywords
        "auto" | "break" | "case" | "char" | "const" | "continue" | "default" |
        "do" | "double" | "else" | "enum" | "extern" | "float" | "for" | "goto" |
        "if" | "int" | "long" | "register" | "return" | "short" | "signed" |
        "sizeof" | "static" | "struct" | "switch" | "typedef" | "union" |
        "unsigned" | "void" | "volatile" | "while" |
        // C99
        "inline" | "restrict" | "_Bool" | "_Complex" | "_Imaginary" |
        // C11
        "_Alignas" | "_Alignof" | "_Atomic" | "_Generic" | "_Noreturn" |
        "_Static_assert" | "_Thread_local" |
        // C23
        "bool" | "true" | "false" | "nullptr" | "alignas" | "alignof" |
        "constexpr" | "static_assert" | "thread_local" | "typeof" |
        // Standard library types/functions that we use in the runtime
        "printf" | "malloc" | "calloc" | "realloc" | "free" | "memcpy" |
        "strlen" | "strdup" | "snprintf" | "atoll" | "atof" |
        // POSIX common
        "size_t" | "ssize_t" | "ptrdiff_t" | "intptr_t" |
        // Our runtime identifiers (prevent user collision)
        "main" | "argc" | "argv" |
        // <math.h> / libm — a LOGOS stdlib function shares the English name of a libm
        // builtin (`sin`, `cos`, `abs`, …). Emitting `int64_t sin(...)` clashes with the
        // compiler's built-in `double sin(double)` ("conflicting types"); mangle to
        // `logos_sin` at both the definition and every call (escape_c_ident is the single
        // choke point, so the pair stays coherent). C89 + C99 <math.h>:
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "atan2" |
        "sinh" | "cosh" | "tanh" | "asinh" | "acosh" | "atanh" |
        "exp" | "exp2" | "expm1" | "log" | "log2" | "log10" | "log1p" | "logb" |
        "pow" | "sqrt" | "cbrt" | "hypot" | "fabs" | "fmod" | "modf" | "frexp" |
        "ldexp" | "scalbn" | "scalbln" | "ilogb" | "nextafter" | "nexttoward" |
        "copysign" | "nan" | "fmin" | "fmax" | "fdim" | "fma" | "remainder" |
        "remquo" | "ceil" | "floor" | "trunc" | "round" | "lround" | "llround" |
        "rint" | "lrint" | "llrint" | "nearbyint" | "erf" | "erfc" |
        "tgamma" | "lgamma" | "isnan" | "isinf" | "isfinite" | "signbit" |
        // <math.h> classification/comparison MACROS (C99). These expand at the preprocessor —
        // emitting `char *fpclassify(double x)` expands to `__builtin_fpclassify(0,1,4,3,2, double x)`
        // ("expected declaration specifiers before numeric constant"), so they MUST be mangled.
        "fpclassify" | "isnormal" | "isgreater" | "isgreaterequal" | "isless" |
        "islessequal" | "islessgreater" | "isunordered" |
        // Legacy/SVID <math.h> functions glibc declares under _DEFAULT_SOURCE (Bessel + friends):
        "gamma" | "j0" | "j1" | "jn" | "y0" | "y1" | "yn" | "scalb" | "significand" | "drem" |
        // <stdlib.h> — English-named collisions LOGOS stdlib is likely to hit:
        "abs" | "labs" | "llabs" | "div" | "ldiv" | "lldiv" | "atoi" | "atol" |
        "strtol" | "strtoll" | "strtod" | "rand" | "srand" | "qsort" | "bsearch" |
        "exit" | "abort" | "system" | "getenv" | "atexit" |
        // <string.h> — likewise:
        "strcpy" | "strncpy" | "strcat" | "strncat" | "strcmp" | "strncmp" |
        "strchr" | "strrchr" | "strstr" | "strtok" | "memset" | "memmove" |
        "memcmp" | "memchr" |
        // <ctype.h> — English predicate names LOGOS text stdlib mirrors:
        "isalpha" | "isdigit" | "isalnum" | "isspace" | "isupper" | "islower" |
        "iscntrl" | "isprint" | "ispunct" | "isxdigit" | "isblank" | "isgraph" |
        "toupper" | "tolower" |
        // <stdio.h> — common:
        "sprintf" | "fprintf" | "scanf" | "sscanf" | "puts" | "putchar" |
        "getchar" | "fopen" | "fclose" | "fread" | "fwrite" | "fgets" | "fputs" |
        // <time.h> — the runtime includes it for the `now`/`today` wall-clock fallback, so its
        // English-named public functions are now in scope. A LOGOS calendar stdlib function shares one
        // (`timegm` in `calendar.lg`); emitting `int64_t timegm(...)` clashes with glibc's declaration.
        // Mangle the whole `<time.h>` surface so no calendar name can collide now or later.
        "time" | "clock" | "difftime" | "mktime" | "timegm" | "gmtime" | "localtime" |
        "gmtime_r" | "localtime_r" | "asctime" | "asctime_r" | "ctime" | "ctime_r" |
        "strftime" | "strptime" | "timespec_get" | "clock_gettime" | "nanosleep"
    )
}

/// Every name the C tier's headers declare at file scope or define as a macro (`c_header_names_lock` reads
/// the headers and keeps the list whole).
fn c_header_names() -> &'static std::collections::HashSet<&'static str> {
    static NAMES: std::sync::OnceLock<std::collections::HashSet<&'static str>> =
        std::sync::OnceLock::new();
    NAMES.get_or_init(|| include_str!("c_header_names.txt").lines().collect())
}

pub(super) fn escape_c_ident(name: &str) -> String {
    // A name the compiler minted may hold a character no C identifier can (`apply_s1_-4`, a specialization
    // at a negative argument): each such character is spelled by its code point.
    let name: std::borrow::Cow<str> = if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        std::borrow::Cow::Borrowed(name)
    } else {
        std::borrow::Cow::Owned(
            name.chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '_' {
                        c.to_string()
                    } else {
                        format!("_u{:x}_", c as u32)
                    }
                })
                .collect(),
        )
    };
    if is_c_reserved(&name) || c_header_names().contains(name.as_ref()) {
        format!("logos_{}", name)
    } else {
        name.into_owned()
    }
}

// =============================================================================
// C Type System
// =============================================================================

/// How the integers an integer collection holds read. Each is an `int64_t`; a machine word is held as its
/// zero-extended bits, so a collection of words shares the `Int` collection's storage and runtime, and only
/// what reads an element back — its type, its display, its order — tells the two apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ints {
    Int,
    Word(u8),
}

impl Ints {
    /// How a collection of `elem` holds its integers; `None` for an element an integer collection does not hold.
    pub(super) fn of(elem: &CType) -> Option<Ints> {
        match elem {
            CType::Int64 => Some(Ints::Int),
            CType::Word(bits) => Some(Ints::Word(*bits)),
            _ => None,
        }
    }

    /// The type of one integer read back out.
    pub(super) fn ctype(self) -> CType {
        match self {
            Ints::Int => CType::Int64,
            Ints::Word(bits) => CType::Word(bits),
        }
    }

    /// The runtime renderers' flag: `1` shows each integer as a word, by its unsigned value; `0` as an `Int`.
    pub(super) fn words(self) -> u8 {
        u8::from(matches!(self, Ints::Word(_)))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum CType {
    Int64,
    Float64,
    Bool,
    String,
    /// `Seq of Int`, or of a machine word: a `Seq_i64`.
    SeqI64(Ints),
    SeqBool,
    SeqStr,
    SeqF64,
    /// `Map of K to V` over two integers: a `Map_i64_i64`, the key's and the value's reading.
    MapI64I64(Ints, Ints),
    /// `Map of Text to` an integer: a `Map_str_i64`, the value's reading.
    MapStrI64(Ints),
    MapStrStr,
    /// `Map of` an integer `to Text`: a `Map_i64_str`, the key's reading.
    MapI64Str(Ints),
    /// `Set of Int`, or of a machine word: a `Set_i64`.
    SetI64(Ints),
    SetStr,
    /// The temporal tower, all lowered to `int64_t` in C but tracked distinctly so `Show` picks the
    /// right formatter: Moment = Unix nanoseconds, Date = days since epoch, Time = nanoseconds-of-day.
    Moment,
    Date,
    Time,
    /// A physical duration: signed nanoseconds (`int64_t`).
    Duration,
    /// A calendar span: whole months and days (`LogosSpan`), applied to a Date or a Moment month-first.
    Span,
    /// Exact complex `re + im·i` — the C mirror of the interpreter's `LogosComplex`, an `LComplex`
    /// (two `LRat` over `LBig`). `+ − × ÷` stay exact and closed; `abs` gives the `f64` modulus.
    Complex,
    /// An exact rational, an `LDec` holding a ratio. The VM keeps a whole rational as the integer it equals, so `/`,
    /// `//`, `%`, `**` and the bitwise operators take a whole one as that integer (the runtime's `ldec_quotient`
    /// and kin); Display is `12` / `1/3`.
    Rational,
    /// An exact base-10 `Decimal`, an `LDec` holding a scaled integer: it keeps its digits through `+ − ×` with an
    /// integer or a decimal, and divides exactly into a rational. Display is `0.10` / `1/3`.
    Decimal,
    /// Arbitrary-precision `Int` — a tagged `LInt` (int64 fast path, `LBig` on overflow). Used only for
    /// an integer expression whose STATIC magnitude provably exceeds i64 (`10000000000 * 10000000000`)
    /// or that consumes such a value; ordinary in-range int arithmetic stays `int64_t` (fast, unchanged).
    BigInt,
    /// Exact `modular(v, n)` — ℤ/nℤ, the C mirror of `logicaffeine_base::Modular` (BigInt value +
    /// modulus). `+ − ×` wrap in the ring; the value is Euclidean-reduced to `[0, modulus)`. An `LMod`.
    Modular,
    /// A `uuid` — the C mirror of `logicaffeine_base::Uuid`, 16 big-endian bytes (`LUuid`). `Show`
    /// renders the canonical lowercase `8-4-4-4-12` string; `==` is a 16-byte compare.
    Uuid,
    /// `money(amount, "USD")` — the C mirror of `logicaffeine_base::Money`, an `LDec` amount quantised
    /// to an ISO-4217 currency's minor unit (`LMoney`). `+ −` combine, `Show` is `"{amount} {CODE}"`.
    Money,
    /// A dimensioned `quantity(v, "m")` / `5 meters` — the C mirror of `logicaffeine_base::Quantity`, an
    /// exact-Rational SI magnitude + a 10-axis dimension vector + a display unit (`LQuant`). Units
    /// resolve at codegen time; `+ − × ÷`, scalar scaling, and `in`-conversion are the dimensional algebra.
    Quantity,
    /// A value of the exact extension (an infinitesimal, a hyperreal, an algebraic number, an ordinal, … and
    /// every list of them): an `LTower`, a collected cell around a handle into the linked value runtime
    /// (see `tower`).
    Tower,
    Struct(Symbol),
    Enum(Symbol),
    /// A dynamic array of a user struct — `Seq of IntPair` → `Seq_IntPair`. The backend generates a
    /// per-struct `Seq_<Name>` (new/push/get/set/len) beside the struct's typedef.
    SeqStruct(Symbol),
    /// An anonymous tuple `(Int, Int)` — lowered to a synthesized `typedef struct { T f0; T f1; … }
    /// LTuple_…;` (positional fields `f0..fN`). Its name is dynamic (element-mangled), so it rides the
    /// `c_type_str_resolved` path like `Struct`.
    Tuple(Vec<CType>),
    /// A named record `(quotient: Int, remainder: Int)` — SHARES the positional `LTuple_…` struct (same
    /// layout), but carries the field names so `r's quotient` resolves to the positional `.fN`. Construction
    /// and item-access are identical to the tuple; only field access differs.
    Record(Vec<(Symbol, CType)>),
    /// A dynamic array of tuples — `Seq of (Int, Int)` → `Seq_LTuple_i64_i64`. Mirrors [`CType::SeqStruct`]
    /// but the element is the synthesized `LTuple_…` struct; the backend emits a `Seq_<LTuple>` helper
    /// family (new/push/get/set/len) beside the tuple's typedef.
    SeqTuple(Vec<CType>),
    /// A NESTED sequence — `Seq of (Seq of Int)` → `Seq_Seq_i64`. The element is itself a `Seq_…` (already a
    /// defined type), so the backend emits only the OUTER wrapper `typedef struct { <inner> *data; … }` +
    /// its new/push/get/set/len family. Preserves the full nesting the flat `Seq*` variants dropped to int64.
    SeqSeq(Box<CType>),
    /// A dynamic array of a VALUE TYPE — `Seq of Money`/`Seq of Quantity`/… → `Seq_LMoney` etc. The
    /// element is one of the exact-value structs (`Money`/`Quantity`/`Modular`/`Uuid`/`Complex`/`decimal`);
    /// its `Seq_<E>` family + `[a, b]` Show are synthesized in the runtime, keyed on the element C-type.
    SeqValue(Box<CType>),
    /// A `Map of Text to <value type>` — `Map_str_LMoney` etc., a string-keyed hash map whose value is
    /// one of the exact-value structs. Its `Map_str_<E>` family is synthesized in the runtime.
    MapStrValue(Box<CType>),
    /// A `Map of Int to <value>` whose value no fixed family holds — a value of the exact extension — an
    /// `Int`-keyed hash map, `Map_i64_LTower`, its `Map_i64_<E>` family synthesized as the string-keyed one is.
    MapI64Value(Box<CType>),
    /// A function value — a closure, or a named function taken as a value — of type `fn(A, B) -> C`: an
    /// `LClosure`, the code it runs (taking the closure's environment first, then `A` and `B`) and the
    /// environment holding what it captured when it was made.
    FnPtr {
        ret: Box<CType>,
        params: Vec<CType>,
    },
    /// A variable holding a value-OR-Nothing of the element it carries, the C mirror of the runtime's
    /// `logos_opt { long long present, value; }` with the value in that element's scalar packing (a Real by
    /// its bits, a Text by its pointer): what `Receive`, `Try to receive`, a select's receive arm and
    /// `Await … into` bind. `Show` renders the value when `present`, else `"nothing"` (the VM's `Nothing`
    /// from a closed pipe, an empty one, or a finished task); `x is nothing` compares presence; a read of
    /// the value is `logos_opt_int` unpacked to the element, which fails on `nothing` (see
    /// `emit::opt_var`). Only ever a variable's type.
    Opt(Box<CType>),
    /// A pipe carrying values of its element type — `Pipe of Real` — held as an `int64_t` handle like every
    /// scheduler object; the element decides how a send packs a value and what a receive binds.
    Pipe(Box<CType>),
    /// A fixed-width machine word of 8, 16, 32 or 64 bits — the ring ℤ/2ᵏℤ the VM's `RuntimeValue::Word`
    /// computes in. Held in an `int64_t` as its zero-extended bits, as the bytecode emitter's registers hold
    /// one: its `+ − ×` wrap at the width, it orders and shows unsigned, and it never mixes with an `Int`
    /// or with a word of another width (the VM's arithmetic refuses both, in its own words).
    Word(u8),
    Void,
}

pub(super) fn c_type_str(ty: &CType) -> &'static str {
    match ty {
        CType::Int64 => "int64_t",
        CType::Float64 => "double",
        CType::Bool => "bool",
        CType::String => "char *",
        CType::SeqI64(_) => "Seq_i64",
        CType::SeqBool => "Seq_bool",
        CType::SeqStr => "Seq_str",
        CType::SeqF64 => "Seq_f64",
        CType::MapI64I64(_, _) => "Map_i64_i64",
        CType::MapStrI64(_) => "Map_str_i64",
        CType::MapStrStr => "Map_str_str",
        CType::MapI64Str(_) => "Map_i64_str",
        CType::SetI64(_) => "Set_i64",
        CType::SetStr => "Set_str",
        CType::Moment | CType::Date | CType::Time | CType::Duration => "int64_t",
        CType::Span => "LogosSpan",
        CType::Complex => "LComplex",
        CType::Rational | CType::Decimal => "LDec",
        CType::BigInt => "LInt",
        CType::Modular => "LMod",
        CType::Uuid => "LUuid",
        CType::Money => "LMoney",
        CType::Quantity => "LQuant",
        CType::Tower => "LTower",
        CType::Struct(_) | CType::Enum(_) => "/* user type */",
        // Fallbacks; the precise names come from `c_type_str_resolved` (which has the interner).
        CType::SeqStruct(_) => "/* Seq of struct */",
        CType::Tuple(_) | CType::Record(_) => "/* tuple */",
        CType::SeqTuple(_) => "/* Seq of tuple */",
        CType::SeqSeq(_) => "/* Seq of Seq */",
        CType::SeqValue(_) => "/* Seq of value */",
        CType::MapStrValue(_) | CType::MapI64Value(_) => "/* Map of value */",
        CType::FnPtr { .. } => "LClosure",
        CType::Opt(_) => "logos_opt",
        CType::Pipe(_) | CType::Word(_) => "int64_t",
        CType::Void => "void",
    }
}

/// Is `ty` one of the exact VALUE-TYPE structs that gets its own `Seq_<E>` family (`Seq of Money` →
/// `Seq_LMoney`), as opposed to a scalar that rides `Seq_i64`? `BigInt` is intentionally excluded — a
/// `Seq of Int` stays `Seq_i64` (bigint promotion is a scalar-only, static-overflow concern).
pub(super) fn is_c_value_type(ty: &CType) -> bool {
    matches!(
        ty,
        CType::Money
            | CType::Quantity
            | CType::Modular
            | CType::Uuid
            | CType::Complex
            | CType::Rational
            | CType::Decimal
    )
}

/// A type usable as a string-keyed Map's VALUE via the generic `Map_str_<V>` family: the value types
/// (fixed families), a scalar or value-type Seq (`Map of Text to Seq of Int` → `Map_str_Seq_i64`) and a value
/// of the exact extension (`Map_str_LTower`).
/// The generic map family is emitted BEFORE struct definitions (so a struct can hold such a map), which
/// is only sound when the value's `Seq_…` type is already defined by then — true for scalar seqs
/// (`C_RUNTIME`) and value-type seqs, but NOT struct/nested seqs (emitted later), so those are excluded.
pub(super) fn is_c_map_value(ty: &CType) -> bool {
    is_c_value_type(ty)
        || matches!(
            ty,
            CType::SeqI64(_)
                | CType::SeqBool
                | CType::SeqStr
                | CType::SeqF64
                | CType::SeqValue(_)
                | CType::Tower
        )
}

/// The C map a `Map of K to V` lowers to, from its key and value types — the one table every map type
/// annotation, field and construction reads. A machine word keys and fills a map as an `Int` does, read back
/// as the word it was.
pub(super) fn map_ctype(key: &CType, val: &CType) -> CType {
    match (key, val, Ints::of(key), Ints::of(val)) {
        // Values that may be nothing are held as values-or-nothing, whatever they hold.
        (CType::String, opt @ CType::Opt(_), ..) => CType::MapStrValue(Box::new(opt.clone())),
        (_, opt @ CType::Opt(_), Some(Ints::Int), _) => CType::MapI64Value(Box::new(opt.clone())),
        (CType::String, CType::String, ..) => CType::MapStrStr,
        (CType::String, _, _, Some(v)) => CType::MapStrI64(v),
        (_, CType::String, Some(k), _) => CType::MapI64Str(k),
        (CType::String, v, ..) if is_c_map_value(v) => CType::MapStrValue(Box::new(v.clone())),
        (_, CType::Tower, Some(Ints::Int), _) => CType::MapI64Value(Box::new(CType::Tower)),
        (_, _, k, v) => CType::MapI64I64(k.unwrap_or(Ints::Int), v.unwrap_or(Ints::Int)),
    }
}

/// The C expression that makes an empty map of the C map type `ty`; `None` for a type that is not a map.
pub(super) fn map_new(ty: &CType, interner: &Interner) -> Option<String> {
    Some(match ty {
        CType::MapI64I64(_, _) => "map_i64_i64_new()".to_string(),
        CType::MapStrI64(_) => "map_str_i64_new()".to_string(),
        CType::MapStrStr => "map_str_str_new()".to_string(),
        CType::MapI64Str(_) => "map_i64_str_new()".to_string(),
        CType::MapStrValue(inner) => {
            format!("map_str_{}_new()", c_type_str_resolved(inner, interner))
        }
        CType::MapI64Value(inner) => {
            format!("map_i64_{}_new()", c_type_str_resolved(inner, interner))
        }
        _ => return None,
    })
}

/// The `Seq_<StructName>` C type name for a struct-seq element symbol.
pub(super) fn seq_struct_type(sym: Symbol, interner: &Interner) -> String {
    format!("Seq_{}", escape_c_ident(interner.resolve(sym)))
}

/// The synthesized C typedef name for a tuple/record type, mangled by its element types so two identical
/// tuples share one typedef (`(Int, Int)` → `LTuple_i64_i64`, `(Int, Text)` → `LTuple_i64_str`).
pub(super) fn tuple_c_name(elems: &[CType], interner: &Interner) -> String {
    let parts: Vec<String> = elems
        .iter()
        .map(|e| tuple_elem_mangle(e, interner))
        .collect();
    format!("LTuple_{}", parts.join("_"))
}
fn tuple_elem_mangle(e: &CType, interner: &Interner) -> String {
    match e {
        CType::Int64 => "i64".to_string(),
        // A temporal value is an `int64_t` like an Int, but renders as itself: its own mangle keeps
        // `(Date, Int)` a different tuple, with a different `_to_string`, from `(Int, Int)`.
        CType::Moment => "moment".to_string(),
        CType::Date => "date".to_string(),
        CType::Time => "time".to_string(),
        CType::Duration => "dur".to_string(),
        CType::Span => "span".to_string(),
        CType::BigInt => "lint".to_string(),
        CType::Float64 => "f64".to_string(),
        CType::Bool => "b".to_string(),
        CType::String => "str".to_string(),
        CType::Struct(sym) | CType::Enum(sym) => escape_c_ident(interner.resolve(*sym)),
        CType::Tuple(inner) => tuple_c_name(inner, interner),
        CType::Record(fields) => {
            let elems: Vec<CType> = fields.iter().map(|(_, t)| t.clone()).collect();
            tuple_c_name(&elems, interner)
        }
        other => c_type_str_resolved(other, interner).replace([' ', '*'], "p"),
    }
}

/// The `seq_<StructName>_<op>` helper name (op = new/push/get/set/len).
pub(super) fn seq_struct_fn(sym: Symbol, op: &str, interner: &Interner) -> String {
    format!("seq_{}_{}", escape_c_ident(interner.resolve(sym)), op)
}

/// The element type of a sequence C type; `None` for a type that is not a sequence.
pub(super) fn seq_element(ty: &CType, registry: &TypeRegistry) -> Option<CType> {
    Some(match ty {
        CType::SeqI64(ints) => ints.ctype(),
        CType::SeqBool => CType::Bool,
        CType::SeqStr => CType::String,
        CType::SeqF64 => CType::Float64,
        CType::SeqTuple(elems) => CType::Tuple(elems.clone()),
        CType::SeqSeq(inner) | CType::SeqValue(inner) => (**inner).clone(),
        CType::SeqStruct(sym) => user_type_ctype(*sym, registry),
        _ => return None,
    })
}

/// Whether `t` is a sequence C type — some `Seq_<E>`.
pub(super) fn is_seq_ctype(t: &CType) -> bool {
    matches!(
        t,
        CType::SeqI64(_)
            | CType::SeqF64
            | CType::SeqBool
            | CType::SeqStr
            | CType::SeqSeq(..)
            | CType::SeqStruct(..)
            | CType::SeqTuple(..)
            | CType::SeqValue(..)
    )
}

/// The runtime helper `op` (new/push/get/set/len) of a sequence C type. Every sequence family is named
/// after its type — `Seq_<key>` has `seq_<key>_<op>`, whether the runtime prelude defines it (`Seq_i64`,
/// `Seq_str`, …) or the program's aggregates do (`Seq_Json`, `Seq_Seq_i64`, `Seq_LTuple_…`, `Seq_LMoney`) —
/// so one derivation serves every element type. `None` for a type that is not a sequence.
pub(super) fn seq_fn(ty: &CType, op: &str, interner: &Interner) -> Option<String> {
    if !is_seq_ctype(ty) {
        return None;
    }
    let name = c_type_str_resolved(ty, interner);
    Some(format!("seq_{}_{}", name.strip_prefix("Seq_")?, op))
}

/// The generated `<Name>_to_string(<Name>) -> char*` renderer for a user struct or enum.
pub(super) fn struct_to_string_fn(sym: Symbol, interner: &Interner) -> String {
    format!("{}_to_string", escape_c_ident(interner.resolve(sym)))
}

/// The C tier can render this type inside a whole-struct `Show` (a `*_to_string` returning a fresh
/// `char*` exists for it): the scalars, the value types, and a nested struct all of whose fields are
/// themselves renderable. Collections / temporal / tuples / enums are not yet handled → `false`, which
/// makes the containing struct keep the inline field-show path. Cycle-safe via `visited`, so a boxed
/// self-referential field (not renderable in one pass) yields `false` rather than recursing forever.
pub(super) fn c_ctype_showable(
    ct: &CType,
    registry: &TypeRegistry,
    interner: &Interner,
    visited: &mut std::collections::HashSet<Symbol>,
) -> bool {
    match ct {
        CType::Int64
        | CType::Float64
        | CType::Bool
        | CType::String
        | CType::Money
        | CType::Quantity
        | CType::Modular
        | CType::Uuid
        | CType::Complex
        | CType::BigInt
        | CType::Rational
        | CType::Decimal => true,
        CType::Struct(sym) => {
            if visited.contains(sym) {
                return false;
            }
            if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = registry.get(*sym) {
                visited.insert(*sym);
                let ok = fields.iter().all(|f| {
                    c_ctype_showable(
                        &field_type_to_ctype(&f.ty, interner, registry),
                        registry,
                        interner,
                        visited,
                    )
                });
                visited.remove(sym);
                ok
            } else {
                false
            }
        }
        // A Seq field renders `[a, b, …]` via the element's `seq_<E>_to_string`. Scalar and value-type
        // elements are always showable; a Seq of structs needs the element struct to be showable too.
        CType::SeqI64(_) | CType::SeqBool | CType::SeqStr | CType::SeqF64 | CType::SeqValue(_) => true,
        CType::SeqStruct(sym) => c_ctype_showable(
            &user_type_ctype(*sym, registry),
            registry,
            interner,
            visited,
        ),
        CType::SeqSeq(inner) => c_ctype_showable(inner, registry, interner, visited),
        // A tuple / record (`(a, b)`) renders `(…)` via `<LTuple>_to_string`; a Seq of tuples via
        // `seq_<LTuple>_to_string`. Showable when every element is.
        CType::Tuple(elems) | CType::SeqTuple(elems) => elems
            .iter()
            .all(|e| c_ctype_showable(e, registry, interner, visited)),
        CType::Record(fields) => fields
            .iter()
            .all(|(_, t)| c_ctype_showable(t, registry, interner, visited)),
        // An enum renders `Variant(a, …)` via its `<Name>_to_string`. A recursive reference to an
        // enum already being checked renders through that same function (its field is boxed).
        CType::Enum(sym) => visited.contains(sym) || c_enum_showable(*sym, registry, interner),
        // Map/Set fields render `{k: v, …}` / `{a, …}` in insertion order via their `*_to_string`.
        CType::MapI64I64(_, _)
        | CType::MapStrI64(_)
        | CType::MapStrStr
        | CType::MapI64Str(_)
        | CType::MapStrValue(_)
        | CType::MapI64Value(_)
        | CType::SetI64(_)
        | CType::SetStr => true,
        _ => false,
    }
}

/// The element type of a `Seq_<Name>` — a record or an enum, as the registry declares `sym`.
pub(super) fn user_type_ctype(sym: Symbol, registry: &TypeRegistry) -> CType {
    match registry.get(sym) {
        Some(crate::analysis::TypeDef::Enum { .. }) => CType::Enum(sym),
        _ => CType::Struct(sym),
    }
}

/// Whether a value of the record or enum `sym` is C-renderable.
pub(super) fn c_user_type_showable(
    sym: Symbol,
    registry: &TypeRegistry,
    interner: &Interner,
) -> bool {
    match registry.get(sym) {
        Some(crate::analysis::TypeDef::Enum { .. }) => c_enum_showable(sym, registry, interner),
        _ => c_struct_showable(sym, registry, interner),
    }
}

/// The sequence type holding elements of `elem` — the ONE answer to "what is a `Seq of X`" that
/// every place typing one asks: a record or an enum → its `Seq_<Name>` family, a tuple or named
/// record → `Seq_<LTuple…>`, a value type → its `Seq_<V>` family, a sequence → a nested
/// `Seq_Seq_…`, the scalars → their runtime families, and anything else the `Int` default.
pub(super) fn seq_of(elem: CType) -> CType {
    match elem {
        // A list of exact values is one exact value: the runtime's list.
        CType::Tower => CType::Tower,
        CType::Bool => CType::SeqBool,
        CType::String => CType::SeqStr,
        CType::Float64 => CType::SeqF64,
        CType::Struct(sym) | CType::Enum(sym) => CType::SeqStruct(sym),
        CType::Tuple(el) => CType::SeqTuple(el),
        CType::Record(fields) => CType::SeqTuple(fields.into_iter().map(|(_, t)| t).collect()),
        seq @ (CType::SeqI64(_)
        | CType::SeqBool
        | CType::SeqStr
        | CType::SeqF64
        | CType::SeqValue(_)
        | CType::SeqStruct(_)
        | CType::SeqTuple(_)
        | CType::SeqSeq(_)
        | CType::MapI64I64(..)
        | CType::MapStrI64(_)
        | CType::MapStrStr
        | CType::MapI64Str(_)
        | CType::MapStrValue(_)
        | CType::MapI64Value(_)
        | CType::SetI64(_)
        | CType::SetStr) => CType::SeqSeq(Box::new(seq)),
        v if is_c_value_type(&v) => CType::SeqValue(Box::new(v)),
        // Items that may be nothing are held as values-or-nothing, whatever they hold.
        opt @ CType::Opt(_) => CType::SeqValue(Box::new(opt)),
        other => CType::SeqI64(Ints::of(&other).unwrap_or(Ints::Int)),
    }
}

/// The item type of a list literal: its first item's, or a rational where integers sit beside rationals, each
/// integer held as the rational it equals.
pub(super) fn list_item_type(elems: &[&Expr], ctx: &CContext) -> CType {
    let types: Vec<CType> = elems.iter().map(|e| infer_expr_type(e, ctx)).collect();
    let integer = |t: &CType| matches!(t, CType::Int64 | CType::BigInt);
    if types.contains(&CType::Rational) && types.iter().all(|t| integer(t) || *t == CType::Rational) {
        return CType::Rational;
    }
    types.into_iter().next().unwrap_or(CType::Int64)
}

/// The set type holding elements of `elem`: a `Set_str` of texts, and a `Set_i64` of anything else, read as
/// the integers or words it holds.
pub(super) fn set_of(elem: CType) -> CType {
    match elem {
        CType::String => CType::SetStr,
        other => CType::SetI64(Ints::of(&other).unwrap_or(Ints::Int)),
    }
}

/// A user enum whose values are C-renderable: every variant's every field is [`c_ctype_showable`],
/// a field naming the enum itself (boxed) included.
pub(super) fn c_enum_showable(sym: Symbol, registry: &TypeRegistry, interner: &Interner) -> bool {
    let mut visited = std::collections::HashSet::new();
    visited.insert(sym);
    if let Some(crate::analysis::TypeDef::Enum { variants, .. }) = registry.get(sym) {
        variants.iter().all(|v| {
            v.fields.iter().all(|f| {
                c_ctype_showable(
                    &field_type_to_ctype(&f.ty, interner, registry),
                    registry,
                    interner,
                    &mut visited,
                )
            })
        })
    } else {
        false
    }
}

/// A user struct whose whole-value `Show` is C-renderable (every field is [`c_ctype_showable`]).
pub(super) fn c_struct_showable(sym: Symbol, registry: &TypeRegistry, interner: &Interner) -> bool {
    let mut visited = std::collections::HashSet::new();
    visited.insert(sym);
    if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = registry.get(sym) {
        fields.iter().all(|f| {
            c_ctype_showable(
                &field_type_to_ctype(&f.ty, interner, registry),
                registry,
                interner,
                &mut visited,
            )
        })
    } else {
        false
    }
}

/// The C expression — a freshly `malloc`'d `char*` the caller must `free` — rendering ONE struct
/// field's value. Gated by [`c_struct_showable`], so the `_` arm is never reached in practice.
pub(super) fn c_field_to_string_expr(ct: &CType, access: &str, interner: &Interner) -> String {
    match ct {
        CType::Int64 => format!("logos_dyn_sprintf(\"%lld\", (long long)({access}))"),
        CType::Word(_) => format!("logos_dyn_sprintf(\"%\" PRIu64, (uint64_t)({access}))"),
        CType::Float64 => format!("logos_dyn_sprintf(\"%g\", {access})"),
        CType::Bool => format!("lstrdup({access} ? \"true\" : \"false\")"),
        CType::String => format!("lstrdup({access} ? {access} : \"\")"),
        CType::Money => format!("money_to_string({access})"),
        CType::Quantity => format!("quant_to_string({access})"),
        CType::Modular => format!("mod_to_string({access})"),
        CType::Uuid => format!("uuid_to_string({access})"),
        CType::Complex => format!("cplx_to_string({access})"),
        CType::BigInt => format!("lint_to_string({access})"),
        CType::Rational | CType::Decimal => format!("ldec_to_string(&{access})"),
        CType::Tower => format!("ltower_to_string({access})"),
        CType::Struct(inner) | CType::Enum(inner) => {
            format!("{}({access})", struct_to_string_fn(*inner, interner))
        }
        // Seq fields — each renders `[a, b, …]` via the element's `seq_<E>_to_string` (takes a pointer).
        CType::SeqI64(Ints::Int) => format!("seq_i64_to_string(&{access})"),
        CType::SeqI64(ints) => format!("seq_i64_to_string_as(&{access}, {})", ints.words()),
        CType::SeqBool => format!("seq_bool_to_string(&{access})"),
        CType::SeqStr => format!("seq_str_to_string(&{access})"),
        CType::SeqF64 => format!("seq_f64_to_string(&{access})"),
        // Items that may be nothing render as their payload does, `nothing` where absent.
        CType::SeqValue(inner) if matches!(**inner, CType::Opt(_)) => {
            let item = c_field_to_string_expr(inner, "__seq.data[__i]", interner);
            format!(
                "({{ Seq_logos_opt __seq = {access}; char *__out = lc_gc_strdup(\"[\"); \
                 for (size_t __i = 0; __i < __seq.len; __i++) {{ char *__item = {item}; \
                 char *__next = logos_dyn_sprintf(\"%s%s%s\", __out, __i ? \", \" : \"\", __item); __out = __next; }} \
                 str_concat(__out, \"]\"); }})"
            )
        }
        CType::SeqValue(inner) => format!("seq_{}_to_string(&{access})", c_type_str(inner)),
        CType::SeqStruct(sym) => format!(
            "seq_{}_to_string(&{access})",
            escape_c_ident(interner.resolve(*sym))
        ),
        CType::SeqSeq(inner) => format!(
            "seq_{}_to_string(&{access})",
            c_type_str_resolved(inner, interner)
        ),
        // Map/Set fields — insertion-ordered `{…}` via their `*_to_string` (takes a pointer).
        CType::MapI64I64(Ints::Int, Ints::Int) => format!("map_i64_i64_to_string(&{access})"),
        CType::MapI64I64(key, val) => format!(
            "map_i64_i64_to_string_as(&{access}, {}, {})",
            key.words(),
            val.words()
        ),
        CType::MapStrI64(Ints::Int) => format!("map_str_i64_to_string(&{access})"),
        CType::MapStrI64(val) => {
            format!("map_str_i64_to_string_as(&{access}, {})", val.words())
        }
        CType::MapStrStr => format!("map_str_str_to_string(&{access})"),
        CType::MapI64Str(Ints::Int) => format!("map_i64_str_to_string(&{access})"),
        CType::MapI64Str(key) => {
            format!("map_i64_str_to_string_as(&{access}, {})", key.words())
        }
        // Values that may be nothing render as their payload does, `nothing` where absent, in insertion order.
        CType::MapStrValue(inner) | CType::MapI64Value(inner) if matches!(**inner, CType::Opt(_)) => {
            let (map, key) = if matches!(ct, CType::MapStrValue(_)) {
                ("Map_str_logos_opt", "lc_gc_strdup(__map.keys[__si])")
            } else {
                ("Map_i64_logos_opt", "logos_dyn_sprintf(\"%\" PRId64, __map.keys[__si])")
            };
            let value = c_field_to_string_expr(inner, "__map.vals[__si]", interner);
            format!(
                "({{ {map} __map = {access}; char *__out = lc_gc_strdup(\"{{\"); int __first = 1; \
                 for (size_t __oi = 0; __oi < __map.ord_len; __oi++) {{ size_t __si = __map.order[__oi]; \
                 if (!__map.state[__si]) continue; char *__k = {key}; char *__v = {value}; \
                 char *__next = logos_dyn_sprintf(\"%s%s%s: %s\", __out, __first ? \"\" : \", \", __k, __v); \
                 __out = __next; __first = 0; }} str_concat(__out, \"}}\"); }})"
            )
        }
        CType::MapStrValue(inner) => format!(
            "map_str_{}_to_string(&{access})",
            c_type_str_resolved(inner, interner)
        ),
        CType::MapI64Value(inner) => format!(
            "map_i64_{}_to_string(&{access})",
            c_type_str_resolved(inner, interner)
        ),
        CType::SetI64(Ints::Int) => format!("set_i64_to_string(&{access})"),
        CType::SetI64(ints) => format!("set_i64_to_string_as(&{access}, {})", ints.words()),
        CType::SetStr => format!("set_str_to_string(&{access})"),
        // Tuple / record field → `<LTuple>_to_string(v)` (by value); a Seq-of-tuple field → its seq form.
        CType::Tuple(elems) => format!("{}_to_string({access})", tuple_c_name(elems, interner)),
        CType::Record(fields) => {
            let elems: Vec<CType> = fields.iter().map(|(_, t)| t.clone()).collect();
            format!("{}_to_string({access})", tuple_c_name(&elems, interner))
        }
        CType::SeqTuple(elems) => {
            format!("seq_{}_to_string(&{access})", tuple_c_name(elems, interner))
        }
        CType::Moment => format!("logos_moment_to_string({access})"),
        CType::Date => format!("logos_date_to_string({access})"),
        CType::Time => format!("logos_format_time_of_day({access})"),
        CType::Duration => format!("logos_duration_to_string({access})"),
        CType::Span => format!("logos_span_to_string({access})"),
        CType::Opt(inner) => {
            let shown = match &**inner {
                CType::Float64 => format!("logos_fmt_f64(logos_bits_real(({access}).value))"),
                other => c_field_to_string_expr(
                    other,
                    &opt_unpack(other, &format!("({access}).value"), interner),
                    interner,
                ),
            };
            format!("(({access}).present ? {shown} : lstrdup(\"nothing\"))")
        }
        CType::Pipe(_) => format!("logos_dyn_sprintf(\"%lld\", (long long)({access}))"),
        CType::FnPtr { .. } => "lstrdup(\"<closure>\")".to_string(),
        // A value this backend could not type. Refused, never rendered as a guess.
        CType::Void => {
            "\n#error \"the C backend cannot type this value, so it cannot render it\"\n"
                .to_string()
        }
    }
}

/// Whether a value of `elem` rides in a task's or a value-or-nothing's scalar cell itself (a Real by its bits, a
/// Text by its pointer, an integer, a machine word or anything else of a word's size as itself) — the one table a
/// launch, a send and a receive all read; any other value rides there boxed, by the address of a copy of its own.
pub(super) fn opt_holds_in_cell(elem: &CType) -> bool {
    matches!(
        elem,
        CType::Int64
            | CType::Word(_)
            | CType::Float64
            | CType::Bool
            | CType::String
            | CType::Moment
            | CType::Date
            | CType::Time
            | CType::Duration
            | CType::Pipe(_)
    )
}

/// The value of element type `elem` a value-or-nothing's scalar `slot` holds.
pub(super) fn opt_unpack(elem: &CType, slot: &str, interner: &Interner) -> String {
    match elem {
        CType::Float64 => format!("logos_bits_real({slot})"),
        e if opt_holds_in_cell(e) => format!("(({}){slot})", c_type_str_resolved(e, interner)),
        e => format!(
            "(*({} *)(intptr_t)({slot}))",
            c_type_str_resolved(e, interner)
        ),
    }
}

/// Declare a parameter/field of type `ty` named `name`.
pub(super) fn c_param_decl(ty: &CType, name: &str, interner: &Interner) -> String {
    format!("{} {}", c_type_str_resolved(ty, interner), name)
}

/// A `mutable` parameter: the caller's variable, passed by reference, so the callee's writes reach it.
pub(super) fn c_inout_param_decl(ty: &CType, name: &str, interner: &Interner) -> String {
    format!("{} *{}", c_type_str_resolved(ty, interner), name)
}

/// Whether a declared parameter type is `mutable` (passed by reference).
pub(super) fn is_inout(declared: &TypeExpr) -> bool {
    matches!(declared, TypeExpr::Mutable { .. })
}

/// The C type of the code a function value of type `fn(params) -> ret` runs: its environment, then its
/// parameters.
pub(super) fn closure_code_type(ret: &CType, params: &[CType], interner: &Interner) -> String {
    let ps: String = params
        .iter()
        .map(|p| format!(", {}", c_type_str_resolved(p, interner)))
        .collect();
    format!("{} (*)(void *{ps})", c_type_str_resolved(ret, interner))
}

pub(super) fn c_type_str_resolved(ty: &CType, interner: &Interner) -> String {
    match ty {
        CType::Struct(sym) | CType::Enum(sym) => escape_c_ident(interner.resolve(*sym)),
        CType::SeqStruct(sym) => seq_struct_type(*sym, interner),
        CType::Tuple(elems) => tuple_c_name(elems, interner),
        CType::Record(fields) => {
            let elems: Vec<CType> = fields.iter().map(|(_, t)| t.clone()).collect();
            tuple_c_name(&elems, interner)
        }
        CType::SeqTuple(elems) => format!("Seq_{}", tuple_c_name(elems, interner)),
        CType::SeqSeq(inner) => format!("Seq_{}", c_type_str_resolved(inner, interner)),
        // `Seq of Money` → `Seq_LMoney` etc. (the value-type element's C struct name).
        CType::SeqValue(inner) => format!("Seq_{}", c_type_str(inner)),
        // `Map of Text to Money` → `Map_str_LMoney`.
        CType::MapStrValue(inner) => format!("Map_str_{}", c_type_str_resolved(inner, interner)),
        // `Map of Int to Hyperreal` → `Map_i64_LTower`.
        CType::MapI64Value(inner) => format!("Map_i64_{}", c_type_str_resolved(inner, interner)),
        _ => c_type_str(ty).to_string(),
    }
}

/// The C type a built-in type NAME lowers to — the one table a field declaration and a written
/// annotation both read. `None` for a name that is not built in (a user type, or unknown).
fn named_ctype(name: &str) -> Option<CType> {
    Some(match name {
        "Int" | "Nat" => CType::Int64,
        "Float" | "Real" => CType::Float64,
        "Bool" => CType::Bool,
        "Text" => CType::String,
        "Money" => CType::Money,
        "Quantity" => CType::Quantity,
        "Modular" => CType::Modular,
        "Uuid" => CType::Uuid,
        "Complex" | "ComplexF" => CType::Complex,
        "Rational" => CType::Rational,
        "Decimal" => CType::Decimal,
        "Date" => CType::Date,
        "Moment" => CType::Moment,
        "Time" => CType::Time,
        "Duration" => CType::Duration,
        "Span" => CType::Span,
        "Word8" => CType::Word(8),
        "Word16" => CType::Word(16),
        "Word32" => CType::Word(32),
        "Word64" => CType::Word(64),
        name if super::tower::TYPE_NAMES.contains(&name) => CType::Tower,
        _ => return None,
    })
}

pub(super) fn field_type_to_ctype(
    ft: &crate::analysis::FieldType,
    interner: &Interner,
    registry: &TypeRegistry,
) -> CType {
    match ft {
        crate::analysis::FieldType::Primitive(sym) | crate::analysis::FieldType::Named(sym) => {
            match named_ctype(interner.resolve(*sym)) {
                Some(ct) => ct,
                None => match registry.get(*sym) {
                    Some(crate::analysis::TypeDef::Struct { .. }) => CType::Struct(*sym),
                    Some(crate::analysis::TypeDef::Enum { .. }) => CType::Enum(*sym),
                    _ => CType::Int64,
                },
            }
        }
        crate::analysis::FieldType::Generic { base, params } => {
            match interner.resolve(*base) {
                // A `SharedSequence of T` is an RGA CRDT; on the single replica it is exactly a `Seq of T`
                // (append = push, show = the list), so it lowers to the same C `Seq_*`.
                "Seq" | "List" | "Vec" | "SharedSequence" => match params.first() {
                    Some(elem) => seq_of(field_type_to_ctype(elem, interner, registry)),
                    None => CType::SeqI64(Ints::Int),
                },
                // A `Pair of A and B` or a `Triple of A, B and C` is the tuple of its parts.
                "Pair" | "Triple" => CType::Tuple(
                    params
                        .iter()
                        .map(|p| field_type_to_ctype(p, interner, registry))
                        .collect(),
                ),
                "Map" | "HashMap" => {
                    let key = params
                        .first()
                        .map(|p| field_type_to_ctype(p, interner, registry))
                        .unwrap_or(CType::Int64);
                    let val = params
                        .get(1)
                        .map(|p| field_type_to_ctype(p, interner, registry))
                        .unwrap_or(CType::Int64);
                    map_ctype(&key, &val)
                }
                "Set" | "HashSet" => match params.first() {
                    Some(elem) => set_of(field_type_to_ctype(elem, interner, registry)),
                    None => CType::SetI64(Ints::Int),
                },
                // A `Divergent T` (MVRegister LWW) on the single replica is just a `T` cell — the last write
                // wins, so `Resolve` is a plain field assign and `Show` renders the inner value.
                "Divergent" => params
                    .first()
                    .map(|p| field_type_to_ctype(p, interner, registry))
                    .unwrap_or(CType::Int64),
                _ => CType::Int64,
            }
        }
        crate::analysis::FieldType::TypeParam(_) => CType::Int64,
        crate::analysis::FieldType::Tuple(items) => CType::Tuple(
            items
                .iter()
                .map(|t| field_type_to_ctype(t, interner, registry))
                .collect(),
        ),
        crate::analysis::FieldType::Record(fields) => CType::Record(
            fields
                .iter()
                .map(|(n, t)| (*n, field_type_to_ctype(t, interner, registry)))
                .collect(),
        ),
    }
}

/// A set's element type — what a loop over it binds — for a set the C tier iterates by bucket.
pub(super) fn set_element(ty: &CType) -> Option<CType> {
    match ty {
        CType::SetI64(ints) => Some(ints.ctype()),
        CType::SetStr => Some(CType::String),
        _ => None,
    }
}

/// A map's key and value types — the two halves of each entry a loop over it binds — for a map the C tier
/// iterates by bucket.
pub(super) fn map_entry_types(ty: &CType) -> Option<(CType, CType)> {
    match ty {
        CType::MapI64I64(key, val) => Some((key.ctype(), val.ctype())),
        CType::MapStrI64(val) => Some((CType::String, val.ctype())),
        CType::MapStrStr => Some((CType::String, CType::String)),
        CType::MapI64Str(key) => Some((key.ctype(), CType::String)),
        _ => None,
    }
}

pub(super) fn resolve_type_expr(ty: &TypeExpr, interner: &Interner) -> CType {
    resolve_type_expr_with_registry(ty, interner, None)
}

pub(super) fn resolve_type_expr_with_registry(
    ty: &TypeExpr,
    interner: &Interner,
    registry: Option<&TypeRegistry>,
) -> CType {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            match named_ctype(interner.resolve(*sym)) {
                Some(ct) => ct,
                None => match registry.map(|reg| reg.get(*sym)) {
                    Some(Some(crate::analysis::TypeDef::Struct { .. })) => CType::Struct(*sym),
                    Some(Some(crate::analysis::TypeDef::Enum { .. })) => CType::Enum(*sym),
                    _ => CType::Int64,
                },
            }
        }
        TypeExpr::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            match base_name {
                "Seq" | "List" | "Vec" => {
                    if let Some(elem) = params.first() {
                        // Registry-aware so a struct element (`Seq of IntPair`) resolves to
                        // `Struct(sym)` rather than the no-registry Int64 default.
                        seq_of(resolve_type_expr_with_registry(elem, interner, registry))
                    } else {
                        CType::SeqI64(Ints::Int)
                    }
                }
                // A `Pair of A and B` or a `Triple of A, B and C` is the tuple of its parts.
                "Pair" | "Triple" => CType::Tuple(
                    params
                        .iter()
                        .map(|p| resolve_type_expr_with_registry(p, interner, registry))
                        .collect(),
                ),
                "Map" | "HashMap" => {
                    let key_type = params
                        .first()
                        .map(|p| resolve_type_expr(p, interner))
                        .unwrap_or(CType::Int64);
                    let val_type = params
                        .get(1)
                        .map(|p| resolve_type_expr(p, interner))
                        .unwrap_or(CType::Int64);
                    map_ctype(&key_type, &val_type)
                }
                "Pipe" => CType::Pipe(Box::new(params.first().map_or(CType::Int64, |elem| {
                    resolve_type_expr_with_registry(elem, interner, registry)
                }))),
                "Set" | "HashSet" => {
                    if let Some(elem) = params.first() {
                        set_of(resolve_type_expr(elem, interner))
                    } else {
                        CType::SetI64(Ints::Int)
                    }
                }
                _ => CType::Int64,
            }
        }
        // A function value's integers are exact: whatever calls it, an argument or a result past `i64` promotes,
        // as the VM's does.
        TypeExpr::Function { inputs, output } => {
            let mut f = CType::FnPtr {
                ret: Box::new(resolve_type_expr_with_registry(output, interner, registry)),
                params: inputs
                    .iter()
                    .map(|t| resolve_type_expr_with_registry(t, interner, registry))
                    .collect(),
            };
            super::exactify_int_ctype(&mut f);
            f
        }
        // An anonymous tuple / named record → a synthesized positional-field struct `LTuple_…`.
        TypeExpr::Tuple(params) => CType::Tuple(
            params
                .iter()
                .map(|t| resolve_type_expr_with_registry(t, interner, registry))
                .collect(),
        ),
        TypeExpr::Record {
            field_names,
            field_types,
        } => CType::Record(
            field_names
                .iter()
                .zip(field_types.iter())
                .map(|(n, t)| (*n, resolve_type_expr_with_registry(t, interner, registry)))
                .collect(),
        ),
        // A `mutable` parameter's value is its inner type's; it passes by reference (`c_inout_param_decl`).
        TypeExpr::Mutable { inner } => resolve_type_expr_with_registry(inner, interner, registry),
        _ => CType::Int64,
    }
}

// =============================================================================
// Codegen Context
// =============================================================================

pub(super) struct CContext<'a> {
    pub(super) vars: HashMap<Symbol, CType>,
    /// The program's reach of the exact extension (see `tower`), or `None` when it reaches nothing.
    pub(super) tower: Option<std::rc::Rc<crate::analysis::tower::TowerTypes>>,
    /// The function whose body is being emitted, or `None` for the program's root: where the reach is read.
    pub(super) tower_scope: crate::analysis::tower::Scope,
    /// Where a stored `nothing` reaches in the program (`analysis::absence`): what holds value-or-nothing.
    pub(super) absence: std::rc::Rc<crate::analysis::absence::Absence>,
    pub(super) funcs: HashMap<Symbol, CType>,
    /// The `## To native …` declarations among `funcs` — bound to their engine implementation by
    /// name, so a builtin arm still answers for them. Every OTHER entry of `funcs` is a function
    /// the program defines, which no builtin arm may take (declarer wins).
    pub(super) native_funcs: std::collections::HashSet<Symbol>,
    pub(super) interner: &'a Interner,
    pub(super) registry: &'a TypeRegistry,
    /// Lambda-lifting: a closure argument (`mapSeq((n) -> n*2, …)`) is lowered to a top-level
    /// `static` C function pushed here, and the argument becomes its name. Emitted before `main`.
    /// `RefCell`/`Cell` so `codegen_expr(&ctx)` can accumulate through the shared reference.
    pub(super) lifted: std::cell::RefCell<Vec<String>>,
    pub(super) lambda_ctr: std::cell::Cell<usize>,
    /// Locals (in the function being emitted) that must be declared `LInt` because they accumulate a
    /// value the Oracle cannot bound to i64 (a self-referential product/growth, or a statically-huge
    /// literal, or a copy of another such value). Recomputed per function by the BigInt pre-pass.
    pub(super) bigint_vars: std::collections::HashSet<Symbol>,
    /// The scope's locals whose bindings give them numbers of different forms: the C type each is declared with,
    /// and those no C type holds ([`super::numbers`]).
    pub(super) number_locals: super::numbers::NumberLocals,
    /// Variables the shared whole-body analysis proves always non-negative.
    /// Direct C consumes the same proof as Rust for power-of-two mask/shift
    /// lowering, including exact integers' guarded small representation.
    pub(super) nonnegative_vars: std::collections::HashSet<Symbol>,
    /// The C type the function currently being emitted returns: a `Return` hands back a value of it — a raw
    /// int64 promoted where it is `LInt`, a tuple's components each so.
    pub(super) current_fn_returns: CType,
    /// One frame per open C block, pushed on entering a braced block and popped on exit: the names the
    /// block declares, and what each meant before. A `Let` that binds a name the CURRENT block already
    /// declared with the same type assigns that variable (an unrolled loop's duplicated `Let dx …`); with
    /// another type it declares a variable of its own under a fresh C name, since C allows one
    /// declaration of a name per block. Closing the block gives every name it bound back its outer type
    /// and C name.
    pub(super) decl_scopes: Vec<DeclScope>,
    /// The C name a value currently answers to where it is not its own spelling: a rebinding of another
    /// type in a block that already declared the name (see [`CContext::declare`]).
    pub(super) renamed: HashMap<Symbol, String>,
    /// Mints the fresh C names of `renamed`.
    pub(super) rename_ctr: std::cell::Cell<usize>,
    /// The frame of the stepped function being lowered, when the program's tasks run on the stepped tier:
    /// every binding and temporary of that function is one of its fields, so a resume may land anywhere in
    /// the body.
    pub(super) stepped: Option<Frame>,
    /// The functions that run as step functions (see `stepped::stepped_functions`): a call to one from a
    /// stepped body pushes its frame instead of calling it. Empty unless the program runs on the stepped tier.
    pub(super) stepped_fns: std::collections::HashSet<Symbol>,
    /// Expressions already computed into a cell ahead of the expression that reads them, keyed by the node:
    /// `codegen_expr` reads each as its cell. The stepped tier runs calls ahead of their statement; an
    /// operator over a value-or-nothing reads the value once it knows it is there.
    pub(super) precomputed: std::cell::RefCell<HashMap<usize, String>>,
    /// The operands, by address, an expression being lowered reads as the value a value-or-nothing holds (its cell
    /// among `precomputed`), with that value's type.
    pub(super) unpacked: std::cell::RefCell<HashMap<usize, CType>>,
    /// Whether this program uses the deterministic concurrency runtime — set once by
    /// `codegen_program_c`. Lets a concurrency statement lower to a `logos_*` scheduler call instead of
    /// the `/* unsupported stmt */` stub; false for every non-concurrency program (no behavior change).
    pub(super) uses_concurrency: bool,
    /// A monotonic counter minting unique C identifiers for the compiler's temporaries — a `Select` arm's
    /// slot (`__selslot_<n>`), a counted loop's bounds (`__lo<n>`, `__hi<n>`) — so nested or repeated
    /// constructs in one function never collide.
    pub(super) select_ctr: std::cell::Cell<usize>,
    /// Each launched function's parameter types, by which a launch packs its arguments into the task's
    /// scalar array and the function's task entry unpacks them. Empty unless the program uses concurrency.
    pub(super) task_params: HashMap<Symbol, Vec<CType>>,
    /// The program's bindings a function reaches: each `## Main` root `Let` a `## To` body names (see
    /// `program_statics`). Each is a file-scope static, so a function reads and writes the program's binding
    /// while its own locals and parameters shadow it and its callers' locals stay out of sight — the
    /// tree-walker's frames. Empty when no function names one of Main's bindings (the C is then unchanged).
    pub(super) program_statics: std::collections::HashSet<Symbol>,
    /// The C type each program static takes, recorded at its root `Let` while Main is lowered.
    pub(super) static_types: HashMap<Symbol, CType>,
    /// The scope depth of Main's root while Main is lowered, else `None`: a `Let` there of a program static
    /// assigns the static.
    pub(super) main_root_depth: Option<usize>,
    /// Every `(struct symbol, field symbol)` whose declared type is a counter CRDT (`ConvergentCount`/
    /// `Tally`). Such a field's C cell holds a `logos_crdt*` handle: a `New` struct seeds it with
    /// `logos_crdt_new()`, `FieldAccess` on it projects to the map SUM, `Increase`/`Decrease` bump the
    /// replica slot, and `Merge`/`Join` take the per-replica MAX. Empty for every non-CRDT program (so
    /// codegen is byte-identical to a world without CRDTs).
    pub(super) counter_fields: std::collections::HashSet<(Symbol, Symbol)>,
    /// Names of EXACT higher-order functions — those taking a `fn`-typed (closure) param but NO sequence param
    /// (a scalar HOF like `apply(f, x)`). Their integer params/return + closure param are exact `LInt`, so a
    /// closure that overflows (`(n) -> n*2` on `i64::MAX`) promotes instead of wrapping — SELECTIVELY, since a
    /// seq-HOF (`map`/`fold`/`reduceSeq`) operates on i64 seqs and must NOT be exactified (it would break passing
    /// an i64 element to an `LInt` closure param). Settled before any signature is emitted; drives call-site arg promotion.
    pub(super) exact_hofs: std::collections::HashSet<Symbol>,
    /// The program's exact-integer slots (see `exact`): which returns, parameters and fields are `LInt`, and
    /// each body's `LInt` locals. A closure's context and a collector's carry the program's copy.
    pub(super) exact: super::exact::ExactSlots,
    /// Each user function's parameter C types as its definition declares them, `LInt` where exact — a call
    /// promotes a machine-word argument to a parameter that is `LInt`.
    pub(super) fn_params: HashMap<Symbol, Vec<CType>>,
    /// Each user function's `mutable` parameter positions: an argument there passes by reference.
    pub(super) fn_inout: HashMap<Symbol, Vec<bool>>,
    /// The texts a `mutable` parameter lends, by name: each such parameter and every name a call lends to one.
    /// Caller and callee hold one string, so it is a plain one on both sides, never a builder (`builds_text`).
    pub(super) plain_texts: std::collections::HashSet<String>,
    /// The program's security-policy registry (predicate/capability definitions), used by a `Check`
    /// statement to emit its guard condition inline. Defaults to an empty registry (via
    /// `empty_policy_registry`) so the helper/child contexts — which only emit `Show`/type inference,
    /// never a `Check` — cost nothing; `codegen_program_c` sets the real one on the top-level context.
    pub(super) policies: &'a PolicyRegistry,
    /// Each monomorphized instance type → the generic type it applies. A value shows under its
    /// generic's name (a `Box of Int` instance shows as `Box { … }`), as on every other tier.
    /// Empty for a program with no instance.
    pub(super) display_names: HashMap<Symbol, Symbol>,
    /// The loops and zones enclosing the statement being lowered, innermost last: what a `Break` or a
    /// `Return` leaves. Empty at the top of a body.
    pub(super) flow: Vec<Flow>,
    /// The copying functions generated so far (see `values::copy_value`), each once.
    pub(super) copy_fns: std::cell::RefCell<std::collections::HashSet<String>>,
    /// The comparing functions generated so far (see `equality`), each once.
    pub(super) equal_fns: std::cell::RefCell<std::collections::HashSet<String>>,
    /// Every name the body being lowered changes in place (`values::mutated_roots`): a binding of a list, map,
    /// set or record to or from one of them takes a copy of its own.
    pub(super) mutated: std::collections::HashSet<Symbol>,
    /// The program statics some body changes in place: changed, for every body that reads one.
    pub(super) changed_statics: std::collections::HashSet<Symbol>,
    /// The names a zone or a spawned agent binds to nothing that the program reads or sets by name (see
    /// `nothing_names`): each gets a variable holding nothing.
    pub(super) nothing_names: std::collections::HashSet<Symbol>,
    /// Whether the body being lowered is `## Main`'s, where a `Return` or a `Break` outside every loop and
    /// zone ends the run as finished.
    pub(super) in_main: bool,
    /// Which parameters each function changes in place (see `sharing`): its callers copy those arguments.
    pub(super) mutating: std::rc::Rc<super::sharing::MutatingParams>,
    /// Which arguments each function's result may share a buffer with (see `values::CallShapes`).
    pub(super) call_shapes: std::rc::Rc<super::values::CallShapes>,
    /// The functions a call to which neither fails nor acts (see `quiet_functions`).
    pub(super) quiet_fns: std::rc::Rc<std::collections::HashSet<Symbol>>,
    /// The integer ranges the abstract interpreter proves for the program's expressions: an operand it bounds
    /// needs no overflow check.
    pub(super) int_oracle: Option<std::rc::Rc<crate::optimize::OracleFacts>>,
    /// The sharing and liveness of the body being lowered, when the analysis covers it; a body without them
    /// copies every value it binds or hands on that a change could reach.
    pub(super) ownership: Option<std::rc::Rc<Ownership>>,
    /// Fixed-size, non-escaping scalar sequences proved by the shared AOT
    /// scalarization analyses.  The C backend keeps its ordinary `Seq_*` API
    /// for these values, but points it at a block-local backing array instead
    /// of growing a heap buffer.  Keying this by the parser's unique symbol
    /// makes nested scopes and same-spelled bindings unambiguous.
    pub(super) fixed_stack_seqs: HashMap<Symbol, usize>,
    /// Read-only text_bytes(source) bindings proved safe by the shared
    /// TextBytesView analysis. The binding keeps its semantic Seq-of-Int type,
    /// while C lowers its storage to a borrowed unsigned-char pointer.
    pub(super) text_bytes_candidates: HashMap<usize, Symbol>,
    /// Borrowed representations activated by emitted declarations in the current scope.
    pub(super) text_bytes_views: HashMap<Symbol, Symbol>,
    /// The statements being lowered, innermost last: what an argument's copy is decided against.
    pub(super) stmt_frames: std::cell::RefCell<Vec<StmtFrame>>,
}

/// One body's facts: which names may share a buffer and which are read again (deciding copies), and the range
/// each integer name holds (deciding which integer arithmetic provably stays within a machine word).
pub(super) struct Ownership {
    pub(super) sharing: super::sharing::Sharing,
    pub(super) live: super::sharing::Liveness,
    pub(super) words: super::word_ranges::WordRanges,
    /// Fresh, function-local buffers whose sharing class is exactly themselves and which no
    /// return (or value evaluated by a return) can reach. The direct-C lowerer releases the
    /// scalar-sequence subset immediately before leaving the function.
    pub(super) owned_drops: std::collections::HashSet<Symbol>,
    /// Zero-based unit accumulators changed only in one i64-bounded counted loop. Their `+1`
    /// cannot overflow, so a branchless C lowering may use a plain machine-word add.
    pub(super) bounded_count_accumulators: std::collections::HashSet<Symbol>,
}

/// A statement being lowered: its address, the name it binds anew, and every read of a name inside it.
pub(super) struct StmtFrame {
    pub(super) key: usize,
    pub(super) binds: Option<Symbol>,
    pub(super) reads: Vec<(usize, Symbol)>,
}

/// A construct a `Break` or a `Return` can leave. A `Break` leaves the innermost loop or zone; a `Return`
/// inside a zone, through any loops, leaves the zone rather than its function — the VM's `FlowCtx`.
pub(super) enum Flow {
    Loop,
    /// A `Zone` block, or one task of an `Attempt all of the following:` or `Simultaneously:` block: a
    /// jump to `exit`, the label the lowering places after it once `taken`.
    Zone {
        exit: String,
        taken: std::cell::Cell<bool>,
    },
}

/// The name a value of type `sym` shows under (see [`CContext::display_name`]).
pub(super) fn display_name_of(
    sym: Symbol,
    display_names: &HashMap<Symbol, Symbol>,
    interner: &Interner,
) -> String {
    let shown = display_names.get(&sym).copied().unwrap_or(sym);
    let name = interner.resolve(shown);
    name.rsplit("::").next().unwrap_or(name).to_string()
}

/// A shared empty `PolicyRegistry` with `'static` lifetime — the default for every `CContext` that
/// isn't the top-level one. Coerces to any `&'a` since `'static: 'a`.
pub(super) fn empty_policy_registry() -> &'static PolicyRegistry {
    static EMPTY: std::sync::OnceLock<PolicyRegistry> = std::sync::OnceLock::new();
    EMPTY.get_or_init(PolicyRegistry::new)
}

impl<'a> CContext<'a> {
    /// What decides a field's C type beyond its declaration (`exact::field_ctype`).
    pub(super) fn field_facts(&self) -> super::exact::FieldFacts<'_> {
        super::exact::FieldFacts {
            exact: &self.exact.fields,
            absence: &self.absence,
        }
    }

    pub(super) fn new(interner: &'a Interner, registry: &'a TypeRegistry) -> Self {
        Self {
            vars: HashMap::new(),
            tower: None,
            tower_scope: None,
            absence: std::rc::Rc::default(),
            funcs: HashMap::new(),
            native_funcs: std::collections::HashSet::new(),
            interner,
            registry,
            lifted: std::cell::RefCell::new(Vec::new()),
            lambda_ctr: std::cell::Cell::new(0),
            bigint_vars: std::collections::HashSet::new(),
            number_locals: super::numbers::NumberLocals::default(),
            nonnegative_vars: std::collections::HashSet::new(),
            current_fn_returns: CType::Void,
            decl_scopes: vec![DeclScope::default()],
            renamed: HashMap::new(),
            rename_ctr: std::cell::Cell::new(0),
            stepped: None,
            stepped_fns: std::collections::HashSet::new(),
            precomputed: std::cell::RefCell::new(HashMap::new()),
            unpacked: std::cell::RefCell::new(HashMap::new()),
            uses_concurrency: false,
            select_ctr: std::cell::Cell::new(0),
            task_params: HashMap::new(),
            program_statics: std::collections::HashSet::new(),
            static_types: HashMap::new(),
            main_root_depth: None,
            counter_fields: std::collections::HashSet::new(),
            exact_hofs: std::collections::HashSet::new(),
            exact: super::exact::ExactSlots::default(),
            fn_params: HashMap::new(),
            fn_inout: HashMap::new(),
            plain_texts: std::collections::HashSet::new(),
            policies: empty_policy_registry(),
            display_names: HashMap::new(),
            flow: Vec::new(),
            nothing_names: std::collections::HashSet::new(),
            copy_fns: std::cell::RefCell::new(std::collections::HashSet::new()),
            equal_fns: std::cell::RefCell::new(std::collections::HashSet::new()),
            mutated: std::collections::HashSet::new(),
            changed_statics: std::collections::HashSet::new(),
            in_main: false,
            mutating: std::rc::Rc::new(HashMap::new()),
            call_shapes: std::rc::Rc::new(HashMap::new()),
            int_oracle: None,
            quiet_fns: std::rc::Rc::new(std::collections::HashSet::new()),
            ownership: None,
            fixed_stack_seqs: HashMap::new(),
            text_bytes_candidates: HashMap::new(),
            text_bytes_views: HashMap::new(),
            stmt_frames: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Lower `body` as the loop it is the body of: a `Break` in it leaves that loop.
    pub(super) fn in_loop<T>(&mut self, body: impl FnOnce(&mut Self) -> T) -> T {
        self.flow.push(Flow::Loop);
        let out = body(self);
        self.flow.pop();
        out
    }

    /// Lower `body` as a zone, appending the zone's exit label to `output` after it when a `Break` or a
    /// `Return` inside jumps to it.
    pub(super) fn in_zone(
        &mut self,
        output: &mut String,
        pad: &str,
        body: impl FnOnce(&mut Self, &mut String),
    ) {
        let n = self.select_ctr.get();
        self.select_ctr.set(n + 1);
        self.flow.push(Flow::Zone {
            exit: format!("__logos_zone_exit{n}"),
            taken: std::cell::Cell::new(false),
        });
        body(self, output);
        if let Some(Flow::Zone { exit, taken }) = self.flow.pop() {
            if taken.get() {
                use std::fmt::Write;
                writeln!(output, "{pad}{exit}: ;").unwrap();
            }
        }
    }

    /// The label of the innermost zone enclosing the statement being lowered, through any loops, marked
    /// taken: where a `Return` inside it goes.
    pub(super) fn enclosing_zone_exit(&self) -> Option<String> {
        self.flow.iter().rev().find_map(|f| match f {
            Flow::Zone { exit, taken } => {
                taken.set(true);
                Some(exit.clone())
            }
            Flow::Loop => None,
        })
    }

    /// Whether a zone encloses the statement being lowered, through any loops.
    pub(super) fn in_a_zone(&self) -> bool {
        self.flow.iter().any(|f| matches!(f, Flow::Zone { .. }))
    }

    /// Lower a new body — a function's, a step's, `## Main`'s — with no loop or zone around it, restoring
    /// the enclosing body's afterwards.
    pub(super) fn in_body<T>(&mut self, is_main: bool, body: impl FnOnce(&mut Self) -> T) -> T {
        let flow = std::mem::take(&mut self.flow);
        let was_main = std::mem::replace(&mut self.in_main, is_main);
        let ownership = self.ownership.take();
        let frames = std::mem::take(&mut *self.stmt_frames.borrow_mut());
        let out = body(self);
        self.flow = flow;
        self.in_main = was_main;
        self.ownership = ownership;
        *self.stmt_frames.borrow_mut() = frames;
        out
    }

    /// The name a value of type `sym` shows under: its generic's for an instance type, its own
    /// otherwise — the last `::` segment of a module-qualified name either way.
    pub(super) fn display_name(&self, sym: Symbol) -> String {
        display_name_of(sym, &self.display_names, self.interner)
    }

    /// Whether `(struct, field)` is a counter-CRDT field (its cell holds a `logos_crdt*` handle).
    pub(super) fn is_counter_field(&self, struct_sym: Symbol, field_sym: Symbol) -> bool {
        self.counter_fields.contains(&(struct_sym, field_sym))
    }

    /// Whether a counter field is GROW-ONLY (`ConvergentCount`/`GCounter`) — it rejects a negative net delta
    /// (a negative `Increase` or any `Decrease`), a trap on the VM; a `Tally`/`PNCounter` allows it.
    pub(super) fn field_is_grow_only_counter(&self, struct_sym: Symbol, field_sym: Symbol) -> bool {
        if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = self.registry.get(struct_sym)
        {
            if let Some(f) = fields.iter().find(|f| f.name == field_sym) {
                return matches!(
                    &f.ty,
                    crate::analysis::FieldType::Named(s) | crate::analysis::FieldType::Primitive(s)
                        if matches!(self.interner.resolve(*s), "ConvergentCount" | "GCounter")
                );
            }
        }
        false
    }

    /// The C name of a VALUE (a local, a parameter). A value may share its name with a function —
    /// a call always means the function — but C has one namespace, where the local would shadow the
    /// function, so such a value is renamed. Functions are named by `escape_c_ident` alone.
    pub(super) fn resolve(&self, sym: Symbol) -> String {
        let name = self.c_name(sym);
        match &self.stepped {
            Some(frame) if self.vars.contains_key(&sym) && frame.field(&name).is_some() => {
                format!("fr->{name}")
            }
            _ => name,
        }
    }

    /// A value's C name, before any frame qualifies it.
    fn c_name(&self, sym: Symbol) -> String {
        match self.renamed.get(&sym) {
            Some(name) => name.clone(),
            None => self.spelled(sym),
        }
    }

    /// Mint a deterministic temporary that cannot capture an interned source name or a renamed binding.
    pub(super) fn fresh_temporary(&self, stem: &str) -> String {
        loop {
            let n = self.select_ctr.get();
            self.select_ctr.set(n + 1);
            let candidate = format!("__logos_{stem}_{n}");
            let used = self.interner.lookup(&candidate).is_some()
                || self.renamed.values().any(|name| name == &candidate)
                || (0..self.interner.len())
                    .any(|i| self.c_name(Symbol::from_index(i)) == candidate);
            if !used {
                return candidate;
            }
        }
    }

    /// A value's C name by its own spelling.
    fn spelled(&self, sym: Symbol) -> String {
        let name = escape_c_ident(self.interner.resolve(sym));
        if self.funcs.contains_key(&sym) {
            format!("__logos_v_{name}")
        } else {
            name
        }
    }

    /// Bind `sym` as a value of type `ty` in the current C block, answering the C name to write it under:
    /// the variable this block already declared for `sym` when the type is the same (the caller assigns
    /// it); a fresh one when the block declared `sym` with another type, or when `sym` is bound outside the
    /// block — C puts a declared name in scope inside its own initializer, so `int64_t x = x + 1;` would read
    /// the new, unset `x` where LOGOS reads the one it shadows; else `sym`'s own spelling.
    pub(super) fn declare(&mut self, sym: Symbol, ty: CType) -> String {
        let here = self.declared_here(sym);
        let same = self.declared_value_here(sym, &ty);
        let was_view = self.text_bytes_views.remove(&sym).is_some();
        let shadows = !here && self.vars.contains_key(&sym);
        self.record(sym);
        if (here && !same) || shadows || was_view {
            self.rename_fresh(sym);
        }
        // One frame holds every binding of a stepped function: a name it already holds at another type
        // (a sibling block's binding) takes a fresh one.
        if self.stepped.as_ref().is_some_and(|frame| {
            frame
                .field(&self.c_name(sym))
                .is_some_and(|held| *held != ty)
        }) {
            self.rename_fresh(sym);
        }
        let name = self.c_name(sym);
        if let Some(frame) = &mut self.stepped {
            frame.hold(&name, ty.clone());
        }
        self.vars.insert(sym, ty);
        self.resolve(sym)
    }

    /// A borrowed byte pointer is distinct from the owned sequence with the same logical type.
    pub(super) fn declare_text_bytes(&mut self, sym: Symbol, source: Symbol) -> String {
        self.record(sym);
        if self.vars.contains_key(&sym) {
            self.rename_fresh(sym);
        }
        self.vars.insert(sym, CType::SeqI64(Ints::Int));
        self.text_bytes_views.insert(sym, source);
        self.resolve(sym)
    }

    pub(super) fn declared_value_here(&self, sym: Symbol, ty: &CType) -> bool {
        self.declared_here(sym)
            && self.vars.get(&sym) == Some(ty)
            && !self.text_bytes_views.contains_key(&sym)
    }

    /// Give `sym` a C name no other binding has.
    fn rename_fresh(&mut self, sym: Symbol) {
        let n = self.rename_ctr.get();
        self.rename_ctr.set(n + 1);
        let fresh = format!("__logos_b{n}_{}", self.spelled(sym));
        self.renamed.insert(sym, fresh);
    }

    /// Bind a parameter in the current C block under its own spelling, the signature's; in a stepped
    /// function it is a field of the frame.
    pub(super) fn bind_in_place(&mut self, sym: Symbol, ty: CType) -> String {
        self.record(sym);
        self.text_bytes_views.remove(&sym);
        let name = self.c_name(sym);
        if let Some(frame) = &mut self.stepped {
            frame.hold(&name, ty.clone());
        }
        self.vars.insert(sym, ty);
        self.resolve(sym)
    }

    /// Bind a program static at Main's root: the file-scope variable every function reads, never a field.
    pub(super) fn bind_static(&mut self, sym: Symbol, ty: CType) -> String {
        self.record(sym);
        self.text_bytes_views.remove(&sym);
        self.vars.insert(sym, ty);
        self.c_name(sym)
    }

    /// A compiler temporary of type `ty`, named from `prefix`; in a stepped function, a field of its frame.
    pub(super) fn temp(&mut self, prefix: &str, ty: CType) -> String {
        let n = self.select_ctr.get();
        self.select_ctr.set(n + 1);
        let name = format!("{prefix}{n}");
        match &mut self.stepped {
            Some(frame) => {
                frame.hold(&name, ty);
                format!("fr->{name}")
            }
            None => name,
        }
    }

    /// What a binding of `name` as a `ty` begins with: its declarator, or, in a stepped function whose frame
    /// already holds every binding, the name alone.
    pub(super) fn decl_head(&self, ty: &CType, name: &str) -> String {
        if self.stepped.is_some() {
            name.to_string()
        } else {
            c_param_decl(ty, name, self.interner)
        }
    }

    /// A statement that fails with the C string `message`: the runtime's failure, or — in a stepped function —
    /// its task's failure through the scheduler core, after which the step returns.
    pub(super) fn fail_stmt(&self, message: &str) -> String {
        if self.stepped.is_some() {
            format!("LOGOS_STEP_FAIL({message});")
        } else {
            format!("logos_fatal({message});")
        }
    }

    /// Note that the current block binds `sym`, keeping what it meant before for the block's close.
    fn record(&mut self, sym: Symbol) {
        let before = (
            self.vars.get(&sym).cloned(),
            self.renamed.get(&sym).cloned(),
        );
        let scope = self
            .decl_scopes
            .last_mut()
            .expect("the base scope is never popped");
        if !scope.shadowed.iter().any(|(s, _, _)| *s == sym) {
            scope.shadowed.push((sym, before.0, before.1));
        }
        scope.declared.insert(sym);
    }

    /// Enter a new C block scope (call when opening a `{`).
    pub(super) fn enter_scope(&mut self) {
        self.decl_scopes.push(DeclScope {
            text_bytes_views: self.text_bytes_views.clone(),
            ..DeclScope::default()
        });
    }

    /// Leave the current C block scope (call when closing its `}`): every name it bound means again what
    /// it meant outside.
    pub(super) fn exit_scope(&mut self) {
        if let Some(scope) = self.decl_scopes.pop() {
            self.text_bytes_views = scope.text_bytes_views;
            for (sym, ty, name) in scope.shadowed.into_iter().rev() {
                match ty {
                    Some(ty) => self.vars.insert(sym, ty),
                    None => self.vars.remove(&sym),
                };
                match name {
                    Some(name) => self.renamed.insert(sym, name),
                    None => self.renamed.remove(&sym),
                };
            }
        }
        if self.decl_scopes.is_empty() {
            // Never underflow below the base scope.
            self.decl_scopes.push(DeclScope::default());
        }
    }

    /// Whether `sym` was already DECLARED in the CURRENT (innermost) C block.
    pub(super) fn declared_here(&self, sym: Symbol) -> bool {
        self.decl_scopes
            .last()
            .is_some_and(|s| s.declared.contains(&sym))
    }
}

/// A stepped function's frame: every binding and temporary it keeps across a scheduling point, with its C
/// type, in the order first bound; the resume points minted so far; and the type its `Return` hands back.
pub(super) struct Frame {
    pub(super) fields: Vec<(String, CType)>,
    resumes: usize,
    pub(super) returns: CType,
}

impl Frame {
    pub(super) fn returning(returns: CType) -> Self {
        Frame {
            fields: Vec::new(),
            resumes: 0,
            returns,
        }
    }

    fn field(&self, name: &str) -> Option<&CType> {
        self.fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, ty)| ty)
    }

    fn hold(&mut self, name: &str, ty: CType) {
        if self.field(name).is_none() {
            self.fields.push((name.to_string(), ty));
        }
    }

    /// The next resume point, `case <n>:` of the step's switch (`0` is the entry).
    pub(super) fn resume_point(&mut self) -> usize {
        self.resumes += 1;
        self.resumes
    }
}

/// One open C block: the names it declared, and what each meant before it did — the type and the C name
/// restored when the block closes.
#[derive(Default)]
pub(super) struct DeclScope {
    declared: std::collections::HashSet<Symbol>,
    shadowed: Vec<(Symbol, Option<CType>, Option<String>)>,
    text_bytes_views: HashMap<Symbol, Symbol>,
}

// =============================================================================
// Type Inference
// =============================================================================

/// The exact magnitude of an ALL-LITERAL integer expression as `i128`, or `None` if it involves a
/// variable/call/non-literal (whose runtime magnitude is unknown — kept on the fast `int64_t` path).
/// Lets a provably-overflowing literal expression (`10000000000 * 10000000000` = 10^20) flip to the
/// exact `LInt` (BigInt) tier while ordinary variable arithmetic stays raw i64 (no test/perf change).
fn int_static_magnitude(expr: &Expr) -> Option<i128> {
    match expr {
        Expr::Literal(Literal::Number(n)) => Some((*n as i128).abs()),
        Expr::BinaryOp { op, left, right } => {
            let l = int_static_magnitude(left)?;
            let r = int_static_magnitude(right)?;
            match op {
                BinaryOpKind::Add | BinaryOpKind::Subtract => l.checked_add(r),
                BinaryOpKind::Multiply => l.checked_mul(r),
                BinaryOpKind::Divide | BinaryOpKind::ExactDivide | BinaryOpKind::FloorDivide => {
                    Some(l)
                }
                BinaryOpKind::Modulo => Some(l.min(r)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Does this integer expression PROVABLY exceed the i64 range by static (all-literal) magnitude?
pub(super) fn int_exceeds_i64(expr: &Expr) -> bool {
    int_static_magnitude(expr).map_or(false, |m| m > i64::MAX as i128)
}

/// The range an operator confines `expr`'s value to whatever its operands are: `x & c` for a non-negative
/// literal `c` lies in `[0, c]`, and `x % c` for a nonzero literal `c` in `(-|c|, |c|)`. The VM answers either
/// within that range or fails, even when `x` is a big integer, so a slot only ever given such values keeps the
/// machine word.
pub(super) fn bounded_int(expr: &Expr) -> Option<(i64, i64)> {
    let literal = |e: &Expr| match e {
        Expr::Literal(Literal::Number(c)) => Some(*c),
        _ => None,
    };
    match expr {
        Expr::BinaryOp {
            op: BinaryOpKind::BitAnd,
            left,
            right,
        } => {
            let mask = literal(right)
                .filter(|c| *c >= 0)
                .or_else(|| literal(left).filter(|c| *c >= 0))?;
            Some((0, mask))
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Modulo,
            right,
            ..
        } => {
            let divisor = literal(right).filter(|c| *c != 0)?;
            let reach = (divisor.unsigned_abs() - 1) as i64;
            Some((-reach, reach))
        }
        _ => None,
    }
}

/// Does `expr` contain an INTEGER exponentiation — a `**` operator or a `pow(a, b)` call — anywhere?
/// Such a value is exact and unbounded (it promotes to BigInt on overflow), so a local assigned it or a
/// function returning it must carry the tagged `LInt`. This is only consulted for NON-float targets and
/// returns (the float pre-pass already claims a `double` Taylor-series power), so a `**`/`pow` reached
/// here is genuinely integer — mirroring `infer_expr_type`, which types integer power as `BigInt`.
pub(super) fn expr_is_int_power_form(expr: &Expr, interner: &Interner) -> bool {
    match expr {
        Expr::BinaryOp {
            op: BinaryOpKind::Pow,
            ..
        } => true,
        Expr::Call { function, args }
            if interner.resolve(*function) == "pow" && args.len() == 2 =>
        {
            true
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_is_int_power_form(left, interner) || expr_is_int_power_form(right, interner)
        }
        Expr::Call { args, .. } | Expr::CallExpr { args, .. } => {
            args.iter().any(|a| expr_is_int_power_form(a, interner))
        }
        Expr::Index { collection, index } => {
            expr_is_int_power_form(collection, interner) || expr_is_int_power_form(index, interner)
        }
        _ => false,
    }
}

/// Whether `expr` carries an exact (`LInt`) integer OUT as its value: a local in `big`, a call to a
/// function whose return is exact, a read of a field some store makes exact — and any integer arithmetic
/// or bitwise operation over one (`a + 1`, `a * 10`, `a & mask`). Nothing else does: a comparison or
/// `is approximately` yields `Bool`, a `combined with` / `followed by` or an `Add` with a text literal
/// yields `Text`/`Seq`, and an `item k of xs` yields the collection's ELEMENT type (never `LInt` — a
/// `Seq of Int` is `Seq_i64`), so an exact operand buried inside one of those does not make the whole
/// exact. A field read is judged by the field's NAME: which record an expression reads is not known
/// before emission, so every field of that name that some store makes exact counts — over-approximating
/// a local to `LInt` costs a tag check, where under-approximating would hand an `LInt` to a machine word.
pub(super) fn expr_carries_exact(
    expr: &Expr,
    big: &std::collections::HashSet<Symbol>,
    slots: &super::exact::ExactSlots,
) -> bool {
    if bounded_int(expr).is_some() {
        return false;
    }
    match expr {
        Expr::Identifier(s) => big.contains(s),
        Expr::Call { function, .. } => {
            slots.returns.contains(function) || slots.fn_values.contains(function)
        }
        Expr::CallExpr { .. } => true,
        Expr::FieldAccess { field, .. } => slots.field_names.contains(field),
        Expr::Index { collection, index } => match index {
            Expr::Literal(Literal::Number(k)) => slots.component_is_exact(collection, *k),
            _ => false,
        },
        Expr::BinaryOp { op, left, right }
            if !matches!(
                op,
                BinaryOpKind::Eq
                    | BinaryOpKind::NotEq
                    | BinaryOpKind::Lt
                    | BinaryOpKind::Gt
                    | BinaryOpKind::LtEq
                    | BinaryOpKind::GtEq
                    | BinaryOpKind::ApproxEq
                    | BinaryOpKind::Concat
                    | BinaryOpKind::SeqConcat
            ) && !is_text_operand(left)
                && !is_text_operand(right) =>
        {
            expr_carries_exact(left, big, slots) || expr_carries_exact(right, big, slots)
        }
        _ => false,
    }
}

/// A text literal or an interpolated string: an `Add` with one is concatenation, never arithmetic.
fn is_text_operand(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Literal(Literal::Text(_)) | Expr::InterpolatedString(_)
    )
}

/// One binding a body makes in its own scope: a `Let`/`Set` of a value (with the `Let`'s annotation), or a
/// variable an `Inspect` arm binds to a field of the arm's variant.
pub(super) enum Binding<'w, 'a> {
    Value(&'w Expr<'a>, Option<&'w TypeExpr<'a>>),
    Field {
        enum_name: Option<Symbol>,
        variant: Symbol,
        field: Symbol,
    },
}

/// Every binding in `body` — each `Let`, each `Set`, and each variable an `Inspect` arm binds — wherever it
/// sits in the body's blocks. A nested function's body and a closure's are scopes of their own, lowered
/// with pre-passes of their own, so the walk does not enter them.
pub(super) fn for_each_binding<'w, 'a>(
    body: &'w [Stmt<'a>],
    mut f: impl FnMut(Symbol, Binding<'w, 'a>),
) {
    use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};
    walk_nodes(body, |node| match node {
        Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => Walk::Past,
        Visited::Stmt(Stmt::Let { var, value, ty, .. }) => {
            f(*var, Binding::Value(value, *ty));
            Walk::Into
        }
        Visited::Stmt(Stmt::Set { target, value }) => {
            f(*target, Binding::Value(value, None));
            Walk::Into
        }
        Visited::Stmt(Stmt::Inspect { arms, .. }) => {
            for arm in arms {
                if let Some(variant) = arm.variant {
                    for (field, binding) in &arm.bindings {
                        f(
                            *binding,
                            Binding::Field {
                                enum_name: arm.enum_name,
                                variant,
                                field: *field,
                            },
                        );
                    }
                }
            }
            Walk::Into
        }
        _ => Walk::Into,
    });
}

/// Every declaration of field `field` of an enum variant named `variant`, with its enum: the one of
/// `enum_name` when the arm names its enum, else that of every enum declaring such a variant.
pub(super) fn variant_fields<'r>(
    enum_name: Option<Symbol>,
    variant: Symbol,
    field: Symbol,
    registry: &'r TypeRegistry,
) -> Vec<(Symbol, &'r crate::analysis::FieldDef)> {
    let in_enum = |esym: Symbol| match registry.get(esym) {
        Some(crate::analysis::TypeDef::Enum { variants, .. }) => variants
            .iter()
            .find(|v| v.name == variant)
            .and_then(|v| v.fields.iter().find(|fd| fd.name == field))
            .map(|fd| (esym, fd)),
        _ => None,
    };
    match enum_name {
        Some(e) => in_enum(e).into_iter().collect(),
        None => registry
            .iter_types()
            .filter_map(|(sym, _)| in_enum(*sym))
            .collect(),
    }
}

/// True when `expr` is float-typed, given the float variables known so far. A float literal, a
/// `-> Real` math function call (`sin`/`cos`/…), or any arithmetic touching a float operand is a
/// float. This is the structural companion to [`collect_float_vars`] — no `CContext` needed, so it
/// runs in the pre-pass that decides BigInt promotion.
fn expr_is_float(
    expr: &Expr,
    floats: &std::collections::HashSet<Symbol>,
    interner: &Interner,
) -> bool {
    match expr {
        Expr::Literal(Literal::Float(_)) => true,
        Expr::Identifier(s) => floats.contains(s),
        Expr::Call { function, .. } => {
            crate::codegen::expr::is_float_returning_math_call(*function, interner)
        }
        Expr::BinaryOp { op, left, right } => {
            matches!(
                op,
                BinaryOpKind::Add
                    | BinaryOpKind::Subtract
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Divide
                    | BinaryOpKind::ExactDivide
                    | BinaryOpKind::FloorDivide
                    | BinaryOpKind::Modulo
                    | BinaryOpKind::Pow
            ) && (expr_is_float(left, floats, interner) || expr_is_float(right, floats, interner))
        }
        _ => false,
    }
}

/// The float-typed locals in `body`: the `Real`/`Float` parameters, plus any local ever bound a
/// float-typed value (a `Real` type annotation, a float literal, a `-> Real` math call, float arithmetic,
/// or a `Real` field an `Inspect` arm binds). Fixpoint, since float-ness flows through assignments (`Let a
/// be b` where `b` is float). This is what keeps the exact-integer pre-pass off `double` accumulators — a
/// float series like `sin`'s `Set term to (0.0 - term) * x2` is NOT an arbitrary-precision integer.
pub(super) fn collect_float_vars(
    body: &[Stmt],
    params: &[(Symbol, CType)],
    interner: &Interner,
    registry: &TypeRegistry,
) -> std::collections::HashSet<Symbol> {
    let mut floats: std::collections::HashSet<Symbol> = params
        .iter()
        .filter(|(_, ty)| *ty == CType::Float64)
        .map(|(s, _)| *s)
        .collect();
    loop {
        let mut changed = false;
        for_each_binding(body, |target, binding| {
            if floats.contains(&target) {
                return;
            }
            let is_float = match binding {
                Binding::Value(value, ty) => {
                    ty.is_some_and(|t| resolve_type_expr(t, interner) == CType::Float64)
                        || expr_is_float(value, &floats, interner)
                }
                Binding::Field {
                    enum_name,
                    variant,
                    field,
                } => variant_fields(enum_name, variant, field, registry)
                    .iter()
                    .any(|(_, fd)| {
                        field_type_to_ctype(&fd.ty, interner, registry) == CType::Float64
                    }),
            };
            if is_float {
                floats.insert(target);
                changed = true;
            }
        });
        if !changed {
            break;
        }
    }
    floats
}

/// What the C tier knows that the shared exactness decision ([`crate::analysis::exactness`]) does not: its locals
/// that hold no `Int` (Reals and words), the variables that arrive exact, and the slots settled so far.
struct CExactHost<'s> {
    bounded_updates: &'s std::collections::HashSet<usize>,
    no_int: &'s std::collections::HashSet<Symbol>,
    arriving: &'s std::collections::HashSet<Symbol>,
    slots: &'s super::exact::ExactSlots,
    oracle: Option<&'s crate::optimize::OracleFacts>,
    interner: &'s Interner,
}

impl crate::analysis::exactness::AstHost for CExactHost<'_> {
    fn holds_no_int(&self, var: Symbol) -> bool {
        self.no_int.contains(&var)
    }

    fn var_arrives_exact(&self, var: Symbol) -> bool {
        self.arriving.contains(&var)
    }

    fn call_arrives_exact(&self, function: Symbol) -> bool {
        self.slots.returns.contains(&function) || self.slots.fn_values.contains(&function)
    }

    fn value_call_arrives_exact(&self) -> bool {
        true
    }

    fn field_arrives_exact(&self, field: Symbol) -> bool {
        self.slots.field_names.contains(&field)
    }

    fn component_arrives_exact(&self, collection: &Expr, k: i64) -> bool {
        self.slots.component_is_exact(collection, k)
    }

    fn arm_field_arrives_exact(
        &self,
        _enum_name: Option<Symbol>,
        variant: Symbol,
        field: Symbol,
    ) -> bool {
        self.slots.fields.contains(&(variant, field))
    }

    fn is_power(&self, function: Symbol) -> bool {
        self.interner.resolve(function) == "pow"
    }

    fn fits_word(&self, e: &Expr) -> bool {
        let guarded_raw =
            matches!(
                e,
                Expr::BinaryOp {
                    op: BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply,
                    ..
                }
            ) && crate::optimize::facts::holds_expr(crate::optimize::facts::Fact::RawIntOp, e);
        self.bounded_updates.contains(&(e as *const Expr as usize))
            || guarded_raw
            || self.oracle.is_some_and(|o| o.expr_int_range(e).is_some())
    }
}

/// The locals in `body` that must be declared `LInt` (exact arbitrary precision) — the shared decision's
/// ([`exact_decision`]) for the body of no function in particular.
pub(super) fn collect_bigint_vars(
    body: &[Stmt],
    no_int: &std::collections::HashSet<Symbol>,
    interner: &Interner,
    slots: &super::exact::ExactSlots,
    oracle: Option<&crate::optimize::OracleFacts>,
    seeds: &std::collections::HashSet<Symbol>,
) -> std::collections::HashSet<Symbol> {
    exact_decision(body, None, no_int, interner, slots, oracle, seeds).vars
}

/// The shared exactness decision ([`crate::analysis::exactness::body_exactness`]) for `body`, the body of `function`
/// (or Main): its exact variables and whether a value it returns is. A local holding no `Int` (`no_int`: a Real, a
/// word) is never exact. The variables
/// that arrive exact are the body's exact parameters (`seeds`), a variable lent to an exact `mutable` parameter
/// (`ExactSlots::lent_exact`) and a program binding some body makes exact (`ExactSlots::program`).
pub(super) fn exact_decision(
    body: &[Stmt],
    function: Option<Symbol>,
    no_int: &std::collections::HashSet<Symbol>,
    interner: &Interner,
    slots: &super::exact::ExactSlots,
    oracle: Option<&crate::optimize::OracleFacts>,
    seeds: &std::collections::HashSet<Symbol>,
) -> crate::analysis::exactness::BodyDecision {
    use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};
    let mut arriving: std::collections::HashSet<Symbol> = seeds.iter().copied().collect();
    arriving.extend(slots.program.iter().copied());
    walk_nodes(body, |node| {
        let (function, args) = match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => {
                return Walk::Past
            }
            Visited::Stmt(Stmt::Call { function, args })
            | Visited::Expr(Expr::Call { function, args }) => (*function, args),
            _ => return Walk::Into,
        };
        for &i in slots.lent_exact.get(&function).into_iter().flatten() {
            if let Some(Expr::Identifier(x)) = args.get(i).copied() {
                arriving.insert(*x);
            }
        }
        Walk::Into
    });
    arriving.retain(|s| !no_int.contains(s));
    let empty = std::collections::HashSet::new();
    let host = CExactHost {
        bounded_updates: &empty,
        no_int,
        arriving: &arriving,
        slots,
        oracle,
        interner,
    };
    // Decide without count certificates first: a bound that is exact must
    // never become a word by relying on the count it is supposed to bound.
    let baseline = crate::analysis::exactness::body_exactness(body, function, &host);
    let counts = crate::codegen::stmt::collect_bounded_count_accumulators_with_purity(
        body,
        &baseline.vars,
        &slots.pure_calls,
    );
    let mut bounded_updates = std::collections::HashSet::new();
    walk_nodes(body, |node| match node {
        Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => Walk::Past,
        Visited::Stmt(Stmt::Set { target, value })
            if counts.contains(target)
                && !arriving.contains(target)
                && !no_int.contains(target) =>
        {
            bounded_updates.insert(*value as *const Expr as usize);
            Walk::Into
        }
        _ => Walk::Into,
    });
    if bounded_updates.is_empty() {
        baseline
    } else {
        let host = CExactHost {
            bounded_updates: &bounded_updates,
            ..host
        };
        crate::analysis::exactness::body_exactness(body, function, &host)
    }
}

/// Does any `Return` in `body` yield an exact value — an exact local, a statically-huge literal, an integer
/// power, or a value carrying one out? Such a function's declared `Int` return is `LInt` in C so the
/// arbitrary-precision value survives. Every `Return` of the body's own scope counts, in whatever block it
/// sits (an `Inspect` arm's included); a nested function's or a closure's are their own.
pub(super) fn body_returns_exact(
    body: &[Stmt],
    big: &std::collections::HashSet<Symbol>,
    slots: &super::exact::ExactSlots,
    interner: &Interner,
) -> bool {
    use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};
    let mut found = false;
    walk_nodes(body, |node| match node {
        Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => Walk::Past,
        Visited::Stmt(Stmt::Return { value: Some(v) }) => {
            // Under the `-> Int` guard this runs behind, a `**`/`pow` is necessarily INTEGER power
            // (exact, overflow-promoting), so its function widens to `LInt` to carry the value out.
            let exact = int_exceeds_i64(v)
                || expr_carries_exact(v, big, slots)
                || expr_is_int_power_form(v, interner);
            if exact && bounded_int(v).is_none() {
                found = true;
            }
            Walk::Into
        }
        _ => Walk::Into,
    });
    found
}

/// The sequence type of `left followed by right`: the operands' own, an empty list literal taking the other
/// side's element type.
pub(super) fn seq_concat_type(left: &Expr, right: &Expr, ctx: &CContext) -> CType {
    let is_empty_literal = |e: &Expr| matches!(e, Expr::List(items) if items.is_empty());
    if is_empty_literal(left) {
        infer_expr_type(right, ctx)
    } else {
        infer_expr_type(left, ctx)
    }
}

/// The type of `l op r` when it is temporal arithmetic, exactly the pairs the interpreter defines
/// (`semantics::arith`): `Date ± Span` is a Date; `Moment ± Span`, `Span + Moment`, `Moment ± Duration`
/// and `Duration + Moment` are Moments; `Duration ± Duration` is a Duration.
pub(super) fn temporal_result(op: BinaryOpKind, l: &CType, r: &CType) -> Option<CType> {
    use BinaryOpKind::{Add, Subtract};
    match (op, l, r) {
        (Add | Subtract, CType::Date, CType::Span) => Some(CType::Date),
        (Add | Subtract, CType::Moment, CType::Span | CType::Duration)
        | (Add, CType::Span | CType::Duration, CType::Moment) => Some(CType::Moment),
        (Add | Subtract, CType::Duration, CType::Duration) => Some(CType::Duration),
        _ => None,
    }
}

/// The word an arithmetic, bitwise or shift operation yields over operands of the kinds `lt` and `rt`, when
/// one is a word: whatever the VM's arithmetic answers for samples of the two kinds, when that is a word —
/// ring arithmetic and bitwise logic on two words of one width, a shift of a word by a count. `None` for a
/// pair the VM refuses, or no word.
fn word_result(op: BinaryOpKind, lt: &CType, rt: &CType) -> Option<CType> {
    if !matches!(lt, CType::Word(_)) && !matches!(rt, CType::Word(_)) {
        return None;
    }
    match crate::semantics::arith::binary_op(
        op,
        super::emit::sample_of(lt, 1)?,
        super::emit::sample_of(rt, 1)?,
    ) {
        Ok(crate::interpreter::RuntimeValue::Word(word)) => Some(CType::Word(word.width() as u8)),
        _ => None,
    }
}

pub(super) fn infer_expr_type(expr: &Expr, ctx: &CContext) -> CType {
    if let Some(held) = ctx.unpacked.borrow().get(&(expr as *const Expr as usize)) {
        return held.clone();
    }
    if let Some(exact) = super::tower::infer(expr, ctx) {
        return exact;
    }
    match expr {
        Expr::Literal(Literal::Number(_)) => CType::Int64,
        Expr::Literal(Literal::Float(_)) => CType::Float64,
        Expr::Literal(Literal::Boolean(_)) => CType::Bool,
        Expr::Literal(Literal::Text(_)) => CType::String,
        Expr::Literal(Literal::Nothing) => CType::Void,
        Expr::Literal(Literal::Date(_)) => CType::Date,
        Expr::Literal(Literal::Moment(_)) => CType::Moment,
        Expr::Literal(Literal::Time(_)) => CType::Time,
        Expr::Literal(Literal::Duration(_)) => CType::Duration,
        Expr::Literal(Literal::Span { .. }) => CType::Span,
        Expr::Literal(Literal::Char(_)) => CType::Int64,
        // The temporal builtins are typed by NAME (they resolve to the injected clock, not a variable):
        // `now` is a `Moment` (nanoseconds), `today` a `Date` (days) — so a `Let m be now` binds a Moment
        // and `Show m` renders `YYYY-MM-DD HH:MM`, matching the VM/tree-walker. The name wins even when
        // shadowed, so this precedes the variable lookup.
        // A value-or-nothing variable read as an expression is an Int (see `emit::opt_var`).
        Expr::Identifier(sym) => match ctx.interner.resolve(*sym) {
            "now" => CType::Moment,
            "today" => CType::Date,
            _ => match ctx.vars.get(sym) {
                Some(CType::Opt(inner)) => (**inner).clone(),
                None => CType::Int64,
                Some(ty) => ty.clone(),
            },
        },
        Expr::Tuple(items) => CType::Tuple(items.iter().map(|e| infer_expr_type(e, ctx)).collect()),
        Expr::BinaryOp { op, left, right } => {
            match op {
                BinaryOpKind::Eq
                | BinaryOpKind::NotEq
                | BinaryOpKind::Lt
                | BinaryOpKind::LtEq
                | BinaryOpKind::Gt
                | BinaryOpKind::GtEq
                | BinaryOpKind::And
                | BinaryOpKind::Or
                | BinaryOpKind::ApproxEq => CType::Bool,
                BinaryOpKind::Concat => CType::String,
                // `a followed by b` is a fresh sequence of the two operands' element type.
                BinaryOpKind::SeqConcat => seq_concat_type(left, right, ctx),
                // `&`/`|`/`xor` on two Bool operands is a BOOL (shown true/false, ≡ the VM); on ints it is an
                // int. The C value is the same `l OP r` either way — only the display type differs.
                BinaryOpKind::BitAnd | BinaryOpKind::BitOr | BinaryOpKind::BitXor => {
                    let (lt, rt) = (infer_expr_type(left, ctx), infer_expr_type(right, ctx));
                    if let Some(word) = word_result(*op, &lt, &rt) {
                        word
                    } else if lt == CType::Bool && rt == CType::Bool {
                        CType::Bool
                    } else {
                        CType::Int64
                    }
                }
                BinaryOpKind::Shl | BinaryOpKind::Shr => {
                    let (lt, rt) = (infer_expr_type(left, ctx), infer_expr_type(right, ctx));
                    word_result(*op, &lt, &rt).unwrap_or(CType::Int64)
                }
                BinaryOpKind::Add
                | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply
                | BinaryOpKind::Divide
                | BinaryOpKind::ExactDivide
                | BinaryOpKind::FloorDivide
                | BinaryOpKind::Modulo
                | BinaryOpKind::Pow => {
                    let lt = infer_expr_type(left, ctx);
                    let rt = infer_expr_type(right, ctx);
                    if let Some(word) = word_result(*op, &lt, &rt) {
                        word
                    } else if let Some(temporal) = temporal_result(*op, &lt, &rt) {
                        temporal
                    } else if lt == CType::String || rt == CType::String {
                        CType::String
                    } else if lt == CType::Modular && rt == CType::Modular {
                        // Only a `Modular ⊕ Modular` form stays in the ring; a mixed `Modular + Int`
                        // is rejected by the interpreter, so it falls through here and fails at gcc.
                        CType::Modular
                    } else if lt == CType::Money && rt == CType::Money {
                        // `money ± money` stays Money; `money ÷ money` is a dimensionless ratio (Decimal).
                        if matches!(op, BinaryOpKind::Divide | BinaryOpKind::ExactDivide) {
                            CType::Rational
                        } else {
                            CType::Money
                        }
                    } else if lt == CType::Money || rt == CType::Money {
                        // `money × k` (or `k × money`) scales the amount → Money.
                        CType::Money
                    } else if lt == CType::Quantity || rt == CType::Quantity {
                        // Any `+ − × ÷` involving a Quantity stays a Quantity (a dimension-combining
                        // `× ÷` yields a synthetic-unit Quantity; a scalar `×` preserves the unit).
                        CType::Quantity
                    } else if lt == CType::Complex || rt == CType::Complex {
                        CType::Complex
                    } else if lt == CType::Float64 || rt == CType::Float64 {
                        CType::Float64
                    } else if lt == CType::Rational || rt == CType::Rational {
                        CType::Rational
                    } else if lt == CType::Decimal || rt == CType::Decimal {
                        // A decimal keeps its digits through `+ − ×` with an integer or a decimal and divides
                        // exactly into a rational.
                        if matches!(op, BinaryOpKind::Divide | BinaryOpKind::ExactDivide) {
                            CType::Rational
                        } else {
                            CType::Decimal
                        }
                    } else if *op == BinaryOpKind::ExactDivide
                        && matches!(lt, CType::Int64 | CType::BigInt | CType::Bool)
                        && matches!(rt, CType::Int64 | CType::BigInt | CType::Bool)
                    {
                        // An exact quotient of integers keeps its fraction.
                        CType::Rational
                    } else if matches!(
                        op,
                        BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply
                    ) && crate::optimize::facts::holds_expr(
                        crate::optimize::facts::Fact::RawIntOp,
                        expr,
                    ) {
                        // A versioning pass constructed this integer node under a
                        // guard that proves its result stays in i64. Its C value and
                        // its inferred representation must agree; otherwise an
                        // int64_t expression is assigned to an LInt slot even though
                        // the cold sibling already preserves the exact fallback.
                        CType::Int64
                    } else if matches!(op, BinaryOpKind::Pow) {
                        // Integer `**` is EXACT and unbounded: it promotes to BigInt on overflow
                        // (`2 ** 100`) and downsizes when the result fits (`2 ** 10 → 1024`), so its
                        // C type is the tagged `LInt`, never a wrapping `int64_t`. (A `Modular`/`Float`
                        // base was already dispatched above to its own ring/`double` power.)
                        CType::BigInt
                    } else if bounded_int(expr).is_some() {
                        // A remainder by a literal lies within it, whatever the dividend.
                        CType::Int64
                    } else if *op == BinaryOpKind::Modulo && rt == CType::Int64 {
                        // A remainder is smaller than its divisor, and a machine-word divisor bounds it
                        // within a machine word, whatever the dividend.
                        CType::Int64
                    } else if lt == CType::BigInt || rt == CType::BigInt || int_exceeds_i64(expr) {
                        // A BigInt operand, or an all-literal product/sum that provably exceeds i64,
                        // makes this exact arbitrary-precision (`LInt`). Ordinary in-range int
                        // arithmetic (any non-literal operand) stays `int64_t`.
                        CType::BigInt
                    } else {
                        CType::Int64
                    }
                }
            }
        }
        Expr::Call { function, args } => {
            // Builtins are not in `funcs` (that map holds user/stdlib `## To` signatures), so a
            // call to one would default to Int64 and mis-type its binding — e.g. `Let sb be
            // text_bytes(s)` needs `Seq_i64`, not `int64_t`, or a downstream `matchAt(sb, …)`
            // fails to type-check. Give the non-Int64-returning builtins the stdlib leans on.
            // A function the program defines is typed by its own signature first (declarer wins):
            // a program's `## To sin (n: Int) -> Int` is not the libm `sin`.
            if !ctx.native_funcs.contains(function) {
                if let Some(declared) = ctx.funcs.get(function) {
                    return declared.clone();
                }
            }
            // A builtin the VM gives a word result is that word — the VM's one table: a constructor or a
            // fixed-width op its own width, a width-generic op (`rotl`, `word_and`, …) its first argument's.
            if let Some(builtin) =
                crate::semantics::builtins::builtin_from_name(ctx.interner.resolve(*function))
            {
                if let Some(bits) = builtin.word_result(None) {
                    return CType::Word(bits);
                }
                if builtin.word_result(Some(8)).is_some() {
                    if let Some(CType::Word(bits)) = args.first().map(|a| infer_expr_type(a, ctx)) {
                        return CType::Word(bits);
                    }
                }
            }
            match ctx.interner.resolve(*function) {
                // The call spellings of `length of x` and `copy of x` are typed as those nodes are.
                "length" if args.len() == 1 => infer_expr_type(
                    &Expr::Length {
                        collection: args[0],
                    },
                    ctx,
                ),
                "copy" if args.len() == 1 => infer_expr_type(&Expr::Copy { expr: args[0] }, ctx),
                "contains" if args.len() == 2 => CType::Bool,
                "text_bytes" => CType::SeqI64(Ints::Int),
                "format" => CType::String,
                // env.lg's native `args()` — the argv sequence `logos_args()` builds.
                "args" => CType::SeqStr,
                "text_from_bytes" | "chr" => CType::String,
                // `pow` is polymorphic: emit.rs keeps integer exponentiation exact in int64 when
                // both operands are integer-typed (`is_int_pow`), else it is the libm double `pow`.
                // Mirror that here so `Show 3 ** 34` dispatches `show_i64` (exact) not `show_f64`.
                // `pow(m, e)` on a ℤ/nℤ base stays in the ring (modular exponentiation).
                "pow"
                    if args.len() == 2
                        && matches!(infer_expr_type(&args[0], ctx), CType::Modular) =>
                {
                    CType::Modular
                }
                "pow" => {
                    let int_pow = args.len() >= 2
                        && !matches!(infer_expr_type(&args[0], ctx), CType::Float64)
                        && !matches!(infer_expr_type(&args[1], ctx), CType::Float64);
                    // Integer `pow(a, b)` is exact and promotes on overflow — the tagged `LInt` — for
                    // the same reason the `**` operator does; a float operand keeps the libm `double` pow.
                    if int_pow {
                        CType::BigInt
                    } else {
                        CType::Float64
                    }
                }
                // Float-returning builtins (translated to their libm form in emit.rs). Without
                // this they'd default to Int64, so `Show sqrt(2.0)` would print `1`, not `1.41…`.
                "sqrt" | "cbrt" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "atan2"
                | "sinh" | "cosh" | "tanh" | "asinh" | "acosh" | "atanh" | "exp" | "exp2"
                | "expm1" | "ln" | "log" | "log2" | "log10" | "log1p" | "hypot" | "fabs"
                | "copysign" | "fmin" | "fmax" | "ldexp" | "parseFloat" => CType::Float64,
                "readReals" => CType::SeqF64,
                // Temporal tower: parse/add yield Moments, format yields a String, the component
                // extractors and spans yield Ints, and date_of/time_of yield Date/Time.
                "parse_timestamp" | "add_seconds" => CType::Moment,
                "format_timestamp" => CType::String,
                "year_of" | "month_of" | "day_of" | "hour_of" | "minute_of" | "second_of"
                | "weekday_of" | "week_of" | "quarter_of" | "seconds_between"
                | "months_between" | "years_between" => CType::Int64,
                "date_of" => CType::Date,
                "time_of" => CType::Time,
                "makeDate" => CType::Date,
                "makeMoment" => CType::Moment,
                // `complex(re, im)` → `LComplex`, whether the parts are integer (exact) or float
                // (dyadic-rational, mirroring the interpreter's `ComplexF` — `cplx_reals` in emit.rs).
                "complex" if args.len() == 2 => CType::Complex,
                // Real / imaginary part extraction yields an `f64`.
                "re" | "im" if args.len() == 1 => CType::Float64,
                // |z| of an exact complex is its `f64` modulus.
                "abs"
                    if args.len() == 1
                        && matches!(infer_expr_type(&args[0], ctx), CType::Complex) =>
                {
                    CType::Float64
                }
                // `abs` of an exact Rational/Decimal stays exact (`LDec`) and of its own type; `floor`/`ceil`/`round`
                // of one yield the exact integer (a tagged `LInt`, which may exceed i64 for a huge rational).
                "abs"
                    if args.len() == 1
                        && matches!(infer_expr_type(&args[0], ctx), CType::Rational | CType::Decimal) =>
                {
                    infer_expr_type(&args[0], ctx)
                }
                "floor" | "ceil" | "round"
                    if args.len() == 1
                        && matches!(infer_expr_type(&args[0], ctx), CType::Rational | CType::Decimal) =>
                {
                    CType::BigInt
                }
                // `decimal(n)` → an exact base-10 decimal (`LDec`).
                "decimal" if args.len() == 1 => CType::Decimal,
                // `modular(v, n)` → exact ℤ/nℤ (LMod). Integer parts only on the C tier.
                "modular"
                    if args.len() == 2
                        && !matches!(infer_expr_type(&args[0], ctx), CType::Float64)
                        && !matches!(infer_expr_type(&args[1], ctx), CType::Float64) =>
                {
                    CType::Modular
                }
                // The `Uuid`-producing builtins → an `LUuid`; `uuid_version` reads the version nibble as
                // an Int; `uuid_bytes` yields the 16 raw bytes as a `Seq_i64`.
                "uuid" | "uuid_nil" | "uuid_max" | "uuid_dns" | "uuid_url" | "uuid_oid"
                | "uuid_x500" | "uuid_from_bytes" => CType::Uuid,
                "uuid_version" => CType::Int64,
                "uuid_bytes" => CType::SeqI64(Ints::Int),
                // `money(amount, "USD")` → an `LMoney` quantised to the currency's minor unit.
                "money" if args.len() == 2 => CType::Money,
                // `quantity(v, "unit")` and `convert(q, "unit")` → an `LQuant`.
                "quantity" | "convert" if args.len() == 2 => CType::Quantity,
                // `min`/`max` answer one of their operands: a Real when either is one, else an exact Int when
                // either is.
                "min" | "max" if args.len() == 2 => {
                    let (a, b) = (infer_expr_type(args[0], ctx), infer_expr_type(args[1], ctx));
                    if a == CType::Float64 || b == CType::Float64 {
                        CType::Float64
                    } else if a == CType::BigInt || b == CType::BigInt {
                        CType::BigInt
                    } else {
                        CType::Int64
                    }
                }
                // A user function's return type; or, for a fn-typed PARAM (a closure `f`), the function
                // pointer's return type — so `f(x)`'s result type is exact `LInt` when `f` is exact (else i64).
                _ => ctx
                    .funcs
                    .get(function)
                    .cloned()
                    .or_else(|| match ctx.vars.get(function) {
                        Some(CType::FnPtr { ret, .. }) => Some((**ret).clone()),
                        _ => None,
                    })
                    .unwrap_or(CType::Int64),
            }
        }
        Expr::CallExpr { .. } => CType::Int64,
        Expr::Length { .. } => CType::Int64,
        Expr::Index { collection, index } => {
            // The collection's own C type — from `ctx.vars` for a variable, else inferred (so
            // `item N of split(s)` sees the callee's `Seq_str` return type instead of defaulting to
            // Int64, which would `show_i64` a `char*` pointer as a number).
            let coll_ty = if let Expr::Identifier(sym) = collection {
                ctx.vars.get(sym).cloned()
            } else {
                Some(infer_expr_type(collection, ctx))
            };
            // A tuple/record index `item N of t` yields the N-th element's type (constant index).
            let tuple_elems: Option<Vec<CType>> = match &coll_ty {
                Some(CType::Tuple(elems)) => Some(elems.clone()),
                Some(CType::Record(fields)) => {
                    Some(fields.iter().map(|(_, t)| t.clone()).collect())
                }
                _ => None,
            };
            if let Some(elems) = tuple_elems {
                // A position the program writes resolves through the VM's own indexing (negative from the end).
                if let Expr::Literal(Literal::Number(n)) = index {
                    use crate::interpreter::RuntimeValue as V;
                    let positions = V::Tuple(std::rc::Rc::new(
                        (0..elems.len() as i64).map(V::Int).collect(),
                    ));
                    if let Ok(V::Int(k)) =
                        crate::semantics::collections::index_get(&positions, &V::Int(*n))
                    {
                        return elems[k as usize].clone();
                    }
                }
                return elems.first().cloned().unwrap_or(CType::Int64);
            }
            match coll_ty {
                Some(CType::SeqI64(ints)) => ints.ctype(),
                Some(CType::SeqBool) => CType::Bool,
                Some(CType::SeqStr) => CType::String,
                Some(CType::SeqF64) => CType::Float64,
                // `item k of <Text>` is the k-th character — a 1-char Text (so equality/`!=` between
                // two indexed chars lower as text comparison, not numeric).
                Some(CType::String) => CType::String,
                Some(CType::MapI64I64(_, val)) | Some(CType::MapStrI64(val)) => val.ctype(),
                Some(CType::MapStrStr) => CType::String,
                Some(CType::MapI64Str(_)) => CType::String,
                Some(CType::SeqStruct(s)) => user_type_ctype(s, ctx.registry),
                Some(CType::SeqTuple(elems)) => CType::Tuple(elems),
                Some(CType::SeqSeq(inner)) => *inner,
                Some(CType::SeqValue(inner)) => *inner,
                Some(CType::MapStrValue(inner) | CType::MapI64Value(inner)) => *inner,
                _ => CType::Int64,
            }
        }
        Expr::New {
            type_name,
            type_args,
            ..
        } => {
            let name = ctx.interner.resolve(*type_name);
            match name {
                "Seq" | "List" => {
                    if let Some(arg) = type_args.first() {
                        seq_of(resolve_type_expr_with_registry(
                            arg,
                            ctx.interner,
                            Some(ctx.registry),
                        ))
                    } else {
                        CType::SeqI64(Ints::Int)
                    }
                }
                "Map" => {
                    let key_type = type_args
                        .first()
                        .map(|p| resolve_type_expr(p, ctx.interner))
                        .unwrap_or(CType::Int64);
                    let val_type = type_args
                        .get(1)
                        .map(|p| resolve_type_expr(p, ctx.interner))
                        .unwrap_or(CType::Int64);
                    map_ctype(&key_type, &val_type)
                }
                "Set" => {
                    if let Some(arg) = type_args.first() {
                        set_of(resolve_type_expr(arg, ctx.interner))
                    } else {
                        CType::SetI64(Ints::Int)
                    }
                }
                _ => match ctx.registry.get(*type_name) {
                    Some(crate::analysis::TypeDef::Struct { .. }) => CType::Struct(*type_name),
                    Some(crate::analysis::TypeDef::Enum { .. }) => CType::Enum(*type_name),
                    _ => CType::Int64,
                },
            }
        }
        Expr::WithCapacity { value, .. } => infer_expr_type(value, ctx),
        Expr::Copy { expr: inner } => infer_expr_type(inner, ctx),
        Expr::List(elems) => seq_of(list_item_type(elems, ctx)),
        Expr::Contains { .. } => CType::Bool,
        Expr::Give { value } => infer_expr_type(value, ctx),
        Expr::Slice { collection, .. } => infer_expr_type(collection, ctx),
        // A set union / intersection has the same element type as its (left) operand — a Set.
        Expr::Union { left, .. } | Expr::Intersection { left, .. } => infer_expr_type(left, ctx),
        Expr::Not { .. } => CType::Bool,
        Expr::FieldAccess { object, field } => {
            let obj_type = infer_expr_type(object, ctx);
            if let CType::Struct(sym) = &obj_type {
                if let Some(crate::analysis::TypeDef::Struct { fields, .. }) =
                    ctx.registry.get(*sym)
                {
                    for f in fields {
                        if f.name == *field {
                            return super::exact::field_ctype(
                                *sym,
                                f,
                                ctx.interner,
                                ctx.registry,
                                &ctx.field_facts(),
                            );
                        }
                    }
                }
            }
            // A named-record field `r's p` → that field's CType (by name), so a Text/Real field dispatches
            // `show_text`/`show_f64` and binds at the right C type — not the Int64 default (which prints a
            // Text field's pointer as an integer and truncates a Real field). Mirrors the `Struct` arm; the
            // record shares the positional `LTuple` struct, so `emit.rs` reads the same `Record` fields.
            if let CType::Record(fields) = &obj_type {
                for (name, cty) in fields {
                    if name == field {
                        return cty.clone();
                    }
                }
            }
            CType::Int64
        }
        Expr::InterpolatedString(_) => CType::String,
        Expr::NewVariant { enum_name, .. } => CType::Enum(*enum_name),
        Expr::OptionSome { .. } | Expr::OptionNone => CType::Int64,
        Expr::Range { .. } | Expr::ManifestOf { .. } => CType::SeqI64(Ints::Int),
        Expr::ChunkAt { .. } => CType::Opt(Box::new(CType::Int64)),
        // A lambda value is a FUNCTION POINTER (it lambda-lifts to a top-level `static`/`__device__`
        // function). Typing it as such makes `Let g be (n) -> …` declare `RET (*g)(ARGS)` instead of an
        // `int64_t` that only survives gcc's implicit fn-ptr↔int conversion (nvcc/C++ rejects it).
        Expr::Closure {
            params,
            body,
            return_type,
        } => {
            let exact = |pt: &TypeExpr| {
                let mut t = resolve_type_expr_with_registry(pt, ctx.interner, Some(ctx.registry));
                super::exactify_int_ctype(&mut t);
                t
            };
            let param_types = params.iter().map(|(_, pt)| exact(pt)).collect();
            let ret = match return_type {
                Some(rt) => exact(rt),
                None => match body {
                    crate::ast::stmt::ClosureBody::Expression(e) => {
                        // Infer the body's type with the params in scope (they drive it).
                        let mut child = CContext::new(ctx.interner, ctx.registry);
                        child.vars = ctx.vars.clone();
                        child.funcs = ctx.funcs.clone();
                        child.exact = ctx.exact.clone();
                        child.fn_params = ctx.fn_params.clone();
                        for (pn, pt) in params.iter() {
                            child.vars.insert(*pn, exact(pt));
                        }
                        let mut t = infer_expr_type(e, &child);
                        super::exactify_int_ctype(&mut t);
                        t
                    }
                    crate::ast::stmt::ClosureBody::Block(_) => CType::BigInt,
                },
            };
            CType::FnPtr {
                ret: Box::new(ret),
                params: param_types,
            }
        }
        _ => CType::Int64,
    }
}
