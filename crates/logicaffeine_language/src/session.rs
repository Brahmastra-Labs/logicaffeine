//! Session Manager for Incremental Evaluation
//!
//! Provides a REPL-style interface for parsing sentences one at a time
//! while maintaining persistent discourse state across turns.
//!
//! # Example
//!
//! ```
//! use logicaffeine_language::Session;
//!
//! let mut session = Session::new();
//! let out1 = session.eval("The boys lifted the piano.").unwrap();
//! let out2 = session.eval("They smiled.").unwrap();  // "They" resolves to "the boys"
//! ```

use crate::analysis;
use logicaffeine_base::Arena;
use crate::arena_ctx::AstContext;
use crate::drs::WorldState;
use crate::error::ParseError;
use logicaffeine_base::Interner;
use crate::lexer::Lexer;
use crate::mwe;
use crate::parser::Parser;
use crate::registry::SymbolRegistry;
use crate::semantics;
use crate::OutputFormat;

/// A persistent session for incremental sentence evaluation.
///
/// Maintains discourse state across multiple `eval()` calls, enabling
/// cross-sentence anaphora resolution and temporal ordering.
pub struct Session {
    /// Persistent discourse state (DRS tree, referents, modal contexts)
    world_state: WorldState,

    /// Symbol interner for string interning across all sentences
    interner: Interner,

    /// Symbol registry for transpilation
    registry: SymbolRegistry,

    /// MWE trie for multi-word expression detection
    mwe_trie: mwe::MweTrie,

    /// Accumulated transpiled outputs from each sentence
    history: Vec<String>,

    /// Output format for transpilation
    format: OutputFormat,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Create a new session with default settings.
    pub fn new() -> Self {
        Session {
            world_state: WorldState::new(),
            interner: Interner::new(),
            registry: SymbolRegistry::new(),
            mwe_trie: mwe::build_mwe_trie(),
            history: Vec::new(),
            format: OutputFormat::Unicode,
        }
    }

    /// Create a new session with a specific output format.
    pub fn with_format(format: OutputFormat) -> Self {
        Session {
            world_state: WorldState::new(),
            interner: Interner::new(),
            registry: SymbolRegistry::new(),
            mwe_trie: mwe::build_mwe_trie(),
            history: Vec::new(),
            format,
        }
    }

    /// Evaluate a single sentence, updating the session state.
    ///
    /// Returns the transpiled logic for just this sentence.
    /// Pronouns in this sentence can resolve to entities from previous sentences.
    pub fn eval(&mut self, input: &str) -> Result<String, ParseError> {
        // Generate event variable for this turn
        let event_var_name = self.world_state.next_event_var();
        let event_var_symbol = self.interner.intern(&event_var_name);

        // Tokenize
        let mut lexer = Lexer::new(input, &mut self.interner);
        let tokens = lexer.tokenize();

        // Apply MWE collapsing
        let tokens = mwe::apply_mwe_pipeline(tokens, &self.mwe_trie, &mut self.interner);

        // Pass 1: Discovery - scan for type definitions
        let type_registry = {
            let mut discovery = analysis::DiscoveryPass::new(&tokens, &mut self.interner);
            discovery.run()
        };

        // Create arenas for this parse (fresh each sentence)
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
        let mut parser = Parser::new(
            tokens,
            &mut self.world_state,
            &mut self.interner,
            ast_ctx,
            type_registry,
        );
        parser.set_discourse_event_var(event_var_symbol);

        // Swap DRS from WorldState into Parser at start
        parser.swap_drs_with_world_state();
        let ast = parser.parse()?;
        // Swap DRS back to WorldState at end
        parser.swap_drs_with_world_state();

        // Mark sentence boundary - collect telescope candidates for cross-sentence anaphora
        self.world_state.end_sentence();

        // Apply semantic axioms
        let ast = semantics::apply_axioms(ast, ast_ctx.exprs, ast_ctx.terms, &mut self.interner);

        // Transpile
        let output = ast.transpile(&mut self.registry, &self.interner, self.format);

        // Store in history
        self.history.push(output.clone());

        Ok(output)
    }

    /// Get the full accumulated logic from all sentences.
    ///
    /// Includes temporal ordering constraints (Precedes relations).
    pub fn history(&self) -> String {
        if self.history.is_empty() {
            return String::new();
        }

        let event_history = self.world_state.event_history();
        let mut precedes = Vec::new();
        for i in 0..event_history.len().saturating_sub(1) {
            precedes.push(format!("Precedes({}, {})", event_history[i], event_history[i + 1]));
        }

        if precedes.is_empty() {
            self.history.join(" ∧ ")
        } else {
            format!("{} ∧ {}", self.history.join(" ∧ "), precedes.join(" ∧ "))
        }
    }

    /// Get the number of sentences processed.
    pub fn turn_count(&self) -> usize {
        self.history.len()
    }

    /// Get direct access to the world state (for advanced use).
    pub fn world_state(&self) -> &WorldState {
        &self.world_state
    }

    /// Get mutable access to the world state (for advanced use).
    pub fn world_state_mut(&mut self) -> &mut WorldState {
        &mut self.world_state
    }

    /// Reset the session to initial state.
    pub fn reset(&mut self) {
        self.world_state = WorldState::new();
        self.history.clear();
        // Keep interner and registry - symbols are still valid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_basic() {
        let mut session = Session::new();
        let out = session.eval("John walked.").unwrap();
        assert!(out.contains("Walk"), "Should have Walk predicate");
    }

    #[test]
    fn test_session_multiple_sentences() {
        let mut session = Session::new();
        session.eval("John walked.").unwrap();
        session.eval("Mary ran.").unwrap();

        assert_eq!(session.turn_count(), 2);

        let history = session.history();
        assert!(history.contains("Walk"));
        assert!(history.contains("Run"));
        assert!(history.contains("Precedes"));
    }
}
