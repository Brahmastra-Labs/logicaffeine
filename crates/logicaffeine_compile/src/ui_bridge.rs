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
use logicaffeine_proof::{DerivationTree, ProofExpr};

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
        LogicExpr::SpeechAct { performer, act_type, content } => {
            AstNode::with_children(
                &format!(
                    "{}!{}",
                    interner.resolve(*act_type),
                    interner.resolve(*performer)
                ),
                "speech_act",
                vec![expr_to_ast_node(content, interner)],
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
        let raw = compile_forest_with_options(input, CompileOptions { format: OutputFormat::SimpleFOL, pragmatic: false });
        let mut seen = HashSet::new();
        raw.into_iter().filter(|r| seen.insert(r.clone())).collect()
    };

    // Generate Kripke readings with explicit world quantification
    let kripke_readings = compile_forest_with_options(input, CompileOptions { format: OutputFormat::Kripke, pragmatic: false });

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
    /// True iff the derivation was certified AND kernel type-checked.
    /// A derivation alone (`derivation.is_some()`) never implies this.
    pub verified: bool,
    /// Where verification broke (certification or type-check), if it did.
    pub verification_error: Option<String>,
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
                verified: false,
                verification_error: None,
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
                verified: false,
                verification_error: None,
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

    let (derivation, verified, verification_error) =
        if matches!(theorem.strategy, ast::theorem::ProofStrategy::Auto) {
            let outcome = logicaffeine_proof::verify::prove_certify_check(&premises, &goal);
            (outcome.derivation, outcome.verified, outcome.verification_error)
        } else {
            (None, false, None)
        };

    TheoremCompileResult {
        name: theorem.name.clone(),
        premises,
        goal: Some(goal),
        goal_string: Some(goal_string),
        derivation,
        verified,
        verification_error,
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

    let type_env = crate::analysis::types::TypeEnv::infer_program(&stmts, &interner, &codegen_registry);
    let rust_code = crate::codegen::codegen_program(&stmts, &codegen_registry, &codegen_policies, &interner, &type_env);
    Ok(rust_code)
}

// ═══════════════════════════════════════════════════════════════════
// Interpreter (async)
// ═══════════════════════════════════════════════════════════════════

/// Interpret LOGOS imperative code and return output lines.
///
/// Same engine dispatch as [`interpret_for_ui_sync`]: the bytecode VM runs
/// synchronous programs (with the tree-walker as the debug shadow oracle);
/// programs needing async (file I/O, sleep, mount) run on the tree-walker's
/// async executor.
pub async fn interpret_for_ui(input: &str) -> InterpreterResult {
    // The synchronous dispatcher covers every non-async program; for async
    // ones it uses block_on, which would nest inside this future — so handle
    // the async case here with a real await.
    let needs_async = with_parsed_program(input, |parsed, _| match parsed {
        Ok((stmts, _, _)) => crate::interpreter::needs_async(stmts),
        Err(_) => false,
    });
    if !needs_async {
        return interpret_for_ui_sync(input);
    }

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

/// The execution front-end shared by every engine: lex → MWE → discovery →
/// parse. The tree-walker and the bytecode VM MUST both come through here so
/// they see the identical token stream, type registry, and policies — a
/// differential test that parses differently per engine is comparing two
/// different programs.
///
/// Parsed statements borrow stack-local arenas, so they are handed to the
/// closure rather than returned. A parse failure is delivered as
/// `Err(socratic advice text)` — also identical for both engines.
pub fn with_parsed_program<R>(
    input: &str,
    f: impl for<'a> FnOnce(
        Result<
            (
                &'a [logicaffeine_language::ast::stmt::Stmt<'a>],
                &'a logicaffeine_language::analysis::TypeRegistry,
                logicaffeine_language::analysis::PolicyRegistry,
            ),
            String,
        >,
        &'a Interner,
    ) -> R,
) -> R {
    use logicaffeine_language::ast::stmt::{Expr, Stmt, TypeExpr};

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
    let type_registry_for_engines = type_registry.clone();
    let parsed = {
        let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
        parser.parse_program()
    };

    match parsed {
        Ok(stmts) => f(Ok((&stmts, &type_registry_for_engines, policy_registry)), &interner),
        Err(e) => {
            let advice = socratic_explanation(&e, &interner);
            f(Err(advice), &interner)
        }
    }
}

/// Interpret LOGOS imperative code synchronously (no async runtime needed).
///
/// The bytecode VM is the LIVE engine for the synchronous path (which is also
/// the browser/WASM path). The tree-walker remains fully wired as: the async
/// path (file I/O, sleep, mount), the shadow oracle in debug builds (every
/// program the corpus runs is differentially checked against it), and the
/// fallback for anything the VM compiler rejects.
pub fn interpret_for_ui_sync(input: &str) -> InterpreterResult {
    with_parsed_program(input, |parsed, interner| match parsed {
        Ok((stmts, type_registry, policies)) => {
            if crate::interpreter::needs_async(stmts) {
                return run_treewalker(stmts, type_registry, policies, interner, true);
            }
            match crate::vm::Compiler::compile_with_types(stmts, interner, Some(type_registry)) {
                Ok(program) => {
                    let mut vm = crate::vm::Vm::new(&program).with_policy_ctx(&policies, interner);
                    if let Some(tier) = crate::vm::installed_native_tier() {
                        vm = vm.with_native_tier(tier);
                    }
                    let error = vm.run().err();
                    let result = InterpreterResult { lines: vm.into_lines(), error };

                    // Debug-build shadow oracle: the SAME program runs on the
                    // tree-walker and the full outcome must match — this turns
                    // the entire existing test corpus into a differential
                    // suite. (Skipped on wasm to keep dev builds light.)
                    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
                    {
                        let shadow = run_treewalker(
                            stmts,
                            type_registry,
                            policies.clone(),
                            interner,
                            false,
                        );
                        assert_eq!(
                            (&result.lines, &result.error),
                            (&shadow.lines, &shadow.error),
                            "VM diverged from the tree-walker oracle for:\n{input}"
                        );
                    }
                    result
                }
                // The VM compiler rejects only constructs outside the parser's
                // reach; run them on the tree-walker rather than failing.
                Err(_) => run_treewalker(stmts, type_registry, policies, interner, false),
            }
        }
        Err(advice) => InterpreterResult { lines: vec![], error: Some(advice) },
    })
}

/// Run a parsed program on the TREE-WALKER (the oracle engine). `force_async`
/// selects the async executor; otherwise the sync path is used.
pub(crate) fn run_treewalker<'a>(
    stmts: &'a [logicaffeine_language::ast::stmt::Stmt<'a>],
    type_registry: &logicaffeine_language::analysis::TypeRegistry,
    policies: logicaffeine_language::analysis::PolicyRegistry,
    interner: &'a Interner,
    force_async: bool,
) -> InterpreterResult {
    let mut interp = crate::interpreter::Interpreter::new(interner)
        .with_type_registry(type_registry)
        .with_policies(policies);
    let run_result = if force_async {
        futures::executor::block_on(interp.run(stmts))
    } else {
        interp.run_sync(stmts)
    };
    match run_result {
        Ok(()) => InterpreterResult { lines: interp.output, error: None },
        Err(e) => InterpreterResult { lines: interp.output, error: Some(e) },
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
    let (proof_exprs, goal_expr) = theorem_proof_exprs(input)?;

    // === STEPS 3-6: Prove → certify → type-check (the one canonical pipeline) ===
    let outcome = logicaffeine_proof::verify::prove_certify_check(&proof_exprs, &goal_expr);
    if outcome.verified {
        Ok((
            outcome
                .proof_term
                .expect("a verified outcome always carries a proof term"),
            outcome.kernel_ctx,
        ))
    } else {
        Err(ParseError {
            kind: logicaffeine_language::error::ParseErrorKind::Custom(
                outcome
                    .verification_error
                    .unwrap_or_else(|| "Theorem verification failed".to_string()),
            ),
            span: logicaffeine_language::token::Span::default(),
        })
    }
}

/// Ask the Z3 oracle whether a theorem's premises semantically entail its goal.
///
/// Same `## Theorem:` block format as [`verify_theorem`], but the answer is an
/// [`SmtVerdict`](logicaffeine_proof::oracle::SmtVerdict) over the standard
/// translation (modal frame axioms, similarity relations, lattice axioms) —
/// **not** a kernel-certified proof. Use this for entailment questions the
/// monotonic kernel cannot express (modality, counterfactuals, mereology).
/// Assemble the lexicon-derived [`SmtTheory`](logicaffeine_proof::oracle::SmtTheory)
/// for a theorem: every mentioned predicate the lexicon tags as a MASS noun
/// becomes a cumulative (Link-lattice) predicate.
#[cfg(feature = "verification")]
fn smt_theory_for(
    premises: &[ProofExpr],
    goal: Option<&ProofExpr>,
) -> logicaffeine_proof::oracle::SmtTheory {
    let mut exprs: Vec<ProofExpr> = premises.to_vec();
    if let Some(g) = goal {
        exprs.push(g.clone());
    }
    let cumulative_predicates = logicaffeine_proof::oracle::predicate_names(&exprs)
        .into_iter()
        .filter(|name| logicaffeine_language::lexicon::is_mass_noun(name))
        .collect();
    logicaffeine_proof::oracle::SmtTheory {
        cumulative_predicates,
    }
}

#[cfg(feature = "verification")]
pub fn check_theorem_smt(
    input: &str,
) -> Result<logicaffeine_proof::oracle::SmtVerdict, ParseError> {
    let (proof_exprs, goal_expr) = theorem_proof_exprs(input)?;
    let theory = smt_theory_for(&proof_exprs, Some(&goal_expr));
    Ok(logicaffeine_proof::oracle::oracle_entails_with_theory(
        &proof_exprs,
        &goal_expr,
        &theory,
    ))
}

/// Ask the Z3 oracle whether a theorem's premises are jointly satisfiable
/// (the goal is parsed but ignored).
///
/// Every non-entailment claim in the test suite pairs with this check so an
/// inconsistent premise set cannot fake a `NotEntailed` via vacuity.
#[cfg(feature = "verification")]
pub fn check_theorem_premises_consistent(
    input: &str,
) -> Result<logicaffeine_proof::oracle::SmtConsistency, ParseError> {
    let (proof_exprs, _goal) = theorem_proof_exprs(input)?;
    let theory = smt_theory_for(&proof_exprs, None);
    Ok(logicaffeine_proof::oracle::oracle_consistent_with_theory(
        &proof_exprs,
        &theory,
    ))
}

/// Ask the defeasible (non-monotonic) layer whether a theorem's premises
/// defeasibly entail its goal: generics and implicatures license cancellable
/// inferences via circumscription over per-rule abnormality predicates.
///
/// The verdict is **not kernel-certified** and is strictly weaker than
/// classical entailment — a defeated default reads as `NotEntailed` while the
/// premise set stays [`SmtConsistency::Consistent`].
#[cfg(feature = "verification")]
pub fn check_theorem_defeasible(
    input: &str,
) -> Result<logicaffeine_proof::oracle::SmtVerdict, ParseError> {
    let (proof_exprs, goal, defaults) = theorem_problem(input, true)?;
    let theory = smt_theory_for(&proof_exprs, Some(&goal));
    Ok(crate::defeasible::defeasible_entails(
        &proof_exprs,
        &goal,
        &defaults,
        &theory,
    ))
}

/// Consistency under the defeasible layer: defaults defeated by exceptions
/// must NOT read as contradictions.
#[cfg(feature = "verification")]
pub fn check_theorem_defeasible_consistent(
    input: &str,
) -> Result<logicaffeine_proof::oracle::SmtConsistency, ParseError> {
    let (proof_exprs, _goal, defaults) = theorem_problem(input, true)?;
    let theory = smt_theory_for(&proof_exprs, None);
    Ok(crate::defeasible::defeasible_consistent(
        &proof_exprs,
        &defaults,
        &theory,
    ))
}

/// Parse a `## Theorem:` block and convert its premises and goal to
/// [`ProofExpr`]s — the shared front half of [`verify_theorem`] and
/// [`check_theorem_smt`].
fn theorem_proof_exprs(input: &str) -> Result<(Vec<ProofExpr>, ProofExpr), ParseError> {
    let (premises, goal, _defaults) = theorem_problem(input, false)?;
    Ok((premises, goal))
}

/// [`theorem_proof_exprs`] with optional DEFEASIBLE conversion: premises keep
/// their generics/implicatures as abnormality-guarded defaults (returned for
/// the circumscription pass); the goal always converts strictly.
fn theorem_problem(
    input: &str,
    defeasible: bool,
) -> Result<
    (
        Vec<ProofExpr>,
        ProofExpr,
        Vec<logicaffeine_language::proof_convert::DefaultRule>,
    ),
    ParseError,
> {
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
    if defeasible {
        // The defeasible layer reasons over the pragmatic channel too:
        // scalar implicatures become guarded defaults.
        parser.set_pragmatic_mode(true);
    }
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

    // === STEP 2: Convert premises and goal to ProofExpr ===
    let mut defaults = Vec::new();
    let proof_exprs: Vec<ProofExpr> = theorem
        .premises
        .iter()
        .map(|premise| {
            if defeasible {
                logicaffeine_language::proof_convert::logic_expr_to_proof_expr_defeasible(
                    premise,
                    &interner,
                    &mut defaults,
                )
            } else {
                logic_expr_to_proof_expr(premise, &interner)
            }
        })
        .collect();
    let goal_expr = logic_expr_to_proof_expr(theorem.goal, &interner);

    Ok((proof_exprs, goal_expr, defaults))
}
