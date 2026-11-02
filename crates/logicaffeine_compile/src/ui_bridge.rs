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
    analysis::DiscoveryPass,
    arena_ctx::AstContext,
    ast::{self, LogicExpr, Term},
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
        TokenType::All
        | TokenType::Some
        | TokenType::No
        | TokenType::Any
        | TokenType::Most
        | TokenType::Few
        | TokenType::Many
        | TokenType::Cardinal(_)
        | TokenType::AtLeast(_)
        | TokenType::AtMost(_) => TokenCategory::Quantifier,
        TokenType::Noun(_) => TokenCategory::Noun,
        TokenType::Verb { .. } => TokenCategory::Verb,
        TokenType::Adjective(_) | TokenType::NonIntersectiveAdjective(_) => {
            TokenCategory::Adjective
        }
        TokenType::And
        | TokenType::Or
        | TokenType::Not
        | TokenType::If
        | TokenType::Then
        | TokenType::Iff
        | TokenType::Because => TokenCategory::Connective,
        TokenType::Article(_) => TokenCategory::Determiner,
        TokenType::Preposition(_) => TokenCategory::Preposition,
        TokenType::Pronoun { .. } => TokenCategory::Pronoun,
        TokenType::Must
        | TokenType::Can
        | TokenType::Should
        | TokenType::Shall
        | TokenType::Would
        | TokenType::Could
        | TokenType::May
        | TokenType::Cannot
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

    tokens
        .iter()
        .map(|t| TokenInfo {
            start: t.span.start,
            end: t.span.end,
            text: input[t.span.start..t.span.end].to_string(),
            category: categorize_token(&t.kind, &interner),
        })
        .collect()
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
/// Every variant gets a node in the notation the Logic Output prints — the operator symbols
/// come from the one table in `logicaffeine_language::debug` — with its sub-expressions and
/// terms as children. The match has no fallback arm, so a new variant cannot reach the tree
/// as a `Debug` dump: it must be given a node here.
pub fn expr_to_ast_node(expr: &LogicExpr, interner: &Interner) -> AstNode {
    use logicaffeine_language::debug::{
        aspect_symbol, binary_temporal_symbol, connective_symbol, focus_symbol, modal_symbol,
        quantifier_symbol, temporal_symbol, voice_symbol, DisplayWith,
    };
    let node = |e: &LogicExpr| expr_to_ast_node(e, interner);
    let term = |t: &Term| term_to_ast_node(t, interner);
    let name = |sym: &logicaffeine_base::Symbol| interner.resolve(*sym).to_string();
    // A node with no sub-expression to open: labelled in the notation itself.
    let written = |node_type: &str| AstNode::leaf(&expr.with(interner).to_string(), node_type);
    match expr {
        LogicExpr::Predicate {
            name: pred, args, ..
        } => AstNode::with_children(
            &format!("{}({})", name(pred), args.len()),
            "predicate",
            args.iter().map(|t| term(t)).collect(),
        ),
        LogicExpr::Identity { left, right } => {
            AstNode::with_children("=", "identity", vec![term(left), term(right)])
        }
        LogicExpr::Metaphor { tenor, vehicle } => {
            AstNode::with_children("Metaphor", "metaphor", vec![term(tenor), term(vehicle)])
        }
        LogicExpr::Quantifier {
            kind,
            variable,
            body,
            ..
        } => AstNode::with_children(
            &format!("{}{}", quantifier_symbol(kind), name(variable)),
            "quantifier",
            vec![node(body)],
        ),
        LogicExpr::Categorical(_) => written("categorical"),
        LogicExpr::Relation(_) => written("relation"),
        LogicExpr::Modal { vector, operand } => {
            AstNode::with_children(modal_symbol(vector), "modal", vec![node(operand)])
        }
        LogicExpr::Temporal { operator, body } => {
            AstNode::with_children(temporal_symbol(operator), "temporal", vec![node(body)])
        }
        LogicExpr::TemporalBinary {
            operator,
            left,
            right,
        } => AstNode::with_children(
            binary_temporal_symbol(operator),
            "temporal",
            vec![node(left), node(right)],
        ),
        LogicExpr::Aspectual { operator, body } => {
            AstNode::with_children(aspect_symbol(operator), "aspect", vec![node(body)])
        }
        LogicExpr::Voice { operator, body } => {
            AstNode::with_children(voice_symbol(operator), "voice", vec![node(body)])
        }
        LogicExpr::BinaryOp { left, op, right } => AstNode::with_children(
            connective_symbol(op),
            "binary_op",
            vec![node(left), node(right)],
        ),
        LogicExpr::UnaryOp { op, operand } => {
            AstNode::with_children(connective_symbol(op), "unary_op", vec![node(operand)])
        }
        LogicExpr::Question { wh_variable, body } => AstNode::with_children(
            &format!("?{}", name(wh_variable)),
            "question",
            vec![node(body)],
        ),
        LogicExpr::YesNoQuestion { body } => {
            AstNode::with_children("?", "question", vec![node(body)])
        }
        LogicExpr::Atom(sym) => AstNode::leaf(&name(sym), "atom"),
        LogicExpr::Lambda { variable, body } => {
            AstNode::with_children(&format!("λ{}", name(variable)), "lambda", vec![node(body)])
        }
        LogicExpr::App { function, argument } => {
            AstNode::with_children("apply", "application", vec![node(function), node(argument)])
        }
        LogicExpr::Intensional { operator, content } => {
            AstNode::with_children(&name(operator), "intensional", vec![node(content)])
        }
        LogicExpr::Event { predicate, adverbs } => {
            let mut children = vec![node(predicate)];
            children.extend(adverbs.iter().map(|a| AstNode::leaf(&name(a), "modifier")));
            AstNode::with_children("event", "event", children)
        }
        // A Neo-Davidsonian event: its verb over the event variable, each thematic role a
        // branch holding its filler, and each adverbial modifier a leaf.
        LogicExpr::NeoEvent(data) => {
            let mut children: Vec<AstNode> = data
                .roles
                .iter()
                .map(|(role, filler)| {
                    AstNode::with_children(&format!("{:?}", role), "role", vec![term(filler)])
                })
                .collect();
            children.extend(
                data.modifiers
                    .iter()
                    .map(|m| AstNode::leaf(&name(m), "modifier")),
            );
            AstNode::with_children(
                &format!("∃{} {}", name(&data.event_var), name(&data.verb)),
                "event",
                children,
            )
        }
        LogicExpr::Imperative { action } => {
            AstNode::with_children("!", "speech_act", vec![node(action)])
        }
        LogicExpr::Exclamative { body, .. } => {
            AstNode::with_children("Exclaim", "speech_act", vec![node(body)])
        }
        LogicExpr::Optative { wish } => {
            AstNode::with_children("Wish", "speech_act", vec![node(wish)])
        }
        LogicExpr::Implicature {
            assertion,
            implicature,
        } => AstNode::with_children(
            "+>",
            "implicature",
            vec![node(assertion), node(implicature)],
        ),
        LogicExpr::SpeechAct {
            performer,
            act_type,
            content,
        } => AstNode::with_children(
            &format!("{}!{}", name(act_type), name(performer)),
            "speech_act",
            vec![node(content)],
        ),
        LogicExpr::Counterfactual {
            antecedent,
            consequent,
        } => AstNode::with_children(
            "□→",
            "conditional",
            vec![node(antecedent), node(consequent)],
        ),
        LogicExpr::Causal { effect, cause } => {
            AstNode::with_children("Cause", "causal", vec![node(cause), node(effect)])
        }
        LogicExpr::Concessive { main, concession } => AstNode::with_children(
            "Concessive",
            "concessive",
            vec![node(main), node(concession)],
        ),
        LogicExpr::Comparative { .. } => written("comparative"),
        LogicExpr::Superlative { .. } => written("superlative"),
        LogicExpr::Scopal { operator, body } => {
            AstNode::with_children(&name(operator), "scopal", vec![node(body)])
        }
        LogicExpr::Control {
            verb,
            subject,
            object,
            infinitive,
        } => {
            let mut children = vec![term(subject)];
            children.extend(object.map(|o| term(o)));
            children.push(node(infinitive));
            AstNode::with_children(&name(verb), "control", children)
        }
        LogicExpr::Presupposition {
            assertion,
            presupposition,
        } => AstNode::with_children(
            "presupposes",
            "presupposition",
            vec![node(assertion), node(presupposition)],
        ),
        LogicExpr::Focus {
            kind,
            focused,
            scope,
        } => AstNode::with_children(
            focus_symbol(kind),
            "focus",
            vec![term(focused), node(scope)],
        ),
        LogicExpr::TemporalAnchor { anchor, body } => {
            AstNode::with_children(&format!("@{}", name(anchor)), "temporal", vec![node(body)])
        }
        LogicExpr::Distributive { predicate } => {
            AstNode::with_children("*", "distributive", vec![node(predicate)])
        }
        LogicExpr::GroupQuantifier {
            group_var,
            count,
            restriction,
            body,
            ..
        } => AstNode::with_children(
            &format!("∃{} (group of {})", name(group_var), count),
            "quantifier",
            vec![node(restriction), node(body)],
        ),
    }
}

fn term_to_ast_node(term: &Term, interner: &Interner) -> AstNode {
    match term {
        Term::Constant(sym) => AstNode::leaf(interner.resolve(*sym), "constant"),
        Term::Variable(sym) => AstNode::leaf(interner.resolve(*sym), "variable"),
        Term::Function(name, args) => {
            let name_str = interner.resolve(*name);
            let arg_nodes: Vec<AstNode> =
                args.iter().map(|t| term_to_ast_node(t, interner)).collect();
            AstNode::with_children(&format!("{}()", name_str), "function", arg_nodes)
        }
        Term::Group(terms) => {
            let term_nodes: Vec<AstNode> = terms
                .iter()
                .map(|t| term_to_ast_node(t, interner))
                .collect();
            AstNode::with_children("⊕", "group", term_nodes)
        }
        // A proposition used as a term (`believes that …`): its own structure beneath it.
        Term::Proposition(expr) => {
            AstNode::with_children("[ ]", "proposition", vec![expr_to_ast_node(expr, interner)])
        }
        _ => {
            use logicaffeine_language::debug::DisplayWith;
            AstNode::leaf(&term.with(interner).to_string(), "term")
        }
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
        let raw = compile_forest_with_options(
            input,
            CompileOptions {
                format: OutputFormat::SimpleFOL,
                pragmatic: false,
            },
        );
        let mut seen = HashSet::new();
        raw.into_iter().filter(|r| seen.insert(r.clone())).collect()
    };

    // Generate Kripke readings with explicit world quantification
    let kripke_readings = compile_forest_with_options(
        input,
        CompileOptions {
            format: OutputFormat::Kripke,
            pragmatic: false,
        },
    );

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
    let mut parser = Parser::new(
        lex_tokens,
        &mut world_state,
        &mut interner,
        ctx,
        type_registry,
    );

    match parser.parse() {
        Ok(ast) => {
            let ast = semantics::apply_axioms(ast, ctx.exprs, ctx.terms, &mut interner);
            let ast = pragmatics::apply_pragmatics(ast, ctx.exprs, &interner);
            let ast_node = expr_to_ast_node(ast, &interner);
            let mut registry = SymbolRegistry::new();
            let logic = ast.transpile_discourse(&mut registry, &interner, OutputFormat::Unicode);
            let simple_logic =
                ast.transpile_discourse(&mut registry, &interner, OutputFormat::SimpleFOL);

            let kripke_ast =
                semantics::apply_kripke_lowering(ast, ctx.exprs, ctx.terms, &mut interner);
            let kripke_logic =
                kripke_ast.transpile_discourse(&mut registry, &interner, OutputFormat::Kripke);

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
    let mut parser = Parser::new(
        lex_tokens,
        &mut world_state,
        &mut interner,
        ctx,
        type_registry,
    );

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
    /// How many obligations the DECLARED method left open and the automation closed.
    ///
    /// `0` means the author's own proof was complete — and also that there was no proof at
    /// all: a count is a fact about a certificate, so an unverified theorem reports 0, exactly
    /// as the prose door annotates only its certified arm. The TEXT door
    /// (`logicaffeine_language::compile_theorem`) has always said this in prose — "Proved!
    /// \[kernel-verified\] (2 residual obligations discharged by automation)" — and this door
    /// silently dropped it, so two Surface A doors reported the same `verified: true` with
    /// different amounts of disclosure. A `Proof:` line that declares a METHOD and closes
    /// nothing (`By induction on n.` on a goal needing no induction) reaches `verified: true`
    /// here, and this is the field that says the machine, not the author, did the work.
    ///
    /// It is disclosure, never trust: every swept obligation is assembled into the SAME
    /// derivation and certified through `qed` → `check_derivation` → the kernel, so
    /// `verified` means exactly what it meant before.
    pub discharged: usize,
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

/// The `Proof:` line a `## Theorem` block declared, lowered to what a driver needs.
///
/// There is ONE lowering of [`ProofStrategy`](logicaffeine_language::ast::theorem::ProofStrategy)
/// in this crate, and it lives in [`declared_proof`]. Every Surface A door reads the declared
/// method from here, which is the invariant that keeps two doors from reaching opposite
/// verdicts on the same input.
#[derive(Debug, Clone, Default)]
struct DeclaredProof {
    /// The tactic-script source to run, or `None` for `Auto` (automatic proof search).
    script: Option<String>,
    /// The INFERENCE-RULE name a one-word `Proof:` line gave (`ProofStrategy::ByRule`).
    /// Only the diagnostic depends on it: a word the tactic namespace does not carry is
    /// reported as a rule name rather than as a bad tactic.
    rule_name: Option<String>,
    /// `Given (h): …` names, parallel to the premises, so a script can cite `h`.
    premise_names: Vec<Option<String>>,
}

/// Lower a theorem block's declared proof strategy. EXHAUSTIVE on purpose: a `_ => None`
/// arm turned every non-`Auto`, non-`Script` strategy into a third, undefined state — "not
/// auto, no script" — which reported the theorem as unproved with no error at all.
fn declared_proof(theorem: &ast::theorem::TheoremBlock<'_>) -> DeclaredProof {
    use ast::theorem::ProofStrategy;
    let (script, rule_name) = match &theorem.strategy {
        ProofStrategy::Auto => (None, None),
        ProofStrategy::Script(s) => (Some(s.clone()), None),
        ProofStrategy::ByRule(rule) => (Some(rule.clone()), Some(rule.clone())),
        ProofStrategy::Induction(variable) => (Some(format!("induction {variable}")), None),
    };
    DeclaredProof {
        script,
        rule_name,
        premise_names: theorem.premise_names.clone(),
    }
}

/// Prove a Surface A theorem BY THE METHOD ITS AUTHOR DECLARED.
///
/// `verify_theorem`, [`prove_theorem_trace`] and [`theorem_certificate`] used to run
/// automatic proof search unconditionally, ignoring the `Proof:` line entirely. A theorem
/// whose declared proof was a tactic script was therefore certified by a search nobody
/// wrote; worse, a theorem whose script is BROKEN still got a kernel term — and a portable
/// CERTIFICATE — from these doors while `compile_theorem` failed loudly on the same input.
/// Two doors, opposite verdicts, and no test between them.
///
/// `Auto` keeps the definition-aware automatic path. A declared script runs through the one
/// vernacular driver. The two cannot be combined: `ProofState` carries no `## Define`
/// environment, so a script plus definitions is refused rather than proved with the
/// definitions silently dropped.
///
/// The second half of the answer is how many obligations the declared method LEFT and the
/// automation closed — see [`TheoremCompileResult::discharged`]. It travels with the verdict
/// rather than beside it, because a door that has the number and does not report it is how
/// the two Surface A doors came to disclose different amounts about the same proof.
fn prove_declared(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    definitions: &[logicaffeine_proof::verify::Definition],
    declared: &DeclaredProof,
) -> (logicaffeine_proof::verify::VerifiedProof, usize) {
    let Some(script) = declared.script.as_deref() else {
        return (
            logicaffeine_proof::verify::prove_certify_check_with_defs(premises, goal, definitions),
            0,
        );
    };
    if !definitions.is_empty() {
        return (
            logicaffeine_proof::verify::VerifiedProof {
                derivation: None,
                proof_term: None,
                kernel_ctx: Default::default(),
                verified: false,
                verification_error: Some(
                    "a `Proof:` tactic script cannot see this document's `## Define` blocks — \
                 the tactic state carries no definition environment. Either prove this \
                 theorem with `Proof: Auto.` (which δ-unfolds the definitions) or inline \
                 the definition"
                        .to_string(),
                ),
            },
            0,
        );
    }
    logicaffeine_language::run_proof_sketch(
        premises,
        &declared.premise_names,
        goal,
        script,
        declared.rule_name.as_deref(),
    )
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
    /// An explicit `Proof:` tactic script (English-esque vernacular), if the block
    /// gave one instead of `Auto`. Run by the tactic framework, kernel-certified.
    script: Option<String>,
    /// The INFERENCE-RULE name a one-word `Proof:` line gave
    /// (`ProofStrategy::ByRule`). Only the diagnostic depends on it: a word the tactic
    /// namespace does not carry is reported as a rule name rather than as a bad tactic.
    rule_name: Option<String>,
    /// Optional names for the premises (`Given (h): …`), parallel to `premises`, so a
    /// script can refer to a premise as `h` rather than `hp0`.
    premise_names: Vec<Option<String>>,
    /// Whether the goal asks ("Who is in Florida?") rather than claims ("Some dog barks."):
    /// a question is answered by its witnesses, a claim is proved.
    asks: bool,
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

    let statements = parser.parse_program().map_err(|e| {
        format!(
            "Parse error: {}",
            socratic_explanation(&e, &Interner::new())
        )
    })?;

    let theorem = statements
        .iter()
        .find_map(|stmt| {
            if let ast::Stmt::Theorem(t) = stmt {
                Some(t)
            } else {
                None
            }
        })
        .ok_or_else(|| "No theorem block found".to_string())?;

    let premises: Vec<ProofExpr> = theorem
        .premises
        .iter()
        .map(|p| logic_expr_to_proof_expr(p, &interner))
        .collect();
    let goal = logic_expr_to_proof_expr(theorem.goal, &interner);

    let mut registry = SymbolRegistry::new();
    let goal_string = theorem
        .goal
        .transpile(&mut registry, &interner, OutputFormat::SimpleFOL);

    let DeclaredProof {
        script,
        rule_name,
        premise_names,
    } = declared_proof(theorem);

    Ok(ParsedTheorem {
        name: theorem.name.clone(),
        premises,
        goal,
        goal_string,
        is_auto: matches!(theorem.strategy, ast::theorem::ProofStrategy::Auto),
        script,
        rule_name,
        premise_names,
        asks: matches!(theorem.goal, ast::LogicExpr::Question { .. }),
    })
}

/// A single theorem's verdict within a compiled formal development.
#[derive(Debug, Clone)]
pub struct TheoryTheoremResult {
    pub name: String,
    pub verified: bool,
    pub error: Option<String>,
}

/// The result of compiling a program's formal development — its `## Axiom` declarations
/// (a shared premise base) plus the theorems of its `## Theory` block, each discharged in
/// citation order and kernel-certified by the multi-theorem driver.
#[derive(Debug, Clone)]
pub struct TheoryCompileResult {
    pub theory_name: Option<String>,
    pub axiom_count: usize,
    /// The NAMES of every admitted axiom (standalone `## Axiom <name>` blocks and
    /// `Axiom <name>` declarations in a `## Theory` body), in discovery order.
    /// The inventory needs these to report axioms as named assumptions and to
    /// reject a theorem/axiom name collision (G13-full) — a count alone cannot.
    pub axiom_names: Vec<String>,
    pub theorems: Vec<TheoryTheoremResult>,
    pub parse_error: Option<String>,
}

impl TheoryCompileResult {
    /// Whether every theorem in the development was kernel-certified (and it parsed).
    pub fn all_verified(&self) -> bool {
        self.parse_error.is_none()
            && !self.theorems.is_empty()
            && self.theorems.iter().all(|t| t.verified)
    }
}

/// Compile a program's formal development end to end: collect every `## Axiom` (a shared
/// premise base) and every `## Theory` block's `Axiom`/`Theorem` declarations, then prove
/// the theorems in citation order, each kernel-certified by the multi-theorem driver. This
/// is the seam that turns a Tarski-style source file into a verified development.
pub fn compile_theory_for_ui(input: &str) -> TheoryCompileResult {
    use logicaffeine_proof::development::parse_development;
    use logicaffeine_proof::formula::parse_formula;
    use logicaffeine_proof::verify::{prove_library_with_axioms, LibraryTheorem};

    // Parse the program with the same wiring as `parse_theorem`.
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
        Ok(s) => s,
        Err(e) => {
            return TheoryCompileResult {
                theory_name: None,
                axiom_count: 0,
                axiom_names: Vec::new(),
                theorems: Vec::new(),
                parse_error: Some(format!(
                    "Parse error: {}",
                    socratic_explanation(&e, &Interner::new())
                )),
            }
        }
    };

    // Collect the development: standalone `## Axiom` blocks + `## Theory` bodies.
    let mut axioms: Vec<ProofExpr> = Vec::new();
    let mut axiom_names: Vec<String> = Vec::new();
    let mut theorems: Vec<LibraryTheorem> = Vec::new();
    let mut theory_name: Option<String> = None;

    for stmt in &statements {
        match stmt {
            ast::Stmt::Axiom(a) => match parse_formula(&a.formula) {
                Ok(expr) => {
                    axioms.push(expr);
                    axiom_names.push(a.name.clone());
                }
                Err(e) => {
                    return TheoryCompileResult {
                        theory_name,
                        axiom_count: axioms.len(),
                        axiom_names,
                        theorems: Vec::new(),
                        parse_error: Some(format!("Axiom '{}': {}", a.name, e)),
                    }
                }
            },
            ast::Stmt::Theory(t) => {
                if theory_name.is_none() {
                    theory_name = Some(t.name.clone());
                }
                if !t.body.trim().is_empty() {
                    match parse_development(&t.body) {
                        Ok(dev) => {
                            axiom_names.extend(dev.axioms.iter().map(|(n, _)| n.clone()));
                            axioms.extend(dev.axiom_exprs());
                            theorems.extend(dev.theorems);
                        }
                        Err(e) => {
                            return TheoryCompileResult {
                                theory_name,
                                axiom_count: axioms.len(),
                                axiom_names,
                                theorems: Vec::new(),
                                parse_error: Some(format!("Theory '{}': {}", t.name, e)),
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let results = prove_library_with_axioms(&axioms, &theorems);
    let theorem_results = theorems
        .iter()
        .zip(results)
        .map(|(t, r)| TheoryTheoremResult {
            name: t.name.clone(),
            verified: r.verified,
            error: r.verification_error,
        })
        .collect();

    TheoryCompileResult {
        theory_name,
        axiom_count: axioms.len(),
        axiom_names,
        theorems: theorem_results,
        parse_error: None,
    }
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
                discharged: 0,
            };
        }
    };
    let ParsedTheorem {
        name,
        premises,
        goal,
        goal_string,
        is_auto,
        script,
        rule_name,
        premise_names,
        asks,
    } = parsed;

    // The easter egg: when the premises take the grid form (declared bijection +
    // disjunctive closures), fill every cell the certified prover can force — no flag,
    // the structure alone triggers it. Attached to every grid result so the studio can
    // render the solved table alongside the headline goal.
    let grid = if is_auto && looks_like_grid(&premises) {
        solve_grid_from_premises(&premises, input)
    } else {
        None
    };

    // How many obligations the DECLARED method left for the automation. Only the declared-
    // script route can leave any: `Auto` IS the automation, so its count is 0 by construction.
    let mut discharged = 0usize;
    let (derivation, verified, verification_error, answer) = if is_auto {
        // A wh-question goal ("Who is in Florida?") is an open ∃-form; the closed-goal
        // prover cannot discharge it and grinds a deep, futile search. Recognize the
        // question and ANSWER it by enumerating witnesses through the same certified no-Z3
        // path the puzzle uses — each candidate proved (or refuted) in ~1ms. A claim that
        // merely has an ∃ ("Some dog barks.") is proved like any other goal.
        if let (true, ProofExpr::Exists { variable, body }) = (asks, &goal) {
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
                        eprintln!(
                            "[grid] solve+emit {:.2?} ({} tree nodes)",
                            t_solve.elapsed(),
                            n
                        );
                    }
                    let t_cert = trace.then(std::time::Instant::now);
                    let solved = tree
                        .map(|tree| {
                            logicaffeine_proof::verify::check_derivation(&solver_input, &g, tree)
                        })
                        .filter(|vp| vp.verified);
                    if let Some(t_cert) = t_cert {
                        eprintln!(
                            "[grid] kernel-certify {:.2?} (verified={})",
                            t_cert.elapsed(),
                            solved.is_some()
                        );
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
                            logicaffeine_proof::verify::prove_certify_check_bounded(
                                &grounded, &g, 60,
                            )
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
            (
                outcome.derivation,
                outcome.verified,
                outcome.verification_error,
                None,
            )
        }
    } else if script.is_some() {
        // An explicit `Proof:` line (English-esque vernacular): the SAME declared-method
        // driver every other Surface A door uses, run on a generous stack and
        // kernel-certified. (This door parses no `## Define` blocks, so its definition
        // environment is empty by construction.)
        let declared = DeclaredProof {
            script,
            rule_name,
            premise_names,
        };
        let run = || {
            let (vp, swept) = prove_declared(&premises, &goal, &[], &declared);
            (vp.derivation, vp.verified, vp.verification_error, swept)
        };
        #[cfg(not(target_arch = "wasm32"))]
        let (derivation, verified, verr, swept) = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(256 * 1024 * 1024)
                .spawn_scoped(s, run)
                .expect("spawn proof-script thread")
                .join()
                .expect("proof-script thread panicked")
        });
        #[cfg(target_arch = "wasm32")]
        let (derivation, verified, verr, swept) = run();
        // A count is a fact about a PROOF. A sketch that did not certify has none, and
        // the prose door annotates only the certified arm — so reporting the sweep of a
        // failed run would make the two doors disagree in the other direction.
        discharged = if verified { swept } else { 0 };
        (derivation, verified, verr, None)
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
        discharged,
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
    let proven = match math_src.as_deref() {
        Some(math) => mixed_proven_module(math)?,
        None => None,
    };
    generate_rust_code_with_proven(&imperative_src, proven.as_deref())
}

/// [`generate_rust_code`] with an already-extracted proven module bundled in.
#[cfg(feature = "codegen")]
pub fn generate_rust_code_with_proven(
    source: &str,
    proven: Option<&str>,
) -> Result<String, ParseError> {
    use logicaffeine_language::ast::stmt::{Expr, Stmt, TypeExpr};
    let library = crate::loader::LibraryProvenance::of(source);

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
    let codegen_policies = policy_registry.clone();

    let mut world_state = drs::WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let _compile_facts = crate::optimize::facts::scope();
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

    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program()?;
    let opt_flags = parser.program_opt_flags();
    let codegen_registry = parser.into_type_registry();
    let mut run_cfg =
        crate::optimization::OptimizationConfig::from_env().merged(&opt_flags);
    run_cfg.normalize();
    let resolved = crate::resolve_program::resolve_program(
        &stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &mut interner,
        run_cfg.is_on(crate::optimization::Opt::Comptime),
        library.functions(),
    );
    let stmts: Vec<Stmt> = match resolved {
        Some(rw) => rw.to_vec(),
        None => stmts,
    };

    let type_env =
        crate::analysis::types::TypeEnv::infer_program(&stmts, &interner, &codegen_registry);
    let rust_code = crate::codegen::codegen_program_with_proven(
        &stmts,
        &codegen_registry,
        &codegen_policies,
        &interner,
        &type_env,
        &crate::optimization::OptimizationConfig::from_env(),
        "proven",
        proven,
    );
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
fn send_escape_rejection(
    stmts: &[logicaffeine_language::ast::stmt::Stmt],
) -> Option<InterpreterResult> {
    crate::concurrency::check_send_escape(stmts)
        .first()
        .map(|d| InterpreterResult {
            lines: Vec::new(),
            error: Some(d.message.clone()),
        })
}

/// Reject a dimension-incoherent program (`2 meters + 1 gram`) BEFORE it runs, so the interpreter
/// tier matches the AOT compiler: a dimension mismatch is an analysis error on every tier, surfaced
/// before any output is produced — not a mid-execution failure. Mirrors `send_escape_rejection`.
fn send_dimension_rejection(
    stmts: &[logicaffeine_language::ast::stmt::Stmt],
    interner: &Interner,
) -> Option<InterpreterResult> {
    crate::analysis::dimension_check::DimensionChecker::new(interner)
        .check_program(stmts)
        .err()
        .map(|e| InterpreterResult {
            lines: Vec::new(),
            error: Some(e.message),
        })
}

pub async fn interpret_for_ui(input: &str) -> InterpreterResult {
    interpret_for_ui_with_args(input, &[]).await
}

/// Each run the IDE, a guide, an exercise or the REPL starts is a page load of its own: the document
/// its program paints starts empty, as a browser's does for a page it loads, so the nodes one run's
/// paint names are never taken for nodes an earlier run left behind.
fn begin_ui_run() {
    crate::semantics::dom::reset();
}

/// The debug shadow oracle runs the program a second time. It paints a document of its own, so the
/// run the caller sees keeps the page its own engine drew, and the two runs' pages never meet.
#[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
fn in_a_document_of_its_own<T>(run: impl FnOnce() -> T) -> T {
    let shown = crate::semantics::dom::take();
    let out = run();
    crate::semantics::dom::restore(shown);
    out
}

/// Like [`interpret_for_ui`], but supplies the program's argument vector to the
/// `args()` system native. `program_args` is the full argv (index 0 is the
/// program name), matching the compiled binary's `env::args()`.
pub async fn interpret_for_ui_with_args(input: &str, program_args: &[String]) -> InterpreterResult {
    begin_ui_run();
    // The synchronous dispatcher covers every non-async program; for async
    // ones it uses block_on, which would nest inside this future — so handle
    // the async case here with a real await.
    let needs_async = with_parsed_program(input, |parsed, interner| match parsed {
        Ok((stmts, _, _)) => crate::interpreter::needs_async(stmts, interner),
        Err(_) => false,
    });
    if needs_async && matches!(engine(), Engine::Jit | Engine::Vm) {
        if std::env::var_os("LOGOS_ENGINE_TRACE").is_some() {
            eprintln!("logos-engine: refused");
        }
        return InterpreterResult {
            lines: Vec::new(),
            error: Some(
                "the selected VM/JIT tier is sync-only; this program requires async execution"
                    .to_string(),
            ),
        };
    }
    if !needs_async {
        return interpret_for_ui_sync_with_args(input, program_args);
    }

    // Match the sync path's preprocessing: wrap a bare script as `## Main`, then
    // demand-import the stdlib modules the program references. Without this an
    // async handler that calls a prelude function (http.lg's `plain`, net.lg's
    // `Message`, …) fails at runtime with "Unknown function".
    let implicit = implicit_main(input);
    let input = implicit.as_deref().unwrap_or(input);
    let prelude_src = crate::loader::apply_prelude(input);
    let input = prelude_src.as_ref();
    let library = crate::loader::LibraryProvenance::of(input);

    use logicaffeine_language::ast::stmt::{Expr, Stmt, TypeExpr};

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let mut tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    // Resolve the qualified escape hatch for cross-module calls (`Alias::fn` →
    // bare `fn`); no-op for single-file programs.
    let functions = crate::analysis::assemble::defined_function_names(input);
    crate::analysis::assemble::resolve_qualified_names(&mut tokens, &mut interner, &functions);

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
    let _compile_facts = crate::optimize::facts::scope();
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
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
    let parsed = parser.parse_program();
    let opt_flags = parser.program_opt_flags();
    let type_registry_for_interp = parser.into_type_registry();

    match parsed {
        Ok(stmts) => {
            let mut run_cfg =
                crate::optimization::OptimizationConfig::from_env().merged(&opt_flags);
            run_cfg.normalize();
            let resolved = crate::resolve_program::resolve_program(
                &stmts,
                crate::ast_map::Arenas {
                    exprs: &imperative_expr_arena,
                    stmts: &stmt_arena,
                    types: &type_expr_arena,
                },
                &mut interner,
                run_cfg.is_on(crate::optimization::Opt::Comptime),
                library.functions(),
            );
            let stmts: Vec<Stmt> = match resolved {
                Some(rw) => rw.to_vec(),
                None => stmts,
            };
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
            let run_result = interp.run(&stmts).await;
            let unhandled = interp.take_page_failure();
            InterpreterResult {
                error: run_result.err().or(unhandled),
                lines: interp.output,
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
///
/// This private core also installs the file's `## No <opt>` decorators as the active config
/// (so the VM compiler's opt gates honor them) and hands the closure the `## Require` demands
/// so a Result-returning caller can enforce them. The public [`with_parsed_program`] /
/// [`with_parsed_program_checked`] wrappers select whether `## Require` is verified.
/// [`parse_seam_e`], with the failure already rendered as its Socratic message.
///
/// The overwhelming majority of callers want the sentence and nothing else. The few that need to
/// point AT the mistake — a dev-server overlay, a diagnostic with a caret — take the seam below
/// and keep the span.
fn parse_seam<R>(
    input: &str,
    body: impl for<'a> FnOnce(
        Result<
            (
                &'a [logicaffeine_language::ast::stmt::Stmt<'a>],
                &'a logicaffeine_language::analysis::TypeRegistry,
                logicaffeine_language::analysis::PolicyRegistry,
            ),
            String,
        >,
        &'a Interner,
        Vec<crate::optimization::Opt>,
    ) -> R,
) -> R {
    parse_seam_e(input, |parsed, interner, facts| {
        body(
            parsed.map_err(|e| socratic_explanation(&e, interner)),
            interner,
            facts.required,
        )
    })
}

/// How a span of the text the parser saw maps back to the author's own text: the seam wraps a bare
/// script as `## Main` (a prefix) and then splices the stdlib modules the program uses (after any
/// leading prose). Both edits are undone here, once, so every span leaving the seam is a span of
/// the file the author wrote.
struct UserText {
    /// The `## Main\n` prefix the implicit-main wrap added, or 0.
    implicit_prefix: usize,
    /// The exact splice `apply_prelude_mapped` performed on the wrapped text.
    segments: Vec<crate::loader::PreludeSegment>,
}

impl UserText {
    /// `span` in the author's text, or `None` when it lies in text the seam added.
    fn span(&self, span: crate::token::Span) -> Option<crate::token::Span> {
        let wrapped = crate::compile::prelude_user_span(&self.segments, span)?;
        if wrapped.start < self.implicit_prefix {
            return None;
        }
        Some(crate::token::Span::new(
            wrapped.start - self.implicit_prefix,
            wrapped.end - self.implicit_prefix,
        ))
    }

    /// `span` in the author's text; a span in added text becomes the default (no location).
    fn span_or_default(&self, span: crate::token::Span) -> crate::token::Span {
        self.span(span).unwrap_or_default()
    }
}

/// What the parser learned about a program besides its statements.
struct ParseFacts {
    /// The optimizations `## Require` demands.
    required: Vec<crate::optimization::Opt>,
    /// The LGX accessibility and structure findings, their spans in the author's text.
    findings: Vec<logicaffeine_language::lgx::check::Finding>,
    /// The value-semantics copy lint's findings, their spans in the author's text — see
    /// `value_copy_findings_of`.
    copy_findings: Vec<logicaffeine_language::lint::Finding>,
    /// Every tensor the program binds, as `(rows, cols)` — see `Parser::tensor_shapes`.
    tensor_shapes: std::collections::HashMap<logicaffeine_base::Symbol, (i64, i64)>,
    /// Every `Stmt::Site` marker's `(span, component, action)`, in the AUTHOR's own file
    /// coordinates — translated here, once, the same way `findings` is just below, so a runtime
    /// failure's location never needs a second, bespoke undoing of the implicit-`## Main`/stdlib
    /// splice (`FailureSite`'s own doc comment names that exact double-mapping shape as a past bug).
    failure_sites: Vec<logicaffeine_language::ast::stmt::FailureSite>,
    /// The map from the parsed text back to the author's.
    user_text: UserText,
}

/// The parse seam, handing a failure over whole — message AND span — so a caller that points at
/// a location has one. [`parse_seam`] is the message-shaped wrapper most callers take.

fn parse_seam_e<R>(
    input: &str,
    body: impl for<'a> FnOnce(
        Result<
            (
                &'a [logicaffeine_language::ast::stmt::Stmt<'a>],
                &'a logicaffeine_language::analysis::TypeRegistry,
                logicaffeine_language::analysis::PolicyRegistry,
            ),
            logicaffeine_language::error::ParseError,
        >,
        &'a Interner,
        ParseFacts,
    ) -> R,
) -> R {
    parse_seam_with(input, None, body)
}

/// [`parse_seam_e`], and — under `mono` — the program handed on is the monomorphic one
/// ([`crate::monomorphize`]): every generic function and applied generic type made concrete, for
/// a backend that lowers only concrete code.
fn parse_seam_with<R>(
    input: &str,
    mono: Option<crate::monomorphize::Options>,
    body: impl for<'a> FnOnce(
        Result<
            (
                &'a [logicaffeine_language::ast::stmt::Stmt<'a>],
                &'a logicaffeine_language::analysis::TypeRegistry,
                logicaffeine_language::analysis::PolicyRegistry,
            ),
            logicaffeine_language::error::ParseError,
        >,
        &'a Interner,
        ParseFacts,
    ) -> R,
) -> R {
    use logicaffeine_language::ast::stmt::{Expr, Stmt, TypeExpr};

    // A bare script (no `##` headers, imperative opening) runs as `## Main`.
    let implicit = implicit_main(input);
    let implicit_prefix = implicit
        .as_ref()
        .map_or(0, |wrapped| wrapped.len() - input.len());
    let input = implicit.as_deref().unwrap_or(input);

    // Phase 10: auto-prepend the stdlib modules the program references (no-op when
    // it uses no stdlib vocabulary, so such programs stay byte-identical).
    let (prelude_src, segments) = crate::loader::apply_prelude_mapped(input);
    let user_text = UserText {
        implicit_prefix,
        segments,
    };
    let input = prelude_src.as_ref();
    let library = crate::loader::LibraryProvenance::of(input);

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let mut tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    // Resolve the qualified escape hatch for cross-module calls (`Alias::fn` →
    // bare `fn`); no-op for single-file programs. Feeds the VM, JIT, wasm, isa,
    // and sync-interpreter tiers that share this seam.
    let functions = crate::analysis::assemble::defined_function_names(input);
    crate::analysis::assemble::resolve_qualified_names(&mut tokens, &mut interner, &functions);

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
    let _compile_facts = crate::optimize::facts::scope();
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
    let (parsed, opt_flags, facts, type_registry_for_engines) = {
        let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
        let stmts = parser.parse_program();
        let flags = parser.program_opt_flags();
        let findings = parser
            .lgx_findings()
            .iter()
            .cloned()
            .map(|mut f| {
                f.span = user_text.span_or_default(f.span);
                f
            })
            .collect();
        let failure_sites = parser
            .failure_sites()
            .iter()
            .cloned()
            .map(|mut s| {
                s.span = user_text.span_or_default(s.span);
                s
            })
            .collect();
        let copy_findings = parser
            .value_copy_findings()
            .iter()
            .cloned()
            .map(|mut f| {
                f.span = user_text.span_or_default(f.span);
                f
            })
            .collect();
        let facts = ParseFacts {
            required: parser.required_optimizations(),
            findings,
            tensor_shapes: parser.tensor_shapes().clone(),
            failure_sites,
            copy_findings,
            user_text,
        };
        (stmts, flags, facts, parser.into_type_registry())
    };

    let parsed =
        parsed.and_then(
            |stmts| match crate::analysis::refuse_retired_calls(&stmts, &interner) {
                Ok(()) => Ok(stmts),
                Err(refusal) => Err(logicaffeine_language::error::ParseError {
                    kind: logicaffeine_language::error::ParseErrorKind::Custom(refusal),
                    span: logicaffeine_language::token::Span::default(),
                }),
            },
        );
    // Under `mono`, the program as its concrete instances (see `parse_seam_with`).
    let parsed = match (parsed, mono) {
        (Ok(stmts), Some(opts)) => crate::monomorphize::monomorphize(
            &stmts,
            &type_registry_for_engines,
            &mut interner,
            crate::ast_map::Arenas {
                exprs: &imperative_expr_arena,
                stmts: &stmt_arena,
                types: &type_expr_arena,
            },
            opts,
        )
        .map(|m| (m.stmts, Some(m.registry)))
        .map_err(|message| logicaffeine_language::error::ParseError {
            kind: logicaffeine_language::error::ParseErrorKind::Custom(message),
            span: logicaffeine_language::token::Span::default(),
        }),
        (Ok(stmts), None) => Ok((stmts, None)),
        (Err(e), _) => Err(e),
    };
    match parsed {
        Ok((stmts, mono_registry)) => {
            let type_registry_for_engines = mono_registry.unwrap_or(type_registry_for_engines);
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
            // Install `## No` as the active config so the VM compiler's opt gates
            // (`NarrowVm`/`FastDiv`/`Borrow`) honor it too — the AOT path gets this via
            // `optimize_program`; the raw VM/wasm/isa seam needs it explicitly.
            crate::optimize::set_active_config(run_cfg);
            let resolved = crate::resolve_program::resolve_program(
                &stmts,
                crate::ast_map::Arenas {
                    exprs: &imperative_expr_arena,
                    stmts: &stmt_arena,
                    types: &type_expr_arena,
                },
                &mut interner,
                run_cfg.is_on(crate::optimization::Opt::Comptime),
                library.functions(),
            );
            let pre = resolved.unwrap_or(stmts.as_slice());
            match crate::tail_call::rewrite_accumulators(
                pre,
                &stmt_arena,
                &imperative_expr_arena,
                &mut interner,
            ) {
                Some(rw) => body(
                    Ok((rw, &type_registry_for_engines, policy_registry)),
                    &interner,
                    facts,
                ),
                None => body(
                    Ok((pre, &type_registry_for_engines, policy_registry)),
                    &interner,
                    facts,
                ),
            }
        }
        Err(e) => body(Err(e), &interner, facts),
    }
}

/// The execution front-end shared by every engine (see `parse_seam`): lex → MWE → discovery →
/// parse, with the file's `## No` decorators installed as the active config. `## Require` is NOT
/// enforced here — this generic variant serves the differential-test seams and callers whose
/// return type can't carry a compile error; use [`with_parsed_program_checked`] for the emit
/// paths that must fail an unfired `## Require`.
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
    parse_seam(input, |parsed, interner, _required| f(parsed, interner))
}

/// [`with_parsed_program`] that also hands over every tensor's `(rows, cols)` — the shape table the parser keeps
/// and the desugared statements do not (a declaration desugars to a flat `Seq` of `rows · cols` elements). The
/// spatial walk reads it to declare each tensor in the shape the program gives it.
pub fn with_parsed_tensor_program<R>(
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
        &std::collections::HashMap<logicaffeine_base::Symbol, (i64, i64)>,
    ) -> R,
) -> R {
    parse_seam_e(input, |parsed, interner, facts| {
        f(
            parsed.map_err(|e| socratic_explanation(&e, interner)),
            interner,
            &facts.tensor_shapes,
        )
    })
}

/// [`with_parsed_program`] for a Result-returning compile (the `emit c`/`isa`/`wasm` backends):
/// installs `## No`, opens a `## Require` firing-trace around `f`, and after `f` succeeds fails
/// the compile on any demanded optimization that did not fire — the honest answer when a backend
/// (e.g. the VM tiers, which don't memoize) cannot provide a required optimization. `f`'s own
/// error propagates unchanged.
///
/// `f` is handed the parse Result RAW — spans still in the prelude-expanded text `f` actually
/// parsed, exactly like every backend's own `stmts` — and undone EXACTLY ONCE, at the single point
/// below where an error leaves this function. `f` may return the parse error unchanged (the
/// overwhelming majority: `let (stmts, ..) = parsed?;`) or a NEW error located at an AST span
/// `compile_with_oracle_located` or a resolver check built (also still prelude-coordinate,
/// unavoidably, since nothing between the parse and here ever undoes the splice) — both need the
/// SAME one undoing, so there is exactly one place to trust and it runs on whatever comes out,
/// not on what goes in. Undoing it going IN as well as coming OUT re-translated an already-correct
/// span a second time: the splice's stdlib segment is typically the largest run in the expanded
/// text, so almost any small, correct, already-undone offset falls inside where IT sits and maps
/// to "added text" — silently becoming `Span::default()` for any caller whose `f` simply forwards
/// `parsed`'s own error, which is the shape every current caller uses.
pub fn with_parsed_program_checked<T>(
    input: &str,
    f: impl for<'a> FnOnce(
        Result<
            (
                &'a [logicaffeine_language::ast::stmt::Stmt<'a>],
                &'a logicaffeine_language::analysis::TypeRegistry,
                logicaffeine_language::analysis::PolicyRegistry,
            ),
            logicaffeine_language::error::ParseError,
        >,
        &'a Interner,
    ) -> Result<T, crate::error::ParseError>,
) -> Result<T, crate::error::ParseError> {
    parse_seam_e(input, |parsed, interner, facts| {
        let gate = crate::optimize::DecoratorGate::for_required(facts.required);
        let user_text = facts.user_text;
        match f(parsed, interner) {
            Ok(t) => gate
                .verify()
                .map(|_| t)
                .map_err(|m| crate::error::ParseError {
                    kind: crate::error::ParseErrorKind::Custom(m),
                    span: crate::token::Span::default(),
                }),
            Err(mut e) => {
                gate.close();
                e.span = user_text.span_or_default(e.span);
                Err(e)
            }
        }
    })
}

/// [`with_parsed_program`] over the program's monomorphic form ([`crate::monomorphize`]) — every
/// generic function and applied generic type made concrete — for a backend that lowers only
/// concrete code.
pub fn with_monomorphic_program<R>(
    input: &str,
    options: crate::monomorphize::Options,
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
    parse_seam_with(input, Some(options), |parsed, interner, _facts| {
        f(
            parsed.map_err(|e| socratic_explanation(&e, interner)),
            interner,
        )
    })
}

/// [`with_parsed_program_checked`] over the program's monomorphic form ([`crate::monomorphize`]) —
/// every generic function and applied generic type made concrete — for a backend that lowers only
/// concrete code.
pub fn with_monomorphic_program_checked<T>(
    input: &str,
    options: crate::monomorphize::Options,
    f: impl for<'a> FnOnce(
        Result<
            (
                &'a [logicaffeine_language::ast::stmt::Stmt<'a>],
                &'a logicaffeine_language::analysis::TypeRegistry,
                logicaffeine_language::analysis::PolicyRegistry,
            ),
            logicaffeine_language::error::ParseError,
        >,
        &'a Interner,
    ) -> Result<T, crate::error::ParseError>,
) -> Result<T, crate::error::ParseError> {
    parse_seam_with(input, Some(options), |parsed, interner, facts| {
        let gate = crate::optimize::DecoratorGate::for_required(facts.required);
        let user_text = facts.user_text;
        match f(parsed, interner) {
            Ok(t) => gate
                .verify()
                .map(|_| t)
                .map_err(|m| crate::error::ParseError {
                    kind: crate::error::ParseErrorKind::Custom(m),
                    span: crate::token::Span::default(),
                }),
            Err(mut e) => {
                gate.close();
                e.span = user_text.span_or_default(e.span);
                Err(e)
            }
        }
    })
}

/// The accessibility and structure warnings `lgx/check.rs` computes for `input`.
///
/// ★ THE CHECKER RAN ON EVERY PARSE AND NOTHING READ IT. `img` without `alt`, a `button` that
/// announces nothing, an input with no label, a link with no href — all computed, all discarded,
/// because `Parser::lgx_findings()` had exactly one caller and it was a test. That is the failure
/// its own lock warns about ("a checker nobody runs is worse than no checker"), one layer up.
pub fn lgx_findings_of(input: &str) -> Vec<logicaffeine_language::lgx::check::Finding> {
    parse_seam_e(input, |_parsed, _interner, facts| facts.findings)
}

/// The value-semantics copy lint's warnings for `input` — see `logicaffeine_language::lint`. Same
/// shape as [`lgx_findings_of`]: computed on every parse, worth nothing until something reads it.
pub fn value_copy_findings_of(input: &str) -> Vec<logicaffeine_language::lint::Finding> {
    parse_seam_e(input, |_parsed, _interner, facts| facts.copy_findings)
}

/// Every [`Stmt::Site`](logicaffeine_language::ast::stmt::Stmt::Site) marker `input` parsed to, as
/// the `(span, component, action)` [`FailureSite`](logicaffeine_language::ast::stmt::FailureSite)
/// the marker's id indexes — in the author's own file coordinates, the implicit-`## Main` wrap and
/// the stdlib-prelude splice already undone (matching [`lgx_findings_of`]/[`parse_error_of`]'s own
/// treatment). The tree-walker's `current_site()` and a compiled program's `site_at` pc-lookup both
/// answer an id into THIS SAME table; whoever renders a runtime failure looks it up here and calls
/// [`FailureSite::render`](logicaffeine_language::ast::stmt::FailureSite::render) with the message.
pub fn failure_sites_of(input: &str) -> Vec<logicaffeine_language::ast::stmt::FailureSite> {
    parse_seam_e(input, |_parsed, _interner, facts| facts.failure_sites)
}

/// The parse error for `input`, in the AUTHOR's own file coordinates — the implicit-`## Main` wrap
/// and the stdlib-prelude splice both undone, exactly as [`with_parsed_program_checked`] undoes
/// them for its callers — or `None` when `input` parses. A pure parse check with no `## Require`
/// gate: a caller that wants to render its OWN caret block (`largo run --interpret`, matching what
/// `largo check` already shows) takes the located error from here instead of formatting whatever
/// `Result::Err` variant happened to reach it.
pub fn parse_error_of(input: &str) -> Option<logicaffeine_language::error::ParseError> {
    parse_seam_e(input, |parsed, _interner, facts| {
        parsed.err().map(|mut e| {
            e.span = facts.user_text.span_or_default(e.span);
            e
        })
    })
}

/// The `## Require <opt>` demands declared in `input` (a lightweight parse via `parse_seam`).
/// Empty for a program with no `## Require` decorators.
pub fn program_required_opts(input: &str) -> Vec<crate::optimization::Opt> {
    parse_seam(input, |_parsed, _interner, required| required)
}

/// The tree-walk interpreter (`largo run --interpret`) is the raw differential tier — it applies
/// NO optimizations, so it cannot satisfy any `## Require <opt>`. Returns the honest error for the
/// first requirement (mirroring the compiled tiers' message), or `None` when there is none.
pub fn interpret_unsatisfiable_require(input: &str) -> Option<String> {
    program_required_opts(input).first().map(|opt| {
        let kw = opt.meta().keyword;
        format!(
            "`## Require {kw}` was declared, but the `{kw}` optimization did not fire — the \
             tree-walk interpreter (`--interpret`) applies no optimizations. Run the compiled \
             tier (`largo run` / `largo build` / `largo emit`) to satisfy it, or remove the \
             `## Require {kw}`."
        )
    })
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
    with_optimized_program_scoped(input, None, f)
}

/// [`with_optimized_program`], with the assembled project's file scope checked
/// on the same parsed AST before it is optimized.  Project runners use this to
/// avoid assembling and parsing the entire project a second time merely to
/// enforce import visibility.
pub fn with_optimized_program_scoped<R>(
    input: &str,
    scope: Option<&crate::analysis::ScopeInfo>,
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
    with_optimized_program_tiered_scoped(input, tier, scope, f)
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
    with_optimized_program_tiered_scoped(input, tier, None, f)
}

fn with_optimized_program_tiered_scoped<R>(
    input: &str,
    tier: crate::optimization::Tier,
    scope: Option<&crate::analysis::ScopeInfo>,
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

    // A bare script (no `##` headers, imperative opening) runs as `## Main`.
    let implicit = implicit_main(input);
    let input = implicit.as_deref().unwrap_or(input);

    // Phase 10: auto-prepend the stdlib modules the program references (no-op when
    // it uses no stdlib vocabulary, so such programs stay byte-identical).
    let prelude_src = crate::loader::apply_prelude(input);
    let input = prelude_src.as_ref();
    let library = crate::loader::LibraryProvenance::of(input);

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let mut tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    // Resolve the qualified escape hatch for cross-module calls (`Alias::fn` →
    // bare `fn`); no-op for single-file programs. Feeds the VM, JIT, wasm, isa,
    // and sync-interpreter tiers that share this seam.
    let functions = crate::analysis::assemble::defined_function_names(input);
    crate::analysis::assemble::resolve_qualified_names(&mut tokens, &mut interner, &functions);

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
    let _compile_facts = crate::optimize::facts::scope();
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
    let (parsed, opt_flags, tier_pins, type_registry_for_engines) = {
        let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
        let stmts = parser.parse_program();
        let flags = parser.program_opt_flags();
        let pins = parser.program_tier_pins();
        (stmts, flags, pins, parser.into_type_registry())
    };

    match parsed {
        Ok(stmts) => {
            if let Some(scope) = scope {
                if let Err(why) = crate::analysis::check_scopes(&stmts, scope, &interner) {
                    return f(Err(why), &interner);
                }
            }
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
            let resolved = crate::resolve_program::resolve_program(
                &stmts,
                crate::ast_map::Arenas {
                    exprs: &imperative_expr_arena,
                    stmts: &stmt_arena,
                    types: &type_expr_arena,
                },
                &mut interner,
                run_cfg.is_on(crate::optimization::Opt::Comptime),
                library.functions(),
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
                Some(rw) => f(
                    Ok((rw, &type_registry_for_engines, policy_registry)),
                    &interner,
                ),
                None => f(
                    Ok((&optimized, &type_registry_for_engines, policy_registry)),
                    &interner,
                ),
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

    // A bare script (no `##` headers, imperative opening) runs as `## Main`.
    let implicit = implicit_main(input);
    let input = implicit.as_deref().unwrap_or(input);

    // Phase 10: auto-prepend the stdlib modules the program references (no-op when
    // it uses no stdlib vocabulary, so such programs stay byte-identical).
    let prelude_src = crate::loader::apply_prelude(input);
    let input = prelude_src.as_ref();
    let library = crate::loader::LibraryProvenance::of(input);

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let mut tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    // Resolve the qualified escape hatch for cross-module calls (`Alias::fn` →
    // bare `fn`); no-op for single-file programs. Feeds the VM, JIT, wasm, isa,
    // and sync-interpreter tiers that share this seam.
    let functions = crate::analysis::assemble::defined_function_names(input);
    crate::analysis::assemble::resolve_qualified_names(&mut tokens, &mut interner, &functions);

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
    let _compile_facts = crate::optimize::facts::scope();
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
    let (parsed, type_registry_for_engines) = {
        let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
        let parsed = parser.parse_program();
        (parsed, parser.into_type_registry())
    };

    match parsed {
        Ok(stmts) => {
            let config = crate::optimization::OptimizationConfig::from_env();
            let resolved = crate::resolve_program::resolve_program(
                &stmts,
                crate::ast_map::Arenas {
                    exprs: &imperative_expr_arena,
                    stmts: &stmt_arena,
                    types: &type_expr_arena,
                },
                &mut interner,
                config.is_on(crate::optimization::Opt::Comptime),
                library.functions(),
            );
            let stmts: Vec<Stmt> = match resolved {
                Some(rw) => rw.to_vec(),
                None => stmts,
            };
            let optimized = crate::optimize::optimize_program(
                stmts,
                &imperative_expr_arena,
                &stmt_arena,
                &mut interner,
                &config,
            );
            f(
                Ok((&optimized, &type_registry_for_engines, policy_registry)),
                &interner,
            )
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

/// In-process execution tier. All choices consume the same optimized program.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    /// Preserve the product dispatcher: JIT/VM for synchronous programs and
    /// the tree-walker for programs requiring async or the scheduler.
    #[default]
    Auto,
    Jit,
    Vm,
    TreeWalker,
}

thread_local! {
    static ENGINE: std::cell::Cell<Engine> = const { std::cell::Cell::new(Engine::Auto) };
}

/// Select the tier used by subsequent in-process runs on this thread.
pub fn set_engine(engine: Engine) {
    ENGINE.with(|selected| selected.set(engine));
}

fn engine() -> Engine {
    ENGINE.with(std::cell::Cell::get)
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

/// The bytecode the VM runs for `input`, as `largo run --interpret` compiles it: the run path's optimizer, the oracle
/// and the VM compiler. One section per function, `## Main` first, then `## To <name>` in code order, each instruction
/// at its pc. A source that does not parse, or that only the tree-walker runs, answers why.
pub fn vm_listing(input: &str) -> Result<String, String> {
    with_optimized_program(input, |parsed, interner| {
        let (stmts, type_registry, _policies) = parsed?;
        if crate::interpreter::needs_async(stmts, interner)
            || crate::concurrency::uses_scheduler(stmts)
        {
            return Err(
                "this program awaits or runs tasks, so it runs on the tree-walker, not the VM"
                    .to_string(),
            );
        }
        let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
        let program = crate::vm::Compiler::compile_with_oracle(
            stmts,
            interner,
            Some(type_registry),
            Some(oracle),
        )
        .map_err(|e| {
            format!(
                "the VM compiler does not take this program, so it runs on the tree-walker: {e:?}"
            )
        })?;
        let mut starts: Vec<(usize, String)> = program
            .functions
            .iter()
            .map(|f| (f.entry_pc, format!("## To {}", interner.resolve(f.name))))
            .collect();
        starts.sort();
        let lines = crate::vm::disasm::disassemble(&program);
        let width = lines.len().saturating_sub(1).to_string().len();
        let mut out = String::from("## Main\n");
        let mut next = starts.iter().peekable();
        for line in &lines {
            while let Some((pc, header)) = next.peek() {
                if *pc > line.pc {
                    break;
                }
                out.push_str(&format!("\n{header}\n"));
                next.next();
            }
            out.push_str(&format!("{:>width$}  {}\n", line.pc, line.text));
        }
        Ok(out)
    })
}

/// Like [`interpret_for_ui_sync`], but supplies the program's argument vector
/// to the `args()` system native. `program_args` is the full argv (index 0 is
/// the program name), matching the compiled binary's `env::args()` — this is
/// what `largo run --interpret N` forwards so an args-driven `main.lg` runs the
/// same on the VM/JIT as on the native binary.
pub fn interpret_for_ui_sync_with_args(input: &str, program_args: &[String]) -> InterpreterResult {
    interpret_for_ui_sync_with_args_impl(input, program_args, None)
}

/// Project-aware form of [`interpret_for_ui_sync_with_args`].  The scope gate
/// consumes the AST this execution already parsed, so explicit VM/JIT runs do
/// not pay for a separate front-end parse on every invocation.
pub fn interpret_for_ui_sync_with_args_scoped(
    input: &str,
    program_args: &[String],
    scope: &crate::analysis::ScopeInfo,
) -> InterpreterResult {
    interpret_for_ui_sync_with_args_impl(input, program_args, Some(scope))
}

fn interpret_for_ui_sync_with_args_impl(
    input: &str,
    program_args: &[String],
    scope: Option<&crate::analysis::ScopeInfo>,
) -> InterpreterResult {
    begin_ui_run();
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
    with_optimized_program_scoped(input, scope, |parsed, interner| match parsed {
        Ok((stmts, type_registry, policies)) => {
            if let Some(rejection) = send_escape_rejection(stmts) {
                return rejection;
            }
            if let Some(rejection) = send_dimension_rejection(stmts, interner) {
                return rejection;
            }
            let needs_async = crate::interpreter::needs_async(stmts, interner);
            let needs_scheduler = crate::concurrency::uses_scheduler(stmts);
            if needs_async || needs_scheduler {
                if !matches!(engine(), Engine::Auto | Engine::TreeWalker) {
                    let why = if needs_async {
                        "the selected VM/JIT tier is sync-only; this program requires async execution"
                    } else {
                        "the selected VM/JIT tier has no scheduler driver; this program requires the tree-walker scheduler"
                    };
                    trace("refused");
                    return InterpreterResult {
                        lines: Vec::new(),
                        error: Some(why.to_string()),
                    };
                }
                trace("treewalker (async)");
                return run_treewalker(
                    stmts,
                    type_registry,
                    policies,
                    interner,
                    true,
                    program_args,
                );
            }
            if engine() == Engine::TreeWalker {
                trace("treewalker");
                return run_treewalker(
                    stmts,
                    type_registry,
                    policies,
                    interner,
                    false,
                    program_args,
                );
            }
            if engine() == Engine::TreeWalker {
                trace("treewalker");
                return run_treewalker(
                    stmts,
                    type_registry,
                    policies,
                    interner,
                    false,
                    program_args,
                );
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
                    let mut vm = crate::vm::Vm::new(&program)
                        .with_policy_ctx(&policies, interner)
                        .with_program_args(program_args.to_vec());
                    // A budgeted run stays on the VM dispatch loop: JIT-tiered
                    // native code executes outside the STEPPED accounting, so a
                    // pure hot loop would escape its step budget entirely.
                    // Unbudgeted runs keep the full tier ladder.
                    let mut native = None;
                    if engine() == Engine::Jit && crate::interpreter::active_fuel().is_some() {
                        trace("refused");
                        return InterpreterResult {
                            lines: Vec::new(),
                            error: Some(
                                "the selected JIT tier cannot preserve --max-steps accounting; use --engine vm"
                                    .to_string(),
                            ),
                        };
                    }
                    if crate::interpreter::active_fuel().is_none()
                        && matches!(engine(), Engine::Auto | Engine::Jit)
                    {
                        if let Some(tier) = crate::vm::installed_native_tier() {
                            vm = vm.with_native_tier(tier);
                            native = Some(tier);
                        }
                    }
                    if engine() == Engine::Jit && native.is_none() {
                        trace("refused");
                        return InterpreterResult {
                            lines: Vec::new(),
                            error: Some(
                                "the selected JIT tier is unavailable on this build or platform"
                                    .to_string(),
                            ),
                        };
                    }
                    trace(if native.is_some() { "vm+jit" } else { "vm" });
                    // Compiled-native tier (HOTSWAP §Axis-3): install any AOT functions
                    // the run was given, so they dispatch to rustc -O3 machine code.
                    #[cfg(not(target_arch = "wasm32"))]
                    install_pending_aot_natives(&mut vm, &program, interner);
                    // A budgeted run (`--max-steps` / MCP `step_budget`) drives
                    // the resumable STEPPED monomorph and stops at the
                    // deterministic op where the shared fuel cell runs dry;
                    // the unbudgeted path is the untouched hot loop.
                    let error = crate::vm::drive_vm(&mut vm);
                    if let Some(tier) = native {
                        trace(&format!(
                            "native-tier {}: {}",
                            tier.engine_name(),
                            tier.compile_summary()
                        ));
                        let declines = tier.decline_summary();
                        if !declines.is_empty() {
                            trace(&format!(
                                "native-declines {}: {declines}",
                                tier.engine_name()
                            ));
                        }
                        let opts = tier.optimization_summary();
                        if !opts.is_empty() {
                            trace(&format!("native-opts {}: {opts}", tier.engine_name()));
                        }
                        let execution = tier.execution_summary();
                        if !execution.is_empty() {
                            trace(&format!(
                                "native-runtime {}: {execution}",
                                tier.engine_name()
                            ));
                        }
                    }
                    let result = InterpreterResult {
                        lines: vm.into_lines(),
                        error,
                    };

                    // Debug-build shadow oracle: the SAME program runs on the
                    // tree-walker and the full outcome must match — this turns
                    // the entire existing test corpus into a differential
                    // suite. (Skipped on wasm to keep dev builds light, and
                    // under fuel: the VM already drained the shared cell, so
                    // the shadow would exhaust at step one and always diverge.
                    // Budgeted equivalence is fuel_budget.rs's job.)
                    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
                    if crate::interpreter::active_fuel().is_none() {
                        let shadow = in_a_document_of_its_own(|| {
                            run_treewalker(
                                stmts,
                                type_registry,
                                policies.clone(),
                                interner,
                                false,
                                program_args,
                            )
                        });
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
                Err(reason) => {
                    if engine() != Engine::Auto {
                        trace("refused");
                        InterpreterResult {
                            lines: Vec::new(),
                            error: Some(format!(
                                "the selected VM/JIT tier cannot compile this program: {reason}"
                            )),
                        }
                    } else {
                        trace("treewalker (vm-reject)");
                        run_treewalker(
                            stmts,
                            type_registry,
                            policies,
                            interner,
                            false,
                            program_args,
                        )
                    }
                }
            }
        }
        Err(advice) => InterpreterResult {
            lines: vec![],
            error: Some(advice),
        },
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
    begin_ui_run();
    // Async programs (file I/O, sleep, mount) run on the tree-walker's async
    // executor regardless of tier — the VM is sync-only — so that case is
    // identical to the optimized entry (and must `await` rather than nest a
    // `block_on`); delegate it.
    let needs_async = with_parsed_program(input, |parsed, interner| match parsed {
        Ok((stmts, _, _)) => crate::interpreter::needs_async(stmts, interner),
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
    begin_ui_run();
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
            if let Some(rejection) = send_dimension_rejection(stmts, interner) {
                return rejection;
            }
            if crate::interpreter::needs_async(stmts, interner)
                || crate::concurrency::uses_scheduler(stmts)
            {
                trace("treewalker (async)");
                return run_treewalker(
                    stmts,
                    type_registry,
                    policies,
                    interner,
                    true,
                    program_args,
                );
            }
            match crate::vm::Compiler::compile_with_types(stmts, interner, Some(type_registry)) {
                Ok(program) => {
                    let mut vm = crate::vm::Vm::new(&program)
                        .with_policy_ctx(&policies, interner)
                        .with_program_args(program_args.to_vec());
                    // A budgeted run stays on the VM dispatch loop: JIT-tiered
                    // native code executes outside the STEPPED accounting, so a
                    // pure hot loop would escape its step budget entirely.
                    // Unbudgeted runs keep the full tier ladder.
                    let mut native = None;
                    if crate::interpreter::active_fuel().is_none() {
                        if let Some(tier) = crate::vm::installed_native_tier() {
                            vm = vm.with_native_tier(tier);
                            native = Some(tier);
                        }
                    }
                    trace(if native.is_some() {
                        "vm+jit (baseline)"
                    } else {
                        "vm (baseline)"
                    });
                    // Compiled-native tier (HOTSWAP §Axis-3): install any AOT functions
                    // the run was given, so they dispatch to rustc -O3 machine code.
                    #[cfg(not(target_arch = "wasm32"))]
                    install_pending_aot_natives(&mut vm, &program, interner);
                    let error = vm.run().err();
                    if let Some(tier) = native {
                        trace(&format!(
                            "native-tier {}: {}",
                            tier.engine_name(),
                            tier.compile_summary()
                        ));
                        let declines = tier.decline_summary();
                        if !declines.is_empty() {
                            trace(&format!(
                                "native-declines {}: {declines}",
                                tier.engine_name()
                            ));
                        }
                        let opts = tier.optimization_summary();
                        if !opts.is_empty() {
                            trace(&format!("native-opts {}: {opts}", tier.engine_name()));
                        }
                        let execution = tier.execution_summary();
                        if !execution.is_empty() {
                            trace(&format!(
                                "native-runtime {}: {execution}",
                                tier.engine_name()
                            ));
                        }
                    }
                    let result = InterpreterResult {
                        lines: vm.into_lines(),
                        error,
                    };

                    // The same debug differential net as the optimized path: the
                    // baseline VM and the tree-walker must agree on the SAME
                    // (unoptimized) statements. (Skipped on wasm to keep dev
                    // builds light.)
                    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
                    {
                        let shadow = in_a_document_of_its_own(|| {
                            run_treewalker(
                                stmts,
                                type_registry,
                                policies.clone(),
                                interner,
                                false,
                                program_args,
                            )
                        });
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
                    run_treewalker(
                        stmts,
                        type_registry,
                        policies,
                        interner,
                        false,
                        program_args,
                    )
                }
            }
        }
        Err(advice) => InterpreterResult {
            lines: vec![],
            error: Some(advice),
        },
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
    main.install_yield_state(main_ys.clone(), err_sink.clone());

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
pub type ObserverCallback =
    std::rc::Rc<std::cell::RefCell<dyn FnMut(logicaffeine_runtime::SchedSnapshot)>>;

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
    main.install_yield_state(main_ys.clone(), err_sink.clone());

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
        Ok((stmts, type_registry, policies)) => run_program_concurrent(
            stmts,
            type_registry,
            policies,
            interner,
            &[],
            None,
            None,
            seed,
        ),
        Err(advice) => InterpreterResult {
            lines: vec![],
            error: Some(advice),
        },
    })
}

/// Run a concurrent program on the **bytecode VM** under the deterministic
/// scheduler (T11): compile to opcodes, then drive the main `Vm` (and any
/// spawned per-task VMs) through `run_until_block` via a `VmTask`. This is the
/// VM analog of `run_program_concurrent`; the default routing still sends
/// concurrent programs to the tree-walker, so this is the explicit VM entry used
/// to exercise and (Phase 5c) differentially compare the VM concurrency tier.
pub fn run_vm_concurrent(input: &str) -> InterpreterResult {
    run_vm_concurrent_seeded(input, 0)
}

/// [`run_vm_concurrent`] under an explicit scheduler `seed` — the VM sibling of
/// [`run_treewalker_concurrent_seeded`] for the cross-tier seeded differential.
pub fn run_vm_concurrent_seeded(input: &str, seed: u64) -> InterpreterResult {
    run_vm_concurrent_traced(input, seed).0
}

/// [`run_vm_concurrent_seeded`], also returning the recorded scheduling trace —
/// the seam the `.replay` failure artifact captures its schedule through.
pub fn run_vm_concurrent_traced(
    input: &str,
    seed: u64,
) -> (InterpreterResult, logicaffeine_runtime::SchedTrace) {
    run_vm_concurrent_traced_with_config(
        input,
        logicaffeine_runtime::SchedulerConfig::default(),
        seed,
    )
}

/// Run a concurrent program on the VM under an EXPLICIT [`SchedulerConfig`](logicaffeine_runtime::SchedulerConfig) — the same knob surface the AOT
/// bytecode-walker's stepped scheduler exposes ([`crate::codegen_c::concurrency::SchedulerKnobs`]). This is
/// the oracle side of the cross-tier knob equivalence: `AOT-at-knobs-K == VM-at-knobs-K`. The default-config
/// wrapper is [`run_vm_concurrent`].
///
/// [`SchedulerConfig`]: logicaffeine_runtime::SchedulerConfig
pub fn run_vm_concurrent_with_config(
    input: &str,
    config: logicaffeine_runtime::SchedulerConfig,
    seed: u64,
) -> InterpreterResult {
    run_vm_concurrent_traced_with_config(input, config, seed).0
}

/// [`run_vm_concurrent_traced`] under an explicit [`SchedulerConfig`](logicaffeine_runtime::SchedulerConfig) (the config-parameterized core both
/// the default-config and knobbed entry points delegate to).
///
/// [`SchedulerConfig`]: logicaffeine_runtime::SchedulerConfig
pub fn run_vm_concurrent_traced_with_config(
    input: &str,
    config: logicaffeine_runtime::SchedulerConfig,
    seed: u64,
) -> (InterpreterResult, logicaffeine_runtime::SchedTrace) {
    use crate::concurrency::vm_driver::VmTask;
    use logicaffeine_runtime::{run_with_seed, RunOutcome, SchedSeed};
    use std::cell::RefCell;
    use std::rc::Rc;

    with_parsed_program(input, |parsed, interner| match parsed {
        Ok((stmts, type_registry, policies)) => {
            let empty_trace = || logicaffeine_runtime::SchedTrace {
                seed: SchedSeed(seed),
                choices: Vec::new(),
            };
            let program =
                match crate::vm::Compiler::compile_with_types(stmts, interner, Some(type_registry))
                {
                    Ok(p) => p,
                    Err(e) => {
                        return (
                            InterpreterResult {
                                lines: vec![],
                                error: Some(e),
                            },
                            empty_trace(),
                        )
                    }
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
            let (outcome, trace) = run_with_seed(config, SchedSeed(seed), move |sched| {
                sched.spawn_main(Box::new(main_task));
            });
            let mut error = err_sink.borrow().clone();
            if error.is_none() {
                if let RunOutcome::Deadlock = outcome {
                    error = Some("deadlock: every task is blocked waiting".to_string());
                }
            }
            let lines = output.borrow().clone();
            (InterpreterResult { lines, error }, trace)
        }
        Err(advice) => {
            let trace = logicaffeine_runtime::SchedTrace {
                seed: logicaffeine_runtime::SchedSeed(seed),
                choices: Vec::new(),
            };
            (
                InterpreterResult {
                    lines: vec![],
                    error: Some(advice),
                },
                trace,
            )
        }
    })
}

/// One cooperative poll interval for the VM net runner — a short async sleep so the relay delivers
/// (and, in the browser, the event loop runs) between `Await` polls. Mirrors the tree-walker's.
async fn vm_net_poll_tick() {
    #[cfg(not(target_arch = "wasm32"))]
    logicaffeine_system::tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    #[cfg(target_arch = "wasm32")]
    gloo_timers::future::TimeoutFuture::new(2).await;
}

/// Service one suspended VM networking request through the shared [`NetInbox`] — the SAME inbox the
/// tree-walker uses, so the wire encode/decode is identical and a `Send`/`Await` produces the same
/// value on both tiers. `Connect`/`Listen` do the async relay I/O; `Send`/`Stream` encode + publish;
/// `Await` drains-and-takes (yielding between polls) until the peer's message arrives.
async fn service_vm_net_block(
    vm: &mut crate::vm::Vm<'_>,
    netbox: &mut crate::concurrency::net_inbox::NetInbox,
    tcpbox: &mut crate::concurrency::tcp_registry::TcpRegistry,
    req: crate::vm::VmBlock,
) -> Result<(), String> {
    use crate::concurrency::marshal::{self};
    use crate::interpreter::RuntimeValue;
    use crate::vm::{Value, VmBlock};
    match req {
        // tcp.lg natives: rebuild the operand values and run the SAME value-level
        // exec the tree-walker calls — one implementation, both tiers.
        VmBlock::Tcp(kind, payloads) => {
            let op = crate::concurrency::tcp_registry::TcpOp::from_code(kind)
                .ok_or_else(|| format!("vm: unknown tcp op code {kind}"))?;
            let vals: Vec<RuntimeValue> = payloads.into_iter().map(marshal::rebuild).collect();
            let result = tcpbox.exec_values(op, vals).await?;
            vm.deliver_resume(Value::from_runtime(result));
            Ok(())
        }
        VmBlock::NetConnect(url_payload) => {
            let raw = marshal::rebuild(url_payload).to_display_string();
            let url = crate::concurrency::net_inbox::relay_url(&raw)?;
            // OFFLINE mode: `Connect` is a single-node local no-op (mirrors the tree-walker's offline
            // `Connect`), so the VM net oracle agrees with it. A relay-connected driver dials for real.
            if !crate::concurrency::net_inbox::net_is_offline() {
                let net = logicaffeine_system::net::Net::connect(&url)
                    .await
                    .map_err(|e| format!("Connect to relay '{url}' failed: {e}"))?;
                netbox.net = Some(net);
            }
            vm.deliver_resume(Value::nothing());
            Ok(())
        }
        VmBlock::NetListen(topic_payload) => {
            let raw = marshal::rebuild(topic_payload).to_display_string();
            let topic = logicaffeine_system::addr::canonical_topic(&raw);
            let hs_topic = crate::concurrency::net_inbox::handshake_topic_for(&topic);
            // LOCAL/OFFLINE mode (no relay): declare the inbox identity locally, skip the relay
            // subscribe — mirrors the tree-walker's offline `Listen` so both tiers agree.
            if let Some(net) = netbox.net.as_mut() {
                net.subscribe(&topic).await?;
                // Also receive peers' capability handshakes on our dedicated handshake topic (kept off
                // the data topic so they never interleave with messages).
                net.subscribe(&hs_topic).await?;
            }
            netbox.inbox = Some(std::rc::Rc::new(topic));
            vm.deliver_resume(Value::nothing());
            Ok(())
        }
        VmBlock::NetSend(to_payload, msg_payload) => {
            let topic = crate::concurrency::net_inbox::peer_topic(&marshal::rebuild(to_payload))?;
            // Advertise our surface to this peer on first contact — on its HANDSHAKE topic, never the
            // data topic — so it can negotiate back. Absorbed by the peer's drain, invisible to data.
            if let Some(hs) = netbox.first_contact_handshake(&topic) {
                netbox.publish(
                    &crate::concurrency::net_inbox::handshake_topic_for(&topic),
                    hs,
                )?;
            }
            let value = marshal::rebuild(msg_payload);
            let from = netbox
                .inbox
                .as_ref()
                .map(|t| t.to_string())
                .unwrap_or_default();
            // The negotiated maximal crush — byte-identical to the tree-walker's plain send (both call
            // `encode_negotiated` with the same program registry). Type-id fires only on a matching
            // peer epoch, so a raw / different-program peer still gets the plain encoding.
            let bytes =
                netbox.encode_negotiated(&from, &value, &topic, netbox.my_registry.clone())?;
            netbox.publish(&topic, bytes)?;
            vm.deliver_resume(Value::nothing());
            Ok(())
        }
        VmBlock::NetStream(to_payload, values_payload) => {
            let topic = crate::concurrency::net_inbox::peer_topic(&marshal::rebuild(to_payload))?;
            if let Some(hs) = netbox.first_contact_handshake(&topic) {
                netbox.publish(
                    &crate::concurrency::net_inbox::handshake_topic_for(&topic),
                    hs,
                )?;
            }
            let list = marshal::rebuild(values_payload);
            let items = match &list {
                RuntimeValue::List(rc) => rc.borrow().to_values(),
                other => vec![other.clone()],
            };
            let from = netbox
                .inbox
                .as_ref()
                .map(|t| t.to_string())
                .unwrap_or_default();
            let blob = marshal::frame_stream_message(&from, &items)?;
            netbox.publish(&topic, blob)?;
            vm.deliver_resume(Value::nothing());
            Ok(())
        }
        VmBlock::NetAwait(from_payload, stream) => {
            let want = crate::concurrency::net_inbox::peer_topic(&marshal::rebuild(from_payload))?;
            if netbox.inbox.is_none() {
                return Err(crate::concurrency::net_inbox::AWAIT_WITHOUT_LISTEN.to_string());
            }
            // OFFLINE (no relay): drain our own loopback outbox (the `Send … to <self>` delivered
            // locally) rather than requiring a relay — the VM net oracle mirrors the tree-walker.
            loop {
                let got = if stream {
                    netbox.try_take_stream(&want)
                } else {
                    netbox.try_take_message(&want, false)
                };
                if let Some(v) = got {
                    vm.deliver_resume(Value::from_runtime(v));
                    return Ok(());
                }
                // No type-id registry: the VM net runner sends self-describing (no `shared` dial),
                // so an empty registry decodes everything correctly.
                netbox.drain(netbox.my_registry.clone());
                let got = if stream {
                    netbox.try_take_stream(&want)
                } else {
                    netbox.try_take_message(&want, false)
                };
                if let Some(v) = got {
                    vm.deliver_resume(Value::from_runtime(v));
                    return Ok(());
                }
                vm_net_poll_tick().await;
            }
        }
        VmBlock::NetMakePeer(addr_payload) => {
            let raw = marshal::rebuild(addr_payload).to_display_string();
            let topic = logicaffeine_system::addr::canonical_topic(&raw);
            vm.deliver_resume(Value::from_runtime(RuntimeValue::Peer(std::rc::Rc::new(
                topic,
            ))));
            Ok(())
        }
        VmBlock::NetSync(topic_payload, current_payload) => {
            let topic = marshal::rebuild(topic_payload).to_display_string();
            let current = marshal::rebuild(current_payload);
            // Encode the counter (Int) or counter-struct as the relay wire form; `None` ⇒ nothing
            // to publish yet. Then merge whatever has arrived since the last sync point.
            let publish_bytes = crate::semantics::arith::crdt_to_wire(&current);
            // LOCAL/OFFLINE mode: single-node sync is a no-op — the local CRDT value stands (mirrors
            // the tree-walker's offline `Sync`). A relay-connected node publishes + merges.
            let merged = if let Some(net) = netbox.net.as_mut() {
                net.subscribe(&topic).await?;
                if let Some(bytes) = publish_bytes {
                    net.publish(&topic, bytes)?;
                }
                let incoming = net.drain();
                let mut merged = current;
                for (_t, data) in incoming {
                    merged = crate::semantics::arith::crdt_merge_wire(merged, &data);
                }
                merged
            } else {
                current
            };
            vm.deliver_resume(Value::from_runtime(merged));
            Ok(())
        }
        // `Op::Sleep` always yields `VmBlock::Sleep`, whichever async driver resumes it — a plain
        // sequential program with a bare `Sleep` (no channels/tasks, no networking) reaches THIS
        // driver too, because `run_strict_vm` refuses any `Sleep` as needing the async runtime and
        // the dispatcher's only other fallback is the concurrency scheduler's virtual clock, which a
        // program with no tasks never drives. Mirrors the tree-walker's own non-scheduled
        // `Stmt::Sleep` arm: a real timer, tokio's reactor when one is current (this driver runs
        // under `futures::executor::block_on`, which has none, so the fallback is what actually
        // fires), a blocking thread sleep otherwise, `gloo_timers` on wasm. `as_ticks` already
        // reports a Duration-valued Sleep in nanoseconds, the unit `Duration::from_nanos` wants.
        VmBlock::Sleep(ticks) => {
            if ticks > 0 {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let dur = std::time::Duration::from_nanos(ticks);
                    match logicaffeine_system::tokio::runtime::Handle::try_current() {
                        Ok(_) => logicaffeine_system::tokio::time::sleep(dur).await,
                        Err(_) => std::thread::sleep(dur),
                    }
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let millis = (ticks / 1_000_000) as u32;
                    if millis > 0 {
                        gloo_timers::future::TimeoutFuture::new(millis).await;
                    }
                }
            }
            vm.deliver_resume(Value::nothing());
            Ok(())
        }
        _ => Err(
            "run_vm_net_async services only peer-networking blocks; this program mixes \
                  channels/tasks with networking, which the VM net runner does not yet drive"
                .to_string(),
        ),
    }
}

/// Run a peer-networking program on the BYTECODE VM — the historical name for
/// [`run_vm_io_async`], kept so existing locks keep reading naturally.
pub async fn run_vm_net_async(input: &str) -> InterpreterResult {
    run_vm_io_async(input).await
}

/// The bytecode-tier HTTP accept loop for a `Serve` statement — the byte-for-byte
/// analogue of the tree-walker's `serve_http_concurrent`. It owns the SAME
/// `Listener` + `FuturesUnordered` overlap machinery: every connection is an
/// OWNED future driven inline (the VM is `!Send`, nothing is spawned), overlapped
/// with the accept future by `select!`, keep-alive per the shared
/// `request_wants_keep_alive` law, at most `MAX_INFLIGHT` in flight, a parse
/// error → 400 + close.
///
/// The ONLY difference from the interpreter is the handler dispatch: each request
/// re-enters `handler` AS VM BYTECODE via [`crate::vm::Vm::call_handler_bytecode`]
/// (a fresh child VM at the handler's entry, fed the marshaled `Request` value),
/// then reads back its `Response` through the SHARED
/// [`crate::interpreter::Interpreter::value_to_http_response`] marshaling. A
/// handler failure (runtime error, nested-IO suspension, or a non-`Response`
/// return) answers a 500 for THAT request and keeps serving — a bad request
/// never takes the server down. The loop never returns except on a fatal bind
/// error; the host aborts the task to stop it.
#[cfg(not(target_arch = "wasm32"))]
async fn serve_vm_accept_loop(
    vm: &crate::vm::Vm<'_>,
    port: u16,
    handler: u16,
) -> Result<(), String> {
    use futures::stream::{FuturesUnordered, StreamExt};
    use logicaffeine_system::http::server::{
        internal_error, read_error_response, Conn, Listener, ReadError,
    };
    use logicaffeine_system::http::wire::{request_wants_keep_alive, HttpRequest, HttpResponse};
    use logicaffeine_system::http::{
        service_unavailable, shutdown_requested, Admission, Lifecycle, LifecycleConfig,
    };
    use std::time::Duration;

    enum Step {
        Request(Conn, Result<HttpRequest, ReadError>),
        Closed,
    }
    type StepFut<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = Step> + 'a>>;

    /// Read one request under the configured body cap and read deadline — the
    /// same bounds the tree-walker and the native engines apply, so a slow or
    /// oversized peer is refused identically on every tier.
    async fn read_bounded(
        conn: &mut Conn,
        max_body: usize,
        deadline: Duration,
    ) -> Option<Result<HttpRequest, ReadError>> {
        match logicaffeine_system::tokio::time::timeout(
            deadline,
            conn.read_request_capped(max_body),
        )
        .await
        {
            Ok(read) => Some(read),
            Err(_) => None,
        }
    }

    fn read_step<'a>(mut conn: Conn, max_body: usize, deadline: Duration) -> StepFut<'a> {
        Box::pin(async move {
            match read_bounded(&mut conn, max_body, deadline).await {
                Some(read) => Step::Request(conn, read),
                None => Step::Closed,
            }
        })
    }

    fn write_step<'a>(
        mut conn: Conn,
        resp: HttpResponse,
        keep_alive: bool,
        max_body: usize,
        deadline: Duration,
    ) -> StepFut<'a> {
        Box::pin(async move {
            if conn.write_response_conn(&resp, keep_alive).await.is_err() || !keep_alive {
                return Step::Closed;
            }
            match read_bounded(&mut conn, max_body, deadline).await {
                Some(read) => Step::Request(conn, read),
                None => Step::Closed,
            }
        })
    }

    // The ONE deployment/dev-proxy seam: `LOGOS_HOST` names the interface (default
    // loopback) and `LOGOS_PORT` (then `PORT`) overrides the program's port —
    // composed into the whole address by `env_bind_address`, shared with the
    // interpreter + AOT tiers so no tier drifts.
    let port = logicaffeine_system::http::server::env_port_override(port);
    let addr = logicaffeine_system::http::server::env_bind_address(port);
    let listener = Listener::bind(&addr)
        .await
        .map_err(|e| format!("Serve: cannot bind {addr}: {e}"))?;
    let bound = listener.local_addr().map_err(|e| format!("Serve: {e}"))?;
    logicaffeine_system::http::server::announce_serving(bound);

    // The SAME lifecycle the tree-walker and the compiled tier get, read from the
    // SAME place, so the three tiers cannot drift.
    let lifecycle = Lifecycle::new(LifecycleConfig::from_env());
    lifecycle.set_ready(true);
    let cfg = lifecycle.config();
    let (max_body, read_deadline, max_inflight) =
        (cfg.max_body, cfg.read_deadline, cfg.max_inflight);

    let shutdown = shutdown_requested();
    futures::pin_mut!(shutdown);
    let mut draining = false;
    let mut inflight: FuturesUnordered<StepFut> = FuturesUnordered::new();
    loop {
        if draining && inflight.is_empty() {
            return Ok(());
        }
        logicaffeine_system::tokio::select! {
            accepted = listener.accept(), if !draining && inflight.len() < max_inflight => {
                if let Ok(conn) = accepted {
                    inflight.push(read_step(conn, max_body, read_deadline));
                }
            }
            _ = &mut shutdown, if !draining => {
                lifecycle.set_ready(false);
                draining = true;
            }
            Some(step) = inflight.next() => {
                match step {
                    Step::Request(conn, Ok(req)) => {
                        let keep_alive = request_wants_keep_alive(&req);
                        if let Some(resp) = lifecycle.probe_response(&req.path) {
                            inflight.push(write_step(conn, resp, keep_alive, max_body, read_deadline));
                            continue;
                        }
                        let guard = match lifecycle.admit() {
                            Admission::Admit(g) => g,
                            Admission::NotReady | Admission::Overloaded => {
                                inflight.push(write_step(conn, service_unavailable(), false, max_body, read_deadline));
                                continue;
                            }
                        };
                        let request_value = crate::interpreter::Interpreter::http_request_to_value(&req);
                        // Re-enter the handler as VM bytecode; its `Show` output is
                        // dropped to stderr like the interpreter's (a running server
                        // never surfaces `lines`).
                        let (result, _handler_output) =
                            vm.call_handler_bytecode(handler, request_value);
                        let resp = match result {
                            Ok(value) => match crate::interpreter::Interpreter::value_to_http_response(&value) {
                                Ok(resp) => resp,
                                Err(e) => {
                                    eprintln!("Serve: handler error: {e}");
                                    internal_error()
                                }
                            },
                            Err(e) => {
                                eprintln!("Serve: handler error: {e}");
                                internal_error()
                            }
                        };
                        drop(guard);
                        lifecycle.notify_slot_freed();
                        inflight.push(write_step(conn, resp, keep_alive, max_body, read_deadline));
                    }
                    Step::Request(conn, Err(ReadError::Parse(ref e))) => {
                        // The SHARED read-error law, the same one the tree-walker and both
                        // native engines apply: a declared body over the limit is 413 so the
                        // client knows to send less, and everything else is 400. Answering a
                        // flat 400 here made this tier disagree with its own oracle.
                        inflight.push(write_step(conn, read_error_response(e), false, max_body, read_deadline));
                    }
                    Step::Request(_, Err(_)) | Step::Closed => {}
                }
            }
        }
    }
}

/// Run an IO program (peer networking and/or tcp.lg sockets) on the BYTECODE VM (single task, no
/// channels): drive the resumable VM and service each `VmBlock::Net*`/`VmBlock::Tcp` through the
/// shared `NetInbox`/`TcpRegistry` — the SAME seams the tree-walker uses — so networking and
/// sockets run byte-identically on both tiers. The cross-tier locks prove they agree.
pub async fn run_vm_io_async(input: &str) -> InterpreterResult {
    use logicaffeine_language::ast::stmt::{Expr, Stmt, TypeExpr};

    // The stdlib prelude splices exactly as on the tree-walker path — the tcp.lg
    // native declarations (and any other referenced modules) must exist on BOTH
    // tiers or the differential is meaningless.
    let input = crate::loader::apply_prelude(input);
    let library = crate::loader::LibraryProvenance::of(input.as_ref());
    let input: &str = &input;
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
    let _compile_facts = crate::optimize::facts::scope();
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
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
    let stmts = match parser.parse_program() {
        Ok(s) => s,
        Err(e) => {
            return InterpreterResult {
                lines: vec![],
                error: Some(socratic_explanation(&e, &Interner::new())),
            }
        }
    };
    let opt_flags = parser.program_opt_flags();
    let type_registry_for_vm = parser.into_type_registry();
    let mut run_cfg =
        crate::optimization::OptimizationConfig::from_env().merged(&opt_flags);
    run_cfg.normalize();
    let resolved = crate::resolve_program::resolve_program(
        &stmts,
        crate::ast_map::Arenas {
            exprs: &imperative_expr_arena,
            stmts: &stmt_arena,
            types: &type_expr_arena,
        },
        &mut interner,
        run_cfg.is_on(crate::optimization::Opt::Comptime),
        library.functions(),
    );
    let stmts: Vec<Stmt> = match resolved {
        Some(rw) => rw.to_vec(),
        None => stmts,
    };
    let program = match crate::vm::Compiler::compile_with_types(
        &stmts,
        &interner,
        Some(&type_registry_for_vm),
    ) {
        Ok(p) => p,
        Err(e) => {
            return InterpreterResult {
                lines: vec![],
                error: Some(e),
            }
        }
    };
    let mut vm = crate::vm::Vm::new(&program).with_policy_ctx(&policy_registry, &interner);
    let mut netbox = crate::concurrency::net_inbox::NetInbox::new();
    let mut tcpbox = crate::concurrency::tcp_registry::TcpRegistry::new();
    // Build the wire type registry from the analysis types — IDENTICALLY to the tree-walker's
    // `build_wire_type_registry` (both derive from the same analysis table) — so two same-program peers
    // advertise matching registry epochs and may elide type NAMES from the wire (type-id).
    {
        let mut structs: Vec<(String, Vec<String>)> = Vec::new();
        let mut enums: Vec<(String, Vec<String>)> = Vec::new();
        for (name_sym, type_def) in type_registry_for_vm.iter_types() {
            // Skip a compiler-provided struct (`HttpError`/`File`) the program never declared —
            // every program's analysis table carries both, so leaving them in would advertise a
            // non-zero epoch (and inflate the registry) for a program with no types of its own.
            if type_registry_for_vm.is_builtin_struct(*name_sym) {
                continue;
            }
            let type_name = interner.resolve(*name_sym).to_string();
            match type_def {
                crate::analysis::registry::TypeDef::Struct { fields, .. } => {
                    structs.push((
                        type_name,
                        fields
                            .iter()
                            .map(|f| interner.resolve(f.name).to_string())
                            .collect(),
                    ));
                }
                crate::analysis::registry::TypeDef::Enum { variants, .. } => {
                    enums.push((
                        type_name,
                        variants
                            .iter()
                            .map(|v| interner.resolve(v.name).to_string())
                            .collect(),
                    ));
                }
                _ => {}
            }
        }
        netbox.set_registry(
            crate::concurrency::marshal::WireTypeRegistry::new(structs).with_enums(enums),
        );
    }
    let mut lines: Vec<String> = Vec::new();
    let mut error = None;
    loop {
        let step = vm.run_until_block();
        lines.extend(vm.drain_lines());
        match step {
            Ok(crate::vm::VmStep::Done(_)) => break,
            Ok(crate::vm::VmStep::Blocked) => {
                let req = match vm.take_pending() {
                    Some(r) => r,
                    None => break,
                };
                // `Serve` owns the accept loop: it never resumes the top-level VM
                // (the loop runs until the host aborts the task), so it is driven
                // HERE rather than through `service_vm_net_block`. The loop only
                // returns on a fatal bind error — a served response never
                // surfaces through `lines` (a running server never completes),
                // exactly as on the tree-walker.
                if let crate::vm::VmBlock::Serve(port, handler) = req {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if let Err(e) = serve_vm_accept_loop(&vm, port, handler).await {
                            error = Some(e);
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        let _ = (port, handler);
                        error = Some(
                            "serve: networking is unavailable on the wasm32 target".to_string(),
                        );
                    }
                    break;
                }
                if let Err(e) = service_vm_net_block(&mut vm, &mut netbox, &mut tcpbox, req).await {
                    error = Some(e);
                    break;
                }
            }
            Ok(crate::vm::VmStep::Paused) => break,
            Err(e) => {
                error = Some(e);
                break;
            }
        }
    }
    InterpreterResult { lines, error }
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
/// `SpawnDesc` crosses a worker boundary; the worker rebuilds the `!Send` task
/// body locally from it.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_vm_workstealing_seeded(input: &str, seed: u64, workers: usize) -> InterpreterResult {
    use crate::concurrency::vm_driver::VmTask;
    use logicaffeine_runtime::{
        run_workstealing_seeded, RunOutcome, SchedSeed, SchedulerConfig, SpawnDesc, Task,
    };

    with_parsed_program(input, |parsed, interner| match parsed {
        Ok((stmts, type_registry, policies)) => {
            let program =
                match crate::vm::Compiler::compile_with_types(stmts, interner, Some(type_registry))
                {
                    Ok(p) => p,
                    Err(e) => {
                        return InterpreterResult {
                            lines: vec![],
                            error: Some(e),
                        }
                    }
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
            let main = SpawnDesc {
                func: 0,
                args: vec![],
                priority: 0,
                is_main: true,
            };
            let config = SchedulerConfig::default().with_workers(workers.max(1));
            let result = run_workstealing_seeded(config, SchedSeed(seed), main, build);
            let error = match result.outcome {
                RunOutcome::Deadlock => Some("deadlock: every task is blocked waiting".to_string()),
                _ => None,
            };
            InterpreterResult {
                lines: result.output,
                error,
            }
        }
        Err(advice) => InterpreterResult {
            lines: vec![],
            error: Some(advice),
        },
    })
}

/// Run a program on the synchronous tree-walker and return its global
/// bindings as sorted `(name, type, value)` rows — the REPL's `:vars`
/// inspection. Returns `None` for programs the sync walker can't host
/// (async/concurrent) or that fail to parse or run.
pub fn repl_global_bindings(
    input: &str,
    program_args: &[String],
) -> Option<Vec<(String, String, String)>> {
    with_parsed_program(input, |parsed, interner| match parsed {
        Ok((stmts, type_registry, policies)) => {
            if crate::interpreter::needs_async(stmts, interner)
                || crate::concurrency::uses_scheduler(stmts)
            {
                return None;
            }
            let mut interp = crate::interpreter::Interpreter::new(interner)
                .with_type_registry(type_registry)
                .with_policies(policies)
                .with_program_args(program_args.to_vec());
            interp.run_sync(stmts).ok()?;
            Some(interp.global_bindings())
        }
        Err(_) => None,
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
            stmts,
            type_registry,
            policies,
            interner,
            program_args,
            Some(std::sync::Arc::new(logicaffeine_system::fs::MemVfs::new())),
            None,
            0,
        );
    }
    let mut interp = crate::interpreter::Interpreter::new(interner)
        .with_type_registry(type_registry)
        .with_policies(policies)
        .with_program_args(program_args.to_vec())
        .with_vfs(std::sync::Arc::new(logicaffeine_system::fs::MemVfs::new()));
    let run_result = if force_async {
        futures::executor::block_on(interp.run(stmts))
    } else {
        interp.run_sync(stmts)
    };
    // ★ AN UNHANDLED ACTION FAILURE IS THE PROGRAM'S ERROR, reported once the run has finished
    // rather than thrown when it happened. Thrown, it would stop a `Replay` at its first trap;
    // dropped, an author gets a button that silently does nothing. A real error the program raised
    // outranks it — that one stopped the run, and it is the one that explains the rest.
    let unhandled = interp.take_page_failure();
    InterpreterResult {
        error: run_result.err().or(unhandled),
        lines: interp.output,
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
pub async fn interpret_streaming<F>(
    input: &str,
    on_output: std::rc::Rc<std::cell::RefCell<F>>,
) -> InterpreterResult
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
    use crate::interpreter::OutputCallback;
    use logicaffeine_language::ast::stmt::{Expr, Stmt, TypeExpr};
    begin_ui_run();

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let mut tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    // Resolve the qualified escape hatch for cross-module calls (`Alias::fn` →
    // bare `fn`); no-op for single-file programs.
    let functions = crate::analysis::assemble::defined_function_names(input);
    crate::analysis::assemble::resolve_qualified_names(&mut tokens, &mut interner, &functions);

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
    let _compile_facts = crate::optimize::facts::scope();
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
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
    let parsed = parser.parse_program();
    let opt_flags = parser.program_opt_flags();
    let type_registry_for_interp = parser.into_type_registry();

    match parsed {
        Ok(stmts) => {
            let mut run_cfg =
                crate::optimization::OptimizationConfig::from_env().merged(&opt_flags);
            run_cfg.normalize();
            let resolved = crate::resolve_program::resolve_program(
                &stmts,
                crate::ast_map::Arenas {
                    exprs: &imperative_expr_arena,
                    stmts: &stmt_arena,
                    types: &type_expr_arena,
                },
                &mut interner,
                run_cfg.is_on(crate::optimization::Opt::Comptime),
                &crate::loader::library_owned_functions(input),
            );
            let stmts: Vec<Stmt> = match resolved {
                Some(rw) => rw.to_vec(),
                None => stmts,
            };
            if let Some(rejection) = send_escape_rejection(&stmts) {
                return rejection;
            }
            // Create the callback wrapper that calls the user's callback
            let callback: OutputCallback =
                std::rc::Rc::new(std::cell::RefCell::new(move |line: String| {
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

use crate::kernel;
use logicaffeine_language::ast::Stmt;
use logicaffeine_language::proof_convert::logic_expr_to_proof_expr;

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
    let (proof_exprs, goal_expr, definitions, declared) = theorem_problem_declared(input)?;

    // === STEPS 3-6: Prove → certify → type-check (the one canonical pipeline), BY THE
    // METHOD THE BLOCK DECLARED — see `prove_declared`. ===
    let (outcome, _) = prove_declared(&proof_exprs, &goal_expr, &definitions, &declared);
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
    let (proof_exprs, goal_expr, definitions, declared) = theorem_problem_declared(input)?;
    let (outcome, _) = prove_declared(&proof_exprs, &goal_expr, &definitions, &declared);
    Ok(TheoremTrace {
        verified: outcome.verified,
        premises: proof_exprs.iter().map(|p| p.to_string()).collect(),
        goal: goal_expr.to_string(),
        trace: outcome.derivation.as_ref().map(|d| d.display_tree()),
        error: outcome.verification_error,
    })
}

/// Build an INDEPENDENTLY RE-CHECKABLE certificate for a proved English `## Theorem`.
///
/// A `largo prove` ✓ is a boolean the caller must trust the prover for. A
/// [`kernel::certificate::TheoremCertificate`] is instead a portable artifact — the
/// kernel proof term, the goal it proves, and the theorem's DISCLOSED environment delta
/// (its declared vocabulary + stated premises as hypotheses) — that a third party can
/// re-verify with nothing but the kernel, by TYPE-CHECKING the term, never by re-running
/// proof search.
///
/// Returns `Ok(None)` when the theorem block parses but is NOT kernel-certified (an
/// unproved goal has no proof term, so no certificate exists — the honest outcome).
/// Only a genuine kernel derivation yields a certificate; there is no finite-enumeration
/// path here that could mint one.
pub fn theorem_certificate(
    input: &str,
) -> Result<Option<kernel::certificate::TheoremCertificate>, ParseError> {
    let (proof_exprs, goal_expr, definitions, declared) = theorem_problem_declared(input)?;
    let (outcome, _) = prove_declared(&proof_exprs, &goal_expr, &definitions, &declared);
    // A derivation alone is NOT a proof: only a `verified` outcome carries a
    // kernel-certified proof term, and only that becomes a certificate.
    if !outcome.verified {
        return Ok(None);
    }
    let Some(proof_term) = outcome.proof_term else {
        return Ok(None);
    };
    let name = english_theorem_name(input);
    Ok(Some(build_theorem_certificate(
        &name,
        proof_term,
        &outcome.kernel_ctx,
    )))
}

/// The display name of an English `## Theorem: Name` block (mirrors the CLI's namer).
fn english_theorem_name(input: &str) -> String {
    input
        .lines()
        .find(|l| l.trim_start().starts_with("## Theorem"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, name)| name.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "theorem".to_string())
}

/// Assemble a [`kernel::certificate::TheoremCertificate`] from a proved theorem's kernel
/// proof term and its proving context. The certificate's claimed type is the term's
/// inferred type in that context; its disclosed delta is EVERY declaration/definition the
/// context holds beyond the standard prelude — the theorem's own vocabulary and premises.
///
/// The prelude baseline is subtracted so the delta carries ONLY the theorem-specific
/// assumptions, in the open, and the checker rebuilds the trusted prelude itself.
pub(crate) fn build_theorem_certificate(
    name: &str,
    proof_term: kernel::Term,
    ctx: &kernel::Context,
) -> kernel::certificate::TheoremCertificate {
    use kernel::certificate::{CertDeclaration, CertDefinition, TheoremCertificate};

    // The trusted baseline: the standard prelude. Anything in `ctx` NOT in the baseline is
    // the theorem's disclosed delta.
    let baseline = kernel::prelude::StandardLibrary::context();

    let declarations: Vec<CertDeclaration> = ctx
        .iter_declarations()
        .filter(|(n, _)| baseline.get_global(n).is_none())
        .map(|(n, ty)| CertDeclaration {
            name: n.to_string(),
            ty: ty.clone(),
        })
        .collect();
    let definitions: Vec<CertDefinition> = ctx
        .iter_definitions()
        .filter(|(n, _, _)| baseline.get_global(n).is_none())
        .map(|(n, ty, body)| CertDefinition {
            name: n.to_string(),
            ty: ty.clone(),
            body: body.clone(),
        })
        .collect();

    // The claimed type is what the term actually proves in the full context. If the term
    // somehow fails to type-check here it is not a real proof; a bogus fallback type would
    // simply be rejected by the independent re-checker, never silently accepted.
    let claimed_type = kernel::infer_type(ctx, &proof_term)
        .unwrap_or_else(|_| kernel::Term::Global("False".to_string()));

    TheoremCertificate::new(name, proof_term, claimed_type, declarations, definitions)
}

// ═══════════════════════════════════════════════════════════════════
// Proof / Math → Rust extraction (the Curry-Howard "Forge", UI-facing)
// ═══════════════════════════════════════════════════════════════════

/// Names the user defined — present in `ctx` but absent from a fresh kernel
/// context (i.e. excluding the StandardLibrary baseline). Inductives are listed
/// before definitions so the extracted module reads top-down; StandardLibrary
/// items are pulled in by [`extract_math_rust`] only when transitively needed.
fn user_defined_entries(ctx: &kernel::Context) -> Vec<String> {
    let base = Baseline::new();

    let mut entries = Vec::new();
    for (name, _) in ctx.iter_inductives() {
        // A user inductive extracts as an enum only if every constructor field is
        // a concrete Rust type. Dependent/indexed families (e.g. `Eq<Nat,n,…>`,
        // value-indexed proof types) and opaque/primitive types are skipped.
        if !base.owns_inductive(name) && inductive_is_emittable(ctx, name) {
            entries.push(name.to_string());
        }
    }
    for (name, ty, body) in ctx.iter_definitions() {
        // Only data-typed definitions whose body references nothing inextractable
        // (no Prop proofs, no axioms/tactics like `syn_diag`/`try_auto`) extract to
        // runnable Rust — so we never emit Rust that references undefined symbols.
        if !base.owns_definition(name, ty, body)
            && type_is_emittable(ctx, ty, &[])
            && crate::extraction::is_extractable(ctx, name)
        {
            entries.push(name.to_string());
        }
    }
    // The module lists its items by name, whatever order they were declared in.
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
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            type_is_emittable(ctx, param_type, generics)
                && type_is_emittable(ctx, body_type, generics)
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
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = cur
    {
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
        while let Term::Pi {
            param_type,
            body_type,
            ..
        } = cur
        {
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
    let checks: Vec<(String, Vec<kernel::Term>)> = property_checks(ctx)
        .into_iter()
        .map(|(n, _, p)| (n, p))
        .collect();
    // The "compiled mathematical object" runs and proves itself.
    Ok(format!(
        "{module}{}",
        math_demo_main(ctx, &entries, &checks)
    ))
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
    let base = Baseline::new();
    let mut notes: Vec<String> = Vec::new();
    for (name, ty, body) in ctx.iter_definitions() {
        if base.owns_definition(name, ty, body) || check_names.contains(name) {
            continue;
        }
        if def_type_is_proposition(ty) {
            let name = ctx.written_name(name);
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
    let base = Baseline::new();
    let mut checks: Vec<(String, String, Vec<kernel::Term>)> = Vec::new();
    for (name, ty, body) in ctx.iter_definitions() {
        if base.owns_definition(name, ty, body) {
            continue;
        }
        if let Some(check_fn) = crate::extraction::emit_property_check(ctx, name, ty) {
            checks.push((name.to_string(), check_fn, pi_param_types(ty)));
        }
    }
    checks.sort_by(|a, b| a.0.cmp(&b.0));
    checks
}

/// The StandardLibrary baseline: what a fresh `Repl::new()` declares, and AS WHAT.
///
/// A NAME is not an identity. `add_zero`, `mul_zero`, `le_add_mono`, `Nat_add` — the prelude's
/// integer laws and structural arithmetic — are all perfectly ordinary names for a user's own
/// theorem, and compared by name alone a user's
/// `Definition add_zero : Eq MyNat (my_add MZero MZero) MZero` IS the prelude's
/// `∀a:Int. a + 0 = a`: it vanishes from the extracted module with no `check_add_zero` and not
/// even the honest note that would say why. The comparison that holds is the one the trust
/// door already makes about an axiom footprint — a name the rebuilt prelude declares AT THE
/// SAME TYPE AND BODY is the prelude's; anything else at that name was introduced by this run.
///
/// INDUCTIVES are compared by name, because in this kernel a redeclared inductive is not a
/// second declaration: its constructors are APPENDED to the same family (which is why
/// `distinct_constructors` exists), so there is no rival declaration to compare against.
struct Baseline {
    inductives: std::collections::HashSet<String>,
    definitions: std::collections::HashMap<String, (kernel::Term, kernel::Term)>,
}

impl Baseline {
    fn new() -> Self {
        let baseline = kernel::interface::Repl::new();
        let base = baseline.context();
        Self {
            inductives: base.iter_inductives().map(|(n, _)| n.to_string()).collect(),
            definitions: base
                .iter_definitions()
                .map(|(n, ty, body)| (n.to_string(), (ty.clone(), body.clone())))
                .collect(),
        }
    }

    /// Is `name` the prelude's inductive (not a user type that reuses the spelling)?
    fn owns_inductive(&self, name: &str) -> bool {
        self.inductives.contains(name)
    }

    /// Is the definition `name : ty := body` the prelude's own, rather than a user
    /// declaration that happens to share its spelling?
    fn owns_definition(&self, name: &str, ty: &kernel::Term, body: &kernel::Term) -> bool {
        self.definitions
            .get(name)
            .is_some_and(|(bty, bbody)| bty == ty && bbody == body)
    }
}

/// Whether a term applies the `div`/`mod` arithmetic builtins (whose extracted Rust
/// `/`/`%` panics on a zero divisor) — used to skip them in the self-verifying demo.
fn term_uses_div_or_mod(term: &kernel::Term) -> bool {
    use kernel::Term;
    match term {
        Term::Global(n) => n == "div" || n == "mod",
        Term::App(f, a) => term_uses_div_or_mod(f) || term_uses_div_or_mod(a),
        Term::Lambda {
            param_type, body, ..
        } => term_uses_div_or_mod(param_type) || term_uses_div_or_mod(body),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => term_uses_div_or_mod(param_type) || term_uses_div_or_mod(body_type),
        Term::Fix { body, .. } => term_uses_div_or_mod(body),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
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
        // The Rust item a key was emitted as, and the name its author wrote, which a key that a
        // shadow minted is not.
        let (item, written) = (
            crate::extraction::rust_global(ctx, name),
            ctx.written_name(name),
        );
        // Skip exercising a def that uses `div`/`mod`: the demo samples Int as 0, and
        // `n / n` / `n % n` at n=0 is a compile-time divide-by-zero panic (and the
        // kernel leaves div-by-0 stuck, diverging from Rust's panic). The function is
        // still EXTRACTED — just not run in the self-verifying demo.
        if term_uses_div_or_mod(body) {
            continue;
        }
        if matches!(body, Term::Lambda { .. } | Term::Fix { .. }) {
            // A function: apply it to kernel-built sample arguments.
            let Some(ty) = ctx.get_definition_type(name) else {
                continue;
            };
            let params = pi_param_types(ty);
            let samples: Option<Vec<Term>> = params.iter().map(|p| sample_value(ctx, p)).collect();
            let Some(samples) = samples else { continue }; // can't sample → skip
            if samples.is_empty() {
                continue;
            }
            let args_rust: Vec<String> = samples
                .iter()
                .map(|s| crate::extraction::emit_value(ctx, s))
                .collect();
            let mut app = Term::Global(name.clone());
            for s in &samples {
                app = Term::App(Box::new(app), Box::new(s.clone()));
            }
            let expected = crate::extraction::emit_value(ctx, &kernel::normalize(ctx, &app));
            let call = format!("{}({})", item, args_rust.join(", "));
            lines.push(format!("    assert_eq!({call}, {expected});"));
            lines.push(format!("    println!(\"{written}(..) = {{:?}}\", {call});"));
        } else {
            // A value: evaluate and self-verify.
            let expected = crate::extraction::emit_value(ctx, &kernel::normalize(ctx, body));
            lines.push(format!("    assert_eq!({item}(), {expected});"));
            lines.push(format!("    println!(\"{written} = {{:?}}\", {item}());"));
        }
    }
    // Run each proven theorem's property check on a sample and assert it holds.
    for (name, param_types) in checks {
        let samples: Option<Vec<kernel::Term>> =
            param_types.iter().map(|p| sample_value(ctx, p)).collect();
        let Some(samples) = samples else { continue };
        let args: Vec<String> = samples
            .iter()
            .map(|s| crate::extraction::emit_value(ctx, s))
            .collect();
        let (item, written) = (
            crate::extraction::rust_global(ctx, name),
            ctx.written_name(name),
        );
        let call = format!("check_{}({})", item, args.join(", "));
        lines.push(format!(
            "    assert!({call}, \"theorem {written} failed on sample\");"
        ));
        lines.push(format!(
            "    println!(\"\\u{{2713}} {written} holds (checked on a sample)\");"
        ));
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
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = cur
    {
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
    // Batch execution groups mutually-recursive inductives declared as separate
    // statements into one mutual block (see `Repl::execute_batch`); errors on individual
    // statements (e.g. `Check`/`Eval`) are ignored — only the resulting
    // definitions/inductives matter for extraction.
    let stmts = parse_math_statements(input);
    let _ = repl.execute_batch(&stmts);
    match extract_math_rust(repl.context()) {
        Ok(rust) => rust,
        Err(e) => format!("// extraction error: {e}"),
    }
}

/// Like [`extract_math_rust_from_source`], but produces the main-less module (the
/// linkable artifact bundled into an imperative program's `mod proven`).
pub fn extract_math_module_from_source(input: &str) -> String {
    let mut repl = kernel::interface::Repl::new();
    // Batch execution groups mutually-recursive inductives (`Tree`↔`Forest`) declared as
    // separate statements into one mutual block, so a forward reference between them
    // registers instead of failing its universe check. A source with no forward
    // references behaves exactly as the per-statement loop did.
    let stmts = parse_math_statements(input);
    let _ = repl.execute_batch(&stmts);
    match extract_math_module(repl.context()) {
        Ok(rust) => rust,
        Err(e) => format!("// extraction error: {e}"),
    }
}

/// IMPLICIT MAIN: a source with no `##` headers whose first non-blank line
/// reads as an imperative statement runs as its own `## Main` body, so a bare
/// script runs rather than parsing as logic prose.
/// Natural-language documents (no statement-keyword opening) are untouched.
/// Returns the wrapped source, or `None` when the input is not a bare script.
pub fn implicit_main(source: &str) -> Option<String> {
    if source.lines().any(|l| l.trim_start().starts_with("##")) {
        return None;
    }
    let first = source.lines().find(|l| !l.trim().is_empty())?;
    const STATEMENT_HEADS: &[&str] = &[
        "Let ",
        "Show ",
        "Set ",
        "If ",
        "While ",
        "Repeat ",
        "Push ",
        "Pop ",
        "Add ",
        "Remove ",
        "Call ",
        "Return ",
        "Increase ",
        "Decrease ",
        "Break",
        "Inspect ",
        "Assert ",
    ];
    let t = first.trim_start();
    if STATEMENT_HEADS.iter().any(|h| t.starts_with(h)) {
        Some(format!("## Main\n{}", source))
    } else {
        None
    }
}

/// Partition a possibly-MIXED document into its imperative and math streams.
///
/// A mixed document interleaves the language's blocks with the kernel's vernacular: Coq-style
/// sentences (`Definition`/`Inductive`/`Axiom`/`Theorem`/`Lemma`/`Fixpoint`) and literate
/// `## Theorem:`/`## Lemma:` blocks in the kernel's own form, the name followed by `Statement:`.
/// The math feeds the Forge (extracted into `mod proven`); the imperative blocks compile
/// normally and call into it.
///
/// What the language reads never reaches the kernel: `## To`/`## A X is one of` stay
/// imperative, an English `## Theorem:` (`Given:`/`Prove:`) is the proof engine's
/// (`opens_kernel_statement`), and a block the language reads as text owns its body to the
/// next header (`language_reads_as_text`), even where a line opens with a kernel keyword.
///
/// Returns `(imperative_src, Some(math_src))` when math is present, or
/// `(source.to_string(), None)` for a pure imperative program — so that path is a
/// guaranteed no-op (byte-identical compile). In `imperative_src` the math lines are
/// BLANKED (kept as empty lines), not deleted, so imperative line numbers — and thus
/// error spans — stay aligned with the original source.
pub fn partition_mixed(source: &str) -> (String, Option<String>) {
    if let Some(wrapped) = implicit_main(source) {
        return (wrapped, None);
    }
    let lines: Vec<&str> = source.lines().collect();
    let mut is_math = vec![false; lines.len()];
    let mut i = 0;
    let mut any = false;
    let mut in_text_block = false;
    while i < lines.len() {
        let t = lines[i].trim();
        if let Some(header) = t.strip_prefix("##") {
            in_text_block = language_reads_as_text(header);
        }
        if in_text_block || !is_math_block_start(t) {
            i += 1;
            continue;
        }
        let literate = t.starts_with("## Theorem:") || t.starts_with("## Lemma:");
        if literate && !opens_kernel_statement(&lines, i) {
            i += 1;
            continue;
        }
        any = true;
        if literate {
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
        "Definition ",
        "Inductive ",
        "Axiom ",
        "Theorem ",
        "Lemma ",
        "Fixpoint ",
    ];
    COQ.iter().any(|k| trimmed.starts_with(k))
        || trimmed.starts_with("## Theorem:")
        || trimmed.starts_with("## Lemma:")
}

/// Whether the block header `header` (the text after `##`) opens a block the language reads as
/// raw text — `## Axiom` and `## Theory`, whose bodies the parser captures whole
/// (`collect_block_text`) up to the next block header.
fn language_reads_as_text(header: &str) -> bool {
    let word = header
        .trim_start()
        .split(|c: char| c.is_whitespace() || c == ':')
        .next()
        .unwrap_or("");
    matches!(word.to_ascii_lowercase().as_str(), "axiom" | "theory")
}

/// Whether the literate block whose header is `lines[at]` is the kernel's: its name followed by
/// the kernel's `Statement:`, on the header line or the first line after it — the one form the
/// kernel's literate parser reads. Any other body (`Given:`/`Prove:`) is the proof engine's
/// English theorem, which the language reads.
fn opens_kernel_statement(lines: &[&str], at: usize) -> bool {
    let header = lines[at].trim();
    let after_colon = header
        .split_once(':')
        .map_or("", |(_, rest)| rest)
        .trim_start();
    let after_name = after_colon
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest)
        .trim_start();
    let next = if after_name.is_empty() {
        lines[at + 1..]
            .iter()
            .map(|l| l.trim())
            .find(|l| !l.is_empty())
            .unwrap_or("")
    } else {
        after_name
    };
    next.strip_prefix("Statement")
        .is_some_and(|rest| rest.trim_start().starts_with(':'))
}

/// Extract a mixed document's math stream into a bundleable `mod proven` body, or
/// `Ok(None)` if it has no public items to call into (e.g. only vacuous proofs) — so
/// we never emit a `use proven::*;` over an empty module.
///
/// Unlike standalone math extraction, this ALSO emits a `Showable` impl for each
/// non-generic proven enum, so imperative `Show <proven value>` works. `Showable`
/// lives in `logicaffeine_system` (in scope inside `mod proven` via `use super::*;`),
/// which is ONLY linked when bundled into an imperative program — hence it is added
/// here, not in `extract_math_module` (whose output must also compile standalone, dep-free).
///
/// A statement the kernel REFUSES refuses the program, in the kernel's own words and naming the
/// statement. Dropping it and extracting the rest leaves the imperative half calling a
/// `check_*` that was never emitted, and the reader is told "Unknown function" three layers
/// away from the refusal that caused it.
pub(crate) fn mixed_proven_module(math_src: &str) -> Result<Option<String>, ParseError> {
    let mut repl = kernel::interface::Repl::new();
    // Batch execution groups mutually-recursive inductives (see `Repl::execute_batch`).
    let stmts = parse_math_statements(math_src);
    for (stmt, outcome) in stmts.iter().zip(repl.execute_batch(&stmts)) {
        if let Err(refusal) = outcome {
            return Err(ParseError {
                kind: crate::error::ParseErrorKind::Custom(format!(
                    "the kernel refused the math statement `{}`: {refusal}",
                    stmt.trim()
                )),
                span: crate::token::Span::default(),
            });
        }
    }
    let ctx = repl.context();
    let mut rust = match extract_math_module(ctx) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    if !(rust.contains("pub fn") || rust.contains("pub enum") || rust.contains("pub struct")) {
        return Ok(None);
    }
    // Show-verb integration: bridge each non-generic proven enum to the runtime
    // `Showable` trait (via Debug). Sorted for deterministic output.
    let base = Baseline::new();
    let mut inds: Vec<String> = Vec::new();
    for (name, _) in ctx.iter_inductives() {
        if !base.owns_inductive(name)
            && inductive_is_emittable(ctx, name)
            && inductive_generics(ctx, name).is_empty()
        {
            inds.push(name.to_string());
        }
    }
    inds.sort();
    for ind in inds {
        let ind = crate::extraction::rust_global(ctx, &ind);
        rust.push_str(&format!(
            "impl Showable for {ind} {{ fn format_show(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {{ write!(f, \"{{:?}}\", self) }} }}\n"
        ));
    }
    Ok(Some(rust))
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
                let is_theorem_part =
                    next_trimmed.starts_with("Statement:") || next_trimmed.starts_with("Proof:");

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
        if (trimmed.starts_with("A ") || trimmed.starts_with("An "))
            && trimmed.contains(" is either")
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

/// Why Logic's Compile has no Rust to give for `input`, or `None` when it has one: a finite-domain
/// puzzle is solved by Execute, and extracting it would run the whole solver — the same test
/// [`extract_logic_rust`] makes before it bails.
pub fn logic_compile_note(input: &str) -> Option<&'static str> {
    match theorem_proof_exprs(input) {
        Ok((premises, _)) if looks_like_grid(&premises) => {
            Some("A finite-domain puzzle is solved by Execute \u{2014} compiling it would run the whole solver to extract it.")
        }
        _ => None,
    }
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
                premises
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                goal
            )
        };
        return Ok(emit(&premises, &goal, input, &fol));
    }

    // A plain sentence: model-check the single formula.
    let proof = compile_for_proof(input);
    if let Some(expr) = proof.proof_expr {
        let fol = proof
            .logic_string
            .clone()
            .unwrap_or_else(|| expr.to_string());
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
    let (proof_exprs, goal, defaults, _definitions, _declared) = theorem_problem(input, true)?;
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
    let (proof_exprs, _goal, defaults, _definitions, _declared) = theorem_problem(input, true)?;
    let theory = smt_theory_for(&proof_exprs, None);
    Ok(crate::defeasible::defeasible_consistent(
        &proof_exprs,
        &defaults,
        &theory,
    ))
}

/// Parse a `## Theorem:` block and convert its premises and goal to
/// [`ProofExpr`]s — the shared front half of [`verify_theorem`] and
/// `check_theorem_smt`.
/// Parse a `## Theorem` document and convert its premises and goal to
/// [`ProofExpr`]. Public so a puzzle solver can obtain the parsed-FOL premises
/// (the Given clues/declarations) and feed them to the entailment oracle.
pub fn theorem_proof_exprs(input: &str) -> Result<(Vec<ProofExpr>, ProofExpr), ParseError> {
    let (premises, goal, _defaults, _definitions, _declared) = theorem_problem(input, false)?;
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
    let (premises, goal, _defaults, definitions, _declared) = theorem_problem(input, false)?;
    Ok((premises, goal, definitions))
}

/// [`theorem_proof_exprs_with_defs`] plus the `Proof:` line the block DECLARED — the entry
/// every kernel-certified Surface A door uses, so none of them can prove a theorem by a
/// method its author did not write (see [`prove_declared`]).
fn theorem_problem_declared(
    input: &str,
) -> Result<
    (
        Vec<ProofExpr>,
        ProofExpr,
        Vec<logicaffeine_proof::verify::Definition>,
        DeclaredProof,
    ),
    ParseError,
> {
    let (premises, goal, _defaults, definitions, declared) = theorem_problem(input, false)?;
    Ok((premises, goal, definitions, declared))
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
    let is_var =
        |t: &ProofTerm| matches!(t, ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if v == var);
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
    let is_var =
        |t: &ProofTerm| matches!(t, ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if v == var);
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
                            out.push(GridClosure {
                                var: variable.clone(),
                                row_sort,
                                disjuncts,
                            });
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
            map.entry(category_stem(word))
                .or_insert_with(|| word.to_string());
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
    let display = |v: &str| {
        surface
            .get(&category_stem(v))
            .cloned()
            .unwrap_or_else(|| v.to_string())
    };
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
        columns.push(GridColumn {
            label,
            values,
            cells,
        });
    }
    Some(SolvedGrid {
        row_label: title_case(&row_sort),
        rows,
        columns,
    })
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
        ProofExpr::Counterfactual {
            antecedent,
            consequent,
        } => {
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
        ProofExpr::And(l, r) => ProofExpr::And(Box::new(erase_tense(l)), Box::new(erase_tense(r))),
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
        DeclaredProof,
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
            kind: logicaffeine_language::error::ParseErrorKind::Custom(
                "No theorem block found in input".to_string(),
            ),
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

    Ok((
        proof_exprs,
        goal_expr,
        defaults,
        definitions,
        declared_proof(theorem),
    ))
}
