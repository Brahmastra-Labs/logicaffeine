//! LOGOS source for an imported Lean environment: one literate `.lg` document — prose headings,
//! literate English forms where they express a declaration exactly, kernel vernacular for the rest —
//! plus the correspondence between Lean and LOGOS names. The document is reparsed and rechecked in a
//! fresh context before anyone may publish it.
//!
//! # What is written
//!
//! The target's dependency CLOSURE, every declaration the kernel checked for it: inductives with
//! their constructors, definitions (the importer's sharing helpers and the recursors LOGOS derived
//! included), theorems with their checked proofs, and axioms. Nothing is written as an assumption
//! that the import checked as a proof, and nothing the import assumed is written as a proof.
//!
//! # How it is checked
//!
//! 1. While the document is built, every declaration is registered in a probe context the moment
//!    it is written and compared with the imported declaration, so a failure names the declaration.
//! 2. Before returning, the COMPLETE document is read by `Repl::execute_document` in a fresh
//!    context, the number of commands is counted, every declaration is compared with the imported
//!    one — kind, universe parameters, type, and body for a transparent definition, by strict
//!    α-equivalence — and the fresh context is searched for anything the document declared that
//!    the correspondence does not list.
//! 3. [`check_statement`] compares the target's reparsed type with an independent reference
//!    translation of the Lean type (competition rule R8).

use super::{declined, rejected, DiagnosticCode, ImportedContext, Result, Verdict};
use crate::interface::surface::{Name, Namespaces, Resolution};
use crate::interface::{print_command, Command, Repl};
use crate::{Context, Term};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[path = "emit_literate.rs"]
mod literate;
#[path = "emit_names.rs"]
mod names;
#[path = "emit_publish.rs"]
mod publish;
#[path = "emit_terms.rs"]
mod terms;

pub use names::LeanPart;
pub use publish::{publish, publish_through, Published, DOCUMENT_EXTENSION, FAILURE_EXTENSION};

/// Where a LOGOS declaration came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Written by the Lean author (or by Lean's elaborator, as an exported declaration).
    Authored,
    /// Introduced by the reader to preserve sharing inside the named authored declaration.
    Helper { of: String },
    /// Synthesized with no single Lean counterpart (for example, a derived recursor).
    Synthesized,
    /// The LOGOS prelude's own checked declaration of the same identity. It is not written in the
    /// document; the fresh context already holds it, and verification compares it all the same.
    Prelude,
}

/// What a declaration is, as the kernel holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeclarationKind {
    Inductive,
    Constructor,
    /// Transparent: its body unfolds.
    Definition,
    /// A checked proposition, Lean's `theorem`: its proof is written, checked by both kernels, and
    /// unfolds exactly as Lean's kernel unfolds a theorem.
    Theorem,
    /// Checked, then sealed (Lean's `opaque`): its value is written and checked, and never unfolds.
    Opaque,
    /// Assumed: no proof exists.
    Axiom,
    /// One of the kernel's four certified quotient primitives (Lean's `init_quot`).
    Quotient,
}

impl DeclarationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DeclarationKind::Inductive => "inductive",
            DeclarationKind::Constructor => "constructor",
            DeclarationKind::Definition => "definition",
            DeclarationKind::Theorem => "theorem",
            DeclarationKind::Opaque => "opaque",
            DeclarationKind::Axiom => "axiom",
            DeclarationKind::Quotient => "quotient",
        }
    }
}

/// Which surface carries a declaration in the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// A literate English form that reparses to the identical kernel declaration.
    Literate,
    /// The kernel vernacular (`Definition`, `Inductive`, `Theorem`, `Opaque`, `Axiom`).
    Vernacular,
    /// Not written: the prelude's own declaration.
    Prelude,
}

/// One Lean declaration and the LOGOS declaration that carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Correspondence {
    /// The Lean name as Lean writes it; for a reader-introduced helper, the importer's own name; empty
    /// for a constant the fresh LOGOS context derives itself (origin `Synthesized`), which Lean never
    /// had.
    pub lean: String,
    /// The key the importer registered the declaration under; empty for a derived constant.
    pub importer_key: String,
    /// The absolute LOGOS name.
    pub logos: String,
    pub kind: DeclarationKind,
    pub origin: Origin,
    pub form: Form,
    /// Universe parameters, in order: (importer's name, LOGOS name).
    pub levels: Vec<(String, String)>,
    /// The 1-based, inclusive line range of the LOGOS declaration in the document. `None` for a
    /// prelude identity, which is not written. The Lean side carries no span: an export records
    /// no source positions.
    pub lines: Option<(usize, usize)>,
}

/// An emitted document.
#[derive(Clone, Debug, PartialEq)]
pub struct EmittedDocument {
    /// The literate `.lg` source.
    pub source: String,
    /// The target's absolute LOGOS name.
    pub target: String,
    pub correspondence: Vec<Correspondence>,
    /// Declarations whose literate readable form was omitted (the vernacular form carries them),
    /// each with the reason; nothing is dropped silently.
    pub omissions: Vec<String>,
    /// The axioms the document assumes, by LOGOS name.
    pub assumptions: Vec<String>,
    /// What the import knew that the document does not carry, each as (category, detail) — read from
    /// the import's metadata (binder annotations), so it is NOT part of the document's bytes: a replay
    /// that rebuilds the environment from its certificate writes the identical source without it.
    pub losses: Vec<(String, String)>,
    /// What every declaration must reparse to, in LOGOS names.
    expected: Vec<Expected>,
}

/// Why an emitted document does not reparse to the imported declarations. Every variant names the
/// LOGOS declaration it concerns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmissionFault {
    /// The fresh context refused the document.
    Reparse { message: String },
    /// The document holds a different number of declaration commands than it should.
    CommandCount { expected: usize, found: usize },
    /// A declaration the document must declare is absent.
    Missing { logos: String },
    /// A declaration was registered as a different kind (a theorem that unfolds, an axiom with a
    /// proof, …).
    Kind {
        logos: String,
        expected: DeclarationKind,
    },
    /// A declaration's universe parameters differ.
    Levels {
        logos: String,
        expected: Vec<String>,
        found: Vec<String>,
    },
    /// A declaration's type differs.
    Type { logos: String },
    /// A transparent definition's body differs.
    Body { logos: String },
    /// An inductive's parameter count or constructors differ.
    Constructors { logos: String },
    /// The document declared something its correspondence does not list.
    Undeclared { logos: String },
    /// The target's reparsed statement differs from the reference translation of the Lean type.
    Statement { logos: String },
    /// A reference term names a constant the correspondence does not map.
    Unmapped { name: String },
    /// The independent re-checker did not confirm a reparsed definition at its declared type.
    Recheck { logos: String },
    /// An inductive gained, lost or changed its structure η (its projections).
    Structure { logos: String },
}

impl std::fmt::Display for EmissionFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmissionFault::Reparse { message } => {
                write!(f, "the fresh context refused the document: {message}")
            }
            EmissionFault::CommandCount { expected, found } => {
                write!(
                    f,
                    "the document holds {found} declaration commands where {expected} were written"
                )
            }
            EmissionFault::Missing { logos } => {
                write!(f, "`{logos}` is not declared after reparsing")
            }
            EmissionFault::Kind { logos, expected } => {
                write!(f, "`{logos}` did not reparse as a {}", expected.as_str())
            }
            EmissionFault::Levels {
                logos,
                expected,
                found,
            } => {
                write!(
                    f,
                    "`{logos}` reparsed with universe parameters {found:?}, not {expected:?}"
                )
            }
            EmissionFault::Type { logos } => write!(f, "`{logos}` reparsed with a different type"),
            EmissionFault::Body { logos } => write!(f, "`{logos}` reparsed with a different body"),
            EmissionFault::Constructors { logos } => {
                write!(
                    f,
                    "`{logos}` reparsed with different parameters or constructors"
                )
            }
            EmissionFault::Undeclared { logos } => {
                write!(
                    f,
                    "the document declares `{logos}`, which its correspondence does not list"
                )
            }
            EmissionFault::Statement { logos } => {
                write!(
                    f,
                    "`{logos}` does not state the reference translation of its Lean type"
                )
            }
            EmissionFault::Unmapped { name } => write!(f, "`{name}` has no LOGOS correspondent"),
            EmissionFault::Structure { logos } => {
                write!(
                    f,
                    "`{logos}` did not reparse with the imported structure η and projections"
                )
            }
            EmissionFault::Recheck { logos } => {
                write!(
                    f,
                    "the independent re-checker does not confirm `{logos}` at its declared type"
                )
            }
        }
    }
}

/// The largest document the emitter writes.
pub const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;

/// The dependency closure of `target`, as a LOGOS document.
///
/// A theorem's proof is carried only when the import retained it (`import_retaining_closure`
/// retains the whole closure); an opaque theorem in the closure whose proof was not retained is
/// refused, never written as an axiom.
pub fn emit_document(imported: &ImportedContext, target: &str) -> Result<EmittedDocument> {
    let proofs = |key: &str| imported.retained(key).map(|(_, body)| body.clone());
    emit_context(imported, &proofs, target)
}

/// [`emit_document`] over any checked context whose constants carry the importer's key scheme.
/// `proofs` hands back the checked proof of an opaque theorem, which a context does not keep.
pub fn emit_context(
    ctx: &Context,
    proofs: &(dyn Fn(&str) -> Option<Term> + Sync),
    target: &str,
) -> Result<EmittedDocument> {
    let closure = Closure::of(ctx, proofs, target)?;
    let depth = closure.depth();
    let rungs = depth
        .saturating_mul(3)
        .saturating_add(crate::term_store::INLINE_DEPTH);
    let stack = crate::term_store::stack_bytes_for_depth(rungs);
    if stack > 8 * 1024 * 1024 * 1024 {
        return Err(declined(
            DiagnosticCode::ResourceLimit,
            format!(
                "a term of depth {depth} is deeper than LOGOS source emission reserves stack for"
            ),
        ));
    }
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("lean-emit".into())
            .stack_size(stack)
            .spawn_scoped(scope, || Emitter::new(ctx, &closure)?.run())
            .map_err(|e| {
                declined(
                    DiagnosticCode::Infrastructure,
                    format!("cannot reserve emitter stack: {e}"),
                )
            })?
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p))
    })
}

/// Reparse `document.source` in a fresh context and compare every declaration with what it must
/// be. Returns the checked context.
pub fn verify(document: &EmittedDocument) -> std::result::Result<Repl, EmissionFault> {
    let depth = document
        .expected
        .iter()
        .flat_map(Expected::terms)
        .map(crate::term_store::term_depth)
        .max()
        .unwrap_or(0);
    let rungs = depth
        .saturating_mul(3)
        .saturating_add(crate::term_store::INLINE_DEPTH);
    let stack = crate::term_store::stack_bytes_for_depth(rungs);
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("lean-emit-verify".into())
            .stack_size(stack)
            .spawn_scoped(scope, || {
                verify_on_this_stack(document, true).map(|(repl, _)| repl)
            })
            .map_err(|e| EmissionFault::Reparse {
                message: format!("cannot reserve verifier stack: {e}"),
            })?
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p))
    })
}

/// Rule R8: the target's REPARSED statement against `reference`, an independent translation of the
/// Lean declaration (`reference::reference_declaration`) in the importer's names. The reference is
/// carried into LOGOS names through the correspondence alone — the emitter's own terms play no part —
/// and must name the target, bind the same universe parameters in the same order, and state the same
/// type up to α.
pub fn check_statement(
    document: &EmittedDocument,
    reference: &super::reference::ReferenceDeclaration,
) -> std::result::Result<(), EmissionFault> {
    let repl = verify(document)?;
    let logos = document.target.clone();
    let target = document
        .correspondence
        .iter()
        .find(|c| c.logos == logos)
        .ok_or_else(|| EmissionFault::Missing {
            logos: logos.clone(),
        })?;
    if reference.name != target.importer_key {
        return Err(EmissionFault::Statement { logos });
    }
    let levels: HashMap<String, String> = target.levels.iter().cloned().collect();
    let reference_levels = reference
        .level_params
        .iter()
        .map(|l| {
            levels
                .get(l)
                .cloned()
                .ok_or_else(|| EmissionFault::Unmapped { name: l.clone() })
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let globals: HashMap<&str, &str> = document
        .correspondence
        .iter()
        .map(|c| (c.importer_key.as_str(), c.logos.as_str()))
        .collect();
    let lookup = |k: &str| globals.get(k).map(|s| s.to_string());
    let renamed = terms::rename(&reference.ty, &lookup, &levels).map_err(|fault| match fault {
        terms::TermFault::UnmappedGlobal(name) | terms::TermFault::UnboundLevel(name) => {
            EmissionFault::Unmapped { name }
        }
        terms::TermFault::FreeVariable(_) | terms::TermFault::Hole => EmissionFault::Statement {
            logos: logos.clone(),
        },
    })?;
    let (found_levels, found) =
        header(repl.context(), &logos).ok_or_else(|| EmissionFault::Missing {
            logos: logos.clone(),
        })?;
    if found_levels != reference_levels {
        return Err(EmissionFault::Levels {
            logos,
            expected: reference_levels,
            found: found_levels,
        });
    }
    if terms::alpha_eq(&renamed, &found) {
        Ok(())
    } else {
        Err(EmissionFault::Statement { logos })
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The closure
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// One unit the document declares (or, for a prelude identity, compares).
#[derive(Clone, Debug)]
enum Decl {
    /// An inductive block the prelude already holds under the same identity (`Nat`).
    Prelude { members: Vec<String> },
    /// An inductive block, members in block order.
    Inductive { members: Vec<String> },
    Definition {
        key: String,
        levels: Vec<String>,
        ty: Term,
        body: Term,
    },
    /// A checked body with dependency evidence: a Lean `theorem` (`transparent`, it unfolds) or a Lean
    /// `opaque` (sealed; its value is only in the retained proofs).
    Theorem {
        key: String,
        levels: Vec<String>,
        ty: Term,
        proof: Term,
        transparent: bool,
    },
    Axiom {
        key: String,
        levels: Vec<String>,
        ty: Term,
    },
    /// A kernel quotient primitive; `equality` is the equality constant `Lift` states respect with.
    Quotient {
        key: String,
        kind: crate::quotient::Kind,
        levels: Vec<String>,
        ty: Term,
        equality: Option<String>,
    },
}

impl Decl {
    /// The key a unit is ordered and reported by.
    fn key(&self) -> &str {
        match self {
            Decl::Prelude { members } | Decl::Inductive { members } => &members[0],
            Decl::Definition { key, .. }
            | Decl::Theorem { key, .. }
            | Decl::Axiom { key, .. }
            | Decl::Quotient { key, .. } => key,
        }
    }
}

/// The keys the fresh context holds under the prelude's identity.
const PRELUDE_IDENTITIES: [&str; 3] = ["Nat", "Zero", "Succ"];

const HELPER_PREFIX: &str = "arena.internal.dag.";

struct Closure {
    decls: Vec<Decl>,
    /// Every key the closure declares → its unit.
    unit_of: BTreeMap<String, usize>,
    /// Per unit, the units it references.
    deps: Vec<BTreeSet<usize>>,
    /// The unit declaring the target.
    target: usize,
    /// The requested key itself (a constructor's, when a constructor was asked for).
    target_key: String,
}

impl Closure {
    fn of(
        ctx: &Context,
        proofs: &(dyn Fn(&str) -> Option<Term> + Sync),
        target: &str,
    ) -> Result<Closure> {
        let mut decls: Vec<Decl> = Vec::new();
        let mut unit_of: BTreeMap<String, usize> = BTreeMap::new();
        let mut refs: Vec<BTreeSet<String>> = Vec::new();
        let mut work: Vec<(String, Option<String>)> = vec![(target.to_string(), None)];
        while let Some((key, referrer)) = work.pop() {
            if unit_of.contains_key(&key) {
                continue;
            }
            let decl = classify(ctx, proofs, &key).map_err(|verdict| match referrer {
                Some(from) => verdict_with_prefix(verdict, format!("`{from}` refers to `{key}`")),
                None => verdict,
            })?;
            let index = decls.len();
            let mut mentioned = BTreeSet::new();
            let covered = unit_keys(ctx, &decl);
            match &decl {
                Decl::Prelude { .. } => {}
                Decl::Inductive { members } => {
                    for member in members {
                        if let Some(arity) = ctx.get_global(member) {
                            terms::references(arity, &mut mentioned);
                        }
                        for (_, ty) in ctx.get_constructors(member) {
                            terms::references(ty, &mut mentioned);
                        }
                        for projection in projections_of(ctx, member) {
                            if let Some((_, ty, body)) = definition_payload(ctx, &projection) {
                                terms::references(&ty, &mut mentioned);
                                terms::references(&body, &mut mentioned);
                            }
                        }
                    }
                }
                Decl::Definition { ty, body, .. } => {
                    terms::references(ty, &mut mentioned);
                    terms::references(body, &mut mentioned);
                }
                Decl::Theorem { ty, proof, .. } => {
                    terms::references(ty, &mut mentioned);
                    terms::references(proof, &mut mentioned);
                }
                Decl::Axiom { ty, .. } | Decl::Quotient { ty, .. } => {
                    terms::references(ty, &mut mentioned)
                }
            }
            for k in &covered {
                unit_of.insert(k.clone(), index);
            }
            for k in &mentioned {
                if !covered.contains(k) {
                    work.push((k.clone(), Some(display_key(&key))));
                }
            }
            mentioned.retain(|k| !covered.contains(k));
            refs.push(mentioned);
            decls.push(decl);
        }
        let deps = refs
            .iter()
            .map(|mentioned| mentioned.iter().map(|k| unit_of[k]).collect())
            .collect();
        Ok(Closure {
            target: unit_of[target],
            target_key: target.to_string(),
            decls,
            unit_of,
            deps,
        })
    }

    /// The deepest term the document will hold.
    fn depth(&self) -> usize {
        self.decls
            .iter()
            .flat_map(|d| match d {
                Decl::Definition { ty, body, .. } => vec![ty, body],
                Decl::Theorem { ty, proof, .. } => vec![ty, proof],
                Decl::Axiom { ty, .. } | Decl::Quotient { ty, .. } => vec![ty],
                Decl::Prelude { .. } | Decl::Inductive { .. } => vec![],
            })
            .map(crate::term_store::term_depth)
            .max()
            .unwrap_or(0)
    }
}

fn verdict_with_prefix(verdict: Verdict, prefix: String) -> Verdict {
    verdict.located(prefix)
}

/// A key as a reader recognizes it: the Lean name when there is one.
fn display_key(key: &str) -> String {
    names::lean_name(key).map_or_else(|| key.to_string(), |p| names::lean_display(&p))
}

/// Every key one unit declares.
fn unit_keys(ctx: &Context, decl: &Decl) -> BTreeSet<String> {
    match decl {
        Decl::Prelude { members } | Decl::Inductive { members } => {
            let mut keys: BTreeSet<String> = members.iter().cloned().collect();
            for member in members {
                keys.extend(
                    ctx.get_constructors(member)
                        .into_iter()
                        .map(|(c, _)| c.to_string()),
                );
                keys.extend(projections_of(ctx, member));
            }
            keys
        }
        other => BTreeSet::from([other.key().to_string()]),
    }
}

/// The projections of `inductive` when the import registered it as a structure (Lean's
/// `toCtorWhenStruct`: definitional η) — they belong to the inductive's unit, written with it and before
/// anything that relies on η.
fn projections_of(ctx: &Context, inductive: &str) -> Vec<String> {
    ctx.struct_info(inductive)
        .map(|info| info.projections.clone())
        .unwrap_or_default()
}

/// What the context holds under `key`.
fn classify(
    ctx: &Context,
    proofs: &(dyn Fn(&str) -> Option<Term> + Sync),
    key: &str,
) -> Result<Decl> {
    if let Some(kind) = quotient_kind(key) {
        let (levels, ty) = header(ctx, key).ok_or_else(|| {
            rejected(
                DiagnosticCode::MissingDeclaration,
                format!(
                    "the quotient primitive `{}` is not installed",
                    display_key(key)
                ),
            )
        })?;
        // `Lift`'s respect hypothesis is stated with an equality, the one constant its signature names
        // besides the quotient itself.
        let equality = if kind == crate::quotient::Kind::Lift {
            let mut names = BTreeSet::new();
            terms::references(&ty, &mut names);
            match names.into_iter().find(|n| quotient_kind(n).is_none()) {
                Some(eq) => Some(eq),
                None => {
                    return Err(rejected(
                        DiagnosticCode::MalformedExport,
                        "the quotient lift names no equality",
                    ))
                }
            }
        } else {
            None
        };
        return Ok(Decl::Quotient {
            key: key.to_string(),
            kind,
            levels,
            ty,
            equality,
        });
    }
    if PRELUDE_IDENTITIES.contains(&key) {
        return Ok(Decl::Prelude {
            members: vec!["Nat".to_string()],
        });
    }
    if let Some(inductive) = ctx.constructor_inductive(key) {
        return classify(ctx, proofs, &inductive.to_string());
    }
    if ctx.is_inductive(key) {
        let members = ctx
            .mutual_block_of(key)
            .map(<[String]>::to_vec)
            .unwrap_or_else(|| vec![key.to_string()]);
        return Ok(Decl::Inductive { members });
    }
    if ctx.checked_opaque_dependencies(key).is_some() {
        if let Some((levels, ty, proof)) = definition_payload(ctx, key) {
            return Ok(Decl::Theorem {
                key: key.to_string(),
                levels,
                ty,
                proof,
                transparent: true,
            });
        }
    }
    if let Some((levels, ty, body)) = ctx.get_universe_poly(key) {
        return Ok(Decl::Definition {
            key: key.to_string(),
            levels: levels.clone(),
            ty: ty.clone(),
            body: body.clone(),
        });
    }
    if let (Some(ty), Some(body)) = (ctx.get_definition_type(key), ctx.get_definition_body(key)) {
        return Ok(Decl::Definition {
            key: key.to_string(),
            levels: Vec::new(),
            ty: ty.clone(),
            body: body.clone(),
        });
    }
    if let Some((levels, ty)) = header(ctx, key) {
        if ctx.checked_opaque_dependencies(key).is_some() {
            let proof = proofs(key).ok_or_else(|| {
                declined(
                    DiagnosticCode::UnsupportedFeature,
                    format!(
                        "the checked value of the opaque declaration `{}` was not retained by the \
                         import, and a checked declaration is never written as an assumption",
                        display_key(key)
                    ),
                )
            })?;
            return Ok(Decl::Theorem {
                key: key.to_string(),
                levels,
                ty,
                proof,
                transparent: false,
            });
        }
        return Ok(Decl::Axiom {
            key: key.to_string(),
            levels,
            ty,
        });
    }
    if let Some((_, blocker)) = ctx
        .pending_obligations()
        .into_iter()
        .find(|(pending, _)| *pending == key)
    {
        return Err(rejected(
            DiagnosticCode::MissingDeclaration,
            format!(
                "`{}` waits for `{}`, which is not declared: an unresolved forward obligation, not a checked declaration",
                display_key(key),
                display_key(blocker)
            ),
        ));
    }
    Err(rejected(
        DiagnosticCode::MissingDeclaration,
        format!(
            "`{}` is not declared in the imported environment",
            display_key(key)
        ),
    ))
}

/// The quotient primitive a kernel key names.
fn quotient_kind(key: &str) -> Option<crate::quotient::Kind> {
    use crate::quotient::Kind;
    [Kind::Type, Kind::Constructor, Kind::Lift, Kind::Induction]
        .into_iter()
        .find(|k| k.name() == key)
}

/// A declaration's level telescope and type, however it was registered.
fn header(ctx: &Context, key: &str) -> Option<(Vec<String>, Term)> {
    if let Some((levels, ty)) = ctx.poly_declaration(key) {
        return Some((levels.to_vec(), ty.clone()));
    }
    ctx.get_global(key).map(|ty| (Vec::new(), ty.clone()))
}

/// A transparent definition's level telescope, type and body, however it was registered.
fn definition_payload(ctx: &Context, key: &str) -> Option<(Vec<String>, Term, Term)> {
    if let Some((levels, ty, body)) = ctx.get_universe_poly(key) {
        return Some((levels.clone(), ty.clone(), body.clone()));
    }
    Some((
        Vec::new(),
        ctx.get_definition_type(key)?.clone(),
        ctx.get_definition_body(key)?.clone(),
    ))
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// What each declaration must reparse to
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
struct ExpectedInductive {
    logos: String,
    levels: Vec<String>,
    arity: Term,
    params: usize,
    constructors: Vec<(String, Term)>,
    /// For a structure (η), its projections: LOGOS name, type, body — in field order.
    structure: Option<Vec<(String, Term, Term)>>,
}

#[derive(Clone, Debug, PartialEq)]
enum Expected {
    Inductive {
        members: Vec<ExpectedInductive>,
        written: bool,
    },
    Definition {
        logos: String,
        levels: Vec<String>,
        ty: Term,
        body: Term,
    },
    /// `body` is the proof a transparent theorem unfolds to; `None` for a sealed `Opaque`.
    Theorem {
        logos: String,
        levels: Vec<String>,
        ty: Term,
        body: Option<Term>,
    },
    Axiom {
        logos: String,
        levels: Vec<String>,
        ty: Term,
    },
    Quotient {
        logos: String,
        levels: Vec<String>,
        ty: Term,
    },
}

impl Expected {
    fn terms(&self) -> Vec<&Term> {
        match self {
            Expected::Inductive { members, .. } => members
                .iter()
                .flat_map(|m| {
                    std::iter::once(&m.arity)
                        .chain(m.constructors.iter().map(|(_, t)| t))
                        .chain(
                            m.structure
                                .iter()
                                .flatten()
                                .flat_map(|(_, ty, body)| [ty, body]),
                        )
                })
                .collect(),
            Expected::Definition { ty, body, .. } => vec![ty, body],
            Expected::Theorem {
                ty,
                body: Some(body),
                ..
            } => vec![ty, body],
            Expected::Theorem { ty, .. }
            | Expected::Axiom { ty, .. }
            | Expected::Quotient { ty, .. } => vec![ty],
        }
    }

    /// How many declaration commands the document spends on this unit.
    fn commands(&self) -> usize {
        match self {
            // The block, then for each structure its projection definitions and its `Structure`.
            Expected::Inductive {
                written: true,
                members,
            } => {
                1 + members
                    .iter()
                    .filter_map(|m| m.structure.as_ref())
                    .map(|p| p.len() + 1)
                    .sum::<usize>()
            }
            Expected::Inductive { written: false, .. } => 0,
            _ => 1,
        }
    }

    /// Every LOGOS key this unit declares.
    fn logos_keys(&self) -> Vec<&str> {
        match self {
            Expected::Inductive { members, .. } => members
                .iter()
                .flat_map(|m| {
                    std::iter::once(m.logos.as_str())
                        .chain(m.constructors.iter().map(|(c, _)| c.as_str()))
                        .chain(m.structure.iter().flatten().map(|(p, _, _)| p.as_str()))
                })
                .collect(),
            Expected::Definition { logos, .. }
            | Expected::Theorem { logos, .. }
            | Expected::Axiom { logos, .. }
            | Expected::Quotient { logos, .. } => {
                vec![logos]
            }
        }
    }
}

/// Is what `ctx` holds exactly what `expected` says?
fn compare(ctx: &Context, expected: &Expected) -> std::result::Result<(), EmissionFault> {
    let levels_match = |logos: &str, want: &[String], got: &[String]| {
        if want == got {
            Ok(())
        } else {
            Err(EmissionFault::Levels {
                logos: logos.to_string(),
                expected: want.to_vec(),
                found: got.to_vec(),
            })
        }
    };
    match expected {
        Expected::Definition {
            logos,
            levels,
            ty,
            body,
        } => {
            let Some((got_levels, got_ty, got_body)) = definition_payload(ctx, logos) else {
                return Err(if header(ctx, logos).is_some() {
                    EmissionFault::Kind {
                        logos: logos.clone(),
                        expected: DeclarationKind::Definition,
                    }
                } else {
                    EmissionFault::Missing {
                        logos: logos.clone(),
                    }
                });
            };
            if ctx.checked_opaque_dependencies(logos).is_some() {
                return Err(EmissionFault::Kind {
                    logos: logos.clone(),
                    expected: DeclarationKind::Definition,
                });
            }
            levels_match(logos, levels, &got_levels)?;
            if !terms::alpha_eq(ty, &got_ty) {
                return Err(EmissionFault::Type {
                    logos: logos.clone(),
                });
            }
            if !terms::alpha_eq(body, &got_body) {
                return Err(EmissionFault::Body {
                    logos: logos.clone(),
                });
            }
            Ok(())
        }
        Expected::Quotient { logos, levels, ty } => {
            let Some((got_levels, got_ty)) = header(ctx, logos) else {
                return Err(EmissionFault::Missing {
                    logos: logos.clone(),
                });
            };
            let primitive =
                ctx.quotient_primitives.get(logos).map(|c| c.kind) == quotient_kind(logos);
            if !primitive || definition_payload(ctx, logos).is_some() {
                return Err(EmissionFault::Kind {
                    logos: logos.clone(),
                    expected: DeclarationKind::Quotient,
                });
            }
            levels_match(logos, levels, &got_levels)?;
            if !terms::alpha_eq(ty, &got_ty) {
                return Err(EmissionFault::Type {
                    logos: logos.clone(),
                });
            }
            Ok(())
        }
        Expected::Theorem {
            logos, levels, ty, ..
        }
        | Expected::Axiom { logos, levels, ty } => {
            // A theorem is checked AND unfolds; an `Opaque` is checked and sealed; an axiom is neither.
            let (checked_expected, body, kind) = match expected {
                Expected::Theorem {
                    body: Some(body), ..
                } => (true, Some(body), DeclarationKind::Theorem),
                Expected::Theorem { body: None, .. } => (true, None, DeclarationKind::Opaque),
                _ => (false, None, DeclarationKind::Axiom),
            };
            let Some((got_levels, got_ty)) = header(ctx, logos) else {
                return Err(EmissionFault::Missing {
                    logos: logos.clone(),
                });
            };
            let payload = definition_payload(ctx, logos);
            let checked = ctx.checked_opaque_dependencies(logos).is_some();
            if payload.is_some() != body.is_some()
                || ctx.is_inductive(logos)
                || ctx.is_constructor(logos)
                || checked != checked_expected
            {
                return Err(EmissionFault::Kind {
                    logos: logos.clone(),
                    expected: kind,
                });
            }
            levels_match(logos, levels, &got_levels)?;
            if !terms::alpha_eq(ty, &got_ty) {
                return Err(EmissionFault::Type {
                    logos: logos.clone(),
                });
            }
            if let (Some(want), Some((_, _, got))) = (body, payload) {
                if !terms::alpha_eq(want, &got) {
                    return Err(EmissionFault::Body {
                        logos: logos.clone(),
                    });
                }
            }
            Ok(())
        }
        Expected::Inductive { members, .. } => {
            for member in members {
                let logos = &member.logos;
                if !ctx.is_inductive(logos) {
                    return Err(if header(ctx, logos).is_some() {
                        EmissionFault::Kind {
                            logos: logos.clone(),
                            expected: DeclarationKind::Inductive,
                        }
                    } else {
                        EmissionFault::Missing {
                            logos: logos.clone(),
                        }
                    });
                }
                let (got_levels, arity) =
                    header(ctx, logos).ok_or_else(|| EmissionFault::Missing {
                        logos: logos.clone(),
                    })?;
                let got_levels = if ctx.is_universe_poly_inductive(logos) {
                    got_levels
                } else {
                    Vec::new()
                };
                levels_match(logos, &member.levels, &got_levels)?;
                if !terms::alpha_eq(&member.arity, &arity) {
                    return Err(EmissionFault::Type {
                        logos: logos.clone(),
                    });
                }
                let ctors = ctx.get_constructors(logos);
                let same = ctx.inductive_num_params(logos) == member.params
                    && ctors.len() == member.constructors.len()
                    && ctors.iter().zip(&member.constructors).all(
                        |((name, ty), (want, want_ty))| {
                            name == want && terms::alpha_eq(want_ty, ty)
                        },
                    );
                if !same {
                    return Err(EmissionFault::Constructors {
                        logos: logos.clone(),
                    });
                }
                // η is registered exactly when the import registered it, with the same projections.
                match (&member.structure, ctx.struct_info(logos)) {
                    (None, None) => {}
                    (Some(want), Some(info)) => {
                        let names: Vec<&str> = want.iter().map(|(p, _, _)| p.as_str()).collect();
                        if info
                            .projections
                            .iter()
                            .map(String::as_str)
                            .ne(names.iter().copied())
                        {
                            return Err(EmissionFault::Structure {
                                logos: logos.clone(),
                            });
                        }
                        for (projection, ty, body) in want {
                            let Some((got_levels, got_ty, got_body)) =
                                definition_payload(ctx, projection)
                            else {
                                return Err(EmissionFault::Missing {
                                    logos: projection.clone(),
                                });
                            };
                            levels_match(projection, &member.levels, &got_levels)?;
                            if !terms::alpha_eq(ty, &got_ty) {
                                return Err(EmissionFault::Type {
                                    logos: projection.clone(),
                                });
                            }
                            if !terms::alpha_eq(body, &got_body) {
                                return Err(EmissionFault::Body {
                                    logos: projection.clone(),
                                });
                            }
                        }
                    }
                    _ => {
                        return Err(EmissionFault::Structure {
                            logos: logos.clone(),
                        })
                    }
                }
            }
            Ok(())
        }
    }
}

/// Where a registration differs from its expectation, for a diagnostic a reader can act on.
fn mismatch_detail(ctx: &Context, expected: &Expected) -> String {
    let Expected::Inductive { members, .. } = expected else {
        return String::new();
    };
    let mut out = String::new();
    for m in members {
        let found_params = ctx.inductive_num_params(&m.logos);
        if found_params != m.params {
            out.push_str(&format!(
                " (`{}` has {found_params} parameters, not {})",
                m.logos, m.params
            ));
        }
        if let Some((_, arity)) = header(ctx, &m.logos) {
            if !terms::alpha_eq(&m.arity, &arity) {
                out.push_str(&format!(" (`{}`: arity {arity}, not {})", m.logos, m.arity));
            }
        }
        let found = ctx.get_constructors(&m.logos);
        if found.len() != m.constructors.len() {
            out.push_str(&format!(
                " (`{}` has {} constructors, not {})",
                m.logos,
                found.len(),
                m.constructors.len()
            ));
        }
        for ((name, ty), (want, want_ty)) in found.iter().zip(&m.constructors) {
            if name != want {
                out.push_str(&format!(
                    " (constructor `{name}` where `{want}` was written)"
                ));
            } else if !terms::alpha_eq(want_ty, ty) {
                out.push_str(&format!(" (`{name}`: {ty}, not {want_ty})"));
            }
        }
    }
    out
}

/// Reparse and compare. Returns the checked context and every constant the fresh context DERIVED
/// while reading the document (an inductive's recursor family, a definition's equation lemmas), each
/// with the LOGOS declaration that produced it. With `listed`, the derived constants must be exactly
/// the correspondence's derived entries.
fn verify_on_this_stack(
    document: &EmittedDocument,
    listed_derivations: bool,
) -> std::result::Result<(Repl, BTreeMap<String, String>), EmissionFault> {
    let mut repl = Repl::new();
    let prelude = repl.context().clone();
    repl.execute_document(&document.source)
        .map_err(|e| EmissionFault::Reparse {
            message: e.to_string(),
        })?;

    let expected_commands: usize = document.expected.iter().map(Expected::commands).sum();
    let found_commands = count_commands(&document.source)?;
    if found_commands != expected_commands {
        return Err(EmissionFault::CommandCount {
            expected: expected_commands,
            found: found_commands,
        });
    }
    for expected in &document.expected {
        compare(repl.context(), expected)?;
    }
    let ctx = repl.context();

    // The second kernel: every transparent definition's body is inferred again by the independent
    // re-checker and must have exactly the declared type.
    for expected in &document.expected {
        if let Expected::Definition {
            logos, ty, body, ..
        } = expected
        {
            let rechecked = crate::recheck(ctx, body).map_err(|_| EmissionFault::Recheck {
                logos: logos.clone(),
            })?;
            if !crate::defeq_for_test(ctx, &rechecked, ty) {
                return Err(EmissionFault::Recheck {
                    logos: logos.clone(),
                });
            }
        }
    }

    let listed: HashSet<&str> = document
        .expected
        .iter()
        .flat_map(Expected::logos_keys)
        .collect();
    let mut declared: BTreeSet<String> = BTreeSet::new();
    declared.extend(ctx.iter_declarations().map(|(k, _)| k.to_string()));
    declared.extend(ctx.iter_definitions().map(|(k, _, _)| k.to_string()));
    for (k, _) in ctx.iter_inductives() {
        declared.insert(k.to_string());
        declared.extend(
            ctx.get_constructors(k)
                .into_iter()
                .map(|(c, _)| c.to_string()),
        );
    }
    for key in listed.iter() {
        declared.remove(*key);
    }
    // What the fresh context derives on its own for an inductive the document declares — the
    // recursor family, no-confusion, injectivity, `below`/`brecOn`, decidable equality, and the
    // equation lemmas of a definition — is LOGOS's own checked derivation, not a declaration the
    // document smuggled in. Such a constant is TRANSPARENT (it has a body); a smuggled inductive,
    // constructor or axiom under a derived name has none, and the command count already refuses a
    // declaration written beside the listed ones.
    let family = derived_family(&document.expected);
    let mut derived: BTreeMap<String, String> = BTreeMap::new();
    for key in declared {
        let in_prelude = prelude.get_global(&key).is_some()
            || prelude.poly_declaration(&key).is_some()
            || prelude.is_inductive(&key);
        if in_prelude {
            continue;
        }
        let owner = family
            .get(&key)
            .cloned()
            .or_else(|| equation_head(&key, &listed, &family));
        match owner {
            Some(owner) if definition_payload(ctx, &key).is_some() => {
                derived.insert(key, owner);
            }
            _ => return Err(EmissionFault::Undeclared { logos: key }),
        }
    }
    if listed_derivations {
        let recorded: BTreeSet<&str> = document
            .correspondence
            .iter()
            .filter(|c| c.importer_key.is_empty())
            .map(|c| c.logos.as_str())
            .collect();
        if let Some(key) = derived.keys().find(|k| !recorded.contains(k.as_str())) {
            return Err(EmissionFault::Undeclared { logos: key.clone() });
        }
        if let Some(key) = recorded.iter().find(|k| !derived.contains_key(**k)) {
            return Err(EmissionFault::Missing {
                logos: key.to_string(),
            });
        }
    }
    Ok((repl, derived))
}

/// The names the fresh context derives for the inductives a document writes — the families
/// `recursor::derive_auxiliaries` produces at declaration time — each with its inductive.
fn derived_family(expected: &[Expected]) -> HashMap<String, String> {
    let mut family = HashMap::new();
    for unit in expected {
        if let Expected::Inductive {
            members,
            written: true,
        } = unit
        {
            for member in members {
                for name in aux_family(
                    &member.logos,
                    member.constructors.iter().map(|(c, _)| c.as_str()),
                ) {
                    family.insert(name, member.logos.clone());
                }
            }
        }
    }
    family
}

/// The declaration an equation lemma was generated for, when it is a listed definition or a derived
/// constant.
fn equation_head(
    key: &str,
    listed: &HashSet<&str>,
    family: &HashMap<String, String>,
) -> Option<String> {
    if !crate::equations::is_equation_name(key) {
        return None;
    }
    let head = match key.strip_suffix(crate::equations::UNFOLD_SUFFIX) {
        Some(head) => head,
        None => &key[..key.rfind(crate::equations::EQUATION_INFIX)?],
    };
    (listed.contains(head) || family.contains_key(head)).then(|| head.to_string())
}

/// The constants `recursor::derive_auxiliaries` names for the inductive `inductive` with the
/// constructors `ctors`: the recursor family, no-confusion, `below`/`brecOn`, decidable equality, and
/// one injectivity lemma per constructor.
fn aux_family<'n>(inductive: &str, ctors: impl Iterator<Item = &'n str>) -> Vec<String> {
    let mut names: Vec<String> = [
        "_rec",
        "_recOn",
        "_casesOn",
        "_noConfusionType",
        "_noConfusion",
        "_below",
        "_brecOn",
    ]
    .iter()
    .map(|suffix| format!("{inductive}{suffix}"))
    .collect();
    names.push(format!("decEq{inductive}"));
    names.extend(ctors.map(|c| format!("{c}_inj")));
    names
}

/// An equation lemma the fresh context generated for a listed definition.
fn is_equation_of_listed(key: &str, listed: &HashSet<&str>) -> bool {
    if !crate::equations::is_equation_name(key) {
        return false;
    }
    let head = match key.strip_suffix(crate::equations::UNFOLD_SUFFIX) {
        Some(head) => head,
        None => &key[..key.rfind(crate::equations::EQUATION_INFIX).unwrap_or(0)],
    };
    listed.contains(head)
}

/// The number of declaration commands in `source`, read through the text-only door — the prose
/// blocks excluded.
fn count_commands(source: &str) -> std::result::Result<usize, EmissionFault> {
    let surface = crate::interface::Surface::new();
    let mut offset = 0;
    let mut count = 0;
    while !source[offset..].trim().is_empty() {
        let (command, consumed) =
            crate::interface::parse_command_prefix_in(&surface, &source[offset..]).map_err(
                |e| EmissionFault::Reparse {
                    message: e.to_string(),
                },
            )?;
        if consumed == 0 {
            return Err(EmissionFault::Reparse {
                message: "the document parser made no progress".into(),
            });
        }
        if !matches!(command, Command::Comment) {
            count += 1;
        }
        offset += consumed;
    }
    Ok(count)
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The emitter
// ─────────────────────────────────────────────────────────────────────────────────────────────

fn unsupported(message: impl Into<String>) -> Verdict {
    declined(DiagnosticCode::UnsupportedFeature, message)
}

/// Section order: assumptions, types, definitions, theorems; the target last of all.
fn rank(decl: &Decl) -> u8 {
    match decl {
        Decl::Axiom { .. } => 0,
        Decl::Quotient { .. } => 1,
        Decl::Prelude { .. } | Decl::Inductive { .. } => 1,
        Decl::Definition { .. } => 2,
        Decl::Theorem { .. } => 3,
    }
}

const SECTIONS: [&str; 5] = [
    "Assumptions",
    "Types",
    "Definitions",
    "Theorems",
    "The target",
];

struct Emitter<'a> {
    ctx: &'a Context,
    closure: &'a Closure,
    /// Importer key → absolute LOGOS key.
    logos: BTreeMap<String, String>,
    /// Per unit, the universe parameter renaming.
    levels: Vec<Vec<(String, String)>>,
    /// Per unit, the helper's owner (by Lean name) for a reader-introduced helper.
    owners: BTreeMap<usize, String>,
    relocated_under: Option<String>,
    /// Every component of every name the document writes — no binder may take one.
    avoid: HashSet<String>,
    /// Every name the LOGOS prelude declares — no binder may take one either, or it would read as
    /// that constant.
    prelude_names: HashSet<String>,
    prelude: Context,
    /// Every Lean binder name the document could not keep: (declaration, Lean name, written, reason),
    /// in the order the document writes them.
    renames: std::cell::RefCell<Vec<(String, terms::Rename)>>,
    /// Whether a hygienic (inaccessible) Lean binder was written under its user-facing name.
    hygienic: std::cell::Cell<bool>,
}

impl<'a> Emitter<'a> {
    fn new(ctx: &'a Context, closure: &'a Closure) -> Result<Emitter<'a>> {
        let prelude = crate::interface::standard_context();
        let mut taken = names::Taken {
            roots: HashSet::new(),
            keys: HashSet::new(),
        };
        {
            let mut add = |k: &str| {
                taken
                    .roots
                    .insert(k.split('.').next().unwrap_or(k).to_string());
                taken.keys.insert(k.to_string());
            };
            prelude.iter_declarations().for_each(|(k, _)| add(k));
            prelude.iter_definitions().for_each(|(k, _, _)| add(k));
            for (k, _) in prelude.iter_inductives() {
                add(k);
                for (c, _) in prelude.get_constructors(k) {
                    add(c);
                }
            }
        }

        // A structure's projections are the reader's, named `<Inductive>.proj_<i>` once the inductive
        // has its LOGOS name.
        let mut projection_owner: BTreeMap<String, (String, usize)> = BTreeMap::new();
        for decl in &closure.decls {
            if let Decl::Inductive { members } = decl {
                for member in members {
                    for (i, projection) in projections_of(ctx, member).into_iter().enumerate() {
                        projection_owner.insert(projection, (member.clone(), i + 1));
                    }
                }
            }
        }
        let mut lean_named: BTreeSet<String> = BTreeSet::new();
        let mut helpers: Vec<String> = Vec::new();
        let mut fixed: BTreeMap<String, String> = BTreeMap::new();
        for key in closure.unit_of.keys() {
            if PRELUDE_IDENTITIES.contains(&key.as_str()) || quotient_kind(key).is_some() {
                fixed.insert(key.clone(), key.clone());
            } else if key.starts_with(HELPER_PREFIX) {
                helpers.push(key.clone());
            } else if projection_owner.contains_key(key) {
                continue;
            } else if names::lean_name(key).is_some() {
                lean_named.insert(key.clone());
            } else {
                return Err(unsupported(format!(
                    "the kernel constant `{key}` has neither a Lean name nor a known reader origin"
                )));
            }
        }

        let definitions: BTreeSet<&str> = closure
            .decls
            .iter()
            .filter_map(|d| match d {
                Decl::Definition { key, .. } => Some(key.as_str()),
                _ => None,
            })
            .collect();
        let inductive_members: Vec<(&str, Vec<String>)> = closure
            .decls
            .iter()
            .filter_map(|d| match d {
                Decl::Inductive { members } => Some(members),
                _ => None,
            })
            .flatten()
            .map(|m| {
                (
                    m.as_str(),
                    ctx.get_constructors(m)
                        .into_iter()
                        .map(|(c, _)| c.to_string())
                        .collect(),
                )
            })
            .collect();
        let reserved = |allocation: &BTreeMap<String, String>| -> BTreeSet<String> {
            let mut derived = BTreeSet::new();
            for (member, ctors) in &inductive_members {
                let Some(l) = allocation.get(*member) else {
                    continue;
                };
                derived.extend(aux_family(
                    l,
                    ctors
                        .iter()
                        .filter_map(|c| allocation.get(c))
                        .map(String::as_str),
                ));
            }
            let written_definitions: HashSet<&str> = definitions
                .iter()
                .filter_map(|k| allocation.get(*k))
                .map(String::as_str)
                .collect();
            for logos in allocation.values() {
                if is_equation_of_listed(logos, &written_definitions) {
                    derived.insert(logos.clone());
                }
            }
            derived
        };
        let allocation = names::allocate(&lean_named, &fixed, &taken, &reserved)
            .map_err(|e| unsupported(format!("LOGOS name allocation: {e}")))?;
        let mut logos = allocation.logos;

        for (projection, (owner, i)) in &projection_owner {
            let mut name = format!("{}.proj_{i}", logos[owner]);
            while logos.values().any(|v| *v == name) || taken.keys.contains(&name) {
                name.push('\'');
            }
            logos.insert(projection.clone(), name);
        }

        // A helper is named inside the declaration whose sharing it preserves.
        let mut owners: BTreeMap<usize, String> = BTreeMap::new();
        let mut referrers: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); closure.decls.len()];
        for (from, deps) in closure.deps.iter().enumerate() {
            for &to in deps {
                referrers[to].insert(from);
            }
        }
        helpers.sort_by_key(|h| h[HELPER_PREFIX.len()..].parse::<u64>().unwrap_or(u64::MAX));
        let mut per_owner: BTreeMap<String, usize> = BTreeMap::new();
        for helper in &helpers {
            let unit = closure.unit_of[helper];
            let owner_unit = owner_of(unit, &referrers, closure).ok_or_else(|| {
                unsupported(format!(
                    "the reader's helper `{helper}` is not used by any declaration of the closure"
                ))
            })?;
            let owner_key = closure.decls[owner_unit].key().to_string();
            let owner_logos = logos[&owner_key].clone();
            let count = per_owner.entry(owner_logos.clone()).or_insert(0);
            *count += 1;
            let mut name = format!("{owner_logos}.shared_{count}");
            while logos.values().any(|v| *v == name) || taken.keys.contains(&name) {
                name.push('\'');
            }
            logos.insert(helper.clone(), name);
            owners.insert(unit, display_key(&owner_key));
        }

        for (key, name) in &logos {
            if fixed.contains_key(key) {
                continue;
            }
            let clashes = prelude.get_global(name).is_some()
                || prelude.poly_declaration(name).is_some()
                || prelude.get_universe_poly(name).is_some();
            let spellable =
                crate::interface::declaration_name(name).is_ok_and(|n| n.key() == *name);
            if clashes || !spellable {
                return Err(unsupported(format!(
                    "the LOGOS name `{name}` allocated for `{}` is not a free, spellable name",
                    display_key(key)
                )));
            }
        }

        let mut avoid: HashSet<String> = HashSet::new();
        for name in logos.values() {
            avoid.extend(name.split('.').map(str::to_string));
        }
        avoid.extend(allocation.relocated_under.iter().cloned());

        let levels = closure
            .decls
            .iter()
            .map(|d| match d {
                Decl::Definition { levels, .. }
                | Decl::Theorem { levels, .. }
                | Decl::Axiom { levels, .. }
                | Decl::Quotient { levels, .. } => level_names(levels),
                Decl::Inductive { members } | Decl::Prelude { members } => level_names(
                    ctx.poly_declaration(&members[0])
                        .filter(|_| ctx.is_universe_poly_inductive(&members[0]))
                        .map(|(l, _)| l)
                        .unwrap_or(&[]),
                ),
            })
            .collect();

        let prelude_names: HashSet<String> =
            taken.roots.iter().chain(&taken.keys).cloned().collect();

        Ok(Emitter {
            ctx,
            closure,
            logos,
            levels,
            owners,
            relocated_under: allocation.relocated_under,
            avoid,
            prelude_names,
            prelude,
            renames: std::cell::RefCell::new(Vec::new()),
            hygienic: std::cell::Cell::new(false),
        })
    }

    fn rename(&self, unit: usize, term: &Term) -> Result<Term> {
        let levels: HashMap<String, String> = self.levels[unit].iter().cloned().collect();
        let globals = |k: &str| self.logos.get(k).cloned();
        terms::rename(term, &globals, &levels).map_err(|fault| {
            unsupported(format!(
                "`{}`: {fault}",
                display_key(self.closure.decls[unit].key())
            ))
        })
    }

    fn level_list(&self, unit: usize) -> Vec<String> {
        self.levels[unit].iter().map(|(_, l)| l.clone()).collect()
    }

    /// What unit `unit` must reparse to.
    fn expected(&self, unit: usize) -> Result<Expected> {
        Ok(match &self.closure.decls[unit] {
            Decl::Definition { key, ty, body, .. } => Expected::Definition {
                logos: self.logos[key].clone(),
                levels: self.level_list(unit),
                ty: self.rename(unit, ty)?,
                body: self.rename(unit, body)?,
            },
            Decl::Theorem {
                key,
                ty,
                proof,
                transparent,
                ..
            } => Expected::Theorem {
                logos: self.logos[key].clone(),
                levels: self.level_list(unit),
                ty: self.rename(unit, ty)?,
                body: if *transparent {
                    Some(self.rename(unit, proof)?)
                } else {
                    None
                },
            },
            Decl::Quotient { key, ty, .. } => Expected::Quotient {
                logos: self.logos[key].clone(),
                levels: self.level_list(unit),
                ty: self.rename(unit, ty)?,
            },
            Decl::Axiom { key, ty, .. } => Expected::Axiom {
                logos: self.logos[key].clone(),
                levels: self.level_list(unit),
                ty: self.rename(unit, ty)?,
            },
            Decl::Inductive { members } | Decl::Prelude { members } => {
                let mut out = Vec::with_capacity(members.len());
                for member in members {
                    let arity = self.ctx.get_global(member).ok_or_else(|| {
                        unsupported(format!(
                            "the inductive `{}` has no arity",
                            display_key(member)
                        ))
                    })?;
                    out.push(ExpectedInductive {
                        logos: self.logos[member].clone(),
                        levels: self.level_list(unit),
                        arity: self.rename(unit, arity)?,
                        params: self.ctx.inductive_num_params(member),
                        constructors: self
                            .ctx
                            .get_constructors(member)
                            .into_iter()
                            .map(|(c, ty)| Ok((self.logos[c].clone(), self.rename(unit, ty)?)))
                            .collect::<Result<_>>()?,
                        structure: match self.ctx.struct_info(member) {
                            None => None,
                            Some(info) => Some(
                                info.projections
                                    .iter()
                                    .map(|p| {
                                        let (_, ty, body) = definition_payload(self.ctx, p).ok_or_else(|| {
                                            unsupported(format!("the projection `{p}` has no checked definition"))
                                        })?;
                                        Ok((self.logos[p].clone(), self.rename(unit, &ty)?, self.rename(unit, &body)?))
                                    })
                                    .collect::<Result<_>>()?,
                            ),
                        },
                    });
                }
                Expected::Inductive {
                    members: out,
                    written: matches!(self.closure.decls[unit], Decl::Inductive { .. }),
                }
            }
        })
    }

    /// The order the document declares its units in: every unit after the units it references,
    /// grouped by section as far as that allows, the target last.
    fn order(&self) -> Vec<usize> {
        let n = self.closure.decls.len();
        let mut waiting: Vec<usize> = self.closure.deps.iter().map(BTreeSet::len).collect();
        let mut referrers: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (from, deps) in self.closure.deps.iter().enumerate() {
            for &to in deps {
                referrers[to].push(from);
            }
        }
        let priority = |u: usize| {
            let r = if u == self.closure.target {
                4
            } else {
                rank(&self.closure.decls[u])
            };
            (r, self.logos[self.closure.decls[u].key()].clone(), u)
        };
        let mut ready: BTreeSet<(u8, String, usize)> =
            (0..n).filter(|&u| waiting[u] == 0).map(priority).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(next) = ready.pop_first() {
            let u = next.2;
            order.push(u);
            for &r in &referrers[u] {
                waiting[r] -= 1;
                if waiting[r] == 0 {
                    ready.insert(priority(r));
                }
            }
        }
        order
    }

    fn run(self) -> Result<EmittedDocument> {
        let order = self.order();
        if order.len() != self.closure.decls.len() {
            return Err(unsupported(
                "the imported declarations reference each other in a cycle",
            ));
        }

        let mut expected = Vec::with_capacity(order.len());
        let mut blocks: Vec<(usize, String, Form)> = Vec::new();
        let mut omissions = Vec::new();
        let mut probe = Repl::new();

        for &unit in &order {
            let want = self.expected(unit)?;
            if let Decl::Prelude { .. } = self.closure.decls[unit] {
                compare(&self.prelude, &want).map_err(|fault| {
                    unsupported(format!(
                        "the imported `Nat` is not the LOGOS prelude's `Nat`: {fault}"
                    ))
                })?;
                expected.push(want);
                continue;
            }
            let written = self.written(unit, &want, probe.context())?;
            let (text, form) = match self.literate(&written) {
                Ok(candidate) => {
                    let mut scratch = Repl::new();
                    *scratch.context_mut() = probe.context().clone();
                    let exact = scratch.execute_document(&candidate).is_ok()
                        && compare(scratch.context(), &want).is_ok();
                    if exact {
                        (candidate, Form::Literate)
                    } else {
                        omissions.push(format!(
                            "{}: the literate form did not reparse to the identical declaration",
                            written.logos_name()
                        ));
                        (self.vernacular(&written)?, Form::Vernacular)
                    }
                }
                Err(why) => {
                    omissions.push(format!("{}: {why}", written.logos_name()));
                    (self.vernacular(&written)?, Form::Vernacular)
                }
            };
            probe.execute_document(&text).map_err(|e| {
                unsupported(format!(
                    "LOGOS refused the emitted declaration of `{}`: {e}",
                    display_key(self.closure.decls[unit].key())
                ))
            })?;
            compare(probe.context(), &want).map_err(|fault| {
                unsupported(format!(
                    "the emitted declaration of `{}` did not reparse to the imported one: {fault}{}\n{text}",
                    display_key(self.closure.decls[unit].key()),
                    mismatch_detail(probe.context(), &want),
                ))
            })?;
            expected.push(want);
            blocks.push((unit, text, form));
        }

        let (source, lines) = self.assemble(&blocks)?;
        if source.len() > MAX_SOURCE_BYTES {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                format!(
                    "the emitted document is {} bytes, over the {MAX_SOURCE_BYTES}-byte limit",
                    source.len()
                ),
            ));
        }
        let forms: BTreeMap<usize, Form> = blocks.iter().map(|(u, _, f)| (*u, *f)).collect();
        let correspondence = self.correspondence(&order, &forms, &lines);
        let assumptions = order
            .iter()
            .filter_map(|&u| match &self.closure.decls[u] {
                Decl::Axiom { key, .. } => Some(self.logos[key].clone()),
                _ => None,
            })
            .collect();
        let document = EmittedDocument {
            source,
            target: self.logos[&self.closure.target_key].clone(),
            correspondence,
            omissions,
            assumptions,
            losses: self.losses(),
            expected,
        };
        let (_, derived) = verify_on_this_stack(&document, false).map_err(|fault| {
            unsupported(format!(
                "the emitted document did not survive its fresh reparse: {fault}"
            ))
        })?;
        // Every constant the fresh context derived while reading the document is listed too, with the
        // written declaration that produced it: nothing the document introduces goes unaccounted.
        let mut document = document;
        for (logos, owner) in derived {
            let producer = document
                .correspondence
                .iter()
                .find(|c| c.logos == owner)
                .cloned();
            document.correspondence.push(Correspondence {
                lean: String::new(),
                importer_key: String::new(),
                logos,
                kind: DeclarationKind::Definition,
                origin: Origin::Synthesized,
                form: producer.as_ref().map_or(Form::Vernacular, |p| p.form),
                levels: Vec::new(),
                lines: producer.and_then(|p| p.lines),
            });
        }
        Ok(document)
    }

    /// A unit as the document spells it: binders fresh, references written relative to the
    /// namespace the declaration elaborates in.
    fn written(&self, unit: usize, want: &Expected, probe: &Context) -> Result<Written> {
        // Exactly what the reading `Repl` knows when it resolves a name in this declaration: the keys the
        // command itself declares, and every constant already in its context, universe-polymorphic ones
        // (a table `get_global` does not read) included.
        let pending: Vec<String> = want.logos_keys().iter().map(|s| s.to_string()).collect();
        let known = |k: &str| {
            pending.iter().any(|p| p == k)
                || probe.get_global(k).is_some()
                || probe.poly_declaration(k).is_some()
        };
        let declared = self.logos[self.closure.decls[unit].key()].clone();
        let record = |renames: Vec<terms::Rename>| {
            self.renames
                .borrow_mut()
                .extend(renames.into_iter().map(|r| (declared.clone(), r)));
        };
        let unavailable = terms::Unavailable {
            constants: &self.avoid,
            prelude: &self.prelude_names,
        };
        // `named`: how many leading Π binders are written with a name even when nothing refers to them.
        let fresh_naming = |t: &Term, named: usize| {
            let mut renames = Vec::new();
            let mut hygienic = false;
            let out = terms::freshen_naming(t, named, &unavailable, &mut renames, &mut hygienic)
                .map_err(|fault| {
                    unsupported(format!(
                        "`{}`: {fault}",
                        display_key(self.closure.decls[unit].key())
                    ))
                })?;
            record(renames);
            self.hygienic.set(self.hygienic.get() || hygienic);
            Ok::<Term, Verdict>(out)
        };
        let fresh = |t: &Term| fresh_naming(t, 0);
        let spell_in = |declared: &str, t: &Term| -> Result<Term> {
            let scope = declaration_scope(declared);
            let mut failure = None;
            let out = terms::respell(t, &mut |key| match written_reference(&scope, &known, key) {
                Some(w) => w,
                None => {
                    failure = Some(key.to_string());
                    key.to_string()
                }
            });
            match failure {
                Some(key) => Err(unsupported(format!(
                    "no written name reaches `{key}` from `{declared}`"
                ))),
                None => Ok(out),
            }
        };
        Ok(match want {
            Expected::Definition {
                logos,
                levels,
                ty,
                body,
            } => Written::Definition {
                name: logos.clone(),
                levels: levels.clone(),
                ty: spell_in(logos, &fresh(ty)?)?,
                body: spell_in(logos, &fresh(body)?)?,
            },
            Expected::Quotient { logos, levels, .. } => {
                let Decl::Quotient { kind, equality, .. } = &self.closure.decls[unit] else {
                    unreachable!("a quotient's expectation comes from a quotient")
                };
                let equality = match equality {
                    Some(eq) => match spell_in(logos, &Term::Global(self.logos[eq].clone()))? {
                        Term::Global(written) => Some(written),
                        _ => unreachable!("a global respells to a global"),
                    },
                    None => None,
                };
                Written::Quotient {
                    name: logos.clone(),
                    kind: *kind,
                    levels: levels.clone(),
                    equality,
                }
            }
            Expected::Axiom { logos, levels, ty } => Written::Axiom {
                name: logos.clone(),
                levels: levels.clone(),
                ty: spell_in(logos, &fresh(ty)?)?,
            },
            Expected::Theorem {
                logos,
                levels,
                ty,
                body,
            } => {
                let Decl::Theorem { proof, .. } = &self.closure.decls[unit] else {
                    unreachable!("a theorem's expectation comes from a theorem")
                };
                Written::Theorem {
                    name: logos.clone(),
                    transparent: body.is_some(),
                    levels: levels.clone(),
                    ty: spell_in(logos, &fresh(ty)?)?,
                    proof: spell_in(logos, &fresh(&self.rename(unit, proof)?)?)?,
                }
            }
            Expected::Inductive { members, .. } => {
                let mut out = Vec::with_capacity(members.len());
                for m in members {
                    // The parameters are named: every constructor refers to them by name.
                    let arity = fresh_naming(&m.arity, m.params)?;
                    let (params, sort) = terms::split_pis(&arity, m.params).ok_or_else(|| {
                        unsupported(format!(
                            "the arity of `{}` has fewer binders than parameters",
                            m.logos
                        ))
                    })?;
                    let param_names: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
                    let enclosing: Vec<&str> = {
                        let parts: Vec<&str> = m.logos.split('.').collect();
                        parts[..parts.len() - 1].to_vec()
                    };
                    // A constructor whose own parameter binders do not state the member's parameter types
                    // exactly (Lean's `HAdd.mk` keeps `outParam (Type w)` where the arity says `Type w`)
                    // is written with its FULL telescope; then every constructor of the member is.
                    let mut split = Vec::with_capacity(m.constructors.len());
                    for (ctor, ty) in &m.constructors {
                        let mut renames = Vec::new();
                        let mut hygienic = false;
                        let ty = terms::freshen_with_prefix(
                            ty,
                            &param_names,
                            &unavailable,
                            &mut renames,
                            &mut hygienic,
                        )
                        .map_err(|fault| unsupported(format!("`{ctor}`: {fault}")))?;
                        record(renames);
                        self.hygienic.set(self.hygienic.get() || hygienic);
                        let (restated, rest) =
                            terms::split_pis(&ty, m.params).ok_or_else(|| {
                                unsupported(format!(
                                    "the constructor `{ctor}` binds fewer parameters than its type"
                                ))
                            })?;
                        split.push((ctor, ty, restated, rest));
                    }
                    let full = split.iter().any(|(_, _, restated, _)| {
                        restated
                            .iter()
                            .zip(&params)
                            .any(|((_, t), (_, p))| !terms::alpha_eq(t, p))
                    });
                    let mut constructors = Vec::with_capacity(split.len());
                    for (ctor, ty, _, rest) in split {
                        let written = if full { ty } else { rest };
                        constructors
                            .push((relative_to(ctor, &enclosing), spell_in(&m.logos, &written)?));
                    }
                    let params = params
                        .into_iter()
                        .map(|(n, t)| Ok((n, spell_in(&m.logos, &t)?)))
                        .collect::<Result<Vec<_>>>()?;
                    let own = m
                        .logos
                        .split('.')
                        .next_back()
                        .unwrap_or(&m.logos)
                        .to_string();
                    let written_self =
                        written_reference(&declaration_scope(&m.logos), &known, &m.logos)
                            .unwrap_or_else(|| m.logos.clone());
                    let structure = match &m.structure {
                        None => None,
                        Some(projections) => Some(
                            projections
                                .iter()
                                .map(|(p, ty, body)| {
                                    Ok((
                                        p.clone(),
                                        spell_in(p, &fresh(ty)?)?,
                                        spell_in(p, &fresh(body)?)?,
                                    ))
                                })
                                .collect::<Result<Vec<_>>>()?,
                        ),
                    };
                    out.push(WrittenInductive {
                        name: m.logos.clone(),
                        levels: m.levels.clone(),
                        params,
                        sort: spell_in(&m.logos, &sort)?,
                        constructors,
                        full,
                        structure,
                        result_heads: vec![m.logos.clone(), written_self, own],
                    });
                }
                Written::Inductive(out)
            }
        })
    }

    /// The vernacular command(s) for a written unit.
    fn vernacular(&self, written: &Written) -> Result<String> {
        let (levels, command) = match written {
            Written::Definition {
                name,
                levels,
                ty,
                body,
            } => (
                levels,
                Command::Definition {
                    name: name.clone(),
                    ty: Some(ty.clone()),
                    body: body.clone(),
                    is_hint: false,
                    implicit_count: 0,
                    proof_goal: None,
                },
            ),
            Written::Theorem {
                name,
                transparent: true,
                levels,
                ty,
                proof,
            } => (
                levels,
                Command::Theorem {
                    name: name.clone(),
                    ty: ty.clone(),
                    proof: proof.clone(),
                },
            ),
            Written::Theorem {
                name,
                transparent: false,
                levels,
                ty,
                proof,
            } => (
                levels,
                Command::Opaque {
                    name: name.clone(),
                    ty: ty.clone(),
                    value: proof.clone(),
                },
            ),
            Written::Axiom { name, levels, ty } => (
                levels,
                Command::Axiom {
                    name: name.clone(),
                    ty: ty.clone(),
                },
            ),
            Written::Quotient {
                kind,
                levels,
                equality,
                ..
            } => (
                levels,
                Command::Quotient {
                    kind: *kind,
                    equality: equality.clone(),
                },
            ),
            // A parameter is referred to by its NAME, exactly as the parser hands a written one
            // over: the block door binds it again when it puts the parameter prefix back.
            Written::Inductive(members) => (
                &members[0].levels,
                Command::InductiveBlock {
                    members: members
                        .iter()
                        .map(|m| {
                            let names: Vec<String> =
                                m.params.iter().map(|(n, _)| n.clone()).collect();
                            crate::interface::command::InductiveMember {
                                name: m.name.clone(),
                                params: m
                                    .params
                                    .iter()
                                    .enumerate()
                                    .map(|(i, (n, t))| {
                                        (n.clone(), terms::name_parameters(t, &names[..i]))
                                    })
                                    .collect(),
                                sort: terms::name_parameters(&m.sort, &names),
                                constructors: m
                                    .constructors
                                    .iter()
                                    .map(|(c, t)| {
                                        let t = if m.full {
                                            t.clone()
                                        } else {
                                            terms::name_parameters(t, &names)
                                        };
                                        (c.clone(), t)
                                    })
                                    .collect(),
                                full_constructors: m.full,
                            }
                        })
                        .collect(),
                },
            ),
        };
        let wrap = |command: Command| {
            if levels.is_empty() {
                command
            } else {
                Command::Universes {
                    params: levels.clone(),
                    inner: Box::new(command),
                }
            }
        };
        let mut commands = vec![wrap(command)];
        // A structure's projections and its η follow its block at once, before anything that relies
        // on η (its recursor's body checks only modulo η).
        if let Written::Inductive(members) = written {
            for m in members {
                let Some(projections) = &m.structure else {
                    continue;
                };
                for (p, ty, body) in projections {
                    commands.push(wrap(Command::Definition {
                        name: p.clone(),
                        ty: Some(ty.clone()),
                        body: body.clone(),
                        is_hint: false,
                        implicit_count: 0,
                        proof_goal: None,
                    }));
                }
                commands.push(Command::Structure {
                    inductive: m.name.clone(),
                    projections: projections.iter().map(|(p, _, _)| p.clone()).collect(),
                });
            }
        }
        let printed: Vec<String> = commands
            .iter()
            .map(print_command)
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| unsupported(format!("`{}`: {e}", written.logos_name())))?;
        Ok(printed.join("\n\n"))
    }

    /// The literate form of a written unit, or why there is none.
    fn literate(&self, written: &Written) -> std::result::Result<String, String> {
        match written {
            Written::Inductive(members) if members.iter().any(|m| m.structure.is_some()) => Err(
                "a structure's projections and η are declared by the vernacular `Structure` command, which \
                 the literate surface has no form for"
                    .into(),
            ),
            Written::Inductive(members) if members.iter().any(|m| m.full) => Err(
                "a constructor restates its parameters with types the inductive's arity does not use, \
                 which only the vernacular's full constructor telescope can write"
                    .into(),
            ),
            Written::Inductive(members) if members.len() == 1 && members[0].levels.is_empty() => {
                let m = &members[0];
                literate::inductive(&literate::WrittenInductive {
                    name: &m.name,
                    params: &m.params,
                    sort: &m.sort,
                    constructors: &m.constructors,
                    result_heads: &m.result_heads,
                })
            }
            Written::Inductive(members) if members.len() > 1 => {
                Err("a mutual block has no literate form".into())
            }
            Written::Inductive(_) => Err("the literate surface has no universe parameters".into()),
            Written::Definition { name, levels, ty, body } if levels.is_empty() => {
                literate::definition(name, ty, body)
            }
            Written::Definition { .. } => Err("the literate surface has no universe parameters".into()),
            Written::Theorem { .. } => Err(
                "the literate `## Theorem:` surface proves by tactic, and this proof is a checked term".into(),
            ),
            Written::Axiom { .. } => Err("the literate surface has no form for an assumption".into()),
            Written::Quotient { .. } => Err("the literate surface has no form for a quotient primitive".into()),
        }
    }

    fn describe(&self, unit: usize) -> String {
        let decl = &self.closure.decls[unit];
        let lean = display_key(decl.key());
        match decl {
            Decl::Axiom { .. } => format!("Axiom `{lean}`: assumed, not proved."),
            Decl::Quotient { .. } => format!(
                "Quotient primitive `{lean}`: the kernel's certified quotient, installed and checked against \
                 Lean's canonical signature."
            ),
            Decl::Inductive { members } if members.len() > 1 => format!(
                "Mutual inductive block {}.",
                members.iter().map(|m| format!("`{}`", display_key(m))).collect::<Vec<_>>().join(", ")
            ),
            Decl::Inductive { members } if members.iter().any(|m| self.ctx.struct_info(m).is_some()) => format!(
                "Inductive `{lean}`, a structure: its projections and its η follow it, before anything that \
                 relies on η."
            ),
            Decl::Inductive { .. } => format!("Inductive `{lean}`."),
            Decl::Theorem { transparent: true, .. } => format!(
                "Theorem `{lean}`: its checked proof is written; like Lean's kernel, LOGOS unfolds it."
            ),
            Decl::Theorem { transparent: false, .. } => {
                format!("Opaque constant `{lean}`: its checked value is written and never unfolds.")
            }
            Decl::Definition { key, .. } if key.starts_with(HELPER_PREFIX) => format!(
                "Shared subterm `{key}` of `{}`, introduced by the Lean reader to keep its sharing; a checked definition.",
                self.owners.get(&unit).cloned().unwrap_or_default()
            ),
            Decl::Definition { key, .. } if is_derived_recursor(self.ctx, key) => {
                format!("Recursor `{lean}`, derived by LOGOS from its inductive and checked.")
            }
            Decl::Definition { .. } => format!("Definition `{lean}`."),
            Decl::Prelude { .. } => format!("`{lean}` is the LOGOS prelude's own."),
        }
    }

    /// The document: a header, then every block under its section heading, each with a prose line
    /// naming the Lean declaration it carries. Returns the source and each unit's line range.
    fn assemble(
        &self,
        blocks: &[(usize, String, Form)],
    ) -> Result<(String, BTreeMap<usize, (usize, usize)>)> {
        let mut out = String::new();
        out.push_str(&self.header());
        let mut section = None;
        let mut lines = BTreeMap::new();
        for (unit, text, _) in blocks {
            let s = if *unit == self.closure.target {
                4
            } else {
                rank(&self.closure.decls[*unit]) as usize
            };
            if section != Some(s) {
                out.push_str(&format!("\n/-! ## {} -/\n", SECTIONS[s]));
                section = Some(s);
            }
            out.push_str(&format!("\n/-- {} -/\n", prose(&self.describe(*unit))));
            let first = out.lines().count() + 1;
            out.push_str(text);
            out.push('\n');
            let last = out.lines().count();
            lines.insert(*unit, (first, last));
        }
        Ok((out, lines))
    }

    /// The declarations whose binder annotations the import recorded and the document does not
    /// write, by LOGOS name.
    fn annotated(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .closure
            .unit_of
            .keys()
            .filter(|key| {
                self.ctx.binder_kinds(key).is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|k| *k != crate::elaborate::ParamKind::Explicit)
                })
            })
            .filter_map(|key| self.logos.get(key).cloned())
            .collect();
        out.sort();
        out
    }

    fn losses(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .annotated()
            .into_iter()
            .map(|name| {
                (
                    "binder_annotations".to_string(),
                    format!("`{name}`: implicit, strict-implicit and instance binders are written as explicit ones"),
                )
            })
            .collect();
        out.extend(
            self.rename_lines()
                .into_iter()
                .map(|line| ("binder_renamed".to_string(), line)),
        );
        out
    }

    /// One line per Lean binder name the document could not keep, each distinct rename once.
    fn rename_lines(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        self.renames
            .borrow()
            .iter()
            .filter(|entry| seen.insert((*entry).clone()))
            .map(|(declared, (lean, written, reason))| {
                format!(
                    "`{declared}`: the Lean binder `{lean}` is written `{written}` because {}",
                    reason.as_str()
                )
            })
            .collect()
    }

    fn header(&self) -> String {
        let target = self.closure.target_key.as_str();
        let lean = display_key(target);
        let written = self
            .closure
            .decls
            .iter()
            .filter(|d| !matches!(d, Decl::Prelude { .. }))
            .count();
        let axioms: Vec<String> = self
            .closure
            .decls
            .iter()
            .filter_map(|d| match d {
                Decl::Axiom { key, .. } => Some(format!(
                    "`{}` (Lean `{}`)",
                    self.logos[key],
                    display_key(key)
                )),
                _ => None,
            })
            .collect();
        let mut text = String::new();
        text.push_str(&format!(
            "# LOGOS source for the Lean declaration `{lean}`\n\n"
        ));
        text.push_str(&format!(
            "The declaration `{}` and the {} declarations it depends on, written in LOGOS from a checked \
             import. Where a literate English form states a declaration exactly it is written that way; \
             every other declaration is written in the kernel vernacular. Before this document was \
             published it was read back in a fresh LOGOS context and every declaration was compared with \
             the imported one.\n\n",
            self.logos[target],
            written.saturating_sub(1)
        ));
        text.push_str("## Assumptions\n\n");
        if axioms.is_empty() {
            text.push_str("Nothing below is assumed: every declaration is a definition, a type or a checked proof.\n\n");
        } else {
            text.push_str("These axioms are assumed, each written below as an `Axiom`:\n\n");
            for a in &axioms {
                text.push_str(&format!("- {a}\n"));
            }
            text.push('\n');
        }
        text.push_str("## Names\n\n");
        match &self.relocated_under {
            Some(root) => text.push_str(&format!(
                "Lean names are kept. A Lean root namespace the LOGOS prelude already uses is placed, with \
                 everything inside it, under `{root}`; Lean `Nat` is the prelude's own `Nat`, `Zero` and \
                 `Succ`. The correspondence index lists every pair.\n\n"
            )),
            None => text.push_str(
                "Lean names are kept; Lean `Nat` is the prelude's own `Nat`, `Zero` and `Succ`. The \
                 correspondence index lists every pair.\n\n",
            ),
        }
        let renamed = self.rename_lines();
        if self.hygienic.get() {
            text.push_str(
                "A hygienic Lean binder, which Lean shows as inaccessible (`a✝`), is written under its own \
                 name (`a`), as Lean's `eraseMacroScopes` reads it.\n\n",
            );
        }
        if renamed.is_empty() {
            text.push_str("Every binder the text names keeps its Lean name.\n\n");
        } else {
            text.push_str(
                "Every binder the text names keeps its Lean name, except where that name would read as \
                 something else — a constant this document writes or the LOGOS prelude declares, an enclosing \
                 binder, or syntax; then the first free `name1`, `name2`, … is written:\n\n",
            );
            for line in &renamed {
                text.push_str(&format!("- {line}\n"));
            }
            text.push('\n');
        }
        // The text depends on the checked declarations alone, so a replay that rebuilds the environment
        // from a certificate writes the same bytes; which declarations had annotated binders is import
        // metadata, reported in `EmittedDocument::losses` rather than here.
        text.push_str("## Not carried\n\n");
        text.push_str(
            "Binder annotations (implicit, strict-implicit and instance binders) are not written: every \
             application below is fully explicit, exactly as the kernel checked it.\n\n",
        );
        text.push_str(
            "A binder nothing refers to is written as an arrow (`A -> B`) and carries no name, as Lean \
             itself displays it.\n",
        );
        format!("/-!\n{}-/\n", prose(&text))
    }

    fn correspondence(
        &self,
        order: &[usize],
        forms: &BTreeMap<usize, Form>,
        lines: &BTreeMap<usize, (usize, usize)>,
    ) -> Vec<Correspondence> {
        let mut out = Vec::new();
        for &unit in order {
            let decl = &self.closure.decls[unit];
            let form = forms.get(&unit).copied().unwrap_or(Form::Prelude);
            let span = lines.get(&unit).copied();
            let levels = self.levels[unit].clone();
            let entry = |key: &str, kind: DeclarationKind, origin: Origin| Correspondence {
                lean: display_key(key),
                importer_key: key.to_string(),
                logos: self.logos[key].clone(),
                kind,
                origin,
                form,
                levels: levels.clone(),
                lines: span,
            };
            match decl {
                Decl::Prelude { members } | Decl::Inductive { members } => {
                    let origin = if matches!(decl, Decl::Prelude { .. }) {
                        Origin::Prelude
                    } else {
                        Origin::Authored
                    };
                    for member in members {
                        out.push(entry(member, DeclarationKind::Inductive, origin.clone()));
                        for (ctor, _) in self.ctx.get_constructors(member) {
                            out.push(entry(ctor, DeclarationKind::Constructor, origin.clone()));
                        }
                        for projection in projections_of(self.ctx, member) {
                            out.push(entry(
                                &projection,
                                DeclarationKind::Definition,
                                Origin::Synthesized,
                            ));
                        }
                    }
                }
                Decl::Definition { key, .. } => {
                    let origin = if key.starts_with(HELPER_PREFIX) {
                        Origin::Helper {
                            of: self.owners.get(&unit).cloned().unwrap_or_default(),
                        }
                    } else if is_derived_recursor(self.ctx, key) {
                        Origin::Synthesized
                    } else {
                        Origin::Authored
                    };
                    let mut e = entry(key, DeclarationKind::Definition, origin);
                    if key.starts_with(HELPER_PREFIX) {
                        e.lean = key.clone();
                    }
                    out.push(e);
                }
                Decl::Theorem {
                    key, transparent, ..
                } => {
                    let kind = if *transparent {
                        DeclarationKind::Theorem
                    } else {
                        DeclarationKind::Opaque
                    };
                    out.push(entry(key, kind, Origin::Authored))
                }
                Decl::Axiom { key, .. } => {
                    out.push(entry(key, DeclarationKind::Axiom, Origin::Authored))
                }
                Decl::Quotient { key, .. } => {
                    out.push(entry(key, DeclarationKind::Quotient, Origin::Authored))
                }
            }
        }
        out
    }
}

/// A Lean recursor: the importer registers `I.rec` from the kernel's own derivation, never from the
/// export's rules.
fn is_derived_recursor(ctx: &Context, key: &str) -> bool {
    match key.strip_suffix("/s:\"rec\"") {
        Some(owner) => ctx.is_inductive(owner),
        None => false,
    }
}

/// The first non-helper unit a helper is reached from, walking referrers.
fn owner_of(unit: usize, referrers: &[BTreeSet<usize>], closure: &Closure) -> Option<usize> {
    let mut seen = BTreeSet::new();
    let mut frontier: Vec<usize> = vec![unit];
    while let Some(u) = frontier.pop() {
        for &r in &referrers[u] {
            if !seen.insert(r) {
                continue;
            }
            if closure.decls[r].key().starts_with(HELPER_PREFIX) {
                frontier.push(r);
            } else {
                return Some(r);
            }
        }
    }
    None
}

/// Universe parameter names: the Lean name's last component when it is a plain identifier, made
/// distinct.
fn level_names(levels: &[String]) -> Vec<(String, String)> {
    let mut used: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(levels.len());
    for level in levels {
        let base = match names::lean_name(level).and_then(|p| p.last().cloned()) {
            Some(LeanPart::Str(s)) if terms::is_plain_identifier(&s) => s,
            Some(LeanPart::Num(n)) => format!("u{n}"),
            _ => "u".to_string(),
        };
        let name = std::iter::once(base.clone())
            .chain((1usize..).map(|i| format!("{base}{i}")))
            .find(|n| !used.contains(n))
            .expect("an unbounded sequence of candidates has a free one");
        used.insert(name.clone());
        out.push((level.clone(), name));
    }
    out
}

/// The namespace scope a declaration named `declared` elaborates in.
fn declaration_scope(declared: &str) -> Namespaces {
    let mut ns = Namespaces::new();
    if let Some(name) = Name::parse(declared) {
        ns.enter_declaration(&name, 0);
    }
    ns
}

/// The shortest written name that resolves to `key` in `scope`, or its `_root_` form.
fn written_reference(
    scope: &Namespaces,
    known: &dyn Fn(&str) -> bool,
    key: &str,
) -> Option<String> {
    let parts: Vec<String> = key.split('.').map(str::to_string).collect();
    for take in 1..=parts.len() {
        let candidate = Name::from_parts(parts[parts.len() - take..].to_vec())?;
        if let Resolution::Found(found) = scope.resolve(&candidate, known) {
            if found == key {
                return Some(candidate.key());
            }
        }
    }
    let rooted = Name::from_parts(
        std::iter::once(crate::interface::surface::ROOT_ESCAPE.to_string())
            .chain(parts)
            .collect(),
    )?;
    match scope.resolve(&rooted, known) {
        Resolution::Found(found) if found == key => Some(rooted.key()),
        _ => None,
    }
}

/// A constructor's name as written inside its inductive's declaration: relative to the namespace
/// the inductive is declared in.
fn relative_to(key: &str, enclosing: &[&str]) -> String {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.len() > enclosing.len() && parts[..enclosing.len()] == *enclosing {
        parts[enclosing.len()..].join(".")
    } else {
        format!("{}.{key}", crate::interface::surface::ROOT_ESCAPE)
    }
}

/// Prose made safe inside a `/-! … -/` or `/-- … -/` block: no nested opener or closer.
fn prose(text: &str) -> String {
    text.replace("/-", "/\u{2010}").replace("-/", "\u{2010}/")
}

/// A unit as it will be printed.
enum Written {
    Definition {
        name: String,
        levels: Vec<String>,
        ty: Term,
        body: Term,
    },
    Theorem {
        name: String,
        transparent: bool,
        levels: Vec<String>,
        ty: Term,
        proof: Term,
    },
    Axiom {
        name: String,
        levels: Vec<String>,
        ty: Term,
    },
    Quotient {
        name: String,
        kind: crate::quotient::Kind,
        levels: Vec<String>,
        equality: Option<String>,
    },
    Inductive(Vec<WrittenInductive>),
}

struct WrittenInductive {
    name: String,
    levels: Vec<String>,
    params: Vec<(String, Term)>,
    sort: Term,
    constructors: Vec<(String, Term)>,
    /// Constructors carry their full parameter telescope.
    full: bool,
    /// For a structure, its projections as written: name, type, body.
    structure: Option<Vec<(String, Term, Term)>>,
    result_heads: Vec<String>,
}

impl Written {
    fn logos_name(&self) -> &str {
        match self {
            Written::Definition { name, .. }
            | Written::Theorem { name, .. }
            | Written::Axiom { name, .. }
            | Written::Quotient { name, .. } => name,
            Written::Inductive(members) => &members[0].name,
        }
    }
}
