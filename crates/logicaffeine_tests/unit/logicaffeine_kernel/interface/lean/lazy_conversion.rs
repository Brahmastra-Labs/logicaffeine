//! Bounded beta/zeta traces over original source closures.
//!
//! This is experimental groundwork, not an importer acceptance path. Full source
//! typing is a separate prerequisite. The producer is untrusted: replay rebuilds
//! every endpoint from source rows and exact captured value environments.
//! Delta, iota, primitive evaluation and quoting open lambda closures are outside
//! this fragment and explicitly decline. An atom is a beta/zeta endpoint, not a
//! claim that its global definition has been fully normalized.
use super::*;

fn spend(budget: &mut usize, amount: usize) -> Result<()> {
    *budget = budget.checked_sub(amount).ok_or_else(|| {
        declined(
            DiagnosticCode::ResourceLimit,
            "lazy conversion trace allowance exhausted",
        )
    })?;
    Ok(())
}

/// Immutable source/context borrow prevents a typing token crossing a mutation.
/// No constructor accepts a caller-supplied Boolean or claimed type.
struct TypedSource<'source, 'input> {
    importer: &'source Importer<'input>,
    root: usize,
    typing: beta::SourceCertificates<'source, 'input>,
}
impl<'source, 'input> TypedSource<'source, 'input> {
    fn check(
        importer: &'source Importer<'input>,
        root: usize,
        params: &HashSet<String>,
        budget: &mut usize,
    ) -> Result<Self> {
        let mut typing = beta::SourceCertificates::new(importer, params, budget)?;
        if !typing.certify(root, budget)? {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "original source typing outside trace prerequisite",
            ));
        }
        Ok(Self {
            importer,
            root,
            typing,
        })
    }
    fn row(&self, id: usize) -> Result<&Value> {
        self.importer
            .exprs
            .get(&id)
            .ok_or_else(|| rejected(DiagnosticCode::InvalidProof, "trace source index"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Closure {
    node: usize,
    env: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Binding {
    parent: usize,
    argument: Closure,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Head {
    Atom(usize),
    Lambda(Closure),
}
#[derive(Clone, Copy, Debug)]
enum Rule {
    Atom,
    Lambda,
    Variable(usize),
    Let(usize),
    Metadata(usize),
    App { function: usize, body: usize },
}
#[derive(Clone, Debug)]
struct Step {
    subject: Closure,
    claimed: Head,
    rule: Rule,
}
struct Trace {
    // Entry zero is empty. Every binding's parent and argument environment
    // strictly precede it: no cyclic environments, including indirect cycles.
    envs: Vec<Option<Binding>>,
    steps: Vec<Step>,
}
fn atom(row: &Value) -> bool {
    row.get("sort").is_some() || row.get("const").is_some() || row.get("natVal").is_some()
}
fn lookup(
    envs: &[Option<Binding>],
    mut env: usize,
    mut index: usize,
    budget: &mut usize,
) -> Result<Closure> {
    loop {
        spend(budget, 1)?;
        let binding = envs
            .get(env)
            .and_then(Option::as_ref)
            .ok_or_else(|| rejected(DiagnosticCode::InvalidProof, "unbound trace variable"))?;
        if index == 0 {
            return Ok(binding.argument);
        }
        index -= 1;
        env = binding.parent;
    }
}

/// Producer tables are search conveniences, not evidence trusted by replay.
fn produce(source: &TypedSource<'_, '_>, budget: &mut usize) -> Result<Trace> {
    enum Frame {
        Visit(Closure),
        Finish(Closure, Closure, RuleTag),
        Function(Closure, Closure, Closure),
        Apply(Closure, usize, Closure),
    }
    #[derive(Clone, Copy)]
    enum RuleTag {
        Variable,
        Let,
        Metadata,
    }
    fn push(work: &mut Vec<Frame>, frame: Frame, budget: &mut usize) -> Result<()> {
        spend(budget, 1)?;
        work.push(frame);
        Ok(())
    }
    fn emit(
        trace: &mut Trace,
        done: &mut HashMap<Closure, usize>,
        active: &mut HashSet<Closure>,
        subject: Closure,
        claimed: Head,
        rule: Rule,
        budget: &mut usize,
    ) -> Result<()> {
        spend(budget, 2)?;
        done.insert(subject, trace.steps.len());
        trace.steps.push(Step {
            subject,
            claimed,
            rule,
        });
        active.remove(&subject);
        Ok(())
    }
    fn extend(
        trace: &mut Trace,
        ids: &mut HashMap<Binding, usize>,
        binding: Binding,
        budget: &mut usize,
    ) -> Result<usize> {
        spend(budget, 1)?;
        if let Some(id) = ids.get(&binding) {
            return Ok(*id);
        }
        spend(budget, 2)?;
        let id = trace.envs.len();
        trace.envs.push(Some(binding));
        ids.insert(binding, id);
        Ok(id)
    }
    spend(budget, 1)?;
    let mut trace = Trace {
        envs: vec![None],
        steps: Vec::new(),
    };
    let mut env_ids = HashMap::new();
    let mut done = HashMap::<Closure, usize>::new();
    let mut active = HashSet::new();
    let mut work = Vec::new();
    push(
        &mut work,
        Frame::Visit(Closure {
            node: source.root,
            env: 0,
        }),
        budget,
    )?;
    while let Some(frame) = work.pop() {
        spend(budget, 1)?;
        match frame {
            Frame::Visit(subject) => {
                if done.contains_key(&subject) {
                    continue;
                }
                spend(budget, 1)?;
                if !active.insert(subject) {
                    return Err(declined(
                        DiagnosticCode::UnsupportedFeature,
                        "cyclic lazy conversion demand",
                    ));
                }
                let row = source.row(subject.node)?;
                if let Some(var) = row.get("bvar") {
                    let target = lookup(&trace.envs, subject.env, index(var)?, budget)?;
                    push(
                        &mut work,
                        Frame::Finish(subject, target, RuleTag::Variable),
                        budget,
                    )?;
                    push(&mut work, Frame::Visit(target), budget)?;
                } else if row.get("lam").is_some() {
                    emit(
                        &mut trace,
                        &mut done,
                        &mut active,
                        subject,
                        Head::Lambda(subject),
                        Rule::Lambda,
                        budget,
                    )?;
                } else if atom(row) {
                    emit(
                        &mut trace,
                        &mut done,
                        &mut active,
                        subject,
                        Head::Atom(subject.node),
                        Rule::Atom,
                        budget,
                    )?;
                } else if let Some(app) = row.get("app") {
                    let fun = Closure {
                        node: index(&app["fn"])?,
                        env: subject.env,
                    };
                    let arg = Closure {
                        node: index(&app["arg"])?,
                        env: subject.env,
                    };
                    push(&mut work, Frame::Function(subject, fun, arg), budget)?;
                    push(&mut work, Frame::Visit(fun), budget)?;
                } else if let Some(binding) = row.get("letE") {
                    let argument = Closure {
                        node: index(&binding["value"])?,
                        env: subject.env,
                    };
                    let env = extend(
                        &mut trace,
                        &mut env_ids,
                        Binding {
                            parent: subject.env,
                            argument,
                        },
                        budget,
                    )?;
                    let body = Closure {
                        node: index(&binding["body"])?,
                        env,
                    };
                    push(
                        &mut work,
                        Frame::Finish(subject, body, RuleTag::Let),
                        budget,
                    )?;
                    push(&mut work, Frame::Visit(body), budget)?;
                } else if let Some(meta) = row.get("mdata") {
                    let body = Closure {
                        node: index(&meta["expr"])?,
                        env: subject.env,
                    };
                    push(
                        &mut work,
                        Frame::Finish(subject, body, RuleTag::Metadata),
                        budget,
                    )?;
                    push(&mut work, Frame::Visit(body), budget)?;
                } else {
                    return Err(declined(
                        DiagnosticCode::UnsupportedFeature,
                        "unsupported lazy conversion source rule",
                    ));
                }
            }
            Frame::Finish(subject, child, kind) => {
                let premise = done[&child];
                let rule = match kind {
                    RuleTag::Variable => Rule::Variable(premise),
                    RuleTag::Let => Rule::Let(premise),
                    RuleTag::Metadata => Rule::Metadata(premise),
                };
                let head = trace.steps[premise].claimed;
                emit(
                    &mut trace,
                    &mut done,
                    &mut active,
                    subject,
                    head,
                    rule,
                    budget,
                )?;
            }
            Frame::Function(subject, function, argument) => {
                let premise = done[&function];
                let Head::Lambda(lambda) = trace.steps[premise].claimed else {
                    return Err(declined(
                        DiagnosticCode::UnsupportedFeature,
                        "lazy trace requires beta; delta and neutral application unsupported",
                    ));
                };
                let env = extend(
                    &mut trace,
                    &mut env_ids,
                    Binding {
                        parent: lambda.env,
                        argument,
                    },
                    budget,
                )?;
                let body = Closure {
                    node: index(&source.row(lambda.node)?["lam"]["body"])?,
                    env,
                };
                push(&mut work, Frame::Apply(subject, premise, body), budget)?;
                push(&mut work, Frame::Visit(body), budget)?;
            }
            Frame::Apply(subject, function, body) => {
                let body = done[&body];
                let head = trace.steps[body].claimed;
                emit(
                    &mut trace,
                    &mut done,
                    &mut active,
                    subject,
                    head,
                    Rule::App { function, body },
                    budget,
                )?;
            }
        }
    }
    Ok(trace)
}

/// The endpoint is tied to the actual immutable typed source, not a caller's
/// asserted root/type/context hash. No shared producer memo is used in replay.
struct CheckedConversion<'typed, 'source, 'input> {
    source: &'typed TypedSource<'source, 'input>,
    head: Head,
}
impl CheckedConversion<'_, '_, '_> {
    fn readback(&self, budget: &mut usize) -> Result<Term> {
        let Head::Atom(id) = self.head else {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "quoting captured lambda closures is not implemented",
            ));
        };
        self.source.typing.closed_readback(id, budget)
    }
}

fn replay<'typed, 'source, 'input>(
    source: &'typed TypedSource<'source, 'input>,
    trace: &Trace,
    budget: &mut usize,
) -> Result<CheckedConversion<'typed, 'source, 'input>> {
    fn premise<'a>(steps: &'a [Step], at: usize, id: usize) -> Result<&'a Step> {
        if id >= at {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "trace premise must precede its conclusion",
            ));
        }
        steps
            .get(id)
            .ok_or_else(|| rejected(DiagnosticCode::InvalidProof, "missing trace premise"))
    }
    fn extension(envs: &[Option<Binding>], env: usize, expected: Binding) -> Result<()> {
        if envs.get(env).copied().flatten() != Some(expected) {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "trace captured value environment mismatch",
            ));
        }
        Ok(())
    }
    spend(budget, 1)?;
    if trace.envs.first() != Some(&None) || trace.steps.is_empty() {
        return Err(rejected(
            DiagnosticCode::InvalidProof,
            "trace requires an empty initial environment and a conclusion",
        ));
    }
    // No untrusted-size reserve occurs before charging that size.
    spend(budget, trace.envs.len())?;
    for (at, frame) in trace.envs.iter().enumerate().skip(1) {
        let frame = frame
            .as_ref()
            .ok_or_else(|| rejected(DiagnosticCode::InvalidProof, "missing trace environment"))?;
        if frame.parent >= at || frame.argument.env >= at {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "cyclic or forward trace environment",
            ));
        }
        source.row(frame.argument.node)?;
    }
    spend(budget, trace.steps.len())?;
    let mut verified = Vec::<Head>::with_capacity(trace.steps.len());
    for (at, step) in trace.steps.iter().enumerate() {
        spend(budget, 1)?;
        if step.subject.env >= trace.envs.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "trace environment index",
            ));
        }
        let row = source.row(step.subject.node)?;
        let endpoint = match step.rule {
            Rule::Atom if atom(row) => Head::Atom(step.subject.node),
            Rule::Lambda if row.get("lam").is_some() => Head::Lambda(step.subject),
            Rule::Variable(child) => {
                let variable = row.get("bvar").ok_or_else(|| {
                    rejected(DiagnosticCode::InvalidProof, "variable trace rule shape")
                })?;
                // Replay resolves indices independently of the producer's lookup.
                // Type environments and producer memo entries are never consulted.
                let mut environment = step.subject.env;
                let mut resolved = None;
                for distance in 0..=index(variable)? {
                    spend(budget, 1)?;
                    let frame = trace
                        .envs
                        .get(environment)
                        .and_then(Option::as_ref)
                        .ok_or_else(|| {
                            rejected(DiagnosticCode::InvalidProof, "unbound replay variable")
                        })?;
                    if distance == index(variable)? {
                        resolved = Some(frame.argument);
                        break;
                    }
                    environment = frame.parent;
                }
                let expected = resolved.ok_or_else(|| {
                    rejected(DiagnosticCode::InvalidProof, "unresolved replay variable")
                })?;
                if premise(&trace.steps, at, child)?.subject != expected {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "trace variable resolves to a different captured value",
                    ));
                }
                verified[child]
            }
            Rule::Let(child) => {
                let binding = row.get("letE").ok_or_else(|| {
                    rejected(DiagnosticCode::InvalidProof, "let trace rule shape")
                })?;
                let next = premise(&trace.steps, at, child)?.subject;
                if next.node != index(&binding["body"])? {
                    return Err(rejected(DiagnosticCode::InvalidProof, "trace let body"));
                }
                extension(
                    &trace.envs,
                    next.env,
                    Binding {
                        parent: step.subject.env,
                        argument: Closure {
                            node: index(&binding["value"])?,
                            env: step.subject.env,
                        },
                    },
                )?;
                verified[child]
            }
            Rule::Metadata(child) => {
                let meta = row.get("mdata").ok_or_else(|| {
                    rejected(DiagnosticCode::InvalidProof, "metadata trace rule shape")
                })?;
                let expected = Closure {
                    node: index(&meta["expr"])?,
                    env: step.subject.env,
                };
                if premise(&trace.steps, at, child)?.subject != expected {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "trace metadata payload",
                    ));
                }
                verified[child]
            }
            Rule::App { function, body } => {
                let app = row.get("app").ok_or_else(|| {
                    rejected(DiagnosticCode::InvalidProof, "application trace rule shape")
                })?;
                let expected = Closure {
                    node: index(&app["fn"])?,
                    env: step.subject.env,
                };
                if premise(&trace.steps, at, function)?.subject != expected {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "trace application function",
                    ));
                }
                let Head::Lambda(lambda) = verified[function] else {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "trace beta premise is not a lambda",
                    ));
                };
                let next = premise(&trace.steps, at, body)?.subject;
                let lam = source.row(lambda.node)?.get("lam").ok_or_else(|| {
                    rejected(DiagnosticCode::InvalidProof, "trace lambda endpoint shape")
                })?;
                if next.node != index(&lam["body"])? {
                    return Err(rejected(DiagnosticCode::InvalidProof, "trace beta body"));
                }
                extension(
                    &trace.envs,
                    next.env,
                    Binding {
                        parent: lambda.env,
                        argument: Closure {
                            node: index(&app["arg"])?,
                            env: step.subject.env,
                        },
                    },
                )?;
                verified[body]
            }
            _ => {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "trace rule does not describe its original source",
                ))
            }
        };
        if endpoint != step.claimed {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "forged trace endpoint",
            ));
        }
        verified.push(endpoint);
    }
    if trace.steps.last().unwrap().subject
        != (Closure {
            node: source.root,
            env: 0,
        })
    {
        return Err(rejected(
            DiagnosticCode::InvalidProof,
            "trace concludes a different source root",
        ));
    }
    Ok(CheckedConversion {
        source,
        head: *verified.last().unwrap(),
    })
}

// These controls exercise real source typing before any trace can be produced.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source() -> Importer<'static> {
        let mut i = Importer::new(false);
        i.levels.insert(1, Universe::Type(0));
        i.level_vars.insert(1, HashSet::new());
        for (id, name) in [(1, "A"), (2, "B"), (3, "f")] {
            i.names.insert(id, name.into());
            i.declarations.insert(name.into(), 0);
        }
        let ty = Term::Sort(Universe::Type(0));
        i.ctx.add_declaration("A", ty.clone());
        i.ctx.add_declaration("B", ty.clone());
        i.ctx.add_declaration(
            "f",
            Term::Pi {
                param: "x".into(),
                param_type: Box::new(ty.clone()),
                body_type: Box::new(ty),
            },
        );
        for (id, row) in [
            (0, json!({"ie":0,"sort":1})),
            (1, json!({"ie":1,"const":{"name":1,"us":[]}})),
            (2, json!({"ie":2,"const":{"name":2,"us":[]}})),
            (3, json!({"ie":3,"bvar":0})),
            (4, json!({"ie":4,"bvar":1})),
            (
                5,
                json!({"ie":5,"lam":{"name":0,"type":0,"body":4,"binderInfo":"default"}}),
            ),
            (
                6,
                json!({"ie":6,"lam":{"name":0,"type":0,"body":5,"binderInfo":"default"}}),
            ),
            (7, json!({"ie":7,"app":{"fn":6,"arg":1}})),
            (8, json!({"ie":8,"app":{"fn":7,"arg":2}})),
        ] {
            i.exprs.insert(id, row);
        }
        i
    }

    #[test]
    fn replay_uses_the_lambda_capture_not_the_callers_argument() {
        let i = source();
        let typed = TypedSource::check(&i, 8, &HashSet::new(), &mut 100_000).unwrap();
        let trace = produce(&typed, &mut 10_000).unwrap();
        let checked = replay(&typed, &trace, &mut 10_000).unwrap();
        assert_eq!(
            checked.readback(&mut 100).unwrap(),
            Term::Global("A".into())
        );
    }

    #[test]
    fn replay_rejects_a_forged_atom_endpoint() {
        let i = source();
        let typed = TypedSource::check(&i, 8, &HashSet::new(), &mut 100_000).unwrap();
        let mut trace = produce(&typed, &mut 10_000).unwrap();
        trace.steps.last_mut().unwrap().claimed = Head::Atom(2);
        assert!(matches!(
            replay(&typed, &trace, &mut 10_000),
            Err(Verdict::Reject(_))
        ));
    }

    #[test]
    fn replay_rejects_forward_and_cyclic_premises() {
        let i = source();
        let typed = TypedSource::check(&i, 8, &HashSet::new(), &mut 100_000).unwrap();
        let mut trace = produce(&typed, &mut 10_000).unwrap();
        let last = trace.steps.len() - 1;
        for target in [last, last + 1] {
            trace.steps[last].rule = Rule::App {
                function: 0,
                body: target,
            };
            assert!(matches!(
                replay(&typed, &trace, &mut 10_000),
                Err(Verdict::Reject(_))
            ));
        }
    }

    #[test]
    fn replay_rejects_a_changed_captured_value_at_the_same_type() {
        let i = source();
        let typed = TypedSource::check(&i, 8, &HashSet::new(), &mut 100_000).unwrap();
        let mut trace = produce(&typed, &mut 10_000).unwrap();
        trace.envs[1].as_mut().unwrap().argument.node = 2;
        assert!(matches!(
            replay(&typed, &trace, &mut 10_000),
            Err(Verdict::Reject(_))
        ));
    }

    #[test]
    fn replay_rejects_an_environment_cycle() {
        let i = source();
        let typed = TypedSource::check(&i, 8, &HashSet::new(), &mut 100_000).unwrap();
        let mut trace = produce(&typed, &mut 10_000).unwrap();
        trace.envs[1].as_mut().unwrap().parent = 1;
        assert!(matches!(
            replay(&typed, &trace, &mut 10_000),
            Err(Verdict::Reject(_))
        ));
    }

    #[test]
    fn source_typing_rejects_an_erased_ill_typed_argument() {
        let mut i = source();
        i.exprs.insert(
            9,
            json!({"ie":9,"lam":{"name":0,"type":0,"body":1,"binderInfo":"default"}}),
        );
        i.exprs.insert(10, json!({"ie":10,"app":{"fn":9,"arg":0}}));
        assert!(matches!(
            TypedSource::check(&i, 10, &HashSet::new(), &mut 100_000),
            Err(Verdict::Reject(_))
        ));
    }

    #[test]
    fn source_let_is_typed_and_replayed_without_substitution() {
        let mut i = source();
        i.exprs.insert(
            9,
            json!({"ie":9,"letE":{"name":0,"type":0,"value":1,"body":3,"nonDep":false}}),
        );
        let typed = TypedSource::check(&i, 9, &HashSet::new(), &mut 100_000).unwrap();
        let trace = produce(&typed, &mut 1000).unwrap();
        let checked = replay(&typed, &trace, &mut 1000).unwrap();
        assert_eq!(
            checked.readback(&mut 100).unwrap(),
            Term::Global("A".into())
        );
        i.exprs.insert(
            9,
            json!({"ie":9,"letE":{"name":0,"type":0,"value":0,"body":1,"nonDep":false}}),
        );
        assert!(matches!(
            TypedSource::check(&i, 9, &HashSet::new(), &mut 100_000),
            Err(Verdict::Reject(_))
        ));
    }

    #[test]
    fn unsupported_neutral_application_is_an_honest_decline() {
        let mut i = source();
        i.exprs
            .insert(9, json!({"ie":9,"const":{"name":3,"us":[]}}));
        i.exprs.insert(10, json!({"ie":10,"app":{"fn":9,"arg":1}}));
        let typed = TypedSource::check(&i, 10, &HashSet::new(), &mut 100_000).unwrap();
        assert!(matches!(
            produce(&typed, &mut 1000),
            Err(Verdict::Decline(_))
        ));
    }

    #[test]
    fn neither_production_nor_replay_refills_an_exhausted_allowance() {
        let i = source();
        assert!(TypedSource::check(&i, 8, &HashSet::new(), &mut 0).is_err());
        let typed = TypedSource::check(&i, 8, &HashSet::new(), &mut 100_000).unwrap();
        assert!(matches!(produce(&typed, &mut 0), Err(Verdict::Decline(_))));
        let trace = produce(&typed, &mut 1000).unwrap();
        assert!(matches!(
            replay(&typed, &trace, &mut 0),
            Err(Verdict::Decline(_))
        ));
    }

    #[test]
    fn a_trace_cannot_be_replayed_for_a_different_source_root() {
        let i = source();
        let first = TypedSource::check(&i, 8, &HashSet::new(), &mut 100_000).unwrap();
        let second = TypedSource::check(&i, 2, &HashSet::new(), &mut 100_000).unwrap();
        let trace = produce(&first, &mut 1000).unwrap();
        assert!(matches!(
            replay(&second, &trace, &mut 1000),
            Err(Verdict::Reject(_))
        ));
    }
    #[test]
    fn source_changes_do_not_reuse_a_previously_valid_trace() {
        let first = source();
        let checked_first = TypedSource::check(&first, 8, &HashSet::new(), &mut 100_000).unwrap();
        let trace = produce(&checked_first, &mut 1000).unwrap();
        let mut second = source();
        second
            .exprs
            .insert(7, json!({"ie":7,"app":{"fn":6,"arg":2}}));
        let checked_second = TypedSource::check(&second, 8, &HashSet::new(), &mut 100_000).unwrap();
        assert!(matches!(
            replay(&checked_second, &trace, &mut 1000),
            Err(Verdict::Reject(_))
        ));
    }

    #[test]
    fn a_lambda_endpoint_does_not_claim_implemented_readback() {
        let i = source();
        let typed = TypedSource::check(&i, 6, &HashSet::new(), &mut 100_000).unwrap();
        let trace = produce(&typed, &mut 1000).unwrap();
        let checked = replay(&typed, &trace, &mut 1000).unwrap();
        assert!(matches!(
            checked.readback(&mut 1000),
            Err(Verdict::Decline(_))
        ));
    }
}

#[cfg(test)]
#[path = "lazy_conversion/source_typing_controls.rs"]
mod source_typing_controls;

#[path = "lazy_conversion/native_conversion.rs"]
mod native_conversion;

#[cfg(test)]
#[path = "lazy_conversion/runtime_controls.rs"]
mod runtime_controls;
#[path = "lazy_conversion/runtime_conversion.rs"]
mod runtime_conversion;

/// Pure validation: the immutable source token remains alive through replay.
/// Publication still belongs to the importer's ordinary declaration path.
pub(super) fn checked_body(
    importer: &Importer<'_>,
    root: usize,
    params: &HashSet<String>,
    declared: &Term,
    budget: &mut usize,
) -> Result<Term> {
    fn goal(
        ctx: &Context,
        actual: &Term,
        declared: &Term,
        bridge: &mut native_conversion::NativeCertificates<'_>,
        budget: &mut usize,
    ) -> Result<()> {
        native_conversion::admit(actual, None, budget)?;
        native_conversion::admit(declared, None, budget)?;
        spend(budget, 1)?;
        let actual_sort = lean_type(ctx, actual)?;
        if !matches!(normal(ctx, &actual_sort)?, Term::Sort(_)) {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "certified declaration type is not a type",
            ));
        }
        // This is an actual conversion obligation between TYPES. Merely
        // checking the reduced value would erase the source's original goal.
        bridge.conversion(actual, declared, &actual_sort, budget)
    }
    let source = TypedSource::check(importer, root, params, budget)?;
    let original_type = source.typing.root_type(root, budget)?;
    let mut bridge = native_conversion::NativeCertificates::new(&importer.ctx, budget)?;
    goal(&importer.ctx, &original_type, declared, &mut bridge, budget)?;
    let body = runtime_conversion::normalize(&source, budget)?;
    native_conversion::admit(&body, None, budget)?;
    spend(budget, 1)?;
    let actual = lean_type(&importer.ctx, &body)?;
    let independent = crate::recheck(&importer.ctx, &body)
        .map_err(|e| independent_verdict(&e, "independent runtime result typing"))?;
    goal(&importer.ctx, &actual, declared, &mut bridge, budget)?;
    goal(&importer.ctx, &independent, declared, &mut bridge, budget)?;
    Ok(body)
}
#[cfg(test)]
#[path = "lazy_conversion/runtime_goal_controls.rs"]
mod runtime_goal_controls;

#[cfg(test)]
#[path = "lazy_conversion/runtime_import_controls.rs"]
mod runtime_import_controls;
