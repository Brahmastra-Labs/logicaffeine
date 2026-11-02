//! Bounded native links for lazy source conversion. Both kernels check an
//! actual conversion witness; matching endpoint types alone grants no evidence.
//! This experimental bridge is not yet an importer acceptance path.
use super::*;
use crate::{instantiate_universes, resolve_const_levels, Literal};

const NATIVE_NODES: usize = 256;
const NATIVE_DEPTH: usize = 128;
const NATIVE_PAYLOAD: usize = 4096;

fn payload(text: &str, budget: &mut usize) -> Result<usize> {
    if text.len() > NATIVE_PAYLOAD {
        return Err(declined(
            DiagnosticCode::ResourceLimit,
            "native text payload bound",
        ));
    }
    spend(budget, text.len())?;
    Ok(text.len())
}

/// Borrowed admission counts expanded occurrences, including the output of a
/// universe substitution BEFORE instantiate_universes creates that output.
pub(super) fn admit(
    term: &Term,
    substitution: Option<&HashMap<String, Universe>>,
    budget: &mut usize,
) -> Result<usize> {
    enum Frame<'a> {
        Term(&'a Term, usize),
        Level(&'a Universe, usize, bool),
        Bind(&'a str),
        Unbind(&'a str),
    }
    fn push<'a>(work: &mut Vec<Frame<'a>>, frame: Frame<'a>, budget: &mut usize) -> Result<()> {
        spend(budget, 1)?;
        work.push(frame);
        Ok(())
    }
    let mut work = Vec::new();
    let mut bound = HashMap::<&str, usize>::new();
    let mut nodes = 0usize;
    let mut level_nodes = 0usize;
    let mut copied = 0usize;
    push(&mut work, Frame::Term(term, 1), budget)?;
    while let Some(frame) = work.pop() {
        match frame {
            Frame::Bind(name) => {
                *bound.entry(name).or_default() += 1;
            }
            Frame::Unbind(name) => {
                let count = bound.get_mut(name).ok_or_else(|| {
                    rejected(DiagnosticCode::InvalidProof, "native scope mismatch")
                })?;
                *count -= 1;
            }
            Frame::Level(level, depth, replace) => {
                if depth >= NATIVE_DEPTH {
                    return Err(declined(
                        DiagnosticCode::ResourceLimit,
                        "native universe depth bound",
                    ));
                }
                if replace {
                    if let Universe::Var(name) = level {
                        if let Some(value) = substitution.and_then(|s| s.get(name)) {
                            push(&mut work, Frame::Level(value, depth, false), budget)?;
                            continue;
                        }
                    }
                }
                level_nodes += 1;
                copied += 1;
                if level_nodes > NATIVE_NODES {
                    return Err(declined(
                        DiagnosticCode::ResourceLimit,
                        "native universe size bound",
                    ));
                }
                match level {
                    Universe::Var(name) => {
                        if name.starts_with('?') {
                            return Err(declined(
                                DiagnosticCode::UnsupportedFeature,
                                "native unresolved universe",
                            ));
                        }
                        copied += payload(name, budget)?;
                    }
                    Universe::Succ(a) => {
                        push(&mut work, Frame::Level(a, depth + 1, replace), budget)?
                    }
                    Universe::Max(a, b) | Universe::IMax(a, b) => {
                        push(&mut work, Frame::Level(b, depth + 1, replace), budget)?;
                        push(&mut work, Frame::Level(a, depth + 1, replace), budget)?;
                    }
                    Universe::Type(_) | Universe::Prop | Universe::SProp => {}
                }
            }
            Frame::Term(term, depth) => {
                nodes += 1;
                copied += 1;
                if nodes > NATIVE_NODES || depth >= NATIVE_DEPTH {
                    return Err(declined(
                        DiagnosticCode::ResourceLimit,
                        "native term size or depth bound",
                    ));
                }
                match term {
                    Term::Sort(level) => push(&mut work, Frame::Level(level, 1, true), budget)?,
                    Term::Var(name) => {
                        copied += payload(name, budget)?;
                        if bound.get(name.as_str()).copied().unwrap_or(0) == 0 {
                            return Err(declined(
                                DiagnosticCode::UnsupportedFeature,
                                "native term has an unbound variable",
                            ));
                        }
                    }
                    Term::Global(name) => {
                        copied += payload(name, budget)?;
                    }
                    Term::Const { name, levels } => {
                        copied += payload(name, budget)?;
                        if levels.len() > NATIVE_NODES {
                            return Err(declined(
                                DiagnosticCode::ResourceLimit,
                                "native universe arity bound",
                            ));
                        }
                        for level in levels {
                            push(&mut work, Frame::Level(level, 1, true), budget)?;
                        }
                    }
                    Term::Pi {
                        param,
                        param_type,
                        body_type,
                    }
                    | Term::Lambda {
                        param,
                        param_type,
                        body: body_type,
                    } => {
                        copied += payload(param, budget)?;
                        push(&mut work, Frame::Unbind(param), budget)?;
                        push(&mut work, Frame::Term(body_type, depth + 1), budget)?;
                        push(&mut work, Frame::Bind(param), budget)?;
                        push(&mut work, Frame::Term(param_type, depth + 1), budget)?;
                    }
                    Term::App(f, a) => {
                        push(&mut work, Frame::Term(a, depth + 1), budget)?;
                        push(&mut work, Frame::Term(f, depth + 1), budget)?;
                    }
                    Term::Let {
                        name,
                        ty,
                        value,
                        body,
                    } => {
                        copied += payload(name, budget)?;
                        push(&mut work, Frame::Unbind(name), budget)?;
                        push(&mut work, Frame::Term(body, depth + 1), budget)?;
                        push(&mut work, Frame::Bind(name), budget)?;
                        push(&mut work, Frame::Term(value, depth + 1), budget)?;
                        push(&mut work, Frame::Term(ty, depth + 1), budget)?;
                    }
                    Term::Match {
                        discriminant,
                        motive,
                        cases,
                    } => {
                        if cases.len() > NATIVE_NODES {
                            return Err(declined(
                                DiagnosticCode::ResourceLimit,
                                "native case count bound",
                            ));
                        }
                        for case in cases {
                            push(&mut work, Frame::Term(case, depth + 1), budget)?;
                        }
                        push(&mut work, Frame::Term(motive, depth + 1), budget)?;
                        push(&mut work, Frame::Term(discriminant, depth + 1), budget)?;
                    }
                    Term::Fix { name, body } => {
                        copied += payload(name, budget)?;
                        push(&mut work, Frame::Unbind(name), budget)?;
                        push(&mut work, Frame::Term(body, depth + 1), budget)?;
                        push(&mut work, Frame::Bind(name), budget)?;
                    }
                    Term::Lit(Literal::Text(text)) => {
                        copied += payload(text, budget)?;
                    }
                    Term::Lit(Literal::Nat(n) | Literal::BigInt(n)) => {
                        let bytes = n.clone_payload_bytes();
                        if bytes > NATIVE_PAYLOAD {
                            return Err(declined(
                                DiagnosticCode::ResourceLimit,
                                "native integer payload bound",
                            ));
                        }
                        spend(budget, bytes)?;
                        copied += bytes;
                    }
                    Term::Lit(_) => {}
                    Term::Annotated { term, .. } => {
                        push(&mut work, Frame::Term(term, depth + 1), budget)?
                    }
                    Term::Hole | Term::MutualFix { .. } => {
                        return Err(declined(
                            DiagnosticCode::UnsupportedFeature,
                            "native syntax outside bounded bridge",
                        ));
                    }
                }
            }
        }
    }
    Ok(copied)
}

pub(super) struct NativeCertificates<'ctx> {
    ctx: &'ctx Context,
}
impl<'ctx> NativeCertificates<'ctx> {
    pub(super) fn new(ctx: &'ctx Context, budget: &mut usize) -> Result<Self> {
        spend(budget, 1)?;
        Ok(Self { ctx })
    }
    fn checked_type(&self, term: &Term, budget: &mut usize) -> Result<Term> {
        spend(budget, 1)?;
        let ty = lean_type(self.ctx, term)?;
        let independent = crate::recheck(self.ctx, term)
            .map_err(|e| independent_verdict(&e, "independent native typing"))?;
        exact(self.ctx, &ty, &independent)?;
        admit(&ty, None, budget)?;
        Ok(ty)
    }
    pub(super) fn expose(&mut self, constant: &Term, budget: &mut usize) -> Result<Term> {
        admit(constant, None, budget)?;
        let (name, levels) = match constant {
            Term::Global(name) => (name.as_str(), None),
            Term::Const { name, levels } => (name.as_str(), Some(levels.as_slice())),
            _ => {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    "native delta requires a constant instance",
                ))
            }
        };
        let (declared, header, body) =
            if let Some((params, ty, body)) = self.ctx.get_universe_poly(name) {
                (Some(params.as_slice()), ty, body)
            } else {
                let header = self.ctx.get_global(name).ok_or_else(|| {
                    rejected(
                        DiagnosticCode::MissingDeclaration,
                        "unknown native constant",
                    )
                })?;
                let body = self.ctx.get_definition_body(name).ok_or_else(|| {
                    declined(
                        DiagnosticCode::UnsupportedFeature,
                        "native constant has no published transparent body",
                    )
                })?;
                (
                    self.ctx.poly_declaration(name).map(|(params, _)| params),
                    header,
                    body,
                )
            };
        // The resolver scans/clones its telescope and levels; admit all of them
        // before using that single authoritative universe-instance algorithm.
        admit(header, None, budget)?;
        admit(body, None, budget)?;
        if let Some(params) = declared {
            spend(budget, params.len())?;
            for param in params {
                payload(param, budget)?;
            }
        }
        let levels = levels.unwrap_or(&[]);
        let substitutions = resolve_const_levels(name, declared, header, levels).map_err(|_| {
            rejected(
                DiagnosticCode::InvalidProof,
                "native constant universe instance mismatch",
            )
        })?;
        let header_cost = admit(header, Some(&substitutions), budget)?;
        spend(budget, header_cost)?;
        let body_cost = admit(body, Some(&substitutions), budget)?;
        spend(budget, body_cost)?;
        let exposed = instantiate_universes(body, &substitutions);
        let actual_type = self.checked_type(constant, budget)?;
        self.conversion(constant, &exposed, &actual_type, budget)?;
        Ok(exposed)
    }
    pub(super) fn conversion(
        &mut self,
        left: &Term,
        right: &Term,
        ty: &Term,
        budget: &mut usize,
    ) -> Result<()> {
        // All three inputs are lexically closed, so these local binders cannot
        // capture any endpoint variable, regardless of names inside endpoints.
        let left_cost = admit(left, None, budget)?;
        let right_cost = admit(right, None, budget)?;
        let type_cost = admit(ty, None, budget)?;
        let (p, h, k, x) = ("native.P", "native.h", "native.k", "native.x");
        let names = p.len() * 3 + h.len() * 2 + k.len() * 2 + x.len();
        spend(budget, left_cost + right_cost + type_cost + 12 + names)?;
        let family = Term::Pi {
            param: x.into(),
            param_type: Box::new(ty.clone()),
            body_type: Box::new(Term::Sort(Universe::Type(0))),
        };
        let from = Term::App(Box::new(Term::Var(p.into())), Box::new(left.clone()));
        let to = Term::App(Box::new(Term::Var(p.into())), Box::new(right.clone()));
        let witness = Term::Lambda {
            param: p.into(),
            param_type: Box::new(family),
            body: Box::new(Term::Lambda {
                param: h.into(),
                param_type: Box::new(from),
                body: Box::new(Term::Let {
                    name: k.into(),
                    ty: Box::new(to),
                    value: Box::new(Term::Var(h.into())),
                    body: Box::new(Term::Var(k.into())),
                }),
            }),
        };
        // Checking the annotated Let forces P(left) = P(right) in BOTH kernels.
        // Their normal inference/reduction budgets remain in force; the caller
        // allowance bounds this bridge's explicit admissions/materializations.
        spend(budget, 1)?;
        lean_type(self.ctx, &witness)?;
        crate::recheck(self.ctx, &witness)
            .map_err(|e| independent_verdict(&e, "independent native conversion witness"))?;
        Ok(())
    }
    pub(super) fn normalize_ground(&mut self, call: &Term, budget: &mut usize) -> Result<Term> {
        admit(call, None, budget)?;
        let ty = self.checked_type(call, budget)?;
        spend(budget, 1)?;
        let normalized_type = normal(self.ctx, &ty)?;
        if matches!(normalized_type, Term::Pi { .. }) {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "native partial application remains lazy",
            ));
        }
        spend(budget, 1)?;
        let result = normal(self.ctx, call)?;
        admit(&result, None, budget)?;
        self.conversion(call, &result, &ty, budget)?;
        Ok(result)
    }
}

#[cfg(test)]
#[path = "native_conversion/controls.rs"]
mod controls;

#[cfg(test)]
#[path = "native_conversion/boundary_controls.rs"]
mod boundary_controls;
