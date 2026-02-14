//! UI bridge for web interface integration.
//!
//! This module provides high-level, UI-friendly wrappers around the compilation
//! pipeline, returning structured results suitable for display in the browser.
//!
//! # Key Functions
//!
//! | Function | Purpose |
//! |----------|---------|
//! | [`compile_for_ui`] | Compile LOGOS to FOL (UI format) |
//! | [`compile_for_proof`] | Compile and search for proofs |
//! | [`compile_theorem_for_ui`] | Compile theorems with derivation trees |
//! | [`verify_theorem`] | Verify a theorem is provable |
//! | [`interpret_for_ui`] | Run imperative code and return output |
//! | [`generate_rust_code`] | Generate Rust source (requires `codegen` feature) |
//!
//! # Result Types
//!
//! All functions return serializable result types:
//! - [`CompileResult`] - Tokens, FOL, AST, errors
//! - [`ProofCompileResult`] - FOL with proof search results
//! - [`TheoremCompileResult`] - Theorem verification with derivation tree
//! - [`InterpreterResult`] - Program output lines and errors
//!
//! # Token Categories
//!
//! The [`TokenCategory`] enum provides syntactic highlighting categories:
//! Quantifier, Noun, Verb, Adjective, Connective, Determiner, etc.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use logicaffeine_base::{Arena, Interner};
use logicaffeine_language::{
    ast::{self, LogicExpr, Term},
    analysis::DiscoveryPass,
    arena_ctx::AstContext,
    compile::{compile_forest, compile_forest_with_options},
    drs,
    error::socratic_explanation,
    lexer::Lexer,
    mwe,
    parser::Parser,
    pragmatics,
    registry::SymbolRegistry,
    semantics,
    token::TokenType,
    CompileOptions, OutputFormat, ParseError,
};
use logicaffeine_proof::{BackwardChainer, DerivationTree, ProofExpr};

// Re-export interpreter result from our interpreter module
pub use crate::interpreter::InterpreterResult;

// ═══════════════════════════════════════════════════════════════════
// Token Visualization
// ═══════════════════════════════════════════════════════════════════

/// Syntactic category of a token for UI highlighting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TokenCategory {
    /// Universal/existential quantifiers: every, some, no, most
    Quantifier,
    /// Common nouns: dog, cat, person
    Noun,
    /// Main verbs: runs, loves, gives
    Verb,
    /// Adjective modifiers: tall, happy, red
    Adjective,
    /// Logical connectives: and, or, not, if
    Connective,
    /// Articles and determiners: a, an, the
    Determiner,
    /// Prepositional words: in, on, to, with
    Preposition,
    /// Personal and relative pronouns: he, she, who
    Pronoun,
    /// Modal auxiliaries: can, must, might
    Modal,
    /// Sentence-ending punctuation
    Punctuation,
    /// Proper names (capitalized)
    Proper,
    /// Uncategorized tokens
    Other,
}

/// Token information for UI display with position and category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    /// Byte offset of token start in source string.
    pub start: usize,
    /// Byte offset of token end (exclusive) in source string.
    pub end: usize,
    /// The actual text of the token.
    pub text: String,
    /// Syntactic category for highlighting.
    pub category: TokenCategory,
}

fn categorize_token(kind: &TokenType, _interner: &Interner) -> TokenCategory {
    match kind {
        TokenType::All | TokenType::Some | TokenType::No | TokenType::Any
        | TokenType::Most | TokenType::Few | TokenType::Many
        | TokenType::Cardinal(_) | TokenType::AtLeast(_) | TokenType::AtMost(_) => TokenCategory::Quantifier,
        TokenType::Noun(_) => TokenCategory::Noun,
        TokenType::Verb { .. } => TokenCategory::Verb,
        TokenType::Adjective(_) | TokenType::NonIntersectiveAdjective(_) => TokenCategory::Adjective,
        TokenType::And | TokenType::Or | TokenType::Not | TokenType::If | TokenType::Then
        | TokenType::Iff | TokenType::Because => TokenCategory::Connective,
        TokenType::Article(_) => TokenCategory::Determiner,
        TokenType::Preposition(_) => TokenCategory::Preposition,
        TokenType::Pronoun { .. } => TokenCategory::Pronoun,
        TokenType::Must | TokenType::Can | TokenType::Should | TokenType::Shall
        | TokenType::Would | TokenType::Could | TokenType::May | TokenType::Cannot
        | TokenType::Might => TokenCategory::Modal,
        TokenType::Period | TokenType::Comma => TokenCategory::Punctuation,
        TokenType::ProperName(_) => TokenCategory::Proper,
        _ => TokenCategory::Other,
    }
}

/// Tokenizes input text and returns token information for UI display.
///
/// Each token includes its byte position, text content, and syntactic
/// category for syntax highlighting in the browser interface.
pub fn tokenize_for_ui(input: &str) -> Vec<TokenInfo> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    tokens.iter().map(|t| TokenInfo {
        start: t.span.start,
        end: t.span.end,
        text: input[t.span.start..t.span.end].to_string(),
        category: categorize_token(&t.kind, &interner),
    }).collect()
}

// ═══════════════════════════════════════════════════════════════════
// AST Visualization
// ═══════════════════════════════════════════════════════════════════

/// AST node for tree visualization in the UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AstNode {
    /// Display label for this node (e.g., "∀x", "Run(x)").
    pub label: String,
    /// Node type for styling (e.g., "quantifier", "predicate", "connective").
    pub node_type: String,
    /// Child nodes in the AST.
    pub children: Vec<AstNode>,
}

impl AstNode {
    pub fn leaf(label: &str, node_type: &str) -> Self {
        AstNode {
            label: label.to_string(),
            node_type: node_type.to_string(),
            children: Vec::new(),
        }
    }

    pub fn with_children(label: &str, node_type: &str, children: Vec<AstNode>) -> Self {
        AstNode {
            label: label.to_string(),
            node_type: node_type.to_string(),
            children,
        }
    }
}

/// Converts a logic expression to an AST node for tree visualization.
///
/// Recursively builds a tree structure with labeled nodes suitable for
/// rendering in the UI. Each node includes a display label, node type
/// for styling, and child nodes.
pub fn expr_to_ast_node(expr: &LogicExpr, interner: &Interner) -> AstNode {
    match expr {
        LogicExpr::Predicate { name, args, .. } => {
            let name_str = interner.resolve(*name);
            let arg_nodes: Vec<AstNode> = args.iter()
                .map(|t| term_to_ast_node(t, interner))
                .collect();
            AstNode::with_children(
                &format!("{}({})", name_str, args.len()),
                "predicate",
                arg_nodes,
            )
        }
        LogicExpr::Quantifier { kind, variable, body, .. } => {
            let var_str = interner.resolve(*variable);
            let symbol = match kind {
                ast::QuantifierKind::Universal => "∀",
                ast::QuantifierKind::Existential => "∃",
                ast::QuantifierKind::Most => "MOST",
                ast::QuantifierKind::Few => "FEW",
                ast::QuantifierKind::Many => "MANY",
                ast::QuantifierKind::Cardinal(n) => return AstNode::with_children(
                    &format!("∃={}{}", n, var_str),
                    "quantifier",
                    vec![expr_to_ast_node(body, interner)],
                ),
                ast::QuantifierKind::AtLeast(n) => return AstNode::with_children(
                    &format!("∃≥{}{}", n, var_str),
                    "quantifier",
                    vec![expr_to_ast_node(body, interner)],
                ),
                ast::QuantifierKind::AtMost(n) => return AstNode::with_children(
                    &format!("∃≤{}{}", n, var_str),
                    "quantifier",
                    vec![expr_to_ast_node(body, interner)],
                ),
                ast::QuantifierKind::Generic => "GEN",
            };
            AstNode::with_children(
                &format!("{}{}", symbol, var_str),
                "quantifier",
                vec![expr_to_ast_node(body, interner)],
            )
        }
        LogicExpr::BinaryOp { left, op, right } => {
            let op_str = match op {
                TokenType::And => "∧",
                TokenType::Or => "∨",
                TokenType::If | TokenType::Then => "→",
                TokenType::Iff => "↔",
                _ => "?",
            };
            AstNode::with_children(
                op_str,
                "binary_op",
                vec![
                    expr_to_ast_node(left, interner),
                    expr_to_ast_node(right, interner),
                ],
            )
        }
        LogicExpr::UnaryOp { op, operand } => {
            let op_str = match op {
                TokenType::Not => "¬",
                _ => "?",
            };
            AstNode::with_children(
                op_str,
                "unary_op",
                vec![expr_to_ast_node(operand, interner)],
            )
        }
        LogicExpr::Identity { left, right } => {
            AstNode::with_children(
                "=",
                "identity",
                vec![
                    term_to_ast_node(left, interner),
                    term_to_ast_node(right, interner),
                ],
            )
        }
        LogicExpr::Modal { vector, operand } => {
            AstNode::with_children(
                &format!("□{:?}", vector.domain),
                "modal",
                vec![expr_to_ast_node(operand, interner)],
            )
        }
        LogicExpr::Lambda { variable, body } => {
            let var_str = interner.resolve(*variable);
            AstNode::with_children(
                &format!("λ{}", var_str),
                "lambda",
                vec![expr_to_ast_node(body, interner)],
            )
        }
        _ => AstNode::leaf(&format!("{:?}", expr), "other"),
    }
}

fn term_to_ast_node(term: &Term, interner: &Interner) -> AstNode {
    match term {
        Term::Constant(sym) => AstNode::leaf(interner.resolve(*sym), "constant"),
        Term::Variable(sym) => AstNode::leaf(interner.resolve(*sym), "variable"),
        Term::Function(name, args) => {
            let name_str = interner.resolve(*name);
            let arg_nodes: Vec<AstNode> = args.iter()
                .map(|t| term_to_ast_node(t, interner))
                .collect();
            AstNode::with_children(&format!("{}()", name_str), "function", arg_nodes)
        }
        Term::Group(terms) => {
            let term_nodes: Vec<AstNode> = terms.iter()
                .map(|t| term_to_ast_node(t, interner))
                .collect();
            AstNode::with_children("⊕", "group", term_nodes)
        }
        _ => AstNode::leaf(&format!("{:?}", term), "term"),
    }
}

// ═══════════════════════════════════════════════════════════════════
// Compilation Results
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Result of compiling English input to FOL with UI metadata.
pub struct CompileResult {
    /// Primary FOL output (Unicode format), if compilation succeeded.
    pub logic: Option<String>,
    /// Simplified FOL with modals stripped (for verification).
    pub simple_logic: Option<String>,
    /// Kripke semantics output with explicit world quantification.
    pub kripke_logic: Option<String>,
    /// AST tree representation for visualization.
    pub ast: Option<AstNode>,
    /// All scope readings in Unicode format.
    pub readings: Vec<String>,
    /// All scope readings in simplified format.
    pub simple_readings: Vec<String>,
    /// All scope readings in Kripke format.
    pub kripke_readings: Vec<String>,
    /// Tokenization with categories for syntax highlighting.
    pub tokens: Vec<TokenInfo>,
    /// Parse/compile error message, if any.
    pub error: Option<String>,
}

/// Compile English input to FOL with full UI metadata.
pub fn compile_for_ui(input: &str) -> CompileResult {
    let tokens = tokenize_for_ui(input);
    let readings = compile_forest(input);

    // Generate Simple readings (modals stripped) - deduplicated
    let simple_readings: Vec<String> = {
        let raw = compile_forest_with_options(input, CompileOptions { format: OutputFormat::SimpleFOL });
        let mut seen = HashSet::new();
        raw.into_iter().filter(|r| seen.insert(r.clone())).collect()
    };

    // Generate Kripke readings with explicit world quantification
    let kripke_readings = compile_forest_with_options(input, CompileOptions { format: OutputFormat::Kripke });

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let lex_tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let lex_tokens = mwe::apply_mwe_pipeline(lex_tokens, &mwe_trie, &mut interner);

    // Pass 1: Discovery
    let type_registry = {
        let mut discovery = DiscoveryPass::new(&lex_tokens, &mut interner);
        discovery.run()
    };

    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();

    let ctx = AstContext::new(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    // Pass 2: Parse
    let mut world_state = drs::WorldState::new();
    let mut parser = Parser::new(lex_tokens, &mut world_state, &mut interner, ctx, type_registry);

    match parser.parse() {
        Ok(ast) => {
            let ast = semantics::apply_axioms(ast, ctx.exprs, ctx.terms, &mut interner);
            let ast = pragmatics::apply_pragmatics(ast, ctx.exprs, &interner);
            let ast_node = expr_to_ast_node(ast, &interner);
            let mut registry = SymbolRegistry::new();
            let logic = ast.transpile_discourse(&mut registry, &interner, OutputFormat::Unicode);
            let simple_logic = ast.transpile_discourse(&mut registry, &interner, OutputFormat::SimpleFOL);

            let kripke_ast = semantics::apply_kripke_lowering(ast, ctx.exprs, ctx.terms, &mut interner);
            let kripke_logic = kripke_ast.transpile_discourse(&mut registry, &interner, OutputFormat::Kripke);

            CompileResult {
                logic: Some(logic),
                simple_logic: Some(simple_logic),
                kripke_logic: Some(kripke_logic),
                ast: Some(ast_node),
                readings,
                simple_readings,
                kripke_readings,
                tokens,
                error: None,
            }
        }
        Err(e) => {
            let advice = socratic_explanation(&e, &interner);
            CompileResult {
                logic: None,
                simple_logic: None,
                kripke_logic: None,
                ast: None,
                readings: Vec::new(),
                simple_readings: Vec::new(),
                kripke_readings: Vec::new(),
                tokens,
                error: Some(advice),
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Proof Integration
// ═══════════════════════════════════════════════════════════════════

/// Result of compiling English to a proof expression.
///
/// Used by the proof engine to search for derivations. Contains the
/// converted proof expression, the simplified FOL string representation,
/// and any compilation error.
#[derive(Debug, Clone)]
pub struct ProofCompileResult {
    /// The compiled proof expression, or `None` on error.
    pub proof_expr: Option<ProofExpr>,
    /// Simplified FOL string representation for display.
    pub logic_string: Option<String>,
    /// Error message if compilation failed.
    pub error: Option<String>,
}

/// Compile English input to ProofExpr for the proof engine.
pub fn compile_for_proof(input: &str) -> ProofCompileResult {
    use logicaffeine_language::proof_convert::logic_expr_to_proof_expr;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let lex_tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let lex_tokens = mwe::apply_mwe_pipeline(lex_tokens, &mwe_trie, &mut interner);

    let type_registry = {
        let mut discovery = DiscoveryPass::new(&lex_tokens, &mut interner);
        discovery.run()
    };

    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();

    let ctx = AstContext::new(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut world_state = drs::WorldState::new();
    let mut parser = Parser::new(lex_tokens, &mut world_state, &mut interner, ctx, type_registry);

    match parser.parse() {
        Ok(ast) => {
            let ast = semantics::apply_axioms(ast, ctx.exprs, ctx.terms, &mut interner);
            let ast = pragmatics::apply_pragmatics(ast, ctx.exprs, &interner);

            let mut registry = SymbolRegistry::new();
            let logic_string = ast.transpile(&mut registry, &interner, OutputFormat::SimpleFOL);
            let proof_expr = logic_expr_to_proof_expr(ast, &interner);

            ProofCompileResult {
                proof_expr: Some(proof_expr),
                logic_string: Some(logic_string),
                error: None,
            }
        }
        Err(e) => {
            let advice = socratic_explanation(&e, &interner);
            ProofCompileResult {
                proof_expr: None,
                logic_string: None,
                error: Some(advice),
            }
        }
    }
}

/// Result of compiling and verifying a theorem block.
///
/// Contains the parsed theorem structure (name, premises, goal) along
/// with the proof derivation tree if automatic proof search succeeded.
#[derive(Debug, Clone)]
pub struct TheoremCompileResult {
    /// The theorem's declared name.
    pub name: String,
    /// Compiled premise expressions (axioms).
    pub premises: Vec<ProofExpr>,
    /// The goal expression to prove, or `None` on parse error.
    pub goal: Option<ProofExpr>,
    /// Simplified FOL string of the goal for display.
    pub goal_string: Option<String>,
    /// Derivation tree from backward chaining, if proof found.
    pub derivation: Option<DerivationTree>,
    /// Error message if compilation or proof failed.
    pub error: Option<String>,
}

/// Compile a theorem block for UI display.
pub fn compile_theorem_for_ui(input: &str) -> TheoremCompileResult {
    use logicaffeine_language::proof_convert::logic_expr_to_proof_expr;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    let type_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        discovery.run()
    };

    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();

    let ctx = AstContext::new(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut world_state = drs::WorldState::new();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);

    let statements = match parser.parse_program() {
        Ok(stmts) => stmts,
        Err(e) => {
            return TheoremCompileResult {
                name: String::new(),
                premises: Vec::new(),
                goal: None,
                goal_string: None,
                derivation: None,
                error: Some(format!("Parse error: {:?}", e)),
            };
        }
    };

    let theorem = match statements.iter().find_map(|stmt| {
        if let ast::Stmt::Theorem(t) = stmt {
            Some(t)
        } else {
            None
        }
    }) {
        Some(t) => t,
        None => {
            return TheoremCompileResult {
                name: String::new(),
                premises: Vec::new(),
                goal: None,
                goal_string: None,
                derivation: None,
                error: Some("No theorem block found".to_string()),
            };
        }
    };

    let premises: Vec<ProofExpr> = theorem
        .premises
        .iter()
        .map(|p| logic_expr_to_proof_expr(p, &interner))
        .collect();

    let goal = logic_expr_to_proof_expr(theorem.goal, &interner);

    let mut registry = SymbolRegistry::new();
    let goal_string = theorem.goal.transpile(&mut registry, &interner, OutputFormat::SimpleFOL);

    let derivation = if matches!(theorem.strategy, ast::theorem::ProofStrategy::Auto) {
        let mut engine = BackwardChainer::new();
        for premise in &premises {
            engine.add_axiom(premise.clone());
        }
        engine.prove(goal.clone()).ok()
    } else {
        None
    };

    TheoremCompileResult {
        name: theorem.name.clone(),
        premises,
        goal: Some(goal),
        goal_string: Some(goal_string),
        derivation,
        error: None,
    }
}

// ═══════════════════════════════════════════════════════════════════
// Code Generation
// ═══════════════════════════════════════════════════════════════════

/// Generate Rust code from LOGOS imperative source.
#[cfg(feature = "codegen")]
pub fn generate_rust_code(source: &str) -> Result<String, ParseError> {
    use logicaffeine_language::ast::stmt::{Stmt, Expr, TypeExpr};

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    let codegen_registry = type_registry.clone();
    let codegen_policies = policy_registry.clone();

    let mut world_state = drs::WorldState::new();
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

    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
    let stmts = parser.parse_program()?;

    let rust_code = crate::codegen::codegen_program(&stmts, &codegen_registry, &codegen_policies, &interner);
    Ok(rust_code)
}

// ═══════════════════════════════════════════════════════════════════
// Interpreter (async)
// ═══════════════════════════════════════════════════════════════════

/// Interpret LOGOS imperative code and return output lines.
pub async fn interpret_for_ui(input: &str) -> InterpreterResult {
    use logicaffeine_language::ast::stmt::{Stmt, Expr, TypeExpr};

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };

    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ctx = AstContext::with_types(
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

    let mut world_state = drs::WorldState::new();
    let type_registry_for_interp = type_registry.clone();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);

    match parser.parse_program() {
        Ok(stmts) => {
            let mut interp = crate::interpreter::Interpreter::new(&interner)
                .with_type_registry(&type_registry_for_interp)
                .with_policies(policy_registry);
            match interp.run(&stmts).await {
                Ok(()) => InterpreterResult {
                    lines: interp.output,
                    error: None,
                },
                Err(e) => InterpreterResult {
                    lines: interp.output,
                    error: Some(e),
                },
            }
        }
        Err(e) => {
            let advice = socratic_explanation(&e, &interner);
            InterpreterResult {
                lines: vec![],
                error: Some(advice),
            }
        }
    }
}

/// Interpret LOGOS imperative code synchronously (no async runtime needed).
///
/// Uses the sync execution path when the program has no async operations
/// (no file I/O, sleep, or mount). Falls back to async via block_on otherwise.
pub fn interpret_for_ui_sync(input: &str) -> InterpreterResult {
    use logicaffeine_language::ast::stmt::{Stmt, Expr, TypeExpr};

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };

    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ctx = AstContext::with_types(
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

    let mut world_state = drs::WorldState::new();
    let type_registry_for_interp = type_registry.clone();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);

    match parser.parse_program() {
        Ok(stmts) => {
            let mut interp = crate::interpreter::Interpreter::new(&interner)
                .with_type_registry(&type_registry_for_interp)
                .with_policies(policy_registry);

            if crate::interpreter::needs_async(&stmts) {
                // Fall back to async path
                use futures::executor::block_on;
                match block_on(interp.run(&stmts)) {
                    Ok(()) => InterpreterResult {
                        lines: interp.output,
                        error: None,
                    },
                    Err(e) => InterpreterResult {
                        lines: interp.output,
                        error: Some(e),
                    },
                }
            } else {
                // Use sync path — no Future allocation overhead
                match interp.run_sync(&stmts) {
                    Ok(()) => InterpreterResult {
                        lines: interp.output,
                        error: None,
                    },
                    Err(e) => InterpreterResult {
                        lines: interp.output,
                        error: Some(e),
                    },
                }
            }
        }
        Err(e) => {
            let advice = socratic_explanation(&e, &interner);
            InterpreterResult {
                lines: vec![],
                error: Some(advice),
            }
        }
    }
}

/// Interpret LOGOS imperative code with streaming output.
///
/// The `on_output` callback is called each time `Show` executes, allowing
/// real-time output display like a REPL. The callback receives the output line.
///
/// # Example
/// ```no_run
/// use std::rc::Rc;
/// use std::cell::RefCell;
/// # use logicaffeine_compile::interpret_streaming;
///
/// # fn main() {}
/// # async fn example() {
/// # let source = "## Main\nShow \"hello\".";
/// let lines = Rc::new(RefCell::new(Vec::new()));
/// let lines_clone = lines.clone();
///
/// interpret_streaming(source, Rc::new(RefCell::new(move |line: String| {
///     lines_clone.borrow_mut().push(line);
/// }))).await;
/// # }
/// ```
pub async fn interpret_streaming<F>(input: &str, on_output: std::rc::Rc<std::cell::RefCell<F>>) -> InterpreterResult
where
    F: FnMut(String) + 'static,
{
    use logicaffeine_language::ast::stmt::{Stmt, Expr, TypeExpr};
    use crate::interpreter::OutputCallback;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };

    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ctx = AstContext::with_types(
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

    let mut world_state = drs::WorldState::new();
    let type_registry_for_interp = type_registry.clone();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);

    match parser.parse_program() {
        Ok(stmts) => {
            // Create the callback wrapper that calls the user's callback
            let callback: OutputCallback = std::rc::Rc::new(std::cell::RefCell::new(move |line: String| {
                (on_output.borrow_mut())(line);
            }));

            let mut interp = crate::interpreter::Interpreter::new(&interner)
                .with_type_registry(&type_registry_for_interp)
                .with_policies(policy_registry)
                .with_output_callback(callback);

            match interp.run(&stmts).await {
                Ok(()) => InterpreterResult {
                    lines: interp.output,
                    error: None,
                },
                Err(e) => InterpreterResult {
                    lines: interp.output,
                    error: Some(e),
                },
            }
        }
        Err(e) => {
            let advice = socratic_explanation(&e, &interner);
            InterpreterResult {
                lines: vec![],
                error: Some(advice),
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Theorem Verification (Kernel-certified)
// ═══════════════════════════════════════════════════════════════════

use logicaffeine_language::ast::Stmt;
use logicaffeine_language::proof_convert::logic_expr_to_proof_expr;
use crate::kernel;

/// Phase 78: Verify a theorem with full kernel certification.
///
/// Pipeline:
/// 1. Parse theorem block
/// 2. Extract symbols and build kernel context
/// 3. Run proof engine
/// 4. Certify derivation tree to kernel term
/// 5. Type-check the term
/// 6. Return (proof_term, context)
pub fn verify_theorem(input: &str) -> Result<(kernel::Term, kernel::Context), ParseError> {
    // === STEP 1: Parse ===
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    let type_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        discovery.run()
    };

    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();

    let ctx = AstContext::new(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    let mut world_state = drs::WorldState::new();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
    let statements = parser.parse_program()?;

    let theorem = statements
        .iter()
        .find_map(|stmt| {
            if let Stmt::Theorem(t) = stmt {
                Some(t)
            } else {
                None
            }
        })
        .ok_or_else(|| ParseError {
            kind: logicaffeine_language::error::ParseErrorKind::Custom("No theorem block found in input".to_string()),
            span: logicaffeine_language::token::Span::default(),
        })?;

    // === STEP 2: Build Kernel Context ===
    let mut kernel_ctx = kernel::Context::new();
    kernel::prelude::StandardLibrary::register(&mut kernel_ctx);

    // Convert premises and goal to ProofExpr
    let mut proof_exprs: Vec<ProofExpr> = Vec::new();
    for premise in &theorem.premises {
        let proof_expr = logic_expr_to_proof_expr(premise, &interner);
        proof_exprs.push(proof_expr);
    }
    let goal_expr = logic_expr_to_proof_expr(theorem.goal, &interner);

    // Collect symbols from all expressions
    let mut collector = SymbolCollector::new();
    for expr in &proof_exprs {
        collector.collect(expr);
    }
    collector.collect(&goal_expr);

    // Register predicates: P : Entity → Prop
    for pred_name in collector.predicates() {
        register_predicate(&mut kernel_ctx, pred_name);
    }

    // Register constants: Socrates : Entity
    for const_name in collector.constants() {
        register_constant(&mut kernel_ctx, const_name);
    }

    // Register axiom hypotheses and build engine
    let mut engine = BackwardChainer::new();
    for (i, proof_expr) in proof_exprs.iter().enumerate() {
        let hyp_name = format!("h{}", i + 1);
        let hyp_type = proof_expr_to_kernel_type(proof_expr)?;
        kernel_ctx.add_declaration(&hyp_name, hyp_type);
        engine.add_axiom(proof_expr.clone());
    }

    // === STEP 3: Prove ===
    let derivation = engine.prove(goal_expr.clone()).map_err(|e| ParseError {
        kind: logicaffeine_language::error::ParseErrorKind::Custom(format!("Proof failed: {}", e)),
        span: logicaffeine_language::token::Span::default(),
    })?;

    // === STEP 4: Certify ===
    let cert_ctx = logicaffeine_proof::certifier::CertificationContext::new(&kernel_ctx);
    let proof_term = logicaffeine_proof::certifier::certify(&derivation, &cert_ctx).map_err(|e| ParseError {
        kind: logicaffeine_language::error::ParseErrorKind::Custom(format!("Certification failed: {}", e)),
        span: logicaffeine_language::token::Span::default(),
    })?;

    // === STEP 5: Type-Check ===
    let _ = kernel::infer_type(&kernel_ctx, &proof_term).map_err(|e| ParseError {
        kind: logicaffeine_language::error::ParseErrorKind::Custom(format!("Type check failed: {}", e)),
        span: logicaffeine_language::token::Span::default(),
    })?;

    // === STEP 6: Return ===
    Ok((proof_term, kernel_ctx))
}

/// Collects predicates and constants from ProofExpr
struct SymbolCollector {
    predicates: HashSet<String>,
    constants: HashSet<String>,
}

impl SymbolCollector {
    fn new() -> Self {
        SymbolCollector {
            predicates: HashSet::new(),
            constants: HashSet::new(),
        }
    }

    fn collect(&mut self, expr: &ProofExpr) {
        match expr {
            ProofExpr::Predicate { name, args, .. } => {
                self.predicates.insert(name.clone());
                for arg in args {
                    self.collect_term(arg);
                }
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => {
                self.collect(l);
                self.collect(r);
            }
            ProofExpr::Not(inner) => {
                self.collect(inner);
            }
            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
                self.collect(body);
            }
            ProofExpr::Atom(_) => {
                // Atoms are propositional constants, not FOL predicates
            }
            ProofExpr::Identity(l, r) => {
                // Collect constants from identity terms
                self.collect_term(l);
                self.collect_term(r);
            }
            _ => {}
        }
    }

    fn collect_term(&mut self, term: &logicaffeine_proof::ProofTerm) {
        match term {
            logicaffeine_proof::ProofTerm::Constant(name) => {
                // Only add if it looks like a proper name (capitalized)
                if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    self.constants.insert(name.clone());
                }
            }
            logicaffeine_proof::ProofTerm::Function(name, args) => {
                self.predicates.insert(name.clone());
                for arg in args {
                    self.collect_term(arg);
                }
            }
            _ => {}
        }
    }

    fn predicates(&self) -> impl Iterator<Item = &String> {
        self.predicates.iter()
    }

    fn constants(&self) -> impl Iterator<Item = &String> {
        self.constants.iter()
    }
}

/// Register a predicate in the kernel context.
/// P : Entity → Prop
fn register_predicate(ctx: &mut kernel::Context, name: &str) {
    // Don't re-register if already present
    if ctx.get_global(name).is_some() {
        return;
    }

    let pred_type = kernel::Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(kernel::Term::Global("Entity".to_string())),
        body_type: Box::new(kernel::Term::Sort(kernel::Universe::Prop)),
    };
    ctx.add_declaration(name, pred_type);
}

/// Register a constant in the kernel context.
/// Socrates : Entity
fn register_constant(ctx: &mut kernel::Context, name: &str) {
    // Don't re-register if already present
    if ctx.get_global(name).is_some() {
        return;
    }

    ctx.add_declaration(name, kernel::Term::Global("Entity".to_string()));
}

/// Convert ProofExpr (engine) to kernel Term (type)
fn proof_expr_to_kernel_type(expr: &ProofExpr) -> Result<kernel::Term, ParseError> {
    match expr {
        ProofExpr::Predicate { name, args, .. } => {
            // P(a, b, c) → ((P a) b) c
            let mut term = kernel::Term::Global(name.clone());
            for arg in args {
                let arg_term = proof_term_to_kernel_term(arg)?;
                term = kernel::Term::App(Box::new(term), Box::new(arg_term));
            }
            Ok(term)
        }
        ProofExpr::ForAll { variable, body } => {
            // ∀x.P(x) → Π(x:Entity). P(x)
            let body_type = proof_expr_to_kernel_type(body)?;
            Ok(kernel::Term::Pi {
                param: variable.clone(),
                param_type: Box::new(kernel::Term::Global("Entity".to_string())),
                body_type: Box::new(body_type),
            })
        }
        ProofExpr::Implies(ante, cons) => {
            // P → Q → Π(_:P). Q
            let ante_type = proof_expr_to_kernel_type(ante)?;
            let cons_type = proof_expr_to_kernel_type(cons)?;
            Ok(kernel::Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(ante_type),
                body_type: Box::new(cons_type),
            })
        }
        ProofExpr::And(l, r) => {
            // P ∧ Q → And P Q
            let l_type = proof_expr_to_kernel_type(l)?;
            let r_type = proof_expr_to_kernel_type(r)?;
            Ok(kernel::Term::App(
                Box::new(kernel::Term::App(
                    Box::new(kernel::Term::Global("And".to_string())),
                    Box::new(l_type),
                )),
                Box::new(r_type),
            ))
        }
        ProofExpr::Or(l, r) => {
            // P ∨ Q → Or P Q
            let l_type = proof_expr_to_kernel_type(l)?;
            let r_type = proof_expr_to_kernel_type(r)?;
            Ok(kernel::Term::App(
                Box::new(kernel::Term::App(
                    Box::new(kernel::Term::Global("Or".to_string())),
                    Box::new(l_type),
                )),
                Box::new(r_type),
            ))
        }
        ProofExpr::Atom(name) => {
            // Propositional atoms: P → P (as a global)
            Ok(kernel::Term::Global(name.clone()))
        }
        ProofExpr::Identity(l, r) => {
            // a = b → Eq Entity a b
            let l_term = proof_term_to_kernel_term(l)?;
            let r_term = proof_term_to_kernel_term(r)?;
            Ok(kernel::Term::App(
                Box::new(kernel::Term::App(
                    Box::new(kernel::Term::App(
                        Box::new(kernel::Term::Global("Eq".to_string())),
                        Box::new(kernel::Term::Global("Entity".to_string())),
                    )),
                    Box::new(l_term),
                )),
                Box::new(r_term),
            ))
        }
        _ => Err(ParseError {
            kind: logicaffeine_language::error::ParseErrorKind::Custom(format!(
                "Unsupported ProofExpr for kernel type: {:?}",
                expr
            )),
            span: logicaffeine_language::token::Span::default(),
        }),
    }
}

/// Convert ProofTerm to kernel Term
fn proof_term_to_kernel_term(term: &logicaffeine_proof::ProofTerm) -> Result<kernel::Term, ParseError> {
    match term {
        logicaffeine_proof::ProofTerm::Constant(name) => Ok(kernel::Term::Global(name.clone())),
        logicaffeine_proof::ProofTerm::Variable(name) => Ok(kernel::Term::Var(name.clone())),
        logicaffeine_proof::ProofTerm::Function(name, args) => {
            let mut t = kernel::Term::Global(name.clone());
            for arg in args {
                let arg_term = proof_term_to_kernel_term(arg)?;
                t = kernel::Term::App(Box::new(t), Box::new(arg_term));
            }
            Ok(t)
        }
        _ => Err(ParseError {
            kind: logicaffeine_language::error::ParseErrorKind::Custom(format!(
                "Unsupported ProofTerm for kernel: {:?}",
                term
            )),
            span: logicaffeine_language::token::Span::default(),
        }),
    }
}
