//! The one teaching truth for LOGOS constructs: every lesson a surface shows
//! — LSP hover, completion documentation, the REPL's `:explain` — comes from
//! THIS table, so the editor and the terminal always teach the same thing.
//!
//! A [`ConstructDoc`] is complete by construction: the type has no optional
//! teaching fields, so an entry cannot exist without a plain-English
//! sentence, a runnable example, and a socratic question or tip. The
//! `doc_for` match is wildcard-free (the `token_class` idiom): adding a
//! `TokenType` variant does not compile until someone decides whether it
//! deserves a lesson — the decision lives here, next to the enum, not in an
//! editor plugin. `doc_for_block` is total: every `##` block type teaches.

use crate::token::{BlockType, TokenType};

/// A complete lesson for one construct. Every field is required by the TYPE —
/// an entry cannot exist without a plain sentence, a runnable example, and a
/// socratic question or tip. Clear-and-easy is structural, not aspirational.
pub struct ConstructDoc {
    /// The construct's display name ("Give", "Main", "Seq").
    pub name: &'static str,
    /// ONE plain sentence: what it does. Kept under 90 characters by the
    /// teach lock — if it needs more, the rest belongs in the question/tip.
    pub what: &'static str,
    /// A runnable snippet (code-block body, no fences). Must lex.
    pub example: &'static str,
    /// The socratic seat: a guiding question (contains `?`) or an explicit
    /// `Tip:` — it leads the reader to the next insight, never just restates.
    pub question_or_tip: &'static str,
    /// A LOGOS_QUICKGUIDE.md heading slug for "read more", when one fits.
    pub guide_anchor: Option<&'static str>,
}

const fn lesson(
    name: &'static str,
    what: &'static str,
    example: &'static str,
    question_or_tip: &'static str,
    guide_anchor: Option<&'static str>,
) -> ConstructDoc {
    ConstructDoc { name, what, example, question_or_tip, guide_anchor }
}

// ---------------------------------------------------------------------------
// Statement keywords
// ---------------------------------------------------------------------------

static LET: ConstructDoc = lesson(
    "Let",
    "Declares a new variable.",
    "Let x be 5.\nLet name: Text be \"Alice\".",
    "Will the value change later? Then declare it `Let mutable x be 5.` — plain `Let` is immutable.",
    Some("2-variables--mutation"),
);

static SET_KW: ConstructDoc = lesson(
    "Set",
    "Updates an existing mutable variable.",
    "Set x to 10.",
    "Was `x` declared with `Let mutable`? Only mutable bindings can be `Set`.",
    Some("2-variables--mutation"),
);

static RETURN: ConstructDoc = lesson(
    "Return",
    "Hands a value back from the current function.",
    "Return x.",
    "Does the value's type match the function's declared `-> Type`? `Return.` alone returns nothing.",
    Some("6-control-flow"),
);

static IF: ConstructDoc = lesson(
    "If",
    "Runs a block only when its condition holds.",
    "If x > 0:\n    Show x.\nOtherwise:\n    Show 0.",
    "What should happen when the condition is false — nothing, an `Otherwise:` branch, or an `elif`?",
    Some("6-control-flow"),
);

static WHILE: ConstructDoc = lesson(
    "While",
    "Repeats a block as long as its condition stays true.",
    "While x > 0:\n    Set x to x - 1.",
    "Does the body change the condition? A body that never does loops forever — `(decreasing e)` proves termination.",
    Some("6-control-flow"),
);

static REPEAT: ConstructDoc = lesson(
    "Repeat",
    "Walks a collection, binding each element in turn.",
    "Repeat for item in items:\n    Show item.",
    "Need positions instead of elements? Count with `for i from 1 to n:` — LOGOS is 1-based.",
    Some("5-collections"),
);

static SHOW: ConstructDoc = lesson(
    "Show",
    "Displays a value while only borrowing it — you keep ownership.",
    "Show x.",
    "Need the value afterward? `Show` lends; `Give` transfers — pick by who owns it next.",
    Some("13-output"),
);

static GIVE: ConstructDoc = lesson(
    "Give",
    "Transfers ownership of a value to a new owner.",
    "Give x to processor.",
    "Who should own this value afterward — you, or the receiver? Keep it by giving `a copy of x`.",
    Some("13-output"),
);

static PUSH: ConstructDoc = lesson(
    "Push",
    "Appends a value to the end of a sequence.",
    "Push 5 to items.",
    "Tip: `Push` grows a Seq by one; for Sets use `Add v to s.` — sets have no order to push onto.",
    Some("5-collections"),
);

static INSPECT: ConstructDoc = lesson(
    "Inspect",
    "Pattern-matches a value, running one branch per variant.",
    "Inspect shape:\n    When Circle (r):\n        Show r.\n    Otherwise:\n        Show \"other\".",
    "Is every variant handled? `Otherwise:` catches the rest — or list each `When` and let the checker verify.",
    Some("6-control-flow"),
);

static CALL: ConstructDoc = lesson(
    "Call",
    "Invokes a function as a statement.",
    "Call process with data.",
    "Tip: in expressions call directly — `add(3, 7)`; `Call f with a and b.` is the statement form.",
    Some("7-functions--closures"),
);

static NEW: ConstructDoc = lesson(
    "New",
    "Creates a struct or variant instance with named field values.",
    "Let p be a new Point with x 10 and y 20.",
    "Tip: fields are set by name — `with x 10 and y 20` — so argument order never bites.",
    Some("8-structs-enums--field-access"),
);

static ESCAPE: ConstructDoc = lesson(
    "Escape",
    "Embeds raw foreign code, skipping every LOGOS checker.",
    "Escape to Rust:\n    println!(\"hello\");",
    "Could plain LOGOS express this? Escaped code is invisible to the ownership and type checkers.",
    None,
);

static CHECK: ConstructDoc = lesson(
    "Check",
    "Enforces a security capability at runtime — a mandatory gate.",
    "Check that balance is at least amount.",
    "What may this code path do, and who says so? `## Policy` blocks define the capabilities.",
    Some("10-contracts-refinement-assert-trust-check"),
);

static POP: ConstructDoc = lesson(
    "Pop",
    "Removes the last element of a sequence.",
    "Pop from xs.\nPop from xs into y.",
    "Need the popped value? `Pop from xs into y.` binds it; plain `Pop` discards it.",
    Some("5-collections"),
);

static ADD: ConstructDoc = lesson(
    "Add",
    "Inserts a value into a set.",
    "Add v to s.",
    "Tip: sets keep one of each value — adding a duplicate changes nothing, and order is not kept.",
    Some("5-collections"),
);

static REMOVE: ConstructDoc = lesson(
    "Remove",
    "Deletes a value from a set.",
    "Remove v from s.",
    "Tip: removing a value that isn't present is a quiet no-op — test `s contains v` when it matters.",
    Some("5-collections"),
);

static BREAK: ConstructDoc = lesson(
    "Break",
    "Exits the innermost loop immediately.",
    "While true:\n    Break.",
    "Which loop should stop? `Break.` only leaves the innermost one.",
    Some("6-control-flow"),
);

static ASSERT: ConstructDoc = lesson(
    "Assert",
    "Checks a condition at runtime, failing loudly when it is false.",
    "Assert that x is equal to 42.",
    "Is this a debugging aid or a security rule? Debug checks `Assert`; mandatory gates use `Check`.",
    Some("10-contracts-refinement-assert-trust-check"),
);

static TRUST: ConstructDoc = lesson(
    "Trust",
    "States a justified assumption, carrying its reason.",
    "Trust that x is greater than 0 because \"set to 10\".",
    "Why is this safe? The `because` reason is required — future readers hold you to it.",
    Some("10-contracts-refinement-assert-trust-check"),
);

static INCREASE: ConstructDoc = lesson(
    "Increase",
    "Grows a shared CRDT counter field — merges without conflicts.",
    "Increase c's points by 10.",
    "Tip: `Increase` is for `Shared` CRDT fields; plain integers update with `Set i to i + 1.`",
    Some("12-distributed-crdt-concurrency-networking-zones"),
);

static DECREASE: ConstructDoc = lesson(
    "Decrease",
    "Shrinks a shared CRDT counter field — merges without conflicts.",
    "Decrease g's score by 30.",
    "Tip: `Decrease` is for `Shared` CRDT fields; plain integers update with `Set i to i - 1.`",
    Some("12-distributed-crdt-concurrency-networking-zones"),
);

static SPAWN: ConstructDoc = lesson(
    "Spawn",
    "Starts an agent — an independent concurrent actor.",
    "Spawn an EchoAgent called \"echo\".",
    "How will you reach it later? The `called \"name\"` handle is how messages find the agent.",
    Some("12-distributed-crdt-concurrency-networking-zones"),
);

static MERGE: ConstructDoc = lesson(
    "Merge",
    "Folds one CRDT replica into another without conflicts.",
    "Merge remote into local.",
    "Tip: merge order never matters — replicas converge to the same state either way.",
    Some("12-distributed-crdt-concurrency-networking-zones"),
);

// ---------------------------------------------------------------------------
// Block headers
// ---------------------------------------------------------------------------

static BLOCK_MAIN: ConstructDoc = lesson(
    "Main",
    "The program's entry point — statements here run top to bottom.",
    "## Main\nShow \"Hello, World!\".",
    "Tip: one `## Main` per program; everything else is definitions it can call.",
    Some("1-program-structure"),
);

static BLOCK_FUNCTION: ConstructDoc = lesson(
    "To",
    "Defines a function; parameters and return type live in the header.",
    "## To add (a: Int, b: Int) -> Int:\n    Return a + b.",
    "What does it give back? `-> Type` declares it; omit the arrow for a procedure.",
    Some("7-functions--closures"),
);

static BLOCK_THEOREM: ConstructDoc = lesson(
    "Theorem",
    "Declares a proposition to be proved.",
    "## Theorem: Socrates\nGiven: All men are mortal. Socrates is a man.\nProve: Socrates is mortal.\nProof: Auto.",
    "What structure does the claim have — universal, implication, equality? The proof strategy follows it.",
    Some("1-program-structure"),
);

static BLOCK_PROOF: ConstructDoc = lesson(
    "Proof",
    "Holds the proof steps for the theorem above it.",
    "Proof: Auto.",
    "Stuck? Start with `Auto.` — the prover reports what it can and cannot discharge.",
    None,
);

static BLOCK_DEFINITION: ConstructDoc = lesson(
    "Definition",
    "Introduces new terminology for later sentences to use.",
    "## Definition: A bachelor is an unmarried man.",
    "Tip: `## Definition` explains terms; executable data shapes live in `## A ... has:` blocks.",
    None,
);

static BLOCK_DEFINE: ConstructDoc = lesson(
    "Define",
    "Mints a predicate the prover can unfold by definition.",
    "## Define: A number is tiny if it is less than 10.",
    "Tip: proofs may unfold this exactly — the prover substitutes the body for the name.",
    None,
);

static BLOCK_AXIOM: ConstructDoc = lesson(
    "Axiom",
    "Declares a named formal axiom as a shared premise.",
    "## Axiom cong_refl: for all a b, Cong(a,b,b,a).",
    "Tip: axioms are believed, not proved — keep them few and inspectable.",
    None,
);

static BLOCK_THEORY: ConstructDoc = lesson(
    "Theory",
    "Names a development grouping the axioms and theorems after it.",
    "## Theory Tarski",
    "Tip: a theory bundles `## Axiom`s with the `## Theorem`s proved from them.",
    None,
);

static BLOCK_TYPEDEF: ConstructDoc = lesson(
    "TypeDef",
    "Defines a struct or enum type in English.",
    "## A Point has:\n    An x: Int.\n    A y: Int.",
    "One thing with fields, or one of several shapes? `has:` makes a struct; `is one of:` an enum.",
    Some("8-structs-enums--field-access"),
);

static BLOCK_POLICY: ConstructDoc = lesson(
    "Policy",
    "Defines the security rules that `Check` statements enforce.",
    "## Policy\nUsers can read public files.",
    "Who may do what? Policies name capabilities; `Check that ...` gates on them.",
    Some("10-contracts-refinement-assert-trust-check"),
);

static BLOCK_LOGIC: ConstructDoc = lesson(
    "Logic",
    "Holds direct logical notation instead of English.",
    "## Logic\nforall x. Man(x) -> Mortal(x)",
    "Tip: use it when symbols say it better — English and notation share one prover.",
    None,
);

static BLOCK_EXAMPLE: ConstructDoc = lesson(
    "Example",
    "Shows an illustrative example the compiler treats as documentation.",
    "## Example\nShow 42.",
    "Tip: examples read as prose — the highlighter fades them so code stands out.",
    None,
);

static BLOCK_NOTE: ConstructDoc = lesson(
    "Note",
    "Documentation prose the compiler skips and the highlighter fades.",
    "## Note\nThis module parses dates.",
    "Tip: a `## Note` right above a definition becomes that definition's documentation.",
    None,
);

static BLOCK_REQUIRES: ConstructDoc = lesson(
    "Requires",
    "Declares external crate dependencies for escaped code.",
    "## Requires\nserde",
    "Tip: only `Escape` blocks need this — pure LOGOS programs never do.",
    None,
);

static BLOCK_HARDWARE: ConstructDoc = lesson(
    "Hardware",
    "Declares hardware signals for verification.",
    "## Hardware\nclk is a signal.",
    "What are the inputs and state bits? Properties below verify against exactly these signals.",
    None,
);

static BLOCK_PROPERTY: ConstructDoc = lesson(
    "Property",
    "States temporal assertions about hardware signals.",
    "## Property\nThe counter is eventually zero.",
    "Always or eventually? Temporal words carry the meaning — the prover checks every cycle.",
    None,
);

static BLOCK_NO: ConstructDoc = lesson(
    "No",
    "Turns one optimization off for the code that follows.",
    "## No Memo",
    "Tip: use it to pin a benchmark or dodge a pathological case — semantics never change.",
    None,
);

static BLOCK_TIER: ConstructDoc = lesson(
    "Tier",
    "Pins the hotness tier at which an optimization runs.",
    "## Tier Memo eager",
    "Tip: `eager`, `t1`–`t3`, or `never` — this tunes WHEN the optimizer fires, not correctness.",
    None,
);

static BLOCK_SUSPECTED_TYPO: ConstructDoc = lesson(
    "Unknown header",
    "An unknown header that looks like a typo of a real one.",
    "## Mian",
    "Did you mean the suggested header? Unknown `##` headers otherwise read as prose.",
    None,
);

// ---------------------------------------------------------------------------
// Primitive and built-in generic types
// ---------------------------------------------------------------------------

static TY_INT: ConstructDoc = lesson(
    "Int",
    "A whole number.",
    "Let n: Int be 42.",
    "Tip: `/` divides and `%` takes the remainder — `x is divisible by n` reads best in conditions.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static TY_NAT: ConstructDoc = lesson(
    "Nat",
    "A whole number that can never be negative.",
    "Let count: Nat be 0.",
    "Can this value ever go below zero? If subtraction might take it there, use Int instead.",
    None,
);

static TY_TEXT: ConstructDoc = lesson(
    "Text",
    "A string of characters.",
    "Let name: Text be \"Alice\".",
    "Tip: build strings by interpolation — `\"Hello, {name}!\"` — or `a combined with b`.",
    Some("4-strings"),
);

static TY_BOOL: ConstructDoc = lesson(
    "Bool",
    "Either true or false.",
    "Let ready: Bool be true.",
    "Tip: `and`/`or`/`not` short-circuit and return Bool; 0 and empty collections count as false.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static TY_FLOAT: ConstructDoc = lesson(
    "Float",
    "A floating-point number for fractional values.",
    "Let pi: Float be 3.14159.",
    "Tip: floats round — format output with `\"{pi:.2}\"`, and avoid them for money.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static TY_UNIT: ConstructDoc = lesson(
    "Unit",
    "The empty value — what procedures return.",
    "## To greet (name: Text):\n    Show name.",
    "Tip: you rarely write Unit; a function without `-> Type` returns it implicitly.",
    None,
);

static TY_CHAR: ConstructDoc = lesson(
    "Char",
    "A single character.",
    "Let c: Char be 'a'.",
    "Tip: Text is a sequence of Chars — one Char is the unit you get when you walk it.",
    None,
);

static TY_BYTE: ConstructDoc = lesson(
    "Byte",
    "A single byte — a whole number from 0 to 255.",
    "Let b: Byte be 255.",
    "Tip: bytes are the unit of binary data; whole numbers past 255 need Int.",
    None,
);

static TY_LIST: ConstructDoc = lesson(
    "List",
    "An ordered, growable collection (another name for Seq).",
    "Let xs be a new List of Int.",
    "Tip: `[1, 2, 3]` is the literal form; `Push` appends; indexing is 1-based.",
    Some("5-collections"),
);

static TY_SEQ: ConstructDoc = lesson(
    "Seq",
    "An ordered, growable collection — the canonical list type.",
    "Let xs: Seq of Int be [1, 2, 3].",
    "Tip: `item 1 of xs` is the FIRST element — LOGOS indexing is 1-based.",
    Some("5-collections"),
);

static TY_MAP: ConstructDoc = lesson(
    "Map",
    "A dictionary from keys to values.",
    "Let m be a new Map of Text to Int.\nSet m at \"a\" to 1.",
    "Tip: read with `item k of m` and write with `Set m at k to v.`",
    Some("5-collections"),
);

static TY_SET: ConstructDoc = lesson(
    "Set",
    "An unordered collection holding each value once.",
    "Let s be a new Set of Int.\nAdd 3 to s.",
    "Tip: on sets `&`/`|`/`^` are intersection/union/symmetric-difference; `a without b` subtracts.",
    Some("5-collections"),
);

static TY_OPTION: ConstructDoc = lesson(
    "Option",
    "A value that is either present or absent.",
    "Let maybe be some 30.\nInspect maybe:\n    When OptionSome (v):\n        Show v.\n    When OptionNone:\n        Show \"nothing\".",
    "What happens when the value is absent? `Inspect` makes you answer both ways.",
    Some("9-options--pattern-matching"),
);

static TY_RESULT: ConstructDoc = lesson(
    "Result",
    "A success value or an error value — one or the other.",
    "## To native read (path: Text) -> Result of Text and Text",
    "Which side is which? `Result of Ok and Err` — handle both with `Inspect`.",
    None,
);

/// Every lesson, for parity ratchets and the REPL's suggestion list.
pub static ALL_DOCS: &[&ConstructDoc] = &[
    // keywords
    &LET, &SET_KW, &RETURN, &IF, &WHILE, &REPEAT, &SHOW, &GIVE, &PUSH, &INSPECT, &CALL, &NEW,
    &ESCAPE, &CHECK, &POP, &ADD, &REMOVE, &BREAK, &ASSERT, &TRUST, &INCREASE, &DECREASE, &SPAWN,
    &MERGE,
    // block headers
    &BLOCK_MAIN, &BLOCK_FUNCTION, &BLOCK_THEOREM, &BLOCK_PROOF, &BLOCK_DEFINITION, &BLOCK_DEFINE,
    &BLOCK_AXIOM, &BLOCK_THEORY, &BLOCK_TYPEDEF, &BLOCK_POLICY, &BLOCK_LOGIC, &BLOCK_EXAMPLE,
    &BLOCK_NOTE, &BLOCK_REQUIRES, &BLOCK_HARDWARE, &BLOCK_PROPERTY, &BLOCK_NO, &BLOCK_TIER,
    &BLOCK_SUSPECTED_TYPO,
    // types
    &TY_INT, &TY_NAT, &TY_TEXT, &TY_BOOL, &TY_FLOAT, &TY_UNIT, &TY_CHAR, &TY_BYTE, &TY_LIST,
    &TY_SEQ, &TY_MAP, &TY_SET, &TY_OPTION, &TY_RESULT,
];

/// The lesson for a statement keyword, when the token deserves one.
///
/// Wildcard-free: a new `TokenType` variant does not compile until someone
/// decides whether it teaches (a lesson arm) or not (the grouped `None` arm).
pub fn doc_for(kind: &TokenType) -> Option<&'static ConstructDoc> {
    match kind {
        TokenType::Let => Some(&LET),
        TokenType::Set => Some(&SET_KW),
        TokenType::Return => Some(&RETURN),
        TokenType::If => Some(&IF),
        TokenType::While => Some(&WHILE),
        TokenType::Repeat => Some(&REPEAT),
        TokenType::Show => Some(&SHOW),
        TokenType::Give => Some(&GIVE),
        TokenType::Push => Some(&PUSH),
        TokenType::Inspect => Some(&INSPECT),
        TokenType::Call => Some(&CALL),
        TokenType::New => Some(&NEW),
        TokenType::Escape => Some(&ESCAPE),
        TokenType::Check => Some(&CHECK),
        TokenType::Pop => Some(&POP),
        TokenType::Add => Some(&ADD),
        TokenType::Remove => Some(&REMOVE),
        TokenType::Break => Some(&BREAK),
        TokenType::Assert => Some(&ASSERT),
        TokenType::Trust => Some(&TRUST),
        TokenType::Increase => Some(&INCREASE),
        TokenType::Decrease => Some(&DECREASE),
        TokenType::Spawn => Some(&SPAWN),
        TokenType::Merge => Some(&MERGE),

        // No lesson (yet): block headers teach through `doc_for_block`;
        // everything else is either structural, symbolic, or a word whose
        // meaning the sentence around it carries.
        TokenType::BlockHeader { .. }
        | TokenType::All
        | TokenType::No
        | TokenType::Some
        | TokenType::Any
        | TokenType::Both
        | TokenType::Most
        | TokenType::Few
        | TokenType::Many
        | TokenType::Cardinal(_)
        | TokenType::AtLeast(_)
        | TokenType::AtMost(_)
        | TokenType::Anything
        | TokenType::Anyone
        | TokenType::Nothing
        | TokenType::Nobody
        | TokenType::NoOne
        | TokenType::Nowhere
        | TokenType::Ever
        | TokenType::Never
        | TokenType::And
        | TokenType::Or
        | TokenType::Then
        | TokenType::Not
        | TokenType::Iff
        | TokenType::Because
        | TokenType::Although
        | TokenType::Until
        | TokenType::Release
        | TokenType::WeakUntil
        | TokenType::Implies
        | TokenType::Must
        | TokenType::Shall
        | TokenType::Should
        | TokenType::Can
        | TokenType::May
        | TokenType::Cannot
        | TokenType::Would
        | TokenType::Could
        | TokenType::Might
        | TokenType::Had
        | TokenType::Be
        | TokenType::For
        | TokenType::In
        | TokenType::From
        | TokenType::Require
        | TokenType::Requires
        | TokenType::Ensures
        | TokenType::Otherwise
        | TokenType::Else
        | TokenType::Elif
        | TokenType::Either
        | TokenType::Native
        | TokenType::EscapeBlock(_)
        | TokenType::Given
        | TokenType::Prove
        | TokenType::Auto
        | TokenType::Read
        | TokenType::Write
        | TokenType::Console
        | TokenType::File
        | TokenType::Copy
        | TokenType::Through
        | TokenType::Length
        | TokenType::At
        | TokenType::Contains
        | TokenType::Union
        | TokenType::Intersection
        | TokenType::Inside
        | TokenType::Zone
        | TokenType::Called
        | TokenType::Size
        | TokenType::Mapped
        | TokenType::Attempt
        | TokenType::Following
        | TokenType::Simultaneously
        | TokenType::Send
        | TokenType::Await
        | TokenType::Portable
        | TokenType::Manifest
        | TokenType::Chunk
        | TokenType::Shared
        | TokenType::Tally
        | TokenType::SharedSet
        | TokenType::SharedSequence
        | TokenType::CollaborativeSequence
        | TokenType::SharedMap
        | TokenType::Divergent
        | TokenType::Append
        | TokenType::Resolve
        | TokenType::RemoveWins
        | TokenType::AddWins
        | TokenType::YATA
        | TokenType::Values
        | TokenType::Listen
        | TokenType::NetConnect
        | TokenType::Sleep
        | TokenType::Sync
        | TokenType::Mount
        | TokenType::Persistent
        | TokenType::Combined
        | TokenType::Followed
        | TokenType::Launch
        | TokenType::Task
        | TokenType::Pipe
        | TokenType::Receive
        | TokenType::Stop
        | TokenType::Try
        | TokenType::Into
        | TokenType::First
        | TokenType::After
        | TokenType::Colon
        | TokenType::Indent
        | TokenType::Dedent
        | TokenType::Newline
        | TokenType::Noun(_)
        | TokenType::Adjective(_)
        | TokenType::NonIntersectiveAdjective(_)
        | TokenType::Adverb(_)
        | TokenType::ScopalAdverb(_)
        | TokenType::TemporalAdverb(_)
        | TokenType::Verb { .. }
        | TokenType::ProperName(_)
        | TokenType::Ambiguous { .. }
        | TokenType::Performative(_)
        | TokenType::Exclamation
        | TokenType::Article(_)
        | TokenType::Auxiliary(_)
        | TokenType::Is
        | TokenType::Are
        | TokenType::Was
        | TokenType::Were
        | TokenType::That
        | TokenType::Who
        | TokenType::What
        | TokenType::Where
        | TokenType::Whose
        | TokenType::When
        | TokenType::Why
        | TokenType::Does
        | TokenType::Do
        | TokenType::Identity
        | TokenType::Equals
        | TokenType::Reflexive
        | TokenType::Reciprocal
        | TokenType::Respectively
        | TokenType::Pronoun { .. }
        | TokenType::Preposition(_)
        | TokenType::Particle(_)
        | TokenType::Comparative(_)
        | TokenType::Superlative(_)
        | TokenType::Than
        | TokenType::To
        | TokenType::PresupTrigger(_)
        | TokenType::Focus(_)
        | TokenType::Measure(_)
        | TokenType::Number(_)
        | TokenType::MoneyLiteral { .. }
        | TokenType::DurationLiteral { .. }
        | TokenType::DateLiteral { .. }
        | TokenType::TimeLiteral { .. }
        | TokenType::CalendarUnit(_)
        | TokenType::Ago
        | TokenType::Hence
        | TokenType::Before
        | TokenType::StringLiteral(_)
        | TokenType::InterpolatedString(_)
        | TokenType::CharLiteral(_)
        | TokenType::Item
        | TokenType::Items
        | TokenType::Possessive
        | TokenType::LParen
        | TokenType::RParen
        | TokenType::LBracket
        | TokenType::RBracket
        | TokenType::LBrace
        | TokenType::Amp
        | TokenType::VBar
        | TokenType::Tilde
        | TokenType::Caret
        | TokenType::RBrace
        | TokenType::Comma
        | TokenType::Period
        | TokenType::Dot
        | TokenType::Xor
        | TokenType::Shifted
        | TokenType::Plus
        | TokenType::Minus
        | TokenType::Star
        | TokenType::Slash
        | TokenType::Percent
        | TokenType::PlusEq
        | TokenType::MinusEq
        | TokenType::StarEq
        | TokenType::SlashEq
        | TokenType::PercentEq
        | TokenType::StarStar
        | TokenType::SlashSlash
        | TokenType::Lt
        | TokenType::Gt
        | TokenType::LtEq
        | TokenType::GtEq
        | TokenType::EqEq
        | TokenType::NotEq
        | TokenType::Arrow
        | TokenType::Assign
        | TokenType::Mut
        | TokenType::Identifier
        | TokenType::EOF => None,
    }
}

/// The lesson for a `##` block header. TOTAL — every block type teaches, and
/// a new `BlockType` variant does not compile until it gets a lesson.
pub fn doc_for_block(block: &BlockType) -> &'static ConstructDoc {
    match block {
        BlockType::SuspectedTypo { .. } => &BLOCK_SUSPECTED_TYPO,
        BlockType::Theorem => &BLOCK_THEOREM,
        BlockType::Main => &BLOCK_MAIN,
        BlockType::Definition => &BLOCK_DEFINITION,
        BlockType::Define => &BLOCK_DEFINE,
        BlockType::Axiom => &BLOCK_AXIOM,
        BlockType::Theory => &BLOCK_THEORY,
        BlockType::Proof => &BLOCK_PROOF,
        BlockType::Example => &BLOCK_EXAMPLE,
        BlockType::Logic => &BLOCK_LOGIC,
        BlockType::Note => &BLOCK_NOTE,
        BlockType::Function => &BLOCK_FUNCTION,
        BlockType::TypeDef => &BLOCK_TYPEDEF,
        BlockType::Policy => &BLOCK_POLICY,
        BlockType::Requires => &BLOCK_REQUIRES,
        BlockType::Hardware => &BLOCK_HARDWARE,
        BlockType::Property => &BLOCK_PROPERTY,
        BlockType::No => &BLOCK_NO,
        BlockType::Tier => &BLOCK_TIER,
    }
}

/// The lesson for a primitive or built-in generic type name.
pub fn doc_for_primitive(name: &str) -> Option<&'static ConstructDoc> {
    match name {
        "Int" => Some(&TY_INT),
        "Nat" => Some(&TY_NAT),
        "Text" => Some(&TY_TEXT),
        "Bool" => Some(&TY_BOOL),
        "Float" => Some(&TY_FLOAT),
        "Unit" => Some(&TY_UNIT),
        "Char" => Some(&TY_CHAR),
        "Byte" => Some(&TY_BYTE),
        "List" => Some(&TY_LIST),
        "Seq" => Some(&TY_SEQ),
        "Map" => Some(&TY_MAP),
        "Set" => Some(&TY_SET),
        "Option" => Some(&TY_OPTION),
        "Result" => Some(&TY_RESULT),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Literate-doc extraction — the prose → documentation pipeline
// ---------------------------------------------------------------------------

/// One documented definition pulled from a literate LOGOS module.
pub struct LiterateDoc {
    /// The defined name (`read`, `Point`).
    pub name: String,
    /// The full `##` header line, verbatim.
    pub signature: String,
    /// The body of the `## Note` block directly above the header, when one
    /// exists. Notes are the ONLY per-definition doc carrier — bare prose
    /// between sections is not literate LOGOS.
    pub doc: Option<String>,
}

/// The module-level documentation: the prose between the `# Title` line and
/// the first `##` section (the region the lexer skips).
pub fn module_doc(source: &str) -> Option<String> {
    let first_section = if source.starts_with("## ") {
        0
    } else {
        source.find("\n## ").map(|i| i + 1).unwrap_or(source.len())
    };
    let prose: Vec<&str> = source[..first_section]
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    (!prose.is_empty()).then(|| prose.join("\n"))
}

/// The documentation for the `##` header starting at `header_start`: the body
/// of a `## Note` block IMMEDIATELY above it (the nearest preceding header
/// must be the Note — any other header in between means no doc).
pub fn doc_for_header_at(source: &str, header_start: usize) -> Option<String> {
    let head = source.get(..header_start)?;
    let mut nearest_header: Option<usize> = None;
    let mut offset = 0;
    for line in head.split_inclusive('\n') {
        if line.trim_start().starts_with("## ") {
            nearest_header = Some(offset);
        }
        offset += line.len();
    }
    let note_start = nearest_header?;
    let note_line_end = note_start + head[note_start..].find('\n')?;
    if head[note_start..note_line_end].trim() != "## Note" {
        return None;
    }
    let body = head[note_line_end + 1..].trim();
    (!body.is_empty()).then(|| body.to_string())
}

/// Every `## To …` / `## A … ` definition in a literate module, with its
/// `## Note` documentation attached where one sits directly above. Types
/// declared inside a `## Definition` body inherit the block's Note.
pub fn extract_literate_docs(source: &str) -> Vec<LiterateDoc> {
    let mut docs = Vec::new();
    let mut offset = 0;
    let mut definition_doc: Option<Option<String>> = None;
    for line in source.split_inclusive('\n') {
        let header = line.trim_end().trim_start();
        if header.starts_with("## ") {
            definition_doc =
                (header == "## Definition").then(|| doc_for_header_at(source, offset));
        }
        if let Some(name) = literate_definition_name(header) {
            docs.push(LiterateDoc {
                name,
                signature: header.to_string(),
                doc: doc_for_header_at(source, offset),
            });
        } else if let Some(block_doc) = &definition_doc {
            if let Some(name) = definition_body_type_name(header) {
                docs.push(LiterateDoc {
                    name,
                    signature: header.to_string(),
                    doc: block_doc.clone(),
                });
            }
        }
        offset += line.len();
    }
    docs
}

/// The type a `## Definition` BODY line declares: `A <Name> has …` /
/// `A <Name> is …` (mirrors the loader's type-header rule).
fn definition_body_type_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("A ").or_else(|| line.strip_prefix("An "))?;
    let word = rest.split(|c: char| c == '(' || c == '.' || c.is_whitespace()).next()?;
    if !word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let tail = rest[word.len()..].trim_start();
    (tail.starts_with("has") || tail.starts_with("is") || tail.starts_with("of"))
        .then(|| word.to_string())
}

/// The name a `##` definition header defines, mirroring the loader's
/// trigger-name rules: `## To [native] <name>` and `## A/An <TypeName> …`.
fn literate_definition_name(header: &str) -> Option<String> {
    if let Some(rest) = header.strip_prefix("## To ") {
        let rest = rest.strip_prefix("native ").unwrap_or(rest);
        let name = rest.split(|c: char| c == '(' || c.is_whitespace()).next()?;
        return (!name.is_empty()).then(|| name.to_string());
    }
    let rest = header.strip_prefix("## A ").or_else(|| header.strip_prefix("## An "))?;
    let word = rest
        .split(|c: char| c == '(' || c == '.' || c == ':' || c.is_whitespace())
        .next()?;
    word.chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
        .then(|| word.to_string())
}

/// Every lesson whose name matches the word, case-insensitively — a word can
/// name more than one construct (`Set` the statement, `Set` the type), and an
/// honest teacher shows both.
pub fn docs_for_word(word: &str) -> Vec<&'static ConstructDoc> {
    ALL_DOCS
        .iter()
        .filter(|doc| doc.name.eq_ignore_ascii_case(word))
        .copied()
        .collect()
}

/// The first lesson whose name matches the word, case-insensitively.
pub fn doc_for_word(word: &str) -> Option<&'static ConstructDoc> {
    docs_for_word(word).into_iter().next()
}
