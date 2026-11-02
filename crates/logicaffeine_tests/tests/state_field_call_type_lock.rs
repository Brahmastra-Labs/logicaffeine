//! LOCK — a `State:` field initialised by a plain call to a `## To` function this program
//! declares is typed from that function's own declared return type: `Let w be noWidget().` is
//! exactly as typed as `Let w: Widget be noWidget().`, without the author writing the type out
//! twice. Before this, the field stayed untyped, which the browser backend cannot lower (it has
//! no record type to give the field's slot), so the sentence that looks the most ordinary — call
//! a function, keep what it returns — was the one state initializer that failed downstream of
//! every check that runs before lowering.

#[path = "language_common/mod.rs"]
mod common;

use common::accepted;
use logicaffeine_language::ast::{Stmt, TypeExpr};

fn state_field_ty(src: &str) -> Option<String> {
    let parsed = accepted(src);
    for s in &parsed.stmts {
        let Stmt::ComponentDef(def) = s else { continue };
        if parsed.interner.resolve(def.name) != "App" {
            continue;
        }
        let field = def.state.first().expect("App declares one state field");
        return field.ty.map(|t| match t {
            TypeExpr::Named(s) | TypeExpr::Primitive(s) => parsed.interner.resolve(*s).to_string(),
            other => format!("{other:?}"),
        });
    }
    panic!("no `App` component in:\n{src}");
}

const RECORD_AND_FN: &str = "## A Widget has:\n    a count, which is Int.\n\n\
    ## To noWidget() -> Widget:\n    Return a new Widget with count 0.\n\n";

#[test]
fn a_state_field_initialised_by_a_call_is_typed_from_its_return_type() {
    let src = format!(
        "{RECORD_AND_FN}## Component App\nState:\n    Let w be noWidget().\nView:\n    <p>{{w's count}}</p>\n\n## Main\nMount App.\n"
    );
    assert_eq!(state_field_ty(&src).as_deref(), Some("Widget"));
}

#[test]
fn a_function_declared_below_its_caller_still_counts() {
    let src = format!(
        "## Component App\nState:\n    Let w be noWidget().\nView:\n    <p>{{w's count}}</p>\n\n\
         ## Main\nMount App.\n\n{RECORD_AND_FN}"
    );
    assert_eq!(state_field_ty(&src).as_deref(), Some("Widget"));
}

#[test]
fn an_explicit_annotation_is_never_overridden() {
    let src = format!(
        "{RECORD_AND_FN}## Component App\nState:\n    Let w: Widget be noWidget().\nView:\n    <p>{{w's count}}</p>\n\n## Main\nMount App.\n"
    );
    assert_eq!(state_field_ty(&src).as_deref(), Some("Widget"));
}

#[test]
fn a_plain_literal_field_is_unaffected() {
    let src =
        "## Component App\nState:\n    Let w be 0.\nView:\n    <p>{w}</p>\n\n## Main\nMount App.\n";
    assert_eq!(state_field_ty(src), None);
}

#[test]
fn a_call_to_an_undeclared_or_untyped_function_leaves_the_field_untyped() {
    let src = "## To identity (x: Int) -> Int:\n    Return x.\n\n\
               ## Component App\nState:\n    Let w be somethingUndeclared().\nView:\n    <p>{w}</p>\n\n## Main\nMount App.\n";
    assert_eq!(state_field_ty(src), None);
}
