//! LOCK — a function the program declares is called by its name, whatever word the name is.
//!
//! `## To note (x: Int):` declares a function a `## To` header accepts under any word, and
//! `note(3).` is a call of it. The statement parser tried its keyword forms first, so a declared
//! name that is also a statement word lost to that word's form: `note(3).` became the observation
//! `Note that …` and silently did nothing (`warn`, `record` alike), and `expect(3).`,
//! `advance(3).`, `during(3).` were refused as malformed test or timing sentences. A declared
//! function's name followed by its argument list is a call before it is anything else.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Statement words a program may also declare as a function.
const WORDS: &[&str] = &[
    "note", "warn", "record", "during", "expect", "advance", "trust", "solve", "minimize",
    "launch", "stop", "receive", "show", "check", "sleep", "sync", "mount", "listen", "connect",
    "spawn", "serve",
];

fn program(word: &str) -> String {
    format!("## To {word} (x: Int):\n    Show x + 1.\n\n## Main\n{word}(3).\nShow \"after\".\n")
}

#[test]
fn a_declared_function_named_like_a_statement_word_is_called_as_a_statement() {
    let mut wrong = Vec::new();
    for word in WORDS {
        let src = program(word);
        for (tier, out) in [("tree-walker", tw_outcome(&src)), ("VM", vm_outcome(&src))] {
            let got = (out.output.trim().to_string(), out.error.clone());
            if got != ("4\nafter".to_string(), None) {
                wrong.push(format!("`{word}(3).` on the {tier}: {got:?}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "each call must run its function:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn a_statement_word_that_names_no_function_keeps_its_own_meaning() {
    // Anti-vacuity: with no `## To show`, `Show (1 + 2).` is the Show statement over a
    // parenthesised expression, not a call.
    let out = tw_outcome("## Main\nShow (1 + 2).\n");
    assert_eq!((out.output.trim(), out.error.as_deref()), ("3", None));
}
