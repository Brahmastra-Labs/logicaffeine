/// Maximum number of readings in a parse forest.
/// Prevents exponential blowup from ambiguous sentences.
pub const MAX_FOREST_READINGS: usize = 12;

pub mod arena;
pub mod arena_ctx;
pub mod ast;
pub mod audio;
pub mod codegen;
#[cfg(not(target_arch = "wasm32"))]
pub mod compile;
// Diagnostic Bridge for ownership error translation
#[cfg(not(target_arch = "wasm32"))]
pub mod diagnostic;
#[cfg(not(target_arch = "wasm32"))]
pub mod sourcemap;
pub mod content;
pub mod context;
pub mod debug;
pub mod drs;
pub mod error;
pub mod formatter;
pub mod game;
pub mod generator;
pub mod grader;
pub mod achievements;
pub mod analysis;
pub mod intern;
pub mod lambda;
pub mod lexer;
pub mod lexicon;
pub mod mwe;
pub mod ontology;
pub mod parser;
pub mod pragmatics;
pub mod progress;
#[cfg(not(target_arch = "wasm32"))]
pub mod project;
#[cfg(all(not(target_arch = "wasm32"), feature = "cli"))]
pub mod cli;
#[cfg(all(not(target_arch = "wasm32"), feature = "verification"))]
pub mod verification;
pub mod runtime_lexicon;
pub mod semantics;
pub mod registry;
pub mod scope;
#[cfg(target_arch = "wasm32")]
pub mod storage;
pub mod srs;
pub mod style;
pub mod unlock;
pub mod learn_state;
pub mod symbol_dict;
pub mod struggle;
pub mod suggest;
pub mod token;
pub mod transpile;
pub mod ui;
pub mod view;
pub mod visitor;
pub mod interpreter;

pub mod test_utils;

pub use analysis::{TypeRegistry, TypeDef, DiscoveryPass, scan_dependencies, Dependency};
#[cfg(not(target_arch = "wasm32"))]
pub use analysis::discover_with_imports;
#[cfg(not(target_arch = "wasm32"))]
pub use project::{Loader, ModuleSource};
#[cfg(not(target_arch = "wasm32"))]
pub use compile::copy_logos_core;
pub use arena::Arena;
pub use arena_ctx::AstContext;
pub use ast::{LogicExpr, NounPhrase, Term, ThematicRole};
pub use context::{DiscourseContext, OwnershipState, TimeConstraint, TimeRelation};
pub use error::{ParseError, ParseErrorKind, socratic_explanation};
pub use debug::{DebugWorld, DisplayWith, WithInterner};
pub use formatter::{LatexFormatter, LogicFormatter, UnicodeFormatter};
pub use intern::{Interner, Symbol, SymbolEq};
pub use lexer::Lexer;
pub use parser::{Parser, ParserMode};
pub use parser::QuantifierParsing;
pub use registry::SymbolRegistry;
pub use scope::{ScopeStack, ScopeEntry};
pub use token::{BlockType, Token, TokenType};
pub use view::{ExprView, NounPhraseView, Resolve, TermView};
pub use visitor::{Visitor, walk_expr, walk_term, walk_np};
pub use interpreter::{Interpreter, InterpreterResult, RuntimeValue};

// ═══════════════════════════════════════════════════════════════════
// Output Format Configuration
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Unicode,
    LaTeX,
    SimpleFOL,
}

// ═══════════════════════════════════════════════════════════════════
// Transpile Context
// ═══════════════════════════════════════════════════════════════════

pub struct TranspileContext<'a> {
    pub registry: &'a mut SymbolRegistry,
    pub interner: &'a Interner,
}

impl<'a> TranspileContext<'a> {
    pub fn new(registry: &'a mut SymbolRegistry, interner: &'a Interner) -> Self {
        TranspileContext { registry, interner }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CompileOptions {
    pub format: OutputFormat,
}

impl Default for CompileOptions {
    fn default() -> Self {
        CompileOptions {
            format: OutputFormat::Unicode,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Public API
// ═══════════════════════════════════════════════════════════════════

pub fn compile(input: &str) -> Result<String, ParseError> {
    compile_with_options(input, CompileOptions::default())
}

pub fn compile_simple(input: &str) -> Result<String, ParseError> {
    compile_with_options(input, CompileOptions {
        format: OutputFormat::SimpleFOL,
    })
}

pub fn compile_with_options(input: &str, options: CompileOptions) -> Result<String, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    // Apply MWE collapsing
    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    // Pass 1: Discovery - scan for type definitions
    let type_registry = {
        let mut discovery = analysis::DiscoveryPass::new(&tokens, &mut interner);
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

    // Pass 2: Parse with type context
    let mut discourse = DiscourseContext::new();
    let mut parser = Parser::with_types(tokens, &mut discourse, &mut interner, ctx, type_registry);
    let ast = parser.parse()?;
    let ast = semantics::apply_axioms(ast, ctx.exprs, ctx.terms, &mut interner);
    let ast = pragmatics::apply_pragmatics(ast, ctx.exprs, &interner);
    let mut registry = SymbolRegistry::new();
    let main_output = ast.transpile(&mut registry, &interner, options.format);

    let constraints = discourse.time_constraints();
    if constraints.is_empty() {
        Ok(main_output)
    } else {
        let constraint_strs: Vec<String> = constraints.iter().map(|c| {
            match c.relation {
                TimeRelation::Precedes => format!("Precedes({}, {})", c.left, c.right),
                TimeRelation::Equals => format!("{}={}", c.left, c.right),
            }
        }).collect();
        Ok(format!("{} ∧ {}", main_output, constraint_strs.join(" ∧ ")))
    }
}

pub fn compile_with_context(input: &str, ctx: &mut DiscourseContext) -> Result<String, ParseError> {
    compile_with_context_options(input, ctx, CompileOptions::default())
}

pub fn compile_with_context_options(
    input: &str,
    ctx: &mut DiscourseContext,
    options: CompileOptions,
) -> Result<String, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    // Apply MWE collapsing
    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    // Pass 1: Discovery - scan for type definitions
    let type_registry = {
        let mut discovery = analysis::DiscoveryPass::new(&tokens, &mut interner);
        discovery.run()
    };

    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();

    let ast_ctx = AstContext::new(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
    );

    // Pass 2: Parse with type context
    let mut parser = Parser::with_types(tokens, ctx, &mut interner, ast_ctx, type_registry);
    let ast = parser.parse()?;
    let mut registry = SymbolRegistry::new();
    Ok(ast.transpile(&mut registry, &interner, options.format))
}

pub fn compile_discourse(sentences: &[&str]) -> Result<String, ParseError> {
    compile_discourse_with_options(sentences, CompileOptions::default())
}

pub fn compile_discourse_with_options(sentences: &[&str], options: CompileOptions) -> Result<String, ParseError> {
    let mut interner = Interner::new();
    let mut ctx = DiscourseContext::new();
    let mut results = Vec::new();
    let mut registry = SymbolRegistry::new();
    let mwe_trie = mwe::build_mwe_trie();

    for sentence in sentences {
        let event_var_name = ctx.next_event_var();
        let event_var_symbol = interner.intern(&event_var_name);

        let mut lexer = Lexer::new(sentence, &mut interner);
        let tokens = lexer.tokenize();

        // Apply MWE collapsing
        let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

        // Pass 1: Discovery - scan for type definitions
        let type_registry = {
            let mut discovery = analysis::DiscoveryPass::new(&tokens, &mut interner);
            discovery.run()
        };

        let expr_arena = Arena::new();
        let term_arena = Arena::new();
        let np_arena = Arena::new();
        let sym_arena = Arena::new();
        let role_arena = Arena::new();
        let pp_arena = Arena::new();

        let ast_ctx = AstContext::new(
            &expr_arena,
            &term_arena,
            &np_arena,
            &sym_arena,
            &role_arena,
            &pp_arena,
        );

        // Pass 2: Parse with type context
        let mut parser = Parser::with_types(tokens, &mut ctx, &mut interner, ast_ctx, type_registry);
        parser.set_discourse_event_var(event_var_symbol);
        let ast = parser.parse()?;
        results.push(ast.transpile(&mut registry, &interner, options.format));
    }

    let event_history = ctx.event_history();
    let mut precedes = Vec::new();
    for i in 0..event_history.len().saturating_sub(1) {
        precedes.push(format!("Precedes({}, {})", event_history[i], event_history[i + 1]));
    }

    if precedes.is_empty() {
        Ok(results.join(" ∧ "))
    } else {
        Ok(format!("{} ∧ {}", results.join(" ∧ "), precedes.join(" ∧ ")))
    }
}

/// Returns all possible scope readings for a sentence.
/// For sentences with multiple quantifiers, this returns all permutations.
/// Example: "Every woman loves a man" returns both:
///   - Surface: ∀x(Woman(x) → ∃y(Man(y) ∧ Loves(x, y)))
///   - Inverse: ∃y(Man(y) ∧ ∀x(Woman(x) → Loves(x, y)))
pub fn compile_all_scopes(input: &str) -> Result<Vec<String>, ParseError> {
    compile_all_scopes_with_options(input, CompileOptions::default())
}

pub fn compile_all_scopes_with_options(input: &str, options: CompileOptions) -> Result<Vec<String>, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    // Apply MWE collapsing
    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    // Pass 1: Discovery - scan for type definitions
    let type_registry = {
        let mut discovery = analysis::DiscoveryPass::new(&tokens, &mut interner);
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

    // Pass 2: Parse with type context
    let mut discourse = DiscourseContext::new();
    let mut parser = Parser::with_types(tokens, &mut discourse, &mut interner, ctx, type_registry);
    let ast = parser.parse()?;

    let scope_arena = Arena::new();
    let scope_term_arena = Arena::new();
    let scopings = lambda::enumerate_scopings(ast, &mut interner, &scope_arena, &scope_term_arena);

    let intensional_arena = Arena::new();
    let intensional_term_arena = Arena::new();
    let intensional_role_arena: Arena<(ast::ThematicRole, ast::Term)> = Arena::new();

    let mut results = Vec::new();
    for scoped_expr in scopings {
        let intensional_readings = lambda::enumerate_intensional_readings(
            scoped_expr,
            &mut interner,
            &intensional_arena,
            &intensional_term_arena,
            &intensional_role_arena,
        );
        for reading in intensional_readings {
            let mut registry = SymbolRegistry::new();
            results.push(reading.transpile(&mut registry, &interner, options.format));
        }
    }

    Ok(results)
}

pub fn compile_ambiguous(input: &str) -> Result<Vec<String>, ParseError> {
    compile_ambiguous_with_options(input, CompileOptions::default())
}

pub fn compile_ambiguous_with_options(input: &str, options: CompileOptions) -> Result<Vec<String>, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    // Apply MWE collapsing
    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    // Pass 1: Discovery - scan for type definitions
    let type_registry = {
        let mut discovery = analysis::DiscoveryPass::new(&tokens, &mut interner);
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

    // Pass 2: Parse with type context
    let mut discourse = DiscourseContext::new();
    let mut parser = Parser::with_types(tokens.clone(), &mut discourse, &mut interner, ctx, type_registry.clone());
    let ast = parser.parse()?;
    let mut registry = SymbolRegistry::new();
    let reading1 = ast.transpile(&mut registry, &interner, options.format);

    let has_pp_ambiguity = tokens.iter().any(|t| {
        if let token::TokenType::Preposition(sym) = &t.kind {
            let prep = interner.resolve(*sym);
            prep == "with" || prep == "by" || prep == "for"
        } else {
            false
        }
    });

    if has_pp_ambiguity {
        let expr_arena2 = Arena::new();
        let term_arena2 = Arena::new();
        let np_arena2 = Arena::new();
        let sym_arena2 = Arena::new();
        let role_arena2 = Arena::new();
        let pp_arena2 = Arena::new();

        let ctx2 = AstContext::new(
            &expr_arena2,
            &term_arena2,
            &np_arena2,
            &sym_arena2,
            &role_arena2,
            &pp_arena2,
        );

        let mut discourse2 = DiscourseContext::new();
        let mut parser2 = Parser::with_types(tokens, &mut discourse2, &mut interner, ctx2, type_registry);
        parser2.set_pp_attachment_mode(true);
        let ast2 = parser2.parse()?;
        let mut registry2 = SymbolRegistry::new();
        let reading2 = ast2.transpile(&mut registry2, &interner, options.format);

        if reading1 != reading2 {
            return Ok(vec![reading1, reading2]);
        }
    }

    Ok(vec![reading1])
}

/// Phase 12: Parse Forest - Returns all valid readings for ambiguous sentences.
/// Handles lexical ambiguity (Noun/Verb) and structural ambiguity (PP attachment).
pub fn compile_forest(input: &str) -> Vec<String> {
    compile_forest_with_options(input, CompileOptions::default())
}

pub fn compile_forest_with_options(input: &str, options: CompileOptions) -> Vec<String> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    // Apply MWE collapsing
    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    // Pass 1: Discovery - scan for type definitions
    let type_registry = {
        let mut discovery = analysis::DiscoveryPass::new(&tokens, &mut interner);
        discovery.run()
    };

    let has_lexical_ambiguity = tokens.iter().any(|t| {
        matches!(t.kind, token::TokenType::Ambiguous { .. })
    });

    let has_pp_ambiguity = tokens.iter().any(|t| {
        if let token::TokenType::Preposition(sym) = &t.kind {
            let prep = interner.resolve(*sym);
            prep == "with" || prep == "by" || prep == "for"
        } else {
            false
        }
    });

    // Phase 18: Detect plurality ambiguity (mixed verb + plural subject)
    let has_mixed_verb = tokens.iter().any(|t| {
        if let token::TokenType::Verb { lemma, .. } = &t.kind {
            Lexer::is_mixed_verb(interner.resolve(*lemma))
        } else {
            false
        }
    });

    // Phase 19: Detect collective verbs (always require group reading with cardinals)
    let has_collective_verb = tokens.iter().any(|t| {
        if let token::TokenType::Verb { lemma, .. } = &t.kind {
            Lexer::is_collective_verb(interner.resolve(*lemma))
        } else {
            false
        }
    });

    let has_plural_subject = tokens.iter().any(|t| {
        matches!(t.kind, token::TokenType::Cardinal(_))
            || matches!(&t.kind, token::TokenType::Article(def) if matches!(def, lexicon::Definiteness::Definite))
    });

    let has_plurality_ambiguity = (has_mixed_verb || has_collective_verb) && has_plural_subject;

    // Phase 41: Detect event adjective + agentive noun ambiguity
    // "beautiful dancer" can mean: Beautiful(x) ∧ Dancer(x) OR ∃e(Dance(e) ∧ Agent(e,x) ∧ Beautiful(e))
    let has_event_adjective_ambiguity = {
        let mut has_event_adj = false;
        let mut has_agentive_noun = false;
        for token in &tokens {
            if let token::TokenType::Adjective(sym) = &token.kind {
                if lexicon::is_event_modifier_adjective(interner.resolve(*sym)) {
                    has_event_adj = true;
                }
            }
            if let token::TokenType::Noun(sym) = &token.kind {
                if lexicon::lookup_agentive_noun(interner.resolve(*sym)).is_some() {
                    has_agentive_noun = true;
                }
            }
        }
        has_event_adj && has_agentive_noun
    };

    let mut results: Vec<String> = Vec::new();

    // Reading 1: Default mode (verb priority for Ambiguous tokens)
    {
        let expr_arena = Arena::new();
        let term_arena = Arena::new();
        let np_arena = Arena::new();
        let sym_arena = Arena::new();
        let role_arena = Arena::new();
        let pp_arena = Arena::new();

        let ast_ctx = AstContext::new(
            &expr_arena,
            &term_arena,
            &np_arena,
            &sym_arena,
            &role_arena,
            &pp_arena,
        );

        // Pass 2: Parse with type context
        let mut discourse_ctx = context::DiscourseContext::new();
        let mut parser = Parser::with_types(tokens.clone(), &mut discourse_ctx, &mut interner, ast_ctx, type_registry.clone());
        parser.set_noun_priority_mode(false);

        if let Ok(ast) = parser.parse() {
            let mut registry = SymbolRegistry::new();
            results.push(ast.transpile(&mut registry, &interner, options.format));
        }
    }

    // Reading 2: Noun priority mode (for lexical ambiguity)
    if has_lexical_ambiguity {
        let expr_arena = Arena::new();
        let term_arena = Arena::new();
        let np_arena = Arena::new();
        let sym_arena = Arena::new();
        let role_arena = Arena::new();
        let pp_arena = Arena::new();

        let ast_ctx = AstContext::new(
            &expr_arena,
            &term_arena,
            &np_arena,
            &sym_arena,
            &role_arena,
            &pp_arena,
        );

        let mut discourse_ctx = context::DiscourseContext::new();
        let mut parser = Parser::with_types(tokens.clone(), &mut discourse_ctx, &mut interner, ast_ctx, type_registry.clone());
        parser.set_noun_priority_mode(true);

        if let Ok(ast) = parser.parse() {
            let mut registry = SymbolRegistry::new();
            let reading = ast.transpile(&mut registry, &interner, options.format);
            if !results.contains(&reading) {
                results.push(reading);
            }
        }
    }

    // Reading 3: PP attachment mode (for structural ambiguity)
    if has_pp_ambiguity {
        let expr_arena = Arena::new();
        let term_arena = Arena::new();
        let np_arena = Arena::new();
        let sym_arena = Arena::new();
        let role_arena = Arena::new();
        let pp_arena = Arena::new();

        let ast_ctx = AstContext::new(
            &expr_arena,
            &term_arena,
            &np_arena,
            &sym_arena,
            &role_arena,
            &pp_arena,
        );

        let mut discourse_ctx = context::DiscourseContext::new();
        let mut parser = Parser::with_types(tokens.clone(), &mut discourse_ctx, &mut interner, ast_ctx, type_registry.clone());
        parser.set_pp_attachment_mode(true);

        if let Ok(ast) = parser.parse() {
            let mut registry = SymbolRegistry::new();
            let reading = ast.transpile(&mut registry, &interner, options.format);
            if !results.contains(&reading) {
                results.push(reading);
            }
        }
    }

    // Reading 4: Collective mode (for plurality ambiguity with mixed verbs)
    if has_plurality_ambiguity {
        let expr_arena = Arena::new();
        let term_arena = Arena::new();
        let np_arena = Arena::new();
        let sym_arena = Arena::new();
        let role_arena = Arena::new();
        let pp_arena = Arena::new();

        let ast_ctx = AstContext::new(
            &expr_arena,
            &term_arena,
            &np_arena,
            &sym_arena,
            &role_arena,
            &pp_arena,
        );

        let mut discourse_ctx = context::DiscourseContext::new();
        let mut parser = Parser::with_types(tokens.clone(), &mut discourse_ctx, &mut interner, ast_ctx, type_registry.clone());
        parser.set_collective_mode(true);

        if let Ok(ast) = parser.parse() {
            // Transform cardinal quantifiers to group quantifiers for collective reading
            if let Ok(transformed) = parser.transform_cardinal_to_group(ast) {
                let mut registry = SymbolRegistry::new();
                let reading = transformed.transpile(&mut registry, &interner, options.format);
                if !results.contains(&reading) {
                    results.push(reading);
                }
            }
        }
    }

    // Reading 5: Event adjective mode (for event-modifying adjectives with agentive nouns)
    if has_event_adjective_ambiguity {
        let expr_arena = Arena::new();
        let term_arena = Arena::new();
        let np_arena = Arena::new();
        let sym_arena = Arena::new();
        let role_arena = Arena::new();
        let pp_arena = Arena::new();

        let ast_ctx = AstContext::new(
            &expr_arena,
            &term_arena,
            &np_arena,
            &sym_arena,
            &role_arena,
            &pp_arena,
        );

        let mut discourse_ctx = context::DiscourseContext::new();
        let mut parser = Parser::with_types(tokens.clone(), &mut discourse_ctx, &mut interner, ast_ctx, type_registry);
        parser.set_event_reading_mode(true);

        if let Ok(ast) = parser.parse() {
            let mut registry = SymbolRegistry::new();
            let reading = ast.transpile(&mut registry, &interner, options.format);
            if !results.contains(&reading) {
                results.push(reading);
            }
        }
    }

    // Enforce MAX_FOREST_READINGS limit
    results.truncate(MAX_FOREST_READINGS);

    results
}

// ═══════════════════════════════════════════════════════════════════
// UI API - For Live Transpilation & Visualization
// ═══════════════════════════════════════════════════════════════════

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TokenCategory {
    Quantifier,
    Noun,
    Verb,
    Adjective,
    Connective,
    Determiner,
    Preposition,
    Pronoun,
    Modal,
    Punctuation,
    Proper,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    pub start: usize,
    pub end: usize,
    pub text: String,
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
        | TokenType::Would | TokenType::Could | TokenType::May | TokenType::Cannot => TokenCategory::Modal,
        TokenType::Period | TokenType::Comma => TokenCategory::Punctuation,
        TokenType::ProperName(_) => TokenCategory::Proper,
        _ => TokenCategory::Other,
    }
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AstNode {
    pub label: String,
    pub node_type: String,
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

pub fn expr_to_ast_node(expr: &LogicExpr, interner: &Interner) -> AstNode {
    match expr {
        LogicExpr::Predicate { name, args } => {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileResult {
    pub logic: Option<String>,
    pub ast: Option<AstNode>,
    pub readings: Vec<String>,
    pub tokens: Vec<TokenInfo>,
    pub error: Option<String>,
}

pub fn compile_for_ui(input: &str) -> CompileResult {
    let tokens = tokenize_for_ui(input);
    let readings = compile_forest(input);

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let lex_tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let lex_tokens = mwe::apply_mwe_pipeline(lex_tokens, &mwe_trie, &mut interner);

    // Pass 1: Discovery - scan for type definitions
    let type_registry = {
        let mut discovery = analysis::DiscoveryPass::new(&lex_tokens, &mut interner);
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

    // Pass 2: Parse with type context
    let mut discourse = DiscourseContext::new();
    let mut parser = Parser::with_types(lex_tokens, &mut discourse, &mut interner, ctx, type_registry);

    match parser.parse() {
        Ok(ast) => {
            let ast_node = expr_to_ast_node(ast, &interner);
            let mut registry = SymbolRegistry::new();
            let logic = ast.transpile(&mut registry, &interner, OutputFormat::Unicode);

            CompileResult {
                logic: Some(logic),
                ast: Some(ast_node),
                readings,
                tokens,
                error: None,
            }
        }
        Err(e) => {
            let advice = socratic_explanation(&e, &interner);
            CompileResult {
                logic: None,
                ast: None,
                readings: Vec::new(),
                tokens,
                error: Some(advice),
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Imperative Interpreter API - For Guide Page Interactive Examples
// ═══════════════════════════════════════════════════════════════════

use crate::ast::stmt::{Stmt, Expr, TypeExpr};

/// Interpret LOGOS imperative code and return output lines.
/// This is used by the Guide page for interactive code examples.
/// Phase 55: Now async to support VFS operations.
pub async fn interpret_for_ui(input: &str) -> InterpreterResult {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    // Apply MWE collapsing (for consistency with compile pipeline)
    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    // Pass 1: Discovery - scan for type definitions
    let type_registry = {
        let mut discovery = analysis::DiscoveryPass::new(&tokens, &mut interner);
        discovery.run()
    };

    // Create arenas for AST allocation
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

    // Pass 2: Parse with type context (imperative mode)
    let mut discourse = DiscourseContext::new();
    let mut parser = Parser::with_types(tokens, &mut discourse, &mut interner, ctx, type_registry);

    match parser.parse_program() {
        Ok(stmts) => {
            let mut interp = interpreter::Interpreter::new(&interner);
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

#[cfg(test)]
mod tests {
    use super::*;

    // ═══════════════════════════════════════════════════════════════════
    // Phase 0: Output Format Configuration (FOL Upgrade)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn compile_with_unicode_default() {
        let result = compile("All men are mortal.").unwrap();
        assert!(
            result.contains("∀") || result.contains("→"),
            "Unicode format should use ∀ or →: got '{}'",
            result
        );
    }

    #[test]
    fn compile_with_latex_option() {
        let options = CompileOptions {
            format: OutputFormat::LaTeX,
        };
        let result = compile_with_options("All men are mortal.", options).unwrap();
        assert!(
            result.contains("\\forall") && result.contains("\\supset"),
            "LaTeX format should use \\forall and \\supset: got '{}'",
            result
        );
    }

    #[test]
    fn latex_uses_latex_operators() {
        let options = CompileOptions {
            format: OutputFormat::LaTeX,
        };
        let result = compile_with_options("If it is raining, then it is pouring.", options).unwrap();
        assert!(
            result.contains("\\supset"),
            "LaTeX format should use \\supset: got '{}'",
            result
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Phase 2: AST Structure Tests (FOL Upgrade)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn term_constant_creation() {
        use crate::ast::Term;
        let mut interner = Interner::new();
        let sym = interner.intern("Socrates");
        let term = Term::Constant(sym);
        assert!(matches!(term, Term::Constant(_)));
    }

    #[test]
    fn term_variable_creation() {
        use crate::ast::Term;
        let mut interner = Interner::new();
        let x = interner.intern("x");
        let term = Term::Variable(x);
        assert!(matches!(term, Term::Variable(_)));
    }

    #[test]
    fn predicate_unary() {
        use crate::ast::{LogicExpr, Term};
        let mut interner = Interner::new();
        let term_arena: Arena<Term> = Arena::new();
        let mortal = interner.intern("Mortal");
        let x = interner.intern("x");
        let expr = LogicExpr::Predicate {
            name: mortal,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        };
        assert!(matches!(expr, LogicExpr::Predicate { .. }));
    }

    #[test]
    fn predicate_binary() {
        use crate::ast::{LogicExpr, Term};
        let mut interner = Interner::new();
        let term_arena: Arena<Term> = Arena::new();
        let loves = interner.intern("Loves");
        let john = interner.intern("John");
        let mary = interner.intern("Mary");
        let expr = LogicExpr::Predicate {
            name: loves,
            args: term_arena.alloc_slice([
                Term::Constant(john),
                Term::Constant(mary),
            ]),
        };
        if let LogicExpr::Predicate { args, .. } = expr {
            assert_eq!(args.len(), 2);
        }
    }

    #[test]
    fn identity_expression() {
        use crate::ast::{LogicExpr, Term};
        let mut interner = Interner::new();
        let term_arena: Arena<Term> = Arena::new();
        let clark = interner.intern("Clark");
        let superman = interner.intern("Superman");
        let expr = LogicExpr::Identity {
            left: term_arena.alloc(Term::Constant(clark)),
            right: term_arena.alloc(Term::Constant(superman)),
        };
        assert!(matches!(expr, LogicExpr::Identity { .. }));
    }

    #[test]
    fn quantifier_universal() {
        use crate::ast::{LogicExpr, QuantifierKind, Term};
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();
        let x = interner.intern("x");
        let mortal = interner.intern("Mortal");
        let body = expr_arena.alloc(LogicExpr::Predicate {
            name: mortal,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        });
        let expr = LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: x,
            body,
            island_id: 0,
        };
        assert!(matches!(
            expr,
            LogicExpr::Quantifier {
                kind: QuantifierKind::Universal,
                ..
            }
        ));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Phase 3: Parser Desugaring Tests (FOL Upgrade)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn all_produces_universal_quantifier() {
        let result = compile("All men are mortal.").unwrap();
        assert!(
            result.contains("∀") && result.contains("→"),
            "All should produce ∀x(S(x) → P(x)): got '{}'",
            result
        );
    }

    #[test]
    fn some_produces_existential_quantifier() {
        let result = compile("Some cats are black.").unwrap();
        assert!(
            result.contains("∃") && result.contains("∧"),
            "Some should produce ∃x(S(x) ∧ P(x)): got '{}'",
            result
        );
    }

    #[test]
    fn no_produces_universal_negation() {
        let result = compile("No dogs are cats.").unwrap();
        assert!(
            result.contains("∀") && result.contains("¬"),
            "No should produce ∀x(S(x) → ¬P(x)): got '{}'",
            result
        );
    }

    #[test]
    fn multiple_quantifiers_have_unique_variables() {
        // Compound sentence with two quantified clauses
        let result = compile("All men are mortal and some cats are black.").unwrap();
        assert!(
            result.contains("x") && result.contains("y"),
            "Multiple quantifiers should have unique variables: got '{}'",
            result
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Phase 4: N-Ary Relations & Prepositions (FOL Upgrade)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn ternary_relation_with_to() {
        // Debug: check tokens
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("John gave the book to Mary.", &mut interner);
        let tokens = lexer.tokenize();
        eprintln!("Tokens: {:?}", tokens);

        let result = compile("John gave the book to Mary.").unwrap();
        eprintln!("Result: {}", result);

        // Should have 3 arguments (2 commas)
        let comma_count = result.matches(',').count();
        assert!(
            comma_count >= 2,
            "Ternary relation should have 3 args (2+ commas): got '{}'",
            result
        );
    }

    #[test]
    fn binary_relation_basic() {
        let result = compile("John loves Mary.").unwrap();
        assert!(
            (result.contains("Agent(e, J)") && result.contains("Theme(e, M)"))
                || result.contains("(J, M)"),
            "Binary relation should have Agent and Theme roles: got '{}'",
            result
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Legacy Tests (will be updated as we progress)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn dogs_and_dangerous_get_different_symbols() {
        let output = compile("All dogs are dangerous.").unwrap();
        assert_ne!(output, "All D is D", "dogs and dangerous should have different symbols");
    }

    #[test]
    fn men_and_mortal_get_different_symbols() {
        let output = compile("All men are mortal.").unwrap();
        assert_ne!(output, "All M is M", "men and mortal should have different symbols");
    }

    #[test]
    fn same_word_gets_same_symbol() {
        let output = compile("All cats are cats.").unwrap();
        // FOL output: ∀x((Cats(x) → Cats(x))) - same word appears twice
        let cats_count = output.matches("Cats(").count();
        assert!(
            cats_count >= 2,
            "same word should get same symbol (Cats appears twice): got '{}'",
            output
        );
    }

    #[test]
    fn compile_conditional() {
        let output = compile("If it is raining, then it is pouring.").unwrap();
        assert!(
            output.contains("→") || output.contains("\\supset"),
            "conditional should produce implication: got '{}'",
            output
        );
    }

    #[test]
    fn parse_adjective_noun_subject() {
        let output = compile("All old men are mortal.").unwrap();
        assert!(
            !output.contains("All O is"),
            "should not treat 'old' as the subject"
        );
    }

    #[test]
    fn parse_multiple_adjectives() {
        let output = compile("All old tired men are happy.").unwrap();
        assert!(
            !output.contains("All O is"),
            "should not treat first adjective as subject"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Phase 4: Transitive Verbs (Relations)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn parse_transitive_verb() {
        let output = compile("John loves Mary.").unwrap();
        assert!(
            (output.contains("Agent(e, J)") && output.contains("Theme(e, M)"))
                || output.contains("(J, M)"),
            "transitive verb should produce Agent/Theme roles or binary predicate: got '{}'",
            output
        );
    }

    #[test]
    fn parse_transitive_verb_symbols_unique() {
        let output = compile("John sees Jane.").unwrap();
        assert!(
            output.contains("J2") || output.contains("(J, J2)"),
            "John and Jane should get unique symbols: got '{}'",
            output
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Phase 5: Modal Vector Theory
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn parse_modal_must_alethic() {
        let output = compile("All cats must sleep.").unwrap();
        assert!(
            output.contains("□_{1.0}") || output.contains("\\Box_{1.0}"),
            "must should produce alethic necessity: got '{}'",
            output
        );
    }

    #[test]
    fn parse_modal_should_deontic() {
        let output = compile("All students should study.").unwrap();
        assert!(
            output.contains("O_{0.6}"),
            "should produces deontic obligation: got '{}'",
            output
        );
    }

    #[test]
    fn parse_modal_can_possibility() {
        let output = compile("Some birds can fly.").unwrap();
        assert!(
            output.contains("◇") || output.contains("\\Diamond"),
            "can should produce possibility: got '{}'",
            output
        );
    }

    #[test]
    fn parse_modal_cannot_impossibility() {
        let output = compile("All code cannot run.").unwrap();
        assert!(
            output.contains("□_{0.0}") || output.contains("\\Box_{0.0}"),
            "cannot should produce impossibility: got '{}'",
            output
        );
    }

    #[test]
    fn parse_compound_modal_sentence() {
        let output = compile("The user should compile and the code cannot run.").unwrap();
        assert!(
            output.contains("O_{0.6}") && (output.contains("□_{0.0}") || output.contains("\\Box_{0.0}")),
            "compound modal should have both operators: got '{}'",
            output
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Phase 1: Identity & Biconditional (TDD RED)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn identity_clark_is_superman() {
        let result = compile("Clark is equal to Superman.").unwrap();
        assert!(
            result.contains("="),
            "Identity should produce =: got '{}'",
            result
        );
    }

    #[test]
    fn identity_same_constant() {
        let result = compile("Socrates is identical to Socrates.").unwrap();
        assert!(
            result.contains("Socrates = Socrates"),
            "Same constant should appear twice: got '{}'",
            result
        );
    }

    #[test]
    fn iff_produces_biconditional() {
        let result = compile("A if and only if B.").unwrap();
        assert!(
            result.contains("↔"),
            "Iff should produce ↔: got '{}'",
            result
        );
    }

    #[test]
    fn iff_latex_uses_equiv() {
        let options = CompileOptions {
            format: OutputFormat::LaTeX,
        };
        let result = compile_with_options("A if and only if B.", options).unwrap();
        assert!(
            result.contains("\\equiv"),
            "LaTeX Iff should use \\equiv: got '{}'",
            result
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Phase 2: Reflexive Binding (TDD RED)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn reflexive_binds_to_subject() {
        let result = compile("John loves himself.").unwrap();
        assert!(
            (result.contains("Agent(e, J)") && result.contains("Theme(e, J)"))
                || result.contains("(J, J)"),
            "Reflexive should bind Agent and Theme to same entity: got '{}'",
            result
        );
    }

    #[test]
    fn reflexive_with_herself() {
        let result = compile("Mary sees herself.").unwrap();
        assert!(
            (result.contains("Agent(e, M)") && result.contains("Theme(e, M)"))
                || result.contains("(M, M)"),
            "Reflexive herself should bind: got '{}'",
            result
        );
    }

    #[test]
    fn reflexive_in_prepositional_phrase() {
        let result = compile("John gave the book to himself.").unwrap();
        assert!(
            result.contains("Agent(e, J)") && result.contains("Theme(e, B)")
                || result.contains("(J, B, J)"),
            "Reflexive in preposition should bind to subject: got '{}'",
            result
        );
    }

    #[test]
    fn relative_clause_with_preposition() {
        let result = compile("All dogs that ran to the house are tired.").unwrap();
        assert!(
            result.contains("Run(x, House)"),
            "Relative clause should support prepositions: got '{}'",
            result
        );
    }

    #[test]
    fn relative_clause_with_reflexive_preposition() {
        let result = compile("All men that speak to themselves are wise.").unwrap();
        assert!(
            result.contains("Speak(x, x)"),
            "Relative clause reflexive should bind to variable: got '{}'",
            result
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Phase 3: Relative Clauses & Adjective Predicates (TDD RED)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn adjectives_as_separate_predicates() {
        let result = compile("All happy dogs are friendly.").unwrap();
        // Subject should be: Happy(x) ∧ Dogs(x) → Friendly(x)
        // Check that adjective creates separate predicate in conjunction
        assert!(
            result.contains("Happy(x)") && result.contains("∧") && result.contains("Dogs(x)"),
            "Adjectives should create separate predicates: got '{}'",
            result
        );
    }

    #[test]
    fn relative_clause_basic() {
        let result = compile("All dogs that bark are loud.").unwrap();
        // Subject should be: Dogs(x) ∧ Bark(x) → Loud(x)
        assert!(
            result.contains("Dogs(x)") && result.contains("∧") && result.contains("Bark(x)"),
            "Relative clause should create conjunction: got '{}'",
            result
        );
    }

    #[test]
    fn relative_clause_with_object() {
        let result = compile("All cats that chase mice are hunters.").unwrap();
        // Subject should be: Cats(x) ∧ Chase(x, Mice) → Hunters(x)
        assert!(
            result.contains("∧") && (result.contains("(x, Mice)") || result.contains("(x,Mice)")),
            "Relative clause should include predicate with object: got '{}'",
            result
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Phase 1: Discourse Context & Pronoun Resolution (TDD RED)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn discourse_basic_pronoun_he() {
        use crate::context::DiscourseContext;
        let mut ctx = DiscourseContext::new();
        let _r1 = compile_with_context("John ran.", &mut ctx).unwrap();
        let r2 = compile_with_context("He stopped.", &mut ctx).unwrap();
        assert!(
            r2.contains("J"),
            "He should resolve to John (J): got '{}'",
            r2
        );
    }

    #[test]
    fn discourse_basic_pronoun_she() {
        use crate::context::DiscourseContext;
        let mut ctx = DiscourseContext::new();
        let _r1 = compile_with_context("Mary ran.", &mut ctx).unwrap();
        let r2 = compile_with_context("She stopped.", &mut ctx).unwrap();
        assert!(
            r2.contains("M"),
            "She should resolve to Mary (M): got '{}'",
            r2
        );
    }

    #[test]
    fn discourse_multiple_entities() {
        use crate::context::DiscourseContext;
        let mut ctx = DiscourseContext::new();
        compile_with_context("John saw Mary.", &mut ctx).unwrap();
        let result = compile_with_context("He loves her.", &mut ctx).unwrap();
        assert!(
            (result.contains("Agent(e, J)") && result.contains("Theme(e, M)"))
                || result.contains("(J, M)")
                || result.contains("(J,M)"),
            "He->John, her->Mary: got '{}'",
            result
        );
    }

    #[test]
    fn discourse_definite_reference() {
        use crate::context::DiscourseContext;
        let mut ctx = DiscourseContext::new();
        compile_with_context("A dog barked.", &mut ctx).unwrap();
        let result = compile_with_context("The dog ran.", &mut ctx).unwrap();
        assert!(
            !result.contains("D2"),
            "The dog should refer to same entity, not D2: got '{}'",
            result
        );
    }

    #[test]
    fn discourse_plural_pronoun_they() {
        use crate::context::DiscourseContext;
        let mut ctx = DiscourseContext::new();
        compile_with_context("The dogs ran.", &mut ctx).unwrap();
        let result = compile_with_context("They barked.", &mut ctx).unwrap();
        assert!(
            result.contains("D"),
            "They should resolve to dogs: got '{}'",
            result
        );
    }

    #[test]
    fn discourse_object_pronoun_him() {
        use crate::context::DiscourseContext;
        let mut ctx = DiscourseContext::new();
        compile_with_context("John entered.", &mut ctx).unwrap();
        let result = compile_with_context("Mary saw him.", &mut ctx).unwrap();
        assert!(
            (result.contains("Agent(e, M)") && result.contains("Theme(e, J)"))
                || result.contains("(M, J)"),
            "him should resolve to John: got '{}'",
            result
        );
    }

    #[test]
    fn compile_discourse_batch() {
        let result = compile_discourse(&["John ran.", "He stopped."]).unwrap();
        assert!(
            result.contains("J") && result.contains("∧"),
            "Batch compile should conjoin and resolve: got '{}'",
            result
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Phase 2: Recursive Relative Clauses (TDD RED)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn relative_clause_object_gap() {
        // "The cat that the dog chased ran."
        // The cat is the OBJECT of "chased" (gap), not the subject
        let result = compile("The cat that the dog chased ran.").unwrap();
        // Structure: ∃x(Cat(x) ∧ Chase(Dog, x) ∧ Ran(x))
        assert!(
            result.contains("Theme(e, x)") && result.contains("Cat(x)"),
            "Object-gap relative: dog chases cat, cat ran: got '{}'",
            result
        );
    }

    #[test]
    fn relative_clause_who_subject() {
        // "The man who loves Mary left."
        // "who" = subject of "loves"
        let result = compile("The man who loves Mary left.").unwrap();
        // Structure: ∃x(Man(x) ∧ Love(x, Mary) ∧ Left(x))
        assert!(
            result.contains("(x, Mary)") && result.contains("Man(x)"),
            "Who-clause should bind subject: got '{}'",
            result
        );
    }

    #[test]
    fn relative_clause_who_object() {
        // "The man who Mary loves left."
        // "who" = object of "loves"
        let result = compile("The man who Mary loves left.").unwrap();
        // Structure: ∃x(Man(x) ∧ Love(e) ∧ Agent(e, M) ∧ Theme(e, x) ∧ Left(x))
        // Uses neo-event semantics with Agent/Theme roles
        assert!(
            (result.contains("Agent(e, M") || result.contains("(Mary"))
                && result.contains("Theme(e, x)")
                && result.contains("Man(x)"),
            "Who as object: Mary loves the man: got '{}'",
            result
        );
    }

    #[test]
    fn nested_relative_clause() {
        // "The rat that the cat that the dog chased ate died."
        // dog chased cat, cat ate rat, rat died
        let result = compile("The rat that the cat that the dog chased ate died.").unwrap();
        assert!(
            result.contains("D(") || result.contains("Die(") || result.contains("Died("),
            "Nested relatives should parse: got '{}'",
            result
        );
    }

    #[test]
    fn relative_clause_with_transitive() {
        // "The book that John read is good."
        let result = compile("The book that John read is good.").unwrap();
        assert!(
            result.contains("Agent(e, J)") && result.contains("Theme(e, x)"),
            "Book is object of read: got '{}'",
            result
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Phase 3: Generalized Quantifiers (TDD RED)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn quantifier_most() {
        let result = compile("Most dogs bark.").unwrap();
        assert!(
            result.contains("MOST") && result.contains("Dogs(x)") && result.contains("Bark(x)"),
            "Most should produce MOST x(Dogs(x), Bark(x)): got '{}'",
            result
        );
    }

    #[test]
    fn quantifier_few() {
        let result = compile("Few cats swim.").unwrap();
        assert!(
            result.contains("FEW") && result.contains("Cats(x)") && result.contains("Swim(x)"),
            "Few should produce FEW x(Cats(x), Swim(x)): got '{}'",
            result
        );
    }

    #[test]
    fn quantifier_cardinal_three() {
        let result = compile("Three dogs bark.").unwrap();
        assert!(
            result.contains("∃≥3") || result.contains("∃=3"),
            "Three should produce cardinal quantifier: got '{}'",
            result
        );
    }

    #[test]
    fn quantifier_at_least_two() {
        let result = compile("At least two birds fly.").unwrap();
        assert!(
            result.contains("∃≥2"),
            "At least two should produce ∃≥2: got '{}'",
            result
        );
    }

    #[test]
    fn quantifier_at_most_five() {
        let result = compile("At most five cats sleep.").unwrap();
        assert!(
            result.contains("∃≤5"),
            "At most five should produce ∃≤5: got '{}'",
            result
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Phase 4: Wh-Questions (TDD RED)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn wh_question_who_subject() {
        let result = compile("Who loves Mary?").unwrap();
        assert!(
            result.contains("λx") && result.contains("Love(x, Mary)"),
            "Who-subject should produce λx.Love(x, Mary): got '{}'",
            result
        );
    }

    #[test]
    fn wh_question_what_object() {
        let result = compile("What does John love?").unwrap();
        assert!(
            result.contains("λx") && result.contains("Love(John, x)"),
            "What-object should produce λx.Love(John, x): got '{}'",
            result
        );
    }

    #[test]
    fn yes_no_question() {
        let result = compile("Does John love Mary?").unwrap();
        assert!(
            result.contains("?") || result.contains("L(J, M)"),
            "Yes/no question should produce query: got '{}'",
            result
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Phase 5: Passive Voice (TDD RED)
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn passive_with_agent() {
        let result = compile("Mary was loved by John.").unwrap();
        assert!(
            result.contains("Love(John, Mary)"),
            "Passive 'Mary was loved by John' should produce Love(John, Mary): got '{}'",
            result
        );
    }

    #[test]
    fn passive_without_agent() {
        let result = compile("The book was read.").unwrap();
        assert!(
            result.contains("∃") && result.contains("Read("),
            "Agentless passive should produce ∃x.Read(x, Book): got '{}'",
            result
        );
    }
}
