//! ★ LOCK — `Element`, `Event` and `View` are NOMINAL types, not spellings of `Int`. ★
//!
//! The DOM handle types used to be rewritten to `Int` by the parser, so `Let x: Element be 3.`
//! type-checked, a `-> Element` function could `Return 0`, and every downstream tier saw a
//! bare integer. A view framework cannot stand on that: a handle is not a number, and the
//! checker must say so. This lock pins that the three names are distinct nominal types in the
//! static checker while their RUNTIME representation stays the i64 handle every tier already
//! moves around (so nothing about execution changes — only what the checker accepts).

use logicaffeine_base::{Arena, Interner};
use logicaffeine_compile::analysis::check_program;
use logicaffeine_compile::compile::{compile_to_rust, compile_to_wasm, tw_outcome, vm_outcome};
use logicaffeine_language::{
    analysis::DiscoveryPass,
    arena_ctx::AstContext,
    ast::{Expr, Stmt, TypeExpr},
    drs::WorldState,
    Lexer, Parser,
};

/// Parse `source` and run the strict type checker over it: `Ok` or the first error's text.
fn check(source: &str) -> Result<(), String> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    let mwe_trie = logicaffeine_language::mwe::build_mwe_trie();
    let tokens = logicaffeine_language::mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);
    let type_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        discovery.run_full().types
    };
    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();
    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );
    // The parser already type-checks a literal against a `Let` annotation, so a rejection may
    // surface as a parse error; either layer saying no is the checker saying no.
    let stmts = {
        let mut parser = Parser::new(
            tokens,
            &mut world_state,
            &mut interner,
            ast_ctx,
            type_registry.clone(),
        );
        match parser.parse_program() {
            Ok(stmts) => stmts,
            Err(e) => return Err(format!("parse: {e:?}")),
        }
    };
    check_program(&stmts, &interner, &type_registry)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}

fn assert_rejects(source: &str, why: &str) {
    match check(source) {
        Ok(()) => panic!("the checker must reject this ({why}):\n{source}"),
        Err(_) => {}
    }
}

fn assert_accepts(source: &str) {
    if let Err(e) = check(source) {
        panic!("the checker must accept this, got {e}:\n{source}");
    }
}

#[test]
fn an_integer_is_not_an_element() {
    assert_rejects(
        "## Main\nLet x: Element be 3.\nShow 1.\n",
        "Int assigned to an Element binding",
    );
}

#[test]
fn an_integer_is_not_an_event() {
    assert_rejects(
        "## Main\nLet e: Event be 0.\nShow 1.\n",
        "Int assigned to an Event binding",
    );
}

#[test]
fn an_integer_is_not_a_view() {
    assert_rejects(
        "## Main\nLet v: View be 3.\nShow 1.\n",
        "Int assigned to a View binding",
    );
}

#[test]
fn an_element_function_cannot_return_a_number() {
    assert_rejects(
        "## To make () -> Element:\n    Return 0.\n## Main\nShow 1.\n",
        "a `-> Element` function returning an Int literal",
    );
}

#[test]
fn an_element_is_not_an_int() {
    assert_rejects(
        "## To make () -> Element:\n    Let el be createElement(\"div\").\n    Return el.\n## Main\nLet n: Int be make().\nShow n.\n",
        "an Element value bound under an Int annotation",
    );
}

#[test]
fn elements_and_events_are_distinct_from_each_other() {
    assert_rejects(
        "## To make () -> Element:\n    Let el be createElement(\"div\").\n    Return el.\n## Main\nLet e: Event be make().\nShow 1.\n",
        "an Element bound under an Event annotation",
    );
}

#[test]
fn handles_flow_through_typed_functions() {
    assert_accepts(
        "## To pass (e: Element) -> Element:\n    Return e.\n## Main\nLet el be createElement(\"div\").\nLet same be pass(el).\nShow 1.\n",
    );
    assert_accepts("## To read (e: Event) -> Text:\n    Return eventValue(e).\n## Main\nShow 1.\n");
}

/// The RUNTIME representation is unchanged: a program that moves handles through typed
/// functions still runs on the tree-walker and VM, lowers to WASM, and compiles to Rust
/// (where the handle types are the plain `i64` they always were — the Rust never mentions
/// the LOGOS type names).
#[test]
fn nominal_types_keep_the_i64_handle_representation_on_every_tier() {
    let src = "## To pass (e: Element) -> Element:\n    Return e.\n## Main\nLet el be createElement(\"div\").\nLet same be pass(el).\nShow \"ok\".\n";
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker runs a handle-typed program");
    assert_eq!(tw.output.trim(), "ok");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM runs a handle-typed program");
    assert_eq!(vm.output.trim(), "ok");
    compile_to_wasm(src).expect("a handle-typed function lowers to WASM");

    // AOT-Rust has no document, so a program that CALLS the DOM is refused there by name — see
    // `dom_tier_refusal_lock`. That refusal is about the program's effects, not about the TYPE:
    // `Element` is still a plain i64 handle in the emitted Rust, and this proves it on a program
    // that declares the type without touching a browser. Splitting the two claims is the point —
    // the type-representation guarantee must not depend on a tier being willing to run the DOM.
    let dom_free = "## To pass (e: Element) -> Element:\n    Return e.\n## Main\nShow \"ok\".\n";
    let rust = compile_to_rust(dom_free).expect("a handle-typed function compiles to Rust");
    assert!(
        compile_to_rust(src).is_err(),
        "a program that CALLS the DOM must be refused by AOT-Rust, which has no document to build"
    );
    for type_position in [
        ": Element",
        "-> Element",
        "<Element>",
        ": Event",
        "-> Event",
        "<Event>",
    ] {
        assert!(!rust.contains(type_position), "the AOT Rust carries plain i64 handles, never the LOGOS type name `{type_position}`:\n{rust}");
    }
    assert!(
        rust.contains("fn pass(e: i64) -> i64"),
        "the handle-typed function signature lowers to i64:\n{rust}"
    );
}
