//! Error types for the Vernacular interface.

use crate::KernelError;
use std::fmt;

/// Errors that can occur in the interface layer.
#[derive(Debug)]
pub enum InterfaceError {
    /// Parse error in command or term syntax.
    Parse(ParseError),

    /// Error from the kernel (type checking, etc.)
    Kernel(KernelError),

    /// A tactic script's step that did not apply, with the goal it ran on.
    Tactic(crate::tactics::ScriptError),
}

/// Parse errors for the Vernacular.
#[derive(Debug, Clone)]
pub enum ParseError {
    /// Unexpected end of input.
    UnexpectedEof,

    /// Unknown command keyword.
    UnknownCommand(String),

    /// Expected a specific token.
    Expected { expected: String, found: String },

    /// Invalid identifier.
    InvalidIdent(String),

    /// One short name, two or more declared constants, at the same resolution priority.
    ///
    /// ★ AN AMBIGUOUS NAME IS REPORTED, NEVER PICKED. This is the failure mode the audit called the
    /// dangerous one: the prelude already owns `Nat`, `Zero`, `Succ`, `Eq`, `True`, `False` and
    /// forty more, so a short name written under an `open` did not fail to resolve — it resolved to
    /// the PRELUDE's constant and nothing said a word. Both readings type-check and they are
    /// different theorems. Lean errors here; we error here and name BOTH candidates, because
    /// "ambiguous" without the list is a puzzle rather than a diagnosis.
    AmbiguousName {
        written: String,
        candidates: Vec<String>,
    },

    /// A document ended in a different scope than it started in.
    ///
    /// ★ A DOCUMENT IS BALANCED. `Namespace N.` with no `End N.` is the quietest failure this
    /// surface has: every declaration after it is written into `N`, nothing is reported, and the
    /// file still checks. Lean does NOT refuse one — measured on lean 4.14.0, a file that ends inside `namespace N` compiles without a word, because a Lean MODULE's scopes close at its end and the next file starts clean. A document here is not a module: it is handed to a live session that goes on executing commands after it, so an unclosed `Namespace N.` keys every later one into `N` and says nothing. The balance rule is therefore ours and deliberate, and the reason is the session rather than Lean. It is
    /// stated as BALANCE and not as "empty at the end", so a caller that was already
    /// inside a namespace when it handed the door a document is not punished for it — and a
    /// document that closes a scope it did not open is the same mistake pointing the other way.
    UnbalancedDocument {
        /// Scopes the document opened and never closed, outermost first.
        unclosed: Vec<String>,
        /// How many of the CALLER's scopes the document closed.
        overclosed: usize,
    },

    /// `Open Typo.` where nothing has ever been declared into `Typo`.
    ///
    /// An `open` that brings in NOTHING is indistinguishable from one that worked, and every short
    /// name written under it then quietly denotes the root's constant. Lean refuses an unknown
    /// namespace; so do we.
    UnknownNamespace(String),

    /// Invalid number literal.
    InvalidNumber(String),

    /// Missing required component.
    Missing(String),

    /// A complete term was parsed but input remained after it.
    ///
    /// ★ THE KERNEL MUST NOT ACCEPT A PREFIX. Parsing one term and discarding whatever followed
    /// meant `Check Nat -> Nat garbage here` checked `Nat -> Nat` and reported success about a
    /// claim nobody made. Every other component here can afford leniency and be merely wrong;
    /// this is the one that answers "is this proved".
    TrailingInput { at: usize, rest: String },

    /// A macro expansion did not reach a fixpoint within the step budget.
    ///
    /// A recursive `macro_rules` whose base case is unreachable must FAIL, loudly and at a stated
    /// limit, rather than spin — an expander that hangs is indistinguishable from a kernel that
    /// is thinking hard.
    MacroDepth { limit: usize },

    /// A construct the vernacular REFUSES by name, with the reason.
    ///
    /// The distinction that matters is refused-versus-MISPARSED. `Inductive Colour := red :
    /// Colour deriving BEq.` used to be accepted, registering `red` at type
    /// `((Colour deriving) BEq)` — a constructor whose declared type applies the inductive to two
    /// globals that are not in the context at all. That is strictly worse than rejecting the
    /// file: it is a declaration that names things which do not exist.
    Unsupported { what: String, why: String },

    // ============================================================
    // Literate Syntax Errors
    // ============================================================
    /// Missing "is either" clause in inductive definition.
    MissingEitherClause,

    /// Missing variant name in inductive definition.
    MissingVariantName,

    /// Invalid Consider/When/Yield syntax.
    InvalidConsiderSyntax(String),

    /// Missing Yield in function body.
    MissingYield,

    /// Missing When clause in Consider block.
    MissingWhenClause,

    /// Unclosed Consider block.
    UnclosedConsiderBlock,
}

impl fmt::Display for InterfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InterfaceError::Parse(e) => write!(f, "Parse error: {}", e),
            InterfaceError::Kernel(e) => write!(f, "Kernel error: {}", e),
            InterfaceError::Tactic(e) => write!(f, "Tactic error: {}", e),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnexpectedEof => write!(f, "Unexpected end of input"),
            ParseError::UnknownCommand(cmd) => write!(f, "Unknown command: {}", cmd),
            ParseError::Expected { expected, found } => {
                write!(f, "Expected {}, found {}", expected, found)
            }
            ParseError::TrailingInput { at, rest } => write!(
                f,
                "Unexpected input after the end of the term, at byte {}: {:?}. A term is read \
                 whole — if you meant two things, they need two commands.",
                at, rest
            ),
            ParseError::InvalidIdent(s) => write!(f, "Invalid identifier: {}", s),
            ParseError::AmbiguousName {
                written,
                candidates,
            } => write!(
                f,
                "the name `{}` is ambiguous — it denotes {} at the same priority, and picking one \
                 silently would change what this says. Write the one you mean in full, or \
                 `_root_.{}` for the top-level name.",
                written,
                candidates
                    .iter()
                    .map(|c| format!("`{c}`"))
                    .collect::<Vec<_>>()
                    .join(" and "),
                written
            ),
            ParseError::UnbalancedDocument {
                unclosed,
                overclosed,
            } if !unclosed.is_empty() => {
                write!(
                    f,
                    "this document ends inside {} \u{2014} a scope it opened and never closed. Every \
                     declaration written after it would land in that scope and nothing would say \
                     so, so the document is refused rather than read half in it. Close it with \
                     `End{}.`",
                    unclosed
                        .iter()
                        .map(|s| format!("`{s}`"))
                        .collect::<Vec<_>>()
                        .join(", inside "),
                    // An ANONYMOUS section has no name to close it by, so the advice must not tell
                    // the reader to write one — a bare `End.` is what closes it.
                    match unclosed.last().expect("non-empty").as_str() {
                        crate::interface::surface::ANONYMOUS_SECTION => "".to_string(),
                        named => format!(" {named}"),
                    }
                )?;
                if *overclosed > 0 {
                    write!(f, " (and it closed {overclosed} scope(s) it did not open)")?;
                }
                Ok(())
            }
            ParseError::UnbalancedDocument { overclosed, .. } => write!(
                f,
                "this document closed {overclosed} scope(s) it did not open. A document may leave \
                 the scope it was handed, and may not unwind past it \u{2014} the caller's later \
                 declarations would silently change where they land."
            ),
            ParseError::UnknownNamespace(ns) => write!(
                f,
                "there is no namespace `{ns}` — nothing has been declared into it. An `open` that \
                 brings in no names looks exactly like one that worked, so it is refused here \
                 rather than left to change what every short name under it means."
            ),
            ParseError::InvalidNumber(s) => write!(f, "Invalid number literal: {}", s),
            ParseError::Missing(what) => write!(f, "Missing {}", what),
            ParseError::Unsupported { what, why } => {
                write!(f, "{} is not supported: {}", what, why)
            }
            ParseError::MacroDepth { limit } => write!(
                f,
                "macro expansion did not terminate within {} steps — a recursive rule is not \
                 reaching its base case",
                limit
            ),
            // Literate syntax errors
            ParseError::MissingEitherClause => {
                write!(f, "Missing 'is either' clause in type definition")
            }
            ParseError::MissingVariantName => write!(f, "Missing variant name in type definition"),
            ParseError::InvalidConsiderSyntax(msg) => write!(f, "Invalid Consider syntax: {}", msg),
            ParseError::MissingYield => write!(f, "Missing 'Yield' in function body"),
            ParseError::MissingWhenClause => write!(f, "Missing 'When' clause in Consider block"),
            ParseError::UnclosedConsiderBlock => write!(f, "Unclosed Consider block"),
        }
    }
}

impl std::error::Error for InterfaceError {}
impl std::error::Error for ParseError {}

impl From<crate::tactics::ScriptError> for InterfaceError {
    fn from(e: crate::tactics::ScriptError) -> Self {
        InterfaceError::Tactic(e)
    }
}

impl From<ParseError> for InterfaceError {
    fn from(e: ParseError) -> Self {
        InterfaceError::Parse(e)
    }
}

impl From<KernelError> for InterfaceError {
    fn from(e: KernelError) -> Self {
        InterfaceError::Kernel(e)
    }
}
