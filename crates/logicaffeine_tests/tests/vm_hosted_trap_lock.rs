//! ★ LOCK — a hosted program's failed mutation leaves its module-level collection intact. ★
//!
//! A page's host calls into a program that has already run `main` — an event handler, a completion —
//! and a call that traps leaves the program callable ([`logicaffeine_compile::vm::session::Session`]).
//! What a trapped call leaves behind is therefore observable: the next call reads the same globals.
//! This is the one place a mutation's failure safety can be seen at all.

use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_compile::vm::session::Session;
use logicaffeine_compile::vm::{Compiler, Vm};

/// ★ A MUTATION OF A MODULE-LEVEL COLLECTION THAT FAILS LEAVES IT EXACTLY AS IT WAS. A write to an
/// author's global reads the collection into a register, mutates it and stores it back; the global
/// keeps its own reference throughout, and a mutating op either completes or fails before it writes.
/// So a hosted program whose function traps partway through — after one write landed, at a second
/// that fails — still holds the first write and nothing of the second, and the next call sees it.
/// A lowering that took the global's value out (storing `nothing` in its place) would leave it empty.
#[test]
fn a_hosted_function_that_traps_mid_mutation_leaves_the_global_intact() {
    let src = "## To grow(n: Int):\n    Push n to items.\n    Set item 9 of items to n.\n\n\
               ## To count -> Int:\n    Return length of items.\n\n\
               ## Main\nLet mutable items be [1, 2].\nShow length of items.\n";
    let outcome =
        logicaffeine_compile::ui_bridge::with_parsed_program_checked(src, |parsed, interner| {
            let (stmts, types, _) = parsed?;
            let program =
                Compiler::compile_with_oracle_located(stmts, interner, Some(types), None)?;
            let mut session = Session::new(Vm::new(&program), interner);
            session.run_main().expect("main runs");
            let trapped = session.invoke("grow", vec![RuntimeValue::Int(7)]);
            let after = session.global_text("items");
            let count = session.invoke("count", Vec::new());
            Ok((trapped, after, count.map(|v| v.to_display_string())))
        })
        .unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
    let (trapped, after, count) = outcome;
    assert!(
        trapped.is_err(),
        "`Set item 9 of` a three-item list must trap"
    );
    assert_eq!(
        after.as_deref(),
        Some("[1, 2, 7]"),
        "the push landed and the failed write left nothing"
    );
    assert_eq!(
        count.as_deref(),
        Ok("3"),
        "the next call sees the global intact"
    );
}
