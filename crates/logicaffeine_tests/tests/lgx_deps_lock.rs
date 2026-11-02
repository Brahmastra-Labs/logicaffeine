//! ★ LOCK — the static dependency graph a fine-grained update is compiled from. ★
//!
//! Stage A repaints a component by CLEARING its mount and rebuilding the subtree. That is correct
//! and it is why every parity lock passes, but it is O(the whole view) for a change to one field:
//! a keystroke into a draft box rebuilds a thousand-row list, and every node the visitor was
//! interacting with is destroyed and replaced.
//!
//! Stage B compiles the update instead. To do that the compiler has to know, statically:
//!
//!   * for every SITE in the view that can change (a text hole, an attribute value, a class
//!     toggle, a condition, a loop's collection, a binding), which STATE FIELDS it reads;
//!   * for every ACTION, which state fields it writes;
//!   * for every DERIVED field, what it is derived FROM.
//!
//! Then a turn that writes `draft` updates exactly the sites that read `draft`.
//!
//! ★ THE READ SET IS CLOSED, and that is the whole difficulty. A hole naming a DERIVED field
//! depends on whatever the derived field reads. A hole inside `{for each t in todos:}` naming `t`
//! depends on `todos` — `t` is not state, it is a window onto state. A hole naming a `{let}`
//! binding depends on whatever that binding's expression reads. A site whose reads were recorded
//! as the names LITERALLY WRITTEN would miss every one of those, and the failure is silent: the
//! page simply does not update, on exactly the constructs a real page is built from.
//!
//! ★ AND A CYCLE AMONG DERIVED FIELDS IS REJECTED. `a always be b + 1` beside `b always be a + 1`
//! has no fixed point; the plan's turn semantics say a second round of change handlers is a
//! compile-time-rejected cycle, and this is where that is decided.

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::TypeRegistry;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::lgx::deps::component_deps;
use logicaffeine_language::{Lexer, Parser};

/// Parse `input`, find its first `## Component`/`## Page`, and describe its dependency graph.
///
/// The description is a deterministic line-per-fact rendering — sites in document order, then the
/// derived graph, then the write sets — so a test reads as the thing it is asserting rather than
/// as a walk over a data structure.
fn deps_of(input: &str) -> Result<Vec<String>, String> {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let expr_arena: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let term_arena: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let np_arena: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sym_arena: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let role_arena: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pp_arena: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));
    let stmt_arena: &'static Arena<Stmt> = Box::leak(Box::new(Arena::new()));
    let iexpr_arena: &'static Arena<Expr> = Box::leak(Box::new(Arena::new()));
    let type_arena: &'static Arena<TypeExpr> = Box::leak(Box::new(Arena::new()));
    let ctx = AstContext::with_types(
        expr_arena,
        term_arena,
        np_arena,
        sym_arena,
        role_arena,
        pp_arena,
        stmt_arena,
        iexpr_arena,
        type_arena,
    );
    let mut lexer = Lexer::new(input, interner);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, TypeRegistry::default());
    // ★ A REFUSAL IS AN ANSWER HERE, NOT A CRASH. A derived cycle is now reported where the author
    // wrote it — `lower_lgx_components` refuses the program by name — so it never reaches
    // `component_deps` from a real parse. It used to be computed here, discarded at both call
    // sites, and surface to the author as `Undefined variable: b` about a field they had declared
    // three lines up. Returning the refusal keeps the cycle tests below asserting exactly what
    // they always asserted: that it is refused, and that the message names the fields.
    let stmts = match parser.parse_program() {
        Ok(s) => s,
        Err(e) => return Err(format!("{e:?}")),
    };
    let def = stmts
        .iter()
        .find_map(|s| match s {
            Stmt::ComponentDef(def) => Some(def),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no component in:\n{input}"));
    component_deps(def, interner)
        .map(|d| d.describe(interner))
        .map_err(|e| format!("{e:?}"))
}

/// The lines, or a panic naming the rejection — for the tests that expect a graph.
fn lines(input: &str) -> Vec<String> {
    deps_of(input).unwrap_or_else(|e| panic!("deps should build: {e}\n{input}"))
}

/// Assert `want` is present, with the whole graph in the failure message.
fn has(input: &str, want: &str) {
    let got = lines(input);
    assert!(
        got.iter().any(|l| l == want),
        "expected fact `{want}`\ngot:\n  {}\nfor:\n{input}",
        got.join("\n  ")
    );
}

fn lacks(input: &str, unwanted: &str) {
    let got = lines(input);
    assert!(
        !got.iter().any(|l| l == unwanted),
        "fact `{unwanted}` must NOT be present\ngot:\n  {}\nfor:\n{input}",
        got.join("\n  ")
    );
}

// ---------------------------------------------------------------------------------------------
// Reads — the sites, and what each one actually depends on.
// ---------------------------------------------------------------------------------------------

const HEAD: &str = "## Component C\n\nState:\n";

#[test]
fn a_text_hole_reads_the_field_it_names() {
    has(
        &format!("{HEAD}    Let draft be \"\".\n\nView:\n    <p>{{draft}}</p>\n"),
        "text@0.0 reads draft",
    );
}

#[test]
fn an_interpolated_hole_reads_every_field_in_it() {
    // A hole is a whole expression, not a name — `{a + b}` must update when EITHER moves, and a
    // read set that recorded only the first would leave a page half-stale.
    has(
        &format!("{HEAD}    Let a be 1.\n    Let b be 2.\n\nView:\n    <p>{{a + b}}</p>\n"),
        "text@0.0 reads a, b",
    );
}

#[test]
fn an_attribute_value_is_its_own_site() {
    // An attribute updates independently of the element's text: rewriting the whole element to
    // change a title is the O(subtree) repaint this work exists to remove.
    has(
        &format!("{HEAD}    Let name be \"x\".\n\nView:\n    <p title={{name}}>hi</p>\n"),
        "attr:title@0 reads name",
    );
}

#[test]
fn a_literal_attribute_is_not_a_site_at_all() {
    // A constant never changes, so it must not appear in any field's update list — otherwise
    // every turn touches it and the op-count ratchet can never be met.
    let got = lines(&format!(
        "{HEAD}    Let n be 1.\n\nView:\n    <p class=\"card\">{{n}}</p>\n"
    ));
    assert!(
        !got.iter().any(|l| l.starts_with("attr:class@")),
        "a literal attribute is not an update site:\n  {}",
        got.join("\n  ")
    );
}

#[test]
fn a_class_toggle_reads_its_condition() {
    has(
        &format!("{HEAD}    Let done be false.\n\nView:\n    <p class:done={{done}}>hi</p>\n"),
        "class:done@0 reads done",
    );
}

#[test]
fn a_condition_is_a_site_and_reads_what_it_tests() {
    has(
        &format!(
            "{HEAD}    Let error be \"\".\n\nView:\n    {{if error is not \"\":}}<p>bad</p>{{end}}\n"
        ),
        "if@0 reads error",
    );
}

#[test]
fn a_loop_reads_its_collection() {
    has(
        &format!(
            "{HEAD}    Let todos be [\"a\"].\n\nView:\n    \
             <ul>{{for each t in todos:}}<li>{{t}}</li>{{end}}</ul>\n"
        ),
        "each@0.0 reads todos",
    );
}

#[test]
fn a_hole_naming_a_loop_variable_reads_the_collection() {
    // ★ `t` is not state — it is a WINDOW onto state. A read set recording the name literally
    // written would say this site depends on `t`, which no action ever writes, so the row would
    // never update when `todos` moved. Silent, and on the single most common construct there is.
    has(
        &format!(
            "{HEAD}    Let todos be [\"a\"].\n\nView:\n    \
             <ul>{{for each t in todos:}}<li>{{t}}</li>{{end}}</ul>\n"
        ),
        "text@0.0.0.0 reads todos",
    );
}

#[test]
fn a_hole_naming_a_template_binding_reads_through_it() {
    // `{let}` is the same shape as a loop variable: a name that is not state, standing for an
    // expression that is.
    has(
        &format!(
            "{HEAD}    Let n be 2.\n\nView:\n    {{let doubled be n * 2}}<p>{{doubled}}</p>\n"
        ),
        "text@1.0 reads n",
    );
}

#[test]
fn a_hole_naming_a_derived_field_reads_what_the_derivation_reads() {
    // ★ THE CLOSURE. `remaining` is recomputed from `todos`, so a site showing `remaining` has to
    // update when `todos` does — nothing ever writes `remaining` directly.
    has(
        &format!(
            "{HEAD}    Let todos be [\"a\"].\n\
            \x20   Let remaining always be length of todos.\n\n\
             View:\n    <p>{{remaining}}</p>\n"
        ),
        "text@0.0 reads todos",
    );
}

#[test]
fn a_derivation_of_a_derivation_closes_all_the_way_down() {
    // One level of closure is the easy half; a chain is where a fixpoint is actually needed.
    has(
        &format!(
            "{HEAD}    Let n be 1.\n\
            \x20   Let doubled always be n * 2.\n\
            \x20   Let quadrupled always be doubled * 2.\n\n\
             View:\n    <p>{{quadrupled}}</p>\n"
        ),
        "text@0.0 reads n",
    );
}

#[test]
fn a_prop_is_not_a_state_read() {
    // A prop cannot change without the PARENT re-rendering this component, so it is not something
    // this instance's own turns can invalidate. Recording it as state would make every parent
    // update look like a local one.
    let src = "## Component Row (label: Text)\n\nView:\n    <li>{label}</li>\n";
    lacks(src, "text@0.0 reads label");
    has(src, "text@0.0 reads props label");
}

#[test]
fn a_bind_is_a_site_that_reads_its_field() {
    // A two-way binding shows the field and writes it back, so it is an update site like any
    // other: typing elsewhere into the same field must move the input.
    has(
        &format!("{HEAD}    Let draft be \"\".\n\nView:\n    <input bind={{draft}}>\n"),
        "bind:draft@0 reads draft",
    );
}

#[test]
fn a_site_that_reads_nothing_says_so() {
    // A static hole (`{1 + 1}`) is a real thing to write and must be excluded from every update
    // list rather than defaulting to "depends on everything".
    has(
        &format!("{HEAD}    Let n be 1.\n\nView:\n    <p>{{1 + 1}}</p>\n"),
        "text@0.0 reads —",
    );
}

#[test]
fn nested_elements_address_sites_by_path() {
    // The patcher has to ADDRESS the node it updates, and the address has to be stable across
    // compilations of the same view or the emitted patcher points at the wrong node.
    let src = format!(
        "{HEAD}    Let a be 1.\n    Let b be 2.\n\nView:\n    \
         <div><span>{{a}}</span><span><em>{{b}}</em></span></div>\n"
    );
    has(&src, "text@0.0.0 reads a");
    has(&src, "text@0.1.0.0 reads b");
}

// ---------------------------------------------------------------------------------------------
// Writes — what a turn can invalidate.
// ---------------------------------------------------------------------------------------------

#[test]
fn an_action_writes_the_field_it_sets() {
    has(
        &format!(
            "{HEAD}    Let draft be \"\".\n\nActions:\n    To clear:\n        Set draft to \"\".\n\n\
             View:\n    <p>{{draft}}</p>\n"
        ),
        "writes clear -> draft",
    );
}

#[test]
fn every_mutating_verb_counts_as_a_write() {
    // `Set` is not the only door. A write the compiler cannot see is a site that never updates —
    // the failure is invisible because the state really did change.
    let src = format!(
        "{HEAD}    Let n be 0.\n    Let xs be [1].\n    Let flag be false.\n\n\
         Actions:\n    To go:\n        Set n to n + 1.\n        Push 1 to xs.\n        Toggle flag.\n\n\
         View:\n    <p>{{n}}</p>\n"
    );
    has(&src, "writes go -> flag, n, xs");
}

#[test]
fn a_write_inside_control_flow_still_counts() {
    // A conditional write is still a write: the read set is a STATIC over-approximation, and
    // missing a branch is the one direction that produces a stale page.
    has(
        &format!(
            "{HEAD}    Let n be 0.\n\nActions:\n    To go:\n        If n is 0:\n            Set n to 1.\n\n\
             View:\n    <p>{{n}}</p>\n"
        ),
        "writes go -> n",
    );
}

#[test]
fn an_action_that_writes_nothing_says_so() {
    has(
        &format!(
            "{HEAD}    Let n be 0.\n\nActions:\n    To go:\n        Show \"hi\".\n\n\
             View:\n    <p>{{n}}</p>\n"
        ),
        "writes go -> —",
    );
}

#[test]
fn a_local_of_the_same_name_is_not_a_state_write() {
    // ★ An action's own `Let` shadows the field, and a write to the local must not be reported as
    // invalidating the state field — that would repaint sites that cannot have changed, quietly
    // undoing the whole point of the analysis.
    has(
        &format!(
            "{HEAD}    Let n be 0.\n\nActions:\n    To go:\n        Let mutable n be 5.\n        Set n to 6.\n\n\
             View:\n    <p>{{n}}</p>\n"
        ),
        "writes go -> —",
    );
}

#[test]
fn an_event_handler_carries_its_own_write_set() {
    // `When "#b" is clicked:` is a turn like an action, and its writes have to be known for the
    // same reason.
    has(
        &format!(
            "{HEAD}    Let n be 0.\n\nActions:\n    When \"#b\" is clicked:\n        Set n to n + 1.\n\n\
             View:\n    <button id=\"b\">{{n}}</button>\n"
        ),
        "writes when:#b:click -> n",
    );
}

// ---------------------------------------------------------------------------------------------
// The derived graph, and the cycle it must refuse.
// ---------------------------------------------------------------------------------------------

#[test]
fn the_derived_graph_is_reported() {
    has(
        &format!(
            "{HEAD}    Let todos be [\"a\"].\n\
            \x20   Let remaining always be length of todos.\n\n\
             View:\n    <p>{{remaining}}</p>\n"
        ),
        "derived remaining <- todos",
    );
}

#[test]
fn a_derived_cycle_is_refused_by_name() {
    // ★ `a always be b + 1` and `b always be a + 1` have no fixed point. Recomputing them in any
    // order gives an answer that depends on the order, and the plan's turn semantics forbid a
    // second round — so this is decided here, at compile time, rather than by whichever value
    // happened to settle.
    let err = deps_of(&format!(
        "{HEAD}    Let a always be b + 1.\n    Let b always be a + 1.\n\nView:\n    <p>{{a}}</p>\n"
    ))
    .expect_err("a derived cycle must be refused");
    let msg = err;
    assert!(
        msg.contains('a') && msg.contains('b'),
        "the refusal must name the fields: {msg}"
    );
}

#[test]
fn a_field_derived_from_itself_is_a_cycle_too() {
    // The one-node case is the easiest to miss in a cycle check that only looks for pairs.
    deps_of(&format!(
        "{HEAD}    Let a always be a + 1.\n\nView:\n    <p>{{a}}</p>\n"
    ))
    .expect_err("a self-referential derivation must be refused");
}

#[test]
fn a_long_derived_chain_is_not_a_cycle() {
    // The refusal has to stay NARROW: a chain is the ordinary, useful shape, and a checker that
    // called it a cycle would refuse most real pages.
    lines(&format!(
        "{HEAD}    Let n be 1.\n    Let b always be n + 1.\n    Let c always be b + 1.\n\
        \x20   Let d always be c + 1.\n\nView:\n    <p>{{d}}</p>\n"
    ));
}

// ---------------------------------------------------------------------------------------------
// The whole point: what does one write actually invalidate?
// ---------------------------------------------------------------------------------------------

#[test]
fn a_write_invalidates_exactly_the_sites_that_read_it() {
    // ★ THE QUESTION THE PATCHER ASKS. Everything above is the input to this one.
    let src = format!(
        "{HEAD}    Let draft be \"\".\n    Let todos be [\"a\"].\n\n\
         Actions:\n    To clear:\n        Set draft to \"\".\n\n\
         View:\n    <div><input bind={{draft}}><p>{{draft}}</p>\
         <ul>{{for each t in todos:}}<li>{{t}}</li>{{end}}</ul></div>\n"
    );
    let got = lines(&src);
    let touched: Vec<&String> = got
        .iter()
        .filter(|l| l.contains("reads draft") && !l.contains("props"))
        .collect();
    assert_eq!(
        touched.len(),
        2,
        "writing `draft` must touch exactly the binding and the paragraph — not the list:\n  {}",
        got.join("\n  ")
    );
}

#[test]
fn the_description_is_deterministic() {
    // The graph is built from maps and sets; emitting it in iteration order would make every
    // downstream lock a coin flip and every generated patcher differ between builds.
    let src = format!(
        "{HEAD}    Let a be 1.\n    Let b be 2.\n    Let c be 3.\n\n\
         Actions:\n    To go:\n        Set a to 2.\n        Set b to 3.\n        Set c to 4.\n\n\
         View:\n    <p>{{a}}</p><p>{{b}}</p><p>{{c}}</p>\n"
    );
    let first = lines(&src);
    for _ in 0..8 {
        assert_eq!(
            lines(&src),
            first,
            "the dependency description must not vary between runs"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Which sites a turn can update IN PLACE — and which force a rebuild.
// ---------------------------------------------------------------------------------------------

/// The `(key, patchable)` pairs, in document order.
fn patchable(input: &str) -> Vec<(String, bool)> {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let expr_arena: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let term_arena: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let np_arena: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sym_arena: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let role_arena: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pp_arena: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));
    let stmt_arena: &'static Arena<Stmt> = Box::leak(Box::new(Arena::new()));
    let iexpr_arena: &'static Arena<Expr> = Box::leak(Box::new(Arena::new()));
    let type_arena: &'static Arena<TypeExpr> = Box::leak(Box::new(Arena::new()));
    let ctx = AstContext::with_types(
        expr_arena,
        term_arena,
        np_arena,
        sym_arena,
        role_arena,
        pp_arena,
        stmt_arena,
        iexpr_arena,
        type_arena,
    );
    let mut lexer = Lexer::new(input, interner);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, TypeRegistry::default());
    let stmts = parser
        .parse_program()
        .unwrap_or_else(|e| panic!("{e:?}\n{input}"));
    let def = stmts
        .iter()
        .find_map(|s| match s {
            Stmt::ComponentDef(def) => Some(def),
            _ => None,
        })
        .expect("a component");
    component_deps(def, interner)
        .expect("deps")
        .sites
        .iter()
        .map(|s| (s.key(interner), s.patchable()))
        .collect()
}

#[test]
fn a_plain_hole_is_patchable_in_place() {
    // It names ONE node that lives as long as the component is mounted, so a turn can write the
    // new text into it and touch nothing else. This is the case the whole engine exists for.
    let got = patchable(&format!(
        "{HEAD}    Let draft be \"\".\n\nView:\n    <p>{{draft}}</p>\n"
    ));
    assert_eq!(got, vec![("text@0.0".to_string(), true)]);
}

#[test]
fn a_site_inside_a_loop_is_not_patchable() {
    // ★ A loop body's path names a SHAPE, not a node: it renders once per item, so `text@0.0.0.0`
    // is a hundred nodes when the list is a hundred long. Patching "the" node there would write
    // into whichever one happened to be recorded last — a wrong answer that looks like an update.
    let got = patchable(&format!(
        "{HEAD}    Let todos be [\"a\"].\n\nView:\n    \
         <ul>{{for each t in todos:}}<li>{{t}}</li>{{end}}</ul>\n"
    ));
    let inner = got
        .iter()
        .find(|(k, _)| k.starts_with("text@"))
        .expect("the row's hole");
    assert!(
        !inner.1,
        "a hole inside a loop body is not patchable in place: {got:?}"
    );
}

#[test]
fn a_site_inside_a_condition_is_not_patchable() {
    // An arm's nodes exist only while its condition holds, so an update there is a MOUNT or an
    // UNMOUNT — a different operation from writing a value into a node that is already present.
    let got = patchable(&format!(
        "{HEAD}    Let error be \"\".\n\nView:\n    \
         {{if error is not \"\":}}<p>{{error}}</p>{{end}}\n"
    ));
    let inner = got
        .iter()
        .find(|(k, _)| k.starts_with("text@"))
        .expect("the arm's hole");
    assert!(
        !inner.1,
        "a hole inside an `{{if}}` arm is not patchable in place: {got:?}"
    );
}

#[test]
fn a_structural_site_is_never_patchable() {
    // The `{if}` and `{for each}` sites THEMSELVES mount and unmount subtrees; they are the
    // signal to rebuild, never something to write a value into.
    let got = patchable(&format!(
        "{HEAD}    Let n be 1.\n    Let todos be [\"a\"].\n\nView:\n    \
         <div>{{if n is 1:}}<p>x</p>{{end}}<ul>{{for each t in todos:}}<li>{{t}}</li>{{end}}</ul></div>\n"
    ));
    for (key, ok) in &got {
        if key.starts_with("if@") || key.starts_with("each@") {
            assert!(
                !ok,
                "`{key}` is structural and must not be patchable: {got:?}"
            );
        }
    }
}

#[test]
fn an_element_nested_in_plain_markup_stays_patchable() {
    // The restriction is about CONTROL FLOW, not about depth: nesting inside ordinary elements
    // keeps a single node per site, and a rule keyed on depth would give up on every real page.
    let got = patchable(&format!(
        "{HEAD}    Let n be 1.\n\nView:\n    <div><section><p>{{n}}</p></section></div>\n"
    ));
    assert_eq!(got, vec![("text@0.0.0.0".to_string(), true)]);
}

#[test]
fn a_binding_is_patchable_and_an_attribute_is_not_yet() {
    // ★ THE SET IS WHAT BOTH TIERS CAN EMIT, not what one of them could. The reference tier and
    // the bytecode tier must agree about which turns patch and which repaint, or their op-logs
    // diverge on the same page — and the op-log is the contract. A binding writes one value into
    // one node on both; an ATTRIBUTE does not yet, because a class toggle recomposes the
    // element's whole class list and the bytecode tier builds that inline during the paint.
    //
    // Asserted in BOTH directions on purpose: the negative half is what keeps the two tiers
    // honest, and it is the line to move — in one place — when the bytecode patcher grows.
    let got = patchable(&format!(
        "{HEAD}    Let draft be \"\".\n    Let title be \"t\".\n\nView:\n    \
         <div><input bind={{draft}}><p title={{title}}>x</p></div>\n"
    ));
    assert!(
        got.iter()
            .any(|(k, ok)| k.starts_with("bind:draft@") && *ok),
        "{got:?}"
    );
    assert!(
        got.iter()
            .any(|(k, ok)| k.starts_with("attr:title@") && !*ok),
        "{got:?}"
    );
}
