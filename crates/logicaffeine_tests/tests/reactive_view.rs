//! Reactive view-DSL desugaring — the parser-level locks for control flow and keying
//! inside a `## Component` `View:` block. The full cross-tier runtime behaviour is locked
//! by `logicaffeine_compile/tests/wasm_dom.rs` and `dom_reconciliation_lock.rs`; these
//! tests pin the *shape* of the desugar, which the language crate owns alone.
//!
//! ★ THE KEYING MECHANISM ITSELF CHANGED WITH THE RETIRED REACTIVE-SIGNAL DIALECT. That
//! dialect kept a component's state on a push/pop SCOPE STACK (`enterComponent(key)` /
//! `exitComponent()`, `BuiltinId::SignalEnterComponent` — wire-frozen, unreachable now,
//! see `legacy_dialect_gone_lock.rs`), so a placed child's identity was the single string
//! argument `enterComponent` was called with — dynamic (`Expr::InterpolatedString`) for a
//! `keyed by` loop, static (`Expr::Literal(Text)`) for an unkeyed one.
//!
//! LGX keeps per-instance state in flat maps indexed by an INSTANCE KEY string built at
//! the placement site (`lgx_lower.rs`'s `child_key`, read at `item <key> of __S_<C>_<field>`):
//! the owner's own key, the placement's static path, and — inside a `for each` — either the
//! loop's `keyed by` expression (an author expression of ANY shape, spliced in as-is) or a
//! synthesized position counter (`__lgx_posN`, never an author expression). The concatenation
//! is built from `+` (`Expr::BinaryOp`), not `Expr::InterpolatedString` — a different tree
//! shape carrying the identical distinction the old dialect drew: a keyed placement's identity
//! contains the AUTHOR's own expression (here, `todo's id` — an `Expr::FieldAccess`, the only
//! source of one anywhere in a synthesized key); an unkeyed placement's contains none, only
//! positions and paths this pass minted itself.

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::TypeRegistry;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::{Lexer, Parser};

fn parse(input: &str) -> (Vec<Stmt<'static>>, &'static Interner) {
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
        .unwrap_or_else(|e| panic!("program should parse: {e:?}\n{input}"));
    (stmts, interner)
}

/// Every nested `Block` a statement carries (so the walkers can recurse into a
/// synthesized `__render_<Component>` function's body, and the `Repeat` inside it).
fn child_blocks<'a>(s: &'a Stmt<'a>) -> Vec<&'a [Stmt<'a>]> {
    match s {
        Stmt::FunctionDef { body, .. }
        | Stmt::Repeat { body, .. }
        | Stmt::While { body, .. }
        | Stmt::Splice { body, .. } => vec![body],
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            let mut v = vec![*then_block];
            if let Some(e) = else_block {
                v.push(*e);
            }
            v
        }
        _ => vec![],
    }
}

/// Every `Stmt::Call` reachable from `stmts` whose callee resolves to `name`.
fn find_calls_to<'a>(
    stmts: &'a [Stmt<'a>],
    name: &str,
    interner: &Interner,
    out: &mut Vec<&'a [&'a Expr<'a>]>,
) {
    for s in stmts {
        if let Stmt::Call { function, args } = s {
            if interner.resolve(*function) == name {
                out.push(args);
            }
        }
        for b in child_blocks(s) {
            find_calls_to(b, name, interner, out);
        }
    }
}

/// The value a `Let` anywhere in `stmts` binds to `name`.
fn let_value<'a>(stmts: &'a [Stmt<'a>], name: Symbol) -> Option<&'a Expr<'a>> {
    stmts.iter().find_map(|s| match s {
        Stmt::Let { var, value, .. } if *var == name => Some(*value),
        _ => child_blocks(s).into_iter().find_map(|b| let_value(b, name)),
    })
}

/// The instance key `List`'s placement of `Item` hands `__render_Item` — its first argument — and
/// `List`'s render, where the locals the key is built from are bound (a temporary's name is unique
/// within one function, not across the program). `Item`'s own mount draws it too, under the key it
/// is mounted with; the placement is the one read here.
fn placed_item_key(
    stmts: &'static [Stmt<'static>],
    interner: &Interner,
) -> (&'static Expr<'static>, &'static [Stmt<'static>]) {
    let placer = stmts
        .iter()
        .find_map(|s| match s {
            Stmt::FunctionDef { name, body, .. } if interner.resolve(*name) == "__render_List" => {
                Some(*body)
            }
            _ => None,
        })
        .expect("`List` lowers to `__render_List`");
    let mut calls = Vec::new();
    find_calls_to(placer, "__render_Item", interner, &mut calls);
    assert!(
        !calls.is_empty(),
        "the placement must lower to a call to __render_Item"
    );
    let key = calls[0]
        .first()
        .expect("__render_Item's first argument is the placed instance's key");
    (key, placer)
}

/// Whether an expression tree contains a `FieldAccess` anywhere — the one node type a
/// synthesized instance key never produces on its own; its presence means an author
/// expression (a `keyed by <expr>`) was spliced into the key. A local the key is built from (the
/// loop's row, the key itself) is read through the `Let` in `stmts` that binds it.
fn contains_field_access(e: &Expr, stmts: &'static [Stmt<'static>]) -> bool {
    match e {
        Expr::FieldAccess { .. } => true,
        Expr::BinaryOp { left, right, .. } => {
            contains_field_access(left, stmts) || contains_field_access(right, stmts)
        }
        Expr::Identifier(local) => {
            let_value(stmts, *local).is_some_and(|v| contains_field_access(v, stmts))
        }
        _ => false,
    }
}

#[test]
fn keyed_for_each_gives_the_placed_component_a_key_built_from_the_authors_expression() {
    // `keyed by todo's id` must make the placed child's instance key carry the author's own
    // expression (`todo's id`, an `Expr::FieldAccess`) — so a stateful list item keeps its own
    // state across add/remove/reorder, because its key travels with the item and not with the
    // position it happens to render at.
    let src = "\
## Component Item
View:
    <li>row</li>

## Component List
State:
    Let todos be [].
View:
    <ul>{for each todo in todos, keyed by todo's id:}<Item/>{end}</ul>

## Main
Mount List at \"#root\".
";
    let (stmts, interner) = parse(src);
    let stmts: &'static [Stmt<'static>] = Box::leak(stmts.into_boxed_slice());
    let (child_key, placer) = placed_item_key(stmts, interner);
    assert!(
        contains_field_access(child_key, placer),
        "a keyed placement's instance key must carry the author's `keyed by` expression \
         (a FieldAccess for `todo's id`), got: {child_key:?}"
    );
}

#[test]
fn unkeyed_for_each_gives_the_placed_component_a_position_only_key() {
    // Without `keyed by`, the placed child's instance key is built from the loop POSITION
    // alone — no author expression reaches it, so two different items rendered at the same
    // position share one instance (the existing, unchanged behaviour a keyed loop opts out of).
    let src = "\
## Component Item
View:
    <li>row</li>

## Component List
State:
    Let todos be [].
View:
    <ul>{for each todo in todos:}<Item/>{end}</ul>

## Main
Mount List at \"#root\".
";
    let (stmts, interner) = parse(src);
    let stmts: &'static [Stmt<'static>] = Box::leak(stmts.into_boxed_slice());
    let (child_key, placer) = placed_item_key(stmts, interner);
    assert!(
        !contains_field_access(child_key, placer),
        "an unkeyed placement's instance key must carry no author expression, only positions \
         and paths this pass minted itself, got: {child_key:?}"
    );
}
