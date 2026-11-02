//! Term walks the emitter needs: the globals a term references, renaming globals and universe
//! variables into LOGOS names, fresh readable binder names, and strict α-equivalence.
//!
//! Every walk here is total over `Term` and refuses what an emitted declaration must never
//! contain: a hole, a free variable, a universe variable the declaration does not bind.

use crate::{Term, Universe};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Every constant `term` names, `Global` and `Const` heads alike.
pub(super) fn references(term: &Term, out: &mut BTreeSet<String>) {
    let mut work = vec![term];
    while let Some(t) = work.pop() {
        match t {
            Term::Global(n) => {
                out.insert(n.clone());
            }
            Term::Const { name, .. } => {
                out.insert(name.clone());
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                work.push(param_type);
                work.push(body_type);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                work.push(param_type);
                work.push(body);
            }
            Term::App(f, a) => {
                work.push(f);
                work.push(a);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.push(discriminant);
                work.push(motive);
                work.extend(cases.iter());
            }
            Term::Fix { body, .. } => work.push(body),
            Term::MutualFix { defs, .. } => work.extend(defs.iter().map(|(_, b)| b)),
            Term::Let {
                ty, value, body, ..
            } => {
                work.push(ty);
                work.push(value);
                work.push(body);
            }
            Term::Annotated { term, .. } => work.push(term),
            Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => {}
        }
    }
}

/// Why a term cannot be carried into LOGOS source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TermFault {
    /// A constant with no LOGOS name.
    UnmappedGlobal(String),
    /// A universe variable the declaration does not bind.
    UnboundLevel(String),
    /// A variable no binder in the term binds.
    FreeVariable(String),
    /// An elaboration hole: a checked declaration never contains one.
    Hole,
}

impl std::fmt::Display for TermFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TermFault::UnmappedGlobal(n) => write!(f, "the constant `{n}` has no LOGOS name"),
            TermFault::UnboundLevel(u) => {
                write!(f, "the universe variable `{u}` is not a parameter")
            }
            TermFault::FreeVariable(v) => write!(f, "the variable `{v}` is free"),
            TermFault::Hole => f.write_str("the term contains an elaboration hole"),
        }
    }
}

/// Rename a universe level's variables through `levels`.
pub(super) fn rename_level(
    u: &Universe,
    levels: &HashMap<String, String>,
) -> Result<Universe, TermFault> {
    Ok(match u {
        Universe::SProp | Universe::Prop | Universe::Type(_) => u.clone(),
        Universe::Var(v) => Universe::Var(
            levels
                .get(v)
                .cloned()
                .ok_or_else(|| TermFault::UnboundLevel(v.clone()))?,
        ),
        Universe::Succ(a) => Universe::Succ(Box::new(rename_level(a, levels)?)),
        Universe::Max(a, b) => Universe::Max(
            Box::new(rename_level(a, levels)?),
            Box::new(rename_level(b, levels)?),
        ),
        Universe::IMax(a, b) => Universe::IMax(
            Box::new(rename_level(a, levels)?),
            Box::new(rename_level(b, levels)?),
        ),
    })
}

/// Rename every constant through `globals` and every universe variable through `levels`. Binders
/// and variables are untouched; a constant or level with no entry is a [`TermFault`].
pub(super) fn rename(
    term: &Term,
    globals: &dyn Fn(&str) -> Option<String>,
    levels: &HashMap<String, String>,
) -> Result<Term, TermFault> {
    let r = |t: &Term| rename(t, globals, levels);
    Ok(match term {
        Term::Sort(u) => Term::Sort(rename_level(u, levels)?),
        Term::Var(v) => Term::Var(v.clone()),
        Term::Global(n) => {
            Term::Global(globals(n).ok_or_else(|| TermFault::UnmappedGlobal(n.clone()))?)
        }
        Term::Const { name, levels: args } => Term::Const {
            name: globals(name).ok_or_else(|| TermFault::UnmappedGlobal(name.clone()))?,
            levels: args
                .iter()
                .map(|u| rename_level(u, levels))
                .collect::<Result<_, _>>()?,
        },
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(r(param_type)?),
            body_type: Box::new(r(body_type)?),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(r(param_type)?),
            body: Box::new(r(body)?),
        },
        Term::App(f, a) => Term::App(Box::new(r(f)?), Box::new(r(a)?)),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(r(discriminant)?),
            motive: Box::new(r(motive)?),
            cases: cases.iter().map(r).collect::<Result<_, _>>()?,
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: Box::new(r(body)?),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| Ok((n.clone(), r(b)?)))
                .collect::<Result<_, _>>()?,
            index: *index,
        },
        Term::Let {
            name,
            ty,
            value,
            body,
        } => Term::Let {
            name: name.clone(),
            ty: Box::new(r(ty)?),
            value: Box::new(r(value)?),
            body: Box::new(r(body)?),
        },
        Term::Lit(l) => Term::Lit(l.clone()),
        Term::Hole => return Err(TermFault::Hole),
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(r(term)?),
        },
    })
}

/// Words no emitted binder may be named: the vernacular's and the literate surface's keywords, and
/// the sort and level words. A superset on purpose — a binder that reads as syntax is a capture.
pub(super) const RESERVED_WORDS: &[&str] = &[
    "fun",
    "forall",
    "fix",
    "mutualfix",
    "match",
    "let",
    "in",
    "with",
    "return",
    "end",
    "by",
    "Type",
    "Prop",
    "SProp",
    "Sort",
    "max",
    "imax",
    "succ",
    "Definition",
    "Theorem",
    "Axiom",
    "Opaque",
    "Inductive",
    "Check",
    "Eval",
    "Open",
    "Namespace",
    "End",
    "Section",
    "Export",
    "Protected",
    "Print",
    "Notation",
    "Universe",
    "universe",
    "_root_",
    "_",
    "of",
    "and",
    "or",
    "is",
    "either",
    "a",
    "an",
    "A",
    "An",
    "the",
    "Name",
    "Variable",
    "Apply",
    "there",
    "exists",
    "such",
    "that",
    "When",
    "Yield",
    "Consider",
    "given",
    "yields",
    "equals",
    "implies",
    "be",
    "Let",
    "To",
    "Statement",
    "Proof",
    "if",
    "then",
    "else",
    "do",
];

pub(super) fn is_reserved_word(word: &str) -> bool {
    RESERVED_WORDS.contains(&word)
}

/// The words the TERM surface reads as syntax where a binder name stands — the only words a binder
/// may never be called. The literate words (`a`, `of`, `is`, …) are deliberately absent: a binder never
/// appears in a literate form the literate reader could misread, because `emit_literate` refuses a
/// literate candidate whose parameter is a literate word and the vernacular is written instead.
const BINDER_RESERVED: &[&str] = &[
    "fun",
    "forall",
    "fix",
    "mutualfix",
    "match",
    "let",
    "in",
    "with",
    "return",
    "end",
    "by",
    "Type",
    "Prop",
    "SProp",
    "Sort",
    "_root_",
    "_",
];

/// A name a binder can carry: one component the name grammar reads whole and the printer writes bare —
/// not a term keyword, not a sort word (`Type1` reads as a universe).
fn is_binder_word(word: &str) -> bool {
    !word.is_empty()
        && !BINDER_RESERVED.contains(&word)
        && !crate::interface::term_parser::is_sort_word(word)
        && word
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
        && crate::interface::term_parser::scan_name(word) == Some((word.to_string(), word.len()))
}

/// Why a Lean binder name could not be kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RenameReason {
    /// A constant the document writes has a component of that name: the binder would capture it.
    Constant,
    /// The LOGOS prelude declares a constant of that name: the binder would read as that constant.
    Prelude,
    /// An enclosing binder already has that name: the binder would shadow it.
    Shadow,
    /// The term surface reads the name as syntax, or cannot read it as one name.
    Syntax,
}

impl RenameReason {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            RenameReason::Constant => "a constant written in this document has that name",
            RenameReason::Prelude => "the LOGOS prelude declares a constant of that name",
            RenameReason::Shadow => "an enclosing binder already has that name",
            RenameReason::Syntax => "the term surface reads that name as syntax",
        }
    }
}

/// The names a written binder may not take, each set with its reason.
pub(super) struct Unavailable<'a> {
    /// Every component of every name the document writes: a binder of that name would capture it.
    pub constants: &'a HashSet<String>,
    /// Every name the LOGOS prelude declares: a binder of that name would read as that constant.
    pub prelude: &'a HashSet<String>,
}

/// The name of a Π binder the printer writes as an arrow (`A -> B`): nothing refers to it, so it has
/// none, exactly as the term door reads an arrow back.
const ARROW: &str = "_";

/// One Lean binder name that was not kept: (Lean name, written name, reason).
pub(super) type Rename = (String, String, RenameReason);

/// A plain LOGOS identifier: one component the name grammar reads whole, not a quoted name, not a
/// reserved word.
pub(super) fn is_plain_identifier(word: &str) -> bool {
    !word.is_empty()
        && !is_reserved_word(word)
        && word
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
        && crate::interface::term_parser::scan_name(word) == Some((word.to_string(), word.len()))
}

/// Name every binder the text will write, readably and freshly; every other Π becomes an arrow.
///
/// A written binder's name is never one of `unavailable` (so it can neither capture a constant the
/// document writes nor read as one the prelude declares) and never the name of an enclosing written
/// binder (so no binder shadows another, whatever the printer does with a shadowed name). A Π binder
/// nothing refers to is written as an arrow and named `_`: it keeps no Lean name and loses none. The
/// term must be closed.
///
/// The first `prefix` Π binders are named whether or not anything refers to them — an inductive's
/// parameters, which its constructors refer to by name.
pub(super) fn freshen_naming(
    term: &Term,
    prefix: usize,
    unavailable: &Unavailable<'_>,
    renames: &mut Vec<Rename>,
    hygienic: &mut bool,
) -> Result<Term, TermFault> {
    let mut named = named_pis(term);
    let mut t = term;
    for _ in 0..prefix {
        let Term::Pi { body_type, .. } = t else { break };
        named.insert(t as *const Term);
        t = body_type;
    }
    let mut f = Freshener::new(unavailable, named);
    let out = f.walk(term)?;
    renames.append(&mut f.renames);
    *hygienic |= f.hygienic;
    Ok(out)
}

/// The Π binders of `term` that a variable in their body refers to — the only Π binders the printer
/// writes with a name. Variables resolve to their innermost binder of that name, as everywhere else.
fn named_pis(term: &Term) -> HashSet<*const Term> {
    enum Visit<'t> {
        Enter(&'t Term),
        Bind(&'t str, Option<*const Term>),
        Unbind(&'t str),
    }
    let mut bound: HashMap<&str, Vec<Option<*const Term>>> = HashMap::new();
    let mut named = HashSet::new();
    let mut work = vec![Visit::Enter(term)];
    while let Some(visit) = work.pop() {
        let t = match visit {
            Visit::Bind(name, pi) => {
                bound.entry(name).or_default().push(pi);
                continue;
            }
            Visit::Unbind(name) => {
                if let Some(stack) = bound.get_mut(name) {
                    stack.pop();
                }
                continue;
            }
            Visit::Enter(t) => t,
        };
        match t {
            Term::Var(v) => {
                if let Some(Some(pi)) = bound.get(v.as_str()).and_then(|stack| stack.last()) {
                    named.insert(*pi);
                }
            }
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                work.push(Visit::Unbind(param));
                work.push(Visit::Enter(body_type));
                work.push(Visit::Bind(param, Some(t as *const Term)));
                work.push(Visit::Enter(param_type));
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                work.push(Visit::Unbind(param));
                work.push(Visit::Enter(body));
                work.push(Visit::Bind(param, None));
                work.push(Visit::Enter(param_type));
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                work.push(Visit::Unbind(name));
                work.push(Visit::Enter(body));
                work.push(Visit::Bind(name, None));
                work.push(Visit::Enter(value));
                work.push(Visit::Enter(ty));
            }
            Term::Fix { name, body } => {
                work.push(Visit::Unbind(name));
                work.push(Visit::Enter(body));
                work.push(Visit::Bind(name, None));
            }
            Term::MutualFix { defs, .. } => {
                for (name, _) in defs {
                    work.push(Visit::Unbind(name));
                }
                for (_, body) in defs {
                    work.push(Visit::Enter(body));
                }
                for (name, _) in defs {
                    work.push(Visit::Bind(name, None));
                }
            }
            Term::App(f, a) => {
                work.push(Visit::Enter(a));
                work.push(Visit::Enter(f));
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.extend(cases.iter().map(Visit::Enter));
                work.push(Visit::Enter(motive));
                work.push(Visit::Enter(discriminant));
            }
            Term::Annotated { term, .. } => work.push(Visit::Enter(term)),
            Term::Sort(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) | Term::Hole => {}
        }
    }
    named
}

/// A Lean binder name with its macro scopes erased, as Lean's `Name.eraseMacroScopes` does: the
/// hygienic `a._@.Init.Prelude._hyg.191`, which Lean displays as the inaccessible `a✝`, is `a`. Also
/// says whether there were scopes to erase.
fn erase_macro_scopes(name: &str) -> (String, bool) {
    match name.find(".\u{ab}_@\u{bb}").or_else(|| name.find("._@.")) {
        Some(at) => (name[..at].to_string(), true),
        None => (name.to_string(), false),
    }
}

/// [`freshen_naming`] with the first `forced.len()` Π binders named exactly `forced` — an inductive's
/// parameters, which every constructor must bind under the same names.
pub(super) fn freshen_with_prefix(
    term: &Term,
    forced: &[String],
    unavailable: &Unavailable<'_>,
    renames: &mut Vec<Rename>,
    hygienic: &mut bool,
) -> Result<Term, TermFault> {
    let mut f = Freshener::new(unavailable, named_pis(term));
    let out = f.forced_prefix(term, forced)?;
    renames.append(&mut f.renames);
    *hygienic |= f.hygienic;
    Ok(out)
}

struct Freshener<'a> {
    unavailable: &'a Unavailable<'a>,
    /// The Π binders the text writes with a name ([`named_pis`], plus a forced prefix).
    named: HashSet<*const Term>,
    /// (source name, emitted name), innermost last.
    scope: Vec<(String, String)>,
    /// Every Lean binder name that could not be kept.
    renames: Vec<Rename>,
    /// Whether a hygienic (inaccessible) Lean binder name was written under its user-facing name.
    hygienic: bool,
}

impl<'a> Freshener<'a> {
    fn new(unavailable: &'a Unavailable<'a>, named: HashSet<*const Term>) -> Self {
        Freshener {
            unavailable,
            named,
            scope: Vec::new(),
            renames: Vec::new(),
            hygienic: false,
        }
    }

    /// The binder's written name. A Lean binder name is KEPT unless it would capture a constant the
    /// document writes, read as a constant the prelude declares, shadow an enclosing binder, or read
    /// as syntax; then the first of `name1`, `name2`, … that is free is written, and the rename is
    /// recorded with its reason. A binder the reader named itself gets a conventional name read off
    /// its type.
    fn pick(&mut self, old: &str, ty: Option<&Term>) -> String {
        let lean = crate::interface::lean::lean_binder_name(old).map(|name| {
            let (base, hygienic) = erase_macro_scopes(name);
            self.hygienic |= hygienic;
            base
        });
        let why = |this: &Self, n: &str| {
            if !is_binder_word(n) {
                Some(RenameReason::Syntax)
            } else if this.unavailable.constants.contains(n) {
                Some(RenameReason::Constant)
            } else if this.unavailable.prelude.contains(n) {
                Some(RenameReason::Prelude)
            } else if this.scope.iter().any(|(_, new)| new == n) {
                Some(RenameReason::Shadow)
            } else {
                None
            }
        };
        let base = match &lean {
            Some(name) if is_binder_word(name) => name.clone(),
            _ => binder_base(old, ty),
        };
        let conflict = why(self, &base);
        let reason = match &lean {
            Some(name) if !is_binder_word(name) => Some(RenameReason::Syntax),
            _ => conflict,
        };
        let written = match conflict {
            None => base,
            Some(_) => (1usize..)
                .map(|i| format!("{base}{i}"))
                .find(|n| why(self, n).is_none())
                .expect("an unbounded sequence of candidates has a free one"),
        };
        if let (Some(name), Some(reason)) = (lean, reason) {
            self.renames.push((name, written.clone(), reason));
        }
        written
    }

    fn bind<T>(&mut self, old: &str, new: String, inner: impl FnOnce(&mut Self) -> T) -> T {
        self.scope.push((old.to_string(), new));
        let out = inner(self);
        self.scope.pop();
        out
    }

    fn forced_prefix(&mut self, term: &Term, forced: &[String]) -> Result<Term, TermFault> {
        match (forced.split_first(), term) {
            (None, _) => self.walk(term),
            (
                Some((name, rest)),
                Term::Pi {
                    param,
                    param_type,
                    body_type,
                },
            ) => {
                let param_type = self.walk(param_type)?;
                let body = self.bind(param, name.clone(), |s| s.forced_prefix(body_type, rest))?;
                Ok(Term::Pi {
                    param: name.clone(),
                    param_type: Box::new(param_type),
                    body_type: Box::new(body),
                })
            }
            (Some(_), _) => self.walk(term),
        }
    }

    fn walk(&mut self, term: &Term) -> Result<Term, TermFault> {
        Ok(match term {
            Term::Var(v) => match self.scope.iter().rev().find(|(old, _)| old == v) {
                Some((_, new)) => Term::Var(new.clone()),
                None => return Err(TermFault::FreeVariable(v.clone())),
            },
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                let param_type = self.walk(param_type)?;
                // An arrow names nothing: no Lean name is kept or lost, and no binder inside it is
                // renamed on its account (`_` is never a written binder's name).
                let new = if self.named.contains(&(term as *const Term)) {
                    self.pick(param, Some(&param_type))
                } else {
                    ARROW.to_string()
                };
                let body = self.bind(param, new.clone(), |s| s.walk(body_type))?;
                Term::Pi {
                    param: new,
                    param_type: Box::new(param_type),
                    body_type: Box::new(body),
                }
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                let param_type = self.walk(param_type)?;
                let new = self.pick(param, Some(&param_type));
                let body = self.bind(param, new.clone(), |s| s.walk(body))?;
                Term::Lambda {
                    param: new,
                    param_type: Box::new(param_type),
                    body: Box::new(body),
                }
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                let ty = self.walk(ty)?;
                let value = self.walk(value)?;
                let new = self.pick(name, Some(&ty));
                let body = self.bind(name, new.clone(), |s| s.walk(body))?;
                Term::Let {
                    name: new,
                    ty: Box::new(ty),
                    value: Box::new(value),
                    body: Box::new(body),
                }
            }
            Term::Fix { name, body } => {
                let new = self.pick(name, None);
                let body = self.bind(name, new.clone(), |s| s.walk(body))?;
                Term::Fix {
                    name: new,
                    body: Box::new(body),
                }
            }
            Term::MutualFix { defs, index } => {
                let mut news = Vec::with_capacity(defs.len());
                for (old, _) in defs {
                    let new = self.pick(old, None);
                    self.scope.push((old.clone(), new.clone()));
                    news.push(new);
                }
                let bodies: Result<Vec<Term>, TermFault> =
                    defs.iter().map(|(_, b)| self.walk(b)).collect();
                for _ in defs {
                    self.scope.pop();
                }
                Term::MutualFix {
                    defs: news.into_iter().zip(bodies?).collect(),
                    index: *index,
                }
            }
            Term::App(f, a) => Term::App(Box::new(self.walk(f)?), Box::new(self.walk(a)?)),
            Term::Match {
                discriminant,
                motive,
                cases,
            } => Term::Match {
                discriminant: Box::new(self.walk(discriminant)?),
                motive: Box::new(self.walk(motive)?),
                cases: cases
                    .iter()
                    .map(|c| self.walk(c))
                    .collect::<Result<_, _>>()?,
            },
            Term::Sort(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) => term.clone(),
            Term::Hole => return Err(TermFault::Hole),
            Term::Annotated { annotation, term } => Term::Annotated {
                annotation: annotation.clone(),
                term: Box::new(self.walk(term)?),
            },
        })
    }
}

/// The stem for a binder with no usable Lean name: its source name when that is already a plain
/// lowercase identifier, otherwise a conventional name read off its type — `α` for a type, the
/// lowercase initial of the type's head constant (`n` for `Nat`), and `x` otherwise.
fn binder_base(old: &str, ty: Option<&Term>) -> String {
    let lowercase_word = old.chars().next().is_some_and(|c| c.is_lowercase())
        && old.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && is_binder_word(old);
    if lowercase_word {
        return old.to_string();
    }
    let Some(ty) = ty else { return "f".to_string() };
    let mut head = ty;
    while let Term::App(f, _) = head {
        head = f;
    }
    match head {
        Term::Sort(_) => "α".to_string(),
        Term::Pi { .. } => "f".to_string(),
        Term::Global(n) | Term::Const { name: n, .. } => {
            let short = n.rsplit('.').next().unwrap_or(n);
            match short.chars().find(|c| c.is_alphabetic()) {
                Some(c) if c.is_ascii() => c.to_ascii_lowercase().to_string(),
                _ => "x".to_string(),
            }
        }
        _ => "x".to_string(),
    }
}

/// Structural α-equivalence with universes compared STRUCTURALLY: `Prop` is not `Type 0`, `max u v`
/// is not `max v u`. Bound names are irrelevant; a hole equals nothing.
pub(super) fn alpha_eq(a: &Term, b: &Term) -> bool {
    Alpha {
        left: Vec::new(),
        right: Vec::new(),
    }
    .eq(a, b)
}

struct Alpha<'a> {
    left: Vec<&'a str>,
    right: Vec<&'a str>,
}

impl<'a> Alpha<'a> {
    fn depth_of(stack: &[&str], name: &str) -> Option<usize> {
        stack.iter().rev().position(|bound| *bound == name)
    }

    fn under(&mut self, l: &'a str, r: &'a str, a: &'a Term, b: &'a Term) -> bool {
        self.left.push(l);
        self.right.push(r);
        let same = self.eq(a, b);
        self.left.pop();
        self.right.pop();
        same
    }

    fn eq(&mut self, a: &'a Term, b: &'a Term) -> bool {
        match (a, b) {
            (Term::Hole, _) | (_, Term::Hole) => false,
            (Term::Sort(u), Term::Sort(v)) => u == v,
            (Term::Lit(x), Term::Lit(y)) => x == y,
            (Term::Global(x), Term::Global(y)) => x == y,
            (
                Term::Const {
                    name: x,
                    levels: lx,
                },
                Term::Const {
                    name: y,
                    levels: ly,
                },
            ) => x == y && lx == ly,
            (Term::Var(x), Term::Var(y)) => {
                match (
                    Self::depth_of(&self.left, x),
                    Self::depth_of(&self.right, y),
                ) {
                    (Some(i), Some(j)) => i == j,
                    (None, None) => x == y,
                    _ => false,
                }
            }
            (
                Term::Pi {
                    param: p,
                    param_type: pt,
                    body_type: pb,
                },
                Term::Pi {
                    param: q,
                    param_type: qt,
                    body_type: qb,
                },
            )
            | (
                Term::Lambda {
                    param: p,
                    param_type: pt,
                    body: pb,
                },
                Term::Lambda {
                    param: q,
                    param_type: qt,
                    body: qb,
                },
            ) => self.eq(pt, qt) && self.under(p, q, pb, qb),
            (Term::App(f, x), Term::App(g, y)) => self.eq(f, g) && self.eq(x, y),
            (
                Term::Match {
                    discriminant: d1,
                    motive: m1,
                    cases: c1,
                },
                Term::Match {
                    discriminant: d2,
                    motive: m2,
                    cases: c2,
                },
            ) => {
                c1.len() == c2.len()
                    && self.eq(d1, d2)
                    && self.eq(m1, m2)
                    && c1.iter().zip(c2).all(|(x, y)| self.eq(x, y))
            }
            (Term::Fix { name: n1, body: b1 }, Term::Fix { name: n2, body: b2 }) => {
                self.under(n1, n2, b1, b2)
            }
            (
                Term::MutualFix {
                    defs: d1,
                    index: i1,
                },
                Term::MutualFix {
                    defs: d2,
                    index: i2,
                },
            ) => {
                if i1 != i2 || d1.len() != d2.len() {
                    return false;
                }
                for ((x, _), (y, _)) in d1.iter().zip(d2) {
                    self.left.push(x);
                    self.right.push(y);
                }
                let same = d1.iter().zip(d2).all(|((_, x), (_, y))| self.eq(x, y));
                for _ in d1 {
                    self.left.pop();
                    self.right.pop();
                }
                same
            }
            (
                Term::Let {
                    name: n1,
                    ty: t1,
                    value: v1,
                    body: b1,
                },
                Term::Let {
                    name: n2,
                    ty: t2,
                    value: v2,
                    body: b2,
                },
            ) => self.eq(t1, t2) && self.eq(v1, v2) && self.under(n1, n2, b1, b2),
            _ => false,
        }
    }
}

/// The first `n` Π binders of `term` and what remains.
pub(super) fn split_pis(term: &Term, n: usize) -> Option<(Vec<(String, Term)>, Term)> {
    let mut binders = Vec::with_capacity(n);
    let mut rest = term;
    for _ in 0..n {
        match rest {
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                binders.push((param.clone(), (**param_type).clone()));
                rest = body_type;
            }
            _ => return None,
        }
    }
    Some((binders, rest.clone()))
}

/// Does `term` mention the variable `name` free?
pub(super) fn mentions_var(term: &Term, name: &str) -> bool {
    match term {
        Term::Var(v) => v == name,
        Term::Pi {
            param,
            param_type,
            body_type,
        } => mentions_var(param_type, name) || (param != name && mentions_var(body_type, name)),
        Term::Lambda {
            param,
            param_type,
            body,
        } => mentions_var(param_type, name) || (param != name && mentions_var(body, name)),
        Term::Let {
            name: n,
            ty,
            value,
            body,
        } => {
            mentions_var(ty, name)
                || mentions_var(value, name)
                || (n != name && mentions_var(body, name))
        }
        Term::App(f, a) => mentions_var(f, name) || mentions_var(a, name),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            mentions_var(discriminant, name)
                || mentions_var(motive, name)
                || cases.iter().any(|c| mentions_var(c, name))
        }
        Term::Fix { name: n, body } => n != name && mentions_var(body, name),
        Term::MutualFix { defs, .. } => {
            !defs.iter().any(|(n, _)| n == name) && defs.iter().any(|(_, b)| mentions_var(b, name))
        }
        Term::Annotated { term, .. } => mentions_var(term, name),
        Term::Sort(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) | Term::Hole => false,
    }
}

/// Rewrite every reference through `spell` — the scope-relative written form of an absolute key. A
/// constant written at explicit levels (`c.{u}`) is a reference exactly as a bare one is: the term door
/// resolves its name in the same scope, so it is spelled the same way.
pub(super) fn respell(term: &Term, spell: &mut dyn FnMut(&str) -> String) -> Term {
    match term {
        Term::Global(k) => Term::Global(spell(k)),
        Term::Const { name, levels } => Term::Const {
            name: spell(name),
            levels: levels.clone(),
        },
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(respell(param_type, spell)),
            body_type: Box::new(respell(body_type, spell)),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(respell(param_type, spell)),
            body: Box::new(respell(body, spell)),
        },
        Term::App(f, a) => Term::App(Box::new(respell(f, spell)), Box::new(respell(a, spell))),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(respell(discriminant, spell)),
            motive: Box::new(respell(motive, spell)),
            cases: cases.iter().map(|c| respell(c, spell)).collect(),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: Box::new(respell(body, spell)),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| (n.clone(), respell(b, spell)))
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
            ty: Box::new(respell(ty, spell)),
            value: Box::new(respell(value, spell)),
            body: Box::new(respell(body, spell)),
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(respell(term, spell)),
        },
        Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => term.clone(),
    }
}

/// Replace every FREE occurrence of a variable in `names` by the constant of that name — how an
/// inductive's parameter is written inside its own declaration.
pub(super) fn name_parameters(term: &Term, names: &[String]) -> Term {
    fn go(t: &Term, names: &[String], bound: &mut Vec<String>) -> Term {
        let under = |binder: &str, x: &Term, bound: &mut Vec<String>| {
            bound.push(binder.to_string());
            let out = go(x, names, bound);
            bound.pop();
            out
        };
        match t {
            Term::Var(v) if names.contains(v) && !bound.contains(v) => Term::Global(v.clone()),
            Term::Pi {
                param,
                param_type,
                body_type,
            } => Term::Pi {
                param: param.clone(),
                param_type: Box::new(go(param_type, names, bound)),
                body_type: Box::new(under(param, body_type, bound)),
            },
            Term::Lambda {
                param,
                param_type,
                body,
            } => Term::Lambda {
                param: param.clone(),
                param_type: Box::new(go(param_type, names, bound)),
                body: Box::new(under(param, body, bound)),
            },
            Term::Let {
                name,
                ty,
                value,
                body,
            } => Term::Let {
                name: name.clone(),
                ty: Box::new(go(ty, names, bound)),
                value: Box::new(go(value, names, bound)),
                body: Box::new(under(name, body, bound)),
            },
            Term::Fix { name, body } => Term::Fix {
                name: name.clone(),
                body: Box::new(under(name, body, bound)),
            },
            Term::MutualFix { defs, index } => {
                bound.extend(defs.iter().map(|(n, _)| n.clone()));
                let out = defs
                    .iter()
                    .map(|(n, b)| (n.clone(), go(b, names, bound)))
                    .collect();
                bound.truncate(bound.len() - defs.len());
                Term::MutualFix {
                    defs: out,
                    index: *index,
                }
            }
            Term::App(f, a) => {
                Term::App(Box::new(go(f, names, bound)), Box::new(go(a, names, bound)))
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => Term::Match {
                discriminant: Box::new(go(discriminant, names, bound)),
                motive: Box::new(go(motive, names, bound)),
                cases: cases.iter().map(|c| go(c, names, bound)).collect(),
            },
            Term::Annotated { annotation, term } => Term::Annotated {
                annotation: annotation.clone(),
                term: Box::new(go(term, names, bound)),
            },
            Term::Sort(_)
            | Term::Var(_)
            | Term::Global(_)
            | Term::Const { .. }
            | Term::Lit(_)
            | Term::Hole => t.clone(),
        }
    }
    go(term, names, &mut Vec::new())
}
