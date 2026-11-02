//! A type nothing declares is refused where it is named; a type something supplies never is.
//!
//! `a new Todo` with no `Todo` anywhere used to compile and run, showing `Todo` — the silence that let a
//! misplaced `Import` go unnoticed until a derived decoder was missing at run time. A written type
//! must now be supplied: declared or imported by the program, built into the language (a scalar,
//! a collection, `Option`/`Result`, a CRDT, a dated or exact value type), a compiler-provided record,
//! or a type parameter the declaration itself introduces. Anything else is refused at the statement
//! that names it, and the message names the type.

use logicaffeine_compile::compile::compile_to_rust;

fn refusal(src: &str) -> Option<String> {
    compile_to_rust(src).err().map(|e| format!("{e:?}"))
}

#[test]
fn a_type_nothing_supplies_is_refused_wherever_it_is_named() {
    let rows: &[(&str, &str, &str)] = &[
        (
            "a construction",
            "## Main\nLet t be a new Todo.\nShow t.\n",
            "Todo",
        ),
        (
            "a parameter type",
            "## To keep (w: Widget) -> Int:\n    Return 1.\n\n## Main\nShow 1.\n",
            "Widget",
        ),
        (
            "a return type",
            "## To make -> Gadget:\n    Return 1.\n\n## Main\nShow 1.\n",
            "Gadget",
        ),
        (
            "a collection's element type",
            "## Main\nLet xs be a new Seq of Gizmo.\nShow length of xs.\n",
            "Gizmo",
        ),
        (
            "an annotation",
            "## To get -> Int:\n    Return 3.\n\n## Main\nLet n: Sprocket be get().\nShow n.\n",
            "Sprocket",
        ),
        (
            "a record's field type",
            "## A Holder has:\n    A part: Cog.\n\n## Main\nShow 1.\n",
            "Cog",
        ),
    ];
    for (what, src, name) in rows {
        let refused = refusal(src).unwrap_or_else(|| {
            panic!("{what}: `{name}` names nothing and must be refused:\n{src}")
        });
        assert!(
            refused.contains(&format!(
                "`{name}` is not a type this program declares or imports"
            )),
            "{what}: the refusal must name `{name}`:\n{refused}"
        );
    }
}

#[test]
fn a_type_something_supplies_is_never_refused() {
    let rows: &[(&str, &str)] = &[
        ("a type declared below its first use", "## Main\nLet p be a new Point with x 1.\nShow p's x.\n\n## A Point has:\n    An x: Int.\n"),
        ("a generic function's own parameter", "## To first of [T] (xs: Seq of T) -> T:\n    Return item 1 of xs.\n\n## Main\nShow first([4, 5]).\n"),
        ("a generic record's own parameter", "## A Box of [T] has:\n    A value: T.\n\n## Main\nLet b be a new Box of Int with value 3.\nShow b's value.\n"),
        ("the exact and dated value types", "## To keep (m: Money, d: Date, u: Uuid, q: Rational, k: Decimal, s: Span, t: Duration) -> Int:\n    Return 1.\n\n## Main\nShow 1.\n"),
        ("Option, Result and Maybe", "## To f (a: Option of Int, b: Result of Int and Text, c: Maybe of Text) -> Int:\n    Return 1.\n\n## Main\nShow 1.\n"),
        ("the written collection names", "## To g (a: Seq of Int, b: List of Text, c: Map of Text to Int, d: Set of Int) -> Int:\n    Return 1.\n\n## Main\nShow 1.\n"),
        ("a CRDT field", "## A Counter is Shared and has:\n    A clicks: ConvergentCount.\n    A tags: SharedSet of Text.\n\n## Main\nLet c be a new Counter.\nShow 1.\n"),
        ("a compiler-provided record", "## Main\nLet e be a new HttpError with status 404 and message \"missing\".\nShow e's status.\n"),
        ("the machine words", "## To w (a: Word32, b: Word64, c: Byte) -> Int:\n    Return 1.\n\n## Main\nShow 1.\n"),
    ];
    for (what, src) in rows {
        if let Some(refused) = refusal(src) {
            assert!(
                !refused.contains("is not a type this program declares or imports"),
                "{what}: a supplied type was refused as undeclared:\n{src}\n{refused}"
            );
        }
    }
}
