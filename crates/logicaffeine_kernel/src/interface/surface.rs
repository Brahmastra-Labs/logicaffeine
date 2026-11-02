//! The SURFACE environment — user-declared notation, fixity and macros.
//!
//! A mathematician coins notation constantly, and the vernacular had exactly ONE infix table,
//! hard-coded, with no extension point. This module is that extension point, and the shape of it
//! is the whole design:
//!
//! * **A parser-extension command declares into the SURFACE, never into the namespace.** The
//!   kernel context holds declarations; this holds ways of spelling them. `Notation "a ⊕ b" := …`
//!   adds no global — not the token, not the command word — because a flat namespace with one
//!   phantom per notation is a collision waiting to happen.
//! * **Two stages, because they are two different things.** A macro is a PATTERN REWRITE and can
//!   be recursive, so it runs as a source-to-source pre-pass to a fixpoint. A notation is a fixed
//!   shape with a PRECEDENCE, so it is a layer in the term parser. Lean stages them the same way
//!   and for the same reason.
//! * **Scope is the section stack.** A declaration lives in the innermost open section and dies
//!   with it, so one token can mean two things in two sections without either shadowing the other.
//!   `Local` is the explicit spelling of what already happens.
//! * **A namespace is the purest case of "a spelling, not a declaration".** `Namespace`/`End`/`Open`
//!   change which constant a short name DENOTES and add nothing to the context. Lean agrees: its
//!   kernel `Environment` maps absolute names to constants and the namespace scope lives in the
//!   elaborator, with `resolveGlobalName` running to completion before anything reaches the kernel.
//!   So it lives here and the resolution runs in the `Repl`, which is why the kernel can stay a flat
//!   map of absolute keys and know nothing about any of it.
//! * **Hygiene is not optional.** A template's own binders are renamed to names that do not occur
//!   anywhere in the expansion before its arguments are spliced in, so a template binder can never
//!   capture a call-site name. Without it `Macro "twiceIt $e" := (fun x : Int => … $e …)` applied
//!   at a call site that also binds `x` computes a silently wrong answer — not a type error.

use std::collections::{HashMap, HashSet};

use super::error::ParseError;
use super::term_parser::{scan_name, BinderKind, BinderSurface, TermParser};
use crate::Term;

/// The precedence an infix notation gets when it does not declare one, and the associativity that
/// comes with it. Two notations at the same level then nest to the RIGHT, so `a ⊕ b ⊗ c` groups as
/// `a ⊕ (b ⊗ c)` — the reading a reader gives an unparenthesised chain of unfamiliar operators.
const DEFAULT_PRECEDENCE: u32 = 65;

/// The precedence a PREFIX or POSTFIX notation gets. Neither has a second operand to compete for,
/// so both bind as tightly as anything can: `1 \u{22c4} 7 \u{203c}` applies the postfix to `7`, not
/// to the whole spine.
const TIGHT_PRECEDENCE: u32 = 1000;

/// How many rewrite steps a macro expansion may take before it is declared non-terminating. A
/// recursive `macro_rules` that never reaches its base case must fail LOUDLY rather than hang.
const MAX_MACRO_STEPS: usize = 1024;

/// One piece of a notation or macro pattern: a literal token to match, or a named hole to capture.
#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Tok(String),
    Hole(String),
}

/// Where a notation's holes sit relative to its tokens. Decided by the pattern, not declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// `a ⊕ b` — a hole, tokens, a hole. Parsed in the operator loop, by precedence.
    Infix,
    /// `∽ a` — leading token. Parsed at atom position.
    Prefix,
    /// `a ‼` — trailing token. Parsed in the operator loop, tighter than any infix.
    Postfix,
    /// `⟦ n ⟧` — tokens on both ends. Parsed at atom position; brackets need no precedence.
    Closed,
}

/// Which way a chain of one operator nests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assoc {
    /// `a ⋄ b ⋄ c` = `f (f a b) c`.
    Left,
    /// `a ▷ b ▷ c` = `g a (g b c)`.
    Right,
}

/// A declaration that extends the SURFACE rather than the context. Emitted by the command parser,
/// applied by the REPL — so the one place a parser extension can land is the surface environment.
#[derive(Debug, Clone)]
pub enum SurfaceDecl {
    /// `Notation "a ⊕ b" := ((myOp a) b).` and the fixity spellings, which are the same thing with
    /// the shape and associativity given by the command word instead of by the pattern.
    Notation {
        pattern: String,
        template: String,
        precedence: Option<u32>,
        assoc: Assoc,
        /// For `Infixl`/`Infixr`/`Prefix`/`Postfix`, the operator's own spelling: the pattern is
        /// just the token and the holes are implied.
        operator_form: Option<OperatorForm>,
    },
    /// `Macro "twiceIt $e" := …` / `MacroRules "nested![$x, $xs]" := …`.
    Macro { pattern: String, template: String },
    /// `Section First.` — or a bare `Section.`, which is Lean's ANONYMOUS section
    /// (`section` with no name), the ordinary way to scope a `local` without inventing a label for
    /// it. `None` is that form.
    OpenSection(Option<String>),
    /// `End First.` — and, when the innermost open scope is a NAMESPACE of that name, the close of
    /// that namespace instead. Lean spells both with one keyword and decides by what is open;
    /// deciding it here rather than in the parser is what lets the parser stay text-only.
    ///
    /// `None` is the bare `End.` that closes an anonymous section. It closes ONLY an anonymous
    /// one: lean 4.14.0 answers `namespace N … end` with `invalid 'end', name is missing
    /// (expected N)`, and a bare `End` that silently closed a named scope would leave every later
    /// declaration somewhere nobody wrote.
    CloseSection(Option<String>),
    /// `Namespace Outer.Inner.` — a namespace declarations are written INTO and short names resolve
    /// against. Nothing is declared: like every other surface declaration this changes how a name
    /// is spelled and read, never what exists.
    OpenNamespace(Name),
}

/// The shape a fixity command implies for its operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorForm {
    Infix,
    Prefix,
    Postfix,
}

/// A notation in scope.
#[derive(Debug, Clone)]
struct Notation {
    pieces: Vec<Piece>,
    shape: Shape,
    precedence: u32,
    assoc: Assoc,
    /// The expansion, parsed once at declaration time with the holes bound, so an invocation is a
    /// SUBSTITUTION and not a re-parse of spliced text.
    template: Term,
}

/// A macro rule in scope. Its template stays TEXT: an alternative may invoke the macro it is
/// defining, and that call can only be expanded after its arguments are known.
#[derive(Debug, Clone)]
struct MacroRule {
    pieces: Vec<Piece>,
    template: String,
}

/// One section's worth of surface declarations.
#[derive(Debug, Default)]
struct Scope {
    name: Option<String>,
    notations: Vec<Notation>,
    macros: Vec<MacroRule>,
}

/// The surface environment: a stack of scopes, innermost last.
#[derive(Debug)]
pub struct Surface {
    scopes: Vec<Scope>,
    /// Monotonic, so two expansions in one term cannot pick the same hygienic name. Atomic rather
    /// than a `Cell` so a `Surface` can be read from the deep-stack thread a long term is parsed on.
    fresh: std::sync::atomic::AtomicU64,
    /// The NAMESPACE scope: what is current and what is open. It sits beside the notation for the
    /// same reason the notation sits beside the kernel context — a way of spelling a name is not a
    /// declaration, and namespace-relative reference is the purest case of that.
    namespaces: Namespaces,
    /// Per-declaration parameter surface — every entry has at least one DEFAULT VALUE — keyed by
    /// the absolute key the kernel context is indexed at.
    ///
    /// ★ A DEFAULT IS A SPELLING, WHICH IS WHY IT LIVES HERE. `(m : Int := 3)` does not change
    /// what `withDefault` IS: its type is `Int -> Int -> Int` either way and exactly the same
    /// terms inhabit it. What it changes is how an application may be WRITTEN — `withDefault 1`
    /// is a spelling of `withDefault 1 3` — and a way of spelling something is what this module
    /// holds. Storing it in the kernel context would make a calling convention part of a
    /// declaration's identity, which is the same mistake as minting a global per notation.
    ///
    /// Empty for every declaration that writes no default, so the map is empty in every session
    /// that never writes one and [`fill_defaults`] is skipped outright.
    defaults: HashMap<String, Vec<BinderSurface>>,
}

impl Default for Surface {
    fn default() -> Self {
        Self::new()
    }
}

impl Surface {
    /// A surface with nothing declared. The parse path it produces is byte-identical to having no
    /// surface at all, which is what keeps the extension point free when nobody uses it.
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope::default()],
            fresh: std::sync::atomic::AtomicU64::new(0),
            namespaces: Namespaces::new(),
            defaults: HashMap::new(),
        }
    }

    /// Record what `key`'s declaration said about its parameters, for as long as `key` is
    /// declared.
    ///
    /// Only a telescope carrying a DEFAULT is kept: nothing else is a spelling, and keeping the
    /// rest would make [`Surface::fills_defaults`] true for every session and cost a term walk per
    /// command for nothing. A redeclaration REPLACES the entry, and a redeclaration that dropped
    /// the default REMOVES it — a stale default would fill an argument the current declaration
    /// never offered.
    pub(super) fn record_defaults(&mut self, key: &str, binders: Vec<BinderSurface>) {
        if binders.iter().any(|b| b.default.is_some()) {
            self.defaults.insert(key.to_string(), binders);
        } else {
            self.defaults.remove(key);
        }
    }

    /// `key`'s parameter surface, if any parameter of it carries a default.
    pub(super) fn defaults_of(&self, key: &str) -> Option<&[BinderSurface]> {
        self.defaults.get(key).map(|b| b.as_slice())
    }

    /// Has this session declared ANY default at all? The guard that keeps [`fill_defaults`] off
    /// the path of every session that has not.
    pub(super) fn fills_defaults(&self) -> bool {
        !self.defaults.is_empty()
    }

    /// The namespace scope: what is current, what is open.
    pub fn namespaces(&self) -> &Namespaces {
        &self.namespaces
    }

    /// The namespace scope, to push and wind back a SCOPED opening — `Open … in <command>`, and a
    /// declaration whose own name names the namespace it is written into.
    pub fn namespaces_mut(&mut self) -> &mut Namespaces {
        &mut self.namespaces
    }

    /// Enter the scope a DECLARATION named `declared` elaborates in, and give back the absolute key
    /// it declares. See [`Namespaces::enter_declaration`]; this is the door, because only the surface
    /// knows how deep the section stack is, and that is what orders the two scope stacks.
    pub fn enter_declaration(&mut self, declared: &Name) -> (String, ScopeMark) {
        let depth = self.scopes.len();
        self.namespaces.enter_declaration(declared, depth)
    }

    /// Is anything declared? A surface with no notations and no macros is skipped entirely.
    pub fn is_empty(&self) -> bool {
        self.scopes
            .iter()
            .all(|s| s.notations.is_empty() && s.macros.is_empty())
    }

    /// How many SECTIONS are open. The root scope is never pushed and never popped, so it is not
    /// one of them.
    pub fn section_depth(&self) -> usize {
        self.scopes.len() - 1
    }

    /// The open sections, outermost first \u{2014} what a document must close before it ends. An
    /// anonymous section is named AS one rather than as the empty string, so an unbalanced
    /// document reports something a reader can act on.
    pub fn section_names(&self) -> Vec<String> {
        self.scopes
            .iter()
            .skip(1)
            .map(|s| {
                s.name
                    .clone()
                    .unwrap_or_else(|| ANONYMOUS_SECTION.to_string())
            })
            .collect()
    }

    /// Close every section opened since `depth`. Only ever SHRINKS: a section already closed took
    /// its notations with it and cannot be brought back, which is why an over-closing document is
    /// reported rather than silently repaired.
    pub fn close_sections_to(&mut self, depth: usize) {
        self.scopes.truncate(depth + 1);
    }

    /// Apply one surface declaration.
    pub fn declare(&mut self, decl: SurfaceDecl) -> Result<(), ParseError> {
        match decl {
            SurfaceDecl::OpenSection(name) => {
                self.scopes.push(Scope {
                    name,
                    ..Scope::default()
                });
                Ok(())
            }
            SurfaceDecl::OpenNamespace(name) => {
                let depth = self.scopes.len();
                self.namespaces.enter(&name, depth);
                Ok(())
            }
            SurfaceDecl::CloseSection(name) => {
                // ★ ONE `End` KEYWORD, AND THE INNERMOST OPEN SCOPE DECIDES — Lean's rule, and it
                // has to be asked in that ORDER. This asked the namespace stack FIRST, which closed
                // a namespace straight THROUGH an open section: `Namespace N. Section S. End N.`
                // popped `N` and left `S` open, so every later declaration was keyed outside the
                // namespace its author was writing in and the file still checked. lean 4.14.0
                // refuses that exact program with `invalid 'end', name mismatch (expected S)`.
                let written = end_spelling(&name);
                if let Some(open) = self.namespaces.innermost_scope(self.scopes.len()).cloned() {
                    // A NAMESPACE is the innermost open scope: only its own name closes it, and a
                    // bare `End` does not (lean: "name is missing (expected N)").
                    if name.as_deref().and_then(Name::parse).as_ref() == Some(&open) {
                        self.namespaces.leave(&open);
                        return Ok(());
                    }
                    return Err(ParseError::Expected {
                        expected: format!(
                            "`End {open}` — the innermost open scope is the namespace {open}"
                        ),
                        found: written,
                    });
                }
                if self.scopes.len() <= 1 {
                    return Err(ParseError::Expected {
                        expected: "an open section or namespace".to_string(),
                        found: written,
                    });
                }
                let open = self
                    .scopes
                    .last()
                    .expect("the length was checked")
                    .name
                    .clone();
                if open == name {
                    self.scopes.pop();
                    return Ok(());
                }
                Err(ParseError::Expected {
                    expected: match &open {
                        Some(open) => format!("`End {open}` — sections close innermost first"),
                        None => format!(
                            "a bare `End` — the innermost open scope is {ANONYMOUS_SECTION}"
                        ),
                    },
                    found: written,
                })
            }
            SurfaceDecl::Macro { pattern, template } => {
                let pieces = macro_pattern(&pattern)?;
                if !matches!(pieces.first(), Some(Piece::Tok(_))) {
                    return Err(ParseError::Expected {
                        expected: "a macro pattern beginning with a literal token".to_string(),
                        found: pattern,
                    });
                }
                // A LATER block is tried FIRST: `macro_rules` prepends its alternatives, so the
                // most recent declaration wins. Innermost-first lookup already gives that inside a
                // scope, since the search runs back to front.
                self.scopes
                    .last_mut()
                    .expect("the root scope is never popped")
                    .macros
                    .push(MacroRule { pieces, template });
                Ok(())
            }
            SurfaceDecl::Notation {
                pattern,
                template,
                precedence,
                assoc,
                operator_form,
            } => {
                let pieces = match operator_form {
                    None => notation_pattern(&pattern)?,
                    Some(OperatorForm::Infix) => vec![
                        Piece::Hole("a".to_string()),
                        Piece::Tok(pattern.trim().to_string()),
                        Piece::Hole("b".to_string()),
                    ],
                    Some(OperatorForm::Prefix) => vec![
                        Piece::Tok(pattern.trim().to_string()),
                        Piece::Hole("a".to_string()),
                    ],
                    Some(OperatorForm::Postfix) => vec![
                        Piece::Hole("a".to_string()),
                        Piece::Tok(pattern.trim().to_string()),
                    ],
                };
                let shape = shape_of(&pieces)?;
                let holes: Vec<String> = pieces
                    .iter()
                    .filter_map(|p| match p {
                        Piece::Hole(h) => Some(h.clone()),
                        Piece::Tok(_) => None,
                    })
                    .collect();

                // An operator form names a FUNCTION; the expansion is that function applied to the
                // holes in order. A `Notation` writes its expansion out.
                let template = match operator_form {
                    None => TermParser::parse_template(&template, &holes)?,
                    Some(_) => {
                        let f = TermParser::parse_template(&template, &[])?;
                        holes.iter().fold(f, |acc, h| {
                            Term::App(Box::new(acc), Box::new(Term::Var(h.clone())))
                        })
                    }
                };

                self.scopes
                    .last_mut()
                    .expect("the root scope is never popped")
                    .notations
                    .push(Notation {
                        pieces,
                        shape,
                        precedence: precedence.unwrap_or(match shape {
                            Shape::Prefix | Shape::Postfix | Shape::Closed => TIGHT_PRECEDENCE,
                            Shape::Infix => DEFAULT_PRECEDENCE,
                        }),
                        assoc,
                        template,
                    });
                Ok(())
            }
        }
    }

    /// The notations in scope, innermost section first and most recent declaration first — so the
    /// same token may mean one thing in one section and another thing in the next.
    fn notations(&self) -> impl Iterator<Item = &Notation> {
        self.scopes
            .iter()
            .rev()
            .flat_map(|s| s.notations.iter().rev())
    }

    /// The macro rules in scope, in the order they must be TRIED: a later block's alternatives
    /// come first.
    fn macros(&self) -> impl Iterator<Item = &MacroRule> {
        self.scopes.iter().rev().flat_map(|s| s.macros.iter().rev())
    }

    // ========================================================================================
    // STAGE 1 — MACRO EXPANSION (source to source, to a fixpoint)
    // ========================================================================================

    /// Rewrite every macro invocation in `text` until none remains.
    ///
    /// Text in, text out, because an alternative may invoke the macro it is defining
    /// (`nested![$x, $xs]` expands to a `mycons` applied to `nested![$xs]`) and that inner call
    /// can only be matched once its argument is known. The step budget is a hard backstop: a
    /// recursive rule that never reaches its base case FAILS rather than hangs.
    pub fn expand_macros(&self, text: &str) -> Result<String, ParseError> {
        if self.scopes.iter().all(|s| s.macros.is_empty()) {
            return Ok(text.to_string());
        }
        let mut current = text.to_string();
        let mut steps = 0usize;
        loop {
            match self.expand_once(&current)? {
                None => return Ok(current),
                Some(next) => {
                    steps += 1;
                    if steps > MAX_MACRO_STEPS {
                        return Err(ParseError::MacroDepth {
                            limit: MAX_MACRO_STEPS,
                        });
                    }
                    current = next;
                }
            }
        }
    }

    /// Rewrite the LEFTMOST macro invocation, or `None` if there is none.
    fn expand_once(&self, text: &str) -> Result<Option<String>, ParseError> {
        let mut at = 0usize;
        let mut in_string = false;
        while at < text.len() {
            if !text.is_char_boundary(at) {
                at += 1;
                continue;
            }
            // A string literal is DATA. Rewriting a macro head inside one would change a value
            // rather than a spelling, which is the one thing an expander must never do.
            if text.as_bytes()[at] == b'"' {
                in_string = !in_string;
                at += 1;
                continue;
            }
            if in_string {
                at += 1;
                continue;
            }
            // A macro head is a token, so it may only start where a token may start.
            if at > 0 && is_word_byte(text.as_bytes()[at - 1]) && is_word_start(&text[at..]) {
                at += 1;
                continue;
            }
            for rule in self.macros() {
                if let Some((end, bindings)) = match_pieces(&rule.pieces, text, at) {
                    let body = self.instantiate_text(&rule.template, &bindings);
                    let mut out = String::with_capacity(text.len() + body.len());
                    out.push_str(&text[..at]);
                    out.push_str(&body);
                    out.push_str(&text[end..]);
                    return Ok(Some(out));
                }
            }
            at += 1;
        }
        Ok(None)
    }

    /// Splice captured arguments into a macro template, hygienically.
    ///
    /// The template's OWN binders are renamed first, to names that occur nowhere in the template
    /// or in any argument, so the splice that follows cannot capture a call-site name.
    fn instantiate_text(&self, template: &str, bindings: &HashMap<String, String>) -> String {
        let mut avoid: HashSet<String> = identifiers_in(template);
        for v in bindings.values() {
            avoid.extend(identifiers_in(v));
        }
        let renamed = self.freshen_text_binders(template, &avoid);
        substitute_holes_in_text(&renamed, bindings)
    }

    /// Rename every binder the template itself introduces to a name nothing else uses.
    fn freshen_text_binders(&self, template: &str, avoid: &HashSet<String>) -> String {
        let bound = text_binder_names(template);
        if bound.is_empty() {
            return template.to_string();
        }
        let mut map = HashMap::new();
        for name in bound {
            map.insert(name.clone(), self.fresh_name(&name, avoid));
        }
        rename_identifiers_in_text(template, &map)
    }

    /// A name that is a legal identifier and occurs nowhere in `avoid`.
    fn fresh_name(&self, base: &str, avoid: &HashSet<String>) -> String {
        loop {
            let n = self
                .fresh
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let candidate = format!("{base}__hyg{n}");
            if !avoid.contains(&candidate) {
                return candidate;
            }
        }
    }

    // ========================================================================================
    // STAGE 2 — NOTATION (a layer in the term parser)
    // ========================================================================================

    /// The atom-position notation — a `Prefix` or a bracketing `Closed` form — whose leading token
    /// matches at `text`. Searched innermost section first and most recent declaration first, so a
    /// rebinding shadows without deleting.
    pub(super) fn atom_notation_at<'a>(&'a self, text: &str) -> Option<NotationMatch<'a>> {
        self.notations()
            .filter(|n| matches!(n.shape, Shape::Prefix | Shape::Closed))
            .find(|n| match n.pieces.first() {
                Some(Piece::Tok(t)) => token_matches(text, t),
                _ => false,
            })
            .map(|n| NotationMatch { notation: n })
    }

    /// Does a notation token END an application here?
    ///
    /// Any NON-INITIAL literal of a notation in scope — an infix operator, a postfix marker, a
    /// closing bracket — stops the argument list, because it belongs to the enclosing notation
    /// rather than to the application. A notation's OPENING token does not stop anything: it
    /// begins an atom, so an application may still take a bracketed form as an argument.
    pub(super) fn stops_application(&self, text: &str) -> bool {
        self.notations().any(|n| {
            n.pieces.iter().skip(1).any(|p| match p {
                Piece::Tok(t) => token_matches(text, t),
                Piece::Hole(_) => false,
            })
        })
    }

    /// The operator-position notation — `Infix` or `Postfix` — whose operator token matches at
    /// `text` and binds at least as tightly as `min_precedence`.
    pub(super) fn operator_notation_at<'a>(
        &'a self,
        text: &str,
        min_precedence: u32,
    ) -> Option<NotationMatch<'a>> {
        self.notations()
            .filter(|n| matches!(n.shape, Shape::Infix | Shape::Postfix))
            .filter(|n| n.precedence >= min_precedence)
            .find(|n| match n.pieces.get(1) {
                Some(Piece::Tok(t)) => token_matches(text, t),
                _ => false,
            })
            .map(|n| NotationMatch { notation: n })
    }
}

/// One notation the parser has decided to use, with the questions the parser needs to ask it.
pub(super) struct NotationMatch<'a> {
    notation: &'a Notation,
}

impl NotationMatch<'_> {
    pub(super) fn is_closed(&self) -> bool {
        self.notation.shape == Shape::Closed
    }

    /// The literal pieces, in order, and the hole names between them — what the parser consumes.
    pub(super) fn pieces(&self) -> Vec<(Option<String>, Option<String>)> {
        self.notation
            .pieces
            .iter()
            .map(|p| match p {
                Piece::Tok(t) => (Some(t.clone()), None),
                Piece::Hole(h) => (None, Some(h.clone())),
            })
            .collect()
    }

    /// The precedence the right operand of an infix notation is parsed at: one tighter for a
    /// left-associative operator, so a chain of it cannot re-enter on the right.
    pub(super) fn right_precedence(&self) -> u32 {
        match self.notation.assoc {
            Assoc::Left => self.notation.precedence + 1,
            Assoc::Right => self.notation.precedence,
        }
    }

    /// Substitute the parsed arguments into the template.
    ///
    /// The template's binders are freshened first, so after it no binder shares a name with a hole
    /// and the substitution that follows is capture-free by construction.
    pub(super) fn expand(&self, surface: &Surface, args: &HashMap<String, Term>) -> Term {
        let mut avoid: HashSet<String> = HashSet::new();
        collect_identifiers(&self.notation.template, &mut avoid);
        for t in args.values() {
            collect_identifiers(t, &mut avoid);
        }
        let mut renames = HashMap::new();
        let freshened = freshen_binders(&self.notation.template, surface, &avoid, &mut renames);
        substitute_vars(&freshened, args)
    }
}

// ============================================================================================
// PATTERNS
// ============================================================================================

/// Read a NOTATION pattern: whitespace-separated pieces, where an identifier-shaped piece is a
/// hole and everything else is a literal token. `"a \u{2295} b"` is two holes around an operator;
/// `"\u{27e6} n \u{27e7}"` is a bracketing form.
fn notation_pattern(pattern: &str) -> Result<Vec<Piece>, ParseError> {
    let pieces: Vec<Piece> = pattern
        .split_whitespace()
        .map(|tok| match scan_name(tok) {
            Some((name, len)) if len == tok.len() => Piece::Hole(name),
            _ => Piece::Tok(tok.to_string()),
        })
        .collect();
    if pieces.is_empty() {
        return Err(ParseError::Missing("a notation pattern".to_string()));
    }
    if !pieces.iter().any(|p| matches!(p, Piece::Tok(_))) {
        return Err(ParseError::Expected {
            expected: "a notation pattern with at least one literal token".to_string(),
            found: pattern.to_string(),
        });
    }
    Ok(pieces)
}

/// Read a MACRO pattern. Holes are written `$name`; everything else is a literal token, and the
/// tokens are NOT whitespace-separated — `nested![$x, $xs]` is `nested![`, `$x`, `,`, `$xs`, `]`,
/// because a macro head runs straight into its opening bracket.
fn macro_pattern(pattern: &str) -> Result<Vec<Piece>, ParseError> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut rest = pattern;
    while !rest.is_empty() {
        let c = rest.chars().next().expect("non-empty");
        if c.is_whitespace() {
            if !literal.is_empty() {
                pieces.push(Piece::Tok(std::mem::take(&mut literal)));
            }
            rest = &rest[c.len_utf8()..];
            continue;
        }
        if c == '$' {
            if !literal.is_empty() {
                pieces.push(Piece::Tok(std::mem::take(&mut literal)));
            }
            let after = &rest['$'.len_utf8()..];
            let (name, len) = scan_name(after).ok_or_else(|| ParseError::Expected {
                expected: "a hole name after `$`".to_string(),
                found: after.chars().take(8).collect::<String>(),
            })?;
            pieces.push(Piece::Hole(format!("${name}")));
            rest = &after[len..];
            continue;
        }
        literal.push(c);
        rest = &rest[c.len_utf8()..];
    }
    if !literal.is_empty() {
        pieces.push(Piece::Tok(literal));
    }
    if pieces.is_empty() {
        return Err(ParseError::Missing("a macro pattern".to_string()));
    }
    Ok(pieces)
}

/// Where the holes sit decides how the notation is parsed.
fn shape_of(pieces: &[Piece]) -> Result<Shape, ParseError> {
    let first_is_hole = matches!(pieces.first(), Some(Piece::Hole(_)));
    let last_is_hole = matches!(pieces.last(), Some(Piece::Hole(_)));
    Ok(match (first_is_hole, last_is_hole) {
        (true, true) => Shape::Infix,
        (false, true) => Shape::Prefix,
        (true, false) => Shape::Postfix,
        (false, false) => Shape::Closed,
    })
}

// ============================================================================================
// MACRO MATCHING (text level)
// ============================================================================================

/// Match `pieces` against `text` starting at `at`, returning where the match ended and what each
/// hole captured.
///
/// A hole captures the text up to its FOLLOWING literal token at bracket depth zero — which is why
/// `nested![$x, $xs]` splits `1, 2, 3` into `1` and `2, 3` rather than demanding three holes. A
/// trailing hole captures up to the enclosing close bracket, so `(bump 5)` binds `5` and not `5)`.
fn match_pieces(
    pieces: &[Piece],
    text: &str,
    at: usize,
) -> Option<(usize, HashMap<String, String>)> {
    let mut pos = at;
    let mut bindings = HashMap::new();
    let mut i = 0usize;
    while i < pieces.len() {
        match &pieces[i] {
            Piece::Tok(tok) => {
                let skipped = pos + leading_space(&text[pos..]);
                if !token_matches(&text[skipped..], tok) {
                    return None;
                }
                pos = skipped + tok.len();
            }
            Piece::Hole(name) => {
                let next_tok = match pieces.get(i + 1) {
                    Some(Piece::Tok(t)) => Some(t.as_str()),
                    // Two holes in a row have no separator to stop the first one.
                    Some(Piece::Hole(_)) => return None,
                    None => None,
                };
                let start = pos + leading_space(&text[pos..]);
                match capture_end(text, start, next_tok) {
                    Some(end) => {
                        let captured = text[start..end].trim();
                        if captured.is_empty() {
                            return None;
                        }
                        bindings.insert(name.clone(), captured.to_string());
                        pos = end;
                    }
                    // THE EMPTY TAIL. `nested![$x, $xs]` is how a comma-separated REST is written,
                    // and a one-element list has no comma in it at all: `nested![3]` must bind
                    // `$x` to `3` and `$xs` to nothing, so its expansion bottoms out at
                    // `nested![]`. Without this a recursive list macro never reaches its base
                    // case, and the pattern every `macro_rules` list is written with does not work.
                    None => {
                        let rest_hole = match (pieces.get(i + 1), pieces.get(i + 2)) {
                            (Some(Piece::Tok(_)), Some(Piece::Hole(h))) => h.clone(),
                            _ => return None,
                        };
                        let end = capture_end(text, start, None)?;
                        let captured = text[start..end].trim();
                        if captured.is_empty() {
                            return None;
                        }
                        bindings.insert(name.clone(), captured.to_string());
                        bindings.insert(rest_hole, String::new());
                        pos = end;
                        i += 2;
                    }
                }
            }
        }
        i += 1;
    }
    Some((pos, bindings))
}

/// Where a hole's capture ends: at `stop` when it appears at bracket depth zero, or at the close
/// bracket that ends the enclosing group when the hole is the pattern's last piece.
fn capture_end(text: &str, start: usize, stop: Option<&str>) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    while i < text.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        if depth == 0 {
            match stop {
                Some(tok) if rest.starts_with(tok) => return Some(i),
                None if rest.starts_with([')', ']', '}']) => return Some(i),
                _ => {}
            }
        }
        let c = rest.chars().next().expect("char boundary");
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth < 0 {
                    return if stop.is_none() { Some(i) } else { None };
                }
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    if stop.is_none() && depth == 0 {
        Some(text.len())
    } else {
        None
    }
}

/// Does `text` begin with `tok` as a whole token? A token that ends in a word character needs a
/// word boundary after it, so `bump` does not match inside `bumps`.
fn token_matches(text: &str, tok: &str) -> bool {
    if !text.starts_with(tok) {
        return false;
    }
    match tok.as_bytes().last() {
        Some(&b) if is_word_byte(b) => !is_word_start(&text[tok.len()..]),
        _ => true,
    }
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

fn is_word_start(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '\'')
}

fn leading_space(text: &str) -> usize {
    text.len() - text.trim_start().len()
}

// ============================================================================================
// TEXT-LEVEL IDENTIFIER SURGERY (macro hygiene)
// ============================================================================================

/// Every identifier that occurs in `text`, ignoring string literals and `$hole` names.
fn identifiers_in(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for_each_identifier(text, |name, _, _| {
        out.insert(name.to_string());
    });
    out
}

/// The names a template BINDS: `fun x : T =>`, `forall x : T,`, `fix f =>`, `let x :=`.
fn text_binder_names(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut pending: Option<&'static str> = None;
    for_each_identifier(text, |name, _, _| {
        if pending.take().is_some() {
            out.insert(name.to_string());
            return;
        }
        pending = match name {
            "fun" | "forall" | "fix" | "let" => Some("binder"),
            _ => None,
        };
    });
    out
}

/// Rewrite every standalone identifier that `map` renames.
fn rename_identifiers_in_text(text: &str, map: &HashMap<String, String>) -> String {
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for_each_identifier(text, |name, start, end| {
        if let Some(to) = map.get(name) {
            edits.push((start, end, to.clone()));
        }
    });
    apply_edits(text, edits)
}

/// Replace every `$hole` occurrence with the text it captured.
fn substitute_holes_in_text(text: &str, bindings: &HashMap<String, String>) -> String {
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut in_string = false;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            in_string = !in_string;
            i += 1;
            continue;
        }
        if !in_string && bytes[i] == b'$' {
            if let Some((name, len)) = scan_name(&text[i + 1..]) {
                let key = format!("${name}");
                if let Some(value) = bindings.get(&key) {
                    edits.push((i, i + 1 + len, value.clone()));
                }
                i += 1 + len;
                continue;
            }
        }
        i += 1;
    }
    apply_edits(text, edits)
}

/// Visit every standalone identifier in `text`, skipping string literals and `$hole` names.
fn for_each_identifier(text: &str, mut visit: impl FnMut(&str, usize, usize)) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut in_string = false;
    while i < bytes.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        if bytes[i] == b'"' {
            in_string = !in_string;
            i += 1;
            continue;
        }
        if in_string {
            i += 1;
            continue;
        }
        if bytes[i] == b'$' {
            // A hole is one atom: its name is not an identifier of the template.
            match scan_name(&text[i + 1..]) {
                Some((_, len)) => i += 1 + len,
                None => i += 1,
            }
            continue;
        }
        match scan_name(&text[i..]) {
            Some((name, len)) => {
                visit(&name, i, i + len);
                i += len;
            }
            None => {
                i += text[i..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
}

/// Apply non-overlapping `(start, end, replacement)` edits, right to left.
fn apply_edits(text: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    if edits.is_empty() {
        return text.to_string();
    }
    edits.sort_by_key(|(s, _, _)| *s);
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for (start, end, replacement) in edits {
        if start < last {
            continue;
        }
        out.push_str(&text[last..start]);
        out.push_str(&replacement);
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

// ============================================================================================
// TERM-LEVEL SURGERY (notation hygiene)
// ============================================================================================

/// Every `Var`/`Global` name and every binder name appearing in a term.
fn collect_identifiers(term: &Term, out: &mut HashSet<String>) {
    match term {
        Term::Var(n) | Term::Global(n) => {
            out.insert(n.clone());
        }
        Term::Const { name, .. } => {
            out.insert(name.clone());
        }
        Term::App(f, a) => {
            collect_identifiers(f, out);
            collect_identifiers(a, out);
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            out.insert(param.clone());
            collect_identifiers(param_type, out);
            collect_identifiers(body, out);
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            out.insert(param.clone());
            collect_identifiers(param_type, out);
            collect_identifiers(body_type, out);
        }
        Term::Fix { name, body } => {
            out.insert(name.clone());
            collect_identifiers(body, out);
        }
        Term::MutualFix { defs, .. } => {
            for (n, b) in defs {
                out.insert(n.clone());
                collect_identifiers(b, out);
            }
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            collect_identifiers(discriminant, out);
            collect_identifiers(motive, out);
            for c in cases {
                collect_identifiers(c, out);
            }
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            out.insert(name.clone());
            collect_identifiers(ty, out);
            collect_identifiers(value, out);
            collect_identifiers(body, out);
        }
        Term::Annotated { term, .. } => collect_identifiers(term, out),
        Term::Sort(_) | Term::Lit(_) | Term::Hole => {}
    }
}

/// Rename every binder in `term` to a name that occurs in neither the template nor any argument.
fn freshen_binders(
    term: &Term,
    surface: &Surface,
    avoid: &HashSet<String>,
    renames: &mut HashMap<String, String>,
) -> Term {
    let rename = |renames: &mut HashMap<String, String>, p: &String| -> String {
        let fresh = surface.fresh_name(p, avoid);
        renames.insert(p.clone(), fresh.clone());
        fresh
    };
    match term {
        Term::Var(n) => Term::Var(renames.get(n).cloned().unwrap_or_else(|| n.clone())),
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let ty = freshen_binders(param_type, surface, avoid, renames);
            let fresh = rename(renames, param);
            let b = freshen_binders(body, surface, avoid, renames);
            Term::Lambda {
                param: fresh,
                param_type: Box::new(ty),
                body: Box::new(b),
            }
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let ty = freshen_binders(param_type, surface, avoid, renames);
            let fresh = if param == "_" {
                "_".to_string()
            } else {
                rename(renames, param)
            };
            let b = freshen_binders(body_type, surface, avoid, renames);
            Term::Pi {
                param: fresh,
                param_type: Box::new(ty),
                body_type: Box::new(b),
            }
        }
        Term::Fix { name, body } => {
            let fresh = rename(renames, name);
            Term::Fix {
                name: fresh,
                body: Box::new(freshen_binders(body, surface, avoid, renames)),
            }
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            let t = freshen_binders(ty, surface, avoid, renames);
            let v = freshen_binders(value, surface, avoid, renames);
            let fresh = rename(renames, name);
            let b = freshen_binders(body, surface, avoid, renames);
            Term::Let {
                name: fresh,
                ty: Box::new(t),
                value: Box::new(v),
                body: Box::new(b),
            }
        }
        Term::App(f, a) => Term::App(
            Box::new(freshen_binders(f, surface, avoid, renames)),
            Box::new(freshen_binders(a, surface, avoid, renames)),
        ),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(freshen_binders(discriminant, surface, avoid, renames)),
            motive: Box::new(freshen_binders(motive, surface, avoid, renames)),
            cases: cases
                .iter()
                .map(|c| freshen_binders(c, surface, avoid, renames))
                .collect(),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| {
                    let fresh = surface.fresh_name(n, avoid);
                    renames.insert(n.clone(), fresh.clone());
                    (fresh, freshen_binders(b, surface, avoid, renames))
                })
                .collect(),
            index: *index,
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(freshen_binders(term, surface, avoid, renames)),
        },
        other => other.clone(),
    }
}

/// Replace each `Var(name)` named by `args` with its term. Capture-free because every binder of
/// the template has already been renamed away from the hole names.
fn substitute_vars(term: &Term, args: &HashMap<String, Term>) -> Term {
    match term {
        Term::Var(n) => args.get(n).cloned().unwrap_or_else(|| term.clone()),
        Term::App(f, a) => Term::App(
            Box::new(substitute_vars(f, args)),
            Box::new(substitute_vars(a, args)),
        ),
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(substitute_vars(param_type, args)),
            body: Box::new(substitute_vars(body, args)),
        },
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(substitute_vars(param_type, args)),
            body_type: Box::new(substitute_vars(body_type, args)),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: Box::new(substitute_vars(body, args)),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| (n.clone(), substitute_vars(b, args)))
                .collect(),
            index: *index,
        },
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(substitute_vars(discriminant, args)),
            motive: Box::new(substitute_vars(motive, args)),
            cases: cases.iter().map(|c| substitute_vars(c, args)).collect(),
        },
        Term::Let {
            name,
            ty,
            value,
            body,
        } => Term::Let {
            name: name.clone(),
            ty: Box::new(substitute_vars(ty, args)),
            value: Box::new(substitute_vars(value, args)),
            body: Box::new(substitute_vars(body, args)),
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(substitute_vars(term, args)),
        },
        other => other.clone(),
    }
}

// ================================================================================================
// HIERARCHICAL NAMES AND NAMESPACE RESOLUTION
//
// Lean's kernel `Environment` maps ABSOLUTE names to constants; `namespace`/`end`/`open` are
// elaborator SCOPE state and `resolveGlobalName` runs to completion before anything reaches the
// kernel. LOGOS follows that staging exactly: the scope state lives here on the surface, the
// resolution runs in the `Repl` (the elaboration driver), and `Context` stays a flat map of
// absolute keys. That is what keeps the Milner invariant — the kernel knows no surface — while
// still giving a short name a namespace to be relative TO.
// ================================================================================================

/// The one name that is never relative: `_root_.x` is the absolute `x`, whatever is open.
pub const ROOT_ESCAPE: &str = "_root_";

/// How an ANONYMOUS section is named in a report. Lean's `section` may be written with no name at
/// all, and a scope with no name still has to be reportable — as an unclosed scope at the end of a
/// document, and as the scope a mismatched `End` failed to close.
pub const ANONYMOUS_SECTION: &str = "an anonymous section";

/// How an `End` was WRITTEN, for a report that has to quote it back.
fn end_spelling(name: &Option<String>) -> String {
    match name {
        Some(name) => format!("End {name}"),
        None => "End".to_string(),
    }
}

/// A hierarchical name — Lean's `Name`, which is a LIST of components and not a string with dots.
///
/// The distinction is not cosmetic. To resolve a short name against an enclosing namespace at all
/// you must be able to walk the PREFIXES of the current namespace, and to store the answer you must
/// be able to re-compose one flat absolute key. A `String` with dots in it can do the second and
/// not the first.
///
/// A component is whatever `scan_name` accepts — ONE grammar for both doors, so every name the
/// declaration door takes is a name a term can spell. A `«guillemet name»` is ONE component even
/// when it contains a `.`, which is how a name with a dot in it stays writable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name {
    parts: Vec<String>,
}

impl Name {
    /// Read a name off the FRONT of `text`, returning it with the bytes consumed. Components are
    /// separated by `.` with no surrounding space — `Outer.Inner.value` is three components, and
    /// `Outer. Inner` is one component followed by a sentence terminator.
    pub fn scan(text: &str) -> Option<(Name, usize)> {
        let (first, mut len) = scan_name(text)?;
        let mut parts = vec![first];
        while text[len..].starts_with('.') {
            let after = len + '.'.len_utf8();
            match scan_name(&text[after..]) {
                Some((part, n)) => {
                    parts.push(part);
                    len = after + n;
                }
                // A trailing `.` is the sentence terminator, not an empty component.
                None => break,
            }
        }
        Some((Name { parts }, len))
    }

    /// Read a name that is the WHOLE of `text`.
    pub fn parse(text: &str) -> Option<Name> {
        match Name::scan(text) {
            Some((name, len)) if len == text.len() => Some(name),
            _ => None,
        }
    }

    /// A name from components already in hand. `None` for the empty list — there is no such name.
    pub fn from_parts(parts: Vec<String>) -> Option<Name> {
        (!parts.is_empty()).then_some(Name { parts })
    }

    /// The flat absolute key the kernel context is indexed by.
    pub fn key(&self) -> String {
        self.parts.join(".")
    }

    /// The components, outermost first.
    pub fn parts(&self) -> &[String] {
        &self.parts
    }

    /// The final component — the short name, with every namespace stripped.
    pub fn short(&self) -> &str {
        self.parts
            .last()
            .expect("a Name always has at least one component")
    }

    /// Everything but the final component: the namespace a DECLARATION name itself names. For
    /// `Foo.Bar.baz` that is `Foo.Bar`, and Lean elaborates such a declaration with `Foo.Bar`
    /// current — which is where a short name inside it finds its siblings.
    pub fn enclosing(&self) -> Vec<String> {
        self.parts[..self.parts.len() - 1].to_vec()
    }

    /// Does this name's flat key read back as this same name?
    ///
    /// The kernel `Context` is keyed by a flat `String`, so a name is only HOLDABLE if its key
    /// identifies it. `«a.b»` is ONE component and keys at `a.b`, which reads back as the
    /// TWO-component `a.b` — two different names, one key, and whichever is declared second
    /// answers to the other's spelling with nothing reported. That is this lane's own defect class
    /// (a short name quietly denoting a constant nobody wrote) arriving through the guillemet
    /// escape instead of through an `open`.
    ///
    /// A key that reads back as NOTHING AT ALL (`a name with spaces`, `weird?!name`) is holdable:
    /// no term-level spelling produces that string either, so there is nothing for it to collide
    /// with, and the quoted form stays the escape it was built to be.
    pub fn key_reads_back(&self) -> bool {
        match Name::parse(&self.key()) {
            Some(read_back) => read_back == *self,
            None => true,
        }
    }

    /// Can the TERM door spell this name, written exactly as `written`?
    ///
    /// ★ THE INVARIANT, CHECKED RATHER THAN RESTATED. "Every name the declaration door accepts, a
    /// term can spell" is the rule this whole grammar exists for, and `scan_name` gives both doors
    /// one COMPONENT grammar — but the term door joins components with its own dot-postfix rule and
    /// reserves its own keywords, so two doors that agree about components can still disagree about
    /// whole names. `Definition end : Nat := Zero.` and `Definition Outer.«a b» : Nat := Zero.` both
    /// scanned clean and both produced a global no `Check` could ever name: write-only names, by a
    /// route the component grammar cannot see.
    ///
    /// So rather than restate the term door's rule here — two grammars again, one step removed —
    /// the declaration door hands the spelling to the term door and requires this same name back.
    /// The check reads the grammar it is checking against, so it cannot drift from it.
    ///
    /// The surface is deliberately EMPTY: a name must be spellable with no notation in force, or it
    /// is a name whose readability depends on a macro that a later `End` can take away.
    pub fn is_spelled_by(&self, written: &str) -> bool {
        let Ok(term) = TermParser::parse_in(&Surface::new(), written) else {
            return false;
        };
        match &term {
            Term::Global(g) => *g == self.key(),
            _ => match flatten_projections(&term) {
                Some((Term::Global(head), fields)) => {
                    self.parts.len() == fields.len() + 1
                        && self.parts[0] == *head
                        && self.parts[1..].iter().zip(&fields).all(|(p, f)| p == f)
                }
                _ => false,
            },
        }
    }

    /// Is this name spelled with the `_root_` escape?
    pub fn is_root_escaped(&self) -> bool {
        self.parts.first().map(String::as_str) == Some(ROOT_ESCAPE)
    }

    /// The same name with a leading `_root_` removed. `None` if `_root_` was the whole of it.
    fn strip_root(&self) -> Option<Name> {
        Name::from_parts(self.parts[1..].to_vec())
    }

    /// The components of `self` that follow `prefix`, or `None` when `prefix` is not one.
    ///
    /// Lean's `Name.isPrefixOf` (`Lean/Data/Name.lean:46-49`) plus the `replacePrefix` that always
    /// follows it: `resolveOpenDecls` (`Lean/ResolveName.lean:146-151`) resolves an explicit
    /// `open Foo (x)` for `x.y` by testing that `x` is a PREFIX of `x.y` and swapping it for
    /// `Foo.x`. An equal name gives the EMPTY remainder, which is that rule's other half
    /// (`openedId == id`, line 144) falling out of the same test.
    pub fn after_prefix(&self, prefix: &Name) -> Option<Vec<String>> {
        (self.parts.len() >= prefix.parts.len()
            && self.parts[..prefix.parts.len()] == prefix.parts[..])
            .then(|| self.parts[prefix.parts.len()..].to_vec())
    }

    /// `self ++ tail`.
    fn followed_by(&self, tail: &[String]) -> Name {
        let mut parts = self.parts.clone();
        parts.extend_from_slice(tail);
        Name { parts }
    }

    /// `prefix ++ self`, as a key.
    fn under(&self, prefix: &[String]) -> String {
        if prefix.is_empty() {
            return self.key();
        }
        let mut parts: Vec<&str> = prefix.iter().map(String::as_str).collect();
        parts.extend(self.parts.iter().map(String::as_str));
        parts.join(".")
    }
}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.key())
    }
}

/// What a written name denotes, once the scope has had its say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Exactly one declared constant answers to the name: this absolute key.
    Found(String),
    /// Nothing declared answers to it. The caller keeps the spelling the user WROTE, so an unknown
    /// global is still reported as the unknown global the user asked for and not as some mangled
    /// candidate they never typed.
    Unresolved(String),
    /// Two or more DIFFERENT constants answer to one short name at the same priority. Lean errors
    /// here and so do we — a resolver that picks silently changes what a proof is ABOUT, and that
    /// is the failure mode with no symptom.
    Ambiguous {
        written: String,
        candidates: Vec<String>,
    },
}

/// Which of Lean's two `OpenDecl` constructors a clause resolves THROUGH, which is the only thing
/// that decides whether `protected` can refuse it.
///
/// `resolveOpenDecls` (`Lean/ResolveName.lean:136-141`) sends a wholesale `open Foo` through
/// `resolveQualifiedName`, and that is the one function in Lean that consults `isProtected`
/// (line 104). The `explicit` arm (lines 142-154) does not: a name that was WRITTEN in the clause
/// was asked for by name, so `protected` has nothing left to protect it from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenKind {
    /// `Open Foo.` / `Open Foo hiding (a).` — Lean's `OpenDecl.simple`.
    Simple,
    /// `Open Foo (a).` / `Open Foo renaming a -> b.` — Lean's `OpenDecl.explicit`.
    Explicit,
}

/// One `open` clause: a namespace, and WHICH of its names it brings in, under WHICH spelling.
///
/// Lean's four forms (`Lean/Elab/Open.lean:63-96`), and they are not sugar for one another:
/// `Open Alpha.` brings in all of them; `Open Alpha (a b).` brings in exactly those two;
/// `Open Alpha hiding (a).` brings in the rest; `Open Alpha renaming a -> b.` brings in `a` under
/// the local spelling `b` and brings in nothing else. Opening a namespace WHOLESALE is what makes
/// short names ambiguous in the first place, so the three narrowing forms are the tools for
/// AVOIDING an ambiguity rather than being told about one — and `renaming` is the only one of
/// them that can bring in a name whose short spelling already means something else here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenClause {
    /// The namespace. As written when the clause is parsed; resolved against the current namespace
    /// when it is applied.
    pub namespace: Name,
    /// `Open Alpha (a b).` — ONLY these names are brought in. `None` is the wholesale opening.
    pub only: Option<Vec<Name>>,
    /// `Open Alpha hiding (a b).` — every name EXCEPT these.
    pub hiding: Vec<Name>,
    /// `Open Alpha renaming a -> b.` — (the member, the local spelling). A clause with any
    /// renaming brings in EXACTLY these, which is why it is its own form and not a modifier on the
    /// others: Lean's `renaming` arm emits only `OpenDecl.explicit` entries and no simple open
    /// (`Lean/Elab/Open.lean:89-96`), so `open Baz renaming orig → ali` leaves `untouched`
    /// unknown.
    pub renaming: Vec<(Name, Name)>,
}

impl OpenClause {
    /// `Open Alpha.` — the wholesale opening.
    pub fn all(namespace: Name) -> Self {
        OpenClause {
            namespace,
            only: None,
            hiding: Vec::new(),
            renaming: Vec::new(),
        }
    }

    /// The absolute key this clause offers for `written`, or `None` if it offers none.
    ///
    /// ★ AN EXPLICIT OPENING MATCHES A PREFIX, A `hiding` MATCHES A WHOLE NAME. That asymmetry is
    /// Lean's, and it is measured rather than inferred: `resolveOpenDecls`
    /// (`Lean/ResolveName.lean:142-154`) resolves an `OpenDecl.explicit` by `openedId == id` OR
    /// `openedId.isPrefixOf id` with the prefix replaced, while the `simple` arm's exclusion list
    /// is an exact `exs.contains id` (line 137). So `open Foo (x)` reaches `Foo.x.y` and
    /// `open Bar hiding a` still reaches `Bar.a.deep`, both confirmed against lean 4.14.0.
    ///
    /// Comparing the WHOLE written name for the `only` form — which is what this did — refused
    /// `x.y` under `Open Foo (x).`: a false error, on a name Lean resolves.
    fn key_for(&self, written: &Name) -> Option<(String, OpenKind)> {
        if !self.renaming.is_empty() {
            return self.renaming.iter().find_map(|(member, local)| {
                let tail = written.after_prefix(local)?;
                let key = member.followed_by(&tail).under(&self.namespace.parts);
                Some((key, OpenKind::Explicit))
            });
        }
        if self.hiding.iter().any(|h| h == written) {
            return None;
        }
        match &self.only {
            Some(only) if !only.iter().any(|o| written.after_prefix(o).is_some()) => None,
            Some(_) => Some((written.under(&self.namespace.parts), OpenKind::Explicit)),
            None => Some((written.under(&self.namespace.parts), OpenKind::Simple)),
        }
    }

    /// Every name this clause NAMES, which must exist under `namespace` for the clause to mean
    /// anything. A clause naming something the namespace does not declare narrows the opening to
    /// nothing and reads exactly like one that worked — the silent no-op this surface refuses
    /// everywhere else. For a renaming it is the MEMBER that must exist, never the local spelling:
    /// the local spelling is the name being minted.
    fn named(&self) -> impl Iterator<Item = &Name> {
        self.only
            .iter()
            .flatten()
            .chain(self.hiding.iter())
            .chain(self.renaming.iter().map(|(member, _)| member))
    }
}

/// One `Namespace …` frame, with the `open` high-water mark it must restore on the way out — an
/// `open` written inside a namespace dies with that namespace — and the SECTION depth it was entered
/// at, which is the only thing that orders two stacks against each other.
#[derive(Debug, Clone)]
struct NsFrame {
    name: Name,
    opens_mark: usize,
    /// How many sections were open when this namespace was entered. `End <section>` is refused while
    /// this is at least the current section depth, because then no section was opened AFTER the
    /// namespace and the namespace is the innermost scope. Without it the two stacks have no
    /// relative order and `Section S. Namespace N. End S.` closes the section straight through the
    /// namespace, leaving `N` current for every later declaration — and the file still checks.
    section_depth: usize,
    /// How long the flattened current namespace was before this frame, so leaving is a truncation.
    current_len: usize,
}

/// Where a scope stood, so a scoped `Open … in …` (or a declaration's own implicit namespace) can be
/// wound back EXACTLY, without knowing what it pushed.
///
/// It saves the scope rather than an index into it because `_root_.baz` elaborates at the root: a
/// bracketed change may REMOVE frames as well as add them, and a high-water mark cannot restore
/// what was cleared. Two short `Vec` clones per command, against a whole elaboration.
#[derive(Debug, Clone)]
pub struct ScopeMark {
    frames: Vec<NsFrame>,
    opens: Vec<OpenClause>,
    current: Vec<String>,
    aliases: Vec<(String, String)>,
}

/// The namespace scope: what is CURRENT, and what is OPEN.
///
/// Empty is the state every existing session is in, and in that state [`Namespaces::resolve`] is
/// the identity on every name that resolves and on every name that does not — the resolver costs
/// one map lookup and changes nothing. That is deliberate: a subsystem that is free when unused is
/// one nobody has to opt out of.
#[derive(Debug, Clone, Default)]
pub struct Namespaces {
    /// One frame per `Namespace …` command, outermost first.
    frames: Vec<NsFrame>,
    /// The flattened current namespace, maintained rather than recomputed. Resolution asks for it
    /// once per NAME, and rebuilding a `Vec<String>` each time would put an allocation on the path
    /// every identifier in every command takes.
    current: Vec<String>,
    /// Every opening in force, earliest first, with its namespace already resolved. They sit at ONE
    /// priority, which is why two of them answering to one short name is an ambiguity and not a
    /// shadow.
    opens: Vec<OpenClause>,
    /// Every `export`ed name: (the absolute key it answers to here, the absolute key it denotes).
    /// A short list consulted on every candidate, and EMPTY unless `Export` was written — which is
    /// what keeps it off the path of every session that never exports anything.
    aliases: Vec<(String, String)>,
    /// Every namespace that EXISTS: entered by a `Namespace` command, or named by the prefix of a
    /// name that was declared. Lean errors on `open` of an unknown namespace, and an `open` that
    /// brings in nothing at all is the quietest possible way to make a short name mean something
    /// other than what was written — so the set is kept, and `open` is checked against it.
    known: std::collections::BTreeSet<String>,
    /// Every key declared `Protected`.
    ///
    /// ★ `protected` IS A RESOLUTION FACT, NOT A PERMISSION. Corpus case T02-012 says so in its own
    /// guard — "it is inert *logically*, but it changes which short name resolves at a use site".
    /// `resolveQualifiedName` (`Lean/ResolveName.lean:104`) drops a protected declaration exactly
    /// when the written name is ATOMIC, so `protected def Foo.x` leaves `x` unresolvable both under
    /// `open Foo` AND inside `namespace Foo`, while `Foo.x` still works — all three measured
    /// against lean 4.14.0. EMPTY unless `Protected` was written, which keeps it off every
    /// session's path.
    protected_keys: std::collections::BTreeSet<String>,
}

impl Namespaces {
    /// A scope with nothing open and nothing current.
    pub fn new() -> Self {
        Self::default()
    }

    /// Is the scope empty? In that state [`resolve`](Self::resolve) is provably the identity — every
    /// name that resolves resolves to itself and every name that does not keeps its spelling — which
    /// is what makes this subsystem free for every session that never writes `Namespace`, `Open` or
    /// `Export`. All three must be absent for the claim to hold, which is why all three are checked.
    ///
    /// `Protected` is deliberately NOT a fourth: it can only refuse a candidate reached through
    /// `lookup_qualified`, which is the enclosing-namespace walk and a
    /// wholesale `open` — both of which require a non-empty scope to exist at all. The root reading
    /// goes through `lookup` and consults it not at all, exactly as Lean's line 177
    /// does not (`ResolveName.lean:177`), so the identity claim survives a `Protected` written at
    /// the root.
    pub fn is_trivial(&self) -> bool {
        self.frames.is_empty() && self.opens.is_empty() && self.aliases.is_empty()
    }

    /// The components of the current namespace, outermost first.
    pub fn current(&self) -> &[String] {
        &self.current
    }

    /// The openings in force, earliest first, each with its namespace already resolved.
    pub fn opens(&self) -> &[OpenClause] {
        &self.opens
    }

    /// How many `Namespace \u{2026}` frames are entered.
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    /// The entered namespaces, outermost first \u{2014} what a document must close before it ends.
    pub fn frame_names(&self) -> Vec<String> {
        self.frames.iter().map(|f| f.name.key()).collect()
    }

    /// Where the scope stands now.
    pub fn mark(&self) -> ScopeMark {
        ScopeMark {
            frames: self.frames.clone(),
            opens: self.opens.clone(),
            current: self.current.clone(),
            aliases: self.aliases.clone(),
        }
    }

    /// Wind the scope back to `mark`. Used for `Open … in <command>`, which scopes its opens to
    /// exactly one command, and for a declaration whose own name names a namespace.
    pub fn rewind(&mut self, mark: ScopeMark) {
        self.frames = mark.frames;
        self.opens = mark.opens;
        self.current = mark.current;
        self.aliases = mark.aliases;
    }

    /// Enter the scope a DECLARATION elaborates in, and give back the absolute key it declares.
    ///
    /// `def Foo.Bar.baz` elaborates with `Foo.Bar` current — a declaration name that NAMES a
    /// namespace opens it for its own body, which is where a short sibling name is found. That is
    /// Lean's rule and it is the whole of T02-011's third assertion. `_root_.baz` declares the
    /// top-level name and elaborates at the root, which is what the escape is for.
    pub fn enter_declaration(
        &mut self,
        declared: &Name,
        section_depth: usize,
    ) -> (String, ScopeMark) {
        let mark = self.mark();
        let absolute = match declared.strip_root() {
            Some(stripped) if declared.is_root_escaped() => {
                // The escape elaborates at the ROOT, so the current namespace goes with the frames.
                self.frames.clear();
                self.current.clear();
                stripped
            }
            _ => declared.clone(),
        };
        let key = absolute.under(&self.current);
        if let Some(inner) = Name::from_parts(absolute.enclosing()) {
            self.push_frame(&inner, section_depth);
        }
        (key, mark)
    }

    /// `Namespace Foo.Bar.` — push one frame. `Foo.Bar` is ONE frame of two components, so
    /// `End Foo.Bar.` closes it and `End Bar.` does not.
    pub fn enter(&mut self, name: &Name, section_depth: usize) {
        let mut prefix = self.current.clone();
        for part in &name.parts {
            prefix.push(part.clone());
            self.known.insert(prefix.join("."));
        }
        self.push_frame(name, section_depth);
    }

    /// Enter a namespace WITHOUT declaring that it exists. Used for the scope a declaration's own
    /// name implies: the namespace comes into being when the declaration lands, not when it is
    /// attempted, so a refused `Definition Typo.x := …` leaves nothing to `Open`.
    fn push_frame(&mut self, name: &Name, section_depth: usize) {
        self.frames.push(NsFrame {
            name: name.clone(),
            opens_mark: self.opens.len(),
            section_depth,
            current_len: self.current.len(),
        });
        self.current.extend(name.parts.iter().cloned());
    }

    /// Note that the absolute key `key` has been declared, so every namespace its prefixes name now
    /// EXISTS and can be opened. Called on SUCCESS only — a refused declaration creates nothing.
    pub fn record_declaration(&mut self, key: &str) {
        let Some(name) = Name::parse(key) else { return };
        for depth in 1..name.parts.len() {
            self.known.insert(name.parts[..depth].join("."));
        }
    }

    /// Mark the absolute key `key` as `protected`: it stops answering to its SHORT name, here and
    /// under any `open`, and keeps answering to its full one. Called on SUCCESS only, like
    /// [`record_declaration`](Self::record_declaration) — a refused declaration protects nothing.
    pub fn record_protected(&mut self, key: &str) {
        self.protected_keys.insert(key.to_string());
    }

    /// Is `key` protected against being reached by the ATOMIC name `written`?
    ///
    /// Both halves are Lean's, in one test: `!id.isAtomic || !isProtected env resolvedId`
    /// (`Lean/ResolveName.lean:104`). A qualified reference is never refused by `protected`, which
    /// is why `Foo.x` still resolves and `x` does not.
    fn shields(&self, key: &str, written: &Name) -> bool {
        written.parts.len() == 1 && self.protected_keys.contains(key)
    }

    /// Which namespace `written` names, read relative to the current one: the longest enclosing
    /// prefix that exists, else the bare name if that exists. `None` if no such namespace has ever
    /// been declared — which is what makes `Open Typo.` an error rather than a silent no-op.
    pub fn resolve_namespace(&self, written: &Name) -> Option<Name> {
        if let Some(stripped) = written.strip_root().filter(|_| written.is_root_escaped()) {
            return self.known.contains(&stripped.key()).then_some(stripped);
        }
        let current = &self.current;
        for depth in (0..=current.len()).rev() {
            let mut parts = current[..depth].to_vec();
            parts.extend(written.parts.iter().cloned());
            let candidate = Name { parts };
            if self.known.contains(&candidate.key()) {
                return Some(candidate);
            }
        }
        None
    }

    /// `End Foo.Bar.` — pop the innermost frame if it is the one named, restoring the `open` state
    /// it was entered with. `false` if the innermost open scope is not that namespace, which is how
    /// the same `End` keyword serves both namespaces and sections without either guessing.
    pub fn leave(&mut self, name: &Name) -> bool {
        match self.frames.last() {
            Some(frame) if frame.name == *name => {
                let (opens_mark, current_len) = (frame.opens_mark, frame.current_len);
                self.frames.pop();
                self.opens.truncate(opens_mark);
                self.current.truncate(current_len);
                // ★ AN `export` OUTLIVES ITS NAMESPACE, AN `open` DOES NOT. This truncated the
                // alias list here too, on the reasoning that both are spellings written inside a
                // scope — but they are not the same kind of thing and Lean does not treat them as
                // one. `addAlias` (`Lean/ResolveName.lean:70-71`) is a persistent ENVIRONMENT
                // extension, while an `OpenDecl` is elaborator `Scope` state; measured on lean
                // 4.14.0, `namespace B export A (x) end B` leaves `#eval B.x` printing 5. Dying
                // with the namespace made `export` useless for the one thing it exists for —
                // publishing a name from inside the namespace that is to answer to it.
                true
            }
            _ => false,
        }
    }

    /// The namespace that is the INNERMOST open scope, if a namespace is — i.e. if no section has
    /// been opened since it was entered. `End` consults this so one keyword can serve both stacks
    /// without either of them guessing.
    pub fn innermost_scope(&self, section_depth: usize) -> Option<&Name> {
        self.frames
            .last()
            .filter(|f| section_depth <= f.section_depth)
            .map(|f| &f.name)
    }

    /// `Open Foo.` — bring a namespace's short names into scope, reading `Foo` relative to the
    /// current namespace. A namespace nothing has ever declared into is REFUSED, as Lean refuses it:
    /// an `open` that brings in nothing looks exactly like an `open` that worked, and every short
    /// name under it then quietly means the root's constant instead.
    pub fn open(&mut self, name: Name) -> Result<(), ParseError> {
        self.open_clause(OpenClause::all(name), &|_| true)
    }

    /// [`open`](Self::open) for a clause that NARROWS the opening — `Open Alpha (a b).` and
    /// `Open Alpha hiding (a).`
    ///
    /// `known` is the environment predicate, and it is needed here for the reason `export` needs
    /// one: a clause that names something the namespace does not declare narrows the opening to
    /// nothing and is indistinguishable from one that worked. The wholesale form names nothing, so
    /// [`open`](Self::open) can still be called without an environment.
    pub fn open_clause(
        &mut self,
        clause: OpenClause,
        known: &dyn Fn(&str) -> bool,
    ) -> Result<(), ParseError> {
        let resolved = self
            .resolve_namespace(&clause.namespace)
            .ok_or_else(|| ParseError::UnknownNamespace(clause.namespace.key()))?;
        for name in clause.named() {
            let target = name.under(&resolved.parts);
            if self.lookup(&target, known).is_none() {
                return Err(ParseError::Missing(format!(
                    "`{target}` — an `Open {resolved}` clause names `{name}`, which {resolved} \
                     does not declare, so the clause would bring in nothing while reading exactly \
                     like one that worked"
                )));
            }
        }
        self.opens.push(OpenClause {
            namespace: resolved,
            ..clause
        });
        Ok(())
    }

    /// The absolute key a DECLARATION written here gets: the current namespace, then the name as
    /// written. `_root_.x` declares the absolute `x`.
    pub fn declaration_key(&self, declared: &Name) -> String {
        match declared.strip_root() {
            Some(absolute) if declared.is_root_escaped() => absolute.key(),
            _ => declared.under(&self.current),
        }
    }

    /// What the absolute key `key` denotes: itself if something is declared there, the `export`ed
    /// target if it is an alias, and nothing otherwise. Aliases are consulted FIRST so an `export`
    /// into a namespace that also has a real declaration of that name cannot be shadowed by it —
    /// `export` is refused in that case, below, so the two can never both exist.
    fn lookup(&self, key: &str, known: &dyn Fn(&str) -> bool) -> Option<String> {
        if let Some((_, target)) = self.aliases.iter().rev().find(|(alias, _)| alias == key) {
            return Some(target.clone());
        }
        known(key).then(|| key.to_string())
    }

    /// [`lookup`](Self::lookup) through Lean's `resolveQualifiedName`, which is the ONE path that
    /// consults `protected` (`Lean/ResolveName.lean:100-110`). The enclosing-namespace walk and a
    /// wholesale `open` both go through it; the exact root reading, the `_root_` escape and an
    /// EXPLICIT `open` do not, which is why `Foo.x`, `_root_.Foo.x` and `open Foo (x)` all still
    /// reach a protected declaration.
    fn lookup_qualified(
        &self,
        key: &str,
        written: &Name,
        known: &dyn Fn(&str) -> bool,
    ) -> Option<String> {
        if let Some((_, target)) = self.aliases.iter().rev().find(|(alias, _)| alias == key) {
            return Some(target.clone());
        }
        (known(key) && !self.shields(key, written)).then(|| key.to_string())
    }

    /// `Export Alpha (token other).` — make each `Alpha.<name>` answer to `<name>` in the CURRENT
    /// namespace, PERMANENTLY — an alias is an environment fact, not scope state, so unlike an
    /// `open` it survives the `End` of the namespace it was written in. That is Lean's own
    /// staging (`addAlias` is a persistent env extension, `OpenDecl` is elaborator `Scope` state)
    /// and it is what `export` is for: publishing a name from inside the namespace that is to
    /// answer to it.
    ///
    /// Every failure mode is LOUD, because a silent one is an alias nobody can see: an unknown source
    /// namespace, a name that namespace does not declare, and a name the current namespace already
    /// declares (which would leave two readings of one key and no way to say which was meant).
    pub fn export(
        &mut self,
        namespace: &Name,
        names: &[Name],
        known: &dyn Fn(&str) -> bool,
    ) -> Result<(), ParseError> {
        let source = self
            .resolve_namespace(namespace)
            .ok_or_else(|| ParseError::UnknownNamespace(namespace.key()))?;
        let mut pending: Vec<(String, String)> = Vec::with_capacity(names.len());
        for name in names {
            let target = name.under(&source.parts);
            if self.lookup(&target, known).is_none() {
                return Err(ParseError::Missing(format!(
                    "`{target}` \u{2014} `Export {source} ({name})` names something {source} does not \
                     declare, so the alias would point nowhere"
                )));
            }
            let alias = name.under(&self.current);
            if alias == target {
                continue; // exporting a namespace into itself is already true
            }
            // A real declaration at the alias key, or an earlier export pointing somewhere ELSE,
            // both leave one key with two readings. Lean reports that; so do we, rather than let
            // the later one win in silence.
            let clash = known(&alias)
                .then(|| alias.clone())
                .or_else(|| {
                    self.aliases
                        .iter()
                        .find(|(a, t)| *a == alias && *t != target)
                        .map(|(_, t)| t.clone())
                })
                .or_else(|| {
                    pending
                        .iter()
                        .find(|(a, t)| *a == alias && *t != target)
                        .map(|(_, t)| t.clone())
                });
            if let Some(other) = clash {
                return Err(ParseError::AmbiguousName {
                    written: alias,
                    candidates: vec![other, target],
                });
            }
            pending.push((alias, target));
        }
        self.aliases.extend(pending);
        Ok(())
    }

    /// Resolve a written name to the absolute key it denotes, in Lean's order.
    ///
    /// 1. `_root_.x` is absolute and consults nothing.
    /// 2. The enclosing namespace, LONGEST prefix first. An inner namespace SHADOWS an outer one —
    ///    Lean's `resolveUsingNamespace` returns at the first prefix that resolves, so this is a
    ///    shadow and never an ambiguity.
    /// 3. Every `open` namespace AND the root, at ONE priority. Lean combines them
    ///    (`resolveOpenDecls` seeded from the root lookup), so a short name that answers to two
    ///    different constants is AMBIGUOUS. This is the case the audit called the dangerous one:
    ///    with 50 prelude names already in the root, a short name written under an `open` used to
    ///    resolve to the PRELUDE's constant and report nothing. It is now reported.
    ///
    /// `known` is the environment predicate — "is this absolute key declared?". Passing it in
    /// rather than a `Context` is what keeps the kernel out of the surface.
    pub fn resolve(&self, written: &Name, known: &dyn Fn(&str) -> bool) -> Resolution {
        if written.is_root_escaped() {
            let Some(absolute) = written.strip_root() else {
                return Resolution::Unresolved(written.key());
            };
            let key = absolute.key();
            return match self.lookup(&key, known) {
                Some(target) => Resolution::Found(target),
                None => Resolution::Unresolved(key),
            };
        }

        let current = &self.current;
        for depth in (1..=current.len()).rev() {
            let key = written.under(&current[..depth]);
            if let Some(target) = self.lookup_qualified(&key, written, known) {
                return Resolution::Found(target);
            }
        }

        // ★ LEAN'S `resolveExact`, AND THE REASON IT IS ITS OWN RUNG. `ResolveName.lean:174`
        // calls it between the namespace walk and the opens, and a `some` there RETURNS — the
        // `open` decls at line 180 are never consulted. `resolveExact` itself
        // (`ResolveName.lean:121-131`) answers `none` for an ATOMIC name, so only a short name
        // ever reaches the branch where the root and the opens sit at one priority.
        //
        // Without this rung a dotted name the root declares was reported AMBIGUOUS the moment any
        // `open` offered a second reading: `Alpha.Inner.x` + `Inner.x` under `Open Alpha.` refused
        // `Inner.x`, where lean 4.14.0 evaluates it to the root's constant without a word. A false
        // error is not the safe side of this trade — it refuses programs Lean accepts, and the
        // rule it was enforcing ("two readings must be reported") was never Lean's rule for a name
        // that is not short.
        //
        // Aliases are deliberately NOT consulted here: Lean's `resolveExact` tests the environment
        // (`containsDeclOrReserved`), and `getAliases` runs in the LATER branch — so an `export`ed
        // dotted name still joins the candidate pool and can still be reported ambiguous.
        if written.parts.len() > 1 {
            let exact = written.key();
            if known(&exact) {
                return Resolution::Found(exact);
            }
        }

        // The opens are tried most-recent first, so the one candidate case picks the reading a
        // reader would expect; two candidates is an error either way, so the order only decides
        // how the message reads.
        let mut candidates: Vec<String> = Vec::new();
        for open in self.opens.iter().rev() {
            let Some((key, kind)) = open.key_for(written) else {
                continue;
            };
            let found = match kind {
                OpenKind::Simple => self.lookup_qualified(&key, written, known),
                OpenKind::Explicit => self.lookup(&key, known),
            };
            if let Some(target) = found {
                if !candidates.contains(&target) {
                    candidates.push(target);
                }
            }
        }
        let root = written.key();
        if let Some(target) = self.lookup(&root, known) {
            if !candidates.contains(&target) {
                candidates.push(target);
            }
        }
        match candidates.len() {
            0 => Resolution::Unresolved(root),
            1 => Resolution::Found(candidates.pop().expect("length checked")),
            _ => Resolution::Ambiguous {
                written: root,
                candidates,
            },
        }
    }
}

/// Is `name` one of the parser's reserved marker heads rather than a name a user wrote? A marker is
/// not spellable by [`scan_name`], so this is belt-and-braces — but the belt is cheap and the
/// consequence of resolving `⟨proj⟩` is a term nobody can read.
fn is_marker(name: &str) -> bool {
    name == crate::elaborate::DOT_MARKER || name == crate::elaborate::ANON_CTOR_MARKER
}

/// `a.b.c` parses as `⟨proj⟩ (⟨proj⟩ a b) c`. Flatten that to the receiver and the field chain in
/// source order, or `None` if the term is not a projection chain at all.
fn flatten_projections(term: &Term) -> Option<(&Term, Vec<&str>)> {
    let mut fields: Vec<&str> = Vec::new();
    let mut cursor = term;
    while let Term::App(f, arg) = cursor {
        match (f.as_ref(), arg.as_ref()) {
            (Term::App(head, receiver), Term::Global(field)) if matches!(head.as_ref(), Term::Global(m) if m == crate::elaborate::DOT_MARKER) =>
            {
                fields.push(field.as_str());
                cursor = receiver;
            }
            _ => break,
        }
    }
    if fields.is_empty() {
        return None;
    }
    fields.reverse();
    Some((cursor, fields))
}

/// Rebuild `⟨proj⟩ base field` for one projection.
fn project(base: Term, field: &str) -> Term {
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global(crate::elaborate::DOT_MARKER.to_string())),
            Box::new(base),
        )),
        Box::new(Term::Global(field.to_string())),
    )
}

/// The environment a written name is resolved against — the two questions resolution asks of it,
/// as closures, which is what keeps the kernel out of the surface.
pub struct Env<'e> {
    /// Is this absolute key declared?
    pub known: &'e dyn Fn(&str) -> bool,
    /// The fresh identity this absolute written name denotes, when a shadow redirects it
    /// (`Context::shadow_of`).
    pub shadow: &'e dyn Fn(&str) -> Option<String>,
}

impl Env<'_> {
    /// What `written` denotes: the absolute key the namespace scope resolves it to, then — unless
    /// it was written with the `_root_` escape — the identity a shadow redirects that key to.
    ///
    /// ★ THE ONE RESOLUTION POINT. Every surface that reads a name reads it here: the term and
    /// command doors through [`resolve_names_in`], the literate door as it reads each name, the
    /// REPL's `Print Axioms`, `Structure` and `Quotient` references. The shadow is applied AFTER
    /// the scope has chosen an absolute key, and to that key alone, so a shadow never takes part in
    /// the scope's own decisions (which namespace, which `open`, whether two readings are
    /// ambiguous), and the escape keeps meaning what it means in Lean: the environment's own
    /// declaration, whatever shadows its name.
    pub fn resolve(&self, ns: &Namespaces, written: &Name) -> Resolution {
        let resolution = ns.resolve(written, self.known);
        if written.is_root_escaped() {
            return resolution;
        }
        match resolution {
            Resolution::Found(key) => Resolution::Found((self.shadow)(&key).unwrap_or(key)),
            Resolution::Unresolved(key) => match (self.shadow)(&key) {
                Some(identity) => Resolution::Found(identity),
                None => Resolution::Unresolved(key),
            },
            ambiguous @ Resolution::Ambiguous { .. } => ambiguous,
        }
    }
}

/// Resolve every global name written in `term` against the scope, BEFORE any elaboration sees it —
/// Lean's staging, and the whole content of "name resolution completes before translation begins".
///
/// `env` is the environment ([`Env`]); `locals` is the stack of names that are BOUND here and so
/// are not global references at all (an inductive's type parameters arrive as `Global`s and become
/// `Var`s only later, so they have to be shielded by hand).
///
/// Returns `Cow::Borrowed` when nothing in the term needed resolving, so an empty scope costs one
/// read-only walk and not a rebuild.
pub fn resolve_names_in<'t>(
    ns: &Namespaces,
    env: &Env<'_>,
    locals: &mut Vec<String>,
    term: &'t Term,
) -> Result<std::borrow::Cow<'t, Term>, ParseError> {
    Walk {
        ns,
        env,
        direction: Direction::Resolve,
    }
    .run(locals, term)
}

/// [`resolve_names_in`] against an environment that is only its declared keys — `known` — and
/// shadows nothing.
pub fn resolve_names<'t>(
    ns: &Namespaces,
    known: &dyn Fn(&str) -> bool,
    locals: &mut Vec<String>,
    term: &'t Term,
) -> Result<std::borrow::Cow<'t, Term>, ParseError> {
    resolve_names_in(
        ns,
        &Env {
            known,
            shadow: &|_| None,
        },
        locals,
        term,
    )
}

/// The inverse of [`resolve_names_in`], for a term already in KEY space: every global is re-spelled
/// so that resolving the result in this same scope reaches exactly the key it holds now.
///
/// A key its own spelling still reaches is kept. A key its spelling no longer reaches — a
/// declaration a shadow hides, or one an enclosing namespace or an `open` captures — is written
/// with the `_root_` escape, which reaches it and only it. An identity no surface can spell is
/// kept as it is: resolution leaves it alone too.
///
/// A door that must know what a name MEANS while it reads (the literate surface compiles equation
/// sets and reflects statements at parse time) resolves as it reads and hands its output to the
/// REPL, which resolves again. Re-spelling the output is what makes that second pass the identity
/// rather than a second, different reading of `_root_.add`.
pub fn respell_names<'t>(
    ns: &Namespaces,
    env: &Env<'_>,
    locals: &mut Vec<String>,
    term: &'t Term,
) -> Result<std::borrow::Cow<'t, Term>, ParseError> {
    Walk {
        ns,
        env,
        direction: Direction::Respell,
    }
    .run(locals, term)
}

/// Which way a [`Walk`] maps the references it meets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// A written name to the key it denotes.
    Resolve,
    /// A key to a spelling that denotes it.
    Respell,
}

/// One walk over a term's references — the structure is shared by both directions, so resolving
/// and re-spelling cannot come to disagree about what is bound, what is a field label and what is
/// a reference.
struct Walk<'a, 'e> {
    ns: &'a Namespaces,
    env: &'a Env<'e>,
    direction: Direction,
}

/// The names bound around the node being resolved: the caller's stack, plus a count per name, so
/// "is this name bound here?" is one lookup rather than a scan of every enclosing binder. A term
/// read back from a printed declaration can sit under thousands of binders, and a scan per name
/// made resolving it quadratic.
struct Locals<'a> {
    stack: &'a mut Vec<String>,
    counts: std::collections::HashMap<String, usize>,
}

impl<'a> Locals<'a> {
    fn new(stack: &'a mut Vec<String>) -> Self {
        let mut counts = std::collections::HashMap::new();
        for name in stack.iter() {
            *counts.entry(name.clone()).or_insert(0) += 1;
        }
        Locals { stack, counts }
    }

    fn contains(&self, name: &str) -> bool {
        self.counts.get(name).is_some_and(|c| *c > 0)
    }

    fn push(&mut self, name: String) {
        *self.counts.entry(name.clone()).or_insert(0) += 1;
        self.stack.push(name);
    }

    fn pop(&mut self) {
        if let Some(name) = self.stack.pop() {
            if let Some(c) = self.counts.get_mut(&name) {
                *c -= 1;
            }
        }
    }
}

impl Walk<'_, '_> {
    fn run<'t>(
        &self,
        locals: &mut Vec<String>,
        term: &'t Term,
    ) -> Result<std::borrow::Cow<'t, Term>, ParseError> {
        let mut bound = Locals::new(locals);
        Ok(match self.step(&mut bound, term)? {
            Some(rebuilt) => std::borrow::Cow::Owned(rebuilt),
            None => std::borrow::Cow::Borrowed(term),
        })
    }

    /// The key or spelling a REFERENCE `name` maps to, or `None` when it maps to itself.
    fn reference(&self, name: &str) -> Result<Option<String>, ParseError> {
        let Some(written) = Name::parse(name) else {
            return Ok(None);
        };
        match (self.direction, self.env.resolve(self.ns, &written)) {
            (Direction::Resolve, Resolution::Found(key)) => Ok((key != name).then_some(key)),
            (Direction::Resolve, Resolution::Unresolved(_)) => Ok(None),
            (
                Direction::Resolve,
                Resolution::Ambiguous {
                    written,
                    candidates,
                },
            ) => Err(ParseError::AmbiguousName {
                written,
                candidates,
            }),
            (Direction::Respell, Resolution::Found(key)) if key == name => Ok(None),
            (Direction::Respell, Resolution::Unresolved(_)) => Ok(None),
            (Direction::Respell, _) => Ok(Some(format!("{ROOT_ESCAPE}.{name}"))),
        }
    }

    /// The walk itself. `None` means "unchanged", which is what keeps the untouched case
    /// allocation-free all the way down the tree.
    fn step(&self, locals: &mut Locals<'_>, term: &Term) -> Result<Option<Term>, ParseError> {
        if let Some((pattern, body)) = super::raw_match::as_case(term) {
            let pattern = super::raw_match::resolve_pattern(pattern, &mut |name| {
                self.run(&mut Vec::new(), name).map(|t| t.into_owned())
            })?;
            let body = self.step(locals, body)?.unwrap_or_else(|| body.clone());
            return Ok(Some(super::raw_match::case(pattern, body)));
        }
        if let Some((receiver, fields)) = flatten_projections(term) {
            return match self.direction {
                Direction::Resolve => self.resolve_chain(locals, receiver, &fields),
                Direction::Respell => self.respell_chain(locals, receiver, &fields),
            };
        }

        match term {
            Term::Global(n) => {
                if is_marker(n) || locals.contains(n) {
                    return Ok(None);
                }
                Ok(self.reference(n)?.map(Term::Global))
            }
            // A constant written at explicit levels (`name.{u}`) is a REFERENCE exactly as a bare
            // global is, so its name reads in the same scope — `Nat.rec.{u}` inside `Namespace Nat`
            // may be written `rec.{u}`. The level arguments are not names and are kept as written.
            Term::Const { name, levels } => Ok(self.reference(name)?.map(|key| Term::Const {
                name: key,
                levels: levels.clone(),
            })),
            Term::Var(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole => Ok(None),
            Term::App(f, a) => {
                let rf = self.step(locals, f)?;
                let ra = self.step(locals, a)?;
                Ok(rebuild2(rf, ra, f, a, |f, a| {
                    Term::App(Box::new(f), Box::new(a))
                }))
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                let rt = self.step(locals, param_type)?;
                locals.push(param.clone());
                let rb = self.step(locals, body);
                locals.pop();
                let rb = rb?;
                Ok(rebuild2(rt, rb, param_type, body, |t, b| Term::Lambda {
                    param: param.clone(),
                    param_type: Box::new(t),
                    body: Box::new(b),
                }))
            }
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                let rt = self.step(locals, param_type)?;
                locals.push(param.clone());
                let rb = self.step(locals, body_type);
                locals.pop();
                let rb = rb?;
                Ok(rebuild2(rt, rb, param_type, body_type, |t, b| Term::Pi {
                    param: param.clone(),
                    param_type: Box::new(t),
                    body_type: Box::new(b),
                }))
            }
            Term::Fix { name, body } => {
                locals.push(name.clone());
                let rb = self.step(locals, body);
                locals.pop();
                Ok(rb?.map(|b| Term::Fix {
                    name: name.clone(),
                    body: Box::new(b),
                }))
            }
            Term::MutualFix { defs, index } => {
                for (n, _) in defs {
                    locals.push(n.clone());
                }
                let mut changed = false;
                let mut out = Vec::with_capacity(defs.len());
                let mut failure = None;
                for (n, b) in defs {
                    match self.step(locals, b) {
                        Ok(Some(nb)) => {
                            changed = true;
                            out.push((n.clone(), nb));
                        }
                        Ok(None) => out.push((n.clone(), b.clone())),
                        Err(e) => {
                            failure = Some(e);
                            break;
                        }
                    }
                }
                for _ in defs {
                    locals.pop();
                }
                if let Some(e) = failure {
                    return Err(e);
                }
                Ok(changed.then_some(Term::MutualFix {
                    defs: out,
                    index: *index,
                }))
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                let rd = self.step(locals, discriminant)?;
                let rm = self.step(locals, motive)?;
                let mut changed = rd.is_some() || rm.is_some();
                let mut out = Vec::with_capacity(cases.len());
                for c in cases {
                    match self.step(locals, c)? {
                        Some(nc) => {
                            changed = true;
                            out.push(nc);
                        }
                        None => out.push(c.clone()),
                    }
                }
                Ok(changed.then(|| Term::Match {
                    discriminant: Box::new(rd.unwrap_or_else(|| (**discriminant).clone())),
                    motive: Box::new(rm.unwrap_or_else(|| (**motive).clone())),
                    cases: out,
                }))
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                let rt = self.step(locals, ty)?;
                let rv = self.step(locals, value)?;
                locals.push(name.clone());
                let rb = self.step(locals, body);
                locals.pop();
                let rb = rb?;
                if rt.is_none() && rv.is_none() && rb.is_none() {
                    return Ok(None);
                }
                Ok(Some(Term::Let {
                    name: name.clone(),
                    ty: Box::new(rt.unwrap_or_else(|| (**ty).clone())),
                    value: Box::new(rv.unwrap_or_else(|| (**value).clone())),
                    body: Box::new(rb.unwrap_or_else(|| (**body).clone())),
                }))
            }
            Term::Annotated { annotation, term } => {
                Ok(self.step(locals, term)?.map(|t| Term::Annotated {
                    annotation: annotation.clone(),
                    term: Box::new(t),
                }))
            }
        }
    }

    /// A projection chain may be a hierarchical NAME rather than a field access, and Lean's rule is
    /// the LONGEST prefix that resolves wins: in `a.b.c`, if the constant `a.b` exists then `c` is
    /// its field. Trying the name first and the projection second is Lean's own order for
    /// generalized field notation.
    fn resolve_chain(
        &self,
        locals: &mut Locals<'_>,
        receiver: &Term,
        fields: &[&str],
    ) -> Result<Option<Term>, ParseError> {
        if let Term::Global(head) = receiver {
            if !is_marker(head) && !locals.contains(head) {
                // The head is ONE component as the term door read it, so a quoted head with a
                // space or a `.` in it is not re-readable as a name — `Name::parse` returns None
                // for `a b`, and without the fallback `«a b».value` declares and can never be
                // referred to. Taking the head verbatim is also the honest reading: it came off
                // `scan_name`, which is what a component IS.
                if let Some(base) =
                    Name::parse(head).or_else(|| Name::from_parts(vec![head.clone()]))
                {
                    for take in (0..=fields.len()).rev() {
                        let mut parts = base.parts.clone();
                        parts.extend(fields[..take].iter().map(|f| f.to_string()));
                        let candidate = Name { parts };
                        match self.env.resolve(self.ns, &candidate) {
                            Resolution::Found(key) => {
                                if take == 0 && key == *head {
                                    return Ok(None); // the whole chain is field access, untouched
                                }
                                let mut out = Term::Global(key);
                                for field in &fields[take..] {
                                    out = project(out, field);
                                }
                                return Ok(Some(out));
                            }
                            Resolution::Ambiguous {
                                written,
                                candidates,
                            } => {
                                return Err(ParseError::AmbiguousName {
                                    written,
                                    candidates,
                                });
                            }
                            Resolution::Unresolved(_) => {}
                        }
                    }
                }
            }
        }
        // No prefix of the chain is a name: it is field access on a term. Resolve the RECEIVER and
        // rebuild with the fields VERBATIM.
        //
        // ★ A FIELD IS A LABEL, NOT A REFERENCE. Letting the generic walk descend into the chain
        // would hand each field name to the resolver, and inside `Namespace Foo` with a `Foo.fst`
        // declared, `p.fst` would silently become `p.Foo.fst` — a projection by a field name the
        // structure does not have, produced from source that never said it.
        let resolved = self.step(locals, receiver)?;
        Ok(resolved.map(|base| fields.iter().fold(base, |acc, field| project(acc, field))))
    }

    /// Re-spell a projection chain whose receiver is already a key: the receiver is re-spelled
    /// and the fields are labels, kept verbatim. A receiver `k` whose chain `k.f…` would read back
    /// as a declared NAME has no spelling that survives resolution as a projection, and is refused
    /// rather than handed on to be read as something else.
    fn respell_chain(
        &self,
        locals: &mut Locals<'_>,
        receiver: &Term,
        fields: &[&str],
    ) -> Result<Option<Term>, ParseError> {
        let respelled = self.step(locals, receiver)?;
        let base = respelled.as_ref().unwrap_or(receiver);
        if let Term::Global(head) = base {
            if !is_marker(head) && !locals.contains(head) {
                if let Some(name) = Name::parse(head) {
                    for take in 1..=fields.len() {
                        let mut parts = name.parts.clone();
                        parts.extend(fields[..take].iter().map(|f| f.to_string()));
                        if let Resolution::Found(key) = self.env.resolve(self.ns, &Name { parts }) {
                            return Err(ParseError::Unsupported {
                                what: format!(
                                    "the projection `.{}` of `{head}`",
                                    fields[..take].join(".")
                                ),
                                why: format!(
                                    "written back, it would read as the declared name `{key}` \
                                     rather than as a field of `{head}`"
                                ),
                            });
                        }
                    }
                }
            }
        }
        Ok(respelled.map(|base| fields.iter().fold(base, |acc, field| project(acc, field))))
    }
}

/// Fill every OMITTED DEFAULT ARGUMENT in `term`, at every application spine in it.
///
/// ★ THIS IS THE SECOND HALF OF THE SAME STAGING [`resolve_names`] IS THE FIRST HALF OF. Lean's
/// order is: resolve what the names denote, expand what the spellings abbreviate, and only then
/// hand a term to the elaborator — which is why the kernel needs to know nothing about either.
/// `withDefault 1` is a SPELLING of `withDefault 1 3`, exactly as `token` inside `open Alpha` is a
/// spelling of `Alpha.token`, so it is expanded here rather than being taught to the elaborator as
/// a fourth kind of argument insertion.
///
/// A BARE NAME IS NOT A CALL SITE. `Check withDefault.` reports the function, and
/// `Definition g : Int -> Int := withDefault.` is the function it already was — Lean answers both
/// the same way, and filling a default into a term nobody applied would change what an unapplied
/// constant MEANS. Filling stops at the first parameter with no default, which is what makes the
/// trailing-position rule positional: `f 1` against `(a) (m := 3) (b)` becomes `f 1 3`, still
/// awaiting `b`.
///
/// Takes and returns a `Cow` so it composes directly onto [`resolve_names`]'s result: a term that
/// omits no default costs one read-only walk, and a session that declared no default at all costs
/// not even that (`Surface::fills_defaults`).
pub fn fill_defaults<'t>(
    surface: &Surface,
    term: std::borrow::Cow<'t, Term>,
) -> std::borrow::Cow<'t, Term> {
    if !surface.fills_defaults() {
        return term;
    }
    match fill_step(surface, &term) {
        Some(rebuilt) => std::borrow::Cow::Owned(rebuilt),
        None => term,
    }
}

/// [`fill_defaults`]'s worker. `None` means "unchanged", which is what keeps the untouched case
/// allocation-free all the way down the tree.
fn fill_step(surface: &Surface, term: &Term) -> Option<Term> {
    match term {
        Term::App(..) => {
            // A spine is filled AS A SPINE. `f a b` is `App(App(f, a), b)`, and a default belongs
            // to the whole application rather than to its innermost node — walking the `App` arms
            // one at a time would offer `f a` its second argument and then apply `b` to the
            // result.
            let (head, args) = spine(term);
            let mut changed = false;
            let filled_head = match fill_step(surface, head) {
                Some(h) => {
                    changed = true;
                    h
                }
                None => head.clone(),
            };
            let mut filled: Vec<Term> = args
                .iter()
                .map(|a| match fill_step(surface, a) {
                    Some(f) => {
                        changed = true;
                        f
                    }
                    None => (*a).clone(),
                })
                .collect();
            if let Term::Global(name) = filled_head.peel_annotations() {
                if let Some(binders) = surface.defaults_of(name) {
                    changed |= extend_with_defaults(surface, binders, &mut filled);
                }
            }
            changed.then(|| {
                filled
                    .into_iter()
                    .fold(filled_head, |f, a| Term::App(Box::new(f), Box::new(a)))
            })
        }
        // A NAME ON ITS OWN IS NOT AN APPLICATION, and neither is a literal, a sort or a hole.
        Term::Global(_)
        | Term::Var(_)
        | Term::Sort(_)
        | Term::Lit(_)
        | Term::Hole
        | Term::Const { .. } => None,
        Term::Lambda {
            param,
            param_type,
            body,
        } => rebuild2(
            fill_step(surface, param_type),
            fill_step(surface, body),
            param_type,
            body,
            |t, b| Term::Lambda {
                param: param.clone(),
                param_type: Box::new(t),
                body: Box::new(b),
            },
        ),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => rebuild2(
            fill_step(surface, param_type),
            fill_step(surface, body_type),
            param_type,
            body_type,
            |t, b| Term::Pi {
                param: param.clone(),
                param_type: Box::new(t),
                body_type: Box::new(b),
            },
        ),
        Term::Fix { name, body } => fill_step(surface, body).map(|b| Term::Fix {
            name: name.clone(),
            body: Box::new(b),
        }),
        Term::MutualFix { defs, index } => {
            let mut changed = false;
            let out: Vec<(String, Term)> = defs
                .iter()
                .map(|(n, b)| match fill_step(surface, b) {
                    Some(nb) => {
                        changed = true;
                        (n.clone(), nb)
                    }
                    None => (n.clone(), b.clone()),
                })
                .collect();
            changed.then_some(Term::MutualFix {
                defs: out,
                index: *index,
            })
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            let rd = fill_step(surface, discriminant);
            let rm = fill_step(surface, motive);
            let mut changed = rd.is_some() || rm.is_some();
            let out: Vec<Term> = cases
                .iter()
                .map(|c| match fill_step(surface, c) {
                    Some(nc) => {
                        changed = true;
                        nc
                    }
                    None => c.clone(),
                })
                .collect();
            changed.then(|| Term::Match {
                discriminant: Box::new(rd.unwrap_or_else(|| (**discriminant).clone())),
                motive: Box::new(rm.unwrap_or_else(|| (**motive).clone())),
                cases: out,
            })
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            let rt = fill_step(surface, ty);
            let rv = fill_step(surface, value);
            let rb = fill_step(surface, body);
            if rt.is_none() && rv.is_none() && rb.is_none() {
                return None;
            }
            Some(Term::Let {
                name: name.clone(),
                ty: Box::new(rt.unwrap_or_else(|| (**ty).clone())),
                value: Box::new(rv.unwrap_or_else(|| (**value).clone())),
                body: Box::new(rb.unwrap_or_else(|| (**body).clone())),
            })
        }
        Term::Annotated { annotation, term } => fill_step(surface, term).map(|t| Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(t),
        }),
    }
}

/// Append the defaults the WRITTEN arguments stopped short of, returning whether any were.
///
/// ★ A WRITTEN ARGUMENT ANSWERS TO AN EXPLICIT BINDER, NOT TO A TELESCOPE POSITION. The filling
/// runs BEFORE elaboration, so `{A}` and `[C x]` binders have not been inserted yet and the
/// caller's `k`-th argument is the `k`-th EXPLICIT parameter — for
/// `{A : Type} -> A -> (m : Int := 7) -> Int`, `tagged Zero` has supplied parameter TWO and the
/// default it stopped short of is parameter THREE. Counting positions instead of explicit
/// positions read `Zero` as the implicit and filled nothing, which is the silent half of this
/// seam: the call is then a partial application and the mistake surfaces as an arity error
/// somewhere else entirely.
///
/// Two things stop the filling, and each is a refusal rather than a guess. A parameter with no
/// default ends it (that is what makes the rule positional). And a default that mentions a
/// parameter whose argument is NOT written — an implicit the elaborator has yet to solve — is not
/// filled at all: substituting the one thing we have for it would put a `Term::Hole` in an
/// argument position, and a hole matches ANY type.
fn extend_with_defaults(
    surface: &Surface,
    binders: &[BinderSurface],
    written: &mut Vec<Term>,
) -> bool {
    // Align the written arguments against the telescope: each explicit binder consumes one, each
    // inserted binder takes a placeholder that is never a legitimate substitution.
    let mut aligned: Vec<Term> = Vec::with_capacity(binders.len());
    let mut consumed = 0usize;
    let mut at = binders.len();
    for (i, binder) in binders.iter().enumerate() {
        if binder.kind != BinderKind::Explicit {
            aligned.push(Term::Hole);
            continue;
        }
        match written.get(consumed) {
            Some(arg) => {
                aligned.push(arg.clone());
                consumed += 1;
            }
            None => {
                at = i;
                break;
            }
        }
    }
    if consumed < written.len() {
        return false; // more arguments than parameters — the kernel reports that, not this
    }

    let mut changed = false;
    while at < binders.len() && binders[at].kind == BinderKind::Explicit {
        let Some(value) = TermParser::inline_default(binders, at, &aligned) else {
            break;
        };
        // A placeholder that reached the value means the default named a parameter the caller has
        // not supplied and the elaborator has not yet solved.
        if has_hole(&value) && !binders[at].default.as_ref().is_some_and(has_hole) {
            break;
        }
        // The default is itself source, so it may omit a default of its own.
        let value = fill_step(surface, &value).unwrap_or(value);
        aligned.push(value.clone());
        written.push(value);
        changed = true;
        at += 1;
    }
    changed
}

/// Does `term` contain a `Term::Hole` anywhere?
fn has_hole(term: &Term) -> bool {
    match term {
        Term::Hole => true,
        Term::App(f, a) => has_hole(f) || has_hole(a),
        Term::Lambda {
            param_type, body, ..
        } => has_hole(param_type) || has_hole(body),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => has_hole(param_type) || has_hole(body_type),
        Term::Fix { body, .. } => has_hole(body),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| has_hole(b)),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => has_hole(discriminant) || has_hole(motive) || cases.iter().any(has_hole),
        Term::Let {
            ty, value, body, ..
        } => has_hole(ty) || has_hole(value) || has_hole(body),
        Term::Annotated { term, .. } => has_hole(term),
        Term::Global(_) | Term::Var(_) | Term::Sort(_) | Term::Lit(_) | Term::Const { .. } => false,
    }
}

/// An application split into its head and its arguments, outermost-last — `f a b` is
/// `(f, [a, b])`. A non-application is its own head with no arguments.
fn spine(term: &Term) -> (&Term, Vec<&Term>) {
    let mut args = Vec::new();
    let mut cur = term;
    while let Term::App(f, a) = cur {
        args.push(a.as_ref());
        cur = f;
    }
    args.reverse();
    (cur, args)
}

/// Rebuild a two-child node only if a child changed, cloning the untouched side.
fn rebuild2(
    left: Option<Term>,
    right: Option<Term>,
    orig_left: &Term,
    orig_right: &Term,
    build: impl FnOnce(Term, Term) -> Term,
) -> Option<Term> {
    if left.is_none() && right.is_none() {
        return None;
    }
    Some(build(
        left.unwrap_or_else(|| orig_left.clone()),
        right.unwrap_or_else(|| orig_right.clone()),
    ))
}
