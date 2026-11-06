//! What the VM says when a run fails, taken at generation time from the very functions the VM calls.
//!
//! A failure the emitted C reports must read, byte for byte, as the VM's. Copying the VM's sentences
//! into C string literals would let the two drift the day either is reworded, so every sentence here
//! is produced by calling the VM's own function on an input that fails the same way, and handed to
//! the emitter as a C string literal. A sentence that carries a value the run computes (an index, a
//! key, a path) is produced with a distinctive placeholder in that position and split around it, so
//! the C fills in the value exactly where the VM's `format!` put it.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::{ListRepr, MapStorage, RuntimeValue};
use crate::semantics::arith;
use crate::semantics::collections;
pub(crate) use crate::semantics::collections::IntegerPlace;

/// The integer placeholder: no sentence ever carries this value for any other reason.
const INT_HOLE: i64 = -7_919_000_000_000_000_007;

/// The text placeholder, for sentences that carry a Text the run computes.
const TEXT_HOLE: &str = "\u{1}LOGOS-HOLE\u{1}";

/// Escape `s` as the body of a C string literal, `%` included, so it can stand in a format string.
fn c_format_body(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '%' => out.push_str("%%"),
            '?' => out.push_str("\\?"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\{:03o}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// A sentence with no hole, as a C string literal (a `printf`-safe format: every `%` is doubled).
pub(super) fn literal(sentence: &str) -> String {
    format!("\"{}\"", c_format_body(sentence))
}

/// A sentence with one integer hole, as a C format string whose single conversion is `PRId64`.
fn with_int_hole(sentence: &str) -> String {
    let hole = INT_HOLE.to_string();
    let (before, after) = sentence.split_once(&hole).unwrap_or_else(|| {
        panic!("the VM's sentence {sentence:?} does not carry the value it was given")
    });
    format!(
        "\"{}%\" PRId64 \"{}\"",
        c_format_body(before),
        c_format_body(after)
    )
}

/// The second integer placeholder, for sentences that carry two values the run computes.
const INT_HOLE_2: i64 = -7_907_000_000_000_000_003;

/// A sentence carrying the placeholders `first` and then `second`, as a C format string whose two conversions
/// are `PRId64` and take the two values in that order.
fn with_int_holes(sentence: &str, first: i64, second: i64) -> String {
    let (before, rest) = sentence.split_once(&first.to_string()).unwrap_or_else(|| {
        panic!("the VM's sentence {sentence:?} does not carry the first value it was given")
    });
    let (between, after) = rest.split_once(&second.to_string()).unwrap_or_else(|| {
        panic!("the VM's sentence {sentence:?} does not carry the second value after the first")
    });
    format!(
        "\"{}%\" PRId64 \"{}%\" PRId64 \"{}\"",
        c_format_body(before),
        c_format_body(between),
        c_format_body(after)
    )
}

/// A sentence with one Text hole, as a C format string whose single conversion is `%s`.
fn with_text_hole(sentence: &str) -> String {
    let (before, after) = sentence.split_once(TEXT_HOLE).unwrap_or_else(|| {
        panic!("the VM's sentence {sentence:?} does not carry the text it was given")
    });
    format!("\"{}%s{}\"", c_format_body(before), c_format_body(after))
}

fn failure(result: Result<RuntimeValue, String>, what: &str) -> String {
    match result {
        Err(sentence) => sentence,
        Ok(v) => panic!(
            "the VM's {what} succeeded with {} where it must fail",
            v.to_display_string()
        ),
    }
}

fn empty_list() -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(Vec::new()))))
}

/// `item i of xs` with `i` naming no element — the format takes the index.
pub(super) fn index_out_of_bounds() -> String {
    with_int_hole(&failure(
        collections::index_get(&empty_list(), &RuntimeValue::Int(INT_HOLE)),
        "index",
    ))
}

/// `item k of m` with no entry under `k` — the format takes the key's display text.
pub(super) fn missing_key() -> String {
    let map = RuntimeValue::Map(Rc::new(RefCell::new(MapStorage::default())));
    let key = RuntimeValue::Text(Rc::new(TEXT_HOLE.to_string()));
    with_text_hole(&failure(collections::index_get(&map, &key), "map lookup"))
}

/// An integer division by zero.
pub(super) fn division_by_zero() -> String {
    literal(&failure(
        arith::divide(RuntimeValue::Int(1), RuntimeValue::Int(0)),
        "integer division",
    ))
}

/// A division by zero with a `Real` operand.
pub(super) fn real_division_by_zero() -> String {
    literal(&failure(
        arith::divide(RuntimeValue::Float(1.0), RuntimeValue::Float(0.0)),
        "real division",
    ))
}

/// An integer remainder by zero.
pub(super) fn modulo_by_zero() -> String {
    literal(&failure(
        arith::modulo(RuntimeValue::Int(1), RuntimeValue::Int(0)),
        "remainder",
    ))
}

/// An integer floor division by zero.
pub(super) fn floor_division_by_zero() -> String {
    literal(&failure(
        arith::floor_divide(RuntimeValue::Int(1), RuntimeValue::Int(0)),
        "floor division",
    ))
}

/// A floor division by zero with a `Real` operand.
pub(super) fn real_floor_division_by_zero() -> String {
    literal(&failure(
        arith::floor_divide(RuntimeValue::Float(1.0), RuntimeValue::Float(0.0)),
        "real floor division",
    ))
}

/// An exact division by zero.
pub(super) fn exact_division_by_zero() -> String {
    literal(&failure(
        arith::exact_divide(RuntimeValue::Int(1), RuntimeValue::Int(0)),
        "exact division",
    ))
}

/// An integer raised to a negative power.
pub(super) fn negative_exponent() -> String {
    literal(&failure(
        arith::power(RuntimeValue::Int(2), RuntimeValue::Int(-1)),
        "integer power",
    ))
}

/// `randomInt(lo, hi)` with `lo` above `hi` — the format takes both bounds, low first.
pub(super) fn random_bounds_refused() -> String {
    let (lo, hi) = (INT_HOLE_2, INT_HOLE);
    with_int_holes(
        &failure(
            crate::semantics::builtins::random_int(lo, hi).map(RuntimeValue::Int),
            "randomInt",
        ),
        lo,
        hi,
    )
}

/// An integer past i64 — the VM holds such a value as a big integer.
fn past_word() -> RuntimeValue {
    RuntimeValue::from_bigint(logicaffeine_base::BigInt::from_i64(2).pow(70))
}

/// An integer raised to a power past `u32`.
pub(super) fn exponent_too_large() -> String {
    literal(&failure(
        arith::power(
            RuntimeValue::Int(2),
            RuntimeValue::Int(i64::from(u32::MAX) + 1),
        ),
        "integer power",
    ))
}

/// An integer past i64 raised to a negative power, or to one past `u32`.
pub(super) fn big_exponent_refused() -> String {
    literal(&failure(
        arith::power(past_word(), RuntimeValue::Int(-1)),
        "big integer power",
    ))
}

/// An index past i64 where the VM refuses one (`why`: a list read, a Text read, a list store), as a plain C string
/// literal: the runtime's `lint_index_i64` reports it as it is, never as a format.
pub(super) fn index_too_wide(why: crate::semantics::failcode::FailCode) -> String {
    super::emit::c_string_literal(&crate::semantics::failcode::message(why.code(), 0))
}

/// A fraction, as the VM holds one.
fn a_fraction() -> RuntimeValue {
    RuntimeValue::from_rational(logicaffeine_base::Rational::from_ratio_i64(1, 2).expect("a nonzero denominator"))
}

/// [`collections::integer_place_refusals`]'s sentence for a whole number past i64, as a plain C string literal.
pub(super) fn integer_past_a_word(place: IntegerPlace) -> String {
    super::emit::c_string_literal(&collections::integer_place_refusals(place).0)
}

/// [`collections::integer_place_refusals`]'s sentence for a fraction, as a plain C string literal.
pub(super) fn fraction_for_an_integer(place: IntegerPlace) -> String {
    super::emit::c_string_literal(&collections::integer_place_refusals(place).1)
}

/// The sentences the exact-number section of the runtime stops a program with ([`super::runtime`]), by the global
/// each is held in, taken from the functions the VM fails with. Plain text: each runtime spells it as its own
/// string.
pub(crate) fn exact_number_sentences() -> Vec<(&'static str, String)> {
    use crate::ast::stmt::BinaryOpKind;
    let int = RuntimeValue::Int;
    let bitwise = |op: BinaryOpKind| failure(arith::binary_op(op, a_fraction(), int(1)), "bitwise operation on a fraction");
    let past = |op: BinaryOpKind| {
        let (left, right) = match op {
            BinaryOpKind::BitAnd => (
                RuntimeValue::from_bigint(logicaffeine_base::BigInt::from_i64(-2).pow(71)),
                int(6),
            ),
            _ => (past_word(), int(1)),
        };
        failure(arith::binary_op(op, left, right), "bitwise operation past i64")
    };
    vec![
        ("logos_says_division_by_zero", failure(arith::divide(int(1), int(0)), "integer division")),
        ("logos_says_modulo_by_zero", failure(arith::modulo(int(1), int(0)), "remainder")),
        ("logos_says_rational_division_by_zero", failure(arith::divide(a_fraction(), int(0)), "rational division")),
        (
            "logos_says_exact_floor_division_by_zero",
            failure(arith::floor_divide(a_fraction(), int(0)), "rational floor division"),
        ),
        ("logos_says_modulo_int_rational", failure(arith::modulo(int(3), a_fraction()), "remainder by a fraction")),
        ("logos_says_modulo_rational_int", failure(arith::modulo(a_fraction(), int(3)), "remainder of a fraction")),
        (
            "logos_says_modulo_rational_rational",
            failure(arith::modulo(a_fraction(), a_fraction()), "remainder of fractions"),
        ),
        ("logos_says_power_int_rational", failure(arith::power(int(2), a_fraction()), "power to a fraction")),
        (
            "logos_says_power_rational_rational",
            failure(arith::power(a_fraction(), a_fraction()), "power of a fraction to a fraction"),
        ),
        ("logos_says_power_int_big", failure(arith::power(int(2), past_word()), "power past i64")),
        ("logos_says_power_rational_big", failure(arith::power(a_fraction(), past_word()), "power past i64")),
        (
            "logos_says_rational_exponent_too_large",
            failure(arith::power(a_fraction(), int(i64::from(i32::MAX) + 1)), "rational power"),
        ),
        ("logos_says_negative_exponent", failure(arith::power(int(2), int(-1)), "integer power")),
        (
            "logos_says_exponent_too_large",
            failure(arith::power(int(2), int(i64::from(u32::MAX) + 1)), "integer power"),
        ),
        ("logos_says_big_exponent", failure(arith::power(past_word(), int(-1)), "big integer power")),
        ("logos_says_bitand_refused", past(BinaryOpKind::BitAnd)),
        ("logos_says_bitor_refused", past(BinaryOpKind::BitOr)),
        ("logos_says_bitxor_refused", past(BinaryOpKind::BitXor)),
        ("logos_says_shl_refused", past(BinaryOpKind::Shl)),
        ("logos_says_shr_refused", past(BinaryOpKind::Shr)),
        ("logos_says_bitand_fraction", bitwise(BinaryOpKind::BitAnd)),
        ("logos_says_bitor_fraction", bitwise(BinaryOpKind::BitOr)),
        ("logos_says_bitxor_fraction", bitwise(BinaryOpKind::BitXor)),
        ("logos_says_shl_fraction", bitwise(BinaryOpKind::Shl)),
        ("logos_says_shr_fraction", bitwise(BinaryOpKind::Shr)),
    ]
}

/// A remainder the VM refuses for these two operands (`Real % Real`).
pub(super) fn modulo_refused(left: RuntimeValue, right: RuntimeValue) -> String {
    literal(&failure(arith::modulo(left, right), "remainder"))
}

/// `is approximately` over these two operands, which are not both numbers.
pub(super) fn approx_refused(left: RuntimeValue, right: RuntimeValue) -> String {
    literal(&failure(
        arith::approx_eq(left, right),
        "approximate comparison",
    ))
}

/// `text contains part` with a part that is not Text.
pub(super) fn text_contains_refused(part: RuntimeValue) -> String {
    literal(&failure(
        collections::contains(&RuntimeValue::Text(Rc::new(String::new())), &part),
        "text containment",
    ))
}

/// The position a crafted input puts an invalid UTF-8 sequence at, so the VM's sentence carries it.
const UTF8_AT: usize = 7919;

/// `text_from_bytes` over bytes that are not UTF-8, as the four C formats its sentence can take — an
/// invalid sequence of 1, 2 or 3 bytes, and a sequence cut off by the end — each taking the offset at
/// which the bytes stop being UTF-8.
pub(super) fn invalid_utf8() -> [String; 4] {
    use crate::semantics::builtins::{call_builtin, BuiltinId};
    let fail = |tail: &[i64]| {
        let mut bytes = vec![i64::from(b'a'); UTF8_AT];
        bytes.extend_from_slice(tail);
        let list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
            bytes.into_iter().map(RuntimeValue::Int).collect(),
        ))));
        let sentence = failure(
            call_builtin(BuiltinId::TextFromBytes, vec![list]),
            "text_from_bytes",
        );
        let (before, after) = sentence
            .split_once(&UTF8_AT.to_string())
            .unwrap_or_else(|| {
                panic!("the VM's sentence {sentence:?} does not carry the offset it was given")
            });
        format!(
            "\"{}%\" PRId64 \"{}\"",
            c_format_body(before),
            c_format_body(after)
        )
    };
    [
        fail(&[0xFF]),
        fail(&[0xE0, 0xA0, 0x41]),
        fail(&[0xF0, 0x90, 0x80, 0x41]),
        fail(&[0xE0, 0xA0]),
    ]
}

/// A builtin the VM refuses for these arguments, in the VM's words.
pub(super) fn builtin_refused(
    id: crate::semantics::builtins::BuiltinId,
    args: Vec<RuntimeValue>,
) -> String {
    literal(&failure(
        crate::semantics::builtins::call_builtin(id, args),
        "builtin",
    ))
}

/// A sentence whose holes are the texts `holes` gives — each a `(placeholder in the sentence, argument name)`
/// — as a C format string with a `%s` per hole, and the argument names in the order the format takes them.
/// Each placeholder is found once, in order; a sentence missing one is a VM whose wording no longer carries
/// the value, which fails generation loudly.
fn with_text_holes(sentence: &str, holes: &[(&str, &'static str)]) -> (String, Vec<&'static str>) {
    let mut marked = sentence.to_string();
    for (i, (needle, name)) in holes.iter().enumerate() {
        let mark = format!("\u{2}{i}\u{2}");
        let at = marked.find(needle).unwrap_or_else(|| {
            panic!("the VM's sentence {sentence:?} does not carry {name} ({needle:?})")
        });
        marked.replace_range(at..at + needle.len(), &mark);
    }
    let mut order: Vec<(usize, usize, &'static str)> = (0..holes.len())
        .map(|i| {
            (
                marked.find(&format!("\u{2}{i}\u{2}")).unwrap(),
                i,
                holes[i].1,
            )
        })
        .collect();
    order.sort();
    let mut body = String::new();
    let mut rest = marked.as_str();
    for (_, i, _) in &order {
        let mark = format!("\u{2}{i}\u{2}");
        let at = rest.find(&mark).unwrap();
        body.push_str(&c_format_body(&rest[..at]));
        body.push_str("%s");
        rest = &rest[at + mark.len()..];
    }
    body.push_str(&c_format_body(rest));
    (
        format!("\"{body}\""),
        order.into_iter().map(|(_, _, name)| name).collect(),
    )
}

/// `Await` in a program that never `Listen`ed.
pub(super) fn await_without_listen() -> String {
    literal(crate::concurrency::net_inbox::AWAIT_WITHOUT_LISTEN)
}

/// `Send`/`Await` addressed with a value that is not an address.
pub(super) fn not_a_peer(value: RuntimeValue) -> String {
    match crate::concurrency::net_inbox::peer_topic(&value) {
        Err(sentence) => literal(&sentence),
        Ok(topic) => panic!(
            "the VM takes {} as the address {topic:?}",
            value.to_display_string()
        ),
    }
}

/// `Connect to <raw>` refused: the C format of the whole sentence, taking the address as written (`raw`) and
/// why it is not one (`why`).
pub(super) fn connect_refused() -> (String, Vec<&'static str>) {
    let why = logicaffeine_system::addr::multiaddr_to_ws_url(TEXT_HOLE)
        .expect_err("a hole is no address");
    let whole =
        crate::concurrency::net_inbox::relay_url(TEXT_HOLE).expect_err("a hole is no address");
    let outer = whole
        .strip_suffix(&why)
        .unwrap_or_else(|| panic!("the VM's sentence {whole:?} does not end with why"));
    let (fmt, mut args) = with_text_holes(outer, &[(TEXT_HOLE, "raw")]);
    args.push("why");
    (format!("{} \"%s\"", fmt), args)
}

/// Why an address is not a `ws://` URL or multiaddr, in `multiaddr_to_ws_url`'s words: each sentence as a C
/// format with the argument names it takes, found by handing the function an address that fails that way.
pub(super) struct AddressWords {
    pub empty: String,
    pub not_an_address: (String, Vec<&'static str>),
    pub no_host_after: (String, Vec<&'static str>),
    pub no_port_after_tcp: (String, Vec<&'static str>),
    pub non_numeric_port: (String, Vec<&'static str>),
    pub unsupported_protocol: (String, Vec<&'static str>),
    pub no_host: (String, Vec<&'static str>),
    pub no_port: (String, Vec<&'static str>),
}

pub(super) fn address_words() -> AddressWords {
    let why = |addr: &str| {
        logicaffeine_system::addr::multiaddr_to_ws_url(addr).expect_err("an address that fails")
    };
    let hole = TEXT_HOLE;
    let host_seg = format!("/p2p/{hole}/dns4");
    let port = format!("/p2p/{hole}/tcp/{hole}x");
    let unsupported = format!("/p2p/{hole}/{hole}z");
    AddressWords {
        empty: literal(&why("")),
        not_an_address: with_text_holes(&why(hole), &[(hole, "trimmed")]),
        no_host_after: with_text_holes(&why(&host_seg), &[(&host_seg, "trimmed"), ("dns4", "seg")]),
        no_port_after_tcp: with_text_holes(
            &why(&format!("/p2p/{hole}/tcp")),
            &[(&format!("/p2p/{hole}/tcp"), "trimmed")],
        ),
        non_numeric_port: with_text_holes(
            &why(&port),
            &[(&port, "trimmed"), (&format!("{hole}x"), "seg")],
        ),
        unsupported_protocol: with_text_holes(
            &why(&unsupported),
            &[(&unsupported, "trimmed"), (&format!("{hole}z"), "seg")],
        ),
        no_host: with_text_holes(
            &why(&format!("/p2p/{hole}/tcp/80")),
            &[(&format!("/p2p/{hole}/tcp/80"), "trimmed")],
        ),
        no_port: with_text_holes(
            &why(&format!("/p2p/{hole}/ip4/h")),
            &[(&format!("/p2p/{hole}/ip4/h"), "trimmed")],
        ),
    }
}

/// A read of the sandboxed file system that finds no file — the format takes the path.
pub(super) fn file_not_found() -> String {
    let e = logicaffeine_system::fs::VfsError::NotFound(TEXT_HOLE.to_string());
    with_text_hole(&crate::semantics::builtins::vfs_read_failure(&e))
}

/// `parseInt(text)` over text that is not an Int — the format takes the text as given.
pub(super) fn not_an_int() -> String {
    let text = RuntimeValue::Text(Rc::new(TEXT_HOLE.to_string()));
    with_text_hole(&failure(
        crate::semantics::builtins::call_builtin(
            crate::semantics::builtins::BuiltinId::ParseInt,
            vec![text],
        ),
        "parseInt",
    ))
}

/// `chr(code)` with a code that names no character — the format takes the code. The placeholder is a code
/// whose low 32 bits are a surrogate, which `char::from_u32` refuses.
pub(super) fn not_a_character() -> String {
    const CODE_HOLE: i64 = 0x7919_0000_D800;
    let sentence = failure(
        crate::semantics::builtins::call_builtin(
            crate::semantics::builtins::BuiltinId::Chr,
            vec![RuntimeValue::Int(CODE_HOLE)],
        ),
        "chr",
    );
    let (before, after) = sentence
        .split_once(&CODE_HOLE.to_string())
        .unwrap_or_else(|| {
            panic!("the VM's sentence {sentence:?} does not carry the code it was given")
        });
    format!(
        "\"{}%\" PRId64 \"{}\"",
        c_format_body(before),
        c_format_body(after)
    )
}

/// Why `tryReadBytes(path)` could not read a file — the Rust facade's sentence around the operating system's
/// reason, as a C format taking the path, then the reason's text and its number (`std::io::Error`'s display
/// of an OS error: the C library's description and ` (os error N)`).
pub(super) fn cannot_read_file() -> String {
    let sentence = logicaffeine_system::file::read_bytes(TEXT_HOLE.to_string())
        .expect_err("a hole names no file");
    let (before, after) = sentence.split_once(TEXT_HOLE).unwrap_or_else(|| {
        panic!("the facade's sentence {sentence:?} does not carry the path it was given")
    });
    let reason_starts = after
        .find(": ")
        .map(|i| i + 2)
        .unwrap_or_else(|| panic!("no reason in {sentence:?}"));
    format!(
        "\"{}%s{}%s (os error %d)\"",
        c_format_body(before),
        c_format_body(&after[..reason_starts])
    )
}
