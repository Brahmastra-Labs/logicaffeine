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
    /// it-cleft / pseudo-cleft: "It was John who left." — focus + exhaustivity.
    Cleft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureKind {
    Much,
    Little,
}

/// Calendar and clock time units for Span/Duration expressions.
///
/// `Day`/`Week`/`Month`/`Year` are variable-length CALENDAR units (they feed a `Span`);
/// `Hour`/`Minute`/`Second`/`Millisecond`/`Microsecond`/`Nanosecond` are fixed CLOCK units
/// (they feed a nanosecond `Duration`) — see `parse_span_literal_from_num`'s own split. One
/// enum drives the whole word-form duration grammar (`3 seconds`, `1 millisecond`, `5 days
/// and 2 hours`); the compact suffix spellings (`100ms`, `50ns`) are a separate lexer path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
    Millisecond,
    Microsecond,
    Nanosecond,
}

/// Document structure block type markers.
///
/// LOGOS uses markdown-style `## Header` syntax to delimit different
/// sections of a program or proof document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    /// An unknown `## Header` a small edit distance from a CONSEQUENTIAL
    /// code header (`## Mian` → `Main`) — the parser fails loudly with the
    /// suggestion instead of silently treating the whole block as prose.
    SuspectedTypo { found: Symbol, suggestion: Symbol },
    /// `## Theorem` - Declares a proposition to be proved.
    Theorem,
    /// `## Main` - Program entry point for imperative code.
    Main,
    /// `## Definition` - Introduces new terminology or type definitions.
    Definition,
    /// `## Define` - Mints a vernacular-logic predicate definition that the
    /// prover unfolds (Rung 0a). Distinct from `## Definition` (type defs).
    Define,
    /// `## Axiom` - Declares a named first-order axiom in formal notation
    /// (`## Axiom name: for all a b, Cong(a,b,b,a).`). Its body is parsed by the
    /// formal-formula parser and registered as a shared premise for later theorems —
    /// the seam for an axiomatic base like Tarski geometry.
    Axiom,
    /// `## Theory` - Names a development that groups the `## Axiom`s and `## Theorem`s
    /// that follow it (`## Theory Tarski`).
    Theory,
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
    /// `## Component <Name> (props)` - An LGX component: `State:`/`Actions:`/`View:`/
    /// `Style:` sections lowered to a `__mount_<Name>`/`__render_<Name>` pair the runtime
    /// drives through turns (an action runs, the view repaints, the DOM commits).
    Component,
    /// Inline type definition: `## A Point has:` or `## A Color is one of:`.
    TypeDef,
    /// `## Session` — the fields a visitor's session carries between requests.
    ///
    /// A RECORD, declared in the same words a record is (`a user, which is Text.`), which the
    /// router loads from a signed cookie before dispatch and saves back when a route wrote to it.
    /// Distinct from `## Store`, which is shared state inside ONE running page: a session belongs
    /// to a visitor and outlives the request, and lives on the server.
    Session,
    /// `## Policy` - Security policy rule definitions.
    Policy,
    /// `## Requires` - External crate dependency declarations.
    Requires,
    /// `## Hardware` - Signal declarations for hardware verification.
    Hardware,
    /// `## Property` - Temporal assertions for hardware verification.
    Property,
    /// `## No` - Optimization annotation (followed by Memo, TCO, Peephole, Borrow, or Optimize).
    No,
    /// `## Tier` - Tiered-optimizer pin: `## Tier <opt> <eager|t1|t2|t3|never>` overrides
    /// the hotness tier at which that optimization runs (HOTSWAP §8).
    Tier,
    /// `## Require <opt>` - Optimization DEMAND: the inverse of `## No`. Compilation
    /// FAILS if the named optimization did not fire for this program (so a program
    /// can declare it depends on an optimization, and losing it becomes an error).
    Require,
    /// `## Test: <prose sentence>` - An independent test entry point with an
    /// imperative body. The prose name is captured RAW at the lexer (never
    /// tokenized as code) and carried on the header — it is the test's
    /// identity.
    Test { name: Symbol },
    /// `## Page /path (params)` — an LGX page component mounted at `path`. The path is
    /// read RAW off the header line (a URL is not LOGOS words); the typed params that
    /// follow lex normally.
    Page { path: Symbol },
    /// `## Store <Name> [persisted as "key" | shared on "topic"]` — a shared model.
    Store,
    /// `## Directive <name> (el: Element) and (value: T):` — a custom attribute behavior.
    Directive,
    /// `## Route <METHOD> /path (params) -> T:` — a typed HTTP route; method and path are
    /// read RAW off the header line, the params and return type lex normally.
    Route { method: Symbol, path: Symbol },
    /// `## Before each request (r: Request) -> Option of Response:` — middleware.
    Middleware,
    /// `## Redirect /from -> /to.` — both paths read raw; the whole line is the header.
    Redirect { from: Symbol, to: Symbol },
    /// `## Error page <status>` — the view served for an HTTP error status.
    ErrorPage { status: u16 },
}

/// Which LGX section a [`TokenType::RawBlock`] body belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    /// A `View:` body (or a `-> View:` function body): HTML with `{…}` template tags.
    View,
    /// A `Style:` body: CSS.
    Style,
    /// A `Head:` body: the head HTML a page declares for its own document
    /// (title, description, social card, canonical). Parsed by the same
    /// template machinery as a `View:`, because it is HTML with holes.
    Head,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenType {
    // Document Structure
    BlockHeader {
        block_type: BlockType,
    },

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
    /// "infinitely many": ∃^∞.
    InfinitelyMany,
    /// "finitely many", "only finitely many": ∃^<∞.
    FinitelyMany,
    /// "all but finitely many": ∀^∞, the cofinite quantifier.
    AllButFinitelyMany,

    // Negative Polarity Items (NPIs)
    Anything,
    Anyone,
    Nothing,
    Nobody,
    NoOne,
    Nowhere,
    Ever,
    Never,

    /// "infinitely often": G F, the scope recurring without end.
    InfinitelyOften,

    // Logical Connectives
    And,
    Or,
    If,
    Then,
    Not,
    Iff,
    Because,
    /// Concessive subordinator: "although"/"though"/"even though".
    Although,
    /// Temporal binary connective: "P until Q"
    Until,
    /// Temporal binary connective: "P release Q" (dual of Until)
    Release,
    /// Temporal binary connective: "P weak-until Q" (Until or Always)
    WeakUntil,
    /// Compiler-generated implication (e.g., quantifier restrictions from Kripke lowering).
    /// Distinguished from `If` which represents user-written conditionals.
    /// This separation gives downstream passes (KG extraction, SVA synthesis)
    /// irrefutable provenance: `If` = user intent, `Implies` = compiler glue.
    Implies,

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
    /// "Swap x and y." — the zero-garbage reversible register exchange (`Op::RevSwap`).
    Swap,
    Return,
    /// Exits the innermost while loop: `Break.`
    Break,
    Be,
    While,
    Repeat,
    For,
    In,
    From,
    Assert,
    /// Documented assertion with justification string.
    Trust,
    /// Enforced runtime invariant: `Require that <cond>.` → a hard `assert!`
    /// (survives release, unlike `Assert` → `debug_assert!`).
    Require,
    /// Function precondition clause: `Requires <check>.` (checked at entry).
    Requires,
    /// Function postcondition clause: `Ensures <check>.` (checked before return).
    Ensures,
    /// Test-only assertion: `Expect that <cond>.` — records-and-continues
    /// inside `## Test`/`## Example` bodies; refused elsewhere.
    Expect,
    /// Virtual-clock control: `Advance time by <duration>.` (test-only).
    Advance,
    /// Loud skip: `Skip this test because "<reason>".` (test-only).
    Skip,
    /// Observation statement (developer): `Note that <clause>.`
    Note,
    /// Observation statement (operator): `Warn that <clause>.`
    Warn,
    /// Observation statement (audit): `Record that <clause>.`
    Record,
    /// Trace span block: `During <gerund>:` (statement-initial, imperative).
    During,
    Otherwise,
    /// Alias for `Otherwise` - Pythonic else clause
    Else,
    /// Python-style else-if shorthand
    Elif,
    Call,
    /// Constructor keyword for struct instantiation.
    New,
    /// Sum type definition keyword.
    Either,
    /// Pattern matching statement keyword.
    Inspect,
    /// Native function modifier for FFI bindings.
    Native,
    /// Escape hatch header keyword: "Escape to Rust:"
    Escape,
    /// Raw code block captured verbatim from an escape hatch body.
    /// The Symbol holds the interned raw foreign code (indentation-stripped).
    EscapeBlock(Symbol),
    /// An LGX `View:` / `Style:` section body captured verbatim (base indentation
    /// stripped, inner indentation and blank lines preserved). `content_start` is the byte
    /// offset of the body's first character in the source and `indent` the whitespace
    /// stripped from each of its lines, so an offset inside the body maps back to the file
    /// and a template error points into the HTML.
    RawBlock {
        kind: RawKind,
        body: Symbol,
        content_start: usize,
        indent: usize,
    },

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
    /// "Converge remote into local" → CRDT idempotent lattice join
    Converge,
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
    /// "a followed by b" → sequence concatenation (merge two sequences into one)
    Followed,

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
    Whose,
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

    /// Currency-symbol money literal: `$19.99`, `€5`, `£10`, `¥100`. Carries the magnitude (digits +
    /// optional decimal point, thousands separators stripped) and the resolved ISO-4217 code, so a
    /// money-aware parser builds `money(..)` while a magnitude-only consumer can still read `amount`.
    MoneyLiteral {
        amount: Symbol,
        currency: Symbol,
    },

    /// Duration literal with SI suffix: 500ms, 2s, 50ns
    /// Stores the value normalized to nanoseconds and preserves the original unit.
    DurationLiteral {
        nanos: i64,
        original_unit: Symbol,
    },

    /// Date literal in ISO-8601 format: 2026-05-20
    /// Stores days since Unix epoch (1970-01-01).
    DateLiteral {
        days: i32,
    },

    /// Time-of-day literal: 4pm, 9:30am, noon, midnight
    /// Stores nanoseconds from midnight (00:00:00).
    TimeLiteral {
        nanos_from_midnight: i64,
    },

    /// Calendar time unit word: day, week, month, year (or plurals)
    /// Used in Span expressions like "3 days" or "2 months and 5 days"
    CalendarUnit(CalendarUnit),

    /// Postfix operator for relative past time: "3 days ago"
    Ago,

    /// Postfix operator for relative future time: "3 days hence"
    Hence,

    /// Binary operator for span subtraction from a date: "3 days before 2026-05-20"
    Before,

    /// String literal: `"hello world"`
    StringLiteral(Symbol),

    /// Interpolated string literal: `"Hello, {name}!"`
    /// Contains raw content with {} holes preserved
    InterpolatedString(Symbol),

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
    /// `{` — map/set literal opener (`{k: v}`, `{a, b}`). Interpolation braces
    /// never reach here (they are consumed inside the string-literal path).
    LBrace,
    /// `&` in IMPERATIVE code — bitwise AND on Int, intersection on Sets.
    /// In prose the same character stays the coordination/firm-name joiner.
    Amp,
    /// `|` in imperative code — bitwise OR on Int, union on Sets. (`Pipe`
    /// is taken by the channel keyword `Pipe of T`.)
    VBar,
    /// `~` in imperative code — bitwise complement (lowers to `x ^ -1`).
    Tilde,
    /// `^` in imperative code — bitwise XOR on Int, symmetric difference on
    /// Sets (the word `xor` remains the English spelling).
    Caret,
    /// `}` — map/set literal closer.
    RBrace,
    Comma,
    Period,
    /// `.` as the FIELD-ACCESS / UFCS-method operator (imperative only): `p.x`
    /// (≡ `p's x`) and `xs.f(a)` (≡ `f(xs, a)`). Distinguished from a sentence
    /// `Period` in the lexer by the no-whitespace + identifier-on-both-sides rule.
    Dot,

    // Bitwise Operators
    /// "x xor y" → bitwise XOR (`^`)
    Xor,
    /// "x shifted left/right by y" → bit shift (`<<`/`>>`)
    Shifted,

    // Arithmetic Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent, // Modulo operator
    /// Compound assignment operators — `x += e` desugars to `Set x to x <op> e`.
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    /// `**` — the exponentiation operator.
    StarStar,
    /// `//` — floor division (rounds toward negative infinity).
    SlashSlash,

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

    /// Assignment operator `=` for `identifier = value` syntax
    Assign,

    /// Mutability keyword `mut` for explicit mutable declarations
    Mut,

    /// Generic identifier (for equals-style assignment)
    Identifier,

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

    /// A determiner beyond `all`, `no`, `some`, `any` and the counted ones, binding a quantifier
    /// of its own: `most`, `few`, `many`, `infinitely many`, `finitely many`, `all but finitely
    /// many`.
    pub fn is_generalized_quantifier(&self) -> bool {
        matches!(
            self,
            TokenType::Most
                | TokenType::Few
                | TokenType::Many
                | TokenType::InfinitelyMany
                | TokenType::FinitelyMany
                | TokenType::AllButFinitelyMany
        )
    }

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
#[path = "../../logicaffeine_tests/unit/logicaffeine_language/token/tests.rs"]
mod tests;
