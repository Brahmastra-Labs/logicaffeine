//! Parser for Kernel Term syntax.
//!
//! Parses the following grammar:
//! ```text
//! term    ::= lambda | forall | fix | match | let | group | instance | arrow
//! arrow   ::= app ("->" term)?
//! app     ::= atom+ ".."?
//! atom    ::= binder | named | "(" term ")" | sort | hole | "@" ident | ident
//! binder  ::= "(" ident+ ":" term (":=" default)? ")" "->"? term
//! group   ::= ("{" | "⦃") ident+ ":" term (":=" default)? ("}" | "⦄") "->"? term
//! instance::= "[" (ident+ ":")? term "]" "->"? term
//! named   ::= "(" ident ":=" term ")"
//! default ::= term | "by" ident
//! sort    ::= "Prop" | "SProp" | "Type" | "Type" digit+ | "Sort" level_atom
//! hole    ::= "_"
//! lambda  ::= "fun" binders (":" term)? "=>" term
//! forall  ::= "forall" binders (":" term)? "," term
//! binders ::= (ident | bgroup)+
//! bgroup  ::= "(" ident+ (":" term)? ")" | "{" ident+ (":" term)? "}"
//!           | "⦃" ident+ (":" term)? "⦄" | "[" (ident+ ":")? term "]"
//! fix     ::= "fix" ident "=>" term ("with" ident "=>" term)* ("return" ident)?
//! let     ::= "let" ident (":" term)? ":=" term "in" term
//!           | "let" ident ":" term ":=" term ";" term
//! match   ::= "match" app ("return" app)? "with" ("|" pattern "=>" term)* "end"?
//!           | "match" app "return" app "with" term ("|" term)* "end"
//! atom    ::= … | atom "." (ident | "«name»" | digit+) | name ".{" (level ("," level)*)? "}"
//! level   ::= level_atom ("+" digit+)*
//! level_atom ::= digit+ | ident | "«name»" | "max" level_atom level_atom
//!              | "imax" level_atom level_atom | "(" level ")"
//! literal ::= "-"? digit+ ("n" | unit)? | "-"? digit+ ("." digit+)? (("e"|"E") ("+"|"-")? digit+)? "f64"
//!           | "0x" hex{16} "f64" | "-"? digit{4,} "-" digit{2} "-" digit{2} time?
//!           | '"' char* '"'
//! time    ::= "T" digit{2} ":" digit{2} ":" digit{2} ("." digit{1,9})? "Z"
//! unit    ::= "ns" | "us" | "μs" | "ms" | "s" | "sec" | "min" | "h" | "hr"
//! ```
//!
//! # The kernel's own terms, spelled exactly
//!
//! Every production above that the Lean importer's terms need reads STRUCTURALLY, so that
//! `printer::print_term` can be its exact inverse:
//!
//! * a LEVEL is never normalised — `u+1` is `Succ(Var u)`, the numeral `1` is `Type 0`, `0` is
//!   `Prop`, and `max`/`imax` are the kernel's own two binary operators;
//! * `name.{ℓ, …}` is `Term::Const` — the explicit level arguments of a universe-polymorphic
//!   constant; a qualified name before `.{` is ONE constant, never a projection chain;
//! * a positional `match … return M with c₀ | c₁ end` (no leading bar) is the kernel's own
//!   `Term::Match`: one case per constructor, in registration order, read with no context at all;
//! * `fix f => b with g => c return g` is `Term::MutualFix`, every name bound in every body;
//! * `let x : T := v; b` is the kernel's `Term::Let`, its binder KEPT — where `let … in` substitutes
//!   and keeps only the ascription;
//! * every `Literal` has one spelling: `5n` is a `Nat`, `5` an `Int`, `1.5f64` and `0x…f64` a `Float`,
//!   `5s` a `Duration`, `2026-05-20` a `Date`, `2026-05-20T10:00:00Z` a `Moment`;
//! * a `«quoted»` name is always a NAME — `«Prop»` is a global, never the sort, and `«fun»` binds.
//!
//! # The argument-passing surface, and where it is carried
//!
//! Five families of production carry information the kernel's `Term` has no field for. Each rides
//! on a
//! RESERVED MARKER global -- the idiom `elaborate::ANON_CTOR_MARKER` / `DOT_MARKER` already
//! established -- so no `Term` variant is added and no signature in the pipeline grows to hold
//! them. `Term::Pi` in particular stays at three fields; it has hundreds of construction sites.
//!
//! | surface | emitted | consumer |
//! |---|---|---|
//! | `f (x := e)` | `f (MARK_named x e)` | the elaborator, resolving into declaration order |
//! | `@f a b` | `MARK_explicit f a b` | the elaborator, suppressing implicit insertion |
//! | `(m : T := e) -> B` | `Pi(m : MARK_default T e). B` | the declaration door, as a default |
//! | `(h : P := by tac) -> B` | `Pi(h : MARK_autoparam P tac). B` | the elaborator, per call site |
//! | `<a, ..>` / `f ..` | `MARK_anon a MARK_ellipsis` / `f MARK_ellipsis` | the elaborator |
//! | `{x : T}` / `⦃x : T⦄` / `[C x]` | `Pi(x : MARK_kind T). B` | read off as [`BinderKind`] |
//! | `fun {A} x => …` / `forall {A}, …` | `Lambda(A : MARK_kind _). …` | read off as [`BinderKind`] |
//!
//! The BINDER-KIND markers are the only ones that never leave this module: [`TermParser::parse`]
//! and every wrapper strips them and hands back the pure kernel `Pi`, after reading the telescope
//! into a [`BinderKind`] vector. A kind is a POSITION in the telescope, not a prefix count, which
//! is why it cannot ride on the `implicit_count` number the parser used to return: that number
//! could not say that the implicit stood third, and it counted a `{...}` written inside a
//! PARAMETER'S OWN TYPE as a parameter of the declaration.
//!
//! A marker must NEVER reach a kernel term. [`TermParser::as_named_arg`],
//! [`TermParser::as_default_param`], [`TermParser::as_auto_param`], [`TermParser::is_ellipsis`],
//! [`TermParser::strip_binder_markers`] and [`TermParser::substitute_param_ref`] are the whole
//! consumer interface, and [`scan_name`] refuses a guillemet name containing the marker brackets
//! so that no user global can spell one.
//!
//! # The declaration door
//!
//! A declaration is where every one of these has to be RECORDED, and reading them back off a
//! marked type is the part each consumer would otherwise reinvent. [`TermParser::read_declaration`]
//! is the one call: it hands back the marker-free [`DeclarationSurface::kernel_type`] the context
//! is keyed at, plus one [`BinderSurface`] per parameter carrying its kind, its default and its
//! auto-param tactic. [`TermParser::name_anonymous_binders`] then recovers the parameter NAMES
//! from the definition's body -- in LOGOS `A -> B` names nothing, so for a non-dependent signature
//! the body's lambda telescope is the only place those names exist -- and
//! [`TermParser::inline_default`] closes a default over the arguments already supplied.

use super::error::ParseError;
use crate::elaborate::ParamKind;
use crate::{Literal, Term, Universe};
use std::collections::{HashMap, HashSet};

use super::surface::Surface;

/// THE name grammar of the vernacular — the ONE both doors read.
///
/// A name is either a plain identifier (alphabetic or `_`, then alphanumerics, `_` and the PRIME
/// `'` — `h`, `h'`, `h''`, and Unicode throughout: `\u{3b1}`, `\u{393}`, `x\u{2081}`) or a
/// `\u{ab}guillemet name\u{bb}`, whose contents are taken verbatim so a name may hold spaces and
/// punctuation. It returns the KEY and the bytes consumed.
///
/// ★ ONE GRAMMAR, BOTH DOORS. `Definition <name> : …` used to take its name as a raw text slice up
/// to the first `:` while the term lexer demanded alphanumerics, so `Definition h' : Nat := Zero.`
/// succeeded and keyed the context at `h'` while `Check h'.` died in the lexer. Those were globals
/// that could be declared and never referenced — write-only names. The invariant this function
/// exists to enforce is: every name the declaration door accepts, a term can spell.
pub(super) fn scan_name(input: &str) -> Option<(String, usize)> {
    let mut chars = input.char_indices();
    match chars.next() {
        Some((_, '\u{ab}')) => {
            let open = '\u{ab}'.len_utf8();
            let end = input[open..].find('\u{bb}')?;
            let name = &input[open..open + end];
            // A GUILLEMET NAME MAY NOT SPELL A RESERVED MARKER. Every surface-sugar marker is
            // documented as collision-proof because "the identifier lexer cannot produce
            // \u{27e8}/\u{27e9}" -- but this verbatim form could, so `Definition \u{ab}\u{27e8}anon\u{27e9}\u{bb} : Nat := Zero.`
            // declared a global whose every later mention was then read as an anonymous
            // constructor: a write-only name again, by a different route. Excluding the two
            // bracket characters outright (exactly as `\u{ab}` is already excluded) makes the
            // stated invariant true by construction, and true for every marker added later.
            if name.is_empty()
                || name.contains('\u{ab}')
                || name.contains('\u{27e8}')
                || name.contains('\u{27e9}')
            {
                return None;
            }
            Some((name.to_string(), open + end + '\u{bb}'.len_utf8()))
        }
        Some((_, c)) if c.is_alphabetic() || c == '_' => {
            let mut end = c.len_utf8();
            for (i, c) in chars {
                if c.is_alphanumeric() || c == '_' || c == '\'' {
                    end = i + c.len_utf8();
                } else {
                    break;
                }
            }
            Some((input[..end].to_string(), end))
        }
        _ => None,
    }
}

/// [`scan_name`], also reporting whether the name was written in GUILLEMETS.
///
/// A quoted name is always a NAME: `«Prop»` is a global and `«fun»` a binder, where the same
/// letters unquoted are a sort and a keyword. That is what makes the guillemets an ESCAPE rather
/// than a second spelling of the same token — and it is the only way a printer can spell a constant
/// or a binder whose name happens to be a word this grammar reads as syntax.
pub(super) fn scan_spelled_name(input: &str) -> Option<(String, usize, bool)> {
    scan_name(input).map(|(name, len)| (name, len, input.starts_with('\u{ab}')))
}

/// The words the term grammar reserves — never a name when written bare. `Prop` and `Type` are
/// deliberately absent: they are SORTS (see [`is_sort_word`]).
pub(super) fn is_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "fun" | "forall" | "fix" | "match" | "let" | "in" | "with" | "return" | "end" | "by"
    )
}

/// Is `word`, written UNQUOTED, read as a SORT rather than a name? `Prop`, `SProp`, `Type`, `Sort`
/// and every `TypeN` whose suffix is all digits.
pub(super) fn is_sort_word(word: &str) -> bool {
    matches!(word, "Prop" | "SProp" | "Type" | "Sort")
        || word
            .strip_prefix("Type")
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// The deepest text the term door reads: four times the kernel's own term-depth ceiling, which
/// leaves room for every bracket and binder a printed or written term of admissible depth carries.
pub(super) const MAX_SYNTACTIC_DEPTH: usize = 4 * crate::type_checker::MAX_TERM_DEPTH;

/// Below this nesting a parse runs on the caller's own stack.
const INLINE_SYNTACTIC_DEPTH: usize = 48;

thread_local! {
    /// The stack this thread was given by [`on_stack_for`], or 0 when it was not.
    static SIZED_FOR: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// An upper bound on how deeply reading `input` can recurse.
///
/// The reader recurses once per open bracket and once per construct that extends to the right —
/// a binder keyword (`fun`, `forall`, `let`, `fix`, `match`) or an arrow — and such a construct
/// stays open until the bracket it stands in closes. So the depth at any point is, over the
/// enclosing brackets, one for each bracket plus one for each such construct already seen inside
/// it; this returns the largest. Strings and «quoted names» are skipped, as the reader skips them.
pub(super) fn syntactic_depth(input: &str) -> usize {
    let bytes = input.as_bytes();
    let mut opened: Vec<usize> = vec![0];
    let (mut depth, mut deepest) = (0usize, 0usize);
    let mut chars = input.char_indices().peekable();
    let word_at = |i: usize, w: &str| {
        input[i..].starts_with(w)
            && input[..i]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '\''))
            && input[i + w.len()..]
                .chars()
                .next()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '\''))
    };
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => {
                let mut escaped = false;
                for (_, c) in chars.by_ref() {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        break;
                    }
                }
            }
            '\u{ab}' => {
                for (_, c) in chars.by_ref() {
                    if c == '\u{bb}' {
                        break;
                    }
                }
            }
            '(' | '{' | '[' | '\u{27e8}' | '\u{2983}' => {
                opened.push(0);
                depth += 1;
            }
            ')' | '}' | ']' | '\u{27e9}' | '\u{2984}' => {
                if opened.len() > 1 {
                    let inner = opened.pop().expect("more than the root");
                    depth = depth.saturating_sub(1 + inner);
                }
            }
            '-' if bytes.get(i + 1) == Some(&b'>') => {
                *opened.last_mut().expect("the root") += 1;
                depth += 1;
            }
            c if c.is_alphabetic()
                && ["fun", "forall", "let", "fix", "match"]
                    .iter()
                    .any(|w| word_at(i, w)) =>
            {
                *opened.last_mut().expect("the root") += 1;
                depth += 1;
            }
            _ => {}
        }
        deepest = deepest.max(depth);
    }
    deepest
}

/// Run `f` on a stack sized for text nested `depth` deep — the caller's own when it is shallow or
/// already big enough, a scoped thread's otherwise. The kernel's own per-rung budget is used, which
/// is generous for the reader.
///
/// wasm32 has no threads to scope one on; the browser client links a stack already big enough for
/// the kernel's own worst case (see `term_store::run_on_stack`), so `f` runs inline there and the
/// sizing is just recorded for any nested call on the same (only) thread.
pub(super) fn on_stack_for<R: Send>(depth: usize, f: impl FnOnce() -> R + Send) -> R {
    let bytes = crate::term_store::stack_bytes_for_depth(depth);
    if depth <= INLINE_SYNTACTIC_DEPTH || SIZED_FOR.with(|s| s.get()) >= bytes {
        return f();
    }
    #[cfg(target_arch = "wasm32")]
    {
        SIZED_FOR.with(|s| s.set(bytes));
        f()
    }
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("logos-deep-surface".into())
            .stack_size(bytes)
            .spawn_scoped(scope, move || {
                SIZED_FOR.with(|s| s.set(bytes));
                f()
            })
            .expect("reserve a stack for a deep term")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// The largest `k` a level may add in one `ℓ+k`. A level is a chain of `Succ` nodes, so an
/// unbounded `k` would let a few bytes of source allocate gigabytes; no level any real development
/// writes comes near this, and a numeral names every `Type n` without a chain at all.
pub(super) const MAX_LEVEL_OFFSET: u64 = 1 << 16;

/// Days since the Unix epoch of the proleptic-Gregorian date `y-m-d` — Howard Hinnant's
/// `days_from_civil`, the exact inverse of the calendar conversion `Literal::Date` displays with.
pub(super) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The proleptic-Gregorian date of a day count — Hinnant's `civil_from_days`, over `i64`.
pub(super) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

/// How many nanoseconds one of each duration UNIT holds. A numeral's suffix is read WHOLE and looked
/// up here exactly, so `min` can never be misread as a shorter unit. The imperative language's own
/// suffix table, so a duration reads the same in both languages.
pub(super) const DURATION_UNITS: &[(&str, i64)] = &[
    ("min", 60_000_000_000),
    ("sec", 1_000_000_000),
    ("ns", 1),
    ("us", 1_000),
    ("\u{3bc}s", 1_000),
    ("ms", 1_000_000),
    ("hr", 3_600_000_000_000),
    ("s", 1_000_000_000),
    ("h", 3_600_000_000_000),
];

/// How a parameter is SUPPLIED at a call site -- Lean's four binder kinds.
///
/// The kernel has exactly one `Pi`, so a binder kind is not a typing distinction: `{A : Type} -> A`
/// and `(A : Type) -> A` and `⦃A : Type⦄ -> A` denote the same type and the same terms inhabit
/// it. What differs is what the ELABORATOR does at an application, and that is why the kind must
/// travel out of the parser beside the term rather than inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinderKind {
    /// `(x : T)` -- the caller writes the argument.
    Explicit,
    /// `{x : T}` -- inserted eagerly, solved by unification.
    Implicit,
    /// `⦃x : T⦄` (Lean's strict-implicit brackets) -- inserted only when a LATER explicit
    /// argument is actually supplied, so an unapplied `nilOf` stays the polymorphic function
    /// rather than becoming an application with an unsolvable metavariable in it.
    StrictImplicit,
    /// `[C x]` -- resolved from the instance database, after the explicit arguments have pinned
    /// the class's type variables.
    Instance,
}

impl BinderKind {
    /// The elaborator's [`ParamKind`] for this binder -- now a BIJECTION, one variant each.
    ///
    /// ★ THE LOSSY STEP IS GONE, AND THIS WAS THE ONE LINE. `ParamKind` had three variants and
    /// this enum four, so a strict implicit could only be collapsed onto `Implicit`. The collapse
    /// was made HERE, in one named function, rather than spelled at each hand-over site, against
    /// the day `ParamKind` grew its fourth variant. It has -- Lean's `BinderInfo` has exactly four
    /// constructors (`Lean/Expr.lean:69`) and `⦃⦄` earns its own because `App.lean:726`'s
    /// `processStrictImplicitArg` gives it a rule no other kind has: insert only
    /// `if (← hasArgsToProcess)`, otherwise `finalize`. `ParamKind::StrictImplicit` now exists and
    /// `elaborate_app_against` implements precisely that rule, so `⦃α⦄` reaches the elaborator as
    /// itself and this function is a BIJECTION with no information left to lose.
    /// `lean_conformance_t05_elaboration::a_strict_implicit_is_not_instantiated_until_an_explicit_argument_follows`
    /// is what pins the behaviour end to end.
    ///
    /// The legacy `implicit_count` model still STOPS at a strict implicit
    /// (`count_implicits`): a count can only describe an
    /// unconditional prefix, and over-inserting there emits a metavariable nothing can solve.
    pub fn as_param_kind(self) -> ParamKind {
        match self {
            BinderKind::Explicit => ParamKind::Explicit,
            BinderKind::Implicit => ParamKind::Implicit,
            BinderKind::StrictImplicit => ParamKind::StrictImplicit,
            BinderKind::Instance => ParamKind::Instance,
        }
    }

    /// The binder kind for an elaborator [`ParamKind`] -- the inverse of
    /// [`as_param_kind`](Self::as_param_kind).
    pub fn of_param_kind(kind: ParamKind) -> BinderKind {
        match kind {
            ParamKind::Explicit => BinderKind::Explicit,
            ParamKind::Implicit => BinderKind::Implicit,
            ParamKind::StrictImplicit => BinderKind::StrictImplicit,
            ParamKind::Instance => BinderKind::Instance,
        }
    }

    /// The reserved marker this kind rides on inside the parser, or `None` for `Explicit` --
    /// which is the absence of a marker, so that an ordinary `A -> B` costs nothing.
    fn marker(self) -> Option<&'static str> {
        match self {
            BinderKind::Explicit => None,
            BinderKind::Implicit => Some(TermParser::IMPLICIT_BINDER_MARKER),
            BinderKind::StrictImplicit => Some(TermParser::STRICT_IMPLICIT_BINDER_MARKER),
            BinderKind::Instance => Some(TermParser::INSTANCE_BINDER_MARKER),
        }
    }
}

/// Everything the SURFACE says about ONE parameter that the kernel's three-field `Term::Pi`
/// cannot hold: how it is supplied, and what fills it when the caller does not.
#[derive(Debug, Clone, PartialEq)]
pub struct BinderSurface {
    /// The parameter's name. `"_"` for an anonymous binder -- `A -> B` names nothing, and
    /// [`TermParser::name_anonymous_binders`] recovers those names from the definition's body.
    pub name: String,
    /// How the caller supplies it.
    pub kind: BinderKind,
    /// The declared type, with every marker stripped -- the type the kernel sees.
    pub declared: Term,
    /// `(m : T := e)` -- the term inlined when the caller omits this argument.
    pub default: Option<Term>,
    /// `(h : P := by tac)` -- the tactic that must prove `declared` AFRESH at each call site.
    /// Distinct from `default` because a default is inlined once per omission and an auto-param
    /// is a different proof term at every call site, about that call site's own proposition.
    pub auto: Option<String>,
}

/// A declared type split into the pure kernel type the context is keyed at and the per-parameter
/// surface information that has to be recorded beside it.
///
/// ★ ONE CALL AT THE DECLARATION DOOR. Before this, the declaration door read a `(Term, usize)`
/// pair and there was nowhere for a default, an auto-param or a non-leading implicit to go -- so
/// `Definition f : Int -> (m : Int := 3) -> Int` registered a type with a reserved marker still
/// inside it, which the kernel then refused as an unbound global. Every piece of surface-only
/// information now leaves the parser through this one struct.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclarationSurface {
    /// The declared type with every binder marker stripped: what `Context` is keyed at.
    pub kernel_type: Term,
    /// One entry per binder of the type's top-level telescope, outermost first.
    pub binders: Vec<BinderSurface>,
}

impl DeclarationSurface {
    /// The elaborator's parameter kinds, one per telescope position.
    pub fn param_kinds(&self) -> Vec<ParamKind> {
        self.binders
            .iter()
            .map(|b| b.kind.as_param_kind())
            .collect()
    }

    /// How many LEADING implicit parameters the declaration has -- the legacy count model, and
    /// exactly the number [`TermParser::parse_with_implicits`] reports.
    pub fn leading_implicits(&self) -> usize {
        self.binders
            .iter()
            .take_while(|b| b.kind == BinderKind::Implicit)
            .count()
    }

    /// Does any binder carry surface information the plain `(Term, usize)` door would lose?
    /// A declaration for which this is false can be registered exactly as it always was.
    pub fn needs_recording(&self) -> bool {
        self.binders
            .iter()
            .any(|b| b.kind != BinderKind::Explicit || b.default.is_some() || b.auto.is_some())
    }
}

/// Recursive descent parser for Term syntax.
pub struct TermParser<'a> {
    input: &'a str,
    pos: usize,
    /// Variables currently in scope (bound by lambda, forall, fix, or match case)
    bound_vars: HashSet<String>,
    /// The user-declared notation in scope, if any. `None` is not a special case: a surface with
    /// nothing declared takes exactly the same path, so the extension point costs nothing when it
    /// is unused.
    surface: Option<&'a Surface>,
}

impl<'a> TermParser<'a> {
    /// Reserved head marking a NAMED ARGUMENT `(x := e)` at an argument position.
    ///
    /// The parser emits `MARKER x e` -- the parameter name carried as a `Global`, exactly as
    /// [`crate::elaborate::DOT_MARKER`] carries a field name. The elaborator resolves it into the
    /// declaration's parameter order and discards the marker; it NEVER reaches a kernel term.
    pub const NAMED_ARG_MARKER: &'static str = "\u{27e8}named\u{27e9}";

    /// Reserved head wrapping a binder's DECLARED TYPE when the binder carries a DEFAULT value:
    /// `(m : Int := 3)` becomes `Pi(m : MARKER Int 3). ...`.
    ///
    /// The binder stays an ordinary `Term::Pi` and the default travels inside its `param_type`,
    /// so the default reaches the declaration door through every existing pipe with no signature
    /// change anywhere -- and a consumer that forgets to strip it fails LOUDLY on an unbound
    /// global rather than silently dropping the default.
    pub const PARAM_DEFAULT_MARKER: &'static str = "\u{27e8}default\u{27e9}";

    /// Reserved head wrapping a binder's declared type when the binder is TACTIC-FILLED:
    /// `(h : P := by decide)` becomes `Pi(h : MARKER P decide). ...`, the tactic name carried as
    /// a `Global`. Distinct from [`Self::PARAM_DEFAULT_MARKER`] because the two are discharged
    /// differently: a default is a TERM inlined once per omission, an auto-param is a PROOF
    /// searched afresh at every call site, about that call site's own proposition.
    pub const AUTO_PARAM_MARKER: &'static str = "\u{27e8}autoparam\u{27e9}";

    /// Reserved head marking an EXPLICIT-ALL application `@f` -- Lean's escape hatch, and the
    /// one the language had no spelling for at all.
    ///
    /// ★ WITHOUT IT AN IMPLICIT ARGUMENT CANNOT BE WRITTEN. Once a declaration's binder kinds are
    /// recorded, `idf Nat Zero` is not "supply the implicit and the explicit": the elaborator
    /// inserts a metavariable for the implicit position FIRST and then has two arguments for one
    /// explicit parameter. `@` is the only way to say "I am writing every parameter myself", which
    /// is exactly what a user reaches for when inference has failed -- so a language without it
    /// has no escape hatch on the day it is needed most.
    ///
    /// `@f a b` is emitted as `MARKER f a b`: the marker is the SPINE HEAD, so a consumer
    /// decomposing an application sees it in the position it already inspects, and the head it
    /// marks is `args[0]`. [`TermParser::is_explicit_all_head`] and
    /// [`TermParser::as_explicit_all`] are the whole reading interface.
    pub const EXPLICIT_ALL_MARKER: &'static str = "\u{27e8}explicit\u{27e9}";

    /// Reserved atom marking the ELLIPSIS `..` -- "fill every remaining position with `_`".
    ///
    /// Only the elaborator knows the arity of the thing being applied, so the parser records the
    /// REQUEST and the elaborator expands it into that many solved holes. Emitted as the last
    /// component of `<a, b, ..>` and as the last argument of `f ..`.
    pub const ELLIPSIS_MARKER: &'static str = "\u{27e8}..\u{27e9}";

    /// Reserved head wrapping a binder's declared type when the binder is IMPLICIT:
    /// `{A : Type} -> B` becomes `Pi(A : MARKER Type). B`.
    ///
    /// Unlike the other three, a kind marker never leaves this module: every public entry point
    /// reads the telescope's kinds off it and then strips it, so the emitted term is the same
    /// pure `Term::Pi` an explicit binder produces. It exists because a binder KIND is a
    /// position in a telescope, and the only structure that survives the recursive descent in
    /// telescope order is the telescope itself.
    pub const IMPLICIT_BINDER_MARKER: &'static str = "\u{27e8}implicit\u{27e9}";

    /// Reserved head wrapping a STRICT-IMPLICIT binder's declared type: `⦃A : Type⦄ -> B`.
    /// Distinct from [`Self::IMPLICIT_BINDER_MARKER`] because Lean inserts the two at different
    /// moments, and collapsing them at the parser would make the difference unrecoverable.
    pub const STRICT_IMPLICIT_BINDER_MARKER: &'static str = "\u{27e8}strict\u{27e9}";

    /// Reserved head wrapping an INSTANCE binder's class type: `[Inhabited A] -> B`.
    pub const INSTANCE_BINDER_MARKER: &'static str = "\u{27e8}instance\u{27e9}";

    /// Lean's ASCII spelling of the strict-implicit OPEN bracket `⦃` -- `{{`.
    ///
    /// `Lean/Parser/Term.lean:346-347` defines both brackets as an alternation, guillemet or
    /// doubled brace, and `:377` documents them as one binder with two spellings: "A strict
    /// implicit binder, `⦃y z : A⦄` or its ASCII alternative `{{y z : A}}`". The ASCII one is the
    /// spelling available to a writer with no unicode input method, so refusing it made Lean's
    /// fourth binder kind unreachable in practice from an ordinary keyboard.
    pub const STRICT_IMPLICIT_OPEN_ASCII: &'static str = "{{";
    /// Lean's ASCII spelling of the strict-implicit CLOSE bracket `⦄` -- `}}`.
    /// See [`STRICT_IMPLICIT_OPEN_ASCII`](Self::STRICT_IMPLICIT_OPEN_ASCII).
    pub const STRICT_IMPLICIT_CLOSE_ASCII: &'static str = "}}";

    /// `BinderKind::Explicit`, nameable without naming the enum.
    pub const EXPLICIT: BinderKind = BinderKind::Explicit;
    /// `BinderKind::Implicit`, nameable without naming the enum.
    pub const IMPLICIT: BinderKind = BinderKind::Implicit;
    /// `BinderKind::StrictImplicit`, nameable without naming the enum.
    pub const STRICT_IMPLICIT: BinderKind = BinderKind::StrictImplicit;
    /// `BinderKind::Instance`, nameable without naming the enum.
    pub const INSTANCE: BinderKind = BinderKind::Instance;

    /// Recognize a NAMED ARGUMENT, returning the parameter name and the supplied value.
    ///
    /// The spine is decomposed rather than pattern-matched on a fixed depth, so a named argument
    /// whose value is itself an application (`(x := f y)`) is recognized too.
    pub fn as_named_arg(term: &Term) -> Option<(&str, &Term)> {
        match term {
            Term::App(f, value) => match &**f {
                Term::App(marker, name) => match (&**marker, &**name) {
                    (Term::Global(m), Term::Global(n)) if m == Self::NAMED_ARG_MARKER => {
                        Some((n.as_str(), value.as_ref()))
                    }
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    /// Is this term the ELLIPSIS marker?
    pub fn is_ellipsis(term: &Term) -> bool {
        matches!(term, Term::Global(n) if n == Self::ELLIPSIS_MARKER)
    }

    /// Is this the HEAD of an explicit-all application -- the thing a decomposed spine's innermost
    /// position holds for `@f a b`? The head `f` is then that spine's first argument.
    pub fn is_explicit_all_head(term: &Term) -> bool {
        matches!(term, Term::Global(n) if n == Self::EXPLICIT_ALL_MARKER)
    }

    /// Recognize an explicit-all application `@f`, returning the head it marks. Only the
    /// UNAPPLIED shape; an applied `@f a b` is read off the spine with
    /// [`is_explicit_all_head`](Self::is_explicit_all_head), because that is the form the
    /// elaborator's application arm already has in hand.
    pub fn as_explicit_all(term: &Term) -> Option<&Term> {
        match term {
            Term::App(marker, head) if Self::is_explicit_all_head(marker) => Some(head.as_ref()),
            _ => None,
        }
    }

    /// Recognize a DEFAULTED binder's `param_type`, returning the declared type and the default
    /// value the caller may omit. A binder may be BOTH defaulted and non-explicit
    /// (`{m : Int := 3}`), so the kind marker is peeled first -- reading only the outermost
    /// marker would have made a defaulted implicit look like an ordinary one and dropped its
    /// default in silence.
    pub fn as_default_param(param_type: &Term) -> Option<(&Term, &Term)> {
        Self::as_marked_binder(Self::peel_kind(param_type), Self::PARAM_DEFAULT_MARKER)
    }

    /// Recognize an AUTO-PARAM binder's `param_type`, returning the declared type (the obligation)
    /// and the name of the tactic that must discharge it at each call site.
    pub fn as_auto_param(param_type: &Term) -> Option<(&Term, &str)> {
        match Self::as_marked_binder(Self::peel_kind(param_type), Self::AUTO_PARAM_MARKER)? {
            (declared, Term::Global(tactic)) => Some((declared, tactic.as_str())),
            _ => None,
        }
    }

    /// Recognize a KIND-marked binder type, returning the kind and the type inside it.
    ///
    /// The shape is `MARKER declared` -- one argument, where the other two binder markers carry
    /// two -- so no marker can ever be mistaken for another.
    fn as_kind_marked(param_type: &Term) -> Option<(BinderKind, &Term)> {
        let Term::App(f, declared) = param_type else {
            return None;
        };
        let Term::Global(m) = &**f else { return None };
        for kind in [
            BinderKind::Implicit,
            BinderKind::StrictImplicit,
            BinderKind::Instance,
        ] {
            if kind.marker() == Some(m.as_str()) {
                return Some((kind, declared.as_ref()));
            }
        }
        None
    }

    /// The type inside a kind marker, or the type itself when there is none.
    fn peel_kind(param_type: &Term) -> &Term {
        Self::as_kind_marked(param_type)
            .map(|(_, d)| d)
            .unwrap_or(param_type)
    }

    /// Wrap a binder's declared type in the marker for `kind`. `Explicit` is the ABSENCE of a
    /// marker, so an ordinary `A -> B` allocates nothing and reads exactly as it always did.
    fn mark_kind(kind: BinderKind, declared: Term) -> Term {
        match kind.marker() {
            None => declared,
            Some(m) => Term::App(Box::new(Term::Global(m.to_string())), Box::new(declared)),
        }
    }

    /// The BINDER KINDS of a type's top-level telescope, outermost first -- one entry per `Pi`,
    /// `Explicit` where no kind marker stands.
    ///
    /// ★ A KIND IS A POSITION, NOT A COUNT. A marker written inside a PARAMETER'S OWN TYPE
    /// (`{A : Type} -> ({B : Type} -> B) -> A`) lives under that parameter's `param_type` and is
    /// therefore not on this spine at all -- which is precisely the distinction the running
    /// `implicit_count` could not make, and why it reported two parameters for a declaration
    /// that has one.
    pub fn binder_kinds_of(ty: &Term) -> Vec<BinderKind> {
        let mut kinds = Vec::new();
        let mut cur = ty;
        loop {
            // ★ A LAMBDA IS A BINDER TELESCOPE TOO. `fun {A} (a : A) => a` declares that its
            // FIRST argument is supplied by unification, exactly as `{A : Type} -> A -> A` does,
            // and in LOGOS an un-ascribed `Definition` takes its whole telescope from the body --
            // so a walk that stopped at `Pi` could not report the kinds of the only spelling that
            // has them. An unmarked binder reads `Explicit` either way, so every term that had no
            // kind marker in it answers exactly as it always did.
            // A binder that carries its kind as an annotation (`Term::Annotated`) answers with it,
            // exactly as a marked domain does.
            let annotated = cur.binder_kind();
            let (param_type, rest) = match cur.peel_annotations() {
                Term::Pi {
                    param_type,
                    body_type,
                    ..
                } => (param_type, body_type),
                Term::Lambda {
                    param_type, body, ..
                } => (param_type, body),
                _ => break,
            };
            kinds.push(
                Self::as_kind_marked(param_type)
                    .map(|(k, _)| k)
                    .or_else(|| annotated.map(BinderKind::of_param_kind))
                    .unwrap_or(BinderKind::Explicit),
            );
            cur = rest;
        }
        kinds
    }

    /// The shared shape of both binder markers: `MARKER declared payload`.
    fn as_marked_binder<'t>(term: &'t Term, marker: &str) -> Option<(&'t Term, &'t Term)> {
        match term {
            Term::App(f, payload) => match &**f {
                Term::App(m, declared) => match &**m {
                    Term::Global(m) if m == marker => Some((declared.as_ref(), payload.as_ref())),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    /// The same term with every BINDER marker removed -- the pure kernel type a declaration is
    /// registered at, once the defaults have been read off it.
    ///
    /// Argument-position sugar (a named argument, an ellipsis) is deliberately NOT stripped: those
    /// carry a request the elaborator has to ACT on, and silently dropping one would change what
    /// the user wrote into something that merely typechecks. Only the decoration on a binder's
    /// type is erased here.
    pub fn strip_binder_markers(term: &Term) -> Term {
        if let Some(inner) = Self::peel_binder_marker(term) {
            return Self::strip_binder_markers(inner);
        }
        Self::strip_binder_markers_inner(term)
    }

    /// The same term with every BINDER-KIND marker removed and nothing else touched -- the pure
    /// kernel `Pi` telescope, with a defaulted or tactic-filled binder still carrying its payload
    /// for the declaration door to read.
    pub fn strip_kind_markers(term: &Term) -> Term {
        let mut owned = term.clone();
        Self::strip_kind_markers_in_place(&mut owned);
        owned
    }

    /// [`strip_kind_markers`](Self::strip_kind_markers) on a term the caller already owns.
    ///
    /// Every parse goes through this, and almost no term has a kind marker in it -- so the walk
    /// must not COPY the tree it is about to hand back untouched. The `clone` below runs only
    /// where a marker actually stands; the traversal itself allocates nothing.
    fn strip_kind_markers_in_place(term: &mut Term) {
        while let Some(inner) = Self::as_kind_marked(term).map(|(_, d)| d.clone()) {
            *term = inner;
        }
        match term {
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                Self::strip_kind_markers_in_place(param_type);
                Self::strip_kind_markers_in_place(body_type);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                Self::strip_kind_markers_in_place(param_type);
                Self::strip_kind_markers_in_place(body);
            }
            Term::App(f, a) => {
                Self::strip_kind_markers_in_place(f);
                Self::strip_kind_markers_in_place(a);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                Self::strip_kind_markers_in_place(discriminant);
                Self::strip_kind_markers_in_place(motive);
                for c in cases {
                    Self::strip_kind_markers_in_place(c);
                }
            }
            Term::Fix { body, .. } => Self::strip_kind_markers_in_place(body),
            Term::MutualFix { defs, .. } => {
                for (_, b) in defs {
                    Self::strip_kind_markers_in_place(b);
                }
            }
            Term::Let {
                ty, value, body, ..
            } => {
                Self::strip_kind_markers_in_place(ty);
                Self::strip_kind_markers_in_place(value);
                Self::strip_kind_markers_in_place(body);
            }
            Term::Annotated { term, .. } => Self::strip_kind_markers_in_place(term),
            _ => {}
        }
    }

    /// One layer of every binder marker: the kind wrapper, then the default or auto-param
    /// wrapper inside it.
    fn peel_binder_marker(term: &Term) -> Option<&Term> {
        if let Some((_, declared)) = Self::as_kind_marked(term) {
            return Some(declared);
        }
        if let Some((declared, _)) = Self::as_marked_binder(term, Self::PARAM_DEFAULT_MARKER) {
            return Some(declared);
        }
        if let Some((declared, _)) = Self::as_marked_binder(term, Self::AUTO_PARAM_MARKER) {
            return Some(declared);
        }
        None
    }

    fn strip_binder_markers_inner(term: &Term) -> Term {
        match term {
            Term::Pi {
                param,
                param_type,
                body_type,
            } => Term::Pi {
                param: param.clone(),
                param_type: Box::new(Self::strip_binder_markers(param_type)),
                body_type: Box::new(Self::strip_binder_markers(body_type)),
            },
            Term::Lambda {
                param,
                param_type,
                body,
            } => Term::Lambda {
                param: param.clone(),
                param_type: Box::new(Self::strip_binder_markers(param_type)),
                body: Box::new(Self::strip_binder_markers(body)),
            },
            Term::App(f, a) => Term::App(
                Box::new(Self::strip_binder_markers(f)),
                Box::new(Self::strip_binder_markers(a)),
            ),
            Term::Match {
                discriminant,
                motive,
                cases,
            } => Term::Match {
                discriminant: Box::new(Self::strip_binder_markers(discriminant)),
                motive: Box::new(Self::strip_binder_markers(motive)),
                cases: cases
                    .iter()
                    .map(|c| Self::strip_binder_markers(c))
                    .collect(),
            },
            Term::Fix { name, body } => Term::Fix {
                name: name.clone(),
                body: Box::new(Self::strip_binder_markers(body)),
            },
            Term::MutualFix { defs, index } => Term::MutualFix {
                defs: defs
                    .iter()
                    .map(|(n, b)| (n.clone(), Self::strip_binder_markers(b)))
                    .collect(),
                index: *index,
            },
            Term::Let {
                name,
                ty,
                value,
                body,
            } => Term::Let {
                name: name.clone(),
                ty: Box::new(Self::strip_binder_markers(ty)),
                value: Box::new(Self::strip_binder_markers(value)),
                body: Box::new(Self::strip_binder_markers(body)),
            },
            Term::Annotated { annotation, term } => Term::Annotated {
                annotation: annotation.clone(),
                term: Box::new(Self::strip_binder_markers(term)),
            },
            other => other.clone(),
        }
    }

    /// Substitute a declaration's ACTUAL argument for a reference to parameter `name` inside a
    /// DEFAULT value -- `(m : Int := mul n 2)` applied as `sized 5` inlines `mul 5 2`.
    ///
    /// A default is parsed in the scope of the binders before it, so a reference to a NAMED
    /// earlier binder is already a `Term::Var` and is substituted capture-avoidingly (an inner
    /// binder of the same name shadows it, and substitution stops). A reference to an ANONYMOUS
    /// earlier binder -- `Int -> (m : Int := mul n 2) -> Int`, where position 0's `Pi` binder is
    /// `"_"` and the name `n` exists only in the definition body's lambda telescope -- parsed as
    /// a `Term::Global`, and a `Global` is not shadowed by any binder, so it is replaced
    /// unconditionally. Handling both is what keeps an inlined default from escaping as an open
    /// term with a free `n` in it, which is the way this goes wrong.
    pub fn substitute_param_ref(term: &Term, name: &str, value: &Term) -> Term {
        match term {
            Term::Var(v) if v == name => value.clone(),
            Term::Global(g) if g == name => value.clone(),
            Term::Pi {
                param,
                param_type,
                body_type,
            } => Term::Pi {
                param: param.clone(),
                param_type: Box::new(Self::substitute_param_ref(param_type, name, value)),
                body_type: Box::new(if param == name {
                    Self::substitute_global_only(body_type, name, value)
                } else {
                    Self::substitute_param_ref(body_type, name, value)
                }),
            },
            Term::Lambda {
                param,
                param_type,
                body,
            } => Term::Lambda {
                param: param.clone(),
                param_type: Box::new(Self::substitute_param_ref(param_type, name, value)),
                body: Box::new(if param == name {
                    Self::substitute_global_only(body, name, value)
                } else {
                    Self::substitute_param_ref(body, name, value)
                }),
            },
            Term::App(f, a) => Term::App(
                Box::new(Self::substitute_param_ref(f, name, value)),
                Box::new(Self::substitute_param_ref(a, name, value)),
            ),
            Term::Match {
                discriminant,
                motive,
                cases,
            } => Term::Match {
                discriminant: Box::new(Self::substitute_param_ref(discriminant, name, value)),
                motive: Box::new(Self::substitute_param_ref(motive, name, value)),
                cases: cases
                    .iter()
                    .map(|c| Self::substitute_param_ref(c, name, value))
                    .collect(),
            },
            Term::Fix { name: f, body } => Term::Fix {
                name: f.clone(),
                body: Box::new(if f == name {
                    Self::substitute_global_only(body, name, value)
                } else {
                    Self::substitute_param_ref(body, name, value)
                }),
            },
            Term::MutualFix { defs, index } => Term::MutualFix {
                defs: defs
                    .iter()
                    .map(|(n, b)| {
                        let rebound = defs.iter().any(|(other, _)| other == name);
                        let b = if rebound {
                            Self::substitute_global_only(b, name, value)
                        } else {
                            Self::substitute_param_ref(b, name, value)
                        };
                        (n.clone(), b)
                    })
                    .collect(),
                index: *index,
            },
            Term::Let {
                name: l,
                ty,
                value: v,
                body,
            } => Term::Let {
                name: l.clone(),
                ty: Box::new(Self::substitute_param_ref(ty, name, value)),
                value: Box::new(Self::substitute_param_ref(v, name, value)),
                body: Box::new(if l == name {
                    Self::substitute_global_only(body, name, value)
                } else {
                    Self::substitute_param_ref(body, name, value)
                }),
            },
            Term::Annotated { annotation, term } => Term::Annotated {
                annotation: annotation.clone(),
                term: Box::new(Self::substitute_param_ref(term, name, value)),
            },
            other => other.clone(),
        }
    }

    /// The `Global`-only half of [`Self::substitute_param_ref`], used under a binder that shadows
    /// `name`: the local `Var` there is a DIFFERENT variable and must survive untouched, while a
    /// `Global` of that spelling still denotes the parameter.
    fn substitute_global_only(term: &Term, name: &str, value: &Term) -> Term {
        match term {
            Term::Var(_) => term.clone(),
            Term::Global(g) if g == name => value.clone(),
            Term::Pi {
                param,
                param_type,
                body_type,
            } => Term::Pi {
                param: param.clone(),
                param_type: Box::new(Self::substitute_global_only(param_type, name, value)),
                body_type: Box::new(Self::substitute_global_only(body_type, name, value)),
            },
            Term::Lambda {
                param,
                param_type,
                body,
            } => Term::Lambda {
                param: param.clone(),
                param_type: Box::new(Self::substitute_global_only(param_type, name, value)),
                body: Box::new(Self::substitute_global_only(body, name, value)),
            },
            Term::App(f, a) => Term::App(
                Box::new(Self::substitute_global_only(f, name, value)),
                Box::new(Self::substitute_global_only(a, name, value)),
            ),
            Term::Match {
                discriminant,
                motive,
                cases,
            } => Term::Match {
                discriminant: Box::new(Self::substitute_global_only(discriminant, name, value)),
                motive: Box::new(Self::substitute_global_only(motive, name, value)),
                cases: cases
                    .iter()
                    .map(|c| Self::substitute_global_only(c, name, value))
                    .collect(),
            },
            Term::Fix { name: f, body } => Term::Fix {
                name: f.clone(),
                body: Box::new(Self::substitute_global_only(body, name, value)),
            },
            Term::MutualFix { defs, index } => Term::MutualFix {
                defs: defs
                    .iter()
                    .map(|(n, b)| (n.clone(), Self::substitute_global_only(b, name, value)))
                    .collect(),
                index: *index,
            },
            Term::Let {
                name: l,
                ty,
                value: v,
                body,
            } => Term::Let {
                name: l.clone(),
                ty: Box::new(Self::substitute_global_only(ty, name, value)),
                value: Box::new(Self::substitute_global_only(v, name, value)),
                body: Box::new(Self::substitute_global_only(body, name, value)),
            },
            Term::Annotated { annotation, term } => Term::Annotated {
                annotation: annotation.clone(),
                term: Box::new(Self::substitute_global_only(term, name, value)),
            },
            other => other.clone(),
        }
    }

    /// The byte index of the first `sep` at TOP LEVEL of `input` -- outside every bracket, outside
    /// a guillemet name, and outside a string literal -- or `None` if there is none.
    ///
    /// WHY THE VERNACULAR NEEDS THIS. `Definition <name> : <type> := <body>.` is split by searching
    /// for `:=`, and the FIRST `:=` in the text is not always the separator. A defaulted binder puts
    /// one inside the TYPE (`Definition f : Int -> (m : Int := 3) -> Int := ...` splits at the
    /// default and the command is refused for a reason that names neither the default nor the
    /// body), and a guillemet name puts one inside the NAME (`Definition \u{ab}a := b\u{bb} : Nat := Zero.`).
    /// The same applies to the `:` that opens the type ascription. The lexical structure of a term
    /// is this parser's knowledge, so the scan lives here rather than being re-derived -- once per
    /// bug -- by every door that has to split a sentence.
    ///
    /// `sep` must not begin with a bracket, a quote or a guillemet: those characters are what the
    /// scan is tracking, so a separator starting with one could never be at top level anyway.
    pub fn find_top_level(input: &str, sep: &str) -> Option<usize> {
        if sep.is_empty() {
            return None;
        }
        let mut depth = 0usize;
        let mut chars = input.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                // A string literal is opaque, escapes included -- `"a := b"` holds no separator.
                '"' => {
                    let mut escaped = false;
                    for (_, c) in chars.by_ref() {
                        if escaped {
                            escaped = false;
                        } else if c == '\\' {
                            escaped = true;
                        } else if c == '"' {
                            break;
                        }
                    }
                    continue;
                }
                // A guillemet name is verbatim and cannot nest (`scan_name` forbids an inner one).
                '\u{ab}' => {
                    for (_, c) in chars.by_ref() {
                        if c == '\u{bb}' {
                            break;
                        }
                    }
                    continue;
                }
                '(' | '{' | '[' | '\u{27e8}' | '\u{2983}' => {
                    depth += 1;
                    continue;
                }
                // An unbalanced closer cannot take the depth negative: the sentence is malformed
                // and the parser will say so, but this scan must still terminate and stay at 0.
                ')' | '}' | ']' | '\u{27e9}' | '\u{2984}' => {
                    depth = depth.saturating_sub(1);
                    continue;
                }
                _ => {}
            }
            if depth == 0 && input[i..].starts_with(sep) {
                return Some(i);
            }
        }
        None
    }

    /// Parse a term from input string.
    pub fn parse(input: &'a str) -> Result<Term, ParseError> {
        Self::parse_with_implicits(input).map(|(t, _)| t)
    }

    /// Parse a term through a SURFACE: macros are expanded to a fixpoint first, then the notation
    /// in scope is read as part of the grammar.
    ///
    /// Macro expansion is source-to-source and runs BEFORE the parse, because an alternative may
    /// invoke the macro it defines; notation is a parser layer, because a notation has a
    /// precedence and text has none. That is the whole staging, and it is the staging Lean uses.
    pub fn parse_in(surface: &Surface, input: &str) -> Result<Term, ParseError> {
        Self::parse_with_implicits_in(surface, input).map(|(t, _)| t)
    }

    /// [`parse_in`](Self::parse_in), also reporting the leading implicit-binder count.
    pub fn parse_with_implicits_in(
        surface: &Surface,
        input: &str,
    ) -> Result<(Term, usize), ParseError> {
        Self::entry(Some(surface), input, Self::finish_implicits)
    }

    /// [`parse`](Self::parse), also reporting the BINDER KIND of every parameter of the parsed
    /// type's top-level telescope -- the vector `Context::set_binder_kinds` is keyed at.
    ///
    /// This is the honest successor to [`parse_with_implicits`](Self::parse_with_implicits): a
    /// count can only describe a PREFIX of implicits, so `Nat -> {A : Type} -> A` -- an implicit
    /// in second position, which Lean writes constantly -- has no number that describes it.
    pub fn parse_with_binder_kinds(input: &str) -> Result<(Term, Vec<BinderKind>), ParseError> {
        Self::entry(None, input, Self::finish_kinds)
    }

    /// [`parse_with_binder_kinds`](Self::parse_with_binder_kinds), through a [`Surface`].
    pub fn parse_with_binder_kinds_in(
        surface: &Surface,
        input: &str,
    ) -> Result<(Term, Vec<BinderKind>), ParseError> {
        Self::entry(Some(surface), input, Self::finish_kinds)
    }

    /// Parse a DECLARED TYPE into the kernel type it is registered at plus the surface
    /// information that has to be recorded beside it -- binder kinds, default values and
    /// auto-param obligations, in telescope order.
    ///
    /// The one call the declaration door needs. See [`DeclarationSurface`].
    pub fn read_declaration(input: &str) -> Result<DeclarationSurface, ParseError> {
        Self::entry(None, input, |t| Self::declaration_surface(&t))
    }

    /// [`read_declaration`](Self::read_declaration), through a [`Surface`].
    pub fn read_declaration_in(
        surface: &Surface,
        input: &str,
    ) -> Result<DeclarationSurface, ParseError> {
        Self::entry(Some(surface), input, |t| Self::declaration_surface(&t))
    }

    /// A DECLARED TYPE as the declaration door must carry it: the parsed type with its
    /// binder-kind markers STILL ON, beside the leading-implicit count the legacy model reads.
    ///
    /// ★ THE CHANNEL BETWEEN THE TWO HALVES WAS A `usize`, AND MATHLIB DOES NOT FIT THROUGH IT.
    /// The elaborator has read `Context::binder_kinds` -- one `ParamKind` per POSITION -- ever
    /// since `full_param_kinds` and the kinds match in `elab_surface_uncoerced` were written, and
    /// this file has parsed all four of Lean's brackets since the binder grammar landed. Between
    /// them sat [`parse_with_implicits_in`](Self::parse_with_implicits_in), which STRIPS the kind
    /// markers and reports a number. A declaration door reading its type through that one cannot
    /// recover a kind afterwards: [`declaration_surface`](Self::declaration_surface) then reads
    /// `Explicit` for every binder, correctly, because by then the type no longer says anything
    /// else. So `Nat -> {A : Type} -> A -> A` registered THREE EXPLICIT parameters -- the count
    /// stops at the first non-leading binder and describes only the prefix -- `⦃A : Type⦄` became
    /// an argument the caller now has to write, and `[Inhabited A]` stopped consulting the
    /// instance database at all. `theorem foo {α} [Monoid α] (a : α)`, the shape of very nearly
    /// every declaration in mathlib, had no working spelling.
    ///
    /// This door keeps the markers on, so the call the declaration door already makes --
    /// `declaration_surface`, which reads `as_kind_marked` and whose `kernel_type` comes back
    /// fully stripped -- returns the truth instead of a flattened telescope. The count is
    /// reported unchanged beside it, so the leading-implicit model that lives next to it is not
    /// disturbed.
    ///
    /// ★ THE FIRST COMPONENT IS NOT A KERNEL TERM. Its parameter types are marker applications,
    /// and every marker is an undeclared global the kernel refuses by name
    /// (`no_reserved_marker_is_inhabited_so_a_forgotten_one_fails_loudly`) -- which is the point:
    /// a door that forgets to split gets `UnboundVariable`, not a term that typechecks by
    /// accident. Take [`DeclarationSurface::kernel_type`] before registering anything.
    pub fn read_declared_type_in(
        surface: &Surface,
        input: &str,
    ) -> Result<(Term, usize), ParseError> {
        Self::entry(Some(surface), input, Self::count_implicits)
    }

    /// [`read_declared_type_in`](Self::read_declared_type_in), with no [`Surface`] in scope.
    pub fn read_declared_type(input: &str) -> Result<(Term, usize), ParseError> {
        Self::entry(None, input, Self::count_implicits)
    }

    /// Split a declared Pi type or an inferred definition's lambda telescope into its
    /// marker-free term and per-binder surface.
    ///
    /// Safe to call on a type that has been through
    /// [`parse_with_implicits`](Self::parse_with_implicits): the binder KINDS are stripped by
    /// that door and every binder then reads as `Explicit`, while the defaults and auto-params
    /// -- which that door deliberately leaves in place -- are still recovered.
    pub fn declaration_surface(ty: &Term) -> DeclarationSurface {
        let mut binders = Vec::new();
        let mut cur = ty;
        let lambda_telescope = matches!(ty, Term::Lambda { .. });
        loop {
            let (param, param_type, body_type) = match cur {
                Term::Pi {
                    param,
                    param_type,
                    body_type,
                } if !lambda_telescope => (param, param_type, body_type),
                Term::Lambda {
                    param,
                    param_type,
                    body,
                } if lambda_telescope => (param, param_type, body),
                _ => break,
            };
            let (kind, inner) = match Self::as_kind_marked(param_type) {
                Some((k, d)) => (k, d),
                None => (BinderKind::Explicit, param_type.as_ref()),
            };
            let (declared, default, auto) = if let Some((d, v)) =
                Self::as_marked_binder(inner, Self::PARAM_DEFAULT_MARKER)
            {
                (d, Some(Self::strip_binder_markers(v)), None)
            } else if let Some((d, t)) = Self::as_marked_binder(inner, Self::AUTO_PARAM_MARKER) {
                let tactic = match t {
                    Term::Global(n) => Some(n.clone()),
                    _ => None,
                };
                (d, None, tactic)
            } else {
                (inner, None, None)
            };
            binders.push(BinderSurface {
                name: param.clone(),
                kind,
                declared: Self::strip_binder_markers(declared),
                default,
                auto,
            });
            cur = body_type;
        }
        DeclarationSurface {
            kernel_type: Self::strip_binder_markers(ty),
            binders,
        }
    }

    /// Give every ANONYMOUS binder the name the definition's BODY binds at that position.
    ///
    /// ★ IN LOGOS THE BODY IS WHERE THE NAMES ARE. `A -> B -> C` is the only spelling of a
    /// non-dependent signature and it names nothing -- `parse_arrow` builds every one of those
    /// binders as `Pi(_ : A). ...`. So for `Definition sub3 : Int -> Int -> Int -> Int := fun x
    /// => fun y => fun z => ...` the parameter names `x`, `y`, `z` exist ONLY in the body's
    /// lambda telescope, and a named argument `sub3 (y := 2)` can be resolved from nowhere else.
    ///
    /// A body name is taken only where the declared binder is anonymous, only while the two
    /// telescopes run in step, and never when it would COLLIDE with a name the declared type
    /// already gives another parameter -- two parameters answering to one name is worse than one
    /// parameter with no name, because a named argument would then silently pick one of them.
    pub fn name_anonymous_binders(binders: &mut [BinderSurface], body: &Term) {
        let mut names: Vec<String> = binders.iter().map(|b| b.name.clone()).collect();
        Self::name_anonymous_positions(&mut names, body);
        for (binder, name) in binders.iter_mut().zip(names) {
            binder.name = name;
        }
    }

    /// [`name_anonymous_binders`](Self::name_anonymous_binders) over a bare telescope of NAMES --
    /// the shape a caller has when it read the names off a `Pi` spine rather than off a
    /// [`DeclarationSurface`].
    ///
    /// ★ ONE ANSWER TO "WHERE ARE THE PARAMETER NAMES", IN THE FILE THAT OWNS THE SURFACE. The
    /// rule is not obvious and it is not local: a non-dependent signature names nothing, the body's
    /// λ-telescope is the only place the writer's names exist, and the two telescopes only run in
    /// step until the first one ends. Re-deriving that at each door is how two doors come to
    /// disagree about which parameter `f (x := …)` means.
    ///
    /// ★ AND A NAME MAY BE TAKEN ONLY ONCE. Two parameters answering to ONE name is worse than one
    /// parameter with no name at all: a named argument silently picks the first, the other becomes
    /// unreachable by name, and nothing reports it. `(x : Int) -> Int -> Int` defined as
    /// `fun y : Int => fun x : Int => …` is exactly that shape -- position 0 is DECLARED `x` while
    /// position 1's body binder is also `x` -- and it is ordinary text, not a contrivance. A
    /// position whose body name is already taken stays `"_"`, so a named argument resolves to the
    /// position that really is `x` or is refused by name.
    pub fn name_anonymous_positions(names: &mut [String], body: &Term) {
        let mut taken: HashSet<String> = names
            .iter()
            .filter(|n| n.as_str() != "_")
            .cloned()
            .collect();
        // A RECURSIVE definition's body is wrapped in a `Fix`, and the λ-telescope carrying the
        // writer's names is inside it. Transparent here: `fix f => fun x => …` and `fun x => …`
        // name their parameters identically, and a caller that had to know this is a caller that
        // can forget it.
        let mut cur = body;
        while let Term::Fix { body, .. } = cur {
            cur = body;
        }
        for slot in names.iter_mut() {
            let Term::Lambda { param, body, .. } = cur else {
                break;
            };
            if slot.as_str() == "_" && param != "_" && !taken.contains(param) {
                *slot = param.clone();
                taken.insert(param.clone());
            }
            cur = body;
        }
    }

    /// The DEFAULT value for parameter `index`, with every earlier parameter's ACTUAL argument
    /// substituted into it -- `sized 5` against `Int -> (m : Int := mul n 2) -> Int` inlines
    /// `mul 5 2`, never an open term with a free `n` in it.
    ///
    /// `args` must hold the arguments already resolved for positions `0..index`; with fewer than
    /// that this returns `None` rather than a term it cannot close. `None` also means the
    /// parameter simply has no default.
    ///
    /// ★ CALL [`name_anonymous_binders`](Self::name_anonymous_binders) FIRST. A default parsed in
    /// the scope of an ANONYMOUS earlier binder reads that reference as a `Term::Global`, because
    /// `A -> B` binds nothing the parser could have put in scope. Substitution is by NAME, so a
    /// telescope still full of `"_"` substitutes nothing and hands back exactly the open term this
    /// function exists to prevent -- which is why the two are documented as a pair and tested as
    /// one.
    pub fn inline_default(binders: &[BinderSurface], index: usize, args: &[Term]) -> Option<Term> {
        let mut value = binders.get(index)?.default.clone()?;
        if args.len() < index {
            return None;
        }
        for (i, earlier) in binders.iter().enumerate().take(index) {
            if earlier.name == "_" {
                continue;
            }
            value = Self::substitute_param_ref(&value, &earlier.name, &args[i]);
        }
        Some(value)
    }

    /// Every public entry point: the raw parse and its post-processing, on a stack deep enough
    /// for the input — see [`syntactic_depth`]. A text nested deeper than the kernel could ever
    /// check is refused here, as an error that names it, rather than by the process running out of
    /// stack.
    fn entry<R: Send>(
        surface: Option<&Surface>,
        input: &str,
        post: impl FnOnce(Term) -> R + Send,
    ) -> Result<R, ParseError> {
        let depth = syntactic_depth(input);
        if depth > MAX_SYNTACTIC_DEPTH {
            return Err(ParseError::Unsupported {
                what: format!("a term nested {depth} constructs deep"),
                why: format!(
                    "the kernel checks terms at most {} deep, and a text nested more than {} deep \
                     cannot denote one",
                    crate::type_checker::MAX_TERM_DEPTH,
                    MAX_SYNTACTIC_DEPTH
                ),
            });
        }
        on_stack_for(depth, move || Self::parse_raw_in(surface, input).map(post))
    }

    /// The raw parse: the whole input, markers intact.
    fn parse_raw_in<'i>(surface: Option<&'i Surface>, input: &'i str) -> Result<Term, ParseError> {
        match surface {
            Some(s) => {
                let expanded = s.expand_macros(input)?;
                let mut parser = TermParser {
                    input: &expanded,
                    pos: 0,
                    bound_vars: HashSet::new(),
                    surface: Some(s),
                };
                parser.parse_whole()
            }
            None => {
                let mut parser = TermParser {
                    input,
                    pos: 0,
                    bound_vars: HashSet::new(),
                    surface: None,
                };
                parser.parse_whole()
            }
        }
    }

    /// The legacy door's post-processing: strip the kind markers and report the LEADING run of
    /// eager implicits.
    ///
    /// ★ A STRICT IMPLICIT STOPS THE RUN. The count model inserts its metavariables
    /// unconditionally, and Lean's `⦃a⦄` is precisely the binder that must NOT be
    /// inserted when no explicit argument follows. Under-inserting leaves the term the
    /// polymorphic function it already was; over-inserting emits a metavariable nothing can
    /// solve and the kernel then reports an unbound variable far from here. Callers that need
    /// the real answer read [`parse_with_binder_kinds`](Self::parse_with_binder_kinds).
    fn finish_implicits(raw: Term) -> (Term, usize) {
        let (mut t, implicits) = Self::count_implicits(raw);
        Self::strip_kind_markers_in_place(&mut t);
        (t, implicits)
    }

    /// The leading-implicit count, leaving the term's kind markers exactly where they are.
    ///
    /// ★ THE COUNT STOPS AT A STRICT IMPLICIT, AND THAT IS THE RULE, NOT AN OVERSIGHT.
    /// `implicit_args` drives an UNCONDITIONAL insertion of that many leading arguments, and
    /// Lean's `⦃x : T⦄` is exactly the binder that must not be inserted when nothing follows it
    /// (`Lean/Elab/App.lean:726`, `processStrictImplicitArg`: insert only `if (← hasArgsToProcess)`,
    /// else `finalize`). A count cannot express a condition, so the honest thing a count can say
    /// about `⦃A : Type⦄ -> A -> A` is zero -- under-inserting leaves the polymorphic function,
    /// which is still correct, where over-inserting emits a metavariable nothing can solve.
    fn count_implicits(raw: Term) -> (Term, usize) {
        let implicits = Self::binder_kinds_of(&raw)
            .iter()
            .take_while(|k| **k == BinderKind::Implicit)
            .count();
        (raw, implicits)
    }

    fn finish_kinds(raw: Term) -> (Term, Vec<BinderKind>) {
        let kinds = Self::binder_kinds_of(&raw);
        let mut t = raw;
        Self::strip_kind_markers_in_place(&mut t);
        (t, kinds)
    }

    /// Parse a notation or macro TEMPLATE, with its holes already in scope so each one reads as a
    /// variable the expansion substitutes rather than as a global nobody declared.
    pub(super) fn parse_template(input: &'a str, holes: &[String]) -> Result<Term, ParseError> {
        let mut parser = Self {
            input,
            pos: 0,
            bound_vars: holes.iter().cloned().collect(),
            surface: None,
        };
        parser.parse_whole().map(|mut t| {
            Self::strip_kind_markers_in_place(&mut t);
            t
        })
    }

    /// Parse a term, also returning how many leading implicit binders `{x:T}` it had —
    /// the implicit-argument count the elaborator uses at application sites.
    pub fn parse_with_implicits(input: &'a str) -> Result<(Term, usize), ParseError> {
        Self::entry(None, input, Self::finish_implicits)
    }

    /// The whole input, or nothing. Binder-kind markers are still ON the result; every public
    /// entry point above strips them.
    fn parse_whole(&mut self) -> Result<Term, ParseError> {
        let parser = self;
        let term = parser.parse_term()?;
        parser.skip_whitespace();
        // ★ THE WHOLE INPUT, OR NOTHING. This returned here, so anything after the first complete
        // term was discarded in silence — `Nat -> Nat garbage here` parsed as `Nat -> Nat`. None of
        // the six callers in `command_parser` re-checked the position, so nothing downstream could
        // notice either. The `at_end` helper below already existed; it was the application loop's
        // stop condition and never a completeness check, which is the entire bug.
        // ★ ONE TRAILING `.` IS A TERMINATOR, NOT INPUT. `Zero.` is a complete term followed by a
        // full stop — the dot-projection parser deliberately declines to treat a dot before a
        // non-identifier as a projection (`surface_sugar::a_bare_trailing_dot_is_not_a_projection`
        // states exactly that), and this language ends its sentences with periods. Refusing it
        // would have made the completeness check reject ordinary well-formed input, which is how a
        // guard like this turns into a bug of its own. Exactly one, and nothing after it: `Zero..`
        // and `Zero. garbage` are still refused.
        if parser.input[parser.pos..].starts_with('.') {
            parser.pos += 1;
            parser.skip_whitespace();
        }
        if !parser.at_end() {
            // The BYTE POSITION and the remainder, because "trailing input" without a location is
            // the anonymous refusal this codebase keeps having to fix.
            return Err(ParseError::TrailingInput {
                at: parser.pos,
                rest: parser.input[parser.pos..].trim_end().to_string(),
            });
        }
        Ok(term)
    }

    /// Parse a term (top-level).
    fn parse_term(&mut self) -> Result<Term, ParseError> {
        self.skip_whitespace();

        if self.peek_keyword("fun") {
            self.parse_lambda()
        } else if self.peek_keyword("forall") {
            self.parse_forall()
        } else if self.peek_keyword("fix") {
            self.parse_fix()
        } else if self.peek_keyword("match") {
            self.parse_match()
        } else if self.peek_keyword("let") {
            self.parse_let()
        } else if self.at_bracket(Self::STRICT_IMPLICIT_OPEN_ASCII) {
            // BEFORE the single brace, exactly as Lean's `bracketedBinder` tries
            // `strictImplicitBinder` before `implicitBinder` (`Lean/Parser/Term.lean:381`):
            // `{{` must win, or its first brace is eaten as an eager implicit whose binder name
            // is then `{`.
            self.parse_grouped_binder(
                BinderKind::StrictImplicit,
                Self::STRICT_IMPLICIT_OPEN_ASCII,
                Self::STRICT_IMPLICIT_CLOSE_ASCII,
            )
        } else if self.peek_char('{') {
            self.parse_grouped_binder(BinderKind::Implicit, "{", "}")
        } else if self.peek_char('\u{2983}') && !self.at_atom_notation() {
            self.parse_grouped_binder(BinderKind::StrictImplicit, "\u{2983}", "\u{2984}")
        } else if self.peek_char('[') && !self.at_atom_notation() {
            self.parse_instance_binder()
        } else {
            self.parse_arrow()
        }
    }

    /// Does a BRACKETING notation in scope start here? `[` opens an instance binder in this
    /// grammar and a declared `[a, b]` list notation in many surfaces, and a user's own notation
    /// must keep winning where it always did -- a new builtin that shadowed a declared one would
    /// change the meaning of existing text rather than add to it.
    fn at_atom_notation(&self) -> bool {
        self.surface
            .is_some_and(|s| s.atom_notation_at(&self.input[self.pos..]).is_some())
    }

    /// Parse a let-expression `let x (":" T)? ":=" e "in" body`.
    ///
    /// ★ AN ASCRIPTION NOBODY CHECKED. The `: T` was parsed and THROWN AWAY -- the comment here
    /// said so -- and the whole form desugared to `body[x := e]`, so `let n : Bool := Zero in n`
    /// was a claim the language accepted and no kernel ever saw. Lean checks it
    /// (`let x : T := e; b` elaborates `e` AGAINST `T`), and so does this kernel the moment the
    /// ascription is allowed to reach it: `Term::Let` is a kernel term whose typing rule is
    /// `infer_sort(ty)` then `check_type(value, ty)` then zeta -- in `type_checker.rs` AND,
    /// independently, in `recheck.rs`, so `double_check` sees the same claim both times.
    ///
    /// The UN-ascribed form keeps the substitution desugaring it always had. There is no claim to
    /// check there, and emitting a `Let` node for it would put a binder in every term that used to
    /// hold none -- a cost paid by every `let` in the suite to buy nothing.
    fn parse_let(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("let")?;
        self.skip_whitespace();
        let name = self.parse_ident()?;
        self.skip_whitespace();
        let ascribed = if self.at_binder_annotation() {
            self.advance();
            let ty = self.parse_term()?;
            self.skip_whitespace();
            Some(ty)
        } else {
            None
        };
        self.expect_str(":=")?;
        self.skip_whitespace();
        let value = self.parse_term()?;
        self.skip_whitespace();
        // `let x : T := v; b` — the KERNEL's let, binder kept. See [`Self::parse_kernel_let_body`].
        if self.try_consume(";") {
            let Some(ty) = ascribed else {
                return Err(ParseError::Expected {
                    expected: format!(
                        "`: <type>` on `let {name}` \u{2014} a `;` let keeps its binder in the kernel \
                         term, and the kernel's let states the type it binds at"
                    ),
                    found: ";".to_string(),
                });
            };
            return self.parse_kernel_let_body(name, ty, value);
        }
        self.consume_keyword("in")?;

        let was_bound = self.bound_vars.contains(&name);
        self.bound_vars.insert(name.clone());
        let body = self.parse_term();
        if !was_bound {
            self.bound_vars.remove(&name);
        }
        let body = body?;

        // ★ THE BODY IS ζ-SUBSTITUTED EITHER WAY; THE `Let` NODE CARRIES ONLY THE CLAIM.
        //
        // Two things have to be true at once. The ascription must reach a kernel -- that is the
        // whole point -- and the REDUCTION behaviour of this surface must not change, because a
        // `let` has always meant its substituted body here. Keeping the bound name inside the node
        // would change it: `term_store::zeta_or_share_opt` deliberately SHARES rather than
        // duplicates when the name occurs many times, so `let y : Int := 5 in mul y y` would
        // survive into normal form as a let-expression and `Eval` would print one where the writer
        // asked for a value. Substituting first makes the node's body closed over `name`, so the
        // share/expand decision cannot arise and the emitted value is byte-identical to the
        // un-ascribed form's.
        //
        // What the node still does is the only thing it was added for: `infer_type` runs
        // `infer_sort(ty)` then `check_type(value, ty)` before it ever looks at the body, and
        // `recheck` does the same independently -- so `let n : Bool := Zero in n` is refused twice,
        // by two kernels, where it used to be accepted by neither because neither was told.
        let body = crate::type_checker::substitute(&body, &name, &value);
        match ascribed {
            Some(ty) => Ok(Term::Let {
                name,
                ty: Box::new(ty),
                value: Box::new(value),
                body: Box::new(body),
            }),
            None => Ok(body),
        }
    }

    /// The body of `let x : T := v; b`, with `x` in scope, as the kernel's own `Term::Let`.
    ///
    /// ★ TWO LETS, AND THE DIFFERENCE IS THE BINDER. `let … in b` substitutes `v` into `b` and keeps
    /// only the ascription as a claim (see [`Self::parse_let`]) — the right reading for source, and
    /// the one every existing `let` has always had. But the kernel HAS a let whose body still
    /// mentions its binder: the Lean importer builds one for every `letE` it does not share, the
    /// term store keeps one where substitution would duplicate work, and both kernels check and
    /// ζ-reduce it. A term like that had no spelling at all — printing it as `let … in` would read
    /// back as the substituted body, a different term. `;` is Lean's own separator for exactly this
    /// let, and it is the spelling that reads back as the node it was printed from.
    fn parse_kernel_let_body(
        &mut self,
        name: String,
        ty: Term,
        value: Term,
    ) -> Result<Term, ParseError> {
        let was_bound = self.bound_vars.contains(&name);
        self.bound_vars.insert(name.clone());
        let body = self.parse_term();
        if !was_bound {
            self.bound_vars.remove(&name);
        }
        Ok(Term::Let {
            name,
            ty: Box::new(ty),
            value: Box::new(value),
            body: Box::new(body?),
        })
    }

    /// Parse a BRACKETED binder group `{x y : T}` or `⦃x y : T⦄` (optionally followed
    /// by `->`) and the body -- one `Pi` per name, all at the shared type, each carrying `kind`.
    ///
    /// `{A B : Type}` is n binders sharing one type, the brace-counterpart of `(x y : T)`, and
    /// each one is its own ARGUMENT -- so the telescope grows by the number of names, not by the
    /// number of groups, or an application would insert too few metavariables. A group may NOT
    /// carry a default: see the refusal below, and `Lean/Parser/Term.lean:334` for whose clause
    /// `binderDefault` is.
    fn parse_grouped_binder(
        &mut self,
        kind: BinderKind,
        open: &str,
        close: &str,
    ) -> Result<Term, ParseError> {
        self.skip_whitespace();
        if !self.consume_bracket(open) {
            return Err(ParseError::Expected {
                expected: format!("'{open}'"),
                found: self.found_here(),
            });
        }
        self.skip_whitespace();
        let names = self
            .try_scan_binder_names()
            .ok_or_else(|| ParseError::Expected {
                expected: "a binder name".to_string(),
                found: self.found_here(),
            })?;
        self.skip_whitespace();
        if !self.at_binder_annotation() {
            return Err(ParseError::Expected {
                expected: format!("`:` and a type for the binder group `{open}{close}`"),
                found: self.found_here(),
            });
        }
        self.advance();
        let declared = self.parse_term()?;
        self.skip_whitespace();
        // ★ ONLY AN EXPLICIT BINDER MAY CARRY A DEFAULT, AND THAT IS NOT A STYLE RULE.
        // Lean gives the `binderDefault`/`binderTactic` clause to `explicitBinder` ALONE
        // (`Lean/Parser/Term.lean:334`); `implicitBinder` (:344) and `strictImplicitBinder` (:361)
        // have no such clause, and the elaborator agrees -- `getOptParamDefault?` and
        // `getAutoParamTactic?` are consulted inside `processExplicitArg`
        // (`Lean/Elab/App.lean:614`) and nowhere else, so `processImplicitArg` could not honour a
        // default even if one could be written.
        //
        // We accepted it, and that was the dangerous combination: the default was recorded at a
        // position `elaborate_app_against` fills with `Fill::Infer`, so the value the writer
        // supplied was SILENTLY DISCARDED. A default that is never consulted is a wrong answer
        // with no diagnostic, which is strictly worse than a refusal -- so refuse, and say which
        // binder would have had to be explicit for the default to mean anything.
        if self.peek_str(":=") {
            return Err(ParseError::Unsupported {
                what: format!(
                    "a default value on the `{open}{close}` binder `{}`",
                    names.join(" ")
                ),
                why: "an implicit parameter is filled by unification, never from a default, so                       the value could never be used -- write it as an explicit `(x : T := v)`                       parameter, which is the only binder Lean gives a default to"
                    .to_string(),
            });
        }
        self.close_binder_group(close, &names)?;

        self.skip_whitespace();
        self.try_consume("->"); // optional arrow between the binder and the body

        let body_type = self.parse_body_with(&names)?;
        Ok(Self::pi_telescope(
            names,
            Self::mark_kind(kind, declared),
            body_type,
        ))
    }

    /// Parse an INSTANCE binder `[C x] -> B` or `[inst : C x] -> B` and the body.
    ///
    /// ★ `ParamKind::Instance` HAD NO SPELLING. The elaborator has resolved instance parameters
    /// from the database since the typeclass package landed, and the only way to install one was
    /// a Rust caller reaching in through `Context::set_binder_kinds` -- which nothing outside the
    /// test suite ever did. So `[Inhabited A]` was a parameter kind the language could not write
    /// down, and every class-constrained declaration had to be spelled as an ordinary explicit
    /// argument the caller passes by hand.
    ///
    /// The anonymous form binds `"_"`, exactly as `A -> B` does: an instance argument is found,
    /// not named. The named form is Lean's `[inst : C x]`, and its name is in scope in the body.
    fn parse_instance_binder(&mut self) -> Result<Term, ParseError> {
        self.expect_char('[')?;
        let after_open = self.pos;
        // `[ name+ : C x ]` -- the NAMED form. The lookahead rewinds, exactly as `parse_parens`
        // does, so `[Inhabited Nat]` (two name-shaped tokens, no colon) still reads as a class
        // type rather than as a binder group missing its annotation.
        if let Some(names) = self.try_scan_binder_names() {
            self.skip_whitespace();
            if self.at_binder_annotation() {
                self.advance();
                let class_type = self.parse_term()?;
                self.skip_whitespace();
                self.expect_char(']')?;
                self.skip_whitespace();
                self.try_consume("->");
                let body = self.parse_body_with(&names)?;
                return Ok(Self::pi_telescope(
                    names,
                    Self::mark_kind(BinderKind::Instance, class_type),
                    body,
                ));
            }
        }
        self.pos = after_open;
        let class_type = self.parse_term()?;
        self.skip_whitespace();
        self.expect_char(']')?;
        self.skip_whitespace();
        self.try_consume("->");
        let body = self.parse_term()?;
        Ok(Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(Self::mark_kind(BinderKind::Instance, class_type)),
            body_type: Box::new(body),
        })
    }

    /// Parse arrow type: A -> B -> C (right associative).
    ///
    /// ★ THE RIGHT-HAND SIDE IS A WHOLE TERM. It used to recurse into `parse_arrow`, one level
    /// BELOW `parse_term` -- so everything `parse_term` dispatches on (an implicit, strict or
    /// instance binder group, a `forall`, a `fun`, a `let`, a `match`, a `fix`) was unreachable
    /// after the first arrow, and a telescope could only ever OPEN with one. `Nat -> {A : Type}
    /// -> A -> A` and `Nat -> forall x : Nat, P x` are ordinary Lean and had no spelling here for
    /// no reason but that the recursion skipped a level. The arrow stays RIGHT-ASSOCIATIVE:
    /// `parse_term` falls through to `parse_arrow` for everything that is not one of those
    /// keywords, so an ordinary `A -> B -> C` takes exactly the path it always took.
    fn parse_arrow(&mut self) -> Result<Term, ParseError> {
        let left = self.parse_notation_expr(0)?;

        self.skip_whitespace();
        if self.try_consume("->") {
            let right = self.parse_term()?;
            // A -> B is sugar for Π(_:A). B
            Ok(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(left),
                body_type: Box::new(right),
            })
        } else {
            Ok(left)
        }
    }

    /// The user-notation layer: precedence climbing over the infix and postfix notations in
    /// scope. With none declared this is one call straight through to [`parse_app`](Self::parse_app).
    fn parse_notation_expr(&mut self, min_precedence: u32) -> Result<Term, ParseError> {
        let mut lhs = self.parse_app()?;
        let Some(surface) = self.surface else {
            return Ok(lhs);
        };
        loop {
            self.skip_whitespace();
            let Some(m) = surface.operator_notation_at(&self.input[self.pos..], min_precedence)
            else {
                return Ok(lhs);
            };
            let pieces = m.pieces();
            let right_precedence = m.right_precedence();
            let mut args: HashMap<String, Term> = HashMap::new();
            let (_, lead_hole) = &pieces[0];
            args.insert(
                lead_hole
                    .clone()
                    .expect("an operator notation begins with a hole"),
                lhs,
            );
            for (index, (token, hole)) in pieces.iter().enumerate().skip(1) {
                match (token, hole) {
                    (Some(tok), _) => {
                        self.skip_whitespace();
                        if !self.try_consume(tok) {
                            return Err(ParseError::Expected {
                                expected: format!("`{tok}` of the notation"),
                                found: self.remaining().chars().take(8).collect::<String>(),
                            });
                        }
                    }
                    (_, Some(name)) => {
                        let last = index + 1 == pieces.len();
                        let operand = if last {
                            self.parse_notation_expr(right_precedence)?
                        } else {
                            self.parse_notation_expr(0)?
                        };
                        args.insert(name.clone(), operand);
                    }
                    _ => unreachable!("a piece is a token or a hole"),
                }
            }
            lhs = m.expand(surface, &args);
        }
    }

    /// A prefix or bracketing notation at atom position, if one starts here.
    fn parse_atom_notation(&mut self) -> Result<Option<Term>, ParseError> {
        let Some(surface) = self.surface else {
            return Ok(None);
        };
        let Some(m) = surface.atom_notation_at(&self.input[self.pos..]) else {
            return Ok(None);
        };
        let pieces = m.pieces();
        let closed = m.is_closed();
        let mut args: HashMap<String, Term> = HashMap::new();
        for (index, (token, hole)) in pieces.iter().enumerate() {
            match (token, hole) {
                (Some(tok), _) => {
                    self.skip_whitespace();
                    if !self.try_consume(tok) {
                        return Err(ParseError::Expected {
                            expected: format!("`{tok}` of the notation"),
                            found: self.remaining().chars().take(8).collect::<String>(),
                        });
                    }
                }
                (_, Some(name)) => {
                    let last = index + 1 == pieces.len();
                    // A prefix operator's operand binds tightly (application level); a bracketed
                    // one is delimited by its own closing token, so it may be any term.
                    let operand = if last && !closed {
                        self.parse_app()?
                    } else {
                        self.parse_notation_expr(0)?
                    };
                    args.insert(name.clone(), operand);
                }
                _ => unreachable!("a piece is a token or a hole"),
            }
        }
        Ok(Some(m.expand(surface, &args)))
    }

    /// Does a notation token stop an application here? An operator or a closing bracket of a
    /// notation in scope ends the argument list; a notation's OPENING token does not, so
    /// `f \u{27e6} 5 \u{27e7}` still applies `f` to the bracketed form.
    fn at_notation_stop(&self) -> bool {
        self.surface
            .is_some_and(|s| s.stops_application(&self.input[self.pos..]))
    }

    /// Parse application: f x y z (left associative), with an optional trailing ELLIPSIS.
    fn parse_app(&mut self) -> Result<Term, ParseError> {
        let mut func = self.parse_atom()?;

        loop {
            let before_space = self.pos;
            self.skip_whitespace();
            // `f ..` -- the ELLIPSIS: "fill every remaining argument with `_`". It must be
            // SEPARATED from the function by whitespace, because `Zero..` is a complete term
            // followed by a sentence terminator and one dot too many, and
            // `term_parser_total_lock::one_trailing_full_stop_is_a_terminator_and_not_input` pins
            // that it stays refused -- a grammar addition that quietly repaired malformed input
            // would be a bug introduced by a feature. Nothing may follow an ellipsis in a spine,
            // so this ends the application.
            if self.pos > before_space && self.at_ellipsis() {
                self.pos += "..".len();
                return Ok(Term::App(
                    Box::new(func),
                    Box::new(Term::Global(Self::ELLIPSIS_MARKER.to_string())),
                ));
            }
            // Check if we can parse another atom
            // Stop at: ), ->, =>, ,, |, with, return, end, ., EOF, or keywords
            if self.at_end()
                || self.peek_char(')')
                || self.peek_char('}')
                || self.peek_char('{')
                // The three brackets that CLOSE a binder group. Without them an application
                // inside one runs past its own closer -- `⦃A : Type⦄ -> A` would read `Type`
                // and then try to apply it to `⦄`, and `[Inhabited Nat] -> Nat` likewise.
                || self.peek_char(']')
                || self.peek_char('⦃')
                || self.peek_char('⦄')
                || self.peek_char('.')
                || self.peek_char(',')
                || self.peek_char(';')
                || self.peek_char('|')
                || self.peek_char('⟩')
                || self.peek_str("->")
                || self.peek_str("=>")
                || self.peek_str(":=")
                || self.peek_keyword("with")
                || self.peek_keyword("return")
                || self.peek_keyword("end")
                || self.peek_keyword("in")
                || self.at_notation_stop()
            {
                break;
            }

            let arg = self.parse_atom()?;
            func = Term::App(Box::new(func), Box::new(arg));
        }

        Ok(func)
    }

    /// Is an ELLIPSIS `..` at the current position? Exactly two dots: `...` is not one (and stays
    /// the trailing input it always was), and a `.field` projection begins with a single dot.
    fn at_ellipsis(&self) -> bool {
        self.peek_str("..") && !self.peek_str("...")
    }

    /// Parse an atom: (term), number, string, sort, or identifier
    fn parse_atom(&mut self) -> Result<Term, ParseError> {
        let base = self.parse_atom_base()?;
        self.parse_dot_postfix(base)
    }

    /// Parse the base of an atom, before any postfix projection.
    fn parse_atom_base(&mut self) -> Result<Term, ParseError> {
        self.skip_whitespace();

        // A prefix or bracketing notation is an atom of the grammar.
        if let Some(t) = self.parse_atom_notation()? {
            return Ok(t);
        }

        // Anonymous constructor ⟨e₁, …, eₙ⟩ (E3)
        if self.peek_char('⟨') {
            return self.parse_anon_ctor();
        }

        // Check for negative number
        if self.peek_char('-') {
            if let Some(lit) = self.try_parse_negative_number()? {
                return Ok(lit);
            }
        }

        // Check for positive number
        if let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                return self.parse_number_literal();
            }
        }

        // Check for string literal
        if self.peek_char('"') {
            return self.parse_string_literal();
        }

        // `@[annotation] atom` -- a TRANSPARENT annotation, `Term::Annotated`: the printer's spelling
        // of one, read back to the same term. `@[` never begins an explicit-all name.
        if self.peek_str("@[") {
            return self.parse_annotated();
        }

        // `@f` / `@Outer.value` -- EXPLICIT-ALL. It binds TIGHTLY to a name: `@ f` with a space is
        // not one, and neither is `@(f)`, exactly as in Lean. The guard is `scan_name` rather than
        // a character class, so the one component grammar decides here too and a quoted name works.
        if self.peek_char('@') {
            let after = self.pos + '@'.len_utf8();
            if scan_name(&self.input[after..]).is_some() {
                self.pos = after;
                let head = self.parse_atom()?;
                return Ok(Term::App(
                    Box::new(Term::Global(Self::EXPLICIT_ALL_MARKER.to_string())),
                    Box::new(head),
                ));
            }
        }

        if self.peek_char('(') {
            self.parse_parens()
        } else {
            self.parse_ident_or_sort()
        }
    }

    /// `@[binder <info>] a`, `@[source "<file:line:column>"] a` or `@[mdata "<key>" "<value>"] a`
    /// -- an [`Annotation`](crate::Annotation) on the atom `a`, whose binder information is one of
    /// Lean's four `BinderInfo` names.
    fn parse_annotated(&mut self) -> Result<Term, ParseError> {
        use crate::{Annotation, Provenance, SourcePosition};
        self.pos += "@[".len();
        self.skip_whitespace();
        let annotation = if self.try_consume_keyword("binder") {
            self.skip_whitespace();
            let info = scan_name(&self.input[self.pos..])
                .and_then(|(name, len)| Some((ParamKind::from_lean_binder_info(&name)?, len)));
            let Some((kind, len)) = info else {
                return Err(ParseError::Expected {
                    expected: "a binder information: `default`, `implicit`, `strictImplicit` or \
                               `instImplicit`"
                        .to_string(),
                    found: self.found_here(),
                });
            };
            self.pos += len;
            Annotation::Binder(kind)
        } else if self.try_consume_keyword("source") {
            self.skip_whitespace();
            let text = self.annotation_text()?;
            let position = SourcePosition::parse(&text).ok_or_else(|| ParseError::Expected {
                expected: "a source position `file:line:column`".to_string(),
                found: format!("{text:?}"),
            })?;
            Annotation::Source(Provenance::Position(position))
        } else if self.try_consume_keyword("mdata") {
            self.skip_whitespace();
            let key = self.annotation_text()?;
            self.skip_whitespace();
            let value = self.annotation_text()?;
            Annotation::Source(Provenance::LeanMetadata { key, value })
        } else {
            return Err(ParseError::Expected {
                expected: "an annotation: `binder`, `source` or `mdata`".to_string(),
                found: self.found_here(),
            });
        };
        self.skip_whitespace();
        self.expect_char(']')?;
        let term = self.parse_atom()?;
        Ok(Term::Annotated {
            annotation,
            term: Box::new(term),
        })
    }

    /// One string literal of an annotation, as its text.
    fn annotation_text(&mut self) -> Result<String, ParseError> {
        if !self.peek_char('"') {
            return Err(ParseError::Expected {
                expected: "a string literal".to_string(),
                found: self.found_here(),
            });
        }
        match self.parse_string_literal()? {
            Term::Lit(Literal::Text(text)) => Ok(text),
            other => Err(ParseError::Expected {
                expected: "a string literal".to_string(),
                found: other.to_string(),
            }),
        }
    }

    /// Apply zero or more TIGHT postfix projections `.field` to `base` (E4). Tight means no
    /// intervening whitespace: `p.fst` projects, while `p .` / `p.` at a clause end stays a
    /// terminator, and only a `.` immediately followed by an identifier start is a field.
    fn parse_dot_postfix(&mut self, mut base: Term) -> Result<Term, ParseError> {
        while self.peek_char('.') {
            let after = self.pos + '.'.len_utf8();
            // `name.{ℓ, …}` — a universe-polymorphic CONSTANT at explicit levels (`Term::Const`).
            if self.input[after..].starts_with('{') {
                let name = Self::constant_name(&base)?;
                self.pos = after + '{'.len_utf8();
                let levels = self.parse_level_list()?;
                base = Term::Const { name, levels };
                continue;
            }
            let Some((field, len)) = Self::dot_component(&self.input[after..], &base) else {
                break;
            };
            self.pos = after + len;
            base = Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global(crate::elaborate::DOT_MARKER.to_string())),
                    Box::new(base),
                )),
                Box::new(Term::Global(field)),
            );
        }
        Ok(base)
    }

    /// The NAME COMPONENT after a `.`, and the bytes it occupies -- or `None` when the dot is not
    /// a projection at all and the caller should stop.
    ///
    /// ★ ONE COMPONENT GRAMMAR, BOTH DOORS. This used to admit a component only when the next
    /// character satisfied `is_alphabetic() || '_'` -- [`scan_name`]'s FIRST rule rather than its
    /// whole rule -- so a guillemet component was unreachable and `Outer.«a b»` was a name no term
    /// could spell. The declaration door asks the term door whether a name is spellable
    /// (`surface::Name::is_spelled_by`), so it had to refuse a name Lean accepts
    /// (`def Foo.«a b»`) to keep that invariant true. Asking `scan_name` -- the very function
    /// the declaration door asks -- makes the two agree by construction rather than by review.
    ///
    /// A NUMERIC component is Lean's anonymous projection `p.1`, and it is refused after a
    /// LITERAL for exactly the reason a lexer refuses it there: `3.14` is a decimal numeral, not a
    /// projection out of `3`, and turning input this grammar REFUSES into something that silently
    /// parses is a bug introduced by a feature. `command_parser::sentence_end` names `3.14` in its
    /// own doc comment as a thing that must be able to sit inside a term without ending the
    /// sentence, so the collision is reachable rather than theoretical.
    fn dot_component(rest: &str, base: &Term) -> Option<(String, usize)> {
        if let Some(found) = scan_name(rest) {
            return Some(found);
        }
        if matches!(base, Term::Lit(_)) {
            return None;
        }
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            return None;
        }
        Some((rest[..digits].to_string(), digits))
    }

    /// Parse an anonymous constructor `⟨e₁, …, eₙ⟩` into the reserved marker application
    /// `⟨anon⟩ e₁ … eₙ` (E3). The elaborator resolves the marker against the expected type.
    fn parse_anon_ctor(&mut self) -> Result<Term, ParseError> {
        self.expect_char('⟨')?;
        let mut comps = Vec::new();
        self.skip_whitespace();
        if !self.peek_char('⟩') {
            loop {
                self.skip_whitespace();
                // A trailing `..` fills the REMAINING fields from the expected type. It ends the
                // component list, because there is nothing after "the rest".
                if self.at_ellipsis() {
                    self.pos += "..".len();
                    comps.push(Term::Global(Self::ELLIPSIS_MARKER.to_string()));
                    break;
                }
                comps.push(self.parse_term()?);
                self.skip_whitespace();
                if self.try_consume(",") {
                    self.skip_whitespace();
                    continue;
                }
                break;
            }
        }
        self.expect_char('⟩')?;
        let mut t = Term::Global(crate::elaborate::ANON_CTOR_MARKER.to_string());
        for c in comps {
            t = Term::App(Box::new(t), Box::new(c));
        }
        Ok(t)
    }

    /// Parse a literal that begins with a digit, at ARBITRARY PRECISION where the kind allows it.
    ///
    /// The digits go through [`crate::int_lit`], the canonical constructor: a value that fits `i64`
    /// is a `Literal::Int` exactly as before, and one that does not is a `Literal::BigInt`. Before
    /// the arbitrary-precision reading the parse was `str::parse::<i64>`, so a numeral wider than a
    /// machine word was `InvalidNumber` -- a value the kernel holds, compares and REDUCES that no
    /// source text could spell.
    ///
    /// What FOLLOWS the digits, with no space, decides the literal's kind — see
    /// [`Self::parse_numeric`]. Plain digits are an `Int`, as they always were.
    fn parse_number_literal(&mut self) -> Result<Term, ParseError> {
        let start = self.pos;
        self.parse_numeric(start, false)
    }

    /// The literal whose digits start at `self.pos`; `sign_start` is where its `-` stood, if
    /// `negative`.
    ///
    /// ★ ONE LEXER FOR EVERY `Literal`. The kernel carries seven literal kinds and the surface
    /// could spell two of them, so a `Nat` literal (every Lean numeral the importer reads), a
    /// `Float`, a `Duration`, a `Date` and a `Moment` were kernel terms with no source text at all.
    /// Each now has exactly one spelling, chosen so no two kinds collide and no spelling this
    /// grammar used to accept changes meaning:
    ///
    /// | written | literal |
    /// |---|---|
    /// | `42`, `-42` | `Int` (`BigInt` past `i64`) |
    /// | `42n` | `Nat` |
    /// | `1.5f64`, `-2.5e-3f64`, `1e300f64` | `Float`, by its shortest exact decimal |
    /// | `0x7ff8000000000000f64` | `Float`, by its IEEE-754 bits — every NaN payload, ±∞ |
    /// | `5s`, `-250ms`, `3min` | `Duration` (the imperative language's unit table) |
    /// | `2026-05-20`, `-0044-03-15` | `Date` |
    /// | `2026-05-20T10:00:00.5Z` | `Moment` (UTC, nanosecond precision) |
    ///
    /// Every form except plain digits was previously either a parse error or — for a letter right
    /// after the digits — an application of a numeral to a name, which denotes nothing. A suffix
    /// that names no kind is REFUSED, never read as that application: `2x` is a typo, not `2 x`.
    fn parse_numeric(&mut self, sign_start: usize, negative: bool) -> Result<Term, ParseError> {
        let digits_start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.advance();
        }
        let digits = self.input[digits_start..self.pos].to_string();
        let rest = &self.input[self.pos..];

        if !negative && digits == "0" && rest.starts_with('x') {
            return self.parse_float_bits();
        }
        if digits.len() >= 4 && Self::starts_month_day(rest) {
            return self.parse_calendar(&digits, negative);
        }
        // A fraction and an exponent make the numeral a DECIMAL, which must then name its type.
        let mut decimal = false;
        if rest.starts_with('.') && rest[1..].chars().next().is_some_and(|c| c.is_ascii_digit()) {
            self.advance();
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.advance();
            }
            decimal = true;
        }
        if Self::starts_exponent(&self.input[self.pos..]) {
            self.advance();
            if self.peek_char('+') || self.peek_char('-') {
                self.advance();
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.advance();
            }
            decimal = true;
        }
        let number_end = self.pos;
        // The suffix is a whole word — a letter, then letters, digits and `_` — so `f64` and
        // `min` are each read in one piece and looked up exactly.
        if self.peek().is_some_and(|c| c.is_alphabetic()) {
            while self
                .peek()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '\'')
            {
                self.advance();
            }
        }
        let suffix = &self.input[number_end..self.pos];
        let written = &self.input[sign_start..number_end];
        if suffix == "f64" {
            return written
                .parse::<f64>()
                .map(|x| Term::Lit(Literal::Float(x)))
                .map_err(|_| ParseError::InvalidNumber(format!("{written}f64")));
        }
        if decimal {
            // ★ A BARE DECIMAL IS REFUSED. `3.14` is not a projection out of `3`, and it is not a
            // float either until it says so: the kernel has more than one literal a decimal could
            // mean to a reader, and a spelling that quietly picked one would turn input this
            // grammar has always refused into a parse (`X_termparser_extra` locks the refusal).
            return Err(ParseError::InvalidNumber(format!(
                "{written}{suffix} \u{2014} a decimal literal names its type: write `{written}f64`"
            )));
        }
        let magnitude = crate::BigInt::parse_decimal(&digits)
            .ok_or_else(|| ParseError::InvalidNumber(digits.clone()))?;
        let value = if negative {
            crate::BigInt::zero().sub(&magnitude)
        } else {
            magnitude
        };
        match suffix {
            "" => Ok(Term::Lit(crate::int_lit(value))),
            "n" if negative => Err(ParseError::InvalidNumber(format!(
                "-{digits}n \u{2014} a `Nat` literal is a natural number and has no sign"
            ))),
            "n" => Ok(Term::Lit(Literal::Nat(value))),
            unit => match DURATION_UNITS.iter().find(|(u, _)| *u == unit) {
                Some((_, nanos)) => {
                    let total = value.mul(&crate::BigInt::from_i64(*nanos));
                    total
                        .to_i64()
                        .map(|n| Term::Lit(Literal::Duration(n)))
                        .ok_or_else(|| {
                            ParseError::InvalidNumber(format!(
                                "{}{digits}{unit} \u{2014} a duration is a signed 64-bit count of \
                             nanoseconds, and this one does not fit",
                                if negative { "-" } else { "" }
                            ))
                        })
                }
                None => Err(ParseError::InvalidNumber(format!(
                    "{}{digits}{unit} \u{2014} a numeral's suffix names what it counts: `n` for a \
                     natural number, or a duration unit (ns, us, ms, s, min, h). Write a space to \
                     apply a numeral to a name",
                    if negative { "-" } else { "" }
                ))),
            },
        }
    }

    /// `-DD-DD` — the month and day of a calendar literal, right after its year.
    fn starts_month_day(rest: &str) -> bool {
        let b = rest.as_bytes();
        b.len() >= 6
            && b[0] == b'-'
            && b[1].is_ascii_digit()
            && b[2].is_ascii_digit()
            && b[3] == b'-'
            && b[4].is_ascii_digit()
            && b[5].is_ascii_digit()
    }

    /// `e5`, `E-7`, `e+10` — a decimal exponent, which makes the numeral before it a float.
    fn starts_exponent(rest: &str) -> bool {
        let b = rest.as_bytes();
        match b {
            [b'e' | b'E', d, ..] if d.is_ascii_digit() => true,
            [b'e' | b'E', b'+' | b'-', d, ..] if d.is_ascii_digit() => true,
            _ => false,
        }
    }

    /// `0x` + sixteen hex digits + `f64`: a `Float` by its IEEE-754 BITS. The one spelling that
    /// reaches every value, because `Literal::Float` compares by bits — two NaNs with different
    /// payloads are different literals, and no decimal names either.
    fn parse_float_bits(&mut self) -> Result<Term, ParseError> {
        let at = self.pos;
        let rest = &self.input[at..];
        let hex = rest
            .get(1..17)
            .filter(|h| h.chars().all(|c| c.is_ascii_hexdigit()));
        let tail_ok = rest.get(17..).is_some_and(|t| {
            t.starts_with("f64")
                && !t[3..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
        });
        match (hex, tail_ok) {
            (Some(hex), true) => {
                let bits = u64::from_str_radix(hex, 16).expect("sixteen hex digits");
                self.pos = at + 20;
                Ok(Term::Lit(Literal::Float(f64::from_bits(bits))))
            }
            _ => Err(ParseError::InvalidNumber(format!(
                "0{} \u{2014} a float by its bits is `0x` followed by exactly sixteen hex digits and \
                 `f64`",
                rest.chars().take(20).collect::<String>()
            ))),
        }
    }

    /// `YYYY-MM-DD`, then optionally `THH:MM:SS(.fffffffff)?Z`: a `Date`, or a `Moment` in UTC.
    ///
    /// Proleptic Gregorian, the calendar `Literal::Date`'s display uses, with the year as a plain
    /// signed number of at least four digits — so every `i32` day count and every `i64` nanosecond
    /// instant has a spelling, and an impossible date (`2026-02-30`) is refused rather than
    /// rolled over into March.
    fn parse_calendar(&mut self, year_digits: &str, negative: bool) -> Result<Term, ParseError> {
        let bad = |what: String| ParseError::InvalidNumber(what);
        let year: i64 = year_digits
            .parse::<i64>()
            .ok()
            .filter(|y| *y <= 100_000_000)
            .ok_or_else(|| {
                bad(format!(
                    "{year_digits} \u{2014} a year this calendar cannot hold"
                ))
            })?;
        let year = if negative { -year } else { year };
        let b = &self.input.as_bytes()[self.pos..];
        let two = |i: usize| ((b[i] - b'0') * 10 + (b[i + 1] - b'0')) as u32;
        let (month, day) = (two(1), two(4));
        self.pos += 6;
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let month_days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => 0,
        };
        if day == 0 || day > month_days {
            return Err(bad(format!(
                "{year}-{month:02}-{day:02} \u{2014} not a day of the calendar"
            )));
        }
        let days = days_from_civil(year, month, day);
        if !self.peek_char('T') {
            return i32::try_from(days)
                .map(|d| Term::Lit(Literal::Date(d)))
                .map_err(|_| bad(format!("{year}-{month:02}-{day:02} \u{2014} outside the 32-bit day range of a date")));
        }
        self.advance();
        let t = self.input.as_bytes()[self.pos..].to_vec();
        let shape = t.len() >= 8
            && t[0].is_ascii_digit()
            && t[1].is_ascii_digit()
            && t[2] == b':'
            && t[3].is_ascii_digit()
            && t[4].is_ascii_digit()
            && t[5] == b':'
            && t[6].is_ascii_digit()
            && t[7].is_ascii_digit();
        if !shape {
            return Err(bad(
                "a time of day `THH:MM:SS` after the date of a moment".to_string()
            ));
        }
        let field = |i: usize| ((t[i] - b'0') as i64) * 10 + (t[i + 1] - b'0') as i64;
        let (h, m, sec) = (field(0), field(3), field(6));
        if h > 23 || m > 59 || sec > 59 {
            return Err(bad(format!(
                "{h:02}:{m:02}:{sec:02} \u{2014} not a time of day"
            )));
        }
        self.pos += 8;
        let mut fraction: i64 = 0;
        if self.peek_char('.') {
            self.advance();
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.advance();
            }
            let digits = &self.input[start..self.pos];
            if digits.is_empty() || digits.len() > 9 {
                return Err(bad(format!(
                    ".{digits} \u{2014} a moment's fraction is one to nine digits of a second"
                )));
            }
            fraction =
                digits.parse::<i64>().expect("ascii digits") * 10i64.pow(9 - digits.len() as u32);
        }
        if !self.peek_char('Z') {
            return Err(bad("the `Z` that says a moment is in UTC".to_string()));
        }
        self.advance();
        let seconds = days as i128 * 86_400 + (h * 3600 + m * 60 + sec) as i128;
        let nanos = seconds * 1_000_000_000 + fraction as i128;
        i64::try_from(nanos)
            .map(|n| Term::Lit(Literal::Moment(n)))
            .map_err(|_| bad("a moment outside the signed 64-bit nanosecond range".to_string()))
    }

    /// Parse a string literal: "contents"
    fn parse_string_literal(&mut self) -> Result<Term, ParseError> {
        self.expect_char('"')?;

        // Collect characters until closing quote
        let mut content = String::new();
        loop {
            match self.peek() {
                Some('"') => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    // Handle escape sequences
                    self.advance();
                    match self.peek() {
                        Some('n') => {
                            content.push('\n');
                            self.advance();
                        }
                        Some('t') => {
                            content.push('\t');
                            self.advance();
                        }
                        Some('\\') => {
                            content.push('\\');
                            self.advance();
                        }
                        Some('"') => {
                            content.push('"');
                            self.advance();
                        }
                        // `\u{…}` — a Unicode scalar by its hex code point. The escape every other
                        // character can be written with, so a printer can put ANY string in source
                        // text: a control character, a line separator, a lone NUL.
                        Some('u') if self.input[self.pos + 1..].starts_with('{') => {
                            let open = self.pos + 2;
                            let close = self.input[open..].find('}').map(|i| open + i);
                            let scalar = close
                                .map(|c| &self.input[open..c])
                                .filter(|h| {
                                    (1..=6).contains(&h.len())
                                        && h.chars().all(|c| c.is_ascii_hexdigit())
                                })
                                .and_then(|h| u32::from_str_radix(h, 16).ok())
                                .and_then(char::from_u32);
                            match (scalar, close) {
                                (Some(c), Some(close)) => {
                                    content.push(c);
                                    self.pos = close + 1;
                                }
                                _ => {
                                    return Err(ParseError::Expected {
                                        expected: "`\\u{…}` holding one to six hex digits of a Unicode scalar value"
                                            .to_string(),
                                        found: self.input[self.pos..].chars().take(12).collect(),
                                    })
                                }
                            }
                        }
                        Some(c) => {
                            content.push(c);
                            self.advance();
                        }
                        None => return Err(ParseError::UnexpectedEof),
                    }
                }
                Some(c) => {
                    content.push(c);
                    self.advance();
                }
                None => return Err(ParseError::UnexpectedEof),
            }
        }

        Ok(Term::Lit(Literal::Text(content)))
    }

    /// A negative literal, or `None` when the `-` is not a numeral's sign at all -- so a declared
    /// prefix or infix `-` notation still reads as one.
    ///
    /// A FAILED ATTEMPT MUST NOT CONSUME INPUT. This used to `advance()` past the `-` and the digits
    /// and THEN return `None` when the value did not fit an `i64`, leaving the parser's position
    /// past a literal nobody had read. `-9223372036854775809 Nat` therefore parsed as `Nat`, with
    /// the numeral gone in silence, and `-9223372036854775808` -- `i64::MIN`, whose digits do not
    /// fit `i64` as a POSITIVE -- had no spelling at all. Silently reading a prefix of the input is
    /// the one failure a proof kernel's parser must not have (`term_parser_total_lock` is entirely
    /// about it). Arbitrary precision removes the overflow case; once a digit follows the `-` the
    /// literal is COMMITTED, and a malformed one is an error that names it rather than a `None`
    /// that lets the caller read something else.
    fn try_parse_negative_number(&mut self) -> Result<Option<Term>, ParseError> {
        if !self.peek_char('-') {
            return Ok(None);
        }
        let after_dash = self.pos + '-'.len_utf8();
        if !self.input[after_dash..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        {
            return Ok(None);
        }
        let sign_start = self.pos;
        self.pos = after_dash;
        self.parse_numeric(sign_start, true).map(Some)
    }

    /// Parse a parenthesized form. THREE productions share the opening `(`, and one token of
    /// lookahead separates them:
    ///
    /// * `( name := e )` -- a NAMED ARGUMENT, emitted as [`Self::NAMED_ARG_MARKER`] `name e`.
    /// * `( name+ : T )`, `( name+ : T := e )`, `( name+ : T := by tac )` -- an EXPLICIT dependent
    ///   BINDER, the counterpart of the `{x : T} -> body` this parser already read. Every name in
    ///   the group is in scope in the body, so `(A : Type) -> A -> A` finally spells what only
    ///   `forall A : Type, A -> A` could spell before.
    /// * `( term )` -- an ordinary grouping.
    ///
    /// The lookahead REWINDS rather than guesses: a `(` whose contents are not `name :=` or
    /// `name+ :` returns to the position just after the `(` and reads an ordinary term. That is
    /// why `(Succ Zero)`, `(x)`, `(fun x : Nat => x)` and `(let y := 0 in y)` take exactly the
    /// path they always took -- and why the KEYWORD GUARD in [`Self::try_scan_binder_names`] is
    /// load-bearing rather than defensive: `fun` is name-shaped and is followed by a `:`.
    fn parse_parens(&mut self) -> Result<Term, ParseError> {
        self.expect_char('(')?;
        let after_open = self.pos;
        if let Some(named) = self.try_parse_named_argument()? {
            return Ok(named);
        }
        self.pos = after_open;
        if let Some(binder) = self.try_parse_explicit_binder()? {
            return Ok(binder);
        }
        self.pos = after_open;
        let term = self.parse_term()?;
        self.skip_whitespace();
        self.expect_char(')')?;
        Ok(term)
    }

    /// `( name := e )` -- a named argument. `Ok(None)` when the contents are not one, with the
    /// position left dirty for [`Self::parse_parens`] to rewind; an `Err` only once `name :=` has
    /// been seen and the form is therefore committed.
    fn try_parse_named_argument(&mut self) -> Result<Option<Term>, ParseError> {
        self.skip_whitespace();
        let Some((name, len, quoted)) = scan_spelled_name(&self.input[self.pos..]) else {
            return Ok(None);
        };
        if !quoted && Self::is_reserved(&name) {
            return Ok(None);
        }
        self.pos += len;
        self.skip_whitespace();
        if !self.try_consume(":=") {
            return Ok(None);
        }
        let value = self.parse_term()?;
        self.skip_whitespace();
        self.expect_char(')')?;
        Ok(Some(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global(Self::NAMED_ARG_MARKER.to_string())),
                Box::new(Term::Global(name)),
            )),
            Box::new(value),
        )))
    }

    /// `( name+ : T (":=" default)? )` followed by an optional `->` and the body. `Ok(None)` when
    /// the contents are not a binder group, position dirty for the caller to rewind.
    fn try_parse_explicit_binder(&mut self) -> Result<Option<Term>, ParseError> {
        let Some(names) = self.try_scan_binder_names() else {
            return Ok(None);
        };
        self.skip_whitespace();
        if !self.at_binder_annotation() {
            return Ok(None);
        }
        self.advance();
        let declared = self.parse_term()?;
        self.skip_whitespace();
        let param_type = if self.try_consume(":=") {
            self.parse_binder_default(declared)?
        } else {
            declared
        };
        self.skip_whitespace();
        self.close_binder_group(")", &names)?;
        self.skip_whitespace();
        self.try_consume("->"); // the arrow between a binder and its body is optional
        let body = self.parse_body_with(&names)?;
        Ok(Some(Self::pi_telescope(names, param_type, body)))
    }

    /// The `:= ...` half of a binder: a DEFAULT VALUE the caller may omit, or `by tac` -- an
    /// obligation a tactic discharges afresh at every call site. Either way the result is the
    /// declared type wrapped in the marker that says WHICH, so the binder itself stays an ordinary
    /// `Term::Pi` and the information travels with the type through every existing pipe.
    fn parse_binder_default(&mut self, declared: Term) -> Result<Term, ParseError> {
        let (marker, payload) = if self.try_consume_keyword("by") {
            self.skip_whitespace();
            (Self::AUTO_PARAM_MARKER, Term::Global(self.parse_ident()?))
        } else {
            self.skip_whitespace();
            (Self::PARAM_DEFAULT_MARKER, self.parse_term()?)
        };
        Ok(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global(marker.to_string())),
                Box::new(declared),
            )),
            Box::new(payload),
        ))
    }

    /// Close a binder group, naming the group in the refusal.
    ///
    /// ★ A REFUSAL THAT NAMES THE BINDER. `Expected { expected: "')'", found: "EOF" }` is what a
    /// declaration whose TYPE contains a `:=` reports today, because the declaration door splits
    /// the sentence at the first `:=` it finds anywhere -- including the one inside
    /// `(m : Int := 3)` -- and hands this parser the truncated text `Int -> (m : Int`. The split
    /// is not this parser's to fix ([`Self::find_top_level`] is the scan that does it properly),
    /// but the diagnosis is: a message that says WHICH binder group never closed points at the
    /// `(m : ...` the reader actually wrote, instead of at a parenthesis they never typed.
    fn close_binder_group(&mut self, close: &str, names: &[String]) -> Result<(), ParseError> {
        self.skip_whitespace();
        if self.consume_bracket(close) {
            return Ok(());
        }
        Err(ParseError::Expected {
            expected: format!(
                "`{}` closing the binder group for `{}`",
                close,
                names.join(" ")
            ),
            found: self.found_here(),
        })
    }

    /// One or more binder NAMES at the current position, or `None` when this is not a binder group.
    ///
    /// A RESERVED word is never a binder name. Without that rule `(fun x : Nat => x)` reads as the
    /// binder group `(fun x : ...)` -- two name-shaped tokens followed by a colon -- and an
    /// ordinary lambda in parentheses stops parsing.
    fn try_scan_binder_names(&mut self) -> Option<Vec<String>> {
        let mut names = Vec::new();
        loop {
            self.skip_whitespace();
            match scan_spelled_name(&self.input[self.pos..]) {
                // A QUOTED reserved word is a name — the guillemets are exactly how a binder is
                // given one — so only the bare word stops a binder group.
                Some((name, _, false)) if Self::is_reserved(&name) => return None,
                Some((name, len, _)) => {
                    names.push(name);
                    self.pos += len;
                }
                None => break,
            }
        }
        if names.is_empty() {
            None
        } else {
            Some(names)
        }
    }

    /// The words this grammar reserves. `Prop` and `Type` are deliberately absent: they are SORTS,
    /// read by [`Self::parse_ident_or_sort`], and a binder group never begins with one.
    fn is_reserved(name: &str) -> bool {
        is_reserved_word(name)
    }

    /// A `:` that opens a binder ANNOTATION -- and NOT the `:=` that opens a default value, which
    /// is a different production. Call it at a non-whitespace position.
    fn at_binder_annotation(&self) -> bool {
        self.peek_char(':') && !self.peek_str(":=")
    }

    /// Parse a term with `names` in scope, restoring the scope afterwards -- on the ERROR path too,
    /// so a binder from a failed parse cannot leak into a sibling term. A name already bound is
    /// left bound (it is shadowed here, not introduced).
    fn parse_body_with(&mut self, names: &[String]) -> Result<Term, ParseError> {
        let introduced: Vec<String> = names
            .iter()
            .filter(|n| self.bound_vars.insert((*n).clone()))
            .cloned()
            .collect();
        let body = self.parse_term();
        for n in &introduced {
            self.bound_vars.remove(n);
        }
        body
    }

    /// `Pi(name0 : T). ... Pi(nameN : T). body` -- one binder per name, all at the same type.
    fn pi_telescope(names: Vec<String>, param_type: Term, body: Term) -> Term {
        names.into_iter().rev().fold(body, |acc, param| Term::Pi {
            param,
            param_type: Box::new(param_type.clone()),
            body_type: Box::new(acc),
        })
    }

    /// Parse identifier or sort (Prop, SProp, Type, Type0, Sort ℓ).
    ///
    /// ★ A QUOTED NAME IS NEVER A SORT. `«Prop»` is the global named `Prop`, not the universe of
    /// propositions: the guillemets are the escape that lets a declaration or a binder be named by a
    /// word this grammar otherwise reads as syntax, and an escape that still meant the keyword would
    /// be no escape at all. Only the unquoted words below are sorts.
    fn parse_ident_or_sort(&mut self) -> Result<Term, ParseError> {
        let (ident, quoted) = self.parse_ident_spelled()?;

        if !quoted {
            match ident.as_str() {
                // ★ `SProp` HAD NO SPELLING. The kernel HAS a definitionally-irrelevant universe --
                // `Universe::SProp`, with its own elimination rule in `type_checker.rs` and a corpus
                // case (`lean_conformance_t01_universes`) about the `0 = 1` derivation that rule prevents
                // -- and `Term::Sort(SProp)` PRINTS as `SProp`, so the kernel emitted a type this
                // parser read straight back as an undeclared global.
                "SProp" => return Ok(Term::Sort(Universe::SProp)),
                "Prop" => return Ok(Term::Sort(Universe::Prop)),
                "Type" => {
                    // Check for Type followed by a LEVEL
                    self.skip_whitespace();
                    return match self.parse_universe_level()? {
                        Some(n) => Ok(Term::Sort(Universe::Type(n))),
                        None => Ok(Term::Sort(Universe::Type(0))),
                    };
                }
                // ★ `Sort ℓ` — THE SORT AT ANY LEVEL, including a level that mentions universe
                // VARIABLES. `Type n` reaches only the concrete rungs, so a universe-polymorphic
                // declaration (`id.{u} : (A : Sort u) -> A -> A`) had no spelling, and neither did
                // any type the Lean importer builds over `Sort u`. The level is one level ATOM, so
                // `Sort u -> Sort v` and `f (Sort u) x` read the way they look.
                "Sort" => {
                    self.skip_whitespace();
                    return self.parse_level_atom().map(Term::Sort);
                }
                _ => {}
            }
            // `TypeN` (`Type0`, `Type1`) is the sort's other spelling.
            //
            // ★ AND IT HAD THE SAME CLIFF THE SPACED FORM WAS FIXED FOR. A suffix that is ALL
            // DIGITS is a universe level; one that does not fit `Universe::Type`'s `u32` is a level
            // this system does not have, and is refused exactly as the spaced form refuses it. A
            // suffix that is NOT all digits (`Typeclass`, `Type_x`) was never a level and is still
            // an ordinary name.
            if let Some(level) = ident.strip_prefix("Type") {
                if !level.is_empty() && level.chars().all(|c| c.is_ascii_digit()) {
                    return match level.parse() {
                        Ok(n) => Ok(Term::Sort(Universe::Type(n))),
                        Err(_) => Err(ParseError::InvalidNumber(level.to_string())),
                    };
                }
            }
        }
        // ★ `_` IS A HOLE, AND NEVER A NAME A TERM REFERS TO. It used to read as
        // `Term::Global("_")`, which is the KERNEL'S OWN match-case placeholder protocol:
        // `type_checker::check_type` matches a λ whose `param_type` is exactly that global and takes
        // the binder's type from the expected `Pi` instead. `Term::Hole` is the honest carrier:
        // `infer_type` answers `CannotInferHole` and the elaborator turns it into a fresh
        // metavariable, which is exactly Lean's `_`. Checked BEFORE the bound-variable lookup, and
        // for the quoted `«_»` too, because the collision it prevents does not care how `_` was
        // written.
        if ident == "_" {
            Ok(Term::Hole)
        } else if self.bound_vars.contains(&ident) {
            Ok(Term::Var(ident))
        } else {
            Ok(Term::Global(ident))
        }
    }

    /// A universe LEVEL: an atom followed by any number of `+k` successors.
    ///
    /// Read STRUCTURALLY and never normalised: `u+2` is `Succ(Succ(Var u))`, `max u u` stays a
    /// `Max`, and the numeral `1` is `Type 0` while `0+1` is `Succ(Prop)`. The kernel decides level
    /// EQUIVALENCE by its own algebra; the surface's job is to say which expression was written, so
    /// that the printer can write back exactly the expression it was handed.
    fn parse_level(&mut self) -> Result<Universe, ParseError> {
        let mut level = self.parse_level_atom()?;
        loop {
            self.skip_whitespace();
            if !self.peek_char('+') {
                return Ok(level);
            }
            self.advance();
            self.skip_whitespace();
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.advance();
            }
            let digits = &self.input[start..self.pos];
            let k: u64 = match digits.parse() {
                Ok(k) if (1..=MAX_LEVEL_OFFSET).contains(&k) => k,
                Ok(0) => {
                    return Err(ParseError::InvalidNumber(
                        "+0 \u{2014} a level successor adds at least one".to_string(),
                    ))
                }
                Ok(_) | Err(_) if !digits.is_empty() => {
                    return Err(ParseError::InvalidNumber(format!(
                        "+{digits} \u{2014} a level may add at most {MAX_LEVEL_OFFSET} at once"
                    )))
                }
                _ => {
                    return Err(ParseError::Expected {
                        expected: "a numeral after `+` in a universe level".to_string(),
                        found: self.found_here(),
                    })
                }
            };
            for _ in 0..k {
                level = Universe::Succ(Box::new(level));
            }
        }
    }

    /// One universe level ATOM: a numeral, a universe variable, `max a b`, `imax a b`, or a
    /// parenthesised level.
    ///
    /// A numeral is Lean's level numeral: `0` is `Prop`, `n + 1` is `Type n`. A variable is any
    /// name; one spelled like a keyword or a sort (`max`, `Prop`, `_`) must be written `«quoted»`,
    /// so `Sort Prop` is refused rather than silently read as a variable named `Prop`.
    fn parse_level_atom(&mut self) -> Result<Universe, ParseError> {
        self.skip_whitespace();
        if self.peek_char('(') {
            self.advance();
            let level = self.parse_level()?;
            self.skip_whitespace();
            self.expect_char(')')?;
            return Ok(level);
        }
        if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.advance();
            }
            let digits = &self.input[start..self.pos];
            let n: u64 = digits
                .parse()
                .map_err(|_| ParseError::InvalidNumber(digits.to_string()))?;
            return match n {
                0 => Ok(Universe::Prop),
                n => u32::try_from(n - 1).map(Universe::Type).map_err(|_| {
                    ParseError::InvalidNumber(format!(
                        "{digits} \u{2014} the concrete universe levels stop at {}",
                        u64::from(u32::MAX) + 1
                    ))
                }),
            };
        }
        match scan_spelled_name(&self.input[self.pos..]) {
            Some((name, len, false)) if name == "max" || name == "imax" => {
                self.pos += len;
                let a = Box::new(self.parse_level_atom()?);
                let b = Box::new(self.parse_level_atom()?);
                Ok(if name == "max" {
                    Universe::Max(a, b)
                } else {
                    Universe::IMax(a, b)
                })
            }
            Some((name, _, false))
                if name == "_" || Self::is_reserved(&name) || is_sort_word(&name) =>
            {
                Err(ParseError::Expected {
                    expected: format!(
                        "a universe level \u{2014} `{name}` is a word of the grammar; a universe \
                         variable spelled that way is written \u{ab}{name}\u{bb}"
                    ),
                    found: name,
                })
            }
            Some((name, len, _)) => {
                self.pos += len;
                Ok(Universe::Var(name))
            }
            None => Err(ParseError::Expected {
                expected:
                    "a universe level: a numeral, a universe variable, `max a b`, `imax a b` \
                           or a parenthesised level"
                        .to_string(),
                found: self.found_here(),
            }),
        }
    }

    /// `ℓ, ℓ, …}` — the level ARGUMENTS of a constant, after its `.{`. May be empty.
    fn parse_level_list(&mut self) -> Result<Vec<Universe>, ParseError> {
        let mut levels = Vec::new();
        self.skip_whitespace();
        if self.try_consume("}") {
            return Ok(levels);
        }
        loop {
            levels.push(self.parse_level()?);
            self.skip_whitespace();
            if self.try_consume(",") {
                continue;
            }
            if self.try_consume("}") {
                return Ok(levels);
            }
            return Err(ParseError::Expected {
                expected: "`,` or `}` in a list of universe levels".to_string(),
                found: self.found_here(),
            });
        }
    }

    /// The constant a `.{…}` instantiates: a global, or a qualified name read as a projection chain
    /// of globals. `.{` can only follow a CONSTANT's name, so a chain before it is one hierarchical
    /// name rather than field access — Lean's `Nat.rec.{u}` is one constant.
    fn constant_name(base: &Term) -> Result<String, ParseError> {
        fn walk(t: &Term, out: &mut Vec<String>) -> Result<(), ParseError> {
            match t {
                Term::Global(n) => {
                    out.push(n.clone());
                    Ok(())
                }
                Term::App(f, field) => match (f.as_ref(), field.as_ref()) {
                    (Term::App(head, receiver), Term::Global(field)) if matches!(head.as_ref(), Term::Global(m) if m == crate::elaborate::DOT_MARKER) =>
                    {
                        walk(receiver, out)?;
                        out.push(field.clone());
                        Ok(())
                    }
                    _ => Err(ParseError::Expected {
                        expected: "a constant's name before `.{` \u{2014} universe arguments \
                                   instantiate a declared constant"
                            .to_string(),
                        found: format!("{t}"),
                    }),
                },
                Term::Var(n) => Err(ParseError::Expected {
                    expected: format!(
                        "a constant before `.{{` \u{2014} `{n}` is a local variable, and a local \
                         takes no universe arguments"
                    ),
                    found: n.clone(),
                }),
                other => Err(ParseError::Expected {
                    expected: "a constant's name before `.{` \u{2014} universe arguments \
                               instantiate a declared constant"
                        .to_string(),
                    found: format!("{other}"),
                }),
            }
        }
        let mut parts = Vec::new();
        walk(base, &mut parts)?;
        Ok(parts.join("."))
    }

    /// Parse a lambda `fun <binders> (":" T)? "=>" body`.
    ///
    /// A binder is a bare identifier -- whose type is POSTPONED, left as a `Term::Hole` for the
    /// elaborator to fill from the expected type, which is what `fun x => Succ x` means and what
    /// this parser could not express at all -- or a BRACKETED GROUP, whose bracket says how the
    /// argument is supplied: `(x y : T)`, `{A}`, `⦃A⦄`, `[Monoid A]`. A trailing
    /// `: T` types every binder that still has none, exactly as Lean's `optType` does, so
    /// `fun x y : Nat => x` binds BOTH at `Nat` and the one-binder `fun x : T => body` this parser
    /// has always read is the same production at n = 1.
    ///
    /// A `Hole` left here is not a hole smuggled into a proof: `infer_type` answers
    /// `CannotInferHole`, so an un-elaborated `fun x => ...` fails loudly rather than checking
    /// against everything.
    fn parse_lambda(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("fun")?;
        let (binders, body) = self.parse_abstraction("=>")?;
        Ok(binders
            .into_iter()
            .rev()
            .fold(body, |acc, (param, param_type, kind)| Term::Lambda {
                param,
                param_type: Box::new(Self::mark_kind(kind, param_type)),
                body: Box::new(acc),
            }))
    }

    /// Parse `forall <binders> (":" T)? "," body` -- the binder grammar of [`Self::parse_lambda`],
    /// building `Pi` instead of `Lambda`.
    fn parse_forall(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("forall")?;
        let (binders, body) = self.parse_abstraction(",")?;
        Ok(binders
            .into_iter()
            .rev()
            .fold(body, |acc, (param, param_type, kind)| Term::Pi {
                param,
                param_type: Box::new(Self::mark_kind(kind, param_type)),
                body_type: Box::new(acc),
            }))
    }

    /// The binder telescope shared by `fun` and `forall`: binders, an optional SHARED annotation,
    /// the `separator`, then the body -- parsed with every binder in scope, and with each group's
    /// names entering scope only AFTER its own type is read, so `(x : Nat) (y : Vec x)` resolves
    /// `x` and `(x : Vec x)` still does not. Returns the binders in SOURCE order.
    fn parse_abstraction(
        &mut self,
        separator: &str,
    ) -> Result<(Vec<(String, Term, BinderKind)>, Term), ParseError> {
        let mut introduced: Vec<String> = Vec::new();
        let parsed = self.parse_abstraction_in_scope(separator, &mut introduced);
        // ★ THE SCOPE IS RESTORED ON EVERY PATH, THE ERROR PATH INCLUDED. A refused abstraction is
        // REWOUND and re-read as an ordinary term -- that is exactly what `parse_parens` does to
        // separate `(fun x : Nat => x)` from a binder group -- so a binder the failed attempt left
        // in scope makes a later `Global` of that spelling read as a `Var`: a name bound by a
        // parse that did not happen. `parse_body_with` already states this rule for the binder
        // GROUPS; the telescope's own `?`s were the one place it was not kept, and hoisting the
        // restore out of the loop makes the rule structural rather than repeated.
        for n in &introduced {
            self.bound_vars.remove(n);
        }
        parsed
    }

    /// [`parse_abstraction`](Self::parse_abstraction) with the binders LEFT in scope -- every
    /// name it puts in `bound_vars` for the first time is recorded in `introduced` for its caller
    /// to take back out.
    fn parse_abstraction_in_scope(
        &mut self,
        separator: &str,
        introduced: &mut Vec<String>,
    ) -> Result<(Vec<(String, Term, BinderKind)>, Term), ParseError> {
        let mut binders: Vec<(String, Option<Term>, BinderKind)> = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek_str(separator) || self.at_binder_annotation() {
                break;
            }
            let (names, declared, kind) = match self.abstraction_bracket() {
                Some((kind, open, close)) => {
                    self.consume_bracket(open);
                    let (names, declared) = self.parse_abstraction_group(kind, close)?;
                    (names, declared, kind)
                }
                None => (vec![self.parse_ident()?], None, BinderKind::Explicit),
            };
            for name in names {
                if self.bound_vars.insert(name.clone()) {
                    introduced.push(name.clone());
                }
                binders.push((name, declared.clone(), kind));
            }
        }
        if binders.is_empty() {
            return Err(ParseError::Expected {
                expected: "at least one binder".to_string(),
                found: self.found_here(),
            });
        }
        let (shared, body) = self.finish_abstraction(separator)?;
        let binders = binders
            .into_iter()
            .map(|(name, ty, kind)| {
                (
                    name,
                    ty.or_else(|| shared.clone()).unwrap_or(Term::Hole),
                    kind,
                )
            })
            .collect();
        Ok((binders, body))
    }

    /// The BRACKET a binder group of an abstraction telescope opens with, and the [`BinderKind`]
    /// that bracket declares -- or `None` at a bare identifier binder.
    ///
    /// ★ A TELESCOPE COULD ONLY BIND EXPLICITLY. `fun` and `forall` read `(x y : T)` and a bare
    /// name and nothing else, so `theorem foo {α} [Monoid α] (a : α)` -- the shape of very nearly
    /// every declaration in mathlib -- had no spelling in an abstraction at all. The Pi-position
    /// grammar has read all four brackets since wave 2; this is the same four, at the one place
    /// they were missing. A `⦃` or `[` that opens a NOTATION in scope is not a bracket here,
    /// exactly as in [`Self::parse_term`]: a user's own notation must keep winning where it
    /// always did, or a new builtin changes the meaning of existing text instead of adding to it.
    fn abstraction_bracket(&self) -> Option<(BinderKind, &'static str, &'static str)> {
        if self.peek_char('(') {
            Some((BinderKind::Explicit, "(", ")"))
        } else if self.at_bracket(Self::STRICT_IMPLICIT_OPEN_ASCII) {
            // `funStrictImplicitBinder <|> funImplicitBinder` -- Lean tries the doubled bracket
            // first here too (`Lean/Parser/Term.lean:493`), for the same reason.
            Some((
                BinderKind::StrictImplicit,
                Self::STRICT_IMPLICIT_OPEN_ASCII,
                Self::STRICT_IMPLICIT_CLOSE_ASCII,
            ))
        } else if self.peek_char('{') {
            Some((BinderKind::Implicit, "{", "}"))
        } else if self.peek_char('\u{2983}') && !self.at_atom_notation() {
            Some((BinderKind::StrictImplicit, "\u{2983}", "\u{2984}"))
        } else if self.peek_char('[') && !self.at_atom_notation() {
            Some((BinderKind::Instance, "[", "]"))
        } else {
            None
        }
    }

    /// One binder group of an abstraction telescope, its opening bracket already consumed:
    /// `(x y : T)`, `(x)`, `{A}`, `⦃A : T⦄`, `[inst : C x]`, or the ANONYMOUS `[C x]`.
    ///
    /// Returns the names and the declared type -- `None` where the group writes none, leaving the
    /// type to the telescope's trailing `: T` annotation and, failing that, to the elaborator as a
    /// `Term::Hole`. An INSTANCE group has no such form: `[inst]` names a binder with no class to
    /// resolve it from, so the contents of a `[...]` that is not `name+ :` are read as the class
    /// TYPE and the binder is `_`, which is what Lean writes and what an instance argument is --
    /// found, not named. The lookahead REWINDS, exactly as [`Self::parse_parens`] does, so
    /// `[Monoid A]` (two name-shaped tokens, no colon) is never mistaken for a binder group whose
    /// annotation is missing.
    fn parse_abstraction_group(
        &mut self,
        kind: BinderKind,
        close: &str,
    ) -> Result<(Vec<String>, Option<Term>), ParseError> {
        let after_open = self.pos;
        if let Some(names) = self.try_scan_binder_names() {
            self.skip_whitespace();
            if self.at_binder_annotation() {
                self.advance();
                let declared = self.parse_term()?;
                self.close_binder_group(close, &names)?;
                return Ok((names, Some(declared)));
            }
            if kind != BinderKind::Instance && self.consume_bracket(close) {
                return Ok((names, None));
            }
        }
        if kind == BinderKind::Instance {
            self.pos = after_open;
            let class_type = self.parse_term()?;
            self.close_binder_group(close, &["_".to_string()])?;
            return Ok((vec!["_".to_string()], Some(class_type)));
        }
        Err(ParseError::Expected {
            expected: "a binder name".to_string(),
            found: self.found_here(),
        })
    }

    /// The optional shared annotation, the separator, and the body of an abstraction. Split out so
    /// [`Self::parse_abstraction`] can restore its binder scope on every path, success or failure.
    fn finish_abstraction(&mut self, separator: &str) -> Result<(Option<Term>, Term), ParseError> {
        let shared = if self.at_binder_annotation() {
            self.advance();
            Some(self.parse_term()?)
        } else {
            None
        };
        self.skip_whitespace();
        self.expect_str(separator)?;
        let body = self.parse_term()?;
        Ok((shared, body))
    }

    /// The character at the current position, for an error message.
    fn found_here(&self) -> String {
        self.peek()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "EOF".to_string())
    }

    /// Parse fix: `fix f => body`, or a MUTUAL block `fix f => b with g => c … return g`.
    ///
    /// ★ `Term::MutualFix` HAD NO SPELLING. It is the body of every mutual inductive's derived
    /// eliminator — `Even.rec`'s fixpoint calls `Odd.rec`'s — so every mutual block the kernel
    /// registers holds terms no source text could write. The block reads like the single form it
    /// extends: each member `name => body`, joined by `with` as Coq joins mutual fixpoints, and
    /// closed by `return` naming the member the whole term denotes. Every member's name is bound in
    /// EVERY body. A one-member block is still a block — `fix f => b return f` is `MutualFix`, and
    /// `fix f => b` alone is `Fix` — because the kernel holds the two as different terms.
    fn parse_fix(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("fix")?;
        self.skip_whitespace();
        let name = self.parse_ident()?;
        self.skip_whitespace();
        self.expect_str("=>")?;

        // The recursion name is in scope in the body, and OUT of scope again on every path --
        // the error path included. A binder a FAILED parse leaves behind makes a later `Global` of
        // that spelling read as a `Var`: a name bound by a parse that did not happen. One rule,
        // stated the same way in `parse_body_with`, `parse_abstraction`, `parse_let` and here.
        let was_bound = self.bound_vars.contains(&name);
        self.bound_vars.insert(name.clone());
        let body = self.parse_term();
        if !was_bound {
            self.bound_vars.remove(&name);
        }
        let body = body?;
        self.skip_whitespace();
        if !self.peek_keyword("with") && !self.peek_keyword("return") {
            return Ok(Term::Fix {
                name,
                body: Box::new(body),
            });
        }
        self.parse_mutual_fix(name, body)
    }

    /// The rest of `fix f₀ => b₀ with f₁ => b₁ … return fᵢ`, the first member already read.
    ///
    /// A body can mention a member whose name is written only AFTER it, so no single left-to-right
    /// pass can have every name in scope. Each body is read with the names seen so far bound, and
    /// every remaining `Global` spelled like a member is then turned into that member's variable —
    /// sound because a member name SHADOWS any global of that spelling in every body, exactly as a
    /// binder does, so a `Global` of that spelling can only be one of the members.
    fn parse_mutual_fix(&mut self, first: String, first_body: Term) -> Result<Term, ParseError> {
        let mut defs = vec![(first, first_body)];
        let mut introduced: Vec<String> = Vec::new();
        let parsed = (|| -> Result<String, ParseError> {
            for (n, _) in &defs {
                if self.bound_vars.insert(n.clone()) {
                    introduced.push(n.clone());
                }
            }
            while self.try_consume_keyword("with") {
                self.skip_whitespace();
                let name = self.parse_ident()?;
                self.skip_whitespace();
                self.expect_str("=>")?;
                if self.bound_vars.insert(name.clone()) {
                    introduced.push(name.clone());
                }
                let body = self.parse_term()?;
                self.skip_whitespace();
                defs.push((name, body));
            }
            if !self.try_consume_keyword("return") {
                return Err(ParseError::Expected {
                    expected:
                        "`return <member>` \u{2014} a mutual `fix` names the member it denotes"
                            .to_string(),
                    found: self.found_here(),
                });
            }
            self.skip_whitespace();
            self.parse_ident()
        })();
        for n in &introduced {
            self.bound_vars.remove(n);
        }
        let selected = parsed?;
        let names: Vec<String> = defs.iter().map(|(n, _)| n.clone()).collect();
        let index = match names.iter().filter(|n| **n == selected).count() {
            1 => names
                .iter()
                .position(|n| *n == selected)
                .expect("counted once"),
            0 => {
                return Err(ParseError::Expected {
                    expected: format!("one of the block's members ({})", names.join(", ")),
                    found: selected,
                })
            }
            _ => {
                return Err(ParseError::Unsupported {
                    what: format!("a mutual `fix` naming `{selected}` twice"),
                    why: "`return` could not say which of them the term denotes".to_string(),
                })
            }
        };
        let defs = defs
            .into_iter()
            .map(|(n, b)| (n, Self::bind_members(&b, &names)))
            .collect();
        Ok(Term::MutualFix { defs, index })
    }

    /// `term` with every `Global` spelled like one of `names` turned into that name's variable —
    /// skipping the three places a `Global` is a LABEL rather than a reference: a named argument's
    /// parameter name, a projection's field, and an auto-param's tactic. A raw match clause's
    /// PATTERN is skipped too: a name there is a constructor or a pattern variable, never a member.
    fn bind_members(term: &Term, names: &[String]) -> Term {
        let recur = |t: &Term| Self::bind_members(t, names);
        if let Some((pattern, body)) = super::raw_match::as_case(term) {
            return super::raw_match::case(pattern.clone(), recur(body));
        }
        match term {
            Term::Global(n) if names.contains(n) => Term::Var(n.clone()),
            Term::App(f, x) => {
                if let Term::App(head, first) = f.as_ref() {
                    if let Term::Global(m) = head.as_ref() {
                        if m == Self::NAMED_ARG_MARKER || m == Self::AUTO_PARAM_MARKER {
                            let (label, payload) = if m == Self::NAMED_ARG_MARKER {
                                (first.as_ref().clone(), recur(x))
                            } else {
                                (recur(first), x.as_ref().clone())
                            };
                            return Term::App(
                                Box::new(Term::App(head.clone(), Box::new(label))),
                                Box::new(payload),
                            );
                        }
                        if m == crate::elaborate::DOT_MARKER {
                            return Term::App(
                                Box::new(Term::App(head.clone(), Box::new(recur(first)))),
                                x.clone(),
                            );
                        }
                    }
                }
                Term::App(Box::new(recur(f)), Box::new(recur(x)))
            }
            Term::Pi {
                param,
                param_type,
                body_type,
            } => Term::Pi {
                param: param.clone(),
                param_type: Box::new(recur(param_type)),
                body_type: Box::new(recur(body_type)),
            },
            Term::Lambda {
                param,
                param_type,
                body,
            } => Term::Lambda {
                param: param.clone(),
                param_type: Box::new(recur(param_type)),
                body: Box::new(recur(body)),
            },
            Term::Match {
                discriminant,
                motive,
                cases,
            } => Term::Match {
                discriminant: Box::new(recur(discriminant)),
                motive: Box::new(recur(motive)),
                cases: cases.iter().map(recur).collect(),
            },
            Term::Fix { name, body } => Term::Fix {
                name: name.clone(),
                body: Box::new(recur(body)),
            },
            Term::MutualFix { defs, index } => Term::MutualFix {
                defs: defs.iter().map(|(n, b)| (n.clone(), recur(b))).collect(),
                index: *index,
            },
            Term::Let {
                name,
                ty,
                value,
                body,
            } => Term::Let {
                name: name.clone(),
                ty: Box::new(recur(ty)),
                value: Box::new(recur(value)),
                body: Box::new(recur(body)),
            },
            Term::Annotated { annotation, term } => Term::Annotated {
                annotation: annotation.clone(),
                term: Box::new(recur(term)),
            },
            other => other.clone(),
        }
    }

    /// Parse match: `match e (return M)? with | C x => t | …` — constructor PATTERNS, compiled
    /// against the context later — or the POSITIONAL form `match e return M with c₀ | c₁ … end`.
    ///
    /// ★ THE KERNEL'S OWN `Term::Match` HAD NO SPELLING. A pattern clause names a constructor, so
    /// it can only be compiled once the context says which constructors exist and in what order;
    /// and a compiled match is not in general the term it was compiled from (the compiler chooses
    /// its own motive, binder types and case nesting). So a kernel match — one case per constructor,
    /// in registration order, each a function of the constructor's fields — could be CHECKED but
    /// not WRITTEN, and every projection the Lean importer lowers is one. The positional form is
    /// that term, read with no context at all: the cases are plain terms after `with` with NO
    /// leading bar (which is what tells the two forms apart), separated by `|` and closed by `end`,
    /// and the motive is required because the kernel's match always has one.
    fn parse_match(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("match")?;
        self.skip_whitespace();
        let discriminant = self.parse_app()?;
        self.skip_whitespace();
        // The `return <motive>` clause is OPTIONAL: when omitted, the motive is left as a
        // `Hole` for the elaborator to infer (`match e with …`).
        let motive = if self.try_consume_keyword("return") {
            self.skip_whitespace();
            let m = self.parse_app()?;
            self.skip_whitespace();
            m
        } else {
            Term::Hole
        };
        self.consume_keyword("with")?;
        self.skip_whitespace();

        let mut cases = Vec::new();
        if self.at_positional_case() {
            loop {
                cases.push(self.parse_term()?);
                self.skip_whitespace();
                if !self.try_consume("|") {
                    break;
                }
            }
        } else {
            loop {
                self.skip_whitespace();
                if !self.try_consume("|") {
                    break;
                }
                self.skip_whitespace();

                // Parse pattern: C x y z (constructor with binders)
                let case_term = self.parse_case_body()?;
                cases.push(case_term);
            }
        }

        // Consume optional 'end' keyword
        self.skip_whitespace();
        let _ = self.try_consume_keyword("end");

        Ok(Term::Match {
            discriminant: Box::new(discriminant),
            motive: Box::new(motive),
            cases,
        })
    }

    /// After `with`: does a POSITIONAL case start here? Anything that begins a term does — which
    /// is everything except the pattern form's `|`, the closing `end`, a closing bracket, a
    /// separator or the end of the input, all of which leave the match with no cases, exactly as
    /// they always did.
    fn at_positional_case(&self) -> bool {
        match self.peek() {
            None => false,
            Some('|' | ')' | ']' | '}' | ',' | ';' | '\u{27e9}' | '\u{2984}') => false,
            Some('.') => false,
            Some(_) => {
                !["end", "with", "return", "in"]
                    .iter()
                    .any(|k| self.peek_keyword(k))
                    && !self.peek_str(":=")
                    && !self.peek_str("=>")
            }
        }
    }

    /// Parse one pattern and keep its labels until the context can compile it.
    fn parse_case_body(&mut self) -> Result<Term, ParseError> {
        let pattern = self.parse_match_pattern()?;
        self.skip_whitespace();
        self.expect_str("=>")?;
        let mut binders = Vec::new();
        super::raw_match::candidates(&pattern, &mut binders);
        let newly_bound: Vec<_> = binders
            .iter()
            .filter(|name| !self.bound_vars.contains(*name))
            .cloned()
            .collect();
        self.bound_vars.extend(binders.iter().cloned());
        let parsed = self.parse_term();
        for name in newly_bound {
            self.bound_vars.remove(&name);
        }
        let mut body = parsed?;
        for binder in binders.into_iter().rev() {
            body = Term::Lambda {
                param: binder,
                param_type: Box::new(Term::Hole),
                body: Box::new(body),
            };
        }
        Ok(super::raw_match::case(pattern, body))
    }

    fn parse_match_pattern(&mut self) -> Result<Term, ParseError> {
        let mut pattern = self.parse_match_pattern_atom()?;
        self.skip_whitespace();
        if !self.peek_str("=>") && !self.peek_char(')') {
            if let Term::Var(name) = pattern {
                pattern = Term::Global(name);
            } else if !matches!(pattern, Term::Global(_)) {
                return Err(ParseError::Expected {
                    expected: "constructor at the head of a pattern application".into(),
                    found: format!("{pattern}"),
                });
            }
            while !self.peek_str("=>") && !self.peek_char(')') {
                let argument = self.parse_match_pattern_atom()?;
                pattern = Term::App(Box::new(pattern), Box::new(argument));
                self.skip_whitespace();
            }
        }
        Ok(pattern)
    }

    fn parse_match_pattern_atom(&mut self) -> Result<Term, ParseError> {
        self.skip_whitespace();
        if self.try_consume("(") {
            let pattern = self.parse_match_pattern()?;
            self.skip_whitespace();
            self.expect_str(")")?;
            return Ok(pattern);
        }
        if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.advance();
            }
            let text = &self.input[start..self.pos];
            let n = text.parse::<u64>().map_err(|_| ParseError::Expected {
                expected: "representable natural-number pattern".into(),
                found: text.into(),
            })?;
            return Ok(Term::Lit(Literal::Nat(crate::BigInt::from_u64(n))));
        }
        let mut parts = vec![self.parse_ident()?];
        while self.peek_char('.') {
            let rest = &self.input[self.pos + 1..];
            let Some((component, len)) = scan_name(rest) else {
                break;
            };
            self.pos += 1 + len;
            parts.push(component);
        }
        let name = super::surface::Name::from_parts(parts)
            .expect("nonempty name")
            .key();
        if name == "_" {
            Ok(Term::Hole)
        } else {
            Ok(Term::Var(name))
        }
    }

    // =========================================================================
    // Low-level parsing utilities
    // =========================================================================

    /// [`Self::parse_ident`], also reporting whether the name was `«quoted»` — see
    /// [`scan_spelled_name`].
    fn parse_ident_spelled(&mut self) -> Result<(String, bool), ParseError> {
        self.skip_whitespace();
        let quoted = self.peek_char('\u{ab}');
        self.parse_ident().map(|name| (name, quoted))
    }

    /// Parse an identifier (alphanumeric + underscore, starting with letter/underscore)
    fn parse_ident(&mut self) -> Result<String, ParseError> {
        self.skip_whitespace();
        match scan_name(&self.input[self.pos..]) {
            Some((name, len)) => {
                self.pos += len;
                Ok(name)
            }
            None => match self.peek() {
                Some(c) => Err(ParseError::Expected {
                    expected: "identifier".to_string(),
                    found: format!("{}", c),
                }),
                None => Err(ParseError::UnexpectedEof),
            },
        }
    }

    /// The explicit UNIVERSE LEVEL after `Type`, if one is written: `Ok(None)` when no digits
    /// stand here, and an ERROR when digits stand here that no level can hold.
    ///
    /// A LEVEL THAT DOES NOT FIT IS NOT LEVEL ZERO. This used to consume the digits and return
    /// `None` on overflow, and the caller reads `None` as "no level was written" -- so
    /// `Type 99999999999999999999` became `Type 0`, silently, with the digits gone and the
    /// completeness guard satisfied because nothing was left over. The kernel then certified a
    /// claim at a universe nobody wrote. Refusing is the only honest answer: `Universe::Type`
    /// holds a `u32`, so a wider level is a level this system does not have.
    fn parse_universe_level(&mut self) -> Result<Option<u32>, ParseError> {
        self.skip_whitespace();
        let start = self.pos;

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }

        if self.pos == start {
            return Ok(None);
        }
        let digits = &self.input[start..self.pos];
        digits
            .parse()
            .map(Some)
            .map_err(|_| ParseError::InvalidNumber(digits.to_string()))
    }

    /// Skip whitespace
    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Peek at current character
    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    /// Check if we're at a specific character
    fn peek_char(&self, c: char) -> bool {
        self.peek() == Some(c)
    }

    /// Check if input starts with a string at current position
    fn peek_str(&self, s: &str) -> bool {
        self.input[self.pos..].starts_with(s)
    }

    /// The byte offset just past the binder bracket `bracket`, or `None` if it is not here.
    ///
    /// ★ A BINDER BRACKET MAY BE TWO OF THEM. Lean's strict-implicit brackets have an ASCII
    /// spelling alongside the guillemets -- `strictImplicitLeftBracket := atomic (group (symbol
    /// "{" >> "{")) <|> "⦃"` (`Lean/Parser/Term.lean:346`), and its `⦄` counterpart on the next
    /// line -- so `{{x y : A}}` is `⦃x y : A⦄`, which is the only spelling available to a writer
    /// with no unicode input method. Those are two separate TOKENS in Lean's grammar, and Lean
    /// tokenises across whitespace, so `{ {x : A} }` is the same binder; this walk says the same
    /// by skipping between the halves and not before the first.
    fn bracket_end(&self, bracket: &str) -> Option<usize> {
        let mut i = self.pos;
        for (n, c) in bracket.chars().enumerate() {
            if n > 0 {
                let rest = &self.input[i..];
                i += rest.len() - rest.trim_start().len();
            }
            if !self.input[i..].starts_with(c) {
                return None;
            }
            i += c.len_utf8();
        }
        Some(i)
    }

    /// Does the binder bracket `bracket` open (or close) here? Non-consuming.
    fn at_bracket(&self, bracket: &str) -> bool {
        self.bracket_end(bracket).is_some()
    }

    /// Consume the binder bracket `bracket` if it is here, leaving the position untouched if not.
    fn consume_bracket(&mut self, bracket: &str) -> bool {
        match self.bracket_end(bracket) {
            Some(end) => {
                self.pos = end;
                true
            }
            None => false,
        }
    }

    /// Check if input starts with a keyword (followed by non-alphanumeric)
    fn peek_keyword(&self, keyword: &str) -> bool {
        if !self.peek_str(keyword) {
            return false;
        }
        // Check that keyword is not part of a longer identifier
        let after = self.pos + keyword.len();
        if after >= self.input.len() {
            return true;
        }
        let next_char = self.input[after..].chars().next();
        !next_char
            .map(|c| c.is_alphanumeric() || c == '_')
            .unwrap_or(false)
    }

    /// Advance position by one character
    fn advance(&mut self) {
        if let Some(c) = self.peek() {
            self.pos += c.len_utf8();
        }
    }

    /// Check if at end of input
    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    /// Try to consume a string, returning true if successful
    fn try_consume(&mut self, s: &str) -> bool {
        if self.peek_str(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    /// Try to consume a keyword, returning true if successful
    fn try_consume_keyword(&mut self, keyword: &str) -> bool {
        self.skip_whitespace();
        if self.peek_keyword(keyword) {
            self.pos += keyword.len();
            true
        } else {
            false
        }
    }

    /// Expect a specific character
    fn expect_char(&mut self, expected: char) -> Result<(), ParseError> {
        self.skip_whitespace();
        if self.peek_char(expected) {
            self.advance();
            Ok(())
        } else {
            Err(ParseError::Expected {
                expected: format!("'{}'", expected),
                found: self
                    .peek()
                    .map(|c| c.to_string())
                    .unwrap_or("EOF".to_string()),
            })
        }
    }

    /// Expect a specific string
    fn expect_str(&mut self, expected: &str) -> Result<(), ParseError> {
        self.skip_whitespace();
        if self.try_consume(expected) {
            Ok(())
        } else {
            let found: String = self.input[self.pos..]
                .chars()
                .take(expected.len())
                .collect();
            Err(ParseError::Expected {
                expected: format!("'{}'", expected),
                found: if found.is_empty() {
                    "EOF".to_string()
                } else {
                    found
                },
            })
        }
    }

    /// Consume a keyword (must be followed by non-alphanumeric)
    fn consume_keyword(&mut self, keyword: &str) -> Result<(), ParseError> {
        self.skip_whitespace();
        if self.peek_keyword(keyword) {
            self.pos += keyword.len();
            Ok(())
        } else {
            Err(ParseError::Expected {
                expected: keyword.to_string(),
                found: self
                    .peek()
                    .map(|c| c.to_string())
                    .unwrap_or("EOF".to_string()),
            })
        }
    }

    /// Get remaining input (for debugging)
    #[allow(dead_code)]
    fn remaining(&self) -> &str {
        &self.input[self.pos..]
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/term_parser/tests.rs"]
mod tests;
