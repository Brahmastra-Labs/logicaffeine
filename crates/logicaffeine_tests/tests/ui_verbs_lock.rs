//! ★ LOCK — the English things an action can DO: element verbs, navigation, persistence,
//! feedback, timers, store calls, and typed fetch. ★
//!
//! These are the vocabulary a component's `Actions:` section is written in. Each parses to a
//! first-class statement (`Stmt::Ui` / `Stmt::Fetch`) that every tier implements natively —
//! no desugaring to builtin calls. They are COMPONENT vocabulary: outside a component,
//! `Focus "#draft".` is an ordinary program's business and stays whatever it was, so the
//! 20k-test suite cannot shift underneath them.

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::{DiscoveryPass, TypeRegistry};
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::stmt::{FetchMethod, UiStmt};
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::{Lexer, Parser};

/// Parse a component whose `add` action holds `body`, and render each of its statements.
fn action_shapes(body: &str) -> Vec<String> {
    action_shapes_with("", body)
}

/// As [`action_shapes`], with `decls` appended after the component — for a body that names a
/// store, which must exist by the end of the file (`Parser::resolve_store_calls`).
fn action_shapes_with(decls: &str, body: &str) -> Vec<String> {
    let src = format!("## Component C\nActions:\n    To add:\n{body}{decls}");
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let ws: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let ea: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let ta: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let na: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sa: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let ra: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pa: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));
    let sta: &'static Arena<Stmt> = Box::leak(Box::new(Arena::new()));
    let ia: &'static Arena<Expr> = Box::leak(Box::new(Arena::new()));
    let tya: &'static Arena<TypeExpr> = Box::leak(Box::new(Arena::new()));
    let ctx = AstContext::with_types(ea, ta, na, sa, ra, pa, sta, ia, tya);
    let mut lexer = Lexer::new(&src, interner);
    let tokens = lexer.tokenize();
    let reg = {
        let mut d = DiscoveryPass::new(&tokens, interner);
        d.run_full().types
    };
    let mut parser = Parser::new(tokens, ws, interner, ctx, reg);
    let stmts = parser
        .parse_program()
        .unwrap_or_else(|e| panic!("must parse: {e:?}\n{src}"));
    drop(parser);
    let def = stmts
        .iter()
        .find_map(|s| match s {
            Stmt::ComponentDef(d) if interner.resolve(d.name) == "C" => Some(d),
            _ => None,
        })
        .expect("the component `C`");
    let action = def.actions.first().expect("the `add` action");
    // `Stmt::Site` is the parser's failure-site marker before each authored sentence — never a
    // sentence itself — so the body is read as its author wrote it (`ComponentDef::authored_len`'s
    // own rule).
    action
        .body
        .iter()
        .filter(|s| !matches!(s, Stmt::Site(_)))
        .map(|s| shape(s, interner))
        .collect()
}

fn e(x: &Expr<'_>, i: &Interner) -> String {
    logicaffeine_language::ast::view::expr_shape(x, i)
}

fn shape(s: &Stmt<'_>, i: &Interner) -> String {
    match s {
        Stmt::Ui(u) => match u {
            UiStmt::Focus { selector } => format!("focus {}", e(selector, i)),
            UiStmt::Submit { selector } => format!("submit {}", e(selector, i)),
            UiStmt::Choose { value, selector } => {
                format!("choose {} in {}", e(value, i), e(selector, i))
            }
            UiStmt::ScrollTo { selector } => format!("scroll to {}", e(selector, i)),
            UiStmt::AddClass { class, selector } => {
                format!("add class {} to {}", e(class, i), e(selector, i))
            }
            UiStmt::RemoveClass { class, selector } => {
                format!("remove class {} from {}", e(class, i), e(selector, i))
            }
            UiStmt::SelectText { selector } => format!("select text in {}", e(selector, i)),
            UiStmt::CopyToClipboard { text } => format!("copy {}", e(text, i)),
            UiStmt::Download { bytes, name } => {
                format!("download {} as {}", e(bytes, i), e(name, i))
            }
            UiStmt::Navigate { url } => format!("navigate {}", e(url, i)),
            UiStmt::ReplaceUrl { url } => format!("replace url {}", e(url, i)),
            UiStmt::GoBack => "go back".into(),
            UiStmt::SetTitle { title } => format!("title {}", e(title, i)),
            UiStmt::Save { value, key } => format!("save {} as {}", e(value, i), e(key, i)),
            UiStmt::Load { key, var } => format!("load {} into {}", e(key, i), i.resolve(*var)),
            UiStmt::Forget { key } => format!("forget {}", e(key, i)),
            UiStmt::Reload => "reload".into(),
            UiStmt::Notify {
                message,
                tone: None,
            } => format!("notify {}", e(message, i)),
            UiStmt::Notify {
                message,
                tone: Some(t),
            } => format!("notify {} as {}", e(message, i), i.resolve(*t)),
            UiStmt::Toggle { field } => format!("toggle {}", i.resolve(*field)),
            UiStmt::CallStore {
                store,
                action,
                args,
            } => {
                let a: Vec<String> = args.iter().map(|x| e(x, i)).collect();
                format!(
                    "call {}'s {}({})",
                    i.resolve(*store),
                    i.resolve(*action),
                    a.join(", ")
                )
            }
            UiStmt::StopTimer { name } => format!("stop timer {}", i.resolve(*name)),
            UiStmt::StartTimer { name } => format!("start timer {}", i.resolve(*name)),
            UiStmt::Click { selector } => format!("click {}", e(selector, i)),
            // The two verbs that let a test CAUSE the events `blurred` and `hovered` — both could
            // be waited for and neither could be made to happen.
            UiStmt::Blur { selector } => format!("blur {}", e(selector, i)),
            UiStmt::Hover { selector } => format!("hover {}", e(selector, i)),
            // A press with no element carries the PAGE as its selector, filled in by the parser
            // so every tier receives one shape rather than each inventing the default.
            UiStmt::Press {
                key,
                selector: Some(sel),
            } => {
                format!("press {} in {}", e(key, i), e(sel, i))
            }
            UiStmt::Press {
                key,
                selector: None,
            } => format!("press {}", e(key, i)),
            UiStmt::TypeInto { text, selector } => {
                format!("type {} into {}", e(text, i), e(selector, i))
            }
            UiStmt::Mount {
                component,
                selector,
            } => match selector {
                None => format!("mount {}", i.resolve(*component)),
                Some(sel) => format!("mount {} at {}", i.resolve(*component), e(sel, i)),
            },
            // ★ THE VALUE IS PRINTED, NOT ELIDED. A `..` here would compile and quietly stop this
            // lock covering the field — and the field is the whole point of the third form: a
            // dropped file's name arrives WITH the event, so a trigger that binds one cannot be
            // caused by a sentence with no way to say it.
            UiStmt::RaiseWindowEvent {
                event,
                target,
                value,
            } => {
                format!(
                    "raise {} on {} with {}",
                    e(event, i),
                    e(target, i),
                    e(value, i)
                )
            }
            // Both printed for the same reason: a publish that dropped either half would be a
            // message with no body or a body sent nowhere.
            UiStmt::Publish { message, topic } => {
                format!("publish {} on {}", e(message, i), e(topic, i))
            }
        },
        Stmt::Fetch {
            method,
            url,
            body,
            as_type,
            then,
        } => {
            let m = match method {
                FetchMethod::Get => "get",
                FetchMethod::Post => "post",
                FetchMethod::Put => "put",
                FetchMethod::Patch => "patch",
                FetchMethod::Delete => "delete",
                FetchMethod::Upload => "upload",
            };
            let b = body.map(|x| format!(" {}", e(x, i))).unwrap_or_default();
            format!(
                "{m}{b} {} as {} then {}",
                e(url, i),
                type_name(as_type, i),
                i.resolve(*then)
            )
        }
        other => format!("{:?}", std::mem::discriminant(other)),
    }
}

fn type_name(t: &TypeExpr<'_>, i: &Interner) -> String {
    match t {
        TypeExpr::Primitive(s) | TypeExpr::Named(s) => i.resolve(*s).to_string(),
        TypeExpr::Generic { base, params } => {
            let ps: Vec<String> = params.iter().map(|p| type_name(p, i)).collect();
            format!("{} of {}", i.resolve(*base), ps.join(" and "))
        }
        other => format!("{other:?}"),
    }
}

#[test]
fn element_verbs_parse() {
    assert_eq!(
        action_shapes(
            "        Focus \"#draft\".\n\
             \x20       Scroll to \"#bottom\".\n\
             \x20       Add class \"shake\" to \"#card\".\n\
             \x20       Remove class \"shake\" from \"#card\".\n\
             \x20       Select the text in \"#draft\".\n\
             \x20       Copy \"hello\" to the clipboard.\n"
        ),
        [
            "focus \"#draft\"",
            "scroll to \"#bottom\"",
            "add class \"shake\" to \"#card\"",
            "remove class \"shake\" from \"#card\"",
            "select text in \"#draft\"",
            "copy \"hello\"",
        ]
    );
}

/// ★ `Raise` — the driver for events that happen to the PAGE rather than to an element.
///
/// Three forms, and each column is load-bearing. The PAGE is the default target, spelled `"body"`
/// by the parser so the driver and the registration name one node rather than each defaulting on
/// its own. The VALUE is what the event carried — a dropped file's name arrives with the event, so
/// a trigger that binds one cannot be caused by a sentence with no way to say it — and it is the
/// empty text when the event carried nothing, which is most of them.
#[test]
fn the_raise_driver_parses_its_three_forms() {
    assert_eq!(
        action_shapes(
            "        Raise \"offline\".\n\
             \x20       Raise \"scrolledtobottom\" on \"#list\".\n\
             \x20       Raise \"filedropped\" on \"#zone\" with \"photo.png\".\n"
        ),
        [
            "raise \"offline\" on \"body\" with \"\"",
            "raise \"scrolledtobottom\" on \"#list\" with \"\"",
            "raise \"filedropped\" on \"#zone\" with \"photo.png\"",
        ]
    );
}

#[test]
fn navigation_and_title_verbs_parse() {
    assert_eq!(
        action_shapes(
            "        Navigate to \"/todos\".\n\
             \x20       Replace the URL with \"/todos/1\".\n\
             \x20       Go back.\n\
             \x20       Set the title to \"Todos\".\n"
        ),
        [
            "navigate \"/todos\"",
            "replace url \"/todos/1\"",
            "go back",
            "title \"Todos\""
        ]
    );
}

#[test]
fn persistence_feedback_state_and_timer_verbs_parse() {
    // `Stop the timer refresh` names a timer, and a timer is named where it is DECLARED — so the
    // component has to declare one (`Parser::check_timer_stops`).
    assert_eq!(
        action_shapes_with(
            "    Every 60 seconds as refresh:\n        Show 1.\n",
            "        Save draft as \"draft-cache\".\n\
             \x20       Load \"draft-cache\" into draft.\n\
             \x20       Forget \"draft-cache\".\n\
             \x20       Notify \"Saved.\".\n\
             \x20       Notify \"Gone.\" as error.\n\
             \x20       Toggle done.\n\
             \x20       Reload the page.\n\
             \x20       Stop the timer refresh.\n\
             \x20       Start the timer refresh.\n",
        ),
        [
            "save draft as \"draft-cache\"",
            "load \"draft-cache\" into draft",
            "forget \"draft-cache\"",
            "notify \"Saved.\"",
            "notify \"Gone.\" as error",
            "toggle done",
            "reload",
            "stop timer refresh",
            "start timer refresh",
        ]
    );
}

#[test]
fn the_page_driving_verbs_parse() {
    assert_eq!(
        action_shapes(
            "        Click \"#go\".\n\
             \x20       Type \"milk\" into \"#draft\".\n\
             \x20       Press \"Enter\" in \"#draft\".\n\
             \x20       Press \"Escape\".\n"
        ),
        [
            "click \"#go\"",
            "type \"milk\" into \"#draft\"",
            "press \"Enter\" in \"#draft\"",
            "press \"Escape\" in \"body\"",
        ]
    );
}

#[test]
fn a_store_action_call_parses() {
    assert_eq!(
        action_shapes_with(
            "\n## Store Session\nState:\n    Let name be \"guest\".\nActions:\n    To login (who: Text):\n        Set name to who.\n    To logout:\n        Set name to \"guest\".\n",
            "        Call Session's login with \"ada\".\n        Call Session's logout.\n",
        ),
        ["call Session's login(\"ada\")", "call Session's logout()"]
    );
}

/// Parse `src` and return the parse error's message, or `None` when it parsed.
fn parse_failure(src: &str) -> Option<String> {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let ws: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let ea: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let ta: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let na: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sa: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let ra: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pa: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));
    let sta: &'static Arena<Stmt> = Box::leak(Box::new(Arena::new()));
    let ia: &'static Arena<Expr> = Box::leak(Box::new(Arena::new()));
    let tya: &'static Arena<TypeExpr> = Box::leak(Box::new(Arena::new()));
    let ctx = AstContext::with_types(ea, ta, na, sa, ra, pa, sta, ia, tya);
    let mut lexer = Lexer::new(src, interner);
    let tokens = lexer.tokenize();
    let reg = {
        let mut d = DiscoveryPass::new(&tokens, interner);
        d.run_full().types
    };
    let mut parser = Parser::new(tokens, ws, interner, ctx, reg);
    parser.parse_program().err().map(|e| format!("{e:?}"))
}

const STORE: &str = "\n## Store Session\nState:\n    Let name be \"guest\".\nActions:\n    To login (who: Text):\n        Set name to who.\n";

#[test]
fn a_store_may_be_declared_after_the_component_that_calls_it() {
    let src = format!(
        "## Component C\nActions:\n    To add:\n        Call Session's login with \"ada\".\n{STORE}"
    );
    assert_eq!(
        parse_failure(&src),
        None,
        "the check runs once the whole file is parsed"
    );
}

#[test]
fn a_store_call_nested_in_a_branch_is_still_checked() {
    // Recorded where it is PARSED, so no amount of nesting can hide one from the check.
    let src = format!(
        "## Component C\nActions:\n    To add:\n        If 1 is equal to 1:\n            Call Session's vanish.\n{STORE}"
    );
    let msg = parse_failure(&src).expect("a nested bad call is still a program error");
    assert!(
        msg.contains("vanish") && msg.contains("Session"),
        "names both: {msg}"
    );
    assert!(
        msg.contains("login"),
        "and says which actions the store does have: {msg}"
    );
}

#[test]
fn calling_a_store_that_does_not_exist_names_the_ones_that_do() {
    let src = format!(
        "## Component C\nActions:\n    To add:\n        Call Ghost's login with \"ada\".\n{STORE}"
    );
    let msg = parse_failure(&src).expect("an undeclared store is a program error");
    assert!(
        msg.contains("Ghost") && msg.contains("not a store"),
        "names it: {msg}"
    );
    assert!(
        msg.contains("Session"),
        "and points at the store that exists: {msg}"
    );
}

#[test]
fn typed_fetch_parses_for_every_method() {
    assert_eq!(
        action_shapes(
            "        Get \"/api/todos\" as Seq of Text, then loaded.\n\
             \x20       Post draft to \"/api/todos\" as Seq of Text, then loaded.\n\
             \x20       Put draft to \"/api/todos/1\" as Text, then loaded.\n\
             \x20       Patch draft to \"/api/todos/1\" as Text, then loaded.\n\
             \x20       Delete \"/api/todos/1\" as Text, then loaded.\n"
        ),
        [
            "get \"/api/todos\" as Seq of Text then loaded",
            "post draft \"/api/todos\" as Seq of Text then loaded",
            "put draft \"/api/todos/1\" as Text then loaded",
            "patch draft \"/api/todos/1\" as Text then loaded",
            "delete \"/api/todos/1\" as Text then loaded",
        ]
    );
}

#[test]
fn an_interpolated_url_keeps_its_holes() {
    assert_eq!(
        action_shapes("        Delete \"/api/todos/{id}\" as Text, then loaded.\n"),
        ["delete \"/api/todos/{id}\" as Text then loaded"]
    );
}

#[test]
fn the_verbs_are_component_vocabulary_not_global() {
    // Outside a component these words keep whatever meaning an ordinary program gave them —
    // the LGX vocabulary must not reinterpret existing programs.
    let src = "## Main\nLet x be 1.\nShow x.\n";
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let ws: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let ea: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let ta: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let na: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sa: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let ra: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pa: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));
    let sta: &'static Arena<Stmt> = Box::leak(Box::new(Arena::new()));
    let ia: &'static Arena<Expr> = Box::leak(Box::new(Arena::new()));
    let tya: &'static Arena<TypeExpr> = Box::leak(Box::new(Arena::new()));
    let ctx = AstContext::with_types(ea, ta, na, sa, ra, pa, sta, ia, tya);
    let mut lexer = Lexer::new(src, interner);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens, ws, interner, ctx, TypeRegistry::default());
    let stmts = parser
        .parse_program()
        .expect("an ordinary program still parses");
    assert!(!stmts
        .iter()
        .any(|s| matches!(s, Stmt::Ui(_) | Stmt::Fetch { .. })));
}

#[test]
fn mounting_an_lgx_component_is_a_first_class_statement() {
    // `Mount App.` on an LGX component is a page statement — not a call to the legacy dialect's
    // synthesized `render` function.
    let src = "## Component App\nState:\n    Let n be 0.\nView:\n    <p>{n}</p>\n\n## Main\nMount App.\nMount App at \"#side\".\n";
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let ws: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let ea: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let ta: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let na: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sa: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let ra: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pa: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));
    let sta: &'static Arena<Stmt> = Box::leak(Box::new(Arena::new()));
    let ia: &'static Arena<Expr> = Box::leak(Box::new(Arena::new()));
    let tya: &'static Arena<TypeExpr> = Box::leak(Box::new(Arena::new()));
    let ctx = AstContext::with_types(ea, ta, na, sa, ra, pa, sta, ia, tya);
    let mut lexer = Lexer::new(src, interner);
    let tokens = lexer.tokenize();
    let reg = {
        let mut d = DiscoveryPass::new(&tokens, interner);
        d.run_full().types
    };
    let mut parser = Parser::new(tokens, ws, interner, ctx, reg);
    let stmts = parser
        .parse_program()
        .unwrap_or_else(|e| panic!("must parse: {e:?}"));
    drop(parser);
    let mounts: Vec<String> = stmts
        .iter()
        .filter(|s| matches!(s, Stmt::Ui(_)))
        .map(|s| shape(s, interner))
        .collect();
    assert_eq!(mounts, ["mount App", "mount App at \"#side\""]);
}
