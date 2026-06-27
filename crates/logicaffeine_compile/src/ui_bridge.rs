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
use logicaffeine_proof::{DerivationTree, ProofExpr, ProofTerm};

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
    /// For a wh-question goal ("Who is in Florida?"), the certified witness(es); `None`
    /// when the goal is a closed proposition.
    pub answer: Option<Vec<String>>,
    /// For a recognized finite-domain grid, the cells the certified prover could fill —
    /// attached whenever the premises take the grid form, no flag required.
    pub grid: Option<SolvedGrid>,
    /// Error message if compilation or proof failed.
    pub error: Option<String>,
}

/// A solved (or partially solved) logic grid: the row entities plus one column per
/// declared category closure. Every filled cell is an entailment the no-Z3 certified
/// prover (CDCL+RUP / kernel) closed, so the whole table is independently checkable.
#[derive(Debug, Clone, PartialEq)]
pub struct SolvedGrid {
    /// Display header for the identity column — the row sort (e.g. "Trip").
    pub row_label: String,
    /// The row entities in declaration order (e.g. Alpha, Beta, Gamma, Delta).
    pub rows: Vec<String>,
    /// One column per category closure (year, state, friend, activity, …).
    pub columns: Vec<GridColumn>,
}

/// One category column of a [`SolvedGrid`].
#[derive(Debug, Clone, PartialEq)]
pub struct GridColumn {
    /// Category header (the value sort, e.g. "Year"), best-effort from the declarations.
    pub label: String,
    /// The category's domain values in closure order.
    pub values: Vec<String>,
    /// The determined value for each row (parallel to [`SolvedGrid::rows`]), or `None`
    /// where the prover could not force a unique value.
    pub cells: Vec<Option<String>>,
}

/// A parsed theorem block: premises and goal as owned (arena-free) `ProofExpr`s, the goal's
/// display string, and whether the strategy is `Auto`. Factored out of
/// [`compile_theorem_for_ui`] so the grounded grid problem can feed every trust tier and the
/// benchmark, not just the kernel path.
struct ParsedTheorem {
    name: String,
    premises: Vec<ProofExpr>,
    goal: ProofExpr,
    goal_string: String,
    is_auto: bool,
}

fn parse_theorem(input: &str) -> Result<ParsedTheorem, String> {
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

    let statements = parser
        .parse_program()
        .map_err(|e| format!("Parse error: {:?}", e))?;

    let theorem = statements
        .iter()
        .find_map(|stmt| if let ast::Stmt::Theorem(t) = stmt { Some(t) } else { None })
        .ok_or_else(|| "No theorem block found".to_string())?;

    let premises: Vec<ProofExpr> = theorem
        .premises
        .iter()
        .map(|p| logic_expr_to_proof_expr(p, &interner))
        .collect();
    let goal = logic_expr_to_proof_expr(theorem.goal, &interner);

    let mut registry = SymbolRegistry::new();
    let goal_string = theorem.goal.transpile(&mut registry, &interner, OutputFormat::SimpleFOL);

    Ok(ParsedTheorem {
        name: theorem.name.clone(),
        premises,
        goal,
        goal_string,
        is_auto: matches!(theorem.strategy, ast::theorem::ProofStrategy::Auto),
    })
}

/// The GROUNDED, quantifier-free grid problem `(solver_input, goal)` for an `Auto` grid
/// theorem — the EXACT problem [`compile_theorem_for_ui`] solves, exposed so the trust tiers
/// (Untrusted CDCL / RUP / Kernel) and the benchmark all run the same input. `None` if the
/// theorem is not an `Auto` finite-domain grid.
pub fn grounded_grid_problem(input: &str) -> Option<(Vec<ProofExpr>, ProofExpr)> {
    let parsed = parse_theorem(input).ok()?;
    if !parsed.is_auto || !looks_like_grid(&parsed.premises) {
        return None;
    }
    let solver_input = prepare_premises_opts(&parsed.premises, true);
    let g = erase_tense(&parsed.goal);
    Some((solver_input, g))
}

/// Compile a theorem block for UI display.
pub fn compile_theorem_for_ui(input: &str) -> TheoremCompileResult {
    let parsed = match parse_theorem(input) {
        Ok(p) => p,
        Err(e) => {
            return TheoremCompileResult {
                name: String::new(),
                premises: Vec::new(),
                goal: None,
                goal_string: None,
                derivation: None,
                verified: false,
                verification_error: None,
                answer: None,
                grid: None,
                error: Some(e),
            };
        }
    };
    let ParsedTheorem { name, premises, goal, goal_string, is_auto } = parsed;

    // The easter egg: when the premises take the grid form (declared bijection +
    // disjunctive closures), fill every cell the certified prover can force — no flag,
    // the structure alone triggers it. Attached to every grid result so the studio can
    // render the solved table alongside the headline goal.
    let grid = if is_auto && looks_like_grid(&premises) {
        solve_grid_from_premises(&premises, input)
    } else {
        None
    };

    let (derivation, verified, verification_error, answer) =
        if is_auto {
            // A wh-question goal ("Who is in Florida?") is an open ∃-form; the closed-goal
            // prover cannot discharge it and grinds a deep, futile search. Recognize the
            // form and ANSWER it by enumerating witnesses through the same certified no-Z3
            // path the puzzle uses — each candidate proved (or refuted) in ~1ms.
            if let ProofExpr::Exists { variable, body } = &goal {
                let witnesses = answer_wh(&premises, variable, body);
                let verified = !witnesses.is_empty();
                let verr = (!verified)
                    .then(|| "no individual in the domain satisfies the question".to_string());
                (None, verified, verr, Some(witnesses))
            } else {
            // A finite-domain GRID (a declared closure / "exactly one") is proved over
            // its GROUNDED, quantifier-free form, so the certified kernel can close it
            // in the browser (no Z3). An OPEN syllogism (Socrates: "every man is
            // mortal") is left untouched, keeping its `UniversalInst`/`ModusPonens`
            // trace. Both remain kernel-certified.
            let outcome = if looks_like_grid(&premises) {
                let g = erase_tense(&goal);
                // The incremental certified grid solver (trail propagation + DPLL) emits
                // a DerivationTree the kernel re-checks; it is faster and avoids the
                // general prover's re-saturation, and is the only path given the
                // FUNCTIONALITY exclusions. It is strictly additive: only a VERIFIED
                // solver tree is used, otherwise we fall back to the bounded backward
                // chainer (without functionality), so the solver can never regress.
                let solver_input = prepare_premises_opts(&premises, true);
                let solve = || {
                    let trace = std::env::var("LOGOS_TRACE").is_ok();
                    let t_solve = trace.then(std::time::Instant::now);
                    let tree = logicaffeine_proof::grid_solver::grid_prove(&solver_input, &g);
                    if let Some(t_solve) = t_solve {
                        let n = tree.as_ref().map(count_tree_nodes).unwrap_or(0);
                        eprintln!("[grid] solve+emit {:.2?} ({} tree nodes)", t_solve.elapsed(), n);
                    }
                    let t_cert = trace.then(std::time::Instant::now);
                    let solved = tree
                        .map(|tree| logicaffeine_proof::verify::check_derivation(&solver_input, &g, tree))
                        .filter(|vp| vp.verified);
                    if let Some(t_cert) = t_cert {
                        eprintln!("[grid] kernel-certify {:.2?} (verified={})", t_cert.elapsed(), solved.is_some());
                    }
                    match solved {
                        Some(vp) => vp,
                        None => {
                            // The grid solver produced no certified tree. Before the
                            // expensive bounded chainer, ask the fast certified RUP tier
                            // whether the goal is even entailed — a non-entailed goal is
                            // decided in ~1ms instead of grinding a deep, futile search.
                            // (RUP runs the SAME `solver_input`, so a NotEntailed verdict is
                            // sound: the goal is not entailed by the premises.)
                            if matches!(
                                logicaffeine_proof::rup::entails_certified(&solver_input, &g),
                                Some(logicaffeine_proof::rup::Verdict::NotEntailed)
                            ) {
                                return logicaffeine_proof::verify::VerifiedProof {
                                    derivation: None,
                                    proof_term: None,
                                    kernel_ctx: Default::default(),
                                    verified: false,
                                    verification_error: Some(
                                        "goal is not entailed (RUP-certified)".to_string(),
                                    ),
                                };
                            }
                            let grounded = prepare_premises(&premises);
                            logicaffeine_proof::verify::prove_certify_check_bounded(&grounded, &g, 60)
                        }
                    }
                };
                // A full multi-category grid's certified derivation recurses deeply
                // (case-analysis over every interdependent clue), so run the solve on a
                // generous stack — well past the default 8 MiB — on native targets.
                #[cfg(not(target_arch = "wasm32"))]
                {
                    std::thread::scope(|s| {
                        std::thread::Builder::new()
                            .stack_size(512 * 1024 * 1024)
                            .spawn_scoped(s, solve)
                            .expect("spawn grid-solve thread")
                            .join()
                            .expect("grid-solve thread panicked")
                    })
                }
                #[cfg(target_arch = "wasm32")]
                {
                    solve()
                }
            } else {
                logicaffeine_proof::verify::prove_certify_check(&premises, &goal)
            };
            (outcome.derivation, outcome.verified, outcome.verification_error, None)
            }
        } else {
            (None, false, None, None)
        };

    TheoremCompileResult {
        name,
        premises,
        goal: Some(goal),
        goal_string: Some(goal_string),
        derivation,
        verified,
        verification_error,
        answer,
        grid,
        error: None,
    }
}

fn count_tree_nodes(t: &logicaffeine_proof::DerivationTree) -> usize {
    1 + t.premises.iter().map(count_tree_nodes).sum::<usize>()
}

// ═══════════════════════════════════════════════════════════════════
// Code Generation
// ═══════════════════════════════════════════════════════════════════

/// Generate Rust code from LOGOS source — the Studio Code path. Mixed-document
/// aware: a source interleaving imperative code with Coq-style math
/// (`Definition`/`## Theorem:`/…) extracts the math into a bundled `mod proven` that
/// the imperative half calls into. Pure imperative source is a no-op partition.
#[cfg(feature = "codegen")]
pub fn generate_rust_code(source: &str) -> Result<String, ParseError> {
    let (imperative_src, math_src) = partition_mixed(source);
    let proven = math_src.as_deref().and_then(mixed_proven_module);
    generate_rust_code_with_proven(&imperative_src, proven.as_deref())
}

/// [`generate_rust_code`] with an already-extracted proven module bundled in.
#[cfg(feature = "codegen")]
pub fn generate_rust_code_with_proven(source: &str, proven: Option<&str>) -> Result<String, ParseError> {
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
    let rust_code = crate::codegen::codegen_program_with_proven(&stmts, &codegen_registry, &codegen_policies, &interner, &type_env, &crate::optimization::OptimizationConfig::from_env(), "proven", proven);
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
/// The first Send/escape-analysis violation in `stmts`, packaged as a ready
/// rejection — or `None` if the program respects the message-passing + CRDT
/// memory model. This is the static concurrency gate (Phase 4): a program whose
/// concurrent branches share non-CRDT mutable state is refused *before* any tier
/// runs, so neither the VM nor the tree-walker executes a data race.
fn send_escape_rejection(stmts: &[logicaffeine_language::ast::stmt::Stmt]) -> Option<InterpreterResult> {
    crate::concurrency::check_send_escape(stmts)
        .first()
        .map(|d| InterpreterResult { lines: Vec::new(), error: Some(d.message.clone()) })
}

pub async fn interpret_for_ui(input: &str) -> InterpreterResult {
    interpret_for_ui_with_args(input, &[]).await
}

/// Like [`interpret_for_ui`], but supplies the program's argument vector to the
/// `args()` system native. `program_args` is the full argv (index 0 is the
/// program name), matching the compiled binary's `env::args()`.
pub async fn interpret_for_ui_with_args(
    input: &str,
    program_args: &[String],
) -> InterpreterResult {
    // The synchronous dispatcher covers every non-async program; for async
    // ones it uses block_on, which would nest inside this future — so handle
    // the async case here with a real await.
    let needs_async = with_parsed_program(input, |parsed, _| match parsed {
        Ok((stmts, _, _)) => crate::interpreter::needs_async(stmts),
        Err(_) => false,
    });
    if !needs_async {
        return interpret_for_ui_sync_with_args(input, program_args);
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
            if let Some(rejection) = send_escape_rejection(&stmts) {
                return rejection;
            }
            // A program that uses channels/tasks AND needs async (e.g. networking) must run
            // on the cooperative scheduler — `interp.run` alone installs none, so a channel op
            // would panic "outside a scheduler context". The async drive loop services both
            // the channel ops and the network awaits (the latter over the host reactor). A
            // pure-channel program (no async) never reaches here; it takes the sync path.
            if crate::concurrency::uses_scheduler(&stmts) {
                return run_program_concurrent_streaming(
                    &stmts,
                    &type_registry_for_interp,
                    policy_registry,
                    &interner,
                    program_args,
                    None,
                    None,
                    None,
                    0,
                )
                .await;
            }
            let mut interp = crate::interpreter::Interpreter::new(&interner)
                .with_type_registry(&type_registry_for_interp)
                .with_policies(policy_registry)
                .with_program_args(program_args.to_vec());
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

    // Phase 10: auto-prepend the stdlib modules the program references (no-op when
    // it uses no stdlib vocabulary, so such programs stay byte-identical).
    let prelude_src = crate::loader::apply_prelude(input);
    let input = prelude_src.as_ref();

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
    let (parsed, opt_flags) = {
        let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
        let stmts = parser.parse_program();
        let flags = parser.program_opt_flags();
        (stmts, flags)
    };

    match parsed {
        Ok(stmts) => {
            // Strength-reduce accumulator recursion to a constant-stack `while`
            // loop so the VM and tree-walker match the AOT (and never hit the
            // call-depth limit on `Return n + f(n-1)`-shaped recursion).
            // Type-directed division: rewrite `Divide → ExactDivide` in Rational
            // contexts (default stays floor), then accumulator-TCO the resolved AST.
            // The constant Rational fold honors the `Opt::Comptime` toggle (env +
            // in-source `## No comptime`), consistent with the AOT and tiered paths.
            let mut run_cfg =
                crate::optimization::OptimizationConfig::from_env().merged(&opt_flags);
            run_cfg.normalize();
            let resolved = crate::resolve_division::resolve_divisions(
                &stmts,
                &stmt_arena,
                &imperative_expr_arena,
                &interner,
                run_cfg.is_on(crate::optimization::Opt::Comptime),
            );
            let pre = resolved.unwrap_or(stmts.as_slice());
            match crate::tail_call::rewrite_accumulators(
                pre,
                &stmt_arena,
                &imperative_expr_arena,
                &mut interner,
            ) {
                Some(rw) => f(Ok((rw, &type_registry_for_engines, policy_registry)), &interner),
                None => f(Ok((pre, &type_registry_for_engines, policy_registry)), &interner),
            }
        }
        Err(e) => {
            let advice = socratic_explanation(&e, &interner);
            f(Err(advice), &interner)
        }
    }
}

/// [`with_parsed_program`], but the statements pass through the RUN-PATH
/// optimizer (EXODIA D1: the Futamura residual — PE, GVN, LICM, closed-form,
/// deforestation, interval analysis, DCE) before reaching the closure. The
/// live `largo run` engines consume this; the differential seams stay on the
/// raw variant so optimizer correctness is itself differentially gated
/// (optimized-VM vs raw-tree-walker).
pub fn with_optimized_program<R>(
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
    // The run-path upfront tier is the env-configured mode (default Eager → T3, i.e.
    // today's behavior, bit-for-bit). `LOGOS_TIER_PROFILE`/`LOGOS_FORCE_TIER` select
    // Tiered/Baseline for the benchmark A/B and the T_optimize measurement (§12.1).
    let tier = crate::optimization::HotswapConfig::from_env().run_tier();
    with_optimized_program_tiered(input, tier, f)
}

/// [`with_optimized_program`] gated by an explicit hotness `tier` (HOTSWAP §4): the
/// statements pass through [`crate::optimize::optimize_for_run_tiered`] at `tier`.
/// `Tier::T3` reproduces `with_optimized_program` exactly; `Tier::T0` skips the
/// optimizer (the baseline tier). The accumulator→loop TCO rewrite is a LANGUAGE
/// SEMANTIC, not an optimization, so it runs at every tier — deep recursion stays
/// constant-stack regardless of the optimization tier.
pub fn with_optimized_program_tiered<R>(
    input: &str,
    tier: crate::optimization::Tier,
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

    // Phase 10: auto-prepend the stdlib modules the program references (no-op when
    // it uses no stdlib vocabulary, so such programs stay byte-identical).
    let prelude_src = crate::loader::apply_prelude(input);
    let input = prelude_src.as_ref();

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
    let (parsed, opt_flags, tier_pins) = {
        let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
        let stmts = parser.parse_program();
        let flags = parser.program_opt_flags();
        let pins = parser.program_tier_pins();
        (stmts, flags, pins)
    };

    match parsed {
        Ok(stmts) => {
            // Respect file-level `## No <opt>` decorators on the run path too, so
            // optimization toggles behave consistently with the AOT compile.
            let mut run_cfg =
                crate::optimization::OptimizationConfig::from_env().merged(&opt_flags);
            run_cfg.normalize();
            // Tiered-optimizer pins: ambient env (`LOGOS_TIER_PIN`) overlaid by the
            // program's in-source `## Tier` decorators (the decorator wins).
            let mut hotswap = crate::optimization::HotswapConfig::from_env();
            hotswap.pins.overlay(&tier_pins);
            // Type-directed division (Divide → ExactDivide in Rational contexts) runs
            // BEFORE optimization so the optimizer sees ExactDivide (opaque, never
            // floor-folded), not a bare `7 / 2` it would fold to `3`.
            let resolved = crate::resolve_division::resolve_divisions(
                &stmts,
                &stmt_arena,
                &imperative_expr_arena,
                &interner,
                run_cfg.is_on(crate::optimization::Opt::Comptime),
            );
            let pre: Vec<_> = match resolved {
                Some(rw) => rw.to_vec(),
                None => stmts,
            };
            let optimized = crate::optimize::optimize_for_run_tiered(
                pre,
                &imperative_expr_arena,
                &stmt_arena,
                &mut interner,
                &run_cfg,
                &hotswap,
                tier,
            );
            // Accumulator recursion → constant-stack loop (matches the AOT and
            // the raw-parse engines).
            match crate::tail_call::rewrite_accumulators(
                &optimized,
                &stmt_arena,
                &imperative_expr_arena,
                &mut interner,
            ) {
                Some(rw) => f(Ok((rw, &type_registry_for_engines, policy_registry)), &interner),
                None => f(Ok((&optimized, &type_registry_for_engines, policy_registry)), &interner),
            }
        }
        Err(e) => {
            let advice = socratic_explanation(&e, &interner);
            f(Err(advice), &interner)
        }
    }
}

/// Like [`with_optimized_program`], but the program runs through the
/// ARCHITECT pipeline (`optimize_program_v2`): equality saturation with
/// kernel-certified rewrites in place of GVN + LICM + closed-form. The
/// differential gates in `phase_exodia_architect.rs` hold this path to the
/// raw tree-walker's outcomes.
pub fn with_v2_optimized_program<R>(
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

    // Phase 10: auto-prepend the stdlib modules the program references (no-op when
    // it uses no stdlib vocabulary, so such programs stay byte-identical).
    let prelude_src = crate::loader::apply_prelude(input);
    let input = prelude_src.as_ref();

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
        Ok(stmts) => {
            let optimized = crate::optimize::optimize_program(
                stmts,
                &imperative_expr_arena,
                &stmt_arena,
                &mut interner,
                &crate::optimization::OptimizationConfig::from_env(),
            );
            f(Ok((&optimized, &type_registry_for_engines, policy_registry)), &interner)
        }
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
    interpret_for_ui_sync_with_args(input, &[])
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// Compiled-native functions to install into the next VM run on this thread
    /// (HOTSWAP §Axis-3). `largo run` loads a `.logos-native` bundle into this; the VM
    /// seam below drains it and installs each via `install_aot_native`, so a `native`-
    /// annotated function dispatches to `rustc -O3` machine code from its first call.
    /// Empty for every other caller — no AOT, behavior unchanged.
    static PENDING_AOT: std::cell::RefCell<Vec<(String, Box<dyn crate::vm::NativeFn>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Queue compiled-native functions for the next VM run on this thread (HOTSWAP §Axis-3).
/// Consumed by the next `interpret_for_ui_*` VM run.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_pending_aot_natives(natives: Vec<(String, Box<dyn crate::vm::NativeFn>)>) {
    PENDING_AOT.with(|p| *p.borrow_mut() = natives);
}

/// Drain the pending compiled-native functions and install them into `vm`, resolving
/// each by name to its function index. No-op when none are queued.
#[cfg(not(target_arch = "wasm32"))]
fn install_pending_aot_natives(
    vm: &mut crate::vm::Vm,
    program: &crate::vm::CompiledProgram,
    interner: &Interner,
) {
    let pending = PENDING_AOT.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for (name, nf) in pending {
        if let Some(fi) = program
            .fn_index
            .iter()
            .find(|(s, _)| interner.resolve(**s) == name)
            .map(|(_, i)| *i as usize)
        {
            vm.install_aot_native(fi, nf);
            if std::env::var_os("LOGOS_ENGINE_TRACE").is_some() {
                eprintln!("logos-engine: aot-native installed for '{name}'");
            }
        }
    }
}

/// Like [`interpret_for_ui_sync`], but supplies the program's argument vector
/// to the `args()` system native. `program_args` is the full argv (index 0 is
/// the program name), matching the compiled binary's `env::args()` — this is
/// what `largo run --interpret N` forwards so an args-driven `main.lg` runs the
/// same on the VM/JIT as on the native binary.
pub fn interpret_for_ui_sync_with_args(input: &str, program_args: &[String]) -> InterpreterResult {
    // Diagnostic for benchmarking: with LOGOS_ENGINE_TRACE set, report which
    // engine actually ran a program to stderr, so a silent tree-walker fallback
    // is never mistaken for the VM+JIT.
    let trace = |engine: &str| {
        if std::env::var_os("LOGOS_ENGINE_TRACE").is_some() {
            eprintln!("logos-engine: {engine}");
        }
    };
    // The LIVE path runs the Futamura residual: both the VM and the debug
    // shadow oracle receive the SAME optimized program (two engines, one
    // program — the optimizer is differentially gated elsewhere against the
    // raw tree-walker).
    with_optimized_program(input, |parsed, interner| match parsed {
        Ok((stmts, type_registry, policies)) => {
            if let Some(rejection) = send_escape_rejection(stmts) {
                return rejection;
            }
            if crate::interpreter::needs_async(stmts) || crate::concurrency::uses_scheduler(stmts) {
                trace("treewalker (async)");
                return run_treewalker(stmts, type_registry, policies, interner, true, program_args);
            }
            // Oracle range analysis (M9) over the EXACT snapshot being
            // compiled → bounds-check elimination in the JIT.
            let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
            match crate::vm::Compiler::compile_with_oracle(
                stmts,
                interner,
                Some(type_registry),
                Some(oracle),
            ) {
                Ok(program) => {
                    trace("vm+jit");
                    let mut vm = crate::vm::Vm::new(&program)
                        .with_policy_ctx(&policies, interner)
                        .with_program_args(program_args.to_vec());
                    if let Some(tier) = crate::vm::installed_native_tier() {
                        vm = vm.with_native_tier(tier);
                    }
                    // Compiled-native tier (HOTSWAP §Axis-3): install any AOT functions
                    // the run was given, so they dispatch to rustc -O3 machine code.
                    #[cfg(not(target_arch = "wasm32"))]
                    install_pending_aot_natives(&mut vm, &program, interner);
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
                            program_args,
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
                Err(_) => {
                    trace("treewalker (vm-reject)");
                    run_treewalker(stmts, type_registry, policies, interner, false, program_args)
                }
            }
        }
        Err(advice) => InterpreterResult { lines: vec![], error: Some(advice) },
    })
}

/// Baseline-tier interpret for the interactive UI (the Studio) — the cold-start
/// engine. Runs on the bytecode VM with NO run-path optimizer
/// (`with_parsed_program`, not `with_optimized_program`) and NO oracle
/// (`compile_with_types`, not `compile_with_oracle`), so there is no
/// optimize/analysis latency before execution. `largo run` and the benchmarks
/// keep [`interpret_for_ui`], which optimizes ahead of execution for peak
/// throughput. Output is identical — the VM is differentially gated against the
/// tree-walker on the SAME parsed statements (see the debug shadow-oracle assert
/// in [`interpret_for_ui_baseline_sync_with_args`]); only the startup latency
/// differs. This is the baseline half of the EXODIA tier split; promoting hot
/// code to the optimized path is a later step.
pub async fn interpret_for_ui_baseline(input: &str) -> InterpreterResult {
    interpret_for_ui_baseline_with_args(input, &[]).await
}

/// Like [`interpret_for_ui_baseline`], but supplies the program's argument
/// vector to the `args()` system native.
pub async fn interpret_for_ui_baseline_with_args(
    input: &str,
    program_args: &[String],
) -> InterpreterResult {
    // Async programs (file I/O, sleep, mount) run on the tree-walker's async
    // executor regardless of tier — the VM is sync-only — so that case is
    // identical to the optimized entry (and must `await` rather than nest a
    // `block_on`); delegate it.
    let needs_async = with_parsed_program(input, |parsed, _| match parsed {
        Ok((stmts, _, _)) => crate::interpreter::needs_async(stmts),
        Err(_) => false,
    });
    if needs_async {
        return interpret_for_ui_with_args(input, program_args).await;
    }
    interpret_for_ui_baseline_sync_with_args(input, program_args)
}

/// The synchronous baseline core: parse (UNoptimized) → bytecode VM (no oracle)
/// → run. Mirrors [`interpret_for_ui_sync_with_args`] exactly, minus
/// `optimize_for_run` and the oracle range analysis.
pub fn interpret_for_ui_baseline_sync_with_args(
    input: &str,
    program_args: &[String],
) -> InterpreterResult {
    let trace = |engine: &str| {
        if std::env::var_os("LOGOS_ENGINE_TRACE").is_some() {
            eprintln!("logos-engine: {engine}");
        }
    };
    with_parsed_program(input, |parsed, interner| match parsed {
        Ok((stmts, type_registry, policies)) => {
            if let Some(rejection) = send_escape_rejection(stmts) {
                return rejection;
            }
            if crate::interpreter::needs_async(stmts) || crate::concurrency::uses_scheduler(stmts) {
                trace("treewalker (async)");
                return run_treewalker(stmts, type_registry, policies, interner, true, program_args);
            }
            match crate::vm::Compiler::compile_with_types(stmts, interner, Some(type_registry)) {
                Ok(program) => {
                    trace("vm (baseline)");
                    let mut vm = crate::vm::Vm::new(&program)
                        .with_policy_ctx(&policies, interner)
                        .with_program_args(program_args.to_vec());
                    if let Some(tier) = crate::vm::installed_native_tier() {
                        vm = vm.with_native_tier(tier);
                    }
                    // Compiled-native tier (HOTSWAP §Axis-3): install any AOT functions
                    // the run was given, so they dispatch to rustc -O3 machine code.
                    #[cfg(not(target_arch = "wasm32"))]
                    install_pending_aot_natives(&mut vm, &program, interner);
                    let error = vm.run().err();
                    let result = InterpreterResult { lines: vm.into_lines(), error };

                    // The same debug differential net as the optimized path: the
                    // baseline VM and the tree-walker must agree on the SAME
                    // (unoptimized) statements. (Skipped on wasm to keep dev
                    // builds light.)
                    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
                    {
                        let shadow = run_treewalker(
                            stmts,
                            type_registry,
                            policies.clone(),
                            interner,
                            false,
                            program_args,
                        );
                        assert_eq!(
                            (&result.lines, &result.error),
                            (&shadow.lines, &shadow.error),
                            "baseline VM diverged from the tree-walker oracle for:\n{input}"
                        );
                    }
                    result
                }
                // The VM compiler rejects only constructs outside the parser's
                // reach; run them on the tree-walker rather than failing.
                Err(_) => {
                    trace("treewalker (vm-reject)");
                    run_treewalker(stmts, type_registry, policies, interner, false, program_args)
                }
            }
        }
        Err(advice) => InterpreterResult { lines: vec![], error: Some(advice) },
    })
}

/// Run a parsed program on the TREE-WALKER (the oracle engine). `force_async`
/// selects the async executor; otherwise the sync path is used.
/// Run a concurrent program on the deterministic scheduler: spawn the main block
/// as a task (which may spawn more), drive the scheduler to quiescence, and
/// collect the output merged through a shared callback.
fn run_program_concurrent<'a>(
    stmts: &'a [logicaffeine_language::ast::stmt::Stmt<'a>],
    type_registry: &logicaffeine_language::analysis::TypeRegistry,
    policies: logicaffeine_language::analysis::PolicyRegistry,
    interner: &'a Interner,
    program_args: &[String],
    vfs: Option<std::sync::Arc<dyn logicaffeine_system::fs::Vfs>>,
    stream: Option<crate::interpreter::OutputCallback>,
    seed: u64,
) -> InterpreterResult {
    use crate::concurrency::bridge::YieldState;
    use crate::concurrency::driver::InterpreterTask;
    use logicaffeine_runtime::{run_with_seed, RunOutcome, SchedSeed, SchedulerConfig};
    use std::cell::RefCell;
    use std::rc::Rc;

    let output_sink: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = output_sink.clone();
    // Collect every task's output for the final result, and — when streaming —
    // forward each line to the live callback as it is produced (the Studio's
    // real-time display).
    let callback: crate::interpreter::OutputCallback =
        Rc::new(RefCell::new(move |line: String| {
            if let Some(s) = &stream {
                (s.borrow_mut())(line.clone());
            }
            sink.borrow_mut().push(line);
        }));
    let err_sink: crate::concurrency::driver::ErrSink = Rc::new(RefCell::new(None));

    let mut main = crate::interpreter::Interpreter::new(interner)
        .with_type_registry(type_registry)
        .with_policies(policies)
        .with_program_args(program_args.to_vec())
        .with_output_callback(callback);
    if let Some(v) = vfs {
        main = main.with_vfs(v);
    }
    let main_ys = Rc::new(RefCell::new(YieldState::new()));
    main.install_yield_state(main_ys.clone());

    let main_fut = Box::pin(async move { main.run(stmts).await });
    let main_task = InterpreterTask::new(main_fut, main_ys, Some(err_sink.clone()));

    let (outcome, _trace) =
        run_with_seed(SchedulerConfig::default(), SchedSeed(seed), move |sched| {
            sched.spawn_main(Box::new(main_task));
        });

    let mut error = err_sink.borrow().clone();
    if error.is_none() {
        match outcome {
            RunOutcome::Deadlock => {
                error = Some("deadlock: every task is blocked waiting".to_string());
            }
            // The synchronous scheduler has no reactor; a program that needs network I/O
            // must run on the async drive loop (it routes there via `needs_async`). Reaching
            // here means it was driven on the wrong tier — surface it instead of half-running.
            RunOutcome::WaitingForIo => {
                error = Some(
                    "networking requires the async runtime; this program was run on the \
                     synchronous scheduler"
                        .to_string(),
                );
            }
            RunOutcome::Done(_) => {}
        }
    }
    let lines = output_sink.borrow().clone();
    InterpreterResult { lines, error }
}

/// A snapshot sink for the Studio's Tasks/Channels strip — invoked between scheduler
/// slices with the live task/channel state.
pub type ObserverCallback = std::rc::Rc<std::cell::RefCell<dyn FnMut(logicaffeine_runtime::SchedSnapshot)>>;

/// Yield a macrotask so the host event loop (Dioxus) can repaint between scheduler
/// slices. On wasm this is a real `setTimeout(0)`; on native there is no event loop to
/// yield to, so it returns immediately and the drive loop continues — the output is
/// identical, only the interleaving with repaints differs.
async fn yield_macrotask() {
    #[cfg(target_arch = "wasm32")]
    {
        gloo_timers::future::TimeoutFuture::new(0).await;
    }
}

/// Yield control back to the host async runtime once, so its reactor can advance a pending
/// network future before the scheduler re-polls the task that awaits it. Runtime-agnostic
/// (no tokio dependency): a future that wakes itself and returns `Pending` exactly once — the
/// same shape as `tokio::task::yield_now`, which lets the current-thread runtime poll its I/O
/// driver between turns. On wasm a `setTimeout(0)` lets the event loop fire the socket
/// callbacks. This is how a [`logicaffeine_runtime::RunOutcome::WaitingForIo`] is serviced.
async fn yield_to_reactor() {
    #[cfg(target_arch = "wasm32")]
    {
        gloo_timers::future::TimeoutFuture::new(0).await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::future::Future;
        use std::pin::Pin;
        use std::task::{Context, Poll};
        struct YieldOnce(bool);
        impl Future for YieldOnce {
            type Output = ();
            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if self.0 {
                    Poll::Ready(())
                } else {
                    self.0 = true;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        }
        YieldOnce(false).await;
    }
}

/// The browser-facing async driver for concurrent programs. Identical semantics to
/// [`run_program_concurrent`] (same deterministic scheduler, same seed, same output), but
/// it advances the scheduler in bounded slices and yields a macrotask between them so the
/// Studio repaints instead of freezing — and emits a [`logicaffeine_runtime::SchedSnapshot`]
/// to `observer` after each slice to drive the Tasks/Channels strip.
#[allow(clippy::too_many_arguments)]
async fn run_program_concurrent_streaming<'a>(
    stmts: &'a [logicaffeine_language::ast::stmt::Stmt<'a>],
    type_registry: &logicaffeine_language::analysis::TypeRegistry,
    policies: logicaffeine_language::analysis::PolicyRegistry,
    interner: &'a Interner,
    program_args: &[String],
    vfs: Option<std::sync::Arc<dyn logicaffeine_system::fs::Vfs>>,
    stream: Option<crate::interpreter::OutputCallback>,
    observer: Option<ObserverCallback>,
    seed: u64,
) -> InterpreterResult {
    use crate::concurrency::bridge::YieldState;
    use crate::concurrency::driver::InterpreterTask;
    use logicaffeine_runtime::{Chooser, RunOutcome, SchedSeed, Scheduler, SchedulerConfig};
    use std::cell::RefCell;
    use std::rc::Rc;

    // The slice budget: how many scheduler steps to run before yielding a macrotask. Large
    // enough that a quiescent program finishes in one slice; small enough that a busy one
    // still repaints. It does not affect output (the scheduler is deterministic).
    const SLICE_STEPS: usize = 256;

    let output_sink: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = output_sink.clone();
    let callback: crate::interpreter::OutputCallback = Rc::new(RefCell::new(move |line: String| {
        if let Some(s) = &stream {
            (s.borrow_mut())(line.clone());
        }
        sink.borrow_mut().push(line);
    }));
    let err_sink: crate::concurrency::driver::ErrSink = Rc::new(RefCell::new(None));

    let mut main = crate::interpreter::Interpreter::new(interner)
        .with_type_registry(type_registry)
        .with_policies(policies)
        .with_program_args(program_args.to_vec())
        .with_output_callback(callback);
    if let Some(v) = vfs {
        main = main.with_vfs(v);
    }
    let main_ys = Rc::new(RefCell::new(YieldState::new()));
    main.install_yield_state(main_ys.clone());

    let main_fut = Box::pin(async move { main.run(stmts).await });
    let main_task = InterpreterTask::new(main_fut, main_ys, Some(err_sink.clone()));

    let mut sched = Scheduler::new(SchedulerConfig::default(), Chooser::record(SchedSeed(seed)));
    sched.spawn_main(Box::new(main_task));

    let outcome = loop {
        match sched.run_slice(SLICE_STEPS) {
            Some(RunOutcome::WaitingForIo) => {
                // A task is awaiting external I/O (a network op). Let the host reactor make
                // progress, then re-poll the parked tasks — they re-drive their network
                // futures, which now observe completion.
                if let Some(ob) = &observer {
                    (ob.borrow_mut())(sched.snapshot());
                }
                yield_to_reactor().await;
                sched.wake_io();
            }
            Some(o) => break o,
            None => {
                if let Some(ob) = &observer {
                    (ob.borrow_mut())(sched.snapshot());
                }
                yield_macrotask().await;
            }
        }
    };

    let mut error = err_sink.borrow().clone();
    if error.is_none() {
        if let RunOutcome::Deadlock = outcome {
            error = Some("deadlock: every task is blocked waiting".to_string());
        }
    }
    let lines = output_sink.borrow().clone();
    InterpreterResult { lines, error }
}

/// Parse `input` and run its concurrent program on the **tree-walker** scheduler
/// under an explicit `seed` — the seeded sibling of the default
/// `run_program_concurrent` entry, used by the cross-tier seeded differential.
pub fn run_treewalker_concurrent_seeded(input: &str, seed: u64) -> InterpreterResult {
    with_parsed_program(input, |parsed, interner| match parsed {
        Ok((stmts, type_registry, policies)) => {
            run_program_concurrent(stmts, type_registry, policies, interner, &[], None, None, seed)
        }
        Err(advice) => InterpreterResult { lines: vec![], error: Some(advice) },
    })
}

/// Run a concurrent program on the **bytecode VM** under the deterministic
/// scheduler (T11): compile to opcodes, then drive the main [`Vm`] (and any
/// spawned per-task VMs) through `run_until_block` via a `VmTask`. This is the
/// VM analog of [`run_program_concurrent`]; the default routing still sends
/// concurrent programs to the tree-walker, so this is the explicit VM entry used
/// to exercise and (Phase 5c) differentially compare the VM concurrency tier.
pub fn run_vm_concurrent(input: &str) -> InterpreterResult {
    run_vm_concurrent_seeded(input, 0)
}

/// [`run_vm_concurrent`] under an explicit scheduler `seed` — the VM sibling of
/// [`run_treewalker_concurrent_seeded`] for the cross-tier seeded differential.
pub fn run_vm_concurrent_seeded(input: &str, seed: u64) -> InterpreterResult {
    use crate::concurrency::vm_driver::VmTask;
    use logicaffeine_runtime::{run_with_seed, RunOutcome, SchedSeed, SchedulerConfig};
    use std::cell::RefCell;
    use std::rc::Rc;

    with_parsed_program(input, |parsed, interner| match parsed {
        Ok((stmts, type_registry, policies)) => {
            let program = match crate::vm::Compiler::compile_with_types(
                stmts,
                interner,
                Some(type_registry),
            ) {
                Ok(p) => p,
                Err(e) => return InterpreterResult { lines: vec![], error: Some(e) },
            };
            let output: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
            let err_sink: crate::concurrency::driver::ErrSink = Rc::new(RefCell::new(None));
            let mut vm = crate::vm::Vm::new(&program).with_policy_ctx(&policies, interner);
            // Task bodies tier exactly like the main program: a hot integer loop
            // inside a task JIT-compiles. Concurrency ops are JIT-ineligible (not
            // integer ops ⇒ never region-selected), so a tiered region is
            // yield-free and a task only ever suspends on the bytecode path.
            // Spawned children inherit this tier via `spawn_task_vm`.
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(tier) = crate::vm::installed_native_tier() {
                vm = vm.with_native_tier(tier);
            }
            let main_task = VmTask::new(vm, output.clone(), Some(err_sink.clone()));
            let (outcome, _trace) =
                run_with_seed(SchedulerConfig::default(), SchedSeed(seed), move |sched| {
                    sched.spawn_main(Box::new(main_task));
                });
            let mut error = err_sink.borrow().clone();
            if error.is_none() {
                if let RunOutcome::Deadlock = outcome {
                    error = Some("deadlock: every task is blocked waiting".to_string());
                }
            }
            let lines = output.borrow().clone();
            InterpreterResult { lines, error }
        }
        Err(advice) => InterpreterResult { lines: vec![], error: Some(advice) },
    })
}

/// [`run_vm_concurrent_seeded`] under the **work-stealing M:N driver**: `workers`
/// OS-thread workers poll task bodies in parallel while one coordinator owns the
/// scheduler and applies channel ops + flushes output in deterministic pick
/// order. The observable result is byte-identical to the cooperative driver at the
/// same seed (`diff_cooperative_eq_workstealing_seeded`) — the difference is that
/// task bodies genuinely run on multiple cores.
///
/// The executor uses scoped threads, so each worker *borrows* the one shared,
/// immutable program (+ policies + interner) — no clone, no leak. Only a `Send`
/// [`SpawnDesc`] crosses a worker boundary; the worker rebuilds the `!Send` task
/// body locally from it.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_vm_workstealing_seeded(input: &str, seed: u64, workers: usize) -> InterpreterResult {
    use crate::concurrency::vm_driver::VmTask;
    use logicaffeine_runtime::{
        run_workstealing_seeded, RunOutcome, SchedSeed, SchedulerConfig, SpawnDesc, Task,
    };

    with_parsed_program(input, |parsed, interner| match parsed {
        Ok((stmts, type_registry, policies)) => {
            let program = match crate::vm::Compiler::compile_with_types(
                stmts,
                interner,
                Some(type_registry),
            ) {
                Ok(p) => p,
                Err(e) => return InterpreterResult { lines: vec![], error: Some(e) },
            };
            // Build a worker-local task body from a `Send` descriptor. Workers run
            // the bytecode path: the native JIT tier is per-thread state (its code
            // cache is `!Sync`), so no shared tier is installed across workers.
            // Tiering is output-preserving, so this stays byte-identical to the
            // cooperative (tiered) driver; per-worker JIT is a future optimization.
            // The parallelism is the task bodies running on separate cores, which
            // the bytecode interpreter already delivers. The main task runs the
            // top-level program (`Vm::new` positions there); a spawned task is
            // positioned at its function via `setup_task`.
            let build = |desc: SpawnDesc| -> Box<dyn Task<'_> + '_> {
                let mut vm = crate::vm::Vm::new(&program).with_policy_ctx(&policies, interner);
                if !desc.is_main {
                    vm.setup_task(desc.func, &desc.args);
                }
                Box::new(VmTask::work_stealing(vm, None))
            };
            let main = SpawnDesc { func: 0, args: vec![], priority: 0, is_main: true };
            let config = SchedulerConfig::default().with_workers(workers.max(1));
            let result = run_workstealing_seeded(config, SchedSeed(seed), main, build);
            let error = match result.outcome {
                RunOutcome::Deadlock => {
                    Some("deadlock: every task is blocked waiting".to_string())
                }
                _ => None,
            };
            InterpreterResult { lines: result.output, error }
        }
        Err(advice) => InterpreterResult { lines: vec![], error: Some(advice) },
    })
}

pub(crate) fn run_treewalker<'a>(
    stmts: &'a [logicaffeine_language::ast::stmt::Stmt<'a>],
    type_registry: &logicaffeine_language::analysis::TypeRegistry,
    policies: logicaffeine_language::analysis::PolicyRegistry,
    interner: &'a Interner,
    force_async: bool,
    program_args: &[String],
) -> InterpreterResult {
    if crate::concurrency::uses_scheduler(stmts) {
        return run_program_concurrent(
            stmts, type_registry, policies, interner, program_args, None, None, 0,
        );
    }
    let mut interp = crate::interpreter::Interpreter::new(interner)
        .with_type_registry(type_registry)
        .with_policies(policies)
        .with_program_args(program_args.to_vec());
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
    interpret_streaming_with_vfs(input, on_output, None).await
}

/// Like [`interpret_streaming`], but routes the interpreter's file I/O
/// (`Write`/`Read`/`Mount`) to `vfs`. The browser Studio passes its `WebVfs`
/// here so the standard-library I/O vocabulary works against OPFS/IndexedDB; a
/// `None` vfs leaves file I/O reporting "VFS not initialized" as before. The
/// clone-per-task model means concurrent tasks inherit the same VFS handle.
pub async fn interpret_streaming_with_vfs<F>(
    input: &str,
    on_output: std::rc::Rc<std::cell::RefCell<F>>,
    vfs: Option<std::sync::Arc<dyn logicaffeine_system::fs::Vfs>>,
) -> InterpreterResult
where
    F: FnMut(String) + 'static,
{
    interpret_streaming_impl(input, on_output, vfs, None).await
}

/// Like [`interpret_streaming_with_vfs`], but also emits a [`logicaffeine_runtime::SchedSnapshot`]
/// to `observer` after each scheduler slice — the Studio's Tasks/Channels strip subscribes
/// here to show a concurrent program's live task and channel state as it runs.
pub async fn interpret_streaming_with_vfs_observer<F>(
    input: &str,
    on_output: std::rc::Rc<std::cell::RefCell<F>>,
    vfs: Option<std::sync::Arc<dyn logicaffeine_system::fs::Vfs>>,
    observer: ObserverCallback,
) -> InterpreterResult
where
    F: FnMut(String) + 'static,
{
    interpret_streaming_impl(input, on_output, vfs, Some(observer)).await
}

async fn interpret_streaming_impl<F>(
    input: &str,
    on_output: std::rc::Rc<std::cell::RefCell<F>>,
    vfs: Option<std::sync::Arc<dyn logicaffeine_system::fs::Vfs>>,
    observer: Option<ObserverCallback>,
) -> InterpreterResult
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
            if let Some(rejection) = send_escape_rejection(&stmts) {
                return rejection;
            }
            // Create the callback wrapper that calls the user's callback
            let callback: OutputCallback = std::rc::Rc::new(std::cell::RefCell::new(move |line: String| {
                (on_output.borrow_mut())(line);
            }));

            // Concurrent programs run on the deterministic scheduler — the browser
            // concurrency path — driven in slices that yield a macrotask between them so
            // the UI repaints, streaming each line as it is produced and emitting a
            // snapshot to the observer (if any). (Without this, a concurrency op would
            // `yield_request` with no scheduler installed and panic.)
            if crate::concurrency::uses_scheduler(&stmts) {
                return run_program_concurrent_streaming(
                    &stmts,
                    &type_registry_for_interp,
                    policy_registry,
                    &interner,
                    &[],
                    vfs,
                    Some(callback),
                    observer,
                    0,
                )
                .await;
            }

            let mut interp = crate::interpreter::Interpreter::new(&interner)
                .with_type_registry(&type_registry_for_interp)
                .with_policies(policy_registry)
                .with_output_callback(callback);
            if let Some(v) = vfs {
                interp = interp.with_vfs(v);
            }

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
    let (proof_exprs, goal_expr, definitions) = theorem_proof_exprs_with_defs(input)?;

    // === STEPS 3-6: Prove → certify → type-check (the one canonical pipeline) ===
    let outcome = logicaffeine_proof::verify::prove_certify_check_with_defs(
        &proof_exprs,
        &goal_expr,
        &definitions,
    );
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

/// An English theorem proved, with its FOL and a human-readable PROOF TRACE — the
/// "English in → proof out, see the trace" path. Parsing yields the premises and
/// goal as FOL; the kernel-certified backward chainer yields the derivation, which
/// renders as an indented step-by-step proof.
#[derive(Debug, Clone)]
pub struct TheoremTrace {
    /// Whether the goal was proved (kernel-certified).
    pub verified: bool,
    /// The premises, rendered as FOL — what the English compiled to.
    pub premises: Vec<String>,
    /// The goal, rendered as FOL.
    pub goal: String,
    /// The derivation rendered as an indented proof trace (`└─ [Rule] conclusion`
    /// per step). `None` when no derivation was found.
    pub trace: Option<String>,
    /// The verification error, when proving failed.
    pub error: Option<String>,
}

/// Prove an English `## Theorem` block and return its FOL plus a rendered PROOF
/// TRACE. Unlike [`verify_theorem`] (which returns the raw kernel term), this
/// surfaces the derivation tree so the proof steps are visible.
pub fn prove_theorem_trace(input: &str) -> Result<TheoremTrace, ParseError> {
    let (proof_exprs, goal_expr, definitions) = theorem_proof_exprs_with_defs(input)?;
    let outcome = logicaffeine_proof::verify::prove_certify_check_with_defs(
        &proof_exprs,
        &goal_expr,
        &definitions,
    );
    Ok(TheoremTrace {
        verified: outcome.verified,
        premises: proof_exprs.iter().map(|p| p.to_string()).collect(),
        goal: goal_expr.to_string(),
        trace: outcome.derivation.as_ref().map(|d| d.display_tree()),
        error: outcome.verification_error,
    })
}

// ═══════════════════════════════════════════════════════════════════
// Proof / Math → Rust extraction (the Curry-Howard "Forge", UI-facing)
// ═══════════════════════════════════════════════════════════════════

/// Names the user defined — present in `ctx` but absent from a fresh kernel
/// context (i.e. excluding the StandardLibrary baseline). Inductives are listed
/// before definitions so the extracted module reads top-down; StandardLibrary
/// items are pulled in by [`extract_math_rust`] only when transitively needed.
fn user_defined_entries(ctx: &kernel::Context) -> Vec<String> {
    let baseline = kernel::interface::Repl::new();
    let base = baseline.context();
    let mut base_names: HashSet<String> = HashSet::new();
    for (name, _) in base.iter_inductives() {
        base_names.insert(name.to_string());
    }
    for (name, _, _) in base.iter_definitions() {
        base_names.insert(name.to_string());
    }

    let mut entries = Vec::new();
    for (name, _) in ctx.iter_inductives() {
        // A user inductive extracts as an enum only if every constructor field is
        // a concrete Rust type. Dependent/indexed families (e.g. `Eq<Nat,n,…>`,
        // value-indexed proof types) and opaque/primitive types are skipped.
        if !base_names.contains(name) && inductive_is_emittable(ctx, name) {
            entries.push(name.to_string());
        }
    }
    for (name, ty, _) in ctx.iter_definitions() {
        // Only data-typed definitions whose body references nothing inextractable
        // (no Prop proofs, no axioms/tactics like `syn_diag`/`try_auto`) extract to
        // runnable Rust — so we never emit Rust that references undefined symbols.
        if !base_names.contains(name)
            && type_is_emittable(ctx, ty, &[])
            && crate::extraction::is_extractable(ctx, name)
        {
            entries.push(name.to_string());
        }
    }
    // Deterministic order (iter_* walk HashMaps with per-instance random seeds).
    entries.sort();
    entries.dedup();
    entries
}

/// Whether a kernel type extracts to a concrete Rust type: a mapped primitive, a
/// user inductive with emittable constructors (→ enum), a declared type parameter
/// (`generics`), or a function/application built from those. `Sort` (Prop/Type),
/// logical/opaque types, and undeclared type variables are not.
fn type_is_emittable(ctx: &kernel::Context, ty: &kernel::Term, generics: &[String]) -> bool {
    use kernel::Term;
    match ty {
        Term::Global(name) => {
            crate::extraction::primitive_rust_type(name).is_some()
                || (ctx.is_inductive(name)
                    && !ctx.get_constructors(name).is_empty()
                    && !crate::extraction::is_logical_type(name))
        }
        // A type variable is only a real Rust type if it's a declared generic of
        // the enclosing inductive — never in a bare definition type.
        Term::Var(v) => generics.iter().any(|g| g == v),
        Term::Pi { param_type, body_type, .. } => {
            type_is_emittable(ctx, param_type, generics) && type_is_emittable(ctx, body_type, generics)
        }
        Term::App(f, a) => {
            type_is_emittable(ctx, f, generics) && type_is_emittable(ctx, a, generics)
        }
        _ => false,
    }
}

/// The leading `: Type` parameters of an inductive (its Rust generics).
fn inductive_generics(ctx: &kernel::Context, ind: &str) -> Vec<String> {
    use kernel::Term;
    let mut names = Vec::new();
    let mut cur = match ctx.get_global(ind) {
        Some(t) => t,
        None => return names,
    };
    while let Term::Pi { param, param_type, body_type } = cur {
        if matches!(param_type.as_ref(), Term::Sort(_)) {
            names.push(param.clone());
            cur = body_type;
        } else {
            break;
        }
    }
    names
}

/// A user inductive is emittable iff it has constructors and every constructor
/// *field* (after the leading type parameters) is itself an emittable type.
fn inductive_is_emittable(ctx: &kernel::Context, ind: &str) -> bool {
    use kernel::Term;
    let ctors = ctx.get_constructors(ind);
    if ctors.is_empty() || crate::extraction::is_logical_type(ind) {
        return false;
    }
    let generics = inductive_generics(ctx, ind);
    for (_, ty) in &ctors {
        let mut cur = *ty;
        for _ in 0..generics.len() {
            if let Term::Pi { body_type, .. } = cur {
                cur = body_type;
            } else {
                break;
            }
        }
        while let Term::Pi { param_type, body_type, .. } = cur {
            if !type_is_emittable(ctx, param_type, &generics) {
                return false;
            }
            cur = body_type;
        }
    }
    true
}

/// Extract every user-defined inductive and definition in `ctx` into one Rust
/// module — the "compile my math to Rust" path for the Math studio. Shared and
/// StandardLibrary dependencies are pulled in only when a user definition
/// transitively needs them, and are emitted exactly once.
pub fn extract_math_rust(ctx: &kernel::Context) -> Result<String, String> {
    let entries = user_defined_entries(ctx);
    if entries.is_empty() {
        return Ok("// nothing defined yet — add a Definition or Inductive".to_string());
    }
    let module = extract_math_module(ctx)?;
    let checks: Vec<(String, Vec<kernel::Term>)> =
        property_checks(ctx).into_iter().map(|(n, _, p)| (n, p)).collect();
    // The "compiled mathematical object" runs and proves itself.
    Ok(format!("{module}{}", math_demo_main(ctx, &entries, &checks)))
}

/// Extract every user-defined inductive/definition in `ctx` into one Rust module —
/// types, functions, and `check_*` property fns from proven theorems — WITHOUT a
/// demo `main`. This is the linkable artifact bundled into an imperative program's
/// `mod proven`; [`extract_math_rust`] wraps it with a self-verifying `main` for the
/// standalone Math compile.
pub fn extract_math_module(ctx: &kernel::Context) -> Result<String, String> {
    let entries = user_defined_entries(ctx);
    if entries.is_empty() {
        return Ok("// nothing defined yet — add a Definition or Inductive".to_string());
    }
    let refs: Vec<&str> = entries.iter().map(|s| s.as_str()).collect();
    let mut module = crate::extraction::extract_programs(ctx, &refs).map_err(|e| e.to_string())?;
    // Proven theorems → runnable property checks over the extracted functions
    // (e.g. `∀n. add Zero n = n` → `fn check_…(n) -> bool { add(Zero, n) == n }`).
    // Appended in sorted order so the module is byte-identical across recompiles.
    let mut check_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (name, check_fn, _) in property_checks(ctx) {
        module.push_str(&check_fn);
        check_names.insert(name);
    }
    // Honest notes for proof-irrelevant theorems (Gödel, consistency, `True`, …): a
    // definition whose TYPE is a proposition is a *proof*, and by Curry-Howard a proof
    // of a Prop has no computational content — so it has no runnable form. The
    // constructive dependencies it uses are still extracted above; the note explains
    // why the theorem itself isn't a function. Sorted for deterministic output.
    let base = baseline_names();
    let mut notes: Vec<String> = Vec::new();
    for (name, ty, _) in ctx.iter_definitions() {
        if base.contains(name) || check_names.contains(name) {
            continue;
        }
        if def_type_is_proposition(ty) {
            notes.push(format!(
                "// note: `{name}` is a proof of a proposition — proof-irrelevant (no \
                 computational content), so it has no runnable form; any constructive \
                 definitions it relies on are extracted above.\n"
            ));
        }
    }
    notes.sort();
    for n in notes {
        module.push_str(&n);
    }
    Ok(module)
}

/// Whether a definition's type is a proposition — i.e. the definition is a *proof*
/// (proof-irrelevant), not computational data. Peels any `∀` (Pi) binders, then
/// checks whether the head of the result is a logical/proof type (`Eq`/`And`/`True`/
/// `Syntax`/`Derivation`/…). Used to emit honest notes for theorems with no runnable form.
fn def_type_is_proposition(ty: &kernel::Term) -> bool {
    use kernel::Term;
    let mut cur = ty;
    while let Term::Pi { body_type, .. } = cur {
        cur = body_type;
    }
    let mut head = cur;
    while let Term::App(f, _) = head {
        head = f;
    }
    matches!(head, Term::Global(n) if crate::extraction::is_logical_type(n))
}

/// Proven theorems → runnable property checks: `(name, check_fn_source, param_types)`,
/// sorted by name for deterministic emission. A theorem yields a check iff
/// [`crate::extraction::emit_property_check`] produces a runnable predicate over the
/// extracted functions (it quantifies only over data types).
fn property_checks(ctx: &kernel::Context) -> Vec<(String, String, Vec<kernel::Term>)> {
    let base = baseline_names();
    let mut checks: Vec<(String, String, Vec<kernel::Term>)> = Vec::new();
    for (name, ty, _) in ctx.iter_definitions() {
        if base.contains(name) {
            continue;
        }
        if let Some(check_fn) = crate::extraction::emit_property_check(ctx, name, ty) {
            checks.push((name.to_string(), check_fn, pi_param_types(ty)));
        }
    }
    checks.sort_by(|a, b| a.0.cmp(&b.0));
    checks
}

/// Names present in a fresh `Repl::new()` (the StandardLibrary baseline).
fn baseline_names() -> std::collections::HashSet<String> {
    let baseline = kernel::interface::Repl::new();
    let base = baseline.context();
    let mut names = std::collections::HashSet::new();
    for (n, _) in base.iter_inductives() {
        names.insert(n.to_string());
    }
    for (n, _, _) in base.iter_definitions() {
        names.insert(n.to_string());
    }
    names
}

/// Whether a term applies the `div`/`mod` arithmetic builtins (whose extracted Rust
/// `/`/`%` panics on a zero divisor) — used to skip them in the self-verifying demo.
fn term_uses_div_or_mod(term: &kernel::Term) -> bool {
    use kernel::Term;
    match term {
        Term::Global(n) => n == "div" || n == "mod",
        Term::App(f, a) => term_uses_div_or_mod(f) || term_uses_div_or_mod(a),
        Term::Lambda { param_type, body, .. } => {
            term_uses_div_or_mod(param_type) || term_uses_div_or_mod(body)
        }
        Term::Pi { param_type, body_type, .. } => {
            term_uses_div_or_mod(param_type) || term_uses_div_or_mod(body_type)
        }
        Term::Fix { body, .. } => term_uses_div_or_mod(body),
        Term::Match { discriminant, motive, cases } => {
            term_uses_div_or_mod(discriminant)
                || term_uses_div_or_mod(motive)
                || cases.iter().any(term_uses_div_or_mod)
        }
        _ => false,
    }
}

/// A self-verifying demo `main`: each value/function result is `assert_eq!`d
/// against `kernel::normalize` (the kernel's evaluator) and printed; each proven
/// theorem's property check is run on a sample and asserted.
fn math_demo_main(
    ctx: &kernel::Context,
    entries: &[String],
    checks: &[(String, Vec<kernel::Term>)],
) -> String {
    use kernel::Term;
    let mut lines = Vec::new();
    for name in entries {
        let Some(body) = ctx.get_definition_body(name) else {
            continue; // an inductive, not a definition
        };
        // Skip exercising a def that uses `div`/`mod`: the demo samples Int as 0, and
        // `n / n` / `n % n` at n=0 is a compile-time divide-by-zero panic (and the
        // kernel leaves div-by-0 stuck, diverging from Rust's panic). The function is
        // still EXTRACTED — just not run in the self-verifying demo.
        if term_uses_div_or_mod(body) {
            continue;
        }
        if matches!(body, Term::Lambda { .. } | Term::Fix { .. }) {
            // A function: apply it to kernel-built sample arguments.
            let Some(ty) = ctx.get_definition_type(name) else { continue };
            let params = pi_param_types(ty);
            let samples: Option<Vec<Term>> =
                params.iter().map(|p| sample_value(ctx, p)).collect();
            let Some(samples) = samples else { continue }; // can't sample → skip
            if samples.is_empty() {
                continue;
            }
            let args_rust: Vec<String> =
                samples.iter().map(|s| crate::extraction::emit_value(ctx, s)).collect();
            let mut app = Term::Global(name.clone());
            for s in &samples {
                app = Term::App(Box::new(app), Box::new(s.clone()));
            }
            let expected = crate::extraction::emit_value(ctx, &kernel::normalize(ctx, &app));
            let call = format!("{}({})", name, args_rust.join(", "));
            lines.push(format!("    assert_eq!({call}, {expected});"));
            lines.push(format!("    println!(\"{name}(..) = {{:?}}\", {call});"));
        } else {
            // A value: evaluate and self-verify.
            let expected = crate::extraction::emit_value(ctx, &kernel::normalize(ctx, body));
            lines.push(format!("    assert_eq!({name}(), {expected});"));
            lines.push(format!("    println!(\"{name} = {{:?}}\", {name}());"));
        }
    }
    // Run each proven theorem's property check on a sample and assert it holds.
    for (name, param_types) in checks {
        let samples: Option<Vec<kernel::Term>> =
            param_types.iter().map(|p| sample_value(ctx, p)).collect();
        let Some(samples) = samples else { continue };
        let args: Vec<String> =
            samples.iter().map(|s| crate::extraction::emit_value(ctx, s)).collect();
        let call = format!("check_{}({})", name, args.join(", "));
        lines.push(format!("    assert!({call}, \"theorem {name} failed on sample\");"));
        lines.push(format!("    println!(\"\\u{{2713}} {name} holds (checked on a sample)\");"));
    }
    if lines.is_empty() {
        return "\nfn main() {}\n".to_string();
    }
    format!("\nfn main() {{\n{}\n}}\n", lines.join("\n"))
}

/// The parameter types of a (possibly curried) function type `A -> B -> C`.
fn pi_param_types(ty: &kernel::Term) -> Vec<kernel::Term> {
    use kernel::Term;
    let mut params = Vec::new();
    let mut cur = ty;
    while let Term::Pi { param_type, body_type, .. } = cur {
        params.push((**param_type).clone());
        cur = body_type;
    }
    params
}

/// A small sample value of a type, for the demo (a primitive zero, or the first
/// nullary constructor of an inductive). `None` if we can't build one cheaply.
fn sample_value(ctx: &kernel::Context, ty: &kernel::Term) -> Option<kernel::Term> {
    use kernel::{Literal, Term};
    match ty {
        Term::Global(name) => match name.as_str() {
            "Int" => Some(Term::Lit(Literal::Int(0))),
            "Float" => Some(Term::Lit(Literal::Float(0.0))),
            "Text" => Some(Term::Lit(Literal::Text(String::new()))),
            // First nullary constructor of a user inductive (Zero, Yes, MNil, …).
            _ if ctx.is_inductive(name) => ctx
                .get_constructors(name)
                .into_iter()
                .find(|(_, cty)| !matches!(cty, Term::Pi { .. }))
                .map(|(cname, _)| Term::Global(cname.to_string())),
            _ => None,
        },
        _ => None,
    }
}

/// Compile a Math-mode SOURCE program (the editor text) to a Rust module: split
/// it into vernacular statements, run them into a fresh kernel, then extract every
/// user-defined inductive/definition. This is the exact pipeline the Studio's
/// Math "🦀 Compile" button drives, exposed as one function so it is testable.
pub fn extract_math_rust_from_source(input: &str) -> String {
    let mut repl = kernel::interface::Repl::new();
    for stmt in parse_math_statements(input) {
        // Errors on individual statements (e.g. `Check`/`Eval`) are ignored — only
        // the resulting definitions/inductives matter for extraction.
        let _ = repl.execute(&stmt);
    }
    match extract_math_rust(repl.context()) {
        Ok(rust) => rust,
        Err(e) => format!("// extraction error: {e}"),
    }
}

/// Like [`extract_math_rust_from_source`], but produces the main-less module (the
/// linkable artifact bundled into an imperative program's `mod proven`).
pub fn extract_math_module_from_source(input: &str) -> String {
    let mut repl = kernel::interface::Repl::new();
    for stmt in parse_math_statements(input) {
        let _ = repl.execute(&stmt);
    }
    match extract_math_module(repl.context()) {
        Ok(rust) => rust,
        Err(e) => format!("// extraction error: {e}"),
    }
}

/// Partition a possibly-MIXED document into its imperative and math streams.
///
/// A mixed document interleaves imperative LOGOS (`## To`/`## Main`/statements) with
/// Coq-style math (`Definition`/`Inductive`/`Axiom`/`Theorem`/`Lemma`/`Fixpoint`) and
/// literate `## Theorem:`/`## Lemma:` blocks. The math blocks feed the Forge (extracted
/// into `mod proven`); the imperative blocks compile normally and call into it.
///
/// Returns `(imperative_src, Some(math_src))` when math is present, or
/// `(source.to_string(), None)` for a pure imperative program — so that path is a
/// guaranteed no-op (byte-identical compile). In `imperative_src` the math lines are
/// BLANKED (kept as empty lines), not deleted, so imperative line numbers — and thus
/// error spans — stay aligned with the original source.
///
/// `## To`/`## A X is one of` stay imperative (only the Coq keywords and `## Theorem:`/
/// `## Lemma:` route to math), so this never steals an imperative function or enum.
pub fn partition_mixed(source: &str) -> (String, Option<String>) {
    let lines: Vec<&str> = source.lines().collect();
    let mut is_math = vec![false; lines.len()];
    let mut i = 0;
    let mut any = false;
    while i < lines.len() {
        let t = lines[i].trim();
        if !is_math_block_start(t) {
            i += 1;
            continue;
        }
        any = true;
        if t.starts_with("## Theorem:") || t.starts_with("## Lemma:") {
            // Literate theorem: header + indented / `Statement:` / `Proof:` lines.
            is_math[i] = true;
            i += 1;
            while i < lines.len() {
                let nt = lines[i].trim();
                let indented = lines[i].starts_with(' ') || lines[i].starts_with('\t');
                if nt.is_empty() {
                    is_math[i] = true;
                    i += 1;
                    continue;
                }
                if indented || nt.starts_with("Statement:") || nt.starts_with("Proof:") {
                    is_math[i] = true;
                    i += 1;
                    if nt.starts_with("Proof:") && nt.ends_with('.') {
                        break;
                    }
                } else {
                    break;
                }
            }
        } else {
            // Coq-style statement: accumulate until a line ending with `.`. Defensive:
            // a `## ` block header (e.g. `## Main`, `## To`) ends the math block even
            // if the terminating `.` is missing/unterminated, so a malformed Definition
            // never swallows the imperative code that follows it.
            let mut ended = t.ends_with('.');
            is_math[i] = true;
            i += 1;
            while !ended && i < lines.len() {
                if lines[i].trim_start().starts_with("## ") {
                    break;
                }
                is_math[i] = true;
                ended = lines[i].trim().ends_with('.');
                i += 1;
            }
        }
    }
    if !any {
        return (source.to_string(), None);
    }
    let imp: Vec<&str> = lines
        .iter()
        .enumerate()
        .map(|(j, l)| if is_math[j] { "" } else { *l })
        .collect();
    let math: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(j, _)| is_math[*j])
        .map(|(_, l)| *l)
        .collect();
    (imp.join("\n"), Some(math.join("\n")))
}

fn is_math_block_start(trimmed: &str) -> bool {
    const COQ: [&str; 6] = [
        "Definition ", "Inductive ", "Axiom ", "Theorem ", "Lemma ", "Fixpoint ",
    ];
    COQ.iter().any(|k| trimmed.starts_with(k))
        || trimmed.starts_with("## Theorem:")
        || trimmed.starts_with("## Lemma:")
}

/// Extract a mixed document's math stream into a bundleable `mod proven` body, or
/// `None` if it has no public items to call into (e.g. only vacuous proofs / parse
/// errors) — so we never emit a `use proven::*;` over an empty module.
///
/// Unlike standalone math extraction, this ALSO emits a `Showable` impl for each
/// non-generic proven enum, so imperative `Show <proven value>` works. `Showable`
/// lives in `logicaffeine_system` (in scope inside `mod proven` via `use super::*;`),
/// which is ONLY linked when bundled into an imperative program — hence it is added
/// here, not in `extract_math_module` (whose output must also compile standalone, dep-free).
pub(crate) fn mixed_proven_module(math_src: &str) -> Option<String> {
    let mut repl = kernel::interface::Repl::new();
    for stmt in parse_math_statements(math_src) {
        let _ = repl.execute(&stmt);
    }
    let ctx = repl.context();
    let mut rust = match extract_math_module(ctx) {
        Ok(r) => r,
        Err(_) => return None,
    };
    if !(rust.contains("pub fn") || rust.contains("pub enum") || rust.contains("pub struct")) {
        return None;
    }
    // Show-verb integration: bridge each non-generic proven enum to the runtime
    // `Showable` trait (via Debug). Sorted for deterministic output.
    let base = baseline_names();
    let mut inds: Vec<String> = Vec::new();
    for (name, _) in ctx.iter_inductives() {
        if !base.contains(name)
            && inductive_is_emittable(ctx, name)
            && inductive_generics(ctx, name).is_empty()
        {
            inds.push(name.to_string());
        }
    }
    inds.sort();
    for ind in inds {
        rust.push_str(&format!(
            "impl Showable for {ind} {{ fn format_show(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {{ write!(f, \"{{:?}}\", self) }} }}\n"
        ));
    }
    Some(rust)
}

/// Split a Math-mode program into complete vernacular statements.
///
/// Handles both Coq-style (period-terminated) and the Literate forms
/// (`## To …` functions, `## Theorem:` blocks, `A X is either …` inductives).
pub fn parse_math_statements(code: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let lines: Vec<&str> = code.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with("--") {
            i += 1;
            continue;
        }

        // Literate function definition: "## To ..."
        if trimmed.starts_with("## To ") {
            let mut block = String::new();
            block.push_str(trimmed);
            i += 1;

            while i < lines.len() {
                let next_line = lines[i];
                let next_trimmed = next_line.trim();

                if next_trimmed.is_empty() {
                    i += 1;
                    continue;
                }
                if next_trimmed.starts_with("--") {
                    i += 1;
                    continue;
                }

                let is_indented = next_line.starts_with(' ') || next_line.starts_with('\t');
                let is_continuation = next_trimmed.starts_with("Consider ")
                    || next_trimmed.starts_with("When ")
                    || next_trimmed.starts_with("Yield ");

                if is_indented || is_continuation {
                    block.push(' ');
                    block.push_str(next_trimmed);
                    i += 1;
                } else {
                    break;
                }
            }

            statements.push(block);
            continue;
        }

        // Literate theorem: "## Theorem: ..." (header + Statement: + Proof:)
        if trimmed.starts_with("## Theorem:") {
            let mut block = String::new();
            block.push_str(trimmed);
            i += 1;

            while i < lines.len() {
                let next_line = lines[i];
                let next_trimmed = next_line.trim();

                if next_trimmed.is_empty() {
                    i += 1;
                    continue;
                }
                if next_trimmed.starts_with("--") {
                    i += 1;
                    continue;
                }

                let is_indented = next_line.starts_with(' ') || next_line.starts_with('\t');
                let is_theorem_part = next_trimmed.starts_with("Statement:")
                    || next_trimmed.starts_with("Proof:");

                if is_indented || is_theorem_part {
                    block.push('\n');
                    block.push_str(next_line);
                    i += 1;
                    if next_trimmed.starts_with("Proof:") && next_trimmed.ends_with('.') {
                        break;
                    }
                } else {
                    break;
                }
            }

            statements.push(block);
            continue;
        }

        // Literate inductive: "A X is either..." / "An X is either..."
        if (trimmed.starts_with("A ") || trimmed.starts_with("An ")) && trimmed.contains(" is either")
        {
            if trimmed.ends_with('.') && !trimmed.trim_end_matches('.').ends_with(':') {
                statements.push(trimmed.to_string());
                i += 1;
                continue;
            }

            let mut block = String::new();
            block.push_str(trimmed);
            i += 1;

            while i < lines.len() {
                let next_line = lines[i];
                let next_trimmed = next_line.trim();

                if next_trimmed.is_empty() {
                    i += 1;
                    continue;
                }
                if next_trimmed.starts_with("--") {
                    i += 1;
                    continue;
                }

                let is_indented = next_line.starts_with(' ') || next_line.starts_with('\t');
                let looks_like_variant = next_trimmed.starts_with("a ")
                    || next_trimmed
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);

                if is_indented
                    || (looks_like_variant
                        && !next_trimmed.starts_with("A ")
                        && !next_trimmed.starts_with("An "))
                {
                    if !block.ends_with(':') {
                        block.push_str(" or ");
                    } else {
                        block.push(' ');
                    }
                    block.push_str(next_trimmed.trim_end_matches('.'));
                    i += 1;
                } else {
                    break;
                }
            }

            if !block.ends_with('.') {
                block.push('.');
            }
            statements.push(block);
            continue;
        }

        // Traditional Coq-style: accumulate until period
        let mut current_stmt = String::new();
        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();

            if trimmed.is_empty() || trimmed.starts_with("--") {
                i += 1;
                continue;
            }

            if !current_stmt.is_empty() {
                current_stmt.push(' ');
            }
            current_stmt.push_str(trimmed);
            i += 1;

            if trimmed.ends_with('.') {
                break;
            }
        }

        if !current_stmt.is_empty() {
            statements.push(current_stmt);
        }
    }

    statements
}

/// Compile a Logic-mode input to runnable Rust.
///
/// A `## Theorem:` block becomes a **model-checker** for `premises ⊨ goal`; a
/// plain sentence becomes a model-checker for that formula — a self-contained
/// `Model` + `holds(&Model) -> bool` + demo `main` you can compile and run
/// ([`crate::extraction::fol_model`]). The finite-domain *puzzle* (Simon grid) is
/// the exception: it would run the full solver synchronously and freeze the page,
/// so it bails with a note (use Execute to solve it).
pub fn extract_logic_rust(input: &str) -> Result<String, String> {
    extract_logic_impl(input, true)
}

/// Like [`extract_logic_rust`], but emits NO demo `main` — the linkable form bundled
/// into an imperative program's `mod proven` (which has its own `main`). Produces the
/// `World` + `holds` (+ `Monitor`) library items only.
pub fn extract_logic_module(input: &str) -> Result<String, String> {
    extract_logic_impl(input, false)
}

fn extract_logic_impl(input: &str, emit_main: bool) -> Result<String, String> {
    use crate::extraction::fol_model::{fol_to_model_checker, fol_to_model_checker_module};
    let emit = |premises: &[logicaffeine_proof::ProofExpr],
                goal: &logicaffeine_proof::ProofExpr,
                english: &str,
                fol: &str| {
        if emit_main {
            fol_to_model_checker(premises, goal, english, fol)
        } else {
            fol_to_model_checker_module(premises, goal, english, fol)
        }
    };

    // A `## Theorem:` block: model-check `premises ⊨ goal` — unless it is a grid
    // puzzle (bail before the solver runs, see above).
    if let Ok((premises, goal)) = theorem_proof_exprs(input) {
        if looks_like_grid(&premises) {
            return Ok("// this is a finite-domain puzzle — run it with Execute. \
                Compiling it to Rust would run the full solver synchronously."
                .to_string());
        }
        let fol = if premises.is_empty() {
            goal.to_string()
        } else {
            format!(
                "{} ⊢ {}",
                premises.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
                goal
            )
        };
        return Ok(emit(&premises, &goal, input, &fol));
    }

    // A plain sentence: model-check the single formula.
    let proof = compile_for_proof(input);
    if let Some(expr) = proof.proof_expr {
        let fol = proof.logic_string.clone().unwrap_or_else(|| expr.to_string());
        return Ok(emit(&[], &expr, input, &fol));
    }

    Ok("// could not parse this input into a logical formula to compile".to_string())
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
    let (proof_exprs, goal, defaults, _definitions) = theorem_problem(input, true)?;
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
    let (proof_exprs, _goal, defaults, _definitions) = theorem_problem(input, true)?;
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
/// Parse a `## Theorem` document and convert its premises and goal to
/// [`ProofExpr`]. Public so a puzzle solver can obtain the parsed-FOL premises
/// (the Given clues/declarations) and feed them to the entailment oracle.
pub fn theorem_proof_exprs(input: &str) -> Result<(Vec<ProofExpr>, ProofExpr), ParseError> {
    let (premises, goal, _defaults, _definitions) = theorem_problem(input, false)?;
    Ok((premises, goal))
}

/// Like [`theorem_proof_exprs`] but also returns the document's `## Define`
/// blocks (Rung 0a) lowered to proof-layer definitions, so the prover can
/// δ-unfold them. This is the entry the kernel-certified theorem path uses.
pub fn theorem_proof_exprs_with_defs(
    input: &str,
) -> Result<
    (
        Vec<ProofExpr>,
        ProofExpr,
        Vec<logicaffeine_proof::verify::Definition>,
    ),
    ParseError,
> {
    let (premises, goal, _defaults, definitions) = theorem_problem(input, false)?;
    Ok((premises, goal, definitions))
}

/// The `uses` dependency graph (Rung 0b) for an English document: its `## Define`
/// blocks plus the theorem's premises and goal, lowered and analyzed. Each node
/// is a definition or the theorem; each edge is a `uses`. This is the structure
/// a `mathscrapes` node/edge compiles into.
pub fn theorem_dependency_graph(
    input: &str,
) -> Result<logicaffeine_proof::verify::DependencyGraph, ParseError> {
    let (premises, goal, definitions) = theorem_proof_exprs_with_defs(input)?;
    Ok(logicaffeine_proof::verify::dependency_graph(
        &definitions,
        &premises,
        &goal,
    ))
}

/// ANSWER a wh-question theorem ("Given: … Prove: Who is a lawyer?"). A wh-goal
/// converts to ∃x.φ(x); the ANSWER is the witness. We find it the same way the
/// puzzle is meant to be solved — by the GENERAL kernel-certified prover, applied
/// per candidate: enumerate the domain individuals named in the premises and prove
/// each candidate goal φ(c); the c's that prove are the answer (usually one, for a
/// puzzle with a unique solution). No question-specific reasoning — every step is
/// the same `prove_certify_check` that proves Socrates.
pub fn answer_question(input: &str) -> Result<Vec<String>, ParseError> {
    let (premises, goal) = theorem_proof_exprs(input)?;
    match &goal {
        ProofExpr::Exists { variable, body } => Ok(answer_wh(&premises, variable, body)),
        _ => Err(ParseError {
            kind: logicaffeine_language::error::ParseErrorKind::Custom(
                "Prove goal is not a question (expected a wh-question ∃-form)".to_string(),
            ),
            span: logicaffeine_language::token::Span::default(),
        }),
    }
}

/// The witnesses of a wh-question `∃var. body(var)`: enumerate the domain individuals
/// named in the premises and keep those `c` for which `body(c)` is entailed by the
/// certified no-Z3 path. Shared by [`answer_question`] and the studio entry
/// [`compile_theorem_for_ui`], so a wh-goal never reaches the closed-goal search.
fn answer_wh(premises: &[ProofExpr], var: &str, body: &ProofExpr) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();
    for p in premises {
        collect_constants(p, &mut candidates);
    }
    candidates.sort();
    candidates.dedup();
    // Prepare the premise set ONCE (tense-erase + finite-domain grounding) — it is
    // identical for every candidate cell, so doing it per candidate would re-ground the
    // whole grid N times. This is the optimization that keeps bigger puzzles tractable.
    let trace = std::env::var("LOGOS_TRACE").is_ok();
    let t0 = trace.then(std::time::Instant::now);
    let prepared = prepare_premises(premises);
    if let Some(t0) = t0 {
        eprintln!(
            "[answer] {} premises → prepared ({} clauses) in {:.2?}; {} candidates",
            premises.len(),
            prepared.len(),
            t0.elapsed(),
            candidates.len()
        );
    }
    let mut answers = Vec::new();
    for c in &candidates {
        let tc = trace.then(std::time::Instant::now);
        let candidate_goal =
            logicaffeine_language::proof_convert::instantiate_var_with_constant(body, var, c);
        let ok = candidate_entailed_prepared(&prepared, &candidate_goal);
        if let Some(tc) = tc {
            eprintln!("[answer]   {:<14} {} ({:.2?})", c, ok, tc.elapsed());
        }
        if ok {
            answers.push(c.clone());
        }
    }
    if let Some(t0) = t0 {
        eprintln!("[answer] total {:.2?} → {:?}", t0.elapsed(), answers);
    }
    answers
}

/// SOLVE the whole grid for the studio (the form-recognized easter egg): re-parse the
/// theorem and, if it is a finite-domain grid, fill every cell the certified prover can
/// force. `None` for a non-grid theorem.
pub fn solve_grid(input: &str) -> Option<SolvedGrid> {
    let parsed = parse_theorem(input).ok()?;
    if !looks_like_grid(&parsed.premises) {
        return None;
    }
    solve_grid_from_premises(&parsed.premises, input)
}

/// One category closure `∀x(RowSort(x) → d₁ ∨ … ∨ dₙ)`: the bound variable, its row
/// sort, and each disjunct paired with the value it contributes.
struct GridClosure {
    var: String,
    row_sort: String,
    disjuncts: Vec<(String, ProofExpr)>,
}

/// Split a (possibly nested) disjunction into its leaf disjuncts.
fn flatten_or<'a>(e: &'a ProofExpr, out: &mut Vec<&'a ProofExpr>) {
    match e {
        ProofExpr::Or(l, r) => {
            flatten_or(l, out);
            flatten_or(r, out);
        }
        other => out.push(other),
    }
}

/// The unary SORT predicate guarding `var` (`Trip(x)` → "trip"), if the antecedent is
/// one — possibly under conjunction.
fn antecedent_sort(e: &ProofExpr, var: &str) -> Option<String> {
    let is_var = |t: &ProofTerm| matches!(t, ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if v == var);
    match e {
        ProofExpr::Predicate { name, args, .. } if args.len() == 1 && is_var(&args[0]) => {
            Some(name.clone())
        }
        ProofExpr::And(l, r) => antecedent_sort(l, var).or_else(|| antecedent_sort(r, var)),
        _ => None,
    }
}

/// The value a disjunct contributes for `var`: the constant argument of a binary relation
/// (`in(x, Florida)` → "Florida") or the predicate name of a unary one (`cycling(x)` →
/// "cycling"). `None` if the disjunct is not a simple predicate over `var`.
fn disjunct_value(d: &ProofExpr, var: &str) -> Option<String> {
    let is_var = |t: &ProofTerm| matches!(t, ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if v == var);
    match d {
        ProofExpr::Predicate { name, args, .. } => match args.as_slice() {
            [a] if is_var(a) => Some(name.clone()),
            [a, ProofTerm::Constant(c)] if is_var(a) => Some(c.clone()),
            [ProofTerm::Constant(c), a] if is_var(a) => Some(c.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Extract one [`GridClosure`] per disjunctive-closure premise (`∀x(Trip(x) → A ∨ B ∨
/// …)`). These closures ARE the grid's columns — the antecedent names the rows, the
/// disjuncts name the cells.
fn extract_grid_closures(premises: &[ProofExpr]) -> Vec<GridClosure> {
    fn from_forall(e: &ProofExpr, out: &mut Vec<GridClosure>) {
        if let ProofExpr::ForAll { variable, body } = e {
            match body.as_ref() {
                ProofExpr::Implies(ante, cons) => {
                    let mut leaves = Vec::new();
                    flatten_or(cons, &mut leaves);
                    let disjuncts: Vec<(String, ProofExpr)> = leaves
                        .iter()
                        .filter_map(|d| disjunct_value(d, variable).map(|v| (v, (*d).clone())))
                        .collect();
                    if let Some(row_sort) = antecedent_sort(ante, variable) {
                        // Only a closure whose every disjunct is a clean predicate over the
                        // row var defines a column (guards against a stray non-grid ∀).
                        if !disjuncts.is_empty() && disjuncts.len() == leaves.len() {
                            out.push(GridClosure { var: variable.clone(), row_sort, disjuncts });
                        }
                    }
                }
                ProofExpr::ForAll { .. } => from_forall(body, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for p in premises {
        from_forall(&erase_tense(p), &mut out);
    }
    out
}

/// Title-case a lowercased sort name for a column/row header ("trip" → "Trip").
fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A human header for a category column: the declared sort whose domain covers the
/// column's values (case-insensitive), else a positional fallback.
fn grid_column_label(
    values: &[String],
    sorts: &std::collections::HashMap<String, Vec<ProofTerm>>,
    idx: usize,
) -> String {
    let want: Vec<String> = values.iter().map(|v| v.to_lowercase()).collect();
    let mut keys: Vec<&String> = sorts.keys().collect();
    keys.sort();
    for k in keys {
        let dom: std::collections::HashSet<String> = sorts[k]
            .iter()
            .filter_map(|t| match t {
                ProofTerm::Constant(c) => Some(c.to_lowercase()),
                _ => None,
            })
            .collect();
        if !dom.is_empty() && want.iter().all(|v| dom.contains(v)) {
            return title_case(k);
        }
    }
    format!("Category {}", idx + 1)
}

/// A coarse stem that bridges a gerund surface form and the base form it normalizes to
/// (`"Cycling"` and `"Cycle"` → `"cycl"`), so a grid value can be matched back to the word
/// the user actually wrote. Strips a trailing `-ing` then a trailing `-e`; leaves
/// already-base words (constants, years, names) effectively unchanged.
fn category_stem(s: &str) -> String {
    let mut w = s.to_lowercase();
    if w.len() > 5 && w.ends_with("ing") {
        w.truncate(w.len() - 3);
    }
    if w.len() > 3 && w.ends_with('e') {
        w.truncate(w.len() - 1);
    }
    w
}

/// Map each grid value back to the SURFACE word the user wrote. Category values
/// normalize to a base form in the FOL (a relation's constant like `Florida` survives, but
/// a gerund activity becomes `Cycle`); the original word ("cycling") lives only in the
/// input text. Key the input words by stem and recover them. First occurrence wins, so the
/// declaration's spelling is preferred.
fn surface_form_map(input: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for word in input.split(|c: char| !c.is_alphanumeric()) {
        if !word.is_empty() {
            map.entry(category_stem(word)).or_insert_with(|| word.to_string());
        }
    }
    map
}

/// Fill the grid from already-parsed premises: rows are the row sort's domain, columns
/// are the disjunctive closures, and each cell is the disjunct value the certified no-Z3
/// prover entails for that row (or `None` if undetermined). Same engine `answer_question`
/// uses per cell, swept across the whole table.
fn solve_grid_from_premises(premises: &[ProofExpr], input: &str) -> Option<SolvedGrid> {
    let closures = extract_grid_closures(premises);
    if closures.is_empty() {
        return None;
    }
    let untensed: Vec<ProofExpr> = premises.iter().map(erase_tense).collect();
    let sorts = logicaffeine_proof::grounding::sort_domains(&untensed);
    let row_sort = closures[0].row_sort.clone();
    let rows: Vec<String> = sorts
        .get(&row_sort)
        .map(|dom| {
            dom.iter()
                .filter_map(|t| match t {
                    ProofTerm::Constant(c) => Some(c.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        return None;
    }
    // Ground ONCE, with functionality, exactly as the fast certified grid path does — the
    // exclusion lemmas let RUP decide each non-entailed candidate in ~1ms.
    let prepared = prepare_premises_opts(premises, true);
    let surface = surface_form_map(input);
    let display = |v: &str| surface.get(&category_stem(v)).cloned().unwrap_or_else(|| v.to_string());
    let mut columns = Vec::new();
    for clo in &closures {
        if clo.row_sort != row_sort {
            continue;
        }
        let values: Vec<String> = clo.disjuncts.iter().map(|(v, _)| display(v)).collect();
        let mut cells = Vec::with_capacity(rows.len());
        for r in &rows {
            let mut found = None;
            for (label, dj) in &clo.disjuncts {
                let atom = erase_tense(
                    &logicaffeine_language::proof_convert::instantiate_var_with_constant(
                        dj, &clo.var, r,
                    ),
                );
                if candidate_entailed_prepared(&prepared, &atom) {
                    found = Some(display(label));
                    break;
                }
            }
            cells.push(found);
        }
        let label = grid_column_label(&values, &sorts, columns.len());
        columns.push(GridColumn { label, values, cells });
    }
    Some(SolvedGrid { row_label: title_case(&row_sort), rows, columns })
}

/// Prepare the premise set for the per-candidate solve, ONCE.
///
/// A logic grid is FINITE and STATIC, so we (1) ERASE TENSE — a grid is one static
/// scenario; the past-tense clue wrappers ("one WAS in Connecticut") carry no
/// information — and (2) GROUND the finite domain SORT-AWARE: each guarded quantifier
/// expands over its sort's declared domain (∀x(Trip(x)→…) over the four trips, not the
/// whole universe). The result is quantifier-free (decidable — neither our kernel nor
/// Z3 can instantiate ∀/∃ forever) and identical for every cell, so the per-cell
/// entailment check below sees a finite, decidable problem.
///
/// This is GENERAL over any declared finite-domain grid: the domains come from the
/// English declarations the parser produced (`sort_domains`), never from puzzle
/// knowledge. Grounding the KERNEL path (not just the Z3 path) is what lets the
/// certified prover close a full multi-category grid in the browser.
/// Does this premise set describe a finite-domain GRID — a declared bijection with a
/// closure (`∀x(Sort(x) → A ∨ B ∨ …)`) or an "exactly one" — as opposed to an open
/// syllogism? Only grids are grounded before proving (so Socrates keeps its
/// `UniversalInst` trace). Structural, never keyed to a particular puzzle.
fn looks_like_grid(premises: &[ProofExpr]) -> bool {
    fn has_disjunctive_closure(e: &ProofExpr) -> bool {
        match e {
            ProofExpr::ForAll { body, .. } => match body.as_ref() {
                ProofExpr::Implies(_, c) => matches!(c.as_ref(), ProofExpr::Or(..)),
                ProofExpr::ForAll { .. } => has_disjunctive_closure(body),
                _ => false,
            },
            ProofExpr::Temporal { body, .. } => has_disjunctive_closure(body),
            _ => false,
        }
    }
    !logicaffeine_proof::grounding::at_most_one_lemmas(premises).is_empty()
        || premises.iter().any(has_disjunctive_closure)
}

fn prepare_premises(premises: &[ProofExpr]) -> Vec<ProofExpr> {
    prepare_premises_opts(premises, false)
}

/// `prepare_premises`, optionally adding the bijection's FUNCTIONALITY half
/// (`∀x(Li → ¬Lj)`: a row takes at most one value per category). Functionality is
/// given ONLY to the incremental grid solver — it propagates the exclusions cheaply —
/// never to the re-saturating backward chainer, which it would slow to a crawl.
fn prepare_premises_opts(premises: &[ProofExpr], with_functionality: bool) -> Vec<ProofExpr> {
    let mut untensed: Vec<ProofExpr> = premises.iter().map(erase_tense).collect();
    // "Exactly one φ" ⟹ pairwise "at most one φ": the existence form grounds to a
    // disjunction the certified kernel cannot use directly; its entailed pairwise
    // uniqueness grounds to the conjunctive exclusion rules unit propagation runs on.
    let lemmas = logicaffeine_proof::grounding::at_most_one_lemmas(&untensed);
    untensed.extend(lemmas);
    // Definite-description clues ("the Florida trip is the hunting trip", "the trip with
    // Yvonne wasn't in Kentucky") name a unique row by a singleton value; rewrite them
    // to row-indexed implications the solver propagates, instead of an existential whose
    // grounding explodes to a row-disjunction.
    let defns = logicaffeine_proof::grounding::definite_property_implications(&untensed);
    untensed.extend(defns);
    if with_functionality {
        let func = logicaffeine_proof::grounding::functionality_lemmas(&untensed);
        untensed.extend(func);
        // HIDDEN SINGLE: the column closures (`⋁_r In(r, v)`) — the existence half of each
        // "exactly one" that `compile` otherwise drops. With functionality they let the
        // solver force a value's last remaining row, deepening the ROOT fixpoint so more
        // cells take the linear prove_var exit instead of search.
        let cols = logicaffeine_proof::grounding::column_closure_lemmas(&untensed);
        untensed.extend(cols);
    }
    let fallback = logicaffeine_proof::grounding::domain_constants(&untensed);
    let mut sorts = logicaffeine_proof::grounding::sort_domains(&untensed);
    bind_occasion_synonyms_to_row_domain(&untensed, &mut sorts);
    let grounded: Vec<ProofExpr> = untensed
        .iter()
        .map(|p| logicaffeine_proof::grounding::ground_sorted(p, &sorts, &fallback))
        .collect();
    // Discharge the now-true sort facts (`Trip(Alpha)`, …) so propagation runs on pure
    // value-literals: a closure becomes a bare disjunction, an at-most-one a one-step
    // exclusion rule. This is what keeps the full-grid search tractable.
    let discharged = logicaffeine_proof::grounding::discharge_unary_facts(&grounded);
    // Fold trivial reflexive identities: a grounded `∃x∃y` of-pair's diagonal
    // (`x = y = c`) carries `¬(c = c) = False`, so that disjunct drops.
    logicaffeine_proof::grounding::simplify_trivial_identities(&discharged)
}

/// Does the premise set entail the candidate answer goal?
///
/// Without `verification`: the KERNEL-CERTIFIED prover over the GROUNDED, tense-erased
/// grid. Grounding made the problem quantifier-free, so the kernel's case-split +
/// reductio strategies close each cell with a real proof term — no Z3, runs in WASM.
/// The bounded depth makes an unentailed cell fail fast rather than search to the hilt.
#[cfg(not(feature = "verification"))]
fn candidate_entailed_prepared(prepared: &[ProofExpr], goal: &ProofExpr) -> bool {
    let g = erase_tense(goal);
    // FAST certified tier first: the CDCL+RUP engine decides the grounded grid in ~1ms with
    // an independently-checked proof (and answers a NON-entailed candidate just as fast,
    // where the kernel path degenerates to a deep futile search). Fall back to the bounded
    // kernel chainer only when the problem isn't purely propositional (RUP returns `None`).
    match logicaffeine_proof::rup::entails_certified(prepared, &g) {
        Some(logicaffeine_proof::rup::Verdict::Entailed) => return true,
        Some(logicaffeine_proof::rup::Verdict::NotEntailed) => return false,
        None => {}
    }
    logicaffeine_proof::verify::prove_certify_check_bounded(prepared, &g, 100).verified
}

/// OCCASION soft-typing for the finite-domain solve.
///
/// In a logic grid the row entity is described by several OCCASION-sort head nouns
/// — "the hunting VACATION", "the Florida TRIP", "the 2004 HOLIDAY" — where the
/// modifier does the referring and the head is a soft type ([`Sort::is_occasion`],
/// the same principle [`drs::DiscourseModel::resolve_definite_by_modifier`] uses for
/// coreference). Only ONE of these head nouns is declared with a domain
/// (`Trip(Alpha)…`); the synonyms carry no domain, so an occasion guard like
/// `Vacation(x)` would ground over the WHOLE universe and a phantom non-row constant
/// (`Florida`, `2003`, …) could satisfy a synonym-headed clue. Point every
/// occasion-sorted guard at the shared row domain (the union of the declared
/// occasion-sort domains).
///
/// This is NOT global synonymy — no `Vacation ↔ Trip` axiom is asserted and "a work
/// trip" never collapses; the binding fires only for occasion-SORTED head predicates
/// inside the grounded solve, leaving ordinary parsing untouched.
fn bind_occasion_synonyms_to_row_domain(
    premises: &[ProofExpr],
    sorts: &mut std::collections::HashMap<String, Vec<logicaffeine_proof::ProofTerm>>,
) {
    use logicaffeine_language::lexicon::lookup_sort;
    use logicaffeine_proof::ProofTerm;
    let is_occasion = |n: &str| lookup_sort(n).map_or(false, |s| s.is_occasion());

    // The shared row domain: the union of every declared occasion-sort domain. In a
    // grid there is one row sort with a domain (`Trip`); its synonyms have none.
    let mut row_domain: Vec<ProofTerm> = Vec::new();
    for (name, dom) in sorts.iter() {
        if is_occasion(name) {
            for c in dom {
                if !row_domain.contains(c) {
                    row_domain.push(c.clone());
                }
            }
        }
    }
    if row_domain.is_empty() {
        return;
    }

    // Every occasion-sorted guard that appears in the premises but lacks its own
    // declared domain inherits the row domain.
    let mut names = HashSet::new();
    for p in premises {
        collect_unary_predicate_names(p, &mut names);
    }
    for name in names {
        if is_occasion(&name) {
            sorts.entry(name).or_insert_with(|| row_domain.clone());
        }
    }
}

/// Collect every UNARY predicate name appearing anywhere in `e` — the candidate sort
/// guards the occasion-binding inspects.
fn collect_unary_predicate_names(e: &ProofExpr, out: &mut HashSet<String>) {
    match e {
        ProofExpr::Predicate { name, args, .. } if args.len() == 1 => {
            out.insert(name.clone());
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            collect_unary_predicate_names(l, out);
            collect_unary_predicate_names(r, out);
        }
        ProofExpr::Counterfactual { antecedent, consequent } => {
            collect_unary_predicate_names(antecedent, out);
            collect_unary_predicate_names(consequent, out);
        }
        ProofExpr::Not(x)
        | ProofExpr::ForAll { body: x, .. }
        | ProofExpr::Exists { body: x, .. }
        | ProofExpr::Temporal { body: x, .. }
        | ProofExpr::Modal { body: x, .. } => collect_unary_predicate_names(x, out),
        _ => {}
    }
}

#[cfg(feature = "verification")]
fn candidate_entailed_prepared(prepared: &[ProofExpr], goal: &ProofExpr) -> bool {
    // The candidate goal is a ground predicate (no quantifiers) — just erase tense.
    let g = erase_tense(goal);
    // FAST certified tier first: the CDCL+RUP engine decides the grounded grid in ~1ms with
    // an independently-checked proof. Only if it can't encode the problem do we fall back to
    // the bounded kernel chainer, then Z3.
    match logicaffeine_proof::rup::entails_certified(prepared, &g) {
        Some(logicaffeine_proof::rup::Verdict::Entailed) => return true,
        Some(logicaffeine_proof::rup::Verdict::NotEntailed) => return false,
        None => {}
    }
    if logicaffeine_proof::verify::prove_certify_check_bounded(prepared, &g, 40).verified {
        return true;
    }
    matches!(
        logicaffeine_proof::oracle::oracle_entails(prepared, &g),
        logicaffeine_proof::oracle::SmtVerdict::Entailed
    )
}

/// Strip `Temporal` wrappers throughout a [`ProofExpr`] — a logic grid is one
/// static scenario, so tense carries no information and would otherwise block the
/// oracle from forcing values across past-tense clues.
fn erase_tense(e: &ProofExpr) -> ProofExpr {
    match e {
        ProofExpr::Temporal { body, .. } => erase_tense(body),
        ProofExpr::And(l, r) => {
            ProofExpr::And(Box::new(erase_tense(l)), Box::new(erase_tense(r)))
        }
        ProofExpr::Or(l, r) => ProofExpr::Or(Box::new(erase_tense(l)), Box::new(erase_tense(r))),
        ProofExpr::Implies(l, r) => {
            ProofExpr::Implies(Box::new(erase_tense(l)), Box::new(erase_tense(r)))
        }
        ProofExpr::Iff(l, r) => ProofExpr::Iff(Box::new(erase_tense(l)), Box::new(erase_tense(r))),
        ProofExpr::Not(x) => ProofExpr::Not(Box::new(erase_tense(x))),
        ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
            variable: variable.clone(),
            body: Box::new(erase_tense(body)),
        },
        ProofExpr::Exists { variable, body } => ProofExpr::Exists {
            variable: variable.clone(),
            body: Box::new(erase_tense(body)),
        },
        other => other.clone(),
    }
}

fn collect_constants(e: &ProofExpr, out: &mut Vec<String>) {
    use logicaffeine_proof::ProofTerm;
    fn term(t: &ProofTerm, out: &mut Vec<String>) {
        match t {
            ProofTerm::Constant(s) => out.push(s.clone()),
            ProofTerm::Function(_, args) | ProofTerm::Group(args) => {
                args.iter().for_each(|a| term(a, out))
            }
            _ => {}
        }
    }
    match e {
        ProofExpr::Predicate { args, .. } => args.iter().for_each(|a| term(a, out)),
        ProofExpr::Identity(a, b) => {
            term(a, out);
            term(b, out);
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            collect_constants(l, out);
            collect_constants(r, out);
        }
        ProofExpr::Not(x) => collect_constants(x, out),
        ProofExpr::ForAll { body, .. }
        | ProofExpr::Exists { body, .. }
        | ProofExpr::Temporal { body, .. } => collect_constants(body, out),
        ProofExpr::Term(t) => term(t, out),
        _ => {}
    }
}

/// Lower a `## Define` block (Rung 0a) to a proof-layer definition: the LHS
/// predicate supplies the (normalized) definiendum name and parameter symbols,
/// the RHS is the definiens. Both sides pass through the SAME
/// `logic_expr_to_proof_expr` + interner as theorem premises and goals, so the
/// definiendum name and parameters match the predicate occurrences the prover
/// sees. Returns `None` if the LHS is not a predicate application.
fn lower_definition(
    def: &logicaffeine_language::ast::DefinitionBlock,
    interner: &Interner,
) -> Option<logicaffeine_proof::verify::Definition> {
    use logicaffeine_language::proof_convert::logic_expr_to_proof_expr;
    let (name, params) = match logic_expr_to_proof_expr(def.definiendum, interner) {
        ProofExpr::Predicate { name, args, .. } => {
            let params = args
                .iter()
                .filter_map(|t| match t {
                    ProofTerm::Constant(n) | ProofTerm::Variable(n) => Some(n.clone()),
                    _ => None,
                })
                .collect();
            (name, params)
        }
        _ => return None,
    };
    let definiens = logic_expr_to_proof_expr(def.definiens, interner);
    Some(logicaffeine_proof::verify::Definition {
        name,
        params,
        definiens,
    })
}

/// [`theorem_proof_exprs`] with optional DEFEASIBLE conversion: premises keep
/// their generics/implicatures as abnormality-guarded defaults (returned for
/// the circumscription pass); the goal always converts strictly. Also returns
/// the document's `## Define` blocks lowered to proof-layer definitions (Rung 0a).
fn theorem_problem(
    input: &str,
    defeasible: bool,
) -> Result<
    (
        Vec<ProofExpr>,
        ProofExpr,
        Vec<logicaffeine_language::proof_convert::DefaultRule>,
        Vec<logicaffeine_proof::verify::Definition>,
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

    // Rung 0a: collect every `## Define` block in the document, lowered to a
    // proof-layer definition the prover can δ-unfold.
    let definitions: Vec<logicaffeine_proof::verify::Definition> = statements
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::Definition(d) => lower_definition(d, &interner),
            _ => None,
        })
        .collect();

    Ok((proof_exprs, goal_expr, defaults, definitions))
}
