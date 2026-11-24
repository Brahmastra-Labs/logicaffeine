//! ★ LOCK — a policy predicate that names itself is refused when its C is written, and the writing ends. ★
//!
//! A `## Policy` predicate defined through itself (`A User is odd if the user is odd.`, or two that name each other)
//! has no condition to lower: resolving it never reaches a field. Each C emitter says so (`a policy predicate that
//! names itself`) and returns, as it does for any construct it does not lower; a predicate that reaches its fields
//! through others is lowered as before. Every compile here runs under a deadline, so a writer that resolves forever
//! fails the row instead of the run.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test c_policy_cycle_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use logicaffeine_compile::compile::{compile_to_c, compile_to_c_via_bytecode};

/// How long writing one program's C may take.
const DEADLINE: Duration = Duration::from_secs(60);

/// What `emit` answers for `src`, or `None` when it has not answered by [`DEADLINE`] (or died).
fn within_deadline(
    emit: fn(&str) -> Result<String, logicaffeine_compile::ParseError>,
    src: &'static str,
) -> Option<Result<String, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let _ = tx.send(emit(src).map_err(|e| format!("{e:?}")));
        })
        .expect("a compile thread");
    rx.recv_timeout(DEADLINE).ok()
}

const NAMES_ITSELF: &str = "## Definition\nA User has:\n    a role: Text.\n\n## Policy\nA User is odd if the user is odd.\n\n## Main\nLet u be a new User with role \"x\".\nCheck that u is odd.\n";

const NAME_EACH_OTHER: &str = "## Definition\nA User has:\n    a role: Text.\n\n## Policy\nA User is odd if the user is even.\nA User is even if the user is odd.\n\n## Main\nLet u be a new User with role \"x\".\nCheck that u is even.\n";

const REACHES_ITS_FIELDS: &str = "## Definition\nA User has:\n    a role: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\nA User is staff if the user is admin.\n\n## Main\nLet u be a new User with role \"admin\".\nCheck that u is staff.\nShow \"staff\".\n";

/// AST C refuses a predicate that names itself, directly or through another, and returns.
#[test]
fn ast_c_refuses_a_predicate_that_names_itself() {
    for src in [NAMES_ITSELF, NAME_EACH_OTHER] {
        match within_deadline(compile_to_c, src) {
            None => panic!("AST C did not finish writing within {DEADLINE:?}:\n{src}"),
            Some(Ok(c)) => assert!(
                c.contains("a policy predicate that names itself"),
                "AST C lowered a predicate with no condition to lower:\n{src}"
            ),
            Some(Err(why)) => assert!(why.contains("a policy predicate that names itself"), "{why}\n{src}"),
        }
    }
}

/// Bytecode C refuses a predicate that names itself, directly or through another, and returns.
#[test]
fn bytecode_c_refuses_a_predicate_that_names_itself() {
    for src in [NAMES_ITSELF, NAME_EACH_OTHER] {
        match within_deadline(compile_to_c_via_bytecode, src) {
            None => panic!("bytecode C did not finish writing within {DEADLINE:?}:\n{src}"),
            Some(Ok(c)) => assert!(
                c.contains("a policy predicate that names itself"),
                "bytecode C lowered a predicate with no condition to lower:\n{src}"
            ),
            Some(Err(why)) => assert!(why.contains("a policy predicate that names itself"), "{why}\n{src}"),
        }
    }
}

/// A predicate that reaches its fields through another is lowered by both, with no refusal.
#[test]
fn a_predicate_through_another_is_still_lowered() {
    for (emitter, emit) in [("AST C", compile_to_c as fn(&str) -> _), ("bytecode C", compile_to_c_via_bytecode)] {
        let c = within_deadline(emit, REACHES_ITS_FIELDS)
            .unwrap_or_else(|| panic!("{emitter} did not finish within {DEADLINE:?}"))
            .unwrap_or_else(|why| panic!("{emitter} refused:\n{why}"));
        assert!(!c.contains("#error"), "{emitter} refused part of:\n{REACHES_ITS_FIELDS}\n{c}");
    }
}
