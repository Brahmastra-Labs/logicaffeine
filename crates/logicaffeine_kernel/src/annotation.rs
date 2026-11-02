//! Annotations: what the source said about a subterm that no judgment of the kernel reads.
//!
//! [`Term::Annotated`] wraps one subterm with one [`Annotation`]:
//!
//! * [`Annotation::Binder`] — how the argument of the wrapped `fun` or `Π` is supplied, Lean's
//!   `BinderInfo`, for a binder no declaration name records the kind of. `set_binder_kinds` keys a
//!   declaration's telescope on its NAME; an anonymous `fun {α} a => a` passed as an argument, a
//!   match case or a motive has no name, and this is where its kind lives.
//! * [`Annotation::Source`] — where the subterm came from ([`Provenance`]): a position in a source
//!   file, or one entry of a Lean `Expr.mdata` map, key and exported value.
//!
//! # Transparent
//!
//! An annotation changes no verdict and no result, and that is enforced at the representation
//! rather than hoped for at each use:
//!
//! * `Term`'s `==` is the kernel's identity, so it does not see annotations: a term and its
//!   annotated copy are EQUAL. [`Term::identical`] is the comparison that does see them.
//! * The kernel's judgments read kernel meaning only. Each checking door — `infer_type`,
//!   `is_subtype`, `def_eq`, the `normalize` family, `recheck`, `double_check` — erases the
//!   annotations of the terms it is handed ONCE, at the outermost call, before the depth door
//!   measures anything, and the [`Context`](crate::Context) stores every term erased: its locals,
//!   and every definition, declaration, inductive, constructor, instance, coercion and default it
//!   registers. Inside a kernel call no annotation exists; the arm every `match` on `Term` carries
//!   for this variant sees through it all the same, and a walk that only collects —
//!   `axiom_dependencies`, free variables, mentioned globals — reads straight through one.
//! * The term store interns the ERASED term, so an annotation never splits sharing: a term and its
//!   annotated copy intern to the same id.
//! * A certificate carries kernel meaning only: the variant has no serialized form, so a capture
//!   that forgot to erase fails loudly instead of widening the certificate language.
//!
//! What reads annotations is everything outside the kernel's judgments: the elaborator (a binder
//! kind on an anonymous `fun` inserts implicit arguments exactly as a declaration's recorded kinds
//! do) and the interface printer and parser (`@[…] t`, read back to the same annotated term).
//!
//! Every walk here runs on an explicit work stack: annotations sit on imported terms, which are
//! deep, and asking whether a term carries one must never be what exhausts the stack.

use crate::elaborate::ParamKind;
use crate::term::Term;
use std::fmt;

/// One piece of information about how a subterm was written. The elaborator and the
/// interface printer read it; every kernel judgment erases it first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Annotation {
    /// How the argument of the wrapped `fun` or `Π` is supplied — Lean's `BinderInfo`.
    Binder(ParamKind),
    /// Where the wrapped subterm came from.
    Source(Provenance),
}

/// Where a subterm came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance {
    /// A position in a source text.
    Position(SourcePosition),
    /// One entry of a Lean `Expr.mdata` map: its key, and the exporter's rendering of its
    /// `DataValue` (`Lean.DataValue.ofBool true`).
    LeanMetadata {
        /// The metadata key (`noImplicitLambda`, `_inaccessible`, `pp.analysis`, …).
        key: String,
        /// The value, as the exporter spells it.
        value: String,
    },
}

/// A position in a source text, in Lean's convention: the LINE is 1-based and the COLUMN is
/// 0-based and counted in Unicode scalar values. Held in its canonical spelling, the one Lean
/// prints a diagnostic's position in — `file:line:column` — so a position IS its text: two
/// positions are equal exactly when they print the same, and reading one back needs no rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePosition {
    text: String,
    file_len: usize,
    line: u32,
    column: u32,
}

impl SourcePosition {
    /// The position `file:line:column`.
    pub fn new(file: impl Into<String>, line: u32, column: u32) -> SourcePosition {
        let file = file.into();
        let file_len = file.len();
        SourcePosition {
            text: format!("{file}:{line}:{column}"),
            file_len,
            line,
            column,
        }
    }

    /// Read a position in its canonical spelling, `file:line:column`. The LAST two `:`-separated
    /// fields are the line and the column, so a file name may itself contain `:`; the file is not
    /// empty, the line is at least 1, and both numbers are written in canonical decimal (no sign,
    /// no leading zero) so that reading and printing are inverse.
    pub fn parse(text: &str) -> Option<SourcePosition> {
        let mut fields = text.rsplitn(3, ':');
        let column = canonical_number(fields.next()?)?;
        let line = canonical_number(fields.next()?)?;
        let file = fields.next()?;
        if file.is_empty() || line == 0 {
            return None;
        }
        Some(SourcePosition::new(file, line, column))
    }

    /// The file (or other source unit) the subterm was written in.
    pub fn file(&self) -> &str {
        &self.text[..self.file_len]
    }

    /// The 1-based line.
    pub fn line(&self) -> u32 {
        self.line
    }

    /// The 0-based column, in Unicode scalar values.
    pub fn column(&self) -> u32 {
        self.column
    }

    /// The canonical spelling, `file:line:column`.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

/// A `u32` in canonical decimal: digits only, no leading zero unless it is `0`.
fn canonical_number(digits: &str) -> Option<u32> {
    let canonical = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    canonical.then(|| digits.parse().ok()).flatten()
}

impl fmt::Display for SourcePosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Provenance::Position(position) => write!(f, "{position}"),
            Provenance::LeanMetadata { key, value } => write!(f, "{key} := {value}"),
        }
    }
}

impl fmt::Display for Annotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Annotation::Binder(kind) => write!(f, "binder {}", kind.lean_binder_info()),
            Annotation::Source(Provenance::Position(position)) => {
                write!(f, "source {:?}", position.as_str())
            }
            Annotation::Source(Provenance::LeanMetadata { key, value }) => {
                write!(f, "mdata {key:?} {value:?}")
            }
        }
    }
}

impl ParamKind {
    /// The name of Lean's `BinderInfo` constructor for this kind — one each, a bijection.
    pub fn lean_binder_info(self) -> &'static str {
        match self {
            ParamKind::Explicit => "default",
            ParamKind::Implicit => "implicit",
            ParamKind::StrictImplicit => "strictImplicit",
            ParamKind::Instance => "instImplicit",
        }
    }

    /// The kind a Lean `BinderInfo` constructor names, or `None` for a name Lean does not have.
    pub fn from_lean_binder_info(name: &str) -> Option<ParamKind> {
        match name {
            "default" => Some(ParamKind::Explicit),
            "implicit" => Some(ParamKind::Implicit),
            "strictImplicit" => Some(ParamKind::StrictImplicit),
            "instImplicit" => Some(ParamKind::Instance),
            _ => None,
        }
    }
}

/// Push `t`'s immediate subterms, the one child enumeration every walk in this file shares.
fn push_children<'t>(t: &'t Term, work: &mut Vec<&'t Term>) {
    match t {
        Term::Sort(_)
        | Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Lit(_)
        | Term::Hole => {}
        Term::Pi {
            param_type,
            body_type: body,
            ..
        }
        | Term::Lambda {
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
        Term::MutualFix { defs, .. } => work.extend(defs.iter().map(|(_, body)| body)),
        Term::Let {
            ty, value, body, ..
        } => {
            work.push(ty);
            work.push(value);
            work.push(body);
        }
        Term::Annotated { term, .. } => work.push(term),
    }
}

/// Replace every annotated node of `root` by the term it wraps, in place.
fn erase_in_place(root: &mut Term) {
    let mut work: Vec<&mut Term> = vec![root];
    while let Some(node) = work.pop() {
        if let Term::Annotated { term, .. } = node {
            let inner = std::mem::replace(term.as_mut(), Term::Hole);
            *node = inner;
            work.push(node);
            continue;
        }
        match node {
            Term::Sort(_)
            | Term::Var(_)
            | Term::Global(_)
            | Term::Const { .. }
            | Term::Lit(_)
            | Term::Hole => {}
            Term::Pi {
                param_type,
                body_type: body,
                ..
            }
            | Term::Lambda {
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
                work.extend(cases.iter_mut());
            }
            Term::Fix { body, .. } => work.push(body),
            Term::MutualFix { defs, .. } => work.extend(defs.iter_mut().map(|(_, body)| body)),
            Term::Let {
                ty, value, body, ..
            } => {
                work.push(ty);
                work.push(value);
                work.push(body);
            }
            Term::Annotated { term, .. } => work.push(term),
        }
    }
}

impl Term {
    /// `term`, carrying `annotation`.
    pub fn annotated(annotation: Annotation, term: Term) -> Term {
        Term::Annotated {
            annotation,
            term: Box::new(term),
        }
    }

    /// The term under every annotation at this term's head.
    pub fn peel_annotations(&self) -> &Term {
        let mut current = self;
        while let Term::Annotated { term, .. } = current {
            current = term;
        }
        current
    }

    /// The annotations at this term's head, outermost first.
    pub fn head_annotations(&self) -> impl Iterator<Item = &Annotation> {
        let mut current = self;
        std::iter::from_fn(move || match current {
            Term::Annotated { annotation, term } => {
                current = term;
                Some(annotation)
            }
            _ => None,
        })
    }

    /// The kind recorded on this binder: the outermost [`Annotation::Binder`] at the head of a
    /// `fun` or `Π`. `None` for a binder that records none, and for a term that is not a binder.
    pub fn binder_kind(&self) -> Option<ParamKind> {
        if !matches!(
            self.peel_annotations(),
            Term::Lambda { .. } | Term::Pi { .. }
        ) {
            return None;
        }
        self.head_annotations()
            .find_map(|annotation| match annotation {
                Annotation::Binder(kind) => Some(*kind),
                Annotation::Source(_) => None,
            })
    }

    /// Where this term came from: the outermost [`Annotation::Source`] at its head.
    pub fn provenance(&self) -> Option<&Provenance> {
        self.head_annotations()
            .find_map(|annotation| match annotation {
                Annotation::Source(provenance) => Some(provenance),
                Annotation::Binder(_) => None,
            })
    }

    /// Whether an annotation stands anywhere in this term.
    pub fn has_annotations(&self) -> bool {
        let mut work = vec![self];
        while let Some(t) = work.pop() {
            if matches!(t, Term::Annotated { .. }) {
                return true;
            }
            push_children(t, &mut work);
        }
        false
    }

    /// This term with every annotation erased — its kernel meaning, the only thing a kernel
    /// judgment reads. Erasing is idempotent, and `t.strip_annotations() == t` always holds.
    pub fn strip_annotations(&self) -> Term {
        self.clone().into_stripped()
    }

    /// [`strip_annotations`](Self::strip_annotations) on a term the caller owns: a term with no
    /// annotation is handed back as it is, without a copy.
    pub fn into_stripped(mut self) -> Term {
        if self.has_annotations() {
            erase_in_place(&mut self);
        }
        self
    }

    /// Structural equality that SEES annotations: same shape, same names, same annotations in
    /// the same places. `==` is the kernel's identity and ignores them.
    pub fn identical(&self, other: &Term) -> bool {
        let mut work = vec![(self, other)];
        while let Some(pair) = work.pop() {
            match pair {
                (
                    Term::Annotated {
                        annotation: a,
                        term: s,
                    },
                    Term::Annotated {
                        annotation: b,
                        term: t,
                    },
                ) => {
                    if a != b {
                        return false;
                    }
                    work.push((s, t));
                }
                (Term::Sort(a), Term::Sort(b)) if a == b => {}
                (Term::Var(a), Term::Var(b)) | (Term::Global(a), Term::Global(b)) if a == b => {}
                (Term::Const { name: m, levels: a }, Term::Const { name: n, levels: b })
                    if m == n && a == b => {}
                (
                    Term::Pi {
                        param: p,
                        param_type: s1,
                        body_type: s2,
                    },
                    Term::Pi {
                        param: q,
                        param_type: t1,
                        body_type: t2,
                    },
                )
                | (
                    Term::Lambda {
                        param: p,
                        param_type: s1,
                        body: s2,
                    },
                    Term::Lambda {
                        param: q,
                        param_type: t1,
                        body: t2,
                    },
                ) if p == q => {
                    work.push((s1, t1));
                    work.push((s2, t2));
                }
                (Term::App(f, a), Term::App(g, b)) => {
                    work.push((f, g));
                    work.push((a, b));
                }
                (
                    Term::Match {
                        discriminant: d,
                        motive: m,
                        cases: cs,
                    },
                    Term::Match {
                        discriminant: e,
                        motive: n,
                        cases: ds,
                    },
                ) if cs.len() == ds.len() => {
                    work.push((d, e));
                    work.push((m, n));
                    work.extend(cs.iter().zip(ds.iter()));
                }
                (Term::Fix { name: m, body: s }, Term::Fix { name: n, body: t }) if m == n => {
                    work.push((s, t));
                }
                (Term::MutualFix { defs: a, index: i }, Term::MutualFix { defs: b, index: j })
                    if i == j
                        && a.len() == b.len()
                        && a.iter().zip(b.iter()).all(|((m, _), (n, _))| m == n) =>
                {
                    work.extend(a.iter().zip(b.iter()).map(|((_, s), (_, t))| (s, t)));
                }
                (
                    Term::Let {
                        name: m,
                        ty: s1,
                        value: s2,
                        body: s3,
                    },
                    Term::Let {
                        name: n,
                        ty: t1,
                        value: t2,
                        body: t3,
                    },
                ) if m == n => {
                    work.push((s1, t1));
                    work.push((s2, t2));
                    work.push((s3, t3));
                }
                (Term::Lit(a), Term::Lit(b)) if a == b => {}
                (Term::Hole, Term::Hole) => {}
                _ => return false,
            }
        }
        true
    }
}

thread_local! {
    /// Whether a kernel door on this thread has already erased the terms of the call under way.
    static ERASED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Holds [`ERASED`] for the call a door opened, restoring it on the way out — including on a
/// panic, so the flag is never left stale.
struct ErasedScope(bool);

impl Drop for ErasedScope {
    fn drop(&mut self) {
        ERASED.with(|erased| erased.set(self.0));
    }
}

impl ErasedScope {
    fn open() -> ErasedScope {
        ErasedScope(ERASED.with(|erased| erased.replace(true)))
    }
}

/// Whether the call under way on this thread was entered through a kernel door, so every term it
/// reads — its input and everything the context holds — is annotation-free.
pub(crate) fn within_erased_call() -> bool {
    ERASED.with(|erased| erased.get())
}

/// THE DOOR of a kernel judgment over one term, in the shape of `term_store::deepen_stack`:
/// `Some(judge(erased))` at the OUTERMOST call, where the term is erased once (and not copied at
/// all when it carries no annotation), and `None` on the re-entrant call inside `judge`, which then
/// runs its own body. The scan is paid once per outermost call, before the depth door measures
/// anything — an annotation adds nesting to the representation and none to the term, so the kernel
/// must refuse exactly the terms it would refuse without it.
pub(crate) fn erased<R>(term: &Term, judge: impl FnOnce(&Term) -> R) -> Option<R> {
    if within_erased_call() {
        return None;
    }
    let _scope = ErasedScope::open();
    Some(if term.has_annotations() {
        judge(&term.strip_annotations())
    } else {
        judge(term)
    })
}

/// [`erased`] for a judgment over two terms.
pub(crate) fn erased_pair<R>(
    a: &Term,
    b: &Term,
    judge: impl FnOnce(&Term, &Term) -> R,
) -> Option<R> {
    if within_erased_call() {
        return None;
    }
    let _scope = ErasedScope::open();
    Some(match (a.has_annotations(), b.has_annotations()) {
        (false, false) => judge(a, b),
        (true, false) => judge(&a.strip_annotations(), b),
        (false, true) => judge(a, &b.strip_annotations()),
        (true, true) => judge(&a.strip_annotations(), &b.strip_annotations()),
    })
}

/// A term as the [`Context`](crate::Context) stores it: erased, so a lookup never hands an
/// annotation into a kernel call. A term stored from inside a kernel call is already erased, and
/// is not scanned again.
pub(crate) fn stored(term: Term) -> Term {
    if within_erased_call() {
        term
    } else {
        term.into_stripped()
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/annotation/tests.rs"]
mod tests;
