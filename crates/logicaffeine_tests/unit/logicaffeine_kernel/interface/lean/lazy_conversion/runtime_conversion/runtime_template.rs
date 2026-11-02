//! Checked static call templates for admitted native lambdas.
//!
//! Program data is untrusted. Validation must reconstruct the closed lambda and
//! check its actual conversion with the original in both kernels.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TemplateRef {
    Argument(usize),
    Ground(usize),
    Call(usize),
}
pub(super) struct CallInstruction {
    pub(super) function: TemplateRef,
    pub(super) arguments: Vec<TemplateRef>,
}
pub(super) struct Program {
    pub(super) parameters: Vec<(String, Term)>,
    pub(super) grounds: Vec<Term>,
    pub(super) calls: Vec<CallInstruction>,
    pub(super) result: TemplateRef,
}
pub(super) struct CheckedTemplate {
    program: Program,
    actual_type: Term,
}
impl CheckedTemplate {
    pub(super) fn program(&self) -> &Program {
        &self.program
    }
    pub(super) fn actual_type(&self) -> &Term {
        &self.actual_type
    }
}

const TEMPLATE_NODES: usize = 256;
const TEMPLATE_DEPTH: usize = 128;
const TEMPLATE_PAYLOAD: usize = 4096;

#[derive(Clone, Copy)]
struct Shape {
    nodes: usize,
    levels: usize,
    depth: usize,
    cost: usize,
}
fn text(name: &str, budget: &mut usize) -> Result<()> {
    if name.len() > TEMPLATE_PAYLOAD {
        return Err(declined(
            DiagnosticCode::ResourceLimit,
            "template name payload bound",
        ));
    }
    spend(budget, name.len())
}
fn plus(a: usize, b: usize) -> Result<usize> {
    a.checked_add(b)
        .ok_or_else(|| declined(DiagnosticCode::ResourceLimit, "template expansion overflow"))
}
fn bounded(shape: Shape) -> Result<Shape> {
    if shape.nodes > TEMPLATE_NODES
        || shape.levels > TEMPLATE_NODES
        || shape.depth >= TEMPLATE_DEPTH
    {
        return Err(declined(
            DiagnosticCode::ResourceLimit,
            "template expanded readback bound",
        ));
    }
    Ok(shape)
}
fn apply_shape(function: Shape, argument: Shape) -> Result<Shape> {
    bounded(Shape {
        nodes: plus(plus(function.nodes, argument.nodes)?, 1)?,
        levels: plus(function.levels, argument.levels)?,
        depth: plus(function.depth.max(argument.depth), 1)?,
        cost: plus(plus(function.cost, argument.cost)?, 1)?,
    })
}
fn universe_nodes(root: &Universe, budget: &mut usize) -> Result<usize> {
    let mut work = Vec::new();
    spend(budget, 1)?;
    work.push(root);
    let mut nodes = 0usize;
    while let Some(level) = work.pop() {
        nodes += 1;
        if nodes > TEMPLATE_NODES {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "template universe inventory bound",
            ));
        }
        match level {
            Universe::Succ(child) => {
                spend(budget, 1)?;
                work.push(child.as_ref());
            }
            Universe::Max(a, b) | Universe::IMax(a, b) => {
                spend(budget, 2)?;
                work.push(b.as_ref());
                work.push(a.as_ref());
            }
            _ => {}
        }
    }
    Ok(nodes)
}
fn term_shape(term: &Term, budget: &mut usize) -> Result<Shape> {
    // The existing borrowed admission bounds all payloads, including levels
    // and BigInt limbs, before this helper can authorize a syntax clone.
    let cost = native_conversion::admit(term, None, budget)?;
    let mut work = Vec::new();
    spend(budget, 1)?;
    work.push((term, 1usize));
    let mut nodes = 0usize;
    let mut levels = 0usize;
    let mut depth = 0usize;
    while let Some((term, at)) = work.pop() {
        nodes += 1;
        depth = depth.max(at);
        if nodes > TEMPLATE_NODES || at >= TEMPLATE_DEPTH {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "template term admission bound",
            ));
        }
        // Count the full level tree of every occurrence. Later layout
        // duplicates this count alongside term nodes when a call slot repeats.
        match term {
            Term::Sort(level) => levels = plus(levels, universe_nodes(level, budget)?)?,
            Term::Const {
                levels: arguments, ..
            } => {
                for level in arguments {
                    levels = plus(levels, universe_nodes(level, budget)?)?;
                }
            }
            _ => {}
        }
        let mut push = |child| -> Result<()> {
            spend(budget, 1)?;
            work.push((child, at + 1));
            Ok(())
        };
        match term {
            Term::App(f, a) => {
                push(f.as_ref())?;
                push(a.as_ref())?;
            }
            Term::Lambda {
                param_type, body, ..
            }
            | Term::Pi {
                param_type,
                body_type: body,
                ..
            } => {
                push(param_type.as_ref())?;
                push(body.as_ref())?;
            }
            Term::Let {
                ty, value, body, ..
            } => {
                push(ty.as_ref())?;
                push(value.as_ref())?;
                push(body.as_ref())?;
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                push(discriminant.as_ref())?;
                push(motive.as_ref())?;
                for case in cases {
                    push(case)?;
                }
            }
            Term::Fix { body, .. } => push(body.as_ref())?,
            _ => {}
        }
    }
    bounded(Shape {
        nodes,
        levels,
        depth,
        cost,
    })
}

fn reference_shape(
    reference: TemplateRef,
    parameters: &[(String, Term)],
    grounds: &[Shape],
    calls: &[Shape],
) -> Result<Shape> {
    match reference {
        TemplateRef::Argument(slot) => {
            let (name, _) = parameters
                .get(slot)
                .ok_or_else(|| rejected(DiagnosticCode::InvalidProof, "template argument slot"))?;
            Ok(Shape {
                nodes: 1,
                levels: 0,
                depth: 1,
                cost: plus(1, name.len())?,
            })
        }
        TemplateRef::Ground(slot) => grounds
            .get(slot)
            .copied()
            .ok_or_else(|| rejected(DiagnosticCode::InvalidProof, "template ground slot")),
        TemplateRef::Call(slot) => calls.get(slot).copied().ok_or_else(|| {
            rejected(
                DiagnosticCode::InvalidProof,
                "template call must precede its use",
            )
        }),
    }
}

/// Check every stored instruction, including unused ones. The returned bound
/// counts EXPANDED occurrences; shared call slots do not discount readback.
fn layout(program: &Program, budget: &mut usize) -> Result<Shape> {
    spend(budget, 1)?;
    let slots = plus(
        plus(program.parameters.len(), program.grounds.len())?,
        program.calls.len(),
    )?;
    if slots > TEMPLATE_NODES {
        return Err(declined(
            DiagnosticCode::ResourceLimit,
            "template program inventory bound",
        ));
    }
    spend(budget, slots)?;
    let mut names = HashSet::new();
    let mut domains = Vec::with_capacity(program.parameters.len());
    for (name, domain) in &program.parameters {
        text(name, budget)?;
        if !names.insert(name.as_str()) {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "template duplicate formal name",
            ));
        }
        // This first template fragment has independently closed domains.
        // Dependent native telescopes conservatively remain on the old path.
        domains.push(term_shape(domain, budget)?);
    }
    let mut grounds = Vec::with_capacity(program.grounds.len());
    for ground in &program.grounds {
        grounds.push(term_shape(ground, budget)?);
    }
    let mut calls = Vec::with_capacity(program.calls.len());
    for call in &program.calls {
        if call.arguments.is_empty() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "template empty call",
            ));
        }
        if call.arguments.len() > TEMPLATE_NODES {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "template call arity bound",
            ));
        }
        spend(budget, call.arguments.len() + 1)?;
        let mut shape = reference_shape(call.function, &program.parameters, &grounds, &calls)?;
        for argument in &call.arguments {
            shape = apply_shape(
                shape,
                reference_shape(*argument, &program.parameters, &grounds, &calls)?,
            )?;
        }
        calls.push(shape);
    }
    let mut shape = reference_shape(program.result, &program.parameters, &grounds, &calls)?;
    for ((name, _), domain) in program.parameters.iter().zip(domains).rev() {
        shape = bounded(Shape {
            nodes: plus(plus(shape.nodes, domain.nodes)?, 1)?,
            levels: plus(shape.levels, domain.levels)?,
            depth: plus(shape.depth.max(domain.depth), 1)?,
            cost: plus(plus(plus(shape.cost, domain.cost)?, name.len())?, 1)?,
        })?;
    }
    Ok(shape)
}

/// Reconstruct syntax only. All expanded term/level/name/literal payload costs
/// are admitted and paid before the first recursive clone or Box allocation.
pub(super) fn reify(program: &Program, budget: &mut usize) -> Result<Term> {
    let shape = layout(program, budget)?;
    spend(budget, shape.cost)?;
    fn build(program: &Program, reference: TemplateRef) -> Term {
        match reference {
            TemplateRef::Argument(slot) => Term::Var(program.parameters[slot].0.clone()),
            TemplateRef::Ground(slot) => program.grounds[slot].clone(),
            TemplateRef::Call(slot) => {
                let call = &program.calls[slot];
                let mut term = build(program, call.function);
                for argument in &call.arguments {
                    term = Term::App(Box::new(term), Box::new(build(program, *argument)));
                }
                term
            }
        }
    }
    // layout established acyclic backward references and depth < 128.
    let mut body = build(program, program.result);
    for (name, domain) in program.parameters.iter().rev() {
        body = Term::Lambda {
            param: name.clone(),
            param_type: Box::new(domain.clone()),
            body: Box::new(body),
        };
    }
    Ok(body)
}

fn checked_type(ctx: &Context, original: &Term, budget: &mut usize) -> Result<Term> {
    native_conversion::admit(original, None, budget)?;
    spend(budget, 1)?;
    let actual = lean_type(ctx, original)?;
    let independent = crate::recheck(ctx, original)
        .map_err(|e| independent_verdict(&e, "independent template typing"))?;
    native_conversion::admit(&actual, None, budget)?;
    native_conversion::admit(&independent, None, budget)?;
    exact(ctx, &actual, &independent)?;
    Ok(actual)
}
fn verify_checked(
    ctx: &Context,
    original: &Term,
    program: &Program,
    actual_type: &Term,
    budget: &mut usize,
) -> Result<()> {
    let rebuilt = reify(program, budget)?;
    let mut bridge = native_conversion::NativeCertificates::new(ctx, budget)?;
    // The actual original function type is used, never a claimed program type.
    // The annotated conversion witness checks the reified endpoint in BOTH
    // kernels, including all its domains and original argument positions.
    bridge.conversion(original, &rebuilt, actual_type, budget)
}
fn validate_checked(
    ctx: &Context,
    original: &Term,
    program: Program,
    actual_type: Term,
    budget: &mut usize,
) -> Result<CheckedTemplate> {
    verify_checked(ctx, original, &program, &actual_type, budget)?;
    Ok(CheckedTemplate {
        program,
        actual_type,
    })
}

/// Independently verify borrowed, untrusted program data. No Program clone or
/// cached token is needed before layout/readback admission. The returned type
/// is freshly inferred from the actual original, not read from producer fields.
pub(super) fn verify(
    ctx: &Context,
    original: &Term,
    program: &Program,
    budget: &mut usize,
) -> Result<Term> {
    let actual_type = checked_type(ctx, original, budget)?;
    verify_checked(ctx, original, program, &actual_type, budget)?;
    Ok(actual_type)
}

/// Independently reconstruct and check an untrusted candidate program.
pub(super) fn validate(
    ctx: &Context,
    original: &Term,
    program: Program,
    budget: &mut usize,
) -> Result<CheckedTemplate> {
    let actual_type = verify(ctx, original, &program, budget)?;
    Ok(CheckedTemplate {
        program,
        actual_type,
    })
}

/// Is this subtree closed on its own? This walk carries its own lexical
/// binders, rather than assuming a surrounding compiler environment is empty.
fn closed(term: &Term, budget: &mut usize) -> Result<bool> {
    enum Frame<'a> {
        Visit(&'a Term),
        Bind(&'a str),
        Unbind,
    }
    fn push<'a>(work: &mut Vec<Frame<'a>>, frame: Frame<'a>, budget: &mut usize) -> Result<()> {
        spend(budget, 1)?;
        work.push(frame);
        Ok(())
    }
    let mut work = Vec::new();
    let mut bound = Vec::new();
    push(&mut work, Frame::Visit(term), budget)?;
    while let Some(frame) = work.pop() {
        match frame {
            Frame::Bind(name) => {
                spend(budget, 1)?;
                bound.push(name);
            }
            Frame::Unbind => {
                bound.pop();
            }
            Frame::Visit(term) => match term {
                Term::Var(name) => {
                    let mut found = false;
                    for binder in bound.iter().rev() {
                        spend(budget, 1 + name.len())?;
                        if *binder == name.as_str() {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        return Ok(false);
                    }
                }
                Term::Lambda {
                    param,
                    param_type,
                    body,
                }
                | Term::Pi {
                    param,
                    param_type,
                    body_type: body,
                } => {
                    push(&mut work, Frame::Unbind, budget)?;
                    push(&mut work, Frame::Visit(body), budget)?;
                    push(&mut work, Frame::Bind(param), budget)?;
                    push(&mut work, Frame::Visit(param_type), budget)?;
                }
                Term::Let {
                    name,
                    ty,
                    value,
                    body,
                } => {
                    push(&mut work, Frame::Unbind, budget)?;
                    push(&mut work, Frame::Visit(body), budget)?;
                    push(&mut work, Frame::Bind(name), budget)?;
                    push(&mut work, Frame::Visit(value), budget)?;
                    push(&mut work, Frame::Visit(ty), budget)?;
                }
                Term::App(f, a) => {
                    push(&mut work, Frame::Visit(a), budget)?;
                    push(&mut work, Frame::Visit(f), budget)?;
                }
                Term::Sort(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) => {}
                // Opaque compound syntax can still be retained as a ground
                // fragment after the existing closed admission checks it.
                _ => return Ok(false),
            },
        }
    }
    Ok(true)
}

struct Compiler<'a> {
    ctx: &'a Context,
    program: Program,
}
impl Compiler<'_> {
    fn ground(&mut self, term: &Term, reduce: bool, budget: &mut usize) -> Result<TemplateRef> {
        if self.program.grounds.len() >= TEMPLATE_NODES {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "template ground inventory bound",
            ));
        }
        let admitted = term_shape(term, budget)?;
        let value = if reduce {
            let actual = checked_type(self.ctx, term, budget)?;
            spend(budget, 1)?;
            if matches!(normal(self.ctx, &actual)?, Term::Pi { .. }) {
                // A partial native application stays compact. Later dynamic
                // arguments are not interpreted or guessed during compilation.
                spend(budget, admitted.cost)?;
                term.clone()
            } else {
                let mut bridge = native_conversion::NativeCertificates::new(self.ctx, budget)?;
                match bridge.normalize_ground(term, budget) {
                    Ok(value) => value,
                    Err(Verdict::Decline(_)) => {
                        // Expanding a closed typeclass instance may exceed the
                        // native output bound. Keep its admitted original
                        // syntax only after BOTH kernels check the identity
                        // link, with the allowance already spent still spent.
                        bridge.conversion(term, term, &actual, budget)?;
                        spend(budget, admitted.cost)?;
                        term.clone()
                    }
                    Err(error) => return Err(error),
                }
            }
        } else {
            spend(budget, admitted.cost)?;
            term.clone()
        };
        term_shape(&value, budget)?;
        spend(budget, 1)?;
        let slot = self.program.grounds.len();
        self.program.grounds.push(value);
        Ok(TemplateRef::Ground(slot))
    }
    fn lower<'t>(
        &mut self,
        term: &'t Term,
        locals: &mut Vec<(&'t str, TemplateRef)>,
        budget: &mut usize,
    ) -> Result<TemplateRef> {
        spend(budget, 1)?;
        match term {
            Term::Var(name) => {
                for (binder, value) in locals.iter().rev() {
                    spend(budget, 1 + name.len())?;
                    if *binder == name.as_str() {
                        return Ok(*value);
                    }
                }
                Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "template free compiler variable",
                ))
            }
            Term::Let {
                name, value, body, ..
            } => {
                // Original whole-term checking preceded this zeta translation,
                // so even an erased annotation and initializer were checked.
                let value = self.lower(value, locals, budget)?;
                spend(budget, 1)?;
                locals.push((name, value));
                let result = self.lower(body, locals, budget);
                locals.pop();
                result
            }
            Term::App(_, _) if closed(term, budget)? => self.ground(term, true, budget),
            Term::App(_, _) => {
                let mut head = term;
                let mut arguments = Vec::new();
                while let Term::App(f, a) = head {
                    // Keep the largest independently closed prefix as ONE
                    // ground function. It is an existing borrowed subtree:
                    // lower/ground admits it before any materialization and
                    // whole-program validation checks the resulting equation.
                    // Once a dynamic operand occurs, all subsequent operands
                    // stay in their original order in the dynamic instruction.
                    if !arguments.is_empty() && closed(head, budget)? {
                        break;
                    }
                    if arguments.len() >= TEMPLATE_NODES {
                        return Err(declined(
                            DiagnosticCode::ResourceLimit,
                            "template compiler arity bound",
                        ));
                    }
                    spend(budget, 1)?;
                    arguments.push(a.as_ref());
                    head = f;
                }
                arguments.reverse();
                let function = self.lower(head, locals, budget)?;
                spend(budget, arguments.len())?;
                let mut inputs = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    inputs.push(self.lower(argument, locals, budget)?);
                }
                if self.program.calls.len() >= TEMPLATE_NODES {
                    return Err(declined(
                        DiagnosticCode::ResourceLimit,
                        "template instruction bound",
                    ));
                }
                spend(budget, 1)?;
                let slot = self.program.calls.len();
                self.program.calls.push(CallInstruction {
                    function,
                    arguments: inputs,
                });
                Ok(TemplateRef::Call(slot))
            }
            _ if closed(term, budget)? => self.ground(term, false, budget),
            _ => Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "native template needs a captured nested lambda or unsupported rule",
            )),
        }
    }
}

/// Compile one small native lambda. Exact source/context identity remains with
/// the caller; the returned code is validated rather than trusted by construction.
pub(super) fn compile(
    ctx: &Context,
    original: &Term,
    budget: &mut usize,
) -> Result<Option<CheckedTemplate>> {
    let actual_type = checked_type(ctx, original, budget)?;
    if !matches!(original, Term::Lambda { .. }) {
        return Ok(None);
    }
    let mut compiler = Compiler {
        ctx,
        program: Program {
            parameters: Vec::new(),
            grounds: Vec::new(),
            calls: Vec::new(),
            result: TemplateRef::Argument(0),
        },
    };
    let mut locals = Vec::new();
    let mut body = original;
    while let Term::Lambda {
        param,
        param_type,
        body: next,
    } = body
    {
        if compiler.program.parameters.len() >= TEMPLATE_NODES {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "template formal inventory bound",
            ));
        }
        // Only closed domains are supported initially. They remain outside
        // the binder's scope and never receive a value substitution.
        let domain = term_shape(param_type, budget)?;
        let slot = compiler.program.parameters.len();
        // Fresh output names cannot capture anything: domains/ground terms
        // are closed, while all open references use exact formal slots.
        spend(budget, 32)?;
        let name = format!("template.parameter.{slot}");
        spend(budget, domain.cost + 2)?;
        compiler
            .program
            .parameters
            .push((name, param_type.as_ref().clone()));
        locals.push((param.as_str(), TemplateRef::Argument(slot)));
        body = next;
    }
    compiler.program.result = compiler.lower(body, &mut locals, budget)?;
    validate_checked(ctx, original, compiler.program, actual_type, budget).map(Some)
}

#[cfg(test)]
#[path = "runtime_template/controls.rs"]
mod controls;

#[cfg(test)]
#[path = "runtime_template/boundary_controls.rs"]
mod boundary_controls;

#[cfg(test)]
#[path = "runtime_template/prefix_controls.rs"]
mod prefix_controls;
