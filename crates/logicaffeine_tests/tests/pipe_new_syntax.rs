//! Regression: `Let p be a **new** Pipe of T.` must parse.
//!
//! Every other collection construction in the language uses the `a new X` idiom
//! (`a new Map of …`, `a new Set of …`, `a new Point with …`), and the guide's
//! concurrency section writes `Let jobs be a new Pipe of Int.`. The channel-creation
//! parser only matched `a Pipe of T` — it checked for the `Pipe` token immediately
//! after the article and never skipped the optional `new`, so the documented syntax
//! failed to parse (and `largo build` of any guide pipe example died before codegen).

use logicaffeine_compile::compile_to_rust;

#[test]
fn new_pipe_construction_parses_and_compiles() {
    let src = r#"## Main
Let messages be a new Pipe of Int.
Send 42 into messages.
Receive x from messages.
Show x."#;
    let rust = compile_to_rust(src)
        .expect("`a new Pipe of Int` must parse and compile (the documented channel syntax)");
    // A channel must actually be created in the generated Rust.
    assert!(
        rust.contains("channel") || rust.contains("Sender") || rust.contains("Receiver"),
        "expected channel codegen for a Pipe, got:\n{rust}"
    );
}

#[test]
fn bare_pipe_construction_still_parses() {
    // The pre-existing `a Pipe of T` form (no `new`) must keep working.
    let src = r#"## Main
Let messages be a Pipe of Int.
Send 7 into messages.
Show "ok"."#;
    compile_to_rust(src).expect("`a Pipe of Int` (no new) must still parse");
}
