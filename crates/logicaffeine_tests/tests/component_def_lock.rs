//! ★ LOCK — the LGX component model parses into first-class AST: `## Page` / `## Component` /
//! `## Store` / `## Error page` with their `State:` / `Actions:` / `View:` / `Style:` sections,
//! plus `## Directive`, `## Route`, `## Before each request` and `## Redirect`. ★
//!
//! Every construct of the component surface has a shape snapshot here: the header (kind,
//! props, path params, store mode), `Use`, state fields (plain and derived, typed and
//! inferred), every English event sentence as its `Trigger`, actions with params, the view
//! tree (holes sub-lexed by the real expression parser, so `{count of todos}` is a real
//! expression), and the style block. Errors point INTO the HTML (span inside the raw body).

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::DiscoveryPass;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::component::RouteBody;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::error::ParseError;
use logicaffeine_language::{Lexer, Parser};

#[path = "support/lgx_lower_source.rs"]
mod lgx_lower_source;

struct Parsed {
    stmts: Vec<Stmt<'static>>,
    interner: &'static Interner,
}

fn parse(input: &str) -> Result<Parsed, ParseError> {
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
    // The real pipeline discovers user types before parsing, so `a new Seq of Todo` resolves.
    let registry = {
        let mut discovery = DiscoveryPass::new(&tokens, interner);
        discovery.run_full().types
    };
    let mut parser = Parser::new(tokens, world_state, interner, ctx, registry);
    let stmts = parser.parse_program()?;
    drop(parser);
    Ok(Parsed { stmts, interner })
}

/// The parse error of a program that must be refused.
fn parse_err(input: &str, why: &str) -> ParseError {
    match parse(input) {
        Ok(_) => panic!("{why}:\n{input}"),
        Err(e) => e,
    }
}

fn shapes(input: &str) -> Vec<String> {
    let p = parse(input).unwrap_or_else(|e| panic!("must parse: {e:?}\n{input}"));
    p.stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::ComponentDef(def) => Some(def.shape(p.interner)),
            Stmt::DirectiveDef {
                name,
                params,
                body,
                removed,
            } => Some(format!(
                "directive {}({}): {}{}",
                p.interner.resolve(*name),
                params
                    .iter()
                    .map(|(n, _)| p.interner.resolve(*n))
                    .collect::<Vec<_>>()
                    .join(", "),
                body.len(),
                removed
                    .map(|r| format!(" removed: {}", r.len()))
                    .unwrap_or_default()
            )),
            Stmt::RouteDef {
                method,
                path,
                params,
                return_type,
                body,
                ..
            } => Some(format!(
                "route {} {}({}){}: {}",
                p.interner.resolve(*method),
                p.interner.resolve(*path),
                params
                    .iter()
                    .map(|(n, _)| p.interner.resolve(*n))
                    .collect::<Vec<_>>()
                    .join(", "),
                if return_type.is_some() { " -> T" } else { "" },
                match &**body {
                    RouteBody::Statements(b) => format!("{} statements", b.len()),
                    RouteBody::View(v) => format!("view{}", v.body_shape(p.interner)),
                }
            )),
            Stmt::MiddlewareDef { param, body } => Some(format!(
                "middleware({}): {}",
                p.interner.resolve(param.0),
                body.len()
            )),
            Stmt::RedirectDef { from, to } => Some(format!(
                "redirect {} -> {}",
                p.interner.resolve(*from),
                p.interner.resolve(*to)
            )),
            _ => None,
        })
        .collect()
}

const TODOS_PAGE: &str = "\
# Todos
## A Todo has:
    an id, which is Int.
    a text, which is Text.
    a done, which is Bool.

## Page /todos/:id (id: Int)
Use Session.

State:
    Let todos be a new Seq of Todo.
    Let draft be \"\".
    Let ratio: Float be 1.
    Let remaining always be length of todos.

Actions:
    When the page loads:
        Set draft to \"\".
    When \"#add\" is submitted:
        Set draft to \"\".
    When the user presses Escape in \"#draft\":
        Set draft to \"\".
    When todos change:
        Set draft to \"\".
    When Session's name changes:
        Set draft to \"\".
    Every 60 seconds:
        Set draft to \"\".
    When an action fails (e: Text):
        Set draft to e.
    To add:
        If draft is equal to \"\":
            Return.
        Set draft to \"\".
    To remove (n: Int):
        Set draft to \"\".

View:
    <div class=\"card\">
      <h1>Todos <small>{remaining} left</small></h1>
      <form id=\"add\">
        <input id=\"draft\" type=\"text\" bind={draft}>
        <button type=\"submit\" disabled={draft is equal to \"\"}>Add</button>
      </form>
      <ul>
        {for each todo in todos, keyed by todo's id:}
          <TodoRow todo={todo} remove={remove}/>
        {else:}
          <li>none</li>
        {end}
      </ul>
    </div>

Style:
    .card { margin: 0; }

## Component TodoRow (todo: Todo) and (remove: fn(Int) -> Unit)
View:
    <li class:done={todo's done}><span>{todo's text}</span></li>

## Store Session persisted as \"session\"
State:
    Let name be \"guest\".
Actions:
    To login (who: Text):
        Set name to who.
";

#[test]
fn a_page_parses_into_a_component_def_with_every_section() {
    let got = shapes(TODOS_PAGE);
    assert_eq!(
        got.len(),
        3,
        "the page, the `TodoRow` it places, and the `Session` store it uses: {got:?}"
    );
    let s = &got[0];
    // ★ `id` IS ORDINARY STATE, and that is the whole fix. `## Page /todos/:id (id: Int)` used to
    // parse and then fail at runtime with `Undefined variable: id`, because
    // `ComponentKind::Page { params }` was carried and never read. A path parameter now lowers to
    // a DERIVED state field over `routeParam`/`routePath` — both of which `web.lg` already shipped
    // — so no tier had to learn what a path parameter is, and a client-side navigation that
    // changes the URL changes the value. A plain binding would freeze the first id the page was
    // ever mounted at.
    // ★ THE `:id` SEGMENT BINDS, AND ITS BINDING IS ORDINARY STATE. The header declares the
    // parameter; the front of `State:` is where it arrives, read the same way any other field
    // is by `State:`, `Actions:` and `View:` alike. It sits FIRST because a later field may
    // read it (`Let post always be findPost(id).`). Before this, `(id: Int)` was parsed,
    // carried through `ComponentKind::Page { params }` and never read, so the page died with
    // `Undefined variable: id` — the same bug `path` itself had, one field away in the same
    // variant. `page_params_lock` holds the behaviour; this holds the shape.
    assert!(s.starts_with("page /todos/:id(id: Int) uses[Session] state{id: Int := parseInt(routeParam(\"/todos/:id\", \"id\", routePath())); todos = new Seq; draft = \"\"; ratio: Float = 1; remaining := length of todos}"), "header/state: {s}");
    assert!(s.contains("events{page loads: 1; \"#add\" submit: 1; \"#draft\" keydown Escape: 1; todos changes: 1; Session's name changes: 1; every 60000000000ms: 1; action fails (e): 1}"), "events: {s}");
    assert!(
        s.contains("actions{add: 2; remove(n: Int): 1}"),
        "actions: {s}"
    );
    assert!(s.contains("view{div[class=\"card\"]{h1{\"Todos \", small{remaining, \" left\"}}, form[id=\"add\"]{input[id=\"draft\", type=\"text\", bind=draft], button[type=\"submit\", disabled=(draft Eq \"\")]{\"Add\"}}, ul{each todo in todos keyed by todo's id{<TodoRow todo=todo remove=remove>} else{li{\"none\"}}}}}"), "view: {s}");
    assert!(s.ends_with(" style"), "style: {s}");
}

#[test]
fn a_component_with_props_and_a_store_with_modes_parse() {
    let src = "## Component TodoRow (todo: Todo) and (remove: fn(Int) -> Unit)\nView:\n    <li class:done={todo's done}><span>{todo's text}</span></li>\n\n## Store Session persisted as \"session\"\nState:\n    Let name be \"guest\".\nActions:\n    To login (who: Text):\n        Set name to who.\n\n## Store Board shared on \"board-{room}\"\nState:\n    Let notes be a new Seq of Text.\n\n## Store Scratch\nState:\n    Let n be 0.\n";
    let got = shapes(src);
    assert_eq!(got.len(), 4, "{got:?}");
    assert_eq!(got[0], "component TodoRow(todo: Todo, remove: Function) view{li[class:done=todo's done]{span{todo's text}}}");
    assert_eq!(got[1], "store Session persisted as \"session\" state{name = \"guest\"} actions{login(who: Text): 1}");
    assert!(
        got[2].starts_with("store Board shared on \"board-{room}\" state{notes = new Seq}"),
        "{}",
        got[2]
    );
    assert_eq!(got[3], "store Scratch state{n = 0}");
}

#[test]
fn directives_routes_middleware_redirects_and_error_pages_parse() {
    let src = "## Directive tooltip (el: Element) and (text: Text):\n    Call setAttribute with el and \"title\" and text.\n    When removed:\n        Show 1.\n\n## Route GET /api/todos/:id (id: Int) -> Result of Todo and HttpError:\n    Return findTodo(id).\n\n## Route GET /report -> View:\n    <h1>{length of allTodos()} todos</h1>\n\n## Before each request (request: Request) -> Option of Response:\n    Return none.\n\n## Redirect /old-todos -> /todos.\n\n## Error page 404\nView:\n    <h1>Lost</h1>\n";
    let got = shapes(src);
    assert_eq!(
        got,
        [
            "directive tooltip(el, text): 1 removed: 1",
            "route GET /api/todos/:id(id) -> T: 1 statements",
            "route GET /report() -> T: view{h1{length of allTodos(), \" todos\"}}",
            "middleware(request): 1",
            "redirect /old-todos -> /todos",
            "error page 404 view{h1{\"Lost\"}}",
        ]
    );
}

#[test]
fn holes_are_real_expressions_with_spans_inside_the_html() {
    // A hole that does not parse as a LOGOS expression is reported at the hole, not at the
    // `View:` line — the span lands inside the raw body.
    let src = "## Page /x\nView:\n    <p>ok</p>\n    <p>{count of}</p>\n";
    let err = parse_err(src, "a broken hole is a parse error");
    let hole_at = src.find("{count of}").unwrap();
    assert!(
        err.span.start >= hole_at && err.span.start < hole_at + "{count of}".len(),
        "error span {:?} must sit inside the hole at {hole_at}: {err:?}",
        err.span
    );
}

#[test]
fn template_errors_point_into_the_html() {
    let src = "## Page /x\nView:\n    <div>\n      <span></div>\n";
    let err = parse_err(src, "a mismatched end tag is a parse error");
    let at = src.find("</div>").unwrap();
    assert_eq!(err.span.start, at, "{err:?}");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("</div>") && msg.contains("<span>"),
        "the Socratic message survives: {msg}"
    );
}

#[test]
fn sections_outside_a_component_are_refused_with_guidance() {
    for (src, needle) in [
        ("## Main\nState:\n    Let x be 1.\n", "State:"),
        ("## To f ():\n    Return 1.\nView:\n    <p>x</p>\n", "View:"),
        ("## Main\nStyle:\n    .a { }\n", "Style:"),
    ] {
        let err = parse_err(src, &format!("{src:?} must be refused"));
        assert!(format!("{err:?}").contains(needle), "{src:?}: {err:?}");
    }
}

/// ★ RETIRED. The legacy dialect's own indentation — a component's `State:`/`View:`
/// sections nested one level UNDER the header, rather than flush with it — is no longer a
/// second accepted layout: `## Component App` always goes through `parse_lgx_component_block`
/// now (the header's trailing `:` is just optional punctuation since item 3), which looks for
/// `State:`/`Actions:`/`View:`/`Style:` at the SAME indentation as the header and finds an
/// `Indent` instead. There is exactly one component layout, not two.
///
/// The refusal names the token that stands for the mistake, not a quoted, empty pair of
/// backticks (`Indent` carries no author-written lexeme to quote) — and teaches the fix by
/// naming both real sections the old, single indented body should have been split into.
#[test]
fn the_legacy_indentation_is_refused_not_silently_reparsed() {
    let src = "## Component App:\n    Let n be 0.\n    View:\n        h1: \"Count: {n}\"\n## Main\nMount App.\n";
    let err = parse_err(
        src,
        "the legacy dialect's nested indentation must no longer parse as a component",
    );
    let message = format!("{err:?}");
    assert!(
        message.contains("State:"),
        "the refusal must teach where state goes, by name: {message}"
    );
    assert!(
        message.contains("View:"),
        "the refusal must teach where the view goes, by name: {message}"
    );
    // ★ NOT THE GENERIC CATCH-ALL. This exact fixture hits the header's stray colon, which the
    // lexer turns into an unnamed `Indent` token — the GENERIC "`{token}` is not part of a
    // component" fallback also happens to mention `State:`/`View:` in its own section list, so the
    // two asserts above alone would pass even quoting an EMPTY pair of backticks for the mistake
    // itself. Pinning the absence of that empty quote is what actually distinguishes "the message
    // names the right sections" from "the message names the right sections AND says what you
    // actually did wrong".
    assert!(
        !message.contains("`` is not part of a component"),
        "this fixture must get the TARGETED colon message, not the generic fallback quoting an \
         empty token: {message}"
    );
}

/// ★ A declared type DECIDES what the slot holds — the same rule at the top level and inside a
/// `State:` block, because LGX is a layer on LOGOS and not a dialect with rules of its own.
///
/// `Let r: Float be 1.` is the canonical explicit annotation (the fixture above writes
/// `Let ratio: Float be 1.`), and a whole number is a value a Real slot has — the same widening
/// LOGOS already allowed for `Let x: Rational be 5`. Getting this wrong is quiet in both
/// directions: refusing it rejects proper LOGOS, and accepting it while storing an Int makes
/// `r / 2` render 0 instead of 0.5. So the annotation is checked AND it widens the literal.
#[test]
fn a_declared_real_slot_holds_a_whole_number_as_a_real() {
    for src in [
        "## Main\nLet r: Float be 1.\nShow r / 2.\n",
        "## Main\nLet r: Real be 3.\nShow r / 2.\n",
        "## Component App\nState:\n    Let r: Real be 3.\nView:\n    <p>{r}</p>\n\n## Main\nMount App.\n",
        "## Component App\nState:\n    Let r: Float be 1.\nView:\n    <p>{r}</p>\n\n## Main\nMount App.\n",
    ] {
        assert!(parse(src).is_ok(), "proper LOGOS must parse:\n{src}");
    }
}

/// The check still bites where the two sides genuinely disagree, and it bites IDENTICALLY in both
/// places — one rule, one message. A second copy of the rule would drift from the first.
#[test]
fn a_slot_and_its_value_must_still_agree() {
    let field = parse_err(
        "## Component App\nState:\n    Let n: Int be \"hi\".\nView:\n    <p>{n}</p>\n\n## Main\nMount App.\n",
        "a `State:` field whose annotation contradicts its initializer must be refused",
    );
    let top = parse_err(
        "## Main\nLet n: Int be \"hi\".\nShow n.\n",
        "a top-level `Let` whose annotation contradicts its initializer must be refused",
    );
    assert_eq!(
        format!("{:?}", top.kind),
        format!("{:?}", field.kind),
        "a State field and a Let must be refused by the SAME rule, not two that resemble each other"
    );
    assert!(
        format!("{field:?}").contains("Int") && format!("{field:?}").contains("Text"),
        "the refusal names BOTH sides so the author can see which is lying: {field:?}"
    );
}

/// An annotation that agrees, and no annotation at all, are both fine — the rule refuses a
/// contradiction, not the act of declaring.
#[test]
fn a_state_field_annotation_that_agrees_is_accepted() {
    for src in [
        "## Component App\nState:\n    Let rate: Real be 3.0.\nView:\n    <p>{rate}</p>\n\n## Main\nMount App.\n",
        "## Component App\nState:\n    Let n: Int be 3.\nView:\n    <p>{n}</p>\n\n## Main\nMount App.\n",
        "## Component App\nState:\n    Let t: Text be \"hi\".\nView:\n    <p>{t}</p>\n\n## Main\nMount App.\n",
        "## Component App\nState:\n    Let n be 3.\nView:\n    <p>{n}</p>\n\n## Main\nMount App.\n",
    ] {
        assert!(parse(src).is_ok(), "an agreeing annotation must be accepted:\n{src}");
    }
}

/// The 1-based line of every `Expr::Closure { … }` in `source` that BUILDS a closure. One that
/// MATCHES a closure — a walk reading what a closure's body reads — is a pattern, not a closure
/// anybody hands a host: its fields end in `..`, or its braces close into `=>` (a match arm), `=`
/// (a `let`/`if let`) or `|` (an or-pattern).
fn closure_constructions(source: &str) -> Vec<usize> {
    const OPEN: &str = "Expr::Closure {";
    let mut lines = Vec::new();
    let mut from = 0;
    while let Some(at) = source[from..].find(OPEN) {
        let start = from + at;
        let brace = start + OPEN.len() - 1;
        let mut depth = 0usize;
        let mut close = brace;
        for (offset, c) in source[brace..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = brace + offset;
                        break;
                    }
                }
                _ => {}
            }
        }
        let fields = &source[brace + 1..close];
        let after = source[close + 1..].trim_start();
        let pattern = fields.contains("..")
            || after.starts_with("=>")
            || (after.starts_with('=') && !after.starts_with("=="))
            || after.starts_with('|');
        if !pattern {
            lines.push(source[..start].matches('\n').count() + 1);
        }
        from = start + OPEN.len();
    }
    lines
}

/// The census counts what it says it counts: a closure built is counted, a closure matched is not.
#[test]
fn the_closure_census_tells_a_closure_built_from_a_closure_matched() {
    let source = "\
let built = self.alloc(Expr::Closure { params, body, return_type });
match e {
    Expr::Closure { body, .. } => walk(body),
    Expr::Closure {
        params,
        body,
        return_type,
    } => walk(body),
}
if let Expr::Closure { params, body, return_type } = e {}
let also = Expr::Closure {
    params: vec![],
    body,
    return_type: None,
};
";
    assert_eq!(
        closure_constructions(source),
        [1, 11],
        "the two builds, and none of the three matches"
    );
}

/// ★ CENSUS — every closure the `lgx_lower` module hands to a callback-registering host import goes
/// through ONE builder. ★
///
/// The wasm trampoline (`__logos_call_closure`) only dispatches a closure taking exactly one
/// `Int` — every other shape traps the instant the host fires it (`emit_trampoline_arm`,
/// `vm/wasm/module.rs`). A route-change handler built with `params: Vec::new()` directly, one
/// line away from the shared builder, went unnoticed until the very host call that exercises it
/// trapped the whole page. Grepping the SOURCE rather than compiling a fixture per trigger kind
/// is deliberate: the invariant this locks is "nobody constructs `Expr::Closure` by hand a second
/// time", which a compiled program's own shape cannot distinguish from "got the right shape by
/// coincidence".
///
/// Exactly two raw `Expr::Closure { .. }` sites are expected, and both are accounted for:
/// `host_callback_closure` itself (the one canonical builder every host-callback closure now
/// goes through, `listener_closure` included — it is `host_callback_closure(vec![call], want)`)
/// and `action_prop_value` (a callback PROP passed from a parent component to a child's own
/// action, LOGOS calling LOGOS through `call_value` — it never crosses the wasm host boundary
/// through the trampoline at all, so the one-`Int` shape is not its rule to follow). A third site
/// appearing means a new closure was built outside the census this test polices — read what it is
/// before raising the count.
#[test]
fn every_lgx_closure_is_built_by_the_host_callback_helper_or_the_named_exception() {
    let sites: Vec<String> = lgx_lower_source::files()
        .iter()
        .flat_map(|(file, source)| {
            closure_constructions(source)
                .into_iter()
                .map(move |line| format!("{file}:{line}"))
        })
        .collect();
    assert_eq!(
        sites.len(),
        2,
        "expected exactly 2 raw `Expr::Closure {{ .. }}` construction sites in the lgx_lower module \
         (`host_callback_closure`'s own, and `action_prop_value`'s named exception) — found at \
         {sites:?}. A new site means a closure was built outside this census: give it a \
         reason here (a non-host-callback shape, like `action_prop_value`) or route it through \
         `host_callback_closure` instead."
    );
}

/// ★ CENSUS — every turn body the `lgx_lower` module builds from an author's own statements is protected
/// against an early `Return` skipping the turn's epilogue. ★
///
/// A `let <name> = self.rewrite_store_calls(...)` marks a TURN ENTRY — the one place in a
/// turn-building function where a whole author-written body (a component action, a `When …:`
/// handler, a `PageLoads`/`Appears` inline handler, a `## Store`'s own action, a field-change
/// watcher) enters the lowering. `rewrite_store_call_stmt`'s OWN recursive sub-calls (into an
/// `If`/`While`/`Repeat`/`Splice`/`Inspect` arm it is already walking) are written inline as a
/// struct field's value, never bound to a `let` first — that shape is deliberately EXCLUDED here,
/// since protecting the outer call already covers everything nested inside it (`protect_early_
/// return`'s own recursion handles the nesting, once, at the entry). Every TURN ENTRY site must be
/// followed by `protect_early_return`, or a `Return` inside THAT shape silently skips the state
/// write-back and `noteTurn` again — the exact data-loss bug this file's sibling lock
/// (`early_return_turn_epilogue_lock.rs`, `logicaffeine_tests`) catches behaviorally. This is the
/// SOURCE half: a new turn-entry site added later without its own `protect_early_return` line
/// would still compile and might even pass a fixture that happens not to use `Return` — this
/// census catches the shape gap directly, the way the closure census above catches a raw
/// `Expr::Closure` the same way.
#[test]
fn every_rewrite_store_calls_turn_entry_is_followed_by_protect_early_return() {
    // Each `let <name> = self.rewrite_store_calls(…)` is PAIRED with the line after it, which must
    // protect that same binding — not counted against every `protect_early_return` in the module,
    // since a block that is no turn entry (a directive's) may protect its own body too.
    let source = lgx_lower_source::source();
    let lines: Vec<&str> = source.lines().map(str::trim_start).collect();
    let mut entries = 0usize;
    let mut unguarded = Vec::new();
    for (i, t) in lines.iter().enumerate() {
        if !(t.starts_with("let ") && t.contains("= self.rewrite_store_calls(")) {
            continue;
        }
        entries += 1;
        let name = t["let ".len()..]
            .split(|c: char| c == ' ' || c == ':' || c == '=')
            .next()
            .unwrap_or("");
        let guard = format!("let {name} = self.protect_early_return({name}.to_vec());");
        let next = lines[i + 1..]
            .iter()
            .find(|l| !l.is_empty())
            .copied()
            .unwrap_or("");
        if next != guard {
            unguarded.push(format!("`{t}` is followed by `{next}`"));
        }
    }
    assert!(
        entries >= 4,
        "read only {entries} turn-entry sites — the reader stopped seeing them"
    );
    assert!(
        unguarded.is_empty(),
        "a turn-building site is missing the guard against an early `Return` skipping its own \
         write-back and `noteTurn` — add `let <name> = self.protect_early_return(<name>.to_vec());` \
         right after the site's own `rewrite_store_calls` call:\n{}",
        unguarded.join("\n")
    );
}

#[test]
fn an_action_fails_handler_with_a_non_text_annotation_is_refused() {
    let src = "## Page /x\nActions:\n    When an action fails (e: Int):\n        Set draft to e.\n";
    let err = parse_err(src, "`(e: Int)` delivers no Int; only Text");
    let message = format!("{err:?}");
    assert!(
        message.contains("Text"),
        "must name what the trigger actually delivers: {message}"
    );
    assert!(
        message.contains("Int"),
        "must name what the author actually wrote: {message}"
    );
}

#[test]
fn a_fetch_fails_handler_with_a_non_text_url_annotation_is_refused() {
    let src = "## Page /x\nActions:\n    When a fetch fails (url: Int) and (error: Text):\n        Set draft to error.\n";
    let err = parse_err(src, "`(url: Int)` delivers no Int; only Text");
    let message = format!("{err:?}");
    assert!(
        message.contains("Text"),
        "must name what the trigger actually delivers: {message}"
    );
}

#[test]
fn a_fetch_fails_handler_with_a_non_text_error_annotation_is_refused() {
    let src = "## Page /x\nActions:\n    When a fetch fails (url: Text) and (error: Int):\n        Set draft to error.\n";
    let err = parse_err(src, "`(error: Int)` delivers no Int; only Text");
    let message = format!("{err:?}");
    assert!(
        message.contains("Text"),
        "must name what the trigger actually delivers: {message}"
    );
}

#[test]
fn a_file_dropped_handler_with_a_non_file_non_text_annotation_is_refused() {
    let src = "## Page /x\nActions:\n    When a file is dropped on \"#zone\" (n: Int):\n        Set draft to n.\n";
    let err = parse_err(src, "`(n: Int)` binds neither `File` nor `Text`");
    let message = format!("{err:?}");
    assert!(
        message.contains("File"),
        "must name the record option: {message}"
    );
    assert!(
        message.contains("Text"),
        "must name the bare-name option: {message}"
    );
}

/// The refusal's converse: both declared shapes a dropped file may actually bind as still parse
/// cleanly, and each shows up in the trigger's own shape exactly as declared — proving the
/// refusal above is precise (rejects the wrong type) rather than accidentally rejecting everything.
#[test]
fn a_file_dropped_handler_accepts_both_its_declared_shapes() {
    let text_src = "## Page /x\nActions:\n    When a file is dropped on \"#zone\" (name: Text):\n        Set draft to name.\n";
    let file_src = "## Page /x\nActions:\n    When a file is dropped on \"#zone\" (file: File):\n        Set draft to file's name.\n";
    for src in [text_src, file_src] {
        let got = shapes(src);
        assert_eq!(got.len(), 1, "{src:?}: {got:?}");
        assert!(
            got[0].contains("file dropped on \"#zone\" (name)")
                || got[0].contains("file dropped on \"#zone\" (file)"),
            "{src:?}: {}",
            got[0]
        );
    }
}
