#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

// Re-export base types for internal use and consumers
pub use logicaffeine_base::{Arena, Interner, Symbol, SymbolEq, Span as BaseSpan};

// Provide an `intern` module alias for internal code that uses `crate::intern::*`
pub mod intern {
    pub use logicaffeine_base::{Interner, Symbol, SymbolEq};
}

// Provide an `arena` module alias for internal code that uses `crate::arena::*`
pub mod arena {
    pub use logicaffeine_base::Arena;
}

// Core modules
pub mod token;
pub mod token_class;
pub mod teach;
pub mod lexer;
pub mod lexicon;
pub mod drs;
pub mod error;

// Parser and AST
pub mod parser;
pub mod ast;

// Semantic analysis
pub mod semantics;
pub mod lambda;
pub mod transpile;

// Compile API
pub mod compile;

// Support modules
pub mod analysis;
pub mod arena_ctx;
pub mod ast_depth;
pub mod formatter;
pub mod mwe;
pub mod ontology;
pub mod pragmatics;
pub mod registry;
pub mod scope;
pub mod session;
pub mod source_format;
pub mod suggest;
pub mod symbol_dict;
pub mod view;
pub mod visitor;
pub mod debug;
pub mod style;

// The single source of truth for compiler optimization toggles.
pub mod optimization;

// Proof conversion: bridges language AST to proof engine
pub mod proof_convert;
pub use proof_convert::{logic_expr_to_proof_expr, term_to_proof_term};

// Re-export key types at crate root
pub use token::{BlockType, FocusKind, MeasureKind, PresupKind, Span, Token, TokenType};
pub use lexer::{Lexer, LineLexer, LineToken};
pub use parser::{Parser, ParserMode, NegativeScopeMode, QuantifierParsing};
pub use error::{ParseError, ParseErrorKind, socratic_explanation};
pub use drs::{Drs, BoxType, WorldState, Gender, Number, Case};
pub use analysis::TypeRegistry;
pub use registry::SymbolRegistry;
pub use arena_ctx::AstContext;
pub use session::Session;

// Compile API re-exports
pub use compile::{
    compile, compile_pragmatic, compile_simple, compile_kripke, compile_kripke_with, compile_with_options,
    compile_with_world_state, compile_with_world_state_options,
    compile_with_discourse, compile_with_world_state_interner_options,
    compile_all_scopes, compile_all_scopes_with_options,
    compile_forest, compile_forest_with_options, MAX_FOREST_READINGS,
    compile_discourse, compile_discourse_with_options,
    compile_ambiguous, compile_ambiguous_with_options,
    compile_theorem,
};

// Runtime lexicon re-export (when dynamic-lexicon feature is enabled)
#[cfg(feature = "dynamic-lexicon")]
pub use logicaffeine_lexicon::runtime as runtime_lexicon;

// Output format configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Unicode,
    LaTeX,
    SimpleFOL,
    /// Kripke semantics output: modals lowered to explicit world quantification.
    Kripke,
}

// Transpile context
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
    /// Enrich the parse with conversational (scalar) implicature (§8.7): a weak
    /// scalar "some" is strengthened to `∃… +> ¬∀…`. Off by default so the literal
    /// truth-conditional output is unchanged.
    pub pragmatic: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        CompileOptions {
            format: OutputFormat::Unicode,
            pragmatic: false,
        }
    }
}
