//! Read-only parameter inference, stated as Datalog.
//!
//! [`super::readonly::ReadonlyParams`] finds, by a fixpoint over the call graph, the `Seq` parameters a function never
//! changes — the ones codegen may borrow as `&[T]` instead of owning or cloning. The same answer is four rules over
//! facts read off the program ([`READONLY_RULES`]), evaluated by `logicaffeine_proof::datalog`:
//!
//! - `MutatesDirectly(f, p)`: the body of `f`, a function the program defines, changes its name `p` (a push, a pop, an
//!   add or remove, an indexed or whole reassignment, a mutable copy of it);
//! - `PassesTo(f, a, g, q)`: `f` hands its name `a` to parameter `q` of a function `g` the program declares;
//! - `SeqParam(f, p)`: `p` is a `Seq` parameter of `f`, a candidate;
//! - `Plain(g, q)`: `q` is a parameter of `g` that is not one — handing a list to it does not read it as a list.
//!
//! A parameter is read-only when it is a candidate nothing mutates; it is mutated when its own body changes it or it is
//! handed to a position that is mutated or plain. `readonly_rules_lock` holds the rules' answer to the hand-written
//! pass's on every program of the corpus.

use std::collections::{HashMap, HashSet};

use logicaffeine_base::Symbol;
use logicaffeine_language::ast::Stmt;
use logicaffeine_proof::datalog::{evaluate, Atom, Program, Term};

use super::readonly::{collect_call_sites, collect_direct_mutations, is_seq_type};
use super::types::TypeEnv;

/// The four rules.
pub const READONLY_RULES: &str = "\
Axiom direct: for all f p, if MutatesDirectly(f, p) then Mutated(f, p).
Axiom passed: for all f a g q, if PassesTo(f, a, g, q) and Mutated(g, q) then Mutated(f, a).
Axiom plain: for all f a g q, if PassesTo(f, a, g, q) and Plain(g, q) then Mutated(f, a).
Axiom readonly: for all f p, if SeqParam(f, p) and not Mutated(f, p) then ReadOnly(f, p).
";

fn name(sym: Symbol) -> String {
    format!("s{}", sym.index())
}

fn fact(predicate: &str, symbols: &[Symbol]) -> Atom {
    Atom::new(predicate, symbols.iter().map(|s| Term::constant(&name(*s))).collect())
}

/// The facts [`READONLY_RULES`] read, from `stmts`, each name written `s<index>` of its symbol.
pub fn readonly_facts(stmts: &[Stmt<'_>], type_env: &TypeEnv) -> Vec<Atom> {
    let mut params: HashMap<Symbol, Vec<Symbol>> = HashMap::new();
    let mut facts = Vec::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name: f, params: ps, .. } = stmt {
            params.insert(*f, ps.iter().map(|(p, _)| *p).collect());
            for (p, _) in ps.iter() {
                let predicate = if is_seq_type(type_env.lookup(*p)) { "SeqParam" } else { "Plain" };
                facts.push(fact(predicate, &[*f, *p]));
            }
        }
    }
    for stmt in stmts {
        let Stmt::FunctionDef { name: f, params: ps, body, is_native, .. } = stmt else {
            continue;
        };
        if *is_native {
            continue;
        }
        let own: HashSet<Symbol> = ps.iter().map(|(p, _)| *p).collect();
        for p in collect_direct_mutations(body, &own) {
            facts.push(fact("MutatesDirectly", &[*f, p]));
        }
        for (g, args) in collect_call_sites(body) {
            let Some(positions) = params.get(&g) else {
                continue;
            };
            for (a, q) in args.iter().zip(positions) {
                if let Some(a) = a {
                    facts.push(fact("PassesTo", &[*f, *a, g, *q]));
                }
            }
        }
    }
    facts
}

/// Each function's read-only `Seq` parameters, by [`READONLY_RULES`] over [`readonly_facts`].
pub fn readonly_by_rules(stmts: &[Stmt<'_>], type_env: &TypeEnv) -> Result<HashMap<Symbol, HashSet<Symbol>>, String> {
    let development =
        logicaffeine_proof::development::parse_development(READONLY_RULES).map_err(|e| e.to_string())?;
    let rules = Program::from_axioms(&development.axioms).map_err(|e| e.to_string())?;
    let program = Program::new(readonly_facts(stmts, type_env), rules.rules().to_vec());
    let model = evaluate(&program).map_err(|e| e.to_string())?;
    let mut symbols: HashMap<String, Symbol> = HashMap::new();
    let mut readonly: HashMap<Symbol, HashSet<Symbol>> = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name: f, params: ps, .. } = stmt {
            symbols.insert(name(*f), *f);
            for (p, _) in ps.iter() {
                symbols.insert(name(*p), *p);
            }
            readonly.entry(*f).or_default();
        }
    }
    for tuple in model.relation("ReadOnly") {
        if let (Some(f), Some(p)) = (symbols.get(&tuple[0]), symbols.get(&tuple[1])) {
            readonly.entry(*f).or_default().insert(*p);
        }
    }
    Ok(readonly)
}
