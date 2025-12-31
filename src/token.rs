use crate::context::{Case, Gender, Number};
use crate::intern::Symbol;
use crate::lexicon::{Aspect, Definiteness, Time, VerbClass};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    Theorem,
    Main,
    Definition,
    Proof,
    Example,
    Logic,
    Note,
    Function,  // Phase 32: ## To blocks
    TypeDef,   // Inline type definitions: ## A Point has:, ## A Color is one of:
    Policy,    // Phase 50: ## Policy blocks for security rules
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
    Trust,    // Phase 35: Documented assertion with justification
    Otherwise,
    Call,
    New,      // Phase 31: Constructor keyword
    Either,   // Phase 33: Sum type definition
    Inspect,  // Phase 33: Pattern matching
    Native,   // Phase 38: Native function modifier

    // Phase 10: IO Keywords
    Read,     // "Read input from..."
    Write,    // "Write x to file..."
    Console,  // "...from the console"
    File,     // "...from file..." or "...to file..."

    // Ownership Keywords (Move/Borrow Semantics)
    Give,  // Move ownership: "Give x to processor"
    Show,  // Immutable borrow: "Show x to console"

    // Phase 43D: Collection Operations
    Push,     // "Push x to items"
    Pop,      // "Pop from items"
    Copy,     // "copy of slice" → slice.to_vec()
    Through,  // "items 1 through 3" → inclusive slice
    Length,   // "length of items" → items.len()
    At,       // "items at i" → items[i]

    // Phase 8.5: Memory Management (Zones)
    Inside,   // "Inside a new zone..."
    Zone,     // "...zone called..."
    Called,   // "...called 'Scratch'"
    Size,     // "...of size 1 MB"
    Mapped,   // "...mapped from 'file.bin'"

    // Phase 9: Structured Concurrency
    Attempt,        // "Attempt all of the following:" -> concurrent (async, I/O-bound)
    Following,      // "the following"
    Simultaneously, // "Simultaneously:" -> parallel (CPU-bound)

    // Phase 46: Agent System (Actor Model)
    Spawn,    // "Spawn a Worker called 'w1'" -> create agent
    Send,     // "Send Ping to 'agent'" -> send message to agent
    Await,    // "Await response from 'agent' into result" -> receive message

    // Phase 47: Serialization
    Portable, // "A Message is Portable and has:" -> serde derives

    // Phase 48: Sipping Protocol
    Manifest, // "the manifest of Zone" -> FileSipper manifest
    Chunk,    // "the chunk at N in Zone" -> FileSipper chunk

    // Phase 49: CRDT Keywords
    Shared,   // "A Counter is Shared and has:" -> CRDT struct
    Merge,    // "Merge remote into local" -> CRDT merge
    Increase, // "Increase x's count by 10" -> GCounter increment

    // Phase 50: Security Keywords
    Check,    // "Check that user is admin" -> mandatory runtime guard

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

    // Lexical Ambiguity (Phase 12: Parse Forest)
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
    Respectively,  // Phase 35: Pairwise list coordination

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

    // Phase 33: String literals "hello world"
    StringLiteral(Symbol),

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

    // Grand Challenge: Comparison Operators
    Lt,        // <
    Gt,        // >
    LtEq,      // <=
    GtEq,      // >=
    EqEq,      // ==
    NotEq,     // !=

    // Phase 38: Arrow for return type syntax
    Arrow,  // ->

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
        use crate::intern::Interner;
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
        assert_eq!(TokenType::MODALS.len(), 6);
        assert!(TokenType::MODALS.contains(&TokenType::Must));
        assert!(TokenType::MODALS.contains(&TokenType::Shall));
        assert!(TokenType::MODALS.contains(&TokenType::Should));
        assert!(TokenType::MODALS.contains(&TokenType::Can));
        assert!(TokenType::MODALS.contains(&TokenType::May));
        assert!(TokenType::MODALS.contains(&TokenType::Cannot));
    }
}
