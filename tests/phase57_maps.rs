//! Phase 57: Polymorphic Indexing Tests
//!
//! RED: These tests define the spec for Map support.

use logos::codegen::{codegen_expr, codegen_stmt, RefinementContext, empty_var_caps};
use logos::ast::{Expr, Stmt, Literal};
use logos::intern::Interner;
use logos::arena::Arena;
use logos::analysis::TypeRegistry;
use std::collections::HashSet;

fn empty_lww_fields() -> HashSet<(String, String)> {
    HashSet::new()
}

fn empty_registry(interner: &mut Interner) -> TypeRegistry {
    TypeRegistry::with_primitives(interner)
}

// ============================================================
// Test 1: Map get with string key generates trait call
// ============================================================
#[test]
fn codegen_map_get_string_key() {
    let mut interner = Interner::new();
    let synced_vars = HashSet::new();
    let arena: Arena<Expr> = Arena::new();

    // registry["iron"]
    let registry = interner.intern("registry");
    let key = interner.intern("iron");

    let collection = arena.alloc(Expr::Identifier(registry));
    let index = arena.alloc(Expr::Literal(Literal::Text(key)));

    let expr = Expr::Index { collection, index };
    let result = codegen_expr(&expr, &interner, &synced_vars);

    // Should use trait method, NOT logos_index
    assert_eq!(result, "LogosIndex::logos_get(&registry, String::from(\"iron\"))");
}

// ============================================================
// Test 2: Map set with string key generates trait call
// ============================================================
#[test]
fn codegen_map_set_string_key() {
    let mut interner = Interner::new();
    let mut synced_vars = HashSet::new();
    let arena: Arena<Expr> = Arena::new();

    // Set item "iron" of registry to plate.
    let registry = interner.intern("registry");
    let key = interner.intern("iron");
    let plate = interner.intern("plate");

    let collection = arena.alloc(Expr::Identifier(registry));
    let index = arena.alloc(Expr::Literal(Literal::Text(key)));
    let value = arena.alloc(Expr::Identifier(plate));

    let stmt = Stmt::SetIndex { collection, index, value };
    let mut ctx = RefinementContext::new();
    let async_fns = HashSet::new();
    let pipe_vars = HashSet::new();
    let type_registry = TypeRegistry::with_primitives(&mut interner);
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::new(), &mut ctx,
                               &empty_lww_fields(), &mut synced_vars, &empty_var_caps(),
                               &async_fns, &pipe_vars, &HashSet::new(), &type_registry);

    // Should use trait method, NOT hardcoded index math
    assert_eq!(result, "LogosIndexMut::logos_set(&mut registry, String::from(\"iron\"), plate);\n");
}

// ============================================================
// Test 3: Vec get with integer preserves 1-based semantics
// ============================================================
#[test]
fn codegen_vec_get_integer_key() {
    let mut interner = Interner::new();
    let synced_vars = HashSet::new();
    let arena: Arena<Expr> = Arena::new();

    // item 1 of items
    let items = interner.intern("items");

    let collection = arena.alloc(Expr::Identifier(items));
    let index = arena.alloc(Expr::Literal(Literal::Number(1)));

    let expr = Expr::Index { collection, index };
    let result = codegen_expr(&expr, &interner, &synced_vars);

    // Should use trait method (trait impl handles 1-based conversion)
    assert_eq!(result, "LogosIndex::logos_get(&items, 1)");
}

// ============================================================
// Test 4: Vec set with integer preserves 1-based semantics
// ============================================================
#[test]
fn codegen_vec_set_integer_key() {
    let mut interner = Interner::new();
    let mut synced_vars = HashSet::new();
    let arena: Arena<Expr> = Arena::new();

    // Set item 2 of items to val.
    let items = interner.intern("items");
    let val = interner.intern("val");

    let collection = arena.alloc(Expr::Identifier(items));
    let index = arena.alloc(Expr::Literal(Literal::Number(2)));
    let value = arena.alloc(Expr::Identifier(val));

    let stmt = Stmt::SetIndex { collection, index, value };
    let mut ctx = RefinementContext::new();
    let async_fns = HashSet::new();
    let pipe_vars = HashSet::new();
    let type_registry = TypeRegistry::with_primitives(&mut interner);
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::new(), &mut ctx,
                               &empty_lww_fields(), &mut synced_vars, &empty_var_caps(),
                               &async_fns, &pipe_vars, &HashSet::new(), &type_registry);

    // Should use trait method (trait impl handles 1-based conversion)
    assert_eq!(result, "LogosIndexMut::logos_set(&mut items, 2, val);\n");
}
