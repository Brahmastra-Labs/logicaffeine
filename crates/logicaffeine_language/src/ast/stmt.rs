//! Imperative statement AST types for the LOGOS language.
//!
//! This module defines statement types for the imperative fragment including:
//!
//! - **[`Stmt`]**: Statement variants (let, if, match, while, for, function defs)
//! - **[`Expr`]**: Imperative expressions (field access, method calls, literals)
//! - **[`TypeExpr`]**: Type annotations with refinements and generics
//! - **[`Literal`]**: Literal values (numbers, strings, booleans)
//! - **[`Block`]**: Statement blocks with optional return expressions
//!
//! The imperative AST is used in LOGOS mode for generating executable Rust code.

use super::axiom::{AxiomBlock, TheoryBlock};
use super::definition::DefinitionBlock;
use super::logic::LogicExpr;
use super::theorem::TheoremBlock;
use crate::token::Span;
use logicaffeine_base::{Interner, Symbol};

/// Type expression for explicit type annotations.
///
/// Represents type syntax like:
/// - `Int` → Primitive(Int)
/// - `User` → Named(User)
/// - `List of Int` → Generic { base: List, params: [Primitive(Int)] }
/// - `List of List of Int` → Generic { base: List, params: [Generic { base: List, params: [Primitive(Int)] }] }
/// - `Result of Int and Text` → Generic { base: Result, params: [Primitive(Int), Primitive(Text)] }
#[derive(Debug, Clone)]
pub enum TypeExpr<'a> {
    /// Primitive type: Int, Nat, Text, Bool
    Primitive(Symbol),
    /// Named type (user-defined): User, Point
    Named(Symbol),
    /// Generic type: List of Int, Option of Text, Result of Int and Text
    Generic {
        base: Symbol,
        params: &'a [TypeExpr<'a>],
    },
    /// Anonymous tuple type: `(Int, Int)`, `(Int, Text, Bool)`. A positional heterogeneous product —
    /// the first-class form of a multiple-return / paired parameter. Resolves to `BoundaryType::Tuple`
    /// (the same cross-tier classification `Pair`/`Triple` already use) and rides one slot per tier.
    Tuple(&'a [TypeExpr<'a>]),
    /// Named record type: `(quotient: Int, remainder: Int)`. A tuple whose positions carry NAMES, so it is
    /// accessed by field (`r's quotient`) rather than position. Its VALUE is a self-describing struct
    /// (`RuntimeValue::Struct` with these field names), built by coercing the returned positional tuple —
    /// so it reuses the entire struct field-access + codegen machinery on every tier. Parallel slices
    /// (`field_names[i]` : `field_types[i]`) keep the arena allocators simple.
    Record {
        field_names: &'a [Symbol],
        field_types: &'a [TypeExpr<'a>],
    },
    /// Function type: fn(A, B) -> C (for higher-order functions)
    Function {
        inputs: &'a [TypeExpr<'a>],
        output: &'a TypeExpr<'a>,
    },
    /// Refinement type with predicate constraint.
    /// Example: `Int where it > 0`
    Refinement {
        /// The base type being refined
        base: &'a TypeExpr<'a>,
        /// The bound variable (usually "it")
        var: Symbol,
        /// The predicate constraint (from Logic Kernel)
        predicate: &'a LogicExpr<'a>,
    },
    /// Persistent storage wrapper type.
    /// Example: `Persistent Counter`
    /// Semantics: Wraps a Shared type with journal-backed storage
    Persistent {
        /// The inner type (must be a Shared/CRDT type)
        inner: &'a TypeExpr<'a>,
    },
    /// Mutable-parameter marker (Mutable Value Semantics escape hatch).
    /// Example: `## To addItem (items: mutable Seq of Int):` →
    /// `Mutable { inner: Generic { List, [Int] } }`.
    /// Semantics: under value semantics collections pass by value by default; a
    /// `Mutable` parameter passes by reference so the callee's mutations are
    /// visible to the caller (the explicit, opt-in form of the void-mutate
    /// idiom). Carried on the parameter's type so the `(Symbol, &TypeExpr)`
    /// parameter representation is unchanged. For every purpose other than the
    /// parameter-passing convention, `Mutable { inner }` behaves as `inner`.
    Mutable {
        /// The underlying parameter type.
        inner: &'a TypeExpr<'a>,
    },
}

/// Source for Read statements.
#[derive(Debug, Clone, Copy)]
pub enum ReadSource<'a> {
    /// Read from console (stdin)
    Console,
    /// Read from file at given path
    File(&'a Expr<'a>),
}

/// Pattern for loop variable binding.
/// Supports single identifiers and tuple destructuring for Map iteration.
#[derive(Debug, Clone)]
pub enum Pattern {
    /// Single identifier: `Repeat for x in list`
    Identifier(Symbol),
    /// Tuple destructuring: `Repeat for (k, v) in map`
    Tuple(Vec<Symbol>),
}

/// Binary operation kinds for imperative expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOpKind {
    Add,
    Subtract,
    Multiply,
    /// `a ** b` — exponentiation. Integer power is EXACT (promotes to BigInt on
    /// overflow); a Float operand uses `powf`; a negative integer exponent is a
    /// loud error. Binds tighter than `* / %`, right-associative.
    Pow,
    Divide,
    /// EXACT division — the type-directed sibling of [`BinaryOpKind::Divide`].
    /// `Divide` floors (`7 / 2 → 3`, the integer default); `ExactDivide` keeps the
    /// quotient exact (`7 / 2 → 7/2`, a `Rational`). The `resolve_divisions` pass
    /// rewrites `Divide → ExactDivide` only where the result flows into a `Rational`
    /// context, so existing (floor) code is untouched.
    ExactDivide,
    /// `a // b` — FLOOR division, rounding toward negative infinity (`-7 // 2 → -4`).
    /// Distinct from [`BinaryOpKind::Divide`], which truncates toward zero (`-7 / 2 →
    /// -3`); the two agree when the operands share a sign. Exact (promotes to BigInt),
    /// integer-producing, and immune to the `resolve_divisions` Rational rewrite — the
    /// explicit spelling for "give me the floored integer quotient."
    FloorDivide,
    Modulo,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    // Logical/bitwise operators — type-aware in codegen (&&/|| for Bool, &/| for Int)
    And,
    Or,
    /// String concatenation ("X combined with Y")
    Concat,
    /// Sequence concatenation ("A followed by B") — merge two sequences into one.
    SeqConcat,
    /// Tolerant float comparison ("a is approximately b") — Python-isclose
    /// semantics (rel 1e-9, abs floor 1e-12). `==` stays IEEE bit-exact;
    /// this is the EXPLICIT spelling for near-equality.
    ApproxEq,
    /// Bitwise XOR: `x ^ y` (the word `xor` is the English spelling); on
    /// Sets, symmetric difference.
    BitXor,
    /// Bitwise AND: `x & y`; on Sets, intersection.
    BitAnd,
    /// Bitwise OR: `x | y`; on Sets, union.
    BitOr,
    /// Left shift: "x shifted left by y" → `x << y`
    Shl,
    /// Right shift: "x shifted right by y" → `x >> y`
    Shr,
}

/// The HTTP method of a [`Stmt::Fetch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    /// `Upload <file> to URL as T, then action.` — a multipart body.
    Upload,
}

/// What a component action can do to the page, the browser, storage or a store. Each is a
/// sentence in the `Actions:` vocabulary; the selector forms take a selector TEXT or an
/// `Element`.
#[derive(Debug, Clone)]
pub enum UiStmt<'a> {
    /// `Focus SEL.` — put focus on the element AND raise the `focus` event, so a
    /// `When "#x" is focused:` handler runs. Setting focus without raising it left that handler
    /// unreachable from any test.
    Focus {
        selector: &'a Expr<'a>,
    },
    /// `Blur SEL.` — take focus off the element and raise `blur`.
    ///
    /// ★ ITS EVENT COULD BE WAITED FOR AND NOT CAUSED. `blurred` is one of the six predicates a
    /// handler may name, and nothing in the language could make one happen — so a page's
    /// leave-the-field path (validate on blur, save a draft, close a menu) was unreachable from
    /// every test in the suite.
    Blur {
        selector: &'a Expr<'a>,
    },
    /// `Hover SEL.` — raise `mouseover`.
    ///
    /// ★ IT WAS ALLOWLISTED WITH NOTHING BEHIND IT: `Hover` appeared in the gate that decides
    /// whether a sentence may open a UI statement, and `parse_ui_statement` had no branch for it,
    /// so the word was accepted into the gate and then fell out the bottom as "I expected a
    /// statement here". Two hand-maintained halves of one table, each complete on its own.
    Hover {
        selector: &'a Expr<'a>,
    },
    /// `Scroll to SEL.`
    ScrollTo {
        selector: &'a Expr<'a>,
    },
    /// `Add class C to SEL.`
    AddClass {
        class: &'a Expr<'a>,
        selector: &'a Expr<'a>,
    },
    /// `Remove class C from SEL.`
    RemoveClass {
        class: &'a Expr<'a>,
        selector: &'a Expr<'a>,
    },
    /// `Select the text in SEL.`
    SelectText {
        selector: &'a Expr<'a>,
    },
    /// `Copy TEXT to the clipboard.`
    CopyToClipboard {
        text: &'a Expr<'a>,
    },
    /// `Download BYTES as "name.ext".`
    Download {
        bytes: &'a Expr<'a>,
        name: &'a Expr<'a>,
    },
    /// `Navigate to URL.`
    Navigate {
        url: &'a Expr<'a>,
    },
    /// `Replace the URL with URL.` — history replace, no new entry.
    ReplaceUrl {
        url: &'a Expr<'a>,
    },
    /// `Go back.`
    /// `Raise "offline".` — cause a WINDOW event in a test.
    ///
    /// ★ ONE DRIVER FOR EVERY EVENT THAT HAPPENS TO THE PAGE RATHER THAN TO AN ELEMENT. Going
    /// offline, coming back online and the window resizing are not things a page does to itself
    /// and not things a visitor does to an element, so none of the element verbs can cause one —
    /// and a trigger no test can cause is a trigger nothing checks. Naming the event as TEXT means
    /// the next window event needs no new sentence.
    RaiseWindowEvent {
        event: &'a Expr<'a>,
        target: &'a Expr<'a>,
        value: &'a Expr<'a>,
    },

    GoBack,
    /// `Set the title to TEXT.`
    SetTitle {
        title: &'a Expr<'a>,
    },
    /// `Save VALUE as "key".`
    Save {
        value: &'a Expr<'a>,
        key: &'a Expr<'a>,
    },
    /// `Load "key" into FIELD.`
    Load {
        key: &'a Expr<'a>,
        var: Symbol,
    },
    /// `Forget "key".`
    Forget {
        key: &'a Expr<'a>,
    },
    /// `Reload the page.` — every component instance, listener and mounted root goes; browser
    /// storage stays. What a `persisted` store survives and a local one does not.
    Reload,
    /// `Notify MESSAGE [as success|error].`
    Notify {
        message: &'a Expr<'a>,
        tone: Option<Symbol>,
    },
    /// `Publish MESSAGE on TOPIC.` — send on a relay topic.
    ///
    /// ★ THE OUTBOUND HALF OF A CONSTRUCT THAT SHIPPED WITH ONLY ITS INBOUND HALF.
    /// `When a message arrives on "t":` subscribes, and nothing in the language could publish —
    /// so a page could hear a server and never answer it, and two tabs of one app could not talk.
    /// The relay had the other side complete the whole time (`RelayFrame::Publish` fans out to
    /// every local subscriber and gossips on to native nodes); what was missing was a sentence.
    ///
    /// It delivers to THIS page's own subscribers as well as sending the frame, so a publish
    /// means one thing wherever the page runs: every handler on the topic hears it exactly once.
    /// The relay excludes the originating connection, so the local delivery is not a second copy.
    Publish {
        message: &'a Expr<'a>,
        topic: &'a Expr<'a>,
    },
    /// `Toggle FIELD.` — a Bool state field.
    Toggle {
        field: Symbol,
    },
    /// `Call <Store>'s <action> [with a and b].`
    CallStore {
        store: Symbol,
        action: Symbol,
        args: Vec<&'a Expr<'a>>,
    },
    /// `Stop the timer NAME.`
    StopTimer {
        name: Symbol,
    },
    /// `Start the timer NAME.` — (re)start a named timer from a whole period, its period read from
    /// its sentence now; a running timer restarts, a stopped one runs again.
    StartTimer {
        name: Symbol,
    },
    /// `Click SEL.` — fire a click on the element the selector names.
    Click {
        selector: &'a Expr<'a>,
    },
    /// `Submit SEL.` — submit a form, as a user pressing the button would.
    Submit {
        selector: &'a Expr<'a>,
    },
    /// `Choose VALUE in SEL.` — pick an option in a `<select>`.
    ///
    /// Its own verb because a dropdown is neither typed into nor clicked; without one a test can
    /// only reach a `<select>` by faking the event some other control raises.
    Choose {
        value: &'a Expr<'a>,
        selector: &'a Expr<'a>,
    },
    /// `Press KEY [in SEL].` — a key press, on an element or on the page.
    Press {
        key: &'a Expr<'a>,
        selector: Option<&'a Expr<'a>>,
    },
    /// `Type TEXT into SEL.` — type into a bound input, as a user would.
    TypeInto {
        text: &'a Expr<'a>,
        selector: &'a Expr<'a>,
    },
    /// `Mount <Component> [at "#sel"].` — create the component's root instance and render it.
    /// (The legacy indentation dialect keeps its own synthesized-`render` lowering.)
    Mount {
        component: Symbol,
        selector: Option<&'a Expr<'a>>,
    },
}

impl<'a> UiStmt<'a> {
    /// Every expression this statement evaluates — the traversal surface for the analyses.
    pub fn exprs(&self) -> Vec<&'a Expr<'a>> {
        match self {
            UiStmt::Focus { selector }
            | UiStmt::Blur { selector }
            | UiStmt::Hover { selector }
            | UiStmt::ScrollTo { selector }
            | UiStmt::SelectText { selector } => vec![selector],
            UiStmt::AddClass { class, selector } | UiStmt::RemoveClass { class, selector } => {
                vec![class, selector]
            }
            UiStmt::CopyToClipboard { text } => vec![text],
            UiStmt::Download { bytes, name } => vec![bytes, name],
            UiStmt::Navigate { url } | UiStmt::ReplaceUrl { url } => vec![url],
            UiStmt::SetTitle { title } => vec![title],
            UiStmt::RaiseWindowEvent {
                event,
                target,
                value,
            } => vec![event, target, value],
            UiStmt::Save { value, key } => vec![value, key],
            UiStmt::Load { key, .. } => vec![key],
            UiStmt::Forget { key } => vec![key],
            UiStmt::Notify { message, .. } => vec![message],
            UiStmt::Publish { message, topic } => vec![message, topic],
            UiStmt::CallStore { args, .. } => args.clone(),
            UiStmt::Click { selector } | UiStmt::Submit { selector } => vec![selector],
            UiStmt::Choose { value, selector } => vec![value, selector],
            UiStmt::Press { key, selector } => {
                let mut v = vec![*key];
                v.extend(selector.iter().copied());
                v
            }
            UiStmt::TypeInto { text, selector } => vec![text, selector],
            UiStmt::Mount { selector, .. } => selector.iter().copied().collect(),
            UiStmt::GoBack
            | UiStmt::Reload
            | UiStmt::Toggle { .. }
            | UiStmt::StopTimer { .. }
            | UiStmt::StartTimer { .. } => Vec::new(),
        }
    }

    /// The binding this statement WRITES, if any (`Load … into x`, `Toggle done`).
    pub fn writes(&self) -> Option<Symbol> {
        match self {
            UiStmt::Load { var, .. } => Some(*var),
            UiStmt::Toggle { field } => Some(*field),
            _ => None,
        }
    }

    /// Where this statement may be written: driving a page from a `## Main`/`## Test` body,
    /// inside a component's `Actions:`, or either.
    pub fn role(&self) -> Role {
        match self {
            // Drive-only: `action_verb_tier_lock`'s `NOT_IN_ACTIONS` proves none of these compile
            // inside `Actions:`, and `page_host::Drive` carries each to a page under test.
            UiStmt::Mount { .. }
            | UiStmt::Click { .. }
            | UiStmt::Submit { .. }
            | UiStmt::Choose { .. }
            | UiStmt::Press { .. }
            | UiStmt::TypeInto { .. }
            | UiStmt::RaiseWindowEvent { .. } => Role::Drive,
            // Both: `action_verb_tier_lock`'s `VERBS` proves each compiles inside `Actions:`, and
            // `page_host::Drive` also carries each to a page under test.
            UiStmt::Focus { .. }
            | UiStmt::Blur { .. }
            | UiStmt::Hover { .. }
            | UiStmt::Navigate { .. }
            | UiStmt::Reload => Role::Both,
            // Action-only: `action_verb_tier_lock`'s `VERBS` proves each compiles inside
            // `Actions:`, and none is a variant of `page_host::Drive`.
            UiStmt::ScrollTo { .. }
            | UiStmt::AddClass { .. }
            | UiStmt::RemoveClass { .. }
            | UiStmt::SelectText { .. }
            | UiStmt::CopyToClipboard { .. }
            | UiStmt::Download { .. }
            | UiStmt::ReplaceUrl { .. }
            | UiStmt::GoBack
            | UiStmt::SetTitle { .. }
            | UiStmt::Save { .. }
            | UiStmt::Load { .. }
            | UiStmt::Forget { .. }
            | UiStmt::Notify { .. }
            | UiStmt::Publish { .. }
            | UiStmt::Toggle { .. }
            | UiStmt::CallStore { .. }
            | UiStmt::StopTimer { .. }
            | UiStmt::StartTimer { .. } => Role::Action,
        }
    }
}

/// Which position a `UiStmt` may be written in — the partition [`UiStmt::role`] holds to, cross-
/// checked against `action_verb_tier_lock`'s own census (which verbs compile inside `Actions:`)
/// and `page_host::Drive` (which verbs carry a `## Main`/`## Test` statement to a page under
/// test), so the two catalogues cannot silently drift apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A `## Main`/`## Test` driving statement only — never legal inside a component's `Actions:`.
    Drive,
    /// Only ever written inside a component's `Actions:` — never a driving statement.
    Action,
    /// Legal in either position.
    Both,
}

/// Block is a sequence of statements.
pub type Block<'a> = &'a [Stmt<'a>];

/// A statement's identity: where it lives. Stable for as long as the AST it
/// was taken from is alive; a statement copied into a new block is a new
/// identity. Diagnostics use it to name the exact statement a finding is
/// about, nested ones included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StmtId(usize);

impl StmtId {
    pub fn of(stmt: &Stmt<'_>) -> Self {
        StmtId(stmt as *const Stmt<'_> as usize)
    }
}

/// Match arm for pattern matching in Inspect statements.
#[derive(Debug, Clone)]
pub struct MatchArm<'a> {
    pub enum_name: Option<Symbol>,       // The enum type (e.g., Shape)
    pub variant: Option<Symbol>,         // None = Otherwise (wildcard)
    pub bindings: Vec<(Symbol, Symbol)>, // (field_name, binding_name)
    pub body: Block<'a>,
}

/// Which sealed-world resources a `## Test` may really touch, granted by
/// capability sentences (`This test may use the real filesystem.`). Everything
/// defaults to sealed; a granted capability moves the test to the `--live` set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TestCapabilities {
    pub filesystem: bool,
    pub network: bool,
    pub environment: bool,
    pub clock: bool,
}

/// A table-driven test's case fixture: `For each of these cases:` followed by
/// a markdown pipe table. Columns bind names in the test body's scope; each
/// data row is a named sub-test.
#[derive(Debug, Clone)]
pub struct CaseTable<'a> {
    pub columns: Vec<Symbol>,
    pub rows: Vec<Vec<&'a Expr<'a>>>,
}

/// Where an `Expect output:` snapshot lives: indented in the document, or in
/// an external golden file (`Expect output as in "golden/report.txt".`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedOutput {
    Inline(Symbol),
    File(Symbol),
}

/// The domain a property quantifies over (`For all integers a and b:`).
/// Generation draws from this; shrinking minimizes toward the domain's zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropDomain {
    Integers,
    Naturals,
    Booleans,
}

/// The speech-act category of an observation statement:
/// audience + retention, not a
/// numeric level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObsCategory {
    /// `Note that …` — for the developer replaying the run; ring-buffered,
    /// elidable in release.
    Note,
    /// `Warn that …` — for the operator; never dropped, never sampled.
    Warn,
    /// `Record that …` — the audit/business trail; durable.
    Record,
}

/// The wire compression codec a `Send compressed [with <codec>]` selects. The
/// transpiler maps this to the runtime's wire codec; bare `compressed` = `Deflate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompressionCodec {
    /// DEFLATE — the balanced default (bare `Send compressed`).
    Deflate,
    /// LZ4 — near-memcpy speed, lighter ratio.
    Lz4,
    /// Zstandard — the best ratio.
    Zstd,
}

/// The wire LAYOUT a `Send` modifier picks — the size↔speed dial the sender chooses for
/// their link. The transpiler maps this to the runtime's numeric codec. The sender knows
/// their use case; this lets them express it in one word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SendLayout {
    /// `compact` / `small` — smallest wire (LEB128 varint). For a bandwidth-bound link
    /// (mobile, WAN, metered). This is also the default when no layout word is given.
    Compact,
    /// `fast` / `quickly` — fastest decode (fixed-width memcpy, zero parse). For a
    /// latency-bound / fat link (LAN, datacenter, RDMA).
    Fast,
    /// `packed` — varint size with SIMD group-varint decode; the balanced middle.
    Packed,
    /// `smallest` / `best` — turn on the per-column compression menu (delta /
    /// delta-of-delta / frame-of-reference / run-length / dictionary), auto-selecting
    /// each column's smallest form and never exceeding plain varint. For a
    /// bandwidth-bound link where CPU is cheap relative to bytes.
    Smallest,
    /// `redundant` / `tough` — forward error correction: the message is split into
    /// Reed-Solomon shards and each is published as its own packet, so a receiver
    /// reconstructs the exact message from any K even after some are lost. For a lossy
    /// / one-way link (UDP, multicast, BLE, LoRa) where retransmit is impossible.
    Redundant,
}

/// Backing-file source for a memory-mapped zone (`… mapped from <here>`).
/// `Inside a zone called "D" mapped from "f.bin"` is [`ZoneSource::Literal`];
/// `… mapped from path` (a runtime `Text` variable) is [`ZoneSource::Variable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneSource {
    /// A string-literal path baked into the source.
    Literal(Symbol),
    /// A variable holding the path, resolved at runtime.
    Variable(Symbol),
}

/// Imperative statement AST (LOGOS §15.0.0).
///
/// Stmt is the primary AST node for imperative code blocks like `## Main`
/// and function bodies. The Assert variant bridges to the Logic Kernel.
/// Which end of the shared pad this peer draws from, for the PNP one-time-pad tier.
/// `as initiator` sends on the first directional half; `as responder` sends on the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecureRole {
    /// Draws the send pad from the first directional half (`i2r`).
    Initiator,
    /// Draws the send pad from the second directional half (`r2i`).
    Responder,
}

/// The `with pad "<path>" as <role>` binding on a `Connect`/`Listen` — activate the PNP one-time-pad
/// session over the pad file at `pad`, with this peer taking `role`.
#[derive(Debug, Clone, Copy)]
pub struct SecurePad<'a> {
    /// The pad file path (a string-literal or variable expression).
    pub pad: &'a Expr<'a>,
    /// Which directional half this peer sends on.
    pub role: SecureRole,
}

#[derive(Debug, Clone)]
pub enum Stmt<'a> {
    /// Variable binding: `Let x be 5.` or `Let x: Int be 5.`
    Let {
        var: Symbol,
        ty: Option<&'a TypeExpr<'a>>,
        value: &'a Expr<'a>,
        mutable: bool,
    },

    /// Mutation: `Set x to 10.`
    Set { target: Symbol, value: &'a Expr<'a> },

    /// Reversible register exchange: `Swap x and y.` — the canonical ZERO-GARBAGE reversible
    /// primitive (`Op::RevSwap`, its own inverse), the first LOGOS surface that emits the abelian
    /// reversible ISA. Exchanges the two variables' values with no history-tape cost.
    Swap { a: Symbol, b: Symbol },

    /// Function call as statement: `Call process with data.`
    Call {
        function: Symbol,
        args: Vec<&'a Expr<'a>>,
    },

    /// Conditional: `If condition: ... Otherwise: ...`
    If {
        cond: &'a Expr<'a>,
        then_block: Block<'a>,
        else_block: Option<Block<'a>>,
    },

    /// Loop: `While condition: ...` or `While condition (decreasing expr): ...`
    While {
        cond: &'a Expr<'a>,
        body: Block<'a>,
        /// Optional decreasing variant for termination proof.
        decreasing: Option<&'a Expr<'a>>,
    },

    /// Iteration: `Repeat for x in list: ...` or `Repeat for i from 1 to 10: ...`
    Repeat {
        pattern: Pattern, // Changed from `var: Symbol` to support tuple destructuring
        iterable: &'a Expr<'a>,
        body: Block<'a>,
    },

    /// Return: `Return x.` or `Return.`
    Return { value: Option<&'a Expr<'a>> },

    /// Break: `Break.` — exits the innermost while loop.
    Break,

    /// Bridge to Logic Kernel: `Assert that P.`
    Assert { proposition: &'a LogicExpr<'a> },

    /// Documented assertion with justification.
    /// `Trust that P because "reason".`
    /// Semantics: Documented runtime check that could be verified statically.
    Trust {
        proposition: &'a LogicExpr<'a>,
        justification: Symbol,
    },

    /// Runtime assertion with imperative condition.
    /// `Assert that condition.` (`hard: false` → `debug_assert!`, a development check)
    /// `Require that condition.` (`hard: true` → `assert!`, an enforced invariant that
    /// survives release — the form a proven property lowers to).
    /// `dim` is the physical dimension of a range contract whose literal quantity bounds the
    /// parser folded to SI magnitudes (`Assume v is between 0.2 volts and 0.7 volts.`) —
    /// metadata for static consumers (the analog lane); `None` for ordinary conditions.
    /// `reason` is the `, otherwise <text>` clause: why the contract exists, reported verbatim when
    /// it breaks and evaluated ONLY then. `None` reports `Assertion failed`.
    RuntimeAssert {
        condition: &'a Expr<'a>,
        hard: bool,
        dim: Option<logicaffeine_base::dimension::Dimension>,
        reason: Option<&'a Expr<'a>>,
    },

    /// `Fail with <reason>.` — raise an error. With no `Result` to return it into and no `Try:`
    /// region around it, the program stops with `reason` as its report.
    Fail { reason: &'a Expr<'a> },

    /// Ownership transfer (move): `Give x to processor.`
    /// Semantics: Move ownership of `object` to `recipient`.
    Give {
        object: &'a Expr<'a>,
        recipient: &'a Expr<'a>,
    },

    /// Immutable borrow: `Show x to console.`
    /// Semantics: Immutable borrow of `object` passed to `recipient`.
    Show {
        object: &'a Expr<'a>,
        recipient: &'a Expr<'a>,
    },

    /// Field mutation: `Set p's x to 10.`
    SetField {
        object: &'a Expr<'a>,
        field: Symbol,
        value: &'a Expr<'a>,
    },

    /// Struct definition for codegen.
    StructDef {
        name: Symbol,
        fields: Vec<(Symbol, Symbol, bool)>, // (name, type_name, is_public)
        is_portable: bool,                   // Derives Serialize/Deserialize
    },

    /// Function definition.
    FunctionDef {
        name: Symbol,
        /// Generic type parameters: empty for monomorphic functions, e.g. `[T, U]` for polymorphic.
        generics: Vec<Symbol>,
        params: Vec<(Symbol, &'a TypeExpr<'a>)>,
        body: Block<'a>,
        return_type: Option<&'a TypeExpr<'a>>,
        is_native: bool,
        /// Rust path for user-defined native functions (e.g., "reqwest::blocking::get").
        /// None for system native functions (read, write, etc.) which use map_native_function().
        native_path: Option<Symbol>,
        /// Whether this function is exported for FFI (C ABI or WASM).
        is_exported: bool,
        /// Export target: None = C ABI (#\[no_mangle\] extern "C"), Some("wasm") = #\[wasm_bindgen\].
        export_target: Option<Symbol>,
        /// Per-function optimization config: each `## No <X>` annotation clears
        /// that optimization's bit (default: all enabled).
        opt_flags: crate::optimization::OptimizationConfig,
    },

    /// A `## Test: <prose sentence>` block — an independent test entry point.
    /// The prose name is the
    /// test's identity everywhere; duplicates in a module are a parse error.
    /// Ignored by normal execution; collected by the `largo test` runner.
    TestDef {
        /// The prose sentence after the colon, whitespace-normalized.
        name: Symbol,
        /// `Skip this test because "<reason>".` — the mandatory reason.
        skip: Option<Symbol>,
        /// Capability sentences granting real-world access (sealed by default).
        capabilities: TestCapabilities,
        /// `This test may run up to N steps.` — op-budget escalation.
        budget: Option<u64>,
        /// `For each of these cases:` + pipe table, when present.
        cases: Option<CaseTable<'a>>,
        body: Block<'a>,
    },

    /// Test-only assertion: `Expect that <condition>.` — records the failure
    /// and CONTINUES (a test reports every failed expectation, not just the
    /// first). Refused outside `## Test`/`## Example` blocks.
    Expect { condition: &'a Expr<'a> },

    /// `Expect this to fail [with "<message>"]:` — the only error-expectation
    /// form; the test FAILS if the block succeeds.
    ExpectFail {
        message: Option<Symbol>,
        body: Block<'a>,
    },

    /// `Expect output:` — byte-exact stdout snapshot (in-document or golden file).
    ExpectOutput { expected: ExpectedOutput },

    /// `Advance time by <duration>.` — moves the sealed world's virtual clock
    /// (test-only; the statement that makes every timeout test instant).
    AdvanceTime {
        /// The advance in milliseconds.
        millis: &'a Expr<'a>,
    },

    /// `Given a file "<path>" containing "<content>".` — pre-seeds the sealed
    /// world's in-memory filesystem (test-only).
    GivenFile { path: Symbol, content: Symbol },

    /// An observation statement: `Note/Warn/Record that <clause> [with <p> and
    /// <p>]`. The that-clause is
    /// captured as the rendered `message`; payloads are the named values that
    /// travel with the event. Legal anywhere; emission is Wave 7.
    Observe {
        category: ObsCategory,
        /// The that-clause, rendered to text (the event's stable message).
        message: Symbol,
        /// Prepositional payloads (`with X and Y`) — name → value expression.
        payloads: Vec<(Symbol, &'a Expr<'a>)>,
        /// Deterministic counter-based sampling (`Note once in every N times
        /// that …`): emit 1-in-N. `None` = always. Sampling is a rendering
        /// decision, never an evidence decision (replay regenerates the rest).
        sample: Option<u64>,
    },

    /// `During <gerund>:` — a lexical trace span (no imperative start/stop, so
    /// a span can never leak). The body is ordinary code.
    During {
        /// The span's name (the gerund phrase or quoted literal), rendered.
        label: Symbol,
        body: Block<'a>,
    },

    /// `For all <domain> <vars> [where <cond>]:` — a property test.
    /// The runner generates seeded
    /// cases, binds the vars, runs the body, and shrinks any counterexample.
    /// Test-only (like `Expect`).
    ForAll {
        vars: Vec<Symbol>,
        domain: PropDomain,
        /// Optional refinement filter (`where n is greater than 0`).
        constraint: Option<&'a Expr<'a>>,
        body: Block<'a>,
    },

    /// Pattern matching on sum types.
    Inspect {
        target: &'a Expr<'a>,
        arms: Vec<MatchArm<'a>>,
        has_otherwise: bool, // For exhaustiveness tracking
    },

    /// Push to collection: `Push x to items.`
    Push {
        value: &'a Expr<'a>,
        collection: &'a Expr<'a>,
    },

    /// Pop from collection: `Pop from items.` or `Pop from items into y.`
    Pop {
        collection: &'a Expr<'a>,
        into: Option<Symbol>,
    },

    /// Add to set: `Add x to set.`
    Add {
        value: &'a Expr<'a>,
        collection: &'a Expr<'a>,
    },

    /// Remove from set: `Remove x from set.`
    Remove {
        value: &'a Expr<'a>,
        collection: &'a Expr<'a>,
    },

    /// Index assignment: `Set item N of X to Y.`
    SetIndex {
        collection: &'a Expr<'a>,
        index: &'a Expr<'a>,
        value: &'a Expr<'a>,
    },

    /// A SCOPE-TRANSPARENT statement sequence — parser-desugar output, never
    /// written by users. One surface statement can lower to several primitive
    /// statements (a nested place-write `Set item j of (item i of grid) to v`
    /// becomes read → copy-on-write element write → write-back; a multi-push
    /// `Push a, b, c to xs` becomes one Push per element). The body runs in
    /// the ENCLOSING scope with no block of its own: temporaries inside are
    /// gensym'd (`__place_*`), so they can never collide with user names, and
    /// engines that execute blocks with scoping must NOT scope this one.
    Splice { body: Block<'a> },

    /// A REVERSIBLE / UNCOMPUTE block: `Reversibly:` (alias `Uncompute:`) introducing an
    /// indented body. Semantically TRANSPARENT — the body runs in the enclosing scope and
    /// produces byte-identical OUTPUT to the same statements outside the block. The only
    /// difference is the LOWERING: an IN-PLACE ABELIAN scalar update inside the body compiles
    /// to the zero-garbage reversible ISA (`Op::RevAdd`/`RevSub`/`RevXor`/`RevNeg`) instead of
    /// the plain journaling op, so `Vm::run_reverse` can undo the whole region with NO history
    /// tape (the adiabatic charge-recovery contract). Recognized in-place forms — where `x` is a
    /// local and `e` provably never observes `x`:
    /// `Set x to x + e` → `RevAdd`, `Set x to x - e` → `RevSub`,
    /// `Set x to x xor e` → `RevXor`, `Set x to -x` → `RevNeg`.
    /// Anything else in the body lowers exactly as it would outside the block.
    Reversibly { body: Block<'a> },

    /// Memory arena block (Zone).
    /// "Inside a new zone called 'Scratch':"
    /// "Inside a zone called 'Buffer' of size 1 MB:"
    /// "Inside a zone called 'Data' mapped from 'file.bin':"
    Zone {
        /// The variable name for the arena handle (e.g., "Scratch")
        name: Symbol,
        /// Optional pre-allocated capacity in bytes (Heap zones only)
        capacity: Option<usize>,
        /// Optional backing file for memory-mapped zones (literal path or runtime variable)
        source_file: Option<ZoneSource>,
        /// The code block executed within this memory context
        body: Block<'a>,
    },

    /// Concurrent execution block (async, I/O-bound).
    /// "Attempt all of the following:"
    /// Semantics: All tasks run concurrently via tokio::join!
    /// Best for: network requests, file I/O, waiting operations
    Concurrent {
        /// The statements to execute concurrently
        tasks: Block<'a>,
    },

    /// Parallel execution block (CPU-bound).
    /// "Simultaneously:"
    /// Semantics: True parallelism via rayon::join or thread::spawn
    /// Best for: computation, data processing, number crunching
    Parallel {
        /// The statements to execute in parallel
        tasks: Block<'a>,
    },

    /// Read from console or file.
    /// `Read input from the console.` or `Read data from file "path.txt".`
    ReadFrom { var: Symbol, source: ReadSource<'a> },

    /// Write to file.
    /// `Write "content" to file "output.txt".`
    WriteFile {
        content: &'a Expr<'a>,
        path: &'a Expr<'a>,
    },

    /// Spawn an agent.
    /// `Spawn a Worker called "w1".`
    Spawn { agent_type: Symbol, name: Symbol },

    /// Send message to agent.
    /// `Send Ping to "agent".`, `Send compressed Ping to "agent".`,
    /// `Send cached Point to "agent".`, or `Send cached compressed Report to "agent".`
    SendMessage {
        message: &'a Expr<'a>,
        destination: &'a Expr<'a>,
        /// The wire compression codec. `None` for a plain `Send`; `Some(codec)` for
        /// `Send compressed [with <codec>]` (bare `compressed` = deflate). Kept only
        /// if it actually shrinks the body.
        compression: Option<CompressionCodec>,
        /// `Send cached …` — use the connection's schema dictionary, so a struct
        /// schema is transmitted once and referenced thereafter (content-addressed,
        /// footgun-free). `false` for a plain `Send`.
        cached: bool,
        /// `Send unchecked …` — drop the wire integrity checksum for the fastest path
        /// (latency↔safety dial). `false` keeps the default checksum.
        unchecked: bool,
        /// `Send fast|compact|packed …` — the wire LAYOUT (size↔speed dial). `None` is
        /// the default (compact / varint). The sender picks for their link.
        layout: Option<SendLayout>,
        /// `Send shared …` — OPT-IN type-id elision: drop struct/enum NAMES off the wire
        /// (ship a small registry id both ends derive from their shared program type
        /// defs). Only safe when the receiver runs the same program, so it is OFF by
        /// default — the default `Send` stays self-describing for any peer / relay.
        shared: bool,
        /// `Send computed f …` — COMPUTE-SHIPPING: when the message is a pure single-arg
        /// function, lower it to a sandboxed generator and ship the COMPUTATION, not data.
        /// The receiver evaluates it in the bounded sandbox (never arbitrary code). OFF by
        /// default; a non-lowerable function under `computed` is rejected at send.
        computed: bool,
        /// `Send indexed …` (alias `addressable`) — encode a record list in the random-access
        /// struct-view LAYOUT (row + field offset tables), so the receiver reaches any
        /// (row, field) in O(1) without decoding the rest — Cap'n Proto's home turf. Composes
        /// with the other knobs (`compressed`, `cached`, `shared`, `unchecked`). OFF by default
        /// (the dense columnar form is smaller); opt in when the peer does random field reads.
        indexed: bool,
        /// `Send deduped …` — Rc-DEDUP: a subtree the same value reaches more than once ships ONCE
        /// (the first occurrence) plus a tiny backref for every repeat, and the receiver rebuilds the
        /// SHARING (one aliased value, not N copies). OFF by default; opt in when the message has
        /// shared sub-structure (a lookup table referenced by many records, one object aliased
        /// across the payload). Self-describing by tag, so any peer decodes it.
        deduped: bool,
        /// `Send unreliably …` — the transport RELIABILITY dial: an unordered, best-effort
        /// datagram (may reorder or drop) instead of the reliable, ordered default. The media
        /// path pairs it with FEC so lossy frames recover without a retransmit stall. `false`
        /// is the default (reliable, ordered).
        unreliable: bool,
    },

    /// Await response from agent.
    /// `Await response from "agent" into result.`
    AwaitMessage {
        source: &'a Expr<'a>,
        into: Symbol,
        /// `Await view from …`: hold a received record-list LAZILY (the zero-copy receive —
        /// decode-on-touch, no rows materialized until a field is read) instead of fully decoding
        /// it. Ignored for non-record-list shapes, which always decode eagerly.
        view: bool,
        /// `Await stream from …`: receive a batch STREAM message and deframe it into a list (the
        /// values a peer `Stream`ed), rather than a single message.
        stream: bool,
    },

    /// Stream a batch of values to a peer in one framed message.
    /// `Stream readings to "sink".` — frames each element of `values` length-delimited so the
    /// receiver (`Await stream from …`) deframes them incrementally; one relay publish ships the
    /// whole batch (Kafka-style streaming that amortizes per-message overhead).
    StreamMessage {
        values: &'a Expr<'a>,
        destination: &'a Expr<'a>,
    },

    /// Merge CRDT state.
    /// `Merge remote into local.` or `Merge remote's field into local's field.`
    MergeCrdt {
        source: &'a Expr<'a>,
        target: &'a Expr<'a>,
    },

    /// Converge CRDT state by the IDEMPOTENT lattice join — the sibling of [`Stmt::MergeCrdt`].
    /// `Converge remote into local.` A bare-`Int` field takes per-field MAX (not ADD), so applying a
    /// redelivered state twice equals applying it once; counter/other fields defer to the same rule as
    /// `Merge` (already idempotent / last-writer-wins).
    JoinCrdt {
        source: &'a Expr<'a>,
        target: &'a Expr<'a>,
    },

    /// Increment GCounter.
    /// `Increase local's points by 10.`
    IncreaseCrdt {
        object: &'a Expr<'a>,
        field: Symbol,
        amount: &'a Expr<'a>,
    },

    /// Decrement PNCounter (Tally).
    /// `Decrease game's score by 5.`
    DecreaseCrdt {
        object: &'a Expr<'a>,
        field: Symbol,
        amount: &'a Expr<'a>,
    },

    /// Append to SharedSequence (RGA).
    /// `Append "Hello" to doc's lines.`
    AppendToSequence {
        sequence: &'a Expr<'a>,
        value: &'a Expr<'a>,
    },

    /// Resolve MVRegister conflicts.
    /// `Resolve page's title to "Final".`
    ResolveConflict {
        object: &'a Expr<'a>,
        field: Symbol,
        value: &'a Expr<'a>,
    },

    /// Security check - mandatory runtime guard.
    /// `Check that user is admin.`
    /// `Check that user can publish the document.`
    /// Semantics: NEVER optimized out. Panics if condition is false.
    Check {
        /// The subject being checked (e.g., "user")
        subject: Symbol,
        /// The predicate name (e.g., "admin") or action (e.g., "publish")
        predicate: Symbol,
        /// True if this is a capability check (`can [action]`)
        is_capability: bool,
        /// For capabilities: the object being acted on (e.g., "document")
        object: Option<Symbol>,
        /// Original English text for error message
        source_text: String,
        /// Source location for error reporting
        span: crate::token::Span,
    },

    /// Listen on network address.
    /// `Listen on "/ip4/127.0.0.1/tcp/8000".`
    /// Semantics: Bind to address, start accepting connections via libp2p
    Listen {
        address: &'a Expr<'a>,
        /// Optional PNP one-time-pad binding: `with pad "<path>" as initiator|responder`.
        secure: Option<SecurePad<'a>>,
    },

    /// Serve HTTP requests on a port with a handler function.
    /// `Serve on port 8080 with handleRequest.`
    /// Semantics: bind an HTTP server on the port and, per request, invoke the
    /// handler (a `Request -> Response` function) and write back its response.
    Serve {
        port: &'a Expr<'a>,
        handler: &'a Expr<'a>,
    },

    /// Connect to remote peer.
    /// `Connect to "/ip4/127.0.0.1/tcp/8000".`
    /// Semantics: Dial peer via libp2p
    ConnectTo {
        address: &'a Expr<'a>,
        /// Optional PNP one-time-pad binding: `with pad "<path>" as initiator|responder`.
        secure: Option<SecurePad<'a>>,
    },

    /// Create PeerAgent remote handle.
    /// `Let remote be a PeerAgent at "/ip4/127.0.0.1/tcp/8000".`
    /// Semantics: Create handle for remote agent communication
    LetPeerAgent { var: Symbol, address: &'a Expr<'a> },

    /// Sleep for milliseconds.
    /// `Sleep 1000.` or `Sleep delay.`
    /// Semantics: Pause execution for N milliseconds (async)
    Sleep { milliseconds: &'a Expr<'a> },

    /// Sync CRDT variable on topic.
    /// `Sync x on "topic".`
    /// Semantics: Subscribe to GossipSub topic, auto-publish on mutation, auto-merge on receive
    Sync { var: Symbol, topic: &'a Expr<'a> },

    /// One of the English things a component action DOES: an element verb, navigation,
    /// persistence, feedback, a store call, or a timer ([`UiStmt`]). First-class: every tier
    /// implements it natively rather than receiving a desugared builtin call.
    Ui(UiStmt<'a>),

    /// A typed request: `Get URL as T, then action.` / `Post <body> to URL as T, then action.`
    /// The completion action receives `Result of T and HttpError` — the decode is derived from
    /// `as_type`, and a non-2xx status becomes the `Err` side. No implicit retry.
    Fetch {
        method: FetchMethod,
        url: &'a Expr<'a>,
        /// The request body (`Post <body> to …`); `None` for `Get` / `Delete`.
        body: Option<&'a Expr<'a>>,
        as_type: &'a TypeExpr<'a>,
        /// The action the response is delivered to.
        then: Symbol,
    },

    /// `Replay these actions: a, b, c.` — re-run a sequence of the page's actions, in order.
    ///
    /// The journal's reason for existing: a bug report becomes a test by naming the actions that
    /// produced it, and the dev panel's time travel is this with a slider on it. Each one is a
    /// REAL TURN — it rolls back if it traps, it is journalled, and the page repaints — because a
    /// replay that bypassed the turn machinery would reproduce a different bug from the one being
    /// investigated.
    Replay { actions: Vec<Symbol> },

    /// `Serve fetches with routes.` — answer the page's requests from the program's OWN routes.
    ///
    /// The full-stack loop with no network: a `Get "/api/todo"` is answered by the matching
    /// `## Route`, through the derived encoder on the way out and the derived decoder on the way
    /// back. That round trip is the only thing that can catch the two halves of the wire
    /// contract drifting apart — a stub answers with whatever the test author typed, which
    /// agrees with itself no matter what the encoder does.
    ///
    /// A DECISION a test makes, never a default: a page whose fetches were silently answered by
    /// the local route table would pass tests that a deployed page fails, because in a browser
    /// nothing local answers anything.
    ServeFetches,

    /// `Given that METHOD "/url" answers STATUS with BODY.` — how a test answers a fetch.
    ///
    /// The sealed-world rule the rest of the suite follows: a page test must not depend on a
    /// network, and a page whose data never arrives is a page that proves nothing. Matched by
    /// method AND url together, because a stub that answered the wrong request is how a test
    /// ends up proving something nobody asked for.
    GivenFetch {
        method: Symbol,
        url: &'a Expr<'a>,
        status: &'a Expr<'a>,
        body: &'a Expr<'a>,
    },

    /// An LGX page / component / store / error page — the component model, first-class
    /// (see [`crate::ast::component`]). Every tier implements it natively.
    ComponentDef(Box<crate::ast::component::ComponentDef<'a>>),

    /// `## Directive name (el: Element) and (value: T):` — a custom attribute behavior:
    /// `body` runs when the attribute appears on an element and whenever its value changes;
    /// `removed` (a `When removed:` block) runs when the element leaves.
    DirectiveDef {
        name: Symbol,
        params: Vec<(Symbol, &'a TypeExpr<'a>)>,
        body: Block<'a>,
        removed: Option<Block<'a>>,
    },

    /// `## Route METHOD /path (params) -> T:` — a typed HTTP route.
    RouteDef {
        method: Symbol,
        path: Symbol,
        params: Vec<(Symbol, &'a TypeExpr<'a>)>,
        return_type: Option<&'a TypeExpr<'a>>,
        /// ★ BOXED, AND THE REASON IS THE WHOLE NODE'S SIZE. `RouteBody` is 88 bytes, which made
        /// `RouteDef` 144 — the single widest `Stmt` variant, and therefore the size of EVERY `Stmt`.
        /// `compiler_stack_safety_lock::ast_nodes_stay_small` holds the node to 128 bytes because the
        /// compiler walk keeps many per frame, so widening it multiplies stack use across every tier.
        /// A route's body is the rare payload and does not belong inline; `ComponentDef` next door is
        /// boxed for the same reason.
        body: Box<crate::ast::component::RouteBody<'a>>,
        /// Where the route was declared. A `## Route` is refused for reasons the author has to
        /// go and look at — a parameter type the path matcher cannot bind, an answer type the
        /// derived codecs do not cover — and those refusals had nowhere to point.
        span: Span,
    },

    /// `## Before each request (request: Request) -> Option of Response:` — middleware.
    MiddlewareDef {
        param: (Symbol, &'a TypeExpr<'a>),
        body: Block<'a>,
    },

    /// `## Redirect /from -> /to.`
    RedirectDef { from: Symbol, to: Symbol },

    /// Mount persistent CRDT from journal file.
    /// `Mount counter at "data/counter.journal".`
    /// Semantics: Load or create journal, replay operations to reconstruct state
    Mount {
        /// The variable name for the mounted value
        var: Symbol,
        /// The path expression for the journal file
        path: &'a Expr<'a>,
    },

    // =========================================================================
    // Go-like Concurrency (Green Threads, Channels, Select)
    // =========================================================================
    /// Launch a fire-and-forget task (green thread).
    /// `Launch a task to process(data).`
    /// Semantics: tokio::spawn with no handle capture
    LaunchTask {
        /// The function to call
        function: Symbol,
        /// Arguments to pass
        args: Vec<&'a Expr<'a>>,
    },

    /// Launch a task with handle for control.
    /// `Let worker be Launch a task to process(data).`
    /// Semantics: tokio::spawn returning JoinHandle
    LaunchTaskWithHandle {
        /// Variable to bind the handle
        handle: Symbol,
        /// The function to call
        function: Symbol,
        /// Arguments to pass
        args: Vec<&'a Expr<'a>>,
    },

    /// Create a bounded channel (pipe).
    /// `Let jobs be a new Pipe of Int.`
    /// Semantics: tokio::sync::mpsc::channel(32)
    CreatePipe {
        /// Variable for the pipe
        var: Symbol,
        /// Type of values in the pipe
        element_type: Symbol,
        /// Optional capacity (defaults to 32)
        capacity: Option<u32>,
    },

    /// Blocking send into pipe.
    /// `Send value into pipe.`
    /// Semantics: pipe_tx.send(value).await
    SendPipe {
        /// The value to send
        value: &'a Expr<'a>,
        /// The pipe to send into
        pipe: &'a Expr<'a>,
    },

    /// Blocking receive from pipe.
    /// `Receive x from pipe.`
    /// Semantics: let x = pipe_rx.recv().await
    ReceivePipe {
        /// Variable to bind the received value
        var: Symbol,
        /// The pipe to receive from
        pipe: &'a Expr<'a>,
    },

    /// Non-blocking send (try).
    /// `Try to send value into pipe.`
    /// Semantics: pipe_tx.try_send(value) - returns immediately
    TrySendPipe {
        /// The value to send
        value: &'a Expr<'a>,
        /// The pipe to send into
        pipe: &'a Expr<'a>,
        /// Variable to bind the result (true/false)
        result: Option<Symbol>,
    },

    /// Non-blocking receive (try).
    /// `Try to receive x from pipe.`
    /// Semantics: pipe_rx.try_recv() - returns Option
    TryReceivePipe {
        /// Variable to bind the received value (if any)
        var: Symbol,
        /// The pipe to receive from
        pipe: &'a Expr<'a>,
    },

    /// Cancel a spawned task.
    /// `Stop worker.`
    /// Semantics: handle.abort()
    StopTask {
        /// The handle to cancel
        handle: &'a Expr<'a>,
    },

    /// Close a channel (pipe).
    /// `Close ch.` or `Close the pipe ch.`
    /// Semantics: mark the channel closed so pending/future receivers wake with the
    /// closed signal (`Op::ChanClose`). Buffered values stay receivable; a later send
    /// is the error. The deterministic-scheduler sibling of `Stop` for tasks.
    CloseChannel {
        /// The channel to close
        channel: &'a Expr<'a>,
    },

    /// Await a spawned task handle's completion.
    /// `Await worker.` or `Wait for worker.` (optionally `… into result`).
    /// Semantics: park until the task finishes, taking its result (`Op::TaskAwait`).
    /// The join sibling of `Stop` (which cancels) — this waits for and observes the
    /// child's completion.
    AwaitTask {
        /// The task handle to join.
        handle: &'a Expr<'a>,
        /// Optional binding for the task's result (`… into r`); `None` discards it.
        result: Option<Symbol>,
    },

    /// Select on multiple channels/timeouts.
    /// `Await the first of:`
    ///     `Receive x from ch:`
    ///         `...`
    ///     `After 5 seconds:`
    ///         `...`
    /// Semantics: tokio::select! with auto-cancel
    Select {
        /// The branches to select from
        branches: Vec<SelectBranch<'a>>,
    },

    /// Theorem block.
    /// `## Theorem: Name`
    /// `Given: Premise.`
    /// `Prove: Goal.`
    /// `Proof: Auto.`
    Theorem(TheoremBlock<'a>),

    /// `## Define` block — a vernacular-logic predicate definition (Rung 0a).
    /// `x is a bachelor if and only if x is unmarried and x is a man.`
    /// Non-executable: like [`Stmt::Theorem`], it is a declaration the proof
    /// layer consumes, not code the VM/AOT runs.
    Definition(DefinitionBlock<'a>),

    /// `## Axiom` block — a named first-order axiom in formal notation. Registers a
    /// shared premise for later theorems (the seam for an axiomatic base like Tarski).
    Axiom(AxiomBlock),

    /// `## Theory` block — a named development grouping formal axioms and theorems.
    Theory(TheoryBlock),

    /// Escape hatch: embed raw foreign code.
    /// `Escape to Rust:` followed by an indented block of raw code.
    ///
    /// Variables from the enclosing LOGOS scope are available in the
    /// escape block as their generated Rust types. The raw code is
    /// emitted verbatim inside a `{ ... }` block in the generated Rust.
    Escape {
        /// Target language ("Rust" for now, forward-compatible with "Python", "WGSL", etc.)
        language: Symbol,
        /// Raw foreign code, captured verbatim with base indentation stripped.
        code: Symbol,
        /// Source span covering the entire escape block (header + body).
        span: crate::token::Span,
    },

    /// Dependency declaration from `## Requires` block.
    /// The "serde" crate version "1.0" with features "derive".
    Require {
        crate_name: Symbol,
        version: Symbol,
        features: Vec<Symbol>,
        span: crate::token::Span,
    },

    /// ★ A RUNTIME FAILURE-SITE MARKER (Wave 3: "runtime failures carry a location") — no `Stmt`
    /// variant carries a span (this one included: its OWN payload is an opaque id, not a span), so
    /// a trap deep inside an action's body has never been able to say WHERE in the source it
    /// happened. This is the fix, and it is a MARKER rather than a span field on every `Stmt`
    /// variant on purpose: giving every variant a span is a schema change touching every
    /// constructor and every exhaustive match in the workspace for a span most of them never need
    /// (a `Let` inside a `## Main` that never fails costs nothing to leave unmarked). A `Site(id)`
    /// is inserted as its OWN statement, immediately before the sentence it marks, by
    /// `Parser::parse_indented_block` whenever `Parser::site_tracking` names a component and
    /// action — so it travels through the ordinary statement pipeline into every backend's own
    /// compiled form (bytecode, wasm, C, …) exactly like any other statement, which a parse-time-
    /// only side table (`Parser::stmt_spans`) cannot: that table is gone by the time a COMPILED
    /// program traps at runtime, and `stmt_spans` covers only top-level statements besides.
    ///
    /// `id` indexes [`Parser::failure_sites`] (built as each marker is inserted) for the
    /// `(file, span, component, action)` it names. Every backend that runs a `Stmt` must accept
    /// this variant in an EXPLICIT arm, not a wildcard — this file's own doc history is full of
    /// wildcard arms that silently swallowed a case nobody noticed was reachable; a backend with
    /// nothing to do here yet says so by name (`Stmt::Site(_) => {}` or equivalent), so the day it
    /// DOES need to do something, the compiler — not a bug report — is what tells whoever adds a
    /// new backend that this one still needs it. The tree-walker's job with it is the simplest one
    /// there is: `current site = id` (`FailureSite`'s own doc comment says why this is enough for a
    /// location); every OTHER backend, not yet given a reason to do more, ignores it explicitly and
    /// moves on.
    ///
    /// [`Parser::failure_sites`]: crate::Parser::failure_sites
    Site(u32),
}

/// One entry in [`Parser::failure_sites`] — everything a runtime failure needs to point BACK at
/// the sentence a [`Stmt::Site`] marker stood in front of.
///
/// ★ "CURRENT SITE = n" IS ENOUGH, AND HERE IS WHY. A trap does not need a call stack to be useful
/// — LOGOS actions do not recurse into each other's OWN bodies the way a general-purpose language's
/// functions do (an action calls another via `Call <Store>'s <action>`, a dispatch through data, not
/// a nested nomination-time invocation whose OWN Sites would need distinguishing on a stack) — so
/// "which Site did the runtime most recently pass through" already answers "which sentence trapped"
/// unambiguously, for the same reason a program counter alone (no full backtrace) already answers it
/// in a language with no first-class stack frames to unwind through. A future construct that DOES
/// nest one action's failure inside another's turn is exactly the case that would need this
/// reconsidered — flagging it here so nobody has to rediscover why a stack was not built.
///
/// ★ NO `file` FIELD, ON PURPOSE. A `Parser` parses ONE flat token stream and has no concept of
/// splicing at all — `loader::apply_prelude_mapped` assembles a user file plus its demand-imported
/// stdlib modules into that ONE stream BEFORE the parser ever sees it, and `ui_bridge.rs`'s checked
/// seam is what undoes the splice, exactly once, on whatever span leaves the compile pipeline (item
/// 10 of this session's own record — the double-mapping bug that fix closed). `span` here is in the
/// SAME spliced coordinates every other span this parser hands out is in, for the same reason: a
/// second, bespoke translation point for `FailureSite` spans specifically would be exactly the
/// double-mapping shape that bug already was. Whoever renders a `FailureSite` for a person translates
/// it through the SAME seam every other diagnostic in this pipeline already goes through.
///
/// [`Parser::failure_sites`]: crate::Parser::failure_sites
#[derive(Debug, Clone, Copy)]
pub struct FailureSite {
    /// Where the marked sentence is, in spliced coordinates — see the field's own note above for
    /// why there is no separate `file` to pair it with.
    pub span: crate::token::Span,
    /// The component (or store, or page) whose action/handler this sentence is written in.
    pub component: Symbol,
    /// The action or event handler this sentence belongs to — `__e3`-style synthesized names for a
    /// `When …:` handler, same as every other place this pipeline names one.
    pub action: Symbol,
}

impl FailureSite {
    /// `message` (the runtime error text itself, unchanged) wrapped with WHICH sentence raised it
    /// — `component`/`action`, in the author's own words wherever this site can recover them.
    /// Deliberately does NOT touch `file:line:col`: `span` is still in spliced coordinates (this
    /// struct's own doc comment says why there is no `file` to pair it with), and turning a byte
    /// offset into a line/column needs the source text this crate never holds once parsing ends —
    /// that translation is `ui_bridge.rs`'s checked seam's job, on the SAME span, same as every
    /// other diagnostic; this method only ever supplies the component/action half.
    ///
    /// An event handler's `action` is a synthesized `__e<n>` (`lgx_lower.rs::action_fn_name`'s own
    /// numbering) — never a name the author wrote, so it is named generically ("an event handler")
    /// rather than surfaced verbatim. An ordinary `To <name>:` action's own name IS what the author
    /// wrote, and is quoted as `` `To <name>` `` to read the same way the section that declares it
    /// does.
    pub fn render(&self, interner: &Interner, message: &str) -> String {
        let component = interner.resolve(self.component);
        let action = interner.resolve(self.action);
        let is_synthesized_event_name = action
            .strip_prefix("__e")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        let where_ = if is_synthesized_event_name {
            format!("an event handler in `{component}`")
        } else {
            format!("`To {action}` in `{component}`")
        };
        format!("{where_}: {message}")
    }
}

/// A branch in a Select statement.
#[derive(Debug, Clone)]
pub enum SelectBranch<'a> {
    /// Receive from a pipe: `Receive x from ch:`
    Receive {
        var: Symbol,
        pipe: &'a Expr<'a>,
        body: Block<'a>,
    },
    /// Timeout: `After N seconds:` or `After N milliseconds:`
    Timeout {
        milliseconds: &'a Expr<'a>,
        body: Block<'a>,
    },
}

/// Shared expression type for pure computations (LOGOS §15.0.0).
///
/// Expr is used by both LogicExpr (as terms) and Stmt (as values).
/// These are pure computations without side effects.
#[derive(Debug)]
pub enum Expr<'a> {
    /// Literal value: 42, "hello", true, nothing
    Literal(Literal),

    /// Variable reference: x
    Identifier(Symbol),

    /// Binary operation: x plus y
    BinaryOp {
        op: BinaryOpKind,
        left: &'a Expr<'a>,
        right: &'a Expr<'a>,
    },

    /// Unary NOT: "not x" → `!x` (logical for Bool, bitwise for Int)
    Not { operand: &'a Expr<'a> },

    /// Function call as expression: f(x, y)
    Call {
        function: Symbol,
        args: Vec<&'a Expr<'a>>,
    },

    /// Dynamic index access: `items at i` (1-indexed).
    Index {
        collection: &'a Expr<'a>,
        index: &'a Expr<'a>,
    },

    /// Dynamic slice access: `items 1 through mid` (1-indexed, inclusive).
    Slice {
        collection: &'a Expr<'a>,
        start: &'a Expr<'a>,
        end: &'a Expr<'a>,
    },

    /// Copy expression: `copy of slice` → slice.to_vec().
    Copy { expr: &'a Expr<'a> },

    /// Give expression: `Give x` → transfers ownership, no clone needed.
    /// Used in function calls to explicitly move values.
    Give { value: &'a Expr<'a> },

    /// Length expression: `length of items` → items.len().
    Length { collection: &'a Expr<'a> },

    /// Set contains: `set contains x` or `x in set`
    Contains {
        collection: &'a Expr<'a>,
        value: &'a Expr<'a>,
    },

    /// Set union: `a union b`
    Union {
        left: &'a Expr<'a>,
        right: &'a Expr<'a>,
    },

    /// Set intersection: `a intersection b`
    Intersection {
        left: &'a Expr<'a>,
        right: &'a Expr<'a>,
    },

    /// Get manifest of a zone.
    /// `the manifest of Zone` → FileSipper::from_zone(&zone).manifest()
    ManifestOf { zone: &'a Expr<'a> },

    /// Get chunk at index from a zone.
    /// `the chunk at N in Zone` → FileSipper::from_zone(&zone).get_chunk(N)
    ChunkAt {
        index: &'a Expr<'a>,
        zone: &'a Expr<'a>,
    },

    /// List literal: [1, 2, 3]
    List(Vec<&'a Expr<'a>>),

    /// Tuple literal: (1, "hello", true)
    Tuple(Vec<&'a Expr<'a>>),

    /// Range: 1 to 10 (inclusive)
    Range {
        start: &'a Expr<'a>,
        end: &'a Expr<'a>,
    },

    /// Field access: `p's x` or `the x of p`.
    FieldAccess { object: &'a Expr<'a>, field: Symbol },

    /// Constructor: `a new Point` or `a new Point with x 10 and y 20`.
    /// Supports generics: `a new Box of Int` and nested types: `a new Seq of (Seq of Int)`
    New {
        type_name: Symbol,
        type_args: Vec<TypeExpr<'a>>, // Empty for non-generic types - now supports nested types
        init_fields: Vec<(Symbol, &'a Expr<'a>)>, // Optional field initialization
    },

    /// Enum variant constructor: `a new Circle with radius 10`.
    NewVariant {
        enum_name: Symbol,                   // Shape (resolved from registry)
        variant: Symbol,                     // Circle
        fields: Vec<(Symbol, &'a Expr<'a>)>, // [(radius, 10)]
    },

    /// Escape hatch expression: raw foreign code that produces a value.
    /// Used in expression position: `Let x: Int be Escape to Rust:`
    Escape { language: Symbol, code: Symbol },

    /// Option Some: `some 30` → Some(30)
    OptionSome { value: &'a Expr<'a> },

    /// Option None: `none` → None
    OptionNone,

    /// Pre-allocation capacity hint wrapping an inner value expression.
    /// `"" with capacity 100` or `a new Seq of Int with capacity n`
    /// Codegen uses with_capacity(); interpreter ignores the hint.
    WithCapacity {
        value: &'a Expr<'a>,
        capacity: &'a Expr<'a>,
    },

    /// Closure expression: `(params) -> body` or `(params) ->:` block body.
    /// Captures variables from the enclosing scope by value (snapshot/clone).
    Closure {
        params: Vec<(Symbol, &'a TypeExpr<'a>)>,
        body: ClosureBody<'a>,
        return_type: Option<&'a TypeExpr<'a>>,
    },

    /// Call an expression that evaluates to a callable value.
    /// `f(x)` where `f` is a variable holding a closure, not a named function.
    CallExpr {
        callee: &'a Expr<'a>,
        args: Vec<&'a Expr<'a>>,
    },

    /// Interpolated string: `"Hello, {name}! Value: {x:.2}"`
    InterpolatedString(Vec<StringPart<'a>>),

    /// A template literal — the body of a `## To … -> View:` function: a view tree that
    /// renders wherever the returned `View` value is placed.
    View(Box<crate::ast::view::ViewDef<'a>>),

    /// A question about the rendered page: `the text of "#draft"`, `the count of ".row"`.
    /// An ordinary expression, so every assertion form already works on it — a page test
    /// needs no statements of its own.
    Dom(DomQuery<'a>),
}

/// What `the <thing> of <selector>` asks the page.
#[derive(Debug, Clone)]
pub enum DomQuery<'a> {
    /// `the text of SEL` — the element's text content (its own text plus its descendants').
    Text { selector: &'a Expr<'a> },
    /// `the count of SEL` — how many elements match.
    Count { selector: &'a Expr<'a> },
    /// `the value of SEL` — an input's current value.
    Value { selector: &'a Expr<'a> },
    /// `SEL is visible` — whether anything matches at all.
    Visible { selector: &'a Expr<'a> },
    /// `SEL is hidden` — the exact complement of `visible`, so an element is always one or the
    /// other and a test written with either word is complete.
    Hidden { selector: &'a Expr<'a> },
    /// `SEL is disabled` — whether the match carries the `disabled` attribute. A PRESENCE, like
    /// the render: asking by comparing against the string `"false"` would call an enabled
    /// control disabled.
    Disabled { selector: &'a Expr<'a> },
    /// `SEL is checked` — a checkbox's or radio's LIVE state, which is a property rather than an
    /// attribute (see `DomOp::SetChecked`), so this follows a click.
    Checked { selector: &'a Expr<'a> },
    /// `SEL is focused` — whether the match is the element focus is on.
    Focused { selector: &'a Expr<'a> },
    /// `the journal` — the ACTIONS that have run, in order, as a `Seq of Text`.
    ///
    /// A value rather than a bespoke assertion, so `the count of`, `contains` and `item N of`
    /// all work on it and the ordinary questions need no journal-specific vocabulary.
    Journal,
    /// `the journal shows A then B` — did `A` run and `B` run AFTER it?
    ///
    /// The one question a sequence operation cannot ask without an index-of. "Then" means after,
    /// not immediately after: pinning adjacency would break a test every time an unrelated turn
    /// appeared between the two it cares about.
    JournalShows { first: Symbol, second: Symbol },
    /// `the title` — the page title.
    Title,
    /// `the URL` — the current path.
    Url,
}

impl<'a> DomQuery<'a> {
    /// The selector expression, when the query has one.
    pub fn selector(&self) -> Option<&'a Expr<'a>> {
        match self {
            DomQuery::Text { selector }
            | DomQuery::Count { selector }
            | DomQuery::Value { selector }
            | DomQuery::Visible { selector }
            | DomQuery::Hidden { selector }
            | DomQuery::Disabled { selector }
            | DomQuery::Checked { selector }
            | DomQuery::Focused { selector } => Some(selector),
            DomQuery::Title | DomQuery::Url | DomQuery::Journal | DomQuery::JournalShows { .. } => {
                None
            }
        }
    }

    /// The English word this query reads as — for messages and shape printers.
    pub fn noun(&self) -> &'static str {
        match self {
            DomQuery::Text { .. } => "text",
            DomQuery::Count { .. } => "count",
            DomQuery::Value { .. } => "value",
            DomQuery::Visible { .. } => "visible",
            DomQuery::Hidden { .. } => "hidden",
            DomQuery::Disabled { .. } => "disabled",
            DomQuery::Checked { .. } => "checked",
            DomQuery::Focused { .. } => "focused",
            DomQuery::Journal => "journal",
            DomQuery::JournalShows { .. } => "journal ordering",
            DomQuery::Title => "title",
            DomQuery::Url => "URL",
        }
    }
}

/// A segment of an interpolated string.
#[derive(Debug, Clone)]
pub enum StringPart<'a> {
    /// Literal text segment
    Literal(Symbol),
    /// Expression with optional format specifier
    Expr {
        value: &'a Expr<'a>,
        format_spec: Option<Symbol>,
        /// Self-documenting debug format: `{var=}` → `"var=42"`
        debug: bool,
    },
}

/// Body of a closure expression.
#[derive(Debug, Clone)]
pub enum ClosureBody<'a> {
    /// Single expression: `(n: Int) -> n * 2`
    Expression(&'a Expr<'a>),
    /// Block of statements: `(n: Int) ->:` followed by indented body
    Block(Block<'a>),
}

/// Literal values in LOGOS.
#[derive(Debug, Clone)]
pub enum Literal {
    /// Integer literal
    Number(i64),
    /// Float literal
    Float(f64),
    /// Text literal
    Text(Symbol),
    /// Boolean literal
    Boolean(bool),
    /// The nothing literal (unit type)
    Nothing,
    /// Character literal
    Char(char),
    /// Duration literal (nanoseconds, signed for negative offsets like "5 minutes early")
    Duration(i64),
    /// Date literal (days since Unix epoch 1970-01-01)
    Date(i32),
    /// Moment literal (nanoseconds since Unix epoch)
    Moment(i64),
    /// Calendar span (months, days) - NOT flattened to seconds
    /// Months and days are kept separate because they're incommensurable.
    Span { months: i32, days: i32 },
    /// Time-of-day literal (nanoseconds from midnight)
    /// Range: 0 to 86_399_999_999_999 (just under 24 hours)
    Time(i64),
}

impl PartialEq for Literal {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Literal::Number(a), Literal::Number(b)) => a == b,
            (Literal::Float(a), Literal::Float(b)) => a.to_bits() == b.to_bits(),
            (Literal::Text(a), Literal::Text(b)) => a == b,
            (Literal::Boolean(a), Literal::Boolean(b)) => a == b,
            (Literal::Nothing, Literal::Nothing) => true,
            (Literal::Char(a), Literal::Char(b)) => a == b,
            (Literal::Duration(a), Literal::Duration(b)) => a == b,
            (Literal::Date(a), Literal::Date(b)) => a == b,
            (Literal::Moment(a), Literal::Moment(b)) => a == b,
            (
                Literal::Span {
                    months: m1,
                    days: d1,
                },
                Literal::Span {
                    months: m2,
                    days: d2,
                },
            ) => m1 == m2 && d1 == d2,
            (Literal::Time(a), Literal::Time(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Literal {}

impl std::hash::Hash for Literal {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Literal::Number(n) => n.hash(state),
            Literal::Float(f) => f.to_bits().hash(state),
            Literal::Text(s) => s.hash(state),
            Literal::Boolean(b) => b.hash(state),
            Literal::Nothing => {}
            Literal::Char(c) => c.hash(state),
            Literal::Duration(d) => d.hash(state),
            Literal::Date(d) => d.hash(state),
            Literal::Moment(m) => m.hash(state),
            Literal::Span { months, days } => {
                months.hash(state);
                days.hash(state);
            }
            Literal::Time(t) => t.hash(state),
        }
    }
}
