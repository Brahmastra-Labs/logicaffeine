//! LOCK — `mutable` is optional intent everywhere a `Let` binds a name: `State:` already treats
//! every field as writable (`Set`/`Toggle`/`Push`/… reach any of them), so `Let mutable x be …` in a
//! `State:` section is accepted as a no-op — the same field `Let x be …` would declare, with the
//! word carrying no meaning the parser enforces. Before this pin, `parse_state_section` read the
//! field name straight after `Let` and `mutable` was taken AS that name, so the section then
//! expected `be` where the real field name stood and refused with `ExpectedKeyword("be")`.

#[path = "language_common/mod.rs"]
mod common;

use common::{accepted, refusal};
use logicaffeine_language::ast::Stmt;

fn state_field_names(src: &str) -> Vec<String> {
    let parsed = accepted(src);
    let mut out = Vec::new();
    for s in &parsed.stmts {
        if let Stmt::ComponentDef(def) = s {
            for f in &def.state {
                out.push(parsed.interner.resolve(f.name).to_string());
            }
        }
    }
    out
}

#[test]
fn a_mutable_state_field_declares_its_own_name_not_the_word_mutable() {
    let names = state_field_names(
        "## Component App\nState:\n    Let mutable draft be \"\".\nView:\n    <p>{draft}</p>\n\n## Main\nMount App.\n",
    );
    assert_eq!(names, vec!["draft".to_string()]);
}

#[test]
fn mut_is_the_same_no_op_as_mutable() {
    let names = state_field_names(
        "## Component App\nState:\n    Let mut draft be \"\".\nView:\n    <p>{draft}</p>\n\n## Main\nMount App.\n",
    );
    assert_eq!(names, vec!["draft".to_string()]);
}

#[test]
fn a_typed_mutable_field_keeps_its_declared_type() {
    let names = state_field_names(
        "## Component App\nState:\n    Let mutable count: Int be 0.\nView:\n    <p>{count}</p>\n\n## Main\nMount App.\n",
    );
    assert_eq!(names, vec!["count".to_string()]);
}

#[test]
fn a_derived_field_may_not_also_be_mutable() {
    // `always` marks a field computed from the rest of the state; declaring it `mutable` too
    // contradicts that, and the section still needs its own error rather than a confusing one about
    // a missing `be`. `mutable always` is refused, naming the conflict.
    let (message, _) = refusal(
        "## Component App\nState:\n    Let mutable total always be 1.\nView:\n    <p>{total}</p>\n\n## Main\nMount App.\n",
    );
    assert!(
        message.contains("mutable") && message.contains("always"),
        "message: {message}"
    );
}

#[test]
fn a_plain_field_is_unaffected() {
    let names = state_field_names(
        "## Component App\nState:\n    Let draft be \"\".\nView:\n    <p>{draft}</p>\n\n## Main\nMount App.\n",
    );
    assert_eq!(names, vec!["draft".to_string()]);
}
