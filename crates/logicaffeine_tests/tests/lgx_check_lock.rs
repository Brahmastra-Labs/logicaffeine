//! ★ LOCK — `lgx::check`: the structural and accessibility checks a template passes
//! BEFORE any LOGOS expression inside it is parsed. ★
//!
//! Pure, over the template tree: a page that `{extends}` a layout may carry only blocks at
//! its top level; a block name is defined once per template; `{children}` appears once;
//! `bind` only fits form controls; `class:` only fits elements (a component takes props).
//! The accessibility lint is the framework refusing to let the common mistakes ship:
//! an image without `alt`, a button with nothing to announce, a form control with no
//! label, a link with nowhere to go. Findings carry a severity, the span, and a message
//! that names the fix.

use logicaffeine_language::lgx::check::{check, FindingKind, Severity};
use logicaffeine_language::lgx::template::parse;

fn kinds(src: &str) -> Vec<(FindingKind, Severity)> {
    let nodes = parse(src).unwrap_or_else(|e| panic!("{src:?} must parse: {e:?}"));
    check(&nodes)
        .into_iter()
        .map(|f| (f.kind, f.severity))
        .collect()
}

fn has(src: &str, kind: FindingKind, severity: Severity) {
    let got = kinds(src);
    assert!(
        got.contains(&(kind.clone(), severity)),
        "{src:?} should report {kind:?} ({severity:?}); got {got:?}"
    );
}

fn clean(src: &str) {
    let got = kinds(src);
    assert!(
        got.is_empty(),
        "{src:?} should have no findings; got {got:?}"
    );
}

#[test]
fn an_extending_view_may_only_hold_blocks_at_its_top_level() {
    has(
        "{extends Site}<p>loose</p>{block content:}<p>x</p>{end}",
        FindingKind::ContentOutsideBlock,
        Severity::Error,
    );
    clean("{extends Site}{block title:}Todos{end}{block content:}<p>x</p>{end}");
}

#[test]
fn block_names_are_unique_and_children_appears_once() {
    has(
        "{extends Site}{block content:}a{end}{block content:}b{end}",
        FindingKind::DuplicateBlock("content".into()),
        Severity::Error,
    );
    has(
        "<div>{children}</div><footer>{children}</footer>",
        FindingKind::DuplicateChildren,
        Severity::Error,
    );
    clean("<main>{block content}</main><aside>{block side}</aside>");
}

#[test]
fn bind_fits_only_form_controls_and_class_toggles_fit_only_elements() {
    has(
        "<div bind={draft}></div>",
        FindingKind::BindOnNonInput("div".into()),
        Severity::Error,
    );
    clean("<label>Task <input bind={draft}></label><label>Notes <textarea bind={notes}></textarea></label><label>Pick <select bind={choice}></select></label>");
    has(
        "<TodoRow class:done={ok}/>",
        FindingKind::ClassToggleOnComponent("TodoRow".into()),
        Severity::Error,
    );
}

#[test]
fn accessibility_lint_catches_the_common_mistakes() {
    has(
        "<img src=\"a.png\">",
        FindingKind::ImageWithoutAlt,
        Severity::Warning,
    );
    clean("<img src=\"a.png\" alt=\"A cat\"><img src=\"b.png\" alt=\"\">");
    has(
        "<button on-click={go}></button>",
        FindingKind::ButtonWithoutName,
        Severity::Warning,
    );
    clean("<button on-click={go}>Go</button><button aria-label=\"Close\" on-click={x}></button><button>{label}</button>");
    has(
        "<input type=\"text\" bind={draft}>",
        FindingKind::InputWithoutLabel,
        Severity::Warning,
    );
    clean("<label for=\"draft\">Task</label><input id=\"draft\" type=\"text\" bind={draft}>");
    clean("<label>Task <input type=\"text\" bind={draft}></label>");
    clean("<input type=\"text\" aria-label=\"Task\" bind={draft}><input type=\"hidden\" name=\"csrf\"><input type=\"submit\" value=\"Go\">");
    has(
        "<a>nowhere</a>",
        FindingKind::LinkWithoutHref,
        Severity::Warning,
    );
    clean("<a href=\"/todos\">Todos</a><a href={url}>dyn</a>");
}

#[test]
fn findings_carry_spans_and_messages_that_name_the_fix() {
    let src = "<img src=\"a.png\">";
    let nodes = parse(src).unwrap();
    let f = &check(&nodes)[0];
    assert_eq!(&src[f.span.start..f.span.end], src);
    assert!(f.message.contains("alt"), "{}", f.message);
}

// ── T3 checks: the ones that needed the component model ──────────────────────────────────

/// Parse `src` and return the parse error's message, or `None` when it parsed.
fn lgx_parse_failure(src: &str) -> Option<String> {
    use logicaffeine_base::{Arena, Interner, Symbol};
    use logicaffeine_language::analysis::DiscoveryPass;
    use logicaffeine_language::arena_ctx::AstContext;
    use logicaffeine_language::ast::{
        Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr,
    };
    use logicaffeine_language::drs::WorldState;
    use logicaffeine_language::{Lexer, Parser};

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

#[test]
fn a_when_sentence_naming_an_element_no_view_renders_is_refused() {
    let msg = lgx_parse_failure(
        "## Component App\nState:\n    Let n be 0.\nActions:\n    When \"#draft\" is clicked:\n        Set n to 1.\nView:\n    <button id=\"go\">x</button>\n",
    )
    .expect("a selector nothing renders is a program error");
    assert!(
        msg.contains("#draft"),
        "the message names the selector: {msg}"
    );
    assert!(
        msg.contains("id `draft`"),
        "and says what is missing: {msg}"
    );
}

#[test]
fn a_when_sentence_may_name_an_element_a_child_component_renders() {
    // The check reads the WHOLE reachable tree, so listening on a child's element is fine.
    let src = "## Component Row\nView:\n    <li class=\"row\">x</li>\n\n## Component App\nState:\n    Let n be 0.\nActions:\n    When \".row\" is clicked:\n        Set n to 1.\nView:\n    <ul><Row/></ul>\n";
    assert_eq!(lgx_parse_failure(src), None, "a child's element counts");
}

#[test]
fn a_when_sentence_may_name_an_element_the_layout_renders() {
    let src = "## Component Site\nView:\n    <header id=\"nav\">n</header>\n    <main>{block content}</main>\n\n## Component App\nState:\n    Let n be 0.\nActions:\n    When \"#nav\" is clicked:\n        Set n to 1.\nView:\n    {extends Site}\n    {block content:}<p>x</p>{end}\n";
    assert_eq!(lgx_parse_failure(src), None, "the layout's element counts");
}

#[test]
fn a_class_toggle_counts_as_a_class_the_view_can_render() {
    let src = "## Component App\nState:\n    Let n be 0.\n    Let done be true.\nActions:\n    When \".done\" is clicked:\n        Set n to 1.\nView:\n    <li class:done={done}>x</li>\n";
    assert_eq!(
        lgx_parse_failure(src),
        None,
        "a conditional class is still a class"
    );
}

#[test]
fn a_computed_id_makes_the_check_stand_down() {
    // A check that cannot see all the markup must not accuse anyone.
    let src = "## Component App\nState:\n    Let n be 0.\n    Let which be \"draft\".\nActions:\n    When \"#draft\" is clicked:\n        Set n to 1.\nView:\n    <input id={which}>\n";
    assert_eq!(
        lgx_parse_failure(src),
        None,
        "a computed id is markup this parse cannot see"
    );
}

#[test]
fn a_raw_block_makes_the_check_stand_down() {
    let src = "## Component App\nState:\n    Let n be 0.\n    Let markup be \"<p id='draft'>x</p>\".\nActions:\n    When \"#draft\" is clicked:\n        Set n to 1.\nView:\n    <div>{raw markup}</div>\n";
    assert_eq!(
        lgx_parse_failure(src),
        None,
        "trusted markup could hold anything"
    );
}

#[test]
fn a_children_slot_makes_the_check_stand_down() {
    let src = "## Component App\nState:\n    Let n be 0.\nActions:\n    When \"#draft\" is clicked:\n        Set n to 1.\nView:\n    <div>{children}</div>\n";
    assert_eq!(
        lgx_parse_failure(src),
        None,
        "the caller supplies markup this parse cannot see"
    );
}

#[test]
fn a_missing_tag_is_named_as_a_tag() {
    let msg = lgx_parse_failure(
        "## Component App\nState:\n    Let n be 0.\nActions:\n    When \"textarea\" is clicked:\n        Set n to 1.\nView:\n    <button>x</button>\n",
    )
    .expect("a tag nothing renders is a program error");
    assert!(
        msg.contains("`<textarea>`"),
        "the message names the tag: {msg}"
    );
}

#[test]
fn a_descendant_selector_is_checked_compound_by_compound() {
    let msg = lgx_parse_failure(
        "## Component App\nState:\n    Let n be 0.\nActions:\n    When \".card .missing\" is clicked:\n        Set n to 1.\nView:\n    <div class=\"card\"><p class=\"there\">x</p></div>\n",
    )
    .expect("half a selector matching is not enough");
    assert!(
        msg.contains("class `missing`"),
        "the message names the part that failed: {msg}"
    );
}
