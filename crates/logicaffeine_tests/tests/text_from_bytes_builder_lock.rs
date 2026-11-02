//! Lock for `Opt::TextFromBytesBuilder` — the write-side byte-builder pass.
//!
//! A `Let out be a new Seq of Int` used ONLY as a byte builder (`Push <byte> to out`) and
//! consumed by `text_from_bytes(out)` lowers to a bare `Vec<u8>` plus a direct
//! `String::from_utf8(out)` — eliminating the `Rc<RefCell<Vec<i64>>>` output (8x memory +
//! per-push `borrow_mut`) and the separate `i64`->`u8` rebuild pass the builtin performed.
//! The transform gates off under `## No textfrombytesbuilder`.

use logicaffeine_compile::compile::compile_to_rust;
use logicaffeine_compile::optimization::decorate_source;

/// The canonical ASCII-upper idiom (a Text PARAM `s`, so the read-side view fires too), driven by
/// a RUNTIME argument so the whole function is not constant-folded away.
const UPPER: &str = "\
## To native args () -> Seq of Text

## To upperOne (s: Text) -> Text:
    Let sb be text_bytes(s).
    Let out be a new Seq of Int.
    Repeat for bt in sb:
        Let mutable cv be bt.
        If bt is at least 97:
            If bt is at most 122:
                Set cv to bt - 32.
        Push cv to out.
    Return text_from_bytes(out).

## Main
Let arguments be args().
Show upperOne(item 2 of arguments).
";

/// A NEGATIVE idiom: `out` is also READ (`item 1 of out`), so the byte-builder analysis must bail
/// and leave the `text_from_bytes(out)` round-trip intact (soundness gate).
const READ_TOO: &str = "\
## To native args () -> Seq of Text

## To firstByteEcho (s: Text) -> Text:
    Let sb be text_bytes(s).
    Let out be a new Seq of Int.
    Repeat for bt in sb:
        Push bt to out.
    Let head be item 1 of out.
    Push head to out.
    Return text_from_bytes(out).

## Main
Let arguments be args().
Show firstByteEcho(item 2 of arguments).
";

#[test]
fn text_from_bytes_builder_lowers_to_vec_u8() {
    let rust = compile_to_rust(UPPER).expect("UPPER compiles");
    assert!(
        rust.contains(": Vec<u8> = Vec::new()"),
        "the byte builder `out` must lower to a bare Vec<u8>; got:\n{rust}"
    );
    assert!(
        rust.contains("String::from_utf8("),
        "the consume must be a direct String::from_utf8; got:\n{rust}"
    );
    // The builtin CALL form is `text_from_bytes(&<var>)`; distinguish it from the runtime panic
    // message string (`text_from_bytes():`) that our direct `String::from_utf8` emission carries.
    assert!(
        !rust.contains("text_from_bytes(&"),
        "the text_from_bytes builtin round-trip must be eliminated; got:\n{rust}"
    );
}

#[test]
fn text_from_bytes_builder_gates_off() {
    let off = decorate_source(UPPER, &["textfrombytesbuilder"]);
    let rust = compile_to_rust(&off).expect("disabled UPPER compiles");
    assert!(
        rust.contains("text_from_bytes("),
        "`## No textfrombytesbuilder` must keep the builtin round-trip; got:\n{rust}"
    );
    assert!(
        !rust.contains(": Vec<u8> = Vec::new()"),
        "disabled: `out` must stay a LogosSeq, not a Vec<u8>; got:\n{rust}"
    );
}

#[test]
fn text_from_bytes_builder_bails_when_out_is_read() {
    let rust = compile_to_rust(READ_TOO).expect("READ_TOO compiles");
    assert!(
        rust.contains("text_from_bytes("),
        "reading `out` (item 1 of out) must force the LogosSeq fallback; got:\n{rust}"
    );
}
