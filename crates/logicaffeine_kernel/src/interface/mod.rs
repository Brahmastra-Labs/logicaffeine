//! Vernacular interface for the Kernel.
//!
//! Provides a text-based command interface for interacting with the kernel:
//! - `Definition name : type := term.` - Add a definition
//! - `Check term.` - Print the type of a term
//! - `Eval term.` - Normalize and print a term
//! - `Inductive Name := C1 : T1 | C2 : T2.` - Define an inductive type
//!
//! # Surface extensions (they declare a SPELLING, never a global)
//!
//! - `Notation "a ⊕ b" := ((myOp a) b).` - Mixfix notation; identifier pieces are holes
//! - `Infixl 65 "⋄" := f.` / `Infixr` / `Prefix "∽" := neg.` / `Postfix "‼" := bang.`
//! - `Macro "twice $e" := ….` / `MacroRules "list![$x, $xs]" := ….` - Pattern rewriting
//! - `Section Name.` … `End Name.` - Scope a `Local Notation` to a section
//! - `/-- doc -/`, `/-! module doc -/`, `-- line comment` - Prose, which declares nothing
//!
//! # Doors
//!
//! [`Repl::execute`] reads ONE command and refuses a prefix; [`Repl::execute_document`] reads a
//! whole document and stops at the first block that does not check.
//!
//! [`parse_command`] and `parse_command_in` read a command from TEXT ALONE. One surface cannot
//! be read from text alone — a literate `Consider`'s equation set has to be compiled against the
//! discriminant's inductive, and the constructor list, its order and its arities live in the
//! kernel context — so [`parse_command_in_ctx`] is the door that has one.
//!
//! # Literate Syntax (English-like alternative)
//!
//! - `A Bool is either Yes or No.` - Define an inductive type
//! - `## To add (n: Nat) and (m: Nat) -> Nat:` - Define a function
//! - `Consider x: When Zero: Yield m.` - Pattern matching

mod command;
mod command_parser;
mod error;
#[cfg(feature = "lean")]
pub mod lean;
pub mod literate_parser;
pub mod match_compile;
pub mod printer;
pub(crate) mod raw_match;
mod repl;
pub mod surface;
mod term_parser;

pub use command::{Command, InductiveMember};
pub use command_parser::{parse_command, parse_command_prefix_in};
pub use error::{InterfaceError, ParseError};
pub use match_compile::{
    compile_clauses, compile_clauses_reporting, Clause, CompileReport, Pattern,
};
pub use printer::{print_command, print_term, print_term_in, PrintError};
pub use repl::Repl;
pub use surface::{Assoc, OperatorForm, Surface, SurfaceDecl};
pub use term_parser::{BinderKind, BinderSurface, DeclarationSurface, TermParser};

/// Every name a term SPELLS, collected into `out`.
///
/// ★ THIS IS WHAT "FRESH" HAS TO BE CHECKED AGAINST. Two places in this module introduce a name
/// of their own — the equation compiler's field binders (`#fld0`, `#fld1`, …) and the literate
/// surface's eigenvariables (`n#eig`) — and both used to argue freshness from the LEXER: "`#`
/// cannot occur in an identifier, so a generated name can collide with nothing a user wrote".
/// The argument is false. `«#fld0»` and `«n#eig»` are `«quoted names»`, which `scan_name`
/// accepts everywhere an identifier is read, so both names are writable — and a collision is a
/// captured binder, which is a well-typed term denoting a different function, or an
/// eigenvariable the statement can NAME, which is ∀-introduction over a constant that is not
/// arbitrary. Neither is reported by anything.
///
/// So freshness is CHECKED. A generated name is compared against this set and stepped until it
/// is outside it, which stays true of any lexer, any escape and any quoting rule added later.
/// Binders are collected as well as occurrences: a generated name that shadowed a binder would
/// be just as wrong as one that captured a variable.
pub(super) fn spelled_names(term: &crate::Term, out: &mut std::collections::HashSet<String>) {
    use crate::Term;
    match term {
        Term::Var(n) | Term::Global(n) => {
            out.insert(n.clone());
        }
        Term::Const { name, .. } => {
            out.insert(name.clone());
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            out.insert(param.clone());
            spelled_names(param_type, out);
            spelled_names(body_type, out);
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            out.insert(param.clone());
            spelled_names(param_type, out);
            spelled_names(body, out);
        }
        Term::App(f, x) => {
            spelled_names(f, out);
            spelled_names(x, out);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            spelled_names(discriminant, out);
            spelled_names(motive, out);
            for c in cases {
                spelled_names(c, out);
            }
        }
        Term::Fix { name, body } => {
            out.insert(name.clone());
            spelled_names(body, out);
        }
        Term::MutualFix { defs, .. } => {
            for (n, b) in defs {
                out.insert(n.clone());
                spelled_names(b, out);
            }
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            out.insert(name.clone());
            spelled_names(ty, out);
            spelled_names(value, out);
            spelled_names(body, out);
        }
        Term::Annotated { term, .. } => spelled_names(term, out),
        Term::Sort(_) | Term::Lit(_) | Term::Hole => {}
    }
}

/// ★ ONE NAME GRAMMAR, EVERY DECLARATION DOOR — the name a declaration may be keyed at.
///
/// The kernel `Context` is a map of FLAT keys, so a declaration name is only useful if two things
/// hold, and neither implies the other:
///
/// * a TERM can spell it — [`surface::Name::is_spelled_by`] hands the spelling to the term door
///   and requires this same name back, so the invariant is checked against the grammar rather
///   than restated beside it and left to drift. `end` scans clean as a component and is a
///   reserved word to the term door, so declaring it mints a global no `Check` can ever name;
/// * its flat key READS BACK as itself — `«a.b»` is one component keying at `a.b`, which reads
///   back as the two-component `a.b`, so the two names share one key and each answers to the
///   other's spelling with nothing reported.
///
/// A name failing either is WRITE-ONLY: the context holds it and no later command can reach it.
///
/// This is the parent module's function because BOTH doors declare names —
/// `command_parser`'s `Definition`/`Inductive` and [`literate_parser`]'s `## To`,
/// `## Theorem:`, `Let … be` and `A … is either` — and a rule about what a declaration may be
/// keyed at that only one door applied would not be a rule, it would be a door's habit.
/// `M_equations_extra::every_literate_declaration_door_refuses_a_write_only_name` pins the
/// literate side and `vernacular_extra::a_declarable_name_is_always_a_referenceable_one` the
/// vernacular one, on the same grid.
pub(super) fn declaration_name(raw: &str) -> Result<surface::Name, ParseError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ParseError::Missing("declaration name".to_string()));
    }
    match surface::Name::scan(raw) {
        Some((name, len)) if len == raw.len() && !name.is_spelled_by(raw) => {
            Err(ParseError::InvalidIdent(format!(
                "{raw:?} \u{2014} a declaration name must be one a TERM can spell, and no term can \
                 spell this one. A component after a `.` must be a plain identifier, and a \
                 reserved word (`fun`, `forall`, `fix`, `match`, `let`, `in`, `with`, `return`, \
                 `end`, `by`, `Type`, `Prop`) is not a name at all \u{2014} declaring one would key the \
                 context at a name nothing could ever refer to"
            )))
        }
        Some((name, len)) if len == raw.len() && name.key_reads_back() => Ok(name),
        Some((name, len)) if len == raw.len() => Err(ParseError::InvalidIdent(format!(
            "{raw:?} \u{2014} this name is keyed at {:?}, which reads back as the different name \
             {:?}. A quoted component may not contain `.`: the kernel context is a map of flat \
             keys, so the two names would collide and each would answer to the other's spelling",
            name.key(),
            surface::Name::parse(&name.key())
                .map(|n| n.parts().join(" \u{b7} "))
                .unwrap_or_default()
        ))),
        _ => Err(ParseError::InvalidIdent(format!(
            "{raw:?} \u{2014} a declaration name must be one a term can spell: an identifier \
             (letters, digits, `_`, `'`), a \u{ab}quoted name\u{bb}, or those joined by `.` into a \
             hierarchical name"
        ))),
    }
}

/// Parse ONE command off the FRONT of `input` with the kernel CONTEXT available, returning it
/// with the number of bytes consumed.
///
/// This is the primitive the context-aware doors are built from, exactly as
/// `command_parser::parse_command_prefix_in` is the primitive the text-only ones are built from:
/// [`parse_command_in_ctx`] adds "and nothing followed", and a document splitter loops it.
///
/// One surface cannot be read from text alone. A literate `Consider`'s equation set has to be
/// compiled against the discriminant's inductive, and the constructor list, its REGISTRATION
/// ORDER and their arities live in the kernel context; a parser holding only a `&str` can only
/// push one case per clause in SOURCE order — which silently computes a different function for
/// every equal-arity inductive, and cannot express a nested pattern or a catch-all at all.
///
/// `## Theorem:` is routed too, for the other fact text cannot carry: how many constructors
/// `Proof: induction <v>.` is eliminating over, so a bullet-count mistake is reported as one.
///
/// Both literate doors read their names in the surface's NAMESPACE SCOPE and through the
/// context's shadow table — the resolution every other door gets from the REPL — because they
/// must know what a name MEANS while they read: a clause is compiled against the constructors of
/// the inductive its discriminant really has, and a `## Theorem`'s statement is reflected into
/// syntax naming the constants it really mentions. What they hand back is re-spelled
/// (`surface::respell_names`), so the REPL's own resolution of it changes nothing.
///
/// Every other surface is routed to the context-free door untouched: this is a pre-dispatch on
/// two prefixes, not a second copy of the dispatcher. [`Repl`] reads every command through this
/// door when it runs it; the document splitter only measures, through
/// `measure_command_prefix`, since a block cannot be read against the context the blocks before
/// it have not built yet.
pub fn parse_command_prefix_in_ctx(
    ctx: &crate::Context,
    surface: &Surface,
    input: &str,
) -> Result<(Command, usize), ParseError> {
    let lead = input.len() - input.trim_start().len();
    let rest = &input[lead..];
    // The two prefixes are spelled EXACTLY as `command_parser`'s own dispatch spells them. A door
    // that recognised one more spelling than the other would be a second dispatcher wearing a
    // pre-dispatch's clothes, and the difference would only ever show up as one door accepting a
    // command the other refuses.
    type Door =
        fn(&crate::Context, &surface::Namespaces, &str) -> Result<(Command, usize), ParseError>;
    let literate: Option<Door> = if rest.starts_with("## To ") {
        Some(literate_parser::parse_definition_scoped)
    } else if rest.starts_with("## Theorem:") {
        Some(literate_parser::parse_theorem_scoped)
    } else {
        None
    };
    match literate {
        Some(parse) => {
            let (cmd, n) = parse(ctx, surface.namespaces(), rest)?;
            Ok((cmd, lead + with_terminator(rest, n)))
        }
        None => command_parser::parse_command_prefix_in(surface, input),
    }
}

/// Run `f` with the environment every name a session reads is resolved against: the keys the
/// context declares (a universe-polymorphic definition is keyed in its own table, which
/// `get_global` does not read), the keys in `pending` — what the command being run is about to
/// declare — and the context's shadow table. One construction, so no door can come to resolve
/// against a different environment than another.
pub(crate) fn with_env<R>(
    ctx: &crate::Context,
    pending: &[String],
    f: impl FnOnce(&surface::Env<'_>) -> R,
) -> R {
    let known = |key: &str| {
        pending.iter().any(|p| p == key)
            || ctx.get_global(key).is_some()
            || ctx.poly_declaration(key).is_some()
    };
    let shadow = |written: &str| ctx.shadow_of(written);
    f(&surface::Env {
        known: &known,
        shadow: &shadow,
    })
}

/// The EXTENT of the command at the front of `input`: the bytes it occupies, read from its syntax
/// alone and never from what its names mean. The document splitter's door — a document is split
/// before any block of it runs, so no block can be read against the context the blocks before it
/// will build (`literate_parser::measure_theorem`). Coq-style sentences are measured by the
/// context-free door, as every door measures them.
pub(crate) fn measure_command_prefix(surface: &Surface, input: &str) -> Result<usize, ParseError> {
    let lead = input.len() - input.trim_start().len();
    let rest = &input[lead..];
    let literate: Option<fn(&str) -> Result<usize, ParseError>> = if rest.starts_with("## To ") {
        Some(literate_parser::measure_definition)
    } else if rest.starts_with("## Theorem:") {
        Some(literate_parser::measure_theorem)
    } else {
        None
    };
    match literate {
        Some(measure) => Ok(lead + with_terminator(rest, measure(rest)?)),
        None => command_parser::parse_command_prefix_in(surface, input).map(|(_, n)| n),
    }
}

/// A literate block's extent INCLUDES the sentence terminator that closes it, if one is written.
///
/// This is the same rule `command_parser::with_terminator` applies to the text-only doors, and
/// `the_two_doors_measure_a_command_the_same_way` pins the two against each other over every
/// surface they share — a command whose extent the two doors disagreed about would split a
/// document differently depending on whether its caller happened to hold a context.
fn with_terminator(rest: &str, n: usize) -> usize {
    let tail = &rest[n..];
    let spaces = tail.len() - tail.trim_start_matches([' ', '\t']).len();
    if tail[spaces..].starts_with('.') {
        n + spaces + 1
    } else {
        n
    }
}

/// Parse ONE command with the kernel CONTEXT available, and require it to be the whole input.
///
/// [`parse_command_prefix_in_ctx`] plus "and nothing followed" — the context-aware counterpart of
/// `parse_command`, and what [`Repl::execute`] should read its command through.
pub fn parse_command_in_ctx(
    ctx: &crate::Context,
    surface: &Surface,
    input: &str,
) -> Result<Command, ParseError> {
    let (cmd, consumed) = parse_command_prefix_in_ctx(ctx, surface, input)?;
    let rest = input[consumed..].trim();
    if !rest.is_empty() {
        return Err(ParseError::TrailingInput {
            at: consumed,
            rest: rest.to_string(),
        });
    }
    Ok(cmd)
}

/// A context of its own for a REPL or a document to extend: a copy-on-write clone of the process's
/// one standard library registration, [`crate::prelude::StandardLibrary::context`].
///
/// Measured on the bench box, optimized: registering the prelude takes 315 ms, a clone of the
/// registered context about 1 µs.
///
/// What comes back is an independent value the caller may mutate freely; it shares the prelude's
/// tables until it writes, and a write copies exactly the table it touches.
/// `the_cached_prelude_is_indistinguishable_from_a_fresh_one` pins it against a freshly registered
/// one, and `Repl::new` starts from it.
pub fn standard_context() -> crate::Context {
    crate::prelude::StandardLibrary::context().clone()
}
