//! Census: every Text-producing expression SHAPE, used as a concat operand, compiles and prints
//! the interpreter's own output on AOT-Rust AND C. Closes the class behind the reported
//! `p's label + p's label` bug (`aot_text_concat_operand_matrix_lock.rs`'s "not in this matrix"
//! note, `tiers.md` §1): a Text-typed FIELD read, concatenated with itself, with no call and no
//! literal on the other side of `+` to carry the "this is definitely Text" signal past the
//! concat-fold decision (`is_string_concat` in `codegen/expr.rs`). That decision can only see a
//! non-literal, non-call operand as Text through a DECLARED-TYPE lookup — and every one of those
//! lookups turned out to have its own blind spot once this census asked each shape the same
//! question against ITSELF (no call, no literal riding along on the other side):
//!
//! - `record_field_rust_type` required the object of a `FieldAccess` to be a bare `Identifier` —
//!   `o's inner's value` (the object of the OUTER access is the INNER `FieldAccess`) always
//!   returned `None`, so a nested field never registered as Text. Now recurses one level per
//!   `'s`.
//! - A Text-typed function PARAMETER is never entered into `string_vars` (only a `Let`-bound
//!   local is, at its own binding site) — `is_text_typed_identifier` closes this from
//!   `variable_types`, the parameter's own source of truth, instead.
//! - `is_definitely_string_expr_with_vars` has no `Expr::Index` arm at all — `item i of xs` /
//!   `item k of m` were invisible to the concat decision regardless of `xs`/`m`'s element type.
//!   `is_text_typed_index` closes this via `seq_element_rust_type` / the Map value type.
//!
//! `TextShape::source`'s `match` has NO wildcard arm: a new Text-producing expression kind added
//! to the language fails this file to compile until its row is written here — the census cannot
//! silently go stale (the same discipline `run_all_tiers`'s `Tier` match uses in `common/mod.rs`).
#![allow(dead_code)]

mod common;

/// Every Text-producing expression shape this census knows about, as a concat OPERAND.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextShape {
    Literal,
    Variable,
    Parameter,
    Call,
    Field,
    NestedField,
    SeqIndex,
    MapLookup,
    MethodBuiltin,
    ParenConcat,
}

impl TextShape {
    const ALL: [TextShape; 10] = [
        TextShape::Literal,
        TextShape::Variable,
        TextShape::Parameter,
        TextShape::Call,
        TextShape::Field,
        TextShape::NestedField,
        TextShape::SeqIndex,
        TextShape::MapLookup,
        TextShape::MethodBuiltin,
        TextShape::ParenConcat,
    ];

    /// A transient shape (a fresh call result, a literal, a parenthesised concat) has no
    /// persistent NAMED binding left to reread afterward. Only a shape bound to a name — a
    /// variable, a parameter, a field, an index — can prove its concat read BORROWED the value
    /// rather than moving it, by rereading the same name once more after the concat.
    fn has_rereadable_binding(self) -> bool {
        !matches!(
            self,
            TextShape::Literal
                | TextShape::Call
                | TextShape::MethodBuiltin
                | TextShape::ParenConcat
        )
    }

    /// LEFT position (`shape + "Q"`), RIGHT position (`"Q" + shape`), SELF-PAIR (`shape +
    /// shape` — the exact class the reported bug is in: the SAME non-literal, non-call shape on
    /// both sides), and, where meaningful, a final bare REREAD proving the shape's own binding
    /// still holds its value after appearing in three concats.
    fn source(self) -> &'static str {
        match self {
            TextShape::Literal => {
                "\
## Main
    Show \"AB\" + \"Q\".
    Show \"Q\" + \"AB\".
    Show \"AB\" + \"AB\".
"
            }
            TextShape::Variable => {
                "\
## Main
    Let v be \"AB\".
    Show v + \"Q\".
    Show \"Q\" + v.
    Show v + v.
    Show v.
"
            }
            // A bare Text PARAMETER: never entered into `string_vars` (only a `Let`-bound local
            // is), so `t + t` fell through to a raw `+` exactly like the reported field bug.
            TextShape::Parameter => {
                "\
## To showParam (t: Text):
    Show t + \"Q\".
    Show \"Q\" + t.
    Show t + t.
    Show t.

## Main
    showParam(\"AB\").
"
            }
            TextShape::Call => {
                "\
## To makeText () -> Text:
    Return \"AB\".

## Main
    Show makeText() + \"Q\".
    Show \"Q\" + makeText().
    Show makeText() + makeText().
"
            }
            // The reported bug's own shape: a Text-typed struct FIELD, both operands of `+`.
            TextShape::Field => {
                "\
## A Parcel has:
    A label: Text.

## Main
    Let p be a new Parcel with label \"AB\".
    Show p's label + \"Q\".
    Show \"Q\" + p's label.
    Show p's label + p's label.
    Show p's label.
"
            }
            // `o's inner's value` — the object of the OUTER `FieldAccess` is itself a
            // `FieldAccess`, not a bare identifier: `record_field_rust_type`'s original
            // one-level-only lookup always missed this.
            TextShape::NestedField => {
                "\
## A Inner has:
    A value: Text.

## A Outer has:
    A inner: Inner.

## Main
    Let i be a new Inner with value \"AB\".
    Let o be a new Outer with inner i.
    Show o's inner's value + \"Q\".
    Show \"Q\" + o's inner's value.
    Show o's inner's value + o's inner's value.
    Show o's inner's value.
"
            }
            // Routed through a function PARAMETER (not a bare Main-level local): a `Seq of Text`
            // built entirely from literals is folded to its literal elements by the partial
            // evaluator, which would make this row pass trivially regardless of whether
            // `is_text_typed_index` exists at all — a parameter boundary keeps `xs` opaque to
            // that fold, so the row genuinely exercises the concat-fold decision at RUNTIME.
            TextShape::SeqIndex => {
                "\
## To showSeqIndex (xs: Seq of Text):
    Show item 1 of xs + \"Q\".
    Show \"Q\" + item 1 of xs.
    Show item 1 of xs + item 1 of xs.
    Show item 1 of xs.

## Main
    Let mutable xs be a new Seq of Text.
    Push \"AB\" to xs.
    showSeqIndex(xs).
"
            }
            TextShape::MapLookup => {
                "\
## Main
    Let mutable m be a new Map of Text to Text.
    Set item \"k\" of m to \"AB\".
    Show item \"k\" of m + \"Q\".
    Show \"Q\" + item \"k\" of m.
    Show item \"k\" of m + item \"k\" of m.
    Show item \"k\" of m.
"
            }
            TextShape::MethodBuiltin => {
                "\
## Main
    Show upper(\"ab\") + \"Q\".
    Show \"Q\" + upper(\"ab\").
    Show upper(\"ab\") + upper(\"ab\").
"
            }
            TextShape::ParenConcat => {
                "\
## Main
    Show (\"A\" + \"B\") + \"Q\".
    Show \"Q\" + (\"A\" + \"B\").
    Show (\"A\" + \"B\") + (\"A\" + \"B\").
"
            }
        }
    }
}

/// Runs the interpreter once to get the reference output (so no expected string is hand-derived
/// and no shape can drift from what the reference engine actually says), then holds AOT-Rust and
/// C to that SAME output — `assert_compiled_equals_interpreted` for the Rust tier,
/// `assert_c_output` (which needs a literal, not a differential) fed the interpreter's own text
/// for the C tier.
fn check_shape(shape: TextShape) {
    let src = shape.source();
    let interp = common::run_interpreter(src);
    assert!(
        interp.success,
        "{:?}: interpreter should succeed.\nSource:\n{src}\nError: {}",
        shape, interp.error
    );
    let expected = interp.output.trim().to_string();
    let expected_rows = if shape.has_rereadable_binding() { 4 } else { 3 };
    assert_eq!(
        expected.lines().count(),
        expected_rows,
        "{:?}: expected {expected_rows} `Show` rows, interpreter printed {:?}",
        shape,
        expected
    );
    common::assert_compiled_equals_interpreted(src);
    common::assert_c_output(src, &expected);
}

#[test]
fn census_literal() {
    check_shape(TextShape::Literal);
}

#[test]
fn census_variable() {
    check_shape(TextShape::Variable);
}

#[test]
fn census_parameter() {
    check_shape(TextShape::Parameter);
}

#[test]
fn census_call() {
    check_shape(TextShape::Call);
}

#[test]
fn census_field() {
    check_shape(TextShape::Field);
}

#[test]
fn census_nested_field() {
    check_shape(TextShape::NestedField);
}

#[test]
fn census_seq_index() {
    check_shape(TextShape::SeqIndex);
}

#[test]
fn census_map_lookup() {
    check_shape(TextShape::MapLookup);
}

#[test]
fn census_method_builtin() {
    check_shape(TextShape::MethodBuiltin);
}

#[test]
fn census_paren_concat() {
    check_shape(TextShape::ParenConcat);
}

/// Belt-and-suspenders over the ten dedicated tests above: walks `TextShape::ALL` itself, so the
/// array the census is defined over is exercised too, not just each hand-written `#[test]`.
#[test]
fn every_declared_text_shape_concats_on_aot_rust_and_c() {
    for shape in TextShape::ALL {
        check_shape(shape);
    }
}
