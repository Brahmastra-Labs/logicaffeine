//! `largo datalog` — a theory's rules evaluated bottom-up to their least model.
//!
//! The file's `## Theory` (and its `## Axiom` blocks) are read as a Datalog program: every ground atom a fact, every
//! `for all …, if … then …` a Horn rule (`logicaffeine_proof::datalog`). The command evaluates it semi-naively and
//! prints each relation the rules derive. `--bdd` evaluates it again over binary decision diagrams, bddbddb's
//! representation, and gives each relation's diagram size; `--certify` proves every derived fact in the kernel from
//! the theory's axioms, and names the facts that rest on the closed world (a negation or a comparison in a rule that
//! derived them), which the least model holds and the axioms alone do not entail; `--json` prints the model as data. `--contexts` numbers the call paths of the theory's
//! `Calls(caller, site, callee)` facts and evaluates over diagrams with the rules reading each method's contexts
//! (`logicaffeine_proof::datalog::Contexts`); a relation over contexts is given by its size, not its tuples.

use std::path::{Path, PathBuf};

use logicaffeine_proof::datalog::{evaluate, evaluate_bdd, evaluate_bdd_with_contexts, Contexts, Program};

use crate::ui::CliError;

/// Handle `largo datalog FILE`.
pub(crate) fn cmd_datalog(
    file: PathBuf,
    bdd: bool,
    certify: bool,
    json: bool,
    contexts: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let closure = crate::closure::assemble_closure(&file)
        .map_err(|e| CliError::new(format!("could not assemble {}: {e}", file.display())))?;
    let development = logicaffeine_compile::ui_bridge::theory_development(&closure.source)
        .map_err(|e| CliError::new(format!("the theory in {} did not parse: {e}", file.display())))?;
    let program = Program::from_axioms(&development.axioms).map_err(|e| CliError::new(e.to_string()))?;
    if contexts {
        return in_contexts(&file, &program, json);
    }
    let model = evaluate(&program).map_err(|e| CliError::new(e.to_string()))?;
    let diagrams = if bdd { Some(evaluate_bdd(&program).map_err(|e| CliError::new(e.to_string()))?) } else { None };

    let derived: std::collections::BTreeSet<&str> = program.rules().iter().map(|r| r.head.predicate.as_str()).collect();
    let given: std::collections::BTreeSet<(String, Vec<String>)> = program
        .facts()
        .iter()
        .map(|f| (f.predicate.clone(), f.ground().expect("a fact is ground")))
        .collect();
    let derived_facts: Vec<(String, Vec<String>)> = model
        .facts()
        .into_iter()
        .filter(|(p, a)| derived.contains(p.as_str()) && !given.contains(&(p.clone(), a.clone())))
        .collect();

    let certified = if certify {
        let axioms: Vec<logicaffeine_proof::ProofExpr> = development.axioms.iter().map(|(_, e)| e.clone()).collect();
        let mut proved = 0usize;
        let mut unproved = Vec::new();
        let mut closed = Vec::new();
        for (predicate, args) in &derived_facts {
            let atom = format!("{predicate}({})", args.join(", "));
            if model.closed_world(predicate, args) {
                closed.push(atom);
                continue;
            }
            let goal = logicaffeine_proof::formula::parse_formula(&atom)
                .map_err(|e| CliError::new(format!("{atom}: {e}")))?;
            if logicaffeine_proof::verify::prove_certify_check(&axioms, &goal).verified {
                proved += 1;
            } else {
                unproved.push(atom);
            }
        }
        Some((proved, unproved, closed))
    } else {
        None
    };

    if json {
        let relations: serde_json::Map<String, serde_json::Value> = derived
            .iter()
            .map(|p| (p.to_string(), serde_json::json!(model.relation(p).into_iter().collect::<Vec<_>>())))
            .collect();
        let mut report = serde_json::json!({
            "file": file.display().to_string(),
            "relations": relations,
            "derived": derived_facts.len(),
            "given": given.len(),
            "rule_instances": model.work(),
        });
        if let Some(d) = &diagrams {
            report["bdd_nodes"] = serde_json::json!(derived
                .iter()
                .map(|p| (p.to_string(), serde_json::json!(d.nodes(p))))
                .collect::<serde_json::Map<_, _>>());
        }
        if let Some((proved, unproved, closed)) = &certified {
            report["certified"] = serde_json::json!(proved);
            report["unproved"] = serde_json::json!(unproved);
            report["closed_world"] = serde_json::json!(closed);
        }
        println!("{report}");
    } else {
        for predicate in &derived {
            let tuples = model.relation(predicate);
            let shown: Vec<String> = tuples.iter().map(|t| format!("({})", t.join(", "))).collect();
            let size = diagrams.as_ref().map(|d| format!("  [BDD nodes: {}]", d.nodes(predicate))).unwrap_or_default();
            println!("{predicate} ({}): {}{size}", tuples.len(), shown.join(" "));
        }
        println!(
            "{} facts derived from {} given by {} rules, semi-naively ({} rule instances)",
            derived_facts.len(),
            given.len(),
            program.rules().len(),
            model.work()
        );
        if let Some((proved, unproved, closed)) = &certified {
            println!("{proved} of {} derived facts kernel-certified", derived_facts.len());
            if !closed.is_empty() {
                println!(
                    "{} rest on the closed world: a rule that derived them has a negation or a comparison, which the \
                     axioms alone do not entail",
                    closed.len()
                );
                for atom in closed {
                    println!("  closed world: {atom}");
                }
            }
            for atom in unproved {
                println!("  not proved: {atom}");
            }
        }
    }
    if let Some((_, unproved, _)) = &certified {
        if !unproved.is_empty() {
            return Err(CliError::new(format!(
                "{} derived facts that rest on the axioms alone the kernel did not prove: the evaluation and the prover \
                 disagree",
                unproved.len()
            ))
            .into());
        }
    }
    Ok(())
}

/// `largo datalog FILE --contexts`: the call paths numbered, the rules evaluated over diagrams in every context.
fn in_contexts(file: &Path, program: &Program, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let numbering = Contexts::number(program).map_err(|e| CliError::new(e.to_string()))?;
    let model = evaluate_bdd_with_contexts(program, &numbering).map_err(|e| CliError::new(e.to_string()))?;
    let derived: std::collections::BTreeSet<&str> = program.rules().iter().map(|r| r.head.predicate.as_str()).collect();
    if json {
        let methods: serde_json::Map<String, serde_json::Value> =
            numbering.methods().map(|(m, n)| (m.to_string(), serde_json::json!(n))).collect();
        let relations: serde_json::Map<String, serde_json::Value> = derived
            .iter()
            .filter(|p| !model.over_contexts(p))
            .map(|p| (p.to_string(), serde_json::json!(model.relation(p).into_iter().collect::<Vec<_>>())))
            .collect();
        let tuples: serde_json::Map<String, serde_json::Value> =
            derived.iter().map(|p| (p.to_string(), count(model.count(p)))).collect();
        let nodes: serde_json::Map<String, serde_json::Value> =
            derived.iter().map(|p| (p.to_string(), serde_json::json!(model.nodes(p)))).collect();
        let report = serde_json::json!({
            "file": file.display().to_string(),
            "contexts": count(numbering.total()),
            "context_bits": numbering.bits(),
            "methods": methods,
            "relations": relations,
            "tuples": tuples,
            "bdd_nodes": nodes,
            "total_nodes": model.total_nodes(),
        });
        println!("{report}");
        return Ok(());
    }
    let bits = numbering.bits();
    println!(
        "{} contexts over {} methods, numbered in {bits} bit{}",
        numbering.total(),
        numbering.methods().count(),
        if bits == 1 { "" } else { "s" }
    );
    for predicate in &derived {
        if model.over_contexts(predicate) {
            println!(
                "{predicate}: {} tuples over contexts  [BDD nodes: {}]",
                model.count(predicate),
                model.nodes(predicate)
            );
        } else {
            let tuples = model.relation(predicate);
            let shown: Vec<String> = tuples.iter().map(|t| format!("({})", t.join(", "))).collect();
            println!("{predicate} ({}): {}  [BDD nodes: {}]", tuples.len(), shown.join(" "), model.nodes(predicate));
        }
    }
    println!("evaluated over binary decision diagrams in {} nodes", model.total_nodes());
    Ok(())
}

/// A count as JSON: a number while it fits one, its decimal digits past that.
fn count(n: u128) -> serde_json::Value {
    u64::try_from(n).map_or_else(|_| serde_json::json!(n.to_string()), |n| serde_json::json!(n))
}
