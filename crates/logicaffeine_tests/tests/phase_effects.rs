mod common;
#[path = "common/thread_cpu.rs"]
mod thread_cpu;

// =============================================================================
// Phase 1: Core Data Structures + Pure/Read/Write
// =============================================================================

#[test]
fn effect_pure_literal() {
    // A literal expression has no effects (Pure)
    let effects = analyze("Let x be 42.");
    assert!(effects.is_pure("x"), "Literal 42 should be Pure");
}

#[test]
fn effect_pure_arithmetic() {
    // Pure arithmetic between literals has no effects
    let effects = analyze("Let x be 2 + 3 * 4.");
    assert!(effects.is_pure("x"), "2 + 3 * 4 should be Pure");
}

#[test]
fn effect_read_variable() {
    // Reading a variable produces a Read effect
    let effects = analyze("Let x be 5.\nLet y be x + 1.");
    assert!(effects.reads("y", "x"), "x + 1 should read x");
}

#[test]
fn effect_write_let() {
    // Let statement writes to the variable
    let effects = analyze_stmt("Let x be 5.");
    assert!(effects.writes("x"), "Let x should write x");
}

#[test]
fn effect_write_set() {
    // Set statement reads and writes
    let effects = analyze_stmt("Let mutable x be 5.\nSet x to x + 1.");
    assert!(effects.writes("x"), "Set x should write x");
}

#[test]
fn effect_write_push() {
    // Push writes to the collection
    let effects = analyze_stmt("Let mutable items: Seq of Int be [1, 2].\nPush 5 to items.");
    assert!(effects.writes("items"), "Push should write items");
}

// =============================================================================
// Phase 2: IO / Alloc / Unknown
// =============================================================================

#[test]
fn effect_io_show() {
    // Show statement is IO
    let effects = analyze_stmt("Let x be 5.\nShow x.");
    assert!(effects.has_io(), "Show should be IO");
}

#[test]
fn effect_alloc_new() {
    // new Seq of Int includes Alloc
    let effects = analyze("Let items be a new Seq of Int.");
    assert!(effects.allocates("items"), "new Seq should allocate");
}

#[test]
fn effect_unknown_escape() {
    // Escape block is Unknown
    let effects = analyze_stmt("Escape to Rust:\n    println!(\"raw\");");
    assert!(effects.has_unknown(), "Escape should be Unknown");
}

// =============================================================================
// Phase 3: Complete Expression Coverage
// =============================================================================

#[test]
fn effect_length_is_read() {
    let effects = analyze("Let items: Seq of Int be [1, 2].\nLet n be length of items.");
    assert!(
        effects.reads("n", "items"),
        "length of items should read items"
    );
}

#[test]
fn effect_index_is_read() {
    let effects = analyze("Let items: Seq of Int be [1, 2].\nLet x be item 1 of items.");
    assert!(
        effects.reads("x", "items"),
        "item 1 of items should read items"
    );
}

#[test]
fn effect_contains_is_read() {
    let effects = analyze("Let items: Seq of Int be [1, 2].\nLet x be items contains 5.");
    assert!(
        effects.reads("x", "items"),
        "items contains 5 should read items"
    );
}

// =============================================================================
// Phase 4: Function-Level Effects + Fixed-Point
// =============================================================================

#[test]
fn effect_pure_function() {
    let effects = analyze_fn("## To double (x: Int) -> Int:\n    Return x * 2.");
    assert!(effects.fn_is_pure("double"), "double should be Pure");
}

#[test]
fn effect_io_function() {
    let effects = analyze_fn("## To greet (name: Text):\n    Show name.");
    assert!(effects.fn_has_io("greet"), "greet should have IO");
}

#[test]
fn effect_transitive_pure() {
    let effects = analyze_fn(
        "## To double (x: Int) -> Int:\n    Return x * 2.\n\n## To quad (x: Int) -> Int:\n    Return double(double(x))."
    );
    assert!(
        effects.fn_is_pure("quad"),
        "quad calling pure double should be Pure"
    );
}

#[test]
fn effect_transitive_io() {
    let effects = analyze_fn(
        "## To greet (msg: Text):\n    Show msg.\n\n## To greetAll (msg: Text):\n    Call greet with msg."
    );
    assert!(
        effects.fn_has_io("greetAll"),
        "greetAll calling greet should have IO"
    );
}

// =============================================================================
// Phase 5: Complete Statement Coverage
// =============================================================================

#[test]
fn effect_check_never_eliminated() {
    // SecurityCheck is non-eliminable
    let effects = analyze_stmt("Check that user is admin.");
    assert!(
        effects.has_security_check(),
        "Check should be SecurityCheck"
    );
}

// =============================================================================
// Phase 6: Edge Cases + Negative Tests
// =============================================================================

#[test]
fn effect_if_conservative() {
    // If branches joined conservatively
    let effects = analyze_stmt(
        "Let mutable x be 0.\nIf true:\n    Set x to 1.\n    Show x.\nOtherwise:\n    Set x to 2.",
    );
    assert!(
        effects.has_io(),
        "If with Show in one branch should have IO (conservative join)"
    );
}

#[test]
fn effect_while_may_diverge() {
    let effects = analyze_stmt("While true:\n    Show 1.");
    assert!(effects.may_diverge(), "While true should may_diverge");
}

// =============================================================================
// Test Infrastructure
// =============================================================================

use logicaffeine_compile::optimize::effects::EffectEnv;

struct TestEffects {
    env: EffectEnv,
}

impl TestEffects {
    fn is_pure(&self, _var: &str) -> bool {
        // Check if the expression bound to var has no effects
        self.env.is_binding_pure(_var)
    }

    fn reads(&self, _var: &str, _read_var: &str) -> bool {
        self.env.binding_reads(_var, _read_var)
    }

    fn writes(&self, _var: &str) -> bool {
        self.env.has_write_to(_var)
    }

    fn allocates(&self, _var: &str) -> bool {
        self.env.binding_allocates(_var)
    }

    fn has_io(&self) -> bool {
        self.env.has_io()
    }

    fn has_unknown(&self) -> bool {
        self.env.has_unknown()
    }

    fn has_security_check(&self) -> bool {
        self.env.has_security_check()
    }

    fn may_diverge(&self) -> bool {
        self.env.may_diverge()
    }

    fn fn_is_pure(&self, _fn_name: &str) -> bool {
        self.env.function_is_pure(_fn_name)
    }

    fn fn_has_io(&self, _fn_name: &str) -> bool {
        self.env.function_has_io(_fn_name)
    }
}

fn analyze(source: &str) -> TestEffects {
    let full_source = format!("## Main\n{}", source);
    let env = EffectEnv::analyze_source(&full_source).expect("should parse");
    TestEffects { env }
}

fn analyze_stmt(source: &str) -> TestEffects {
    let full_source = format!("## Main\n{}", source);
    let env = EffectEnv::analyze_source(&full_source).expect("should parse");
    TestEffects { env }
}

fn analyze_fn(source: &str) -> TestEffects {
    let full_source = format!("{}\n\n## Main\nShow 1.", source);
    let env = EffectEnv::analyze_source(&full_source).expect("should parse");
    TestEffects { env }
}

/// `n` functions, each calling the next with its argument; the last `last_body` (one statement over `x`).
fn call_chain(n: usize, last_body: &str) -> String {
    let mut out = String::new();
    for i in 1..n {
        out.push_str(&format!("## To f{i} (x: Int):\n    Call f{} with x.\n\n", i + 1));
    }
    out.push_str(&format!("## To f{n} (x: Int):\n    {last_body}\n"));
    out
}

#[test]
fn a_deep_call_chain_carries_its_last_callees_effect_to_its_first_caller() {
    let effects = analyze_fn(&call_chain(200, "Show x."));
    for i in [1, 2, 100, 199, 200] {
        assert!(effects.fn_has_io(&format!("f{i}")), "f{i} reaches the `Show` at the end of the chain");
    }
}

#[test]
fn a_pure_call_chain_stays_pure() {
    let effects = analyze_fn(&call_chain(200, "Let y be x * 2."));
    for i in [1, 100, 200] {
        assert!(effects.fn_is_pure(&format!("f{i}")), "f{i} calls nothing that acts");
    }
}

#[test]
fn every_function_of_a_recursive_component_and_its_callers_carry_its_effects() {
    let effects = analyze_fn(
        "## To ping (n: Int):\n    If n is greater than 0:\n        Call pong with n - 1.\n\n## To pong (n: Int):\n    Show n.\n    Call ping with n.\n\n## To start (n: Int):\n    Call ping with n.",
    );
    for f in ["ping", "pong", "start"] {
        assert!(effects.fn_has_io(f), "{f} reaches pong's `Show` through the recursion");
        assert!(!effects.fn_is_pure(f), "{f} acts and may not return");
    }
}

/// The CPU the effect analysis alone takes over the parsed chain of `n` functions.
fn analysis_cpu(n: usize) -> std::time::Duration {
    use logicaffeine_base::{Arena, Interner, Symbol};
    use logicaffeine_language::analysis::{DiscoveryPass, TypeRegistry};
    use logicaffeine_language::arena_ctx::AstContext;
    use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
    use logicaffeine_language::drs::WorldState;
    use logicaffeine_language::{Lexer, Parser};
    let source = format!("{}\n\n## Main\nShow 1.", call_chain(n, "Show x."));
    let mut interner = Interner::new();
    let mut world_state = WorldState::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();
    let np_arena: Arena<NounPhrase> = Arena::new();
    let sym_arena: Arena<Symbol> = Arena::new();
    let role_arena: Arena<(ThematicRole, Term)> = Arena::new();
    let pp_arena: Arena<&LogicExpr> = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let iexpr_arena: Arena<Expr> = Arena::new();
    let type_arena: Arena<TypeExpr> = Arena::new();
    let ctx = AstContext::with_types(
        &expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena, &stmt_arena, &iexpr_arena, &type_arena,
    );
    let tokens = Lexer::new(&source, &mut interner).tokenize();
    let registry: TypeRegistry = DiscoveryPass::new(&tokens, &mut interner).run_full().types;
    let stmts = Parser::new(tokens, &mut world_state, &mut interner, ctx, registry).parse_program().expect("the chain parses");
    let start = thread_cpu::thread_cpu_time();
    let env = EffectEnv::from_stmts(&stmts, &interner);
    let spent = thread_cpu::thread_cpu_time() - start;
    assert!(env.function_has_io("f1"), "the chain's first function reaches its `Show`");
    spent
}

/// The analysis is one pass over the call graph's components, so its cost follows the call graph's size: the chain four
/// times as long costs under eight times the CPU, where a propagation round per level of the chain made it sixteen.
#[test]
fn the_effect_analysis_grows_linearly_with_a_call_chain() {
    let short = analysis_cpu(500);
    let long = analysis_cpu(2000);
    let ratio = long.as_secs_f64() / short.as_secs_f64().max(0.000_001);
    assert!(ratio < 8.0, "the analysis of 2000 functions took {long:?}, of 500 {short:?}: {ratio:.1}x for 4x the chain");
}
