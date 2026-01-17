//! # Compilation API
//!
//! This module provides the public entry points for natural language to first-order
//! logic translation.
//!
//! ## Compilation Functions
//!
//! | Function | Use Case |
//! |----------|----------|
//! | [`compile`] | Single sentence, Unicode output |
//! | [`compile_simple`] | Single sentence, ASCII output |
//! | [`compile_kripke`] | Modal logic with world quantification |
//! | [`compile_with_discourse`] | Multi-sentence with anaphora resolution |
//! | [`compile_forest`] | Ambiguous sentences, all readings |
//! | [`compile_all_scopes`] | All quantifier scope permutations |
//! | [`compile_discourse`] | Multi-sentence with temporal ordering |
//! | [`compile_theorem`] | Theorem proving with backward chaining |
//!
//! ## Example
//!
//! ```rust
//! use logicaffeine_language::{compile, compile_forest};
//!
//! // Simple compilation
//! let fol = compile("John loves Mary.").unwrap();
//! assert!(fol.contains("Love"));
//!
//! // Handle ambiguity
//! let readings = compile_forest("Every woman loves a man.");
//! assert!(readings.len() >= 1); // Surface and possibly inverse scope
//! ```

use crate::{
    analysis, Arena, CompileOptions, drs, Interner, lambda, lexicon, Lexer, mwe,
    OutputFormat, Parser, pragmatics, semantics, SymbolRegistry, ParseError, token,
    arena_ctx::AstContext,
    parser::{NegativeScopeMode, ModalPreference, QuantifierParsing},
};

/// Maximum number of readings in a parse forest.
/// Prevents exponential blowup from ambiguous sentences.
pub const MAX_FOREST_READINGS: usize = 12;

/// Compile natural language input to first-order logic with default options.
pub fn compile(input: &str) -> Result<String, ParseError> {
    compile_with_options(input, CompileOptions::default())
}

/// Compile with simple FOL format.
pub fn compile_simple(input: &str) -> Result<String, ParseError> {
    compile_with_options(input, CompileOptions {
        format: OutputFormat::SimpleFOL,
    })
}

/// Compile with Kripke semantics lowering.
/// Modal operators are transformed into explicit possible world quantification.
pub fn compile_kripke(input: &str) -> Result<String, ParseError> {
    compile_with_options(input, CompileOptions {
        format: OutputFormat::Kripke,
    })
}

/// Compile natural language input to first-order logic with specified options.
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
    let mut world_state = drs::WorldState::new();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
    let ast = parser.parse()?;
    let ast = semantics::apply_axioms(ast, ctx.exprs, ctx.terms, &mut interner);

    // Apply Kripke lowering for Kripke format (before pragmatics to preserve modal structure)
    let ast = if options.format == OutputFormat::Kripke {
        semantics::apply_kripke_lowering(ast, ctx.exprs, ctx.terms, &mut interner)
    } else {
        ast
    };

    let ast = pragmatics::apply_pragmatics(ast, ctx.exprs, &interner);
    let mut registry = SymbolRegistry::new();
    // Use transpile_discourse to format multiple sentences as numbered formulas
    let main_output = ast.transpile_discourse(&mut registry, &interner, options.format);

    // Append Reichenbach temporal constraints
    let constraints = world_state.time_constraints();
    if constraints.is_empty() {
        Ok(main_output)
    } else {
        let constraint_strs: Vec<String> = constraints.iter().map(|c| {
            match c.relation {
                drs::TimeRelation::Precedes => format!("Precedes({}, {})", c.left, c.right),
                drs::TimeRelation::Equals => format!("{}={}", c.left, c.right),
            }
        }).collect();
        Ok(format!("{} ∧ {}", main_output, constraint_strs.join(" ∧ ")))
    }
}

/// Compile with shared WorldState for cross-sentence discourse.
pub fn compile_with_world_state(input: &str, world_state: &mut drs::WorldState) -> Result<String, ParseError> {
    compile_with_world_state_options(input, world_state, CompileOptions::default())
}

/// Compile with shared WorldState and options.
pub fn compile_with_world_state_options(
    input: &str,
    world_state: &mut drs::WorldState,
    options: CompileOptions,
) -> Result<String, ParseError> {
    let mut interner = Interner::new();
    compile_with_world_state_interner_options(input, world_state, &mut interner, options)
}

/// Compile with shared WorldState AND Interner for proper cross-sentence discourse.
/// Use this when you need pronouns to resolve across multiple sentences.
pub fn compile_with_discourse(
    input: &str,
    world_state: &mut drs::WorldState,
    interner: &mut Interner,
) -> Result<String, ParseError> {
    compile_with_world_state_interner_options(input, world_state, interner, CompileOptions::default())
}

/// Compile with full control over WorldState, Interner, and options.
pub fn compile_with_world_state_interner_options(
    input: &str,
    world_state: &mut drs::WorldState,
    interner: &mut Interner,
    options: CompileOptions,
) -> Result<String, ParseError> {
    let mut lexer = Lexer::new(input, interner);
    let tokens = lexer.tokenize();

    // Apply MWE collapsing
    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, interner);

    // Pass 1: Discovery
    let type_registry = {
        let mut discovery = analysis::DiscoveryPass::new(&tokens, interner);
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

    let mut parser = Parser::new(tokens, world_state, interner, ctx, type_registry);
    // Swap DRS from WorldState into Parser at start
    parser.swap_drs_with_world_state();
    let ast = parser.parse()?;
    // Swap DRS back to WorldState at end
    parser.swap_drs_with_world_state();
    let ast = semantics::apply_axioms(ast, ctx.exprs, ctx.terms, interner);

    // Mark sentence boundary for telescoping support
    world_state.end_sentence();

    let ast = if options.format == OutputFormat::Kripke {
        semantics::apply_kripke_lowering(ast, ctx.exprs, ctx.terms, interner)
    } else {
        ast
    };

    let ast = pragmatics::apply_pragmatics(ast, ctx.exprs, interner);
    let mut registry = SymbolRegistry::new();
    let main_output = ast.transpile_discourse(&mut registry, interner, options.format);

    let constraints = world_state.time_constraints();
    if constraints.is_empty() {
        Ok(main_output)
    } else {
        let constraint_strs: Vec<String> = constraints.iter().map(|c| {
            match c.relation {
                drs::TimeRelation::Precedes => format!("Precedes({}, {})", c.left, c.right),
                drs::TimeRelation::Equals => format!("{}={}", c.left, c.right),
            }
        }).collect();
        Ok(format!("{} ∧ {}", main_output, constraint_strs.join(" ∧ ")))
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

/// Returns all scope readings with specified output format.
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
    let mut world_state = drs::WorldState::new();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
    let ast = parser.parse()?;

    let scope_arena = Arena::new();
    let scope_term_arena = Arena::new();
    let scopings = lambda::enumerate_scopings(ast, &mut interner, &scope_arena, &scope_term_arena);

    let intensional_arena = Arena::new();
    let intensional_term_arena = Arena::new();
    let intensional_role_arena: Arena<(crate::ast::ThematicRole, crate::ast::Term)> = Arena::new();

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
            let reading = semantics::apply_axioms(reading, &intensional_arena, &intensional_term_arena, &mut interner);
            let mut registry = SymbolRegistry::new();
            results.push(reading.transpile(&mut registry, &interner, options.format));
        }
    }

    Ok(results)
}

// ═══════════════════════════════════════════════════════════════════
// Parse Forest Compilation (Ambiguity Resolution)
// ═══════════════════════════════════════════════════════════════════

/// Compile natural language input, producing all valid parse readings.
/// Handles lexical ambiguity (Noun/Verb) and structural ambiguity (PP attachment).
pub fn compile_forest(input: &str) -> Vec<String> {
    compile_forest_with_options(input, CompileOptions::default())
}

/// Compile natural language input with options, producing all valid parse readings.
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

    // Detect plurality ambiguity (mixed verb + plural subject)
    let has_mixed_verb = tokens.iter().any(|t| {
        if let token::TokenType::Verb { lemma, .. } = &t.kind {
            Lexer::is_mixed_verb(interner.resolve(*lemma))
        } else {
            false
        }
    });

    // Detect collective verbs (always require group reading with cardinals)
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

    // Detect event adjective + agentive noun ambiguity
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

    // Detect lexically negative verbs (e.g., "lacks", "miss") for scope ambiguity
    let has_negative_verb = tokens.iter().any(|t| {
        if let token::TokenType::Verb { lemma, .. } = &t.kind {
            lexicon::get_canonical_verb(&interner.resolve(*lemma).to_lowercase())
                .map(|(_, is_neg)| is_neg)
                .unwrap_or(false)
        } else {
            false
        }
    });

    // Detect modal polysemy (may, can, could)
    let has_may = tokens.iter().any(|t| matches!(t.kind, token::TokenType::May));
    let has_can = tokens.iter().any(|t| matches!(t.kind, token::TokenType::Can));
    let has_could = tokens.iter().any(|t| matches!(t.kind, token::TokenType::Could));

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

        let mut world_state = drs::WorldState::new();
        let mut parser = Parser::new(tokens.clone(), &mut world_state, &mut interner, ast_ctx, type_registry.clone());
        parser.set_noun_priority_mode(false);

        if let Ok(ast) = parser.parse() {
            let ast = semantics::apply_axioms(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner);
            let ast = if options.format == OutputFormat::Kripke {
                semantics::apply_kripke_lowering(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner)
            } else {
                ast
            };
            let mut registry = SymbolRegistry::new();
            results.push(ast.transpile_discourse(&mut registry, &interner, options.format));
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

        let mut world_state = drs::WorldState::new();
        let mut parser = Parser::new(tokens.clone(), &mut world_state, &mut interner, ast_ctx, type_registry.clone());
        parser.set_noun_priority_mode(true);

        if let Ok(ast) = parser.parse() {
            let ast = semantics::apply_axioms(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner);
            let ast = if options.format == OutputFormat::Kripke {
                semantics::apply_kripke_lowering(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner)
            } else {
                ast
            };
            let mut registry = SymbolRegistry::new();
            let reading = ast.transpile_discourse(&mut registry, &interner, options.format);
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

        let mut world_state = drs::WorldState::new();
        let mut parser = Parser::new(tokens.clone(), &mut world_state, &mut interner, ast_ctx, type_registry.clone());
        parser.set_pp_attachment_mode(true);

        if let Ok(ast) = parser.parse() {
            let ast = semantics::apply_axioms(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner);
            let ast = if options.format == OutputFormat::Kripke {
                semantics::apply_kripke_lowering(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner)
            } else {
                ast
            };
            let mut registry = SymbolRegistry::new();
            let reading = ast.transpile_discourse(&mut registry, &interner, options.format);
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

        let mut world_state = drs::WorldState::new();
        let mut parser = Parser::new(tokens.clone(), &mut world_state, &mut interner, ast_ctx, type_registry.clone());
        parser.set_collective_mode(true);

        if let Ok(ast) = parser.parse() {
            if let Ok(transformed) = parser.transform_cardinal_to_group(ast) {
                let transformed = semantics::apply_axioms(transformed, ast_ctx.exprs, ast_ctx.terms, &mut interner);
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

        let mut world_state = drs::WorldState::new();
        let mut parser = Parser::new(tokens.clone(), &mut world_state, &mut interner, ast_ctx, type_registry.clone());
        parser.set_event_reading_mode(true);

        if let Ok(ast) = parser.parse() {
            let ast = semantics::apply_axioms(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner);
            let ast = if options.format == OutputFormat::Kripke {
                semantics::apply_kripke_lowering(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner)
            } else {
                ast
            };
            let mut registry = SymbolRegistry::new();
            let reading = ast.transpile_discourse(&mut registry, &interner, options.format);
            if !results.contains(&reading) {
                results.push(reading);
            }
        }
    }

    // Reading 6: Wide scope negation mode (for lexically negative verbs like "lacks")
    if has_negative_verb {
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

        let mut world_state = drs::WorldState::new();
        let mut parser = Parser::new(tokens.clone(), &mut world_state, &mut interner, ast_ctx, type_registry.clone());
        parser.set_negative_scope_mode(NegativeScopeMode::Wide);

        if let Ok(ast) = parser.parse() {
            let ast = semantics::apply_axioms(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner);
            let ast = if options.format == OutputFormat::Kripke {
                semantics::apply_kripke_lowering(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner)
            } else {
                ast
            };
            let mut registry = SymbolRegistry::new();
            let reading = ast.transpile_discourse(&mut registry, &interner, options.format);
            if !results.contains(&reading) {
                results.push(reading);
            }
        }
    }

    // Reading 7: Epistemic modal preference (May=Possibility, Could=Possibility)
    if has_may || has_could {
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

        let mut world_state = drs::WorldState::new();
        let mut parser = Parser::new(tokens.clone(), &mut world_state, &mut interner, ast_ctx, type_registry.clone());
        parser.set_modal_preference(ModalPreference::Epistemic);

        if let Ok(ast) = parser.parse() {
            let ast = semantics::apply_axioms(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner);
            let ast = if options.format == OutputFormat::Kripke {
                semantics::apply_kripke_lowering(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner)
            } else {
                ast
            };
            let mut registry = SymbolRegistry::new();
            let reading = ast.transpile_discourse(&mut registry, &interner, options.format);
            if !results.contains(&reading) {
                results.push(reading);
            }
        }
    }

    // Reading 8: Deontic modal preference (Can=Permission)
    if has_can {
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

        let mut world_state = drs::WorldState::new();
        let mut parser = Parser::new(tokens.clone(), &mut world_state, &mut interner, ast_ctx, type_registry.clone());
        parser.set_modal_preference(ModalPreference::Deontic);

        if let Ok(ast) = parser.parse() {
            let ast = semantics::apply_axioms(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner);
            let ast = if options.format == OutputFormat::Kripke {
                semantics::apply_kripke_lowering(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner)
            } else {
                ast
            };
            let mut registry = SymbolRegistry::new();
            let reading = ast.transpile_discourse(&mut registry, &interner, options.format);
            if !results.contains(&reading) {
                results.push(reading);
            }
        }
    }

    // Reading 9: Wide scope negation + Deontic modal preference
    if has_negative_verb && has_can {
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

        let mut world_state = drs::WorldState::new();
        let mut parser = Parser::new(tokens.clone(), &mut world_state, &mut interner, ast_ctx, type_registry);
        parser.set_negative_scope_mode(NegativeScopeMode::Wide);
        parser.set_modal_preference(ModalPreference::Deontic);

        if let Ok(ast) = parser.parse() {
            let ast = semantics::apply_axioms(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner);
            let ast = if options.format == OutputFormat::Kripke {
                semantics::apply_kripke_lowering(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner)
            } else {
                ast
            };
            let mut registry = SymbolRegistry::new();
            let reading = ast.transpile_discourse(&mut registry, &interner, options.format);
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
// Discourse Compilation
// ═══════════════════════════════════════════════════════════════════

/// Compile multiple sentences as a discourse, tracking temporal ordering.
pub fn compile_discourse(sentences: &[&str]) -> Result<String, ParseError> {
    compile_discourse_with_options(sentences, CompileOptions::default())
}

/// Compile multiple sentences as a discourse with specified options.
pub fn compile_discourse_with_options(sentences: &[&str], options: CompileOptions) -> Result<String, ParseError> {
    let mut interner = Interner::new();
    let mut world_state = drs::WorldState::new();
    let mut results = Vec::new();
    let mut registry = SymbolRegistry::new();
    let mwe_trie = mwe::build_mwe_trie();

    for sentence in sentences {
        let event_var_name = world_state.next_event_var();
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

        // Pass 2: Parse with WorldState (DRS persists across sentences)
        let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
        parser.set_discourse_event_var(event_var_symbol);
        // Swap DRS from WorldState into Parser at start
        parser.swap_drs_with_world_state();
        let ast = parser.parse()?;
        // Swap DRS back to WorldState at end
        parser.swap_drs_with_world_state();

        // Mark sentence boundary - collect telescope candidates for cross-sentence anaphora
        world_state.end_sentence();

        let ast = semantics::apply_axioms(ast, ast_ctx.exprs, ast_ctx.terms, &mut interner);
        results.push(ast.transpile_discourse(&mut registry, &interner, options.format));
    }

    let event_history = world_state.event_history();
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

// ═══════════════════════════════════════════════════════════════════
// Ambiguity Handling
// ═══════════════════════════════════════════════════════════════════

/// Compile with PP attachment ambiguity detection.
/// Returns multiple readings if structural ambiguity exists.
pub fn compile_ambiguous(input: &str) -> Result<Vec<String>, ParseError> {
    compile_ambiguous_with_options(input, CompileOptions::default())
}

/// Compile with PP attachment ambiguity detection and specified options.
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
    let mut world_state = drs::WorldState::new();
    let mut parser = Parser::new(tokens.clone(), &mut world_state, &mut interner, ctx, type_registry.clone());
    let ast = parser.parse()?;
    let ast = semantics::apply_axioms(ast, ctx.exprs, ctx.terms, &mut interner);
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

        let mut world_state2 = drs::WorldState::new();
        let mut parser2 = Parser::new(tokens, &mut world_state2, &mut interner, ctx2, type_registry);
        parser2.set_pp_attachment_mode(true);
        let ast2 = parser2.parse()?;
        let ast2 = semantics::apply_axioms(ast2, ctx2.exprs, ctx2.terms, &mut interner);
        let mut registry2 = SymbolRegistry::new();
        let reading2 = ast2.transpile(&mut registry2, &interner, options.format);

        if reading1 != reading2 {
            return Ok(vec![reading1, reading2]);
        }
    }

    Ok(vec![reading1])
}

// ═══════════════════════════════════════════════════════════════════
// Theorem Compilation
// ═══════════════════════════════════════════════════════════════════

use crate::ast::{self, Stmt};
use crate::token::Span;
use crate::error::ParseErrorKind;
use logicaffeine_proof::BackwardChainer;
use crate::proof_convert::logic_expr_to_proof_expr;

/// Compile and prove a theorem block.
pub fn compile_theorem(input: &str) -> Result<String, ParseError> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    // Apply MWE collapsing
    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    // Pass 1: Discovery
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

    // Parse as program to get statements including Theorem blocks
    let mut world_state = drs::WorldState::new();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
    let statements = parser.parse_program()?;

    // Find the first Theorem statement
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
            kind: ParseErrorKind::Custom("No theorem block found in input".to_string()),
            span: Span::default(),
        })?;

    // Convert premises from LogicExpr to ProofExpr
    let mut engine = BackwardChainer::new();
    for premise in &theorem.premises {
        let proof_expr = logic_expr_to_proof_expr(premise, &interner);
        engine.add_axiom(proof_expr);
    }

    // Convert goal from LogicExpr to ProofExpr
    let goal = logic_expr_to_proof_expr(theorem.goal, &interner);

    // Attempt to prove the goal
    match engine.prove(goal.clone()) {
        Ok(derivation) => {
            Ok(format!(
                "Theorem '{}' Proved!\n{}",
                theorem.name,
                derivation.display_tree()
            ))
        }
        Err(e) => {
            // Return error with context about what was attempted
            Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "Theorem '{}' failed.\n  Goal: {}\n  Premises: {}\n  Error: {}",
                    theorem.name,
                    goal,
                    theorem.premises.len(),
                    e
                )),
                span: Span::default(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_simple_sentence() {
        let result = compile("John runs.");
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("Run"));
        assert!(output.contains("John"));
    }

    #[test]
    fn test_compile_with_unicode_format() {
        let options = CompileOptions { format: OutputFormat::Unicode };
        let result = compile_with_options("Every dog barks.", options);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("∀") || output.contains("Forall"));
    }

    #[test]
    fn test_compile_all_scopes() {
        let result = compile_all_scopes("Every woman loves a man.");
        assert!(result.is_ok());
        let readings = result.unwrap();
        assert!(readings.len() >= 1);
    }
}
