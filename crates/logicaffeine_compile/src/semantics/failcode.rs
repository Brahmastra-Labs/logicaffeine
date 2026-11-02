//! The failure codes a compiled module reports, each mapped onto its sentence in `logicaffeine_data::failure`,
//! the one place every tier's failure wording lives.
//!
//! A runtime failure comes in two shapes. Most carry a message the compiler already wrote and
//! interned (`Op::FailWith { msg: ConstIdx }`), so the wasm backend hands that Text straight to
//! the host. But an INLINE check — a bounds test, a division guard — has no constant: the
//! tree-walker builds its message while running, from a value that does not exist until then
//! (`Index 9 out of bounds`). There is nothing to hand over.
//!
//! So those report a CODE and the offending VALUE, and the host renders the sentence.
//!
//! ★ WHICH PUTS THE WORDING IN A HOST AS WELL, AND THAT IS THE WHOLE DANGER. The browser saying
//! something subtly different from the reference tier is exactly the drift every lock in this
//! campaign exists to prevent. So [`message`] is what the interpreter formats with, what the Node
//! host is generated from ([`js_cases`]), and what `runtime.js` is checked against
//! (`fail_code_lock` reads its cases and holds them to this function). A new code that the host
//! does not render fails that lock rather than reaching a visitor as a blank page.
//!
//! The codes are APPEND-ONLY for the same reason the host-import list is: the emitter is compiled
//! into a module and the host ships separately, so renumbering silently re-labels every failure a
//! previously-built module reports.

use std::cell::RefCell;
use std::rc::Rc;

use logicaffeine_base::BigInt;

use crate::interpreter::{ListRepr, RuntimeValue};

/// Why a compiled module stopped, for the failures whose message needs a runtime value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum FailCode {
    /// `item i of xs` where `i` names no element. The value is the index the program asked for.
    IndexOutOfBounds = 1,
    /// An integer division or modulo by zero. The value is unused.
    DivisionByZero = 2,
    /// `text_from_bytes(bytes)` where the bytes are not valid UTF-8. The value is unused: the
    /// reference tier appends the standard library's own description of the fault, which a
    /// compiled module has no way to reproduce, so the module reports the sentence they share.
    InvalidUtf8 = 3,
    /// The allocator could not grow linear memory far enough for the next allocation. The value
    /// is unused: there is nothing more specific to report than the shape of the failure itself.
    OutOfMemory = 4,
    /// `item i of xs` on a list, where `i` is an integer past a machine word. The value is unused: the
    /// reference tier names the index's type, not its digits.
    ListIndexTooWide = 5,
    /// `item i of t` on a Text, where `i` is an integer past a machine word. The value is unused.
    TextIndexTooWide = 6,
    /// `Set item i of xs to v`, where `i` is an integer past a machine word. The value is unused.
    ListStoreIndexTooWide = 7,
    /// An integer past a machine word reaching a place the module holds a machine word: an element, a
    /// field, a builtin's argument. The value is unused.
    IntTooWide = 8,
}

impl FailCode {
    /// The wire value the module reports. Append-only.
    pub fn code(self) -> i32 {
        self as i32
    }
}

/// Every code, in wire order — so a lock can enumerate them rather than trusting a hand-written list.
pub const ALL: &[FailCode] = &[
    FailCode::IndexOutOfBounds,
    FailCode::DivisionByZero,
    FailCode::InvalidUtf8,
    FailCode::OutOfMemory,
    FailCode::ListIndexTooWide,
    FailCode::TextIndexTooWide,
    FailCode::ListStoreIndexTooWide,
    FailCode::IntTooWide,
];

/// Every code's sentence as JavaScript: one `case` arm per code assigning `target`, then a `default` arm,
/// with the offending value spliced in from the variable `value` (a JS string) and an unknown code from the
/// variable `code`. Rendered from [`message`] itself, so a host written out as JavaScript cannot word a
/// failure differently from the reference tier.
pub fn js_cases(target: &str, value: &str, code: &str) -> String {
    const VALUE: i64 = -7_919_000_000_000_000_007;
    const UNKNOWN: i32 = -7919;
    let js = |text: String, hole: &str, var: &str| {
        let escaped = text
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n");
        format!("'{}'", escaped.replace(hole, &format!("' + {var} + '")))
    };
    let mut arms: Vec<String> = ALL
        .iter()
        .map(|c| {
            format!(
                "case {}: {target} = {}; break;",
                c.code(),
                js(message(c.code(), VALUE), &VALUE.to_string(), value)
            )
        })
        .collect();
    arms.push(format!(
        "default: {target} = {};",
        js(message(UNKNOWN, 0), &UNKNOWN.to_string(), code)
    ));
    arms.join(" ")
}

/// The sentence a failure reports, from its code and the value that caused it.
///
/// This is the single definition: the interpreter formats with it, and the browser host is held
/// to it. An unknown code is named rather than guessed — a module built after the host would
/// otherwise report a plausible-looking wrong reason.
pub fn message(code: i32, value: i64) -> String {
    match code {
        c if c == FailCode::IndexOutOfBounds.code() => {
            logicaffeine_data::failure::index_out_of_bounds(value)
        }
        c if c == FailCode::DivisionByZero.code() => {
            logicaffeine_data::failure::DIVISION_BY_ZERO.to_string()
        }
        c if c == FailCode::InvalidUtf8.code() => "text_from_bytes(): invalid UTF-8".to_string(),
        c if c == FailCode::OutOfMemory.code() => "Out of memory".to_string(),
        c if c == FailCode::ListIndexTooWide.code() => wide_index_refusal(empty_list()),
        c if c == FailCode::TextIndexTooWide.code() => {
            wide_index_refusal(RuntimeValue::Text(Rc::new(String::new())))
        }
        c if c == FailCode::ListStoreIndexTooWide.code() => {
            crate::semantics::collections::index_set(
                &empty_list(),
                &past_the_word(),
                RuntimeValue::Nothing,
            )
            .expect_err("a list refuses a store at an index past a machine word")
        }
        c if c == FailCode::IntTooWide.code() => {
            logicaffeine_data::failure::INT_TOO_WIDE.to_string()
        }
        other => format!("the program failed with an unrecognized code {other}"),
    }
}

/// An integer one past the largest machine word.
fn past_the_word() -> RuntimeValue {
    RuntimeValue::from_bigint(BigInt::from_i64(i64::MAX).add(&BigInt::from_i64(1)))
}

fn empty_list() -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(Vec::new()))))
}

/// What the reference tier says when `collection` is indexed by an integer past a machine word: its own
/// sentence, asked of the function it indexes with.
fn wide_index_refusal(collection: RuntimeValue) -> String {
    crate::semantics::collections::index_get(&collection, &past_the_word())
        .expect_err("a collection refuses an index past a machine word")
}
