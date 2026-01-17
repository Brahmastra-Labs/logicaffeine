//! Token types for the LOGOS lexer and parser.
//!
//! This module defines the vocabulary of the LOGOS language at the token level.
//! Tokens represent the atomic syntactic units produced by the lexer and consumed
//! by the parser.
//!
//! ## Token Categories
//!
//! | Category | Examples | Description |
//! |----------|----------|-------------|
//! | **Quantifiers** | every, some, no | Bind variables over domains |
//! | **Determiners** | the, a, this | Select referents |
//! | **Nouns** | cat, philosopher | Predicates over individuals |
//! | **Verbs** | runs, loves | Relations between arguments |
//! | **Adjectives** | red, happy | Modify noun denotations |
//! | **Connectives** | and, or, implies | Combine propositions |
//! | **Pronouns** | he, she, it | Resolve to antecedents |
//!
//! ## Block Types
//!
//! LOGOS uses markdown-style block headers for structured documents:
//!
//! - `## Theorem`: Declares a proposition to be proved
//! - `## Proof`: Contains the proof steps
//! - `## Definition`: Introduces new terminology
//! - `## Main`: Program entry point

use logicaffeine_base::Symbol;
use logicaffeine_lexicon::{Aspect, Case, Definiteness, Gender, Number, Time, VerbClass};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresupKind {
    Stop,
    Start,
    Regret,
    Continue,
    Realize,
    Know,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusKind {
    Only,
    Even,
    Just,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureKind {
    Much,
    Little,
}

/// Document structure block type markers.
///
/// LOGOS uses markdown-style `## Header` syntax to delimit different
/// sections of a program or proof document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    /// `## Theorem` - Declares a proposition to be proved.
    Theorem,
    /// `## Main` - Program entry point for imperative code.
    Main,
    /// `## Definition` - Introduces new terminology or type definitions.
    Definition,
    /// `## Proof` - Contains proof steps for a theorem.
    Proof,
    /// `## Example` - Illustrative examples.
    Example,
    /// `## Logic` - Direct logical notation input.
    Logic,
    /// `## Note` - Explanatory documentation.
    Note,
    /// `## To` - Function definition block.
    Function,
    /// Inline type definition: `## A Point has:` or `## A Color is one of:`.
    TypeDef,
    /// `## Policy` - Security policy rule definitions.
    Policy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenType {
    // Document Structure
    BlockHeader { block_type: BlockType },

    // Quantifiers
    All,
    No,
    Some,
    Any,
    Both, // Correlative conjunction marker: "both X and Y"
    Most,
    Few,
    Many,
    Cardinal(u32),
    AtLeast(u32),
    AtMost(u32),

    // Negative Polarity Items (NPIs)
    Anything,
    Anyone,
    Nothing,
    Nobody,
    NoOne,
    Nowhere,
    Ever,
    Never,

    // Logical Connectives
    And,
    Or,
    If,
    Then,
    Not,
    Iff,
    Because,

    // Modal Operators
    Must,
    Shall,
    Should,
    Can,
    May,
    Cannot,
    Would,
    Could,
    Might,
    Had,

    // Imperative Statement Keywords
    Let,
    Set,
    Return,
    Be,
    While,
    Repeat,
    For,
    In,
    From,
    Assert,
    /// Documented assertion with justification string.
    Trust,
    Otherwise,
    Call,
    /// Constructor keyword for struct instantiation.
    New,
    /// Sum type definition keyword.
    Either,
    /// Pattern matching statement keyword.
    Inspect,
    /// Native function modifier for FFI bindings.
    Native,

    // Theorem Keywords
    /// Premise marker in theorem blocks.
    Given,
    /// Goal marker in theorem blocks.
    Prove,
    /// Automatic proof strategy directive.
    Auto,

    // IO Keywords
    /// "Read input from..."
    Read,
    /// "Write x to file..."
    Write,
    /// "...from the console"
    Console,
    /// "...from file..." or "...to file..."
    File,

    // Ownership Keywords (Move/Borrow Semantics)
    /// Move ownership: "Give x to processor"
    Give,
    /// Immutable borrow: "Show x to console"
    Show,

    // Collection Operations
    /// "Push x to items"
    Push,
    /// "Pop from items"
    Pop,
    /// "copy of slice" → slice.to_vec()
    Copy,
    /// "items 1 through 3" → inclusive slice
    Through,
    /// "length of items" → items.len()
    Length,
    /// "items at i" → `items[i]`
    At,

    // Set Operations
    /// "Add x to set" (insert)
    Add,
    /// "Remove x from set"
    Remove,
    /// "set contains x"
    Contains,
    /// "a union b"
    Union,
    /// "a intersection b"
    Intersection,

    // Memory Management (Zones)
    /// "Inside a new zone..."
    Inside,
    /// "...zone called..."
    Zone,
    /// "...called 'Scratch'"
    Called,
    /// "...of size 1 MB"
    Size,
    /// "...mapped from 'file.bin'"
    Mapped,

    // Structured Concurrency
    /// "Attempt all of the following:" → concurrent (async, I/O-bound)
    Attempt,
    /// "the following"
    Following,
    /// "Simultaneously:" → parallel (CPU-bound)
    Simultaneously,

    // Agent System (Actor Model)
    /// "Spawn a Worker called 'w1'" → create agent
    Spawn,
    /// "Send Ping to 'agent'" → send message to agent
    Send,
    /// "Await response from 'agent' into result" → receive message
    Await,

    // Serialization
    /// "A Message is Portable and has:" → serde derives
    Portable,

    // Sipping Protocol
    /// "the manifest of Zone" → FileSipper manifest
    Manifest,
    /// "the chunk at N in Zone" → FileSipper chunk
    Chunk,

    // CRDT Keywords
    /// "A Counter is Shared and has:" → CRDT struct
    Shared,
    /// "Merge remote into local" → CRDT merge
    Merge,
    /// "Increase x's count by 10" → GCounter increment
    Increase,

    // Extended CRDT Keywords
    /// "Decrease x's count by 5" → PNCounter decrement
    Decrease,
    /// "which is a Tally" → PNCounter type
    Tally,
    /// "which is a SharedSet of T" → ORSet type
    SharedSet,
    /// "which is a SharedSequence of T" → RGA type
    SharedSequence,
    /// "which is a CollaborativeSequence of T" → YATA type
    CollaborativeSequence,
    /// "which is a SharedMap from K to V" → ORMap type
    SharedMap,
    /// "which is a Divergent T" → MVRegister type
    Divergent,
    /// "Append x to seq" → RGA append
    Append,
    /// "Resolve x to value" → MVRegister resolve
    Resolve,
    /// "(RemoveWins)" → ORSet bias
    RemoveWins,
    /// "(AddWins)" → ORSet bias (default)
    AddWins,
    /// "(YATA)" → Sequence algorithm
    YATA,
    /// "x's values" → MVRegister values accessor
    Values,

    // Security Keywords
    /// "Check that user is admin" → mandatory runtime guard
    Check,

    // P2P Networking Keywords
    /// "Listen on \[addr\]" → bind to network address
    Listen,
    /// "Connect to \[addr\]" → dial a peer (NetConnect to avoid conflict)
    NetConnect,
    /// "Sleep N." → pause execution for N milliseconds
    Sleep,

    // GossipSub Keywords
    /// "Sync x on 'topic'" → automatic CRDT replication
    Sync,

    // Persistence Keywords
    /// "Mount x at \[path\]" → load/create persistent CRDT from journal
    Mount,
    /// "Persistent Counter" → type wrapped with journaling
    Persistent,
    /// "x combined with y" → string concatenation
    Combined,

    // Go-like Concurrency Keywords
    /// "Launch a task to..." → spawn green thread
    Launch,
    /// "a task" → identifier for task context
    Task,
    /// "Pipe of Type" → channel creation
    Pipe,
    /// "Receive from pipe" → recv from channel
    Receive,
    /// "Stop handle" → abort task
    Stop,
    /// "Try to send/receive" → non-blocking variant
    Try,
    /// "Send value into pipe" → channel send
    Into,
    /// "Await the first of:" → select statement
    First,
    /// "After N seconds:" → timeout branch
    After,

    // Block Scoping
    Colon,
    Indent,
    Dedent,
    Newline,

    // Content Words
    Noun(Symbol),
    Adjective(Symbol),
    NonIntersectiveAdjective(Symbol),
    Adverb(Symbol),
    ScopalAdverb(Symbol),
    TemporalAdverb(Symbol),
    Verb {
        lemma: Symbol,
        time: Time,
        aspect: Aspect,
        class: VerbClass,
    },
    ProperName(Symbol),

    /// Lexically ambiguous token (e.g., "fish" as noun or verb).
    ///
    /// The parser tries the primary interpretation first, then alternatives
    /// if parsing fails. Used for parse forest generation.
    Ambiguous {
        primary: Box<TokenType>,
        alternatives: Vec<TokenType>,
    },

    // Speech Acts (Performatives)
    Performative(Symbol),
    Exclamation,

    // Articles (Definiteness)
    Article(Definiteness),

    // Temporal Auxiliaries
    Auxiliary(Time),

    // Copula & Functional
    Is,
    Are,
    Was,
    Were,
    That,
    Who,
    What,
    Where,
    When,
    Why,
    Does,
    Do,

    // Identity & Reflexive (FOL)
    Identity,
    Equals,
    Reflexive,
    Reciprocal,
    /// Pairwise list coordination: "A and B respectively love C and D"
    Respectively,

    // Pronouns (Discourse)
    Pronoun {
        gender: Gender,
        number: Number,
        case: Case,
    },

    // Prepositions (for N-ary relations)
    Preposition(Symbol),

    // Phrasal Verb Particles (up, down, out, in, off, on, away)
    Particle(Symbol),

    // Comparatives & Superlatives (Pillar 3 - Degree Semantics)
    Comparative(Symbol),
    Superlative(Symbol),
    Than,

    // Control Verbs (Chomsky's Control Theory)
    To,

    // Presupposition Triggers (Austin/Strawson)
    PresupTrigger(PresupKind),

    // Focus Particles (Rooth)
    Focus(FocusKind),

    // Mass Noun Measure
    Measure(MeasureKind),

    // Numeric Literals (prover-ready: stores raw string for symbolic math)
    Number(Symbol),

    /// String literal: `"hello world"`
    StringLiteral(Symbol),

    // Character literal: `x` (backtick syntax)
    CharLiteral(Symbol),

    // Index Access (1-indexed)
    Item,
    Items,

    // Possession (Genitive Case)
    Possessive,

    // Punctuation
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Period,

    // Arithmetic Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,  // Modulo operator

    // Comparison Operators
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    LtEq,
    /// `>=`
    GtEq,
    /// `==`
    EqEq,
    /// `!=`
    NotEq,

    /// Arrow for return type syntax: `->`
    Arrow,

    EOF,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenType,
    pub lexeme: Symbol,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenType, lexeme: Symbol, span: Span) -> Self {
        Token { kind, lexeme, span }
    }
}

impl TokenType {
    pub const WH_WORDS: &'static [TokenType] = &[
        TokenType::Who,
        TokenType::What,
        TokenType::Where,
        TokenType::When,
        TokenType::Why,
    ];

    pub const MODALS: &'static [TokenType] = &[
        TokenType::Must,
        TokenType::Shall,
        TokenType::Should,
        TokenType::Can,
        TokenType::May,
        TokenType::Cannot,
        TokenType::Would,
        TokenType::Could,
        TokenType::Might,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_new_stores_positions() {
        let span = Span::new(5, 10);
        assert_eq!(span.start, 5);
        assert_eq!(span.end, 10);
    }

    #[test]
    fn span_default_is_zero() {
        let span = Span::default();
        assert_eq!(span.start, 0);
        assert_eq!(span.end, 0);
    }

    #[test]
    fn token_has_span_field() {
        use logicaffeine_base::Interner;
        let mut interner = Interner::new();
        let lexeme = interner.intern("test");
        let token = Token::new(TokenType::Noun(lexeme), lexeme, Span::new(0, 4));
        assert_eq!(token.span.start, 0);
        assert_eq!(token.span.end, 4);
    }

    #[test]
    fn wh_words_contains_all_wh_tokens() {
        assert_eq!(TokenType::WH_WORDS.len(), 5);
        assert!(TokenType::WH_WORDS.contains(&TokenType::Who));
        assert!(TokenType::WH_WORDS.contains(&TokenType::What));
        assert!(TokenType::WH_WORDS.contains(&TokenType::Where));
        assert!(TokenType::WH_WORDS.contains(&TokenType::When));
        assert!(TokenType::WH_WORDS.contains(&TokenType::Why));
    }

    #[test]
    fn modals_contains_all_modal_tokens() {
        assert_eq!(TokenType::MODALS.len(), 9);
        assert!(TokenType::MODALS.contains(&TokenType::Must));
        assert!(TokenType::MODALS.contains(&TokenType::Shall));
        assert!(TokenType::MODALS.contains(&TokenType::Should));
        assert!(TokenType::MODALS.contains(&TokenType::Can));
        assert!(TokenType::MODALS.contains(&TokenType::May));
        assert!(TokenType::MODALS.contains(&TokenType::Cannot));
        assert!(TokenType::MODALS.contains(&TokenType::Would));
        assert!(TokenType::MODALS.contains(&TokenType::Could));
        assert!(TokenType::MODALS.contains(&TokenType::Might));
    }
}
